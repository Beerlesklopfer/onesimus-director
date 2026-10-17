//! Typed read-only MCP tools (S-30): one tool per console command family
//! (`list`/`llist`, `show`, `status`, `messages`, `version`, `time` and the
//! read-only dot listings). Every tool takes JSON-Schema parameters, checks
//! them (names restricted to Bareos name characters, integers in range,
//! enums for levels, statuses and resource types) and builds exactly one
//! console command line. No value can add a second command or argument.
//!
//! Output: the Director's JSON API (`.api json compact=yes` for the one
//! command, `.api 0` afterwards) where the command supports it; reports
//! that are only text (`status client=`, `status storage=`,
//! `status configuration`) and `format: "text"` use the text output.
//! Under the JSON API the Director never asks for input, so a missing or
//! unknown name comes back as a JSON error instead of a selection list.
//!
//! These tools exist in every build (production and development).

use serde_json::{Value, json};

use std::sync::Mutex;

use crate::mcp::{CommandRunner, run_plain, tool_result};

mod ovfs;
pub(crate) mod query;

/// Every read tool for tools/list, in this order: the generic `query`, the
/// `ovfs_*` tools, the typed tools.
pub fn all_tools() -> Vec<Value> {
    let mut all = vec![query::tool()];
    all.extend(ovfs::tools());
    all.extend(tools());
    all
}

/// Handle a tools/call of a read tool; `None` if `name` is none of them.
/// `write_enabled` only words the refusal of a writing `query` command.
pub fn call<R: CommandRunner>(
    name: &str,
    arguments: &Value,
    write_enabled: bool,
    runner: &Mutex<R>,
) -> Option<Result<Value, (i64, String)>> {
    if let Some(built) = command(name, arguments) {
        return Some(match built {
            Ok(command) => runner
                .lock()
                .map_err(|_| (-32603, "Director runner unavailable".to_string()))
                .map(|mut runner| run(&mut *runner, &command)),
            Err(msg) => Ok(tool_result(&msg, true)),
        });
    }
    if name.starts_with("ovfs_") {
        return match ovfs::command(name, arguments) {
            Ok(Some(command)) => Some(run_plain(runner, &command)),
            Ok(None) => None,
            Err(msg) => Some(Ok(tool_result(&msg, true))),
        };
    }
    if name != "query" {
        return None;
    }
    let command = query::command_arg(arguments);
    // Tool-level problems are reported as tool results so the model can react
    if let Err(msg) = query::validate_command(command) {
        return Some(Ok(tool_result(&msg, true)));
    }
    if !query::is_read_only(command) {
        return Some(Ok(tool_result(
            &format!(
                "'{command}' is not a read-only command. {}",
                if write_enabled {
                    "Use the 'execute' tool."
                } else {
                    "Write access is unavailable in this build or disabled."
                }
            ),
            true,
        )));
    }
    Some(run_plain(runner, command))
}

/// One checked command line and how to run it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadCommand {
    pub line: String,
    /// Run under the Director's JSON API.
    pub json: bool,
    /// Text reports whose unknown name would open a selection list: the
    /// name is checked first against this dot listing (`.clients`, `.storages`).
    pub require: Option<(&'static str, String)>,
}

const JOB_STATUS: &[&str] = &[
    "C", "R", "B", "T", "W", "E", "e", "f", "D", "A", "I", "L", "F", "S", "m", "M", "s", "j", "c",
    "d", "t", "p", "a", "i", "terminated", "warnings", "error", "fatal", "canceled", "running",
];
const JOB_LEVEL: &[&str] = &[
    "F", "I", "D", "f", "B", "S", "C", "V", "O", "d", "A", "Full", "Incremental", "Differential",
    "VirtualFull", "Base",
];
const JOB_TYPE: &[&str] = &[
    "B", "R", "V", "D", "C", "c", "M", "g", "O", "A", "S", "U", "I", "Backup", "Restore", "Verify",
    "Admin", "Copy", "Migrate", "Consolidate", "Archive",
];
const JOB_TYPE_NAMES: &[&str] =
    &["Backup", "Restore", "Verify", "Admin", "Copy", "Migrate", "Consolidate", "Archive"];
const POOL_TYPE: &[&str] = &["Backup", "Scratch", "Copy", "Archive", "Cloned", "Migration", "Save"];
/// `show` resource types: singular keyword, plural listing keyword.
const SHOW_TYPES: &[(&str, &str)] = &[
    ("director", "directors"),
    ("client", "clients"),
    ("job", "jobs"),
    ("jobdefs", "jobdefs"),
    ("storage", "storages"),
    ("catalog", "catalogs"),
    ("schedule", "schedules"),
    ("fileset", "filesets"),
    ("pool", "pools"),
    ("messages", "messages"),
    ("counter", "counters"),
    ("profile", "profiles"),
    ("console", "consoles"),
    ("user", "users"),
];

/// Bareos resource/volume names: letters, digits and `:.-_/ ` (IsNameValid), at most 127.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 127
        && name.chars().all(|c| c.is_ascii_alphanumeric() || ":.-_/ ".contains(c))
        && !name.starts_with(' ')
        && !name.ends_with(' ')
}

/// `key=value`, quoted when the name contains a space.
pub fn name_arg(key: &str, value: &str) -> Result<String, String> {
    if !valid_name(value) {
        return Err(format!(
            "{key}: '{}' is not a valid name (letters, digits and : . - _ / space, at most 127 characters)",
            value.escape_debug()
        ));
    }
    Ok(if value.contains(' ') { format!("{key}=\"{value}\"") } else { format!("{key}={value}") })
}

/// Typed access to the tool arguments.
struct Args<'a>(&'a Value);

