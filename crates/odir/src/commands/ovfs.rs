//! OVFS console commands — Onesimus extensions without Bareos equivalent.
//!
//! `.ovfs_status`, `.ovfs_latest`, `.ovfs_tree`, `.ovfs_ls`, `.ovfs_versions`
//! give restore browsers (oconsole, MCP, GUI) the directory trees of the
//! catalog through the PostgreSQL extension `ovfs` when it is installed, and
//! through the Director fallback otherwise (see `crate::ovfs`). The values
//! are the same on both paths; `.ovfs_status` tells which one is used.
//!
//! Arguments are `key=value`; job lists are comma separated:
//!
//! ```text
//! .ovfs_status [refresh]
//! .ovfs_latest client=<name> [fileset=<name>]
//! .ovfs_tree   jobid=<list> | client=<name> [fileset=<name>]  [root=<dir>] [depth=<n>] [max_entries=<n>]
//! .ovfs_ls     jobid=<list> | client=<name> [fileset=<name>]  path=<dir>
//! .ovfs_versions client=<name> path=<dir> name=<file> [copies]
//! ```
//!
//! Output: `.api 2` returns the values as JSON (`result`), the other modes a
//! readable text; the keyword `json` prints the JSON object as one line of
//! text in every mode (MCP). Access: Command ACL of the command name (as for
//! every command), Job/Client/FileSet ACL of every job of the chain and of
//! `client=`/`fileset=`; denials are audited like the BVFS checks.
//!
//! BVFS (`.bvfs_*`) is unchanged and does not use OVFS.

use serde_json::{json, Map, Value};

use crate::acl::{self, Kind};
use crate::catalog_worker::CatalogConnection;
use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;
use crate::ovfs::{self, Capability, Catalog, LsRow, Ovfs, VersionRow};

/// Default `max_entries` of `.ovfs_tree`: lower than the extension's default
/// (1,000,000) because the console result is held in memory once more as a
/// JSON value; pass `max_entries=` for larger trees.
pub const CONSOLE_MAX_ENTRIES: i64 = 100_000;

/// Default `depth` of `.ovfs_tree`.
pub const CONSOLE_DEPTH: i32 = 1;

// --- arguments -------------------------------------------------------------------

fn value<'a>(ast: &'a CommandAst, keys: &[&str]) -> Option<&'a str> {
    ast.arguments.iter().rev()
        .find(|a| keys.iter().any(|k| a.key.eq_ignore_ascii_case(k)))
        .and_then(|a| a.value.as_deref())
}

fn flag(ast: &CommandAst, key: &str) -> bool {
    ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_none())
}

/// `1,2,3` → jobids (each a positive integer).
pub fn parse_jobids(text: &str) -> Result<Vec<i32>, String> {
    let ids: Result<Vec<i32>, _> = text.split(',').map(str::trim).filter(|s| !s.is_empty())
        .map(|s| s.parse::<i32>().ok().filter(|id| *id > 0).ok_or(()))
        .collect();
    match ids {
        Ok(ids) if !ids.is_empty() => Ok(ids),
        _ => Err(format!("Invalid jobid list \"{text}\": expected positive integers separated by commas.\n")),
    }
}

fn parse_int<T: std::str::FromStr>(ast: &CommandAst, key: &str, default: T) -> Result<T, String> {
    match value(ast, &[key]) {
        None => Ok(default),
        Some(text) => text.trim().parse().map_err(|_| format!("Invalid {key}=\"{text}\": expected an integer.\n")),
    }
}

/// Where the job chain comes from.
enum Selection {
    Jobs(Vec<i32>),
    Latest { client: String, fileset: Option<String> },
}

fn selection(ast: &CommandAst) -> Result<Selection, String> {
    if let Some(list) = value(ast, &["jobid", "jobids"]) {
        return parse_jobids(list).map(Selection::Jobs);
    }
    if let Some(client) = value(ast, &["client"]) {
        return Ok(Selection::Latest { client: client.to_string(), fileset: value(ast, &["fileset"]).map(str::to_string) });
    }
    Err("Need jobid=<list> or client=<name> [fileset=<name>].\n".into())
}

// --- output -------------------------------------------------------------------------

