//! Interactive restore file selection as MCP tools (S-36): the 26 commands
//! of the Director's restore tree (ua_tree.cc) behind `restore_tree_start`
//! … `restore_tree_done` / `restore_tree_abort`.
//!
//! - gRPC: `StartRestoreSession` and the `RestoreTree*` RPCs of the typed
//!   `DirectorApi`; the session lives in the Director (bound to the login).
//!   A production Director answers UNIMPLEMENTED (its tree RPCs exist only
//!   with `grpc-development`); `ExecuteCommand` has no dialog, so the tools
//!   report that and point to the one-line `restore` tool or BNET.
//! - BNET: the console dialog itself. `restore …` stops at the tree prompt
//!   (`$ `); every tool sends one tree command and reads up to the next
//!   prompt. While the selection is open the connection is in the dialog,
//!   so the server refuses every other tool until done/abort.
//!
//! Results have the shape of the typed responses (RestoreSessionResponse,
//! RestoreTreeListResponse, RestoreTreeCountResponse, ActionResponse) on
//! both transports.

use std::sync::Mutex;

use serde_json::{Map, Value, json};

use crate::mcp::{CommandRunner, MAX_QUESTIONS, tool_result};
use crate::mcp_typed::{self, Outcome};

/// Open BNET file selection of this server (gRPC sessions live in the Director).
#[derive(Debug, Default)]
pub struct TreeState {
    bnet: Option<String>,
    counter: u64,
}

impl TreeState {
    pub fn bnet_session(&self) -> Option<String> {
        self.bnet.clone()
    }
}

pub const TOOLS: &[&str] = &[
    "restore_tree_start", "restore_tree_cd", "restore_tree_ls", "restore_tree_pwd", "restore_tree_mark",
    "restore_tree_unmark", "restore_tree_count", "restore_tree_find", "restore_tree_estimate",
    "restore_tree_help", "restore_tree_done", "restore_tree_abort",
];

pub fn is_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

fn s(d: &str) -> Value {
    json!({ "type": "string", "description": d })
}

fn tool(name: &str, title: &str, description: &str, mut properties: Map<String, Value>, required: &[&str],
        read_only: bool, destructive: bool) -> Value {
    if name != "restore_tree_start" {
        properties.insert("session_id".into(), s("session_id from restore_tree_start"));
    }
    let mut req: Vec<&str> = required.to_vec();
    if name != "restore_tree_start" {
        req.insert(0, "session_id");
    }
    json!({
        "name": name, "title": title, "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": req, "additionalProperties": false },
        "annotations": { "readOnlyHint": read_only, "destructiveHint": destructive,
                         "idempotentHint": read_only, "openWorldHint": false },
    })
}

