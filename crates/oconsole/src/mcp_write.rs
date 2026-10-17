//! Typed write and restore MCP tools (S-31, S-36).
//!
//! Offered with write access (C-087): local stdio in every build (unless
//! `--mcp-read-only`), Streamable HTTP only in the development build with
//! `--mcp-allow-write`. The generic `execute` exists only in the development
//! build. Further write tools for the remaining commands (add, create,
//! export/import/move, update slots/job/stats, sqlquery, query, .sql, .bvfs_*,
//! .acl/.profile changes, configure delete/update, use, setdevice, session
//! options) are in mcp_write/admin.rs; the restore file selection in mcp_tree.rs.
//!
//! Route: over gRPC the typed DirectorApi RPC (mcp_typed.rs); a Director
//! that answers UNIMPLEMENTED (production build) gets the console line via
//! ExecuteCommand; over BNET the console line with its dialog.
//!
//! Every tool builds exactly one non-interactive bconsole command line
//! (`… yes` where the command knows it) from typed, validated arguments, so
//! no argument can smuggle in a second command or keyword. Questions the
//! Director still asks are answered only where the tool says so (e.g.
//! `Purge (yes/no)?` after `confirm: true`); every other question is
//! cancelled with "." and reported as an error.
//!
//! Destructive tools (cancel_job, relabel_volume, prune, purge, delete,
//! truncate) carry `destructiveHint`; relabel/prune/purge/delete/truncate
//! additionally require the argument `confirm: true`.

use std::sync::Mutex;

use serde_json::{Map, Value, json};

use crate::mcp::CommandRunner;
#[cfg(feature = "mcp-development")]
use crate::mcp::{run_plain, tool_result};
#[cfg(feature = "mcp-development")]
use crate::mcp_read::query::{command_arg, validate_command};
use crate::mcp_typed::{self, Outcome};

mod admin;

/// Every write tool for tools/list: the typed tools, the admin tools, the
/// restore file selection, then (development build) the generic `execute`.
pub fn all_tools() -> Vec<Value> {
    let mut all = tools();
    all.extend(admin::tools());
    all.extend(crate::mcp_tree::tools());
    #[cfg(feature = "mcp-development")]
    all.push(execute_tool());
    all
}

/// Whether `name` is a write tool of this build (for the BNET dialog guard).
pub fn is_tool(name: &str) -> bool {
    all_tools().iter().any(|t| t["name"] == name)
}

/// Schema of a typed write tool (tools() or admin::tools()).
fn schema(name: &str) -> Option<Value> {
    tools().into_iter().chain(admin::tools()).find(|t| t["name"] == name)
}

/// The generic `execute` tool: any single command line.
#[cfg(feature = "mcp-development")]
fn execute_tool() -> Value {
    json!({
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
    })
}

/// Handle a tools/call of a write tool; `None` if `name` is none of them.
/// The caller only asks with write access enabled.
pub fn call<R: CommandRunner>(
    name: &str,
    arguments: &Value,
    runner: &Mutex<R>,
) -> Option<Result<Value, (i64, String)>> {
    #[cfg(feature = "mcp-development")]
    if name == "execute" {
        let command = command_arg(arguments);
        return Some(match validate_command(command) {
            Err(msg) => Ok(tool_result(&msg, true)),
            Ok(()) => run_plain(runner, command),
        });
    }
    let plan = plan(name, arguments)?;
    Some(
        runner
            .lock()
            .map_err(|_| (-32603, "Director runner unavailable".to_string()))
            .map(|mut runner| {
                // Typed DirectorApi first (gRPC), only for valid arguments.
                if plan.is_ok() {
                    if let Some((rpc, request)) = mcp_typed::request(name, arguments) {
                        if let Outcome::Done(result) = mcp_typed::call(&mut *runner, rpc, &request) {
                            return result;
                        }
                    }
                }
                execute(&mut *runner, plan)
            }),
    )
}

/// One Director command and the questions the tool may answer.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub command: String,
    /// Questions answered with "yes".
    pub yes_to: &'static [&'static str],
    /// Further answers, used in order: (part of the question, reply); an
    /// empty part matches any question.
    pub answers: Vec<(&'static str, String)>,
    /// Over ExecuteCommand (gRPC, no dialog) append this word instead of
    /// answering (e.g. `yes` for GetConfirmation).
    pub grpc_suffix: Option<&'static str>,
    /// The command only works as a console dialog (its questions have no
    /// command-line form): over gRPC only the typed RPC can run it.
    pub dialog: bool,
}

impl Plan {
    fn new(command: String) -> Self {
        Self { command, yes_to: &[], answers: Vec::new(), grpc_suffix: None, dialog: false }
    }
}

const LEVELS: &[&str] = &[
    "Full", "Incremental", "Differential", "Since", "VirtualFull",
    "InitCatalog", "Catalog", "VolumeToCatalog", "DiskToCatalog", "Data",
];
const REPLACE: &[&str] = &["always", "ifnewer", "ifolder", "never"];
const CANCEL_STATES: &[&str] = &["created", "blocked", "waiting", "running"];
const VOLSTATUS: &[&str] = &[
    "Append", "Archive", "Disabled", "Full", "Used", "Cleaning", "Recycle", "Read-Only", "Error",
];
const ENABLED: &[&str] = &["yes", "no", "archived"];
const ACTION_ON_PURGE: &[&str] = &["None", "Truncate"];
const RESOURCE_KINDS: &[&str] = &["job", "client", "schedule"];
const CONFIGURE_TYPES: &[&str] = &[
    "client", "jobdefs", "job", "storage", "catalog", "schedule", "fileset", "pool",
    "messages", "counter", "profile", "console", "user",
];

// ---------------------------------------------------------------------------
// Tool metadata

fn annotations(read_only: bool, destructive: bool, idempotent: bool) -> Value {
    json!({
        "readOnlyHint": read_only,
        "destructiveHint": destructive,
        "idempotentHint": idempotent,
        "openWorldHint": false
    })
}

fn tool(name: &str, title: &str, description: &str, properties: Value, required: &[&str], hints: Value) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        },
        "annotations": hints,
    })
}

fn s(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}
fn e(values: &[&str], description: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}
fn i(min: i64, description: &str) -> Value {
    json!({ "type": "integer", "minimum": min, "description": description })
}
fn b(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}
fn ids(description: &str) -> Value {
    json!({ "type": "array", "items": { "type": "integer", "minimum": 1 }, "minItems": 1, "description": description })
}
fn confirm() -> Value {
    json!({ "type": "boolean", "const": true,
        "description": "must be true: the user explicitly confirmed this destructive action" })
}

