//! Daemon side of a native Backup job in the forked job worker (S-19,
//! Compatibility log C-058): backup.cc DoNativeBackup() in its order —
//! UpdateJobStartRecord, Storage daemon connection and Job command
//! (msgchan.cc StartStorageDaemonJob), File daemon connection,
//! SendJobInfoToFileDaemon (Client Uname), SendLevelCommand,
//! SendIncludeExcludeLists, SendSecureEraseReqToFd, the second
//! UpdateJobStartRecord, ReserveWriteDevice, ConfigureMessageThread
//! ("storage address=", SD "run", message thread), "backup FileIndex=0"
//! and WaitForJobTermination (FD "2800 End Job", SD "3099 Job end").

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::Ordering;

use bareos_bnet::stream::AnyStream;
use bareos_bnet::tls_psk::PskStream;
use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};
use tracing::{debug, warn};

use crate::backup::{BackupEnd, ClientRetention, JobIds};
use crate::job_worker::{self, JobParams, PipeMessage, WorkerPipes, M_INFO};

const M_FATAL: u32 = 3;
const M_WARNING: u32 = 5;
const M_ERROR: u32 = 4;

/// What the worker needs beyond JobParams (JobParams.backup).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BackupSession {
    /// Catalog ids and the effective level of the Job record.
    pub ids: JobIds,
    /// SendSinceTime(): starttime_string as utime and PrevJob.
    pub since_utime: i64,
    pub prev_job: String,
    pub accurate: bool,
    /// Client resource values for UpdateClientRecord().
    pub client_retention: ClientRetention,
    /// SendIncludeExcludeLists(): "fileset…" and the item lines (None: nothing sent).
    pub fileset_lines: Option<Vec<String>>,
    /// The SD Job command fields (type 'B', level, names, FileSet MD5, spooling).
    pub sd_job: crate::sd_copy::CopySession,
    /// The write Storage (ReserveWriteDevice) and its address for the FD.
    pub write: crate::sd_copy::StorageUse,
    /// SendPluginOptions(): "pluginoptions …" lines (S-23).
    #[serde(default)]
    pub plugin_options: Vec<String>,
    /// SendPreviousRestoreObjects(): header, name and object messages (S-23).
    #[serde(default)]
    pub restore_objects: Vec<Vec<u8>>,
    /// SendBwlimitToFd(): bytes per second, 0 for none (S-23).
    #[serde(default)]
    pub max_bandwidth: u64,
    /// Client Passive = yes: the SD connects to the FD (S-23).
    #[serde(default)]
    pub passive_client: bool,
    /// StorageAddressToContact() / ClientAddressToContact() (LanAddress).
    #[serde(default)]
    pub storage_contact: String,
    #[serde(default)]
    pub client_contact: String,
    /// IsChecksumNeededByFileset(): the accurate list carries checksums.
    #[serde(default)]
    pub accurate_checksum: bool,
    /// The client's quota (quota.cc), None without Soft/Hard Quota (S-23).
    #[serde(default)]
    pub quota: Option<crate::backup::Quota>,
    /// The handshake mode to start with is cleartext (stored, or the client
    /// has TLS Enable = no); the client requires TLS (TLS Require, default
    /// yes: no cleartext fallback). S-27.
    #[serde(default)]
    pub handshake_cleartext: bool,
    #[serde(default)]
    pub client_tls_required: bool,
    /// Director FD/SD Connect Timeout (max_retry_time of the connections).
    #[serde(default)]
    pub fd_connect_timeout: u64,
    #[serde(default)]
    pub sd_connect_timeout: u64,
}

