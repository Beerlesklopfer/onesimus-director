//! `.acl list|set|delete|clear|copy` — Onesimus extension (no Bareos
//! counterpart; the original answers ".acl: is an invalid command.").
//!
//! Shows and changes the ACL directives of Console resources:
//!
//! - `.acl [list] [console=<name>]`: the effective ACLs of a console, i.e. for
//!   every ACL type the console's own entries followed by the entries of its
//!   Profiles in evaluation order (crate::acl / ua_acl.cc: the first matching
//!   entry decides, console list before the profiles). Without `console=` the
//!   own console, or all consoles for consoles allowed to `configure`.
//! - `.acl set console=<name> <type>acl=<e1>,<e2> ...`: replace the console's
//!   own entries of each given type.
//! - `.acl delete console=<name> <type>acl=<e1>,...`: remove these entries;
//!   `<type>acl` without a value removes the whole type.
//! - `.acl clear console=<name>`: remove all ACL directives of the console.
//! - `.acl copy from=<console> to=<console>`: give `to` the ACL and Profile
//!   directives of `from` (same effective ACLs).
//!
//! Changes need the CommandACL `configure` (like `configure add`), change the
//! running configuration and rewrite the console's resource file the way
//! `configure add` writes resources (`<name>.conf.tmp`, then rename). A
//! console defined in a file together with other resources is not changed.

use std::path::Path;

use bareos_config::raw::{ConfResource, Directive, Tok};
use serde_json::{json, Map, Value};

use crate::acl::Kind;
use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::director_config::DirectorConfig;

/// ACL directives of a Console resource in the order of the directive
/// table (dird_conf.cc ConsoleResource items).
const ACL_TYPES: &[&str] = &[
    "JobAcl", "ClientAcl", "StorageAcl", "ScheduleAcl", "PoolAcl", "CommandAcl",
    "FileSetAcl", "CatalogAcl", "WhereAcl", "PluginOptionsAcl",
];

const USAGE: &str = ".acl: usage:\n  .acl list [console=<name>]\n  \
    .acl set console=<name> <type>acl=<entry1>,<entry2> ...\n  \
    .acl delete console=<name> <type>acl[=<entry1>,...] ...\n  \
    .acl clear console=<name>\n  \
    .acl copy from=<console> to=<console>\n\
    <type>: job, client, storage, schedule, pool, command, fileset, catalog, where, pluginoptions\n";

fn arg<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref())
}

/// `jobacl` → `JobAcl`
fn acl_type(key: &str) -> Option<&'static str> {
    ACL_TYPES.iter().copied().find(|t| t.eq_ignore_ascii_case(key))
}

fn norm(key: &str) -> String {
    bareos_config::raw::normalize_key(key)
}

fn split_entries(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|e| !e.is_empty()).map(str::to_string).collect()
}

/// Name of the console this session authenticated as.
fn own_console(ctx: &ExecContext) -> Option<String> {
    match &ctx.session_mgr {
        Some(manager) => manager.authenticated_console(ctx.session_id),
        None => Some("*UserAgent*".into()),
    }
}

fn may_configure(ctx: &ExecContext) -> bool {
    crate::acl::access(ctx, Kind::Command, "configure")
}

fn find<'a>(config: &'a DirectorConfig, rtype: &str, name: &str) -> Option<&'a ConfResource> {
    config.raw.iter().find(|r| r.norm_type() == rtype && r.name() == Some(name))
}

fn entries<'a>(resource: &'a ConfResource, acl: &str) -> Vec<&'a str> {
    let key = norm(acl);
    resource.directives.iter().filter(|d| d.norm_key() == key).flat_map(|d| d.values()).collect()
}

fn profiles(resource: &ConfResource) -> Vec<&str> {
    resource.directives.iter().filter(|d| d.norm_key() == "profile").flat_map(|d| d.values()).collect()
}

