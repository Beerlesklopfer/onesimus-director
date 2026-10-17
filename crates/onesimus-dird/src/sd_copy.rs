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
    /// The writing Storage daemon of an SD-SD Copy/Migrate (remote
    /// replicate): mig_jcr's Job command there, `listen`, and `replicate`
    /// to the reading SD. None: one SD reads and writes. S-20.
    #[serde(default)]
    pub remote: Option<RemoteWrite>,
}

/// mig_jcr of a remote replicate (StartStorageDaemonJob(mig_jcr, false)).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RemoteWrite {
    pub job_id: u32,
    pub unique_job: String,
    pub job_resource: String,
    pub client: String,
    pub fileset: String,
    pub fileset_md5: String,
    pub job_type: char,
    pub level: char,
    /// TlsPolicy for the SD-SD connection (`replicate ... ssl=`).
    pub tls_need: i32,
}

use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;
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

/// The fields of one Job command (msgchan.cc jobcmd).
struct JobCmd<'a> {
    job_id: u32,
    unique_job: &'a str,
    job_resource: &'a str,
    client: &'a str,
    job_type: char,
    level: char,
    fileset: &'a str,
    fileset_md5: &'a str,
    no_attributes: bool,
}

fn format_job_command(cmd: &JobCmd, copy: &CopySession) -> String {
    let or_none = |v: &str| if v.is_empty() { "**None**".to_string() } else { bash(v) };
    format!("JobId={} job={} job_name={} client_name={} type={} level={} FileSet={} NoAttr={} SpoolAttr={} \
             FileSetMD5={} SpoolData={} PreferMountedVols={} SpoolSize={} rerunning=0 VolSessionId=0 \
             VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n",
        cmd.job_id, cmd.unique_job, bash(cmd.job_resource), or_none(cmd.client), cmd.job_type as u32,
        cmd.level as u32, or_none(cmd.fileset), cmd.no_attributes as i32, copy.spool_attributes as i32,
        if cmd.fileset_md5.is_empty() { "**Dummy**" } else { cmd.fileset_md5 }, copy.spool_data as i32,
        copy.prefer_mounted_volumes as i32, copy.spool_size)
}

/// The Job command (msgchan.cc jobcmd).
pub fn job_command(params: &JobParams, copy: &CopySession) -> String {
    format_job_command(&JobCmd {
        job_id: params.job_id, unique_job: &params.unique_job, job_resource: &copy.job_resource, client: &copy.client,
        job_type: copy.job_type, level: copy.level, fileset: &copy.fileset, fileset_md5: &copy.fileset_md5,
        no_attributes: copy.no_attributes,
    }, copy)
}

/// The Job command of mig_jcr on the writing SD of a remote replicate.
pub fn remote_job_command(remote: &RemoteWrite, copy: &CopySession) -> String {
    format_job_command(&JobCmd {
        job_id: remote.job_id, unique_job: &remote.unique_job, job_resource: &remote.job_resource,
        client: &remote.client, job_type: remote.job_type, level: remote.level, fileset: &remote.fileset,
        fileset_md5: &remote.fileset_md5, no_attributes: copy.no_attributes,
    }, copy)
}

/// migrate.cc replicatecmd.
pub fn replicate_command(remote: &RemoteWrite, write: &StorageUse, auth_key: &str) -> String {
    format!("replicate JobId={} Job={} address={} port={} ssl={} Authorization={auth_key}\n",
        remote.job_id, remote.unique_job, write.address, write.port, remote.tls_need)
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

/// A reply of the SD; its job messages are forwarded for JCR `job_id` (BgetDirmsg).
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

/// The worker's pipe to the Director, shared by the message threads of
/// both Storage daemons of a remote replicate.
struct Pipe<'a>(Mutex<&'a mut WorkerPipes>);

impl<'a> Pipe<'a> {
    fn lock(&self) -> std::sync::MutexGuard<'_, &'a mut WorkerPipes> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One pipe message, written as one line in a single write under the
    /// worker's pipe lock (job_worker::send_jmsg, also used by worker_jmsg).
    fn send(&self, msg: &PipeMessage) {
        let _guard = self.lock();
        crate::job_worker::send_jmsg(msg.clone());
    }

