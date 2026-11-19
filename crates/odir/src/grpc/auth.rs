//! gRPC authentication: challenge/response login + session tokens.
//!
//! Login (see director_service.proto):
//!   key   = lowercase hex MD5 of the console password (as for TLS-PSK)
//!   proof = lowercase hex HMAC-SHA256(key, nonce)
//! The password never crosses the wire; the transport is TLS anyway.
//! After a successful login every RPC carries the session token in the
//! `x-onesimus-session` metadata header.
//!
//! Failed logins are throttled per source address and per console name
//! ([`LoginThrottle`], finding K-19): after a number of free failures each
//! further failure refuses logins of that key for an exponentially growing,
//! capped time. Blocks are always time-limited; a successful login resets
//! both keys. Each failed proof is reported through the Director's
//! Messages resource like a failed console authentication.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use tonic::Status;
use tonic::metadata::MetadataMap;
use tracing::debug;

use crate::director_config::SharedConfig;

/// Metadata header that carries the session token of a logged-in client.
pub const SESSION_HEADER: &str = "x-onesimus-session";
/// How long a challenge nonce stays valid.
const NONCE_TTL: Duration = Duration::from_secs(60);
/// Idle time after which a session token expires.
const SESSION_IDLE_TTL: Duration = Duration::from_secs(8 * 3600);
/// Console name of the default console (Director password).
const DEFAULT_CONSOLE: &str = "*UserAgent*";

/// A logged-in gRPC session behind a token.
struct Session {
    console_name: String,
    session_id: u64,
    last_used: Instant,
}

/// Login state of the gRPC services: nonces, session tokens and the login
/// throttle.
pub struct AuthState {
    config: SharedConfig,
    session_mgr: std::sync::Arc<crate::session_manager::SessionManager>,
    /// nonce -> (console name, issued at); single use
    nonces: Mutex<HashMap<String, (String, Instant)>>,
    /// token -> session
    sessions: Mutex<HashMap<String, Session>>,
    /// Throttle of failed logins per source address and console name.
    throttle: LoginThrottle,
}

/// Why a login was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginError {
    /// Challenge or proof wrong; the text goes back to the client.
    Failed(String),
    /// Too many failed logins for the source address or the console; the
    /// login is refused without looking at the proof for the remaining time.
    Throttled(Duration),
}

impl std::fmt::Display for LoginError {
    /// The message for the client (LoginResponse.error_message or Status).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed(e) => f.write_str(e),
            Self::Throttled(wait) => write!(f, "too many failed logins, retry in {} s", wait.as_secs().max(1)),
        }
    }
}

impl AuthState {
    /// Login state with the system clock for the throttle.
    pub fn new(config: SharedConfig, session_mgr: std::sync::Arc<crate::session_manager::SessionManager>) -> Self {
        Self::with_clock(config, session_mgr, Arc::new(SystemClock))
    }

    /// Login state whose throttle reads the time from `clock` (tests).
    pub(crate) fn with_clock(config: SharedConfig, session_mgr: std::sync::Arc<crate::session_manager::SessionManager>,
                             clock: Arc<dyn Clock>) -> Self {
        Self {
            config,
            session_mgr,
            nonces: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            throttle: LoginThrottle::new(clock),
        }
    }

    /// Issue a single-use nonce for `console_name`.
    pub fn challenge(&self, console_name: &str) -> String {
        let nonce = random_hex(32);
        let mut nonces = self.nonces.lock().unwrap();
        nonces.retain(|_, (_, t)| t.elapsed() < NONCE_TTL);
        nonces.insert(nonce.clone(), (normalize_console(console_name), Instant::now()));
        nonce
    }

    /// Verify a login proof without a known source address (only the
    /// per-console throttle applies). Returns the new session token. Used
    /// by tests; the Login RPC calls [`Self::login_from`].
    #[cfg(test)]
    pub fn login(&self, console_name: &str, nonce: &str, proof: &str) -> Result<String, String> {
        self.login_from(console_name, nonce, proof, None).map_err(|e| e.to_string())
    }

