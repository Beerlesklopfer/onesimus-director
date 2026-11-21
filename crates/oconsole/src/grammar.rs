//! The client side of the console grammar (docs/plans/OCONSOLE-COMPLETION.md):
//! the model read from the Director's `.grammar` (schema 1) or, without it
//! (Bareos, older odir), rebuilt from the `.help` usage lines; completion,
//! the fish-style autosuggestion and `help <command>` as pure functions,
//! testable without a terminal. main.rs's `BareosHelper` keeps a
//! [`Grammar`] and a [`ValueCache`] and calls [`complete`] and [`suggest`]
//! from its Completer and Hinter.

#![warn(clippy::missing_docs_in_private_items)]

use std::collections::HashMap;

use serde_json::Value;

/// Value source of arguments that name a command (`help <command>`): the
/// command words of the grammar itself.
pub const COMMANDS_SOURCE: &str = "@commands";

/// Grammar schema this client understands.
pub const SCHEMA: u64 = 1;

/// Bare word or `key=value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A bare word without `=`.
    Keyword,
    /// `key=value`, or a positional value.
    Value,
}

/// Where the values of an argument come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Values {
    /// No completion values.
    None,
    /// A fixed list.
    Fixed(Vec<String>),
    /// From a dot-command, cached at connect (".clients", ".pools", ...), or [`COMMANDS_SOURCE`].
    Source(String),
}

/// One argument of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    /// Key or keyword; the placeholder of a positional value.
    pub name: String,
    /// Bare word or `key=value`.
    pub kind: Kind,
    /// Value type as the Director names it (`string`, `integer`, `bool`, `enum`, `resource`).
    pub arg_type: String,
    /// Offered first.
    pub required: bool,
    /// May be given several times.
    pub repeatable: bool,
    /// Arguments of the same group exclude each other.
    pub group: Option<String>,
    /// Several values separated by commas inside one value.
    pub list: bool,
    /// A value without key.
    pub positional: bool,
    /// The key may stand alone without `=value`.
    pub value_optional: bool,
    /// Completion values.
    pub values: Values,
    /// One-line description.
    pub description: String,
    /// Default the Director uses.
    pub default: Option<String>,
}

/// A command or a subcommand word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The word as typed.
    pub word: String,
    /// Other words for the same node.
    pub aliases: Vec<String>,
    /// One-line description.
    pub description: String,
    /// A dot-command.
    pub dot: bool,
    /// `bareos` or `onesimus`.
    pub origin: String,
    /// The word's own value (`jobid=<jobid>`).
    pub value: Option<Box<Arg>>,
    /// DESCRIPTION paragraphs.
    pub long: Vec<String>,
    /// Example lines with their explanation.
    pub examples: Vec<(String, String)>,
    /// Related commands.
    pub see_also: Vec<String>,
    /// The rest of the line is free text.
    pub free_text: bool,
    /// Arguments; valid in every subcommand below.
    pub args: Vec<Arg>,
    /// Subcommand words.
    pub subs: Vec<Node>,
    /// The usage string of `.help` (fallback model only).
    pub usage: Option<String>,
}

/// The console grammar of one Director.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grammar {
    /// The commands the console may run.
    pub commands: Vec<Node>,
    /// true: read from `.grammar`; false: rebuilt from `.help` (Bareos, older odir).
    pub typed: bool,
}

/// Values of dot-commands: ".clients" -> ["PDC-fd", ...].
pub type ValueCache = HashMap<String, Vec<String>>;

/// The string field `key` of a JSON object, empty if missing.
fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or("").to_string()
}

