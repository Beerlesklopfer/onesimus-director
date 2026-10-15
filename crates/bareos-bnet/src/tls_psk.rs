//! TLS support for Bareos — PSK and X.509 certificate modes.
//!
//! Bareos supports two TLS modes:
//! - PSK (Pre-Shared Key): Default for daemon-to-daemon, key from password
//! - X.509 Certificates: Optional, configured via TLS Certificate/Key/CA
//!
//! Uses OpenSSL via the `openssl` crate (rustls doesn't support PSK).
//!
//! # Limitierung: TLSv1.2 only fuer PSK
//!
//! TLS-PSK ist auf TLSv1.2 beschraenkt (`set_max_proto_version(TLS1_2)`).
//!
//! ## Grund
//!
//! TLSv1.3 hat einen grundlegend anderen PSK-Mechanismus als TLSv1.2:
//!
//! - **TLSv1.2**: `SSL_CTX_set_psk_server_callback` — der Callback bekommt
//!   die Identity und gibt den PSK als Byte-Array zurueck. Die `openssl` Crate
//!   exposed diesen Callback als `set_psk_server_callback()`.
//!
//! - **TLSv1.3**: `SSL_CTX_set_psk_find_session_callback` — der Callback muss
//!   eine komplette `SSL_SESSION` mit Cipher, Protokoll-Version und Master-Key
//!   erzeugen. OpenSSL verifiziert damit den PSK-Binder aus dem ClientHello.
//!   Die `openssl` Crate (0.10.x) exposed diesen Callback **nicht**.
//!
//! Ohne den `find_session` Callback schickt ein TLSv1.3-Client einen PSK-Binder
//! im ClientHello, OpenSSL kann ihn nicht verifizieren und bricht den Handshake
//! mit `tls_psk_do_binder: binder does not verify` ab — noch bevor ein Fallback
//! auf TLSv1.2 moeglich waere.
//!
//! ## Alternativen (nicht umgesetzt)
//!
//! - **openssl-sys FFI**: `SSL_CTX_set_psk_find_session_callback` direkt ueber
//!   `openssl_sys` aufrufen, `SSL_SESSION` manuell mit `SSL_SESSION_new()`,
//!   `SSL_SESSION_set_cipher()`, `SSL_SESSION_set1_master_key()` aufbauen.
//!   Getestet, funktioniert prinzipiell, aber der Binder-Hash muss exakt zum
//!   Client-Cipher passen (SHA256 vs SHA384). Fragil und schwer wartbar.
//!
//! - **Upstream PR**: Der `openssl` Crate einen `set_psk_find_session_callback`
//!   hinzufuegen (analog zu `set_psk_server_callback`).
//!
//! ## Kompatibilitaet
//!
//! Der C++ Bareos Director hat die gleiche Einschraenkung: er setzt nur
//! `SSL_CTX_set_psk_server_callback` (kein `find_session`), daher handelt
//! auch er TLSv1.2 fuer PSK-Verbindungen aus. Clients (bconsole, FD, SD)
//! akzeptieren TLSv1.2 problemlos. Es gibt keinen funktionalen Nachteil.

use std::io::{self, Read, Write};
use std::net::TcpStream;

use openssl::ssl::{self, SslMethod, SslConnector, SslAcceptor, SslStream, SslFiletype, SslVerifyMode};

/// TLS stream (PSK or certificate-based) wrapping a TCP connection.
pub struct PskStream {
    inner: SslStream<TcpStream>,
}

impl PskStream {
    /// Negotiated cipher and protocol, e.g. "TLS_CHACHA20_POLY1305_SHA256 TLSv1.3"
    /// (C++ TlsOpenSsl::TlsCipherGetName, lib/tls_openssl.cc:89); empty without a cipher.
    pub fn cipher_name(&self) -> String {
        let ssl = self.inner.ssl();
        match ssl.current_cipher() {
            Some(cipher) => format!("{} {}", cipher.name(), ssl.version_str()),
            None => String::new(),
        }
    }
}

/// " Encryption: <cipher>" or " Encryption: None" without TLS
/// (C++ BareosSocket::GetCipherMessageString, lib/bsock.cc:755).
pub fn cipher_message_string(cipher: Option<&str>) -> String {
    format!(" Encryption: {}", cipher.unwrap_or("None"))
}

impl Read for PskStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for PskStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

const BAREOS_CIPHERS: &str = "ALL:!ADH:!LOW:!EXP:!MD5:@STRENGTH";

// ============================================================================
// PSK mode
// ============================================================================

