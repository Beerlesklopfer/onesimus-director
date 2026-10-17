//! Session/interactive commands — non-dot, non-admin commands.
//!
//! Dot commands have been consolidated into commands/dot.rs.

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;

/// Send SD command via bareos-stored crate (resolves storage from config).
fn sd_cmd_via_config(ctx: &ExecContext, storage_name: &str, command: &str) -> Result<String, String> {
    let config = ctx.config.as_ref()
        .and_then(|c| c.read().ok())
        .ok_or("No config")?;
    let stor = config.storages.iter().find(|s| s.name == storage_name)
        .ok_or(format!("Storage '{storage_name}' not found"))?;
    let dir_name = config.director.name.clone();
    let addr = stor.address.clone();
    let port = stor.port;
    let password = stor.password.clone();
    drop(config);
    bareos_stored::command::sd_command(&addr, port, &password, &dir_name, command)
}

// --- Interactive/session commands ---

/// C++ FindArgKeyword(ua, {"on", "off"}): first argument keyword (case
/// insensitive) that is `on` or `off`; anything else is ignored.
pub(crate) fn on_off_keyword(ast: &CommandAst) -> Option<bool> {
    ast.arguments.iter().find_map(|a| {
        if a.key.eq_ignore_ascii_case("on") { Some(true) }
        else if a.key.eq_ignore_ascii_case("off") { Some(false) }
        else { None }
    })
}

/// C++ AutodisplayCmd/gui_cmd reply: silent on success; without on/off an
/// ErrorMsg is sent, but the handler still returns true (CMD_OK).
fn on_off_reply(ast: &CommandAst) -> CommandResult {
    match on_off_keyword(ast) {
        Some(_) => CommandResult::ok(String::new()),
        None => CommandResult {
            success: true,
            error_message: "ON or OFF keyword missing.\n".to_string(),
            ..Default::default()
        },
    }
}

/// C++ ua_output.cc AutodisplayCmd; the session applies the flag.
pub fn cmd_autodisplay(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    on_off_reply(ast)
}

/// C++ ua_cmds.cc AutomountCmd: exactly one argument, anything but `off`
/// (case insensitive) means on. The console session asks for the value
/// ("Turn on or off? ") when the argument count differs; without an answer
/// the flag is left unchanged.
pub fn cmd_automount(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if let ([arg], Some(mgr)) = (ast.arguments.as_slice(), &ctx.session_mgr) {
        mgr.set_session_automount(ctx.session_id, !arg.key.eq_ignore_ascii_case("off"));
    }
    CommandResult::ok(String::new())
}

/// C++ ua_output.cc gui_cmd; the session applies the flag.
pub fn cmd_gui(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    on_off_reply(ast)
}

/// DoMessages(): the queued console messages go raw to the attached console
/// (in every API mode, before the command's result); a caller without a
/// console gets them as text output.
pub(crate) fn do_messages(ctx: &ExecContext) -> CommandResult {
    let mut output = String::new();
    crate::console_msg::do_messages(ctx, |messages| {
        crate::ua_select::send_raw(messages).unwrap_or_else(|| {
            output = messages.iter().map(|m| String::from_utf8_lossy(m)).collect();
            true
        })
    });
    CommandResult::ok(output)
}

/// C++ ua_output.cc MessagesCmd: pending messages when the command line
/// (`ua->cmd`) passes the Command ACL, otherwise the decoration
/// "You have no messages." (plain console mode only).
pub fn cmd_messages(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let cmd = if ast.raw_line.is_empty() { ast.command_name.as_str() } else { ast.raw_line.as_str() };
    if crate::console_msg::pending() && crate::acl::access(ctx, crate::acl::Kind::Command, cmd) {
        return do_messages(ctx);
    }
    let decoration = if ctx.api_mode == crate::command_executor::ApiMode::Human { "You have no messages.\n" } else { "" };
    CommandResult::ok(decoration.to_string())
}

/// C++ ua_cmds.cc MemoryCmd: prints ListDirStatusHeader.
pub fn cmd_memory(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::commands::status::dir_status_header(ctx)
}

/// C++ ua_cmds.cc TraceCmd(): with not exactly one argument the value is
/// asked for (GetCmd "Turn on or off? "); anything but "off" turns tracing
/// on. No answer (lone dot, batch mode) changes nothing. Silent.
pub fn cmd_trace(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    let onoff = match ast.arguments.as_slice() {
        [arg] => arg.key.clone(),
        _ => match ua.get_cmd("Turn on or off? ") {
            Some(answer) => answer,
            None => return ua.finish(true),
        },
    };
    let path = trace_path(ctx);
    if let Err(error) = bareos_bnet::debug::set_trace(&path, !onoff.eq_ignore_ascii_case("off")) {
        ua.error_msg(format!("trace: {}: {error}\n", path.display()));
    }
    ua.finish(true)
}

pub(crate) fn trace_path(ctx: &ExecContext) -> std::path::PathBuf {
    let directory = ctx.config.as_ref().and_then(|c| c.read().ok())
        .map(|c| c.director.working_directory.clone()).unwrap_or_else(|| "/var/lib/bareos".into());
    std::path::Path::new(&directory).join("bareos-dir.trace")
}

pub fn cmd_var(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc var_cmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_var_db(ast, ctx))
}

/// C++ var_cmd(): expand everything behind the first blank of the input
/// line (`ua->cmd`) and print it; a failed expansion prints an empty line
/// (the reason goes to the job messages).
fn cmd_var_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let line = if ast.raw_line.is_empty() { reconstruct_line(ast) } else { ast.raw_line.clone() };
    // `var` without blank: the C++ loop reads behind the string terminator
    // into stale buffer contents (undefined); here the rest is empty.
    let rest = line.find(' ').map(|i| line[i..].trim_start_matches(' ')).unwrap_or("");
    let Some(mut resolver) = console_resolver(ctx) else {
        return CommandResult::ok("\n".to_string());
    };
    let value = match crate::expand::variable_expansion(rest.as_bytes(), &mut resolver) {
        Ok(value) => value,
        Err(message) => {
            tracing::error!("{}", message.trim_end());
            Vec::new()
        }
    };
    CommandResult::ok(format!("{}\n", String::from_utf8_lossy(&value)))
}

/// Command line of a structured (non-text) request.
fn reconstruct_line(ast: &CommandAst) -> String {
    let mut line = ast.command_name.clone();
    for arg in &ast.arguments {
        line.push(' ');
        line.push_str(&arg.key);
        if let Some(value) = &arg.value {
            line.push('=');
            line.push_str(value);
        }
    }
    line
}

/// Expansion context of the console JobControlRecord (ua_server.cc
/// new_control_jcr("-Console-", JT_CONSOLE)): job defaults of the first Job
/// resource, type Console, no level, JobId 0, the session catalog.
fn console_resolver(ctx: &ExecContext) -> Option<crate::expand::DirectorResolver<'static>> {
    use crate::expand::{CounterDef, DirectorResolver, JobVars};
    static SEQ: std::sync::Mutex<u32> = std::sync::Mutex::new(0);

    let config = ctx.config.as_ref()?.read().ok()?;
    let job = config.jobs.first();
    let storage = job.and_then(|j| j.storage.split(',').next()).unwrap_or("").trim().to_string();
    let media_type = config.storages.iter().find(|s| s.name == storage)
        .map(|s| s.media_type.clone()).unwrap_or_default();
    let catalog = crate::ua_db::selected_catalog(ctx)
        .or_else(|| config.catalogs.first().map(|c| c.name.clone()))
        .unwrap_or_default();
    // CreateUniqueJobName(): sequence 1..59, then 0
    let seq = {
        let mut seq = SEQ.lock().unwrap_or_else(|e| e.into_inner());
        *seq = if *seq >= 59 { 0 } else { *seq + 1 };
        *seq
    };
    let job_name = format!("-Console-.{}_{seq:02}", crate::localtime::strftime_now("%Y-%m-%d_%H.%M.%S"));
    let catalog_handle = ctx.catalog.clone();
    Some(DirectorResolver {
        job: JobVars {
            job: job.map(|j| j.name.clone()).unwrap_or_default(),
            dir: config.director.name.clone(),
            level: " ".into(),
            job_type: "Console".into(),
            job_id: 0,
            client: job.map(|j| j.client.clone()).unwrap_or_default(),
            num_vols: 0,
            pool: job.map(|j| j.pool.clone()).unwrap_or_default(),
            storage,
            catalog,
            media_type,
            job_name,
        },
        counters: CounterDef::from_raw(&config.raw),
        update_counter: Some(Box::new(move |def: &CounterDef, value: i32| {
            let handle = catalog_handle.as_ref().ok_or("no catalog")?;
            handle.raw_query(&crate::expand::update_counter_sql(def, value)).map(|_| ())
        })),
    })
}

/// `whoami` — the name of the resource behind the console's ACL (the named
/// Console), or `root` for the default console (ua_cmds.cc WhoAmICmd).
pub fn cmd_whoami(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let name = ctx.session_mgr.as_ref()
        .and_then(|smgr| smgr.get_session_names().into_iter()
            .find(|(id, _)| *id == ctx.session_id).map(|(_, name)| name))
        .filter(|name| name != crate::director_config::USER_AGENT_CONSOLE)
        .unwrap_or_else(|| "root".to_string());
    CommandResult::ok(format!("{name}\n"))
        .with_json(serde_json::json!({ "whoami": name }))
}

