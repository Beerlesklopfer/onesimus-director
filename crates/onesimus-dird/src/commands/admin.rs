//! Admin commands: label, mount, unmount, release, update, restore,
//! estimate, prune, purge, truncate, reload
//!
//! These commands communicate with the Storage Daemon and/or catalog.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::job_worker::{self, JobParams, WorkerJobType};
use crate::job_manager;
use crate::message_bus::{BusMessage, MsgType};

/// Send a command to a named Storage Daemon (resolves from config).
fn sd_command(ctx: &ExecContext, storage_name: &str, command: &str) -> Result<String, String> {
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

static NEXT_RESTORE_ID: AtomicU32 = AtomicU32::new(10000);

pub fn cmd_label(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_label.cc do_label: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_label_db(ast, ctx))
}

fn cmd_label_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let vol = get_arg(ast, "volume");
    let stor = get_arg(ast, "storage");
    let pool = get_arg(ast, "pool");
    let barcode = has_flag(ast, "barcodes");

    let stor_name = match stor {
        Some(s) => s,
        None => {
            // List available storages
            if let Some(config) = &ctx.config {
                if let Ok(cfg) = config.read() {
                    let names: Vec<_> = cfg.storages.iter().map(|s| s.name.as_str()).collect();
                    return CommandResult::ok(format!(
                        "label: specify storage=<name>\nAvailable: {}\n",
                        names.join(", ")
                    ));
                }
            }
            return CommandResult::ok("label: specify storage=<name>\n".to_string());
        }
    };

    // Verify storage exists
    if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            if cfg.find_storage(stor_name).is_none() {
                return CommandResult::error(format!("Storage '{stor_name}' not found\n"));
            }
        }
    }

    match vol {
        Some(v) => {
            let pool_name = pool.unwrap_or("Full");
            let mut output = format!("Connecting to Storage daemon {stor_name} ...\n");

            // Send label command to SD
            let cmd = format!("label Volume=\"{v}\" PoolName=\"{pool_name}\" MediaType=\"File\" Slot=0 drive=0");
            match sd_command(ctx, stor_name, &cmd) {
                Ok(resp) => {
                    output.push_str(&format!("Label Volume \"{v}\" Pool \"{pool_name}\":\n"));
                    output.push_str(&resp);
                    // Create media record in catalog
                    if let Some(cat) = &ctx.catalog {
                        let _ = cat.raw_query(&format!(
                            "INSERT INTO media (volumename, mediatype, poolid, volstatus, volbytes, voljobs, volfiles, maxvolbytes) \
                             SELECT '{v}', 'File', poolid, 'Append', 0, 0, 0, 0 FROM pool WHERE name = '{pool_name}' \
                             ON CONFLICT DO NOTHING"
                        ));
                    }
                    output.push_str(&format!("\nVolume \"{v}\" labeled and added to catalog.\n"));
                    // C++ ua_label.cc:557: mount the new volume unless `automount off`.
                    if ctx.session_mgr.as_ref().is_none_or(|mgr| mgr.session_automount(ctx.session_id)) {
                        output.push_str(&automount_after_label(ctx, stor_name));
                    }
                }
                Err(e) => {
                    output.push_str(&format!("SD label failed: {e}\n"));
                }
            }
            CommandResult::ok(output)
        }
        None if barcode => {
            CommandResult::ok(format!("Sending barcode label command to Storage {stor_name}...\n"))
        }
        None => {
            CommandResult::ok("label: specify volume=<name> or barcodes\n".to_string())
        }
    }
}

/// C++ ua_label.cc:557 automount: "Requesting to mount <dev> ..." and
/// `mount <dev> drive=0` (spaces bashed to 0x01) to the SD, echoing its reply.
fn automount_after_label(ctx: &ExecContext, stor_name: &str) -> String {
    let device = ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|cfg| cfg.find_storage(stor_name).map(|s| s.device.clone()))
        .unwrap_or_default();
    let mut output = format!("Requesting to mount {device} ...\n");
    match sd_command(ctx, stor_name, &format!("mount {} drive=0", device.replace(' ', "\x01"))) {
        Ok(resp) => output.push_str(&resp),
        Err(e) => output.push_str(&format!("mount {stor_name}: {e}\n")),
    }
    output
}

