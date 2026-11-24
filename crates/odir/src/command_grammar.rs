// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The console grammar: every registered command as a tree of subcommand
//! words, bare keywords and `key=value` arguments (docs/plans/OCONSOLE-COMPLETION.md).
//!
//! The command registry ([`crate::command_registry`]) names the commands and
//! their handlers; [`crate::grammar_table`] describes what each command
//! accepts. Together they are the single source of the console grammar,
//! published to clients by the dot-command `.grammar` (Onesimus extension):
//! API mode 2 returns the JSON of [`grammar_json`] (schema [`GRAMMAR_SCHEMA`]),
//! other modes a readable tree ([`grammar_text`]). The same tree feeds the
//! man pages ([`crate::manpages`]) and the validator ([`validate`]) that keeps
//! the grammar complete against the parity cases.
//!
//! Semantics a client relies on:
//! - A node is a command or a subcommand word. A subcommand word may carry a
//!   value of its own (`list jobid=5`, `status client=fd`, `purge volume[=v]`):
//!   [`Node::value`].
//! - The arguments of a node stay valid below it (`list ... limit=5` after
//!   any `list` subcommand): the arguments in scope are those of every node
//!   on the path from the command to the current subcommand.
//! - Arguments of the same [`GrammarArg::group`] exclude each other; a
//!   [`GrammarArg::repeatable`] key may be given several times; a
//!   [`GrammarArg::list`] value holds several values separated by commas.
//! - [`GrammarArg::required`] arguments are completed first. The Director
//!   asks for most missing values interactively, so a line without them is
//!   not refused by the grammar ([`validate`]).

#![warn(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::command_registry::CommandDef;

/// Version of the `.grammar` JSON; increases only on breaking changes
/// (clients ignore unknown fields).
pub const GRAMMAR_SCHEMA: u32 = 1;

/// Value source of arguments that name a console command (`help <command>`,
/// `.help item=`, `.grammar command=`): the client takes the command words of
/// the grammar itself.
pub const COMMANDS_SOURCE: &str = "@commands";

/// Bare word (`dir`, `all`, `yes`, `dryrun`) or `key=value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    /// A bare word without `=`.
    Keyword,
    /// `key=value` (or a positional value, see [`GrammarArg::positional`]).
    Value,
}

impl ArgKind {
    /// The JSON spelling (`"keyword"`, `"value"`).
    pub fn json_name(self) -> &'static str {
        match self {
            ArgKind::Keyword => "keyword",
            ArgKind::Value => "value",
        }
    }
}

/// Type of a value, for clients and the man pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgType {
    /// Free-form text.
    String,
    /// A number.
    Integer,
    /// `yes` or `no`.
    Bool,
    /// One of a fixed set of values ([`Values::Fixed`]).
    Enum,
    /// A resource or catalog name whose values come from a dot-command.
    Resource,
}

impl ArgType {
    /// The JSON spelling.
    pub fn json_name(self) -> &'static str {
        match self {
            ArgType::String => "string",
            ArgType::Integer => "integer",
            ArgType::Bool => "bool",
            ArgType::Enum => "enum",
            ArgType::Resource => "resource",
        }
    }
}

/// Where the values of an argument come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Values {
    /// No completion values.
    None,
    /// A fixed list.
    Fixed(Vec<String>),
    /// The output lines of a dot-command (`.clients`, `.pools`, ...), or
    /// [`COMMANDS_SOURCE`] for command names.
    Source(String),
}

/// Who defined a node: the Bareos console grammar or an Onesimus extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A Bareos console command (same grammar as bconsole).
    Bareos,
    /// An Onesimus extension (`.acl`, `purge volume all`, `configure update`, `.ovfs_*`, ...).
    Onesimus,
}

impl Origin {
    /// The JSON spelling (`"bareos"`, `"onesimus"`).
    pub fn json_name(self) -> &'static str {
        match self {
            Origin::Bareos => "bareos",
            Origin::Onesimus => "onesimus",
        }
    }
}

/// One argument of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarArg {
    /// Key or keyword; for a positional value the placeholder shown in usage lines.
    pub name: String,
    /// Bare word or `key=value`.
    pub kind: ArgKind,
    /// Type of the value.
    pub arg_type: ArgType,
    /// Completed first; the Director asks for it when it is missing.
    pub required: bool,
    /// The key may be given several times (`.acl set jobacl=a jobacl=b`).
    pub repeatable: bool,
    /// Arguments of the same group exclude each other (`pool`/`allpools`, `dryrun`/`yes`).
    pub group: Option<String>,
    /// Several values separated by commas inside one value (`volstatus=Full,Used`).
    pub list: bool,
    /// A value without a key (`.api 2`, `help <command>`).
    pub positional: bool,
    /// The key may also stand alone without `=value` (`.acl delete jobacl`).
    pub value_optional: bool,
    /// Default the Director uses when the argument is missing.
    pub default: Option<String>,
    /// One-line description.
    pub description: String,
    /// Completion values.
    pub values: Values,
}

/// A command or a subcommand word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The word as typed (lower case).
    pub word: String,
    /// Other words for the same node (`llist`, `exit`, `dir`).
    pub aliases: Vec<String>,
    /// One-line description.
    pub description: String,
    /// A dot-command (machine interface).
    pub dot: bool,
    /// Bareos grammar or Onesimus extension.
    pub origin: Origin,
    /// The word's own value (`jobid=<jobid>`): its name is the placeholder;
    /// `value_optional` allows the bare word as well.
    pub value: Option<Box<GrammarArg>>,
    /// DESCRIPTION paragraphs of the man page and `help <command>`.
    pub long: Vec<String>,
    /// Example lines with a one-line explanation.
    pub examples: Vec<(String, String)>,
    /// Related commands (command words).
    pub see_also: Vec<String>,
    /// The rest of the line is free text (`var`): every word is accepted.
    pub free_text: bool,
    /// Arguments; they stay valid in every subcommand below this node.
    pub args: Vec<GrammarArg>,
    /// Subcommand words.
    pub subs: Vec<Node>,
}

