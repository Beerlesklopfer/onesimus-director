// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Die Datenphase mit dem File Daemon: append open/data/end/close und
//! read open/data/close.
//!
//! Zweck: Nach `run` führt der FD die Sitzung. Er öffnet sie (`append
//! open session`), schickt die Datensätze (`append data`, je Datensatz ein
//! Kopf `<FileIndex> <Stream> 0`, Daten, EOD) und schließt sie wieder. osd
//! ist hier der **Erzeuger** des Rings: Er packt die Datensätze in Blöcke
//! wie bareos-sd (erst das SOS-Label, dann die Datensätze, am Ende das
//! EOS-Label beim Schreiber) und legt jeden vollen Block in ein Fach. Der
//! Geräteschreiber (eigener Prozess) schreibt sie aufs Volume.
//!
//! Beim Restore und Verify (`read open`, `read data`, `read close`) liest
//! [`crate::read_session`] die Volumes über den Ring in Gegenrichtung.
//!
//! Die Datenphase ist vom Verbindungsaufbau getrennt: Sie bekommt einen
//! [`Channel`], gleich ob der FD angerufen hat (aktiv) oder osd den FD
//! (passiv, `passive client …`), und später gleich ob BNET oder gRPC.
//!
//! Attribute (Stream `UNIX_ATTRIBUTES`, Prüfsummen, Restore-Objekte) gehen
//! wie bei bareos-sd erst an den Director, wenn der Block mit ihnen
//! geschrieben ist: hier, sobald der Schreiber das Fach freigegeben hat.
//!
//! Bareos-Gegenstück: `core/src/stored/fd_cmds.cc` (`RunJob`,
//! `DoFdCommands`, `AppendOpenSession`, `AppendDataCmd`, `AppendEndSession`,
//! `AppendCloseSession`, `ReadOpenSession`, `ReadDataCmd`,
//! `ReadCloseSession`), `stored/append.cc` (`DoAppendData`, `SetupDCR`,
//! `IsAttribute`, `SendAttrsToDir`), `stored/label.cc` (`WriteSessionLabel`),
//! `stored/reserve.cc` (`TryReserveAfterUse`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bareos_bnet::{BnetMessage, BnetSignal};
use bareos_format::label::SOS_LABEL;
use bareos_format::msg::{M_FATAL, M_INFO};
use bareos_format::stream;
use bareos_format::volume::{BlockBuilder, RecordProgress, RecordRef};
use osd_ring::{Ring, RingConfig, State};

use crate::daemon::Daemon;
use crate::device::{with_suffix, FileDevice};
use crate::jobs::Job;
use crate::ringproto::{ring_name, JobInfo, Request, SlotKind, SlotMeta};
use crate::transport::{lock, text, Channel, SharedChannel};

/// Default number of ring slots (`Ring Slots` in osd.d overrides it): 8 where
/// the ring wakes waiters (Linux), 64 where it polls (macOS, Windows; measured
/// in docs/plans/OSD-RECHERCHE-SPEICHER-UND-RING.md: with 8 slots the sleep of
/// a polling reader limits the ring to a few hundred MiB/s).
#[cfg(target_os = "linux")]
pub const DEFAULT_RING_SLOTS: u32 = 8;
/// See above.
#[cfg(not(target_os = "linux"))]
pub const DEFAULT_RING_SLOTS: u32 = 64;

/// Longest wait for a free slot before the writer counts as stuck.
const CLAIM_TIMEOUT: Duration = Duration::from_secs(3600);

/// `IsAttribute`: records the Director gets as file attributes.
pub fn is_attribute(stream_id: i32) -> bool {
    let masked = stream_id & stream::STREAMMASK_TYPE;
    masked == stream::STREAM_UNIX_ATTRIBUTES
        || masked == stream::STREAM_UNIX_ATTRIBUTES_EX
        || masked == stream::STREAM_RESTORE_OBJECT
        || stream::is_digest_stream(masked)
}

