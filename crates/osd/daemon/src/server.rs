//! Verbindungen annehmen und aufbauen: Director, aktiver und passiver FD.
//!
//! Zweck: Auf dem `SD Port` (BNET) und über gRPC (`StorageDaemon`,
//! [`crate::grpc`]) rufen Director und File Daemons an. Bei BNET entscheidet
//! das erste Byte über TLS-PSK (Schlüssel nach PSK-Identität: Director →
//! sein Passwort, `R_JOB` → der Schlüssel des Jobs), bei gRPC ist der Strom
//! schon TLS. Danach ist alles gleich ([`handle_session`]): Die erste
//! Zeile entscheidet über die Art, `Hello Start Job <Job>` ist ein FD
//! (aktiver Client), alles andere geht an die Director-Sitzung. Ein
//! gRPC-Strom nimmt nur die Art an, für die seine RPC steht ([`Peer`]).
//! Bei passivem Client (`Passive = yes`) baut osd die Verbindung zum FD
//! selbst auf ([`connect_passive_fd`]): über gRPC (`FileDaemon/Data`), wenn
//! odir die gRPC-Angaben des FD mitschickt, sonst und bei jedem Fehler
//! über BNET. Danach ist die Datenphase in allen Fällen dieselbe
//! ([`crate::fd_session`], [`crate::read_session`]).
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
use daemon_crate::accept::{open_bnet, AnyBnet, PskIdentity, Rw};

use crate::daemon::Daemon;
use crate::jobs::{Breaker, FdConnection, Job};

/// Read limit while a gRPC peer logs in (Hello, CRAM-MD5).
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Which peers a connection may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peer {
    /// Director or FD (the BNET port).
    Any,
    /// Only a Director (`StorageDaemon/Control`).
    Director,
    /// Only the FD of a job (`StorageDaemon/Data`).
    Fd,
}

/// `HandleConnectionRequest`: one incoming BNET connection.
pub fn handle_connection(d: Daemon, stream: TcpStream, session: u64) {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let tcp = stream.try_clone().ok();
    let lookup_daemon = d.clone();
    let Some(bnet) = open_bnet(stream, session, move |id| psk_for(&lookup_daemon, id)) else { return };
    let unlimited = tcp.as_ref().and_then(|t| t.try_clone().ok());
    let after_hello = move || {
        if let Some(tcp) = &unlimited {
            tcp.set_read_timeout(None).ok();
        }
    };
    handle_session(d, bnet, session, Peer::Any, "BNET", after_hello, Breaker::socket(tcp));
}

/// The common part of BNET and gRPC: reads the Hello, then runs the
/// Director session or hands an FD to its job. `after_hello` lifts the
/// read limit of the login; `breaker` breaks an FD connection on cancel.
pub fn handle_session(
    d: Daemon,
    mut bnet: AnyBnet,
    session: u64,
    peer: Peer,
    transport: &str,
    after_hello: impl FnOnce(),
    breaker: Breaker,
) {
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
    after_hello();
    if let Some(job) = hello.strip_prefix("Hello Start Job ").and_then(|r| r.split_whitespace().next()) {
        if peer == Peer::Director {
            tracing::warn!("session {session}: StorageDaemon/Control accepts only a Director: {}", hello.trim_end());
            return;
        }
        tracing::info!("session {session}: File daemon of job {job} connects over {transport}");
        handle_fd(&d, bnet, job, breaker, session);
        return;
    }
    if peer == Peer::Fd {
        tracing::warn!("session {session}: StorageDaemon/Data accepts only a File daemon: {}", hello.trim_end());
        return;
    }
    tracing::info!("session {session}: Director connects over {transport}");
    crate::director::handle(d, bnet, &hello, session);
}

