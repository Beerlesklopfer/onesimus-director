//! Job control commands: run, cancel, enable, disable

use std::sync::atomic::{AtomicU32, Ordering};
use tracing::info;

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::job_worker::{self, JobParams, WorkerJobType};
use crate::message_bus::BusMessage;
use bareos_config::resources::JobType;

static NEXT_JOB_ID: AtomicU32 = AtomicU32::new(1);

pub fn cmd_run(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_run.cc DoRunCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_run_db(ast, ctx))
}

fn cmd_run_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let job_name = ast.arguments.iter()
        .find(|a| a.key == "job")
        .and_then(|a| a.value.as_deref());

    let config = match &ctx.config {
        Some(c) => match c.read() {
            Ok(c) => c,
            _ => return CommandResult::error("Config read error\n".to_string()),
        },
        None => return CommandResult::error("No config loaded\n".to_string()),
    };

    // ua_run.cc ScanCommandLineArguments: a missing or unknown job leads to
    // select_job_resource (ua_select.cc), never to a default job (C-012).
    let mut shown = String::new();
    let job = match job_name.and_then(|name| config.find_job(name))
        .filter(|job| crate::acl::access(ctx, crate::acl::Kind::Job, &job.name)) {
        Some(j) => j.clone(),
        None => {
            match job_name {
                Some(name) if !name.is_empty() => shown.push_str(&format!("Job \"{name}\" not found\n")),
                Some(_) => {}
                None => shown.push_str("A job name must be specified.\n"),
            }
            let jobs = crate::director_config::sort_case_insensitive(config.jobs.iter()
                .map(|j| j.name.as_str()).filter(|name| crate::acl::access(ctx, crate::acl::Kind::Job, name)))
                .into_iter().map(str::to_string).collect();
            match super::prompt::select_resource(ctx, "The defined Job resources are:\n", "Job",
                "Select Job resource", jobs) {
                Ok((name, text)) => {
                    shown.push_str(&text);
                    config.find_job(&name).cloned().expect("selected from the configured jobs")
                }
                Err(result) => return crate::ua_db::with_prefix(shown, result),
            }
        }
    };
    crate::ua_db::with_prefix(shown, run_selected(ast, ctx, config, job))
}

