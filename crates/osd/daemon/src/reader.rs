// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Der Geräteleser: liest die Volumes eines Restores und füllt den Ring.
//!
//! Zweck: Restore und Verify (VolumeToCatalog) lesen über denselben Ring wie
//! das Backup, nur in Gegenrichtung (Plan „Datenpfad: Ringpuffer in
//! /dev/shm“): Der Geräteleser ist ein eigener Prozess (`osd
//! --device-reader <Ring>`, D-13) und der **Erzeuger**, die Netzsitzung
//! ([`crate::read_session`]) ist der Verbraucher. Der Leser nimmt die
//! Volumes in der Reihenfolge des Bootstraps, prüft je Volume den Katalog
//! (`GetVolInfo … write=0`) und das Volume-Label, springt an den Anfang
//! jedes Adressbereichs (`VolAddr`) und legt jeden Block dieser Bereiche
//! unverändert in ein Fach. Welche Datensätze an den FD gehen, entscheidet
//! die Netzsitzung. Am Ende eines Volumes geht es auf dem nächsten weiter;
//! danach gibt er das Device frei und meldet dem Katalog die Volume-Daten
//! (`UpdateMedia`).
//!
//! Meldungen wie bareos-sd: „Ready to read from volume …“, „Forward
//! spacing Volume …“, „End of Volume at file … on device …“, „End of all
//! volumes.“, „Releasing device …“; bei falschem Volume „Read acquire:
//! Wrong Volume mounted on device …: Wanted … have …“ und „Too many errors
//! trying to mount device … for reading.“
//!
//! Der Leser spricht nur über den Ring: Fächer mit Blöcken, Anfragen (CatReq,
//! Job-Meldungen, „bereit“) mit `Ring::ask_consumer` an die Netzsitzung.
//!
//! Bareos-Gegenstück: `core/src/stored/acquire.cc`
//! (`AcquireDeviceForRead`, `ReleaseDevice`), `stored/mount.cc`
//! (`MountNextReadVolume`), `stored/read_record.cc`
//! (`ReadNextBlockFromDevice`), `stored/device.cc`
//! (`PositionDeviceToFirstFile`, `TryDeviceRepositioning`),
//! `stored/label.cc` (`ReadDevVolumeLabel`), `stored/askdir.cc`
//! (`DirGetVolumeInfo`, `DirUpdateVolumeInfo`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use bareos_format::bsr::{Bsr, RestoreVolume};
use bareos_format::msg::{M_ERROR, M_FATAL, M_INFO, M_MOUNT, M_WARNING};
use bareos_format::volume::{verify_block, BlockHeader, BLKHDR2_LENGTH};
use osd::askdir::VolumeInfo;
use osd_ring::{Ring, RingError, State};

use crate::device::{FileDevice, LabelState};
use crate::ringproto::{BlockMeta, JobInfo, Request};

/// The network session is consumer 0 of a read ring.
pub const CONSUMER: usize = 0;

/// Longest wait for an answer of the network session.
const ASK_TIMEOUT: Duration = Duration::from_secs(1800);

/// Longest wait for a free slot (the FD may be slow writing files).
const CLAIM_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

/// The volume being read.
struct Mounted {
    /// Volume name.
    name: String,
    /// The open file.
    file: File,
    /// Its length.
    len: u64,
    /// Length of the label block (data starts behind it).
    label_len: u64,
    /// The catalog record (`GetVolInfo … write=0`), if the Director had one.
    cat: Option<VolumeInfo>,
    /// Time spent reading (µs, `VolReadTime`).
    read_time: i64,
}

/// How reading one volume ended.
enum VolumeEnd {
    /// Every range was read; true when the last one reached the end of the volume file.
    Done(bool),
    /// The network session needs no more blocks (it is done or gone).
    Stopped,
}

/// One device reader.
struct Reader<'a> {
    /// The job's ring.
    ring: &'a Ring,
    /// The job.
    info: JobInfo,
    /// The device being read.
    dev: FileDevice,
    /// Every device the job may read from: the targets of a Distributor
    /// (the initial device and [`JobInfo::alternatives`]); one entry otherwise.
    pool: Vec<FileDevice>,
    /// The mounted volume.
    vol: Option<Mounted>,
}

