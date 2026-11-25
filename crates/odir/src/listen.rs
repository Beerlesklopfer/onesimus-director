// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Listen addresses of the Director: `--bind-address`, `DirAddresses`
//! (block form), `DirAddress`/`DirPort`, or every interface of both families.
//!
//! As Bareos 25 (lib/res.cc StoreAddresses*, lib/address_conf.cc AddAddress,
//! lib/bnet_server_tcp.cc):
//! - one socket per address; a name in a `DirAddresses` entry stands for
//!   all of its addresses of that family (duplicates dropped), `DirAddress`
//!   takes the first address a name resolves to;
//! - `ip` and `ipv4` entries are IPv4, `ipv6` entries IPv6; an entry without
//!   `addr` is the wildcard of its family, an entry without `port` takes 9101
//!   (the directive's default, not `DirPort`); one `addr` and one `port` per
//!   entry;
//! - `DirAddresses` cannot be mixed with `DirAddress` or `DirPort`, and an
//!   empty block is an error;
//! - without any address: 0.0.0.0, and `::` when the host has IPv6
//!   (InitDefaultAddresses);
//! - IPv6 sockets are IPv6-only, every socket has SO_REUSEADDR; a failed
//!   bind is tried three times, five seconds apart, then the start fails
//!   (OpenSocketAndBind, OpenAndBindSockets).

use std::net::{SocketAddr, TcpListener};
#[cfg(test)]
use std::net::{IpAddr, Ipv6Addr};

#[cfg(test)]
use bareos_config::listen::ipv6_available;
use bareos_config::raw::ConfResource;

/// The socket addresses the Director listens on, in configuration order,
/// without duplicates. `port` is `DirPort` (after `-P`). The rules are in
/// [`bareos_config::listen`] (shared with ofd).
pub fn listen_addresses(raw: &[ConfResource], cli_bind: Option<&str>, port: u16) -> Result<Vec<SocketAddr>, String> {
    bareos_config::listen::listen_addresses(raw, &bareos_config::listen::DIRECTOR, cli_bind, port)
}

/// Bind one listening socket. IPv6 sockets are IPv6-only, so `ipv4 = 0.0.0.0`
/// and `ipv6 = ::` on the same port do not collide.
pub fn bind(addr: SocketAddr) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    Ok(socket.into())
}

/// Bind every address; a failed bind is tried `tries` times, `pause`
/// apart (OpenSocketAndBind: 3 tries, 5 s), then the first failure ends it.
pub fn bind_all(addrs: &[SocketAddr]) -> Result<Vec<TcpListener>, String> {
    bind_all_with(addrs, 3, std::time::Duration::from_secs(5))
}

