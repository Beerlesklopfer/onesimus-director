// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Die Netzseite des Lesens: `read data` – Blöcke aus dem Ring, Datensätze an den FD.
//!
//! Zweck: Bei Restore und Verify (VolumeToCatalog) schickt der SD dem FD die
//! gesicherten Datensätze. osd liest dafür über den Ring in Gegenrichtung:
//! [`do_read_data`] legt den Ring an, meldet sich als **Verbraucher** an und
//! startet den Geräteleser ([`crate::reader`], eigener Prozess), der die
//! Blöcke der Volumes in die Fächer legt. Die Netzsitzung setzt daraus die
//! Datensätze zusammen (je Sitzung, auch über Volume-Grenzen hinweg),
//! prüft jeden gegen den Bootstrap ([`bareos_format::bsr::BsrMatcher`]) und
//! schickt die passenden an den FD: je Datensatz eine Kopfzeile
//! `rechdr <VolSessionId> <VolSessionTime> <FileIndex> <Stream> <Länge>`,
//! dann die Daten; am Ende ein EOD. Labels (negativer FileIndex) gehen
//! nicht an den FD. Die Anfragen des Lesers (CatReq, Job-Meldungen)
//! beantwortet ein eigener Faden mit der Director-Verbindung.
//!
//! Ablauf wie bareos-sd: Volume belegen (Leser meldet „bereit“), `3000 OK
//! data` an den FD, Job-Status `R`, Datensätze, EOD, Device freigeben. Ist
//! kein Volume genannt oder scheitert das Belegen, bekommt der FD `3000
//! error` und der Job endet mit Fehler.
//!
//! Bareos-Gegenstück: `core/src/stored/read.cc` (`DoReadData`,
//! `RecordCb`), `stored/read_record.cc` (`ReadRecords`,
//! `ReadNextRecordFromBlock`, `ReadContextSetRecord`), `stored/bsr.cc`
//! (`MatchBsr`, `IsThisBsrDone`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bareos_bnet::BnetSignal;
use bareos_format::bsr::{BsrMatcher, Match, RecordKey, RestoreVolume};
use bareos_format::label::{SessionLabel, SOS_LABEL};
use bareos_format::msg::{M_FATAL, M_INFO};
use bareos_format::record::RecHdr;
use bareos_format::volume::{RecordAssembler, DEFAULT_BLOCK_SIZE};
use osd_ring::{Ring, RingConfig, RingError, State};

use crate::daemon::Daemon;
use crate::device::FileDevice;
use crate::fd_session::WriterHandle;
use crate::jobs::{Bootstrap, Job};
use crate::reader::CONSUMER;
use crate::ringproto::{ring_name, BlockMeta, JobInfo, Request};
use crate::transport::{Channel, SharedChannel};

/// `FD_error` of `stored/read.cc`.
const FD_ERROR: &str = "3000 error\n";

/// Set in the JobId of the read ring of a copy inside one SD (the write ring keeps the JobId).
const READ_RING_FLAG: u32 = 0x8000_0000;

/// Smallest ring slot for reading: a volume may hold blocks up to the
/// `Maximum Block Size` it was written with (osd and bareos-sd write up to
/// 1 MiB on a File device by default).
const MIN_READ_SLOT: u32 = 1 << 20;

/// How long one wait for the next slot lasts before cancel and the reader
/// are checked again.
const POLL: Duration = Duration::from_millis(500);

/// The ring of one read job with its reader and mailbox server.
struct ReadPipe {
    /// The ring.
    ring: Arc<Ring>,
    /// The device reader.
    reader: WriterHandle,
    /// The mailbox server thread and its stop flag.
    mail: Option<(std::thread::JoinHandle<()>, Arc<AtomicBool>)>,
    /// Set when the reader reported the first volume ready.
    ready: Arc<AtomicBool>,
}

