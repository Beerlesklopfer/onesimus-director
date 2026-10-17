//! Which DirectorApi RPC covers which console command / subcommand.
//!
//! The test below checks this table against the command inventory
//! docs/wiki/commands/bareos-director-commands.json (88 Director commands,
//! the restore file-selection commands and the Onesimus extensions) and the
//! RPCs of director_api.proto, and keeps the coverage table in
//! docs/wiki/grpc-api.md in sync (UPDATE_GRPC_API_DOC=1 rewrites it).
//!
//! Subcommand notation: `key=` is the `key=<value>` form, a bare word the
//! keyword form; `add <type>` for configure; empty = the command itself.

#![cfg_attr(not(test), allow(dead_code))]

/// (command, subcommand, RPC)
pub const COVERAGE: &[(&str, &str, &str)] = &[
    (".", "", "Noop"),
    (".actiononpurge", "", "DotActiononpurge"),
    (".api", "", "DotApi"),
    (".authorized", "", "DotAuthorized"),
    (".catalogs", "", "DotCatalogs"),
    (".clients", "", "DotClients"),
    (".consoles", "", "DotConsoles"),
    (".users", "", "DotUsers"),
    (".defaults", "", "DotDefaults"),
    (".filesets", "", "DotFilesets"),
    (".help", "", "DotHelp"),
    (".jobdefs", "", "DotJobdefs"),
    (".jobs", "", "DotJobs"),
    (".jobstatus", "", "DotJobstatus"),
    (".levels", "", "DotLevels"),
    (".locations", "", "DotLocations"),
    (".messages", "", "DotMessages"),
    (".media", "", "DotMedia"),
    (".mediatypes", "", "DotMediatypes"),
    (".msgs", "", "DotMsgs"),
    (".pools", "", "DotPools"),
    (".profiles", "", "DotProfiles"),
    (".quit", "", "DotQuit"),
    (".sql", "", "DotSql"),
    (".schedule", "", "DotSchedule"),
    (".status", "dir", "DotStatusDirector"),
    (".status", "storage=", "DotStatusStorage"),
    (".status", "client=", "DotStatusClient"),
    (".storages", "", "DotStorages"),
    (".types", "", "DotTypes"),
    (".volstatus", "", "DotVolstatus"),
    (".bvfs_lsdirs", "", "DotBvfsLsdirs"),
    (".bvfs_lsfiles", "", "DotBvfsLsfiles"),
    (".bvfs_update", "", "DotBvfsUpdate"),
    (".bvfs_get_jobids", "", "DotBvfsGetJobids"),
    (".bvfs_versions", "", "DotBvfsVersions"),
    (".bvfs_restore", "", "DotBvfsRestore"),
    (".bvfs_cleanup", "", "DotBvfsCleanup"),
    (".bvfs_clear_cache", "", "DotBvfsClearCache"),
    ("add", "", "AddVolumes"),
    ("autodisplay", "", "Autodisplay"),
    ("automount", "", "Automount"),
    ("cancel", "", "Cancel"),
    ("configure", "add client", "ConfigureAdd"),
    ("configure", "add jobdefs", "ConfigureAdd"),
    ("configure", "add job", "ConfigureAdd"),
    ("configure", "add storage", "ConfigureAdd"),
    ("configure", "add catalog", "ConfigureAdd"),
    ("configure", "add schedule", "ConfigureAdd"),
    ("configure", "add fileset", "ConfigureAdd"),
    ("configure", "add pool", "ConfigureAdd"),
    ("configure", "add messages", "ConfigureAdd"),
    ("configure", "add counter", "ConfigureAdd"),
    ("configure", "add profile", "ConfigureAdd"),
    ("configure", "add console", "ConfigureAdd"),
    ("configure", "add user", "ConfigureAdd"),
    ("configure", "export", "ConfigureExport"),
    ("configure", "delete", "ConfigureDelete"),
    ("configure", "update", "ConfigureUpdate"),
    ("create", "", "CreatePool"),
    ("delete", "volume=", "DeleteVolume"),
    ("delete", "volume", "DeleteVolume"),
    ("delete", "pool=", "DeletePool"),
    ("delete", "jobid=", "DeleteJob"),
    ("delete", "storage", "DeleteStorage"),
    ("disable", "", "Disable"),
    ("enable", "", "Enable"),
    ("estimate", "", "Estimate"),
    ("exit", "", "Exit"),
    ("export", "", "ExportVolumes"),
    ("gui", "", "Gui"),
    ("help", "", "Help"),
    ("import", "", "ImportVolumes"),
    ("label", "", "Label"),
    ("list", "basefiles", "ListBaseFiles"),
    ("list", "backups", "ListBackups"),
    ("list", "client=", "ListClients"),
    ("list", "clients", "ListClients"),
    ("list", "copies", "ListCopies"),
    ("list", "files", "ListFiles"),
    ("list", "filesets", "ListFilesets"),
    ("list", "fileset", "ListFilesets"),
    ("list", "jobs", "ListJobs"),
    ("list", "job=", "ListJobs"),
    ("list", "jobid=", "GetJob"),
    ("list", "ujobid=", "GetJob"),
    ("list", "joblog", "ListJobLog"),
    ("list", "jobmedia", "ListJobMedia"),
    ("list", "jobtotals", "ListJobTotals"),
    ("list", "jobstatistics", "ListJobStatistics"),
    ("list", "log", "ListLog"),
    ("list", "media", "ListMedia"),
    ("list", "media=", "GetVolume"),
    ("list", "nextvolume", "ListNextVolume"),
    ("list", "pools", "ListPools"),
    ("list", "pool=", "ListPools"),
    ("list", "poolid=", "ListPools"),
    ("list", "storages", "ListStorages"),
    ("list", "volumes", "ListVolumes"),
    ("list", "volume=", "GetVolume"),
    ("list", "volumeid=", "GetVolume"),
    ("list", "mediaid=", "GetVolume"),
    ("llist", "basefiles", "ListBaseFiles"),
    ("llist", "backups", "ListBackups"),
    ("llist", "client=", "ListClients"),
    ("llist", "clients", "ListClients"),
    ("llist", "copies", "ListCopies"),
    ("llist", "files", "ListFiles"),
    ("llist", "filesets", "ListFilesets"),
    ("llist", "fileset", "ListFilesets"),
    ("llist", "jobs", "ListJobs"),
    ("llist", "job=", "ListJobs"),
    ("llist", "jobid=", "GetJob"),
    ("llist", "ujobid=", "GetJob"),
    ("llist", "joblog", "ListJobLog"),
    ("llist", "jobmedia", "ListJobMedia"),
    ("llist", "jobtotals", "ListJobTotals"),
    ("llist", "jobstatistics", "ListJobStatistics"),
    ("llist", "log", "ListLog"),
    ("llist", "media", "ListMedia"),
    ("llist", "media=", "GetVolume"),
    ("llist", "nextvolume", "ListNextVolume"),
    ("llist", "pools", "ListPools"),
    ("llist", "pool=", "ListPools"),
    ("llist", "poolid=", "ListPools"),
    ("llist", "storages", "ListStorages"),
    ("llist", "volumes", "ListVolumes"),
    ("llist", "volume=", "GetVolume"),
    ("llist", "volumeid=", "GetVolume"),
    ("llist", "mediaid=", "GetVolume"),
    ("messages", "", "GetMessages"),
    ("memory", "", "GetMemory"),
    ("mount", "", "Mount"),
    ("move", "", "MoveVolumes"),
    ("prune", "files", "PruneFiles"),
    ("prune", "jobs", "PruneJobs"),
    ("prune", "volume", "PruneVolume"),
    ("prune", "stats", "PruneStats"),
    ("prune", "directory", "PruneDirectory"),
    ("purge", "files", "PurgeFiles"),
    ("purge", "jobs", "PurgeJobs"),
    ("purge", "volume", "PurgeVolume"),
    ("purge", "quota", "PurgeQuota"),
    ("quit", "", "Quit"),
    ("query", "", "Query"),
    ("restore", "", "Restore"),
    ("restore", "interaktive Dateiauswahl", "StartRestoreSession"),
    ("relabel", "", "Relabel"),
    ("release", "", "Release"),
    ("reload", "", "Reload"),
    ("rerun", "", "Rerun"),
    ("resolve", "", "Resolve"),
    ("run", "", "Run"),
    ("status", "all", "GetStatusAll"),
    ("status", "dir=", "GetDirectorStatus"),
    ("status", "dir", "GetDirectorStatus"),
    ("status", "director", "GetDirectorStatus"),
    ("status", "days=", "GetDirectorStatus"),
    ("status", "scheduler", "GetSchedulerStatus"),
    ("status", "schedule=", "GetSchedulerStatus"),
    ("status", "client=", "GetClientStatus"),
    ("status", "storage=", "GetStorageStatus"),
    ("status", "job=", "GetJobStatus"),
    ("status", "subscriptions", "GetSubscriptions"),
    ("status", "configuration", "GetConfigurationStatus"),
    ("setbandwidth", "", "SetBandwidth"),
    ("setdebug", "", "SetDebug"),
    ("setdevice", "", "SetDevice"),
    ("setip", "", "SetIp"),
    ("show", "catalog=", "ShowCatalogs"),
    ("show", "client=", "ShowClients"),
    ("show", "console=", "ShowConsoles"),
    ("show", "director=", "ShowDirectors"),
    ("show", "fileset=", "ShowFilesets"),
    ("show", "jobdefs=", "ShowJobdefs"),
    ("show", "job=", "ShowJobs"),
    ("show", "message=", "ShowMessages"),
    ("show", "pool=", "ShowPools"),
    ("show", "profile=", "ShowProfiles"),
    ("show", "schedule=", "ShowSchedules"),
    ("show", "storage=", "ShowStorages"),
    ("show", "user=", "ShowUsers"),
    ("show", "counter=", "ShowCounters"),
    ("show", "catalog", "ShowCatalogs"),
    ("show", "clients", "ShowClients"),
    ("show", "consoles", "ShowConsoles"),
    ("show", "directors", "ShowDirectors"),
    ("show", "filesets", "ShowFilesets"),
    ("show", "jobdefs", "ShowJobdefs"),
    ("show", "jobs", "ShowJobs"),
    ("show", "messages", "ShowMessages"),
    ("show", "pools", "ShowPools"),
    ("show", "profiles", "ShowProfiles"),
    ("show", "schedules", "ShowSchedules"),
    ("show", "storages", "ShowStorages"),
    ("show", "users", "ShowUsers"),
    ("show", "counters", "ShowCounters"),
    ("show", "disabled", "ShowDisabled"),
    ("show", "all", "ShowAll"),
    ("sqlquery", "", "SqlQuery"),
    ("time", "", "GetTime"),
    ("trace", "", "Trace"),
    ("truncate", "", "Truncate"),
    ("unmount", "", "Unmount"),
    ("umount", "", "Unmount"),
    ("update", "volume=", "UpdateVolume"),
    ("update", "pool=", "UpdatePool"),
    ("update", "slots", "UpdateSlots"),
    ("update", "jobid=", "UpdateJob"),
    ("update", "stats", "UpdateStats"),
    ("use", "", "UseCatalog"),
    ("var", "", "ExpandVariables"),
    ("version", "", "GetVersion"),
    ("wait", "", "Wait"),
    ("whoami", "", "Whoami"),
    // Onesimus extensions
    (".acl", "list", "DotAclList"),
    (".acl", "set", "DotAclSet"),
    (".acl", "delete", "DotAclDelete"),
    (".acl", "clear", "DotAclClear"),
    (".acl", "copy", "DotAclCopy"),
    (".grpc", "", "DotGrpc"),
    (".ovfs_latest", "", "DotOvfsLatest"),
    (".ovfs_ls", "", "DotOvfsLs"),
    (".ovfs_status", "", "DotOvfsStatus"),
    (".ovfs_tree", "", "DotOvfsTree"),
    (".ovfs_versions", "", "DotOvfsVersions"),
    (".profile", "list", "DotProfileList"),
    (".profile", "create", "DotProfileCreate"),
    (".profile", "set", "DotProfileSet"),
    (".profile", "delete", "DotProfileDelete"),
    (".profile", "assign", "DotProfileAssign"),
    (".profile", "unassign", "DotProfileUnassign"),
    (".sqlcount", "", "DotSqlCount"),
    (".sqlschema", "", "DotSqlSchema"),
    (".sqltables", "", "DotSqlTables"),
    (".storage", "", "DotStorages"),
];

