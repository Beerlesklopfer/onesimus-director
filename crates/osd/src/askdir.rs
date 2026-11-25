// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Nachrichten des SD an den Director während eines Jobs.
//!
//! Zweck: Der SD fragt den Director nach Volumes und meldet, was er
//! geschrieben hat (Katalog-Anfragen, `CatReq`), schickt die Attribute
//! jeder Datei (`UpdCat`), Job-Meldungen (`Jmsg`) und den Job-Status
//! (`Status`). Dieses Modul formatiert die Zeilen genau wie bareos-sd und
//! liest die Antwort `1000 OK VolName=…`. Gegenseite im Director ist
//! `odir/src/catreq.rs` (`dird/catreq.cc`).
//!
//! | Nachricht | Antwort des Directors |
//! |---|---|
//! | [`find_media`] | [`VolumeInfo`] oder Fehlerzeile |
//! | [`get_vol_info`] | [`VolumeInfo`] oder Fehlerzeile |
//! | [`UpdateMedia::format`] | [`VolumeInfo`] |
//! | [`CreateJobMedia::format`] | `1000 OK CreateJobMedia` |
//! | [`delete_null_jobmedia`] | `1000 OK …` |
//! | [`file_attributes`] | keine |
//! | [`jmsg`], [`job_status`] | keine |
//!
//! Bareos-Gegenstück: `core/src/stored/askdir.cc` (`Find_media`,
//! `Get_Vol_Info`, `Update_media`, `Create_job_media`, `FileAttributes`,
//! `OK_media`, `DoGetVolumeInfo`, `DeleteNullJobmediaRecords`),
//! `lib/message.cc` (`JobMessage`), `lib/jcr.cc` (`Job_status`).

use bareos_format::bash::{bash_spaces, unbash_spaces};

/// `Find_media`: the `index`-th appendable volume of `pool`/`media_type`,
/// not one of `unwanted` (comma separated).
pub fn find_media(job: &str, index: u32, pool: &str, media_type: &str, unwanted: &str) -> String {
    format!(
        "CatReq Job={job} FindMedia={index} pool_name={} media_type={} unwanted_volumes={}\n",
        bash_spaces(pool),
        bash_spaces(media_type),
        bash_spaces(unwanted)
    )
}

/// `Get_Vol_Info`: the catalog record of `volume` (`write`: for appending).
pub fn get_vol_info(job: &str, volume: &str, write: bool) -> String {
    format!("CatReq Job={job} GetVolInfo VolName={} write={}\n", bash_spaces(volume), write as i32)
}

/// `CatReq Job=%s DeleteNullJobmediaRecords jobid=%u`.
pub fn delete_null_jobmedia(job: &str, job_id: u32) -> String {
    format!("CatReq Job={job} DeleteNullJobmediaRecords jobid={job_id}")
}

/// `JobMessage`: a job message of `daemon` (`<daemon> JobId <n>: <text>`),
/// with message type `msg_type` (`M_*`) and time `mtime`.
pub fn jmsg(job: &str, daemon: &str, job_id: u32, msg_type: i32, mtime: i64, text: &str) -> String {
    // lib/message.cc Jmsg(): the text of an error, a warning … starts with its kind
    use bareos_format::msg::{M_ERROR, M_FATAL, M_SECURITY, M_WARNING};
    let kind = match msg_type {
        M_FATAL => "Fatal error: ",
        M_ERROR => "Error: ",
        M_WARNING => "Warning: ",
        M_SECURITY => "Security violation: ",
        _ => "",
    };
    format!("Jmsg Job={job} type={msg_type} level={mtime} {daemon} JobId {job_id}: {kind}{text}")
}

/// `Job_status`: `Status Job=%s JobStatus=%d`.
pub fn job_status(job: &str, status: char) -> String {
    format!("Status Job={job} JobStatus={}\n", status as u32)
}

