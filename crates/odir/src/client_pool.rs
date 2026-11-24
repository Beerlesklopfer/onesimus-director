// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Client initiated connections (`Connection From Client To Director`).
//!
//! A File Daemon behind NAT connects to the Director itself
//! (`Hello Client <name> FdProtocolVersion=<n> calling`), authenticates and
//! waits. The next job of that client, and also `status client`, `estimate`
//! and the other console commands that talk to it, take the waiting
//! connection instead of connecting: the Director sends its first command
//! at once, without `Hello Director` (the FD has authenticated already).
//! The FD connects again as soon as a connection was used.
//!
//! Job workers run in forked processes: the Director moves the connection
//! into a hand-off slot before the fork ([`prepare_handoff`]), the worker
//! takes it from there ([`take_handoff`]) and keeps its descriptor open, the
//! Director drops its copy after the fork ([`drop_handoff`]).
//!
//! Measured against bareos-dir 25.1.3 and bareos-fd 25.1.3 with a recording
//! proxy (`onesimus-rig/tests/client_initiated_capture.rs`,
//! docs/plans/CLIENT-INITIIERT.md).
//!
//! Bareos-Gegenstück: `dird/socket_server.cc` (HandleConnectionRequest),
//! `dird/fd_cmds.cc` (HandleFiledConnection, UseWaitingClient),
//! `lib/connection_pool.cc`, `dird/run_on_incoming_connect_interval.cc`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bareos_bnet::cram_md5::CramMd5Auth;
use bareos_bnet::{AnyStream, BnetMessage, BnetSignal, BnetStream};
use tracing::{debug, info, warn};

use crate::catalog_worker::CatalogHandle;
use crate::director_config::SharedConfig;

/// One authenticated connection of a client, waiting for a job.
struct Waiting {
    /// The client's name (Client resource).
    name: String,
    /// The FD protocol version of its Hello.
    protocol_version: i32,
    /// When it connected (seconds since 1970).
    connect_time: i64,
    /// The connection, authenticated, idle.
    stream: BnetStream<AnyStream>,
}

/// The connection pool (connection_pool.h: several connections per name, newest last).
static POOL: Mutex<Vec<Waiting>> = Mutex::new(Vec::new());

/// A connection on its way into a forked job worker.
struct Handoff {
    /// The job that takes it.
    job_id: u32,
    /// The connection.
    stream: BnetStream<AnyStream>,
    /// The FD protocol version.
    protocol_version: i32,
}

/// The hand-off slot (one at a time: the JobManager forks one worker after the other).
static HANDOFF: Mutex<Option<Handoff>> = Mutex::new(None);

/// Whether the idle connection is still open (connection_pool.cc check_connection): no data and no
/// end of file on the socket; a heartbeat is read and the connection kept.
fn alive(stream: &mut BnetStream<AnyStream>) -> bool {
    let Some(tcp) = stream.inner().tcp().and_then(|t| t.try_clone().ok()) else { return true };
    let mut byte = [0u8; 1];
    if tcp.set_nonblocking(true).is_err() {
        return false;
    }
    let peeked = tcp.peek(&mut byte);
    let _ = tcp.set_nonblocking(false);
    match peeked {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => true,
        Ok(0) | Err(_) => false,
        Ok(_) => {
            let _ = tcp.set_read_timeout(Some(Duration::from_millis(200)));
            let message = stream.recv();
            let _ = tcp.set_read_timeout(None);
            matches!(message, Ok(BnetMessage::Signal(BnetSignal::Heartbeat | BnetSignal::HeartbeatResponse)))
        }
    }
}

/// Drops the connections that are no longer open (remove_inactive).
fn remove_inactive(pool: &mut Vec<Waiting>) {
    pool.retain_mut(|w| {
        let keep = alive(&mut w.stream);
        if !keep {
            debug!("client initiated connection of {} closed", w.name);
        }
        keep
    });
}

/// Adds an authenticated connection (add_authenticated_connection).
fn add(name: &str, protocol_version: i32, stream: BnetStream<AnyStream>) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    remove_inactive(&mut pool);
    let connect_time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    pool.push(Waiting { name: name.to_string(), protocol_version, connect_time, stream });
}

/// The newest open connection of client `name`, taken out of the pool (UseWaitingClient,
/// take_by_name): the stream and the FD protocol version.
pub fn take(name: &str) -> Option<(BnetStream<AnyStream>, i32)> {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    remove_inactive(&mut pool);
    let at = pool.iter().rposition(|w| w.name == name)?;
    let w = pool.remove(at);
    debug!("using the client initiated connection of {name} (FD protocol {})", w.protocol_version);
    Some((w.stream, w.protocol_version))
}

