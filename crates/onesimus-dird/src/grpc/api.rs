//! DirectorApi (proto/director_api.proto): typed RPCs on top of the console
//! commands.
//!
//! A read RPC is a console command line built from the typed request (head
//! words plus the request fields as `key=value` / bare keywords). It runs
//! through the same command queue, registry and console ACLs as
//! `ExecuteCommand`/BNET (no second SQL path), in API 2 JSON mode; the JSON
//! result is converted into the typed response by the descriptor-driven
//! codec in `bareos_proto_rs::typed`. Commands without an API 2 result run
//! in text mode and fill the `text` field.
//!
//! RPCs not listed in [`read_rpcs`] (write, session and restore-tree RPCs)
//! are answered with UNIMPLEMENTED by the generated default stubs until
//! S-35.

use std::sync::{Arc, OnceLock};

use prost::Message;
use serde_json::{Map, Value};
use tonic::{Request, Response, Status};

use bareos_proto_rs::bareos::director::api::{self as pb, director_api_server::DirectorApi};
use bareos_proto_rs::bareos::director::commands as cmd;
use bareos_proto_rs::typed;

use super::auth::AuthState;
use super::service::run_command;
use crate::command_ast::{Argument, CommandAst, CommandSource, ResponseFormat};
use crate::command_queue::{CommandQueue, CommandResult, MsgKind};
use crate::session::parse_cli_command;

pub struct DirectorApiImpl {
    cmd_queue: Arc<CommandQueue>,
    auth: Arc<AuthState>,
}

impl DirectorApiImpl {
    pub fn new(cmd_queue: Arc<CommandQueue>, auth: Arc<AuthState>) -> Self {
        Self { cmd_queue, auth }
    }
}

/// First words of the console command.
#[derive(Clone, Copy)]
pub enum Head {
    Fixed(&'static str),
    /// `with` when one of `fields` is set in the request, else `without`
    /// (`list pool=<name>` vs. `list pools`).
    Unless { fields: &'static [&'static str], with: &'static str, without: &'static str },
}

/// How a read RPC becomes a console command and its result a typed message.
#[derive(Clone)]
pub struct ReadRpc {
    /// RPC name as in director_api.proto
    pub rpc: &'static str,
    pub head: Head,
    /// Request fields passed on (None = all)
    pub fields: Option<&'static [&'static str]>,
    /// Request fields given as bare words (`help list`)
    pub positional: &'static [&'static str],
    /// Words appended as they are (`jobid=0`)
    pub fixed: &'static [&'static str],
    /// Text mode (no API 2 result); `Some(field)`: text unless that bool
    /// request field is set
    pub text: Option<Option<&'static str>>,
    /// Top-level JSON key renames (from, to)
    pub renames: &'static [(&'static str, &'static str)],
    /// Adjust the JSON object before the conversion
    pub post: Option<fn(&mut Map<String, Value>)>,
}

fn rpc(rpc: &'static str, head: &'static str) -> ReadRpc {
    ReadRpc { rpc, head: Head::Fixed(head), fields: None, positional: &[], fixed: &[], text: None, renames: &[], post: None }
}

impl ReadRpc {
    fn unless(mut self, fields: &'static [&'static str], with: &'static str) -> Self {
        let Head::Fixed(without) = self.head else { return self };
        self.head = Head::Unless { fields, with, without };
        self
    }
    fn fields(mut self, fields: &'static [&'static str]) -> Self { self.fields = Some(fields); self }
    fn positional(mut self, fields: &'static [&'static str]) -> Self { self.positional = fields; self }
    fn fixed(mut self, words: &'static [&'static str]) -> Self { self.fixed = words; self }
    fn text(mut self) -> Self { self.text = Some(None); self }
    fn text_unless(mut self, field: &'static str) -> Self { self.text = Some(Some(field)); self }
    fn rename(mut self, renames: &'static [(&'static str, &'static str)]) -> Self { self.renames = renames; self }
    fn post(mut self, post: fn(&mut Map<String, Value>)) -> Self { self.post = Some(post); self }

    fn text_mode(&self, request: &Map<String, Value>) -> bool {
        match self.text {
            None => false,
            Some(None) => true,
            Some(Some(field)) => request.get(field) != Some(&Value::Bool(true)),
        }
    }

    /// Console command line for a decoded request.
    pub fn command(&self, request: &Map<String, Value>) -> CommandAst {
        let mut head = match self.head {
            Head::Fixed(head) => head,
            Head::Unless { fields, with, without } =>
                if fields.iter().any(|f| request.contains_key(*f)) { with } else { without },
        }.to_string();
        if request.get("llist") == Some(&Value::Bool(true)) && head.starts_with("list") {
            head.insert(0, 'l');
        }
        let mut ast = parse_cli_command(&head);
        for (name, value) in request {
            if name == "llist" || self.fields.is_some_and(|f| !f.contains(&name.as_str())) {
                continue;
            }
            let positional = self.positional.contains(&name.as_str());
            for (key, value) in console_values(name, value) {
                ast.arguments.push(match (positional, value) {
                    (true, Some(v)) => Argument { key: v, value: None },
                    (_, value) => Argument { key, value },
                });
            }
        }
        for word in self.fixed {
            let ast_word = parse_cli_command(&format!("x {word}"));
            ast.arguments.extend(ast_word.arguments);
        }
        ast.raw_line = console_line(&ast);
        ast
    }
}

/// The console line of a command (`ua->cmd`), values with blanks quoted.
pub fn console_line(ast: &CommandAst) -> String {
    let quote = |s: &str| if s.contains(char::is_whitespace) { format!("\"{s}\"") } else { s.to_string() };
    let mut line = ast.command_name.clone();
    for arg in &ast.arguments {
        line.push(' ');
        line.push_str(&quote(&arg.key));
        if let Some(value) = &arg.value {
            line.push('=');
            line.push_str(&quote(value));
        }
    }
    line
}

/// Console arguments of one request field: bool true -> bare keyword,
/// scalars -> `key=value`, lists -> `key=a,b`, maps -> one `k=v` each.
fn console_values(name: &str, value: &Value) -> Vec<(String, Option<String>)> {
    match value {
        Value::Bool(true) => vec![(name.to_string(), None)],
        Value::Bool(false) | Value::Null => Vec::new(),
        Value::String(s) if s.is_empty() => Vec::new(),
        Value::String(s) => vec![(name.to_string(), Some(s.clone()))],
        Value::Number(n) => vec![(name.to_string(), Some(n.to_string()))],
        Value::Array(items) => vec![(name.to_string(), Some(items.iter()
            .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
            .collect::<Vec<_>>().join(",")))],
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), Some(typed::extra_text(v)))).collect(),
    }
}

