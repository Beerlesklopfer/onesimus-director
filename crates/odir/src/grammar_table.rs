//! What every registered console command accepts: the grammar table of
//! [`crate::command_grammar`].
//!
//! One function per command family; [`describe`] maps a registry name to
//! its root node (word, aliases, dot flag and, when empty, the description
//! are filled in from the registry). The Bareos commands follow the usage
//! strings of `ua_cmds.cc` ([`crate::ua_commands_table`]) and the keywords
//! the odir handlers read; Onesimus extensions are marked
//! [`Node::onesimus`]. `configure add <type>` is derived from the generated
//! Bareos usage ([`crate::commands::configure_usage::USAGE`]) and
//! `configure update` from the directive tables, so they follow the
//! configuration schema.
//!
//! Keep this table in step with the handlers: the test
//! `grammar_accepts_the_parity_cases` walks every parity case line.

#![warn(clippy::missing_docs_in_private_items)]

use crate::command_grammar::{boolean, enm, int, kw, pos, res, sub, val, vsub, ArgType, GrammarArg, Node, Values, COMMANDS_SOURCE};
use crate::commands::show_items::{Item, Ty};

// Value sources (dot-commands whose output lines are the values).
/// Value source `.clients`.
const CLIENTS: &str = ".clients";
/// Value source `.jobs`.
const JOBS: &str = ".jobs";
/// Value source `.pools`.
const POOLS: &str = ".pools";
/// Value source `.storages`.
const STORAGES: &str = ".storages";
/// Value source `.filesets`.
const FILESETS: &str = ".filesets";
/// Value source `.schedule`.
const SCHEDULES: &str = ".schedule";
/// Value source `.catalogs`.
const CATALOGS: &str = ".catalogs";
/// Value source `.consoles`.
const CONSOLES: &str = ".consoles";
/// Value source `.profiles`.
const PROFILES: &str = ".profiles";
/// Value source `.media`.
const MEDIA: &str = ".media";
/// Value source `.types`.
const TYPES: &str = ".types";
/// Value source `.msgs`.
const MESSAGES: &str = ".msgs";
/// Value source `.users`.
const USERS: &str = ".users";
/// Value source `.jobdefs`.
const JOBDEFS: &str = ".jobdefs";

/// Backup levels (`level=`).
const LEVEL_VALUES: &[&str] = &["Full", "Incremental", "Differential", "VirtualFull", "Base", "Since",
    "VolumeToCatalog", "DiskToCatalog", "Catalog", "Data", "InitCatalog"];
/// Volume status values `update volume volstatus=` accepts.
const VOLSTATUS_UPDATE: &[&str] = &["Append", "Archive", "Disabled", "Full", "Used", "Cleaning", "Read-Only", "Recycle", "Purged", "Error"];
/// Replace modes of run and restore.
const REPLACE: &[&str] = &["always", "ifnewer", "ifolder", "never"];
/// `on|off` of autodisplay, automount, gui, trace.
const ON_OFF: &[&str] = &["on", "off"];
/// Resource types `configure update|delete` and `.config` edit.
const EDITABLE: &[&str] = &["client", "job", "jobdefs", "schedule", "fileset", "pool", "storage"];

