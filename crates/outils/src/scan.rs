// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Katalogeinträge aus Volumes wiederherstellen (`oscan`, Ersatz für `bscan`).
//!
//! Aus den Labels eines Volumes entsteht ein SQL-Skript. Es legt fehlende
//! Pool-, Client-, FileSet-, Media-, Job- und JobMedia-Einträge an und mit
//! [`Options::files`] auch die File-Einträge samt Prüfsumme. Das Skript ist
//! wiederholbar: Was schon im Katalog steht (gleiche JobId, gleiches Volume,
//! Dateien eines Jobs), wird nicht noch einmal angelegt. `oscan` gibt es
//! aus (Voreinstellung) oder führt es in einer Transaktion mit `psql` aus.
//!
//! Was aus einem Volume nicht hervorgeht, bekommt eine vorsichtige
//! Voreinstellung: ein neu angelegtes Volume hat den Status `Full` (mit
//! `update volume volstatus=Append` freigeben), ein neuer Pool die Art
//! `Backup`. Ein Job, dessen JobId schon im Katalog mit einem anderen
//! Job-Namen steht, wird nicht angefasst und gemeldet.

use std::collections::BTreeMap;

use bareos_format::label::{SessionLabel, VolumeLabel, EOS_LABEL, SOS_LABEL, VOL_LABEL, PRE_LABEL};
use bareos_format::stream::*;
use bareos_format::volume::VolRecord;

use crate::parse_attributes;

/// Was `oscan` anlegt.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// File-Einträge anlegen (`-s`).
    pub files: bool,
    /// Media-Eintrag eines vorhandenen Volumes nachführen (`-m`): Größe und
    /// Ende.
    pub update_media: bool,
    /// Pool statt des im Label stehenden (`-p`).
    pub pool: Option<String>,
}

/// Ein Job auf dem Volume.
#[derive(Debug, Clone)]
struct Job {
    /// SOS-Label.
    start: SessionLabel,
    /// EOS-Label, falls vorhanden.
    end: Option<SessionLabel>,
    /// Byte-Offset des ersten Blocks mit Datensätzen der Session.
    start_offset: u64,
    /// Letztes Byte des letzten Blocks mit Datensätzen der Session.
    end_offset: u64,
    /// Erster FileIndex, der in der Session vorkommt.
    first_index: i32,
    /// Letzter FileIndex.
    last_index: i32,
    /// Datei in Arbeit: (FileIndex, Pfad, Name, LStat).
    pending: Option<(i32, String, String, String)>,
    /// Fertige Dateien: (FileIndex, Pfad, Name, LStat, Prüfsumme).
    files: Vec<(i32, String, String, String, String)>,
}

/// Das Skript zu einem oder mehreren Volumes.
pub struct Scan {
    /// Die Optionen.
    options: Options,
    /// Das SQL.
    sql: String,
    /// Hinweise und Warnungen für den Aufrufer.
    pub notes: Vec<String>,
    /// Die Jobs, in Reihenfolge des ersten Auftretens.
    order: Vec<(u32, u32)>,
    /// Die Jobs des gerade gelesenen Volumes.
    jobs: BTreeMap<(u32, u32), Job>,
    /// Das Label des gerade gelesenen Volumes.
    label: Option<VolumeLabel>,
    /// Ende (Byte hinter dem Block) des gerade gelesenen Blocks.
    block_end: u64,
    /// Größe der Volume-Datei.
    volume_bytes: u64,
    /// Anzahl der Volumes im Skript.
    volumes: usize,
    /// Anzahl der Jobs im Skript.
    pub jobs_found: usize,
}

/// SQL-Zeichenkette in Hochkommas.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''").replace('\0', ""))
}

/// Pfad und Dateiname: der Pfad endet auf `/`, bei einem Verzeichnis ist der
/// Name leer (wie im Katalog).
pub fn split_path(full: &str) -> (String, String) {
    if full.ends_with('/') {
        return (full.to_string(), String::new());
    }
    match full.rfind('/') {
        Some(i) => (full[..=i].to_string(), full[i + 1..].to_string()),
        None => (String::new(), full.to_string()),
    }
}

