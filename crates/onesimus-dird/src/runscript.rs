//! Director-side RunScripts: lib/runscript.cc RunScripts() and
//! RunScript::Run(), lib/util.cc edit_job_codes() with
//! dird_conf.cc job_code_callback_director(), lib/bpipe_unix.cc
//! OpenBpipe()/CloseBpipe() and lib/berrno.cc, console commands as
//! job.cc RunConsoleCommand(), and the client scripts fd_cmds.cc
//! SendRunscriptsCommands() sends to the File daemon.
//!
//! Callers: job_thread "BeforeJob" (the worker asks the Director, see
//! `PipeMessage::RunScripts`), "AfterJob" at the end of every job
//! (job_manager.rs) and in `cancel` (job.cc:741). Compatibility log C-048,
//! C-058, S-07.

use std::io::{BufRead, BufReader};
use std::sync::{Mutex, Weak};

use bareos_config::raw::ConfResource;

use crate::catalog_worker::CatalogHandle;
use crate::commands::show::{runscripts, Script};
use crate::director_config::{DirectorConfig, SharedConfig};
use crate::job_manager::{JobManager, ScriptJob};
use crate::job_worker::WorkerJobType;
use crate::message_bus::{BusMessage, MessageBus, MsgType};

/// The values edit_job_codes() substitutes for a job.
#[derive(Debug, Clone, Default)]
pub struct JobCodes {
    pub job_id: u32,
    /// jcr->Job (unique job name)
    pub job: String,
    pub client: String,
    pub client_address: String,
    pub level: String,
    pub job_type: String,
    pub files: i64,
    pub bytes: i64,
    pub status: char,
    pub director: String,
    pub fileset: String,
    pub pool: String,
    pub storage: String,
    /// jcr->VolumeName (%v, %V); None: no JCR buffer ("*None*").
    pub volume: Option<String>,
}

const NONE: &str = "*None*";

/// lib/util.cc job_status_to_str().
fn job_status_to_str(status: char) -> &'static str {
    match status {
        'T' => "OK",
        'W' => "OK -- with warnings",
        'E' | 'e' => "Error",
        'f' => "Fatal Error",
        'A' => "Canceled",
        'D' => "Differences",
        _ => "Unknown term code",
    }
}

/// lib/util.cc edit_job_codes() with the Director's callback; `to` is the
/// recipient list (%r).
pub fn edit_job_codes(input: &str, codes: &JobCodes, to: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(code) = chars.next() else {
            // "%" at the end: the callback knows no NUL code.
            out.push('%');
            break;
        };
        let value: String = match code {
            '%' => "%".into(),
            'B' => format!("{}B", crate::commands::status::edit_uint64_with_suffix(codes.bytes.max(0) as u64)),
            'F' => codes.files.to_string(),
            'P' => std::process::id().to_string(),
            'b' => codes.bytes.to_string(),
            'c' => if codes.client.is_empty() { NONE.into() } else { codes.client.clone() },
            'd' | 'D' => codes.director.clone(),
            'e' => job_status_to_str(codes.status).into(),
            'i' => codes.job_id.to_string(),
            'j' => codes.job.clone(),
            'l' => codes.level.clone(),
            'n' => {
                // There are three periods after the Job name.
                let mut name = codes.job.as_str();
                for _ in 0..3 {
                    if let Some(at) = name.rfind('.') { name = &name[..at]; }
                }
                name.into()
            }
            'r' => to.into(),
            'v' | 'V' => codes.volume.clone().unwrap_or_else(|| NONE.into()),
            's' | 'O' | 'N' => NONE.into(),
            't' => codes.job_type.clone(),
            // job_code_callback_director()
            'f' if !codes.fileset.is_empty() => codes.fileset.clone(),
            'h' if !codes.client_address.is_empty() => codes.client_address.clone(),
            'p' if !codes.pool.is_empty() => codes.pool.clone(),
            'w' if !codes.storage.is_empty() => codes.storage.clone(),
            'x' | 'C' => "no".into(),
            other => format!("%{other}"),
        };
        out.push_str(&value);
    }
    out
}

