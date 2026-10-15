//! Stand-ins for the interactive dialogs of the Bareos console (C-012).
//!
//! Upstream opens a dialog (StartPrompt/DoPrompt, GetCmd, GetYesno, GetPint;
//! the prompt is sent with BNET_SUB_PROMPT -27) wherever arguments are
//! missing or a confirmation is required. Onesimus does not implement those
//! dialogs yet (compatibility log C-011). Until it does, every such path ends
//! in [`selection_required`]: the text upstream sends before it waits is
//! returned, nothing is changed and no daemon is contacted. Each call site
//! names the upstream prompt it stands for, so C-011 can replace the call
//! with the real dialog.

use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;

/// End a command where upstream would wait for console input.
///
/// `text` is everything upstream sends up to and including the prompt
/// string. The command has no side effect; C-011 replaces this hook with the
/// real BNET_SUB_PROMPT dialog.
pub fn selection_required(text: String) -> CommandResult {
    CommandResult::ok(text)
}

/// `SendCmdUsage` (ua_output.cc:1923): "<message>\nUSAGE: <cmd> <usage>\n".
pub fn usage(command: &str, message: &str) -> String {
    let usage = crate::ua_commands_table::UA_COMMANDS.iter()
        .find(|entry| entry.0 == command).map(|entry| entry.2).unwrap_or("");
    format!("{message}\nUSAGE: {command} {usage}\n")
}

/// Prompt list as DoPrompt prints it (ua_select.cc:1251) for short lists:
/// header, "N: item" lines and "<msg> (1-N): ".
pub fn prompt_list(header: &str, items: &[String], msg: &str) -> String {
    let mut text = header.to_string();
    for (index, item) in items.iter().enumerate() {
        text.push_str(&format!("{}: {item}\n", index + 1));
    }
    text.push_str(&format!("{msg} ({}-{}): ", 1, items.len()));
    text
}

/// DoKeywordPrompt (ua_select.cc) — "You have the following choices:".
pub fn keyword_prompt(before: &str, keywords: &[&str], msg: &str) -> CommandResult {
    let items: Vec<String> = keywords.iter().map(|k| k.to_string()).collect();
    selection_required(format!("{before}{}",
        prompt_list("You have the following choices:\n", &items, msg)))
}

/// Resource selection of DoPrompt (ua_select.cc:1210) over an already
/// filtered, sorted list. One entry is selected automatically (with the
/// "Automatically selected" line outside API mode), an empty list fails with
/// the upstream error, several entries need the dialog.
pub fn select_resource(ctx: &ExecContext, header: &str, automsg: &str, msg: &str,
    items: Vec<String>) -> Result<(String, String), CommandResult> {
    match items.len() {
        0 => {
            let mut result = CommandResult::error(
                format!("Selection list for \"{automsg}\" is empty!\n"));
            result.text_output = header.to_string();
            Err(result)
        }
        1 => {
            let name = items.into_iter().next().unwrap();
            let text = if ctx.api_mode == ApiMode::Human {
                format!("Automatically selected {automsg}: {name}\n")
            } else { String::new() };
            Ok((name, text))
        }
        _ => Err(selection_required(prompt_list(header, &items, msg))),
    }
}

/// `get_storage_resource(ua, false, autochangers_only)` (ua_select.cc:1320)
/// for commands without a default argument: `storage=`/`sd=` names the
/// resource, `jobid=`/`job=`/`ujobid=` refer to running jobs, otherwise
/// `select_storage_resource` (ua_select.cc:197) offers the accessible
/// (autochanger) storages. Ok carries the resource and the text upstream
/// printed while selecting it.
pub fn select_storage(ast: &crate::command_ast::CommandAst, ctx: &ExecContext, autochangers_only: bool)
    -> Result<(bareos_config::resources::StorageResource, String), CommandResult> {
    use crate::acl::{access, Kind};
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else {
        return Err(CommandResult::error("No config\n".into()));
    };
    let mut errors = String::new();
    for arg in &ast.arguments {
        let key = arg.key.to_ascii_lowercase();
        match key.as_str() {
            "storage" | "sd" => {
                // A bare keyword (NULL argv) leaves the name empty: selection.
                if let Some(name) = arg.value.as_deref().filter(|name| !name.is_empty()) {
                    // GetStoreResWithName applies the Storage ACL (ua_acl.cc:279).
                    match config.find_storage(name) {
                        Some(store) if access(ctx, Kind::Storage, &store.name) =>
                            return Ok((store.clone(), String::new())),
                        _ => errors.push_str(&format!("Storage resource \"{name}\": not found\n")),
                    }
                }
                break;
            }
            "jobid" => {
                let id = arg.value.as_deref().and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0);
                if id <= 0 {
                    return Err(CommandResult::error(format!("Expecting jobid=nn command, got: {}\n", arg.key)));
                }
                // Onesimus keeps no storage per running job: treated as not running.
                return Err(CommandResult::error(format!("JobId {id} is not running.\n")));
            }
            "job" | "jobname" | "ujobid" => {
                let Some(name) = arg.value.as_deref() else {
                    let what = if key == "ujobid" { "ujobid" } else { "job" };
                    return Err(CommandResult::error(format!("Expecting {what}=xxx, got: {}.\n", arg.key)));
                };
                return Err(CommandResult::error(format!("Job \"{name}\" is not running.\n")));
            }
            _ => {}
        }
    }
    let names: Vec<String> = crate::director_config::sort_case_insensitive(config.storages.iter()
        .filter(|s| access(ctx, Kind::Storage, &s.name))
        .filter(|s| !autochangers_only || is_autochanger(&config, &s.name))
        .map(|s| s.name.as_str()))
        .into_iter().map(str::to_string).collect();
    let header = if autochangers_only { "The defined Autochanger Storage resources are:\n" }
        else { "The defined Storage resources are:\n" };
    let with_errors = |mut result: CommandResult| {
        result.text_output = format!("{errors}{}", result.text_output);
        result
    };
    match select_resource(ctx, header, "Storage", "Select Storage resource", names) {
        Ok((name, text)) => match config.find_storage(&name) {
            Some(store) => Ok((store.clone(), format!("{errors}{text}"))),
            None => Err(with_errors(CommandResult::error(String::new()))),
        },
        Err(result) => Err(with_errors(result)),
    }
}

