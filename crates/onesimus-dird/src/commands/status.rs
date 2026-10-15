//! Status commands: status, .status

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;

pub fn cmd_status(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // "status dir" → bare arg "dir"
    // "status client=bareos-fd" → key="client" value="bareos-fd"
    let target = ast.arguments.first()
        .map(|a| a.key.as_str())
        .unwrap_or("dir");

    match target.to_lowercase().as_str() {
        "dir" | "director" => status_director(ctx),
        "client" => {
            let name = ast.arguments.iter()
                .find(|a| a.key == "client")
                .and_then(|a| a.value.as_deref())
                .unwrap_or("*");
            status_client(name, ctx)
        }
        "all" => {
            let mut out = status_director(ctx).text_output;
            out.push_str("\n");
            out.push_str(&status_client("*", ctx).text_output);
            out.push_str("\n");
            out.push_str(&status_storage("*", ctx).text_output);
            CommandResult::ok(out)
        }
        _ => status_storage_selected(ast, ctx),
    }
}

pub fn cmd_dot_status(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let arg = |i: usize| ast.arguments.get(i).map(|a| a.key.to_lowercase());
    if arg(0).as_deref() == Some("dir") && arg(1).as_deref() == Some("header") {
        return dir_status_header(ctx);
    }
    cmd_status(ast, ctx)
}

/// C++ ua_status.cc ListDirStatusHeader (also `memory`). Plugin list,
/// secure erase command and configuration warnings are not supported by the
/// Rust Director and therefore never printed.
pub fn dir_status_header(ctx: &ExecContext) -> CommandResult {
    let header = dir_status_header_json(ctx);
    let text = format!(
        "{} Version: {} ({}) {}\n\
         Daemon started {}. Jobs: run={}, running={} db:postgresql, {} binary\n",
        header["director"].as_str().unwrap_or_default(),
        crate::version_strings::FULL,
        crate::version_strings::DATE,
        crate::version_strings::os_info(),
        header["daemon_started"].as_str().unwrap_or_default(),
        header["jobs_run"], header["jobs_running"],
        crate::version_strings::BINARY_INFO,
    );
    CommandResult::ok(text).with_json(serde_json::json!({ "header": header }))
}

/// JSON object of ListDirStatusHeader.
fn dir_status_header_json(ctx: &ExecContext) -> serde_json::Value {
    let name = ctx.config.as_ref()
        .and_then(|c| c.read().ok().map(|c| c.director.name.clone()))
        .unwrap_or_default();
    let (jobs_run, jobs_running) = ctx.job_manager.as_ref()
        .map(|mgr| {
            let jobs = mgr.lock().unwrap();
            (jobs.finished_count(), jobs.running_jobs().len())
        })
        .unwrap_or((0, 0));
    serde_json::json!({
        "director": name,
        "version": crate::version_strings::FULL,
        "release_date": crate::version_strings::DATE,
        "os": crate::version_strings::os_info(),
        "binary_info": crate::version_strings::BINARY_INFO,
        "daemon_started": crate::localtime::strftime("%d-%b-%y %H:%M", crate::localtime::daemon_started()),
        "jobs_run": jobs_run,
        "jobs_running": jobs_running,
        "config_warnings": false,
    })
}

fn status_director(ctx: &ExecContext) -> CommandResult {
    let mut output = dir_status_header(ctx).text_output;
    output.push('\n');

    // Config info
    if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            output.push_str(&format!(
                "Director: {}\n\
                 Clients: {}\n\
                 Jobs: {}\n\
                 Storages: {}\n\
                 Pools: {}\n\
                 Schedules: {}\n\
                 Catalogs: {}\n\
                 \n",
                cfg.director.name,
                cfg.clients.len(),
                cfg.jobs.len(),
                cfg.storages.len(),
                cfg.pools.len(),
                cfg.schedules.len(),
                cfg.catalogs.len(),
            ));
        }
    }

    // Session info
    if let Some(smgr) = &ctx.session_mgr {
        let sessions = smgr.active_session_count();
        let fds = smgr.fd_connection_count();
        let sds = smgr.sd_connection_count();

        output.push_str(&format!(
            "Console Connections: {sessions}\n\
             FD Connections: {fds}\n\
             SD Connections: {sds}\n\
             \n"
        ));

        // List connected clients
        let fd_list = smgr.get_fd_connections();
        if !fd_list.is_empty() {
            output.push_str("Connected File Daemons:\n");
            for fd in &fd_list {
                output.push_str(&format!(
                    "  {} (proto={}, alive={})\n",
                    fd.client_name, fd.protocol_version, fd.alive
                ));
            }
            output.push('\n');
        }

        // List sessions
        let session_list = smgr.get_session_names();
        if !session_list.is_empty() {
            output.push_str("Console Sessions:\n");
            for (id, name) in &session_list {
                output.push_str(&format!("  #{id}: {name}\n"));
            }
            output.push('\n');
        }
    } else {
        output.push_str(
            "Console Connections: 0\n\
             FD Connections: 0\n\
             SD Connections: 0\n\n"
        );
    }

    // Running jobs from JobManager
    output.push_str("Running Jobs:\n");
    if let Some(ref mgr) = ctx.job_manager {
        let jobs = mgr.lock().unwrap();
        let running = jobs.running_jobs();
        if running.is_empty() {
            output.push_str("  No running jobs.\n");
        } else {
            output.push_str("  JobId    Name                           Phase\n");
            output.push_str("  ============================================================\n");
            for (id, name, phase) in &running {
                output.push_str(&format!("  {id:<8} {name:<30} {phase}\n"));
            }
        }
    } else {
        output.push_str("  No running jobs.\n");
    }

    // Terminated jobs from catalog
    output.push_str("\nTerminated Jobs:\n");
    if let Some(cat) = &ctx.catalog {
        match cat.raw_query(
            "SELECT jobid, jobname, type, level, jobstatus, clientname, \
             starttime, endtime, jobfiles, jobbytes \
             FROM vw_job_summary WHERE jobstatus IN ('T','E','f','A') \
             ORDER BY jobid DESC LIMIT 10"
        ) {
            Ok(result) if !result.contains("No results") => {
                output.push_str(&result);
            }
            _ => output.push_str("  No terminated jobs.\n"),
        }
    } else {
        output.push_str("  No terminated jobs.\n");
    }
    output.push_str("====\n");

    CommandResult::ok(output).with_json(status_director_json(ctx))
}