/// The root node of a registered command, None for a name without entry.
pub fn describe(name: &str) -> Option<Node> {
    Some(match name {
        // --- basic ---------------------------------------------------------------
        "version" => sub(name, "").long(&["Prints the Director's name, version and build date."]),
        "time" => sub(name, "").long(&["Prints the Director's current date and time."]),
        "quit" => sub(name, "").long(&["Ends the console session."]),
        "help" => sub(name, "").args(vec![pos("command", "Command to describe").source(COMMANDS_SOURCE)])
            .long(&["Without argument a table of all commands the console may run; with a command its usage."])
            .example("help purge", "usage of purge"),
        "whoami" => sub(name, "").long(&["Prints the name of the console of this session."]),
        "messages" => sub(name, "").long(&["Prints the queued Director and job messages of this console."]),
        "memory" => sub(name, ""),
        "reload" => sub(name, "").long(&["Re-reads the configuration; a configuration with errors is refused and the running one stays."])
            .see(&["configure"]),
        "setip" => sub(name, ""),
        "sqlquery" => sub(name, "").long(&["Reads SQL statements interactively and prints their results; an empty line ends the input."]),
        "query" => sub(name, "").long(&["Selects one of the predefined catalog queries and asks for its parameters."]),
        "autodisplay" | "automount" | "gui" | "trace" => sub(name, "").args(vec![pos("on|off", "Switch on or off").values(ON_OFF)]),
        "var" => sub(name, "").free_text("text", "Text with variables to expand"),
        "use" => sub(name, "").args(vec![res("catalog", "Catalog name", CATALOGS), pos("catalog-name", "Catalog name without key").source(CATALOGS)]),
        "wait" => sub(name, "").args(vec![
            int("jobid", "Job ID"), val("jobname", "Running job name prefix"), val("job", "Same as jobname="),
            val("ujobid", "Unique running job name"),
            kw("mount", "Wait for a media or mount request"), int("timeout", "Mount wait timeout in seconds"), catalog(),
        ]),
        "status" => status(),
        ".status" => dot_status(),
        "run" => run(),
        "rerun" => sub(name, "").args(vec![
            val("jobid", "Job ID").list(), int("since_jobid", "Rerun failed jobs since this JobId"),
            int("until_jobid", "Up to this JobId"), int("days", "Failed jobs of the last days"),
            int("hours", "Failed jobs of the last hours"), kw("yes", "Do not ask"),
        ]),
        "cancel" => sub(name, "").args(vec![
            res("storage", "Jobs of a storage", STORAGES), sd(), val("jobid", "Job ID").list(), res("job", "Job name", JOBS),
            val("ujobid", "Unique job name"), val("state", "Jobs in this state (created, blocked, waiting, running)"),
            kw("all", "Every job"), kw("yes", "Do not ask"),
        ]).long(&["Cancels running or waiting jobs; without a selection the Director asks for one."]),
        "restore" => restore(),
        "enable" | "disable" => sub(name, "").args(vec![
            res("job", "Job name", JOBS), res("client", "Client name", CLIENTS),
            res("schedule", "Schedule name", SCHEDULES), res("storage", "Storage name", STORAGES),
        ]),
        "estimate" => sub(name, "").args(vec![
            res("job", "Job name", JOBS), res("client", "Client name", CLIENTS), fd(), res("fileset", "FileSet name", FILESETS),
            enm("level", "Backup level", LEVEL_VALUES), boolean("accurate", "Accurate mode"), kw("listing", "List every file"),
            catalog(),
        ]),
        "list" => list(),
        "show" => show(),
        "label" => sub(name, "").args(label_args(false)),
        "relabel" => sub(name, "").args(vec![
            res("storage", "Storage name", STORAGES), res("oldvolume", "Current volume name", MEDIA),
            val("volume", "New volume name"), res("pool", "Pool name", POOLS), int("slot", "Slot"),
            int("drive", "Drive number"), kw("encrypt", "Encrypt the volume"), kw("yes", "Do not ask"),
        ]),
        "mount" => sub(name, "").args(vec![
            res("storage", "Storage name", STORAGES), sd(), pos("storage-name", "Storage name without key").source(STORAGES), int("slot", "Slot"), int("drive", "Drive number"),
            val("device", "Device name"), int("jobid", "Storage of a job"), res("job", "Storage of a job", JOBS),
            val("ujobid", "Storage of a job"),
        ]),
        "unmount" | "release" => sub(name, "").args(vec![
            res("storage", "Storage name", STORAGES), sd(), pos("storage-name", "Storage name without key").source(STORAGES), int("drive", "Drive number"), val("device", "Device name"),
            kw("alldrives", "Every drive of the autochanger"), int("jobid", "Storage of a job"),
            res("job", "Storage of a job", JOBS), val("ujobid", "Storage of a job"),
        ]),
        "update" => update(),
        "delete" => delete(),
        "prune" => prune(),
        "purge" => purge(),
        "truncate" => sub(name, "").args(vec![
            enm("volstatus", "Only Purged volumes", &["Purged"]).req(), res("storage", "Storage name", STORAGES),
            res("pool", "Pool name", POOLS), res("volume", "Volume name", MEDIA).list(), int("drive", "Drive number"),
            kw("yes", "Do not ask"), catalog(),
        ]),
        "configure" => configure(),
        "setdebug" => sub(name, "").args(vec![
            int("level", "Debug level"), enm("trace", "Trace flag", &["-1", "0", "1"]),
            enm("timestamp", "Timestamp flag", &["-1", "0", "1"]), enm("hangup", "FD hangup flag", &["-1", "0", "1"]),
            val("tags", "Debug tags"), res("client", "File Daemon", CLIENTS), res("storage", "Storage Daemon", STORAGES),
            kw("dir", "The Director"), kw("director", "The Director"), kw("all", "Every daemon"),
        ]),
        "setbandwidth" => sub(name, "").args(vec![
            res("client", "Client name", CLIENTS), fd(), res("storage", "Storage name", STORAGES), sd(), val("jobid", "Job ID").list(),
            res("job", "Job name", JOBS), val("ujobid", "Unique job name"), val("state", "Jobs in this state"),
            kw("all", "Every running job"), int("limit", "KiB/s (0: unlimited)"), kw("yes", "Do not ask"),
        ]),
        "setdevice" => sub(name, "").args(vec![
            res("storage", "Storage name", STORAGES), val("device", "Device name"), boolean("autoselect", "Autoselect flag"),
        ]),
        "resolve" => sub(name, "").args(vec![
            res("client", "Resolve on this client", CLIENTS), fd(), res("storage", "Resolve on this storage", STORAGES), sd(),
            pos("hostname", "Host name to resolve").repeat(),
        ]),
        "create" => sub(name, "").args(vec![res("pool", "Pool name", POOLS)]),
        "add" => sub(name, "").args(vec![
            res("pool", "Pool name", POOLS), res("storage", "Storage name", STORAGES), int("jobid", "Storage of a job"),
            val("volume", "Volume name"), int("slot", "Slot"), int("drive", "Drive number"),
        ]),
        "import" | "export" => sub(name, "").args(vec![
            res("storage", "Storage name", STORAGES), val("srcslots", "Source slots (list or ranges)"),
            val("dstslots", "Destination slots (list or ranges)"), res("volume", "Volume names", MEDIA).list(),
            kw("scan", "Scan the volumes"), int("drive", "Drive number"),
        ]),
        "move" => sub(name, "").args(vec![
            res("storage", "Storage name", STORAGES), val("srcslots", "Source slots"), val("dstslots", "Destination slots"),
            int("drive", "Drive number"),
        ]),

        // --- dot commands ----------------------------------------------------------
        ".help" => sub(name, "").args(vec![
            kw("all", "Every command"), val("item", "One command").source(COMMANDS_SOURCE), kw("full", "With arguments"),
        ]),
        ".quit" | ".messages" | ".msgs" | ".catalogs" | ".consoles" | ".jobdefs" | ".profiles" | ".locations"
        | ".media" | ".mediatypes" | ".users" | ".filesets" | ".types" | ".volstatus" | ".actiononpurge" => sub(name, ""),
        "." => sub(name, "").free_text("text", "Ignored"),
        ".clients" | ".storage" | ".schedule" => sub(name, "").args(enabled_group()),
        ".jobs" => sub(name, "").args({
            let mut args = vec![res("type", "Job type", TYPES)];
            args.extend(enabled_group());
            args
        }),
        ".pools" => sub(name, "").args(vec![val("type", "Pool type").values(&["Backup", "Copy", "Cloned", "Archive", "Migration", "Scratch"])]),
        ".defaults" => sub(name, "").args(vec![
            res("job", "Job name", JOBS), res("client", "Client name", CLIENTS), res("storage", "Storage name", STORAGES),
            res("pool", "Pool name", POOLS),
        ]),
        ".jobstatus" => sub(name, "").value(val("jobstatus", "One JobStatus letter").optional_value()).args(vec![catalog()]),
        ".authorized" => sub(name, "").args(vec![
            res("job", "Job name", JOBS), res("client", "Client name", CLIENTS), res("storage", "Storage name", STORAGES),
            res("schedule", "Schedule name", SCHEDULES), res("pool", "Pool name", POOLS),
            val("cmd", "Command name").source(COMMANDS_SOURCE), res("fileset", "FileSet name", FILESETS),
            res("catalog", "Catalog name", CATALOGS),
        ]),
        ".levels" => sub(name, "").args(vec![pos("type", "Job type").source(TYPES)]),
        ".api" => sub(name, "").args(vec![
            pos("mode", "API mode").values(&["0", "1", "2", "off", "on", "json"]), boolean("compact", "Compact JSON"),
        ]),
        ".grpc" => sub(name, "").args(vec![
            kw("on", "Switch on").group("switch"), kw("off", "Switch off").group("switch"),
            enm("format", "Output format", &["json", "text"]), boolean("compact", "Compact JSON"),
        ]).long(&["Switches the console connection to the gRPC message format, or shows its state."]),
        ".sql" => sub(name, "").args(vec![val("query", "SQL statement").req(), res("catalog", "Catalog name", CATALOGS)]),
        ".sqltables" => sub(name, "").args(vec![res("catalog", "Catalog name", CATALOGS)])
            .long(&["Lists the tables of the catalog database."]),
        ".sqlschema" | ".sqlcount" => sub(name, "").args(vec![val("table", "Table name"), res("catalog", "Catalog name", CATALOGS)]),
        ".acl" => acl(),
        ".bvfs_update" => sub(name, "").args(vec![val("jobid", "Job IDs").list(), res("catalog", "Catalog name", CATALOGS)]),
        ".bvfs_cleanup" => sub(name, "").args(vec![val("path", "Handle (table name)").req()]),
        ".bvfs_clear_cache" => sub(name, "").args(vec![kw("yes", "Confirm")]),
        ".bvfs_get_jobids" => sub(name, "").args(vec![
            int("jobid", "Job ID"), val("ujobid", "Unique job name"), kw("all", "Every job of the chain"),
        ]),
        ".bvfs_lsdirs" | ".bvfs_lsfiles" => sub(name, "").args(vec![
            val("jobid", "Job IDs").list(), val("path", "Directory"), int("pathid", "Path ID"),
            int("limit", "Most rows"), int("offset", "Rows to skip"), val("pattern", "Name filter (regular expression)"),
            val("filename", "File name"),
        ]),
        ".bvfs_versions" => sub(name, "").args(vec![
            val("jobid", "Job ID"), res("client", "Client name", CLIENTS), int("pathid", "Path ID"), val("path", "Directory"),
            val("filename", "File name"), val("fname", "File name"), val("name", "File name"), kw("copies", "Include copies"),
            kw("versions", "Every version"), int("limit", "Most rows"), int("offset", "Rows to skip"),
        ]),
        ".bvfs_restore" => sub(name, "").args(vec![
            val("path", "Handle (table name)").req(), val("jobid", "Job IDs").list().req(), val("fileid", "File IDs").list(),
            val("dirid", "Directory IDs").list(), val("hardlink", "JobId,FileIndex pairs").list(), kw("useemptypath", "Use an empty path"),
        ]),
        ".config" => config_crud(),
        ".ovfs_status" => sub(name, "").args(vec![kw("refresh", "Check the extension again now"), json_kw()]),
        ".ovfs_latest" => sub(name, "").args(vec![res("client", "Client name", CLIENTS).req(), res("fileset", "FileSet name", FILESETS), json_kw()]),
        ".ovfs_tree" => sub(name, "").args(vec![
            val("jobid", "Job ID list").list(), res("client", "Client name (latest state)", CLIENTS), res("fileset", "FileSet name", FILESETS),
            val("root", "Directory").default("/"), int("depth", "Levels below root").default("1"),
            int("max_entries", "Most nodes in the result").default("100000"), json_kw(),
        ]),
        ".ovfs_ls" => sub(name, "").args(vec![
            val("jobid", "Job ID list").list(), res("client", "Client name (latest state)", CLIENTS), res("fileset", "FileSet name", FILESETS),
            val("path", "Directory").req(), json_kw(),
        ]),
        ".ovfs_versions" => sub(name, "").args(vec![
            res("client", "Client name", CLIENTS).req(), val("path", "Directory").req(), val("name", "File name ('' = the directory)"),
            kw("copies", "Include copy jobs"), json_kw(),
        ]),
        ".grammar" => sub(name, "Print the console grammar (Onesimus)").args(vec![
            val("command", "Only this command").source(COMMANDS_SOURCE),
        ]).long(&[
            "Prints the grammar of every command the console may run (CommandACL): subcommand words, keywords, key=value \
             arguments, value sources, repeatable keys and exclusive groups. In API mode 2 the reply is the JSON object \
             \"grammar\" (schema 1), otherwise one usage line per command and subcommand.",
            "oconsole loads it at connect for completion and hints. The Bareos Director does not know the command and \
             answers \"is an invalid command\"; clients then fall back to .help.",
        ]).example(".api 2", "switch to JSON").example(".grammar", "the grammar of every permitted command")
            .example(".grammar command=purge", "only purge").see(&[".help", "help"]),
        _ => return None,
    })
}

