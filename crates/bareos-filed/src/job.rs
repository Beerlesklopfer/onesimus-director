//! Director command loop of the File Daemon: filed/dir_cmd.cc
//! process_director_commands() with the job commands of a restore
//! (JobId, storage, session, setauthorization, bootstrap, restoreobject,
//! pluginoptions, restore, endrestore, cancel) and the EndJob report.
//!
//! Backup, estimate and verify are not implemented by this File Daemon:
//! `backup`/`verify` end the job with a fatal error (see docs/STUBS.md).

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tracing::{debug, info};

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};

use crate::attribs::Where;
use crate::config::{DirectorAuth, FdConfig};
use crate::create_file::{CreateState, REPLACE_NEVER};
use crate::jmsg::*;
use crate::restore::{Counters, Restore, RestoreParams};
use crate::sd::{self, JobShared, SdStream};
use crate::{backup, status};

/// How long `restore` waits for a passive Storage daemon connection.
const PASSIVE_SD_WAIT: Duration = Duration::from_secs(60);

/// The jcr of one Director connection.
pub struct Session<'a> {
    config: &'a FdConfig,
    director: &'a DirectorAuth,
    session_id: u64,
    msgs: JobMsgs,
    counters: Counters,
    shared: Option<Arc<JobShared>>,
    sd_auth_key: String,
    sd_tls_policy: i32,
    multi_restore: bool,
    sd: Option<SdStream>,
    vol_session_id: u32,
    vol_session_time: u32,
    start_file: u32,
    end_file: u32,
    start_block: u32,
    end_block: u32,
    bootstrap: Vec<String>,
}

/// Command handler: false ends the command loop.
type Handler<'a, S> = fn(&mut Session<'a>, &mut BnetStream<S>, &str) -> bool;

/// A token "key=value" of a command line.
fn arg<'b>(cmd: &'b str, key: &str) -> Option<&'b str> {
    cmd.split_whitespace().find_map(|t| t.strip_prefix(key)?.strip_prefix('='))
}

/// UnbashSpaces(): 0x01 back to a space.
fn unbash(s: &str) -> String {
    s.replace('\x01', " ")
}

impl<'a> Session<'a> {
    pub fn new(config: &'a FdConfig, director: &'a DirectorAuth, session_id: u64) -> Self {
        Self {
            config,
            director,
            session_id,
            msgs: JobMsgs::new(&config.name),
            counters: Counters::default(),
            shared: None,
            sd_auth_key: String::new(),
            sd_tls_policy: 0,
            multi_restore: false,
            sd: None,
            vol_session_id: 0,
            vol_session_time: 0,
            start_file: 0,
            end_file: 0,
            start_block: 0,
            end_block: 0,
            bootstrap: Vec::new(),
        }
    }

