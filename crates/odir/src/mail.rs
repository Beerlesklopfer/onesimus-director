// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The `mail`, `mailonerror`, `mailonsuccess` and `operator` message
//! destinations (Bareos 25 lib/message.cc).
//!
//! DispatchMessage() (message.cc:780-808) collects the messages of a mail
//! destination per copy of the Messages resource: one per job (InitJobMsg,
//! message.cc:215) and one for the daemon (`daemon_msgs`, JobId 0).
//! CloseMsg() (message.cc:332-475) pipes everything collected into the
//! MailCommand once, when the job's JCR goes away (lib/jcr.cc:368), after
//! which the daemon's mail is flushed as well (lib/jcr.cc:373); the daemon
//! copy is also flushed when the Director ends (TermMsg, message.cc:487).
//! `mailonerror` is only sent when the job did not end JS_Terminated or
//! JS_Warnings, `mailonsuccess` only when it did (message.cc:382-407).
//! An `operator` message goes to the OperatorCommand at once, one program
//! run per message (message.cc:760-779).
//!
//! Without a MailCommand (OperatorCommand) the program is
//! `/usr/lib/sendmail -F BAREOS <recipients>` with a "Subject: BAREOS
//! Message" header (open_mail_pipe, message.cc:303-318); Bareos has no
//! bsmtp default in the code, only in its sample configuration.
//!
//! Differences to Bareos: the collected messages stay in memory instead of
//! `<WorkingDirectory>/<name>.<job>.<n>.mail` (MakeUniqueMailFilename,
//! message.cc:283), and the programs run on a thread of their own
//! ("dir:mail", in order), so a slow mail program never holds the JobManager
//! or a message lock; [`shutdown`] waits for them.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{Read, Write};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bareos_config::raw::{ConfResource, Tok};

use crate::director_config::SharedConfig;
use crate::job_manager::ScriptJob;
use crate::runscript::JobCodes;

/// Owner of collected mail: the JobId, 0 for the daemon's Messages copy.
/// The second part keeps the daemon copies of parallel tests apart.
type Owner = (u32, u64);

/// A mail destination of one Messages copy (MessageDestinationInfo with
/// its mail file).
struct Mailbox {
    code: &'static str,
    where_: String,
    /// The MailCommand the destination was parsed with (d->mail_cmd_).
    cmd: String,
    text: String,
}

static MAILBOXES: Mutex<BTreeMap<Owner, Vec<Mailbox>>> = Mutex::new(BTreeMap::new());

/// Jobs whose messages were closed (CloseMsg(jcr) deleted jcr_msgs): a late
/// message of such a job is not collected any more.
static CLOSED: Mutex<VecDeque<u32>> = Mutex::new(VecDeque::new());
const MAX_CLOSED: usize = 1024;

/// Job codes of the queued and running jobs for the operator command.
static JOBS: Mutex<Option<HashMap<u32, JobCodes>>> = Mutex::new(None);

/// my_name (%d) for messages without a job.
#[cfg(not(test))]
static DIRECTOR: Mutex<String> = Mutex::new(String::new());
#[cfg(test)]
thread_local! {
    static DIRECTOR: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// OpenBpipe(cmd, 120, "rw") in open_mail_pipe(): the program is killed
/// after 120 seconds (message.cc:314).
const PIPE_WAIT: Duration = Duration::from_secs(120);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Remember the Director's name (my_name) for %d of messages without a job.
pub(crate) fn set_director(name: &str) {
    #[cfg(test)]
    DIRECTOR.with(|d| if *d.borrow() != name { *d.borrow_mut() = name.to_string() });
    #[cfg(not(test))]
    {
        let mut d = lock(&DIRECTOR);
        if *d != name { *d = name.to_string(); }
    }
}

fn director() -> String {
    #[cfg(test)]
    return DIRECTOR.with(|d| d.borrow().clone());
    #[cfg(not(test))]
    lock(&DIRECTOR).clone()
}

fn owner(job_id: Option<u32>) -> Owner {
    match job_id.filter(|id| *id != 0) {
        Some(id) => (id, 0),
        None => (0, daemon_slot()),
    }
}

#[cfg(not(test))]
fn daemon_slot() -> u64 { 0 }

#[cfg(test)]
fn daemon_slot() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut h);
    h.finish() | 1
}