/// `catalog=`: every command that opens the catalog (OpenClientDb) accepts it.
fn catalog() -> GrammarArg {
    res("catalog", "Catalog name", CATALOGS)
}

/// `sd=`: Bareos accepts it for `storage=` wherever a storage is selected.
fn sd() -> GrammarArg {
    res("sd", "Storage name (as storage=)", STORAGES)
}

/// `fd=`: Bareos accepts it for `client=` wherever a client is selected.
fn fd() -> GrammarArg {
    res("fd", "Client name (as client=)", CLIENTS)
}

/// `json`: the Onesimus commands print their JSON object as text.
fn json_kw() -> GrammarArg {
    kw("json", "Print the JSON object as text")
}

/// `enabled | disabled` of the resource listings.
fn enabled_group() -> Vec<GrammarArg> {
    vec![kw("enabled", "Only enabled resources").group("state"), kw("disabled", "Only disabled resources").group("state")]
}

/// `status`: the first argument selects the daemon (StatusCmd).
fn status() -> Node {
    sub("status", "").long(&[
        "Shows the state of the Director, a client, a storage or the scheduler. Without argument the Director asks which daemon.",
    ]).subs(vec![
        sub("all", "Director, every storage and every client"),
        vsub("director", "Director status", val("director", "Director name").optional_value()).alias(&["dir"])
            .args(vec![int("days", "Scheduled jobs of the next days"), kw("verbose", "More detail")]),
        vsub("client", "Status of a File Daemon", res("client", "Client name", CLIENTS).optional_value()).alias(&["fd"]),
        vsub("storage", "Status of a Storage Daemon", res("storage", "Storage name", STORAGES).optional_value()).alias(&["sd"])
            .args(vec![kw("slots", "Autochanger slots"), int("drive", "Drive number"), val("device", "Device name")]),
        vsub("job", "Status of the storage of a job", res("job", "Job name", JOBS)),
        vsub("jobid", "Status of the storage of a job", int("jobid", "Job ID")),
        vsub("ujobid", "Status of the storage of a job", val("ujobid", "Unique job name")),
        vsub("scheduler", "Jobs the scheduler will start", res("schedule", "Schedule name", SCHEDULES).optional_value())
            .alias(&["schedule", "sched"])
            .args(vec![res("client", "Jobs of a client", CLIENTS), res("job", "One job", JOBS), int("days", "Days ahead"),
                res("schedule", "Schedule name", SCHEDULES)]),
        sub("subscriptions", "Subscription and unit counts").alias(&["subscription", "sub"]).args(vec![
            kw("detail", "Details"), kw("clients", "Per client"), kw("plugins", "Per plugin"), kw("all", "Everything"),
            kw("anonymize", "Anonymize names"), res("client", "One client", CLIENTS), kw("unknown", "Unknown clients"),
        ]),
        sub("configuration", "Configuration warnings").alias(&["config", "conf"]),
    ])
}

