//! Coverage of the Director's command set by MCP tools (S-36, C-087).
//!
//! Every RPC of `DirectorApi` stands for one console command or subcommand
//! (the table "Abdeckung: Befehl → RPC" in docs/wiki/grpc-api.md, generated
//! from the command inventory docs/wiki/commands/bareos-director-commands.json:
//! the 88 Director commands with their subcommands, the Onesimus extensions
//! and the 26 commands of the restore file selection). This module maps each
//! RPC to the MCP tool that covers it, with sample arguments, and checks:
//!
//! - every DirectorApi RPC and every row of the wiki table has a tool (or is
//!   one of the session commands that make no sense through MCP);
//! - every command and restore-tree command of the JSON inventory appears
//!   in the wiki table;
//! - each tool is listed with write access, builds its console line from the
//!   sample arguments and — where it has a typed route — maps them to
//!   exactly that RPC without losing an argument.

use serde_json::{Value, json};

/// Session commands that MCP does not offer: they would end or reconfigure
/// the tool's own Director session (quit/exit/.quit, .api/.grpc output
/// mode, gui) or need a BNET console address (setip). The typed API answers
/// Gui/DotApi/DotGrpc/SetIp with FAILED_PRECONDITION for the same reason.
pub const NOT_APPLICABLE: &[(&str, &str)] = &[
    ("Gui", "gui on/off only switches the console's GUI mode"),
    ("DotApi", "oconsole sets the API mode per tool call itself"),
    ("DotGrpc", "the output format is fixed per tool call"),
    ("SetIp", "needs the address of a BNET console, not a tool call"),
    ("Quit", "would end the MCP server's Director session"),
    ("Exit", "would end the MCP server's Director session"),
    ("DotQuit", "would end the MCP server's Director session"),
];

