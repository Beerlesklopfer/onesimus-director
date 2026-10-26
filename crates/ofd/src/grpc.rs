//! Additional TLS gRPC control listener. BNET remains available on the FD port
//! for legacy Directors and for passive Storage Daemon connections.
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use crate::config::FdConfig;
use bareos_bnet::{BnetStream, fd_grpc};
use onesimus_proto_rs::onesimus::filed::{
    ControlFrame,
    file_daemon_server::{FileDaemon, FileDaemonServer},
};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};

static SESSION: AtomicU64 = AtomicU64::new(1 << 32);
const MAX_SESSIONS: usize = 128;

struct Service {
    config: Arc<FdConfig>,
    shutdown: &'static AtomicBool,
    sessions: Arc<tokio::sync::Semaphore>,
}

#[tonic::async_trait]
impl FileDaemon for Service {
    type ControlStream =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<ControlFrame, Status>> + Send>>;

    async fn control(
        &self,
        request: Request<tonic::Streaming<ControlFrame>>,
    ) -> Result<Response<Self::ControlStream>, Status> {
        let permit = self
            .sessions
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many FD control sessions"))?;
        let mut input = request.into_inner();
        // Idle unauthenticated sessions must not retain a worker indefinitely.
        let (mut io, peer) = fd_grpc::pair(Some(Duration::from_secs(30)));
        io.cancel = Some(self.shutdown);
        let authenticated = io.authenticated.clone();
        let config = self.config.clone();
        let session = SESSION.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            loop {
                let result = tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if peer.incoming.is_disconnected() { break; }
                        continue;
                    },
                    result = input.message() => result,
                };
                match result {
                    Ok(Some(frame)) => {
                        if peer.incoming.send_async(Ok(frame)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(status) => {
                        let _ = peer
                            .incoming
                            .send_async(Err(std::io::Error::other(status)))
                            .await;
                        break;
                    }
                }
            }
        });
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            // Authentication is shared with BNET, including monitor restrictions.
            // After Hello/CRAM, established jobs may wait for volumes indefinitely.
            crate::accept::handle_session(
                BnetStream::new(Box::new(io)),
                session,
                &config,
                None,
                || authenticated.store(true, Ordering::Relaxed),
            );
        });
        Ok(Response::new(Box::pin(
            ReceiverStream::new(peer.outgoing).map(Ok),
        )))
    }
}

