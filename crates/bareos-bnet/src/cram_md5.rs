//! CRAM-MD5 two-way authentication for Bareos daemon connections.
//!
//! Protocol:
//! 1. Server sends challenge: "auth cram-md5 <random.timestamp@name> ssl=N\n"
//! 2. Client responds with Base64(HMAC-MD5(challenge, password))
//! 3. Server verifies and sends "1000 OK auth\n" or "1999 Authorization failed.\n"
//! 4. Roles swap for two-way auth.

use std::io::{Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, Mac};
use md5::Md5;
use rand::Rng;

use crate::wire::{BnetStream, BnetError};

type HmacMd5 = Hmac<Md5>;

/// Result of a CRAM-MD5 two-way authentication.
#[derive(Debug)]
pub struct AuthResult {
    /// Whether authentication succeeded.
    pub success: bool,
    /// The server's challenge string (for session token derivation).
    pub server_challenge: String,
}

/// CRAM-MD5 authenticator.
pub struct CramMd5Auth {
    password: String,
    qualified_name: String,
    tls_policy: i32,
}

impl CramMd5Auth {
    /// Create authenticator with raw password (already MD5-hashed hex string).
    pub fn new_raw(password_md5: &str, qualified_name: &str, tls_policy: i32) -> Self {
        Self {
            password: password_md5.to_string(),
            qualified_name: bash_spaces(qualified_name),
            tls_policy,
        }
    }

    /// Create authenticator from a configured password (plaintext, or
    /// "[md5]<hex>" as Bareos stores it). The key is the MD5 hex (Bareos convention).
    pub fn new(password: &str, qualified_name: &str, tls_policy: i32) -> Self {
        let password_md5 = password_key(password);
        Self {
            password: password_md5,
            qualified_name: bash_spaces(qualified_name),
            tls_policy,
        }
    }

    /// Generate a challenge string in Bareos format.
    fn generate_challenge(&self) -> String {
        let mut rng = rand::thread_rng();
        let random_id: u32 = rng.r#gen();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as u32;

        format!(
            "<{random_id}.{timestamp}@{}>",
            self.qualified_name
        )
    }

    /// Act as challenger (server): send challenge, verify response.
    /// Returns true if the peer authenticated correctly.
    pub fn challenge<S: Read + Write>(
        &self,
        stream: &mut BnetStream<S>,
    ) -> Result<(bool, String), BnetError> {
        let challenge = self.generate_challenge();

        // Send challenge
        let auth_msg = format!(
            "auth cram-md5 {challenge} ssl={}\n",
            self.tls_policy
        );
        stream.send_str(&auth_msg)?;

        // Receive HMAC response
        let response = stream.recv_str()?;

        // Compute HMAC-MD5
        let mut mac = HmacMd5::new_from_slice(self.password.as_bytes())
            .expect("HMAC accepts any key length");
        mac.update(challenge.as_bytes());
        let hmac = mac.finalize().into_bytes();

        // Try all encoding variants (like C++ CramMd5Challenge does)
        let expected_std = BASE64.encode(&hmac);
        let expected_compat = bareos_bin_to_base64(&hmac, true);
        let expected_bareos = bareos_bin_to_base64(&hmac, false);

        let resp = response.trim().trim_end_matches('\0');
        crate::dmsg!(100, "CRAM-MD5 challenge sent: {challenge:?}");
        crate::dmsg!(100, "CRAM-MD5 response got={resp:?} expected_bareos={expected_bareos:?}");

        let ok = resp == expected_std.trim()
              || resp == expected_std.trim_end_matches('=')
              || resp == expected_compat
              || resp == expected_bareos;

        crate::dmsg!(100, "CRAM-MD5 challenge phase: match={ok}");

        // Send result
        if ok {
            stream.send_str("1000 OK auth\n")?;
        } else {
            stream.send_str("1999 Authorization failed.\n")?;
        }

        Ok((ok, challenge))
    }

