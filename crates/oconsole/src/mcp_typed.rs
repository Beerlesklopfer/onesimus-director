//! Typed DirectorApi route of the MCP tools (S-36, C-087).
//!
//! When oconsole is connected via gRPC, a tool first calls the typed RPC of
//! `DirectorApi` (proto/director_api.proto): the checked tool arguments
//! become the request message, the typed response becomes the tool result
//! (`structuredContent` = the response as JSON with the proto field names).
//! Requests and responses are encoded with the descriptors of
//! `bareos_proto_rs::typed`, so one generic call serves every RPC.
//!
//! Order of the routes (see [`call`]):
//! 1. gRPC, typed RPC implemented → typed result;
//! 2. gRPC, the Director answers UNIMPLEMENTED (production Director: write
//!    RPCs, S-35) or an argument has no request field → the tool's console
//!    line via `ExecuteCommand` on the same session (the caller's fallback);
//! 3. BNET (e.g. an original bareos-dir) → the console line (fallback).
//!
//! The tool's own argument checks always run first: the typed request is
//! only built from arguments the console-line builder accepted.

use serde_json::{Map, Value, json};

use bareos_proto_rs::typed;

use crate::mcp::{CommandRunner, TypedError};

/// Outcome of the typed route.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The tool result (typed response or a gRPC error).
    Done(Value),
    /// Use the console line (BNET, or ExecuteCommand over gRPC).
    Fallback,
}

/// (input type, output type) of a DirectorApi RPC.
pub fn rpc_types(rpc: &str) -> Option<(String, String)> {
    typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, rpc)
}

/// All RPC names of DirectorApi, in declaration order.
#[cfg_attr(not(test), allow(dead_code))]
pub fn rpc_names() -> Vec<String> {
    typed::registry().rpc_names(typed::DIRECTOR_API_SERVICE)
}

/// Encode `request` as the input message of `rpc`; Err if the RPC is
/// unknown or a key has no field (then the console line is used).
pub fn encode(rpc: &str, request: &Value) -> Result<Vec<u8>, String> {
    let (input, _) = rpc_types(rpc).ok_or_else(|| format!("unknown RPC {rpc}"))?;
    let encoded = typed::json_to_message(&input, request);
    if encoded.unmapped.is_empty() {
        Ok(encoded.bytes)
    } else {
        Err(format!("{rpc}: no request field for {}", encoded.unmapped.join(", ")))
    }
}

/// Decode a response of `rpc` to JSON.
pub fn decode(rpc: &str, bytes: &[u8]) -> Value {
    match rpc_types(rpc) {
        Some((_, output)) => typed::message_to_json(&output, bytes),
        None => Value::Null,
    }
}

fn is_action(rpc: &str) -> bool {
    rpc_types(rpc).is_some_and(|(_, out)| out.ends_with(".ActionResponse"))
}

/// Run the typed RPC of a tool if the connection offers the typed API.
pub fn call<R: CommandRunner + ?Sized>(runner: &mut R, rpc: &str, request: &Value) -> Outcome {
    let Ok(bytes) = encode(rpc, request) else { return Outcome::Fallback };
    match runner.typed_call(rpc, bytes) {
        None | Some(Err(TypedError::Unimplemented)) => Outcome::Fallback,
        Some(Ok(reply)) => Outcome::Done(result(rpc, decode(rpc, &reply))),
        Some(Err(TypedError::Status(code, message))) => Outcome::Done(json!({
            "content": [{ "type": "text", "text": format!("Director ({rpc}, gRPC {code}): {}", message.trim_end()) }],
            "isError": true,
        })),
    }
}

/// Tool result of a typed response.
pub fn result(rpc: &str, response: Value) -> Value {
    if is_action(rpc) {
        let ok = response.get("success").and_then(Value::as_bool).unwrap_or(false);
        let mut text = String::new();
        if let Some(command) = response.get("command").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            text.push_str(&format!("> {command}\n"));
        }
        text.push_str(response.get("text").and_then(Value::as_str).unwrap_or(""));
        for e in response.get("errors").and_then(Value::as_array).into_iter().flatten() {
            if let Some(e) = e.as_str().filter(|e| !text.contains(*e)) {
                text.push_str(&format!("\n{e}"));
            }
        }
        if let Some(q) = response.get("question").and_then(Value::as_str).filter(|q| !q.is_empty()) {
            text.push_str(&format!(
                "\n[oconsole] The Director asked '{q}', which this tool does not answer; it was cancelled."
            ));
        }
        if text.trim().is_empty() {
            text = "(no output)".into();
        }
        return json!({ "content": [{ "type": "text", "text": text }], "isError": !ok,
                       "structuredContent": response });
    }
    let structured = if response.is_object() { response.clone() } else { json!({ "value": response }) };
    json!({ "content": [{ "type": "text", "text": response.to_string() }], "isError": false,
            "structuredContent": structured })
}