impl Args<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key).filter(|v| !v.is_null())
    }

    fn name(&self, key: &str) -> Result<Option<String>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if s.is_empty() => Ok(None),
            Some(Value::String(s)) => name_arg(key, s).map(Some),
            Some(_) => Err(format!("{key} must be a string")),
        }
    }

    fn name_as(&self, key: &str, keyword: &str) -> Result<Option<String>, String> {
        Ok(self.name(key)?.map(|arg| format!("{keyword}{}", &arg[key.len()..])))
    }

    fn required_name(&self, key: &str) -> Result<String, String> {
        self.name(key)?.ok_or_else(|| format!("{key} is required"))
    }

    fn int(&self, key: &str, min: i64, max: i64) -> Result<Option<i64>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => match v.as_i64() {
                Some(n) if (min..=max).contains(&n) => Ok(Some(n)),
                Some(_) => Err(format!("{key} must be between {min} and {max}")),
                None => Err(format!("{key} must be an integer")),
            },
        }
    }

    fn flag(&self, key: &str) -> Result<bool, String> {
        match self.get(key) {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("{key} must be true or false")),
        }
    }

    fn choice(&self, key: &str, allowed: &[&str]) -> Result<Option<String>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(Some(s.clone())),
            Some(_) => Err(format!("{key} must be one of: {}", allowed.join(", "))),
        }
    }

    /// One value or a list of values from `allowed`, joined with commas.
    fn choices(&self, key: &str, allowed: &[&str]) -> Result<Option<String>, String> {
        let values: Vec<&Value> = match self.get(key) {
            None => return Ok(None),
            Some(Value::Array(items)) if items.is_empty() => return Ok(None),
            Some(Value::Array(items)) => items.iter().collect(),
            Some(v) => vec![v],
        };
        let mut out = Vec::new();
        for v in values {
            match v.as_str() {
                Some(s) if allowed.contains(&s) => out.push(s),
                _ => return Err(format!("{key} values must be from: {}", allowed.join(", "))),
            }
        }
        Ok(Some(out.join(",")))
    }

    fn json(&self) -> Result<bool, String> {
        Ok(self.choice("format", &["json", "text"])?.as_deref() != Some("text"))
    }
}

fn props(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn enum_list(values: &[&str], description: &str) -> Value {
    json!({ "description": description, "anyOf": [
        { "type": "string", "enum": values },
        { "type": "array", "items": { "type": "string", "enum": values } }
    ]})
}

fn tool(name: &str, title: &str, description: &str, mut properties: serde_json::Map<String, Value>,
        required: &[&str], json_output: bool) -> Value {
    if json_output {
        properties.insert("format".into(), json!({ "type": "string", "enum": ["json", "text"],
            "default": "json",
            "description": "json (default): the Director's JSON API result; text: the bconsole text output" }));
    }
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": { "type": "object", "properties": properties, "required": required,
                         "additionalProperties": false },
        "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true,
                         "openWorldHint": false },
    })
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn integer(min: i64, max: i64, description: &str) -> Value {
    json!({ "type": "integer", "minimum": min, "maximum": max, "description": description })
}

