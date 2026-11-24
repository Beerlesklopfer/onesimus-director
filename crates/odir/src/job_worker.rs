// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Job worker — autonomous backup/restore execution via fork().
//!
//! Each job runs in its own process. The worker uses a State Machine
//! to track protocol progress and communicates with the Director
//! via bidirectional pipes.
//!
//! ## Worker FSM States
//!
//! ```text
//! Init → ConnectingFD → ConnectingSD → SDJobSetup → SDUseDevice
//!   → SDRun → FDJobSetup → FDStorage → FDSession → FDBackup
//!   → Running → Finishing → Completed
//!                ↓ SIGTERM        ↓ SIGUSR1
//!            Canceling          Paused → (SIGUSR2) → Running
//!                ↓                               any → Error
//! ```

use std::io::{self, Read, Write, BufRead, BufReader};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{info, debug, warn, error};

use bareos_bnet::{BnetStream, BnetSignal, BnetMessage, CramMd5Auth};
use bareos_bnet::hello::make_director_hello;
use bareos_bnet::tls_psk;

// ============================================================================
// Public types (used by Director + Worker)
// ============================================================================

/// Job type for worker dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WorkerJobType {
    Backup,
    Restore,
    /// admin.cc: no daemon contact; the worker only reports the start and
    /// ends the job (JS_Terminated). Compatibility log C-058.
    Admin,
    /// Copy/Migrate control job and Consolidate: run by the Director itself
    /// (internal_jobs.rs), never forked. Compatibility log C-058.
    Copy,
    Migrate,
    Consolidate,
    /// verify.cc DoVerify(): FD (and for VolumeToCatalog SD) contact by the
    /// worker, the catalog comparison by the Director (verify.rs). C-058.
    Verify,
}

impl WorkerJobType {
    /// The Director-side job of this type, if it is one.
    pub fn internal(self) -> Option<crate::internal_jobs::Kind> {
        match self {
            Self::Copy => Some(crate::internal_jobs::Kind::Copy),
            Self::Migrate => Some(crate::internal_jobs::Kind::Migrate),
            Self::Consolidate => Some(crate::internal_jobs::Kind::Consolidate),
            _ => None,
        }
    }
}

/// The TLS policy of a Client resource without TLS directives (TLS Enable and Require, both yes).
fn default_fd_tls_policy() -> i32 {
    2
}

/// Parameters passed from Director to Worker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobParams {
    /// Restore: "pluginoptions …" lines for the FD (SendPluginOptions()).
    #[serde(default)]
    pub plugin_options: Vec<String>,
    pub job_id: u32,
    pub unique_job: String,
    pub job_name: String,
    pub job_type: WorkerJobType,
    pub level: String,
    pub fd_address: String,
    pub fd_port: u16,
    pub fd_password: String,
    /// `ssl=` of the Director's CRAM-MD5 challenge to the FD ([`crate::fd_protocol::tls_policy`] of the
    /// Client's `TLS Enable`/`TLS Require`): 0 also means the FD is reached in clear text.
    #[serde(default = "default_fd_tls_policy")]
    pub fd_tls_policy: i32,
    /// AutoPrune and retentions of the Client resource: with them the worker updates the Client record
    /// (with the FD's uname) after `2000 OK Job`, like SendJobInfoToFileDaemon; `None`: no update.
    #[serde(default)]
    pub client_retention: Option<crate::backup::ClientRetention>,
    #[serde(default)]
    pub fd_grpc: Option<crate::fd_grpc::Target>,
    /// gRPC endpoint of the Storage (osd), snapshotted before fork.
    #[serde(default)]
    pub sd_grpc: Option<crate::fd_grpc::Target>,
    pub sd_address: String,
    pub sd_port: u16,
    pub sd_password: String,
    pub sd_device: String,
    pub sd_media_type: String,
    pub director_name: String,
    pub pool_name: String,
    /// Name of the Job resource's FileSet (`FileSet=` of the SD job
    /// command); empty sends `**None**`.
    #[serde(default)]
    pub fileset_name: String,
    pub fileset_includes: Vec<String>,
    pub fileset_excludes: Vec<String>,
    // Incremental/Differential: timestamp of reference backup
    pub since_utime: i64,          // 0 = Full, >0 = epoch of last relevant backup
    pub prev_job: String,          // unique job name of reference backup (for incr/diff)
    // Restore-specific
    pub where_path: Option<String>,
    /// RegexWhere of the run or the Job (BuildRestoreCommand: before where).
    #[serde(default)]
    pub regex_where: Option<String>,
    /// Job `Prefix Links` (restore `prelinks=`).
    #[serde(default)]
    pub prefix_links: bool,
    pub replace: Option<String>,       // "always", "never", "ifnewer", "ifolder"
    pub restore_jobid: Option<u32>,     // JobId of backup to restore from
    /// Restore: the bootstrap file (jcr->RestoreBootstrap) the job reads.
    pub bootstrap: Option<String>,
    /// Client resource name (job messages "Connected Client: <name> ...").
    #[serde(default)]
    pub client_name: String,
    /// First connection to the client since the configuration was loaded
    /// (fd_cmds.cc "Probing client protocol..." and the handshake text).
    #[serde(default)]
    pub client_first_connection: bool,
    /// Client Passive = yes: the SD connects to the FD (restore).
    #[serde(default)]
    pub passive_client: bool,
    /// Verify job data (verify.rs), None for other job types.
    #[serde(default)]
    pub verify: Option<crate::verify::VerifyParams>,
    /// SD read+write session of a Copy/Migrate worker job or a VirtualFull
    /// (sd_copy.rs); None for jobs with a File daemon. C-058.
    #[serde(default)]
    pub copy: Option<crate::sd_copy::CopySession>,
    /// Native Backup: the DoNativeBackup session (backup_worker.rs, S-19).
    #[serde(default)]
    pub backup: Option<crate::backup_worker::BackupSession>,
}

/// Messages on the Worker↔Director pipe (JSON, one per line).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum PipeMessage {
    #[serde(rename = "state")]
    State { job_id: u32, state: String },
    #[serde(rename = "status")]
    Status { job_id: u32, files: i64, bytes: i64, errors: i32 },
    #[serde(rename = "job_status")]
    JobStatus { job_id: u32, status: i32 },
    #[serde(rename = "cat_req")]
    CatReq { job_id: u32, request: String },
    #[serde(rename = "cat_resp")]
    CatResp { response: String },
    /// SD `UpdCat` attribute record, hex encoded (binary; no answer).
    #[serde(rename = "upd_cat")]
    UpdCat { job_id: u32, data: String },
    #[serde(rename = "job_end")]
    JobEnd { job_id: u32, term_code: i32, files: i64, bytes: i64, errors: i32,
             #[serde(default)] vol_session_id: Option<i32>,
             #[serde(default)] vol_session_time: Option<i32> },
    /// A job message (Jmsg) for the Director to dispatch: `msg_type` is the
    /// M_* code, `mtime` the message time (0: when dispatched). `raw`: a
    /// complete message a daemon sent ("Jmsg Job=.. type=.. level=.."),
    /// otherwise the body the Director puts its Jmsg header in front of.
    #[serde(rename = "jmsg")]
    Jmsg { job_id: u32, msg_type: u32, mtime: i64, text: String, #[serde(default)] raw: bool },
    /// A catalog statement of the job the Director runs for the worker
    /// (UpdateJobStartRecord, UpdateClientRecord; backup_worker.rs, S-19).
    #[serde(rename = "catalog_sql")]
    CatalogSql { job_id: u32, sql: String },
    /// FD/SD end counters of a native backup, sent before its JobEnd (S-19).
    #[serde(rename = "backup_end")]
    BackupEnd { job_id: u32, end: crate::backup::BackupEnd },
    /// SendAccurateCurrentFiles(): the worker asks for the accurate list of
    /// a job started at `start_time` (S-23) ...
    #[serde(rename = "accurate_request")]
    AccurateRequest { job_id: u32, start_time: i64 },
    /// ... and the Director answers with the list file (or the fatal text).
    #[serde(rename = "accurate_ready")]
    AccurateReady { path: String, estimate: String, error: Option<String> },
    /// DoNativeBackup(): CheckHardquotas/CheckSoftquotas in the Director (S-23) ...
    #[serde(rename = "quota_check")]
    QuotaCheck { job_id: u32 },
    /// ... the M_FATAL text of an exceeded quota and FetchRemainingQuotas().
    #[serde(rename = "quota_checked")]
    QuotaChecked { fatal: Option<String>, remaining: u64 },
    /// The handshake mode the client connection succeeded with
    /// (connection_successful_handshake_; cleartext or immediate TLS). S-27.
    #[serde(rename = "client_handshake")]
    ClientHandshake { job_id: u32, client: String, cleartext: bool },
    /// jcr->setJobStatus(JS_Running) without the priority check (fd_cmds.cc
    /// UpdateFailedConnectionHandshakeMode): even a cancel is forgotten. S-28.
    #[serde(rename = "reset_status")]
    ResetStatus { job_id: u32 },
    /// job_thread() RunScripts(jcr, ..., label) in the Director (console
    /// commands need it); the worker waits for [`PipeMessage::RunScriptsDone`].
    #[serde(rename = "run_scripts")]
    RunScripts { job_id: u32, label: String },
    /// The job status after the scripts, their M_ERROR count, and the
    /// fd_cmds.cc SendRunscriptsCommands() "Run ..." lines for the client
    /// (with "RunBeforeNow" when `run_before_now`). S-07.
    #[serde(rename = "run_scripts_done")]
    RunScriptsDone { status: i32, errors: u32, client_commands: Vec<String>, run_before_now: bool },
    /// The writing Storage daemon's job end of an SD-SD Copy/Migrate
    /// (mig_jcr: VolSessionId/Time from its Job command, SDJobFiles/Bytes/
    /// Errors/Status from its "3099 Job end"). C-058, S-20.
    #[serde(rename = "write_session")]
    WriteSession { job_id: u32, vol_session_id: i32, vol_session_time: i32, files: i64, bytes: i64, errors: i32, status: i32 },
    /// A Verify job event for the Director (verify.rs).
    #[serde(rename = "verify")]
    Verify { job_id: u32, event: crate::verify::Event },
}

/// What the Director's RunScripts "BeforeJob" left for the worker.
#[derive(Debug, Default)]
pub(crate) struct BeforeJob {
    pub(crate) status: char,
    pub(crate) errors: u32,
    pub(crate) client_commands: Vec<String>,
    pub(crate) run_before_now: bool,
}

/// Ask the Director to run the "BeforeJob" scripts and wait for them.
pub(crate) fn run_before_job(pipes: &mut WorkerPipes, job_id: u32) -> BeforeJob {
    pipes.send(&PipeMessage::RunScripts { job_id, label: "BeforeJob".into() });
    loop {
        match pipes.recv() {
            Some(PipeMessage::RunScriptsDone { status, errors, client_commands, run_before_now }) => {
                return BeforeJob { status: char::from_u32(status as u32).unwrap_or('R'), errors, client_commands, run_before_now };
            }
            Some(_) => continue,
            // The Director is gone: nothing ran.
            None => return BeforeJob { status: 'R', ..Default::default() },
        }
    }
}

/// The start messages a job prepared at queue time, with job_thread's
/// RunScripts "BeforeJob" right after its "Version:" message (first when
/// there is none); the rest only when the scripts did not fail the job.
pub(crate) fn start_messages_with_before(pipes: &mut WorkerPipes, job_id: u32, messages: &[(u32, String)]) -> BeforeJob {
    let at = messages.iter().position(|(_, text)| text.starts_with("Version: ")).map_or(0, |i| i + 1);
    for (code, text) in &messages[..at] {
        worker_jmsg(job_id, *code, text.clone());
    }
    let before = run_before_job(pipes, job_id);
    if !crate::job_manager::is_job_canceled(before.status) {
        for (code, text) in &messages[at..] {
            worker_jmsg(job_id, *code, text.clone());
        }
    }
    before
}

/// fd_cmds.cc SendRunscriptsCommands(): the client's scripts, then
/// "RunBeforeNow" (the File daemon runs ClientRunBeforeJob before it
/// answers). Failure: "Client \"%s\" RunScript failed." (M_FATAL).
pub(crate) fn send_client_runscripts<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, p: &JobParams, before: &BeforeJob) -> Result<(), String> {
    let mut commands: Vec<(&str, &str, &str)> = before.client_commands.iter()
        .map(|c| (c.as_str(), "2000 OK RunScript\n", "RunScript")).collect();
    if before.run_before_now {
        commands.push(("RunBeforeNow\n", "2000 OK RunBeforeNow\n", "RunBeforeNow"));
    }
    for (command, resp, name) in commands {
        if let Err(e) = director_response(fd, p.job_id, command, resp, name) {
            worker_jmsg(p.job_id, M_FATAL, format!("Client \"{}\" RunScript failed.\n", p.client_name));
            WORKER_FATAL.store(true, Ordering::SeqCst);
            return Err(e);
        }
    }
    Ok(())
}

