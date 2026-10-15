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

fn cmd_var_db(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let name = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("?");
    CommandResult::ok(format!("Variable {name}: undefined\n"))
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

pub fn cmd_setip(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    CommandResult::ok("Client address updated\n".to_string())
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
        // Build a synthetic AST for "run job=<name> yes"
        let run_ast = crate::command_ast::CommandAst {
            command_name: "run".to_string(),
            arguments: vec![
                crate::command_ast::Argument { key: "job".to_string(), value: Some(job_name) },
                crate::command_ast::Argument { key: "yes".to_string(), value: None },
            ],
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
    let oldvol = ast.arguments.iter().find(|a| a.key == "oldvolume")
        .and_then(|a| a.value.as_deref()).unwrap_or("?");
    let newvol = ast.arguments.iter().find(|a| a.key == "volume")
        .and_then(|a| a.value.as_deref());
    let stor = ast.arguments.iter().find(|a| a.key == "storage")
        .and_then(|a| a.value.as_deref()).unwrap_or("File");

    let Some(newname) = newvol else {
        return CommandResult::ok("relabel: specify oldvolume=<old> volume=<new> storage=<name>\n".to_string());
    };

    match sd_cmd_via_config(ctx, stor, &format!("relabel OldVolume=\"{oldvol}\" Volume=\"{newname}\"")) {
        Ok(resp) => {
            if let Some(cat) = &ctx.catalog {
                let _ = cat.raw_query(&format!("UPDATE media SET volumename='{newname}' WHERE volumename='{oldvol}'"));
            }
            CommandResult::ok(format!("Relabel {oldvol} → {newname}:\n{resp}\n"))
        }
        Err(e) => CommandResult::error(format!("relabel: {e}\n")),
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
    let what = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
    match what {
        "volume" => {
            let vol = ast.arguments.iter().find(|a| a.key == "volume")
                .and_then(|a| a.value.as_deref()).unwrap_or("?");
            // Delete jobmedia first (FK), then media record
            let _ = cat.raw_query(&format!(
                "DELETE FROM jobmedia WHERE mediaid IN (SELECT mediaid FROM media WHERE volumename = '{vol}')"));
            match cat.raw_query(&format!("DELETE FROM media WHERE volumename = '{vol}'")) {
                Ok(_) => CommandResult::ok(format!("Volume {vol} deleted from catalog\n")),
                Err(e) => CommandResult::error(format!("Delete error: {e}\n")),
            }
        }
        "jobid" => {
            let id = ast.arguments.iter().find(|a| a.key == "jobid")
                .and_then(|a| a.value.as_deref()).unwrap_or("0");
            let _ = cat.raw_query(&format!("DELETE FROM jobmedia WHERE jobid = {id}"));
            let _ = cat.raw_query(&format!("DELETE FROM file WHERE jobid = {id}"));
            match cat.raw_query(&format!("DELETE FROM job WHERE jobid = {id}")) {
                Ok(_) => CommandResult::ok(format!("JobId {id} deleted from catalog\n")),
                Err(e) => CommandResult::error(format!("Delete error: {e}\n")),
            }
        }
        _ => CommandResult::ok("delete: specify volume=<name> or jobid=<id>\n".to_string()),
    }
}

pub fn cmd_create(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc CreateCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_create_db(ast, ctx))
}

fn cmd_create_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let pool_name = ast.arguments.iter().find(|a| a.key == "pool")
        .and_then(|a| a.value.as_deref());
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };
    let config = match &ctx.config {
        Some(c) => match c.read() { Ok(c) => c, Err(_) => return CommandResult::error("Config error\n".to_string()) },
        None => return CommandResult::ok("No config\n".to_string()),
    };

    let pools: Vec<_> = match pool_name {
        Some(name) => config.pools.iter().filter(|p| p.name == name).collect(),
        None => config.pools.iter().collect(),
    };

    let mut created = 0;
    for pool in &pools {
        let sql = format!(
            "INSERT INTO pool (name, pooltype, maxvols, autoprune, volretention, fileretention, jobretention, numvols) \
             VALUES ('{}', '{}', {}, {}, {}, {}, {}, 0) ON CONFLICT (name) DO NOTHING",
            pool.name, pool.pool_type, pool.maximum_volumes,
            if pool.auto_prune { 1 } else { 0 },
            pool.volume_retention.as_secs(), 0, 0,
        );
        if cat.raw_query(&sql).is_ok() { created += 1; }
    }
    CommandResult::ok(format!("{created} pool(s) created from resource\n"))
}