/// `RunJob`: the job's FD session, framed by `3010 Job … start` and
/// `3099 Job … end …` to the Director.
pub fn run_job(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel) {
    let name = job.cmd.job.clone();
    let _ = lock(dir).send_str(&format!("3010 Job {name} start\n"));
    d.send_status(dir, job, 'R');
    let Some(mut conn) = job.fd.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    if !conn.sd && !conn.replicate {
        // DoListenRun (the writing SD of a replication) sends no version line
        d.jmsg(dir, job, M_INFO, &format!("{}\n", daemon_crate::version::version_line()));
    }
    let started = Instant::now();
    let mut session = Session { opened: false, started };
    if conn.replicate {
        // the reading side of a Copy: osd sends the data
        match crate::read_session::do_replicate_read(d, job, dir, conn.channel.as_mut()) {
            Some((files, bytes)) => {
                job.files.store(files as _, Ordering::Release);
                *job.bytes.lock().unwrap_or_else(|e| e.into_inner()) = bytes as _;
                job.set_status('T');
            }
            None => job.set_status('E'),
        }
        let _ = conn.channel.signal(BnetSignal::Terminate);
        let end_status = job.status() as u32;
        let (files, bytes) = (job.files.load(Ordering::Acquire), *job.bytes.lock().unwrap_or_else(|e| e.into_inner()));
        let mut dir_lock = lock(dir);
        let _ = dir_lock.send_str(&format!("3099 Job {name} end JobStatus={end_status} JobFiles={files} JobBytes={bytes} JobErrors=0\n"));
        let _ = dir_lock.signal(BnetSignal::Eod);
        drop(dir_lock);
        return;
    }
    #[cfg(unix)]
    let socket_fd = match &conn.breaker {
        crate::jobs::Breaker::Socket(s) => std::os::fd::AsRawFd::as_raw_fd(s) as i64,
        _ => -1,
    };
    #[cfg(not(unix))]
    let socket_fd = -1;
    job.net.fd.store(socket_fd, Ordering::Relaxed);
    let mut counted = crate::transport::Counted { inner: conn.channel.as_mut(), counters: &job.net };
    if conn.sd {
        do_sd_commands(d, job, dir, &mut counted, &mut session);
    } else {
        do_fd_commands(d, job, dir, &mut counted, &mut session);
    }
    let _ = conn.channel.signal(BnetSignal::Terminate);
    job.set_status('T');
    let bytes = *job.bytes.lock().unwrap_or_else(|e| e.into_inner());
    let mut dir_lock = lock(dir);
    let _ = dir_lock.send_str(&format!(
        "3099 Job {name} end JobStatus={} JobFiles={} JobBytes={bytes} JobErrors=0\n",
        job.status() as u32,
        job.files.load(Ordering::Acquire)
    ));
    let _ = dir_lock.signal(BnetSignal::Eod);
    drop(dir_lock);
    drop(conn);
}

/// The copy inside one SD (VirtualFull, Copy to a second device of this
/// SD; `use storage append=0`, then `append=1`, then `run`): the reading
/// and the writing side of the copy protocol run against each other over
/// an in-process connection, the read side on this thread, the write
/// side on another. The job's end is reported like a backup's.
pub fn run_local_copy(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel) {
    let name = job.cmd.job.clone();
    let _ = lock(dir).send_str(&format!("3010 Job {name} start\n"));
    d.send_status(dir, job, 'R');
    let write = job.local_write.lock().unwrap_or_else(|e| e.into_inner()).take();
    let (mut reader_end, mut writer_end) = crate::transport::MemChannel::pair();
    let (opened_tx, opened_rx) = std::sync::mpsc::channel::<()>();
    let writer = {
        let (d, job, dir) = (d.clone(), job.clone(), dir.clone());
        std::thread::Builder::new().name(format!("local-copy:{}", job.cmd.job_id)).spawn(move || {
            // the read side has claimed its volumes (its ring is up) before the write side starts
            if opened_rx.recv().is_err() {
                return;
            }
            let mut session = Session { opened: false, started: Instant::now() };
            do_sd_commands(&d, &job, &dir, &mut writer_end, &mut session);
        })
    };
    let result = match writer {
        Ok(thread) => {
            // the write side uses the write storage from now on
            let read_storage = job.storage.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let r = crate::read_session::do_local_read(d, job, dir, &mut reader_end, write, &opened_tx);
            drop(opened_tx);
            let _ = reader_end.signal(BnetSignal::Terminate);
            let _ = thread.join();
            *job.storage.lock().unwrap_or_else(|e| e.into_inner()) = read_storage;
            r
        }
        Err(e) => {
            d.jmsg(dir, job, M_FATAL, &format!("Cannot start the write side of the copy: {e}\n"));
            None
        }
    };
    if result.is_none() && !job.canceled.load(Ordering::Acquire) {
        job.set_status('E');
    } else if result.is_some() && job.status() != 'E' as _ {
        job.set_status('T');
    }
    let bytes = *job.bytes.lock().unwrap_or_else(|e| e.into_inner());
    let mut dir_lock = lock(dir);
    let _ = dir_lock.send_str(&format!(
        "3099 Job {name} end JobStatus={} JobFiles={} JobBytes={bytes} JobErrors=0\n",
        job.status() as u32,
        job.files.load(Ordering::Acquire)
    ));
    let _ = dir_lock.signal(BnetSignal::Eod);
}