// --- JSON adjustments ------------------------------------------------------

/// help/.help: the result object is keyed by command name
fn wrap_commands(obj: &mut Map<String, Value>) {
    let commands = std::mem::take(obj);
    obj.insert("commands".into(), Value::Object(commands));
}

/// list volumes/media without filter: {"volumes": {"<pool>": [...]}}
fn volumes_by_pool(obj: &mut Map<String, Value>) {
    if obj.get("volumes").is_some_and(Value::is_object) {
        let pools = obj.remove("volumes").unwrap();
        obj.insert("pools".into(), pools);
    }
}

/// .msgs lists {"text": <name>}
fn msgs_names(obj: &mut Map<String, Value>) {
    if let Some(Value::Array(items)) = obj.get_mut("items") {
        for item in items {
            if let Some(o) = item.as_object_mut() {
                if let Some(text) = o.remove("text") {
                    o.insert("name".into(), text);
                }
            }
        }
    }
}

/// list nextvolume (text): "The next Volume to be used by Job "J" (Pool=P, Level=L) will be V"
fn parse_nextvolume(obj: &mut Map<String, Value>) {
    let text = obj.get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let volumes: Vec<Value> = text.lines().filter_map(|line| {
        let rest = line.strip_prefix("The next Volume to be used by Job \"")?;
        let (job, rest) = rest.split_once("\" (Pool=")?;
        let (pool, rest) = rest.split_once(", Level=")?;
        let (level, volume) = rest.split_once(") will be ")?;
        Some(serde_json::json!({ "job": job, "pool": pool, "level": level, "volume": volume.trim_end() }))
    }).collect();
    obj.insert("volumes".into(), Value::Array(volumes));
}

