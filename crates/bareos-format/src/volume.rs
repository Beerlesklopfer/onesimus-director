//! Volume-Blockformat: Blockkopf `BB02`, Prüfsumme, Datensätze im Block.
//!
//! Zweck: Ein Datei-Volume des SD ist eine lückenlose Folge von Blöcken.
//! Jeder Block hat einen Kopf und enthält Datensätze; ein Datensatz, der
//! nicht mehr in den Block passt, läuft mit einem Fortsetzungskopf im
//! nächsten Block weiter. Dieses Modul schreibt und liest das byte-genau
//! wie bareos-sd 25, ohne IO: [`BlockBuilder`] füllt einen Block wie
//! `WriteRecordToBlock`, [`BlockBuilder::finish`] setzt den Kopf mit
//! Prüfsumme wie `SerBlockHeader`, [`RecordAssembler`] setzt Datensätze
//! aus Blöcken wieder zusammen (je Sitzung, auch über Volumes hinweg, für
//! den Restore), [`read_volume`] zerlegt ein ganzes Volume damit,
//! [`summarize`] fasst Label, Sitzungen und
//! Dateien zusammen (Prüfung im Gerüst, Vorstufe für `bls` in Stufe 2).
//!
//! Format (alle Zahlen Big Endian, `lib/serial.cc`):
//!
//! | Teil | Länge | Felder |
//! |---|---|---|
//! | Blockkopf `BB02` | 24 | `CheckSum` u32, `block_len` u32, `BlockNumber` u32, `"BB02"`, `VolSessionId` u32, `VolSessionTime` u32 |
//! | Datensatzkopf (Version 2) | 12 | `FileIndex` i32, `Stream` i32, `data_len` u32 (die noch fehlenden Bytes des Datensatzes) |
//!
//! - Die Prüfsumme ist CRC-32 (Polynom `0xEDB88320`, wie zlib) über den
//!   Block ab Byte 4 bis `block_len` (`stored/block.cc` `SerBlockHeader`,
//!   `stored/crc32/crc32.cc`).
//! - `block_len` ist die Zahl der belegten Bytes. Ein Datei-Device schreibt
//!   genau so viele (kein Auffüllen, `CAP_ADJWRITESIZE` fehlt bei Dateien).
//! - Passt ein Datensatzkopf nicht mehr ganz, endet der Block. Passt der
//!   Kopf, aber kein Datenbyte, folgt im nächsten Block ein Fortsetzungskopf
//!   mit negativem Stream (`st_header_cont`).
//! - `VolSessionId`/`VolSessionTime` im Blockkopf stammen vom zuletzt in den
//!   Block geschriebenen Datensatzkopf.
//!
//! Version 1 (`BB01`, Kopf 16 Byte, Datensatzkopf 20 Byte mit Sitzung) liest
//! [`read_volume`] ebenfalls; geschrieben wird nur Version 2.
//!
//! Bareos-Gegenstück: `core/src/stored/block.h` (Konstanten),
//! `stored/block.cc` (`SerBlockHeader`, `unSerBlockHeader`,
//! `WriteBlockToDev`), `stored/record.cc` (`WriteHeaderToBlock`,
//! `WriteDataToBlock`, `WriteRecordToBlock`, `CanWriteRecordToBlock`,
//! `ReadRecordFromBlock`), Bareos `18be77e6`.

use crate::label::{SessionLabel, VolumeLabel, EOS_LABEL, SOS_LABEL, VOL_LABEL, PRE_LABEL};

/// `BLKHDR2_LENGTH`: length of a `BB02` block header.
pub const BLKHDR2_LENGTH: usize = 24;
/// `BLKHDR1_LENGTH`: length of a `BB01` block header.
pub const BLKHDR1_LENGTH: usize = 16;
/// `BLKHDR_CS_LENGTH`: the checksum field, excluded from the checksum.
pub const BLKHDR_CS_LENGTH: usize = 4;
/// `RECHDR2_LENGTH`: length of a version 2 record header.
pub const RECHDR2_LENGTH: usize = 12;
/// `RECHDR1_LENGTH`: length of a version 1 record header.
pub const RECHDR1_LENGTH: usize = 20;
/// `DEFAULT_BLOCK_SIZE`: 64 512 bytes (126 × 512).
pub const DEFAULT_BLOCK_SIZE: usize = 512 * 126;
/// `MAX_BLOCK_LENGTH`: sanity limit of a block.
pub const MAX_BLOCK_LENGTH: usize = 20_000_000;