pub fn cmd_use(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::ua_db::use_catalog(ast, ctx)
}

/// `setip` (ua_cmds.cc:995 SetipCmd): only a named console whose name is
/// also a Client may set that client's address to its own peer address.
/// The default console has no corresponding resource (C-012).
pub fn cmd_setip(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let console = ctx.session_mgr.as_ref()
        .and_then(|manager| manager.authenticated_console(ctx.session_id))
        .filter(|name| name != "*UserAgent*");
    let Some(console) = console else {
        return CommandResult::error("No corresponding client found.\n".into());
    };
    let known = ctx.config.as_ref().and_then(|c| c.read().ok())
        .is_some_and(|cfg| cfg.find_client(&console).is_some());
    if !known || !crate::acl::access(ctx, crate::acl::Kind::Client, &console) {
        return CommandResult::error(format!("Client \"{console}\" not found.\n"));
    }
    // Upstream stores the console's peer address; the session carries none
    // here, so the address is left unchanged rather than reported as set.
    CommandResult::error(format!("Client \"{console}\": address of the console connection unknown, nothing changed.\n"))
}

pub fn cmd_setdevice(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // SetDeviceCommand::SendToSd(): only a failed connect is an error; the
    // SD's single reply is relayed if one arrives (recv() >= 0), otherwise
    // nothing is printed.
    setdevice_with(ast, ctx, |storage, director, command| {
        use bareos_bnet::BnetMessage;
        let mut sd = bareos_stored::connect::connect_sd(&storage.address, storage.port,
            &storage.password, director)?;
        let reply = match sd.send_str(command).ok().and_then(|_| sd.recv().ok()) {
            Some(BnetMessage::Data(bytes)) => String::from_utf8_lossy(&bytes).into_owned(),
            _ => String::new(),
        };
        let _ = sd.send_signal(bareos_bnet::BnetSignal::Terminate);
        Ok(reply)
    })
}

fn setdevice_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    send: impl FnOnce(&bareos_config::resources::StorageResource, &str, &str) -> Result<String, String>,
) -> CommandResult {
    // SetDeviceCommand::Cmd(): ScanCommandLine errors are followed by the
    // command usage and the command fails.
    let usage = |errors: String| CommandResult {
        success: false, error_message: errors, errors_first: true,
        text_output: crate::command_registry::cmd_usage("setdevice", ""), ..Default::default()
    };
    for arg in &ast.arguments {
        if !matches!(arg.key.as_str(), "storage" | "device" | "autoselect") {
            return usage(format!("Wrong argument: {}\n", arg.key));
        }
    }
    // FindArgWithValue(): only the first argument with this keyword counts.
    let value = |key: &str| crate::ua_select::find_arg_with_value(ast, key).filter(|s| !s.is_empty());
    let mut missing = String::new();
    for key in ["autoselect", "device", "storage"] {
        if value(key).is_none() { missing.push_str(&format!("Argument missing: {key}\n")); }
    }
    if !missing.is_empty() { return usage(missing); }
    // BoolString (lib/bool_string.h): case-sensitive true/yes/1, false/no/0.
    let autoselect = match value("autoselect").unwrap() {
        "yes" | "true" | "1" => 1,
        "no" | "false" | "0" => 0,
        other => return usage(format!("Wrong argument: {other}\n")),
    };
    let name = value("storage").unwrap();
    let device = value("device").unwrap();
    if device.chars().any(char::is_control) {
        return CommandResult::error("Invalid control character in device name\n".into());
    }
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else {
        return CommandResult::error("No config\n".into());
    };
    // SetDeviceCommand::Cmd() (ua_cmds.cc:1597): a restricted console is
    // checked first, then GetStoreResWithName() (audited ACL check + lookup).
    if crate::acl::has_restrictions(ctx, crate::acl::Kind::Storage)
        && !crate::acl::access(ctx, crate::acl::Kind::Storage, name)
    {
        return CommandResult::error(format!("Access to storage \"{name}\" forbidden\n"));
    }
    let storage = crate::acl::access_audited(ctx, crate::acl::Kind::Storage, name)
        .then(|| config.find_storage(name).cloned()).flatten();
    let Some(storage) = storage else {
        return CommandResult::error(format!("Storage \"{name}\" not found.\n"));
    };
    // SetDeviceCommand::SendToSd() (ua_cmds.cc:1552): an NDMP storage is
    // skipped silently and counts as success (compatibility log C-050).
    if storage.protocol.is_ndmp() {
        return CommandResult::ok(String::new());
    }
    let director = config.director.name.clone();
    drop(config);
    let command = format!("setdevice device={device} autoselect={autoselect}");
    match send(&storage, &director, &command) {
        Ok(response) => CommandResult::ok(format!("Connecting to Storage daemon {} at {}:{}\n{response}",
            storage.name, storage.address, storage.port)),
        Err(error) => {
            tracing::debug!("setdevice: SD connect failed: {error}");
            CommandResult::error("Failed to connect to Storage daemon.\n".into())
        }
    }
}

/// Bareos Is_a_number(): optional sign, digits, optional fraction/exponent.
fn is_a_number(value: &str) -> bool {
    let value = value.strip_prefix(['+', '-']).unwrap_or(value);
    let (mantissa, exponent) = match value.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e.strip_prefix(['+', '-']).unwrap_or(e))),
        None => (value, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    !int.is_empty() && digits(int) && digits(frac)
        && exponent.is_none_or(|e| !e.is_empty() && digits(e))
}

/// ua_run.cc reRunCmd(): OpenClientDb, GetRerunCmdlineArguments, then the
/// jobid list or the selection of failed jobs (since_jobid/days/hours) with
/// "rerun these jobids? (yes/no): ", and reRunJob() for each job.
pub fn cmd_rerun(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let query = |sql: &str| match &ctx.catalog {
        Some(cat) => cat.query_rows(sql).map(|t| t.rows),
        None => Err("no catalog".to_string()),
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64).unwrap_or(0);
    let mut ua = crate::ua_select::Ua::new(ctx);
    let ok = rerun_dialog(&mut ua, ast, &query, now, crate::commands::job::start_job);
    ua.finish(ok)
}

/// RerunArguments (ua_run.h:29); the numbers are C `int`.
#[derive(Debug, Default, PartialEq)]
struct RerunArguments {
    since_jobid: i32,
    until_jobid: i32,
    days: i32,
    hours: i32,
    yes: bool,
    jobids: Vec<u32>,
}

/// GetRerunCmdlineArguments (ua_run.cc:203); None after a parsing error.
fn rerun_arguments(ua: &mut crate::ua_select::Ua, ast: &CommandAst) -> Option<RerunArguments> {
    use crate::ua_select::{find_arg, find_arg_with_value};
    let int = |key: &str| find_arg_with_value(ast, key)
        .map(|v| crate::commands::job::str_to_int64(v) as i32).unwrap_or(0);
    let mut args = RerunArguments {
        since_jobid: int("since_jobid"), until_jobid: int("until_jobid"),
        days: int("days"), hours: int("hours"), yes: find_arg(ast, "yes").is_some(),
        jobids: Vec::new(),
    };
    let timeframe = args.days != 0 || args.hours != 0;
    let Some(jobid) = find_arg_with_value(ast, "jobid") else {
        if !timeframe && args.since_jobid == 0 {
            ua.send_msg("Please specify jobid, since_jobid, hours or days\n");
            return None;
        }
        return Some(args);
    };
    // split_string() reads with std::getline: a trailing delimiter (or an
    // empty value) yields no last element.
    let mut parts: Vec<&str> = jobid.split(',').collect();
    if jobid.is_empty() || jobid.ends_with(',') {
        parts.pop();
    }
    for part in parts {
        if !is_a_number(part) {
            ua.send_msg(format!("Specified jobid \"{part}\" is not valid\n"));
            return None;
        }
        args.jobids.push(crate::commands::job::str_to_int64(part) as u32);
    }
    if args.since_jobid != 0 {
        ua.send_msg("Please specify either jobid or since_jobid (and optionally until_jobid)\n");
        return None;
    }
    if timeframe {
        ua.send_msg("Please specify either jobid or a timeframe\n");
        return None;
    }
    Some(args)
}

/// PrepareRerunSqlQuery (ua_run.cc:271) with ConvertDaysHoursToSecs.
fn rerun_sql(args: &RerunArguments, now: i64) -> String {
    let secs = args.days.wrapping_mul(86400).wrapping_add(args.hours.wrapping_mul(3600));
    let schedtime = crate::localtime::strftime("%Y-%m-%d %H:%M:%S", (now - i64::from(secs)) as libc::time_t);
    let mut select = "SELECT JobId FROM Job WHERE JobStatus = 'f'".to_string();
    if args.since_jobid != 0 {
        select += &format!(" AND JobId >= {}", args.since_jobid);
        if args.until_jobid != 0 {
            select += &format!(" AND JobId <= {}", args.until_jobid);
        }
    } else {
        select += &format!(" AND SchedTime > '{schedtime}'");
    }
    select + " ORDER BY JobId"
}

type Rows = Vec<Vec<Option<String>>>;
type StartJob = fn(&CommandAst, &ExecContext, bareos_config::resources::JobResource) -> CommandResult;