/// The write and restore tools (development build with --mcp-allow-write).
pub fn tools() -> Vec<Value> {
    let storage_target = |extra: Value| {
        let mut p = json!({
            "storage": s("Storage resource name"),
            "drive": i(0, "drive number (default 0)"),
            "jobid": i(1, "use the storage of this running job instead"),
            "job": s("use the storage of the running job with this name"),
            "ujobid": s("use the storage of the running job with this unique name"),
        });
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    vec![
        tool("run_job", "Start a job",
            "Start a configured job now or at a given time (bconsole 'run … yes', never interactive). \
             Every override is optional; the job resource supplies the rest. Returns the Director's \
             reply with 'Job queued. JobId=N'; follow it with wait_job or list tools.",
            json!({
                "job": s("Job resource name"),
                "level": e(LEVELS, "level override (backup or verify level)"),
                "client": s("Client override"),
                "fileset": s("FileSet override"),
                "pool": s("Pool override"),
                "nextpool": s("Next Pool (VirtualFull, Copy, Migrate)"),
                "storage": s("Storage override"),
                "when": s("start time 'YYYY-MM-DD HH:MM:SS' (default: now)"),
                "since": s("with level Since: 'YYYY-MM-DD HH:MM:SS'"),
                "priority": i(1, "job priority (lower runs first)"),
                "spooldata": b("data spooling for this run"),
                "accurate": b("accurate mode for this run"),
                "jobid": i(1, "Verify/Copy/Migrate: the job to verify or copy"),
                "comment": s("comment stored with the job")
            }), &["job"], annotations(false, false, false)),
        tool("rerun_job", "Rerun failed jobs",
            "Start jobs again with their original parameters (bconsole 'rerun … yes'). \
             Give jobids, or since_jobid (optionally until_jobid), or days/hours (failed jobs of that period).",
            json!({
                "jobids": ids("JobIds to rerun"),
                "since_jobid": i(1, "all failed jobs from this JobId on"),
                "until_jobid": i(1, "upper bound for since_jobid"),
                "days": i(1, "failed jobs of the last n days"),
                "hours": i(1, "failed jobs of the last n hours")
            }), &[], annotations(false, false, false)),
        tool("cancel_job", "Cancel jobs",
            "Cancel running or waiting jobs (bconsole 'cancel … yes'). Exactly one of jobid, job, ujobid, \
             all=true or state. Confirm with the user before cancelling.",
            json!({
                "jobid": i(1, "JobId of the job to cancel"),
                "job": s("cancel the running jobs of this job name"),
                "ujobid": s("unique job name, e.g. Backup.2026-10-17_10.00.00_05"),
                "all": b("cancel all running and waiting jobs"),
                "state": e(CANCEL_STATES, "cancel all jobs in this state"),
                "storage": s("with jobid: cancel directly in this Storage daemon")
            }), &[], annotations(false, true, false)),
        tool("wait_job", "Wait for a job",
            "Block until the job has finished and return its JobStatus (bconsole 'wait'). \
             Exactly one of jobid, job, ujobid, or mount=true (wait until a job waits for a mount; \
             with timeout seconds). Holds the Director session while waiting.",
            json!({
                "jobid": i(1, "JobId"),
                "job": s("running job whose name starts with this"),
                "ujobid": s("unique job name"),
                "mount": b("wait until a running job waits for a volume mount"),
                "timeout": i(1, "with mount: give up after this many seconds")
            }), &[], annotations(true, false, true)),
        tool("restore", "Restore files",
            "Start a restore job in one non-interactive line ('restore … done yes'). Select the backups with \
             jobids, or with before (state at that time) or the latest state (default) of client+fileset. \
             Select files with all=true, files (full paths) or directories (with a trailing /; needs jobids), \
             or fileregex. Browse first with ovfs_latest / ovfs_tree / ovfs_ls / ovfs_versions to find paths. \
             Target: where (prefix, '/' = original place) or regexwhere or strip_prefix/add_prefix/add_suffix; \
             replace decides about existing files. Confirm with the user where files go and what replace does.",
            json!({
                "client": s("Client whose backup is restored (also the target unless restoreclient)"),
                "jobids": ids("JobIds whose files form the tree (Full, Differential, Incrementals)"),
                "fileset": s("FileSet for latest/before selection"),
                "before": s("state before 'YYYY-MM-DD HH:MM:SS' (instead of jobids)"),
                "all": b("restore every file of the selection"),
                "files": { "type": "array", "items": { "type": "string" }, "minItems": 1,
                    "description": "full file paths to restore (latest version within the selection)" },
                "directories": { "type": "array", "items": { "type": "string" }, "minItems": 1,
                    "description": "whole directories (path ending in /); needs jobids" },
                "fileregex": s("only files whose path matches this POSIX regex"),
                "where": s("target directory prefixed to every path; '/' restores in place"),
                "regexwhere": s("path rewrite, e.g. !/home/!/tmp/home/! (instead of where)"),
                "strip_prefix": s("remove this prefix from every path"),
                "add_prefix": s("put this prefix before every path"),
                "add_suffix": s("append this to every file name"),
                "replace": e(REPLACE, "existing files at the target"),
                "restoreclient": s("target client if different from client"),
                "restorejob": s("Restore job resource (needed if several exist)"),
                "storage": s("read from this Storage"),
                "pool": s("only jobs on volumes of this pool"),
                "comment": s("comment of the restore job")
            }), &["client"], annotations(false, true, false)),
        tool("label_volume", "Label a new volume",
            "Write a label to a new volume in a storage device and create it in the catalog \
             (bconsole 'label … yes').",
            json!({
                "storage": s("Storage resource"),
                "volume": s("new volume name (letters, digits, :.-_/)"),
                "pool": s("pool of the new volume"),
                "slot": i(0, "autochanger slot"),
                "drive": i(0, "drive number"),
                "encrypt": b("create a hardware-encryption key (LTO)")
            }), &["storage", "volume", "pool"], annotations(false, false, false)),
        tool("relabel_volume", "Relabel a volume",
            "Overwrite the label of an existing volume (Purged/Recycle status) with a new name: \
             its old contents are lost. Requires confirm=true.",
            json!({
                "storage": s("Storage resource"),
                "oldvolume": s("existing volume"),
                "volume": s("new volume name"),
                "pool": s("pool of the old volume"),
                "slot": i(0, "autochanger slot"),
                "drive": i(0, "drive number"),
                "encrypt": b("create a hardware-encryption key"),
                "confirm": confirm()
            }), &["storage", "oldvolume", "volume", "confirm"], annotations(false, true, false)),
        tool("mount_storage", "Mount a storage device",
            "Mount the volume in a storage device (bconsole 'mount'). Give storage or a running job.",
            storage_target(json!({ "slot": i(0, "autochanger slot to load first") })), &[],
            annotations(false, false, true)),
        tool("unmount_storage", "Unmount a storage device",
            "Unmount a storage device so the volume can be removed (bconsole 'unmount').",
            storage_target(json!({ "alldrives": b("all drives of the storage") })), &[],
            annotations(false, false, true)),
        tool("release_storage", "Release a storage device",
            "Release a storage device: close the volume, keep the device usable (bconsole 'release').",
            storage_target(json!({ "alldrives": b("all drives of the storage") })), &[],
            annotations(false, false, true)),
        tool("update_volume", "Change volume properties",
            "Change catalog properties of one volume (bconsole 'update volume=…'), or reset volumes from \
             their pool (frompool, allfrompool, fromallpools). At least one change.",
            json!({
                "volume": s("volume name"),
                "volstatus": e(VOLSTATUS, "new volume status"),
                "volretention": s("retention, e.g. '30 days'"),
                "voluse": s("use duration, e.g. '1 day'"),
                "maxvoljobs": i(0, "maximum jobs (0 = unlimited)"),
                "maxvolfiles": i(0, "maximum files"),
                "maxvolbytes": s("maximum size, e.g. 10G"),
                "recycle": b("may be recycled"),
                "inchanger": b("volume is in the autochanger"),
                "slot": i(0, "autochanger slot"),
                "enabled": e(ENABLED, "enabled for new jobs"),
                "pool": s("move to this pool"),
                "recyclepool": s("pool the volume goes to when recycled"),
                "actiononpurge": e(ACTION_ON_PURGE, "action when purged"),
                "storage": s("storage of the volume"),
                "frompool": b("reset the volume's defaults from its pool"),
                "allfrompool": s("reset all volumes of this pool from the pool (instead of volume)"),
                "fromallpools": b("reset all volumes of all pools (instead of volume)")
            }), &[], annotations(false, false, true)),
        tool("update_pool", "Update a pool from the configuration",
            "Write the Pool resource of the configuration into the catalog (bconsole 'update pool=…').",
            json!({ "pool": s("Pool resource name") }), &["pool"], annotations(false, false, true)),
        tool("prune", "Prune catalog records",
            "Remove catalog records whose retention has expired (bconsole 'prune … yes'): files, jobs, \
             volume (one, or all_volumes), stats or directory. Requires confirm=true.",
            json!({
                "kind": e(&["files", "jobs", "volume", "stats", "directory"], "what to prune"),
                "client": s("client (files, jobs, directory)"),
                "pool": s("pool (files, jobs, volume)"),
                "volume": s("volume (kind volume)"),
                "all_volumes": b("kind volume: all volumes (of pool)"),
                "directory": s("kind directory: path"),
                "recursive": b("kind directory: include subdirectories"),
                "confirm": confirm()
            }), &["kind", "confirm"], annotations(false, true, true)),
        tool("purge", "Purge catalog records",
            "Remove catalog records regardless of retention (bconsole 'purge'; the Director's \
             'Purge (yes/no)?' is answered yes): files of a job/client/volume, jobs of a client/volume/pool, \
             volumes (optionally with action truncate), or quota. Requires confirm=true.",
            json!({
                "kind": e(&["files", "jobs", "volume", "quota"], "what to purge"),
                "jobid": i(1, "files: of this job"),
                "job": s("files: of this job name"),
                "client": s("files/jobs/quota: of this client"),
                "volume": { "type": "array", "items": { "type": "string" }, "minItems": 1,
                    "description": "files/jobs: of this volume (one); volume: these volumes" },
                "pool": s("jobs: of this pool; volume with action: volumes of this pool"),
                "jobstatus": s("jobs/volume: only jobs with these status letters, comma-separated, e.g. E,f"),
                "action": e(&["truncate", "all"], "volume: run ActionOnPurge for purged volumes"),
                "storage": s("with action: storage"),
                "allpools": b("with action: volumes of all pools"),
                "drive": i(0, "with action: drive"),
                "confirm": confirm()
            }), &["kind", "confirm"], annotations(false, true, true)),
        tool("delete", "Delete catalog entries",
            "Delete a volume (and its jobs), a pool, jobs or a storage entry from the catalog \
             (bconsole 'delete'). Requires confirm=true.",
            json!({
                "kind": e(&["volume", "pool", "jobid", "storage"], "what to delete"),
                "volume": s("volume name"),
                "pool": s("pool name (kind pool; or the pool of the volume)"),
                "jobids": ids("JobIds (kind jobid)"),
                "storage": s("storage entry (kind storage)"),
                "confirm": confirm()
            }), &["kind", "confirm"], annotations(false, true, true)),
        tool("truncate", "Truncate purged volumes",
            "Free the space of volumes in status Purged (bconsole 'truncate volstatus=Purged … yes'). \
             Requires confirm=true.",
            json!({
                "storage": s("only volumes of this storage"),
                "pool": s("only volumes of this pool"),
                "volume": s("only this volume"),
                "drive": i(0, "drive"),
                "confirm": confirm()
            }), &["confirm"], annotations(false, true, true)),
        tool("enable", "Enable a job, client or schedule",
            "Enable a job, client or schedule in the running Director (until reload).",
            json!({ "resource": e(RESOURCE_KINDS, "resource type"), "name": s("resource name") }),
            &["resource", "name"], annotations(false, false, true)),
        tool("disable", "Disable a job, client or schedule",
            "Disable a job, client or schedule in the running Director (until reload): no scheduled runs.",
            json!({ "resource": e(RESOURCE_KINDS, "resource type"), "name": s("resource name") }),
            &["resource", "name"], annotations(false, false, true)),
        tool("configure_add", "Add a configuration resource",
            "Create a Director resource and write it to bareos-dir.d/<type>/<name>.conf \
             (bconsole 'configure add'). directives: lowercase directive names without spaces \
             (e.g. {\"address\":\"host\",\"password\":\"x\"}); an array value repeats the directive. \
             Mandatory per type: client address+password; job/jobdefs type, messages, pool (or jobdefs); \
             storage address, password, device, mediatype; catalog dbname, dbuser; console password.",
            json!({
                "resource": e(CONFIGURE_TYPES, "resource type"),
                "name": s("resource name"),
                "directives": { "type": "object", "description": "directive -> value (string, number, boolean or array)",
                    "additionalProperties": { "type": ["string", "number", "boolean", "array"] } }
            }), &["resource", "name"], annotations(false, false, false)),
        tool("configure_export", "Export a client's Director resource",
            "Write the Director resource a client's File Daemon needs (bconsole 'configure export client=…').",
            json!({ "client": s("Client resource name") }), &["client"], annotations(false, false, true)),
        tool("reload", "Reload the Director configuration",
            "Re-read the Director configuration (bconsole 'reload'). Running jobs keep their resources.",
            json!({}), &[], annotations(false, false, true)),
        tool("setdebug", "Set a daemon's debug level",
            "Set the debug level of the Director, a File or Storage daemon, or all (bconsole 'setdebug').",
            json!({
                "level": i(0, "debug level, 0 = off"),
                "target": e(&["dir", "client", "storage", "all"], "which daemon"),
                "name": s("client or storage name (target client/storage)"),
                "trace": b("write debug output to a trace file"),
                "timestamp": b("timestamp every debug line"),
                "hangup": i(0, "File daemon test aid: drop the connection after n files")
            }), &["level", "target"], annotations(false, false, true)),
        tool("setbandwidth", "Limit bandwidth",
            "Set the bandwidth limit (kB/s, 0 = none) of a client, a storage or running jobs (bconsole \
             'setbandwidth'). Exactly one of client, storage, jobid, job, ujobid, all=true, state.",
            json!({
                "limit": i(0, "limit in kB/s, 0 removes it"),
                "client": s("Client resource"),
                "storage": s("Storage resource"),
                "jobid": i(1, "running job"),
                "job": s("running job name"),
                "ujobid": s("unique job name"),
                "all": b("all running jobs"),
                "state": e(CANCEL_STATES, "running jobs in this state")
            }), &["limit"], annotations(false, false, true)),
    ]
}

// ---------------------------------------------------------------------------
// Argument handling

struct Args<'a> {
    map: Map<String, Value>,
    tool: &'a str,
}

