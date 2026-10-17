//! S-35: write, session and restore-tree RPCs of DirectorApi.
//!
//! The command lines (plans) are always compiled, so their unit tests run
//! in every build; the RPC methods that execute them exist only with the
//! cargo feature `grpc-development` (api.rs). The production build answers
//! UNIMPLEMENTED, like oconsole's MCP write tools in its production build.
//!
//! Every write RPC builds exactly one console command from the typed
//! request (`… yes` where the command knows it): the arguments go into the
//! command AST as they are, so no value can add a second keyword or
//! command. The command runs through the command queue with the console
//! ACLs of the session, in text mode with a console that answers only the
//! questions the plan names ("Purge (yes/no)?" …) and scripted answers
//! (query, sqlquery); any other question is cancelled and reported in
//! `ActionResponse.question`. Destructive RPCs (Relabel, Prune*, Purge*,
//! Delete*, Truncate) need `confirm = true`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value};

use crate::command_ast::{Argument, CommandAst};
use crate::ua_select::Console;

use super::api::console_line;

/// What runs after the command: typed records of what it changed.
#[derive(Debug, Clone, PartialEq)]
pub enum Follow {
    None,
    /// JobIds from "Job queued. JobId=N" in the reply
    QueuedJobs,
    JobIds(Vec<u64>),
    Volumes(Vec<String>),
    Pools(Vec<String>),
}

/// How the command runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Text mode with an answering console (default)
    Text,
    /// API 2 JSON mode without console (result into ActionResponse.result)
    Json,
}

/// One console command, the questions answered with "yes", scripted
/// answers for other prompts, and the follow-up query.
#[derive(Debug, Clone)]
pub struct Plan {
    pub ast: CommandAst,
    pub yes_to: &'static [&'static str],
    pub answers: Vec<String>,
    pub follow: Follow,
    pub mode: Mode,
}

impl Plan {
    fn new(line: Line) -> Self {
        Self { ast: line.ast(), yes_to: &[], answers: Vec::new(), follow: Follow::None, mode: Mode::Text }
    }
    fn yes_to(mut self, questions: &'static [&'static str]) -> Self { self.yes_to = questions; self }
    fn follow(mut self, follow: Follow) -> Self { self.follow = follow; self }
    fn json(mut self) -> Self { self.mode = Mode::Json; self }
    fn answers(mut self, answers: Vec<String>) -> Self { self.answers = answers; self }

    /// The console line (for logs, tests and ActionResponse.command).
    pub fn line(&self) -> String {
        self.ast.raw_line.clone()
    }
}

/// Builder of the command arguments.
pub struct Line {
    command: String,
    args: Vec<Argument>,
}

impl Line {
    fn new(words: &str) -> Self {
        let mut it = words.split(' ');
        let command = it.next().unwrap_or("").to_string();
        Self { command, args: it.map(|w| Argument { key: w.into(), value: None }).collect() }
    }
    fn word(&mut self, word: &str) -> &mut Self {
        self.args.push(Argument { key: word.into(), value: None });
        self
    }
    fn kv(&mut self, key: &str, value: impl Into<String>) -> &mut Self {
        self.args.push(Argument { key: key.into(), value: Some(value.into()) });
        self
    }
    fn ast(self) -> CommandAst {
        let mut ast = CommandAst { command_name: self.command, arguments: self.args, ..Default::default() };
        ast.raw_line = console_line(&ast);
        ast
    }
}

/// Typed access to a decoded request (proto field names; only fields
/// present on the wire).
pub struct Req<'a>(pub &'a Map<String, Value>);