/// A JSON array of strings (other elements skipped).
fn strings(value: &Value) -> Vec<String> {
    value.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// One argument of the `.grammar` JSON; None without a name.
fn arg_from_json(value: &Value) -> Option<Arg> {
    let name = value["name"].as_str()?.to_string();
    let values = if let Some(values) = value["values"].as_array() {
        Values::Fixed(values.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
    } else if let Some(source) = value["source"].as_str() {
        Values::Source(source.to_string())
    } else {
        Values::None
    };
    Some(Arg {
        name,
        kind: if value["kind"] == "keyword" { Kind::Keyword } else { Kind::Value },
        arg_type: text(value, "type"),
        required: value["required"].as_bool().unwrap_or(false),
        repeatable: value["repeatable"].as_bool().unwrap_or(false),
        group: value["group"].as_str().map(str::to_string),
        list: value["list"].as_bool().unwrap_or(false),
        positional: value["positional"].as_bool().unwrap_or(false),
        value_optional: value["value_optional"].as_bool().unwrap_or(false),
        values,
        description: text(value, "description"),
        default: value["default"].as_str().map(str::to_string),
    })
}

/// One node of the `.grammar` JSON with its subtree; None without a word.
fn node_from_json(value: &Value) -> Option<Node> {
    Some(Node {
        word: value["word"].as_str()?.to_string(),
        aliases: strings(&value["aliases"]),
        description: text(value, "description"),
        dot: value["dot"].as_bool().unwrap_or(false),
        origin: text(value, "origin"),
        value: value.get("value").filter(|v| !v.is_null()).and_then(arg_from_json).map(Box::new),
        long: strings(&value["long"]),
        examples: value["examples"].as_array().map(|a| a.iter()
            .map(|e| (text(e, "line"), text(e, "explanation"))).collect()).unwrap_or_default(),
        see_also: strings(&value["see_also"]),
        free_text: value["free_text"].as_bool().unwrap_or(false),
        args: value["args"].as_array().map(|a| a.iter().filter_map(arg_from_json).collect()).unwrap_or_default(),
        subs: value["subs"].as_array().map(|a| a.iter().filter_map(node_from_json).collect()).unwrap_or_default(),
        usage: None,
    })
}

impl Node {
    /// Does `word` name this node (word or alias, case-insensitive)?
    pub fn is_named(&self, word: &str) -> bool {
        self.word.eq_ignore_ascii_case(word) || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(word))
    }
}

impl Grammar {
    /// From the reply of `.grammar` in API mode 2 (with or without the JSON-RPC
    /// envelope); None if it is not one (Bareos: "is an invalid command") or
    /// has another schema. Unknown fields are ignored.
    pub fn from_grammar_json(reply: &Value) -> Option<Grammar> {
        let grammar = reply.get("grammar").or_else(|| reply.get("result").and_then(|r| r.get("grammar")))?;
        if grammar["schema"].as_u64() != Some(SCHEMA) {
            return None;
        }
        let commands: Vec<Node> = grammar["commands"].as_array()?.iter().filter_map(node_from_json).collect();
        (!commands.is_empty()).then_some(Grammar { commands, typed: true })
    }

    /// From the lines of `.help` in API mode 1 (Bareos: `name description -- usage`),
    /// one node per command with the keywords and `key=` arguments of its usage.
    pub fn from_help_lines(lines: &[String]) -> Grammar {
        let commands = lines.iter().filter_map(|line| help_node(line)).collect();
        Grammar { commands, typed: false }
    }

    /// The minimal command list when the Director answered neither.
    pub fn minimal() -> Grammar {
        let commands = ["version", "time", "quit", "exit", "help", ".help", "status", "run", "cancel", "list", "reload"]
            .iter().map(|w| plain_node(w, "")).collect();
        Grammar { commands, typed: false }
    }

    /// Every dot-command a value list needs, once, sorted.
    pub fn value_sources(&self) -> Vec<String> {
        fn collect(node: &Node, out: &mut Vec<String>) {
            for arg in node.value.iter().map(|b| b.as_ref()).chain(node.args.iter()) {
                match &arg.values {
                    Values::Source(source) if source.starts_with('.') && !out.contains(source) => out.push(source.clone()),
                    _ => {}
                }
            }
            for sub in &node.subs {
                collect(sub, out);
            }
        }
        let mut out = Vec::new();
        for command in &self.commands {
            collect(command, &mut out);
        }
        out.sort();
        out
    }

    /// The command named `word` (word or alias).
    pub fn command(&self, word: &str) -> Option<&Node> {
        self.commands.iter().find(|c| c.is_named(word))
    }

    /// Number of subcommand words and arguments, for the connect message.
    pub fn element_count(&self) -> usize {
        fn count(node: &Node) -> usize {
            node.args.len() + node.subs.iter().map(|s| 1 + count(s)).sum::<usize>()
        }
        self.commands.iter().map(count).sum()
    }
}

/// A node without arguments and subcommands.
fn plain_node(word: &str, description: &str) -> Node {
    Node {
        word: word.to_string(), aliases: Vec::new(), description: description.to_string(), dot: word.starts_with('.'),
        origin: String::new(), value: None, long: Vec::new(), examples: Vec::new(), see_also: Vec::new(),
        free_text: false, args: Vec::new(), subs: Vec::new(), usage: None,
    }
}

// --- Fallback: the `.help` usage lines -------------------------------------------

/// One `.help` line as a node: `name description -- usage` (Bareos API 1) or
/// the tab-separated form of older Onesimus tables.
fn help_node(line: &str) -> Option<Node> {
    let (name, description, usage, arguments) = if line.contains('\t') {
        let mut fields = line.split('\t');
        (fields.next()?.trim(), fields.next().unwrap_or(""), fields.next().unwrap_or(""), Some(fields.next().unwrap_or("")))
    } else {
        let (header, usage) = line.rsplit_once(" --")?;
        let (name, description) = header.split_once(' ')?;
        (name.trim(), description, usage.trim(), None)
    };
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_') {
        return None;
    }
    let source_arguments;
    let arguments = match arguments {
        Some(arguments) => arguments,
        None => {
            source_arguments = usage_arguments(usage, name);
            &source_arguments
        }
    };
    let mut node = plain_node(name, description);
    node.usage = Some(usage.to_string());
    node.args = parse_arguments(arguments);
    Some(node)
}

/// The keywords (`!word`) and keys (`key`) of a Bareos usage string, comma separated.
pub fn usage_arguments(usage: &str, command: &str) -> String {
    let mut arguments = Vec::new();
    // Placeholders may contain alternatives, spaces and punctuation. Ignore
    // their contents, but retain the surrounding grammar (including adjacent
    // optional groups such as `[maxvoljobs=<nb>][enabled=<yes/no>]`).
    let mut grammar = String::new();
    let mut placeholder = false;
    for ch in usage.chars() {
        match ch {
            '<' => {
                placeholder = true;
                grammar.push_str(" <value> ");
            }
            '>' => placeholder = false,
            _ if placeholder => {}
            '[' | ']' | '(' | ')' | '|' | ',' => grammar.push(' '),
            '=' => grammar.push_str(" = "),
            _ => grammar.push(ch),
        }
    }
    let tokens: Vec<_> = grammar.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        if *token == command || !token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            continue;
        }
        // Literal values (e.g. trace=0/1 or jobid=0) are not subcommands.
        if index > 0 && tokens[index - 1] == "=" {
            continue;
        }
        let argument = if tokens.get(index + 1) == Some(&"=") { token.to_string() } else { format!("!{token}") };
        if !arguments.contains(&argument) {
            arguments.push(argument);
        }
    }
    arguments.join(",")
}

