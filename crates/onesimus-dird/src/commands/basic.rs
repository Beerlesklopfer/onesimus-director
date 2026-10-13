//! Basic commands: version, time, quit, help, .help
//!
//! `.help` is the authoritative command reference — it lists ALL commands
//! (regular + dot-commands) in a machine-readable, tab-separated format.
//! `help` lists only regular commands and refers the user to `.help`.

use crate::command_ast::CommandAst;
use crate::command_executor::{ExecContext, ApiMode};
use crate::command_queue::CommandResult;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn cmd_version(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let os_release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let field = |key: &str| os_release.lines()
        .find_map(|l| l.strip_prefix(key))
        .map(|v| v.trim_matches('"').to_string());
    let os = field("PRETTY_NAME=").unwrap_or_else(|| "Linux".to_string());
    let distname = field("ID=").unwrap_or_default();
    let name = ctx.config.as_ref()
        .and_then(|c| c.read().ok().map(|c| c.director.name.clone()))
        .unwrap_or_else(|| "bareos-dir".to_string());
    // Text keeps the "bareos-dir Version:" form for bconsole/WebUI compatibility
    CommandResult::ok(format!("bareos-dir Version: {VERSION} ({os})\n")).with_json(serde_json::json!({
        "version": {
            "name": name,
            "type": "bareos-director",
            "version": VERSION,
            "bdate": env!("ONESIMUS_BUILD_DATE"),
            "operatingsystem": os,
            "distname": distname,
            "distversion": os,
            "customversionid": "",
        }
    }))
}

/// C++ time_cmd: bstrftime "%a %d-%b-%Y %H:%M:%S" (locale-dependent) + parts.
pub fn cmd_time(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let full = crate::localtime::strftime_now("%a %d-%b-%Y %H:%M:%S");
    let part = |f: &str| crate::localtime::strftime_now(f);
    CommandResult::ok(format!("{full}\n")).with_json(serde_json::json!({
        "time": {
            "full": full,
            "year": part("%Y"),
            "month": part("%m"),
            "day": part("%d"),
            "hour": part("%H"),
            "minute": part("%M"),
            "second": part("%S"),
        }
    }))
}

pub fn cmd_quit(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    CommandResult::ok("bye\n".to_string())
}

/// `help [command]` — human-readable help from the command registry (SSOT).
pub fn cmd_help(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let registry = crate::command_executor::get_registry();
    let topic = ast.arguments.first().map(|a| a.value.as_deref().unwrap_or(a.key.as_str()));

    if let Some(cmd) = topic {
        match crate::command_registry::find_command(registry, cmd) {
            Some(def) => {
                let args: String = def.args.iter().map(|a| {
                    if a.required { format!(" {}=<{}>", a.name, a.name) }
                    else { format!(" [{}=<{}>]", a.name, a.name) }
                }).collect();
                CommandResult::ok(format!(
                    "{}: {}\n  Usage: {}{}\n  Arguments: {}\n",
                    def.name, def.description, def.name, args,
                    if def.args.is_empty() { "none".to_string() } else {
                        def.args.iter().map(|a| format!("{}({:?})", a.name, a.arg_type)).collect::<Vec<_>>().join(", ")
                    }
                ))
            }
            None => CommandResult::ok(format!("Unknown command: {cmd}\nUse \".help\" for complete command list.\n")),
        }
    } else {
        let mut output = String::from("  \x1b[1mAvailable commands:\x1b[0m\n\n");
        for def in registry.iter().filter(|d| !d.dot_command) {
            output.push_str(&format!("  \x1b[32m{:<20}\x1b[0m {}\n", def.name, def.description));
        }
        output.push_str("\n  \x1b[2mUse \".help\" for dot-commands. \"help <cmd>\" for details.\x1b[0m\n");
        CommandResult::ok(output)
    }
}

