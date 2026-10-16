//! `configure` — port of dird/ua_configure.cc (Bareos 25.1.2).
//!
//! Original subcommands, byte-compatible with the C++ Director:
//! - `configure add <resourcetype> <key>=<value> ...` builds the resource
//!   text from the directive tables (`show_items.rs`), parses it with the
//!   checks of the C++ config parser, appends it to the running configuration
//!   and writes `<configdir>/bareos-dir.d/<type>/<name>.conf`;
//! - `configure export client=<name>` writes the client-side Director
//!   resource to `<configdir>/bareos-dir-export/client/<name>/bareos-fd.d/
//!   director/<director>.conf`.
//!
//! Onesimus extensions (not in Bareos; only reached with subcommands the
//! original rejects as `invalid subcommand`): `configure read|show|get`,
//! `configure update` and `configure delete`.

use std::path::{Path, PathBuf};
use bareos_config::resources::*;
use serde_json::{json, Value};

use super::show_items::{self as tables, Item, Ty};
use crate::command_ast::{Argument, CommandAst};
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::director_config::DirectorConfig;

/// Write a resource config file to disk.
/// Path: <config_dir>/bareos-dir.d/<resource_type>/<name>.conf
fn persist_resource(config_dir: &str, resource_type: &str, name: &str, content: &str) -> Result<(), String> {
    let dir = format!("{config_dir}/bareos-dir.d/{resource_type}");
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {dir}: {e}"))?;
    let path = format!("{dir}/{name}.conf");
    std::fs::write(&path, content).map_err(|e| format!("write {path}: {e}"))?;
    tracing::info!("Persisted {resource_type}/{name}.conf");
    Ok(())
}

/// Delete a resource config file from disk.
fn delete_resource_file(config_dir: &str, resource_type: &str, name: &str) -> Result<(), String> {
    let path = format!("{config_dir}/bareos-dir.d/{resource_type}/{name}.conf");
    if Path::new(&path).exists() {
        std::fs::remove_file(&path).map_err(|e| format!("rm {path}: {e}"))?;
        tracing::info!("Deleted {resource_type}/{name}.conf");
    }
    Ok(())
}

/// Configuration layout derived from the `-c` path, like the C++
/// ConfigurationParser (config_dir_, config_include_dir_).
struct Layout {
    /// `<configdir>` (base of `bareos-dir.d` and `bareos-dir-export`)
    config_dir: PathBuf,
    /// `<configdir>/bareos-dir.d`, the directory the resources are read from
    include_dir: PathBuf,
    /// IsUsingConfigIncludeDir()
    using_include_dir: bool,
}

fn layout(path: &Path) -> Layout {
    if path.is_dir() {
        let dir_d = path.join("bareos-dir.d");
        if dir_d.is_dir() {
            return Layout { config_dir: path.to_path_buf(), include_dir: dir_d, using_include_dir: true };
        }
        let parent = path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.to_path_buf());
        return Layout { config_dir: parent, include_dir: path.to_path_buf(), using_include_dir: true };
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    Layout { include_dir: parent.join("bareos-dir.d"), config_dir: parent, using_include_dir: false }
}

/// Get the config directory path (`<configdir>`, see [`Layout`]).
fn config_dir(_ctx: &ExecContext) -> Option<String> {
    crate::reload::config_path().map(|p| layout(&p).config_dir.to_string_lossy().into_owned())
}

const USAGE: &str = "usage:\n  configure add <resourcetype> <key1>=<value1> ...\n  configure export client=<clientname>\n";
const INCLUDE_DIR_WARNING: &str = "It seems that the configuration is not adapted to the include directory structure. This means, that the configure command may not work as expected. Your configuration changes may not survive a reload/restart. ";

pub fn cmd_configure(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let layout = crate::reload::config_path().map(|p| layout(&p));
    let warning = match &layout {
        Some(l) if !l.using_include_dir => INCLUDE_DIR_WARNING,
        _ => "",
    };
    let with_warning = |mut result: CommandResult| {
        if !warning.is_empty() {
            // ua->WarningMsg() before anything else: first in the output,
            // first message (BNET_WARNING_MSG, `.api 2` "warning" list).
            if result.text_output.is_empty() || !result.errors_first {
                result.text_output.insert_str(0, warning);
                result.errors_first = false;
            } else {
                result.error_message.insert_str(0, warning);
            }
            result.messages.insert(0, (crate::command_queue::MsgKind::Warning, warning.to_string()));
        }
        result
    };

    // ua->argc counts the command itself.
    if ast.arguments.len() + 1 < 3 {
        return with_warning(CommandResult::error(USAGE.to_string()));
    }
    let action = ast.arguments[0].key.as_str();
    let result = if action.eq_ignore_ascii_case("add") {
        configure_add(ast, ctx, layout.as_ref())
    } else if action.eq_ignore_ascii_case("export") {
        configure_export(ast, ctx, layout.as_ref())
    } else {
        match action {
            // Onesimus extensions
            "read" | "show" | "get" => configure_read(ast, ctx),
            "update" => configure_update(ast, ctx),
            "delete" => configure_delete(ast, ctx),
            _ => CommandResult::error(format!("invalid subcommand {action}.\n")),
        }
    };
    with_warning(result)
}

// ---------------------------------------------------------------------------
// Resource tables (dird_conf.cc dird_resource_tables)
// ---------------------------------------------------------------------------

struct Table {
    name: &'static str,
    /// rcode name used by ResourceItem::code
    rcode: &'static str,
    items: Option<&'static [Item]>,
}