/// The dot-command a well-known key takes its values from (Bareos has no grammar).
fn known_source(key: &str) -> Option<&'static str> {
    Some(match key {
        "job" => ".jobs",
        "client" => ".clients",
        "storage" => ".storages",
        "pool" => ".pools",
        "fileset" => ".filesets",
        "schedule" => ".schedule",
        "level" => ".levels",
        "type" => ".types",
        "catalog" => ".catalogs",
        "volume" => ".media",
        _ => return None,
    })
}

/// `!word` → keyword, `key` → `key=value` (with the value source of [`known_source`]).
fn parse_arguments(arguments: &str) -> Vec<Arg> {
    arguments.split(',').map(str::trim).filter(|a| !a.is_empty()).map(|a| {
        let (name, kind) = match a.strip_prefix('!') {
            Some(word) => (word, Kind::Keyword),
            None => (a, Kind::Value),
        };
        let values = match (kind, known_source(name)) {
            (Kind::Value, Some(source)) => Values::Source(source.to_string()),
            _ => Values::None,
        };
        Arg {
            name: name.to_string(), kind, arg_type: if matches!(values, Values::Source(_)) { "resource" } else { "string" }.into(),
            required: false, repeatable: kind == Kind::Keyword, group: None, list: false, positional: false,
            value_optional: false, values, description: String::new(), default: None,
        }
    }).collect()
}

// --- Tokens -----------------------------------------------------------------------

/// A token of the line with its byte span (Bareos quoting: `key="a b"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The raw text, quotes included.
    pub text: String,
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset after the last character.
    pub end: usize,
}

impl Token {
    /// Key (before the first `=` outside quotes) and the unquoted value.
    pub fn key_value(&self) -> (String, Option<String>) {
        let mut key = String::new();
        let mut in_quotes = false;
        for (i, c) in self.text.char_indices() {
            match c {
                '"' => in_quotes = !in_quotes,
                '=' if !in_quotes => return (key, Some(self.text[i + 1..].replace('"', ""))),
                _ => key.push(c),
            }
        }
        (key, None)
    }
}

/// Split like the Director (whitespace separates, double quotes group);
/// the second value is true when the line ends inside quotes.
pub fn tokenize(line: &str) -> (Vec<Token>, bool) {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        if c.is_whitespace() && !in_quotes {
            if let Some(s) = start.take() {
                tokens.push(Token { text: line[s..i].to_string(), start: s, end: i });
            }
            continue;
        }
        if start.is_none() {
            start = Some(i);
        }
        if c == '"' {
            in_quotes = !in_quotes;
        }
    }
    if let Some(s) = start {
        tokens.push(Token { text: line[s..].to_string(), start: s, end: line.len() });
    }
    (tokens, in_quotes)
}

// --- Completion -------------------------------------------------------------------

/// One completion candidate: what is inserted and what the list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Inserted text.
    pub replacement: String,
    /// Shown in the list (`word  -- description`).
    pub display: String,
}

/// A candidate shown as `replacement  -- description`.
fn candidate(replacement: String, description: &str) -> Candidate {
    let display = if description.is_empty() { replacement.clone() } else { format!("{replacement:<22} -- {description}") };
    Candidate { replacement, display }
}

/// A value in double quotes when it contains blanks (Bareos quoting).
fn quote(value: &str) -> String {
    if value.contains(char::is_whitespace) { format!("\"{value}\"") } else { value.to_string() }
}

/// Case-insensitive `starts_with`.
fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len() && text.is_char_boundary(prefix.len()) && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// The values of an argument: fixed, from the cache, or the command words.
fn values_of(grammar: &Grammar, cache: &ValueCache, arg: &Arg) -> Vec<String> {
    match &arg.values {
        Values::None => Vec::new(),
        Values::Fixed(values) => values.clone(),
        Values::Source(source) if source == COMMANDS_SOURCE => grammar.commands.iter().map(|c| c.word.clone()).collect(),
        Values::Source(source) => cache.get(source).cloned().unwrap_or_default(),
    }
}

/// Where the line stands: the command, the subcommand path and the arguments used so far.
pub struct Position<'g> {
    /// Command and subcommands, outermost first.
    pub path: Vec<&'g Node>,
    /// Lower-case keys and keywords given (positional values under their placeholder).
    pub used: Vec<String>,
    /// Arguments of a subcommand not given at this node yet.
    pub any_arg: bool,
}

impl<'g> Position<'g> {
    /// The arguments in scope: those of every node on the path.
    pub fn scope(&self) -> Vec<&'g Arg> {
        self.path.iter().flat_map(|n| n.args.iter()).collect()
    }

    /// The innermost node.
    pub fn node(&self) -> &'g Node {
        self.path[self.path.len() - 1]
    }

    /// How often the argument `name` was given.
    fn count(&self, name: &str) -> usize {
        self.used.iter().filter(|u| u.eq_ignore_ascii_case(name)).count()
    }

    /// Whether `arg` may still be offered: not given (or repeatable) and no
    /// other member of its group given.
    pub fn open(&self, arg: &Arg) -> bool {
        if self.count(&arg.name) > 0 && !arg.repeatable {
            return false;
        }
        match &arg.group {
            Some(group) => !self.scope().iter().any(|o| o.group.as_deref() == Some(group) && o.name != arg.name && self.count(&o.name) > 0),
            None => true,
        }
    }
}