/// getmsg.cc response(jcr, fd, resp, cmd, DISPLAY_ERROR) after sending
/// `command`: daemon job messages are dispatched (BgetDirmsg), anything
/// but `resp` is a fatal "Bad response" error.
fn director_response<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, job_id: u32, command: &str, resp: &str, name: &str) -> Result<(), String> {
    let socket_error = |e: String| {
        let text = format!("Socket error on {name} command: ERR={e}\n");
        worker_jmsg(job_id, M_FATAL, text.clone());
        text
    };
    fd.send_str(command).map_err(|e| socket_error(e.to_string()))?;
    loop {
        match fd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let msg = String::from_utf8_lossy(&data).to_string();
                if msg.starts_with("Jmsg") { forward_daemon_jmsg(job_id, &msg); continue; }
                if msg == resp { return Ok(()); }
                let text = format!("Bad response to {name} command: wanted {resp}, got {msg}\n");
                worker_jmsg(job_id, M_FATAL, text.clone());
                return Err(text);
            }
            Ok(BnetMessage::Signal(_)) => continue,
            Err(e) => return Err(socket_error(e.to_string())),
        }
    }
}
/// lib/message_severity.h M_* codes (one definition: `bareos_format::msg`)
/// as the `u32` the worker pipe carries; the other worker modules use these.
pub(crate) const M_FATAL: u32 = bareos_format::msg::M_FATAL as u32;
/// `M_ERROR` as `u32` (see [`M_FATAL`]).
pub(crate) const M_ERROR: u32 = bareos_format::msg::M_ERROR as u32;
/// `M_WARNING` as `u32` (see [`M_FATAL`]).
pub(crate) const M_WARNING: u32 = bareos_format::msg::M_WARNING as u32;
/// `M_INFO` as `u32` (see [`M_FATAL`]).
pub(crate) const M_INFO: u32 = bareos_format::msg::M_INFO as u32;

/// The worker's pipe to the Director for job messages (any thread).
static JMSG_PIPE: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

/// Write one pipe message as one line in a single write.
fn write_pipe_line(file: &mut std::fs::File, msg: &PipeMessage) {
    if let Ok(mut json) = serde_json::to_string(msg) {
        json.push('\n');
        let _ = file.write_all(json.as_bytes());
    }
}

/// One pipe message as one write under the pipe lock: lines of the worker's
/// threads never interleave (a torn JSON line is dropped by the Director).
pub(crate) fn send_jmsg(msg: PipeMessage) {
    let mut pipe = JMSG_PIPE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(file) = pipe.as_mut() {
        write_pipe_line(file, &msg);
    }
}

/// Jmsg(jcr, type, 0, text) of the worker's job.
pub(crate) fn worker_jmsg(job_id: u32, msg_type: u32, text: String) {
    send_jmsg(PipeMessage::Jmsg { job_id, msg_type, mtime: 0, text, raw: false });
}