    /// Act as responder (client): receive challenge, send HMAC response.
    /// Returns the server's challenge string.
    pub fn respond<S: Read + Write>(
        &self,
        stream: &mut BnetStream<S>,
    ) -> Result<(bool, String), BnetError> {
        // Receive challenge
        let challenge_msg = stream.recv_str()?;
        crate::dmsg!(100, "CRAM-MD5 respond: received challenge");

        // Parse challenge: "auth cram-md5 <...> ssl=N\n"
        let challenge = parse_challenge(&challenge_msg)
            .unwrap_or_default();

        // Compute HMAC-MD5 using Bareos-compatible encoding (compatible_=false)
        let hmac_response = compute_hmac_md5_bareos_base64(challenge.as_bytes(), self.password.as_bytes());
        crate::dmsg!(116, "CRAM-MD5 respond: sending HMAC");

        // Send response
        stream.send_str(&hmac_response)?;

        // Receive result
        let result = stream.recv_str()?;
        crate::dmsg!(100, "CRAM-MD5 respond: result={}", result.trim());
        let ok = result.contains("1000 OK auth");

        Ok((ok, challenge))
    }

    /// Two-way authentication.
    ///
    /// - `initiated_by_remote=false`: We are the client (respond first, then challenge).
    /// - `initiated_by_remote=true`: We are the server (challenge first, then respond).
    pub fn two_way_authenticate<S: Read + Write>(
        &self,
        stream: &mut BnetStream<S>,
        initiated_by_remote: bool,
    ) -> Result<AuthResult, BnetError> {
        if initiated_by_remote {
            // Server: challenge first, then respond
            let (peer_ok, _our_challenge) = self.challenge(stream)?;
            if !peer_ok {
                return Ok(AuthResult {
                    success: false,
                    server_challenge: String::new(),
                });
            }
            let (we_ok, server_challenge) = self.respond(stream)?;
            Ok(AuthResult {
                success: we_ok,
                server_challenge,
            })
        } else {
            // Client: respond first, then challenge
            let (we_ok, server_challenge) = self.respond(stream)?;
            if !we_ok {
                return Ok(AuthResult {
                    success: false,
                    server_challenge,
                });
            }
            let (peer_ok, _our_challenge) = self.challenge(stream)?;
            Ok(AuthResult {
                success: peer_ok,
                server_challenge,
            })
        }
    }
}

/// Compute HMAC-MD5 and return Base64-encoded result (standard encoding).
fn compute_hmac_md5_base64(challenge: &[u8], password: &[u8]) -> String {
    let mut mac = HmacMd5::new_from_slice(password)
        .expect("HMAC accepts any key length");
    mac.update(challenge);
    let result = mac.finalize().into_bytes();
    BASE64.encode(result)
}

/// Compute HMAC-MD5 and return Bareos-compatible Base64 (non-standard).
fn compute_hmac_md5_bareos_base64(challenge: &[u8], password: &[u8]) -> String {
    let mut mac = HmacMd5::new_from_slice(password)
        .expect("HMAC accepts any key length");
    mac.update(challenge);
    let result = mac.finalize().into_bytes();
    bareos_bin_to_base64(&result, false)
}

/// Bareos-specific Base64 encoding — exact port of C++ BinToBase64 from base64.cc.
///
/// compatible=true: treats input bytes as uint8_t (standard)
/// compatible=false: treats input bytes as int8_t (sign-extension, Bareos-specific)
pub fn bareos_bin_to_base64(bin: &[u8], compatible: bool) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut buf = Vec::new();
    let mut reg: u32 = 0;
    let mut rem: u32 = 0;
    let mut i: usize = 0;

    while i < bin.len() {
        if rem < 6 {
            reg <<= 8;
            if compatible {
                reg |= bin[i] as u32;
            } else {
                // C++: reg |= (int8_t)bin[i];
                // int8_t sign-extends negative values, wrapping in u32
                reg |= (bin[i] as i8 as i32 as u32);
            }
            i += 1;
            rem += 8;
        }
        let save = reg;
        reg >>= rem - 6;
        buf.push(DIGITS[(reg & 0x3F) as usize]);
        reg = save;
        rem -= 6;
    }

    if rem > 0 {
        let mask: u32 = (1 << rem) - 1;
        if compatible {
            buf.push(DIGITS[((reg & mask) << (6 - rem)) as usize]);
        } else {
            buf.push(DIGITS[(reg & mask) as usize]);
        }
    }

    String::from_utf8(buf).unwrap()
}

