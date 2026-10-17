//! Copy/Migrate worker jobs (Compatibility log C-058, S-05): the part of
//! dird/migrate.cc after the selection — StartNewMigrationJob() (the
//! `run job=<Copy|Migrate> jobid=N` of the control job), DoMigrationInit()
//! for a worker job (previous job, bootstrap, the new job's record "mig_jcr",
//! read/write pools and storages), the run-time checks of DoActualMigration()
//! and MigrationCleanup()/GenerateMigrateSummary() at the job end. The
//! Storage daemon session itself runs in the forked worker (sd_copy.rs).
//! Only the local replicate (read and write storage on the same Storage
//! daemon) is available; SD-SD replication ends the job with a fatal error.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::catalog_worker::CatalogHandle;
use crate::director_config::DirectorConfig;
use crate::internal_jobs::Kind;
use crate::job_messages::{self, ReportKind};
use crate::message_bus::MsgType;

pub type JobMsg = (MsgType, String);

/// The previous job (jcr->dir_impl->previous_jr).
#[derive(Debug, Clone, Default)]
pub struct PrevJob {
    pub job_id: u32,
    pub job: String,
    pub name: String,
    pub job_type: char,
    pub job_files: u64,
    pub start_time: String,
    pub end_time: String,
    pub job_tdate: String,
    pub fileset_id: String,
    pub client_id: String,
    pub purged_files: String,
    pub has_base: String,
}

/// What MigrationCleanup() needs of a worker job (its jcr and mig_jcr).
#[derive(Debug, Clone, Default)]
pub struct WorkerInfo {
    pub copy: bool,
    pub job: String,
    pub job_resource: String,
    pub prev: PrevJob,
    /// mig_jcr->JobId: the new backup job (None: no mig_jcr).
    pub new_job_id: Option<u32>,
    pub level: char,
    pub client: String,
    pub fileset: String,
    pub read_pool: (String, String),
    pub read_storage: (String, String),
    pub write_pool: (String, String),
    pub write_storage: (String, String),
    pub next_pool: (String, String),
    pub catalog: (String, String),
    pub priority: i32,
    pub trigger: String,
    pub next_pool_id: String,
    pub purge_migrate_job: bool,
    /// DoActualMigration() ended the job JS_Terminated before the SD
    /// ("already copied", same read and write storage).
    pub terminated_early: bool,
}

static WORKERS: Mutex<Option<HashMap<u32, WorkerInfo>>> = Mutex::new(None);

fn workers() -> std::sync::MutexGuard<'static, Option<HashMap<u32, WorkerInfo>>> {
    WORKERS.lock().unwrap_or_else(|e| e.into_inner())
}

/// mig_jcr->JobId of a running worker job: the catalog requests of the
/// Storage daemon write JobMedia and File records for it (catreq.cc:352/542).
pub fn new_job_of(worker: u32) -> Option<u32> {
    workers().as_ref()?.get(&worker)?.new_job_id
}

/// A catalog number for SQL (0 when empty).
fn num(text: &str) -> &str {
    if text.is_empty() { "0" } else { text }
}

fn esc(text: &str) -> String {
    text.replace('\'', "''")
}

fn first_row(catalog: &CatalogHandle, sql: &str) -> Result<Option<Vec<String>>, String> {
    Ok(catalog.query_rows(sql)?.rows.into_iter().next().map(|r| r.into_iter().map(Option::unwrap_or_default).collect()))
}

fn raw(config: &DirectorConfig, kind: &str, name: &str, key: &str) -> Option<String> {
    job_messages::raw_directive(config, kind, name, key).filter(|v| !v.is_empty())
}

fn job_value(config: &DirectorConfig, job: &str, key: &str) -> Option<String> {
    crate::commands::defaults::job_directives(config, job).iter().rev().find(|d| d.norm_key() == key)
        .and_then(|d| d.first()).map(str::to_string).filter(|v| !v.is_empty())
}