// --- Builders: keep the grammar table readable ------------------------------------

/// An optional, single argument; the builders below set the kind and type.
fn arg(name: &str, kind: ArgKind, arg_type: ArgType, description: &str, values: Values) -> GrammarArg {
    GrammarArg {
        name: name.to_string(),
        kind,
        arg_type,
        required: false,
        repeatable: false,
        group: None,
        list: false,
        positional: false,
        value_optional: false,
        default: None,
        description: description.to_string(),
        values,
    }
}

/// A bare keyword (`all`, `yes`).
pub fn kw(name: &str, description: &str) -> GrammarArg {
    arg(name, ArgKind::Keyword, ArgType::String, description, Values::None)
}

/// `key=<text>`.
pub fn val(name: &str, description: &str) -> GrammarArg {
    arg(name, ArgKind::Value, ArgType::String, description, Values::None)
}

/// `key=<number>`.
pub fn int(name: &str, description: &str) -> GrammarArg {
    arg(name, ArgKind::Value, ArgType::Integer, description, Values::None)
}

/// `key=yes|no`.
pub fn boolean(name: &str, description: &str) -> GrammarArg {
    arg(name, ArgKind::Value, ArgType::Bool, description, Values::Fixed(vec!["yes".into(), "no".into()]))
}

/// `key=<name>` with the values of a dot-command.
pub fn res(name: &str, description: &str, source: &str) -> GrammarArg {
    arg(name, ArgKind::Value, ArgType::Resource, description, Values::Source(source.to_string()))
}

/// `key=<one of values>`.
pub fn enm(name: &str, description: &str, values: &[&str]) -> GrammarArg {
    arg(name, ArgKind::Value, ArgType::Enum, description,
        Values::Fixed(values.iter().map(|v| v.to_string()).collect()))
}

/// A positional value without key; `name` is its placeholder.
pub fn pos(name: &str, description: &str) -> GrammarArg {
    val(name, description).positional()
}

impl GrammarArg {
    /// Mark as required.
    pub fn req(mut self) -> Self {
        self.required = true;
        self
    }

    /// Mark as repeatable.
    pub fn repeat(mut self) -> Self {
        self.repeatable = true;
        self
    }

    /// Mark the value as a comma-separated list.
    pub fn list(mut self) -> Self {
        self.list = true;
        self
    }

    /// Put into an exclusion group.
    pub fn group(mut self, group: &str) -> Self {
        self.group = Some(group.to_string());
        self
    }

    /// Mark as positional (a value without key).
    pub fn positional(mut self) -> Self {
        self.positional = true;
        self.kind = ArgKind::Value;
        self
    }

    /// The key may stand alone without a value.
    pub fn optional_value(mut self) -> Self {
        self.value_optional = true;
        self
    }

    /// Default value.
    pub fn default(mut self, value: &str) -> Self {
        self.default = Some(value.to_string());
        self
    }

    /// Completion values from a fixed list (type Enum).
    pub fn values(mut self, values: &[&str]) -> Self {
        self.values = Values::Fixed(values.iter().map(|v| v.to_string()).collect());
        self.arg_type = ArgType::Enum;
        self
    }

    /// Completion values from a dot-command (type Resource).
    pub fn source(mut self, source: &str) -> Self {
        self.values = Values::Source(source.to_string());
        self.arg_type = ArgType::Resource;
        self
    }
}

/// A subcommand word.
pub fn sub(word: &str, description: &str) -> Node {
    Node {
        word: word.to_string(),
        aliases: Vec::new(),
        description: description.to_string(),
        dot: word.starts_with('.'),
        origin: Origin::Bareos,
        value: None,
        long: Vec::new(),
        examples: Vec::new(),
        see_also: Vec::new(),
        free_text: false,
        args: Vec::new(),
        subs: Vec::new(),
    }
}

/// A subcommand word with its own value `word=<value>` (`value` describes it).
pub fn vsub(word: &str, description: &str, value: GrammarArg) -> Node {
    sub(word, description).value(value)
}

impl Node {
    /// Set the arguments.
    pub fn args(mut self, args: Vec<GrammarArg>) -> Self {
        self.args = args;
        self
    }

    /// Set the subcommands.
    pub fn subs(mut self, subs: Vec<Node>) -> Self {
        self.subs = subs;
        self
    }

    /// Add other words for this node.
    pub fn alias(mut self, aliases: &[&str]) -> Self {
        self.aliases.extend(aliases.iter().map(|a| a.to_string()));
        self
    }

    /// The word's own value.
    pub fn value(mut self, mut value: GrammarArg) -> Self {
        value.kind = ArgKind::Value;
        self.value = Some(Box::new(value));
        self
    }

    /// Mark as Onesimus extension.
    pub fn onesimus(mut self) -> Self {
        self.origin = Origin::Onesimus;
        self
    }

    /// DESCRIPTION paragraphs.
    pub fn long(mut self, paragraphs: &[&str]) -> Self {
        self.long.extend(paragraphs.iter().map(|p| p.to_string()));
        self
    }

    /// Add an example line with its explanation.
    pub fn example(mut self, line: &str, explanation: &str) -> Self {
        self.examples.push((line.to_string(), explanation.to_string()));
        self
    }

    /// Related commands.
    pub fn see(mut self, commands: &[&str]) -> Self {
        self.see_also.extend(commands.iter().map(|c| c.to_string()));
        self
    }

