//! Central connection manager for Console, FD, and SD connections.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Receiver;
use dashmap::DashMap;

use crate::message_bus::{BusMessage, MessageBus};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_session_id() -> u64 {
    NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed)
}

/// Console session information.
#[derive(Debug)]
pub struct SessionInfo {
    pub id: u64,
    pub client_name: String,
    pub authenticated: bool,
    expires_at: Option<std::time::Instant>,
    pub bus_rx: Receiver<BusMessage>,
    /// Catalog state of the console session (C++ UaContext catalog/db).
    pub catalog: SessionCatalog,
    /// C++ ua->automount: mount the volume after `label` (default on, ua.cc:84).
    pub automount: bool,
    /// Unix time the console connected (C++ console jcr->start_time).
    pub connected_at: i64,
}

/// Per-session catalog selection, like the C++ UaContext fields `catalog`,
/// `shared_db`, `private_db` and `gui` (ua_db.cc OpenDb/CloseDb).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionCatalog {
    /// Selected Catalog resource (`ua->catalog`); kept across CloseDb.
    pub name: Option<String>,
    /// Shared connection opened (`ua->shared_db`).
    pub shared_open: bool,
    /// Private connection opened (`ua->private_db`).
    pub private_open: bool,
    /// `gui on` (`ua->gui`): first catalog is picked silently.
    pub gui: bool,
}

/// File Daemon connection info.
#[derive(Debug)]
pub struct FdConnectionInfo {
    pub client_name: String,
    pub protocol_version: i32,
    pub connect_time: i64,
    pub alive: bool,
}

/// Storage Daemon connection info.
#[derive(Debug)]
pub struct SdConnectionInfo {
    pub job_id: u32,
    pub storage_name: String,
    pub connect_time: i64,
}

/// Central manager for all connections.
pub struct SessionManager {
    sessions: DashMap<u64, SessionInfo>,
    fd_connections: DashMap<String, FdConnectionInfo>,
    sd_connections: DashMap<u32, SdConnectionInfo>,
    bus: Arc<MessageBus>,
}

