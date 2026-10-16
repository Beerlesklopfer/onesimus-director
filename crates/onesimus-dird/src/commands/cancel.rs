//! `cancel` — C++ ua_cmds.cc CancelCmd (:803) with CancelStorageDaemonJob
//! (:758), CancelJobs (:779), ua_select.cc select_jobs (:1566), job.cc
//! CancelJob (:673), fd_cmds.cc CancelFileDaemonJob (:976) and sd_cmds.cc
//! CancelStorageDaemonJob (:590, :620). Compatibility log C-034.

use bareos_bnet::{BnetMessage, BnetSignal};
use bareos_config::resources::{ClientResource, StorageResource};

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::job_manager::ActiveJob;
use crate::message_bus::BusMessage;
use crate::ua_select::{self, Ua};
use crate::ua_storage_select::{NetSd, SdLink};

/// The daemon connections `cancel` opens (fakes in the unit tests).
pub trait CancelLinks {
    /// ConnectToFileDaemon with a UaContext, one command and every reply up
    /// to the first signal. The text is what the connection attempt prints
    /// (SendRawMsg) and is returned even when it failed.
    fn file_daemon(&mut self, client: &ClientResource, director: &str, command: &str)
        -> (String, Result<Vec<String>, String>);
    /// ConnectToStorageDaemon, one command and every reply up to the first signal.
    fn storage_daemon(&mut self, store: &StorageResource, director: &str, command: &str)
        -> Result<Vec<String>, String>;
}

/// The real daemon connections.
pub struct NetLinks;

impl CancelLinks for NetLinks {
    fn file_daemon(&mut self, client: &ClientResource, director: &str, command: &str)
        -> (String, Result<Vec<String>, String>) {
        let (info, fd) = crate::fd_protocol::connect_fd_console(
            &client.address, client.port, &client.password, director, &client.name);
        let reply = fd.map(|mut fd| {
            let mut messages = Vec::new();
            // `while (fd->recv() >= 0)`: data frames up to the first signal.
            if fd.send_str(command).is_ok() {
                loop {
                    match fd.recv() {
                        Ok(BnetMessage::Data(data)) => messages.push(String::from_utf8_lossy(&data).into_owned()),
                        Ok(BnetMessage::Signal(BnetSignal::Heartbeat)) => {
                            let _ = fd.send_signal(BnetSignal::HeartbeatResponse);
                        }
                        Ok(BnetMessage::Signal(_)) | Err(_) => break,
                    }
                }
            }
            let _ = fd.send_signal(BnetSignal::Terminate);
            messages
        });
        (info, reply)
    }

    fn storage_daemon(&mut self, store: &StorageResource, director: &str, command: &str)
        -> Result<Vec<String>, String> {
        NetSd { director: director.to_string() }.exchange(store, command)
    }
}

pub fn cmd_cancel(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    cancel_with(ast, ctx, &mut NetLinks)
}

/// CancelCmd(): `storage=<name>` cancels on the Storage daemon only,
/// everything else selects running jobs.
pub fn cancel_with(ast: &CommandAst, ctx: &ExecContext, links: &mut dyn CancelLinks) -> CommandResult {
    let mut ua = Ua::new(ctx);
    if ua_select::find_arg_with_value(ast, "storage").is_some() {
        cancel_storage_daemon_job(&mut ua, ast, links);
    } else {
        let active = ctx.job_manager.as_ref()
            .map(|mgr| mgr.lock().unwrap().active_jobs()).unwrap_or_default();
        if let Some(selection) = select_jobs(&mut ua, ast, &active, "cancel") {
            for id in selection {
                // get_jcr_by_id(): the job may have ended meanwhile.
                if let Some(job) = active.iter().find(|job| job.id == id) {
                    cancel_job(&mut ua, job, links);
                }
            }
        }
    }
    ua.finish(true)
}

fn director_name(ctx: &ExecContext) -> String {
    ctx.config.as_ref().and_then(|c| c.read().ok().map(|c| c.director.name.clone())).unwrap_or_default()
}