/// lib/bpipe_unix.cc BuildArgcArgv(): blanks separate the arguments, an
/// argument starting with a quote runs to the next same quote.
pub fn build_argv(cmd: &str) -> Vec<String> {
    let bytes = cmd.as_bytes();
    let mut argv = Vec::new();
    let mut p = 0;
    let skip_blanks = |p: &mut usize| while *p < bytes.len() && (bytes[*p] == b' ' || bytes[*p] == b'\t') { *p += 1; };
    skip_blanks(&mut p);
    let mut quote = None;
    if p < bytes.len() && (bytes[p] == b'"' || bytes[p] == b'\'') {
        quote = Some(bytes[p]);
        p += 1;
    }
    while p < bytes.len() && argv.len() < 100 {
        let mut q = p;
        match quote.take() {
            Some(quote) => while q < bytes.len() && bytes[q] != quote { q += 1; },
            None => while q < bytes.len() && bytes[q] != b' ' { q += 1; },
        }
        argv.push(String::from_utf8_lossy(&bytes[p..q]).into_owned());
        p = if q < bytes.len() { q + 1 } else { q };
        skip_blanks(&mut p);
        if p < bytes.len() && (bytes[p] == b'"' || bytes[p] == b'\'') {
            quote = Some(bytes[p]);
            p += 1;
        }
    }
    argv
}

/// The RunScripts of a Job resource with those of its JobDefs.
pub fn job_scripts(raw: &[ConfResource], job_name: &str) -> Vec<Script> {
    let Some(effective) = bareos_config::jobdefs::effective(raw, "job", job_name) else { return Vec::new() };
    runscripts(&ConfResource { rtype: "Job".into(), directives: bareos_config::jobdefs::directives(&effective), source: None })
}

/// lib/runscript.h SCRIPT_After / SCRIPT_Before / SCRIPT_AfterVSS.
const SCRIPT_AFTER: u8 = 1;
const SCRIPT_BEFORE: u8 = 2;
const SCRIPT_AFTER_VSS: u8 = 4;

/// RunScript::when.
fn script_when(script: &Script) -> u8 {
    match script.when {
        Some("before") => SCRIPT_BEFORE,
        Some("after") => SCRIPT_AFTER,
        Some("aftervss") => SCRIPT_AFTER_VSS,
        Some("always") => SCRIPT_BEFORE | SCRIPT_AFTER,
        _ => 0,
    }
}

/// RunScripts(): the `when` a label stands for.
fn label_when(label: &str) -> u8 {
    if label.contains("Before") { SCRIPT_BEFORE } else if label == "ClientAfterVSS" { SCRIPT_AFTER_VSS } else { SCRIPT_AFTER }
}

/// RunScripts(): whether a local script runs for `when` in job `status`.
fn runs(script: &Script, when: u8, status: char) -> bool {
    let canceled = crate::job_manager::is_job_canceled(status);
    let sw = script_when(script);
    let on = |cond_success: bool| (script.on_success && cond_success) || (script.on_failure && (canceled || status == 'D'));
    (sw & SCRIPT_BEFORE != 0 && when & SCRIPT_BEFORE != 0 && on(matches!(status, 'R' | 'C')))
        || (sw & SCRIPT_AFTER_VSS != 0 && when & SCRIPT_AFTER_VSS != 0
            && ((script.on_success && status == 'B') || (script.on_failure && canceled)))
        || (sw & SCRIPT_AFTER != 0 && when & SCRIPT_AFTER != 0 && on(matches!(status, 'T' | 'W')))
}

/// RunScripts(): the local scripts `label` runs for a job whose status
/// stays `status` (no script fails).
pub fn selected<'a>(scripts: &'a [Script], label: &str, status: char) -> Vec<&'a Script> {
    let when = label_when(label);
    scripts.iter().filter(|script| script.target.is_empty() && runs(script, when, status)).collect()
}

/// Whether the Job resource (with its JobDefs) has any RunScript.
pub fn has_scripts(config: &DirectorConfig, job_name: &str) -> bool {
    !job_scripts(&config.raw, job_name).is_empty()
}

/// ua_cmds.cc commands[] with allowed_in_runscript = true.
const RUNSCRIPT_COMMANDS: &[&str] = &[
    ".", ".actiononpurge", ".clients", ".consoles", ".users", ".jobdefs", ".jobs", ".jobstatus", ".locations",
    ".media", ".mediatypes", ".pools", ".profiles", ".storages", ".volstatus", ".bvfs_lsdirs", ".bvfs_lsfiles",
    ".bvfs_update", ".bvfs_get_jobids", ".bvfs_versions", ".bvfs_restore", ".bvfs_cleanup", "delete", "disable",
    "enable", "estimate", "export", "import", "list", "llist", "memory", "move", "prune", "purge", "release",
    "reload", "status", "setbandwidth", "setdebug", "setdevice", "show", "time", "trace", "truncate", "update",
    "version", "whoami",
];