/// lib/res.cc:311-347 StoreMsgs(): a mail or operator destination takes the
/// MailCommand (OperatorCommand for `operator`) the resource has when the
/// destination is first parsed; a later line for the same destination only
/// adds message types (messages_resource.cc:119-139 AddToExistingChain).
/// The destination is the recipient list joined with blanks.
pub(crate) fn command_of(res: &ConfResource, code: &str, where_: &str) -> String {
    let mut mail_cmd = "";
    let mut operator_cmd = "";
    for d in &res.directives {
        match d.norm_key().as_str() {
            "mailcommand" => mail_cmd = d.first().unwrap_or(""),
            "operatorcommand" => operator_cmd = d.first().unwrap_or(""),
            key if key == code => {
                let Some(eq) = d.tokens.iter().position(|t| *t == Tok::Eq) else { continue };
                let dest = d.tokens[..eq].iter().filter_map(Tok::text).collect::<Vec<_>>().join(" ");
                if dest == where_ {
                    return if code == "operator" { operator_cmd } else { mail_cmd }.to_string();
                }
            }
            _ => {}
        }
    }
    String::new()
}

/// lib/util.cc:749 edit_job_codes() with the Director's callback; without a
/// job (jcr NULL) the job values are "*None*". The callback codes %f %h %p
/// %w %x %C would dereference the NULL jcr in Bareos (dird_conf.cc:3099-3123);
/// here they stay as written.
fn edit_codes(cmd: &str, codes: Option<&JobCodes>, to: &str) -> String {
    if let Some(codes) = codes {
        return crate::runscript::edit_job_codes(cmd, codes, to);
    }
    let mut out = String::new();
    let mut chars = cmd.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(code) = chars.next() else {
            out.push('%');
            break;
        };
        match code {
            '%' => out.push('%'),
            'P' => out.push_str(&std::process::id().to_string()),
            'd' | 'D' => out.push_str(&director()),
            'r' => out.push_str(to),
            'B' | 'F' | 'b' | 'c' | 'e' | 'i' | 'j' | 'l' | 'n' | 's' | 't' | 'v' | 'V' | 'O' | 'N' => out.push_str("*None*"),
            other => { out.push('%'); out.push(other); }
        }
    }
    out
}

/// open_mail_pipe() (message.cc:297-326): the program line and what goes
/// first into its stdin.
fn mail_pipe(cmd: &str, codes: Option<&JobCodes>, where_: &str) -> (String, String) {
    if cmd.is_empty() {
        (format!("/usr/lib/sendmail -F BAREOS {where_}"), "Subject: BAREOS Message\r\n\r\n".into())
    } else {
        (edit_codes(cmd, codes, where_), String::new())
    }
}

/// DispatchMessage() kMail/KMailOnError/kMailOnSuccess (message.cc:780-808):
/// append `line` (timestamp and message) to the mail of the destination
/// `code`/`where_` of `res` for the job `job_id` (None or 0: the daemon).
pub(crate) fn collect(res: &ConfResource, code: &'static str, where_: &str, line: &str, job_id: Option<u32>) {
    let owner = owner(job_id);
    if owner.0 != 0 && lock(&CLOSED).contains(&owner.0) {
        return;
    }
    let mut boxes = lock(&MAILBOXES);
    let list = boxes.entry(owner).or_default();
    match list.iter_mut().find(|b| b.code == code && b.where_ == where_) {
        Some(mailbox) => mailbox.text.push_str(line),
        None => list.push(Mailbox { code, where_: where_.to_string(), cmd: command_of(res, code, where_),
            text: line.to_string() }),
    }
}

/// DispatchMessage() kOperator (message.cc:760-779): one OperatorCommand
/// run with `line` (timestamp and message) for the destination `where_`.
pub(crate) fn operator(res: &ConfResource, where_: &str, line: &str, job_id: Option<u32>) {
    let codes = job_id.filter(|id| *id != 0)
        .and_then(|id| lock(&JOBS).as_ref().and_then(|jobs| jobs.get(&id).cloned()));
    let (command, mut input) = mail_pipe(&command_of(res, "operator", where_), codes.as_ref(), where_);
    input.push_str(line);
    submit(Work::Pipe { command, input, kind: Kind::Operator });
}