/// Runs the reader of ring `ring_name` (the `--device-reader` mode);
/// returns the exit code.
pub fn run(ring_name: &str) -> i32 {
    let mut ring = match Ring::open(ring_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("osd device reader: {e}");
            return 1;
        }
    };
    // The consumer is the network session in the parent; it may block on a
    // slow FD for long, its process liveness is still checked.
    ring.set_liveness(CLAIM_TIMEOUT);
    ring.take_producer();
    tracing::info!("device reader pid {} attached to ring {ring_name}", std::process::id());
    let ok = match JobInfo::decode(&String::from_utf8_lossy(&ring.info())) {
        Ok(info) => {
            let dev = info.device();
            let mut pool = info.alternatives.clone();
            pool.push(dev.clone());
            Reader { ring: &ring, info, dev, pool, vol: None }.work()
        }
        Err(e) => {
            eprintln!("osd device reader: {e}");
            false
        }
    };
    ring.finish(if ok { State::Done } else { State::Failed });
    if ok { 0 } else { 1 }
}

/// Microseconds since `t`.
fn micros(t: Instant) -> i64 {
    t.elapsed().as_micros() as i64
}

impl Reader<'_> {
    /// A job message through the network session.
    fn jmsg(&self, msg_type: i32, text: &str) {
        let text = if text.ends_with('\n') { text.to_string() } else { format!("{text}\n") };
        let _ = self.ring.ask_consumer(CONSUMER, &Request::Jmsg(msg_type, text).encode(), ASK_TIMEOUT);
    }

    /// One catalog request; the Director's answer line.
    fn cat_req(&self, line: &str) -> Result<String, String> {
        let answer = self.ring.ask_consumer(CONSUMER, &Request::CatReq(line.to_string()).encode(), ASK_TIMEOUT).map_err(|e| e.to_string())?;
        let answer = String::from_utf8_lossy(&answer).into_owned();
        match answer.strip_prefix('E') {
            Some(err) if !answer.starts_with("Error") => Err(format!("Network error with the Director: {err}")),
            _ => Ok(answer),
        }
    }

    /// The whole job: every volume of the bootstrap, then release.
    fn work(&mut self) -> bool {
        let bsr = match std::fs::read_to_string(&self.info.bootstrap).map_err(|e| e.to_string()).and_then(|t| Bsr::parse(&t).map_err(|e| e.to_string())) {
            Ok(b) => b,
            Err(e) => {
                self.jmsg(M_FATAL, &format!("Error parsing bootstrap file {}: {e}", self.info.bootstrap));
                return false;
            }
        };
        let volumes = bsr.restore_volumes();
        if volumes.is_empty() {
            self.jmsg(M_FATAL, "No Volume names found for restore.");
            return false;
        }
        let mut ok = true;
        let mut finished_all = false;
        for (index, vol) in volumes.iter().enumerate() {
            if let Err(e) = self.acquire(vol) {
                self.jmsg(M_FATAL, &e);
                if index > 0 {
                    self.jmsg(M_FATAL, &format!("Cannot open Dev={}, Vol={}", self.dev.print_name(), vol.name));
                }
                ok = false;
                break;
            }
            if index == 0 && self.ring.ask_consumer(CONSUMER, &Request::Ready.encode(), ASK_TIMEOUT).is_err() {
                ok = false;
                break;
            }
            match self.read_volume(&bsr, index as u32, &vol.name) {
                // the last volume's ranges end before its end: bareos-sd's MatchBsr says "done" with
                // the next record and the read stops without these messages (the volume goes on with
                // other jobs' data)
                Ok(VolumeEnd::Done(false)) if index + 1 == volumes.len() => {}
                Ok(VolumeEnd::Done(_)) => {
                    let file = self.vol.as_ref().map(|v| v.len >> 32).unwrap_or(0);
                    self.jmsg(M_INFO, &format!("End of Volume at file {file} on device {}, Volume \"{}\"", self.dev.print_name(), vol.name));
                    finished_all = index + 1 == volumes.len();
                }
                Ok(VolumeEnd::Stopped) => break,
                Err(e) => {
                    self.jmsg(M_FATAL, &e);
                    ok = false;
                    break;
                }
            }
        }
        if finished_all {
            self.jmsg(M_INFO, "End of all volumes.");
        }
        self.release() && ok
    }

    /// `AcquireDeviceForRead` for a File device: catalog record, open the
    /// file, check its label. bareos-sd asks the operator to mount the
    /// volume when this fails; osd ends the job (as it does when writing).
    fn acquire(&mut self, vol: &RestoreVolume) -> Result<(), String> {
        // `DirAskSysopToMountVolume` (mount.cc): a Volume that is not there
        // (file missing, wrong or unlabeled Volume) is asked of the operator
        // (M_MOUNT), JS_WaitMedia, woken by `mount`/`label` on the device;
        // the wait starts at one hour and doubles up to one day, nine waits.
        const MIN_WAIT: u64 = 60 * 60;
        const MAX_WAIT: u64 = 24 * 60 * 60;
        const MAX_NUM_WAIT: u32 = 9;
        let (mut wait_sec, mut num_wait) = (MIN_WAIT, 0u32);
        let mut waited = false;
        loop {
            match self.try_acquire(vol) {
                Err(e) if e.starts_with("Too many errors trying to mount") => {
                    self.jmsg(M_MOUNT, &format!(
                        "Please mount read Volume \"{}\" for:\n    Job:          {}\n    Storage:      {}\n    Pool:         {}\n    Media type:   {}\n",
                        vol.name, self.info.job, self.dev.print_name(), self.info.pool_name, self.info.media_type
                    ));
                    let answer = self.ring
                        .ask_consumer(CONSUMER, &Request::WaitMedia(wait_sec as u32).encode(), Duration::from_secs(wait_sec + 60))
                        .map_err(|e| e.to_string())?;
                    waited = true;
                    match answer.as_slice() {
                        b"c" => {
                            self.jmsg(M_INFO, &format!(
                                "Job {} canceled while waiting for mount on Storage Device {}.\n", self.info.job, self.dev.print_name()));
                            return Err(String::new());
                        }
                        b"w" => {} // mount or label: try again
                        _ => {
                            wait_sec = (wait_sec * 2).min(MAX_WAIT);
                            num_wait += 1;
                            if num_wait >= MAX_NUM_WAIT {
                                return Err(format!(
                                    "Max time exceeded waiting to mount Storage Device {} for Job {}", self.dev.print_name(), self.info.job));
                            }
                        }
                    }
                }
                other => {
                    if other.is_ok() && waited {
                        let _ = self.ring.ask_consumer(CONSUMER, &Request::JobStatus('R').encode(), ASK_TIMEOUT);
                    }
                    return other;
                }
            }
        }
    }

    /// One try of [`Reader::acquire`].
    fn try_acquire(&mut self, vol: &RestoreVolume) -> Result<(), String> {
        self.close_volume();
        if !vol.media_type.is_empty() && vol.media_type != self.dev.media_type {
            // a Distributor's target of the volume's media type: read from it (`Changing read device`)
            if let Some(next) = self.pool.iter().find(|d| d.media_type == vol.media_type).cloned() {
                self.jmsg(M_INFO, &format!(
                    "Changing read device. Want Media Type=\"{}\" have=\"{}\"\n  device={}",
                    vol.media_type,
                    self.dev.media_type,
                    self.dev.print_name()
                ));
                self.jmsg(M_INFO, &format!(
                    "Media Type change.  New read device {} chosen.", next.print_name()));
                self.dev = next;
            }
        }
        if !vol.media_type.is_empty() && vol.media_type != self.dev.media_type {
            self.jmsg(M_INFO, &format!(
                "Changing read device. Want Media Type=\"{}\" have=\"{}\"\n  device={}",
                vol.media_type,
                self.dev.media_type,
                self.dev.print_name()
            ));
            return Err(format!("No suitable device found to read Volume \"{}\"", vol.name));
        }
        // Volume info is always needed because of VolParts
        let answer = self.cat_req(&osd::askdir::get_vol_info(&self.info.job, &vol.name, false))?;
        let cat = VolumeInfo::parse(&answer);
        if cat.is_none() {
            self.jmsg(M_WARNING, &format!("Read acquire: Error getting Volume info: {answer}"));
        }
        let too_many = format!("Too many errors trying to mount device {} for reading.", self.dev.print_name());
        let path = self.dev.volume_path(&vol.name);
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) => {
                self.jmsg(M_WARNING, &format!(
                    "Read open device {} Volume \"{}\" failed: ERR={e}",
                    self.dev.print_name(),
                    vol.name
                ));
                return Err(too_many);
            }
        };
        match self.dev.read_label(&vol.name) {
            LabelState::Labeled(l) if l.volume_name == vol.name => {}
            LabelState::Labeled(l) => {
                self.jmsg(M_WARNING, &format!(
                    "Read acquire: Wrong Volume mounted on device {}: Wanted {} have {}",
                    self.dev.print_name(),
                    vol.name,
                    l.volume_name
                ));
                return Err(too_many);
            }
            LabelState::NoLabel => {
                self.jmsg(M_WARNING, &format!(
                    "Read acquire: Requested Volume \"{}\" on {} is not a Bareos labeled Volume, because: ERR=empty volume",
                    vol.name,
                    self.dev.print_name()
                ));
                return Err(too_many);
            }
            LabelState::Broken(e) => {
                self.jmsg(M_WARNING, &format!(
                    "Read acquire: Requested Volume \"{}\" on {} is not a Bareos labeled Volume, because: ERR={e}",
                    vol.name,
                    self.dev.print_name()
                ));
                return Err(too_many);
            }
        }
        let len = file.metadata().map(|m| m.len()).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut head = [0u8; BLKHDR2_LENGTH];
        (&file).read_exact(&mut head).map_err(|e| format!("{}: {e}", path.display()))?;
        let label_len = u64::from(BlockHeader::parse(&head)?.block_len);
        self.vol = Some(Mounted { name: vol.name.clone(), file, len, label_len, cat, read_time: 0 });
        self.jmsg(M_INFO, &format!("Ready to read from volume \"{}\" on device {}.", vol.name, self.dev.print_name()));
        Ok(())
    }

    /// Reads the bootstrap's ranges of the mounted volume (index `index` of
    /// the restore list) block by block into the ring.
    fn read_volume(&mut self, bsr: &Bsr, index: u32, name: &str) -> Result<VolumeEnd, String> {
        let ranges = bsr.volume_ranges(name).unwrap_or_else(|| vec![(0, u64::MAX)]);
        let print_name = self.dev.print_name();
        // PositionDeviceToFirstFile
        if let Some(&(first, _)) = ranges.first()
            && first > 0
        {
            let msg = format!("Forward spacing Volume \"{name}\" to file:block {}:{}.", first >> 32, first as u32);
            self.jmsg(M_INFO, &msg);
        }
        let vol = self.vol.as_mut().ok_or("no volume mounted")?;
        let mut pos = vol.label_len;
        let mut block = Vec::new();
        for (start, end) in ranges {
            pos = pos.max(start);
            while pos <= end && pos < vol.len {
                let started = Instant::now();
                vol.file.seek(SeekFrom::Start(pos)).map_err(|e| format!("Seek error on device {print_name}: {e}"))?;
                block.resize(BLKHDR2_LENGTH, 0);
                vol.file.read_exact(&mut block).map_err(|e| format!("Read error on device {print_name} at {pos}: {e}"))?;
                let header = BlockHeader::parse(&block).map_err(|e| format!("Volume data error at 0:{pos}! {e}"))?;
                let len = header.block_len as usize;
                if len > self.ring.slot_size() {
                    return Err(format!(
                        "Block length {len} at 0:{pos} on Volume \"{name}\" is greater than the ring slot of {} bytes.",
                        self.ring.slot_size()
                    ));
                }
                if len < BLKHDR2_LENGTH {
                    return Err(format!("Volume data error at 0:{pos}! Block length {len} is too short."));
                }
                block.resize(len, 0);
                vol.file
                    .read_exact(&mut block[BLKHDR2_LENGTH..])
                    .map_err(|e| format!("Read error on device {print_name} at {pos}: {e}"))?;
                if let Err(e) = verify_block(&block) {
                    let msg = format!("Volume data error at 0:{pos}! {e}");
                    let _ = self.ring.ask_consumer(CONSUMER, &Request::Jmsg(M_ERROR, format!("{msg}\n")).encode(), ASK_TIMEOUT);
                    return Err(msg);
                }
                vol.read_time += micros(started);
                let mut slot = match self.ring.claim(CLAIM_TIMEOUT) {
                    Ok(s) => s,
                    Err(RingError::NoConsumer) => return Ok(VolumeEnd::Stopped),
                    Err(e) => return Err(format!("device reader: {e}")),
                };
                slot.data[..len].copy_from_slice(&block);
                slot.meta = BlockMeta { volume: index, address: pos }.to_words();
                slot.publish(len);
                pos += len as u64;
            }
        }
        Ok(VolumeEnd::Done(pos >= vol.len))
    }

    /// `VolumeUnused` + close: the file of the previous volume.
    fn close_volume(&mut self) {
        self.vol = None;
    }

    /// `ReleaseDevice` for reading: "Releasing device", then
    /// `DirUpdateVolumeInfo` for the last volume read. False when the
    /// catalog update failed.
    fn release(&mut self) -> bool {
        self.jmsg(M_INFO, &format!("Releasing device {}.", self.dev.print_name()));
        let Some(vol) = self.vol.take() else { return true };
        let Some(cat) = &vol.cat else { return true };
        let mut upd = cat.to_update();
        upd.read_time += vol.read_time;
        upd.end_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        match self.cat_req(&upd.format(&self.info.job)) {
            Ok(answer) if VolumeInfo::parse(&answer).is_some() => true,
            Ok(answer) => {
                self.jmsg(M_FATAL, &format!("Didn't get vol info vol={}: ERR=Error getting Volume info: {answer}", vol.name));
                false
            }
            Err(e) => {
                self.jmsg(M_FATAL, &format!("Didn't get vol info vol={}: ERR={e}", vol.name));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_session::{Decision, RecordFilter};
    use crate::writer::tests::{job, FakeDirector};
    use bareos_format::bsr::BsrMatcher;
    use bareos_format::volume::RecordAssembler;
    use osd_ring::RingConfig;

    /// The bootstrap odir would write for the writer test job: one section
    /// per non-placeholder `CreateJobMedia` (MediaId n = volume `Voln`).
    fn bootstrap(fake: &FakeDirector) -> String {
        let mut text = String::new();
        for r in fake.requests.iter().filter(|r| r.contains(" CreateJobMedia ") && !r.contains(" FirstIndex=0 ")) {
            let n = |k: &str| r.split_whitespace().find_map(|w| w.strip_prefix(k)).unwrap().parse::<u64>().unwrap();
            let (first, last) = (n("FirstIndex="), n("LastIndex="));
            let start = n("StartFile=") << 32 | n("StartBlock=");
            let end = n("EndFile=") << 32 | n("EndBlock=");
            text += &format!(
                "Storage=\"test-sd\"\nVolume=\"Vol{}\"\nMediaType=\"File\"\nVolSessionId=1\nVolSessionTime=1795200000\n\
                 VolAddr={start}-{end}\nFileIndex={first}-{last}\nCount={}\n",
                n("MediaId="),
                last - first + 1
            );
        }
        text
    }

    /// A record as the "FD" got it: (FileIndex, Stream, data).
    type Sent = (i32, i32, Vec<u8>);

    /// Reads with a reader thread like the network session does; returns
    /// the records that went to the "FD", the reader's exit code and the
    /// fake Director.
    fn read(dir: &std::path::Path, ring_name: &str, bsr_text: &str, fake: FakeDirector) -> (Vec<Sent>, i32, FakeDirector) {
        read_with(dir, ring_name, bsr_text, fake, Vec::new())
    }

    /// [`read`] with the other targets of a Distributor the job holds.
    fn read_with(dir: &std::path::Path, ring_name: &str, bsr_text: &str, mut fake: FakeDirector,
                 alternatives: Vec<FileDevice>) -> (Vec<Sent>, i32, FakeDirector) {
        let bsr_path = dir.join("test.bootstrap");
        std::fs::write(&bsr_path, bsr_text).unwrap();
        let info = JobInfo {
            job: "R.2026-11-20_02.00.00_02".into(),
            job_id: 10,
            media_type: "File".into(),
            storage: "test-sd".into(),
            device: "Dev".into(),
            archive_device: dir.display().to_string(),
            max_block_size: 4096,
            label_block_size: 64512,
            bootstrap: bsr_path.display().to_string(),
            alternatives,
            ..Default::default()
        };
        let _ = Ring::remove(ring_name);
        let ring = Ring::create(ring_name, RingConfig { slots: 2, slot_size: 1 << 16, consumers: 1 }).unwrap();
        ring.set_info(info.encode().as_bytes()).unwrap();
        ring.attach(CONSUMER).unwrap();
        let name = ring_name.to_string();
        let reader = std::thread::spawn(move || run(&name));
        let bsr = Bsr::parse(bsr_text).unwrap();
        let volumes = bsr.restore_volumes();
        let mut filter = RecordFilter::new(BsrMatcher::new(bsr));
        let mut assembler = RecordAssembler::new();
        let mut sent = Vec::new();
        let mut ready = false;
        loop {
            ring.serve_producer(CONSUMER, Duration::from_millis(5), |req| fake.answer(req, &mut ready));
            match ring.next(CONSUMER, Duration::from_millis(5)) {
                Ok(Some(slot)) => {
                    let meta = BlockMeta::from_words(slot.meta).unwrap();
                    let data = slot.data.to_vec();
                    ring.release(CONSUMER);
                    let (h, recs) = assembler.push_block(&data, meta.address, true).unwrap();
                    filter.start_block();
                    for r in recs {
                        let vol = &volumes[meta.volume as usize].name;
                        if filter.decide(vol, meta.address + u64::from(h.block_len) - 1, &r) == Decision::Send {
                            sent.push((r.file_index, r.stream, r.data));
                        }
                    }
                }
                Ok(None) | Err(RingError::ProducerGone) => break,
                Err(_) => {}
            }
        }
        // the reader may still release the device through the mailbox
        while !reader.is_finished() {
            ring.serve_producer(CONSUMER, Duration::from_millis(5), |req| fake.answer(req, &mut ready));
        }
        let code = reader.join().unwrap();
        Ring::remove(ring_name).unwrap();
        (sent, code, fake)
    }

    #[test]
    fn a_job_over_two_volumes_is_read_back_whole() {
        let dir = std::env::temp_dir().join(format!("osd-reader-two-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ring = format!("osd-readertest-{}-two", std::process::id());
        let mut fake = job(&dir, &ring, 30, 1000, 20_000);
        assert!(dir.join("Vol2").exists(), "the backup spans two volumes");
        let text = bootstrap(&fake);
        assert_eq!(text.matches("Volume=").count(), 2, "{text}");
        fake.requests.clear();
        fake.messages.clear();
        let (sent, code, fake) = read(&dir, &ring, &text, fake);
        assert_eq!(code, 0, "{:?}", fake.messages);
        // 30 records: FileIndex 1..15, streams 1 and 2, as the writer test wrote them
        let expected: Vec<Sent> = (0..30i32).map(|i| (i / 2 + 1, if i % 2 == 0 { 1 } else { 2 }, vec![(i % 251) as u8; 1000])).collect();
        assert_eq!(sent.len(), expected.len());
        assert!(sent == expected, "records differ");
        let kinds: Vec<&str> = fake.requests.iter().map(|r| r.split_whitespace().nth(2).unwrap()).collect();
        assert_eq!(kinds, ["GetVolInfo", "GetVolInfo", "UpdateMedia"]);
        assert!(fake.requests[0].ends_with("VolName=Vol1 write=0") && fake.requests[1].ends_with("VolName=Vol2 write=0"));
        let m = &fake.messages;
        for want in ["Ready to read from volume \"Vol1\"", "Forward spacing Volume \"Vol1\"", "End of Volume at file 0 on device \"Dev\"",
            "Ready to read from volume \"Vol2\"", "End of all volumes.", "Releasing device \"Dev\""] {
            assert!(m.iter().any(|x| x.starts_with(want)), "{want} missing in {m:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A Distributor's job: Vol1 is on the device `Dev` (media type `File`), Vol2 on `Dev2` (`File2`).
    #[test]
    fn a_job_over_two_targets_of_a_distributor_is_read_from_both() {
        let dir = std::env::temp_dir().join(format!("osd-reader-dist-{}", std::process::id()));
        let dir2 = std::env::temp_dir().join(format!("osd-reader-dist2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&dir2).unwrap();
        let ring = format!("osd-readertest-{}-dist", std::process::id());
        let mut fake = job(&dir, &ring, 30, 1000, 20_000);
        std::fs::rename(dir.join("Vol2"), dir2.join("Vol2")).unwrap();
        let text = bootstrap(&fake);
        let (head, tail) = text.split_once("Volume=\"Vol2\"").unwrap();
        let text = format!("{head}Volume=\"Vol2\"{}", tail.replacen("MediaType=\"File\"", "MediaType=\"File2\"", 1));
        assert!(text.contains("MediaType=\"File2\""), "{text}");
        fake.requests.clear();
        fake.messages.clear();
        let second = FileDevice { name: "Dev2".into(), archive_device: dir2.clone(), media_type: "File2".into(), label_media: true,
            max_block_size: 4096, label_block_size: 64512, max_volume_size: 0 };
        let (sent, code, fake) = read_with(&dir, &ring, &text, fake, vec![second]);
        assert_eq!(code, 0, "{:?}", fake.messages);
        assert_eq!(sent.len(), 30, "all records of both volumes");
        let m = &fake.messages;
        for want in ["Ready to read from volume \"Vol1\" on device \"Dev\"", "Changing read device. Want Media Type=\"File2\" have=\"File\"",
            "Media Type change.  New read device \"Dev2\"", "Ready to read from volume \"Vol2\" on device \"Dev2\"", "End of all volumes."] {
            assert!(m.iter().any(|x| x.starts_with(want)), "{want} missing in {m:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&dir2).ok();
    }

    #[test]
    fn a_wrong_volume_is_rejected_like_bareos_sd() {
        let dir = std::env::temp_dir().join(format!("osd-reader-wrong-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ring = format!("osd-readertest-{}-wrong", std::process::id());
        let mut fake = job(&dir, &ring, 4, 100, 0);
        // the operator is asked to mount the volume and cancels the job on the first wait
        fake.cancel_waits_from = Some(1);
        // the file "Vol1" now carries the label of another volume
        std::fs::rename(dir.join("Vol1"), dir.join("Other")).unwrap();
        let label = crate::device::new_label("Other", "P", "File", "test-sd");
        let dev = FileDevice { name: "Dev".into(), archive_device: dir.clone(), media_type: "File".into(), label_media: true,
            max_block_size: 4096, label_block_size: 64512, max_volume_size: 0 };
        dev.write_label(&label, 0, 0).unwrap();
        std::fs::rename(dir.join("Other"), dir.join("Vol1")).unwrap();
        let (sent, code, fake) = read(&dir, &ring, "Volume=\"Vol1\"\nMediaType=\"File\"\nVolSessionId=1\nVolSessionTime=1795200000\n", fake);
        assert!(sent.is_empty());
        assert_eq!(code, 1);
        let m = &fake.messages;
        assert!(m.iter().any(|x| x.starts_with("Read acquire: Wrong Volume mounted on device \"Dev\"") && x.contains("Wanted Vol1 have Other")), "{m:?}");
        assert!(m.iter().any(|x| x.starts_with("Please mount read Volume \"Vol1\" for:")), "no mount request: {m:?}");
        assert!(m.iter().any(|x| x.starts_with("Job ") && x.contains("canceled while waiting for mount on Storage Device \"Dev\"")), "{m:?}");
        assert_eq!(fake.waits, [3600], "one wait of one hour");
        std::fs::remove_dir_all(&dir).ok();
    }
}
