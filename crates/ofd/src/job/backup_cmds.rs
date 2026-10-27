//! The backup side of the Director commands (filed/dir_cmd.cc LevelCmd,
//! FilesetCmd, AccurateCmd, BackupCmd, EstimateCmd, VerifyCmd,
//! RunscriptCmd, RunbeforenowCmd, SetbandwidthCmd, ValidateCommand) and
//! verify_vol.cc DoVerifyVolume.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};
use tracing::debug;

use super::Session;
use crate::scan::scan;
use crate::accurate::FileList;
use crate::backup::{Backup, BackupHandler, EstimateHandler, JobEnv, SdSender, VerifyHandler};
use crate::compression;
use crate::fileset::{FO_ACL, FO_COMPRESS, FO_FORCE_ENCRYPT, FO_XATTR, FileSet};
use crate::find::{FfPkt, Finder};
use crate::jmsg::*;
use crate::runscript::{self, JobCodes, SCRIPT_AFTER, SCRIPT_AFTER_VSS, SCRIPT_BEFORE};
use crate::streams::*;

/// edit_uint64_with_commas()
pub fn with_commas(v: u64) -> String {
    let s = v.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn now_btime() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

/// job_level_to_str()
fn level_str(l: u8) -> &'static str {
    match l {
        b'F' => "Full",
        b'I' => "Incremental",
        b'D' => "Differential",
        b'S' => "Since",
        b'B' => "Base",
        b'C' => "Verify Catalog",
        b'V' => "Verify Init",
        b'O' => "Verify Volume to Catalog",
        b'd' => "Verify Disk to Catalog",
        b'A' => "Verify Data",
        _ => " ",
    }
}

/// job_type_to_str()
fn type_str(t: u8) -> &'static str {
    match t {
        b'B' => "Backup",
        b'R' => "Restore",
        b'V' => "Verify",
        _ => "Unknown Type",
    }
}