/// fd_cmds.cc ConnectToFileDaemon(): the TCP connection (retried), then the
/// handshake in the stored mode or, probing, immediate TLS first and after
/// a TLS failure cleartext (UpdateFailedConnectionHandshakeMode), at most
/// three tries. Err: the fatal text TerminateBackupWithError starts with
/// (empty when the messages were already sent). S-27.
fn connect_file_daemon(params: &JobParams, session: &BackupSession) -> Result<BnetStream<AnyStream>, String> {
    if let Some((stream, _)) = crate::fd_grpc::try_worker(params) {
        state(params.job_id, "BackupFDConnected");
        jmsg(params.job_id, M_INFO, format!("Connected Client: {} via gRPC TLS\n", params.client_name));
        return Ok(stream);
    }
    use job_worker::FdHandshakeError;
    let job_id = params.job_id;
    let probing = params.client_first_connection;
    let mut tls = !session.handshake_cleartext;
    let who = format!("Client: {}", params.client_name);
    for _ in 0..3 {
        let sock = match connect_retry(job_id, &who, &params.fd_address, params.fd_port, session.fd_connect_timeout) {
            Ok(sock) => sock,
            Err(e) => {
                if !e.is_empty() {
                    jmsg(job_id, M_FATAL, e);
                }
                return Err(format!("\nFailed to connect to client \"{}\".\n", params.client_name));
            }
        };
        // jcr->file_bsock is set once the TCP connection stands (a cancel
        // now goes to the File daemon).
        state(job_id, "BackupFDConnected");
        match job_worker::connect_fd_mode_on(params, sock, tls) {
            Ok((fd, cipher)) => {
                jmsg(job_id, M_INFO, crate::job_messages::connected_text(&who, &params.fd_address, params.fd_port, cipher.as_deref()));
                // SendInfoSuccess(), SendInfoChosenCipher()
                let mode = if tls { "Immediate TLS" } else { "Cleartext" };
                let text = format!("{} Handshake: {mode},", if probing { "\r\u{b}" } else { "" });
                jmsg(job_id, M_INFO, format!("{}\n", text.replace(['\r', '\u{b}', ','], " ")));
                jmsg(job_id, M_INFO, crate::job_messages::cipher_text(cipher.as_deref()));
                job_worker::send_jmsg(PipeMessage::ClientHandshake { job_id, client: params.client_name.clone(), cleartext: !tls });
                return Ok(fd);
            }
            Err(FdHandshakeError::Tls(error)) => {
                // OpensslPostErrors(): one message per OpenSSL error. (A cancel
                // shut the socket down; the original's handshake fails when the
                // connection breaks, the same way.)
                for err in openssl_errors(&error) {
                    jmsg(job_id, M_FATAL, format!("lib/tls_openssl_private.cc:403 Connect failure: ERR={err}\n"));
                }
                if probing {
                    jmsg(job_id, M_INFO, "TLS negotiation failed (while probing client protocol)\n".into());
                } else {
                    jmsg(job_id, M_FATAL, "TLS negotiation failed\n".into());
                }
            }
            Err(FdHandshakeError::Hello(e)) => jmsg(job_id, M_FATAL, format!(
                "Error sending Hello to File daemon at \"{}:{}\". ERR={e}\n", params.fd_address, params.fd_port)),
            Err(FdHandshakeError::Auth) => jmsg(job_id, M_FATAL, format!(
                "Unable to authenticate with File daemon at \"{}:{}\"\n", params.fd_address, params.fd_port)),
            Err(FdHandshakeError::Response(e)) => jmsg(job_id, M_FATAL, format!(
                "Bad response from File daemon at \"{}:{}\" to Hello command: ERR={e}\n", params.fd_address, params.fd_port)),
        }
        // UpdateFailedConnectionHandshakeMode(): after TLS first the job is
        // JS_Running again and falls back to cleartext unless the client
        // requires TLS; cleartext has failed.
        if tls {
            // jcr->setJobStatus(JS_Running): also a cancel is forgotten.
            FIRST_ERROR.store(0, Ordering::SeqCst);
            if job_worker::WORKER_CANCEL.load(Ordering::SeqCst) {
                job_worker::forget_cancel();
            }
            job_worker::send_jmsg(PipeMessage::ResetStatus { job_id });
            if session.client_tls_required {
                break;
            }
            tls = false;
        } else {
            break;
        }
    }
    Err(String::new())
}

/// ERR_error_string_n() of each OpenSSL error in a handshake failure text
/// ("error:0A00041B:SSL routines::tlsv1 alert decrypt error").
fn openssl_errors(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("error:") {
        let fields: Vec<&str> = rest[at..].splitn(6, ':').collect();
        if fields.len() >= 5 && fields[1].len() == 8 && fields[1].chars().all(|c| c.is_ascii_hexdigit()) {
            // OpenSSL 3 has no function names in ERR_error_string_n().
            out.push(format!("{}:{}:{}::{}", fields[0], fields[1], fields[2], fields[4]));
        }
        rest = &rest[at + 6..];
    }
    out
}

