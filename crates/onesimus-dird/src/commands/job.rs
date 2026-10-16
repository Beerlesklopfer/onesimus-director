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
    let mut ua = crate::ua_select::Ua::new(ctx);
    let success = run_with_ua(&mut ua, ast, start_job);
    ua.finish(success)
}

/// Start the job of a confirmed run: the resolved request (job, level,
/// client, fileset, storage, pool and `yes`) goes through run_selected.
pub(crate) fn start_job(ast: &CommandAst, ctx: &ExecContext, job: bareos_config::resources::JobResource) -> CommandResult {
    match ctx.config.as_ref().map(|c| c.read()) {
        Some(Ok(config)) => run_selected(ast, ctx, config, job),
        Some(Err(_)) => CommandResult::error("Config read error\n".to_string()),
        None => CommandResult::error("No config loaded\n".to_string()),
    }
}

/// C++ ua_run.cc DoRunCmd(): OpenClientDb, job selection
/// (ScanCommandLineArguments/select_job_resource), then — unless `yes` was
/// given or the console is in batch (JSON) mode — DisplayJobParameters and
/// "OK to run? (yes/mod/no): " with the ModifyJobParameters dialog. `start`
/// runs the confirmed job; it is never called on "no", on a cancelled
/// dialog or without a console (GetCmd fails: "Job not run."). Returns the
/// DoRunCmd result.
pub(crate) fn run_with_ua(
    ua: &mut crate::ua_select::Ua,
    ast: &CommandAst,
    start: impl FnOnce(&CommandAst, &ExecContext, bareos_config::resources::JobResource) -> CommandResult,
) -> bool {
    let ctx = ua.ctx;
    if !ua.open_client_db(ast, false) {
        return false;
    }
    let Some(job) = run_select_job(ua, ast) else { return false };
    let mut state = RunState::new(ast, job);

    // Run without prompting? (`ua->batch` is the JSON mode here.)
    let yes = crate::ua_select::find_arg(ast, "yes").is_some();
    if yes || ctx.api_mode == crate::command_executor::ApiMode::Json {
        return run_start(ua, &state, start);
    }

    loop {
        state.display(ua);
        // Prompt User until we have a valid response.
        let answer = loop {
            let Some(answer) = ua.get_cmd("OK to run? (yes/mod/no): ") else {
                ua.send_msg("Job not run.\n");
                return false;
            };
            if run_answer_valid(&answer) {
                break answer;
            }
            ua.warning_msg(format!("Illegal response {answer}\n"));
        };
        let lower = answer.to_ascii_lowercase();
        // ".mod <args>" / "mod <args>": parse the new arguments, show again.
        if lower.starts_with(".mod ") || (lower.starts_with("mod ") && answer.len() > 6) {
            let rest = &answer[answer.find(' ').unwrap_or(answer.len())..];
            state.apply(&crate::session::parse_cli_command(&format!("mod {rest}")));
            continue;
        }
        if !answer.is_empty() && prefix_of(&lower, "mod") {
            match state.modify(ua) {
                Some(()) => continue,
                None => {
                    ua.send_msg("Job not run.\n");
                    return false;
                }
            }
        }
        if answer.is_empty() || prefix_of(&lower, "yes") {
            return run_start(ua, &state, start);
        }
        ua.send_msg("Job not run.\n");
        return false;
    }
}

/// `bstrncasecmp(cmd, word, strlen(cmd))`: the answer is a prefix of word.
fn prefix_of(answer_lower: &str, word: &str) -> bool {
    word.starts_with(answer_lower)
}

/// C++ DoRunCmd valid_response: empty, or a prefix of ".mod "/"mod "
/// (compared over at most the keyword length) or of "yes"/"no".
fn run_answer_valid(answer: &str) -> bool {
    let lower = answer.to_ascii_lowercase();
    let head = |n: usize| lower.chars().take(n).collect::<String>();
    answer.is_empty()
        || ".mod ".starts_with(&head(5))
        || "mod ".starts_with(&head(4))
        || prefix_of(&lower, "yes")
        || prefix_of(&lower, "no")
}

fn run_start(
    ua: &mut crate::ua_select::Ua,
    state: &RunState,
    start: impl FnOnce(&CommandAst, &ExecContext, bareos_config::resources::JobResource) -> CommandResult,
) -> bool {
    let result = start(&state.to_ast(), ua.ctx, state.job.clone());
    let success = result.success;
    ua.send_result(result);
    success
}

