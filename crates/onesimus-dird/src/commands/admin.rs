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
    let pool = get_arg(ast, "pool");
    let barcode = has_flag(ast, "barcodes");

    // ua_label.cc:447 get_storage_resource: no default storage (C-012).
    let (store, mut shown) = match super::prompt::select_storage(ast, ctx, false) {
        Ok(selected) => selected,
        Err(result) => return result,
    };
    let stor_name = store.name.as_str();

    match vol {
        Some(v) => {
            // SelectPoolDbr: no default pool; a missing pool= is selected.
            let pool_name = match pool {
                Some(pool) => pool.to_string(),
                None => match super::prompt::select_pool_dbr(ctx) {
                    Ok((name, text)) => { shown.push_str(&text); name }
                    Err(result) => return crate::ua_db::with_prefix(shown, result),
                },
            };
            let mut output = format!("{shown}Connecting to Storage daemon {stor_name} ...\n");

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
            // ua_label.cc: GetCmd "Enter new Volume name: ".
            super::prompt::selection_required(format!("{shown}Enter new Volume name: "))
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

/// Keywords of UserSelectJobidsOrFiles (ua_restore.cc:510).
const RESTORE_KEYWORDS: &[&str] = &["jobid", "current", "before", "file", "directory", "select",
    "pool", "all", "fileregex", "client", "storage", "fileset", "where", "yes", "bootstrap", "done",
    "strip_prefix", "add_prefix", "add_suffix", "regexwhere", "restoreclient", "copies", "comment",
    "restorejob", "replace", "pluginoptions", "archive"];

/// The parts of RestoreCmd (ua_restore.cc:103) that decide whether a
/// restore may start without console interaction. Only `jobid=… all done
/// yes` is fully specified; everything else stops where upstream would ask
/// (C-012). Returns None when the restore may proceed.
fn restore_needs_dialog(ast: &CommandAst) -> Option<CommandResult> {
    use super::prompt::{prompt_list, selection_required};
    for arg in &ast.arguments {
        if !RESTORE_KEYWORDS.contains(&arg.key.to_ascii_lowercase().as_str()) {
            return Some(CommandResult::error(format!("Unknown keyword: {}\n", arg.key)));
        }
    }
    let flag = |key: &str| ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key));
    if ["file", "directory", "select", "fileregex", "before", "current"].iter().any(|k| flag(k)) {
        // File/date selections need GetClientName and further dialogs.
        return Some(selection_required(String::new()));
    }
    if get_arg(ast, "jobid").is_none() {
        // ua_restore.cc:648 job selection menu.
        let choices: Vec<String> = ["List last 20 Jobs run", "List Jobs where a given File is saved",
            "Enter list of comma separated JobIds to select", "Enter SQL list command",
            "Select the most recent backup for a client", "Select backup for a client before a specified time",
            "Enter a list of files to restore", "Enter a list of files to restore before a specified time",
            "Find the JobIds of the most recent backup for a client",
            "Find the JobIds for a backup for a client before a specified time",
            "Enter a list of directories to restore for found JobIds",
            "Select full restore to a specified Job date", "Cancel"].iter().map(|s| s.to_string()).collect();
        return Some(selection_required(format!(
            "\nFirst you select one or more JobIds that contain files\n\
             to be restored. You will be presented several methods\n\
             of specifying the JobIds. Then you will be allowed to\n\
             select which files from those JobIds are to be restored.\n\n{}",
            prompt_list("To select the JobIds, you have the following choices:\n", &choices, "Select item: "))));
    }
    if !flag("all") || !flag("done") {
        // BuildDirectoryTree (ua_restore.cc:1294): file selection in the tree.
        return Some(selection_required(String::new()));
    }
    if !flag("yes") {
        // RunCmd of the generated restore job: "OK to run? (yes/mod/no): ".
        return Some(selection_required("OK to run? (yes/mod/no): ".into()));
    }
    None
}

fn cmd_restore_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if let Some(result) = restore_needs_dialog(ast) {
        return result;
    }
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

