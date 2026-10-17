//! Storage daemon read+write session of the jobs that copy data from
//! volume to volume without a File daemon (Compatibility log C-058, S-05):
//! Copy/Migrate worker jobs (migrate.cc DoActualMigration, local replicate)
//! and VirtualFull backups (vbackup.cc). Runs in the forked job worker:
//! msgchan.cc StartStorageDaemonJob (Job command, bootstrap, secure erase),
//! ReserveReadAndWriteDevices, "run", then the Storage daemon message
//! thread (catalog requests over the worker pipe) until "3099 Job end".

/// One Storage resource as ReserveReadDevice/ReserveWriteDevice send it.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StorageUse {
    /// Storage resource name (`use storage=`).
    pub name: String,
    pub address: String,
    pub port: u16,
    pub password: String,
    pub media_type: String,
    /// Device names of the Storage resource (`use device=` each).
    pub devices: Vec<String>,
    /// Pool name and Pool Type (read: rpool, write: pool).
    pub pool: String,
    pub pool_type: String,
}

/// What the worker needs for the SD session (JobParams.copy).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CopySession {
    pub read: StorageUse,
    pub write: StorageUse,
    /// Bootstrap file content (SendBootstrapFileToSd).
    pub bootstrap: String,
    /// Job command `type=` (jcr->getJobType(): 'c', 'g', or 'B').
    pub job_type: char,
    /// Job command `level=` (jcr->getJobLevel()).
    pub level: char,
    /// Job command job_name= (Job resource), client_name=, FileSet=.
    pub job_resource: String,
    pub client: String,
    pub fileset: String,
    pub fileset_md5: String,
    /// !pool->catalog_files
    pub no_attributes: bool,
    pub spool_attributes: bool,
    pub spool_data: bool,
    pub prefer_mounted_volumes: bool,
    pub spool_size: i64,
    /// Job messages the worker sends first (M_* code, text).
    #[serde(default)]
    pub start_messages: Vec<(u32, String)>,
    /// End the job JS_FatalError after the start messages, without the SD.
    #[serde(default)]
    pub fail_before_sd: bool,
}

use std::io::{BufRead, BufReader};
use std::sync::atomic::Ordering;

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};
use bareos_bnet::tls_psk::PskStream;
use tracing::{debug, warn};

use crate::job_worker::{self, JobParams, PipeMessage, WorkerPipes, M_INFO};

const M_FATAL: u32 = 3;

/// bnet BashSpaces(): blanks travel as 0x01.
fn bash(text: &str) -> String {
    crate::catreq::bash_spaces(text)
}

/// The Job command (msgchan.cc jobcmd).
pub fn job_command(params: &JobParams, copy: &CopySession) -> String {
    let or_none = |v: &str| if v.is_empty() { "**None**".to_string() } else { bash(v) };
    format!("JobId={} job={} job_name={} client_name={} type={} level={} FileSet={} NoAttr={} SpoolAttr={} \
             FileSetMD5={} SpoolData={} PreferMountedVols={} SpoolSize={} rerunning=0 VolSessionId=0 \
             VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n",
        params.job_id, params.unique_job, bash(&copy.job_resource), or_none(&copy.client), copy.job_type as u32,
        copy.level as u32, or_none(&copy.fileset), copy.no_attributes as i32, copy.spool_attributes as i32,
        if copy.fileset_md5.is_empty() { "**Dummy**" } else { &copy.fileset_md5 }, copy.spool_data as i32,
        copy.prefer_mounted_volumes as i32, copy.spool_size)
}

/// The `use storage`/`use device` lines of one Storage (ReserveRead/WriteDevice).
pub fn use_storage_commands(store: &StorageUse, append: bool) -> Vec<String> {
    let mut lines = vec![format!("use storage={} media_type={} pool_name={} pool_type={} append={} copy=0 stripe=0\n",
        bash(&store.name), bash(&store.media_type), bash(&store.pool), bash(&store.pool_type), append as i32)];
    for device in &store.devices {
        lines.push(format!("use device={}\n", bash(device)));
    }
    lines
}

/// A reply of the SD; its job messages are forwarded (BgetDirmsg).
fn dir_msg(sd: &mut BnetStream<PskStream>, job_id: u32) -> Result<String, String> {
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let text = String::from_utf8_lossy(&data).into_owned();
                if text.starts_with("Jmsg") {
                    job_worker::forward_daemon_jmsg(job_id, &text);
                    continue;
                }
                return Ok(text);
            }
            Ok(BnetMessage::Signal(_)) => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn fatal(job_id: u32, text: String) {
    job_worker::worker_jmsg(job_id, M_FATAL, text);
}

/// The SD's job end ("3099 Job %s end JobStatus=%d JobFiles=%d JobBytes=%lld JobErrors=%u").
struct SdEnd {
    status: u8,
    files: i64,
    bytes: i64,
    errors: i32,
}

