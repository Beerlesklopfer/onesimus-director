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
//! - `.acl set|delete|clear|copy`: **refused** (since 19.11.2026). The ACLs
//!   of the configuration files are the outer frame of the two-tier ACL model
//!   (docs/plans/ACL-ZWEISTUFIG.md) and are changed in the editor only. Until
//!   then these rewrote the console's resource file, so a console allowed
//!   `.acl` and `configure` could widen its own rights. The inner tier (a
//!   catalog table that can only restrict further) will take these
//!   subcommands over.

use bareos_config::raw::ConfResource;
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
    (set, delete, clear and copy are refused: file ACLs are changed in the editor only)\n";

fn arg<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref())
}

fn norm(key: &str) -> String {
    bareos_config::raw::normalize_key(key)
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
        Some(action @ ("set" | "delete" | "clear" | "copy")) => acl_change(action),
        Some(_) => CommandResult::error(USAGE.to_string()).with_error_parts(),
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
// set / delete / clear / copy: refused
// ---------------------------------------------------------------------------

/// The text of a refused change (two-tier ACLs, docs/plans/ACL-ZWEISTUFIG.md):
/// ACLs in the configuration files are the outer frame and are set in the
/// editor only; the inner restriction in the catalog is not implemented yet.
fn refusal(action: &str) -> String {
    format!(".acl {action}: refused. ACLs of the configuration files (Console, Profile) are changed in the \
        editor only; the catalog ACL that can restrict them further is not implemented yet \
        (docs/plans/ACL-ZWEISTUFIG.md).\n")
}

/// `.acl set|delete|clear|copy`: refused for every console, whatever its
/// CommandACL. Until 19.11.2026 these rewrote the console's resource file;
/// a console allowed `.acl` and `configure` (e.g. the MCP console
/// `<name>-mcp`) could so widen its own rights.
fn acl_change(action: &str) -> CommandResult {
    CommandResult::error(refusal(action))
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
            std::fs::write(&path, format!("# Console {} (test file)\n", resource.name().unwrap())).unwrap();
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
    fn restricted_console_lists_only_itself() {
        let (ctx, dir) = setup("viewer", "viewer");
        let own = run(&ctx, ".acl list");
        assert!(own.text_output.starts_with("Console \"viewer\":\n"));
        assert_eq!(own.json.unwrap()["acl"].as_array().unwrap().len(), 1);
        let other = run(&ctx, ".acl list console=admin");
        assert!(!other.success);
        assert!(other.error_message.contains("requires the CommandACL configure"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// File ACLs are set in the editor only: every change is refused, also
    /// for a console with `configure` and `*all*`, and neither the file nor
    /// the running configuration changes.
    #[test]
    fn changes_are_refused_and_nothing_is_written() {
        let (ctx, dir) = setup("refused", "admin");
        let before: Vec<_> = ["admin", "op", "viewer"].iter()
            .map(|c| std::fs::read_to_string(dir.join(format!("{c}.conf"))).unwrap()).collect();
        for line in [".acl set console=viewer commandacl=*all*", ".acl delete console=op jobacl",
                     ".acl clear console=op", ".acl copy from=admin to=viewer"] {
            let result = run(&ctx, line);
            assert!(!result.success, "{line}");
            assert!(result.error_message.contains("refused") && result.error_message.contains("editor only"),
                "{line}: {}", result.error_message);
        }
        let after: Vec<_> = ["admin", "op", "viewer"].iter()
            .map(|c| std::fs::read_to_string(dir.join(format!("{c}.conf"))).unwrap()).collect();
        assert_eq!(before, after);
        let config = ctx.config.as_ref().unwrap().read().unwrap();
        assert!(!crate::acl::allowed(&config, "viewer", Kind::Command, "delete"));
        assert!(crate::acl::allowed(&config, "op", Kind::Client, "fd1"));
        drop(config);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn usage_and_unknown_console() {
        let (ctx, dir) = setup("usage", "admin");
        assert_eq!(run(&ctx, ".acl frobnicate").error_message, USAGE);
        assert_eq!(run(&ctx, ".acl list console=ghost").error_message, "Console \"ghost\" not found.\n");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
