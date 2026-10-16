//! SD job protocol: setup, use device, bootstrap, run.

use bareos_bnet::{BnetStream, BnetSignal};
use bareos_bnet::tls_psk::PskStream;
use tracing::debug;

/// SD session info returned from job setup.
#[derive(Debug, Clone)]
pub struct SdSession {
    pub auth_key: String,
    pub vol_session_id: i32,
    pub vol_session_time: i32,
}

/// Job type for SD protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdJobType {
    Backup,
    Restore,
}

/// Parameters for SD job setup.
#[derive(Debug, Clone)]
pub struct SdJobParams {
    pub job_id: u32,
    pub unique_job: String,
    pub job_name: String,
    pub job_type: SdJobType,
    pub level: String,
    pub client_address: String,
    pub media_type: String,
    pub pool_name: String,
    pub device_name: String,
}

/// Send job setup command to SD, receive Authorization key + SDid/SDtime.
///
/// Protocol: `JobId=N job=UNIQUE type=T level=L ...`
/// Response: `3000 OK Job SDid=N SDtime=T Authorization=KEY`
pub fn sd_job_setup(sd: &mut BnetStream<PskStream>, p: &SdJobParams) -> Result<SdSession, String> {
    let (type_int, level_int, fileset_md5, backup_fmt) = match p.job_type {
        SdJobType::Backup => (66, match p.level.as_str() {
            "full" => 70, "incremental" => 73, "differential" => 68, _ => 70,
        }, "0", "NativeBackup"),
        SdJobType::Restore => (82, 32, "**Dummy**", "Native"),
    };

    let cmd = format!(
        "JobId={} job={} job_name={} client_name={} \
         type={type_int} level={level_int} FileSet=LinuxAll NoAttr=0 SpoolAttr=0 FileSetMD5={fileset_md5} \
         SpoolData=0 PreferMountedVols=1 SpoolSize=0 \
         rerunning=0 VolSessionId=0 VolSessionTime=0 Quota=0 \
         Protocol=0 BackupFormat={backup_fmt}\n",
        p.job_id, p.unique_job, p.job_name, p.client_address
    );
    sd.send_str(&cmd).map_err(|e| format!("send: {e}"))?;

    let resp = sd.recv_str().map_err(|e| format!("recv: {e}"))?;

    let auth_key = extract_field(&resp, "Authorization=")
        .ok_or_else(|| format!("no auth key: {resp}"))?;
    let vol_session_id = extract_field(&resp, "SDid=")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let vol_session_time = extract_field(&resp, "SDtime=")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // getSecureEraseCmd (SD expects it after job setup)
    sd.send_str("getSecureEraseCmd\n").map_err(|e| format!("send secErase: {e}"))?;
    let _ = sd.recv_str();

    Ok(SdSession { auth_key, vol_session_id, vol_session_time })
}

/// Send "use storage/device" commands to SD.
/// Returns the SD's answer ("3000 OK use device device=...") and the job
/// messages ("Jmsg Job=...") it sent before.
pub fn sd_use_device(sd: &mut BnetStream<PskStream>, p: &SdJobParams) -> Result<(String, Vec<String>), String> {
    let append = if p.job_type == SdJobType::Restore { 0 } else { 1 };
    let cmd = format!(
        "use storage=File media_type={} pool_name={} pool_type=Backup append={append} copy=0 stripe=0\n",
        p.media_type, p.pool_name
    );
    sd.send_str(&cmd).map_err(|e| format!("send: {e}"))?;
    sd.send_str(&format!("use device={}\n", p.device_name)).map_err(|e| format!("send: {e}"))?;
    sd.send_signal(BnetSignal::Eod).map_err(|e| format!("eod1: {e}"))?;
    sd.send_signal(BnetSignal::Eod).map_err(|e| format!("eod2: {e}"))?;

    let mut jmsgs = Vec::new();
    loop {
        match sd.recv_str() {
            Ok(r) if r.contains("3000 OK") => return Ok((r, jmsgs)),
            Ok(r) if r.starts_with("Jmsg") => { debug!("SD Jmsg: {}", r.trim()); jmsgs.push(r); continue; }
            Ok(r) => return Err(format!("unexpected: {r}")),
            Err(e) => return Err(format!("recv: {e}")),
        }
    }
}

/// Send bootstrap data to SD for restore.
pub fn sd_send_bootstrap(sd: &mut BnetStream<PskStream>, bootstrap: &str) -> Result<(), String> {
    if bootstrap.is_empty() {
        return Err("no bootstrap data".into());
    }
    sd.send_str("bootstrap\n").map_err(|e| format!("send bootstrap: {e}"))?;
    for line in bootstrap.lines() {
        sd.send_str(&format!("{line}\n")).map_err(|e| format!("bsr line: {e}"))?;
    }
    sd.send_signal(BnetSignal::Eod).map_err(|e| format!("bsr eod: {e}"))?;

    loop {
        match sd.recv_str() {
            Ok(r) if r.contains("3000 OK bootstrap") => return Ok(()),
            Ok(r) if r.starts_with("Jmsg") => { debug!("SD Jmsg: {}", r.trim()); continue; }
            Ok(r) => return Err(format!("bootstrap resp: {r}")),
            Err(e) => return Err(format!("bootstrap recv: {e}")),
        }
    }
}

/// Send "run" command to SD (starts WaitFD).
pub fn sd_run(sd: &mut BnetStream<PskStream>) -> Result<(), String> {
    sd.send_str("run\n").map_err(|e| format!("run: {e}"))
}

fn extract_field(resp: &str, prefix: &str) -> Option<String> {
    resp.split_whitespace()
        .find(|s| s.starts_with(prefix))
        .and_then(|s| s.strip_prefix(prefix))
        .map(|s| s.to_string())
}
