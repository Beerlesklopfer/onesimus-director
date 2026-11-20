//! Command Registry — Single Source of Truth for all Director commands.
//!
//! Every command is declared with its name, aliases, description and
//! handler. The registry drives command dispatch and the ACL checks; what a
//! command accepts (subcommands, keywords, `key=value` arguments, value
//! sources) is its grammar ([`crate::command_grammar`], table in
//! [`crate::grammar_table`]), published by `.grammar` and used for the man
//! pages.

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::commands;

/// Handler function signature.
pub type CmdHandler = fn(&CommandAst, &ExecContext) -> CommandResult;

/// Command definition — the single source of truth.
#[derive(Clone)]
pub struct CommandDef {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub handler: CmdHandler,
    /// True for dot-commands (machine interface)
    pub dot_command: bool,
}

// ============================================================================
// The Registry
// ============================================================================

/// Build the complete command registry.
pub fn build_registry() -> Vec<CommandDef> {
    vec![
        // === Basic ===
        CommandDef { name: "version", aliases: &[], description: "Show Director version", handler: commands::basic::cmd_version, dot_command: false },
        CommandDef { name: "time", aliases: &[], description: "Show current date and time", handler: commands::basic::cmd_time, dot_command: false },
        CommandDef { name: "quit", aliases: &["exit"], description: "Disconnect from Director", handler: commands::basic::cmd_quit, dot_command: false },
        CommandDef { name: "help", aliases: &[], description: "List available commands", handler: commands::basic::cmd_help, dot_command: false },

        // === Status ===
        CommandDef { name: "status", aliases: &[], description: "Show Director/Client/Storage status", handler: commands::status::cmd_status, dot_command: false },

        // === Job control ===
        CommandDef { name: "run", aliases: &[], description: "Start a backup/restore job", handler: commands::job::cmd_run, dot_command: false },

        CommandDef { name: "cancel", aliases: &[], description: "Cancel a running job", handler: commands::cancel::cmd_cancel, dot_command: false },

        CommandDef { name: "restore", aliases: &[], description: "Restore files from backup", handler: commands::restore::cmd_restore, dot_command: false },

        CommandDef { name: "enable", aliases: &[], description: "Enable a resource", handler: commands::job::cmd_enable, dot_command: false },

        CommandDef { name: "disable", aliases: &[], description: "Disable a resource", handler: commands::job::cmd_disable, dot_command: false },

        // === List ===
        CommandDef { name: "list", aliases: &["llist"], description: "List catalog records", handler: commands::list::cmd_list, dot_command: false },

        // === Admin ===
        CommandDef { name: "label", aliases: &[], description: "Label a new volume", handler: commands::admin::cmd_label, dot_command: false },

        CommandDef { name: "mount", aliases: &[], description: "Mount a storage device", handler: commands::admin::cmd_mount, dot_command: false },

        CommandDef { name: "unmount", aliases: &["umount"], description: "Unmount a storage device", handler: commands::admin::cmd_unmount, dot_command: false },

        CommandDef { name: "release", aliases: &[], description: "Release a storage device", handler: commands::admin::cmd_release, dot_command: false },

        CommandDef { name: "update", aliases: &[], description: "Update volume/pool attributes", handler: commands::admin::cmd_update, dot_command: false },

        CommandDef { name: "estimate", aliases: &[], description: "Estimate backup size", handler: commands::estimate::cmd_estimate, dot_command: false },

        CommandDef { name: "delete", aliases: &[], description: "Delete volume or job records", handler: commands::extra::cmd_delete, dot_command: false },

        CommandDef { name: "prune", aliases: &[], description: "Prune expired catalog records", handler: commands::admin::cmd_prune, dot_command: false },

        CommandDef { name: "purge", aliases: &[], description: "Purge all records from volume", handler: commands::admin::cmd_purge, dot_command: false },

        CommandDef { name: "truncate", aliases: &[], description: "Truncate purged volume", handler: commands::admin::cmd_truncate, dot_command: false },

        CommandDef { name: "reload", aliases: &[], description: "Reload Director configuration", handler: commands::admin::cmd_reload, dot_command: false },
        CommandDef { name: "configure", aliases: &[], description: "Add/modify/delete resources", handler: commands::configure::cmd_configure, dot_command: false },

        CommandDef { name: "sqlquery", aliases: &[], description: "Execute raw SQL", handler: commands::sql::cmd_sql_query, dot_command: false },
        CommandDef { name: "query", aliases: &[], description: "Query catalog", handler: commands::sql::cmd_query, dot_command: false },

        // === Session ===
        CommandDef { name: "autodisplay", aliases: &[], description: "Toggle automatic message display", handler: commands::extra::cmd_autodisplay, dot_command: false },
        CommandDef { name: "automount", aliases: &[], description: "Toggle automatic mounting", handler: commands::extra::cmd_automount, dot_command: false },
        CommandDef { name: "gui", aliases: &[], description: "Set GUI mode", handler: commands::extra::cmd_gui, dot_command: false },
        CommandDef { name: "messages", aliases: &[], description: "Display pending messages", handler: commands::extra::cmd_messages, dot_command: false },
        CommandDef { name: "memory", aliases: &[], description: "Show memory usage", handler: commands::extra::cmd_memory, dot_command: false },
        CommandDef { name: "show", aliases: &[], description: "Show resource configuration", handler: commands::show::cmd_show, dot_command: false },
        CommandDef { name: "trace", aliases: &[], description: "Toggle trace output", handler: commands::extra::cmd_trace, dot_command: false },
        CommandDef { name: "var", aliases: &[], description: "Display variable expansion", handler: commands::extra::cmd_var, dot_command: false },
        CommandDef { name: "wait", aliases: &[], description: "Wait for a job to complete", handler: commands::wait::cmd_wait, dot_command: false },
        CommandDef { name: "whoami", aliases: &[], description: "Show current console identity", handler: commands::extra::cmd_whoami, dot_command: false },
        CommandDef { name: "use", aliases: &[], description: "Switch catalog", handler: commands::extra::cmd_use, dot_command: false },
        CommandDef { name: "setdebug", aliases: &[], description: "Set debug level", handler: commands::debug::cmd_setdebug, dot_command: false },
        CommandDef { name: "setbandwidth", aliases: &[], description: "Set bandwidth limit", handler: commands::bandwidth::cmd_setbandwidth, dot_command: false },
        CommandDef { name: "setip", aliases: &[], description: "Update client IP address", handler: commands::extra::cmd_setip, dot_command: false },
        CommandDef { name: "setdevice", aliases: &[], description: "Set device parameters", handler: commands::extra::cmd_setdevice, dot_command: false },
        CommandDef { name: "resolve", aliases: &[], description: "DNS resolve hostname", handler: commands::resolve::cmd_resolve, dot_command: false },
        CommandDef { name: "rerun", aliases: &[], description: "Re-run a failed job", handler: commands::extra::cmd_rerun, dot_command: false },
        CommandDef { name: "relabel", aliases: &[], description: "Re-label a volume", handler: commands::extra::cmd_relabel, dot_command: false },
        CommandDef { name: "create", aliases: &[], description: "Create pool from resource", handler: commands::extra::cmd_create, dot_command: false },
        CommandDef { name: "add", aliases: &[], description: "Add volume to pool", handler: commands::extra::cmd_add, dot_command: false },
        CommandDef { name: "import", aliases: &[], description: "Import volumes from magazine", handler: commands::extra::cmd_import, dot_command: false },
        CommandDef { name: "export", aliases: &[], description: "Export volumes to magazine", handler: commands::extra::cmd_export, dot_command: false },
        CommandDef { name: "move", aliases: &[], description: "Move volume between pools", handler: commands::extra::cmd_move, dot_command: false },

        // =================================================================
        // DOT COMMANDS
        // =================================================================
        CommandDef { name: ".help", aliases: &[], description: "List commands (machine format)", handler: commands::basic::cmd_dot_help, dot_command: true },
        CommandDef { name: ".quit", aliases: &[], description: "Disconnect", handler: commands::basic::cmd_quit, dot_command: true },
        CommandDef { name: ".status", aliases: &[], description: "Status (machine format)", handler: commands::status::cmd_dot_status, dot_command: true },
        CommandDef { name: ".", aliases: &[], description: "No-op", handler: commands::dot::cmd_dot_noop, dot_command: true },

        // Resource listing
        CommandDef { name: ".clients", aliases: &[], description: "List client names", handler: commands::dot::cmd_dot_clients, dot_command: true },
        CommandDef { name: ".jobs", aliases: &[], description: "List job names", handler: commands::dot::cmd_dot_jobs, dot_command: true },
        CommandDef { name: ".pools", aliases: &[], description: "List pool names", handler: commands::dot::cmd_dot_pools, dot_command: true },
        CommandDef { name: ".storage", aliases: &[".storages"], description: "List storage names", handler: commands::dot::cmd_dot_storage, dot_command: true },
        CommandDef { name: ".filesets", aliases: &[], description: "List fileset names", handler: commands::dot::cmd_dot_filesets, dot_command: true },
        CommandDef { name: ".schedule", aliases: &[], description: "List schedule names", handler: commands::dot::cmd_dot_schedule, dot_command: true },
        CommandDef { name: ".catalogs", aliases: &[], description: "List catalog names", handler: commands::dot::cmd_dot_catalogs, dot_command: true },
        CommandDef { name: ".consoles", aliases: &[], description: "List console names", handler: commands::dot::cmd_dot_consoles, dot_command: true },
        CommandDef { name: ".jobdefs", aliases: &[], description: "List jobdefs names", handler: commands::dot::cmd_dot_jobdefs, dot_command: true },
        CommandDef { name: ".profiles", aliases: &[], description: "List profile names", handler: commands::dot::cmd_dot_profiles, dot_command: true },
        CommandDef { name: ".locations", aliases: &[], description: "List location names", handler: commands::dot::cmd_dot_locations, dot_command: true },
        CommandDef { name: ".media", aliases: &[], description: "List volume names", handler: commands::dot::cmd_dot_media, dot_command: true },
        CommandDef { name: ".mediatypes", aliases: &[], description: "List media types", handler: commands::dot::cmd_dot_mediatypes, dot_command: true },
        CommandDef { name: ".messages", aliases: &[], description: "List message resource names", handler: commands::dot::cmd_dot_messages, dot_command: true },
        CommandDef { name: ".msgs", aliases: &[], description: "List pending message types", handler: commands::dot::cmd_dot_msgs, dot_command: true },
        CommandDef { name: ".users", aliases: &[], description: "List user names", handler: commands::dot::cmd_dot_users, dot_command: true },

        // Query
        CommandDef { name: ".defaults", aliases: &[], description: "Show default values", handler: commands::dot::cmd_dot_defaults, dot_command: true },
        CommandDef { name: ".jobstatus", aliases: &[], description: "List job status codes", handler: commands::dot::cmd_dot_jobstatus, dot_command: true },
        CommandDef { name: ".authorized", aliases: &[], description: "Check command authorization", handler: commands::dot::cmd_dot_authorized, dot_command: true },
        CommandDef { name: ".levels", aliases: &[], description: "List backup levels", handler: commands::dot::cmd_dot_levels, dot_command: true },
        CommandDef { name: ".types", aliases: &[], description: "List job types", handler: commands::dot::cmd_dot_types, dot_command: true },
        CommandDef { name: ".volstatus", aliases: &[], description: "List volume status values", handler: commands::dot::cmd_dot_volstatus, dot_command: true },
        CommandDef { name: ".actiononpurge", aliases: &[], description: "List action-on-purge options", handler: commands::dot::cmd_dot_actiononpurge, dot_command: true },

        // API/protocol
        CommandDef { name: ".api", aliases: &[], description: "Set API mode (0/1/2)", handler: commands::dot::cmd_dot_api, dot_command: true },
        CommandDef { name: ".grpc", aliases: &[], description: "Toggle gRPC mode", handler: commands::dot::cmd_dot_grpc, dot_command: true },

        // SQL
        CommandDef { name: ".sql", aliases: &[], description: "Execute SQL", handler: commands::sql::cmd_dot_sql, dot_command: true },
        CommandDef { name: ".sqltables", aliases: &[], description: "List catalog tables", handler: commands::dot::cmd_dot_sqltables, dot_command: true },
        CommandDef { name: ".sqlschema", aliases: &[], description: "Show catalog schema", handler: commands::dot::cmd_dot_sqlschema, dot_command: true },
        CommandDef { name: ".sqlcount", aliases: &[], description: "Count catalog records", handler: commands::dot::cmd_dot_sqlcount, dot_command: true },

        // ACL
        CommandDef { name: ".acl", aliases: &[".profile"], description: "Manage console ACLs", handler: commands::acl::cmd_dot_acl, dot_command: true },

        // Console grammar for completion and man pages (Onesimus extension)
        CommandDef { name: ".grammar", aliases: &[], description: "Print the console grammar (Onesimus)", handler: commands::grammar::cmd_dot_grammar, dot_command: true },

        // Catalog

        // BVFS
        CommandDef { name: ".bvfs_update", aliases: &[], description: "Update BVFS cache", handler: commands::bvfs::cmd_bvfs_update, dot_command: true },
        CommandDef { name: ".bvfs_cleanup", aliases: &[], description: "Cleanup BVFS temp tables", handler: commands::bvfs::cmd_bvfs_cleanup, dot_command: true },
        CommandDef { name: ".bvfs_clear_cache", aliases: &[], description: "Clear BVFS cache", handler: commands::bvfs::cmd_bvfs_clear_cache, dot_command: true },
        CommandDef { name: ".bvfs_get_jobids", aliases: &[], description: "Get job IDs for restore", handler: commands::bvfs::cmd_bvfs_get_jobids, dot_command: true },
        CommandDef { name: ".bvfs_lsdirs", aliases: &[], description: "List directories (BVFS)", handler: commands::bvfs::cmd_bvfs_lsdirs, dot_command: true },
        CommandDef { name: ".bvfs_lsfiles", aliases: &[], description: "List files (BVFS)", handler: commands::bvfs::cmd_bvfs_lsfiles, dot_command: true },
        CommandDef { name: ".bvfs_versions", aliases: &[], description: "List file versions", handler: commands::bvfs::cmd_bvfs_versions, dot_command: true },
        CommandDef { name: ".bvfs_restore", aliases: &[], description: "Restore from BVFS selection", handler: commands::bvfs::cmd_bvfs_restore, dot_command: true },

        // Configuration CRUD (Onesimus extension, raw text; same logic as the DirectorApi RPCs)
        CommandDef { name: ".config", aliases: &[], description: "Read, create, change, delete configuration resources as raw text (Onesimus)", handler: commands::config_crud::cmd_dot_config, dot_command: true },

        // OVFS (Onesimus extension, no Bareos equivalent; exact names only)
        CommandDef { name: ".ovfs_status", aliases: &[], description: "Show whether restore trees use the ovfs extension or the Director fallback", handler: commands::ovfs::cmd_ovfs_status, dot_command: true },
        CommandDef { name: ".ovfs_latest", aliases: &[], description: "Job chain of a client's latest state (restore option 5)", handler: commands::ovfs::cmd_ovfs_latest, dot_command: true },
        CommandDef { name: ".ovfs_tree", aliases: &[], description: "Restore tree of a job chain as JSON (OVFS)", handler: commands::ovfs::cmd_ovfs_tree, dot_command: true },
        CommandDef { name: ".ovfs_ls", aliases: &[], description: "Direct children of one directory of a job chain (OVFS)", handler: commands::ovfs::cmd_ovfs_ls, dot_command: true },
        CommandDef { name: ".ovfs_versions", aliases: &[], description: "All versions of one file of a client (OVFS)", handler: commands::ovfs::cmd_ovfs_versions, dot_command: true },
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
/// `audit`: a denial is audited (AclAccessOk(Command_ACL, key, true)).
fn resolve_permitted<'a>(registry: &'a [CommandDef], ast: &CommandAst, ctx: &ExecContext, audit: bool) -> Option<(&'a str, &'a CommandDef)> {
    let name = resolve_command_name(registry, &ast.command_name)?;
    let command = find_command(registry, name)?;
    let acl_name=if crate::ua_commands_table::UA_COMMANDS.iter().any(|entry|entry.0==name) {name} else {command.name};
    let access = if audit { crate::acl::access_audited } else { crate::acl::access };
    if !matches!(acl_name, "exit" | "quit" | "whoami") && !access(ctx, crate::acl::Kind::Command, acl_name) {
        return None;
    }
    Some((name, command))
}

/// True when Do_a_command finds no permitted command; it then sends neither
/// CMD_BEGIN nor CMD_OK/CMD_FAILED in API mode.
pub fn is_invalid_command(registry: &[CommandDef], ast: &CommandAst, ctx: &ExecContext) -> bool {
    resolve_permitted(registry, ast, ctx, false).is_none()
}

/// Preserve the resolved alias (e.g. llist) for handlers with alias-specific output.
pub fn dispatch_registry(registry: &[CommandDef], ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some((name, command)) = resolve_permitted(registry, ast, ctx, true) else {
        // ua_cmds.cc Do_a_command(): ErrorMsg("%s: is an invalid command.\n", argk[0])
        return CommandResult::error(format!("{}: is an invalid command.\n", ast.command_name));
    };
    let mut resolved = ast.clone();
    resolved.command_name = name.to_string();
    // ua_output.cc gui_cmd: `jcr->gui` changes the catalog list format.
    let gui = ctx.session_mgr.as_ref()
        .and_then(|m| m.with_session_catalog(ctx.session_id, |c| c.gui))
        .unwrap_or(false);
    crate::sql_list::with_gui(gui, || (command.handler)(&resolved, ctx))
}

/// UaContext::SendCmdUsage(): "<message>\nUSAGE: <key> <usage>\n" (a SendMsg).
pub fn cmd_usage(key: &str, message: &str) -> String {
    match crate::ua_commands_table::UA_COMMANDS.iter().find(|entry| entry.0 == key) {
        Some((key, _, usage)) => format!("{message}\nUSAGE: {key} {usage}\n"),
        None => message.to_string(),
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
