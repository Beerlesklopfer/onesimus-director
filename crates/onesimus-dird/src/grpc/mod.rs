//! gRPC server — dir:grpc thread.
//!
//! Runs a shared tonic gRPC server in its own tokio runtime.
//! Unlike the C++ per-client model, this uses a single server with
//! session multiplexing (fixes the >16 connection scaling issue).

pub mod service;
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
use bareos_proto_rs::bareos::director::director_server::DirectorServer;

/// Everything the gRPC server needs besides the shared queues.
pub struct GrpcSettings {
    pub bind_address: String,
    pub port: u16,
    pub bnet_port: u16,
    pub director_name: String,
    pub identity: tls::GrpcIdentity,
    pub auth: Arc<auth::AuthState>,
}

/// Run the gRPC server (TLS only). Called from the dir:grpc named thread.
/// Creates its own tokio runtime.
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
        let svc = DirectorServiceImpl::new(
            b, q, settings.auth, settings.director_name, settings.bnet_port,
        );
        let tls = ServerTlsConfig::new()
            .identity(Identity::from_pem(&settings.identity.cert_pem, &settings.identity.key_pem));

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
            .serve_with_shutdown(addr, async { let _ = shutdown_rx.await; });

        if let Err(e) = result.await {
            error!("gRPC server error: {e}");
        }

        info!("gRPC server stopped");
    });
}
