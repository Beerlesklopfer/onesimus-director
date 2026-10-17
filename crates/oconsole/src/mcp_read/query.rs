//! The generic read-only `query` tool: one bconsole line, checked against
//! an allowlist of reading commands. In every build. Moved unchanged from
//! mcp.rs; `validate_command` also guards the development `execute` tool.

use serde_json::{Value, json};

/// The `query` tool definition.
pub fn tool() -> Value {
    json!({
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
    })
}

/// The `command` argument of `query`/`execute`, trimmed ("" if missing).
pub fn command_arg(arguments: &Value) -> &str {
    arguments.get("command").and_then(Value::as_str).map(str::trim).unwrap_or("")
}

/// One single command line, no control characters (BNET would take them literally).
pub fn validate_command(command: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err("empty command".into());
    }
    if command.chars().any(|c| c.is_control()) {
        return Err("command must be a single line without control characters".into());
    }
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(first.as_str(), "quit" | "exit" | ".quit" | ".api" | "@exit")
        || first.starts_with('@')
    {
        return Err(format!("'{first}' is not available via MCP"));
    }
    Ok(())
}

/// Commands that only read Director/catalog state.
pub fn is_read_only(command: &str) -> bool {
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if first.starts_with('.') {
        // Full names only: a prefix could resolve to a mutating command.
        return matches!(
            first.as_str(),
            "." | ".actiononpurge"
                | ".authorized"
                | ".catalogs"
                | ".clients"
                | ".consoles"
                | ".defaults"
                | ".filesets"
                | ".help"
                | ".jobdefs"
                | ".jobs"
                | ".jobstatus"
                | ".levels"
                | ".locations"
                | ".messages"
                | ".media"
                | ".mediatypes"
                | ".msgs"
                | ".pools"
                | ".profiles"
                | ".schedule"
                | ".status"
                | ".storage"
                | ".storages"
                | ".types"
                | ".users"
                | ".volstatus"
                | ".bvfs_get_jobids"
                | ".bvfs_lsdirs"
                | ".bvfs_lsfiles"
                | ".bvfs_versions"
                | ".sqltables"
                | ".sqlschema"
                | ".sqlcount"
                | ".ovfs_status"
                | ".ovfs_latest"
                | ".ovfs_tree"
                | ".ovfs_ls"
                | ".ovfs_versions"
        );
    }
    matches!(
        first.as_str(),
        "status"
            | "list"
            | "llist"
            | "show"
            | "version"
            | "help"
            | "time"
            | "messages"
            | "estimate"
    )
}
