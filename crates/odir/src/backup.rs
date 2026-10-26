//! Director side of a native Backup job (S-19, Compatibility log C-058):
//! job.cc SetupJob for JT_BACKUP (GetOrCreateClientRecord,
//! GetOrCreateFilesetRecord, GetLevelSinceTime), backup.cc
//! DoNativeBackupInit (PoolId) and the catalog rows DoNativeBackup and
//! NativeBackupCleanup write (UpdateJobStartRecord, UpdateClientRecord,
//! UpdateJobEndRecord). The daemon side runs in backup_worker.rs.

use bareos_config::raw::{ConfResource, Directive};

use crate::catalog_worker::CatalogHandle;
use crate::commands::defaults::{bool_directive, time_directive};

/// EscapeString(): quotes doubled.
pub(crate) fn esc(text: &str) -> String {
    text.replace('\'', "''")
}

fn first_row(cat: &CatalogHandle, sql: &str) -> Result<Option<Vec<Option<String>>>, String> {
    Ok(cat.query_rows(sql)?.rows.into_iter().next())
}

fn cell(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

/// AutoPrune, FileRetention, JobRetention of a Client resource
/// (dird_conf.cc defaults: no, 60 days, 180 days).
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct ClientRetention {
    pub auto_prune: bool,
    pub file_retention: u64,
    pub job_retention: u64,
}

pub(crate) fn client_retention(cfg: &crate::director_config::DirectorConfig, client: &str) -> ClientRetention {
    let dirs: Vec<Directive> = crate::commands::defaults::raw(cfg, "client", client)
        .map(|r| r.directives.clone()).unwrap_or_default();
    let time = |key: &str, default: u64| {
        if dirs.iter().any(|d| d.norm_key() == key) { time_directive(&dirs, key) } else { default }
    };
    ClientRetention {
        auto_prune: bool_directive(&dirs, "autoprune", false),
        file_retention: time("fileretention", 60 * 86400),
        job_retention: time("jobretention", 180 * 86400),
    }
}

/// job.cc GetOrCreateClientRecord() / sql_create.cc CreateClientRecord():
/// the ClientId of the client, inserted with the resource's retention.
pub(crate) fn get_or_create_client_record(cat: &CatalogHandle, name: &str, ret: ClientRetention) -> Result<i64, String> {
    if let Some(row) = first_row(cat, &format!("SELECT ClientId,Uname FROM Client WHERE Name='{}'", esc(name)))? {
        return Ok(cell(&row, 0).parse().unwrap_or(0));
    }
    let row = first_row(cat, &format!(
        "INSERT INTO Client (Name,Uname,AutoPrune,FileRetention,JobRetention) VALUES ('{}','',{},{},{}) RETURNING ClientId",
        esc(name), ret.auto_prune as i32, ret.file_retention, ret.job_retention))?;
    row.map(|r| cell(&r, 0).parse().unwrap_or(0)).filter(|id| *id > 0)
        .ok_or_else(|| format!("Could not create Client record. ERR=Create DB Client record {name} failed\n"))
}

/// fd_cmds.cc SendJobInfoToFileDaemon(): UpdateClientRecord() with the
/// Uname the File daemon answered with ("2000 OK Job <uname>").
pub(crate) fn update_client_record_sql(name: &str, ret: ClientRetention, uname: &str) -> String {
    format!("UPDATE Client SET AutoPrune={},FileRetention={},JobRetention={},Uname='{}' WHERE Name='{}'",
        ret.auto_prune as i32, ret.file_retention, ret.job_retention, esc(uname), esc(name))
}

/// inc_conf.cc: the FileSet MD5 is updated with every `File =` and
/// `Plugin =` name of the Include/Exclude blocks (pass 1, in file order);
/// job.cc GetOrCreateFilesetRecord: BinToBase64(digest, compatible=false).
/// None when the FileSet has no Include/Exclude block (!have_MD5).
pub(crate) fn fileset_md5(res: &ConfResource) -> Option<String> {
    use md5::{Digest, Md5};
    let blocks: Vec<&Vec<Directive>> = res.directives.iter()
        .filter(|d| matches!(d.norm_key().as_str(), "include" | "exclude"))
        .filter_map(|d| d.block.as_ref()).collect();
    if blocks.is_empty() {
        return None;
    }
    let mut md5 = Md5::new();
    for body in blocks {
        for d in body.iter().filter(|d| matches!(d.norm_key().as_str(), "file" | "plugin")) {
            for value in d.values() {
                md5.update(value.as_bytes());
            }
        }
    }
    Some(bareos_bnet::cram_md5::bareos_bin_to_base64(&md5.finalize(), false))
}

/// FileSetId and CreateTime of the job's FileSet record.
#[derive(Debug, Clone, Default)]
pub(crate) struct FilesetRecord {
    pub id: i64,
    pub create_time: String,
    pub md5: String,
    /// "FileSet MD5 digest not found." (M_WARNING) when the FileSet has none.
    pub warning: Option<String>,
}

/// job.cc GetOrCreateFilesetRecord() with sql_create.cc CreateFilesetRecord().
pub(crate) fn get_or_create_fileset_record(cat: &CatalogHandle, res: &ConfResource, now: i64) -> Result<FilesetRecord, String> {
    let name = res.name().unwrap_or_default().to_string();
    let (md5, warning) = match fileset_md5(res) {
        Some(md5) => (md5, None),
        None => (String::new(), Some("FileSet MD5 digest not found.\n".to_string())),
    };
    let ignore_changes = bool_directive(&res.directives, "ignorefilesetchanges", false);
    if ignore_changes {
        // GetFilesetRecord(): the newest record of that name.
        if let Some(row) = first_row(cat, &format!(
            "SELECT FileSetId,FileSet,MD5,CreateTime FROM FileSet WHERE FileSet='{}' ORDER BY CreateTime DESC LIMIT 1",
            esc(&name)))? {
            return Ok(FilesetRecord { id: cell(&row, 0).parse().unwrap_or(0), create_time: cell(&row, 3),
                md5: cell(&row, 2), warning });
        }
    }
    let text = crate::commands::show::fileset_text(res);
    let error = |e: String| format!("Could not create FileSet \"{name}\" record. ERR={e}\n");
    if let Some(row) = first_row(cat, &format!(
        "SELECT FileSetId,CreateTime FROM FileSet WHERE FileSet='{}' AND MD5='{}'", esc(&name), esc(&md5))).map_err(error)? {
        let (id, create_time) = (cell(&row, 0).parse().unwrap_or(0), cell(&row, 1));
        cat.execute(&format!(
            "UPDATE FileSet SET (FileSet,MD5,CreateTime,FileSetText) = ('{0}','{1}','{2}','{3}') WHERE FileSet='{0}' AND MD5='{1}' ",
            esc(&name), esc(&md5), create_time, esc(&text))).map_err(error)?;
        return Ok(FilesetRecord { id, create_time, md5, warning });
    }
    let create_time = crate::job_messages::bstrutime(now);
    let row = first_row(cat, &format!(
        "INSERT INTO FileSet (FileSet,MD5,CreateTime,FileSetText) VALUES ('{}','{}','{}','{}') RETURNING FileSetId",
        esc(&name), esc(&md5), create_time, esc(&text))).map_err(error)?;
    let id = row.map(|r| cell(&r, 0).parse().unwrap_or(0)).unwrap_or(0);
    if id == 0 {
        return Err(error("Create DB FileSet record failed".into()));
    }
    Ok(FilesetRecord { id, create_time, md5, warning })
}

/// GetLevelSinceTime() result.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LevelSince {
    /// The job level after any upgrade ('F', 'I', 'D').
    pub level: char,
    /// jcr->dir_impl->since: ", since=…" or " (upgraded from …)".
    pub since: String,
    /// jcr->starttime_string and PrevJob of the level command.
    pub stime: String,
    pub prev_job: String,
    /// The level was upgraded (ApplyPoolOverrides(force)).
    pub pool_updated: bool,
    /// Jmsg M_INFO texts, in order.
    pub messages: Vec<String>,
}