/// (RPC, tool, sample arguments, typed route); `false`: the tool always
/// uses the console line (its dialog answers have no request field).
pub const COVERAGE: &[(&str, &str, &str, bool)] = &[
    ("GetVersion", "version", "{}", true),
    ("GetTime", "time", "{}", true),
    ("Whoami", "whoami", "{}", true),
    ("GetMemory", "memory", "{}", true),
    ("Noop", "dot_noop", "{}", true),
    ("Help", "help", r#"{"command":"list"}"#, true),
    ("DotHelp", "dot_help", r#"{"item":"list"}"#, true),
    ("GetMessages", "messages", "{}", true),
    ("DotMessages", "dot_messages", "{}", true),
    ("ExpandVariables", "var_expand", r#"{"text":"x"}"#, true),
    ("Resolve", "resolve", r#"{"hostname":"localhost"}"#, true),
    ("Estimate", "estimate", r#"{"job":"j","level":"Full","accurate":true}"#, true),
    ("ListJobs", "list_jobs", r#"{"client":"fd","jobstatus":["T"],"long":true}"#, true),
    ("GetJob", "list_job", r#"{"jobid":1}"#, true),
    ("ListBackups", "list_backups", r#"{"client":"fd"}"#, true),
    ("ListCopies", "list_copies", r#"{"jobids":[1,2]}"#, true),
    ("ListJobLog", "list_joblog", r#"{"jobid":1}"#, true),
    ("ListJobMedia", "list_jobmedia", r#"{"jobid":1}"#, true),
    ("ListJobTotals", "list_jobtotals", "{}", true),
    ("ListJobStatistics", "list_jobstatistics", r#"{"jobid":1}"#, true),
    ("ListFiles", "list_files", r#"{"jobid":1,"limit":5}"#, true),
    ("ListBaseFiles", "list_basefiles", r#"{"jobid":1}"#, true),
    ("ListLog", "list_log", r#"{"limit":5,"reverse":true}"#, true),
    ("ListVolumes", "list_volumes", r#"{"pool":"Full"}"#, true),
    ("ListMedia", "list_volumes", r#"{"media":true}"#, true),
    ("GetVolume", "list_volumes", r#"{"volume":"V"}"#, true),
    ("ListPools", "list_pools", r#"{"poolid":1}"#, true),
    ("ListClients", "list_clients", "{}", true),
    ("ListFilesets", "list_filesets", "{}", true),
    ("ListStorages", "list_storages", "{}", true),
    ("ListNextVolume", "list_nextvolume", r#"{"job":"j"}"#, true),
    ("GetDirectorStatus", "status_director", r#"{"days":2}"#, true),
    ("GetStatusAll", "status_all", "{}", true),
    ("GetSchedulerStatus", "status_scheduler", r#"{"days":3}"#, true),
    ("GetClientStatus", "status_client", r#"{"client":"fd"}"#, true),
    ("GetStorageStatus", "status_storage", r#"{"storage":"File"}"#, true),
    ("GetSubscriptions", "status_subscriptions", r#"{"all":true}"#, true),
    ("GetConfigurationStatus", "status_configuration", "{}", true),
    ("GetJobStatus", "status_job", r#"{"job":"j"}"#, true),
    ("DotStatusDirector", "dot_status", r#"{"target":"dir","keyword":"running"}"#, true),
    ("DotStatusClient", "dot_status", r#"{"target":"client","name":"fd","keyword":"header"}"#, true),
    ("DotStatusStorage", "dot_status", r#"{"target":"storage","name":"File","keyword":"devices"}"#, true),
    ("ShowCatalogs", "show", r#"{"resource":"catalog"}"#, true),
    ("ShowClients", "show", r#"{"resource":"client","name":"fd"}"#, true),
    ("ShowConsoles", "show", r#"{"resource":"console"}"#, true),
    ("ShowDirectors", "show", r#"{"resource":"director"}"#, true),
    ("ShowFilesets", "show", r#"{"resource":"fileset"}"#, true),
    ("ShowJobdefs", "show", r#"{"resource":"jobdefs"}"#, true),
    ("ShowJobs", "show", r#"{"resource":"job","name":"j"}"#, true),
    ("ShowMessages", "show", r#"{"resource":"messages"}"#, true),
    ("ShowPools", "show", r#"{"resource":"pool"}"#, true),
    ("ShowProfiles", "show", r#"{"resource":"profile"}"#, true),
    ("ShowSchedules", "show", r#"{"resource":"schedule"}"#, true),
    ("ShowStorages", "show", r#"{"resource":"storage"}"#, true),
    ("ShowUsers", "show", r#"{"resource":"user"}"#, true),
    ("ShowCounters", "show", r#"{"resource":"counter"}"#, true),
    ("ShowAll", "show", r#"{"resource":"all","verbose":true}"#, true),
    ("ShowDisabled", "show", r#"{"resource":"disabled","disabled_type":"jobs"}"#, true),
    ("DotJobs", "dot_jobs", r#"{"type":"Backup"}"#, true),
    ("DotJobdefs", "dot_list", r#"{"kind":"jobdefs"}"#, true),
    ("DotClients", "dot_clients", "{}", true),
    ("DotPools", "dot_pools", "{}", true),
    ("DotStorages", "dot_storages", "{}", true),
    ("DotFilesets", "dot_filesets", "{}", true),
    ("DotCatalogs", "dot_list", r#"{"kind":"catalogs"}"#, true),
    ("DotConsoles", "dot_list", r#"{"kind":"consoles"}"#, true),
    ("DotUsers", "dot_list", r#"{"kind":"users"}"#, true),
    ("DotSchedule", "dot_schedule", "{}", true),
    ("DotMsgs", "dot_msgs", "{}", true),
    ("DotProfiles", "dot_list", r#"{"kind":"profiles"}"#, true),
    ("DotTypes", "dot_types", "{}", true),
    ("DotVolstatus", "dot_list", r#"{"kind":"volstatus"}"#, true),
    ("DotActiononpurge", "dot_list", r#"{"kind":"actiononpurge"}"#, true),
    ("DotMedia", "dot_list", r#"{"kind":"media"}"#, true),
    ("DotMediatypes", "dot_list", r#"{"kind":"mediatypes"}"#, true),
    ("DotLocations", "dot_list", r#"{"kind":"locations"}"#, true),
    ("DotLevels", "dot_levels", "{}", true),
    ("DotJobstatus", "dot_jobstatus", "{}", true),
    ("DotDefaults", "dot_defaults", r#"{"name":"j"}"#, true),
    ("DotAuthorized", "dot_authorized", r#"{"cmd":"run"}"#, true),
    ("DotSqlTables", "sql_meta", r#"{"kind":"tables"}"#, true),
    ("DotSqlCount", "sql_meta", r#"{"kind":"count","table":"job"}"#, true),
    ("DotSqlSchema", "sql_meta", r#"{"kind":"schema","table":"job"}"#, true),
    ("DotAclList", "acl_list", r#"{"console":"c"}"#, true),
    ("DotProfileList", "profile_list", "{}", true),
    ("DotBvfsGetJobids", "bvfs_get_jobids", r#"{"jobid":1}"#, true),
    ("DotBvfsLsdirs", "bvfs_lsdirs", r#"{"jobids":[1],"path":"/"}"#, true),
    ("DotBvfsLsfiles", "bvfs_lsfiles", r#"{"jobids":[1],"path":"/etc/","limit":10}"#, true),
    ("DotBvfsVersions", "bvfs_versions", r#"{"client":"fd","pathid":1,"filename":"x"}"#, true),
    ("DotOvfsStatus", "ovfs_status", "{}", true),
    ("DotOvfsLatest", "ovfs_latest", r#"{"client":"fd"}"#, true),
    ("DotOvfsTree", "ovfs_tree", r#"{"jobids":"1,2","depth":1}"#, true),
    ("DotOvfsLs", "ovfs_ls", r#"{"jobids":"1","path":"/"}"#, true),
    ("DotOvfsVersions", "ovfs_versions", r#"{"client":"fd","path":"/etc/","name":"x"}"#, true),
    ("Run", "run_job", r#"{"job":"j","level":"Full"}"#, true),
    ("Rerun", "rerun_job", r#"{"jobids":[1]}"#, true),
    ("Cancel", "cancel_job", r#"{"jobid":1}"#, true),
    ("Restore", "restore", r#"{"client":"fd","jobids":[1],"all":true,"where":"/tmp/r"}"#, true),
    ("Label", "label_volume", r#"{"storage":"S","volume":"V","pool":"P"}"#, true),
    ("Relabel", "relabel_volume", r#"{"storage":"S","oldvolume":"A","volume":"B","confirm":true}"#, true),
    ("Mount", "mount_storage", r#"{"storage":"S","slot":1}"#, true),
    ("Unmount", "unmount_storage", r#"{"storage":"S"}"#, true),
    ("Release", "release_storage", r#"{"storage":"S"}"#, true),
    ("AddVolumes", "add_volumes", r#"{"pool":"P","storage":"S","count":2,"name":"V"}"#, false),
    ("CreatePool", "create_pool", r#"{"pool":"P"}"#, true),
    ("DeleteVolume", "delete", r#"{"kind":"volume","volume":"V","confirm":true}"#, true),
    ("DeletePool", "delete", r#"{"kind":"pool","pool":"P","confirm":true}"#, true),
    ("DeleteJob", "delete", r#"{"kind":"jobid","jobids":[1,2],"confirm":true}"#, true),
    ("DeleteStorage", "delete", r#"{"kind":"storage","storage":"S","confirm":true}"#, true),
    ("Enable", "enable", r#"{"resource":"job","name":"j"}"#, true),
    ("Disable", "disable", r#"{"resource":"schedule","name":"s"}"#, true),
    ("ExportVolumes", "export_volumes", r#"{"storage":"S","srcslots":"1-2"}"#, true),
    ("ImportVolumes", "import_volumes", r#"{"storage":"S"}"#, true),
    ("MoveVolumes", "move_volumes", r#"{"storage":"S","srcslots":"1","dstslots":"2"}"#, true),
    ("PruneFiles", "prune", r#"{"kind":"files","client":"fd","confirm":true}"#, true),
    ("PruneJobs", "prune", r#"{"kind":"jobs","pool":"P","confirm":true}"#, true),
    ("PruneVolume", "prune", r#"{"kind":"volume","volume":"V","confirm":true}"#, true),
    ("PruneStats", "prune", r#"{"kind":"stats","confirm":true}"#, true),
    ("PruneDirectory", "prune", r#"{"kind":"directory","directory":"/tmp/","client":"fd","confirm":true}"#, true),
    ("PurgeFiles", "purge", r#"{"kind":"files","jobid":1,"confirm":true}"#, true),
    ("PurgeJobs", "purge", r#"{"kind":"jobs","client":"fd","confirm":true}"#, true),
    ("PurgeVolume", "purge", r#"{"kind":"volume","volume":["V"],"confirm":true}"#, true),
    ("PurgeQuota", "purge", r#"{"kind":"quota","client":"fd","confirm":true}"#, true),
    ("UpdateVolume", "update_volume", r#"{"volume":"V","volstatus":"Used","recycle":true}"#, true),
    ("UpdatePool", "update_pool", r#"{"pool":"P"}"#, true),
    ("UpdateSlots", "update_slots", r#"{"storage":"S","scan":true}"#, true),
    ("UpdateJob", "update_job", r#"{"jobid":1,"jobname":"x"}"#, true),
    ("UpdateStats", "update_stats", r#"{"days":3}"#, true),
    ("Truncate", "truncate", r#"{"pool":"P","confirm":true}"#, true),
    ("ConfigureAdd", "configure_add", r#"{"resource":"client","name":"x","directives":{"address":"h","password":"p"}}"#, true),
    ("ConfigureExport", "configure_export", r#"{"client":"fd"}"#, true),
    ("ConfigureDelete", "configure_delete", r#"{"resource":"client","name":"x","confirm":true}"#, true),
    ("ConfigureUpdate", "configure_update", r#"{"resource":"client","name":"x","directives":{"address":"h"}}"#, true),
    ("Reload", "reload", "{}", true),
    ("SetDebug", "setdebug", r#"{"level":0,"target":"dir"}"#, true),
    ("SetBandwidth", "setbandwidth", r#"{"limit":100,"client":"fd"}"#, true),
    ("SetDevice", "setdevice", r#"{"storage":"S","device":"D","autoselect":true}"#, true),
    ("Trace", "session_option", r#"{"option":"trace","on":false}"#, true),
    ("DotSql", "dot_sql", r#"{"query":"select 1"}"#, true),
    ("SqlQuery", "sqlquery", r#"{"query":"select 1"}"#, true),
    ("Query", "run_query", r#"{"number":1,"parameters":["fd"]}"#, true),
    ("DotBvfsUpdate", "bvfs_update", r#"{"jobids":[1]}"#, true),
    ("DotBvfsRestore", "bvfs_restore", r#"{"path":"b21","jobid":"1","fileid":"1,2"}"#, true),
    ("DotBvfsCleanup", "bvfs_cleanup", r#"{"path":"b21"}"#, true),
    ("DotBvfsClearCache", "bvfs_clear_cache", "{}", true),
    ("DotAclSet", "acl_modify", r#"{"action":"set","console":"c","type":"job","value":"a,b"}"#, true),
    ("DotAclDelete", "acl_modify", r#"{"action":"delete","console":"c","type":"job","value":"a"}"#, true),
    ("DotAclClear", "acl_modify", r#"{"action":"clear","console":"c"}"#, true),
    ("DotAclCopy", "acl_modify", r#"{"action":"copy","from":"a","to":"b"}"#, true),
    ("DotProfileCreate", "profile_modify", r#"{"action":"create","name":"p"}"#, true),
    ("DotProfileSet", "profile_modify", r#"{"action":"set","name":"p","type":"job","value":"a"}"#, true),
    ("DotProfileDelete", "profile_modify", r#"{"action":"delete","name":"p","type":"job","value":"a"}"#, true),
    ("DotProfileAssign", "profile_modify", r#"{"action":"assign","console":"c","profile":"p"}"#, true),
    ("DotProfileUnassign", "profile_modify", r#"{"action":"unassign","console":"c","profile":"p"}"#, true),
    ("UseCatalog", "use_catalog", r#"{"catalog":"MyCatalog"}"#, true),
    ("Wait", "wait_job", r#"{"jobid":1}"#, true),
    ("Autodisplay", "session_option", r#"{"option":"autodisplay","on":true}"#, true),
    ("Automount", "session_option", r#"{"option":"automount","on":true}"#, true),
    ("StartRestoreSession", "restore_tree_start", r#"{"client":"fd","jobids":[1],"where":"/tmp/r"}"#, true),
    ("RestoreTreeCd", "restore_tree_cd", r#"{"session_id":"s","path":"/etc/"}"#, true),
    ("RestoreTreeLs", "restore_tree_ls", r#"{"session_id":"s","pattern":"*.conf"}"#, true),
    ("RestoreTreeLsDir", "restore_tree_ls", r#"{"session_id":"s","mode":"dirs"}"#, true),
    ("RestoreTreeLsMark", "restore_tree_ls", r#"{"session_id":"s","mode":"marked"}"#, true),
    ("RestoreTreePwd", "restore_tree_pwd", r#"{"session_id":"s"}"#, true),
    ("RestoreTreeMark", "restore_tree_mark", r#"{"session_id":"s","pattern":"hosts"}"#, true),
    ("RestoreTreeUnmark", "restore_tree_unmark", r#"{"session_id":"s","pattern":"hosts"}"#, true),
    ("RestoreTreeMarkDir", "restore_tree_mark", r#"{"session_id":"s","pattern":"etc","dirs_only":true}"#, true),
    ("RestoreTreeUnmarkDir", "restore_tree_unmark", r#"{"session_id":"s","pattern":"etc","dirs_only":true}"#, true),
    ("RestoreTreeCount", "restore_tree_count", r#"{"session_id":"s"}"#, true),
    ("RestoreTreeFind", "restore_tree_find", r#"{"session_id":"s","pattern":"*.conf"}"#, true),
    ("RestoreTreeEstimate", "restore_tree_estimate", r#"{"session_id":"s"}"#, true),
    ("RestoreTreeHelp", "restore_tree_help", r#"{"session_id":"s"}"#, true),
    ("RestoreTreeDone", "restore_tree_done", r#"{"session_id":"s"}"#, true),
    ("RestoreTreeAbort", "restore_tree_abort", r#"{"session_id":"s"}"#, true),
];

/// The console line a tool builds from its arguments.
fn console_line(tool: &str, args: &Value) -> Result<String, String> {
    if let Some(line) = crate::mcp_read::console_line(tool, args) {
        return line;
    }
    if crate::mcp_tree::is_tool(tool) {
        return if tool == "restore_tree_start" {
            crate::mcp_tree::start_line(args)
        } else {
            crate::mcp_tree::tree_line(tool, args)
        };
    }
    crate::mcp_write::plan(tool, args).ok_or_else(|| format!("{tool}: no such tool"))?.map(|p| p.command)
}

fn doc_rows() -> Vec<(String, String, String)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/wiki/grpc-api.md");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let table = text.split("<!-- coverage:begin").nth(1).and_then(|t| t.split("<!-- coverage:end").next())
        .expect("coverage table in grpc-api.md");
    table.lines().filter(|l| l.starts_with("| ") && !l.starts_with("| Befehl")).map(|l| {
        let cells: Vec<String> = l.split('|').map(|c| c.trim().replace('`', "")).collect();
        (cells[1].clone(), cells[2].clone(), cells[3].clone())
    }).collect()
}

#[test]
fn every_rpc_has_a_tool() {
    for rpc in crate::mcp_typed::rpc_names() {
        let covered = COVERAGE.iter().any(|(r, ..)| *r == rpc) || NOT_APPLICABLE.iter().any(|(r, _)| *r == rpc);
        assert!(covered, "DirectorApi RPC {rpc} has no MCP tool");
    }
    for (rpc, ..) in COVERAGE {
        assert!(crate::mcp_typed::rpc_types(rpc).is_some(), "{rpc} is not a DirectorApi RPC");
    }
}

#[test]
fn every_command_and_subcommand_has_a_tool() {
    let rows = doc_rows();
    assert!(rows.len() > 250, "coverage table too short: {}", rows.len());
    for (command, sub, rpc) in &rows {
        let covered = COVERAGE.iter().any(|(r, ..)| r == rpc) || NOT_APPLICABLE.iter().any(|(r, _)| r == rpc);
        assert!(covered, "{command} {sub} ({rpc}) has no MCP tool");
    }
    // The table lists every command of the inventory.
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/wiki/commands/bareos-director-commands.json");
    let inventory: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let commands = inventory["commands"].as_array().unwrap();
    assert_eq!(commands.len(), 88);
    for c in commands {
        let name = c["name"].as_str().unwrap();
        assert!(rows.iter().any(|(cmd, ..)| cmd == name), "command {name} missing in the coverage table");
    }
    let tree = inventory["restore_file_selection_commands"].as_array().unwrap();
    assert_eq!(tree.len(), 26);
    for c in tree {
        let name = format!("Restore-Baum: {}", c["name"].as_str().unwrap());
        assert!(rows.iter().any(|(cmd, ..)| *cmd == name), "{name} missing in the coverage table");
    }
    for c in inventory["onesimus_extensions"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        assert!(rows.iter().any(|(cmd, ..)| cmd == name), "extension {name} missing in the coverage table");
    }
}

#[test]
fn every_tool_builds_its_line_and_typed_request() {
    let listed: Vec<String> = crate::mcp_read::all_tools().into_iter().chain(crate::mcp_write::all_tools())
        .map(|t| t["name"].as_str().unwrap().to_string()).collect();
    for (rpc, tool, args, typed) in COVERAGE {
        assert!(listed.iter().any(|t| t == tool), "{tool} ({rpc}) not listed with write access");
        let args: Value = serde_json::from_str(args).unwrap();
        let line = console_line(tool, &args).unwrap_or_else(|e| panic!("{tool} {args}: {e}"));
        assert!(!line.is_empty() && !line.contains('\n'), "{tool}: {line:?}");
        let request = crate::mcp_typed::request(tool, &args);
        if *typed {
            let (got, request) = request.unwrap_or_else(|| panic!("{tool}: no typed route"));
            assert_eq!(got, *rpc, "{tool} {args}");
            crate::mcp_typed::encode(rpc, &request).unwrap_or_else(|e| panic!("{tool} {args}: {e}"));
        } else if let Some((got, request)) = request {
            assert_eq!(got, *rpc);
            assert!(crate::mcp_typed::encode(rpc, &request).is_err(), "{tool}: expected the console line");
        }
    }
}

/// Every write and read tool of the lists appears in the coverage table
/// (no tool without a command), except the generic ones.
#[test]
fn every_tool_covers_a_command() {
    let generic = ["query", "execute"];
    for t in crate::mcp_read::all_tools().into_iter().chain(crate::mcp_write::all_tools()) {
        let name = t["name"].as_str().unwrap();
        assert!(generic.contains(&name) || COVERAGE.iter().any(|(_, tool, ..)| *tool == name), "{name} covers no RPC");
    }
    let _ = json!({});
}