fn level_char(level: &bareos_config::resources::JobLevel) -> char {
    use bareos_config::resources::JobLevel::*;
    match level {
        Full => 'F',
        Incremental => 'I',
        Differential => 'D',
        VirtualFull => 'f',
        _ => 'F',
    }
}

fn level_name(level: char) -> &'static str {
    match level {
        'F' => "Full",
        'I' => "Incremental",
        'D' => "Differential",
        'f' => "VirtualFull",
        'B' => "Base",
        _ => " ",
    }
}

/// GetOrCreateClientRecord(): the ClientId (0 without a client).
fn client_id(catalog: &CatalogHandle, client: &str) -> Result<String, String> {
    if client.is_empty() {
        return Ok("0".into());
    }
    if let Some(row) = first_row(catalog, &format!("SELECT ClientId FROM Client WHERE Name='{}'", esc(client)))? {
        return Ok(row[0].clone());
    }
    first_row(catalog, &format!("INSERT INTO Client (Name,Uname,AutoPrune,FileRetention,JobRetention) \
        VALUES ('{}','',0,5184000,15552000) RETURNING ClientId", esc(client)))?
        .map(|r| r[0].clone()).ok_or_else(|| "CreateClientRecord failed".into())
}

/// GetOrCreatePoolRecord(): the PoolId ("" when the pool is unknown).
fn pool_id(catalog: &CatalogHandle, pool: &str) -> String {
    first_row(catalog, &format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(pool))).ok().flatten()
        .map(|r| r[0].clone()).unwrap_or_default()
}

/// CreateJobRecord(): the Job row of a job (JobStatus 'C').
fn create_job_record(catalog: &CatalogHandle, unique: &str, name: &str, job_type: char, level: char,
                     client_id: &str, now: i64) -> Result<u32, String> {
    let sql = format!(
        "INSERT INTO Job (Job,Name,Type,Level,JobStatus,SchedTime,JobTDate,ClientId,Comment) \
         VALUES ('{}','{}','{job_type}','{level}','C','{}',{now},{client_id},'') RETURNING JobId",
        esc(unique), esc(name), job_messages::bstrutime(now));
    first_row(catalog, &sql)?.and_then(|r| r[0].parse().ok()).ok_or_else(|| "Create DB Job record failed".into())
}

/// GetJobRecord() of the previous job.
fn previous_job(catalog: &CatalogHandle, job_id: u32) -> Result<PrevJob, String> {
    let row = first_row(catalog, &format!(
        "SELECT JobId,Job,Name,Type,JobFiles,StartTime,EndTime,JobTDate,FileSetId,ClientId,PurgedFiles,HasBase \
         FROM Job WHERE JobId={job_id}"))?
        .ok_or_else(|| format!("sql_get.cc:298 No Job found for JobId {job_id}\n"))?;
    Ok(PrevJob {
        job_id, job: row[1].clone(), name: row[2].clone(), job_type: row[3].chars().next().unwrap_or(' '),
        job_files: row[4].parse().unwrap_or(0), start_time: row[5].clone(), end_time: row[6].clone(),
        job_tdate: row[7].clone(), fileset_id: row[8].clone(), client_id: row[9].clone(),
        purged_files: row[10].clone(), has_base: row[11].clone(),
    })
}

/// A worker job a run/control job started (RunJob() succeeded) or the
/// messages of a refused start (RunJob() returned 0: "Job failed.").
pub struct Started {
    pub job_id: u32,
    pub job_name: String,
    /// Job messages of the worker job posted at its setup.
    pub messages: Vec<JobMsg>,
    pub child: Option<crate::internal_jobs::ChildStart>,
}

/// What a `run job=<Copy|Migrate> jobid=N` asks for.
pub struct WorkerRequest<'a> {
    pub kind: Kind,
    pub job_name: &'a str,
    pub prev_jobid: u32,
    /// `pool=` / `nextpool=` overrides of the run command.
    pub pool: Option<String>,
    pub next_pool: Option<String>,
    pub trigger: &'a str,
}