    /// Verify a login proof from `peer`. Returns the new session token.
    ///
    /// A console or address blocked by the throttle is refused with
    /// [`LoginError::Throttled`] before the challenge is looked at (the
    /// nonce is still consumed). A wrong challenge is no password guess and
    /// is not counted; an unknown console or a wrong proof counts for both
    /// the address and the console and is reported through the Director's
    /// Messages resource. Success resets both counters.
    pub fn login_from(&self, console_name: &str, nonce: &str, proof: &str, peer: Option<SocketAddr>)
                      -> Result<String, LoginError> {
        let console = normalize_console(console_name);
        let address = peer.map(|p| p.ip().to_canonical());

        let issued = self.nonces.lock().unwrap().remove(nonce);
        if let Err(wait) = self.throttle.check(address, &console) {
            debug!("gRPC login of console '{console}' from {peer:?} refused for {} s more", wait.as_secs());
            return Err(LoginError::Throttled(wait));
        }
        match issued {
            Some((c, t)) if c == console && t.elapsed() < NONCE_TTL => {}
            _ => return Err(LoginError::Failed("invalid or expired challenge".into())),
        }

        let verified = self.console_key(&console).is_some_and(|key|
            constant_time_eq(compute_proof(&key, nonce).as_bytes(), proof.to_ascii_lowercase().as_bytes()));
        if !verified {
            self.report_failure(&console, peer, self.throttle.failure(address, &console));
            return Err(LoginError::Failed("authentication failed".into()));
        }
        self.throttle.success(address, &console);

        let token = random_hex(32);
        let mut sessions = self.sessions.lock().unwrap();
        sessions.retain(|_, s| {
            if s.last_used.elapsed() < SESSION_IDLE_TTL { true }
            else { self.session_mgr.remove_session(s.session_id); false }
        });
        let session_id=self.session_mgr.create_session(&console);
        self.session_mgr.authenticate_session_with_ttl(session_id,SESSION_IDLE_TTL);
        sessions.insert(token.clone(), Session { console_name: console, session_id, last_used: Instant::now() });
        Ok(token)
    }

    /// A failed proof through the Director's Messages resource, like the
    /// failed console authentication of Bareos (Emsg M_ERROR "Unable to
    /// authenticate console ..."), and a warning when it starts a block.
    fn report_failure(&self, console: &str, peer: Option<SocketAddr>, blocked: Option<Duration>) {
        let at = peer.map_or_else(|| "unknown address".to_string(), |p| p.to_string());
        let Ok(cfg) = self.config.read() else { return };
        crate::audit::daemon_message(&cfg, crate::message_bus::MsgType::Error,
            &format!("Unable to authenticate console \"{console}\" at {at} (gRPC).\n"));
        if let Some(wait) = blocked {
            crate::audit::daemon_message(&cfg, crate::message_bus::MsgType::Warning,
                &format!("Too many failed gRPC logins of console \"{console}\" or from {at}: logins refused for {} s.\n",
                         wait.as_secs().max(1)));
        }
    }