    /// The rest of the line is free text, shown as one repeatable positional value.
    pub fn free_text(mut self, name: &str, description: &str) -> Self {
        self.free_text = true;
        self.args = vec![pos(name, description).repeat()];
        self
    }

    /// Does `word` name this node (word or alias, case-insensitive)?
    pub fn is_named(&self, word: &str) -> bool {
        self.word.eq_ignore_ascii_case(word) || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(word))
    }

    /// Every node of the subtree, this one first (depth first).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn walk(&self) -> Vec<&Node> {
        let mut out = vec![self];
        for sub in &self.subs {
            out.extend(sub.walk());
        }
        out
    }
}

// --- The grammar of the registry -------------------------------------------------

/// The root node of every registered command, in registry order: word,
/// aliases and dot flag from the registry; description from the Bareos
/// command table where the command is a Bareos command, else the registry;
/// origin Onesimus for commands Bareos does not know.
pub fn roots_for(registry: &[CommandDef]) -> Vec<Node> {
    registry.iter().map(|command| {
        let mut node = crate::grammar_table::describe(command.name).unwrap_or_else(|| sub(command.name, ""));
        node.word = command.name.to_string();
        node.aliases = command.aliases.iter().map(|a| a.to_string()).collect();
        node.dot = command.dot_command;
        let bareos = crate::ua_commands_table::UA_COMMANDS.iter()
            .find(|entry| entry.0 == command.name || command.aliases.contains(&entry.0));
        if node.description.is_empty() {
            node.description = match bareos {
                Some(entry) if entry.1 != "NULL" => entry.1.to_string(),
                _ => command.description.to_string(),
            };
        }
        if bareos.is_none() {
            node.origin = Origin::Onesimus;
        }
        crate::grammar_docs::apply(&mut node);
        node
    }).collect()
}

/// The grammar of the Director's registry (built once).
pub fn roots() -> &'static [Node] {
    static ROOTS: std::sync::OnceLock<Vec<Node>> = std::sync::OnceLock::new();
    ROOTS.get_or_init(|| roots_for(crate::command_executor::get_registry()))
}

/// The root of `name` (word or alias), as the console resolves it.
pub fn find_root<'a>(roots: &'a [Node], name: &str) -> Option<&'a Node> {
    let registry = crate::command_executor::get_registry();
    let resolved = crate::command_registry::resolve_command_name(registry, name)
        .unwrap_or(name);
    roots.iter().find(|n| n.is_named(resolved))
}

// --- Usage lines -----------------------------------------------------------------

/// The placeholder of a value: `<name>` or, for a short fixed list, the values.
fn placeholder(arg: &GrammarArg) -> String {
    let inner = match &arg.values {
        Values::Fixed(values) if values.len() <= 4 && arg.arg_type != ArgType::Bool => values.join("|"),
        Values::Fixed(_) if arg.arg_type == ArgType::Bool => "yes|no".to_string(),
        _ => arg.name.clone(),
    };
    let mut text = format!("<{inner}>");
    if arg.list {
        text.push_str("[,...]");
    }
    text
}

/// One argument as written in a usage line, without brackets.
pub fn arg_usage(arg: &GrammarArg) -> String {
    let mut text = match (arg.kind, arg.positional) {
        (_, true) => placeholder(arg),
        (ArgKind::Keyword, _) => arg.name.clone(),
        (ArgKind::Value, _) if arg.value_optional => format!("{}[={}]", arg.name, placeholder(arg)),
        (ArgKind::Value, _) => format!("{}={}", arg.name, placeholder(arg)),
    };
    if arg.repeatable {
        text.push_str(" ...");
    }
    text
}

/// A node word as written in a usage line (`jobid=<jobid>`, `volume[=<volume>]`).
pub fn word_usage(node: &Node) -> String {
    match &node.value {
        None => node.word.clone(),
        Some(value) if value.value_optional => format!("{}[={}]", node.word, placeholder(value)),
        Some(value) => format!("{}={}", node.word, placeholder(value)),
    }
}

/// The arguments of a node as a usage fragment: required ones bare, the
/// others in brackets, a group as `[a | b]` (no brackets when a member is
/// required), in table order.
pub fn args_usage(args: &[GrammarArg]) -> String {
    let mut parts = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    for arg in args {
        match &arg.group {
            Some(group) => {
                if done.contains(&group.as_str()) {
                    continue;
                }
                done.push(group);
                let members: Vec<&GrammarArg> = args.iter().filter(|a| a.group.as_deref() == Some(group)).collect();
                let text = members.iter().map(|a| arg_usage(a)).collect::<Vec<_>>().join(" | ");
                parts.push(if members.iter().any(|a| a.required) { text } else { format!("[{text}]") });
            }
            None if arg.required => parts.push(arg_usage(arg)),
            None => parts.push(format!("[{}]", arg_usage(arg))),
        }
    }
    parts.join(" ")
}

/// Usage lines of a node and its subtree: one line per node that can end a
/// command (every node), `prefix` being the words before it.
pub fn usage_lines(node: &Node, prefix: &str) -> Vec<String> {
    let head = if prefix.is_empty() { word_usage(node) } else { format!("{prefix} {}", word_usage(node)) };
    let mut lines = Vec::new();
    let args = args_usage(&node.args);
    let has_own = !node.args.is_empty() || node.subs.is_empty() || node.value.is_some() || prefix.is_empty();
    if has_own {
        lines.push(if args.is_empty() { head.clone() } else { format!("{head} {args}") });
    }
    for sub in &node.subs {
        lines.extend(usage_lines(sub, &head));
    }
    lines
}