/// The job codes of a job (%j, %c, %e ... of the commands).
fn codes_of(config: Option<&SharedConfig>, job: &ScriptJob) -> JobCodes {
    config.and_then(|c| c.read().ok()).map(|config| crate::runscript::job_codes(&config, job))
        .unwrap_or_else(|| JobCodes { job_id: job.job_id, job: job.unique_job.clone(), client: job.client.clone(),
            files: job.files, bytes: job.bytes, status: job.status, director: director(), ..Default::default() })
}

/// A job was queued: its codes for operator messages until it ends.
pub(crate) fn register_job(config: Option<&SharedConfig>, job: &ScriptJob) {
    let codes = codes_of(config, job);
    lock(&JOBS).get_or_insert_with(HashMap::new).insert(job.job_id, codes);
}

/// The job ended (lib/jcr.cc:368 CloseMsg(jcr), then :373 CloseMsg(nullptr)):
/// send its collected mail with the job codes of its end, then the daemon's.
pub(crate) fn close_job(config: Option<&SharedConfig>, job: &ScriptJob) {
    let codes = codes_of(config, job);
    set_director(&codes.director);
    {
        let mut closed = lock(&CLOSED);
        closed.push_back(job.job_id);
        while closed.len() > MAX_CLOSED { closed.pop_front(); }
    }
    if let Some(jobs) = lock(&JOBS).as_mut() { jobs.remove(&job.job_id); }
    let boxes = lock(&MAILBOXES).remove(&owner(Some(job.job_id))).unwrap_or_default();
    send(boxes, Some(&codes));
    close_daemon();
}

/// CloseMsg(nullptr): send the mail collected for the daemon (JobId 0).
/// Without a JCR, mailonerror and mailonsuccess are sent as well.
pub(crate) fn close_daemon() {
    let boxes = lock(&MAILBOXES).remove(&owner(None)).unwrap_or_default();
    send(boxes, None);
}

/// TermMsg() at the Director's end (dird.cc:415, message.cc:487): the
/// daemon's mail, then wait until every mail and operator program ran.
pub(crate) fn shutdown() {
    close_daemon();
    drain();
}

/// CloseMsg() (message.cc:366-465) over the mail destinations of one copy.
fn send(boxes: Vec<Mailbox>, codes: Option<&JobCodes>) {
    for mailbox in boxes {
        if let Some(codes) = codes {
            let success = matches!(codes.status, 'T' | 'W');
            match mailbox.code {
                "mailonerror" if success => continue,
                "mailonsuccess" if !success => continue,
                _ => {}
            }
        }
        let (command, mut input) = mail_pipe(&mailbox.cmd, codes, &mailbox.where_);
        input.push_str(&mailbox.text);
        submit(Work::Pipe { command, input, kind: Kind::Mail { job: codes.is_some() } });
    }
}

// --- the mail thread ----------------------------------------------------------

enum Kind {
    /// CloseMsg(): `job` is false for the daemon copy (jcr NULL), whose
    /// program output and exit status are not looked at (message.cc:429-445).
    Mail { job: bool },
    Operator,
}

enum Work {
    Pipe { command: String, input: String, kind: Kind },
    Barrier(mpsc::Sender<()>),
}

static WORKER: OnceLock<Mutex<mpsc::Sender<Work>>> = OnceLock::new();

fn submit(work: Work) {
    let sender = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Work>();
        let spawned = std::thread::Builder::new().name("dir:mail".into()).spawn(move || {
            for work in rx {
                match work {
                    Work::Pipe { command, input, kind } => deliver(&command, input.as_bytes(), &kind),
                    Work::Barrier(done) => { let _ = done.send(()); }
                }
            }
        });
        if let Err(e) = spawned {
            tracing::error!("cannot start the mail thread: {e}");
        }
        Mutex::new(tx)
    });
    if lock(sender).send(work).is_err() {
        delivery_error("mail thread not running\n");
    }
}

/// Wait until the programs submitted so far ran.
pub(crate) fn drain() {
    let (tx, rx) = mpsc::channel();
    submit(Work::Barrier(tx));
    let _ = rx.recv();
}

/// DeliveryError() (message.cc:113-139): "Message delivery ERROR: ..." on
/// stdout and syslog; here in the Director's log.
fn delivery_error(text: &str) {
    tracing::error!("Message delivery ERROR: {}", text.trim_end());
    #[cfg(test)]
    lock(&TEST_ERRORS).push(text.to_string());
}