/// JobLevelToString().
pub(crate) fn level_to_string(level: char) -> &'static str {
    match level {
        'F' => "Full",
        'I' => "Incremental",
        'D' => "Differential",
        'B' => "Base",
        'V' => "VirtualFull",
        'S' => "Since",
        _ => "Unknown",
    }
}

/// The catalog queries of GetLevelSinceTime (sql_find.cc), so the logic
/// can be tested without a database.
pub(crate) trait SinceDb {
    /// First row (StartTime, Job/Level) of a query; Err: query error.
    fn row(&self, sql: &str) -> Result<Option<(String, String)>, String>;
}

impl SinceDb for CatalogHandle {
    fn row(&self, sql: &str) -> Result<Option<(String, String)>, String> {
        Ok(first_row(self, sql)?.map(|r| (cell(&r, 0), cell(&r, 1))))
    }
}

/// Job directives GetLevelSinceTime uses.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LevelPolicy {
    pub max_diff_interval: i64,
    pub max_full_interval: i64,
    pub max_vfull_interval: i64,
    pub rerun_failed_levels: bool,
}

impl LevelPolicy {
    pub(crate) fn of_job(dirs: &[Directive]) -> Self {
        Self {
            max_diff_interval: time_directive(dirs, "maxdiffinterval") as i64,
            max_full_interval: time_directive(dirs, "maxfullinterval") as i64,
            max_vfull_interval: time_directive(dirs, "maxvirtualfullinterval") as i64,
            rerun_failed_levels: bool_directive(dirs, "rerunfailedlevels", false),
        }
    }
}

