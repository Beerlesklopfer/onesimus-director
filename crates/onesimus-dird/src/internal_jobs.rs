//! Jobs the Director runs itself without daemon contact (Compatibility log
//! C-058): the selection of a Copy/Migrate control job (migrate.cc
//! DoMigrationSelection/getJobs_to_migrate) and a Consolidate job without
//! Always Incremental jobs (consolidate.cc DoConsolidate). The JobManager
//! runs them when the job leaves the job queue (job_thread); the end of the
//! job (UpdateJobEnd and the report) is the JobManager's as for workers.

use crate::catalog_worker::CatalogHandle;
use crate::director_config::DirectorConfig;
use crate::message_bus::MsgType;

/// What the job does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Copy,
    Migrate,
    Consolidate,
}

impl Kind {
    /// JobControlRecord::get_OperationName().
    pub fn operation(self) -> &'static str {
        match self {
            Kind::Copy => "Copying",
            Kind::Migrate => "Migration",
            Kind::Consolidate => "Consolidating",
        }
    }

    /// JobControlRecord::get_ActionName(past).
    pub fn action(self, past: bool) -> &'static str {
        match (self, past) {
            (Kind::Copy, false) => "copy",
            (Kind::Copy, true) => "copied",
            (Kind::Migrate, false) => "migrate",
            (Kind::Migrate, true) => "migrated",
            (Kind::Consolidate, false) => "consolidate",
            (Kind::Consolidate, true) => "consolidated",
        }
    }
}

/// The result of a run: final JobStatus, the job messages in order, whether
/// the control job selected jobs (jcr->dir_impl->HasSelectedJobs), the
/// Job record's PoolId (UpdateJobStartRecord) and the jobs it started.
#[derive(Default)]
pub struct Outcome {
    pub status: char,
    pub messages: Vec<(MsgType, String)>,
    pub selected: bool,
    pub pool_id: Option<String>,
    /// Jobs started during the run (DoRunCmd of StartNewConsolidationJob /
    /// StartNewMigrationJob): their Job records exist; the JobManager
    /// queues them after the run.
    pub children: Vec<ChildStart>,
}

/// A job a Director-run job started: its catalog record exists (JobId), the
/// JobManager puts it in the job queue (JobqAdd).
pub struct ChildStart {
    pub job_id: u32,
    pub job_name: String,
    pub start: crate::job_manager::QueuedStart,
    pub report: crate::job_messages::JobReport,
    pub client: String,
    pub storage: String,
}

impl std::fmt::Debug for ChildStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildStart").field("job_id", &self.job_id).field("job_name", &self.job_name).finish()
    }
}

impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outcome").field("status", &self.status).field("messages", &self.messages)
            .field("selected", &self.selected).field("pool_id", &self.pool_id).field("children", &self.children).finish()
    }
}

/// What the run needs to know about the job.
pub struct Request<'a> {
    pub kind: Kind,
    pub job_id: u32,
    pub unique_job: &'a str,
    pub job_name: &'a str,
    /// The (read) pool of the control job.
    pub pool: &'a str,
}

fn sql_esc(text: &str) -> String {
    text.replace('\'', "''")
}

