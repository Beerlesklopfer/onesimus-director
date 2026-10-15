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

/// DoMessages sends queued text verbatim; dot mode is silent when empty.
pub fn cmd_messages(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut output = String::new();
    if let Some(rx) = &ctx.bus_rx {
        while let Ok(message) = rx.try_recv() {
            let text = match message {
                crate::message_bus::BusMessage::ConsoleText { text }
                | crate::message_bus::BusMessage::JobMessage { text, .. } => text,
                _ => continue,
            };
            output.push_str(&text);
        }
    }
    if output.is_empty() && ast.command_name != ".messages"
        && ctx.api_mode == crate::command_executor::ApiMode::Human {
        output.push_str("You have no messages.\n");
    }
    CommandResult::ok(output)
}

/// C++ ua_cmds.cc MemoryCmd: prints ListDirStatusHeader.
pub fn cmd_memory(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::commands::status::dir_status_header(ctx)
}

pub fn cmd_show(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let what = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("all");

    if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            let mut output = String::new();
            match what {
                "director" | "dir" => {
                    output.push_str(&format!("Director: name={} port={} maxjobs={}\n",
                        cfg.director.name, cfg.director.port, cfg.director.max_concurrent_jobs));
                }
                "clients" | "client" => {
                    for c in &cfg.clients {
                        output.push_str(&format!("Client: name={} address={} port={} enabled={}\n",
                            c.name, c.address, c.port, c.enabled));
                    }
                }
                "jobs" | "job" => {
                    for j in &cfg.jobs {
                        output.push_str(&format!("Job {{\n  Name = \"{}\"\n  Type = {:?}\n  Level = {:?}\n  Client = {}\n  FileSet = \"{}\"\n  Storage = {}\n  Pool = {}\n  Messages = {}\n  Priority = {}\n",
                            j.name, j.job_type, j.level, j.client, j.fileset, j.storage, j.pool, j.messages, j.priority));
                        if !j.schedule.is_empty() { output.push_str(&format!("  Schedule = \"{}\"\n", j.schedule)); }
                        if !j.full_backup_pool.is_empty() { output.push_str(&format!("  Full Backup Pool = {}\n", j.full_backup_pool)); }
                        if !j.differential_backup_pool.is_empty() { output.push_str(&format!("  Differential Backup Pool = {}\n", j.differential_backup_pool)); }
                        if !j.incremental_backup_pool.is_empty() { output.push_str(&format!("  Incremental Backup Pool = {}\n", j.incremental_backup_pool)); }
                        if !j.write_bootstrap.is_empty() { output.push_str(&format!("  Write Bootstrap = \"{}\"\n", j.write_bootstrap)); }
                        output.push_str(&format!("  Enabled = {}\n}}\n\n", if j.enabled { "yes" } else { "no" }));
                    }
                }
                "jobdefs" | "jobdef" => {
                    for jd in &cfg.jobdefs {
                        let j = &jd.job;
                        output.push_str(&format!("JobDefs {{\n  Name = \"{}\"\n  Type = {:?}\n  Level = {:?}\n  Client = {}\n  FileSet = \"{}\"\n  Storage = {}\n  Pool = {}\n  Messages = {}\n  Priority = {}\n",
                            jd.name, j.job_type, j.level, j.client, j.fileset, j.storage, j.pool, j.messages, j.priority));
                        if !j.schedule.is_empty() { output.push_str(&format!("  Schedule = \"{}\"\n", j.schedule)); }
                        if !j.full_backup_pool.is_empty() { output.push_str(&format!("  Full Backup Pool = {}\n", j.full_backup_pool)); }
                        if !j.differential_backup_pool.is_empty() { output.push_str(&format!("  Differential Backup Pool = {}\n", j.differential_backup_pool)); }
                        if !j.incremental_backup_pool.is_empty() { output.push_str(&format!("  Incremental Backup Pool = {}\n", j.incremental_backup_pool)); }
                        if !j.write_bootstrap.is_empty() { output.push_str(&format!("  Write Bootstrap = \"{}\"\n", j.write_bootstrap)); }
                        output.push_str(&format!("  Enabled = {}\n}}\n\n", if j.enabled { "yes" } else { "no" }));
                    }
                }
                "storages" | "storage" => {
                    for s in &cfg.storages {
                        output.push_str(&format!("Storage: name={} address={} port={} device={} mediatype={}\n",
                            s.name, s.address, s.port, s.device, s.media_type));
                    }
                }
                "pools" | "pool" => {
                    for p in &cfg.pools {
                        output.push_str(&format!("Pool: name={} type={} recycle={} maxvols={}\n",
                            p.name, p.pool_type, p.recycle, p.maximum_volumes));
                    }
                }
                "all" => {
                    output.push_str(&format!("Director: {}\n", cfg.director.name));
                    output.push_str(&format!("Clients: {}\n", cfg.clients.len()));
                    output.push_str(&format!("Jobs: {}\n", cfg.jobs.len()));
                    output.push_str(&format!("JobDefs: {}\n", cfg.jobdefs.len()));
                    output.push_str(&format!("Storages: {}\n", cfg.storages.len()));
                    output.push_str(&format!("Pools: {}\n", cfg.pools.len()));
                    output.push_str(&format!("FileSets: {}\n", cfg.filesets.len()));
                    output.push_str(&format!("Schedules: {}\n", cfg.schedules.len()));
                    output.push_str(&format!("Consoles: {}\n", cfg.consoles.len()));
                }
                other => output.push_str(&format!("show {other}: try director, clients, jobs, jobdefs, storages, pools, all\n")),
            }
            return CommandResult::ok(output);
        }
    }
    CommandResult::ok("No config\n".to_string())
}

