// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
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
        // One ErrorMsg call (one BNET_ERROR_MSG, one `.api 2` message).
        return with_warning(CommandResult::error(USAGE.to_string()).with_error_parts());
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

/// Types config_add_directive writes in quotes; every other type (time,
/// size, number, yes/no, ...) is written as given, because the original's
/// store functions for them refuse a quoted value ("expected a time period",
/// lib/res.cc:1095).
fn quoted_type(ty: Ty) -> bool {
    matches!(ty, Ty::Autopassword | Ty::Dir | Ty::DirOrCmd | Ty::Label | Ty::Name
        | Ty::Res | Ty::Shrtrunscript | Ty::Stdstr | Ty::Stdstrdir | Ty::Str | Ty::Strname)
}

/// config_add_directive: format one directive, quoting string-like types.
fn directive_text(item: &Item, value: &str) -> String {
    if !quoted_type(item.ty) {
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
    let jobdefs = own("JobDefs").and_then(|jd| bareos_config::jobdefs::effective(&cfg.raw, "jobdefs", &jd));
    let get = |name: &str| own(name).or_else(|| jobdefs.as_ref().and_then(|dirs| dirs.iter()
        .map(|e| &e.directive)
        .find(|d| bareos_config::jobdefs::item_key(&d.key) == name.to_ascii_lowercase())
        .and_then(|d| d.first().map(str::to_string))));
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
                // PropagateJobdefs() for the new Job/JobDefs (ua_configure.cc)
                Resource::Job(mut j) => {
                    bareos_config::jobdefs::propagate_job(&cfg.jobdefs, &mut j);
                    cfg.jobs.push(j);
                }
                Resource::Storage(s) => cfg.storages.push(s),
                Resource::Catalog(c) => cfg.catalogs.push(c),
                Resource::Schedule(s) => cfg.schedules.push(s),
                Resource::Pool(p) => cfg.pools.push(p),
                Resource::FileSet(f) => cfg.filesets.push(f),
                Resource::Console(c) => if !c.name.starts_with("_skip_") { cfg.consoles.push(c) },
                Resource::JobDefs(mut j) => {
                    if j.job.jobdefs != j.name {
                        if let Some(src) = cfg.jobdefs.iter().find(|d| d.name == j.job.jobdefs) {
                            let src = src.job.clone();
                            bareos_config::jobdefs::propagate_typed_one(&mut j.job, &src);
                        }
                    }
                    cfg.jobdefs.push(j);
                }
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
    add_resource_by(cfg, layout, args, "")
}

/// [`add_resource`] for the console `creator` ("" for the internal callers
/// that have no console): an assistant console stores no program (S-55).
fn add_resource_by(cfg: &mut DirectorConfig, layout: &Layout, args: &[Argument], creator: &str) -> Result<(String, Value), String> {
    let type_key = args[0].key.as_str();
    let table = table_by_name(type_key).filter(|t| t.items.is_some())
        .ok_or_else(|| format!("invalid resource type {type_key}.\n"))?;
    if table.rcode == "R_DIRECTOR" {
        return Err("Only one Director resource allowed.\n".into());
    }
    let items = table.items.unwrap();
    let (name, text, lines) = create_resource_string(table, items, args)?;
    // an assistant console stores no program (S-55)
    if let Some(why) = crate::config_edit::assistant_refusal(creator, &text) {
        return Err(format!("{why}\n"));
    }
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

/// Resource types `configure add` refuses (decision of the maintainer,
/// 19.11.2026, F-1 of docs/plans/ACL-ZWEISTUFIG.md): Profile and User carry
/// the ACLs of the outer tier, which are written in the editor only.
/// Deviation from Bareos (ConfigureAddResource accepts them). Console is
/// accepted in the narrow form of [`configure_add_console`] since the
/// amendment of 20.11.2026.
const ADD_REFUSED: [&str; 2] = ["profile", "user"];

/// The directives `configure add console` accepts (normalized keys).
const CONSOLE_DIRECTIVES: [&str; 4] = ["name", "password", "profile", "description"];

/// The checked arguments of `configure add console`.
struct ConsoleRequest {
    /// Name of the new console.
    name: String,
    /// Its password (never echoed).
    password: String,
    /// Its profiles in the given order (at least one).
    profiles: Vec<String>,
    /// Optional description.
    description: Option<String>,
}

/// Parse and check the arguments of `configure add console` (`args[0]` is
/// the resource type). Only name, password, one or more profile (repeated
/// or as a comma list) and an optional description are accepted; keys are
/// normalized like Bareos keywords.
fn console_request(cfg: &DirectorConfig, args: &[Argument]) -> Result<ConsoleRequest, String> {
    if args[0].value.is_some() {
        return Err("give the name as name=<name>".into());
    }
    let (mut name, mut password, mut description) = (None, None, None);
    let mut profiles: Vec<String> = Vec::new();
    for arg in &args[1..] {
        let key = bareos_config::edit::norm_key(&arg.key);
        if key.ends_with("acl") {
            return Err(format!("directive \"{}\" not allowed: a new console has no own ACL lines; it gets the \
                rights of its profiles, the catalog ACL (.acl set) can narrow them", arg.key));
        }
        if !CONSOLE_DIRECTIVES.contains(&key.as_str()) {
            return Err(format!("directive \"{}\" not allowed: only name, password, profile and description \
                (other directives in the editor)", arg.key));
        }
        let Some(value) = arg.value.as_deref() else {
            return Err(format!("missing value for directive \"{}\"", arg.key));
        };
        if value.chars().any(|c| c.is_control() || matches!(c, '"' | '\\' | ';' | '@')) {
            return Err(format!("directive \"{key}\": control characters, '\"', '\\', ';' and '@' are not allowed"));
        }
        let slot = match key.as_str() {
            "name" => &mut name,
            "password" => &mut password,
            "description" => &mut description,
            _ => {
                let given = list_values(value);
                if given.is_empty() {
                    return Err("profile: empty value".into());
                }
                profiles.extend(given.into_iter().map(str::to_string));
                continue;
            }
        };
        if slot.replace(value.to_string()).is_some() {
            return Err(format!("directive \"{key}\" given twice"));
        }
    }
    let name = name.ok_or("missing name=<name>")?;
    if name == "*UserAgent*" {
        return Err("the name *UserAgent* is reserved for the default console".into());
    }
    if let Some(e) = name_error(&name) {
        return Err(format!("invalid name: {}", e.trim_end()));
    }
    if cfg.raw.iter().any(|r| r.norm_type() == "console" && r.name() == Some(name.as_str())) {
        return Err(format!("Resource \"Console\" with name \"{name}\" already exists"));
    }
    let password = password.filter(|p| !p.is_empty()).ok_or("missing password=<password>")?;
    if profiles.is_empty() {
        return Err("give at least one profile=<profile>; the new console gets the rights of its profiles".into());
    }
    for profile in &profiles {
        if name_error(profile).is_some() || profile.contains(' ') {
            return Err(format!("invalid profile name \"{profile}\""));
        }
        if !cfg.raw.iter().any(|r| r.norm_type() == "profile" && r.name() == Some(profile.as_str())) {
            return Err(format!("unknown Profile \"{profile}\""));
        }
    }
    Ok(ConsoleRequest { name, password, profiles, description })
}

/// `configure add console name=<n> password=<p> profile=<P1> [profile=<P2>
/// ...] [description=<d>]` (F-1 amendment of 20.11.2026,
/// docs/plans/ACL-ZWEISTUFIG.md). The new console has no own ACL lines and
/// gets the rights of its profiles; [`crate::acl::grant_error`] refuses
/// profiles that exceed the rights of the creating console. Written and
/// added to the running configuration like every `configure add`
/// ([`add_resource`]); the password is masked in the reply.
fn configure_add_console(ast: &CommandAst, ctx: &ExecContext, layout: Option<&Layout>) -> CommandResult {
    let refused = |e: String| CommandResult::error(format!("configure add console: refused, {e}.\n"))
        .with_json(json!({ "configure": {} }));
    let (Some(config), Some(layout)) = (&ctx.config, layout) else {
        return CommandResult::error("No config loaded\n".to_string());
    };
    let mut cfg = match config.write() {
        Ok(cfg) => cfg,
        Err(_) => return CommandResult::error("Config error\n".to_string()),
    };
    let request = match console_request(&cfg, &ast.arguments[1..]) {
        Ok(request) => request,
        Err(e) => return refused(e),
    };
    let creator = super::config_crud::console_of(ctx);
    let profiles: Vec<&str> = request.profiles.iter().map(String::as_str).collect();
    if let Some(e) = crate::acl::grant_error(&cfg, &creator, &profiles) {
        tracing::warn!("configure add console {}: refused for Console \"{creator}\": {e}", request.name);
        return refused(e);
    }
    let arg = |key: &str, value: &str| Argument { key: key.into(), value: Some(value.into()) };
    let mut args = vec![Argument { key: "console".into(), value: None }, arg("name", &request.name),
        arg("password", &request.password)];
    if let Some(description) = &request.description {
        args.push(arg("description", description));
    }
    args.extend(request.profiles.iter().map(|p| arg("profile", p)));
    let (text, mut json) = match add_resource(&mut cfg, layout, &args) {
        Ok(added) => added,
        // The md5 checks quote one character of the value: never echo it.
        Err(e) if e.contains("md5 password") || e.contains(&request.password) =>
            return refused("the password is not valid (clear text, or [md5] with 32 hex digits)".into()),
        Err(e) => return CommandResult::error(e).with_json(json!({ "configure": {} })),
    };
    // Post-condition: the console as parsed has exactly the requested
    // directives and profiles; otherwise roll back.
    let table = table_by_name("console").expect("console table");
    let path = layout.include_dir.join("console").join(format!("{}.conf", request.name));
    let parsed_ok = cfg.raw.iter().find(|r| r.norm_type() == "console" && r.name() == Some(request.name.as_str()))
        .is_some_and(|r| {
            r.directives.iter().all(|d| CONSOLE_DIRECTIVES.contains(&bareos_config::edit::norm_key(&d.key).as_str()))
                && r.directives.iter().filter(|d| d.norm_key() == "profile").flat_map(|d| d.values())
                    .eq(request.profiles.iter().map(String::as_str))
        });
    if !parsed_ok {
        let _ = std::fs::remove_file(&path);
        remove_resource(&mut cfg, table, &request.name);
        return refused("the written resource does not parse as requested; nothing was added".into());
    }
    let clear = directive_text(find_item(tables::CONSOLE_ITEMS, "password").expect("password item"), &request.password);
    let masked = format!("  Password = \"{}\"\n", crate::secrets::MASK);
    let text = text.replace(&clear, &masked);
    if let Some(content) = json.pointer_mut("/add/content") {
        *content = Value::String(content.as_str().unwrap_or("").replace(&clear, &masked));
    }
    tracing::info!("configure add console: Console \"{creator}\" created Console \"{}\" with Profile {}",
        request.name, request.profiles.join(","));
    CommandResult::ok(text).with_json(json!({ "configure": json }))
}

fn configure_add(ast: &CommandAst, ctx: &ExecContext, layout: Option<&Layout>) -> CommandResult {
    if let Some(kind) = ast.arguments.get(1).map(|a| a.key.to_ascii_lowercase()) {
        if ADD_REFUSED.contains(&kind.as_str()) {
            return CommandResult::error(format!("configure add {kind}: refused. Profile and User resources \
                carry ACLs and are created in the editor only (docs/plans/ACL-ZWEISTUFIG.md).\n"));
        }
        if kind == "console" {
            return configure_add_console(ast, ctx, layout);
        }
    }
    let (Some(config), Some(layout)) = (&ctx.config, layout) else {
        return CommandResult::error("No config loaded\n".to_string());
    };
    let mut cfg = match config.write() {
        Ok(cfg) => cfg,
        Err(_) => return CommandResult::error("Config error\n".to_string()),
    };
    // ConfigureAdd: ObjectStart("configure") once the resource type is
    // valid and not the Director, so a later failure keeps "configure": {}.
    let type_key = ast.arguments[1].key.as_str();
    let started = table_by_name(type_key).filter(|t| t.items.is_some()).is_some_and(|t| t.rcode != "R_DIRECTOR");
    match add_resource_by(&mut cfg, layout, &ast.arguments[1..], &super::config_crud::console_of(ctx)) {
        Ok((text, json)) => CommandResult::ok(text).with_json(json!({ "configure": json })),
        Err(errors) if started => CommandResult::error(errors).with_json(json!({ "configure": {} })),
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
    let password = if res.protocol.is_ndmp() {
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
        // ConfigureExport: ObjectStart("configure") before the export.
        Err(errors) => CommandResult::error(errors).with_json(json!({ "configure": {} })),
    }
}

// ---------------------------------------------------------------------------
// Onesimus extensions: configure read / update / delete
// ---------------------------------------------------------------------------

/// `configure delete <type>=<name> [yes]` (Onesimus extension).
///
/// Goes through the configuration CRUD ([`crate::config_edit`]), the same
/// path as DeleteResource over gRPC and `.config delete` over BNET: the
/// resource is read for its current SHA-256, then deleted with that
/// checksum, so the reference check (REFERENCED: a Job still names the
/// Client/FileSet/Schedule …), the `odir -t` check of the remaining
/// configuration, the atomic file change and the reload with rollback all
/// apply. Types the CRUD does not edit (Console) are refused:
/// the earlier implementation removed them from memory and deleted the file
/// without any reference check.
fn configure_delete(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let editor = match ctx.config.as_ref().and_then(crate::config_edit::shared_editor) {
        Some(e) => e,
        None => return CommandResult::error("configure delete: configuration editing is not available (no configuration path)\n".to_string()),
    };
    configure_delete_with(ast, ctx, &editor)
}

/// [`configure_delete`] with an explicit editor (tests: a throw-away configuration).
pub(crate) fn configure_delete_with(ast: &CommandAst, ctx: &ExecContext, editor: &crate::config_edit::ConfigEditor) -> CommandResult {
    use crate::config_edit::{RType, Status};
    /// Types `configure delete` accepted before the CRUD path; those without
    /// a CRUD type are refused with a hint.
    const UNSAFE: [&str; 1] = ["console"];
    let Some((key, name)) = ast.arguments.iter()
        .find_map(|a| a.value.as_deref().map(|v| (a.key.to_ascii_lowercase(), v.to_string()))) else {
        return CommandResult::ok("configure delete: specify client=<name>, job=<name>, jobdefs=<name>, schedule=<name>, fileset=<name>, pool=<name> or storage=<name>\n".to_string());
    };
    let Some(rtype) = RType::ALL.into_iter().find(|t| t.norm() == key) else {
        if UNSAFE.contains(&key.as_str()) {
            return CommandResult::error(format!("configure delete: {key} resources cannot be deleted safely yet \
                (no reference check); remove the file by hand and reload\n"));
        }
        return CommandResult::error(format!("configure delete: unknown resource type \"{key}\"\n"));
    };
    let allow = |n: &str| crate::acl::access(ctx, rtype.acl(), n);
    let console = super::config_crud::console_of(ctx);
    let read = match editor.read(rtype, &name, &allow) {
        Ok(o) => o,
        Err(e) => return CommandResult::error(format!("configure delete: {e}\n")),
    };
    if read.status != Status::Ok {
        return CommandResult::error(format!("{}\n", read.message));
    }
    let o = match editor.delete(rtype, &name, &read.sha256, false, &console, &allow) {
        Ok(o) => o,
        Err(e) => return CommandResult::error(format!("configure delete: {e}\n")),
    };
    match o.status {
        Status::Ok => CommandResult::ok(format!("Deleted {} '{name}'\n", rtype.word())),
        Status::Referenced => {
            let mut text = format!("{}; nothing deleted:\n", o.message);
            for (t, n) in &o.referenced_by {
                text.push_str(&format!("  {t} \"{n}\"\n"));
            }
            CommandResult::error(text)
        }
        _ => {
            let mut text = format!("{}\n", o.message);
            if let Some(p) = &o.problem {
                text.push_str(&p.text);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            CommandResult::error(text)
        }
    }
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

    // Same resource ACLs as `show <type>=<name>` (IsResAllowed): a denied
    // resource reads like an unknown one (compatibility log C-062).
    use crate::acl::{access_locked, Kind as Acl};
    let ok = |kind: Acl, name: &str| access_locked(ctx, &config, kind, name);

    if let Some(name) = get_arg("client") {
        if let Some(c) = config.find_client(name).filter(|_| ok(Acl::Client, name)) {
            return CommandResult::ok(format!(
                "Client {{\n  Name = \"{}\"\n  Address = \"{}\"\n  Port = {}\n  Enabled = {}\n}}\n",
                c.name, c.address, c.port, if c.enabled { "yes" } else { "no" }
            ));
        }
        return CommandResult::error(format!("Client '{name}' not found\n"));
    }
    if let Some(name) = get_arg("job") {
        if let Some(j) = config.find_job(name).filter(|_| ok(Acl::Job, name)) {
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
        if let Some(p) = config.find_pool(name).filter(|_| ok(Acl::Pool, name)) {
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
        if let Some(s) = config.find_storage(name).filter(|_| ok(Acl::Storage, name)) {
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

/// `configure update <type>=<name> <directive>=<value> ...` (Onesimus extension).
///
/// Goes through the configuration CRUD ([`crate::config_edit`]) like
/// `configure delete`: the resource is read with its SHA-256, the given
/// directives are changed in its raw text ([`bareos_config::edit::apply`]:
/// comments, order, indentation, line ends and every other directive stay
/// byte for byte), and the text is written back with UpdateResource and
/// the read checksum (the `odir -t` check, the atomic file change and the
/// reload with rollback apply). The earlier implementation rewrote a
/// Client file with four directives and changed a Job only in memory (K-10).
fn configure_update(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let editor = match ctx.config.as_ref().and_then(crate::config_edit::shared_editor) {
        Some(e) => e,
        None => return CommandResult::error("configure update: configuration editing is not available (no configuration path)\n".to_string()),
    };
    configure_update_with(ast, ctx, &editor)
}

/// Usage text of `configure update`.
const UPDATE_USAGE: &str = "configure update: usage: configure update <type>=<name> <directive>=<value> ...\n  \
    type: client, job, jobdefs, schedule, fileset, pool, storage; a directive given several times (Run) replaces all its \
    lines, <directive>= removes it; with the switch `overlay` the directives are Onesimus directives (gRPC, mDNS, Health \
    Check Interval, DB Driver) and go to odir.d/ in the Onesimus configuration directory, not to bareos-dir.d\n";

/// One directive of a `configure update` line: the key as typed (aliases
/// resolved) and its values in command line order (empty: remove).
struct UpdateArg {
    /// Key as typed (old Job aliases `newclient` … mapped to `client` …).
    key: String,
    /// Values as typed; an empty list removes the directive.
    values: Vec<String>,
}

/// The directive arguments of `configure update` after the `<type>=<name>`
/// argument, grouped by normalized key (first appearance order).
fn update_args(args: &[Argument], rtype: crate::config_edit::RType) -> Result<Vec<UpdateArg>, String> {
    use crate::config_edit::RType;
    use bareos_config::edit::norm_key;
    let mut out: Vec<UpdateArg> = Vec::new();
    for a in args {
        let Some(value) = a.value.as_deref() else {
            if a.key.eq_ignore_ascii_case("yes") {
                continue;
            }
            return Err(format!("\"{}\" is not a <directive>=<value> argument", a.key));
        };
        let mut key = a.key.as_str();
        if matches!(rtype, RType::Job | RType::JobDefs) {
            // aliases of the earlier implementation
            key = match key.to_ascii_lowercase().as_str() {
                "newclient" => "client",
                "newstorage" => "storage",
                "newpool" => "pool",
                _ => key,
            };
        }
        if key.trim().is_empty() || key.chars().any(char::is_control) {
            return Err("empty directive name or control characters in it".into());
        }
        if !key.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '_') {
            return Err(format!("directive \"{key}\": letters, digits, blanks and underscores only"));
        }
        let norm = norm_key(key);
        if norm == "name" {
            return Err("the Name cannot be changed (renaming is not supported)".into());
        }
        if value.chars().any(|c| c.is_control() || c == '"') {
            return Err(format!("directive \"{key}\": control characters, line ends and '\"' are not allowed in the value"));
        }
        match out.iter_mut().find(|u| norm_key(&u.key) == norm) {
            Some(u) => {
                if value.is_empty() || u.values.is_empty() {
                    return Err(format!("directive \"{key}\": an empty value (remove) cannot be combined with other values"));
                }
                u.values.push(value.to_string());
            }
            None => out.push(UpdateArg {
                key: key.to_string(),
                values: if value.is_empty() { Vec::new() } else { vec![value.to_string()] },
            }),
        }
    }
    if out.is_empty() {
        return Err("no directive given".into());
    }
    Ok(out)
}

/// `configure update <type>=<name> <directive>=<value> … overlay`: the Onesimus
/// directives go to `odir.d/<type>/<name>.conf` in the Onesimus configuration
/// directory ([`DirectorConfig::overlay_dir`]); the configuration is reloaded and
/// the file put back when the Director does not accept it.
fn configure_update_overlay(ctx: &ExecContext, rtype: crate::config_edit::RType, name: &str, args: &[UpdateArg]) -> CommandResult {
    use bareos_config::edit::{norm_key, Edit};
    let allow = |n: &str| crate::acl::access(ctx, rtype.acl(), n);
    if !allow(name) {
        return CommandResult::error(format!("configure update: access to {} \"{name}\" is denied\n", rtype.word()));
    }
    let exists = ctx.config.as_ref().and_then(|c| c.read().ok()).is_some_and(|c| {
        c.raw.iter().any(|r| r.norm_type() == rtype.norm() && r.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
    });
    if !exists {
        return CommandResult::error(format!("configure update: {} \"{name}\" does not exist in the configuration\n", rtype.word()));
    }
    let mut edits = Vec::new();
    for a in args {
        let norm = norm_key(&a.key);
        if !DirectorConfig::EXTENSION_DIRECTIVES.contains(&(rtype.norm(), norm.as_str())) {
            return CommandResult::error(format!(
                "configure update: \"{}\" is not an Onesimus directive of a {}; `overlay` takes only Onesimus directives \
                 (see docs/wiki/osd.md); Bareos directives go to bareos-dir.d without `overlay`\n",
                a.key,
                rtype.word()
            ));
        }
        edits.push(Edit { key: a.key.clone(), values: a.values.iter().map(|v| bareos_config::edit::format_value(rtype.norm(), &a.key, v)).collect::<Result<_, _>>().unwrap_or_default() });
    }
    let summary = args.iter().map(|a| if a.values.is_empty() { format!("{}=", a.key) } else { a.values.iter().map(|v| format!("{}={v}", a.key)).collect::<Vec<_>>().join(", ") }).collect::<Vec<_>>().join(", ");
    let dir = DirectorConfig::overlay_dir();
    let (path, old, new) = match overlay_file_edit(&dir, rtype.norm(), rtype.word(), name, &edits) {
        Ok(t) => t,
        Err(e) => return CommandResult::error(format!("configure update: {e}\n")),
    };
    if old.as_deref() == Some(new.as_str()) {
        return CommandResult::ok(format!("{} '{name}' overlay: no change ({summary})\n", rtype.word()));
    }
    if let Err(e) = std::fs::create_dir_all(path.parent().unwrap_or(&dir)).and_then(|()| std::fs::write(&path, &new)) {
        return CommandResult::error(format!("configure update: cannot write {}: {e}\n", path.display()));
    }
    if let Err(e) = crate::reload::reload_now() {
        // the Director refused the new file: put the old one back
        let _ = match &old {
            Some(old) => std::fs::write(&path, old),
            None => std::fs::remove_file(&path),
        };
        return CommandResult::error(format!("configure update: the Director does not accept the change ({e}); {} restored\n", path.display()));
    }
    CommandResult::ok(format!("{} '{name}' overlay updated: {summary} ({})\n", rtype.word(), path.display()))
}

/// The file of resource `name` in the overlay tree `dir` after `edits`: its path,
/// the old text (`None`: the file did not exist) and the new text.
fn overlay_file_edit(dir: &Path, kind: &str, word: &str, name: &str, edits: &[bareos_config::edit::Edit]) -> Result<(std::path::PathBuf, Option<String>, String), String> {
    if let Some(e) = crate::config_edit::name_error(name) {
        return Err(e);
    }
    let path = dir.join(kind).join(format!("{name}.conf"));
    let old = std::fs::read_to_string(&path).ok();
    let base = old.clone().unwrap_or_else(|| format!("{} {{\n  Name = {name}\n}}\n", capitalize(word)));
    let (new, _) = bareos_config::edit::apply(&base, edits).map_err(|e| format!("{word} \"{name}\": {e}"))?;
    Ok((path, old, new))
}

/// `storage` → `Storage`.
fn capitalize(word: &str) -> String {
    let mut c = word.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// [`configure_update`] with an explicit editor (tests: a throw-away configuration).
pub(crate) fn configure_update_with(ast: &CommandAst, ctx: &ExecContext, editor: &crate::config_edit::ConfigEditor) -> CommandResult {
    use crate::config_edit::{RType, Status};
    use bareos_config::edit::{norm_key, Edit};
    /// Types without a CRUD type: refused with a hint.
    const UNSUPPORTED: [&str; 1] = ["console"];
    let Some(at) = ast.arguments.iter().position(|a| a.value.is_some()) else {
        return CommandResult::error(UPDATE_USAGE.to_string());
    };
    let key = ast.arguments[at].key.to_ascii_lowercase();
    let name = ast.arguments[at].value.clone().unwrap_or_default();
    let Some(rtype) = RType::ALL.into_iter().find(|t| t.norm() == key) else {
        if UNSUPPORTED.contains(&key.as_str()) {
            return CommandResult::error(format!("configure update: {key} resources cannot be changed with configure \
                update yet; change the file by hand and reload\n"));
        }
        return CommandResult::error(format!("configure update: unknown resource type \"{key}\"\n{UPDATE_USAGE}"));
    };
    let overlay = ast.arguments[at + 1..].iter().any(|a| a.value.is_none() && a.key.eq_ignore_ascii_case("overlay"));
    let arguments: Vec<Argument> =
        ast.arguments[at + 1..].iter().filter(|a| !(a.value.is_none() && a.key.eq_ignore_ascii_case("overlay"))).cloned().collect();
    let args = match update_args(&arguments, rtype) {
        Ok(a) => a,
        Err(e) => return CommandResult::error(format!("configure update: {e}\n")),
    };
    if overlay {
        return configure_update_overlay(ctx, rtype, &name, &args);
    }
    // directive names as in the item tables when the directive is added
    let items = crate::config_check::items_of(rtype.norm()).unwrap_or(&[]);
    let mut edits = Vec::new();
    for a in &args {
        let norm = norm_key(&a.key);
        let written = items.iter().find(|i| norm_key(i.name) == norm || i.aliases.iter().any(|x| norm_key(x) == norm))
            .map(|i| i.name.to_string()).unwrap_or_else(|| a.key.clone());
        let mut values = Vec::new();
        for v in &a.values {
            match bareos_config::edit::format_value(rtype.norm(), &a.key, v) {
                Ok(f) => values.push(f),
                Err(e) => return CommandResult::error(format!("configure update: {e}\n")),
            }
        }
        edits.push(Edit { key: written, values });
    }
    let allow = |n: &str| crate::acl::access(ctx, rtype.acl(), n);
    let console = super::config_crud::console_of(ctx);
    let read = match editor.read(rtype, &name, &allow) {
        Ok(o) => o,
        Err(e) => return CommandResult::error(format!("configure update: {e}\n")),
    };
    if read.status != Status::Ok {
        return CommandResult::error(format!("{}\n", read.message));
    }
    let text = match bareos_config::edit::apply(&read.text, &edits) {
        Ok((text, _)) => text,
        Err(e) => return CommandResult::error(format!("configure update: {} \"{name}\": {e}\n", rtype.word())),
    };
    let summary = args.iter().flat_map(|a| {
        let masked = norm_key(&a.key) == "password";
        if a.values.is_empty() {
            return vec![format!("{}=", a.key)];
        }
        a.values.iter().map(|v| format!("{}={}", a.key, if masked { "***" } else { v })).collect()
    }).collect::<Vec<_>>().join(", ");
    if text == read.text {
        return CommandResult::ok(format!("{} '{name}': no change ({summary})\n", rtype.word()));
    }
    let o = match editor.update(rtype, &name, &read.sha256, &text, false, &console, &allow) {
        Ok(o) => o,
        Err(e) => return CommandResult::error(format!("configure update: {e}\n")),
    };
    match o.status {
        Status::Ok => CommandResult::ok(format!("{} '{name}' updated: {summary}\n", rtype.word())),
        _ => {
            let mut text = format!("{}\n", o.message);
            if let Some(p) = o.problem.as_ref().filter(|p| p.text.trim_end() != o.message) {
                text.push_str(&p.text);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            CommandResult::error(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table of unquoted directives in bareos-config (`configure update`, the MCP tool)
    /// lists exactly the time, size and speed items of the Director's tables (C-102).
    #[test]
    fn unquoted_directives_match_the_item_tables() {
        use bareos_config::edit::{norm_key, UNQUOTED_DIRECTIVES};
        let mut expected = Vec::new();
        for (rtype, items) in [("director", tables::DIR_ITEMS), ("client", tables::CLIENT_ITEMS),
            ("storage", tables::STORAGE_ITEMS), ("catalog", tables::CATALOG_ITEMS), ("job", tables::JOB_ITEMS),
            ("fileset", tables::FILESET_ITEMS), ("schedule", tables::SCHEDULE_ITEMS), ("pool", tables::POOL_ITEMS),
            ("counter", tables::COUNTER_ITEMS), ("messages", tables::MESSAGES_ITEMS), ("profile", tables::PROFILE_ITEMS),
            ("console", tables::CONSOLE_ITEMS), ("user", tables::USER_ITEMS)] {
            for item in items.iter().filter(|i| matches!(i.ty, Ty::Time | Ty::Size32 | Ty::Size64 | Ty::Speed)) {
                for name in std::iter::once(&item.name).chain(item.aliases) {
                    expected.push((rtype.to_string(), norm_key(name)));
                }
            }
        }
        let mut table: Vec<(String, String)> = UNQUOTED_DIRECTIVES.iter().map(|(t, k)| (t.to_string(), k.to_string())).collect();
        expected.sort();
        table.sort();
        assert_eq!(table, expected);
    }

    #[test]
    fn the_overlay_file_is_created_changed_and_cleared() {
        use bareos_config::edit::Edit;
        let dir = std::env::temp_dir().join(format!("onesimus-overlay-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let set = |v: &[&str]| vec![Edit { key: "Health Check Interval".into(), values: v.iter().map(|s| s.to_string()).collect() }];
        // a new file with the resource header and the directive
        let (path, old, new) = overlay_file_edit(&dir, "storage", "storage", "H340", &set(&["1 hour"])).unwrap();
        assert_eq!(path, dir.join("storage").join("H340.conf"));
        assert_eq!(old, None);
        assert!(new.starts_with("Storage {\n  Name = H340\n") && new.contains("Health Check Interval = 1 hour"), "{new}");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &new).unwrap();
        // changed
        let (_, old, new2) = overlay_file_edit(&dir, "storage", "storage", "H340", &set(&["2 hours"])).unwrap();
        assert_eq!(old.as_deref(), Some(new.as_str()));
        assert!(new2.contains("Health Check Interval = 2 hours") && !new2.contains("1 hour"), "{new2}");
        // cleared: an empty value removes the directive
        let (_, _, cleared) = overlay_file_edit(&dir, "storage", "storage", "H340", &set(&[])).unwrap();
        assert!(!cleared.contains("Health Check Interval"), "{cleared}");
        // a name that cannot be a file name is refused
        assert!(overlay_file_edit(&dir, "storage", "storage", "../x", &set(&["1 hour"])).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_onesimus_directives_are_allowed_in_the_overlay() {
        assert!(DirectorConfig::EXTENSION_DIRECTIVES.contains(&("storage", "healthcheckinterval")));
        assert!(DirectorConfig::EXTENSION_DIRECTIVES.contains(&("client", "grpcenable")));
        assert!(!DirectorConfig::EXTENSION_DIRECTIVES.contains(&("storage", "password")));
        assert!(!DirectorConfig::EXTENSION_DIRECTIVES.contains(&("job", "healthcheckinterval")));
    }

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

    /// F-1: `configure add profile|user` is refused before anything is
    /// parsed or written, whatever the console's CommandACL; `configure add
    /// console` with ACL lines or without a profile too.
    #[test]
    fn configure_add_of_acl_resources_is_refused() {
        use std::sync::{Arc, RwLock};
        let (cfg, layout) = setup("refused");
        let ctx = ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(cfg))), session_mgr: None,
            catalog: None, api_mode: crate::command_executor::ApiMode::Human, bus_rx: None, job_manager: None };
        for line in ["configure add console name=x password=p commandacl=*all*", "configure add profile name=p commandacl=*all*",
                     "configure add user name=u commandacl=*all*", "configure add Console name=y password=p"] {
            let r = configure_add(&crate::session::parse_cli_command(line), &ctx, Some(&layout));
            assert!(!r.success && r.error_message.contains("refused"), "{line}: {}", r.error_message);
        }
        assert_eq!(std::fs::read_dir(layout.include_dir.join("console")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(layout.include_dir.join("profile")).unwrap().count(), 0);
        assert!(ctx.config.as_ref().unwrap().read().unwrap().find_console("x").is_none());
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

    /// Configuration of the `configure add console` tests: an unrestricted
    /// creator, a creator like PDC's MCP console, a creator narrowed by the
    /// catalog tier and profiles of different reach.
    const CONSOLES: &str = r#"
Director { Name = bareos-dir; Password = "dirpw" }
Client { Name = bareos-fd; Address = localhost; Password = "fdpw" }
Profile { Name = everything; CommandAcl = *all*; JobAcl = *all*; ClientAcl = *all*; StorageAcl = *all*
  ScheduleAcl = *all*; PoolAcl = *all*; FileSetAcl = *all*; CatalogAcl = *all*; WhereAcl = *all*; PluginOptionsAcl = *all* }
Profile { Name = operator; CommandAcl = status, show, list, llist, messages; JobAcl = backup-a, !secret
  ClientAcl = bareos-fd; PoolAcl = Full; WhereAcl = /tmp/bareos-restores }
Profile { Name = jobs-b; JobAcl = backup-b }
Profile { Name = jobs-regex; JobAcl = "backup-.*" }
Profile { Name = where-other; WhereAcl = /srv }
Profile { Name = acl-admin; CommandAcl = .acl, configure }
Profile { Name = narrowing; JobAcl = !backup-a }
Console { Name = admin; Password = "adminpw"; CommandAcl = *all*; JobAcl = *all*; ClientAcl = *all*; StorageAcl = *all*
  ScheduleAcl = *all*; PoolAcl = *all*; FileSetAcl = *all*; CatalogAcl = *all*; WhereAcl = *all*; PluginOptionsAcl = *all* }
Console { Name = pdc-ops; Password = "mcppw"; CommandAcl = "!.acl", "!.profile", "configure", ".config"; Profile = operator }
Console { Name = narrowed; Password = "npw"; CommandAcl = configure; Profile = everything }
"#;

    /// Secret of the new consoles; must never appear in a reply.
    const SECRET: &str = "S3cr3t-Pw";

    /// A context authenticated as `console` on [`CONSOLES`] with a fresh
    /// include directory.
    fn console_ctx(tag: &str, console: &str) -> (ExecContext, Layout) {
        use std::sync::{Arc, RwLock};
        let base = std::env::temp_dir().join(format!("onesimus-configure-console-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for t in ["console", "profile", "user"] {
            std::fs::create_dir_all(base.join("bareos-dir.d").join(t)).unwrap();
        }
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session(console);
        manager.authenticate_session(id);
        let config = DirectorConfig::from_str(CONSOLES).unwrap();
        config.catalog_acl.set_for_test(crate::catalog_acl::LoadState::Loaded, &[
            ("narrowed", crate::acl::Kind::Job, &["backup-a"]),
        ]);
        let ctx = ExecContext { session_id: id, config: Some(Arc::new(RwLock::new(config))), session_mgr: Some(manager),
            catalog: None, api_mode: crate::command_executor::ApiMode::Json, bus_rx: None, job_manager: None };
        (ctx, layout(&base))
    }

    /// Run the console line `line` through `configure add` and assert that
    /// [`SECRET`] appears nowhere in the reply (text, errors, JSON).
    fn add_line(ctx: &ExecContext, layout: &Layout, line: &str) -> CommandResult {
        let r = configure_add(&crate::session::parse_cli_command(line), ctx, Some(layout));
        let json = r.json.as_ref().map(Value::to_string).unwrap_or_default();
        for text in [&r.text_output, &r.error_message, &json] {
            assert!(!text.contains(SECRET), "{line}: password in the reply: {text}");
        }
        r
    }

    /// The new console's effective ACL equals the file ACL of `profiles`
    /// (first match over their lists) for a sample of names of every type.
    fn assert_effective_is_profiles(cfg: &DirectorConfig, console: &str, profiles: &[&str]) {
        use crate::acl::{allowed, first_match, Kind};
        let names = ["status", "delete", "configure", ".acl", "backup-a", "backup-b", "secret", "bareos-fd",
            "Full", "/tmp/bareos-restores", "/srv", "other"];
        for kind in Kind::ALL {
            let entries: Vec<&str> = profiles.iter()
                .filter_map(|p| cfg.raw.iter().find(|r| r.norm_type() == "profile" && r.name() == Some(*p)))
                .flat_map(|p| p.directives.iter().filter(|d| d.norm_key() == kind.directive().to_ascii_lowercase())
                    .flat_map(|d| d.values()))
                .collect();
            for name in names {
                let valid = matches!(kind, Kind::Where | Kind::PluginOptions) || crate::acl::valid_name(name);
                let expected = valid && first_match(entries.iter().copied(), name) == Some(true);
                assert_eq!(allowed(cfg, console, kind, name), expected, "{console} {kind:?} {name}");
            }
        }
    }

    /// F-1 amendment: an unrestricted creator may add a console with a
    /// profile granting everything; the file is written, the console is in
    /// the running configuration with exactly the profile's rights, and the
    /// password is masked in text and JSON.
    #[test]
    fn unrestricted_creator_adds_console_with_all_profile() {
        let (ctx, layout) = console_ctx("unrestricted", "admin");
        let r = add_line(&ctx, &layout, &format!("configure add console name=newc password={SECRET} Profile=everything description=GUI"));
        assert!(r.success, "{}", r.error_message);
        let path = layout.include_dir.join("console/newc.conf");
        let content = format!("Console {{\n  Name = \"newc\"\n  Password = \"{SECRET}\"\n  Description = \"GUI\"\n  Profile = everything\n}}\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        let masked = "Console {\n  Name = \"newc\"\n  Password = \"***\"\n  Description = \"GUI\"\n  Profile = everything\n}\n";
        assert_eq!(r.text_output, format!("Created resource config file \"{}\":\n{masked}", path.display()));
        let json = r.json.unwrap();
        assert_eq!(json["configure"]["add"]["resource"], "Console");
        assert_eq!(json["configure"]["add"]["name"], "newc");
        assert_eq!(json["configure"]["add"]["filename"], path.display().to_string());
        assert_eq!(json["configure"]["add"]["content"], masked);
        let cfg = ctx.config.as_ref().unwrap().read().unwrap();
        assert!(cfg.find_console("newc").is_some());
        assert_effective_is_profiles(&cfg, "newc", &["everything"]);
    }

    /// Anti-escalation: a creator like PDC's MCP console may hand out its
    /// own profile, but no profile with `*all*`, a regex or a name beyond
    /// its rights; several profiles are all checked.
    #[test]
    fn restricted_creator_cannot_escalate() {
        let (ctx, layout) = console_ctx("mcp", "pdc-ops");
        for (line, why) in [
            ("profile=everything", "the profiles grant CommandAcl"),
            ("profile=operator profile=everything", "the profiles grant CommandAcl"),
            ("profile=operator,jobs-b", "grant JobAcl \"backup-b\", which Console \"pdc-ops\" may not use"),
            ("profile=jobs-regex", "grant JobAcl names beyond a fixed list"),
            ("profile=where-other", "grant WhereAcl \"/srv\""),
            ("profile=acl-admin", "grant CommandAcl \".acl\""),
        ] {
            let r = add_line(&ctx, &layout, &format!("configure add console name=x password={SECRET} {line}"));
            assert!(!r.success && r.error_message.contains(why), "{line}: {}", r.error_message);
            assert_eq!(r.json.unwrap(), json!({ "configure": {} }));
        }
        assert_eq!(std::fs::read_dir(layout.include_dir.join("console")).unwrap().count(), 0);
        let r = add_line(&ctx, &layout, &format!("configure add console name=op2 password={SECRET} profile=operator profile=narrowing"));
        assert!(r.success, "{}", r.error_message);
        let cfg = ctx.config.as_ref().unwrap().read().unwrap();
        assert_effective_is_profiles(&cfg, "op2", &["operator", "narrowing"]);
        assert!(!crate::acl::allowed(&cfg, "op2", crate::acl::Kind::Command, "configure"));
    }

    /// Anti-escalation with the catalog tier: catalog entries narrowing a
    /// type make the creator restricted for it, so a profile naming
    /// something the catalog forbids (or a pattern) is refused; profiles
    /// granting `.acl` are refused while the creator has catalog entries;
    /// a locked creator (F-4) creates nothing.
    #[test]
    fn catalog_tier_limits_the_creator() {
        let (ctx, layout) = console_ctx("catalog", "narrowed");
        for (line, why) in [
            ("profile=jobs-b", "grant JobAcl \"backup-b\""),
            ("profile=everything", "grant JobAcl names beyond"),
            ("profile=acl-admin", "the profiles grant .acl"),
        ] {
            let r = add_line(&ctx, &layout, &format!("configure add console name=x password={SECRET} {line}"));
            assert!(!r.success && r.error_message.contains(why), "{line}: {}", r.error_message);
        }
        let r = add_line(&ctx, &layout, &format!("configure add console name=ops password={SECRET} profile=operator"));
        assert!(r.success, "{}", r.error_message);
        let config = ctx.config.as_ref().unwrap();
        config.read().unwrap().catalog_acl.set_for_test(crate::catalog_acl::LoadState::Failed("down".into()), &[
            ("narrowed", crate::acl::Kind::Job, &["backup-a"]),
        ]);
        let r = add_line(&ctx, &layout, &format!("configure add console name=y password={SECRET} profile=operator"));
        assert!(!r.success && r.error_message.contains("locked"), "{}", r.error_message);
        // Without catalog entries the same creator is unrestricted again.
        config.read().unwrap().catalog_acl.set_for_test(crate::catalog_acl::LoadState::Loaded, &[]);
        let r = add_line(&ctx, &layout, &format!("configure add console name=z password={SECRET} profile=acl-admin"));
        assert!(r.success, "{}", r.error_message);
    }

    /// Refusals of the narrow form; Profile and User stay refused.
    #[test]
    fn console_form_refusals() {
        let (ctx, layout) = console_ctx("refusals", "admin");
        for (line, why) in [
            (format!("configure add console name=x password={SECRET} profile=operator jobacl=*all*"), "a new console has no own ACL lines"),
            (format!("configure add console name=x password={SECRET} profile=operator Command_Acl=*all*"), "no own ACL lines"),
            (format!("configure add console name=x password={SECRET} profile=operator tlsenable=no"), "only name, password, profile and description"),
            (format!("configure add console name=x password={SECRET} profile=nosuch"), "unknown Profile \"nosuch\""),
            (format!("configure add console name=x password={SECRET}"), "at least one profile"),
            ("configure add console name=x profile=operator".to_string(), "missing password"),
            (format!("configure add console name=admin password={SECRET} profile=operator"), "already exists"),
            (format!("configure add console name=*UserAgent* password={SECRET} profile=operator"), "reserved"),
            (format!("configure add console name=a! password={SECRET} profile=operator"), "invalid name"),
            (format!("configure add console=x password={SECRET} profile=operator"), "name=<name>"),
            (format!("configure add console name=x name=y password={SECRET} profile=operator"), "given twice"),
            (format!("configure add console name=x password={SECRET} profile=operator description=@x"), "not allowed"),
            ("configure add console name=x password=[md5]S3cr3t-Pw0000000000000000000000 profile=operator".to_string(), "password is not valid"),
            ("configure add profile name=p commandacl=*all*".to_string(), "Profile and User resources"),
            ("configure add user name=u commandacl=*all*".to_string(), "Profile and User resources"),
        ] {
            let r = add_line(&ctx, &layout, &line);
            assert!(!r.success && r.error_message.contains("refused") && r.error_message.contains(why), "{line}: {}", r.error_message);
        }
        assert_eq!(std::fs::read_dir(layout.include_dir.join("console")).unwrap().count(), 0);
        assert_eq!(std::fs::read_dir(layout.include_dir.join("profile")).unwrap().count(), 0);
        let cfg = ctx.config.as_ref().unwrap().read().unwrap();
        assert!(cfg.find_console("x").is_none());
        // Quotes and backslashes could end the quoted value in the file.
        for bad in ["a\"b", "a\\b", "a\nb"] {
            let args = [Argument { key: "console".into(), value: None },
                Argument { key: "name".into(), value: Some("x".into()) },
                Argument { key: "password".into(), value: Some(bad.into()) },
                Argument { key: "profile".into(), value: Some("operator".into()) }];
            assert!(console_request(&cfg, &args).is_err_and(|e| e.contains("not allowed")), "{bad:?}");
        }
    }
}
