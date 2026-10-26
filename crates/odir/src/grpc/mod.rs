//! gRPC server — dir:grpc thread.
//!
//! Runs a shared tonic gRPC server in its own tokio runtime.
//! Unlike the C++ per-client model, this uses a single server with
//! session multiplexing (fixes the >16 connection scaling issue).

pub mod service;
pub mod api;
pub mod coverage;
#[cfg(feature = "grpc-development")]
pub mod api_write;
#[cfg(feature = "grpc-development")]
pub mod api_dev;
pub mod auth;
pub mod push_events;
pub mod tls;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tonic::transport::{Identity, Server, ServerTlsConfig};
use tracing::{info, error};

use crate::message_bus::MessageBus;
use crate::command_queue::CommandQueue;

use service::DirectorServiceImpl;
use onesimus_proto_rs::onesimus::director::director_server::DirectorServer;
use onesimus_proto_rs::onesimus::director::api::director_api_server::DirectorApiServer;

/// What a command needs to run outside the command worker (typed API:
/// restore sessions, blocking commands), like a console session.
#[derive(Clone)]
#[cfg_attr(not(feature = "grpc-development"), allow(dead_code))]
pub struct ExecParts {
    pub config: crate::director_config::SharedConfig,
    pub session_mgr: Arc<crate::session_manager::SessionManager>,
    pub catalog: Option<crate::catalog_worker::CatalogHandle>,
    pub job_manager: Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
}

impl ExecParts {
    #[cfg_attr(not(feature = "grpc-development"), allow(dead_code))]
    pub fn context(&self, session_id: u64, api_mode: crate::command_executor::ApiMode) -> crate::command_executor::ExecContext {
        crate::command_executor::ExecContext {
            session_id,
            config: Some(self.config.clone()),
            session_mgr: Some(self.session_mgr.clone()),
            catalog: self.catalog.clone(),
            api_mode,
            bus_rx: None,
            job_manager: self.job_manager.clone(),
        }
    }
}

/// Everything the gRPC server needs besides the shared queues.
pub struct GrpcSettings {
    pub bind_address: String,
    pub port: u16,
    pub bnet_port: u16,
    pub director_name: String,
    pub identity: tls::GrpcIdentity,
    pub auth: Arc<auth::AuthState>,
    pub exec: ExecParts,
}

/// Run the gRPC server (TLS only). Called from the dir:grpc named thread.
/// Creates its own tokio runtime.
/// Set while the gRPC server is serving (shown by `.grpc`).
pub static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn run_grpc_server(
    bus: &Arc<MessageBus>,
    cmd_queue: &Arc<CommandQueue>,
    settings: GrpcSettings,
    shutdown: &AtomicBool,
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("grpc-worker")
        .enable_all()
        .build();

    let rt = match rt {
        Ok(rt) => rt,
        Err(e) => {
            error!("failed to create tokio runtime for gRPC: {e}");
            return;
        }
    };

    let b = bus.clone();
    let q = cmd_queue.clone();
    let shutdown_flag = shutdown as *const AtomicBool;

    rt.block_on(async move {
        let addr = match format!("{}:{}", settings.bind_address, settings.port).parse() {
            Ok(a) => a,
            Err(e) => {
                error!("invalid gRPC bind address {}:{}: {e}", settings.bind_address, settings.port);
                return;
            }
        };
        // Typed API (director_api.proto): same port, TLS, session tokens and ACLs
        let api = api::DirectorApiImpl::new(q.clone(), settings.auth.clone()).with_exec(settings.exec.clone());
        let svc = DirectorServiceImpl::new(
            b, q, settings.auth, settings.director_name, settings.bnet_port,
        );
        let tls = ServerTlsConfig::new()
            .identity(Identity::from_pem(&settings.identity.cert_pem, &settings.identity.key_pem));

        RUNNING.store(true, Ordering::Relaxed);
        info!("gRPC server (TLS) listening on {addr}, certificate SHA-256 {}", settings.identity.fingerprint);

        // Use a channel to signal shutdown
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        // Spawn a task to poll the shutdown flag
        let shutdown_flag = unsafe { &*shutdown_flag };
        tokio::spawn(async move {
            loop {
                if shutdown_flag.load(Ordering::Relaxed) {
                    let _ = shutdown_tx.send(());
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            }
        });

        let mut builder = match Server::builder().tls_config(tls) {
            Ok(b) => b,
            Err(e) => {
                error!("gRPC TLS setup failed: {e}");
                return;
            }
        };
        let result = builder
            .add_service(DirectorServer::new(svc))
            .add_service(DirectorApiServer::new(api))
            .serve_with_shutdown(addr, async { let _ = shutdown_rx.await; });

        if let Err(e) = result.await {
            error!("gRPC server error: {e}");
        }

        RUNNING.store(false, Ordering::Relaxed);
        info!("gRPC server stopped");
    });
}
