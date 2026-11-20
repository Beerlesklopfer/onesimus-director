//! SD job protocol: setup, use device, bootstrap, run.

use bareos_bnet::{BnetStream, BnetSignal};
use bareos_bnet::AnyStream;
use tracing::debug;

/// SD session info returned from job setup.
#[derive(Debug, Clone)]
pub struct SdSession {
    /// `Authorization=`: the key the FD presents to the SD for this job.
    pub auth_key: String,
    /// `SDid=`: VolSessionId the SD writes the job's records under.
    pub vol_session_id: i32,
    /// `SDtime=`: VolSessionTime (SD start time) of that session.
    pub vol_session_time: i32,
}

/// Job type for SD protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdJobType {
    /// `type=B`, level from [`SdJobParams::level`].
    Backup,
    /// `type=R`, level blank.
    Restore,
}

/// Parameters for SD job setup.
#[derive(Debug, Clone)]
pub struct SdJobParams {
    /// `JobId=`: the job's catalog id.
    pub job_id: u32,
    /// `job=`: the unique job name (`<Job>.<date>_<time>_<n>`).
    pub unique_job: String,
    /// `job_name=`: name of the Job resource.
    pub job_name: String,
    /// Backup or restore.
    pub job_type: SdJobType,
    /// Backup level (`full`, `incremental`, `differential`); ignored for restores.
    pub level: String,
    /// Name of the Client resource (`client_name=`), not its address (D11).
    pub client_name: String,
    /// Name of the FileSet (`FileSet=`); empty sends `**None**` as Bareos does
    /// without a FileSet.
    pub fileset: String,
    /// FileSet digest (`FileSetMD5=`); empty sends `**Dummy**`.
    pub fileset_md5: String,
    /// `media_type=` of `use storage`.
    pub media_type: String,
    /// `pool_name=` of `use storage`.
    pub pool_name: String,
    /// `use device=`: the device the job asks for.
    pub device_name: String,
}

/// The Job command of `p` (msgchan.cc jobcmd), formatted by
/// `bareos_format::sd_job::JobCommand` like every other Job command.
///
/// Before 19.11.2026 this was a third formatter with defect D11: the client
/// address as `client_name`, a fixed `FileSet=LinuxAll`, no BashSpaces,
/// `FileSetMD5=0` and `BackupFormat=NativeBackup` for backups.
pub fn job_command(p: &SdJobParams) -> String {
    let (job_type, level) = match p.job_type {
        SdJobType::Backup => ('B', match p.level.as_str() {
            "incremental" => 'I',
            "differential" => 'D',
            _ => 'F',
        }),
        SdJobType::Restore => ('R', ' '),
    };
    bareos_format::sd_job::JobCommand {
        job_id: p.job_id,
        unique_job: &p.unique_job,
        job_name: &p.job_name,
        client_name: &p.client_name,
        job_type,
        level,
        fileset: &p.fileset,
        fileset_md5: &p.fileset_md5,
        no_attributes: false,
        spool_attributes: false,
        spool_data: false,
        prefer_mounted_volumes: true,
        spool_size: 0,
    }
    .format()
}