impl Req<'_> {
    fn has(&self, key: &str) -> bool {
        match self.0.get(key) {
            None | Some(Value::Null) => false,
            Some(Value::String(s)) => !s.is_empty(),
            Some(Value::Array(a)) => !a.is_empty(),
            Some(Value::Bool(b)) => *b,
            Some(_) => true,
        }
    }
    /// A string value (non-empty); no control characters or quotes.
    fn text(&self, key: &str) -> Result<Option<String>, String> {
        match self.0.get(key) {
            Some(Value::String(s)) if !s.is_empty() => {
                if s.chars().any(|c| c.is_control() || c == '"') {
                    return Err(format!("{key}: control characters and '\"' are not allowed"));
                }
                Ok(Some(s.clone()))
            }
            _ => Ok(None),
        }
    }
    fn req(&self, key: &str) -> Result<String, String> {
        self.text(key)?.ok_or_else(|| format!("{key} is required"))
    }
    fn num(&self, key: &str) -> Option<u64> {
        self.0.get(key).and_then(Value::as_u64)
    }
    /// A number given on the wire (optional fields: also 0).
    fn num_present(&self, key: &str) -> Option<u64> {
        self.num(key)
    }
    fn flag(&self, key: &str) -> bool {
        self.0.get(key) == Some(&Value::Bool(true))
    }
    /// An optional bool: Some(false) when sent as false.
    fn opt_bool(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(Value::as_bool)
    }
    fn nums(&self, key: &str) -> Vec<u64> {
        self.0.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default()
    }
    fn texts(&self, key: &str) -> Result<Vec<String>, String> {
        let items: Vec<String> = self.0.get(key).and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if items.iter().any(|s| s.is_empty() || s.chars().any(|c| c.is_control() || c == '"')) {
            return Err(format!("{key}: empty values, control characters and '\"' are not allowed"));
        }
        Ok(items)
    }
    fn confirmed(&self, rpc: &str) -> Result<(), String> {
        if self.flag("confirm") {
            Ok(())
        } else {
            Err(format!("{rpc} is destructive: set confirm = true after the user confirmed it"))
        }
    }
    /// Exactly one of `keys` is set.
    fn one_of(&self, keys: &[&str]) -> Result<String, String> {
        let set: Vec<&&str> = keys.iter().filter(|k| self.has(k)).collect();
        match set.as_slice() {
            [one] => Ok(one.to_string()),
            _ => Err(format!("give exactly one of {}", keys.join(", "))),
        }
    }
    fn reject(&self, keys: &[&str], what: &str) -> Result<(), String> {
        match keys.iter().find(|k| self.has(k)) {
            Some(k) => Err(format!("{k} does not apply to {what}")),
            None => Ok(()),
        }
    }
    fn choice(&self, key: &str, allowed: &[&str]) -> Result<Option<String>, String> {
        match self.text(key)? {
            None => Ok(None),
            Some(v) => allowed.iter().find(|a| a.eq_ignore_ascii_case(&v)).map(|a| Some(a.to_string()))
                .ok_or_else(|| format!("{key} must be one of {}", allowed.join(", "))),
        }
    }
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn ids(list: &[u64]) -> String {
    list.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

fn push_texts(line: &mut Line, r: &Req, keys: &[&str]) -> Result<(), String> {
    for key in keys {
        if let Some(v) = r.text(key)? {
            line.kv(key, v);
        }
    }
    Ok(())
}

fn push_nums(line: &mut Line, r: &Req, keys: &[&str]) {
    for key in keys {
        if let Some(v) = r.num(key).filter(|v| *v > 0) {
            line.kv(key, v.to_string());
        }
    }
}

const LEVELS: &[&str] = &["Full", "Incremental", "Differential", "Since", "VirtualFull", "Base",
    "InitCatalog", "Catalog", "VolumeToCatalog", "DiskToCatalog", "Data"];
const REPLACE: &[&str] = &["always", "ifnewer", "ifolder", "never"];
const CANCEL_STATES: &[&str] = &["created", "blocked", "waiting", "running"];
const VOLSTATUS: &[&str] = &["Append", "Full", "Used", "Recycle", "Purged", "Error", "Archive", "Read-Only", "Disabled", "Cleaning"];
const ENABLED: &[&str] = &["yes", "no", "archived"];
const ACTION_ON_PURGE: &[&str] = &["none", "truncate"];
const ENCRYPT: &[&str] = &["yes", "no", "rotate"];

/// Plan of a write RPC for a decoded request; None: not a planned RPC.
pub fn plan(rpc: &str, m: &Map<String, Value>) -> Option<Result<Plan, String>> {
    let r = Req(m);
    let builder: fn(&Req) -> Result<Plan, String> = match rpc {
        "Run" => run,
        "Rerun" => rerun,
        "Cancel" => cancel,
        "Restore" => restore,
        "Label" => label,
        "Relabel" => relabel,
        "Mount" => |r| storage_cmd(r, "mount", true),
        "Unmount" => |r| storage_cmd(r, "unmount", false),
        "Release" => |r| storage_cmd(r, "release", false),
        "AddVolumes" => |r| {
            let mut l = Line::new("add");
            push_texts(&mut l, r, &["pool", "storage"])?;
            push_nums(&mut l, r, &["jobid"]);
            Ok(Plan::new(l))
        },
        "CreatePool" => |r| {
            let pool = r.req("pool")?;
            let mut l = Line::new("create");
            l.kv("pool", pool.clone());
            Ok(Plan::new(l).follow(Follow::Pools(vec![pool])))
        },
        "DeleteVolume" => |r| delete(r, "volume"),
        "DeletePool" => |r| delete(r, "pool"),
        "DeleteJob" => |r| delete(r, "jobid"),
        "DeleteStorage" => |r| delete(r, "storage"),
        "Enable" => |r| toggle(r, "enable"),
        "Disable" => |r| toggle(r, "disable"),
        "ExportVolumes" => |r| slots_cmd(r, "export"),
        "ImportVolumes" => |r| slots_cmd(r, "import"),
        "MoveVolumes" => |r| slots_cmd(r, "move"),
        "PruneFiles" => |r| prune_target(r, "files"),
        "PruneJobs" => |r| prune_target(r, "jobs"),
        "PruneStats" => |r| prune_target(r, "stats"),
        "PruneVolume" => prune_volume,
        "PruneDirectory" => prune_directory,
        "PurgeFiles" => |r| purge(r, "files"),
        "PurgeJobs" => |r| purge(r, "jobs"),
        "PurgeVolume" => |r| purge(r, "volume"),
        "PurgeQuota" => |r| purge(r, "quota"),
        "UpdateVolume" => update_volume,
        "UpdatePool" => |r| {
            let pool = r.req("pool")?;
            let mut l = Line::new("update");
            l.kv("pool", pool.clone());
            Ok(Plan::new(l).follow(Follow::Pools(vec![pool])))
        },
        "UpdateSlots" => |r| {
            let mut l = Line::new("update slots");
            push_texts(&mut l, r, &["storage"])?;
            if r.flag("scan") {
                l.word("scan");
            }
            if let Some(d) = r.0.get("drive").and_then(Value::as_i64) {
                l.kv("drive", d.to_string());
            }
            Ok(Plan::new(l))
        },
        "UpdateJob" => |r| {
            let jobid = r.num("jobid").filter(|j| *j > 0).ok_or("jobid is required")?;
            let mut l = Line::new("update");
            l.kv("jobid", jobid.to_string());
            push_texts(&mut l, r, &["jobname", "starttime", "client", "filesetid", "jobtype"])?;
            Ok(Plan::new(l).follow(Follow::JobIds(vec![jobid])))
        },
        "UpdateStats" => |r| {
            let mut l = Line::new("update stats");
            push_nums(&mut l, r, &["days"]);
            Ok(Plan::new(l))
        },
        "Truncate" => truncate,
        "ConfigureAdd" => |r| configure(r, "add"),
        "ConfigureExport" => |r| {
            let mut l = Line::new("configure export");
            l.kv("client", r.req("client")?);
            Ok(Plan::new(l))
        },
        "ConfigureDelete" => configure_delete,
        "ConfigureUpdate" => configure_update,
        "Reload" => |_| Ok(Plan::new(Line::new("reload"))),
        "SetDebug" => setdebug,
        "SetBandwidth" => setbandwidth,
        "SetDevice" => |r| {
            let mut l = Line::new("setdevice");
            l.kv("storage", r.req("storage")?).kv("device", r.req("device")?);
            if let Some(v) = r.choice("autoselect", &["yes", "no"])? {
                l.kv("autoselect", v);
            }
            Ok(Plan::new(l))
        },
        "Trace" => |r| on_off(r, "trace"),
        "Autodisplay" => |r| on_off(r, "autodisplay"),
        "Automount" => |r| on_off(r, "automount"),
        "UseCatalog" => |r| {
            let mut l = Line::new("use");
            l.kv("catalog", r.req("catalog")?);
            Ok(Plan::new(l))
        },
        "Wait" => |r| {
            let mut l = Line::new("wait");
            push_texts(&mut l, r, &["jobname", "ujobid"])?;
            push_nums(&mut l, r, &["jobid", "timeout"]);
            Ok(Plan::new(l))
        },
        "DotSql" => |r| {
            let mut l = Line::new(".sql");
            l.kv("query", r.req("query")?);
            Ok(Plan::new(l).json())
        },
        "SqlQuery" => |r| {
            let mut query = r.req("query")?;
            if !query.trim_end().ends_with(';') {
                query.push(';');
            }
            Ok(Plan::new(Line::new("sqlquery")).answers(vec![query, String::new()]))
        },
        "Query" => |r| {
            let number = r.num("number").filter(|n| *n > 0).ok_or("number is required")?;
            let mut answers = vec![number.to_string()];
            answers.extend(r.texts("parameters")?);
            Ok(Plan::new(Line::new("query")).answers(answers))
        },
        "DotBvfsUpdate" => |r| {
            let mut l = Line::new(".bvfs_update");
            push_nums(&mut l, r, &["jobid"]);
            Ok(Plan::new(l).json())
        },
        "DotBvfsRestore" => |r| {
            let mut l = Line::new(".bvfs_restore");
            l.kv("path", r.req("path")?);
            push_nums(&mut l, r, &["jobid"]);
            push_texts(&mut l, r, &["fileid", "dirid", "hardlink"])?;
            Ok(Plan::new(l).json())
        },
        "DotBvfsCleanup" => |r| {
            let mut l = Line::new(".bvfs_cleanup");
            l.kv("path", r.req("path")?);
            Ok(Plan::new(l).json())
        },
        "DotBvfsClearCache" => |_| Ok(Plan::new(Line::new(".bvfs_clear_cache yes")).json()),
        "DotAclSet" => |r| acl_change(r, "set"),
        "DotAclDelete" => |r| acl_change(r, "delete"),
        "DotAclClear" => |r| {
            let mut l = Line::new(".acl clear");
            l.kv("console", r.req("console")?);
            Ok(Plan::new(l).json())
        },
        "DotAclCopy" => |r| {
            let mut l = Line::new(".acl copy");
            l.kv("from", r.req("from")?).kv("to", r.req("to")?);
            Ok(Plan::new(l).json())
        },
        "DotProfileCreate" => |r| profile(r, "create", &["name"]),
        "DotProfileSet" => |r| profile(r, "set", &["name", "type", "value"]),
        "DotProfileDelete" => |r| profile(r, "delete", &["name", "type", "value"]),
        "DotProfileAssign" => |r| profile(r, "assign", &["console", "profile"]),
        "DotProfileUnassign" => |r| profile(r, "unassign", &["console", "profile"]),
        _ => return None,
    };
    Some(builder(&r))
}

fn run(r: &Req) -> Result<Plan, String> {
    let mut l = Line::new("run");
    l.kv("job", r.req("job")?);
    if let Some(level) = r.choice("level", LEVELS)? {
        l.kv("level", level);
    }
    push_texts(&mut l, r, &["client", "fileset", "pool", "nextpool", "storage", "when", "since"])?;
    push_nums(&mut l, r, &["priority", "jobid"]);
    for key in ["spooldata", "accurate"] {
        if let Some(v) = r.opt_bool(key) {
            l.kv(key, yes_no(v));
        }
    }
    push_texts(&mut l, r, &["comment", "catalog", "pluginoptions"])?;
    l.word("yes");
    Ok(Plan::new(l).follow(Follow::QueuedJobs))
}

fn rerun(r: &Req) -> Result<Plan, String> {
    let mut l = Line::new("rerun");
    let jobids = r.nums("jobid");
    let since = r.num("since_jobid").filter(|v| *v > 0);
    let until = r.num("until_jobid").filter(|v| *v > 0);
    let period = r.has("days") || r.has("hours");
    match (jobids.is_empty(), since, period) {
        (false, None, false) if until.is_none() => { l.kv("jobid", ids(&jobids)); }
        (true, Some(since), false) => {
            l.kv("since_jobid", since.to_string());
            if let Some(u) = until {
                l.kv("until_jobid", u.to_string());
            }
        }
        (true, None, true) if until.is_none() => push_nums(&mut l, r, &["days", "hours"]),
        _ => return Err("give jobid, or since_jobid [until_jobid], or days/hours".into()),
    }
    l.word("yes");
    Ok(Plan::new(l).follow(Follow::QueuedJobs))
}

fn cancel(r: &Req) -> Result<Plan, String> {
    let mut l = Line::new("cancel");
    match r.one_of(&["jobid", "job", "ujobid", "all", "state"])?.as_str() {
        "jobid" => { l.kv("jobid", r.num("jobid").unwrap_or(0).to_string()); }
        "all" => { l.word("all"); }
        "state" => { l.kv("state", r.choice("state", CANCEL_STATES)?.unwrap_or_default()); }
        key => { l.kv(key, r.req(key)?); }
    }
    if r.has("storage") {
        if !r.has("jobid") {
            return Err("storage needs jobid".into());
        }
        l.kv("storage", r.req("storage")?);
    }
    l.word("yes");
    let jobids = r.num("jobid").map(|j| vec![j]).unwrap_or_default();
    Ok(Plan::new(l).follow(if jobids.is_empty() { Follow::None } else { Follow::JobIds(jobids) }))
}

/// Selection part shared by Restore and StartRestoreSession.
fn restore_selection(r: &Req, l: &mut Line, files_given: bool) -> Result<(), String> {
    let jobids = r.nums("jobid");
    let before = r.text("before")?;
    match (jobids.is_empty(), &before) {
        (false, None) => { l.kv("jobid", ids(&jobids)); }
        (true, Some(date)) => { l.word("select").kv("before", date.clone()); }
        (true, None) if !files_given => { l.word("select").word("current"); }
        (true, None) => {}
        (false, Some(_)) => return Err("give jobid or before, not both".into()),
    }
    l.kv("client", r.req("client")?);
    push_texts(l, r, &["fileset", "pool"])?;
    if jobids.is_empty() && (before.is_some() || !files_given) && !r.has("fileset") {
        return Err("fileset is required to select the latest backup or the state before a date".into());
    }
    Ok(())
}

fn restore_target(r: &Req, l: &mut Line) -> Result<(), String> {
    let target = ["where", "regexwhere"].iter().filter(|k| r.has(k)).count()
        + usize::from(["strip_prefix", "add_prefix", "add_suffix"].iter().any(|k| r.has(k)));
    if target > 1 {
        return Err("give only one of where, regexwhere or strip_prefix/add_prefix/add_suffix".into());
    }
    push_texts(l, r, &["where", "regexwhere", "strip_prefix", "add_prefix", "add_suffix"])?;
    if let Some(v) = r.choice("replace", REPLACE)? {
        l.kv("replace", v);
    }
    push_texts(l, r, &["restoreclient", "restorejob", "storage", "comment", "pluginoptions"])
}

fn restore(r: &Req) -> Result<Plan, String> {
    let mut l = Line::new("restore");
    let files = r.texts("file")?;
    let dirs = r.texts("directory")?;
    restore_selection(r, &mut l, !files.is_empty())?;
    let regex = r.text("fileregex")?;
    let selections = [r.flag("all"), !files.is_empty(), !dirs.is_empty(), regex.is_some()].iter().filter(|x| **x).count();
    if selections != 1 {
        return Err("select files with exactly one of all, file, directory or fileregex".into());
    }
    if r.flag("all") {
        l.word("all");
    }
    for file in &files {
        if !file.starts_with('/') && !file.contains(":/") {
            return Err(format!("file '{file}' must be a full path"));
        }
        l.kv("file", file.clone());
    }
    if !dirs.is_empty() && r.nums("jobid").is_empty() {
        return Err("directory needs jobid".into());
    }
    for dir in &dirs {
        if !dir.ends_with('/') {
            return Err(format!("directory '{dir}' must end with /"));
        }
        l.kv("directory", dir.clone());
    }
    if let Some(regex) = regex {
        l.kv("fileregex", regex);
    }
    l.word("done");
    restore_target(r, &mut l)?;
    l.word("yes");
    Ok(Plan::new(l).follow(Follow::QueuedJobs))
}

/// `restore ...` up to the file selection tree (StartRestoreSession).
pub fn restore_session_plan(m: &Map<String, Value>) -> Result<Plan, String> {
    let r = Req(m);
    let mut l = Line::new("restore");
    restore_selection(&r, &mut l, false)?;
    restore_target(&r, &mut l)?;
    Ok(Plan::new(l).follow(Follow::QueuedJobs))
}

fn label(r: &Req) -> Result<Plan, String> {
    let mut l = Line::new("label");
    l.kv("storage", r.req("storage")?);
    let volume = r.text("volume")?;
    match (&volume, r.flag("barcodes")) {
        (Some(v), false) => { l.kv("volume", v.clone()); }
        (None, true) => {}
        _ => return Err("give volume or barcodes".into()),
    }
    l.kv("pool", r.req("pool")?);
    push_nums(&mut l, r, &["slot", "drive"]);
    if r.flag("encrypt") {
        l.word("encrypt");
    }
    if r.flag("barcodes") {
        l.word("barcodes");
    }
    l.word("yes");
    Ok(Plan::new(l).follow(volume.map(|v| Follow::Volumes(vec![v])).unwrap_or(Follow::None)))
}

fn relabel(r: &Req) -> Result<Plan, String> {
    r.confirmed("Relabel")?;
    let mut l = Line::new("relabel");
    let volume = r.req("volume")?;
    l.kv("storage", r.req("storage")?).kv("oldvolume", r.req("oldvolume")?).kv("volume", volume.clone());
    push_texts(&mut l, r, &["pool"])?;
    push_nums(&mut l, r, &["slot", "drive"]);
    if r.flag("encrypt") {
        l.word("encrypt");
    }
    l.word("yes");
    Ok(Plan::new(l).follow(Follow::Volumes(vec![volume])))
}

fn storage_cmd(r: &Req, command: &str, slot: bool) -> Result<Plan, String> {
    let mut l = Line::new(command);
    let allowed: &[&str] = if command == "release" { &["storage"] } else { &["storage", "jobid", "job", "ujobid"] };
    match r.one_of(allowed)?.as_str() {
        "jobid" => { l.kv("jobid", r.num("jobid").unwrap_or(0).to_string()); }
        key => { l.kv(key, r.req(key)?); }
    }
    if let Some(d) = r.num_present("drive") {
        l.kv("drive", d.to_string());
    }
    if slot {
        push_nums(&mut l, r, &["slot"]);
    }
    Ok(Plan::new(l))
}

fn delete(r: &Req, kind: &str) -> Result<Plan, String> {
    r.confirmed(match kind { "volume" => "DeleteVolume", "pool" => "DeletePool", "jobid" => "DeleteJob", _ => "DeleteStorage" })?;
    let mut l = Line::new("delete");
    let plan = match kind {
        "volume" => {
            r.reject(&["jobid", "storage"], "DeleteVolume")?;
            let volume = r.req("volume")?;
            l.kv("volume", volume);
            push_texts(&mut l, r, &["pool"])?;
            l.word("yes");
            Plan::new(l)
        }
        "pool" => {
            r.reject(&["volume", "jobid", "storage"], "DeletePool")?;
            l.kv("pool", r.req("pool")?);
            Plan::new(l)
        }
        "jobid" => {
            r.reject(&["volume", "pool", "storage"], "DeleteJob")?;
            let jobids = r.nums("jobid");
            if jobids.is_empty() {
                return Err("jobid is required".into());
            }
            l.kv("jobid", ids(&jobids));
            Plan::new(l)
        }
        _ => {
            r.reject(&["volume", "pool", "jobid"], "DeleteStorage")?;
            // S-38: `delete storage=<n>` — a bare `storage` before it would
            // make FindArgWithValue stop at the keyword and prompt.
            l.kv("storage", r.req("storage")?);
            Plan::new(l)
        }
    };
    Ok(plan.yes_to(&["Are you sure you want to delete"]))
}

fn toggle(r: &Req, command: &str) -> Result<Plan, String> {
    let mut l = Line::new(command);
    let key = r.one_of(&["job", "client", "schedule", "storage"])?;
    l.kv(&key, r.req(&key)?);
    Ok(Plan::new(l))
}

fn slots_cmd(r: &Req, command: &str) -> Result<Plan, String> {
    let mut l = Line::new(command);
    l.kv("storage", r.req("storage")?);
    push_texts(&mut l, r, &["srcslots", "dstslots", "volume"])?;
    Ok(Plan::new(l))
}

fn prune_target(r: &Req, kind: &str) -> Result<Plan, String> {
    let rpc = match kind { "files" => "PruneFiles", "jobs" => "PruneJobs", _ => "PruneStats" };
    r.confirmed(rpc)?;
    let mut l = Line::new("prune");
    l.word(kind);
    if kind == "stats" {
        r.reject(&["client", "pool"], "PruneStats")?;
    } else {
        if !r.has("client") && !r.has("pool") {
            return Err(format!("prune {kind} needs client or pool"));
        }
        push_texts(&mut l, r, &["client", "pool"])?;
    }
    l.word("yes");
    Ok(Plan::new(l))
}

fn prune_volume(r: &Req) -> Result<Plan, String> {
    r.confirmed("PruneVolume")?;
    let mut l = Line::new("prune");
    let volume = r.text("volume")?;
    match (&volume, r.flag("all")) {
        (Some(v), false) => { l.kv("volume", v.clone()); }
        (None, true) => { l.word("volume").word("all"); }
        _ => return Err("give volume or all".into()),
    }
    push_texts(&mut l, r, &["pool"])?;
    l.word("yes");
    Ok(Plan::new(l).follow(volume.map(|v| Follow::Volumes(vec![v])).unwrap_or(Follow::None)))
}

fn prune_directory(r: &Req) -> Result<Plan, String> {
    r.confirmed("PruneDirectory")?;
    let mut l = Line::new("prune");
    l.kv("directory", r.req("directory")?).kv("client", r.req("client")?);
    if r.flag("recursive") {
        l.word("recursive");
    }
    l.word("yes");
    Ok(Plan::new(l))
}

fn purge(r: &Req, kind: &str) -> Result<Plan, String> {
    let rpc = format!("Purge{}{}", kind[..1].to_ascii_uppercase(), &kind[1..]);
    r.confirmed(&rpc)?;
    let allowed: &[&str] = match kind {
        "files" => &["jobid", "job", "client", "volume"],
        "jobs" => &["client", "volume", "pool", "jobstatus"],
        "volume" => &["volume", "jobstatus", "action", "storage", "pool", "allpools", "drive", "devicetype"],
        _ => &["client"],
    };
    let all = ["jobid", "job", "client", "volume", "pool", "jobstatus", "action", "storage", "allpools", "drive", "devicetype"];
    let foreign: Vec<&str> = all.iter().copied().filter(|k| !allowed.contains(k)).collect();
    r.reject(&foreign, &rpc)?;
    let volumes = r.texts("volume")?;
    let mut l = Line::new("purge");
    match kind {
        "files" => {
            r.one_of(&["jobid", "job", "client", "volume"])?;
            l.word("files");
            push_nums(&mut l, r, &["jobid"]);
            push_texts(&mut l, r, &["job", "client"])?;
        }
        "jobs" => {
            r.one_of(&["client", "volume", "pool"])?;
            l.word("jobs");
            push_texts(&mut l, r, &["client", "pool"])?;
        }
        "volume" => {
            if volumes.is_empty() && !r.has("action") {
                return Err("PurgeVolume needs volume (or action)".into());
            }
            if volumes.is_empty() {
                l.word("volume");
            }
        }
        _ => {
            l.word("quota");
            push_texts(&mut l, r, &["client"])?;
        }
    }
    if kind != "volume" && volumes.len() > 1 {
        return Err(format!("{rpc} takes one volume"));
    }
    for v in &volumes {
        l.kv("volume", v.clone());
    }
    if let Some(s) = r.text("jobstatus")? {
        if !s.split(',').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphabetic())) {
            return Err("jobstatus must be status letters, comma-separated".into());
        }
        l.kv("jobstatus", s);
    }
    if kind == "volume" {
        if let Some(action) = r.choice("action", &["truncate", "all"])? {
            l.kv("action", action);
            push_texts(&mut l, r, &["storage", "pool", "devicetype"])?;
            if r.flag("allpools") {
                l.word("allpools");
            }
            if let Some(d) = r.num_present("drive") {
                l.kv("drive", d.to_string());
            }
        } else if ["storage", "pool", "allpools", "drive", "devicetype"].iter().any(|k| r.has(k)) {
            return Err("storage, pool, allpools, drive and devicetype need action".into());
        }
    }
    let follow = if kind == "volume" && !volumes.is_empty() { Follow::Volumes(volumes) } else { Follow::None };
    Ok(Plan::new(l).yes_to(&["Purge (yes/no)?"]).follow(follow))
}