/// job.cc GetLevelSinceTime() with sql_find.cc FindJobStartTime(),
/// FindLastJobStartTime() and FindFailedJobSince() (JobType 'B').
pub(crate) fn get_level_since_time(db: &dyn SinceDb, job_name: &str, client_id: i64, fileset_id: i64, level: char,
                                   policy: LevelPolicy, now: i64) -> LevelSince {
    let mut out = LevelSince { level, ..Default::default() };
    if !matches!(level, 'D' | 'I') {
        return out;
    }
    let name = esc(job_name);
    let last = |lvl: char| format!(
        "SELECT StartTime, Job FROM Job WHERE JobStatus IN ('T','W') AND Type='B' AND Level='{lvl}' AND Name='{name}' \
         AND ClientId={client_id} AND FileSetId={fileset_id} ORDER BY StartTime DESC LIMIT 1");
    let mut do_full = false;

    // FindJobStartTime(): stime defaults to "0000-00-00 00:00:00".
    out.stime = "0000-00-00 00:00:00".into();
    let full_query = last('F');
    let found: Result<(String, String), String> = (|| {
        let cmd = if level == 'I' {
            match db.row(&full_query) {
                Err(e) => return Err(format!("cats/sql_find.cc:94 Query error for start time request: ERR={e}\nCMD={full_query}\n")),
                Ok(None) => return Err("No prior Full backup Job record found.\n".into()),
                Ok(Some(_)) => format!(
                    "SELECT StartTime, Job FROM Job WHERE JobStatus IN ('T','W') AND Type='B' AND Level IN ('I','D','F') \
                     AND Name='{name}' AND ClientId={client_id} AND FileSetId={fileset_id} ORDER BY StartTime DESC LIMIT 1"),
            }
        } else {
            full_query.clone()
        };
        match db.row(&cmd) {
            // Mmsg2() puts "file:line " in front of the text.
            Err(e) => Err(format!("cats/sql_find.cc:125 Query error for start time request: ERR={e}\nCMD={cmd}\n")),
            Ok(None) => Err(format!("cats/sql_find.cc:131 No Job record found: ERR=\nCMD={cmd}\n")),
            Ok(Some(row)) => Ok(row),
        }
    })();
    match found {
        Ok((stime, job)) => {
            out.stime = stime;
            out.prev_job = job;
        }
        Err(message) => {
            out.messages.push(message);
            do_full = true;
        }
    }

    let full = db.row(&last('F')).ok().flatten();
    let last_full_time = full.as_ref().map(|(t, _)| crate::btime::str_to_utime(t)).unwrap_or(0);
    let have_full = full.is_some();
    if !have_full {
        do_full = true;
    }
    let mut do_diff = false;
    if have_full && level == 'I' && policy.max_diff_interval > 0 {
        let last_diff_time = match db.row(&last('D')).ok().flatten() {
            Some((t, _)) => crate::btime::str_to_utime(&t).max(last_full_time),
            None => last_full_time,
        };
        do_diff = now - last_diff_time >= policy.max_diff_interval;
    }
    let mut do_vfull = false;
    if have_full && policy.max_full_interval > 0 {
        do_full = now - last_full_time >= policy.max_full_interval;
    } else if have_full && policy.max_vfull_interval > 0 {
        do_vfull = now - last_full_time >= policy.max_vfull_interval;
    }

    let upgraded = |from: char| format!(" (upgraded from {})", level_to_string(from));
    if do_full {
        out.messages.push("No prior or suitable Full backup found in catalog. Doing FULL backup.\n".into());
        out.since = upgraded(level);
        out.level = 'F';
        out.pool_updated = true;
    } else if do_vfull {
        out.messages.push("No prior or suitable Full backup found in catalog. Doing Virtual FULL backup.\n".into());
        out.since = upgraded(level);
        out.level = 'V';
        out.pool_updated = true;
    } else if do_diff {
        out.messages.push("No prior or suitable Differential backup found in catalog. Doing Differential backup.\n".into());
        out.since = upgraded(level);
        out.level = 'D';
        out.pool_updated = true;
    } else {
        if policy.rerun_failed_levels {
            let failed = format!(
                "SELECT Level FROM Job WHERE JobStatus NOT IN ('T','W') AND Type='B' AND Level IN ('F','D') \
                 AND Name='{name}' AND ClientId={client_id} AND FileSetId={fileset_id} AND StartTime>'{}' \
                 ORDER BY StartTime DESC LIMIT 1", out.stime);
            if let Ok(Some(_)) = db.row(&failed) {
                // The original upgrades to the level it already has (JobLevel).
                out.messages.push(format!("Prior failed job found in catalog. Upgrading to {}.\n", level_to_string(level)));
                out.since = upgraded(level);
                out.pool_updated = true;
                return out;
            }
        }
        out.since = format!(", since={}", out.stime);
    }
    out
}

/// backup.cc DoNativeBackupInit(): GetOrCreatePoolRecord (lookup).
pub(crate) fn pool_id(cat: &CatalogHandle, pool: &str) -> i64 {
    first_row(cat, &format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(pool))).ok().flatten()
        .map(|r| cell(&r, 0).parse().unwrap_or(0)).unwrap_or(0)
}

/// Catalog ids of the job's Job record (jcr->dir_impl->jr).
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct JobIds {
    pub level: char,
    pub client_id: i64,
    pub pool_id: i64,
    pub fileset_id: i64,
}

