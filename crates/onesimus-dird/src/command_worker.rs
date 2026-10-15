//! Command worker thread — processes commands from the CommandQueue.
//!
//! Takes commands submitted by gRPC handlers or other producers,
//! dispatches them through the command executor, and sends results back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{info, debug, warn};

use crate::catalog_worker::CatalogHandle;
use crate::command_ast::ResponseFormat;
use crate::command_executor::{self, ApiMode, ExecContext};
use crate::job_manager::JobManager;
use crate::command_queue::CommandQueue;
use crate::director_config::SharedConfig;
use crate::session_manager::SessionManager;

/// Command worker loop. Runs in the dir:cmdworker named thread.
pub fn command_worker_loop(
    cmd_queue: &Arc<CommandQueue>,
    config: Option<SharedConfig>,
    session_mgr: Option<Arc<SessionManager>>,
    catalog: Option<CatalogHandle>,
    job_manager: Option<Arc<std::sync::Mutex<JobManager>>>,
    shutdown: &AtomicBool,
) {
    info!("command worker started");

    while !shutdown.load(Ordering::Relaxed) {
        if let Some(cmd) = cmd_queue.take() {
            let ctx = ExecContext {
                session_id: cmd.session_id,
                config: config.clone(),
                session_mgr: session_mgr.clone(),
                catalog: catalog.clone(),
                api_mode: match cmd.ast.response_format {
                    ResponseFormat::Json => ApiMode::Json,
                    ResponseFormat::Text => ApiMode::Human,
                    _ => ApiMode::Api,
                },
                bus_rx: None,
                job_manager: job_manager.clone(),
            };

            debug!("executing command '{}' from queue", cmd.ast.command_name);
            let mut result = command_executor::dispatch(&cmd.ast, &ctx);
            // JSON clients get the same json-rpc envelope as BNET `.api 2`
            if ctx.api_mode == ApiMode::Json {
                result.text_output = crate::json_output::envelope(&result, false);
            }

            // Send result back to the submitter
            if cmd.result_tx.send(result).is_err() {
                warn!("command result receiver dropped");
            }
        }
    }

    info!("command worker stopped");
}