/// RunJob() of a Copy/Migrate worker job: SetupJob() (Job record) and
/// DoMigrationInit(); the job is queued by the caller (child.start).
pub fn start_worker(config: &DirectorConfig, catalog: &CatalogHandle, req: &WorkerRequest) -> Started {
    let copy = req.kind == Kind::Copy;
    let (operation, action, action_past) = (req.kind.operation(), req.kind.action(false), req.kind.action(true));
    let mut started = Started { job_id: 0, job_name: req.job_name.to_string(), messages: Vec::new(), child: None };
    let Some(job) = config.find_job(req.job_name).cloned() else { return started };
    let now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
    let unique = crate::commands::job::unique_job_name(&job.name, now);
    let client = job.client.clone();
    let Ok(cid) = client_id(catalog, &client) else { return started };
    let level = 'F';
    let Ok(job_id) = create_job_record(catalog, &unique, &job.name, if copy { 'c' } else { 'g' }, level, &cid, now) else {
        return started;
    };
    started.job_id = job_id;
    let msgs = &mut started.messages;
    let fatal = |msgs: &mut Vec<JobMsg>, text: String| msgs.push((MsgType::Fatal, text));

    // DoMigrationInit(): the read pool.
    let read_pool = req.pool.clone().unwrap_or_else(|| job.pool.clone());
    let read_pool_source = if req.pool.as_ref().is_some_and(|p| *p != job.pool) { "Run pool override" } else { "Job resource" };
    if pool_id(catalog, &read_pool).is_empty() {
        fatal(msgs, "Could not get or create a Pool record.\n".into());
        return started;
    }
    let prev = match previous_job(catalog, req.prev_jobid) {
        Ok(prev) => prev,
        Err(e) => {
            fatal(msgs, format!("Could not get job record for JobId {} to {action}. ERR={e}", req.prev_jobid));
            return started;
        }
    };
    msgs.push((MsgType::Info, format!("{operation} using JobId={} Job={}\n", prev.job_id, prev.job)));

    // CreateRestoreBootstrapFile()
    let mut bsr = crate::commands::restore_bsr::Bsr::default();
    for fi in 1..=prev.job_files.min(i32::MAX as u64) {
        bsr.add_findex(prev.job_id, fi as i32);
    }
    if let Err(e) = bsr.add_volume_information_db(catalog) {
        msgs.push((MsgType::Info, e));
        fatal(msgs, "Create bootstrap file failed.\n".into());
        return started;
    }
    let mut texts = Vec::new();
    let (expected, fname) = bsr.write_file_for_job(config, "", &mut texts);
    msgs.extend(texts.into_iter().map(|t| (MsgType::Info, t)));
    let bootstrap = std::fs::read_to_string(&fname).unwrap_or_default();

    let Some(prev_job) = config.find_job(&prev.name).cloned() else {
        fatal(msgs, format!("Previous Job resource not found for \"{}\".\n", prev.name));
        return started;
    };
    let job_level = level_char(&prev_job.level);
    let fileset = if job.fileset.is_empty() { prev_job.fileset.clone() } else { job.fileset.clone() };

    // SetMigrationNextPool()
    let (next_pool, next_source, storage_source) = if let Some(next) = req.next_pool.clone() {
        (Some(next), "Run NextPool override", "Storage from Run NextPool override")
    } else if let Some(next) = job_value(config, &job.name, "nextpool") {
        (Some(next), "Job's NextPool resource", "Storage from Job's NextPool resource")
    } else {
        (raw(config, "pool", &read_pool, "nextpool"), "Job Pool's NextPool resource", "Storage from Pool's NextPool resource")
    };
    let Some(next_pool) = next_pool else {
        fatal(msgs, format!("No Next Pool specification found in Pool \"{read_pool}\".\n"));
        return started;
    };
    let next_pool_id = pool_id(catalog, &next_pool);
    if next_pool_id.is_empty() {
        return started;
    }
    let Some(write_store) = raw(config, "pool", &next_pool, "storage").map(|s| s.split(',').next().unwrap_or("").trim().to_string())
    else {
        fatal(msgs, format!("No Storage specification found in Next Pool \"{next_pool}\".\n"));
        return started;
    };
    // CopyRstorage(pool->storage, "Pool resource"), else the previous Job's storage.
    let (read_store, read_store_source) = match raw(config, "pool", &read_pool, "storage") {
        Some(store) => (store.split(',').next().unwrap_or("").trim().to_string(), "Pool resource"),
        None => (prev_job.storage.split(',').next().unwrap_or("").trim().to_string(), "previous Job"),
    };

    // UpdateJobStartRecord() parts the catalog requests rely on: the write pool.
    let _ = catalog.execute(&format!("UPDATE Job SET Level='{job_level}',PoolId={next_pool_id} WHERE JobId={job_id}"));
    // mig_jcr: the new backup job (SetJcrDefaults(prev_job), SetupJob) —
    // not when the previous job has no data ("no work").
    let new_job_id = if expected == 0 {
        None
    } else {
        let new_unique = crate::commands::job::unique_job_name(&prev.name, now);
        let prev_client = if prev_job.client.is_empty() { client.clone() } else { prev_job.client.clone() };
        let Ok(new_cid) = client_id(catalog, &prev_client) else { return started };
        let new_job_id = match create_job_record(catalog, &new_unique, &prev.name, 'B', job_level, &new_cid, now) {
            Ok(id) => id,
            Err(_) => {
                fatal(msgs, "setup job failed.\n".into());
                return started;
            }
        };
        let _ = catalog.execute(&format!(
            "UPDATE Job SET Level=(SELECT Level FROM Job WHERE JobId={p}),PoolId={next_pool_id},FileSetId={f} WHERE JobId={new_job_id}",
            p = prev.job_id, f = num(&prev.fileset_id)));
        Some(new_job_id)
    };

    let (catalog_name, catalog_source) = catalog_of(config, &job, &read_pool, &client);
    let store = |name: &str, pool: &str| -> Option<crate::sd_copy::StorageUse> {
        let s = config.find_storage(name)?;
        Some(crate::sd_copy::StorageUse {
            name: s.name.clone(), address: s.address.clone(), port: s.port, password: s.password.clone(),
            media_type: s.media_type.clone(),
            devices: s.device.split(',').map(|d| d.trim().to_string()).filter(|d| !d.is_empty()).collect(),
            pool: pool.to_string(),
            pool_type: config.find_pool(pool).map(|p| p.pool_type.clone()).unwrap_or_else(|| "Backup".into()),
        })
    };
    let (Some(read), Some(write)) = (store(&read_store, &read_pool), store(&write_store, &next_pool)) else {
        fatal(msgs, format!("Storage resource \"{read_store}\" or \"{write_store}\" not found.\n"));
        return started;
    };

    let mut info = WorkerInfo {
        copy, job: unique.clone(), job_resource: job.name.clone(), prev: prev.clone(), new_job_id,
        level: job_level, client: client.clone(), fileset: fileset.clone(),
        read_pool: (read_pool.clone(), read_pool_source.into()),
        read_storage: (read_store.clone(), read_store_source.into()),
        write_pool: (next_pool.clone(), next_source.into()),
        write_storage: (write_store.clone(), storage_source.into()),
        next_pool: (next_pool.clone(), next_source.into()),
        catalog: (catalog_name.clone(), catalog_source.clone()),
        priority: job.priority, trigger: req.trigger.to_string(), next_pool_id: next_pool_id.clone(),
        purge_migrate_job: crate::commands::defaults::bool_directive(
            &crate::commands::defaults::job_directives(config, &job.name), "purgemigrationjob", false),
        terminated_early: false,
    };

    // DoActualMigration() before the SD: Version (job_thread), the checks,
    // the start message.
    let mut start_messages = vec![(crate::job_worker::M_INFO, job_messages::version_text())];
    let mut fail_before_sd = false;
    if expected == 0 {
        start_messages.push((crate::job_worker::M_INFO, format!("Previous Job has no data to {action}.\n")));
        fail_before_sd = true;
        info.terminated_early = true;
    } else if !matches!(prev.job_type, 'B' | 'C' | 'A') {
        start_messages.push((crate::job_worker::M_INFO,
            format!("JobId {} already {action_past} probably by another Job. {operation} stopped.\n", prev.job_id)));
        fail_before_sd = true;
        info.terminated_early = true;
    } else if read_store == write_store && !crate::ua_select::is_autochanger(config, &read_store) {
        start_messages.push((3, format!("JobId {} cannot {operation} using the same read and write storage.\n", prev.job_id)));
        fail_before_sd = true;
        info.terminated_early = true;
    } else {
        start_messages.push((crate::job_worker::M_INFO, format!("Start {operation} JobId {job_id}, Job={unique}\n")));
        if read.address != write.address || read.port != write.port {
            // remote_replicate: "listen"/"replicate" between two SDs.
            start_messages.push((3, "Copy/Migrate between two Storage daemons is not available in Onesimus yet.\n".into()));
            fail_before_sd = true;
        }
    }

    let job_dirs = crate::commands::defaults::job_directives(config, &job.name);
    let catalog_files = raw(config, "pool", &next_pool, "catalogfiles")
        .and_then(|v| bareos_config::values::parse_bool(&v).ok().map(|(_, on)| on)).unwrap_or(true);
    let copy_session = crate::sd_copy::CopySession {
        read: read.clone(), write: write.clone(), bootstrap, job_type: if copy { 'c' } else { 'g' }, level: job_level,
        job_resource: job.name.clone(), client: client.clone(), fileset: fileset.clone(), fileset_md5: String::new(),
        no_attributes: !catalog_files,
        spool_attributes: crate::commands::defaults::bool_directive(&job_dirs, "spoolattributes", false),
        spool_data: crate::commands::defaults::bool_directive(&job_dirs, "spooldata", false),
        prefer_mounted_volumes: crate::commands::defaults::bool_directive(&job_dirs, "prefermountedvolumes", true),
        spool_size: 0, start_messages, fail_before_sd,
    };
    let params = crate::job_worker::JobParams {
        job_id, unique_job: unique.clone(), job_name: job.name.clone(),
        job_type: crate::job_worker::WorkerJobType::Backup, level: level_name(job_level).to_ascii_lowercase(),
        fd_address: String::new(), fd_port: 0, fd_password: String::new(),
        sd_address: write.address.clone(), sd_port: write.port, sd_password: write.password.clone(),
        sd_device: write.devices.first().cloned().unwrap_or_default(), sd_media_type: write.media_type.clone(),
        director_name: config.director.name.clone(), pool_name: next_pool.clone(),
        fileset_includes: Vec::new(), fileset_excludes: Vec::new(), since_utime: 0, prev_job: String::new(),
        where_path: None, regex_where: None, prefix_links: false, replace: None, restore_jobid: None, bootstrap: None, client_name: client.clone(),
        client_first_connection: false, copy: Some(copy_session),
    };
    let report = job_messages::JobReport {
        kind: ReportKind::MigrateWorker(copy), job: unique.clone(), level: level_name(job_level).into(),
        client: client.clone(), fileset, pool: next_pool.clone(), pool_source: next_source.into(),
        catalog: catalog_name, catalog_source, storage: write_store.clone(), storage_source: storage_source.into(),
        sched_time: now, start_time: now, priority: job.priority, trigger: req.trigger.to_string(),
        ..Default::default()
    };
    workers().get_or_insert_with(HashMap::new).insert(job_id, info);
    started.child = Some(crate::internal_jobs::ChildStart {
        job_id, job_name: job.name.clone(),
        start: crate::job_manager::QueuedStart { params, priority: job.priority, sched_time: now },
        report, client, storage: write_store,
    });
    started
}