/// State of the FD dialog (`session_opened`).
struct Session {
    /// `append open session` was accepted.
    opened: bool,
    /// When the job started running (transfer rate).
    started: Instant,
}

/// `DoFdCommands`: reads the FD's commands until it hangs up.
fn do_fd_commands(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, fd: &mut dyn Channel, s: &mut Session) {
    loop {
        let msg = match fd.recv() {
            Ok(BnetMessage::Data(data)) if !data.is_empty() => text(&data),
            Ok(BnetMessage::Data(_)) | Ok(BnetMessage::Signal(_)) => continue,
            Err(_) => break,
        };
        tracing::debug!("{}: <filed: {}", job.cmd.job, msg.trim_end());
        let quit = if msg.starts_with("append open") {
            if s.opened {
                let _ = fd.send_str("3901 Error session already open\n");
                fatal_with_fd(d, dir, job, "Attempt to open already open session.\n");
                true
            } else {
                s.opened = true;
                fd.send_str(&format!("3000 OK open ticket = {}\n", job.vol_session_id)).is_err()
            }
        } else if msg.starts_with("append data") {
            if s.opened {
                let ok = do_append_data(d, job, dir, fd, s, false);
                if !ok {
                    let _ = fd.send_str("3903 Error append data\n");
                    fatal_with_fd(d, dir, job, "Append data error.\n");
                }
                !ok
            } else {
                let _ = fd.send_str("3902 Error session not opened\n");
                fatal_with_fd(d, dir, job, "Attempt to append on non-open session.\n");
                true
            }
        } else if msg.starts_with("append end") {
            if s.opened {
                fd.send_str("3000 OK end\n").is_err()
            } else {
                let _ = fd.send_str("3902 Error session not opened\n");
                fatal_with_fd(d, dir, job, "Attempt to close non-open session.\n");
                true
            }
        } else if msg.starts_with("append close") {
            if s.opened {
                let r = fd.send_str(&format!("3000 OK close Status = {}\n", job.status() as u32));
                let _ = fd.signal(BnetSignal::Eod);
                s.opened = false;
                r.is_err()
            } else {
                let _ = fd.send_str("3902 Error session not opened\n");
                fatal_with_fd(d, dir, job, "Attempt to close non-open session.\n");
                true
            }
        } else if msg.starts_with("read open") {
            // ReadOpenSession: the fields (`read open session = <Volume>
            // <VolSessionId> <VolSessionTime> <StartFile> <EndFile>
            // <StartBlock> <EndBlock>`) are only logged by bareos-sd; the
            // bootstrap decides what is read.
            if s.opened {
                let _ = fd.send_str("3901 Error session already open\n");
                fatal_with_fd(d, dir, job, "Attempt to open read on non-open session.\n");
                true
            } else {
                s.opened = true;
                fd.send_str(&format!("3000 OK open ticket = {}\n", job.vol_session_id)).is_err()
            }
        } else if msg.starts_with("read data") {
            if s.opened {
                let ok = crate::read_session::do_read_data(d, job, dir, fd);
                if !ok {
                    fatal_with_fd(d, dir, job, "");
                }
                !ok
            } else {
                let _ = fd.send_str("3902 Error session not opened\n");
                fatal_with_fd(d, dir, job, "Attempt to read on non-open session.\n");
                true
            }
        } else if msg.starts_with("read close") {
            if s.opened {
                let r = fd.send_str(&format!("3000 OK close Status = {}\n", job.status() as u32));
                let _ = fd.signal(BnetSignal::Eod);
                s.opened = false;
                r.is_err()
            } else {
                let _ = fd.send_str("3902 Error session not opened\n");
                fatal_with_fd(d, dir, job, "");
                true
            }
        } else {
            if !job.canceled.load(Ordering::Acquire) {
                d.jmsg(dir, job, M_FATAL, &format!("FD command not found: {msg}"));
            }
            let _ = fd.send_str("3900 Invalid command\n");
            job.set_status('E');
            break;
        };
        if quit {
            break;
        }
    }
}