/// Connect via TLS-PSK (client mode).
pub fn psk_connect(stream: TcpStream, identity: &str, psk: &[u8]) -> io::Result<PskStream> {
    let mut builder = SslConnector::builder(SslMethod::tls())
        .map_err(ssl_err)?;

    let identity_owned = identity.to_string();
    let psk_owned = psk.to_vec();
    builder.set_psk_client_callback(move |_ssl, _hint, identity_buf, psk_buf| {
        let id_bytes = identity_owned.as_bytes();
        let id_len = id_bytes.len().min(identity_buf.len() - 1);
        identity_buf[..id_len].copy_from_slice(&id_bytes[..id_len]);
        identity_buf[id_len] = 0;
        let psk_len = psk_owned.len().min(psk_buf.len());
        psk_buf[..psk_len].copy_from_slice(&psk_owned[..psk_len]);
        Ok(psk_len)
    });

    builder.set_cipher_list(BAREOS_CIPHERS).map_err(ssl_err)?;
    let connector = builder.build();
    let ssl_stream = connector.connect("bareos", stream)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("PSK connect: {e}")))?;
    Ok(PskStream { inner: ssl_stream })
}

/// Accept via TLS-PSK, choosing the key by the client's PSK identity
/// (e.g. "R_CONSOLE\x1e<name>"), like the C++ daemons. Unknown identities
/// fail the handshake. Returns the stream and the identity the client used.
pub fn psk_accept_with<F>(stream: TcpStream, lookup: F) -> io::Result<(PskStream, String)>
where
    F: Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static,
{
    let mut ctx = ssl::SslContext::builder(SslMethod::tls_server())
        .map_err(ssl_err)?;
    // TLSv1.2 only — see module docs
    ctx.set_max_proto_version(Some(ssl::SslVersion::TLS1_2)).map_err(ssl_err)?;

    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let seen_cb = seen.clone();
    ctx.set_psk_server_callback(move |_ssl, identity, psk_buf| {
        let id = identity.map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        *seen_cb.lock().unwrap() = id.clone();
        match lookup(&id) {
            Some(psk) => {
                let n = psk.len().min(psk_buf.len());
                psk_buf[..n].copy_from_slice(&psk[..n]);
                Ok(n)
            }
            None => Ok(0), // no key: handshake fails
        }
    });
    ctx.set_cipher_list(BAREOS_CIPHERS).map_err(ssl_err)?;

    let ctx = ctx.build();
    let ssl_stream = ssl::Ssl::new(&ctx)
        .map_err(ssl_err)?
        .accept(stream)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("PSK accept: {e}")))?;
    let identity = seen.lock().unwrap().clone();
    Ok((PskStream { inner: ssl_stream }, identity))
}

/// Accept via TLS-PSK (server mode).
pub fn psk_accept(stream: TcpStream, psk: &[u8]) -> io::Result<PskStream> {
    let mut ctx = ssl::SslContext::builder(SslMethod::tls_server())
        .map_err(ssl_err)?;

    // TLSv1.2 only — siehe Modul-Doku oben fuer die ausfuehrliche Begruendung.
    // Kurzfassung: TLSv1.3 PSK braucht SSL_CTX_set_psk_find_session_callback,
    // den die openssl Crate nicht exposed. Ohne ihn scheitert der TLSv1.3
    // Binder-Check und der Handshake bricht ab ohne TLSv1.2-Fallback.
    ctx.set_max_proto_version(Some(ssl::SslVersion::TLS1_2)).map_err(ssl_err)?;

    // PSK server callback
    let psk_owned = psk.to_vec();
    ctx.set_psk_server_callback(move |_ssl, _identity, psk_buf| {
        let n = psk_owned.len().min(psk_buf.len());
        psk_buf[..n].copy_from_slice(&psk_owned[..n]);
        Ok(n)
    });

    ctx.set_cipher_list(BAREOS_CIPHERS).map_err(ssl_err)?;

    let ctx = ctx.build();
    let ssl_stream = ssl::Ssl::new(&ctx)
        .map_err(ssl_err)?
        .accept(stream)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("PSK accept: {e}")))?;

    Ok(PskStream { inner: ssl_stream })
}

// ============================================================================
// X.509 Certificate mode
// ============================================================================

/// Connect via TLS with X.509 certificates (client mode).
pub fn cert_connect(
    stream: TcpStream,
    cert_file: Option<&str>,
    key_file: Option<&str>,
    ca_file: Option<&str>,
    verify_peer: bool,
) -> io::Result<PskStream> {
    let mut builder = SslConnector::builder(SslMethod::tls()).map_err(ssl_err)?;

    if let (Some(cert), Some(key)) = (cert_file, key_file) {
        builder.set_certificate_chain_file(cert).map_err(ssl_err)?;
        builder.set_private_key_file(key, SslFiletype::PEM).map_err(ssl_err)?;
    }
    if let Some(ca) = ca_file {
        builder.set_ca_file(ca).map_err(ssl_err)?;
    }
    if !verify_peer {
        builder.set_verify(SslVerifyMode::NONE);
    }

    builder.set_cipher_list(BAREOS_CIPHERS).map_err(ssl_err)?;
    let connector = builder.build();
    let ssl_stream = connector.connect("bareos", stream)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("cert connect: {e}")))?;
    Ok(PskStream { inner: ssl_stream })
}