/// Walk the complete tokens after the command: a token naming a subcommand
/// of the current node descends; the others are arguments.
pub fn position<'g>(root: &'g Node, tokens: &[Token]) -> Position<'g> {
    let mut position = Position { path: vec![root], used: Vec::new(), any_arg: false };
    for token in tokens {
        let (key, value) = token.key_value();
        let current = position.node();
        let sub = current.subs.iter().find(|s| s.is_named(&key) && match (&s.value, &value) {
            (None, None) | (Some(_), Some(_)) => true,
            (Some(v), None) => v.value_optional,
            (None, Some(_)) => false,
        });
        match sub {
            Some(sub) => {
                position.path.push(sub);
                position.any_arg = false;
            }
            None => {
                position.any_arg = true;
                let named = position.scope().into_iter().find(|a| !a.positional && a.name.eq_ignore_ascii_case(&key));
                match named {
                    Some(arg) => position.used.push(arg.name.to_ascii_lowercase()),
                    None => {
                        let positional = position.scope().into_iter()
                            .find(|a| a.positional && (a.repeatable || position.count(&a.name) == 0));
                        position.used.push(positional.map(|a| a.name.clone()).unwrap_or(key).to_ascii_lowercase());
                    }
                }
            }
        }
    }
    position
}

/// Completion at `pos`: the start of the replaced text and the candidates.
pub fn complete(grammar: &Grammar, cache: &ValueCache, line: &str, pos: usize) -> (usize, Vec<Candidate>) {
    let before = &line[..pos];
    let (tokens, in_quotes) = tokenize(before);
    let at_new_word = before.is_empty() || (before.ends_with(char::is_whitespace) && !in_quotes);
    let (done, partial) = if at_new_word { (&tokens[..], None) } else { (&tokens[..tokens.len() - 1], tokens.last()) };
    let start = partial.map(|t| t.start).unwrap_or(pos);
    let prefix = partial.map(|t| t.text.as_str()).unwrap_or("");

    let Some(first) = done.first() else {
        let mut out: Vec<Candidate> = grammar.commands.iter().filter(|c| starts_with_ci(&c.word, prefix))
            .map(|c| candidate(c.word.clone(), &c.description)).collect();
        out.sort_by(|a, b| a.replacement.cmp(&b.replacement));
        return (start, out);
    };
    let Some(root) = grammar.command(&first.key_value().0) else { return (pos, Vec::new()) };
    if root.free_text {
        return (pos, Vec::new());
    }
    let position = position(root, &done[1..]);
    let scope = position.scope();

    // a value after `key=`
    let with_value = partial.and_then(|t| match t.key_value() {
        (key, Some(_)) => Some((t, key)),
        _ => None,
    });
    if let Some((token, key)) = with_value {
        let raw_value = &token.text[key.len() + 1..];
        let node = position.node();
        let target = node.subs.iter().find(|s| s.is_named(&key)).and_then(|s| s.value.as_deref())
            .or_else(|| scope.iter().copied().find(|a| !a.positional && a.kind == Kind::Value && a.name.eq_ignore_ascii_case(&key)));
        let Some(arg) = target else { return (pos, Vec::new()) };
        let (done_values, current) = match (arg.list, raw_value.rfind(',')) {
            (true, Some(comma)) => (&raw_value[..=comma], &raw_value[comma + 1..]),
            _ => ("", raw_value),
        };
        let given: Vec<String> = done_values.split(',').map(|v| v.trim_matches('"').to_string()).collect();
        let current_bare = current.trim_start_matches('"');
        let mut values: Vec<Candidate> = values_of(grammar, cache, arg).into_iter()
            .filter(|v| starts_with_ci(v, current_bare) && !given.contains(v))
            .map(|v| Candidate { replacement: quote(&v), display: v }).collect();
        values.dedup();
        return (token.start + key.len() + 1 + done_values.len(), values);
    }

    // subcommand words, keywords, `key=`, positional values
    let mut required = Vec::new();
    let mut rest = Vec::new();
    for sub in &position.node().subs {
        let replacement = match &sub.value {
            Some(v) if !v.value_optional => format!("{}=", sub.word),
            _ => sub.word.clone(),
        };
        rest.push(candidate(replacement, &sub.description));
    }
    for arg in scope.iter().filter(|a| position.open(a)) {
        let target = if arg.required { &mut required } else { &mut rest };
        match (arg.positional, arg.kind) {
            (true, _) => {
                for value in values_of(grammar, cache, arg) {
                    target.push(candidate(quote(&value), ""));
                }
            }
            (false, Kind::Keyword) => target.push(candidate(arg.name.clone(), &arg.description)),
            (false, Kind::Value) => target.push(candidate(format!("{}=", arg.name), &arg.description)),
        }
    }
    let mut out = Vec::new();
    for mut group in [required, rest] {
        group.retain(|c| starts_with_ci(&c.replacement, prefix));
        group.sort_by(|a, b| a.replacement.cmp(&b.replacement));
        group.dedup_by(|a, b| a.replacement == b.replacement);
        for c in group {
            if !out.iter().any(|o: &Candidate| o.replacement == c.replacement) {
                out.push(c);
            }
        }
    }
    (start, out)
}

// --- Autosuggestion ---------------------------------------------------------------

