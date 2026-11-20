//! `.acl list|set|delete|clear|copy` — Onesimus extension (no Bareos
//! counterpart; the original answers ".acl: is an invalid command.").
//!
//! Two-tier console ACLs (docs/plans/ACL-ZWEISTUFIG.md): the ACL directives
//! of the Console and Profile resources in the configuration files are the
//! outer frame and are changed in the editor only; the catalog ACL
//! (`onesimus.console_acl`, [`crate::catalog_acl`]) can only restrict
//! further. Effective is what both allow ([`crate::acl::allowed`]).
//!
//! - `.acl [list] [console=<name>]`: per ACL type the file entries (the
//!   console's own, then those of its Profiles in evaluation order), the
//!   catalog entries (an entry that allows only what the file denies is
//!   marked "no effect", decision F-3) and a summary of the effective result.
//!   Without `console=` the own console, or all consoles for consoles allowed
//!   to `configure`.
//! - `.acl set|delete|clear|copy`: change the **catalog** ACL; files are never
//!   written. They need the CommandACL `.acl` (the command itself) **and**
//!   `configure` (decision F-2). Another console's catalog ACL may be changed
//!   freely (it never reaches beyond that console's file ACL); the own
//!   console's only narrower, see [`narrower`]. Every change goes to the
//!   Director's messages (audit line with console, target, type, old and new).

use bareos_config::raw::ConfResource;
use serde_json::{json, Map, Value};

use crate::acl::Kind;
use crate::catalog_acl::{CatalogEntry, ConsoleLists, ConsoleSnapshot, LoadState, LOCKED_COMMANDS};
use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::director_config::DirectorConfig;

/// Short help, shown for an unknown subcommand.
const USAGE: &str = ".acl: usage:\n  .acl list [console=<name>]\n  \
    .acl set console=<name> <type>acl=<entry1>,<entry2> ...\n  \
    .acl delete console=<name> <type>acl[=<entry1>,...] ...\n  \
    .acl clear console=<name>\n  \
    .acl copy from=<console> to=<console>\n\
    set, delete, clear and copy change the catalog ACL, which can only restrict the file ACL \
    (file ACLs are changed in the editor only).\n\
    <type>: job, client, storage, schedule, pool, command, fileset, catalog, where, pluginoptions\n";

/// Longest catalog ACL entry accepted (regular expressions may exceed the
/// 127 characters of a resource name).
const MAX_ENTRY: usize = 1024;

/// Value of the argument `key` (case-insensitive key).
fn arg<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref())
}

/// Normalized directive key (`JobAcl` → `jobacl`).
fn norm(key: &str) -> String {
    bareos_config::raw::normalize_key(key)
}

/// Comma-separated entries of an argument value, trimmed, empty ones dropped.
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

/// Whether the session may run `configure` (effective ACL, both tiers).
fn may_configure(ctx: &ExecContext) -> bool {
    crate::acl::access(ctx, Kind::Command, "configure")
}

/// The resource of type `rtype` (normalized) named `name`.
fn find<'a>(config: &'a DirectorConfig, rtype: &str, name: &str) -> Option<&'a ConfResource> {
    config.raw.iter().find(|r| r.norm_type() == rtype && r.name() == Some(name))
}

/// Entries of the directive `acl` of one resource, in order.
fn entries<'a>(resource: &'a ConfResource, acl: &str) -> Vec<&'a str> {
    let key = norm(acl);
    resource.directives.iter().filter(|d| d.norm_key() == key).flat_map(|d| d.values()).collect()
}

/// Profile names assigned to a console, in order.
fn profiles(resource: &ConfResource) -> Vec<&str> {
    resource.directives.iter().filter(|d| d.norm_key() == "profile").flat_map(|d| d.values()).collect()
}

