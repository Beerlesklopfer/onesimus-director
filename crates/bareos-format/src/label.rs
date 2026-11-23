// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Labels im Volume: Volume-Label und Sitzungslabels (SOS, EOS).
//!
//! Zweck: Ein Volume beginnt mit einem Volume-Label (Name, Pool,
//! MediaType, wer es schrieb). Jeder Job, der auf das Volume schreibt,
//! rahmt seine Datensätze mit einem Start-of-Session- (SOS) und einem
//! End-of-Session-Label (EOS). Labels sind gewöhnliche Datensätze
//! ([`crate::volume::RecordRef`]) mit negativem FileIndex. Dieses Modul
//! serialisiert und liest ihre Nutzdaten byte-genau wie bareos-sd 25.
//!
//! Kodierung (`lib/serial.cc`): Zahlen Big Endian, Zeichenketten mit
//! abschließendem NUL, `btime` als i64 Mikrosekunden seit 1970, `float64`
//! als IEEE 754 Big Endian (hier immer 0).
//!
//! | Label | Felder in dieser Reihenfolge |
//! |---|---|
//! | Volume (`CreateVolumeLabelRecord`) | `Id`, `VerNum` u32, `label_btime`, `write_btime`, `write_date` f64, `write_time` f64, `VolumeName`, `PrevVolumeName`, `PoolName`, `PoolType`, `MediaType`, `HostName`, `LabelProg`, `ProgVersion`, `ProgDate` |
//! | SOS (`CreateSessionLabel`) | `Id`, `VerNum` u32, `JobId` u32, `write_btime`, `write_time` f64, `PoolName`, `PoolType`, `JobName`, `ClientName`, `Job`, `FileSetName`, `JobType` u32, `JobLevel` u32, `FileSetMD5` |
//! | EOS | wie SOS, dazu `JobFiles` u32, `JobBytes` u64, `StartBlock`, `EndBlock`, `StartFile`, `EndFile`, `JobErrors`, `JobStatus` (je u32) |
//!
//! Datensatzkopf der Labels: Volume-Label `FileIndex = VOL_LABEL` (−2),
//! `Stream` = Zahl der vom Job geschriebenen Volumes (`NumWriteVolumes`),
//! Sitzung des Jobs; SOS/EOS `FileIndex = SOS_LABEL`/`EOS_LABEL`,
//! `Stream = JobId`.
//!
//! Bareos-Gegenstück: `core/src/stored/label.cc` (`CreateVolumeLabel`,
//! `CreateVolumeLabelRecord`, `CreateSessionLabel`, `WriteSessionLabel`,
//! `UnserVolumeLabel`, `UnserSessionLabel`), `stored/record.h`
//! (`Volume_Label`, `Session_Label`, Label-Arten), `stored/block.h`
//! (`BareosId`, `BareosTapeVersion`), Bareos `18be77e6`.

/// `PRE_LABEL`: volume label on an unwritten tape.
pub const PRE_LABEL: i32 = -1;
/// `VOL_LABEL`: the volume label.
pub const VOL_LABEL: i32 = -2;
/// `EOM_LABEL`: end of medium.
pub const EOM_LABEL: i32 = -3;
/// `SOS_LABEL`: start of session.
pub const SOS_LABEL: i32 = -4;
/// `EOS_LABEL`: end of session.
pub const EOS_LABEL: i32 = -5;
/// `EOT_LABEL`: end of tape.
pub const EOT_LABEL: i32 = -6;

/// `BareosId`: the label id string.
pub const BAREOS_ID: &str = "Bareos 2.0 immortal\n";
/// `BareosTapeVersion`.
pub const BAREOS_TAPE_VERSION: u32 = 20;
/// `SER_LENGTH_Volume_Label` / `SER_LENGTH_Session_Label`: upper bound.
pub const SER_LENGTH_LABEL: usize = 1024;
/// `MAX_NAME_LENGTH`: names are cut to 127 bytes (`bstrncpy` into 128).
pub const MAX_NAME_LENGTH: usize = 128;