    /// A catalog request and the Director's answer.
    fn catreq(&self, msg: &PipeMessage) -> String {
        let mut pipes = self.lock();
        crate::job_worker::send_jmsg(msg.clone());
        read_response(&mut pipes.from_director)
    }
}

/// ConnectToStorageDaemon() to one Storage: its "Connected Storage daemon"
/// and cipher messages go to JCR `msg_job`.
fn connect(params: &JobParams, store: &StorageUse, msg_job: u32) -> Result<BnetStream<PskStream>, String> {
    let mut target = params.clone();
    target.sd_address = store.address.clone();
    target.sd_port = store.port;
    target.sd_password = store.password.clone();
    target.copy = None;
    let (sd, cipher) = job_worker::connect_sd(&target)?;
    job_worker::worker_jmsg(msg_job, M_INFO,
        crate::job_messages::connected_text("Storage daemon", &store.address, store.port, Some(&cipher)));
    job_worker::worker_jmsg(msg_job, M_INFO, crate::job_messages::cipher_text(Some(&cipher)));
    Ok(sd)
}

/// The worker part of the job: start messages, SD session, job end.
pub fn run(params: &JobParams, copy: &CopySession, pipes: &mut WorkerPipes) -> i32 {
    let job_id = params.job_id;
    // job_thread(): "Version:", RunScripts "BeforeJob", then the job (S-07).
    let before = job_worker::start_messages_with_before(pipes, job_id, &copy.start_messages);
    if crate::job_manager::is_job_canceled(before.status) {
        pipes.send(&PipeMessage::JobEnd { job_id, term_code: before.status as i32, files: 0, bytes: 0,
            errors: before.errors as i32, vol_session_id: Some(0), vol_session_time: Some(0) });
        return job_worker::EXIT_ERROR;
    }
    let pipe = Pipe(Mutex::new(pipes));
    let state = |name: &str| pipe.send(&PipeMessage::State { job_id, state: name.into() });
    let end = |status: u8, sd: (i32, i32), files: i64, bytes: i64, errors: i32| {
        pipe.send(&PipeMessage::JobEnd { job_id, term_code: status as i32, files, bytes, errors,
            vol_session_id: Some(sd.0), vol_session_time: Some(sd.1) });
        if matches!(status, b'T' | b'W') { job_worker::EXIT_SUCCESS } else { job_worker::EXIT_ERROR }
    };
    let failed = || if job_worker::WORKER_CANCEL.load(Ordering::SeqCst) { b'A' } else { b'E' };
    if copy.fail_before_sd {
        return end(b'f', (0, 0), 0, 0, 0);
    }
    // ConnectToStorageDaemon(): JS_WaitSD. A remote replicate connects the
    // jcr to the reading and mig_jcr to the writing Storage daemon.
    state("ConnectingSD");
    let first = if copy.remote.is_some() { &copy.read } else { &copy.write };
    let mut sd = match connect(params, first, job_id) {
        Ok(sd) => sd,
        Err(e) => {
            fatal(job_id, format!("Failed to connect to Storage daemon: {e}\n"));
            return end(b'f', (0, 0), 0, 0, 0);
        }
    };
    let mut write_sd = match &copy.remote {
        None => None,
        Some(remote) => match connect(params, &copy.write, remote.job_id) {
            Ok(sd) => Some(sd),
            Err(e) => {
                fatal(job_id, format!("Failed to connect to Storage daemon: {e}\n"));
                let _ = sd.send_signal(BnetSignal::Terminate);
                return end(b'f', (0, 0), 0, 0, 0);
            }
        },
    };
    state("SDJobSetup");
    let (session, _) = match start_job(&mut sd, job_id, &job_command(params, copy), Some(&copy.bootstrap)) {
        Ok(session) => session,
        Err(e) => {
            fatal(job_id, e);
            let _ = sd.send_signal(BnetSignal::Terminate);
            return end(b'f', (0, 0), 0, 0, 0);
        }
    };
    let reservations: &[(&StorageUse, bool)] = if copy.remote.is_some() {
        &[(&copy.read, false)]
    } else {
        &[(&copy.read, false), (&copy.write, true)]
    };
    for (store, append) in reservations {
        if let Err(e) = reserve(&mut sd, job_id, store, *append) {
            fatal(job_id, e);
            let _ = sd.send_signal(BnetSignal::Terminate);
            return end(b'f', session, 0, 0, 0);
        }
    }
    let Some(remote) = copy.remote.as_ref() else {
        // Local replicate: one SD reads and writes.
        if sd.send_str("run").is_err() {
            return end(b'f', session, 0, 0, 0);
        }
        state("Running");
        let result = message_thread(&mut sd, job_id, job_id, &pipe, true);
        let _ = sd.send_signal(BnetSignal::Terminate);
        return match result {
            Some(e) => end(e.status, session, e.files, e.bytes, e.errors),
            None => end(failed(), session, 0, 0, 1),
        };
    };

    // Remote replicate: mig_jcr's job on the writing SD, `listen` there,
    // then `replicate` to the reading SD and `run`.
    let mut wsd = write_sd.take().expect("connected");
    let mig = remote.job_id;
    let ((w_id, w_time), auth_key) = match start_job(&mut wsd, mig, &remote_job_command(remote, copy), None) {
        Ok(started) => started,
        Err(e) => {
            fatal(job_id, e);
            let _ = sd.send_signal(BnetSignal::Terminate);
            let _ = wsd.send_signal(BnetSignal::Terminate);
            return end(b'f', session, 0, 0, 0);
        }
    };
    if let Err(e) = reserve(&mut wsd, mig, &copy.write, true) {
        fatal(job_id, e);
        let _ = sd.send_signal(BnetSignal::Terminate);
        let _ = wsd.send_signal(BnetSignal::Terminate);
        return end(b'f', session, 0, 0, 0);
    }
    if wsd.send_str("listen").is_err() {
        return end(b'f', session, 0, 0, 0);
    }
    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            let result = message_thread(&mut wsd, job_id, mig, &pipe, false);
            let _ = wsd.send_signal(BnetSignal::Terminate);
            result
        });
        let replicated = sd.send_str(&replicate_command(remote, &copy.write, &auth_key)).is_ok()
            && dir_msg(&mut sd, job_id).is_ok_and(|reply| reply == "3000 OK replicate\n");
        let result = if replicated && sd.send_str("run").is_ok() {
            state("Running");
            message_thread(&mut sd, job_id, job_id, &pipe, true)
        } else {
            None
        };
        let _ = sd.send_signal(BnetSignal::Terminate);
        // WaitForStorageDaemonTermination(mig_jcr)
        let written = writer.join().ok().flatten();
        if let Some(w) = &written {
            pipe.send(&PipeMessage::WriteSession { job_id, vol_session_id: w_id, vol_session_time: w_time,
                files: w.files, bytes: w.bytes, errors: w.errors, status: w.status as i32 });
        }
        match result {
            Some(e) => end(e.status, session, e.files, e.bytes, e.errors),
            None => end(failed(), session, 0, 0, 1),
        }
    })
}

