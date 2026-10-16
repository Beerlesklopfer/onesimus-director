//! Dot commands — machine-readable query commands.
//!
//! Compatible with C++ ua_dotcmds.cc output format.
//! Each command returns one item per line (for list commands)
//! or key=value pairs (for .defaults); .jobstatus lists the catalog Status table.
//!
//! Filtering: .clients [enabled|disabled], .jobs [type=<T>] [enabled|disabled],
//!            .pools [type=<T>], .storages [enabled|disabled], .schedule [enabled|disabled]

use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;
use serde_json::json;
use crate::acl::Kind;

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

/// Names that pass `ua->AclAccessOk(acl, name)` (audit_event=false), the
/// filter of the ua_dotcmds.cc resource listings (compatibility log C-025).
/// acl::access takes the config lock itself, so this runs before `with_config`.
fn permitted(ctx: &ExecContext, kind: Kind, names: impl FnOnce(&crate::director_config::DirectorConfig) -> Vec<String>) -> Vec<String> {
    ctx.config.as_ref().and_then(|c| c.read().ok().map(|cfg| names(&cfg))).unwrap_or_default()
        .into_iter().filter(|name| crate::acl::access(ctx, kind, name)).collect()
}

/// C++ `enabled = FindArg(ua, "enabled") >= 0; disabled = FindArg(ua,
/// "disabled") >= 0;` (keywords case-insensitive, a value does not matter)
/// and the loop filter `if (enabled && !res->enabled) continue; if
/// (disabled && res->enabled) continue;` — both keywords list nothing.
fn enabled_filter(ast: &CommandAst) -> impl Fn(bool) -> bool {
    let enabled = crate::ua_select::find_arg(ast, "enabled").is_some();
    let disabled = crate::ua_select::find_arg(ast, "disabled").is_some();
    move |is_enabled| !(enabled && !is_enabled) && !(disabled && is_enabled)
}

/// C++ FindArgWithValue(ua, "type"): value of the first `type` keyword
/// (case-insensitive); None when that one has no value.
fn type_filter<'a>(ast: &'a CommandAst) -> Option<&'a str> {
    crate::ua_select::find_arg_with_value(ast, "type")
}

/// `.` — no op (C++ compatibility)
pub fn cmd_dot_noop(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    CommandResult::ok(String::new())
}

/// `.clients [enabled|disabled]`
pub fn cmd_dot_clients(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    let allowed = permitted(ctx, Kind::Client, |cfg| cfg.clients.iter().map(|c| c.name.clone()).collect());
    with_config(ctx, "clients", |cfg| name_list("clients", "name", cfg.clients.iter()
        .filter(|c| allowed.contains(&c.name))
        .filter(|c| filter(c.enabled))
        .map(|c| (c.name.as_str(), Some(c.enabled)))))
}

/// `.jobs [type=<jobtype>] [enabled|disabled]`
pub fn cmd_dot_jobs(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    let type_f = type_filter(ast);
    let allowed = permitted(ctx, Kind::Job, |cfg| cfg.jobs.iter().map(|j| j.name.clone()).collect());
    with_config(ctx, "jobs", |cfg| name_list("jobs", "name", cfg.jobs.iter()
        .filter(|j| allowed.contains(&j.name))
        // C++ `type = ua->argv[pos][0]` compared with the job type code
        // (case-sensitive, only the first byte; `type=` lists all).
        .filter(|j| match type_f.and_then(|t| t.bytes().next()) {
            Some(t) => job_type_code(&format!("{:?}", j.job_type)) == Some(t as u32),
            None => true,
        })
        .filter(|j| filter(j.enabled))
        .map(|j| (j.name.as_str(), Some(j.enabled)))))
}

/// `.pools [type=<pooltype>]`
pub fn cmd_dot_pools(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let type_f = type_filter(ast);
    let allowed = permitted(ctx, Kind::Pool, |cfg| cfg.pools.iter().map(|p| p.name.clone()).collect());
    with_config(ctx, "pools", |cfg| name_list("pools", "name", cfg.pools.iter()
        .filter(|p| allowed.contains(&p.name))
        // C++ bstrncasecmp(pool_type, type, strlen(type)) (ua_dotcmds.cc:894):
        // type= is a case-insensitive prefix of the pool type.
        .filter(|p| type_f.map_or(true, |t| p.pool_type.len() >= t.len()
            && p.pool_type.as_bytes()[..t.len()].eq_ignore_ascii_case(t.as_bytes())))
        .map(|p| (p.name.as_str(), None))))
}