/// JSON shape of C++ DoDirectorStatus (ua_status.cc). Only facts the Rust
/// Director actually has: the scheduler does not dispatch jobs yet, so
/// "scheduled" stays empty; "terminated" (C++: jobs finished since start)
/// is not tracked yet either.
fn status_director_json(ctx: &ExecContext) -> serde_json::Value {
    use serde_json::json;
    let running: Vec<_> = ctx.job_manager.as_ref()
        .map(|mgr| mgr.lock().unwrap().running_jobs().into_iter()
            .map(|(id, name, phase)| json!({ "jobid": id, "name": name, "status": phase }))
            .collect())
        .unwrap_or_default();

    let clients: Vec<_> = ctx.session_mgr.as_ref()
        .map(|s| s.get_fd_connections().into_iter().map(|fd| json!({
            "connecttime": crate::localtime::strftime("%d-%b-%y %H:%M", fd.connect_time as libc::time_t),
            "protocol_version": fd.protocol_version,
            "authenticated": true,
            "name": fd.client_name,
        })).collect())
        .unwrap_or_default();

    json!({
        "header": dir_status_header_json(ctx),
        "scheduled": [],
        "running": running,
        "terminated": [],
        "client-connection": clients,
    })
}

fn status_client(name: &str, ctx: &ExecContext) -> CommandResult {
    let config = match &ctx.config {
        Some(c) => c.read().ok(),
        None => return CommandResult::ok(format!("Client {name}: no config loaded\n")),
    };
    let config = match config {
        Some(c) => c,
        None => return CommandResult::ok(format!("Client {name}: config lock error\n")),
    };

    let dir_name = config.director.name.clone();

    // Find all matching clients
    let clients: Vec<_> = if name == "*" {
        config.clients.iter().collect()
    } else {
        config.clients.iter().filter(|c| c.name == name).collect()
    };

    if clients.is_empty() {
        return CommandResult::ok(format!("Client '{name}' not found in configuration\n"));
    }

    let mut output = String::new();
    for client in &clients {
        // DoNativeClientStatus (fd_cmds.cc:999)
        if ctx.api_mode == crate::command_executor::ApiMode::Human {
            output.push_str(&format!("Connecting to Client {} at {}:{}\n",
                                      client.name, client.address, client.port));
        }
        let (info, fd) = crate::fd_protocol::connect_fd_console(
            &client.address, client.port, &client.password, &dir_name, &client.name);
        output.push_str(&info);
        match fd.and_then(|fd| crate::fd_protocol::fd_query_commands(fd, &["status".to_string()])) {
            Ok(status) => output.push_str(&status),
            Err(_) => output.push_str(&format!("\nFailed to connect to Client {}.\n====\n", client.name)),
        }
    }

    CommandResult::ok(output)
}

fn status_storage(name: &str, ctx: &ExecContext) -> CommandResult {
    let config = match &ctx.config {
        Some(c) => match c.read() {
            Ok(c) => c,
            Err(_) => return CommandResult::ok(format!("Storage {name}: config error\n")),
        },
        None => return CommandResult::ok(format!("Storage {name}: no config\n")),
    };

    let dir_name = config.director.name.clone();
    let storages: Vec<_> = if name == "*" {
        config.storages.iter().collect()
    } else {
        config.storages.iter().filter(|s| s.name == name).collect()
    };

    if storages.is_empty() {
        return CommandResult::ok(format!("Storage '{name}' not found\n"));
    }

    let mut output = String::new();
    for stor in &storages {
        output.push_str(&format!("Connecting to Storage {} at {}:{}\n",
                                  stor.name, stor.address, stor.port));

        let result = crate::sd_protocol::query_sd_status(
            &stor.address, stor.port, &stor.password, &dir_name, &stor.name,
        );

        if result.success {
            output.push_str(&format!("Connected to Storage {}\n", stor.name));
            output.push_str(&result.status_text);
        } else {
            output.push_str(&format!("Failed: {}\n", result.status_text));
        }
        output.push('\n');
    }

    CommandResult::ok(output)
}

/// Storage branch of C++ StatusCmd (ua_status.cc:200-221): any other first
/// keyword selects a storage with get_storage_resource(ua, false,
/// autochangers_only = `slots` given) and reports its status.
fn status_storage_selected(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let slots = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("slots"));
    match crate::ua_storage_select::get_storage_resource(ast, ctx, false, slots) {
        Ok((store, text)) => crate::ua_db::with_prefix(text, status_storage(&store.name, ctx)),
        // StatusCmd returns true whatever the selection printed.
        Err(result) => CommandResult { success: true, ..result },
    }
}