const TABLES: &[Table] = &[
    Table { name: "Director", rcode: "R_DIRECTOR", items: Some(tables::DIR_ITEMS) },
    Table { name: "Client", rcode: "R_CLIENT", items: Some(tables::CLIENT_ITEMS) },
    Table { name: "JobDefs", rcode: "R_JOBDEFS", items: Some(tables::JOB_ITEMS) },
    Table { name: "Job", rcode: "R_JOB", items: Some(tables::JOB_ITEMS) },
    Table { name: "Storage", rcode: "R_STORAGE", items: Some(tables::STORAGE_ITEMS) },
    Table { name: "Catalog", rcode: "R_CATALOG", items: Some(tables::CATALOG_ITEMS) },
    Table { name: "Schedule", rcode: "R_SCHEDULE", items: Some(tables::SCHEDULE_ITEMS) },
    Table { name: "FileSet", rcode: "R_FILESET", items: Some(tables::FILESET_ITEMS) },
    Table { name: "Pool", rcode: "R_POOL", items: Some(tables::POOL_ITEMS) },
    Table { name: "Messages", rcode: "R_MSGS", items: Some(tables::MESSAGES_ITEMS) },
    Table { name: "Counter", rcode: "R_COUNTER", items: Some(tables::COUNTER_ITEMS) },
    Table { name: "Profile", rcode: "R_PROFILE", items: Some(tables::PROFILE_ITEMS) },
    Table { name: "Console", rcode: "R_CONSOLE", items: Some(tables::CONSOLE_ITEMS) },
    Table { name: "Device", rcode: "R_DEVICE", items: None },
    Table { name: "User", rcode: "R_USER", items: Some(tables::USER_ITEMS) },
];

/// ConfigurationParser::GetResourceTable (case-insensitive).
fn table_by_name(name: &str) -> Option<&'static Table> {
    TABLES.iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

fn table_by_rcode(rcode: &str) -> Option<&'static Table> {
    TABLES.iter().find(|t| t.rcode == rcode)
}

/// ConfigurationParser::GetResourceItem: name or alias, case-insensitive.
fn find_item(items: &'static [Item], key: &str) -> Option<&'static Item> {
    items.iter().find(|i| i.name.eq_ignore_ascii_case(key) || i.aliases.iter().any(|a| a.eq_ignore_ascii_case(key)))
}

/// LoadedConfiguration::GetResWithName (exact name) on the running config.
fn resource_exists(cfg: &DirectorConfig, table: &Table, name: &str) -> bool {
    cfg.raw.iter().any(|r| r.norm_type() == table.name.to_ascii_lowercase() && r.name() == Some(name))
}

// ---------------------------------------------------------------------------
// configure add
// ---------------------------------------------------------------------------

/// One directive line of the generated resource text.
struct Line {
    item: Option<&'static Item>,
    value: String,
}

/// config_add_directive: format one directive, quoting string-like types.
fn directive_text(item: &Item, value: &str) -> String {
    let quotable = matches!(item.ty, Ty::Autopassword | Ty::Dir | Ty::DirOrCmd | Ty::Label | Ty::Name
        | Ty::Res | Ty::Shrtrunscript | Ty::Stdstr | Ty::Stdstrdir | Ty::Str | Ty::Strname);
    if !quotable {
        return format!("  {} = {}\n", item.name, value);
    }
    let value = if matches!(item.ty, Ty::DirOrCmd | Ty::Shrtrunscript) { escape_string(value) } else { value.to_string() };
    format!("  {} = \"{}\"\n", item.name, value)
}

/// lib/util.cc EscapeString
fn escape_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() * 2);
    for c in value.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            '(' | ')' | '<' | '>' | '"' => { out.push('\\'); out.push(c); }
            _ => out.push(c),
        }
    }
    out
}

/// config_get_res_item: directive known and value without '@' / ';'.
fn res_item(table: &Table, items: &'static [Item], key: &str, value: &str) -> Result<&'static Item, String> {
    let item = find_item(items, key)
        .ok_or_else(|| format!("Resource \"{}\" does not permit directive \"{key}\".\n", table.name))?;
    let forbidden = if value.contains(';') { Some("';' (end of directive)") }
        else if value.contains('@') { Some("'@' (include)") } else { None };
    match forbidden {
        Some(what) => Err(format!("Could not add directive \"{key}\": character {what} is forbidden.\n")),
        None => Ok(item),
    }
}

/// configure_create_resource_string. `args[0]` is the resource type argument.
fn create_resource_string(table: &Table, items: &'static [Item], args: &[Argument])
    -> Result<(String, String, Vec<Line>), String>
{
    let mut name = String::new();
    let mut lines = Vec::new();
    if let Some(value) = &args[0].value {
        name.push_str(value);
        lines.push(Line { item: Some(res_item(table, items, "name", value)?), value: name.clone() });
    }
    for arg in &args[1..] {
        let Some(value) = &arg.value else {
            return Err(format!("Missing value for directive \"{}\"\n", arg.key));
        };
        if arg.key.eq_ignore_ascii_case("name") {
            name.push_str(value);
        }
        lines.push(Line { item: Some(res_item(table, items, &arg.key, value)?), value: value.clone() });
    }
    if name.is_empty() {
        return Err(format!("Resource \"{}\": missing name parameter.\n", table.name));
    }
    let mut text = format!("{} {{\n", table.name);
    for line in &lines {
        text.push_str(&directive_text(line.item.unwrap(), &line.value));
    }
    text.push_str("}\n");
    Ok((name, text, lines))
}

/// lib/edit.cc Is_a_number
fn is_a_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'-' || b[i] == b'+') { i += 1; }
    let start = i;
    while i < b.len() && b[i].is_ascii_digit() { i += 1; }
    let digit_seen = i > start;
    if digit_seen && i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() { i += 1; }
    }
    if digit_seen && i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let next = b.get(i + 1).copied();
        let after = b.get(i + 2).copied();
        if next.is_some_and(|c| c.is_ascii_digit())
            || (matches!(next, Some(b'-') | Some(b'+')) && after.is_some_and(|c| c.is_ascii_digit())) {
            i += 2;
            while i < b.len() && b[i].is_ascii_digit() { i += 1; }
        }
    }
    i == b.len() && digit_seen
}

