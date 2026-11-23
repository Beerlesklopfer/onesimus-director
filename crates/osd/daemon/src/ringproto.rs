// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Was Netzsitzung und Geräteschreiber über den Ring austauschen.
//!
//! Zweck: Der Ring (`osd-ring`) transportiert nur Bytes. Hier steht, was
//! sie bedeuten: die Job-Angaben im Info-Bereich ([`JobInfo`]), die Arten
//! der Fächer ([`SlotKind`], Metawörter) und die Anfragen im Postfach
//! ([`Request`]). Es gibt keinen anderen Weg zwischen den beiden; deshalb
//! kann der Schreiber ein eigener Prozess sein (D-13).
//!
//! | Fach | Nutzdaten | Metawörter |
//! |---|---|---|
//! | [`SlotKind::Data`] | Datensätze eines vollen Blocks (ohne Blockkopf) | Art, FirstIndex, LastIndex, VolSessionId, VolSessionTime |
//! | [`SlotKind::End`] | der angefangene letzte Block | dazu JobFiles, JobBytes, JobErrors und JobStatus für das EOS-Label |
//! | [`SlotKind::Block`] (Lesen) | ein ganzer Block, wie er auf dem Volume steht (Kopf, Datensätze) | Art, Nummer des Volumes in der Restore-Liste, Adresse des Blocks ([`BlockMeta`]) |
//!
//! Beim Schreiben füllt die Netzsitzung die Fächer und der Geräteschreiber
//! stellt die Anfragen; beim Lesen füllt der Geräteleser die Fächer und
//! stellt die Anfragen (`Ring::ask_consumer`), die Netzsitzung beantwortet
//! sie.
//!
//! | Anfrage | Antwort |
//! |---|---|
//! | `C<CatReq-Zeile>` | Antwortzeile des Directors, oder `E<Fehler>` |
//! | `M<Typ> <Text>` | `ok` (Job-Meldung an den Director) |
//! | `R` | `ok` (Device belegt, Volume bereit) |
//!
//! Bareos-Gegenstück: keins (bareos-sd schreibt im Job-Thread selbst).

/// Name of the ring of job `job_id` on the SD listening on `port`. The port
/// keeps rings of several osd instances on one host apart.
pub fn ring_name(port: u16, job_id: u32) -> String {
    format!("{}{job_id}.ring", ring_prefix(port))
}

/// Prefix of the rings of the osd on `port` (stale-ring cleanup).
pub fn ring_prefix(port: u16) -> String {
    format!("osd-{port}-")
}

/// Kind of a slot (meta word 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// A full block's records.
    Data = 1,
    /// The last, partial block and the end-of-session counters.
    End = 2,
    /// Read direction: a whole block from a volume.
    Block = 3,
}

impl SlotKind {
    /// From meta word 0.
    pub fn from_word(v: u64) -> Option<SlotKind> {
        match v {
            1 => Some(SlotKind::Data),
            2 => Some(SlotKind::End),
            3 => Some(SlotKind::Block),
            _ => None,
        }
    }
}

/// The meta words of a slot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlotMeta {
    /// [`SlotKind`] as word.
    pub kind: u64,
    /// `FirstIndex` of the block.
    pub first_index: i32,
    /// `LastIndex` of the block.
    pub last_index: i32,
    /// `VolSessionId` of the last record header.
    pub vol_session_id: u32,
    /// `VolSessionTime` of the last record header.
    pub vol_session_time: u32,
    /// End slot: `JobFiles`.
    pub job_files: u32,
    /// End slot: `JobErrors`.
    pub job_errors: u32,
    /// End slot: `JobStatus` letter.
    pub job_status: u8,
    /// End slot: `JobBytes`.
    pub job_bytes: u64,
}

impl SlotMeta {
    /// Packs into the ring's seven meta words.
    pub fn to_words(self) -> [u64; osd_ring::SLOT_META] {
        [
            self.kind,
            self.first_index as u32 as u64,
            self.last_index as u32 as u64,
            self.vol_session_id as u64,
            self.vol_session_time as u64,
            // JobFiles, JobStatus and the low 24 bits of JobErrors share word 5
            self.job_files as u64 | (self.job_status as u64) << 32 | (self.job_errors as u64 & 0xff_ffff) << 40,
            self.job_bytes,
        ]
    }