fn run_selected(
    ast: &CommandAst,
    ctx: &ExecContext,
    config: std::sync::RwLockReadGuard<'_, crate::director_config::DirectorConfig>,
    job: bareos_config::resources::JobResource,
) -> CommandResult {
    let job_name = job.name.as_str();
    let yes = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes"));

    if !job.enabled {
        return CommandResult::error(format!("Job '{job_name}' is disabled\n"));
    }

    let default_level = format!("{:?}", job.level);
    let requested_level = ast.arguments.iter()
        .find(|a| a.key == "level").and_then(|a| a.value.as_deref()).unwrap_or(&default_level);

    let client_name = ast.arguments.iter()
        .find(|a| a.key == "client")
        .and_then(|a| a.value.as_deref())
        .unwrap_or(&job.client);

    let storage_name = ast.arguments.iter()
        .find(|a| a.key == "storage")
        .and_then(|a| a.value.as_deref())
        .unwrap_or(&job.storage);

    let fileset_name = ast.arguments.iter().find(|a| a.key == "fileset")
        .and_then(|a| a.value.as_deref()).unwrap_or(&job.fileset);
    let client_res = config.find_client(client_name);
    let storage_res = config.find_storage(storage_name);
    let fileset_res = config.find_fileset(fileset_name);
    if client_res.is_none() { return CommandResult::error(format!("Client '{client_name}' not found\n")); }
    if storage_res.is_none() { return CommandResult::error(format!("Storage '{storage_name}' not found\n")); }
    if fileset_res.is_none() { return CommandResult::error(format!("FileSet '{fileset_name}' not found\n")); }

    // ua_run.cc:408 — without `yes` the job parameters are shown and the
    // console is asked "OK to run? (yes/mod/no): " (GetCmd, ua_run.cc:434).
    // Nothing is created or started before the answer (C-012).
    if !yes {
        return super::prompt::selection_required(format!("{}OK to run? (yes/mod/no): ",
            run_parameters(ast, &job, requested_level, client_name, fileset_name, storage_name)));
    }

    let (level, level_char, since_utime, prev_job) = match backup_level(requested_level, |level| {
        let cat = ctx.catalog.as_ref().ok_or("A catalog is required for incremental/differential backups")?;
        cat.since_utime(client_name, &job.name, fileset_name, level)
    }) {
        Ok(value) => value,
        Err(error) => return CommandResult::error(format!("run: {error}\n")),
    };
    // Use the effective level's pool after any missing-base upgrade to Full.
    let default_pool = match level_char {
        "F" if !job.full_backup_pool.is_empty() => &job.full_backup_pool,
        "D" if !job.differential_backup_pool.is_empty() => &job.differential_backup_pool,
        "I" if !job.incremental_backup_pool.is_empty() => &job.incremental_backup_pool,
        _ => &job.pool,
    };
    let pool = ast.arguments.iter().find(|a| a.key == "pool")
        .and_then(|a| a.value.as_deref()).unwrap_or(default_pool);

    let client = client_res.unwrap();
    let storage = storage_res.unwrap();
    let dir_name = config.director.name.clone();

    // Unique job name
    let now = chrono::Local::now();
    let unique_job = format!("{}.{}", job.name, now.format("%Y-%m-%d_%H.%M.%S_%f"));

    // Create job record in DB — JobId comes from the database
    let job_type_str = match job.job_type { JobType::Backup => "B", JobType::Restore => "R", _ => "B" };
    let job_id = if let Some(cat) = &ctx.catalog {
        match cat.create_job(
            &unique_job, &job.name, job_type_str, level_char,
            client_name, pool, fileset_name, now.timestamp(),
        ) {
            Ok(id) => id as u32,
            Err(e) => {
                tracing::error!("Failed to create job record: {e}");
                return CommandResult::error(format!("Failed to create job record: {e}\n"));
            }
        }
    } else {
        NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed)
    };

    // Build JobParams for the autonomous worker
    let params = JobParams {
        job_id,
        unique_job: unique_job.clone(),
        job_name: job.name.clone(),
        job_type: WorkerJobType::Backup,
        level: level.clone(),
        fd_address: client.address.clone(),
        fd_port: client.port,
        fd_password: client.password.clone(),
        sd_address: storage.address.clone(),
        sd_port: storage.port,
        sd_password: storage.password.clone(),
        sd_device: storage.device.clone(),
        sd_media_type: storage.media_type.clone(),
        director_name: dir_name,
        pool_name: pool.to_string(),
        fileset_includes: fileset_res.map(|f| f.include_blocks.clone()).unwrap_or_default(),
        fileset_excludes: fileset_res.map(|f| f.exclude_blocks.clone()).unwrap_or_default(),
        since_utime,
        prev_job,
        // Write Bootstrap from job config
        where_path: None,
        replace: None,
        restore_jobid: None,
        bootstrap: if !job.write_bootstrap.is_empty() { Some(job.write_bootstrap.clone()) } else { None },
    };

    drop(config); // Release RwLock before fork

    // Post queued status
    if let Some(smgr) = &ctx.session_mgr {
        smgr.bus().post(BusMessage::JobStatus {
            job_id, job_name: job.name.clone(), status: "Queued".to_string(),
            timestamp: now_unix(),
        });
        smgr.bus().post(BusMessage::JobMessage {
            job_id, job_name: job.name.clone(),
            msg_type: crate::message_bus::MsgType::Info,
            text: format!("Start Backup JobId={job_id}, Job={}, Level={level}", job.name),
            timestamp: now_unix(),
        });
    }

    // Fork the autonomous job worker
    match job_worker::spawn_job_worker(job_id, &job.name, &params) {
        Ok(handle) => {
            tracing::info!("Job {job_id} '{}': worker pid={}", job.name, handle.pid);
            // Register with JobManager — monitor thread polls all jobs
            if let Some(ref mgr) = ctx.job_manager {
                mgr.lock().unwrap().add_job(job_id, &job.name, handle);
            }
        }
        Err(e) => {
            tracing::error!("Job {job_id}: fork failed: {e}");
            return CommandResult::error(format!("Failed to start job: {e}\n"));
        }
    }

    let output = format!(
        "Job queued. JobId={job_id}\n\
         Run {:?} job\n\
         Job:        {}\n\
         Level:      {level}\n\
         Client:     {client_name}\n\
         Storage:    {storage_name}\n\
         Pool:       {pool}\n\
         FileSet:    {}\n\
         Priority:   {}\n\
         When:       now\n",
        job.job_type, job.name, fileset_name, job.priority,
    );

    CommandResult::ok(output)
}

