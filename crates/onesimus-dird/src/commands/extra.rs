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
    // C++ ua_run.cc reRunCmd: OpenClientDb() before anything else. The
    // catalog lines go out before a run dialog (C-011), otherwise in front.
    match crate::ua_db::open_client_db(ast, ctx, false) {
        Ok(mut prefix) => {
            let result = cmd_rerun_db(ast, ctx, &mut prefix);
            crate::ua_db::with_prefix(prefix, result)
        }
        Err(result) => result,
    }
}

fn cmd_rerun_db(ast: &CommandAst, ctx: &ExecContext, prefix: &mut String) -> CommandResult {
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
        if ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes")) {
            arguments.push(crate::command_ast::Argument { key: "yes".to_string(), value: None });
        }
        let run_ast = crate::command_ast::CommandAst {
            command_name: "run".to_string(),
            arguments,
            ..Default::default()
        };
        // The run dialog follows the text so far (C-011).
        let mut ua = crate::ua_select::Ua::new(ctx);
        ua.send_msg(std::mem::take(prefix) + &std::mem::take(&mut text));
        let ok = crate::commands::job::run_with_ua(&mut ua, &run_ast, crate::commands::job::start_job);
        let run = ua.finish(ok);
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
            None => return crate::ua_db::with_prefix(shown, selection_required(String::new())),
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
    let mut ua = crate::ua_select::Ua::new(ctx);
    let ok = delete_dialog(&mut ua, ast, delete_execute);
    ua.finish(ok)
}

/// What a confirmed delete removes from the catalog.
#[derive(Debug, PartialEq, Eq)]
enum DeleteAction {
    Volume { name: String, purged: bool },
    Pool(String),
    Job(u64),
    Storage(String),
}

/// DeleteCmd (ua_cmds.cc:2220): OpenClientDb, keyword (or the warning and
/// "Choose catalog item to delete"), then DeleteVolume/DeletePool/DeleteJob/
/// DeleteStorage with their selections and confirmations. `act` runs only
/// for a selected and confirmed item.
fn delete_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    mut act: impl FnMut(&mut crate::ua_select::Ua, DeleteAction)) -> bool {
    use crate::ua_select::{self, find_arg, find_arg_with_value};
    const KEYWORDS: [&str; 4] = ["volume", "pool", "jobid", "storage"];
    let ctx = ua.ctx;
    if !ua.open_client_db(ast, true) {
        return true;
    }
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
            act(ua, DeleteAction::Volume { purged: media.vol_status == "Purged", name: media.volume_name });
        }
        Some(1) => {
            // DeletePool
            if ua.batch() {
                ua.error_msg("Deleting pools from the catalog is not supported in batch mode.\n");
                return false;
            }
            let Some(pool) = ua_select::select_pool_dbr(ua, ast, "pool") else { return true };
            let question = format!("Are you sure you want to delete Pool \"{pool}\"? (yes/no): ");
            if ua.get_yesno(&question) == Some(true) {
                act(ua, DeleteAction::Pool(pool));
            }
        }
        Some(2) => {
            // DeleteJob
            if find_arg_with_value(ast, "jobid").is_none() {
                if let Some(jobid) = ua.get_pint("Enter JobId to delete: ") {
                    act(ua, DeleteAction::Job(u64::from(jobid)));
                }
                return true;
            }
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
                            act(ua, DeleteAction::Job(jobid));
                        }
                    } else if number(token) {
                        act(ua, DeleteAction::Job(crate::commands::job::str_to_int64(token) as u64));
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
            act(ua, DeleteAction::Storage(given));
        }
        None => ua.warning_msg("Nothing done.\n"),
    }
    true
}

/// The catalog work of a confirmed delete.
fn delete_execute(ua: &mut crate::ua_select::Ua, action: DeleteAction) {
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    match action {
        DeleteAction::Volume { name, .. } => {
            let vol = name.replace('\'', "''");
            // Delete jobmedia first (FK), then media record
            let _ = cat.raw_query(&format!(
                "DELETE FROM jobmedia WHERE mediaid IN (SELECT mediaid FROM media WHERE volumename = '{vol}')"));
            match cat.raw_query(&format!("DELETE FROM media WHERE volumename = '{vol}'")) {
                Ok(_) => ua.send_msg(format!("Volume {name} deleted.\n")),
                Err(e) => ua.error_msg(format!("Delete error: {e}\n")),
            }
        }
        DeleteAction::Job(id) => {
            let _ = cat.raw_query(&format!("DELETE FROM jobmedia WHERE jobid = {id}"));
            let _ = cat.raw_query(&format!("DELETE FROM file WHERE jobid = {id}"));
            match cat.raw_query(&format!("DELETE FROM job WHERE jobid = {id}")) {
                Ok(_) => ua.send_msg(format!("Jobid {id} and associated records deleted from the catalog.\n")),
                Err(e) => ua.error_msg(format!("Delete error: {e}\n")),
            }
        }
        DeleteAction::Pool(_) => ua.send_msg("delete pool: not supported\n"),
        DeleteAction::Storage(_) => ua.send_msg("delete storage: not supported\n"),
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

/// CreatePool(POOL_OP_CREATE) and the "Pool %s created." message.
fn create_pool(ua: &mut crate::ua_select::Ua, pool: bareos_config::resources::PoolResource) {
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    let sql = format!(
        "INSERT INTO pool (name, pooltype, maxvols, autoprune, volretention, fileretention, jobretention, numvols) \
         VALUES ('{}', '{}', {}, {}, {}, {}, {}, 0) ON CONFLICT (name) DO NOTHING",
        pool.name.replace('\'', "''"), pool.pool_type, pool.maximum_volumes,
        if pool.auto_prune { 1 } else { 0 },
        pool.volume_retention.as_secs(), 0, 0,
    );
    // ua_cmds.cc:845 (the "already exists" case is not distinguished yet).
    if let Err(e) = cat.raw_query(&sql) {
        ua.error_msg(format!("{e}\n"));
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
    let storage = ua_select::get_storage_resource(ua, ast, false, false);
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
        use crate::ua_select::tests::scripted;
        let ctx = context("Pool { Name = Full; Pool Type = Backup }\nPool { Name = Inc; Pool Type = Backup }");
        let delete = |line: &str, answers: &[&str]| {
            let mut acted = Vec::new();
            let (_, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                delete_dialog(&mut ua, &crate::session::parse_cli_command(line), |_, action| acted.push(action));
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
        delete_dialog(&mut ua, &crate::session::parse_cli_command("delete"), |_, _| panic!("batch"));
        let result = ua.finish(true);
        assert!(result.text_output.contains("Your request has multiple choices for \"\""), "{}", result.text_output);
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
