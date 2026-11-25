// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Client initiated connection to a Director (`Connection From Client To Director = yes` in its
//! Director resource).
//!
//! The FD connects to the Director's `Address`/`Port`, says
//! `Hello Client <name> FdProtocolVersion=54 calling`, answers the Director's
//! CRAM-MD5 challenge, challenges it in turn (`R_CLIENT::<name>`) and expects
//! `1000 OK: <director> Version: …`. Then it waits: the Director sends a job's
//! or a console command's first message at once, without `Hello Director`,
//! and the connection runs like an incoming Director session. When the
//! Director ends it, the FD connects again at once; after a failed attempt
//! after 60 seconds.
//!
//! Measured against bareos-dir and bareos-fd 25.1.3
//! (`onesimus-rig/tests/client_initiated_capture.rs`).
//!
//! Bareos-Gegenstück: `filed/dir_cmd.cc` (connect_to_director,
//! handle_connection_to_director), `filed/socket_server.cc`.

use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use bareos_bnet::cram_md5::CramMd5Auth;
use bareos_bnet::{BnetStream, tls_psk};
use tracing::{info, warn};

use crate::config::{DirectorAuth, FdConfig};
use crate::job::Session;
use crate::sd::Rw;

/// Retry period after a failed connection (filed/dir_cmd.cc retry_period).
const RETRY: Duration = Duration::from_secs(60);

/// FD protocol version in the Hello (FD_VERSION 54).
const FD_PROTOCOL_VERSION: u32 = 54;

/// Session ids of client initiated connections (apart from the accept loop's).
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1 << 40);

/// Connects to the Director, authenticates both ways and returns the waiting connection.
fn connect(config: &FdConfig, director: &DirectorAuth) -> Result<(BnetStream<Box<dyn Rw>>, TcpStream), String> {
    let address = director.address.as_deref().ok_or("The address config directive is missing.")?;
    if director.port == 0 {
        return Err("The port config directive is missing.".into());
    }
    use std::net::ToSocketAddrs;
    let socket = (address, director.port).to_socket_addrs().map_err(|e| format!("{address}: {e}"))?
        .next().ok_or_else(|| format!("{address}: no address"))?;
    let tcp = TcpStream::connect_timeout(&socket, Duration::from_secs(15)).map_err(|e| format!("connect {socket}: {e}"))?;
    tcp.set_nodelay(true).ok();
    tcp.set_read_timeout(Some(Duration::from_secs(60))).ok();
    let stream: Box<dyn Rw> = if director.tls_enable {
        let psk = tls_psk::password_to_psk(&director.password);
        Box::new(tls_psk::psk_connect(tcp.try_clone().map_err(|e| e.to_string())?, &format!("R_CLIENT\x1e{}", config.name), &psk)
            .map_err(|e| format!("TLS-PSK: {e}"))?)
    } else {
        Box::new(tcp.try_clone().map_err(|e| e.to_string())?)
    };
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&format!("Hello Client {} FdProtocolVersion={FD_PROTOCOL_VERSION} calling\n", config.name))
        .map_err(|e| format!("Hello: {e}"))?;
    let policy = if director.tls_enable { 2 } else { 0 };
    let auth = CramMd5Auth::new(&director.password, &format!("R_CLIENT::{}", config.name), policy);
    if !auth.two_way_authenticate(&mut bnet, false).map_err(|e| format!("authentication: {e}"))?.success {
        return Err(format!("Unable to authenticate with Director \"{}\"", director.name));
    }
    let reply = bnet.recv_str().map_err(|e| format!("Director reply: {e}"))?;
    if !reply.starts_with("1000 OK:") {
        return Err(format!("Bad response from Director: {}", reply.trim_end()));
    }
    // the Director's command may come much later
    tcp.set_read_timeout(None).ok();
    Ok((bnet, tcp))
}

/// Waits until the Director sends something on the idle connection; with `Heartbeat Interval` set,
/// a BNET heartbeat goes to the Director each interval meanwhile (keeps NAT and firewalls open).
/// False when the connection ended.
fn wait_for_director(bnet: &mut BnetStream<Box<dyn Rw>>, tcp: &TcpStream, heartbeat: Duration, shutdown: &AtomicBool) -> bool {
    let step = if heartbeat.is_zero() { Duration::from_secs(5) } else { heartbeat };
    let mut byte = [0u8; 1];
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return false;
        }
        tcp.set_read_timeout(Some(step)).ok();
        let peeked = tcp.peek(&mut byte);
        tcp.set_read_timeout(None).ok();
        match peeked {
            Ok(0) => return false,
            Ok(_) => return true,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if !heartbeat.is_zero() && bnet.send_signal(bareos_bnet::BnetSignal::Heartbeat).is_err() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
}

/// The loop of one Director with `Connection From Client To Director`: connect, wait for the
/// Director's commands, run them, connect again.
fn connect_loop(config: Arc<FdConfig>, director: DirectorAuth, shutdown: &AtomicBool) {
    while !shutdown.load(Ordering::SeqCst) {
        match connect(&config, &director) {
            Ok((mut bnet, tcp)) => {
                let session_id = NEXT_SESSION.fetch_add(1, Ordering::Relaxed);
                info!("session {session_id}: connected to Director {} (client initiated), waiting for its commands", director.name);
                if !wait_for_director(&mut bnet, &tcp, Duration::from_secs(config.heartbeat_interval_secs), shutdown) {
                    info!("session {session_id}: the Director closed the idle connection");
                    continue;
                }
                Session::new(&config, &director, session_id).run(&mut bnet);
                info!("session {session_id} ({}) ended", director.name);
            }
            Err(e) => {
                warn!("Failed to connect to Director \"{}\": {e}. Retry in 60s.", director.name);
                let until = std::time::Instant::now() + RETRY;
                while std::time::Instant::now() < until && !shutdown.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }
}

/// StartConnectToDirectorThreads(): one thread per Director resource with `Connection From Client
/// To Director = yes`.
pub fn start(config: &Arc<FdConfig>, shutdown: &'static AtomicBool) -> Vec<std::thread::JoinHandle<()>> {
    config.directors.iter()
        .filter(|d| d.connect_to_director)
        .filter_map(|d| {
            let (config, director) = (config.clone(), d.clone());
            std::thread::Builder::new()
                .name(format!("fd:dir-connect:{}", d.name))
                .spawn(move || connect_loop(config, director, shutdown))
                .ok()
        })
        .collect()
}