/// Serializer of `lib/serial.h`.
#[derive(Default)]
struct Ser(Vec<u8>);

impl Ser {
    /// `ser_uint32`.
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    /// `ser_uint64`.
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    /// `SerBtime`.
    fn btime(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_be_bytes());
    }
    /// `ser_float64` of 0.0 (all labels written since VerNum 11).
    fn zero_f64(&mut self) {
        self.0.extend_from_slice(&0f64.to_be_bytes());
    }
    /// `SerString`: the bytes up to the first NUL, then NUL.
    fn string(&mut self, s: &str) {
        let b = s.as_bytes();
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        self.0.extend_from_slice(&b[..end]);
        self.0.push(0);
    }
}

/// Deserializer of `lib/serial.h`.
struct Unser<'a> {
    /// The data.
    buf: &'a [u8],
    /// Read position.
    at: usize,
}

impl<'a> Unser<'a> {
    /// `n` bytes or an error.
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.buf.get(self.at..self.at + n).ok_or_else(|| format!("label too short at byte {}", self.at))?;
        self.at += n;
        Ok(s)
    }
    /// `unser_uint32`.
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap_or_default()))
    }
    /// `unser_uint64`.
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap_or_default()))
    }
    /// `UnserBtime` (also used for the bytes of a `float64`).
    fn i64(&mut self) -> Result<i64, String> {
        Ok(self.u64()? as i64)
    }
    /// `UnserString`: up to NUL.
    fn string(&mut self) -> Result<String, String> {
        let rest = &self.buf[self.at.min(self.buf.len())..];
        let end = rest.iter().position(|&c| c == 0).ok_or("unterminated string in label")?;
        self.at += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
}

/// `bstrncpy(dst, src, MAX_NAME_LENGTH)`: at most 127 bytes.
fn name(s: &str) -> String {
    let mut end = s.len().min(MAX_NAME_LENGTH - 1);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// The volume label (`Volume_Label`, the fields written since VerNum 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLabel {
    /// `Id`: [`BAREOS_ID`].
    pub id: String,
    /// `VerNum`: [`BAREOS_TAPE_VERSION`].
    pub ver_num: u32,
    /// `label_btime`: when the volume was labeled (µs since 1970).
    pub label_btime: i64,
    /// `write_btime`: when this label was written (µs since 1970).
    pub write_btime: i64,
    /// `VolumeName`.
    pub volume_name: String,
    /// `PrevVolumeName` (empty).
    pub prev_volume_name: String,
    /// `PoolName`.
    pub pool_name: String,
    /// `PoolType` (`Backup`).
    pub pool_type: String,
    /// `MediaType` of the device.
    pub media_type: String,
    /// `HostName` of the writing computer (`gethostname`).
    pub host_name: String,
    /// `LabelProg`: the daemon name (`bareos-sd`; osd writes `osd`).
    pub label_prog: String,
    /// `ProgVersion`: `Ver. <version> <date>`.
    pub prog_version: String,
    /// `ProgDate`: `Build <date time>`.
    pub prog_date: String,
}

