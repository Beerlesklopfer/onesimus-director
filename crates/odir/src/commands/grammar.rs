//! `.grammar` (Onesimus extension): the console grammar for clients
//! ([`crate::command_grammar`]).

#![warn(clippy::missing_docs_in_private_items)]

use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_grammar::{find_root, grammar_json, grammar_text, permitted_roots, roots, Node};
use crate::command_queue::CommandResult;

/// `.grammar [command=<name>]`: the grammar of every command the console may
/// run (CommandACL, both tiers: the check dispatch uses). API mode 2 returns
/// the JSON, other modes the text tree; with `command=` only that command.
pub fn cmd_dot_grammar(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let all = permitted_roots(roots(), |name| crate::acl::access(ctx, crate::acl::Kind::Command, name));
    let wanted = ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case("command"))
        .map(|a| a.value.clone().unwrap_or_default());
    let selected: Vec<Node> = match &wanted {
        None => all,
        Some(name) => match find_root(&all, name) {
            Some(node) => vec![node.clone()],
            None => return CommandResult::error(format!(".grammar: unknown command \"{name}\".\n")),
        },
    };
    let text = match ctx.api_mode {
        ApiMode::Json => String::new(),
        _ => grammar_text(&selected),
    };
    CommandResult::ok(text).with_json(grammar_json(&selected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(api_mode: ApiMode) -> ExecContext {
        ExecContext { session_id: 0, config: None, session_mgr: None, catalog: None, api_mode, bus_rx: None, job_manager: None }
    }

    #[test]
    fn json_in_api_2_and_text_otherwise() {
        let ast = crate::session::parse_cli_command(".grammar");
        let result = cmd_dot_grammar(&ast, &context(ApiMode::Json));
        let json = result.json.clone().expect("json");
        assert_eq!(json["grammar"]["schema"], 1);
        assert!(json["grammar"]["commands"].as_array().unwrap().iter().any(|c| c["word"] == ".grammar"));
        let text = cmd_dot_grammar(&ast, &context(ApiMode::Human)).text_output;
        assert!(text.contains("purge -- Purge records from catalog\n"), "{text}");
        assert!(text.contains("    purge volume[=<volume>] [all]"), "{text}");
    }

    #[test]
    fn one_command_and_unknown_command() {
        let ast = crate::session::parse_cli_command(".grammar command=llist");
        let result = cmd_dot_grammar(&ast, &context(ApiMode::Json));
        let commands = result.json.unwrap()["grammar"]["commands"].as_array().unwrap().clone();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["word"], "list");
        let ast = crate::session::parse_cli_command(".grammar command=nosuch");
        assert!(!cmd_dot_grammar(&ast, &context(ApiMode::Human)).error_message.is_empty());
    }
}