fn props(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

pub fn tools() -> Vec<Value> {
    let pattern = |d: &str| props(json!({ "pattern": s(d) }));
    vec![
        tool("restore_tree_start", "Start an interactive restore",
            "Build the restore tree of a backup (bconsole 'restore …' up to the file selection) and open a \
             selection session. Select the backups with jobids, or the latest state (default) / the state \
             before a date of client+fileset. Then cd/ls/mark with the other restore_tree_* tools and start \
             the job with restore_tree_done (or restore_tree_abort). Target options as in 'restore'.",
            props(json!({
                "client": s("Client whose backup is restored"),
                "jobids": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "minItems": 1,
                    "description": "JobIds of the tree (Full, Differential, Incrementals)" },
                "fileset": s("FileSet for the latest state or before"),
                "before": s("state before 'YYYY-MM-DD HH:MM:SS'"),
                "pool": s("only jobs on volumes of this pool"),
                "where": s("target directory prefix; '/' restores in place"),
                "regexwhere": s("path rewrite (instead of where)"),
                "strip_prefix": s("remove this prefix"),
                "add_prefix": s("add this prefix"),
                "add_suffix": s("append to file names"),
                "replace": { "type": "string", "enum": ["always", "ifnewer", "ifolder", "never"], "description": "existing files" },
                "restoreclient": s("target client if different"),
                "restorejob": s("Restore job resource"),
                "storage": s("read from this Storage"),
                "comment": s("comment of the restore job"),
            })), &["client"], false, false),
        tool("restore_tree_cd", "Change directory in the restore tree", "cd <dir> in the file selection.",
            props(json!({ "path": s("directory, absolute or relative, '..' allowed") })), &["path"], true, false),
        tool("restore_tree_ls", "List the restore tree",
            "Entries of the current directory (ls), only directories (.lsdir) or only marked ones (lsmark, \
             recursive); optional wildcard pattern. Marked entries carry marked=true.",
            {
                let mut p = pattern("wildcard pattern, e.g. *.conf");
                p.insert("mode".into(), json!({ "type": "string", "enum": ["all", "dirs", "marked"], "default": "all",
                    "description": "all (ls), dirs (.lsdir), marked (lsmark)" }));
                p
            }, &[], true, false),
        tool("restore_tree_pwd", "Current directory of the restore tree", "pwd in the file selection.",
            Map::new(), &[], true, false),
        tool("restore_tree_mark", "Mark files for the restore",
            "mark <pattern> (files and directories with everything below), or with dirs_only markdir (only \
             the directory entries).",
            {
                let mut p = pattern("name or wildcard in the current directory, '*' for all");
                p.insert("dirs_only".into(), json!({ "type": "boolean", "description": "markdir: directories only" }));
                p
            }, &["pattern"], false, false),
        tool("restore_tree_unmark", "Unmark files", "unmark <pattern>, or with dirs_only unmarkdir.",
            {
                let mut p = pattern("name or wildcard in the current directory, '*' for all");
                p.insert("dirs_only".into(), json!({ "type": "boolean", "description": "unmarkdir: directories only" }));
                p
            }, &["pattern"], false, false),
        tool("restore_tree_count", "Count marked files", "count: total and marked files/dirs.",
            Map::new(), &[], true, false),
        tool("restore_tree_find", "Find files in the restore tree", "find <pattern>: paths matching a wildcard.",
            pattern("wildcard pattern"), &["pattern"], true, false),
        tool("restore_tree_estimate", "Estimate the restore", "estimate: files, marked files and bytes.",
            Map::new(), &[], true, false),
        tool("restore_tree_help", "Restore tree commands", "help of the file selection.", Map::new(), &[], true, false),
        tool("restore_tree_done", "Start the restore job",
            "done: leave the file selection and start the restore job of the marked files (answers \
             'OK to run?' with yes). Confirm the target (where/replace) with the user first.",
            Map::new(), &[], false, true),
        tool("restore_tree_abort", "Abort the restore", "abort/quit: discard the selection, no job.",
            Map::new(), &[], false, false),
    ]
}

/// Tree-command argument: a path or wildcard, quoted if it contains blanks.
fn tree_arg(key: &str, value: &str) -> Result<String, String> {
    if value.is_empty() || value.chars().any(|c| c.is_control() || c == '"') {
        return Err(format!("{key} must be a non-empty single line without '\"'"));
    }
    Ok(if value.contains(' ') { format!("\"{value}\"") } else { value.to_string() })
}

/// The tree command line (BNET) of a tool; the arguments are checked here.
pub fn tree_line(tool: &str, args: &Value) -> Result<String, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    let flag = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
    let known: &[&str] = match tool {
        "restore_tree_cd" => &["session_id", "path"],
        "restore_tree_ls" => &["session_id", "pattern", "mode"],
        "restore_tree_mark" | "restore_tree_unmark" => &["session_id", "pattern", "dirs_only"],
        "restore_tree_find" => &["session_id", "pattern"],
        _ => &["session_id"],
    };
    for key in args.as_object().into_iter().flat_map(|m| m.keys()) {
        if !known.contains(&key.as_str()) {
            return Err(format!("unknown argument '{key}' for {tool}"));
        }
    }
    let with = |cmd: &str, key: &str, need: bool| -> Result<String, String> {
        match text(key) {
            Some(v) => Ok(format!("{cmd} {}", tree_arg(key, v)?)),
            None if need => Err(format!("{key} is required")),
            None => Ok(cmd.to_string()),
        }
    };
    match tool {
        "restore_tree_cd" => with("cd", "path", true),
        "restore_tree_ls" => match text("mode").unwrap_or("all") {
            "all" => with("ls", "pattern", false),
            "dirs" => with(".lsdir", "pattern", false),
            "marked" => with("lsmark", "pattern", false),
            _ => Err("mode must be one of: all, dirs, marked".into()),
        },
        "restore_tree_pwd" => Ok("pwd".into()),
        "restore_tree_mark" => with(if flag("dirs_only") { "markdir" } else { "mark" }, "pattern", true),
        "restore_tree_unmark" => with(if flag("dirs_only") { "unmarkdir" } else { "unmark" }, "pattern", true),
        "restore_tree_count" => Ok("count".into()),
        "restore_tree_find" => with("find", "pattern", true),
        "restore_tree_estimate" => Ok("estimate".into()),
        "restore_tree_help" => Ok("help".into()),
        "restore_tree_done" => Ok("done".into()),
        "restore_tree_abort" => Ok("abort".into()),
        _ => Err(format!("unknown tool {tool}")),
    }
}