/// CRC-32 table of the zlib polynomial `0xEDB88320`.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
};

/// `crc32_fast(data, len, 0)` of `stored/crc32/crc32.cc` (zlib CRC-32).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc = CRC_TABLE[((crc ^ u32::from(b)) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

/// A block header as read from a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    /// `CheckSum` as stored.
    pub checksum: u32,
    /// `block_len`: bytes of the block including the header.
    pub block_len: u32,
    /// `BlockNumber`.
    pub block_number: u32,
    /// Block version: 1 (`BB01`) or 2 (`BB02`).
    pub version: u8,
    /// `VolSessionId` (version 2 only, else 0).
    pub vol_session_id: u32,
    /// `VolSessionTime` (version 2 only, else 0).
    pub vol_session_time: u32,
}

impl BlockHeader {
    /// Length of this header on the volume.
    pub fn len(&self) -> usize {
        if self.version == 1 { BLKHDR1_LENGTH } else { BLKHDR2_LENGTH }
    }

    /// Always false: a header has a fixed length (clippy's `len` companion).
    pub fn is_empty(&self) -> bool {
        false
    }

    /// `unSerBlockHeader`: the header at the start of `buf`, checked for
    /// its id and length. The checksum is checked by [`verify_block`].
    pub fn parse(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < BLKHDR1_LENGTH {
            return Err(format!("block header needs {BLKHDR1_LENGTH} bytes, got {}", buf.len()));
        }
        let u = |at: usize| u32::from_be_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]]);
        let id = &buf[12..16];
        let (version, sid, stime) = match id {
            b"BB01" => (1, 0, 0),
            b"BB02" => {
                if buf.len() < BLKHDR2_LENGTH {
                    return Err("short BB02 header".into());
                }
                (2, u(16), u(20))
            }
            other => return Err(format!("Wanted ID: \"BB02\", got \"{}\"", String::from_utf8_lossy(other))),
        };
        let header = BlockHeader { checksum: u(0), block_len: u(4), block_number: u(8), version, vol_session_id: sid, vol_session_time: stime };
        if header.block_len as usize > MAX_BLOCK_LENGTH {
            return Err(format!("Block length {} is insane (too large)", header.block_len));
        }
        if (header.block_len as usize) < header.len() {
            return Err(format!("Block length {} shorter than its header", header.block_len));
        }
        Ok(header)
    }
}

/// Checks the checksum of the whole block `block` (exactly `block_len` bytes).
pub fn verify_block(block: &[u8]) -> Result<BlockHeader, String> {
    let header = BlockHeader::parse(block)?;
    let len = header.block_len as usize;
    if block.len() < len {
        return Err(format!("block {} truncated: {} of {len} bytes", header.block_number, block.len()));
    }
    let calc = crc32(&block[BLKHDR_CS_LENGTH..len]);
    if calc != header.checksum {
        return Err(format!(
            "Block checksum mismatch in block={} len={len}: calc={calc:x} blk={:x}",
            header.block_number, header.checksum
        ));
    }
    Ok(header)
}

/// One record to be written: what `DeviceRecord` carries into
/// `WriteRecordToBlock`.
#[derive(Debug, Clone, Copy)]
pub struct RecordRef<'a> {
    /// `VolSessionId` of the writing job.
    pub vol_session_id: u32,
    /// `VolSessionTime` of the writing job.
    pub vol_session_time: u32,
    /// `FileIndex`; negative for labels.
    pub file_index: i32,
    /// Stream id (`STREAM_*`), JobId for session labels.
    pub stream: i32,
    /// The record's data.
    pub data: &'a [u8],
}

/// `rec_state` of `stored/record.h`: where a partly written record stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecordState {
    /// `st_none`: nothing written yet.
    #[default]
    None,
    /// `st_header`: the header is to be written.
    Header,
    /// `st_header_cont`: a continuation header is to be written.
    HeaderCont,
    /// `st_data`: data bytes are to be written.
    Data,
}

/// The write progress of one record across blocks (`rec->state`,
/// `rec->remainder`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordProgress {
    /// The state machine of `WriteRecordToBlock`.
    pub state: RecordState,
    /// Bytes of the record not yet in a block.
    pub remainder: u32,
}