/// lib/edit.cc IsNameValid
fn name_error(name: &str) -> Option<String> {
    if name.starts_with(' ') {
        return Some("Name cannot start with space.\n".into());
    }
    if let Some(c) = name.chars().find(|c| !(c.is_ascii_alphanumeric() || ":.-_/ ".contains(*c))) {
        return Some(format!("Illegal character \"{c}\" in name.\n"));
    }
    if name.len() >= MAX_NAME_LENGTH {
        return Some("Name too long.\n".into());
    }
    if name.is_empty() {
        return Some("Name must be at least one character long.\n".into());
    }
    if name.ends_with(' ') {
        return Some("Name cannot end with space.\n".into());
    }
    None
}

const MAX_NAME_LENGTH: usize = 128;

/// StoreAutopassword/StoreMd5Password/StoreClearpassword checks.
fn password_error(table: &Table, item: &Item, value: &str, resource_name: &str) -> Option<String> {
    let clear = table.rcode == "R_CATALOG" || (table.rcode == "R_DIRECTOR" && item.code == Some("1"));
    if clear {
        return (item.required && value.is_empty())
            .then(|| format!("Empty Password not allowed in Resource \"{resource_name}\" not allowed.\n"));
    }
    match value.strip_prefix("[md5]") {
        Some(hash) => {
            if item.required && hash.starts_with("d41d8cd98f00b204e9800998ecf8427e") {
                return Some(format!("Empty Password not allowed in Resource \"{resource_name}\"\n"));
            }
            if hash.len() != 32 {
                return Some(format!("md5 password does not have the right size; expected: 32, got: {}\n", hash.len()));
            }
            hash.chars().find(|c| !c.is_ascii_hexdigit())
                .map(|c| format!("md5 password contains non hexadecimal characters, e.g. '{c}'\n"))
        }
        None => (item.required && value.is_empty())
            .then(|| format!("Empty Password not allowed in Resource \"{resource_name}\"\n")),
    }
}

/// Values of a list directive (`a,b, c`), like the lexer splits at commas.
fn list_values(value: &str) -> Vec<&str> {
    value.split(',').map(str::trim).filter(|v| !v.is_empty()).collect()
}

/// ConfigurationParser::ParseConfigFile on the new resource file, reduced to
/// the checks that apply to directives `configure add` can write: pass 1
/// stores (names, booleans, integers, passwords) and SaveResource, pass 2
/// resolves resource references. Returns the ConfigureLexErrorHandler lines.
fn parse_check(cfg: &DirectorConfig, table: &Table, items: &'static [Item], name: &str, lines: &[Line]) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();
    let mut stored_name: Option<String> = None;
    let mut present: Vec<&str> = Vec::new();
    for line in lines {
        let item = line.item.unwrap();
        let value = line.value.as_str();
        let word = value.split_whitespace().next().unwrap_or("");
        let error = match item.ty {
            Ty::Name => {
                if value.len() > MAX_NAME_LENGTH - 1 {
                    Some(format!("name {value} length {} too long, max is {}\n", value.len(), MAX_NAME_LENGTH - 1))
                } else if let Some(msg) = name_error(value) {
                    Some(format!("{msg}\n"))
                } else if let Some(old) = &stored_name {
                    Some(format!("Attempt to redefine name \"{old}\" to \"{value}\"."))
                } else {
                    stored_name = Some(value.to_string());
                    None
                }
            }
            Ty::Bool | Ty::Bit => (!["yes", "true", "no", "false"].iter().any(|b| word.eq_ignore_ascii_case(b)))
                .then(|| format!("Expect YES, NO, TRUE, or FALSE, got: {word}")),
            Ty::Pint32 => (!is_a_number(word) || word.parse::<f64>().map_or(true, |v| v < 0.0))
                .then(|| format!("expected a positive integer number, got: {word}")),
            Ty::Int32 => {
                let token = if word.starts_with(|c: char| c.is_ascii_alphabetic()) { "BCT_IDENTIFIER" }
                    else if word.starts_with(|c: char| c.is_ascii_digit()) { "BCT_NUMBER" }
                    else { "BCT_UNQUOTED_STRING" };
                (token != "BCT_NUMBER" || !is_a_number(word))
                    .then(|| format!("expected an integer number, got {token}: {word}"))
            }
            Ty::Autopassword => password_error(table, item, value, stored_name.as_deref().unwrap_or("")),
            _ => None,
        };
        match error {
            Some(e) => errors.push(e),
            None => present.push(item.name),
        }
    }
    let fail = |mut errors: Vec<String>, last: &str| {
        errors.push(last.to_string());
        Err(errors.iter().map(|e| format!("configure error: {e}\n")).collect::<String>())
    };
    // BCT_EOB: name check, then SaveResource (ValidateResource except for
    // Job/JobDefs, which are checked after applying JobDefs).
    if stored_name.is_none() {
        errors.push("Name not specified for resource".into());
        return fail(errors, "ParseAllTokens failed.");
    }
    if table.rcode != "R_JOB" && table.rcode != "R_JOBDEFS"
        && items.iter().any(|i| i.required && !present.contains(&i.name)) {
        errors.push("SaveResource failed".into());
        return fail(errors, "ParseAllTokens failed.");
    }
    if !errors.is_empty() {
        return fail(errors, "Parser Error occurred.");
    }
    // Pass 2: resource references (the new resource already exists then).
    for (index, line) in lines.iter().enumerate() {
        let item = line.item.unwrap();
        let Some(target) = item.code.and_then(table_by_rcode) else { continue };
        let exists = |v: &str| resource_exists(cfg, target, v) || (target.rcode == table.rcode && v == name);
        let line_no = index + 2;
        let text = directive_text(item, &line.value);
        match item.ty {
            Ty::Res => {
                let v = line.value.as_str();
                if !exists(v) {
                    errors.push(format!("Could not find config resource \"{v}\" referenced on line {line_no}: {text}"));
                }
            }
            Ty::AlistRes => {
                if list_values(&line.value).iter().any(|v| !exists(v)) {
                    errors.push(format!("Could not find config Resource \"{}\" referenced on line {line_no} : {text}\n", item.name));
                }
            }
            _ => {}
        }
    }
    if !errors.is_empty() {
        return fail(errors, "Parser Error occurred.");
    }
    Ok(())
}