/// Der Buchstabe aus einer Zahl im Label (`'B'` = 66), sonst `?`.
fn letter(value: u32) -> char {
    char::from_u32(value).filter(char::is_ascii_alphabetic).unwrap_or('?')
}

impl Scan {
    /// Ein leeres Skript.
    pub fn new(options: Options) -> Self {
        let mut sql = String::from(
            "-- oscan: catalog entries from volumes (replaces bscan); one transaction\n\
             CREATE TEMPORARY TABLE oscan_file (jobid integer, fileindex integer, path text, name text, lstat text, md5 text) ON COMMIT DROP;\n",
        );
        sql.push('\n');
        Self { options, sql, notes: Vec::new(), order: Vec::new(), jobs: BTreeMap::new(), label: None, block_end: 0, volume_bytes: 0, volumes: 0, jobs_found: 0 }
    }

    /// Beginnt ein Volume (`bytes`: Größe der Datei).
    pub fn begin_volume(&mut self, bytes: u64) {
        self.jobs.clear();
        self.label = None;
        self.volume_bytes = bytes;
    }

    /// Meldet den nächsten Block (Byte-Offset und Länge); vor seinen Datensätzen.
    pub fn block(&mut self, offset: u64, len: u32) {
        self.block_end = offset + u64::from(len);
    }

