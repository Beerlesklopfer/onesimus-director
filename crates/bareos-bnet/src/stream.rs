//! Unified stream type that supports plaintext, TLS (rustls), and TLS-PSK (openssl).

use std::io::{self, Read, Write};
use std::net::TcpStream;

use crate::fd_grpc::ControlIo;
use crate::tls_psk::PskStream;

/// A unified stream that can be plaintext TCP, TLS (rustls), or TLS-PSK (openssl).
pub enum AnyStream {
    Plain(TcpStream),
    Grpc(ControlIo),
    Psk(PskStream),
    RustlsServer(rustls::StreamOwned<rustls::ServerConnection, TcpStream>),
    RustlsClient(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
}

impl Read for AnyStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            AnyStream::Plain(s) => s.read(buf),
            AnyStream::Grpc(s) => s.read(buf),
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
            AnyStream::Grpc(s) => s.write(buf),
            AnyStream::Psk(s) => s.write(buf),
            AnyStream::RustlsServer(s) => s.write(buf),
            AnyStream::RustlsClient(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            AnyStream::Plain(s) => s.flush(),
            AnyStream::Grpc(s) => s.flush(),
            AnyStream::Psk(s) => s.flush(),
            AnyStream::RustlsServer(s) => s.flush(),
            AnyStream::RustlsClient(s) => s.flush(),
        }
    }
}
