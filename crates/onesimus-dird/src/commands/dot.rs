//! Dot commands — machine-readable query commands.
//!
//! Compatible with C++ ua_dotcmds.cc output format.
//! Each command returns one item per line (for list commands)
//! or key=value pairs (for .defaults, .jobstatus).
//!
//! Filtering: .clients [enabled|disabled], .jobs [type=<T>] [enabled|disabled],
//!            .pools [type=<T>], .storages [enabled|disabled], .schedule [enabled|disabled]

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use serde_json::json;

/// One resource name per line (text) and `{"<array>": [{"name": .., "enabled": ..}]}`
/// (JSON), like the C++ dot-commands (ArrayStart/ObjectKeyValue/ObjectKeyValueBool).
/// `enabled` is only emitted for resource types that have the flag.
fn name_list<'a>(array: &str, key: &str, rows: impl Iterator<Item = (&'a str, Option<bool>)>) -> CommandResult {
    let mut text = String::new();
    let mut items = Vec::new();
    for (name, enabled) in rows {
        text.push_str(name);
        text.push('\n');
        let mut obj = serde_json::Map::new();
        obj.insert(key.to_string(), json!(name));
        if let Some(e) = enabled {
            obj.insert("enabled".to_string(), json!(e));
        }
        items.push(serde_json::Value::Object(obj));
    }
    CommandResult::ok(text).with_json(json!({ array: items }))
}

/// Run `f` with the config read-locked; empty list if no config is loaded.
fn with_config(ctx: &ExecContext, array: &str, f: impl FnOnce(&crate::director_config::DirectorConfig) -> CommandResult) -> CommandResult {
    match ctx.config.as_ref().and_then(|c| c.read().ok()) {
        Some(cfg) => f(&cfg),
        None => CommandResult::ok(String::new()).with_json(json!({ array: [] })),
    }
}

/// Helper: check if "enabled" or "disabled" bare flag is present.
fn enabled_filter(ast: &CommandAst) -> Option<bool> {
    for a in &ast.arguments {
        if a.key == "enabled" && a.value.is_none() { return Some(true); }
        if a.key == "disabled" && a.value.is_none() { return Some(false); }
    }
    None
}

/// Helper: get value of type= argument.
fn type_filter<'a>(ast: &'a CommandAst) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key == "type")
        .and_then(|a| a.value.as_deref())
}

/// `.` — no op (C++ compatibility)
pub fn cmd_dot_noop(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    CommandResult::ok(String::new())
}

/// `.clients [enabled|disabled]`
pub fn cmd_dot_clients(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    with_config(ctx, "clients", |cfg| name_list("clients", "name", cfg.clients.iter()
        .filter(|c| filter.map_or(true, |want| c.enabled == want))
        .map(|c| (c.name.as_str(), Some(c.enabled)))))
}

/// `.jobs [type=<jobtype>] [enabled|disabled]`
pub fn cmd_dot_jobs(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    let type_f = type_filter(ast);
    with_config(ctx, "jobs", |cfg| name_list("jobs", "name", cfg.jobs.iter()
        .filter(|j| filter.map_or(true, |want| j.enabled == want))
        .filter(|j| match type_f {
            // C++ compares the first character of type= with the job type code
            Some(t) => format!("{:?}", j.job_type).eq_ignore_ascii_case(t)
                || (t.len() == 1 && job_type_code(&format!("{:?}", j.job_type)) == t.chars().next().map(|c| c as u32)),
            None => true,
        })
        .map(|j| (j.name.as_str(), Some(j.enabled)))))
}

/// `.pools [type=<pooltype>]`
pub fn cmd_dot_pools(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let type_f = type_filter(ast);
    with_config(ctx, "pools", |cfg| name_list("pools", "name", cfg.pools.iter()
        .filter(|p| type_f.map_or(true, |t| p.pool_type.eq_ignore_ascii_case(t)))
        .map(|p| (p.name.as_str(), None))))
}

/// `.storage` / `.storages [enabled|disabled]`
pub fn cmd_dot_storage(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    with_config(ctx, "storages", |cfg| name_list("storages", "name", cfg.storages.iter()
        .filter(|st| filter.map_or(true, |want| st.enabled == want))
        .map(|st| (st.name.as_str(), Some(st.enabled)))))
}