/// `FileAttributes`: the binary `UpdCat` message for one attribute record.
pub fn file_attributes(job: &str, vol_session_id: u32, vol_session_time: u32, file_index: i32, stream: i32, data: &[u8]) -> Vec<u8> {
    let mut m = format!("UpdCat Job={job} FileAttributes ").into_bytes();
    m.extend_from_slice(&vol_session_id.to_be_bytes());
    m.extend_from_slice(&vol_session_time.to_be_bytes());
    m.extend_from_slice(&file_index.to_be_bytes());
    m.extend_from_slice(&stream.to_be_bytes());
    m.extend_from_slice(&(data.len() as u32).to_be_bytes());
    m.extend_from_slice(data);
    m
}

/// The volume counters the SD reports with `UpdateMedia`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateMedia {
    /// `VolName`.
    pub volume: String,
    /// `VolJobs`.
    pub jobs: u32,
    /// `VolFiles`.
    pub files: u32,
    /// `VolBlocks`.
    pub blocks: u32,
    /// `VolBytes`.
    pub bytes: u64,
    /// `VolMounts`.
    pub mounts: u32,
    /// `VolErrors`.
    pub errors: u32,
    /// `VolWrites`.
    pub writes: u32,
    /// `MaxVolBytes`.
    pub max_bytes: u64,
    /// `EndTime`: last written (seconds since 1970).
    pub end_time: u64,
    /// `VolStatus` (`Append`, `Full` …).
    pub status: String,
    /// `Slot`.
    pub slot: i32,
    /// `relabel`: 1 right after labeling.
    pub relabel: bool,
    /// `InChanger`.
    pub in_changer: bool,
    /// `VolReadTime` (µs).
    pub read_time: i64,
    /// `VolWriteTime` (µs).
    pub write_time: i64,
    /// `VolFirstWritten`.
    pub first_written: u64,
}

impl UpdateMedia {
    /// `Update_media` of `askdir.cc` for `job`.
    pub fn format(&self, job: &str) -> String {
        format!(
            "CatReq Job={job} UpdateMedia VolName={} VolJobs={} VolFiles={} VolBlocks={} VolBytes={} VolMounts={} \
             VolErrors={} VolWrites={} MaxVolBytes={} EndTime={} VolStatus={} Slot={} relabel={} InChanger={} \
             VolReadTime={} VolWriteTime={} VolFirstWritten={}\n",
            bash_spaces(&self.volume),
            self.jobs,
            self.files,
            self.blocks,
            self.bytes,
            self.mounts,
            self.errors,
            self.writes,
            self.max_bytes,
            self.end_time,
            self.status,
            self.slot,
            self.relabel as i32,
            self.in_changer as i32,
            self.read_time,
            self.write_time,
            self.first_written
        )
    }
}

/// A JobMedia record: which FileIndexes of the job lie where on a volume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CreateJobMedia {
    /// `FirstIndex`.
    pub first_index: u32,
    /// `LastIndex`.
    pub last_index: u32,
    /// `StartFile` (high 32 bits of the start address on a file volume).
    pub start_file: u32,
    /// `EndFile`.
    pub end_file: u32,
    /// `StartBlock` (low 32 bits of the start address).
    pub start_block: u32,
    /// `EndBlock` (low 32 bits of the address of the last byte).
    pub end_block: u32,
    /// `MediaId`.
    pub media_id: u64,
}

impl CreateJobMedia {
    /// `Create_job_media` of `askdir.cc` (Copy=0, Strip=0); the placeholder
    /// record is all zeros except the MediaId.
    pub fn format(&self, job: &str) -> String {
        format!(
            "CatReq Job={job} CreateJobMedia FirstIndex={} LastIndex={} StartFile={} EndFile={} StartBlock={} \
             EndBlock={} Copy=0 Strip=0 MediaId={}\n",
            self.first_index, self.last_index, self.start_file, self.end_file, self.start_block, self.end_block, self.media_id
        )
    }
}

