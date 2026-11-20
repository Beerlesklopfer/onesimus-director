//! Der Geräteschreiber: liest Blöcke aus dem Ring und schreibt das Volume.
//!
//! Zweck: Je Job und Ziel-Device läuft ein Schreiber, in Stufe 1 genau
//! einer, als eigener Prozess (`osd --device-writer <Ring>`, D-13). Er
//! spricht nur über den Ring: Blöcke aus den Fächern, Katalog-Anfragen und
//! Job-Meldungen über das Postfach (die Netzsitzung leitet sie an den
//! Director weiter). Er belegt das Volume (Volume suchen, bei Bedarf
//! labeln, ans Ende springen), schreibt jeden Block mit Kopf und
//! Prüfsumme, legt die JobMedia-Sätze an, wechselt bei vollem Volume auf
//! das nächste, schreibt am Ende das EOS-Label und gibt das Device frei.
//!
//! Ablauf und Katalog-Anfragen wie bareos-sd (im Gerüst gemessen,
//! 20.11.2026): `FindMedia` → `GetVolInfo` → (neues Volume: Label,
//! `UpdateMedia relabel=1`, `GetVolInfo`) → `UpdateMedia` (VolMounts+1) →
//! `UpdateMedia` (VolJobs+1) → beim ersten Block `CreateJobMedia` mit
//! Nullen (Platzhalter) → … → beim Freigeben `CreateJobMedia` mit
//! FirstIndex/LastIndex und Adressen → `UpdateMedia`.
//!
//! Adressen auf einem Datei-Volume: `StartBlock`/`EndBlock` sind die
//! unteren, `StartFile`/`EndFile` die oberen 32 Bit der Byte-Adresse
//! (`SeekMode::BYTES`).
//!
//! Bareos-Gegenstück: `core/src/stored/acquire.cc`
//! (`AcquireDeviceForAppend`, `ReleaseDevice`), `stored/mount.cc`
//! (`MountNextWriteVolume`, `find_a_volume`, `TryAutolabel`,
//! `is_eod_valid`), `stored/block.cc` (`WriteBlockToDev`,
//! `TerminateWritingVolume`), `stored/device.cc`
//! (`FixupDeviceBlockWriteError`, `SetNewVolumeParameters`),
//! `stored/label.cc` (`WriteSessionLabel` für EOS), `stored/askdir.cc`.

use std::fs::File;
use std::io::Write;
use std::time::{Duration, Instant};

use bareos_format::label::{SessionLabel, BAREOS_ID, BAREOS_TAPE_VERSION, EOS_LABEL};
use bareos_format::msg::{M_ERROR, M_FATAL, M_INFO, M_WARNING};
use bareos_format::volume::{BlockBuilder, RecordProgress, RecordRef, BLKHDR2_LENGTH};
use osd::askdir::{CreateJobMedia, UpdateMedia, VolumeInfo};
use osd_ring::{Ring, State};

use crate::device::{with_commas, FileDevice, LabelState};
use crate::ringproto::{JobInfo, Request, SlotKind, SlotMeta};

/// The writer is consumer 0 (one device per job in stage 1).
pub const CONSUMER: usize = 0;

/// Longest wait for the Director's answer to a catalog request.
const CATREQ_TIMEOUT: Duration = Duration::from_secs(1800);

/// Longest wait for the next block from the network session.
const NEXT_TIMEOUT: Duration = Duration::from_secs(24 * 3600);

/// The volume the writer appends to.
struct Volume {
    /// Volume name.
    name: String,
    /// The open file.
    file: File,
    /// Next write address (`file_addr`).
    addr: u64,
    /// The catalog counters (`dev->VolCatInfo`).
    cat: UpdateMedia,
    /// `MediaId`.
    media_id: u64,
}

/// One device writer.
pub struct Writer<'a> {
    /// The job's ring.
    ring: &'a Ring,
    /// The job.
    info: JobInfo,
    /// The device.
    dev: FileDevice,
    /// The mounted volume.
    vol: Option<Volume>,
    /// `block->BlockNumber` of the next block.
    block_number: u32,
    /// `dcr->VolFirstIndex`.
    first_index: u32,
    /// `dcr->VolLastIndex`.
    last_index: u32,
    /// `dcr->StartBlock`.
    start_block: u32,
    /// `dcr->StartFile`.
    start_file: u32,
    /// `dcr->EndBlock`.
    end_block: u32,
    /// `dcr->EndFile`.
    end_file: u32,
    /// `dcr->WroteVol`.
    wrote: bool,
}