/// reRunCmd() (ua_run.cc:300); `query` is the catalog, `start` starts a
/// confirmed run (job::start_job).
fn rerun_dialog(
    ua: &mut crate::ua_select::Ua,
    ast: &CommandAst,
    query: &dyn Fn(&str) -> Result<Rows, String>,
    now: i64,
    start: StartJob,
) -> bool {
    if !ua.open_client_db(ast, false) {
        return true;
    }
    let Some(mut args) = rerun_arguments(ua, ast) else { return false };
    if args.days != 0 || args.hours != 0 || args.since_jobid != 0 {
        // GetQueryDbids: a failed query leaves the list empty (Jmsg M_ERROR).
        let ids: Vec<u32> = query(&rerun_sql(&args, now)).unwrap_or_default().into_iter()
            .map(|row| row.into_iter().next().flatten()
                .map(|id| crate::commands::job::str_to_int64(&id) as u32).unwrap_or(0))
            .collect();
        if ids.is_empty() {
            ua.send_msg("No jobids with the specified options were found.\n");
            return true;
        }
        ua.send_msg("The following ids were selected for rerun:\n");
        for (i, id) in ids.iter().enumerate() {
            ua.send_msg(if i > 0 { format!(",{}", *id as i32) } else { format!("{}", *id as i32) });
        }
        ua.send_msg("\n");
        if !args.yes && ua.get_yesno("rerun these jobids? (yes/no): ") != Some(true) {
            return false;
        }
        args.jobids.extend(ids);
    }
    let when = crate::localtime::strftime("%Y-%m-%d %H:%M:%S", now as libc::time_t);
    for &jobid in &args.jobids {
        if !rerun_job(ua, jobid, args.yes, &when, query, start) {
            return false;
        }
    }
    true
}

/// JobLevelToString() (dird_conf.cc:2090) for a catalog level code.
fn job_level_to_string(level: u8) -> String {
    match level {
        b'F' => "Full", b'B' => "Base", b'I' => "Incremental", b'D' => "Differential",
        b'S' => "Since", b'f' => "VirtualFull", b'C' => "Catalog", b'V' => "InitCatalog",
        b'O' => "VolumeToCatalog", b'd' => "DiskToCatalog", b'A' => "Data", b' ' => " ",
        other => return format!("{} ({other})", other as char),
    }.to_string()
}

/// reRunJob() (ua_run.cc:71): announce, build the run command line, then
/// RunCmd().
fn rerun_job(
    ua: &mut crate::ua_select::Ua,
    jobid: u32,
    yes: bool,
    when: &str,
    query: &dyn Fn(&str) -> Result<Rows, String>,
    start: StartJob,
) -> bool {
    ua.send_msg(format!("rerunning jobid {}\n", jobid as i32));
    match rerun_command(ua.ctx, jobid, yes, when, query) {
        Err(()) => false,
        Ok(None) => true,
        Ok(Some(run)) => crate::commands::job::run_with_ua(ua, &run, start),
    }
}

/// The run command line of reRunJob() from the Job record and its Client,
/// Pool and FileSet records. Err: a record is missing (only a job message,
/// Jmsg M_WARNING, in C++); Ok(None): not a Backup, Copy or Migrate job.
fn rerun_command(
    ctx: &ExecContext,
    jobid: u32,
    yes: bool,
    when: &str,
    query: &dyn Fn(&str) -> Result<Rows, String>,
) -> Result<Option<CommandAst>, ()> {
    use crate::command_ast::Argument;
    let first_row = |sql: String| query(&sql).ok().and_then(|rows| rows.into_iter().next()).ok_or(());
    // ua_run.cc:80-187: Jmsg(ua->jcr, M_WARNING, ...) of the console JCR
    let warn = |record: &str, error: String| {
        if let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) {
            crate::console_msg::job_message(&config, "-Console-", 0, crate::message_bus::MsgType::Warning, 0,
                &format!("Error getting {record} record for Job rerun: ERR={error}"));
        }
    };
    let job = first_row(format!(
        "SELECT Name, Type, Level, ClientId, PoolId, FileSetId FROM Job WHERE JobId={jobid}"))
        .inspect_err(|_| warn("Job", format!("No Job found for JobId {jobid}\n")))?;
    let field = |i: usize| job.get(i).cloned().flatten().unwrap_or_default();
    let id = |i: usize| crate::commands::job::str_to_int64(&field(i));
    let name = field(0);
    let job_type = field(1).bytes().next().unwrap_or(0);
    let level = field(2).bytes().next().unwrap_or(0);
    if !matches!(job_type, b'B' | b'c' | b'g') {
        return Ok(None);
    }
    let arg = |key: &str, value: String| Argument { key: key.to_string(), value: Some(value) };
    let mut arguments = vec![arg("job", name.clone())];
    if level != b' ' {
        arguments.push(arg("level", job_level_to_string(level)));
    }
    let name_of = |table: &str, column: &str, key: &str, value: i64| {
        first_row(format!("SELECT {column} FROM {table} WHERE {key}={value}"))
            .map(|row| row.into_iter().next().flatten().unwrap_or_default())
    };
    if id(3) != 0 {
        let client = name_of("Client", "Name", "ClientId", id(3))
            .inspect_err(|_| warn("Client", "Client record not found in Catalog.\n".to_string()))?;
        arguments.push(arg("client", client));
    }
    if id(4) != 0 {
        // GetPoolRecord leaves errmsg as it was when no row matches
        let pool = name_of("Pool", "Name", "PoolId", id(4)).inspect_err(|_| warn("Pool", String::new()))?;
        // Copy, Migrate and VirtualFull read from the Job resource's pool
        // (GetJobResWithName, ACL checked) and write to the record's pool.
        if matches!(job_type, b'c' | b'g') || level == b'f' {
            let source = crate::acl::access(ctx, crate::acl::Kind::Job, &name).then(|| {
                ctx.config.as_ref().and_then(|c| c.read().ok())
                    .and_then(|c| c.find_job(&name).map(|j| j.pool.clone()))
            }).flatten();
            if let Some(source) = source {
                arguments.push(arg("pool", source));
            }
            arguments.push(arg("nextpool", pool));
        } else {
            arguments.push(arg("pool", pool));
        }
    }
    if id(5) != 0 {
        let fileset = name_of("FileSet", "FileSet", "FileSetId", id(5))
            .inspect_err(|_| warn("FileSet", "FileSet record \"\" not found.\n".to_string()))?;
        arguments.push(arg("fileset", fileset));
    }
    arguments.push(arg("when", when.to_string()));
    if yes {
        arguments.push(Argument { key: "yes".to_string(), value: None });
    }
    Ok(Some(CommandAst { command_name: "run".to_string(), arguments, ..Default::default() }))
}

pub fn cmd_relabel(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    crate::commands::admin::label_dialog(&mut ua, ast, true, |ua, request| {
        relabel_execute(ua, request, |stor, command| sd_cmd_via_config(ctx, stor, command));
    });
    ua.finish(true)
}

/// The SD relabel and the catalog rename of a complete relabel request.
fn relabel_execute(ua: &mut crate::ua_select::Ua, request: crate::commands::admin::LabelRequest,
    send: impl FnOnce(&str, &str) -> Result<String, String>) {
    let crate::commands::admin::LabelRequest::Label { storage, volume: newname, old_volume: Some(oldvol), .. } = request else {
        return;
    };
    match send(&storage, &format!("relabel OldVolume=\"{oldvol}\" Volume=\"{newname}\"")) {
        Ok(resp) => {
            if let Some(cat) = &ua.ctx.catalog {
                let _ = cat.raw_query(&format!("UPDATE media SET volumename='{}' WHERE volumename='{}'",
                    newname.replace('\'', "''"), oldvol.replace('\'', "''")));
            }
            ua.send_msg(format!("Relabel {oldvol} → {newname}:\n{resp}\n"));
        }
        Err(e) => ua.error_msg(format!("relabel: {e}\n")),
    }
}

pub fn cmd_delete(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    let mut deleted = None;
    let ok = delete_dialog(&mut ua, ast, &mut deleted, delete_execute);
    let result = ua.finish(ok);
    match deleted {
        Some(deleted) => result.with_json(serde_json::json!({ "deleted": deleted })),
        None => result,
    }
}

/// What a confirmed delete removes from the catalog.
#[derive(Debug, PartialEq, Eq)]
enum DeleteAction {
    Volume { name: String, media_id: i64, purged: bool },
    Pool(String),
    Job(u64),
    Storage(String),
}

