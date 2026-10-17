//! Verify jobs: port of dird/verify.cc (DoVerifyInit, DoVerify,
//! VerifyCleanup, GetAttributesAndCompareToCatalog, MissingHandler) and
//! fd_cmds.cc GetAttributesAndPutInCatalog() for Level=InitCatalog.
//!
//! The job is split like the other jobs of this Director:
//! * `run_verify` (console side, when the job is queued): the Job record,
//!   the previous job (FindLastJobid/GetJobRecord), the bootstrap of a
//!   VolumeToCatalog (CreateRestoreBootstrapFile) and the start messages;
//! * `run_worker` (forked job worker): SD (VolumeToCatalog only) and FD
//!   protocol, the attribute stream is forwarded to the Director;
//! * `event` / `finish` (Director, job monitor): catalog comparison or
//!   catalog insert, the missing files, the job end values;
//! * `summary`: the VerifyCleanup() report.
//!
//! Compatibility log C-058 (S-04).

use std::collections::HashMap;
use std::sync::Mutex;

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};
use bareos_bnet::tls_psk::PskStream;
use serde::{Deserialize, Serialize};

use crate::catalog_worker::CatalogHandle;
use crate::catreq::esc;
use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::commands::bvfs::Db;
use crate::job_worker::{JobParams, PipeMessage, WorkerJobType, WorkerPipes};
use crate::message_bus::MsgType;

/// include/job_level.h verify levels.
pub const L_VERIFY_CATALOG: u8 = b'C';
pub const L_VERIFY_INIT: u8 = b'V';
pub const L_VERIFY_VOLUME_TO_CATALOG: u8 = b'O';
pub const L_VERIFY_DISK_TO_CATALOG: u8 = b'd';
pub const L_VERIFY_DATA: u8 = b'A';

const STREAM_UNIX_ATTRIBUTES: i64 = 1;
const STREAM_MD5_DIGEST: i64 = 3;
const STREAM_UNIX_ATTRIBUTES_EX: i64 = 5;
const STREAM_SHA1_DIGEST: i64 = 10;
const STREAM_SHA256_DIGEST: i64 = 17;
const STREAM_SHA512_DIGEST: i64 = 18;
const STREAM_RESTORE_OBJECT: i64 = 28;
const STREAM_XXH128_DIGEST: i64 = 40;

/// lib/message.h M_FATAL.
const M_FATAL: u32 = 3;

/// What the job worker needs besides the common [`JobParams`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyParams {
    /// The job level code (L_VERIFY_*).
    pub level: char,
    /// DoVerify() failed before contacting a daemon (the fatal message was
    /// already sent): the job ends JS_FatalError.
    pub failed: bool,
    /// VolumeToCatalog without files: VerifyCleanup(JS_Terminated) without
    /// daemon contact.
    pub nothing_to_do: bool,
    /// SendIncludeExcludeLists() messages (empty: nothing is sent).
    pub fileset: Vec<String>,
    pub fileset_name: String,
    /// Read storage (VolumeToCatalog).
    pub storage_name: String,
    pub pool_type: String,
    /// The bootstrap file contents (VolumeToCatalog).
    pub bootstrap: String,
    /// The job messages of job_thread()/DoVerify() before the daemon
    /// contact (M_* code, text): sent when the job leaves the queue.
    pub start_messages: Vec<(u32, String)>,
}

/// Verify events of the worker for the Director.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Event {
    /// One FD attribute/digest message (hex encoded, binary).
    #[serde(rename = "attr")]
    Attr { data: String },
    /// The FD ended the attribute stream (BgetDirmsg() < 0); `error`: the
    /// socket error (IsBnetError) text.
    #[serde(rename = "attrs_done")]
    AttrsDone { error: Option<String> },
    /// WaitForJobTermination(): FDJobStatus ('\0' when the FD sent no end
    /// status) and ReadBytes.
    #[serde(rename = "fd_end")]
    FdEnd { status: u32, read_bytes: u64 },
}

/// Messages of the Director (Jmsg type, text without header).
pub type Messages = Vec<(MsgType, String)>;

// ---------------------------------------------------------------------------
// Director-side job state
// ---------------------------------------------------------------------------

/// The catalog record GetFileRecord() found (FileDbRecord).
#[derive(Default)]
struct FileRecord {
    file_id: String,
    lstat: String,
    digest: String,
}

/// The attribute GetAttributesAndPutInCatalog() keeps (jcr->ar).
#[derive(Default)]
struct CachedAttr {
    file_index: i64,
    fname: Vec<u8>,
    attr: Vec<u8>,
    digest: Option<String>,
}

/// The JCR parts of a running verify job (director_jcr_impl.h).
#[derive(Default)]
struct Session {
    level: u8,
    job_id: u32,
    /// prev_jr (JobId and ClientId) of Catalog/VolumeToCatalog/DiskToCatalog.
    prev_job_id: u32,
    prev_client_id: String,
    /// jcr->dir_impl->res.verify_job name ("" without).
    verify_job: String,
    expected_files: u64,
    /// jcr->dir_impl->fname / fn_printed / FileIndex.
    fname: Vec<u8>,
    fn_printed: bool,
    file_index: i64,
    digest_wanted: bool,
    fdbr: FileRecord,
    /// The attribute loop stopped (goto bail_out).
    bailed: bool,
    /// Director-side JobErrors (Jmsg M_FATAL/M_ERROR).
    errors: u32,
    /// FDJobStatus ('\0': none).
    fd_status: char,
    read_bytes: u64,
    cached: Option<CachedAttr>,
}

static SESSIONS: Mutex<Option<HashMap<u32, Session>>> = Mutex::new(None);

fn with_session<T>(job_id: u32, f: impl FnOnce(&mut Session) -> T) -> Option<T> {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    sessions.get_or_insert_with(HashMap::new).get_mut(&job_id).map(f)
}

fn add_session(session: Session) {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    sessions.get_or_insert_with(HashMap::new).insert(session.job_id, session);
}

fn take_session(job_id: u32) -> Option<Session> {
    let mut sessions = SESSIONS.lock().unwrap_or_else(|e| e.into_inner());
    sessions.get_or_insert_with(HashMap::new).remove(&job_id)
}

/// True for a job this module runs (its report is VerifyCleanup()).
pub fn is_verify_job(job_id: u32) -> bool {
    with_session(job_id, |_| ()).is_some()
}

/// Jmsg(M_FATAL): JobErrors becomes at least 1 (lib/message.cc).
fn fatal(session: &mut Session, msgs: &mut Messages, text: String) -> Option<char> {
    if session.errors == 0 {
        session.errors = 1;
    }
    msgs.push((MsgType::Fatal, text));
    Some('f')
}