impl ReadPipe {
    /// Creates the ring, attaches as consumer, starts the mailbox server
    /// and the reader.
    fn start(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, info: &JobInfo, slots: u32, slot_size: u32) -> Result<ReadPipe, String> {
        // a copy inside one SD also writes: its ring has the JobId, the read ring another name
        let local = job.local_write.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        let name = ring_name(d.config.storage.port, job.cmd.job_id | if local { READ_RING_FLAG } else { 0 });
        let _ = Ring::remove(&name);
        let cfg = RingConfig { slots, slot_size, consumers: 1 };
        let ring = Arc::new(Ring::create(&name, cfg).map_err(|e| format!("Cannot create the job ring: {e}"))?);
        ring.set_info(info.encode().as_bytes()).map_err(|e| e.to_string())?;
        ring.attach(CONSUMER).map_err(|e| e.to_string())?;
        let ready = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let mail = {
            let (ring, ready, stop, dir, job, daemon) = (ring.clone(), ready.clone(), stop.clone(), dir.clone(), job.clone(), d.clone());
            let device = info.device.clone();
            std::thread::Builder::new()
                .name(format!("rmail:{}", job.cmd.job_id))
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        ring.serve_producer(CONSUMER, Duration::from_millis(100), |req| match Request::decode(req) {
                            Some(Request::CatReq(line)) => daemon.cat_req(&dir, &line),
                            Some(Request::Jmsg(t, text)) => {
                                daemon.jmsg(&dir, &job, t, &text);
                                b"ok".to_vec()
                            }
                            Some(Request::Ready) => {
                                ready.store(true, Ordering::Release);
                                b"ok".to_vec()
                            }
                            // a volume is not there: wait for `mount`/`label` on the device
                            Some(Request::WaitMedia(secs)) => daemon.wait_for_sysop(&dir, &job, &device, secs, 'M'),
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
        let reader = match d.launch_reader(&name) {
            Ok(r) => r,
            Err(e) => {
                stop.store(true, Ordering::Release);
                let _ = mail.join();
                let _ = Ring::remove(&name);
                return Err(e);
            }
        };
        Ok(ReadPipe { ring, reader, mail: Some((mail, stop)), ready })
    }

    /// Waits until the reader has the first volume (true) or failed.
    fn wait_ready(&mut self, job: &Job) -> bool {
        loop {
            if self.ready.load(Ordering::Acquire) {
                return true;
            }
            if self.reader.exited() || self.ring.producer_state() == State::Failed || job.canceled.load(Ordering::Acquire) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Ends the read: the session needs no more slots, the reader releases
    /// the device (served by the mailbox thread until it exits), the ring
    /// goes away. True when the reader ended normally.
    fn shutdown(&mut self) -> bool {
        self.ring.set_consumer_state(CONSUMER, State::Done);
        while !self.reader.exited() {
            std::thread::sleep(Duration::from_millis(20));
        }
        self.reader.join();
        if let Some((t, stop)) = self.mail.take() {
            stop.store(true, Ordering::Release);
            let _ = t.join();
        }
        let _ = Ring::remove(self.ring.name());
        self.ring.producer_state() == State::Done
    }
}

/// `DoReadData`: reads the job's volumes and sends the wanted records to
/// the FD. False when the job failed (the reason went out as a job
/// message).
pub fn do_read_data(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, fd: &mut dyn Channel) -> bool {
    let Some((mut pipe, bsr, volumes)) = open_read(d, job, dir) else {
        let _ = fd.send_str(FD_ERROR);
        return false;
    };
    if fd.send_str("3000 OK data\n").is_err() {
        pipe.shutdown();
        return false;
    }
    d.send_status(dir, job, 'R');
    let mut sink = FdSink { fd };
    let mut ok = send_records(d, job, dir, &mut sink, &mut pipe, BsrMatcher::new(bsr), &volumes);
    let _ = fd.signal(BnetSignal::Eod);
    if !pipe.shutdown() {
        ok = false;
    }
    ok
}

/// The reading side of a Copy/Migrate (`DoMacRun`, stored/mac.cc): reads the
/// job's volumes and sends the wanted records to the writing SD `sd` the
/// way an FD sends a backup: `start replicate`, `replicate data <ticket>`,
/// the records under a `<FileIndex> <Stream> 0` header, two EODs, `end
/// replicate`. Returns the records' file count and byte count, or `None`
/// when the job failed (the reason went out as a job message).
pub fn do_replicate_read(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, sd: &mut dyn Channel) -> Option<(u64, u64)> {
    do_replicate_read_with(d, job, dir, sd, &mut || {})
}

/// [`do_replicate_read`] that calls `opened` once the volumes are claimed.
fn do_replicate_read_with(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, sd: &mut dyn Channel, opened: &mut dyn FnMut()) -> Option<(u64, u64)> {
    let fail = |text: &str| {
        d.jmsg(dir, job, M_FATAL, text);
        None
    };
    let (mut pipe, bsr, volumes) = open_read(d, job, dir)?;
    opened();
    let ticket = match sd_dialog(sd, "start replicate\n", "3000 OK start replicate ticket = ") {
        Ok(rest) => rest.trim().to_string(),
        Err(e) => {
            pipe.shutdown();
            return fail(&format!("Start replicate failed: {e}\n"));
        }
    };
    if let Err(e) = sd_dialog(sd, &format!("replicate data {ticket}\n"), "3000 OK data") {
        pipe.shutdown();
        return fail(&format!("Replicate data failed: {e}\n"));
    }
    d.send_status(dir, job, 'R');
    // a VirtualFull job (type Backup) reads several jobs into one: the file indexes are renumbered
    let renumber = job.cmd.job_type == 'B';
    let mut sink = SdSink { sd, last: None, files: 0, bytes: 0, renumber, source: None };
    let mut ok = send_records(d, job, dir, &mut sink, &mut pipe, BsrMatcher::new(bsr), &volumes);
    let (files, bytes) = (sink.files, sink.bytes);
    // end of the stream and of the data: two EODs
    if ok && (sink.finish().is_err()) {
        ok = false;
    }
    if !pipe.shutdown() {
        ok = false;
    }
    if !ok {
        return None;
    }
    if let Err(e) = sd_wait(sd, "3000 OK replicate data") {
        return fail(&format!("The writing Storage daemon did not accept the data: {e}\n"));
    }
    if let Err(e) = sd_dialog(sd, "end replicate\n", "3000 OK end replicate") {
        return fail(&format!("End replicate failed: {e}\n"));
    }
    let _ = sd.signal(BnetSignal::Terminate);
    Some((files, bytes))
}

/// The read side of a copy inside one SD: like [`do_replicate_read`] with
/// the writing side on the other end of `sd` (in this process). The write
/// storage replaces the job's storage once the volumes are claimed; `go`
/// tells the write side to start then.
pub fn do_local_read(
    d: &Daemon,
    job: &Arc<Job>,
    dir: &SharedChannel,
    sd: &mut dyn Channel,
    write: Option<crate::jobs::UseStorage>,
    go: &std::sync::mpsc::Sender<()>,
) -> Option<(u64, u64)> {
    // the volumes of the read side are claimed inside do_replicate_read; the
    // write side is released there (before the first record) through `go`
    do_replicate_read_with(d, job, dir, sd, &mut || {
        *job.storage.lock().unwrap_or_else(|e| e.into_inner()) = write.clone();
        let _ = go.send(());
    })
}

/// Sends `command` to the writing SD and waits for an answer that starts
/// with `expect`; the rest of the line is returned.
fn sd_dialog(sd: &mut dyn Channel, command: &str, expect: &str) -> Result<String, String> {
    sd.send_str(command)?;
    sd_wait(sd, expect)
}

/// The next text message of the writing SD, which must start with `expect`.
fn sd_wait(sd: &mut dyn Channel, expect: &str) -> Result<String, String> {
    loop {
        match sd.recv()? {
            crate::transport::Message::Data(data) if !data.is_empty() => {
                let text = String::from_utf8_lossy(&data).to_string();
                return match text.strip_prefix(expect) {
                    Some(rest) => Ok(rest.to_string()),
                    None => Err(text.trim_end().to_string()),
                };
            }
            _ => continue,
        }
    }
}

/// Where the wanted records go: the FD of a restore or the writing SD of a Copy.
trait RecordSink {
    /// Sends one record.
    fn record(&mut self, rec: &bareos_format::volume::VolRecord) -> Result<(), String>;
}

/// The records of a restore: `rechdr` and the data to the FD.
struct FdSink<'a> {
    fd: &'a mut dyn Channel,
}

impl RecordSink for FdSink<'_> {
    fn record(&mut self, rec: &bareos_format::volume::VolRecord) -> Result<(), String> {
        let hdr = RecHdr {
            vol_session_id: rec.vol_session_id,
            vol_session_time: rec.vol_session_time,
            file_index: rec.file_index,
            stream: rec.stream,
            data_len: rec.data.len() as u32,
        };
        self.fd.send_str(&hdr.format()).and_then(|_| self.fd.send(&rec.data))
    }
}

/// The records of a Copy: like an FD's backup stream to the writing SD. A
/// `<FileIndex> <Stream> 0` header starts each run of records of one
/// (FileIndex, Stream), a change of either ends the run with an EOD.
struct SdSink<'a> {
    sd: &'a mut dyn Channel,
    /// (FileIndex, Stream) of the open run.
    last: Option<(i32, i32)>,
    /// The highest FileIndex sent.
    files: u64,
    /// Bytes of data sent.
    bytes: u64,
    /// Number the files anew (several jobs go into one, as for a VirtualFull job).
    renumber: bool,
    /// (VolSessionId, VolSessionTime, FileIndex) of the file being copied, when renumbering.
    source: Option<(u32, u32, i32)>,
}

impl SdSink<'_> {
    /// Ends the open run and the data (two EODs).
    fn finish(&mut self) -> Result<(), String> {
        if self.last.take().is_some() {
            self.sd.signal(BnetSignal::Eod)?;
        }
        self.sd.signal(BnetSignal::Eod)
    }
}

impl RecordSink for SdSink<'_> {
    fn record(&mut self, rec: &bareos_format::volume::VolRecord) -> Result<(), String> {
        let mut file_index = rec.file_index;
        if self.renumber {
            // mac.cc RecordCb: a new (job, FileIndex) is the next file of the new job
            let source = (rec.vol_session_id, rec.vol_session_time, rec.file_index);
            if self.source != Some(source) {
                self.source = Some(source);
                self.files += 1;
            }
            file_index = self.files as i32;
        }
        let key = (file_index, rec.stream);
        if self.last != Some(key) {
            if self.last.is_some() {
                self.sd.signal(BnetSignal::Eod)?;
            }
            self.sd.send_str(&format!("{} {} 0", file_index, rec.stream))?;
            self.last = Some(key);
        }
        if !self.renumber {
            self.files = self.files.max(rec.file_index.max(0) as u64);
        }
        self.bytes += rec.data.len() as u64;
        if rec.data.is_empty() {
            return Ok(());
        }
        self.sd.send(&rec.data)
    }
}