/// `.status dir|storage=|client=` with the part to show (DotStatusCmd).
fn dot_status() -> Node {
    let what = |words: &[&str]| words.iter().map(|w| kw(w, "Part of the status").group("part")).collect::<Vec<_>>();
    sub(".status", "").subs(vec![
        sub("dir", "Director").alias(&["director"]).args(what(&["current", "last", "header", "scheduled", "running", "terminated"])),
        vsub("storage", "Storage Daemon", res("storage", "Storage name", STORAGES))
            .args(what(&["header", "waitreservation", "devices", "volumes", "spooling", "running", "terminated"])),
        vsub("client", "File Daemon", res("client", "Client name", CLIENTS)).args(what(&["header", "terminated", "running"])),
    ])
}

/// `run`: the keywords of ua_run.cc (`fd`/`sd` as aliases of client/storage).
fn run() -> Node {
    sub("run", "").long(&[
        "Starts a job. Missing values come from the Job resource; without yes the Director shows the job and asks \
         yes, mod or no.",
    ]).args(vec![
        res("job", "Job name", JOBS).req(), res("client", "Client name", CLIENTS), res("fd", "Client name", CLIENTS),
        res("fileset", "FileSet name", FILESETS), enm("level", "Backup level", LEVEL_VALUES),
        res("storage", "Storage name", STORAGES), res("sd", "Storage name", STORAGES), val("where", "Restore prefix"),
        val("regexwhere", "Restore path rewrite"), val("bootstrap", "Bootstrap file"), enm("replace", "Replace mode", REPLACE),
        val("when", "Start time (YYYY-MM-DD HH:MM:SS)"), int("priority", "Priority"), res("verifyjob", "Job to verify", JOBS),
        val("files", "Files"), res("catalog", "Catalog name", CATALOGS), val("since", "Since time"), val("cloned", "Cloned job").optional_value(),
        val("verifylist", "Verify list"), res("migrationjob", "Migration job", JOBS), res("pool", "Pool name", POOLS),
        res("nextpool", "Next pool", POOLS), res("backupclient", "Backup client", CLIENTS),
        res("restoreclient", "Restore client", CLIENTS), val("pluginoptions", "Plugin options"),
        boolean("spooldata", "Spool data"), val("comment", "Comment"), boolean("ignoreduplicatecheck", "Ignore duplicate check"),
        boolean("accurate", "Accurate mode"), val("jobid", "Job IDs").list(), val("backupformat", "Backup format"),
        res("consolidatejob", "Consolidate job", JOBS), res("schedule", "Schedule name", SCHEDULES), kw("yes", "Do not ask"),
    ]).example("run job=BackupClient1 level=Full yes", "start a full backup without asking").see(&["rerun", "cancel", "status"])
}

/// `restore`: selection, target and the keywords of the file selection.
fn restore() -> Node {
    sub("restore", "").long(&[
        "Selects files from backups and starts a restore job. Without a selection the Director shows the selection menu; \
         `done` ends the file selection.",
    ]).args(vec![
        val("where", "Restore prefix"), res("client", "Client name", CLIENTS), res("storage", "Storage name", STORAGES),
        val("bootstrap", "Bootstrap file"), res("restorejob", "Restore job", JOBS), val("comment", "Comment"),
        val("jobid", "Job IDs").list(), res("fileset", "FileSet name", FILESETS), enm("replace", "Replace mode", REPLACE),
        val("pluginoptions", "Plugin options"), val("regexwhere", "Restore path rewrite"), val("fileregex", "Select files by regex"),
        res("restoreclient", "Restore client", CLIENTS), val("backupformat", "Backup format"), res("pool", "Pool name", POOLS),
        val("file", "File to restore").repeat(), val("directory", "Directory to restore").repeat(),
        val("before", "Most recent backup before this time").optional_value(), val("strip_prefix", "Strip prefix"),
        val("stripprefix", "Strip prefix"), val("add_prefix", "Add prefix"), val("addprefix", "Add prefix"),
        val("add_suffix", "Add suffix"), val("addsuffix", "Add suffix"), val("select", "Select").optional_value(),
        kw("current", "Most recent backup"), kw("copies", "Use copies"), kw("archive", "Use archive jobs"),
        kw("done", "End the selection"), kw("all", "Select every file"), res("job", "Job name", JOBS), kw("yes", "Do not ask"),
        catalog(),
    ]).example("restore client=fd select current all done yes where=/tmp/r", "restore everything of a client").see(&["run", ".bvfs_restore"])
}

