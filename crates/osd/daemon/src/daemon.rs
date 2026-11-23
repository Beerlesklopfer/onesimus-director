// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Der gemeinsame Zustand des Daemons und die Wege zum Director.
//!
//! Zweck: [`Daemon`] hält, was alle Verbindungen brauchen: Konfiguration,
//! Jobtabelle, Startzeit (`VolSessionTime`), die Belegung der Devices und
//! wie Geräteschreiber gestartet werden. Dazu die Sendewege zum Director:
//! Job-Meldungen (`Jmsg`), Job-Status (`Status`), Katalog-Anfragen mit
//! Antwort und Attribute (`UpdCat`).
//!
//! Bareos-Gegenstück: `core/src/stored/stored.cc` (globale Daten,
//! `vol_session_time`), `lib/message.cc` (`Jmsg` → Director),
//! `lib/jcr.cc` (`sendJobStatus`), `stored/reserve.cc` (Belegung).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bareos_bnet::BnetMessage;
use bareos_config::sd::{RouteFacts, SdConfig, SdDevice, SdDistributor};

use crate::device::FileDevice;
use crate::fd_session::{WriterHandle, DEFAULT_RING_SLOTS};
use crate::jobs::{Job, JobCommand, JobTable, UseStorage};
use crate::transport::{lock, SharedChannel};

/// How device writers and readers are started.
#[derive(Debug, Clone)]
pub enum Launcher {
    /// As child process `<exe> --device-writer <ring>` or
    /// `<exe> --device-reader <ring>` (D-13).
    Process(PathBuf),
    /// As thread of this process (tests); it talks only through the ring.
    Thread,
}

/// A device reserved for a job.
#[derive(Debug, Clone)]
pub struct Reserved {
    /// The device.
    pub device: FileDevice,
    /// Ring slots for the job (osd.d `Ring Slots`, else the default).
    pub slots: u32,
    /// Reading through a Distributor: the other targets the job holds (the
    /// reader takes the one that has the volume); empty otherwise.
    pub alternatives: Vec<FileDevice>,
}

/// Suffix of the holder name of a device a read job holds as alternative
/// target of a Distributor (the job name follows it never; it ends the name).
const ALT_MARK: &str = "\u{1}alt";

/// The holder name of an alternative device of job `job`.
fn alt_holder(job: &str) -> String {
    format!("{job}{ALT_MARK}")
}

/// The daemon-wide state, cheap to clone.
#[derive(Clone)]
pub struct Daemon {
    /// The configuration (`bareos-sd.d` plus `osd.d`).
    pub config: Arc<SdConfig>,
    /// The jobs.
    pub jobs: Arc<JobTable>,
    /// Daemon start (seconds since 1970); also the `VolSessionTime`.
    pub start_time: u32,
    /// How writers are started.
    pub launcher: Launcher,
    /// Device name → unique job name using it.
    busy: Arc<Mutex<HashMap<String, String>>>,
    /// Jobs run since start.
    pub jobs_run: Arc<std::sync::atomic::AtomicUsize>,
    /// Media events (`label`, `mount`) counted per device; a job waiting for
    /// media (`WaitForSysop`) wakes on a change of its device's count.
    media: Arc<MediaEvents>,
    /// Devices whose job waits for the operator (BST_WAITING_FOR_SYSOP):
    /// `label` is allowed on them although a job holds them.
    waiting: Arc<Mutex<std::collections::HashSet<String>>>,
}

/// Media event counters per device and the condition they signal.
type MediaEvents = (Mutex<HashMap<String, u64>>, std::sync::Condvar);

/// `WaitForSysop`'s wait: until the event count of `device` changes (`w`),
/// `canceled` is set (`c`) or `timeout` passed (`t`); wakes at least every
/// second to see a cancel.
fn wait_media_event(media: &MediaEvents, device: &str, canceled: &std::sync::atomic::AtomicBool,
                    timeout: std::time::Duration) -> &'static [u8] {
    let (count, cond) = media;
    let end = std::time::Instant::now() + timeout;
    let mut events = count.lock().unwrap_or_else(|e| e.into_inner());
    let seen = events.get(device).copied().unwrap_or(0);
    loop {
        if canceled.load(std::sync::atomic::Ordering::Acquire) {
            return b"c";
        }
        if events.get(device).copied().unwrap_or(0) != seen {
            return b"w";
        }
        let now = std::time::Instant::now();
        if now >= end {
            return b"t";
        }
        let step = (end - now).min(std::time::Duration::from_secs(1));
        events = cond.wait_timeout(events, step).unwrap_or_else(|e| e.into_inner()).0;
    }
}

