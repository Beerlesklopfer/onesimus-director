//! Man pages generated from the console grammar ([`crate::command_grammar`]):
//! one page per command, `odir-<command>(7)`, the collection
//! `odir-commands(7)`, and the hand-written pages `oconsole(1)`, `odir(8)`
//! and `ofd(8)` (templates in packaging/man/src) with their generated parts.
//! [`help_text`] is the plain text of the same content; oconsole renders it
//! from the `.grammar` JSON for `help <command>` (fixture
//! crates/odir/tests/fixtures/help_*.txt keeps both in step).
//!
//! `odir --write-man-pages <dir>` writes every page; the committed pages in
//! packaging/man must equal the generated ones (test
//! `committed_pages_are_up_to_date`).

use std::path::Path;

use crate::command_grammar::{arg_usage, usage_lines, word_usage, ArgKind, ArgType, GrammarArg, Node, Origin, Values};

/// Hand-written page templates: (file name, text). `@VERSION@` is replaced
/// by the version, a line `.\" @COMMANDS@` by the generated command list.
const TEMPLATES: &[(&str, &str)] = &[
    ("oconsole.1", include_str!("../../../packaging/man/src/oconsole.1.in")),
    ("odir.8", include_str!("../../../packaging/man/src/odir.8.in")),
    ("ofd.8", include_str!("../../../packaging/man/src/ofd.8.in")),
];

/// File name of a command's page without section: `purge` → `odir-purge`,
/// `.acl` → `odir-dot-acl`, `.` → `odir-dot-noop`.
pub fn page_name(command: &str) -> String {
    match command.strip_prefix('.') {
        Some("") => "odir-dot-noop".to_string(),
        Some(rest) => format!("odir-dot-{rest}"),
        None => format!("odir-{command}"),
    }
}

/// Escape text for roff: backslash as `\e`, `-` as `\-`, a leading `.` or
/// `'` of a line protected with `\&`.
pub fn roff(text: &str) -> String {
    text.lines().map(|line| {
        let escaped = line.replace('\\', "\\e").replace('-', "\\-");
        if escaped.starts_with('.') || escaped.starts_with('\'') {
            format!("\\&{escaped}")
        } else {
            escaped
        }
    }).collect::<Vec<_>>().join("\n")
}

/// The SYNOPSIS lines of a command: one per command and subcommand.
pub fn synopsis(node: &Node) -> Vec<String> {
    usage_lines(node, "")
}

