//! The read-only OVFS tools (Director `.ovfs_*` commands, Onesimus
//! extension), in every build. Moved unchanged from mcp.rs.

use serde_json::{Value, json};

/// The read-only OVFS tools (Director `.ovfs_*` commands, Onesimus
/// extension): browse restore trees lazily — `ovfs_latest` or a job list,
/// `ovfs_tree` with a small depth, `ovfs_ls` per expanded directory.
pub fn tools() -> Vec<Value> {
    let chain = json!({
        "jobids": { "type": "string", "description": "comma-separated JobIds of the chain (Full, Differential, Incrementals); or use client" },
        "client": { "type": "string", "description": "client name: the chain of its latest state (restore option 5)" },
        "fileset": { "type": "string", "description": "FileSet name, needed if the client has backups of several FileSets" }
    });
    let with = |extra: Value| {
        let mut properties = chain.as_object().unwrap().clone();
        properties.extend(extra.as_object().unwrap().clone());
        Value::Object(properties)
    };
    let annotations = json!({ "readOnlyHint": true, "openWorldHint": false });
    vec![
        json!({
            "name": "ovfs_status",
            "title": "Restore-tree backend status",
            "description": "Whether the catalog has the OVFS PostgreSQL extension (version) and whether \
                restore trees use it or the Director fallback. Results are the same either way.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": annotations,
        }),
        json!({
            "name": "ovfs_latest",
            "title": "Job chain of a client's latest backup",
            "description": "JobIds (Full, Differential, Incrementals) that restore the client's latest state, \
                as Bareos restore option 5 selects them. Pass them to ovfs_tree/ovfs_ls.",
            "inputSchema": { "type": "object", "properties": {
                "client": { "type": "string", "description": "client name" },
                "fileset": { "type": "string", "description": "FileSet name (if the client has several)" }
            }, "required": ["client"] },
            "annotations": annotations,
        }),
        json!({
            "name": "ovfs_tree",
            "title": "Restore tree of a job chain",
            "description": "Directory tree below root as JSON, depth levels deep (1 = root and its children). \
                Directories at the depth limit carry 'entries' instead of children; expand them with ovfs_ls. \
                Every node carries jobid/fileindex (restore identity). Keep depth small.",
            "inputSchema": { "type": "object", "properties": with(json!({
                "root": { "type": "string", "description": "directory, default /" },
                "depth": { "type": "integer", "minimum": 0, "description": "levels below root, default 1" },
                "max_entries": { "type": "integer", "minimum": 1, "description": "most nodes in the result, default 100000" }
            })) },
            "annotations": annotations,
        }),
        json!({
            "name": "ovfs_ls",
            "title": "List one directory of a job chain",
            "description": "Direct children of one directory (files first, then directories with 'entries' counts).",
            "inputSchema": { "type": "object", "properties": with(json!({
                "path": { "type": "string", "description": "directory, e.g. /home/anna/" }
            })), "required": ["path"] },
            "annotations": annotations,
        }),
        json!({
            "name": "ovfs_versions",
            "title": "Versions of one file",
            "description": "All backed-up versions of a file of a client, newest first, with volumes.",
            "inputSchema": { "type": "object", "properties": {
                "client": { "type": "string", "description": "client name" },
                "path": { "type": "string", "description": "directory of the file, e.g. /etc/" },
                "name": { "type": "string", "description": "file name; empty for the directory itself" },
                "copies": { "type": "boolean", "description": "include copy jobs" }
            }, "required": ["client", "path"] },
            "annotations": annotations,
        }),
    ]
}

/// `key="value"` with the console's quoting (backslash escapes).
fn ovfs_arg(key: &str, value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err(format!("{key} must not contain control characters"));
    }
    Ok(format!("{key}=\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The Director command line of an OVFS tool; `Ok(None)` for an unknown tool.
pub fn command(tool: &str, arguments: &Value) -> Result<Option<String>, String> {
    let text = |key: &str| -> Result<Option<String>, String> {
        match arguments.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(Value::Number(n)) => Ok(Some(n.to_string())),
            Some(_) => Err(format!("{key} must be a string")),
        }
    };
    let number = |key: &str| -> Result<Option<i64>, String> {
        match arguments.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_i64().map(Some).ok_or_else(|| format!("{key} must be an integer")),
        }
    };
    let required = |key: &str| -> Result<String, String> {
        text(key)?.filter(|v| !v.is_empty()).ok_or_else(|| format!("{key} is required"))
    };
    let mut parts: Vec<String> = Vec::new();
    let chain = |parts: &mut Vec<String>| -> Result<(), String> {
        if let Some(jobids) = text("jobids")? {
            if jobids.is_empty() || !jobids.split(',').all(|id| !id.trim().is_empty() && id.trim().bytes().all(|b| b.is_ascii_digit())) {
                return Err("jobids must be comma-separated JobIds".into());
            }
            parts.push(format!("jobid={}", jobids.replace(' ', "")));
        } else if let Some(client) = text("client")? {
            parts.push(ovfs_arg("client", &client)?);
        } else {
            return Err("jobids or client is required".into());
        }
        if let Some(fileset) = text("fileset")? {
            parts.push(ovfs_arg("fileset", &fileset)?);
        }
        Ok(())
    };
    let command = match tool {
        "ovfs_status" => ".ovfs_status".to_string(),
        "ovfs_latest" => {
            parts.push(ovfs_arg("client", &required("client")?)?);
            if let Some(fileset) = text("fileset")? {
                parts.push(ovfs_arg("fileset", &fileset)?);
            }
            format!(".ovfs_latest {}", parts.join(" "))
        }
        "ovfs_tree" => {
            chain(&mut parts)?;
            if let Some(root) = text("root")? {
                parts.push(ovfs_arg("root", &root)?);
            }
            if let Some(depth) = number("depth")? {
                parts.push(format!("depth={depth}"));
            }
            if let Some(max) = number("max_entries")? {
                parts.push(format!("max_entries={max}"));
            }
            format!(".ovfs_tree {}", parts.join(" "))
        }
        "ovfs_ls" => {
            chain(&mut parts)?;
            parts.push(ovfs_arg("path", &required("path")?)?);
            format!(".ovfs_ls {}", parts.join(" "))
        }
        "ovfs_versions" => {
            parts.push(ovfs_arg("client", &required("client")?)?);
            parts.push(ovfs_arg("path", &required("path")?)?);
            parts.push(ovfs_arg("name", &text("name")?.unwrap_or_default())?);
            if arguments.get("copies").and_then(Value::as_bool) == Some(true) {
                parts.push("copies".into());
            }
            format!(".ovfs_versions {}", parts.join(" "))
        }
        _ => return Ok(None),
    };
    Ok(Some(format!("{} json", command.trim_end())))
}