/// sql_update.cc UpdateJobStartRecord().
pub(crate) fn update_job_start_record_sql(job_id: u32, status: char, ids: &JobIds, start_time: i64,
                                          vol_session_id: u32, vol_session_time: u32) -> String {
    format!("UPDATE Job SET JobStatus='{status}',Level='{}',StartTime='{}',ClientId={},JobTDate={start_time},PoolId={},\
             FileSetId={},VolSessionId={vol_session_id}, VolSessionTime={vol_session_time} WHERE JobId={job_id}",
        ids.level, crate::job_messages::bstrutime(start_time), ids.client_id, ids.pool_id, ids.fileset_id)
}

/// What the backup worker reports at the end (backup.cc
/// WaitForJobTermination: the FD's "2800 End Job" and the SD's
/// "3099 Job end" counters).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BackupEnd {
    pub ids: JobIds,
    pub fd_status: char,
    pub read_bytes: u64,
    pub sd_status: char,
    pub sd_files: u64,
    pub sd_bytes: u64,
    pub sd_errors: u32,
    pub vss: bool,
    pub encrypt: bool,
    /// FDSecureEraseCmd / SDSecureEraseCmd were asked for (else the report
    /// shows "(null)"). S-27.
    #[serde(default)]
    pub fd_erase_asked: bool,
    #[serde(default)]
    pub sd_erase_asked: bool,
}

/// job.cc UpdateJobEndRecord() / sql_update.cc UpdateJobEndRecord().
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_job_end_record_sql(job_id: u32, status: char, end_time: i64, files: i64, bytes: i64, errors: i32,
                                        vol_session_id: i32, vol_session_time: i32, end: &BackupEnd) -> String {
    let t = crate::job_messages::bstrutime(end_time);
    format!("UPDATE Job SET JobStatus='{status}',Level='{}',EndTime='{t}',ClientId={},JobBytes={},ReadBytes={},\
             JobFiles={},JobErrors={},VolSessionId={},VolSessionTime={},PoolId={},FileSetId={},JobTDate={end_time},\
             RealEndTime='{t}',PriorJobId=0,HasBase=0,PurgedFiles=0 WHERE JobId={job_id}",
        end.ids.level, end.ids.client_id, bytes.max(0), end.read_bytes, files.max(0) as u32, errors.max(0) as u32,
        vol_session_id as u32, vol_session_time as u32, end.ids.pool_id, end.ids.fileset_id)
}

/// lib/edit.cc speed_to_uint64() (CFG_TYPE_SPEED): a number with an
/// optional `k/s`, `kb/s`, `m/s` or `mb/s` (bytes per second).
pub(crate) fn speed_value(text: &str) -> Option<u64> {
    let text = text.trim();
    let end = text.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(text.len());
    let number: f64 = text[..end].parse().ok()?;
    let mult = match text[end..].trim().to_ascii_lowercase().as_str() {
        "" | "*" => 1.0,
        "k/s" => 1024.0,
        "kb/s" => 1000.0,
        "m/s" => 1_048_576.0,
        "mb/s" => 1_000_000.0,
        _ => return None,
    };
    Some((number * mult) as u64)
}

/// backup.cc DoNativeBackup(): the Job's Maximum Bandwidth, else the
/// Client's Maximum Bandwidth Per Job (0: no limit).
pub(crate) fn max_bandwidth(job_dirs: &[Directive], client_dirs: &[Directive]) -> u64 {
    let speed = |dirs: &[Directive], key: &str| dirs.iter().rev().find(|d| d.norm_key() == key)
        .and_then(|d| speed_value(&d.values().join(" "))).unwrap_or(0);
    match speed(job_dirs, "maximumbandwidth") {
        0 => speed(client_dirs, "maximumbandwidthperjob"),
        job => job,
    }
}

/// fd_cmds.cc SendPluginOptions(): "pluginoptions %s" for the run's plugin
/// options and every FD Plugin Options of the Job (spaces bashed).
pub(crate) fn plugin_option_lines(run_options: Option<&str>, job_dirs: &[Directive]) -> Vec<String> {
    let bash = crate::catreq::bash_spaces;
    let mut out: Vec<String> = run_options.map(|o| format!("pluginoptions {}\n", bash(o))).into_iter().collect();
    for d in job_dirs.iter().filter(|d| d.norm_key() == "fdpluginoptions") {
        for value in d.values() {
            out.push(format!("pluginoptions {}\n", bash(value)));
        }
    }
    out
}