/// `.help [all | item=<cmd>]` — from registry, format depends on API mode.
pub fn cmd_dot_help(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    if ctx.api_mode == ApiMode::Json {
        return dot_help_json(ast);
    }
    let registry = crate::command_executor::get_registry();
    let args: Vec<&str> = ast.arguments.iter()
        .map(|a| a.value.as_deref().unwrap_or(a.key.as_str()))
        .collect();

    let all = args.contains(&"all");
    let item = args.iter().find(|a| a.starts_with("item="))
        .or_else(|| args.iter().find(|a| **a != "all" && !a.is_empty()));

    if let Some(item) = item {
        let cmd = item.strip_prefix("item=").unwrap_or(item);
        return match crate::command_registry::find_command(registry, cmd) {
            Some(def) => {
                let arg_str: String = def.args.iter()
                    .map(|a| format!("{}=<{:?}>", a.name, a.arg_type))
                    .collect::<Vec<_>>().join(" ");
                let usage = format!("{} {}", def.name, arg_str);
                match ctx.api_mode {
                    ApiMode::Json => CommandResult::ok(format!(
                        "{{\"command\":\"{}\",\"description\":\"{}\",\"usage\":\"{}\",\"arguments\":\"{}\"}}\n",
                        def.name, def.description, usage, arg_str
                    )),
                    ApiMode::Api => CommandResult::ok(format!(
                        "{}\t{}\t{}\t{}\n", def.name, def.description, usage, arg_str
                    )),
                    ApiMode::Human => CommandResult::ok(format!(
                        "  Command:     {}\n  Description: {}\n  Usage:       {}\n  Arguments:   {}\n",
                        def.name, def.description, usage,
                        if def.args.is_empty() { "none".to_string() } else { arg_str }
                    )),
                }
            }
            None => CommandResult::ok(format!("Command '{cmd}' not found.\n")),
        };
    }

    let commands: Vec<_> = if all {
        registry.iter().collect()
    } else {
        registry.iter().filter(|d| d.dot_command).collect()
    };

    let mut output = String::new();
    match ctx.api_mode {
        ApiMode::Json => {
            output.push_str("[\n");
            for (i, def) in commands.iter().enumerate() {
                let arg_str: String = def.args.iter()
                    .map(|a| format!("{}=<{:?}>", a.name, a.arg_type)).collect::<Vec<_>>().join(" ");
                output.push_str(&format!(
                    "  {{\"command\":\"{}\",\"description\":\"{}\",\"arguments\":\"{}\"}}",
                    def.name, def.description, arg_str
                ));
                if i + 1 < commands.len() { output.push(','); }
                output.push('\n');
            }
            output.push_str("]\n");
        }
        ApiMode::Api => {
            // C++ compatible: name\tdescription\tusage\targuments\taudit
            // arguments = comma-separated, !prefix for bare keywords
            for def in &commands {
                use crate::command_registry::{ArgType, CompletionSource};
                let usage: String = {
                    let args: String = def.args.iter().map(|a| format!(" [{}=<{}>]", a.name, a.name)).collect();
                    format!("{}{}", def.name, args)
                };
                let arguments: String = def.args.iter().map(|a| {
                    match a.arg_type {
                        ArgType::Bool | ArgType::String if a.name == "yes" || a.name == "all" || a.name == "done"
                            => format!("!{}", a.name), // bare keyword
                        _ => a.name.to_string(), // key=value arg
                    }
                }).collect::<Vec<_>>().join(",");
                output.push_str(&format!("{}\t{}\t{}\t{}\t{}\n",
                    def.name, def.description, usage, arguments, !def.dot_command));
            }
        }
        ApiMode::Human => {
            output.push_str(&format!("  \x1b[1m{:<22} {}\x1b[0m\n", "Command", "Description"));
            output.push_str(&format!("  {:<22} {}\n", "\u{2500}".repeat(20), "\u{2500}".repeat(50)));
            for def in &commands {
                let color = if def.dot_command { "\x1b[33m" } else { "\x1b[32m" };
                output.push_str(&format!("  {color}{:<22}\x1b[0m {}\n", def.name, def.description));
            }
            output.push_str(&format!("\n  \x1b[2m{} commands.\x1b[0m\n", commands.len()));
        }
    }

    CommandResult::ok(output)
}

/// `.help` in JSON mode, as C++ DotHelpCmd with the Bareos command table:
/// `{"<cmd>": {"command", "description", "arguments", "permission"}}`.
/// `all` lists non-dot commands, `item=<cmd>` one command, `full` ignores ACLs.
fn dot_help_json(ast: &CommandAst) -> CommandResult {
    use crate::ua_commands_table::UA_COMMANDS;
    let has = |k: &str| ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(k));
    if ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("full") && a.value.is_some()) {
        return CommandResult::error("The full option does not take a value.\n".to_string());
    }
    let entry = |(key, help, usage): &(&str, &str, &str)| serde_json::json!({
        "command": key, "description": help, "arguments": usage, "permission": true,
    });
    let mut result = serde_json::Map::new();
    if let Some(item) = ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case("item")).and_then(|a| a.value.as_deref()) {
        if let Some(c) = UA_COMMANDS.iter().find(|c| c.0 == item) {
            result.insert(c.0.to_lowercase(), entry(c));
        }
    } else {
        let all = has("all");
        for c in UA_COMMANDS.iter().filter(|c| !all || !c.0.starts_with('.')) {
            result.insert(c.0.to_lowercase(), entry(c));
        }
    }
    CommandResult::ok(String::new()).with_json(serde_json::Value::Object(result))
}

// Legacy static command tables removed — all data now in command_registry.rs