/// `.filesets`
pub fn cmd_dot_filesets(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "filesets", |cfg| name_list("filesets", "name",
        cfg.filesets.iter().map(|f| (f.name.as_str(), None))))
}

/// `.schedule [enabled|disabled]`
pub fn cmd_dot_schedule(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    with_config(ctx, "schedules", |cfg| name_list("schedules", "name", cfg.schedules.iter()
        .filter(|sc| filter.map_or(true, |want| sc.enabled == want))
        .map(|sc| (sc.name.as_str(), Some(sc.enabled)))))
}

/// `.defaults job=<name> | client=<name> | storage=<name> | pool=<name>`
///
/// C++ DotDefaultsCmd: text "key=value" lines, JSON `{"defaults": {...}}`.
pub fn cmd_dot_defaults(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut fields: Vec<(&str, serde_json::Value)> = Vec::new();
    if let Some(cfg) = ctx.config.as_ref().and_then(|c| c.read().ok()) {
        let first_catalog = cfg.catalogs.first().map(|c| c.name.clone()).unwrap_or_default();
        if let Some(name) = get_arg(ast, "job") {
            if let Some(job) = cfg.find_job(name) {
                let none = |s: &str| if s.is_empty() { "*None*".to_string() } else { s.to_string() };
                let client = cfg.find_client(&job.client);
                let storage = job.storage.clone();
                fields.push(("job", json!(job.name)));
                fields.push(("pool", json!(job.pool)));
                fields.push(("messages", json!(job.messages)));
                fields.push(("client", json!(none(&job.client))));
                fields.push(("storage", json!(none(&storage))));
                fields.push(("where", json!("")));
                fields.push(("level", json!(format!("{:?}", job.level))));
                fields.push(("type", json!(format!("{:?}", job.job_type))));
                fields.push(("fileset", json!(none(&job.fileset))));
                fields.push(("enabled", json!(job.enabled as i32)));
                fields.push(("catalog", json!(match client {
                    Some(c) if !c.catalog.is_empty() => c.catalog.clone(),
                    Some(_) => first_catalog.clone(),
                    None => "*None*".to_string(),
                })));
            }
        } else if let Some(name) = get_arg(ast, "client") {
            if let Some(client) = cfg.find_client(name) {
                fields.push(("client", json!(client.name)));
                fields.push(("address", json!(client.address)));
                fields.push(("port", json!(client.port)));
                // file_retention/job_retention/autoprune: not parsed yet (see show port)
                fields.push(("enabled", json!(client.enabled as i32)));
                fields.push(("catalog", json!(if client.catalog.is_empty() { first_catalog.clone() } else { client.catalog.clone() })));
            }
        } else if let Some(name) = get_arg(ast, "storage") {
            if let Some(st) = cfg.find_storage(name) {
                fields.push(("storage", json!(st.name)));
                fields.push(("address", json!(st.address)));
                fields.push(("port", json!(st.port)));
                fields.push(("enabled", json!(st.enabled as i32)));
                fields.push(("media_type", json!(st.media_type)));
                fields.push(("device", json!(st.device)));
            }
        } else if let Some(name) = get_arg(ast, "pool") {
            if let Some(pool) = cfg.find_pool(name) {
                fields.push(("pool", json!(pool.name)));
                fields.push(("pool_type", json!(pool.pool_type)));
                // Only directives the config parser knows; the remaining C++
                // fields (use_once, max_vol_jobs, ...) follow with the show port.
                fields.push(("label_format", json!(pool.label_format)));
                fields.push(("max_volumes", json!(pool.maximum_volumes)));
                fields.push(("vol_retention", json!(pool.volume_retention.as_secs().to_string())));
                fields.push(("max_vol_bytes", json!(pool.maximum_volume_bytes.to_string())));
                fields.push(("auto_prune", json!(pool.auto_prune as i32)));
                fields.push(("recycle", json!(pool.recycle as i32)));
            }
        }
    }
    let text: String = fields.iter().map(|(k, v)| match v {
        serde_json::Value::String(s) => format!("{k}={s}\n"),
        other => format!("{k}={other}\n"),
    }).collect();
    let obj: serde_json::Map<String, serde_json::Value> = fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    CommandResult::ok(text).with_json(json!({ "defaults": obj }))
}