    /// Send queued job messages, then the reply.
    fn reply<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, text: &str) -> bool {
        self.msgs.flush(dir);
        dir.send_str(text).is_ok()
    }

    fn is_canceled(&self) -> bool {
        self.msgs.is_canceled() || self.shared.as_ref().is_some_and(|s| s.canceled.load(Ordering::SeqCst))
    }

    /// The command table of dir_cmd.cc (prefix match in table order),
    /// `true` when monitor directors may use the command.
    fn select<S: Read + Write>(cmd: &str) -> Option<(Handler<'a, S>, bool)> {
        let table: [(&str, Handler<'a, S>, bool); 24] = [
            ("accurate", Self::accurate_cmd, false),
            ("backup", Self::backup_cmd, false),
            ("bootstrap", Self::bootstrap_cmd, false),
            ("cancel", Self::cancel_cmd, false),
            ("endrestore", Self::end_restore_cmd, false),
            ("estimate", Self::estimate_cmd, false),
            ("fileset", Self::fileset_cmd, false),
            ("JobId=", Self::job_cmd, false),
            ("level = ", Self::level_cmd, false),
            ("pluginoptions", Self::pluginoptions_cmd, false),
            ("RunBeforeNow", Self::run_before_now_cmd, false),
            ("Run", Self::runscript_cmd, false),
            ("restoreobject", Self::restore_object_cmd, false),
            ("restore ", Self::restore_cmd, false),
            ("getSecureEraseCmd", Self::secure_erase_cmd, false),
            ("session", Self::session_cmd, false),
            ("setauthorization", Self::setauthorization_cmd, false),
            ("setbandwidth=", Self::setbandwidth_cmd, false),
            ("setdebug", Self::setdebug_cmd, false),
            ("status", Self::status_cmd, true),
            (".status", Self::status_cmd, true),
            ("storage ", Self::storage_cmd, false),
            ("verify", Self::verify_cmd, false),
            ("quit", Self::quit_cmd, true),
        ];
        table.iter().find(|(name, _, _)| cmd.starts_with(name)).map(|(_, h, m)| (*h, *m))
    }

    /// process_director_commands()
    pub fn run<S: Read + Write>(mut self, dir: &mut BnetStream<S>) {
        loop {
            let cmd = match dir.recv() {
                Ok(BnetMessage::Data(d)) => String::from_utf8_lossy(&d).into_owned(),
                // dir->recv() < 0: signal or connection terminated
                Ok(BnetMessage::Signal(_)) | Err(_) => break,
            };
            debug!("session {}: <dird: {}", self.session_id, cmd.trim_end());
            let Some((handler, monitor_access)) = Self::select::<S>(&cmd) else {
                debug!("Command \"{}\" not found.", cmd.trim_end());
                self.reply(dir, "2999 Invalid command\n");
                break;
            };
            if !monitor_access && self.director.monitor {
                self.reply(dir, "2997 Invalid command for a Director with Monitor directive enabled.\n");
                let _ = dir.send_signal(BnetSignal::Eod);
                break;
            }
            let go_on = handler(&mut self, dir, &cmd);
            self.msgs.flush(dir);
            if !go_on {
                break;
            }
        }
        if let Some(mut sd) = self.sd.take() {
            let _ = sd.send_signal(BnetSignal::Terminate);
        }
        if self.msgs.job_id != 0 {
            if self.is_canceled() {
                self.msgs.set_status(JS_CANCELED);
            }
            let c = self.counters;
            let end = format!(
                "2800 End Job TermCode={} JobFiles={} ReadBytes={} JobBytes={} Errors={} VSS=0 Encrypt=0\n",
                self.msgs.status(), c.job_files, c.read_bytes, c.job_bytes, self.msgs.job_errors
            );
            info!("job {} ({}) ended: {}", self.msgs.job_id, self.msgs.job, end.trim_end());
            self.reply(dir, &end);
        }
        self.msgs.flush(dir);
        let _ = dir.send_signal(BnetSignal::Terminate);
        if let Some(shared) = self.shared.take() {
            sd::unregister(&shared.job);
        }
    }

    // --- commands --------------------------------------------------------

    /// job_cmd(): "JobId=%d Job=%127s SDid=%d SDtime=%d Authorization=%100s ssl=%d"
    fn job_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let job_id = arg(cmd, "JobId").and_then(|v| v.parse::<u32>().ok());
        let job = arg(cmd, "Job");
        let (Some(job_id), Some(job)) = (job_id, job) else {
            self.msgs.jmsg(M_FATAL, &format!("Bad Job Command: {cmd}"));
            self.reply(dir, "2901 Bad Job\n");
            return false;
        };
        self.msgs.job_id = job_id;
        self.msgs.job = job.to_string();
        self.vol_session_id = arg(cmd, "SDid").and_then(|v| v.parse().ok()).unwrap_or(0);
        self.vol_session_time = arg(cmd, "SDtime").and_then(|v| v.parse().ok()).unwrap_or(0);
        let tls = arg(cmd, "ssl").and_then(|v| v.parse().ok()).unwrap_or(0);
        if let Some(old) = self.shared.take() {
            sd::unregister(&old.job);
        }
        self.shared = Some(sd::register(job, ""));
        self.set_storage_auth_key(arg(cmd, "Authorization").unwrap_or(""), tls);
        info!("session {}: job {job_id} '{job}'", self.session_id);
        // The Director stores the text after "2000 OK Job" as the uname.
        self.reply(dir, &format!("2000 OK Job {} ({}) {},{}", crate::version::FULL, crate::version::SHORT_DATE,
            crate::version::os_info(), crate::version::platform()))
    }

    /// SetStorageAuthKeyAndTlsPolicy()
    fn set_storage_auth_key(&mut self, key: &str, policy: i32) {
        if key.is_empty() {
            return;
        }
        self.sd = None;
        if !self.sd_auth_key.is_empty() {
            // A second key: the Director does a multi storage restore.
            self.multi_restore = true;
        }
        self.sd_auth_key = key.to_string();
        self.sd_tls_policy = policy;
        if let Some(s) = &self.shared {
            *s.sd_auth_key.lock().unwrap_or_else(|e| e.into_inner()) = key.to_string();
        }
    }

    fn secure_erase_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.reply(dir, "2000 OK FDSecureEraseCmd *None*\n")
    }

    /// SetauthorizationCmd() (passive client)
    fn setauthorization_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let Some(key) = arg(cmd, "Authorization") else {
            self.reply(dir, "2902 Bad setauthorization\n");
            return false;
        };
        let policy = self.sd_tls_policy;
        self.set_storage_auth_key(key, policy);
        self.reply(dir, "2000 OK Authorization\n")
    }

    /// SetbandwidthCmd(): accepted; this File Daemon does not limit.
    fn setbandwidth_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.reply(dir, "2000 OK Bandwidth\n")
    }

    fn setdebug_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.reply(dir, "2000 OK setdebug\n");
        dir.send_signal(BnetSignal::Eod).is_ok()
    }

    fn status_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        let output = status::fd_status(self.config);
        self.reply(dir, &output);
        dir.send_signal(BnetSignal::Eod).is_ok()
    }

    fn quit_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.reply(dir, "2000 OK bye\n");
        false
    }

    /// CancelCmd(): "cancel Job=%127s"
    fn cancel_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        match cmd.trim_end().strip_prefix("cancel Job=").map(|j| j.split_whitespace().next().unwrap_or("")) {
            Some(job) => match sd::find(job) {
                Some(shared) => {
                    shared.cancel();
                    self.reply(dir, &format!("2001 Job {job} marked to be canceled.\n"));
                }
                None => {
                    self.reply(dir, &format!("2901 Job {job} not found.\n"));
                }
            },
            None => {
                self.reply(dir, "2902 Error scanning cancel command.\n");
            }
        }
        let _ = dir.send_signal(BnetSignal::Eod);
        true
    }

    /// StorageCmd(): "storage address=%s port=%d ssl=%d [Authorization=%100s]"
    fn storage_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let addr = arg(cmd, "address");
        let port = arg(cmd, "port").and_then(|p| p.parse::<u16>().ok());
        let tls = arg(cmd, "ssl").and_then(|p| p.parse::<i32>().ok());
        let (Some(addr), Some(port), Some(tls)) = (addr, port, tls) else {
            self.msgs.jmsg(M_FATAL, &format!("Bad storage command: {cmd}"));
            self.reply(dir, "2902 Bad storage\n");
            return false;
        };
        self.set_storage_auth_key(arg(cmd, "Authorization").unwrap_or(""), tls);
        let Some(shared) = self.shared.clone() else {
            self.msgs.jmsg(M_FATAL, "Improper calling sequence.\n");
            self.reply(dir, "2902 Bad storage\n");
            return false;
        };
        debug!("Open storage: {addr}:{port} ssl={tls}");
        let timeout = Duration::from_secs(self.config.sd_connect_timeout_secs);
        match sd::connect_storage(addr, port, tls, &self.msgs.job, &self.sd_auth_key, &self.config.name, timeout, &shared) {
            Ok(c) => {
                for i in &c.infos {
                    if self.msgs.job_id != 0 {
                        self.msgs.jmsg(M_INFO, i);
                    }
                }
                self.sd = Some(c.sd);
                self.reply(dir, "2000 OK storage\n")
            }
            Err(e) if e == "auth" => {
                self.msgs.jmsg(M_FATAL, "Failed to authenticate Storage daemon.\n");
                self.reply(dir, "2902 Bad storage\n");
                false
            }
            Err(e) => {
                debug!("Failed to connect to Storage daemon: {addr}:{port}: {e}");
                self.msgs.jmsg(M_FATAL, &format!("Failed to connect to Storage daemon: {addr}:{port}\n"));
                self.reply(dir, "2902 Bad storage\n");
                false
            }
        }
    }

    /// SessionCmd(): "session %127s %ld %ld %ld %ld %ld %ld"
    fn session_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let f: Vec<&str> = cmd.split_whitespace().collect();
        let n = |i: usize| f.get(i).and_then(|v| v.parse::<u32>().ok());
        match (n(2), n(3), n(4), n(5), n(6), n(7)) {
            (Some(a), Some(b), Some(c), Some(d), Some(e), Some(g)) => {
                (self.vol_session_id, self.vol_session_time) = (a, b);
                (self.start_file, self.end_file, self.start_block, self.end_block) = (c, d, e, g);
                self.reply(dir, "2000 OK session\n")
            }
            _ => {
                self.msgs.jmsg(M_FATAL, &format!("Bad session command: {cmd}"));
                false
            }
        }
    }

    /// BootstrapCmd(): kept for the SD read session (old protocol).
    fn bootstrap_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.bootstrap.clear();
        while let Ok(BnetMessage::Data(d)) = dir.recv() {
            self.bootstrap.push(String::from_utf8_lossy(&d).into_owned());
        }
        self.reply(dir, "2000 OK bootstrap\n")
    }

    /// RestoreObjectCmd(): objects are for plugins, none are loaded here.
    fn restore_object_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        if cmd == "restoreobject end\n" || cmd.trim_end() == "restoreobject end" {
            return self.reply(dir, "2000 OK ObjectRestored\n");
        }
        // Object name and object follow.
        let ok = matches!(dir.recv(), Ok(BnetMessage::Data(_))) && matches!(dir.recv(), Ok(BnetMessage::Data(_)));
        if !ok {
            self.reply(dir, "2909 Bad RestoreObject command.\n");
            return false;
        }
        true
    }

    /// PluginoptionsCmd(): no plugins to pass them to.
    fn pluginoptions_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.reply(dir, "2000 OK PluginOptions\n")
    }

    fn run_before_now_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.reply(dir, "2000 OK RunBeforeNow\n")
    }

    /// RunscriptCmd(): client run scripts are not run by this File Daemon.
    fn runscript_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let command = cmd.split_once("Command=").map(|(_, c)| unbash(c.trim_end())).unwrap_or_default();
        self.msgs.jmsg(M_WARNING, &format!("Client RunScript not supported by this File Daemon, not run: {command}\n"));
        self.reply(dir, "2000 OK RunScript\n")
    }

    fn end_restore_cmd<S: Read + Write>(&mut self, _dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        debug!("EndRestoreCmd");
        false
    }

    // Backup side (not implemented, see backup.rs)
    fn level_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.reply(dir, "2000 OK level\n")
    }
    fn fileset_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        while let Ok(BnetMessage::Data(_)) = dir.recv() {}
        self.reply(dir, "2000 OK include\n")
    }
    fn accurate_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        while let Ok(BnetMessage::Data(_)) = dir.recv() {}
        self.reply(dir, "2000 OK accurate\n")
    }
    fn estimate_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        backup::estimate(self, dir)
    }
    fn backup_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        backup::backup(self, dir)
    }
    fn verify_cmd<S: Read + Write>(&mut self, _dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.msgs.jmsg(M_FATAL, "Verify jobs are not supported by this File Daemon.\n");
        false
    }

    /// For backup.rs: reply and job messages.
    pub fn send_reply<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, text: &str) -> bool {
        self.reply(dir, text)
    }
    pub fn job_msg(&mut self, t: i32, text: &str) {
        self.msgs.jmsg(t, text);
    }

    /// RestoreCmd(): "restore replace=%c prelinks=%d where=%s" or
    /// "... regexwhere=%s"; then the SD read session and DoRestore().
    fn restore_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let line = cmd.trim_end_matches(['\n', '\0']);
        let replace = arg(line, "replace").and_then(|r| r.bytes().next());
        let prelinks = arg(line, "prelinks").and_then(|p| p.parse::<i32>().ok());
        let (where_arg, regex) = match (line.split_once(" where="), line.split_once(" regexwhere=")) {
            (Some((_, w)), _) => (w.split_whitespace().next().unwrap_or("").to_string(), false),
            (None, Some((_, w))) => (w.split_whitespace().next().unwrap_or("").to_string(), true),
            (None, None) => (String::new(), false),
        };
        let has_where = line.contains(" where=") || line.contains(" regexwhere=");
        let (Some(replace), Some(prelinks), true) = (replace, prelinks, has_where) else {
            self.msgs.jmsg(M_FATAL, &format!("Bad replace command. CMD={cmd}"));
            return false;
        };
        let mut args = where_arg;
        if args == "/" {
            args.clear();
        }
        let args = unbash(&args);
        debug!("Got replace {}, where={args}", replace as char);
        let mut create = CreateState { keep_path_list: replace == REPLACE_NEVER, ..Default::default() };
        let where_ = if regex {
            match crate::breg::get_bregexps(&args) {
                Some(list) => Where::Regex(list),
                None => {
                    self.msgs.jmsg(M_FATAL, &format!("Bad where regexp. where={args}\n"));
                    return false;
                }
            }
        } else {
            Where::Prefix(args.into_bytes())
        };
        let params = RestoreParams { where_, replace, prefix_links: prelinks != 0 };
        self.reply(dir, "2000 OK restore\n");
        self.msgs.set_status(JS_BLOCKED);

        let mut ticket = 0;
        if self.open_sd_read_session(&mut ticket) {
            self.msgs.set_status(JS_RUNNING);
            let shared = self.shared.clone().expect("job registered");
            let mut sd = self.sd.take().expect("sd open");
            {
                let mut flush = |m: &mut JobMsgs| m.flush(dir);
                Restore {
                    params: &params,
                    msgs: &mut self.msgs,
                    counters: &mut self.counters,
                    create: &mut create,
                    canceled: &shared.canceled,
                }
                .run(&mut sd, &mut flush);
            }
            if self.is_canceled() {
                self.msgs.set_status(JS_CANCELED);
            }
            self.msgs.set_status(if self.msgs.job_warnings > 0 { JS_WARNINGS } else { JS_TERMINATED });
            // Close the read session and tell the SD we are done.
            if sd.send_str(&format!("read close session {ticket}\n")).is_ok() {
                let _ = sd.recv();
            }
            let _ = sd.send_signal(BnetSignal::Terminate);
            *shared.sd_socket.lock().unwrap_or_else(|e| e.into_inner()) = None;
        } else {
            self.msgs.set_status(JS_ERROR_TERMINATED);
        }
        if self.msgs.job_errors > 0 {
            self.msgs.set_status(JS_ERROR_TERMINATED);
        }
        let mut go_on = false;
        if self.multi_restore {
            self.reply(dir, "2000 OK storage end\n");
            go_on = true;
        }
        if self.is_canceled() {
            go_on = false;
        }
        go_on
    }

    /// OpenSdReadSession()
    fn open_sd_read_session(&mut self, ticket: &mut i32) -> bool {
        if self.sd.is_none() {
            // Passive client: the SD connects to us after "run".
            if let Some(shared) = self.shared.clone() {
                self.sd = shared.wait_passive_sd(PASSIVE_SD_WAIT);
            }
        }
        let Some(sd) = self.sd.as_mut() else {
            self.msgs.jmsg(M_FATAL, "Improper calling sequence.\n");
            return false;
        };
        let open = format!(
            "read open session = DummyVolume {} {} {} {} {} {}\n",
            self.vol_session_id, self.vol_session_time, self.start_file, self.end_file, self.start_block, self.end_block
        );
        if sd.send_str(&open).is_err() {
            self.msgs.jmsg(M_FATAL, "Bad response from stored to read open command\n");
            return false;
        }
        match next_msg(sd) {
            Some(m) => match m.trim_end_matches(['\n', '\0']).strip_prefix("3000 OK open ticket = ").and_then(|t| t.trim().parse().ok()) {
                Some(t) => *ticket = t,
                None => {
                    self.msgs.jmsg(M_FATAL, &format!("Bad response to SD read open: {m}\n"));
                    return false;
                }
            },
            None => {
                self.msgs.jmsg(M_FATAL, "Bad response from stored to read open command\n");
                return false;
            }
        }
        if sd.send_str(&format!("read data {ticket}\n")).is_err() {
            self.msgs.jmsg(M_FATAL, "Comm error with SD. bad response to Read Data. ERR=send\n");
            return false;
        }
        match next_msg(sd) {
            Some(m) if m == "3000 OK data\n" => true,
            Some(m) => {
                self.msgs.jmsg(M_FATAL, &format!("Bad response to Read Data command. Wanted 3000 OK data\n, got {m}\n"));
                false
            }
            None => {
                self.msgs.jmsg(M_FATAL, "Comm error with SD. bad response to Read Data. ERR=connection closed\n");
                false
            }
        }
    }
}

