//! MCP server mode (`oconsole --mcp`).
//!
//! Speaks the Model Context Protocol (JSON-RPC 2.0, one message per line) on
//! stdin/stdout so AI assistants can query the Director. The Director
//! connection (BNET or gRPC) is opened before serving, with the same config
//! and login as the interactive console. stdout carries protocol only; all
//! diagnostics go to stderr.
//!
//! Tools:
//!   query   — read-only commands (list, show, status, dot-commands, …)
//!   execute — any command; only with --mcp-allow-write

use std::io::{BufRead, Write};

use serde_json::{json, Value};

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Runs one command line against the Director and returns its text output.
pub trait CommandRunner {
    fn run(&mut self, command: &str) -> anyhow::Result<String>;
}

pub struct McpServer<R: CommandRunner> {
    runner: R,
    allow_write: bool,
    director_name: String,
}

impl<R: CommandRunner> McpServer<R> {
    pub fn new(runner: R, allow_write: bool, director_name: String) -> Self {
        Self { runner, allow_write, director_name }
    }

    /// Serve until stdin closes.
    pub fn serve(&mut self) {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_line(&line) {
                let _ = writeln!(stdout, "{reply}");
                let _ = stdout.flush();
            }
        }
    }

    /// Handle one JSON-RPC message. Returns the response line, if any.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return Some(error_response(Value::Null, -32700, &format!("parse error: {e}")).to_string()),
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        // Notifications (no id) never get a response
        let id = id?;

        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => self.tools_call(&params),
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, msg)) => error_response(id, code, &msg),
        }.to_string())
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
        let version = if PROTOCOL_VERSIONS.contains(&requested) { requested } else { PROTOCOL_VERSIONS[0] };
        let mode = if self.allow_write { "read and write" } else { "read-only" };
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "oconsole", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!(
                "Connected to the Onesimus/Bareos Director '{}' ({mode}). \
                 Commands are bconsole command lines, e.g. 'status dir', 'list jobs', \
                 'list volumes', 'show client=NAME', 'llist jobid=42'. \
                 Use 'help' or '.help all' to discover commands.",
                self.director_name
            ),
        })
    }

    fn tools_list(&self) -> Value {
        let mut tools = vec![json!({
            "name": "query",
            "title": "Query the Director (read-only)",
            "description": "Run a read-only bconsole command on the Director and return its output. \
                Allowed: status, list, llist, show, version, help, time, messages, estimate, \
                and dot-commands such as .jobs, .clients, .pools, .storages, .filesets, .help. \
                Examples: 'status dir', 'list jobs', 'llist jobid=42', 'show fileset=Linux'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "bconsole command line" }
                },
                "required": ["command"]
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        })];
        if self.allow_write {
            tools.push(json!({
                "name": "execute",
                "title": "Execute a Director command",
                "description": "Run any bconsole command on the Director, including ones that change \
                    state: run, cancel, label, mount, update, prune, purge, delete, configure. \
                    Prefer 'query' for read-only commands. Confirm destructive actions with the user first.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "bconsole command line" }
                    },
                    "required": ["command"]
                },
                "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false }
            }));
        }
        json!({ "tools": tools })
    }

    fn tools_call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let command = params
            .get("arguments")
            .and_then(|a| a.get("command"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");

        match name {
            "query" => {}
            "execute" if self.allow_write => {}
            "execute" => return Err((-32602, "tool 'execute' is disabled (start oconsole with --mcp-allow-write)".into())),
            _ => return Err((-32602, format!("unknown tool: {name}"))),
        }

        // Tool-level problems are reported as tool results so the model can react
        if let Err(msg) = validate_command(command) {
            return Ok(tool_result(&msg, true));
        }
        if name == "query" && !is_read_only(command) {
            return Ok(tool_result(
                &format!("'{command}' is not a read-only command. {}",
                    if self.allow_write { "Use the 'execute' tool." }
                    else { "Write access is disabled (oconsole --mcp-allow-write)." }),
                true,
            ));
        }

        match self.runner.run(command) {
            Ok(out) if out.trim().is_empty() => Ok(tool_result("(no output)", false)),
            Ok(out) => Ok(tool_result(&out, false)),
            Err(e) => Ok(tool_result(&format!("Director error: {e}"), true)),
        }
    }
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// One single command line, no control characters (BNET would take them literally).
fn validate_command(command: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err("empty command".into());
    }
    if command.chars().any(|c| c.is_control()) {
        return Err("command must be a single line without control characters".into());
    }
    let first = command.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if matches!(first.as_str(), "quit" | "exit" | ".quit" | ".api" | "@exit") || first.starts_with('@') {
        return Err(format!("'{first}' is not available via MCP"));
    }
    Ok(())
}