fn boolean(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

const LONG: &str = "true: llist (all fields of every record) instead of list";

/// The typed read-only tools for tools/list.
pub fn tools() -> Vec<Value> {
    let page = props(json!({
        "limit": integer(1, 1_000_000, "at most this many rows"),
        "offset": integer(0, i64::MAX, "skip this many rows (with limit)"),
    }));
    let with = |base: Value, extra: &serde_json::Map<String, Value>| {
        let mut p = props(base);
        p.extend(extra.clone());
        p
    };
    let state = json!({ "type": "string", "enum": ["enabled", "disabled"],
        "description": "only enabled or only disabled resources" });
    let mut tools = vec![
        tool("list_jobs", "List jobs from the catalog",
            "Jobs from the catalog (list jobs / llist jobs), newest last. Filters combine. \
             Example: {\"client\":\"bareos-fd\",\"jobstatus\":[\"T\",\"W\"],\"limit\":10}.",
            with(json!({
                "job": string("only jobs with this job name"),
                "client": string("only jobs of this client"),
                "jobstatus": enum_list(JOB_STATUS, "job status letter(s) or name(s): T terminated, W warnings, E error, f fatal, A canceled, R running, …"),
                "jobtype": enum_list(JOB_TYPE, "job type letter(s) or exact name(s): B/Backup, R/Restore, V/Verify, D/Admin, c/Copy, g/Migrate, O/Consolidate, A/Archive"),
                "level": enum_list(JOB_LEVEL, "job level(s): F/Full, I/Incremental, D/Differential, f/VirtualFull, B/Base"),
                "volume": string("only jobs that wrote to this volume"),
                "pool": string("only jobs of this pool"),
                "days": integer(0, 100_000, "only jobs started in the last n days"),
                "hours": integer(0, 10_000_000, "only jobs started in the last n hours"),
                "last": boolean("only the last run of each job name"),
                "count": boolean("only the number of matching jobs"),
                "order": { "type": "string", "enum": ["asc", "desc"], "description": "sort by start time" },
                "long": boolean(LONG),
            }), &page), &[], true),
        tool("list_job", "One job record",
            "One job from the catalog by JobId or unique job name (list jobid= / llist jobid=).",
            props(json!({
                "jobid": integer(1, i64::MAX, "JobId"),
                "ujobid": string("unique job name, e.g. backup-bareos-fd.2026-10-13_21.00.00_09"),
                "long": boolean(LONG),
            })), &[], true),
        tool("list_joblog", "Log of one job",
            "The stored job log with timestamps (list joblog jobid=).",
            with(json!({
                "jobid": integer(1, i64::MAX, "JobId"),
                "ujobid": string("unique job name"),
                "count": boolean("only the number of log lines"),
            }), &page), &[], true),
        tool("list_jobmedia", "Volumes written by jobs",
            "JobMedia records: which volumes a job (or every job) wrote (list jobmedia).",
            props(json!({
                "jobid": integer(1, i64::MAX, "JobId; all jobs if omitted"),
                "ujobid": string("unique job name"),
                "long": boolean(LONG),
            })), &[], true),
        tool("list_files", "Files of one job",
            "Paths backed up by one job (list files jobid=). Use limit for big jobs.",
            with(json!({
                "jobid": integer(1, i64::MAX, "JobId"),
                "ujobid": string("unique job name"),
            }), &page), &[], true),
        tool("list_volumes", "List volumes (media)",
            "Volumes grouped by pool (list volumes). With pool: only that pool; with jobid: the \
             volumes of that job; with volume: one volume record; all: one table.",
            with(json!({
                "pool": string("only volumes of this pool"),
                "jobid": integer(1, i64::MAX, "only the volumes this job used"),
                "volume": string("one volume by name"),
                "all": boolean("all volumes in one table"),
                "count": boolean("only the number of volumes"),
                "long": boolean(LONG),
            }), &page), &[], true),
        tool("list_pools", "List pools",
            "Pool records from the catalog (list pools), or one pool with its prune details.",
            props(json!({ "pool": string("one pool by name"), "long": boolean(LONG) })), &[], true),
        tool("list_clients", "List clients",
            "Client records from the catalog (list clients), or one client.",
            props(json!({ "client": string("one client by name"), "long": boolean(LONG) })), &[], true),
        tool("list_storages", "List storages",
            "Storage records from the catalog (list storages).",
            props(json!({ "long": boolean(LONG) })), &[], true),
        tool("list_filesets", "List filesets",
            "FileSet records (list filesets); with jobid or filesetid one FileSet with its stored text.",
            props(json!({
                "jobid": integer(1, i64::MAX, "FileSet of this job"),
                "filesetid": integer(1, i64::MAX, "FileSet by id"),
                "long": boolean(LONG),
            })), &[], true),
        tool("list_jobtotals", "Job totals",
            "Number of jobs, files and bytes per job name and in total (list jobtotals).",
            props(json!({})), &[], true),
        tool("list_copies", "Copy jobs",
            "Copy jobs, optionally only those of the given original JobIds (list copies).",
            props(json!({ "jobids": { "type": "array", "items": { "type": "integer", "minimum": 1 },
                "description": "original JobIds" }, "long": boolean(LONG) })), &[], true),
        tool("list_backups", "Backups of a client",
            "Backups of one client (list backups client=), optionally of one FileSet.",
            with(json!({
                "client": string("client name"),
                "fileset": string("FileSet name, or 'any'"),
                "jobstatus": enum_list(JOB_STATUS, "job status letter(s) or name(s)"),
                "level": enum_list(JOB_LEVEL, "job level(s)"),
                "order": { "type": "string", "enum": ["asc", "desc"], "description": "sort by start time" },
                "days": integer(0, 100_000, "only backups of the last n days"),
                "hours": integer(0, 10_000_000, "only backups of the last n hours"),
                "long": boolean(LONG),
            }), &page), &["client"], true),
        tool("list_nextvolume", "Next volume of a job",
            "Which volume the job would write next (list nextvolume job=).",
            props(json!({
                "job": string("job name"),
                "days": integer(0, 50, "days ahead (at most 50)"),
            })), &["job"], false),
        tool("show", "Show configuration resources",
            "Resources as configured in the Director (show). resource with name: one resource; \
             without name: all of that type; 'all': everything; 'disabled': disabled jobs, \
             clients and schedules. Passwords are not shown.",
            props(json!({
                "resource": { "type": "string",
                    "enum": SHOW_TYPES.iter().map(|(s, _)| *s).chain(["all", "disabled"]).collect::<Vec<_>>(),
                    "description": "resource type" },
                "name": string("resource name; omit for all resources of the type"),
                "disabled_type": { "type": "string", "enum": ["jobs", "clients", "schedules"],
                    "description": "with resource=disabled: only this type" },
                "verbose": boolean("also directives with default values"),
            })), &["resource"], true),
        tool("status_director", "Director status",
            "Director status: header, scheduled, running and terminated jobs, client connections (status director).",
            props(json!({ "days": integer(0, 500, "days of scheduled jobs, default 1") })), &[], true),
        tool("status_client", "File daemon status",
            "Status report of one File Daemon (status client=), as text. Contacts the client.",
            props(json!({ "client": string("client name") })), &["client"], false),
        tool("status_storage", "Storage daemon status",
            "Status report of one Storage Daemon (status storage=), as text; slots: the autochanger slot table.",
            props(json!({ "storage": string("storage name"), "slots": boolean("autochanger slot table") })),
            &["storage"], false),
        tool("status_scheduler", "Scheduler preview",
            "Which schedules trigger which jobs and a preview of the next runs (status scheduler).",
            props(json!({
                "days": integer(-365, 365, "preview for n days (negative: past days), default 7"),
                "days_from": integer(-366, 366, "start of the range in days relative to now (with days_to)"),
                "days_to": integer(-366, 366, "end of the range in days relative to now (with days_from)"),
                "job": string("only this job"),
                "client": string("only jobs of this client"),
                "schedule": string("only this schedule"),
            })), &[], true),
        tool("status_subscriptions", "Subscription report",
            "Backup units for subscriptions, from the catalog (status subscriptions).",
            props(json!({
                "clients": boolean("summary per client"),
                "plugins": boolean("summary per plugin"),
                "all": boolean("all partial reports"),
                "anonymize": boolean("replace client names by 'Client 1', 'Client 2', …"),
                "client": string("detail per FileSet of this client"),
            })), &[], true),
        tool("status_configuration", "Configuration check",
            "Checks the configuration for deprecated settings (status configuration), as text.",
            props(json!({})), &[], false),
        tool("messages", "Pending Director messages",
            "Returns the console's queued Director messages (job reports, warnings). \
             Reading empties the queue.",
            props(json!({})), &[], true),
        tool("version", "Director version", "Version of the Director (version).", props(json!({})), &[], true),
        tool("time", "Director time", "Current time on the Director (time).", props(json!({})), &[], true),
        tool("dot_jobs", "Job names",
            "Names of the configured jobs (.jobs).",
            props(json!({ "type": { "type": "string", "enum": JOB_TYPE_NAMES, "description": "only jobs of this type" },
                "state": state })), &[], true),
        tool("dot_clients", "Client names", "Names of the configured clients (.clients).",
            props(json!({ "state": state })), &[], true),
        tool("dot_pools", "Pool names", "Names of the configured pools (.pools).",
            props(json!({ "type": { "type": "string", "enum": POOL_TYPE, "description": "only pools of this type" } })),
            &[], true),
        tool("dot_storages", "Storage names", "Names of the configured storages (.storages).",
            props(json!({ "state": state })), &[], true),
        tool("dot_filesets", "FileSet names", "Names of the configured FileSets (.filesets).",
            props(json!({})), &[], true),
        tool("dot_schedule", "Schedule names", "Names of the configured schedules (.schedule).",
            props(json!({ "state": state })), &[], true),
        tool("dot_levels", "Job levels", "Job levels, optionally of one job type (.levels).",
            props(json!({ "jobtype": { "type": "string", "enum": JOB_TYPE_NAMES,
                "description": "only the levels of this job type" } })), &[], true),
        tool("dot_types", "Job types", "Job types (.types).", props(json!({})), &[], true),
        tool("dot_msgs", "Messages resources", "Names of the Messages resources (.msgs).",
            props(json!({})), &[], true),
        tool("dot_defaults", "Resource defaults",
            "Defaults of one job, client, storage or pool as key/value pairs (.defaults), \
             e.g. what a run of the job would use.",
            props(json!({
                "resource": { "type": "string", "enum": ["job", "client", "storage", "pool"],
                    "description": "resource type, default job" },
                "name": string("resource name"),
            })), &["name"], true),
        tool("dot_jobstatus", "Job status codes",
            "Job status codes with their meaning (.jobstatus), or one code.",
            props(json!({ "code": { "type": "string", "minLength": 1, "maxLength": 1,
                "description": "one status letter, e.g. T" } })), &[], true),
    ];
    // json!() cannot hold the computed enum; fix up the show schema's array.
    for tool in &mut tools {
        if tool["name"] == "show" {
            let list: Vec<&str> = SHOW_TYPES.iter().map(|(s, _)| *s).chain(["all", "disabled"]).collect();
            tool["inputSchema"]["properties"]["resource"]["enum"] = json!(list);
        }
    }
    tools
}

/// The command of a read tool; `None` if `tool` is not one of them.
pub fn command(tool: &str, arguments: &Value) -> Option<Result<ReadCommand, String>> {
    if !tools_names().contains(&tool) {
        return None;
    }
    if !arguments.is_object() && !arguments.is_null() {
        return Some(Err("arguments must be an object".into()));
    }
    let empty = json!({});
    let a = Args(if arguments.is_null() { &empty } else { arguments });
    Some(build(tool, &a))
}

fn tools_names() -> &'static [&'static str] {
    &[
        "list_jobs", "list_job", "list_joblog", "list_jobmedia", "list_files", "list_volumes",
        "list_pools", "list_clients", "list_storages", "list_filesets", "list_jobtotals",
        "list_copies", "list_backups", "list_nextvolume", "show", "status_director",
        "status_client", "status_storage", "status_scheduler", "status_subscriptions",
        "status_configuration", "messages", "version", "time", "dot_jobs", "dot_clients",
        "dot_pools", "dot_storages", "dot_filesets", "dot_schedule", "dot_levels", "dot_types",
        "dot_msgs", "dot_defaults", "dot_jobstatus",
    ]
}