/// All implemented (read) RPCs.
pub fn read_rpcs() -> &'static [ReadRpc] {
    static RPCS: OnceLock<Vec<ReadRpc>> = OnceLock::new();
    RPCS.get_or_init(|| vec![
        // info, help, messages
        rpc("GetVersion", "version"),
        rpc("GetTime", "time"),
        rpc("Whoami", "whoami"),
        rpc("GetMemory", "memory"),
        rpc("Noop", "."),
        rpc("Help", "help").positional(&["command"]).post(wrap_commands),
        rpc("DotHelp", ".help").post(wrap_commands),
        rpc("GetMessages", "messages").text(),
        rpc("DotMessages", ".messages").text(),
        rpc("ExpandVariables", "var").positional(&["text"]).text(),
        rpc("Resolve", "resolve").positional(&["hostname"]).text(),
        rpc("Estimate", "estimate").text(),
        // list / llist
        rpc("ListJobs", "list jobs"),
        rpc("GetJob", "list"),
        rpc("ListBackups", "list backups").rename(&[("backups", "jobs")]),
        rpc("ListCopies", "list copies"),
        rpc("ListJobLog", "list joblog"),
        rpc("ListJobMedia", "list jobmedia"),
        rpc("ListJobTotals", "list jobtotals"),
        rpc("ListJobStatistics", "list jobstatistics"),
        rpc("ListFiles", "list files"),
        rpc("ListBaseFiles", "list basefiles").rename(&[("files", "filenames")]),
        rpc("ListLog", "list log"),
        rpc("ListVolumes", "list volumes").post(volumes_by_pool),
        rpc("ListMedia", "list media").post(volumes_by_pool),
        rpc("GetVolume", "list"),
        rpc("ListPools", "list pools").unless(&["pool", "poolid"], "list"),
        rpc("ListClients", "list clients").unless(&["client"], "list"),
        rpc("ListFilesets", "list filesets").unless(&["filesetid", "jobid", "ujobid"], "list fileset"),
        rpc("ListStorages", "list storages"),
        rpc("ListNextVolume", "list nextvolume").text().post(parse_nextvolume),
        // status / .status
        rpc("GetDirectorStatus", "status dir").fields(&["days", "header", "running", "terminated"]),
        rpc("GetStatusAll", "status all").fields(&[]),
        rpc("GetSchedulerStatus", "status scheduler"),
        rpc("GetClientStatus", "status").fields(&["client"]).text(),
        rpc("GetStorageStatus", "status").text_unless("slots"),
        rpc("GetSubscriptions", "status subscriptions"),
        rpc("GetConfigurationStatus", "status configuration").fields(&[]).text(),
        rpc("GetJobStatus", "status").text(),
        rpc("DotStatusDirector", ".status dir"),
        rpc("DotStatusClient", ".status").text(),
        rpc("DotStatusStorage", ".status").text(),
        // show
        rpc("ShowCatalogs", "show catalog").unless(&["catalog"], "show").fields(&["catalog"]),
        rpc("ShowClients", "show clients").unless(&["client"], "show").fields(&["client"]),
        rpc("ShowConsoles", "show consoles").unless(&["console"], "show").fields(&["console"]),
        rpc("ShowDirectors", "show directors").unless(&["director"], "show").fields(&["director"]),
        rpc("ShowFilesets", "show filesets").unless(&["fileset"], "show").fields(&["fileset"]),
        rpc("ShowJobdefs", "show jobdefs").unless(&["jobdefs"], "show").fields(&["jobdefs"]),
        rpc("ShowJobs", "show jobs").unless(&["job"], "show").fields(&["job"]),
        rpc("ShowMessages", "show messages").unless(&["message"], "show").fields(&["message"]),
        rpc("ShowPools", "show pools").unless(&["pool"], "show").fields(&["pool"]),
        rpc("ShowProfiles", "show profiles").unless(&["profile"], "show").fields(&["profile"]),
        rpc("ShowSchedules", "show schedules").unless(&["schedule"], "show").fields(&["schedule"]),
        rpc("ShowStorages", "show storages").unless(&["storage"], "show").fields(&["storage"]),
        rpc("ShowUsers", "show users").unless(&["user"], "show"),
        rpc("ShowCounters", "show counters").unless(&["counter"], "show"),
        rpc("ShowAll", "show all"),
        rpc("ShowDisabled", "show disabled"),
        // dot listings
        rpc("DotJobs", ".jobs").rename(&[("jobs", "items")]),
        rpc("DotJobdefs", ".jobdefs").rename(&[("jobdefs", "items")]),
        rpc("DotClients", ".clients").rename(&[("clients", "items")]),
        rpc("DotPools", ".pools").rename(&[("pools", "items")]),
        rpc("DotStorages", ".storages").rename(&[("storages", "items")]),
        rpc("DotFilesets", ".filesets").rename(&[("filesets", "items")]),
        rpc("DotCatalogs", ".catalogs").rename(&[("catalogs", "items")]),
        rpc("DotConsoles", ".consoles").rename(&[("consoles", "items")]),
        rpc("DotUsers", ".users").rename(&[("users", "items")]),
        rpc("DotSchedule", ".schedule").rename(&[("schedules", "items")]),
        rpc("DotMsgs", ".msgs").rename(&[("messages", "items")]).post(msgs_names),
        rpc("DotProfiles", ".profiles").rename(&[("profiles", "items")]),
        rpc("DotTypes", ".types").rename(&[("jobtypes", "items")]),
        rpc("DotVolstatus", ".volstatus").rename(&[("volstatus", "items")]),
        rpc("DotActiononpurge", ".actiononpurge").rename(&[("actiononpurge", "items")]),
        rpc("DotMedia", ".media").rename(&[("media", "items")]),
        rpc("DotMediatypes", ".mediatypes").rename(&[("mediatypes", "items")]),
        rpc("DotLocations", ".locations").rename(&[("locations", "items")]),
        rpc("DotLevels", ".levels"),
        rpc("DotJobstatus", ".jobstatus"),
        rpc("DotDefaults", ".defaults"),
        rpc("DotAuthorized", ".authorized"),
        rpc("DotSqlTables", ".sqltables"),
        rpc("DotSqlCount", ".sqlcount"),
        rpc("DotSqlSchema", ".sqlschema"),
        rpc("DotAclList", ".acl"),
        rpc("DotProfileList", ".profile").rename(&[("profiles", "items")]),
        // BVFS / OVFS
        rpc("DotBvfsGetJobids", ".bvfs_get_jobids"),
        rpc("DotBvfsLsdirs", ".bvfs_lsdirs"),
        rpc("DotBvfsLsfiles", ".bvfs_lsfiles"),
        rpc("DotBvfsVersions", ".bvfs_versions").fixed(&["jobid=0"]),
        rpc("DotOvfsStatus", ".ovfs_status"),
        rpc("DotOvfsLatest", ".ovfs_latest"),
        rpc("DotOvfsTree", ".ovfs_tree"),
        rpc("DotOvfsLs", ".ovfs_ls"),
        rpc("DotOvfsVersions", ".ovfs_versions"),
    ])
}