/// Commands that only read Director/catalog state.
fn is_read_only(command: &str) -> bool {
    let first = command.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    if first.starts_with('.') {
        // Dot-commands are queries, except those that modify state or run raw SQL
        return !matches!(
            first.as_str(),
            ".sql" | ".bvfs_restore" | ".bvfs_cleanup" | ".bvfs_clear_cache" | ".die" | ".exit" | ".consoles_kill"
        );
    }
    matches!(
        first.as_str(),
        "status" | "list" | "llist" | "show" | "version" | "help" | "time" | "messages" | "estimate"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo(Vec<String>);
    impl CommandRunner for Echo {
        fn run(&mut self, command: &str) -> anyhow::Result<String> {
            self.0.push(command.to_string());
            Ok(format!("ran {command}\n"))
        }
    }

    fn call(srv: &mut McpServer<Echo>, line: Value) -> Value {
        serde_json::from_str(&srv.handle_line(&line.to_string()).unwrap()).unwrap()
    }

    fn tool_call(tool: &str, cmd: &str) -> Value {
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":tool,"arguments":{"command":cmd}}})
    }

    #[test]
    fn initialize_negotiates_version() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(&mut srv, json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}));
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(r["result"]["serverInfo"]["name"], "oconsole");
        let r = call(&mut srv, json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}));
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn notifications_get_no_response() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        assert!(srv.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    }

    #[test]
    fn read_only_mode_lists_only_query() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(&mut srv, json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}));
        let names: Vec<_> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].clone()).collect();
        assert_eq!(names, vec![json!("query")]);
    }

    #[test]
    fn query_runs_read_only_commands() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(&mut srv, tool_call("query", "list jobs"));
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(r["result"]["content"][0]["text"], "ran list jobs\n");
        let r = call(&mut srv, tool_call("query", ".clients"));
        assert_eq!(r["result"]["isError"], false);
    }

    #[test]
    fn query_rejects_writes_without_running_them() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        for cmd in ["delete volume=Full-0001 yes", "run job=Backup yes", ".sql query=\"DELETE FROM job\"", "purge volume=x"] {
            let r = call(&mut srv, tool_call("query", cmd));
            assert_eq!(r["result"]["isError"], true, "{cmd}");
        }
        assert!(srv.runner.0.is_empty());
    }

    #[test]
    fn execute_disabled_without_flag() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(&mut srv, tool_call("execute", "run job=Backup yes"));
        assert!(r["error"].is_object());
        assert!(srv.runner.0.is_empty());
    }

    #[test]
    fn execute_allowed_with_flag() {
        let mut srv = McpServer::new(Echo(vec![]), true, "dir".into());
        let r = call(&mut srv, tool_call("execute", "run job=Backup yes"));
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(srv.runner.0, vec!["run job=Backup yes"]);
    }

    #[test]
    fn multi_line_and_session_commands_rejected() {
        let mut srv = McpServer::new(Echo(vec![]), true, "dir".into());
        for cmd in ["list jobs\ndelete volume=x yes", "quit", ".api 2", "@exit", ""] {
            let r = call(&mut srv, tool_call("execute", cmd));
            assert_eq!(r["result"]["isError"], true, "{cmd:?}");
        }
        assert!(srv.runner.0.is_empty());
    }

    #[test]
    fn unknown_method_is_an_error() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(&mut srv, json!({"jsonrpc":"2.0","id":9,"method":"resources/list"}));
        assert_eq!(r["error"]["code"], -32601);
    }
}
