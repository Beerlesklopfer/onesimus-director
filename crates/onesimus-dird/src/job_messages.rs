//! Job message texts of the Director (C++ dird/job.cc, backup.cc,
//! restore.cc, admin.cc, msgchan.cc, fd_cmds.cc, sd_cmds.cc, lib/bsock.cc)
//! and the `catalog` message destination (dird.cc DirDbLogInsert).
//!
//! The texts are the bodies handed to Jmsg(); the "<director> JobId <n>: "
//! header is added by `console_msg::job_message_text`, the delivery to the
//! Messages destinations by `audit::dispatch`.

use std::sync::Mutex;

use crate::catalog_worker::CatalogHandle;
use crate::message_bus::MsgType;
use crate::version_strings as version;

// --- catalog destination ----------------------------------------------------

/// The Director's catalog connection used for the Log table (jcr->db).
static CATALOG: Mutex<Option<CatalogHandle>> = Mutex::new(None);

#[cfg(test)]
thread_local! {
    static TEST_CATALOG: std::cell::RefCell<Option<CatalogHandle>> = const { std::cell::RefCell::new(None) };
}

/// Register the catalog for the `catalog` message destination.
pub fn set_catalog(catalog: Option<CatalogHandle>) {
    #[cfg(test)]
    return TEST_CATALOG.with(|c| *c.borrow_mut() = catalog);
    #[cfg(not(test))]
    {
        *CATALOG.lock().unwrap_or_else(|e| e.into_inner()) = catalog;
    }
}

fn catalog() -> Option<CatalogHandle> {
    #[cfg(test)]
    return TEST_CATALOG.with(|c| c.borrow().clone());
    #[cfg(not(test))]
    CATALOG.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// C++ EscapeString (PostgreSQL, standard_conforming_strings on).
fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// The INSERT of DirDbLogInsert(jcr, mtime, msg).
pub fn log_insert_sql(job_id: u32, mtime: i64, msg: &str) -> String {
    format!("INSERT INTO Log (JobId, Time, LogText) VALUES ({job_id},'{}','{}')", bstrutime(mtime), esc(msg))
}

/// DirDbLogInsert(): false without a catalog or when the INSERT fails.
pub fn log_insert(job_id: u32, mtime: i64, msg: &str) -> bool {
    let Some(catalog) = catalog() else { return false };
    catalog.query_rows(&log_insert_sql(job_id, mtime, msg)).is_ok()
}

// --- formatting helpers -------------------------------------------------------

/// bstrftimes(): "%d-%b-%Y %H:%M:%S" in local time and the LC_TIME locale.
pub fn bstrftimes(t: i64) -> String {
    crate::localtime::strftime("%d-%b-%Y %H:%M:%S", t as libc::time_t)
}

/// bstrutime(): "%Y-%m-%d %H:%M:%S" in local time.
pub fn bstrutime(t: i64) -> String {
    crate::localtime::strftime("%Y-%m-%d %H:%M:%S", t as libc::time_t)
}

/// edit_uint64_with_commas().
pub(crate) fn commas(v: u64) -> String {
    crate::sql_list::add_commas(&v.to_string())
}

/// edit_uint64_with_suffix().
pub(crate) fn suffix(v: u64) -> String {
    crate::commands::status::edit_uint64_with_suffix(v)
}

/// The decimal point of the LC_NUMERIC locale of the environment, which
/// bareos-dir uses after setlocale(LC_ALL, "") ("24154,1 KB/s" on a German
/// system).
fn radix() -> char {
    static RADIX: std::sync::OnceLock<char> = std::sync::OnceLock::new();
    *RADIX.get_or_init(|| unsafe {
        let empty = std::ffi::CString::new("").unwrap();
        let locale = libc::newlocale(libc::LC_NUMERIC_MASK, empty.as_ptr(), std::ptr::null_mut());
        if locale.is_null() {
            return '.';
        }
        let point = libc::nl_langinfo_l(libc::RADIXCHAR, locale);
        let c = if point.is_null() {
            '.'
        } else {
            std::ffi::CStr::from_ptr(point).to_str().ok().and_then(|s| s.chars().next()).unwrap_or('.')
        };
        libc::freelocale(locale);
        c
    })
}

/// printf("%.1f") in the process locale.
pub(crate) fn fixed1(v: f64) -> String {
    format!("{v:.1}").replace('.', &radix().to_string())
}

/// JobstatusToAscii() (lib/util.cc).
pub fn job_status_to_ascii(status: char) -> String {
    match status {
        'C' => "Created", 'R' => "Running", 'B' => "Blocked", 'T' => "OK",
        'I' => "Error: incomplete job", 'f' => "Fatal Error", 'E' => "Error",
        'e' => "Non-fatal error", 'W' => "OK -- with warnings", 'A' => "Canceled",
        'D' => "Verify differences", 'F' => "Waiting on FD", 'S' => "Wait on SD",
        'm' => "Wait for new Volume", 'M' => "Waiting for mount",
        's' => "Waiting for Storage resource", 'j' => "Waiting for Job resource",
        'c' => "Waiting for Client resource", 'd' => "Waiting on Max Jobs",
        't' => "Waiting for Start Time", 'p' => "Waiting on Priority",
        'L' => "SD committing Data", 'l' => "SD despooling Data",
        'a' => "SD despooling Attributes", 'i' => "Dir inserting Attributes",
        '\0' => "",
        other => return format!("Unknown Job termination status={}", other as u32),
    }.to_string()
}

/// The bus message type of an M_* code (inverse of `audit::code`).
pub fn msg_type_of(code: u32) -> Option<MsgType> {
    [MsgType::Fatal, MsgType::Error, MsgType::Warning, MsgType::Info, MsgType::Saved, MsgType::Skipped,
     MsgType::Mount, MsgType::Restored, MsgType::Audit].into_iter().find(|t| crate::audit::code(*t) == code)
}

/// JobTrigger of a job started by a console (`run`, `restore`, ...).
pub const TRIGGER_USER: &str = "User";

// --- job start ------------------------------------------------------------------

/// job.cc:430 JobThread(): "Version: %s (%s) %s\n".
pub fn version_text() -> String {
    format!("Version: {} ({}) {}\n", version::FULL, version::DATE, version::os_info())
}

/// backup.cc:505 DoNativeBackup().
pub fn start_backup_text(job_id: u32, job: &str) -> String {
    format!("Start Backup JobId {job_id}, Job={job}\n")
}

/// restore.cc:405 DoNativeRestore().
pub fn start_restore_text(job: &str) -> String {
    format!("Start Restore Job {job}\n")
}

/// admin.cc:58 DoAdmin().
pub fn start_admin_text(job_id: u32, job: &str) -> String {
    format!("Start Admin JobId {job_id}, Job={job}\n")
}

/// cats/sql_find.cc FindJobStartTime(): no Full before an Incremental or
/// Differential (the strerror() GetLevelSinceTime() reports).
pub const NO_PRIOR_FULL_TEXT: &str = "No prior Full backup Job record found.\n";

/// job.cc:1133 GetLevelSinceTime(): the level is upgraded to Full.
pub const UPGRADE_TO_FULL_TEXT: &str = "No prior or suitable Full backup found in catalog. Doing FULL backup.\n";

/// `jcr->dir_impl->since` of an upgraded job: " (upgraded from %s)".
pub fn upgraded_since(level: &str) -> String {
    format!(" (upgraded from {level})")
}

/// `jcr->dir_impl->since` with the start time of the reference job.
pub fn since_text(since_time: &str) -> String {
    format!(", since={since_time}")
}

// --- daemon connections -----------------------------------------------------------

/// lib/tls_openssl.cc TlsLogConninfo() / lib/bsock.cc:588 (Qmsg M_INFO):
/// `who` is "Storage daemon" or "Client: <name>".
pub fn connected_text(who: &str, host: &str, port: u16, cipher: Option<&str>) -> String {
    match cipher {
        Some(cipher) => format!("Connected {who} at {host}:{port}, encryption: {}\n",
                                if cipher.is_empty() { "Unknown" } else { cipher }),
        None => format!("Connected {who} at {host}:{port}, encryption: None\n"),
    }
}

/// BareosSocket::GetCipherMessageString() plus newline (sd_cmds.cc:152,
/// fd_cmds.cc:181 SendInfoChosenCipher).
pub fn cipher_text(cipher: Option<&str>) -> String {
    format!("{}\n", bareos_bnet::tls_psk::cipher_message_string(cipher))
}

/// fd_cmds.cc:174 OutputMessageForConnectionTry(): first connection to a
/// client since the configuration was loaded.
pub const PROBING_TEXT: &str = "Probing client protocol... (result will be saved until config reload)\n";

/// fd_cmds.cc:190 SendInfoSuccess() for an immediate TLS handshake; `probed`
/// is the first connection (the "\r\v" prefix, blanked for the job log).
pub fn handshake_text(probed: bool) -> String {
    let m = format!("{} Handshake: Immediate TLS,", if probed { "\r\u{b}" } else { "" });
    format!("{}\n", m.replace(['\r', '\u{b}', ','], " "))
}

/// UnbashSpaces(): 0x01 back to blanks.
pub fn unbash_spaces(s: &str) -> String {
    s.replace('\u{1}', " ")
}

/// msgchan.cc:259/183: the device the Storage daemon accepted.
pub fn using_device_text(device: &str, write: bool) -> String {
    format!("Using Device \"{device}\" to {}.\n", if write { "write" } else { "read" })
}

// --- end of job -----------------------------------------------------------------------

/// What kind of job report the job gets at its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReportKind {
    #[default]
    Backup,
    Restore,
    /// AdminCleanup() (C-058).
    Admin,
    /// ConsolidateCleanup() (C-058).
    Consolidate,
    /// MigrationCleanup() of a Copy (true) or Migrate control job (C-058).
    MigrateControl(bool),
    /// MigrationCleanup() of a Copy (true) or Migrate worker job: the report
    /// comes from migrate::cleanup (C-058, S-05).
    MigrateWorker(bool),
}

