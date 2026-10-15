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
    super::defaults::cmd_defaults(ast, ctx)
}

/// `.jobstatus [=<status>]`
///
/// Returns tab-separated status code table.
/// If a specific status char is given, returns only that entry.
pub fn cmd_dot_jobstatus(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ DotJobstatusCmd: OpenClientDb(ua) before the query.
    let prefix = match crate::ua_db::open_client_db(ast, ctx, false) {
        Ok(prefix) => prefix,
        Err(result) => return result,
    };
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

    crate::ua_db::with_prefix(prefix, CommandResult::ok(output))
}

/// `.authorized job=<name> | client=<name> | ...`
///
/// Checks if the current console has access to the specified resource.
/// Returns "authorized" or "not authorized".
pub fn cmd_dot_authorized(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut authorized=false;
    for argument in &ast.arguments {
        if let Some(kind)=crate::acl::Kind::selector(&argument.key) {
            authorized=argument.value.as_deref().is_some_and(|name|crate::acl::access(ctx,kind,name));
        }
    }
    let mut result=CommandResult::ok(if authorized {"authorized\n"} else {"not authorized\n"}.into())
        .with_json(serde_json::json!({".authorized":{"authorized":authorized}}));
    result.success=authorized;
    result
}

/// `.catalogs`
pub fn cmd_dot_catalogs(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "catalogs", |cfg| name_list("catalogs", "name",
        cfg.catalogs.iter().map(|c| (c.name.as_str(), None))))
}

/// `.consoles` — the Console chain, which ends with the internal `*UserAgent*`
/// console the C++ Director appends after loading the configuration.
pub fn cmd_dot_consoles(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::director_config::USER_AGENT_CONSOLE;
    with_config(ctx, "consoles", |cfg| name_list("consoles", "name",
        cfg.consoles.iter().map(|c| c.name.as_str())
            .chain((!cfg.director.name.is_empty()).then_some(USER_AGENT_CONSOLE))
            .map(|name| (name, None))))
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
pub fn cmd_dot_locations(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotLocationsCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_dot_locations_db(ast, ctx))
}

fn cmd_dot_locations_db(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    // Location resources are rarely used — return empty for now
    CommandResult::ok(String::new())
}

/// `.media` — list all media (volumes) from catalog
pub fn cmd_dot_media(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotMediaCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_dot_media_db(ast, ctx))
}

fn cmd_dot_media_db(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
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
pub fn cmd_dot_mediatypes(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotMediatypesCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_dot_mediatypes_db(ast, ctx))
}

fn cmd_dot_mediatypes_db(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
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
pub fn cmd_dot_messages(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::commands::extra::cmd_messages(ast, ctx)
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

/// `.users` lists User resources, independently of Console resources.
pub fn cmd_dot_users(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    with_config(ctx, "users", |cfg| name_list("users", "name",
        cfg.raw.iter().filter(|resource| resource.norm_type() == "user")
            .filter_map(|resource| resource.name().map(|name| (name, None)))))
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

#[cfg(test)]
mod users_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    #[test]
    fn users_are_not_consoles_and_keep_source_order() {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nConsole { Name = console-only }\nUser { Name = alice }\nUser { Name = bob }"
        ).unwrap();
        let ctx = ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let result = cmd_dot_users(&CommandAst::default(), &ctx);
        assert_eq!(result.text_output, "alice\nbob\n");
        assert_eq!(result.json.unwrap(), json!({"users":[{"name":"alice"},{"name":"bob"}]}));
    }

    #[test]
    fn consoles_keep_chain_order_and_end_with_user_agent() {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nConsole { Name = zeta }\nConsole { Name = alpha }"
        ).unwrap();
        let ctx = ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let result = cmd_dot_consoles(&CommandAst::default(), &ctx);
        assert_eq!(result.text_output, "zeta\nalpha\n*UserAgent*\n");
        assert_eq!(result.json.unwrap(),
            json!({"consoles":[{"name":"zeta"},{"name":"alpha"},{"name":"*UserAgent*"}]}));
    }
}