/// `.acl` entry point: dispatch on the first bare argument.
pub fn cmd_dot_acl(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let action = ast.arguments.first()
        .filter(|a| a.value.is_none())
        .map(|a| a.key.to_ascii_lowercase());
    match action.as_deref() {
        None | Some("list") => acl_list(ast, ctx),
        Some(action @ ("set" | "delete" | "clear" | "copy")) => acl_change(action, ast, ctx),
        Some(_) => CommandResult::error(USAGE.to_string()).with_error_parts(),
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// Names an ACL check of `kind` can meet, for the effective summary and the
/// "no effect" test of regex entries: the configured resources of the type,
/// the registered command names for Command; None for Where and
/// PluginOptions (free-form values).
fn known_names(config: &DirectorConfig, kind: Kind) -> Option<Vec<String>> {
    match (kind, kind.resource_type()) {
        (Kind::Command, _) => Some(crate::command_executor::get_registry().iter().map(|c| c.name.to_string()).collect()),
        (_, Some(rtype)) => Some(config.raw.iter()
            .filter(|r| r.norm_type() == rtype)
            .filter_map(|r| r.name().map(str::to_string))
            .collect()),
        _ => None,
    }
}

/// Decision F-3: whether the catalog entry `entry` of `console` has no
/// effect because everything it allows is denied by the file ACL.
///
/// - `!` entries restrict and are never marked.
/// - `*all*`: marked when the file ACL allows nothing of the type.
/// - A name: marked when the file ACL denies that name.
/// - A regular expression: marked when it matches at least one known name
///   ([`known_names`]) and the file ACL denies every name it matches.
///   Regexes of Where/PluginOptions (no known names) are never marked.
///
/// The test is informational: the entry is stored anyway, because the file
/// may change later.
fn ineffective(config: &DirectorConfig, console: &str, kind: Kind, entry: &str) -> bool {
    if entry.starts_with('!') {
        return false;
    }
    if entry.eq_ignore_ascii_case("*all*") {
        return !crate::acl::file_any_allowed(config, console, kind);
    }
    if !crate::acl::is_regex(entry) {
        return !crate::acl::file_allowed(config, console, kind, entry);
    }
    let Some(names) = known_names(config, kind) else { return false };
    let matched: Vec<&String> = names.iter().filter(|n| crate::acl::matches(entry, n)).collect();
    !matched.is_empty() && matched.iter().all(|n| !crate::acl::file_allowed(config, console, kind, n))
}

/// Text and JSON of the catalog load state.
fn catalog_state(snapshot: &ConsoleSnapshot) -> (String, Value) {
    match &snapshot.load {
        LoadState::Off => ("not used (no catalog)".into(), json!({ "state": "off" })),
        LoadState::Loaded if snapshot.table_missing => (
            format!("not available (table {} missing in the catalog)", crate::catalog_acl::TABLE),
            json!({ "state": "table_missing" })),
        LoadState::Loaded => ("loaded".into(), json!({ "state": "loaded" })),
        LoadState::Failed(error) if snapshot.locked => (
            format!("NOT READABLE ({error}); console limited to {}", LOCKED_COMMANDS.join(", ")),
            json!({ "state": "failed", "error": error })),
        LoadState::Failed(error) => (
            format!("not readable ({error}); console has no catalog entries, file ACL applies"),
            json!({ "state": "failed", "error": error })),
    }
}

/// Effective summary of one type: text and JSON.
fn effective(config: &DirectorConfig, console: &str, kind: Kind, snapshot: &ConsoleSnapshot) -> (String, Value) {
    let has_catalog = snapshot.lists.get(&kind).is_some_and(|l| !l.is_empty());
    let source = if snapshot.locked { "locked" } else if has_catalog { "file+catalog" } else { "file" };
    let prefix = match source {
        "locked" => "locked (catalog ACL not readable)",
        "file+catalog" => "file ACL and catalog ACL",
        _ => "file ACL only",
    };
    let Some(names) = known_names(config, kind) else {
        return (prefix.to_string(), json!({ "source": source, "allowed": null, "configured": null }));
    };
    let (allowed, denied): (Vec<&String>, Vec<&String>) =
        names.iter().partition(|n| crate::acl::allowed(config, console, kind, n));
    let what = if kind == Kind::Command { "commands" } else { "configured" };
    let join = |v: &[&String]| v.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
    let detail = if names.is_empty() {
        format!("none {what}")
    } else if denied.is_empty() {
        format!("all {} {what}", names.len())
    } else if allowed.is_empty() {
        format!("none of {} {what}", names.len())
    } else if allowed.len() <= denied.len() {
        format!("{} of {} {what}: {}", allowed.len(), names.len(), join(&allowed))
    } else {
        format!("{} of {} {what}, all except: {}", allowed.len(), names.len(), join(&denied))
    };
    (format!("{prefix}: {detail}"), json!({ "source": source, "allowed": allowed, "configured": names.len() }))
}

/// File, catalog and effective ACLs of one console as text and JSON.
fn describe(config: &DirectorConfig, console: &ConfResource) -> (String, Value) {
    let name = console.name().unwrap_or("");
    let snapshot = config.catalog_acl.snapshot(name);
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
    let (state_text, state_json) = catalog_state(&snapshot);
    text.push_str(&format!("  Catalog ACL = {state_text}\n"));
    object.insert("catalog_state".into(), state_json);
    object.insert("locked".into(), snapshot.locked.into());
    let mut catalog = Map::new();
    let mut summary = Map::new();
    for kind in Kind::ALL {
        let acl = kind.directive();
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

        let rows = snapshot.lists.get(&kind).map(Vec::as_slice).unwrap_or_default();
        let mut shown = Vec::new();
        let mut list = Vec::new();
        for (position, row) in rows.iter().enumerate() {
            let no_effect = ineffective(config, name, kind, &row.entry);
            shown.push(if no_effect {
                format!("{} (no effect: not allowed by the file ACL)", row.entry)
            } else {
                row.entry.clone()
            });
            list.push(json!({ "value": row.entry, "position": position, "ineffective": no_effect,
                "changed_by": row.changed_by, "changed_at": row.changed_at }));
        }
        let shown = if shown.is_empty() { "(none)".to_string() } else { shown.join(", ") };
        text.push_str(&format!("    Catalog = {shown}\n"));
        catalog.insert(acl.to_ascii_lowercase(), Value::Array(list));
        let (effective_text, effective_json) = effective(config, name, kind, &snapshot);
        text.push_str(&format!("    Effective = {effective_text}\n"));
        summary.insert(acl.to_ascii_lowercase(), effective_json);
    }
    object.insert("catalog".into(), Value::Object(catalog));
    object.insert("effective".into(), Value::Object(summary));
    (text, Value::Object(object))
}

/// `.acl [list] [console=<name>]`.
fn acl_list(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some(shared) = &ctx.config else { return CommandResult::error("No config loaded\n".into()) };
    let own = own_console(ctx);
    let wanted = arg(ast, "console");
    // acl::access takes the configuration lock itself: decide before reading.
    let all = may_configure(ctx);
    let Ok(config) = shared.read() else { return CommandResult::error("Config error\n".into()) };
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
// set / delete / clear / copy: the catalog ACL
// ---------------------------------------------------------------------------

/// Decision F-2, the rule for the **own** console: may its catalog list of
/// one type change from `old` to `new`?
///
/// Yes when
/// 1. `old` is empty (the type had no catalog entries, i.e. no inner
///    restriction: every list allows at most everything), or
/// 2. `new` is not empty and arises from `old` only by **inserting `!`
///    entries** (anywhere) and **removing entries without `!`**; the
///    remaining entries keep their relative order and every `!` entry of
///    `old` stays. Entries are compared as exact strings.
///
/// Why this never widens, whatever the entries match (names, regexes,
/// `*all*`): a Bareos list is decided by the first matching entry and denies
/// when none matches. An inserted `!x` can only turn an item's decision into
/// "deny". Removing a positive entry `p` changes only items whose first
/// match was `p` (allowed before); they now fall to a later entry or to the
/// default deny. Neither step can allow an item that was denied. Removing a
/// `!` entry, adding a positive entry, reordering, or removing the whole
/// list (no restriction) could widen and are refused, even where a
/// particular case would not widen: the rule is syntactic and therefore
/// decidable, and it is conservative (inclusion of regular-expression
/// lists in general is not checked).
pub(crate) fn narrower(old: &[String], new: &[String]) -> bool {
    if old.is_empty() {
        return true;
    }
    if new.is_empty() {
        return false;
    }
    // Greedy embedding of the kept entries of `old` into `new`: every entry
    // of `new` either matches the next kept entry of `old` (skipping only
    // positive entries of `old`, which are removed) or is an inserted `!`
    // entry. Matching as early as possible is optimal: positive entries of
    // `old` are optional, `!` entries of `old` must all be matched.
    let mut j = 0;
    for entry in new {
        let next = old[j..].iter().position(|o| o == entry)
            .filter(|&k| old[j..j + k].iter().all(|skipped| !skipped.starts_with('!')));
        match next {
            Some(k) => j += k + 1,
            None if entry.starts_with('!') => {}
            None => return false,
        }
    }
    old[j..].iter().all(|rest| !rest.starts_with('!'))
}

/// Check one catalog entry before it is stored.
fn valid_entry(entry: &str) -> Result<(), String> {
    let pattern = entry.strip_prefix('!').unwrap_or(entry);
    if pattern.trim().is_empty() {
        return Err(format!("invalid ACL entry \"{entry}\": empty"));
    }
    if entry.len() > MAX_ENTRY {
        return Err(format!("invalid ACL entry \"{entry}\": longer than {MAX_ENTRY} characters"));
    }
    if entry.chars().any(char::is_control) {
        return Err(format!("invalid ACL entry \"{entry}\": control character"));
    }
    Ok(())
}

/// The catalog lists of `console` as plain strings.
fn plain(lists: &ConsoleLists, kind: Kind) -> Vec<String> {
    lists.get(&kind).map(|l| l.iter().map(|e: &CatalogEntry| e.entry.clone()).collect()).unwrap_or_default()
}

/// One planned change of a catalog list: type, old list, new list.
type Change = (Kind, Vec<String>, Vec<String>);

/// The new lists of the changed types (`(type, old, new)`), computed from
/// the current catalog lists of the target (`current`) and, for `copy`, of
/// the source (`source`). Types whose list does not change are left out.
fn plan(action: &str, ast: &CommandAst, target: &str, current: &ConsoleLists, source: &ConsoleLists)
    -> Result<Vec<Change>, String> {
    let typed: Vec<(Kind, Option<&str>)> = ast.arguments.iter()
        .filter_map(|a| Kind::from_argument(&a.key).filter(|_| a.key.to_ascii_lowercase().ends_with("acl"))
            .map(|k| (k, a.value.as_deref())))
        .collect();
    let mut changes = Vec::new();
    let mut push = |kind: Kind, new: Vec<String>| {
        let old = plain(current, kind);
        if old != new {
            changes.push((kind, old, new));
        }
    };
    match action {
        "set" => {
            if typed.is_empty() {
                return Err(".acl set: specify at least one <type>acl=<entries>.\n".into());
            }
            for (kind, value) in typed {
                let values = split_entries(value.unwrap_or(""));
                if values.is_empty() {
                    return Err(format!(".acl set: {} needs at least one entry (use .acl delete to remove it).\n",
                        kind.directive()));
                }
                for entry in &values {
                    valid_entry(entry).map_err(|e| format!(".acl set: {e}.\n"))?;
                }
                push(kind, values);
            }
        }
        "delete" => {
            if typed.is_empty() {
                return Err(".acl delete: specify at least one <type>acl[=<entries>].\n".into());
            }
            for (kind, value) in typed {
                let old = plain(current, kind);
                if old.is_empty() {
                    return Err(format!("Console \"{target}\" has no catalog {} entries.\n", kind.directive()));
                }
                let new = match value {
                    None => Vec::new(),
                    Some(value) => {
                        let remove = split_entries(value);
                        if let Some(missing) = remove.iter().find(|e| !old.contains(e)) {
                            return Err(format!("{} entry \"{missing}\" not found in the catalog ACL of Console \"{target}\".\n",
                                kind.directive()));
                        }
                        old.iter().filter(|e| !remove.contains(e)).cloned().collect()
                    }
                };
                push(kind, new);
            }
        }
        "clear" => {
            for kind in Kind::ALL {
                push(kind, Vec::new());
            }
        }
        "copy" => {
            for kind in Kind::ALL {
                push(kind, plain(source, kind));
            }
        }
        _ => unreachable!("cmd_dot_acl dispatches set, delete, clear and copy only"),
    }
    Ok(changes)
}

/// `.acl set|delete|clear|copy`: change the catalog ACL of a console.
fn acl_change(action: &str, ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // `.acl` itself was checked by the dispatcher (both tiers).
    if !may_configure(ctx) {
        return CommandResult::error(format!(
            ".acl {action}: permission denied, changing the catalog ACL requires the CommandACL .acl and configure.\n"));
    }
    let target = if action == "copy" { arg(ast, "to") } else { arg(ast, "console") };
    let Some(target) = target.filter(|t| !t.is_empty()).map(str::to_string) else {
        return CommandResult::error(if action == "copy" {
            ".acl copy: from=<console> to=<console> required.\n".to_string()
        } else {
            format!(".acl {action}: console=<name> required.\n")
        });
    };
    let from = arg(ast, "from").unwrap_or("").to_string();
    if action == "copy" && from.is_empty() {
        return CommandResult::error(".acl copy: from=<console> to=<console> required.\n".into());
    }
    let Some(own) = own_console(ctx) else {
        return CommandResult::error(format!(".acl {action}: session not authenticated.\n"));
    };
    let Some(shared) = &ctx.config else { return CommandResult::error("No config loaded\n".into()) };
    let store = {
        let Ok(config) = shared.read() else { return CommandResult::error("Config error\n".into()) };
        let known = |name: &str| find(&config, "console", name).is_some();
        // Catalog entries for a console that is not configured would do
        // nothing; clear/delete may still remove those of a removed console.
        let leftover = config.catalog_acl.consoles().iter().any(|c| c == &target);
        if !known(&target) && !(matches!(action, "clear" | "delete") && leftover) {
            return CommandResult::error(format!("Console \"{target}\" not found.\n"));
        }
        if action == "copy" && !known(&from) {
            return CommandResult::error(format!("Console \"{from}\" not found.\n"));
        }
        config.catalog_acl.clone()
    };
    // Decide on the catalog's current content, not on a stale cache.
    match store.refresh() {
        LoadState::Loaded if store.snapshot(&target).table_missing => {
            return CommandResult::error(format!(".acl {action}: refused, the catalog has no table {} \
                (schema extension v2253 missing, see the Director's start messages).\n", crate::catalog_acl::TABLE));
        }
        LoadState::Loaded => {}
        LoadState::Off => {
            return CommandResult::error(format!(".acl {action}: no catalog, the catalog ACL is not available.\n"));
        }
        LoadState::Failed(error) => {
            return CommandResult::error(format!(".acl {action}: refused, the catalog ACL is not readable ({error}).\n"));
        }
    }
    let current = store.snapshot(&target).lists;
    let source = if action == "copy" { store.snapshot(&from).lists } else { ConsoleLists::new() };
    let changes = match plan(action, ast, &target, &current, &source) {
        Ok(changes) => changes,
        Err(e) => return CommandResult::error(e),
    };
    // F-2: the own console only narrower.
    let widening = changes.iter().find(|(_, old, new)| !narrower(old, new)).filter(|_| target == own);
    if let Some((kind, _, _)) = widening {
        return CommandResult::error(format!(
            ".acl {action}: refused, Console \"{target}\" may only restrict its own catalog ACL further; \
             the change of {} could allow more. On the own console only inserting ! entries, removing \
             entries without !, or a first list for a type without catalog entries is allowed.\n",
            kind.directive()));
    }
    if changes.is_empty() {
        let Ok(config) = shared.read() else { return CommandResult::error("Config error\n".into()) };
        let Some(console) = find(&config, "console", &target) else {
            return CommandResult::ok(format!("Catalog ACL of Console \"{target}\" unchanged.\n"));
        };
        let (text, list) = describe(&config, console);
        return CommandResult::ok(format!("Catalog ACL of Console \"{target}\" unchanged.\n{text}"))
            .with_json(json!({ "acl": { "action": action, "console": target, "changes": [], "effective": list } }));
    }
    let writes: Vec<(Kind, Vec<String>)> = changes.iter().map(|(k, _, new)| (*k, new.clone())).collect();
    let written = store.write(&target, &writes, &own);
    let Ok(config) = shared.read() else { return CommandResult::error("Config error\n".into()) };
    if let Err(error) = written {
        let line = format!("Console [{own}] catalog acl {action} Console \"{target}\" failed: {error}");
        tracing::warn!("{line}");
        crate::audit::daemon_message(&config, crate::message_bus::MsgType::Warning, &format!("{line}\n"));
        return CommandResult::error(format!(".acl {action}: writing the catalog ACL failed: {error}\n"));
    }
    let mut text = format!("Catalog ACL of Console \"{target}\" changed:\n");
    let mut changed = Vec::new();
    let mut notes = Vec::new();
    let show = |l: &[String]| if l.is_empty() { "(none)".to_string() } else { l.join(", ") };
    for (kind, old, new) in &changes {
        // Audit like config_edit: console, target, type, old and new.
        let line = format!("Console [{own}] catalog acl {action} Console \"{target}\" {}: old [{}], new [{}]",
            kind.directive(), old.join(", "), new.join(", "));
        tracing::info!("{line}");
        crate::audit::daemon_message(&config, crate::message_bus::MsgType::Info, &format!("{line}\n"));
        text.push_str(&format!("  {}: old = {}; new = {}\n", kind.directive(), show(old), show(new)));
        changed.push(json!({ "type": kind.directive(), "old": old, "new": new }));
        for entry in new.iter().filter(|e| ineffective(&config, &target, *kind, e)) {
            text.push_str(&format!("  Note: {} entry \"{entry}\" has no effect: not allowed by the file ACL \
                (stored, it applies if the file ACL allows it later).\n", kind.directive()));
            notes.push(json!({ "type": kind.directive(), "value": entry }));
        }
    }
    let effective = match find(&config, "console", &target) {
        Some(console) => {
            let (t, list) = describe(&config, console);
            text.push_str(&t);
            list
        }
        None => Value::Null,
    };
    CommandResult::ok(text).with_json(json!({ "acl": {
        "action": action, "console": target, "changes": changed, "ineffective": notes, "effective": effective,
    }}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse, SqlTable};
    use crate::command_executor::ApiMode;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, RwLock};

    const CONFIG: &str = r#"
Director { Name = dir; Password = secret }
Profile { Name = operator; CommandAcl = status, list; JobAcl = *all* }
Job { Name = backup-a; Type = Backup }
Job { Name = backup-b; Type = Backup }
Job { Name = restore; Type = Restore }
Console { Name = admin; Password = pw; CommandAcl = *all*; JobAcl = *all* }
Console { Name = op; Password = "pw2"; CommandAcl = !delete; ClientAcl = fd1; Profile = operator }
Console { Name = viewer; Password = pw3; CommandAcl = .acl, status }
Console { Name = mcp; Password = pw4; CommandAcl = *all*; JobAcl = backup-a, restore }
"#;

    /// An in-memory `onesimus.console_acl` behind a mock catalog: the rows
    /// (console, type, entry) in order; `fail` makes every query fail.
    #[derive(Clone, Default)]
    struct FakeTable {
        rows: Arc<Mutex<Vec<(String, String, String)>>>,
        fail: Arc<AtomicBool>,
    }

    impl FakeTable {
        /// The mock catalog serving this table (reads and the writes of
        /// `catalog_acl::write_sql`).
        fn handle(&self) -> CatalogHandle {
            let table = self.clone();
            CatalogHandle::mock(move |request| {
                if table.fail.load(Ordering::SeqCst) {
                    return CatalogResponse::Error("could not connect to server".into());
                }
                match request {
                    CatalogRequest::TextRows { sql } if sql.contains("to_regclass") => CatalogResponse::Rows(SqlTable {
                        columns: vec![], rows: vec![vec![Some(crate::catalog_acl::TABLE.into())]] }),
                    CatalogRequest::TextRows { .. } => {
                        let mut rows = table.rows.lock().unwrap().clone();
                        rows.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
                        CatalogResponse::Rows(SqlTable { columns: vec![], rows: rows.into_iter().enumerate()
                            .map(|(i, (c, t, e))| vec![Some(c), Some(t), Some(i.to_string()), Some(e),
                                Some("admin".into()), Some("2026-11-20 00:00:00+01".into())]).collect() })
                    }
                    CatalogRequest::Execute { sql } => {
                        let mut rows = table.rows.lock().unwrap();
                        for statement in sql.lines() {
                            let values: Vec<String> = statement.split('\'').skip(1).step_by(2).map(str::to_string).collect();
                            if statement.starts_with("DELETE") {
                                rows.retain(|(c, t, _)| !(c == &values[0] && t == &values[1]));
                            } else if statement.starts_with("INSERT") {
                                rows.push((values[0].clone(), values[1].clone(), values[2].clone()));
                            }
                        }
                        CatalogResponse::Ok(1)
                    }
                    _ => CatalogResponse::Error("unexpected".into()),
                }
            })
        }

        /// Entries of `console` and type `kind` (catalog name) in order.
        fn list(&self, console: &str, kind: &str) -> Vec<String> {
            self.rows.lock().unwrap().iter().filter(|(c, t, _)| c == console && t == kind).map(|r| r.2.clone()).collect()
        }
    }

    /// Context of a session authenticated as `console`, with a catalog ACL
    /// served by `table` and loaded.
    fn setup(console: &str, table: &FakeTable) -> ExecContext {
        let config = DirectorConfig::from_str(CONFIG).unwrap();
        config.catalog_acl.attach(table.handle());
        config.catalog_acl.refresh();
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session(console);
        manager.authenticate_session(id);
        ExecContext {
            session_id: id,
            config: Some(Arc::new(RwLock::new(config))),
            session_mgr: Some(manager),
            catalog: None,
            api_mode: ApiMode::Human,
            bus_rx: None,
            job_manager: None,
        }
    }

    /// Run one `.acl` line.
    fn run(ctx: &ExecContext, line: &str) -> CommandResult {
        cmd_dot_acl(&crate::session::parse_cli_command(line), ctx)
    }

    /// The effective decision of `console` (both tiers).
    fn allowed(ctx: &ExecContext, console: &str, kind: Kind, item: &str) -> bool {
        crate::acl::allowed(&ctx.config.as_ref().unwrap().read().unwrap(), console, kind, item)
    }

    /// Strings of a list of literals.
    fn s(list: &[&str]) -> Vec<String> {
        list.iter().map(|e| e.to_string()).collect()
    }

    /// `.acl list`: file entries with their source, catalog entries and the
    /// effective summary per type, in text and JSON.
    #[test]
    fn list_shows_file_catalog_and_effective_per_type() {
        let table = FakeTable::default();
        let ctx = setup("admin", &table);
        assert!(run(&ctx, ".acl set console=op jobacl=!restore,backup-.*").success);
        let result = run(&ctx, ".acl list console=op");
        assert!(result.success, "{}", result.error_message);
        let text = &result.text_output;
        assert!(text.starts_with("Console \"op\":\n  Profile = operator\n  Catalog ACL = loaded\n  \
            JobAcl = *all* (Profile operator)\n    Catalog = !restore, backup-.*\n    \
            Effective = file ACL and catalog ACL: 2 of 3 configured, all except: restore\n  \
            ClientAcl = fd1\n    Catalog = (none)\n    Effective = file ACL only: none configured\n"), "{text}");
        assert!(text.contains("  CommandAcl = !delete, status (Profile operator), list (Profile operator)\n    \
            Catalog = (none)\n    Effective = file ACL only: 2 of "), "{text}");
        assert!(text.contains("  WhereAcl = (none)\n    Catalog = (none)\n    Effective = file ACL only\n"), "{text}");
        let json = result.json.unwrap();
        let op = &json["acl"][0];
        assert_eq!(op["console"], "op");
        assert_eq!(op["commandacl"][1], json!({ "value": "status", "source": "Profile operator" }));
        assert_eq!(op["catalog"]["jobacl"][1]["value"], "backup-.*");
        assert_eq!(op["catalog"]["jobacl"][1]["ineffective"], false);
        assert_eq!(op["effective"]["jobacl"], json!({ "source": "file+catalog", "allowed": ["backup-a", "backup-b"], "configured": 3 }));
        assert_eq!(op["catalog_state"], json!({ "state": "loaded" }));
        // Without console= a configure-allowed console sees all consoles.
        assert_eq!(run(&ctx, ".acl").json.unwrap()["acl"].as_array().unwrap().len(), 4);
    }

    /// The typed RPC DotAclList keeps working: the file entries map onto
    /// ConsoleAclInfo as before, the new catalog/effective objects go to
    /// `extra_fields` (JSON text) and nothing is lost.
    #[test]
    fn list_json_maps_onto_the_typed_acl_list_response() {
        use prost::Message;
        let table = FakeTable::default();
        let ctx = setup("admin", &table);
        assert!(run(&ctx, ".acl set console=op jobacl=!restore,*all*").success);
        let json = run(&ctx, ".acl list console=op").json.unwrap();
        let encoded = onesimus_proto_rs::typed::json_to_message("onesimus.director.api.AclListResponse", &json);
        assert!(encoded.unmapped.is_empty(), "{:?}", encoded.unmapped);
        let typed = onesimus_proto_rs::onesimus::director::api::AclListResponse::decode(encoded.bytes.as_slice()).unwrap();
        let op = &typed.acl[0];
        assert_eq!(op.console, "op");
        assert_eq!(op.commandacl[0].value, "!delete");
        let catalog: Value = serde_json::from_str(&op.extra_fields["catalog"]).unwrap();
        assert_eq!(catalog["jobacl"][0]["value"], "!restore");
        assert!(op.extra_fields.contains_key("effective") && op.extra_fields.contains_key("catalog_state"));
    }

    /// A console without `configure` lists only itself and changes nothing.
    #[test]
    fn restricted_console_lists_only_itself_and_cannot_change() {
        let table = FakeTable::default();
        let ctx = setup("viewer", &table);
        let own = run(&ctx, ".acl list");
        assert!(own.text_output.starts_with("Console \"viewer\":\n"));
        assert_eq!(own.json.unwrap()["acl"].as_array().unwrap().len(), 1);
        let other = run(&ctx, ".acl list console=admin");
        assert!(other.error_message.contains("requires the CommandACL configure"));
        // .acl alone is not enough to change (F-2: .acl and configure).
        let change = run(&ctx, ".acl set console=viewer commandacl=!status");
        assert!(change.error_message.contains("requires the CommandACL .acl and configure"), "{}", change.error_message);
        assert!(table.rows.lock().unwrap().is_empty());
    }

    /// Intersection per type: the catalog only restricts, `!`, regex and
    /// `*all*` follow Bareos list semantics, a type without catalog entries
    /// does not restrict, and the cache changes with every `.acl` change.
    #[test]
    fn effective_is_file_and_catalog_and_cache_follows_changes() {
        let table = FakeTable::default();
        let ctx = setup("admin", &table);
        assert!(allowed(&ctx, "op", Kind::Command, "status") && allowed(&ctx, "op", Kind::Command, "list"));
        let r = run(&ctx, ".acl set console=op commandacl=!list,*all*");
        assert!(r.success, "{}", r.error_message);
        assert_eq!(table.list("op", "command"), ["!list", "*all*"]);
        // Negation in the catalog removes list; *all* allows only what the file allows.
        assert!(!allowed(&ctx, "op", Kind::Command, "list"));
        assert!(allowed(&ctx, "op", Kind::Command, "status"));
        assert!(!allowed(&ctx, "op", Kind::Command, "delete"), "file denial stays");
        assert!(!allowed(&ctx, "op", Kind::Command, "run"), "file does not allow run");
        // Types without catalog entries keep the file ACL.
        assert!(allowed(&ctx, "op", Kind::Client, "fd1") && allowed(&ctx, "op", Kind::Job, "anything"));
        // Regex in the catalog; no match denies.
        assert!(run(&ctx, ".acl set console=op jobacl=backup-[ab]").success);
        assert!(allowed(&ctx, "op", Kind::Job, "backup-a") && allowed(&ctx, "op", Kind::Job, "BACKUP-B"));
        assert!(!allowed(&ctx, "op", Kind::Job, "backup-c") && !allowed(&ctx, "op", Kind::Job, "restore"));
        // Delete one entry, the whole type, clear.
        assert!(run(&ctx, ".acl delete console=op commandacl=!list").success);
        assert!(allowed(&ctx, "op", Kind::Command, "list"));
        assert!(run(&ctx, ".acl delete console=op jobacl").success);
        assert!(allowed(&ctx, "op", Kind::Job, "restore"));
        assert!(run(&ctx, ".acl clear console=op").success);
        assert!(table.rows.lock().unwrap().is_empty());
        // Copy gives the target the source's catalog lists.
        assert!(run(&ctx, ".acl set console=viewer commandacl=!status").success);
        assert!(run(&ctx, ".acl copy from=viewer to=op").success);
        assert_eq!(table.list("op", "command"), ["!status"]);
        assert!(!allowed(&ctx, "op", Kind::Command, "status") && !allowed(&ctx, "op", Kind::Command, "list"));
        // Files are never written.
        let config = ctx.config.as_ref().unwrap().read().unwrap();
        assert_eq!(entries(find(&config, "console", "op").unwrap(), "CommandAcl"), ["!delete"]);
    }

    /// F-2: the own console may narrow, never widen; other consoles freely.
    #[test]
    fn own_console_only_narrower_other_consoles_freely() {
        let table = FakeTable::default();
        let ctx = setup("mcp", &table);
        // First list for a type without entries: allowed (restricts).
        assert!(run(&ctx, ".acl set console=mcp commandacl=status,list,.acl,configure,*all*").success);
        // Insert a negation, remove positive entries: narrower.
        let r = run(&ctx, ".acl set console=mcp commandacl=!delete,status,.acl,configure,*all*");
        assert!(r.success, "{}", r.error_message);
        assert!(run(&ctx, ".acl delete console=mcp commandacl=*all*").success);
        assert_eq!(table.list("mcp", "command"), ["!delete", "status", ".acl", "configure"]);
        assert!(!allowed(&ctx, "mcp", Kind::Command, "run"));
        // Widening: removing a negation, adding a positive entry, removing the
        // whole type or clearing is refused and nothing is written.
        for line in [".acl delete console=mcp commandacl=!delete", ".acl set console=mcp commandacl=!delete,status,.acl,configure,run",
                     ".acl delete console=mcp commandacl", ".acl clear console=mcp",
                     ".acl set console=mcp commandacl=status,!delete,.acl,configure",
                     ".acl copy from=admin to=mcp"] {
            let r = run(&ctx, line);
            assert!(!r.success, "{line}");
            assert!(r.error_message.contains("may only restrict its own catalog ACL further"), "{line}: {}", r.error_message);
        }
        assert_eq!(table.list("mcp", "command"), ["!delete", "status", ".acl", "configure"]);
        // Another console: any change, also widening.
        assert!(run(&ctx, ".acl set console=op commandacl=!status").success);
        assert!(run(&ctx, ".acl delete console=op commandacl").success);
        assert!(table.list("op", "command").is_empty());
    }

    /// The own-console rule ([`narrower`]) on plain lists.
    #[test]
    fn narrower_rule() {
        assert!(narrower(&[], &s(&["*all*"])));
        assert!(narrower(&s(&["a", "b"]), &s(&["!x", "a", "!y", "b", "!z"])));
        assert!(narrower(&s(&["a", "b"]), &s(&["b"])));
        assert!(narrower(&s(&["!a", "b", "!c", "d"]), &s(&["!a", "!c"])));
        assert!(narrower(&s(&["a", "!a"]), &s(&["!a"])), "removing a skipped positive before a kept negation");
        assert!(!narrower(&s(&["a"]), &[]));
        assert!(!narrower(&s(&["!a", "*all*"]), &s(&["*all*"])));
        assert!(!narrower(&s(&["a"]), &s(&["a", "b"])));
        assert!(!narrower(&s(&["a", "b"]), &s(&["b", "a"])));
        assert!(!narrower(&s(&["!a", "b"]), &s(&["b", "!a"])), "moving a negation behind a positive");
        assert!(!narrower(&s(&["!a", "!b"]), &s(&["!b"])));
    }

    /// F-3: entries that allow only what the file denies are stored and
    /// reported as without effect.
    #[test]
    fn entries_beyond_the_file_are_stored_and_reported_without_effect() {
        let table = FakeTable::default();
        let ctx = setup("admin", &table);
        let r = run(&ctx, ".acl set console=mcp jobacl=backup-a,backup-b,backup-.*");
        assert!(r.success, "{}", r.error_message);
        assert!(r.text_output.contains("Note: JobAcl entry \"backup-b\" has no effect"), "{}", r.text_output);
        assert!(!r.text_output.contains("entry \"backup-a\" has no effect"));
        assert!(!r.text_output.contains("entry \"backup-.*\" has no effect"), "matches the allowed backup-a");
        assert_eq!(r.json.unwrap()["acl"]["ineffective"], json!([{ "type": "JobAcl", "value": "backup-b" }]));
        assert_eq!(table.list("mcp", "job"), ["backup-a", "backup-b", "backup-.*"]);
        assert!(!allowed(&ctx, "mcp", Kind::Job, "backup-b"));
        let list = run(&ctx, ".acl list console=mcp");
        assert!(list.text_output.contains("    Catalog = backup-a, backup-b (no effect: not allowed by the file ACL), backup-.*\n"),
            "{}", list.text_output);
        assert_eq!(list.json.unwrap()["acl"][0]["catalog"]["jobacl"][1]["ineffective"], true);
        // *all* without anything allowed by the file, regex matching only denied names.
        let config = ctx.config.as_ref().unwrap().read().unwrap();
        assert!(ineffective(&config, "viewer", Kind::Pool, "*all*"));
        assert!(ineffective(&config, "mcp", Kind::Job, "backup-[b]"));
        assert!(!ineffective(&config, "mcp", Kind::Job, "nothing-matches-.*"));
        assert!(!ineffective(&config, "mcp", Kind::Job, "!backup-b"));
    }

    /// F-4: an unreadable catalog limits consoles with (possible) catalog
    /// entries to status/version, never silently to the file ACL; consoles
    /// known to have none keep the file ACL; changes are refused.
    #[test]
    fn unreadable_catalog_locks_consoles_with_entries() {
        let table = FakeTable::default();
        let ctx = setup("admin", &table);
        assert!(run(&ctx, ".acl set console=op commandacl=*all*").success);
        table.fail.store(true, Ordering::SeqCst);
        let store = ctx.config.as_ref().unwrap().read().unwrap().catalog_acl.clone();
        assert!(matches!(store.refresh(), LoadState::Failed(_)));
        assert!(allowed(&ctx, "op", Kind::Command, "status"));
        assert!(!allowed(&ctx, "op", Kind::Command, "list"), "file and catalog allow list, but locked");
        assert!(!allowed(&ctx, "op", Kind::Client, "fd1") && !allowed(&ctx, "op", Kind::Job, "backup-a"));
        assert!(allowed(&ctx, "admin", Kind::Command, "delete"), "known without catalog entries");
        let list = run(&ctx, ".acl list console=op");
        assert!(list.text_output.contains("Catalog ACL = NOT READABLE (could not connect to server); console limited to status, .status, version, .api"),
            "{}", list.text_output);
        assert!(list.text_output.contains("    Effective = locked (catalog ACL not readable): 1 of"), "{}", list.text_output);
        let change = run(&ctx, ".acl set console=viewer commandacl=!status");
        assert!(change.error_message.contains("refused, the catalog ACL is not readable"), "{}", change.error_message);
        // Back: the catalog decides again.
        table.fail.store(false, Ordering::SeqCst);
        assert_eq!(store.refresh(), LoadState::Loaded);
        assert!(allowed(&ctx, "op", Kind::Command, "list") && allowed(&ctx, "op", Kind::Client, "fd1"));
    }

    /// The cache survives a configuration reload (same store) and is read
    /// again with it.
    #[test]
    fn reload_keeps_the_store_and_reloads_it() {
        let table = FakeTable::default();
        let ctx = setup("admin", &table);
        assert!(run(&ctx, ".acl set console=op commandacl=!status,*all*").success);
        // A row written behind odir's back shows up after the reload's refresh.
        table.rows.lock().unwrap().push(("op".into(), "client".into(), "!fd1".into()));
        assert!(allowed(&ctx, "op", Kind::Client, "fd1"));
        let shared = ctx.config.as_ref().unwrap();
        let mut fresh = DirectorConfig::from_str(CONFIG).unwrap();
        let store = shared.read().unwrap().catalog_acl.clone();
        fresh.catalog_acl = store.clone(); // what reload::install_config does
        *shared.write().unwrap() = fresh;
        store.refresh(); // what reload::reload_config does after installing
        assert!(!allowed(&ctx, "op", Kind::Command, "status"));
        assert!(!allowed(&ctx, "op", Kind::Client, "fd1"));
    }

    /// Usage, argument and catalog errors.
    #[test]
    fn usage_errors_and_unknown_console() {
        let table = FakeTable::default();
        let ctx = setup("admin", &table);
        assert_eq!(run(&ctx, ".acl frobnicate").error_message, USAGE);
        assert_eq!(run(&ctx, ".acl list console=ghost").error_message, "Console \"ghost\" not found.\n");
        assert_eq!(run(&ctx, ".acl set console=ghost jobacl=a").error_message, "Console \"ghost\" not found.\n");
        assert_eq!(run(&ctx, ".acl set console=op").error_message, ".acl set: specify at least one <type>acl=<entries>.\n");
        assert!(run(&ctx, ".acl set console=op jobacl=!").error_message.contains("invalid ACL entry"));
        assert_eq!(run(&ctx, ".acl delete console=op jobacl=x").error_message, "Console \"op\" has no catalog JobAcl entries.\n");
        assert_eq!(run(&ctx, ".acl copy from=op").error_message, ".acl copy: from=<console> to=<console> required.\n");
        // Without a catalog the inner tier is off.
        let mut off = setup("admin", &table);
        off.config = Some(Arc::new(RwLock::new(DirectorConfig::from_str(CONFIG).unwrap())));
        assert!(run(&off, ".acl set console=op jobacl=a").error_message.contains("no catalog"));
    }
}