/// The Catalog of a job and its source (SetJcrDefaults: Pool, Job, Client
/// resource, else the first Catalog).
fn catalog_of(config: &DirectorConfig, job: &bareos_config::resources::JobResource, pool: &str, client: &str) -> (String, String) {
    if let Some(catalog) = raw(config, "pool", pool, "catalog") {
        (catalog, "Pool resource".into())
    } else if let Some(catalog) = job_value(config, &job.name, "catalog") {
        (catalog, "Job resource".into())
    } else {
        let client_catalog = config.find_client(client).map(|c| c.catalog.clone()).filter(|c| !c.is_empty());
        (client_catalog.or_else(|| config.catalogs.first().map(|c| c.name.clone())).unwrap_or_default(),
            "Client resource".into())
    }
}

/// The end of a worker job as the SD reported it.
pub struct WorkerEnd {
    pub status: char,
    pub sd_status: char,
    pub sd_files: u64,
    pub sd_bytes: u64,
    pub sd_errors: u32,
    pub job_errors: u32,
    pub vol_session_id: i32,
    pub vol_session_time: i32,
    pub start_time: i64,
    pub end_time: i64,
}

/// MigrationCleanup() of a worker job: the catalog updates of both jobs and
/// the report. None when `job_id` is no worker job of this module; else the
/// final JobStatus and the job messages.
pub fn cleanup(director: &str, catalog: Option<&CatalogHandle>, job_id: u32, end: &WorkerEnd) -> Option<(char, Vec<JobMsg>)> {
    let info = workers().as_mut()?.remove(&job_id)?;
    let mut msgs = Vec::new();
    let mut status = if info.terminated_early { 'T' } else { end.status };
    let (operation, action) = if info.copy { ("Copying", "copy") } else { ("Migration", "migrate") };
    let (sd_files, sd_bytes, sd_status, vol_id, vol_time) = if info.terminated_early {
        (0, 0, '\0', 0, 0)
    } else {
        (end.sd_files, end.sd_bytes, end.sd_status, end.vol_session_id, end.vol_session_time)
    };
    let mut volumes = String::new();
    let mut last_volume_bytes = 0u64;
    if let Some(cat) = catalog {
        let new = info.new_job_id.unwrap_or(0);
        let old = info.prev.job_id;
        let now = end.end_time;
        // UpdateJobEnd(jcr)
        let _ = cat.execute(&format!(
            "UPDATE Job SET JobStatus='{status}',EndTime=to_timestamp({now}),JobBytes=0,ReadBytes=0,JobFiles=0,\
             JobErrors={},VolSessionId={vol_id},VolSessionTime={vol_time},PoolId={},RealEndTime=to_timestamp({now}) \
             WHERE JobId={job_id}", end.job_errors, if info.next_pool_id.is_empty() { "0" } else { &info.next_pool_id }));
        if info.new_job_id.is_some() {
            let _ = cat.execute(&format!("UPDATE Job SET priorjobid='{new}' WHERE JobId={job_id}"));
            let mig_status = if status == 'T' && (end.job_errors > 0 || end.sd_errors > 0) { 'W' } else { status };
            // UpdateJobEnd(mig_jcr)
            let _ = cat.execute(&format!(
                "UPDATE Job SET JobStatus='{mig_status}',EndTime=to_timestamp({now}),JobBytes={sd_bytes},ReadBytes=0,\
                 JobFiles={sd_files},JobErrors=0,VolSessionId={vol_id},VolSessionTime={vol_time},\
                 RealEndTime=to_timestamp({now}),PriorJobId={old},HasBase={hb},PurgedFiles={pf} WHERE JobId={new}",
                hb = num(&info.prev.has_base), pf = num(&info.prev.purged_files)));
            let _ = cat.execute(&format!("UPDATE Job SET StartTime='{}',EndTime='{}',JobTDate={} WHERE JobId={new}",
                info.prev.start_time, info.prev.end_time, if info.prev.job_tdate.is_empty() { "0" } else { &info.prev.job_tdate }));
            if matches!(status, 'T' | 'W') {
                if info.copy {
                    let _ = cat.execute(&format!(
                        "INSERT INTO Log (JobId, Time, LogText ) SELECT {new}, Time, LogText FROM Log WHERE JobId={old}"));
                    let _ = cat.execute(&format!("UPDATE Job SET Type='C' WHERE JobId={new}"));
                } else {
                    let _ = cat.execute(&format!("UPDATE Job SET Type='M' WHERE JobId={old}"));
                    let _ = cat.execute(&format!("UPDATE Log SET JobId={new} WHERE JobId={old}"));
                    match cat.dedicated() {
                        Ok(db) if info.purge_migrate_job => crate::commands::purge::purge_jobs(&db, &old.to_string()),
                        Ok(db) => crate::commands::purge::purge_files(&db, &old.to_string()),
                        Err(e) => msgs.push((MsgType::Error, format!("{e}\n"))),
                    }
                }
            }
            // GetJobVolumeNames(mig_jcr) and the last volume's Media record.
            if let Ok(table) = cat.query_rows(&format!(
                "SELECT DISTINCT VolumeName, MAX(VolIndex) FROM JobMedia,Media WHERE JobMedia.JobId={new} \
                 AND JobMedia.MediaId=Media.MediaId GROUP BY VolumeName ORDER BY 2 ASC")) {
                let names: Vec<String> = table.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect();
                volumes = names.join("|");
                if let Some(last) = names.last() {
                    last_volume_bytes = first_row(cat, &format!("SELECT VolBytes FROM Media WHERE VolumeName='{}'", esc(last)))
                        .ok().flatten().and_then(|r| r[0].parse().ok()).unwrap_or(0);
                }
            }
        }
    }
    let (msg_type, term) = match status {
        // No mig_jcr (and no HasSelectedJobs): "%s -- no files to %s".
        _ if info.new_job_id.is_none() => (MsgType::Info, format!("{operation} -- no files to {action}")),
        'T' => (MsgType::Info, format!("{operation} OK")),
        'W' => (MsgType::Info, format!("{operation} OK -- with warnings")),
        'A' => (MsgType::Info, format!("{operation} Canceled")),
        'f' | 'E' => (MsgType::Error, format!("*** {operation} Error ***")),
        _ => {
            status = end.status;
            (MsgType::Info, format!("Inappropriate {operation} term code"))
        }
    };
    if info.new_job_id.is_none() {
        volumes = "*None*".into();
    }
    let run_time = end.end_time - end.start_time;
    let kbps = if run_time <= 0 { 0.0 } else { sd_bytes as f64 / (1000.0 * run_time as f64) };
    let opt = |id: Option<u32>| id.map_or("*None*".to_string(), |i| i.to_string());
    let text = format!(
        "{head}  OS Information:         {os}\n  Prev Backup JobId:      {prev_id}\n  Prev Backup Job:        {prev_job}\n\
         \x20 New Backup JobId:       {new_id}\n  Current JobId:          {job_id}\n  Current Job:            {job}\n\
         \x20 Backup Level:           {level}\n  Client:                 {client}\n  FileSet:                \"{fileset}\"\n\
         \x20 Read Pool:              \"{rp}\" (From {rps})\n  Read Storage:           \"{rs}\" (From {rss})\n\
         \x20 Write Pool:             \"{wp}\" (From {wps})\n  Write Storage:          \"{ws}\" (From {wss})\n\
         \x20 Next Pool:              \"{np}\" (From {nps})\n  Catalog:                \"{cat}\" (From {cats})\n\
         \x20 Start time:             {start}\n  End time:               {endt}\n  Elapsed time:           {elapsed}\n\
         \x20 Priority:               {priority}\n  SD Files Written:       {files}\n  SD Bytes Written:       {bytes} ({sbytes}B)\n\
         \x20 Rate:                   {rate} KB/s\n  Volume name(s):         {volumes}\n  Volume Session Id:      {vol_id}\n\
         \x20 Volume Session Time:    {vol_time}\n  Last Volume Bytes:      {lvb} ({slvb}B)\n  SD Errors:              {sd_errors}\n\
         \x20 SD termination status:  {sd_status}\n  Bareos binary info:     {binary}\n  Job triggered by:       {trigger}\n\
         \x20 Termination:            {term}\n\n",
        head = job_messages::summary_head(director), os = crate::version_strings::os_info(),
        prev_id = info.prev.job_id, prev_job = info.prev.job, new_id = opt(info.new_job_id), job = info.job,
        level = level_name(info.level), client = info.client, fileset = info.fileset,
        rp = info.read_pool.0, rps = info.read_pool.1, rs = info.read_storage.0, rss = info.read_storage.1,
        wp = info.write_pool.0, wps = info.write_pool.1, ws = info.write_storage.0, wss = info.write_storage.1,
        np = info.next_pool.0, nps = info.next_pool.1, cat = info.catalog.0, cats = info.catalog.1,
        start = job_messages::bstrftimes(end.start_time), endt = job_messages::bstrftimes(end.end_time),
        elapsed = crate::ua_select::edit_utime(run_time.max(0) as u64), priority = info.priority,
        files = job_messages::commas(sd_files), bytes = job_messages::commas(sd_bytes), sbytes = job_messages::suffix(sd_bytes),
        rate = job_messages::fixed1(kbps as f32 as f64), lvb = job_messages::commas(last_volume_bytes),
        slvb = job_messages::suffix(last_volume_bytes), sd_errors = if info.terminated_early { 0 } else { end.sd_errors },
        sd_status = job_messages::job_status_to_ascii(sd_status), binary = crate::version_strings::JOBLOG_MESSAGE,
        trigger = info.trigger);
    msgs.push((msg_type, text));
    Some((status, msgs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_jobs_are_not_cleaned_up() {
        let end = WorkerEnd { status: 'T', sd_status: 'T', sd_files: 0, sd_bytes: 0, sd_errors: 0, job_errors: 0,
            vol_session_id: 0, vol_session_time: 0, start_time: 0, end_time: 0 };
        assert!(cleanup("dir", None, 987_654, &end).is_none());
        assert_eq!(new_job_of(987_654), None);
    }

    #[test]
    fn worker_report_follows_generate_migrate_summary() {
        let info = WorkerInfo {
            copy: true, job: "mig-copy.2026-10-17_06.53.05_05".into(), job_resource: "mig-copy".into(),
            prev: PrevJob { job_id: 630, job: "mig-backup.2026-10-17_06.52.59_03".into(), ..Default::default() },
            new_job_id: Some(633), level: 'F', client: "mig-fd".into(), fileset: "MigSet".into(),
            read_pool: ("MigSrc".into(), "Job resource".into()), read_storage: ("SrcStore".into(), "Pool resource".into()),
            write_pool: ("MigDst".into(), "Job Pool's NextPool resource".into()),
            write_storage: ("DstStore".into(), "Storage from Pool's NextPool resource".into()),
            next_pool: ("MigDst".into(), "Job Pool's NextPool resource".into()),
            catalog: ("MyCatalog".into(), "Client resource".into()), priority: 10, trigger: "User".into(),
            ..Default::default()
        };
        workers().get_or_insert_with(HashMap::new).insert(987_001, info);
        assert_eq!(new_job_of(987_001), Some(633));
        let end = WorkerEnd { status: 'T', sd_status: 'T', sd_files: 6, sd_bytes: 60_678, sd_errors: 0, job_errors: 0,
            vol_session_id: 4, vol_session_time: 1_792_219_807, start_time: 1_000, end_time: 1_000 };
        let (status, msgs) = cleanup("bareos-dir", None, 987_001, &end).unwrap();
        assert_eq!(status, 'T');
        let text = &msgs[0].1;
        for line in ["  Prev Backup JobId:      630\n", "  New Backup JobId:       633\n", "  Current JobId:          987001\n",
            "  Read Pool:              \"MigSrc\" (From Job resource)\n",
            "  Write Storage:          \"DstStore\" (From Storage from Pool's NextPool resource)\n",
            "  SD Files Written:       6\n", "  SD Bytes Written:       60,678 (60.67 KB)\n", "  Volume Session Id:      4\n",
            "  SD termination status:  OK\n", "  Termination:            Copying OK\n\n"] {
            assert!(text.contains(line), "{line:?} in {text}");
        }
        assert_eq!(new_job_of(987_001), None);
    }
}