/// What console commands of run scripts need besides the configuration.
#[derive(Clone, Default)]
pub struct Host {
    pub config: Option<SharedConfig>,
    pub catalog: Option<CatalogHandle>,
    /// None while the caller holds the JobManager lock (the command then
    /// runs without job information instead of deadlocking).
    pub job_manager: Option<Weak<Mutex<JobManager>>>,
}

/// The job status after RunScripts() and the M_ERROR job messages it
/// produced (jcr->JobErrors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    pub status: char,
    pub errors: u32,
}

/// job.cc RunConsoleCommand(): Do_a_command() in a UA context without
/// console (ua->runscript): its output goes to the "-RunScript-" control
/// JCR (JobId 0, the Director's Messages resource), one message per send.
pub fn run_console_command(host: &Host, cmd: &str) -> bool {
    let emit = |text: &str| {
        if let Some(config) = host.config.as_ref().and_then(|c| c.read().ok()) {
            crate::console_msg::job_message(&config, "-RunScript-", 0, MsgType::Info, 0, text);
        }
    };
    let mut ast = crate::session::parse_cli_command(cmd);
    ast.raw_line = cmd.to_string();
    if ast.command_name.is_empty() {
        return false;
    }
    let typed = ast.command_name.clone();
    let lower = typed.to_ascii_lowercase();
    let Some(key) = crate::ua_commands_table::UA_COMMANDS.iter().map(|e| e.0).find(|k| k.starts_with(&lower)) else {
        emit(&format!("{typed}: is an invalid command.\n"));
        return false;
    };
    if !RUNSCRIPT_COMMANDS.contains(&key) {
        emit(&format!("Can't use {key} command in a runscript"));
        emit(&format!("{typed}: is an invalid command.\n"));
        return false;
    }
    ast.command_name = key.to_string();
    let ctx = crate::command_executor::ExecContext {
        session_id: 0, config: host.config.clone(), session_mgr: None, catalog: host.catalog.clone(),
        api_mode: crate::command_executor::ApiMode::Human, bus_rx: None,
        job_manager: host.job_manager.as_ref().and_then(|m| m.upgrade()),
    };
    let result = crate::command_executor::dispatch(&ast, &ctx);
    let errors: Vec<String> = if result.errors_inline { Vec::new() } else { result.error_lines() };
    let text: Vec<String> = result.text_output.split_inclusive('\n').map(str::to_string).collect();
    let (first, second) = if result.errors_first { (errors, text) } else { (text, errors) };
    for part in first.iter().chain(second.iter()) {
        emit(part);
    }
    result.success
}

/// lib/signal.cc sig_names.
fn signal_name(signal: i32) -> &'static str {
    match signal {
        libc::SIGHUP => "Hangup",
        libc::SIGINT => "Interrupt",
        libc::SIGQUIT => "Quit",
        libc::SIGILL => "Illegal instruction",
        libc::SIGTRAP => "Trace/Breakpoint trap",
        libc::SIGABRT => "Abort",
        libc::SIGBUS => "BUS error",
        libc::SIGFPE => "Floating-point exception",
        libc::SIGKILL => "Kill, unblockable",
        libc::SIGUSR1 => "User-defined signal 1",
        libc::SIGSEGV => "Segmentation violation",
        libc::SIGUSR2 => "User-defined signal 2",
        libc::SIGPIPE => "Broken pipe",
        libc::SIGALRM => "Alarm clock",
        libc::SIGTERM => "Termination",
        _ => "Invalid signal number",
    }
}

/// lib/bpipe_unix.cc execvp_errors: a failed execvp exits 200 + index.
const EXECVP_ERRORS: [i32; 9] = [libc::EACCES, libc::ENOEXEC, libc::EFAULT, libc::EINTR, libc::E2BIG,
    libc::ENAMETOOLONG, libc::ENOMEM, libc::ETXTBSY, libc::ENOENT];

