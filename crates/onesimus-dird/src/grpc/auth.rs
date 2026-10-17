//! gRPC authentication: challenge/response login + session tokens.
//!
//! Login (see director_service.proto):
//!   key   = lowercase hex MD5 of the console password (as for TLS-PSK)
//!   proof = lowercase hex HMAC-SHA256(key, nonce)
//! The password never crosses the wire; the transport is TLS anyway.
//! After a successful login every RPC carries the session token in the
//! `x-onesimus-session` metadata header.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use tonic::Status;
use tonic::metadata::MetadataMap;

use crate::director_config::SharedConfig;

pub const SESSION_HEADER: &str = "x-onesimus-session";
const NONCE_TTL: Duration = Duration::from_secs(60);
const SESSION_IDLE_TTL: Duration = Duration::from_secs(8 * 3600);
const DEFAULT_CONSOLE: &str = "*UserAgent*";

struct Session {
    console_name: String,
    session_id: u64,
    last_used: Instant,
}

pub struct AuthState {
    config: SharedConfig,
    session_mgr: std::sync::Arc<crate::session_manager::SessionManager>,
    /// nonce -> (console name, issued at); single use
    nonces: Mutex<HashMap<String, (String, Instant)>>,
    /// token -> session
    sessions: Mutex<HashMap<String, Session>>,
}

impl AuthState {
    pub fn new(config: SharedConfig, session_mgr: std::sync::Arc<crate::session_manager::SessionManager>) -> Self {
        Self {
            config,
            session_mgr,
            nonces: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
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

    /// Verify a login proof. Returns the new session token.
    pub fn login(&self, console_name: &str, nonce: &str, proof: &str) -> Result<String, String> {
        let console = normalize_console(console_name);

        let issued = self.nonces.lock().unwrap().remove(nonce);
        match issued {
            Some((c, t)) if c == console && t.elapsed() < NONCE_TTL => {}
            _ => return Err("invalid or expired challenge".into()),
        }

        let key = self.console_key(&console).ok_or("authentication failed")?;
        if !constant_time_eq(compute_proof(&key, nonce).as_bytes(), proof.to_ascii_lowercase().as_bytes()) {
            return Err("authentication failed".into());
        }

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

    /// Check the session header. Returns the console name of the session.
    pub fn check(&self, metadata: &MetadataMap) -> Result<String, Status> {
        self.identity(metadata).map(|(name,_)|name)
    }
    pub fn check_session(&self, metadata: &MetadataMap) -> Result<u64, Status> {
        self.identity(metadata).map(|(_,id)|id)
    }
    fn identity(&self, metadata: &MetadataMap) -> Result<(String,u64), Status> {
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

    #[cfg(test)]
    pub(crate) fn expire_for_test(&self, token:&str) {
        self.sessions.lock().unwrap().get_mut(token).unwrap().last_used=Instant::now()-SESSION_IDLE_TTL-Duration::from_secs(1);
    }
    pub fn allowed(&self, id:u64, kind:crate::acl::Kind, item:&str) -> bool {
        let Some(console)=self.session_mgr.authenticated_console(id) else {return false;};
        self.config.read().map(|cfg|crate::acl::allowed(&cfg,&console,kind,item)).unwrap_or(false)
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

/// MD5 hex of the password; config values already stored as "[md5]<hex>" are used as is.
pub fn password_key(password: &str) -> String {
    match password.strip_prefix("[md5]") {
        Some(hex) => hex.to_ascii_lowercase(),
        None => bareos_bnet::cram_md5::md5_hash_password(password),
    }
}

pub fn compute_proof(key: &str, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(nonce.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn normalize_console(name: &str) -> String {
    if name.is_empty() { DEFAULT_CONSOLE.to_string() } else { name.to_string() }
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};
    use tonic::metadata::MetadataValue;

    fn state() -> AuthState {
        let cfg = crate::director_config::DirectorConfig::from_str(
            "Director {\n  Name = test-dir\n  Password = \"dirpass\"\n}\n\
             Console {\n  Name = admin\n  Password = \"adminpass\"\n}\n",
        ).unwrap();
        AuthState::new(Arc::new(RwLock::new(cfg)), Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16)))))
    }

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

    #[test]
    fn login_default_console_uses_director_password() {
        let st = state();
        let nonce = st.challenge("");
        let proof = compute_proof(&password_key("dirpass"), &nonce);
        assert!(st.login("*UserAgent*", &nonce, &proof).is_ok());
    }

    #[test]
    fn wrong_password_rejected() {
        let st = state();
        let nonce = st.challenge("admin");
        let proof = compute_proof(&password_key("wrong"), &nonce);
        assert!(st.login("admin", &nonce, &proof).is_err());
    }

    #[test]
    fn nonce_is_single_use() {
        let st = state();
        let nonce = st.challenge("admin");
        let proof = compute_proof(&password_key("adminpass"), &nonce);
        assert!(st.login("admin", &nonce, &proof).is_ok());
        assert!(st.login("admin", &nonce, &proof).is_err());
    }

    #[test]
    fn nonce_bound_to_console() {
        let st = state();
        let nonce = st.challenge("admin");
        let proof = compute_proof(&password_key("dirpass"), &nonce);
        assert!(st.login("*UserAgent*", &nonce, &proof).is_err());
    }

    #[test]
    fn md5_prefixed_password() {
        let hex = bareos_bnet::cram_md5::md5_hash_password("secret");
        assert_eq!(password_key(&format!("[md5]{hex}")), hex);
        assert_eq!(password_key("secret"), hex);
    }

    #[test]
    fn expired_token_revokes_server_allocated_session() {
        let st=state();let nonce=st.challenge("admin");
        let token=st.login("admin",&nonce,&compute_proof(&password_key("adminpass"),&nonce)).unwrap();
        let mut md=MetadataMap::new();md.insert(SESSION_HEADER,MetadataValue::try_from(token.as_str()).unwrap());
        let id=st.check_session(&md).unwrap();assert_eq!(st.session_mgr.authenticated_console(id).as_deref(),Some("admin"));
        st.expire_for_test(&token);assert!(st.check_session(&md).is_err());
        assert!(st.session_mgr.authenticated_console(id).is_none());
    }
    #[test]
    fn missing_or_bad_token() {
        let st = state();
        assert!(st.check(&MetadataMap::new()).is_err());
        let mut md = MetadataMap::new();
        md.insert(SESSION_HEADER, MetadataValue::from_static("nope"));
        assert!(st.check(&md).is_err());
    }
}