    /// Unpacks the seven meta words.
    pub fn from_words(w: [u64; osd_ring::SLOT_META]) -> SlotMeta {
        SlotMeta {
            kind: w[0],
            first_index: w[1] as u32 as i32,
            last_index: w[2] as u32 as i32,
            vol_session_id: w[3] as u32,
            vol_session_time: w[4] as u32,
            job_files: w[5] as u32,
            job_status: (w[5] >> 32) as u8,
            job_errors: (w[5] >> 40) as u32,
            job_bytes: w[6],
        }
    }
}

/// The meta words of a [`SlotKind::Block`] slot (read direction).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockMeta {
    /// Index of the volume in the restore volume list
    /// (`bareos_format::bsr::Bsr::restore_volumes`).
    pub volume: u32,
    /// Byte address of the block on its volume.
    pub address: u64,
}

impl BlockMeta {
    /// Packs into the ring's meta words.
    pub fn to_words(self) -> [u64; osd_ring::SLOT_META] {
        [SlotKind::Block as u64, self.volume as u64, self.address, 0, 0, 0, 0]
    }

    /// Unpacks the meta words; `None` for another slot kind.
    pub fn from_words(w: [u64; osd_ring::SLOT_META]) -> Option<BlockMeta> {
        (SlotKind::from_word(w[0]) == Some(SlotKind::Block)).then_some(BlockMeta { volume: w[1] as u32, address: w[2] })
    }
}

/// A request of the device writer or reader to the network session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// A catalog request line for the Director; the answer is its reply.
    CatReq(String),
    /// A job message (`M_*` type, text).
    Jmsg(i32, String),
    /// The device is acquired and the volume ready.
    Ready,
    /// No appendable volume: report JS_WaitMedia and wait at most this many
    /// seconds for a `label`/`mount` on the device or a cancel
    /// (`WaitForSysop`). Answer `w` (woken), `t` (timeout), `c` (canceled).
    WaitMedia(u32),
    /// `sendJobStatus(status)` (back to `R` after waiting for media).
    JobStatus(char),
}

impl Request {
    /// Bytes for the mailbox.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Request::CatReq(line) => format!("C{line}").into_bytes(),
            Request::Jmsg(t, text) => format!("M{t} {text}").into_bytes(),
            Request::Ready => b"R".to_vec(),
            Request::WaitMedia(secs) => format!("W{secs}").into_bytes(),
            Request::JobStatus(c) => format!("S{c}").into_bytes(),
        }
    }

    /// From mailbox bytes.
    pub fn decode(bytes: &[u8]) -> Option<Request> {
        let text = String::from_utf8_lossy(bytes);
        let (tag, rest) = text.split_at_checked(1)?;
        match tag {
            "C" => Some(Request::CatReq(rest.to_string())),
            "M" => {
                let (t, msg) = rest.split_once(' ')?;
                Some(Request::Jmsg(t.parse().ok()?, msg.to_string()))
            }
            "R" => Some(Request::Ready),
            "W" => Some(Request::WaitMedia(rest.parse().ok()?)),
            "S" => Some(Request::JobStatus(rest.chars().next()?)),
            _ => None,
        }
    }
}