pub fn cmd_trace(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if ast.arguments.len() != 1 || ast.arguments[0].value.is_some() {
        return CommandResult::error("trace: specify on or off\n".into());
    }
    let enabled = match ast.arguments[0].key.to_ascii_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => return CommandResult::error("trace: specify on or off\n".into()),
    };
    let path = trace_path(ctx);
    match bareos_bnet::debug::set_trace(&path, enabled) {
        Ok(()) => CommandResult::ok(String::new()),
        Err(error) => CommandResult::error(format!("trace: {}: {error}\n", path.display())),
    }
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
    setdevice_with(ast, ctx, |storage, director, command| {
        bareos_stored::command::sd_command(&storage.address, storage.port,
            &storage.password, director, command)
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
    let value = |key: &str| ast.arguments.iter().find(|a| a.key == key)
        .and_then(|a| a.value.as_deref()).filter(|s| !s.is_empty());
    let mut missing = String::new();
    for key in ["autoselect", "device", "storage"] {
        if value(key).is_none() { missing.push_str(&format!("Argument missing: {key}\n")); }
    }
    if !missing.is_empty() { return usage(missing); }
    let autoselect = match value("autoselect").unwrap().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => 1,
        "no" | "false" | "off" | "0" => 0,
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
    let Some(storage) = config.find_storage(name).cloned() else {
        return CommandResult::error(format!("Storage \"{name}\" not found.\n"));
    };
    let director = config.director.name.clone();
    drop(config);
    if !crate::acl::access(ctx, crate::acl::Kind::Storage, &storage.name) {
        return CommandResult::error(format!("Storage \"{}\" not found.\n", storage.name));
    }
    let command = format!("setdevice device={device} autoselect={autoselect}");
    match send(&storage, &director, &command) {
        Ok(response) => CommandResult::ok(format!("Connecting to Storage daemon {} at {}:{}\n{response}",
            storage.name, storage.address, storage.port)),
        Err(error) => CommandResult::error(format!("Failed to connect to Storage daemon.\n{error}\n")),
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

/// ua_run.cc reRunCmd()/GetRerunCmdlineArguments(): argument errors are
/// SendMsg lines and make the command fail.
pub fn cmd_rerun(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_run.cc reRunCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_rerun_db(ast, ctx))
}

fn cmd_rerun_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let value = |key: &str| ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref());
    let number = |key: &str| value(key).map(|v| v.trim().parse::<i64>().unwrap_or(0)).unwrap_or(0);
    let failed = |text: &str| CommandResult { success: false, text_output: text.to_string(), ..Default::default() };
    let (since_jobid, timeframe) = (number("since_jobid"), number("days") != 0 || number("hours") != 0);
    let Some(jobids) = value("jobid") else {
        if !timeframe && since_jobid == 0 {
            return failed("Please specify jobid, since_jobid, hours or days\n");
        }
        return CommandResult::ok("rerun: since_jobid, days and hours are not supported yet\n".to_string());
    };
    let mut ids = Vec::new();
    for jobid in jobids.split(',') {
        if !is_a_number(jobid) {
            return failed(&format!("Specified jobid \"{jobid}\" is not valid\n"));
        }
        ids.push(jobid.parse::<i64>().unwrap_or(0));
    }
    if since_jobid != 0 {
        return failed("Please specify either jobid or since_jobid (and optionally until_jobid)\n");
    }
    if timeframe {
        return failed("Please specify either jobid or a timeframe\n");
    }

    let mut text = String::new();
    for id in ids {
        // reRunJob(): announce, then look up the Job record.
        text.push_str(&format!("rerunning jobid {id}\n"));
        let Some(cat) = &ctx.catalog else { return failed(&text) };
        let job_name = match cat.raw_query(&format!("SELECT name FROM job WHERE jobid = {id}")) {
            Ok(result) => result.lines().nth(2).map(|l| l.trim().to_string()).unwrap_or_default(),
            Err(e) => return failed(&format!("{text}Catalog error: {e}\n")),
        };
        // C++ reports a missing record only as a job message (Jmsg M_WARNING).
        if job_name.is_empty() { return failed(&text); }
        // reRunJob (ua_run.cc:71) builds "run job=<name> ..." and appends
        // " yes" only when the rerun itself had it (ua_run.cc:200); without
        // it the run asks "OK to run?" (C-012).
        let mut arguments = vec![crate::command_ast::Argument { key: "job".to_string(), value: Some(job_name) }];
        // The record's level, client, pool and fileset (ua_run.cc:97-193).
        let record = cat.query_rows(&format!("SELECT j.Level, c.Name, p.Name, f.FileSet FROM Job j \
            LEFT JOIN Client c ON c.ClientId=j.ClientId LEFT JOIN Pool p ON p.PoolId=j.PoolId \
            LEFT JOIN FileSet f ON f.FileSetId=j.FileSetId WHERE j.JobId={id}")).ok()
            .and_then(|t| t.rows.into_iter().next()).unwrap_or_default();
        let level = match record.first().cloned().flatten().as_deref() {
            Some("F") => Some("Full".to_string()), Some("I") => Some("Incremental".to_string()),
            Some("D") => Some("Differential".to_string()), _ => None,
        };
        for (key, value) in [("level", level), ("client", record.get(1).cloned().flatten()),
            ("pool", record.get(2).cloned().flatten()), ("fileset", record.get(3).cloned().flatten())] {
            if let Some(value) = value {
                arguments.push(crate::command_ast::Argument { key: key.to_string(), value: Some(value) });
            }
        }
        if ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes")) {
            arguments.push(crate::command_ast::Argument { key: "yes".to_string(), value: None });
        }
        let run_ast = crate::command_ast::CommandAst {
            command_name: "run".to_string(),
            arguments,
            ..Default::default()
        };
        let mut run = crate::commands::job::cmd_run(&run_ast, ctx);
        run.text_output = format!("{text}{}", run.text_output);
        if !run.success { return run; }
        text = run.text_output;
    }
    CommandResult::ok(text)
}