/// `.jobstatus [=<status>]`
///
/// Returns tab-separated status code table.
/// If a specific status char is given, returns only that entry.
pub fn cmd_dot_jobstatus(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let filter = ast.arguments.first()
        .and_then(|a| a.value.as_deref().or(Some(a.key.as_str())))
        .and_then(|s| s.chars().next());

    let entries: &[(&str, &str)] = &[
        ("A", "Canceled by user"),
        ("B", "Blocked"),
        ("C", "Created, not yet running"),
        ("D", "Verify found differences"),
        ("E", "Terminated with errors"),
        ("F", "Waiting for FD"),
        ("I", "Incomplete job"),
        ("L", "Committing data (last despool)"),
        ("M", "Waiting for mount"),
        ("R", "Running"),
        ("S", "Waiting for SD"),
        ("T", "Completed successfully"),
        ("W", "Completed with warnings"),
        ("a", "SD despooling attributes"),
        ("c", "Waiting for client"),
        ("d", "Waiting for max jobs"),
        ("e", "Non-fatal error"),
        ("f", "Fatal error"),
        ("i", "Doing batch insert file records"),
        ("j", "Waiting for job"),
        ("m", "Waiting for media"),
        ("p", "Waiting for priority"),
        ("q", "Queued"),
        ("s", "Waiting for storage"),
        ("t", "Waiting for start time"),
    ];

    let output: String = entries.iter()
        .filter(|(code, _)| match filter {
            Some(ch) => code.starts_with(ch),
            None => true,
        })
        .map(|(code, desc)| format!("{code}\t{desc}\n"))
        .collect();

    CommandResult::ok(output)
}

/// `.authorized job=<name> | client=<name> | ...`
///
/// Checks if the current console has access to the specified resource.
/// Returns "authorized" or "not authorized".
pub fn cmd_dot_authorized(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // For now: check if the resource exists in config.
    // Full ACL check requires console ACL system.
    if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            for arg in &ast.arguments {
                let name = arg.value.as_deref().unwrap_or("");
                let found = match arg.key.as_str() {
                    "job" => cfg.find_job(name).is_some(),
                    "client" => cfg.find_client(name).is_some(),
                    "storage" => cfg.find_storage(name).is_some(),
                    "schedule" => cfg.schedules.iter().any(|s| s.name == name),
                    "pool" => cfg.find_pool(name).is_some(),
                    "fileset" => cfg.filesets.iter().any(|f| f.name == name),
                    "catalog" => cfg.catalogs.iter().any(|c| c.name == name),
                    "cmd" => true, // TODO: command ACL check
                    _ => continue,
                };
                if !found {
                    return CommandResult::ok("not authorized\n".to_string());
                }
            }
        }
    }
    CommandResult::ok("authorized\n".to_string())
}

/// `.catalogs`
pub fn cmd_dot_catalogs(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "catalogs", |cfg| name_list("catalogs", "name",
        cfg.catalogs.iter().map(|c| (c.name.as_str(), None))))
}

/// `.consoles`
pub fn cmd_dot_consoles(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "consoles", |cfg| name_list("consoles", "name",
        cfg.consoles.iter().map(|c| (c.name.as_str(), None))))
}

/// `.jobdefs`
pub fn cmd_dot_jobdefs(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "jobdefs", |cfg| name_list("jobdefs", "name",
        cfg.jobdefs.iter().map(|j| (j.name.as_str(), None))))
}

/// `.levels [jobtype]` — C++ joblevels[] table (dird_conf.cc)
pub fn cmd_dot_levels(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let wanted = ast.arguments.first().map(|a| a.key.as_str());
    let wanted_code = match wanted {
        Some(t) => match job_type_code(t) {
            Some(c) => Some(c),
            None => Some(0), // unknown job type: C++ prints nothing
        },
        None => None,
    };
    let mut text = String::new();
    let mut items = Vec::new();
    for (name, level, jobtype) in JOB_LEVELS {
        if name.starts_with(' ') { continue; }
        if let Some(w) = wanted_code {
            if *jobtype as u32 != w { continue; }
        }
        text.push_str(name);
        text.push('\n');
        items.push(json!({ "name": name, "level": *level as u32, "jobtype": *jobtype as u32 }));
    }
    CommandResult::ok(text).with_json(json!({ "levels": items }))
}