impl VolumeLabel {
    /// `CreateVolumeLabel`: a new label for `volume_name` in `pool_name`
    /// with the device's `media_type`, labeled and written at `btime`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(volume_name: &str, pool_name: &str, media_type: &str, host_name: &str, label_prog: &str, prog_version: &str, prog_date: &str, btime: i64) -> Self {
        VolumeLabel {
            id: BAREOS_ID.into(),
            ver_num: BAREOS_TAPE_VERSION,
            label_btime: btime,
            write_btime: btime,
            volume_name: name(volume_name),
            prev_volume_name: String::new(),
            pool_name: name(pool_name),
            pool_type: "Backup".into(),
            media_type: name(media_type),
            host_name: name(host_name),
            label_prog: label_prog.chars().take(49).collect(),
            prog_version: prog_version.chars().take(49).collect(),
            prog_date: prog_date.chars().take(49).collect(),
        }
    }

    /// `CreateVolumeLabelRecord`: the record data.
    pub fn serialize(&self) -> Vec<u8> {
        let mut s = Ser::default();
        s.string(&self.id);
        s.u32(self.ver_num);
        s.btime(self.label_btime);
        s.btime(self.write_btime);
        s.zero_f64();
        s.zero_f64();
        for v in [&self.volume_name, &self.prev_volume_name, &self.pool_name, &self.pool_type, &self.media_type,
            &self.host_name, &self.label_prog, &self.prog_version, &self.prog_date] {
            s.string(v);
        }
        s.0
    }

    /// `UnserVolumeLabel` for VerNum ≥ 11; older labels are refused.
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        let mut u = Unser { buf: data, at: 0 };
        let id = u.string()?;
        let ver_num = u.u32()?;
        if ver_num < 11 {
            return Err(format!("volume label version {ver_num} (< 11) not supported"));
        }
        let label_btime = u.i64()?;
        let write_btime = u.i64()?;
        u.i64()?;
        u.i64()?;
        Ok(VolumeLabel {
            id,
            ver_num,
            label_btime,
            write_btime,
            volume_name: u.string()?,
            prev_volume_name: u.string()?,
            pool_name: u.string()?,
            pool_type: u.string()?,
            media_type: u.string()?,
            host_name: u.string()?,
            label_prog: u.string()?,
            prog_version: u.string()?,
            prog_date: u.string()?,
        })
    }
}

/// A session label (`Session_Label`); the EOS fields are `Some` only in an
/// EOS label.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionLabel {
    /// `Id`.
    pub id: String,
    /// `VerNum`.
    pub ver_num: u32,
    /// `JobId`.
    pub job_id: u32,
    /// `write_btime` (µs since 1970).
    pub write_btime: i64,
    /// `PoolName`.
    pub pool_name: String,
    /// `PoolType`.
    pub pool_type: String,
    /// `JobName`: name of the Job resource.
    pub job_name: String,
    /// `ClientName`.
    pub client_name: String,
    /// `Job`: the unique job name.
    pub job: String,
    /// `FileSetName`.
    pub fileset_name: String,
    /// `JobType` (letter as number, `'B'` = 66).
    pub job_type: u32,
    /// `JobLevel` (letter as number, `'F'` = 70).
    pub job_level: u32,
    /// `FileSetMD5`.
    pub fileset_md5: String,
    /// EOS: `JobFiles`.
    pub job_files: Option<u32>,
    /// EOS: `JobBytes`.
    pub job_bytes: Option<u64>,
    /// EOS: `StartBlock`.
    pub start_block: Option<u32>,
    /// EOS: `EndBlock`.
    pub end_block: Option<u32>,
    /// EOS: `StartFile`.
    pub start_file: Option<u32>,
    /// EOS: `EndFile`.
    pub end_file: Option<u32>,
    /// EOS: `JobErrors`.
    pub job_errors: Option<u32>,
    /// EOS: `JobStatus` (letter as number, `'T'` = 84).
    pub job_status: Option<u32>,
}

impl SessionLabel {
    /// `CreateSessionLabel`: the record data; the EOS fields are written
    /// when `job_files` is `Some` (missing ones as 0).
    pub fn serialize(&self) -> Vec<u8> {
        let mut s = Ser::default();
        s.string(&self.id);
        s.u32(self.ver_num);
        s.u32(self.job_id);
        s.btime(self.write_btime);
        s.zero_f64();
        for v in [&self.pool_name, &self.pool_type, &self.job_name, &self.client_name, &self.job, &self.fileset_name] {
            s.string(v);
        }
        s.u32(self.job_type);
        s.u32(self.job_level);
        s.string(&self.fileset_md5);
        if let Some(files) = self.job_files {
            s.u32(files);
            s.u64(self.job_bytes.unwrap_or(0));
            for v in [self.start_block, self.end_block, self.start_file, self.end_file, self.job_errors, self.job_status] {
                s.u32(v.unwrap_or(0));
            }
        }
        s.0
    }