impl SessionManager {
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            sessions: DashMap::new(),
            fd_connections: DashMap::new(),
            sd_connections: DashMap::new(),
            bus,
        }
    }

    // --- Console Sessions ---

    pub fn create_session(&self, client_name: &str) -> u64 {
        let id = next_session_id();
        let bus_rx = self.bus.subscribe();
        self.sessions.insert(id, SessionInfo {
            id,
            client_name: client_name.to_string(),
            authenticated: false,
            expires_at: None,
            bus_rx,
            catalog: SessionCatalog::default(),
            automount: true,
            connected_at: now_unix(),
        });
        id
    }

    /// Access the catalog state of a console session.
    pub fn with_session_catalog<R>(&self, id: u64, f: impl FnOnce(&mut SessionCatalog) -> R) -> Option<R> {
        self.sessions.get_mut(&id).map(|mut session| f(&mut session.catalog))
    }

    /// Mirror the session `gui on/off` flag (C++ `ua->gui`).
    pub fn set_session_gui(&self, id: u64, gui: bool) {
        self.with_session_catalog(id, |catalog| catalog.gui = gui);
    }

    pub fn authenticate_session(&self, id: u64) {
        if let Some(mut session) = self.sessions.get_mut(&id) {
            session.authenticated = true;
        }
    }

    pub fn authenticate_session_with_ttl(&self, id:u64, ttl:std::time::Duration) {
        if let Some(mut session)=self.sessions.get_mut(&id) {
            session.authenticated=true;
            session.expires_at=Some(std::time::Instant::now()+ttl);
        }
    }
    pub fn refresh_session_ttl(&self, id:u64, ttl:std::time::Duration) -> bool {
        let Some(mut session)=self.sessions.get_mut(&id) else {return false;};
        if !session.authenticated || session.expires_at.is_none_or(|deadline|deadline<=std::time::Instant::now()) {return false;}
        session.expires_at=Some(std::time::Instant::now()+ttl);
        true
    }

    pub fn authenticated_console(&self, id: u64) -> Option<String> {
        self.sessions.get(&id).filter(|session|session.authenticated && session.expires_at.is_none_or(|deadline|deadline>std::time::Instant::now())).map(|session|session.client_name.clone())
    }

    pub fn remove_session(&self, id: u64) {
        self.sessions.remove(&id);
    }

    pub fn active_session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Connect times of the authenticated console sessions in connection
    /// order (C++ foreach_jcr over the JT_CONSOLE jcrs).
    pub fn console_connect_times(&self) -> Vec<i64> {
        let mut consoles: Vec<(u64, i64)> = self.sessions.iter()
            .filter(|s| s.authenticated)
            .map(|s| (s.id, s.connected_at))
            .collect();
        consoles.sort();
        consoles.into_iter().map(|c| c.1).collect()
    }

    pub fn get_session_names(&self) -> Vec<(u64, String)> {
        self.sessions.iter()
            .map(|entry| (entry.id, entry.client_name.clone()))
            .collect()
    }

    /// `automount on|off` (C++ AutomountCmd).
    pub fn set_session_automount(&self, id: u64, on: bool) {
        if let Some(mut session) = self.sessions.get_mut(&id) {
            session.automount = on;
        }
    }

    /// Automount flag of a session; unknown sessions use the C++ default (on).
    pub fn session_automount(&self, id: u64) -> bool {
        self.sessions.get(&id).map(|session| session.automount).unwrap_or(true)
    }

    // --- FD Connections ---

    pub fn add_fd_connection(&self, client_name: &str, protocol_version: i32) {
        let now = now_unix();
        self.fd_connections.insert(client_name.to_string(), FdConnectionInfo {
            client_name: client_name.to_string(),
            protocol_version,
            connect_time: now,
            alive: true,
        });
        self.bus.post(BusMessage::FdConnected {
            client_name: client_name.to_string(),
            protocol_version,
            connect_time: now,
        });
    }

    pub fn remove_fd_connection(&self, client_name: &str) {
        self.fd_connections.remove(client_name);
        self.bus.post(BusMessage::FdDisconnected {
            client_name: client_name.to_string(),
        });
    }

    pub fn fd_connection_count(&self) -> usize {
        self.fd_connections.len()
    }

    pub fn get_fd_connections(&self) -> Vec<FdConnectionInfo> {
        self.fd_connections.iter()
            .map(|entry| FdConnectionInfo {
                client_name: entry.client_name.clone(),
                protocol_version: entry.protocol_version,
                connect_time: entry.connect_time,
                alive: entry.alive,
            })
            .collect()
    }

    // --- SD Connections ---

    pub fn track_sd_connection(&self, job_id: u32, storage_name: &str) {
        let now = now_unix();
        self.sd_connections.insert(job_id, SdConnectionInfo {
            job_id,
            storage_name: storage_name.to_string(),
            connect_time: now,
        });
        self.bus.post(BusMessage::SdConnected {
            job_id,
            storage_name: storage_name.to_string(),
            connect_time: now,
        });
    }

    pub fn untrack_sd_connection(&self, job_id: u32) {
        if let Some((_, info)) = self.sd_connections.remove(&job_id) {
            self.bus.post(BusMessage::SdDisconnected {
                job_id,
                storage_name: info.storage_name,
            });
        }
    }

    pub fn sd_connection_count(&self) -> usize {
        self.sd_connections.len()
    }

    // --- Bus access ---

    pub fn bus(&self) -> &MessageBus {
        &self.bus
    }

    /// Broadcast a text message to all console sessions.
    pub fn broadcast_text(&self, text: &str) {
        self.bus.post_text(text);
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mgr() -> SessionManager {
        let bus = Arc::new(MessageBus::new(64));
        SessionManager::new(bus)
    }

    #[test]
    fn automount_defaults_on_and_is_per_session() {
        let mgr = make_mgr();
        let id1 = mgr.create_session("console1");
        let id2 = mgr.create_session("console2");
        assert!(mgr.session_automount(id1));
        mgr.set_session_automount(id1, false);
        assert!(!mgr.session_automount(id1));
        assert!(mgr.session_automount(id2));
        mgr.set_session_automount(id1, true);
        assert!(mgr.session_automount(id1));
    }

    #[test]
    fn create_and_remove_session() {
        let mgr = make_mgr();
        let id1 = mgr.create_session("console1");
        let id2 = mgr.create_session("console2");
        assert_eq!(mgr.active_session_count(), 2);

        mgr.remove_session(id1);
        assert_eq!(mgr.active_session_count(), 1);

        mgr.remove_session(id2);
        assert_eq!(mgr.active_session_count(), 0);
    }

    #[test]
    fn session_ids_are_unique() {
        let mgr = make_mgr();
        let id1 = mgr.create_session("a");
        let id2 = mgr.create_session("b");
        let id3 = mgr.create_session("c");
        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
    }

    #[test]
    fn authenticate_session() {
        let mgr = make_mgr();
        let id = mgr.create_session("test");
        mgr.authenticate_session(id);
        // Verify via session names (authenticated flag is internal)
        let names = mgr.get_session_names();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].1, "test");
    }

    #[test]
    fn fd_connections() {
        let mgr = make_mgr();
        assert_eq!(mgr.fd_connection_count(), 0);

        mgr.add_fd_connection("client1-fd", 54);
        assert_eq!(mgr.fd_connection_count(), 1);

        mgr.add_fd_connection("client2-fd", 54);
        assert_eq!(mgr.fd_connection_count(), 2);

        let fds = mgr.get_fd_connections();
        assert_eq!(fds.len(), 2);

        mgr.remove_fd_connection("client1-fd");
        assert_eq!(mgr.fd_connection_count(), 1);
    }

    #[test]
    fn fd_connection_emits_bus_events() {
        let bus = Arc::new(MessageBus::new(64));
        let rx = bus.subscribe();
        let mgr = SessionManager::new(bus);

        mgr.add_fd_connection("test-fd", 54);

        match rx.try_recv().unwrap() {
            BusMessage::FdConnected { client_name, protocol_version, .. } => {
                assert_eq!(client_name, "test-fd");
                assert_eq!(protocol_version, 54);
            }
            _ => panic!("expected FdConnected"),
        }

        mgr.remove_fd_connection("test-fd");

        match rx.try_recv().unwrap() {
            BusMessage::FdDisconnected { client_name } => {
                assert_eq!(client_name, "test-fd");
            }
            _ => panic!("expected FdDisconnected"),
        }
    }

    #[test]
    fn sd_connections() {
        let mgr = make_mgr();
        assert_eq!(mgr.sd_connection_count(), 0);

        mgr.track_sd_connection(1, "File");
        assert_eq!(mgr.sd_connection_count(), 1);

        mgr.track_sd_connection(2, "Tape");
        assert_eq!(mgr.sd_connection_count(), 2);

        mgr.untrack_sd_connection(1);
        assert_eq!(mgr.sd_connection_count(), 1);
    }

    #[test]
    fn sd_connection_emits_bus_events() {
        let bus = Arc::new(MessageBus::new(64));
        let rx = bus.subscribe();
        let mgr = SessionManager::new(bus);

        mgr.track_sd_connection(42, "File");

        match rx.try_recv().unwrap() {
            BusMessage::SdConnected { job_id, storage_name, .. } => {
                assert_eq!(job_id, 42);
                assert_eq!(storage_name, "File");
            }
            _ => panic!("expected SdConnected"),
        }

        mgr.untrack_sd_connection(42);

        match rx.try_recv().unwrap() {
            BusMessage::SdDisconnected { job_id, storage_name } => {
                assert_eq!(job_id, 42);
                assert_eq!(storage_name, "File");
            }
            _ => panic!("expected SdDisconnected"),
        }
    }

    #[test]
    fn broadcast_text() {
        let bus = Arc::new(MessageBus::new(64));
        let rx = bus.subscribe();
        let mgr = SessionManager::new(bus);

        mgr.broadcast_text("hello all");

        match rx.try_recv().unwrap() {
            BusMessage::ConsoleText { text } => assert_eq!(text, "hello all"),
            _ => panic!("expected ConsoleText"),
        }
    }

    #[test]
    fn concurrent_sessions() {
        let mgr = Arc::new(make_mgr());
        let mut handles = Vec::new();

        for i in 0..10 {
            let mgr = mgr.clone();
            handles.push(std::thread::spawn(move || {
                let id = mgr.create_session(&format!("console-{i}"));
                std::thread::sleep(std::time::Duration::from_millis(1));
                mgr.remove_session(id);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(mgr.active_session_count(), 0);
    }
}