// ---------------------------------------------------------------------------
// Levels
// ---------------------------------------------------------------------------

/// JobLevelToString() of the verify levels.
pub fn level_name(level: u8) -> &'static str {
    match level {
        L_VERIFY_INIT => "InitCatalog",
        L_VERIFY_CATALOG => "Catalog",
        L_VERIFY_VOLUME_TO_CATALOG => "VolumeToCatalog",
        L_VERIFY_DISK_TO_CATALOG => "DiskToCatalog",
        L_VERIFY_DATA => "Data",
        _ => "",
    }
}

fn level_from_name(name: &str) -> Option<u8> {
    [L_VERIFY_INIT, L_VERIFY_CATALOG, L_VERIFY_VOLUME_TO_CATALOG, L_VERIFY_DISK_TO_CATALOG, L_VERIFY_DATA]
        .into_iter().find(|l| level_name(*l).eq_ignore_ascii_case(name))
}

/// The `verify level=` word of verify.cc.
fn fd_level(level: u8) -> &'static str {
    match level {
        L_VERIFY_INIT => "init",
        L_VERIFY_CATALOG => "catalog",
        L_VERIFY_VOLUME_TO_CATALOG => "volume",
        L_VERIFY_DATA => "data",
        _ => "disk_to_catalog",
    }
}

/// Storage the job queue counts (DoVerifyInit FreeRstorage/FreeWstorage):
/// none, the read storage, or read and write storage (Data).
pub fn storage_uses(params: &JobParams) -> usize {
    match params.verify.as_ref().map(|v| v.level as u8) {
        Some(L_VERIFY_DATA) => 2,
        Some(L_VERIFY_VOLUME_TO_CATALOG) => 1,
        Some(_) => 0,
        None => 1,
    }
}

// ---------------------------------------------------------------------------
// Console side: RunJob() of a Verify job
// ---------------------------------------------------------------------------

fn first_row(cat: &CatalogHandle, sql: &str) -> Result<Option<Vec<String>>, String> {
    Ok(cat.query_rows(sql)?.rows.into_iter().next()
        .map(|r| r.into_iter().map(|v| v.unwrap_or_default()).collect()))
}

/// Job record fields of the previous job (GetJobRecord).
struct PrevJob {
    job_id: u32,
    job: String,
    status: char,
    client_id: String,
    job_files: u64,
}

/// The previous job of DoVerify(): Err with the fatal message text.
fn previous_job(cat: &CatalogHandle, level: u8, name: &str, verify_job: Option<&str>, client_id: &str,
                supplied: u32) -> Result<PrevJob, String> {
    let verify_jobid = if supplied != 0 {
        supplied
    } else {
        // cats/sql_find.cc FindLastJobid()
        let sql = if level == L_VERIFY_CATALOG {
            format!("SELECT JobId FROM Job WHERE Type='V' AND Level='{}' AND  JobStatus IN ('T','W') AND Name='{}' AND \
                     ClientId={client_id} ORDER BY StartTime DESC LIMIT 1", L_VERIFY_INIT as char, esc(name))
        } else if let Some(vj) = verify_job {
            format!("SELECT JobId FROM Job WHERE Type='B' AND JobStatus IN ('T','W') AND Name='{}' \
                     ORDER BY StartTime DESC LIMIT 1", esc(vj))
        } else {
            format!("SELECT JobId FROM Job WHERE Type='B' AND JobStatus IN ('T','W') AND ClientId={client_id} \
                     ORDER BY StartTime DESC LIMIT 1")
        };
        match first_row(cat, &sql).ok().flatten().and_then(|r| r.first().and_then(|v| v.parse::<u32>().ok())) {
            Some(id) if id > 0 => id,
            _ => {
                return Err(if level == L_VERIFY_CATALOG {
                    "Unable to find JobId of previous InitCatalog Job.\nPlease run a Verify with Level=InitCatalog before\n\
                     running the current Job.\n".to_string()
                } else {
                    "Unable to find JobId of previous Job for this client.\n".to_string()
                });
            }
        }
    };
    let row = first_row(cat, &format!(
        "SELECT JobId,Job,JobStatus,ClientId,JobFiles FROM Job WHERE JobId={verify_jobid}"))
        .map_err(|e| format!("Could not get job record for previous Job. ERR={e}"))?
        .ok_or_else(|| format!("Could not get job record for previous Job. ERR=cats/sql_get.cc:275 No Job found for JobId {verify_jobid}\n"))?;
    let prev = PrevJob {
        job_id: verify_jobid,
        job: row.get(1).cloned().unwrap_or_default(),
        status: row.get(2).and_then(|s| s.chars().next()).unwrap_or('\0'),
        client_id: row.get(3).cloned().unwrap_or_default(),
        job_files: row.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
    };
    if !matches!(prev.status, 'T' | 'W') {
        return Err(format!("Last Job {verify_jobid} did not Terminate normally. JobStatus={}\n", prev.status));
    }
    Ok(prev)
}