fn build(tool: &str, a: &Args) -> Result<ReadCommand, String> {
    // Reject unknown keys: a typo must not silently widen a filter.
    let schema = tools().into_iter().find(|t| t["name"] == tool).expect("listed tool");
    if let Some(map) = a.0.as_object() {
        for key in map.keys() {
            if schema["inputSchema"]["properties"].get(key).is_none() {
                return Err(format!("unknown argument '{key}' for {tool}"));
            }
        }
    }
    let mut json = a.json()?;
    let mut require = None;
    let mut parts: Vec<String> = Vec::new();
    let verb = |a: &Args| -> Result<&'static str, String> { Ok(if a.flag("long")? { "llist" } else { "list" }) };
    let page = |a: &Args, parts: &mut Vec<String>| -> Result<(), String> {
        let limit = a.int("limit", 1, 1_000_000)?;
        let offset = a.int("offset", 0, i64::MAX)?;
        if offset.is_some() && limit.is_none() {
            return Err("offset needs limit".into());
        }
        parts.extend(limit.map(|n| format!("limit={n}")));
        parts.extend(offset.map(|n| format!("offset={n}")));
        Ok(())
    };
    // jobid= or ujobid= (one of them), `need`: one is required
    let job_ref = |a: &Args, parts: &mut Vec<String>, need: bool| -> Result<(), String> {
        let jobid = a.int("jobid", 1, i64::MAX)?;
        let ujobid = a.name("ujobid")?;
        match (jobid, ujobid) {
            (Some(_), Some(_)) => Err("give jobid or ujobid, not both".into()),
            (Some(id), None) => { parts.push(format!("jobid={id}")); Ok(()) }
            (None, Some(u)) => { parts.push(u); Ok(()) }
            (None, None) if need => Err("jobid or ujobid is required".into()),
            (None, None) => Ok(()),
        }
    };
    let line = match tool {
        "list_jobs" => {
            parts.push(verb(a)?.into());
            parts.push("jobs".into());
            parts.extend(a.name("job")?);
            parts.extend(a.name("client")?);
            parts.extend(a.choices("jobstatus", JOB_STATUS)?.map(|v| format!("jobstatus={v}")));
            parts.extend(a.choices("jobtype", JOB_TYPE)?.map(|v| format!("jobtype={v}")));
            parts.extend(a.choices("level", JOB_LEVEL)?.map(|v| format!("joblevel={v}")));
            parts.extend(a.name("volume")?);
            parts.extend(a.name("pool")?);
            parts.extend(a.int("days", 0, 100_000)?.map(|n| format!("days={n}")));
            parts.extend(a.int("hours", 0, 10_000_000)?.map(|n| format!("hours={n}")));
            if a.flag("last")? { parts.push("last".into()); }
            if a.flag("count")? { parts.push("count".into()); }
            parts.extend(a.choice("order", &["asc", "desc"])?.map(|v| format!("order={v}")));
            page(a, &mut parts)?;
            parts.join(" ")
        }
        "list_job" => {
            parts.push(verb(a)?.into());
            job_ref(a, &mut parts, true)?;
            parts.join(" ")
        }
        "list_joblog" => {
            parts.push("list joblog".into());
            job_ref(a, &mut parts, true)?;
            if a.flag("count")? { parts.push("count".into()); }
            page(a, &mut parts)?;
            parts.join(" ")
        }
        "list_jobmedia" => {
            parts.push(format!("{} jobmedia", verb(a)?));
            job_ref(a, &mut parts, false)?;
            parts.join(" ")
        }
        "list_files" => {
            parts.push("list files".into());
            job_ref(a, &mut parts, true)?;
            page(a, &mut parts)?;
            parts.join(" ")
        }
        "list_volumes" => {
            let verb = verb(a)?;
            let pool = a.name("pool")?;
            let jobid = a.int("jobid", 1, i64::MAX)?;
            let volume = a.name("volume")?;
            let all = a.flag("all")?;
            if [pool.is_some(), jobid.is_some(), volume.is_some(), all].iter().filter(|b| **b).count() > 1 {
                return Err("give at most one of pool, jobid, volume, all".into());
            }
            if let Some(volume) = volume {
                parts.push(format!("{verb} {volume}"));
            } else {
                parts.push(format!("{verb} volumes"));
                parts.extend(pool);
                parts.extend(jobid.map(|id| format!("jobid={id}")));
                if all { parts.push("all".into()); }
                if a.flag("count")? { parts.push("count".into()); }
            }
            page(a, &mut parts)?;
            parts.join(" ")
        }
        "list_pools" => match a.name("pool")? {
            Some(pool) => format!("{} {pool}", verb(a)?),
            None => format!("{} pools", verb(a)?),
        },
        "list_clients" => match a.name("client")? {
            Some(client) => format!("{} {client}", verb(a)?),
            None => format!("{} clients", verb(a)?),
        },
        "list_storages" => format!("{} storages", verb(a)?),
        "list_filesets" => {
            let jobid = a.int("jobid", 1, i64::MAX)?;
            let filesetid = a.int("filesetid", 1, i64::MAX)?;
            match (jobid, filesetid) {
                (Some(_), Some(_)) => return Err("give jobid or filesetid, not both".into()),
                (Some(id), None) => format!("{} fileset jobid={id}", verb(a)?),
                (None, Some(id)) => format!("{} fileset filesetid={id}", verb(a)?),
                (None, None) => format!("{} filesets", verb(a)?),
            }
        }
        "list_jobtotals" => "list jobtotals".into(),
        "list_copies" => {
            parts.push(format!("{} copies", verb(a)?));
            match a.get("jobids") {
                None => {}
                Some(Value::Array(ids)) if ids.is_empty() => {}
                Some(Value::Array(ids)) => {
                    let ids: Option<Vec<String>> = ids.iter()
                        .map(|v| v.as_i64().filter(|n| *n > 0).map(|n| n.to_string())).collect();
                    let ids = ids.ok_or("jobids must be positive integers")?;
                    parts.push(format!("jobid={}", ids.join(",")));
                }
                Some(_) => return Err("jobids must be a list of JobIds".into()),
            }
            parts.join(" ")
        }
        "list_backups" => {
            parts.push(format!("{} backups", verb(a)?));
            parts.push(a.required_name("client")?);
            parts.extend(a.name("fileset")?);
            parts.extend(a.choices("jobstatus", JOB_STATUS)?.map(|v| format!("jobstatus={v}")));
            parts.extend(a.choices("level", JOB_LEVEL)?.map(|v| format!("level={v}")));
            parts.extend(a.choice("order", &["asc", "desc"])?.map(|v| format!("order={v}")));
            parts.extend(a.int("days", 0, 100_000)?.map(|n| format!("days={n}")));
            parts.extend(a.int("hours", 0, 10_000_000)?.map(|n| format!("hours={n}")));
            page(a, &mut parts)?;
            parts.join(" ")
        }
        "list_nextvolume" => {
            // .api 2 answers nextvolume with an empty result: text only.
            json = false;
            parts.push("list nextvolume".into());
            parts.push(a.required_name("job")?);
            parts.extend(a.int("days", 0, 50)?.map(|n| format!("days={n}")));
            parts.join(" ")
        }
        "show" => {
            let resource = a.choice("resource",
                &SHOW_TYPES.iter().map(|(s, _)| *s).chain(["all", "disabled"]).collect::<Vec<_>>())?
                .ok_or("resource is required")?;
            parts.push("show".into());
            match resource.as_str() {
                "all" => parts.push("all".into()),
                "disabled" => {
                    parts.push("disabled".into());
                    parts.extend(a.choice("disabled_type", &["jobs", "clients", "schedules"])?);
                }
                singular => {
                    let plural = SHOW_TYPES.iter().find(|(s, _)| *s == singular).unwrap().1;
                    let keyword = if singular == "messages" { "message" } else { singular };
                    match a.name_as("name", keyword)? {
                        Some(arg) => parts.push(arg),
                        None => parts.push(plural.into()),
                    }
                }
            }
            if resource != "disabled" && a.get("disabled_type").is_some() {
                return Err("disabled_type only with resource=disabled".into());
            }
            if resource == "all" && a.get("name").is_some() {
                return Err("name not with resource=all".into());
            }
            if a.flag("verbose")? { parts.push("verbose".into()); }
            parts.join(" ")
        }
        "status_director" => {
            parts.push("status director".into());
            parts.extend(a.int("days", 0, 500)?.map(|n| format!("days={n}")));
            parts.join(" ")
        }
        "status_client" => {
            json = false;
            let client = a.required_name("client")?;
            require = Some((".clients", a.get("client").and_then(Value::as_str).unwrap().to_string()));
            format!("status {client}")
        }
        "status_storage" => {
            json = false;
            let storage = a.required_name("storage")?;
            require = Some((".storages", a.get("storage").and_then(Value::as_str).unwrap().to_string()));
            parts.push(format!("status {storage}"));
            if a.flag("slots")? { parts.push("slots".into()); }
            parts.join(" ")
        }
        "status_scheduler" => {
            parts.push("status scheduler".into());
            let days = a.int("days", -365, 365)?;
            let from = a.int("days_from", -366, 366)?;
            let to = a.int("days_to", -366, 366)?;
            match (days, from, to) {
                (Some(_), Some(_), _) | (Some(_), _, Some(_)) => return Err("give days or days_from/days_to".into()),
                (Some(n), None, None) => parts.push(format!("days={n}")),
                (None, Some(f), Some(t)) => parts.push(format!("days={f},{t}")),
                (None, None, None) => {}
                _ => return Err("days_from and days_to go together".into()),
            }
            parts.extend(a.name("schedule")?);
            parts.extend(a.name("client")?);
            parts.extend(a.name("job")?);
            parts.join(" ")
        }
        "status_subscriptions" => {
            parts.push("status subscriptions".into());
            for word in ["clients", "plugins", "all", "anonymize"] {
                if a.flag(word)? { parts.push(word.into()); }
            }
            parts.extend(a.name("client")?);
            parts.join(" ")
        }
        "status_configuration" => {
            json = false;
            "status configuration".into()
        }
        "messages" => "messages".into(),
        "version" => "version".into(),
        "time" => "time".into(),
        "dot_jobs" => {
            parts.push(".jobs".into());
            parts.extend(a.choice("type", JOB_TYPE_NAMES)?.map(|t| format!("type={t}")));
            parts.extend(a.choice("state", &["enabled", "disabled"])?);
            parts.join(" ")
        }
        "dot_clients" | "dot_storages" | "dot_schedule" => {
            parts.push(match tool { "dot_clients" => ".clients", "dot_storages" => ".storages", _ => ".schedule" }.into());
            parts.extend(a.choice("state", &["enabled", "disabled"])?);
            parts.join(" ")
        }
        "dot_pools" => {
            parts.push(".pools".into());
            parts.extend(a.choice("type", POOL_TYPE)?.map(|t| format!("type={t}")));
            parts.join(" ")
        }
        "dot_filesets" => ".filesets".into(),
        "dot_levels" => {
            parts.push(".levels".into());
            parts.extend(a.choice("jobtype", JOB_TYPE_NAMES)?);
            parts.join(" ")
        }
        "dot_types" => ".types".into(),
        "dot_msgs" => ".msgs".into(),
        "dot_defaults" => {
            let kind = a.choice("resource", &["job", "client", "storage", "pool"])?.unwrap_or_else(|| "job".into());
            let name = a.name_as("name", &kind)?.ok_or("name is required")?;
            format!(".defaults {name}")
        }
        "dot_jobstatus" => match a.get("code") {
            None => ".jobstatus".into(),
            Some(Value::String(c)) if c.chars().count() == 1 && c.chars().all(|c| c.is_ascii_alphabetic()) => {
                format!(".jobstatus={c}")
            }
            Some(_) => return Err("code must be one status letter".into()),
        },
        _ => unreachable!("listed tool {tool}"),
    };
    Ok(ReadCommand { line, json, require })
}

