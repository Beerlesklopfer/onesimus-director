//! Command Registry — Single Source of Truth for all Director commands.
//!
//! Every command is declared with its arguments, types, defaults,
//! completion sources, and handler function. The registry drives:
//! - Command dispatch (replaces the flat match block)
//! - .help output (machine + human format)
//! - Tab-completion in oconsole (argument names + values)
//! - Argument validation before dispatch
//! - Wiki generation (future)

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::commands;

/// Handler function signature.
pub type CmdHandler = fn(&CommandAst, &ExecContext) -> CommandResult;

/// Argument type for validation and completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgType {
    /// Free-form string
    String,
    /// Positive integer
    Integer,
    /// Boolean (yes/no)
    Bool,
    /// One of a fixed set of values (completion from enum_values)
    Enum,
    /// Value comes from a dot-command (e.g. .clients, .jobs)
    Resource,
}

/// Where to get completion values for this argument.
#[derive(Debug, Clone)]
pub enum CompletionSource {
    /// No completion
    None,
    /// Fixed list of values
    Fixed(Vec<&'static str>),
    /// Query a dot-command for values (e.g. ".clients", ".jobs")
    DotCommand(&'static str),
}

/// Argument definition.
#[derive(Debug, Clone)]
pub struct ArgDef {
    pub name: &'static str,
    pub arg_type: ArgType,
    pub required: bool,
    pub default: Option<&'static str>,
    pub description: &'static str,
    pub completion: CompletionSource,
}

/// Command definition — the single source of truth.
#[derive(Clone)]
pub struct CommandDef {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub args: Vec<ArgDef>,
    pub handler: CmdHandler,
    /// True for dot-commands (machine interface)
    pub dot_command: bool,
}

// ============================================================================
// Argument builder helpers
// ============================================================================

const fn arg(name: &'static str, description: &'static str) -> ArgDef {
    ArgDef { name, arg_type: ArgType::String, required: false, default: None, description, completion: CompletionSource::None }
}

const fn arg_required(name: &'static str, description: &'static str) -> ArgDef {
    ArgDef { name, arg_type: ArgType::String, required: true, default: None, description, completion: CompletionSource::None }
}

// Non-const helpers (use Fixed/DotCommand which allocate)
fn arg_enum(name: &'static str, description: &'static str, values: Vec<&'static str>) -> ArgDef {
    ArgDef { name, arg_type: ArgType::Enum, required: false, default: None, description, completion: CompletionSource::Fixed(values) }
}

fn arg_resource(name: &'static str, description: &'static str, dot_cmd: &'static str) -> ArgDef {
    ArgDef { name, arg_type: ArgType::Resource, required: false, default: None, description, completion: CompletionSource::DotCommand(dot_cmd) }
}

fn arg_int(name: &'static str, description: &'static str) -> ArgDef {
    ArgDef { name, arg_type: ArgType::Integer, required: false, default: None, description, completion: CompletionSource::None }
}

// ============================================================================
// The Registry
// ============================================================================

/// Build the complete command registry.
pub fn build_registry() -> Vec<CommandDef> {
    vec![
        // === Basic ===
        CommandDef { name: "version", aliases: &[], description: "Show Director version", args: vec![], handler: commands::basic::cmd_version, dot_command: false },
        CommandDef { name: "time", aliases: &[], description: "Show current date and time", args: vec![], handler: commands::basic::cmd_time, dot_command: false },
        CommandDef { name: "quit", aliases: &["exit"], description: "Disconnect from Director", args: vec![], handler: commands::basic::cmd_quit, dot_command: false },
        CommandDef { name: "help", aliases: &[], description: "List available commands", args: vec![arg("command", "Show help for specific command")], handler: commands::basic::cmd_help, dot_command: false },

        // === Status ===
        CommandDef { name: "status", aliases: &[], description: "Show Director/Client/Storage status", args: vec![
            arg("dir", "Director status"),
            arg_resource("client", "Client name", ".clients"),
            arg_resource("storage", "Storage name", ".storage"),
        ], handler: commands::status::cmd_status, dot_command: false },

        // === Job control ===
        CommandDef { name: "run", aliases: &[], description: "Start a backup/restore job", args: vec![
            arg_resource("job", "Job name", ".jobs"),
            arg_enum("level", "Backup level", vec!["Full", "Incremental", "Differential"]),
            arg_resource("client", "Client name", ".clients"),
            arg_resource("storage", "Storage name", ".storage"),
            arg_resource("pool", "Pool name", ".pools"),
            arg_resource("fileset", "FileSet name", ".filesets"),
            arg("when", "Schedule time"),
            arg_int("priority", "Job priority (1-100)"),
            arg("yes", "Skip confirmation"),
        ], handler: commands::job::cmd_run, dot_command: false },

        CommandDef { name: "cancel", aliases: &[], description: "Cancel a running job", args: vec![
            arg_int("jobid", "Job ID to cancel"),
        ], handler: commands::job::cmd_cancel, dot_command: false },

        CommandDef { name: "restore", aliases: &[], description: "Restore files from backup", args: vec![
            arg_resource("client", "Client name", ".clients"),
            arg_int("jobid", "Backup JobId to restore from"),
            arg("where", "Restore destination path"),
            arg_enum("replace", "File replace mode", vec!["always", "never", "ifnewer", "ifolder"]),
            arg("all", "Select all files"),
            arg("done", "Skip interactive selection"),
            arg("yes", "Skip confirmation"),
        ], handler: commands::admin::cmd_restore, dot_command: false },

        CommandDef { name: "enable", aliases: &[], description: "Enable a resource", args: vec![
            arg_resource("job", "Job name", ".jobs"),
            arg_resource("client", "Client name", ".clients"),
            arg_resource("schedule", "Schedule name", ".schedule"),
            arg_resource("storage", "Storage name", ".storage"),
        ], handler: commands::job::cmd_enable, dot_command: false },

        CommandDef { name: "disable", aliases: &[], description: "Disable a resource", args: vec![
            arg_resource("job", "Job name", ".jobs"),
            arg_resource("client", "Client name", ".clients"),
            arg_resource("schedule", "Schedule name", ".schedule"),
            arg_resource("storage", "Storage name", ".storage"),
        ], handler: commands::job::cmd_disable, dot_command: false },

        // === List ===
        CommandDef { name: "list", aliases: &["llist"], description: "List catalog records", args: vec![
            arg_enum("what", "What to list", vec!["jobs", "volumes", "clients", "pools", "files", "storages", "schedules", "filesets", "jobtotals"]),
            arg_int("limit", "Max rows"),
            arg_int("jobid", "Filter by JobId"),
            arg_resource("job", "Filter by job name", ".jobs"),
        ], handler: commands::list::cmd_list, dot_command: false },

        // === Admin ===
        CommandDef { name: "label", aliases: &[], description: "Label a new volume", args: vec![
            arg_resource("storage", "Storage name", ".storage"),
            arg("volume", "Volume name"),
            arg_resource("pool", "Pool name", ".pools"),
        ], handler: commands::admin::cmd_label, dot_command: false },

        CommandDef { name: "mount", aliases: &[], description: "Mount a storage device", args: vec![
            arg_resource("storage", "Storage name", ".storage"),
        ], handler: commands::admin::cmd_mount, dot_command: false },

        CommandDef { name: "unmount", aliases: &["umount"], description: "Unmount a storage device", args: vec![
            arg_resource("storage", "Storage name", ".storage"),
        ], handler: commands::admin::cmd_unmount, dot_command: false },

        CommandDef { name: "release", aliases: &[], description: "Release a storage device", args: vec![
            arg_resource("storage", "Storage name", ".storage"),
        ], handler: commands::admin::cmd_release, dot_command: false },

        CommandDef { name: "update", aliases: &[], description: "Update volume/pool attributes", args: vec![
            arg("volume", "Volume name"),
            arg_resource("pool", "Pool name", ".pools"),
        ], handler: commands::admin::cmd_update, dot_command: false },

        CommandDef { name: "estimate", aliases: &[], description: "Estimate backup size", args: vec![
            arg_resource("job", "Job name", ".jobs"),
            arg_resource("client", "Client name", ".clients"),
            arg_resource("fileset", "FileSet name", ".filesets"),
            arg_enum("level", "Backup level", vec!["Full", "Incremental", "Differential"]),
        ], handler: commands::estimate::cmd_estimate, dot_command: false },

        CommandDef { name: "delete", aliases: &[], description: "Delete volume or job records", args: vec![
            arg("volume", "Volume name"),
            arg_int("jobid", "Job ID"),
        ], handler: commands::extra::cmd_delete, dot_command: false },

        CommandDef { name: "prune", aliases: &[], description: "Prune expired catalog records", args: vec![
            arg("volume", "Volume name"),
            arg_resource("client", "Client name", ".clients"),
        ], handler: commands::admin::cmd_prune, dot_command: false },

        CommandDef { name: "purge", aliases: &[], description: "Purge all records from volume", args: vec![
            arg("volume", "Volume name"),
        ], handler: commands::admin::cmd_purge, dot_command: false },

        CommandDef { name: "truncate", aliases: &[], description: "Truncate purged volume", args: vec![
            arg_resource("storage", "Storage name", ".storage"),
            arg("volume", "Volume name"),
        ], handler: commands::admin::cmd_truncate, dot_command: false },

        CommandDef { name: "reload", aliases: &[], description: "Reload Director configuration", args: vec![], handler: commands::admin::cmd_reload, dot_command: false },
        CommandDef { name: "configure", aliases: &[], description: "Add/modify/delete resources", args: vec![
            arg_enum("sub", "Sub-command", vec!["add", "read", "delete"]),
        ], handler: commands::configure::cmd_configure, dot_command: false },

        CommandDef { name: "sqlquery", aliases: &[], description: "Execute raw SQL", args: vec![], handler: commands::sql::cmd_sql_query, dot_command: false },
        CommandDef { name: "query", aliases: &[], description: "Query catalog", args: vec![], handler: commands::sql::cmd_query, dot_command: false },

        // === Session ===
        CommandDef { name: "autodisplay", aliases: &[], description: "Toggle automatic message display", args: vec![], handler: commands::extra::cmd_autodisplay, dot_command: false },
        CommandDef { name: "automount", aliases: &[], description: "Toggle automatic mounting", args: vec![], handler: commands::extra::cmd_automount, dot_command: false },
        CommandDef { name: "gui", aliases: &[], description: "Set GUI mode", args: vec![], handler: commands::extra::cmd_gui, dot_command: false },
        CommandDef { name: "messages", aliases: &[], description: "Display pending messages", args: vec![], handler: commands::extra::cmd_messages, dot_command: false },
        CommandDef { name: "memory", aliases: &[], description: "Show memory usage", args: vec![], handler: commands::extra::cmd_memory, dot_command: false },
        CommandDef { name: "show", aliases: &[], description: "Show resource configuration", args: vec![
            arg_enum("what", "Resource type", vec!["directors", "clients", "jobs", "jobdefs", "storages", "catalogs",
                "schedules", "filesets", "pools", "messages", "counters", "profiles", "consoles", "users", "all", "verbose", "disabled"]),
        ], handler: commands::show::cmd_show, dot_command: false },
        CommandDef { name: "trace", aliases: &[], description: "Toggle trace output", args: vec![], handler: commands::extra::cmd_trace, dot_command: false },
        CommandDef { name: "var", aliases: &[], description: "Display variable expansion", args: vec![], handler: commands::extra::cmd_var, dot_command: false },
        CommandDef { name: "wait", aliases: &[], description: "Wait for a job to complete", args: vec![arg_int("jobid", "Job ID"),
            arg("jobname", "Running job name prefix"), arg("ujobid", "Unique running job name"),
            arg("mount", "Wait for a media or mount request"), arg_int("timeout", "Mount wait timeout in seconds")], handler: commands::wait::cmd_wait, dot_command: false },
        CommandDef { name: "whoami", aliases: &[], description: "Show current console identity", args: vec![], handler: commands::extra::cmd_whoami, dot_command: false },
        CommandDef { name: "use", aliases: &[], description: "Switch catalog", args: vec![arg_resource("catalog", "Catalog name", ".catalogs")], handler: commands::extra::cmd_use, dot_command: false },
        CommandDef { name: "setdebug", aliases: &[], description: "Set debug level", args: vec![
            arg_int("level", "Debug level"),
            arg_enum("trace", "Trace flag (-1 keeps current setting)", vec!["-1", "0", "1"]),
            arg_enum("timestamp", "Timestamp flag", vec!["-1", "0", "1"]),
            arg_enum("hangup", "FD hangup flag", vec!["-1", "0", "1"]),
            arg_resource("client", "File Daemon", ".clients"),
            arg_resource("storage", "Storage Daemon", ".storages"),
            arg("dir", "Select the Director"), arg("all", "Select all daemons"),
        ], handler: commands::debug::cmd_setdebug, dot_command: false },
        CommandDef { name: "setbandwidth", aliases: &[], description: "Set bandwidth limit", args: vec![arg_resource("client", "Client name", ".clients"), arg_resource("storage", "Storage name", ".storages"), arg_int("limit", "KiB/s (0: unlimited)")], handler: commands::bandwidth::cmd_setbandwidth, dot_command: false },
        CommandDef { name: "setip", aliases: &[], description: "Update client IP address", args: vec![arg_resource("client", "Client name", ".clients")], handler: commands::extra::cmd_setip, dot_command: false },
        CommandDef { name: "setdevice", aliases: &[], description: "Set device parameters", args: vec![arg_resource("storage", "Storage name", ".storage")], handler: commands::extra::cmd_setdevice, dot_command: false },
        CommandDef { name: "resolve", aliases: &[], description: "DNS resolve hostname", args: vec![arg_resource("client", "Client-side resolver", ".clients"), arg_resource("storage", "Storage-side resolver", ".storages")], handler: commands::resolve::cmd_resolve, dot_command: false },
        CommandDef { name: "rerun", aliases: &[], description: "Re-run a failed job", args: vec![arg_int("jobid", "Job ID")], handler: commands::extra::cmd_rerun, dot_command: false },
        CommandDef { name: "relabel", aliases: &[], description: "Re-label a volume", args: vec![arg_resource("storage", "Storage name", ".storage"), arg("volume", "Volume name")], handler: commands::extra::cmd_relabel, dot_command: false },
        CommandDef { name: "create", aliases: &[], description: "Create pool from resource", args: vec![arg_resource("pool", "Pool name", ".pools")], handler: commands::extra::cmd_create, dot_command: false },
        CommandDef { name: "add", aliases: &[], description: "Add volume to pool", args: vec![arg_resource("pool", "Pool name", ".pools")], handler: commands::extra::cmd_add, dot_command: false },
        CommandDef { name: "import", aliases: &[], description: "Import volumes from magazine", args: vec![arg_resource("storage", "Storage name", ".storage")], handler: commands::extra::cmd_import, dot_command: false },
        CommandDef { name: "export", aliases: &[], description: "Export volumes to magazine", args: vec![arg_resource("storage", "Storage name", ".storage")], handler: commands::extra::cmd_export, dot_command: false },
        CommandDef { name: "move", aliases: &[], description: "Move volume between pools", args: vec![], handler: commands::extra::cmd_move, dot_command: false },

        // =================================================================
        // DOT COMMANDS
        // =================================================================
        CommandDef { name: ".help", aliases: &[], description: "List commands (machine format)", args: vec![], handler: commands::basic::cmd_dot_help, dot_command: true },
        CommandDef { name: ".quit", aliases: &[], description: "Disconnect", args: vec![], handler: commands::basic::cmd_quit, dot_command: true },
        CommandDef { name: ".status", aliases: &[], description: "Status (machine format)", args: vec![], handler: commands::status::cmd_dot_status, dot_command: true },
        CommandDef { name: ".", aliases: &[], description: "No-op", args: vec![], handler: commands::dot::cmd_dot_noop, dot_command: true },

        // Resource listing
        CommandDef { name: ".clients", aliases: &[], description: "List client names", args: vec![], handler: commands::dot::cmd_dot_clients, dot_command: true },
        CommandDef { name: ".jobs", aliases: &[], description: "List job names", args: vec![], handler: commands::dot::cmd_dot_jobs, dot_command: true },
        CommandDef { name: ".pools", aliases: &[], description: "List pool names", args: vec![], handler: commands::dot::cmd_dot_pools, dot_command: true },
        CommandDef { name: ".storage", aliases: &[".storages"], description: "List storage names", args: vec![], handler: commands::dot::cmd_dot_storage, dot_command: true },
        CommandDef { name: ".filesets", aliases: &[], description: "List fileset names", args: vec![], handler: commands::dot::cmd_dot_filesets, dot_command: true },
        CommandDef { name: ".schedule", aliases: &[], description: "List schedule names", args: vec![], handler: commands::dot::cmd_dot_schedule, dot_command: true },
        CommandDef { name: ".catalogs", aliases: &[], description: "List catalog names", args: vec![], handler: commands::dot::cmd_dot_catalogs, dot_command: true },
        CommandDef { name: ".consoles", aliases: &[], description: "List console names", args: vec![], handler: commands::dot::cmd_dot_consoles, dot_command: true },
        CommandDef { name: ".jobdefs", aliases: &[], description: "List jobdefs names", args: vec![], handler: commands::dot::cmd_dot_jobdefs, dot_command: true },
        CommandDef { name: ".profiles", aliases: &[], description: "List profile names", args: vec![], handler: commands::dot::cmd_dot_profiles, dot_command: true },
        CommandDef { name: ".locations", aliases: &[], description: "List location names", args: vec![], handler: commands::dot::cmd_dot_locations, dot_command: true },
        CommandDef { name: ".media", aliases: &[], description: "List volume names", args: vec![], handler: commands::dot::cmd_dot_media, dot_command: true },
        CommandDef { name: ".mediatypes", aliases: &[], description: "List media types", args: vec![], handler: commands::dot::cmd_dot_mediatypes, dot_command: true },
        CommandDef { name: ".messages", aliases: &[], description: "List message resource names", args: vec![], handler: commands::dot::cmd_dot_messages, dot_command: true },
        CommandDef { name: ".msgs", aliases: &[], description: "List pending message types", args: vec![], handler: commands::dot::cmd_dot_msgs, dot_command: true },
        CommandDef { name: ".users", aliases: &[], description: "List user names", args: vec![], handler: commands::dot::cmd_dot_users, dot_command: true },

        // Query
        CommandDef { name: ".defaults", aliases: &[], description: "Show default values", args: vec![arg_resource("job", "Job name", ".jobs")], handler: commands::dot::cmd_dot_defaults, dot_command: true },
        CommandDef { name: ".jobstatus", aliases: &[], description: "List job status codes", args: vec![], handler: commands::dot::cmd_dot_jobstatus, dot_command: true },
        CommandDef { name: ".authorized", aliases: &[], description: "Check command authorization", args: vec![arg("job", "Command name")], handler: commands::dot::cmd_dot_authorized, dot_command: true },
        CommandDef { name: ".levels", aliases: &[], description: "List backup levels", args: vec![], handler: commands::dot::cmd_dot_levels, dot_command: true },
        CommandDef { name: ".types", aliases: &[], description: "List job types", args: vec![], handler: commands::dot::cmd_dot_types, dot_command: true },
        CommandDef { name: ".volstatus", aliases: &[], description: "List volume status values", args: vec![], handler: commands::dot::cmd_dot_volstatus, dot_command: true },
        CommandDef { name: ".actiononpurge", aliases: &[], description: "List action-on-purge options", args: vec![], handler: commands::dot::cmd_dot_actiononpurge, dot_command: true },

        // API/protocol
        CommandDef { name: ".api", aliases: &[], description: "Set API mode (0/1/2)", args: vec![arg_enum("mode", "API mode", vec!["0", "1", "2"])], handler: commands::dot::cmd_dot_api, dot_command: true },
        CommandDef { name: ".grpc", aliases: &[], description: "Toggle gRPC mode", args: vec![], handler: commands::dot::cmd_dot_grpc, dot_command: true },

        // SQL
        CommandDef { name: ".sql", aliases: &[], description: "Execute SQL", args: vec![arg("query", "SQL query")], handler: commands::sql::cmd_dot_sql, dot_command: true },
        CommandDef { name: ".sqltables", aliases: &[], description: "List catalog tables", args: vec![], handler: commands::dot::cmd_dot_sqltables, dot_command: true },
        CommandDef { name: ".sqlschema", aliases: &[], description: "Show catalog schema", args: vec![], handler: commands::dot::cmd_dot_sqlschema, dot_command: true },
        CommandDef { name: ".sqlcount", aliases: &[], description: "Count catalog records", args: vec![], handler: commands::dot::cmd_dot_sqlcount, dot_command: true },

        // ACL
        CommandDef { name: ".acl", aliases: &[".profile"], description: "Manage console ACLs", args: vec![], handler: commands::acl::cmd_dot_acl, dot_command: true },

        // Catalog
        CommandDef { name: ".catalog", aliases: &[], description: "Catalog management", args: vec![], handler: commands::catalog::cmd_dot_catalog, dot_command: true },

        // BVFS
        CommandDef { name: ".bvfs_update", aliases: &[], description: "Update BVFS cache", args: vec![arg_int("jobid", "Job ID")], handler: commands::dot::cmd_bvfs_update, dot_command: true },
        CommandDef { name: ".bvfs_cleanup", aliases: &[], description: "Cleanup BVFS temp tables", args: vec![], handler: commands::dot::cmd_bvfs_cleanup, dot_command: true },
        CommandDef { name: ".bvfs_clear_cache", aliases: &[], description: "Clear BVFS cache", args: vec![arg("yes", "Confirm")], handler: commands::dot::cmd_bvfs_clear_cache, dot_command: true },
        CommandDef { name: ".bvfs_get_jobids", aliases: &[], description: "Get job IDs for restore", args: vec![arg_int("jobid", "Job ID")], handler: commands::dot::cmd_bvfs_get_jobids, dot_command: true },
        CommandDef { name: ".bvfs_lsdirs", aliases: &[], description: "List directories (BVFS)", args: vec![arg_int("jobid", "Job ID"), arg_int("pathid", "Path ID")], handler: commands::dot::cmd_bvfs_lsdirs, dot_command: true },
        CommandDef { name: ".bvfs_lsfiles", aliases: &[], description: "List files (BVFS)", args: vec![arg_int("jobid", "Job ID"), arg_int("pathid", "Path ID")], handler: commands::dot::cmd_bvfs_lsfiles, dot_command: true },
        CommandDef { name: ".bvfs_versions", aliases: &[], description: "List file versions", args: vec![arg_int("jobid", "Job ID"), arg_int("pathid", "Path ID"), arg("fname", "File name")], handler: commands::dot::cmd_bvfs_versions, dot_command: true },
        CommandDef { name: ".bvfs_restore", aliases: &[], description: "Restore from BVFS selection", args: vec![arg_int("jobid", "Job ID")], handler: commands::dot::cmd_bvfs_restore, dot_command: true },
    ]
}

// ============================================================================
// Registry-based dispatch
// ============================================================================

/// Find a command by name or alias.
/// Bareos resolves the first case-insensitive prefix in its original table.
/// Local-only aliases/extensions remain exact matches after that lookup.
pub fn resolve_command_name<'a>(registry: &'a [CommandDef], name: &str) -> Option<&'a str> {
    if name.is_empty() { return None; }
    let lower = name.to_ascii_lowercase();
    if let Some(entry) = crate::ua_commands_table::UA_COMMANDS.iter()
        .find(|entry| entry.0.starts_with(&lower)) {
        return Some(entry.0);
    }
    registry.iter().find_map(|cmd| {
        if cmd.name == lower { Some(cmd.name) }
        else { cmd.aliases.iter().find(|alias| **alias == lower).copied() }
    })
}