/// StartStorageDaemonJob(jcr, send_bsr): (VolSessionId, VolSessionTime) and
/// the authorization key.
fn start_job(sd: &mut BnetStream<PskStream>, job_id: u32, command: &str, bootstrap: Option<&str>)
    -> Result<((i32, i32), String), String> {
    sd.send_str(command).map_err(|e| format!("<stored: bad response to Job command: {e}\n"))?;
    let reply = dir_msg(sd, job_id).map_err(|e| format!("<stored: bad response to Job command: {e}\n"))?;
    let field = |key: &str| reply.split_whitespace().find_map(|w| w.strip_prefix(key)).map(str::to_string);
    let session = match (reply.starts_with("3000 OK Job SDid="), field("SDid="), field("SDtime="), field("Authorization=")) {
        (true, Some(id), Some(time), Some(key)) => ((id.parse().unwrap_or(0), time.parse::<u32>().unwrap_or(0) as i32), key),
        _ => return Err(format!("Storage daemon rejected Job command: {reply}")),
    };
    if let Some(bootstrap) = bootstrap {
        // SendBootstrapFileToSd + response(OKbootstrap).
        sd.send_str("bootstrap\n").map_err(|e| e.to_string())?;
        for line in bootstrap.split_inclusive('\n') {
            sd.send_str(line).map_err(|e| e.to_string())?;
        }
        sd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
        let reply = dir_msg(sd, job_id).map_err(|e| format!("Bootstrap: {e}\n"))?;
        if reply != "3000 OK bootstrap\n" {
            return Err(format!("Bad response to Bootstrap command: wanted 3000 OK bootstrap\n, got {reply}\n"));
        }
    }
    // SendSecureEraseReqToSd()
    sd.send_str("getSecureEraseCmd\n").map_err(|e| e.to_string())?;
    let _ = dir_msg(sd, job_id);
    Ok(session)
}