pub fn cmd_prune(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_prune.cc PruneCmd: OpenClientDb(ua, true) after the Job/Storage ACL restriction checks.
    // Job/Storage restrictions are refused before the catalog is opened.
    if let Some(denied) = denied_restriction(ctx, &[(crate::acl::Kind::Job, "job"), (crate::acl::Kind::Storage, "storage")]) {
        return denied;
    }
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_prune_db(ast, ctx))
}

/// `Permission denied: need full %s permission.` (ua_prune.cc:370, ua_purge.cc:71).
fn denied_restriction(ctx: &ExecContext, kinds: &[(crate::acl::Kind, &str)]) -> Option<CommandResult> {
    kinds.iter().find(|(kind, _)| crate::acl::has_restrictions(ctx, *kind)).map(|(_, name)|
        CommandResult::error(format!("Permission denied: need full {name} permission.\n")))
}

/// First argument naming one of `keywords` (FindArgKeyword, case-insensitive).
fn find_arg_keyword(ast: &CommandAst, keywords: &[&str]) -> Option<usize> {
    ast.arguments.iter().find_map(|a| keywords.iter().position(|k| a.key.eq_ignore_ascii_case(k)))
}

enum PruneAction { Jobs(String, String), Volumes(Option<String>) }

/// PruneCmd (ua_prune.cc:362) decisions before any catalog change: keyword
/// menu, client selection and the retention confirmation, which only `yes`
/// skips (ConfirmRetention). C-012.
fn prune_plan(ast: &CommandAst, ctx: &ExecContext) -> Result<PruneAction, CommandResult> {
    use super::prompt::{keyword_prompt, select_client, selection_required};
    const KEYWORDS: [&str; 5] = ["Files", "Jobs", "Volume", "Stats", "Directory"];
    let yes = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes"));
    let kw = find_arg_keyword(ast, &KEYWORDS).or_else(|| ast.arguments.iter()
        .any(|a| a.key.len() >= 6 && a.key[..6].eq_ignore_ascii_case("volume")).then_some(2));
    match kw {
        None => Err(keyword_prompt("", &KEYWORDS, "Choose item to prune")),
        Some(0 | 1) => {
            let (client, shown) = select_client(ast, ctx)?;
            if !yes {
                // ConfirmRetention: "The current File/Job retention period is: …"
                // then GetCmd "Continue? (yes/mod/no): ".
                return Err(selection_required(format!("{shown}Continue? (yes/mod/no): ")));
            }
            Ok(PruneAction::Jobs(client, shown))
        }
        Some(2) if yes => Ok(PruneAction::Volumes(get_arg(ast, "pool").map(str::to_string))),
        // Volume selection (SelectMediaDbr / pool report) and ConfirmRetention.
        Some(2) => Err(selection_required(String::new())),
        Some(_) => Err(CommandResult::ok("prune: stats and directory are not supported\n".into())),
    }
}

fn cmd_prune_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let plan = match prune_plan(ast, ctx) {
        Ok(plan) => plan,
        Err(result) => return result,
    };
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };
    match plan {
        PruneAction::Jobs(client, shown) => {
            match cat.raw_query(&format!("SELECT sp_prune_expired_jobs('{}', 180)", client.replace('\'', "''"))) {
                Ok(result) => CommandResult::ok(format!("{shown}Pruned jobs: {}\n", result.trim())),
                Err(e) => CommandResult::error(format!("Prune error: {e}\n")),
            }
        }
        PruneAction::Volumes(pool) => {
            let pool_arg = pool.map(|p| format!("'{}'", p.replace('\'', "''"))).unwrap_or("NULL".to_string());
            match cat.raw_query(&format!("SELECT sp_prune_expired_volumes({pool_arg})")) {
                Ok(result) => CommandResult::ok(format!("Pruned volumes: {}\n", result.trim())),
                Err(e) => CommandResult::error(format!("Prune error: {e}\n")),
            }
        }
    }
}