/// The `restore …` line that opens the selection (no file selection, no `done`).
pub fn start_line(args: &Value) -> Result<String, String> {
    let mut checked = args.clone();
    if let Some(map) = checked.as_object_mut() {
        // the one-line builder needs a file selection; `all` is removed again below
        map.insert("all".into(), json!(true));
    }
    let plan = crate::mcp_write::plan("restore", &checked).expect("restore tool")?;
    let line = plan.command;
    let line = line.strip_suffix(" yes").unwrap_or(&line);
    // drop " all done": the selection starts empty
    let (head, tail) = line.split_once(" all done").ok_or("unexpected restore line")?;
    Ok(format!("{head}{tail}"))
}

/// The Director's prompt `$ ` of the file selection ends the output.
fn at_tree_prompt(d: &crate::mcp::Dialog) -> bool {
    d.waiting && d.output.trim_end().ends_with('$')
}

fn strip_prompt(output: &str) -> String {
    let t = output.trim_end();
    t.strip_suffix('$').unwrap_or(t).trim_end().to_string() + "\n"
}

/// Entries of ls/.lsdir/lsmark/find output (`*` marked, `+` below marked).
pub fn entries(text: &str) -> Vec<Value> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("cwd is:")).map(|line| {
        let line = line.trim_start();
        let marked = line.starts_with('*');
        let name = line.trim_start_matches(['*', '+']);
        json!({ "name": name, "is_dir": name.ends_with('/'), "marked": marked })
    }).collect()
}

fn numbers(text: &str) -> Vec<u64> {
    text.split(|c: char| !(c.is_ascii_digit() || c == ','))
        .filter_map(|w| w.replace(',', "").parse().ok()).collect()
}

fn cwd_of(text: &str) -> String {
    text.lines().rev().find(|l| !l.trim().is_empty())
        .map(|l| l.strip_prefix("cwd is: ").unwrap_or(l).trim_end().to_string()).unwrap_or_default()
}

fn ok(structured: Value) -> Value {
    json!({ "content": [{ "type": "text", "text": structured.to_string() }], "isError": false,
            "structuredContent": structured })
}

/// Handle a restore tree tool; `None` if `name` is none of them.
pub fn call<R: CommandRunner>(name: &str, args: &Value, runner: &Mutex<R>, state: &Mutex<TreeState>)
    -> Option<Result<Value, (i64, String)>> {
    if !is_tool(name) {
        return None;
    }
    let mut runner = match runner.lock() {
        Ok(r) => r,
        Err(_) => return Some(Err((-32603, "Director runner unavailable".into()))),
    };
    let mut state = match state.lock() {
        Ok(s) => s,
        Err(_) => return Some(Err((-32603, "restore state unavailable".into()))),
    };
    Some(Ok(run(name, args, &mut *runner, &mut state)))
}