/// DeleteCmd (ua_cmds.cc:2220): OpenClientDb, keyword (or the warning and
/// "Choose catalog item to delete"), then DeleteVolume/DeletePool/DeleteJob/
/// DeleteStorage with their selections and confirmations. `act` runs only
/// for a selected and confirmed item. `deleted` is the `.api 2` result
/// object (ObjectStart("deleted") after OpenClientDb); the "jobids" array
/// of `delete jobid=` is filled here, `act` adds its own arrays.
fn delete_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    deleted: &mut Option<serde_json::Map<String, serde_json::Value>>,
    mut act: impl FnMut(&mut crate::ua_select::Ua, DeleteAction, &mut serde_json::Map<String, serde_json::Value>)) -> bool {
    use crate::ua_select::{self, find_arg, find_arg_with_value};
    const KEYWORDS: [&str; 4] = ["volume", "pool", "jobid", "storage"];
    let ctx = ua.ctx;
    if !ua.open_client_db(ast, true) {
        return true;
    }
    let deleted = deleted.insert(serde_json::Map::new());
    let keyword = match ast.arguments.iter().find_map(|a| KEYWORDS.iter().position(|k| a.key.eq_ignore_ascii_case(k))) {
        Some(keyword) => Some(keyword),
        None => {
            ua.warning_msg("In general it is not a good idea to delete either a\n\
                Pool or a Volume since they may contain data.\n\n");
            ua.do_keyword_prompt("Choose catalog item to delete", &KEYWORDS)
        }
    };
    match keyword {
        Some(0) => {
            // DeleteVolume
            let Some(media) = ua_select::select_media_dbr(ua, ast) else { return true };
            ua.warning_msg(format!("\nThis command will delete volume {}\n\
                and all Jobs saved on that volume from the Catalog\n", media.volume_name));
            if !ua.batch() && find_arg(ast, "yes").is_none() {
                let question = format!("Are you sure you want to delete Volume \"{}\"? (yes/no): ", media.volume_name);
                if ua.get_yesno(&question) != Some(true) {
                    return true;
                }
            }
            act(ua, DeleteAction::Volume { purged: media.vol_status == "Purged", media_id: media.media_id,
                name: media.volume_name }, deleted);
        }
        Some(1) => {
            // DeletePool
            if ua.batch() {
                // DeletePool's false is ignored by DeleteCmd.
                ua.error_msg("Deleting pools from the catalog is not supported in batch mode.\n");
                return true;
            }
            let Some(pool) = ua_select::select_pool_dbr(ua, ast, "pool") else { return true };
            let question = format!("Are you sure you want to delete Pool \"{pool}\"? (yes/no): ");
            if ua.get_yesno(&question) == Some(true) {
                act(ua, DeleteAction::Pool(pool), deleted);
            }
        }
        Some(2) => {
            // DeleteJob
            if find_arg_with_value(ast, "jobid").is_none() {
                // DoJobDelete's ArrayItem without an array is dropped.
                if let Some(jobid) = ua.get_pint("Enter JobId to delete: ") {
                    act(ua, DeleteAction::Job(u64::from(jobid)), deleted);
                }
                return true;
            }
            deleted.insert("jobids".into(), serde_json::Value::Array(Vec::new()));
            let mut job_delete = |ua: &mut crate::ua_select::Ua, deleted: &mut serde_json::Map<String, serde_json::Value>, jobid: u64| {
                act(ua, DeleteAction::Job(jobid), deleted);
                if let Some(serde_json::Value::Array(ids)) = deleted.get_mut("jobids") {
                    ids.push(serde_json::Value::String((jobid as u32).to_string()));
                }
            };
            let number = |s: &str| ua_select::is_a_number(s);
            for value in ast.arguments.iter().filter(|a| a.key.eq_ignore_ascii_case("jobid")).filter_map(|a| a.value.clone()) {
                for token in value.split(',') {
                    if let Some((low, high)) = token.split_once('-') {
                        // DeleteJobIdRange
                        if !(number(low) && number(high)) {
                            ua.error_msg(format!("Illegal JobId range {low} - {high}, ignored\n"));
                            continue;
                        }
                        let (first, last) = (crate::commands::job::str_to_int64(low) as u64,
                            crate::commands::job::str_to_int64(high) as u64);
                        if last <= first {
                            ua.error_msg(format!("Illegal JobId range {low} - {high} should define increasing JobIds, ignored\n"));
                            continue;
                        }
                        if !ua.batch() && last - first > 25 {
                            let question = format!("Are you sure you want to delete {} JobIds ? (yes/no): ", last - first);
                            if ua.get_yesno(&question) != Some(true) {
                                continue;
                            }
                        }
                        for jobid in first..=last {
                            job_delete(ua, deleted, jobid);
                        }
                    } else if number(token) {
                        job_delete(ua, deleted, crate::commands::job::str_to_int64(token) as u64);
                    } else {
                        ua.error_msg(format!("Illegal JobId {token} ignored\n"));
                    }
                }
            }
        }
        Some(_) => {
            // DeleteStorage
            let given = match find_arg_with_value(ast, "storage") {
                Some(name) => name.to_string(),
                None => ua.get_cmd("Enter storage to delete: ").unwrap_or_default(),
            };
            let configured: Vec<String> = ctx.config.as_ref().and_then(|c| c.read().ok())
                .map(|c| c.storages.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
            let orphaned: Vec<String> = ctx.catalog.as_ref()
                .and_then(|cat| cat.query_rows("SELECT Name FROM Storage").ok())
                .map(|t| t.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>())
                .unwrap_or_default().into_iter().filter(|name| !configured.contains(name)).collect();
            ua.send_msg(format!("Found {} orphaned Storage records.\n", orphaned.len()));
            if !orphaned.contains(&given) {
                ua.error_msg(format!("The given storage '{given}' either does not exist at all, or still exists in \
                    the configuration. In order to remove a storage from the catalog, its configuration must be removed first.\n"));
                return true;
            }
            act(ua, DeleteAction::Storage(given), deleted);
        }
        None => ua.warning_msg("Nothing done.\n"),
    }
    true
}

/// The catalog work of a confirmed delete (DeleteVolume after the
/// confirmation, DoJobDelete, DeletePoolRecord, delete_storages).
fn delete_execute(ua: &mut crate::ua_select::Ua, action: DeleteAction,
    deleted: &mut serde_json::Map<String, serde_json::Value>) {
    use serde_json::Value;
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    match action {
        DeleteAction::Volume { name, media_id, purged } => {
            if !purged {
                // GetVolumeJobids, PurgeJobsFromCatalog.
                let Ok(table) = cat.query_rows(&format!("SELECT DISTINCT JobId FROM JobMedia WHERE MediaId={media_id}")) else {
                    ua.error_msg("Can't list jobs on this volume\n");
                    return;
                };
                let jobids: Vec<String> = table.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect();
                if !jobids.is_empty() {
                    let list = jobids.join(",");
                    if let Ok(db) = cat.dedicated() {
                        super::purge::purge_jobs(&db, &list);
                    }
                    deleted.insert("jobids".into(), Value::Array(jobids.iter().cloned().map(Value::String).collect()));
                    ua.info_msg(format!("Deleted {} jobs and associated records deleted from the catalog (jobids: {list}).\n",
                        jobids.len()));
                }
                // DeleteMediaRecord: DoMediaPurge of a not purged volume.
                if let Ok(table) = cat.query_rows(&format!("SELECT JobId from JobMedia WHERE MediaId={media_id}")) {
                    for jobid in table.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()) {
                        for table in ["Job", "File", "JobMedia"] {
                            let _ = cat.raw_query(&format!("DELETE FROM {table} WHERE JobId={jobid}"));
                        }
                    }
                }
            }
            let _ = cat.raw_query(&format!("DELETE FROM Media WHERE MediaId={media_id}"));
            deleted.insert("volumes".into(), Value::Array(vec![Value::String(name.clone())]));
            ua.send_msg(format!("Volume {name} deleted.\n"));
        }
        DeleteAction::Job(id) => {
            // PurgeJobsFromCatalog.
            match cat.dedicated() {
                Ok(db) => super::purge::purge_jobs(&db, &id.to_string()),
                Err(e) => ua.error_msg(format!("{e}\n")),
            }
            ua.send_msg(format!("Jobid {id} and associated records deleted from the catalog.\n"));
        }
        DeleteAction::Pool(pool) => {
            // BareosDb::DeletePoolRecord(): its volumes, then the pool (no
            // message, errors only in the catalog's errmsg).
            let id = cat.query_rows(&format!("SELECT PoolId FROM Pool WHERE Name='{}'", pool.replace('\'', "''")))
                .ok().filter(|t| t.rows.len() == 1)
                .and_then(|t| t.rows.into_iter().next())
                .map(|row| super::job::str_to_int64(row.first().cloned().flatten().as_deref().unwrap_or("")));
            if let Some(id) = id {
                let id = id as i32;
                let _ = cat.execute(&format!("DELETE FROM Media WHERE Media.PoolId = {id}"));
                let _ = cat.execute(&format!("DELETE FROM Pool WHERE Pool.PoolId = {id}"));
            }
        }
        DeleteAction::Storage(storage) => {
            // get_deletable_storageids() (dbcheck_utils.cc:168): only
            // storages no volume or device refers to; the Emsg warnings go
            // to the daemon log.
            let column = |sql: &str| cat.query_rows(sql).map(|t| t.rows.into_iter()
                .filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>()).unwrap_or_default();
            let mut deletable = Vec::new();
            for id in column(&format!("SELECT storageid FROM storage WHERE Name in ('{storage}')")) {
                let volumes = column(&format!("SELECT volumename FROM media WHERE storageid={id}"));
                let devices = column(&format!("SELECT name FROM device WHERE storageid={id}"));
                for (what, name) in volumes.iter().map(|v| ("volume", v)).chain(devices.iter().map(|d| ("device", d))) {
                    tracing::warn!("Orphaned storage '{storage}' is being used by {what} '{name}'. \
                        An orphaned storage will only be removed when it is no longer referenced.");
                }
                if volumes.is_empty() && devices.is_empty() {
                    deletable.push(id);
                }
            }
            if !deletable.is_empty() {
                ua.send_msg(format!("Deleting {} orphaned storage record(s).\n", deletable.len()));
                for id in deletable {
                    let _ = cat.execute(&format!("DELETE FROM storage WHERE storageid={id}"));
                }
            }
        }
    }
}