/// Claims the job's volumes: ring, reader process, first volume ready. The
/// reason of a failure went out as a job message.
fn open_read(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel) -> Option<(ReadPipe, bareos_format::bsr::Bsr, Vec<RestoreVolume>)> {
    let bootstrap = job.bootstrap.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let volumes = bootstrap.as_ref().map(|b| b.bsr.restore_volumes()).unwrap_or_default();
    let (Some(bootstrap), false) = (bootstrap, volumes.is_empty()) else {
        d.jmsg(dir, job, M_FATAL, "No Volume names found for restore.\n");
        return None;
    };
    let read_device = job.read_device.lock().unwrap_or_else(|e| e.into_inner()).clone().or_else(|| d.reserved_device(&job.cmd.job));
    let device = read_device.and_then(|n| d.config.find_device(&n).cloned());
    let Some(mut conf) = device else {
        d.jmsg(dir, job, M_FATAL, "No device reserved for reading. Job canceled.\n");
        return None;
    };
    // The volumes are of another media type than the device the Director named (a restore with
    // `storage=` of another media type): read from a free device of this SD that has it
    // (`Changing read device`, reserve.cc)
    let want = volumes[0].media_type.clone();
    if !want.is_empty() && want != conf.media_type && volumes.iter().all(|v| v.media_type == want) {
        d.jmsg(dir, job, M_INFO, &format!(
            "Changing read device. Want Media Type=\"{want}\" have=\"{}\"\n  device={}\n",
            conf.media_type, conf.name
        ));
        let any = crate::jobs::UseStorage {
            media_type: want.clone(),
            devices: d.config.devices.iter().map(|x| x.name.clone()).collect(),
            ..Default::default()
        };
        let old = conf.name.clone();
        let Some(r) = d.reserve_device(&any, &job.cmd.job) else {
            d.jmsg(dir, job, M_FATAL, &format!("No suitable device found to read Volume \"{}\"\n", volumes[0].name));
            return None;
        };
        d.release_device(&old);
        *job.read_device.lock().unwrap_or_else(|e| e.into_inner()) = Some(r.device.name.clone());
        match d.config.find_device(&r.device.name) {
            Some(c) => conf = c.clone(),
            None => return None,
        }
        d.jmsg(dir, job, M_INFO, &format!("Media Type change.  New read device \"{}\" ({}) chosen.\n", conf.name, conf.archive_device));
    }
    let dev = FileDevice::from_config(&conf);
    let info = read_info(d, job, &dev, &bootstrap);
    let slot_size = dev.max_block_size.max(dev.label_block_size).max(DEFAULT_BLOCK_SIZE as u32).max(MIN_READ_SLOT);
    let mut pipe = match ReadPipe::start(d, job, dir, &info, d.ring_slots(&conf), slot_size) {
        Ok(p) => p,
        Err(e) => {
            d.jmsg(dir, job, M_FATAL, &format!("{e}\n"));
            return None;
        }
    };
    if !pipe.wait_ready(job) {
        pipe.shutdown();
        return None;
    }
    Some((pipe, bootstrap.bsr, volumes))
}