/// Parse the challenge string from an auth message.
/// Input: "auth cram-md5 <12345.67890@name> ssl=0\n"
/// Output: "<12345.67890@name>"
fn parse_challenge(msg: &str) -> Option<String> {
    let msg = msg.trim();
    // Find the challenge between angle brackets
    let start = msg.find('<')?;
    let end = msg.find('>')?;
    if end > start {
        Some(msg[start..=end].to_string())
    } else {
        None
    }
}

/// Parse the TLS policy from an auth message.
/// Input: "auth cram-md5 <...> ssl=2\n"
/// Output: Some(2)
pub fn parse_tls_policy(msg: &str) -> Option<i32> {
    let msg = msg.trim();
    if let Some(pos) = msg.find("ssl=") {
        let rest = &msg[pos + 4..];
        let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        num_str.parse().ok()
    } else {
        None
    }
}

/// Replace spaces with underscores (Bareos "BashSpaces" convention).
pub fn bash_spaces(s: &str) -> String {
    s.replace(' ', "_")
}

/// Replace underscores with spaces (Bareos "UnbashSpaces" convention).
pub fn unbash_spaces(s: &str) -> String {
    s.replace('_', " ")
}

/// Key for CRAM-MD5 / TLS-PSK from a configured password: "[md5]<hex>" is
/// already the MD5 hex (Bareos stores passwords that way); anything else is
/// hashed. Without this, an "[md5]" password would be hashed a second time.
pub fn password_key(password: &str) -> String {
    match password.strip_prefix("[md5]") {
        Some(hex) => hex.to_ascii_lowercase(),
        None => md5_hash_password(password),
    }
}

