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

/// Parameters passed from Director to Worker.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobParams {
    pub job_id: u32,
    pub unique_job: String,
    pub job_name: String,
    pub job_type: WorkerJobType,
    pub level: String,
    pub fd_address: String,
    pub fd_port: u16,
    pub fd_password: String,
    pub sd_address: String,
    pub sd_port: u16,
    pub sd_password: String,
    pub sd_device: String,
    pub sd_media_type: String,
    pub director_name: String,
    pub pool_name: String,
    pub fileset_includes: Vec<String>,
    pub fileset_excludes: Vec<String>,
    // Incremental/Differential: timestamp of reference backup
    pub since_utime: i64,          // 0 = Full, >0 = epoch of last relevant backup
    pub prev_job: String,          // unique job name of reference backup (for incr/diff)
    // Restore-specific
    pub where_path: Option<String>,
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
}

/// lib/message.h M_* codes of the worker's own job messages.
const M_FATAL: u32 = 3;
const M_ERROR: u32 = 4;
const M_INFO: u32 = 6;

/// The worker's pipe to the Director for job messages (any thread).
static JMSG_PIPE: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

/// Write one pipe message as one line in a single write.
fn write_pipe_line(file: &mut std::fs::File, msg: &PipeMessage) {
    if let Ok(mut json) = serde_json::to_string(msg) {
        json.push('\n');
        let _ = file.write_all(json.as_bytes());
    }
}

fn send_jmsg(msg: PipeMessage) {
    let mut pipe = JMSG_PIPE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(file) = pipe.as_mut() {
        write_pipe_line(file, &msg);
    }
}