/// A block being filled: `DeviceBlock` with `binbuf`, `FirstIndex`,
/// `LastIndex` and the session of the last record header.
#[derive(Debug, Clone)]
pub struct BlockBuilder {
    /// The block buffer; the first 24 bytes are the header space.
    buf: Vec<u8>,
    /// `buf_len`: the block size.
    buf_len: usize,
    /// `FirstIndex`: first positive FileIndex in this block, 0 if none.
    pub first_index: i32,
    /// `LastIndex`: last positive FileIndex in this block.
    pub last_index: i32,
    /// `VolSessionId` of the last record header written.
    pub vol_session_id: u32,
    /// `VolSessionTime` of the last record header written.
    pub vol_session_time: u32,
}

impl BlockBuilder {
    /// An empty block of `buf_len` bytes (`new_block`, `EmptyBlock`).
    pub fn new(buf_len: usize) -> Self {
        let mut buf = Vec::with_capacity(buf_len);
        buf.resize(BLKHDR2_LENGTH, 0);
        BlockBuilder { buf, buf_len, first_index: 0, last_index: 0, vol_session_id: 0, vol_session_time: 0 }
    }

    /// A block continuing from `payload` (bytes after the header) with the
    /// given indexes and session, e.g. a partial block handed over by
    /// another process.
    pub fn from_payload(buf_len: usize, payload: &[u8], first_index: i32, last_index: i32, vol_session_id: u32, vol_session_time: u32) -> Self {
        let mut b = BlockBuilder::new(buf_len);
        b.buf.extend_from_slice(payload);
        b.first_index = first_index;
        b.last_index = last_index;
        b.vol_session_id = vol_session_id;
        b.vol_session_time = vol_session_time;
        b
    }

    /// `binbuf`: bytes in the block including the header space.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True when the block holds no record (`binbuf <= WRITE_BLKHDR_LENGTH`).
    pub fn is_empty(&self) -> bool {
        self.buf.len() <= BLKHDR2_LENGTH
    }

    /// The block size it was made with.
    pub fn buf_len(&self) -> usize {
        self.buf_len
    }

    /// The records written so far, without the header space.
    pub fn payload(&self) -> &[u8] {
        &self.buf[BLKHDR2_LENGTH..]
    }

    /// `BlockWriteNavail`.
    fn navail(&self) -> usize {
        self.buf_len - self.buf.len()
    }

    /// `CanWriteRecordToBlock` for a record with `remainder` bytes.
    pub fn can_write(&self, remainder: usize) -> bool {
        self.navail() >= RECHDR2_LENGTH + remainder
    }

    /// `WriteHeaderToBlock`; false when the header does not fit.
    fn write_header(&mut self, rec: &RecordRef<'_>, stream: i32, remainder: u32) -> bool {
        if self.navail() < RECHDR2_LENGTH {
            return false;
        }
        self.vol_session_id = rec.vol_session_id;
        self.vol_session_time = rec.vol_session_time;
        self.buf.extend_from_slice(&rec.file_index.to_be_bytes());
        self.buf.extend_from_slice(&stream.to_be_bytes());
        self.buf.extend_from_slice(&remainder.to_be_bytes());
        if rec.file_index > 0 {
            if self.first_index == 0 {
                self.first_index = rec.file_index;
            }
            self.last_index = rec.file_index;
        }
        true
    }

    /// `WriteRecordToBlock`: writes as much of `rec` as fits. True when
    /// the whole record is in the block; false when the block must be
    /// written ([`BlockBuilder::finish`]) and this called again with the
    /// same `progress`.
    pub fn write_record(&mut self, rec: &RecordRef<'_>, progress: &mut RecordProgress) -> bool {
        loop {
            match progress.state {
                RecordState::None => {
                    progress.state = RecordState::Header;
                    progress.remainder = rec.data.len() as u32;
                }
                RecordState::Header => {
                    if !self.write_header(rec, rec.stream, progress.remainder) {
                        return false;
                    }
                    if self.navail() == 0 {
                        progress.state = RecordState::HeaderCont;
                        return false;
                    }
                    progress.state = RecordState::Data;
                }
                RecordState::HeaderCont => {
                    let ok = self.write_header(rec, -rec.stream, progress.remainder);
                    assert!(ok, "couldn't write continuation header");
                    progress.state = RecordState::Data;
                    if self.navail() == 0 {
                        return false;
                    }
                }
                RecordState::Data => {
                    if progress.remainder > 0 {
                        let start = rec.data.len() - progress.remainder as usize;
                        let n = (progress.remainder as usize).min(self.navail());
                        self.buf.extend_from_slice(&rec.data[start..start + n]);
                        progress.remainder -= n as u32;
                        if progress.remainder > 0 {
                            progress.state = RecordState::HeaderCont;
                            return false;
                        }
                    }
                    progress.remainder = 0;
                    progress.state = RecordState::None;
                    return true;
                }
            }
        }
    }