/// `HandleFiledConnection`: the FD of a job calls; authenticate it with the
/// job's key and hand the connection to the job.
fn handle_fd(d: &Daemon, mut bnet: AnyBnet, job_name: &str, breaker: Breaker, session: u64) {
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
            job.connect_fd(FdConnection { channel: Box::new(bnet), breaker });
        }
        _ => {
            tracing::error!("session {session}: Authorization problem: Two way security handshake failed with File daemon");
            job.set_status('E');
            job.fd_arrived.notify_all();
        }
    }
}

/// `PassiveCmd`: connects to the FD of `job`. With `grpc` (odir sent the
/// FD's gRPC target) first over `FileDaemon/Data`; when that fails, and
/// without it, over BNET ([`connect_passive_fd_bnet`]). Returns the
/// connection and its transport name.
pub fn connect_passive_fd(
    d: &Daemon,
    job: &Arc<Job>,
    addr: &str,
    port: u16,
    ssl: i32,
    grpc: Option<&onesimus_bnet_grpc::Target>,
) -> Result<(FdConnection, &'static str), String> {
    if let Some(target) = grpc {
        match connect_passive_fd_grpc(d, job, target) {
            Ok(conn) => {
                tracing::info!("{}: connected to File daemon {}:{} over gRPC TLS (passive client)", job.cmd.job, target.host, target.port);
                return Ok((conn, onesimus_bnet_grpc::TRANSPORT));
            }
            Err(e) => tracing::warn!(
                "{}: File daemon {}:{}: {e}; falling back to BNET on port {port}",
                job.cmd.job,
                target.host,
                target.port
            ),
        }
    }
    let conn = connect_passive_fd_bnet(d, job, addr, port, ssl)?;
    tracing::info!("{}: connected to File daemon {addr}:{port} over BNET (passive client)", job.cmd.job);
    Ok((conn, "BNET"))
}

/// The gRPC part of [`connect_passive_fd`]: `FileDaemon/Data`, Hello,
/// CRAM-MD5 with the job's key.
fn connect_passive_fd_grpc(d: &Daemon, job: &Arc<Job>, target: &onesimus_bnet_grpc::Target) -> Result<FdConnection, String> {
    let mut io = onesimus_bnet_grpc::client::connect(target, onesimus_bnet_grpc::Rpc::FileDaemonData, Some(LOGIN_TIMEOUT))
        .map_err(|e| e.to_string())?;
    io.cancel = Some(&crate::SHUTDOWN);
    let close = io.close_handle();
    let mut bnet = BnetStream::new(io);
    bnet.send_str(&format!("Hello Storage calling Start Job {}\n", job.cmd.job)).map_err(|e| e.to_string())?;
    let auth = CramMd5Auth::new_raw(&job.auth_key, &format!("R_STORAGE::{}", d.name()), 2);
    match auth.two_way_authenticate(&mut bnet, false) {
        Ok(r) if r.success => {
            let mut io = bnet.into_inner();
            io.set_read_timeout(None);
            let channel: AnyBnet = BnetStream::new(Box::new(io));
            Ok(FdConnection { channel: Box::new(channel), breaker: Breaker::Grpc(close) })
        }
        Ok(_) => Err("Failed to authenticate File daemon.".into()),
        Err(e) => Err(format!("CRAM: {e}")),
    }
}

/// The BNET part of [`connect_passive_fd`]: connects to the FD at
/// `addr:port` (retrying for up to 30 s), TLS-PSK with identity
/// `R_JOB\x1e<Job>` when `ssl` is `kBnetTlsAuto`, `Hello Storage calling
/// Start Job <Job>`, CRAM-MD5 with the job's key.
fn connect_passive_fd_bnet(d: &Daemon, job: &Arc<Job>, addr: &str, port: u16, ssl: i32) -> Result<FdConnection, String> {
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
    let breaker = Breaker::socket(tcp.try_clone().ok());
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
        Ok(r) if r.success => Ok(FdConnection { channel: Box::new(bnet), breaker }),
        _ => Err("Failed to authenticate File daemon.".into()),
    }
}