/// Arguments of `label`.
fn label_args(_relabel: bool) -> Vec<GrammarArg> {
    vec![
        res("storage", "Storage name", STORAGES), sd(), val("volume", "Volume name"), res("pool", "Pool name", POOLS),
        val("slot", "Slot").list(), catalog(), val("slots", "Slots (list or ranges)"), int("drive", "Drive number"),
        kw("barcodes", "Use the barcodes as names"), kw("barcode", "Use the barcodes as names"), kw("encrypt", "Encrypt"),
        kw("yes", "Do not ask"), val("device", "Device name"),
    ]
}

/// `list`/`llist`: the subcommands of the Bareos usage, `reverse` (Onesimus) and the modifiers.
fn list() -> Node {
    let jobid = || int("jobid", "Job ID");
    let ujobid = || val("ujobid", "Unique job name");
    let job_filters = || vec![
        res("client", "Client name", CLIENTS), val("jobstatus", "Job status letters").list(), val("jobtype", "Job type letters").list(),
        val("joblevel", "Job level letters").list(), val("level", "Job level letters").list(), res("volume", "Volume name", MEDIA),
        res("pool", "Pool name", POOLS), int("days", "Of the last days"), int("hours", "Of the last hours"),
        kw("last", "Only the last run of each job"), kw("count", "Only count"), kw("reverse", "Newest first (Onesimus)"),
        res("job", "Job name", JOBS), val("jobname", "Job name"), jobid().list(), val("ujobid", "Unique job name"),
        res("fileset", "FileSet name", FILESETS),
    ];
    let mut job_named = job_filters();
    job_named.retain(|a| a.name != "job");
    sub("list", "").long(&[
        "Lists catalog records; llist prints every field of each record. `limit=` and `offset=` page through the result, \
         `current` limits to records of configured resources, `enabled`/`disabled` to resources in that state.",
    ]).args(vec![
        int("limit", "Most rows"), int("offset", "Rows to skip"), kw("current", "Only configured resources"),
        kw("enabled", "Only enabled").group("state"), kw("disabled", "Only disabled").group("state"),
        res("catalog", "Catalog name", CATALOGS),
    ]).subs(vec![
        sub("basefiles", "Base files of a job").args(vec![jobid(), ujobid()]),
        sub("backups", "Backups of a client").args(vec![
            res("client", "Client name", CLIENTS).req(), res("fileset", "FileSet name", FILESETS), val("jobstatus", "Job status letters"),
            enm("level", "Level", &["F", "I", "D"]).list(), enm("order", "Sort order", &["asc", "desc", "ascending", "descending"]),
            int("days", "Of the last days"), int("hours", "Of the last hours"), kw("reverse", "Newest first"),
        ]),
        vsub("client", "One client", res("client", "Client name", CLIENTS)),
        sub("clients", "Every client"),
        sub("copies", "Copies of jobs").args(vec![jobid().list()]),
        sub("files", "Files of a job").args(vec![jobid(), ujobid(), val("type", "File type")]),
        sub("filesets", "Every FileSet"),
        vsub("fileset", "One FileSet", res("fileset", "FileSet name", FILESETS).optional_value())
            .args(vec![int("filesetid", "FileSet ID"), jobid(), ujobid()]),
        sub("jobs", "Jobs").args(job_filters()),
        vsub("job", "Jobs of one name", res("job", "Job name", JOBS)).args(job_named.clone()),
        vsub("jobname", "Jobs of one name", res("jobname", "Job name", JOBS)).args(job_named),
        vsub("jobid", "One job", val("jobid", "Job IDs").list()),
        vsub("ujobid", "One job", val("ujobid", "Unique job name")),
        sub("joblog", "Log of a job").args(vec![jobid(), ujobid(), kw("count", "Only count"), kw("reverse", "Newest first")]),
        sub("jobmedia", "Volumes of a job").args(vec![jobid(), ujobid()]),
        sub("jobtotals", "Totals per job name"),
        sub("jobstatistics", "Statistics of a job").alias(&["jobstats"]).args(vec![jobid(), res("job", "Job name", JOBS)]),
        sub("log", "Director log").args(vec![res("client", "Client name", CLIENTS), kw("reverse", "Newest first")]),
        vsub("media", "Volumes", res("media", "Volume name", MEDIA).optional_value())
            .args(vec![jobid(), ujobid(), res("pool", "Pool name", POOLS), kw("all", "Every pool")]),
        sub("nextvolume", "Next volume of a job").alias(&["nextvol"])
            .args(vec![res("job", "Job name", JOBS), int("days", "Days ahead")]),
        sub("pools", "Every pool"),
        vsub("pool", "One pool", res("pool", "Pool name", POOLS)),
        vsub("poolid", "One pool", int("poolid", "Pool ID")),
        sub("storages", "Every storage"),
        vsub("storage", "One storage", res("storage", "Storage name", STORAGES)),
        sub("volumes", "Volumes").args(vec![
            jobid(), ujobid(), res("pool", "Pool name", POOLS), kw("all", "Every pool"), kw("count", "Only count"),
        ]),
        vsub("volume", "One volume", res("volume", "Volume name", MEDIA)),
        vsub("volumeid", "One volume", int("volumeid", "Media ID")),
        vsub("mediaid", "One volume", int("mediaid", "Media ID")),
        sub("schedules", "Not a catalog list: answered as unknown keyword (see show schedules)"),
    ]).example("list jobs client=fd last", "the last run of every job of a client")
        .example("list jobs reverse limit=10", "the ten newest jobs (Onesimus)")
        .example("llist volume=Full-0001", "every field of one volume")
        .see(&["show", "status", "query"])
}