pub fn find_command<'a>(registry: &'a [CommandDef], name: &str) -> Option<&'a CommandDef> {
    let resolved = resolve_command_name(registry, name)?;
    registry.iter().find(|cmd| cmd.name == resolved || cmd.aliases.contains(&resolved))
}

/// Resolve a command the console may execute (Do_a_command): unknown and
/// ACL-denied commands are both rejected as "is an invalid command".
fn resolve_permitted<'a>(registry: &'a [CommandDef], ast: &CommandAst, ctx: &ExecContext) -> Option<(&'a str, &'a CommandDef)> {
    let name = resolve_command_name(registry, &ast.command_name)?;
    let command = find_command(registry, name)?;
    let acl_name=if crate::ua_commands_table::UA_COMMANDS.iter().any(|entry|entry.0==name) {name} else {command.name};
    if !matches!(acl_name, "exit" | "quit" | "whoami") && !crate::acl::access(ctx, crate::acl::Kind::Command, acl_name) {
        return None;
    }
    Some((name, command))
}

/// True when Do_a_command finds no permitted command; it then sends neither
/// CMD_BEGIN nor CMD_OK/CMD_FAILED in API mode.
pub fn is_invalid_command(registry: &[CommandDef], ast: &CommandAst, ctx: &ExecContext) -> bool {
    resolve_permitted(registry, ast, ctx).is_none()
}