pub fn cmd_relabel(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_label.cc do_label: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_relabel_db(ast, ctx))
}

fn cmd_relabel_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    relabel_with(ast, ctx, |stor, command| sd_cmd_via_config(ctx, stor, command))
}

/// do_label(relabel=true) (ua_label.cc:393): storage, old volume, new name
/// and pool are selected, never defaulted; the old volume must be Purged or
/// Recycled (ua_label.cc:470). C-012.
fn relabel_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    send: impl FnOnce(&str, &str) -> Result<String, String>,
) -> CommandResult {
    use super::prompt::{select_pool_dbr, select_storage, selection_required};
    let value = |key: &str| ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref());
    let (store, mut shown) = match select_storage(ast, ctx, false) {
        Ok(selected) => selected,
        Err(result) => return result,
    };
    let stor = store.name.as_str();
    let Some(oldvol) = value("oldvolume") else {
        // SelectMediaDbr (ua_select.cc:918): pool, media list, then
        // GetCmd 'E.g. "full-0001" or "*42": '.
        if value("volume").is_some() {
            return crate::ua_db::with_prefix(shown, selection_required(String::new()));
        }
        return match select_pool_dbr(ctx) {
            Ok((_, text)) => selection_required(format!("{shown}{text}Enter the volume name or MediaId \
                of the volume prefixed with an asterisk (*).\nE.g. \"full-0001\" or \"*42\": ")),
            Err(result) => crate::ua_db::with_prefix(shown, result),
        };
    };
    if let Some(cat) = &ctx.catalog {
        let status = cat.query_rows(&format!("SELECT VolStatus FROM Media WHERE VolumeName='{}'",
            oldvol.replace('\'', "''"))).ok()
            .and_then(|t| t.rows.into_iter().next()).and_then(|row| row.into_iter().next().flatten());
        match status {
            None => {
                // GetMediaRecord fails (ua_label.cc:462), then SelectMediaDbr
                // tries volume= the same way before asking.
                let missing = |name: &str| format!("cats/sql_get.cc:1177 Media record for Volume \"{name}\" not found.\n");
                let mut errors = missing(oldvol);
                if let Some(newname) = value("volume") {
                    let exists = cat.query_rows(&format!("SELECT MediaId FROM Media WHERE VolumeName='{}'",
                        newname.replace('\'', "''"))).is_ok_and(|t| !t.rows.is_empty());
                    if exists {
                        // That record would become the volume to relabel: ask.
                        return crate::ua_db::with_prefix(shown, selection_required(String::new()));
                    }
                    errors.push_str(&missing(newname));
                    let mut result = CommandResult::error(errors);
                    result.text_output = shown;
                    return result;
                }
                // No volume=: SelectMediaDbr asks for pool and volume.
                let mut result = match select_pool_dbr(ctx) {
                    Ok((_, text)) => selection_required(format!("{text}Enter the volume name or MediaId \
                        of the volume prefixed with an asterisk (*).\nE.g. \"full-0001\" or \"*42\": ")),
                    Err(result) => result,
                };
                result.text_output.insert_str(0, &format!("{shown}{errors}"));
                return result;
            }
            Some(status) if status != "Purged" && status != "Recycle" => {
                return crate::ua_db::with_prefix(shown, CommandResult::error(format!(
                    "Volume \"{oldvol}\" has VolStatus {status}. It must be Purged or Recycled before relabeling.\n")));
            }
            Some(_) => {}
        }
    }
    let Some(newname) = value("volume") else {
        return selection_required(format!("{shown}Enter new Volume name: "));
    };
    if value("pool").is_none() {
        match select_pool_dbr(ctx) {
            Ok((_, text)) => shown.push_str(&text),
            Err(result) => return crate::ua_db::with_prefix(shown, result),
        }
    }

    match send(stor, &format!("relabel OldVolume=\"{oldvol}\" Volume=\"{newname}\"")) {
        Ok(resp) => {
            if let Some(cat) = &ctx.catalog {
                let _ = cat.raw_query(&format!("UPDATE media SET volumename='{}' WHERE volumename='{}'",
                    newname.replace('\'', "''"), oldvol.replace('\'', "''")));
            }
            CommandResult::ok(format!("{shown}Relabel {oldvol} → {newname}:\n{resp}\n"))
        }
        Err(e) => crate::ua_db::with_prefix(shown, CommandResult::error(format!("relabel: {e}\n"))),
    }
}