// ---------------------------------------------------------------------------
// Tool arguments -> (RPC, request)

struct Req<'a> {
    args: &'a Map<String, Value>,
    out: Map<String, Value>,
}

impl<'a> Req<'a> {
    fn get(&self, key: &str) -> Option<&'a Value> {
        self.args.get(key).filter(|v| !v.is_null())
    }
    fn has(&self, key: &str) -> bool {
        self.get(key).is_some_and(|v| v != &Value::Bool(false) && v != &json!(""))
    }
    fn str(&self, key: &str) -> Option<&'a str> {
        self.get(key).and_then(Value::as_str)
    }
    /// Copy the given keys unchanged.
    fn copy(mut self, keys: &[&str]) -> Self {
        for key in keys {
            if let Some(v) = self.get(key) {
                if v != &json!("") {
                    self.out.insert(key.to_string(), v.clone());
                }
            }
        }
        self
    }
    fn rename(mut self, from: &str, to: &str) -> Self {
        if let Some(v) = self.get(from) {
            self.out.insert(to.into(), v.clone());
        }
        self
    }
    /// One value or a list, as one comma-separated string.
    fn joined(mut self, from: &str, to: &str) -> Self {
        let text = match self.get(from) {
            Some(Value::Array(items)) if !items.is_empty() => Some(items.iter()
                .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
                .collect::<Vec<_>>().join(",")),
            Some(Value::Array(_)) | None => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(v) => Some(v.to_string()),
        };
        if let Some(text) = text {
            self.out.insert(to.into(), json!(text));
        }
        self
    }
    /// One value or a list, as a list (repeated field).
    fn list(mut self, from: &str, to: &str) -> Self {
        match self.get(from) {
            Some(Value::Array(items)) => { self.out.insert(to.into(), json!(items)); }
            Some(v) => { self.out.insert(to.into(), json!([v])); }
            None => {}
        }
        self
    }
    /// A number argument for a string field (`pathid`).
    fn text_of(mut self, key: &str) -> Self {
        if let Some(v) = self.get(key) {
            let text = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
            self.out.insert(key.into(), json!(text));
        }
        self
    }
    /// A boolean argument for a yes/no string field.
    fn yes_no(mut self, key: &str) -> Self {
        if let Some(v) = self.get(key).and_then(Value::as_bool) {
            self.out.insert(key.into(), json!(if v { "yes" } else { "no" }));
        }
        self
    }
    fn llist(self) -> Self {
        self.rename("long", "llist")
    }
    fn set(mut self, key: &str, value: Value) -> Self {
        self.out.insert(key.into(), value);
        self
    }
    /// Keys the RPC has no field for: makes the encoder fall back.
    fn keep_unmapped(mut self, keys: &[&str]) -> Self {
        for key in keys {
            if self.has(key) {
                self.out.insert(format!("__{key}"), json!(true));
            }
        }
        self
    }
    fn done(self, rpc: &'static str) -> Option<(&'static str, Value)> {
        Some((rpc, Value::Object(self.out)))
    }
}