#[cfg(test)]
static TEST_ERRORS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn errno_text(error: &std::io::Error) -> String {
    let text = error.to_string();
    text.split(" (os error").next().unwrap_or_default().to_string()
}

/// OpenBpipe(cmd, 120, "rw") with stdout and stderr in one pipe, `input` into
/// stdin, then CloseBpipe(): the program's output and, when it failed, the
/// BErrNo text of its status. A program that cannot be executed reports
/// like the child of OpenBpipe() does (perror, exit 200 + errno index).
fn run_pipe(command: &str, input: &[u8]) -> (Vec<u8>, Option<String>) {
    let argv = crate::runscript::build_argv(command);
    let spawned = (|| {
        let (reader, writer) = std::io::pipe()?;
        let stderr = writer.try_clone()?;
        let Some((program, args)) = argv.split_first() else {
            return Ok(Err(std::io::Error::from_raw_os_error(libc::EFAULT)));
        };
        let child = std::process::Command::new(program).args(args).stdin(std::process::Stdio::piped())
            .stdout(writer).stderr(stderr).spawn();
        // The Command and with it the parent's write ends are gone here.
        Ok::<_, std::io::Error>(child.map(|child| (child, reader)))
    })();
    let (mut child, mut reader) = match spawned {
        Ok(Ok(pair)) => pair,
        Ok(Err(error)) | Err(error) => {
            let text = errno_text(&error);
            return (format!("Program execution failed: {text}\n").into_bytes(), Some(text));
        }
    };
    let writer = child.stdin.take().map(|mut stdin| {
        let input = input.to_vec();
        std::thread::spawn(move || { let _ = stdin.write_all(&input); })
    });
    let output = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = reader.read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + PIPE_WAIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                break child.wait();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => break Err(e),
        }
    };
    if let Some(writer) = writer { let _ = writer.join(); }
    let output = output.join().unwrap_or_default();
    use std::os::unix::process::ExitStatusExt;
    let failure = match status {
        Ok(status) => match (status.code(), status.signal()) {
            (Some(0), _) => None,
            (Some(code), _) => Some(crate::runscript::exit_error(code, false).1),
            (None, Some(signal)) => Some(crate::runscript::exit_error(signal, true).1),
            (None, None) => Some(crate::runscript::exit_error(255, false).1),
        },
        Err(e) => Some(errno_text(&e)),
    };
    (output, failure)
}