pub fn cmd_delete(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc DeleteCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_delete_db(ast, ctx))
}

fn cmd_delete_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };
    match delete_request(ast) {
        DeleteRequest::Dialog(result) => result,
        DeleteRequest::Volume(vol) => {
            let vol = vol.replace('\'', "''");
            // Delete jobmedia first (FK), then media record
            let _ = cat.raw_query(&format!(
                "DELETE FROM jobmedia WHERE mediaid IN (SELECT mediaid FROM media WHERE volumename = '{vol}')"));
            match cat.raw_query(&format!("DELETE FROM media WHERE volumename = '{vol}'")) {
                Ok(_) => CommandResult::ok(format!("Volume {vol} deleted from catalog\n")),
                Err(e) => CommandResult::error(format!("Delete error: {e}\n")),
            }
        }
        DeleteRequest::Jobs(ids, errors) => {
            let mut output = String::new();
            for id in ids {
                let _ = cat.raw_query(&format!("DELETE FROM jobmedia WHERE jobid = {id}"));
                let _ = cat.raw_query(&format!("DELETE FROM file WHERE jobid = {id}"));
                match cat.raw_query(&format!("DELETE FROM job WHERE jobid = {id}")) {
                    Ok(_) => output.push_str(&format!("Jobid {id} and associated records deleted from the catalog.\n")),
                    Err(e) => return CommandResult::error(format!("Delete error: {e}\n")),
                }
            }
            let mut result = CommandResult::ok(output);
            result.error_message = errors;
            result
        }
        DeleteRequest::Unsupported => CommandResult::ok("delete: specify volume=<name> or jobid=<id>\n".to_string()),
    }
}

enum DeleteRequest {
    Dialog(CommandResult),
    Volume(String),
    Jobs(Vec<u64>, String),
    Unsupported,
}