/// MD5-hash a plaintext password to hex string (Bareos password storage convention).
/// "testpassword" → "e16b2ab8d12314bf4efbd6203906ea6c"
pub fn md5_hash_password(password: &str) -> String {
    use md5::{Md5, Digest};
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

/// Derive a session token from password and server challenge using HMAC-SHA256.
/// Used for gRPC authentication after CRAM-MD5 handshake.
pub fn derive_session_token(password: &str, server_challenge: &str) -> String {
    use hmac::Mac;
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(password.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(server_challenge.as_bytes());
    let result = mac.finalize().into_bytes();
    hex::encode(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn make_pair() -> (BnetStream<std::net::TcpStream>, BnetStream<std::net::TcpStream>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = std::net::TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        (BnetStream::new(client), BnetStream::new(server))
    }

    #[test]
    fn hmac_md5_test_vector() {
        // RFC 2104 test vector 2
        let result = compute_hmac_md5_base64(
            b"what do ya want for nothing?",
            b"Jefe",
        );
        // Expected HMAC-MD5: 750c783e6ab0b503eaa86e310a5db738
        let decoded = BASE64.decode(&result).unwrap();
        let hex: String = decoded.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "750c783e6ab0b503eaa86e310a5db738");
    }

    #[test]
    fn parse_challenge_valid() {
        let msg = "auth cram-md5 <12345.67890@bareos-dir> ssl=0\n";
        assert_eq!(parse_challenge(msg), Some("<12345.67890@bareos-dir>".into()));
    }

    #[test]
    fn parse_challenge_with_qualified_name() {
        let msg = "auth cram-md5 <99.100@my-dir> ssl=1 qualified-name=bareos-sd\n";
        assert_eq!(parse_challenge(msg), Some("<99.100@my-dir>".into()));
    }

    #[test]
    fn parse_tls_policy_values() {
        assert_eq!(parse_tls_policy("auth cram-md5 <x> ssl=0\n"), Some(0));
        assert_eq!(parse_tls_policy("auth cram-md5 <x> ssl=2\n"), Some(2));
        assert_eq!(parse_tls_policy("auth cram-md5 <x> ssl=3\n"), Some(3));
        assert_eq!(parse_tls_policy("no ssl here"), None);
    }

    #[test]
    fn bash_unbash_spaces() {
        assert_eq!(bash_spaces("my director"), "my_director");
        assert_eq!(unbash_spaces("my_director"), "my director");
        assert_eq!(bash_spaces("no_spaces"), "no_spaces");
    }

    #[test]
    fn session_token_deterministic() {
        let t1 = derive_session_token("password123", "<12345.67890@dir>");
        let t2 = derive_session_token("password123", "<12345.67890@dir>");
        assert_eq!(t1, t2);
        assert_eq!(t1.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn session_token_different_inputs() {
        let t1 = derive_session_token("pass1", "<challenge1>");
        let t2 = derive_session_token("pass2", "<challenge1>");
        let t3 = derive_session_token("pass1", "<challenge2>");
        assert_ne!(t1, t2);
        assert_ne!(t1, t3);
    }

    #[test]
    fn bareos_base64_compatible() {
        // Standard base64 of the HMAC test vector
        let hmac_bytes = hex::decode("750c783e6ab0b503eaa86e310a5db738").unwrap();
        let compatible = bareos_bin_to_base64(&hmac_bytes, true);
        let standard = BASE64.encode(&hmac_bytes);
        // compatible=true should match standard base64 (minus padding)
        assert_eq!(compatible, standard.trim_end_matches('='));
    }

    #[test]
    fn bareos_base64_non_compatible() {
        // Non-compatible encoding differs from standard
        let hmac_bytes = hex::decode("750c783e6ab0b503eaa86e310a5db738").unwrap();
        let non_compat = bareos_bin_to_base64(&hmac_bytes, false);
        // Should produce output without '=' padding and possibly different chars
        assert!(!non_compat.is_empty());
        assert!(!non_compat.contains('='));
    }

    #[test]
    fn two_way_auth_success() {
        let (mut client_stream, mut server_stream) = make_pair();
        let password = "test_password";

        let server = std::thread::spawn(move || {
            let auth = CramMd5Auth::new(password, "bareos-dir", 0);
            auth.two_way_authenticate(&mut server_stream, true).unwrap()
        });

        let client_auth = CramMd5Auth::new(password, "client-fd", 0);
        let client_result = client_auth
            .two_way_authenticate(&mut client_stream, false)
            .unwrap();

        let server_result = server.join().unwrap();

        assert!(client_result.success, "client auth should succeed");
        assert!(server_result.success, "server auth should succeed");
        assert!(!client_result.server_challenge.is_empty());
    }

    #[test]
    fn two_way_auth_wrong_password() {
        let (mut client_stream, mut server_stream) = make_pair();

        let server = std::thread::spawn(move || {
            let auth = CramMd5Auth::new("server_pass", "bareos-dir", 0);
            auth.two_way_authenticate(&mut server_stream, true).unwrap()
        });

        let client_auth = CramMd5Auth::new("wrong_pass", "client-fd", 0);
        let client_result = client_auth
            .two_way_authenticate(&mut client_stream, false)
            .unwrap();

        let server_result = server.join().unwrap();

        assert!(!server_result.success, "server should reject wrong password");
        // Client may or may not see failure depending on timing
    }

    #[test]
    fn md5_prefixed_password_is_not_hashed_twice() {
        let hex = md5_hash_password("secret");
        assert_eq!(password_key("secret"), hex);
        assert_eq!(password_key(&format!("[md5]{}", hex.to_uppercase())), hex);
    }
}