pub fn cmd_add(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc add_cmd: OpenClientDb(ua) after the "label" hint.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_add_db(ast, ctx))
}

fn cmd_add_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let pool = ast.arguments.iter().find(|a| a.key == "pool")
        .and_then(|a| a.value.as_deref()).unwrap_or("Full");
    let volume = ast.arguments.iter().find(|a| a.key == "volume")
        .and_then(|a| a.value.as_deref());
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };
    match volume {
        Some(vol) => {
            let sql = format!(
                "INSERT INTO media (volumename, mediatype, poolid, volstatus, volbytes, voljobs, volfiles, maxvolbytes) \
                 SELECT '{vol}', 'File', poolid, 'Append', 0, 0, 0, 0 FROM pool WHERE name = '{pool}' \
                 ON CONFLICT DO NOTHING"
            );
            match cat.raw_query(&sql) {
                Ok(_) => CommandResult::ok(format!("Volume \"{vol}\" added to pool \"{pool}\"\n")),
                Err(e) => CommandResult::error(format!("Add error: {e}\n")),
            }
        }
        None => CommandResult::ok("add: specify volume=<name> pool=<name>\n".to_string()),
    }
}

pub fn cmd_import(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let stor = ast.arguments.iter().find(|a| a.key == "storage")
        .and_then(|a| a.value.as_deref()).unwrap_or("File");
    let srcslots = ast.arguments.iter().find(|a| a.key == "srcslots")
        .and_then(|a| a.value.as_deref()).unwrap_or("0");

    match sd_cmd_via_config(ctx, stor, &format!("import srcslots={srcslots}")) {
        Ok(resp) => CommandResult::ok(format!("Import from storage {stor}:\n{resp}\n")),
        Err(e) => CommandResult::error(format!("import {stor}: {e}\n")),
    }
}

pub fn cmd_export(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let stor = ast.arguments.iter().find(|a| a.key == "storage")
        .and_then(|a| a.value.as_deref()).unwrap_or("File");
    let srcslots = ast.arguments.iter().find(|a| a.key == "srcslots")
        .and_then(|a| a.value.as_deref()).unwrap_or("0");

    match sd_cmd_via_config(ctx, stor, &format!("export srcslots={srcslots}")) {
        Ok(resp) => CommandResult::ok(format!("Export from storage {stor}:\n{resp}\n")),
        Err(e) => CommandResult::error(format!("export {stor}: {e}\n")),
    }
}

pub fn cmd_move(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let stor = ast.arguments.iter().find(|a| a.key == "storage")
        .and_then(|a| a.value.as_deref()).unwrap_or("File");
    let from = ast.arguments.iter().find(|a| a.key == "srcslot")
        .and_then(|a| a.value.as_deref()).unwrap_or("0");
    let to = ast.arguments.iter().find(|a| a.key == "dstslot")
        .and_then(|a| a.value.as_deref()).unwrap_or("0");

    match sd_cmd_via_config(ctx, stor, &format!("move srcslot={from} dstslot={to}")) {
        Ok(resp) => CommandResult::ok(format!("Move slot {from} → {to} on {stor}:\n{resp}\n")),
        Err(e) => CommandResult::error(format!("move: {e}\n")),
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