/// The worker part of the job: start messages, SD session, job end.
pub fn run(params: &JobParams, copy: &CopySession, pipes: &mut WorkerPipes) -> i32 {
    let job_id = params.job_id;
    let state = |pipes: &mut WorkerPipes, name: &str| pipes.send(&PipeMessage::State { job_id, state: name.into() });
    for (code, text) in &copy.start_messages {
        job_worker::worker_jmsg(job_id, *code, text.clone());
    }
    let end = |pipes: &mut WorkerPipes, status: u8, sd: (i32, i32), files: i64, bytes: i64, errors: i32| {
        pipes.send(&PipeMessage::JobEnd { job_id, term_code: status as i32, files, bytes, errors,
            vol_session_id: Some(sd.0), vol_session_time: Some(sd.1) });
        if matches!(status, b'T' | b'W') { job_worker::EXIT_SUCCESS } else { job_worker::EXIT_ERROR }
    };
    if copy.fail_before_sd {
        return end(pipes, b'f', (0, 0), 0, 0, 0);
    }
    // ConnectToStorageDaemon(): JS_WaitSD.
    state(pipes, "ConnectingSD");
    let mut sd = match job_worker::connect_sd(params) {
        Ok((sd, cipher)) => {
            job_worker::worker_jmsg(job_id, M_INFO,
                crate::job_messages::connected_text("Storage daemon", &params.sd_address, params.sd_port, Some(&cipher)));
            job_worker::worker_jmsg(job_id, M_INFO, crate::job_messages::cipher_text(Some(&cipher)));
            sd
        }
        Err(e) => {
            fatal(job_id, format!("Failed to connect to Storage daemon: {e}\n"));
            return end(pipes, b'f', (0, 0), 0, 0, 0);
        }
    };
    state(pipes, "SDJobSetup");
    let session = match start_job(&mut sd, params, copy) {
        Ok(session) => session,
        Err(e) => {
            fatal(job_id, e);
            let _ = sd.send_signal(BnetSignal::Terminate);
            return end(pipes, b'f', (0, 0), 0, 0, 0);
        }
    };
    for (store, append) in [(&copy.read, false), (&copy.write, true)] {
        if let Err(e) = reserve(&mut sd, job_id, store, append) {
            fatal(job_id, e);
            let _ = sd.send_signal(BnetSignal::Terminate);
            return end(pipes, b'f', session, 0, 0, 0);
        }
    }
    if sd.send_str("run").is_err() {
        return end(pipes, b'f', session, 0, 0, 0);
    }
    state(pipes, "Running");
    let result = message_thread(&mut sd, job_id, pipes);
    let _ = sd.send_signal(BnetSignal::Terminate);
    match result {
        Some(e) => end(pipes, e.status, session, e.files, e.bytes, e.errors),
        None => {
            let status = if job_worker::WORKER_CANCEL.load(Ordering::SeqCst) { b'A' } else { b'E' };
            end(pipes, status, session, 0, 0, 1)
        }
    }
}

/// StartStorageDaemonJob(jcr, send_bsr=true): (VolSessionId, VolSessionTime).
fn start_job(sd: &mut BnetStream<PskStream>, params: &JobParams, copy: &CopySession) -> Result<(i32, i32), String> {
    let job_id = params.job_id;
    sd.send_str(&job_command(params, copy)).map_err(|e| format!("<stored: bad response to Job command: {e}\n"))?;
    let reply = dir_msg(sd, job_id).map_err(|e| format!("<stored: bad response to Job command: {e}\n"))?;
    let field = |key: &str| reply.split_whitespace().find_map(|w| w.strip_prefix(key)).map(str::to_string);
    let session = match (reply.starts_with("3000 OK Job SDid="), field("SDid="), field("SDtime="), field("Authorization=")) {
        (true, Some(id), Some(time), Some(_)) => (id.parse().unwrap_or(0), time.parse::<u32>().unwrap_or(0) as i32),
        _ => return Err(format!("Storage daemon rejected Job command: {reply}")),
    };
    // SendBootstrapFileToSd + response(OKbootstrap).
    sd.send_str("bootstrap\n").map_err(|e| e.to_string())?;
    for line in copy.bootstrap.split_inclusive('\n') {
        sd.send_str(line).map_err(|e| e.to_string())?;
    }
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    let reply = dir_msg(sd, job_id).map_err(|e| format!("Bootstrap: {e}\n"))?;
    if reply != "3000 OK bootstrap\n" {
        return Err(format!("Bad response to Bootstrap command: wanted 3000 OK bootstrap\n, got {reply}\n"));
    }
    // SendSecureEraseReqToSd()
    sd.send_str("getSecureEraseCmd\n").map_err(|e| e.to_string())?;
    let _ = dir_msg(sd, job_id);
    Ok(session)
}