/// RunJob() of a Verify job: SetupJob (Job record, DoVerifyInit) and the
/// first part of DoVerify() that needs the catalog; the job then waits in
/// the job queue for its worker.
pub(crate) fn run_verify(
    ast: &CommandAst,
    ctx: &ExecContext,
    config: std::sync::RwLockReadGuard<'_, crate::director_config::DirectorConfig>,
    job: bareos_config::resources::JobResource,
) -> CommandResult {
    if !ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes")) {
        return CommandResult::error("Job not run.\n".to_string());
    }
    let value = |key: &str| ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(key)).and_then(|a| a.value.clone());
    let job_dirs = crate::commands::defaults::job_directives(&config, &job.name);
    let directive = |key: &str| job_dirs.iter().rev()
        .find(|d| bareos_config::jobdefs::item_key(&d.key) == key).and_then(|d| d.first()).map(str::to_string);
    let level = value("level").or_else(|| directive("level")).and_then(|l| level_from_name(&l))
        .unwrap_or(L_VERIFY_CATALOG);
    let client_name = value("client").unwrap_or_else(|| job.client.clone());
    let Some(client) = config.find_client(&client_name).cloned() else {
        return CommandResult::error(format!("Client '{client_name}' not found\n"));
    };
    let mut fileset = value("fileset").unwrap_or_else(|| job.fileset.clone());
    let storage_name = value("storage").unwrap_or_else(|| job.storage.split(',').next().unwrap_or("").trim().to_string());
    let storage = config.find_storage(&storage_name).cloned();
    let verify_job = value("verifyjob").or_else(|| directive("jobtoverify")).or_else(|| directive("verifyjob"));
    let supplied_jobid: u32 = value("jobid").and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    let pool = job.pool.clone();
    let pool_type = config.raw.iter().find(|r| r.norm_type() == "pool" && r.name() == Some(pool.as_str()))
        .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == "pooltype").and_then(|d| d.first()))
        .unwrap_or("Backup").to_string();
    // DoVerify(): VolumeToCatalog verifies the FileSet of the Verify Job.
    if level == L_VERIFY_VOLUME_TO_CATALOG {
        if let Some(vj) = verify_job.as_deref().and_then(|n| config.find_job(n)) {
            fileset = vj.fileset.clone();
        }
    }
    let fileset_messages = match crate::commands::estimate::include_exclude_messages(&config, &fileset) {
        Ok(messages) => messages.unwrap_or_default(),
        Err(e) => return CommandResult::error(format!("{e}\n")),
    };
    let director = config.director.name.clone();
    drop(config);

    let Some(cat) = ctx.catalog.clone() else {
        return CommandResult::error("Job failed.\n".to_string());
    };
    let now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
    let sched_time = value("schedtime").and_then(|t| t.parse().ok()).unwrap_or(now);
    let unique = crate::commands::job::unique_job_name(&job.name, now);
    let job_id = match crate::commands::job::create_job_record(ctx, &unique, &job.name, 'V', level as char,
        Some(&client.name), sched_time, "") {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Failed to create job record: {e}");
            return CommandResult::error("Job failed.\n".to_string());
        }
    };
    let client_id = first_row(&cat, &format!("SELECT ClientId FROM Client WHERE Name='{}'", esc(&client.name)))
        .ok().flatten().and_then(|r| r.into_iter().next()).unwrap_or_else(|| "0".into());

    // job_thread(): "Version:", then DoVerify().
    let mut messages: Messages = vec![(MsgType::Info, crate::job_messages::version_text())];
    let mut session = Session { level, job_id, verify_job: verify_job.clone().filter(|_| true).unwrap_or_default(),
        ..Default::default() };
    let mut verify = VerifyParams { level: level as char, fileset: fileset_messages, fileset_name: fileset.clone(),
        storage_name: storage.as_ref().map(|s| s.name.clone()).unwrap_or_default(), pool_type, ..Default::default() };
    'prep: {
        if matches!(level, L_VERIFY_CATALOG | L_VERIFY_VOLUME_TO_CATALOG | L_VERIFY_DISK_TO_CATALOG) {
            let name = verify_job.as_deref().filter(|_| level != L_VERIFY_CATALOG);
            match previous_job(&cat, level, &job.name, name, &client_id, supplied_jobid) {
                Ok(prev) => {
                    messages.push((MsgType::Info, format!("Verifying against JobId={} Job={}\n", prev.job_id, prev.job)));
                    session.prev_job_id = prev.job_id;
                    session.prev_client_id = prev.client_id.clone();
                    if level == L_VERIFY_VOLUME_TO_CATALOG {
                        // CreateRestoreBootstrapFile()
                        let mut ua = crate::ua_select::Ua::new(ctx);
                        let mut bsr = crate::commands::restore_bsr::Bsr::default();
                        bsr.add_findex_all(prev.job_id);
                        let ok = bsr.add_volume_information(&mut ua);
                        let (count, path) = if ok && prev.job_files > 0 {
                            let mut store = None;
                            bsr.write_file(&mut ua, &CommandAst::default(), "", &mut store)
                        } else if ok {
                            ua.info_msg("No files found to read. No bootstrap file written.\n");
                            (0, String::new())
                        } else {
                            (0, String::new())
                        };
                        let result = ua.finish(true);
                        for line in [(MsgType::Error, result.error_message), (MsgType::Info, result.text_output)] {
                            if !line.1.is_empty() {
                                messages.push(line);
                            }
                        }
                        if !ok {
                            verify.failed = true;
                            break 'prep;
                        }
                        if count == 0 {
                            verify.nothing_to_do = true;
                            break 'prep;
                        }
                        session.expected_files = count as u64;
                        verify.bootstrap = std::fs::read_to_string(&path).unwrap_or_default();
                    }
                }
                Err(text) => {
                    fatal(&mut session, &mut messages, text);
                    verify.failed = true;
                    break 'prep;
                }
            }
        }
        messages.push((MsgType::Info, format!("Start Verify JobId={job_id} Level={} Job={unique}\n", level_name(level))));
    }

    let mut params = crate::commands::job::plain_params(job_id, &unique, &job, WorkerJobType::Verify,
        &(level as char).to_string(), &director, &client.name);
    params.fd_address = client.address.clone();
    params.fd_port = client.port;
    params.fd_password = client.password.clone();
    params.client_first_connection = !verify.failed && !verify.nothing_to_do
        && crate::job_messages::first_client_connection(&client.name);
    params.pool_name = pool;
    if let Some(store) = &storage {
        params.sd_address = store.address.clone();
        params.sd_port = store.port;
        params.sd_password = store.password.clone();
        params.sd_device = store.device.clone();
        params.sd_media_type = store.media_type.clone();
    }
    let queue_storage = match level {
        L_VERIFY_VOLUME_TO_CATALOG | L_VERIFY_DATA => verify.storage_name.clone(),
        _ => String::new(),
    };
    params.verify = Some(verify);
    let report = crate::job_messages::JobReport {
        kind: crate::job_messages::ReportKind::Verify, job: unique, client: client.name.clone(), fileset,
        level: level_name(level).to_string(), sched_time, start_time: now, priority: job.priority,
        trigger: crate::job_messages::TRIGGER_USER.to_string(), ..Default::default()
    };
    if let Some(verify) = params.verify.as_mut() {
        verify.start_messages = messages.into_iter().map(|(t, text)| (crate::audit::code(t), text)).collect();
    }
    add_session(session);
    crate::commands::job::queue_job(ctx, &job, params, report, &client.name, &queue_storage, sched_time)
}

// ---------------------------------------------------------------------------
// Director side while the job runs
// ---------------------------------------------------------------------------

/// sscanf(msg, "%ld %d %100s"): the number of fields converted and values.
fn scan_header(msg: &[u8]) -> (usize, i64, i64, Vec<u8>) {
    let mut p = 0;
    let skip_ws = |p: &mut usize| while *p < msg.len() && msg[*p].is_ascii_whitespace() { *p += 1 };
    let int = |p: &mut usize| -> Option<i64> {
        skip_ws(p);
        let start = *p;
        if *p < msg.len() && (msg[*p] == b'-' || msg[*p] == b'+') { *p += 1; }
        let digits = *p;
        while *p < msg.len() && msg[*p].is_ascii_digit() { *p += 1; }
        if *p == digits { *p = start; return None; }
        std::str::from_utf8(&msg[start..*p]).ok()?.parse().ok()
    };
    let Some(file_index) = int(&mut p) else { return (0, 0, 0, Vec::new()) };
    let Some(stream) = int(&mut p) else { return (1, file_index, 0, Vec::new()) };
    skip_ws(&mut p);
    let start = p;
    while p < msg.len() && p - start < 100 && msg[p] != 0 && !msg[p].is_ascii_whitespace() { p += 1; }
    if p == start {
        return (2, file_index, stream, Vec::new());
    }
    (3, file_index, stream, msg[start..p].to_vec())
}