fn update_volume(r: &Req) -> Result<Plan, String> {
    let mut l = Line::new("update");
    let target = r.one_of(&["volume", "allfrompool", "fromallpools"])?;
    match target.as_str() {
        "volume" => { l.kv("volume", r.req("volume")?); }
        "allfrompool" => { l.word("volume").kv("allfrompool", r.req("allfrompool")?); }
        _ => { l.word("volume").word("fromallpools"); }
    }
    let base = l.args.len();
    if let Some(v) = r.choice("volstatus", VOLSTATUS)? {
        l.kv("volstatus", v);
    }
    push_texts(&mut l, r, &["volretention", "voluse", "maxvolbytes"])?;
    for key in ["maxvoljobs", "maxvolfiles", "slot"] {
        if let Some(v) = r.num_present(key) {
            l.kv(key, v.to_string());
        }
    }
    for key in ["recycle", "inchanger"] {
        if let Some(v) = r.opt_bool(key) {
            l.kv(key, yes_no(v));
        }
    }
    if let Some(v) = r.choice("enabled", ENABLED)? {
        l.kv("enabled", v);
    }
    if let Some(v) = r.choice("actiononpurge", ACTION_ON_PURGE)? {
        l.kv("actiononpurge", v);
    }
    push_texts(&mut l, r, &["pool", "recyclepool", "storage"])?;
    if r.flag("frompool") {
        l.kv("frompool", "yes");
    }
    if let Some(v) = r.choice("encrypt", ENCRYPT)? {
        l.kv("encrypt", v);
    }
    if target == "volume" && l.args.len() == base {
        return Err("give at least one property to change".into());
    }
    if target != "volume" && l.args.len() > base {
        return Err("allfrompool/fromallpools take no other properties".into());
    }
    let follow = r.text("volume")?.map(|v| Follow::Volumes(vec![v])).unwrap_or(Follow::None);
    Ok(Plan::new(l).follow(follow))
}