fn bind_all_with(addrs: &[SocketAddr], tries: u32, pause: std::time::Duration) -> Result<Vec<TcpListener>, String> {
    let mut out = Vec::new();
    for a in addrs {
        let mut attempt = 1;
        loop {
            match bind(*a) {
                Ok(l) => {
                    out.push(l);
                    break;
                }
                Err(e) if attempt < tries => {
                    tracing::warn!("Cannot bind address {a}: {e}. Retrying...");
                    attempt += 1;
                    std::thread::sleep(pause);
                }
                Err(e) => return Err(format!("Cannot bind address {a}: {e}")),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(director_body: &str) -> Vec<ConfResource> {
        bareos_config::raw::parse_str(&format!("Director {{\n Name = d\n{director_body}\n}}\n"), None).unwrap()
    }

    fn addrs(body: &str, cli: Option<&str>) -> Result<Vec<String>, String> {
        listen_addresses(&raw(body), cli, 9101).map(|v| v.iter().map(|a| a.to_string()).collect())
    }

    #[test]
    fn default_is_every_interface_of_both_families() {
        if !ipv6_available() {
            assert_eq!(addrs("", None).unwrap(), ["0.0.0.0:9101"]);
            return;
        }
        assert_eq!(addrs("", None).unwrap(), ["0.0.0.0:9101", "[::]:9101"]);
        assert_eq!(listen_addresses(&raw("DirPort = 9200"), None, 9200).unwrap()[1].to_string(), "[::]:9200");
    }

    /// The PDC block after the switch, verbatim (two entries per family,
    /// "::1" unquoted, a blank before ";").
    #[test]
    fn pdc_block_with_loopback() {
        let body = "  DirAddresses = {\n    ipv4 = { addr = 192.168.24.250; port = 9101; }\n    ipv4 = { addr = 127.0.0.1; port = 9101; }\n    \
            ipv6 = { addr = 2001:9e8:5e48:3c00::250; port = 9101; }\n    ipv6 = { addr = ::1 ; port = 9101; }\n  }";
        assert_eq!(addrs(body, None).unwrap(),
            ["192.168.24.250:9101", "127.0.0.1:9101", "[2001:9e8:5e48:3c00::250]:9101", "[::1]:9101"]);
    }

    /// bareos-dir rejects quoted addr/port in an entry; odir does too, so -t
    /// finds it before a way back to Bareos is needed.
    #[test]
    fn quoted_addr_or_port_is_refused_as_by_bareos() {
        let err = addrs("DirAddresses = {\n ipv6 = { addr = \"::1\" ; port = 9101; }\n}", None).unwrap_err();
        assert!(err.contains("Expected an IP number or a hostname"), "{err}");
        assert!(addrs("DirAddresses = {\n ipv4 = { addr = 127.0.0.1; port = \"9101\"; }\n}", None).is_err());
    }

    #[test]
    fn a_failed_bind_is_retried_then_fatal() {
        let taken = bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let addr = taken.local_addr().unwrap();
        // SO_REUSEADDR does not allow two listeners on one address
        let err = bind_all_with(&[addr], 2, std::time::Duration::from_millis(10)).unwrap_err();
        assert!(err.contains(&addr.to_string()), "{err}");
        // an address the host does not have is not skipped
        assert!(bind_all_with(&["192.0.2.1:0".parse().unwrap()], 1, std::time::Duration::ZERO).is_err());
    }

    #[test]
    fn dir_address_and_cli_override() {
        assert_eq!(addrs("DirAddress = 127.0.0.1", None).unwrap(), ["127.0.0.1:9101"]);
        assert_eq!(addrs("DirAddress = ::1", None).unwrap(), ["[::1]:9101"]);
        assert_eq!(addrs("DirAddress = 127.0.0.1", Some("[::1]")).unwrap(), ["[::1]:9101"]);
        assert!(addrs("DirAddress = no.such.host.invalid", None).is_err());
    }

    #[test]
    fn dir_addresses_binds_every_entry() {
        // The form of the PDC configuration
        let body = "DirAddresses = {\n ipv4 = { addr = 192.168.24.250; port = 9101; }\n ipv6 = { addr = 2001:db8::250; port = 9101; }\n}";
        assert_eq!(addrs(body, None).unwrap(), ["192.168.24.250:9101", "[2001:db8::250]:9101"]);
    }

    #[test]
    fn dir_addresses_defaults_and_errors() {
        let body = "DirAddresses = {\n ipv4 = { port = 9300 }\n ipv6 = {\n addr = ::1\n }\n ip = { addr = 127.0.0.1 }\n ipv4 = { addr = 127.0.0.1 }\n}";
        // "ip" is IPv4 (StoreAddresses), one addr and one port per entry
        assert!(addrs("DirAddresses = {\n ip = { addr = ::1 }\n}", None).is_err());
        assert!(addrs("DirAddresses = {\n ipv4 = { addr = 127.0.0.1; addr = 127.0.0.2 }\n}", None).unwrap_err().contains("Only one addr"));
        assert!(addrs("DirAddresses = {\n ipv4 = { port = 1; port = 2 }\n}", None).unwrap_err().contains("Only one port"));
        assert_eq!(addrs(body, None).unwrap(), ["0.0.0.0:9300", "[::1]:9101", "127.0.0.1:9101"]);
        // An entry without port takes 9101, not DirPort (-P)
        assert_eq!(listen_addresses(&raw("DirAddresses = {\n ip = { addr = 127.0.0.1 }\n}"), None, 9200).unwrap()[0].to_string(),
            "127.0.0.1:9101");
        // old and new style cannot be mixed; an empty block is an error
        assert!(addrs("DirPort = 9101\nDirAddresses = {\n ipv4 = { addr = 127.0.0.1 }\n}", None).unwrap_err().contains("cannot be mixed"));
        assert!(addrs("DirAddresses = {\n ipv4 = { addr = 127.0.0.1 }\n}\nAddress = 127.0.0.1", None).unwrap_err().contains("cannot be mixed"));
        assert!(addrs("DirAddresses = {\n}", None).is_err());
        // Aliases of the Director resource (K-18)
        assert_eq!(addrs("Address = 127.0.0.1", None).unwrap(), ["127.0.0.1:9101"]);
        assert_eq!(addrs("Addresses = {\n ipv4 = { addr = 127.0.0.1 }\n}", None).unwrap(), ["127.0.0.1:9101"]);
        assert!(addrs("DirAddresses = {\n ipv4 = { addr = ::1 }\n}", None).is_err());
        assert!(addrs("DirAddresses = {\n ipv4 = { port = x }\n}", None).is_err());
        assert!(addrs("DirAddresses = {\n tcp = { port = 1 }\n}", None).is_err());
        assert!(addrs("DirAddresses = {\n ipv4 = { addr = no.such.host.invalid }\n}", None).is_err());
    }

    #[test]
    fn wildcards_of_both_families_bind_together() {
        let v4 = bind("0.0.0.0:0".parse().unwrap()).unwrap();
        let port = v4.local_addr().unwrap().port();
        // Without IPV6_V6ONLY this would fail with "address in use".
        match bind(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)) {
            Ok(_) => {}
            // hosts without IPv6
            Err(e) if e.raw_os_error() == Some(libc::EAFNOSUPPORT) || e.raw_os_error() == Some(libc::EADDRNOTAVAIL) => {}
            Err(e) => panic!("{e}"),
        }
    }
}
