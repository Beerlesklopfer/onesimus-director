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

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, ToSocketAddrs};

use bareos_config::raw::{ConfResource, Directive, Tok};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    V4,
    V6,
}

impl Family {
    fn admits(self, ip: &IpAddr) -> bool {
        match self {
            Family::V4 => ip.is_ipv4(),
            Family::V6 => ip.is_ipv6(),
        }
    }
}

/// IsFamilyEnabled(IpFamily::V6): the host can open an IPv6 socket.
fn ipv6_available() -> bool {
    socket2::Socket::new(socket2::Domain::IPV6, socket2::Type::STREAM, None).is_ok()
}

/// Port of a `DirAddresses` entry without `port` (default value of the item).
const ADDRESSES_DEFAULT_PORT: u16 = 9101;

/// The socket addresses the Director listens on, in configuration order,
/// without duplicates. `port` is `DirPort` (after `-P`).
pub fn listen_addresses(raw: &[ConfResource], cli_bind: Option<&str>, port: u16) -> Result<Vec<SocketAddr>, String> {
    if let Some(host) = cli_bind {
        return Ok(vec![first_address(host, port, "--bind-address")?]);
    }
    let director = raw.iter().find(|r| r.norm_type() == "director");
    let last = |keys: &[&str]| director.and_then(|r| r.directives.iter().rev().find(|d| keys.contains(&d.norm_key().as_str())));

    if let Some(block) = last(&["diraddresses", "addresses"]) {
        if let Some(old) = last(&["diraddress", "address", "dirport", "port"]) {
            return Err(format!("DirAddresses: the old style addresses cannot be mixed with new style ({})", old.key));
        }
        let mut out = Vec::new();
        let entries = block.tokens.iter().filter_map(|t| match t { Tok::Block(e) => Some(e), _ => None }).flatten();
        let mut any = false;
        for entry in entries {
            any = true;
            for addr in block_entry(entry, ADDRESSES_DEFAULT_PORT)? {
                if !out.contains(&addr) {
                    out.push(addr);
                }
            }
        }
        if !any {
            return Err("DirAddresses: Empty addr block is not allowed".into());
        }
        return Ok(out);
    }
    if let Some(host) = last(&["diraddress", "address"]).and_then(|d| d.first()) {
        return Ok(vec![first_address(host, port, "DirAddress")?]);
    }
    let mut out = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)];
    if ipv6_available() {
        out.push(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port));
    }
    Ok(out)
}

/// One `ip|ipv4|ipv6 = { addr = …; port = … }` entry.
fn block_entry(entry: &Directive, default_port: u16) -> Result<Vec<SocketAddr>, String> {
    let family = match entry.norm_key().as_str() {
        // StoreAddresses: "ip" and "ipv4" are both AF_INET
        "ip" | "ipv4" => Family::V4,
        "ipv6" => Family::V6,
        other => return Err(format!("DirAddresses: expected ip, ipv4 or ipv6, got \"{}\" ({other})", entry.key)),
    };
    let inner: &[Directive] = match entry.tokens.iter().find_map(|t| match t { Tok::Block(b) => Some(b), _ => None }) {
        Some(b) => b,
        None => entry.block.as_deref().unwrap_or(&[]),
    };
    let mut addr = None;
    let mut port = None;
    for d in inner {
        // StoreAddresses takes addr and port only unquoted (BCT_UNQUOTED_STRING,
        // BCT_IDENTIFIER, BCT_NUMBER): bareos-dir rejects addr = "::1".
        let value = match d.tokens.iter().find(|t| !matches!(t, Tok::Eq)) {
            Some(Tok::Word(v)) => v.as_str(),
            Some(Tok::Quoted(v)) if d.norm_key() == "addr" =>
                return Err(format!("DirAddresses: Expected an IP number or a hostname, got: \"{v}\" (write it without quotes)")),
            Some(Tok::Quoted(v)) if d.norm_key() == "port" =>
                return Err(format!("DirAddresses: Expected a number or a string, got: \"{v}\" (write it without quotes)")),
            _ => return Err(format!("DirAddresses: {} without a value", d.key)),
        };
        match d.norm_key().as_str() {
            "addr" if addr.is_some() => return Err("DirAddresses: Only one addr per address block".into()),
            "port" if port.is_some() => return Err("DirAddresses: Only one port per address block".into()),
            "addr" => addr = Some(value.to_string()),
            "port" => port = Some(value.parse().map_err(|_| format!("DirAddresses: invalid port \"{value}\""))?),
            _ => return Err(format!("DirAddresses: expected addr or port, got \"{}\"", d.key)),
        }
    }
    let port = port.unwrap_or(default_port);
    let Some(host) = addr else {
        let ip = match family {
            Family::V6 => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            _ => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        return Ok(vec![SocketAddr::new(ip, port)]);
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let found: Vec<SocketAddr> = (host, port).to_socket_addrs()
        .map_err(|e| format!("DirAddresses {host}: {e}"))?
        .filter(|a| family.admits(&a.ip()))
        .collect();
    if found.is_empty() {
        return Err(format!("DirAddresses {host}: no address of the requested family"));
    }
    Ok(found)
}

fn first_address(host: &str, port: u16, what: &str) -> Result<SocketAddr, String> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (host, port).to_socket_addrs().map_err(|e| format!("{what} {host}: {e}"))?
        .next().ok_or_else(|| format!("{what} {host}: no address"))
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