/// `.api 2`: the object as JSON result; `json`: the object as one text line;
/// otherwise the readable text.
fn output(ast: &CommandAst, ctx: &ExecContext, prefix: String, object: Value, text: String, warning: Option<String>) -> CommandResult {
    let body = if ctx.api_mode != ApiMode::Json && flag(ast, "json") {
        format!("{}\n", serde_json::to_string(&object).unwrap_or_default())
    } else {
        text
    };
    let mut result = CommandResult::ok(prefix + &body).with_json(object);
    if let Some(warning) = warning {
        result = result.with_warning(format!("Warning: {warning}\n"));
    }
    result
}

fn failure(prefix: String, message: impl Into<String>) -> CommandResult {
    let mut message = message.into();
    if !message.ends_with('\n') {
        message.push('\n');
    }
    let mut result = CommandResult::error(message);
    result.text_output = prefix;
    result
}

fn mtime_text(mtime: Option<i64>) -> String {
    mtime.map(|t| crate::localtime::strftime("%Y-%m-%d %H:%M:%S", t as libc::time_t)).unwrap_or_default()
}

// --- catalog --------------------------------------------------------------------------

/// The capability to use: the recorded one, checked now if unknown.
fn capability(ctx: &ExecContext) -> Capability {
    match &ctx.catalog {
        Some(catalog) => match catalog.ovfs_capability() {
            Capability::Unknown => catalog.ovfs_recheck(),
            known => known,
        },
        None => Capability::Unknown,
    }
}

/// Catalog selection (OpenClientDb) and a reserved connection.
fn open(ast: &CommandAst, ctx: &ExecContext) -> Result<(String, CatalogConnection), CommandResult> {
    let prefix = crate::ua_db::open_client_db(ast, ctx, true)?;
    match ctx.catalog.as_ref().map(|c| c.dedicated()) {
        Some(Ok(db)) => Ok((prefix, db)),
        Some(Err(e)) => Err(failure(prefix, format!("Catalog not available: {e}"))),
        None => Err(failure(prefix, "No catalog configured.")),
    }
}

/// Run `call` on the path the capability selects. If the extension call
/// fails and the extension is no longer usable (dropped, privileges
/// revoked), the fallback answers instead.
fn run<T>(ctx: &ExecContext, db: &dyn Catalog, call: impl Fn(&Ovfs) -> ovfs::Result<T>) -> (ovfs::Result<T>, &'static str) {
    let ovfs = Ovfs::new(db, &capability(ctx));
    let result = call(&ovfs);
    if result.is_err() && ovfs.path() == "ovfs" {
        if let Some(catalog) = &ctx.catalog {
            if !catalog.ovfs_recheck().usable() {
                let fallback = Ovfs::fallback(db);
                return (call(&fallback), fallback.path());
            }
        }
    }
    (result, ovfs.path())
}

// --- ACL ----------------------------------------------------------------------------------

const ERR_UNAUTHORIZED: &str = "Unauthorized command from this console.\n";