pub fn cmd_dot_acl(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let action = ast.arguments.first()
        .filter(|a| a.value.is_none())
        .map(|a| a.key.to_ascii_lowercase());
    match action.as_deref() {
        None | Some("list") => acl_list(ast, ctx),
        Some(action @ ("set" | "delete" | "clear" | "copy")) => acl_change(action, ast, ctx),
        Some(_) => CommandResult::error(USAGE.to_string()),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// Effective ACLs of one console as text and JSON.
fn describe(config: &DirectorConfig, console: &ConfResource) -> (String, Value) {
    let name = console.name().unwrap_or("");
    let profile_names = profiles(console);
    let mut text = format!("Console \"{name}\":\n");
    let mut object = Map::new();
    object.insert("console".into(), name.into());
    if profile_names.is_empty() {
        text.push_str("  Profile = (none)\n");
    } else {
        let shown: Vec<String> = profile_names.iter().map(|p| match find(config, "profile", p) {
            Some(_) => p.to_string(),
            None => format!("{p} (not defined)"),
        }).collect();
        text.push_str(&format!("  Profile = {}\n", shown.join(", ")));
    }
    object.insert("profiles".into(), profile_names.iter().map(|p| Value::from(*p)).collect());
    for acl in ACL_TYPES {
        let mut shown = Vec::new();
        let mut list = Vec::new();
        for entry in entries(console, acl) {
            shown.push(entry.to_string());
            list.push(json!({ "value": entry, "source": "Console" }));
        }
        for profile in &profile_names {
            let Some(resource) = find(config, "profile", profile) else { continue };
            for entry in entries(resource, acl) {
                shown.push(format!("{entry} (Profile {profile})"));
                list.push(json!({ "value": entry, "source": format!("Profile {profile}") }));
            }
        }
        let shown = if shown.is_empty() { "(none)".to_string() } else { shown.join(", ") };
        text.push_str(&format!("  {acl} = {shown}\n"));
        object.insert(acl.to_ascii_lowercase(), Value::Array(list));
    }
    (text, Value::Object(object))
}

fn acl_list(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some(shared) = &ctx.config else { return CommandResult::error("No config loaded\n".into()) };
    let Ok(config) = shared.read() else { return CommandResult::error("Config error\n".into()) };
    let own = own_console(ctx);
    let wanted = arg(ast, "console");
    let all = may_configure(ctx);
    // Other consoles' ACLs only for consoles allowed to configure.
    if let Some(name) = wanted {
        if !all && own.as_deref() != Some(name) {
            return CommandResult::error(format!(
                ".acl: permission denied, listing the ACLs of Console \"{name}\" requires the CommandACL configure.\n"));
        }
    }
    let consoles: Vec<&ConfResource> = config.raw.iter()
        .filter(|r| r.norm_type() == "console")
        .filter(|r| match wanted {
            Some(name) => r.name() == Some(name),
            None => all || r.name() == own.as_deref(),
        })
        .collect();
    if consoles.is_empty() {
        return match wanted {
            Some(name) => CommandResult::error(format!("Console \"{name}\" not found.\n")),
            // The default console (*UserAgent*) has no ACLs.
            None => CommandResult::ok("No Console resources.\n".into()).with_json(json!({ "acl": [] })),
        };
    }
    let mut text = String::new();
    let mut list = Vec::new();
    for console in consoles {
        let (t, j) = describe(&config, console);
        text.push_str(&t);
        list.push(j);
    }
    CommandResult::ok(text).with_json(json!({ "acl": list }))
}

// ---------------------------------------------------------------------------
// set / delete / clear / copy
// ---------------------------------------------------------------------------

/// Replace all `acl` directives by one directive with `values` at the
/// position of the first one (or at the end).
fn replace_entries(resource: &mut ConfResource, acl: &str, values: &[String]) {
    let key = norm(acl);
    let position = resource.directives.iter().position(|d| d.norm_key() == key);
    resource.directives.retain(|d| d.norm_key() != key);
    if values.is_empty() {
        return;
    }
    let mut tokens = Vec::new();
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            tokens.push(Tok::Comma);
        }
        tokens.push(Tok::Quoted(value.clone()));
    }
    let directive = Directive { key: acl.to_string(), tokens, block: None };
    let at = position.unwrap_or(resource.directives.len()).min(resource.directives.len());
    resource.directives.insert(at, directive);
}