/// The ring info of a read job.
fn read_info(d: &Daemon, job: &Job, dev: &FileDevice, bootstrap: &Bootstrap) -> JobInfo {
    let storage = job.storage.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
    JobInfo {
        job: job.cmd.job.clone(),
        job_id: job.cmd.job_id,
        vol_session_id: job.vol_session_id,
        vol_session_time: job.vol_session_time,
        pool_name: storage.pool_name,
        pool_type: storage.pool_type,
        media_type: dev.media_type.clone(),
        storage: d.config.storage.name.clone(),
        device: dev.name.clone(),
        archive_device: dev.archive_device.display().to_string(),
        label_media: dev.label_media,
        max_block_size: dev.max_block_size,
        label_block_size: dev.label_block_size,
        max_volume_size: dev.max_volume_size,
        job_name: job.cmd.job_name.clone(),
        client_name: job.cmd.client_name.clone(),
        fileset_name: job.cmd.fileset.clone(),
        fileset_md5: job.cmd.fileset_md5.clone(),
        job_type: job.cmd.job_type as u32,
        job_level: job.cmd.level as u32,
        bootstrap: bootstrap.path.display().to_string(),
        vol_poll_interval: 300,
    }
}

/// What to do with one complete record (`ReadNextRecordFromBlock` and
/// `RecordCb`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Send it to the FD.
    Send,
    /// Not wanted.
    Skip,
    /// Not wanted, and nothing more on this volume is (`IsThisBsrDone` with
    /// `TryDeviceRepositioning` towards the next volume).
    SkipVolume,
    /// The restore is complete (`MatchBsr` returned -1).
    Done,
}