    /// `SerBlockHeader` with checksum: the finished block (exactly
    /// `binbuf` bytes) with `block_number`. The builder is emptied
    /// (`EmptyBlock`).
    pub fn finish(&mut self, block_number: u32) -> Vec<u8> {
        let mut block = std::mem::replace(&mut self.buf, Vec::with_capacity(self.buf_len));
        self.buf.resize(BLKHDR2_LENGTH, 0);
        seal_block(&mut block, block_number, self.vol_session_id, self.vol_session_time);
        self.first_index = 0;
        self.last_index = 0;
        block
    }
}

/// Writes the `BB02` header into the first 24 bytes of `block` (its whole
/// length is `block_len`) and the checksum over the rest (`SerBlockHeader`).
pub fn seal_block(block: &mut [u8], block_number: u32, vol_session_id: u32, vol_session_time: u32) {
    let len = block.len() as u32;
    block[0..4].copy_from_slice(&0u32.to_be_bytes());
    block[4..8].copy_from_slice(&len.to_be_bytes());
    block[8..12].copy_from_slice(&block_number.to_be_bytes());
    block[12..16].copy_from_slice(b"BB02");
    block[16..20].copy_from_slice(&vol_session_id.to_be_bytes());
    block[20..24].copy_from_slice(&vol_session_time.to_be_bytes());
    let checksum = crc32(&block[BLKHDR_CS_LENGTH..]);
    block[0..4].copy_from_slice(&checksum.to_be_bytes());
}

/// One record read back from a volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolRecord {
    /// Number of the block the record starts in.
    pub block_number: u32,
    /// Byte offset of that block in the volume.
    pub block_offset: u64,
    /// `VolSessionId`.
    pub vol_session_id: u32,
    /// `VolSessionTime`.
    pub vol_session_time: u32,
    /// `FileIndex`.
    pub file_index: i32,
    /// Stream (positive, continuation sign removed).
    pub stream: i32,
    /// The whole data, reassembled across blocks.
    pub data: Vec<u8>,
}

/// A whole volume read back: blocks and records.
#[derive(Debug, Clone, Default)]
pub struct VolumeContents {
    /// The block headers in order, with their offsets.
    pub blocks: Vec<(u64, BlockHeader)>,
    /// The records in order.
    pub records: Vec<VolRecord>,
    /// Bytes missing from the last record when it continues on the next
    /// volume.
    pub truncated: Option<usize>,
}

/// Reassembles records from blocks, one partial record per session
/// (`ReadRecordFromBlock` with `READ_CTX` and `ReadContextSetRecord`).
///
/// A record split across blocks continues with a header whose stream is
/// negated; the pieces are joined for the same session and stream. The
/// assembler keeps its state across volumes, so a record that a job
/// continued on its next volume comes back whole when the blocks of both
/// volumes are pushed in order.
#[derive(Debug, Default)]
pub struct RecordAssembler {
    /// Records still missing data, with the number of bytes missing; at
    /// most one per session (`VolSessionId`, `VolSessionTime`).
    pending: Vec<(VolRecord, usize)>,
}