impl Session<'_> {
    /// ValidateCommand(): the Director's (else the Client's) Allowed Job
    /// Command list.
    pub(super) fn validate_command(&mut self, cmd: &str) -> bool {
        let list = if !self.director.allowed_job_cmds.is_empty() {
            &self.director.allowed_job_cmds
        } else {
            &self.config.allowed_job_cmds
        };
        if list.is_empty() || list.iter().any(|c| c.eq_ignore_ascii_case(cmd)) {
            return true;
        }
        self.msgs.jmsg(M_FATAL, &format!("Illegal \"{cmd}\" command not allowed by Allowed Job Cmds setting of this filed.\n"));
        false
    }

    fn allowed_script_dirs(&self) -> Vec<String> {
        if !self.director.allowed_script_dirs.is_empty() {
            self.director.allowed_script_dirs.clone()
        } else {
            self.config.allowed_script_dirs.clone()
        }
    }

    fn job_codes_run(&mut self, cmd: &str) -> String {
        let codes = JobCodes {
            job_bytes: self.counters.job_bytes,
            job_files: self.counters.job_files,
            client: &self.config.name,
            my_name: &self.config.name,
            status: self.msgs.status(),
            job_id: self.msgs.job_id,
            job: &self.msgs.job,
            level: level_str(self.level),
            job_type: type_str(self.job_type),
            director: &self.director.name,
            since_time: self.since_time,
        };
        runscript::edit_job_codes(cmd, &codes)
    }

    /// RunScripts()
    pub(super) fn run_scripts(&mut self, label: &str) {
        let when = if label.contains("Before") {
            if self.pre_scripts_ran {
                return;
            }
            self.pre_scripts_ran = true;
            SCRIPT_BEFORE
        } else if label == "ClientAfterVSS" {
            SCRIPT_AFTER_VSS
        } else {
            if self.post_scripts_ran {
                return;
            }
            self.post_scripts_ran = true;
            SCRIPT_AFTER
        };
        let allowed = self.allowed_script_dirs();
        for script in self.runscripts.clone() {
            let canceled = self.job_canceled();
            if !runscript::should_run(&script, when, self.msgs.status(), canceled) {
                continue;
            }
            if !runscript::script_dir_allowed(&script.command, &allowed) {
                self.msgs.jmsg(M_ERROR, &format!(
                    "Runscript: run {label} \"{}\" could not execute, not in one of the allowed scripts dirs\n", script.command));
                self.msgs.set_status(JS_ERROR_TERMINATED);
                return;
            }
            let ecmd = self.job_codes_run(&script.command);
            self.msgs.jmsg(M_INFO, &format!("shell command: run {label} \"{ecmd}\"\n"));
            let ok = match runscript::run_program(&ecmd) {
                Ok((out, status)) => {
                    for line in out.lines() {
                        self.msgs.jmsg(M_INFO, &format!("{label}: {}\n", line.trim_end()));
                    }
                    if status != 0 {
                        // BErrNo::bstrerror() of b_errno_exit: codes >= 200 are execvp errors
                        const EXECVP: [i32; 9] = [libc::EACCES, libc::ENOEXEC, libc::EFAULT, libc::EINTR, libc::E2BIG,
                            libc::ENAMETOOLONG, libc::ENOMEM, libc::ETXTBSY, libc::ENOENT];
                        let err = if status >= 200 {
                            match EXECVP.get((status - 200) as usize) {
                                Some(e) => crate::berrno::BErr(*e).to_string(),
                                None => "Unknown error during program execvp".to_string(),
                            }
                        } else {
                            format!("Child exited with code {status}")
                        };
                        self.msgs.jmsg(M_ERROR, &format!("Runscript: {label} returned non-zero status={status}. ERR={err}\n"));
                        false
                    } else {
                        true
                    }
                }
                Err(e) => {
                    // OpenBpipe() forks, execvp() fails in the child:
                    // exit code 200 + index of execvp_errors.
                    const EXECVP: [i32; 9] = [libc::EACCES, libc::ENOEXEC, libc::EFAULT, libc::EINTR, libc::E2BIG,
                        libc::ENAMETOOLONG, libc::ENOMEM, libc::ETXTBSY, libc::ENOENT];
                    let errno = e.raw_os_error().unwrap_or(0);
                    let code = EXECVP.iter().position(|&x| x == errno).map(|i| 200 + i as i32).unwrap_or(255);
                    let err = if code == 255 { "Unknown error during program execvp".to_string() } else { std::io::Error::from_raw_os_error(errno).to_string() };
                    self.msgs.jmsg(M_ERROR, &format!("Runscript: {label} returned non-zero status={code}. ERR={err}\n"));
                    false
                }
            };
            if !ok && script.fail_on_error {
                self.msgs.set_status(JS_ERROR_TERMINATED);
            }
        }
    }

    /// RunscriptCmd()
    pub(super) fn runscript_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        if !self.validate_command("runscript") {
            self.reply(dir, "2905 Failed RunScript\n");
            return false;
        }
        match runscript::parse_runscript(cmd.trim_end_matches(['\n', '\0'])) {
            Some(s) => {
                self.runscripts.push(s);
                self.reply(dir, "2000 OK RunScript\n")
            }
            None => {
                self.msgs.jmsg(M_FATAL, &format!("Bad RunScript command: {cmd}"));
                self.reply(dir, "2905 Failed RunScript\n");
                false
            }
        }
    }

    /// RunbeforenowCmd()
    pub(super) fn run_before_now_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        self.run_scripts("ClientBeforeJob");
        if self.job_canceled() {
            self.reply(dir, "2905 Failed RunScript\n");
            false
        } else {
            self.reply(dir, "2000 OK RunBeforeNow\n")
        }
    }

    /// SetbandwidthCmd(): "setbandwidth=%lld Job=%127s"
    pub(super) fn setbandwidth_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let v = scan(cmd.as_bytes(), "setbandwidth=%lld Job=%127s", 2);
        let Some((bw, job)) = v.map(|v| (v[0].int(), v[1].text())).filter(|(bw, _)| *bw >= 0) else {
            self.reply(dir, &format!("2991 Bad setbandwidth command: {cmd}\n"));
            return false;
        };
        if !job.is_empty() {
            match crate::sd::find(&job) {
                Some(shared) => {
                    shared.bw.limit.store(bw as u64, Ordering::Relaxed);
                    shared.bw.bursting.store(self.config.allow_bw_bursting, Ordering::Relaxed);
                }
                None => {
                    self.reply(dir, &format!("2901 Job {job} not found.\n"));
                }
            }
        }
        self.reply(dir, "2000 OK Bandwidth\n")
    }

    /// LevelCmd()
    pub(super) fn level_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let line = cmd.trim_end_matches(['\n', '\0']);
        if line.contains("accurate") {
            self.accurate = true;
        }
        if line.contains("rerunning") {
            self.rerunning = true;
        }
        let bad = |s: &mut Self, dir: &mut BnetStream<S>| {
            s.msgs.jmsg(M_FATAL, &format!("Bad level command: {cmd}\n"));
            let _ = dir;
            false
        };
        let Some(level) = line.strip_prefix("level = ").and_then(|r| r.split_whitespace().next()) else {
            return bad(self, dir);
        };
        if level == "base" {
            self.level = b'B';
        } else if level == "full" {
            self.level = b'F';
        } else if level.contains("differential") {
            self.level = b'D';
            return true;
        } else if level.contains("incremental") {
            self.level = b'I';
            return true;
        } else if level == "since_utime" {
            if self.level == b' ' {
                self.level = b'S';
            }
            let rest = line.split_once("since_utime").map(|(_, r)| r).unwrap_or("");
            let mut it = rest.split_whitespace();
            let Some(since) = it.next().and_then(|v| v.parse::<i64>().ok()) else { return bad(self, dir) };
            let Some(_mtime_only) = it.next().and_then(|v| v.strip_prefix("mtime_only=")).and_then(|v| v.parse::<i32>().ok()) else {
                return bad(self, dir);
            };
            if let Some(pj) = it.next().and_then(|v| v.strip_prefix("prev_job=")) {
                self.prev_job = pj.to_string();
            }
            // Sync clocks: 10 BNET_BTIME polls, the first two tossed.
            let mut bt_adj: i64 = 0;
            for i in 0..10 {
                let bt_start = now_btime();
                if dir.send_signal(BnetSignal::Btime).is_err() {
                    return bad(self, dir);
                }
                let his = match dir.recv() {
                    Ok(BnetMessage::Data(d)) => String::from_utf8_lossy(&d).into_owned(),
                    _ => return bad(self, dir),
                };
                let Some(his) = his.strip_prefix("btime ").and_then(|v| v.trim().parse::<i64>().ok()) else {
                    return bad(self, dir);
                };
                if i < 2 {
                    continue;
                }
                let rt = now_btime() - bt_start;
                bt_adj += bt_start - his - rt / 2;
            }
            bt_adj /= 8;
            let adj = bt_adj / 1_000_000;
            let since = since + adj;
            if !(-3..=3).contains(&adj) {
                let t = if !(-600..=600).contains(&adj) { M_WARNING } else { M_INFO };
                self.msgs.jmsg(t, &format!("DIR and FD clocks differ by {adj} seconds, FD automatically compensating.\n"));
            }
            let _ = dir.send_signal(BnetSignal::Eod);
            self.incremental = true;
            self.since_time = since;
        } else {
            self.msgs.jmsg(M_FATAL, &format!("Unknown backup level: {level}\n"));
            return false;
        }
        self.reply(dir, "2000 OK level\n")
    }

    /// FilesetCmd()
    pub(super) fn fileset_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, _cmd: &str) -> bool {
        if self.fileset.is_some() {
            return false;
        }
        let mut fs = FileSet::default();
        let director = self.director.name.clone();
        let since = self.since_time;
        while let Ok(BnetMessage::Data(d)) = dir.recv() {
            let line = String::from_utf8_lossy(&d);
            let line = line.trim_end_matches(|c: char| c.is_whitespace() || c == '\0');
            let msgs = &mut self.msgs;
            let program = |p: &str| {
                // edit_job_codes(jcr, ..., job_code_callback_filed)
                p.replace("%D", &director).replace("%m", &since.to_string())
            };
            fs.add(line, &mut |m| msgs.jmsg(M_FATAL, &m), &program);
        }
        let ok = !fs.is_error();
        self.fileset = Some(fs);
        if !ok {
            return false;
        }
        self.reply(dir, "2000 OK include\n")
    }

    /// AccurateCmd(): "accurate files=%u", then the file records. No reply.
    pub(super) fn accurate_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        if self.is_canceled() {
            return true;
        }
        let Some(count) = scan(cmd.as_bytes(), "accurate files=%u", 1).map(|v| v[0].int() as usize) else {
            self.reply(dir, "2991 Bad accurate command\n");
            return false;
        };
        let mut list = FileList::new(count);
        self.accurate = true;
        while let Ok(BnetMessage::Data(d)) = dir.recv() {
            list.add_record(&d);
        }
        for m in list.end_load_messages() {
            self.msgs.jmsg(M_ERROR, &m);
        }
        self.file_list = Some(list);
        true
    }

    /// The packet FindFiles() starts with (SetFindOptions).
    fn new_ff(&self) -> FfPkt {
        FfPkt { incremental: self.incremental, save_time: self.since_time, ..Default::default() }
    }

    /// BackupCmd()
    pub(super) fn backup_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        if !self.validate_command("backup") {
            return false;
        }
        if let Some(v) = scan(cmd.as_bytes(), "backup FileIndex=%ld\n", 1) {
            self.counters.job_files = v[0].int() as u32;
        }
        let mut fs = self.fileset.take().unwrap_or_default();
        // ClearCompressionFlagInFileset(), LogFlagStatus()
        let mut warnings = Vec::new();
        for inc in &mut fs.include {
            for fo in &mut inc.opts {
                if fo.flags.has(FO_COMPRESS) && !compression::compressor_available(fo.compress_algo) {
                    warnings.push(format!(
                        "{} compression support requested in fileset but not available on this platform. Disabling ...\n",
                        compression::compressor_name(fo.compress_algo)));
                    fo.flags.clear(FO_COMPRESS);
                    fo.compress_algo = 0;
                }
            }
        }
        for w in warnings {
            self.msgs.jmsg(M_WARNING, &w);
        }
        let flag_on = |bit| fs.include.iter().any(|i| i.opts.iter().any(|o| o.flags.has(bit)));
        let (xattr, acl) = (flag_on(FO_XATTR), flag_on(FO_ACL));
        self.msgs.jmsg(M_INFO, &format!("Extended attribute support {}\n", if xattr { "is enabled" } else { "is disabled" }));
        self.msgs.jmsg(M_INFO, &format!("ACL support {}\n", if acl { "is enabled" } else { "is disabled" }));
        // GetWantedCryptoCipher() without PKI keys
        let ciphers = fs.include.iter().flat_map(|i| i.opts.iter()).filter(|o| o.encryption).count();
        if ciphers > 1 {
            self.msgs.jmsg(M_FATAL, "Fileset contains multiple cipher settings\n");
            self.reply(dir, "2902 Bad backup\n");
            return false;
        } else if ciphers == 1 {
            self.msgs.jmsg(M_FATAL, "Fileset contains cipher settings but PKI Key Pair is not configured\n");
            self.reply(dir, "2902 Bad backup\n");
            return false;
        }
        if flag_on(FO_FORCE_ENCRYPT) {
            self.msgs.jmsg(M_FATAL, "Fileset forces encryption but encryption is not configured\n");
            self.reply(dir, "2902 Bad backup\n");
            return false;
        }
        self.msgs.set_status(JS_BLOCKED);
        self.job_type = b'B';
        if self.sd.is_none() {
            // Passive client: the SD connects to us after "run".
            if let Some(shared) = self.shared.clone() {
                self.sd = shared.wait_passive_sd(super::PASSIVE_SD_WAIT);
            }
        }
        let Some(mut sd) = self.sd.take() else {
            self.msgs.jmsg(M_FATAL, "Cannot contact Storage daemon\n");
            self.reply(dir, "2902 Bad backup\n");
            return false;
        };
        self.reply(dir, "2000 OK backup\n");

        let ticket = match send_and_get(&mut sd, "append open session\n") {
            Some(m) => match scan(m.as_bytes(), "3000 OK open ticket = %d", 1) {
                Some(v) => v[0].int() as i32,
                None => {
                    self.msgs.jmsg(M_FATAL, &format!("Bad response to append open: {m}\n"));
                    self.sd = Some(sd);
                    return false;
                }
            },
            None => {
                self.msgs.jmsg(M_FATAL, "Bad response from stored to open command\n");
                self.sd = Some(sd);
                return false;
            }
        };
        if !self.sd_response(&mut sd, &format!("append data {ticket}\n"), "3000 OK data\n", "Append Data") {
            self.sd = Some(sd);
            return false;
        }

        // BlastDataToStorageDaemon()
        let shared = self.shared.clone().expect("job registered");
        let ok = self.blast_data(&mut sd, &fs, dir, &shared.canceled);
        self.fileset = Some(fs);
        if !ok {
            self.msgs.set_status(JS_ERROR_TERMINATED);
        } else {
            self.msgs.set_status(JS_TERMINATED);
            if !matches!(self.msgs.status(), JS_TERMINATED | JS_WARNINGS) {
                self.sd = Some(sd);
                return false;
            }
            let fin = (|| {
                if !self.sd_response_only(&mut sd, "3000 OK append data\n", "Append Data") {
                    return false;
                }
                if !self.sd_response(&mut sd, &format!("append end session {ticket}\n"), "3000 OK end\n", "Append End") {
                    return false;
                }
                if sd.send_str(&format!("append close session {ticket}\n")).is_err() {
                    self.msgs.jmsg(M_FATAL, "Append Close with SD failed.\n");
                    return false;
                }
                let mut status = None;
                while let Some(m) = next_data(&mut sd) {
                    if let Some(v) = scan(m.as_bytes(), "3000 OK close Status = %d\n", 1) {
                        status = Some(v[0].int() as i32);
                    }
                }
                match status {
                    None => self.msgs.jmsg(M_FATAL, "Append Close with SD failed.\n"),
                    Some(s) if s != i32::from(JS_TERMINATED) && s != i32::from(JS_WARNINGS) => {
                        self.msgs.jmsg(M_FATAL, &format!("Bad status {s} returned from Storage Daemon.\n"));
                    }
                    _ => {}
                }
                true
            })();
            if !fin {
                self.msgs.set_status(JS_ERROR_TERMINATED);
            }
        }
        self.sd = Some(sd);
        false
    }

    /// The data part of BlastDataToStorageDaemon().
    fn blast_data<S: Read + Write, D: Read + Write>(&mut self, sd: &mut BnetStream<S>, fs: &FileSet, dir: &mut BnetStream<D>, canceled: &AtomicBool) -> bool {
        self.msgs.set_status(JS_RUNNING);
        self.msgs.jmsg(M_INFO, &format!("Version: {} ({}) {}\n", crate::version::FULL, crate::version::DATE, crate::version::os_info()));
        self.msgs.flush(dir);
        let mut ff = self.new_ff();
        let mut list = self.file_list.take();
        let level_full = matches!(self.level, b'F');
        let rerunning = self.rerunning;
        let accurate = self.accurate;
        let buf_size = self.config.buf_size();
        let workers = self.config.max_workers_per_job;
        let shared = self.shared.clone();
        if let Some(s) = &shared {
            s.progress.start(self.msgs.job_id, &format!("{} Backup", level_str(self.level)));
        }
        let mut flush = |m: &mut JobMsgs| m.flush(dir);
        let mut b = Backup {
            sd: SdSender { sd, msgs: &mut self.msgs, counters: &mut self.counters, canceled,
                           progress: shared.as_ref().map(|s| &s.progress) },
            env: JobEnv { canceled, accurate: if accurate { list.as_mut() } else { None }, level_full, rerunning, base_size: 0 },
            flush: &mut flush,
            buf_size,
            workers,
            acl_errors: 0,
            xattr_errors: 0,
            xattr_disabled: false,
        };
        let mut ok = {
            let mut h = BackupHandler { b: &mut b, fileset: fs };
            Finder::new(fs).find_files(&mut ff, &mut h) != 0
        };
        if !ok {
            b.sd.msgs.set_status(JS_ERROR_TERMINATED);
        }
        if b.acl_errors > 0 {
            let n = b.acl_errors;
            b.sd.msgs.jmsg(M_WARNING, &format!("Encountered {n} acl errors while doing backup\n"));
        }
        if b.xattr_errors > 0 {
            let n = b.xattr_errors;
            b.sd.msgs.jmsg(M_WARNING, &format!("Encountered {n} xattr errors while doing backup\n"));
        }
        if !b.accurate_finish() {
            ok = false;
        }
        if b.sd.sd.send_signal(BnetSignal::Eod).is_err() {
            ok = false;
        }
        (b.flush)(b.sd.msgs);
        ok
    }

    /// response(): send `cmd`, expect exactly `want`.
    fn sd_response<S: Read + Write>(&mut self, sd: &mut BnetStream<S>, cmd: &str, want: &str, what: &str) -> bool {
        if sd.send_str(cmd).is_err() {
            self.msgs.jmsg(M_FATAL, &format!("Comm error with SD. bad response to {what}. ERR=Broken pipe\n"));
            return false;
        }
        self.sd_response_only(sd, want, what)
    }

    fn sd_response_only<S: Read + Write>(&mut self, sd: &mut BnetStream<S>, want: &str, what: &str) -> bool {
        match next_data(sd) {
            Some(m) if m == want => true,
            Some(m) => {
                if !self.is_canceled() {
                    self.msgs.jmsg(M_FATAL, &format!("Bad response to {what} command. Wanted {want}, got {m}\n"));
                }
                false
            }
            None => {
                if !self.is_canceled() {
                    self.msgs.jmsg(M_FATAL, &format!("Comm error with SD. bad response to {what}. ERR=connection closed\n"));
                }
                false
            }
        }
    }

    /// EstimateCmd(): "estimate listing=%d"
    pub(super) fn estimate_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        if !self.validate_command("estimate") {
            self.reply(dir, "2992 Bad estimate command.\n");
            return false;
        }
        let Some(listing) = scan(cmd.as_bytes(), "estimate listing=%d", 1).map(|v| v[0].int()) else {
            self.msgs.jmsg(M_FATAL, &format!("Bad estimate command: {cmd}\n"));
            self.reply(dir, "2992 Bad estimate command.\n");
            return false;
        };
        self.msgs.set_status(JS_RUNNING);
        let fs = self.fileset.take().unwrap_or_default();
        let mut ff = self.new_ff();
        let mut list = self.file_list.take();
        let canceled = AtomicBool::new(false);
        let level_full = matches!(self.level, b'F');
        let accurate = self.accurate;
        {
            let mut h = EstimateHandler {
                counters: &mut self.counters,
                msgs: &mut self.msgs,
                env: JobEnv { canceled: &canceled, accurate: if accurate { list.as_mut() } else { None }, level_full,
                    rerunning: self.rerunning, base_size: 0 },
                listing: listing != 0,
            };
            Finder::new(&fs).find_files(&mut ff, &mut h);
        }
        self.fileset = Some(fs);
        let reply = format!("2000 OK estimate files={} bytes={}\n", with_commas(u64::from(self.counters.num_files_examined)),
            with_commas(self.counters.job_bytes));
        self.reply(dir, &reply);
        let _ = dir.send_signal(BnetSignal::Eod);
        true
    }

    /// VerifyCmd(): "verify level=%30s"
    pub(super) fn verify_cmd<S: Read + Write>(&mut self, dir: &mut BnetStream<S>, cmd: &str) -> bool {
        let line = cmd.trim_end_matches(['\n', '\0']);
        if !self.validate_command("verify") {
            self.reply(dir, &format!("2994 Bad verify command: {cmd}\n"));
            return false;
        }
        self.job_type = b'V';
        let level = scan(line.as_bytes(), "verify level=%30s", 1).map(|v| v[0].text());
        let Some(level) = level.as_deref() else {
            self.reply(dir, &format!("2994 Bad verify command: {cmd}\n"));
            return false;
        };
        self.level = match level.to_ascii_lowercase().as_str() {
            "init" => b'V',
            "catalog" => b'C',
            "volume" => b'O',
            "data" => b'A',
            "disk_to_catalog" => b'd',
            _ => {
                self.reply(dir, &format!("2994 Bad verify level: {cmd}\n"));
                return false;
            }
        };
        self.reply(dir, "2000 OK verify\n");
        match self.level {
            b'V' | b'C' | b'd' => self.do_verify(dir),
            b'O' => {
                let mut ticket = 0;
                if !self.open_sd_read_session(&mut ticket) {
                    return false;
                }
                let mut sd = self.sd.take().expect("sd open");
                self.do_verify_volume(&mut sd, dir);
                if sd.send_str(&format!("read close session {ticket}\n")).is_ok() {
                    let _ = sd.recv();
                }
                let _ = sd.send_signal(BnetSignal::Terminate);
                self.old_sd.push(sd);
            }
            _ => {
                self.reply(dir, &format!("2994 Bad verify level: {cmd}\n"));
                return false;
            }
        }
        let _ = dir.send_signal(BnetSignal::Eod);
        false
    }

    /// DoVerify()
    fn do_verify<S: Read + Write>(&mut self, dir: &mut BnetStream<S>) {
        self.msgs.set_status(JS_RUNNING);
        let fs = self.fileset.take().unwrap_or_default();
        let mut ff = self.new_ff();
        let canceled = self.shared.as_ref().map(|s| s.canceled.load(Ordering::Relaxed)).unwrap_or(false);
        let canceled = AtomicBool::new(canceled);
        {
            let mut h = VerifyHandler { dir, counters: &mut self.counters, msgs: &mut self.msgs, canceled: &canceled };
            Finder::new(&fs).find_files(&mut ff, &mut h);
        }
        self.fileset = Some(fs);
        self.msgs.set_status(JS_TERMINATED);
    }

    /// DoVerifyVolume(): the attributes and digests read from the volume
    /// to the Director.
    fn do_verify_volume<S: Read + Write, D: Read + Write>(&mut self, sd: &mut BnetStream<S>, dir: &mut BnetStream<D>) {
        self.msgs.set_status(JS_RUNNING);
        let mut fname: Vec<u8> = Vec::new();
        let ok = loop {
            if self.is_canceled() {
                break true;
            }
            self.msgs.flush(dir);
            let Some(hdr) = next_raw(sd) else { break true };
            let h = String::from_utf8_lossy(&hdr).into_owned();
            let parsed = scan(&hdr, "rechdr %u %u %d %d %u", 5).map(|v| (v[3].int() as i32, v[4].int() as u32));
            let Some((stream, size)) = parsed else {
                self.msgs.jmsg(M_FATAL, &format!("Record header scan error: {h}\n"));
                break false;
            };
            let Some(data) = next_raw(sd) else {
                self.msgs.jmsg(M_FATAL, "Data record error. ERR=connection closed\n");
                break false;
            };
            if size as usize != data.len() {
                self.msgs.jmsg(M_FATAL, &format!("Actual data size {} not same as header {size}\n", data.len()));
                break false;
            }
            let digest_name = match stream {
                STREAM_MD5_DIGEST => Some("MD5"),
                STREAM_SHA1_DIGEST => Some("SHA1"),
                STREAM_SHA256_DIGEST => Some("SHA256"),
                STREAM_SHA512_DIGEST => Some("SHA512"),
                STREAM_XXH128_DIGEST => Some("XXH128"),
                _ => None,
            };
            match stream {
                STREAM_UNIX_ATTRIBUTES | STREAM_UNIX_ATTRIBUTES_EX => {
                    let mut parts = data.splitn(3, |&b| b == b' ');
                    let _fi = parts.next();
                    let _ty = parts.next();
                    let Some(t) = scan(&data, "%d %d", 2).map(|v| v[1].int() as i32) else {
                        self.msgs.jmsg(M_FATAL, &format!("Error scanning record header: {h}\n"));
                        break false;
                    };
                    let rest = parts.next().unwrap_or(&[]);
                    let mut fields = rest.split(|&b| b == 0);
                    fname = fields.next().unwrap_or(&[]).to_vec();
                    let attr = fields.next().unwrap_or(&[]).to_vec();
                    let lname = if t == crate::attribs::FT_LNK || t == crate::attribs::FT_LNKSAVED {
                        fields.next().unwrap_or(&[]).to_vec()
                    } else {
                        Vec::new()
                    };
                    self.counters.job_files += 1;
                    self.counters.num_files_examined += 1;
                    let fi = if t == crate::attribs::FT_DELETED { 0 } else { self.counters.job_files };
                    let mut msg = format!("{fi} {STREAM_UNIX_ATTRIBUTES} pinsug5 ").into_bytes();
                    msg.extend_from_slice(&fname);
                    msg.push(0);
                    msg.extend_from_slice(&attr);
                    msg.push(0);
                    msg.extend_from_slice(&lname);
                    msg.push(0);
                    if dir.send(&msg).is_err() {
                        self.msgs.jmsg(M_FATAL, "Network error in send to Director: ERR=Broken pipe\n");
                        break false;
                    }
                }
                STREAM_RESTORE_OBJECT => {
                    self.counters.job_files += 1;
                    self.counters.num_files_examined += 1;
                    let mut msg = format!("{} {STREAM_RESTORE_OBJECT} ReStOrEObJeCt ", self.counters.job_files).into_bytes();
                    msg.extend_from_slice(&fname);
                    msg.extend_from_slice(&[0, 0, 0]);
                    let _ = dir.send(&msg);
                }
                _ => {
                    if let Some(name) = digest_name {
                        let b64 = crate::digest::to_base64(&data);
                        let fi = self.counters.job_files;
                        let _ = dir.send_str(&format!("{fi} {stream} {b64} *{name}-{fi}*"));
                    }
                }
            }
        };
        self.msgs.set_status(if ok { JS_TERMINATED } else { JS_ERROR_TERMINATED });
        debug!("End Verify-Vol. Files={} Bytes={}", self.counters.job_files, self.counters.job_bytes);
    }
}

/// BgetMsg(): the next data message, skipping heartbeats; None at a signal.
fn next_raw<S: Read + Write>(sd: &mut BnetStream<S>) -> Option<Vec<u8>> {
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(d)) => return Some(d),
            Ok(BnetMessage::Signal(BnetSignal::Heartbeat | BnetSignal::HeartbeatResponse)) => continue,
            _ => return None,
        }
    }
}

fn next_data<S: Read + Write>(sd: &mut BnetStream<S>) -> Option<String> {
    next_raw(sd).map(|d| String::from_utf8_lossy(&d).into_owned())
}

fn send_and_get<S: Read + Write>(sd: &mut BnetStream<S>, cmd: &str) -> Option<String> {
    sd.send_str(cmd).ok()?;
    next_data(sd)
}

#[cfg(test)]
mod tests {
    #[test]
    fn commas() {
        assert_eq!(super::with_commas(1234567), "1,234,567");
        assert_eq!(super::with_commas(12), "12");
    }
}