/// ua_cmds.cc CancelStorageDaemonJob(ua, cmd) (:758).
fn cancel_storage_daemon_job(ua: &mut Ua, ast: &CommandAst, links: &mut dyn CancelLinks) {
    let ctx = ua.ctx;
    let Some(store) = ua_select::get_storage_resource(ua, ast, false, false) else { return };
    let Some(jobid) = ua_select::find_arg_with_value(ast, "jobid") else {
        ua.warning_msg("Missing jobid=JobId specification\n");
        return;
    };
    if !ua_select::is_a_number(jobid) {
        ua.warning_msg(format!("JobId {jobid} not a number\n"));
    }
    // sd_cmds.cc CancelStorageDaemonJob(ua, store, JobId) (:590).
    match links.storage_daemon(&store, &director_name(ctx), &format!("cancel Job={jobid}\n")) {
        Ok(messages) => messages.into_iter().for_each(|message| ua.send_msg(message)),
        Err(_) => ua.error_msg("Failed to connect to Storage daemon.\n"),
    }
}

/// ua_select.cc select_jobs() (:1566) over the jobs the JobManager runs;
/// None on bail_out.
pub fn select_jobs(ua: &mut Ua, ast: &CommandAst, active: &[ActiveJob], reason: &str) -> Option<Vec<u32>> {
    let ctx = ua.ctx;
    let mut selected: Vec<u32> = Vec::new();
    let select_all = ua_select::find_arg(ast, "all").is_some();
    let state = ua_select::find_arg_with_value(ast, "state");

    // FindArgKeyword(ua, {"job", "jobid", "ujobid"}) > 0: the first of these
    // keywords decides, and "job" (index 0) skips the explicit selection.
    let keyword = ast.arguments.iter().find_map(|arg| ["job", "jobid", "ujobid"].iter()
        .position(|key| arg.key.eq_ignore_ascii_case(key)));
    if keyword.is_some_and(|index| index > 0) {
        for arg in &ast.arguments {
            let value = arg.value.as_deref();
            let job = if arg.key.eq_ignore_ascii_case("jobid") {
                let id = crate::commands::job::str_to_int64(value.unwrap_or(""));
                if id == 0 { continue; }
                match active.iter().find(|job| i64::from(job.id) == id) {
                    Some(job) => job,
                    None => {
                        ua.error_msg(format!("JobId {} is not running. Use Job name to {reason} inactive jobs.\n",
                            value.unwrap_or("")));
                        continue;
                    }
                }
            } else if arg.key.eq_ignore_ascii_case("job") || arg.key.eq_ignore_ascii_case("ujobid") {
                let Some(name) = value else { continue };
                // get_jcr_by_partial_name (job=) / get_jcr_by_full_name (ujobid=)
                let partial = arg.key.eq_ignore_ascii_case("job");
                match active.iter().find(|job| if partial { job.unique_name.starts_with(name) } else { job.unique_name == name }) {
                    Some(job) => job,
                    None => {
                        ua.warning_msg(format!("Warning Job {name} is not running. Continuing anyway ...\n"));
                        continue;
                    }
                }
            } else {
                continue;
            };
            if !crate::acl::access_audited(ctx, crate::acl::Kind::Job, &job.name) {
                ua.error_msg("Unauthorized command from this console.\n");
                return None;
            }
            if !selected.contains(&job.id) { selected.push(job.id); }
        }
    }
    if !selected.is_empty() {
        return Some(selected);
    }

    let authorized: Vec<&ActiveJob> = active.iter()
        .filter(|job| crate::acl::access(ctx, crate::acl::Kind::Job, &job.name)).collect();
    if authorized.is_empty() {
        ua.send_msg(if active.is_empty() { "No Jobs running.\n" } else { "None of your jobs are running.\n" });
        return None;
    }

    if select_all || state.is_some() {
        let wanted: fn(&ActiveJob) -> bool = if select_all {
            |_| true
        } else {
            match state.map(str::to_ascii_lowercase).as_deref() {
                Some("created") => |job| job.status == 'C',
                Some("blocked") => |job| job.job_started && job.status == 'B',
                Some("waiting") => ActiveJob::waiting,
                Some("running") => |job| job.job_started && job.status == 'R',
                _ => {
                    ua.error_msg("Illegal state either created, blocked, waiting or running\n");
                    return None;
                }
            }
        };
        for job in authorized.iter().filter(|job| wanted(job)) {
            if !selected.contains(&job.id) {
                selected.push(job.id);
            }
            ua.send_msg(format!("Selected Job {} for cancelling\n", job.id));
        }
        if selected.is_empty() {
            ua.send_msg("No Jobs selected.\n");
            return None;
        }
        // Only ask when not in batch mode and without "yes" on the cmdline.
        if !ua.batch() && ua_select::find_arg(ast, "yes").is_none()
            && ua.get_yesno("Confirm cancel (yes/no): ") != Some(true) {
            return None;
        }
        return Some(selected);
    }

    ua.start_prompt("Select Job:\n");
    for job in &authorized {
        ua.add_prompt(&format!("JobId={} Job={}", job.id, job.unique_name));
    }
    let (_, choice) = ua.do_prompt("Job", &format!("Choose Job to {reason}"))?;
    if reason == "cancel" && authorized.len() == 1 {
        let question = if ua.api() {
            format!("Cancel: {choice}\n\nConfirm cancel?")
        } else {
            "Confirm cancel (yes/no): ".to_string()
        };
        if ua.get_yesno(&question) != Some(true) {
            return None;
        }
    }
    // sscanf(buf, "JobId=%d Job=%127s") and get_jcr_by_full_name(JobName).
    let name: String = choice.split_once(" Job=").map(|(_, rest)| rest).unwrap_or("")
        .split_whitespace().next().unwrap_or("").chars().take(127).collect();
    match active.iter().find(|job| job.unique_name == name) {
        Some(job) => Some(vec![job.id]),
        None => {
            ua.warning_msg(format!("Job \"{name}\" not found.\n"));
            None
        }
    }
}