impl RecordAssembler {
    /// An assembler with nothing pending.
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one block (exactly `block_len` bytes, checksum verified by the
    /// caller or with `verify`) found at byte `offset` of its volume and
    /// returns its header and the records that are complete now, in block
    /// order. A continuation without a pending first part comes back as it
    /// is (the start of a volume read on its own).
    pub fn push_block(&mut self, block: &[u8], offset: u64, verify: bool) -> Result<(BlockHeader, Vec<VolRecord>), String> {
        let header = if verify { verify_block(block)? } else { BlockHeader::parse(block)? };
        let end = (header.block_len as usize).min(block.len());
        let mut out = Vec::new();
        let mut at = header.len();
        let rhl = if header.version == 1 { RECHDR1_LENGTH } else { RECHDR2_LENGTH };
        let u = |p: usize| u32::from_be_bytes([block[p], block[p + 1], block[p + 2], block[p + 3]]);
        while end - at >= rhl {
            let (sid, stime, h) = if header.version == 1 {
                (u(at), u(at + 4), at + 8)
            } else {
                (header.vol_session_id, header.vol_session_time, at)
            };
            let file_index = u(h) as i32;
            let stream = u(h + 4) as i32;
            let data_bytes = u(h + 8) as usize;
            at += rhl;
            if data_bytes >= MAX_BLOCK_LENGTH {
                return Err(format!("block {}: record length {data_bytes} fails the sanity check", header.block_number));
            }
            let slot = self.pending.iter().position(|(r, _)| r.vol_session_id == sid && r.vol_session_time == stime);
            let continued = slot.filter(|&i| stream < 0 && self.pending[i].0.stream == -stream).map(|i| self.pending.swap_remove(i).0);
            if continued.is_none()
                && let Some(i) = slot
            {
                // a new record of the session: the unfinished one is dropped
                self.pending.swap_remove(i);
            }
            let mut record = continued.unwrap_or(VolRecord {
                block_number: header.block_number,
                block_offset: offset,
                vol_session_id: sid,
                vol_session_time: stime,
                file_index,
                stream: stream.abs(),
                data: Vec::new(),
            });
            let n = data_bytes.min(end - at);
            record.data.extend_from_slice(&block[at..at + n]);
            at += n;
            if n < data_bytes {
                self.pending.push((record, data_bytes - n));
            } else {
                out.push(record);
            }
        }
        Ok((header, out))
    }

    /// Takes the records still missing data, with the bytes missing.
    pub fn take_pending(&mut self) -> Vec<(VolRecord, usize)> {
        std::mem::take(&mut self.pending)
    }
}

/// Reads every block of a file volume (`ReadBlockFromDev` with checksum,
/// `ReadRecordFromBlock`) and reassembles records split across blocks. A
/// volume of a job that continues on another volume may end inside a
/// record ([`VolumeContents::truncated`]) and the next volume starts with a
/// continuation header; such pieces come back as they are.
pub fn read_volume(volume: &[u8]) -> Result<VolumeContents, String> {
    let mut out = VolumeContents::default();
    let mut offset = 0usize;
    let mut assembler = RecordAssembler::new();
    while offset < volume.len() {
        let header = BlockHeader::parse(&volume[offset..]).map_err(|e| format!("at byte {offset}: {e}"))?;
        let end = (offset + header.block_len as usize).min(volume.len());
        let (header, records) = assembler.push_block(&volume[offset..end], offset as u64, true).map_err(|e| format!("at byte {offset}: {e}"))?;
        out.blocks.push((offset as u64, header));
        out.records.extend(records);
        offset += header.block_len as usize;
    }
    // A record that continues on the next volume (the job spanned volumes)
    // is returned with the bytes this volume holds.
    for (rec, missing) in assembler.take_pending() {
        out.truncated = Some(missing);
        out.records.push(rec);
    }
    Ok(out)
}

/// One job session on a volume: its SOS label and, if written, EOS label.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    /// The Start-of-Session label.
    pub start: SessionLabel,
    /// The End-of-Session label, `None` if missing.
    pub end: Option<SessionLabel>,
    /// `VolSessionId` of the session.
    pub vol_session_id: u32,
    /// `VolSessionTime` of the session.
    pub vol_session_time: u32,
}

/// What [`summarize`] finds on a volume.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeSummary {
    /// The volume label (first record).
    pub label: VolumeLabel,
    /// Number of blocks, all with good checksums.
    pub blocks: usize,
    /// Number of records.
    pub records: usize,
    /// The sessions in order.
    pub sessions: Vec<SessionSummary>,
    /// Sorted, distinct FileIndexes with an attributes record
    /// (`STREAM_UNIX_ATTRIBUTES`/`_EX`).
    pub attribute_file_indexes: Vec<i32>,
    /// File names from the attribute records, in volume order.
    pub file_names: Vec<String>,
}

impl std::fmt::Display for VolumeSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let l = &self.label;
        writeln!(
            f,
            "Volume Label: Id={:?} VerNum={} VolName={} Pool={} PoolType={} MediaType={} Host={} Prog={} {}",
            l.id.trim_end(), l.ver_num, l.volume_name, l.pool_name, l.pool_type, l.media_type, l.host_name, l.label_prog, l.prog_version
        )?;
        writeln!(f, "{} blocks (checksums OK), {} records", self.blocks, self.records)?;
        for s in &self.sessions {
            write!(f, "Session VolSessionId={} VolSessionTime={}: SOS JobId={} Job={} Client={} FileSet={} Pool={}",
                s.vol_session_id, s.vol_session_time, s.start.job_id, s.start.job, s.start.client_name, s.start.fileset_name, s.start.pool_name)?;
            match &s.end {
                Some(e) => writeln!(f, "; EOS JobFiles={} JobBytes={} StartBlock={} EndBlock={} JobStatus={}",
                    e.job_files.unwrap_or(0), e.job_bytes.unwrap_or(0), e.start_block.unwrap_or(0), e.end_block.unwrap_or(0),
                    e.job_status.and_then(char::from_u32).unwrap_or('?'))?,
                None => writeln!(f, "; no EOS")?,
            }
        }
        writeln!(f, "Files: {}", self.file_names.join(", "))
    }
}