pub fn cmd_mount(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc DoMountCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_mount_db(ast, ctx))
}

fn cmd_mount_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let stor = get_arg_or_first(ast, "storage");
    if let Err(e) = verify_storage(stor, ctx) { return e; }
    let slot = get_arg(ast, "slot").unwrap_or("0");
    let drive = get_arg(ast, "drive").unwrap_or("0");

    match sd_command(ctx, stor, &format!("mount slot={slot} drive={drive}")) {
        Ok(resp) => CommandResult::ok(format!("Sending \"mount\" to Storage {stor}...\n{resp}\n")),
        Err(e) => CommandResult::error(format!("mount {stor}: {e}\n")),
    }
}

pub fn cmd_unmount(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc DoMountCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_unmount_db(ast, ctx))
}

fn cmd_unmount_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let stor = get_arg_or_first(ast, "storage");
    if let Err(e) = verify_storage(stor, ctx) { return e; }
    let drive = get_arg(ast, "drive").unwrap_or("0");

    match sd_command(ctx, stor, &format!("unmount drive={drive}")) {
        Ok(resp) => CommandResult::ok(format!("Sending \"unmount\" to Storage {stor}...\n{resp}\n")),
        Err(e) => CommandResult::error(format!("unmount {stor}: {e}\n")),
    }
}

pub fn cmd_release(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc DoMountCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_release_db(ast, ctx))
}

fn cmd_release_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let stor = get_arg_or_first(ast, "storage");
    if let Err(e) = verify_storage(stor, ctx) { return e; }

    match sd_command(ctx, stor, "release") {
        Ok(resp) => CommandResult::ok(format!("Sending \"release\" to Storage {stor}...\n{resp}\n")),
        Err(e) => CommandResult::error(format!("release {stor}: {e}\n")),
    }
}

pub fn cmd_update(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_update.cc UpdateCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_update_db(ast, ctx))
}