/// lib/bsock_tcp.cc BareosSocketTCP::connect(): the TCP connection, tried
/// every 10 seconds; the warning comes with the second failure (then every
/// 5 minutes), the fatal "Unable to connect" after `max_retry` seconds. A
/// canceled job stops trying (Err with an empty text).
fn connect_retry(job_id: u32, name: &str, host: &str, port: u16, max_retry: u64) -> Result<std::net::TcpStream, String> {
    const RETRY_INTERVAL: i64 = 10;
    let begin = now();
    let mut i: i64 = 0;
    loop {
        let err = match job_worker::tcp_connect(host, port) {
            Ok(sock) => return Ok(sock),
            Err(e) => crate::commands::restore::errno_text(&e),
        };
        if job_worker::WORKER_CANCEL.load(Ordering::SeqCst) {
            return Err(String::new());
        }
        if i < 0 {
            i = 60 * 5;
            jmsg(job_id, M_WARNING, format!("lib/bsock_tcp.cc:171 Could not connect to {name} on {host}:{port}. ERR={err}\nRetrying ...\n"));
        }
        for _ in 0..RETRY_INTERVAL {
            if job_worker::WORKER_CANCEL.load(Ordering::SeqCst) {
                return Err(String::new());
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        if begin + max_retry as i64 <= now() {
            return Err(format!("lib/bsock_tcp.cc:179 Unable to connect to {name} on {host}:{port}. ERR={err}\n"));
        }
        i -= RETRY_INTERVAL;
    }
}

fn send_catalog(job_id: u32, sql: String) {
    job_worker::send_jmsg(PipeMessage::CatalogSql { job_id, sql });
}

fn jmsg(job_id: u32, msg_type: u32, text: String) {
    note_type(msg_type);
    job_worker::worker_jmsg(job_id, msg_type, text);
}

/// SendSecureEraseReqToFd / SendSecureEraseReqToSd ran.
static FD_ERASE_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SD_ERASE_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// jcr->JobStatus of the job: the first error status wins
/// (setJobStatusWithPriorityCheck); 0 while there is none.
static FIRST_ERROR: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// jcr->JobErrors: a fatal message makes it at least 1, every error adds one.
static JOB_ERRORS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn set_status(status: u8) {
    let _ = FIRST_ERROR.compare_exchange(0, status, Ordering::SeqCst, Ordering::SeqCst);
}

fn canceled() -> bool {
    let canceled = job_worker::WORKER_CANCEL.load(Ordering::SeqCst);
    if canceled {
        set_status(b'A');
    }
    canceled
}

/// lib/message.cc Jmsg(): M_FATAL sets JS_FatalError (and JobErrors to at
/// least 1), M_ERROR counts a job error; also for daemon job messages.
fn note_type(msg_type: u32) {
    canceled();
    match msg_type {
        M_FATAL => {
            set_status(b'f');
            let _ = JOB_ERRORS.compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
        }
        M_ERROR => {
            JOB_ERRORS.fetch_add(1, Ordering::SeqCst);
        }
        _ => {}
    }
}

/// A daemon's "Jmsg Job=.. type=.. level=.." line: BgetDirmsg() hands it to
/// DispatchMessage(), which leaves JobStatus and JobErrors alone.
fn forward(job_id: u32, line: &str) {
    job_worker::forward_daemon_jmsg(job_id, line);
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

/// getmsg.cc BgetDirmsg(): the next message that is not a job message;
/// signals are answered (BTIME, POLL, STATUS) or returned as None.
fn dir_msg<S: Read + Write>(bs: &mut BnetStream<S>, job_id: u32) -> Result<Option<String>, String> {
    loop {
        match bs.recv().map_err(|e| e.to_string())? {
            BnetMessage::Data(data) => {
                let text = String::from_utf8_lossy(&data).into_owned();
                if text.starts_with("Jmsg") {
                    forward(job_id, &text);
                    continue;
                }
                return Ok(Some(text));
            }
            BnetMessage::Signal(BnetSignal::Btime) => {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                bs.send_str(&format!("btime {}\n", t.as_micros())).map_err(|e| e.to_string())?;
            }
            BnetMessage::Signal(BnetSignal::EodPoll) => {
                bs.send_str("1000 OK\n").map_err(|e| e.to_string())?;
                return Ok(None);
            }
            BnetMessage::Signal(BnetSignal::Poll) => bs.send_str("1000 OK\n").map_err(|e| e.to_string())?,
            BnetMessage::Signal(BnetSignal::Heartbeat | BnetSignal::HeartbeatResponse) => {}
            BnetMessage::Signal(BnetSignal::Status) => {
                bs.send_str("Status OK\n").map_err(|e| e.to_string())?;
                bs.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
            }
            BnetMessage::Signal(_) => return Ok(None),
        }
    }
}

/// getmsg.cc response(): the next message must be `expected`; otherwise
/// "<cmd> command: wanted …, got …" (M_FATAL, DISPLAY_ERROR).
fn response<S: Read + Write>(bs: &mut BnetStream<S>, job_id: u32, expected: &str, cmd: &str) -> Result<(), String> {
    match dir_msg(bs, job_id) {
        Ok(Some(msg)) if msg == expected => Ok(()),
        Ok(Some(msg)) => Err(format!("Bad response to {cmd} command: wanted {expected}, got {msg}\n")),
        Ok(None) => Err(format!("Socket error on {cmd} command: ERR=No data available\n")),
        Err(e) => Err(format!("Socket error on {cmd} command: ERR={e}\n")),
    }
}

fn send<S: Read + Write>(bs: &mut BnetStream<S>, text: &str) -> Result<(), String> {
    bs.send_str(text).map_err(|e| format!("Network error: {e}\n"))
}

/// Counters of the SD's "3099 Job %s end JobStatus=%d JobFiles=%d JobBytes=%lld JobErrors=%u".
#[derive(Default)]
struct SdEnd {
    status: u8,
    files: u64,
    bytes: u64,
    errors: u32,
}

/// The job end the worker reports; `term` is the job status character.
#[allow(clippy::too_many_arguments)]
fn finish(job_id: u32, session: &BackupSession, term: u8, files: i64, bytes: i64, errors: i32,
          vol: (u32, u32), counts: Option<(u8, u64, &SdEnd, bool, bool)>) -> i32 {
    let (fd_status, read_bytes, sd, vss, encrypt) = match counts {
        Some((fd, read, sd, vss, enc)) => (fd as char, read, Some(sd), vss, enc),
        // No "2800 End Job": the Director decides (FD reached or not).
        None => ('\0', 0, None, false, false),
    };
    job_worker::send_jmsg(PipeMessage::BackupEnd { job_id, end: BackupEnd {
        ids: session.ids, fd_status, read_bytes,
        sd_status: sd.map(|s| s.status as char).unwrap_or('\0'),
        sd_files: sd.map(|s| s.files).unwrap_or(0), sd_bytes: sd.map(|s| s.bytes).unwrap_or(0),
        sd_errors: sd.map(|s| s.errors).unwrap_or(0), vss, encrypt,
        fd_erase_asked: FD_ERASE_ASKED.load(Ordering::SeqCst), sd_erase_asked: SD_ERASE_ASKED.load(Ordering::SeqCst),
    }});
    job_worker::send_jmsg(PipeMessage::JobEnd { job_id, term_code: term as i32, files, bytes, errors,
        vol_session_id: Some(vol.0 as i32), vol_session_time: Some(vol.1 as i32) });
    if matches!(term, b'T' | b'W') { job_worker::EXIT_SUCCESS } else { job_worker::EXIT_ERROR }
}

fn state(job_id: u32, name: &str) {
    job_worker::send_jmsg(PipeMessage::State { job_id, state: name.into() });
}

/// DoNativeBackup() in the worker.
pub fn run(params: &JobParams, session: &BackupSession, pipes: &mut WorkerPipes) -> i32 {
    let job_id = params.job_id;
    let ids = session.ids;
    // DoNativeBackup() returned false: NativeBackupCleanup(JS_ErrorTerminated)
    // with the status the job already has (a fatal message: JS_FatalError).
    let fail = |text: String, vol: (u32, u32)| {
        if !text.is_empty() && !canceled() {
            jmsg(job_id, M_FATAL, text);
        }
        set_status(b'E');
        let status = FIRST_ERROR.load(Ordering::SeqCst);
        finish(job_id, session, status, 0, 0, JOB_ERRORS.load(Ordering::SeqCst) as i32, vol, None)
    };

    // job_thread(): "Version: …"; DoNativeBackup(): "Start Backup JobId %u, Job=%s".
    jmsg(job_id, M_INFO, crate::job_messages::version_text());
    // job_thread(): RunScripts "BeforeJob" in the Director (S-07).
    let before = job_worker::run_before_job(pipes, job_id);
    if crate::job_manager::is_job_canceled(before.status) {
        return finish(job_id, session, before.status as u8, 0, 0, before.errors as i32, (0, 0), None);
    }
    jmsg(job_id, M_INFO, crate::job_messages::start_backup_text(job_id, &params.unique_job));

    // jcr->setJobStatus(JS_Running); UpdateJobStartRecord().
    let start = now();
    send_catalog(job_id, crate::backup::update_job_start_record_sql(job_id, 'R', &ids, start, 0, 0));

    // CheckHardquotas() / CheckSoftquotas(); FetchRemainingQuotas() for the SD.
    let mut remaining_quota = 0;
    if session.quota.is_some() {
        pipes.send(&PipeMessage::QuotaCheck { job_id });
        loop {
            match pipes.recv() {
                Some(PipeMessage::QuotaChecked { fatal: Some(text), .. }) => return fail(text, (0, 0)),
                Some(PipeMessage::QuotaChecked { remaining, .. }) => {
                    remaining_quota = remaining;
                    break;
                }
                Some(_) => continue,
                None => break,
            }
        }
    }

    // ConnectToStorageDaemon() (JS_WaitSD) and StartStorageDaemonJob().
    state(job_id, "BackupConnectingSD");
    let sd = connect_retry(job_id, "Storage daemon", &params.sd_address, params.sd_port, session.sd_connect_timeout)
        .and_then(|sock| job_worker::connect_sd_on(params, sock).map_err(|e| format!("Failed to connect to Storage daemon: {e}\n")));
    let mut sd = match sd {
        Ok((sd, cipher)) => {
            jmsg(job_id, M_INFO, crate::job_messages::connected_text("Storage daemon", &params.sd_address, params.sd_port, Some(&cipher)));
            jmsg(job_id, M_INFO, crate::job_messages::cipher_text(Some(&cipher)));
            sd
        }
        Err(e) => return fail(e, (0, 0)),
    };
    state(job_id, "BackupSDJobSetup");
    let (vol, auth_key) = match start_sd_job(&mut sd, params, session, remaining_quota) {
        Ok(v) => v,
        Err(e) => {
            let _ = sd.send_signal(BnetSignal::Terminate);
            return fail(e, (0, 0));
        }
    };

    // ConnectToFileDaemon() (JS_WaitFD).
    state(job_id, "BackupConnectingFD");
    if params.client_first_connection {
        jmsg(job_id, M_INFO, crate::job_messages::PROBING_TEXT.to_string());
    }
    let mut fd = match connect_file_daemon(params, session) {
        Ok(fd) => fd,
        Err(text) => {
            if !text.is_empty() {
                jmsg(job_id, M_FATAL, text);
            }
            return terminate(job_id, session, None, None, Some(sd), vol, before.errors);
        }
    };
    state(job_id, "FDJobSetup");
    if let Err(e) = setup_fd(&mut fd, params, session, vol, &auth_key, &before) {
        // TerminateBackupWithError(): WaitForJobTermination() on the FD.
        if !e.is_empty() {
            jmsg(job_id, M_FATAL, e);
        }
        return terminate(job_id, session, Some(&mut fd), None, Some(sd), vol, before.errors);
    }

    // The job start record again, after the client run scripts.
    let start = now();
    send_catalog(job_id, crate::backup::update_job_start_record_sql(job_id, 'R', &ids, start, vol.0, vol.1));

    // SendAccurateCurrentFiles()
    if let Err(e) = send_accurate_files(&mut fd, pipes, job_id, session, start) {
        // TerminateBackupWithError(): WaitForJobTermination() on the FD.
        if !e.is_empty() {
            jmsg(job_id, M_FATAL, e);
        }
        return terminate(job_id, session, Some(&mut fd), None, Some(sd), vol, before.errors);
    }

    // ReserveWriteDevice().
    if let Err(e) = reserve_write_device(&mut sd, job_id, &session.write) {
        let _ = fd.send_signal(BnetSignal::Terminate);
        let _ = sd.send_signal(BnetSignal::Terminate);
        return fail(e, vol);
    }

    // ConfigureMessageThread(): the FD's storage address (or, for a passive
    // client, the SD connects to the FD), SD "run", message thread.
    let contact = if session.passive_client {
        let cmd = format!("passive client address={} port={} ssl=4\n", session.client_contact, params.fd_port);
        send(&mut sd, &cmd).and_then(|_| {
            std::thread::sleep(std::time::Duration::from_secs(2));
            response(&mut sd, job_id, "2000 OK passive client\n", "Passive client")
        })
    } else {
        let cmd = format!("storage address={} port={} ssl=4\n", session.storage_contact, session.write.port);
        send(&mut fd, &cmd).and_then(|_| response(&mut fd, job_id, "2000 OK storage\n", "Storage"))
    };
    if let Err(e) = contact {
        // TerminateBackupWithError(): WaitForJobTermination() on the FD.
        if !e.is_empty() {
            jmsg(job_id, M_FATAL, e);
        }
        return terminate(job_id, session, Some(&mut fd), None, Some(sd), vol, before.errors);
    }
    if sd.send_str("run").is_err() {
        let _ = fd.send_signal(BnetSignal::Terminate);
        return fail("Network error with SD\n".into(), vol);
    }
    let from_director = match pipes.from_director.try_clone() {
        Ok(f) => f,
        Err(e) => return fail(format!("Cannot create message thread: {e}\n"), vol),
    };
    let sd_thread = std::thread::Builder::new().name(format!("sd-msg:{job_id}"))
        .spawn(move || message_thread(&mut sd, job_id, from_director));
    let Ok(sd_thread) = sd_thread else {
        return fail("Cannot create message thread\n".into(), vol);
    };

    // jcr->file_bsock->fsend(backupcmd, jcr->JobFiles)
    state(job_id, "FDBackup");
    if let Err(e) = send(&mut fd, "backup FileIndex=0\n").and_then(|_| response(&mut fd, job_id, "2000 OK backup\n", "Backup")) {
        jmsg(job_id, M_FATAL, e);
    } else {
        state(job_id, "Running");
    }

    terminate(job_id, session, Some(&mut fd), Some(sd_thread), None, vol, before.errors)
}

/// backup.cc WaitForJobTermination() and the status NativeBackupCleanup
/// gets: the FD's messages up to its end ("2800 End Job", "Unexpected Client
/// Job message", "Network error with FD …"), TERMINATE, the SD message
/// thread, "No Job status returned from FD."; the first error status of the
/// Director, FD or SD. `sd` is the SD connection without message thread.
#[allow(clippy::too_many_arguments)]
fn terminate(job_id: u32, session: &BackupSession, fd: Option<&mut BnetStream<AnyStream>>,
             sd_thread: Option<std::thread::JoinHandle<Option<SdEnd>>>, sd: Option<BnetStream<PskStream>>,
             vol: (u32, u32), before_errors: u32) -> i32 {
    if sd_thread.is_none() {
        // TerminateBackupWithError(): JS_ErrorTerminated.
        set_status(b'E');
    }
    let mut fd_end: Option<(u8, u64, u64, u64, u32, bool, bool)> = None;
    let mut fd_error = None;
    if let Some(fd) = fd {
        loop {
            match dir_msg(fd, job_id) {
                Ok(Some(msg)) => {
                    match fd_end.is_none().then(|| parse_end_job(&msg)).flatten() {
                        Some(end) => fd_end = Some(end),
                        None => jmsg(job_id, M_WARNING, format!("Unexpected Client Job message: {msg}")),
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    fd_error = Some(e);
                    break;
                }
            }
            if canceled() {
                break;
            }
        }
        // A canceled job's SD connection dropped first (its message thread
        // reports that before the FD's network error).
        // A socket the cancel shut down reads like the original's timed-out one.
        if canceled() && fd_error.is_some() {
            fd_error = Some("Resource temporarily unavailable".into());
        }
        if fd_error.is_none() || !canceled() {
            if let Some(e) = fd_error.take() {
                jmsg(job_id, M_FATAL, format!("Network error with FD during Backup: ERR={e}\n"));
            }
        }
        let _ = fd.send_signal(BnetSignal::Terminate);
        state(job_id, "Finishing");
        let sd_end = sd_thread.and_then(|t| t.join().ok().flatten());
        if let Some(e) = fd_error {
            jmsg(job_id, M_FATAL, format!("Network error with FD during Backup: ERR={e}\n"));
        }
        return finish_wait(job_id, session, fd_end, sd_end, vol, before_errors);
    }
    if let Some(mut sd) = sd {
        let _ = sd.send_signal(BnetSignal::Terminate);
    }
    state(job_id, "Finishing");
    let sd_end = sd_thread.and_then(|t| t.join().ok().flatten());
    finish_wait(job_id, session, None, sd_end, vol, before_errors)
}

/// The end of WaitForJobTermination(): the FD's counters or "No Job status
/// returned from FD." (FDJobStatus JS_ErrorTerminated), then the status.
fn finish_wait(job_id: u32, session: &BackupSession, fd_end: Option<(u8, u64, u64, u64, u32, bool, bool)>,
               sd_end: Option<SdEnd>, vol: (u32, u32), before_errors: u32) -> i32 {
    let sd_end = sd_end.unwrap_or_default();
    let Some((fd_status, files, read_bytes, bytes, errors, vss, encrypt)) = fd_end else {
        jmsg(job_id, M_FATAL, "No Job status returned from FD.\n".into());
        canceled();
        let status = FIRST_ERROR.load(Ordering::SeqCst);
        let errors = JOB_ERRORS.load(Ordering::SeqCst) + before_errors;
        return finish(job_id, session, status, 0, 0, errors as i32, vol, Some((b'E', 0, &sd_end, false, false)));
    };
    canceled();
    // The first error status of Dir, FD or SD.
    let status = match FIRST_ERROR.load(Ordering::SeqCst) {
        0 if fd_status != b'T' => fd_status,
        0 if sd_end.status == 0 => b'E',
        0 => sd_end.status,
        first => first,
    };
    let errors = JOB_ERRORS.load(Ordering::SeqCst) + errors + before_errors;
    finish(job_id, session, status, files as i64, bytes as i64, errors as i32, vol,
        Some((fd_status, read_bytes, &sd_end, vss, encrypt)))
}

/// backup.cc SendAccurateCurrentFiles(): the Director writes the list
/// (AccurateGetJobids, GetFileList, "Sending Accurate information ..."), the
/// worker sends "accurate files=N", the entries and EOD.
fn send_accurate_files(fd: &mut BnetStream<AnyStream>, pipes: &mut WorkerPipes, job_id: u32, session: &BackupSession,
                       start_time: i64) -> Result<(), String> {
    if !session.accurate || !matches!(session.ids.level, 'I' | 'D') {
        return Ok(());
    }
    let timer = std::time::Instant::now();
    pipes.send(&PipeMessage::AccurateRequest { job_id, start_time });
    let (path, estimate) = loop {
        match pipes.recv() {
            Some(PipeMessage::AccurateReady { error: Some(error), .. }) => return Err(error),
            Some(PipeMessage::AccurateReady { path, estimate, .. }) => break (path, estimate),
            Some(_) => continue,
            None => return Err("Cannot find previous jobids.\n".into()),
        }
    };
    let data = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    let data = data.map_err(|e| format!("Cannot read accurate list {path}: {e}\n"))?;
    send(fd, &format!("accurate files={estimate}\n"))?;
    let mut sent = 0u64;
    let mut at = 0;
    while at + 4 <= data.len() {
        let len = u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
        let Some(message) = data.get(at + 4..at + 4 + len) else { break };
        fd.send(message).map_err(|e| format!("Network error: {e}\n"))?;
        sent += 1;
        at += 4 + len;
    }
    let secs = timer.elapsed().as_secs();
    jmsg(job_id, M_INFO, format!("Sent Accurate information for {sent} files (skipping 0 deleted files) in {:02}:{:02}:{:02}.\n",
        secs / 3600, secs / 60 % 60, secs % 60));
    fd.send_signal(BnetSignal::Eod).map_err(|e| format!("Network error: {e}\n"))
}

/// EndJob: "2800 End Job TermCode=%d JobFiles=%u ReadBytes=%llu JobBytes=%llu Errors=%u VSS=%d Encrypt=%d"
fn parse_end_job(msg: &str) -> Option<(u8, u64, u64, u64, u32, bool, bool)> {
    let rest = msg.strip_prefix("2800 End Job ")?;
    let mut values = Vec::new();
    for (word, key) in rest.split_whitespace().zip(["TermCode=", "JobFiles=", "ReadBytes=", "JobBytes=", "Errors=", "VSS=", "Encrypt="]) {
        values.push(word.strip_prefix(key)?.parse::<u64>().ok()?);
    }
    (values.len() == 7).then(|| (values[0] as u8, values[1], values[2], values[3], values[4] as u32, values[5] != 0, values[6] != 0))
}

/// StartStorageDaemonJob(jcr, send_bsr=false): ((VolSessionId, VolSessionTime), Authorization).
fn start_sd_job(sd: &mut BnetStream<PskStream>, params: &JobParams, session: &BackupSession, quota: u64)
    -> Result<((u32, u32), String), String> {
    let job_id = params.job_id;
    // The Job command with Quota= of FetchRemainingQuotas().
    let command = crate::sd_copy::job_command(params, &session.sd_job).replacen(" Quota=0 ", &format!(" Quota={quota} "), 1);
    send(sd, &command)?;
    let reply = match dir_msg(sd, job_id) {
        Ok(Some(reply)) => reply,
        Ok(None) => return Err("<stored: bad response to Job command: No data available\n".into()),
        Err(e) => return Err(format!("<stored: bad response to Job command: {e}\n")),
    };
    let field = |key: &str| reply.split_whitespace().find_map(|w| w.strip_prefix(key)).map(str::to_string);
    let result = match (reply.starts_with("3000 OK Job SDid="), field("SDid="), field("SDtime="), field("Authorization=")) {
        (true, Some(id), Some(time), Some(key)) => ((id.parse().unwrap_or(0), time.parse().unwrap_or(0)), key),
        _ => return Err(format!("Storage daemon rejected Job command: {reply}")),
    };
    // SendSecureEraseReqToSd()
    send(sd, "getSecureEraseCmd\n")?;
    SD_ERASE_ASKED.store(true, Ordering::SeqCst);
    let _ = dir_msg(sd, job_id);
    Ok(result)
}

/// SendJobInfoToFileDaemon, SendLevelCommand, SendIncludeExcludeLists,
/// SendSecureEraseReqToFd (and the spot of SendRunscriptsCommands).
fn setup_fd(fd: &mut BnetStream<AnyStream>, params: &JobParams, session: &BackupSession, vol: (u32, u32),
            auth_key: &str, before: &job_worker::BeforeJob) -> Result<(), String> {
    let job_id = params.job_id;
    send(fd, &format!("JobId={job_id} Job={} SDid={} SDtime={} Authorization={auth_key} ssl=4\n",
        params.unique_job, vol.0, vol.1))?;
    match dir_msg(fd, job_id) {
        Ok(Some(msg)) if msg.starts_with("2000 OK Job") => {
            let uname = msg.get("2000 OK Job".len() + 1..).unwrap_or("");
            send_catalog(job_id, crate::backup::update_client_record_sql(&params.client_name, session.client_retention, uname));
        }
        Ok(Some(msg)) => return Err(format!("File daemon \"{}\" rejected Job command: {msg}", params.client_name)),
        Ok(None) => return Err("FD gave bad response to JobId command: No data available\n".into()),
        Err(e) => return Err(format!("FD gave bad response to JobId command: {e}\n")),
    }

    // SendLevelCommand()
    let accurate = if session.accurate { "accurate_" } else { "" };
    match session.ids.level {
        'B' => send(fd, "level = base  mtime_only=0 \n")?,
        'D' | 'I' => {
            let name = if session.ids.level == 'D' { "differential" } else { "incremental" };
            send(fd, &format!("level = {accurate}{name}  mtime_only=0 \n"))?;
            // SendSinceTime(): the FD polls the Director's time, then EOD.
            send(fd, &format!("level = since_utime {} mtime_only=0 prev_job={}\n", session.since_utime.max(0), session.prev_job))?;
            while let Some(msg) = dir_msg(fd, job_id).map_err(|e| format!("Network error: {e}\n"))? {
                jmsg(job_id, M_INFO, format!("{msg}\n"));
            }
        }
        _ => send(fd, "level = full  mtime_only=0 \n")?,
    }
    response(fd, job_id, "2000 OK level\n", "Level")?;

    // SendIncludeExcludeLists()
    if let Some(lines) = &session.fileset_lines {
        for line in lines {
            send(fd, line)?;
        }
        fd.send_signal(BnetSignal::Eod).map_err(|e| format!("Network error: {e}\n"))?;
        response(fd, job_id, "2000 OK include\n", "Include")?;
    }

    // SendPluginOptions()
    for line in &session.plugin_options {
        send(fd, line)?;
        if let Err(e) = response(fd, job_id, "2000 OK PluginOptions\n", "PluginOptions") {
            jmsg(job_id, M_FATAL, e);
            return Err("Plugin options failed.\n".into());
        }
    }

    // SendPreviousRestoreObjects() (Incremental/Differential).
    if matches!(session.ids.level, 'I' | 'D') && !session.restore_objects.is_empty() {
        for object in &session.restore_objects {
            fd.send(object).map_err(|e| format!("Network error: {e}\n"))?;
        }
        send(fd, "restoreobject end\n")?;
        if let Err(e) = response(fd, job_id, "2000 OK ObjectRestored\n", "RestoreObject") {
            jmsg(job_id, M_FATAL, e);
            return Err("RestoreObject failed.\n".into());
        }
    }

    // SendSecureEraseReqToFd()
    send(fd, "getSecureEraseCmd\n")?;
    FD_ERASE_ASKED.store(true, Ordering::SeqCst);
    if let Some(msg) = dir_msg(fd, job_id).map_err(|e| format!("Network error: {e}\n"))? {
        if !msg.starts_with("2000 OK FDSecureEraseCmd ") {
            jmsg(job_id, M_WARNING, format!("Unexpected Client Secure Erase Cmd: {msg}"));
        }
    }

    // SendBwlimitToFd(); a refused limit is only reported.
    if session.max_bandwidth > 0 {
        send(fd, &format!("setbandwidth={} Job={}\n", session.max_bandwidth, params.unique_job))?;
        if let Err(e) = response(fd, job_id, "2000 OK Bandwidth\n", "Bandwidth") {
            jmsg(job_id, M_FATAL, e);
        }
    }
    // SendRunscriptsCommands(): the client run scripts (S-07); the
    // failure message is emitted there.
    job_worker::send_client_runscripts(fd, params, before).map_err(|_| String::new())
}

/// msgchan.cc ReserveWriteDevice(): "Using Device \"%s\" to write."
fn reserve_write_device(sd: &mut BnetStream<PskStream>, job_id: u32, store: &crate::sd_copy::StorageUse) -> Result<(), String> {
    for line in crate::sd_copy::use_storage_commands(store, true) {
        send(sd, &line)?;
    }
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    let reply = dir_msg(sd, job_id).ok().flatten().unwrap_or_default();
    let device = reply.strip_prefix("3000 OK use device device=").and_then(|d| d.split_whitespace().next());
    let last = store.devices.last().cloned().unwrap_or_default();
    match device {
        Some(device) => {
            jmsg(job_id, M_INFO, crate::job_messages::using_device_text(&crate::job_messages::unbash_spaces(device), true));
            Ok(())
        }
        None if !reply.is_empty() => Err(format!("\n     Storage daemon didn't accept Device \"{last}\" because:\n     {reply}\n")),
        None => Err(format!("\n     Storage daemon didn't accept Device \"{last}\" command.\n")),
    }
}

/// msgchan.cc msg_thread(): catalog requests and job messages of the SD
/// until "3099 Job end" (None: the SD went away first).
fn message_thread(sd: &mut BnetStream<PskStream>, job_id: u32, mut from_director: std::fs::File) -> Option<SdEnd> {
    let result = loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let raw = String::from_utf8_lossy(&data).into_owned();
                match data.first() {
                    Some(b'U') => job_worker::send_jmsg(PipeMessage::UpdCat { job_id, data: hex::encode(&data) }),
                    Some(b'C' | b'B') => {
                        job_worker::send_jmsg(PipeMessage::CatReq { job_id, request: raw.clone() });
                        let response = read_response(&mut from_director);
                        if !response.is_empty() {
                            let _ = sd.send_str(&response);
                        }
                    }
                    _ if raw.starts_with("Jmsg") => forward(job_id, &raw),
                    _ if raw.starts_with("Status Job=") => {
                        if let Ok(status) = job_worker::storage_job_status(raw.trim()) {
                            job_worker::send_jmsg(PipeMessage::JobStatus { job_id, status: status as i32 });
                        }
                    }
                    _ if raw.starts_with("3099 Job") => {
                        let value = |key: &str| raw.split_whitespace().find_map(|w| w.strip_prefix(key))
                            .and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                        let status = value("JobStatus=") as u8;
                        job_worker::send_jmsg(PipeMessage::JobStatus { job_id, status: status as i32 });
                        break Some(SdEnd { status, files: value("JobFiles="), bytes: value("JobBytes="),
                            errors: value("JobErrors=") as u32 });
                    }
                    _ => debug!("Worker {job_id} SD: {}", raw.trim_end()),
                }
            }
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break None,
            Ok(BnetMessage::Signal(_)) => {}
            Err(e) => {
                // msgchan.cc msg_thread(): BNET_HARDEOF; IsBnetError: JS_ErrorTerminated.
                warn!("Worker {job_id} SD recv: {e}");
                jmsg(job_id, M_FATAL, "Director's comm line to SD dropped.\n".into());
                break Some(SdEnd { status: b'E', ..Default::default() });
            }
        }
    };
    let _ = sd.send_signal(BnetSignal::Terminate);
    result
}

/// The Director's answer to a catalog request (CatResp on the pipe).
fn read_response(from_director: &mut std::fs::File) -> String {
    let mut line = String::new();
    let mut reader = BufReader::new(from_director);
    match reader.read_line(&mut line) {
        Ok(n) if n > 0 => match serde_json::from_str::<PipeMessage>(&line) {
            Ok(PipeMessage::CatResp { response }) => response,
            _ => "1000 OK\n".into(),
        },
        _ => "1000 OK\n".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_job_follows_filed_endjob_format() {
        assert_eq!(parse_end_job("2800 End Job TermCode=84 JobFiles=7 ReadBytes=60016 JobBytes=60016 Errors=0 VSS=0 Encrypt=0\n"),
            Some((b'T', 7, 60016, 60016, 0, false, false)));
        assert_eq!(parse_end_job("2800 End Job TermCode=84"), None);
    }
}
