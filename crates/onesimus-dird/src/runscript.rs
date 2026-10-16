//! Director-side RunScripts: lib/runscript.cc RunScripts() and
//! RunScript::Run(), lib/util.cc edit_job_codes() with
//! dird_conf.cc job_code_callback_director(), lib/bpipe_unix.cc
//! OpenBpipe()/CloseBpipe() and lib/berrno.cc. Used for the "AfterJob"
//! scripts `cancel` runs (job.cc:741). Compatibility log C-048.

use std::io::{BufRead, BufReader};

use bareos_config::raw::ConfResource;

use crate::commands::show::{runscripts, Script};
use crate::director_config::SharedConfig;
use crate::job_manager::ScriptJob;
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
            's' | 'v' | 'V' | 'O' | 'N' => NONE.into(),
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

/// RunScripts(): the scripts `label` runs for a job in `status` (local
/// scripts only; console commands are not supported by Onesimus).
pub fn selected<'a>(scripts: &'a [Script], label: &str, status: char) -> Vec<&'a Script> {
    const AFTER: u8 = 1;
    const BEFORE: u8 = 2;
    const AFTER_VSS: u8 = 4;
    let when = if label.contains("Before") { BEFORE } else if label == "ClientAfterVSS" { AFTER_VSS } else { AFTER };
    let canceled = crate::job_manager::is_job_canceled(status);
    scripts.iter().filter(|script| {
        if !script.target.is_empty() || script.console {
            return false;
        }
        let script_when = match script.when {
            Some("before") => BEFORE,
            Some("after") => AFTER,
            Some("aftervss") => AFTER_VSS,
            Some("always") => BEFORE | AFTER,
            _ => 0,
        };
        let on = |cond_success: bool| (script.on_success && cond_success) || (script.on_failure && (canceled || status == 'D'));
        (script_when & BEFORE != 0 && when & BEFORE != 0 && on(matches!(status, 'R' | 'C')))
            || (script_when & AFTER_VSS != 0 && when & AFTER_VSS != 0
                && ((script.on_success && status == 'B') || (script.on_failure && canceled)))
            || (script_when & AFTER != 0 && when & AFTER != 0 && on(matches!(status, 'T' | 'W')))
    }).collect()
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
pub fn run_shell_command(command: &str, name: &str) -> Vec<(MsgType, String)> {
    let mut messages = vec![(MsgType::Info, format!("shell command: run {name} \"{command}\"\n"))];
    let argv = build_argv(command);
    let status = match argv.split_first() {
        // execvp(NULL, ...) fails with EFAULT
        None => Err(202),
        Some((program, args)) => match std::process::Command::new(program).args(args)
            .stdout(std::process::Stdio::piped()).spawn() {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() {
                    let mut reader = BufReader::new(stdout);
                    let mut line = Vec::new();
                    // fgets() into a PM_NAME buffer (130 bytes).
                    while reader.read_until(b'\n', &mut line).is_ok_and(|n| n > 0) {
                        for chunk in line.chunks(129) {
                            let mut chunk = chunk;
                            while let [rest @ .., b' ' | b'\n' | b'\r'] = chunk { chunk = rest; }
                            messages.push((MsgType::Info, format!("{name}: {}\n", String::from_utf8_lossy(chunk))));
                        }
                        line.clear();
                    }
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
            Err(error) => Err(error.raw_os_error()
                .and_then(|errno| EXECVP_ERRORS.iter().position(|e| *e == errno))
                .map_or(255, |index| 200 + index as i32)),
        },
    };
    if let Err(code) = status {
        let (code, text) = if code < 0 { exit_error(-code, true) } else { exit_error(code, false) };
        messages.push((MsgType::Error, format!("Runscript: {name} returned non-zero status={code}. ERR={text}\n")));
    }
    messages
}

/// RunScripts(jcr, job->RunScripts, label) for a job of the JobManager:
/// runs the selected scripts and posts their messages as job messages.
pub fn run_job_scripts(config: Option<&SharedConfig>, bus: Option<&MessageBus>, job: &ScriptJob, label: &str)
    -> Vec<(MsgType, String)> {
    let Some((scripts, codes)) = config.and_then(|c| c.read().ok()).map(|config| {
        let resource = config.find_job(&job.job_name);
        let client = config.find_client(&job.client);
        let codes = JobCodes {
            job_id: job.job_id,
            job: job.unique_job.clone(),
            client: job.client.clone(),
            client_address: client.map(|c| c.address.clone()).unwrap_or_default(),
            level: job.params.as_ref().map(|p| p.level.clone()).unwrap_or_default(),
            job_type: match job.params.as_ref().map(|p| p.job_type) {
                Some(crate::job_worker::WorkerJobType::Restore) => "Restore".into(),
                _ => "Backup".into(),
            },
            files: job.files,
            bytes: job.bytes,
            status: job.status,
            director: config.director.name.clone(),
            fileset: resource.map(|r| r.fileset.clone()).unwrap_or_default(),
            pool: job.params.as_ref().map(|p| p.pool_name.clone()).unwrap_or_default(),
            storage: job.storage.clone(),
        };
        (job_scripts(&config.raw, &job.job_name), codes)
    }) else { return Vec::new() };
    let mut all = Vec::new();
    for script in selected(&scripts, label, job.status) {
        let command = edit_job_codes(&script.command, &codes, "");
        for (msg_type, text) in run_shell_command(&command, label) {
            if let Some(bus) = bus {
                crate::console_msg::post_job_message(bus, config, BusMessage::JobMessage {
                    job_id: job.job_id, job_name: job.job_name.clone(), msg_type, text: text.clone(),
                    timestamp: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default().as_secs() as i64,
                });
            }
            all.push((msg_type, text));
        }
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_codes() {
        let codes = JobCodes { job_id: 7, job: "backup.2026-10-16_10.00.00_07".into(), client: "fd1".into(),
            client_address: "fd.example".into(), level: "Full".into(), job_type: "Backup".into(), files: 3,
            bytes: 1_500_000, status: 'A', director: "dir".into(), fileset: "fs".into(), pool: "Full".into(),
            storage: "File".into() };
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
        assert_eq!(commands('A'), vec!["/bin/echo failure", "/bin/echo always", "/bin/echo inherited"]);
        assert_eq!(commands('T'), vec!["/bin/echo success", "/bin/echo failure"]);
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
        assert_eq!(run_shell_command("/nonexistent/prog", "AfterJob")[1],
            (MsgType::Error, "Runscript: AfterJob returned non-zero status=208. ERR=No such file or directory\n".into()));
        assert_eq!(run_shell_command("/bin/sh -c 'kill -TERM $$'", "AfterJob")[1],
            (MsgType::Error, "Runscript: AfterJob returned non-zero status=15. ERR=Child died from signal 15: Termination\n".into()));
    }
}