fn truncate(r: &Req) -> Result<Plan, String> {
    r.confirmed("Truncate")?;
    let mut l = Line::new("truncate volstatus=Purged");
    // "volstatus=Purged" came in as one word: split it like the console
    l.args[0] = Argument { key: "volstatus".into(), value: Some("Purged".into()) };
    push_texts(&mut l, r, &["storage", "pool", "volume"])?;
    if let Some(d) = r.num_present("drive") {
        l.kv("drive", d.to_string());
    }
    l.word("yes");
    let follow = r.text("volume")?.map(|v| Follow::Volumes(vec![v])).unwrap_or(Follow::None);
    Ok(Plan::new(l).follow(follow))
}

/// ConfigureResourceType number -> configure keyword.
fn resource_type(r: &Req) -> Result<String, String> {
    let number = r.0.get("resource_type").and_then(Value::as_i64).unwrap_or(0) as i32;
    let registry = onesimus_proto_rs::typed::registry();
    let e = registry.enumeration(".onesimus.director.commands.ConfigureResourceType").ok_or("no enum")?;
    e.value.iter().find(|v| v.number() == number && number != 0)
        .map(|v| v.name().trim_start_matches("CONFIGURE_RESOURCE_").to_ascii_lowercase())
        .ok_or_else(|| "resource_type is required".to_string())
}