/// One argument for the ARGUMENTS section: kind and type, values or value
/// source, default, list/repeat/group/required.
pub fn argument_item(arg: &GrammarArg) -> String {
    let mut parts = Vec::new();
    parts.push(match (arg.kind, arg.positional, arg.arg_type) {
        (_, true, _) => "positional value".to_string(),
        (ArgKind::Keyword, _, _) => "keyword".to_string(),
        (_, _, ArgType::Integer) => "number".to_string(),
        (_, _, ArgType::Bool) => "yes or no".to_string(),
        (_, _, ArgType::Enum) => "one of a fixed set".to_string(),
        (_, _, ArgType::Resource) => "name".to_string(),
        (_, _, ArgType::String) => "text".to_string(),
    });
    match &arg.values {
        Values::Fixed(values) if arg.arg_type != ArgType::Bool => parts.push(format!("values: {}", values.join(", "))),
        Values::Source(source) if source == crate::command_grammar::COMMANDS_SOURCE => parts.push("a command name".to_string()),
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

/// The ORIGIN text of a command.
fn origin_text(node: &Node) -> String {
    if node.origin == Origin::Onesimus {
        return "Onesimus extension. The Bareos Director does not know this command and answers \
            \"is an invalid command\".".to_string();
    }
    let mut all = Vec::new();
    paths(node, "", &mut all);
    let extensions: Vec<String> = all.iter().skip(1).filter(|(_, n)| n.origin == Origin::Onesimus)
        .map(|(p, _)| p.clone()).collect();
    if extensions.is_empty() {
        "Bareos console command.".to_string()
    } else {
        format!("Bareos console command. Onesimus extensions: {}.", extensions.join(", "))
    }
}

/// The DESCRIPTION paragraphs: `long`, or the one-line description.
fn paragraphs(node: &Node) -> Vec<String> {
    if node.long.is_empty() {
        vec![format!("{}.", node.description.trim_end_matches('.'))]
    } else {
        node.long.clone()
    }
}

/// The nodes that have arguments, with their path.
fn argument_blocks(node: &Node) -> Vec<(String, &Node)> {
    let mut all = Vec::new();
    paths(node, "", &mut all);
    all.into_iter().filter(|(_, n)| !n.args.is_empty() || n.value.is_some()).collect()
}

/// Arguments of a block, the word's own value first.
fn block_args(node: &Node) -> Vec<GrammarArg> {
    let mut args: Vec<GrammarArg> = node.value.iter().map(|v| {
        let mut own = (**v).clone();
        own.name = node.word.clone();
        own
    }).collect();
    args.extend(node.args.iter().cloned());
    args
}

/// Plain text of `help <command>` in the console: the content of the man page without roff.
pub fn help_text(node: &Node) -> String {
    let mut out = format!("{} -- {}\n\nUSAGE\n", node.word, node.description);
    for line in synopsis(node) {
        out.push_str(&format!("  {line}\n"));
    }
    out.push_str("\nDESCRIPTION\n");
    out.push_str(&paragraphs(node).iter().map(|p| format!("  {p}\n")).collect::<Vec<_>>().join("\n"));
    let mut all = Vec::new();
    paths(node, "", &mut all);
    if all.len() > 1 {
        out.push_str("\nSUBCOMMANDS\n");
        for (path, sub) in all.iter().skip(1) {
            out.push_str(&format!("  {path}\n      {}\n", sub.description));
        }
    }
    let blocks = argument_blocks(node);
    if !blocks.is_empty() {
        out.push_str("\nARGUMENTS\n");
        for (path, block) in blocks {
            out.push_str(&format!("  {path}\n"));
            for arg in block_args(block) {
                out.push_str(&format!("    {}\n        {} ({})\n", arg_usage(&arg), arg.description, argument_item(&arg)));
            }
        }
    }
    if !node.examples.is_empty() {
        out.push_str("\nEXAMPLES\n");
        for (line, text) in &node.examples {
            out.push_str(&format!("  {line}\n      {text}\n"));
        }
    }
    out.push_str(&format!("\nORIGIN\n  {}\n", origin_text(node)));
    if !node.see_also.is_empty() {
        out.push_str(&format!("\nSEE ALSO\n  {}\n", node.see_also.join(", ")));
    }
    out
}

/// The roff body of a command (sections from NAME's successor on); `head`
/// is the section macro (`.SH` on a command page, `.SS` with bold labels in
/// the collection).
fn command_body(node: &Node, page: bool) -> String {
    let section = |title: &str| if page { format!(".SH {title}\n") } else { format!(".PP\n\\fB{}\\fR\n", title_case(title)) };
    let mut out = String::new();
    out.push_str(&section("SYNOPSIS"));
    out.push_str(".nf\n");
    for line in synopsis(node) {
        out.push_str(&roff(&line));
        out.push('\n');
    }
    out.push_str(".fi\n");
    out.push_str(&section("DESCRIPTION"));
    for paragraph in paragraphs(node) {
        out.push_str(&format!(".PP\n{}\n", roff(&paragraph)));
    }
    let mut all = Vec::new();
    paths(node, "", &mut all);
    if all.len() > 1 {
        out.push_str(&section("SUBCOMMANDS"));
        for (path, sub) in all.iter().skip(1) {
            out.push_str(&format!(".TP\n.B {}\n{}\n", roff_arg(path), roff(&sub.description)));
        }
    }
    let blocks = argument_blocks(node);
    if !blocks.is_empty() {
        out.push_str(&section("ARGUMENTS"));
        for (path, block) in blocks {
            if page {
                out.push_str(&format!(".SS {}\n", roff_arg(&path)));
            } else {
                out.push_str(&format!(".PP\n\\fI{}\\fR\n", roff(&path)));
            }
            for arg in block_args(block) {
                out.push_str(&format!(".TP\n.B {}\n{} ({})\n", roff_arg(&arg_usage(&arg)), roff(&arg.description), roff(&argument_item(&arg))));
            }
        }
    }
    if !node.examples.is_empty() {
        out.push_str(&section("EXAMPLES"));
        for (line, text) in &node.examples {
            out.push_str(&format!(".TP\n.B {}\n{}\n", roff_arg(line), roff(text)));
        }
    }
    out.push_str(&section("ORIGIN"));
    out.push_str(&format!("{}\n", roff(&origin_text(node))));
    out
}

/// A macro argument: escaped and quoted (it may contain blanks).
fn roff_arg(text: &str) -> String {
    format!("\"{}\"", roff(text).replace('"', "\\(dq"))
}

fn title_case(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
}

fn th(name: &str, section: &str, version: &str, manual: &str) -> String {
    format!(".\\\" Generated by odir --write-man-pages from the console grammar; do not edit.\n\
             .TH \"{}\" \"{section}\" \"\" \"odir {version}\" \"{manual}\"\n", name.to_ascii_uppercase())
}

/// roff of one command page.
pub fn command_page(node: &Node, version: &str) -> String {
    let name = page_name(&node.word);
    let mut out = th(&name, "7", version, "Onesimus Director Commands");
    out.push_str(&format!(".SH NAME\n{} \\- {}\n", roff(&name), roff(&format!("{}: {}", node.word, node.description))));
    out.push_str(&command_body(node, true));
    out.push_str(".SH SEE ALSO\n");
    let mut refs: Vec<String> = node.see_also.iter().map(|c| format!(".BR {} (7),", roff(&page_name(c)))).collect();
    refs.push(".BR odir\\-commands (7),".to_string());
    refs.push(".BR oconsole (1)".to_string());
    out.push_str(&refs.join("\n"));
    out.push('\n');
    out
}

/// roff of odir-commands(7): an overview and every command's section.
pub fn commands_page(roots: &[Node], version: &str) -> String {
    let mut out = th("odir-commands", "7", version, "Onesimus Director Commands");
    out.push_str(".SH NAME\nodir\\-commands \\- console commands of the Onesimus Director\n");
    out.push_str(".SH DESCRIPTION\n.PP\nThe commands a console (oconsole, bconsole, a gRPC or MCP client) sends to the \
        Onesimus Director. Words in brackets are optional, \\fIa\\fR | \\fIb\\fR means one of them, \\fB...\\fR a \
        repeatable argument and \\fB[,...]\\fR a comma separated list. Each command also has a page of its own, \
        for example \\fBodir\\-purge\\fR(7) or \\fBodir\\-dot\\-acl\\fR(7) for \\fB.acl\\fR.\n.PP\nThe grammar comes \
        from the Director's command registry; \\fB.grammar\\fR prints it for the commands the console may run.\n");
    out.push_str(".SH OVERVIEW\n");
    for root in roots {
        out.push_str(&format!(".TP\n.B {}\n{} (\\fB{}\\fR(7))\n", roff_arg(&root.word), roff(&root.description), roff(&page_name(&root.word))));
    }
    out.push_str(".SH COMMANDS\n");
    for root in roots {
        out.push_str(&format!(".SS {}\n{}\n", roff_arg(&root.word), roff(&root.description)));
        out.push_str(&command_body(root, false));
    }
    out.push_str(".SH SEE ALSO\n.BR oconsole (1),\n.BR odir (8)\n");
    out
}

/// The command list included in the hand-written pages (`.\" @COMMANDS@`).
fn command_list(roots: &[Node]) -> String {
    roots.iter().map(|r| format!(".TP\n.B {}\n{}\n", roff_arg(&r.word), roff(&r.description))).collect()
}

/// A hand-written page with its generated parts.
fn template_page(text: &str, roots: &[Node], version: &str) -> String {
    let mut out = String::from(".\\\" Generated by odir --write-man-pages from packaging/man/src; edit the template.\n");
    for line in text.lines() {
        if line.trim() == ".\\\" @COMMANDS@" {
            out.push_str(&command_list(roots));
        } else {
            out.push_str(&line.replace("@VERSION@", version));
            out.push('\n');
        }
    }
    out
}

/// Every page: (file name, roff).
pub fn all_pages(roots: &[Node], version: &str) -> Vec<(String, String)> {
    let mut pages = vec![("odir-commands.7".to_string(), commands_page(roots, version))];
    for root in roots {
        pages.push((format!("{}.7", page_name(&root.word)), command_page(root, version)));
    }
    for (name, text) in TEMPLATES {
        pages.push((name.to_string(), template_page(text, roots, version)));
    }
    pages
}

/// `odir --write-man-pages <dir>`: writes every page; returns the file names written.
pub fn write_all(roots: &[Node], dir: &Path, version: &str) -> std::io::Result<Vec<String>> {
    std::fs::create_dir_all(dir)?;
    let mut names = Vec::new();
    for (name, text) in all_pages(roots, version) {
        std::fs::write(dir.join(&name), text)?;
        names.push(name);
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_grammar::roots;

    fn root(word: &str) -> &'static Node {
        roots().iter().find(|n| n.word == word).unwrap()
    }

    #[test]
    fn synopsis_of_purge_volume() {
        let lines = synopsis(root("purge"));
        assert!(lines.iter().any(|l| l.starts_with("purge volume[=<volume>] [all] [pool=<pool> | allpools] [volstatus=<volstatus>[,...]]")
            && l.ends_with("[dryrun | yes]")), "{lines:?}");
    }

    #[test]
    fn page_names() {
        assert_eq!(page_name("purge"), "odir-purge");
        assert_eq!(page_name(".acl"), "odir-dot-acl");
        assert_eq!(page_name("."), "odir-dot-noop");
        assert_eq!(page_name(".bvfs_update"), "odir-dot-bvfs_update");
    }

    #[test]
    fn roff_escapes_leading_dots_and_dashes() {
        assert_eq!(roff(".acl list"), "\\&.acl list");
        assert_eq!(roff("'quoted"), "\\&'quoted");
        assert_eq!(roff("a -- b"), "a \\-\\- b");
        assert_eq!(roff("a\\b"), "a\\eb");
        assert_eq!(roff("x\n.y"), "x\n\\&.y");
        let page = command_page(root(".acl"), "1.0");
        assert!(page.contains(".SH NAME\nodir\\-dot\\-acl \\- \\&.acl: "), "{page}");
        assert!(!page.lines().any(|l| l.starts_with(".acl")), "unescaped request line");
    }

    #[test]
    fn every_command_has_a_page_and_a_description() {
        let pages = all_pages(roots(), "1.0");
        for r in roots() {
            assert!(!r.description.is_empty(), "{}", r.word);
            let name = format!("{}.7", page_name(&r.word));
            let page = &pages.iter().find(|(n, _)| *n == name).unwrap_or_else(|| panic!("{name}")).1;
            assert!(page.contains(".SH SYNOPSIS") && page.contains(".SH DESCRIPTION"), "{name}");
        }
        for name in ["oconsole.1", "odir.8", "ofd.8", "odir-commands.7"] {
            assert!(pages.iter().any(|(n, _)| n == name), "{name}");
        }
        let odir = &pages.iter().find(|(n, _)| n == "odir.8").unwrap().1;
        assert!(odir.contains(".B \"purge\"") && !odir.contains("@VERSION@") && !odir.contains("@COMMANDS@"));
    }

    fn man_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/man")
    }

    #[test]
    fn committed_pages_are_up_to_date() {
        let version = env!("CARGO_PKG_VERSION");
        let dir = man_dir();
        if std::env::var_os("UPDATE_MAN_PAGES").is_some() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_file() && path.extension().is_some_and(|e| e == "1" || e == "7" || e == "8") {
                    std::fs::remove_file(path).unwrap();
                }
            }
            write_all(roots(), &dir, version).unwrap();
        }
        let pages = all_pages(roots(), version);
        let mut differ = Vec::new();
        for (name, text) in &pages {
            if std::fs::read_to_string(dir.join(name)).ok().as_deref() != Some(text.as_str()) {
                differ.push(name.clone());
            }
        }
        let committed: Vec<String> = std::fs::read_dir(&dir).unwrap().filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            (name.ends_with(".1") || name.ends_with(".7") || name.ends_with(".8")).then_some(name)
        }).collect();
        for name in committed {
            if !pages.iter().any(|(n, _)| *n == name) {
                differ.push(format!("{name} (no longer generated)"));
            }
        }
        assert!(differ.is_empty(), "packaging/man outdated: {differ:?}; run odir --write-man-pages packaging/man \
            or this test with UPDATE_MAN_PAGES=1");
    }

    /// `mandoc -T lint` (or `groff -ww -z`) on every generated page, if installed.
    #[test]
    fn pages_lint_clean() {
        let dir = std::env::temp_dir().join(format!("odir-man-lint-{}", std::process::id()));
        let names = write_all(roots(), &dir, "1.0").unwrap();
        let mandoc = std::process::Command::new("mandoc").arg("-V").output().is_ok();
        let groff = std::process::Command::new("groff").arg("-v").output().is_ok();
        let mut problems = Vec::new();
        for name in &names {
            let path = dir.join(name);
            let output = if mandoc {
                std::process::Command::new("mandoc").args(["-T", "lint", "-W", "warning"]).arg(&path).output()
            } else if groff {
                std::process::Command::new("groff").args(["-man", "-ww", "-z"]).arg(&path).output()
            } else {
                break;
            }.unwrap();
            let text = String::from_utf8_lossy(&output.stderr).to_string() + &String::from_utf8_lossy(&output.stdout);
            if !text.trim().is_empty() {
                problems.push(format!("{name}: {}", text.trim()));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn help_text_matches_the_page() {
        let node = root("purge");
        let help = help_text(node);
        let page = command_page(node, "1.0");
        for line in synopsis(node) {
            assert!(help.contains(&format!("  {line}\n")), "{line}");
            assert!(page.contains(&roff(&line)), "{line}");
        }
        for (path, block) in argument_blocks(node) {
            assert!(help.contains(&format!("  {path}\n")));
            for arg in block_args(block) {
                assert!(help.contains(&argument_item(&arg)) && page.contains(&roff(&argument_item(&arg))), "{}", arg.name);
            }
        }
        for section in ["USAGE", "DESCRIPTION", "SUBCOMMANDS", "ARGUMENTS", "EXAMPLES", "ORIGIN", "SEE ALSO"] {
            assert!(help.contains(&format!("\n{section}\n")), "{section}");
        }
    }

    /// The help texts oconsole must reproduce from the `.grammar` JSON
    /// (crates/oconsole/src/grammar.rs reads these fixtures).
    #[test]
    fn help_fixtures_are_current() {
        for word in ["purge", ".acl", "status", "version"] {
            let name = word.trim_start_matches('.');
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/help_{name}.txt"));
            let actual = help_text(root(word));
            if std::env::var_os("UPDATE_GRAMMAR_SNAPSHOT").is_some() {
                std::fs::write(&path, &actual).unwrap();
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), actual,
                "{} outdated; rerun with UPDATE_GRAMMAR_SNAPSHOT=1", path.display());
        }
    }
}