/// Runs a checked command and builds the MCP tool result.
pub fn run<R: CommandRunner + ?Sized>(runner: &mut R, command: &ReadCommand) -> Value {
    if let Some((listing, name)) = &command.require {
        match runner.run_json(listing) {
            Ok(out) => {
                let known: Vec<String> = serde_json::from_str::<Value>(out.trim())
                    .ok()
                    .and_then(|v| v["result"].as_object().and_then(|r| r.values().next().cloned()))
                    .and_then(|list| list.as_array().cloned())
                    .map(|list| list.iter().filter_map(|e| e["name"].as_str().map(String::from)).collect())
                    .unwrap_or_default();
                if !known.is_empty() && !known.contains(name) {
                    return result(&format!("'{name}' is not a configured resource; known: {}", known.join(", ")), true);
                }
            }
            Err(e) => return result(&format!("Director error: {e}"), true),
        }
    }
    let output = if command.json { runner.run_json(&command.line) } else { runner.run(&command.line) };
    match output {
        Ok(out) if command.json => {
            // One compact line on every transport (gRPC returns pretty JSON)
            match serde_json::from_str::<Value>(out.trim()) {
                Ok(v) => result(&v.to_string(), v.get("error").is_some()),
                Err(_) if out.trim().is_empty() => result("(no output)", false),
                Err(_) => result(out.trim(), false),
            }
        }
        Ok(out) if out.trim().is_empty() => result("(no output)", false),
        Ok(out) => result(&out, false),
        Err(e) => result(&format!("Director error: {e}"), true),
    }
}