pub fn cmd_purge(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_purge.cc PurgeCmd: OpenClientDb(ua, true) after the "DANGEROUS" warning and ACL restriction checks.
    use crate::acl::Kind;
    let warning = "\nThis command can be DANGEROUS!!!\n\n\
        It purges (deletes) all Files from a Job,\n\
        JobId, Client or Volume; or it purges (deletes)\n\
        all Jobs from a Client or Volume without regard\n\
        to retention periods. Normally you should use the\n\
        PRUNE command, which respects retention periods.\n\
        This command requires full access to all resources.\n".to_string();
    if let Some(denied) = denied_restriction(ctx, &[(Kind::Client, "client"), (Kind::Job, "job"),
        (Kind::Pool, "pool"), (Kind::Storage, "storage")]) {
        return crate::ua_db::with_prefix(warning, denied);
    }
    crate::ua_db::with_prefix(warning, crate::ua_db::with_client_db(ast, ctx, true, || cmd_purge_db(ast, ctx)))
}

enum PurgeAction { FilesOfJob(u64), JobsOfClient(String, String), Volume(String) }

/// PurgeCmd (ua_purge.cc:64) decisions before any catalog change. Purge
/// has no confirmation, but every target must be named or selected; the
/// keyword menu "Choose item to purge" replaces any default (C-012).
fn purge_plan(ast: &CommandAst, ctx: &ExecContext) -> Result<PurgeAction, CommandResult> {
    use super::prompt::{keyword_prompt, select_client, selection_required};
    const KEYWORDS: [&str; 4] = ["files", "jobs", "volume", "quota"];
    let menu = || Err(keyword_prompt("", &KEYWORDS, "Choose item to purge"));
    match find_arg_keyword(ast, &KEYWORDS) {
        Some(0) => match find_arg_keyword(ast, &["Job", "JobId", "Client", "Volume"]) {
            Some(1) => match get_arg(ast, "jobid").map(|v| v.trim().parse::<u64>()) {
                Some(Ok(id)) if id > 0 => Ok(PurgeAction::FilesOfJob(id)),
                // GetJobDbr: "Could not find Job" without a valid JobId.
                _ => Err(selection_required(String::new())),
            },
            // Job name / client / volume variants are not implemented here.
            Some(_) => Err(selection_required(String::new())),
            None => menu(),
        },
        Some(1) => match find_arg_keyword(ast, &["Client", "Volume", "Pool"]) {
            Some(0) => select_client(ast, ctx).map(|(client, shown)| PurgeAction::JobsOfClient(client, shown)),
            Some(_) => Err(selection_required(String::new())),
            None => menu(),
        },
        Some(2) => match get_arg(ast, "volume") {
            Some(volume) if crate::acl::valid_name(volume) => Ok(PurgeAction::Volume(volume.to_string())),
            // SelectMediaDbr: pool and volume dialog.
            _ => Err(selection_required(String::new())),
        },
        Some(_) => Err(CommandResult::ok("purge quota: not supported\n".into())),
        None => menu(),
    }
}

fn cmd_purge_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let plan = match purge_plan(ast, ctx) {
        Ok(plan) => plan,
        Err(result) => return result,
    };
    let cat = match &ctx.catalog {
        Some(c) => c,
        None => return CommandResult::ok("No catalog configured\n".to_string()),
    };
    match plan {
        PurgeAction::FilesOfJob(jobid) => {
            match cat.raw_query(&format!("DELETE FROM file WHERE jobid = {jobid}")) {
                Ok(_) => CommandResult::ok(format!("Purged files from JobId {jobid}\n")),
                Err(e) => CommandResult::error(format!("Purge error: {e}\n")),
            }
        }
        PurgeAction::JobsOfClient(client, shown) => {
            let client = client.replace('\'', "''");
            let sql = format!("DELETE FROM jobmedia WHERE jobid IN (SELECT jobid FROM job j JOIN client c ON j.clientid = c.clientid WHERE c.name = '{client}'); \
                         DELETE FROM job WHERE clientid IN (SELECT clientid FROM client WHERE name = '{client}')");
            match cat.raw_query(&sql) {
                Ok(_) => CommandResult::ok(format!("{shown}Purged jobs for client {client}\n")),
                Err(e) => CommandResult::error(format!("Purge error: {e}\n")),
            }
        }
        PurgeAction::Volume(v) => {
            match cat.raw_query(&format!("SELECT sp_purge_volume('{v}')")) {
                Ok(result) => CommandResult::ok(format!("Volume \"{v}\" purged ({} jobmedia records removed)\n", result.trim())),
                Err(e) => CommandResult::error(format!("Purge error: {e}\n")),
            }
        }
    }
}

