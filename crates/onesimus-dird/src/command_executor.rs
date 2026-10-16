//! Command dispatcher — routes CommandAST via the command registry.

use std::sync::Arc;
use std::sync::OnceLock;

use crossbeam_channel::Receiver;

use crate::catalog_worker::CatalogHandle;
use crate::command_ast::CommandAst;
use crate::command_queue::CommandResult;
use crate::command_registry::{self, CommandDef};
use crate::director_config::SharedConfig;
use crate::job_manager::JobManager;
use crate::message_bus::BusMessage;
use crate::session_manager::SessionManager;

/// API output mode (set by `.api` command, per session).
/// 0 = human-readable, 1 = machine (tab-separated), 2 = JSON
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApiMode {
    #[default]
    Human = 0,
    Api = 1,
    Json = 2,
}

/// Execution context passed to each command handler.
pub struct ExecContext {
    pub session_id: u64,
    pub config: Option<SharedConfig>,
    pub session_mgr: Option<Arc<SessionManager>>,
    pub catalog: Option<CatalogHandle>,
    pub api_mode: ApiMode,
    /// Bus receiver for this session — drains pending messages.
    pub bus_rx: Option<Receiver<BusMessage>>,
    /// Job manager for run/cancel/status commands
    pub job_manager: Option<Arc<std::sync::Mutex<JobManager>>>,
}

/// Global command registry (built once, used forever).
static REGISTRY: OnceLock<Vec<CommandDef>> = OnceLock::new();

fn registry() -> &'static [CommandDef] {
    REGISTRY.get_or_init(command_registry::build_registry)
}

/// Get the global command registry.
pub fn get_registry() -> &'static [CommandDef] {
    registry()
}

/// Dispatch a command via the registry (SSOT).
pub fn dispatch(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::sql_list::with_api1(ctx.api_mode == ApiMode::Api,
        || command_registry::dispatch_registry(registry(), ast, ctx))
}

/// Unknown or ACL-denied command (no API framing signals, see Do_a_command).
pub fn is_invalid_command(ast: &CommandAst, ctx: &ExecContext) -> bool {
    command_registry::is_invalid_command(registry(), ast, ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_ast::{CommandSource, ResponseFormat, Argument};

    fn make_ast(name: &str) -> CommandAst {
        CommandAst {
            command_name: name.to_string(),
            arguments: Vec::new(),
            session_id: String::new(),
            source: CommandSource::Cli,
            response_format: ResponseFormat::Default,
            raw_line: String::new(),
        }
    }

    fn make_ctx() -> ExecContext {
        ExecContext { session_id: 1, config: None, session_mgr: None, catalog: None, api_mode: ApiMode::Human, bus_rx: None, job_manager: None }
    }

    #[test]
    fn dispatch_version() {
        let result = dispatch(&make_ast("version"), &make_ctx());
        assert!(result.success);
        assert!(result.text_output.contains("bareos-dir Version:"));
    }

    #[test]
    fn dispatch_unknown() {
        let result = dispatch(&make_ast("nonexistent"), &make_ctx());
        assert!(!result.success);
    }

    #[test]
    fn dispatch_time() {
        let result = dispatch(&make_ast("time"), &make_ctx());
        assert!(result.success);
    }

    #[test]
    fn registry_has_all_commands() {
        let reg = registry();
        // Verify key commands exist
        assert!(command_registry::find_command(reg, "version").is_some());
        assert!(command_registry::find_command(reg, "run").is_some());
        assert!(command_registry::find_command(reg, "restore").is_some());
        assert!(command_registry::find_command(reg, "list").is_some());
        assert!(command_registry::find_command(reg, ".help").is_some());
        assert!(command_registry::find_command(reg, ".clients").is_some());
        assert!(command_registry::find_command(reg, ".bvfs_restore").is_some());
        // Aliases
        assert!(command_registry::find_command(reg, "exit").is_some());
        assert!(command_registry::find_command(reg, "llist").is_some());
        assert!(command_registry::find_command(reg, "umount").is_some());
        assert!(command_registry::find_command(reg, ".storages").is_some());
    }
}