/// Jmsg(jcr, type, 0, text) of the worker's job.
fn worker_jmsg(job_id: u32, msg_type: u32, text: String) {
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
fn forward_daemon_jmsg(job_id: u32, line: &str) {
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
struct WorkerPipes {
    to_director: std::fs::File,
    from_director: std::fs::File,
}

impl WorkerPipes {
    fn send(&mut self, msg: &PipeMessage) {
        if let Ok(json) = serde_json::to_string(msg) {
            let _ = writeln!(self.to_director, "{json}");
        }
    }

    fn recv(&mut self) -> Option<PipeMessage> {
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

    let mut fd: Option<BnetStream<tls_psk::PskStream>> = None;
    let mut sd: Option<BnetStream<tls_psk::PskStream>> = None;
    let mut sd_auth_key = String::new();
    let mut vol_session_id: i32 = 0;
    let mut vol_session_time: i32 = 0;
    let mut bootstrap = String::new();

    // admin.cc do_admin(): "Start Admin JobId %d, Job=%s", then
    // AdminCleanup(jcr, JS_Terminated) in the Director (job end).
    if params.job_type == WorkerJobType::Admin {
        // job.cc job_thread(): "Version: …" when the job runs.
        worker_jmsg(job_id, M_INFO, crate::job_messages::version_text());
        worker_jmsg(job_id, M_INFO, crate::job_messages::start_admin_text(job_id, &params.unique_job));
        pipes.send(&PipeMessage::JobEnd { job_id, term_code: b'T' as i32, files: 0, bytes: 0, errors: 0,
            vol_session_id: Some(0), vol_session_time: Some(0) });
        return EXIT_SUCCESS;
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
                match bareos_stored::job::sd_send_bootstrap(sd.as_mut().unwrap(), &bootstrap) {
                    Ok(()) => {
                        debug!("Worker {job_id}: bootstrap sent to SD, sending run");
                        // After bootstrap, SD expects "run" — then enters WaitFD()
                        let _ = sd.as_mut().unwrap().send_str("run\n");
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
                match result {
                    Ok(()) => WorkerState::FDStorage,
                    Err(e) => WorkerState::Error(format!("FD setup: {e}"))
                }
            }

            WorkerState::FDStorage => {
                // Restore: storage command includes Authorization (FD uses it to auth to SD)
                let cmd = if params.job_type == WorkerJobType::Restore {
                    format!("storage address={} port={} ssl=4 Authorization={}\n",
                        params.sd_address, params.sd_port, sd_auth_key)
                } else {
                    format!("storage address={} port={} ssl=4\n",
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
                let where_path = params.where_path.as_deref().unwrap_or("/tmp/bareos-restore");
                let replace_char = match params.replace.as_deref() {
                    Some("never") => 'n',
                    Some("ifnewer") => 'i',
                    Some("ifolder") => 'o',
                    _ => 'a', // always
                };
                let cmd = format!("restore replace={replace_char} prelinks=0 where={where_path}\n");
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
                if let Some(ref mut f) = fd { let _ = f.send_signal(BnetSignal::Terminate); }
                if let Some(ref mut s) = sd { let _ = s.send_signal(BnetSignal::Terminate); }
                pipes.send(&PipeMessage::JobEnd {
                    job_id, term_code: if WORKER_CANCEL.load(Ordering::SeqCst) { b'A' as i32 } else { b'E' as i32 }, files: 0, bytes: 0, errors: 1,
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
    mut fd: BnetStream<tls_psk::PskStream>,
    mut sd: BnetStream<tls_psk::PskStream>,
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
// SD Protocol helpers — delegated to bareos-stored crate
// ============================================================================

use bareos_stored::job::{SdSession, SdJobParams, SdJobType};

fn sd_job_setup(sd: &mut BnetStream<tls_psk::PskStream>, p: &JobParams) -> Result<SdSession, String> {
    bareos_stored::job::sd_job_setup(sd, &to_sd_params(p))
}

/// The device the Storage daemon accepted (OK_device "3000 OK use device
/// device=%s", unbashed); its job messages are forwarded.
fn sd_use_device(sd: &mut BnetStream<tls_psk::PskStream>, p: &JobParams) -> Result<String, String> {
    let (answer, jmsgs) = bareos_stored::job::sd_use_device(sd, &to_sd_params(p))?;
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
        client_address: p.fd_address.clone(),
        media_type: p.sd_media_type.clone(),
        pool_name: p.pool_name.clone(),
        device_name: p.sd_device.clone(),
    }
}

/// FD setup for restore: JobId + storage only — NO level, NO fileset.
/// C++ Director sends: JobId → storage → session → restore
/// See: core/src/dird/fd_cmds.cc SendRunscriptsToFd / DoRestoreInit
fn fd_restore_setup(fd: &mut BnetStream<tls_psk::PskStream>, p: &JobParams, sd_auth_key: &str) -> Result<(), String> {
    let cmd = format!("JobId={} Job={} SDid=1 SDtime={} Authorization={sd_auth_key} ssl=4\n",
        p.job_id, p.unique_job, now_unix());
    send_expect(fd, &cmd, "2000 OK Job", p.job_id, "FD JobId")?;
    // getSecureEraseCmd
    fd.send_str("getSecureEraseCmd\n").map_err(|e| format!("send secErase: {e}"))?;
    let _ = fd.recv_str(); // 2000 OK FDSecureEraseCmd *None*
    // Restore: NO level command, NO fileset — go straight to storage
    Ok(())
}

// Bareos getmsg.cc: Status Job=%127s JobStatus=%d; job_status.h: M/m/R.
fn storage_job_status(message: &str) -> Result<u8, &'static str> {
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

fn sd_catrec_loop(
    sd: &mut BnetStream<tls_psk::PskStream>,
    job_id: u32,
    to_dir: &mut std::fs::File,
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
                    let pipe_msg = PipeMessage::UpdCat { job_id, data: hex::encode(&data) };
                    if let Ok(json) = serde_json::to_string(&pipe_msg) {
                        let _ = writeln!(to_dir, "{json}");
                    }
                } else if matches!(data.first(), Some(b'C' | b'B')) {
                    debug!("Worker {job_id} SD CatReq: {msg}");
                    let pipe_msg = PipeMessage::CatReq { job_id, request: msg_raw.into_owned() };
                    if let Ok(json) = serde_json::to_string(&pipe_msg) {
                        let _ = writeln!(to_dir, "{json}");
                    }
                    // Read CatResp from Director via pipe
                    let mut buf = String::new();
                    let mut reader = BufReader::new(&mut *from_dir);
                    match reader.read_line(&mut buf) {
                        Ok(n) if n > 0 => {
                            debug!("Worker {job_id} CatResp pipe ({n} bytes): {}", buf.trim());
                            match serde_json::from_str::<PipeMessage>(&buf) {
                                Ok(PipeMessage::CatResp { response }) => {
                                    debug!("Worker {job_id} → SD: {}", response.chars().take(60).collect::<String>());
                                    if !response.is_empty() {
                                        let _ = sd.send_str(&response);
                                    }
                                }
                                other => {
                                    warn!("Worker {job_id}: unexpected pipe msg: {other:?}, sending OK");
                                    let _ = sd.send_str("1000 OK\n");
                                }
                            }
                        }
                        Ok(_) => {
                            warn!("Worker {job_id}: pipe EOF while waiting for CatResp");
                            let _ = sd.send_str("1000 OK\n");
                        }
                        Err(e) => {
                            error!("Worker {job_id}: pipe read error: {e}");
                            let _ = sd.send_str("1000 OK\n");
                        }
                    }
                } else if msg.starts_with("Status Job=") {
                    match storage_job_status(&msg) {
                        Ok(status) => {
                            let update = PipeMessage::JobStatus { job_id, status: status as i32 };
                            if let Ok(json) = serde_json::to_string(&update) { let _ = writeln!(to_dir, "{json}"); }
                        }
                        Err(error) => warn!("Worker {job_id}: {error}"),
                    }
                } else if msg.starts_with("3099 Job") {
                    if let Some(status) = msg.split_whitespace().find_map(|part| part.strip_prefix("JobStatus=").and_then(|value| value.parse::<u8>().ok())) {
                        let update = PipeMessage::JobStatus { job_id, status: status as i32 };
                        if let Ok(json) = serde_json::to_string(&update) { let _ = writeln!(to_dir, "{json}"); }
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
            Err(e) => { debug!("Worker {job_id} SD recv: {e}"); break; }
        }
    }
    let _ = sd.send_signal(BnetSignal::Terminate);
    debug!("Worker {job_id}: SD CatReq thread ended");
}

// ============================================================================
// FD Protocol helpers
// ============================================================================

fn fd_job_setup(fd: &mut BnetStream<tls_psk::PskStream>, p: &JobParams, sd_auth_key: &str) -> Result<(), String> {
    let cmd = format!("JobId={} Job={} SDid=1 SDtime={} Authorization={sd_auth_key} ssl=4\n", p.job_id, p.unique_job, now_unix());
    send_expect(fd, &cmd, "2000 OK Job", p.job_id, "FD JobId")?;
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

fn wait_for_fd_end_job(fd: &mut BnetStream<tls_psk::PskStream>, job_id: u32) -> (i64, i64, i32, i32) {
    loop {
        match fd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let msg = String::from_utf8_lossy(&data).trim().to_string();
                if msg.starts_with("2800 End Job") {
                    info!("Worker {job_id}: FD End Job: {msg}");
                    let mut tc = 0i32; let mut f = 0i64; let mut b = 0i64; let mut e = 0i32;
                    for part in msg.split_whitespace() {
                        if let Some(v) = part.strip_prefix("TermCode=") { tc = v.parse().unwrap_or(0); }
                        else if let Some(v) = part.strip_prefix("JobFiles=") { f = v.parse().unwrap_or(0); }
                        else if let Some(v) = part.strip_prefix("JobBytes=") { b = v.parse().unwrap_or(0); }
                        else if let Some(v) = part.strip_prefix("Errors=") { e = v.parse().unwrap_or(0); }
                    }
                    return (f, b, e, tc);
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
                if matches!(sig, BnetSignal::Terminate) { return (0, 0, 1, 2); }
            }
            Err(e) => {
                error!("Worker {job_id} FD recv error: {e}");
                return (0, 0, 1, 2);
            }
        }
    }
}

// ============================================================================
// Connection helpers
// ============================================================================

fn connect_fd(p: &JobParams) -> Result<(BnetStream<tls_psk::PskStream>, String), String> {
    let sock = resolve_connect(&format!("{}:{}", p.fd_address, p.fd_port), 3)?;
    sock.set_read_timeout(None).ok();
    let psk = tls_psk::password_to_psk(&p.fd_password);
    let stream = tls_psk::psk_connect(sock, &format!("R_DIRECTOR\x1e{}", p.director_name), &psk)
        .map_err(|e| format!("TLS-PSK: {e}"))?;
    let cipher = stream.cipher_name();
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(&p.director_name)).map_err(|e| format!("hello: {e}"))?;
    let auth = CramMd5Auth::new(&p.fd_password, &p.director_name, 2);
    let r = auth.two_way_authenticate(&mut bnet, false).map_err(|e| format!("CRAM: {e}"))?;
    if !r.success { return Err("CRAM failed".into()); }
    let hello = bnet.recv_str().map_err(|e| format!("hello resp: {e}"))?;
    if !hello.contains("2000 OK Hello") { return Err(format!("bad hello: {hello}")); }
    Ok((bnet, cipher))
}

fn connect_sd(p: &JobParams) -> Result<(BnetStream<tls_psk::PskStream>, String), String> {
    let sock = resolve_connect(&format!("{}:{}", p.sd_address, p.sd_port), 3)?;
    sock.set_read_timeout(None).ok();
    let psk = tls_psk::password_to_psk(&p.sd_password);
    let stream = tls_psk::psk_connect(sock, &format!("R_DIRECTOR\x1e{}", p.director_name), &psk)
        .map_err(|e| format!("TLS-PSK: {e}"))?;
    let cipher = stream.cipher_name();
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(&p.director_name)).map_err(|e| format!("hello: {e}"))?;
    let auth = CramMd5Auth::new(&p.sd_password, &p.director_name, 2);
    let r = auth.two_way_authenticate(&mut bnet, false).map_err(|e| format!("CRAM: {e}"))?;
    if !r.success { return Err("CRAM failed".into()); }
    let _ = bnet.recv_str(); // 3000 OK Hello
    Ok((bnet, cipher))
}

fn resolve_connect(addr: &str, timeout_secs: u64) -> Result<std::net::TcpStream, String> {
    use std::net::ToSocketAddrs;
    let sa = addr.to_socket_addrs().map_err(|e| format!("DNS {addr}: {e}"))?
        .next().ok_or_else(|| format!("DNS {addr}: none"))?;
    std::net::TcpStream::connect_timeout(&sa, std::time::Duration::from_secs(timeout_secs))
        .map_err(|e| format!("connect {addr}: {e}"))
}

fn send_expect(bnet: &mut BnetStream<tls_psk::PskStream>, cmd: &str, expect: &str, job_id: u32, label: &str) -> Result<String, String> {
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
            Ok(BnetMessage::Signal(s)) => { debug!("Worker {job_id} {label}: signal {s:?}"); continue; }
            Err(e) => return Err(format!("{label} recv: {e}")),
        }
    }
}

fn now_unix() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64 }

// ============================================================================
// Signal handlers (worker-side)
// ============================================================================

static WORKER_CANCEL: AtomicBool = AtomicBool::new(false);
static WORKER_PAUSED: AtomicBool = AtomicBool::new(false);

pub fn install_worker_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, worker_sigterm as libc::sighandler_t);
        libc::signal(libc::SIGUSR1, worker_sigusr1 as libc::sighandler_t);
        libc::signal(libc::SIGUSR2, worker_sigusr2 as libc::sighandler_t);
    }
}

extern "C" fn worker_sigterm(_: libc::c_int) { WORKER_CANCEL.store(true, Ordering::SeqCst); }
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
            let keep = [w2d_w.as_raw_fd(), d2w_r.as_raw_fd()];
            for fd in 3..1024 { if !keep.contains(&fd) { unsafe { libc::close(fd); } } }

            let proc_name = format!("job:{job_id}");
            #[cfg(target_os = "linux")]
            { use std::ffi::CString; if let Ok(n) = CString::new(proc_name.as_str()) { unsafe { libc::prctl(libc::PR_SET_NAME, n.as_ptr()); } } }

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
