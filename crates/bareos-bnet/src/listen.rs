//! Listening sockets of the daemons: one socket for IPv4 and IPv6.
//!
//! Zweck: `[::]:port` soll IPv4 und IPv6 annehmen ("dual-stack on every
//! interface", ofd/config.rs). Linux tut das von selbst (`bindv6only=0`),
//! Windows nicht: dort ist ein Socket auf `[::]` nur IPv6, und jeder
//! IPv4-Client wird abgewiesen (gemessen auf Windows 11: `127.0.0.1:19102`
//! und die Adresse der VM verweigern, `[::]` lauscht). [`bind`] setzt
//! `IPV6_V6ONLY` für die unspezifizierte IPv6-Adresse ausdrücklich zurück.
//!
//! Bareos-Gegenstück: `lib/bnet_network.cc` (`BnetThreadServerTcp` bindet je
//! Adresse einen eigenen Socket, IPv4 und IPv6 getrennt).

use std::io;
use std::net::{SocketAddr, TcpListener};

use socket2::{Domain, Protocol, Socket, Type};

/// Backlog of the listening socket (as the standard library's `bind`).
const BACKLOG: i32 = 128;

/// Binds `addr` like `TcpListener::bind`; an unspecified IPv6 address
/// (`[::]`) accepts IPv4 connections too, on every platform.
pub fn bind(addr: SocketAddr) -> io::Result<TcpListener> {
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() && addr.ip().is_unspecified() {
        socket.set_only_v6(false)?;
    }
    // the standard library sets SO_REUSEADDR on Unix (not on Windows, where
    // it would allow a second listener on the same port)
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(BACKLOG)?;
    Ok(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    /// `[::]` takes an IPv4 connection (skipped where the host has no IPv6).
    #[test]
    fn a_wildcard_v6_socket_accepts_ipv4() {
        let Ok(listener) = bind("[::]:0".parse().unwrap()) else { return };
        let port = listener.local_addr().unwrap().port();
        let client = TcpStream::connect(("127.0.0.1", port));
        assert!(client.is_ok(), "IPv4 connect to [::]:{port}: {client:?}");
        assert!(listener.accept().is_ok());
    }

    /// An IPv4 address stays IPv4.
    #[test]
    fn an_ipv4_address_binds() {
        let listener = bind("127.0.0.1:0".parse().unwrap()).unwrap();
        assert!(listener.local_addr().unwrap().is_ipv4());
    }
}