/// getmsg.cc BgetDirmsg(): "Jmsg Job=%127s type=%d level=%lld <message>"
/// gives (type, mtime, message); None when malformed.
fn parse_daemon_jmsg(line: &str) -> Option<(u32, i64, String)> {
    let rest = line.strip_prefix("Jmsg Job=")?;
    let (job, rest) = rest.split_once(' ')?;
    if job.is_empty() {
        return None;
    }
    let rest = rest.trim_start_matches(' ');
    let (msg_type, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let msg_type = msg_type.strip_prefix("type=")?.parse::<i32>().ok()?;
    let rest = rest.trim_start_matches(' ');
    let (level, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let mtime = level.strip_prefix("level=")?.parse::<i64>().ok()?;
    Some((msg_type as u32, mtime, rest.to_string()))
}

/// A job message line of the File or Storage daemon: dispatched by the
/// Director as it is; "Malformed message: %s\n" (M_ERROR) otherwise.
pub(crate) fn forward_daemon_jmsg(job_id: u32, line: &str) {
    match parse_daemon_jmsg(line) {
        Some((msg_type, mtime, text)) => send_jmsg(PipeMessage::Jmsg { job_id, msg_type, mtime, text, raw: true }),
        None => worker_jmsg(job_id, M_ERROR, format!("Malformed message: {line}\n")),
    }
}

/// Legacy local exit codes and original FD termination characters.
pub fn catalog_job_status(term_code: i32) -> &'static str {
    match term_code {
        0 | 84 => "T", 87 => "W", 101 => "e", 69 | 2 => "E",
        102 => "f", 65 | 3 => "A", 68 => "D", 73 => "I", _ => "E",
    }
}

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_FATAL: i32 = 2;
pub const EXIT_CANCELED: i32 = 3;

// ============================================================================
// Worker FSM
// ============================================================================

#[derive(Debug, Clone)]
enum WorkerState {
    Init,
    ConnectingFD,
    ConnectingSD,
    SDJobSetup,
    SDUseDevice,
    SDRun,
    SDBootstrap,    // Restore: send bootstrap to SD
    FDJobSetup,
    FDStorage,
    FDSession,
    FDBackup,       // Backup: "backup" command
    FDRestore,      // Restore: "restore replace=a where=/path"
    Running,
    Paused,
    Canceling,
    Finishing { files: i64, bytes: i64, errors: i32, term_code: i32 },
    Completed { files: i64, bytes: i64, errors: i32 },
    Error(String),
}

impl WorkerState {
    fn name(&self) -> &str {
        match self {
            Self::Init => "Init",
            Self::ConnectingFD => "ConnectingFD",
            Self::ConnectingSD => "ConnectingSD",
            Self::SDJobSetup => "SDJobSetup",
            Self::SDUseDevice => "SDUseDevice",
            Self::SDRun => "SDRun",
            Self::SDBootstrap => "SDBootstrap",
            Self::FDJobSetup => "FDJobSetup",
            Self::FDStorage => "FDStorage",
            Self::FDSession => "FDSession",
            Self::FDBackup => "FDBackup",
            Self::FDRestore => "FDRestore",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Canceling => "Canceling",
            Self::Finishing { .. } => "Finishing",
            Self::Completed { .. } => "Completed",
            Self::Error(_) => "Error",
        }
    }
}

/// Pipes for Worker↔Director communication.
pub(crate) struct WorkerPipes {
    pub(crate) to_director: std::fs::File,
    pub(crate) from_director: std::fs::File,
}

impl WorkerPipes {
    pub(crate) fn send(&mut self, msg: &PipeMessage) {
        // Locked single write (JMSG_PIPE is a clone of to_director): the SD
        // thread writes to the same pipe.
        send_jmsg(msg.clone());
    }

    pub(crate) fn recv(&mut self) -> Option<PipeMessage> {
        let mut buf = String::new();
        let mut reader = BufReader::new(&mut self.from_director);
        match reader.read_line(&mut buf) {
            Ok(0) => None,
            Ok(_) => serde_json::from_str(&buf).ok(),
            Err(_) => None,
        }
    }

    fn send_state(&mut self, job_id: u32, state: &WorkerState) {
        self.send(&PipeMessage::State { job_id, state: state.name().to_string() });
    }
}

// ============================================================================
// Worker State Machine — runs in fork()-child
// ============================================================================

/// Main entry point for the autonomous worker process.
pub fn run_worker(
    params: &JobParams,
    to_director: std::fs::File,
    from_director: std::fs::File,
) -> i32 {
    *JMSG_PIPE.lock().unwrap_or_else(|e| e.into_inner()) = to_director.try_clone().ok();
    let mut pipes = WorkerPipes { to_director, from_director };
    let mut state = WorkerState::Init;
    let job_id = params.job_id;

    let mut fd: Option<BnetStream<bareos_bnet::AnyStream>> = None;
    let mut sd: Option<BnetStream<bareos_bnet::AnyStream>> = None;
    let mut sd_auth_key = String::new();
    let mut vol_session_id: i32 = 0;
    let mut vol_session_time: i32 = 0;
    let mut bootstrap = String::new();

    // admin.cc do_admin(): "Start Admin JobId %d, Job=%s", then
    // AdminCleanup(jcr, JS_Terminated) in the Director (job end).
    // Copy/Migrate worker jobs and VirtualFull: SD read+write session (C-058).
    if let Some(copy) = &params.copy {
        return crate::sd_copy::run(params, copy, &mut pipes);
    }

    // backup.cc DoNativeBackup() (S-19).
    if let Some(backup) = &params.backup {
        return crate::backup_worker::run(params, backup, &mut pipes);
    }

    if params.job_type == WorkerJobType::Admin {
        // job.cc job_thread(): "Version: …" when the job runs.
        worker_jmsg(job_id, M_INFO, crate::job_messages::version_text());
        let before = run_before_job(&mut pipes, job_id);
        // A failed BeforeJob script (FailJobOnError) cancels the job:
        // AdminCleanup(jcr, JS_Canceled) keeps JS_ErrorTerminated.
        let status = if crate::job_manager::is_job_canceled(before.status) { before.status } else {
            worker_jmsg(job_id, M_INFO, crate::job_messages::start_admin_text(job_id, &params.unique_job));
            'T'
        };
        pipes.send(&PipeMessage::JobEnd { job_id, term_code: status as i32, files: 0, bytes: 0, errors: before.errors as i32,
            vol_session_id: Some(0), vol_session_time: Some(0) });
        return if status == 'T' { EXIT_SUCCESS } else { EXIT_ERROR };
    }

    // Verify: BeforeJob after its "Version:" start message (verify.rs).
    if params.job_type == WorkerJobType::Verify {
        return crate::verify::run_worker(params, &mut pipes);
    }

    // job_thread(): RunScripts "BeforeJob" before the job contacts a daemon.
    let before = run_before_job(&mut pipes, job_id);
    if crate::job_manager::is_job_canceled(before.status) {
        pipes.send(&PipeMessage::JobEnd { job_id, term_code: before.status as i32, files: 0, bytes: 0,
            errors: before.errors as i32, vol_session_id: Some(0), vol_session_time: Some(0) });
        return EXIT_ERROR;
    }

    loop {
        let old_name = state.name().to_string();
        state = match state {
            // job_thread "Version: …", restore.cc DoNativeRestore: the
            // bootstrap file is checked and read before any connection.
            WorkerState::Init if params.job_type == WorkerJobType::Restore => {
                worker_jmsg(job_id, M_INFO, crate::job_messages::version_text());
                match restore_bootstrap(job_id, params.bootstrap.as_deref(), &params.unique_job) {
                    Ok(data) => { bootstrap = data; WorkerState::ConnectingFD }
                    Err(fatal) => {
                        worker_jmsg(job_id, M_FATAL, fatal);
                        pipes.send(&PipeMessage::JobEnd { job_id, term_code: b'f' as i32, files: 0, bytes: 0,
                            errors: 1, vol_session_id: Some(0), vol_session_time: Some(0) });
                        return EXIT_FATAL;
                    }
                }
            }
            WorkerState::Init => {
                WorkerState::ConnectingFD
            }

            WorkerState::ConnectingFD => {
                // fd_cmds.cc ConnectToFileDaemon(): OutputMessageForConnectionTry,
                // TlsLogConninfo, SendInfoSuccess, SendInfoChosenCipher
                if params.client_first_connection {
                    worker_jmsg(job_id, M_INFO, crate::job_messages::PROBING_TEXT.to_string());
                }
                match connect_fd(params) {
                    Ok((b, cipher)) => {
                        let who = format!("Client: {}", params.client_name);
                        worker_jmsg(job_id, M_INFO, crate::job_messages::connected_text(&who, &params.fd_address, params.fd_port, Some(&cipher)));
                        worker_jmsg(job_id, M_INFO, crate::job_messages::handshake_text(params.client_first_connection));
                        worker_jmsg(job_id, M_INFO, crate::job_messages::cipher_text(Some(&cipher)));
                        fd = Some(b); info!("Worker {job_id}: FD connected"); WorkerState::ConnectingSD
                    }
                    Err(e) => WorkerState::Error(format!("FD connect: {e}"))
                }
            }

            WorkerState::ConnectingSD => {
                match connect_sd(params) {
                    Ok((b, cipher)) => {
                        // TlsLogConninfo (Qmsg) and sd_cmds.cc:152
                        worker_jmsg(job_id, M_INFO, crate::job_messages::connected_text("Storage daemon", &params.sd_address, params.sd_port, Some(&cipher)));
                        worker_jmsg(job_id, M_INFO, crate::job_messages::cipher_text(Some(&cipher)));
                        sd = Some(b); info!("Worker {job_id}: SD connected"); WorkerState::SDJobSetup
                    }
                    Err(e) => WorkerState::Error(format!("SD connect: {e}"))
                }
            }

            WorkerState::SDJobSetup => {
                match sd_job_setup(sd.as_mut().unwrap(), params) {
                    Ok(sess) => {
                        sd_auth_key = sess.auth_key;
                        vol_session_id = sess.vol_session_id;
                        vol_session_time = sess.vol_session_time;
                        info!("Worker {job_id}: SD auth key ({} chars) SDid={vol_session_id} SDtime={vol_session_time}", sd_auth_key.len());
                        WorkerState::SDUseDevice
                    }
                    Err(e) => WorkerState::Error(format!("SD job: {e}"))
                }
            }

            WorkerState::SDUseDevice => {
                match sd_use_device(sd.as_mut().unwrap(), params) {
                    Ok(device) => {
                        // msgchan.cc:183/259
                        let write = params.job_type != WorkerJobType::Restore;
                        worker_jmsg(job_id, M_INFO, crate::job_messages::using_device_text(&device, write));
                        WorkerState::SDRun
                    }
                    Err(e) => WorkerState::Error(format!("SD use: {e}"))
                }
            }

            WorkerState::SDRun => {
                if params.job_type == WorkerJobType::Restore {
                    WorkerState::SDBootstrap
                } else {
                    debug!("Worker {job_id}: SD run");
                    let _ = sd.as_mut().unwrap().send_str("run\n");
                    WorkerState::FDJobSetup
                }
            }

            WorkerState::SDBootstrap => {
                match osd::job::sd_send_bootstrap(sd.as_mut().unwrap(), &bootstrap) {
                    Ok(()) => {
                        debug!("Worker {job_id}: bootstrap sent to SD, sending run");
                        // After bootstrap, SD expects "run" — then enters WaitFD().
                        // Passive client: "passive client" comes first (FDStorage).
                        if !params.passive_client {
                            let _ = sd.as_mut().unwrap().send_str("run\n");
                        }
                        WorkerState::FDJobSetup
                    }
                    Err(e) => WorkerState::Error(format!("SD bootstrap: {e}"))
                }
            }

            WorkerState::FDJobSetup => {
                let result = if params.job_type == WorkerJobType::Restore {
                    fd_restore_setup(fd.as_mut().unwrap(), params, &sd_auth_key)
                } else {
                    fd_job_setup(fd.as_mut().unwrap(), params, &sd_auth_key)
                };
                match result.and_then(|()| send_client_runscripts(fd.as_mut().unwrap(), params, &before)) {
                    Ok(()) => WorkerState::FDStorage,
                    Err(e) => WorkerState::Error(format!("FD setup: {e}"))
                }
            }

            WorkerState::FDStorage if params.job_type == WorkerJobType::Restore && params.passive_client => {
                // Client Passive = yes: the SD connects to the FD, so the FD
                // gets no "storage" command (restore.cc, passive client).
                let grpc = crate::sd_grpc::tokens(params.fd_grpc.as_ref(), &params.fd_address,
                    sd.as_ref().is_some_and(crate::sd_grpc::on_grpc), fd.as_ref().is_some_and(crate::sd_grpc::on_grpc));
                let cmd = format!("passive client address={} port={} ssl=4{grpc}\n", params.fd_address, params.fd_port);
                match send_expect(sd.as_mut().unwrap(), &cmd, "2000 OK passive client", job_id, "SD passive client") {
                    Ok(_) => match sd.as_mut().unwrap().send_str("run\n") {
                        Ok(_) => WorkerState::FDRestore,
                        Err(e) => WorkerState::Error(format!("SD run: {e}")),
                    },
                    Err(e) => WorkerState::Error(format!("SD passive client: {e}")),
                }
            }

            WorkerState::FDStorage => {
                // Restore: storage command includes Authorization (FD uses it to auth to SD)
                let grpc = crate::sd_grpc::tokens(params.sd_grpc.as_ref(), &params.sd_address,
                    sd.as_ref().is_some_and(crate::sd_grpc::on_grpc), fd.as_ref().is_some_and(crate::sd_grpc::on_grpc));
                let cmd = if params.job_type == WorkerJobType::Restore {
                    format!("storage address={} port={} ssl=4 Authorization={}{grpc}\n",
                        params.sd_address, params.sd_port, sd_auth_key)
                } else {
                    format!("storage address={} port={} ssl=4{grpc}\n",
                        params.sd_address, params.sd_port)
                };
                match send_expect(fd.as_mut().unwrap(), &cmd, "2000 OK storage", job_id, "FD storage") {
                    Ok(_) => {
                        if params.job_type == WorkerJobType::Restore {
                            // Restore: NO session command — FD does "read open session" itself
                            WorkerState::FDRestore
                        } else {
                            WorkerState::FDSession
                        }
                    }
                    Err(e) => WorkerState::Error(format!("FD storage: {e}"))
                }
            }

            WorkerState::FDSession => {
                let cmd = format!("session {} 1 {} 0 0 0 0\n", "", now_unix());
                let next = if params.job_type == WorkerJobType::Restore {
                    WorkerState::FDRestore
                } else {
                    WorkerState::FDBackup
                };
                match send_expect(fd.as_mut().unwrap(), &cmd, "2000 OK session", job_id, "FD session") {
                    Ok(_) => next,
                    Err(e) => WorkerState::Error(format!("FD session: {e}"))
                }
            }

            WorkerState::FDBackup => {
                match send_expect(fd.as_mut().unwrap(), "backup\n", "2000 OK backup", job_id, "FD backup") {
                    Ok(_) => {
                        info!("Worker {job_id}: backup running");
                        WorkerState::Running
                    }
                    Err(e) => WorkerState::Error(format!("FD backup: {e}"))
                }
            }

            WorkerState::FDRestore => {
                let cmd = fd_restore_command(params);
                let where_path = params.regex_where.as_deref().or(params.where_path.as_deref()).unwrap_or("");
                match send_expect(fd.as_mut().unwrap(), &cmd, "2000 OK restore", job_id, "FD restore") {
                    Ok(_) => {
                        info!("Worker {job_id}: restore running → {where_path}");
                        WorkerState::Running
                    }
                    Err(e) => WorkerState::Error(format!("FD restore: {e}"))
                }
            }

            WorkerState::Running => {
                // Two parallel tasks: FD read + SD CatReq proxy
                let fd_stream = fd.take().unwrap();
                let sd_stream = sd.take().unwrap();
                let result = run_backup_phase(fd_stream, sd_stream, job_id, &mut pipes);
                match result {
                    Ok((files, bytes, errors, term_code)) => {
                        WorkerState::Finishing { files, bytes, errors, term_code }
                    }
                    Err(e) => WorkerState::Error(format!("Running: {e}"))
                }
            }

            WorkerState::Paused => {
                while WORKER_PAUSED.load(Ordering::SeqCst) {
                    if WORKER_CANCEL.load(Ordering::SeqCst) { break; }
                    thread::sleep(std::time::Duration::from_millis(100));
                }
                if WORKER_CANCEL.load(Ordering::SeqCst) {
                    WorkerState::Canceling
                } else {
                    WorkerState::Running
                }
            }

            WorkerState::Canceling => {
                if let Some(ref mut f) = fd { let _ = f.send_signal(BnetSignal::Terminate); }
                if let Some(ref mut s) = sd { let _ = s.send_signal(BnetSignal::Terminate); }
                WorkerState::Error("Canceled".to_string())
            }

            WorkerState::Finishing { files, bytes, errors, term_code } => {
                // Preserve the FD's actual termination status for the catalog and wait.
                pipes.send(&PipeMessage::JobEnd {
                    job_id, term_code, files, bytes, errors,
                    vol_session_id: Some(vol_session_id), vol_session_time: Some(vol_session_time),
                });
                return if matches!(catalog_job_status(term_code), "T" | "W") { EXIT_SUCCESS } else { EXIT_ERROR };
            }

            WorkerState::Completed { files, bytes, errors } => {
                info!("Worker {job_id}: completed — files={files} bytes={bytes}");
                pipes.send(&PipeMessage::JobEnd {
                    job_id, term_code: 0, files, bytes, errors,
                    vol_session_id: Some(vol_session_id), vol_session_time: Some(vol_session_time),
                });
                return EXIT_SUCCESS;
            }

            WorkerState::Error(ref e) => {
                error!("Worker {job_id}: error — {e}");
                // The Joblog gets the cause too (a canceled job has its own message).
                if !WORKER_CANCEL.load(Ordering::SeqCst) {
                    worker_jmsg(job_id, M_FATAL, format!("{e}\n"));
                }
                if let Some(ref mut f) = fd { let _ = f.send_signal(BnetSignal::Terminate); }
                if let Some(ref mut s) = sd { let _ = s.send_signal(BnetSignal::Terminate); }
                pipes.send(&PipeMessage::JobEnd {
                    job_id, term_code: if WORKER_CANCEL.load(Ordering::SeqCst) { b'A' as i32 } else if WORKER_FATAL.load(Ordering::SeqCst) { b'f' as i32 } else { b'E' as i32 }, files: 0, bytes: 0, errors: 1,
                    vol_session_id: Some(vol_session_id), vol_session_time: Some(vol_session_time),
                });
                return EXIT_FATAL;
            }
        };

        // Log + pipe state transition
        let new_name = state.name();
        if old_name != new_name {
            debug!("Worker {job_id}: {old_name} → {new_name}");
            pipes.send_state(job_id, &state);
        }

        // Check signals
        if WORKER_CANCEL.load(Ordering::SeqCst) && !matches!(state, WorkerState::Canceling | WorkerState::Error(_) | WorkerState::Completed { .. }) {
            state = WorkerState::Canceling;
        }
        if WORKER_PAUSED.load(Ordering::SeqCst) && matches!(state, WorkerState::Running) {
            state = WorkerState::Paused;
        }
    }
}

/// Running phase — two threads: FD read + SD CatReq proxy.
/// Returns (files, bytes, errors, term_code) from FD "2800 End Job".
fn run_backup_phase(
    mut fd: BnetStream<bareos_bnet::AnyStream>,
    mut sd: BnetStream<bareos_bnet::AnyStream>,
    job_id: u32,
    pipes: &mut WorkerPipes,
) -> Result<(i64, i64, i32, i32), String> {
    // SD CatReq thread
    let mut pipe_to = pipes.to_director.try_clone().map_err(|e| format!("pipe clone: {e}"))?;
    let mut pipe_from = pipes.from_director.try_clone().map_err(|e| format!("pipe clone: {e}"))?;

    let sd_handle = thread::Builder::new()
        .name(format!("sd-cat:{job_id}"))
        .spawn(move || {
            sd_catrec_loop(&mut sd, job_id, &mut pipe_to, &mut pipe_from);
        })
        .map_err(|e| format!("sd thread: {e}"))?;

    // Main thread: read FD messages until restore is done.
    // The FD sends "2000 OK storage end" when SD data transfer is complete,
    // then waits for the Director to signal end-of-commands (EOD/Terminate).
    // After receiving the signal, FD sends "2800 End Job" and quits.
    let result = wait_for_fd_end_job(&mut fd, job_id);
    let _ = fd.send_signal(BnetSignal::Terminate);
    sd_handle.join().ok();

    Ok(result)
}

// ============================================================================
// SD Protocol helpers — delegated to osd crate
// ============================================================================

use osd::job::{SdSession, SdJobParams, SdJobType};

fn sd_job_setup(sd: &mut BnetStream<bareos_bnet::AnyStream>, p: &JobParams) -> Result<SdSession, String> {
    osd::job::sd_job_setup(sd, &to_sd_params(p))
}

/// The device the Storage daemon accepted (OK_device "3000 OK use device
/// device=%s", unbashed); its job messages are forwarded.
fn sd_use_device(sd: &mut BnetStream<bareos_bnet::AnyStream>, p: &JobParams) -> Result<String, String> {
    let (answer, jmsgs) = osd::job::sd_use_device(sd, &to_sd_params(p))?;
    for line in &jmsgs {
        forward_daemon_jmsg(p.job_id, line);
    }
    let device = answer.trim_end().split_once("3000 OK use device device=").map(|(_, d)| d)
        .and_then(|d| d.split_whitespace().next()).unwrap_or(&p.sd_device);
    Ok(crate::job_messages::unbash_spaces(device))
}

/// restore.cc DoNativeRestore and bsr.cc OpenBootstrapFile: the bootstrap
/// file of the restore job (jcr->RestoreBootstrap) and the job messages
/// around it, "Start Restore Job" once a file is named; `Err` is the
/// M_FATAL text the job ends with. Never a bootstrap built otherwise.
fn restore_bootstrap(job_id: u32, path: Option<&str>, unique_job: &str) -> Result<String, String> {
    let Some(path) = path.filter(|p| !p.is_empty()) else {
        return Err("Cannot restore without a bootstrap file.\n\
            You probably ran a restore job directly. All restore jobs must\n\
            be run using the restore command.\n".to_string());
    };
    worker_jmsg(job_id, M_INFO, crate::job_messages::start_restore_text(unique_job));
    std::fs::read_to_string(path).map_err(|e| format!("Could not open bootstrap file {path}: ERR={}\n",
        crate::commands::restore::errno_text(&e)))
}

fn to_sd_params(p: &JobParams) -> SdJobParams {
    SdJobParams {
        job_id: p.job_id,
        unique_job: p.unique_job.clone(),
        job_name: p.job_name.clone(),
        job_type: match p.job_type { WorkerJobType::Restore => SdJobType::Restore, _ => SdJobType::Backup },
        level: p.level.clone(),
        client_name: p.client_name.clone(),
        // The Job resource's FileSet, as msgchan.cc sends jcr->res.fileset (D11).
        fileset: p.fileset_name.clone(),
        fileset_md5: String::new(),
        media_type: p.sd_media_type.clone(),
        pool_name: p.pool_name.clone(),
        device_name: p.sd_device.clone(),
    }
}

/// SendJobInfoToFileDaemon(): after `2000 OK Job <uname>` the Client record gets the resource's AutoPrune
/// and retentions and the FD's uname (UpdateClientRecord), for every job type.
pub(crate) fn update_client_record(p: &JobParams, reply: &str) {
    let Some(retention) = p.client_retention else { return };
    let uname = reply.trim_end_matches(['\n', '\r']).get("2000 OK Job".len() + 1..).unwrap_or("");
    send_jmsg(PipeMessage::CatalogSql { job_id: p.job_id, sql: crate::backup::update_client_record_sql(&p.client_name, retention, uname) });
}

/// FD setup for restore: JobId + storage only — NO level, NO fileset.
/// C++ Director sends: JobId → storage → session → restore
/// See: core/src/dird/fd_cmds.cc SendRunscriptsToFd / DoRestoreInit
fn fd_restore_setup(fd: &mut BnetStream<bareos_bnet::AnyStream>, p: &JobParams, sd_auth_key: &str) -> Result<(), String> {
    let cmd = format!("JobId={} Job={} SDid=1 SDtime={} Authorization={sd_auth_key} ssl=4\n",
        p.job_id, p.unique_job, now_unix());
    let reply = send_expect(fd, &cmd, "2000 OK Job", p.job_id, "FD JobId")?;
    update_client_record(p, &reply);
    // getSecureEraseCmd
    fd.send_str("getSecureEraseCmd\n").map_err(|e| format!("send secErase: {e}"))?;
    let _ = fd.recv_str(); // 2000 OK FDSecureEraseCmd *None*
    // SendPluginOptions(): the restore's plugin options (e.g. restore_mode=online).
    for line in &p.plugin_options {
        send_expect(fd, line, "2000 OK PluginOptions", p.job_id, "FD PluginOptions")?;
    }
    // Restore: NO level command, NO fileset — go straight to storage
    Ok(())
}

// Bareos getmsg.cc: Status Job=%127s JobStatus=%d; job_status.h: M/m/R.
pub(crate) fn storage_job_status(message: &str) -> Result<u8, &'static str> {
    let parts: Vec<_> = message.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "Status"
        || !parts[1].starts_with("Job=") || parts[1].len() == 4 {
        return Err("Malformed Storage job status");
    }
    let status = parts[2].strip_prefix("JobStatus=")
        .and_then(|value| value.parse::<u8>().ok()).ok_or("Malformed Storage job status")?;
    Ok(status)
}