pub fn cmd_create(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    create_dialog(&mut ua, ast, create_pool);
    ua.finish(true)
}

/// CreateCmd (ua_cmds.cc:820): OpenClientDb, get_pool_resource (`pool=` or
/// the selection list — never a default), then CreatePool. `create` runs
/// only for a selected pool resource.
fn create_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    create: impl FnOnce(&mut crate::ua_select::Ua, bareos_config::resources::PoolResource)) {
    if !ua.open_client_db(ast, false) {
        return;
    }
    let Some(name) = crate::ua_select::get_pool_resource(ua, ast) else { return };
    let pool = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|c| c.find_pool(&name).cloned());
    if let Some(pool) = pool {
        create(ua, pool);
    }
}

/// CreatePool(POOL_OP_CREATE) (ua_cmds.cc:831) and the "Pool %s created."
/// message, which follows even the errors (C-066).
fn create_pool(ua: &mut crate::ua_select::Ua, pool: bareos_config::resources::PoolResource) {
    use crate::commands::pool_db;
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    let settings = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|c| pool_db::pool_settings(&c, &pool.name));
    if let Some(settings) = settings {
        let (mut errmsg, mut msgs) = (String::new(), Vec::new());
        let mut db = crate::catreq::Cat::new(&cat, &mut errmsg, &mut msgs);
        match pool_db::create_pool(&mut db, &settings, true) {
            0 => ua.error_msg(format!("Error: Pool {} already exists.\nUse update to change it.\n", pool.name)),
            -1 => ua.error_msg(errmsg),
            _ => {}
        }
    }
    ua.send_msg(format!("Pool {} created.\n", pool.name));
}

pub fn cmd_add(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    add_dialog(&mut ua, ast, add_volumes);
    ua.finish(true)
}

/// The volumes `add` creates.
#[derive(Debug, PartialEq, Eq)]
struct AddVolumes {
    pool: String,
    pool_id: i64,
    media_type: String,
    names: Vec<String>,
    slot: i64,
    in_changer: bool,
}

/// add_cmd (ua_cmds.cc:611): hint, OpenClientDb, GetPoolDbr, maximum check,
/// storage/media type, then number, (base) name, starting number and for
/// autochangers slot and InChanger. `create` runs only after the complete
/// dialog.
fn add_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    create: impl FnOnce(&mut crate::ua_select::Ua, AddVolumes)) {
    ua.send_msg("You probably don't want to be using this command since it\n\
        creates database records without labeling the Volumes.\n\
        You probably want to use the \"label\" command.\n\n");
    if !ua.open_client_db(ast, false) {
        return;
    }
    if let Some(volumes) = add_questions(ua, ast) {
        create(ua, volumes);
    }
}

fn add_questions(ua: &mut crate::ua_select::Ua, ast: &CommandAst) -> Option<AddVolumes> {
    use crate::ua_select;
    let ctx = ua.ctx;
    let pool = ua_select::select_pool_dbr(ua, ast, "pool")?;
    let counts = ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows(&format!("SELECT PoolId, MaxVols, NumVols FROM Pool WHERE Name='{}'",
            pool.replace('\'', "''"))).ok())
        .and_then(|t| t.rows.into_iter().next())
        .map(|row| row.into_iter().map(|v| v.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0)).collect::<Vec<_>>())
        .unwrap_or_else(|| vec![0, 0, 0]);
    let (pool_id, mut max_vols, num_vols) = (counts[0], counts[1], counts[2]);
    while max_vols > 0 && num_vols >= max_vols {
        ua.warning_msg(format!("Pool already has maximum volumes={max_vols}\n"));
        max_vols = i64::from(ua.get_pint("Enter new maximum (zero for unlimited): ")?);
    }
    // Media type of the storage, otherwise GetMediaType.
    let storage = ua_select::get_storage_resource(ua, ast, false, false).map(|s| s.name);
    let media_type = match &storage {
        Some(name) => ctx.config.as_ref().and_then(|c| c.read().ok())
            .and_then(|c| c.find_storage(name).map(|s| s.media_type.clone())).unwrap_or_default(),
        None => ua_select::get_media_type(ua, ast)?,
    };
    let max = if max_vols == 0 { 1000 } else { max_vols - num_vols };
    let num = loop {
        let num = i64::from(ua.get_pint(&format!("Enter number of Volumes to create. 0=>fixed name. Max={max}: "))?);
        if num > max {
            ua.warning_msg(format!("The number must be between 0 and {max}\n"));
            continue;
        }
        break num;
    };
    let base = loop {
        let name = ua.get_cmd(if num == 0 { "Enter Volume name: " } else { "Enter base volume name: " })?;
        if !ua_select::is_volume_name_legal(ua, &name) {
            continue;
        }
        if name.len() >= 128 - 10 {
            ua.warning_msg("Volume name too long.\n");
            continue;
        }
        break name;
    };
    let (start, count) = if num > 0 {
        let start = loop {
            let start = i64::from(ua.get_pint("Enter the starting number: ")?);
            if start < 1 {
                ua.warning_msg("Start number must be greater than zero.\n");
                continue;
            }
            break start;
        };
        (start, num)
    } else {
        (1, 1)
    };
    let (mut slot, mut in_changer) = (0, false);
    if storage.as_deref().is_some_and(|name| ctx.config.as_ref().and_then(|c| c.read().ok())
        .is_some_and(|c| crate::ua_select::is_autochanger(&c, name))) {
        slot = i64::from(ua.get_pint("Enter slot (0 for none): ")?);
        in_changer = ua.get_yesno("InChanger? yes/no: ")?;
    }
    let names = (start..start + count)
        .map(|i| if num > 0 { format!("{base}{i:04}") } else { base.clone() })
        .collect();
    Some(AddVolumes { pool, pool_id, media_type, names, slot, in_changer })
}

/// CreateMediaRecord for each volume (pool defaults, VolStatus Unlabeled)
/// and the pool's NumVols.
fn add_volumes(ua: &mut crate::ua_select::Ua, volumes: AddVolumes) {
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    for (index, name) in volumes.names.iter().enumerate() {
        let sql = format!(
            "INSERT INTO Media (VolumeName, MediaType, PoolId, VolStatus, Enabled, Slot, InChanger, Recycle, \
             VolRetention, VolUseDuration, MaxVolJobs, MaxVolFiles, MaxVolBytes, RecyclePoolId, ActionOnPurge) \
             SELECT '{}', '{}', PoolId, 'Unlabeled', 1, {}, {}, Recycle, VolRetention, VolUseDuration, \
             MaxVolJobs, MaxVolFiles, MaxVolBytes, RecyclePoolId, ActionOnPurge FROM Pool WHERE PoolId={}",
            name.replace('\'', "''"), volumes.media_type.replace('\'', "''"),
            volumes.slot + index as i64, i32::from(volumes.in_changer), volumes.pool_id);
        if let Err(e) = cat.raw_query(&sql) {
            ua.error_msg(format!("{e}\n"));
            return;
        }
    }
    if let Err(e) = cat.raw_query(&format!("UPDATE Pool SET NumVols=NumVols+{} WHERE PoolId={}",
        volumes.names.len(), volumes.pool_id)) {
        ua.warning_msg(format!("{e}\n"));
        return;
    }
    ua.send_msg(format!("{} Volumes created in pool {}\n", volumes.names.len(), volumes.pool));
}

pub fn cmd_import(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    move_operation(ast, ctx, crate::ua_impexp::MoveOp::Import)
}

pub fn cmd_export(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    move_operation(ast, ctx, crate::ua_impexp::MoveOp::Export)
}

pub fn cmd_move(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    move_operation(ast, ctx, crate::ua_impexp::MoveOp::Move)
}

/// ImportCmd/ExportCmd/move_cmd (ua_impexp.cc:1186): PerformMoveOperation
/// with the Storage daemon (C-069/C-071).
fn move_operation(ast: &CommandAst, ctx: &ExecContext, op: crate::ua_impexp::MoveOp) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    let ok = crate::ua_impexp::move_cmd(&mut ua, ast, op);
    ua.finish(ok)
}

#[cfg(test)]
mod messages_tests {
    use super::*;

