// SPDX-License-Identifier: AGPL-3.0-or-later
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
    if conn.sd {
        do_sd_commands(d, job, dir, conn.channel.as_mut(), &mut session);
    } else {
        do_fd_commands(d, job, dir, conn.channel.as_mut(), &mut session);
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
        let dev = d.reserve_device(&storage, &job.cmd).ok_or("Could not reserve any device for this job.")?;
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
        // a job of a branch group (main job or branch): the ring is larger (D-25)
        let grouped = !job.branches.planned().is_empty() || job.branch_of.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let slots = match d.config.find_device(&dev.device.name) {
            Some(conf) if grouped => d.branch_ring_slots(conf),
            _ => dev.slots,
        };
        let result = Self::start(d, job, dir, &info, &dev.device, slots);
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

/// One receiver of the records of an append: the job itself (the first
/// sink) or a branch job attached to it. Every sink has its own ring, device
/// writer, volumes and catalog records; a failing sink ends alone (D-11).
struct Sink {
    /// The job the records are written for.
    job: Arc<Job>,
    /// The job's Director connection (messages, attributes, catalog requests).
    dir: SharedChannel,
    /// The ring side; `None` before the setup (just-in-time reservation) and
    /// after the sink was finished.
    producer: Option<Producer>,
    /// The device the job holds, released when the sink is finished.
    device_name: Option<String>,
    /// The sink gets no more records (it failed, was canceled or is done).
    ended: bool,
    /// [`finish_sink`] ran.
    finalized: bool,
    /// Data bytes so far (`JobBytes`).
    bytes: u64,
    /// The last FileIndex so far (`JobFiles`).
    files: u32,
    /// For a branch: signalled when the sink is finished.
    branch: Option<Arc<crate::jobs::BranchEnd>>,
}

impl Sink {
    /// A sink for `job` on its Director connection `dir`.
    fn new(job: Arc<Job>, dir: SharedChannel, branch: Option<Arc<crate::jobs::BranchEnd>>) -> Sink {
        Sink { job, dir, producer: None, device_name: None, ended: false, finalized: false, bytes: 0, files: 0, branch }
    }

    /// `SetupDCR` for the sink: device, ring, writer, volume. False (the
    /// sink ended, its job failed) with the reason sent to its job's log.
    fn setup(&mut self, d: &Daemon) -> bool {
        match Producer::setup(d, &self.job, &self.dir) {
            Ok(p) => {
                let read = self.job.read_device.lock().unwrap_or_else(|e| e.into_inner()).clone();
                self.device_name = d.reserved_device_except(&self.job.cmd.job, read.as_deref());
                self.producer = Some(p);
                true
            }
            Err(e) => {
                d.jmsg(&self.dir, &self.job, M_FATAL, &format!("{e}\n"));
                self.job.set_status('E');
                self.ended = true;
                false
            }
        }
    }

    /// The sink's job failed on its own with `reason` (a branch; the others go on).
    fn fail(&mut self, d: &Daemon, started: Instant, reason: &str) {
        d.jmsg(&self.dir, &self.job, M_FATAL, &format!("{reason}\n"));
        self.job.set_status('E');
        self.ended = true;
        finish_sink(d, self, started);
    }
}

/// Ends the sink's ring and reports the end of its job to the Director: the
/// End slot (the writer appends the EOS label), the attributes still waiting,
/// the device, the placeholder `JobMedia` records and the transfer rate. A
/// branch is released at last. True when the device writer ended well.
fn finish_sink(d: &Daemon, sink: &mut Sink, started: Instant) -> bool {
    if sink.finalized {
        return true;
    }
    sink.finalized = true;
    let (job, dir) = (sink.job.clone(), sink.dir.clone());
    let mut ok = true;
    job.files.store(sink.files, Ordering::Release);
    if let Some(mut p) = sink.producer.take() {
        // EOS label and the last block: the writer appends the EOS label to
        // the partial block it gets in the End slot.
        let meta = SlotMeta {
            kind: SlotKind::End as u64,
            job_files: sink.files,
            job_errors: 0,
            job_status: job.status() as u8,
            job_bytes: sink.bytes,
            ..Default::default()
        };
        let ended = p.publish(meta);
        let state = if ended.is_ok() { State::Done } else { State::Failed };
        p.shutdown(state);
        if p.ring.consumer_state(crate::writer::CONSUMER) != State::Done {
            if !job.canceled.load(Ordering::Acquire) {
                d.jmsg(&dir, &job, M_FATAL, "Fatal append error on device: the device writer failed.\n");
            }
            job.set_status('E');
            ok = false;
        }
        p.flush_attrs(d, &dir, true);
        if let Some(dev) = sink.device_name.take() {
            d.release_device(&dev);
        }
    } else if job.status() == 'T' {
        d.jmsg(
            &dir,
            &job,
            M_INFO,
            "Because no backup data was received, no device was reserved. As such no Session Labels were written for this job.\n",
        );
    }
    let answer = d.cat_req(&dir, &osd::askdir::delete_null_jobmedia(&job.cmd.job, job.cmd.job_id));
    if !answer.starts_with(b"1000") {
        d.jmsg(&dir, &job, bareos_format::msg::M_WARNING, "Could not delete placeholder media records.\n");
    }
    let elapsed = started.elapsed().as_secs().max(1);
    d.jmsg(&dir, &job, M_INFO, &format!(
        "Elapsed time={:02}:{:02}:{:02}, Transfer rate={} Bytes/second\n",
        elapsed / 3600,
        elapsed % 3600 / 60,
        elapsed % 60,
        with_suffix(sink.bytes / elapsed)
    ));
    d.send_status(&dir, &job, job.status());
    if let Some(end) = &sink.branch {
        *job.bytes.lock().unwrap_or_else(|e| e.into_inner()) = sink.bytes;
        end.complete(job.status());
    }
    ok
}

/// The branches a main job waits for before it writes (`BranchPlan`): every
/// planned one has attached or is gone (its `use storage` failed, it was
/// canceled), or `Branch Attach Wait` passed. A branch that is missing ends
/// without data: the main job's log says why.
fn collect_branches(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel) -> Vec<crate::jobs::AttachedBranch> {
    if job.branches.planned().is_empty() {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    let wait = Duration::from_secs(d.config.storage.branch_attach_wait);
    let (attached, missing) = job.branches.collect(wait, |name| match d.jobs.find(name) {
        Some(j) => {
            seen.insert(name.to_string());
            if j.canceled.load(Ordering::Acquire) {
                Some("it was canceled".to_string())
            } else if matches!(j.status(), 'E' | 'f') {
                Some("its job failed".to_string())
            } else {
                None
            }
        }
        None => seen.contains(name).then(|| "its job ended before it attached".to_string()),
    });
    for (name, why) in missing {
        d.jmsg(dir, job, bareos_format::msg::M_WARNING, &format!("Branch job {name} is not written: {why}.\n"));
    }
    for b in &attached {
        d.jmsg(dir, job, M_INFO, &format!("Branch job {} (JobId {}) takes this stream.\n", b.job.cmd.job, b.job.cmd.job_id));
    }
    attached
}

/// `DoAppendData`: receives the data of the FD and writes it through the
/// ring of every sink: the job's own and those of its branches. True when
/// everything went well for the job itself.
fn do_append_data(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, fd: &mut dyn Channel, s: &mut Session, sd: bool) -> bool {
    let mut ok = true;
    let mut sinks = vec![Sink::new(job.clone(), dir.clone(), None)];
    if !sd {
        sinks.extend(collect_branches(d, job, dir).into_iter().map(|b| Sink::new(b.job, b.dir, Some(b.end))));
    }
    let jit = d.config.storage.just_in_time_reservation;
    let started = s.started;
    // `SetupDCR` for every sink; false when none is left
    let setup_all = |sinks: &mut Vec<Sink>| -> bool {
        for sink in sinks.iter_mut() {
            sink.setup(d);
        }
        sinks.iter().any(|s| !s.ended)
    };
    if !jit && !setup_all(&mut sinks) {
        // nothing is written: the branches end, the job itself fails as before
        for sink in sinks.iter_mut().skip(1) {
            finish_sink(d, sink, started);
        }
        return false;
    }
    if fd.send_str("3000 OK data\n").is_err() {
        d.jmsg(dir, job, M_FATAL, "Network send error to FD.\n");
        ok = false;
    }
    let what = if sd { "SD" } else { "FD" };
    let mut setup_done = !jit;
    let mut last_file_index: i64 = 0;
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
            if !setup_done {
                d.jmsg(dir, job, M_INFO, "JustInTime Reservation: Finding drive to reserve.\n");
                setup_done = true;
                if !setup_all(&mut sinks) {
                    d.jmsg(dir, job, M_FATAL, "Unable to setup device for this job.\n");
                    ok = false;
                    break 'outer;
                }
            }
            for sink in sinks.iter_mut().filter(|s| !s.ended) {
                if sink.job.canceled.load(Ordering::Acquire) {
                    // a branch canceled on its own: its job ends, the others go on
                    sink.ended = true;
                    finish_sink(d, sink, started);
                    continue;
                }
                let rec = RecordRef {
                    vol_session_id: sink.job.vol_session_id,
                    vol_session_time: sink.job.vol_session_time,
                    file_index: file_index as i32,
                    stream: stream_id as i32,
                    data: &data,
                };
                let Some(p) = sink.producer.as_mut() else { continue };
                if let Err(e) = p.write(&rec) {
                    sink.fail(d, started, &e);
                    continue;
                }
                sink.bytes += data.len() as u64;
                *sink.job.bytes.lock().unwrap_or_else(|e| e.into_inner()) = sink.bytes;
                if is_attribute(stream_id as i32) && !sink.job.cmd.no_attributes {
                    let msg = osd::askdir::file_attributes(
                        &sink.job.cmd.job,
                        sink.job.vol_session_id,
                        sink.job.vol_session_time,
                        file_index as i32,
                        stream_id as i32,
                        &data,
                    );
                    p.pending_attrs.push_back((p.published, msg));
                }
                sink.files = file_index as u32;
                p.flush_attrs(d, &sink.dir, false);
            }
            if sinks.iter().all(|s| s.ended) {
                ok = false;
                break 'outer;
            }
        }
    }
    if job.canceled.load(Ordering::Acquire) {
        ok = false;
        job.set_status('A');
    }
    for sink in sinks.iter().filter(|s| !s.ended) {
        sink.job.set_status(if ok { 'T' } else { 'E' });
    }
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
    // the branches first, then the job: its end is what the Director waits for
    for sink in sinks.iter_mut().skip(1) {
        finish_sink(d, sink, started);
    }
    if let Some(main) = sinks.first_mut() {
        if !finish_sink(d, main, started) {
            ok = false;
        }
    }
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