/// `show`: any number of resource items in one line, `disabled` with its types.
fn show() -> Node {
    // Bareos show takes any number of items in one line (`show pool=Full client=fd`)
    let mut args = Vec::new();
    for (word, source) in [("catalog", Some(CATALOGS)), ("client", Some(CLIENTS)), ("console", Some(CONSOLES)),
        ("director", None), ("fileset", Some(FILESETS)), ("job", Some(JOBS)), ("message", Some(MESSAGES)),
        ("pool", Some(POOLS)), ("profile", Some(PROFILES)), ("schedule", Some(SCHEDULES)),
        ("storage", Some(STORAGES)), ("user", Some(USERS)), ("counter", None)] {
        let value = match source {
            Some(source) => res(word, "One resource, or every resource of the type", source),
            None => val(word, "One resource, or every resource of the type"),
        };
        args.push(value.optional_value());
        args.push(kw(&format!("{word}s"), "Every resource of the type"));
    }
    args.push(res("jobdefs", "One JobDefs, or every JobDefs", JOBDEFS).optional_value());
    args.push(kw("all", "Every resource"));
    args.push(kw("verbose", "Every directive, also defaults"));
    sub("show", "").args(args).subs(vec![sub("disabled", "Disabled resources").args(vec![
        kw("clients", "Disabled clients"), kw("jobs", "Disabled jobs"), kw("schedules", "Disabled schedules"),
    ])]).long(&["Prints resources of the running configuration in configuration syntax; passwords are masked. \
        Several items may be given in one line."])
        .example("show pool=Full client=fd", "two resources")
        .see(&["configure", "list"])
}

/// `update volume|pool|slots|jobid|stats` (UpdateCmd keywords).
fn update() -> Node {
    sub("update", "").long(&[
        "Changes catalog records of volumes, pools, slots and jobs; without arguments the Director asks what to update.",
    ]).subs(vec![
        vsub("volume", "Volume record", res("volume", "Volume name", MEDIA).optional_value()).alias(&["media"]).args(vec![
            enm("volstatus", "Volume status", VOLSTATUS_UPDATE), val("volretention", "Retention time"),
            val("voluse", "Use duration"), int("maxvoljobs", "Most jobs"), int("maxvolfiles", "Most files"),
            val("maxvolbytes", "Most bytes"), boolean("recycle", "Recycle"), boolean("inchanger", "In changer"),
            int("slot", "Slot"), res("pool", "Move to pool", POOLS), res("recyclepool", "Recycle pool", POOLS),
            kw("fromallpools", "Pool defaults for every volume"), res("allfrompool", "Pool defaults for the volumes of a pool", POOLS),
            boolean("enabled", "Enabled"), val("actiononpurge", "Action on purge").values(&["Truncate", "None"]),
            res("storage", "Storage name", STORAGES), val("encrypt", "Encryption").values(&["yes", "no", "rotate"]),
        ]),
        vsub("pool", "Pool record from the resource", res("pool", "Pool name", POOLS).optional_value()).args(vec![
            val("maxvolbytes", "Most bytes"), int("maxvolfiles", "Most files"), int("maxvoljobs", "Most jobs"),
            boolean("enabled", "Enabled"), res("recyclepool", "Recycle pool", POOLS), val("actiononpurge", "Action on purge"),
        ]),
        vsub("slots", "Slots from the autochanger", val("slots", "Slots (list or ranges)").optional_value()).alias(&["slot"]).args(vec![
            res("storage", "Storage name", STORAGES), sd(), pos("storage-name", "Storage name without key").source(STORAGES),
            kw("scan", "Read the labels"), int("drive", "Drive number"), boolean("enabled", "Enable or disable the volumes"),
        ]),
        vsub("jobid", "Job record", int("jobid", "Job ID")).args(vec![
            val("jobname", "Job name"), val("starttime", "Start time"), res("client", "Client name", CLIENTS),
            int("filesetid", "FileSet ID"), val("jobtype", "Job type letter"),
        ]),
        sub("stats", "Job statistics").args(vec![int("days", "Keep this many days")]),
    ]).see(&["list", "label"])
}

/// `delete volume|pool|jobid|storage`.
fn delete() -> Node {
    sub("delete", "").long(&["Deletes catalog records of volumes, pools, jobs or storages; asks unless yes is given."])
        .args(vec![kw("yes", "Do not ask"), catalog()])
        .subs(vec![
            vsub("volume", "A volume record", res("volume", "Volume name", MEDIA).optional_value())
                .args(vec![res("pool", "Pool of the volumes to select", POOLS)]),
            vsub("pool", "A pool record", res("pool", "Pool name", POOLS).optional_value()),
            vsub("jobid", "Job records (list or ranges)", val("jobid", "Job IDs").list()),
            vsub("storage", "A storage record", res("storage", "Storage name", STORAGES).optional_value())
                .args(vec![res("storage", "Storage name", STORAGES)]),
        ]).see(&["purge", "prune"])
}

/// `prune files|jobs|volume|stats|directory`.
fn prune() -> Node {
    let yes = || kw("yes", "Do not ask");
    sub("prune", "").long(&["Removes expired catalog records, respecting the retention periods."]).subs(vec![
        sub("files", "File records").args(vec![res("client", "Client name", CLIENTS), res("pool", "Pool name", POOLS), yes()]),
        sub("jobs", "Job records").args(vec![
            res("client", "Client name", CLIENTS), res("pool", "Pool name", POOLS), val("jobtype", "Job type"), yes(),
        ]),
        vsub("volume", "Volume records", res("volume", "Volume name", MEDIA).optional_value())
            .args(vec![kw("all", "Every volume"), res("pool", "Pool name", POOLS), yes()]),
        sub("stats", "Job statistics").args(vec![yes()]),
        vsub("directory", "Directory cache", val("directory", "Directory").optional_value())
            .args(vec![res("client", "Client name", CLIENTS), kw("recursive", "With subdirectories"), yes()]),
    ]).see(&["purge"])
}