/// The job as the device writer needs it, in the ring's info area.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobInfo {
    /// Unique job name (`job=`).
    pub job: String,
    /// JobId.
    pub job_id: u32,
    /// `VolSessionId`.
    pub vol_session_id: u32,
    /// `VolSessionTime`.
    pub vol_session_time: u32,
    /// Pool of `use storage`.
    pub pool_name: String,
    /// Pool type of `use storage`.
    pub pool_type: String,
    /// Media type of `use storage`.
    pub media_type: String,
    /// Name of the Storage resource (daemon name in messages and labels).
    pub storage: String,
    /// Device name.
    pub device: String,
    /// `Archive Device`.
    pub archive_device: String,
    /// `Label Media`.
    pub label_media: bool,
    /// `Maximum Block Size`.
    pub max_block_size: u32,
    /// `Label Block Size`.
    pub label_block_size: u32,
    /// `Maximum Volume Size` (0 = none).
    pub max_volume_size: u64,
    /// `job_name=`: name of the Job resource.
    pub job_name: String,
    /// `client_name=`.
    pub client_name: String,
    /// `FileSet=`.
    pub fileset_name: String,
    /// Read direction: the bootstrap file of the job (empty when writing).
    pub bootstrap: String,
    /// `FileSetMD5=`.
    pub fileset_md5: String,
    /// `type=` as number (`'B'` = 66).
    pub job_type: u32,
    /// `level=` as number (`'F'` = 70).
    pub job_level: u32,
    /// `Volume Poll Interval` of the device in seconds (Bareos default 300;
    /// 0 = no polling, only `label`/`mount` wake a waiting job).
    pub vol_poll_interval: u64,
    /// Read direction through a Distributor: the other target devices the
    /// job holds; the reader switches to the one whose media type matches
    /// the next volume (empty otherwise).
    pub alternatives: Vec<crate::device::FileDevice>,
}

impl JobInfo {
    /// `key=value` lines (values without newlines).
    pub fn encode(&self) -> String {
        let alternatives: String = self
            .alternatives
            .iter()
            .map(|a| {
                format!(
                    "alt={}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    a.name,
                    a.archive_device.display(),
                    a.media_type,
                    a.label_media as u8,
                    a.max_block_size,
                    a.label_block_size,
                    a.max_volume_size
                )
            })
            .collect();
        format!("{}{alternatives}", self.encode_fields())
    }

    /// The `key=value` lines of the fields except [`JobInfo::alternatives`].
    fn encode_fields(&self) -> String {
        format!(
            "job={}\njob_id={}\nvol_session_id={}\nvol_session_time={}\npool_name={}\npool_type={}\nmedia_type={}\n\
             storage={}\ndevice={}\narchive_device={}\nlabel_media={}\nmax_block_size={}\nlabel_block_size={}\n\
             max_volume_size={}\njob_name={}\nclient_name={}\nfileset_name={}\nfileset_md5={}\njob_type={}\n\
             job_level={}\nbootstrap={}\nvol_poll_interval={}\n",
            self.job,
            self.job_id,
            self.vol_session_id,
            self.vol_session_time,
            self.pool_name,
            self.pool_type,
            self.media_type,
            self.storage,
            self.device,
            self.archive_device,
            self.label_media as u8,
            self.max_block_size,
            self.label_block_size,
            self.max_volume_size,
            self.job_name,
            self.client_name,
            self.fileset_name,
            self.fileset_md5,
            self.job_type,
            self.job_level,
            self.bootstrap,
            self.vol_poll_interval
        )
    }

    /// From the info area.
    pub fn decode(text: &str) -> Result<JobInfo, String> {
        let get = |key: &str| -> Result<String, String> {
            text.lines()
                .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
                .map(str::to_string)
                .ok_or_else(|| format!("ring info: {key} missing"))
        };
        let num = |key: &str| -> Result<u64, String> { get(key)?.parse().map_err(|_| format!("ring info: {key} not a number")) };
        Ok(JobInfo {
            job: get("job")?,
            job_id: num("job_id")? as u32,
            vol_session_id: num("vol_session_id")? as u32,
            vol_session_time: num("vol_session_time")? as u32,
            pool_name: get("pool_name")?,
            pool_type: get("pool_type")?,
            media_type: get("media_type")?,
            storage: get("storage")?,
            device: get("device")?,
            archive_device: get("archive_device")?,
            label_media: num("label_media")? != 0,
            max_block_size: num("max_block_size")? as u32,
            label_block_size: num("label_block_size")? as u32,
            max_volume_size: num("max_volume_size")?,
            job_name: get("job_name")?,
            client_name: get("client_name")?,
            fileset_name: get("fileset_name")?,
            fileset_md5: get("fileset_md5")?,
            job_type: num("job_type")? as u32,
            job_level: num("job_level")? as u32,
            bootstrap: get("bootstrap")?,
            // older rings without the line: the Bareos default
            vol_poll_interval: num("vol_poll_interval").unwrap_or(300),
            alternatives: text.lines().filter_map(|l| l.strip_prefix("alt=")).map(Self::decode_alternative).collect::<Result<_, _>>()?,
        })
    }

