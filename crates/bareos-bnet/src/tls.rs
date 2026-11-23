// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! TLS support for BNET connections using rustls.
//!
//! TLS wraps the entire TCP socket — all BNET framing (headers + payloads)
//! is encrypted after the TLS handshake completes.

use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};

/// TLS configuration loaded from Bareos resource directives.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub ca_certfile: Option<String>,
    pub certfile: Option<String>,
    pub keyfile: Option<String>,
    pub verify_peer: bool,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            ca_certfile: None,
            certfile: None,
            keyfile: None,
            verify_peer: true,
        }
    }
}

/// A stream that is either plaintext TCP or TLS-encrypted.
pub enum TlsOrPlain {
    Plain(TcpStream),
    Tls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
    TlsServer(rustls::StreamOwned<rustls::ServerConnection, TcpStream>),
}

impl Read for TlsOrPlain {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            TlsOrPlain::Plain(s) => s.read(buf),
            TlsOrPlain::Tls(s) => s.read(buf),
            TlsOrPlain::TlsServer(s) => s.read(buf),
        }
    }
}

impl Write for TlsOrPlain {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            TlsOrPlain::Plain(s) => s.write(buf),
            TlsOrPlain::Tls(s) => s.write(buf),
            TlsOrPlain::TlsServer(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            TlsOrPlain::Plain(s) => s.flush(),
            TlsOrPlain::Tls(s) => s.flush(),
            TlsOrPlain::TlsServer(s) => s.flush(),
        }
    }
}

/// Load PEM certificates from file.
pub fn load_certs(path: &str) -> io::Result<Vec<CertificateDer<'static>>> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    CertificateDer::pem_reader_iter(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// Load private key from PEM file.
pub fn load_private_key(path: &str) -> io::Result<PrivateKeyDer<'static>> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    PrivateKeyDer::from_pem_reader(&mut reader)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("no private key found in PEM: {e}")))
}

/// Build a rustls ServerConfig from TlsConfig.
pub fn build_server_config(config: &TlsConfig) -> io::Result<Arc<rustls::ServerConfig>> {
    let certs = load_certs(config.certfile.as_deref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "certfile required"))?)?;
    let key = load_private_key(config.keyfile.as_deref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "keyfile required"))?)?;

    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    Ok(Arc::new(server_config))
}

/// Build a rustls ClientConfig from TlsConfig.
pub fn build_client_config(config: &TlsConfig) -> io::Result<Arc<rustls::ClientConfig>> {
    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ca_path) = &config.ca_certfile {
        let ca_certs = load_certs(ca_path)?;
        for cert in ca_certs {
            root_store.add(cert)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        }
    }

    let builder = rustls::ClientConfig::builder()
        .with_root_certificates(root_store);

    let client_config = if let (Some(certfile), Some(keyfile)) = (&config.certfile, &config.keyfile) {
        let certs = load_certs(certfile)?;
        let key = load_private_key(keyfile)?;
        builder.with_client_auth_cert(certs, key)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
    } else {
        builder.with_no_client_auth()
    };

    Ok(Arc::new(client_config))
}

/// Upgrade a TCP stream to TLS (client mode).
pub fn tls_connect(
    stream: TcpStream,
    config: &Arc<rustls::ClientConfig>,
    server_name: &str,
) -> io::Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>> {
    let name = ServerName::try_from(server_name.to_string())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let conn = rustls::ClientConnection::new(config.clone(), name)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(rustls::StreamOwned::new(conn, stream))
}

/// Upgrade a TCP stream to TLS (server mode).
pub fn tls_accept(
    stream: TcpStream,
    config: &Arc<rustls::ServerConfig>,
) -> io::Result<rustls::StreamOwned<rustls::ServerConnection, TcpStream>> {
    let conn = rustls::ServerConnection::new(config.clone())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok(rustls::StreamOwned::new(conn, stream))
}

/// Bareos TLS policy values (matches C++ TlsPolicy enum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum TlsPolicy {
    None = 0,
    Auto = 1,
    Desired = 2,
    Required = 3,
}

impl TlsPolicy {
    pub fn from_i32(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::None),
            1 => Some(Self::Auto),
            2 => Some(Self::Desired),
            3 => Some(Self::Required),
            _ => None,
        }
    }
}