/// One tree tool call (runner and state locked).
pub fn run<R: CommandRunner + ?Sized>(name: &str, args: &Value, runner: &mut R, state: &mut TreeState) -> Value {
    // Arguments first, the same checks for both transports.
    let line = if name == "restore_tree_start" { start_line(args) } else { tree_line(name, args) };
    let line = match line {
        Ok(l) => l,
        Err(msg) => return tool_result(&msg, true),
    };
    let session = args.get("session_id").and_then(Value::as_str).unwrap_or("").to_string();
    if name != "restore_tree_start" && session.is_empty() {
        return tool_result("session_id is required", true);
    }
    let bnet_open = state.bnet.clone();
    // BNET session of this server
    if bnet_open.as_deref() == Some(session.as_str()) && name != "restore_tree_start" {
        return bnet_tree(name, &line, runner, state);
    }
    // gRPC: typed RPC
    if let Some((rpc, request)) = mcp_typed::request(name, args) {
        match mcp_typed::call(runner, rpc, &request) {
            Outcome::Done(result) => return result,
            Outcome::Fallback if runner.is_grpc() => {
                return tool_result(&format!(
                    "The Director does not offer {rpc} (a production Director implements the interactive \
                     restore RPCs only when built with grpc-development), and ExecuteCommand has no dialog. \
                     Use the one-line 'restore' tool (files, directories or fileregex; browse with ovfs_*), \
                     or connect oconsole via BNET."), true);
            }
            Outcome::Fallback => {}
        }
    }
    if name != "restore_tree_start" {
        return tool_result(&format!("no open restore session '{session}'; start one with restore_tree_start"), true);
    }
    if let Some(open) = bnet_open {
        return tool_result(&format!("restore session '{open}' is still open; finish it with restore_tree_done or restore_tree_abort"), true);
    }
    // BNET: run `restore …` up to the tree prompt
    let first = match runner.dialog_start(&line) {
        Ok(d) => d,
        Err(e) => return tool_result(&format!("Director error: {e}"), true),
    };
    if at_tree_prompt(&first) {
        state.counter += 1;
        let id = format!("bnet-{}", state.counter);
        state.bnet = Some(id.clone());
        let text = strip_prompt(&first.output);
        let files = text.lines().find(|l| l.contains("files inserted into the tree"))
            .and_then(|l| numbers(l).first().copied()).unwrap_or(0);
        let jobids: Vec<u64> = args.get("jobids").and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
        return ok(json!({ "session_id": id, "jobids": jobids, "files": files, "cwd": cwd_of(&text),
            "text": format!("> {line}\n{text}") }));
    }
    let (text, question) = finish(runner, first, |_| None);
    let mut msg = format!("> {line}\n{text}");
    if let Some(q) = question {
        msg.push_str(&format!("\n[oconsole] The restore asked '{q}' before the file selection; it was cancelled. \
            Give the missing argument (jobids, fileset, restorejob, …) and call restore_tree_start again."));
    }
    tool_result(&msg, true)
}

/// Answer the remaining questions of a dialog (`answer` or "." to cancel)
/// until the command ends; returns the text and the first cancelled question.
fn finish<R: CommandRunner + ?Sized>(runner: &mut R, mut step: crate::mcp::Dialog,
    answer: impl Fn(&str) -> Option<&'static str>) -> (String, Option<String>) {
    let mut text = step.output.clone();
    let mut cancelled = None;
    let mut rounds = 0;
    while step.waiting {
        rounds += 1;
        let q = step.output.trim_end().rsplit('\n').next().unwrap_or("").trim().to_string();
        let reply = if rounds > MAX_QUESTIONS { None } else { answer(&q) };
        if reply.is_none() && cancelled.is_none() {
            cancelled = Some(q);
        }
        let reply = reply.unwrap_or(".");
        text.push_str(reply);
        text.push('\n');
        step = match runner.dialog_send(reply) {
            Ok(s) => s,
            Err(e) => {
                text.push_str(&format!("Director error: {e}\n"));
                break;
            }
        };
        text.push_str(&step.output);
    }
    (text, cancelled)
}