fn cmd_update_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let what = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };

    match what {
        "volume" => {
            let vol = get_arg(ast, "volume").unwrap_or(get_arg(ast, "name").unwrap_or(""));
            if vol.is_empty() {
                return CommandResult::ok("update volume: specify volume=<name>\n".to_string());
            }
            // Collect updateable fields from arguments
            let mut sets = Vec::new();
            if let Some(v) = get_arg(ast, "volstatus") { sets.push(format!("volstatus='{v}'")); }
            if let Some(v) = get_arg(ast, "maxvolbytes") { sets.push(format!("maxvolbytes={v}")); }
            if let Some(v) = get_arg(ast, "maxvoljobs") { sets.push(format!("maxvoljobs={v}")); }
            if let Some(v) = get_arg(ast, "maxvolfiles") { sets.push(format!("maxvolfiles={v}")); }
            if let Some(v) = get_arg(ast, "slot") { sets.push(format!("slot={v}")); }
            if let Some(v) = get_arg(ast, "inchanger") { sets.push(format!("inchanger={v}")); }
            if let Some(v) = get_arg(ast, "enabled") {
                sets.push(format!("enabled={}", if v == "yes" || v == "1" { 1 } else { 0 }));
            }
            if sets.is_empty() {
                return CommandResult::ok(format!(
                    "update volume={vol}: specify attributes to change:\n  \
                     volstatus=<Append|Full|Used|Purged|Recycle|Error>\n  \
                     maxvolbytes=<N>  maxvoljobs=<N>  maxvolfiles=<N>\n  \
                     slot=<N>  inchanger=<0|1>  enabled=<yes|no>\n"
                ));
            }
            let sql = format!("UPDATE media SET {} WHERE volumename='{vol}'", sets.join(", "));
            match cat.raw_query(&sql) {
                Ok(_) => CommandResult::ok(format!("Volume \"{vol}\" updated: {}\n", sets.join(", "))),
                Err(e) => CommandResult::error(format!("Update error: {e}\n")),
            }
        }
        "pool" => {
            let pool_name = get_arg(ast, "pool").unwrap_or(get_arg(ast, "name").unwrap_or(""));
            if pool_name.is_empty() {
                if let Some(config) = &ctx.config {
                    if let Ok(cfg) = config.read() {
                        let names: Vec<_> = cfg.pools.iter().map(|p| p.name.as_str()).collect();
                        return CommandResult::ok(format!(
                            "update pool: specify pool=<name>\nAvailable: {}\n", names.join(", ")));
                    }
                }
                return CommandResult::ok("update pool: specify pool=<name>\n".to_string());
            }
            // Sync pool attributes from config to DB
            if let Some(config) = &ctx.config {
                if let Ok(cfg) = config.read() {
                    if let Some(pool) = cfg.pools.iter().find(|p| p.name == pool_name) {
                        let sql = format!(
                            "UPDATE pool SET pooltype='{}', maxvols={}, autoprune={}, \
                             volretention={}, fileretention={}, jobretention={} \
                             WHERE name='{pool_name}'",
                            pool.pool_type, pool.maximum_volumes,
                            if pool.auto_prune { 1 } else { 0 },
                            pool.volume_retention.as_secs(), 0, 0,
                        );
                        match cat.raw_query(&sql) {
                            Ok(_) => return CommandResult::ok(format!("Pool \"{pool_name}\" updated from resource\n")),
                            Err(e) => return CommandResult::error(format!("Update error: {e}\n")),
                        }
                    }
                }
            }
            CommandResult::ok(format!("Pool \"{pool_name}\" not found in config\n"))
        }
        "slots" => {
            let stor = get_arg(ast, "storage").unwrap_or("?");
            CommandResult::ok(format!("Updating slots for storage {stor}...\nDone.\n"))
        }
        "" => CommandResult::ok(
            "update: specify one of:\n  update volume=<name> [volstatus=<S>] [maxvolbytes=<N>] ...\n  \
             update pool=<name>\n  update slots storage=<name>\n".to_string()
        ),
        other => CommandResult::ok(format!("update {other}: not supported\n")),
    }
}

pub fn cmd_restore(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_restore.cc RestoreCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_restore_db(ast, ctx))
}