/// `DoStoredCommands` (stored/sd_cmds.cc): the commands of the reading SD
/// of a Copy/Migrate: `start replicate`, `replicate data` (the records as an
/// FD sends them), `end replicate`, until it hangs up.
fn do_sd_commands(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, sd: &mut dyn Channel, s: &mut Session) {
    loop {
        let msg = match sd.recv() {
            Ok(BnetMessage::Data(data)) if !data.is_empty() => text(&data),
            Ok(BnetMessage::Data(_)) | Ok(BnetMessage::Signal(_)) => continue,
            Err(_) => break,
        };
        tracing::debug!("{}: <stored: {}", job.cmd.job, msg.trim_end());
        let quit = if msg.starts_with("start replicate") {
            if s.opened {
                let _ = sd.send_str("3901 Error replicate session already open\n");
                fatal_with_sd(d, dir, job, "Attempt to open already open session.\n");
                true
            } else {
                s.opened = true;
                sd.send_str(&format!("3000 OK start replicate ticket = {}\n", job.vol_session_id)).is_err()
            }
        } else if msg.starts_with("replicate data") {
            if s.opened {
                let ok = do_append_data(d, job, dir, sd, s, true);
                if !ok {
                    let _ = sd.send_str("3903 Error replicate data\n");
                    fatal_with_sd(d, dir, job, "Replicate data error.\n");
                }
                !ok
            } else {
                let _ = sd.send_str("3902 Error replicate session not opened\n");
                fatal_with_sd(d, dir, job, "Attempt to replicate on non-open session.\n");
                true
            }
        } else if msg.starts_with("end replicate") {
            sd.send_str("3000 OK end replicate\n").is_err()
        } else {
            if !job.canceled.load(Ordering::Acquire) {
                d.jmsg(dir, job, M_FATAL, &format!("Storage daemon command not found: {msg}"));
            }
            let _ = sd.send_str("3900 Invalid command\n");
            job.set_status('E');
            break;
        };
        if quit {
            break;
        }
    }
}

/// `Command error with SD, hanging up.` and `JS_ErrorTerminated`.
fn fatal_with_sd(d: &Daemon, dir: &SharedChannel, job: &Arc<Job>, reason: &str) {
    if !job.canceled.load(Ordering::Acquire) {
        d.jmsg(dir, job, M_FATAL, &format!("Command error with SD, hanging up. {reason}"));
        job.set_status('E');
    }
}

/// `Command error with FD, hanging up. <reason>` (or without a reason
/// `Command error with FD, hanging up.`) and `JS_ErrorTerminated`.
fn fatal_with_fd(d: &Daemon, dir: &SharedChannel, job: &Arc<Job>, reason: &str) {
    if !job.canceled.load(Ordering::Acquire) {
        let text = if reason.is_empty() {
            "Command error with FD, hanging up.\n".to_string()
        } else {
            format!("Command error with FD, hanging up. {reason}")
        };
        d.jmsg(dir, job, M_FATAL, &text);
        job.set_status('E');
    }
}

/// The ring side of one append: the ring, its writer and the block being
/// filled.
struct Producer {
    /// The job's ring.
    ring: Arc<Ring>,
    /// The writer process or thread.
    writer: WriterHandle,
    /// The mailbox server thread and its stop flag.
    mail: Option<(std::thread::JoinHandle<()>, Arc<AtomicBool>)>,
    /// The block being filled.
    builder: BlockBuilder,
    /// Block size.
    block_size: usize,
    /// Slots published so far.
    published: u64,
    /// Attribute messages waiting until their slot is written: (slot, message).
    pending_attrs: VecDeque<(u64, Vec<u8>)>,
}