/// Job data known when the job is started (the JCR resources and their
/// sources, job.cc SetJcrDefaults/ApplyPoolOverrides, ua_run.cc).
#[derive(Debug, Clone, Default)]
pub struct JobReport {
    pub kind: ReportKind,
    pub job: String,
    pub level: String,
    pub since: String,
    pub client: String,
    pub fileset: String,
    pub pool: String,
    pub pool_source: String,
    pub catalog: String,
    pub catalog_source: String,
    pub storage: String,
    pub storage_source: String,
    pub sched_time: i64,
    pub start_time: i64,
    pub priority: i32,
    pub allow_mixed_priority: bool,
    pub accurate: bool,
    pub trigger: String,
    /// Restore: files of the selection (`jcr->dir_impl->ExpectedFiles`);
    /// None when the restore does not know it.
    pub expected_files: Option<u64>,
    /// Copy/Migrate control job: jobs were selected (HasSelectedJobs).
    pub selected: bool,
    /// A VirtualFull backup (vbackup.rs): NativeVbackupCleanup data.
    pub vf: Option<crate::vbackup::VfInfo>,
}

/// Clients connected since the Director started (ClientResource
/// `connection_successful_handshake_`).
static CONNECTED_CLIENTS: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// True for the first job connection to client `name` (then the Director
/// probes the client protocol).
pub fn first_client_connection(name: &str) -> bool {
    CONNECTED_CLIENTS.lock().unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Default::default).insert(name.to_string())
}

pub(crate) fn raw_directive(config: &crate::director_config::DirectorConfig, kind: &str, name: &str, key: &str) -> Option<String> {
    config.raw.iter().find(|r| r.norm_type() == kind && r.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))?
        .directives.iter().rev().find(|d| d.norm_key() == key).and_then(|d| d.first()).map(str::to_string)
}

