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
                Err(e) => return Err(format!("Query error for start time request: ERR={e}\nCMD={full_query}\n")),
                Ok(None) => return Err("No prior Full backup Job record found.\n".into()),
                Ok(Some(_)) => format!(
                    "SELECT StartTime, Job FROM Job WHERE JobStatus IN ('T','W') AND Type='B' AND Level IN ('I','D','F') \
                     AND Name='{name}' AND ClientId={client_id} AND FileSetId={fileset_id} ORDER BY StartTime DESC LIMIT 1"),
            }
        } else {
            full_query.clone()
        };
        match db.row(&cmd) {
            Err(e) => Err(format!("Query error for start time request: ERR={e}\nCMD={cmd}\n")),
            Ok(None) => Err(format!("No Job record found: ERR=\nCMD={cmd}\n")),
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(diff.messages[0].starts_with("No Job record found: ERR=\nCMD=SELECT StartTime, Job FROM Job WHERE"));
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
