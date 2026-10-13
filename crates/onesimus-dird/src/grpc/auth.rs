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
    last_used: Instant,
}

pub struct AuthState {
    config: SharedConfig,
    /// nonce -> (console name, issued at); single use
    nonces: Mutex<HashMap<String, (String, Instant)>>,
    /// token -> session
    sessions: Mutex<HashMap<String, Session>>,
}

impl AuthState {
    pub fn new(config: SharedConfig) -> Self {
        Self {
            config,
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
        sessions.retain(|_, s| s.last_used.elapsed() < SESSION_IDLE_TTL);
        sessions.insert(token.clone(), Session { console_name: console, last_used: Instant::now() });
        Ok(token)
    }

    /// Check the session header. Returns the console name of the session.
    pub fn check(&self, metadata: &MetadataMap) -> Result<String, Status> {
        let token = metadata
            .get(SESSION_HEADER)
            .ok_or_else(|| Status::unauthenticated("not logged in"))?
            .to_str()
            .map_err(|_| Status::unauthenticated("invalid session token"))?;

        let mut sessions = self.sessions.lock().unwrap();
        match sessions.get_mut(token) {
            Some(s) if s.last_used.elapsed() < SESSION_IDLE_TTL => {
                s.last_used = Instant::now();
                Ok(s.console_name.clone())
            }
            _ => Err(Status::unauthenticated("invalid session token")),
        }
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
        AuthState::new(Arc::new(RwLock::new(cfg)))
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
    fn missing_or_bad_token() {
        let st = state();
        assert!(st.check(&MetadataMap::new()).is_err());
        let mut md = MetadataMap::new();
        md.insert(SESSION_HEADER, MetadataValue::from_static("nope"));
        assert!(st.check(&md).is_err());
    }
}