/// The report data of a backup started by a console: `pool` is the pool
/// used, `level_pool` the one of the Job resource for the effective level
/// (`level` "F", "I" or "D"); `storage` the storage used.
#[allow(clippy::too_many_arguments)]
pub fn backup_report(config: &crate::director_config::DirectorConfig, job: &bareos_config::resources::JobResource,
                     client: &bareos_config::resources::ClientResource, storage: &str, pool: &str,
                     level_pool: &str, level: &str, fileset: &str, now: i64) -> JobReport {
    // job.cc:1229-1303 ApplyPoolOverrides, ua_run.cc:818 "command line"
    let pool_override = match level {
        "F" if !job.full_backup_pool.is_empty() => Some("Job FullPool override"),
        "I" if !job.incremental_backup_pool.is_empty() => Some("Job IncPool override"),
        "D" if !job.differential_backup_pool.is_empty() => Some("Job DiffPool override"),
        _ => None,
    };
    let pool_source = if pool != level_pool { "command line" } else { pool_override.unwrap_or("Job resource") };
    // job.cc:1707-1722 SetJcrDefaults, 1315 ApplyPoolOverrides
    let job_dirs = crate::commands::defaults::job_directives(config, &job.name);
    let job_catalog = job_dirs.iter().rev().find(|d| d.norm_key() == "catalog").and_then(|d| d.first()).map(str::to_string);
    let (catalog, catalog_source) = if let Some(catalog) = raw_directive(config, "pool", pool, "catalog") {
        (catalog, "Pool resource")
    } else if let Some(catalog) = job_catalog {
        (catalog, "Job resource")
    } else if !client.catalog.is_empty() {
        (client.catalog.clone(), "Client resource")
    } else {
        (config.catalogs.first().map(|c| c.name.clone()).unwrap_or_default(), "Client resource")
    };
    // job.cc:1617-1641 GetJobStorage, ua_run.cc:2168 "command line"
    let storage_source = if !storage.eq_ignore_ascii_case(&job.storage) {
        "command line"
    } else if raw_directive(config, "pool", pool, "storage").is_some() {
        "Pool resource"
    } else {
        "Job resource"
    };
    JobReport {
        kind: ReportKind::Backup,
        client: client.name.clone(),
        fileset: fileset.to_string(),
        pool: pool.to_string(),
        pool_source: pool_source.to_string(),
        catalog,
        catalog_source: catalog_source.to_string(),
        storage: storage.to_string(),
        storage_source: storage_source.to_string(),
        sched_time: now,
        start_time: now,
        priority: job.priority,
        allow_mixed_priority: crate::commands::defaults::bool_directive(&job_dirs, "allowmixedpriority", false),
        accurate: crate::commands::defaults::bool_directive(&job_dirs, "accurate", false),
        trigger: TRIGGER_USER.to_string(),
        ..Default::default()
    }
}

/// Job data known when the job has ended.
#[derive(Debug, Clone, Default)]
pub struct JobOutcome {
    pub job_id: u32,
    /// Final JobStatus (after JS_Warnings promotion).
    pub status: char,
    pub end_time: i64,
    pub files: u64,
    pub bytes: u64,
    pub read_bytes: u64,
    pub sd_files: u64,
    pub sd_bytes: u64,
    pub errors: u32,
    pub sd_errors: u32,
    pub vol_session_id: i32,
    pub vol_session_time: i32,
    pub fd_status: char,
    pub sd_status: char,
    pub vss: bool,
    pub encrypt: bool,
}

/// One Jmsg of the job report sequence.
pub type JobMsg = (MsgType, String);

fn inappropriate(status: char) -> String {
    format!("Inappropriate term code: {status}\n")
}

/// NativeBackupCleanup() term message and message type.
pub fn backup_term_msg(status: char) -> (MsgType, String) {
    match status {
        'T' => (MsgType::Info, "Backup OK".into()),
        'I' => (MsgType::Info, "Backup failed -- incomplete".into()),
        'W' => (MsgType::Info, "Backup OK -- with warnings".into()),
        'f' | 'E' => (MsgType::Error, "*** Backup Error ***".into()),
        'A' => (MsgType::Info, "Backup Canceled".into()),
        other => (MsgType::Info, inappropriate(other)),
    }
}

/// NativeRestoreCleanup() term message and message type.
pub fn restore_term_msg(status: char) -> (MsgType, String) {
    match status {
        'T' => (MsgType::Info, "Restore OK".into()),
        'W' => (MsgType::Info, "Restore OK -- with warnings".into()),
        'f' | 'E' => (MsgType::Error, "*** Restore Error ***".into()),
        'A' => (MsgType::Info, "Restore Canceled".into()),
        other => (MsgType::Info, inappropriate(other)),
    }
}

/// AdminCleanup() term message and message type.
pub fn admin_term_msg(status: char) -> (MsgType, String) {
    match status {
        'T' => (MsgType::Info, "Admin OK".into()),
        'f' | 'E' => (MsgType::Error, "*** Admin Error ***".into()),
        'A' => (MsgType::Info, "Admin Canceled".into()),
        other => (MsgType::Info, inappropriate(other)),
    }
}

/// Catalog values the summaries read (GetClientRecord, GetJobVolumeNames,
/// GetMediaRecord, the FileSet record of the job).
#[derive(Debug, Clone)]
pub struct CatalogFacts {
    /// Client Uname, Err with the strerror() text.
    pub uname: Result<String, String>,
    pub fs_create_time: String,
    /// VolumeNames joined by "|", Err with the strerror() text.
    pub volume_names: Result<String, String>,
    /// VolBytes of the last volume, Err with the strerror() text.
    pub last_vol_bytes: Result<u64, String>,
}

fn first_cell(catalog: &CatalogHandle, sql: &str) -> Result<Option<Vec<Option<String>>>, String> {
    Ok(catalog.query_rows(sql)?.rows.into_iter().next())
}