/// Reads a volume and summarizes label, sessions and files.
pub fn summarize(volume: &[u8]) -> Result<VolumeSummary, String> {
    let contents = read_volume(volume)?;
    let first = contents.records.first().ok_or("volume has no records")?;
    if first.file_index != VOL_LABEL && first.file_index != PRE_LABEL {
        return Err(format!("first record is not a volume label: FileIndex {}", first.file_index));
    }
    let label = VolumeLabel::parse(&first.data)?;
    let mut sessions: Vec<SessionSummary> = Vec::new();
    let mut attrs = Vec::new();
    let mut file_names = Vec::new();
    for rec in &contents.records[1..] {
        match rec.file_index {
            SOS_LABEL => sessions.push(SessionSummary {
                start: SessionLabel::parse(&rec.data, false)?,
                end: None,
                vol_session_id: rec.vol_session_id,
                vol_session_time: rec.vol_session_time,
            }),
            EOS_LABEL => {
                let end = SessionLabel::parse(&rec.data, true)?;
                if let Some(s) = sessions.iter_mut().rev().find(|s| s.vol_session_id == rec.vol_session_id && s.vol_session_time == rec.vol_session_time) {
                    s.end = Some(end);
                }
            }
            fi if fi > 0 => {
                let masked = rec.stream & crate::stream::STREAMMASK_TYPE;
                if masked == crate::stream::STREAM_UNIX_ATTRIBUTES || masked == crate::stream::STREAM_UNIX_ATTRIBUTES_EX {
                    attrs.push(fi);
                    // "<FileIndex> <Type> <Filename>\0<Attributes>\0…"
                    let mut parts = rec.data.splitn(3, |&b| b == b' ');
                    let _ = (parts.next(), parts.next());
                    if let Some(rest) = parts.next() {
                        let name = rest.split(|&b| b == 0).next().unwrap_or_default();
                        file_names.push(String::from_utf8_lossy(name).into_owned());
                    }
                }
            }
            _ => {}
        }
    }
    attrs.sort_unstable();
    attrs.dedup();
    Ok(VolumeSummary {
        label,
        blocks: contents.blocks.len(),
        records: contents.records.len(),
        sessions,
        attribute_file_indexes: attrs,
        file_names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_zlibs() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// A record of `len` bytes with FileIndex 1, stream 2, session 7/9.
    fn rec(data: &[u8]) -> RecordRef<'_> {
        RecordRef { vol_session_id: 7, vol_session_time: 9, file_index: 1, stream: 2, data }
    }

    #[test]
    fn one_small_record_gives_known_bytes() {
        let mut b = BlockBuilder::new(DEFAULT_BLOCK_SIZE);
        let mut p = RecordProgress::default();
        assert!(b.write_record(&rec(b"abc"), &mut p));
        let block = b.finish(5);
        // header (checksum filled below), record header, data
        let mut expected = vec![0, 0, 0, 0, 0, 0, 0, 39, 0, 0, 0, 5, b'B', b'B', b'0', b'2', 0, 0, 0, 7, 0, 0, 0, 9];
        expected.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, b'a', b'b', b'c']);
        let cs = crc32(&expected[4..]);
        expected[0..4].copy_from_slice(&cs.to_be_bytes());
        assert_eq!(block, expected);
        assert!(b.is_empty() && b.first_index == 0);
    }

    #[test]
    fn records_split_with_continuation_headers_and_read_back() {
        // 24 header + 12 record header + 20 data = 56: the second record's
        // header fits only partly -> new block.
        let size = 64;
        let data1 = [1u8; 20];
        let data2: Vec<u8> = (0..50).collect();
        let mut blocks = Vec::new();
        let mut b = BlockBuilder::new(size);
        let mut n = 0;
        for (fi, data) in [(1, &data1[..]), (2, &data2[..])] {
            let r = RecordRef { vol_session_id: 1, vol_session_time: 2, file_index: fi, stream: 2, data };
            let mut p = RecordProgress::default();
            while !b.write_record(&r, &mut p) {
                blocks.push(b.finish(n));
                n += 1;
            }
        }
        blocks.push(b.finish(n));
        // block 0: rec1 complete (56 bytes, 8 left < 12); block 1: header of
        // rec2 + 28 bytes; block 2: continuation header (stream -2) + 22 bytes
        assert_eq!(blocks.iter().map(Vec::len).collect::<Vec<_>>(), vec![56, 64, 58]);
        assert_eq!(&blocks[1][24..36], &[0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 50]);
        assert_eq!(&blocks[2][24..36], &[0, 0, 0, 2, 0xff, 0xff, 0xff, 0xfe, 0, 0, 0, 22]);
        let volume = blocks.concat();
        let contents = read_volume(&volume).unwrap();
        assert_eq!(contents.blocks.len(), 3);
        assert_eq!(contents.records.len(), 2);
        assert_eq!(contents.records[1].data, data2);
        assert_eq!(contents.records[1].stream, 2);
        assert_eq!(contents.records[1].block_number, 1);
    }

    #[test]
    fn header_that_exactly_fills_the_block_continues_with_a_cont_header() {
        // 24 + 12 = 36: header fits, no data byte -> st_header_cont
        let mut b = BlockBuilder::new(36);
        let mut p = RecordProgress::default();
        let r = rec(b"xy");
        assert!(!b.write_record(&r, &mut p));
        assert_eq!(p.state, RecordState::HeaderCont);
        let first = b.finish(0);
        assert_eq!(&first[24..36], &[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 2]);
        let mut b2 = BlockBuilder::new(64);
        assert!(b2.write_record(&r, &mut p));
        let second = b2.finish(1);
        assert_eq!(&second[24..38], &[0, 0, 0, 1, 0xff, 0xff, 0xff, 0xfe, 0, 0, 0, 2, b'x', b'y']);
        let contents = read_volume(&[first, second].concat()).unwrap();
        assert_eq!(contents.records.len(), 1);
        assert_eq!(contents.records[0].data, b"xy");
    }

    #[test]
    fn corrupt_checksum_is_reported() {
        let mut b = BlockBuilder::new(100);
        assert!(b.write_record(&rec(b"abc"), &mut RecordProgress::default()));
        let mut block = b.finish(0);
        block[30] ^= 1;
        let e = read_volume(&block).unwrap_err();
        assert!(e.contains("checksum mismatch"), "{e}");
        assert!(BlockHeader::parse(b"0000000000000000BB03").is_err());
    }
    /// The label block of a volume bareos-sd 25.1.3 wrote in the rig
    /// (`tests/osd.rs` with `ONESIMUS_RIG_SD=/usr/sbin/bareos-sd`, 20.11.2026).
    /// Only the host name was replaced by one of the same length, and the
    /// checksum recomputed with Python's `zlib.crc32`.
    const BAREOS_LABEL_BLOCK: &[u8] = include_bytes!("../testdata/bareos-sd-25-label-block.bin");

    /// The last block (number 12, 9321 bytes) of the same volume, verbatim:
    /// the tail of `large.bin`, the remaining files and the EOS label.
    const BAREOS_LAST_BLOCK: &[u8] = include_bytes!("../testdata/bareos-sd-25-last-block.bin");

    #[test]
    fn bareos_sd_label_block_is_rebuilt_byte_for_byte() {
        let contents = read_volume(BAREOS_LABEL_BLOCK).unwrap();
        assert_eq!(contents.blocks.len(), 1);
        let (_, header) = contents.blocks[0];
        assert_eq!((header.block_len, header.block_number, header.vol_session_id), (230, 0, 1));
        let rec = &contents.records[0];
        // WriteNewVolumeLabelToDev: FileIndex VOL_LABEL, Stream 0
        assert_eq!((rec.file_index, rec.stream, rec.data.len()), (crate::label::VOL_LABEL, 0, 194));
        let label = VolumeLabel::parse(&rec.data).unwrap();
        assert_eq!(label.volume_name, "VVol-0001");
        assert_eq!(label.label_prog, "vsd"); // my_name: the Storage resource name
        assert_eq!(label.prog_version, "Ver. 25.1.3~pre24.0e7742c39 11 November 2026");
        let data = label.serialize();
        assert_eq!(data, rec.data);
        let mut b = BlockBuilder::new(DEFAULT_BLOCK_SIZE);
        let r = RecordRef { vol_session_id: header.vol_session_id, vol_session_time: header.vol_session_time,
            file_index: crate::label::VOL_LABEL, stream: 0, data: &data };
        assert!(b.write_record(&r, &mut RecordProgress::default()));
        assert_eq!(b.finish(0), BAREOS_LABEL_BLOCK);
    }

    #[test]
    fn bareos_sd_last_block_is_rebuilt_byte_for_byte() {
        let header = verify_block(BAREOS_LAST_BLOCK).unwrap();
        assert_eq!((header.block_len, header.block_number), (9321, 12));
        // Walk the raw record headers: the first one continues `large.bin`.
        let mut at = BLKHDR2_LENGTH;
        let mut b = BlockBuilder::new(1 << 20);
        let mut eos = None;
        while BAREOS_LAST_BLOCK.len() - at >= RECHDR2_LENGTH {
            let h = &BAREOS_LAST_BLOCK[at..at + 12];
            let fi = i32::from_be_bytes(h[0..4].try_into().unwrap());
            let stream = i32::from_be_bytes(h[4..8].try_into().unwrap());
            let len = u32::from_be_bytes(h[8..12].try_into().unwrap()) as usize;
            let data = &BAREOS_LAST_BLOCK[at + 12..at + 12 + len];
            let r = RecordRef { vol_session_id: header.vol_session_id, vol_session_time: header.vol_session_time,
                file_index: fi, stream: stream.abs(), data };
            let mut p = if stream < 0 {
                RecordProgress { state: RecordState::HeaderCont, remainder: len as u32 }
            } else {
                RecordProgress::default()
            };
            if fi == crate::label::EOS_LABEL {
                eos = Some(SessionLabel::parse(data, true).unwrap());
                assert_eq!(eos.as_ref().unwrap().serialize(), data);
                assert!(b.can_write(len));
            }
            assert!(b.write_record(&r, &mut p));
            at += 12 + len;
        }
        let eos = eos.expect("EOS label in the last block");
        assert_eq!((eos.job_id, eos.job_files, eos.job_status), (1, Some(13), Some('T' as u32)));
        assert_eq!(b.finish(12), BAREOS_LAST_BLOCK);
    }

    #[test]
    fn a_record_continued_on_the_next_volume_is_reassembled() {
        // one 100-byte record over four 64-byte blocks (28 data bytes
        // each after the headers); the first block ends "volume 1", the
        // others are "volume 2"
        let data: Vec<u8> = (0..100).collect();
        let r = RecordRef { vol_session_id: 4, vol_session_time: 5, file_index: 7, stream: 2, data: &data };
        let mut b = BlockBuilder::new(64);
        let mut p = RecordProgress::default();
        let mut blocks = Vec::new();
        let mut n = 0;
        while !b.write_record(&r, &mut p) {
            blocks.push(b.finish(n));
            n += 1;
        }
        blocks.push(b.finish(n));
        assert_eq!(blocks.len(), 4);
        let v1 = read_volume(&blocks[0]).unwrap();
        assert_eq!((v1.records.len(), v1.truncated), (1, Some(72)), "the first volume ends inside the record");
        let mut a = RecordAssembler::new();
        assert!(a.push_block(&blocks[0], 0, true).unwrap().1.is_empty());
        assert!(a.push_block(&blocks[1], 0, true).unwrap().1.is_empty());
        assert!(a.push_block(&blocks[2], 64, true).unwrap().1.is_empty());
        let (_, done) = a.push_block(&blocks[3], 128, true).unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!((done[0].file_index, done[0].stream, &done[0].data), (7, 2, &data));
        assert_eq!(done[0].block_number, 0, "the record starts in the first block");
        assert!(a.take_pending().is_empty());
    }

    #[test]
    fn interleaved_sessions_keep_their_own_partial_records() {
        let one = RecordRef { vol_session_id: 1, vol_session_time: 9, file_index: 1, stream: 2, data: &[1u8; 50] };
        let two = RecordRef { vol_session_id: 2, vol_session_time: 9, file_index: 1, stream: 2, data: &[2u8; 50] };
        let mut blocks = Vec::new();
        for (r, size) in [(&one, 60), (&two, 60), (&one, 100), (&two, 100)] {
            let mut b = BlockBuilder::new(size);
            let mut p = if size == 60 { RecordProgress::default() } else { RecordProgress { state: RecordState::HeaderCont, remainder: 26 } };
            let _ = b.write_record(r, &mut p);
            blocks.push(b.finish(blocks.len() as u32));
        }
        let mut a = RecordAssembler::new();
        let mut out = Vec::new();
        for blk in &blocks {
            out.extend(a.push_block(blk, 0, true).unwrap().1);
        }
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].vol_session_id, out[0].data.as_slice()), (1, &[1u8; 50][..]));
        assert_eq!((out[1].vol_session_id, out[1].data.as_slice()), (2, &[2u8; 50][..]));
    }

}