/// The program run of CloseMsg() (message.cc:409-445) or of the operator
/// destination (message.cc:763-776) and its delivery errors.
fn deliver(command: &str, input: &[u8], kind: &Kind) {
    let (output, failure) = run_pipe(command, input);
    match kind {
        Kind::Mail { job: true } => {
            // "Read what mail prog returned -- should be nothing"
            for line in output.split_inclusive(|b| *b == b'\n') {
                delivery_error(&format!("Mail prog: {}", String::from_utf8_lossy(line)));
            }
            if let Some(err) = failure {
                delivery_error(&format!("Mail program terminated in error.\nCMD={command}\nERR={err}\n"));
            }
        }
        Kind::Mail { job: false } => {}
        Kind::Operator => {
            if let Some(err) = failure {
                delivery_error(&format!(
                    "Msg delivery error: Operator mail program terminated in error.\nCMD={command}\nERR={err}\n"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::director_config::DirectorConfig;
    use crate::message_bus::MsgType;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, RwLock};

    /// A directory under /tmp with a script that appends its arguments and
    /// its stdin to `out` (one record per run).
    fn sandbox(name: &str) -> (PathBuf, PathBuf) {
        let dir = PathBuf::from(format!("/tmp/onesimus-mail-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("catch.sh");
        std::fs::write(&script, format!(
            "#!/bin/sh\n{{ echo \"ARGS:$*\"; echo \"STDIN:\"; cat; echo \"END\"; }} >> {}/out\nexit 0\n", dir.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, script)
    }

    fn shared(text: &str) -> SharedConfig {
        Arc::new(RwLock::new(DirectorConfig::from_str(text).unwrap()))
    }

    fn job(job_id: u32, name: &str, status: char) -> ScriptJob {
        ScriptJob { job_id, job_name: name.into(), unique_job: format!("{name}.2026-11-19_01.00.00_07"), status,
            files: 12, bytes: 3400, client: "pdc-fd".into(), storage: "File".into(), params: None }
    }

    fn records(dir: &Path) -> Vec<String> {
        drain();
        std::fs::read_to_string(dir.join("out")).unwrap_or_default()
            .split("END\n").filter(|r| !r.is_empty()).map(str::to_string).collect()
    }

    fn strip_time(record: &str) -> String {
        // "19-Nov 01:00 " timestamp of TimestampFormat "%d-%b %H:%M"
        record.lines().map(|l| match l.split_once(' ') {
            Some((date, rest)) if date.contains('-') && rest.len() > 6 && rest.as_bytes()[2] == b':' => &rest[6..],
            _ => l,
        }).collect::<Vec<_>>().join("\n")
    }

    fn config(script: &Path, job_messages: &str) -> String {
        format!(r#"
Director {{ Name = mail-dir; Password = secret; Messages = Daemon }}
Messages {{ Name = Daemon
  MailCommand = "{0} -d %d -j %j -r %r"
  Mail = ops@example = all, !skipped
  OperatorCommand = "{0} op %j %d"
  Operator = ops@example = mount
}}
Messages {{ Name = Standard
  MailCommand = "{0} -f \"\(Bareos\) \<%r\>\" -s \"Bareos: %t %e of %c %l\" %r"
  OperatorCommand = "{0} -s \"Intervention needed for %j\" %r"
  {job_messages}
}}
Client {{ Name = pdc-fd; Address = localhost; Password = x }}
Job {{ Name = nightly; Type = Backup; Client = pdc-fd; Messages = Standard; FileSet = fs; Pool = Full }}
"#, script.display())
    }

    fn post(config: &SharedConfig, job_id: u32, msg_type: MsgType, text: &str) {
        let cfg = config.read().unwrap();
        crate::console_msg::job_message(&cfg, "nightly", job_id, msg_type, 0, text);
    }

    #[test]
    fn mail_is_sent_once_at_the_job_end_with_all_messages_in_order() {
        let (dir, script) = sandbox("mail");
        let config = shared(&config(&script, "Mail = root, admin = all, !skipped"));
        let id = 970001;
        post(&config, id, MsgType::Info, "Start Backup JobId 970001\n");
        post(&config, id, MsgType::Skipped, "skipped file\n");
        post(&config, id, MsgType::Warning, "a warning\n");
        assert!(records(&dir).is_empty(), "nothing is sent before the job ends");
        close_job(Some(&config), &job(id, "nightly", 'W'));
        let records = records(&dir);
        assert_eq!(records.len(), 1, "{records:?}");
        // %l of a job without level is " " (job_level_to_str)
        assert_eq!(strip_time(&records[0]), [
            "ARGS:-f (Bareos) <root admin> -s Bareos: Backup OK -- with warnings of pdc-fd   root admin",
            "STDIN:",
            "mail-dir JobId 970001: Start Backup JobId 970001",
            "mail-dir JobId 970001: Warning: a warning",
        ].join("\n"));
        // A late message of the closed job is not collected again.
        post(&config, id, MsgType::Info, "late\n");
        close_job(Some(&config), &job(id, "nightly", 'T'));
        assert_eq!(self::records(&dir).len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mail_on_error_and_mail_on_success_follow_the_end_status() {
        let (dir, script) = sandbox("onerror");
        let config = shared(&config(&script, "MailOnError = err = all\n  MailOnSuccess = ok = all"));
        for (id, status) in [(970011, 'T'), (970012, 'f'), (970013, 'W'), (970014, 'A'), (970015, 'E')] {
            post(&config, id, MsgType::Info, &format!("job {id}\n"));
            close_job(Some(&config), &job(id, "nightly", status));
        }
        let got: Vec<(String, String)> = records(&dir).iter().map(|r| {
            let args = r.lines().next().unwrap().rsplit(' ').next().unwrap().to_string();
            let job = r.lines().find(|l| l.contains("JobId")).unwrap().rsplit(' ').next().unwrap().to_string();
            (args, job)
        }).collect();
        assert_eq!(got, [("ok", "970011"), ("err", "970012"), ("ok", "970013"), ("err", "970014"), ("err", "970015")]
            .map(|(a, b)| (a.to_string(), b.to_string())));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn operator_messages_go_out_one_by_one_with_job_codes() {
        let (dir, script) = sandbox("operator");
        let config = shared(&config(&script, "Operator = root = mount"));
        let id = 970021;
        register_job(Some(&config), &job(id, "nightly", 'R'));
        post(&config, id, MsgType::Info, "not for the operator\n");
        post(&config, id, MsgType::Mount, "Please mount Volume \"Full-0001\"\n");
        post(&config, id, MsgType::Mount, "Please mount Volume \"Full-0002\"\n");
        let records = records(&dir);
        assert_eq!(records.len(), 2, "{records:?}");
        assert_eq!(strip_time(&records[0]), [
            "ARGS:-s Intervention needed for nightly.2026-11-19_01.00.00_07 root",
            "STDIN:",
            "mail-dir JobId 970021: Please mount Volume \"Full-0001\"",
        ].join("\n"));
        assert!(records[1].contains("Full-0002"));
        close_job(Some(&config), &job(id, "nightly", 'T'));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn daemon_messages_without_a_job() {
        let (dir, script) = sandbox("daemon");
        let config = shared(&config(&script, ""));
        let cfg = config.read().unwrap();
        crate::audit::daemon_message(&cfg, MsgType::Error, "reload failed\n");
        crate::audit::daemon_message(&cfg, MsgType::Mount, "mount it\n");
        crate::console_msg::job_message(&cfg, "nightly", 0, MsgType::Warning, 0, "job zero\n");
        drop(cfg);
        let records = records(&dir);
        assert_eq!(records.len(), 1, "operator at once, the mail waits: {records:?}");
        assert_eq!(strip_time(&records[0]), "ARGS:op *None* mail-dir\nSTDIN:\nmail-dir: mount it");
        close_daemon();
        let records = self::records(&dir);
        assert_eq!(records.len(), 2);
        assert_eq!(strip_time(&records[1]), [
            "ARGS:-d mail-dir -j *None* -r ops@example",
            "STDIN:",
            "mail-dir: reload failed",
            "mail-dir: mount it",
            "mail-dir JobId 0: Warning: job zero",
        ].join("\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mail_command_is_the_one_before_the_destination() {
        let config = DirectorConfig::from_str(r#"
Director { Name = d; Password = p; Messages = M }
Messages { Name = M
  Mail = first = all
  MailCommand = "/bin/mail-a %r"
  Mail = second, third = all
  MailCommand = "/bin/mail-b %r"
  Mail = first = error
  OperatorCommand = "/bin/op"
  Operator = op = mount
}
"#).unwrap();
        let res = crate::audit::messages_resource(&config, "M").unwrap();
        assert_eq!(command_of(res, "mail", "first"), "");
        assert_eq!(command_of(res, "mail", "second third"), "/bin/mail-a %r");
        assert_eq!(command_of(res, "operator", "op"), "/bin/op");
        assert_eq!(mail_pipe("", None, "a b"),
            ("/usr/lib/sendmail -F BAREOS a b".to_string(), "Subject: BAREOS Message\r\n\r\n".to_string()));
    }

    #[test]
    fn failing_mail_program_is_a_delivery_error() {
        let (dir, _) = sandbox("failing");
        let failing = dir.join("fail.sh");
        std::fs::write(&failing, "#!/bin/sh\necho complaint\nexit 3\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&failing, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = shared(&config(&failing, "Mail = root = all"));
        post(&config, 970031, MsgType::Info, "x\n");
        close_job(Some(&config), &job(970031, "nightly", 'T'));
        drain();
        let errors: Vec<String> = lock(&TEST_ERRORS).iter().filter(|e| e.contains(&*failing.to_string_lossy())
            || e.starts_with("Mail prog: complaint")).cloned().collect();
        assert!(errors.iter().any(|e| e == "Mail prog: complaint\n"), "{errors:?}");
        assert!(errors.iter().any(|e| e.starts_with("Mail program terminated in error.\nCMD=")
            && e.ends_with("ERR=Child exited with code 3\n")), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn codes_without_a_job() {
        set_director("nojob-dir");
        assert_eq!(edit_codes("%d %r %j %c %e %i %% %f %", None, "to"), "nojob-dir to *None* *None* *None* *None* % %f %");
    }
}