/// Read the [`CatalogFacts`] of a job with the upstream queries.
pub fn catalog_facts(catalog: Option<&CatalogHandle>, job_id: u32, client: &str) -> CatalogFacts {
    let Some(catalog) = catalog else {
        return CatalogFacts {
            uname: Err("Client record not found in Catalog.\n".into()),
            fs_create_time: String::new(),
            volume_names: Err(format!("No Volume for JobId {job_id} found in Catalog.\n")),
            last_vol_bytes: Ok(0),
        };
    };
    let uname = match first_cell(catalog, &format!(
        "SELECT ClientId,Name,Uname,AutoPrune,FileRetention,JobRetention FROM Client WHERE Client.Name='{}'", esc(client))) {
        Ok(Some(row)) => Ok(row.get(2).cloned().flatten().unwrap_or_default()),
        _ => Err("Client record not found in Catalog.\n".to_string()),
    };
    let fs_create_time = first_cell(catalog, &format!(
        "SELECT FileSet.CreateTime FROM Job, FileSet WHERE Job.JobId={job_id} AND FileSet.FileSetId=Job.FileSetId"))
        .ok().flatten().and_then(|row| row.into_iter().next().flatten()).unwrap_or_default();
    let volume_names = match catalog.query_rows(&format!(
        "SELECT VolumeName,MAX(VolIndex) FROM JobMedia,Media WHERE JobMedia.JobId={job_id} \
         AND JobMedia.MediaId=Media.MediaId GROUP BY VolumeName ORDER BY 2 ASC")) {
        Ok(table) if table.rows.is_empty() => Err(format!("No volumes found for JobId={job_id}\n")),
        Ok(table) => Ok(table.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>().join("|")),
        Err(_) => Err(format!("No Volume for JobId {job_id} found in Catalog.\n")),
    };
    let last_vol_bytes = match &volume_names {
        Ok(names) if !names.is_empty() => {
            let last = names.rsplit('|').next().unwrap_or(names);
            match first_cell(catalog, &format!("SELECT VolBytes FROM Media WHERE VolumeName='{}'", esc(last))) {
                Ok(Some(row)) => Ok(row.into_iter().next().flatten().and_then(|v| v.parse().ok()).unwrap_or(0)),
                _ => Err(format!("Media record for Volume \"{last}\" not found.\n")),
            }
        }
        _ => Ok(0),
    };
    CatalogFacts { uname, fs_create_time, volume_names, last_vol_bytes }
}

/// The header line of the summaries: "%s %s %s (%s):\n" (BAREOS, my_name,
/// Full, ShortDate).
pub(crate) fn summary_head(director: &str) -> String {
    format!("Bareos {director} {} ({}):\n", version::FULL, version::SHORT_DATE)
}

/// NativeBackupCleanup() from the Client record warning on, UpdateBootstrap
/// excluded, and GenerateBackupSummary() (backup.cc:780-1219) for a native
/// backup without quotas, base jobs or secure erase.
pub fn backup_summary(director: &str, report: &JobReport, outcome: &JobOutcome, facts: &CatalogFacts) -> Vec<JobMsg> {
    let mut messages = Vec::new();
    let uname = match &facts.uname {
        Ok(uname) => uname.clone(),
        Err(error) => {
            messages.push((MsgType::Warning, format!("Error getting Client record for Job report: ERR={error}")));
            String::new()
        }
    };
    let (msg_type, term_msg) = backup_term_msg(outcome.status);
    // A VirtualFull reports the times of the newest consolidated job
    // (NativeVbackupCleanup sets them before GetJobRecord).
    let (start_time, end_time) = match report.vf.as_ref().and_then(|vf| vf.previous.as_ref()) {
        Some((start, end, _)) => (crate::btime::str_to_utime(start), crate::btime::str_to_utime(end)),
        None => (report.start_time, outcome.end_time),
    };
    let run_time = end_time - start_time;
    let kbps = if run_time <= 0 { 0.0 } else { outcome.bytes as f64 / (1000.0 * run_time as f64) };
    let terminated_ok = matches!(outcome.status, 'T' | 'W');
    let volume_names = match &facts.volume_names {
        Ok(names) => names.clone(),
        Err(error) => {
            if terminated_ok && outcome.bytes != 0 {
                messages.push((MsgType::Error, error.clone()));
            }
            String::new()
        }
    };
    let last_vol_bytes = if volume_names.is_empty() {
        0
    } else {
        match &facts.last_vol_bytes {
            Ok(bytes) => *bytes,
            Err(error) => {
                let last = volume_names.rsplit('|').next().unwrap_or(&volume_names);
                messages.push((MsgType::Warning,
                    format!("Error getting Media record for Volume \"{last}\": ERR={error}")));
                0
            }
        }
    };
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let (level_info, statistics, client_options, daemon_status) = if report.vf.is_some() {
        ("  Backup Level:           Virtual Full\n".to_string(),
         format!("  SD Files Written:       {}\n  SD Bytes Written:       {} ({}B)\n",
            commas(outcome.sd_files), commas(outcome.sd_bytes), suffix(outcome.sd_bytes)),
         String::new(),
         format!("  SD Errors:              {}\n  SD termination status:  {}\n  Accurate:               {}\n",
            outcome.sd_errors, job_status_to_ascii(outcome.sd_status), yes_no(report.accurate)))
    } else {
    let level_info = format!("  Backup Level:           {}{}\n", report.level, report.since);
    let statistics = format!(
        "  FD Files Written:       {}\n  SD Files Written:       {}\n  FD Bytes Read:          {} ({}B)\n\
         \x20 FD Bytes Written:       {} ({}B)\n  SD Bytes Written:       {} ({}B)\n",
        commas(outcome.files), commas(outcome.sd_files), commas(outcome.read_bytes), suffix(outcome.read_bytes),
        commas(outcome.bytes), suffix(outcome.bytes), commas(outcome.sd_bytes), suffix(outcome.sd_bytes));
    let client_options = format!(
        "  Software Compression:   {}\n  VSS:                    {}\n  Encryption:             {}\n  Accurate:               {}\n",
        "None", yes_no(outcome.vss), yes_no(outcome.encrypt), yes_no(report.accurate));
    let daemon_status = format!(
        "  Non-fatal FD errors:    {}\n  SD Errors:              {}\n  FD termination status:  {}\n  SD termination status:  {}\n",
        outcome.errors, outcome.sd_errors, job_status_to_ascii(outcome.fd_status), job_status_to_ascii(outcome.sd_status));
    (level_info, statistics, client_options, daemon_status)
    };
    let text = format!(
        "{head}  OS Information:         {os}\n  JobId:                  {job_id}\n  Job:                    {job}\n\
         {level_info}  Client:                 \"{client}\" {uname}\n  FileSet:                \"{fileset}\" {fs_time}\n\
         \x20 Pool:                   \"{pool}\" (From {pool_source})\n  Catalog:                \"{catalog}\" (From {catalog_source})\n\
         \x20 Storage:                \"{storage}\" (From {storage_source})\n  Scheduled time:         {sched}\n\
         \x20 Start time:             {start}\n  End time:               {end}\n  Elapsed time:           {elapsed}\n\
         \x20 Priority:               {priority}\n  Allow Mixed Priority:   {mixed}\n{statistics}\
         \x20 Rate:                   {rate} KB/s\n{client_options}  Volume name(s):         {volume_names}\n\
         \x20 Volume Session Id:      {vsid}\n  Volume Session Time:    {vstime}\n  Last Volume Bytes:      {lvb} ({lvbs}B)\n\
         {daemon_status}  Bareos binary info:     {binary}\n  Job triggered by:       {trigger}\n  Termination:            {term_msg}\n\n",
        head = summary_head(director), os = version::os_info(), job_id = outcome.job_id, job = report.job,
        client = report.client, fileset = report.fileset, fs_time = facts.fs_create_time,
        pool = report.pool, pool_source = report.pool_source, catalog = report.catalog,
        catalog_source = report.catalog_source, storage = report.storage, storage_source = report.storage_source,
        sched = bstrftimes(report.sched_time), start = bstrftimes(start_time), end = bstrftimes(end_time),
        elapsed = crate::ua_select::edit_utime(run_time.max(0) as u64), priority = report.priority,
        mixed = yes_no(report.allow_mixed_priority), rate = fixed1(kbps), vsid = outcome.vol_session_id,
        vstime = outcome.vol_session_time, lvb = commas(last_vol_bytes), lvbs = suffix(last_vol_bytes),
        binary = version::JOBLOG_MESSAGE, trigger = report.trigger);
    messages.push((msg_type, text));
    messages
}