/// `purge files|jobs|volume|quota` with the Onesimus extension `purge volume all`.
fn purge() -> Node {
    let yes = || kw("yes", "Do not ask");
    sub("purge", "").long(&[
        "Deletes catalog records without regard to retention periods. It needs full access to every resource.",
        "purge volume all (Onesimus extension) purges every volume of one pool (pool=) or of all pools the console may \
         access (allpools) whose status is one of volstatus= (default Full,Used,Error). It is never interactive: \
         dryrun only shows what would be purged, yes purges.",
    ]).args(vec![res("catalog", "Catalog name", CATALOGS)]).subs(vec![
        sub("files", "File records").args(vec![
            res("job", "Job name", JOBS), val("jobid", "Job IDs").list(), res("client", "Client name", CLIENTS),
            res("volume", "Volume name", MEDIA), yes(),
        ]),
        sub("jobs", "Job records").args(vec![
            res("client", "Client name", CLIENTS), res("volume", "Volume name", MEDIA), res("pool", "Pool name", POOLS),
            val("jobstatus", "Job status letters").list(), yes(),
        ]),
        vsub("volume", "Volume records", res("volume", "Volume name", MEDIA).optional_value()).args(vec![
            kw("all", "Every volume of the pool selection (Onesimus)"),
            res("pool", "Pool name", POOLS).group("scope"),
            kw("allpools", "Every pool").group("scope"),
            enm("volstatus", "Volume status (Onesimus); Purged and Recycle volumes are never selected, Cleaning tapes cannot be purged", &crate::commands::purge_bulk::VOLSTATUS).list(),
            res("volume", "Volumes for action= (several allowed)", MEDIA).repeat(),
            res("storage", "Storage name", STORAGES), val("devicetype", "Device type"), int("drive", "Drive number"),
            val("action", "Action on purge").values(&["truncate", "all"]),
            kw("dryrun", "Only show what would be purged (Onesimus)").group("mode"),
            kw("yes", "Do not ask").group("mode"),
        ]),
        sub("quota", "Quota of a client").args(vec![res("client", "Client name", CLIENTS), yes()]),
    ]).example("purge volume=Full-0001 yes", "purge one volume")
        .example("purge volume all pool=Full volstatus=Full,Used dryrun", "show what would be purged (Onesimus)")
        .example("purge volume all allpools yes", "purge every Full, Used or Error volume (Onesimus)")
        .see(&["prune", "delete", "truncate"])
}

/// Value source of a directive that references a resource of the type
/// `code` (`R_POOL` → `.pools`). None for a Counter (`WrapCounter`): no
/// dot-command lists counters, in Bareos neither.
pub(crate) fn resource_source(code: &str) -> Option<&'static str> {
    Some(match code {
        "R_CATALOG" => CATALOGS,
        "R_CLIENT" => CLIENTS,
        "R_CONSOLE" => CONSOLES,
        "R_FILESET" => FILESETS,
        "R_JOB" => JOBS,
        "R_JOBDEFS" => JOBDEFS,
        "R_MSGS" => MESSAGES,
        "R_POOL" => POOLS,
        "R_PROFILE" => PROFILES,
        "R_SCHEDULE" => SCHEDULES,
        "R_STORAGE" => STORAGES,
        "R_USER" => USERS,
        _ => return None,
    })
}

/// Type and values of a configuration directive from its item in the
/// directive tables: a resource reference gets the dot-command of the
/// referenced type, an enum keyword the values of the Director's keyword
/// tables ([`crate::commands::show::enum_values`]), a flag yes/no.
fn typed_directive(arg: GrammarArg, item: Option<&Item>) -> GrammarArg {
    let Some(item) = item else { return arg };
    match item.ty {
        Ty::Res | Ty::AlistRes => match item.code.and_then(resource_source) {
            Some(source) => arg.source(source),
            None => arg,
        },
        Ty::Bool | Ty::Bit => GrammarArg {
            arg_type: ArgType::Bool,
            values: Values::Fixed(vec!["yes".into(), "no".into()]),
            ..arg
        },
        ty => match crate::commands::show::enum_values(ty) {
            Some(values) => arg.values(&values),
            None => arg,
        },
    }
}

/// Arguments of `configure add <type>` from the generated Bareos usage line
/// (`name=<NAME> [description=<STRING>] ...`): bracketed ones optional,
/// list types repeatable; type and values from the directive table of `kind`.
fn configure_add_args(kind: &str, line: &str) -> Vec<GrammarArg> {
    let items = crate::config_check::items_of(kind).unwrap_or(&[]);
    line.split_whitespace().skip(2).filter_map(|word| {
        let optional = word.starts_with('[');
        let word = word.trim_matches(|c| c == '[' || c == ']');
        let (key, ty) = word.split_once("=<")?;
        let ty = ty.trim_end_matches('>');
        let mut arg = match ty {
            "BOOLEAN" => boolean(key, "Directive (yes/no)"),
            "PINT32" | "INT32" => int(key, "Directive (number)"),
            _ => val(key, "Directive"),
        };
        arg.description = format!("{} ({})", arg.description.split(" (").next().unwrap_or("Directive"), ty.to_ascii_lowercase());
        if matches!(ty, "ACL" | "RESOURCE_LIST" | "STRING_LIST" | "RUNSCRIPT_SHORT" | "MESSAGES" | "SCHEDULE_RUN_COMMAND") {
            arg = arg.repeat();
        }
        if !optional {
            arg = arg.req();
        }
        Some(typed_directive(arg, crate::config_check::item_of(items, key)))
    }).collect()
}