/// Preserve the resolved alias (e.g. llist) for handlers with alias-specific output.
pub fn dispatch_registry(registry: &[CommandDef], ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some((name, command)) = resolve_permitted(registry, ast, ctx) else {
        // ua_cmds.cc Do_a_command(): ErrorMsg("%s: is an invalid command.\n", argk[0])
        return CommandResult::error(format!("{}: is an invalid command.\n", ast.command_name));
    };
    let mut resolved = ast.clone();
    resolved.command_name = name.to_string();
    (command.handler)(&resolved, ctx)
}

/// UaContext::SendCmdUsage(): "<message>\nUSAGE: <key> <usage>\n" (a SendMsg).
pub fn cmd_usage(key: &str, message: &str) -> String {
    match crate::ua_commands_table::UA_COMMANDS.iter().find(|entry| entry.0 == key) {
        Some((key, _, usage)) => format!("{message}\nUSAGE: {key} {usage}\n"),
        None => message.to_string(),
    }
}

/// Generate .help output from registry (one command per line, machine format).
pub fn generate_help(registry: &[CommandDef]) -> String {
    let mut out = String::new();
    for cmd in registry {
        if cmd.name == "." { continue; }
        let args: Vec<String> = cmd.args.iter().map(|a| {
            if a.required {
                format!("{}=<{}>", a.name, a.name)
            } else {
                format!("[{}=<{}>]", a.name, a.name)
            }
        }).collect();
        out.push_str(&format!("{} {} — {}\n", cmd.name, args.join(" "), cmd.description));
    }
    out
}