/// job.cc CancelJob() (:673): a job still in the queue is only marked and
/// leaves the queue (JobqRemove), a started job is canceled on the File and
/// Storage daemon it is connected to. A forked worker is signalled in every
/// case (C++ setJobStatus(JS_Canceled) plus the thread signal); the job's own
/// report follows when it has ended. When the daemons took the cancel, the
/// job's RunScripts "AfterJob" run (job.cc:741). Compatibility log C-034, C-048.
fn cancel_job(ua: &mut Ua, job: &ActiveJob, links: &mut dyn CancelLinks) {
    let ctx = ua.ctx;
    let mut ok = true;
    if crate::job_manager::is_queued_status(job.status) {
        ua.info_msg(format!("JobId {}, Job {} marked to be canceled.\n", job.id, job.unique_name));
    } else if job.fd_connected || job.sd_connected {
        // jcr->file_bsock / jcr->store_bsock
        let (client, store) = job_daemons(ctx, job);
        let director = director_name(ctx);
        let command = format!("cancel Job={}\n", job.unique_name);
        if let (true, Some(client)) = (job.fd_connected, client) {
            // fd_cmds.cc CancelFileDaemonJob (:976)
            let (info, reply) = links.file_daemon(&client, &director, &command);
            ua.send_msg(info);
            match reply {
                Ok(messages) => messages.into_iter().for_each(|message| ua.send_msg(message)),
                Err(_) => {
                    ua.error_msg("\nFailed to connect to File daemon.\n");
                    ok = false;
                }
            }
        }
        if let (true, true, Some(store)) = (ok, job.sd_connected, store) {
            // sd_cmds.cc CancelStorageDaemonJob(ua, jcr, interactive) (:620)
            match links.storage_daemon(&store, &director, &command) {
                Ok(messages) => messages.into_iter().for_each(|message| ua.send_msg(message)),
                Err(_) => {
                    ua.error_msg("Failed to connect to Storage daemon.\n");
                    ok = false;
                }
            }
        }
    }
    let canceled = ctx.job_manager.as_ref().is_some_and(|mgr| mgr.lock().unwrap().cancel_job(job.id));
    if canceled {
        if let Some(smgr) = &ctx.session_mgr {
            smgr.bus().post(BusMessage::JobStatus {
                job_id: job.id, job_name: job.name.clone(), status: "Canceling".to_string(),
                timestamp: now_unix(),
            });
        }
    }
    if ok {
        let script_job = ctx.job_manager.as_ref().and_then(|mgr| mgr.lock().unwrap().take_post_scripts(job.id));
        if let Some(script_job) = script_job {
            crate::runscript::run_job_scripts(ctx.config.as_ref(), ctx.session_mgr.as_ref().map(|s| s.bus()),
                &script_job, "AfterJob");
        }
    }
}