/// ScanCommandLineArguments' job part: `job=<name>`, else "A job name must
/// be specified." / "Job \"x\" not found" and select_job_resource.
fn run_select_job(ua: &mut crate::ua_select::Ua, ast: &CommandAst) -> Option<bareos_config::resources::JobResource> {
    let ctx = ua.ctx;
    let job_name = ast.arguments.iter().find(|a| a.key == "job").and_then(|a| a.value.clone());
    let find = |name: &str| ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|config| config.find_job(name).cloned())
        .filter(|job| crate::acl::access(ctx, crate::acl::Kind::Job, &job.name));
    if let Some(job) = job_name.as_deref().and_then(find) {
        return Some(job);
    }
    match job_name.as_deref() {
        Some(name) if !name.is_empty() => ua.send_msg(format!("Job \"{name}\" not found\n")),
        Some(_) => {}
        None => ua.send_msg("A job name must be specified.\n"),
    }
    let name = crate::ua_select::select_job_resource(ua)?;
    find(&name)
}

/// The parameters of an interactive run (RunContext and the JobControlRecord
/// fields that DisplayJobParameters shows and ModifyJobParameters changes).
struct RunState {
    job: bareos_config::resources::JobResource,
    level: String,
    client: String,
    fileset: String,
    storage: String,
    storage_source: &'static str,
    /// Explicit pool (command line or user input) with its source.
    pool: Option<(String, &'static str)>,
    /// Pool of the level overrides, applied once before the first display
    /// (DoRunCmd do_pool_overrides) and again only after a Level change.
    applied_pool: Option<(String, &'static str)>,
    priority: i64,
    when: String,
    format: String,
    plugin_options: Option<String>,
}

/// JobLevelToString() for the backup levels.
fn level_name(level: &str) -> String {
    match level.to_ascii_lowercase().as_str() {
        "full" | "f" => "Full",
        "incremental" | "i" => "Incremental",
        "differential" | "d" => "Differential",
        "since" | "s" => "Since",
        "virtualfull" | "v" => "VirtualFull",
        _ => return level.to_string(),
    }.to_string()
}

fn now_time() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

impl RunState {
    fn new(ast: &CommandAst, job: bareos_config::resources::JobResource) -> Self {
        let mut state = Self::defaults(job);
        state.apply(ast);
        state
    }

    /// SetJcrDefaults: everything from the Job resource.
    fn defaults(job: bareos_config::resources::JobResource) -> Self {
        Self {
            level: format!("{:?}", job.level),
            client: job.client.clone(),
            fileset: job.fileset.clone(),
            storage: job.storage.clone(),
            storage_source: "Job resource",
            pool: None,
            applied_pool: None,
            priority: i64::from(job.priority),
            when: now_time(),
            format: "Native".to_string(),
            plugin_options: None,
            job,
        }
    }

    /// Command line overrides (ScanCommandLineArguments, also `mod <args>`).
    fn apply(&mut self, ast: &CommandAst) {
        let arg = |key: &str| crate::ua_select::find_arg_with_value(ast, key).map(str::to_string);
        if let Some(level) = arg("level") { self.level = level_name(&level); }
        if let Some(client) = arg("client") { self.client = client; }
        if let Some(fileset) = arg("fileset") { self.fileset = fileset; }
        if let Some(storage) = arg("storage") {
            self.storage = storage;
            self.storage_source = "command line";
        }
        if let Some(pool) = arg("pool") { self.pool = Some((pool, "command line")); }
        if let Some(priority) = arg("priority") { self.priority = str_to_int64(&priority); }
        if let Some(options) = arg("pluginoptions") { self.plugin_options = Some(options); }
    }

    /// ApplyPoolOverrides (job.cc:1225) unless a pool was chosen.
    fn pool(&self) -> (String, &'static str) {
        if let Some((pool, source)) = self.pool.as_ref().or(self.applied_pool.as_ref()) {
            return (pool.clone(), source);
        }
        let job = &self.job;
        match self.level.as_str() {
            "Full" if !job.full_backup_pool.is_empty() => (job.full_backup_pool.clone(), "Job FullPool override"),
            "Incremental" if !job.incremental_backup_pool.is_empty() => (job.incremental_backup_pool.clone(), "Job IncPool override"),
            "Differential" if !job.differential_backup_pool.is_empty() => (job.differential_backup_pool.clone(), "Job DiffPool override"),
            _ => (job.pool.clone(), "Job resource"),
        }
    }

    /// DisplayJobParameters (ua_run.cc:1223) for Backup jobs; other job
    /// types show nothing yet.
    fn display(&mut self, ua: &mut crate::ua_select::Ua) {
        if !matches!(self.job.job_type, JobType::Backup) {
            return;
        }
        let (pool, pool_source) = self.pool();
        self.applied_pool.get_or_insert((pool.clone(), pool_source));
        let plugin = self.plugin_options.as_ref().map(|o| format!("Plugin Options: {o}\n")).unwrap_or_default();
        if ua.api() {
            ua.send_signal(bareos_bnet::BnetSignal::RunCmd);
            ua.send_msg(format!("Type: Backup\nTitle: Run Backup Job\nJobName:  {}\nLevel:    {}\nClient:   {}\n\
                Format:   {}\nFileSet:  {}\nPool:     {pool}\nStorage:  {}\nWhen:     {}\nPriority: {}\n{plugin}",
                self.job.name, self.level, self.client, self.format, self.fileset, self.storage, self.when, self.priority));
        } else {
            ua.send_msg(format!("Run Backup job\nJobName:  {}\nLevel:    {}\nClient:   {}\nFormat:   {}\n\
                FileSet:  {}\nPool:     {pool} (From {pool_source})\nStorage:  {} (From {})\nWhen:     {}\n\
                Priority: {}\n{plugin}",
                self.job.name, self.level, self.client, self.format, self.fileset, self.storage,
                self.storage_source, self.when, self.priority));
        }
    }

    /// ModifyJobParameters (ua_run.cc:531) for Backup jobs: Some(()) shows
    /// the parameters again, None ends the run ("Job not run.").
    fn modify(&mut self, ua: &mut crate::ua_select::Ua) -> Option<()> {
        use crate::ua_select;
        let backup = matches!(self.job.job_type, JobType::Backup);
        ua.start_prompt("Parameters to modify:\n");
        for item in ["Level", "Storage", "Job", "FileSet"] {
            ua.add_prompt(item);
        }
        ua.add_prompt(if matches!(self.job.job_type, JobType::Restore) { "Restore Client" } else { "Client" });
        for item in ["Backup Format", "When", "Priority"] {
            ua.add_prompt(item);
        }
        if matches!(self.job.job_type, JobType::Backup | JobType::Copy | JobType::Migrate | JobType::Verify) {
            ua.add_prompt("Pool");
            if self.level == "VirtualFull" || matches!(self.job.job_type, JobType::Copy | JobType::Migrate) {
                ua.add_prompt("NextPool");
                if backup { ua.add_prompt("Plugin Options"); }
            } else if matches!(self.job.job_type, JobType::Verify) {
                ua.add_prompt("Verify Job");
            } else if backup {
                ua.add_prompt("Plugin Options");
            }
        }
        let (choice, item) = ua.do_prompt("", "Select parameter to modify")?;
        match (choice, item.as_str()) {
            (0, _) => {
                // SelectJobLevel
                if backup {
                    ua.start_prompt("Levels:\n");
                    for level in ["Full", "Incremental", "Differential", "Since", "VirtualFull"] {
                        ua.add_prompt(level);
                    }
                    if let Some((_, level)) = ua.do_prompt("", "Select level") {
                        self.level = level;
                    }
                    // ApplyPoolOverrides(jcr, true) for the new level.
                    self.applied_pool = None;
                } else {
                    ua.warning_msg("Level not appropriate for this Job. Cannot be changed.\n");
                }
            }
            (1, _) => {
                self.storage = ua_select::select_storage_resource(ua, false)?;
                self.storage_source = "user selection";
            }
            (2, _) => {
                let name = ua_select::select_job_resource(ua)?;
                let job = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
                    .and_then(|c| c.find_job(&name).cloned())?;
                *self = Self::defaults(job);
            }
            (3, _) => self.fileset = ua_select::select_fileset_resource(ua)?,
            (4, _) => self.client = ua_select::select_client_resource(ua)?,
            (5, _) => self.format = ua.get_cmd("Please enter Backup Format: ")?,
            (6, _) => {
                let when = ua.get_cmd("Please enter desired start time as YYYY-MM-DD HH:MM:SS (return for now): ")?;
                self.when = if when.is_empty() {
                    now_time()
                } else if let Ok(time) = chrono::NaiveDateTime::parse_from_str(&when, "%Y-%m-%d %H:%M:%S") {
                    time.format("%Y-%m-%d %H:%M:%S").to_string()
                } else {
                    ua.send_msg("Invalid time, using current time.\n");
                    now_time()
                };
            }
            (7, _) => {
                let priority = ua.get_pint("Enter new Priority: ")?;
                if priority == 0 {
                    ua.send_msg("Priority must be a positive integer.\n");
                } else {
                    self.priority = i64::from(priority);
                }
            }
            (8, _) => {
                let pool = ua_select::select_pool_resource(ua)?;
                self.pool = Some((pool, "user input"));
            }
            (_, "Plugin Options") => {
                // GetPluginOptions
                if let Some(options) = ua.get_cmd("Please enter Plugin Options string: ") {
                    if crate::acl::access(ua.ctx, crate::acl::Kind::PluginOptions, &options) {
                        self.plugin_options = Some(options);
                    } else {
                        ua.send_msg("No authorization for \"PluginOptions\" specification.\n");
                    }
                }
            }
            (_, "Verify Job") => {
                ua_select::select_job_resource(ua);
            }
            (_, "NextPool") => {
                ua_select::select_pool_resource(ua)?;
            }
            _ => {}
        }
        Some(())
    }

    /// The confirmed request for run_selected.
    fn to_ast(&self) -> CommandAst {
        use crate::command_ast::Argument;
        let arg = |key: &str, value: &str| Argument { key: key.to_string(), value: Some(value.to_string()) };
        let mut arguments = vec![
            arg("job", &self.job.name),
            arg("level", &self.level),
            arg("client", &self.client),
            arg("fileset", &self.fileset),
            arg("storage", &self.storage),
        ];
        // Interactive runs keep the displayed pool (IgnoreLevelPoolOverrides).
        if let Some((pool, _)) = self.pool.as_ref().or(self.applied_pool.as_ref()) {
            arguments.push(arg("pool", pool));
        }
        arguments.push(Argument { key: "yes".to_string(), value: None });
        CommandAst { command_name: "run".to_string(), arguments, ..Default::default() }
    }
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

    // ua_run.cc:408 — without `yes` the run dialog (run_with_ua) asks first;
    // run_selected only starts confirmed requests.
    if !yes {
        return CommandResult::error("Job not run.\n".to_string());
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

    let resources = (client.name.clone(), storage.name.clone());
    drop(config); // Release RwLock before fork

    // Post queued status
    if let Some(smgr) = &ctx.session_mgr {
        smgr.bus().post(BusMessage::JobStatus {
            job_id, job_name: job.name.clone(), status: "Queued".to_string(),
            timestamp: now_unix(),
        });
        crate::console_msg::post_job_message(smgr.bus(), ctx.config.as_ref(), BusMessage::JobMessage {
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
                let mut mgr = mgr.lock().unwrap();
                mgr.add_job(job_id, &job.name, handle);
                mgr.set_job_resources(job_id, &resources.0, &resources.1);
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
pub(crate) fn str_to_int64(value: &str) -> i64 {
    let value = value.trim_start();
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let number = digits.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |n, d| n.wrapping_mul(10).wrapping_add(i64::from(d - b'0')));
    if negative { -number } else { number }
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
            // select_enable_disable_*_resource: DoPrompt dialog (C-011).
            let mut ua = crate::ua_select::Ua::new(ctx);
            ua.start_prompt(&format!("The defined {label} resources are:\n"));
            for name in &candidates {
                ua.add_prompt(name);
            }
            let msg = if res_type == "job" { "Select Job resource".to_string() } else { format!("Select {label} resource") };
            // EnableCmd/DisableCmd return true even when nothing was selected.
            let Some((_, name)) = ua.do_prompt(label, &msg) else { return ua.finish(true) };
            ua.send_result(toggle_enabled(config, res_type, &name, enable));
            return ua.finish(true);
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
        use crate::ua_select::tests::scripted;
        for line in ["enable storage=File", "enable", "enable job"] {
            let (result, _) = scripted(&[], || cmd_enable(&crate::session::parse_cli_command(line), &ctx));
            assert_eq!(result.text_output, "The defined Job resources are:\n", "{line}");
            assert_eq!(result.error_message, "Selection list for \"Job\" is empty!\n", "{line}");
        }
        let (_, console) = scripted(&["."], || cmd_disable(&crate::session::parse_cli_command("disable storage=File"), &ctx));
        assert_eq!(console.transcript(), "The defined Job resources are:\n1: a\n2: b\nSelect Job resource (1-2): .\n\
            Selection aborted, nothing done.\n");
        let cfg = ctx.config.as_ref().unwrap().read().unwrap();
        assert_eq!(cfg.find_storage("File").unwrap().enabled, storage_state);
        assert!(cfg.jobs.iter().all(|j| j.enabled));
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
        cmd_run(&crate::session::parse_cli_command(line), ctx)
    }

    /// Runs `line` with scripted answers; the start hook must never fire.
    fn dialog(line: &str, ctx: &ExecContext, answers: &[&str]) -> (bool, crate::ua_select::tests::Scripted) {
        crate::ua_select::tests::scripted(answers, || {
            let mut ua = crate::ua_select::Ua::new(ctx);
            let ok = run_with_ua(&mut ua, &crate::session::parse_cli_command(line),
                |_, _, _| panic!("{line}: {answers:?} must not start a job"));
            ua.finish(ok);
            ok
        })
    }

    #[test]
    fn missing_or_unknown_job_ends_in_the_selection() {
        let ctx = context(&format!("{JOB}\n{}", JOB.replace("b1", "A2")));
        // Without console: batch mode lists the choices.
        assert_eq!(run("run", &ctx).text_output,
            "A job name must be specified.\nThe defined Job resources are:\n1: A2\n2: b1\n\
             Your request has multiple choices for \"Job\". Selection is not possible in batch mode.\n");
        let (ok, console) = dialog("run job=nosuch yes", &ctx, &["0", "x", "."]);
        assert!(!ok);
        assert_eq!(console.transcript(),
            "Job \"nosuch\" not found\nThe defined Job resources are:\n1: A2\n2: b1\nSelect Job resource (1-2): 0\n\
             Please enter a number between 1 and 2\nSelect Job resource (1-2): x\n\
             Expected a positive integer, got: x\nSelect Job resource (1-2): .\nSelection aborted, nothing done.\n");
    }

    /// Context whose catalog and job manager record what a run would do.
    pub(crate) fn recording_context(jobs: &str) -> (ExecContext, Arc<std::sync::Mutex<Vec<String>>>) {
        let mut ctx = context(jobs);
        let (catalog, log) = crate::commands::admin::side_effect_tests::recording_catalog("b1");
        ctx.catalog = Some(catalog);
        ctx.job_manager = Some(Arc::new(std::sync::Mutex::new(crate::job_manager::JobManager::new(
            Arc::new(crate::message_bus::MessageBus::new(8)), ctx.config.clone(), None))));
        (ctx, log)
    }

    #[test]
    fn run_rerun_without_yes_create_and_start_nothing() {
        let (ctx, log) = recording_context(JOB);
        // No console: GetCmd fails, "Job not run.".
        for line in ["run", "run job=b1", "run job=b1 level=Full pool=Full", "run job=nosuch"] {
            let result = cmd_run(&crate::session::parse_cli_command(line), &ctx);
            assert!(result.text_output.ends_with("Job not run.\n"), "{line}: {}", result.text_output);
        }
        // Console answers that never start.
        for answers in [&["no"][..], &["n"], &["."], &["mod", "."], &["maybe", "no"], &["mod", "8", "."]] {
            let (ctx2, log2) = recording_context(JOB);
            let (ok, console) = crate::ua_select::tests::scripted(answers, || cmd_run(
                &crate::session::parse_cli_command("run job=b1"), &ctx2).success);
            assert!(!ok, "{answers:?}");
            assert!(console.text().ends_with("Job not run.\n"), "{answers:?}: {}", console.text());
            assert!(!log2.lock().unwrap().iter().any(|request| request == "CreateJob"), "{answers:?}");
        }
        let rerun = crate::commands::extra::cmd_rerun(&crate::session::parse_cli_command("rerun jobid=1"), &ctx);
        // The double has no Job record: reRunJob stops after the announcement.
        assert_eq!(rerun.text_output, "rerunning jobid 1\n");
        assert!(!log.lock().unwrap().iter().any(|request| request == "CreateJob"), "{:?}", log.lock().unwrap());
        assert!(ctx.job_manager.as_ref().unwrap().lock().unwrap().running_jobs().is_empty());
        // Control: with `yes` the double sees the job record request (and
        // refuses it, so nothing is forked).
        let started = cmd_run(&crate::session::parse_cli_command("run job=b1 level=Full yes"), &ctx);
        assert!(!started.success);
        assert!(log.lock().unwrap().iter().any(|request| request == "CreateJob"));
    }

    #[test]
    fn job_without_yes_is_displayed_and_asked() {
        let ctx = context(JOB);
        let result = run("run job=b1", &ctx);
        assert!(!result.success);
        assert!(result.text_output.starts_with("Run Backup job\nJobName:  b1\nLevel:    Incremental\nClient:   fd\n\
            Format:   Native\nFileSet:  fs\nPool:     Full (From Job resource)\nStorage:  File (From Job resource)\nWhen:     "),
            "{}", result.text_output);
        assert!(result.text_output.ends_with("Priority: 10\nJob not run.\n"));
        // A single job is selected automatically, then the question.
        let (ok, console) = dialog("run", &ctx, &["no"]);
        assert!(!ok);
        let text = console.transcript();
        assert!(text.starts_with("A job name must be specified.\nAutomatically selected Job: b1\nRun Backup job\n"), "{text}");
        assert!(text.ends_with("Priority: 10\nOK to run? (yes/mod/no): no\nJob not run.\n"), "{text}");
        assert_eq!(console.signals(), vec![bareos_bnet::BnetSignal::SubPrompt]);
    }

    #[test]
    fn illegal_answers_and_modify_dialog() {
        let ctx = context(JOB);
        let (_, console) = dialog("run job=b1", &ctx, &["yes please", "mod", "1", "1", "mod", "8", "2", "no"]);
        let text = console.transcript();
        assert!(text.contains("OK to run? (yes/mod/no): yes please\nIllegal response yes please\n"), "{text}");
        assert!(text.contains("Parameters to modify:\n 1: Level\n 2: Storage\n 3: Job\n 4: FileSet\n 5: Client\n\
            \x206: Backup Format\n 7: When\n 8: Priority\n 9: Pool\n10: Plugin Options\n\
            Select parameter to modify (1-10): 1\nLevels:\n1: Full\n2: Incremental\n3: Differential\n4: Since\n\
            5: VirtualFull\nSelect level (1-5): 1\nRun Backup job\nJobName:  b1\nLevel:    Full\n"), "{text}");
        assert!(text.contains("Select parameter to modify (1-10): 8\nEnter new Priority: 2\nRun Backup job\n"), "{text}");
        assert!(text.contains("Priority: 2\nOK to run? (yes/mod/no): no\nJob not run.\n"), "{text}");
        // Cancel inside the menu ends the run.
        let (_, console) = dialog("run job=b1", &ctx, &["mod", "."]);
        assert!(console.transcript().ends_with("Select parameter to modify (1-10): .\n\
            Selection aborted, nothing done.\nJob not run.\n"));
        // Single storage/pool are selected automatically; ".mod" re-scans.
        let (_, console) = dialog("run job=b1", &ctx, &["mod", "9", ".mod level=Differential", "no"]);
        let text = console.transcript();
        assert!(text.contains("Select parameter to modify (1-10): 9\nAutomatically selected Pool: Full\nRun Backup job\n"), "{text}");
        assert!(text.contains("Pool:     Full (From user input)\n"), "{text}");
        assert!(text.contains(".mod level=Differential\nRun Backup job\nJobName:  b1\nLevel:    Differential\n"), "{text}");
        assert!(run_answer_valid("") && run_answer_valid("y") && run_answer_valid("Ye") && run_answer_valid("n")
            && run_answer_valid("m") && run_answer_valid(".mo") && run_answer_valid("mod x"));
        assert!(!run_answer_valid("yess") && !run_answer_valid("x") && !run_answer_valid("modx"));
    }

    #[test]
    fn api_mode_signals_run_cmd() {
        let mut ctx = context(JOB);
        ctx.api_mode = crate::command_executor::ApiMode::Api;
        let (_, console) = dialog("run job=b1", &ctx, &["no"]);
        use bareos_bnet::BnetSignal::*;
        assert_eq!(console.signals(), vec![RunCmd, TextInput]);
        assert!(console.text().starts_with("Type: Backup\nTitle: Run Backup Job\nJobName:  b1\n"));
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