/// Seconds since 1970.
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// `bstrftime` with the Bareos default format `%d-%b-%Y %H:%M`.
fn bareos_time() -> String {
    // SAFETY: localtime_r/strftime with valid buffers.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        let mut buf = [0u8; 64];
        let n = libc::strftime(buf.as_mut_ptr().cast(), buf.len(), c"%d-%b-%Y %H:%M".as_ptr(), &tm);
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

/// Runs the writer of ring `ring_name` (the `--device-writer` mode);
/// returns the exit code.
pub fn run(ring_name: &str) -> i32 {
    let ring = match Ring::open(ring_name) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("osd device writer: {e}");
            return 1;
        }
    };
    tracing::info!("device writer pid {} attached to ring {ring_name}", std::process::id());
    if let Err(e) = ring.attach(CONSUMER) {
        eprintln!("osd device writer: {e}");
        return 1;
    }
    let ok = match JobInfo::decode(&String::from_utf8_lossy(&ring.info())) {
        Ok(info) => Writer::new(&ring, info).work(),
        Err(e) => {
            eprintln!("osd device writer: {e}");
            false
        }
    };
    ring.set_consumer_state(CONSUMER, if ok { State::Done } else { State::Failed });
    if ok { 0 } else { 1 }
}

impl<'a> Writer<'a> {
    /// A writer for `info` on `ring`.
    pub fn new(ring: &'a Ring, info: JobInfo) -> Writer<'a> {
        let dev = info.device();
        Writer {
            ring,
            info,
            dev,
            vol: None,
            block_number: 0,
            first_index: 0,
            last_index: 0,
            start_block: 0,
            start_file: 0,
            end_block: 0,
            end_file: 0,
            wrote: false,
        }
    }

    /// The whole job: acquire, write every slot, end the session, release.
    /// False when the job failed (the reason went out as a job message).
    pub fn work(&mut self) -> bool {
        if let Err(e) = self.acquire() {
            self.jmsg(M_FATAL, &e);
            return false;
        }
        loop {
            let slot = match self.ring.next(CONSUMER, NEXT_TIMEOUT) {
                Ok(Some(slot)) => slot,
                Ok(None) => {
                    // The session finished without an End slot: canceled or
                    // failed before the end. Release what was written.
                    let _ = self.release();
                    return false;
                }
                Err(e) => {
                    // The session is gone: nobody can take catalog requests.
                    eprintln!("osd device writer {}: {e}", self.info.job);
                    return false;
                }
            };
            let meta = SlotMeta::from_words(slot.meta);
            let payload = slot.data.to_vec();
            self.ring.release(CONSUMER);
            let result = match SlotKind::from_word(meta.kind) {
                Some(SlotKind::Data) => self.write_block(&payload, &meta),
                Some(SlotKind::End) => self.end_session(&payload, &meta).and_then(|_| self.release()),
                Some(SlotKind::Block) | None => Err(format!("ring slot of unknown kind {}", meta.kind)),
            };
            match result {
                Ok(()) if SlotKind::from_word(meta.kind) == Some(SlotKind::End) => return true,
                Ok(()) => {}
                Err(e) => {
                    self.jmsg(M_FATAL, &e);
                    return false;
                }
            }
        }
    }

    /// A job message through the network session.
    fn jmsg(&self, msg_type: i32, text: &str) {
        let text = if text.ends_with('\n') { text.to_string() } else { format!("{text}\n") };
        let _ = self.ring.call(CONSUMER, &Request::Jmsg(msg_type, text).encode(), CATREQ_TIMEOUT);
    }

    /// One catalog request; the Director's answer line.
    fn cat_req(&self, line: &str) -> Result<String, String> {
        let answer = self.ring.call(CONSUMER, &Request::CatReq(line.to_string()).encode(), CATREQ_TIMEOUT).map_err(|e| e.to_string())?;
        let answer = String::from_utf8_lossy(&answer).into_owned();
        match answer.strip_prefix('E') {
            Some(err) if !answer.starts_with("Error") => Err(format!("Network error with the Director: {err}")),
            _ => Ok(answer),
        }
    }

    /// A catalog request answered with a volume record.
    fn vol_req(&self, line: &str) -> Result<VolumeInfo, String> {
        let answer = self.cat_req(line)?;
        VolumeInfo::parse(&answer).ok_or_else(|| format!("Error getting Volume info: {answer}"))
    }

    /// `DirUpdateVolumeInfo`: sends the counters, takes over the answer.
    fn update_media(&mut self, relabel: bool) -> Result<(), String> {
        let job = self.info.job.clone();
        let vol = self.vol.as_mut().ok_or("no volume mounted")?;
        vol.cat.end_time = now();
        vol.cat.relabel = relabel;
        // just labeled, or a manually labeled volume was written on
        if relabel || (vol.cat.status == "Unlabeled" && vol.cat.bytes > 0) {
            vol.cat.status = "Append".into();
        }
        let line = vol.cat.format(&job);
        let answer = self.vol_req(&line).map_err(|e| format!("Didn't get vol info vol={}: ERR={e}", vol_name(&self.vol)))?;
        let vol = self.vol.as_mut().ok_or("no volume mounted")?;
        let write_time = vol.cat.write_time;
        vol.cat = answer.to_update();
        vol.cat.write_time = vol.cat.write_time.max(write_time);
        Ok(())
    }

    /// `AcquireDeviceForAppend`: mounts the first volume, VolJobs+1, and
    /// tells the network session the device is ready.
    fn acquire(&mut self) -> Result<(), String> {
        self.mount(None)?;
        self.block_number = 0;
        self.ring.call(CONSUMER, &Request::Ready.encode(), CATREQ_TIMEOUT).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// `MountNextWriteVolume` for a File device: find a volume, label it if
    /// new, check the label, go to its end (`is_eod_valid`), VolMounts+1,
    /// VolJobs+1. `previous`: the full volume being replaced.
    fn mount(&mut self, previous: Option<&str>) -> Result<(), String> {
        let job = self.info.job.clone();
        let find = osd::askdir::find_media(&job, 1, &self.info.pool_name, &self.info.media_type, previous.unwrap_or(""));
        let found = self.vol_req(&find).map_err(|_| {
            format!(
                "Job {job} is waiting. Cannot find any appendable volumes.\nPlease use the \"label\" command to create a new \
                 Volume for:\n    Storage:      {}\n    Pool:         {}\n    Media type:   {}\n(osd stage 1 does not wait for an \
                 operator; the job ends.)",
                self.dev.print_name(),
                self.info.pool_name,
                self.info.media_type
            )
        })?;
        let mut info = self.vol_req(&osd::askdir::get_vol_info(&job, &found.volume, true))?;
        let name = info.volume.clone();
        match self.dev.read_label(&name) {
            LabelState::NoLabel | LabelState::Broken(_) if info.bytes == 0 || info.status == "Recycle" => {
                if !self.dev.label_media {
                    return Err(format!("Device {} not configured to autolabel Volumes.", self.dev.print_name()));
                }
                let label = crate::device::new_label(&name, &self.info.pool_name, &self.dev.media_type, &self.info.storage);
                let len = self.dev.write_label(&label, self.info.vol_session_id, self.info.vol_session_time)?;
                let (file, addr) = self.dev.open_append(&name)?;
                let mut cat = info.to_update();
                cat.bytes = len;
                cat.blocks = 1;
                cat.writes = 1;
                self.vol = Some(Volume { name: name.clone(), file, addr, cat, media_id: info.media_id });
                self.update_media(true)?;
                self.jmsg(M_INFO, &format!("Labeled new Volume \"{name}\" on device {}.", self.dev.print_name()));
                info = self.vol_req(&osd::askdir::get_vol_info(&job, &name, true))?;
            }
            LabelState::Labeled(label) if label.volume_name == name => {}
            LabelState::Labeled(label) => {
                return Err(format!(
                    "Volume \"{name}\" not on device {}: the file holds Volume \"{}\".",
                    self.dev.print_name(),
                    label.volume_name
                ));
            }
            LabelState::NoLabel => {
                return Err(format!("Volume \"{name}\" not on device {} (no file, catalog VolBytes={}).", self.dev.print_name(), info.bytes));
            }
            LabelState::Broken(e) => return Err(format!("Could not check volume label of \"{name}\": ERR={e}")),
        }
        let (file, addr) = self.dev.open_append(&name)?;
        let previous_write_time = self.vol.as_ref().map(|v| v.cat.write_time).unwrap_or(0);
        let mut cat = info.to_update();
        cat.write_time = cat.write_time.max(previous_write_time);
        self.vol = Some(Volume { name: name.clone(), file, addr, cat, media_id: info.media_id });
        self.jmsg(M_INFO, &format!("Moving to end of data on volume \"{name}\""));
        let catalog_bytes = info.bytes;
        if addr == catalog_bytes {
            self.jmsg(M_INFO, &format!("Ready to append to end of Volume \"{name}\" size={addr}"));
        } else if addr > catalog_bytes {
            self.jmsg(M_WARNING, &format!(
                "For Volume \"{name}\":\nThe sizes do not match! Volume={} Catalog={}\nCorrecting Catalog",
                with_commas(addr),
                with_commas(catalog_bytes)
            ));
            if let Some(v) = self.vol.as_mut() {
                v.cat.bytes = addr;
                v.cat.files = (addr >> 32) as u32;
            }
            self.update_media(false)?;
        } else {
            let msg = format!(
                "Bareos cannot write on disk Volume \"{name}\" because: The sizes do not match! Volume={} Catalog={}",
                with_commas(addr),
                with_commas(catalog_bytes)
            );
            self.jmsg(M_ERROR, &msg);
            self.mark_in_error()?;
            return Err(msg);
        }
        if let Some(v) = self.vol.as_mut() {
            v.cat.mounts += 1;
        }
        self.update_media(false)?;
        if let Some(v) = self.vol.as_mut() {
            v.cat.jobs += 1;
        }
        self.update_media(false)?;
        // SetNewVolumeParameters / SetStartVolPosition
        self.start_block = addr as u32;
        self.start_file = (addr >> 32) as u32;
        self.first_index = 0;
        self.last_index = 0;
        self.wrote = false;
        Ok(())
    }

    /// `MarkVolumeInError`.
    fn mark_in_error(&mut self) -> Result<(), String> {
        self.jmsg(M_INFO, &format!("Marking Volume \"{}\" in Error in Catalog.", vol_name(&self.vol)));
        if let Some(v) = self.vol.as_mut() {
            v.cat.status = "Error".into();
        }
        self.update_media(false)
    }

    /// `DirCreateJobmediaRecord(zero)`.
    fn create_jobmedia(&mut self, zero: bool) -> Result<(), String> {
        if !zero && self.first_index == 0 && (self.start_block != 0 || self.end_block != 0) {
            return Ok(());
        }
        if !self.wrote {
            return Ok(());
        }
        self.wrote = false;
        let media_id = self.vol.as_ref().map(|v| v.media_id).unwrap_or(0);
        let jm = if zero {
            CreateJobMedia { media_id, ..Default::default() }
        } else {
            CreateJobMedia {
                first_index: self.first_index,
                last_index: self.last_index,
                start_file: self.start_file,
                end_file: self.end_file,
                start_block: self.start_block,
                end_block: self.end_block,
                media_id,
            }
        };
        let answer = self.cat_req(&jm.format(&self.info.job))?;
        if answer != "1000 OK CreateJobMedia\n" {
            return Err(format!("Error creating JobMedia record: {answer}"));
        }
        Ok(())
    }

    /// `WriteBlockToDevice` with `FixupDeviceBlockWriteError` for a full
    /// volume: seals the block (header, number, checksum) and appends it.
    fn write_block(&mut self, payload: &[u8], meta: &SlotMeta) -> Result<(), String> {
        let mut block = vec![0u8; BLKHDR2_LENGTH];
        block.extend_from_slice(payload);
        let len = block.len() as u64;
        let (bytes, max_cat) = self.vol.as_ref().map(|v| (v.cat.bytes, v.cat.max_bytes)).unwrap_or((0, 0));
        let hit_dev = self.dev.max_volume_size > 0 && bytes + len >= self.dev.max_volume_size;
        let hit_cat = max_cat > 0 && bytes + len >= max_cat;
        if hit_dev || hit_cat {
            let cap = if hit_dev { self.dev.max_volume_size } else { max_cat };
            self.jmsg(M_INFO, &format!(
                "User defined maximum volume capacity {} exceeded on device {}.",
                with_commas(cap),
                self.dev.print_name()
            ));
            self.terminate_volume()?;
            let previous = vol_name(&self.vol);
            let (vbytes, vblocks) = self.vol.as_ref().map(|v| (v.cat.bytes, v.cat.blocks)).unwrap_or((0, 0));
            self.jmsg(M_INFO, &format!(
                "End of medium on Volume \"{previous}\" Bytes={} Blocks={} at {}.",
                with_commas(vbytes),
                with_commas(vblocks as u64),
                bareos_time()
            ));
            let saved_block_number = self.block_number;
            self.mount(Some(&previous))?;
            self.block_number = saved_block_number;
            self.jmsg(M_INFO, &format!(
                "New volume \"{}\" mounted on device {} at {}.",
                vol_name(&self.vol),
                self.dev.print_name(),
                bareos_time()
            ));
        }
        bareos_format::volume::seal_block(&mut block, self.block_number, meta.vol_session_id, meta.vol_session_time);
        let started = Instant::now();
        let vol = self.vol.as_mut().ok_or("Attempt to write on closed device")?;
        let addr = vol.addr;
        vol.file.write_all(&block).map_err(|e| {
            format!("Write error at 0:{addr} on device {}. ERR={e}.", self.dev.print_name())
        })?;
        vol.cat.write_time += started.elapsed().as_micros() as i64;
        if self.first_index == 0 && meta.first_index > 0 {
            self.wrote = true;
            self.create_jobmedia(true).map_err(|_| "Was not able to create null job media record.".to_string())?;
            self.first_index = meta.first_index as u32;
            self.start_block = addr as u32;
            self.start_file = (addr >> 32) as u32;
        }
        if meta.last_index > 0 {
            self.last_index = meta.last_index as u32;
        }
        let vol = self.vol.as_mut().ok_or("no volume")?;
        vol.cat.bytes += len;
        vol.cat.blocks += 1;
        vol.cat.writes += 1;
        let last = addr + len - 1;
        self.end_block = last as u32;
        self.end_file = (last >> 32) as u32;
        vol.addr += len;
        self.block_number = self.block_number.wrapping_add(1);
        self.wrote = true;
        Ok(())
    }

    /// `TerminateWritingVolume`: JobMedia of the full volume, VolStatus
    /// Full, catalog update.
    fn terminate_volume(&mut self) -> Result<(), String> {
        self.create_jobmedia(false)?;
        if let Some(v) = self.vol.as_mut() {
            v.cat.status = "Full".into();
            v.cat.files = (v.addr >> 32) as u32;
            let _ = v.file.sync_data();
        }
        self.update_media(false)
    }

    /// `WriteSessionLabel(EOS_LABEL)` into the last partial block, then the
    /// final block write.
    fn end_session(&mut self, payload: &[u8], meta: &SlotMeta) -> Result<(), String> {
        let mut builder = BlockBuilder::from_payload(
            self.dev.max_block_size as usize,
            payload,
            meta.first_index,
            meta.last_index,
            meta.vol_session_id,
            meta.vol_session_time,
        );
        let eos = SessionLabel {
            id: BAREOS_ID.into(),
            ver_num: BAREOS_TAPE_VERSION,
            job_id: self.info.job_id,
            write_btime: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_micros() as i64,
            pool_name: self.info.pool_name.clone(),
            pool_type: self.info.pool_type.clone(),
            job_files: Some(meta.job_files),
            job_bytes: Some(meta.job_bytes),
            start_block: Some(self.start_block),
            end_block: Some(self.end_block),
            start_file: Some(self.start_file),
            end_file: Some(self.end_file),
            job_errors: Some(meta.job_errors),
            job_status: Some(meta.job_status as u32),
            ..self.info.session_label(0)
        };
        let data = eos.serialize();
        let rec = RecordRef {
            vol_session_id: self.info.vol_session_id,
            vol_session_time: self.info.vol_session_time,
            file_index: EOS_LABEL,
            stream: self.info.job_id as i32,
            data: &data,
        };
        if !builder.can_write(data.len()) {
            let m = SlotMeta { first_index: builder.first_index, last_index: builder.last_index, ..*meta };
            let full = builder.payload().to_vec();
            builder = BlockBuilder::new(self.dev.max_block_size as usize);
            self.write_block(&full, &m)?;
        }
        let ok = builder.write_record(&rec, &mut RecordProgress::default());
        debug_assert!(ok, "an EOS label always fits an empty block");
        let m = SlotMeta {
            first_index: builder.first_index,
            last_index: builder.last_index,
            vol_session_id: builder.vol_session_id,
            vol_session_time: builder.vol_session_time,
            ..*meta
        };
        let last = builder.payload().to_vec();
        self.write_block(&last, &m)
    }

    /// `ReleaseDevice`: "Releasing device", the job's JobMedia, the final
    /// catalog update.
    fn release(&mut self) -> Result<(), String> {
        self.jmsg(M_INFO, &format!("Releasing device {}.", self.dev.print_name()));
        if self.vol.is_none() {
            return Ok(());
        }
        if let Some(v) = self.vol.as_mut() {
            v.file.sync_data().map_err(|e| format!("Failed to flush device {}: {e}", self.dev.print_name()))?;
        }
        if self.first_index > 0 {
            self.create_jobmedia(false)?;
        }
        if let Some(v) = self.vol.as_mut() {
            v.cat.files = (v.addr >> 32) as u32;
        }
        self.update_media(false)?;
        self.vol = None;
        Ok(())
    }
}

/// Name of the mounted volume, or empty.
fn vol_name(vol: &Option<Volume>) -> String {
    vol.as_ref().map(|v| v.name.clone()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bareos_format::label::SOS_LABEL;
    use osd_ring::RingConfig;
    use std::collections::HashMap;

    /// A catalog record as odir keeps it, answered as `OK_media`.
    #[derive(Clone, Default)]
    struct Media {
        /// MediaId.
        id: u64,
        /// The counters the SD reported last.
        upd: UpdateMedia,
    }

    /// `1000 OK VolName=…` for `m` with `max_bytes`.
    fn ok_media(m: &Media, max_bytes: u64) -> String {
        let u = &m.upd;
        format!(
            "1000 OK VolName={} VolJobs={} VolFiles={} VolBlocks={} VolBytes={} VolMounts={} VolErrors=0 VolWrites={} \
             MaxVolBytes={max_bytes} VolCapacityBytes=0 VolStatus={} Slot=0 MaxVolJobs=0 MaxVolFiles=0 InChanger=0 \
             VolReadTime=0 VolWriteTime={} EndFile=0 EndBlock=0 LabelType=0 MediaId={} EncryptionKey=*None* MinBlocksize=0 \
             MaxBlocksize=0\n",
            u.volume, u.jobs, u.files, u.blocks, u.bytes, u.mounts, u.writes, u.status, u.write_time, m.id
        )
    }

    /// A fake Director: answers the catalog requests of the writer, creating
    /// volumes `Vol1`, `Vol2`, … in the pool on demand.
    #[derive(Default)]
    pub(crate) struct FakeDirector {
        /// Volumes by name.
        media: HashMap<String, Media>,
        /// Every request line, in order.
        pub(crate) requests: Vec<String>,
        /// Job messages.
        pub(crate) messages: Vec<String>,
        /// MaxVolBytes of new volumes.
        max_bytes: u64,
    }

    impl FakeDirector {
        /// The answer to one mailbox request.
        pub(crate) fn answer(&mut self, req: &[u8], ready: &mut bool) -> Vec<u8> {
            match Request::decode(req).unwrap() {
                Request::Ready => {
                    *ready = true;
                    b"ok".to_vec()
                }
                Request::Jmsg(_, text) => {
                    self.messages.push(text);
                    b"ok".to_vec()
                }
                Request::CatReq(line) => {
                    self.requests.push(line.trim_end().to_string());
                    let word = |k: &str| line.split_whitespace().find_map(|w| w.strip_prefix(k)).unwrap_or("").to_string();
                    if line.contains(" FindMedia=") {
                        let unwanted = word("unwanted_volumes=");
                        let found = self.media.values().find(|m| m.upd.status == "Append" && m.upd.volume != unwanted).cloned();
                        let m = found.unwrap_or_else(|| {
                            let id = self.media.len() as u64 + 1;
                            let m = Media { id, upd: UpdateMedia { volume: format!("Vol{id}"), status: "Unlabeled".into(), ..Default::default() } };
                            self.media.insert(m.upd.volume.clone(), m.clone());
                            m
                        });
                        ok_media(&m, self.max_bytes).into_bytes()
                    } else if line.contains(" GetVolInfo ") {
                        ok_media(&self.media[&word("VolName=")], self.max_bytes).into_bytes()
                    } else if line.contains(" UpdateMedia ") {
                        let name = word("VolName=");
                        let n = |k: &str| word(k).parse::<u64>().unwrap();
                        let m = self.media.get_mut(&name).unwrap();
                        m.upd.jobs = n("VolJobs=") as u32;
                        m.upd.blocks = n("VolBlocks=") as u32;
                        m.upd.bytes = n("VolBytes=");
                        m.upd.mounts = n("VolMounts=") as u32;
                        m.upd.writes = n("VolWrites=") as u32;
                        m.upd.status = word("VolStatus=");
                        ok_media(&self.media[&name], self.max_bytes).into_bytes()
                    } else if line.contains(" CreateJobMedia ") {
                        b"1000 OK CreateJobMedia\n".to_vec()
                    } else {
                        b"1999 unexpected\n".to_vec()
                    }
                }
            }
        }
    }

    /// Runs one job through the ring with a writer thread: SOS, `records`
    /// data records of `size` bytes, End slot. Returns the fake Director.
    pub(crate) fn job(dir: &std::path::Path, ring_name: &str, records: usize, size: usize, max_bytes: u64) -> FakeDirector {
        let info = JobInfo {
            job: "J.2026-11-20_01.00.00_01".into(),
            job_id: 9,
            vol_session_id: 1,
            vol_session_time: 1_795_200_000,
            pool_name: "P".into(),
            pool_type: "Backup".into(),
            media_type: "File".into(),
            storage: "test-sd".into(),
            device: "Dev".into(),
            archive_device: dir.display().to_string(),
            label_media: true,
            max_block_size: 4096,
            label_block_size: 64512,
            max_volume_size: 0,
            job_name: "J".into(),
            client_name: "c".into(),
            fileset_name: "F".into(),
            fileset_md5: "m".into(),
            job_type: 'B' as u32,
            job_level: 'F' as u32,
            bootstrap: String::new(),
        };
        let _ = Ring::remove(ring_name);
        let ring = Ring::create(ring_name, RingConfig { slots: 3, slot_size: 4096, consumers: 1 }).unwrap();
        ring.set_info(info.encode().as_bytes()).unwrap();
        let name = ring_name.to_string();
        let writer = std::thread::spawn(move || run(&name));
        let mut fake = FakeDirector { max_bytes, ..Default::default() };
        let mut ready = false;
        let serve = |fake: &mut FakeDirector, ready: &mut bool| {
            ring.serve_mail(Duration::from_millis(20), |_, req| fake.answer(req, ready));
        };
        while !ready {
            serve(&mut fake, &mut ready);
        }
        // the producer side: blocks as fd_session builds them
        let mut builder = BlockBuilder::new(4096);
        let sos = info.session_label(1).serialize();
        let mut recs: Vec<(i32, i32, Vec<u8>)> = vec![(SOS_LABEL, 9, sos)];
        for i in 0..records {
            recs.push(((i / 2 + 1) as i32, if i % 2 == 0 { 1 } else { 2 }, vec![(i % 251) as u8; size]));
        }
        let publish = |builder: &BlockBuilder, kind: SlotKind, fake: &mut FakeDirector, ready: &mut bool| {
            let slot = loop {
                match ring.claim(Duration::from_millis(20)) {
                    Ok(s) => break s,
                    Err(osd_ring::RingError::Timeout) => serve(fake, ready),
                    Err(e) => panic!("{e}"),
                }
            };
            let mut slot = slot;
            let p = builder.payload();
            slot.data[..p.len()].copy_from_slice(p);
            slot.meta = SlotMeta {
                kind: kind as u64,
                first_index: builder.first_index,
                last_index: builder.last_index,
                vol_session_id: 1,
                vol_session_time: 1_795_200_000,
                job_files: (records / 2) as u32,
                job_bytes: (records * size) as u64,
                job_status: b'T',
                job_errors: 0,
            }
            .to_words();
            slot.publish(p.len());
        };
        for (fi, stream, data) in &recs {
            let r = RecordRef { vol_session_id: 1, vol_session_time: 1_795_200_000, file_index: *fi, stream: *stream, data };
            let mut p = RecordProgress::default();
            while !builder.write_record(&r, &mut p) {
                publish(&builder, SlotKind::Data, &mut fake, &mut ready);
                builder = BlockBuilder::new(4096);
            }
        }
        publish(&builder, SlotKind::End, &mut fake, &mut ready);
        ring.finish(State::Done);
        while !writer.is_finished() {
            serve(&mut fake, &mut ready);
        }
        assert_eq!(writer.join().unwrap(), 0, "writer failed: {:?}", fake.messages);
        assert_eq!(ring.consumer_state(CONSUMER), State::Done);
        Ring::remove(ring_name).unwrap();
        fake
    }

    #[test]
    fn one_volume_like_bareos_sd() {
        let dir = std::env::temp_dir().join(format!("osd-writer-one-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = job(&dir, &format!("osd-writertest-{}-one", std::process::id()), 20, 1000, 0);
        let kinds: Vec<&str> = fake.requests.iter().map(|r| r.split_whitespace().nth(2).unwrap()).collect();
        assert_eq!(kinds, [
            "FindMedia=1", "GetVolInfo", "UpdateMedia", "GetVolInfo", "UpdateMedia", "UpdateMedia", "CreateJobMedia",
            "CreateJobMedia", "UpdateMedia",
        ]);
        assert!(fake.requests[2].contains(" relabel=1 ") && fake.requests[2].contains(" VolStatus=Append "));
        assert!(fake.requests[6].contains(" FirstIndex=0 LastIndex=0 "));
        assert!(fake.requests[7].contains(" FirstIndex=1 LastIndex=10 StartFile=0 EndFile=0 "));
        let volume = std::fs::read(dir.join("Vol1")).unwrap();
        let s = bareos_format::volume::summarize(&volume).unwrap();
        assert_eq!(s.label.volume_name, "Vol1");
        assert_eq!(s.sessions.len(), 1);
        let end = s.sessions[0].end.as_ref().unwrap();
        assert_eq!((end.job_files, end.job_status), (Some(10), Some('T' as u32)));
        assert_eq!(fake.media["Vol1"].upd.bytes, volume.len() as u64);
        assert!(fake.messages.iter().any(|m| m.starts_with("Labeled new Volume \"Vol1\"")));
        assert!(fake.messages.iter().any(|m| m.starts_with("Releasing device \"Dev\"")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_full_volume_continues_on_the_next() {
        let dir = std::env::temp_dir().join(format!("osd-writer-two-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = job(&dir, &format!("osd-writertest-{}-two", std::process::id()), 30, 1000, 20_000);
        assert_eq!(fake.media["Vol1"].upd.status, "Full");
        assert_eq!(fake.media["Vol2"].upd.status, "Append");
        assert!(fake.messages.iter().any(|m| m.starts_with("User defined maximum volume capacity 20,000 exceeded")));
        assert!(fake.messages.iter().any(|m| m.starts_with("New volume \"Vol2\" mounted on device \"Dev\"")));
        let jobmedia: Vec<&String> = fake.requests.iter().filter(|r| r.contains("CreateJobMedia") && !r.contains("FirstIndex=0 ")).collect();
        assert_eq!(jobmedia.len(), 2, "{jobmedia:?}");
        assert!(jobmedia[1].contains(" LastIndex=15 "), "{jobmedia:?}");
        let v1 = std::fs::read(dir.join("Vol1")).unwrap();
        let v2 = std::fs::read(dir.join("Vol2")).unwrap();
        assert!(v1.len() < 20_000);
        let s1 = bareos_format::volume::summarize(&v1).unwrap();
        let s2 = bareos_format::volume::summarize(&v2).unwrap();
        assert!(s1.sessions[0].end.is_none());
        assert!(s2.sessions.is_empty(), "the SOS label is only on the first volume");
        let last = bareos_format::volume::read_volume(&v2).unwrap();
        assert_eq!(last.records.last().unwrap().file_index, bareos_format::label::EOS_LABEL);
        std::fs::remove_dir_all(&dir).ok();
    }
}
