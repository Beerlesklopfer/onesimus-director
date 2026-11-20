//! Additional TLS gRPC listener (`onesimus.filed.FileDaemon`). BNET remains
//! available on the FD port for legacy Directors and Storage Daemons.
//!
//! - `Control`: a Director session (Hello, CRAM-MD5, commands), the same
//!   session as on the BNET port (`accept::handle_session`).
//! - `Data`: the connection of an Onesimus Storage Daemon to a passive
//!   client ("Hello Storage calling Start Job <Job>"), handed to the job
//!   like a BNET connection (`sd::handle_stored_connection`).
//!
//! Stream adapter and server loop: `onesimus-bnet-grpc` (shared with osd).
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

use crate::config::FdConfig;
use bareos_bnet::BnetStream;
use onesimus_bnet_grpc::server::{ResponseStream, accept};
use onesimus_proto_rs::onesimus::filed::{
    ControlFrame,
    file_daemon_server::{FileDaemon, FileDaemonServer},
};
use tonic::{Request, Response, Status};

static SESSION: AtomicU64 = AtomicU64::new(1 << 32);
const MAX_SESSIONS: usize = 128;
/// Idle unauthenticated sessions must not retain a worker indefinitely.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

struct Service {
    config: Arc<FdConfig>,
    shutdown: &'static AtomicBool,
    sessions: Arc<tokio::sync::Semaphore>,
}

impl Service {
    fn permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit, Status> {
        self.sessions
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many FD gRPC sessions"))
    }
}

#[tonic::async_trait]
impl FileDaemon for Service {
    type ControlStream = ResponseStream<ControlFrame>;
    type DataStream = ResponseStream<ControlFrame>;

    async fn control(
        &self,
        request: Request<tonic::Streaming<ControlFrame>>,
    ) -> Result<Response<Self::ControlStream>, Status> {
        let permit = self.permit()?;
        let (mut io, output) = accept(request.into_inner(), Some(LOGIN_TIMEOUT));
        io.cancel = Some(self.shutdown);
        let authenticated = io.authenticated.clone();
        let config = self.config.clone();
        let session = SESSION.fetch_add(1, Ordering::Relaxed);
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
        Ok(Response::new(output))
    }

    async fn data(
        &self,
        request: Request<tonic::Streaming<ControlFrame>>,
    ) -> Result<Response<Self::DataStream>, Status> {
        let permit = self.permit()?;
        let (mut io, output) = accept(request.into_inner(), Some(LOGIN_TIMEOUT));
        io.cancel = Some(self.shutdown);
        let config = self.config.clone();
        let session = SESSION.fetch_add(1, Ordering::Relaxed);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let close = io.close_handle();
            io.set_read_timeout(Some(LOGIN_TIMEOUT));
            let authenticated = io.authenticated.clone();
            let mut bnet: crate::sd::SdStream = BnetStream::new(Box::new(io));
            let hello = match bnet.recv_str() {
                Ok(h) => h,
                Err(e) => {
                    tracing::debug!("session {session}: failed to read hello: {e}");
                    return;
                }
            };
            if !hello.starts_with("Hello Storage") {
                tracing::warn!("session {session}: FileDaemon/Data accepts only a Storage daemon: {}", hello.trim_end());
                return;
            }
            tracing::info!("session {session}: Storage daemon connects over gRPC (passive client)");
            // A job's data transfer may pause for as long as a mount takes.
            authenticated.store(true, Ordering::Relaxed);
            crate::sd::handle_stored_grpc_connection(bnet, &hello, &config.name, close);
        });
        Ok(Response::new(output))
    }
}

pub fn start(
    config: Arc<FdConfig>,
    address: SocketAddr,
    cert: &Path,
    key: &Path,
    shutdown: &'static AtomicBool,
) -> Result<(SocketAddr, std::thread::JoinHandle<()>), String> {
    let tls = onesimus_daemon::grpc_tls::server_tls_config(cert, key)?;
    let started = onesimus_bnet_grpc::server::spawn("fd:grpc", address, tls, shutdown, move |builder| {
        let service = FileDaemonServer::new(Service {
            config,
            shutdown,
            sessions: Arc::new(tokio::sync::Semaphore::new(MAX_SESSIONS)),
        })
        .max_decoding_message_size(onesimus_bnet_grpc::io::MAX_MESSAGE)
        .max_encoding_message_size(onesimus_bnet_grpc::io::MAX_MESSAGE);
        builder.add_service(service)
    })?;
    tracing::info!("FD gRPC listening on {} (TLS)", started.0);
    Ok(started)
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
            let target = onesimus_bnet_grpc::Target {
                host: "127.0.0.1".into(),
                port: addr.port(),
                ca,
                server_name: "localhost".into(),
            };
            onesimus_bnet_grpc::client::connect(
                &target,
                onesimus_bnet_grpc::Rpc::FileDaemonControl,
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
        let text = |fd: &mut BnetStream<onesimus_bnet_grpc::ControlIo>| -> String {
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