/// The waiting connections for `status dir` (ListConnectedClients): name, protocol version and
/// connect time, oldest first.
pub fn list() -> Vec<(String, i32, i64)> {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    remove_inactive(&mut pool);
    pool.iter().map(|w| (w.name.clone(), w.protocol_version, w.connect_time)).collect()
}

/// Before the fork of job `job_id`: its client's waiting connection into the hand-off slot; the
/// socket's descriptor, which the worker has to keep open.
#[cfg(unix)]
pub fn prepare_handoff(job_id: u32, client: &str) -> Option<i32> {
    use std::os::fd::AsRawFd;
    let (stream, protocol_version) = take(client)?;
    let fd = stream.inner().tcp().map(|t| t.as_raw_fd());
    *HANDOFF.lock().unwrap_or_else(|e| e.into_inner()) = Some(Handoff { job_id, stream, protocol_version });
    fd
}

/// The descriptor of the connection in the hand-off slot for job `job_id` (kept by the worker).
#[cfg(unix)]
pub fn handoff_fd(job_id: u32) -> Option<i32> {
    use std::os::fd::AsRawFd;
    let slot = HANDOFF.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().filter(|h| h.job_id == job_id).and_then(|h| h.stream.inner().tcp().map(|t| t.as_raw_fd()))
}

/// In the worker of job `job_id`: the connection handed over (stream and FD protocol version).
pub fn take_handoff(job_id: u32) -> Option<(BnetStream<AnyStream>, i32)> {
    let mut slot = HANDOFF.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|h| h.job_id == job_id) {
        return slot.take().map(|h| (h.stream, h.protocol_version));
    }
    None
}

/// In the Director after the fork of job `job_id`: its copy of the handed-over connection is
/// dropped without a word on the wire (the worker owns the connection now).
pub fn drop_handoff(job_id: u32) {
    let mut slot = HANDOFF.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|h| h.job_id == job_id) {
        // dropping closes only this process's descriptor; TLS sends nothing on drop
        *slot = None;
    }
}

/// What the Director knows about a client for its connection: password, TLS policy for the
/// CRAM-MD5 challenge, own name, `Connection From Client To Director`.
struct ClientSettings {
    password: String,
    tls_policy: i32,
    director: String,
    allowed: bool,
}

/// The settings of client `name` (None: no such Client resource).
fn client_settings(config: &SharedConfig, name: &str) -> Option<ClientSettings> {
    let cfg = config.read().ok()?;
    let client = cfg.find_client(name)?;
    let allowed = crate::commands::defaults::raw(&cfg, "client", name)
        .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == "connectionfromclienttodirector"))
        .and_then(|d| d.first())
        .and_then(|v| bareos_config::values::parse_bool(v).ok())
        .is_some_and(|(_, v)| v);
    Some(ClientSettings {
        password: client.password.clone(),
        tls_policy: crate::fd_protocol::tls_policy(client.tls_enable, client.tls_require),
        director: cfg.director.name.clone(),
        allowed,
    })
}

/// HandleFiledConnection(): a File Daemon's `Hello Client <name> … calling` on the Director port.
/// The Director challenges first (`auth cram-md5 <…@R_DIRECTOR::<director>>`), then answers the
/// FD's challenge, both with the Client resource's password; then `1000 OK: <director> Version: …`.
/// The connection waits in the pool; jobs with `Run On Incoming Connect Interval` may start.
#[allow(clippy::too_many_arguments)]
pub fn accept_client(mut bnet: BnetStream<AnyStream>, session_id: u64, name: &str, protocol_version: i32,
                     tls_identity: Option<&str>, config: &SharedConfig, catalog: Option<&CatalogHandle>,
                     session_mgr: &Arc<crate::session_manager::SessionManager>) {
    let Some(settings) = client_settings(config, name) else {
        warn!("session {session_id}: Client \"{name}\" not found, connection refused");
        return;
    };
    if tls_identity.is_some_and(|id| id != format!("R_CLIENT\x1e{name}")) {
        warn!("session {session_id}: client '{name}' does not match the TLS-PSK identity, rejected");
        return;
    }
    if !settings.allowed {
        warn!("session {session_id}: Connection from client \"{name}\" to the Director is not allowed");
        return;
    }
    let auth = CramMd5Auth::new(&settings.password, &format!("R_DIRECTOR::{}", settings.director), settings.tls_policy);
    let ok = auth.two_way_authenticate(&mut bnet, true).is_ok_and(|r| r.success);
    if !ok {
        warn!("session {session_id}: Unable to authenticate client \"{name}\"");
        let _ = bnet.send_str("1999 You are not authorized.\n");
        std::thread::sleep(Duration::from_secs(5));
        return;
    }
    let hello = format!("1000 OK: {} Version: {} ({})\n", settings.director, daemon_crate::version::VERSION,
        daemon_crate::version::BUILD_DATE);
    if bnet.send_str(&hello).is_err() {
        return;
    }
    info!("session {session_id}: client initiated connection of {name} (FD protocol {protocol_version}) waits for a job");
    add(name, protocol_version, bnet);
    session_mgr.add_fd_connection(name, protocol_version);
    run_on_incoming_connect_interval(config, catalog, name);
}