/// DisplayJobParameters (ua_run.cc:1223) for the console, before any
/// override is applied; the job type selects the upstream title.
fn run_parameters(ast: &CommandAst, job: &bareos_config::resources::JobResource, level: &str,
    client: &str, fileset: &str, storage: &str) -> String {
    if !matches!(job.job_type, JobType::Backup) {
        return String::new();
    }
    let level = match level.to_ascii_lowercase().as_str() {
        "full" | "f" => "Full",
        "incremental" | "i" => "Incremental",
        "differential" | "d" => "Differential",
        _ => level,
    };
    let arg = |key: &str| ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref());
    // job.cc:1225 ApplyPoolOverrides / ua_run.cc:818 pool sources.
    let (pool, pool_source) = match arg("pool") {
        Some(pool) => (pool, "command line"),
        None => match level {
            "Full" if !job.full_backup_pool.is_empty() => (job.full_backup_pool.as_str(), "Job FullPool override"),
            "Incremental" if !job.incremental_backup_pool.is_empty() => (job.incremental_backup_pool.as_str(), "Job IncPool override"),
            "Differential" if !job.differential_backup_pool.is_empty() => (job.differential_backup_pool.as_str(), "Job DiffPool override"),
            _ => (job.pool.as_str(), "Job resource"),
        },
    };
    let storage_source = if arg("storage").is_some() { "command line" } else { "Job resource" };
    format!("Run Backup job\nJobName:  {}\nLevel:    {level}\nClient:   {client}\nFormat:   Native\n\
             FileSet:  {fileset}\nPool:     {pool} (From {pool_source})\nStorage:  {storage} (From {storage_source})\n\
             When:     {}\nPriority: {}\n",
        job.name, chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), job.priority)
}

/// Decide the effective backup level before creating its catalog record or worker.
fn backup_level(
    requested: &str,
    mut lookup: impl FnMut(&str) -> Result<Option<(i64, String)>, String>,
) -> Result<(String, &'static str, i64, String), String> {
    let (name, code) = match requested.to_ascii_lowercase().as_str() {
        "full" | "f" => ("full", "F"),
        "incremental" | "i" => ("incremental", "I"),
        "differential" | "d" => ("differential", "D"),
        _ => return Err(format!("Unsupported backup level: {requested}")),
    };
    if code == "F" { return Ok((name.into(), code, 0, String::new())); }
    match lookup(code)? {
        Some((timestamp, previous)) if timestamp > 0 => Ok((name.into(), code, timestamp, previous)),
        _ => {
            info!("No matching Full backup chain for {name}; upgrading to Full");
            Ok(("full".into(), "F", 0, String::new()))
        }
    }
}

/// Bareos str_to_int64(): leading blanks, optional sign, leading digits.
fn str_to_int64(value: &str) -> i64 {
    let value = value.trim_start();
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let number = digits.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |n, d| n.wrapping_mul(10).wrapping_add(i64::from(d - b'0')));
    if negative { -number } else { number }
}