impl Daemon {
    /// A daemon for `config`, started now.
    pub fn new(config: SdConfig, launcher: Launcher) -> Daemon {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        Daemon {
            config: Arc::new(config),
            jobs: Arc::new(JobTable::default()),
            start_time: now as u32,
            launcher,
            busy: Arc::new(Mutex::new(HashMap::new())),
            jobs_run: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            media: Arc::default(),
            waiting: Arc::default(),
        }
    }

    /// The daemon's name in messages and labels (Bareos `my_name`).
    pub fn name(&self) -> &str {
        &self.config.storage.name
    }

    /// `Jmsg` to the Director: `<name> JobId <n>: <text>`.
    pub fn jmsg(&self, dir: &SharedChannel, job: &Job, msg_type: i32, text: &str) {
        let mtime = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = osd::askdir::jmsg(&job.cmd.job, self.name(), job.cmd.job_id, msg_type, mtime as i64, text);
        tracing::info!("{}", line.trim_end());
        let _ = lock(dir).send_str(&line);
    }

    /// `sendJobStatus(status)`: sets and reports the job status.
    pub fn send_status(&self, dir: &SharedChannel, job: &Job, status: char) {
        job.set_status(status);
        let _ = lock(dir).send_str(&osd::askdir::job_status(&job.cmd.job, job.status()));
    }

    /// A catalog request and the Director's answer (`E<error>` when the
    /// connection failed).
    pub fn cat_req(&self, dir: &SharedChannel, line: &str) -> Vec<u8> {
        let mut d = lock(dir);
        tracing::debug!(">dird {}", line.trim_end());
        if let Err(e) = d.send_str(line) {
            return format!("E{e}").into_bytes();
        }
        loop {
            match d.recv() {
                Ok(BnetMessage::Data(data)) => {
                    tracing::debug!("<dird {}", String::from_utf8_lossy(&data).trim_end());
                    return data;
                }
                Ok(BnetMessage::Signal(_)) => continue,
                Err(e) => return format!("E{e}").into_bytes(),
            }
        }
    }

    /// Sends a prepared message (attributes) to the Director.
    pub fn send_raw(&self, dir: &SharedChannel, data: &[u8]) {
        let _ = lock(dir).send(data);
    }

    /// `TryReserveAfterUse` for appending: the first device of `storage`
    /// that exists, has the job's media type and is free. A name that is a
    /// Distributor (`osd.d`, P2) stands for the target its routes pick for
    /// `job` when writing ([`Daemon::reserve_distributed`]).
    pub fn reserve_device(&self, storage: &UseStorage, job: &JobCommand) -> Option<Reserved> {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        for name in &storage.devices {
            if let Some(dist) = self.config.find_distributor(name) {
                if let Some(reserved) = self.reserve_distributed(&mut busy, dist, storage, job) {
                    return Some(reserved);
                }
                continue;
            }
            let Some(dev) = self.config.find_device(name) else { continue };
            let file_type = dev.device_type.as_deref().is_none_or(|t| t.eq_ignore_ascii_case("file"));
            if dev.media_type != storage.media_type || !file_type || busy.contains_key(&dev.name) {
                continue;
            }
            busy.insert(dev.name.clone(), job.job.clone());
            let slots = self.ring_slots(dev);
            return Some(Reserved { device: FileDevice::from_config(dev), slots, alternatives: Vec::new() });
        }
        None
    }