fn skip_nonspaces(msg: &[u8], mut p: usize) -> usize {
    while p < msg.len() && msg[p] != 0 && msg[p] != b' ' { p += 1; }
    p
}

fn skip_spaces(msg: &[u8], mut p: usize) -> usize {
    while p < msg.len() && msg[p] == b' ' { p += 1; }
    p
}

fn cstr(msg: &[u8], p: usize) -> &[u8] {
    if p >= msg.len() {
        return &[];
    }
    let end = msg[p..].iter().position(|c| *c == 0).map(|i| p + i).unwrap_or(msg.len());
    &msg[p..end]
}

/// The filename and attributes of an attribute message (after the three
/// sscanf'ed fields).
fn name_and_attr(msg: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut p = skip_nonspaces(msg, 0);
    p = skip_spaces(msg, p);
    p = skip_nonspaces(msg, p);
    p = skip_spaces(msg, p);
    p = skip_nonspaces(msg, p);
    p += 1;
    let fname = cstr(msg, p).to_vec();
    let attr = cstr(msg, p + fname.len() + 1).to_vec();
    (fname, attr)
}

/// CryptoDigestStreamType() != CRYPTO_DIGEST_NONE.
fn is_digest_stream(stream: i64) -> bool {
    matches!(stream, STREAM_MD5_DIGEST | STREAM_SHA1_DIGEST | STREAM_SHA256_DIGEST | STREAM_SHA512_DIGEST
        | STREAM_XXH128_DIGEST)
}

/// stream_to_ascii() of the digest streams.
fn stream_to_ascii(stream: i64) -> String {
    match stream {
        STREAM_MD5_DIGEST => "MD5 digest".into(),
        STREAM_SHA1_DIGEST => "SHA1 digest".into(),
        STREAM_SHA256_DIGEST => "SHA256 digest".into(),
        STREAM_SHA512_DIGEST => "SHA512 digest".into(),
        STREAM_XXH128_DIGEST => "XXH128 digest".into(),
        other => other.to_string(),
    }
}

/// SplitPathAndFile(): everything up to the last '/' is the path.
fn split_path(fname: &[u8]) -> (String, String) {
    let split = fname.iter().rposition(|c| *c == b'/').map(|i| i + 1).unwrap_or(0);
    (String::from_utf8_lossy(&fname[..split]).into_owned(), String::from_utf8_lossy(&fname[split..]).into_owned())
}

/// GetPathRecord(): 0 when the path is not in the catalog.
fn path_id(db: &dyn Db, path: &str) -> String {
    db.rows(&format!("SELECT PathId FROM Path WHERE Path='{}'", esc(path))).ok()
        .and_then(|t| t.rows.into_iter().next()).and_then(|r| r.into_iter().next().flatten())
        .unwrap_or_else(|| "0".into())
}

/// GetFileAttributesRecord() / GetFileRecord() of the verify levels.
fn file_record(db: &dyn Db, s: &Session, fname: &[u8], file_index: i64) -> Option<FileRecord> {
    let (path, name) = split_path(fname);
    let path_id = path_id(db, &path);
    let sql = match s.level {
        L_VERIFY_DISK_TO_CATALOG => format!(
            "SELECT FileId, LStat, MD5, Fhinfo, Fhnode FROM File,Job WHERE File.JobId=Job.JobId AND File.PathId={path_id} AND \
             File.Name='{}' AND Job.Type='B' AND Job.JobStatus IN ('T','W') AND ClientId={} ORDER BY StartTime DESC LIMIT 1",
            esc(&name), s.prev_client_id),
        L_VERIFY_VOLUME_TO_CATALOG => format!(
            "SELECT FileId, LStat, MD5, Fhinfo, Fhnode FROM File WHERE File.JobId={} AND File.PathId={path_id} AND \
             File.Name='{}' AND File.FileIndex={}", s.prev_job_id, esc(&name), file_index as u32),
        _ => format!(
            "SELECT FileId, LStat, MD5, Fhinfo, Fhnode FROM File WHERE File.JobId={} AND File.PathId={path_id} AND \
             File.Name='{}'", s.prev_job_id, esc(&name)),
    };
    let row = db.rows(&sql).ok()?.rows.into_iter().next()?;
    let cell = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    Some(FileRecord { file_id: cell(0), lstat: cell(1), digest: cell(2) })
}

/// PrtFname()
fn print_fname(s: &mut Session, msgs: &mut Messages) {
    if !s.fn_printed {
        msgs.push((MsgType::Info, format!("File: {}\n", String::from_utf8_lossy(&s.fname))));
        s.fn_printed = true;
    }
}