/// NativeRestoreCleanup() and GenerateRestoreSummary() (restore.cc:419-602)
/// for a native restore without secure erase.
pub fn restore_summary(director: &str, report: &JobReport, outcome: &JobOutcome, facts: &CatalogFacts) -> Vec<JobMsg> {
    let mut messages = Vec::new();
    let mut status = outcome.status;
    if let Some(expected) = report.expected_files.filter(|expected| *expected != outcome.files) {
        messages.push((MsgType::Warning, format!("File count mismatch: expected={expected} , restored={}\n", outcome.files)));
        if status == 'T' {
            status = 'W';
        }
    }
    let (msg_type, term_msg) = restore_term_msg(status);
    let run_time = outcome.end_time - report.start_time;
    let mut kbps = if run_time <= 0 { 0.0 } else { outcome.bytes as f64 / (1000.0 * run_time as f64) };
    if kbps < 0.05 {
        kbps = 0.0;
    }
    let uname = match &facts.uname {
        Ok(uname) => uname.clone(),
        Err(error) => {
            messages.push((MsgType::Warning, format!("Error getting Client record for Job report: ERR={error}")));
            String::new()
        }
    };
    let text = format!(
        "{head}  OS Information:         {os}\n  JobId:                  {job_id}\n  Job:                    {job}\n\
         \x20 Restore Client:         \"{client}\" {uname}\n  Start time:             {start}\n  End time:               {end}\n\
         \x20 Elapsed time:           {elapsed}\n  Files Expected:         {expected}\n  Files Restored:         {files}\n\
         \x20 Bytes Restored:         {bytes}\n  Rate:                   {rate} KB/s\n  FD Errors:              {errors}\n\
         \x20 FD termination status:  {fd}\n  SD termination status:  {sd}\n  Bareos binary info:     {binary}\n\
         \x20 Job triggered by:       {trigger}\n  Termination:            {term_msg}\n\n",
        head = summary_head(director), os = version::os_info(), job_id = outcome.job_id, job = report.job,
        client = report.client, start = bstrftimes(report.start_time), end = bstrftimes(outcome.end_time),
        elapsed = crate::ua_select::edit_utime(run_time.max(0) as u64),
        expected = commas(report.expected_files.unwrap_or(0)), files = commas(outcome.files),
        bytes = commas(outcome.bytes), rate = fixed1(kbps as f32 as f64), errors = outcome.errors,
        fd = job_status_to_ascii(outcome.fd_status), sd = job_status_to_ascii(outcome.sd_status),
        binary = version::JOBLOG_MESSAGE, trigger = report.trigger);
    messages.push((msg_type, text));
    messages
}

/// AdminCleanup() summary (admin.cc:105-121).
pub fn admin_summary(report: &JobReport, outcome: &JobOutcome) -> JobMsg {
    let (msg_type, term_msg) = admin_term_msg(outcome.status);
    let edt = bstrftimes(outcome.end_time);
    (msg_type, format!(
        "BAREOS {} ({}): {edt}\n  JobId:                  {}\n  Job:                    {}\n  Scheduled time:         {}\n\
         \x20 Start time:             {}\n  End time:               {edt}\n  Bareos binary info:     {}\n\
         \x20 Job triggered by:       {}\n  Termination:            {term_msg}\n\n",
        version::FULL, version::SHORT_DATE, outcome.job_id, report.job, bstrftimes(report.sched_time),
        bstrftimes(report.start_time), version::JOBLOG_MESSAGE, report.trigger))
}

/// The AdminCleanup()-shaped report of ConsolidateCleanup() (consolidate.cc).
pub fn consolidate_summary(report: &JobReport, outcome: &JobOutcome) -> JobMsg {
    let (msg_type, term_msg) = match outcome.status {
        'T' => (MsgType::Info, "Consolidate OK".to_string()),
        'f' | 'E' => (MsgType::Error, "*** Consolidate Error ***".to_string()),
        'A' => (MsgType::Info, "Consolidate Canceled".to_string()),
        other => (MsgType::Info, inappropriate(other)),
    };
    let (_, text) = admin_summary(report, outcome);
    let admin_term = admin_term_msg(outcome.status).1;
    let text = text.replacen(&format!("  Termination:            {admin_term}\n"),
        &format!("  Termination:            {term_msg}\n"), 1);
    (msg_type, text)
}