fn configure(r: &Req, action: &str) -> Result<Plan, String> {
    let kind = resource_type(r)?;
    let mut l = Line::new(&format!("configure {action}"));
    l.word(&kind).kv("name", r.req("name")?);
    if let Some(Value::Object(directives)) = r.0.get("directives") {
        for (key, value) in directives {
            if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) || key == "name" {
                return Err(format!("directive '{key}': lowercase letters and digits only (not name)"));
            }
            let value = value.as_str().unwrap_or("");
            if value.is_empty() || value.chars().any(|c| c.is_control() || c == '"' || c == '@' || c == ';') {
                return Err(format!("directive '{key}': empty value or not allowed characters"));
            }
            l.kv(key, value);
        }
    }
    Ok(Plan::new(l))
}

/// Resource types `configure delete` removes (commands/configure.rs).
const CONFIGURE_DELETE_TYPES: &[&str] = &["client", "job", "pool", "storage", "console", "schedule"];
/// Resource types and directives `configure update` changes.
const CONFIGURE_UPDATE: &[(&str, &[&str])] = &[
    ("client", &["address", "port", "password"]),
    ("job", &["newclient", "newstorage", "newpool"]),
];

/// S-38: `configure delete <type>=<name> [yes]`, the line the command
/// accepts (it takes the first `<type>=<name>` argument).
fn configure_delete(r: &Req) -> Result<Plan, String> {
    let kind = resource_type(r)?;
    if !CONFIGURE_DELETE_TYPES.contains(&kind.as_str()) {
        return Err(format!("configure delete supports resource types {}", CONFIGURE_DELETE_TYPES.join(", ")));
    }
    let mut l = Line::new("configure delete");
    l.kv(&kind, r.req("name")?);
    if r.flag("yes") {
        l.word("yes");
    }
    Ok(Plan::new(l))
}

/// S-38: `configure update client=<n> address=… port=… password=…` or
/// `configure update job=<n> newclient=… newstorage=… newpool=…`, the
/// lines the command accepts.
fn configure_update(r: &Req) -> Result<Plan, String> {
    let kind = resource_type(r)?;
    let (_, allowed) = CONFIGURE_UPDATE.iter().find(|(k, _)| *k == kind)
        .ok_or_else(|| format!("configure update supports resource types {}",
            CONFIGURE_UPDATE.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")))?;
    let mut l = Line::new("configure update");
    l.kv(&kind, r.req("name")?);
    let Some(Value::Object(directives)) = r.0.get("directives").filter(|d| d.as_object().is_some_and(|m| !m.is_empty())) else {
        return Err(format!("directives: give at least one of {}", allowed.join(", ")));
    };
    for key in *allowed {
        let Some(value) = directives.get(*key) else { continue };
        let value = value.as_str().unwrap_or("");
        if value.is_empty() || value.chars().any(|c| c.is_control() || c == '"' || c == '@' || c == ';') {
            return Err(format!("directive '{key}': empty value or not allowed characters"));
        }
        l.kv(key, value);
    }
    if let Some(key) = directives.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("directive '{key}' does not apply to {kind} (allowed: {})", allowed.join(", ")));
    }
    Ok(Plan::new(l))
}

fn setdebug(r: &Req) -> Result<Plan, String> {
    let mut l = Line::new("setdebug");
    l.kv("level", r.num("level").unwrap_or(0).to_string());
    for key in ["trace", "timestamp"] {
        if let Some(v) = r.opt_bool(key) {
            l.kv(key, if v { "1" } else { "0" });
        }
    }
    let target = r.one_of(&["dir", "all", "client", "storage"])?;
    if let Some(h) = r.num_present("hangup") {
        if !matches!(target.as_str(), "client" | "all") {
            return Err("hangup only with client or all".into());
        }
        l.kv("hangup", h.to_string());
    }
    match target.as_str() {
        "dir" | "all" => { l.word(&target); }
        key => { l.kv(key, r.req(key)?); }
    }
    Ok(Plan::new(l))
}