    /// Check the session header. Returns the console name of the session.
    pub fn check(&self, metadata: &MetadataMap) -> Result<String, Status> {
        self.identity(metadata).map(|(name,_)|name)
    }
    /// Check the session header. Returns the session manager id of the session.
    pub fn check_session(&self, metadata: &MetadataMap) -> Result<u64, Status> {
        self.identity(metadata).map(|(_,id)|id)
    }
    /// Console name and session id of the session header.
    pub fn identity(&self, metadata: &MetadataMap) -> Result<(String,u64), Status> {
        let token = metadata
            .get(SESSION_HEADER)
            .ok_or_else(|| Status::unauthenticated("not logged in"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("invalid session token"))?;

        let mut sessions = self.sessions.lock().unwrap();
        match sessions.get_mut(token) {
            Some(s) if s.last_used.elapsed() < SESSION_IDLE_TTL && self.session_mgr.refresh_session_ttl(s.session_id,SESSION_IDLE_TTL) => {
                s.last_used = Instant::now();
                Ok((s.console_name.clone(),s.session_id))
            }
            _ => {
                if let Some(session)=sessions.remove(token) { self.session_mgr.remove_session(session.session_id); }
                Err(Status::unauthenticated("invalid session token"))
            },
        }
    }

    /// End the session of a token (Quit/Exit RPCs).
    #[cfg_attr(not(feature = "grpc-development"), allow(dead_code))]
    pub fn logout(&self, token: &str) {
        if let Some(session) = self.sessions.lock().unwrap().remove(token) {
            self.session_mgr.remove_session(session.session_id);
        }
    }

    /// Let the session of `token` look idle beyond its TTL (tests).
    #[cfg(test)]
    pub(crate) fn expire_for_test(&self, token:&str) {
        self.sessions.lock().unwrap().get_mut(token).unwrap().last_used=Instant::now()-SESSION_IDLE_TTL-Duration::from_secs(1);
    }
    /// Whether the console of session `id` may access `item` of ACL `kind`
    /// (acl::allowed); an expired or unknown session may access nothing.
    pub fn allowed(&self, id:u64, kind:crate::acl::Kind, item:&str) -> bool {
        let Some(console)=self.session_mgr.authenticated_console(id) else {return false;};
        self.config.read().map(|cfg|crate::acl::allowed(&cfg,&console,kind,item)).unwrap_or(false)
    }

    /// Whether the push event made from `msg` may go to session `id`
    /// (push_events::visible_to with the current configuration, so a reload
    /// applies at once). An expired or unknown session gets no events.
    pub fn event_visible(&self, id: u64, msg: &crate::message_bus::BusMessage) -> bool {
        let Some(console) = self.session_mgr.authenticated_console(id) else { return false };
        self.config.read().map(|cfg| super::push_events::visible_to(&cfg, &console, msg)).unwrap_or(false)
    }

    /// Whether the ACL `kind` of `console` allows any resource (acl::any_allowed).
    pub fn any_allowed(&self, console: &str, kind: crate::acl::Kind) -> bool {
        self.config.read().map(|cfg| crate::acl::any_allowed(&cfg, console, kind)).unwrap_or(false)
    }

    /// MD5 key for a console: Director password for the default console,
    /// otherwise the password of the named Console resource.
    fn console_key(&self, console: &str) -> Option<String> {
        let cfg = self.config.read().ok()?;
        let password = if console == DEFAULT_CONSOLE {
            cfg.director.password.clone()
        } else {
            cfg.consoles.iter().find(|c| c.name == console)?.password.clone()
        };
        if password.is_empty() {
            return None;
        }
        Some(password_key(&password))
    }
}

/// Source of the current time for the login throttle; tests inject a clock
/// they advance by hand.
pub trait Clock: Send + Sync {
    /// The current monotonic time.
    fn now(&self) -> Instant;
}

/// The system's monotonic clock.
struct SystemClock;

impl Clock for SystemClock {
    /// `Instant::now()`.
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Failures a source address may have before its logins are refused.
const FREE_FAILURES_PER_ADDRESS: u32 = 5;
/// Failures a console name may have (from all addresses together) before
/// its logins are refused; higher than per address, since a block of the
/// console also refuses its rightful user.
const FREE_FAILURES_PER_CONSOLE: u32 = 10;
/// Refusal after the first failure beyond the free ones; doubles with each
/// further failure.
const BASE_DELAY: Duration = Duration::from_secs(1);
/// Longest refusal: no key is ever locked out for longer.
const MAX_DELAY: Duration = Duration::from_secs(300);
/// A key without a failure for this long is forgotten (its count restarts).
const FAILURE_MEMORY: Duration = Duration::from_secs(15 * 60);
/// Most keys tracked per kind; beyond it the oldest entry is dropped, so
/// random console names or addresses cannot grow the tables without bound.
const MAX_TRACKED: usize = 4096;

/// Failed logins of one key (source address or console name).
#[derive(Debug, Clone, Copy)]
struct Failures {
    /// Failures since the last success or since the key was forgotten.
    count: u32,
    /// Time of the last failure.
    last: Instant,
    /// Logins of the key are refused until then.
    blocked_until: Option<Instant>,
}

/// The tables of the throttle, behind one lock.
#[derive(Default)]
struct ThrottleTables {
    /// Failures per source address (IPv4-mapped IPv6 as IPv4).
    by_address: HashMap<IpAddr, Failures>,
    /// Failures per console name (normalized, `*UserAgent*` for the default).
    by_console: HashMap<String, Failures>,
}

/// Login throttle against password guessing on the gRPC port.
///
/// Bareos itself has no such throttle; the BNET console login of odir
/// (session.rs) only logs a failure. Policy: the first
/// [`FREE_FAILURES_PER_ADDRESS`] / [`FREE_FAILURES_PER_CONSOLE`] failures
/// are free; failure n beyond them refuses logins of that key for
/// `BASE_DELAY * 2^(n-1)`, at most [`MAX_DELAY`]. There is no permanent
/// lockout: every block ends on its own, and a key without failures for
/// [`FAILURE_MEMORY`] starts over. A successful login clears the address
/// and the console.
pub struct LoginThrottle {
    /// Time source (injectable for tests).
    clock: Arc<dyn Clock>,
    /// Failure counters.
    tables: Mutex<ThrottleTables>,
}

impl LoginThrottle {
    /// An empty throttle reading the time from `clock`.
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock, tables: Mutex::new(ThrottleTables::default()) }
    }