#[cfg(test)]
fn storage_wait_state(message: &str) -> Result<Option<&'static str>, &'static str> {
    Ok(match storage_job_status(message)? { b'M' => Some("WaitingMount"), b'm' => Some("WaitingMedia"), b'R' => Some("Running"), _ => None })
}

pub(crate) fn sd_catrec_loop(
    sd: &mut BnetStream<bareos_bnet::AnyStream>,
    job_id: u32,
    _to_dir: &mut std::fs::File,
    from_dir: &mut std::fs::File,
) {
    debug!("Worker {job_id}: SD CatReq thread started");
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let msg_raw = String::from_utf8_lossy(&data);
                let msg = msg_raw.trim().to_string();
                // getmsg.cc BgetDirmsg(): `U`pdCat records go to CatalogUpdate()
                // without an answer, `C`atReq/Catreq and `B`lastAttr to the
                // Director, which may answer (an empty answer sends nothing).
                if data.first() == Some(&b'U') {
                    send_jmsg(PipeMessage::UpdCat { job_id, data: hex::encode(&data) });
                } else if matches!(data.first(), Some(b'C' | b'B')) {
                    debug!("Worker {job_id} SD CatReq: {msg}");
                    send_jmsg(PipeMessage::CatReq { job_id, request: msg_raw.into_owned() });
                    // Read CatResp from Director via pipe
                    match read_catalog_answer(from_dir) {
                        Ok(response) => {
                            debug!("Worker {job_id} → SD: {}", response.chars().take(60).collect::<String>());
                            if !response.is_empty() {
                                let _ = sd.send_str(&response);
                            }
                        }
                        Err(why) => {
                            abort_without_catalog(job_id, &why);
                            break;
                        }
                    }
                } else if msg.starts_with("Status Job=") {
                    match storage_job_status(&msg) {
                        Ok(status) => {
                            send_jmsg(PipeMessage::JobStatus { job_id, status: status as i32 });
                        }
                        Err(error) => warn!("Worker {job_id}: {error}"),
                    }
                } else if msg.starts_with("3099 Job") {
                    if let Some(status) = msg.split_whitespace().find_map(|part| part.strip_prefix("JobStatus=").and_then(|value| value.parse::<u8>().ok())) {
                        send_jmsg(PipeMessage::JobStatus { job_id, status: status as i32 });
                    }
                    debug!("Worker {job_id} SD: job end");
                    break;
                } else if msg.starts_with("3010 Job") {
                    debug!("Worker {job_id} SD: job start");
                } else if msg.starts_with("Jmsg") {
                    forward_daemon_jmsg(job_id, &msg_raw);
                } else {
                    debug!("Worker {job_id} SD: {}", &msg[..msg.len().min(80)]);
                }
            }
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
            Ok(BnetMessage::Signal(sig)) => debug!("Worker {job_id} SD signal: {sig:?}"),
            Err(e) => {
                debug!("Worker {job_id} SD recv: {e}");
                // msgchan.cc:509-517: a lost SD connection is FATAL and the
                // SD status JS_ErrorTerminated, so a restore cannot end "OK"
                // (a canceled job leaves the loop first, msgchan.cc:482). J-8.
                if !WORKER_CANCEL.load(Ordering::SeqCst) {
                    worker_jmsg(job_id, M_FATAL, "Director's comm line to SD dropped.\n".into());
                    send_jmsg(PipeMessage::JobStatus { job_id, status: b'E' as i32 });
                }
                break;
            }
        }
    }
    let _ = sd.send_signal(BnetSignal::Terminate);
    debug!("Worker {job_id}: SD CatReq thread ended");
}

// ============================================================================
// FD Protocol helpers
// ============================================================================

fn fd_job_setup(fd: &mut BnetStream<bareos_bnet::AnyStream>, p: &JobParams, sd_auth_key: &str) -> Result<(), String> {
    let cmd = format!("JobId={} Job={} SDid=1 SDtime={} Authorization={sd_auth_key} ssl=4\n", p.job_id, p.unique_job, now_unix());
    let reply = send_expect(fd, &cmd, "2000 OK Job", p.job_id, "FD JobId")?;
    update_client_record(p, &reply);
    // getSecureEraseCmd
    fd.send_str("getSecureEraseCmd\n").map_err(|e| format!("send secErase: {e}"))?;
    let _ = fd.recv_str(); // 2000 OK FDSecureEraseCmd *None*

    // C++ format: "level = %s%s%s mtime_only=%d %s%s\n"
    // Full:        "level = full  mtime_only=0 \n"
    // Incremental: "level = since_utime <epoch> mtime_only=0 prev_job=<name>\n"
    let level_str = match p.level.to_lowercase().as_str() {
        "full" | "f" => "level = full  mtime_only=0 \n".to_string(),
        "incremental" | "i" | "differential" | "d" => {
            if p.since_utime == 0 {
                // No reference backup → upgrade to Full
                warn!("Worker {}: no reference backup for {} — upgrading to Full", p.job_id, p.level);
                "level = full  mtime_only=0 \n".to_string()
            } else {
                format!("level =  since_utime {} mtime_only=0 prev_job={}\n",
                    p.since_utime, p.prev_job)
            }
        }
        other => format!("level = {other}  mtime_only=0 \n"),
    };
    send_expect(fd, &level_str, "2000 OK level", p.job_id, "FD level")?;

    // FileSet
    fd.send_str("fileset vss=0\n").map_err(|e| format!("fileset: {e}"))?;
    for inc in &p.fileset_includes {
        fd.send_str("I\n").map_err(|e| format!("I: {e}"))?;
        for path in inc.lines() {
            let path = path.trim();
            if !path.is_empty() { fd.send_str(&format!("F {path}\n")).map_err(|e| format!("F: {e}"))?; }
        }
        fd.send_str("N\n").map_err(|e| format!("N: {e}"))?;
    }
    for exc in &p.fileset_excludes {
        fd.send_str("E\n").map_err(|e| format!("E: {e}"))?;
        for path in exc.lines() {
            let path = path.trim();
            if !path.is_empty() { fd.send_str(&format!("F {path}\n")).map_err(|e| format!("F: {e}"))?; }
        }
        fd.send_str("N\n").map_err(|e| format!("N: {e}"))?;
    }
    fd.send_signal(BnetSignal::Eod).map_err(|e| format!("fileset eod: {e}"))?;
    loop {
        match fd.recv_str() {
            Ok(r) if r.contains("2000 OK") => break,
            Ok(r) if r.starts_with("Jmsg") => { forward_daemon_jmsg(p.job_id, &r); continue }
            Ok(r) => return Err(format!("fileset: {r}")),
            Err(e) => return Err(format!("fileset recv: {e}")),
        }
    }
    Ok(())
}

fn wait_for_fd_end_job(fd: &mut BnetStream<bareos_bnet::AnyStream>, job_id: u32) -> (i64, i64, i32, i32) {
    loop {
        match fd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let msg = String::from_utf8_lossy(&data).trim().to_string();
                if msg.starts_with("2800 End Job") {
                    info!("Worker {job_id}: FD End Job: {msg}");
                    match end_job_of(&msg) {
                        Some(end) => return end,
                        // backup.cc:690-701: only the complete EndJob format
                        // ends the wait; anything else is "Unexpected Client
                        // Job message" (M_WARNING), never a success. J-17.
                        None => {
                            worker_jmsg(job_id, M_WARNING, format!("Unexpected Client Job message: {}", String::from_utf8_lossy(&data)));
                            continue;
                        }
                    }
                }
                if msg.starts_with("2000 OK storage end") {
                    // FD is done with SD data transfer. Signal end-of-commands
                    // so FD exits its command loop and sends "2800 End Job".
                    info!("Worker {job_id}: FD storage end — sending EOD");
                    let _ = fd.send_signal(BnetSignal::Eod);
                    continue;
                }
                if msg.starts_with("Jmsg") {
                    forward_daemon_jmsg(job_id, &String::from_utf8_lossy(&data));
                    continue;
                }
                debug!("Worker {job_id} FD: {}", &msg[..msg.len().min(120)]);
            }
            Ok(BnetMessage::Signal(sig)) => {
                debug!("Worker {job_id} FD signal: {sig:?}");
                if matches!(sig, BnetSignal::Terminate) {
                    // backup.cc:739-741: no EndJob: FDJobStatus JS_ErrorTerminated.
                    worker_jmsg(job_id, M_FATAL, "No Job status returned from FD.\n".into());
                    return (0, 0, 1, 2);
                }
            }
            Err(e) => {
                error!("Worker {job_id} FD recv error: {e}");
                worker_jmsg(job_id, M_FATAL, format!("Network error with FD: ERR={e}\n"));
                worker_jmsg(job_id, M_FATAL, "No Job status returned from FD.\n".into());
                return (0, 0, 1, 2);
            }
        }
    }
}