    /// One `alt=` line (without the key) as a device.
    fn decode_alternative(line: &str) -> Result<crate::device::FileDevice, String> {
        let f: Vec<&str> = line.split('\t').collect();
        let bad = || format!("ring info: bad alt= line {line:?}");
        let [name, archive, media, label, max_block, label_block, max_volume] = f[..] else { return Err(bad()) };
        Ok(crate::device::FileDevice {
            name: name.to_string(),
            archive_device: archive.into(),
            media_type: media.to_string(),
            label_media: label == "1",
            max_block_size: max_block.parse().map_err(|_| bad())?,
            label_block_size: label_block.parse().map_err(|_| bad())?,
            max_volume_size: max_volume.parse().map_err(|_| bad())?,
        })
    }

    /// `CreateSessionLabel`: the SOS label of this job written at
    /// `write_btime` (µs since 1970); EOS adds the end fields.
    pub fn session_label(&self, write_btime: i64) -> bareos_format::label::SessionLabel {
        bareos_format::label::SessionLabel {
            id: bareos_format::label::BAREOS_ID.into(),
            ver_num: bareos_format::label::BAREOS_TAPE_VERSION,
            job_id: self.job_id,
            write_btime,
            pool_name: self.pool_name.clone(),
            pool_type: self.pool_type.clone(),
            job_name: self.job_name.clone(),
            client_name: self.client_name.clone(),
            job: self.job.clone(),
            fileset_name: self.fileset_name.clone(),
            job_type: self.job_type,
            job_level: self.job_level,
            fileset_md5: self.fileset_md5.clone(),
            ..Default::default()
        }
    }

    /// The File device described by this info.
    pub fn device(&self) -> crate::device::FileDevice {
        crate::device::FileDevice {
            name: self.device.clone(),
            archive_device: self.archive_device.clone().into(),
            media_type: self.media_type.clone(),
            label_media: self.label_media,
            max_block_size: self.max_block_size,
            label_block_size: self.label_block_size,
            max_volume_size: self.max_volume_size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_words_roundtrip() {
        let m = SlotMeta {
            kind: SlotKind::End as u64,
            first_index: 1,
            last_index: -5,
            vol_session_id: 3,
            vol_session_time: 1_795_204_762,
            job_files: 13,
            job_errors: 2,
            job_status: b'T',
            job_bytes: 12_590_471,
        };
        assert_eq!(SlotMeta::from_words(m.to_words()), m);
    }

    #[test]
    fn requests_and_info_roundtrip() {
        for r in [Request::CatReq("CatReq Job=x GetVolInfo VolName=V write=1\n".into()), Request::Jmsg(6, "a b\n".into()), Request::Ready] {
            assert_eq!(Request::decode(&r.encode()), Some(r));
        }
        assert_eq!(Request::decode(b""), None);
        let info = JobInfo { job: "J.1".into(), job_id: 7, pool_name: "P".into(), label_media: true, max_block_size: 1 << 20, ..Default::default() };
        assert_eq!(JobInfo::decode(&info.encode()).unwrap(), info);
        let read = JobInfo { bootstrap: "/var/lib/bareos/vsd.J.1.bootstrap".into(), ..info };
        assert_eq!(JobInfo::decode(&read.encode()).unwrap(), read);
        let alt = crate::device::FileDevice {
            name: "B".into(),
            archive_device: "/srv/b".into(),
            media_type: "VFile2".into(),
            label_media: true,
            max_block_size: 1 << 20,
            label_block_size: 64512,
            max_volume_size: 5,
        };
        let routed = JobInfo { alternatives: vec![alt.clone(), crate::device::FileDevice { name: "C".into(), ..alt }], ..read };
        assert_eq!(JobInfo::decode(&routed.encode()).unwrap(), routed);
        let b = BlockMeta { volume: 2, address: 5_000_230 };
        assert_eq!(BlockMeta::from_words(b.to_words()), Some(b));
        assert_eq!(BlockMeta::from_words(SlotMeta::default().to_words()), None);
        assert_eq!(ring_name(39473, 7), "osd-39473-7.ring");
    }
}