/// `OK_media`: the Director's answer with a volume's catalog record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VolumeInfo {
    /// `VolName`.
    pub volume: String,
    /// `VolJobs`.
    pub jobs: u32,
    /// `VolFiles`.
    pub files: u32,
    /// `VolBlocks`.
    pub blocks: u32,
    /// `VolBytes`.
    pub bytes: u64,
    /// `VolMounts`.
    pub mounts: u32,
    /// `VolErrors`.
    pub errors: u32,
    /// `VolWrites`.
    pub writes: u32,
    /// `MaxVolBytes`.
    pub max_bytes: u64,
    /// `VolCapacityBytes`.
    pub capacity_bytes: u64,
    /// `VolStatus`.
    pub status: String,
    /// `Slot`.
    pub slot: i32,
    /// `MaxVolJobs`.
    pub max_jobs: u32,
    /// `MaxVolFiles`.
    pub max_files: u32,
    /// `InChanger`.
    pub in_changer: bool,
    /// `VolReadTime`.
    pub read_time: i64,
    /// `VolWriteTime`.
    pub write_time: i64,
    /// `EndFile`.
    pub end_file: u32,
    /// `EndBlock`.
    pub end_block: u32,
    /// `LabelType`.
    pub label_type: i32,
    /// `MediaId`.
    pub media_id: u64,
    /// `EncryptionKey` (`*None*` style placeholder when none).
    pub encryption_key: String,
    /// `MinBlocksize`.
    pub min_blocksize: u32,
    /// `MaxBlocksize`.
    pub max_blocksize: u32,
}

impl VolumeInfo {
    /// `sscanf(msg, OK_media, …) == 24` of `DoGetVolumeInfo`; `None` for any
    /// other answer (e.g. `1901 No Media.`).
    pub fn parse(msg: &str) -> Option<VolumeInfo> {
        let rest = msg.strip_prefix("1000 OK ")?;
        let words: Vec<&str> = rest.split_whitespace().collect();
        let keys = [
            "VolName=", "VolJobs=", "VolFiles=", "VolBlocks=", "VolBytes=", "VolMounts=", "VolErrors=", "VolWrites=",
            "MaxVolBytes=", "VolCapacityBytes=", "VolStatus=", "Slot=", "MaxVolJobs=", "MaxVolFiles=", "InChanger=",
            "VolReadTime=", "VolWriteTime=", "EndFile=", "EndBlock=", "LabelType=", "MediaId=", "EncryptionKey=",
            "MinBlocksize=", "MaxBlocksize=",
        ];
        if words.len() < keys.len() {
            return None;
        }
        let mut v = Vec::with_capacity(keys.len());
        for (w, k) in words.iter().zip(keys) {
            v.push(w.strip_prefix(k)?);
        }
        let n = |i: usize| v[i].parse::<i64>().ok();
        Some(VolumeInfo {
            volume: unbash_spaces(v[0]),
            jobs: n(1)? as u32,
            files: n(2)? as u32,
            blocks: n(3)? as u32,
            bytes: v[4].parse().ok()?,
            mounts: n(5)? as u32,
            errors: n(6)? as u32,
            writes: n(7)? as u32,
            max_bytes: v[8].parse().ok()?,
            capacity_bytes: v[9].parse().ok()?,
            status: v[10].to_string(),
            slot: n(11)? as i32,
            max_jobs: n(12)? as u32,
            max_files: n(13)? as u32,
            in_changer: n(14)? != 0,
            read_time: n(15)?,
            write_time: n(16)?,
            end_file: n(17)? as u32,
            end_block: n(18)? as u32,
            label_type: n(19)? as i32,
            media_id: v[20].parse().ok()?,
            encryption_key: v[21].to_string(),
            min_blocksize: n(22)? as u32,
            max_blocksize: n(23)? as u32,
        })
    }