impl<'a> Args<'a> {
    fn new(tool: &'a str, arguments: &Value) -> Result<Self, String> {
        match arguments {
            Value::Object(map) => Ok(Self { map: map.clone(), tool }),
            Value::Null => Ok(Self { map: Map::new(), tool }),
            _ => Err("arguments must be an object".into()),
        }
    }

    /// Unknown argument names are errors, not silently ignored.
    fn check_known(&self) -> Result<(), String> {
        let tool = schema(self.tool);
        let known = tool.as_ref().and_then(|t| t["inputSchema"]["properties"].as_object());
        for key in self.map.keys() {
            if !known.is_some_and(|k| k.contains_key(key)) {
                return Err(format!("unknown argument '{key}' for {}", self.tool));
            }
        }
        Ok(())
    }

    fn has(&self, key: &str) -> bool {
        !matches!(self.map.get(key), None | Some(Value::Null))
    }

    fn text(&self, key: &str) -> Result<Option<String>, String> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(v)) if v.is_empty() => Err(format!("{key} must not be empty")),
            Some(Value::String(v)) if v.chars().any(char::is_control) => {
                Err(format!("{key} must be a single line without control characters"))
            }
            Some(Value::String(v)) => Ok(Some(v.clone())),
            Some(_) => Err(format!("{key} must be a string")),
        }
    }

    fn int(&self, key: &str, min: i64) -> Result<Option<i64>, String> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_i64() {
                Some(n) if n >= min => Ok(Some(n)),
                _ => Err(format!("{key} must be an integer >= {min}")),
            },
        }
    }

    fn flag(&self, key: &str) -> Result<bool, String> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(v)) => Ok(*v),
            Some(_) => Err(format!("{key} must be a boolean")),
        }
    }

    fn opt_bool(&self, key: &str) -> Result<Option<bool>, String> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(v)) => Ok(Some(*v)),
            Some(_) => Err(format!("{key} must be a boolean")),
        }
    }

    fn choice(&self, key: &str, values: &[&str]) -> Result<Option<String>, String> {
        let Some(v) = self.text(key)? else { return Ok(None) };
        values
            .iter()
            .find(|c| c.eq_ignore_ascii_case(&v))
            .map(|c| Some(c.to_string()))
            .ok_or_else(|| format!("{key} must be one of: {}", values.join(", ")))
    }

    fn ids(&self, key: &str) -> Result<Option<Vec<i64>>, String> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) if !items.is_empty() => items
                .iter()
                .map(|v| v.as_i64().filter(|n| *n >= 1))
                .collect::<Option<Vec<_>>>()
                .map(Some)
                .ok_or_else(|| format!("{key} must be a list of JobIds (integers >= 1)")),
            Some(v) if v.as_i64().is_some_and(|n| n >= 1) => Ok(Some(vec![v.as_i64().unwrap()])),
            Some(_) => Err(format!("{key} must be a non-empty list of JobIds")),
        }
    }

    fn strings(&self, key: &str) -> Result<Option<Vec<String>>, String> {
        match self.map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(v)) => Ok(Some(vec![v.clone()])),
            Some(Value::Array(items)) if !items.is_empty() => items
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()
                .map(Some)
                .ok_or_else(|| format!("{key} must be a list of strings")),
            Some(_) => Err(format!("{key} must be a non-empty list of strings")),
        }
    }

    fn confirmed(&self) -> Result<(), String> {
        if self.map.get("confirm") == Some(&Value::Bool(true)) {
            Ok(())
        } else {
            Err(format!(
                "{} is destructive: ask the user, then call it again with confirm=true",
                self.tool
            ))
        }
    }
}

/// Bareos resource and volume names: letters, digits, `:.-_/` and blanks
/// inside, at most 127 characters. Blanks need quotes on the command line.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 127
        && name.trim() == name
        && !name.starts_with('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '-' | '_' | '/' | ' '))
}

fn name_arg(key: &str, value: &str) -> Result<String, String> {
    if !valid_name(value) {
        return Err(format!(
            "{key} '{value}' is not a valid name (letters, digits, ':.-_/' and inner blanks)"
        ));
    }
    Ok(if value.contains(' ') { format!("{key}=\"{value}\"") } else { format!("{key}={value}") })
}