/// CloseBpipe() status, as (code, BErrNo::bstrerror text).
fn exit_error(code: i32, signal: bool) -> (i32, String) {
    if signal {
        return (code, format!("Child died from signal {code}: {}", signal_name(code)));
    }
    let text = match code {
        0 => "Child exited normally.".to_string(),
        200.. if (code - 200) < EXECVP_ERRORS.len() as i32 =>
            std::io::Error::from_raw_os_error(EXECVP_ERRORS[(code - 200) as usize]).to_string()
                .split(" (os error").next().unwrap_or_default().to_string(),
        200.. => "Unknown error during program execvp".to_string(),
        _ => format!("Child exited with code {code}"),
    };
    (code, text)
}

/// RunScript::Run() of a shell command: the Jmsg texts it produces in order.
/// OpenBpipe(cmd, 0, "r") gives the child's stdout and stderr in one pipe;
/// a failing execvp() prints perror("Program execution failed") into it
/// and the child exits 255.
pub fn run_shell_command(command: &str, name: &str) -> Vec<(MsgType, String)> {
    let mut messages = vec![(MsgType::Info, format!("shell command: run {name} \"{command}\"\n"))];
    let argv = build_argv(command);
    let spawned = (|| {
        let (reader, writer) = std::io::pipe()?;
        let stderr = writer.try_clone()?;
        let child = match argv.split_first() {
            // execvp(NULL, ...) fails with EFAULT
            None => return Ok(Err(std::io::Error::from_raw_os_error(libc::EFAULT))),
            Some((program, args)) => std::process::Command::new(program).args(args)
                .stdin(std::process::Stdio::null()).stdout(writer).stderr(stderr).spawn(),
        };
        // The Command (and with it the pipe's write ends) is gone here.
        Ok::<_, std::io::Error>(child.map(|child| (child, reader)))
    })();
    let mut output: Vec<u8> = Vec::new();
    let status = match spawned {
        Ok(Ok((mut child, reader))) => {
            let mut reader = BufReader::new(reader);
            while reader.read_until(b'\n', &mut output).is_ok_and(|n| n > 0) {
                push_output_lines(&mut messages, name, &output);
                output.clear();
            }
            match child.wait() {
                Ok(exit) => {
                    use std::os::unix::process::ExitStatusExt;
                    match (exit.code(), exit.signal()) {
                        (Some(0), _) => Ok(()),
                        (Some(code), _) => Err(code),
                        (None, Some(signal)) => Err(-signal),
                        (None, None) => Err(255),
                    }
                }
                Err(_) => Err(255),
            }
        }
        Ok(Err(error)) => {
            let text = error.to_string();
            let text = text.split(" (os error").next().unwrap_or_default();
            push_output_lines(&mut messages, name, format!("Program execution failed: {text}\n").as_bytes());
            Err(255)
        }
        Err(error) => {
            messages.push((MsgType::Error, format!("Runscript: {name} could not execute. ERR={error}\n")));
            return messages;
        }
    };
    if let Err(code) = status {
        let (code, text) = if code < 0 { exit_error(-code, true) } else { exit_error(code, false) };
        messages.push((MsgType::Error, format!("Runscript: {name} returned non-zero status={code}. ERR={text}\n")));
    }
    messages
}

/// fgets() into a PM_NAME buffer (130 bytes), StripTrailingJunk(), one
/// "<name>: <line>" message per read.
fn push_output_lines(messages: &mut Vec<(MsgType, String)>, name: &str, line: &[u8]) {
    for chunk in line.chunks(129) {
        let mut chunk = chunk;
        while let [rest @ .., b' ' | b'\n' | b'\r'] = chunk { chunk = rest; }
        messages.push((MsgType::Info, format!("{name}: {}\n", String::from_utf8_lossy(chunk))));
    }
}

/// lib/util.cc job_level_to_str() of a level as the worker parameters
/// carry it (letter, name in any case, or empty for L_NONE).
fn job_level_to_str(level: &str) -> String {
    let level = level.trim();
    // job_level.h letters (case matters: 'D' Differential, 'd' DiskToCatalog)
    let by_letter = match level {
        "" | " " => Some(" "),
        "B" => Some("Base"),
        "F" => Some("Full"),
        "I" => Some("Incremental"),
        "D" => Some("Differential"),
        "S" => Some("Since"),
        "C" => Some("Verify Catalog"),
        "V" => Some("Verify Init Catalog"),
        "O" => Some("Verify Volume to Catalog"),
        "d" => Some("Verify Disk to Catalog"),
        "A" => Some("Verify Data"),
        "f" => Some("Virtual Full"),
        _ => None,
    };
    let by_name = || match level.to_ascii_lowercase().replace(' ', "").as_str() {
        "base" => "Base",
        "full" => "Full",
        "incremental" => "Incremental",
        "differential" => "Differential",
        "since" => "Since",
        "catalog" => "Verify Catalog",
        "initcatalog" => "Verify Init Catalog",
        "volumetocatalog" => "Verify Volume to Catalog",
        "disktocatalog" => "Verify Disk to Catalog",
        "data" => "Verify Data",
        "virtualfull" => "Virtual Full",
        _ => level,
    }.to_string();
    by_letter.map(str::to_string).unwrap_or_else(by_name)
}