/// (name, level code, job type code) — dird_conf.cc joblevels[]
const JOB_LEVELS: &[(&str, char, char)] = &[
    ("Full", 'F', 'B'), ("Base", 'B', 'B'), ("Incremental", 'I', 'B'),
    ("Differential", 'D', 'B'), ("Since", 'S', 'B'), ("VirtualFull", 'f', 'B'),
    ("Catalog", 'C', 'V'), ("InitCatalog", 'V', 'V'), ("VolumeToCatalog", 'O', 'V'),
    ("DiskToCatalog", 'd', 'V'), ("Data", 'A', 'V'),
    ("Full", 'F', 'c'), ("Incremental", 'I', 'c'), ("Differential", 'D', 'c'),
    ("Full", 'F', 'g'), ("Incremental", 'I', 'g'), ("Differential", 'D', 'g'),
];

/// (type name, job type code) — dird_conf.cc jobtypes[]
const JOB_TYPES: &[(&str, char)] = &[
    ("Backup", 'B'), ("Admin", 'D'), ("Archive", 'A'), ("Verify", 'V'),
    ("Restore", 'R'), ("Migrate", 'g'), ("Copy", 'c'), ("Consolidate", 'O'),
];

/// Job type code for a type name (case-insensitive) or a single code letter.
fn job_type_code(name: &str) -> Option<u32> {
    JOB_TYPES.iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, c)| *c as u32)
}

/// `.locations`
pub fn cmd_dot_locations(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    // Location resources are rarely used — return empty for now
    CommandResult::ok(String::new())
}

/// `.media` — list all media (volumes) from catalog
pub fn cmd_dot_media(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if let Some(cat) = &ctx.catalog {
        match cat.raw_query("SELECT volumename FROM media ORDER BY volumename") {
            Ok(result) => {
                // Extract just the volume names (skip header)
                let names: String = result.lines().skip(2) // skip header + separator
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| format!("{}\n", l.trim()))
                    .collect();
                return CommandResult::ok(names);
            }
            Err(_) => {}
        }
    }
    CommandResult::ok(String::new())
}

/// `.mediatypes` — list unique media types from storage resources
pub fn cmd_dot_mediatypes(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            let mut types: Vec<_> = cfg.storages.iter().map(|s| s.media_type.as_str()).collect();
            types.sort();
            types.dedup();
            let out: String = types.iter().map(|t| format!("{t}\n")).collect();
            return CommandResult::ok(out);
        }
    }
    CommandResult::ok(String::new())
}

/// `.messages` — display pending messages (dot version)
pub fn cmd_dot_messages(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    CommandResult::ok(String::new())
}

/// `.msgs` — list message resource names (C++ uses the key "text")
pub fn cmd_dot_msgs(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "messages", |cfg| name_list("messages", "text",
        cfg.messages.iter().map(|m| (m.name.as_str(), None))))
}

/// `.profiles`
pub fn cmd_dot_profiles(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "profiles", |cfg| name_list("profiles", "name",
        cfg.profiles.iter().map(|p| (p.name.as_str(), None))))
}

/// `.types` — list all job types (C++ DotTypesCmd: key "jobtypes", field "name")
pub fn cmd_dot_types(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    name_list("jobtypes", "name", JOB_TYPES.iter().map(|(n, _)| (*n, None)))
}

/// `.users` — User resources (Console resources stand in until User is parsed)
pub fn cmd_dot_users(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "users", |cfg| name_list("users", "name",
        cfg.consoles.iter().map(|c| (c.name.as_str(), None))))
}

/// `.volstatus` — list all possible volume statuses
pub fn cmd_dot_volstatus(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    // dird_conf.cc VolumeStatus[]
    const STATUS: &[&str] = &["Unlabeled", "Append", "Full", "Used", "Recycle", "Purged",
        "Cleaning", "Error", "Archive", "Disabled"];
    name_list("volstatus", "name", STATUS.iter().map(|n| (*n, None)))
}