/// Get all command names for tab-completion.
pub fn command_names(registry: &[CommandDef]) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for cmd in registry {
        names.push(cmd.name);
        for alias in cmd.aliases {
            names.push(alias);
        }
    }
    names
}

/// Get argument names for a command (for tab-completion after command name).
pub fn argument_names<'a>(registry: &'a [CommandDef], cmd_name: &str) -> Vec<&'a str> {
    match find_command(registry, cmd_name) {
        Some(cmd) => cmd.args.iter().map(|a| a.name).collect(),
        None => vec![],
    }
}

#[cfg(test)]
mod prefix_tests {
    use super::*;

    #[test]
    fn matches_original_table_order_not_arbitrary_exact_precedence() {
        let registry = build_registry();
        for (input, expected) in [("l", "label"), ("LL", "llist"), (".j", ".jobdefs"),
            (".stor", ".storages"), ("q", "quit"), ("ex", "exit"), (".a", ".actiononpurge"),
            (".api", ".api"), ("VERSION", "version"), (".sqlcount", ".sqlcount")] {
            assert_eq!(resolve_command_name(&registry, input), Some(expected), "{input}");
        }
        assert!(resolve_command_name(&registry, "").is_none());
        assert!(resolve_command_name(&registry, "unknown").is_none());
    }

    #[test]
    fn unknown_command_uses_original_text_and_usage_matches_send_cmd_usage() {
        let ctx = ExecContext { session_id: 0, config: None, session_mgr: None, catalog: None,
            api_mode: crate::command_executor::ApiMode::Human, bus_rx: None, job_manager: None };
        let ast = crate::session::parse_cli_command("FooBar arg=1");
        assert!(is_invalid_command(&build_registry(), &ast, &ctx));
        assert_eq!(dispatch_registry(&build_registry(), &ast, &ctx).error_message, "FooBar: is an invalid command.\n");
        assert_eq!(cmd_usage("setdevice", ""), "\nUSAGE: setdevice storage=<storage-name> device=<device-name> autoselect=<bool>\n");
    }
}
