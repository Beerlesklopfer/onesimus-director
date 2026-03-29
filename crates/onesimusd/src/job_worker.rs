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
    pub bootstrap: Option<String>,      // Bootstrap data (volumes + file indices)
}

/// Messages on the Worker↔Director pipe (JSON, one per line).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum PipeMessage {
    #[serde(rename = "state")]
    State { job_id: u32, state: String },
    #[serde(rename = "status")]
    Status { job_id: u32, files: i64, bytes: i64, errors: i32 },
    #[serde(rename = "cat_req")]
    CatReq { job_id: u32, request: String },
    #[serde(rename = "cat_resp")]
    CatResp { response: String },
    #[serde(rename = "job_end")]
    JobEnd { job_id: u32, term_code: i32, files: i64, bytes: i64, errors: i32,
             #[serde(default)] vol_session_id: Option<i32>,
             #[serde(default)] vol_session_time: Option<i32> },
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
    shm: Option<crate::ipc_shm::ShmWorkerEnd>,
) -> i32 {
    let mut pipes = WorkerPipes { to_director, from_director };
    let mut state = WorkerState::Init;
    let job_id = params.job_id;
    let mut shm_worker = shm;

    let mut fd: Option<BnetStream<tls_psk::PskStream>> = None;
    let mut sd: Option<BnetStream<tls_psk::PskStream>> = None;
    let mut sd_auth_key = String::new();
    let mut vol_session_id: i32 = 0;
    let mut vol_session_time: i32 = 0;

    loop {
        let old_name = state.name().to_string();
        state = match state {
            WorkerState::Init => {
                WorkerState::ConnectingFD
            }

            WorkerState::ConnectingFD => {
                match connect_fd(params) {
                    Ok(b) => { fd = Some(b); info!("Worker {job_id}: FD connected"); WorkerState::ConnectingSD }
                    Err(e) => WorkerState::Error(format!("FD connect: {e}"))
                }
            }

            WorkerState::ConnectingSD => {
                match connect_sd(params) {
                    Ok(b) => { sd = Some(b); info!("Worker {job_id}: SD connected"); WorkerState::SDJobSetup }
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
                    Ok(()) => WorkerState::SDRun,
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
                match sd_send_bootstrap(sd.as_mut().unwrap(), params) {
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
                let result = run_backup_phase(fd_stream, sd_stream, job_id, &mut pipes, shm_worker.take());
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
                // TermCode: 0=OK, 84='T'=Success, 87='W'=Warnings, 69='E'=Error, 102='f'=Fatal
                let success = term_code == 0 || term_code == 84 || term_code == 87; // T or W
                if success && errors == 0 {
                    WorkerState::Completed { files, bytes, errors }
                } else {
                    WorkerState::Error(format!("TermCode={term_code} Errors={errors}"))
                }
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
                    job_id, term_code: 2, files: 0, bytes: 0, errors: 1,
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
    shm: Option<crate::ipc_shm::ShmWorkerEnd>,
) -> Result<(i64, i64, i32, i32), String> {
    // SD CatReq thread — use SHM if available, pipe as fallback
    let sd_handle = if let Some(mut shm_end) = shm {
        thread::Builder::new()
            .name(format!("sd-cat:{job_id}"))
            .spawn(move || {
                sd_catrec_loop_shm(&mut sd, job_id, &mut shm_end);
            })
            .map_err(|e| format!("sd thread: {e}"))?
    } else {
        let mut pipe_to = pipes.to_director.try_clone().map_err(|e| format!("pipe clone: {e}"))?;
        let mut pipe_from = pipes.from_director.try_clone().map_err(|e| format!("pipe clone: {e}"))?;
        thread::Builder::new()
            .name(format!("sd-cat:{job_id}"))
            .spawn(move || {
                sd_catrec_loop_pipe(&mut sd, job_id, &mut pipe_to, &mut pipe_from);
            })
            .map_err(|e| format!("sd thread: {e}"))?
    };

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

fn sd_use_device(sd: &mut BnetStream<tls_psk::PskStream>, p: &JobParams) -> Result<(), String> {
    bareos_stored::job::sd_use_device(sd, &to_sd_params(p))
}

fn sd_send_bootstrap(sd: &mut BnetStream<tls_psk::PskStream>, p: &JobParams) -> Result<(), String> {
    let bsr = p.bootstrap.as_deref().unwrap_or("");
    bareos_stored::job::sd_send_bootstrap(sd, bsr)
}

fn to_sd_params(p: &JobParams) -> SdJobParams {
    SdJobParams {
        job_id: p.job_id,
        unique_job: p.unique_job.clone(),
        job_name: p.job_name.clone(),
        job_type: match p.job_type { WorkerJobType::Backup => SdJobType::Backup, WorkerJobType::Restore => SdJobType::Restore },
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

/// SD CatReq loop via iceoryx2 shared memory (zero-copy IPC).
fn sd_catrec_loop_shm(
    sd: &mut BnetStream<tls_psk::PskStream>,
    job_id: u32,
    shm: &mut crate::ipc_shm::ShmWorkerEnd,
) {
    debug!("Worker {job_id}: SD CatReq loop started (SHM)");
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let msg = String::from_utf8_lossy(&data).trim().to_string();
                if msg.starts_with("CatReq") {
                    debug!("Worker {job_id} CatReq: {}", &msg[..msg.len().min(60)]);
                    // Send via shared memory — ~1μs instead of ~10ms pipe roundtrip
                    match shm.request(&msg) {
                        Ok(response) => {
                            debug!("Worker {job_id} → SD: {}", &response[..response.len().min(60)]);
                            let _ = sd.send_str(&response);
                        }
                        Err(e) => {
                            error!("Worker {job_id} SHM CatReq failed: {e}");
                            let _ = sd.send_str("1000 OK\n");
                        }
                    }
                } else if msg.starts_with("3099 Job") {
                    debug!("Worker {job_id} SD: job end");
                    break;
                } else if msg.starts_with("3010 Job") {
                    debug!("Worker {job_id} SD: job start");
                } else if msg.starts_with("Jmsg") {
                    debug!("Worker {job_id} SD Jmsg: {msg}");
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
    debug!("Worker {job_id}: SD CatReq loop ended (SHM)");
}

/// Legacy pipe-based SD CatReq loop (fallback if SHM not available).
fn sd_catrec_loop_pipe(
    sd: &mut BnetStream<tls_psk::PskStream>,
    job_id: u32,
    to_dir: &mut std::fs::File,
    from_dir: &mut std::fs::File,
) {
    debug!("Worker {job_id}: SD CatReq thread started (pipe)");
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let msg = String::from_utf8_lossy(&data).trim().to_string();
                if msg.starts_with("CatReq") {
                    debug!("Worker {job_id} SD CatReq: {msg}");
                    let pipe_msg = PipeMessage::CatReq { job_id, request: msg };
                    if let Ok(json) = serde_json::to_string(&pipe_msg) {
                        let _ = writeln!(to_dir, "{json}");
                    }
                    let mut buf = String::new();
                    let mut reader = BufReader::new(&mut *from_dir);
                    match reader.read_line(&mut buf) {
                        Ok(n) if n > 0 => {
                            if let Ok(PipeMessage::CatResp { response }) = serde_json::from_str(&buf) {
                                let _ = sd.send_str(&response);
                            } else {
                                let _ = sd.send_str("1000 OK\n");
                            }
                        }
                        _ => { let _ = sd.send_str("1000 OK\n"); }
                    }
                } else if msg.starts_with("3099 Job") { break; }
                  else if msg.starts_with("Jmsg") { debug!("Worker {job_id} Jmsg: {msg}"); }
            }
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
            Ok(BnetMessage::Signal(_)) => {}
            Err(_) => break,
        }
    }
    let _ = sd.send_signal(BnetSignal::Terminate);
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
            Ok(r) if r.starts_with("Jmsg") => continue,
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

fn connect_fd(p: &JobParams) -> Result<BnetStream<tls_psk::PskStream>, String> {
    let sock = resolve_connect(&format!("{}:{}", p.fd_address, p.fd_port), 3)?;
    sock.set_read_timeout(None).ok();
    let psk = tls_psk::password_to_psk(&p.fd_password);
    let stream = tls_psk::psk_connect(sock, &format!("R_DIRECTOR\x1e{}", p.director_name), &psk)
        .map_err(|e| format!("TLS-PSK: {e}"))?;
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(&p.director_name)).map_err(|e| format!("hello: {e}"))?;
    let auth = CramMd5Auth::new(&p.fd_password, &p.director_name, 2);
    let r = auth.two_way_authenticate(&mut bnet, false).map_err(|e| format!("CRAM: {e}"))?;
    if !r.success { return Err("CRAM failed".into()); }
    let hello = bnet.recv_str().map_err(|e| format!("hello resp: {e}"))?;
    if !hello.contains("2000 OK Hello") { return Err(format!("bad hello: {hello}")); }
    Ok(bnet)
}

fn connect_sd(p: &JobParams) -> Result<BnetStream<tls_psk::PskStream>, String> {
    let sock = resolve_connect(&format!("{}:{}", p.sd_address, p.sd_port), 3)?;
    sock.set_read_timeout(None).ok();
    let psk = tls_psk::password_to_psk(&p.sd_password);
    let stream = tls_psk::psk_connect(sock, &format!("R_DIRECTOR\x1e{}", p.director_name), &psk)
        .map_err(|e| format!("TLS-PSK: {e}"))?;
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(&p.director_name)).map_err(|e| format!("hello: {e}"))?;
    let auth = CramMd5Auth::new(&p.sd_password, &p.director_name, 2);
    let r = auth.two_way_authenticate(&mut bnet, false).map_err(|e| format!("CRAM: {e}"))?;
    if !r.success { return Err("CRAM failed".into()); }
    let _ = bnet.recv_str(); // 3000 OK Hello
    Ok(bnet)
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
                if r.starts_with("Jmsg") || r.starts_with("2001") { debug!("Worker {job_id} {label} Jmsg: {}", r.trim()); continue; }
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
    from_worker: Option<std::fs::File>,
    to_worker: Option<std::fs::File>,
}

impl JobHandle {
    pub fn try_read_message(&mut self) -> Option<PipeMessage> {
        let r = self.from_worker.as_mut()?;
        // poll() to check if data is available before blocking read
        use std::os::unix::io::AsRawFd;
        let fd = r.as_raw_fd();
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut pfd, 1, 0) }; // timeout=0 → non-blocking
        if ready <= 0 || (pfd.revents & libc::POLLIN) == 0 {
            return None; // no data available
        }
        // Data is available — safe to read_line (will not block)
        let mut buf = String::new();
        let mut reader = BufReader::new(&*r);
        match reader.read_line(&mut buf) {
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
        use nix::sys::wait::{waitpid, WaitPidFlag};
        use nix::unistd::Pid;
        match waitpid(Pid::from_raw(self.pid), Some(WaitPidFlag::WNOHANG)) {
            Ok(nix::sys::wait::WaitStatus::Exited(_, c)) => Some(c),
            Ok(nix::sys::wait::WaitStatus::Signaled(_, s, _)) => Some(128 + s as i32),
            _ => None,
        }
    }

    pub fn cancel(&self) { unsafe { libc::kill(self.pid, libc::SIGTERM); } }
    pub fn pause(&self)  { unsafe { libc::kill(self.pid, libc::SIGUSR1); } }
    pub fn resume(&self) { unsafe { libc::kill(self.pid, libc::SIGUSR2); } }
}

/// Spawn a job worker process with dual pipes.
#[cfg(unix)]
/// Spawn result includes optional SHM director end for CatReq handling.
pub struct SpawnResult {
    pub handle: JobHandle,
    pub shm_director: Option<crate::ipc_shm::ShmDirectorEnd>,
}

pub fn spawn_job_worker(job_id: u32, job_name: &str, params: &JobParams) -> io::Result<SpawnResult> {
    use nix::unistd::{fork, ForkResult, pipe};

    let (w2d_r, w2d_w) = pipe().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let (d2w_r, d2w_w) = pipe().map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // Create SHM channel BEFORE fork — both processes inherit the shared memory
    let shm = crate::ipc_shm::create_shm_channel(job_id);
    let (shm_dir, shm_worker) = match shm {
        Ok((d, w)) => {
            info!("SHM channel created for job {job_id}");
            (Some(d), Some(w))
        }
        Err(e) => {
            warn!("SHM channel failed for job {job_id}: {e} — falling back to pipes");
            (None, None)
        }
    };

    match unsafe { fork() } {
        Ok(ForkResult::Parent { child }) => {
            drop(w2d_w); drop(d2w_r);
            drop(shm_worker); // parent doesn't need worker end
            let from_worker = unsafe { std::fs::File::from_raw_fd(w2d_r.into_raw_fd()) };
            let to_worker = unsafe { std::fs::File::from_raw_fd(d2w_w.into_raw_fd()) };
            info!("spawned worker pid={} job={job_id} '{job_name}' shm={}", child, shm_dir.is_some());
            Ok(SpawnResult {
                handle: JobHandle { pid: child.as_raw(), job_id, job_name: job_name.to_string(),
                    from_worker: Some(from_worker), to_worker: Some(to_worker) },
                shm_director: shm_dir,
            })
        }
        Ok(ForkResult::Child) => {
            drop(w2d_r); drop(d2w_w);
            drop(shm_dir); // child doesn't need director end
            // Close inherited FDs (TCP listener etc)
            let keep = [w2d_w.as_raw_fd(), d2w_r.as_raw_fd()];
            for fd in 3..1024 { if !keep.contains(&fd) { unsafe { libc::close(fd); } } }

            let proc_name = format!("job:{job_id}");
            #[cfg(target_os = "linux")]
            { use std::ffi::CString; if let Ok(n) = CString::new(proc_name.as_str()) { unsafe { libc::prctl(libc::PR_SET_NAME, n.as_ptr()); } } }

            install_worker_signal_handlers();
            eprintln!("[worker:{job_id}] started pid={} shm={}", std::process::id(), shm_worker.is_some());

            let to_dir = unsafe { std::fs::File::from_raw_fd(w2d_w.into_raw_fd()) };
            let from_dir = unsafe { std::fs::File::from_raw_fd(d2w_r.into_raw_fd()) };
            let code = run_worker(params, to_dir, from_dir, shm_worker);
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
