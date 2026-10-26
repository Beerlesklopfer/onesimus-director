//! VirtualFull backups (dird/vbackup.cc), Compatibility log C-058, S-06.
//!
//! The Director side: DoNativeVbackupInit (read pool and storage, Next Pool
//! and its storage), the part of DoNativeVbackup before the Storage daemon
//! (JobId list, catalog consistency check, bootstrap) and
//! NativeVbackupCleanup (level of the first job, times of the newest job,
//! replication of deleted files, purge of consolidated jobs). The Storage
//! daemon read+write session runs in the job worker (sd_copy.rs).
//!
//! Deviations: the bootstrap and the JobId list are made when the job is
//! queued (the original: when it starts); a failing init (no Next Pool,
//! no storage) creates no Job record; AllowDuplicateJob is not checked.

use crate::catalog_worker::CatalogHandle;
use crate::commands::defaults::{bool_directive, first, job_directives, time_directive};
use crate::director_config::DirectorConfig;
use crate::job_messages::JobMsg;
use crate::message_bus::MsgType;
use bareos_config::raw::Directive;
use bareos_config::resources::JobResource;

/// lib/message.h M_* codes for the worker's start messages.
const M_FATAL: u32 = 3;
const M_ERROR: u32 = 4;
const M_WARNING: u32 = 5;
const M_INFO: u32 = 6;

/// What `run job=.. level=VirtualFull ..` asks for (ua_run.cc RunContext).
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub job: JobResource,
    /// `jobid=` (jcr->dir_impl->vf_jobids).
    pub jobids: Option<String>,
    pub priority: Option<i32>,
    pub accurate: Option<bool>,
    pub spool_data: Option<bool>,
    /// `consolidatejob=`: the Consolidate job that started it.
    pub consolidate_job: Option<String>,
    pub client: Option<String>,
    pub fileset: Option<String>,
    /// `pool=` (command line).
    pub pool: Option<String>,
    /// `nextpool=` (Run NextPool override).
    pub next_pool: Option<String>,
    pub sched_time: i64,
    pub trigger: String,
    /// JobId and Job name of a backup upgraded to VirtualFull by Max Virtual
    /// Full Interval (S-23); None creates the Job record.
    pub existing: Option<(u32, String)>,
}

/// Job data NativeVbackupCleanup needs (JobReport.vf).
#[derive(Debug, Clone, Default)]
pub struct VfInfo {
    /// The JobIds consolidated (GetVfJobids()).
    pub jobids: String,
    /// jcr->dir_impl->vf_jobids: the JobIds given with `jobid=`.
    pub vf_jobids: Option<String>,
    /// Level of the first consolidated job (JobLevel_of_first_job).
    pub first_level: Option<char>,
    /// previous_jr: StartTime, EndTime (catalog text) and JobTDate of the
    /// newest consolidated job.
    pub previous: Option<(String, String, i64)>,
    /// AlwaysIncremental with AlwaysIncrementalJobRetention: purge the
    /// consolidated jobs afterwards.
    pub purge: bool,
}

fn sql_esc(text: &str) -> String {
    text.replace('\'', "''")
}

fn first_value(catalog: &CatalogHandle, sql: &str) -> Result<Option<String>, String> {
    Ok(catalog.query_rows(sql)?.rows.into_iter().next().and_then(|r| r.into_iter().next().flatten()))
}