/// S-39: `limit` is `optional` in SetBandwidthLimitCommand, so 0 (no
/// limit) arrives as a present field.
fn setbandwidth(r: &Req) -> Result<Plan, String> {
    let limit = r.0.get("limit").and_then(Value::as_i64).filter(|l| *l >= 0).ok_or("limit is required")?;
    let mut l = Line::new("setbandwidth");
    l.kv("limit", limit.to_string());
    match r.one_of(&["client", "storage", "jobid", "job", "ujobid", "state", "all"])?.as_str() {
        "all" => { l.word("all"); }
        "jobid" => { l.kv("jobid", r.0.get("jobid").and_then(Value::as_i64).unwrap_or(0).to_string()); }
        "state" => { l.kv("state", r.choice("state", CANCEL_STATES)?.unwrap_or_default()); }
        key => { l.kv(key, r.req(key)?); }
    }
    l.word("yes");
    Ok(Plan::new(l))
}

fn on_off(r: &Req, command: &str) -> Result<Plan, String> {
    let mut l = Line::new(command);
    match (r.flag("on"), r.flag("off")) {
        (true, false) => { l.word("on"); }
        (false, true) => { l.word("off"); }
        _ => return Err("give exactly one of on, off".into()),
    }
    Ok(Plan::new(l))
}

/// `.acl set|delete console=<name> <type>acl=<value>`
fn acl_change(r: &Req, action: &str) -> Result<Plan, String> {
    let mut l = Line::new(&format!(".acl {action}"));
    l.kv("console", r.req("console")?);
    let kind = r.req("type")?.to_ascii_lowercase();
    let kind = if kind.ends_with("acl") { kind } else { format!("{kind}acl") };
    if !kind.chars().all(|c| c.is_ascii_lowercase()) {
        return Err("type: e.g. job, JobAcl, command".into());
    }
    match (action, r.text("value")?) {
        (_, Some(value)) => { l.kv(&kind, value); }
        ("delete", None) => { l.word(&kind); }
        _ => return Err("value is required".into()),
    }
    Ok(Plan::new(l).json())
}

fn profile(r: &Req, action: &str, keys: &[&str]) -> Result<Plan, String> {
    let mut l = Line::new(&format!(".profile {action}"));
    push_texts(&mut l, r, keys)?;
    Ok(Plan::new(l).json())
}

// ---------------------------------------------------------------------------
// Reply parsing
// ---------------------------------------------------------------------------

/// JobIds of "Job queued. JobId=N" (and "JobId=N" in rerun replies).
pub fn queued_jobids(text: &str) -> Vec<u64> {
    let mut ids = Vec::new();
    for line in text.lines() {
        if !line.contains("queued") {
            continue;
        }
        if let Some(rest) = line.split("JobId=").nth(1) {
            if let Ok(id) = rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse() {
                ids.push(id);
            }
        }
    }
    ids
}

/// The last line of the output so far: the Director's question.
pub fn question(output: &str) -> String {
    output.trim_end().rsplit('\n').next().unwrap_or("").trim().to_string()
}

/// Output and the cancelled question of an [`AnswerConsole`].
#[derive(Default, Debug)]
pub struct Transcript {
    pub text: String,
    pub question: Option<String>,
}

/// Console of a write RPC: records the output, answers the plan's
/// questions, cancels every other one.
pub struct AnswerConsole {
    pub transcript: Arc<Mutex<Transcript>>,
    yes_to: &'static [&'static str],
    answers: VecDeque<String>,
}

impl AnswerConsole {
    pub fn new(plan: &Plan) -> Self {
        Self { transcript: Arc::default(), yes_to: plan.yes_to, answers: plan.answers.iter().cloned().collect() }
    }
}

impl Console for AnswerConsole {
    fn send(&mut self, text: &str) -> bool {
        self.transcript.lock().unwrap().text.push_str(text);
        true
    }
    fn signal(&mut self, _: bareos_bnet::BnetSignal) -> bool {
        true
    }
    fn recv(&mut self) -> Option<String> {
        let mut t = self.transcript.lock().unwrap();
        let q = question(&t.text);
        let answer = if self.yes_to.iter().any(|p| q.contains(p)) {
            Some("yes".to_string())
        } else {
            self.answers.pop_front()
        };
        match &answer {
            Some(a) => {
                t.text.push_str(a);
                t.text.push('\n');
            }
            None => t.question = Some(q),
        }
        answer
    }
}

// ---------------------------------------------------------------------------
// Restore tree replies
// ---------------------------------------------------------------------------

/// Entries of ls/.lsdir/lsmark/find output: `*` marked, `+` marked below,
/// trailing `/` directory.
pub fn tree_entries(text: &str) -> Vec<(String, bool, bool)> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("cwd is:")).map(|line| {
        let line = line.trim_start();
        let marked = line.starts_with('*');
        let name = line.trim_start_matches(['*', '+']);
        let is_dir = name.ends_with('/');
        (name.to_string(), is_dir, marked)
    }).collect()
}

/// Numbers in a tree reply ("1,234 total files; 12 marked …").
pub fn numbers(text: &str) -> Vec<u64> {
    text.split(|c: char| !(c.is_ascii_digit() || c == ','))
        .filter_map(|w| w.replace(',', "").parse().ok()).collect()
}