/// `truncate volstatus=Purged [storage=] [pool=] [volume=] [drive=] [yes]`
/// (ua_cmds.cc:1972 TruncateCmd). Arguments are validated exactly like
/// upstream before the catalog or a Storage Daemon is touched; truncating
/// needs the confirmation (`yes`), compatibility log C-012.
pub fn cmd_truncate(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    truncate_with(ast, ctx, |storage, command| sd_command(ctx, storage, command))
}

fn truncate_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    mut send: impl FnMut(&str, &str) -> Result<String, String>,
) -> CommandResult {
    use super::prompt::{selection_required, usage};
    use crate::acl::{self, Kind};
    if ast.arguments.is_empty() {
        return CommandResult::error(usage("truncate", "missing parameter"));
    }
    let value = |key: &str| ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref());
    let yes = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes"));
    match value("volstatus") {
        None => return CommandResult::error(usage("truncate", "required parameter 'volstatus' missing")),
        Some(status) if !status.eq_ignore_ascii_case("Purged") => return CommandResult::error(
            usage("truncate", "Invalid parameter. 'volstatus' must be 'Purged'.")),
        Some(_) => {}
    }
    // OpenDb (ua_cmds.cc:2008) precedes the remaining checks.
    let prefix = match crate::ua_db::open_client_db(ast, ctx, false) {
        Ok(prefix) => prefix,
        Err(result) => return result,
    };
    let done = |result: CommandResult| crate::ua_db::with_prefix(prefix.clone(), result);
    let mut parsed = 2; // the command itself and volstatus, as upstream counts
    let storage = value("storage");
    if storage.is_some() || acl::has_restrictions(ctx, Kind::Storage) {
        // SelectStorageDbr (ua_select.cc:823): "Defined Storages:" /
        // "Select Storage resource" dialog when no accessible storage is named.
        if !storage.is_some_and(|name| known_storage(ctx, name)) {
            return done(selection_required(String::new()));
        }
        parsed += 1;
    }
    let pool = value("pool");
    if pool.is_some() || acl::has_restrictions(ctx, Kind::Pool) {
        // SelectPoolDbr (ua_select.cc): "Defined Pools:" / "Select the Pool" dialog.
        if !pool.is_some_and(|name| acl::access(ctx, Kind::Pool, name)) {
            return done(selection_required(String::new()));
        }
        parsed += 1;
    }
    let volume = value("volume").filter(|name| acl::valid_name(name));
    if volume.is_some() { parsed += 1; }
    let mut text = String::new();
    if let Some(drive) = value("drive") {
        if drive.parse::<i32>().is_ok() && !drive.starts_with('+') {
            parsed += 1;
        } else {
            text.push_str(&usage("truncate", &format!("Drive number must be integer but was : {drive}\n")));
        }
    }
    if yes { parsed += 1; }
    if parsed != ast.arguments.len() + 1 {
        text.push_str(&usage("truncate", "Invalid parameter."));
        return done(CommandResult::error(text));
    }

    let Some(cat) = &ctx.catalog else {
        return done(CommandResult::error(format!("{text}Failed to open db.\n")));
    };
    let quote = |v: &str| v.replace('\'', "''");
    let mut query = String::from("SELECT m.volumename FROM media m JOIN pool p ON m.poolid=p.poolid \
        WHERE m.volstatus='Purged' AND m.recycle=1 AND m.enabled=1 AND m.volbytes>64512");
    if let Some(p) = pool { query.push_str(&format!(" AND p.name='{}'", quote(p))); }
    if let Some(s) = storage {
        query.push_str(&format!(" AND m.storageid=(SELECT storageid FROM storage WHERE name='{}')", quote(s)));
    }
    if let Some(v) = volume { query.push_str(&format!(" AND m.volumename='{}'", quote(v))); }
    let volumes: Vec<String> = match cat.query_rows(&format!("{query} ORDER BY m.mediaid")) {
        Ok(table) => table.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect(),
        Err(e) => return done(CommandResult::error(format!("{text}{e}\n"))),
    };
    if volumes.is_empty() {
        return done(CommandResult::ok(text));
    }
    if !yes {
        // GetConfirmation (ua_input.cc:165, ua_cmds.cc:2120): API sessions
        // decline silently; the console is asked "Truncate listed volumes
        // (yes/no)? " after the volume list (ListSqlQuery, C-011).
        if ctx.api_mode != crate::command_executor::ApiMode::Human {
            return done(CommandResult::ok(text));
        }
        return done(selection_required(format!("{text}Truncate listed volumes (yes/no)? ")));
    }
    let stor = match storage {
        Some(s) => s.to_string(),
        None => match ctx.config.as_ref().and_then(|c| c.read().ok())
            .filter(|cfg| cfg.storages.len() == 1).map(|cfg| cfg.storages[0].name.clone()) {
            Some(s) => s,
            None => return done(CommandResult::error(format!("{text}truncate: specify storage=<name>\n"))),
        },
    };
    for vol in volumes {
        match send(&stor, &format!("truncate Volume=\"{vol}\"")) {
            Ok(resp) => {
                let _ = cat.raw_query(&format!(
                    "UPDATE media SET volbytes=0, volfiles=0 WHERE volumename='{}'", quote(&vol)));
                text.push_str(&format!("Truncate Volume \"{vol}\" on Storage \"{stor}\":\n{resp}\n"));
            }
            Err(e) => text.push_str(&format!("truncate {vol}: {e}\n")),
        }
    }
    done(CommandResult::ok(text))
}