/// The autosuggestion shown dimmed after the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Shown text.
    pub display: String,
    /// Inserted when accepted; None for a usage hint (shown only).
    pub completion: Option<String>,
}

impl Suggestion {
    /// A suggestion that is shown and inserted when accepted.
    fn insert(text: &str) -> Suggestion {
        Suggestion { display: text.to_string(), completion: Some(text.to_string()) }
    }
}

/// The fish autosuggestion at the end of the line: 1. the newest history
/// entry that starts with the line and was not refused by the Director;
/// 2. the rest of the only completion candidate; 3. the usage of what may
/// follow (shown, not inserted). `history` is newest first.
pub fn suggest(grammar: &Grammar, cache: &ValueCache, history: &[String], failed: &[String],
               line: &str, pos: usize) -> Option<Suggestion> {
    if pos < line.len() || line.trim().is_empty() {
        return None;
    }
    if let Some(entry) = history.iter().find(|h| h.len() > line.len() && h.starts_with(line) && !failed.contains(h)) {
        return Some(Suggestion::insert(&entry[line.len()..]));
    }
    // Like fish: the first completion (required arguments first, then
    // alphabetical) that extends the typed word, even among several; Right
    // accepts it, Tab lists all. After a blank fish suggests nothing from
    // completions; the usage below shows what may follow.
    let (start, candidates) = complete(grammar, cache, line, pos);
    let typed = &line[start..];
    let extends = |c: &&Candidate| c.replacement.len() > typed.len() && c.replacement.starts_with(typed);
    if let Some(first) = candidates.iter().filter(|_| !typed.is_empty()).find(extends) {
        return Some(Suggestion::insert(&first.replacement[typed.len()..]));
    }
    usage_hint(grammar, line).map(|display| Suggestion { display, completion: None })
}

/// What may follow the line: the subcommand words of the current node
/// (`<files|jobs|volume|quota>`), else its open arguments as a usage
/// fragment; for the `.help` fallback the rest of the usage string.
fn usage_hint(grammar: &Grammar, line: &str) -> Option<String> {
    if !line.ends_with(char::is_whitespace) {
        return None;
    }
    let (tokens, in_quotes) = tokenize(line);
    if in_quotes {
        return None;
    }
    let root = grammar.command(&tokens.first()?.key_value().0)?;
    if let Some(usage) = &root.usage {
        return (tokens.len() == 1 && !usage.trim().is_empty()).then(|| usage.trim().to_string());
    }
    let position = position(root, &tokens[1..]);
    let node = position.node();
    if !node.subs.is_empty() && !position.any_arg {
        let words: Vec<String> = node.subs.iter().map(word_usage).collect();
        return Some(format!("<{}>", words.join("|")));
    }
    let open: Vec<Arg> = node.args.iter().filter(|a| position.open(a)).cloned().collect();
    let text = args_usage(&open);
    (!text.is_empty()).then_some(text)
}

// --- Usage lines and help text (the same text as odir's manpages::help_text) -----

/// The placeholder of a value (`<name>`, short fixed lists as `<a|b>`), as odir writes it.
fn placeholder(arg: &Arg) -> String {
    let inner = match &arg.values {
        Values::Fixed(_) if arg.arg_type == "bool" => "yes|no".to_string(),
        Values::Fixed(values) if values.len() <= 4 => values.join("|"),
        _ => arg.name.clone(),
    };
    let mut text = format!("<{inner}>");
    if arg.list {
        text.push_str("[,...]");
    }
    text
}

/// One argument as written in a usage line.
pub fn arg_usage(arg: &Arg) -> String {
    let mut text = match (arg.kind, arg.positional) {
        (_, true) => placeholder(arg),
        (Kind::Keyword, _) => arg.name.clone(),
        (Kind::Value, _) if arg.value_optional => format!("{}[={}]", arg.name, placeholder(arg)),
        (Kind::Value, _) => format!("{}={}", arg.name, placeholder(arg)),
    };
    if arg.repeatable {
        text.push_str(" ...");
    }
    text
}

/// A node word as written in a usage line.
pub fn word_usage(node: &Node) -> String {
    match &node.value {
        None => node.word.clone(),
        Some(value) if value.value_optional => format!("{}[={}]", node.word, placeholder(value)),
        Some(value) => format!("{}={}", node.word, placeholder(value)),
    }
}

/// Arguments as a usage fragment: required bare, others in brackets, a group as `[a | b]`.
pub fn args_usage(args: &[Arg]) -> String {
    let mut parts = Vec::new();
    let mut done: Vec<&str> = Vec::new();
    for arg in args {
        match &arg.group {
            Some(group) => {
                if done.contains(&group.as_str()) {
                    continue;
                }
                done.push(group);
                let members: Vec<&Arg> = args.iter().filter(|a| a.group.as_deref() == Some(group)).collect();
                let text = members.iter().map(|a| arg_usage(a)).collect::<Vec<_>>().join(" | ");
                parts.push(if members.iter().any(|a| a.required) { text } else { format!("[{text}]") });
            }
            None if arg.required => parts.push(arg_usage(arg)),
            None => parts.push(format!("[{}]", arg_usage(arg))),
        }
    }
    parts.join(" ")
}