/// ReserveReadDevice / ReserveWriteDevice: "Using Device ... to read/write."
/// for JCR `job_id`.
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

/// msgchan.cc msg_thread() of JCR `jcr` (the worker job or mig_jcr): catalog
/// requests and job messages until the SD reports the job end. Only the
/// worker job's own SD status reaches its JobStatus (`own_status`).
fn message_thread(sd: &mut BnetStream<PskStream>, job_id: u32, jcr: u32, pipe: &Pipe, own_status: bool) -> Option<SdEnd> {
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(data)) => {
                let raw = String::from_utf8_lossy(&data).into_owned();
                match data.first() {
                    Some(b'U') => pipe.send(&PipeMessage::UpdCat { job_id: jcr, data: hex::encode(&data) }),
                    Some(b'C' | b'B') => {
                        let response = pipe.catreq(&PipeMessage::CatReq { job_id: jcr, request: raw.clone() });
                        if !response.is_empty() {
                            let _ = sd.send_str(&response);
                        }
                    }
                    _ if raw.starts_with("Jmsg") => job_worker::forward_daemon_jmsg(jcr, &raw),
                    _ if raw.starts_with("Status Job=") => {
                        if let (true, Ok(status)) = (own_status, job_worker::storage_job_status(raw.trim())) {
                            pipe.send(&PipeMessage::JobStatus { job_id, status: status as i32 });
                        }
                    }
                    _ if raw.starts_with("3099 Job") => {
                        let value = |key: &str| raw.split_whitespace().find_map(|w| w.strip_prefix(key))
                            .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                        let status = value("JobStatus=") as u8;
                        if own_status {
                            pipe.send(&PipeMessage::JobStatus { job_id, status: status as i32 });
                        }
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

    #[test]
    fn remote_replicate_commands_follow_migrate_cc() {
        let copy = CopySession { prefer_mounted_volumes: true, ..Default::default() };
        let remote = RemoteWrite { job_id: 634, unique_job: "mig-backup.2026-10-17_07.21.14_05".into(),
            job_resource: "mig-backup".into(), client: "mig-fd".into(), fileset: "MigSet".into(),
            fileset_md5: "91+oUS+n23wyQ7+EpB50DA".into(), job_type: 'B', level: 'F', tls_need: 4 };
        // sd2.log of the original: the Job command of mig_jcr on the writing SD.
        assert_eq!(remote_job_command(&remote, &copy),
            "JobId=634 job=mig-backup.2026-10-17_07.21.14_05 job_name=mig-backup client_name=mig-fd type=66 level=70 \
             FileSet=MigSet NoAttr=0 SpoolAttr=0 FileSetMD5=91+oUS+n23wyQ7+EpB50DA SpoolData=0 PreferMountedVols=1 \
             SpoolSize=0 rerunning=0 VolSessionId=0 VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n");
        let write = StorageUse { address: "127.0.0.1".into(), port: 39196, ..Default::default() };
        assert_eq!(replicate_command(&remote, &write, "DOCI-AJPJ"),
            "replicate JobId=634 Job=mig-backup.2026-10-17_07.21.14_05 address=127.0.0.1 port=39196 ssl=4 Authorization=DOCI-AJPJ\n");
    }
}