/// DeleteCmd (ua_cmds.cc:2220) argument handling: what may be deleted
/// without console interaction. Missing values, a volume without `yes` and
/// ranges of more than 25 JobIds end at the dialog hook (C-012).
fn delete_request(ast: &CommandAst) -> DeleteRequest {
    use super::prompt::{keyword_prompt, selection_required};
    const KEYWORDS: [&str; 4] = ["volume", "pool", "jobid", "storage"];
    let keyword = ast.arguments.iter().find_map(|a|
        KEYWORDS.iter().position(|k| a.key.eq_ignore_ascii_case(k)));
    let value = |key: &str| ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref());
    match keyword {
        None => DeleteRequest::Dialog(keyword_prompt(
            "In general it is not a good idea to delete either a\n\
             Pool or a Volume since they may contain data.\n\n", &KEYWORDS, "Choose catalog item to delete")),
        Some(0) => match value("volume") {
            // DeleteVolume → SelectMediaDbr (ua_select.cc:918).
            None => DeleteRequest::Dialog(selection_required(String::new())),
            Some(vol) if ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes")) =>
                DeleteRequest::Volume(vol.to_string()),
            // ua_cmds.cc:2433-2441 warning, then GetYesno.
            Some(vol) => DeleteRequest::Dialog(selection_required(format!(
                "\nThis command will delete volume {vol}\nand all Jobs saved on that volume from the Catalog\n\
                 Are you sure you want to delete Volume \"{vol}\"? (yes/no): "))),
        },
        Some(2) => {
            // DeleteJob (ua_cmds.cc:2311): GetPint without a value.
            let Some(list) = value("jobid") else {
                return DeleteRequest::Dialog(selection_required("Enter JobId to delete: ".into()));
            };
            let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
            let (mut ids, mut errors) = (Vec::new(), String::new());
            for token in list.split(',') {
                match token.split_once('-') {
                    // DeleteJobIdRange (ua_cmds.cc:2376).
                    Some((a, b)) if number(a) && number(b) => {
                        let (a, b) = (a.parse::<u64>().unwrap_or(0), b.parse::<u64>().unwrap_or(0));
                        if b > a {
                            if b - a > 25 {
                                return DeleteRequest::Dialog(selection_required(format!(
                                    "Are you sure you want to delete {} JobIds ? (yes/no): ", b - a)));
                            }
                            ids.extend(a..=b);
                        }
                    }
                    _ if number(token) => ids.push(token.parse().unwrap_or(0)),
                    _ => errors.push_str(&format!("Illegal JobId {token} ignored\n")),
                }
            }
            DeleteRequest::Jobs(ids, errors)
        }
        Some(_) => DeleteRequest::Unsupported,
    }
}

pub fn cmd_create(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc CreateCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_create_db(ast, ctx))
}

fn cmd_create_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let pool_name = ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case("pool"))
        .and_then(|a| a.value.as_deref());
    let config = match &ctx.config {
        Some(c) => match c.read() { Ok(c) => c, Err(_) => return CommandResult::error("Config error\n".to_string()) },
        None => return CommandResult::ok("No config\n".to_string()),
    };

    // get_pool_resource (ua_select.cc:1008): exactly one pool, selected when
    // pool= is missing or unknown — never all pools at once (C-012).
    let mut shown = String::new();
    let mut errors = String::new();
    let allowed = |name: &str| crate::acl::access(ctx, crate::acl::Kind::Pool, name);
    let named = pool_name.filter(|name| allowed(name));
    let pool = match named.and_then(|name| config.pools.iter().find(|p| p.name == name)) {
        Some(pool) => pool,
        None => {
            if let Some(name) = named {
                errors.push_str(&format!("Error: Pool resource \"{name}\" does not exist.\n"));
            }
            let names = crate::director_config::sort_case_insensitive(config.pools.iter()
                .map(|p| p.name.as_str()).filter(|name| allowed(name)))
                .into_iter().map(str::to_string).collect();
            match super::prompt::select_resource(ctx, "The defined Pool resources are:\n", "Pool",
                "Select Pool resource", names) {
                Ok((name, text)) => {
                    shown = text;
                    config.pools.iter().find(|p| p.name == name).expect("selected from the configured pools")
                }
                Err(mut result) => {
                    result.text_output.insert_str(0, &errors);
                    return result;
                }
            }
        }
    };
    let Some(cat) = &ctx.catalog else {
        return CommandResult::ok(format!("{errors}{shown}No catalog configured\n"));
    };
    let sql = format!(
        "INSERT INTO pool (name, pooltype, maxvols, autoprune, volretention, fileretention, jobretention, numvols) \
         VALUES ('{}', '{}', {}, {}, {}, {}, {}, 0) ON CONFLICT (name) DO NOTHING",
        pool.name.replace('\'', "''"), pool.pool_type, pool.maximum_volumes,
        if pool.auto_prune { 1 } else { 0 },
        pool.volume_retention.as_secs(), 0, 0,
    );
    match cat.raw_query(&sql) {
        // ua_cmds.cc:845 (the "already exists" case is not distinguished yet).
        Ok(_) => CommandResult::ok(format!("{errors}{shown}Pool {} created.\n", pool.name)),
        Err(e) => {
            let mut result = CommandResult::error(format!("{e}\n"));
            result.text_output = format!("{errors}{shown}");
            result
        }
    }
}