/// Job, Client and FileSet ACL of every job of the list (audited, like
/// BvfsValidateJobid). Unknown jobs are left to the OVFS call, which reports
/// them as "job N does not exist".
fn jobs_permitted(ctx: &ExecContext, db: &dyn Catalog, jobids: &[i32]) -> Result<bool, String> {
    let sql = format!(
        "SELECT j.jobid, j.name, c.name, fs.fileset FROM job j \
         LEFT JOIN client c ON c.clientid = j.clientid \
         LEFT JOIN fileset fs ON fs.filesetid = j.filesetid \
         WHERE j.jobid = ANY({})",
        ovfs::int_array(jobids)
    );
    let table = db.rows(&sql).map_err(|e| ovfs::server_message(&e))?;
    for row in &table.rows {
        let get = |i: usize| ovfs::cell(row, i);
        if !acl::access_audited(ctx, Kind::Job, get(1).unwrap_or_default()) {
            return Ok(false);
        }
        if let Some(client) = get(2) {
            if !acl::access_audited(ctx, Kind::Client, client) {
                return Ok(false);
            }
        }
        if let Some(fileset) = get(3) {
            if !acl::access_audited(ctx, Kind::FileSet, fileset) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Client (and FileSet) ACL of `client=`/`fileset=`, audited.
fn client_permitted(ctx: &ExecContext, client: &str, fileset: Option<&str>) -> bool {
    acl::access_audited(ctx, Kind::Client, client) && fileset.is_none_or(|f| acl::access_audited(ctx, Kind::FileSet, f))
}

/// The job chain of the selection, ACL-checked; `Err` is the finished result.
fn chain(ctx: &ExecContext, db: &dyn Catalog, prefix: &str, selection: Selection) -> Result<(Vec<i32>, Option<String>), CommandResult> {
    let (jobids, warning) = match selection {
        Selection::Jobs(jobids) => (jobids, None),
        Selection::Latest { client, fileset } => {
            let (latest, _) = run(ctx, db, |o| o.jobids_latest(&client, fileset.as_deref()));
            let latest = latest.map_err(|e| failure(prefix.to_string(), e.0))?;
            (latest.jobids, latest.warning)
        }
    };
    match jobs_permitted(ctx, db, &jobids) {
        Ok(true) => Ok((jobids, warning)),
        Ok(false) => Err(failure(prefix.to_string(), ERR_UNAUTHORIZED)),
        Err(e) => Err(failure(prefix.to_string(), e)),
    }
}

/// Client/FileSet ACL before the catalog is opened (`client=` also selects
/// the client's catalog in OpenClientDb).
fn precheck(ctx: &ExecContext, ast: &CommandAst) -> Result<(), CommandResult> {
    if let Some(client) = value(ast, &["client"]) {
        if !client_permitted(ctx, client, value(ast, &["fileset"])) {
            return Err(failure(String::new(), ERR_UNAUTHORIZED));
        }
    }
    Ok(())
}

// --- commands ------------------------------------------------------------------------------

/// `.ovfs_status [refresh]`
pub fn cmd_ovfs_status(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some(catalog) = &ctx.catalog else {
        return failure(String::new(), "No catalog configured.");
    };
    let capability = if flag(ast, "refresh") || catalog.ovfs_capability() == Capability::Unknown {
        catalog.ovfs_recheck()
    } else {
        catalog.ovfs_capability()
    };
    let mut status = Map::new();
    status.insert("state".into(), json!(capability.state()));
    status.insert("installed".into(), json!(capability.installed()));
    status.insert("usable".into(), json!(capability.usable()));
    status.insert("version".into(), json!(capability.version()));
    match &capability {
        Capability::Available { schema, .. } => {
            status.insert("schema".into(), json!(schema));
        }
        Capability::Unusable { reason, .. } => {
            status.insert("reason".into(), json!(reason));
        }
        Capability::CheckFailed { error } => {
            status.insert("reason".into(), json!(error));
        }
        _ => {}
    }
    status.insert("path".into(), json!(capability.path()));
    let path_text = if capability.usable() { "ovfs extension" } else { "Director fallback (plain SQL)" };
    let text = format!("OVFS extension: {capability}\nRestore trees: {path_text}\n");
    output(ast, ctx, String::new(), json!({ "ovfs": Value::Object(status) }), text, None)
}

/// `.ovfs_latest client=<name> [fileset=<name>]`
pub fn cmd_ovfs_latest(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some(client) = value(ast, &["client"]) else {
        return failure(String::new(), "Need client=<name> [fileset=<name>].");
    };
    if let Err(r) = precheck(ctx, ast) {
        return r;
    }
    let (prefix, db) = match open(ast, ctx) {
        Ok(opened) => opened,
        Err(r) => return r,
    };
    let fileset = value(ast, &["fileset"]).map(str::to_string);
    let (jobids, warning) = match chain(ctx, &db, &prefix, Selection::Latest { client: client.to_string(), fileset }) {
        Ok(chain) => chain,
        Err(r) => return r,
    };
    let path = capability(ctx).path();
    let list: Vec<String> = jobids.iter().map(i32::to_string).collect();
    let object = json!({ "path": path, "client": client, "jobids": jobids });
    output(ast, ctx, prefix, object, format!("{}\n", list.join(",")), warning)
}

/// `.ovfs_tree jobid=<list> | client=<name> [fileset=] [root=/] [depth=1] [max_entries=]`
pub fn cmd_ovfs_tree(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let selection = match selection(ast) {
        Ok(s) => s,
        Err(e) => return failure(String::new(), e),
    };
    let root = value(ast, &["root", "path"]).unwrap_or("/").to_string();
    let depth = match parse_int(ast, "depth", CONSOLE_DEPTH) {
        Ok(d) => d,
        Err(e) => return failure(String::new(), e),
    };
    let max_entries = match parse_int(ast, "max_entries", CONSOLE_MAX_ENTRIES) {
        Ok(m) => m,
        Err(e) => return failure(String::new(), e),
    };
    if let Err(r) = precheck(ctx, ast) {
        return r;
    }
    let (prefix, db) = match open(ast, ctx) {
        Ok(opened) => opened,
        Err(r) => return r,
    };
    let (jobids, warning) = match chain(ctx, &db, &prefix, selection) {
        Ok(chain) => chain,
        Err(r) => return r,
    };
    let (result, path) = run(ctx, &db, |o| o.tree(&jobids, &root, depth, max_entries));
    let tree_text = match result {
        Ok(tree) => tree,
        Err(e) => return failure(prefix, e.0),
    };
    // `json` keyword: the tree text is passed on as it is, without building
    // a JSON value of it (a value tree costs several times the text).
    if ctx.api_mode != ApiMode::Json && flag(ast, "json") {
        let text = format!(
            "{{\"path\":{},\"jobids\":{},\"root\":{},\"depth\":{depth},\"tree\":{}}}\n",
            json!(path), json!(jobids), json!(root), tree_text.as_deref().unwrap_or("null")
        );
        let mut result = CommandResult::ok(prefix + &text);
        if let Some(warning) = warning {
            result = result.with_warning(format!("Warning: {warning}\n"));
        }
        return result;
    }
    let tree: Value = match &tree_text {
        Some(text) => match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => return failure(prefix, format!("invalid tree JSON: {e}")),
        },
        None => Value::Null,
    };
    let mut text = format!("jobids: {}\n", join(&jobids));
    if tree.is_null() {
        text.push_str(&format!("{root}: nothing to restore\n"));
    } else {
        render_tree(&tree, 0, &mut text);
    }
    let object = json!({ "path": path, "jobids": jobids, "root": root, "depth": depth, "tree": tree });
    output(ast, ctx, prefix, object, text, warning)
}

/// `.ovfs_ls jobid=<list> | client=<name> [fileset=] path=<dir>`
pub fn cmd_ovfs_ls(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let selection = match selection(ast) {
        Ok(s) => s,
        Err(e) => return failure(String::new(), e),
    };
    let Some(dir) = value(ast, &["path", "dir"]).map(str::to_string) else {
        return failure(String::new(), "Need path=<directory>.");
    };
    if let Err(r) = precheck(ctx, ast) {
        return r;
    }
    let (prefix, db) = match open(ast, ctx) {
        Ok(opened) => opened,
        Err(r) => return r,
    };
    let (jobids, warning) = match chain(ctx, &db, &prefix, selection) {
        Ok(chain) => chain,
        Err(r) => return r,
    };
    let (result, path) = run(ctx, &db, |o| o.ls(&jobids, &dir));
    let rows = match result {
        Ok(rows) => rows,
        Err(e) => return failure(prefix, e.0),
    };
    let mut text = format!("jobids: {}\n", join(&jobids));
    for row in &rows {
        text.push_str(&ls_line(row));
    }
    let entries: Vec<Value> = rows.iter().map(ls_json).collect();
    let object = json!({ "path": path, "jobids": jobids, "dir": dir, "entries": entries });
    output(ast, ctx, prefix, object, text, warning)
}

/// `.ovfs_versions client=<name> path=<dir> name=<file> [copies]`
pub fn cmd_ovfs_versions(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let (Some(client), Some(dir)) = (value(ast, &["client"]), value(ast, &["path", "dir"])) else {
        return failure(String::new(), "Need client=<name> path=<directory> name=<file>.");
    };
    // name='' (or a missing name) lists the versions of the directory itself
    let name = value(ast, &["name", "fname", "filename"]).unwrap_or("");
    let copies = flag(ast, "copies");
    if let Err(r) = precheck(ctx, ast) {
        return r;
    }
    let (prefix, db) = match open(ast, ctx) {
        Ok(opened) => opened,
        Err(r) => return r,
    };
    let (result, path) = run(ctx, &db, |o| o.versions(client, dir, name, copies));
    let rows = match result {
        Ok(rows) => rows,
        Err(e) => return failure(prefix, e.0),
    };
    // Versions of jobs the console may not see are left out (not audited).
    let rows = match visible_versions(ctx, &db, rows) {
        Ok(rows) => rows,
        Err(e) => return failure(prefix, e),
    };
    let mut text = String::new();
    for row in &rows {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            row.jobid, row.level, row.job_type, row.starttime.as_deref().unwrap_or(""),
            row.size.map(|s| s.to_string()).unwrap_or_default(), mtime_text(row.mtime),
            row.fileindex, row.volumes.join(","), if row.inchanger { 1 } else { 0 }, row.md5
        ));
    }
    let versions: Vec<Value> = rows.iter().map(version_json).collect();
    let object = json!({ "path": path, "client": client, "dir": dir, "name": name, "versions": versions });
    output(ast, ctx, prefix, object, text, None)
}