/// backup.cc:72-75 EndJob "2800 End Job TermCode=%d JobFiles=%u ReadBytes=%llu
/// JobBytes=%llu Errors=%u VSS=%d Encrypt=%d": (JobFiles, JobBytes, Errors,
/// TermCode) when all seven fields are there (sscanf() == 7). Without a
/// TermCode the FD's status is unknown, not JS_Terminated. J-17.
fn end_job_of(msg: &str) -> Option<(i64, i64, i32, i32)> {
    crate::backup_worker::parse_end_job(msg)
        .map(|(term_code, files, _, bytes, errors, _, _)| (files as i64, bytes as i64, errors as i32, term_code as i32))
}

// ============================================================================
// Connection helpers
// ============================================================================

pub(crate) fn connect_fd(p: &JobParams) -> Result<(BnetStream<bareos_bnet::AnyStream>, String), String> {
    // a client initiated connection the Director handed over (authenticated already)
    if let Some((stream, _)) = crate::client_pool::take_handoff(p.job_id) {
        return Ok((stream, String::new()));
    }
    if let Some(stream) = crate::fd_grpc::try_worker(p) {
        return Ok(stream);
    }
    let sock = resolve_connect(&format!("{}:{}", p.fd_address, p.fd_port), 3)?;
    connect_fd_on(p, sock)
}

/// The TCP connection of a daemon connection (the socket is shut down on
/// SIGTERM); the io::Error for BareosSocketTCP::connect()'s retry messages.
pub(crate) fn tcp_connect(host: &str, port: u16) -> std::io::Result<std::net::TcpStream> {
    use std::net::ToSocketAddrs;
    let sa = (host, port).to_socket_addrs()?.next()
        .ok_or_else(|| std::io::Error::from_raw_os_error(libc::EHOSTUNREACH))?;
    let sock = std::net::TcpStream::connect_timeout(&sa, std::time::Duration::from_secs(3))?;
    register_cancel_fd(std::os::fd::AsRawFd::as_raw_fd(&sock));
    if WORKER_CANCEL.load(Ordering::SeqCst) {
        let _ = sock.shutdown(std::net::Shutdown::Both);
    }
    Ok(sock)
}

/// Why ConnectToFileDaemon() failed on a connected socket (S-27).
#[derive(Debug)]
pub(crate) enum FdHandshakeError {
    /// DoTlsHandshake() failed.
    Tls(String),
    /// Sending the Hello failed.
    Hello(String),
    /// AuthenticateOutboundConnection() failed.
    Auth,
    /// No answer to the Hello.
    Response(String),
}

/// fd_cmds.cc ConnectToFileDaemon() / authenticate.cc
/// AuthenticateWithFileDaemon() in one handshake mode: immediate TLS
/// (`tls`) or cleartext, then Hello and CRAM-MD5 (S-27).
pub(crate) fn connect_fd_mode_on(p: &JobParams, sock: std::net::TcpStream, tls: bool)
    -> Result<(BnetStream<bareos_bnet::stream::AnyStream>, Option<String>), FdHandshakeError> {
    use bareos_bnet::stream::AnyStream;
    sock.set_read_timeout(None).ok();
    let (stream, cipher) = if tls {
        let psk = tls_psk::password_to_psk(&p.fd_password);
        let stream = tls_psk::psk_connect(sock, &format!("R_DIRECTOR\x1e{}", p.director_name), &psk)
            .map_err(|e| FdHandshakeError::Tls(e.to_string()))?;
        let cipher = stream.cipher_name();
        (AnyStream::Psk(stream), Some(cipher))
    } else {
        (AnyStream::Plain(sock), None)
    };
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(&p.director_name)).map_err(|e| FdHandshakeError::Hello(e.to_string()))?;
    let auth = CramMd5Auth::new(&p.fd_password, &p.director_name, p.fd_tls_policy);
    match auth.two_way_authenticate(&mut bnet, false) {
        Ok(r) if r.success => {}
        _ => return Err(FdHandshakeError::Auth),
    }
    match bnet.recv_str() {
        Ok(hello) if hello.contains("2000 OK Hello") => Ok((bnet, cipher)),
        Ok(hello) => Err(FdHandshakeError::Response(hello)),
        Err(e) => Err(FdHandshakeError::Response(e.to_string())),
    }
}

/// TLS-PSK, Hello and CRAM-MD5 with the File daemon on a connected socket.
pub(crate) fn connect_fd_on(p: &JobParams, sock: std::net::TcpStream) -> Result<(BnetStream<bareos_bnet::AnyStream>, String), String> {
    sock.set_read_timeout(None).ok();
    // a client with `TLS Enable = no` (policy 0) speaks clear text
    let (stream, cipher) = if p.fd_tls_policy == 0 {
        (bareos_bnet::AnyStream::Plain(sock), String::new())
    } else {
        let psk = tls_psk::password_to_psk(&p.fd_password);
        let stream = tls_psk::psk_connect(sock, &format!("R_DIRECTOR\x1e{}", p.director_name), &psk)
            .map_err(|e| format!("TLS-PSK: {e}"))?;
        let cipher = stream.cipher_name();
        (bareos_bnet::AnyStream::Psk(stream), cipher)
    };
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(&p.director_name)).map_err(|e| format!("hello: {e}"))?;
    let auth = CramMd5Auth::new(&p.fd_password, &p.director_name, p.fd_tls_policy);
    let r = auth.two_way_authenticate(&mut bnet, false).map_err(|e| format!("CRAM: {e}"))?;
    if !r.success { return Err("CRAM failed".into()); }
    let hello = bnet.recv_str().map_err(|e| format!("hello resp: {e}"))?;
    if !hello.contains("2000 OK Hello") { return Err(format!("bad hello: {hello}")); }
    Ok((bnet, cipher))
}

pub(crate) fn connect_sd(p: &JobParams) -> Result<(BnetStream<bareos_bnet::AnyStream>, String), String> {
    // osd: gRPC first (Storage Grpc Enable = yes), BNET on any failure.
    if let Some(sd) = crate::sd_grpc::try_connect(p.sd_grpc.as_ref(), &p.sd_address, p.sd_port, &p.sd_password, &p.director_name, None) {
        return Ok((sd, onesimus_bnet_grpc::TRANSPORT.to_string()));
    }
    let sock = resolve_connect(&format!("{}:{}", p.sd_address, p.sd_port), 3)?;
    connect_sd_on(p, sock)
}

/// TLS-PSK, Hello, CRAM-MD5 and the OK-Hello with the Storage daemon on a
/// connected socket (`osd::connect::connect_sd_on`): a read timeout during
/// the handshake, none afterwards, because a job may wait on the SD for as
/// long as a mount takes.
pub(crate) fn connect_sd_on(p: &JobParams, sock: std::net::TcpStream) -> Result<(BnetStream<bareos_bnet::AnyStream>, String), String> {
    osd::connect::connect_sd_on(sock, &p.sd_password, &p.director_name, None).map_err(|e| e.to_string())
}

fn resolve_connect(addr: &str, timeout_secs: u64) -> Result<std::net::TcpStream, String> {
    use std::net::ToSocketAddrs;
    let sa = addr.to_socket_addrs().map_err(|e| format!("DNS {addr}: {e}"))?
        .next().ok_or_else(|| format!("DNS {addr}: none"))?;
    let sock = std::net::TcpStream::connect_timeout(&sa, std::time::Duration::from_secs(timeout_secs))
        .map_err(|e| format!("connect {addr}: {e}"))?;
    register_cancel_fd(std::os::fd::AsRawFd::as_raw_fd(&sock));
    if WORKER_CANCEL.load(Ordering::SeqCst) {
        let _ = sock.shutdown(std::net::Shutdown::Both);
    }
    Ok(sock)
}

pub(crate) fn send_expect<S: Read + Write>(bnet: &mut BnetStream<S>, cmd: &str, expect: &str, job_id: u32, label: &str) -> Result<String, String> {
    debug!("Worker {job_id} {label}: sending '{}'", cmd.trim());
    bnet.send_str(cmd).map_err(|e| format!("{label} send: {e}"))?;
    loop {
        match bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                let r = String::from_utf8_lossy(&data).to_string();
                if r.contains(expect) { debug!("Worker {job_id} {label}: {}", r.trim()); return Ok(r); }
                if r.starts_with("Jmsg") { forward_daemon_jmsg(job_id, &r); continue; }
                if r.starts_with("2001") { debug!("Worker {job_id} {label} Jmsg: {}", r.trim()); continue; }
                return Err(format!("{label}: {r}"));
            }
            Ok(BnetMessage::Signal(BnetSignal::Eod)) => continue,
            // getmsg.cc BgetDirmsg(): BNET_BTIME is answered with our time
            // (the FD's clock sync in LevelCmd for since_utime).
            Ok(BnetMessage::Signal(BnetSignal::Btime)) => {
                let now_us = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros()).unwrap_or(0);
                bnet.send_str(&format!("btime {now_us}\n")).map_err(|e| format!("{label} send: {e}"))?;
            }
            Ok(BnetMessage::Signal(s)) => { debug!("Worker {job_id} {label}: signal {s:?}"); continue; }
            Err(e) => return Err(format!("{label} recv: {e}")),
        }
    }
}

/// dird/restore.cc BuildRestoreCommand(): "restore replace=%c prelinks=%d
/// where=%s" or "... regexwhere=%s" (regexwhere wins), spaces bashed.
fn fd_restore_command(params: &JobParams) -> String {
    let bash = bareos_format::bash::bash_spaces;
    let (key, value) = match params.regex_where.as_deref() {
        Some(r) => ("regexwhere", bash(r)),
        None => ("where", bash(params.where_path.as_deref().unwrap_or(""))),
    };
    let replace = match params.replace.as_deref() {
        Some("never") => 'n',
        Some("ifnewer") => 'w', // REPLACE_IFNEWER (baconfig.h)
        Some("ifolder") => 'o',
        _ => 'a', // always
    };
    format!("restore replace={replace} prelinks={} {key}={value}\n", u8::from(params.prefix_links))
}

pub(crate) fn now_unix() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }

// ============================================================================
// Signal handlers (worker-side)
// ============================================================================

pub(crate) static WORKER_CANCEL: AtomicBool = AtomicBool::new(false);

/// The job's status was set back to JS_Running after a cancel (S-28): the
/// worker goes on as if it had not been canceled.
pub(crate) fn forget_cancel() {
    WORKER_CANCEL.store(false, Ordering::SeqCst);
}
/// A Jmsg M_FATAL of the worker set JS_FatalError.
static WORKER_FATAL: AtomicBool = AtomicBool::new(false);
static WORKER_PAUSED: AtomicBool = AtomicBool::new(false);

pub fn install_worker_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, worker_sigterm as *const () as libc::sighandler_t);
        libc::signal(libc::SIGUSR1, worker_sigusr1 as *const () as libc::sighandler_t);
        libc::signal(libc::SIGUSR2, worker_sigusr2 as *const () as libc::sighandler_t);
        // The Director blocks SIGINT/SIGTERM in all its threads (main.rs);
        // the forked worker inherits that mask and would never see a cancel.
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGTERM, libc::SIGUSR1, libc::SIGUSR2] {
            libc::sigaddset(&mut set, sig);
        }
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
    }
}

extern "C" fn worker_sigterm(_: libc::c_int) {
    WORKER_CANCEL.store(true, Ordering::SeqCst);
    // CancelJob(): a worker blocked on a daemon socket wakes up (shutdown(2)
    // is async-signal-safe). S-23.
    for slot in &CANCEL_FDS {
        let fd = slot.load(Ordering::SeqCst);
        if fd >= 0 {
            unsafe { libc::shutdown(fd, libc::SHUT_RDWR); }
        }
    }
}

/// The Director's answer to a catalog request of the SD (CatResp on the
/// pipe); `Err` when the Director is gone (EOF, read error) or answered
/// something else. J-15.
pub(crate) fn read_catalog_answer(from_director: &mut std::fs::File) -> Result<String, String> {
    let mut line = String::new();
    match BufReader::new(from_director).read_line(&mut line) {
        Ok(0) => Err("pipe to the Director closed".into()),
        Ok(_) => match serde_json::from_str::<PipeMessage>(&line) {
            Ok(PipeMessage::CatResp { response }) => Ok(response),
            _ => Err(format!("unexpected answer from the Director: {}", line.trim_end())),
        },
        Err(e) => Err(format!("pipe to the Director: {e}")),
    }
}