pub fn start(
    config: Arc<FdConfig>,
    address: SocketAddr,
    cert: &Path,
    key: &Path,
    shutdown: &'static AtomicBool,
) -> Result<(SocketAddr, std::thread::JoinHandle<()>), String> {
    let identity = Identity::from_pem(
        std::fs::read(cert).map_err(|e| e.to_string())?,
        std::fs::read(key).map_err(|e| e.to_string())?,
    );
    let mut builder = Server::builder()
        .tls_config(ServerTlsConfig::new().identity(identity))
        .map_err(|e| e.to_string())?;
    // Bind before reporting a successful startup (also catches port conflicts).
    let listener = std::net::TcpListener::bind(address).map_err(|e| e.to_string())?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let handle = std::thread::Builder::new()
        .name("fd:grpc".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("FD gRPC runtime");
            rt.block_on(async move {
                let listener =
                    tokio::net::TcpListener::from_std(listener).expect("FD gRPC listener");
                let service = FileDaemonServer::new(Service {
                    config,
                    shutdown,
                    sessions: Arc::new(tokio::sync::Semaphore::new(MAX_SESSIONS)),
                })
                .max_decoding_message_size(1_000_064)
                .max_encoding_message_size(1_000_064);
                let serve = builder
                    .add_service(service)
                    .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
                tokio::select! {
                    result = serve => if let Err(error) = result {
                        tracing::error!("FD gRPC server: {error}");
                    },
                    _ = async {
                        while !shutdown.load(Ordering::Relaxed) {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    } => {},
                }
            });
            rt.shutdown_timeout(Duration::from_secs(2));
        })
        .map_err(|e| e.to_string())?;
    tracing::info!("FD gRPC listening on {address} (TLS)");
    Ok((address, handle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bareos_bnet::{BnetMessage, BnetSignal, CramMd5Auth};

    #[test]
    fn tls_control_auth_status_monitor_and_disconnect() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let dir = std::env::temp_dir().join(format!("ofd-grpc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let certificate = dir.join("cert.pem");
        let key = dir.join("key.pem");
        std::fs::write(&certificate, cert.cert.pem()).unwrap();
        std::fs::write(&key, cert.key_pair.serialize_pem()).unwrap();
        let config = Arc::new(FdConfig::parse("Client {\n Name = grpc-test-fd\n}\nDirector {\n Name = test-dir\n Password = secret\n}\nDirector {\n Name = monitor-dir\n Password = monitor\n Monitor = yes\n}\n").unwrap());
        let shutdown = Box::leak(Box::new(AtomicBool::new(false)));
        let (addr, handle) = start(
            config,
            "127.0.0.1:0".parse().unwrap(),
            &certificate,
            &key,
            shutdown,
        )
        .unwrap();
        let connect = |ca: Vec<u8>| {
            fd_grpc::connect(
                "127.0.0.1",
                addr.port(),
                ca,
                "localhost",
                Some(Duration::from_secs(5)),
            )
        };
        let ca = cert.cert.pem().into_bytes();
        assert!(connect(b"not a certificate".to_vec()).is_err());
        let login = |name: &str, password: &str| {
            let mut fd = BnetStream::new(connect(ca.clone()).unwrap());
            fd.send_str(&bareos_bnet::hello::make_director_hello(name))
                .unwrap();
            let ok = CramMd5Auth::new(password, name, 2)
                .two_way_authenticate(&mut fd, false)
                .is_ok_and(|r| r.success);
            (fd, ok)
        };
        let (mut fd, ok) = login("test-dir", "secret");
        assert!(ok);
        assert_eq!(fd.recv_str().unwrap(), "2000 OK Hello 54\n");
        fd.send_str("status").unwrap();
        let mut status = String::new();
        loop {
            match fd.recv().unwrap() {
                BnetMessage::Data(data) => status.push_str(&String::from_utf8_lossy(&data)),
                BnetMessage::Signal(BnetSignal::Eod) => break,
                other => panic!("unexpected status frame: {other:?}"),
            }
        }
        assert!(
            status.contains("Version:") && status.contains("Running Jobs:"),
            "{status}"
        );
        // Real restore engine: gRPC control, existing BNET Storage data stream.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let sd_port = listener.local_addr().unwrap().port();
        let storage = crate::job::tests::fake_sd(listener, "grpc-sd-key");
        let restored = dir.join("restored");
        let text = |fd: &mut BnetStream<fd_grpc::ControlIo>| -> String {
            loop {
                match fd.recv().unwrap() {
                    BnetMessage::Data(bytes) => {
                        return String::from_utf8_lossy(&bytes).into_owned();
                    }
                    BnetMessage::Signal(_) => continue,
                }
            }
        };
        fd.send_str("JobId=43 Job=GrpcRestore SDid=1 SDtime=2 Authorization=grpc-sd-key ssl=0\n")
            .unwrap();
        assert!(text(&mut fd).starts_with("2000 OK Job "));
        fd.send_str(&format!(
            "storage address=127.0.0.1 port={sd_port} ssl=0 Authorization=grpc-sd-key\n"
        ))
        .unwrap();
        loop {
            if text(&mut fd).starts_with("2000 OK storage") {
                break;
            }
        }
        fd.send_str(&format!(
            "restore replace=a prelinks=0 where={}\n",
            restored.display()
        ))
        .unwrap();
        loop {
            if text(&mut fd).starts_with("2000 OK storage end") {
                break;
            }
        }
        fd.send_str("endrestore").unwrap();
        loop {
            let reply = text(&mut fd);
            if reply.starts_with("2800 End Job") {
                assert!(
                    reply.contains("TermCode=84 JobFiles=1 ReadBytes=5 JobBytes=5 Errors=0"),
                    "{reply}"
                );
                break;
            }
        }
        assert_eq!(
            std::fs::read(restored.join("x/file.txt")).unwrap(),
            b"data\n"
        );
        assert_eq!(storage.join().unwrap()[0], "Hello Start Job GrpcRestore\n");
        let _ = fd.send_signal(BnetSignal::Terminate);
        drop(fd);
        let (mut monitor, ok) = login("monitor-dir", "monitor");
        assert!(ok);
        assert!(monitor.recv_str().unwrap().starts_with("2000 OK Hello"));
        monitor.send_str("backup").unwrap();
        assert!(
            monitor
                .recv_str()
                .unwrap()
                .contains("Monitor directive enabled")
        );
        drop(monitor);
        assert!(!login("test-dir", "wrong-password").1);
        // An unknown identity must never reach the job command dispatcher.
        let mut unknown = BnetStream::new(connect(ca.clone()).unwrap());
        unknown
            .send_str("Hello Director unknown-dir calling\n")
            .unwrap();
        assert!(unknown.recv_str().is_err());
        drop(unknown);
        let (mut idle, ok) = login("test-dir", "secret");
        assert!(ok);
        assert!(idle.recv_str().unwrap().starts_with("2000 OK Hello"));
        shutdown.store(true, Ordering::SeqCst);
        handle.join().unwrap();
        assert!(idle.recv_str().is_err());
        drop(idle);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