/// The job codes of a job of the JobManager.
fn job_codes(config: &DirectorConfig, job: &ScriptJob) -> JobCodes {
    let resource = config.find_job(&job.job_name);
    let client = config.find_client(&job.client);
    let level = job.params.as_ref().map(|p| p.level.clone()).unwrap_or_default();
    JobCodes {
        job_id: job.job_id,
        job: job.unique_job.clone(),
        client: job.client.clone(),
        client_address: client.map(|c| c.address.clone()).unwrap_or_default(),
        level: job_level_to_str(&level),
        job_type: match job.params.as_ref().map(|p| p.job_type) {
            Some(WorkerJobType::Restore) => "Restore".into(),
            Some(WorkerJobType::Admin) => "Admin".into(),
            Some(WorkerJobType::Copy) => "Copy".into(),
            Some(WorkerJobType::Migrate) => "Migrate".into(),
            Some(WorkerJobType::Consolidate) => "Consolidate".into(),
            Some(WorkerJobType::Verify) => "Verify".into(),
            _ => "Backup".into(),
        },
        files: job.files,
        bytes: job.bytes,
        status: job.status,
        director: config.director.name.clone(),
        fileset: resource.map(|r| r.fileset.clone()).unwrap_or_default(),
        // jcr->dir_impl->res.pool: the job's pool unless the run chose one
        pool: job.params.as_ref().map(|p| p.pool_name.clone()).filter(|p| !p.is_empty())
            .or_else(|| resource.map(|r| r.pool.clone())).unwrap_or_default(),
        storage: job.storage.clone(),
        volume: Some(String::new()),
    }
}

/// RunScripts(jcr, job->RunScripts, label) for a job of the JobManager:
/// runs the selected scripts in configuration order and posts their job
/// messages. A failing script with FailJobOnError sets JS_ErrorTerminated,
/// which already decides about the scripts after it.
pub fn run_scripts(host: &Host, bus: Option<&MessageBus>, job: &ScriptJob, label: &str) -> Outcome {
    let mut outcome = Outcome { status: job.status, errors: 0 };
    let Some((scripts, mut codes)) = host.config.as_ref().and_then(|c| c.read().ok())
        .map(|config| (job_scripts(&config.raw, &job.job_name), job_codes(&config, job))) else { return outcome };
    let post = |msg_type: MsgType, text: String| {
        let message = BusMessage::JobMessage {
            job_id: job.job_id, job_name: job.job_name.clone(), msg_type, text,
            timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default().as_secs() as i64,
        };
        match bus {
            Some(bus) => crate::console_msg::post_job_message(bus, host.config.as_ref(), message),
            None => if let (BusMessage::JobMessage { job_id, job_name, msg_type, text, timestamp }, Some(config)) =
                (&message, host.config.as_ref().and_then(|c| c.read().ok())) {
                crate::console_msg::job_message(&config, job_name, *job_id, *msg_type, *timestamp, text);
            },
        }
    };
    let admin = job.params.as_ref().is_some_and(|p| p.job_type == WorkerJobType::Admin);
    let when = label_when(label);
    for script in &scripts {
        if !script.target.is_empty() {
            if admin {
                post(MsgType::Warning, format!(
                    "Invalid runscript definition (command={}). Admin Jobs only support local runscripts.\n", script.command));
            }
            continue;
        }
        if !runs(script, when, outcome.status) {
            continue;
        }
        codes.status = outcome.status;
        let command = edit_job_codes(&script.command, &codes, "");
        let ok = if script.console {
            post(MsgType::Info, format!("console command: run {label} \"{command}\"\n"));
            run_console_command(host, &command)
        } else {
            let mut ok = true;
            for (msg_type, text) in run_shell_command(&command, label) {
                if msg_type == MsgType::Error {
                    ok = false;
                    outcome.errors += 1;
                }
                post(msg_type, text);
            }
            ok
        };
        if !ok && script.fail_on_error {
            outcome.status = crate::job_manager::merge_status(outcome.status, 'E');
        }
    }
    outcome
}

