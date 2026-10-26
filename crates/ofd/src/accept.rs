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
//! There are no FD-to-FD connections in Bareos ("Hello Client ..." is a
//! client initiated connection to a Director, never to a File Daemon).

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use tracing::{debug, error, info, warn};

use bareos_bnet::{BnetStream, CramMd5Auth, tls_psk};

use crate::config::FdConfig;
use crate::job::Session;
use crate::sd::{self, Rw};

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

/// filed/authenticate.cc OK_hello (FD protocol version 54).
const OK_HELLO: &str = "2000 OK Hello 54\n";
const DIR_SORRY: &str = "2999 Authentication failed.\n";

/// Main accept loop for the File Daemon.
pub fn accept_loop(config: &Arc<FdConfig>, shutdown: &AtomicBool) {
    // Bind dual-stack (IPv4 + IPv6) — localhost kann ::1 oder 127.0.0.1 sein
    let addr = format!("[::]:{}", config.port);

    let listener = match TcpListener::bind(&addr) {
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

    listener.set_nonblocking(true).ok();

    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                debug!("new connection from {peer_addr}");
                stream.set_nonblocking(false).ok();

                let cfg = config.clone();
                let session_id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);

                thread::Builder::new()
                    .name(format!("fd:session:{session_id}"))
                    .spawn(move || {
                        handle_connection(stream, session_id, cfg);
                    })
                    .ok();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => {
                warn!("accept error: {e}");
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }

    info!("accept loop stopped");
}

/// The TLS-PSK key for a peer's PSK identity.
fn psk_for_identity(config: &FdConfig, identity: &str) -> Option<Vec<u8>> {
    let (kind, name) = identity.split_once('\x1e')?;
    match kind {
        "R_DIRECTOR" => config.find_director(name).map(|d| tls_psk::password_to_psk(&d.password)),
        "R_JOB" => sd::psk_for_job(name),
        _ => None,
    }
}

/// Handle a single incoming connection.
fn handle_connection(stream: TcpStream, session_id: u64, config: Arc<FdConfig>) {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let Ok(tcp) = stream.try_clone() else { return };

    // Peek first byte: 0x16 = TLS ClientHello, else plaintext BNET
    let mut peek_buf = [0u8; 1];
    let bnet: BnetStream<Box<dyn Rw>> = match stream.peek(&mut peek_buf) {
        Ok(1) if peek_buf[0] == 0x16 => {
            let cfg = config.clone();
            match tls_psk::psk_accept_with(stream, move |id| psk_for_identity(&cfg, id)) {
                Ok((s, identity)) => {
                    debug!("session {session_id}: TLS-PSK accept OK ({})", identity.replace('\x1e', ":"));
                    BnetStream::new(Box::new(s))
                }
                Err(e) => {
                    debug!("session {session_id}: TLS-PSK failed: {e}");
                    return;
                }
            }
        }
        Ok(1) => {
            debug!("session {session_id}: plaintext connection");
            BnetStream::new(Box::new(stream))
        }
        _ => {
            debug!("session {session_id}: peek failed");
            return;
        }
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

    // AuthenticateDirector(): "Hello Director %s calling"
    let dir_name = match hello.strip_prefix("Hello Director ").and_then(|r| r.split_whitespace().next()) {
        Some(name) if hello.len() >= 25 && hello.len() <= 500 => name.replace('\x01', " "),
        _ => {
            warn!("session {session_id}: Bad Hello command from Director: {}", hello.trim_end());
            return;
        }
    };
    let Some(director) = config.find_director(&dir_name) else {
        warn!("session {session_id}: Connection from unknown Director {dir_name} rejected.");
        return;
    };

    // CRAM-MD5 two-way authentication (FD is server side)
    let auth = CramMd5Auth::new(&director.password, &format!("R_CLIENT::{}", config.name), 2);
    match auth.two_way_authenticate(&mut bnet, true) {
        Ok(result) if result.success => {}
        _ => {
            warn!("session {session_id}: Unable to authenticate Director {dir_name}.");
            let _ = bnet.send_str(DIR_SORRY);
            return;
        }
    }
    if bnet.send_str(OK_HELLO).is_err() {
        return;
    }
    // Jobs wait for the Director as long as it takes (mount requests ...).
    if let Some(tcp) = tcp { tcp.set_read_timeout(None).ok(); }
    info!("session {session_id}: director '{dir_name}' authenticated");

    on_authenticated();
    Session::new(config, director, session_id).run(&mut bnet);
    info!("session {session_id} ({dir_name}) ended");
}