pub fn find_rpc(name: &str) -> Option<&'static ReadRpc> {
    read_rpcs().iter().find(|r| r.rpc == name)
}

/// Error text of a failed command (API 2 "error" messages).
fn error_text(result: &CommandResult) -> String {
    let errors: Vec<String> = result.message_list().into_iter()
        .filter(|(kind, _)| *kind == MsgKind::Error).map(|(_, text)| text).collect();
    let text = if !errors.is_empty() {
        errors.concat()
    } else if !result.error_message.is_empty() {
        result.error_message.clone()
    } else {
        result.text_output.clone()
    };
    let text = text.trim_end().to_string();
    if text.is_empty() { "command failed".into() } else { text }
}

/// JSON object for the typed response: API 2 result (JSON mode) or
/// `{"text": ...}` (text mode), renamed and adjusted.
pub fn result_object(spec: &ReadRpc, text_mode: bool, result: &CommandResult) -> Map<String, Value> {
    let mut obj = if text_mode {
        let mut o = Map::new();
        o.insert("text".into(), Value::String(result.text_output.clone()));
        o
    } else {
        result.json.as_ref().and_then(Value::as_object).cloned().unwrap_or_default()
    };
    for (from, to) in spec.renames {
        if let Some(value) = obj.remove(*from) {
            obj.insert((*to).to_string(), value);
        }
    }
    if let Some(post) = spec.post {
        post(&mut obj);
    }
    obj
}

impl DirectorApiImpl {
    async fn call<Q: Message, R: Message + Default>(&self, request: Request<Q>, name: &str) -> Result<Response<R>, Status> {
        let session_id = self.auth.check_session(request.metadata())?;
        let spec = find_rpc(name).ok_or_else(|| Status::unimplemented(name.to_string()))?;
        let (input, output) = typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, name)
            .ok_or_else(|| Status::internal(format!("no descriptor for {name}")))?;
        let fields = match typed::message_to_json(&input, &request.into_inner().encode_to_vec()) {
            Value::Object(fields) => fields,
            _ => Map::new(),
        };
        let text_mode = spec.text_mode(&fields);
        let mut ast = spec.command(&fields);
        if !self.auth.allowed(session_id, crate::acl::Kind::Command, &ast.command_name) {
            return Err(Status::permission_denied(format!("command \"{}\" not allowed", ast.command_name)));
        }
        ast.session_id = session_id.to_string();
        ast.source = CommandSource::Grpc;
        ast.response_format = if text_mode { ResponseFormat::Text } else { ResponseFormat::Json };
        let result = run_command(&self.cmd_queue, ast, session_id).await?;
        if !result.success || result.messages.iter().any(|(kind, _)| *kind == MsgKind::Error) {
            return Err(Status::failed_precondition(error_text(&result)));
        }
        let obj = result_object(spec, text_mode, &result);
        let encoded = typed::json_to_message(&output, &Value::Object(obj));
        if !encoded.unmapped.is_empty() {
            tracing::debug!("DirectorApi {name}: JSON keys without field: {:?}", encoded.unmapped);
        }
        R::decode(encoded.bytes.as_slice())
            .map(Response::new)
            .map_err(|e| Status::internal(format!("{name}: {e}")))
    }
}

/// Implements the listed RPCs via [`DirectorApiImpl::call`]; all other RPCs
/// keep the generated UNIMPLEMENTED default.
macro_rules! read_rpc_impls {
    ($( $method:ident ( $req:ty ) -> $resp:ty = $name:literal; )*) => {
        #[tonic::async_trait]
        impl DirectorApi for DirectorApiImpl {
            $(
                async fn $method(&self, request: Request<$req>) -> Result<Response<$resp>, Status> {
                    self.call(request, $name).await
                }
            )*
        }

        /// (RPC name, trait method) of every implemented RPC
        pub const IMPLEMENTED: &[(&str, &str)] = &[$( ($name, stringify!($method)) ),*];
    };
}