// --- JSON (schema 1) -------------------------------------------------------------

/// The JSON of one argument.
pub fn arg_json(arg: &GrammarArg) -> Value {
    let mut object = json!({
        "name": arg.name,
        "kind": arg.kind.json_name(),
        "type": arg.arg_type.json_name(),
        "required": arg.required,
        "repeatable": arg.repeatable,
        "group": arg.group,
        "list": arg.list,
        "positional": arg.positional,
        "value_optional": arg.value_optional,
        "description": arg.description,
    });
    match &arg.values {
        Values::None => {}
        Values::Fixed(values) => {
            object["values"] = json!(values);
        }
        Values::Source(source) => {
            object["source"] = json!(source);
        }
    }
    if let Some(default) = &arg.default {
        object["default"] = json!(default);
    }
    object
}

/// The JSON of one node and its subtree.
pub fn node_json(node: &Node) -> Value {
    json!({
        "word": node.word,
        "aliases": node.aliases,
        "dot": node.dot,
        "origin": node.origin.json_name(),
        "description": node.description,
        "value": node.value.as_deref().map(arg_json),
        "long": node.long,
        "examples": node.examples.iter().map(|(line, text)| json!({"line": line, "explanation": text})).collect::<Vec<_>>(),
        "see_also": node.see_also,
        "free_text": node.free_text,
        "args": node.args.iter().map(arg_json).collect::<Vec<_>>(),
        "subs": node.subs.iter().map(node_json).collect::<Vec<_>>(),
    })
}

/// `{"grammar": {"schema": 1, "commands": [...]}}` of the given roots.
pub fn grammar_json(roots: &[Node]) -> Value {
    json!({"grammar": {"schema": GRAMMAR_SCHEMA, "commands": roots.iter().map(node_json).collect::<Vec<_>>()}})
}

/// Text form: per command a line `word -- description` and its usage lines, indented.
pub fn grammar_text(roots: &[Node]) -> String {
    let mut out = String::new();
    for root in roots {
        out.push_str(&format!("{} -- {}\n", root.word, root.description));
        for line in usage_lines(root, "") {
            out.push_str(&format!("    {line}\n"));
        }
    }
    out
}

// --- ACL filter ------------------------------------------------------------------

/// The roots a console may run: `permitted(acl_name)` decides per word with
/// the name the dispatcher checks (Bareos table name, else the registry
/// name). A root whose own word is denied but an alias allowed is published
/// under that alias; denied aliases are dropped.
pub fn permitted_roots(roots: &[Node], permitted: impl Fn(&str) -> bool) -> Vec<Node> {
    let registry = crate::command_executor::get_registry();
    roots.iter().filter_map(|root| {
        let main = registry.iter().find(|c| c.name == root.word).map(|c| c.name).unwrap_or(&root.word);
        let acl_name = |word: &str| -> String {
            if crate::ua_commands_table::UA_COMMANDS.iter().any(|e| e.0 == word) { word.to_string() } else { main.to_string() }
        };
        let check = |word: &str| matches!(word, "exit" | "quit" | "whoami") || permitted(&acl_name(word));
        let words: Vec<String> = std::iter::once(root.word.clone()).chain(root.aliases.iter().cloned())
            .filter(|w| check(w)).collect();
        let (first, rest) = words.split_first()?;
        let mut node = root.clone();
        node.word = first.clone();
        node.aliases = rest.to_vec();
        Some(node)
    }).collect()
}

// --- Validation: keeps the grammar complete --------------------------------------