/// PQunescapeBytea() of a bytea column in text form (`\x…` hex or escape).
fn unescape_bytea(text: &str) -> Vec<u8> {
    if let Some(hexed) = text.strip_prefix("\\x") {
        return hex::decode(hexed).unwrap_or_default();
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
            out.push(b'\\');
            i += 2;
        } else if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push(((bytes[i + 1] - b'0') << 6) | ((bytes[i + 2] - b'0') << 3) | (bytes[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// fd_cmds.cc SendPreviousRestoreObjects() → SendRestoreObjects(JobId of
/// the previous job, send_global=false): the messages for the File daemon,
/// FT_RESTORE_FIRST (25) then FT_PLUGIN_CONFIG (27) objects, each as
/// "restoreobject JobId=…" header, object name and object; empty without
/// objects (then "restoreobject end" is not sent either).
pub(crate) fn previous_restore_objects(cat: &CatalogHandle, prev_job: &str) -> Vec<Vec<u8>> {
    if prev_job.is_empty() {
        return Vec::new();
    }
    let Some(jobid) = first_row(cat, &format!("SELECT JobId FROM Job WHERE Job='{}'", esc(prev_job))).ok().flatten()
        .map(|r| cell(&r, 0)).filter(|id| !id.is_empty()) else { return Vec::new() };
    let mut out = Vec::new();
    for object_type in [25, 27] {
        let Ok(table) = cat.query_rows(&format!(
            "SELECT JobId, ObjectLength, ObjectFullLength, ObjectIndex, ObjectType, ObjectCompression, FileIndex, \
             ObjectName, RestoreObject, PluginName FROM RestoreObject WHERE JobId IN ({jobid}) AND ObjectType = {object_type} \
             ORDER BY ObjectIndex ASC")) else { continue };
        for row in table.rows {
            let f = |i: usize| cell(&row, i);
            out.push(format!("restoreobject JobId={} {},{},{},{},{},{},{}\n", f(0), f(1), f(2), f(3), f(4), f(5), f(6),
                crate::catreq::bash_spaces(&f(9))).into_bytes());
            out.push(f(7).into_bytes());
            out.push(unescape_bytea(&f(8)));
        }
    }
    out
}

/// backup.cc IsChecksumNeededByFileset() of a non-base job: an MD5/SHA1
/// signature in an Options block with Accurate (`C`) of a non-Full job.
pub(crate) fn checksum_needed(opts: &[String], level: char) -> bool {
    for opts in opts {
        let mut in_block = false;
        for c in opts.chars() {
            match c {
                'V' | ':' => in_block = false,
                'C' => in_block = level != 'F',
                '5' | '1' if in_block => return true,
                _ => {}
            }
        }
    }
    false
}

/// backup.cc SendAccurateCurrentFiles() in the Director: AccurateGetJobids()
/// up to the job's StartTime, the estimate ("SELECT sum(JobFiles) …") and
/// GetFileList() written to `path` as length-prefixed FD messages
/// ("path\0LStat\0MD5\0DeltaSeq"). Ok((estimate, sent)); Err: the M_FATAL text.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_accurate_list(cat: &CatalogHandle, job_id: u32, client_id: i64, fileset_id: i64, level: char,
                                  start_time: i64, use_md5: bool, path: &str) -> Result<(String, u64), String> {
    let date = crate::job_messages::bstrutime(start_time + 1);
    let jobids = crate::commands::bvfs::accurate_get_jobids_job(cat, job_id, client_id, 'B', &date, fileset_id,
        level == 'I', 0).unwrap_or_default();
    if jobids.is_empty() {
        return Err("Cannot find previous jobids.\n".into());
    }
    let jobids = jobids.join(",");
    let estimate = first_row(cat, &format!("SELECT sum(JobFiles) FROM Job WHERE JobId IN ({jobids})"))?
        .map(|r| cell(&r, 0)).unwrap_or_default();
    let files = cat.query_rows(&format!(
        "SELECT Path.Path, T1.Name, T1.FileIndex, T1.JobId, LStat, DeltaSeq, MD5, Fhinfo, Fhnode FROM ( \
         SELECT DISTINCT ON (Name, PathId) JobTDate, JobId, FileId, FileIndex, PathId, FileName AS Name, \
         LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM (SELECT FileId, JobId, PathId, Name AS FileName, FileIndex, LStat, MD5, \
         DeltaSeq, Fhinfo, Fhnode FROM File WHERE JobId IN ({jobids}) UNION ALL SELECT File.FileId, File.JobId, PathId, \
         File.Name AS FileName, File.FileIndex, LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM BaseFiles JOIN File USING (FileId) \
         WHERE BaseFiles.JobId IN ({jobids}) ) AS T JOIN Job USING (JobId) ORDER BY Name, PathId, JobTDate DESC \
         ) AS T1 JOIN Path ON (Path.PathId = T1.PathId) WHERE FileIndex > 0 ORDER BY T1.JobTDate, FileIndex ASC"))?;
    let mut out = Vec::new();
    let mut sent = 0u64;
    for row in files.rows {
        let field = |i: usize| cell(&row, i);
        // AccurateListHandler(): with a checksum unless it is "0".
        let md5 = field(6);
        let checksum = if use_md5 && md5.len() > 1 { md5 } else { String::new() };
        let mut message = format!("{}{}", field(0), field(1)).into_bytes();
        for part in [field(4), checksum, field(5)] {
            message.push(0);
            message.extend_from_slice(part.as_bytes());
        }
        out.extend_from_slice(&(message.len() as u32).to_be_bytes());
        out.extend_from_slice(&message);
        sent += 1;
    }
    std::fs::write(path, out).map_err(|e| format!("Cannot write accurate list {path}: {e}\n"))?;
    Ok((estimate, sent))
}

/// sql_get.cc GetJobVolumeParameters() + backup.cc UpdateBootstrapFile()
/// text for a backup job: the header and one record per JobMedia; the
/// M_ERROR text "Could not get Job Volume Parameters …" without volumes.
pub(crate) fn bootstrap_text(cat: &CatalogHandle, job_id: u32, header_job: &str, level_since: &str,
                             vol_session_id: u32, vol_session_time: u32, now: i64) -> (String, Option<String>) {
    let rows = cat.query_rows(&format!(
        "SELECT VolumeName,MediaType,FirstIndex,LastIndex,StartFile,JobMedia.EndFile,StartBlock,JobMedia.EndBlock,Slot \
         FROM JobMedia,Media WHERE JobMedia.JobId={job_id} AND JobMedia.MediaId=Media.MediaId ORDER BY VolIndex,JobMediaId"))
        .map(|t| t.rows).unwrap_or_default();
    let error = rows.is_empty().then(|| format!(
        "Could not get Job Volume Parameters to update Bootstrap file. ERR=sql_get.cc:414 No volumes found for JobId={job_id}\n"));
    let mut text = format!("# {} - {header_job} - {level_since}\n", crate::job_messages::bstrftimes(now));
    for row in rows {
        let value = |i: usize| cell(&row, i);
        let n = |i: usize| value(i).parse::<u64>().unwrap_or(0);
        text.push_str(&format!("Volume=\"{}\"\nMediaType=\"{}\"\n", value(0), value(1)));
        if n(8) > 0 {
            text.push_str(&format!("Slot={}\n", n(8)));
        }
        let start = (n(4) as u32 as u64) << 32 | n(6) as u32 as u64;
        let end = (n(5) as u32 as u64) << 32 | n(7) as u32 as u64;
        text.push_str(&format!("VolSessionId={vol_session_id}\nVolSessionTime={vol_session_time}\nVolAddr={start}-{end}\nFileIndex={}-{}\n",
            n(2) as i32, n(3) as i32));
    }
    (text, error)
}

/// The quota of a client (dird/quota.cc): its Client resource values, the
/// Quota record (GraceTime, QuotaLimit) and jr.JobSumTotalBytes once known.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Quota {
    pub client_id: i64,
    pub hard: u64,
    pub soft: u64,
    pub grace_period: u64,
    pub strict: bool,
    pub include_failed: bool,
    pub job_retention: u64,
    pub grace_time: u64,
    pub quota_limit: u64,
    pub has_quota: bool,
    pub sum: u64,
}

impl Quota {
    /// job.cc GetOrCreateClientRecord(): with a Soft or Hard Quota the Quota
    /// record is read (or created, then 0/0). None without quotas.
    pub(crate) fn setup(cat: &CatalogHandle, client_dirs: &[Directive], client_id: i64, job_retention: u64,
                        messages: &mut Vec<(char, String)>) -> Option<Self> {
        let size = |key: &str| crate::commands::defaults::number(client_dirs, key, 0, crate::commands::defaults::SIZES)
            .unwrap_or(0);
        let mut quota = Quota {
            client_id, hard: size("hardquota"), soft: size("softquota"),
            grace_period: time_directive(client_dirs, "softquotagraceperiod"),
            strict: bool_directive(client_dirs, "strictquotas", false),
            include_failed: bool_directive(client_dirs, "quotaincludefailedjobs", true),
            job_retention, ..Default::default()
        };
        if quota.hard == 0 && quota.soft == 0 {
            return None;
        }
        match first_row(cat, &format!("SELECT GraceTime, QuotaLimit FROM Quota WHERE ClientId = {client_id}")) {
            Ok(Some(row)) => {
                quota.grace_time = cell(&row, 0).parse().unwrap_or(0);
                quota.quota_limit = cell(&row, 1).parse::<i64>().unwrap_or(0).max(0) as u64;
            }
            _ => {
                let exists = first_row(cat, &format!("SELECT ClientId FROM Quota WHERE ClientId='{client_id}'")).ok().flatten();
                if exists.is_none() {
                    if let Err(e) = cat.execute(&format!(
                        "INSERT INTO Quota (ClientId, GraceTime, QuotaLimit) VALUES ('{client_id}', '0', 0)")) {
                        messages.push(('f', format!("Could not create Quota record. ERR={e}\n")));
                    }
                }
            }
        }
        Some(quota)
    }

    /// get_quota_jobbytes[_nofailed](): jr.JobSumTotalBytes once per job.
    fn load_sum(&mut self, cat: &CatalogHandle, job_id: u32, now: i64, messages: &mut Vec<(char, String)>) -> bool {
        if self.has_quota {
            return true;
        }
        let since = crate::job_messages::bstrutime(now - self.job_retention as i64 + 5);
        let failed = if self.include_failed { "" } else { "\n   AND JobStatus NOT IN ('E','f','A')" };
        let job = if self.include_failed { "JobId" } else { "Job.JobId" };
        let sql = format!("SELECT SUM(JobBytes)\n  FROM Job\n WHERE ClientId  =  {}\n   AND {job} != {job_id}\n   AND SchedTime >  TIMESTAMP '{since}'{failed}\n",
            self.client_id);
        match first_row(cat, &sql) {
            Ok(row) => self.sum = row.map(|r| cell(&r, 0).parse().unwrap_or(0)).unwrap_or(0),
            Err(e) => {
                messages.push(('e', format!("JobBytes sum select failed: ERR={e}\n")));
                messages.push(('w', format!("Error getting Quota value: ERR=JobBytes sum select failed: ERR={e}\n")));
                return false;
            }
        }
        self.has_quota = true;
        true
    }

    /// quota.cc CheckHardquotas(): true when the Hard Quota is exceeded.
    pub(crate) fn check_hard(&mut self, cat: &CatalogHandle, job_id: u32, now: i64, messages: &mut Vec<(char, String)>) -> bool {
        if self.hard == 0 || !self.load_sum(cat, job_id, now, messages) {
            return false;
        }
        self.sum > self.hard
    }

    /// quota.cc CheckSoftquotas(); `sd_bytes` is SDJobBytes (0 before the job).
    pub(crate) fn check_soft(&mut self, cat: &CatalogHandle, job_id: u32, now: i64, sd_bytes: u64,
                             messages: &mut Vec<(char, String)>) -> bool {
        if self.soft == 0 {
            return false;
        }
        let had_quota = self.has_quota;
        if !self.load_sum(cat, job_id, now, messages) {
            return false;
        }
        if !had_quota && !self.include_failed {
            messages.push(('i', "Quota does NOT include Failed Jobs\n".into()));
        }
        let now_u = now as u64;
        let client = self.client_id;
        if self.sum + sd_bytes > self.soft {
            if self.grace_time == 0 && self.grace_period > 0 {
                match cat.execute(&format!("UPDATE Quota SET GraceTime={now} WHERE ClientId='{client}'")) {
                    Ok(_) => messages.push(('e', "Softquota Exceeded, Grace Period starts now.\n".into())),
                    Err(e) => messages.push(('w', format!("Error setting Quota gracetime: ERR={e}\n"))),
                }
                self.grace_time = now_u;
                return false;
            } else if self.grace_period > 0 && now_u.saturating_sub(self.grace_time) < self.grace_period {
                messages.push(('e', "Softquota Exceeded, will be enforced after Grace Period expires.\n".into()));
            } else if self.grace_period > 0 && now_u.saturating_sub(self.grace_time) > self.grace_period {
                if self.quota_limit < 1 {
                    // UpdateQuotaSoftlimit(): JobSumTotalBytes + jr.JobBytes (0 here).
                    if let Err(e) = cat.execute(&format!("UPDATE Quota SET QuotaLimit={} WHERE ClientId='{client}'", self.sum)) {
                        messages.push(('w', format!("Error setting Quota Softlimit: ERR={e}\n")));
                    }
                    messages.push(('w', "Softquota Exceeded and Grace Period expired.\n".into()));
                    messages.push(('i', format!("Setting Burst Quota to {} Bytes.\n", self.sum)));
                    self.quota_limit = self.sum;
                    return true;
                } else if self.strict {
                    if self.sum > self.soft {
                        return true;
                    }
                } else if self.sum >= self.quota_limit {
                    messages.push(('w', "Soft Quota exceeded, enforcing Burst Quota Limit.\n".into()));
                    return true;
                }
            }
        } else if self.grace_time != 0 {
            match cat.execute(&format!("UPDATE Quota SET GraceTime='0', QuotaLimit='0' WHERE ClientId='{client}'")) {
                Ok(_) => {
                    self.grace_time = 0;
                    messages.push(('i', "Soft Quota reset, Grace Period ends now.\n".into()));
                }
                Err(e) => messages.push(('w', format!("Error setting Quota gracetime: ERR={e}\n"))),
            }
        }
        false
    }

    /// GenerateBackupSummary(): the quota lines of the job report.
    pub(crate) fn summary(&self, sd_bytes: u64) -> String {
        use crate::job_messages::{commas, suffix};
        let grace = if self.grace_time != 0 {
            crate::job_messages::bstrftimes((self.grace_time + self.grace_period) as i64)
        } else {
            "Soft Quota not exceeded".to_string()
        };
        let used = self.sum + sd_bytes;
        format!("  Quota Used:             {} ({}B)\n  Burst Quota:            {} ({}B)\n  Soft Quota:             {} ({}B)\n\
                 \x20 Hard Quota:             {} ({}B)\n  Grace Expiry Date:      {grace}\n",
            commas(used), suffix(used), commas(self.quota_limit), suffix(self.quota_limit), commas(self.soft),
            suffix(self.soft), commas(self.hard), suffix(self.hard))
    }

    /// quota.cc FetchRemainingQuotas(): the SD Job command's Quota=.
    pub(crate) fn remaining(&self, now: i64) -> u64 {
        if !self.has_quota {
            return 0;
        }
        let now = now as u64;
        let grace_over = self.soft > 0 && self.grace_time > 0 && self.grace_period > 0
            && now.saturating_sub(self.grace_time) > self.grace_period;
        if self.strict && grace_over {
            self.soft.wrapping_sub(self.sum)
        } else if !self.strict && grace_over {
            if self.quota_limit > self.soft { self.quota_limit.wrapping_sub(self.sum) } else { self.soft.wrapping_sub(self.sum) }
        } else if self.sum < self.hard {
            self.hard - self.sum
        } else {
            0
        }
    }
}

/// Message type of a quota message code ('f' fatal, 'e' error, 'w' warning, 'i' info).
pub(crate) fn quota_msg_type(code: char) -> crate::message_bus::MsgType {
    use crate::message_bus::MsgType;
    match code {
        'f' => MsgType::Fatal,
        'e' => MsgType::Error,
        'w' => MsgType::Warning,
        _ => MsgType::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaining_quota_follows_fetch_remaining_quotas() {
        let mut q = Quota { hard: 1000, sum: 400, has_quota: true, ..Default::default() };
        assert_eq!(q.remaining(100), 600);
        q.sum = 1200;
        assert_eq!(q.remaining(100), 0);
        q.has_quota = false;
        assert_eq!(q.remaining(100), 0);
        let q = Quota { soft: 500, sum: 100, grace_time: 10, grace_period: 5, strict: true, has_quota: true, ..Default::default() };
        assert_eq!(q.remaining(100), 400);
    }

    #[test]
    fn speed_and_bytea_follow_the_original() {
        assert_eq!(speed_value("10 mb/s"), Some(10_000_000));
        assert_eq!(speed_value("2k/s"), Some(2048));
        assert_eq!(speed_value("500"), Some(500));
        assert_eq!(speed_value("5 gb/s"), None);
        assert_eq!(unescape_bytea("\\x41420a"), b"AB\n");
        assert_eq!(unescape_bytea("a\\\\b\\101"), b"a\\bA");
        assert!(checksum_needed(&["C5".into()], 'I'));
        assert!(!checksum_needed(&["C5".into()], 'F'));
        assert!(!checksum_needed(&["5".into()], 'I'));
    }

    struct Rows(Vec<(&'static str, Option<(String, String)>)>);
    impl SinceDb for Rows {
        fn row(&self, sql: &str) -> Result<Option<(String, String)>, String> {
            Ok(self.0.iter().find(|(k, _)| sql.contains(k)).and_then(|(_, v)| v.clone()))
        }
    }

    #[test]
    fn fileset_md5_hashes_file_names_like_inc_conf() {
        let cfg = bareos_config::raw::parse_str(
            "FileSet { Name = MigSet\n Include {\n Options { Signature = XXH128 }\n File = /tmp/claude-10001/b39210/data\n }\n}\n", None)
            .unwrap();
        // The original Director stored this MD5 for the same FileSet.
        assert_eq!(fileset_md5(&cfg[0]).as_deref(), Some("P6ZCi//eQ6Bi98Qr+0cFPC"));
    }

    #[test]
    fn level_since_time_follows_get_level_since_time() {
        let none = Rows(vec![]);
        let inc = get_level_since_time(&none, "j", 2, 5, 'I', LevelPolicy::default(), 100);
        assert_eq!(inc.level, 'F');
        assert_eq!(inc.since, " (upgraded from Incremental)");
        assert_eq!(inc.messages, vec!["No prior Full backup Job record found.\n".to_string(),
            "No prior or suitable Full backup found in catalog. Doing FULL backup.\n".to_string()]);
        let diff = get_level_since_time(&none, "j", 2, 5, 'D', LevelPolicy::default(), 100);
        assert!(diff.messages[0].starts_with("cats/sql_find.cc:131 No Job record found: ERR=\nCMD=SELECT StartTime, Job FROM Job WHERE"));
        let full = Some(("2026-10-17 07:20:10".to_string(), "j.2026-10-17_07.20.08_03".to_string()));
        let db = Rows(vec![("Level='F'", full.clone()), ("Level IN ('I','D','F')", full)]);
        let inc = get_level_since_time(&db, "j", 2, 5, 'I', LevelPolicy::default(), 100);
        assert_eq!((inc.level, inc.since.as_str(), inc.prev_job.as_str()),
            ('I', ", since=2026-10-17 07:20:10", "j.2026-10-17_07.20.08_03"));
        assert!(inc.messages.is_empty());
    }

    #[test]
    fn job_record_updates_follow_sql_update() {
        let ids = JobIds { level: 'I', client_id: 2, pool_id: 5, fileset_id: 7 };
        let sql = update_job_start_record_sql(631, 'R', &ids, 0, 0, 0);
        assert!(sql.starts_with("UPDATE Job SET JobStatus='R',Level='I',StartTime='"));
        assert!(sql.ends_with(",ClientId=2,JobTDate=0,PoolId=5,FileSetId=7,VolSessionId=0, VolSessionTime=0 WHERE JobId=631"));
        let end = BackupEnd { ids, read_bytes: 20029, ..Default::default() };
        let sql = update_job_end_record_sql(631, 'T', 0, 3, 20029, 0, 2, 1792221542, &end);
        assert!(sql.contains(",JobBytes=20029,ReadBytes=20029,JobFiles=3,JobErrors=0,VolSessionId=2,VolSessionTime=1792221542,PoolId=5,FileSetId=7,JobTDate=0,"));
    }
}
