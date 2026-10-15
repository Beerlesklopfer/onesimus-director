//! Basic commands: version, time, quit, help, .help
//!
//! Help text and structured metadata follow the upstream Bareos command table.
//! Both help variants render protocol output without terminal styling.

use crate::command_ast::CommandAst;
use crate::command_executor::{ExecContext, ApiMode};
use crate::command_queue::CommandResult;
use crate::version_strings as v;

/// C++ ua_cmds.cc VersionCmd: "<name> Version: <Full> (<Date>) <OsInfo>
/// <BAREOS_PLATFORM> <OsInfo> <CustomVersionId>\n".
pub fn cmd_version(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let os = v::os_info();
    let (name, verid) = ctx.config.as_ref()
        .and_then(|c| c.read().ok().map(|c| {
            // Director "Ver Id" directive (me->verid), only kept in the raw config
            let verid = c.raw.iter()
                .filter(|r| r.norm_type() == "director")
                .flat_map(|r| r.directives.iter())
                .find(|d| d.norm_key() == "verid")
                .and_then(|d| d.first())
                .unwrap_or("")
                .to_string();
            (c.director.name.clone(), verid)
        }))
        .unwrap_or_else(|| ("bareos-dir".to_string(), String::new()));
    let text = format!("{name} Version: {} ({}) {os} {} {os} {verid}\n",
        v::FULL, v::DATE, v::PLATFORM);
    CommandResult::ok(text).with_json(serde_json::json!({
        "version": {
            "name": name,
            "type": "bareos-director",
            "version": v::FULL,
            "bdate": v::DATE,
            "operatingsystem": os,
            "distname": v::PLATFORM,
            "distversion": os,
            "customversionid": verid,
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

/// C++ ua_cmds.cc quit_cmd(): no output, the session loop closes the socket.
pub fn cmd_quit(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    CommandResult::ok(String::new())
}

// Bareos ua_cmds.cc help_cmd / DotHelpCmd use the upstream command table,
// not the completion registry. ACL filtering is still a Director-wide gap:
// until session ACLs exist, these objects describe the unrestricted console.
const HELP_HEADER: &str = "  Command            Description\n  =======            ===========\n";
const HELP_FOOTER: &str = "\nWhen at a prompt, entering a period (.) cancels the command.\n\n";

type HelpEntry = (&'static str, &'static str, &'static str);

fn description(entry: &HelpEntry) -> &str {
    // The generated table represents the upstream .locations NULL pointer as
    // literal "NULL". PoolMem(NULL) renders an empty string in OutputFormatter.
    if entry.0 == ".locations" && entry.1 == "NULL" { "" } else { entry.1 }
}

fn usage(entry: &HelpEntry) -> &'static str {
    if entry.0 == "configure" { super::configure_usage::USAGE } else { entry.2 }
}

fn help_object(entry: &HelpEntry, arguments: String, permission:bool) -> serde_json::Value {
    serde_json::json!({
        "command": entry.0, "description": description(entry),
        "arguments": arguments, "permission": permission,
    })
}

/// Port of OutputFormatter::rewrap. Width counts bytes, as in the C++ loop;
/// brackets prevent wrapping within optional arguments and placeholders.
fn rewrap_help(value: &str, width: usize, human: bool) -> String {
    let mut output = Vec::with_capacity(value.len() * 2);
    let mut open: i32 = 0;
    let mut chars_in_line = 0;
    let bytes = value.as_bytes();
    for (i, &byte) in bytes.iter().enumerate() {
        chars_in_line += 1;
        let next = bytes.get(i + 1).copied();
        match byte {
            b' ' => {
                if human && width > 0 && chars_in_line >= width && open <= 0 && next != Some(b'|') {
                    output.extend_from_slice(b"\n\t");
                    chars_in_line = 0;
                } else if chars_in_line > 1 {
                    output.push(byte);
                }
            }
            b'|' => {
                output.push(byte);
                if human && width > 0 && open <= 0 {
                    output.extend_from_slice(b"\n\t");
                    chars_in_line = 0;
                }
            }
            b'[' | b'<' => { open += 1; output.push(byte); }
            b']' | b'>' => { open -= 1; output.push(byte); }
            b'\n' | b'\t' => {
                if chars_in_line > 1 && !matches!(next, Some(b'\n' | b'\t' | b' ')) {
                    output.push(b' ');
                }
            }
            _ => output.push(byte),
        }
    }
    String::from_utf8(output).expect("wrapping preserves UTF-8")
}

pub fn cmd_help(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::ua_commands_table::UA_COMMANDS;
    let human = ctx.api_mode == ApiMode::Human;
    let mut text = if human { HELP_HEADER.to_string() } else { String::new() };
    let mut objects = serde_json::Map::new();
    // Upstream argc == 2 selects a topic; extra arguments instead list commands.
    let topic = if ast.arguments.len() == 1 { Some(ast.arguments[0].key.as_str()) } else { None };
    let selected: Vec<_> = match topic {
        Some(name) => UA_COMMANDS.iter().filter(|c| c.0.eq_ignore_ascii_case(name)).collect(),
        None => UA_COMMANDS.iter().filter(|c| !c.0.starts_with('.')).collect(),
    };
    if let Some(topic) = topic.filter(|_| selected.is_empty()) {
        // SendMsg is an immediate informational message, including in API mode.
        text.push_str(&format!("\nCan't find {topic} command.\n\n"));
    }
    for entry in selected {
        let permission=crate::acl::access(ctx,crate::acl::Kind::Command,entry.0);
        if topic.is_none() && !permission {continue;}
        let arguments = rewrap_help(usage(entry), if topic.is_some() { 45 } else { 0 }, human);
        if ctx.api_mode != ApiMode::Json {
            text.push_str(&format!("  {:<18} {}\n", entry.0, description(entry)));
            if topic.is_some() {
                text.push_str(&format!("\nArguments:\n\t{arguments}\n"));
            }
        }
        objects.insert(entry.0.to_lowercase(), help_object(entry, arguments, permission));
    }
    if human { text.push_str(HELP_FOOTER); }
    CommandResult::ok(text).with_json(serde_json::Value::Object(objects))
}

pub fn cmd_dot_help(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::ua_commands_table::UA_COMMANDS;
    if ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case("full")).is_some_and(|a| a.value.is_some()) {
        return CommandResult::error("The full option does not take a value.\n".into());
    }
    let item = ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case("item"))
        .and_then(|a| a.value.as_deref());
    let full=ast.arguments.iter().any(|a|a.key.eq_ignore_ascii_case("full"));
    let all = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("all"));
    let selected: Vec<_> = match item {
        Some(name) => UA_COMMANDS.iter().filter(|c| c.0 == name).collect(),
        None => UA_COMMANDS.iter().filter(|c| !all || !c.0.starts_with('.')).collect(),
    };
    let mut text = String::new();
    let mut objects = serde_json::Map::new();
    for entry in selected {
        let permission=crate::acl::access(ctx,crate::acl::Kind::Command,entry.0);
        if item.is_none() && !full && !permission {continue;}
        let arguments = rewrap_help(usage(entry), 0, false);
        if ctx.api_mode != ApiMode::Json {
            if item.is_some() {
                text.push_str(&format!("{arguments}\n"));
            } else if all {
                text.push_str(&format!("{}\n", entry.0));
            } else {
                text.push_str(&format!("{} {} -- {arguments}\n", entry.0, description(entry)));
            }
        }
        objects.insert(entry.0.to_lowercase(), help_object(entry, arguments, permission));
    }
    CommandResult::ok(text).with_json(serde_json::Value::Object(objects))
}

#[cfg(test)]
mod help_tests {
    use super::*;
    use crate::ua_commands_table::UA_COMMANDS;

    fn execute(command: &str, api_mode: ApiMode) -> CommandResult {
        let ctx = ExecContext { session_id: 0, config: None, session_mgr: None,
            catalog: None, api_mode, bus_rx: None, job_manager: None };
        let ast = crate::session::parse_cli_command(command);
        if ast.command_name == "help" { cmd_help(&ast, &ctx) } else { cmd_dot_help(&ast, &ctx) }
    }

    #[test]
    fn help_topic_uses_source_layout_exact_case_insensitive_name_and_mode() {
        let human = execute("help VeRsIoN", ApiMode::Human);
        assert_eq!(human.text_output,
            "  Command            Description\n  =======            ===========\n  version            Print Director version\n\nArguments:\n\t\n\nWhen at a prompt, entering a period (.) cancels the command.\n\n");
        assert_eq!(execute("help version", ApiMode::Api).text_output,
            "  version            Print Director version\n\nArguments:\n\t\n");
        let json = execute("help version", ApiMode::Json);
        assert_eq!(json.text_output, "");
        assert_eq!(json.json.unwrap(), serde_json::json!({"version": {
            "command": "version", "description": "Print Director version", "arguments": "", "permission": true
        }}));
        assert!(execute("help ver", ApiMode::Human).text_output.contains("Can't find ver command."));
        assert!(!execute("help ver", ApiMode::Human).text_output.contains("Print Director version"));
        assert_eq!(execute("help unknown", ApiMode::Api).text_output, "\nCan't find unknown command.\n\n");
    }

    #[test]
    fn help_lists_main_commands_in_upstream_order_with_extra_args() {
        let expected: String = UA_COMMANDS.iter().filter(|c| !c.0.starts_with('.'))
            .map(|c| format!("  {:<18} {}\n", c.0, description(c))).collect();
        assert_eq!(execute("help", ApiMode::Api).text_output, expected);
        assert_eq!(execute("help run ignored", ApiMode::Api).text_output, expected);
        assert_eq!(execute("help", ApiMode::Human).text_output, format!("{HELP_HEADER}{expected}{HELP_FOOTER}"));
        let json = execute("help", ApiMode::Json);
        assert_eq!(json.text_output, "");
        assert_eq!(json.json.unwrap().as_object().unwrap().len(), UA_COMMANDS.iter().filter(|c| !c.0.starts_with('.')).count());
    }

    #[test]
    fn dot_help_variants_have_upstream_selection_and_text_order() {
        let all: String = UA_COMMANDS.iter().filter(|c| !c.0.starts_with('.')).map(|c| format!("{}\n", c.0)).collect();
        let every: String = UA_COMMANDS.iter().map(|c| format!("{} {} -- {}\n", c.0, description(c), rewrap_help(usage(c), 0, false))).collect();
        for mode in [ApiMode::Human, ApiMode::Api] {
            assert_eq!(execute(".help", mode).text_output, every);
            assert_eq!(execute(".help full", mode).text_output, every);
            assert_eq!(execute(".help all", mode).text_output, all);
            assert_eq!(execute(".help all full", mode).text_output, all);
            assert_eq!(execute(".help item=version all", mode).text_output, "\n");
            assert_eq!(execute(".help item=VERSION", mode).text_output, "");
            assert_eq!(execute(".help item=ver", mode).text_output, "");
            assert_eq!(execute(".help item item=version", mode).text_output, every);
            assert_eq!(execute(".help full full=yes", mode).text_output, every);
        }
    }

    #[test]
    fn dot_help_json_and_invalid_full_are_consistent_in_all_modes() {
        for mode in [ApiMode::Human, ApiMode::Api, ApiMode::Json] {
            let invalid = execute(".help full=yes item=version", mode);
            assert!(!invalid.success);
            assert_eq!(invalid.error_message, "The full option does not take a value.\n");
        }
        for command in [".help", ".help full", ".help all", ".help all full", ".help item=version", ".help item=VERSION"] {
            let result = execute(command, ApiMode::Json);
            assert!(result.success);
            assert_eq!(result.text_output, "");
            let object = result.json.unwrap();
            let object = object.as_object().unwrap();
            if command.ends_with("item=VERSION") { assert!(object.is_empty()); }
            else if command.ends_with("item=version") { assert_eq!(object.len(), 1); }
            else if command.contains("all") { assert!(object.keys().all(|k| !k.starts_with('.'))); }
            else { assert_eq!(object.len(), UA_COMMANDS.len()); }
            for (name, entry) in object {
                assert_eq!(entry["command"], *name);
                assert_eq!(entry["permission"], true);
                assert!(entry["arguments"].is_string());
                assert!(entry["description"].is_string());
            }
        }
        assert_eq!(execute(".help item=.locations", ApiMode::Json).json.unwrap()[".locations"]["description"], "");
    }

    #[test]
    fn source_rewrap_preserves_brackets_and_wraps_top_level_alternatives() {
        assert_eq!(rewrap_help("a | b [x | y] <p | q> z", 45, true), "a |\n\tb [x | y] <p | q> z");
        assert_eq!(rewrap_help("  a\n\t b", 0, false), " a b");
        assert_eq!(rewrap_help("a ".repeat(24).as_str(), 45, true), format!("{}\n\ta ", "a ".repeat(22) + "a"));
        let run = UA_COMMANDS.iter().find(|c| c.0 == "run").unwrap();
        let human = execute("help run", ApiMode::Human);
        assert!(human.text_output.contains(&rewrap_help(run.2, 45, true)));
        assert!(execute("help run", ApiMode::Api).text_output.contains(run.2));
        assert_eq!(execute("help run", ApiMode::Json).json.unwrap()["run"]["arguments"], run.2);
    }

    #[test]
    fn configure_help_uses_generated_resource_syntax_in_every_variant() {
        let original = super::super::configure_usage::USAGE;
        assert!(original.starts_with("add client name=<NAME>"));
        assert!(original.ends_with("export client=<client>"));
        for command in ["help configure", ".help item=configure", ".help", ".help all"] {
            let result=execute(command,ApiMode::Json);
            assert_eq!(result.json.unwrap()["configure"]["arguments"],rewrap_help(original,0,false));
        }
        let text=execute(".help item=configure",ApiMode::Api).text_output;
        assert_eq!(text,format!("{}\n",rewrap_help(original,0,false)));
        assert!(!text.contains("GetUsageStringForConsoleConfigureCommand"));
        assert!(execute("help configure",ApiMode::Human).text_output.contains("export client=<client>"));
    }

    #[test]
    fn help_protocol_responses_have_no_ansi_styling() {
        for command in ["help", ".help", "help run", ".help item=run"] {
            let result = execute(command, ApiMode::Human);
            assert!(!result.text_output.contains('\x1b'), "{command}");
            assert!(!result.text_output.is_empty());
        }
    }
}