/// J-15: a catalog request of the SD the Director cannot answer. A worker
/// never answers in the catalog's place (it used to send "1000 OK"): the
/// job ends with an error and its daemon connections are shut down, as
/// they would be if the Director process itself had gone away; the SD sees
/// the lost Director connection (e.g. stored/askdir.cc:438-442 "Error
/// creating JobMedia record", M_FATAL), the FD read fails.
pub(crate) fn abort_without_catalog(job_id: u32, why: &str) {
    error!("Worker {job_id}: catalog request not answered ({why}); aborting the job");
    worker_jmsg(job_id, M_FATAL, format!("Catalog request of the Storage daemon not answered: {why}\n"));
    WORKER_FATAL.store(true, Ordering::SeqCst);
    for slot in &CANCEL_FDS {
        let fd = slot.load(Ordering::SeqCst);
        if fd >= 0 {
            unsafe { libc::shutdown(fd, libc::SHUT_RDWR); }
        }
    }
}

/// The daemon sockets of the worker, shut down by SIGTERM (S-23).
static CANCEL_FDS: [std::sync::atomic::AtomicI32; 4] = [const { std::sync::atomic::AtomicI32::new(-1) }; 4];

/// Remember a daemon socket for the SIGTERM handler.
fn register_cancel_fd(fd: i32) {
    for slot in &CANCEL_FDS {
        if slot.compare_exchange(-1, fd, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
            return;
        }
    }
}
extern "C" fn worker_sigusr1(_: libc::c_int) { WORKER_PAUSED.store(true, Ordering::SeqCst); }
extern "C" fn worker_sigusr2(_: libc::c_int) { WORKER_PAUSED.store(false, Ordering::SeqCst); }

// ============================================================================
// fork() + pipe infrastructure
// ============================================================================

/// Handle to a forked worker (held by Director).
#[derive(Debug)]
pub struct JobHandle {
    pub pid: i32,
    pub job_id: u32,
    pub job_name: String,
    pub unique_job: String,
    from_worker: Option<BufReader<std::fs::File>>,
    to_worker: Option<std::fs::File>,
}

impl JobHandle {
    pub fn try_read_message(&mut self) -> Option<PipeMessage> {
        let r = self.from_worker.as_mut()?;
        // poll() to check if data is available before blocking read
        use std::os::unix::io::AsRawFd;
        let fd = r.get_ref().as_raw_fd();
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut pfd, 1, 0) }; // timeout=0 → non-blocking
        if r.buffer().is_empty() && (ready <= 0 || (pfd.revents & libc::POLLIN) == 0) {
            return None; // no data available
        }
        // Data is available — safe to read_line (will not block)
        let mut buf = String::new();
        match r.read_line(&mut buf) {
            Ok(n) if n > 0 => serde_json::from_str(&buf).ok(),
            _ => None,
        }
    }

    /// A second handle on the pipe to the worker, for an answer prepared
    /// on another thread while the worker waits for nothing else (S-27).
    pub fn sender(&self) -> Option<std::fs::File> {
        self.to_worker.as_ref().and_then(|w| w.try_clone().ok())
    }

    pub fn send_message(&mut self, msg: &PipeMessage) -> bool {
        if let Some(w) = &mut self.to_worker {
            if let Ok(json) = serde_json::to_string(msg) { return writeln!(w, "{json}").is_ok(); }
        }
        false
    }

    pub fn try_wait(&self) -> Option<i32> {
        // A queued job has no process yet; waitpid(0) would reap any child.
        if self.pid <= 0 { return None; }
        use nix::sys::wait::{waitpid, WaitPidFlag};
        use nix::unistd::Pid;
        match waitpid(Pid::from_raw(self.pid), Some(WaitPidFlag::WNOHANG)) {
            Ok(nix::sys::wait::WaitStatus::Exited(_, c)) => Some(c),
            Ok(nix::sys::wait::WaitStatus::Signaled(_, s, _)) => Some(128 + s as i32),
            _ => None,
        }
    }

    /// A handle without pipes for a process the test owns.
    #[cfg(test)]
    pub fn detached(pid: i32, job_id: u32, job_name: &str, unique_job: &str) -> Self {
        Self { pid, job_id, job_name: job_name.into(), unique_job: unique_job.into(), from_worker: None, to_worker: None }
    }

    /// The handle of a queued job whose worker is not forked yet (jobq.cc
    /// waiting queue). Compatibility log C-048.
    pub fn pending(job_id: u32, job_name: &str, unique_job: &str) -> Self {
        Self { pid: 0, job_id, job_name: job_name.into(), unique_job: unique_job.into(), from_worker: None, to_worker: None }
    }

    /// True once a worker process exists for the job.
    pub fn spawned(&self) -> bool { self.pid > 0 }

    pub fn cancel(&self) -> bool { self.pid > 0 && unsafe { libc::kill(self.pid, libc::SIGTERM) == 0 } }
    // kill(0, ...) would signal the whole process group: only real workers.
    pub fn pause(&self)  { if self.pid > 0 { unsafe { libc::kill(self.pid, libc::SIGUSR1); } } }
    pub fn resume(&self) { if self.pid > 0 { unsafe { libc::kill(self.pid, libc::SIGUSR2); } } }
}

/// The descriptors of tokio's process-wide signal pipe (see
/// [`reserve_tokio_signal_fds`]); a worker keeps them open.
static TOKIO_SIGNAL_FDS: std::sync::OnceLock<Vec<i32>> = std::sync::OnceLock::new();

/// The open file descriptors of this process (Linux: `/proc/self/fd`).
fn open_fds() -> std::collections::BTreeSet<i32> {
    std::fs::read_dir("/proc/self/fd")
        .map(|dir| dir.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect())
        .unwrap_or_default()
}

/// Call first in `main`, before any thread or runtime: initialises tokio's
/// process-wide signal pipe now and records its descriptors.
///
/// Every tokio runtime with I/O (the gRPC clients of `fd_grpc`/`sd_grpc`
/// in a worker) clones that pipe. A worker closes all inherited
/// descriptors after `fork`; when the parent had already created the pipe
/// (catalog, gRPC server), the clone then failed with "Bad file descriptor"
/// or, once the number was reused, "Invalid argument", and every worker
/// silently fell back from gRPC to BNET. Keeping these two descriptors
/// open in the worker fixes that.
pub fn reserve_tokio_signal_fds() {
    let before = open_fds();
    if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
        drop(rt);
    }
    let fds = open_fds().difference(&before).copied().collect();
    let _ = TOKIO_SIGNAL_FDS.set(fds);
}

/// Spawn a job worker process with dual pipes.
#[cfg(unix)]
pub fn spawn_job_worker(job_id: u32, job_name: &str, params: &JobParams) -> io::Result<JobHandle> {
    use nix::unistd::{fork, ForkResult, pipe};

    let (w2d_r, w2d_w) = pipe().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let (d2w_r, d2w_w) = pipe().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Prevent inheriting a reload-layer write lock held by another thread.
    let logging_guard = crate::logging::guard_reload_for_fork();
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            drop(logging_guard);
            drop(w2d_w); drop(d2w_r);
            let from_worker = unsafe { std::fs::File::from_raw_fd(w2d_r.into_raw_fd()) };
            let to_worker = unsafe { std::fs::File::from_raw_fd(d2w_w.into_raw_fd()) };
            info!("spawned worker pid={} job={job_id} '{job_name}'", child);
            Ok(JobHandle { pid: child.as_raw(), job_id, job_name: job_name.to_string(), unique_job: params.unique_job.clone(),
                from_worker: Some(BufReader::new(from_worker)), to_worker: Some(to_worker) })
        }
        Ok(ForkResult::Child) => {
            drop(logging_guard);
            drop(w2d_r); drop(d2w_w);
            // Close inherited FDs (TCP listener etc)
            let mut keep = vec![w2d_w.as_raw_fd(), d2w_r.as_raw_fd()];
            // a client initiated connection handed to this job (client_pool.rs)
            keep.extend(crate::client_pool::handoff_fd(job_id));
            keep.extend(TOKIO_SIGNAL_FDS.get().into_iter().flatten());
            for fd in 3..1024 { if !keep.contains(&fd) { unsafe { libc::close(fd); } } }

            #[cfg(target_os = "linux")]
            {
                let proc_name = format!("job:{job_id}");
                use std::ffi::CString; if let Ok(n) = CString::new(proc_name.as_str()) { unsafe { libc::prctl(libc::PR_SET_NAME, n.as_ptr()); } }
            }

            install_worker_signal_handlers();
            eprintln!("[worker:{job_id}] started pid={}", std::process::id());

            let to_dir = unsafe { std::fs::File::from_raw_fd(w2d_w.into_raw_fd()) };
            let from_dir = unsafe { std::fs::File::from_raw_fd(d2w_r.into_raw_fd()) };
            let code = run_worker(params, to_dir, from_dir);
            eprintln!("[worker:{job_id}] exit {code}");
            std::process::exit(code);
        }
        Err(e) => Err(io::Error::new(io::ErrorKind::Other, format!("fork: {e}")))
    }
}

#[cfg(not(unix))]
pub fn spawn_job_worker(_: u32, _: &str, _: &JobParams) -> io::Result<JobHandle> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "fork not available"))
}

