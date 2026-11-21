//! Der gRPC-Zugang von osd: `onesimus.stored.StorageDaemon`.
//!
//! Zweck: gRPC mit TLS ist der Hauptweg zwischen odir/ofd und osd (Plan,
//! „Grundsatz“). Neben dem BNET-Port lauscht osd auf einem gRPC-Port
//! (osd.d `Grpc Port`, Vorgabe `SD Port` + 1000; `--disable-grpc` schaltet
//! ihn ab). Zwei RPCs:
//!
//! - `Control`: die Director-Sitzung ([`crate::director`]), genau wie über
//!   BNET: Hello, CRAM-MD5 mit dem Storage-Passwort, Befehle, CatReq.
//! - `Data`: die Datenverbindung des FD eines Jobs (aktiver Client):
//!   `Hello Start Job <Job>`, CRAM-MD5 mit dem Job-Schlüssel, dann die
//!   Datenphase ([`crate::fd_session`] bzw. [`crate::read_session`]).
//!
//! Jeder Strom wird mit `onesimus_bnet_grpc::server::accept` zu einem
//! `ControlIo` und läuft als `transport::Channel` (über
//! `bareos_bnet::BnetStream`) durch denselben Code wie eine
//! BNET-Verbindung ([`crate::server::handle_session`]). Das ist die zweite
//! Umsetzung von `Channel`, die Stufe 1 vorbereitet hat.
//!
//! TLS: eigenes Zertifikat (`--grpc-cert`/`--grpc-key`) oder ein beim
//! ersten Start im Arbeitsverzeichnis erzeugtes selbstsigniertes
//! (`onesimus-sd-grpc.crt`/`.key`, `onesimus_bnet_grpc::tls`). Der
//! Director hält es mit `Grpc CA Certificate File` in seiner
//! Storage-Ressource fest.
//!
//! Bareos-Gegenstück: keins (Onesimus-Erweiterung); die Sitzungen sind die
//! von `core/src/stored/socket_server.cc`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bareos_bnet::BnetStream;
use onesimus_bnet_grpc::server::{ResponseStream, accept};
use daemon_crate::accept::AnyBnet;
use onesimus_proto_rs::onesimus::stored::StoredFrame;
use onesimus_proto_rs::onesimus::stored::storage_daemon_server::{StorageDaemon, StorageDaemonServer};
use tonic::{Request, Response, Status};

use crate::daemon::Daemon;
use crate::jobs::Breaker;
use crate::server::{LOGIN_TIMEOUT, Peer};

/// File base name of osd's certificate and key in the working directory.
pub const CERT_BASE: &str = "onesimus-sd-grpc";

/// Most gRPC sessions at once.
const MAX_SESSIONS: usize = 128;

/// Session numbers of gRPC streams (apart from the BNET ones).
static SESSION: AtomicU64 = AtomicU64::new(1 << 32);

/// The StorageDaemon service.
struct Service {
    /// The daemon.
    daemon: Daemon,
    /// Limits the sessions.
    sessions: Arc<tokio::sync::Semaphore>,
}

impl Service {
    /// Runs one stream as a session for `peer`.
    fn session(&self, input: tonic::Streaming<StoredFrame>, peer: Peer) -> Result<ResponseStream<StoredFrame>, Status> {
        let permit = self
            .sessions
            .clone()
            .try_acquire_owned()
            .map_err(|_| Status::resource_exhausted("too many SD gRPC sessions"))?;
        let (mut io, output) = accept(input, Some(LOGIN_TIMEOUT));
        io.cancel = Some(&crate::SHUTDOWN);
        let authenticated = io.authenticated.clone();
        let breaker = Breaker::Grpc(io.close_handle());
        let d = self.daemon.clone();
        let session = SESSION.fetch_add(1, Ordering::Relaxed);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let bnet: AnyBnet = BnetStream::new(Box::new(io));
            // After the Hello a job may wait for a volume as long as it takes.
            let after_hello = move || authenticated.store(true, Ordering::Relaxed);
            crate::server::handle_session(d, bnet, session, peer, onesimus_bnet_grpc::TRANSPORT, after_hello, breaker);
        });
        Ok(output)
    }
}

#[tonic::async_trait]
impl StorageDaemon for Service {
    type ControlStream = ResponseStream<StoredFrame>;
    type DataStream = ResponseStream<StoredFrame>;

    async fn control(&self, request: Request<tonic::Streaming<StoredFrame>>) -> Result<Response<Self::ControlStream>, Status> {
        self.session(request.into_inner(), Peer::Director).map(Response::new)
    }

    async fn data(&self, request: Request<tonic::Streaming<StoredFrame>>) -> Result<Response<Self::DataStream>, Status> {
        self.session(request.into_inner(), Peer::Fd).map(Response::new)
    }
}

/// The certificate and key: `cert`/`key` if given, else generated once in
/// the working directory for the Storage name.
pub fn certificate(d: &Daemon, cert: Option<&Path>, key: Option<&Path>) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    match (cert, key) {
        (Some(c), Some(k)) => Ok((c.to_path_buf(), k.to_path_buf())),
        _ => {
            let dir = d.config.storage.working_directory.as_deref().unwrap_or(".");
            onesimus_bnet_grpc::tls::load_or_generate(dir, CERT_BASE, d.name())
        }
    }
}

/// Starts the gRPC listener on `address` with `cert`/`key`.
pub fn start(d: Daemon, address: SocketAddr, cert: &Path, key: &Path) -> Result<(SocketAddr, std::thread::JoinHandle<()>), String> {
    let tls = onesimus_bnet_grpc::tls::server_tls_config(cert, key)?;
    onesimus_bnet_grpc::server::spawn("sd:grpc", address, tls, &crate::SHUTDOWN, move |builder| {
        let service = StorageDaemonServer::new(Service { daemon: d, sessions: Arc::new(tokio::sync::Semaphore::new(MAX_SESSIONS)) })
            .max_decoding_message_size(onesimus_bnet_grpc::io::MAX_MESSAGE)
            .max_encoding_message_size(onesimus_bnet_grpc::io::MAX_MESSAGE);
        builder.add_service(service)
    })
}