/// `.storage` / `.storages [enabled|disabled]`
pub fn cmd_dot_storage(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    let allowed = permitted(ctx, Kind::Storage, |cfg| cfg.storages.iter().map(|st| st.name.clone()).collect());
    with_config(ctx, "storages", |cfg| name_list("storages", "name", cfg.storages.iter()
        .filter(|st| allowed.contains(&st.name))
        .filter(|st| filter(st.enabled))
        .map(|st| (st.name.as_str(), Some(st.enabled)))))
}

/// `.filesets`
pub fn cmd_dot_filesets(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let allowed = permitted(ctx, Kind::FileSet, |cfg| cfg.filesets.iter().map(|f| f.name.clone()).collect());
    with_config(ctx, "filesets", |_| name_list("filesets", "name",
        allowed.iter().map(|name| (name.as_str(), None))))
}

/// `.schedule [enabled|disabled]`
pub fn cmd_dot_schedule(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let filter = enabled_filter(ast);
    let allowed = permitted(ctx, Kind::Schedule, |cfg| cfg.schedules.iter().map(|sc| sc.name.clone()).collect());
    with_config(ctx, "schedules", |cfg| name_list("schedules", "name", cfg.schedules.iter()
        .filter(|sc| allowed.contains(&sc.name))
        .filter(|sc| filter(sc.enabled))
        .map(|sc| (sc.name.as_str(), Some(sc.enabled)))))
}

/// `.defaults job=<name> | client=<name> | storage=<name> | pool=<name>`
///
/// C++ DotDefaultsCmd: text "key=value" lines, JSON `{"defaults": {...}}`.
pub fn cmd_dot_defaults(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    super::defaults::cmd_defaults(ast, ctx)
}

/// `.jobstatus[=<status>]` — C++ ua_dotcmds.cc DotJobstatusCmd: only the
/// value of the command word itself (`ua->argv[0]`) filters; it must be a
/// single character. The rows come from the catalog Status table
/// (get_jobstatus_details), listed with ListSqlQuery(HORZ_LIST).
pub fn cmd_dot_jobstatus(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // ua->argv[0]: the value of the command word (`.jobstatus=T`).
    let argv0 = crate::session::split_args(&ast.raw_line).into_iter().next()
        .and_then(|token| token.split_once('=').map(|(_, v)| v.to_string()));
    let mut filter = String::new();
    if let Some(status) = argv0.as_deref() {
        let mut chars = status.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii() => filter = format!("WHERE JobStatus = '{c}' "),
            _ => return CommandResult::error(format!(
                "Unknown JobStatus '{status}'. JobStatus must be a single character.\n")),
        }
    }
    crate::ua_db::with_client_db(ast, ctx, false, || {
        let Some(cat) = &ctx.catalog else {
            return CommandResult { success: false, ..Default::default() };
        };
        // cats/postgresql_queries.inc 0062_get_jobstatus_details
        let sql = format!("SELECT DISTINCT JobStatus, JobStatusLong, Severity,
       CASE
       WHEN JobStatus in ('T') THEN '0'
       WHEN JobStatus in ('W') THEN '1'
       WHEN JobStatus in ('e', 'f', 'A', 'E') THEN '2'
       END as ExitLevel,
       CASE
       WHEN JobStatus in ('T') THEN 'Ok'
       WHEN JobStatus in ('W') THEN 'Warning'
       WHEN JobStatus in ('e', 'f', 'A', 'E') THEN 'Error'
       END as ExitStatus
  FROM Status {filter}
 ORDER BY JobStatus
");
        match cat.query_rows(&sql) {
            Ok(t) => {
                let text = match ctx.api_mode {
                    ApiMode::Api => horz_api1(&t),
                    _ => crate::sql_list::render(&t, crate::sql_list::ListType::Horz),
                };
                CommandResult::ok(text)
                    .with_json(json!({ "jobstatus": crate::sql_list::json_rows(&t) }))
            }
            // ListSqlQuery(..., verbose=false): errmsg is not sent.
            Err(_) => CommandResult { success: false, ..Default::default() },
        }
    })
}