/// `key="value"` with the console's quoting (backslash escapes); free text
/// such as paths, regexes and comments.
fn quoted(key: &str, value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err(format!("{key} must not contain control characters"));
    }
    Ok(format!("{key}=\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// 'YYYY-MM-DD HH:MM:SS' (seconds optional).
fn datetime(key: &str, value: &str) -> Result<String, String> {
    let b = value.as_bytes();
    let digit = |r: std::ops::Range<usize>| r.into_iter().all(|i| b.get(i).is_some_and(u8::is_ascii_digit));
    let ok = (b.len() == 16 || b.len() == 19)
        && digit(0..4) && b[4] == b'-' && digit(5..7) && b[7] == b'-' && digit(8..10)
        && b[10] == b' ' && digit(11..13) && b[13] == b':' && digit(14..16)
        && (b.len() == 16 || (b[16] == b':' && digit(17..19)));
    if !ok {
        return Err(format!("{key} must be 'YYYY-MM-DD HH:MM:SS'"));
    }
    Ok(format!("{key}=\"{value}\""))
}

/// Durations ('30 days') and sizes ('10G'): letters, digits, blanks.
fn measure(key: &str, value: &str) -> Result<String, String> {
    if value.trim().is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '.') {
        return Err(format!("{key} must be a number with unit, e.g. '30 days' or '10G'"));
    }
    Ok(format!("{key}=\"{}\"", value.trim()))
}

fn yes_no(v: bool) -> &'static str {
    if v { "yes" } else { "no" }
}

fn id_list(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

/// Exactly one of the given selectors must be present.
fn one_of(args: &Args, keys: &[&str]) -> Result<String, String> {
    let present: Vec<_> = keys.iter().filter(|k| args.has(k) && args.map.get(**k) != Some(&Value::Bool(false))).collect();
    match present.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(format!("one of {} is required", keys.join(", "))),
        _ => Err(format!("only one of {} may be given", keys.join(", "))),
    }
}

// ---------------------------------------------------------------------------
// Command lines

/// The plan of a write tool; `None` if `tool` is not one of them.
pub fn plan(tool: &str, arguments: &Value) -> Option<Result<Plan, String>> {
    let builder: fn(&Args) -> Result<Plan, String> = match tool {
        "run_job" => run_job,
        "rerun_job" => rerun_job,
        "cancel_job" => cancel_job,
        "wait_job" => wait_job,
        "restore" => restore,
        "label_volume" => label_volume,
        "relabel_volume" => relabel_volume,
        "mount_storage" => |a| storage_cmd(a, "mount"),
        "unmount_storage" => |a| storage_cmd(a, "unmount"),
        "release_storage" => |a| storage_cmd(a, "release"),
        "update_volume" => update_volume,
        "update_pool" => |a| Ok(Plan::new(format!("update {}", name_arg("pool", &req(a, "pool")?)?))),
        "prune" => prune,
        "purge" => purge,
        "delete" => delete,
        "truncate" => truncate,
        "enable" => |a| toggle(a, "enable"),
        "disable" => |a| toggle(a, "disable"),
        "configure_add" => configure_add,
        "configure_export" => |a| {
            Ok(Plan::new(format!("configure export {}", name_arg("client", &req(a, "client")?)?)))
        },
        "reload" => |_| Ok(Plan::new("reload".into())),
        "setdebug" => setdebug,
        "setbandwidth" => setbandwidth,
        _ => return admin::plan(tool, arguments),
    };
    Some(Args::new(tool, arguments).and_then(|args| {
        args.check_known()?;
        builder(&args)
    }))
}

fn req(args: &Args, key: &str) -> Result<String, String> {
    args.text(key)?.ok_or_else(|| format!("{key} is required"))
}

fn push_name(parts: &mut Vec<String>, args: &Args, key: &str) -> Result<(), String> {
    if let Some(v) = args.text(key)? {
        parts.push(name_arg(key, &v)?);
    }
    Ok(())
}

fn push_int(parts: &mut Vec<String>, args: &Args, key: &str, min: i64) -> Result<(), String> {
    if let Some(v) = args.int(key, min)? {
        parts.push(format!("{key}={v}"));
    }
    Ok(())
}

