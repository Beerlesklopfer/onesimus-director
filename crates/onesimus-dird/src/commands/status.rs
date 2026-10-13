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
        "storage" => {
            let name = ast.arguments.iter()
                .find(|a| a.key == "storage")
                .and_then(|a| a.value.as_deref())
                .unwrap_or("*");
            status_storage(name, ctx)
        }
        "all" => {
            let mut out = status_director(ctx).text_output;
            out.push_str("\n");
            out.push_str(&status_client("*", ctx).text_output);
            out.push_str("\n");
            out.push_str(&status_storage("*", ctx).text_output);
            CommandResult::ok(out)
        }
        other => {
            // Try as client name
            status_client(other, ctx)
        }
    }
}

pub fn cmd_dot_status(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    cmd_status(ast, ctx)
}

fn status_director(ctx: &ExecContext) -> CommandResult {
    let version = env!("CARGO_PKG_VERSION");
    let started = crate::localtime::daemon_started();

    let mut output = format!(
        "bareos-dir Version: {version}\n\
         Daemon started {}\n\
         \n",
        crate::localtime::strftime("%d-%b-%Y %H:%M:%S", started)
    );

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
    let name = ctx.config.as_ref()
        .and_then(|c| c.read().ok().map(|c| c.director.name.clone()))
        .unwrap_or_default();
    let os = std::fs::read_to_string("/etc/os-release").ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string()))
        .unwrap_or_else(|| "Linux".to_string());

    let (running, jobs_run) = match &ctx.job_manager {
        Some(mgr) => {
            let jobs = mgr.lock().unwrap();
            let running: Vec<_> = jobs.running_jobs().into_iter()
                .map(|(id, name, phase)| json!({ "jobid": id, "name": name, "status": phase }))
                .collect();
            let finished = jobs.finished_count();
            (running, finished)
        }
        None => (Vec::new(), 0),
    };

    let clients: Vec<_> = ctx.session_mgr.as_ref()
        .map(|s| s.get_fd_connections().into_iter().map(|fd| json!({
            "connecttime": crate::localtime::strftime("%d-%b-%y %H:%M", fd.connect_time as libc::time_t),
            "protocol_version": fd.protocol_version,
            "authenticated": true,
            "name": fd.client_name,
        })).collect())
        .unwrap_or_default();

    json!({
        "header": {
            "director": name,
            "version": env!("CARGO_PKG_VERSION"),
            "release_date": env!("ONESIMUS_BUILD_DATE"),
            "os": os,
            "binary_info": "Onesimus",
            "daemon_started": crate::localtime::strftime("%d-%b-%y %H:%M", crate::localtime::daemon_started()),
            "jobs_run": jobs_run,
            "jobs_running": running.len(),
            "config_warnings": false,
        },
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
        output.push_str(&format!("Connecting to Client {} at {}:{}\n",
                                  client.name, client.address, client.port));

        let result = crate::fd_protocol::query_fd_status(
            &client.address,
            client.port,
            &client.password,
            &dir_name,
            &client.name,
        );

        if result.success {
            output.push_str(&format!("Connected to Client {}", client.name));
            if !result.version.is_empty() {
                output.push_str(&format!(" Version: {}", result.version));
            }
            output.push('\n');
            output.push_str(&result.status_text);
        } else {
            output.push_str(&format!("Failed: {}\n", result.status_text));
        }
        output.push('\n');
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
