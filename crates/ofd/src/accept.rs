//! TCP accept loop — fd:accept thread (filed/socket_server.cc).
//!
//! Listens for incoming BNET connections. Each connection gets its own
//! named thread (fd:session:N). Like the original, two peers connect to a
//! File Daemon:
//! - a Director: "Hello Director <name> calling" (handle_director_connection)
//! - a Storage daemon in passive client mode: "Hello Storage calling Start
//!   Job <Job>" (handle_stored_connection), for a job already running here.
//!
//! TLS-PSK comes first when the peer starts with a ClientHello; the key is
//! chosen by the PSK identity: "R_DIRECTOR\x1e<name>" → the Director's
//! password, "R_JOB\x1e<Job>" → the job's Storage daemon authorization key.
//! The loop, the TLS-PSK/plaintext choice and the Director handshake are
//! onesimus-daemon (`accept`, `handshake`) since 20.11.2026; binding the
//! port, the key lookup and the session stay here.
//! There are no FD-to-FD connections in Bareos ("Hello Client ..." is a
//! client initiated connection to a Director, never to a File Daemon).

use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use tracing::{debug, error, info, warn};

use bareos_bnet::{BnetStream, tls_psk};
use onesimus_daemon::accept::PskIdentity;
use onesimus_daemon::handshake::{DirectorAuthError, HelloReplies, authenticate_director};

use crate::config::FdConfig;
use crate::job::Session;
use crate::sd::{self, Rw};

/// filed/authenticate.cc OK_hello (FD protocol version 54).
const OK_HELLO: &str = "2000 OK Hello 54\n";
const DIR_SORRY: &str = "2999 Authentication failed.\n";

/// Main accept loop for the File Daemon.
pub fn accept_loop(config: &Arc<FdConfig>, shutdown: &AtomicBool) {
    // FD Address, else dual-stack (IPv4 + IPv6) on every interface.
    let addr = match config.listen_addresses(config.port) {
        Ok(a) => a[0],
        Err(e) => {
            error!("FATAL: {e}");
            std::process::exit(1);
        }
    };

    let listener = match bareos_bnet::listen::bind(addr) {
        Ok(l) => {
            info!("listening on {addr}");
            l
        }
        Err(e) => {
            error!("FATAL: failed to bind {addr}: {e}");
            error!("Another file daemon already running? Check: ss -tlnp | grep {}", config.port);
            std::process::exit(1);
        }
    };

    let cfg = config.clone();
    onesimus_daemon::accept::serve(listener, "fd:session", shutdown, move |stream, session_id| {
        handle_connection(stream, session_id, cfg.clone());
    });
}

/// The TLS-PSK key for a peer's PSK identity.
fn psk_for_identity(config: &FdConfig, identity: &str) -> Option<Vec<u8>> {
    match PskIdentity::parse(identity)? {
        PskIdentity::Director(name) => config.find_director(name).map(|d| tls_psk::password_to_psk(&d.password)),
        PskIdentity::Job(name) => sd::psk_for_job(name),
        PskIdentity::Other(..) => None,
    }
}

/// Handle a single incoming connection.
fn handle_connection(stream: TcpStream, session_id: u64, config: Arc<FdConfig>) {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let Ok(tcp) = stream.try_clone() else { return };

    // Peek first byte: 0x16 = TLS ClientHello, else plaintext BNET
    let cfg = config.clone();
    let Some(bnet) = onesimus_daemon::accept::open_bnet(stream, session_id, move |id| psk_for_identity(&cfg, id)) else {
        return;
    };
    handle_session(bnet, session_id, &config, Some(&tcp), || {});
}

/// Authenticate the peer and run its session.
pub(crate) fn handle_session(mut bnet: BnetStream<Box<dyn Rw>>, session_id: u64, config: &FdConfig, tcp: Option<&TcpStream>, on_authenticated: impl FnOnce()) {
    let hello = match bnet.recv_str() {
        Ok(msg) => msg,
        Err(e) => {
            debug!("session {session_id}: failed to read hello: {e}");
            return;
        }
    };

    if hello.starts_with("Hello Storage") {
        if tcp.is_none() { return; }
        // Passive client: the SD connects for a running job.
        if let Some(tcp) = tcp { tcp.set_read_timeout(None).ok(); }
        sd::handle_stored_connection(bnet, &hello, &config.name);
        return;
    }

    // AuthenticateDirector(): "Hello Director %s calling", CRAM-MD5 two-way
    // authentication (FD is server side), OK_hello.
    let replies = HelloReplies { ok: OK_HELLO, sorry: DIR_SORRY };
    let own = format!("R_CLIENT::{}", config.name);
    let dir_name = match authenticate_director(&mut bnet, &hello, &own, replies, |name| {
        config.find_director(name).map(|d| d.password.clone())
    }) {
        Ok(name) => name,
        Err(DirectorAuthError::BadHello) => {
            warn!("session {session_id}: Bad Hello command from Director: {}", hello.trim_end());
            return;
        }
        Err(DirectorAuthError::UnknownDirector(dir_name)) => {
            warn!("session {session_id}: Connection from unknown Director {dir_name} rejected.");
            return;
        }
        Err(DirectorAuthError::AuthFailed(dir_name)) => {
            warn!("session {session_id}: Unable to authenticate Director {dir_name}.");
            return;
        }
        Err(DirectorAuthError::SendFailed(_)) => return,
    };
    let Some(director) = config.find_director(&dir_name) else { return };
    // Jobs wait for the Director as long as it takes (mount requests ...).
    if let Some(tcp) = tcp { tcp.set_read_timeout(None).ok(); }
    info!("session {session_id}: director '{dir_name}' authenticated");

    on_authenticated();
    Session::new(config, director, session_id).run(&mut bnet);
    info!("session {session_id} ({dir_name}) ended");
}