fn run_job(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["run".to_string(), name_arg("job", &req(a, "job")?)?];
    if let Some(level) = a.choice("level", LEVELS)? {
        p.push(format!("level={level}"));
    }
    for key in ["client", "fileset", "pool", "nextpool", "storage"] {
        push_name(&mut p, a, key)?;
    }
    for key in ["when", "since"] {
        if let Some(v) = a.text(key)? {
            p.push(datetime(key, &v)?);
        }
    }
    push_int(&mut p, a, "priority", 1)?;
    push_int(&mut p, a, "jobid", 1)?;
    for key in ["spooldata", "accurate"] {
        if let Some(v) = a.opt_bool(key)? {
            p.push(format!("{key}={}", yes_no(v)));
        }
    }
    if let Some(v) = a.text("comment")? {
        if v.contains('\'') {
            return Err("comment must not contain '".into());
        }
        p.push(quoted("comment", &v)?);
    }
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn rerun_job(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["rerun".to_string()];
    let jobids = a.ids("jobids")?;
    let since = a.int("since_jobid", 1)?;
    let until = a.int("until_jobid", 1)?;
    let days = a.int("days", 1)?;
    let hours = a.int("hours", 1)?;
    match (jobids, since, days.is_some() || hours.is_some()) {
        (Some(ids), None, false) if until.is_none() => p.push(format!("jobid={}", id_list(&ids))),
        (None, Some(since), false) => {
            p.push(format!("since_jobid={since}"));
            if let Some(until) = until {
                p.push(format!("until_jobid={until}"));
            }
        }
        (None, None, true) if until.is_none() => {
            if let Some(d) = days {
                p.push(format!("days={d}"));
            }
            if let Some(h) = hours {
                p.push(format!("hours={h}"));
            }
        }
        _ => return Err("give jobids, or since_jobid [until_jobid], or days/hours".into()),
    }
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn cancel_job(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["cancel".to_string()];
    match one_of(a, &["jobid", "job", "ujobid", "all", "state"])?.as_str() {
        "jobid" => push_int(&mut p, a, "jobid", 1)?,
        "all" => p.push("all".into()),
        "state" => p.push(format!("state={}", a.choice("state", CANCEL_STATES)?.unwrap())),
        key => push_name(&mut p, a, key)?,
    }
    if a.has("storage") {
        if !a.has("jobid") {
            return Err("storage needs jobid".into());
        }
        push_name(&mut p, a, "storage")?;
    }
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn wait_job(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["wait".to_string()];
    match one_of(a, &["jobid", "job", "ujobid", "mount"])?.as_str() {
        "jobid" => push_int(&mut p, a, "jobid", 1)?,
        "mount" => {
            a.flag("mount")?;
            p.push("mount".into());
            push_int(&mut p, a, "timeout", 1)?;
        }
        key => push_name(&mut p, a, key)?,
    }
    if a.has("timeout") && !a.has("mount") {
        return Err("timeout only with mount".into());
    }
    Ok(Plan::new(p.join(" ")))
}

fn restore(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["restore".to_string()];
    let jobids = a.ids("jobids")?;
    let before = a.text("before")?;
    match (&jobids, &before) {
        (Some(ids), None) => p.push(format!("jobid={}", id_list(ids))),
        (None, Some(date)) => {
            p.push("select".into());
            p.push(datetime("before", date)?);
        }
        (None, None) if !a.has("files") => {
            p.push("select".into());
            p.push("current".into());
        }
        (None, None) => {}
        (Some(_), Some(_)) => return Err("give jobids or before, not both".into()),
    }
    p.push(name_arg("client", &req(a, "client")?)?);
    for key in ["fileset", "pool"] {
        push_name(&mut p, a, key)?;
    }
    if jobids.is_none() && (before.is_some() || !a.has("files")) && !a.has("fileset") {
        return Err("fileset is required to select the latest backup or the state before a date".into());
    }
    let all = a.flag("all")?;
    let files = a.strings("files")?;
    let dirs = a.strings("directories")?;
    let regex = a.text("fileregex")?;
    let selections = [all, files.is_some(), dirs.is_some(), regex.is_some()].iter().filter(|x| **x).count();
    if selections != 1 {
        return Err("select files with exactly one of all=true, files, directories or fileregex".into());
    }
    if all {
        p.push("all".into());
    }
    for file in files.iter().flatten() {
        if !file.starts_with('/') && !file.contains(":/") {
            return Err(format!("file '{file}' must be a full path"));
        }
        if file.starts_with('<') || file.starts_with('?') {
            return Err("files must be paths".into());
        }
        p.push(quoted("file", file)?);
    }
    if let Some(dirs) = &dirs {
        if jobids.is_none() {
            return Err("directories needs jobids".into());
        }
        for dir in dirs {
            if !dir.ends_with('/') {
                return Err(format!("directory '{dir}' must end with /"));
            }
            p.push(quoted("directory", dir)?);
        }
    }
    if let Some(r) = &regex {
        p.push(quoted("fileregex", r)?);
    }
    p.push("done".into());
    let target = ["where", "regexwhere"].iter().filter(|k| a.has(k)).count()
        + usize::from(["strip_prefix", "add_prefix", "add_suffix"].iter().any(|k| a.has(k)));
    if target > 1 {
        return Err("give only one of where, regexwhere or strip_prefix/add_prefix/add_suffix".into());
    }
    for key in ["where", "regexwhere", "strip_prefix", "add_prefix", "add_suffix"] {
        if let Some(v) = a.text(key)? {
            p.push(quoted(key, &v)?);
        }
    }
    if let Some(r) = a.choice("replace", REPLACE)? {
        p.push(format!("replace={r}"));
    }
    for key in ["restoreclient", "restorejob", "storage"] {
        push_name(&mut p, a, key)?;
    }
    if let Some(v) = a.text("comment")? {
        p.push(quoted("comment", &v)?);
    }
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn label_volume(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["label".to_string()];
    for key in ["storage", "volume", "pool"] {
        p.push(name_arg(key, &req(a, key)?)?);
    }
    push_int(&mut p, a, "slot", 0)?;
    push_int(&mut p, a, "drive", 0)?;
    if a.flag("encrypt")? {
        p.push("encrypt".into());
    }
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn relabel_volume(a: &Args) -> Result<Plan, String> {
    a.confirmed()?;
    let mut p = vec!["relabel".to_string()];
    for key in ["storage", "oldvolume", "volume"] {
        p.push(name_arg(key, &req(a, key)?)?);
    }
    push_name(&mut p, a, "pool")?;
    push_int(&mut p, a, "slot", 0)?;
    push_int(&mut p, a, "drive", 0)?;
    if a.flag("encrypt")? {
        p.push("encrypt".into());
    }
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn storage_cmd(a: &Args, command: &str) -> Result<Plan, String> {
    let mut p = vec![command.to_string()];
    match one_of(a, &["storage", "jobid", "job", "ujobid"])?.as_str() {
        "jobid" => push_int(&mut p, a, "jobid", 1)?,
        key => push_name(&mut p, a, key)?,
    }
    push_int(&mut p, a, "drive", 0)?;
    push_int(&mut p, a, "slot", 0)?;
    if a.flag("alldrives")? {
        p.push("alldrives".into());
    }
    Ok(Plan::new(p.join(" ")))
}

fn update_volume(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["update".to_string()];
    match one_of(a, &["volume", "allfrompool", "fromallpools"])?.as_str() {
        "volume" => p.push(name_arg("volume", &req(a, "volume")?)?),
        "allfrompool" => {
            p.push("volume".into());
            p.push(name_arg("allfrompool", &req(a, "allfrompool")?)?);
        }
        _ => {
            a.flag("fromallpools")?;
            p.push("volume fromallpools".into());
        }
    }
    let base = p.len();
    if let Some(v) = a.choice("volstatus", VOLSTATUS)? {
        p.push(format!("volstatus={v}"));
    }
    for key in ["volretention", "voluse", "maxvolbytes"] {
        if let Some(v) = a.text(key)? {
            p.push(measure(key, &v)?);
        }
    }
    for key in ["maxvoljobs", "maxvolfiles", "slot"] {
        push_int(&mut p, a, key, 0)?;
    }
    for key in ["recycle", "inchanger"] {
        if let Some(v) = a.opt_bool(key)? {
            p.push(format!("{key}={}", yes_no(v)));
        }
    }
    if let Some(v) = a.choice("enabled", ENABLED)? {
        p.push(format!("enabled={v}"));
    }
    if let Some(v) = a.choice("actiononpurge", ACTION_ON_PURGE)? {
        p.push(format!("actiononpurge={v}"));
    }
    for key in ["pool", "recyclepool", "storage"] {
        push_name(&mut p, a, key)?;
    }
    if a.flag("frompool")? {
        p.push("frompool=yes".into());
    }
    if a.has("volume") && p.len() == base {
        return Err("give at least one property to change".into());
    }
    if !a.has("volume") && p.len() > base {
        return Err("allfrompool/fromallpools take no other properties".into());
    }
    Ok(Plan::new(p.join(" ")))
}

fn prune(a: &Args) -> Result<Plan, String> {
    a.confirmed()?;
    let kind = a.choice("kind", &["files", "jobs", "volume", "stats", "directory"])?.ok_or("kind is required")?;
    let mut p = vec!["prune".to_string()];
    let allowed: &[&str] = match kind.as_str() {
        "files" | "jobs" => &["client", "pool"],
        "volume" => &["pool", "volume", "all_volumes"],
        "stats" => &[],
        _ => &["client", "directory", "recursive"],
    };
    for key in ["client", "pool", "volume", "all_volumes", "directory", "recursive"] {
        if a.has(key) && !allowed.contains(&key) {
            return Err(format!("{key} does not apply to prune {kind}"));
        }
    }
    match kind.as_str() {
        "volume" => match (a.text("volume")?, a.flag("all_volumes")?) {
            (Some(v), false) => p.push(name_arg("volume", &v)?),
            (None, true) => p.push("volume all".into()),
            _ => return Err("prune volume needs volume or all_volumes=true".into()),
        },
        "directory" => {
            let dir = a.text("directory")?.ok_or("prune directory needs directory")?;
            p.push(quoted("directory", &dir)?);
            if a.text("client")?.is_none() {
                return Err("prune directory needs client".into());
            }
        }
        "files" | "jobs" => {
            p.push(kind.clone());
            if a.text("client")?.is_none() && a.text("pool")?.is_none() {
                return Err(format!("prune {kind} needs client or pool"));
            }
        }
        _ => p.push(kind.clone()),
    }
    push_name(&mut p, a, "client")?;
    push_name(&mut p, a, "pool")?;
    if a.flag("recursive")? {
        p.push("recursive".into());
    }
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn jobstatus(value: &str) -> Result<String, String> {
    if value.split(',').all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())) {
        Ok(format!("jobstatus={value}"))
    } else {
        Err("jobstatus must be status letters or names, comma-separated".into())
    }
}

fn purge(a: &Args) -> Result<Plan, String> {
    a.confirmed()?;
    let kind = a.choice("kind", &["files", "jobs", "volume", "quota"])?.ok_or("kind is required")?;
    let volumes = a.strings("volume")?;
    let mut p = vec!["purge".to_string(), kind.clone()];
    let allowed: &[&str] = match kind.as_str() {
        "files" => &["jobid", "job", "client", "volume"],
        "jobs" => &["client", "volume", "pool", "jobstatus"],
        "volume" => &["volume", "jobstatus", "action", "storage", "pool", "allpools", "drive"],
        _ => &["client"],
    };
    for key in ["jobid", "job", "client", "volume", "pool", "jobstatus", "action", "storage", "allpools", "drive"] {
        if a.has(key) && a.map.get(key) != Some(&Value::Bool(false)) && !allowed.contains(&key) {
            return Err(format!("{key} does not apply to purge {kind}"));
        }
    }
    match kind.as_str() {
        "files" => {
            one_of(a, &["jobid", "job", "client", "volume"])?;
            push_int(&mut p, a, "jobid", 1)?;
            push_name(&mut p, a, "job")?;
            push_name(&mut p, a, "client")?;
        }
        "jobs" => {
            one_of(a, &["client", "volume", "pool"])?;
            push_name(&mut p, a, "client")?;
            push_name(&mut p, a, "pool")?;
        }
        "volume" => {
            if volumes.is_none() && !a.has("action") {
                return Err("purge volume needs volume (or action)".into());
            }
        }
        _ => {
            push_name(&mut p, a, "client")?;
        }
    }
    if let Some(volumes) = volumes {
        if kind != "volume" && volumes.len() > 1 {
            return Err(format!("purge {kind} takes one volume"));
        }
        // `purge volume=x`: the first value belongs to the keyword itself
        if kind == "volume" {
            p.pop();
        }
        for v in &volumes {
            p.push(name_arg("volume", v)?);
        }
    }
    if let Some(s) = a.text("jobstatus")? {
        p.push(jobstatus(&s)?);
    }
    if kind == "volume" {
        if let Some(action) = a.choice("action", &["truncate", "all"])? {
            p.push(format!("action={action}"));
            push_name(&mut p, a, "storage")?;
            push_name(&mut p, a, "pool")?;
            if a.flag("allpools")? {
                p.push("allpools".into());
            }
            push_int(&mut p, a, "drive", 0)?;
        } else if ["storage", "pool", "allpools", "drive"].iter().any(|k| a.has(k)) {
            return Err("storage, pool, allpools and drive need action".into());
        }
    }
    Ok(Plan { command: p.join(" "), yes_to: &["Purge (yes/no)?"], answers: Vec::new(), grpc_suffix: Some("yes"), dialog: false })
}

fn delete(a: &Args) -> Result<Plan, String> {
    a.confirmed()?;
    let kind = a.choice("kind", &["volume", "pool", "jobid", "storage"])?.ok_or("kind is required")?;
    let dialog = kind == "pool";
    let p = match kind.as_str() {
        "volume" => {
            let mut p = vec!["delete".to_string(), name_arg("volume", &req(a, "volume")?)?];
            push_name(&mut p, a, "pool")?;
            p.push("yes".into());
            p
        }
        "pool" => vec!["delete".to_string(), name_arg("pool", &req(a, "pool")?)?],
        "jobid" => {
            let ids = a.ids("jobids")?.ok_or("jobids is required")?;
            vec!["delete".to_string(), format!("jobid={}", id_list(&ids))]
        }
        _ => vec!["delete".to_string(), name_arg("storage", &req(a, "storage")?)?],
    };
    let used: &[&str] = match kind.as_str() {
        "volume" => &["volume", "pool"],
        "pool" => &["pool"],
        "jobid" => &["jobids"],
        _ => &["storage"],
    };
    for key in ["volume", "pool", "jobids", "storage"] {
        if a.has(key) && !used.contains(&key) {
            return Err(format!("{key} does not apply to delete {kind}"));
        }
    }
    Ok(Plan {
        command: p.join(" "),
        yes_to: &["Are you sure you want to delete"],
        answers: Vec::new(),
        grpc_suffix: None,
        dialog,
    })
}

fn truncate(a: &Args) -> Result<Plan, String> {
    a.confirmed()?;
    let mut p = vec!["truncate".to_string(), "volstatus=Purged".to_string()];
    for key in ["storage", "pool", "volume"] {
        push_name(&mut p, a, key)?;
    }
    push_int(&mut p, a, "drive", 0)?;
    p.push("yes".into());
    Ok(Plan::new(p.join(" ")))
}

fn toggle(a: &Args, command: &str) -> Result<Plan, String> {
    let kind = a.choice("resource", RESOURCE_KINDS)?.ok_or("resource is required")?;
    Ok(Plan::new(format!("{command} {}", name_arg(&kind, &req(a, "name")?)?)))
}

fn configure_add(a: &Args) -> Result<Plan, String> {
    let kind = a.choice("resource", CONFIGURE_TYPES)?.ok_or("resource is required")?;
    let mut p = vec!["configure add".to_string(), kind, name_arg("name", &req(a, "name")?)?];
    match a.map.get("directives") {
        None | Some(Value::Null) => {}
        Some(Value::Object(directives)) => {
            for (key, value) in directives {
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) || key == "name" {
                    return Err(format!("directive '{key}': lowercase letters and digits only, without spaces (not name)"));
                }
                let values = match value {
                    Value::Array(items) if !items.is_empty() => items.clone(),
                    Value::Array(_) => return Err(format!("directive '{key}': empty list")),
                    v => vec![v.clone()],
                };
                for v in values {
                    let text = match v {
                        Value::String(s) => s,
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => yes_no(b).to_string(),
                        _ => return Err(format!("directive '{key}': string, number or boolean")),
                    };
                    if text.contains('@') || text.contains(';') {
                        return Err(format!("directive '{key}': '@' and ';' are not allowed"));
                    }
                    p.push(quoted(key, &text)?);
                }
            }
        }
        Some(_) => return Err("directives must be an object".into()),
    }
    Ok(Plan::new(p.join(" ")))
}

fn setdebug(a: &Args) -> Result<Plan, String> {
    let level = a.int("level", 0)?.ok_or("level is required")?;
    let mut p = vec!["setdebug".to_string(), format!("level={level}")];
    for key in ["trace", "timestamp"] {
        if let Some(v) = a.opt_bool(key)? {
            p.push(format!("{key}={}", u8::from(v)));
        }
    }
    let target = a.choice("target", &["dir", "client", "storage", "all"])?.ok_or("target is required")?;
    if a.has("hangup") && !matches!(target.as_str(), "client" | "all") {
        return Err("hangup only for target client or all".into());
    }
    push_int(&mut p, a, "hangup", 0)?;
    match target.as_str() {
        "client" | "storage" => p.push(name_arg(&target, &a.text("name")?.ok_or("name is required for client/storage")?)?),
        _ if a.has("name") => return Err("name only for target client or storage".into()),
        t => p.push(t.to_string()),
    }
    Ok(Plan::new(p.join(" ")))
}

fn setbandwidth(a: &Args) -> Result<Plan, String> {
    let limit = a.int("limit", 0)?.ok_or("limit is required")?;
    let mut p = vec!["setbandwidth".to_string(), format!("limit={limit}")];
    match one_of(a, &["client", "storage", "jobid", "job", "ujobid", "all", "state"])?.as_str() {
        "jobid" => push_int(&mut p, a, "jobid", 1)?,
        "all" => p.push("all".into()),
        "state" => p.push(format!("state={}", a.choice("state", CANCEL_STATES)?.unwrap())),
        key => push_name(&mut p, a, key)?,
    }
    Ok(Plan::new(p.join(" ")))
}

// ---------------------------------------------------------------------------
// Execution

/// The last line of the output so far: the Director's question.
fn question(output: &str) -> &str {
    output.trim_end().rsplit('\n').next().unwrap_or("").trim()
}

/// Run a plan: questions listed in `yes_to` are answered "yes", any other
/// question is cancelled with "." and makes the result an error.
pub fn execute<R: CommandRunner + ?Sized>(runner: &mut R, plan: Result<Plan, String>) -> Value {
    let plan = match plan {
        Ok(plan) => plan,
        Err(msg) => return result(&msg, true, None),
    };
    if plan.dialog && runner.is_grpc() {
        return result(&format!(
            "'{}' needs the Director's console dialog, which ExecuteCommand cannot answer, and this Director \
             does not offer the typed RPC (production build). Connect oconsole via BNET for this tool{}.",
            plan.command,
            if plan.command == "sqlquery" { ", or use dot_sql" } else { "" }), true, None);
    }
    let command = match plan.grpc_suffix {
        Some(word) if runner.is_grpc() => format!("{} {word}", plan.command),
        _ => plan.command.clone(),
    };
    let mut cancelled: Vec<String> = Vec::new();
    let mut answers = plan.answers.iter();
    let mut answer = |output: &str| {
        let q = question(output);
        if plan.yes_to.iter().any(|p| q.contains(p)) {
            return Some("yes".to_string());
        }
        let next = answers.clone().next();
        match next {
            Some((part, reply)) if part.is_empty() || q.contains(part) => {
                answers.next();
                Some(reply.clone())
            }
            _ => {
                cancelled.push(q.to_string());
                None
            }
        }
    };
    match runner.run_dialog(&command, &mut answer) {
        Ok(out) => {
            let jobids = queued_jobids(&out);
            let mut text = if out.trim().is_empty() { "(no output)".to_string() } else { out };
            if !cancelled.is_empty() {
                text.push_str(&format!(
                    "\n[oconsole] The Director asked '{}', which this tool does not answer; it was cancelled. \
                     Give the missing argument (e.g. a valid name) and call the tool again.",
                    cancelled[0]
                ));
            }
            text = format!("> {command}\n{text}");
            result(&text, !cancelled.is_empty(), (!jobids.is_empty()).then_some(jobids))
        }
        Err(e) => result(&format!("Director error: {e}"), true, None),
    }
}

/// JobIds of started jobs ("Job queued. JobId=N", "Job started. JobId=N").
fn queued_jobids(output: &str) -> Vec<i64> {
    output
        .lines()
        .filter(|l| l.starts_with("Job queued.") || l.starts_with("Job started."))
        .filter_map(|l| l.rsplit("JobId=").next()?.trim().parse().ok())
        .collect()
}

fn result(text: &str, is_error: bool, jobids: Option<Vec<i64>>) -> Value {
    let mut r = json!({ "content": [{ "type": "text", "text": text }], "isError": is_error });
    if let Some(jobids) = jobids {
        r["structuredContent"] = json!({ "jobids": jobids });
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(tool: &str, args: Value) -> String {
        plan(tool, &args).expect("write tool").expect("valid").command
    }

    fn err(tool: &str, args: Value) -> String {
        plan(tool, &args).expect("write tool").expect_err("must be rejected")
    }

    #[test]
    fn schemas_are_complete_and_annotated() {
        let tools = tools();
        assert_eq!(tools.len(), 23);
        let destructive = ["cancel_job", "restore", "relabel_volume", "prune", "purge", "delete", "truncate"];
        for t in &tools {
            let name = t["name"].as_str().unwrap();
            assert!(plan(name, &json!({})).is_some(), "{name} has a builder");
            assert!(t["title"].is_string() && t["description"].is_string());
            assert_eq!(t["inputSchema"]["type"], "object");
            assert_eq!(t["annotations"]["destructiveHint"], destructive.contains(&name), "{name}");
            for r in t["inputSchema"]["required"].as_array().unwrap() {
                assert!(t["inputSchema"]["properties"].get(r.as_str().unwrap()).is_some(), "{name}: {r}");
            }
            let needs_confirm = ["relabel_volume", "prune", "purge", "delete", "truncate"].contains(&name);
            assert_eq!(t["inputSchema"]["required"].as_array().unwrap().contains(&json!("confirm")), needs_confirm, "{name}");
        }
        assert_eq!(tools.iter().filter(|t| t["annotations"]["readOnlyHint"] == true).count(), 1, "only wait_job reads");
        assert!(plan("query", &json!({})).is_none());
        assert!(plan("list_jobs", &json!({})).is_none());
    }

    #[test]
    fn run_and_job_control_lines() {
        assert_eq!(line("run_job", json!({"job":"Backup"})), "run job=Backup yes");
        assert_eq!(
            line("run_job", json!({"job":"Backup Linux","level":"incremental","client":"fd","fileset":"Set",
                "pool":"Full","storage":"File","when":"2026-10-18 03:00:00","priority":5,"spooldata":true,
                "accurate":false,"comment":"by \"MCP\""})),
            "run job=\"Backup Linux\" level=Incremental client=fd fileset=Set pool=Full storage=File \
             when=\"2026-10-18 03:00:00\" priority=5 spooldata=yes accurate=no comment=\"by \\\"MCP\\\"\" yes"
        );
        assert_eq!(line("rerun_job", json!({"jobids":[3,4]})), "rerun jobid=3,4 yes");
        assert_eq!(line("rerun_job", json!({"since_jobid":3,"until_jobid":9})), "rerun since_jobid=3 until_jobid=9 yes");
        assert_eq!(line("rerun_job", json!({"days":1,"hours":2})), "rerun days=1 hours=2 yes");
        assert_eq!(line("cancel_job", json!({"jobid":7})), "cancel jobid=7 yes");
        assert_eq!(line("cancel_job", json!({"state":"waiting"})), "cancel state=waiting yes");
        assert_eq!(line("cancel_job", json!({"all":true})), "cancel all yes");
        assert_eq!(line("cancel_job", json!({"jobid":7,"storage":"File"})), "cancel jobid=7 storage=File yes");
        assert_eq!(line("wait_job", json!({"jobid":7})), "wait jobid=7");
        assert_eq!(line("wait_job", json!({"mount":true,"timeout":30})), "wait mount timeout=30");
    }

    #[test]
    fn restore_lines() {
        assert_eq!(
            line("restore", json!({"client":"fd","jobids":[1,2],"all":true,"where":"/tmp/r","replace":"never","restorejob":"RestoreFiles"})),
            "restore jobid=1,2 client=fd all done where=\"/tmp/r\" replace=never restorejob=RestoreFiles yes"
        );
        assert_eq!(
            line("restore", json!({"client":"fd","fileset":"Set","all":true})),
            "restore select current client=fd fileset=Set all done yes"
        );
        assert_eq!(
            line("restore", json!({"client":"fd","fileset":"Set","before":"2026-10-01 12:00:00","fileregex":"\\.conf$"})),
            "restore select before=\"2026-10-01 12:00:00\" client=fd fileset=Set fileregex=\"\\\\.conf$\" done yes"
        );
        assert_eq!(
            line("restore", json!({"client":"fd","files":["/etc/passwd","/home/a b/\"x\""],"strip_prefix":"/home","add_prefix":"/tmp/r","restoreclient":"other-fd"})),
            "restore client=fd file=\"/etc/passwd\" file=\"/home/a b/\\\"x\\\"\" done strip_prefix=\"/home\" add_prefix=\"/tmp/r\" restoreclient=other-fd yes"
        );
        assert_eq!(
            line("restore", json!({"client":"fd","jobids":[5],"directories":["/etc/"]})),
            "restore jobid=5 client=fd directory=\"/etc/\" done yes"
        );
    }

    #[test]
    fn storage_volume_and_catalog_lines() {
        assert_eq!(line("label_volume", json!({"storage":"File","volume":"Vol-1","pool":"Full","drive":0})),
            "label storage=File volume=Vol-1 pool=Full drive=0 yes");
        assert_eq!(line("relabel_volume", json!({"storage":"File","oldvolume":"A","volume":"B","confirm":true})),
            "relabel storage=File oldvolume=A volume=B yes");
        assert_eq!(line("mount_storage", json!({"storage":"File","slot":3})), "mount storage=File slot=3");
        assert_eq!(line("unmount_storage", json!({"storage":"File","alldrives":true})), "unmount storage=File alldrives");
        assert_eq!(line("release_storage", json!({"jobid":9})), "release jobid=9");
        assert_eq!(
            line("update_volume", json!({"volume":"Vol-1","volstatus":"used","volretention":"30 days","recycle":false,"maxvolbytes":"10G","pool":"Other","enabled":"no"})),
            "update volume=Vol-1 volstatus=Used volretention=\"30 days\" maxvolbytes=\"10G\" recycle=no enabled=no pool=Other"
        );
        assert_eq!(line("update_volume", json!({"allfrompool":"Full"})), "update volume allfrompool=Full");
        assert_eq!(line("update_volume", json!({"fromallpools":true})), "update volume fromallpools");
        assert_eq!(line("update_pool", json!({"pool":"Full"})), "update pool=Full");
        assert_eq!(line("prune", json!({"kind":"jobs","client":"fd","confirm":true})), "prune jobs client=fd yes");
        assert_eq!(line("prune", json!({"kind":"volume","all_volumes":true,"pool":"Full","confirm":true})), "prune volume all pool=Full yes");
        assert_eq!(line("prune", json!({"kind":"volume","volume":"V1","confirm":true})), "prune volume=V1 yes");
        assert_eq!(line("prune", json!({"kind":"stats","confirm":true})), "prune stats yes");
        assert_eq!(line("purge", json!({"kind":"volume","volume":["A","B"],"confirm":true})), "purge volume=A volume=B");
        assert_eq!(line("purge", json!({"kind":"files","jobid":4,"confirm":true})), "purge files jobid=4");
        assert_eq!(line("purge", json!({"kind":"jobs","client":"fd","jobstatus":"E,f","confirm":true})), "purge jobs client=fd jobstatus=E,f");
        assert_eq!(line("purge", json!({"kind":"volume","volume":"A","action":"truncate","storage":"File","confirm":true})),
            "purge volume=A action=truncate storage=File");
        assert_eq!(line("delete", json!({"kind":"volume","volume":"A","confirm":true})), "delete volume=A yes");
        assert_eq!(line("delete", json!({"kind":"jobid","jobids":[1,2,3],"confirm":true})), "delete jobid=1,2,3");
        assert_eq!(line("delete", json!({"kind":"pool","pool":"Old","confirm":true})), "delete pool=Old");
        assert_eq!(line("truncate", json!({"pool":"Full","confirm":true})), "truncate volstatus=Purged pool=Full yes");
        assert_eq!(plan("purge", &json!({"kind":"quota","confirm":true})).unwrap().unwrap().yes_to, &["Purge (yes/no)?"]);
    }

    #[test]
    fn configuration_and_daemon_lines() {
        assert_eq!(line("enable", json!({"resource":"schedule","name":"Nightly"})), "enable schedule=Nightly");
        assert_eq!(line("disable", json!({"resource":"job","name":"Backup"})), "disable job=Backup");
        assert_eq!(
            line("configure_add", json!({"resource":"client","name":"new-fd","directives":{"address":"10.0.0.1","password":"p w","fdport":9102,"run":["a","b"]}})),
            "configure add client name=new-fd address=\"10.0.0.1\" password=\"p w\" fdport=\"9102\" run=\"a\" run=\"b\""
        );
        assert_eq!(line("configure_export", json!({"client":"fd"})), "configure export client=fd");
        assert_eq!(line("reload", json!({})), "reload");
        assert_eq!(line("setdebug", json!({"level":100,"target":"client","name":"fd","trace":true})), "setdebug level=100 trace=1 client=fd");
        assert_eq!(line("setdebug", json!({"level":0,"target":"dir"})), "setdebug level=0 dir");
        assert_eq!(line("setbandwidth", json!({"limit":1000,"client":"fd"})), "setbandwidth limit=1000 client=fd");
        assert_eq!(line("setbandwidth", json!({"limit":0,"jobid":4})), "setbandwidth limit=0 jobid=4");
    }

    #[test]
    fn destructive_tools_need_confirm() {
        for (tool, args) in [
            ("relabel_volume", json!({"storage":"File","oldvolume":"A","volume":"B"})),
            ("prune", json!({"kind":"stats"})),
            ("purge", json!({"kind":"volume","volume":"A","confirm":false})),
            ("delete", json!({"kind":"jobid","jobids":[1]})),
            ("delete", json!({"kind":"jobid","jobids":[1],"confirm":"yes"})),
            ("truncate", json!({"pool":"Full"})),
        ] {
            assert!(err(tool, args).contains("confirm=true"), "{tool}");
        }
    }

    #[test]
    fn injection_and_bad_arguments_are_rejected() {
        for (tool, args) in [
            ("run_job", json!({})),
            ("run_job", json!({"job":"Backup yes\ndelete volume=x"})),
            ("run_job", json!({"job":"Backup level=Full"})),
            ("run_job", json!({"job":"B\"x"})),
            ("run_job", json!({"job":"B;x"})),
            ("run_job", json!({"job":"B","level":"Fast"})),
            ("run_job", json!({"job":"B","when":"tomorrow"})),
            ("run_job", json!({"job":"B","priority":0})),
            ("run_job", json!({"job":"B","comment":"it's"})),
            ("run_job", json!({"job":"B","bogus":1})),
            ("rerun_job", json!({})),
            ("rerun_job", json!({"jobids":[1],"days":2})),
            ("cancel_job", json!({})),
            ("cancel_job", json!({"jobid":1,"all":true})),
            ("cancel_job", json!({"job":"B","storage":"File"})),
            ("wait_job", json!({})),
            ("restore", json!({"client":"fd","jobids":[1]})),
            ("restore", json!({"client":"fd","jobids":[1],"all":true,"files":["/a"]})),
            ("restore", json!({"client":"fd","all":true})),
            ("restore", json!({"client":"fd","jobids":[0],"all":true})),
            ("restore", json!({"client":"fd","jobids":"1;2","all":true})),
            ("restore", json!({"client":"fd","files":["relative"]})),
            ("restore", json!({"client":"fd","files":["/a\nb"]})),
            ("restore", json!({"client":"fd","fileset":"S","directories":["/etc/"]})),
            ("restore", json!({"client":"fd","jobids":[1],"directories":["/etc"]})),
            ("restore", json!({"client":"fd","jobids":[1],"all":true,"where":"/a","regexwhere":"!a!b!"})),
            ("label_volume", json!({"storage":"File","volume":"/abs","pool":"Full"})),
            ("update_volume", json!({"volume":"V"})),
            ("update_volume", json!({"volume":"V","volretention":"30 days; delete"})),
            ("update_volume", json!({"allfrompool":"P","recycle":true})),
            ("prune", json!({"kind":"files","confirm":true})),
            ("prune", json!({"kind":"stats","client":"fd","confirm":true})),
            ("purge", json!({"kind":"files","confirm":true})),
            ("purge", json!({"kind":"jobs","client":"fd","jobstatus":"E;x","confirm":true})),
            ("delete", json!({"kind":"volume","confirm":true})),
            ("delete", json!({"kind":"jobid","jobids":[1],"volume":"V","confirm":true})),
            ("enable", json!({"resource":"storage","name":"x"})),
            ("configure_add", json!({"resource":"director","name":"x"})),
            ("configure_add", json!({"resource":"client","name":"x","directives":{"Address":"a"}})),
            ("configure_add", json!({"resource":"client","name":"x","directives":{"address":"@/etc/passwd"}})),
            ("setdebug", json!({"level":1,"target":"client"})),
            ("setdebug", json!({"level":1,"target":"dir","hangup":3})),
            ("setbandwidth", json!({"limit":1})),
        ] {
            let msg = err(tool, args.clone());
            assert!(!msg.is_empty(), "{tool} {args}");
        }
    }

    /// Fake Director: questions come as `?<text>` lines of the canned output.
    struct Dialog {
        commands: Vec<String>,
        answers: Vec<String>,
        script: Vec<&'static str>,
    }
    impl CommandRunner for Dialog {
        fn run(&mut self, command: &str) -> anyhow::Result<String> {
            self.run_dialog(command, &mut |_| None)
        }
        fn run_dialog(&mut self, command: &str, answer: &mut dyn FnMut(&str) -> Option<String>) -> anyhow::Result<String> {
            self.commands.push(command.into());
            let mut out = String::new();
            for part in &self.script {
                if let Some(q) = part.strip_prefix('?') {
                    out.push_str(q);
                    let reply = answer(&out).unwrap_or_else(|| ".".into());
                    out.push_str(&format!("{reply}\n"));
                    self.answers.push(reply);
                } else {
                    out.push_str(part);
                }
            }
            Ok(out)
        }
    }

    #[test]
    fn purge_question_is_answered_other_questions_cancelled() {
        let mut d = Dialog { commands: vec![], answers: vec![], script: vec![
            "This command can be DANGEROUS!!!\n\nFound 1 Jobs for media \"A\" in catalog \"MyCatalog\".\n",
            "?Purge (yes/no)? ", "1 Job on Volume \"A\" purged from catalog.\n"] };
        let r = execute(&mut d, plan("purge", &json!({"kind":"volume","volume":"A","confirm":true})).unwrap());
        assert_eq!(r["isError"], false, "{r}");
        assert_eq!(d.answers, vec!["yes"]);
        assert_eq!(d.commands, vec!["purge volume=A"]);

        let mut d = Dialog { commands: vec![], answers: vec![], script: vec![
            "The defined Job resources are:\n 1: A\n 2: B\n", "?Select Job resource (1-2): "] };
        let r = execute(&mut d, plan("run_job", &json!({"job":"Nosuch"})).unwrap());
        assert_eq!(r["isError"], true);
        assert_eq!(d.answers, vec!["."]);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("Select Job resource (1-2):"));
    }

    #[test]
    fn started_jobids_are_structured() {
        let mut d = Dialog { commands: vec![], answers: vec![], script: vec!["Using Catalog \"MyCatalog\"\nJob queued. JobId=630\n"] };
        let r = execute(&mut d, plan("run_job", &json!({"job":"B"})).unwrap());
        assert_eq!(r["structuredContent"]["jobids"], json!([630]));
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("> run job=B yes\n"));
        let r = execute(&mut d, plan("cancel_job", &json!({})).unwrap());
        assert_eq!(r["isError"], true);
        assert_eq!(d.commands.len(), 1, "invalid arguments never reach the Director");
    }

    /// Route order: typed RPC (gRPC) → ExecuteCommand (gRPC, UNIMPLEMENTED)
    /// → console dialog (BNET).
    #[derive(Clone, Copy, PartialEq)]
    enum Mode { Typed, Unimplemented, Bnet }
    struct Route { mode: Mode, log: Vec<String> }
    impl CommandRunner for Route {
        fn run(&mut self, c: &str) -> anyhow::Result<String> {
            self.run_dialog(c, &mut |_| None)
        }
        fn run_dialog(&mut self, c: &str, answer: &mut dyn FnMut(&str) -> Option<String>) -> anyhow::Result<String> {
            self.log.push(format!("line {c}"));
            if self.mode == Mode::Bnet && c.starts_with("purge") {
                let reply = answer("Purge (yes/no)? ").unwrap_or_default();
                self.log.push(format!("answer {reply}"));
            }
            Ok("done\n".into())
        }
        fn is_grpc(&self) -> bool { self.mode != Mode::Bnet }
        fn typed_call(&mut self, rpc: &str, _r: Vec<u8>) -> Option<Result<Vec<u8>, crate::mcp::TypedError>> {
            match self.mode {
                Mode::Bnet => None,
                Mode::Unimplemented => { self.log.push(format!("rpc {rpc} unimplemented")); Some(Err(crate::mcp::TypedError::Unimplemented)) }
                Mode::Typed => {
                    self.log.push(format!("rpc {rpc}"));
                    let (_, out) = mcp_typed::rpc_types(rpc).unwrap();
                    Some(Ok(onesimus_proto_rs::typed::json_to_message(&out, &json!({"success":true,"text":"typed","command":"purge volume=A yes"})).bytes))
                }
            }
        }
    }

    #[test]
    fn fallback_order_typed_executecommand_bnet() {
        let args = json!({"kind":"volume","volume":["A"],"confirm":true});
        let expect = [
            (Mode::Typed, vec!["rpc PurgeVolume"]),
            (Mode::Unimplemented, vec!["rpc PurgeVolume unimplemented", "line purge volume=A yes"]),
            (Mode::Bnet, vec!["line purge volume=A", "answer yes"]),
        ];
        for (mode, log) in expect {
            let runner = Mutex::new(Route { mode, log: vec![] });
            let r = call("purge", &args, &runner).unwrap().unwrap();
            assert_eq!(r["isError"], false, "{r}");
            assert_eq!(runner.lock().unwrap().log, log);
            if mode == Mode::Typed {
                assert_eq!(r["structuredContent"]["text"], "typed");
            }
        }
        // invalid arguments reach no route at all
        let runner = Mutex::new(Route { mode: Mode::Typed, log: vec![] });
        let r = call("purge", &json!({"kind":"volume","volume":["A"]}), &runner).unwrap().unwrap();
        assert_eq!(r["isError"], true);
        assert!(runner.lock().unwrap().log.is_empty());
    }
}