/// Send job setup command to SD, receive Authorization key + SDid/SDtime.
///
/// Protocol: `JobId=N job=UNIQUE type=T level=L ...` ([`job_command`])
/// Response: `3000 OK Job SDid=N SDtime=T Authorization=KEY`
pub fn sd_job_setup(sd: &mut BnetStream<AnyStream>, p: &SdJobParams) -> Result<SdSession, String> {
    let cmd = job_command(p);
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

/// The `use storage` and `use device` lines of `p` (msgchan.cc
/// `use_storage`, `use_device`); pool, media type and device go through
/// BashSpaces as in `ReserveReadDevice`/`ReserveWriteDevice`. The storage
/// name is `File`: the parameters carry none (open point, P1).
pub fn use_device_commands(p: &SdJobParams) -> [String; 2] {
    let bash = bareos_format::bash::bash_spaces;
    let append = if p.job_type == SdJobType::Restore { 0 } else { 1 };
    [
        format!(
            "use storage=File media_type={} pool_name={} pool_type=Backup append={append} copy=0 stripe=0\n",
            bash(&p.media_type),
            bash(&p.pool_name)
        ),
        format!("use device={}\n", bash(&p.device_name)),
    ]
}

/// Send "use storage/device" commands to SD.
/// Returns the SD's answer ("3000 OK use device device=...") and the job
/// messages ("Jmsg Job=...") it sent before.
pub fn sd_use_device(sd: &mut BnetStream<AnyStream>, p: &SdJobParams) -> Result<(String, Vec<String>), String> {
    for cmd in use_device_commands(p) {
        sd.send_str(&cmd).map_err(|e| format!("send: {e}"))?;
    }
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
pub fn sd_send_bootstrap(sd: &mut BnetStream<AnyStream>, bootstrap: &str) -> Result<(), String> {
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
pub fn sd_run(sd: &mut BnetStream<AnyStream>) -> Result<(), String> {
    sd.send_str("run\n").map_err(|e| format!("run: {e}"))
}

fn extract_field(resp: &str, prefix: &str) -> Option<String> {
    resp.split_whitespace()
        .find(|s| s.starts_with(prefix))
        .and_then(|s| s.strip_prefix(prefix))
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restore parameters as odir's job_worker builds them.
    fn restore_params() -> SdJobParams {
        SdJobParams {
            job_id: 3,
            unique_job: "VRestore.2026-11-19_05.06.30_08".into(),
            job_name: "V Restore".into(),
            job_type: SdJobType::Restore,
            level: String::new(),
            client_name: "my fd".into(),
            fileset: String::new(),
            fileset_md5: String::new(),
            media_type: "VFile".into(),
            pool_name: "VPool".into(),
            device_name: "VDev".into(),
        }
    }

    #[test]
    fn d11_client_name_not_address_no_linuxall_bashed() {
        let cmd = job_command(&restore_params());
        assert_eq!(
            cmd,
            "JobId=3 job=VRestore.2026-11-19_05.06.30_08 job_name=V\u{1}Restore client_name=my\u{1}fd type=82 \
             level=32 FileSet=**None** NoAttr=0 SpoolAttr=0 FileSetMD5=**Dummy** SpoolData=0 PreferMountedVols=1 \
             SpoolSize=0 rerunning=0 VolSessionId=0 VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n"
        );
        assert!(!cmd.contains("LinuxAll") && !cmd.contains("127.0.0.1"));
    }

    #[test]
    fn d11_restore_with_the_real_fileset() {
        let mut p = restore_params();
        p.fileset = "Linux All".into();
        let cmd = job_command(&p);
        assert!(cmd.contains(" client_name=my\u{1}fd type=82 level=32 FileSet=Linux\u{1}All "), "{cmd}");
    }

    #[test]
    fn use_device_lines_are_bashed_like_msgchan() {
        let mut p = restore_params();
        p.media_type = "V File".into();
        p.pool_name = "V Pool".into();
        p.device_name = "V Dev".into();
        assert_eq!(use_device_commands(&p), [
            "use storage=File media_type=V\u{1}File pool_name=V\u{1}Pool pool_type=Backup append=0 copy=0 stripe=0\n"
                .to_string(),
            "use device=V\u{1}Dev\n".to_string(),
        ]);
        p.job_type = SdJobType::Backup;
        assert!(use_device_commands(&p)[0].contains(" append=1 "));
    }

    #[test]
    fn backup_levels_and_named_fileset() {
        let mut p = restore_params();
        p.job_type = SdJobType::Backup;
        p.fileset = "Linux All".into();
        p.fileset_md5 = "abc".into();
        for (level, code) in [("full", "70"), ("incremental", "73"), ("differential", "68"), ("", "70")] {
            p.level = level.into();
            let cmd = job_command(&p);
            assert!(cmd.contains(&format!(" type=66 level={code} FileSet=Linux\u{1}All ")), "{cmd}");
            assert!(cmd.contains(" FileSetMD5=abc ") && cmd.ends_with("BackupFormat=Native\n"), "{cmd}");
        }
    }
}