fn visible_versions(ctx: &ExecContext, db: &dyn Catalog, rows: Vec<VersionRow>) -> Result<Vec<VersionRow>, String> {
    if rows.is_empty() || !acl::has_restrictions(ctx, Kind::Job) {
        return Ok(rows);
    }
    let jobids: Vec<i32> = rows.iter().map(|r| r.jobid).collect();
    let sql = format!("SELECT jobid, name FROM job WHERE jobid = ANY({})", ovfs::int_array(&jobids));
    let table = db.rows(&sql).map_err(|e| ovfs::server_message(&e))?;
    let allowed: std::collections::HashSet<i32> = table.rows.iter()
        .filter(|r| acl::access(ctx, Kind::Job, ovfs::cell(r, 1).unwrap_or_default()))
        .filter_map(|r| ovfs::cell_i32(r, 0))
        .collect();
    Ok(rows.into_iter().filter(|r| allowed.contains(&r.jobid)).collect())
}

fn join(jobids: &[i32]) -> String {
    jobids.iter().map(i32::to_string).collect::<Vec<_>>().join(",")
}

// --- readable text ---------------------------------------------------------------------

/// One line per node, two spaces per level; directories end with `/`.
fn render_tree(node: &Value, level: usize, out: &mut String) {
    let name = node["name"].as_str().unwrap_or("");
    let is_dir = node["type"] == "directory";
    let label = if is_dir && level > 0 { format!("{name}/") } else { name.to_string() };
    let mut line = format!("{}{label}", "  ".repeat(level));
    let mut attrs = Vec::new();
    if !is_dir {
        attrs.push(node["type"].as_str().unwrap_or("").to_string());
        if let Some(size) = node["size"].as_i64() {
            attrs.push(format!("size={size}"));
        }
    }
    if let Some(mtime) = node["mtime"].as_i64() {
        attrs.push(format!("mtime={}", mtime_text(Some(mtime))));
    }
    if let (Some(jobid), Some(fileindex)) = (node["jobid"].as_i64(), node["fileindex"].as_i64()) {
        attrs.push(format!("jobid={jobid} fileindex={fileindex}"));
    }
    if let Some(seq) = node["delta_seq"].as_i64() {
        attrs.push(format!("delta_seq={seq}"));
    }
    if let Some(link) = node["link_fileindex"].as_i64() {
        attrs.push(format!("link_fileindex={link}"));
    }
    if let Some(entries) = node["entries"].as_i64() {
        attrs.push(format!("entries={entries}"));
    }
    if !attrs.is_empty() {
        line.push_str("  ");
        line.push_str(&attrs.join(" "));
    }
    out.push_str(&line);
    out.push('\n');
    if let Some(children) = node["children"].as_array() {
        for child in children {
            render_tree(child, level + 1, out);
        }
    }
}