/// One message of GetAttributesAndCompareToCatalog(): the job messages and
/// the job status to set (JS_Differences / JS_FatalError).
fn compare(db: &dyn Db, s: &mut Session, msg: &[u8], msgs: &mut Messages) -> Option<char> {
    let (fields, file_index, stream, opts) = scan_header(msg);
    if fields != 3 {
        s.bailed = true;
        return fatal(s, msgs, format!("dird<filed: bad attributes, expected 3 fields got {fields}\n mslen={} msg={}\n",
            msg.len(), String::from_utf8_lossy(cstr(msg, 0))));
    }
    let (fname, attr) = name_and_attr(msg);
    let mut status = None;
    match stream {
        STREAM_UNIX_ATTRIBUTES | STREAM_UNIX_ATTRIBUTES_EX => {
            s.file_index = file_index;
            let statf = crate::commands::bvfs::decode_stat(&String::from_utf8_lossy(&attr));
            s.digest_wanted = false;
            s.fn_printed = false;
            s.fname = fname.clone();
            let Some(fdbr) = file_record(db, s, &fname, file_index) else {
                msgs.push((MsgType::Info, format!("New file: {}\n", String::from_utf8_lossy(&fname))));
                return Some('D');
            };
            // MarkFileRecord()
            let _ = db.exec(&format!("UPDATE File SET MarkId={} WHERE FileId={}", s.job_id, fdbr.file_id));
            let statc = crate::commands::bvfs::decode_stat(&fdbr.lstat);
            s.fdbr = fdbr;
            for c in opts.iter().copied() {
                let differ = match c {
                    b'i' if statc.ino != statf.ino => Some(format!("      st_ino   differ. Cat: {} File: {}\n",
                        statc.ino as u64, statf.ino as u64)),
                    b'p' if statc.mode != statf.mode => Some(format!("      st_mode  differ. Cat: {:x} File: {:x}\n",
                        statc.mode as u32, statf.mode as u32)),
                    b'n' if statc.nlink != statf.nlink => Some(format!("      st_nlink differ. Cat: {} File: {}\n",
                        statc.nlink as u32 as i32, statf.nlink as u32 as i32)),
                    b'u' if statc.uid != statf.uid => Some(format!("      st_uid   differ. Cat: {} File: {}\n",
                        statc.uid, statf.uid)),
                    b'g' if statc.gid != statf.gid => Some(format!("      st_gid   differ. Cat: {} File: {}\n",
                        statc.gid, statf.gid)),
                    b's' if statc.size != statf.size => Some(format!("      st_size  differ. Cat: {} File: {}\n",
                        statc.size as u64, statf.size as u64)),
                    b'a' if statc.atime != statf.atime => Some("      st_atime differs\n".to_string()),
                    b'm' if statc.mtime != statf.mtime => Some("      st_mtime differs\n".to_string()),
                    b'c' if statc.ctime != statf.ctime => Some("      st_ctime differs\n".to_string()),
                    b'd' if statc.size > statf.size => Some(format!("      st_size  decrease. Cat: {} File: {}\n",
                        statc.size as u64, statf.size as u64)),
                    b'5' | b'1' => {
                        s.digest_wanted = true;
                        None
                    }
                    _ => None,
                };
                if let Some(text) = differ {
                    print_fname(s, msgs);
                    msgs.push((MsgType::Info, text));
                    status = Some('D');
                }
            }
        }
        STREAM_RESTORE_OBJECT => {}
        _ if is_digest_stream(stream) => {
            if s.file_index != file_index {
                s.bailed = true;
                return fatal(s, msgs, format!("MD5/SHA1 index {} not same as attributes {}\n",
                    file_index as i32, s.file_index as u32));
            }
            if s.digest_wanted {
                let digest = esc(&String::from_utf8_lossy(&opts));
                if digest != s.fdbr.digest {
                    print_fname(s, msgs);
                    msgs.push((MsgType::Info, format!("      {} differs. File={digest} Cat={}\n",
                        stream_to_ascii(stream), s.fdbr.digest)));
                    status = Some('D');
                }
                s.digest_wanted = false;
            }
        }
        _ => {}
    }
    status
}

/// CreateFileAttributesRecord() (non-batch): Path record, then File record.
fn create_file_record(db: &dyn Db, job_id: u32, ar: &CachedAttr) -> Result<(), String> {
    let (path, name) = split_path(&ar.fname);
    let mut id = path_id(db, &path);
    if id == "0" {
        let sql = format!("INSERT INTO Path (Path) VALUES ('{}') RETURNING PathId", esc(&path));
        id = db.rows(&sql).map_err(|e| format!("Create db Path record {sql} failed. ERR={e}\n"))?
            .rows.into_iter().next().and_then(|r| r.into_iter().next().flatten()).unwrap_or_default();
    }
    let digest = ar.digest.as_deref().filter(|d| !d.is_empty()).unwrap_or("0");
    let sql = format!("INSERT INTO File (FileIndex,JobId,PathId,Name,LStat,MD5,DeltaSeq,Fhinfo,Fhnode) \
        VALUES ({},{job_id},{id},'{}','{}','{}',0,0,0)", ar.file_index as u32, esc(&name),
        String::from_utf8_lossy(&ar.attr), digest);
    db.exec(&sql).map(|_| ()).map_err(|e| format!("Create db File record {sql} failed. ERR={e}"))
}

/// One message of GetAttributesAndPutInCatalog().
fn put_in_catalog(db: &dyn Db, s: &mut Session, msg: &[u8], msgs: &mut Messages) -> Option<char> {
    let (fields, file_index, stream, digest) = scan_header(msg);
    if fields != 3 {
        s.bailed = true;
        s.cached = None;
        return fatal(s, msgs, format!("<filed: bad attributes, expected 3 fields got {fields}\nmessage_length={} msg={}\n",
            msg.len(), String::from_utf8_lossy(cstr(msg, 0))));
    }
    let mut status = None;
    if stream == STREAM_UNIX_ATTRIBUTES || stream == STREAM_UNIX_ATTRIBUTES_EX {
        if let Some(ar) = s.cached.take() {
            if let Err(e) = create_file_record(db, s.job_id, &ar) {
                status = fatal(s, msgs, format!("Attribute create error. {e}"));
            }
        }
        let (fname, attr) = name_and_attr(msg);
        s.file_index = file_index;
        s.cached = Some(CachedAttr { file_index, fname, attr, digest: None });
    } else if is_digest_stream(stream) {
        if s.file_index != file_index {
            s.errors += 1;
            msgs.push((MsgType::Error, format!("{} index {} not same as attributes {}\n", stream_to_ascii(stream),
                file_index as u32, s.file_index as u32)));
            return None;
        }
        if let Some(ar) = s.cached.as_mut() {
            ar.digest = Some(esc(&String::from_utf8_lossy(&digest)));
        }
    }
    status
}