/// The record filter of a read session: bootstrap state, SOS labels per
/// session and the FileIndex of the previous record in the block.
pub struct RecordFilter {
    /// The bootstrap with its "done" state.
    matcher: BsrMatcher,
    /// SOS labels by (`VolSessionId`, `VolSessionTime`).
    sessions: HashMap<(u32, u32), SessionLabel>,
    /// `rctx->lastFileIndex`: reset at every block.
    last_file_index: Option<i32>,
}

impl RecordFilter {
    /// A filter for `matcher`.
    pub fn new(matcher: BsrMatcher) -> RecordFilter {
        RecordFilter { matcher, sessions: HashMap::new(), last_file_index: None }
    }

    /// A new block starts (`READ_NO_FILEINDEX`).
    pub fn start_block(&mut self) {
        self.last_file_index = None;
    }

    /// The decision for one complete record read from `volume`, from the
    /// block whose last byte is at `address`.
    pub fn decide(&mut self, volume: &str, address: u64, rec: &bareos_format::volume::VolRecord) -> Decision {
        if rec.file_index < 0 {
            if rec.file_index == SOS_LABEL
                && let Ok(l) = SessionLabel::parse(&rec.data, false)
            {
                self.sessions.insert((rec.vol_session_id, rec.vol_session_time), l);
            }
            return Decision::Skip;
        }
        let key = RecordKey {
            volume,
            address,
            vol_session_id: rec.vol_session_id,
            vol_session_time: rec.vol_session_time,
            file_index: rec.file_index,
            stream: rec.stream,
            data: &rec.data,
        };
        let session = self.sessions.get(&(rec.vol_session_id, rec.vol_session_time));
        match self.matcher.match_record(&key, session) {
            Match::Done => Decision::Done,
            Match::No => Decision::Skip,
            Match::Yes(entry) => {
                if let (Some(last), Some(entry)) = (self.last_file_index, entry)
                    && last != rec.file_index
                    && self.matcher.file_done(entry)
                    && self.matcher.next_address(volume).is_none()
                {
                    return Decision::SkipVolume;
                }
                self.last_file_index = Some(rec.file_index);
                Decision::Send
            }
        }
    }
}