/// Apply the change to a copy of the console resource.
fn change(action: &str, ast: &CommandAst, config: &DirectorConfig, console: &ConfResource)
    -> Result<ConfResource, String> {
    let mut changed = console.clone();
    let name = console.name().unwrap_or("");
    let typed: Vec<(&'static str, Option<&str>)> = ast.arguments.iter()
        .filter_map(|a| acl_type(&a.key).map(|t| (t, a.value.as_deref())))
        .collect();
    match action {
        "set" => {
            if typed.is_empty() {
                return Err(".acl set: specify at least one <type>acl=<entries>.\n".into());
            }
            for (acl, value) in typed {
                let values = split_entries(value.unwrap_or(""));
                if values.is_empty() {
                    return Err(format!(".acl set: {acl} needs at least one entry (use .acl delete to remove it).\n"));
                }
                replace_entries(&mut changed, acl, &values);
            }
        }
        "delete" => {
            if typed.is_empty() {
                return Err(".acl delete: specify at least one <type>acl[=<entries>].\n".into());
            }
            for (acl, value) in typed {
                let current: Vec<String> = entries(&changed, acl).into_iter().map(str::to_string).collect();
                let remove = match value {
                    None => current.clone(),
                    Some(value) => split_entries(value),
                };
                if current.is_empty() {
                    return Err(format!("Console \"{name}\" has no {acl} entries.\n"));
                }
                if let Some(missing) = remove.iter().find(|e| !current.contains(e)) {
                    return Err(format!("{acl} entry \"{missing}\" not found in Console \"{name}\".\n"));
                }
                let keep: Vec<String> = current.into_iter().filter(|e| !remove.contains(e)).collect();
                replace_entries(&mut changed, acl, &keep);
            }
        }
        "clear" => {
            for acl in ACL_TYPES {
                replace_entries(&mut changed, acl, &[]);
            }
        }
        "copy" => {
            let from = arg(ast, "from").unwrap_or("");
            let source = find(config, "console", from)
                .ok_or_else(|| format!("Console \"{from}\" not found.\n"))?;
            for acl in ACL_TYPES.iter().copied().chain(["Profile"]) {
                let values: Vec<String> = if acl == "Profile" {
                    profiles(source).into_iter().map(str::to_string).collect()
                } else {
                    entries(source, acl).into_iter().map(str::to_string).collect()
                };
                replace_entries(&mut changed, acl, &values);
            }
        }
        _ => unreachable!(),
    }
    Ok(changed)
}

/// Text of a raw token list as the lexer reads it back.
fn tokens_text(tokens: &[Tok], depth: usize) -> String {
    let mut out = String::new();
    for token in tokens {
        match token {
            Tok::Comma => out.push(','),
            Tok::Eq => out.push_str(" ="),
            Tok::Word(word) => {
                if !out.is_empty() { out.push(' '); }
                out.push_str(word);
            }
            Tok::Quoted(text) => {
                if !out.is_empty() { out.push(' '); }
                out.push('"');
                for c in text.chars() {
                    if c == '"' || c == '\\' { out.push('\\'); }
                    out.push(c);
                }
                out.push('"');
            }
            Tok::Block(block) => {
                if !out.is_empty() { out.push(' '); }
                out.push_str("{\n");
                out.push_str(&directives_text(block, depth + 1));
                out.push_str(&"  ".repeat(depth));
                out.push('}');
            }
        }
    }
    out
}

fn directives_text(directives: &[Directive], depth: usize) -> String {
    let indent = "  ".repeat(depth);
    let mut out = String::new();
    for directive in directives {
        out.push_str(&indent);
        out.push_str(&directive.key);
        if !directive.tokens.is_empty() {
            out.push_str(" = ");
            out.push_str(&tokens_text(&directive.tokens, depth));
        }
        if let Some(block) = &directive.block {
            out.push_str(" {\n");
            out.push_str(&directives_text(block, depth + 1));
            out.push_str(&indent);
            out.push('}');
        }
        out.push('\n');
    }
    out
}

/// Resource text as written to its own file.
pub(crate) fn resource_text(resource: &ConfResource) -> String {
    format!("{} {{\n{}}}\n", resource.rtype, directives_text(&resource.directives, 1))
}

/// Rewrite the console's own resource file like ConfigureAddResource writes
/// one (temporary file, then rename; mode of the old file).
fn persist(path: &Path, name: &str, text: &str) -> Result<(), String> {
    let old = std::fs::read_to_string(path)
        .map_err(|e| format!("Could not read resource config file \"{}\": {e}\n", path.display()))?;
    let resources = bareos_config::raw::parse_str(&old, Some(path)).unwrap_or_default();
    if resources.len() != 1 || resources[0].name() != Some(name) {
        return Err(format!(
            "Console \"{name}\" is defined in \"{}\" together with other resources; \
             .acl only changes consoles that have their own resource file.\n", path.display()));
    }
    let tmp = std::path::PathBuf::from(format!("{}.tmp", path.display()));
    if tmp.exists() {
        return Err(format!("Temporary resource config file \"{}\" already exists.\n", tmp.display()));
    }
    let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err("failed to write config resource file\n".into());
    }
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("failed to update config file \"{}\"\n", path.display()));
    }
    Ok(())
}