/// Job requirements after PropagateJobdefs (ValidateResource,
/// JobResource::Validate) for `configure add job`.
fn job_valid(cfg: &DirectorConfig, lines: &[Line]) -> bool {
    let own = |name: &str| lines.iter().find(|l| l.item.is_some_and(|i| i.name == name)).map(|l| l.value.clone());
    let jobdefs = own("JobDefs").and_then(|jd| cfg.raw.iter()
        .find(|r| r.norm_type() == "jobdefs" && r.name() == Some(jd.as_str())).cloned());
    let get = |name: &str| own(name).or_else(|| jobdefs.as_ref().and_then(|r| r.directives.iter()
        .find(|d| d.norm_key() == name.to_ascii_lowercase()).and_then(|d| d.first().map(str::to_string))));
    if tables::JOB_ITEMS.iter().any(|i| i.required && get(i.name).is_none()) {
        return false;
    }
    let job_type = get("Type").unwrap_or_default().to_ascii_lowercase();
    if job_type != "copy" && job_type != "migrate" {
        if get("Client").is_none() || get("FileSet").is_none() {
            return false;
        }
        let pool_storage = get("Pool").is_some_and(|p| cfg.raw.iter().any(|r| r.norm_type() == "pool"
            && r.name() == Some(p.as_str()) && r.directives.iter().any(|d| d.norm_key() == "storage")));
        if get("Storage").is_none() && !pool_storage {
            return false;
        }
    }
    true
}

/// Add the parsed resource to the running configuration (end of its chain).
fn append_resource(cfg: &mut DirectorConfig, text: &str, path: &Path) {
    match bareos_config::raw::parse_str(text, Some(path)) {
        Ok(raw) => cfg.raw.extend(raw),
        Err(e) => tracing::warn!("configure add: raw view of {}: {e}", path.display()),
    }
    match bareos_config::parse_config(text) {
        Ok(resources) => for resource in resources {
            match resource {
                Resource::Director(_) => {}
                Resource::Client(c) => cfg.clients.push(c),
                Resource::Job(j) => cfg.jobs.push(j),
                Resource::Storage(s) => cfg.storages.push(s),
                Resource::Catalog(c) => cfg.catalogs.push(c),
                Resource::Schedule(s) => cfg.schedules.push(s),
                Resource::Pool(p) => cfg.pools.push(p),
                Resource::FileSet(f) => cfg.filesets.push(f),
                Resource::Console(c) => if !c.name.starts_with("_skip_") { cfg.consoles.push(c) },
                Resource::JobDefs(j) => cfg.jobdefs.push(j),
                Resource::Messages(m) => cfg.messages.push(m),
                Resource::Profile(p) => cfg.profiles.push(p),
            }
        },
        Err(e) => tracing::warn!("configure add: typed view of {}: {e}", path.display()),
    }
}

/// ConfigurationParser::RemoveResource for a failed add.
fn remove_resource(cfg: &mut DirectorConfig, table: &Table, name: &str) {
    let rtype = table.name.to_ascii_lowercase();
    cfg.raw.retain(|r| !(r.norm_type() == rtype && r.name() == Some(name)));
    match table.rcode {
        "R_CLIENT" => cfg.clients.retain(|r| r.name != name),
        "R_JOB" => cfg.jobs.retain(|r| r.name != name),
        "R_STORAGE" => cfg.storages.retain(|r| r.name != name),
        "R_CATALOG" => cfg.catalogs.retain(|r| r.name != name),
        "R_SCHEDULE" => cfg.schedules.retain(|r| r.name != name),
        "R_POOL" => cfg.pools.retain(|r| r.name != name),
        "R_FILESET" => cfg.filesets.retain(|r| r.name != name),
        "R_CONSOLE" => cfg.consoles.retain(|r| r.name != name),
        "R_JOBDEFS" => cfg.jobdefs.retain(|r| r.name != name),
        "R_MSGS" => cfg.messages.retain(|r| r.name != name),
        "R_PROFILE" => cfg.profiles.retain(|r| r.name != name),
        _ => {}
    }
}