/// The typed RPC and request of a tool call (arguments already checked by
/// the tool); `None`: the tool has no typed route (`query`, `execute`, or
/// `format: "text"`).
pub fn request(tool: &str, arguments: &Value) -> Option<(&'static str, Value)> {
    let empty = Map::new();
    let args = arguments.as_object().unwrap_or(&empty);
    if args.get("format").and_then(Value::as_str) == Some("text") {
        return None;
    }
    let r = Req { args, out: Map::new() };
    match tool {
        // ---- list / llist
        "list_jobs" => r.copy(&["job", "client", "volume", "pool", "days", "hours", "last", "count", "limit", "offset"])
            .joined("jobstatus", "jobstatus").joined("jobtype", "jobtype").joined("level", "joblevel")
            .keep_unmapped(&["order"]).llist().done("ListJobs"),
        "list_job" => r.copy(&["jobid", "ujobid"]).llist().done("GetJob"),
        "list_joblog" => r.copy(&["jobid", "ujobid", "count"]).keep_unmapped(&["limit", "offset"]).done("ListJobLog"),
        "list_jobmedia" => r.copy(&["jobid", "ujobid"]).llist().done("ListJobMedia"),
        "list_files" => r.copy(&["jobid", "ujobid", "limit", "offset"]).done("ListFiles"),
        "list_basefiles" => r.copy(&["jobid", "ujobid"]).done("ListBaseFiles"),
        "list_jobstatistics" => r.copy(&["jobid"]).done("ListJobStatistics"),
        "list_log" => r.copy(&["client", "reverse", "limit", "offset"]).done("ListLog"),
        "list_volumes" => {
            if r.has("volume") || r.has("volumeid") || r.has("mediaid") {
                let media = r.has("media");
                let r = if media { r.rename("volume", "media") } else { r.copy(&["volume"]) };
                r.copy(&["volumeid", "mediaid"]).keep_unmapped(&["count", "limit", "offset", "all"]).llist().done("GetVolume")
            } else {
                let rpc = if r.has("media") { "ListMedia" } else { "ListVolumes" };
                r.copy(&["pool", "jobid", "all", "count", "limit", "offset"]).llist().done(rpc)
            }
        }
        "list_pools" => r.copy(&["pool", "poolid"]).llist().done("ListPools"),
        "list_clients" => r.copy(&["client"]).llist().done("ListClients"),
        "list_storages" => r.llist().done("ListStorages"),
        "list_filesets" => r.copy(&["jobid", "filesetid"]).llist().done("ListFilesets"),
        "list_jobtotals" => r.done("ListJobTotals"),
        "list_copies" => r.joined("jobids", "jobid").llist().done("ListCopies"),
        "list_backups" => r.copy(&["client", "fileset", "order", "days", "hours", "limit", "offset"])
            .joined("jobstatus", "jobstatus").joined("level", "level").llist().done("ListBackups"),
        "list_nextvolume" => r.copy(&["job", "days"]).done("ListNextVolume"),
        // ---- show
        "show" => {
            let resource = r.str("resource")?;
            match resource {
                "all" => r.copy(&["verbose"]).done("ShowAll"),
                "disabled" => {
                    let kind = r.str("disabled_type");
                    let r = match kind { Some(k) => r.set(k, json!(true)), None => r };
                    r.keep_unmapped(&["verbose"]).done("ShowDisabled")
                }
                kind => {
                    let (rpc, field) = match kind {
                        "director" => ("ShowDirectors", "director"),
                        "client" => ("ShowClients", "client"),
                        "job" => ("ShowJobs", "job"),
                        "jobdefs" => ("ShowJobdefs", "jobdefs"),
                        "storage" => ("ShowStorages", "storage"),
                        "catalog" => ("ShowCatalogs", "catalog"),
                        "schedule" => ("ShowSchedules", "schedule"),
                        "fileset" => ("ShowFilesets", "fileset"),
                        "pool" => ("ShowPools", "pool"),
                        "messages" => ("ShowMessages", "message"),
                        "counter" => ("ShowCounters", "counter"),
                        "profile" => ("ShowProfiles", "profile"),
                        "console" => ("ShowConsoles", "console"),
                        "user" => ("ShowUsers", "user"),
                        _ => return None,
                    };
                    r.rename("name", field).keep_unmapped(&["verbose"]).done(rpc)
                }
            }
        }
        // ---- status
        "status_director" => r.copy(&["days"]).done("GetDirectorStatus"),
        "status_all" => r.done("GetStatusAll"),
        "status_client" => r.copy(&["client"]).done("GetClientStatus"),
        "status_storage" => r.copy(&["storage", "slots"]).done("GetStorageStatus"),
        "status_job" => r.copy(&["job"]).done("GetJobStatus"),
        "status_scheduler" => {
            let negative = r.get("days").and_then(Value::as_i64).is_some_and(|d| d < 0);
            let r = if negative { r.set("__days", json!(true)) } else { r.copy(&["days"]) };
            r.copy(&["schedule", "client", "job"]).keep_unmapped(&["days_from", "days_to"]).done("GetSchedulerStatus")
        }
        "status_subscriptions" => r.copy(&["clients", "plugins", "all", "anonymize", "client"]).done("GetSubscriptions"),
        "status_configuration" => r.done("GetConfigurationStatus"),
        "dot_status" => {
            let keyword = r.str("keyword").map(str::to_string);
            let (rpc, r) = match r.str("target")? {
                "dir" => ("DotStatusDirector", r),
                "client" => ("DotStatusClient", r.rename("name", "client")),
                _ => ("DotStatusStorage", r.rename("name", "storage")),
            };
            match keyword { Some(k) => r.set(&k, json!(true)).done(rpc), None => r.done(rpc) }
        }
        // ---- info
        "messages" => r.done("GetMessages"),
        "dot_messages" => r.done("DotMessages"),
        "version" => r.done("GetVersion"),
        "time" => r.done("GetTime"),
        "whoami" => r.done("Whoami"),
        "memory" => r.done("GetMemory"),
        "dot_noop" => r.done("Noop"),
        "help" => r.copy(&["command"]).done("Help"),
        "dot_help" => r.copy(&["all", "item", "full"]).done("DotHelp"),
        "resolve" => r.copy(&["client", "storage", "hostname"]).done("Resolve"),
        "var_expand" => r.copy(&["text"]).done("ExpandVariables"),
        "estimate" => {
            let accurate = r.get("accurate").and_then(Value::as_bool);
            let r = r.copy(&["job", "client", "fileset", "level"]).keep_unmapped(&["listing"]);
            match accurate { Some(a) => r.set("accurate", json!(if a { "yes" } else { "no" })), None => r }.done("Estimate")
        }
        // ---- dot listings
        "dot_jobs" => r.copy(&["type"]).keep_unmapped(&["state"]).done("DotJobs"),
        "dot_clients" => r.keep_unmapped(&["state"]).done("DotClients"),
        "dot_pools" => r.copy(&["type"]).done("DotPools"),
        "dot_storages" => r.keep_unmapped(&["state"]).done("DotStorages"),
        "dot_filesets" => r.done("DotFilesets"),
        "dot_schedule" => r.keep_unmapped(&["state"]).done("DotSchedule"),
        "dot_levels" => r.keep_unmapped(&["jobtype"]).done("DotLevels"),
        "dot_types" => r.done("DotTypes"),
        "dot_msgs" => r.done("DotMsgs"),
        "dot_jobstatus" => r.keep_unmapped(&["code"]).done("DotJobstatus"),
        "dot_defaults" => {
            let kind = r.str("resource").unwrap_or("job").to_string();
            r.rename("name", &kind).done("DotDefaults")
        }
        "dot_list" => {
            let rpc = match r.str("kind")? {
                "catalogs" => "DotCatalogs",
                "consoles" => "DotConsoles",
                "users" => "DotUsers",
                "jobdefs" => "DotJobdefs",
                "profiles" => "DotProfiles",
                "locations" => "DotLocations",
                "media" => "DotMedia",
                "mediatypes" => "DotMediatypes",
                "volstatus" => "DotVolstatus",
                "actiononpurge" => "DotActiononpurge",
                _ => return None,
            };
            r.done(rpc)
        }
        "dot_authorized" => r.copy(&["job", "client", "storage", "schedule", "pool", "cmd", "fileset", "catalog"]).done("DotAuthorized"),
        "sql_meta" => {
            let rpc = match r.str("kind")? { "tables" => "DotSqlTables", "count" => "DotSqlCount", _ => "DotSqlSchema" };
            r.copy(&["table", "catalog"]).done(rpc)
        }
        "acl_list" => r.copy(&["console"]).done("DotAclList"),
        "profile_list" => r.done("DotProfileList"),
        "bvfs_get_jobids" => r.copy(&["jobid", "ujobid"]).keep_unmapped(&["all"]).done("DotBvfsGetJobids"),
        "bvfs_lsdirs" | "bvfs_lsfiles" => {
            let rpc = if tool == "bvfs_lsdirs" { "DotBvfsLsdirs" } else { "DotBvfsLsfiles" };
            // the request has one jobid; a chain goes through the console line
            let chain = matches!(r.get("jobids"), Some(Value::Array(a)) if a.len() > 1);
            let r = if chain { r.set("__jobids", json!(true)) } else { r.joined("jobids", "jobid") };
            let r = match r.out.get("jobid").and_then(Value::as_str).and_then(|s| s.parse::<i64>().ok()) {
                Some(id) => r.set("jobid", json!(id)),
                None => r,
            };
            r.copy(&["path", "limit", "offset"]).text_of("pathid").done(rpc)
        }
        "bvfs_versions" => r.copy(&["client", "filename"]).text_of("pathid").keep_unmapped(&["jobid", "copies", "versions"]).done("DotBvfsVersions"),
        // ---- OVFS
        "ovfs_status" => r.done("DotOvfsStatus"),
        "ovfs_latest" => r.copy(&["client", "fileset"]).done("DotOvfsLatest"),
        "ovfs_tree" => r.rename("jobids", "jobid").copy(&["client", "fileset", "root", "depth", "max_entries"]).done("DotOvfsTree"),
        "ovfs_ls" => r.rename("jobids", "jobid").copy(&["client", "fileset", "path"]).done("DotOvfsLs"),
        "ovfs_versions" => r.copy(&["client", "path", "name", "copies"]).done("DotOvfsVersions"),
        // ---- write: jobs
        "run_job" => r.copy(&["job", "level", "client", "fileset", "pool", "nextpool", "storage", "when", "since",
            "priority", "jobid", "spooldata", "accurate", "comment"]).done("Run"),
        "rerun_job" => r.list("jobids", "jobid").copy(&["since_jobid", "until_jobid", "days", "hours"]).done("Rerun"),
        "cancel_job" => r.copy(&["jobid", "job", "ujobid", "storage", "state", "all"]).set("yes", json!(true)).done("Cancel"),
        "wait_job" => r.copy(&["jobid", "ujobid", "timeout"]).rename("job", "jobname").keep_unmapped(&["mount"]).done("Wait"),
        "restore" => r.list("jobids", "jobid").list("files", "file").list("directories", "directory")
            .copy(&["client", "fileset", "pool", "before", "all", "fileregex", "where", "regexwhere", "strip_prefix",
                "add_prefix", "add_suffix", "replace", "restoreclient", "restorejob", "storage", "comment"]).done("Restore"),
        // ---- write: volumes and devices
        "label_volume" => r.copy(&["storage", "volume", "pool", "slot", "drive", "encrypt"]).done("Label"),
        "relabel_volume" => r.copy(&["storage", "oldvolume", "volume", "pool", "slot", "drive", "encrypt", "confirm"]).done("Relabel"),
        "mount_storage" => r.copy(&["storage", "slot", "drive", "jobid", "job", "ujobid"]).done("Mount"),
        "unmount_storage" => r.copy(&["storage", "drive", "jobid", "job", "ujobid"]).keep_unmapped(&["alldrives"]).done("Unmount"),
        "release_storage" => r.copy(&["storage", "drive"]).keep_unmapped(&["jobid", "job", "ujobid", "alldrives"]).done("Release"),
        "update_volume" => r.copy(&["volume", "allfrompool", "fromallpools", "volstatus", "volretention", "voluse",
            "maxvolbytes", "maxvoljobs", "maxvolfiles", "slot", "recycle", "inchanger", "enabled", "actiononpurge",
            "pool", "recyclepool", "storage", "frompool"]).done("UpdateVolume"),
        "update_pool" => r.copy(&["pool"]).done("UpdatePool"),
        "update_slots" => r.copy(&["storage", "drive", "scan"]).done("UpdateSlots"),
        "update_job" => r.copy(&["jobid", "jobname", "starttime", "client", "filesetid", "jobtype"]).done("UpdateJob"),
        "update_stats" => r.copy(&["days"]).done("UpdateStats"),
        "add_volumes" => r.copy(&["pool", "storage", "jobid"])
            .keep_unmapped(&["count", "name", "start", "slot", "inchanger"]).done("AddVolumes"),
        "create_pool" => r.copy(&["pool"]).done("CreatePool"),
        "export_volumes" => r.copy(&["storage", "srcslots", "dstslots", "volume"]).keep_unmapped(&["scan"]).done("ExportVolumes"),
        "import_volumes" => r.copy(&["storage", "srcslots", "dstslots", "volume"]).keep_unmapped(&["scan"]).done("ImportVolumes"),
        "move_volumes" => r.copy(&["storage", "srcslots", "dstslots"]).done("MoveVolumes"),
        "truncate" => r.copy(&["storage", "pool", "volume", "drive", "confirm"]).done("Truncate"),
        // ---- write: catalog
        "prune" => {
            let kind = r.str("kind")?;
            match kind {
                "files" => r.copy(&["client", "pool", "confirm"]).done("PruneFiles"),
                "jobs" => r.copy(&["client", "pool", "confirm"]).done("PruneJobs"),
                "stats" => r.copy(&["confirm"]).done("PruneStats"),
                "volume" => r.copy(&["volume", "pool", "confirm"]).rename("all_volumes", "all").set("yes", json!(true)).done("PruneVolume"),
                _ => r.copy(&["directory", "client", "recursive", "confirm"]).set("yes", json!(true)).done("PruneDirectory"),
            }
        }
        "purge" => {
            let rpc = match r.str("kind")? { "files" => "PurgeFiles", "jobs" => "PurgeJobs", "volume" => "PurgeVolume", _ => "PurgeQuota" };
            r.list("volume", "volume").copy(&["jobid", "job", "client", "pool", "jobstatus", "action", "storage", "allpools", "drive", "confirm"]).done(rpc)
        }
        "delete" => {
            let rpc = match r.str("kind")? { "volume" => "DeleteVolume", "pool" => "DeletePool", "jobid" => "DeleteJob", _ => "DeleteStorage" };
            // DeleteStorage sends `delete storage storage=<n>`, which asks for the
            // name again: console line (see configure_update).
            let r = if rpc == "DeleteStorage" { r.set("__console_line", json!(true)) } else { r };
            r.list("jobids", "jobid").copy(&["volume", "pool", "storage", "confirm"]).done(rpc)
        }
        "dot_sql" => r.copy(&["query"]).done("DotSql"),
        "sqlquery" => r.copy(&["query"]).done("SqlQuery"),
        "run_query" => r.copy(&["number"]).list("parameters", "parameters").done("Query"),
        "bvfs_update" => {
            let one = match r.get("jobids") { Some(Value::Array(a)) if a.len() == 1 => a[0].as_i64(), Some(Value::Array(_)) => None, Some(v) => v.as_i64(), None => Some(0) };
            match one {
                Some(0) => r.done("DotBvfsUpdate"),
                Some(id) => r.set("jobid", json!(id)).done("DotBvfsUpdate"),
                None => r.set("__jobids", json!(true)).done("DotBvfsUpdate"),
            }
        }
        "bvfs_restore" => r.copy(&["path", "jobid", "fileid", "dirid", "hardlink"]).done("DotBvfsRestore"),
        "bvfs_cleanup" => r.copy(&["path"]).done("DotBvfsCleanup"),
        "bvfs_clear_cache" => r.done("DotBvfsClearCache"),
        // ---- write: configuration and daemons
        "enable" | "disable" => {
            let kind = r.str("resource")?.to_string();
            r.rename("name", &kind).done(if tool == "enable" { "Enable" } else { "Disable" })
        }
        "configure_add" | "configure_update" | "configure_delete" => {
            let rpc = match tool { "configure_add" => "ConfigureAdd", "configure_update" => "ConfigureUpdate", _ => "ConfigureDelete" };
            let mut r = r.rename("resource", "resource_type").copy(&["name"]);
            if tool == "configure_delete" {
                r = r.set("yes", json!(true));
            }
            if tool != "configure_add" {
                // The Director's ConfigureUpdate/ConfigureDelete build
                // `configure update|delete <type> name=<n>`, which its own
                // configure command rejects (it wants <type>=<name>): use the
                // console line until that is fixed.
                r = r.set("__console_line", json!(true));
            }
            if let Some(Value::Object(directives)) = r.get("directives") {
                let mut map = Map::new();
                for (key, value) in directives {
                    let text = match value {
                        Value::String(s) => json!(s),
                        Value::Number(n) => json!(n.to_string()),
                        Value::Bool(b) => json!(if *b { "yes" } else { "no" }),
                        _ => return r.set("__directives", json!(true)).done(rpc),
                    };
                    map.insert(key.clone(), text);
                }
                r = r.set("directives", Value::Object(map));
            }
            r.done(rpc)
        }
        "configure_export" => r.copy(&["client"]).done("ConfigureExport"),
        "reload" => r.done("Reload"),
        "setdebug" => {
            let target = r.str("target")?.to_string();
            let r = r.copy(&["level", "trace", "timestamp", "hangup"]);
            match target.as_str() {
                "client" | "storage" => r.rename("name", &target).done("SetDebug"),
                t => r.set(t, json!(true)).done("SetDebug"),
            }
        }
        "setbandwidth" => {
            // proto3 cannot send limit=0 (it is the absent default): console line
            let zero = r.get("limit").and_then(Value::as_i64) == Some(0);
            let r = if zero { r.set("__limit", json!(0)) } else { r };
            r.copy(&["limit", "client", "storage", "jobid", "job", "ujobid", "state"]).keep_unmapped(&["all"]).done("SetBandwidth")
        }
        "setdevice" => r.copy(&["storage", "device"]).yes_no("autoselect").done("SetDevice"),
        "session_option" => {
            let rpc = match r.str("option")? { "autodisplay" => "Autodisplay", "automount" => "Automount", _ => "Trace" };
            let on = r.get("on").and_then(Value::as_bool).unwrap_or(false);
            r.set(if on { "on" } else { "off" }, json!(true)).done(rpc)
        }
        "use_catalog" => r.copy(&["catalog"]).done("UseCatalog"),
        "acl_modify" => {
            match r.str("action")? {
                "set" => r.copy(&["console", "type", "value"]).done("DotAclSet"),
                "delete" => r.copy(&["console", "type", "value"]).done("DotAclDelete"),
                "clear" => r.copy(&["console"]).done("DotAclClear"),
                _ => r.copy(&["from", "to"]).done("DotAclCopy"),
            }
        }
        "profile_modify" => {
            match r.str("action")? {
                "create" => r.copy(&["name"]).done("DotProfileCreate"),
                "set" => r.copy(&["name", "type", "value"]).done("DotProfileSet"),
                "delete" => r.copy(&["name", "type", "value"]).done("DotProfileDelete"),
                "assign" => r.copy(&["console", "profile"]).done("DotProfileAssign"),
                _ => r.copy(&["console", "profile"]).done("DotProfileUnassign"),
            }
        }
        // ---- restore file selection (mcp_tree.rs)
        "restore_tree_start" => r.list("jobids", "jobid").copy(&["client", "fileset", "pool", "before", "where", "regexwhere",
            "strip_prefix", "add_prefix", "add_suffix", "replace", "restoreclient", "restorejob", "storage", "comment"])
            .done("StartRestoreSession"),
        "restore_tree_cd" => r.copy(&["session_id", "path"]).done("RestoreTreeCd"),
        "restore_tree_ls" => {
            let rpc = match r.str("mode") { Some("dirs") => "RestoreTreeLsDir", Some("marked") => "RestoreTreeLsMark", _ => "RestoreTreeLs" };
            r.copy(&["session_id"]).rename("pattern", "path").done(rpc)
        }
        "restore_tree_pwd" => r.copy(&["session_id"]).done("RestoreTreePwd"),
        "restore_tree_mark" | "restore_tree_unmark" => {
            let dirs = r.has("dirs_only");
            let rpc = match (tool, dirs) {
                ("restore_tree_mark", false) => "RestoreTreeMark",
                ("restore_tree_mark", true) => "RestoreTreeMarkDir",
                (_, false) => "RestoreTreeUnmark",
                _ => "RestoreTreeUnmarkDir",
            };
            r.copy(&["session_id"]).rename("pattern", "path").done(rpc)
        }
        "restore_tree_count" => r.copy(&["session_id"]).done("RestoreTreeCount"),
        "restore_tree_find" => r.copy(&["session_id"]).rename("pattern", "path").done("RestoreTreeFind"),
        "restore_tree_estimate" => r.copy(&["session_id"]).done("RestoreTreeEstimate"),
        "restore_tree_help" => r.copy(&["session_id"]).done("RestoreTreeHelp"),
        "restore_tree_done" => r.copy(&["session_id"]).done("RestoreTreeDone"),
        "restore_tree_abort" => r.copy(&["session_id"]).done("RestoreTreeAbort"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_rejects_unknown_fields_and_rpcs() {
        assert!(encode("ListJobs", &json!({"client": "fd", "limit": 2, "llist": true})).is_ok());
        assert!(encode("ListJobs", &json!({"order": "asc"})).is_err());
        assert!(encode("NoSuchRpc", &json!({})).is_err());
    }

    #[test]
    fn requests_map_argument_names() {
        let (rpc, req) = request("list_jobs", &json!({"client":"fd","jobstatus":["T","W"],"level":"F","long":true,"limit":3})).unwrap();
        assert_eq!(rpc, "ListJobs");
        assert_eq!(req, json!({"client":"fd","jobstatus":"T,W","joblevel":"F","llist":true,"limit":3}));
        assert_eq!(request("show", &json!({"resource":"messages","name":"Daemon"})).unwrap(),
            ("ShowMessages", json!({"message":"Daemon"})));
        assert_eq!(request("restore", &json!({"client":"fd","jobids":[1,2],"files":["/etc/hosts"]})).unwrap(),
            ("Restore", json!({"client":"fd","jobid":[1,2],"file":["/etc/hosts"]})));
        assert_eq!(request("setdebug", &json!({"level":1,"target":"dir"})).unwrap(), ("SetDebug", json!({"level":1,"dir":true})));
        assert!(request("version", &json!({"format":"text"})).is_none());
        assert!(request("query", &json!({"command":"list jobs"})).is_none());
        // arguments without a request field force the console line
        let (rpc, req) = request("list_jobs", &json!({"order":"desc"})).unwrap();
        assert!(encode(rpc, &req).is_err());
    }

    /// Fake gRPC Director: canned typed replies, or a status.
    struct Fake { reply: Result<Value, TypedError>, calls: Vec<(String, Value)> }
    impl CommandRunner for Fake {
        fn run(&mut self, command: &str) -> anyhow::Result<String> { Ok(format!("ran {command}")) }
        fn is_grpc(&self) -> bool { true }
        fn typed_call(&mut self, rpc: &str, request: Vec<u8>) -> Option<Result<Vec<u8>, TypedError>> {
            let (input, output) = rpc_types(rpc).unwrap();
            self.calls.push((rpc.into(), typed::message_to_json(&input, &request)));
            Some(self.reply.clone().map(|v| typed::json_to_message(&output, &v).bytes))
        }
    }

    #[test]
    fn typed_results_unimplemented_and_status() {
        let mut fake = Fake { reply: Ok(json!({"version":{"version":"25.1.2"}})), calls: vec![] };
        let Outcome::Done(r) = call(&mut fake, "GetVersion", &json!({})) else { panic!() };
        assert_eq!(r["structuredContent"]["version"]["version"], "25.1.2");
        assert_eq!(r["isError"], false);
        let mut fake = Fake { reply: Ok(json!({"success":false,"text":"no","command":"run job=x yes","question":"Select"})), calls: vec![] };
        let Outcome::Done(r) = call(&mut fake, "Run", &json!({"job":"x"})) else { panic!() };
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("> run job=x yes\nno"));
        assert_eq!(fake.calls, vec![("Run".to_string(), json!({"job":"x"}))]);
        let mut fake = Fake { reply: Err(TypedError::Unimplemented), calls: vec![] };
        assert_eq!(call(&mut fake, "Run", &json!({"job":"x"})), Outcome::Fallback);
        let mut fake = Fake { reply: Err(TypedError::Status("FailedPrecondition".into(), "boom".into())), calls: vec![] };
        let Outcome::Done(r) = call(&mut fake, "Run", &json!({"job":"x"})) else { panic!() };
        assert!(r["content"][0]["text"].as_str().unwrap().contains("boom"));
        // unmapped key: no call at all
        let mut fake = Fake { reply: Ok(json!({})), calls: vec![] };
        assert_eq!(call(&mut fake, "ListJobs", &json!({"__order": true})), Outcome::Fallback);
        assert!(fake.calls.is_empty());
    }
}