/// ListResult(HORZ_LIST) under `.api 1`: the Decoration() parts (dashes,
/// header, leading "|", "No results to list.") are suppressed, only the
/// preformatted cells " <value> |" of each row (newline after the last) remain.
/// Interim until sql_list::render() itself is API-1 aware (C-028); an
/// already stripped rendering is passed through unchanged.
fn horz_api1(t: &crate::catalog_worker::SqlTable) -> String {
    if t.rows.is_empty() {
        return String::new();
    }
    let table = crate::sql_list::render(t, crate::sql_list::ListType::Horz);
    if !table.starts_with('+') {
        return table;
    }
    let lines: Vec<&str> = table.lines().collect();
    lines[3..lines.len() - 1].iter()
        .map(|l| format!("{}\n", l.strip_prefix('|').unwrap_or(l)))
        .collect()
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
    let allowed = permitted(ctx, Kind::Catalog, |cfg| cfg.catalogs.iter().map(|c| c.name.clone()).collect());
    with_config(ctx, "catalogs", |_| name_list("catalogs", "name",
        allowed.iter().map(|name| (name.as_str(), None))))
}

/// `.consoles` — the Console chain, which ends with the internal `*UserAgent*`
/// console the C++ Director appends after loading the configuration
/// (consoles added later by `configure add` follow it).
pub fn cmd_dot_consoles(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::director_config::USER_AGENT_CONSOLE;
    with_config(ctx, "consoles", |cfg| {
        let (loaded, added) = cfg.consoles.split_at(cfg.loaded_consoles.min(cfg.consoles.len()));
        name_list("consoles", "name",
            loaded.iter().map(|c| c.name.as_str())
                .chain((!cfg.director.name.is_empty()).then_some(USER_AGENT_CONSOLE))
                .chain(added.iter().map(|c| c.name.as_str()))
                .map(|name| (name, None)))
    })
}

/// `.jobdefs`
pub fn cmd_dot_jobdefs(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let allowed = permitted(ctx, Kind::Job, |cfg| cfg.jobdefs.iter().map(|j| j.name.clone()).collect());
    with_config(ctx, "jobdefs", |_| name_list("jobdefs", "name",
        allowed.iter().map(|name| (name.as_str(), None))))
}

/// `.levels [jobtype]` — C++ joblevels[] table (dird_conf.cc)
pub fn cmd_dot_levels(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    // C++ DotLevelsCmd: argc 1 lists all, argc 2 filters by argk[1],
    // more arguments list nothing.
    if ast.arguments.len() > 1 {
        return CommandResult::ok(String::new()).with_json(json!({ "levels": [] }));
    }
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

fn cmd_dot_locations_db(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    one_column(ctx, "locations", "SELECT DISTINCT Location FROM Location ORDER BY Location", "List Location")
}

/// `.media` — list all media (volumes) from catalog
pub fn cmd_dot_media(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotMediaCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_dot_media_db(ast, ctx))
}

fn cmd_dot_media_db(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    one_column(ctx, "media", "SELECT DISTINCT Media.VolumeName FROM Media ORDER BY VolumeName", "List Media")
}

/// `.mediatypes` — the media types of the catalog MediaType table
pub fn cmd_dot_mediatypes(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotMediatypesCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_dot_mediatypes_db(ast, ctx))
}

fn cmd_dot_mediatypes_db(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    one_column(ctx, "mediatypes", "SELECT DISTINCT MediaType FROM MediaType ORDER BY MediaType", "List MediaType")
}

/// C++ ua_dotcmds.cc SqlQuery(..., OneHandler): the first column of every
/// row as `{"name": ..}` inside `array`; a failed query is reported with
/// ErrorMsg("<what> failed: ERR=..") and the command still succeeds.
fn one_column(ctx: &ExecContext, array: &str, sql: &str, what: &str) -> CommandResult {
    let Some(cat) = &ctx.catalog else {
        return CommandResult::ok(String::new()).with_json(json!({ array: [] }));
    };
    match cat.query_rows(sql) {
        Ok(t) => name_list(array, "name", t.rows.iter()
            .map(|row| (row.first().and_then(|v| v.as_deref()).unwrap_or(""), None))),
        Err(e) => CommandResult::ok_with_errors(String::new(), format!("{what} failed: ERR={e}\n"))
            .with_json(json!({ array: [] })),
    }
}