/// Usage lines of a node and its subtree.
pub fn usage_lines(node: &Node, prefix: &str) -> Vec<String> {
    let head = if prefix.is_empty() { word_usage(node) } else { format!("{prefix} {}", word_usage(node)) };
    let mut lines = Vec::new();
    let args = args_usage(&node.args);
    if !node.args.is_empty() || node.subs.is_empty() || node.value.is_some() || prefix.is_empty() {
        lines.push(if args.is_empty() { head.clone() } else { format!("{head} {args}") });
    }
    for sub in &node.subs {
        lines.extend(usage_lines(sub, &head));
    }
    lines
}

/// The description of an argument in the help text, as odir writes it.
fn argument_item(arg: &Arg) -> String {
    let mut parts = Vec::new();
    parts.push(match (arg.kind, arg.positional, arg.arg_type.as_str()) {
        (_, true, _) => "positional value",
        (Kind::Keyword, _, _) => "keyword",
        (_, _, "integer") => "number",
        (_, _, "bool") => "yes or no",
        (_, _, "enum") => "one of a fixed set",
        (_, _, "resource") => "name",
        _ => "text",
    }.to_string());
    match &arg.values {
        Values::Fixed(values) if arg.arg_type != "bool" => parts.push(format!("values: {}", values.join(", "))),
        Values::Source(source) if source == COMMANDS_SOURCE => parts.push("a command name".to_string()),
        Values::Source(source) => parts.push(format!("values from {source}")),
        _ => {}
    }
    if arg.value_optional {
        parts.push("the value may be left out".to_string());
    }
    if arg.list {
        parts.push("several values separated by commas".to_string());
    }
    if arg.repeatable {
        parts.push("may be repeated".to_string());
    }
    if let Some(group) = &arg.group {
        parts.push(format!("excludes the other arguments of group {group}"));
    }
    if let Some(default) = &arg.default {
        parts.push(format!("default {default}"));
    }
    if arg.required {
        parts.push("required".to_string());
    }
    parts.join("; ")
}

/// Every node below `node` with its full path (`purge volume`), depth first.
fn paths<'a>(node: &'a Node, prefix: &str, out: &mut Vec<(String, &'a Node)>) {
    let path = if prefix.is_empty() { node.word.clone() } else { format!("{prefix} {}", word_usage(node)) };
    out.push((path.clone(), node));
    for sub in &node.subs {
        paths(sub, &path, out);
    }
}

/// `help <command>`: usage, description, subcommands, arguments, examples,
/// origin and related commands (odir's man page as plain text).
pub fn help_text(node: &Node) -> String {
    let mut out = format!("{} -- {}\n\nUSAGE\n", node.word, node.description);
    for line in usage_lines(node, "") {
        out.push_str(&format!("  {line}\n"));
    }
    out.push_str("\nDESCRIPTION\n");
    let paragraphs = if node.long.is_empty() { vec![format!("{}.", node.description.trim_end_matches('.'))] } else { node.long.clone() };
    out.push_str(&paragraphs.iter().map(|p| format!("  {p}\n")).collect::<Vec<_>>().join("\n"));
    let mut all = Vec::new();
    paths(node, "", &mut all);
    if all.len() > 1 {
        out.push_str("\nSUBCOMMANDS\n");
        for (path, sub) in all.iter().skip(1) {
            out.push_str(&format!("  {path}\n      {}\n", sub.description));
            for paragraph in &sub.long {
                out.push_str(&format!("\n      {paragraph}\n"));
            }
        }
    }
    let blocks: Vec<&(String, &Node)> = all.iter().filter(|(_, n)| !n.args.is_empty() || n.value.is_some()).collect();
    if !blocks.is_empty() {
        out.push_str("\nARGUMENTS\n");
        for (path, block) in blocks {
            out.push_str(&format!("  {path}\n"));
            let own = block.value.as_deref().map(|v| Arg { name: block.word.clone(), ..v.clone() });
            for arg in own.iter().chain(block.args.iter()) {
                out.push_str(&format!("    {}\n        {} ({})\n", arg_usage(arg), arg.description, argument_item(arg)));
            }
        }
    }
    // the command's examples, then those of its subcommands in tree order
    let examples: Vec<&(String, String)> = all.iter().flat_map(|(_, n)| n.examples.iter()).collect();
    if !examples.is_empty() {
        out.push_str("\nEXAMPLES\n");
        for (line, text) in examples {
            out.push_str(&format!("  {line}\n      {text}\n"));
        }
    }
    let origin = if node.origin == "onesimus" {
        "Onesimus extension. The Bareos Director does not know this command and answers \"is an invalid command\".".to_string()
    } else {
        let extensions: Vec<String> = all.iter().skip(1).filter(|(_, n)| n.origin == "onesimus").map(|(p, _)| p.clone()).collect();
        if extensions.is_empty() {
            "Bareos console command.".to_string()
        } else {
            format!("Bareos console command. Onesimus extensions: {}.", extensions.join(", "))
        }
    };
    out.push_str(&format!("\nORIGIN\n  {origin}\n"));
    if !node.see_also.is_empty() {
        out.push_str(&format!("\nSEE ALSO\n  {}\n", node.see_also.join(", ")));
    }
    out
}