#[cfg(test)]
mod wait_protocol_tests {
    use super::*;
    #[test]
    fn restore_command_like_build_restore_command() {
        let mut p = JobParams {
            job_id: 1, unique_job: "R.1".into(), job_name: "R".into(), job_type: WorkerJobType::Restore,
            level: "full".into(), fd_address: String::new(), fd_port: 0, fd_password: String::new(), fd_tls_policy: 2, client_retention: None, fd_grpc: None, sd_grpc: None,
            sd_address: String::new(), sd_port: 0, sd_password: String::new(), sd_device: String::new(),
            sd_media_type: String::new(), director_name: String::new(), pool_name: String::new(),
            fileset_name: String::new(), fileset_includes: vec![], fileset_excludes: vec![], since_utime: 0, prev_job: String::new(),
            where_path: Some("/tmp/with space".into()), regex_where: None, prefix_links: false,
            replace: Some("ifnewer".into()), restore_jobid: None, bootstrap: None,
            client_name: String::new(), client_first_connection: false, passive_client: false, copy: None, verify: None, plugin_options: Vec::new(), backup: None,
        };
        assert_eq!(fd_restore_command(&p), "restore replace=w prelinks=0 where=/tmp/with\x01space\n");
        p.regex_where = Some("!/a!/b!".into());
        p.prefix_links = true;
        p.replace = None;
        assert_eq!(fd_restore_command(&p), "restore replace=a prelinks=1 regexwhere=!/a!/b!\n");
        p.regex_where = None;
        p.where_path = None;
        assert_eq!(fd_restore_command(&p), "restore replace=a prelinks=1 where=\n");
    }
    #[test]
    fn termination_status_is_not_collapsed_to_success_or_error() {
        for (code, status) in [(0,"T"),(84,"T"),(87,"W"),(101,"e"),(69,"E"),(102,"f"),(65,"A"),(68,"D"),(2,"E"),(3,"A"),(999,"E")] {
            assert_eq!(catalog_job_status(code), status);
        }
    }
    #[test]
    fn storage_wait_status_uses_original_numeric_wire_codes() {
        for (code, expected) in [(77, Some("WaitingMount")), (109, Some("WaitingMedia")),
            (82, Some("Running")), (84, None)] {
            assert_eq!(storage_wait_state(&format!("Status Job=Backup.001 JobStatus={code}\n")).unwrap(), expected);
        }
        for invalid in ["Status Job= JobStatus=77", "Status Job=x JobStatus=no", "Status Job=x", "Status Job=x JobStatus=999"] {
            assert!(storage_wait_state(invalid).is_err());
        }
    }
    fn held_worker(messages: &[PipeMessage]) -> (JobHandle, std::process::ChildStdin) {
        held_worker_exit(messages, 0)
    }
    fn held_worker_exit(messages: &[PipeMessage], code: i32) -> (JobHandle, std::process::ChildStdin) {
        use std::process::{Command, Stdio};
        let wire = messages.iter().map(|message| serde_json::to_string(message).unwrap()).collect::<Vec<_>>().join("\n");
        let mut child = Command::new("/bin/sh").args(["-c", "trap '' TERM; printf '%s\n' \"$1\"; read release; exit \"$2\"", "--"])
            .arg(wire).arg(code.to_string()).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let handle = JobHandle {pid: child.id() as i32, job_id: 42, job_name: "Backup".into(), unique_job: "Backup.001".into(),
            from_worker: Some(BufReader::new(unsafe {std::fs::File::from_raw_fd(child.stdout.take().unwrap().into_raw_fd())})), to_worker: None};
        (handle, child.stdin.take().unwrap())
    }
    fn until(manager: &mut crate::job_manager::JobManager, ready: impl Fn(&crate::job_manager::JobManager) -> bool) {
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(3);
        loop { manager.poll_all(); if ready(manager) { break; }
            assert!(std::time::Instant::now()<deadline,"worker lifecycle timed out");
            std::thread::sleep(std::time::Duration::from_millis(1)); }
    }
    fn ending(status: u8) -> PipeMessage { PipeMessage::JobEnd {job_id:42,term_code:status as i32,
        files:12,bytes:1024,errors:1,vol_session_id:None,vol_session_time:None} }
    #[test]
    fn wait_keeps_job_active_until_real_worker_exit_and_final_catalog_update() {
        let (handle,mut release)=held_worker(&[ending(b'W')]);
        let cat=crate::catalog_worker::CatalogHandle::mock(|request| match request {
            crate::catalog_worker::CatalogRequest::UpdateJobEnd {status,job_files,..} => {
                assert_eq!(status,"W");assert_eq!(job_files,12);crate::catalog_worker::CatalogResponse::Ok(1)
            }, _=>panic!("unexpected catalog request"),
        });
        let mut manager=crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)),None,Some(cat));
        manager.add_job(42,"Backup",handle);
        until(&mut manager,|manager| manager.get_job(42).unwrap().phase==crate::job_manager::DirJobPhase::Finishing);
        assert_eq!(manager.active_count(),1);
        writeln!(release,"exit").unwrap();
        until(&mut manager,|manager| manager.active_count()==0);
        assert_eq!(manager.get_job(42).unwrap().phase,crate::job_manager::DirJobPhase::Completed);
        assert_eq!((manager.get_job(42).unwrap().last_files,manager.get_job(42).unwrap().last_bytes),(12,1024));
    }
    #[test]
    fn cancel_waits_for_worker_and_incomplete_status_beats_cancellation() {
        for (updates,expected) in [(vec![ending(b'T')],"A"),
            (vec![PipeMessage::JobStatus{job_id:42,status:b'I' as i32},ending(b'T')],"I")] {
            let (handle,mut release)=held_worker(&updates);
            let cat=crate::catalog_worker::CatalogHandle::mock(move |request| match request {
                crate::catalog_worker::CatalogRequest::UpdateJobEnd {status,job_files,..} => {
                    assert_eq!(status,expected);assert_eq!(job_files,12);crate::catalog_worker::CatalogResponse::Ok(1)
                }, _=>panic!("unexpected catalog request"),
            });
            let mut manager=crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)),None,Some(cat));
            manager.add_job(42,"Backup",handle);
            until(&mut manager,|manager| manager.get_job(42).unwrap().phase==crate::job_manager::DirJobPhase::Finishing);
            assert!(manager.cancel_job(42));assert_eq!(manager.active_count(),1);
            manager.poll_all();assert_eq!(manager.active_count(),1);
            writeln!(release,"exit").unwrap();until(&mut manager,|manager|manager.active_count()==0);
            assert!(!manager.cancel_job(42));
        }
    }
    #[test]
    fn sd_wait_status_is_kept_apart_from_job_status() {
        // Compatibility log C-048: getmsg.cc SetJcrSdJobStatus().
        let (handle,mut release)=held_worker(&[PipeMessage::State{job_id:42,state:"ConnectingSD".into()},
            PipeMessage::State{job_id:42,state:"Running".into()},PipeMessage::JobStatus{job_id:42,status:b'M' as i32}]);
        let mut manager=crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)),None,None);
        manager.add_job(42,"Backup",handle);
        until(&mut manager,|manager| manager.active_jobs()[0].sd_status=='M');
        let job=manager.active_jobs().remove(0);
        assert_eq!((job.status,job.phase,job.waiting(),job.fd_connected,job.sd_connected,job.job_started),
            ('R',crate::job_manager::DirJobPhase::WaitingMount,true,true,true,true));
        writeln!(release,"exit").unwrap();
        until(&mut manager,|manager| manager.active_count()==0);
        assert_eq!((manager.finished_count(),manager.finished_job_ids()),(1,vec![42]));
    }
    #[test]
    fn nonzero_exit_cannot_publish_success_even_after_successful_job_end() {
        let (handle,mut release)=held_worker_exit(&[ending(b'T')],1);
        let cat=crate::catalog_worker::CatalogHandle::mock(|request|match request {
            crate::catalog_worker::CatalogRequest::UpdateJobEnd {status,..} => {assert_eq!(status,"E");crate::catalog_worker::CatalogResponse::Ok(1)},
            _=>panic!("unexpected request"),
        });
        let mut manager=crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)),None,Some(cat));
        manager.add_job(42,"Backup",handle);writeln!(release,"exit").unwrap();
        until(&mut manager,|manager|manager.active_count()==0);
        assert_eq!(manager.get_job(42).unwrap().phase,crate::job_manager::DirJobPhase::Error);
    }
    #[test]
    fn final_catalog_failure_remains_visible_to_selected_and_wait_all_consumers() {
        let since=std::time::Instant::now();
        let (handle,mut release)=held_worker(&[ending(b'T')]);
        let cat=crate::catalog_worker::CatalogHandle::mock(|_|crate::catalog_worker::CatalogResponse::Error("injected update failure".into()));
        let mut manager=crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)),None,Some(cat));
        manager.add_job(42,"Backup",handle);writeln!(release,"exit").unwrap();
        until(&mut manager,|manager| manager.active_count()==0);
        for id in [Some(42),None] {assert!(manager.completion_error(id,since).unwrap().contains("injected update failure"));}
    }
    /// stored/askdir.cc FileAttributes: an UpdCat record of the SD.
    fn upd_cat(file_index: u32, stream: u32, record: &[u8]) -> PipeMessage {
        let mut m = b"UpdCat Job=Backup.001 FileAttributes ".to_vec();
        for v in [1u32, 1_781_000_000, file_index, stream, record.len() as u32] {
            m.extend_from_slice(&v.to_be_bytes());
        }
        m.extend_from_slice(record);
        PipeMessage::UpdCat { job_id: 42, data: hex::encode(m) }
    }
    /// A catalog double: `fail` rejects statements of the job's private
    /// batch connection and of the shared one; UpdateJobEnd goes to `end`.
    fn failing_catalog(fail: &'static str, end: std::sync::mpsc::Sender<(String, i32)>) -> crate::catalog_worker::CatalogHandle {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse, ConnectionOp, SqlTable};
        crate::catalog_worker::CatalogHandle::mock(move |request| match request {
            CatalogRequest::TextRows { .. } => CatalogResponse::Rows(SqlTable::default()),
            CatalogRequest::Execute { sql } if sql.starts_with(fail) => CatalogResponse::Error("server closed the connection unexpectedly".into()),
            CatalogRequest::Dedicated { ops } => {
                std::thread::spawn(move || while let Ok((op, answer)) = ops.recv() {
                    let _ = answer.send(match op {
                        ConnectionOp::Execute(sql) if sql.starts_with(fail) =>
                            CatalogResponse::Error("server closed the connection unexpectedly".into()),
                        ConnectionOp::Rows(_) => CatalogResponse::Rows(SqlTable::default()),
                        ConnectionOp::Execute(_) => CatalogResponse::Ok(0),
                    });
                });
                CatalogResponse::Ok(0)
            }
            CatalogRequest::UpdateJobEnd { status, job_errors, .. } => { end.send((status, job_errors)).unwrap(); CatalogResponse::Ok(1) }
            _ => CatalogResponse::Ok(0),
        })
    }
    /// J-1: the batch insert fails at the end of the job ("Batch end",
    /// M_FATAL): the job ends JS_FatalError, not "OK" (lib/message.cc:1239-1242).
    #[test]
    fn failed_batch_insert_makes_the_job_fatal() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (handle, mut release) = held_worker(&[upd_cat(1, 1, b"1 3 /etc/hosts\0P0A\0\0\0"),
            PipeMessage::JobEnd { job_id: 42, term_code: b'T' as i32, files: 1, bytes: 10, errors: 0, vol_session_id: None, vol_session_time: None }]);
        let mut manager = crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)), None,
            Some(failing_catalog("INSERT INTO batch", tx)));
        manager.add_job(42, "Backup", handle);
        until(&mut manager, |manager| manager.get_job(42).unwrap().phase == crate::job_manager::DirJobPhase::Finishing);
        writeln!(release, "exit").unwrap();
        until(&mut manager, |manager| manager.active_count() == 0);
        assert_eq!(rx.try_recv().unwrap(), ("f".to_string(), 1));
        assert_eq!(manager.get_job(42).unwrap().phase, crate::job_manager::DirJobPhase::Error);
    }
    /// J-1: an M_ERROR of a catalog request counts in JobErrors; a backup
    /// that terminated with errors is "OK -- with warnings" (backup.cc:768-771).
    #[test]
    fn catalog_error_counts_in_job_errors() {
        let (tx, rx) = std::sync::mpsc::channel();
        // A digest without its attribute record: UPDATE File SET MD5 (sql_update.cc).
        let (handle, mut release) = held_worker(&[upd_cat(0, 3, &[7; 16]),
            PipeMessage::JobEnd { job_id: 42, term_code: b'T' as i32, files: 1, bytes: 10, errors: 0, vol_session_id: None, vol_session_time: None }]);
        let mut manager = crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)), None,
            Some(failing_catalog("UPDATE File SET MD5", tx)));
        manager.add_job(42, "Backup", handle);
        until(&mut manager, |manager| manager.get_job(42).unwrap().phase == crate::job_manager::DirJobPhase::Finishing);
        writeln!(release, "exit").unwrap();
        until(&mut manager, |manager| manager.active_count() == 0);
        // UpdateDB() reports the failed statement (sql.cc:274, M_ERROR), then
        // UpdateAttribute() "Catalog error updating file digest" (M_ERROR).
        assert_eq!(rx.try_recv().unwrap(), ("W".to_string(), 2));
    }
    /// J-8: the end status of a restore in the catalog (UpdateJobEnd).
    fn restore_end_status(messages: &[PipeMessage], expected_files: Option<u64>) -> String {
        let (tx, rx) = std::sync::mpsc::channel();
        let (handle, mut release) = held_worker(messages);
        let mut manager = crate::job_manager::JobManager::new(std::sync::Arc::new(crate::message_bus::MessageBus::new(10)), None,
            Some(failing_catalog("nothing fails", tx)));
        manager.add_job(42, "RestoreFiles", handle);
        manager.set_job_report(42, crate::job_messages::JobReport { kind: crate::job_messages::ReportKind::Restore,
            job: "RestoreFiles.001".into(), expected_files, ..Default::default() });
        until(&mut manager, |manager| manager.get_job(42).unwrap().phase == crate::job_manager::DirJobPhase::Finishing);
        writeln!(release, "exit").unwrap();
        until(&mut manager, |manager| manager.active_count() == 0);
        rx.try_recv().unwrap().0
    }
    fn restore_end(files: i64) -> PipeMessage {
        PipeMessage::JobEnd { job_id: 42, term_code: b'T' as i32, files, bytes: 10, errors: 0, vol_session_id: None, vol_session_time: None }
    }
    /// J-8: WaitForJobTermination() (backup.cc:752-757): with the FD at
    /// JS_Terminated the SD's end status is the restore's.
    #[test]
    fn restore_takes_the_storage_daemon_end_status() {
        let sd = |status: u8| PipeMessage::JobStatus { job_id: 42, status: status as i32 };
        assert_eq!(restore_end_status(&[sd(b'R'), sd(b'T'), restore_end(5)], Some(5)), "T");
        assert_eq!(restore_end_status(&[sd(b'R'), sd(b'E'), restore_end(5)], Some(5)), "E");
        assert_eq!(restore_end_status(&[sd(b'A'), restore_end(5)], Some(5)), "A");
        // No final status from the SD: the FD's.
        assert_eq!(restore_end_status(&[sd(b'R'), restore_end(5)], Some(5)), "T");
    }
    /// J-8: restore.cc:435-441: a file count mismatch is JS_Warnings in the
    /// catalog, not only in the report.
    #[test]
    fn restore_file_count_mismatch_is_warnings_in_the_catalog() {
        assert_eq!(restore_end_status(&[restore_end(5)], Some(6)), "W");
        assert_eq!(restore_end_status(&[restore_end(6)], Some(6)), "T");
        assert_eq!(restore_end_status(&[restore_end(6)], None), "T");
    }
    /// J-15: only a CatResp of the Director answers the SD; a closed pipe
    /// (Director gone) or another message is an error, never "1000 OK".
    #[test]
    fn catalog_answer_comes_only_from_the_director() {
        let pipe_with = |lines: &[&str]| {
            let (read, write) = nix::unistd::pipe().unwrap();
            let mut write = std::fs::File::from(write);
            for line in lines { writeln!(write, "{line}").unwrap(); }
            drop(write);
            std::fs::File::from(read)
        };
        let answer = serde_json::to_string(&PipeMessage::CatResp { response: "1000 OK VolName=Full-0001\n".into() }).unwrap();
        assert_eq!(read_catalog_answer(&mut pipe_with(&[&answer])).unwrap(), "1000 OK VolName=Full-0001\n");
        assert_eq!(read_catalog_answer(&mut pipe_with(&[])).unwrap_err(), "pipe to the Director closed");
        let other = serde_json::to_string(&PipeMessage::ResetStatus { job_id: 42 }).unwrap();
        assert!(read_catalog_answer(&mut pipe_with(&[&other])).unwrap_err().starts_with("unexpected answer from the Director: "));
        assert!(read_catalog_answer(&mut pipe_with(&["{torn"])).is_err());
    }
    /// J-17: backup.cc:690-701: only the complete EndJob message ends the
    /// wait for the FD; "End Job" without TermCode is not a success.
    #[test]
    fn end_job_needs_the_complete_endjob_format() {
        assert_eq!(end_job_of("2800 End Job TermCode=84 JobFiles=5 ReadBytes=60 JobBytes=50 Errors=1 VSS=0 Encrypt=0"),
            Some((5, 50, 1, 84)));
        assert_eq!(end_job_of("2800 End Job TermCode=69 JobFiles=0 ReadBytes=0 JobBytes=0 Errors=2 VSS=0 Encrypt=0\n"),
            Some((0, 0, 2, 69)));
        for incomplete in ["2800 End Job", "2800 End Job JobFiles=5 JobBytes=50 Errors=0",
            "2800 End Job TermCode=84 JobFiles=5", "2800 End Job TermCode=T JobFiles=5 ReadBytes=60 JobBytes=50 Errors=1 VSS=0 Encrypt=0"] {
            assert_eq!(end_job_of(incomplete), None, "{incomplete}");
        }
    }
    #[test]
    fn consecutive_pipe_messages_survive_reader_buffering_and_eof() {
        let (read, write) = nix::unistd::pipe().unwrap();
        let mut write = std::fs::File::from(write);
        for state in ["WaitingMedia", "Running"] {
            writeln!(write, "{}", serde_json::to_string(&PipeMessage::State {job_id: 42, state: state.into()}).unwrap()).unwrap();
        }
        drop(write);
        let mut handle = JobHandle { pid: -1, job_id: 42, job_name: "Backup".into(), unique_job: "Backup.001".into(),
            from_worker: Some(BufReader::new(std::fs::File::from(read))), to_worker: None };
        for expected in ["WaitingMedia", "Running"] {
            assert!(matches!(handle.try_read_message(), Some(PipeMessage::State {state,..}) if state == expected));
        }
        assert!(handle.try_read_message().is_none());
    }
}