fn raw_dirs(config: &DirectorConfig, kind: &str, name: &str) -> Vec<Directive> {
    config.raw.iter()
        .find(|r| r.norm_type() == kind && r.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .map(|r| r.directives.clone()).unwrap_or_default()
}

/// The first value of a resource-list directive ("A, B" or several values).
fn first_item(dirs: &[Directive], key: &str) -> Option<String> {
    first(dirs, key).map(|v| v.split(',').next().unwrap_or("").trim().to_string()).filter(|v| !v.is_empty())
}

/// All Device names of a Storage resource.
fn storage_devices(config: &DirectorConfig, storage: &str) -> Vec<String> {
    let mut devices: Vec<String> = raw_dirs(config, "storage", storage).iter()
        .filter(|d| d.norm_key() == "device")
        .flat_map(|d| d.values().iter().flat_map(|v| v.split(',').map(|s| s.trim().to_string())).collect::<Vec<_>>())
        .filter(|d| !d.is_empty()).collect();
    if devices.is_empty() {
        if let Some(store) = config.find_storage(storage) {
            devices.push(store.device.clone());
        }
    }
    devices
}

fn storage_use(config: &DirectorConfig, storage: &str, pool: &str) -> Result<crate::sd_copy::StorageUse, String> {
    let store = config.find_storage(storage).ok_or_else(|| format!("Storage resource \"{storage}\" not found.\n"))?;
    let pool_type = first(&raw_dirs(config, "pool", pool), "pooltype").unwrap_or("Backup").to_string();
    Ok(crate::sd_copy::StorageUse {
        name: store.name.clone(), address: store.address.clone(), port: store.port, password: store.password.clone(),
        media_type: store.media_type.clone(), devices: storage_devices(config, storage), pool: pool.to_string(),
        pool_type,
    })
}

/// GetOrCreateClientRecord(): the ClientId.
fn client_id(catalog: &CatalogHandle, client: &str) -> Result<i64, String> {
    let id = match first_value(catalog, &format!("SELECT ClientId FROM Client WHERE Name='{}'", sql_esc(client)))? {
        Some(id) => id,
        None => first_value(catalog, &format!(
            "INSERT INTO Client (Name,Uname,AutoPrune,FileRetention,JobRetention) \
             VALUES ('{}','',0,5184000,15552000) RETURNING ClientId", sql_esc(client)))?
            .ok_or("CreateClientRecord failed")?,
    };
    Ok(id.parse().unwrap_or(0))
}

/// The FileSet record of a FileSet resource (newest of that name) and its
/// MD5; (0, "**Dummy**") without one.
fn fileset_record(catalog: &CatalogHandle, fileset: &str) -> (i64, String) {
    catalog.query_rows(&format!(
        "SELECT FileSetId,MD5 FROM FileSet WHERE FileSet='{}' ORDER BY CreateTime DESC, FileSetId DESC LIMIT 1",
        sql_esc(fileset))).ok()
        .and_then(|t| t.rows.into_iter().next())
        .map(|r| (r.first().cloned().flatten().and_then(|v| v.parse().ok()).unwrap_or(0),
                  r.get(1).cloned().flatten().filter(|m| !m.is_empty()).unwrap_or_else(|| "**Dummy**".into())))
        .unwrap_or((0, "**Dummy**".to_string()))
}

/// GetOrCreatePoolRecord() (lookup).
fn pool_id(catalog: &CatalogHandle, pool: &str) -> Option<i64> {
    first_value(catalog, &format!("SELECT PoolId FROM Pool WHERE Name='{}'", sql_esc(pool))).ok().flatten()
        .and_then(|v| v.parse().ok())
}

/// AccurateGetJobids() of a job with JobLevel Incremental/VirtualFull.
pub fn accurate_jobids(catalog: &CatalogHandle, job_id: u32, client_id: i64, fileset_id: i64, start_time: i64,
                       limit: u64) -> Vec<String> {
    let start = if start_time != 0 { start_time } else { (unsafe { libc::time(std::ptr::null_mut()) }) as i64 };
    let date = crate::job_messages::bstrutime(start + 1);
    crate::commands::bvfs::accurate_get_jobids_job(catalog, job_id, client_id, 'B', &date, fileset_id, true, limit)
        .unwrap_or_default()
}

/// A queued VirtualFull: the job start and the messages of a refused start
/// (DoRunCmd() returned 0).
pub fn start(config: &DirectorConfig, catalog: Option<&CatalogHandle>, req: &Request)
    -> Result<crate::internal_jobs::ChildStart, Vec<JobMsg>> {
    let fatal = |text: String| vec![(MsgType::Fatal, text)];
    let job = &req.job;
    let dirs = job_directives(config, &job.name);
    let Some(catalog) = catalog else { return Err(fatal("Could not open database \"bareos\".\n".into())) };
    let client = req.client.clone().unwrap_or_else(|| job.client.clone());
    let fileset = req.fileset.clone().unwrap_or_else(|| job.fileset.clone());
    let Some(client_res) = config.find_client(&client) else {
        return Err(fatal(format!("Client \"{client}\" not found.\n")));
    };

    // ApplyPoolOverrides(): the VirtualFull Backup Pool; then the read pool.
    let (rpool, rpool_source) = match (&req.pool, first_item(&dirs, "virtualfullbackuppool")) {
        (Some(pool), _) => (pool.clone(), "command line".to_string()),
        (None, Some(pool)) => (pool, "Job VFullPool override".to_string()),
        (None, None) => (job.pool.clone(), "Job resource".to_string()),
    };
    let rpool_dirs = raw_dirs(config, "pool", &rpool);

    // SetupJob(): GetOrCreateClientRecord, CreateJobRecord.
    let client_id = client_id(catalog, &client).map_err(|e| fatal(format!("{e}\n")))?;
    let (fileset_id, fileset_md5) = fileset_record(catalog, &fileset);
    let now = (unsafe { libc::time(std::ptr::null_mut()) }) as i64;
    let sched_time = if req.sched_time != 0 { req.sched_time } else { now };
    let (job_id, unique) = match &req.existing {
        // A backup GetLevelSinceTime() upgraded to VirtualFull keeps its
        // Job record (job.cc SetupJob), now with Level 'f' (S-23).
        Some((job_id, unique)) => {
            let _ = catalog.execute(&format!("UPDATE Job SET Level='f' WHERE JobId={job_id}"));
            (*job_id, unique.clone())
        }
        None => {
            let unique = crate::commands::job::unique_job_name(&job.name, now);
            let job_id: u32 = first_value(catalog, &format!(
                "INSERT INTO Job (Job,Name,Type,Level,JobStatus,SchedTime,JobTDate,ClientId,Comment) \
                 VALUES ('{}','{}','B','f','C','{}',{sched_time},{client_id},'') RETURNING JobId",
                sql_esc(&unique), sql_esc(&job.name), crate::job_messages::bstrutime(sched_time)))
                .ok().flatten().and_then(|id| id.parse().ok())
                .ok_or_else(|| fatal("Create DB Job record failed\n".into()))?;
            (job_id, unique)
        }
    };

    // DoNativeVbackupInit() failed: NativeVbackupCleanup(JS_ErrorTerminated)
    // with the write storage of SetJcrDefaults; RunJob() returns 0.
    let fail_init = |text: String, pool: &str, pool_source: &str, pool_id: i64| {
        let (storage, storage_source) = match first_item(&dirs, "storage") {
            Some(store) => (store, "Job resource"),
            None => (first_item(&rpool_dirs, "storage").unwrap_or_default(), "Pool resource"),
        };
        let _ = catalog.execute(&format!(
            "UPDATE Job SET JobStatus='f',StartTime='{t}',EndTime='{t}',\
             RealEndTime='{t}',JobTDate={now},PoolId={pool_id},FileSetId={fileset_id},\
             ClientId={client_id} WHERE JobId={job_id}", t = crate::job_messages::bstrutime(now)));
        let mut report = crate::job_messages::backup_report(config, job, client_res, &storage, pool, pool, "f",
            &fileset, sched_time);
        report.job = unique.clone();
        report.pool_source = pool_source.to_string();
        report.storage_source = storage_source.to_string();
        report.start_time = now;
        report.accurate = req.accurate.unwrap_or_else(|| bool_directive(&dirs, "accurate", false));
        report.priority = req.priority.unwrap_or(job.priority);
        // jcr->dir_impl->job_trigger is set after RunJob() returned.
        report.trigger = "Unknown".to_string();
        report.vf = Some(VfInfo::default());
        let outcome = crate::job_messages::JobOutcome { job_id, status: 'f', end_time: now, fd_status: 'f',
            ..Default::default() };
        let mut messages = vec![(MsgType::Fatal, text)];
        messages.extend(crate::job_messages::end_of_job(&config.director.name, Some(catalog), &report, &outcome));
        for (msg_type, text) in &messages {
            crate::console_msg::job_message(config, &job.name, job_id, *msg_type, now, text);
        }
        Vec::new()
    };
    let Some(rpool_id) = pool_id(catalog, &rpool) else {
        return Err(fail_init("Could not get or create a Pool record.\n".into(), &rpool, &rpool_source, 0));
    };
    // If pool storage specified, use it for restore (CopyRstorage).
    let (rstore, _rstore_source) = match first_item(&rpool_dirs, "storage") {
        Some(store) => (store, "Pool resource"),
        None => (job.storage.split(',').next().unwrap_or("").trim().to_string(), "Job resource"),
    };
    // Next Pool: Run override, Job's, else the Pool's.
    let (next_pool, npool_source, storage_source) = if let Some(pool) = &req.next_pool {
        (Some(pool.clone()), "Run NextPool override", "Storage from Run NextPool override")
    } else if let Some(pool) = first_item(&dirs, "nextpool") {
        (Some(pool), "Job's NextPool resource", "Storage from Job's NextPool resource")
    } else {
        (first_item(&rpool_dirs, "nextpool"), "Job Pool's NextPool resource", "Storage from Pool's NextPool resource")
    };
    // SetMigrationWstorage()
    let Some(wpool) = next_pool else {
        return Err(fail_init(format!("No Next Pool specification found in Pool \"{rpool}\".\n"), &rpool,
            npool_source, rpool_id));
    };
    let wpool_dirs = raw_dirs(config, "pool", &wpool);
    let Some(wstore) = first_item(&wpool_dirs, "storage") else {
        return Err(fail_init(format!("No Storage specification found in Next Pool \"{wpool}\".\n"), &rpool,
            npool_source, rpool_id));
    };
    let read = storage_use(config, &rstore, &rpool)
        .map_err(|e| fail_init(e, &rpool, npool_source, rpool_id))?;
    let write = storage_use(config, &wstore, &wpool)
        .map_err(|e| fail_init(e, &rpool, npool_source, rpool_id))?;
    let wpool_id = pool_id(catalog, &wpool).unwrap_or(rpool_id);
    // UpdateJobStartRecord (PoolId, FileSetId, ClientId); the write pool,
    // which the catalog requests of the Storage daemon check.
    let _ = catalog.execute(&format!(
        "UPDATE Job SET PoolId={wpool_id},FileSetId={fileset_id},ClientId={client_id} WHERE JobId={job_id}"));

    let accurate = req.accurate.unwrap_or_else(|| bool_directive(&dirs, "accurate", false));
    let spool_data = req.spool_data.unwrap_or_else(|| bool_directive(&dirs, "spooldata", false));
    let always_incremental = bool_directive(&dirs, "alwaysincremental", false);
    let mut vf = VfInfo {
        vf_jobids: req.jobids.clone(),
        purge: always_incremental && time_directive(&dirs, "alwaysincrementaljobretention") > 0,
        ..Default::default()
    };
    // DoNativeVbackup() up to the Storage daemon.
    let mut messages: Vec<(u32, String)> = vec![
        (M_INFO, crate::job_messages::version_text()),
        (M_INFO, format!("Start Virtual Backup JobId {job_id}, Job={unique}\n")),
    ];
    if !accurate {
        messages.push((M_WARNING, "This Job is not an Accurate backup so is not equivalent to a Full backup.\n".into()));
    }
    let bootstrap = prepare(config, catalog, job_id, req, always_incremental, client_id, fileset_id,
        &mut vf, &mut messages);
    let fail_before_sd = bootstrap.is_none();

    let wpool_catalog_files = first(&wpool_dirs, "catalogfiles")
        .and_then(|v| bareos_config::values::parse_bool(v).ok().map(|(_, on)| on)).unwrap_or(true);
    let session = crate::sd_copy::CopySession {
        read, write: write.clone(), bootstrap: bootstrap.unwrap_or_default(), job_type: 'B', level: 'f',
        job_resource: job.name.clone(), client: client.clone(), fileset: fileset.clone(), fileset_md5,
        no_attributes: !wpool_catalog_files, spool_attributes: bool_directive(&dirs, "spoolattributes", false),
        spool_data, prefer_mounted_volumes: bool_directive(&dirs, "prefermountedvolumes", true),
        spool_size: crate::commands::defaults::number(&dirs, "spoolsize", 0, crate::commands::defaults::SIZES)
            .unwrap_or(0) as i64,
        start_messages: messages, fail_before_sd, remote: None,
    };
    let params = crate::job_worker::JobParams {
        job_id, unique_job: unique.clone(), job_name: job.name.clone(),
        job_type: crate::job_worker::WorkerJobType::Backup, level: "VirtualFull".into(),
        fd_address: String::new(), fd_port: 0, fd_password: String::new(), fd_grpc: None,
        sd_address: write.address.clone(), sd_port: write.port, sd_password: write.password.clone(),
        sd_device: write.devices.first().cloned().unwrap_or_default(), sd_media_type: write.media_type.clone(),
        director_name: config.director.name.clone(), pool_name: wpool.clone(), fileset_includes: Vec::new(),
        fileset_excludes: Vec::new(), since_utime: 0, prev_job: String::new(), where_path: None, replace: None,
        restore_jobid: None, bootstrap: None, client_name: client.clone(), client_first_connection: false, passive_client: false,
        copy: Some(session), regex_where: None, prefix_links: false, verify: None, backup: None,
    };

    let mut report = crate::job_messages::backup_report(config, job, client_res, &wstore, &rpool, &rpool, "f",
        &fileset, sched_time);
    report.job = unique;
    report.level = "VirtualFull".into();
    report.pool = wpool;
    report.pool_source = npool_source.to_string();
    report.storage = wstore.clone();
    report.storage_source = storage_source.to_string();
    report.accurate = accurate;
    report.priority = req.priority.unwrap_or(job.priority);
    if !req.trigger.is_empty() {
        report.trigger = req.trigger.clone();
    }
    // A VirtualFull of a Consolidate job shares its AllowMixedPriority.
    if let Some(consolidate) = &req.consolidate_job {
        report.allow_mixed_priority = bool_directive(&job_directives(config, consolidate), "allowmixedpriority", false);
    }
    report.vf = Some(vf);
    Ok(crate::internal_jobs::ChildStart {
        job_id, job_name: job.name.clone(),
        start: crate::job_manager::QueuedStart { params, priority: report.priority, sched_time },
        report, client, storage: wstore,
    })
}

/// DoNativeVbackup() before the Storage daemon: the JobIds, the catalog
/// consistency check, the first and newest job and the bootstrap
/// (CreateBootstrapFile). None: the job ends JS_FatalError.
#[allow(clippy::too_many_arguments)]
fn prepare(config: &DirectorConfig, catalog: &CatalogHandle, job_id: u32, req: &Request, always_incremental: bool,
           client_id: i64, fileset_id: i64, vf: &mut VfInfo, messages: &mut Vec<(u32, String)>) -> Option<String> {
    // GetVfJobids()
    let jobids = match &req.jobids {
        Some(ids) => ids.clone(),
        None if always_incremental => {
            messages.push((M_ERROR, "Cannot run always incremental job at level VirtualFull with no jobid list.\n".into()));
            String::new()
        }
        None => accurate_jobids(catalog, job_id, client_id, fileset_id, 0, 0).join(","),
    };
    let jobid_list: Vec<String> = jobids.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
    if jobid_list.is_empty() {
        messages.push((M_FATAL, "No previous Jobs found.\n".into()));
        return None;
    }
    vf.jobids = jobids.clone();

    // JobConsistencyChecker
    let rows = catalog.query_rows(&format!(
        "SELECT JobId, Type, ClientId, FilesetId, PurgedFiles FROM Job WHERE JobId IN ({jobids})"))
        .map(|t| t.rows).unwrap_or_default();
    let found: Vec<String> = rows.iter().map(|r| r.first().cloned().flatten().unwrap_or_default()).collect();
    if found.len() != jobid_list.len() {
        let mut wanted = jobid_list.clone();
        wanted.sort();
        let mut have = found.clone();
        have.sort();
        for missing in wanted.iter().filter(|id| have.binary_search(id).is_err()) {
            messages.push((M_ERROR, format!("JobId {missing} is not present in the catalog\n")));
        }
        messages.push((M_FATAL, "Jobs missing from catalog. Cannot continue.\n".into()));
        return None;
    }
    let purged: Vec<&String> = rows.iter().zip(&found)
        .filter(|(r, _)| !r.get(4).cloned().flatten().unwrap_or_default().starts_with('0')).map(|(_, id)| id).collect();
    if !purged.is_empty() {
        for id in purged {
            messages.push((M_ERROR, format!("Files for JobId {id} have been purged from the catalog\n")));
        }
        messages.push((M_FATAL, "At least one job's files were pruned from the catalog.\n".into()));
        return None;
    }

    // The level of the first job and the times of the newest.
    let record = |id: &str| catalog.query_rows(&format!(
        "SELECT Level,StartTime,EndTime,JobTDate FROM Job WHERE JobId={id}")).ok()
        .and_then(|t| t.rows.into_iter().next())
        .map(|r| r.into_iter().map(|v| v.unwrap_or_default()).collect::<Vec<_>>());
    let Some(first_job) = record(&jobid_list[0]) else {
        messages.push((M_FATAL, format!("Error getting Job record for first Job: ERR=No Job found for JobId {}\n",
            jobid_list[0])));
        return None;
    };
    vf.first_level = first_job[0].chars().next();
    let last = jobid_list.last().unwrap();
    let Some(last_job) = record(last) else {
        messages.push((M_FATAL, format!("Error getting Job record for previous Job: ERR=No Job found for JobId {last}\n")));
        return None;
    };
    vf.previous = Some((last_job[1].clone(), last_job[2].clone(), last_job[3].parse().unwrap_or(0)));

    // CreateBootstrapFile(): GetFileList(jobids, no md5, delta).
    let sql = format!("SELECT T1.JobId, T1.FileIndex \
        FROM ( SELECT DISTINCT ON (Name, PathId, DeltaSeq) JobTDate, JobId, FileId, FileIndex, PathId, \
        Filename AS Name, LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM (SELECT FileId, JobId, PathId, Name AS FileName, \
        FileIndex, LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM File WHERE JobId IN ({jobids}) UNION ALL \
        SELECT File.FileId, File.JobId, PathId, File.Name AS FileName, File.FileIndex, LStat, MD5, DeltaSeq, Fhinfo, \
        Fhnode FROM BaseFiles JOIN File USING (FileId) WHERE BaseFiles.JobId IN ({jobids}) ) AS T \
        JOIN Job USING (JobId) ORDER BY Name, PathId, DeltaSeq, JobTDate DESC ) AS T1 \
        JOIN Path ON (Path.PathId = T1.PathId) WHERE FileIndex > 0 ORDER BY T1.JobTDate, FileIndex ASC");
    let mut bsr = crate::commands::restore_bsr::Bsr::default();
    let mut found_files = false;
    match catalog.query_rows(&sql) {
        Ok(table) => {
            for row in table.rows {
                let value = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                found_files = true;
                bsr.add_findex(value(0).parse().unwrap_or(0), value(1).parse().unwrap_or(0));
            }
        }
        Err(e) => messages.push((M_ERROR, format!("{e}\n"))),
    }
    let fail = |messages: &mut Vec<(u32, String)>| {
        messages.push((M_FATAL, "Could not create bootstrap file\n".into()));
        None
    };
    if !found_files {
        messages.push((M_ERROR, format!("No files were found in the jobids {jobids}\n")));
        return fail(messages);
    }
    // AddVolumeInformationToBsr/WriteBsrFile of a UaContext without
    // console: their console messages are M_INFO job messages.
    let mut ua_messages = Vec::new();
    if let Err(e) = bsr.add_volume_information_db(catalog) {
        ua_messages.push(e);
    }
    let (count, fname) = bsr.write_file_for_job(config, "", &mut ua_messages);
    messages.extend(ua_messages.into_iter().map(|m| (M_INFO, m)));
    if count == 0 {
        return fail(messages);
    }
    let text = std::fs::read_to_string(&fname).unwrap_or_default();
    // unlink_bsr: the bootstrap goes to the worker, the file is not kept.
    let _ = std::fs::remove_file(&fname);
    messages.push((M_INFO, format!("Consolidating JobIds {jobids} containing {count} files\n")));
    Some(text)
}

/// NativeVbackupCleanup() before UpdateJobEnd(): the final status (the
/// job ended JS_ErrorTerminated unless the Storage daemon terminated it
/// normally) and "Joblevel was set ...".
pub fn before_job_end(vf: &VfInfo, status: &mut char) -> Vec<JobMsg> {
    if !matches!(*status, 'T' | 'W' | 'A' | 'f') {
        *status = 'E';
    }
    match (*status, vf.first_level) {
        ('T' | 'W', Some(level)) => vec![(MsgType::Info, format!("Joblevel was set to joblevel of first consolidated job: {}\n",
            job_level_to_str(level)))],
        _ => Vec::new(),
    }
}

/// NativeVbackupCleanup() after UpdateJobEnd(): the level of the first
/// job, the times of the newest job, and the deleted files of the given
/// JobIds replicated to the new job.
pub fn after_job_end(catalog: Option<&CatalogHandle>, job_id: u32, vf: &VfInfo, status: char) -> Vec<JobMsg> {
    let mut messages = Vec::new();
    let Some(catalog) = catalog else { return messages };
    if let (true, Some(level)) = (matches!(status, 'T' | 'W'), vf.first_level) {
        let _ = catalog.execute(&format!("UPDATE Job SET Level='{level}' WHERE JobId={job_id}"));
    }
    if let Some((start, end, tdate)) = &vf.previous {
        let _ = catalog.execute(&format!(
            "UPDATE Job SET StartTime='{start}',EndTime='{end}',JobTDate={tdate} WHERE JobId={job_id}"));
    }
    if let Some(ids) = vf.vf_jobids.as_deref().filter(|ids| !ids.is_empty()) {
        messages.push((MsgType::Info, format!("Replicating deleted files from jobids {ids} to jobid {job_id}\n")));
        let inner = format!("SELECT DISTINCT ON (Name, PathId) JobTDate, JobId, FileId, FileIndex, PathId, \
            FileName AS Name, LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM (SELECT FileId, JobId, PathId, Name AS FileName, \
            FileIndex, LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM File WHERE JobId IN ({ids}) UNION ALL \
            SELECT File.FileId, File.JobId, PathId, File.Name AS FileName, File.FileIndex, LStat, MD5, DeltaSeq, \
            Fhinfo, Fhnode FROM BaseFiles JOIN File USING (FileId) WHERE BaseFiles.JobId IN ({ids}) ) AS T \
            JOIN Job USING (JobId) ORDER BY Name, PathId, JobTDate DESC");
        let sql = format!("INSERT INTO File (FileIndex, JobId, PathId, LStat, MD5, Name) SELECT FileIndex, {job_id} AS JobId, \
            PathId, LStat, MD5, Name FROM ({inner}) T WHERE FileIndex = 0");
        if let Err(e) = catalog.execute(&sql) {
            messages.push((MsgType::Warning, format!("Error replicating deleted files: ERR={e}\n")));
        }
    }
    messages
}

/// DoNativeVbackup() after NativeVbackupCleanup(): the consolidated jobs of
/// an Always Incremental job with AlwaysIncrementalJobRetention go.
pub fn after_report(catalog: Option<&CatalogHandle>, job_id: u32, vf: &VfInfo, status: char) -> Vec<JobMsg> {
    if !matches!(status, 'T' | 'W') || !vf.purge || vf.jobids.is_empty() {
        return Vec::new();
    }
    if let Some(catalog) = catalog {
        crate::commands::purge::purge_jobs(catalog, &vf.jobids);
    }
    vec![(MsgType::Info, format!("purged JobIds {} as they were consolidated into Job {job_id}\n", vf.jobids))]
}

/// job_level_to_str() (lib/util.cc).
pub fn job_level_to_str(level: char) -> &'static str {
    match level {
        'B' => "Base", 'F' => "Full", 'I' => "Incremental", 'D' => "Differential", 'S' => "Since",
        'C' => "Verify Catalog", 'V' => "Verify Init Catalog", 'O' => "Verify Volume to Catalog",
        'd' => "Verify Disk to Catalog", 'A' => "Verify Data", 'f' => "Virtual Full", ' ' => " ",
        _ => "Unknown Job Level",
    }
}