    /// Verarbeitet einen Datensatz des Volumes.
    pub fn record(&mut self, rec: &VolRecord) {
        let key = (rec.vol_session_id, rec.vol_session_time);
        if let Some(job) = self.jobs.get_mut(&key) {
            job.end_offset = self.block_end.saturating_sub(1);
        }
        match rec.file_index {
            VOL_LABEL | PRE_LABEL => match VolumeLabel::parse(&rec.data) {
                Ok(l) => self.label = Some(l),
                Err(e) => self.notes.push(format!("volume label not readable: {e}")),
            },
            SOS_LABEL => match SessionLabel::parse(&rec.data, false) {
                Ok(start) => {
                    if !self.order.contains(&key) {
                        self.order.push(key);
                    }
                    self.jobs.insert(key, Job { start, end: None, start_offset: rec.block_offset, end_offset: 0, first_index: 0, last_index: 0, pending: None, files: Vec::new() });
                }
                Err(e) => self.notes.push(format!("session label not readable: {e}")),
            },
            EOS_LABEL => {
                if let Some(job) = self.jobs.get_mut(&key) {
                    flush_pending(job);
                    match SessionLabel::parse(&rec.data, true) {
                        Ok(end) => job.end = Some(end),
                        Err(e) => self.notes.push(format!("end label not readable: {e}")),
                    }
                }
            }
            fi if fi > 0 => {
                let Some(job) = self.jobs.get_mut(&key) else { return };
                if job.first_index == 0 || fi < job.first_index {
                    job.first_index = fi;
                }
                job.last_index = job.last_index.max(fi);
                if !self.options.files {
                    return;
                }
                match rec.stream & STREAMMASK_TYPE {
                    STREAM_UNIX_ATTRIBUTES | STREAM_UNIX_ATTRIBUTES_EX => {
                        flush_pending(job);
                        if let Some((name, lstat, _link)) = parse_attributes(&rec.data) {
                            let (path, file) = split_path(&name);
                            job.pending = Some((fi, path, file, String::from_utf8_lossy(&lstat).into_owned()));
                        }
                    }
                    STREAM_MD5_DIGEST | STREAM_SHA1_DIGEST | STREAM_SHA256_DIGEST | STREAM_SHA512_DIGEST | STREAM_XXH128_DIGEST => {
                        if let Some((index, path, name, lstat)) = job.pending.take() {
                            let digest = String::from_utf8_lossy(&rec.data).trim_end_matches('\0').to_string();
                            job.files.push((index, path, name, lstat, digest));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Das Volume ist gelesen: seine SQL-Anweisungen anhängen.
    pub fn end_volume(&mut self, file_name: &str) {
        let Some(label) = self.label.take() else {
            self.notes.push(format!("{file_name}: no volume label, nothing scanned"));
            return;
        };
        self.volumes += 1;
        let pool = self.options.pool.clone().unwrap_or_else(|| label.pool_name.clone());
        let volume = quote(&label.volume_name);
        let s = &mut self.sql;
        s.push_str(&format!("-- volume {}\n", label.volume_name));
        s.push_str(&format!("INSERT INTO Pool (Name, PoolType) SELECT {0}, 'Backup' WHERE NOT EXISTS (SELECT 1 FROM Pool WHERE Name = {0});\n", quote(&pool)));
        s.push_str(&format!(
            "INSERT INTO Media (VolumeName, MediaType, MediaTypeId, PoolId, MaxVolBytes, VolCapacityBytes, Recycle, VolRetention, VolUseDuration, \
             MaxVolJobs, MaxVolFiles, VolStatus, Slot, VolBytes, InChanger, VolReadTime, VolWriteTime, EndFile, EndBlock, LabelType, StorageId, \
             DeviceId, LocationId, ScratchPoolId, RecyclePoolId, Enabled, ActionOnPurge, EncryptionKey, MinBlocksize, MaxBlocksize, VolFiles, LabelDate)\n \
             SELECT {volume}, {mt}, 0, (SELECT PoolId FROM Pool WHERE Name = {pool}), 0, 0, 1, 31536000, 0, 0, 0, 'Full', 0, {bytes}, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, '', 0, 0, 0, \
             to_timestamp({label_time})::timestamp\n WHERE NOT EXISTS (SELECT 1 FROM Media WHERE VolumeName = {volume});\n",
            mt = quote(&label.media_type), pool = quote(&pool), bytes = self.volume_bytes, label_time = label.label_btime / 1_000_000));
        if self.options.update_media {
            s.push_str(&format!("UPDATE Media SET VolBytes = {} WHERE VolumeName = {volume};\n", self.volume_bytes));
        }
        let keys: Vec<_> = self.order.iter().filter(|k| self.jobs.contains_key(k)).copied().collect();
        let mut files_written = 0usize;
        for key in keys {
            let Some(job) = self.jobs.get_mut(&key) else { continue };
            flush_pending(job);
            let st = &job.start;
            let end = job.end.as_ref();
            self.jobs_found += 1;
            if end.is_none() {
                self.notes.push(format!("JobId {}: no end label on {}; the job did not finish on this volume (JobStatus 'f')", st.job_id, label.volume_name));
            }
            let status = end.and_then(|e| e.job_status).map(letter).filter(|c| *c != '?').unwrap_or('f');
            let (files, bytes, errors) = end.map_or((0, 0, 0), |e| (e.job_files.unwrap_or(0), e.job_bytes.unwrap_or(0), e.job_errors.unwrap_or(0)));
            let id = st.job_id;
            s.push_str(&format!("-- JobId {id}: {}\n", st.job));
            s.push_str(&format!(
                "INSERT INTO Client (Name, Uname, AutoPrune, FileRetention, JobRetention) SELECT {0}, '', 1, 2592000, 15552000 WHERE NOT EXISTS (SELECT 1 FROM Client WHERE Name = {0});\n",
                quote(&st.client_name)));
            s.push_str(&format!(
                "INSERT INTO FileSet (FileSet, MD5, CreateTime) SELECT {0}, {1}, now() WHERE NOT EXISTS (SELECT 1 FROM FileSet WHERE FileSet = {0});\n",
                quote(&st.fileset_name), quote(&st.fileset_md5)));
            s.push_str(&format!(
                "DO $$ BEGIN IF EXISTS (SELECT 1 FROM Job WHERE JobId = {id} AND Job <> {job}) THEN RAISE EXCEPTION 'JobId {id} is in the catalog as another job'; END IF; END $$;\n",
                job = quote(&st.job)));
            s.push_str(&format!(
                "INSERT INTO Job (JobId, Job, Name, Type, Level, ClientId, JobStatus, SchedTime, StartTime, EndTime, RealEndTime, JobTDate, VolSessionId, VolSessionTime, \
                 JobFiles, JobBytes, JobErrors, PoolId, FileSetId, PriorJobId, HasBase, HasCache, Reviewed, Comment)\n \
                 SELECT {id}, {job}, {name}, '{ty}', '{lv}', (SELECT MIN(ClientId) FROM Client WHERE Name = {client}), '{status}', \
                 to_timestamp({t0})::timestamp, to_timestamp({t0})::timestamp, to_timestamp({t1})::timestamp, to_timestamp({t1})::timestamp, {t1}, {sid}, {stime}, \
                 {files}, {bytes}, {errors}, (SELECT PoolId FROM Pool WHERE Name = {pool}), (SELECT MIN(FileSetId) FROM FileSet WHERE FileSet = {fs}), 0, 0, 0, 0, ''\n \
                 WHERE NOT EXISTS (SELECT 1 FROM Job WHERE JobId = {id});\n",
                job = quote(&st.job), name = quote(&st.job_name), ty = letter(st.job_type), lv = letter(st.job_level), client = quote(&st.client_name),
                t0 = st.write_btime / 1_000_000, t1 = end.map_or(st.write_btime, |e| e.write_btime) / 1_000_000, sid = key.0, stime = key.1,
                pool = quote(&pool), fs = quote(&st.fileset_name)));
            // file volumes: the position is the byte offset in the file (StartFile 0), as osd and bareos-sd record it
            let (sf, sb, ef, eb) = (0u32, job.start_offset as u32, 0u32, job.end_offset as u32);
            s.push_str(&format!(
                "INSERT INTO JobMedia (JobId, MediaId, FirstIndex, LastIndex, StartFile, EndFile, StartBlock, EndBlock, VolIndex, JobBytes)\n \
                 SELECT {id}, m.MediaId, {fi}, {li}, {sf}, {ef}, {sb}, {eb}, (SELECT COUNT(*) + 1 FROM JobMedia WHERE JobId = {id}), 0 FROM Media m \
                 WHERE m.VolumeName = {volume} AND NOT EXISTS (SELECT 1 FROM JobMedia j WHERE j.JobId = {id} AND j.MediaId = m.MediaId);\n",
                fi = job.first_index, li = job.last_index));
            for chunk in job.files.chunks(500) {
                let rows: Vec<String> = chunk.iter().map(|(fi, path, name, lstat, md5)| {
                    format!("({id}, {fi}, {}, {}, {}, {})", quote(path), quote(name), quote(lstat), quote(md5))
                }).collect();
                s.push_str(&format!("INSERT INTO oscan_file VALUES {};\n", rows.join(", ")));
            }
            files_written += job.files.len();
            if !job.files.is_empty() {
                s.push_str(&format!(
                    "INSERT INTO Path (Path) SELECT DISTINCT f.path FROM oscan_file f WHERE f.jobid = {id} AND NOT EXISTS (SELECT 1 FROM Path p WHERE p.Path = f.path);\n\
                     INSERT INTO File (FileIndex, JobId, PathId, Name, DeltaSeq, MarkId, LStat, MD5)\n \
                     SELECT f.fileindex, f.jobid, (SELECT MIN(p.PathId) FROM Path p WHERE p.Path = f.path), f.name, 0, 0, f.lstat, f.md5 FROM oscan_file f \
                     WHERE f.jobid = {id} AND NOT EXISTS (SELECT 1 FROM File x WHERE x.JobId = {id});\n"));
            }
        }
        s.push_str("SELECT setval(pg_get_serial_sequence('job', 'jobid'), GREATEST((SELECT COALESCE(MAX(JobId), 1) FROM Job), 1));\n\n");
        if self.options.files && files_written == 0 {
            self.notes.push(format!("{}: no file records found (the volume holds no completed file with attributes)", label.volume_name));
        }
    }

    /// Das Skript.
    pub fn finish(self) -> String {
        let mut sql = self.sql;
        sql.push_str(&format!("-- {} volume(s), {} job(s)\n", self.volumes, self.jobs_found));
        sql
    }
}

/// Eine Datei in Arbeit ohne Prüfsumme gilt als fertig (leere Datei, Verzeichnis, Link).
fn flush_pending(job: &mut Job) {
    if let Some((index, path, name, lstat)) = job.pending.take() {
        job.files.push((index, path, name, lstat, String::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_like_the_catalog() {
        assert_eq!(split_path("/etc/hosts"), ("/etc/".into(), "hosts".into()));
        assert_eq!(split_path("/etc/"), ("/etc/".into(), String::new()));
        assert_eq!(split_path("hosts"), (String::new(), "hosts".into()));
    }

    #[test]
    fn quotes_are_doubled_and_nul_dropped() {
        assert_eq!(quote("it's\0"), "'it''s'");
    }
}