    #[test]
    fn messages_preserve_bytes_and_are_consumed_once() {
        let _queue = crate::console_msg::test_queue("extra");
        let ctx = ExecContext { session_id: 0, config: None, session_mgr: None,
            catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        crate::console_msg::append("", "first\n");
        crate::console_msg::append("dt ", "second\r\n");
        let dot = crate::session::parse_cli_command(".messages");
        assert_eq!(crate::commands::dot::cmd_dot_messages(&dot, &ctx).text_output, "first\ndt second\r\n");
        assert_eq!(crate::commands::dot::cmd_dot_messages(&dot, &ctx).text_output, "");
        assert_eq!(cmd_messages(&crate::session::parse_cli_command("messages"), &ctx).text_output, "You have no messages.\n");
        crate::console_msg::append("", "third\n");
        assert_eq!(cmd_messages(&crate::session::parse_cli_command("messages"), &ctx).text_output, "third\n");
        // API modes: no decoration
        let api = ExecContext { api_mode: crate::command_executor::ApiMode::Api, ..ctx };
        assert_eq!(cmd_messages(&crate::session::parse_cli_command("messages"), &api).text_output, "");
    }
}

#[cfg(test)]
mod setdevice_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nStorage { Name = File; Address = localhost; Port = 9103 }"
        ).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    #[test]
    fn setdevice_resource_acl_blocks_network_and_allows_named_storage() {
        let mut ctx = context();
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        ctx.session_id = manager.create_session("limited");
        manager.authenticate_session(ctx.session_id);
        ctx.session_mgr = Some(manager);
        let ast = crate::session::parse_cli_command("setdevice storage=File device=drive0 autoselect=no");
        let denied = setdevice_with(&ast, &ctx, |_,_,_| panic!("denied storage must never contact SD"));
        assert!(!denied.success);
        assert_eq!(denied.error_message, "Access to storage \"File\" forbidden\n");
        let extra = crate::director_config::DirectorConfig::from_str("Director {Name=dir}\nConsole {Name=limited; Password=x; StorageACL=File}").unwrap();
        ctx.config.as_ref().unwrap().write().unwrap().raw.extend(extra.raw.into_iter().filter(|r| r.norm_type() == "console"));
        assert!(setdevice_with(&ast, &ctx, |storage,_,_| {
            assert_eq!(storage.name, "File"); Ok("3000 OK setdevice\n".into())
        }).success);
    }
    #[test]
    fn setdevice_sends_protocol_command_and_preserves_reply() {
        let ast = crate::session::parse_cli_command("setdevice storage=File device=drive0 autoselect=no");
        let result = setdevice_with(&ast, &context(), |storage, director, wire| {
            assert_eq!(storage.name, "File");
            assert_eq!(director, "dir");
            assert_eq!(wire, "setdevice device=drive0 autoselect=0");
            Ok("3000 OK setdevice\n".into())
        });
        assert!(result.success);
        assert_eq!(result.text_output, "Connecting to Storage daemon File at localhost:9103\n3000 OK setdevice\n");
    }

    #[test]
    fn setdevice_does_not_send_invalid_or_incomplete_requests() {
        for line in ["setdevice storage=File", "setdevice storage=File device=x autoselect=maybe",
            "setdevice storage=File device=x autoselect=yes bogus=1", "setdevice storage=Unknown device=x autoselect=yes",
            "setdevice storage=File device=x autoselect=on", "setdevice storage=File device=x autoselect=Yes",
            "setdevice Storage=File device=x autoselect=yes"] {
            let ast = crate::session::parse_cli_command(line);
            let result = setdevice_with(&ast, &context(), |_, _, _| panic!("must not contact SD"));
            assert!(!result.success, "{line}");
        }
    }

    #[test]
    fn setdevice_argument_errors_are_followed_by_usage() {
        let result = setdevice_with(&crate::session::parse_cli_command("setdevice"), &context(), |_, _, _| unreachable!());
        assert_eq!(result.error_message, "Argument missing: autoselect\nArgument missing: device\nArgument missing: storage\n");
        assert_eq!(result.text_output, "\nUSAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>\n");
        assert!(result.errors_first && !result.success);
    }

    #[test]
    fn rerun_argument_errors_use_original_texts() {
        let ctx = context();
        for (line, expected) in [("rerun", "Please specify jobid, since_jobid, hours or days\n"),
            ("rerun jobid=1,abc", "Specified jobid \"abc\" is not valid\n"),
            ("rerun jobid=1 since_jobid=2", "Please specify either jobid or since_jobid (and optionally until_jobid)\n"),
            ("rerun jobid=1 days=2", "Please specify either jobid or a timeframe\n"),
            ("rerun jobid=999999", "rerunning jobid 999999\n")] {
            let result = cmd_rerun(&crate::session::parse_cli_command(line), &ctx);
            assert_eq!((result.success, result.text_output.as_str()), (false, expected), "{line}");
        }
        assert!(is_a_number("-1.5e3") && !is_a_number("") && !is_a_number("1e") && !is_a_number("inf"));
    }
}

#[cfg(test)]
mod move_operation_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn context(extra: &str) -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(&format!(
            "Director {{ Name = dir }}\nStorage {{ Name = File; Address = localhost; Port = 9103 }}\n{extra}"
        )).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    struct NoSd;

    impl crate::ua_storage_select::SdLink for NoSd {
        fn exchange(&mut self, _store: &bareos_config::resources::StorageResource, command: &str)
            -> Result<Vec<String>, String> {
            panic!("{command}: must not contact the SD")
        }
    }

    fn run(line: &str, ctx: &ExecContext) -> CommandResult {
        use crate::ua_impexp::MoveOp;
        let ast = crate::session::parse_cli_command(line);
        let op = match ast.command_name.as_str() {
            "import" => MoveOp::Import,
            "export" => MoveOp::Export,
            _ => MoveOp::Move,
        };
        let mut ua = crate::ua_select::Ua::new(ctx);
        let ok = crate::ua_impexp::move_command(&mut ua, &ast, op, &mut NoSd, &mut crate::ua_impexp::NoCatalog);
        ua.finish(ok)
    }

    #[test]
    fn no_autochanger_means_empty_selection_and_no_sd_command() {
        let ctx = context("");
        for line in ["import", "export", "move", "import srcslots=1", "move srcslot=1 dstslot=2"] {
            let result = run(line, &ctx);
            assert!(!result.success, "{line}");
            // Without console DoPrompt is in batch mode.
            assert_eq!(result.text_output, "The defined Autochanger Storage resources are:\n\
                Your request has multiple choices for \"Storage\". Selection is not possible in batch mode.\n", "{line}");
        }
        // With a console: nothing to ask, so the result carries the text.
        let (result, console) = crate::ua_select::tests::scripted(&[], || run("import", &ctx));
        assert!(!result.success && console.events.is_empty());
        assert_eq!(result.text_output, "The defined Autochanger Storage resources are:\n");
        assert_eq!(result.error_message, "Selection list for \"Storage\" is empty!\n");
        let result = run("import storage=Nope", &ctx);
        assert_eq!(result.error_message, "Storage resource \"Nope\": not found\n");
        assert_eq!(result.text_output, "The defined Autochanger Storage resources are:\nYour request has multiple choices for \"Storage\". Selection is not possible in batch mode.\n");
        assert_eq!(run("export jobid=x", &ctx).error_message, "Expecting jobid=nn command, got: jobid\n");
        assert_eq!(run("move job=Backup", &ctx).error_message, "Job \"Backup\" is not running.\n");
    }


    #[test]
    fn delete_acts_only_on_complete_requests() {
        use crate::ua_select::tests::scripted;
        let ctx = context("Pool { Name = Full; Pool Type = Backup }\nPool { Name = Inc; Pool Type = Backup }");
        let delete = |line: &str, answers: &[&str]| {
            let mut acted = Vec::new();
            let (_, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                delete_dialog(&mut ua, &crate::session::parse_cli_command(line), &mut None, |_, action, _| acted.push(action));
                ua.finish(true);
            });
            (console.transcript(), acted)
        };
        let (text, acted) = delete("delete", &["."]);
        assert_eq!(text, "In general it is not a good idea to delete either a\nPool or a Volume since they may contain data.\n\n\
            You have the following choices:\n1: volume\n2: pool\n3: jobid\n4: storage\n\
            Choose catalog item to delete (1-4): .\nSelection aborted, nothing done.\nNothing done.\n");
        assert!(acted.is_empty());
        let (text, acted) = delete("delete", &["3", "."]);
        assert!(text.ends_with("Choose catalog item to delete (1-4): 3\nEnter JobId to delete: .\n"), "{text}");
        assert!(acted.is_empty());
        assert_eq!(delete("delete jobid", &["x", "12"]).1, vec![DeleteAction::Job(12)]);
        let (text, acted) = delete("delete jobid=3,5-7,x,9-8", &[]);
        assert_eq!(acted, vec![DeleteAction::Job(3), DeleteAction::Job(5), DeleteAction::Job(6), DeleteAction::Job(7)]);
        assert_eq!(text, "");
        let (text, acted) = delete("delete jobid=1-100", &["no"]);
        assert_eq!((text.as_str(), acted.len()), ("Are you sure you want to delete 99 JobIds ? (yes/no): no\n", 0));
        // Without catalog the volume and pool selections fail: nothing is deleted.
        for line in ["delete volume", "delete volume=v1", "delete volume=v1 yes", "delete pool"] {
            assert!(delete(line, &["."]).1.is_empty(), "{line}");
        }
        // Batch mode: no question can be answered.
        let mut ua = crate::ua_select::Ua::new(&ctx);
        delete_dialog(&mut ua, &crate::session::parse_cli_command("delete"), &mut None, |_, _, _| panic!("batch"));
        let result = ua.finish(true);
        assert!(result.text_output.contains("Your request has multiple choices for \"\""), "{}", result.text_output);
    }

    #[test]
    fn delete_result_object_matches_delete_cmd() {
        // DeleteCmd: ObjectStart("deleted") after OpenClientDb; `jobid=`
        // opens the "jobids" array and DoJobDelete adds each JobId string.
        let ctx = context("Pool { Name = Full; Pool Type = Backup }");
        let deleted = |line: &str| {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            let mut deleted = None;
            delete_dialog(&mut ua, &crate::session::parse_cli_command(line), &mut deleted, |_, _, _| {});
            ua.finish(true);
            deleted.map(serde_json::Value::Object)
        };
        assert_eq!(deleted("delete"), Some(serde_json::json!({})));
        assert_eq!(deleted("delete jobid=3,5-7,x,9-8"), Some(serde_json::json!({"jobids": ["3", "5", "6", "7"]})));
        assert_eq!(deleted("delete jobid=x"), Some(serde_json::json!({"jobids": []})));
        assert_eq!(deleted("delete storage=nosuch"), Some(serde_json::json!({})));
    }

    #[test]
    fn create_and_add_never_default_the_pool() {
        use crate::ua_select::tests::scripted;
        let ctx = context("Pool { Name = Full; Pool Type = Backup }\nPool { Name = Inc; Pool Type = Backup }");
        let create = |line: &str, answers: &[&str]| {
            let mut created = None;
            let (_, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                create_dialog(&mut ua, &crate::session::parse_cli_command(line), |_, pool| created = Some(pool.name));
                ua.finish(true);
            });
            (console.transcript(), created)
        };
        assert_eq!(create("create", &["."]), ("The defined Pool resources are:\n1: Full\n2: Inc\n\
            Select Pool resource (1-2): .\nSelection aborted, nothing done.\n".to_string(), None));
        assert_eq!(create("create pool=Nope", &["2"]), ("Error: Pool resource \"Nope\" does not exist.\n\
            The defined Pool resources are:\n1: Full\n2: Inc\nSelect Pool resource (1-2): 2\n".to_string(), Some("Inc".to_string())));
        assert_eq!(create("create pool=Full", &[]).1, Some("Full".to_string()));
        // add: no catalog -> no pool list, nothing created.
        let (_, console) = scripted(&["."], || {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            add_dialog(&mut ua, &crate::session::parse_cli_command("add pool=Full"), |_, _| panic!("no pool record"));
            ua.finish(true)
        });
        assert!(console.answers.len() == 1, "nothing asked without catalog");
    }

}