/// How a device writer or reader runs.
pub enum WriterHandle {
    /// A child process (`osd --device-writer`, D-13).
    Process(std::process::Child),
    /// A thread of this process (tests): it, too, talks only via the ring.
    Thread(Option<std::thread::JoinHandle<i32>>),
}

impl WriterHandle {
    /// Whether the writer has ended.
    pub fn exited(&mut self) -> bool {
        match self {
            WriterHandle::Process(child) => !matches!(child.try_wait(), Ok(None)),
            WriterHandle::Thread(t) => t.as_ref().is_none_or(|t| t.is_finished()),
        }
    }

    /// Waits for the end of the writer.
    pub fn join(&mut self) {
        match self {
            WriterHandle::Process(child) => {
                let _ = child.wait();
            }
            WriterHandle::Thread(t) => {
                if let Some(t) = t.take() {
                    let _ = t.join();
                }
            }
        }
    }
}

impl Producer {
    /// `SetupDCR`/`TryReserveAfterUse` + `AcquireDeviceForAppend`: choose
    /// the device, create the ring, start the writer, wait until it has a
    /// volume, write the SOS label.
    fn setup(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel) -> Result<Producer, String> {
        d.send_status(dir, job, 'S');
        let storage = job.storage.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
        // a busy device: the job waits until it is free (step 1 of several jobs per device; bareos-sd
        // writes them interleaved into one volume, osd one after the other for now)
        let dev = loop {
            if let Some(dev) = d.reserve_device(&storage, &job.cmd) {
                break dev;
            }
            if !d.reservable_when_free(&storage, &job.cmd) {
                return Err("Could not reserve any device for this job.".into());
            }
            if job.canceled.load(Ordering::Acquire) {
                return Err("Job canceled while waiting for a device.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        };
        d.jmsg(dir, job, M_INFO, &format!("Using Device {} to write.\n", dev.device.print_name()));
        let info = JobInfo {
            job: job.cmd.job.clone(),
            job_id: job.cmd.job_id,
            vol_session_id: job.vol_session_id,
            vol_session_time: job.vol_session_time,
            pool_name: storage.pool_name.clone(),
            pool_type: storage.pool_type.clone(),
            media_type: dev.device.media_type.clone(),
            storage: d.config.storage.name.clone(),
            device: dev.device.name.clone(),
            archive_device: dev.device.archive_device.display().to_string(),
            label_media: dev.device.label_media,
            max_block_size: dev.device.max_block_size,
            label_block_size: dev.device.label_block_size,
            max_volume_size: dev.device.max_volume_size,
            job_name: job.cmd.job_name.clone(),
            client_name: job.cmd.client_name.clone(),
            fileset_name: job.cmd.fileset.clone(),
            fileset_md5: job.cmd.fileset_md5.clone(),
            job_type: job.cmd.job_type as u32,
            job_level: job.cmd.level as u32,
            bootstrap: String::new(),
            vol_poll_interval: d.config.find_device(&dev.device.name).map(vol_poll_interval).unwrap_or(300),
            alternatives: Vec::new(),
        };
        let result = Self::start(d, job, dir, &info, &dev.device, dev.slots);
        if result.is_err() {
            d.release_device(&dev.device.name);
        }
        result
    }

    /// The part of [`Producer::setup`] after the reservation.
    fn start(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, info: &JobInfo, dev: &FileDevice, slots: u32) -> Result<Producer, String> {
        let name = ring_name(d.config.storage.port, job.cmd.job_id);
        let _ = Ring::remove(&name);
        let cfg = RingConfig { slots, slot_size: dev.max_block_size, consumers: 1 };
        let ring = Arc::new(Ring::create(&name, cfg).map_err(|e| format!("Cannot create the job ring: {e}"))?);
        ring.set_info(info.encode().as_bytes()).map_err(|e| e.to_string())?;
        let ready = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let mail = {
            let (ring, ready, stop, dir, job) = (ring.clone(), ready.clone(), stop.clone(), dir.clone(), job.clone());
            let device = dev.name.clone();
            let daemon = d.clone();
            std::thread::Builder::new()
                .name(format!("mail:{}", job.cmd.job_id))
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        ring.serve_mail(Duration::from_millis(100), |_, req| match Request::decode(req) {
                            Some(Request::CatReq(line)) => daemon.cat_req(&dir, &line),
                            Some(Request::Jmsg(t, text)) => {
                                daemon.jmsg(&dir, &job, t, &text);
                                b"ok".to_vec()
                            }
                            Some(Request::Ready) => {
                                ready.store(true, Ordering::Release);
                                b"ok".to_vec()
                            }
                            Some(Request::WaitMedia(secs)) => daemon.wait_for_sysop(&dir, &job, &device, secs, 'm'),
                            Some(Request::JobStatus(c)) => {
                                daemon.send_status(&dir, &job, c);
                                b"ok".to_vec()
                            }
                            None => b"Ebad request".to_vec(),
                        });
                    }
                })
                .map_err(|e| e.to_string())?
        };
        let mut producer = Producer {
            writer: d.launch_writer(&name)?,
            ring: ring.clone(),
            mail: Some((mail, stop)),
            builder: BlockBuilder::new(dev.max_block_size as usize),
            block_size: dev.max_block_size as usize,
            published: 0,
            pending_attrs: VecDeque::new(),
        };
        while !ready.load(Ordering::Acquire) {
            if producer.writer.exited() || ring.consumer_state(crate::writer::CONSUMER) == State::Failed || job.canceled.load(Ordering::Acquire) {
                producer.shutdown(State::Failed);
                return Err(format!("Unable to setup device {} for this job.", dev.print_name()));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        d.send_status(dir, job, 'R');
        // WriteSessionLabel(SOS_LABEL)
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_micros() as i64;
        let sos = info.session_label(now).serialize();
        let rec = RecordRef {
            vol_session_id: job.vol_session_id,
            vol_session_time: job.vol_session_time,
            file_index: SOS_LABEL,
            stream: job.cmd.job_id as i32,
            data: &sos,
        };
        producer.write(&rec)?;
        Ok(producer)
    }

    /// `WriteRecord`: the record into blocks, full blocks into slots.
    fn write(&mut self, rec: &RecordRef<'_>) -> Result<(), String> {
        let mut progress = RecordProgress::default();
        while !self.builder.write_record(rec, &mut progress) {
            self.publish(SlotMeta { kind: SlotKind::Data as u64, ..Default::default() })?;
        }
        Ok(())
    }

    /// Puts the current block into the next slot with `meta` (kind and, for
    /// the End slot, the counters).
    fn publish(&mut self, mut meta: SlotMeta) -> Result<(), String> {
        let mut slot = self.ring.claim(CLAIM_TIMEOUT).map_err(|e| format!("device writer: {e}"))?;
        let payload = self.builder.payload();
        slot.data[..payload.len()].copy_from_slice(payload);
        meta.first_index = self.builder.first_index;
        meta.last_index = self.builder.last_index;
        meta.vol_session_id = self.builder.vol_session_id;
        meta.vol_session_time = self.builder.vol_session_time;
        slot.meta = meta.to_words();
        let len = payload.len();
        slot.publish(len);
        self.published += 1;
        self.builder = BlockBuilder::new(self.block_size);
        Ok(())
    }

    /// Sends the attributes whose block the writer has written.
    fn flush_attrs(&mut self, d: &Daemon, dir: &SharedChannel, all: bool) {
        let released = self.ring.released_up_to();
        while let Some((seq, _)) = self.pending_attrs.front() {
            if !all && *seq >= released {
                break;
            }
            let (_, msg) = self.pending_attrs.pop_front().unwrap_or_default();
            d.send_raw(dir, &msg);
        }
    }

    /// Ends the ring: producer state, mailbox server, writer, ring file.
    fn shutdown(&mut self, state: State) {
        self.ring.finish(state);
        // Keep serving the mailbox until the writer is done (it releases the
        // device with catalog requests).
        while !self.writer.exited() {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.writer.join();
        if let Some((t, stop)) = self.mail.take() {
            stop.store(true, Ordering::Release);
            let _ = t.join();
        }
        let _ = Ring::remove(self.ring.name());
    }
}

/// `DoAppendData`: receives the data of the FD and writes it through the
/// ring. True when everything went well.
fn do_append_data(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, fd: &mut dyn Channel, s: &mut Session, sd: bool) -> bool {
    let mut ok = true;
    let mut producer: Option<Producer> = None;
    let mut device_name: Option<String> = None;
    let jit = d.config.storage.just_in_time_reservation;
    let setup = |producer: &mut Option<Producer>, device_name: &mut Option<String>| -> bool {
        match Producer::setup(d, job, dir) {
            Ok(p) => {
                let read = job.read_device.lock().unwrap_or_else(|e| e.into_inner()).clone();
                *device_name = d.reserved_device_except(&job.cmd.job, read.as_deref());
                *producer = Some(p);
                true
            }
            Err(e) => {
                d.jmsg(dir, job, M_FATAL, &format!("{e}\n"));
                job.set_status('E');
                false
            }
        }
    };
    if !jit && !setup(&mut producer, &mut device_name) {
        return false;
    }
    if fd.send_str("3000 OK data\n").is_err() {
        d.jmsg(dir, job, M_FATAL, "Network send error to FD.\n");
        ok = false;
    }
    let what = if sd { "SD" } else { "FD" };
    let mut last_file_index: i64 = 0;
    let mut job_bytes: u64 = 0;
    let mut job_files: u32 = 0;
    'outer: while ok && !job.canceled.load(Ordering::Acquire) {
        // stream header
        let header = match fd.recv() {
            Ok(BnetMessage::Data(data)) => data,
            Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
            Ok(BnetMessage::Signal(sig)) => {
                d.jmsg(dir, job, M_FATAL, &format!("Unexpected signal from {what}: {}\n", sig as i32));
                ok = false;
                break;
            }
            Err(e) => {
                if !job.canceled.load(Ordering::Acquire) {
                    d.jmsg(dir, job, M_FATAL, &format!("Error reading data header from {what}. ERR={e}\n"));
                }
                ok = false;
                break;
            }
        };
        let Some((file_index, stream_id)) = bareos_format::record::parse_data_header(&header) else {
            d.jmsg(dir, job, M_FATAL, &format!("Malformed data header from {what}: {}\n", text(&header)));
            ok = false;
            break;
        };
        let rerun_start = job.cmd.rerunning && file_index > 0 && last_file_index == 0;
        let sequential = file_index > 0 && (file_index == last_file_index || file_index == last_file_index + 1);
        if !rerun_start && !sequential {
            d.jmsg(dir, job, M_FATAL, &format!("FileIndex={file_index} from {what} not positive or sequential={last_file_index}\n"));
            ok = false;
            break;
        }
        if file_index != last_file_index {
            last_file_index = file_index;
        }
        // data records up to EOD
        loop {
            if job.canceled.load(Ordering::Acquire) {
                break 'outer;
            }
            let data = match fd.recv() {
                Ok(BnetMessage::Data(data)) => data,
                Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
                Ok(BnetMessage::Signal(sig)) => {
                    d.jmsg(dir, job, M_FATAL, &format!("Unexpected signal from {what}: {}\n", sig as i32));
                    ok = false;
                    break 'outer;
                }
                Err(e) => {
                    if !job.canceled.load(Ordering::Acquire) {
                        d.jmsg(dir, job, M_FATAL, &format!("Network error reading from {what}. ERR={e}\n"));
                    }
                    ok = false;
                    break 'outer;
                }
            };
            if producer.is_none() {
                d.jmsg(dir, job, M_INFO, "JustInTime Reservation: Finding drive to reserve.\n");
                if !setup(&mut producer, &mut device_name) {
                    d.jmsg(dir, job, M_FATAL, "Unable to setup device for this job.\n");
                    ok = false;
                    break 'outer;
                }
            }
            let Some(p) = producer.as_mut() else { break 'outer };
            let rec = RecordRef {
                vol_session_id: job.vol_session_id,
                vol_session_time: job.vol_session_time,
                file_index: file_index as i32,
                stream: stream_id as i32,
                data: &data,
            };
            if let Err(e) = p.write(&rec) {
                d.jmsg(dir, job, M_FATAL, &format!("{e}\n"));
                ok = false;
                break 'outer;
            }
            job_bytes += data.len() as u64;
            *job.bytes.lock().unwrap_or_else(|e| e.into_inner()) = job_bytes;
            if is_attribute(stream_id as i32) && !job.cmd.no_attributes {
                let msg = osd::askdir::file_attributes(
                    &job.cmd.job,
                    job.vol_session_id,
                    job.vol_session_time,
                    file_index as i32,
                    stream_id as i32,
                    &data,
                );
                p.pending_attrs.push_back((p.published, msg));
            }
            job_files = file_index as u32;
            p.flush_attrs(d, dir, false);
        }
    }
    if job.canceled.load(Ordering::Acquire) {
        ok = false;
        job.set_status('A');
    }
    job.set_status(if ok { 'T' } else { 'E' });
    if ok && sd {
        // replicate data: back to the commands of the reading SD
        let _ = fd.send_str("3000 OK replicate data\n");
        do_sd_commands(d, job, dir, fd, s);
    } else if ok {
        let _ = fd.send_str("3000 OK append data\n");
        do_fd_commands(d, job, dir, fd, s);
    } else {
        let _ = fd.send_str("3999 Failed append\n");
    }
    job.files.store(job_files, Ordering::Release);
    if let Some(mut p) = producer {
        // EOS label and the last block: the writer appends the EOS label to
        // the partial block it gets in the End slot.
        let meta = SlotMeta {
            kind: SlotKind::End as u64,
            job_files,
            job_errors: 0,
            job_status: job.status() as u8,
            job_bytes,
            ..Default::default()
        };
        let ended = p.publish(meta);
        let state = if ended.is_ok() { State::Done } else { State::Failed };
        p.shutdown(state);
        if p.ring.consumer_state(crate::writer::CONSUMER) != State::Done {
            if ok && !job.canceled.load(Ordering::Acquire) {
                d.jmsg(dir, job, M_FATAL, "Fatal append error on device: the device writer failed.\n");
            }
            job.set_status('E');
            ok = false;
        }
        p.flush_attrs(d, dir, true);
        if let Some(dev) = device_name {
            d.release_device(&dev);
        }
    } else if ok {
        d.jmsg(
            dir,
            job,
            M_INFO,
            "Because no backup data was received, no device was reserved. As such no Session Labels were written for this job.\n",
        );
    }
    let answer = d.cat_req(dir, &osd::askdir::delete_null_jobmedia(&job.cmd.job, job.cmd.job_id));
    if !answer.starts_with(b"1000") {
        d.jmsg(dir, job, bareos_format::msg::M_WARNING, "Could not delete placeholder media records.\n");
    }
    let elapsed = s.started.elapsed().as_secs().max(1);
    d.jmsg(dir, job, M_INFO, &format!(
        "Elapsed time={:02}:{:02}:{:02}, Transfer rate={} Bytes/second\n",
        elapsed / 3600,
        elapsed % 3600 / 60,
        elapsed % 60,
        with_suffix(job_bytes / elapsed)
    ));
    d.send_status(dir, job, job.status());
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_like_is_attribute() {
        assert!(is_attribute(stream::STREAM_UNIX_ATTRIBUTES));
        assert!(is_attribute(stream::STREAM_MD5_DIGEST));
        assert!(is_attribute(stream::STREAM_RESTORE_OBJECT));
        assert!(!is_attribute(stream::STREAM_FILE_DATA));
        assert!(!is_attribute(stream::STREAM_GZIP_DATA));
    }
}

/// `Volume Poll Interval` of a device (seconds; Bareos default 300).
fn vol_poll_interval(dev: &bareos_config::sd::SdDevice) -> u64 {
    dev.other.iter().find(|d| d.norm_key() == "volumepollinterval")
        .and_then(|d| d.first())
        .and_then(|v| bareos_config::values::parse_time(v).ok())
        .map(|(_, t)| t.as_secs())
        .unwrap_or(300)
}