pub fn cmd_add(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc add_cmd: OpenClientDb(ua) after the "label" hint.
    let hint = "You probably don't want to be using this command since it\n\
        creates database records without labeling the Volumes.\n\
        You probably want to use the \"label\" command.\n\n".to_string();
    crate::ua_db::with_prefix(hint, crate::ua_db::with_client_db(ast, ctx, false, || cmd_add_db(ast, ctx)))
}

/// add_cmd (ua_cmds.cc:611) always asks for the number of volumes and their
/// names (GetPint/GetCmd); command line arguments only preselect pool and
/// storage. Without the dialog nothing is created (C-012).
fn cmd_add_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use super::prompt::{select_pool_dbr, select_storage, selection_required};
    let pool = ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case("pool"))
        .and_then(|a| a.value.as_deref())
        .filter(|name| crate::acl::access(ctx, crate::acl::Kind::Pool, name));
    let (pool, mut shown) = match pool {
        Some(pool) => (pool.to_string(), String::new()),
        None => match select_pool_dbr(ctx) {
            Ok(selected) => selected,
            Err(result) => return result,
        },
    };
    // get_storage_resource(ua) for the media type; GetMediaType otherwise.
    match select_storage(ast, ctx, false) {
        Ok((_, text)) => shown.push_str(&text),
        Err(result) => return crate::ua_db::with_prefix(shown, result),
    }
    let max = ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows(&format!("SELECT MaxVols, NumVols FROM Pool WHERE Name='{}'",
            pool.replace('\'', "''"))).ok())
        .and_then(|t| t.rows.into_iter().next())
        .map(|row| row.into_iter().map(|v| v.and_then(|v| v.parse::<i64>().ok()).unwrap_or(0)).collect::<Vec<_>>())
        .map(|v| if v[0] == 0 { 1000 } else { v[0] - v[1] })
        .unwrap_or(1000);
    selection_required(format!("{shown}Enter number of Volumes to create. 0=>fixed name. Max={max}: "))
}

pub fn cmd_import(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    move_operation(ast, ctx, "import", |stor, command| sd_cmd_via_config(ctx, stor, command))
}

pub fn cmd_export(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    move_operation(ast, ctx, "export", |stor, command| sd_cmd_via_config(ctx, stor, command))
}

pub fn cmd_move(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    move_operation(ast, ctx, "move", |stor, command| sd_cmd_via_config(ctx, stor, command))
}

/// PerformMoveOperation (ua_impexp.cc:897): the storage comes from
/// get_storage_resource(ua, false, true) — never a default — and export/move
/// refuse to run without the slot selection they need (C-012).
fn move_operation(
    ast: &CommandAst,
    ctx: &ExecContext,
    operation: &str,
    send: impl FnOnce(&str, &str) -> Result<String, String>,
) -> CommandResult {
    let (store, mut text) = match super::prompt::select_storage(ast, ctx, true) {
        Ok(selected) => selected,
        Err(result) => return result,
    };
    let value = |keys: &[&str]| keys.iter().find_map(|key| ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref()));
    let src = value(&["srcslots", "srcslot"]);
    let dst = value(&["dstslots", "dstslot"]);
    let volume = value(&["volume"]);
    // ua_impexp.cc:1042-1058 operation specific checks.
    let refusal = match operation {
        "export" if src.is_none() && volume.is_none() =>
            Some("Cannot perform an export operation without source slot selection\n"),
        "move" if src.is_none() || dst.is_none() =>
            Some("Cannot perform a move operation without source and/or destination selection\n"),
        _ => None,
    };
    if let Some(refusal) = refusal {
        let mut result = CommandResult::error(refusal.to_string());
        result.text_output = text;
        return result;
    }
    let mut command = operation.to_string();
    for (key, value) in [("srcslots", src), ("dstslots", dst), ("volume", volume)] {
        if let Some(value) = value { command.push_str(&format!(" {key}={value}")); }
    }
    match send(&store.name, &command) {
        Ok(resp) => {
            text.push_str(&format!("{operation} on storage {}:\n{resp}\n", store.name));
            CommandResult::ok(text)
        }
        Err(e) => {
            let mut result = CommandResult::error(format!("{operation} {}: {e}\n", store.name));
            result.text_output = text;
            result
        }
    }
}