/// ConfigureAdd + ConfigureAddResource. `args[0]` is the resource type.
/// Returns the text output and the JSON object, or the error lines.
fn add_resource(cfg: &mut DirectorConfig, layout: &Layout, args: &[Argument]) -> Result<(String, Value), String> {
    let type_key = args[0].key.as_str();
    let table = table_by_name(type_key).filter(|t| t.items.is_some())
        .ok_or_else(|| format!("invalid resource type {type_key}.\n"))?;
    if table.rcode == "R_DIRECTOR" {
        return Err("Only one Director resource allowed.\n".into());
    }
    let items = table.items.unwrap();
    let (name, text, lines) = create_resource_string(table, items, args)?;
    if resource_exists(cfg, table, &name) {
        return Err(format!("Resource \"{}\" with name \"{name}\" already exists.\n", table.name));
    }

    // GetPathOfNewResource(error_if_exists = true, no directory creation)
    let directory = layout.include_dir.join(table.name.to_ascii_lowercase());
    let path = directory.join(format!("{name}.conf"));
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    if !directory.is_dir() {
        return Err(format!("Resource config directory \"{}/\" does not exist.\n", directory.display()));
    }
    if path.exists() {
        return Err(format!("Resource config file \"{}\" already exists.\n", path.display()));
    }
    if tmp.exists() {
        return Err(format!("Temporary resource config file \"{}.tmp\" already exists.\n", path.display()));
    }
    let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()));
    if written.is_err() {
        return Err("failed to write config resource file\n".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o640));
    }

    if let Err(errors) = parse_check(cfg, table, items, &name, &lines) {
        let _ = std::fs::remove_file(&tmp);
        return Err(errors);
    }
    append_resource(cfg, &text, &path);
    // ValidateResource after PropagateJobdefs (always true for JobDefs).
    if table.rcode == "R_JOB" && !job_valid(cfg, &lines) {
        let _ = std::fs::remove_file(&tmp);
        remove_resource(cfg, table, &name);
        return Err(format!("failed to create config resource \"{name}\"\n"));
    }
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        remove_resource(cfg, table, &name);
        return Err(format!("failed to create config file \"{}\"\n", path.display()));
    }

    let mut output = String::new();
    let mut object = serde_json::Map::new();
    // Adding a client also creates the File Daemon's Director resource.
    if table.rcode == "R_CLIENT" {
        match export_client(cfg, layout, &name) {
            Ok((text, json)) => { output.push_str(&text); object.insert("export".into(), json); }
            Err(e) => tracing::warn!("configure add client: export failed: {e}"),
        }
    }
    let filename = path.display().to_string();
    output.push_str(&format!("Created resource config file \"{filename}\":\n{text}"));
    object.insert("add".into(), json!({
        "resource": table.name, "name": name, "filename": filename, "content": text,
    }));
    Ok((output, Value::Object(object)))
}

fn configure_add(ast: &CommandAst, ctx: &ExecContext, layout: Option<&Layout>) -> CommandResult {
    let (Some(config), Some(layout)) = (&ctx.config, layout) else {
        return CommandResult::error("No config loaded\n".to_string());
    };
    let mut cfg = match config.write() {
        Ok(cfg) => cfg,
        Err(_) => return CommandResult::error("Config error\n".to_string()),
    };
    match add_resource(&mut cfg, layout, &ast.arguments[1..]) {
        Ok((text, json)) => CommandResult::ok(text).with_json(json!({ "configure": json })),
        Err(errors) => CommandResult::error(errors),
    }
}

// ---------------------------------------------------------------------------
// configure export
// ---------------------------------------------------------------------------

/// ConfigureCreateFdResource: write the client's Director resource for the
/// File Daemon below `<configdir>/bareos-dir-export/`.
fn export_client(cfg: &DirectorConfig, layout: &Layout, client: &str) -> Result<(String, Value), String> {
    let res = cfg.find_client(client).ok_or_else(String::new)?;
    let ndmp = cfg.raw.iter()
        .find(|r| r.norm_type() == "client" && r.name() == Some(client))
        .and_then(|r| r.directives.iter().find(|d| d.norm_key() == "protocol").and_then(|d| d.first().map(str::to_string)))
        .is_some_and(|p| p.to_ascii_lowercase().starts_with("ndmp"));
    let password = if ndmp {
        res.password.clone()
    } else {
        match res.password.strip_prefix("[md5]") {
            Some(hash) => format!("[md5]{hash}"),
            None => format!("[md5]{}", bareos_bnet::cram_md5::md5_hash_password(&res.password)),
        }
    };
    let dirname = cfg.director.name.clone();
    let content = format!("Director {{\n  Name = \"{dirname}\"\n  Password = \"{password}\"\n}}\n");

    let directory = layout.config_dir
        .join(format!("bareos-dir-export/client/{client}/bareos-fd.d"))
        .join("director");
    let _ = std::fs::create_dir_all(&directory);
    if !directory.is_dir() {
        return Err(format!("Resource config directory \"{}/\" does not exist.\n", directory.display()));
    }
    let path = directory.join(format!("{dirname}.conf"));
    let written = std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, content.as_bytes()));
    if written.is_err() {
        return Err("failed to write filedaemon config resource file\n".into());
    }
    let filename = path.display().to_string();
    let text = format!("Exported resource file \"{filename}\":\n{content}");
    Ok((text, json!({
        "clientname": client, "component": "bareos-fd", "resource": "director",
        "name": dirname, "filename": filename, "content": content,
    })))
}

fn configure_export(ast: &CommandAst, ctx: &ExecContext, layout: Option<&Layout>) -> CommandResult {
    const EXPORT_USAGE: &str = "usage: configure export client=<clientname>\n";
    // FindArgWithValue(ua, "client") + ua->GetClientResWithName (ACL checked)
    let Some(client) = ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case("client") && a.value.is_some())
        .and_then(|a| a.value.clone()) else {
        return CommandResult::error(EXPORT_USAGE.to_string());
    };
    let (Some(config), Some(layout)) = (&ctx.config, layout) else {
        return CommandResult::error(EXPORT_USAGE.to_string());
    };
    let known = config.read().map(|cfg| cfg.find_client(&client).is_some()).unwrap_or(false);
    if !known || !crate::acl::access(ctx, crate::acl::Kind::Client, &client) {
        return CommandResult::error(EXPORT_USAGE.to_string());
    }
    let cfg = match config.read() {
        Ok(cfg) => cfg,
        Err(_) => return CommandResult::error("Config error\n".to_string()),
    };
    match export_client(&cfg, layout, &client) {
        Ok((text, json)) => CommandResult::ok(text).with_json(json!({ "configure": { "export": json } })),
        Err(errors) => CommandResult::error(errors),
    }
}

