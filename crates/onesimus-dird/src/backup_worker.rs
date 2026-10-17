//! Daemon side of a native Backup job in the forked job worker (S-19,
//! Compatibility log C-058): backup.cc DoNativeBackup() in its order —
//! UpdateJobStartRecord, Storage daemon connection and Job command
//! (msgchan.cc StartStorageDaemonJob), File daemon connection,
//! SendJobInfoToFileDaemon (Client Uname), SendLevelCommand,
//! SendIncludeExcludeLists, SendSecureEraseReqToFd, the second
//! UpdateJobStartRecord, ReserveWriteDevice, ConfigureMessageThread
//! ("storage address=", SD "run", message thread), "backup FileIndex=0"
//! and WaitForJobTermination (FD "2800 End Job", SD "3099 Job end").

use std::io::{BufRead, BufReader};
use std::sync::atomic::Ordering;

use bareos_bnet::tls_psk::PskStream;
use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};
use tracing::{debug, warn};

use crate::backup::{BackupEnd, ClientRetention, JobIds};
use crate::job_worker::{self, JobParams, PipeMessage, WorkerPipes, M_INFO};

const M_FATAL: u32 = 3;
const M_WARNING: u32 = 5;

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
}

fn send_catalog(job_id: u32, sql: String) {
    job_worker::send_jmsg(PipeMessage::CatalogSql { job_id, sql });
}