/// `help <command>` the console answers itself: with a typed grammar and
/// exactly one known command word; None sends the line to the Director.
pub fn local_help(grammar: &Grammar, line: &str) -> Option<String> {
    if !grammar.typed {
        return None;
    }
    let (tokens, _) = tokenize(line);
    match tokens.as_slice() {
        [help, word] if help.text.eq_ignore_ascii_case("help") => grammar.command(&word.text).map(help_text),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The full grammar the odir test `console_grammar_doc_is_current` writes.
    fn odir_grammar() -> Grammar {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/wiki/commands/console-grammar.json");
        let json: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        Grammar::from_grammar_json(&json).expect("schema 1")
    }

    fn cache() -> ValueCache {
        let mut cache = ValueCache::new();
        cache.insert(".pools".into(), vec!["Full".into(), "Incremental".into()]);
        cache.insert(".filesets".into(), vec!["Full Set".into(), "LinuxAll".into()]);
        cache.insert(".consoles".into(), vec!["admin".into(), "ops".into()]);
        cache.insert(".media".into(), vec!["Full-0001".into()]);
        cache
    }

    fn words(grammar: &Grammar, line: &str) -> Vec<String> {
        complete(grammar, &cache(), line, line.len()).1.into_iter().map(|c| c.replacement).collect()
    }

    #[test]
    fn reads_grammar_schema_1() {
        let grammar = odir_grammar();
        assert!(grammar.typed);
        let purge = grammar.command("purge").unwrap();
        assert!(purge.subs.iter().any(|s| s.word == "volume"));
        assert!(grammar.command("llist").is_some(), "aliases");
        assert!(grammar.value_sources().contains(&".pools".to_string()));
        // envelope of .api 2, unknown fields ignored, other schema refused
        let wrapped = serde_json::json!({"jsonrpc": "2.0", "result": {"grammar": {"schema": 1, "future": true,
            "commands": [{"word": "x", "new_field": 1, "args": [], "subs": []}]}}});
        assert_eq!(Grammar::from_grammar_json(&wrapped).unwrap().commands[0].word, "x");
        let other = serde_json::json!({"grammar": {"schema": 2, "commands": [{"word": "x"}]}});
        assert!(Grammar::from_grammar_json(&other).is_none());
        // Bareos: the error object of an invalid command
        let bareos = serde_json::json!({"jsonrpc": "2.0", "id": null, "error": {"code": 1, "message": "failed"}});
        assert!(Grammar::from_grammar_json(&bareos).is_none());
    }

    #[test]
    fn falls_back_to_help_lines() {
        let lines = vec![
            "status Report status -- all | dir=<dir-name> | director | scheduler | client=<client-name>".to_string(),
            "run Run a job -- job=<job-name> client=<client-name> level=<level> [ yes ]".to_string(),
            "noise without separator".to_string(),
        ];
        let grammar = Grammar::from_help_lines(&lines);
        assert!(!grammar.typed);
        assert_eq!(grammar.commands.len(), 2);
        let run = grammar.command("run").unwrap();
        assert!(run.args.iter().any(|a| a.name == "yes" && a.kind == Kind::Keyword));
        assert!(run.args.iter().any(|a| a.name == "client" && a.values == Values::Source(".clients".into())));
        assert_eq!(words(&grammar, "run cl"), ["client="]);
        assert_eq!(words(&grammar, "st"), ["status"]);
        assert!(words(&grammar, "status ").contains(&"director".to_string()));
        // usage as hint after the command
        let hint = suggest(&grammar, &cache(), &[], &[], "run ", 4).unwrap();
        assert!(hint.display.starts_with("job=<job-name>") && hint.completion.is_none());
    }

    #[test]
    fn usage_arguments_keep_partial_keys_and_groups() {
        assert_eq!(usage_arguments("[maxvoljobs=<nb>][enabled=<yes/no>] | slots [scan]", "update"), "maxvoljobs,enabled,!slots,!scan");
        assert_eq!(usage_arguments("level=<nn> trace=0/1 dir", "setdebug"), "level,trace,!dir");
    }

    #[test]
    fn walks_subcommands() {
        let grammar = odir_grammar();
        assert_eq!(words(&grammar, "purge vo"), ["volume"]);
        let after = words(&grammar, "purge volume ");
        for word in ["all", "pool=", "allpools", "volstatus=", "dryrun", "yes", "catalog="] {
            assert!(after.contains(&word.to_string()), "{word} in {after:?}");
        }
        assert!(!after.contains(&"files".to_string()));
        assert!(words(&grammar, "list ").contains(&"jobid=".to_string()), "value subcommand offers key=");
        assert!(words(&grammar, "list jobs ").contains(&"reverse".to_string()));
    }

    #[test]
    fn hides_used_keys_but_not_repeatable_ones() {
        let grammar = odir_grammar();
        let after = words(&grammar, ".acl set console=x jobacl=a ");
        assert!(after.contains(&"jobacl=".to_string()), "repeatable");
        assert!(!after.contains(&"console=".to_string()), "used");
        let profiles = words(&grammar, "configure add console name=n profile=a ");
        assert!(profiles.contains(&"profile=".to_string()) && !profiles.contains(&"name=".to_string()));
    }

    #[test]
    fn required_arguments_come_first() {
        let grammar = odir_grammar();
        let after = words(&grammar, ".acl copy ");
        assert_eq!(&after[..2], ["from=", "to="]);
        let set = words(&grammar, ".acl set ");
        assert_eq!(set[0], "console=");
    }

    #[test]
    fn one_of_groups() {
        let grammar = odir_grammar();
        let after = words(&grammar, "purge volume all allpools ");
        assert!(!after.contains(&"pool=".to_string()) && !after.contains(&"allpools".to_string()));
        let after = words(&grammar, "purge volume all pool=Full dryrun ");
        assert!(!after.contains(&"yes".to_string()) && !after.contains(&"dryrun".to_string()));
        assert!(after.contains(&"volstatus=".to_string()));
    }

    #[test]
    fn values_from_fixed_and_cache_quoted() {
        let grammar = odir_grammar();
        assert_eq!(words(&grammar, "estimate fileset=F"), ["\"Full Set\""]);
        assert_eq!(words(&grammar, "estimate fileset=\"Fu"), ["\"Full Set\""]);
        let line = "purge volume all pool=";
        let (start, candidates) = complete(&grammar, &cache(), line, line.len());
        assert_eq!(start, line.len());
        assert_eq!(candidates.iter().map(|c| c.replacement.as_str()).collect::<Vec<_>>(), ["Full", "Incremental"]);
        assert_eq!(words(&grammar, ".api "), ["0", "1", "2", "compact=", "json", "off", "on"]);
        assert!(words(&grammar, "help pu").contains(&"purge".to_string()), "@commands");
    }

    #[test]
    fn list_values_after_a_comma() {
        let grammar = odir_grammar();
        let line = "purge volume all allpools volstatus=Full,";
        let (start, candidates) = complete(&grammar, &cache(), line, line.len());
        assert_eq!(start, line.len());
        let values: Vec<_> = candidates.iter().map(|c| c.replacement.as_str()).collect();
        assert!(values.contains(&"Used") && !values.contains(&"Full"), "{values:?}");
        assert_eq!(words(&grammar, "purge volume all allpools volstatus=Full,Us"), ["Used"]);
    }

    #[test]
    fn descriptions_in_the_display() {
        let grammar = odir_grammar();
        let (_, candidates) = complete(&grammar, &cache(), "purge volume all", 16);
        assert_eq!(candidates.len(), 2, "all and allpools");
        assert!(candidates[0].display.contains(" -- "), "{:?}", candidates[0]);
        let (start, commands) = complete(&grammar, &cache(), "pur", 3);
        assert_eq!((start, commands[0].replacement.as_str()), (0, "purge"));
        assert!(commands[0].display.starts_with("purge                  -- Purge records"));
    }

    #[test]
    fn suggestion_prefers_history() {
        let grammar = odir_grammar();
        let history = vec!["list jobs last".to_string(), "list volumes".to_string()];
        let s = suggest(&grammar, &cache(), &history, &[], "list ", 5).unwrap();
        assert_eq!(s.completion.as_deref(), Some("jobs last"));
        assert!(suggest(&grammar, &cache(), &history, &[], "list ", 3).is_none(), "cursor not at the end");
    }

    #[test]
    fn suggestion_skips_failed_lines() {
        let grammar = odir_grammar();
        let history = vec!["list jbos".to_string(), "list jobs".to_string()];
        let failed = vec!["list jbos".to_string()];
        let s = suggest(&grammar, &cache(), &history, &failed, "list j", 6).unwrap();
        assert_eq!(s.completion.as_deref(), Some("obs"));
    }

    #[test]
    fn suggestion_completes_the_first_candidate() {
        let grammar = odir_grammar();
        let s = suggest(&grammar, &cache(), &[], &[], "purge volume all allp", 21).unwrap();
        assert_eq!(s.completion.as_deref(), Some("ools"));
        // several candidates: the first, as fish does
        let (_, all) = complete(&grammar, &cache(), "purge vol", 9);
        assert!(!all.is_empty());
        let s = suggest(&grammar, &cache(), &[], &[], "purge vol", 9).unwrap();
        assert_eq!(s.completion.as_deref(), Some(&all[0].replacement["vol".len()..]));
        let (_, all) = complete(&grammar, &cache(), "list jo", 7);
        assert!(all.len() > 1, "{all:?}");
        let s = suggest(&grammar, &cache(), &[], &[], "list jo", 7).unwrap();
        assert_eq!(s.completion.as_deref(), Some(&all[0].replacement["jo".len()..]));
        // usage: subcommand words, then open arguments; shown, not inserted
        let s = suggest(&grammar, &cache(), &[], &[], "purge ", 6).unwrap();
        assert_eq!(s.display, "<files|jobs|volume[=<volume>]|quota>");
        assert!(s.completion.is_none());
        let s = suggest(&grammar, &cache(), &[], &[], ".acl copy ", 10).unwrap();
        assert_eq!(s.display, "from=<from> to=<to>");
    }

    #[test]
    fn help_text_is_odirs() {
        let grammar = odir_grammar();
        for (word, file) in [("purge", "purge"), (".acl", "acl"), ("status", "status"), ("version", "version")] {
            let path = format!("{}/../odir/tests/fixtures/help_{file}.txt", env!("CARGO_MANIFEST_DIR"));
            let expected = std::fs::read_to_string(&path).unwrap();
            assert_eq!(help_text(grammar.command(word).unwrap()), expected, "{word}");
        }
        assert!(local_help(&grammar, "help purge").unwrap().starts_with("purge -- "));
        assert!(local_help(&grammar, "help").is_none() && local_help(&grammar, "help nosuch").is_none());
        let fallback = Grammar::from_help_lines(&["purge Purge -- files".to_string()]);
        assert!(local_help(&fallback, "help purge").is_none(), "Bareos answers help itself");
    }

    #[test]
    fn tokens_keep_quotes_and_spans() {
        let (tokens, open) = tokenize("run fileset=\"Full Set\" yes");
        assert!(!open);
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[1].key_value(), ("fileset".to_string(), Some("Full Set".to_string())));
        assert_eq!((tokens[2].start, tokens[2].end), (23, 26));
        assert!(tokenize("run fileset=\"Full ").1);
    }
}