/// Restore file-selection commands (ua_tree.cc) -> RPC
pub const RESTORE_TREE: &[(&str, &str)] = &[
    ("abort", "RestoreTreeAbort"),
    ("add", "RestoreTreeMark"),
    ("cd", "RestoreTreeCd"),
    ("count", "RestoreTreeCount"),
    ("delete", "RestoreTreeUnmark"),
    ("dir", "RestoreTreeLs"),
    (".dir", "RestoreTreeLs"),
    ("done", "RestoreTreeDone"),
    ("estimate", "RestoreTreeEstimate"),
    ("exit", "RestoreTreeDone"),
    ("find", "RestoreTreeFind"),
    ("help", "RestoreTreeHelp"),
    ("ls", "RestoreTreeLs"),
    (".ls", "RestoreTreeLs"),
    (".lsdir", "RestoreTreeLsDir"),
    ("lsmark", "RestoreTreeLsMark"),
    (".lsmark", "RestoreTreeLsMark"),
    ("mark", "RestoreTreeMark"),
    ("markdir", "RestoreTreeMarkDir"),
    ("pwd", "RestoreTreePwd"),
    (".pwd", "RestoreTreePwd"),
    ("unmark", "RestoreTreeUnmark"),
    ("unmarkdir", "RestoreTreeUnmarkDir"),
    ("quit", "RestoreTreeAbort"),
    (".help", "RestoreTreeHelp"),
    ("?", "RestoreTreeHelp"),
];