#[cfg(test)]
mod messages_tests {
    use super::*;
    use crate::message_bus::{BusMessage, MessageBus, MsgType};

    #[test]
    fn messages_preserve_bytes_and_are_consumed_once() {
        let bus = MessageBus::new(8);
        let rx = bus.subscribe();
        let ctx = ExecContext { session_id: 0, config: None, session_mgr: None,
            catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: Some(rx), job_manager: None };
        bus.post_text("first\n");
        bus.post(BusMessage::JobMessage { job_id: 42, job_name: "job".into(),
            msg_type: MsgType::Warning, text: "second\r\n".into(), timestamp: 0 });
        let dot = crate::session::parse_cli_command(".messages");
        assert_eq!(crate::commands::dot::cmd_dot_messages(&dot, &ctx).text_output, "first\nsecond\r\n");
        assert_eq!(crate::commands::dot::cmd_dot_messages(&dot, &ctx).text_output, "");
        assert_eq!(cmd_messages(&crate::session::parse_cli_command("messages"), &ctx).text_output, "You have no messages.\n");
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
        assert_eq!(denied.error_message, "Storage \"File\" not found.\n");
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
            "setdevice storage=File device=x autoselect=yes bogus=1", "setdevice storage=Unknown device=x autoselect=yes"] {
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

    fn run(line: &str, ctx: &ExecContext) -> CommandResult {
        let ast = crate::session::parse_cli_command(line);
        let operation = ast.command_name.clone();
        move_operation(&ast, ctx, &operation, |_, _| panic!("{line}: must not contact the SD"))
    }

    #[test]
    fn no_autochanger_means_empty_selection_and_no_sd_command() {
        let ctx = context("");
        for line in ["import", "export", "move", "import srcslots=1", "move srcslot=1 dstslot=2"] {
            let result = run(line, &ctx);
            assert!(!result.success, "{line}");
            assert_eq!(result.text_output, "The defined Autochanger Storage resources are:\n", "{line}");
            assert_eq!(result.error_message, "Selection list for \"Storage\" is empty!\n", "{line}");
        }
        let result = run("import storage=Nope", &ctx);
        assert_eq!(result.text_output, "Storage resource \"Nope\": not found\nThe defined Autochanger Storage resources are:\n");
        assert_eq!(run("export jobid=x", &ctx).error_message, "Expecting jobid=nn command, got: jobid\n");
        assert_eq!(run("move job=Backup", &ctx).error_message, "Job \"Backup\" is not running.\n");
    }

    #[test]
    fn relabel_never_defaults_storage_volume_or_pool() {
        let ctx = context("Pool { Name = Full; Pool Type = Backup }\nPool { Name = Inc; Pool Type = Backup }\n\
            Storage { Name = Other; Address = localhost }");
        let run = |line: &str| relabel_with(&crate::session::parse_cli_command(line), &ctx,
            |_, _| panic!("{line}: must not contact the SD"));
        assert_eq!(run("relabel").text_output,
            "The defined Storage resources are:\n1: File\n2: Other\nSelect Storage resource (1-2): ");
        assert_eq!(run("relabel storage=File").text_output,
            "Defined Pools:\n1: Full\n2: Inc\nSelect the Pool (1-2): ");
        assert_eq!(run("relabel storage=File oldvolume=a").text_output, "Enter new Volume name: ");
        assert_eq!(run("relabel storage=File oldvolume=a volume=b").text_output,
            "Defined Pools:\n1: Full\n2: Inc\nSelect the Pool (1-2): ");
        assert_eq!(run("relabel storage=File volume=b").text_output, "");
    }

    #[test]
    fn delete_acts_only_on_complete_requests() {
        let req = |line: &str| delete_request(&crate::session::parse_cli_command(line));
        for line in ["delete", "delete volume", "delete volume=v1", "delete jobid", "delete jobid=1-100"] {
            assert!(matches!(req(line), DeleteRequest::Dialog(_)), "{line}");
        }
        let DeleteRequest::Dialog(menu) = req("delete") else { unreachable!() };
        assert_eq!(menu.text_output, "In general it is not a good idea to delete either a\nPool or a Volume since they may contain data.\n\n\
            You have the following choices:\n1: volume\n2: pool\n3: jobid\n4: storage\nChoose catalog item to delete (1-4): ");
        assert!(matches!(req("delete volume=v1 yes"), DeleteRequest::Volume(v) if v == "v1"));
        let DeleteRequest::Jobs(ids, errors) = req("delete jobid=3,5-7,x") else { panic!() };
        assert_eq!((ids, errors.as_str()), (vec![3, 5, 6, 7], "Illegal JobId x ignored\n"));
        assert!(matches!(req("delete jobid=1;DROP"), DeleteRequest::Jobs(ids, _) if ids.is_empty()));

        let mut ctx = context("Pool { Name = Full; Pool Type = Backup }\nPool { Name = Inc; Pool Type = Backup }");
        let (catalog, log) = crate::commands::admin::side_effect_tests::recording_catalog("b1");
        ctx.catalog = Some(catalog);
        for line in ["delete", "delete volume", "delete volume=v1", "delete jobid", "delete jobid=1-100", "delete jobid=x"] {
            cmd_delete_db(&crate::session::parse_cli_command(line), &ctx);
        }
        for line in ["create", "create pool=Nope", "add", "add pool=Full volume=v1"] {
            let ast = crate::session::parse_cli_command(line);
            if line.starts_with("create") { cmd_create_db(&ast, &ctx); } else { cmd_add_db(&ast, &ctx); }
        }
        // add asks the pool's MaxVols/NumVols (read only) — nothing else may reach the catalog.
        assert!(log.lock().unwrap().iter().all(|sql| sql.starts_with("SELECT")), "{:?}", log.lock().unwrap());
        cmd_delete_db(&crate::session::parse_cli_command("delete volume=v1 yes"), &ctx);
        assert!(log.lock().unwrap().iter().any(|sql| sql.starts_with("DELETE")), "control");
    }

    #[test]
    fn create_and_add_never_default_the_pool() {
        let ctx = context("Pool { Name = Full; Pool Type = Backup }\nPool { Name = Inc; Pool Type = Backup }");
        let run = |f: fn(&CommandAst, &ExecContext) -> CommandResult, line: &str|
            f(&crate::session::parse_cli_command(line), &ctx);
        assert_eq!(run(cmd_create_db, "create").text_output,
            "The defined Pool resources are:\n1: Full\n2: Inc\nSelect Pool resource (1-2): ");
        assert_eq!(run(cmd_create_db, "create pool=Nope").text_output,
            "Error: Pool resource \"Nope\" does not exist.\nThe defined Pool resources are:\n1: Full\n2: Inc\nSelect Pool resource (1-2): ");
        assert_eq!(run(cmd_add_db, "add").text_output, "Defined Pools:\n1: Full\n2: Inc\nSelect the Pool (1-2): ");
        assert_eq!(run(cmd_add_db, "add pool=Full volume=v1 storage=File").text_output,
            "Enter number of Volumes to create. 0=>fixed name. Max=1000: ");
    }

    #[test]
    fn export_and_move_need_slot_selection() {
        let ctx = context("Storage { Name = Tape; Address = localhost; Auto Changer = yes }");
        let export = run("export", &ctx);
        assert_eq!(export.text_output, "Automatically selected Storage: Tape\n");
        assert_eq!(export.error_message, "Cannot perform an export operation without source slot selection\n");
        let moved = run("move srcslots=1", &ctx);
        assert_eq!(moved.error_message, "Cannot perform a move operation without source and/or destination selection\n");
        let ast = crate::session::parse_cli_command("import");
        let result = move_operation(&ast, &ctx, "import", |storage, command| {
            assert_eq!((storage, command), ("Tape", "import"));
            Ok("ok".into())
        });
        assert!(result.success);
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
    fn invalid_trace_input_does_not_change_trace_state() {
        let ctx = ExecContext { session_id: 0, config: None, session_mgr: None,
            catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let before = bareos_bnet::debug::trace_enabled();
        for line in ["trace", "trace maybe", "trace on off", "trace on=yes"] {
            assert!(!cmd_trace(&crate::session::parse_cli_command(line), &ctx).success);
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