/// MigrationCleanup() + GenerateMigrateSummary() of a control job that only
/// selected jobs (no mig_jcr): "%s OK" etc. with HasSelectedJobs, else
/// "%s -- no files to %s" (migrate.cc:1813-1846, 1622-1645).
pub fn migrate_selection_summary(director: &str, report: &JobReport, outcome: &JobOutcome, copy: bool) -> JobMsg {
    let (operation, action) = if copy { ("Copying", "copy") } else { ("Migration", "migrate") };
    let (msg_type, term_msg) = if report.selected {
        match outcome.status {
            'T' => (MsgType::Info, format!("{operation} OK")),
            'W' => (MsgType::Info, format!("{operation} OK -- with warnings")),
            'f' | 'E' => (MsgType::Error, format!("*** {operation} Error ***")),
            'A' => (MsgType::Info, format!("{operation} Canceled")),
            _ => (MsgType::Info, format!("Inappropriate {operation} term code")),
        }
    } else {
        (MsgType::Info, format!("{operation} -- no files to {action}"))
    };
    let run_time = (outcome.end_time - report.start_time).max(0) as u64;
    (msg_type, format!(
        "{head}  OS Information:         {os}\n  Current JobId:          {job_id}\n  Current Job:            {job}\n\
         \x20 Catalog:                \"{catalog}\" (From {catalog_source})\n  Start time:             {start}\n\
         \x20 End time:               {end}\n  Elapsed time:           {elapsed}\n  Priority:               {priority}\n\
         \x20 Bareos binary info:     {binary}\n  Job triggered by:       {trigger}\n  Termination:            {term_msg}\n\n",
        head = summary_head(director), os = version::os_info(), job_id = outcome.job_id, job = report.job,
        catalog = report.catalog, catalog_source = report.catalog_source, start = bstrftimes(report.start_time),
        end = bstrftimes(outcome.end_time), elapsed = crate::ua_select::edit_utime(run_time),
        priority = report.priority, binary = version::JOBLOG_MESSAGE, trigger = report.trigger))
}