/// A verify event of the worker: job messages and the status to set.
pub fn event(catalog: Option<&CatalogHandle>, job_id: u32, event: &Event) -> (Messages, Option<char>) {
    let mut msgs = Messages::new();
    let status = with_session(job_id, |s| {
        let db = catalog.map(|c| c as &dyn Db)?;
        match event {
            Event::Attr { data } => {
                if s.bailed {
                    return None;
                }
                let msg = hex::decode(data).unwrap_or_default();
                if s.level == L_VERIFY_INIT {
                    put_in_catalog(db, s, &msg, &mut msgs)
                } else {
                    compare(db, s, &msg, &mut msgs)
                }
            }
            Event::AttrsDone { error } => {
                if let Some(error) = error {
                    s.bailed = true;
                    s.cached = None;
                    let text = if s.level == L_VERIFY_INIT {
                        format!("<filed: Network error getting attributes. ERR={error}\n")
                    } else {
                        format!("dir<filed: bad attributes from filed n=-1 : {error}\n")
                    };
                    return fatal(s, &mut msgs, text);
                }
                if s.level == L_VERIFY_INIT {
                    if let Some(ar) = s.cached.take() {
                        if let Err(e) = create_file_record(db, s.job_id, &ar) {
                            return fatal(s, &mut msgs, format!("Attribute create error. {e}"));
                        }
                    }
                    return None;
                }
                if s.bailed {
                    return None;
                }
                // The files in the catalog that were not marked (MissingHandler).
                let sql = format!("SELECT Path.Path,File.Name FROM File,Path WHERE File.JobId={} AND File.FileIndex > 0 \
                    AND File.MarkId!={} AND File.PathId=Path.PathId ", s.prev_job_id, s.job_id);
                let rows = db.rows(&sql).map(|t| t.rows).unwrap_or_default();
                for (i, row) in rows.iter().enumerate() {
                    if i == 0 {
                        msgs.push((MsgType::Warning, format!("The following files are in the Catalog but not on {}:\n",
                            if s.level == L_VERIFY_VOLUME_TO_CATALOG { "the Volume(s)" } else { "disk" })));
                    }
                    let cell = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                    msgs.push((MsgType::Info, format!("      {}{}\n", cell(0), cell(1))));
                }
                (!rows.is_empty()).then_some('D')
            }
            Event::FdEnd { status, read_bytes } => {
                s.fd_status = char::from_u32(*status).unwrap_or('\0');
                s.read_bytes = *read_bytes;
                None
            }
        }
    }).flatten();
    (msgs, status)
}

/// What VerifyCleanup() adds at the job end.
pub struct Finish {
    /// Director-side JobErrors.
    pub errors: i32,
    /// TermCode correction (VolumeToCatalog: ExpectedFiles != JobFiles).
    pub status: Option<char>,
}

/// VerifyCleanup() before UpdateJobEnd(): `files` is jcr->JobFiles.
pub fn finish(catalog: Option<&CatalogHandle>, job_id: u32, files: i64) -> Option<Finish> {
    with_session(job_id, |s| {
        if let Some(cat) = catalog {
            let _ = cat.raw_query(&format!("UPDATE Job SET ReadBytes={} WHERE JobId={job_id}", s.read_bytes));
        }
        let status = (s.level == L_VERIFY_VOLUME_TO_CATALOG && s.expected_files != files.max(0) as u64).then_some('E');
        Finish { errors: s.errors as i32, status }
    })
}

/// VerifyCleanup() term message and message type.
fn term_msg(status: char) -> (MsgType, String) {
    match status {
        'T' => (MsgType::Info, "Verify OK".into()),
        'f' | 'E' => (MsgType::Error, "*** Verify Error ***".into()),
        'e' => (MsgType::Info, "Verify warnings".into()),
        'A' => (MsgType::Info, "Verify Canceled".into()),
        'D' => (MsgType::Info, "Verify Differences".into()),
        other => (MsgType::Info, format!("Inappropriate term code: {} {other}\n", other as u32)),
    }
}

/// The VerifyCleanup() report.
pub fn summary(director: &str, report: &crate::job_messages::JobReport, outcome: &crate::job_messages::JobOutcome)
    -> Vec<crate::job_messages::JobMsg> {
    use crate::version_strings as version;
    let session = take_session(outcome.job_id).unwrap_or_default();
    let (msg_type, term) = term_msg(outcome.status);
    let head = format!("Bareos {director} {} ({}):\n", version::FULL, version::SHORT_DATE);
    let fd = crate::job_messages::job_status_to_ascii(session.fd_status);
    let common_head = format!("  JobId:                  {}\n  Job:                    {}\n  FileSet:                {}\n\
        \x20 Verify Level:           {}\n  Client:                 {}\n  Verify JobId:           0\n\
        \x20 Verify Job:             {}\n  Start time:             {}\n  End time:               {}\n",
        outcome.job_id, report.job, report.fileset, report.level, report.client, session.verify_job,
        crate::job_messages::bstrftimes(report.start_time), crate::job_messages::bstrftimes(outcome.end_time));
    let tail = format!("  Bareos binary info:     {}\n  Job triggered by:       {}\n  Termination:            {term}\n\n",
        version::JOBLOG_MESSAGE, report.trigger);
    let commas = crate::job_messages::commas;
    let text = if session.level == L_VERIFY_VOLUME_TO_CATALOG {
        format!("{head}  OS Information:         {}\n{common_head}  Files Expected:         {}\n  Files Examined:         {}\n\
            \x20 Non-fatal FD errors:    {}\n  FD termination status:  {fd}\n  SD termination status:  {}\n{tail}",
            version::os_info(), commas(session.expected_files), commas(outcome.files), outcome.errors,
            crate::job_messages::job_status_to_ascii(outcome.sd_status))
    } else {
        format!("{head}  Build:                  {}\n{common_head}  Files Examined:         {}\n\
            \x20 Non-fatal FD errors:    {}\n  FD termination status:  {fd}\n{tail}",
            version::os_info(), commas(outcome.files), outcome.errors)
    };
    vec![(msg_type, text)]
}

// ---------------------------------------------------------------------------
// Job worker
// ---------------------------------------------------------------------------

fn send_event(pipes: &mut WorkerPipes, job_id: u32, event: Event) {
    pipes.send(&PipeMessage::Verify { job_id, event });
}

fn info(job_id: u32, text: String) {
    crate::job_worker::worker_jmsg(job_id, 6, text);
}

type Bnet = BnetStream<PskStream>;