    /// `Err(remaining)` while the address or the console is blocked (the
    /// longer of both remaining times), otherwise `Ok`.
    pub fn check(&self, address: Option<IpAddr>, console: &str) -> Result<(), Duration> {
        let now = self.clock.now();
        let tables = self.tables.lock().unwrap_or_else(|e| e.into_inner());
        let remaining = |f: Option<&Failures>| f.and_then(|f| f.blocked_until)
            .and_then(|until| until.checked_duration_since(now)).filter(|d| !d.is_zero());
        let wait = remaining(address.and_then(|a| tables.by_address.get(&a)))
            .max(remaining(tables.by_console.get(console)));
        wait.map_or(Ok(()), Err)
    }

    /// Count a failed login for the address and the console. Returns the
    /// refusal it starts, if it starts one (the longer of both).
    pub fn failure(&self, address: Option<IpAddr>, console: &str) -> Option<Duration> {
        let now = self.clock.now();
        let mut tables = self.tables.lock().unwrap_or_else(|e| e.into_inner());
        let by_address = address.and_then(|a| record(&mut tables.by_address, a, now, FREE_FAILURES_PER_ADDRESS));
        let by_console = record(&mut tables.by_console, console.to_string(), now, FREE_FAILURES_PER_CONSOLE);
        by_address.max(by_console)
    }

    /// A successful login: forget the failures of the address and the console.
    pub fn success(&self, address: Option<IpAddr>, console: &str) {
        let mut tables = self.tables.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(a) = address {
            tables.by_address.remove(&a);
        }
        tables.by_console.remove(console);
    }
}

/// Count a failure of `key` at `now` in `table` and return the refusal it
/// starts (`free` failures are free). Forgets keys idle for
/// [`FAILURE_MEMORY`] and keeps the table at [`MAX_TRACKED`] entries.
fn record<K: std::hash::Hash + Eq + Clone>(table: &mut HashMap<K, Failures>, key: K, now: Instant, free: u32)
                                           -> Option<Duration> {
    let idle = |f: &Failures| now.saturating_duration_since(f.last) >= FAILURE_MEMORY
        && f.blocked_until.is_none_or(|until| until <= now);
    table.retain(|_, f| !idle(f));
    if !table.contains_key(&key) && table.len() >= MAX_TRACKED {
        if let Some(oldest) = table.iter().min_by_key(|(_, f)| f.last).map(|(k, _)| k.clone()) {
            table.remove(&oldest);
        }
    }
    let entry = table.entry(key).or_insert(Failures { count: 0, last: now, blocked_until: None });
    entry.count = entry.count.saturating_add(1);
    entry.last = now;
    let beyond = entry.count.checked_sub(free).filter(|n| *n > 0)?;
    let delay = BASE_DELAY.checked_mul(1u32.checked_shl(beyond - 1).unwrap_or(u32::MAX))
        .unwrap_or(MAX_DELAY).min(MAX_DELAY);
    entry.blocked_until = Some(now + delay);
    Some(delay)
}

