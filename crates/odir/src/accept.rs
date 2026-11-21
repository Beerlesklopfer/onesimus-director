//! TCP accept loop — dir:accept thread.
//!
//! Listens for incoming BNET connections from consoles and file daemons.
//! Each connection gets its own named thread.
//! Supports optional TLS (required for C++ bconsole compatibility).

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use tracing::{info, error, debug, warn};

use bareos_bnet::{BnetStream, Hello};
use bareos_bnet::hello::{name_type_and_version_from_hello, parse_hello};
use bareos_bnet::tls::{self, TlsConfig};

use crate::catalog_worker::CatalogHandle;
use crate::session_manager::SessionManager;
use crate::command_queue::CommandQueue;
use crate::session;

/// Accept loop configuration.
use crate::director_config::SharedConfig;

pub struct AcceptConfig {
    /// Bound sockets, one per listen address (`listen::bind_all`).
    pub listeners: Vec<TcpListener>,
    pub password: String,
    pub tls_config: Option<TlsConfig>,
    pub shared_config: Option<SharedConfig>,
    pub job_manager: Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
    pub catalog: Option<CatalogHandle>,
}

impl Default for AcceptConfig {
    fn default() -> Self {
        Self {
            listeners: Vec::new(),
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
    for listener in &config.listeners {
        if let Ok(addr) = listener.local_addr() {
            info!("listening on {addr}");
        }
    }

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

    // Set non-blocking so we can check shutdown flag and serve every socket
    for listener in &config.listeners {
        listener.set_nonblocking(true).ok();
    }

    let open = Arc::new(AtomicUsize::new(0));
    while !shutdown.load(Ordering::Relaxed) {
        let mut idle = true;
        for listener in &config.listeners {
            match listener.accept() {
                Ok((stream, peer_addr)) => {
                    idle = false;
                    debug!("new connection from {peer_addr}");
                    stream.set_nonblocking(false).ok();
                    stream.set_nodelay(true).ok(); // Disable Nagle — EOD signal must arrive immediately

                    // Maximum Console Connections (read per connection, so a reload applies)
                    let limit = max_console_connections(&config.shared_config);
                    let in_use = open.fetch_add(1, Ordering::AcqRel);
                    if connection_refused(in_use, limit) {
                        open.fetch_sub(1, Ordering::AcqRel);
                        warn!("connection from {peer_addr} refused: {in_use} connections open, Maximum Console Connections = {limit}");
                        continue;
                    }
                    let slot = OpenSlot(open.clone());

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
                            let _slot = slot; // released when the session ends
                            handle_connection(stream, session_id, &password, tls_cfg, &smgr, &q, &shared_cfg, &job_mgr, &cat);
                        })
                        .ok();
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    warn!("accept error: {e}");
                }
            }
        }
        if idle {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    info!("accept loop stopped");
}

/// One connection counted against `Maximum Console Connections`.
struct OpenSlot(Arc<AtomicUsize>);

impl Drop for OpenSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The configured `Maximum Console Connections` (default 20).
fn max_console_connections(shared_config: &Option<SharedConfig>) -> usize {
    shared_config
        .as_ref()
        .and_then(|c| c.read().ok())
        .map_or(20, |c| c.director.max_console_connections as usize)
}

/// True when a connection arriving with `in_use` others open is over the limit.
fn connection_refused(in_use: usize, limit: usize) -> bool {
    in_use >= limit
}

/// Handle a single incoming connection.
fn handle_connection(
    stream: TcpStream,
    session_id: u64,
    _password: &str,
    _tls_config: Option<Arc<rustls::ServerConfig>>,
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
            // Client-initiated connections (Connection From Client To Director) are
            // not served (docs/STUBS.md S-46). Registering the name would list a
            // connection nobody authenticated and no job can use, and the entry
            // would stay after the socket is closed.
            let _ = session_mgr;
            debug!("FD '{client_name}' (proto={fd_protocol_version}) connected: client-initiated connections are not served, closed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_config_defaults() {
        let config = AcceptConfig::default();
        assert!(config.listeners.is_empty());
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

    #[test]
    fn the_connection_over_the_limit_is_refused() {
        assert!(!connection_refused(0, 20));
        assert!(!connection_refused(19, 20));
        assert!(connection_refused(20, 20));
        assert!(connection_refused(0, 0));
    }

    #[test]
    fn a_slot_is_released_when_dropped() {
        let n = Arc::new(AtomicUsize::new(1));
        drop(OpenSlot(n.clone()));
        assert_eq!(n.load(Ordering::Acquire), 0);
    }


    /// A PAM that accepts the user `testuser` with the password `testpass`
    /// and asks like `pam_unix` does: the name (echoed) if it has none, then
    /// the password (not echoed).
    struct TestPam;

    impl crate::pam_auth::Backend for TestPam {
        fn authenticate(&self, user: Option<&str>, conversation: &mut dyn crate::pam_auth::Conversation)
            -> Result<String, String> {
            let user = match user {
                Some(user) => user.to_string(),
                None => conversation.prompt(true, "login: ")?,
            };
            let password = conversation.prompt(false, "Password: ")?;
            if user == "testuser" && password == "testpass" { Ok(user) } else { Err("authentication failure".into()) }
        }
    }

    /// Runs the Director's console accept path on a loopback port for
    /// `connections` connections and returns the address.
    fn serve(connections: usize) -> (std::net::SocketAddr, thread::JoinHandle<()>) {
        crate::pam_auth::set_test_backend(Some(Arc::new(TestPam)));
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir; Password = dirpw }\n\
             Console { Name = pamc; Password = pw; Use Pam Authentication = yes }\n\
             Console { Name = plainc; Password = pw }\n\
             User { Name = testuser; Command ACL = version, .api }\n").unwrap();
        let shared: SharedConfig = Arc::new(std::sync::RwLock::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let sessions = Arc::new(SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(64))));
            let queue = CommandQueue::new(256);
            for id in 0..connections {
                let (stream, _) = listener.accept().unwrap();
                handle_connection(stream, id as u64 + 1, "", None, &sessions, &queue, &Some(shared.clone()), &None, &None);
            }
        });
        (address, handle)
    }

    /// Runs the installed original `bconsole` against `address` as the
    /// console `console` with `stdin` and returns what it printed.
    fn bconsole(address: std::net::SocketAddr, console: &str, credentials: Option<&str>, stdin: &str) -> String {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("odir-pam-{}-{}", std::process::id(), address.port()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("bconsole.conf");
        std::fs::write(&conf, format!(
            "Director {{ Name = dir; DIRport = {}; Address = 127.0.0.1; Password = \"dirpw\" }}\n\
             Console {{ Name = {console}; Password = \"pw\" }}\n", address.port())).unwrap();
        let mut command = std::process::Command::new("/usr/sbin/bconsole");
        command.arg("-c").arg(&conf).arg("-u").arg("20");
        if let Some(credentials) = credentials {
            let file = dir.join("pam.cred");
            std::fs::write(&file, credentials).unwrap();
            command.arg("-p").arg(&file);
        }
        let mut child = command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        eprintln!("--- bconsole as {console} (credentials {credentials:?}):\n{text}");
        text
    }

    /// S-43 against the installed original console: credentials from a file
    /// (`bconsole -p`), then the interactive dialog; the session runs with the
    /// ACLs of the User, a wrong password gets no session.
    #[test]
    #[ignore = "needs the original /usr/sbin/bconsole: run with --ignored"]
    fn pam_console_logs_in_through_the_original_bconsole() {
        let (address, server) = serve(5);
        // 1. credentials file: version is allowed to the User, status is not
        let out = bconsole(address, "pamc", Some("testuser\ntestpass\n"), "version\nstatus dir\nquit\n");
        assert!(out.contains("Version:"), "direct credentials: version missing\n{out}");
        assert!(!out.contains("Daemon started"), "status must be refused for the User\n{out}");
        // 2. interactive: the console answers the questions from stdin
        let out = bconsole(address, "pamc", None, "testuser\ntestpass\nversion\nquit\n");
        assert!(out.contains("Version:"), "interactive: version missing\n{out}");
        // 3. wrong password: no session, no command output
        let out = bconsole(address, "pamc", Some("testuser\nwrong\n"), "version\nquit\n");
        assert!(!out.contains("Version:"), "wrong password must not log in\n{out}");
        let out = bconsole(address, "pamc", None, "testuser\nwrong\nversion\nquit\n");
        assert!(!out.contains("Version:"), "interactive wrong password must not log in\n{out}");
        // 4. a PAM user without a User resource gets nothing
        let out = bconsole(address, "pamc", Some("nobody\ntestpass\n"), "version\nquit\n");
        assert!(!out.contains("Version:"), "unknown user\n{out}");
        server.join().unwrap();
        crate::pam_auth::set_test_backend(None);
    }
}