#[cfg(test)]
mod job_message_tests {
    use super::*;

    /// restore.cc DoNativeRestore / bsr.cc OpenBootstrapFile: the restore
    /// job's bootstrap file, else the M_FATAL text; never one built otherwise.
    #[test]
    fn restore_job_reads_only_its_bootstrap_file() {
        let fatal = restore_bootstrap(1, None, "r").unwrap_err();
        assert_eq!(fatal, "Cannot restore without a bootstrap file.\nYou probably ran a restore job directly. \
            All restore jobs must\nbe run using the restore command.\n");
        assert_eq!(restore_bootstrap(1, Some(""), "r").unwrap_err(), fatal);
        let dir = std::env::temp_dir();
        let missing = dir.join(format!("onesimus-test-{}-missing.bsr", std::process::id())).display().to_string();
        assert_eq!(restore_bootstrap(1, Some(&missing), "r").unwrap_err(),
            format!("Could not open bootstrap file {missing}: ERR=No such file or directory\n"));
        let bsr = dir.join(format!("onesimus-test-{}-read.bsr", std::process::id())).display().to_string();
        std::fs::write(&bsr, "Storage=\"Tape\"\nVolume=\"V1\"\n").unwrap();
        assert_eq!(restore_bootstrap(1, Some(&bsr), "r").unwrap(), "Storage=\"Tape\"\nVolume=\"V1\"\n");
        let _ = std::fs::remove_file(&bsr);
    }
    use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
    use std::sync::{Arc, Mutex, RwLock};

    #[test]
    fn daemon_job_messages_are_parsed_like_bget_dirmsg() {
        assert_eq!(parse_daemon_jmsg("Jmsg Job=backup.2026 type=6 level=1783627202 bareos-sd JobId 626: Ready to append\n"),
            Some((6, 1_783_627_202, "bareos-sd JobId 626: Ready to append\n".to_string())));
        assert_eq!(parse_daemon_jmsg("Jmsg Job=x type=4 level=0 "), Some((4, 0, String::new())));
        for malformed in ["Jmsg Job= type=6 level=1 m", "Jmsg Job=x type=a level=1 m", "Jmsg Job=x level=1", "Jmsgx"] {
            assert_eq!(parse_daemon_jmsg(malformed), None, "{malformed}");
        }
    }

    fn rows(columns: &[&str], values: Vec<Vec<&str>>) -> CatalogResponse {
        CatalogResponse::Rows(SqlTable {
            columns: columns.iter().map(|c| SqlColumn { name: c.to_string(), numeric: false }).collect(),
            rows: values.into_iter().map(|r| r.into_iter().map(|v| Some(v.to_string())).collect()).collect(),
        })
    }

    /// A worker double: a shell that writes `messages` on the pipe and exits.
    fn worker(messages: &[PipeMessage]) -> JobHandle {
        use std::process::{Command, Stdio};
        let wire = messages.iter().map(|m| serde_json::to_string(m).unwrap()).collect::<Vec<_>>().join("\n");
        let mut child = Command::new("/bin/sh").args(["-c", "printf '%s\n' \"$1\"", "--"]).arg(wire)
            .stdout(Stdio::piped()).spawn().unwrap();
        JobHandle { pid: child.id() as i32, job_id: 42, job_name: "Backup".into(), unique_job: "Backup.2026-10-16_10.00.00_01".into(),
            from_worker: Some(BufReader::new(unsafe { std::fs::File::from_raw_fd(child.stdout.take().unwrap().into_raw_fd()) })),
            to_worker: None }
    }

    #[test]
    fn job_messages_reach_console_queue_and_log_table_in_order() {
        let queue = crate::console_msg::test_queue("job-messages");
        let config = crate::director_config::DirectorConfig::from_str(r#"
Director { Name = bareos-dir; Password = secret; Messages = Daemon }
Messages { Name = Daemon; console = all }
Messages { Name = Standard; console = all, !skipped, !saved, !audit; catalog = all, !skipped, !saved, !audit }
Client { Name = bareos-fd; Address = localhost; Password = pw; Catalog = MyCatalog }
Job { Name = Backup; Type = Backup; Client = bareos-fd; FileSet = SelfTest; Storage = File; Pool = Full; Messages = Standard }
"#).unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let catalog = CatalogHandle::mock(move |request| match request {
            CatalogRequest::UpdateJobEnd { .. } => CatalogResponse::Ok(1),
            CatalogRequest::TextRows { sql } => {
                if sql.starts_with("INSERT INTO Log") { seen.lock().unwrap().push(sql); return rows(&[], vec![]); }
                if sql.starts_with("SELECT ClientId") { return rows(&["c", "n", "uname"], vec![vec!["1", "bareos-fd", "u"]]); }
                if sql.starts_with("SELECT VolumeName") { return rows(&["v", "m"], vec![vec!["Full-0001", "1"]]); }
                if sql.starts_with("SELECT VolBytes") { return rows(&["b"], vec![vec!["1000"]]); }
                rows(&[], vec![])
            }
            _ => CatalogResponse::Error("unexpected".into()),
        });
        crate::job_messages::set_catalog(Some(catalog.clone()));
        let messages = [
            PipeMessage::Jmsg { job_id: 42, msg_type: 6, mtime: 0, text: "Using Device \"FileStorage\" to write.\n".into(), raw: false },
            PipeMessage::Jmsg { job_id: 42, msg_type: 6, mtime: 1_783_627_202,
                text: "bareos-sd JobId 42: Ready to append to end of Volume \"Full-0001\" size=1\n".into(), raw: true },
            PipeMessage::JobStatus { job_id: 42, status: b'T' as i32 },
            PipeMessage::JobEnd { job_id: 42, term_code: b'T' as i32, files: 5, bytes: 2000, errors: 0,
                vol_session_id: Some(1), vol_session_time: Some(2) },
        ];
        let shared = Arc::new(RwLock::new(config));
        let mut manager = crate::job_manager::JobManager::new(Arc::new(crate::message_bus::MessageBus::new(10)),
            Some(shared), Some(catalog));
        manager.add_job(42, "Backup", worker(&messages));
        manager.set_job_report(42, crate::job_messages::JobReport { job: "Backup.2026-10-16_10.00.00_01".into(),
            level: "Full".into(), client: "bareos-fd".into(), trigger: "User".into(), ..Default::default() });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while manager.active_count() > 0 {
            manager.poll_all();
            assert!(std::time::Instant::now() < deadline, "worker double timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let lines: Vec<String> = queue.take().into_iter().map(|l| String::from_utf8(l).unwrap()).collect();
        let bodies: Vec<&str> = lines.iter().map(|l| l.splitn(3, ' ').nth(2).unwrap_or("")).collect();
        assert_eq!(bodies[0], "bareos-dir JobId 42: Using Device \"FileStorage\" to write.\n");
        assert_eq!(bodies[1], "bareos-sd JobId 42: Ready to append to end of Volume \"Full-0001\" size=1\n");
        assert_eq!(bodies[2], format!("bareos-dir JobId 42: Bareos bareos-dir {} ({}):\n",
            crate::version_strings::FULL, crate::version_strings::SHORT_DATE));
        assert!(lines.iter().any(|l| l == "  Termination:            Backup OK\n"), "{lines:?}");
        let inserts = log.lock().unwrap().clone();
        assert_eq!(inserts.len(), 3);
        assert!(inserts[0].starts_with("INSERT INTO Log (JobId, Time, LogText) VALUES (42,'"));
        assert!(inserts[0].ends_with("','bareos-dir JobId 42: Using Device \"FileStorage\" to write.\n')"));
        // the daemon's mtime (local time; other tests may switch TZ meanwhile)
        assert!(inserts[1].starts_with("INSERT INTO Log (JobId, Time, LogText) VALUES (42,'2026-07-"), "{}", inserts[1]);
        assert!(inserts[1].ends_with(":00:02','bareos-sd JobId 42: Ready to append to end of Volume \"Full-0001\" size=1\n')"),
            "{}", inserts[1]);
        assert!(inserts[2].contains("  Volume name(s):         Full-0001\n"));
        assert!(inserts[2].contains("  Last Volume Bytes:      1,000 (1.000 KB)\n"), "{}", inserts[2]);
        crate::job_messages::set_catalog(None);
    }

    #[test]
    fn console_jcr_messages_use_the_director_messages_resource() {
        let queue = crate::console_msg::test_queue("console-jcr");
        let config = crate::director_config::DirectorConfig::from_str(r#"
Director { Name = bareos-dir; Password = secret; Messages = Daemon }
Messages { Name = Daemon; console = all, !skipped, !saved, !audit }
"#).unwrap();
        crate::console_msg::job_message(&config, "-Console-", 0, crate::message_bus::MsgType::Warning, 0,
            "Error getting Job record for Job rerun: ERR=x\n");
        let lines: Vec<String> = queue.take().into_iter().map(|l| String::from_utf8(l).unwrap()).collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].ends_with(" bareos-dir JobId 0: Warning: Error getting Job record for Job rerun: ERR=x\n"));
    }
}