fn bnet_tree<R: CommandRunner + ?Sized>(name: &str, line: &str, runner: &mut R, state: &mut TreeState) -> Value {
    let step = match runner.dialog_send(line) {
        Ok(s) => s,
        Err(e) => {
            state.bnet = None;
            return tool_result(&format!("Director error: {e}"), true);
        }
    };
    if matches!(name, "restore_tree_done" | "restore_tree_abort") {
        state.bnet = None;
        let done = name == "restore_tree_done";
        let (text, question) = finish(runner, step, |q| (done && q.contains("OK to run?")).then_some("yes"));
        let jobids: Vec<u64> = text.lines()
            .filter(|l| l.contains("queued") || l.starts_with("Job started"))
            .filter_map(|l| l.split("JobId=").nth(1))
            .filter_map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()).collect();
        let success = if done { !jobids.is_empty() } else { true };
        let mut out = format!("> {line}\n{text}");
        if let Some(q) = &question {
            out.push_str(&format!("\n[oconsole] The Director asked '{q}', which this tool does not answer; it was cancelled."));
        }
        return json!({ "content": [{ "type": "text", "text": out }], "isError": !success,
            "structuredContent": { "success": success, "text": text, "jobids": jobids, "command": line,
                "question": question.unwrap_or_default() } });
    }
    if !at_tree_prompt(&step) {
        // The selection ended unexpectedly: drain and close.
        state.bnet = None;
        let (text, _) = finish(runner, step, |_| None);
        return tool_result(&format!("The restore file selection ended:\n{text}"), true);
    }
    let text = strip_prompt(&step.output);
    match name {
        "restore_tree_cd" | "restore_tree_pwd" => ok(json!({ "cwd": cwd_of(&text), "entries": [], "text": text })),
        "restore_tree_ls" | "restore_tree_find" => {
            let cwd = match runner.dialog_send(".pwd") {
                Ok(p) if at_tree_prompt(&p) => cwd_of(&strip_prompt(&p.output)),
                _ => String::new(),
            };
            ok(json!({ "cwd": cwd, "entries": entries(&text), "text": text }))
        }
        "restore_tree_help" => ok(json!({ "text": text })),
        _ => {
            let n = numbers(&text);
            if matches!(name, "restore_tree_count" | "restore_tree_estimate") {
                ok(json!({ "total": n.first().copied().unwrap_or(0), "marked": n.get(1).copied().unwrap_or(0),
                    "bytes": n.get(2).copied().unwrap_or(0), "text": text }))
            } else {
                ok(json!({ "marked": n.first().copied().unwrap_or(0), "text": text }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{Dialog, TypedError};

    #[test]
    fn lines_and_schemas() {
        assert_eq!(tools().len(), TOOLS.len());
        for t in tools() {
            assert!(is_tool(t["name"].as_str().unwrap()));
        }
        assert_eq!(start_line(&json!({"client":"fd","jobids":[5],"where":"/tmp/r"})).unwrap(),
            "restore jobid=5 client=fd where=\"/tmp/r\"");
        assert_eq!(start_line(&json!({"client":"fd","fileset":"Set"})).unwrap(), "restore select current client=fd fileset=Set");
        assert!(start_line(&json!({"client":"fd"})).is_err());
        let sid = |v: Value| { let mut v = v; v["session_id"] = json!("x"); v };
        assert_eq!(tree_line("restore_tree_cd", &sid(json!({"path":"/etc/a b/"}))).unwrap(), "cd \"/etc/a b/\"");
        assert_eq!(tree_line("restore_tree_ls", &sid(json!({"mode":"marked"}))).unwrap(), "lsmark");
        assert_eq!(tree_line("restore_tree_mark", &sid(json!({"pattern":"*","dirs_only":true}))).unwrap(), "markdir *");
        assert_eq!(tree_line("restore_tree_unmark", &sid(json!({"pattern":"hosts"}))).unwrap(), "unmark hosts");
        assert!(tree_line("restore_tree_cd", &sid(json!({"path":"a\"b"}))).is_err());
        assert!(tree_line("restore_tree_cd", &sid(json!({"path":"a\nb"}))).is_err());
        assert!(tree_line("restore_tree_cd", &sid(json!({"bogus":1}))).is_err());
        assert_eq!(entries("*hosts\npasswd\n+ssh/\n"), vec![
            json!({"name":"hosts","is_dir":false,"marked":true}),
            json!({"name":"passwd","is_dir":false,"marked":false}),
            json!({"name":"ssh/","is_dir":true,"marked":false})]);
    }

    /// Fake BNET Director: a scripted restore dialog.
    #[derive(Default)]
    struct Bnet { sent: Vec<String> }
    impl CommandRunner for Bnet {
        fn run(&mut self, c: &str) -> anyhow::Result<String> { self.sent.push(c.into()); Ok(String::new()) }
        fn dialog_start(&mut self, c: &str) -> anyhow::Result<Dialog> {
            self.sent.push(c.into());
            Ok(Dialog { output: "12 files inserted into the tree.\n\nYou are now entering file selection mode …\ncwd is: /\n$ ".into(), waiting: true })
        }
        fn dialog_send(&mut self, c: &str) -> anyhow::Result<Dialog> {
            self.sent.push(c.into());
            let (output, waiting) = match c {
                "cd /etc/" => ("cwd is: /etc/\n$ ", true),
                "mark hosts" => ("1 file marked.\n$ ", true),
                "ls" => ("*hosts\npasswd\n$ ", true),
                ".pwd" => ("/etc/\n$ ", true),
                "count" => ("12 total files/dirs. 1 marked to be restored.\n$ ", true),
                "done" => ("Bootstrap records written …\nOK to run? (yes/mod/no): ", true),
                "yes" => ("Job queued. JobId=77\n", false),
                _ => ("", false),
            };
            Ok(Dialog { output: output.into(), waiting })
        }
    }

    #[test]
    fn bnet_dialog_session() {
        let (mut r, mut st) = (Bnet::default(), TreeState::default());
        let start = run("restore_tree_start", &json!({"client":"fd","jobids":[5],"where":"/tmp/r"}), &mut r, &mut st);
        assert_eq!(start["isError"], false, "{start}");
        assert_eq!(start["structuredContent"]["files"], 12);
        let id = start["structuredContent"]["session_id"].clone();
        assert_eq!(st.bnet_session().as_deref(), Some("bnet-1"));
        let again = run("restore_tree_start", &json!({"client":"fd","jobids":[5]}), &mut r, &mut st);
        assert_eq!(again["isError"], true);
        let cd = run("restore_tree_cd", &json!({"session_id": id, "path": "/etc/"}), &mut r, &mut st);
        assert_eq!(cd["structuredContent"]["cwd"], "/etc/");
        let m = run("restore_tree_mark", &json!({"session_id": id, "pattern": "hosts"}), &mut r, &mut st);
        assert_eq!(m["structuredContent"]["marked"], 1);
        let ls = run("restore_tree_ls", &json!({"session_id": id}), &mut r, &mut st);
        assert_eq!(ls["structuredContent"]["entries"][0]["marked"], true);
        assert_eq!(ls["structuredContent"]["cwd"], "/etc/");
        let c = run("restore_tree_count", &json!({"session_id": id}), &mut r, &mut st);
        assert_eq!(c["structuredContent"]["total"], 12);
        let done = run("restore_tree_done", &json!({"session_id": id}), &mut r, &mut st);
        assert_eq!(done["isError"], false, "{done}");
        assert_eq!(done["structuredContent"]["jobids"], json!([77]));
        assert!(st.bnet_session().is_none());
        assert_eq!(r.sent, vec!["restore jobid=5 client=fd where=\"/tmp/r\"", "cd /etc/", "mark hosts", "ls", ".pwd",
            "count", "done", "yes"]);
    }

    /// Fake gRPC Director: typed tree RPCs, or UNIMPLEMENTED (production).
    struct Grpc { implemented: bool, calls: Vec<String> }
    impl CommandRunner for Grpc {
        fn run(&mut self, c: &str) -> anyhow::Result<String> { self.calls.push(c.into()); Ok(String::new()) }
        fn is_grpc(&self) -> bool { true }
        fn typed_call(&mut self, rpc: &str, _req: Vec<u8>) -> Option<Result<Vec<u8>, TypedError>> {
            self.calls.push(rpc.into());
            if !self.implemented {
                return Some(Err(TypedError::Unimplemented));
            }
            let (_, out) = mcp_typed::rpc_types(rpc).unwrap();
            let reply = match rpc {
                "StartRestoreSession" => json!({"session_id":"abc","files":3,"cwd":"/"}),
                _ => json!({"marked":1}),
            };
            Some(Ok(onesimus_proto_rs::typed::json_to_message(&out, &reply).bytes))
        }
    }

    #[test]
    fn grpc_typed_session_and_unimplemented() {
        let (mut g, mut st) = (Grpc { implemented: true, calls: vec![] }, TreeState::default());
        let start = run("restore_tree_start", &json!({"client":"fd","jobids":[5]}), &mut g, &mut st);
        assert_eq!(start["structuredContent"]["session_id"], "abc");
        let m = run("restore_tree_mark", &json!({"session_id":"abc","pattern":"*","dirs_only":true}), &mut g, &mut st);
        assert_eq!(m["structuredContent"]["marked"], 1);
        assert_eq!(g.calls, vec!["StartRestoreSession", "RestoreTreeMarkDir"]);
        let (mut g, mut st) = (Grpc { implemented: false, calls: vec![] }, TreeState::default());
        let start = run("restore_tree_start", &json!({"client":"fd","jobids":[5]}), &mut g, &mut st);
        assert_eq!(start["isError"], true);
        assert!(start["content"][0]["text"].as_str().unwrap().contains("grpc-development"));
        assert_eq!(g.calls, vec!["StartRestoreSession"], "no ExecuteCommand fallback for dialogs");
    }
}