/// `.messages` — display pending messages (dot version)
pub fn cmd_dot_messages(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotGetmsgsCmd: pending messages, independent of autodisplay
    if crate::console_msg::pending() {
        return crate::commands::extra::do_messages(ctx);
    }
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
/// The mode and the compact flag are persisted per session (session.rs
/// applies [`parse_dot_api`] before dispatch, so the reply already uses them).
pub fn cmd_dot_api(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    // C++ DotApiCmd: anything else returns false without a message.
    let Some((api_mode, _)) = parse_dot_api(ast) else {
        return CommandResult { success: false, ..Default::default() };
    };
    let api_mode = api_mode as i32;
    // C++ DotApiCmd: ObjectKeyValue("api", "%s: ", api, "%d\n")
    CommandResult::ok(format!("api: {api_mode}\n")).with_json(serde_json::json!({ "api": api_mode }))
}

/// C++ ua_dotcmds.cc DotApiCmd(): the new API mode and, for json with
/// exactly `compact=<value>` as second argument, the compact flag (`yes`
/// switches it on, any other value off; without it the flag stays). The
/// keywords are compared case-insensitively; no argument means 1; more than
/// two arguments or an unknown keyword fail.
pub fn parse_dot_api(ast: &CommandAst) -> Option<(ApiMode, Option<bool>)> {
    let args = &ast.arguments;
    let Some(first) = args.first() else { return Some((ApiMode::Api, None)) };
    if args.len() > 2 {
        return None;
    }
    let mode = match first.key.to_ascii_lowercase().as_str() {
        "off" | "0" => ApiMode::Human,
        "on" | "1" => ApiMode::Api,
        "json" | "2" => ApiMode::Json,
        _ => return None,
    };
    // FindArgWithValue(ua, "compact") == 2
    let compact = args.get(1)
        .filter(|arg| mode == ApiMode::Json && arg.key.eq_ignore_ascii_case("compact"))
        .and_then(|arg| arg.value.as_deref())
        .map(|value| value.eq_ignore_ascii_case("yes"));
    Some((mode, compact))
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
    crate::commands::sql::cmd_sql_tables(ast, ctx)
}

/// `.sqlschema [table=<name>] [catalog=<name>]`
pub fn cmd_dot_sqlschema(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::commands::sql::cmd_sql_schema(ast, ctx)
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

    /// Compatibility log C-025: the resource listings filter by the
    /// console's ACL (ua_dotcmds.cc AclAccessOk, no audit event).
    #[test]
    fn dot_listings_filter_by_console_acl() {
        use std::sync::{Arc, RwLock};
        let config = crate::director_config::DirectorConfig::from_str(r#"
Director { Name=dir; Password=secret }
Profile { Name=p; ScheduleACL="Week.*"; CatalogACL=MyCatalog }
Console { Name=limited; Password=pw; JobACL=!blocked,*all*; PoolACL="Inc.*",Full; ClientACL=fd1; Profile=p }
Job { Name=blocked; Client=fd1 }
Job { Name=allowed; Client=fd1 }
JobDefs { Name=blocked }
JobDefs { Name=defs }
Client { Name=fd1; Password=x }
Client { Name=fd2; Password=x }
Pool { Name=Full }
Pool { Name=Scratch }
Pool { Name=Incremental }
Storage { Name=File; Address=localhost; Password=x; Device=d; MediaType=m }
FileSet { Name=fs }
Schedule { Name=WeeklyCycle }
Schedule { Name=Nightly }
Catalog { Name=MyCatalog; DbName=bareos }
Catalog { Name=Other; DbName=other }
"#).unwrap();
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session("limited");
        manager.authenticate_session(id);
        let ctx = ExecContext { session_id: id, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: Some(manager), catalog: None, api_mode: ApiMode::Human,
            bus_rx: None, job_manager: None };
        let ast = CommandAst::default();
        let text = |f: fn(&CommandAst, &ExecContext) -> CommandResult| f(&ast, &ctx).text_output;
        assert_eq!(text(cmd_dot_jobs), "allowed\n");
        assert_eq!(text(cmd_dot_jobdefs), "defs\n");
        assert_eq!(text(cmd_dot_clients), "fd1\n");
        assert_eq!(text(cmd_dot_pools), "Full\nIncremental\n");
        assert_eq!(text(cmd_dot_storage), "");
        assert_eq!(text(cmd_dot_filesets), "");
        assert_eq!(text(cmd_dot_schedule), "WeeklyCycle\n");
        assert_eq!(text(cmd_dot_catalogs), "MyCatalog\n");
        assert_eq!(cmd_dot_jobs(&ast, &ctx).json.unwrap(),
            json!({"jobs":[{"name":"allowed","enabled":true}]}));
    }

    #[test]
    fn dot_api_arguments_follow_dot_api_cmd() {
        let parse = |line: &str| parse_dot_api(&crate::session::parse_cli_command(line));
        assert_eq!(parse(".api"), Some((ApiMode::Api, None)));
        assert_eq!(parse(".api OFF"), Some((ApiMode::Human, None)));
        assert_eq!(parse(".api on"), Some((ApiMode::Api, None)));
        assert_eq!(parse(".api 2"), Some((ApiMode::Json, None)));
        assert_eq!(parse(".api json compact=yes"), Some((ApiMode::Json, Some(true))));
        assert_eq!(parse(".api JSON Compact=YES"), Some((ApiMode::Json, Some(true))));
        assert_eq!(parse(".api 2 compact=no"), Some((ApiMode::Json, Some(false))));
        assert_eq!(parse(".api 2 compact=maybe"), Some((ApiMode::Json, Some(false))));
        assert_eq!(parse(".api 2 compact"), Some((ApiMode::Json, None)));
        assert_eq!(parse(".api 1 compact=yes"), Some((ApiMode::Api, None)));
        assert_eq!(parse(".api 3"), None);
        assert_eq!(parse(".api 2 compact=yes x"), None);
        let ctx = ExecContext { session_id: 0, config: None, session_mgr: None, catalog: None,
            api_mode: ApiMode::Human, bus_rx: None, job_manager: None };
        let failed = cmd_dot_api(&crate::session::parse_cli_command(".api foo"), &ctx);
        assert!(!failed.success && failed.text_output.is_empty() && failed.error_message.is_empty());
    }
}

#[cfg(test)]
mod jobstatus_tests {
    use super::*;

    fn ctx() -> ExecContext {
        ExecContext { session_id: 0, config: None, session_mgr: None, catalog: None,
            api_mode: ApiMode::Human, bus_rx: None, job_manager: None }
    }

    /// C++ DotJobstatusCmd: only `.jobstatus=X` filters; the value must be
    /// exactly one byte, checked before the catalog is opened.
    #[test]
    fn jobstatus_value_must_be_one_character() {
        for (line, status) in [(".jobstatus=TT", "TT"), (".jobstatus=", ""), (".jobstatus=ä", "ä")] {
            let ast = crate::session::parse_cli_command(line);
            let r = cmd_dot_jobstatus(&ast, &ctx());
            assert!(!r.success, "{line}");
            assert_eq!(r.error_message,
                format!("Unknown JobStatus '{status}'. JobStatus must be a single character.\n"));
        }
        // Other arguments are ignored (no error before the catalog query).
        let ast = crate::session::parse_cli_command(".jobstatus TT status=TT");
        assert!(cmd_dot_jobstatus(&ast, &ctx()).error_message.is_empty());
    }

    #[test]
    fn levels_with_two_arguments_list_nothing() {
        let ast = crate::session::parse_cli_command(".levels Backup Restore");
        assert_eq!(cmd_dot_levels(&ast, &ctx()).text_output, "");
        let ast = crate::session::parse_cli_command(".levels Backup");
        assert!(cmd_dot_levels(&ast, &ctx()).text_output.starts_with("Full\n"));
    }
}