fn acl_change(action: &str, ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if !may_configure(ctx) {
        return CommandResult::error(format!(
            ".acl {action}: permission denied, changing ACLs requires the CommandACL configure.\n"));
    }
    let target = if action == "copy" { arg(ast, "to") } else { arg(ast, "console") };
    let Some(target) = target.filter(|t| !t.is_empty()) else {
        return CommandResult::error(if action == "copy" {
            ".acl copy: from=<console> to=<console> required.\n".to_string()
        } else {
            format!(".acl {action}: console=<name> required.\n")
        });
    };
    if action == "copy" && arg(ast, "from").is_none_or(str::is_empty) {
        return CommandResult::error(".acl copy: from=<console> to=<console> required.\n".into());
    }
    let Some(shared) = &ctx.config else { return CommandResult::error("No config loaded\n".into()) };
    let Ok(mut config) = shared.write() else { return CommandResult::error("Config error\n".into()) };
    let Some(index) = config.raw.iter()
        .position(|r| r.norm_type() == "console" && r.name() == Some(target)) else {
        return CommandResult::error(format!("Console \"{target}\" not found.\n"));
    };
    let changed = match change(action, ast, &config, &config.raw[index]) {
        Ok(changed) => changed,
        Err(e) => return CommandResult::error(e),
    };
    let content = resource_text(&changed);
    let filename = config.raw[index].source.clone();
    if let Some(path) = &filename {
        if let Err(e) = persist(path, target, &content) {
            return CommandResult::error(e);
        }
    }
    config.raw[index] = changed;
    let mut text = match &filename {
        Some(path) => format!("Updated resource config file \"{}\":\n{content}", path.display()),
        None => format!("Updated Console \"{target}\" in the running configuration:\n{content}"),
    };
    let (effective, list) = describe(&config, &config.raw[index]);
    text.push_str(&effective);
    let filename = filename.map(|p| p.display().to_string());
    CommandResult::ok(text).with_json(json!({ "acl": {
        "action": action, "console": target, "filename": filename, "content": content, "effective": list,
    }}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_executor::ApiMode;
    use std::sync::{Arc, RwLock};

    const CONFIG: &str = r#"
Director { Name = dir; Password = secret }
Profile { Name = operator; CommandAcl = status, list; JobAcl = *all* }
Console { Name = admin; Password = pw; CommandAcl = *all*; JobAcl = *all* }
Console { Name = op; Password = "pw2"; CommandAcl = !delete; ClientAcl = fd1; Profile = operator }
Console { Name = viewer; Password = pw3; CommandAcl = .acl, status }
"#;

    /// Context of a session authenticated as `console`; every console gets
    /// its own resource file below a fresh directory.
    fn setup(tag: &str, console: &str) -> (ExecContext, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("onesimus-acl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = DirectorConfig::from_str(CONFIG).unwrap();
        for resource in config.raw.iter_mut().filter(|r| r.norm_type() == "console") {
            let path = dir.join(format!("{}.conf", resource.name().unwrap()));
            std::fs::write(&path, resource_text(resource)).unwrap();
            resource.source = Some(path);
        }
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session(console);
        manager.authenticate_session(id);
        let ctx = ExecContext {
            session_id: id,
            config: Some(Arc::new(RwLock::new(config))),
            session_mgr: Some(manager),
            catalog: None,
            api_mode: ApiMode::Human,
            bus_rx: None,
            job_manager: None,
        };
        (ctx, dir)
    }

    fn run(ctx: &ExecContext, line: &str) -> CommandResult {
        cmd_dot_acl(&crate::session::parse_cli_command(line), ctx)
    }

    #[test]
    fn list_shows_console_and_profile_entries_in_evaluation_order() {
        let (ctx, dir) = setup("list", "admin");
        let result = run(&ctx, ".acl list console=op");
        assert!(result.success, "{}", result.error_message);
        assert_eq!(result.text_output, "Console \"op\":\n  Profile = operator\n  \
            JobAcl = *all* (Profile operator)\n  ClientAcl = fd1\n  StorageAcl = (none)\n  \
            ScheduleAcl = (none)\n  PoolAcl = (none)\n  \
            CommandAcl = !delete, status (Profile operator), list (Profile operator)\n  \
            FileSetAcl = (none)\n  CatalogAcl = (none)\n  WhereAcl = (none)\n  PluginOptionsAcl = (none)\n");
        let json = result.json.unwrap();
        assert_eq!(json["acl"][0]["console"], "op");
        assert_eq!(json["acl"][0]["commandacl"][1], json!({ "value": "status", "source": "Profile operator" }));
        // Without console= a configure-allowed console sees all consoles.
        let all = run(&ctx, ".acl");
        assert_eq!(all.json.unwrap()["acl"].as_array().unwrap().len(), 3);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn restricted_console_lists_only_itself_and_cannot_change() {
        let (ctx, dir) = setup("viewer", "viewer");
        let own = run(&ctx, ".acl list");
        assert!(own.text_output.starts_with("Console \"viewer\":\n"));
        assert_eq!(own.json.unwrap()["acl"].as_array().unwrap().len(), 1);
        let other = run(&ctx, ".acl list console=admin");
        assert!(!other.success);
        assert!(other.error_message.contains("requires the CommandACL configure"));
        let set = run(&ctx, ".acl set console=viewer commandacl=*all*");
        assert!(!set.success);
        assert_eq!(set.error_message, ".acl set: permission denied, changing ACLs requires the CommandACL configure.\n");
        assert!(std::fs::read_to_string(dir.join("viewer.conf")).unwrap().contains("CommandAcl = .acl, status\n"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn set_replaces_entries_in_running_config_and_file() {
        let (ctx, dir) = setup("set", "admin");
        let result = run(&ctx, ".acl set console=op jobacl=backup-a,backup-b clientacl=fd2");
        assert!(result.success, "{}", result.error_message);
        let path = dir.join("op.conf");
        let content = "Console {\n  Name = op\n  Password = \"pw2\"\n  CommandAcl = !delete\n  \
            ClientAcl = \"fd2\"\n  Profile = operator\n  JobAcl = \"backup-a\", \"backup-b\"\n}\n";
        assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        assert!(result.text_output.starts_with(&format!("Updated resource config file \"{}\":\n{content}", path.display())));
        // The running configuration decides with the new lists.
        let config = ctx.config.as_ref().unwrap().read().unwrap();
        assert!(crate::acl::allowed(&config, "op", Kind::Job, "backup-b"));
        assert!(crate::acl::allowed(&config, "op", Kind::Client, "fd2"));
        assert!(!crate::acl::allowed(&config, "op", Kind::Client, "fd1"));
        // The written file parses back to the same resource.
        let reread = bareos_config::raw::parse_str(&std::fs::read_to_string(&path).unwrap(), Some(&path)).unwrap();
        let running = config.raw.iter().find(|r| r.name() == Some("op")).unwrap();
        assert_eq!(reread[0].directives, running.directives);
        drop(config);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn delete_entries_or_whole_type() {
        let (ctx, dir) = setup("delete", "admin");
        assert!(run(&ctx, ".acl set console=op jobacl=a,b,c").success);
        let result = run(&ctx, ".acl delete console=op jobacl=b");
        assert!(result.success, "{}", result.error_message);
        assert!(result.text_output.contains("  JobAcl = \"a\", \"c\"\n"));
        let missing = run(&ctx, ".acl delete console=op jobacl=zz");
        assert_eq!(missing.error_message, "JobAcl entry \"zz\" not found in Console \"op\".\n");
        assert!(run(&ctx, ".acl delete console=op jobacl").success);
        let text = std::fs::read_to_string(dir.join("op.conf")).unwrap();
        assert!(!text.contains("JobAcl"), "{text}");
        let none = run(&ctx, ".acl delete console=op poolacl");
        assert_eq!(none.error_message, "Console \"op\" has no PoolAcl entries.\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn clear_and_copy() {
        let (ctx, dir) = setup("copy", "admin");
        assert!(run(&ctx, ".acl clear console=op").success);
        let text = std::fs::read_to_string(dir.join("op.conf")).unwrap();
        assert_eq!(text, "Console {\n  Name = op\n  Password = \"pw2\"\n  Profile = operator\n}\n");
        let result = run(&ctx, ".acl copy from=admin to=viewer");
        assert!(result.success, "{}", result.error_message);
        let text = std::fs::read_to_string(dir.join("viewer.conf")).unwrap();
        assert_eq!(text, "Console {\n  Name = viewer\n  Password = pw3\n  CommandAcl = \"*all*\"\n  JobAcl = \"*all*\"\n}\n");
        let config = ctx.config.as_ref().unwrap().read().unwrap();
        assert!(crate::acl::allowed(&config, "viewer", Kind::Command, "delete"));
        drop(config);
        assert_eq!(run(&ctx, ".acl copy from=nobody to=viewer").error_message, "Console \"nobody\" not found.\n");
        assert_eq!(run(&ctx, ".acl copy to=viewer").error_message, ".acl copy: from=<console> to=<console> required.\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shared_resource_file_is_not_rewritten() {
        let (ctx, dir) = setup("shared", "admin");
        let path = dir.join("op.conf");
        let shared = "Console {\n  Name = op\n  Password = \"pw2\"\n}\nProfile { Name = other }\n";
        std::fs::write(&path, shared).unwrap();
        let result = run(&ctx, ".acl set console=op jobacl=x");
        assert!(!result.success);
        assert!(result.error_message.contains("together with other resources"), "{}", result.error_message);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), shared);
        // Running configuration unchanged as well.
        let config = ctx.config.as_ref().unwrap().read().unwrap();
        assert!(entries(config.raw.iter().find(|r| r.name() == Some("op")).unwrap(), "JobAcl").is_empty());
        drop(config);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn usage_and_unknown_console() {
        let (ctx, dir) = setup("usage", "admin");
        assert_eq!(run(&ctx, ".acl frobnicate").error_message, USAGE);
        assert_eq!(run(&ctx, ".acl set jobacl=x").error_message, ".acl set: console=<name> required.\n");
        assert_eq!(run(&ctx, ".acl set console=ghost jobacl=x").error_message, "Console \"ghost\" not found.\n");
        assert_eq!(run(&ctx, ".acl list console=ghost").error_message, "Console \"ghost\" not found.\n");
        assert_eq!(run(&ctx, ".acl set console=op").error_message, ".acl set: specify at least one <type>acl=<entries>.\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
