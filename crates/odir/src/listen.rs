//! Listen addresses of the Director: `--bind-address`, `DirAddresses`
//! (block form), `DirAddress`/`DirPort`, or every interface of both families.
//!
//! As Bareos 25 (lib/res.cc StoreAddresses*, lib/address_conf.cc AddAddress,
//! lib/bnet_server_tcp.cc):
//! - one socket per address; a name in a `DirAddresses` entry stands for
//!   all of its addresses of that family (duplicates dropped), `DirAddress`
//!   takes the first address a name resolves to;
//! - an entry without `addr` is the wildcard of its family, an entry without
//!   `port` takes 9101 (the directive's default, not `DirPort`);
//! - `DirAddresses` cannot be mixed with `DirAddress` or `DirPort`, and an
//!   empty block is an error;
//! - without any address: 0.0.0.0 and `::` (InitDefaultAddresses);
//! - IPv6 sockets are IPv6-only, every socket has SO_REUSEADDR; a socket the
//!   host cannot open (no IPv6) is skipped with a warning while another one
//!   is bound.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, ToSocketAddrs};

use bareos_config::raw::{ConfResource, Directive, Tok};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Any,
    V4,
    V6,
}

impl Family {
    fn admits(self, ip: &IpAddr) -> bool {
        match self {
            Family::Any => true,
            Family::V4 => ip.is_ipv4(),
            Family::V6 => ip.is_ipv6(),
        }
    }
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
    Ok(vec![
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port),
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port),
    ])
}

/// One `ip|ipv4|ipv6 = { addr = …; port = … }` entry.
fn block_entry(entry: &Directive, default_port: u16) -> Result<Vec<SocketAddr>, String> {
    let family = match entry.norm_key().as_str() {
        "ip" => Family::Any,
        "ipv4" => Family::V4,
        "ipv6" => Family::V6,
        other => return Err(format!("DirAddresses: expected ip, ipv4 or ipv6, got \"{}\" ({other})", entry.key)),
    };
    let inner: &[Directive] = match entry.tokens.iter().find_map(|t| match t { Tok::Block(b) => Some(b), _ => None }) {
        Some(b) => b,
        None => entry.block.as_deref().unwrap_or(&[]),
    };
    let mut addr = None;
    let mut port = default_port;
    for d in inner {
        let value = d.first().ok_or_else(|| format!("DirAddresses: {} without a value", d.key))?;
        match d.norm_key().as_str() {
            "addr" => addr = Some(value.to_string()),
            "port" => port = value.parse().map_err(|_| format!("DirAddresses: invalid port \"{value}\""))?,
            _ => return Err(format!("DirAddresses: expected addr or port, got \"{}\"", d.key)),
        }
    }
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

/// The host has no such address family or address (e.g. IPv6 switched off).
fn unavailable(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(libc::EAFNOSUPPORT) | Some(libc::EADDRNOTAVAIL))
}

/// Bind every address. An address the host cannot provide is skipped with a
/// warning while another one is bound; any other failure names the address.
pub fn bind_all(addrs: &[SocketAddr]) -> Result<Vec<TcpListener>, String> {
    let mut out = Vec::new();
    let mut skipped = Vec::new();
    for a in addrs {
        match bind(*a) {
            Ok(l) => out.push(l),
            Err(e) if unavailable(&e) => skipped.push(format!("{a}: {e}")),
            Err(e) => return Err(format!("failed to bind {a}: {e}")),
        }
    }
    if out.is_empty() {
        return Err(format!("failed to bind any address ({})", skipped.join("; ")));
    }
    for s in skipped {
        tracing::warn!("cannot listen on {s}");
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
        assert_eq!(addrs("", None).unwrap(), ["0.0.0.0:9101", "[::]:9101"]);
        assert_eq!(listen_addresses(&raw("DirPort = 9200"), None, 9200).unwrap()[1].to_string(), "[::]:9200");
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