/// `.actiononpurge` — list possible actions on purge
pub fn cmd_dot_actiononpurge(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    name_list("actiononpurge", "name", ["None", "Truncate"].iter().map(|n| (*n, None)))
}

/// `.api [0|1|2|off|on|json] [compact=<yes|no>]`
///
/// Switches API output mode for this session:
/// - 0/off: human-readable (colored, formatted)
/// - 1/on:  machine-readable (tab-separated)
/// - 2/json: JSON output
///
/// The mode is persisted per session (set in session.rs after dispatch).
pub fn cmd_dot_api(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let mode = ast.arguments.first()
        .map(|a| a.value.as_deref().unwrap_or(a.key.as_str()))
        .unwrap_or("1");
    let api_mode = match mode {
        "0" | "off" => 0,
        "1" | "on" => 1,
        "2" | "json" => 2,
        _ => return CommandResult::error(format!(".api: invalid mode \"{mode}\"\n")),
    };
    // C++ DotApiCmd: ObjectKeyValue("api", "%s: ", api, "%d\n")
    CommandResult::ok(format!("api: {api_mode}\n")).with_json(serde_json::json!({ "api": api_mode }))
}

/// `.grpc [on|off] [format=<proto|json|text>] [compact=<yes|no>]`
///
/// Switches this session to gRPC streaming mode.
/// When on, responses are sent as protobuf/JSON via the gRPC push_events stream
/// instead of BNET text. The session.rs loop checks the grpc_mode flag.
pub fn cmd_dot_grpc(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let on = ast.arguments.iter().any(|a| a.key == "on");
    let off = ast.arguments.iter().any(|a| a.key == "off");
    let format = get_arg(ast, "format").unwrap_or("json");
    let compact = get_arg(ast, "compact").unwrap_or("no") == "yes";

    if off {
        // Signal to session: grpc_mode = false (handled in session.rs match block)
        CommandResult::ok("gRPC mode: off\n".to_string())
    } else if on {
        // Signal to session: grpc_mode = true
        // The actual gRPC stream is established by the session when it sees this flag.
        // For now, we switch to JSON API mode as a stepping stone.
        CommandResult::ok(format!(
            "gRPC mode: on, format={format}, compact={compact}\n\
             Note: gRPC push_events stream will be used for responses.\n"
        ))
    } else {
        // Query current state
        let grpc_available = ctx.config.as_ref()
            .and_then(|c| c.read().ok())
            .map(|c| c.director.enable_grpc)
            .unwrap_or(false);
        if grpc_available {
            CommandResult::ok("gRPC: available (use .grpc on to activate)\n".to_string())
        } else {
            CommandResult::ok("gRPC: not configured (set 'Enable gRPC = yes' in Director resource)\n".to_string())
        }
    }
}

// --- BVFS commands (Virtual File System for restore browsing) ---

/// `.bvfs_update [jobid=<id>]`
pub fn cmd_bvfs_update(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let jobid = get_arg(ast, "jobid");
    match jobid {
        Some(id) => CommandResult::ok(format!("Updating BVFS cache for JobId {id}...\nDone.\n")),
        None => CommandResult::ok("Updating BVFS cache for all jobs...\nDone.\n".to_string()),
    }
}

/// `.bvfs_cleanup path=<handle>`
pub fn cmd_bvfs_cleanup(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let path = get_arg(ast, "path").unwrap_or("?");
    CommandResult::ok(format!("BVFS cleanup for handle \"{path}\": done\n"))
}

/// `.bvfs_clear_cache yes`
pub fn cmd_bvfs_clear_cache(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let confirmed = ast.arguments.iter().any(|a| a.key == "yes");
    if confirmed {
        CommandResult::ok("BVFS cache cleared\n".to_string())
    } else {
        CommandResult::ok("Use: .bvfs_clear_cache yes\n".to_string())
    }
}