/// The values of the first column, each once, in query order (UniqueDbidHandler).
fn unique_ids(catalog: &CatalogHandle, sql: &str) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = Vec::new();
    for row in catalog.query_rows(sql)?.rows {
        let Some(Some(id)) = row.into_iter().next() else { break };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// The names of the first column, sorted with strcmp and each once (UniqueNameHandler).
fn unique_names(catalog: &CatalogHandle, sql: &str) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = catalog.query_rows(sql)?.rows.into_iter()
        .filter_map(|row| row.into_iter().next().flatten()).collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// regcomp(REG_EXTENDED) + regexec(): Err with regerror() text.
fn posix_matcher(pattern: &str) -> Result<impl Fn(&str) -> bool, String> {
    let cpattern = std::ffi::CString::new(pattern).map_err(|_| "invalid pattern".to_string())?;
    // Compiled per use: a regex_t stays where regcomp() put it.
    let compile = move |text: Option<&std::ffi::CStr>| -> Result<bool, String> {
        unsafe {
            let mut preg = std::mem::MaybeUninit::<libc::regex_t>::zeroed();
            let rc = libc::regcomp(preg.as_mut_ptr(), cpattern.as_ptr(), libc::REG_EXTENDED);
            if rc != 0 {
                let mut buf = [0 as libc::c_char; 500];
                libc::regerror(rc, preg.as_ptr(), buf.as_mut_ptr(), buf.len());
                return Err(std::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned());
            }
            let mut found = std::mem::MaybeUninit::<libc::regmatch_t>::uninit();
            let matched = text.is_some_and(|text| libc::regexec(preg.as_ptr(), text.as_ptr(), 1, found.as_mut_ptr(), 0) == 0);
            libc::regfree(preg.as_mut_ptr());
            Ok(matched)
        }
    };
    compile(None)?;
    Ok(move |text: &str| {
        std::ffi::CString::new(text).ok().is_some_and(|ctext| compile(Some(&ctext)).unwrap_or(false))
    })
}

const SQL_JOB: &str = "SELECT DISTINCT Job.Name from Job,Pool WHERE Pool.Name='%s' AND Job.PoolId=Pool.PoolId";
const SQL_JOBIDS_FROM_JOB: &str = "SELECT DISTINCT Job.JobId,Job.StartTime FROM Job,Pool WHERE Job.Name='%s' \
    AND Pool.Name='%s' AND Job.PoolId=Pool.PoolId ORDER by Job.StartTime";
const SQL_CLIENT: &str = "SELECT DISTINCT Client.Name from Client,Pool,Job WHERE Pool.Name='%s' AND \
    Job.ClientId=Client.ClientId AND Job.PoolId=Pool.PoolId";
const SQL_JOBIDS_FROM_CLIENT: &str = "SELECT DISTINCT Job.JobId,Job.StartTime FROM Job,Pool,Client WHERE \
    Client.Name='%s' AND Pool.Name='%s' AND Job.PoolId=Pool.PoolId AND Job.ClientId=Client.ClientId AND \
    Job.Type IN ('B','C') AND Job.JobStatus IN ('T','W') ORDER by Job.StartTime";
const SQL_VOL: &str = "SELECT DISTINCT VolumeName FROM Media,Pool WHERE VolStatus in ('Full','Used','Error') AND \
    Media.Enabled=1 AND Media.PoolId=Pool.PoolId AND Pool.Name='%s'";
const SQL_JOBIDS_FROM_VOL: &str = "SELECT DISTINCT Job.JobId,Job.StartTime FROM Media,JobMedia,Job WHERE \
    Media.VolumeName='%s' AND Media.MediaId=JobMedia.MediaId AND JobMedia.JobId=Job.JobId AND \
    Job.Type IN ('B','C') AND Job.JobStatus IN ('T','W') AND Media.Enabled=1 ORDER by Job.StartTime";
const SQL_SMALLEST_VOL: &str = "SELECT Media.MediaId FROM Media,Pool,JobMedia WHERE Media.MediaId in \
    (SELECT DISTINCT MediaId from JobMedia) AND Media.VolStatus in ('Full','Used','Error') AND Media.Enabled=1 AND \
    Media.PoolId=Pool.PoolId AND Pool.Name='%s' ORDER BY VolBytes ASC LIMIT 1";
const SQL_OLDEST_VOL: &str = "SELECT Media.MediaId FROM Media,Pool,JobMedia WHERE Media.MediaId in \
    (SELECT DISTINCT MediaId from JobMedia) AND Media.VolStatus in ('Full','Used','Error') AND Media.Enabled=1 AND \
    Media.PoolId=Pool.PoolId AND Pool.Name='%s' ORDER BY LastWritten ASC LIMIT 1";
const SQL_JOBIDS_FROM_MEDIAID: &str = "SELECT DISTINCT Job.JobId,Job.StartTime FROM JobMedia,Job WHERE \
    JobMedia.JobId=Job.JobId AND JobMedia.MediaId IN (%s) AND Job.Type IN ('B','C') AND Job.JobStatus IN ('T','W') \
    ORDER by Job.StartTime";
const SQL_POOL_TIME: &str = "SELECT DISTINCT Job.JobId FROM Pool,Job,Media,JobMedia WHERE Pool.Name='%s' AND \
    Media.PoolId=Pool.PoolId AND VolStatus IN ('Full','Used','Error') AND Media.Enabled=1 AND \
    Job.Type IN ('B','C') AND Job.JobStatus IN ('T','W') AND JobMedia.JobId=Job.JobId AND \
    Job.PoolId=Media.PoolId AND Job.RealEndTime<='%s'";
const SQL_JOBIDS_OF_POOL_UNCOPIED_JOBS: &str = "SELECT DISTINCT Job.JobId,Job.StartTime FROM Job,Pool WHERE \
    Pool.Name = '%s' AND Pool.PoolId = Job.PoolId AND Job.Type IN ('B','A') AND Job.JobStatus IN ('T','W') AND \
    Job.jobBytes > 0 AND Job.JobId NOT IN (SELECT PriorJobId FROM Job WHERE Type IN ('B','C') AND \
    Job.JobStatus IN ('T','W') AND PriorJobId != 0) ORDER by Job.StartTime";

/// Mmsg() with %s arguments in order.
fn fill(template: &str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = template;
    for arg in args {
        let Some(at) = rest.find("%s") else { break };
        out.push_str(&rest[..at]);
        out.push_str(arg);
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// Run the job: Ok outcome (its status is the job's), Err for a run that
/// could not even begin.
pub fn run(config: &DirectorConfig, catalog: Option<&CatalogHandle>, request: &Request) -> Outcome {
    let mut out = Outcome { status: 'T', ..Default::default() };
    match request.kind {
        Kind::Consolidate => {
            // DoConsolidate(): the start message, then ConsolidateJobs() over
            // the Always Incremental jobs (refused before the job was queued).
            out.messages.push((MsgType::Info,
                format!("Start Consolidate JobId {}, Job={}\n", request.job_id, request.unique_job)));
        }
        Kind::Copy | Kind::Migrate => {
            let Some(catalog) = catalog else {
                out.messages.push((MsgType::Fatal, "Could not open database \"bareos\".\n".into()));
                out.status = 'f';
                return out;
            };
            // DoMigrationInit(): the Pool record of the (read) pool.
            match unique_ids(catalog, &format!("SELECT PoolId FROM Pool WHERE Name='{}'", sql_esc(request.pool))) {
                Ok(ids) if !ids.is_empty() => out.pool_id = ids.into_iter().next(),
                _ => {
                    out.messages.push((MsgType::Fatal, "Could not get or create a Pool record.\n".into()));
                    out.status = 'f';
                    return out;
                }
            }
            // DoMigrationSelection()
            if select_jobs(config, catalog, request, &mut out) {
                out.status = 'T';
            } else {
                out.status = 'E';
            }
        }
    }
    out
}

/// getJobs_to_migrate(): false on error.
fn select_jobs(config: &DirectorConfig, catalog: &CatalogHandle, request: &Request, out: &mut Outcome) -> bool {
    let kind = request.kind;
    let dirs = crate::commands::defaults::job_directives(config, request.job_name);
    let value = |key: &str| dirs.iter().rev().find(|d| d.norm_key() == key).and_then(|d| d.first()).map(str::to_string);
    let selection_type = value("selectiontype").unwrap_or_default().to_ascii_lowercase();
    let pattern = value("selectionpattern");
    let pool = sql_esc(request.pool);
    let fatal = |out: &mut Outcome, text: String| out.messages.push((MsgType::Fatal, text));
    let info = |out: &mut Outcome, text: String| out.messages.push((MsgType::Info, text));

    let ids: Vec<String> = match selection_type.as_str() {
        "job" | "client" | "volume" => {
            let (query1, query2, type_name) = match selection_type.as_str() {
                "job" => (SQL_JOB, SQL_JOBIDS_FROM_JOB, "Job"),
                "client" => (SQL_CLIENT, SQL_JOBIDS_FROM_CLIENT, "Client"),
                _ => (SQL_VOL, SQL_JOBIDS_FROM_VOL, "Volume"),
            };
            // regex_find_jobids()
            let Some(pattern) = pattern else {
                fatal(out, format!("No {} {type_name} selection pattern specified.\n", kind.operation()));
                return false;
            };
            let names = match unique_names(catalog, &fill(query1, &[&pool])) {
                Ok(names) => names,
                Err(e) => {
                    fatal(out, format!("SQL to get {type_name} failed. ERR={e}\n"));
                    return false;
                }
            };
            if names.is_empty() {
                info(out, format!("Query of Pool \"{}\" returned no Jobs to {}.\n", request.pool, kind.action(false)));
                Vec::new()
            } else {
                let matcher = match posix_matcher(&pattern) {
                    Ok(m) => m,
                    Err(e) => {
                        fatal(out, format!("Could not compile regex pattern \"{pattern}\" ERR={e}\n"));
                        return false;
                    }
                };
                let names: Vec<String> = names.into_iter().filter(|n| matcher(n)).collect();
                if names.is_empty() {
                    info(out, format!("Regex pattern matched no Jobs to {}.\n", kind.action(false)));
                    Vec::new()
                } else {
                    let mut ids: Vec<String> = Vec::new();
                    for name in names {
                        match unique_ids(catalog, &fill(query2, &[&sql_esc(&name), &pool])) {
                            Ok(found) => {
                                for id in found {
                                    if !ids.contains(&id) {
                                        ids.push(id);
                                    }
                                }
                            }
                            Err(e) => {
                                fatal(out, format!("SQL failed. ERR={e}\n"));
                                return false;
                            }
                        }
                    }
                    if ids.is_empty() {
                        info(out, format!("No {type_name}s found to {}.\n", kind.action(false)));
                    }
                    ids
                }
            }
        }
        "sqlquery" => {
            let Some(pattern) = pattern else {
                fatal(out, format!("No {} SQL selection pattern specified.\n", kind.operation()));
                return false;
            };
            match unique_ids(catalog, &pattern) {
                Ok(ids) => ids,
                Err(e) => {
                    fatal(out, format!("SQL failed. ERR={e}\n"));
                    return false;
                }
            }
        }
        "smallestvolume" | "oldestvolume" => {
            let (query, type_name) = if selection_type == "smallestvolume" {
                (SQL_SMALLEST_VOL, "Smallest Volume")
            } else {
                (SQL_OLDEST_VOL, "Oldest Volume")
            };
            // find_mediaid_then_jobids()
            let media = match unique_ids(catalog, &fill(query, &[&pool])) {
                Ok(ids) => ids,
                Err(e) => {
                    fatal(out, format!("SQL failed. ERR={e}\n"));
                    return false;
                }
            };
            if media.is_empty() {
                info(out, format!("No {type_name} found to {}.\n", kind.action(false)));
                Vec::new()
            } else if media.len() != 1 {
                fatal(out, format!("SQL error. Expected 1 MediaId got {}\n", media.len()));
                return false;
            } else {
                match unique_ids(catalog, &fill(SQL_JOBIDS_FROM_MEDIAID, &[&media.join(",")])) {
                    Ok(ids) => {
                        if ids.is_empty() {
                            info(out, format!("No {type_name}s found to {}.\n", kind.action(false)));
                        }
                        ids
                    }
                    Err(e) => {
                        fatal(out, format!("SQL failed. ERR={e}\n"));
                        return false;
                    }
                }
            }
        }
        "pooltime" => {
            let pool_dirs: Vec<bareos_config::raw::Directive> = config.raw.iter()
                .find(|r| r.norm_type() == "pool" && r.name().is_some_and(|n| n.eq_ignore_ascii_case(request.pool)))
                .map(|r| r.directives.clone()).unwrap_or_default();
            let migration_time = crate::commands::defaults::time_directive(&pool_dirs, "migrationtime") as i64;
            let now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
            let dt = crate::localtime::strftime("%Y-%m-%d %H:%M:%S", (now - migration_time) as libc::time_t);
            match unique_ids(catalog, &fill(SQL_POOL_TIME, &[&pool, &dt])) {
                Ok(ids) if ids.is_empty() => {
                    info(out, format!("No Volumes found to {}.\n", kind.action(false)));
                    return true;
                }
                Ok(ids) => ids,
                Err(e) => {
                    fatal(out, format!("SQL failed. ERR={e}\n"));
                    return false;
                }
            }
        }
        "pooluncopiedjobs" => {
            // FindJobidsOfPoolUncopiedJobs()
            if kind != Kind::Copy {
                fatal(out, "Selection Type 'pooluncopiedjobs' only applies to Copy Jobs\n".into());
                return false;
            }
            match unique_ids(catalog, &fill(SQL_JOBIDS_OF_POOL_UNCOPIED_JOBS, &[&pool])) {
                Ok(ids) => ids,
                Err(e) => {
                    fatal(out, format!("SQL to get uncopied jobs failed. ERR={e}\n"));
                    return false;
                }
            }
        }
        "pooloccupancy" => {
            // Needs the Pool's Migration High/Low Bytes accounting (open, C-058).
            fatal(out, format!("{} Selection Type PoolOccupancy is not available yet.\n", kind.operation()));
            return false;
        }
        _ => {
            fatal(out, format!("Unknown {} Selection Type.\n", kind.operation()));
            return false;
        }
    };

    if ids.is_empty() {
        info(out, format!("No JobIds found to {}.\n", kind.action(false)));
        return true;
    }
    info(out, format!("The following {} JobId{} chosen to be {}: {}\n", ids.len(),
        if ids.len() < 2 { " was" } else { "s were" }, kind.action(true), ids.join(",")));
    // StartNewMigrationJob() per JobId (Maximum Concurrent Copies). The
    // worker jobs that read and write the volumes are not available in
    // Onesimus yet (C-058): each start fails like a refused DoRunCmd().
    let limit = value("maxconcurrentcopies").and_then(|v| v.parse::<usize>().ok()).filter(|l| *l > 0);
    for _ in ids.iter().take(limit.unwrap_or(usize::MAX)) {
        out.messages.push((MsgType::Error, "Could not start migration job.\n".into()));
    }
    out.selected = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
    use std::sync::{Arc, Mutex};

    fn catalog(answers: Vec<(&'static str, Vec<&'static str>)>, log: Arc<Mutex<Vec<String>>>) -> CatalogHandle {
        CatalogHandle::mock(move |request| match request {
            CatalogRequest::TextRows { sql } => {
                log.lock().unwrap().push(sql.clone());
                let rows = answers.iter().find(|(prefix, _)| sql.starts_with(prefix))
                    .map(|(_, rows)| rows.iter().map(|r| r.split('|').map(|v| Some(v.to_string())).collect()).collect())
                    .unwrap_or_default();
                CatalogResponse::Rows(SqlTable { columns: vec![SqlColumn { name: "x".into(), numeric: false }], rows })
            }
            _ => CatalogResponse::Error("double".into()),
        })
    }

    fn config(job: &str) -> DirectorConfig {
        DirectorConfig::from_str(&format!("Director {{ Name = dir }}\nPool {{ Name = Full; Pool Type = Backup }}\n{job}"))
            .unwrap()
    }

    fn request<'a>(kind: Kind, name: &'a str) -> Request<'a> {
        Request { kind, job_id: 9, unique_job: "m.2026-10-16_10.00.00_01", job_name: name, pool: "Full" }
    }

    #[test]
    fn job_selection_without_matching_names() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let cat = catalog(vec![("SELECT PoolId", vec!["3"]), ("SELECT DISTINCT Job.Name", vec!["b", "a", "b"])], log.clone());
        let cfg = config("Job { Name = m; Type = Migrate; Selection Type = Job; Selection Pattern = \"nomatch\" }");
        let out = run(&cfg, Some(&cat), &request(Kind::Migrate, "m"));
        assert_eq!(out.status, 'T');
        assert_eq!(out.pool_id.as_deref(), Some("3"));
        assert!(!out.selected);
        assert_eq!(out.messages, vec![(MsgType::Info, "Regex pattern matched no Jobs to migrate.\n".to_string()),
            (MsgType::Info, "No JobIds found to migrate.\n".to_string())]);
        assert_eq!(log.lock().unwrap()[1], "SELECT DISTINCT Job.Name from Job,Pool WHERE Pool.Name='Full' AND Job.PoolId=Pool.PoolId");
    }

    #[test]
    fn job_selection_with_matches_and_children_not_available() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let cat = catalog(vec![("SELECT PoolId", vec!["3"]), ("SELECT DISTINCT Job.Name", vec!["backup-a", "other"]),
            ("SELECT DISTINCT Job.JobId", vec!["5|t", "7|t", "5|t"])], log.clone());
        let cfg = config("Job { Name = c; Type = Copy; Selection Type = Job; Selection Pattern = \"^backup\" }");
        let out = run(&cfg, Some(&cat), &request(Kind::Copy, "c"));
        assert_eq!(out.status, 'T');
        assert!(out.selected, "{:?} {:?}", out.messages, log.lock().unwrap());
        assert_eq!(out.messages[0], (MsgType::Info, "The following 2 JobIds were chosen to be copied: 5,7\n".to_string()));
        assert_eq!(out.messages.len(), 3);
        assert!(log.lock().unwrap()[2].starts_with("SELECT DISTINCT Job.JobId,Job.StartTime FROM Job,Pool WHERE Job.Name='backup-a'"));
    }

    #[test]
    fn posix_extended_regex() {
        let matcher = posix_matcher("^backup").unwrap();
        assert!(matcher("backup-a") && !matcher("other"));
        assert!(posix_matcher("(").is_err());
    }

    #[test]
    fn selection_errors() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let cat = catalog(vec![("SELECT PoolId", vec!["3"]), ("SELECT DISTINCT Job.Name", vec!["a"])], log);
        for (job, kind, text) in [
            ("Job { Name = m; Type = Migrate }", Kind::Migrate, "Unknown Migration Selection Type.\n"),
            ("Job { Name = m; Type = Migrate; Selection Type = Job }", Kind::Migrate,
                "No Migration Job selection pattern specified.\n"),
            ("Job { Name = m; Type = Migrate; Selection Type = PoolUncopiedJobs }", Kind::Migrate,
                "Selection Type 'pooluncopiedjobs' only applies to Copy Jobs\n"),
            ("Job { Name = m; Type = Copy; Selection Type = Job; Selection Pattern = \"(\" }", Kind::Copy,
                "Could not compile regex pattern \"(\" ERR="),
        ] {
            let out = run(&config(job), Some(&cat), &request(kind, "m"));
            assert_eq!(out.status, 'E', "{job}");
            assert_eq!(out.messages[0].0, MsgType::Fatal, "{job}");
            assert!(out.messages[0].1.starts_with(text), "{job}: {:?}", out.messages);
        }
        // No Pool record.
        let none = catalog(vec![], Arc::new(Mutex::new(Vec::new())));
        let out = run(&config("Job { Name = m; Type = Migrate }"), Some(&none), &request(Kind::Migrate, "m"));
        assert_eq!((out.status, out.messages[0].1.as_str()), ('f', "Could not get or create a Pool record.\n"));
    }

    #[test]
    fn consolidate_reports_its_start() {
        let out = run(&config(""), None, &request(Kind::Consolidate, "c"));
        assert_eq!(out.status, 'T');
        assert_eq!(out.messages, vec![(MsgType::Info, "Start Consolidate JobId 9, Job=m.2026-10-16_10.00.00_01\n".to_string())]);
    }
}