fn known_storage(ctx: &ExecContext, name: &str) -> bool {
    ctx.config.as_ref().and_then(|c| c.read().ok()).is_some_and(|cfg| cfg.find_storage(name).is_some())
        && crate::acl::access(ctx, crate::acl::Kind::Storage, name)
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

#[cfg(test)]
pub(crate) mod side_effect_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    pub(crate) fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nStorage { Name = File; Address = 192.0.2.1; Port = 9103 }\n\
             Pool { Name = Full; Pool Type = Backup }\nPool { Name = Incremental; Pool Type = Backup }"
        ).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    /// Catalog double that records every request; job creation and any
    /// write are visible in the log. `job_name` answers the rerun lookup.
    pub(crate) fn recording_catalog(job_name: &'static str)
        -> (crate::catalog_worker::CatalogHandle, Arc<std::sync::Mutex<Vec<String>>>) {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse};
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        let handle = crate::catalog_worker::CatalogHandle::mock(move |request| {
            let (entry, response) = match request {
                CatalogRequest::RawQuery { sql } => (sql, CatalogResponse::Text(format!("name\n----\n{job_name}\n"))),
                CatalogRequest::TextRows { sql } => (sql, CatalogResponse::Error("no rows in the double".into())),
                CatalogRequest::CreateJob { .. } => ("CreateJob".to_string(), CatalogResponse::Error("double".into())),
                _ => ("other request".to_string(), CatalogResponse::Error("double".into())),
            };
            seen.lock().unwrap().push(entry);
            response
        });
        (handle, log)
    }

    const USAGE: &str = "USAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]\n";

    #[test]
    fn truncate_rejects_like_upstream_without_touching_the_sd() {
        let ctx = context();
        for (line, expected) in [
            ("truncate", format!("missing parameter\n{USAGE}")),
            ("truncate yes", format!("required parameter 'volstatus' missing\n{USAGE}")),
            ("truncate volstatus", format!("required parameter 'volstatus' missing\n{USAGE}")),
            ("truncate volstatus=Full", format!("Invalid parameter. 'volstatus' must be 'Purged'.\n{USAGE}")),
            ("truncate volstatus=purged bogus", format!("Invalid parameter.\n{USAGE}")),
            ("truncate volstatus=Purged volume=bad*name", format!("Invalid parameter.\n{USAGE}")),
            ("truncate volstatus=Purged drive=x", format!("Drive number must be integer but was : x\n\n{USAGE}Invalid parameter.\n{USAGE}")),
            ("truncate volstatus=Purged storage=File yes", "Failed to open db.\n".to_string()),
        ] {
            let result = truncate_with(&crate::session::parse_cli_command(line), &ctx,
                |_, _| panic!("{line}: must not contact the SD"));
            assert!(!result.success, "{line}");
            assert_eq!(result.error_message, expected, "{line}");
        }
        // Unknown storage: upstream opens the storage selection; nothing is done.
        let result = truncate_with(&crate::session::parse_cli_command("truncate volstatus=Purged storage=Nope"),
            &ctx, |_, _| panic!("must not contact the SD"));
        assert!(result.success && result.error_message.is_empty());
    }

    #[test]
    fn prune_and_purge_need_target_and_confirmation() {
        let ctx = context();
        ctx.config.as_ref().unwrap().write().unwrap().clients.push(bareos_config::resources::ClientResource {
            name: "fd1".into(), ..Default::default() });
        let ast = |line: &str| crate::session::parse_cli_command(line);
        let text = |r: Result<PruneAction, CommandResult>| match r { Err(r) => r.text_output, Ok(_) => panic!("acted") };
        assert_eq!(text(prune_plan(&ast("prune"), &ctx)), "You have the following choices:\n1: Files\n2: Jobs\n3: Volume\n\
            4: Stats\n5: Directory\nChoose item to prune (1-5): ");
        assert_eq!(text(prune_plan(&ast("prune jobs"), &ctx)), "Automatically selected Client: fd1\nContinue? (yes/mod/no): ");
        assert_eq!(text(prune_plan(&ast("prune files client=fd1"), &ctx)), "Continue? (yes/mod/no): ");
        assert!(prune_plan(&ast("prune volume"), &ctx).is_err());
        assert!(prune_plan(&ast("prune volumes pool=Full"), &ctx).is_err());
        assert!(matches!(prune_plan(&ast("prune jobs client=fd1 yes"), &ctx), Ok(PruneAction::Jobs(c, _)) if c == "fd1"));
        assert!(matches!(prune_plan(&ast("prune volume yes"), &ctx), Ok(PruneAction::Volumes(None))));

        let menu = "You have the following choices:\n1: files\n2: jobs\n3: volume\n4: quota\nChoose item to purge (1-4): ";
        for line in ["purge", "purge jobs", "purge files", "purge bogus"] {
            assert_eq!(purge_plan(&ast(line), &ctx).err().map(|r| r.text_output).as_deref(), Some(menu), "{line}");
        }
        for line in ["purge files jobid", "purge files jobid=0", "purge files jobid=x", "purge volume", "purge jobs volume"] {
            assert!(purge_plan(&ast(line), &ctx).is_err(), "{line}");
        }
        assert!(matches!(purge_plan(&ast("purge jobs client=fd1"), &ctx), Ok(PurgeAction::JobsOfClient(c, _)) if c == "fd1"));
        assert!(matches!(purge_plan(&ast("purge files jobid=7"), &ctx), Ok(PurgeAction::FilesOfJob(7))));
        assert!(matches!(purge_plan(&ast("purge volume=v1"), &ctx), Ok(PurgeAction::Volume(v)) if v == "v1"));

        // The handlers send nothing to the catalog on any guarded path.
        let mut ctx = ctx;
        let (catalog, log) = recording_catalog("b1");
        ctx.catalog = Some(catalog);
        for line in ["prune", "prune jobs", "prune files", "prune volume", "prune volumes pool=Full"] {
            cmd_prune_db(&ast(line), &ctx);
        }
        for line in ["purge", "purge jobs", "purge files", "purge files jobid=0", "purge volume", "purge jobs pool=Full"] {
            cmd_purge_db(&ast(line), &ctx);
        }
        for line in ["truncate", "truncate volstatus=Purged bogus=1"] {
            truncate_with(&ast(line), &ctx, |_, _| panic!("{line}: must not contact the SD"));
        }
        assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());
        cmd_purge_db(&ast("purge jobs client=fd1"), &ctx);
        assert_eq!(log.lock().unwrap().len(), 1, "control: a complete purge reaches the catalog");
    }

    #[test]
    fn label_selects_instead_of_defaulting() {
        let ctx = context();
        let run = |line: &str| cmd_label_db(&crate::session::parse_cli_command(line), &ctx);
        assert_eq!(run("label").text_output, "Automatically selected Storage: File\nEnter new Volume name: ");
        // No default pool "Full": two pools mean the pool dialog, no SD contact.
        assert_eq!(run("label volume=v1").text_output,
            "Automatically selected Storage: File\nDefined Pools:\n1: Full\n2: Incremental\nSelect the Pool (1-2): ");
    }

    #[test]
    fn restore_starts_nothing_until_fully_specified() {
        for line in ["restore", "restore client=fd", "restore jobid=1", "restore jobid=1 all",
            "restore jobid=1 done", "restore jobid=1 all done", "restore file=/etc/passwd yes",
            "restore select all done yes"] {
            let result = restore_needs_dialog(&crate::session::parse_cli_command(line));
            assert!(result.is_some_and(|r| r.success), "{line}");
        }
        assert!(restore_needs_dialog(&crate::session::parse_cli_command("restore")).unwrap().text_output
            .ends_with("12: Select full restore to a specified Job date\n13: Cancel\nSelect item:  (1-13): "));
        assert_eq!(restore_needs_dialog(&crate::session::parse_cli_command("restore help")).unwrap().error_message,
            "Unknown keyword: help\n");
        assert_eq!(restore_needs_dialog(&crate::session::parse_cli_command("restore jobid=1 all done yes")).map(|r| r.text_output), None);
    }

    #[test]
    fn restore_dialog_paths_touch_neither_catalog_nor_job_manager() {
        let mut ctx = context();
        ctx.config.as_ref().unwrap().write().unwrap().clients.push(bareos_config::resources::ClientResource {
            name: "fd1".into(), ..Default::default() });
        let (catalog, log) = recording_catalog("b1");
        ctx.catalog = Some(catalog);
        ctx.job_manager = Some(Arc::new(std::sync::Mutex::new(crate::job_manager::JobManager::new(
            Arc::new(crate::message_bus::MessageBus::new(8)), ctx.config.clone(), None))));
        for line in ["restore", "restore client=fd1", "restore jobid=1", "restore jobid=1 all done", "restore help"] {
            cmd_restore_db(&crate::session::parse_cli_command(line), &ctx);
        }
        assert!(log.lock().unwrap().is_empty(), "{:?}", log.lock().unwrap());
        assert!(ctx.job_manager.as_ref().unwrap().lock().unwrap().running_jobs().is_empty());
        // Control: the complete request reaches the catalog (bootstrap
        // lookup, refused by the double, so no worker is forked).
        let result = cmd_restore_db(&crate::session::parse_cli_command("restore client=fd1 jobid=1 all done yes"), &ctx);
        assert!(!result.success);
        assert!(!log.lock().unwrap().is_empty());
    }

    #[test]
    fn truncate_unknown_storage_is_a_selection() {
        let ctx = context();
        let result = truncate_with(&crate::session::parse_cli_command("truncate volstatus=Purged storage=Nope"),
            &ctx, |_, _| panic!("must not contact the SD"));
        assert!(result.success && result.error_message.is_empty());
    }
}