/// The end-of-job messages of a job: reads the catalog facts and builds the
/// summary of its kind.
pub fn end_of_job(director: &str, catalog: Option<&CatalogHandle>, report: &JobReport, outcome: &JobOutcome) -> Vec<JobMsg> {
    match report.kind {
        ReportKind::Admin => return vec![admin_summary(report, outcome)],
        ReportKind::Consolidate => return vec![consolidate_summary(report, outcome)],
        ReportKind::MigrateControl(copy) => return vec![migrate_selection_summary(director, report, outcome, copy)],
        ReportKind::MigrateWorker(_) => return Vec::new(),
        _ => {}
    }
    let facts = catalog_facts(catalog, outcome.job_id, &report.client);
    match report.kind {
        ReportKind::Admin | ReportKind::Consolidate | ReportKind::MigrateControl(_) | ReportKind::MigrateWorker(_) => unreachable!(),
        ReportKind::Backup => backup_summary(director, report, outcome, &facts),
        ReportKind::Restore => restore_summary(director, report, outcome, &facts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlColumn, SqlTable};

    fn table(columns: &[&str], rows: Vec<Vec<&str>>) -> CatalogResponse {
        CatalogResponse::Rows(SqlTable {
            columns: columns.iter().map(|c| SqlColumn { name: c.to_string(), numeric: false }).collect(),
            rows: rows.into_iter().map(|r| r.into_iter().map(|v| Some(v.to_string())).collect()).collect(),
        })
    }

    /// Catalog double answering the report queries of job 626 of the parity
    /// catalog (`list joblog jobid=626`).
    fn catalog_626(log: std::sync::Arc<Mutex<Vec<String>>>) -> CatalogHandle {
        CatalogHandle::mock(move |request| {
            let CatalogRequest::TextRows { sql } = request else { return CatalogResponse::Error("unexpected".into()) };
            log.lock().unwrap().push(sql.clone());
            if sql.starts_with("SELECT ClientId,Name,Uname") {
                table(&["ClientId", "Name", "Uname", "AutoPrune", "FileRetention", "JobRetention"],
                      vec![vec!["1", "bareos-fd", "25.0.4~pre179.6e6f5b19f (16Jun26) Debian GNU/Linux 13 (trixie),debian",
                                "1", "5184000", "15552000"]])
            } else if sql.starts_with("SELECT FileSet.CreateTime") {
                table(&["createtime"], vec![vec!["2026-03-21 23:44:48"]])
            } else if sql.starts_with("SELECT VolumeName,MAX(VolIndex)") {
                table(&["volumename", "max"], vec![vec!["Incremental-47322", "1"]])
            } else if sql.starts_with("SELECT VolBytes") {
                table(&["volbytes"], vec![vec!["366260149"]])
            } else {
                table(&[], vec![])
            }
        })
    }

    fn local(text: &str) -> i64 {
        let t = chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").unwrap();
        use chrono::TimeZone;
        chrono::Local.from_local_datetime(&t).single().unwrap().timestamp()
    }

    fn report_626() -> JobReport {
        JobReport {
            kind: ReportKind::Backup,
            job: "backup-bareos-fd.2026-07-09_21.00.00_02".into(),
            level: "Incremental".into(),
            since: since_text("2026-06-26 21:00:03"),
            client: "bareos-fd".into(),
            fileset: "SelfTest".into(),
            pool: "Incremental".into(),
            pool_source: "Job IncPool override".into(),
            catalog: "MyCatalog".into(),
            catalog_source: "Client resource".into(),
            storage: "File".into(),
            storage_source: "Job resource".into(),
            sched_time: local("2026-07-09 21:00:00"),
            start_time: local("2026-07-09 21:00:02"),
            priority: 10,
            allow_mixed_priority: false,
            accurate: false,
            trigger: "Scheduler".into(),
            expected_files: None,
            selected: false,
            vf: None,
        }
    }

    fn outcome_626() -> JobOutcome {
        JobOutcome {
            job_id: 626, status: 'T', end_time: local("2026-07-09 21:00:03"), files: 5, bytes: 24_154_072,
            read_bytes: 24_154_072, sd_files: 5, sd_bytes: 24_154_607, errors: 0, sd_errors: 0,
            vol_session_id: 1, vol_session_time: 1_783_588_680, fd_status: 'T', sd_status: 'T',
            vss: false, encrypt: false,
        }
    }

    #[test]
    fn backup_summary_matches_the_original_job_log() {
        // `list joblog jobid=626` of the parity catalog (written by bareos-dir
        // 25.0.4 under a German locale; the version, OS and binary lines are
        // the own ones, the rate uses the locale's decimal point).
        let expected = format!("Bareos bareos-dir {} ({}):
  OS Information:         {}
  JobId:                  626
  Job:                    backup-bareos-fd.2026-07-09_21.00.00_02
  Backup Level:           Incremental, since=2026-06-26 21:00:03
  Client:                 \"bareos-fd\" 25.0.4~pre179.6e6f5b19f (16Jun26) Debian GNU/Linux 13 (trixie),debian
  FileSet:                \"SelfTest\" 2026-03-21 23:44:48
  Pool:                   \"Incremental\" (From Job IncPool override)
  Catalog:                \"MyCatalog\" (From Client resource)
  Storage:                \"File\" (From Job resource)
  Scheduled time:         {}
  Start time:             {}
  End time:               {}
  Elapsed time:           1 sec
  Priority:               10
  Allow Mixed Priority:   no
  FD Files Written:       5
  SD Files Written:       5
  FD Bytes Read:          24,154,072 (24.15 MB)
  FD Bytes Written:       24,154,072 (24.15 MB)
  SD Bytes Written:       24,154,607 (24.15 MB)
  Rate:                   24154{}1 KB/s
  Software Compression:   None
  VSS:                    no
  Encryption:             no
  Accurate:               no
  Volume name(s):         Incremental-47322
  Volume Session Id:      1
  Volume Session Time:    1783588680
  Last Volume Bytes:      366,260,149 (366.2 MB)
  Non-fatal FD errors:    0
  SD Errors:              0
  FD termination status:  OK
  SD termination status:  OK
  Bareos binary info:     {}
  Job triggered by:       Scheduler
  Termination:            Backup OK

", version::FULL, version::SHORT_DATE, version::os_info(),
            bstrftimes(local("2026-07-09 21:00:00")), bstrftimes(local("2026-07-09 21:00:02")),
            bstrftimes(local("2026-07-09 21:00:03")), radix(), version::JOBLOG_MESSAGE);
        let log = std::sync::Arc::new(Mutex::new(Vec::new()));
        let catalog = catalog_626(log.clone());
        let messages = end_of_job("bareos-dir", Some(&catalog), &report_626(), &outcome_626());
        assert_eq!(messages, vec![(MsgType::Info, expected)]);
        assert_eq!(log.lock().unwrap()[2],
            "SELECT VolumeName,MAX(VolIndex) FROM JobMedia,Media WHERE JobMedia.JobId=626 \
             AND JobMedia.MediaId=Media.MediaId GROUP BY VolumeName ORDER BY 2 ASC");
    }

    #[test]
    fn backup_errors_follow_native_backup_cleanup() {
        let facts = CatalogFacts {
            uname: Err("Client record not found in Catalog.\n".into()),
            fs_create_time: String::new(),
            volume_names: Err("No volumes found for JobId=626\n".into()),
            last_vol_bytes: Ok(0),
        };
        let messages = backup_summary("bareos-dir", &report_626(), &outcome_626(), &facts);
        assert_eq!(messages[0], (MsgType::Warning,
            "Error getting Client record for Job report: ERR=Client record not found in Catalog.\n".to_string()));
        assert_eq!(messages[1], (MsgType::Error, "No volumes found for JobId=626\n".to_string()));
        assert!(messages[2].1.contains("  Client:                 \"bareos-fd\" \n"));
        assert!(messages[2].1.contains("  Volume name(s):         \n"));
        let failed = JobOutcome { status: 'f', fd_status: 'f', sd_status: '\0', ..outcome_626() };
        let messages = backup_summary("bareos-dir", &report_626(), &failed, &facts);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].0, MsgType::Error);
        assert!(messages[1].1.ends_with(&format!("  FD termination status:  Fatal Error\n  SD termination status:  \n  \
            Bareos binary info:     {}\n  Job triggered by:       Scheduler\n  \
            Termination:            *** Backup Error ***\n\n", version::JOBLOG_MESSAGE)));
    }

    #[test]
    fn term_messages_and_status_texts() {
        assert_eq!(backup_term_msg('W'), (MsgType::Info, "Backup OK -- with warnings".into()));
        assert_eq!(backup_term_msg('A'), (MsgType::Info, "Backup Canceled".into()));
        assert_eq!(backup_term_msg('R'), (MsgType::Info, "Inappropriate term code: R\n".into()));
        assert_eq!(restore_term_msg('E'), (MsgType::Error, "*** Restore Error ***".into()));
        assert_eq!(restore_term_msg('I'), (MsgType::Info, "Inappropriate term code: I\n".into()));
        assert_eq!(admin_term_msg('T'), (MsgType::Info, "Admin OK".into()));
        assert_eq!(job_status_to_ascii('q'), "Unknown Job termination status=113");
        assert_eq!(job_status_to_ascii('W'), "OK -- with warnings");
        assert_eq!(job_status_to_ascii('\0'), "");
    }

    #[test]
    fn start_and_connection_texts_match_the_original_job_log() {
        assert_eq!(start_backup_text(626, "backup-bareos-fd.2026-07-09_21.00.00_02"),
                   "Start Backup JobId 626, Job=backup-bareos-fd.2026-07-09_21.00.00_02\n");
        assert_eq!(start_restore_text("RestoreFiles.2026-07-09_21.00.00_02"), "Start Restore Job RestoreFiles.2026-07-09_21.00.00_02\n");
        assert_eq!(start_admin_text(7, "admin.x"), "Start Admin JobId 7, Job=admin.x\n");
        let cipher = "TLS_CHACHA20_POLY1305_SHA256 TLSv1.3";
        assert_eq!(connected_text("Storage daemon", "WS-JOERG.intranet.bernau.family", 9103, Some(cipher)),
                   format!("Connected Storage daemon at WS-JOERG.intranet.bernau.family:9103, encryption: {cipher}\n"));
        assert_eq!(connected_text("Client: bareos-fd", "localhost", 9102, Some(cipher)),
                   format!("Connected Client: bareos-fd at localhost:9102, encryption: {cipher}\n"));
        assert_eq!(connected_text("Storage daemon", "h", 1, None), "Connected Storage daemon at h:1, encryption: None\n");
        assert_eq!(cipher_text(Some(cipher)), format!(" Encryption: {cipher}\n"));
        assert_eq!(handshake_text(true), "   Handshake: Immediate TLS \n");
        assert_eq!(handshake_text(false), " Handshake: Immediate TLS \n");
        assert_eq!(using_device_text("JustInTime Device", true), "Using Device \"JustInTime Device\" to write.\n");
        assert_eq!(using_device_text("FileStorage", false), "Using Device \"FileStorage\" to read.\n");
        assert!(version_text().starts_with(&format!("Version: {} ({}) ", version::FULL, version::DATE)));
        assert_eq!(since_text("2026-06-26 21:00:03"), ", since=2026-06-26 21:00:03");
        assert_eq!(upgraded_since("Incremental"), " (upgraded from Incremental)");
    }

    #[test]
    fn restore_summary_follows_generate_restore_summary() {
        let report = JobReport { kind: ReportKind::Restore, job: "RestoreFiles.2026-07-09_21.00.00_02".into(),
            client: "bareos-fd".into(), start_time: 1000, trigger: TRIGGER_USER.into(),
            expected_files: Some(6), ..Default::default() };
        let outcome = JobOutcome { job_id: 9, status: 'T', end_time: 1001, files: 5, bytes: 49, fd_status: 'T',
            sd_status: 'T', ..Default::default() };
        let facts = CatalogFacts { uname: Ok("u".into()), fs_create_time: String::new(), volume_names: Ok(String::new()), last_vol_bytes: Ok(0) };
        let messages = restore_summary("bareos-dir", &report, &outcome, &facts);
        assert_eq!(messages[0], (MsgType::Warning, "File count mismatch: expected=6 , restored=5\n".to_string()));
        assert_eq!(messages[1].1, format!("Bareos bareos-dir {} ({}):
  OS Information:         {}
  JobId:                  9
  Job:                    RestoreFiles.2026-07-09_21.00.00_02
  Restore Client:         \"bareos-fd\" u
  Start time:             {}
  End time:               {}
  Elapsed time:           1 sec
  Files Expected:         6
  Files Restored:         5
  Bytes Restored:         49
  Rate:                   0{}0 KB/s
  FD Errors:              0
  FD termination status:  OK
  SD termination status:  OK
  Bareos binary info:     {}
  Job triggered by:       User
  Termination:            Restore OK -- with warnings

", version::FULL, version::SHORT_DATE, version::os_info(), bstrftimes(1000), bstrftimes(1001), radix(),
            version::JOBLOG_MESSAGE));
    }

    #[test]
    fn admin_summary_follows_admin_cleanup() {
        let report = JobReport { job: "a.1".into(), sched_time: 10, start_time: 11, trigger: "User".into(), ..Default::default() };
        let outcome = JobOutcome { job_id: 3, status: 'T', end_time: 12, ..Default::default() };
        let (kind, text) = admin_summary(&report, &outcome);
        assert_eq!(kind, MsgType::Info);
        assert_eq!(text, format!("BAREOS {} ({}): {}\n  JobId:                  3\n  Job:                    a.1\n  \
            Scheduled time:         {}\n  Start time:             {}\n  End time:               {}\n  \
            Bareos binary info:     {}\n  Job triggered by:       User\n  Termination:            Admin OK\n\n",
            version::FULL, version::SHORT_DATE, bstrftimes(12), bstrftimes(10), bstrftimes(11), bstrftimes(12),
            version::JOBLOG_MESSAGE));
    }

    #[test]
    fn consolidate_and_migrate_control_summaries() {
        let report = JobReport { job: "c.1".into(), sched_time: 10, start_time: 11, trigger: "User".into(),
            catalog: "MyCatalog".into(), catalog_source: "Client resource".into(), priority: 10, ..Default::default() };
        let outcome = JobOutcome { job_id: 3, status: 'T', end_time: 12, ..Default::default() };
        let (kind, text) = consolidate_summary(&report, &outcome);
        assert_eq!(kind, MsgType::Info);
        assert!(text.ends_with("  Termination:            Consolidate OK\n\n"), "{text}");
        assert_eq!(consolidate_summary(&report, &JobOutcome { status: 'E', ..outcome.clone() }).0, MsgType::Error);
        let (kind, text) = migrate_selection_summary("dir", &report, &outcome, false);
        assert_eq!(kind, MsgType::Info);
        assert_eq!(text, format!("Bareos dir {} ({}):\n  OS Information:         {}\n  Current JobId:          3\n  \
            Current Job:            c.1\n  Catalog:                \"MyCatalog\" (From Client resource)\n  \
            Start time:             {}\n  End time:               {}\n  Elapsed time:           1 sec\n  \
            Priority:               10\n  Bareos binary info:     {}\n  Job triggered by:       User\n  \
            Termination:            Migration -- no files to migrate\n\n",
            version::FULL, version::SHORT_DATE, version::os_info(), bstrftimes(11), bstrftimes(12),
            version::JOBLOG_MESSAGE));
        let selected = JobReport { selected: true, ..report };
        assert!(migrate_selection_summary("dir", &selected, &outcome, true).1.ends_with("Termination:            Copying OK\n\n"));
    }

    #[test]
    fn log_insert_follows_dir_db_log_insert() {
        assert_eq!(log_insert_sql(626, local("2026-07-09 21:00:02"), "bareos-dir JobId 626: it's\n"),
                   "INSERT INTO Log (JobId, Time, LogText) VALUES (626,'2026-07-09 21:00:02','bareos-dir JobId 626: it''s\n')");
        set_catalog(None);
        assert!(!log_insert(1, 1, "x"));
    }
}