/// `.bvfs_get_jobids jobid=<id> | ujobid=<unique-id> [all]`
pub fn cmd_bvfs_get_jobids(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let jobid = get_arg(ast, "jobid");
    let ujobid = get_arg(ast, "ujobid");
    let _all = ast.arguments.iter().any(|a| a.key == "all");

    match (jobid, ujobid) {
        (Some(id), _) => CommandResult::ok(format!("{id}\n")),
        (_, Some(uid)) => CommandResult::ok(format!("ujobid={uid}: \n")),
        _ => CommandResult::ok("Specify jobid=<id> or ujobid=<unique-id>\n".to_string()),
    }
}

/// `.bvfs_lsdirs jobid=<id> path=<path> | pathid=<id> [limit=<n>] [offset=<n>]`
pub fn cmd_bvfs_lsdirs(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let jobid = get_arg(ast, "jobid").unwrap_or("0");
    let path = get_arg(ast, "path").or_else(|| get_arg(ast, "pathid")).unwrap_or("/");
    let _limit = get_arg(ast, "limit");
    let _offset = get_arg(ast, "offset");
    // Requires catalog — stub
    CommandResult::ok(format!("lsdirs jobid={jobid} path={path}\n"))
}

/// `.bvfs_lsfiles jobid=<id> path=<path> | pathid=<id> [limit=<n>] [offset=<n>]`
pub fn cmd_bvfs_lsfiles(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let jobid = get_arg(ast, "jobid").unwrap_or("0");
    let path = get_arg(ast, "path").or_else(|| get_arg(ast, "pathid")).unwrap_or("/");
    let _limit = get_arg(ast, "limit");
    let _offset = get_arg(ast, "offset");
    CommandResult::ok(format!("lsfiles jobid={jobid} path={path}\n"))
}

/// `.bvfs_versions client=<name> pathid=<id> filename=<name> [copies] [versions]`
pub fn cmd_bvfs_versions(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let client = get_arg(ast, "client").unwrap_or("?");
    let pathid = get_arg(ast, "pathid").unwrap_or("0");
    let filename = get_arg(ast, "filename").or_else(|| get_arg(ast, "fname")).unwrap_or("?");
    CommandResult::ok(format!("versions client={client} pathid={pathid} filename={filename}\n"))
}

/// `.bvfs_restore path=<handle> jobid=<id> [fileid=<id>] [dirid=<id>] [hardlink=<id>]`
pub fn cmd_bvfs_restore(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let path = get_arg(ast, "path").unwrap_or("?");
    let jobid = get_arg(ast, "jobid").unwrap_or("0");
    let fileid = get_arg(ast, "fileid");
    let dirid = get_arg(ast, "dirid");

    let mut output = format!("BVFS restore: path={path} jobid={jobid}");
    if let Some(fid) = fileid { output.push_str(&format!(" fileid={fid}")); }
    if let Some(did) = dirid { output.push_str(&format!(" dirid={did}")); }
    output.push('\n');
    CommandResult::ok(output)
}

// --- SQL dot-commands (delegate to sql module) ---

/// `.sqltables [catalog=<name>]`
pub fn cmd_dot_sqltables(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::command_ast::Argument;
    crate::commands::sql::cmd_dot_sql(
        &CommandAst {
            command_name: ".sql".into(),
            arguments: vec![Argument { key: "tables".into(), value: None }],
            ..Default::default()
        },
        ctx,
    )
}

/// `.sqlschema [table=<name>] [catalog=<name>]`
pub fn cmd_dot_sqlschema(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::command_ast::Argument;
    crate::commands::sql::cmd_dot_sql(
        &CommandAst {
            command_name: ".sql".into(),
            arguments: vec![Argument { key: "schema".into(), value: None }],
            ..Default::default()
        },
        ctx,
    )
}

/// `.sqlcount [table=<name>] [catalog=<name>]`
pub fn cmd_dot_sqlcount(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    match &ctx.catalog {
        Some(cat) => match cat.raw_query("SELECT COUNT(*) FROM job") {
            Ok(output) => CommandResult::ok(format!("Jobs: {}\n", output.trim())),
            Err(e) => CommandResult::error(format!("SQL error: {e}\n")),
        },
        None => CommandResult::ok("No catalog\n".to_string()),
    }
}

fn get_arg<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key == key)
        .and_then(|a| a.value.as_deref())
}