// ---------------------------------------------------------------------------
// Onesimus extensions: configure read / update / delete
// ---------------------------------------------------------------------------

fn configure_delete(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let config = match &ctx.config {
        Some(c) => c,
        None => return CommandResult::error("No config loaded\n".to_string()),
    };

    // Check for client=, job=, pool= arguments
    for arg in &ast.arguments {
        if let Some(name) = &arg.value {
            let mut cfg = config.write().unwrap();
            match arg.key.as_str() {
                "client" | "job" | "pool" | "storage" | "console" | "schedule" => {
                    let resource_type = arg.key.as_str();
                    let (vec_len_before, removed) = match resource_type {
                        "client" => { let b = cfg.clients.len(); cfg.clients.retain(|c| c.name != *name); (b, b > cfg.clients.len()) }
                        "job" => { let b = cfg.jobs.len(); cfg.jobs.retain(|j| j.name != *name); (b, b > cfg.jobs.len()) }
                        "pool" => { let b = cfg.pools.len(); cfg.pools.retain(|p| p.name != *name); (b, b > cfg.pools.len()) }
                        "storage" => { let b = cfg.storages.len(); cfg.storages.retain(|s| s.name != *name); (b, b > cfg.storages.len()) }
                        "console" => {
                            let b = cfg.consoles.len();
                            if cfg.consoles.iter().position(|c| c.name == *name).is_some_and(|i| i < cfg.loaded_consoles) {
                                cfg.loaded_consoles -= 1;
                            }
                            cfg.consoles.retain(|c| c.name != *name);
                            (b, b > cfg.consoles.len())
                        }
                        "schedule" => { let b = cfg.schedules.len(); cfg.schedules.retain(|s| s.name != *name); (b, b > cfg.schedules.len()) }
                        _ => (0, false),
                    };
                    if removed {
                        // Delete from disk
                        if let Some(dir) = config_dir(ctx) {
                            let _ = delete_resource_file(&dir, resource_type, name);
                        }
                        let rt = resource_type.chars().next().unwrap().to_uppercase().to_string()
                            + &resource_type[1..];
                        return CommandResult::ok(format!("Deleted {rt} '{name}'\n"));
                    }
                    let rt = resource_type.chars().next().unwrap().to_uppercase().to_string()
                        + &resource_type[1..];
                    return CommandResult::error(format!("{rt} '{name}' not found\n"));
                }
                _ => {}
            }
        }
    }

    CommandResult::ok("configure delete: specify client=<name>, job=<name>, pool=<name>, etc.\n".to_string())
}

fn configure_read(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let config = match &ctx.config {
        Some(c) => match c.read() {
            Ok(c) => c,
            Err(_) => return CommandResult::error("Config error\n".to_string()),
        },
        None => return CommandResult::error("No config\n".to_string()),
    };

    let get_arg = |key: &str| -> Option<&str> {
        ast.arguments.iter().find(|a| a.key == key).and_then(|a| a.value.as_deref())
    };

    if let Some(name) = get_arg("client") {
        if let Some(c) = config.find_client(name) {
            return CommandResult::ok(format!(
                "Client {{\n  Name = \"{}\"\n  Address = \"{}\"\n  Port = {}\n  Enabled = {}\n}}\n",
                c.name, c.address, c.port, if c.enabled { "yes" } else { "no" }
            ));
        }
        return CommandResult::error(format!("Client '{name}' not found\n"));
    }
    if let Some(name) = get_arg("job") {
        if let Some(j) = config.find_job(name) {
            return CommandResult::ok(format!(
                "Job {{\n  Name = \"{}\"\n  Type = {:?}\n  Level = {:?}\n  Client = \"{}\"\n  \
                 Storage = \"{}\"\n  Pool = \"{}\"\n  FileSet = \"{}\"\n  Priority = {}\n  Enabled = {}\n}}\n",
                j.name, j.job_type, j.level, j.client, j.storage, j.pool, j.fileset, j.priority,
                if j.enabled { "yes" } else { "no" }
            ));
        }
        return CommandResult::error(format!("Job '{name}' not found\n"));
    }
    if let Some(name) = get_arg("pool") {
        if let Some(p) = config.find_pool(name) {
            return CommandResult::ok(format!(
                "Pool {{\n  Name = \"{}\"\n  Pool Type = \"{}\"\n  Recycle = {}\n  \
                 Auto Prune = {}\n  Maximum Volumes = {}\n}}\n",
                p.name, p.pool_type, if p.recycle { "yes" } else { "no" },
                if p.auto_prune { "yes" } else { "no" }, p.maximum_volumes
            ));
        }
        return CommandResult::error(format!("Pool '{name}' not found\n"));
    }
    if let Some(name) = get_arg("storage") {
        if let Some(s) = config.find_storage(name) {
            return CommandResult::ok(format!(
                "Storage {{\n  Name = \"{}\"\n  Address = \"{}\"\n  Port = {}\n  \
                 Device = \"{}\"\n  Media Type = \"{}\"\n}}\n",
                s.name, s.address, s.port, s.device, s.media_type
            ));
        }
        return CommandResult::error(format!("Storage '{name}' not found\n"));
    }

    CommandResult::ok("configure read: specify client=, job=, pool=, or storage=\n".to_string())
}