/// fd_cmds.cc SendRunscriptsCommands(): the "Run ..." commands for the
/// scripts whose target is the job's client, and whether "RunBeforeNow"
/// follows. None when the job has no client scripts (HaveClientRunscripts).
pub fn client_commands(config: &DirectorConfig, job: &ScriptJob) -> Option<(Vec<String>, bool)> {
    let scripts = job_scripts(&config.raw, &job.job_name);
    if scripts.iter().all(|s| s.target.is_empty()) {
        return None;
    }
    let codes = job_codes(config, job);
    let mut commands = Vec::new();
    let mut before = false;
    for script in &scripts {
        if !script.target.is_empty() && edit_job_codes(&script.target, &codes, "") == job.client {
            commands.push(format!("Run OnSuccess={} OnFailure={} AbortOnError={} When={} Command={}\n",
                script.on_success as u8, script.on_failure as u8, script.fail_on_error as u8,
                script_when(script), script.command.replace(' ', "\x01")));
        }
        if script_when(script) & (SCRIPT_BEFORE | SCRIPT_AFTER_VSS) != 0 {
            before = true;
        }
    }
    Some((commands, before))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_codes() {
        let codes = JobCodes { job_id: 7, job: "backup.2026-10-16_10.00.00_07".into(), client: "fd1".into(),
            client_address: "fd.example".into(), level: "Full".into(), job_type: "Backup".into(), files: 3,
            bytes: 1_500_000, status: 'A', director: "dir".into(), fileset: "fs".into(), pool: "Full".into(),
            storage: "File".into(), volume: None };
        assert_eq!(edit_job_codes("%i %j %n %c %h %l %t %e %F %b %B %d %D %f %p %w %x %C %v %s %r %q %% %", &codes, "to"),
            "7 backup.2026-10-16_10.00.00_07 backup fd1 fd.example Full Backup Canceled 3 1500000 1.500 MB dir dir fs \
             Full File no no *None* *None* to %q % %");
        let none = JobCodes::default();
        assert_eq!(edit_job_codes("%c %f %e", &none, ""), "*None* %f Unknown term code");
    }

    #[test]
    fn argv_like_build_argc_argv() {
        assert_eq!(build_argv("  /bin/echo a  'b c' \"d\"e"), vec!["/bin/echo", "a", "b c", "d", "e"]);
        assert_eq!(build_argv("\"/bin/my prog\" x"), vec!["/bin/my prog", "x"]);
        assert!(build_argv("   ").is_empty());
    }

    fn scripts(config: &str) -> Vec<Script> {
        let raw = bareos_config::raw::parse_str(config, None).unwrap();
        job_scripts(&raw, "backup")
    }

    #[test]
    fn after_job_selection_for_canceled_job() {
        let all = scripts(r#"
JobDefs { Name = d; RunAfterFailedJob = "/bin/echo inherited" }
Job { Name = backup; JobDefs = d
  RunAfterJob = "/bin/echo success"
  RunBeforeJob = "/bin/echo before"
  ClientRunAfterJob = "/bin/echo client"
  RunScript { Command = "/bin/echo failure"; RunsWhen = After; RunsOnFailure = yes; RunsOnClient = no }
  RunScript { Command = "/bin/echo always"; RunsWhen = Always; RunsOnSuccess = no; RunsOnFailure = yes; RunsOnClient = no }
  RunScript { Console = "status dir"; RunsWhen = After; RunsOnFailure = yes }
}
"#);
        let commands = |status| selected(&all, "AfterJob", status).iter().map(|s| s.command.clone()).collect::<Vec<_>>();
        assert_eq!(commands('A'), vec!["/bin/echo failure", "/bin/echo always", "status dir", "/bin/echo inherited"]);
        assert_eq!(commands('T'), vec!["/bin/echo success", "/bin/echo failure", "status dir"]);
        assert_eq!(commands('I'), Vec::<String>::new());
        let before: Vec<_> = selected(&all, "BeforeJob", 'R').iter().map(|s| s.command.clone()).collect();
        assert_eq!(before, vec!["/bin/echo before"]);
    }

    #[test]
    fn shell_command_output_and_errors() {
        assert_eq!(run_shell_command("/bin/echo hello   world", "AfterJob"), vec![
            (MsgType::Info, "shell command: run AfterJob \"/bin/echo hello   world\"\n".into()),
            (MsgType::Info, "AfterJob: hello world\n".into())]);
        let long = run_shell_command(&format!("/bin/echo {}", "x".repeat(200)), "AfterJob");
        assert_eq!((long.len(), long[1].1.len(), long[2].1.len()), (3, "AfterJob: \n".len() + 129, "AfterJob: \n".len() + 71));
        assert_eq!(run_shell_command("/bin/sh -c 'exit 3'", "AfterJob")[1],
            (MsgType::Error, "Runscript: AfterJob returned non-zero status=3. ERR=Child exited with code 3\n".into()));
        assert_eq!(run_shell_command("/nonexistent/prog", "AfterJob")[2],
            (MsgType::Error, "Runscript: AfterJob returned non-zero status=255. ERR=Unknown error during program execvp\n".into()));
        assert_eq!(run_shell_command("/bin/sh -c 'kill -TERM $$'", "AfterJob")[1],
            (MsgType::Error, "Runscript: AfterJob returned non-zero status=15. ERR=Child died from signal 15: Termination\n".into()));
        // stderr shares the pipe; a failing execvp prints perror() and exits 255
        assert_eq!(run_shell_command("/bin/sh -c 'echo err >&2'", "BeforeJob")[1],
            (MsgType::Info, "BeforeJob: err\n".into()));
        assert_eq!(run_shell_command("/nonexistent/prog", "BeforeJob")[1],
            (MsgType::Info, "BeforeJob: Program execution failed: No such file or directory\n".into()));
    }

    fn script_job(name: &str, status: char, job_type: WorkerJobType) -> ScriptJob {
        let params = crate::job_worker::JobParams {
            job_id: 5, unique_job: format!("{name}.2026-10-17_06.50.25_03"), job_name: name.into(), job_type,
            level: String::new(), fd_address: String::new(), fd_port: 0, fd_password: String::new(),
            sd_address: String::new(), sd_port: 0, sd_password: String::new(), sd_device: String::new(),
            sd_media_type: String::new(), director_name: "dir".into(), pool_name: "Incremental".into(),
            fileset_includes: Vec::new(), fileset_excludes: Vec::new(), since_utime: 0, prev_job: String::new(),
            where_path: None, regex_where: None, prefix_links: false, replace: None, restore_jobid: None, bootstrap: None,
            client_name: "fd1".into(), copy: None, verify: None, backup: None,
            client_first_connection: false };
        ScriptJob { job_id: 5, job_name: name.into(), unique_job: params.unique_job.clone(), status, files: 0, bytes: 0,
            client: "fd1".into(), storage: "File".into(), params: Some(params) }
    }

    fn host(config: &str) -> Host {
        let config = crate::director_config::DirectorConfig::from_str(config).unwrap();
        Host { config: Some(std::sync::Arc::new(std::sync::RwLock::new(config))), ..Default::default() }
    }

    #[test]
    fn failing_script_changes_the_status_for_the_scripts_after_it() {
        let host = host(r#"
Director { Name = dir; Password = x }
Client { Name = fd1; Address = fd.example; Password = x }
Job { Name = adm; Type = Admin; Client = fd1
  RunBeforeJob = "/bin/sh -c 'exit 2'"
  RunBeforeJob = "/bin/echo second"
  ClientRunBeforeJob = "/bin/echo client"
  RunScript { RunsWhen = Before; RunsOnFailure = yes; RunsOnSuccess = no; RunsOnClient = no; Command = "/bin/echo failed %e" }
}
"#);
        let job = script_job("adm", 'R', WorkerJobType::Admin);
        assert_eq!(run_scripts(&host, None, &job, "BeforeJob"), Outcome { status: 'E', errors: 1 });
        let config = host.config.as_ref().unwrap().read().unwrap();
        assert_eq!(client_commands(&config, &job), Some((vec![
            "Run OnSuccess=1 OnFailure=0 AbortOnError=1 When=2 Command=/bin/echo\x01client\n".to_string()], true)));
    }

    #[test]
    fn console_commands_outside_the_runscript_list_fail() {
        let host = host("Director { Name = dir; Password = x }\n");
        assert!(!run_console_command(&host, "wait"));
        assert!(!run_console_command(&host, "nosuchcommand"));
        assert!(!run_console_command(&host, ""));
    }
}