/// `cwd is: /etc/` or the plain `.pwd` path.
pub fn cwd_of(text: &str) -> Option<String> {
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    Some(line.strip_prefix("cwd is: ").unwrap_or(line).trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn line(rpc: &str, request: Value) -> String {
        plan(rpc, request.as_object().unwrap()).unwrap().unwrap().line()
    }

    fn err(rpc: &str, request: Value) -> String {
        plan(rpc, request.as_object().unwrap()).unwrap().unwrap_err()
    }

    #[test]
    fn job_lines() {
        assert_eq!(line("Run", json!({"job": "backup-bareos-fd", "level": "full", "spooldata": false, "priority": 5})),
                   "run job=backup-bareos-fd level=Full priority=5 spooldata=no yes");
        assert_eq!(line("Run", json!({"job": "j", "comment": "nightly test"})), "run job=j comment=\"nightly test\" yes");
        assert!(err("Run", json!({"job": "j\nrun job=x"})).contains("control"));
        assert!(err("Run", json!({"level": "Full"})).contains("job is required"));
        assert_eq!(line("Rerun", json!({"jobid": [4, 5]})), "rerun jobid=4,5 yes");
        assert_eq!(line("Rerun", json!({"since_jobid": 3, "until_jobid": 9})), "rerun since_jobid=3 until_jobid=9 yes");
        assert!(err("Rerun", json!({"jobid": [4], "days": 1})).contains("give jobid"));
        assert_eq!(line("Cancel", json!({"jobid": 7})), "cancel jobid=7 yes");
        assert_eq!(line("Cancel", json!({"state": "waiting"})), "cancel state=waiting yes");
        assert!(err("Cancel", json!({"jobid": 7, "job": "x"})).contains("exactly one"));
    }

    #[test]
    fn restore_lines() {
        assert_eq!(line("Restore", json!({"client": "fd", "jobid": [5], "all": true, "where": "/tmp/r"})),
                   "restore jobid=5 client=fd all done where=/tmp/r yes");
        assert_eq!(line("Restore", json!({"client": "fd", "fileset": "SelfTest", "file": ["/etc/hosts"], "replace": "always"})),
                   "restore client=fd fileset=SelfTest file=/etc/hosts done replace=always yes");
        assert_eq!(line("Restore", json!({"client": "fd", "fileset": "SelfTest", "all": true})),
                   "restore select current client=fd fileset=SelfTest all done yes");
        assert!(err("Restore", json!({"client": "fd", "jobid": [5]})).contains("exactly one"));
        assert!(err("Restore", json!({"client": "fd", "all": true})).contains("fileset is required"));
        let session = restore_session_plan(json!({"client": "fd", "jobid": [5, 6], "where": "/tmp/r"}).as_object().unwrap());
        assert_eq!(session.unwrap().line(), "restore jobid=5,6 client=fd where=/tmp/r");
    }

    #[test]
    fn volume_lines() {
        assert_eq!(line("Label", json!({"storage": "File", "volume": "V1", "pool": "Full"})),
                   "label storage=File volume=V1 pool=Full yes");
        assert!(err("Relabel", json!({"storage": "File", "oldvolume": "A", "volume": "B"})).contains("confirm"));
        assert_eq!(line("Relabel", json!({"storage": "File", "oldvolume": "A", "volume": "B", "confirm": true})),
                   "relabel storage=File oldvolume=A volume=B yes");
        assert_eq!(line("Mount", json!({"storage": "File", "drive": 0, "slot": 3})), "mount storage=File drive=0 slot=3");
        assert_eq!(line("Unmount", json!({"storage": "File"})), "unmount storage=File");
        assert_eq!(line("Release", json!({"storage": "File"})), "release storage=File");
        assert_eq!(line("UpdateVolume", json!({"volume": "V1", "volstatus": "used", "recycle": false})),
                   "update volume=V1 volstatus=Used recycle=no");
        assert_eq!(line("UpdateVolume", json!({"volume": "V1", "maxvoljobs": 0})), "update volume=V1 maxvoljobs=0");
        assert!(err("UpdateVolume", json!({"volume": "V1"})).contains("at least one"));
        assert_eq!(line("UpdatePool", json!({"pool": "Full"})), "update pool=Full");
        assert_eq!(line("UpdateSlots", json!({"storage": "Tape", "scan": true})), "update slots storage=Tape scan");
        assert_eq!(line("UpdateJob", json!({"jobid": 5, "jobname": "x"})), "update jobid=5 jobname=x");
        assert_eq!(line("UpdateStats", json!({"days": 30})), "update stats days=30");
        assert_eq!(line("Truncate", json!({"pool": "Full", "confirm": true})), "truncate volstatus=Purged pool=Full yes");
        assert_eq!(line("CreatePool", json!({"pool": "Full"})), "create pool=Full");
    }

    #[test]
    fn destructive_lines_need_confirm() {
        for (rpc, req) in [
            ("DeleteVolume", json!({"volume": "A"})),
            ("DeleteJob", json!({"jobid": [1]})),
            ("PruneFiles", json!({"client": "fd"})),
            ("PruneVolume", json!({"volume": "A"})),
            ("PruneDirectory", json!({"directory": "/x/", "client": "fd"})),
            ("PurgeVolume", json!({"volume": ["A"]})),
            ("PurgeQuota", json!({})),
            ("Truncate", json!({})),
            ("Relabel", json!({"storage": "s", "oldvolume": "a", "volume": "b", "confirm": false})),
        ] {
            assert!(err(rpc, req).contains("confirm = true"), "{rpc}");
        }
        assert_eq!(line("DeleteVolume", json!({"volume": "A", "pool": "Full", "confirm": true})), "delete volume=A pool=Full yes");
        assert_eq!(line("DeletePool", json!({"pool": "Old", "confirm": true})), "delete pool=Old");
        assert_eq!(line("DeleteJob", json!({"jobid": [1, 2], "confirm": true})), "delete jobid=1,2");
        assert_eq!(line("DeleteStorage", json!({"storage": "S", "confirm": true})), "delete storage=S");
        assert!(err("DeletePool", json!({"pool": "Old", "jobid": [1], "confirm": true})).contains("does not apply"));
        let p = plan("DeleteJob", json!({"jobid": [1], "confirm": true}).as_object().unwrap()).unwrap().unwrap();
        assert_eq!(p.yes_to, &["Are you sure you want to delete"]);
        assert_eq!(line("PruneJobs", json!({"pool": "Full", "confirm": true})), "prune jobs pool=Full yes");
        assert_eq!(line("PruneStats", json!({"confirm": true})), "prune stats yes");
        assert_eq!(line("PruneVolume", json!({"all": true, "pool": "Full", "confirm": true})), "prune volume all pool=Full yes");
        assert_eq!(line("PruneDirectory", json!({"directory": "/x/", "client": "fd", "recursive": true, "confirm": true})),
                   "prune directory=/x/ client=fd recursive yes");
        assert_eq!(line("PurgeVolume", json!({"volume": ["A", "B"], "confirm": true})), "purge volume=A volume=B");
        assert_eq!(line("PurgeVolume", json!({"volume": ["A"], "action": "truncate", "storage": "File", "confirm": true})),
                   "purge volume=A action=truncate storage=File");
        assert_eq!(line("PurgeFiles", json!({"jobid": 4, "confirm": true})), "purge files jobid=4");
        assert_eq!(line("PurgeJobs", json!({"client": "fd", "jobstatus": "E,f", "confirm": true})), "purge jobs client=fd jobstatus=E,f");
        assert_eq!(line("PurgeQuota", json!({"confirm": true})), "purge quota");
        assert!(err("PurgeFiles", json!({"pool": "x", "jobid": 1, "confirm": true})).contains("does not apply"));
    }

    #[test]
    fn admin_lines() {
        assert_eq!(line("Enable", json!({"job": "j"})), "enable job=j");
        assert_eq!(line("Disable", json!({"schedule": "s"})), "disable schedule=s");
        assert_eq!(line("ExportVolumes", json!({"storage": "T", "srcslots": "1-3"})), "export storage=T srcslots=1-3");
        assert_eq!(line("MoveVolumes", json!({"storage": "T", "srcslots": "1", "dstslots": "2"})), "move storage=T srcslots=1 dstslots=2");
        assert_eq!(line("ConfigureAdd", json!({"resource_type": 1, "name": "c1", "directives": {"address": "h", "password": "p"}})),
                   "configure add client name=c1 address=h password=p");
        // S-38: the lines `configure update|delete` accept
        assert_eq!(line("ConfigureUpdate", json!({"resource_type": 1, "name": "c1", "directives": {"port": "9102", "address": "h"}})),
                   "configure update client=c1 address=h port=9102");
        assert_eq!(line("ConfigureUpdate", json!({"resource_type": 3, "name": "j", "directives": {"newpool": "Full"}})),
                   "configure update job=j newpool=Full");
        assert!(err("ConfigureUpdate", json!({"resource_type": 1, "name": "c1"})).contains("at least one"));
        assert!(err("ConfigureUpdate", json!({"resource_type": 1, "name": "c1", "directives": {"enabled": "no"}})).contains("does not apply"));
        assert!(err("ConfigureUpdate", json!({"resource_type": 8, "name": "p", "directives": {"x": "y"}})).contains("supports"));
        assert_eq!(line("ConfigureDelete", json!({"resource_type": 6, "name": "s1", "yes": true})), "configure delete schedule=s1 yes");
        assert_eq!(line("ConfigureDelete", json!({"resource_type": 8, "name": "p"})), "configure delete pool=p");
        assert!(err("ConfigureDelete", json!({"resource_type": 7, "name": "fs"})).contains("supports"));
        assert_eq!(line("ConfigureExport", json!({"client": "fd"})), "configure export client=fd");
        assert_eq!(line("SetDebug", json!({"level": 100, "trace": true, "dir": true})), "setdebug level=100 trace=1 dir");
        assert_eq!(line("SetDebug", json!({"level": 0, "client": "fd"})), "setdebug level=0 client=fd");
        assert_eq!(line("SetBandwidth", json!({"limit": 1000, "client": "fd"})), "setbandwidth limit=1000 client=fd yes");
        assert_eq!(line("SetBandwidth", json!({"limit": 0, "all": true})), "setbandwidth limit=0 all yes");
        assert!(err("SetBandwidth", json!({"client": "fd"})).contains("limit is required"));
        assert_eq!(line("SetDevice", json!({"storage": "S", "device": "D", "autoselect": "no"})), "setdevice storage=S device=D autoselect=no");
        assert_eq!(line("Trace", json!({"on": true})), "trace on");
        assert_eq!(line("Automount", json!({"off": true})), "automount off");
        assert_eq!(line("UseCatalog", json!({"catalog": "MyCatalog"})), "use catalog=MyCatalog");
        assert_eq!(line("Wait", json!({"jobid": 5})), "wait jobid=5");
        assert_eq!(line("Reload", json!({})), "reload");
        assert_eq!(line("DotSql", json!({"query": "select 1"})), ".sql query=\"select 1\"");
        let q = plan("SqlQuery", json!({"query": "select 1"}).as_object().unwrap()).unwrap().unwrap();
        assert_eq!((q.line(), q.answers.clone()), ("sqlquery".to_string(), vec!["select 1;".to_string(), String::new()]));
        let q = plan("Query", json!({"number": 3, "parameters": ["fd"]}).as_object().unwrap()).unwrap().unwrap();
        assert_eq!(q.answers, vec!["3".to_string(), "fd".to_string()]);
        assert_eq!(line("DotBvfsClearCache", json!({})), ".bvfs_clear_cache yes");
        assert_eq!(line("DotAclSet", json!({"console": "c", "type": "JobAcl", "value": "a,b"})), ".acl set console=c jobacl=a,b");
        assert_eq!(line("DotAclDelete", json!({"console": "c", "type": "job"})), ".acl delete console=c jobacl");
        assert_eq!(line("DotAclCopy", json!({"from": "a", "to": "b"})), ".acl copy from=a to=b");
    }

    /// S-38: the planned lines run through `configure` itself and change
    /// the running configuration (no config path in tests: nothing on disk).
    #[test]
    fn configure_update_delete_lines_are_accepted() {
        use std::sync::{Arc, RwLock};
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nClient { Name = c1; Address = old; Password = pw }\n\
             Pool { Name = Full; Pool Type = Backup }\nSchedule { Name = s1 }\n\
             Job { Name = j; Type = Backup; Client = c1; Pool = Full; Messages = Standard }"
        ).unwrap();
        let config = Arc::new(RwLock::new(config));
        let ctx = crate::command_executor::ExecContext { session_id: 0, config: Some(config.clone()),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let run = |rpc: &str, request: Value| {
            let p = plan(rpc, request.as_object().unwrap()).unwrap().unwrap();
            let result = crate::commands::configure::cmd_configure(&p.ast, &ctx);
            format!("{}{}", result.text_output, result.error_message)
        };
        assert_eq!(run("ConfigureUpdate", json!({"resource_type": 1, "name": "c1", "directives": {"address": "new", "port": "9112"}})),
                   "Client 'c1' updated: address=new, port=9112\n");
        assert_eq!(config.read().unwrap().find_client("c1").unwrap().address, "new");
        assert_eq!(run("ConfigureUpdate", json!({"resource_type": 3, "name": "j", "directives": {"newpool": "Full"}})),
                   "Job 'j' updated: pool=Full\n");
        assert_eq!(run("ConfigureDelete", json!({"resource_type": 6, "name": "s1", "yes": true})), "Deleted Schedule 's1'\n");
        assert!(config.read().unwrap().schedules.iter().all(|s| s.name != "s1"));
        assert_eq!(run("ConfigureDelete", json!({"resource_type": 1, "name": "nosuch"})), "Client 'nosuch' not found\n");
    }

    /// S-39: limit=0 survives the wire (optional field) and becomes
    /// `limit=0` in the line.
    #[test]
    fn setbandwidth_limit_zero_on_the_wire() {
        use onesimus_proto_rs::onesimus::director::api::SetBandwidthLimitCommand;
        use onesimus_proto_rs::typed;
        use prost::Message;
        let (input, _) = typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, "SetBandwidth").unwrap();
        assert!(input.ends_with("SetBandwidthLimitCommand"), "{input}");
        let encoded = typed::json_to_message(&input, &json!({"limit": 0, "client": "fd"}));
        assert!(encoded.unmapped.is_empty());
        let request = SetBandwidthLimitCommand::decode(encoded.bytes.as_slice()).unwrap();
        assert_eq!(request.limit, Some(0));
        let Value::Object(m) = typed::message_to_json(&input, &request.encode_to_vec()) else { panic!() };
        assert_eq!(plan("SetBandwidth", &m).unwrap().unwrap().line(), "setbandwidth limit=0 client=fd yes");
        // absent limit stays an error
        let request = SetBandwidthLimitCommand { client: "fd".into(), ..Default::default() };
        let Value::Object(m) = typed::message_to_json(&input, &request.encode_to_vec()) else { panic!() };
        assert!(plan("SetBandwidth", &m).unwrap().unwrap_err().contains("limit is required"));
    }

    #[test]
    fn every_write_rpc_has_a_plan() {
        use crate::grpc::coverage::{LOGOUT, NOT_APPLICABLE};
        for (rpc, _) in crate::grpc::api::DEV_IMPLEMENTED {
            let special = rpc.starts_with("RestoreTree") || *rpc == "StartRestoreSession"
                || LOGOUT.contains(rpc) || NOT_APPLICABLE.iter().any(|(n, _)| n == rpc);
            assert_eq!(plan(rpc, &Map::new()).is_some(), !special, "{rpc}");
        }
    }

    #[test]
    fn replies_parse() {
        assert_eq!(queued_jobids("Using Catalog\nJob queued. JobId=631\nJob queued. JobId=632\n"), vec![631, 632]);
        assert_eq!(question("text\nPurge (yes/no)? "), "Purge (yes/no)?");
        let entries = tree_entries("*etc/\nhosts\n+usr/\n");
        assert_eq!(entries[0], ("etc/".to_string(), true, true));
        assert_eq!(entries[1], ("hosts".to_string(), false, false));
        assert_eq!(entries[2], ("usr/".to_string(), true, false));
        assert_eq!(numbers("1,234 total files; 12 marked to be restored; 5,000 bytes."), vec![1234, 12, 5000]);
        assert_eq!(cwd_of("cwd is: /etc/\n").as_deref(), Some("/etc/"));
        let mut console = AnswerConsole::new(&Plan::new(Line::new("purge")).yes_to(&["Purge (yes/no)?"]));
        console.send("This is dangerous.\nPurge (yes/no)? ");
        assert_eq!(console.recv().as_deref(), Some("yes"));
        console.send("Select something: ");
        assert_eq!(console.recv(), None);
        assert_eq!(console.transcript.lock().unwrap().question.as_deref(), Some("Select something:"));
    }
}