#[cfg(test)]
mod setip_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    #[test]
    fn setip_never_changes_a_client_address() {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nClient { Name = fd1; Address = 192.0.2.1; Password = x }\n\
             Console { Name = fd1; Password = pw; CommandACL = *all*; ClientACL = *all* }\n\
             Console { Name = other; Password = pw; CommandACL = *all*; ClientACL = *all* }").unwrap();
        let mut ctx = ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let ast = crate::session::parse_cli_command("setip");
        assert_eq!(cmd_setip(&ast, &ctx).error_message, "No corresponding client found.\n");
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        for (console, expected) in [("other", "Client \"other\" not found.\n"),
            ("fd1", "Client \"fd1\": address of the console connection unknown, nothing changed.\n")] {
            ctx.session_id = manager.create_session(console);
            manager.authenticate_session(ctx.session_id);
            ctx.session_mgr = Some(manager.clone());
            let result = cmd_setip(&ast, &ctx);
            assert!(!result.success);
            assert_eq!(result.error_message, expected);
        }
        let config = ctx.config.as_ref().unwrap().read().unwrap();
        assert_eq!(config.find_client("fd1").unwrap().address, "192.0.2.1");
    }
}

#[cfg(test)]
mod trace_command_tests {
    use super::*;

    #[test]
    fn trace_without_argument_asks_and_cancel_changes_nothing() {
        let ctx = ExecContext { session_id: 0, config: None, session_mgr: None,
            catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let before = bareos_bnet::debug::trace_enabled();
        for line in ["trace", "trace on off"] {
            // Batch mode (no console): no question, no change, no output.
            let result = cmd_trace(&crate::session::parse_cli_command(line), &ctx);
            assert!(result.success && result.text_output.is_empty() && result.error_message.is_empty());
            let (result, console) = crate::ua_select::tests::scripted(&["."], || {
                cmd_trace(&crate::session::parse_cli_command(line), &ctx)
            });
            assert!(result.success);
            assert_eq!(console.transcript(), "Turn on or off? .\n");
            assert_eq!(console.signals(), vec![bareos_bnet::BnetSignal::SubPrompt]);
        }
        assert_eq!(bareos_bnet::debug::trace_enabled(), before);
    }
}

#[cfg(test)]
mod toggle_tests {
    use super::*;
    use crate::session::parse_cli_command;
    use std::sync::Arc;

    fn context() -> ExecContext {
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16))));
        let session_id = manager.create_session("*UserAgent*");
        ExecContext { session_id, config: None, session_mgr: Some(manager), catalog: None,
            api_mode: crate::command_executor::ApiMode::Human, bus_rx: None, job_manager: None }
    }

    #[test]
    fn on_off_keyword_follows_find_arg_keyword() {
        let kw = |line: &str| on_off_keyword(&parse_cli_command(line));
        assert_eq!(kw("gui on"), Some(true));
        assert_eq!(kw("gui OFF"), Some(false));
        assert_eq!(kw("gui foo off"), Some(false));
        assert_eq!(kw("gui on=1"), Some(true));
        assert_eq!(kw("gui off on"), Some(false));
        assert_eq!(kw("gui"), None);
        assert_eq!(kw("gui foo"), None);
        assert_eq!(kw("gui x=on"), None);
    }

    #[test]
    fn autodisplay_and_gui_are_silent_or_report_missing_keyword() {
        let ctx = context();
        let handlers: [(fn(&CommandAst, &ExecContext) -> CommandResult, &str); 2] =
            [(cmd_autodisplay, "autodisplay"), (cmd_gui, "gui")];
        for (handler, name) in handlers {
            for arg in ["on", "off", "On"] {
                let result = handler(&parse_cli_command(&format!("{name} {arg}")), &ctx);
                assert!(result.success);
                assert_eq!(result.text_output, "");
                assert_eq!(result.error_message, "");
            }
            for line in [name.to_string(), format!("{name} foo")] {
                let result = handler(&parse_cli_command(&line), &ctx);
                assert!(result.success, "{line}: C++ handler returns true");
                assert_eq!(result.text_output, "");
                assert_eq!(result.error_message, "ON or OFF keyword missing.\n");
            }
        }
    }

    #[test]
    fn automount_is_silent_and_sets_session_flag() {
        let ctx = context();
        let mgr = ctx.session_mgr.clone().unwrap();
        let run = |line: &str| cmd_automount(&parse_cli_command(line), &ctx);
        assert!(mgr.session_automount(ctx.session_id));
        let result = run("automount OFF");
        assert!(result.success);
        assert_eq!((result.text_output.as_str(), result.error_message.as_str()), ("", ""));
        assert!(!mgr.session_automount(ctx.session_id));
        run("automount foo");
        assert!(mgr.session_automount(ctx.session_id), "anything but off means on");
        run("automount off");
        run("automount");
        run("automount on off");
        assert!(!mgr.session_automount(ctx.session_id), "wrong argument count leaves the flag");
        run("automount off=1");
        assert!(!mgr.session_automount(ctx.session_id));
        run("automount on");
        assert!(mgr.session_automount(ctx.session_id));
    }
}

#[cfg(test)]
mod whoami_tests {
    use super::*;
    use std::sync::Arc;

    fn whoami(console: &str) -> CommandResult {
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16),
        )));
        let id = manager.create_session(console);
        let ctx = ExecContext { session_id: id, config: None, session_mgr: Some(manager),
            catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        cmd_whoami(&crate::session::parse_cli_command("whoami"), &ctx)
    }

    #[test]
    fn default_console_is_root_and_named_console_is_its_name() {
        let root = whoami("*UserAgent*");
        assert_eq!(root.text_output, "root\n");
        assert_eq!(root.json, Some(serde_json::json!({"whoami": "root"})));
        let named = whoami("operator");
        assert_eq!(named.text_output, "operator\n");
        assert_eq!(named.json, Some(serde_json::json!({"whoami": "operator"})));
    }
}

