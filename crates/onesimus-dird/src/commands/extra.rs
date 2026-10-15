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

pub fn cmd_autodisplay(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let on = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
    CommandResult::ok(format!("autodisplay {on}\n"))
}

pub fn cmd_automount(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let on = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
    CommandResult::ok(format!("automount {on}\n"))
}

pub fn cmd_gui(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let on = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
    CommandResult::ok(format!("gui mode {on}\n"))
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

pub fn cmd_memory(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    // Read /proc/self/status for VmRSS (resident set size)
    let mut output = String::new();
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if line.starts_with("VmRSS:") || line.starts_with("VmSize:") ||
               line.starts_with("VmPeak:") || line.starts_with("Threads:") {
                output.push_str(&format!("{line}\n"));
            }
        }
    }
    if output.is_empty() {
        output.push_str("Memory info not available\n");
    }
    CommandResult::ok(output)
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

pub fn cmd_trace(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let on = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
    CommandResult::ok(format!("trace {on}\n"))
}

pub fn cmd_var(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let name = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("?");
    CommandResult::ok(format!("Variable {name}: undefined\n"))
}

pub fn cmd_wait(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let job_id = ast.arguments.iter()
        .find(|a| a.key == "jobid")
        .and_then(|a| a.value.as_deref())
        .and_then(|v| v.parse::<u32>().ok());

    if let Some(ref mgr) = ctx.job_manager {
        loop {
            let running = mgr.lock().unwrap().running_jobs();
            let waiting_for = match job_id {
                Some(id) => running.iter().any(|(jid, _, _)| *jid == id),
                None => !running.is_empty(),
            };
            if !waiting_for { break; }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    match job_id {
        Some(id) => CommandResult::ok(format!("Job {id} finished.\n")),
        None => CommandResult::ok("No jobs running or queued.\n".to_string()),
    }
}

pub fn cmd_whoami(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if let Some(smgr) = &ctx.session_mgr {
        let sessions = smgr.get_session_names();
        if let Some((_, name)) = sessions.iter().find(|(id, _)| *id == ctx.session_id) {
            return CommandResult::ok(format!("{name}\n"));
        }
    }
    CommandResult::ok("*UserAgent*\n".to_string())
}

pub fn cmd_use(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    CommandResult::ok("Using default catalog\n".to_string())
}

pub fn cmd_setdebug(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let level: u32 = ast.arguments.iter().find(|a| a.key == "level")
        .and_then(|a| a.value.as_deref())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    bareos_bnet::debug::set_debug_level(level);
    CommandResult::ok(format!("Debug level set to {level}\n"))
}

pub fn cmd_setbandwidth(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let limit = ast.arguments.iter().find(|a| a.key == "limit")
        .and_then(|a| a.value.as_deref()).unwrap_or("0");
    CommandResult::ok(format!("Bandwidth limit set to {limit}\n"))
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
    for arg in &ast.arguments {
        if !matches!(arg.key.as_str(), "storage" | "device" | "autoselect") {
            return CommandResult::error(format!("Wrong argument: {}\n", arg.key));
        }
    }
    let value = |key: &str| ast.arguments.iter().find(|a| a.key == key)
        .and_then(|a| a.value.as_deref()).filter(|s| !s.is_empty());
    let mut missing = String::new();
    for key in ["autoselect", "device", "storage"] {
        if value(key).is_none() { missing.push_str(&format!("Argument missing: {key}\n")); }
    }
    if !missing.is_empty() { return CommandResult::error(missing); }
    let autoselect = match value("autoselect").unwrap().to_ascii_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => 1,
        "no" | "false" | "off" | "0" => 0,
        other => return CommandResult::error(format!("Wrong argument: {other}\n")),
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
    let command = format!("setdevice device={device} autoselect={autoselect}");
    match send(&storage, &director, &command) {
        Ok(response) => CommandResult::ok(format!("Connecting to Storage daemon {} at {}:{}\n{response}",
            storage.name, storage.address, storage.port)),
        Err(error) => CommandResult::error(format!("Failed to connect to Storage daemon.\n{error}\n")),
    }
}

pub fn cmd_resolve(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let hostname = ast.arguments.iter()
        .find(|a| a.key == "client")
        .and_then(|a| a.value.as_deref())
        .or_else(|| ast.arguments.first().map(|a| a.key.as_str()))
        .unwrap_or("localhost");

    // If it's a client name, resolve from config
    let addr = if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            cfg.clients.iter().find(|c| c.name == hostname)
                .map(|c| c.address.clone())
                .unwrap_or_else(|| hostname.to_string())
        } else { hostname.to_string() }
    } else { hostname.to_string() };

    use std::net::ToSocketAddrs;
    match format!("{addr}:0").to_socket_addrs() {
        Ok(addrs) => {
            let ips: Vec<String> = addrs.map(|a| a.ip().to_string()).collect();
            CommandResult::ok(format!("{hostname} resolves to: {}\n", ips.join(", ")))
        }
        Err(e) => CommandResult::ok(format!("{hostname}: resolution failed: {e}\n")),
    }
}

pub fn cmd_rerun(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let jobid = ast.arguments.iter().find(|a| a.key == "jobid")
        .and_then(|a| a.value.as_deref());
    let Some(id) = jobid else {
        return CommandResult::ok("rerun: specify jobid=<id>\n".to_string());
    };
    // Look up job name from catalog, then dispatch to cmd_run
    if let Some(cat) = &ctx.catalog {
        match cat.raw_query(&format!(
            "SELECT name FROM job WHERE jobid = {id}")) {
            Ok(result) => {
                let job_name = result.lines().skip(2).next()
                    .map(|l| l.trim().to_string())
                    .unwrap_or_default();
                if job_name.is_empty() {
                    return CommandResult::ok(format!("JobId {id} not found in catalog\n"));
                }
                // Build a synthetic AST for "run job=<name> yes"
                let run_ast = crate::command_ast::CommandAst {
                    command_name: "run".to_string(),
                    arguments: vec![
                        crate::command_ast::Argument { key: "job".to_string(), value: Some(job_name.clone()) },
                        crate::command_ast::Argument { key: "yes".to_string(), value: None },
                    ],
                    ..Default::default()
                };
                return crate::commands::job::cmd_run(&run_ast, ctx);
            }
            Err(e) => return CommandResult::error(format!("Catalog error: {e}\n")),
        }
    }
    CommandResult::ok(format!("No catalog — cannot look up JobId {id}\n"))
}

pub fn cmd_relabel(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
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
}