/// get_client_resource (ua_select.cc:506): `client=`/`fd=` names an
/// accessible client, otherwise select_client_resource offers them.
pub fn select_client(ast: &crate::command_ast::CommandAst, ctx: &ExecContext)
    -> Result<(String, String), CommandResult> {
    use crate::acl::{access, Kind};
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else {
        return Err(CommandResult::error("No config\n".into()));
    };
    let mut errors = String::new();
    if let Some(name) = ast.arguments.iter()
        .find(|a| (a.key.eq_ignore_ascii_case("client") || a.key.eq_ignore_ascii_case("fd")) && a.value.is_some())
        .and_then(|a| a.value.as_deref()) {
        if config.find_client(name).is_some() && access(ctx, Kind::Client, name) {
            return Ok((name.to_string(), String::new()));
        }
        errors = format!("Error: Client resource {name} does not exist.\n");
    }
    let names = crate::director_config::sort_case_insensitive(config.clients.iter()
        .map(|c| c.name.as_str()).filter(|name| access(ctx, Kind::Client, name)))
        .into_iter().map(str::to_string).collect();
    match select_resource(ctx, "The defined Client resources are:\n", "Client",
        "Select Client (File daemon) resource", names) {
        Ok((name, text)) => Ok((name, format!("{errors}{text}"))),
        Err(mut result) => {
            result.text_output.insert_str(0, &errors);
            Err(result)
        }
    }
}

/// SelectPoolDbr (ua_select.cc) without a usable `pool=`: the catalog pools
/// in PoolId order (configured pools without a catalog), filtered by the
/// Pool ACL, under "Defined Pools:" / "Select the Pool".
pub fn select_pool_dbr(ctx: &ExecContext) -> Result<(String, String), CommandResult> {
    use crate::acl::{access, Kind};
    let from_catalog = ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows("SELECT Name FROM Pool ORDER BY PoolId").ok())
        .map(|table| table.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect::<Vec<_>>());
    let names = from_catalog.unwrap_or_else(|| ctx.config.as_ref().and_then(|c| c.read().ok())
        .map(|cfg| cfg.pools.iter().map(|p| p.name.clone()).collect()).unwrap_or_default());
    let names = names.into_iter().filter(|name| access(ctx, Kind::Pool, name)).collect();
    select_resource(ctx, "Defined Pools:\n", "Pool", "Select the Pool", names)
}

/// `Auto Changer = yes` in the Storage resource (dird_conf.cc "autochanger").
fn is_autochanger(config: &crate::director_config::DirectorConfig, name: &str) -> bool {
    config.raw.iter()
        .find(|r| r.norm_type() == "storage" && r.name() == Some(name))
        .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == "autochanger"))
        .and_then(|d| d.first())
        .is_some_and(|v| bareos_config::values::parse_bool(v).is_ok_and(|(_, on)| on))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ExecContext {
        ExecContext { session_id: 0, config: None, session_mgr: None, catalog: None,
            api_mode: ApiMode::Human, bus_rx: None, job_manager: None }
    }

    #[test]
    fn usage_uses_original_table() {
        assert_eq!(usage("truncate", "missing parameter"), "missing parameter\nUSAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]\n");
    }

    #[test]
    fn selection_follows_do_prompt() {
        let empty = select_resource(&ctx(), "H:\n", "Storage", "Select", vec![]).unwrap_err();
        assert_eq!((empty.text_output.as_str(), empty.error_message.as_str()),
            ("H:\n", "Selection list for \"Storage\" is empty!\n"));
        assert_eq!(select_resource(&ctx(), "H:\n", "Storage", "Select", vec!["A".into()]).unwrap(),
            ("A".to_string(), "Automatically selected Storage: A\n".to_string()));
        let many = select_resource(&ctx(), "H:\n", "Storage", "Select", vec!["A".into(), "B".into()]).unwrap_err();
        assert_eq!(many.text_output, "H:\n1: A\n2: B\nSelect (1-2): ");
    }
}