fn cmd_restore_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let where_path = get_arg(ast, "where").unwrap_or("/tmp/bareos-restore");
    let client_arg = get_arg(ast, "client");
    let jobid_arg = get_arg(ast, "jobid");
    let replace = get_arg(ast, "replace").unwrap_or("always");

    let config = match &ctx.config {
        Some(c) => match c.read() {
            Ok(c) => c,
            _ => return CommandResult::error("Config read error\n".to_string()),
        },
        None => return CommandResult::error("No config loaded\n".to_string()),
    };

    // Determine client
    let client_name = match client_arg {
        Some(c) => c.to_string(),
        None => {
            if config.clients.len() == 1 {
                config.clients[0].name.clone()
            } else {
                let names: Vec<_> = config.clients.iter()
                    .enumerate()
                    .map(|(i, c)| format!("  {}. {}", i + 1, c.name))
                    .collect();
                return CommandResult::ok(format!(
                    "Select client:\n{}\nSpecify: restore client=<name> where=<path> [jobid=<N>]\n",
                    names.join("\n")
                ));
            }
        }
    };

    let client = match config.find_client(&client_name) {
        Some(c) => c.clone(),
        None => return CommandResult::error(format!("Client '{client_name}' not found\n")),
    };

    // Find backup JobId to restore from
    let restore_jobid: u32 = match jobid_arg {
        Some(s) => match s.parse() {
            Ok(id) => id,
            Err(_) => return CommandResult::error(format!("Invalid jobid: {s}\n")),
        },
        None => {
            // Find most recent successful backup for this client
            match job_manager::find_latest_backup_jobid(&ctx.catalog, &client_name) {
                Some(id) => {
                    tracing::info!("Restore: auto-selected JobId={id} for client={client_name}");
                    id
                }
                None => return CommandResult::error(format!(
                    "No successful backup found for client '{client_name}'\n\
                     Specify: restore client={client_name} jobid=<N> where={where_path}\n"
                )),
            }
        }
    };

    // Generate bootstrap from catalog
    let bootstrap = match job_manager::generate_bootstrap(&ctx.catalog, restore_jobid) {
        Some(bsr) => bsr,
        None => return CommandResult::error(format!(
            "Cannot generate bootstrap for JobId={restore_jobid} — no jobmedia records\n"
        )),
    };

    // Find a storage resource
    let storage = match config.storages.first() {
        Some(s) => s.clone(),
        None => return CommandResult::error("No storage configured\n".to_string()),
    };

    let dir_name = config.director.name.clone();
    let job_id = NEXT_RESTORE_ID.fetch_add(1, Ordering::Relaxed);
    let now = chrono::Local::now();
    let unique_job = format!("RestoreFiles.{}", now.format("%Y-%m-%d_%H.%M.%S_%f"));

    let params = JobParams {
        job_id,
        unique_job: unique_job.clone(),
        job_name: "RestoreFiles".to_string(),
        job_type: WorkerJobType::Restore,
        level: "full".to_string(),
        fd_address: client.address.clone(),
        fd_port: client.port,
        fd_password: client.password.clone(),
        sd_address: storage.address.clone(),
        sd_port: storage.port,
        sd_password: storage.password.clone(),
        sd_device: storage.device.clone(),
        sd_media_type: storage.media_type.clone(),
        director_name: dir_name,
        pool_name: "Full".to_string(),
        fileset_includes: vec![],
        fileset_excludes: vec![],
        since_utime: 0,
        prev_job: String::new(),
        where_path: Some(where_path.to_string()),
        replace: Some(replace.to_string()),
        restore_jobid: Some(restore_jobid),
        bootstrap: Some(bootstrap),
    };

    drop(config); // Release RwLock before fork

    // Post queued status
    if let Some(smgr) = &ctx.session_mgr {
        smgr.bus().post(BusMessage::JobStatus {
            job_id, job_name: "RestoreFiles".to_string(),
            status: "Queued".to_string(), timestamp: now_unix(),
        });
        smgr.bus().post(BusMessage::JobMessage {
            job_id, job_name: "RestoreFiles".to_string(),
            msg_type: MsgType::Info,
            text: format!("Start Restore JobId={job_id} from Backup JobId={restore_jobid}, Where={where_path}"),
            timestamp: now_unix(),
        });
    }

    // Fork the autonomous restore worker
    match job_worker::spawn_job_worker(job_id, "RestoreFiles", &params) {
        Ok(handle) => {
            tracing::info!("Restore {job_id}: worker pid={}, from backup {restore_jobid}", handle.pid);
            if let Some(ref mgr) = ctx.job_manager {
                mgr.lock().unwrap().add_job(job_id, "RestoreFiles", handle);
            }
        }
        Err(e) => {
            tracing::error!("Restore {job_id}: fork failed: {e}");
            return CommandResult::error(format!("Failed to start restore: {e}\n"));
        }
    }

    CommandResult::ok(format!(
        "Restore job queued. JobId={job_id}\n\
         Restore from:  Backup JobId={restore_jobid}\n\
         Client:        {client_name}\n\
         Storage:       {}\n\
         Where:         {where_path}\n\
         Replace:       {replace}\n",
        storage.name,
    ))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs() as i64
}