/// MD5 hex of the password; config values already stored as "[md5]<hex>" are used as is.
pub fn password_key(password: &str) -> String {
    match password.strip_prefix("[md5]") {
        Some(hex) => hex.to_ascii_lowercase(),
        None => bareos_bnet::cram_md5::md5_hash_password(password),
    }
}

/// Login proof: lowercase hex HMAC-SHA256 of `nonce` keyed with `key`.
pub fn compute_proof(key: &str, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(nonce.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Empty console name means the default console.
fn normalize_console(name: &str) -> String {
    if name.is_empty() { DEFAULT_CONSOLE.to_string() } else { name.to_string() }
}

/// `bytes` random bytes as lowercase hex.
fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Byte comparison whose time does not depend on where the inputs differ.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};
    use tonic::metadata::MetadataValue;

    /// Login state with a Director password and console `admin`.
    fn state() -> AuthState {
        let cfg = crate::director_config::DirectorConfig::from_str(
            "Director {\n  Name = test-dir\n  Password = \"dirpass\"\n}\n\
             Console {\n  Name = admin\n  Password = \"adminpass\"\n}\n",
        ).unwrap();
        AuthState::new(Arc::new(RwLock::new(cfg)), Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16)))))
    }

    /// A named console logs in with its own password.
    #[test]
    fn login_named_console() {
        let st = state();
        let nonce = st.challenge("admin");
        let proof = compute_proof(&password_key("adminpass"), &nonce);
        let token = st.login("admin", &nonce, &proof).unwrap();

        let mut md = MetadataMap::new();
        md.insert(SESSION_HEADER, MetadataValue::try_from(token.as_str()).unwrap());
        assert_eq!(st.check(&md).unwrap(), "admin");
    }

    /// The default console uses the Director password.
    #[test]
    fn login_default_console_uses_director_password() {
        let st = state();
        let nonce = st.challenge("");
        let proof = compute_proof(&password_key("dirpass"), &nonce);
        assert!(st.login("*UserAgent*", &nonce, &proof).is_ok());
    }

    /// A wrong password is rejected.
    #[test]
    fn wrong_password_rejected() {
        let st = state();
        let nonce = st.challenge("admin");
        let proof = compute_proof(&password_key("wrong"), &nonce);
        assert!(st.login("admin", &nonce, &proof).is_err());
    }

    /// A nonce cannot be replayed.
    #[test]
    fn nonce_is_single_use() {
        let st = state();
        let nonce = st.challenge("admin");
        let proof = compute_proof(&password_key("adminpass"), &nonce);
        assert!(st.login("admin", &nonce, &proof).is_ok());
        assert!(st.login("admin", &nonce, &proof).is_err());
    }

    /// A nonce only works for the console it was issued to.
    #[test]
    fn nonce_bound_to_console() {
        let st = state();
        let nonce = st.challenge("admin");
        let proof = compute_proof(&password_key("dirpass"), &nonce);
        assert!(st.login("*UserAgent*", &nonce, &proof).is_err());
    }

    /// [md5] passwords are used as the key directly.
    #[test]
    fn md5_prefixed_password() {
        let hex = bareos_bnet::cram_md5::md5_hash_password("secret");
        assert_eq!(password_key(&format!("[md5]{hex}")), hex);
        assert_eq!(password_key("secret"), hex);
    }

    /// An expired token also ends the session-manager session.
    #[test]
    fn expired_token_revokes_server_allocated_session() {
        let st=state();let nonce=st.challenge("admin");
        let token=st.login("admin",&nonce,&compute_proof(&password_key("adminpass"),&nonce)).unwrap();
        let mut md=MetadataMap::new();md.insert(SESSION_HEADER,MetadataValue::try_from(token.as_str()).unwrap());
        let id=st.check_session(&md).unwrap();assert_eq!(st.session_mgr.authenticated_console(id).as_deref(),Some("admin"));
        st.expire_for_test(&token);assert!(st.check_session(&md).is_err());
        assert!(st.session_mgr.authenticated_console(id).is_none());
    }
    /// A clock the test advances by hand.
    struct ManualClock(Mutex<Instant>);

    impl ManualClock {
        /// A clock standing at the current time.
        fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(Instant::now())))
        }
        /// Move the clock forward by `d`.
        fn advance(&self, d: Duration) {
            *self.0.lock().unwrap() += d;
        }
    }

    impl Clock for ManualClock {
        /// The time the test has set.
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    /// Source address of the throttle tests.
    fn ip(last: u8) -> Option<IpAddr> {
        Some(IpAddr::from([192, 0, 2, last]))
    }

    /// Free failures, then 1, 2, 4 ... s up to the cap; blocks end on their own.
    #[test]
    fn throttle_doubles_per_address_and_is_capped() {
        let clock = ManualClock::new();
        let t = LoginThrottle::new(clock.clone());
        for _ in 0..FREE_FAILURES_PER_ADDRESS {
            assert_eq!(t.failure(ip(1), "admin"), None);
            assert!(t.check(ip(1), "admin").is_ok());
        }
        let mut expected = BASE_DELAY;
        for _ in 0..12 {
            assert_eq!(t.failure(ip(1), "admin"), Some(expected));
            assert_eq!(t.check(ip(1), "admin"), Err(expected));
            assert!(t.check(ip(2), "other").is_ok(), "other address and console unaffected");
            clock.advance(expected - Duration::from_millis(1));
            assert!(t.check(ip(1), "admin").is_err());
            clock.advance(Duration::from_millis(1));
            assert!(t.check(ip(1), "admin").is_ok(), "block ends after its time");
            expected = (expected * 2).min(MAX_DELAY);
        }
        assert_eq!(expected, MAX_DELAY);
    }

    /// The console counter spans addresses and has its own free failures.
    #[test]
    fn throttle_per_console_across_addresses() {
        let clock = ManualClock::new();
        let t = LoginThrottle::new(clock.clone());
        for n in 0..FREE_FAILURES_PER_CONSOLE {
            assert_eq!(t.failure(ip(n as u8 + 10), "admin"), None);
        }
        assert_eq!(t.failure(ip(99), "admin"), Some(BASE_DELAY));
        assert_eq!(t.check(ip(100), "admin"), Err(BASE_DELAY), "fresh address, blocked console");
        assert!(t.check(ip(100), "viewer").is_ok());
        assert!(t.check(None, "admin").is_err(), "no address: console throttle still applies");
    }

    /// Success clears address and console; idle keys are forgotten.
    #[test]
    fn throttle_reset_by_success_and_by_idle_time() {
        let clock = ManualClock::new();
        let t = LoginThrottle::new(clock.clone());
        for _ in 0..=FREE_FAILURES_PER_ADDRESS {
            t.failure(ip(1), "admin");
        }
        assert!(t.check(ip(1), "admin").is_err());
        clock.advance(BASE_DELAY);
        t.success(ip(1), "admin");
        assert_eq!(t.failure(ip(1), "admin"), None, "count restarted after success");

        let t = LoginThrottle::new(clock.clone());
        for _ in 0..FREE_FAILURES_PER_ADDRESS {
            t.failure(ip(1), "admin");
        }
        clock.advance(FAILURE_MEMORY);
        assert_eq!(t.failure(ip(1), "admin"), None, "forgotten after FAILURE_MEMORY");
    }

    /// Random keys cannot grow the tables beyond MAX_TRACKED.
    #[test]
    fn throttle_tables_are_bounded() {
        let t = LoginThrottle::new(ManualClock::new());
        for n in 0..MAX_TRACKED + 50 {
            t.failure(None, &format!("guess{n}"));
        }
        assert_eq!(t.tables.lock().unwrap().by_console.len(), MAX_TRACKED);
    }

    /// End to end through login_from: wrong proofs block the address, a
    /// blocked login is refused even with the right proof, the block
    /// expires, and success resets the counter.
    #[test]
    fn login_from_is_throttled_and_recovers() {
        let clock = ManualClock::new();
        let base = state();
        let st = AuthState::with_clock(base.config.clone(), base.session_mgr.clone(), clock.clone());
        let peer: SocketAddr = "192.0.2.7:50000".parse().unwrap();
        let attempt = |password: &str| {
            let nonce = st.challenge("admin");
            st.login_from("admin", &nonce, &compute_proof(&password_key(password), &nonce), Some(peer))
        };
        for _ in 0..FREE_FAILURES_PER_ADDRESS + 1 {
            assert_eq!(attempt("wrong"), Err(LoginError::Failed("authentication failed".into())));
        }
        assert_eq!(attempt("adminpass"), Err(LoginError::Throttled(BASE_DELAY)));
        let other: SocketAddr = "192.0.2.8:50000".parse().unwrap();
        let nonce = st.challenge("admin");
        assert!(st.login_from("admin", &nonce, &compute_proof(&password_key("adminpass"), &nonce), Some(other)).is_ok(),
                "another address is not blocked by this one");
        clock.advance(BASE_DELAY);
        assert!(attempt("adminpass").is_ok());
        assert_eq!(attempt("wrong"), Err(LoginError::Failed("authentication failed".into())), "counter was reset");
        assert!(attempt("adminpass").is_ok());
    }

    /// An expired or foreign challenge is no password guess and not counted.
    #[test]
    fn bad_challenge_is_not_counted() {
        let st = state();
        for _ in 0..FREE_FAILURES_PER_CONSOLE * 2 {
            assert!(matches!(st.login_from("admin", "nonce", "proof", None), Err(LoginError::Failed(_))));
        }
        let nonce = st.challenge("admin");
        assert!(st.login("admin", &nonce, &compute_proof(&password_key("adminpass"), &nonce)).is_ok());
    }

    /// An unknown console counts as a failure like a wrong proof.
    #[test]
    fn unknown_console_counts() {
        let st = state();
        for _ in 0..FREE_FAILURES_PER_CONSOLE {
            let nonce = st.challenge("ghost");
            assert!(matches!(st.login_from("ghost", &nonce, "00", None), Err(LoginError::Failed(_))));
        }
        let nonce = st.challenge("ghost");
        assert!(matches!(st.login_from("ghost", &nonce, "00", None), Err(LoginError::Failed(_))));
        let nonce = st.challenge("ghost");
        assert!(matches!(st.login_from("ghost", &nonce, "00", None), Err(LoginError::Throttled(_))));
    }

    /// Push events follow the console of the session and stop with it.
    #[test]
    fn event_visibility_follows_session_console() {
        let cfg = crate::director_config::DirectorConfig::from_str(
            "Director {\n  Name = test-dir\n  Password = \"dirpass\"\n}\n\
             Console {\n  Name = limited\n  Password = \"pw\"\n  JobACL = Mine\n}\n",
        ).unwrap();
        let st = AuthState::new(Arc::new(RwLock::new(cfg)), Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16)))));
        let nonce = st.challenge("limited");
        let token = st.login("limited", &nonce, &compute_proof(&password_key("pw"), &nonce)).unwrap();
        let mut md = MetadataMap::new();
        md.insert(SESSION_HEADER, MetadataValue::try_from(token.as_str()).unwrap());
        let id = st.check_session(&md).unwrap();
        let job = |name: &str| crate::message_bus::BusMessage::JobStatus { job_id: 1, job_name: name.into(), status: "R".into(), timestamp: 0 };
        assert!(st.event_visible(id, &job("Mine")));
        assert!(!st.event_visible(id, &job("Theirs")));
        st.logout(&token);
        assert!(!st.event_visible(id, &job("Mine")), "no events after logout");
    }

    /// A missing or unknown token is rejected.
    #[test]
    fn missing_or_bad_token() {
        let st = state();
        assert!(st.check(&MetadataMap::new()).is_err());
        let mut md = MetadataMap::new();
        md.insert(SESSION_HEADER, MetadataValue::from_static("nope"));
        assert!(st.check(&md).is_err());
    }
}