    /// The device of Distributor `dist` for `job`, reserved in `busy`.
    ///
    /// - **Writing** (`storage.append`): the Director's media type must be the
    ///   Distributor's; the first route that matches level, client, pool …
    ///   names the target. A busy target is not replaced by another one: the
    ///   reservation fails like one of a busy plain device (D-1: no evasion).
    ///   The target keeps its own media type, which the catalog traffic then
    ///   uses (D-2 variant a).
    /// - **Reading**: the volumes are not known yet, so the job holds the
    ///   free targets: the first one whose media type is the Director's (else
    ///   the first free one) is the read device, the others
    ///   [`Reserved::alternatives`], from which the reader takes the device of
    ///   each volume by its media type.
    fn reserve_distributed(&self, busy: &mut HashMap<String, String>, dist: &SdDistributor, storage: &UseStorage,
                           job: &JobCommand) -> Option<Reserved> {
        let file = |dev: &SdDevice| dev.device_type.as_deref().is_none_or(|t| t.eq_ignore_ascii_case("file"));
        if storage.append {
            if dist.media_type != storage.media_type {
                return None;
            }
            let facts = RouteFacts {
                job: &job.job_name,
                client: &job.client_name,
                level: job.level,
                pool: &storage.pool_name,
                job_type: job.job_type,
                fileset: &job.fileset,
            };
            let route = dist.select(&facts)?;
            let dev = self.config.find_device(&route.device).filter(|d| file(d))?;
            if busy.contains_key(&dev.name) {
                return None;
            }
            busy.insert(dev.name.clone(), job.job.clone());
            tracing::info!("Distributor {}: job {} goes to device {} ({})", dist.name, job.job, dev.name, route.conditions());
            return Some(Reserved { device: FileDevice::from_config(dev), slots: self.ring_slots(dev), alternatives: Vec::new() });
        }
        let free: Vec<&SdDevice> = dist
            .targets()
            .into_iter()
            .filter_map(|n| self.config.find_device(n))
            .filter(|d| file(d) && !busy.contains_key(&d.name))
            .collect();
        let primary = free.iter().find(|d| d.media_type == storage.media_type).or(free.first()).copied()?;
        if dist.media_type != storage.media_type && primary.media_type != storage.media_type {
            return None;
        }
        busy.insert(primary.name.clone(), job.job.clone());
        let mut alternatives = Vec::new();
        for dev in free.iter().filter(|d| d.name != primary.name) {
            busy.insert(dev.name.clone(), alt_holder(&job.job));
            alternatives.push(FileDevice::from_config(dev));
        }
        Some(Reserved { device: FileDevice::from_config(primary), slots: self.ring_slots(primary), alternatives })
    }