/// ua_cmds.cc CancelJobs() with ua_select.cc select_jobs(): explicit
/// jobid/ujobid/job selections first, then "No Jobs running." when nothing
/// (authorized) runs. ErrorMsg lines precede the SendMsg text.
pub fn cmd_cancel(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let active = ctx.job_manager.as_ref()
        .map(|mgr| mgr.lock().unwrap().active_jobs()).unwrap_or_default();
    let mut text = String::new();
    let mut errors = String::new();
    let mut selected: Vec<u32> = Vec::new();
    let result = |text: String, errors: String| {
        let mut result = CommandResult::ok_with_errors(text, errors);
        result.errors_first = true;
        result
    };

    // FindArgKeyword(ua, {"job", "jobid", "ujobid"}) > 0: the first of these
    // keywords decides, and "job" (index 0) skips the explicit selection.
    let keyword = ast.arguments.iter().find_map(|arg| ["job", "jobid", "ujobid"].iter()
        .position(|key| arg.key.eq_ignore_ascii_case(key)));
    if keyword.is_some_and(|index| index > 0) {
        for arg in &ast.arguments {
            let value = arg.value.as_deref();
            let job = if arg.key.eq_ignore_ascii_case("jobid") {
                let id = str_to_int64(value.unwrap_or(""));
                if id == 0 { continue; }
                match active.iter().find(|job| i64::from(job.id) == id) {
                    Some(job) => job,
                    None => {
                        errors.push_str(&format!("JobId {} is not running. Use Job name to cancel inactive jobs.\n",
                            value.unwrap_or("")));
                        continue;
                    }
                }
            } else if arg.key.eq_ignore_ascii_case("job") || arg.key.eq_ignore_ascii_case("ujobid") {
                let Some(name) = value else { continue };
                let partial = arg.key.eq_ignore_ascii_case("job");
                match active.iter().find(|job| if partial { job.unique_name.starts_with(name) } else { job.unique_name == name }) {
                    Some(job) => job,
                    None => {
                        // WarningMsg: no warning channel yet, ordered as text.
                        text.push_str(&format!("Warning Job {name} is not running. Continuing anyway ...\n"));
                        continue;
                    }
                }
            } else {
                continue;
            };
            if !crate::acl::access(ctx, crate::acl::Kind::Job, &job.name) {
                errors.push_str("Unauthorized command from this console.\n");
                return result(text, errors);
            }
            if !selected.contains(&job.id) { selected.push(job.id); }
        }
    }

    if selected.is_empty() {
        let authorized = active.iter().filter(|job| crate::acl::access(ctx, crate::acl::Kind::Job, &job.name)).count();
        if authorized == 0 {
            text.push_str(if active.is_empty() { "No Jobs running.\n" } else { "None of your jobs are running.\n" });
            return result(text, errors);
        }
        // Interactive selection (select_jobs prompt) is not implemented yet.
        let mut out = String::from("Running jobs:\n");
        out.push_str("  JobId    Name                           Phase\n");
        for job in active.iter().filter(|job| crate::acl::access(ctx, crate::acl::Kind::Job, &job.name)) {
            out.push_str(&format!("  {:<8} {:<30} {}\n", job.id, job.name, job.phase.name()));
        }
        out.push_str("\ncancel jobid=<id> to cancel a job.\n");
        text.push_str(&out);
        return result(text, errors);
    }

    for id in selected {
        // Send SIGTERM to worker process via JobManager
        let canceled = ctx.job_manager.as_ref().is_some_and(|mgr| mgr.lock().unwrap().cancel_job(id));
        if canceled {
            if let Some(smgr) = &ctx.session_mgr {
                smgr.bus().post(BusMessage::JobStatus {
                    job_id: id, job_name: String::new(), status: "Canceling".to_string(),
                    timestamp: now_unix(),
                });
            }
            // Final status, counters and EndTime are committed after worker exit.
            text.push_str(&format!("Job {id} canceled (SIGTERM sent to worker)\n"));
        } else {
            text.push_str(&format!("Job {id} not found or already finished\n"));
        }
    }
    result(text, errors)
}

pub fn cmd_enable(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    en_disable(ast, ctx, true)
}

pub fn cmd_disable(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    en_disable(ast, ctx, false)
}