read_rpc_impls! {
    get_version(cmd::VersionCommand) -> pb::VersionResponse = "GetVersion";
    get_time(cmd::TimeCommand) -> pb::TimeResponse = "GetTime";
    whoami(cmd::WhoamiCommand) -> pb::WhoamiResponse = "Whoami";
    get_memory(cmd::MemoryCommand) -> pb::MemoryResponse = "GetMemory";
    noop(cmd::DotCommand) -> pb::NoopResponse = "Noop";
    help(pb::HelpTopicCommand) -> pb::HelpResponse = "Help";
    dot_help(pb::DotHelpTopicCommand) -> pb::HelpResponse = "DotHelp";
    get_messages(cmd::MessagesCommand) -> pb::MessagesResponse = "GetMessages";
    dot_messages(cmd::DotMessagesCommand) -> pb::MessagesResponse = "DotMessages";
    expand_variables(pb::VarExpandCommand) -> pb::TextResponse = "ExpandVariables";
    resolve(pb::ResolveHostCommand) -> pb::TextResponse = "Resolve";
    estimate(cmd::EstimateCommand) -> pb::TextResponse = "Estimate";
    list_jobs(pb::ListJobsCommand) -> pb::ListJobsResponse = "ListJobs";
    get_job(pb::ListJobidCommand) -> pb::ListJobsResponse = "GetJob";
    list_backups(pb::ListBackupsCommand) -> pb::ListJobsResponse = "ListBackups";
    list_copies(pb::ListCopiesCommand) -> pb::ListCopiesResponse = "ListCopies";
    list_job_log(pb::ListJoblogCommand) -> pb::ListJobLogResponse = "ListJobLog";
    list_job_media(pb::ListJobmediaCommand) -> pb::ListJobMediaResponse = "ListJobMedia";
    list_job_totals(pb::ListJobtotalsCommand) -> pb::ListJobTotalsResponse = "ListJobTotals";
    list_job_statistics(pb::ListJobstatisticsCommand) -> pb::ListJobStatisticsResponse = "ListJobStatistics";
    list_files(pb::ListFilesCommand) -> pb::ListFilesResponse = "ListFiles";
    list_base_files(pb::ListBasefilesCommand) -> pb::ListFilesResponse = "ListBaseFiles";
    list_log(pb::ListLogCommand) -> pb::ListLogResponse = "ListLog";
    list_volumes(pb::ListVolumesCommand) -> pb::ListVolumesResponse = "ListVolumes";
    list_media(pb::ListVolumesCommand) -> pb::ListVolumesResponse = "ListMedia";
    get_volume(pb::ListVolumeCommand) -> pb::GetVolumeResponse = "GetVolume";
    list_pools(pb::ListPoolsCommand) -> pb::ListPoolsResponse = "ListPools";
    list_clients(pb::ListClientsCommand) -> pb::ListClientsResponse = "ListClients";
    list_filesets(pb::ListFilesetsCommand) -> pb::ListFilesetsResponse = "ListFilesets";
    list_storages(pb::ListStoragesCommand) -> pb::ListStoragesResponse = "ListStorages";
    list_next_volume(pb::ListNextvolumeCommand) -> pb::NextVolumeResponse = "ListNextVolume";
    get_director_status(cmd::StatusCommand) -> pb::DirectorStatusResponse = "GetDirectorStatus";
    get_status_all(cmd::StatusCommand) -> pb::DirectorStatusResponse = "GetStatusAll";
    get_scheduler_status(pb::StatusSchedulerCommand) -> pb::SchedulerStatusResponse = "GetSchedulerStatus";
    get_client_status(cmd::StatusCommand) -> pb::ClientStatusResponse = "GetClientStatus";
    get_storage_status(pb::StatusStorageCommand) -> pb::StorageStatusResponse = "GetStorageStatus";
    get_subscriptions(pb::StatusSubscriptionsCommand) -> pb::SubscriptionsResponse = "GetSubscriptions";
    get_configuration_status(cmd::StatusCommand) -> pb::TextResponse = "GetConfigurationStatus";
    get_job_status(pb::StatusJobCommand) -> pb::TextResponse = "GetJobStatus";
    dot_status_director(pb::DotStatusDirCommand) -> pb::DirectorStatusResponse = "DotStatusDirector";
    dot_status_client(pb::DotStatusClientCommand) -> pb::ClientStatusResponse = "DotStatusClient";
    dot_status_storage(pb::DotStatusStorageCommand) -> pb::StorageStatusResponse = "DotStatusStorage";
    show_catalogs(cmd::ShowCommand) -> pb::ShowResponse = "ShowCatalogs";
    show_clients(cmd::ShowCommand) -> pb::ShowResponse = "ShowClients";
    show_consoles(cmd::ShowCommand) -> pb::ShowResponse = "ShowConsoles";
    show_directors(cmd::ShowCommand) -> pb::ShowResponse = "ShowDirectors";
    show_filesets(cmd::ShowCommand) -> pb::ShowResponse = "ShowFilesets";
    show_jobdefs(cmd::ShowCommand) -> pb::ShowResponse = "ShowJobdefs";
    show_jobs(cmd::ShowCommand) -> pb::ShowResponse = "ShowJobs";
    show_messages(cmd::ShowCommand) -> pb::ShowResponse = "ShowMessages";
    show_pools(cmd::ShowCommand) -> pb::ShowResponse = "ShowPools";
    show_profiles(cmd::ShowCommand) -> pb::ShowResponse = "ShowProfiles";
    show_schedules(cmd::ShowCommand) -> pb::ShowResponse = "ShowSchedules";
    show_storages(cmd::ShowCommand) -> pb::ShowResponse = "ShowStorages";
    show_users(pb::ShowUserCommand) -> pb::ShowResponse = "ShowUsers";
    show_counters(pb::ShowCounterCommand) -> pb::ShowResponse = "ShowCounters";
    show_all(pb::ShowAllCommand) -> pb::ShowResponse = "ShowAll";
    show_disabled(pb::ShowDisabledCommand) -> pb::ShowDisabledResponse = "ShowDisabled";
    dot_jobs(cmd::DotJobsCommand) -> pb::NameListResponse = "DotJobs";
    dot_jobdefs(cmd::DotJobdefsCommand) -> pb::NameListResponse = "DotJobdefs";
    dot_clients(cmd::DotClientsCommand) -> pb::NameListResponse = "DotClients";
    dot_pools(cmd::DotPoolsCommand) -> pb::NameListResponse = "DotPools";
    dot_storages(cmd::DotStoragesCommand) -> pb::NameListResponse = "DotStorages";
    dot_filesets(cmd::DotFilesetsCommand) -> pb::NameListResponse = "DotFilesets";
    dot_catalogs(cmd::DotCatalogsCommand) -> pb::NameListResponse = "DotCatalogs";
    dot_consoles(cmd::DotConsolesCommand) -> pb::NameListResponse = "DotConsoles";
    dot_users(cmd::DotUsersCommand) -> pb::NameListResponse = "DotUsers";
    dot_schedule(cmd::DotScheduleCommand) -> pb::NameListResponse = "DotSchedule";
    dot_msgs(cmd::DotMsgsCommand) -> pb::NameListResponse = "DotMsgs";
    dot_profiles(cmd::DotProfilesCommand) -> pb::NameListResponse = "DotProfiles";
    dot_types(cmd::DotTypesCommand) -> pb::NameListResponse = "DotTypes";
    dot_volstatus(cmd::DotVolstatusCommand) -> pb::NameListResponse = "DotVolstatus";
    dot_actiononpurge(cmd::DotActiononpurgeCommand) -> pb::NameListResponse = "DotActiononpurge";
    dot_media(cmd::DotMediaCommand) -> pb::NameListResponse = "DotMedia";
    dot_mediatypes(cmd::DotMediatypesCommand) -> pb::NameListResponse = "DotMediatypes";
    dot_locations(cmd::DotLocationsCommand) -> pb::NameListResponse = "DotLocations";
    dot_levels(cmd::DotLevelsCommand) -> pb::LevelListResponse = "DotLevels";
    dot_jobstatus(cmd::DotJobstatusCommand) -> pb::JobStatusListResponse = "DotJobstatus";
    dot_defaults(cmd::DotDefaultsCommand) -> pb::DefaultsResponse = "DotDefaults";
    dot_authorized(cmd::DotAuthorizedCommand) -> pb::AuthorizedResponse = "DotAuthorized";
    dot_sql_tables(cmd::DotSqlTablesCommand) -> pb::SqlTablesResponse = "DotSqlTables";
    dot_sql_count(cmd::DotSqlCountCommand) -> pb::SqlTablesResponse = "DotSqlCount";
    dot_sql_schema(cmd::DotSqlSchemaCommand) -> pb::SqlSchemaResponse = "DotSqlSchema";
    dot_acl_list(cmd::DotAclListCommand) -> pb::AclListResponse = "DotAclList";
    dot_profile_list(cmd::DotProfileListCommand) -> pb::NameListResponse = "DotProfileList";
    dot_bvfs_get_jobids(cmd::DotBvfsGetJobidsCommand) -> pb::BvfsJobIdsResponse = "DotBvfsGetJobids";
    dot_bvfs_lsdirs(cmd::DotBvfsLsdirsCommand) -> pb::BvfsListResponse = "DotBvfsLsdirs";
    dot_bvfs_lsfiles(cmd::DotBvfsLsfilesCommand) -> pb::BvfsListResponse = "DotBvfsLsfiles";
    dot_bvfs_versions(cmd::DotBvfsVersionsCommand) -> pb::BvfsListResponse = "DotBvfsVersions";
    dot_ovfs_status(pb::DotOvfsStatusCommand) -> pb::OvfsStatusResponse = "DotOvfsStatus";
    dot_ovfs_latest(pb::DotOvfsLatestCommand) -> pb::OvfsLatestResponse = "DotOvfsLatest";
    dot_ovfs_tree(pb::DotOvfsTreeCommand) -> pb::OvfsTreeResponse = "DotOvfsTree";
    dot_ovfs_ls(pb::DotOvfsLsCommand) -> pb::OvfsLsResponse = "DotOvfsLs";
    dot_ovfs_versions(pb::DotOvfsVersionsCommand) -> pb::OvfsVersionsResponse = "DotOvfsVersions";
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Console line of an RPC for a typed request.
    fn line<Q: Message>(rpc: &str, request: Q) -> String {
        let (input, _) = typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, rpc).unwrap();
        let Value::Object(fields) = typed::message_to_json(&input, &request.encode_to_vec()) else { panic!() };
        find_rpc(rpc).unwrap().command(&fields).raw_line
    }

    #[test]
    fn requests_become_console_lines() {
        assert_eq!(line("ListJobs", pb::ListJobsCommand { job: "backup fd".into(), limit: 2, llist: true, ..Default::default() }),
                   "llist jobs job=\"backup fd\" limit=2");
        assert_eq!(line("ListJobs", pb::ListJobsCommand { count: true, last: false, ..Default::default() }), "list jobs count");
        assert_eq!(line("GetJob", pb::ListJobidCommand { jobid: 628, ..Default::default() }), "list jobid=628");
        // StatusCommand fields of other status forms are not passed on
        assert_eq!(line("GetDirectorStatus", cmd::StatusCommand { dir: true, client: "x".into(), days: 3, ..Default::default() }),
                   "status dir days=3");
        assert_eq!(line("GetClientStatus", cmd::StatusCommand { client: "fd".into(), running: true, ..Default::default() }),
                   "status client=fd");
        assert_eq!(line("ShowClients", cmd::ShowCommand { client: "fd".into(), job: "j".into(), ..Default::default() }),
                   "show client=fd");
        assert_eq!(line("ShowClients", cmd::ShowCommand::default()), "show clients");
        assert_eq!(line("ShowMessages", cmd::ShowCommand { message: "Standard".into(), ..Default::default() }),
                   "show message=Standard");
        assert_eq!(line("ListPools", pb::ListPoolsCommand::default()), "list pools");
        assert_eq!(line("ListPools", pb::ListPoolsCommand { poolid: 3, ..Default::default() }), "list poolid=3");
        assert_eq!(line("ListFilesets", pb::ListFilesetsCommand { jobid: 5, ..Default::default() }), "list fileset jobid=5");
        assert_eq!(line("Help", pb::HelpTopicCommand { command: "list".into() }), "help list");
        assert_eq!(line("Resolve", pb::ResolveHostCommand { client: "fd".into(), hostname: "localhost".into(), ..Default::default() }),
                   "resolve client=fd localhost");
        assert_eq!(line("DotBvfsVersions", cmd::DotBvfsVersionsCommand { client: "fd".into(), pathid: "1".into(), filename: "x".into() }),
                   ".bvfs_versions client=fd pathid=1 filename=x jobid=0");
        assert_eq!(line("DotStatusDirector", pb::DotStatusDirCommand { running: true, ..Default::default() }), ".status dir running");
        assert_eq!(line("GetSubscriptions", pb::StatusSubscriptionsCommand { all: true, client: "fd".into(), ..Default::default() }),
                   "status subscriptions all client=fd");
    }

    fn json_result(json: Value) -> CommandResult {
        CommandResult::ok(String::new()).with_json(json)
    }

    fn typed_response<R: Message + Default>(rpc: &str, text_mode: bool, result: &CommandResult) -> R {
        let (_, output) = typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, rpc).unwrap();
        let obj = result_object(find_rpc(rpc).unwrap(), text_mode, result);
        let encoded = typed::json_to_message(&output, &Value::Object(obj));
        assert!(encoded.unmapped.is_empty(), "{rpc}: {:?}", encoded.unmapped);
        R::decode(encoded.bytes.as_slice()).unwrap()
    }

    #[test]
    fn api2_results_map_to_typed_responses() {
        let r: pb::ListVolumesResponse = typed_response("ListVolumes", false, &json_result(json!({"volumes": {
            "full": [{"mediaid": "47318", "volumename": "Full-0001", "volstatus": "Error", "volbytes": "23902210671",
                      "enabled": "1", "recycle": "1", "lastwritten": "2026-03-30 22:54:15"}],
            "scratch": []}})));
        let full = &r.pools["full"].volumes[0];
        assert_eq!((full.mediaid, full.volbytes, full.volstatus()), (47318, 23902210671, pb::VolumeStatus::Error));
        assert!(full.recycle && full.enabled == 1);
        assert!(r.pools["scratch"].volumes.is_empty());

        let r: pb::ListVolumesResponse = typed_response("ListVolumes", false,
            &json_result(json!({"volumes": [{"jobmediaid": "23265", "jobid": "629", "mediaid": "47321", "volumename": "Full-47321"}]})));
        assert_eq!(r.volumes[0].jobmediaid, 23265);

        let r: pb::ListJobsResponse = typed_response("ListBackups", false, &json_result(json!({
            "backups": [{"jobid": "600", "level": "F", "type": "B", "jobstatus": "T"}],
            "meta": {"range": {"limit": 1, "filtered": 0}}})));
        assert_eq!(r.jobs[0].jobid, 600);
        assert_eq!(r.jobs[0].level(), pb::JobLevel::Full);

        let r: pb::DirectorStatusResponse = typed_response("GetDirectorStatus", false, &json_result(json!({
            "header": {"director": "bareos-dir", "jobs_run": 0, "config_warnings": false},
            "scheduled": [{"name": "backup-bareos-fd", "type": "Backup", "level": "Differential", "priority": 10}],
            "running": [], "terminated": [],
            "client-connection": [{"connecttime": "x", "protocol_version": 54, "authenticated": true, "name": "fd"}]})));
        assert_eq!(r.scheduled[0].r#type(), pb::JobType::Backup);
        assert_eq!(r.scheduled[0].level(), pb::JobLevel::Differential);
        assert_eq!(r.client_connection[0].protocol_version, 54);

        let r: pb::HelpResponse = typed_response("Help", false, &json_result(json!({
            "list": {"command": "list", "description": "List objects from catalog", "arguments": "", "permission": true}})));
        assert!(r.commands["list"].permission);

        let r: pb::NameListResponse = typed_response("DotMsgs", false, &json_result(json!({"messages": [{"text": "Standard"}]})));
        assert_eq!(r.items[0].name, "Standard");

        let r: pb::NameListResponse = typed_response("DotJobs", false,
            &json_result(json!({"jobs": [{"name": "backup-bareos-fd", "enabled": true}]})));
        assert_eq!((r.items[0].name.as_str(), r.items[0].enabled), ("backup-bareos-fd", Some(true)));

        let r: pb::AuthorizedResponse = typed_response("DotAuthorized", false,
            &json_result(json!({".authorized": {"authorized": true}})));
        assert!(r.authorized.unwrap().authorized);
    }

    #[test]
    fn text_results_fill_the_text_field() {
        let text = "Using Catalog \"MyCatalog\"\nThe next Volume to be used by Job \"backup-bareos-fd\" \
                    (Pool=Incremental, Level=Differential) will be Incremental-47322\n";
        let r: pb::NextVolumeResponse = typed_response("ListNextVolume", true, &CommandResult::ok(text.into()));
        assert_eq!(r.text, text);
        assert_eq!(r.volumes.len(), 1);
        assert_eq!((r.volumes[0].pool.as_str(), r.volumes[0].volume.as_str()), ("Incremental", "Incremental-47322"));
        let r: pb::ClientStatusResponse = typed_response("GetClientStatus", true, &CommandResult::ok("fd status\n".into()));
        assert_eq!(r.text, "fd status\n");
        // status storage: text unless slots
        let fields = |slots: bool| {
            let mut m = Map::new();
            m.insert("storage".into(), json!("File"));
            if slots {
                m.insert("slots".into(), json!(true));
            }
            m
        };
        let spec = find_rpc("GetStorageStatus").unwrap();
        assert!(spec.text_mode(&fields(false)) && !spec.text_mode(&fields(true)));
    }

    /// Service with a session of console "limited" (or the default console
    /// with the Director password "secret").
    fn service_for(console: &str, password: &str) -> (DirectorApiImpl, Arc<CommandQueue>, crate::command_executor::ExecContext, String) {
        let ctx = crate::acl::tests::context();
        let manager = ctx.session_mgr.as_ref().unwrap().clone();
        let auth = Arc::new(AuthState::new(ctx.config.clone().unwrap(), manager));
        let nonce = auth.challenge(console);
        let proof = super::super::auth::compute_proof(&super::super::auth::password_key(password), &nonce);
        let token = auth.login(console, &nonce, &proof).unwrap();
        let queue = Arc::new(CommandQueue::new(8));
        (DirectorApiImpl::new(queue.clone(), auth), queue, ctx, token)
    }

    fn service() -> (DirectorApiImpl, Arc<CommandQueue>, crate::command_executor::ExecContext, String) {
        service_for("limited", "pw")
    }

    fn with_token<Q>(message: Q, token: &str) -> Request<Q> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(super::super::auth::SESSION_HEADER, token.parse().unwrap());
        request
    }

    #[tokio::test]
    async fn rpcs_use_session_acl_and_the_command_queue() {
        let (api, queue, ctx, token) = service();
        // no token
        let err = api.get_version(Request::new(cmd::VersionCommand {})).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
        // CommandACL of console "limited" has no "list"
        let err = api.list_jobs(with_token(pb::ListJobsCommand::default(), &token)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        // version is allowed: runs through the queue in JSON mode
        let task = tokio::spawn(async move { api.get_version(with_token(cmd::VersionCommand {}, &token)).await });
        let queued = tokio::task::spawn_blocking(move || queue.take().unwrap()).await.unwrap();
        assert_eq!(queued.ast.command_name, "version");
        assert_eq!(queued.ast.response_format, ResponseFormat::Json);
        let mut exec = ctx;
        exec.session_id = queued.session_id;
        exec.api_mode = crate::command_executor::ApiMode::Json;
        queued.result_tx.send(crate::command_executor::dispatch(&queued.ast, &exec)).unwrap();
        let version = task.await.unwrap().unwrap().into_inner().version.unwrap();
        assert!(!version.version.is_empty());
    }

    #[tokio::test]
    async fn failed_commands_and_write_rpcs() {
        let (api, queue, _, token) = service_for("*UserAgent*", "secret");
        let task = tokio::spawn(async move {
            api.get_job_status(with_token(pb::StatusJobCommand { job: "x".into() }, &token)).await
        });
        let queued = tokio::task::spawn_blocking(move || queue.take().unwrap()).await.unwrap();
        queued.result_tx.send(CommandResult::error("Job \"x\" is not running.\n".into())).unwrap();
        let err = task.await.unwrap().unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(err.message(), "Job \"x\" is not running.");

        let (api, _, _, token) = service();
        let err = api.run(with_token(cmd::RunCommand::default(), &token)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        let err = api.restore_tree_cd(with_token(pb::RestoreTreePathCommand::default(), &token)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}