    /// The device job `job` holds (as its own device, not as an alternative).
    pub fn reserved_device(&self, job: &str) -> Option<String> {
        self.busy.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(_, j)| j.as_str() == job).map(|(d, _)| d.clone())
    }

    /// A `label` or `mount` on device `name`: wakes a job waiting for media.
    pub fn media_event(&self, name: &str) {
        let (count, cond) = &*self.media;
        *count.lock().unwrap_or_else(|e| e.into_inner()).entry(name.to_string()).or_default() += 1;
        cond.notify_all();
    }

    /// Whether the job on device `name` waits for the operator.
    pub fn waiting_for_sysop(&self, name: &str) -> bool {
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).contains(name)
    }

    /// `DirAskSysopToCreateAppendableVolume` → `WaitForSysop` for a device
    /// writer: JS_WaitMedia, then wait up to `secs` for a media event on
    /// `device` or a cancel. `status` is `m` (JS_WaitMedia, a writer without a volume) or
    /// `M` (JS_WaitMount, a reader whose volume is not there). Answer `w`, `t` or `c` (ring [`Request::WaitMedia`]).
    ///
    /// [`Request::WaitMedia`]: crate::ringproto::Request::WaitMedia
    pub fn wait_for_sysop(&self, dir: &SharedChannel, job: &Job, device: &str, secs: u32, status: char) -> Vec<u8> {
        if job.status() != status {
            self.send_status(dir, job, status);
        }
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).insert(device.to_string());
        let answer = wait_media_event(&self.media, device, &job.canceled, std::time::Duration::from_secs(secs as u64));
        self.waiting.lock().unwrap_or_else(|e| e.into_inner()).remove(device);
        answer.to_vec()
    }

    /// The job using device `name`.
    pub fn device_user(&self, name: &str) -> Option<String> {
        self.busy.lock().unwrap_or_else(|e| e.into_inner()).get(name).map(|j| j.strip_suffix(ALT_MARK).unwrap_or(j).to_string())
    }

    /// The device `job` has reserved, not counting `except` (the read
    /// device of a copy inside one SD, when the write device is wanted).
    pub fn reserved_device_except(&self, job: &str, except: Option<&str>) -> Option<String> {
        self.busy
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|(d, j)| j.as_str() == job && Some(d.as_str()) != except)
            .map(|(d, _)| d.clone())
    }

    /// The TLS policy osd tells a peer in the CRAM-MD5 of a connection that
    /// did not start with TLS-PSK: `ssl=0` when the Storage has `TLS Enable = no`,
    /// else `ssl=2`.
    pub fn tls_policy(&self) -> i32 {
        if self.config.storage.tls_enable {
            2
        } else {
            0
        }
    }

    /// Frees every device `job` has reserved.
    pub fn release_job_devices(&self, job: &str) {
        self.busy.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, j| j.as_str() != job && j.strip_suffix(ALT_MARK) != Some(job));
    }

    /// Frees device `name`.
    pub fn release_device(&self, name: &str) {
        self.busy.lock().unwrap_or_else(|e| e.into_inner()).remove(name);
    }

    /// Ring slots for a job on `dev` (osd.d `Ring Slots` of the device,
    /// else of the Storage, else [`DEFAULT_RING_SLOTS`]).
    pub fn ring_slots(&self, dev: &bareos_config::sd::SdDevice) -> u32 {
        dev.ring_slots.or(self.config.storage.ring_slots).unwrap_or(DEFAULT_RING_SLOTS)
    }

    /// Starts the device writer for ring `ring`.
    pub fn launch_writer(&self, ring: &str) -> Result<WriterHandle, String> {
        self.launch(ring, "--device-writer", "device writer", crate::writer::run)
    }

    /// Starts the device reader for ring `ring` (restore, verify).
    pub fn launch_reader(&self, ring: &str) -> Result<WriterHandle, String> {
        self.launch(ring, "--device-reader", "device reader", crate::reader::run)
    }

    /// Starts `<exe> <flag> <ring>`, or `run(ring)` in a thread.
    fn launch(&self, ring: &str, flag: &str, what: &str, run: fn(&str) -> i32) -> Result<WriterHandle, String> {
        match &self.launcher {
            Launcher::Process(exe) => std::process::Command::new(exe)
                .arg(flag)
                .arg(ring)
                .stdin(std::process::Stdio::null())
                .spawn()
                .map(WriterHandle::Process)
                .map_err(|e| format!("Cannot start the {what} {}: {e}", exe.display())),
            Launcher::Thread => {
                let ring = ring.to_string();
                std::thread::Builder::new()
                    .name(what.replace(' ', "-"))
                    .spawn(move || run(&ring))
                    .map(|t| WriterHandle::Thread(Some(t)))
                    .map_err(|e| e.to_string())
            }
        }
    }
}

#[cfg(test)]
mod media_wait_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    /// C-089: a label/mount on the device wakes the waiting writer, an event
    /// on another device does not, a cancel ends the wait, else timeout.
    #[test]
    fn label_wakes_cancel_ends_else_timeout() {
        let media: Arc<MediaEvents> = Arc::default();
        let canceled = AtomicBool::new(false);
        assert_eq!(wait_media_event(&media, "Dev", &canceled, Duration::from_millis(50)), b"t");
        let m = media.clone();
        let other = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            *m.0.lock().unwrap().entry("Other".into()).or_default() += 1;
            m.1.notify_all();
            std::thread::sleep(Duration::from_millis(50));
            *m.0.lock().unwrap().entry("Dev".into()).or_default() += 1;
            m.1.notify_all();
        });
        assert_eq!(wait_media_event(&media, "Dev", &canceled, Duration::from_secs(5)), b"w");
        other.join().unwrap();
        canceled.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(wait_media_event(&media, "Dev", &canceled, Duration::from_secs(5)), b"c");
    }
}

#[cfg(test)]
mod distributor_tests {
    use super::*;

    /// A daemon with two File devices (`A`: media type `MA`, `B`: `MB`) and
    /// the distributor `Dist` (media type `MD`): Full to A, client `cloud-fd`
    /// and pool `Monthly` to B, the rest to A.
    fn daemon() -> Daemon {
        let text = "Storage {\n Name = sd\n}\nDirector {\n Name = dir\n Password = x\n}\n\
            Device {\n Name = A\n Media Type = MA\n Archive Device = /tmp/a\n}\n\
            Device {\n Name = B\n Media Type = MB\n Archive Device = /tmp/b\n}\n";
        let mut config = SdConfig::from_resources(&bareos_config::raw::parse_str(text, None).unwrap()).unwrap();
        let overlay = "Distributor {\n Name = Dist\n Media Type = MD\n Route { Level = Full; Device = A }\n \
            Route { Client = cloud-fd; Device = B }\n Route { Pool = Monthly; Device = B }\n Route { Device = A }\n}\n";
        config.apply_overlay(&bareos_config::raw::parse_str(overlay, None).unwrap()).unwrap();
        Daemon::new(config, Launcher::Thread)
    }