fn ls_line(row: &LsRow) -> String {
    let name = if row.kind == "directory" { format!("{}/", row.name) } else { row.name.clone() };
    let mut attrs = vec![format!("{:<9}", row.kind)];
    attrs.push(format!("{:>12}", row.size.map(|s| s.to_string()).unwrap_or_default()));
    attrs.push(format!("{:<19}", mtime_text(row.mtime)));
    attrs.push(name);
    if let (Some(jobid), Some(fileindex)) = (row.jobid, row.fileindex) {
        attrs.push(format!("jobid={jobid} fileindex={fileindex}"));
    }
    if let Some(entries) = row.entries {
        attrs.push(format!("entries={entries}"));
    }
    format!("{}\n", attrs.join("  "))
}

/// The columns of `ovfs.ls` as an object; NULL columns are left out (as
/// the keys of the JSON tree).
pub fn ls_json(row: &LsRow) -> Value {
    let mut o = Map::new();
    o.insert("name".into(), json!(row.name));
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            o.insert(key.into(), value);
        }
    };
    put("name_b64", row.name_b64.as_ref().map(|v| json!(v)));
    put("path", row.path.as_ref().map(|v| json!(v)));
    put("path_b64", row.path_b64.as_ref().map(|v| json!(v)));
    put("kind", Some(json!(row.kind)));
    put("jobid", row.jobid.map(|v| json!(v)));
    put("fileid", row.fileid.map(|v| json!(v)));
    put("fileindex", row.fileindex.map(|v| json!(v)));
    put("delta_seq", row.delta_seq.map(|v| json!(v)));
    put("delta_parts", row.delta_parts.as_ref().map(|parts| {
        json!(parts.iter().map(|p| json!({ "jobid": p.jobid, "fileindex": p.fileindex })).collect::<Vec<_>>())
    }));
    put("mode", row.mode.map(|v| json!(v)));
    put("uid", row.uid.map(|v| json!(v)));
    put("gid", row.gid.map(|v| json!(v)));
    put("size", row.size.map(|v| json!(v)));
    put("mtime", row.mtime.map(|v| json!(v)));
    put("link_fileindex", row.link_fileindex.map(|v| json!(v)));
    put("entries", row.entries.map(|v| json!(v)));
    Value::Object(o)
}

