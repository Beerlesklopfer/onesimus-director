// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Unified stream type that supports plaintext, TLS (rustls), TLS-PSK
//! (openssl) and any other transport the caller brings (gRPC: the crate
//! `onesimus-bnet-grpc`), so `BnetStream<AnyStream>` runs over each.
//!
//! Since 20.11.2026 the gRPC transport is no longer a variant here: this
//! crate stays free of tonic and the protos (ofd audit §5 point 1); a
//! transport outside it implements [`ExternalStream`].

use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::tls_psk::PskStream;

/// A byte stream from outside this crate (e.g. a gRPC stream carrying
/// BNET frames).
pub trait ExternalStream: Read + Write + Send {
    /// Name of the transport for log lines and job messages
    /// (e.g. `"gRPC TLS"`).
    fn transport(&self) -> &str;
}

/// A unified stream that can be plaintext TCP, TLS (rustls), TLS-PSK
/// (openssl) or an [`ExternalStream`].
pub enum AnyStream {
    /// Plaintext TCP.
    Plain(TcpStream),
    /// A transport from outside this crate (gRPC).
    External(Box<dyn ExternalStream>),
    /// TLS-PSK (openssl).
    Psk(PskStream),
    /// TLS server side (rustls).
    RustlsServer(rustls::StreamOwned<rustls::ServerConnection, TcpStream>),
    /// TLS client side (rustls).
    RustlsClient(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl AnyStream {
    /// The TCP socket under the stream (None for an external transport): for a liveness check of an
    /// idle connection and to keep its descriptor across a fork.
    pub fn tcp(&self) -> Option<&TcpStream> {
        match self {
            AnyStream::Plain(s) => Some(s),
            AnyStream::Psk(s) => Some(s.tcp()),
            AnyStream::RustlsServer(s) => Some(&s.sock),
            AnyStream::RustlsClient(s) => Some(&s.sock),
            AnyStream::External(_) => None,
        }
    }

    /// Name of the transport: `"BNET"`, `"BNET TLS-PSK"`, `"BNET TLS"` or
    /// the external stream's own name.
    pub fn transport(&self) -> &str {
        match self {
            AnyStream::Plain(_) => "BNET",
            AnyStream::External(s) => s.transport(),
            AnyStream::Psk(_) => "BNET TLS-PSK",
            AnyStream::RustlsServer(_) | AnyStream::RustlsClient(_) => "BNET TLS",
        }
    }

    /// The cipher name for "Encryption: …" lines: the TLS-PSK cipher, the
    /// rustls cipher suite, the external transport's name; empty for
    /// plaintext.
    pub fn cipher_name(&self) -> String {
        match self {
            AnyStream::Plain(_) => String::new(),
            AnyStream::External(s) => s.transport().to_string(),
            AnyStream::Psk(s) => s.cipher_name(),
            AnyStream::RustlsServer(s) => s.conn.negotiated_cipher_suite().map(|c| format!("{:?}", c.suite())).unwrap_or_default(),
            AnyStream::RustlsClient(s) => s.conn.negotiated_cipher_suite().map(|c| format!("{:?}", c.suite())).unwrap_or_default(),
        }
    }

    /// Whether this is an [`ExternalStream`] (the gRPC path).
    pub fn is_external(&self) -> bool {
        matches!(self, AnyStream::External(_))
    }
}

impl Read for AnyStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            AnyStream::Plain(s) => s.read(buf),
            AnyStream::External(s) => s.read(buf),
            AnyStream::Psk(s) => s.read(buf),
            AnyStream::RustlsServer(s) => s.read(buf),
            AnyStream::RustlsClient(s) => s.read(buf),
        }
    }
}

impl Write for AnyStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            AnyStream::Plain(s) => s.write(buf),
            AnyStream::External(s) => s.write(buf),
            AnyStream::Psk(s) => s.write(buf),
            AnyStream::RustlsServer(s) => s.write(buf),
            AnyStream::RustlsClient(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            AnyStream::Plain(s) => s.flush(),
            AnyStream::External(s) => s.flush(),
            AnyStream::Psk(s) => s.flush(),
            AnyStream::RustlsServer(s) => s.flush(),
            AnyStream::RustlsClient(s) => s.flush(),
        }
    }
}