/// BgetMsg() for command replies: the next data message as text.
fn next_msg(sd: &mut SdStream) -> Option<String> {
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(d)) => return Some(String::from_utf8_lossy(&d).into_owned()),
            Ok(BnetMessage::Signal(BnetSignal::Heartbeat | BnetSignal::HeartbeatResponse)) => continue,
            _ => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bareos_bnet::CramMd5Auth;
    use std::net::{TcpListener, TcpStream};

    fn recv_text<S: Read + Write>(b: &mut BnetStream<S>) -> String {
        loop {
            match b.recv().unwrap() {
                BnetMessage::Data(d) => return String::from_utf8_lossy(&d).into_owned(),
                BnetMessage::Signal(_) => continue,
            }
        }
    }

    /// A Storage daemon serving one read session with one small file.
    fn fake_sd(listener: TcpListener, key: &'static str) -> std::thread::JoinHandle<Vec<String>> {
        std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut sd = BnetStream::new(s);
            let mut seen = vec![recv_text(&mut sd)];
            let auth = CramMd5Auth::new_raw(key, "fake-sd", 0);
            assert!(auth.two_way_authenticate(&mut sd, true).unwrap().success);
            seen.push(recv_text(&mut sd));
            sd.send_str("3000 OK open ticket = 7\n").unwrap();
            seen.push(recv_text(&mut sd));
            sd.send_str("3000 OK data\n").unwrap();
            let lstat = "A A IGk B A A A F A A A A A A A";
            let attr = format!("1 3 /x/file.txt\0{lstat}\0\0\0");
            for (stream, data) in [(1, attr.as_bytes()), (2, &b"data\n"[..])] {
                sd.send_str(&format!("rechdr 1 2 1 {stream} {}", data.len())).unwrap();
                sd.send(data).unwrap();
            }
            sd.send_signal(BnetSignal::Eod).unwrap();
            seen.push(recv_text(&mut sd));
            sd.send_str("3000 OK close Status = 84\n").unwrap();
            seen
        })
    }

    #[test]
    fn restore_session_with_simulated_director_and_sd() {
        let tmp = std::env::temp_dir().join(format!("bareos-filed-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let sd_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let sd_port = sd_listener.local_addr().unwrap().port();
        let sd_thread = fake_sd(sd_listener, "the-sd-key");

        let config = FdConfig::parse("Client {\n Name = test-fd\n}\nDirector {\n Name = test-dir\n Password = pw\n}\n").unwrap();
        let dir_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = dir_listener.local_addr().unwrap();
        let fd_thread = std::thread::spawn(move || {
            let (s, _) = dir_listener.accept().unwrap();
            let mut b = BnetStream::new(s);
            let director = config.directors[0].clone();
            Session::new(&config, &director, 1).run(&mut b);
        });
        let mut dir = BnetStream::new(TcpStream::connect(addr).unwrap());
        dir.send_str("JobId=42 Job=RestoreFiles.2026-10-17_10.00.00_01 SDid=1 SDtime=2 Authorization=the-sd-key ssl=0\n").unwrap();
        assert!(recv_text(&mut dir).starts_with("2000 OK Job "));
        dir.send_str(&format!("storage address=127.0.0.1 port={sd_port} ssl=0 Authorization=the-sd-key\n")).unwrap();
        let mut replies = Vec::new();
        loop {
            let r = recv_text(&mut dir);
            let done = r.starts_with("2000 OK storage") || r.starts_with("29");
            replies.push(r);
            if done {
                break;
            }
        }
        assert_eq!(replies.last().unwrap(), "2000 OK storage\n", "{replies:?}");
        dir.send_str(&format!("restore replace=a prelinks=0 where={}\n", tmp.display())).unwrap();
        let mut texts = Vec::new();
        loop {
            let r = recv_text(&mut dir);
            texts.push(r.clone());
            if r.starts_with("2000 OK storage end") {
                break;
            }
        }
        assert!(texts.iter().any(|t| t == "2000 OK restore\n"), "{texts:?}");
        // Director ends the job (onesimus-dird sends EOD, the original "endrestore")
        dir.send_str("endrestore").unwrap();
        let end = loop {
            let r = recv_text(&mut dir);
            if r.starts_with("2800 End Job") {
                break r;
            }
        };
        assert_eq!(end, "2800 End Job TermCode=84 JobFiles=1 ReadBytes=5 JobBytes=5 Errors=0 VSS=0 Encrypt=0\n");
        fd_thread.join().unwrap();
        let seen = sd_thread.join().unwrap();
        assert_eq!(seen[0], "Hello Start Job RestoreFiles.2026-10-17_10.00.00_01\n");
        assert_eq!(seen[1], "read open session = DummyVolume 1 2 0 0 0 0\n");
        assert_eq!(seen[2], "read data 7\n");
        assert_eq!(seen[3], "read close session 7\n");
        assert_eq!(std::fs::read(tmp.join("x/file.txt")).unwrap(), b"data\n");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn unknown_command_and_bad_job() {
        let config = FdConfig::parse("Client {\n Name = test-fd\n}\nDirector {\n Name = test-dir\n Password = pw\n}\n").unwrap();
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let t = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let director = config.directors[0].clone();
            Session::new(&config, &director, 1).run(&mut BnetStream::new(s));
        });
        let mut dir = BnetStream::new(TcpStream::connect(addr).unwrap());
        dir.send_str("nonsense\n").unwrap();
        assert_eq!(recv_text(&mut dir), "2999 Invalid command\n");
        t.join().unwrap();
    }
}