/// Accept via TLS with X.509 certificates (server mode).
pub fn cert_accept(
    stream: TcpStream,
    cert_file: &str,
    key_file: &str,
    ca_file: Option<&str>,
    verify_peer: bool,
) -> io::Result<PskStream> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).map_err(ssl_err)?;

    builder.set_certificate_chain_file(cert_file).map_err(ssl_err)?;
    builder.set_private_key_file(key_file, SslFiletype::PEM).map_err(ssl_err)?;
    if let Some(ca) = ca_file {
        builder.set_ca_file(ca).map_err(ssl_err)?;
    }
    if verify_peer {
        builder.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
    } else {
        builder.set_verify(SslVerifyMode::NONE);
    }

    builder.set_cipher_list(BAREOS_CIPHERS).map_err(ssl_err)?;
    let acceptor = builder.build();
    let ssl_stream = acceptor.accept(stream)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("cert accept: {e}")))?;
    Ok(PskStream { inner: ssl_stream })
}

// ============================================================================
// Combined mode: PSK + certificates in one acceptor
// ============================================================================

/// Accept TLS — supports both PSK and certificate clients.
pub fn accept_any_tls(
    stream: TcpStream,
    psk: &[u8],
    cert_file: Option<&str>,
    key_file: Option<&str>,
    ca_file: Option<&str>,
) -> io::Result<PskStream> {
    let mut builder = SslAcceptor::mozilla_intermediate(SslMethod::tls()).map_err(ssl_err)?;

    // PSK callback
    let psk_owned = psk.to_vec();
    builder.set_psk_server_callback(move |_ssl, _identity, psk_buf| {
        let psk_len = psk_owned.len().min(psk_buf.len());
        psk_buf[..psk_len].copy_from_slice(&psk_owned[..psk_len]);
        Ok(psk_len)
    });

    // Certificates (optional)
    if let (Some(cert), Some(key)) = (cert_file, key_file) {
        let _ = builder.set_certificate_chain_file(cert);
        let _ = builder.set_private_key_file(key, SslFiletype::PEM);
    }
    if let Some(ca) = ca_file {
        let _ = builder.set_ca_file(ca);
    }

    builder.set_verify(SslVerifyMode::NONE);
    builder.set_cipher_list(BAREOS_CIPHERS).map_err(ssl_err)?;

    let acceptor = builder.build();
    let ssl_stream = acceptor.accept(stream)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("TLS accept: {e}")))?;
    Ok(PskStream { inner: ssl_stream })
}

// ============================================================================
// Helpers
// ============================================================================

/// PSK aus einem Bareos-Passwort ableiten.
///
/// Der C++ Config-Parser (StoreMd5Password) hasht Klartext-Passwoerter zu MD5-Hex
/// und speichert das Ergebnis. Dieser 32-Byte ASCII-Hex-String ist der PSK.
///
/// Unser Rust-Parser speichert das Passwort als Klartext, daher hashen wir hier.
pub fn password_to_psk(password: &str) -> Vec<u8> {
    crate::cram_md5::password_key(password).into_bytes()
}

fn ssl_err(e: openssl::error::ErrorStack) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn psk_client_server_roundtrip() {
        let psk = password_to_psk("testpassword");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let psk_server = psk.clone();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut s = psk_accept(stream, &psk_server).unwrap();
            let mut buf = [0u8; 100];
            let n = s.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"hello psk");
            s.write_all(b"hello back").unwrap();
        });

        let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        let mut c = psk_connect(stream, "test-dir", &psk).unwrap();
        c.write_all(b"hello psk").unwrap();
        c.flush().unwrap();
        let mut buf = [0u8; 100];
        let n = c.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello back");
        server.join().unwrap();
    }

    #[test]
    fn cert_client_server_roundtrip() {
        // Generate self-signed cert for testing
        use std::process::Command;
        let dir = std::env::temp_dir().join("bareos-cert-test");
        std::fs::create_dir_all(&dir).ok();
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");

        Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048",
                   "-keyout", key.to_str().unwrap(),
                   "-out", cert.to_str().unwrap(),
                   "-days", "1", "-nodes", "-subj", "/CN=test"])
            .output().ok();

        if !cert.exists() {
            // Skip if openssl not available
            return;
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let cert_s = cert.to_str().unwrap().to_string();
        let key_s = key.to_str().unwrap().to_string();
        let cert_c = cert_s.clone();

        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut s = cert_accept(stream, &cert_s, &key_s, None, false).unwrap();
            let mut buf = [0u8; 100];
            let n = s.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"cert hello");
            s.write_all(b"cert reply").unwrap();
        });

        let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        let mut c = cert_connect(stream, None, None, Some(&cert_c), false).unwrap();
        c.write_all(b"cert hello").unwrap();
        c.flush().unwrap();
        let mut buf = [0u8; 100];
        let n = c.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"cert reply");
        server.join().unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn password_to_psk_deterministic() {
        let psk1 = password_to_psk("test");
        let psk2 = password_to_psk("test");
        assert_eq!(psk1, psk2);
        assert_eq!(psk1.len(), 32);
    }
}