/// DoEnDisableCmd (ua_cmds.cc:1022): `schedule`, else `client`, else a job.
/// There is no storage branch — `storage=` and missing names lead to the
/// select_enable_disable_*_resource selection (ua_select.cc), never to a
/// toggle of something else (C-012).
fn en_disable(ast: &CommandAst, ctx: &ExecContext, enable: bool) -> CommandResult {
    use crate::acl::{access, Kind};
    let config = match &ctx.config {
        Some(c) => c,
        None => return CommandResult::error("No config loaded\n".to_string()),
    };
    let has = |key: &str| ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key));
    let value = |key: &str| ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some()).and_then(|a| a.value.as_deref());
    let (res_type, label, kind) = if has("schedule") { ("schedule", "Schedule", Kind::Schedule) }
        else if has("client") { ("client", "Client", Kind::Client) }
        else { ("job", "Job", Kind::Job) };
    let (name, shown) = match value(res_type) {
        Some(name) if access(ctx, kind, name) => (name.to_string(), String::new()),
        Some(name) => return CommandResult::ok_with_errors(String::new(), format!("{label} \"{name}\" not found.\n")),
        None => {
            let candidates: Vec<String> = {
                let cfg = config.read().unwrap();
                let states: Vec<(&str, bool)> = match res_type {
                    "schedule" => cfg.schedules.iter().map(|s| (s.name.as_str(), s.enabled)).collect(),
                    "client" => cfg.clients.iter().map(|c| (c.name.as_str(), c.enabled)).collect(),
                    _ => cfg.jobs.iter().map(|j| (j.name.as_str(), j.enabled)).collect(),
                };
                crate::director_config::sort_case_insensitive(states.into_iter()
                    .filter(|(name, enabled)| *enabled != enable && access(ctx, kind, name)).map(|(name, _)| name))
                    .into_iter().map(str::to_string).collect()
            };
            match super::prompt::select_resource(ctx, &format!("The defined {label} resources are:\n"), label,
                &format!("Select {label} resource"), candidates) {
                Ok(selected) => selected,
                // EnableCmd/DisableCmd return true even when nothing was selected.
                Err(mut result) => { result.success = true; return result; }
            }
        }
    };
    crate::ua_db::with_prefix(shown, toggle_enabled(config, res_type, &name, enable))
}

/// ua_cmds.cc DoEnDisableCmd(): an unknown resource is an ErrorMsg, the
/// command itself still returns true.
fn toggle_enabled(config: &crate::director_config::SharedConfig, res_type: &str, name: &str, enable: bool) -> CommandResult {
    let action = if enable { "enabled" } else { "disabled" };
    let mut cfg = config.write().unwrap();
    let (label, found) = match res_type {
        "client" => ("Client", cfg.clients.iter_mut().find(|c| c.name == name).map(|c| c.enabled = enable)),
        "job" => ("Job", cfg.jobs.iter_mut().find(|j| j.name == name).map(|j| j.enabled = enable)),
        "schedule" => ("Schedule", cfg.schedules.iter_mut().find(|s| s.name == name).map(|s| s.enabled = enable)),
        _ => return CommandResult::ok(format!("{res_type}: not supported\n")),
    };
    match found {
        Some(()) => CommandResult::ok(format!("{label} \"{name}\" {action}\n")),
        None => CommandResult::ok_with_errors(String::new(), format!("{label} \"{name}\" not found.\n")),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs() as i64
}

#[cfg(test)]
mod error_text_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str("Director { Name = dir }").unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    #[test]
    fn enable_disable_unknown_resource_is_an_error_message_of_a_successful_command() {
        let ctx = context();
        for (line, expected) in [("enable job=nosuch", "Job \"nosuch\" not found.\n"),
            ("disable client=nosuch", "Client \"nosuch\" not found.\n")] {
            let ast = crate::session::parse_cli_command(line);
            let result = if line.starts_with("enable") { cmd_enable(&ast, &ctx) } else { cmd_disable(&ast, &ctx) };
            assert!(result.success, "{line}");
            assert_eq!(result.error_message, expected);
        }
    }

    #[test]
    fn enable_disable_never_toggle_storages_or_guess_a_resource() {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nStorage { Name = File; Address = localhost; Enabled = no }\n\
             Job { Name = a; Type = Backup }\nJob { Name = b; Type = Backup }").unwrap();
        let ctx = ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let storage_state = ctx.config.as_ref().unwrap().read().unwrap().find_storage("File").unwrap().enabled;
        for line in ["enable storage=File", "enable", "enable job"] {
            let result = cmd_enable(&crate::session::parse_cli_command(line), &ctx);
            assert_eq!(result.text_output, "The defined Job resources are:\n", "{line}");
            assert_eq!(result.error_message, "Selection list for \"Job\" is empty!\n", "{line}");
        }
        let result = cmd_disable(&crate::session::parse_cli_command("disable storage=File"), &ctx);
        assert_eq!(result.text_output, "The defined Job resources are:\n1: a\n2: b\nSelect Job resource (1-2): ");
        let cfg = ctx.config.as_ref().unwrap().read().unwrap();
        assert_eq!(cfg.find_storage("File").unwrap().enabled, storage_state);
        assert!(cfg.jobs.iter().all(|j| j.enabled));
    }

    #[test]
    fn cancel_follows_select_jobs() {
        let ctx = context();
        let cancel = |line: &str| cmd_cancel(&crate::session::parse_cli_command(line), &ctx);
        let result = cancel("cancel jobid=999999");
        assert_eq!((result.error_message.as_str(), result.text_output.as_str()),
            ("JobId 999999 is not running. Use Job name to cancel inactive jobs.\n", "No Jobs running.\n"));
        assert!(result.success && result.errors_first);
        // "job" as first keyword skips the explicit selection; jobid=abc is ignored.
        for line in ["cancel", "cancel job=nosuch", "cancel jobid=abc", "cancel all"] {
            let result = cancel(line);
            assert_eq!((result.error_message.as_str(), result.text_output.as_str()), ("", "No Jobs running.\n"), "{line}");
        }
        assert_eq!(cancel("cancel ujobid=nosuch").text_output,
            "Warning Job nosuch is not running. Continuing anyway ...\nNo Jobs running.\n");
        assert_eq!((str_to_int64(" 12abc"), str_to_int64("-3"), str_to_int64("abc")), (12, -3, 0));
    }
}