    /// The job command of a backup of `client` at `level`, unique name `name`.
    fn job(name: &str, client: &str, level: char) -> JobCommand {
        JobCommand { job: name.into(), job_name: "Backup".into(), client_name: client.into(), level, job_type: 'B', ..Default::default() }
    }

    /// `use storage` of the distributor for writing into `pool`.
    fn write(pool: &str, media_type: &str) -> UseStorage {
        UseStorage {
            storage: "s".into(),
            media_type: media_type.into(),
            pool_name: pool.into(),
            append: true,
            devices: vec!["Dist".into()],
            ..Default::default()
        }
    }

    #[test]
    fn writing_goes_to_the_target_of_the_first_matching_route() {
        let d = daemon();
        let full = d.reserve_device(&write("P", "MD"), &job("J.1", "x-fd", 'F')).unwrap();
        assert_eq!((full.device.name.as_str(), full.device.media_type.as_str()), ("A", "MA"), "the target keeps its media type");
        let by_client = d.reserve_device(&write("P", "MD"), &job("J.2", "cloud-fd", 'I')).unwrap();
        assert_eq!(by_client.device.name, "B");
        d.release_job_devices("J.1");
        d.release_job_devices("J.2");
        let by_pool = d.reserve_device(&write("Monthly", "MD"), &job("J.3", "x-fd", 'I')).unwrap();
        assert_eq!(by_pool.device.name, "B");
        d.release_job_devices("J.3");
        let rest = d.reserve_device(&write("P", "MD"), &job("J.4", "x-fd", 'I')).unwrap();
        assert_eq!(rest.device.name, "A");
    }

    #[test]
    fn a_busy_target_is_not_evaded_and_the_media_type_must_match() {
        let d = daemon();
        assert!(d.reserve_device(&write("P", "MD"), &job("J.1", "x-fd", 'F')).is_some());
        // the second Full job wants A as well: no evasion to B
        assert!(d.reserve_device(&write("P", "MD"), &job("J.2", "x-fd", 'F')).is_none());
        assert_eq!(d.device_user("A").as_deref(), Some("J.1"));
        // the Director's Storage has another media type than the distributor
        assert!(d.reserve_device(&write("P", "MA"), &job("J.3", "x-fd", 'I')).is_none());
        d.release_job_devices("J.1");
        assert!(d.reserve_device(&write("P", "MD"), &job("J.2", "x-fd", 'F')).is_some());
    }

    #[test]
    fn reading_holds_the_targets_and_prefers_the_media_type() {
        let d = daemon();
        let read = |media: &str| UseStorage { append: false, ..write("P", media) };
        let r = d.reserve_device(&read("MD"), &job("R.1", "x-fd", 'F')).unwrap();
        assert_eq!((r.device.name.as_str(), r.alternatives.len()), ("A", 1));
        assert_eq!(r.alternatives[0].name, "B");
        // both are held: another reader gets nothing; the alternative shows as used by the job
        assert!(d.reserve_device(&read("MD"), &job("R.2", "x-fd", 'F')).is_none());
        assert_eq!(d.device_user("B").as_deref(), Some("R.1"));
        assert_eq!(d.reserved_device("R.1").as_deref(), Some("A"));
        d.release_job_devices("R.1");
        assert_eq!(d.device_user("A"), None);
        assert_eq!(d.device_user("B"), None);
        // the Director names the media type of one target: that target is the read device
        let r = d.reserve_device(&read("MB"), &job("R.3", "x-fd", 'F')).unwrap();
        assert_eq!((r.device.name.as_str(), r.alternatives.len()), ("B", 1));
        // a media type that neither the distributor nor a target has
        d.release_job_devices("R.3");
        assert!(d.reserve_device(&read("MX"), &job("R.4", "x-fd", 'F')).is_none());
    }
}