/// ReserveReadDevice / ReserveWriteDevice: "Using Device ... to read/write."
fn reserve(sd: &mut BnetStream<PskStream>, job_id: u32, store: &StorageUse, append: bool) -> Result<(), String> {
    for line in use_storage_commands(store, append) {
        sd.send_str(&line).map_err(|e| e.to_string())?;
    }
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
    let reply = dir_msg(sd, job_id).unwrap_or_default();
    let device = reply.strip_prefix("3000 OK use device device=").and_then(|d| d.split_whitespace().next());
    let last = store.devices.last().cloned().unwrap_or_default();
    match device {
        Some(device) => {
            job_worker::worker_jmsg(job_id, M_INFO, crate::job_messages::using_device_text(
                &crate::job_messages::unbash_spaces(device), append));
            Ok(())
        }
        None if !reply.is_empty() => Err(format!("\n     Storage daemon didn't accept Device \"{last}\" because:\n     {reply}\n")),
        None => Err(format!("\n     Storage daemon didn't accept Device \"{last}\" command.\n")),
    }
}

/// msgchan.cc msg_thread(): catalog requests and job messages until the SD
/// reports the job end.
fn message_thread(sd: &mut BnetStream<PskStream>, job_id: u32, pipes: &mut WorkerPipes) -> Option<SdEnd> {
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let raw = String::from_utf8_lossy(&data).into_owned();
                match data.first() {
                    Some(b'U') => pipes.send(&PipeMessage::UpdCat { job_id, data: hex::encode(&data) }),
                    Some(b'C' | b'B') => {
                        pipes.send(&PipeMessage::CatReq { job_id, request: raw.clone() });
                        let response = read_response(&mut pipes.from_director);
                        if !response.is_empty() {
                            let _ = sd.send_str(&response);
                        }
                    }
                    _ if raw.starts_with("Jmsg") => job_worker::forward_daemon_jmsg(job_id, &raw),
                    _ if raw.starts_with("Status Job=") => {
                        if let Ok(status) = job_worker::storage_job_status(raw.trim()) {
                            pipes.send(&PipeMessage::JobStatus { job_id, status: status as i32 });
                        }
                    }
                    _ if raw.starts_with("3099 Job") => {
                        let value = |key: &str| raw.split_whitespace().find_map(|w| w.strip_prefix(key))
                            .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                        let status = value("JobStatus=") as u8;
                        pipes.send(&PipeMessage::JobStatus { job_id, status: status as i32 });
                        return Some(SdEnd { status, files: value("JobFiles="), bytes: value("JobBytes="),
                            errors: value("JobErrors=") as i32 });
                    }
                    _ => debug!("Worker {job_id} SD: {}", raw.trim_end()),
                }
            }
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => return None,
            Ok(BnetMessage::Signal(_)) => {}
            Err(e) => {
                warn!("Worker {job_id} SD recv: {e}");
                return None;
            }
        }
    }
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
    fn job_and_use_commands_follow_msgchan() {
        let params = JobParams { job_id: 632, unique_job: "mig-copy.2026-10-17_06.53.05_05".into(),
            ..serde_json::from_value(serde_json::json!({
                "job_id": 0, "unique_job": "", "job_name": "", "job_type": "Copy", "level": "", "fd_address": "",
                "fd_port": 0, "fd_password": "", "sd_address": "", "sd_port": 0, "sd_password": "", "sd_device": "",
                "sd_media_type": "", "director_name": "", "pool_name": "", "fileset_includes": [],
                "fileset_excludes": [], "since_utime": 0, "prev_job": "", "where_path": null, "replace": null,
                "restore_jobid": null, "bootstrap": null})).unwrap() };
        let copy = CopySession { job_type: 'c', level: 'F', job_resource: "mig copy".into(), client: "mig-fd".into(),
            fileset: "MigSet".into(), prefer_mounted_volumes: true, ..Default::default() };
        assert_eq!(job_command(&params, &copy),
            "JobId=632 job=mig-copy.2026-10-17_06.53.05_05 job_name=mig\x01copy client_name=mig-fd type=99 level=70 \
             FileSet=MigSet NoAttr=0 SpoolAttr=0 FileSetMD5=**Dummy** SpoolData=0 PreferMountedVols=1 SpoolSize=0 \
             rerunning=0 VolSessionId=0 VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n");
        let store = StorageUse { name: "SrcStore".into(), media_type: "FileSrc".into(), devices: vec!["SrcDev".into()],
            pool: "MigSrc".into(), pool_type: "Backup".into(), ..Default::default() };
        assert_eq!(use_storage_commands(&store, false), vec![
            "use storage=SrcStore media_type=FileSrc pool_name=MigSrc pool_type=Backup append=0 copy=0 stripe=0\n".to_string(),
            "use device=SrcDev\n".to_string()]);
    }
}
