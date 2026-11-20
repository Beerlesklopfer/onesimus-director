//! Verbindungen annehmen und aufbauen: Director, aktiver und passiver FD.
//!
//! Zweck: Auf dem `SD Port` rufen Director und File Daemons an. Das erste
//! Byte entscheidet über TLS-PSK (Schlüssel nach PSK-Identität: Director →
//! sein Passwort, `R_JOB` → der Schlüssel des Jobs), die erste Zeile über
//! die Art: `Hello Start Job <Job>` ist ein FD (aktiver Client), alles
//! andere geht an die Director-Sitzung. Bei passivem Client
//! (`Passive = yes`) baut osd die Verbindung zum FD selbst auf
//! ([`connect_passive_fd`]). Danach ist die Datenphase in beiden Fällen
//! dieselbe ([`crate::fd_session`]).
//!
//! Bareos-Gegenstück: `core/src/stored/socket_server.cc`
//! (`HandleConnectionRequest`), `stored/fd_cmds.cc`
//! (`HandleFiledConnection`), `stored/authenticate.cc`
//! (`AuthenticateFiledaemon`, `AuthenticateWithFiledaemon`),
//! `stored/dir_cmd.cc` (`PassiveCmd`).

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bareos_bnet::{tls_psk, BnetStream, CramMd5Auth};
use onesimus_daemon::accept::{open_bnet, PskIdentity, Rw};

use crate::daemon::Daemon;
use crate::jobs::{FdConnection, Job};

/// Shortest valid Hello (`MIN_MSG_LEN`).
const MIN_HELLO: usize = 25;

/// TLS policy `kBnetTlsAuto` the Director sends as `ssl=4`.
const TLS_AUTO: i32 = 4;

/// The TLS-PSK key for a peer's identity.
fn psk_for(d: &Daemon, identity: &str) -> Option<Vec<u8>> {
    match PskIdentity::parse(identity)? {
        PskIdentity::Director(name) => d.config.find_director(name).map(|r| tls_psk::password_to_psk(&r.password)),
        PskIdentity::Job(job) => d.jobs.find(job).map(|j| j.auth_key.clone().into_bytes()),
        PskIdentity::Other(..) => None,
    }
}

/// `HandleConnectionRequest`: one incoming connection.
pub fn handle_connection(d: Daemon, stream: TcpStream, session: u64) {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let tcp = stream.try_clone().ok();
    let lookup_daemon = d.clone();
    let Some(mut bnet) = open_bnet(stream, session, move |id| psk_for(&lookup_daemon, id)) else { return };
    let hello = match bnet.recv_str() {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("session {session}: Connection request failed: {e}");
            return;
        }
    };
    if hello.len() < MIN_HELLO {
        tracing::warn!("session {session}: Invalid connection. Len={}", hello.len());
        return;
    }
    if let Some(tcp) = &tcp {
        tcp.set_read_timeout(None).ok();
    }
    if let Some(job) = hello.strip_prefix("Hello Start Job ").and_then(|r| r.split_whitespace().next()) {
        handle_fd(&d, bnet, job, tcp, session);
        return;
    }
    crate::director::handle(d, bnet, &hello, session);
}

/// `HandleFiledConnection`: the FD of a job calls; authenticate it with the
/// job's key and hand the connection to the job.
fn handle_fd(d: &Daemon, mut bnet: BnetStream<Box<dyn Rw>>, job_name: &str, tcp: Option<TcpStream>, session: u64) {
    let Some(job) = d.jobs.find(job_name) else {
        tracing::error!("session {session}: FD connect failed: Job name not found: {job_name}");
        return;
    };
    if job.authenticated.load(std::sync::atomic::Ordering::Acquire) {
        tracing::error!("session {session}: JobId {} Job {} already authenticated.", job.cmd.job_id, job.cmd.job);
        return;
    }
    let auth = CramMd5Auth::new_raw(&job.auth_key, &format!("R_STORAGE::{}", d.name()), 2);
    match auth.two_way_authenticate(&mut bnet, true) {
        Ok(r) if r.success => {
            tracing::debug!("session {session}: OK Authentication jid={} Job {}", job.cmd.job_id, job.cmd.job);
            job.connect_fd(FdConnection { channel: Box::new(bnet), socket: tcp });
        }
        _ => {
            tracing::error!("session {session}: Authorization problem: Two way security handshake failed with File daemon");
            job.set_status('E');
            job.fd_arrived.notify_all();
        }
    }
}

/// `PassiveCmd`: connects to the FD at `addr:port` (retrying for up to 30 s),
/// TLS-PSK with identity `R_JOB\x1e<Job>` when `ssl` is `kBnetTlsAuto`,
/// `Hello Storage calling Start Job <Job>`, CRAM-MD5 with the job's key.
pub fn connect_passive_fd(d: &Daemon, job: &Arc<Job>, addr: &str, port: u16, ssl: i32) -> Result<FdConnection, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let tcp = loop {
        let attempt = (addr, port)
            .to_socket_addrs()
            .map_err(|e| e.to_string())
            .and_then(|mut a| a.next().ok_or_else(|| "no address".to_string()))
            .and_then(|sa| TcpStream::connect_timeout(&sa, Duration::from_secs(10)).map_err(|e| e.to_string()));
        match attempt {
            Ok(s) => break s,
            Err(e) if Instant::now() >= deadline => return Err(format!("connect {addr}:{port}: {e}")),
            Err(_) => std::thread::sleep(Duration::from_secs(1)),
        }
    };
    tcp.set_nodelay(true).ok();
    let socket = tcp.try_clone().ok();
    let stream: Box<dyn Rw> = if ssl == TLS_AUTO {
        let identity = format!("R_JOB\x1e{}", job.cmd.job);
        Box::new(tls_psk::psk_connect(tcp, &identity, job.auth_key.as_bytes()).map_err(|e| format!("TLS: {e}"))?)
    } else {
        Box::new(tcp)
    };
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&format!("Hello Storage calling Start Job {}\n", job.cmd.job)).map_err(|e| e.to_string())?;
    let auth = CramMd5Auth::new_raw(&job.auth_key, &format!("R_STORAGE::{}", d.name()), 2);
    match auth.two_way_authenticate(&mut bnet, false) {
        Ok(r) if r.success => Ok(FdConnection { channel: Box::new(bnet), socket }),
        _ => Err("Failed to authenticate File daemon.".into()),
    }
}