fn configure_update(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let config = match &ctx.config {
        Some(c) => c,
        None => return CommandResult::error("No config loaded\n".to_string()),
    };

    let get_arg = |key: &str| -> Option<&str> {
        ast.arguments.iter()
            .find(|a| a.key == key)
            .and_then(|a| a.value.as_deref())
    };

    // configure update client=<name> address=<new> port=<new>
    if let Some(name) = get_arg("client") {
        let mut cfg = config.write().unwrap();
        if let Some(c) = cfg.clients.iter_mut().find(|c| c.name == name) {
            let mut changes = Vec::new();
            if let Some(addr) = get_arg("address") {
                c.address = addr.to_string();
                changes.push(format!("address={addr}"));
            }
            if let Some(port) = get_arg("port") {
                if let Ok(p) = port.parse() {
                    c.port = p;
                    changes.push(format!("port={port}"));
                }
            }
            if let Some(pw) = get_arg("password") {
                c.password = pw.to_string();
                changes.push("password=***".to_string());
            }
            if changes.is_empty() {
                return CommandResult::ok(format!("Client '{name}': no changes specified\n"));
            }
            // Re-persist the updated client
            if let Some(dir) = config_dir(ctx) {
                let content = format!(
                    "Client {{\n  Name = \"{}\"\n  Address = \"{}\"\n  Port = {}\n  Password = \"{}\"\n}}\n",
                    c.name, c.address, c.port, c.password
                );
                let _ = persist_resource(&dir, "client", name, &content);
            }
            return CommandResult::ok(format!("Client '{name}' updated: {}\n", changes.join(", ")));
        }
        return CommandResult::error(format!("Client '{name}' not found\n"));
    }

    if let Some(name) = get_arg("job") {
        let mut cfg = config.write().unwrap();
        if let Some(j) = cfg.jobs.iter_mut().find(|j| j.name == name) {
            let mut changes = Vec::new();
            if let Some(client) = get_arg("newclient") {
                j.client = client.to_string();
                changes.push(format!("client={client}"));
            }
            if let Some(storage) = get_arg("newstorage") {
                j.storage = storage.to_string();
                changes.push(format!("storage={storage}"));
            }
            if let Some(pool) = get_arg("newpool") {
                j.pool = pool.to_string();
                changes.push(format!("pool={pool}"));
            }
            if changes.is_empty() {
                return CommandResult::ok(format!("Job '{name}': no changes specified\n"));
            }
            return CommandResult::ok(format!("Job '{name}' updated: {}\n", changes.join(", ")));
        }
        return CommandResult::error(format!("Job '{name}' not found\n"));
    }

    CommandResult::ok(
        "configure update: specify resource to update:\n  \
         configure update client=<name> address=<new> port=<new>\n  \
         configure update job=<name> newclient=<c> newstorage=<s>\n"
            .to_string()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
Director { Name = bareos-dir; Password = "dirpw" }
Client { Name = bareos-fd; Address = localhost; Password = "1VapECOFxpMPqIm1xhwB+ekXe8ZDcY0HNUc3P5LJNR2u" }
Messages { Name = Standard }
Pool { Name = Full; Pool Type = Backup; Storage = File }
Storage { Name = File; Address = localhost; Password = "sdpw"; Device = FileStorage; Media Type = File }
FileSet { Name = LinuxAll }
JobDefs { Name = DefaultJob; Type = Backup; Client = bareos-fd; FileSet = LinuxAll; Messages = Standard; Pool = Full }
Profile { Name = operator }
"#;

    fn setup(tag: &str) -> (DirectorConfig, Layout) {
        let base = std::env::temp_dir().join(format!("onesimus-configure-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for t in ["client", "console", "job", "jobdefs", "pool", "profile"] {
            std::fs::create_dir_all(base.join("bareos-dir.d").join(t)).unwrap();
        }
        let layout = layout(&base);
        (DirectorConfig::from_str(BASE).unwrap(), layout)
    }

    fn args(line: &str) -> Vec<Argument> {
        line.split(' ').map(|t| match t.split_once('=') {
            Some((k, v)) => Argument { key: k.into(), value: Some(v.into()) },
            None => Argument { key: t.into(), value: None },
        }).collect()
    }

    #[test]
    fn add_console_writes_file_and_running_config() {
        let (mut cfg, layout) = setup("console");
        let (text, json) = add_resource(&mut cfg, &layout, &args("console name=op password=secret jobacl=*all* commandacl=status,show")).unwrap();
        let path = layout.include_dir.join("console/op.conf");
        let content = "Console {\n  Name = \"op\"\n  Password = \"secret\"\n  JobAcl = *all*\n  CommandAcl = status,show\n}\n";
        assert_eq!(text, format!("Created resource config file \"{}\":\n{content}", path.display()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        assert!(!PathBuf::from(format!("{}.tmp", path.display())).exists());
        assert_eq!(json["add"]["resource"], "Console");
        assert!(cfg.find_console("op").is_some());
        assert!(cfg.raw.last().unwrap().name() == Some("op"));
        let err = add_resource(&mut cfg, &layout, &args("console name=op password=x")).unwrap_err();
        assert_eq!(err, "Resource \"Console\" with name \"op\" already exists.\n");
    }

    #[test]
    fn add_errors_match_original_texts() {
        let (mut cfg, layout) = setup("errors");
        let err = |cfg: &mut DirectorConfig, line: &str| add_resource(cfg, &layout, &args(line)).unwrap_err();
        assert_eq!(err(&mut cfg, "nosuch name=x"), "invalid resource type nosuch.\n");
        assert_eq!(err(&mut cfg, "device name=x"), "invalid resource type device.\n");
        assert_eq!(err(&mut cfg, "director name=x"), "Only one Director resource allowed.\n");
        assert_eq!(err(&mut cfg, "console name=a password=b nosuch=1"), "Resource \"Console\" does not permit directive \"nosuch\".\n");
        assert_eq!(err(&mut cfg, "console name=a password=b;c"), "Could not add directive \"password\": character ';' (end of directive) is forbidden.\n");
        assert_eq!(err(&mut cfg, "console password=b"), "Resource \"Console\": missing name parameter.\n");
        assert_eq!(err(&mut cfg, "console name=a password"), "Missing value for directive \"password\"\n");
        assert_eq!(err(&mut cfg, "console name=a"), "configure error: SaveResource failed\nconfigure error: ParseAllTokens failed.\n");
        assert_eq!(err(&mut cfg, "console name=a password=b usepamauthentication=maybe"),
            "configure error: Expect YES, NO, TRUE, or FALSE, got: maybe\nconfigure error: Parser Error occurred.\n");
        assert_eq!(err(&mut cfg, "console name=a! password=b"),
            "configure error: Illegal character \"!\" in name.\n\n\nconfigure error: Name not specified for resource\nconfigure error: ParseAllTokens failed.\n");
        assert_eq!(err(&mut cfg, "console=a name=b password=c"),
            "configure error: Attempt to redefine name \"a\" to \"b\".\nconfigure error: Parser Error occurred.\n");
        assert_eq!(err(&mut cfg, "console name=a password="),
            "configure error: Empty Password not allowed in Resource \"a\"\n\nconfigure error: SaveResource failed\nconfigure error: ParseAllTokens failed.\n");
        assert_eq!(err(&mut cfg, "console name=a password=b profile=nosuch"),
            "configure error: Could not find config Resource \"Profile\" referenced on line 4 :   Profile = nosuch\n\n\nconfigure error: Parser Error occurred.\n");
        assert_eq!(err(&mut cfg, "profile name=operator"), "Resource \"Profile\" with name \"operator\" already exists.\n");
        let counter = layout.include_dir.join("counter");
        assert_eq!(err(&mut cfg, "counter name=c"), format!("Resource config directory \"{}/\" does not exist.\n", counter.display()));
        // nothing was added or left behind
        assert!(cfg.find_console("a").is_none());
        assert_eq!(std::fs::read_dir(layout.include_dir.join("console")).unwrap().count(), 0);
    }

    #[test]
    fn add_pool_and_job_validate_references_and_jobdefs() {
        let (mut cfg, layout) = setup("job");
        let err = add_resource(&mut cfg, &layout, &args("pool name=P storage=Nope")).unwrap_err();
        assert_eq!(err, "configure error: Could not find config Resource \"Storage\" referenced on line 3 :   Storage = Nope\n\n\nconfigure error: Parser Error occurred.\n");
        let err = add_resource(&mut cfg, &layout, &args("pool name=P maximumvolumes=-1")).unwrap_err();
        assert_eq!(err, "configure error: expected a positive integer number, got: -1\nconfigure error: Parser Error occurred.\n");
        add_resource(&mut cfg, &layout, &args("pool name=P storage=File")).unwrap();
        assert!(cfg.find_pool("P").is_some());
        // Job without client/fileset/messages after JobDefs propagation
        let err = add_resource(&mut cfg, &layout, &args("job name=J type=Backup pool=P")).unwrap_err();
        assert_eq!(err, "failed to create config resource \"J\"\n");
        assert!(cfg.find_job("J").is_none());
        let (text, _) = add_resource(&mut cfg, &layout, &args("job name=J jobdefs=DefaultJob")).unwrap();
        assert!(text.ends_with("Job {\n  Name = \"J\"\n  JobDefs = \"DefaultJob\"\n}\n"));
        assert!(cfg.find_job("J").is_some());
    }

    #[test]
    fn add_client_also_exports_fd_director_resource() {
        let (mut cfg, layout) = setup("client");
        let (text, json) = add_resource(&mut cfg, &layout, &args("client name=c1 address=host password=secret")).unwrap();
        let export = layout.config_dir.join("bareos-dir-export/client/c1/bareos-fd.d/director/bareos-dir.conf");
        let fd = format!("Director {{\n  Name = \"bareos-dir\"\n  Password = \"[md5]{}\"\n}}\n",
            bareos_bnet::cram_md5::md5_hash_password("secret"));
        let add = layout.include_dir.join("client/c1.conf");
        assert_eq!(text, format!("Exported resource file \"{}\":\n{fd}Created resource config file \"{}\":\n\
            Client {{\n  Name = \"c1\"\n  Address = \"host\"\n  Password = \"secret\"\n}}\n", export.display(), add.display()));
        assert_eq!(std::fs::read_to_string(&export).unwrap(), fd);
        assert_eq!(json["export"]["component"], "bareos-fd");
        assert!(cfg.find_client("c1").is_some());
    }

    #[test]
    fn export_overwrites_existing_file() {
        let (cfg, layout) = setup("export");
        let (first, _) = export_client(&cfg, &layout, "bareos-fd").unwrap();
        let (second, _) = export_client(&cfg, &layout, "bareos-fd").unwrap();
        assert_eq!(first, second);
        assert!(first.contains("Password = \"[md5]c161a32b19919446d823477532ff45e5\""));
    }

    #[test]
    fn layout_follows_config_path() {
        let (_, l) = setup("layout");
        assert!(l.using_include_dir);
        let inner = layout(&l.include_dir);
        assert_eq!(inner.config_dir, l.config_dir);
        assert_eq!(inner.include_dir, l.include_dir);
    }
}