/// Does `line` fit the grammar? `Err` names the first argument that does not.
///
/// Like Bareos, the subcommand words may stand anywhere after the command
/// (FindArgKeyword searches every argument): the first pass walks into a
/// subcommand whenever an argument names one of the current node; the
/// second checks every other argument against the arguments in scope: known
/// key or keyword, a value where one is needed and none on a keyword,
/// groups exclusive, non-repeatable keys once; bare words and `=value`
/// without key fill the positional arguments. Values are not checked, nor
/// are required arguments (the Director asks for them).
#[cfg_attr(not(test), allow(dead_code))]
pub fn validate(roots: &[Node], line: &str) -> Result<(), String> {
    let ast = crate::session::parse_cli_command(line);
    if ast.command_name.is_empty() {
        return Ok(());
    }
    let root = find_root(roots, &ast.command_name)
        .ok_or_else(|| format!("{}: unknown command", ast.command_name))?;
    let mut path = vec![root];
    let mut consumed = vec![false; ast.arguments.len()];
    for (i, argument) in ast.arguments.iter().enumerate() {
        let current = path[path.len() - 1];
        let found = current.subs.iter().find(|s| {
            s.is_named(&argument.key) && match (&s.value, &argument.value) {
                (None, None) => true,
                (Some(_), Some(_)) => true,
                (Some(v), None) => v.value_optional,
                (None, Some(_)) => false,
            }
        });
        if let Some(sub) = found {
            path.push(sub);
            consumed[i] = true;
        }
    }
    if path.iter().any(|n| n.free_text) {
        return Ok(());
    }
    let scope: Vec<&GrammarArg> = path.iter().flat_map(|n| n.args.iter()).collect();
    let mut used: HashMap<&str, usize> = HashMap::new();
    let mut groups: HashMap<&str, &str> = HashMap::new();
    for (i, argument) in ast.arguments.iter().enumerate() {
        if consumed[i] {
            continue;
        }
        let shown = match &argument.value {
            Some(v) => format!("{}={v}", argument.key),
            None => argument.key.clone(),
        };
        let named = (!argument.key.is_empty()).then(|| scope.iter()
            .find(|a| !a.positional && a.name.eq_ignore_ascii_case(&argument.key))).flatten();
        let arg = match named {
            Some(arg) => {
                match (arg.kind, &argument.value) {
                    (ArgKind::Keyword, Some(_)) => return Err(format!("{shown}: {} takes no value", arg.name)),
                    (ArgKind::Value, None) if !arg.value_optional => return Err(format!("{shown}: {} needs a value", arg.name)),
                    _ => {}
                }
                *arg
            }
            None if argument.key.is_empty() || argument.value.is_none() => {
                match scope.iter().find(|a| a.positional && (a.repeatable || !used.contains_key(a.name.as_str()))) {
                    Some(arg) => *arg,
                    None => return Err(format!("{shown}: unknown word")),
                }
            }
            None => return Err(format!("{shown}: unknown key")),
        };
        let count = used.entry(arg.name.as_str()).or_insert(0);
        *count += 1;
        if *count > 1 && !arg.repeatable && !arg.positional {
            return Err(format!("{shown}: {} given twice", arg.name));
        }
        if let Some(group) = &arg.group {
            match groups.get(group.as_str()) {
                Some(other) if *other != arg.name => return Err(format!("{shown}: excludes {other}")),
                _ => {
                    groups.insert(group, &arg.name);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_roots() -> &'static [Node] {
        roots()
    }

    #[test]
    fn grammar_covers_every_registered_command() {
        let roots = registry_roots();
        let registry = crate::command_executor::get_registry();
        assert_eq!(roots.len(), registry.len());
        for command in registry {
            let root = roots.iter().find(|n| n.word == command.name)
                .unwrap_or_else(|| panic!("{} has no grammar", command.name));
            assert!(!root.description.is_empty(), "{}", command.name);
            assert!(crate::grammar_table::describe(command.name).is_some(), "{} has no entry in grammar_table", command.name);
        }
        for root in roots {
            for node in root.walk() {
                let mut names: Vec<String> = node.args.iter().filter(|a| !a.positional)
                    .map(|a| a.name.to_ascii_lowercase()).collect();
                names.sort();
                let before = names.len();
                names.dedup();
                assert_eq!(before, names.len(), "{} {}: an argument name twice", root.word, node.word);
                for a in &node.args {
                    assert!(!a.name.is_empty() && !a.description.is_empty(), "{} {}: {:?}", root.word, node.word, a.name);
                    assert_eq!(a.arg_type == ArgType::Enum, matches!(a.values, Values::Fixed(_)) && a.arg_type != ArgType::Bool,
                        "{} {} {}: Enum iff fixed values", root.word, node.word, a.name);
                }
            }
        }
    }

    #[test]
    fn grammar_json_is_stable() {
        let roots = registry_roots();
        let pick: Vec<Node> = ["purge", ".acl", "list", ".grammar"].iter()
            .map(|w| roots.iter().find(|n| n.word == *w).unwrap().clone()).collect();
        let actual = serde_json::to_string_pretty(&grammar_json(&pick)).unwrap() + "\n";
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/grammar_snapshot.json");
        if std::env::var_os("UPDATE_GRAMMAR_SNAPSHOT").is_some() {
            std::fs::write(path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(path).unwrap_or_default();
        assert!(expected == actual, "grammar snapshot changed; check and rerun with UPDATE_GRAMMAR_SNAPSHOT=1");
    }

    /// docs/wiki/commands/console-grammar.json: the full grammar (no ACL
    /// filter), read by scripts/generate_director_reference.py for the
    /// declared arguments of each command.
    #[test]
    fn console_grammar_doc_is_current() {
        let actual = serde_json::to_string_pretty(&grammar_json(registry_roots())).unwrap() + "\n";
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/wiki/commands/console-grammar.json");
        if std::env::var_os("UPDATE_GRAMMAR_SNAPSHOT").is_some() {
            std::fs::write(path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(path).unwrap_or_default();
        assert!(expected == actual, "docs/wiki/commands/console-grammar.json outdated; rerun with UPDATE_GRAMMAR_SNAPSHOT=1");
    }

    #[test]
    fn purge_volume_all_in_json() {
        let json = grammar_json(registry_roots());
        let purge = json["grammar"]["commands"].as_array().unwrap().iter().find(|c| c["word"] == "purge").unwrap();
        let volume = purge["subs"].as_array().unwrap().iter().find(|s| s["word"] == "volume").unwrap();
        let args = volume["args"].as_array().unwrap();
        let find = |n: &str| args.iter().find(|a| a["name"] == n).unwrap_or_else(|| panic!("{n}"));
        assert_eq!(find("all")["kind"], "keyword");
        assert_eq!(find("pool")["group"], "scope");
        assert_eq!(find("allpools")["group"], "scope");
        assert_eq!(find("volstatus")["list"], true);
        assert_eq!(find("volstatus")["values"][0], "Full");
        assert_eq!(find("dryrun")["group"], "mode");
        assert_eq!(volume["value"]["source"], ".media");
        assert_eq!(json["grammar"]["schema"], 1);
    }

    /// The console lines of tests/parity/cases/*.jsonl the Director accepts
    /// as commands: every `setup` and `steps` string whose first word the
    /// Director resolves to a registered command (`is an invalid command`
    /// otherwise). Steps that answer a dialog (`yes`, `mod`, `2`, restore
    /// tree commands such as `mark *`) do not resolve and are not console
    /// lines. A line the grammar refuses on purpose is in [`PARITY_REFUSED`].
    #[test]
    fn grammar_accepts_the_parity_cases() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/parity/cases");
        let registry = crate::command_executor::get_registry();
        let roots = registry_roots();
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl")).collect();
        files.sort();
        let (mut accepted, mut allowed) = (0, std::collections::BTreeSet::<String>::new());
        let mut failures = Vec::new();
        for file in files {
            for line in std::fs::read_to_string(&file).unwrap().lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let case: Value = serde_json::from_str(line).unwrap();
                let steps = case["setup"].as_array().into_iter().flatten().chain(case["steps"].as_array().into_iter().flatten());
                for step in steps.filter_map(Value::as_str) {
                    let ast = crate::session::parse_cli_command(step);
                    if ast.command_name.is_empty()
                        || crate::command_registry::resolve_command_name(registry, &ast.command_name).is_none() {
                        continue;
                    }
                    match validate(roots, step) {
                        Ok(()) => accepted += 1,
                        Err(_) if PARITY_REFUSED.iter().any(|(l, _)| *l == step) => {
                            allowed.insert(step.to_string());
                        }
                        Err(e) => failures.push(format!("{step:?}: {e}")),
                    }
                }
            }
        }
        failures.sort();
        failures.dedup();
        assert!(failures.is_empty(), "{} parity lines refused by the grammar:\n{}", failures.len(), failures.join("\n"));
        let stale: Vec<_> = PARITY_REFUSED.iter().filter(|(l, _)| !allowed.contains(*l)).collect();
        assert!(stale.is_empty(), "allow-list entries the grammar accepts or no case uses: {stale:?}");
        assert!(accepted > 2000, "only {accepted} parity lines accepted");
        eprintln!("grammar_accepts_the_parity_cases: {accepted} lines accepted, {} on the allow-list", allowed.len());
    }

    /// Parity lines the grammar refuses on purpose, each with the reason.
    /// They test how the Director treats input outside its grammar (Bareos
    /// ignores most unknown words, asks for missing values, reports errors);
    /// completion must not offer them. The test fails for an entry the
    /// grammar accepts or no case uses any more.
    const PARITY_REFUSED: &[(&str, &str)] = {
        const NO_VALUE: &str = "key without value: the Director asks for the value or reports an error";
        const KEYWORD_VALUE: &str = "keyword with a value: Bareos only looks for the keyword and ignores the value";
        const CONTRADICTION: &str = "contradictory keywords: the Director takes one of them, completion offers only one";
        const TWICE: &str = "key given twice: the Director refuses it or uses the first";
        const UNKNOWN_KEY: &str = "unknown key: the Director ignores it or refuses the line (error case)";
        const UNKNOWN_WORD: &str = "unknown word: the Director ignores it or refuses the line (error case)";
        const TREE: &str = "an answer in the restore file selection (tree command), not a console line";
        const PREFIX_CATALOG: &str = ".catalog is a prefix of .catalogs, which ignores its arguments";
        const PREFIX: &str = "Bareos matches this word as a prefix (status conf..., show resource types)";
        const SQLQUERY: &str = "sqlquery ignores its arguments and reads the statement interactively";
        const LEVELS: &str = ".levels takes one job type as a bare word; anything else lists nothing";
        &[
            // cases c091 to c094 (24.11.2026): keys the original ignores or answers with an error
            (".defaults fileset=SelfTest", UNKNOWN_KEY),
            ("add pool=Full storage=File num=2 name=ParityArg- start=5", UNKNOWN_KEY),
            ("cancel client=nosuch", UNKNOWN_KEY),
            ("help configure add", UNKNOWN_WORD),
            ("update pool=Full maxvols=77", UNKNOWN_KEY),
            ("update pool=Full volretention=45d", UNKNOWN_KEY),
            ("update volume=Full-47321 maxbytes=1G", UNKNOWN_KEY),
            (".acl frobnicate", UNKNOWN_WORD),
            (".api 2 compact", NO_VALUE),
            (".api 2 compact=yes extra", UNKNOWN_WORD),
            (".catalog reset yes confirm=bareos", PREFIX_CATALOG),
            (".catalog validate", PREFIX_CATALOG),
            (".clients disabled=yes", KEYWORD_VALUE),
            (".clients enabled disabled", CONTRADICTION),
            (".defaults job", NO_VALUE),
            (".jobs disabled enabled", CONTRADICTION),
            (".jobs disabled=no", KEYWORD_VALUE),
            (".jobs enabled disabled", CONTRADICTION),
            (".jobstatus T", UNKNOWN_WORD),
            (".jobstatus status=T", UNKNOWN_KEY),
            (".levels Backup Restore", LEVELS),
            (".levels type=B", LEVELS),
            (".levels type=Backup", LEVELS),
            (".levels type=R", LEVELS),
            (".pools type type=Scratch", NO_VALUE),
            (".pools type", NO_VALUE),
            (".schedule enabled disabled", CONTRADICTION),
            (".schedule enabled=1", KEYWORD_VALUE),
            (".sql query", NO_VALUE),
            (".status dir nosuch", UNKNOWN_WORD),
            (".status nosuch", UNKNOWN_WORD),
            (".storages enabled disabled", CONTRADICTION),
            (".storages enabled=x", KEYWORD_VALUE),
            ("a b", TREE),
            ("add sbin/zic", TREE),
            ("autodisplay foo off", UNKNOWN_WORD),
            ("autodisplay on=1", UNKNOWN_KEY),
            ("automount on off", UNKNOWN_WORD),
            ("cancel state", NO_VALUE),
            ("cancel storage", NO_VALUE),
            ("cancel storage=File jobid", NO_VALUE),
            ("configure add console name=c019a password=secret nosuch=1", UNKNOWN_KEY),
            ("configure add console name=c019c password", NO_VALUE),
            ("configure add console name=c019e password=secret description=\"Parity console\" jobacl=*all* commandacl=status,show", UNKNOWN_KEY),
            ("configure add console=c019f password=secret", UNKNOWN_KEY),
            ("configure add device name=x", UNKNOWN_WORD),
            ("configure add director name=x", UNKNOWN_WORD),
            ("configure add nosuch name=x", UNKNOWN_WORD),
            ("configure export client", NO_VALUE),
            ("configure export foo", UNKNOWN_WORD),
            ("configure foo bar", UNKNOWN_WORD),
            ("configure nosuch foo", UNKNOWN_WORD),
            ("configure nosuch", UNKNOWN_WORD),
            ("delete jobid", UNKNOWN_WORD),
            ("delete sbin/", TREE),
            ("disable schedule", NO_VALUE),
            ("enable client", NO_VALUE),
            ("estimate job", NO_VALUE),
            ("estimate job=backup-bareos-fd accurate", NO_VALUE),
            ("estimate job=backup-bareos-fd client", NO_VALUE),
            ("estimate job=backup-bareos-fd fileset", NO_VALUE),
            ("estimate job=backup-bareos-fd level", NO_VALUE),
            ("export jobid=x", UNKNOWN_KEY),
            ("list bogus", UNKNOWN_WORD),
            ("list client", UNKNOWN_WORD),
            ("list job", UNKNOWN_WORD),
            ("list jobid", UNKNOWN_WORD),
            ("list jobs LIMIT=1 ORDER=asc", UNKNOWN_KEY),
            ("list jobs client client=nonexistent limit=1", NO_VALUE),
            ("list jobs job=BackupCatalog limit limit=1", NO_VALUE),
            ("list jobs order=DESC limit=2", UNKNOWN_KEY),
            ("list jobs=abc", UNKNOWN_KEY),
            ("list mediaid", UNKNOWN_WORD),
            ("list mediatypes", UNKNOWN_WORD),
            ("list nosuch", UNKNOWN_WORD),
            ("list poolid", UNKNOWN_WORD),
            ("llist mediatypes", UNKNOWN_WORD),
            ("m /usr", TREE),
            ("messages all", UNKNOWN_WORD),
            ("mount File File", UNKNOWN_WORD),
            ("mount job", NO_VALUE),
            ("mount storage", NO_VALUE),
            ("move job=nosuch", UNKNOWN_KEY),
            ("purge volume=Full-47321 jobid=1", UNKNOWN_KEY),
            ("purge volume=Full-47321 jobstatus=xx", UNKNOWN_KEY),
            ("resolve client", NO_VALUE),
            ("resolve foo=bar", UNKNOWN_KEY),
            ("restore comment= help", UNKNOWN_WORD),
            ("restore comment=a<b help", UNKNOWN_WORD),
            ("restore file", NO_VALUE),
            ("restore fileregex=( help", UNKNOWN_WORD),
            ("restore help", UNKNOWN_WORD),
            ("restore jobid", NO_VALUE),
            ("run job=RestoreFiles bogus=1", UNKNOWN_KEY),
            ("run job=RestoreFiles where", NO_VALUE),
            ("run job=RestoreFiles where=/a where=/b", TWICE),
            ("run job=backup-bareos-fd job=RestoreFiles", TWICE),
            ("setbandwidth limit", NO_VALUE),
            ("setbandwidth limit=1 client", NO_VALUE),
            ("setbandwidth limit=1 storage", NO_VALUE),
            ("setbandwidth limit=1 ujobid", NO_VALUE),
            ("setbandwidth limit=5 jobid=999998 jobid=999999", TWICE),
            ("setdebug level=0 storage", NO_VALUE),
            ("setdevice foo=bar", UNKNOWN_KEY),
            ("setdevice storage storage=File device=nosuchdev autoselect=no", NO_VALUE),
            ("setdevice storage=File device=nosuchdev autoselect=yes bogus=1", UNKNOWN_KEY),
            ("show Fi=LinuxAll", PREFIX),
            ("show POO", PREFIX),
            ("show disabled nosuch", UNKNOWN_WORD),
            ("show help", UNKNOWN_WORD),
            ("show nosuch client=bareos-fd", UNKNOWN_WORD),
            ("show nosuch", UNKNOWN_WORD),
            ("show nosuch=foo", UNKNOWN_KEY),
            ("sqlquery select 1;", SQLQUERY),
            ("status ConfigurationX", PREFIX),
            ("status nosuch", UNKNOWN_WORD),
            ("status sched days days=1", NO_VALUE),
            ("truncate volstatus volstatus=Purged", NO_VALUE),
            ("truncate volstatus=Purged bogus", UNKNOWN_WORD),
            ("truncate volstatus=Purged client=bareos-fd", UNKNOWN_KEY),
            ("truncate volstatus=Purged storage storage=File", NO_VALUE),
            ("update jobid", UNKNOWN_WORD),
            ("update volume=Full-0002 frompool=x", UNKNOWN_KEY),
            ("use catalog", NO_VALUE),
            ("wait foo", UNKNOWN_WORD),
            ("wait jobid", NO_VALUE),
        ]
    };

    #[test]
    fn grammar_respects_command_acl() {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir; Password = x }\n\
             Console { Name = limited; Password = pw; CommandACL = list, status, .grammar }").unwrap();
        let limited = permitted_roots(registry_roots(), |name| {
            crate::acl::allowed(&config, "limited", crate::acl::Kind::Command, name)
        });
        let words: Vec<&str> = limited.iter().map(|n| n.word.as_str()).collect();
        assert!(words.contains(&"list") && words.contains(&"status") && words.contains(&".grammar"));
        assert!(!words.contains(&"purge") && !words.contains(&".acl") && !words.contains(&"configure"));
        // always allowed, as in dispatch
        assert!(words.contains(&"quit") && words.contains(&"whoami"));
        // llist is its own ACL name and not granted
        let list = limited.iter().find(|n| n.word == "list").unwrap();
        assert!(!list.aliases.contains(&"llist".to_string()));
        let json = grammar_json(&limited);
        assert!(!json.to_string().contains("\"word\":\"purge\""));
    }

    #[test]
    fn groups_and_repeatable_keys() {
        let roots = registry_roots();
        assert!(validate(roots, "purge volume all pool=Full dryrun").is_ok());
        assert!(validate(roots, "purge volume all allpools volstatus=Full,Used yes").is_ok());
        assert!(validate(roots, "purge volume all pool=Full allpools dryrun").unwrap_err().contains("excludes"));
        assert!(validate(roots, "purge volume all pool=Full dryrun yes").unwrap_err().contains("excludes"));
        assert!(validate(roots, ".acl set console=c jobacl=a jobacl=b,c").is_ok());
        assert!(validate(roots, ".acl copy from=a to=b").is_ok());
        assert!(validate(roots, ".acl copy from=a from=b").unwrap_err().contains("twice"));
        assert!(validate(roots, ".acl delete console=c jobacl").is_ok());
        assert!(validate(roots, "configure add console name=n password=p profile=a profile=b").is_ok());
        assert!(validate(roots, "list jobs client=x reverse limit=3").is_ok());
        assert!(validate(roots, "list jobs bogus=1").unwrap_err().contains("unknown key"));
        assert!(validate(roots, "status dir").is_ok());
        assert!(validate(roots, ".api 2").is_ok());
        assert!(validate(roots, "help purge").is_ok());
        assert!(validate(roots, "nosuchcommand").is_err());
    }

    /// Every directive of `configure add <type>` and `configure update
    /// <type>=` whose item references a resource completes from the
    /// dot-command of that type, every enum-typed item has its keywords and
    /// every flag yes/no (directive tables of show_items.rs).
    #[test]
    fn configure_directives_have_value_sources() {
        use crate::commands::show_items::Ty;
        let configure = registry_roots().iter().find(|n| n.word == "configure").unwrap();
        let (mut resources, mut enums) = (0, 0);
        for verb in configure.subs.iter().filter(|s| s.word == "add" || s.word == "update") {
            for node in &verb.subs {
                let items = crate::config_check::items_of(&node.word).unwrap_or_else(|| panic!("{}", node.word));
                for arg in &node.args {
                    let item = crate::config_check::item_of(items, &arg.name)
                        .or_else(|| arg.name.strip_prefix("new").and_then(|k| crate::config_check::item_of(items, k)));
                    let Some(item) = item else { continue };
                    let at = format!("configure {} {} {}", verb.word, node.word, arg.name);
                    match item.ty {
                        Ty::Res | Ty::AlistRes => {
                            let code = item.code.unwrap_or_else(|| panic!("{at}: no resource code"));
                            match crate::grammar_table::resource_source(code) {
                                Some(source) => {
                                    assert_eq!(arg.values, Values::Source(source.into()), "{at}");
                                    assert_eq!(arg.arg_type, ArgType::Resource, "{at}");
                                    resources += 1;
                                }
                                None => assert_eq!(code, "R_COUNTER", "{at}: resource without source"),
                            }
                        }
                        Ty::Bool | Ty::Bit => assert_eq!(arg.arg_type, ArgType::Bool, "{at}"),
                        ty => if let Some(values) = crate::commands::show::enum_values(ty) {
                            assert_eq!(arg.arg_type, ArgType::Enum, "{at}");
                            assert_eq!(arg.values, Values::Fixed(values.iter().map(|v| v.to_string()).collect()), "{at}");
                            enums += 1;
                        },
                    }
                }
            }
        }
        assert!(resources > 50 && enums > 15, "{resources} resources, {enums} enums");
        let add_job = &configure.subs[0].subs.iter().find(|n| n.word == "job").unwrap().args;
        let find = |n: &str| add_job.iter().find(|a| a.name == n).unwrap_or_else(|| panic!("{n}"));
        assert_eq!(find("client").values, Values::Source(".clients".into()));
        assert_eq!(find("messages").values, Values::Source(".msgs".into()));
        assert!(matches!(&find("type").values, Values::Fixed(v) if v.contains(&"Backup".to_string())));
        assert!(matches!(&find("level").values, Values::Fixed(v) if v.contains(&"Incremental".to_string())));
    }

    /// Every value source names a registered dot-command.
    #[test]
    fn value_sources_are_registered_commands() {
        let roots = registry_roots();
        for root in roots {
            for node in root.walk() {
                for arg in node.args.iter().chain(node.value.as_deref()) {
                    if let Values::Source(source) = &arg.values {
                        assert!(source == COMMANDS_SOURCE || find_root(roots, source).is_some_and(|r| r.dot),
                            "{} {} {}: unknown source {source}", root.word, node.word, arg.name);
                    }
                }
            }
        }
    }

    #[test]
    fn usage_of_purge_volume() {
        let purge = registry_roots().iter().find(|n| n.word == "purge").unwrap();
        let lines = usage_lines(purge, "");
        let volume = lines.iter().find(|l| l.starts_with("purge volume")).unwrap();
        assert!(volume.contains("[all]"), "{volume}");
        assert!(volume.contains("[pool=<pool> | allpools]"), "{volume}");
        assert!(volume.contains("volstatus=<volstatus>[,...]"), "{volume}");
        assert!(volume.starts_with("purge volume[=<volume>]"), "{volume}");
    }

    #[test]
    fn text_tree_lists_every_command() {
        let text = grammar_text(registry_roots());
        for root in registry_roots() {
            assert!(text.contains(&format!("{} -- ", root.word)), "{}", root.word);
        }
    }
}