/// The jobs RunOnIncomingConnectInterval::Run() would queue for client `client` at `now` (seconds
/// since 1970): every job of the client with `Run On Incoming Connect Interval` ≠ 0 whose last start
/// (`last_start`: the newest StartTime of the job at that client with JobStatus T, W, C or R; None:
/// none; Err: catalog error, nothing starts) is more than the interval ago, or that never ran.
fn due_jobs(jobs: &[(String, u64)], now: i64, mut last_start: impl FnMut(&str) -> Result<Option<i64>, String>) -> Vec<String> {
    jobs.iter()
        .filter(|(_, interval)| *interval != 0)
        .filter_map(|(job, interval)| match last_start(job) {
            Err(_) => None,
            Ok(None) => Some(job.clone()),
            Ok(Some(start)) => (now - start > *interval as i64).then(|| job.clone()),
        })
        .collect()
}

/// RunOnIncomingConnectInterval(client).Run(): queues the due jobs of `client` (job trigger
/// "Client"). The original queues them for the scheduler's next look; Onesimus starts them at once.
fn run_on_incoming_connect_interval(config: &SharedConfig, catalog: Option<&CatalogHandle>, client: &str) {
    let jobs: Vec<(String, u64)> = {
        let Ok(cfg) = config.read() else { return };
        cfg.jobs.iter()
            .filter(|j| j.client == client)
            .filter_map(|j| {
                let interval = crate::commands::defaults::job_directives(&cfg, &j.name).iter().rev()
                    .find(|d| d.norm_key() == "runonincomingconnectinterval")
                    .and_then(|d| d.first().map(str::to_string))
                    .and_then(|v| bareos_config::values::parse_time(&v).ok())
                    .map(|(_, d)| d.as_secs())?;
                Some((j.name.clone(), interval))
            })
            .collect()
    };
    if jobs.is_empty() {
        return;
    }
    let Some(catalog) = catalog else { return };
    let esc = |s: &str| s.replace('\'', "''");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let due = due_jobs(&jobs, now, |job| {
        // FindLastJobStartTimeForJobAndClient(), read back with StrToUtime (local time)
        let sql = format!(
            "SELECT CASE WHEN StartTime IS NULL THEN NOW()::timestamp ELSE StartTime::timestamp END FROM Job \
             WHERE Job.Name='{}' AND Job.JobStatus IN ('T','W','C','R') AND Job.ClientId=(SELECT ClientId \
             FROM Client WHERE Client.Name='{}') ORDER BY StartTime DESC LIMIT 1", esc(job), esc(client));
        let table = catalog.query_rows(&sql)?;
        Ok(table.rows.first().and_then(|r| r.first().cloned().flatten()).map(|t| crate::btime::str_to_utime(&t)))
    });
    for job in due {
        debug!("Run On Incoming Connect Interval: job {job} for client {client}");
        crate::scheduled_run::execute_on_incoming_connect(&job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_run_when_the_interval_has_passed_or_they_never_ran() {
        let jobs = vec![("a".to_string(), 3600), ("b".to_string(), 3600), ("c".to_string(), 0), ("d".to_string(), 3600), ("e".to_string(), 3600)];
        let now = 100_000;
        let due = due_jobs(&jobs, now, |job| match job {
            "a" => Ok(Some(now - 3601)), // more than the interval ago
            "b" => Ok(Some(now - 3600)), // exactly the interval: not strictly more
            "d" => Ok(None),             // never ran
            "e" => Err("db".into()),     // catalog error
            _ => Ok(None),
        });
        assert_eq!(due, ["a", "d"]);
    }
}