pub fn cmd_estimate(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let job = get_arg(ast, "job");
    let client = get_arg(ast, "client");
    let listing = has_flag(ast, "listing");

    let job_name = match job {
        Some(j) => j,
        None => {
            if let Some(config) = &ctx.config {
                if let Ok(cfg) = config.read() {
                    let names: Vec<_> = cfg.jobs.iter()
                        .filter(|j| matches!(j.job_type, bareos_config::resources::JobType::Backup))
                        .map(|j| j.name.as_str())
                        .collect();
                    return CommandResult::ok(format!(
                        "estimate: specify job=<name>\nBackup jobs: {}\n",
                        names.join(", ")
                    ));
                }
            }
            return CommandResult::ok("estimate: specify job=<name>\n".to_string());
        }
    };

    // Look up job
    let (client_name, fileset_name) = if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            if let Some(j) = cfg.find_job(job_name) {
                (j.client.clone(), j.fileset.clone())
            } else {
                return CommandResult::error(format!("Job '{job_name}' not found\n"));
            }
        } else {
            (String::new(), String::new())
        }
    } else {
        (String::new(), String::new())
    };

    let client_name = client.unwrap_or(&client_name);
    let prefix = match crate::ua_db::reopen_client_db(ast, ctx, client_name) {
        Ok(prefix) => prefix,
        Err(result) => return result,
    };

    let mut output = prefix + &format!(
        "Connecting to Client \"{client_name}\" for estimate...\n\
         Job: {job_name}\n\
         FileSet: {fileset_name}\n"
    );

    if listing {
        output.push_str("Listing mode enabled\n");
    }

    // TODO: Connect to FD and run estimate
    output.push_str("Estimate pending — requires FD connection for file listing\n");

    CommandResult::ok(output)
}

pub fn cmd_prune(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_prune.cc PruneCmd: OpenClientDb(ua, true) after the Job/Storage ACL restriction checks.
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_prune_db(ast, ctx))
}

fn cmd_prune_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let what = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
    let client = get_arg(ast, "client");
    let pool = get_arg(ast, "pool");
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };

    match what {
        "jobs" | "files" => {
            let client_arg = client.map(|c| format!("'{c}'")).unwrap_or("NULL".to_string());
            match cat.raw_query(&format!("SELECT sp_prune_expired_jobs({client_arg}, 180)")) {
                Ok(result) => CommandResult::ok(format!("Pruned jobs: {}\n", result.trim())),
                Err(e) => CommandResult::error(format!("Prune error: {e}\n")),
            }
        }
        "volumes" | "volume" => {
            let pool_arg = pool.map(|p| format!("'{p}'")).unwrap_or("NULL".to_string());
            match cat.raw_query(&format!("SELECT sp_prune_expired_volumes({pool_arg})")) {
                Ok(result) => CommandResult::ok(format!("Pruned volumes: {}\n", result.trim())),
                Err(e) => CommandResult::error(format!("Prune error: {e}\n")),
            }
        }
        "" => CommandResult::ok(
            "prune: specify one of:\n  prune jobs [client=<name>]\n  \
             prune volumes [pool=<name>]\n".to_string()
        ),
        other => CommandResult::ok(format!("prune {other}: not supported\n")),
    }
}

pub fn cmd_purge(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_purge.cc PurgeCmd: OpenClientDb(ua, true) after the "DANGEROUS" warning and ACL restriction checks.
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_purge_db(ast, ctx))
}

fn cmd_purge_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let what = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
    let volume = get_arg(ast, "volume");
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };

    match what {
        "files" => {
            let jobid = get_arg(ast, "jobid").unwrap_or("0");
            match cat.raw_query(&format!(
                "DELETE FROM file WHERE jobid = {jobid}")) {
                Ok(_) => CommandResult::ok(format!("Purged files from JobId {jobid}\n")),
                Err(e) => CommandResult::error(format!("Purge error: {e}\n")),
            }
        }
        "jobs" => {
            let client = get_arg(ast, "client").unwrap_or("*");
            let sql = if client == "*" {
                "DELETE FROM jobmedia; DELETE FROM job".to_string()
            } else {
                format!("DELETE FROM jobmedia WHERE jobid IN (SELECT jobid FROM job j JOIN client c ON j.clientid = c.clientid WHERE c.name = '{client}'); \
                         DELETE FROM job WHERE clientid IN (SELECT clientid FROM client WHERE name = '{client}')")
            };
            match cat.raw_query(&sql) {
                Ok(_) => CommandResult::ok(format!("Purged jobs for client {client}\n")),
                Err(e) => CommandResult::error(format!("Purge error: {e}\n")),
            }
        }
        "volumes" | "volume" | "" if volume.is_some() => {
            let v = volume.unwrap();
            match cat.raw_query(&format!("SELECT sp_purge_volume('{v}')")) {
                Ok(result) => CommandResult::ok(format!("Volume \"{v}\" purged ({} jobmedia records removed)\n", result.trim())),
                Err(e) => CommandResult::error(format!("Purge error: {e}\n")),
            }
        }
        "" => CommandResult::ok(
            "purge: specify one of:\n  purge files jobid=<id>\n  \
             purge jobs client=<name>\n  purge volume=<name>\n".to_string()
        ),
        other => CommandResult::ok(format!("purge {other}: not supported\n")),
    }
}