/// RPCs that do not apply to the typed API (FAILED_PRECONDITION in the
/// development build) and why.
pub const NOT_APPLICABLE: &[(&str, &str)] = &[
    ("Gui", "gui on/off only changes how a bconsole session prompts; the typed API never prompts"),
    ("DotApi", "the output format of the typed API is fixed by the RPC"),
    ("DotGrpc", "the output format of the typed API is fixed by the RPC"),
    ("SetIp", "setip takes the address of a BNET console connection; the gRPC path has none"),
];

/// RPCs that end the session (session token invalid afterwards).
pub const LOGOUT: &[&str] = &["Quit", "Exit", "DotQuit"];

/// Implementation state of an RPC for the documentation (independent of the
/// build: read RPCs everywhere, all others in the development build).
pub fn state(rpc: &str) -> &'static str {
    if super::api::READ_IMPLEMENTED.iter().any(|(name, _)| *name == rpc) {
        "implementiert"
    } else if NOT_APPLICABLE.iter().any(|(name, _)| *name == rpc) {
        "nicht anwendbar (Entwicklungs-Build: FAILED_PRECONDITION)"
    } else {
        "Entwicklungs-Build (`grpc-development`)"
    }
}

/// Markdown coverage table (docs/wiki/grpc-api.md).
pub fn markdown_table() -> String {
    let mut out = String::from("| Befehl | Unterbefehl | RPC | Stand |\n|---|---|---|---|\n");
    let cell = |s: &str| if s.is_empty() { "–".to_string() } else { format!("`{}`", s.replace('|', "\\|")) };
    for (command, sub, rpc) in COVERAGE {
        out.push_str(&format!("| `{command}` | {} | `{rpc}` | {} |\n", cell(sub), state(rpc)));
    }
    for (command, rpc) in RESTORE_TREE {
        out.push_str(&format!("| Restore-Baum: `{command}` | – | `{rpc}` | {} |\n", state(rpc)));
    }
    out
}