/// BgetDirmsg(): the next message that is not a job message; None at a
/// signal, Err at a socket error.
fn next_message(bs: &mut Bnet, job_id: u32) -> Result<Option<Vec<u8>>, String> {
    loop {
        match bs.recv() {
            Ok(BnetMessage::Data(data)) => {
                if data.starts_with(b"Jmsg") {
                    crate::job_worker::forward_daemon_jmsg(job_id, &String::from_utf8_lossy(&data));
                    continue;
                }
                return Ok(Some(data));
            }
            Ok(BnetMessage::Signal(BnetSignal::Heartbeat | BnetSignal::HeartbeatResponse)) => continue,
            Ok(BnetMessage::Signal(_)) => return Ok(None),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// response(): the next message must start with `expect`.
fn response(bs: &mut Bnet, job_id: u32, expect: &str, what: &str) -> Result<String, String> {
    match next_message(bs, job_id) {
        Ok(Some(data)) => {
            let text = String::from_utf8_lossy(&data).into_owned();
            if text.starts_with(expect) {
                Ok(text)
            } else {
                Err(format!("Bad response to {what} command: wanted {expect}, got {text}"))
            }
        }
        Ok(None) => Err(format!("Bad response to {what} command: wanted {expect}, got signal\n")),
        Err(e) => Err(format!("Bad response to {what} command: ERR={e}\n")),
    }
}

/// StartStorageDaemonJob() + ReserveReadDevice() + "run": the SD session
/// (VolSessionId, VolSessionTime, Authorization).
fn start_sd(sd: &mut Bnet, p: &JobParams, v: &VerifyParams) -> Result<(i32, i32, String), String> {
    let bash = crate::sd_label::bash_spaces;
    let job_id = p.job_id;
    sd.send_str(&format!("JobId={} job={} job_name={} client_name={} type={} level={} FileSet={} NoAttr=0 SpoolAttr=0 \
        FileSetMD5=**Dummy** SpoolData=0 PreferMountedVols=1 SpoolSize=0 rerunning=0 VolSessionId=0 VolSessionTime=0 \
        Quota=0 Protocol=0 BackupFormat=Native\n", job_id, p.unique_job, bash(&p.job_name), bash(&p.client_name),
        b'V', L_VERIFY_VOLUME_TO_CATALOG, bash(&v.fileset_name))).map_err(|e| e.to_string())?;
    let reply = match next_message(sd, job_id)? {
        Some(data) => String::from_utf8_lossy(&data).into_owned(),
        None => return Err("<stored: bad response to Job command: signal\n".into()),
    };
    let field = |key: &str| reply.split_whitespace().find_map(|w| w.strip_prefix(key)).map(str::to_string);
    let (Some(sdid), Some(sdtime), Some(key)) = (field("SDid="), field("SDtime="), field("Authorization=")) else {
        return Err(format!("Storage daemon rejected Job command: {reply}"));
    };
    sd.send_str("bootstrap\n").map_err(|e| e.to_string())?;
    for line in v.bootstrap.split_inclusive('\n') {
        sd.send_str(line).map_err(|e| e.to_string())?;
    }
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    response(sd, job_id, "3000 OK bootstrap", "Bootstrap")?;
    // SendSecureEraseReqToSd()
    sd.send_str("getSecureEraseCmd\n").map_err(|e| e.to_string())?;
    let _ = next_message(sd, job_id);
    // ReserveReadDevice()
    sd.send_str(&format!("use storage={} media_type={} pool_name={} pool_type={} append=0 copy=0 stripe=0\n",
        bash(&v.storage_name), bash(&p.sd_media_type), bash(&p.pool_name), bash(&v.pool_type)))
        .map_err(|e| e.to_string())?;
    for device in p.sd_device.split(',').map(str::trim).filter(|d| !d.is_empty()) {
        sd.send_str(&format!("use device={}\n", bash(device))).map_err(|e| e.to_string())?;
    }
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    let reply = next_message(sd, job_id)?.map(|d| String::from_utf8_lossy(&d).into_owned()).unwrap_or_default();
    let Some(device) = reply.strip_prefix("3000 OK use device device=").and_then(|d| d.split_whitespace().next()) else {
        return Err(format!("\n     Storage daemon didn't accept Device \"{}\" because:\n     {reply}\n", p.sd_device));
    };
    info(job_id, crate::job_messages::using_device_text(&crate::job_messages::unbash_spaces(device), false));
    sd.send_str("run").map_err(|e| e.to_string())?;
    Ok((sdid.parse().unwrap_or(0), sdtime.parse().unwrap_or(0), key))
}

/// DoVerify() from the daemon connections on, in the job worker.
pub(crate) fn run_worker(p: &JobParams, pipes: &mut WorkerPipes) -> i32 {
    use crate::job_worker::{EXIT_ERROR, EXIT_SUCCESS};
    let job_id = p.job_id;
    let v = p.verify.clone().unwrap_or_default();
    let level = v.level as u8;
    // job_thread(): "Version:", RunScripts "BeforeJob", then DoVerify (S-07).
    let before = crate::job_worker::start_messages_with_before(pipes, job_id, &v.start_messages);
    let end = |pipes: &mut WorkerPipes, status: u8, files: i64, bytes: i64, errors: i32, sdid: i32, sdtime: i32| {
        pipes.send(&PipeMessage::JobEnd { job_id, term_code: status as i32, files, bytes, errors,
            vol_session_id: Some(sdid), vol_session_time: Some(sdtime) });
        if matches!(status, b'T' | b'W') { EXIT_SUCCESS } else { EXIT_ERROR }
    };
    if crate::job_manager::is_job_canceled(before.status) {
        return end(pipes, before.status as u8, 0, 0, before.errors as i32, 0, 0);
    }
    if v.failed {
        return end(pipes, b'f', 0, 0, 0, 0, 0);
    }
    if v.nothing_to_do {
        return end(pipes, b'T', 0, 0, 0, 0, 0);
    }
    let fail = |pipes: &mut WorkerPipes, text: String, sdid: i32, sdtime: i32| {
        crate::job_worker::worker_jmsg(job_id, M_FATAL, text);
        end(pipes, b'f', 0, 0, 1, sdid, sdtime)
    };
    let state = |pipes: &mut WorkerPipes, state: &str| pipes.send(&PipeMessage::State { job_id, state: state.into() });

    // VolumeToCatalog: the Storage daemon first.
    let mut sd_thread = None;
    let (mut sdid, mut sdtime, mut auth_key) = (0, 0, "dummy".to_string());
    if level == L_VERIFY_VOLUME_TO_CATALOG {
        state(pipes, "ConnectingSD");
        let mut sd = match crate::job_worker::connect_sd(p) {
            Ok((sd, cipher)) => {
                info(job_id, crate::job_messages::connected_text("Storage daemon", &p.sd_address, p.sd_port, Some(&cipher)));
                info(job_id, crate::job_messages::cipher_text(Some(&cipher)));
                sd
            }
            Err(e) => return fail(pipes, format!("Failed to connect to Storage daemon: {e}\n"), 0, 0),
        };
        state(pipes, "SDJobSetup");
        match start_sd(&mut sd, p, &v) {
            Ok((id, time, key)) => (sdid, sdtime, auth_key) = (id, time, key),
            Err(e) => return fail(pipes, e, sdid, sdtime),
        }
        let (Ok(mut to_dir), Ok(mut from_dir)) = (pipes.to_director.try_clone(), pipes.from_director.try_clone()) else {
            return fail(pipes, "Cannot create message thread\n".into(), sdid, sdtime);
        };
        sd_thread = std::thread::Builder::new().name(format!("sd-verify:{job_id}"))
            .spawn(move || crate::job_worker::sd_catrec_loop(&mut sd, job_id, &mut to_dir, &mut from_dir)).ok();
    }

    state(pipes, "ConnectingFD");
    if p.client_first_connection {
        info(job_id, crate::job_messages::PROBING_TEXT.to_string());
    }
    let mut fd = match crate::job_worker::connect_fd(p) {
        Ok((fd, cipher)) => {
            info(job_id, crate::job_messages::connected_text(&format!("Client: {}", p.client_name), &p.fd_address,
                p.fd_port, Some(&cipher)));
            info(job_id, crate::job_messages::handshake_text(p.client_first_connection));
            info(job_id, crate::job_messages::cipher_text(Some(&cipher)));
            fd
        }
        Err(e) => return fail(pipes, format!("Failed to connect to Client: {e}\n"), sdid, sdtime),
    };
    state(pipes, "FDJobSetup");
    let bail = |pipes: &mut WorkerPipes, fd: &mut Bnet, text: String| {
        let _ = fd.send_signal(BnetSignal::Terminate);
        crate::job_worker::worker_jmsg(job_id, M_FATAL, text);
        end(pipes, b'f', 0, 0, 1, sdid, sdtime)
    };
    // SendJobInfoToFileDaemon()
    let ssl = if level == L_VERIFY_VOLUME_TO_CATALOG { 4 } else { 254 };
    if let Err(e) = fd.send_str(&format!("JobId={job_id} Job={} SDid={sdid} SDtime={sdtime} Authorization={auth_key} ssl={ssl}\n",
        p.unique_job)) {
        return bail(pipes, &mut fd, format!("{e}\n"));
    }
    match next_message(&mut fd, job_id) {
        Ok(Some(data)) if data.starts_with(b"2000 OK Job") => {}
        other => {
            let text = match other { Ok(Some(d)) => String::from_utf8_lossy(&d).into_owned(), _ => String::new() };
            return bail(pipes, &mut fd, format!("File daemon \"{}\" rejected Job command: {text}", p.client_name));
        }
    }
    // SendIncludeExcludeLists()
    if !v.fileset.is_empty() {
        for line in &v.fileset {
            if fd.send_str(line).is_err() {
                return bail(pipes, &mut fd, "Error sending include and exclude lists.\n".into());
            }
        }
        let _ = fd.send_signal(BnetSignal::Eod);
        if let Err(e) = response(&mut fd, job_id, "2000 OK include", "Include") {
            return bail(pipes, &mut fd, e);
        }
    }
    if level == L_VERIFY_VOLUME_TO_CATALOG {
        let _ = fd.send_str(&format!("storage address={} port={} ssl=4 Authorization={auth_key}\n", p.sd_address, p.sd_port));
        if let Err(e) = response(&mut fd, job_id, "2000 OK storage", "Storage") {
            return bail(pipes, &mut fd, e);
        }
    }
    // SendRunscriptsCommands() (S-07)
    if let Err(e) = crate::job_worker::send_client_runscripts(&mut fd, p, &before) {
        tracing::debug!("Worker {job_id}: client runscripts: {e}");
        let _ = fd.send_signal(BnetSignal::Terminate);
        return end(pipes, b'f', 0, 0, 1, sdid, sdtime);
    }
    let _ = fd.send_str(&format!("verify level={}\n", fd_level(level)));
    if let Err(e) = response(&mut fd, job_id, "2000 OK verify", "Verify") {
        return bail(pipes, &mut fd, e);
    }
    if level == L_VERIFY_DATA {
        // verify.cc:416: no comparison is implemented for Level=Data.
        return bail(pipes, &mut fd, format!("dird/verify.cc:416 Unimplemented verify level {}\n", L_VERIFY_DATA));
    }
    state(pipes, "Running");
    let error = loop {
        match next_message(&mut fd, job_id) {
            Ok(Some(data)) => send_event(pipes, job_id, Event::Attr { data: hex::encode(&data) }),
            Ok(None) => break None,
            Err(e) => break Some(e),
        }
    };
    send_event(pipes, job_id, Event::AttrsDone { error });

    // WaitForJobTermination()
    let mut fd_end: Option<(u8, i64, u64, i64, i32)> = None;
    let mut errors = 0;
    loop {
        match next_message(&mut fd, job_id) {
            Ok(Some(data)) => {
                let text = String::from_utf8_lossy(&data).into_owned();
                let field = |key: &str| text.split_whitespace().find_map(|w| w.strip_prefix(key)).and_then(|v| v.parse::<i64>().ok());
                if fd_end.is_none() && text.starts_with("2800 End Job ") {
                    if let (Some(tc), Some(files), Some(read), Some(bytes), Some(errs)) = (field("TermCode="),
                        field("JobFiles="), field("ReadBytes="), field("JobBytes="), field("Errors=")) {
                        fd_end = Some((tc as u8, files, read as u64, bytes, errs as i32));
                        continue;
                    }
                }
                crate::job_worker::worker_jmsg(job_id, 5, format!("Unexpected Client Job message: {text}\n"));
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    let _ = fd.send_signal(BnetSignal::Terminate);
    if fd_end.is_none() {
        crate::job_worker::worker_jmsg(job_id, M_FATAL, "No Job status returned from FD.\n".into());
        errors = 1;
    }
    if let Some(thread) = sd_thread {
        let _ = thread.join();
    }
    state(pipes, "Finishing");
    match fd_end {
        Some((status, files, read_bytes, bytes, errs)) => {
            send_event(pipes, job_id, Event::FdEnd { status: status as u32, read_bytes });
            end(pipes, status, files, bytes, errs, sdid, sdtime)
        }
        None => {
            send_event(pipes, job_id, Event::FdEnd { status: b'E' as u32, read_bytes: 0 });
            end(pipes, b'f', 0, 0, errors, sdid, sdtime)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_scan_follows_sscanf() {
        assert_eq!(scan_header(b"12 1 pins5 /a/b\0attr\0").0, 3);
        assert_eq!(scan_header(b"2994 Bad verify level").0, 1);
        assert_eq!(scan_header(b"5 3").0, 2);
        assert_eq!(scan_header(b"x").0, 0);
        let (f, a) = name_and_attr(b"3 1 V:pins5 /data/file.txt\0P0C A\0");
        assert_eq!((f.as_slice(), a.as_slice()), (b"/data/file.txt".as_slice(), b"P0C A".as_slice()));
    }

    #[test]
    fn levels_and_term_messages() {
        assert_eq!(level_from_name("disktocatalog"), Some(L_VERIFY_DISK_TO_CATALOG));
        assert_eq!(fd_level(L_VERIFY_DISK_TO_CATALOG), "disk_to_catalog");
        assert_eq!(term_msg('D').1, "Verify Differences");
        assert_eq!(term_msg('f').0, MsgType::Error);
        assert_eq!(term_msg('W').1, "Inappropriate term code: 87 W\n");
        assert_eq!(split_path(b"/a/b/"), ("/a/b/".to_string(), String::new()));
    }
}