fn jmsg(job_id: u32, msg_type: u32, text: String) {
    job_worker::worker_jmsg(job_id, msg_type, text);
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

/// getmsg.cc BgetDirmsg(): the next message that is not a job message;
/// signals are answered (BTIME, POLL, STATUS) or returned as None.
fn dir_msg(bs: &mut BnetStream<PskStream>, job_id: u32) -> Result<Option<String>, String> {
    loop {
        match bs.recv().map_err(|e| e.to_string())? {
            BnetMessage::Data(data) => {
                let text = String::from_utf8_lossy(&data).into_owned();
                if text.starts_with("Jmsg") {
                    job_worker::forward_daemon_jmsg(job_id, &text);
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
fn response(bs: &mut BnetStream<PskStream>, job_id: u32, expected: &str, cmd: &str) -> Result<(), String> {
    match dir_msg(bs, job_id) {
        Ok(Some(msg)) if msg == expected => Ok(()),
        Ok(Some(msg)) => Err(format!("Bad response to {cmd} command: wanted {expected}, got {msg}\n")),
        Ok(None) => Err(format!("Socket error on {cmd} command: ERR=No data available\n")),
        Err(e) => Err(format!("Socket error on {cmd} command: ERR={e}\n")),
    }
}

fn send(bs: &mut BnetStream<PskStream>, text: &str) -> Result<(), String> {
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
        None => ('E', 0, None, false, false),
    };
    job_worker::send_jmsg(PipeMessage::BackupEnd { job_id, end: BackupEnd {
        ids: session.ids, fd_status, read_bytes,
        sd_status: sd.map(|s| s.status as char).unwrap_or('\0'),
        sd_files: sd.map(|s| s.files).unwrap_or(0), sd_bytes: sd.map(|s| s.bytes).unwrap_or(0),
        sd_errors: sd.map(|s| s.errors).unwrap_or(0), vss, encrypt,
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
    let fail = |text: String, vol: (u32, u32)| {
        if !text.is_empty() {
            jmsg(job_id, M_FATAL, text);
        }
        finish(job_id, session, b'f', 0, 0, 1, vol, None)
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

    // ConnectToStorageDaemon() (JS_WaitSD) and StartStorageDaemonJob().
    state(job_id, "ConnectingSD");
    let mut sd = match job_worker::connect_sd(params) {
        Ok((sd, cipher)) => {
            jmsg(job_id, M_INFO, crate::job_messages::connected_text("Storage daemon", &params.sd_address, params.sd_port, Some(&cipher)));
            jmsg(job_id, M_INFO, crate::job_messages::cipher_text(Some(&cipher)));
            sd
        }
        Err(e) => return fail(format!("Failed to connect to Storage daemon: {e}\n"), (0, 0)),
    };
    state(job_id, "SDJobSetup");
    let (vol, auth_key) = match start_sd_job(&mut sd, params, session) {
        Ok(v) => v,
        Err(e) => {
            let _ = sd.send_signal(BnetSignal::Terminate);
            return fail(e, (0, 0));
        }
    };

    // ConnectToFileDaemon() (JS_WaitFD).
    state(job_id, "ConnectingFD");
    if params.client_first_connection {
        jmsg(job_id, M_INFO, crate::job_messages::PROBING_TEXT.to_string());
    }
    let mut fd = match job_worker::connect_fd(params) {
        Ok((fd, cipher)) => {
            let who = format!("Client: {}", params.client_name);
            jmsg(job_id, M_INFO, crate::job_messages::connected_text(&who, &params.fd_address, params.fd_port, Some(&cipher)));
            jmsg(job_id, M_INFO, crate::job_messages::handshake_text(params.client_first_connection));
            jmsg(job_id, M_INFO, crate::job_messages::cipher_text(Some(&cipher)));
            fd
        }
        Err(e) => {
            let _ = sd.send_signal(BnetSignal::Terminate);
            return fail(format!("Failed to connect to File daemon: {e}\n"), vol);
        }
    };
    state(job_id, "FDJobSetup");
    if let Err(e) = setup_fd(&mut fd, params, session, vol, &auth_key, &before) {
        let _ = fd.send_signal(BnetSignal::Terminate);
        let _ = sd.send_signal(BnetSignal::Terminate);
        return fail(e, vol);
    }

    // The job start record again, after the client run scripts.
    send_catalog(job_id, crate::backup::update_job_start_record_sql(job_id, 'R', &ids, now(), vol.0, vol.1));

    // ReserveWriteDevice().
    if let Err(e) = reserve_write_device(&mut sd, job_id, &session.write) {
        let _ = fd.send_signal(BnetSignal::Terminate);
        let _ = sd.send_signal(BnetSignal::Terminate);
        return fail(e, vol);
    }

    // ConfigureMessageThread(): the FD's storage address, SD "run", message thread.
    let storage_cmd = format!("storage address={} port={} ssl=4\n", session.write.address, session.write.port);
    if let Err(e) = send(&mut fd, &storage_cmd).and_then(|_| response(&mut fd, job_id, "2000 OK storage\n", "Storage")) {
        let _ = fd.send_signal(BnetSignal::Terminate);
        let _ = sd.send_signal(BnetSignal::Terminate);
        return fail(e, vol);
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

    // WaitForJobTermination()
    let mut fd_end: Option<(u8, u64, u64, u64, u32, bool, bool)> = None;
    loop {
        match dir_msg(&mut fd, job_id) {
            Ok(Some(msg)) => {
                match fd_end.is_none().then(|| parse_end_job(&msg)).flatten() {
                    Some(end) => fd_end = Some(end),
                    None => jmsg(job_id, M_WARNING, format!("Unexpected Client Job message: {msg}")),
                }
            }
            Ok(None) => break,
            Err(e) => {
                jmsg(job_id, M_FATAL, format!("Network error with FD during Backup: ERR={e}\n"));
                break;
            }
        }
        if job_worker::WORKER_CANCEL.load(Ordering::SeqCst) {
            break;
        }
    }
    let _ = fd.send_signal(BnetSignal::Terminate);
    let canceled = job_worker::WORKER_CANCEL.load(Ordering::SeqCst);
    state(job_id, "Finishing");
    let sd_end = sd_thread.join().ok().flatten().unwrap_or_default();

    let Some((fd_status, files, read_bytes, bytes, errors, vss, encrypt)) = fd_end else {
        jmsg(job_id, M_FATAL, "No Job status returned from FD.\n".into());
        let term = if canceled { b'A' } else { b'E' };
        return finish(job_id, session, term, 0, 0, 1, vol, Some((b'E', 0, &sd_end, false, false)));
    };
    // The first error status of Dir, FD or SD.
    let term = if canceled { b'A' } else if fd_status != b'T' { fd_status } else if sd_end.status == 0 { b'E' } else { sd_end.status };
    finish(job_id, session, term, files as i64, bytes as i64, errors as i32 + before.errors as i32, vol,
        Some((fd_status, read_bytes, &sd_end, vss, encrypt)))
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
fn start_sd_job(sd: &mut BnetStream<PskStream>, params: &JobParams, session: &BackupSession) -> Result<((u32, u32), String), String> {
    let job_id = params.job_id;
    send(sd, &crate::sd_copy::job_command(params, &session.sd_job))?;
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
    let _ = dir_msg(sd, job_id);
    Ok(result)
}

/// SendJobInfoToFileDaemon, SendLevelCommand, SendIncludeExcludeLists,
/// SendSecureEraseReqToFd (and the spot of SendRunscriptsCommands).
fn setup_fd(fd: &mut BnetStream<PskStream>, params: &JobParams, session: &BackupSession, vol: (u32, u32),
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

    // SendSecureEraseReqToFd()
    send(fd, "getSecureEraseCmd\n")?;
    if let Some(msg) = dir_msg(fd, job_id).map_err(|e| format!("Network error: {e}\n"))? {
        if !msg.starts_with("2000 OK FDSecureEraseCmd ") {
            jmsg(job_id, M_WARNING, format!("Unexpected Client Secure Erase Cmd: {msg}"));
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
                    _ if raw.starts_with("Jmsg") => job_worker::forward_daemon_jmsg(job_id, &raw),
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
                warn!("Worker {job_id} SD recv: {e}");
                break None;
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