    /// `UnserSessionLabel` for VerNum ≥ 11; `eos` reads the EOS fields.
    pub fn parse(data: &[u8], eos: bool) -> Result<Self, String> {
        let mut u = Unser { buf: data, at: 0 };
        let id = u.string()?;
        let ver_num = u.u32()?;
        if ver_num < 11 {
            return Err(format!("session label version {ver_num} (< 11) not supported"));
        }
        let job_id = u.u32()?;
        let write_btime = u.i64()?;
        u.i64()?;
        let mut l = SessionLabel {
            id,
            ver_num,
            job_id,
            write_btime,
            pool_name: u.string()?,
            pool_type: u.string()?,
            job_name: u.string()?,
            client_name: u.string()?,
            job: u.string()?,
            fileset_name: u.string()?,
            job_type: u.u32()?,
            job_level: u.u32()?,
            fileset_md5: u.string()?,
            ..Default::default()
        };
        if eos {
            l.job_files = Some(u.u32()?);
            l.job_bytes = Some(u.u64()?);
            l.start_block = Some(u.u32()?);
            l.end_block = Some(u.u32()?);
            l.start_file = Some(u.u32()?);
            l.end_file = Some(u.u32()?);
            l.job_errors = Some(u.u32()?);
            l.job_status = Some(u.u32()?);
        }
        Ok(l)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_label_bytes_follow_create_volume_label_record() {
        let l = VolumeLabel::new("Vol-1", "Pool", "File", "host", "osd", "Ver. 25", "Build x", 0x0102_0304_0506_0708);
        let b = l.serialize();
        let mut expected = b"Bareos 2.0 immortal\n\0".to_vec();
        expected.extend_from_slice(&[0, 0, 0, 20]);
        expected.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8]);
        expected.extend_from_slice(&[0; 16]);
        expected.extend_from_slice(b"Vol-1\0\0Pool\0Backup\0File\0host\0osd\0Ver. 25\0Build x\0");
        assert_eq!(b, expected);
        assert_eq!(VolumeLabel::parse(&b).unwrap(), l);
    }

    #[test]
    fn session_labels_roundtrip_and_eos_is_longer() {
        let mut l = SessionLabel {
            id: BAREOS_ID.into(),
            ver_num: BAREOS_TAPE_VERSION,
            job_id: 3,
            write_btime: 42,
            pool_name: "P".into(),
            pool_type: "Backup".into(),
            job_name: "J".into(),
            client_name: "C".into(),
            job: "J.2026".into(),
            fileset_name: "F".into(),
            job_type: 'B' as u32,
            job_level: 'F' as u32,
            fileset_md5: "m".into(),
            ..Default::default()
        };
        let sos = l.serialize();
        // id 21 + ver 4 + jobid 4 + btime 8 + f64 8 + "P\0Backup\0J\0C\0J.2026\0F\0" 22 + 8 + "m\0" 2
        assert_eq!(sos.len(), 21 + 4 + 4 + 8 + 8 + 22 + 8 + 2);
        assert_eq!(&sos[25..29], &[0, 0, 0, 3]);
        assert_eq!(SessionLabel::parse(&sos, false).unwrap(), l);
        l.job_files = Some(13);
        l.job_bytes = Some(1 << 40);
        l.start_block = Some(1);
        l.end_block = Some(9);
        l.start_file = Some(0);
        l.end_file = Some(0);
        l.job_errors = Some(0);
        l.job_status = Some('T' as u32);
        let eos = l.serialize();
        assert_eq!(eos.len(), sos.len() + 4 + 8 + 6 * 4);
        assert_eq!(SessionLabel::parse(&eos, true).unwrap(), l);
    }

    #[test]
    fn names_are_cut_like_bstrncpy() {
        let long = "x".repeat(200);
        assert_eq!(VolumeLabel::new(&long, "p", "m", "h", "osd", "v", "d", 0).volume_name.len(), 127);
    }
}