/// `configure add|export` (Bareos) and `read|update|delete` (Onesimus).
fn configure() -> Node {
    let usage = crate::commands::configure_usage::USAGE;
    let mut add_subs = Vec::new();
    for line in usage.split(" |\n") {
        let mut words = line.split_whitespace();
        if words.next() != Some("add") {
            continue;
        }
        let Some(kind) = words.next() else { continue };
        let node = if kind == "console" {
            // Onesimus narrow form (F-1): name, password, profile(s), description only
            sub(kind, "New console with the rights of its profiles").onesimus().args(vec![
                val("name", "Console name").req(), val("password", "Password").req(),
                res("profile", "Profile; several profiles repeated or as a comma list", PROFILES).req().repeat().list(),
                val("description", "Description"),
            ])
        } else {
            sub(kind, "New resource").args(configure_add_args(kind, line))
        };
        add_subs.push(node);
    }
    let editable = |what: &str| -> Vec<Node> {
        EDITABLE.iter().map(|t| {
            let source = match *t {
                "client" => CLIENTS, "job" => JOBS, "jobdefs" => JOBDEFS, "schedule" => SCHEDULES, _ => FILESETS,
            };
            let node = vsub(t, &format!("{what} a {t}"), res(t, "Resource name", source));
            if what == "Change" {
                node.args(update_directives(t))
            } else {
                node
            }
        }).collect()
    };
    sub("configure", "").long(&[
        "Adds resources to the configuration and exports them. Onesimus extensions read, update and delete \
         resources of the editable types (client, job, jobdefs, schedule, fileset, pool, storage); every change is checked like \
         a reload and written to the configuration directory.",
    ]).subs(vec![
        sub("add", "Add a resource").subs(add_subs),
        sub("export", "Export a client resource").args(vec![res("client", "Client name", CLIENTS).req()]),
        sub("read", "Show a resource (Onesimus)").alias(&["show", "get"]).onesimus().args(vec![
            res("client", "Client name", CLIENTS).group("what"), res("job", "Job name", JOBS).group("what"),
            res("pool", "Pool name", POOLS).group("what"), res("storage", "Storage name", STORAGES).group("what"),
        ]),
        sub("update", "Change directives of a resource (Onesimus)").onesimus().args(vec![kw("yes", "Do not ask")])
            .subs(editable("Change")),
        sub("delete", "Delete an unreferenced resource (Onesimus)").onesimus().subs(editable("Delete")),
    ]).example("configure add client name=fd2 address=fd2.example.org password=secret", "add a client")
        .example("configure add console name=ops password=pw profile=operator", "a console with the rights of a profile")
        .example("configure update schedule=Nightly run=\"Full sun at 23:00\" run=\"Incremental mon-sat at 23:00\"", "replace the Run lines (Onesimus)")
        .see(&["reload", "show", ".config"])
}

/// Directives `configure update <type>=<name>` accepts: every directive of
/// the type's item table (normalized key), repeatable (a directive given
/// several times replaces all its lines), an empty value removes it.
fn update_directives(rtype: &str) -> Vec<GrammarArg> {
    let items = crate::config_check::items_of(rtype).unwrap_or(&[]);
    let mut args: Vec<GrammarArg> = Vec::new();
    let add = |key: String, item: Option<&Item>, args: &mut Vec<GrammarArg>| {
        if key != "name" && !args.iter().any(|a| a.name == key) {
            args.push(typed_directive(val(&key, "Directive (empty value removes it)").repeat(), item));
        }
    };
    for item in items {
        if item.deprecated {
            continue;
        }
        add(bareos_config::edit::norm_key(item.name), Some(item), &mut args);
        for alias in item.aliases {
            add(bareos_config::edit::norm_key(alias), Some(item), &mut args);
        }
    }
    if matches!(rtype, "job" | "jobdefs") {
        // old Job aliases of client, fileset, pool, storage, schedule
        for key in ["newclient", "newfileset", "newpool", "newstorage", "newschedule"] {
            let item = crate::config_check::item_of(items, &key[3..]);
            add(key.to_string(), item, &mut args);
        }
    }
    args
}

/// `.acl list|set|delete|clear|copy` (Onesimus); the ACL types come from [`crate::acl::Kind`].
fn acl() -> Node {
    let types: Vec<&str> = crate::acl::Kind::ALL.iter().map(|k| k.key()).collect();
    let console = || res("console", "Console name", CONSOLES);
    let set_types = types.iter().map(|t| val(t, "Catalog ACL entries (comma list, repeatable)").repeat().list()).collect::<Vec<_>>();
    let delete_types = types.iter()
        .map(|t| val(t, "Entries to delete; without value every entry of the type").repeat().list().optional_value())
        .collect::<Vec<_>>();
    let mut set_args = vec![console().req()];
    set_args.extend(set_types);
    let mut delete_args = vec![console().req()];
    delete_args.extend(delete_types);
    sub(".acl", "Manage console ACLs (Onesimus)").long(&[
        "Shows and changes the catalog tier of the two-tier console ACLs. The ACL directives of the Console and \
         Profile resources are the outer frame (changed in the editor); the catalog ACL can only restrict further. \
         Effective is what both allow.",
        "set, delete, clear and copy need the CommandACL .acl and configure. Entries are comma separated; commas \
         inside {}, [] or () belong to the entry (regular expressions), a repeated key adds entries.",
    ]).subs(vec![
        sub("list", "File and catalog entries, effective result").args(vec![console()]),
        sub("set", "Replace catalog entries of types").args(set_args),
        sub("delete", "Delete catalog entries").args(delete_args),
        sub("clear", "Delete every catalog entry of a console").args(vec![console().req()]),
        sub("copy", "Copy the catalog entries of a console").args(vec![
            res("from", "Source console", CONSOLES).req(), res("to", "Target console", CONSOLES).req(),
        ]),
    ]).example(".acl list console=ops", "file and catalog ACL of ops")
        .example(".acl set console=ops jobacl=Backup-*,Restore commandacl=status,list", "restrict ops")
        .example(".acl delete console=ops jobacl", "remove the catalog JobACL entries")
        .see(&["configure", ".profiles"])
}

/// `.config` (Onesimus): configuration resources as raw text.
fn config_crud() -> Node {
    let type_arg = || enm("type", "Resource type", EDITABLE).req();
    let name = || val("name", "Resource name").req();
    let sha = || val("sha256", "SHA-256 of the text read").req();
    let text = || val("text64", "Resource text, base64").req();
    let check = || kw("validate_only", "Check only, write nothing");
    sub(".config", "").long(&[
        "Reads, creates, changes and deletes configuration resources as raw text (the same logic as the DirectorApi \
         configuration RPCs). Changes need the CommandACL configure and the SHA-256 of the text read.",
    ]).args(vec![json_kw()]).subs(vec![
        sub("capabilities", "Editable types and operations"),
        sub("list", "Resources of a type").args(vec![type_arg()]),
        sub("read", "Raw text of a resource").args(vec![type_arg(), name()]),
        sub("create", "New resource from text").args(vec![type_arg(), text(), check()]),
        sub("update", "Replace a resource").args(vec![type_arg(), name(), sha(), text(), check()]),
        sub("delete", "Delete a resource").args(vec![type_arg(), name(), sha(), check()]),
    ]).see(&["configure"])
}