#[cfg(test)]
mod rerun_tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::{Arc, RwLock};

    thread_local! {
        /// Requests the start double received (`key=value` per argument).
        static STARTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn line_of(ast: &CommandAst) -> String {
        ast.arguments.iter()
            .map(|a| a.value.as_ref().map_or(a.key.clone(), |v| format!("{}={v}", a.key)))
            .collect::<Vec<_>>().join(" ")
    }

    /// Job manager double: records the confirmed run instead of starting it.
    fn start_double(ast: &CommandAst, _: &ExecContext, _: bareos_config::resources::JobResource) -> CommandResult {
        STARTED.with(|s| s.borrow_mut().push(line_of(ast)));
        CommandResult::ok(String::new())
    }

    fn started() -> Vec<String> {
        STARTED.with(|s| std::mem::take(&mut *s.borrow_mut()))
    }

    fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nClient { Name = fd; Address = localhost; Password = x }\n\
             Storage { Name = File; Address = localhost; Password = x; Device = d; Media Type = File }\n\
             FileSet { Name = fs; Include { File = /etc } }\nPool { Name = Full; Pool Type = Backup }\n\
             Pool { Name = Copy; Pool Type = Backup; Storage = File }\n\
             Job { Name = b1; Type = Backup; Level = Incremental; Client = fd; FileSet = fs; Storage = File; Pool = Full; Messages = m }\n\
             Job { Name = c1; Type = Copy; Client = fd; FileSet = fs; Storage = File; Pool = Full; Messages = m }"
        ).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    /// Catalog double: failed jobs 5, 7 and 9; Job records 5 (Backup), 7
    /// (Copy), 8 (Restore), 9 (VirtualFull without client and fileset),
    /// 10 (Copy of a job without resource), 11 (client record missing).
    fn catalog(sql: &str) -> Result<Rows, String> {
        let row = |cells: &[&str]| vec![cells.iter().map(|c| Some(c.to_string())).collect::<Vec<_>>()];
        Ok(match sql {
            s if s.starts_with("SELECT JobId FROM Job WHERE JobStatus = 'f'") => {
                if s.contains(">= 100") { vec![] } else { [row(&["5"]), row(&["7"]), row(&["9"])].concat() }
            }
            s if s.ends_with("WHERE JobId=5") => row(&["b1", "B", "F", "1", "2", "3"]),
            s if s.ends_with("WHERE JobId=7") => row(&["c1", "c", "F", "1", "4", "3"]),
            s if s.ends_with("WHERE JobId=8") => row(&["r1", "R", " ", "1", "0", "0"]),
            s if s.ends_with("WHERE JobId=9") => row(&["b1", "B", "f", "0", "4", "0"]),
            s if s.ends_with("WHERE JobId=10") => row(&["gone", "g", "I", "0", "4", "0"]),
            s if s.ends_with("WHERE JobId=11") => row(&["b1", "B", "D", "99", "2", "3"]),
            "SELECT Name FROM Client WHERE ClientId=1" => row(&["fd"]),
            "SELECT Name FROM Pool WHERE PoolId=2" => row(&["Full"]),
            "SELECT Name FROM Pool WHERE PoolId=4" => row(&["Copy"]),
            "SELECT FileSet FROM FileSet WHERE FileSetId=3" => row(&["fs"]),
            _ => vec![],
        })
    }

    const NOW: i64 = 1_760_000_000;

    /// Runs `line` with scripted answers: (success, transcript + rest).
    fn rerun(line: &str, answers: &[&str]) -> (bool, String) {
        let ctx = context();
        let ((ok, text), console) = crate::ua_select::tests::scripted(answers, || {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            let ok = rerun_dialog(&mut ua, &crate::session::parse_cli_command(line), &catalog, NOW, start_double);
            let result = ua.finish(ok);
            (result.success, result.text_output)
        });
        (ok, console.transcript() + &text)
    }

    #[test]
    fn arguments_follow_get_rerun_cmdline_arguments() {
        let ctx = context();
        let parse = |line: &str| {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            let args = rerun_arguments(&mut ua, &crate::session::parse_cli_command(line));
            (args, ua.finish(true).text_output)
        };
        assert_eq!(parse("rerun since_jobid=12abc until_jobid=+20 yes").0.unwrap(),
            RerunArguments { since_jobid: 12, until_jobid: 20, yes: true, ..Default::default() });
        // std::getline: nothing after a trailing comma, nothing for "".
        assert_eq!(parse("rerun jobid=3,4,").0.unwrap().jobids, vec![3, 4]);
        assert_eq!(parse("rerun jobid=").0.unwrap().jobids, Vec::<u32>::new());
        assert_eq!(parse("rerun jobid=-1,2.7").0.unwrap().jobids, vec![u32::MAX, 2]);
        assert_eq!(parse("rerun jobid=,1").1, "Specified jobid \"\" is not valid\n");
        assert_eq!(parse("rerun since_jobid=abc days=0").1, "Please specify jobid, since_jobid, hours or days\n");
        assert_eq!(parse("rerun hours=x until_jobid=5").1, "Please specify jobid, since_jobid, hours or days\n");
        assert_eq!(parse("rerun jobid=1 hours=2").1, "Please specify either jobid or a timeframe\n");
        assert_eq!(parse("rerun days=4294967297").0.unwrap().days, 1, "C int");
    }

    #[test]
    fn selection_query_matches_prepare_rerun_sql_query() {
        let args = |since, until, days, hours| RerunArguments { since_jobid: since, until_jobid: until, days, hours, ..Default::default() };
        assert_eq!(rerun_sql(&args(10, 0, 1, 0), NOW), "SELECT JobId FROM Job WHERE JobStatus = 'f' AND JobId >= 10 ORDER BY JobId");
        assert_eq!(rerun_sql(&args(10, 20, 0, 0), NOW),
            "SELECT JobId FROM Job WHERE JobStatus = 'f' AND JobId >= 10 AND JobId <= 20 ORDER BY JobId");
        let at = crate::localtime::strftime("%Y-%m-%d %H:%M:%S", (NOW - 2 * 86400 - 3 * 3600) as libc::time_t);
        assert_eq!(rerun_sql(&args(0, 20, 2, 3), NOW),
            format!("SELECT JobId FROM Job WHERE JobStatus = 'f' AND SchedTime > '{at}' ORDER BY JobId"));
    }

    #[test]
    fn selection_lists_and_asks_without_starting() {
        started();
        assert_eq!(rerun("rerun since_jobid=100", &[]), (true, "No jobids with the specified options were found.\n".into()));
        // The console closes at the question: GetYesno fails.
        let (ok, text) = rerun("rerun days=1", &[]);
        assert!(!ok);
        assert_eq!(text, "The following ids were selected for rerun:\n5,7,9\nrerun these jobids? (yes/no): ");
        // Without a console GetCmd fails at once.
        let ctx = context();
        let result = cmd_rerun(&crate::session::parse_cli_command("rerun days=1"), &ctx);
        assert_eq!((result.success, result.text_output.as_str()), (true, "No jobids with the specified options were found.\n"),
            "no catalog: the failed query selects nothing");
        for answers in [&["no"][..], &["."], &["maybe", "nope", "", "n"]] {
            let (ok, text) = rerun("rerun hours=3", answers);
            assert!(!ok, "{answers:?}");
            assert!(!text.contains("rerunning"), "{answers:?}: {text}");
        }
        let (_, text) = rerun("rerun hours=3", &["may", "no"]);
        assert!(text.ends_with("(yes/no): may\nInvalid response. You must answer yes or no.\n\
            rerun these jobids? (yes/no): no\n"), "{text}");
        // "yes" to the selection, then the run dialog of the first job: "no".
        let (ok, text) = rerun("rerun since_jobid=1", &["yes", "no"]);
        assert!(!ok);
        assert!(text.contains("(yes/no): yes\nrerunning jobid 5\nRun Backup job\nJobName:  b1\nLevel:    Full\n"), "{text}");
        assert!(text.ends_with("OK to run? (yes/mod/no): no\nJob not run.\n"), "{text}");
        assert!(started().is_empty());
    }

    #[test]
    fn yes_reruns_every_selected_job() {
        started();
        let (ok, text) = rerun("rerun since_jobid=1 yes", &[]);
        assert!(ok, "{text}");
        assert_eq!(text, "The following ids were selected for rerun:\n5,7,9\n\
            rerunning jobid 5\nrerunning jobid 7\nrerunning jobid 9\n");
        let started = started();
        assert_eq!(started.len(), 3, "{started:?}");
        assert!(started[0].starts_with("job=b1 level=Full client=fd fileset=fs"), "{started:?}");
        assert!(started[1].starts_with("job=c1 level=Full client=fd fileset=fs"), "{started:?}");
        assert!(started[2].starts_with("job=b1 level=VirtualFull"), "{started:?}");
    }

    #[test]
    fn rerun_command_carries_the_historic_parameters() {
        let ctx = context();
        let when = "2025-10-09 08:53:20";
        let line = |jobid, yes| rerun_command(&ctx, jobid, yes, when, &catalog).map(|ast| ast.as_ref().map(line_of));
        assert_eq!(line(5, false), Ok(Some(format!("job=b1 level=Full client=fd pool=Full fileset=fs when={when}"))));
        assert_eq!(line(7, true), Ok(Some(format!("job=c1 level=Full client=fd pool=Full nextpool=Copy fileset=fs when={when} yes"))));
        assert_eq!(line(9, false), Ok(Some(format!("job=b1 level=VirtualFull pool=Full nextpool=Copy when={when}"))));
        assert_eq!(line(10, false), Ok(Some(format!("job=gone level=Incremental nextpool=Copy when={when}"))));
        assert_eq!(line(8, true), Ok(None), "Restore jobs are skipped");
        assert_eq!(line(6, true), Err(()), "missing Job record");
        assert_eq!(line(11, true), Err(()), "missing Client record");
        assert_eq!(job_level_to_string(b'd'), "DiskToCatalog");
        assert_eq!(job_level_to_string(b'x'), "x (120)");
    }

    #[test]
    fn jobid_list_runs_in_order_and_stops_at_a_failure() {
        started();
        let (ok, text) = rerun("rerun jobid=8,5,6,7 yes", &[]);
        assert!(!ok);
        assert_eq!(text, "rerunning jobid 8\nrerunning jobid 5\nrerunning jobid 6\n");
        assert_eq!(started().len(), 1);
    }
}