    /// The counters of this record as an `UpdateMedia` starting point.
    pub fn to_update(&self) -> UpdateMedia {
        UpdateMedia {
            volume: self.volume.clone(),
            jobs: self.jobs,
            files: self.files,
            blocks: self.blocks,
            bytes: self.bytes,
            mounts: self.mounts,
            errors: self.errors,
            writes: self.writes,
            max_bytes: self.max_bytes,
            end_time: 0,
            status: self.status.clone(),
            slot: self.slot,
            relabel: false,
            in_changer: self.in_changer,
            read_time: self.read_time,
            write_time: self.write_time,
            first_written: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The answer odir gave bareos-sd in the rig (sd.log, 20.11.2026).
    const ANSWER: &str = "1000 OK VolName=VVol-0001 VolJobs=0 VolFiles=0 VolBlocks=1 VolBytes=230 VolMounts=0 VolErrors=0 \
        VolWrites=1 MaxVolBytes=1073741824 VolCapacityBytes=0 VolStatus=Append Slot=0 MaxVolJobs=0 MaxVolFiles=0 \
        InChanger=0 VolReadTime=0 VolWriteTime=2 EndFile=0 EndBlock=0 LabelType=0 MediaId=1 EncryptionKey=*None* \
        MinBlocksize=0 MaxBlocksize=0\n";

    #[test]
    fn ok_media_is_parsed_like_do_get_volume_info() {
        let v = VolumeInfo::parse(ANSWER).unwrap();
        assert_eq!((v.volume.as_str(), v.blocks, v.bytes, v.max_bytes), ("VVol-0001", 1, 230, 1 << 30));
        assert_eq!((v.status.as_str(), v.media_id, v.write_time), ("Append", 1, 2));
        assert_eq!(VolumeInfo::parse("1901 No Media.\n"), None);
        assert_eq!(VolumeInfo::parse(&ANSWER.replace("VolBytes=230", "VolBytes=x")), None);
    }

    #[test]
    fn requests_match_the_lines_bareos_sd_sent_in_the_rig() {
        let job = "VBackup.2026-11-20_19.59.23_03";
        assert_eq!(find_media(job, 1, "VPool", "VFile", ""),
            format!("CatReq Job={job} FindMedia=1 pool_name=VPool media_type=VFile unwanted_volumes=\n"));
        assert_eq!(get_vol_info(job, "VVol-0001", true), format!("CatReq Job={job} GetVolInfo VolName=VVol-0001 write=1\n"));
        let mut u = VolumeInfo::parse(ANSWER).unwrap().to_update();
        u.end_time = 1795204766;
        u.relabel = true;
        assert_eq!(u.format(job), format!(
            "CatReq Job={job} UpdateMedia VolName=VVol-0001 VolJobs=0 VolFiles=0 VolBlocks=1 VolBytes=230 VolMounts=0 \
             VolErrors=0 VolWrites=1 MaxVolBytes=1073741824 EndTime=1795204766 VolStatus=Append Slot=0 relabel=1 \
             InChanger=0 VolReadTime=0 VolWriteTime=2 VolFirstWritten=0\n"));
        let jm = CreateJobMedia { first_index: 1, last_index: 13, start_block: 230, end_block: 12592462, media_id: 1, ..Default::default() };
        assert_eq!(jm.format(job), format!(
            "CatReq Job={job} CreateJobMedia FirstIndex=1 LastIndex=13 StartFile=0 EndFile=0 StartBlock=230 \
             EndBlock=12592462 Copy=0 Strip=0 MediaId=1\n"));
        assert_eq!(job_status(job, 'R'), format!("Status Job={job} JobStatus=82\n"));
        assert_eq!(jmsg(job, "vsd", 1, 6, 5, "Releasing device.\n"), format!("Jmsg Job={job} type=6 level=5 vsd JobId 1: Releasing device.\n"));
        assert_eq!(jmsg(job, "vsd", 1, 3, 5, "No device.\n"), format!("Jmsg Job={job} type=3 level=5 vsd JobId 1: Fatal error: No device.\n"));
        assert!(jmsg(job, "vsd", 1, 5, 5, "x").ends_with(": Warning: x") && jmsg(job, "vsd", 1, 4, 5, "x").ends_with(": Error: x"));
    }

    #[test]
    fn file_attributes_header_is_serialized_big_endian() {
        let m = file_attributes("J", 1, 2, 3, 4, b"xy");
        let head = b"UpdCat Job=J FileAttributes ";
        assert_eq!(&m[..head.len()], head);
        assert_eq!(&m[head.len()..], &[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0, 2, b'x', b'y']);
    }
}