/// The Client and Storage resources of a running job: recorded at start,
/// else those of its Job resource.
fn job_daemons(ctx: &ExecContext, job: &ActiveJob) -> (Option<ClientResource>, Option<StorageResource>) {
    let recorded = ctx.job_manager.as_ref().and_then(|mgr| mgr.lock().unwrap().job_resources(job.id));
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else { return (None, None) };
    let (client, storage) = recorded.unwrap_or_else(|| config.find_job(&job.name)
        .map(|res| (res.client.clone(), res.storage.clone())).unwrap_or_default());
    (config.find_client(&client).cloned(), config.find_storage(&storage).cloned())
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_executor::ApiMode;
    use crate::job_manager::{DirJobPhase, JobManager};
    use crate::ua_select::tests::{context, scripted};
    use std::sync::{Arc, Mutex};

    const CONFIG: &str = r#"
Director { Name = dir; Password = secret }
Client { Name = fd1; Address = fd.example; Password = x }
Storage { Name = File; Address = sd.example; Password = y; Device = FileStorage; Media Type = File }
Job { Name = backup; Client = fd1; Storage = File }
Job { Name = other; Client = fd1; Storage = File }
"#;

    /// Fake daemons: records (daemon, command) and answers like bareos-fd/-sd.
    #[derive(Default)]
    struct Fake {
        calls: Vec<(&'static str, String)>,
        fd_down: bool,
        sd_down: bool,
    }

    impl CancelLinks for Fake {
        fn file_daemon(&mut self, client: &ClientResource, _: &str, command: &str)
            -> (String, Result<Vec<String>, String>) {
            self.calls.push(("fd", format!("{}:{command}", client.name)));
            if self.fd_down { return ("Probing client protocol...".into(), Err("connect: refused".into())); }
            let job = command.trim_end().strip_prefix("cancel Job=").unwrap();
            (" Handshake: Immediate TLS,\n".into(), Ok(vec![format!("2001 Job {job} marked to be canceled.\n")]))
        }
        fn storage_daemon(&mut self, store: &StorageResource, _: &str, command: &str) -> Result<Vec<String>, String> {
            self.calls.push(("sd", format!("{}:{command}", store.name)));
            if self.sd_down { return Err("connect: refused".into()); }
            let job = command.trim_end().strip_prefix("cancel Job=").unwrap();
            // Unique job names (with a dot) run on this fake SD.
            Ok(vec![if job.contains('.') {
                format!("3000 JobId=1 Job=\"{job}\" marked to be canceled.\n")
            } else {
                format!("3904 Job {job} not found.\n")
            }])
        }
    }

    /// A worker stand-in: a child process the cancel must SIGTERM.
    struct Worker(std::process::Child);
    impl Worker {
        fn spawn() -> Self { Self(std::process::Command::new("sleep").arg("30").spawn().unwrap()) }
        fn terminated(&mut self) -> bool {
            for _ in 0..300 {
                if self.0.try_wait().unwrap().is_some() { return true; }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            false
        }
    }
    impl Drop for Worker {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }

    fn ctx_with(api: ApiMode, jobs: &[(u32, &str, DirJobPhase, &Worker)]) -> ExecContext {
        let mut ctx = context(CONFIG, api);
        let mut manager = JobManager::new(Arc::new(crate::message_bus::MessageBus::new(10)), None, None);
        for (id, name, phase, worker) in jobs {
            let unique = format!("{name}.2026-10-16_10.00.00_0{id}");
            manager.add_job(*id, name, crate::job_worker::JobHandle::detached(worker.0.id() as i32, *id, name, &unique));
            manager.set_phase(*id, *phase);
        }
        ctx.job_manager = Some(Arc::new(Mutex::new(manager)));
        ctx
    }

    fn phase(ctx: &ExecContext, id: u32) -> DirJobPhase {
        ctx.job_manager.as_ref().unwrap().lock().unwrap().get_job(id).unwrap().phase
    }

    fn run(line: &str, ctx: &ExecContext, fake: &mut Fake, answers: &[&str]) -> (CommandResult, String) {
        let (result, console) = scripted(answers, || cancel_with(&crate::session::parse_cli_command(line), ctx, fake));
        (result, console.transcript())
    }

    #[test]
    fn no_running_jobs() {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut fake = Fake::default();
        let (result, _) = run("cancel jobid=999999", &ctx, &mut fake, &[]);
        assert_eq!((result.error_message.as_str(), result.text_output.as_str()),
            ("JobId 999999 is not running. Use Job name to cancel inactive jobs.\n", "No Jobs running.\n"));
        assert!(result.success && result.errors_first);
        // "job" as first keyword skips the explicit selection; jobid=abc is ignored.
        for line in ["cancel", "cancel job=nosuch", "cancel jobid=abc", "cancel all", "cancel all yes",
            "cancel state=idle", "cancel state=running", "cancel storage"] {
            let (result, _) = run(line, &ctx, &mut fake, &[]);
            assert_eq!((result.error_message.as_str(), result.text_output.as_str()), ("", "No Jobs running.\n"), "{line}");
        }
        assert_eq!(run("cancel ujobid=nosuch", &ctx, &mut fake, &[]).0.text_output,
            "Warning Job nosuch is not running. Continuing anyway ...\nNo Jobs running.\n");
        assert!(fake.calls.is_empty());
        use crate::commands::job::str_to_int64;
        assert_eq!((str_to_int64(" 12abc"), str_to_int64("-3"), str_to_int64("abc")), (12, -3, 0));
    }

    #[test]
    fn storage_daemon_cancel() {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut fake = Fake::default();
        let (result, _) = run("cancel storage=File jobid=999999", &ctx, &mut fake, &[]);
        assert_eq!(result.text_output, "3904 Job 999999 not found.\n");
        let (result, _) = run("cancel storage=File jobid=abc", &ctx, &mut fake, &[]);
        assert_eq!(result.text_output, "JobId abc not a number\n3904 Job abc not found.\n");
        for line in ["cancel storage=File", "cancel storage=File jobid"] {
            let (result, _) = run(line, &ctx, &mut fake, &[]);
            assert_eq!(result.text_output, "Missing jobid=JobId specification\n", "{line}");
        }
        let (result, _) = run("cancel storage=nosuch jobid=7", &ctx, &mut fake, &[]);
        assert_eq!((result.error_message.as_str(), result.text_output.as_str()),
            ("Storage resource \"nosuch\": not found\n", "Automatically selected Storage: File\n3904 Job 7 not found.\n"));
        let (result, _) = run("cancel jobid=7 storage=File", &ctx, &mut fake, &[]);
        assert_eq!(result.error_message, "JobId 7 is not running.\n");
        assert_eq!(fake.calls, vec![("sd", "File:cancel Job=999999\n".into()), ("sd", "File:cancel Job=abc\n".into()),
            ("sd", "File:cancel Job=7\n".into())]);
        fake.sd_down = true;
        let (result, _) = run("cancel storage=File jobid=1", &ctx, &mut fake, &[]);
        assert_eq!((result.error_message.as_str(), result.text_output.as_str()), ("Failed to connect to Storage daemon.\n", ""));
    }

    #[test]
    fn running_job_is_canceled_on_fd_and_sd_and_worker_ends() {
        let mut worker = Worker::spawn();
        let ctx = ctx_with(ApiMode::Human, &[(5, "backup", DirJobPhase::Running, &worker)]);
        let mut fake = Fake::default();
        let (result, _) = run("cancel jobid=5", &ctx, &mut fake, &[]);
        let unique = "backup.2026-10-16_10.00.00_05";
        assert_eq!(result.text_output, format!(" Handshake: Immediate TLS,\n2001 Job {unique} marked to be canceled.\n\
            3000 JobId=1 Job=\"{unique}\" marked to be canceled.\n"));
        assert_eq!(fake.calls, vec![("fd", format!("fd1:cancel Job={unique}\n")), ("sd", format!("File:cancel Job={unique}\n"))]);
        assert_eq!(phase(&ctx, 5), DirJobPhase::Canceling);
        assert!(worker.terminated(), "worker got SIGTERM");
    }

    #[test]
    fn recorded_resources_and_fd_failure() {
        let worker = Worker::spawn();
        let ctx = context(&format!("{CONFIG}Client {{ Name = fd2; Address = fd2.example; Password = z }}\n"), ApiMode::Human);
        let mut manager = JobManager::new(Arc::new(crate::message_bus::MessageBus::new(10)), None, None);
        manager.add_job(6, "backup", crate::job_worker::JobHandle::detached(worker.0.id() as i32, 6, "backup", "backup.x"));
        manager.set_phase(6, DirJobPhase::WaitingMount);
        manager.set_job_resources(6, "fd2", "File");
        let mut ctx = ctx;
        ctx.job_manager = Some(Arc::new(Mutex::new(manager)));
        let mut fake = Fake { fd_down: true, ..Default::default() };
        let (result, _) = run("cancel ujobid=backup.x", &ctx, &mut fake, &[]);
        // No SD cancel after a failed FD connection (CancelJob returns false).
        assert_eq!(fake.calls, vec![("fd", "fd2:cancel Job=backup.x\n".into())]);
        assert_eq!((result.text_output.as_str(), result.error_message.as_str()),
            ("Probing client protocol...", "\nFailed to connect to File daemon.\n"));
        assert_eq!(phase(&ctx, 6), DirJobPhase::Canceling);
    }

    #[test]
    fn queued_job_is_only_marked() {
        let mut worker = Worker::spawn();
        let ctx = ctx_with(ApiMode::Human, &[(7, "backup", DirJobPhase::Starting, &worker)]);
        let mut fake = Fake::default();
        let (result, _) = run("cancel job=backup", &ctx, &mut fake, &["yes"]);
        assert!(fake.calls.is_empty());
        // job= as first keyword: interactive selection of the only job.
        assert!(result.text_output.is_empty());
        assert!(worker.terminated());
    }

    #[test]
    fn single_job_dialog_declined_and_confirmed() {
        let worker = Worker::spawn();
        let ctx = ctx_with(ApiMode::Human, &[(7, "backup", DirJobPhase::Starting, &worker)]);
        let mut fake = Fake::default();
        let (_, transcript) = run("cancel", &ctx, &mut fake, &["no"]);
        assert_eq!(transcript, "Automatically selected Job: JobId=7 Job=backup.2026-10-16_10.00.00_07\n\
            Confirm cancel (yes/no): no\n");
        assert_eq!(phase(&ctx, 7), DirJobPhase::Starting);
        let (_, transcript) = run("cancel", &ctx, &mut fake, &["yes"]);
        assert_eq!(transcript, "Automatically selected Job: JobId=7 Job=backup.2026-10-16_10.00.00_07\n\
            Confirm cancel (yes/no): yes\nJobId 7, Job backup.2026-10-16_10.00.00_07 marked to be canceled.\n");
        assert_eq!(phase(&ctx, 7), DirJobPhase::Canceling);
        assert!(fake.calls.is_empty());
    }

    #[test]
    fn single_job_dialog_api() {
        let worker = Worker::spawn();
        let ctx = ctx_with(ApiMode::Api, &[(7, "backup", DirJobPhase::Running, &worker)]);
        let mut fake = Fake::default();
        let (_, transcript) = run("cancel", &ctx, &mut fake, &["no"]);
        assert_eq!(transcript, "Cancel: JobId=7 Job=backup.2026-10-16_10.00.00_07\n\nConfirm cancel?no\n");
        assert_eq!(phase(&ctx, 7), DirJobPhase::Running);
    }

    #[test]
    fn several_jobs_menu_without_confirmation() {
        let (w1, w2) = (Worker::spawn(), Worker::spawn());
        let ctx = ctx_with(ApiMode::Human, &[(7, "backup", DirJobPhase::Running, &w1), (8, "other", DirJobPhase::Running, &w2)]);
        let mut fake = Fake::default();
        let (_, transcript) = run("cancel", &ctx, &mut fake, &["."]);
        assert!(transcript.starts_with("Select Job:\n"), "{transcript}");
        assert!(transcript.contains("Choose Job to cancel (1-2): ."), "{transcript}");
        assert!(fake.calls.is_empty());
        let (_, transcript) = run("cancel", &ctx, &mut fake, &["2"]);
        assert!(!transcript.contains("Confirm"), "{transcript}");
        assert_eq!(fake.calls.len(), 2);
        assert!(fake.calls[0].1.ends_with("other.2026-10-16_10.00.00_08\n"));
        assert_eq!((phase(&ctx, 7), phase(&ctx, 8)), (DirJobPhase::Running, DirJobPhase::Canceling));
    }

    #[test]
    fn state_and_all_selection() {
        let (w1, w2, w3) = (Worker::spawn(), Worker::spawn(), Worker::spawn());
        let ctx = ctx_with(ApiMode::Human, &[(7, "backup", DirJobPhase::Starting, &w1),
            (8, "backup", DirJobPhase::WaitingMedia, &w2), (9, "other", DirJobPhase::Paused, &w3)]);
        let mut fake = Fake::default();
        let (result, _) = run("cancel state=idle", &ctx, &mut fake, &[]);
        assert_eq!(result.error_message, "Illegal state either created, blocked, waiting or running\n");
        // A job waiting for media keeps JobStatus R (the wait is the SD's).
        let (_, transcript) = run("cancel state=running", &ctx, &mut fake, &["no"]);
        assert_eq!(transcript, "Selected Job 8 for cancelling\nConfirm cancel (yes/no): no\n");
        // Declined confirmation: nothing canceled.
        let (_, transcript) = run("cancel state=WAITING", &ctx, &mut fake, &["no"]);
        assert_eq!(transcript, "Selected Job 8 for cancelling\nConfirm cancel (yes/no): no\n");
        let (_, transcript) = run("cancel state=blocked", &ctx, &mut fake, &["yes"]);
        assert!(transcript.starts_with("Selected Job 9 for cancelling\nConfirm cancel (yes/no): yes\n"), "{transcript}");
        assert_eq!(phase(&ctx, 9), DirJobPhase::Canceling);
        let (result, _) = run("cancel state=created yes", &ctx, &mut fake, &[]);
        assert_eq!(result.text_output, "Selected Job 7 for cancelling\n\
            JobId 7, Job backup.2026-10-16_10.00.00_07 marked to be canceled.\n");
        let (result, _) = run("cancel all yes", &ctx, &mut fake, &[]);
        assert!(result.text_output.starts_with("Selected Job 7 for cancelling\nSelected Job 8 for cancelling\n\
            Selected Job 9 for cancelling\n"), "{}", result.text_output);
        assert_eq!(phase(&ctx, 8), DirJobPhase::Canceling);
    }

    #[test]
    fn phase_status_codes() {
        use DirJobPhase as P;
        let codes: String = [P::Queued, P::Starting, P::Running, P::Paused, P::Canceling, P::WaitingCatReq,
            P::WaitingMedia, P::WaitingMount, P::Finishing, P::Completed, P::Error].iter().map(|p| p.job_status()).collect();
        assert_eq!(codes, "CCRBARmMRTf");
    }

    /// job.cc:698/703: only the daemons the job is connected to are contacted.
    #[test]
    fn only_connected_daemons_are_canceled() {
        let (mut w1, mut w2) = (Worker::spawn(), Worker::spawn());
        let ctx = ctx_with(ApiMode::Human, &[(5, "backup", DirJobPhase::Running, &w1), (6, "other", DirJobPhase::Starting, &w2)]);
        {
            let mut manager = ctx.job_manager.as_ref().unwrap().lock().unwrap();
            // Job 5 waits on the SD (connected to the FD only), job 6 on the FD.
            manager.set_connections(5, true, false, 'S');
            manager.set_connections(6, false, false, 'F');
        }
        let mut fake = Fake::default();
        let (result, _) = run("cancel jobid=5", &ctx, &mut fake, &[]);
        let unique = "backup.2026-10-16_10.00.00_05";
        assert_eq!(result.text_output, format!(" Handshake: Immediate TLS,\n2001 Job {unique} marked to be canceled.\n"));
        assert_eq!(fake.calls, vec![("fd", format!("fd1:cancel Job={unique}\n"))]);
        // JS_WaitFD is no queue state: nothing is printed, no daemon is contacted.
        let (result, _) = run("cancel jobid=6", &ctx, &mut fake, &[]);
        assert_eq!((result.text_output.as_str(), fake.calls.len()), ("", 1));
        assert!(w1.terminated() && w2.terminated());
        assert_eq!((phase(&ctx, 5), phase(&ctx, 6)), (DirJobPhase::Canceling, DirJobPhase::Canceling));
    }

    /// job.cc:741 RunScripts(jcr, ..., "AfterJob") after a cancel, once per job;
    /// not when the File daemon could not be reached.
    #[test]
    fn after_job_scripts_run_on_cancel() {
        let dir = std::env::temp_dir().join(format!("onesimus-c048-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("after.log");
        let _ = std::fs::remove_file(&log);
        let config = format!("{CONFIG}Job {{ Name = scripted; Client = fd1; Storage = File
  RunAfterFailedJob = \"/bin/sh -c 'echo %i %e %n %c >> {log}'\"
  RunAfterJob = \"/bin/sh -c 'echo success >> {log}'\" }}\n", log = log.display());
        let (w1, w2, w3) = (Worker::spawn(), Worker::spawn(), Worker::spawn());
        let mut ctx = context(&config, ApiMode::Human);
        let mut manager = JobManager::new(Arc::new(crate::message_bus::MessageBus::new(10)), None, None);
        for (id, worker) in [(7, &w1), (8, &w2), (9, &w3)] {
            let unique = format!("scripted.2026-10-16_10.00.00_0{id}");
            manager.add_job(id, "scripted", crate::job_worker::JobHandle::detached(worker.0.id() as i32, id, "scripted", &unique));
            manager.set_phase(id, DirJobPhase::Running);
            manager.set_job_resources(id, "fd1", "File");
        }
        manager.set_phase(8, DirJobPhase::Starting);
        ctx.job_manager = Some(Arc::new(Mutex::new(manager)));
        let read = || std::fs::read_to_string(&log).unwrap_or_default();
        let mut fake = Fake::default();
        run("cancel jobid=7", &ctx, &mut fake, &[]);
        assert_eq!(read(), "7 Canceled scripted fd1\n");
        // Once per job: a second cancel does not run them again.
        run("cancel jobid=7", &ctx, &mut fake, &[]);
        // A job still in the queue (JS_Created) runs them as well.
        let (result, _) = run("cancel jobid=8", &ctx, &mut fake, &[]);
        assert_eq!(result.text_output, "JobId 8, Job scripted.2026-10-16_10.00.00_08 marked to be canceled.\n");
        assert_eq!(read(), "7 Canceled scripted fd1\n8 Canceled scripted fd1\n");
        let mut fake = Fake { fd_down: true, ..Default::default() };
        run("cancel jobid=9", &ctx, &mut fake, &[]);
        assert_eq!(read(), "7 Canceled scripted fd1\n8 Canceled scripted fd1\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