pub fn version_json(row: &VersionRow) -> Value {
    json!({
        "jobid": row.jobid, "level": row.level, "job_type": row.job_type, "jobtdate": row.jobtdate,
        "starttime": row.starttime, "fileid": row.fileid, "fileindex": row.fileindex,
        "delta_seq": row.delta_seq, "md5": row.md5, "mode": row.mode, "uid": row.uid, "gid": row.gid,
        "size": row.size, "mtime": row.mtime, "volumes": row.volumes, "inchanger": row.inchanger,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_ast::Argument;

    fn ast(name: &str, args: &[(&str, Option<&str>)]) -> CommandAst {
        CommandAst {
            command_name: name.into(),
            arguments: args.iter().map(|(k, v)| Argument { key: k.to_string(), value: v.map(str::to_string) }).collect(),
            ..Default::default()
        }
    }

    fn ctx(api_mode: ApiMode) -> ExecContext {
        ExecContext { session_id: 0, config: None, session_mgr: None, catalog: None, api_mode, bus_rx: None, job_manager: None }
    }

    /// `.ovfs_*` never shadows a Bareos command and is never shadowed by one:
    /// Bareos prefix resolution runs first, and no Bareos command name starts
    /// with `.ovfs` or is a prefix match of these names; the extensions
    /// resolve by exact name only.
    #[test]
    fn names_do_not_collide_with_bareos_prefixes() {
        let registry = crate::command_registry::build_registry();
        let names = [".ovfs_status", ".ovfs_latest", ".ovfs_tree", ".ovfs_ls", ".ovfs_versions"];
        for (bareos, _, _) in crate::ua_commands_table::UA_COMMANDS.iter() {
            assert!(!bareos.starts_with(".o"), "{bareos}");
        }
        for name in names {
            assert_eq!(crate::command_registry::resolve_command_name(&registry, name), Some(name));
        }
        // prefixes of the extensions resolve to nothing (no prefix matching)
        for prefix in [".o", ".ov", ".ovfs", ".ovfs_t", ".ovfs_l"] {
            assert_eq!(crate::command_registry::resolve_command_name(&registry, prefix), None, "{prefix}");
        }
        // BVFS keeps its own handlers
        for bvfs in [".bvfs_lsdirs", ".bvfs_lsfiles", ".bvfs_versions", ".bvfs_get_jobids"] {
            assert_eq!(crate::command_registry::resolve_command_name(&registry, bvfs), Some(bvfs));
        }
    }

    #[test]
    fn jobid_lists() {
        assert_eq!(parse_jobids("1,2, 3").unwrap(), [1, 2, 3]);
        assert_eq!(parse_jobids("7,").unwrap(), [7]);
        assert!(parse_jobids("").is_err());
        assert!(parse_jobids("1,x").is_err());
        assert!(parse_jobids("0").is_err());
        assert!(parse_jobids("-3").is_err());
    }

    #[test]
    fn selection_needs_jobs_or_client() {
        assert!(matches!(selection(&ast(".ovfs_tree", &[("jobids", Some("4,5"))])), Ok(Selection::Jobs(j)) if j == [4, 5]));
        assert!(matches!(selection(&ast(".ovfs_tree", &[("client", Some("fd")), ("fileset", Some("Linux"))])),
            Ok(Selection::Latest { client, fileset }) if client == "fd" && fileset.as_deref() == Some("Linux")));
        assert!(selection(&ast(".ovfs_tree", &[])).is_err());
    }

    #[test]
    fn without_catalog() {
        let r = cmd_ovfs_status(&ast(".ovfs_status", &[]), &ctx(ApiMode::Human));
        assert!(!r.success && r.error_message == "No catalog configured.\n");
        let r = cmd_ovfs_tree(&ast(".ovfs_tree", &[("depth", Some("x")), ("jobid", Some("1"))]), &ctx(ApiMode::Human));
        assert_eq!(r.error_message, "Invalid depth=\"x\": expected an integer.\n");
        let r = cmd_ovfs_ls(&ast(".ovfs_ls", &[("jobid", Some("1"))]), &ctx(ApiMode::Human));
        assert_eq!(r.error_message, "Need path=<directory>.\n");
    }

    #[test]
    fn status_reports_capability_and_path() {
        let catalog = crate::catalog_worker::CatalogHandle::mock(|request| match request {
            crate::catalog_worker::CatalogRequest::TextRows { .. } => {
                crate::catalog_worker::CatalogResponse::Rows(crate::catalog_worker::SqlTable::default())
            }
            _ => crate::catalog_worker::CatalogResponse::Error("unexpected".into()),
        });
        let mut c = ctx(ApiMode::Human);
        c.catalog = Some(catalog);
        let r = cmd_ovfs_status(&ast(".ovfs_status", &[]), &c);
        assert_eq!(r.text_output, "OVFS extension: not installed\nRestore trees: Director fallback (plain SQL)\n");
        assert_eq!(r.json.as_ref().unwrap()["ovfs"]["path"], "fallback");
        let r = cmd_ovfs_status(&ast(".ovfs_status", &[("json", None)]), &c);
        assert_eq!(r.text_output, "{\"ovfs\":{\"state\":\"not_installed\",\"installed\":false,\"usable\":false,\"version\":null,\"path\":\"fallback\"}}\n");
    }

    #[test]
    fn tree_text_rendering() {
        let tree: Value = serde_json::from_str(r#"{"name":"/","path":"/","type":"directory","children":[
            {"name":"vmlinuz","type":"file","jobid":10,"fileid":2,"fileindex":2,"mode":33188,"uid":0,"gid":0,"size":20,"mtime":0},
            {"name":"etc","path":"/etc/","type":"directory","jobid":10,"fileid":3,"fileindex":3,"entries":3}]}"#).unwrap();
        let mut out = String::new();
        render_tree(&tree, 0, &mut out);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "/");
        assert!(lines[1].starts_with("  vmlinuz  file size=20 mtime=") && lines[1].ends_with("jobid=10 fileindex=2"), "{}", lines[1]);
        assert_eq!(lines[2], "  etc/  jobid=10 fileindex=3 entries=3");
    }

    #[test]
    fn ls_json_omits_null_columns() {
        let row = LsRow { name: "etc".into(), path: Some("/etc/".into()), kind: "directory".into(), entries: Some(3), ..Default::default() };
        assert_eq!(serde_json::to_string(&ls_json(&row)).unwrap(), r#"{"name":"etc","path":"/etc/","kind":"directory","entries":3}"#);
    }
}