/// Subcommand keywords of a usage string: top-level alternatives (`|`
/// outside brackets), first word, `key=` for `key=<value>`.
pub fn usage_subcommands(usage: &str) -> Vec<String> {
    let mut usage = usage.trim();
    if usage.starts_with('[') && usage.ends_with(']') {
        // one bracket around everything (delete)
        let inner = &usage[1..usage.len() - 1];
        let mut depth = 0i32;
        let balanced = inner.chars().all(|c| {
            match c { '[' | '(' => depth += 1, ']' | ')' => depth -= 1, _ => {} }
            depth >= 0
        });
        if balanced && depth == 0 {
            usage = inner;
        }
    }
    let mut alternatives = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for c in usage.chars() {
        match c {
            '[' | '(' => depth += 1,
            // upstream usage strings are not always balanced (purge)
            ']' | ')' => depth = (depth - 1).max(0),
            '|' if depth == 0 => {
                alternatives.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    alternatives.push(current);
    alternatives.iter().filter_map(|alt| {
        let word = alt.split_whitespace().next()?.trim_start_matches(['[', '(']);
        let word = word.split('[').next().unwrap_or("");
        let word = match word.split_once('=') {
            Some((key, _)) => format!("{key}="),
            None => word.trim_end_matches([']', ')']).to_string(),
        };
        (!word.is_empty()).then(|| word.to_ascii_lowercase())
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use bareos_proto_rs::typed::{registry, DIRECTOR_API_SERVICE};
    use serde_json::Value;

    const INVENTORY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/wiki/commands/bareos-director-commands.json");
    const DOC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/wiki/grpc-api.md");
    const BEGIN: &str = "<!-- coverage:begin (generiert, crates/onesimus-dird/src/grpc/coverage.rs) -->";
    const END: &str = "<!-- coverage:end -->";

    /// Commands whose usage lists subcommands; (command, modifiers that are
    /// not subcommands, subcommands missing in the usage text)
    const WITH_SUBCOMMANDS: &[(&str, &[&str], &[&str])] = &[
        ("list", &["current", "enabled", "limit="], &[]),
        ("llist", &["current", "enabled", "limit="], &[]),
        ("show", &[], &["users", "user=", "counters", "counter="]),
        ("status", &[], &["dir"]),
        ("update", &[], &["slots"]),
        ("prune", &[], &[]),
        ("purge", &[], &[]),
        ("delete", &[], &[]),
        (".status", &[], &[]),
    ];

    fn inventory() -> Value {
        serde_json::from_str(&std::fs::read_to_string(INVENTORY).unwrap()).unwrap()
    }

    #[test]
    fn usage_parser_finds_subcommands() {
        assert_eq!(usage_subcommands("files [client=<client>] [yes] | jobs [pool=<pool>] | volume [all] [=volume]"),
                   ["files", "jobs", "volume"]);
        assert_eq!(usage_subcommands("[volume=<vol-name> [yes] | volume pool=<pool-name> | jobid=<jobid>]"),
                   ["volume=", "volume", "jobid="]);
        assert_eq!(usage_subcommands("[files [job=<job>]] | [volume[=<volume>] [storage=<storage>]] | [quota]"),
                   ["files", "volume", "quota"]);
    }

    #[test]
    fn every_command_and_subcommand_has_an_rpc() {
        let inventory = inventory();
        let rows: BTreeSet<(String, String)> = COVERAGE.iter().map(|(c, s, _)| (c.to_string(), s.to_string())).collect();
        let commands: BTreeSet<&str> = COVERAGE.iter().map(|(c, _, _)| *c).collect();
        let mut missing = Vec::new();

        let listed = inventory["commands"].as_array().unwrap();
        assert_eq!(listed.len(), 88, "inventory should list the 88 Director commands");
        for command in listed.iter().chain(inventory["onesimus_extensions"].as_array().unwrap()) {
            let name = command["name"].as_str().unwrap();
            if !commands.contains(name) {
                missing.push(format!("command {name}"));
            }
        }
        for command in listed {
            let name = command["name"].as_str().unwrap();
            if name == "configure" {
                for kind in command["configure_add_resource_types"].as_array().unwrap() {
                    let sub = format!("add {}", kind.as_str().unwrap());
                    if !rows.contains(&(name.to_string(), sub.clone())) {
                        missing.push(format!("configure {sub}"));
                    }
                }
                if !rows.contains(&(name.to_string(), "export".to_string())) {
                    missing.push("configure export".into());
                }
                continue;
            }
            let Some((_, ignore, extra)) = WITH_SUBCOMMANDS.iter().find(|(c, _, _)| *c == name) else { continue };
            let usage = command["usage"].as_str().unwrap_or("");
            let mut expected: BTreeSet<String> = usage_subcommands(usage).into_iter()
                .filter(|s| !ignore.contains(&s.as_str())).collect();
            expected.extend(extra.iter().map(|s| s.to_string()));
            assert!(expected.len() > 2, "{name}: no subcommands parsed from {usage:?}");
            for sub in expected {
                if !rows.contains(&(name.to_string(), sub.clone())) {
                    missing.push(format!("{name} {sub}"));
                }
            }
        }
        for command in inventory["restore_file_selection_commands"].as_array().unwrap() {
            let name = command["name"].as_str().unwrap();
            if !RESTORE_TREE.iter().any(|(c, _)| *c == name) {
                missing.push(format!("restore tree {name}"));
            }
        }
        assert!(missing.is_empty(), "no RPC for: {missing:?}");
    }

    #[test]
    fn coverage_rpcs_match_the_service() {
        let rpcs: BTreeSet<String> = registry().rpc_names(DIRECTOR_API_SERVICE).into_iter().collect();
        assert!(rpcs.len() > 150);
        let used: BTreeSet<String> = COVERAGE.iter().map(|(_, _, r)| r.to_string())
            .chain(RESTORE_TREE.iter().map(|(_, r)| r.to_string())).collect();
        let unknown: Vec<_> = used.difference(&rpcs).collect();
        assert!(unknown.is_empty(), "rows name RPCs missing in director_api.proto: {unknown:?}");
        let unused: Vec<_> = rpcs.difference(&used).collect();
        assert!(unused.is_empty(), "RPCs without a command row: {unused:?}");
        // every implemented RPC has a spec, a trait method of the same name
        // and exists in the proto; every spec is implemented
        for (rpc, method) in super::super::api::READ_IMPLEMENTED {
            assert!(rpcs.contains(*rpc), "{rpc}");
            assert!(super::super::api::find_rpc(rpc).is_some(), "{rpc} has no ReadRpc spec");
            let snake: String = rpc.chars().enumerate().flat_map(|(i, c)| {
                let lower = c.to_ascii_lowercase();
                if c.is_ascii_uppercase() && i > 0 { vec!['_', lower] } else { vec![lower] }
            }).collect();
            assert_eq!(&snake, method, "{rpc}");
        }
        // every other RPC is implemented in the development build (S-35)
        let read: BTreeSet<&str> = super::super::api::READ_IMPLEMENTED.iter().map(|(r, _)| *r).collect();
        let dev: BTreeSet<&str> = super::super::api::DEV_IMPLEMENTED.iter().map(|(r, _)| *r).collect();
        assert!(read.is_disjoint(&dev));
        let all: BTreeSet<String> = read.union(&dev).map(|r| r.to_string()).collect();
        assert_eq!(all, rpcs, "every RPC must be a read RPC or a development-build RPC");
        for (rpc, _) in NOT_APPLICABLE {
            assert!(dev.contains(rpc), "{rpc}");
        }
        for (rpc, method) in super::super::api::DEV_IMPLEMENTED {
            let snake: String = rpc.chars().enumerate().flat_map(|(i, c)| {
                let lower = c.to_ascii_lowercase();
                if c.is_ascii_uppercase() && i > 0 { vec!['_', lower] } else { vec![lower] }
            }).collect();
            assert_eq!(&snake, method, "{rpc}");
        }
        for spec in super::super::api::read_rpcs() {
            assert!(super::super::api::READ_IMPLEMENTED.iter().any(|(r, _)| *r == spec.rpc), "{} not wired", spec.rpc);
        }
    }

    /// Replace the text between `begin` and `end`; None when unchanged.
    fn sync_block(doc: &str, begin: &str, end: &str, content: &str) -> Option<String> {
        let (head, rest) = doc.split_once(begin).unwrap_or_else(|| panic!("marker {begin} missing in grpc-api.md"));
        let (current, tail) = rest.split_once(end).unwrap_or_else(|| panic!("marker {end} missing"));
        (current.trim() != content.trim()).then(|| format!("{head}{begin}\n{content}{end}{tail}"))
    }

    #[test]
    fn wiki_tables_are_current() {
        let doc = std::fs::read_to_string(DOC).unwrap_or_default();
        let after_coverage = sync_block(&doc, BEGIN, END, &markdown_table());
        let base = after_coverage.clone().unwrap_or(doc);
        let after_rpcs = sync_block(&base, RPCS_BEGIN, RPCS_END, &rpc_table());
        if after_coverage.is_some() || after_rpcs.is_some() {
            if std::env::var_os("UPDATE_GRPC_API_DOC").is_some() {
                std::fs::write(DOC, after_rpcs.unwrap_or(base)).unwrap();
            } else {
                panic!("docs/wiki/grpc-api.md tables outdated; run with UPDATE_GRPC_API_DOC=1");
            }
        }
    }

    const RPCS_BEGIN: &str = "<!-- rpcs:begin (generiert, crates/onesimus-dird/src/grpc/coverage.rs) -->";
    const RPCS_END: &str = "<!-- rpcs:end -->";

    /// All RPCs with request and response type (from the descriptors).
    fn rpc_table() -> String {
        let service = registry().service(DIRECTOR_API_SERVICE).unwrap();
        let short = |t: &str| t.trim_start_matches(".bareos.director.").to_string();
        let mut out = String::from("| RPC | Request | Response | Stand |\n|---|---|---|---|\n");
        for method in &service.method {
            out.push_str(&format!("| `{}` | `{}` | `{}` | {} |\n", method.name(),
                short(method.input_type()), short(method.output_type()), state(method.name())));
        }
        out
    }
}