fn result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(tool: &str, args: Value) -> String {
        command(tool, &args).expect("read tool").unwrap_or_else(|e| panic!("{tool} {args}: {e}")).line
    }

    fn rejected(tool: &str, args: Value) -> String {
        command(tool, &args).expect("read tool").expect_err(&format!("{tool} {args} accepted"))
    }

    #[test]
    fn every_listed_tool_builds_a_command_and_is_read_only() {
        let tools = tools();
        assert_eq!(tools.len(), tools_names().len());
        for t in &tools {
            let name = t["name"].as_str().unwrap();
            assert!(tools_names().contains(&name), "{name}");
            assert_eq!(t["annotations"]["readOnlyHint"], true);
            assert_eq!(t["annotations"]["destructiveHint"], false);
            assert_eq!(t["inputSchema"]["type"], "object");
            assert!(!t["description"].as_str().unwrap().is_empty());
            let mut args = json!({});
            for key in t["inputSchema"]["required"].as_array().unwrap() {
                let key = key.as_str().unwrap();
                args[key] = json!(if key == "resource" { "job" } else { "bareos-fd" });
            }
            if ["list_job", "list_joblog", "list_files"].contains(&name) {
                args["jobid"] = json!(1);
            }
            assert!(command(name, &args).unwrap().is_ok(), "{name}");
        }
        assert!(command("query", &json!({})).is_none());
        assert!(command("run_job", &json!({})).is_none());
    }

    #[test]
    fn list_command_lines() {
        assert_eq!(line("list_jobs", json!({})), "list jobs");
        assert_eq!(
            line("list_jobs", json!({"job":"backup-bareos-fd","client":"bareos-fd","jobstatus":["T","W"],
                "jobtype":"B","level":["F","Incremental"],"volume":"Full-0001","pool":"Full","days":3,
                "hours":5,"last":true,"count":true,"order":"desc","limit":10,"offset":2,"long":true})),
            "llist jobs job=backup-bareos-fd client=bareos-fd jobstatus=T,W jobtype=B joblevel=F,Incremental \
             volume=Full-0001 pool=Full days=3 hours=5 last count order=desc limit=10 offset=2");
        assert_eq!(line("list_job", json!({"jobid":14})), "list jobid=14");
        assert_eq!(line("list_job", json!({"ujobid":"a.2026-10-13_21.00.00_09","long":true})),
            "llist ujobid=a.2026-10-13_21.00.00_09");
        assert_eq!(line("list_joblog", json!({"jobid":1,"count":true})), "list joblog jobid=1 count");
        assert_eq!(line("list_jobmedia", json!({})), "list jobmedia");
        assert_eq!(line("list_jobmedia", json!({"jobid":1})), "list jobmedia jobid=1");
        assert_eq!(line("list_files", json!({"jobid":5,"limit":2,"offset":1})), "list files jobid=5 limit=2 offset=1");
        assert_eq!(line("list_volumes", json!({})), "list volumes");
        assert_eq!(line("list_volumes", json!({"pool":"Full","count":true})), "list volumes pool=Full count");
        assert_eq!(line("list_volumes", json!({"volume":"Full-0001","long":true})), "llist volume=Full-0001");
        assert_eq!(line("list_volumes", json!({"jobid":1})), "list volumes jobid=1");
        assert_eq!(line("list_volumes", json!({"all":true})), "list volumes all");
        assert_eq!(line("list_pools", json!({"pool":"My Pool"})), "list pool=\"My Pool\"");
        assert_eq!(line("list_clients", json!({})), "list clients");
        assert_eq!(line("list_storages", json!({"long":true})), "llist storages");
        assert_eq!(line("list_filesets", json!({"jobid":1})), "list fileset jobid=1");
        assert_eq!(line("list_filesets", json!({})), "list filesets");
        assert_eq!(line("list_jobtotals", json!({})), "list jobtotals");
        assert_eq!(line("list_copies", json!({"jobids":[1,2]})), "list copies jobid=1,2");
        assert_eq!(line("list_backups", json!({"client":"bareos-fd","fileset":"any","level":"F","limit":2})),
            "list backups client=bareos-fd fileset=any level=F limit=2");
        assert_eq!(line("list_nextvolume", json!({"job":"backup-bareos-fd","days":3})),
            "list nextvolume job=backup-bareos-fd days=3");
    }

    #[test]
    fn show_status_and_dot_command_lines() {
        assert_eq!(line("show", json!({"resource":"client"})), "show clients");
        assert_eq!(line("show", json!({"resource":"fileset","name":"LinuxAll","verbose":true})),
            "show fileset=LinuxAll verbose");
        assert_eq!(line("show", json!({"resource":"messages","name":"Daemon"})), "show message=Daemon");
        assert_eq!(line("show", json!({"resource":"disabled","disabled_type":"jobs"})), "show disabled jobs");
        assert_eq!(line("show", json!({"resource":"all"})), "show all");
        assert_eq!(line("status_director", json!({"days":3})), "status director days=3");
        assert_eq!(line("status_client", json!({"client":"bareos-fd"})), "status client=bareos-fd");
        assert_eq!(line("status_storage", json!({"storage":"File","slots":true})), "status storage=File slots");
        assert_eq!(line("status_scheduler", json!({"days_from":-2,"days_to":3,"job":"j"})),
            "status scheduler days=-2,3 job=j");
        assert_eq!(line("status_subscriptions", json!({"all":true,"anonymize":true})),
            "status subscriptions all anonymize");
        assert_eq!(line("status_configuration", json!({})), "status configuration");
        assert_eq!(line("dot_jobs", json!({"type":"Backup","state":"enabled"})), ".jobs type=Backup enabled");
        assert_eq!(line("dot_pools", json!({"type":"Scratch"})), ".pools type=Scratch");
        assert_eq!(line("dot_levels", json!({"jobtype":"Backup"})), ".levels Backup");
        assert_eq!(line("dot_defaults", json!({"name":"backup-bareos-fd"})), ".defaults job=backup-bareos-fd");
        assert_eq!(line("dot_defaults", json!({"resource":"pool","name":"Full"})), ".defaults pool=Full");
        assert_eq!(line("dot_jobstatus", json!({"code":"T"})), ".jobstatus=T");
        for (tool, json_mode) in [("list_jobs", true), ("status_client", false), ("status_configuration", false),
                                  ("list_nextvolume", false), ("version", true)] {
            let args = if tool == "status_client" { json!({"client":"x"}) }
                else if tool == "list_nextvolume" { json!({"job":"x"}) } else { json!({}) };
            assert_eq!(command(tool, &args).unwrap().unwrap().json, json_mode, "{tool}");
        }
        assert!(!command("version", &json!({"format":"text"})).unwrap().unwrap().json);
    }

    #[test]
    fn injection_and_bad_values_are_rejected() {
        for (tool, args) in [
            ("list_jobs", json!({"client":"fd; delete volume=x yes"})),
            ("list_jobs", json!({"client":"fd\nquit"})),
            ("list_jobs", json!({"client":"fd\" yes"})),
            ("list_jobs", json!({"client":"fd=x"})),
            ("list_jobs", json!({"jobstatus":"T,delete"})),
            ("list_jobs", json!({"level":"Q"})),
            ("list_jobs", json!({"limit":"5"})),
            ("list_jobs", json!({"limit":0})),
            ("list_jobs", json!({"offset":2})),
            ("list_jobs", json!({"clinet":"typo"})),
            ("list_job", json!({})),
            ("list_job", json!({"jobid":1,"ujobid":"x"})),
            ("list_job", json!({"jobid":-1})),
            ("list_copies", json!({"jobids":["1;x"]})),
            ("list_volumes", json!({"pool":"Full","volume":"x"})),
            ("list_backups", json!({})),
            ("list_nextvolume", json!({"job":"x","days":51})),
            ("show", json!({"resource":"password"})),
            ("show", json!({"resource":"job","name":"a b\tc"})),
            ("show", json!({})),
            ("status_director", json!({"days":501})),
            ("status_scheduler", json!({"days_from":1})),
            ("dot_jobs", json!({"type":"backup"})),
            ("dot_defaults", json!({"name":""})),
            ("dot_jobstatus", json!({"code":"TT"})),
            ("version", json!({"format":"xml"})),
        ] {
            rejected(tool, args);
        }
    }

    struct Fake(Vec<(bool, String)>);
    impl CommandRunner for Fake {
        fn run(&mut self, command: &str) -> anyhow::Result<String> {
            self.0.push((false, command.into()));
            Ok(format!("text {command}\n"))
        }
        fn run_json(&mut self, command: &str) -> anyhow::Result<String> {
            self.0.push((true, command.into()));
            Ok(match command {
                ".clients" => r#"{"jsonrpc":"2.0","id":null,"result":{"clients":[{"name":"bareos-fd","enabled":true}]}}"#.into(),
                "list jobid=99" => r#"{"jsonrpc":"2.0","id":null,"error":{"code":1,"message":"failed"}}"#.into(),
                _ => format!(r#"{{"jsonrpc":"2.0","id":null,"result":{{"cmd":"{command}"}}}}"#),
            })
        }
    }

    #[test]
    fn run_uses_json_api_text_and_name_checks() {
        let mut fake = Fake(vec![]);
        let r = run(&mut fake, &command("list_jobs", &json!({"limit":1})).unwrap().unwrap());
        assert_eq!(r["isError"], false);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("list jobs limit=1"));
        let r = run(&mut fake, &command("list_job", &json!({"jobid":99})).unwrap().unwrap());
        assert_eq!(r["isError"], true);
        let r = run(&mut fake, &command("status_client", &json!({"client":"nosuch"})).unwrap().unwrap());
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("known: bareos-fd"));
        let r = run(&mut fake, &command("status_client", &json!({"client":"bareos-fd"})).unwrap().unwrap());
        assert_eq!(r["content"][0]["text"], "text status client=bareos-fd\n");
        assert_eq!(fake.0, vec![
            (true, "list jobs limit=1".to_string()),
            (true, "list jobid=99".into()),
            (true, ".clients".into()),
            (true, ".clients".into()),
            (false, "status client=bareos-fd".into()),
        ]);
    }
}