#[cfg(test)]
mod run_confirmation_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn context(jobs: &str) -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(&format!(
            "Director {{ Name = dir }}\nClient {{ Name = fd; Address = localhost; Password = x }}\n\
             Storage {{ Name = File; Address = localhost; Password = x; Device = d; Media Type = File }}\n\
             FileSet {{ Name = fs; Include {{ File = /etc }} }}\nPool {{ Name = Full; Pool Type = Backup }}\n{jobs}"
        )).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    const JOB: &str = "Job { Name = b1; Type = Backup; Level = Incremental; Client = fd; FileSet = fs; Storage = File; Pool = Full; Messages = m; Priority = 10 }";

    fn run(line: &str, ctx: &ExecContext) -> CommandResult {
        cmd_run_db(&crate::session::parse_cli_command(line), ctx)
    }

    #[test]
    fn missing_or_unknown_job_ends_in_the_selection() {
        let ctx = context(&format!("{JOB}\n{}", JOB.replace("b1", "A2")));
        assert_eq!(run("run", &ctx).text_output,
            "A job name must be specified.\nThe defined Job resources are:\n1: A2\n2: b1\nSelect Job resource (1-2): ");
        assert_eq!(run("run job=nosuch yes", &ctx).text_output,
            "Job \"nosuch\" not found\nThe defined Job resources are:\n1: A2\n2: b1\nSelect Job resource (1-2): ");
    }

    #[test]
    fn job_without_yes_is_only_displayed() {
        let ctx = context(JOB);
        let result = run("run job=b1", &ctx);
        assert!(result.success);
        assert!(result.text_output.starts_with("Run Backup job\nJobName:  b1\nLevel:    Incremental\nClient:   fd\n\
            Format:   Native\nFileSet:  fs\nPool:     Full (From Job resource)\nStorage:  File (From Job resource)\nWhen:     "),
            "{}", result.text_output);
        assert!(result.text_output.ends_with("Priority: 10\nOK to run? (yes/mod/no): "));
        // A single job is selected automatically, but still not started.
        let result = run("run", &ctx);
        assert!(result.text_output.starts_with("A job name must be specified.\nAutomatically selected Job: b1\nRun Backup job\n"));
        assert!(result.text_output.ends_with("OK to run? (yes/mod/no): "));
    }
}

#[cfg(test)]
mod backup_level_tests {
    use super::*;

    #[test]
    fn valid_incremental_and_differential_keep_their_reference() {
        for (requested, code) in [("Incremental", "I"), ("D", "D")] {
            let (_, effective, since, previous) = backup_level(requested, |selected| {
                assert_eq!(selected, code);
                Ok(Some((123456, "previous-job".into())))
            }).unwrap();
            assert_eq!((effective, since, previous.as_str()), (code, 123456, "previous-job"));
        }
    }

    #[test]
    fn missing_chain_upgrades_but_errors_do_not_become_full_backups() {
        assert_eq!(backup_level("I", |_| Ok(None)).unwrap().1, "F");
        assert!(backup_level("D", |_| Err("catalog unavailable".into())).is_err());
        assert!(backup_level("VirtualFull", |_| panic!("invalid level must not query")).is_err());
        assert_eq!(backup_level("Full", |_| panic!("full must not query")).unwrap().1, "F");
    }
}