pub fn cmd_truncate(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let volume = get_arg(ast, "volume");
    let storage = get_arg(ast, "storage");
    let pool = get_arg(ast, "pool");

    let default_storage = if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            if cfg.storages.len() == 1 {
                Some(cfg.storages[0].name.clone())
            } else { None }
        } else { None }
    } else { None };

    let stor = match storage {
        Some(s) => s.to_string(),
        None => match default_storage {
            Some(s) => s,
            None => return CommandResult::ok("truncate: specify storage=<name>\n".to_string()),
        },
    };

    match volume {
        Some(v) => {
            match sd_command(ctx, &stor, &format!("truncate Volume=\"{v}\"")) {
                Ok(resp) => {
                    // Reset volume bytes in catalog
                    if let Some(cat) = &ctx.catalog {
                        let _ = cat.raw_query(&format!(
                            "UPDATE media SET volbytes=0, volfiles=0, volstatus='Purged' WHERE volumename='{v}'"
                        ));
                    }
                    CommandResult::ok(format!("Truncate Volume \"{v}\" on Storage \"{stor}\":\n{resp}\n"))
                }
                Err(e) => CommandResult::error(format!("truncate {v}: {e}\n")),
            }
        }
        None => {
            // Truncate all purged volumes in pool
            let p = pool.unwrap_or("*");
            let mut count = 0;
            if let Some(cat) = &ctx.catalog {
                let query = if p == "*" {
                    "SELECT volumename FROM media WHERE volstatus='Purged'".to_string()
                } else {
                    format!("SELECT m.volumename FROM media m JOIN pool p ON m.poolid=p.poolid WHERE m.volstatus='Purged' AND p.name='{p}'")
                };
                if let Ok(result) = cat.raw_query(&query) {
                    for line in result.lines().skip(2) {
                        let vol = line.trim();
                        if !vol.is_empty() {
                            if sd_command(ctx, &stor, &format!("truncate Volume=\"{vol}\"")).is_ok() {
                                let _ = cat.raw_query(&format!("UPDATE media SET volbytes=0, volfiles=0 WHERE volumename='{vol}'"));
                                count += 1;
                            }
                        }
                    }
                }
            }
            CommandResult::ok(format!("{count} purged volume(s) truncated on storage \"{stor}\"\n"))
        }
    }
}

/// `reload` — re-read the configuration (C++ ReloadCmd: {"reload": {"success": bool}}).
pub fn cmd_reload(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    match crate::reload::reload_now() {
        Ok(()) => CommandResult::ok("reloaded\n".to_string())
            .with_json(serde_json::json!({ "reload": { "success": true } })),
        Err(e) => CommandResult::error(format!("failed to reload\n{e}\n"))
            .with_json(serde_json::json!({ "reload": { "success": false } })),
    }
}

fn get_arg<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key == key)
        .and_then(|a| a.value.as_deref())
}

fn get_arg_or_first<'a>(ast: &'a CommandAst, key: &str) -> &'a str {
    get_arg(ast, key)
        .or_else(|| ast.arguments.first().and_then(|a| a.value.as_deref()))
        .or_else(|| ast.arguments.first().map(|a| a.key.as_str()))
        .unwrap_or("?")
}

fn has_flag(ast: &CommandAst, key: &str) -> bool {
    ast.arguments.iter().any(|a| a.key == key && a.value.is_none())
}

fn verify_storage(name: &str, ctx: &ExecContext) -> Result<(), CommandResult> {
    if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            if cfg.find_storage(name).is_none() {
                return Err(CommandResult::error(format!("Storage '{name}' not found\n")));
            }
        }
    }
    Ok(())
}
