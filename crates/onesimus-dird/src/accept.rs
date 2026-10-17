//! TCP accept loop — dir:accept thread.
//!
//! Listens for incoming BNET connections from consoles and file daemons.
//! Each connection gets its own named thread.
//! Supports optional TLS (required for C++ bconsole compatibility).

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tracing::{info, error, debug, warn};

use bareos_bnet::{BnetStream, Hello};
use bareos_bnet::hello::{name_type_and_version_from_hello, parse_hello};
use bareos_bnet::tls::{self, TlsConfig, TlsOrPlain};

use crate::catalog_worker::CatalogHandle;
use crate::session_manager::SessionManager;
use crate::command_queue::CommandQueue;
use crate::session;

/// Accept loop configuration.
use crate::director_config::SharedConfig;

pub struct AcceptConfig {
    pub bind_address: String,
    pub port: u16,
    pub password: String,
    pub tls_config: Option<TlsConfig>,
    pub shared_config: Option<SharedConfig>,
    pub job_manager: Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
    pub catalog: Option<CatalogHandle>,
}

impl Default for AcceptConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            port: 9101,
            password: String::new(),
            tls_config: None,
            shared_config: None,
            job_manager: None,
            catalog: None,
        }
    }
}

/// Main accept loop. Runs in the dir:accept named thread.
pub fn accept_loop(
    session_mgr: &Arc<SessionManager>,
    cmd_queue: &Arc<CommandQueue>,
    config: &AcceptConfig,
    shutdown: &AtomicBool,
) {
    let addr = format!("{}:{}", config.bind_address, config.port);

    let listener = match TcpListener::bind(&addr) {
        Ok(l) => {
            info!("listening on {addr}");
            l
        }
        Err(e) => {
            error!("FATAL: failed to bind {addr}: {e}");
            error!("Another director already running? Check: ss -tlnp | grep {}", config.port);
            std::process::exit(1);
        }
    };

    // Build TLS server config if certs are provided
    let tls_server_config = config.tls_config.as_ref().and_then(|tc| {
        if tc.certfile.is_some() && tc.keyfile.is_some() {
            match tls::build_server_config(tc) {
                Ok(cfg) => {
                    info!("TLS enabled");
                    Some(cfg)
                }
                Err(e) => {
                    error!("TLS config error: {e}");
                    None
                }
            }
        } else {
            None
        }
    });

    // Set non-blocking so we can check shutdown flag
    listener.set_nonblocking(true).ok();

    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                debug!("new connection from {peer_addr}");
                stream.set_nonblocking(false).ok();
                stream.set_nodelay(true).ok(); // Disable Nagle — EOD signal must arrive immediately

                let smgr = session_mgr.clone();
                let q = cmd_queue.clone();
                let password = config.password.clone();
                let tls_cfg = tls_server_config.clone();
                let shared_cfg = config.shared_config.clone();
                let job_mgr = config.job_manager.clone();
                let cat = config.catalog.clone();
                let session_id = crate::session_manager::next_session_id();

                thread::Builder::new()
                    .name(format!("dir:session:{session_id}"))
                    .spawn(move || {
                        handle_connection(stream, session_id, &password, tls_cfg, &smgr, &q, &shared_cfg, &job_mgr, &cat);
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

/// Handle a single incoming connection.
fn handle_connection(
    stream: TcpStream,
    session_id: u64,
    _password: &str,
    tls_config: Option<Arc<rustls::ServerConfig>>,
    session_mgr: &Arc<SessionManager>,
    cmd_queue: &CommandQueue,
    shared_config: &Option<SharedConfig>,
    job_manager: &Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
    catalog: &Option<CatalogHandle>,
) {
    // Peek at first byte to detect TLS ClientHello (0x16) vs plaintext BNET
    // Use a short read timeout so we don't block forever
    // Console-Sessions: kein harter read-timeout — User kann lange am Prompt sitzen.
    // Die Session endet durch quit/SIGTERM/Connection-Reset, nicht durch Timeout.
    stream.set_read_timeout(None).ok();
    if let Ok(peer) = stream.peer_addr() {
        crate::audit::set_peer_host(peer.ip().to_string());
    }
    let mut peek_buf = [0u8; 1];
    match stream.peek(&mut peek_buf) {
        Ok(1) => {
            if peek_buf[0] == 0x16 {
                // TLS ClientHello — TLS-PSK, key chosen by the PSK identity
                // ("R_CONSOLE\x1e<console>"), like the C++ Director
                let cfg = shared_config.clone();
                let lookup = move |identity: &str| psk_for_identity(&cfg, identity);
                match bareos_bnet::tls_psk::psk_accept_with(stream, lookup) {
                    Ok((psk_stream, identity)) => {
                        debug!("session {session_id}: TLS-PSK accept OK (identity {})", identity.replace('\x1e', "/"));
                        let cipher = psk_stream.cipher_name();
                        handle_hello(BnetStream::new(psk_stream), session_id, Some(identity), Some(cipher),
                                     session_mgr, cmd_queue, shared_config, job_manager, catalog);
                    }
                    Err(e) => {
                        debug!("session {session_id}: TLS-PSK accept failed: {e}");
                    }
                }
            } else {
                // Plaintext BNET — first byte is part of i32 length header
                debug!("session {session_id}: plaintext connection detected");
                handle_hello(BnetStream::new(stream), session_id, None, None,
                             session_mgr, cmd_queue, shared_config, job_manager, catalog);
            }
        }
        Ok(_) | Err(_) => {
            debug!("session {session_id}: peek failed, dropping connection");
        }
    }
}

/// Password a console authenticates with: the Director password for the
/// default console ("*UserAgent*"), the Console resource's password for a
/// named console. None if unknown or empty (connection is refused).
fn console_password(shared_config: &Option<SharedConfig>, name: &str) -> Option<String> {
    let cfg = shared_config.as_ref()?.read().ok()?;
    let pw = if name == "*UserAgent*" {
        cfg.director.password.clone()
    } else {
        cfg.consoles.iter().find(|c| c.name == name)?.password.clone()
    };
    (!pw.is_empty()).then_some(pw)
}

/// TLS-PSK key for a client identity "R_CONSOLE\x1e<name>" / "R_CLIENT\x1e<name>".
fn psk_for_identity(shared_config: &Option<SharedConfig>, identity: &str) -> Option<Vec<u8>> {
    let (kind, name) = identity.split_once('\x1e')?;
    let password = match kind {
        "R_CONSOLE" => console_password(shared_config, name)?,
        "R_CLIENT" => {
            let cfg = shared_config.as_ref()?.read().ok()?;
            let pw = cfg.clients.iter().find(|c| c.name == name)?.password.clone();
            if pw.is_empty() { return None; }
            pw
        }
        _ => return None,
    };
    Some(bareos_bnet::tls_psk::password_to_psk(&password))
}

/// Handle connection after stream type is determined.
fn handle_hello<S: std::io::Read + std::io::Write>(
    mut bnet: BnetStream<S>,
    session_id: u64,
    tls_identity: Option<String>,
    tls_cipher: Option<String>,
    session_mgr: &Arc<SessionManager>,
    cmd_queue: &CommandQueue,
    shared_config: &Option<SharedConfig>,
    job_manager: &Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
    catalog: &Option<CatalogHandle>,
) {

    // Read hello message
    let hello_msg = match bnet.recv_str() {
        Ok(msg) => msg,
        Err(e) => {
            debug!("session {session_id}: failed to read hello: {e}");
            return;
        }
    };

    let hello = match parse_hello(&hello_msg) {
        Some(h) => h,
        None => {
            warn!("session {session_id}: invalid hello: {hello_msg}");
            return;
        }
    };

    let console_name = match &hello {
        Hello::DefaultConsole => Some("*UserAgent*".to_string()),
        Hello::NamedConsole { name, .. } => Some(name.clone()),
        Hello::FileDaemon { .. } => None,
    };
    if let Some(name) = console_name {
        // ua_server.cc HandleUserAgentClientRequest(): new_control_jcr("-Console-")
        // takes a CreateUniqueJobName() sequence number before authentication.
        let _ = crate::commands::job::new_control_jcr_name("-Console-");
        // The Hello must name the console the TLS-PSK key belongs to
        if let Some(id) = &tls_identity {
            if *id != format!("R_CONSOLE\x1e{name}") {
                warn!("session {session_id}: console '{name}' does not match TLS-PSK identity, rejected");
                return;
            }
        }
        // Default console: Director password; named console: its own password
        let Some(password) = console_password(shared_config, &name) else {
            warn!("session {session_id}: unknown console '{name}' or no password configured, rejected");
            return;
        };
        // C++ CreateConsoleAuthenticator: the Hello version selects the
        // protocol (before/from 18.2); an unparsable version is an invalid Hello
        let Some((_, _, version_number)) = name_type_and_version_from_hello(&hello_msg) else {
            warn!("session {session_id}: UA Hello is invalid. Got: {hello_msg}");
            return;
        };
        let connection = session::ConsoleConnection { version: version_number, tls_cipher };
        if let Hello::NamedConsole { version, wants_grpc, .. } = &hello {
            debug!("named console '{name}' v{version} connected (grpc={wants_grpc})");
        } else {
            debug!("default console connected (session {session_id})");
        }
        session::handle_console_session_with(
            bnet, session_id, &name, &password, &connection, session_mgr, cmd_queue, shared_config, job_manager, catalog,
        );
        return;
    }

    match hello {
        Hello::DefaultConsole | Hello::NamedConsole { .. } => unreachable!(),
        Hello::FileDaemon { client_name, fd_protocol_version } => {
            debug!("FD '{client_name}' connected (proto={fd_protocol_version})");
            session_mgr.add_fd_connection(&client_name, fd_protocol_version);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_config_defaults() {
        let config = AcceptConfig::default();
        assert_eq!(config.bind_address, "0.0.0.0");
        assert_eq!(config.port, 9101);
    }

    #[test]
    fn hello_routing() {
        assert!(matches!(
            parse_hello("Hello *UserAgent* calling"),
            Some(Hello::DefaultConsole)
        ));
        assert!(matches!(
            parse_hello("Hello admin calling version 25.0.0"),
            Some(Hello::NamedConsole { .. })
        ));
        assert!(matches!(
            parse_hello("Hello Client bareos-fd FdProtocolVersion=54 calling"),
            Some(Hello::FileDaemon { .. })
        ));
    }
}