/// `ReadRecords` with `RecordCb`: drains the ring and sends every wanted
/// record to the FD. False on an error (job message sent).
fn send_records(
    d: &Daemon,
    job: &Arc<Job>,
    dir: &SharedChannel,
    sink: &mut dyn RecordSink,
    pipe: &mut ReadPipe,
    matcher: BsrMatcher,
    volumes: &[RestoreVolume],
) -> bool {
    let mut filter = RecordFilter::new(matcher);
    let mut assembler = RecordAssembler::new();
    let mut skip_volume: Option<u32> = None;
    let mut block = Vec::new();
    loop {
        if job.canceled.load(Ordering::Acquire) {
            return false;
        }
        let meta = match pipe.ring.next(CONSUMER, POLL) {
            Ok(Some(slot)) => {
                block.clear();
                block.extend_from_slice(slot.data);
                let meta = BlockMeta::from_words(slot.meta);
                pipe.ring.release(CONSUMER);
                meta
            }
            Ok(None) => return true,
            Err(RingError::Timeout) => {
                if pipe.reader.exited() && pipe.ring.producer_state() != State::Done {
                    return false;
                }
                continue;
            }
            // the reader failed and said why
            Err(_) => return false,
        };
        let Some(meta) = meta else {
            d.jmsg(dir, job, M_FATAL, "Read error: ring slot of unknown kind.\n");
            return false;
        };
        if skip_volume == Some(meta.volume) {
            continue;
        }
        let Some(volume) = volumes.get(meta.volume as usize) else {
            d.jmsg(dir, job, M_FATAL, &format!("Read error: volume {} not in the restore list.\n", meta.volume));
            return false;
        };
        let (header, records) = match assembler.push_block(&block, meta.address, false) {
            Ok(r) => r,
            Err(e) => {
                d.jmsg(dir, job, M_FATAL, &format!("Volume data error at 0:{} on Volume \"{}\"! {e}\n", meta.address, volume.name));
                return false;
            }
        };
        let last_byte = meta.address + u64::from(header.block_len) - 1;
        filter.start_block();
        for rec in records {
            match filter.decide(&volume.name, last_byte, &rec) {
                Decision::Skip => continue,
                Decision::SkipVolume => {
                    skip_volume = Some(meta.volume);
                    break;
                }
                Decision::Done => return true,
                Decision::Send => {
                    if let Err(e) = sink.record(&rec) {
                        if !job.canceled.load(Ordering::Acquire) {
                            d.jmsg(dir, job, M_FATAL, &format!("Error sending to File daemon. ERR={e}\n"));
                        }
                        return false;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bareos_format::bsr::Bsr;
    use bareos_format::volume::VolRecord;

    /// A record of session 1/9 with `file_index` and `stream`.
    fn rec(file_index: i32, stream: i32) -> VolRecord {
        VolRecord { block_number: 1, block_offset: 0, vol_session_id: 1, vol_session_time: 9, file_index, stream, data: vec![1, 2] }
    }

    #[test]
    fn only_wanted_file_indexes_are_sent_and_labels_never() {
        let bsr = Bsr::parse("Volume=V\nVolSessionId=1\nVolSessionTime=9\nVolAddr=0-1000\nFileIndex=2-3\nCount=2\n").unwrap();
        let mut f = RecordFilter::new(BsrMatcher::new(bsr));
        f.start_block();
        let sos = SessionLabel { job_id: 4, ..Default::default() };
        let label = VolRecord { data: sos.serialize(), ..rec(SOS_LABEL, 4) };
        assert_eq!(f.decide("V", 99, &label), Decision::Skip);
        assert_eq!(f.decide("V", 99, &rec(1, 1)), Decision::Skip);
        assert_eq!(f.decide("V", 99, &rec(2, 1)), Decision::Send);
        assert_eq!(f.decide("V", 99, &rec(2, 2)), Decision::Send);
        assert_eq!(f.decide("V", 99, &rec(3, 1)), Decision::Send);
        assert_eq!(f.decide("W", 99, &rec(3, 2)), Decision::Skip, "other volume");
        assert_eq!(f.decide("V", 99, &rec(4, 1)), Decision::Done);
    }

    #[test]
    fn a_file_continued_on_the_next_volume_is_sent_from_both() {
        let text = "Volume=V1\nVolSessionId=1\nVolSessionTime=9\nVolAddr=0-1000\nFileIndex=1-2\nCount=2\n\
                    Volume=V2\nVolSessionId=1\nVolSessionTime=9\nVolAddr=0-1000\nFileIndex=2-3\nCount=2\n";
        let mut f = RecordFilter::new(BsrMatcher::new(Bsr::parse(text).unwrap()));
        f.start_block();
        assert_eq!(f.decide("V1", 500, &rec(1, 1)), Decision::Send);
        assert_eq!(f.decide("V1", 500, &rec(2, 1)), Decision::Send);
        f.start_block();
        assert_eq!(f.decide("V2", 300, &rec(2, 2)), Decision::Send, "rest of file 2 on the second volume");
        assert_eq!(f.decide("V2", 300, &rec(3, 1)), Decision::Send);
        // past the last range of V2; the V1 section is not done (its volume
        // no longer matches), so bareos-sd answers "no match" here, not -1
        assert_eq!(f.decide("V2", 1300, &rec(3, 2)), Decision::Skip);
    }
}
