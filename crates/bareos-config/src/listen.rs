// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Listen addresses of a daemon: the block form (`DirAddresses`, `FDAddresses`), the single
//! address with its port (`DirAddress`/`DirPort`, `FDAddress`/`FDPort`), or every interface of
//! both families. Shared by odir and ofd (moved from odir `listen.rs`, 25.11.2026).
//!
//! As Bareos 25 (lib/res.cc StoreAddresses*, lib/address_conf.cc AddAddress,
//! lib/bnet_server_tcp.cc):
//! - one socket per address; a name in a block entry stands for all of its addresses of that
//!   family (duplicates dropped), the single address takes the first address a name resolves to;
//! - `ip` and `ipv4` entries are IPv4, `ipv6` entries IPv6; an entry without `addr` is the
//!   wildcard of its family, an entry without `port` takes the item's default (9101, 9102), not
//!   the port directive; one `addr` and one `port` per entry;
//! - the block cannot be mixed with the single address or the port, and an empty block is an
//!   error;
//! - without any address: 0.0.0.0, and `::` when the host has IPv6 (InitDefaultAddresses).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

use crate::raw::{ConfResource, Directive, Tok};

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
pub fn ipv6_available() -> bool {
    std::net::TcpListener::bind((Ipv6Addr::UNSPECIFIED, 0)).is_ok()
}

/// The directives of one daemon's listen addresses.
#[derive(Debug, Clone, Copy)]
pub struct AddressDirectives {
    /// Resource types that carry them (normalized, e.g. `director`).
    pub resources: &'static [&'static str],
    /// Keys of the block form (`diraddresses`, aliases).
    pub block: &'static [&'static str],
    /// Keys of the single address (`diraddress`, aliases).
    pub single: &'static [&'static str],
    /// Keys of the port (`dirport`, aliases).
    pub port: &'static [&'static str],
    /// Port of a block entry without `port` (the item's default, not the port directive).
    pub default_port: u16,
    /// Name of the block directive in messages (`DirAddresses`).
    pub block_name: &'static str,
    /// Name of the single directive in messages (`DirAddress`).
    pub single_name: &'static str,
}

/// The Director resource (`DirAddresses`, `DirAddress`, `DirPort`, aliases K-18).
pub const DIRECTOR: AddressDirectives = AddressDirectives {
    resources: &["director"],
    block: &["diraddresses", "addresses"],
    single: &["diraddress", "address"],
    port: &["dirport", "port"],
    default_port: 9101,
    block_name: "DirAddresses",
    single_name: "DirAddress",
};

/// The Client (FileDaemon) resource of bareos-fd (`FdAddresses`, `FdAddress`, `FdPort`).
pub const FILE_DAEMON: AddressDirectives = AddressDirectives {
    resources: &["client", "filedaemon"],
    block: &["fdaddresses"],
    single: &["fdaddress"],
    port: &["fdport"],
    default_port: 9102,
    block_name: "FDAddresses",
    single_name: "FDAddress",
};

/// The socket addresses a daemon listens on, in configuration order, without duplicates. `port`
/// is the port directive's value (after `-P`); `cli_bind` an address given on the command line.
pub fn listen_addresses(raw: &[ConfResource], spec: &AddressDirectives, cli_bind: Option<&str>, port: u16)
    -> Result<Vec<SocketAddr>, String> {
    if let Some(host) = cli_bind {
        return Ok(vec![first_address(host, port, "--bind-address")?]);
    }
    let resource = raw.iter().find(|r| spec.resources.contains(&r.norm_type().as_str()));
    let last = |keys: &[&str]| resource.and_then(|r| r.directives.iter().rev().find(|d| keys.contains(&d.norm_key().as_str())));
    let old_keys: Vec<&str> = spec.single.iter().chain(spec.port.iter()).copied().collect();
    let name = spec.block_name;

    if let Some(block) = last(spec.block) {
        if let Some(old) = last(&old_keys) {
            return Err(format!("{name}: the old style addresses cannot be mixed with new style ({})", old.key));
        }
        let mut out = Vec::new();
        let entries = block.tokens.iter().filter_map(|t| match t { Tok::Block(e) => Some(e), _ => None }).flatten();
        let mut any = false;
        for entry in entries {
            any = true;
            for addr in block_entry(entry, spec.default_port, name)? {
                if !out.contains(&addr) {
                    out.push(addr);
                }
            }
        }
        if !any {
            return Err(format!("{name}: Empty addr block is not allowed"));
        }
        return Ok(out);
    }
    if let Some(host) = last(spec.single).and_then(|d| d.first()) {
        return Ok(vec![first_address(host, port, spec.single_name)?]);
    }
    let mut out = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port)];
    if ipv6_available() {
        out.push(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port));
    }
    Ok(out)
}

/// One `ip|ipv4|ipv6 = { addr = …; port = … }` entry.
fn block_entry(entry: &Directive, default_port: u16, name: &str) -> Result<Vec<SocketAddr>, String> {
    let family = match entry.norm_key().as_str() {
        // StoreAddresses: "ip" and "ipv4" are both AF_INET
        "ip" | "ipv4" => Family::V4,
        "ipv6" => Family::V6,
        other => return Err(format!("{name}: expected ip, ipv4 or ipv6, got \"{}\" ({other})", entry.key)),
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
                return Err(format!("{name}: Expected an IP number or a hostname, got: \"{v}\" (write it without quotes)")),
            Some(Tok::Quoted(v)) if d.norm_key() == "port" =>
                return Err(format!("{name}: Expected a number or a string, got: \"{v}\" (write it without quotes)")),
            _ => return Err(format!("{name}: {} without a value", d.key)),
        };
        match d.norm_key().as_str() {
            "addr" if addr.is_some() => return Err(format!("{name}: Only one addr per address block")),
            "port" if port.is_some() => return Err(format!("{name}: Only one port per address block")),
            "addr" => addr = Some(value.to_string()),
            "port" => port = Some(value.parse().map_err(|_| format!("{name}: invalid port \"{value}\""))?),
            _ => return Err(format!("{name}: expected addr or port, got \"{}\"", d.key)),
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
        .map_err(|e| format!("{name} {host}: {e}"))?
        .filter(|a| family.admits(&a.ip()))
        .collect();
    if found.is_empty() {
        return Err(format!("{name} {host}: no address of the requested family"));
    }
    Ok(found)
}

pub fn first_address(host: &str, port: u16, what: &str) -> Result<SocketAddr, String> {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (host, port).to_socket_addrs().map_err(|e| format!("{what} {host}: {e}"))?
        .next().ok_or_else(|| format!("{what} {host}: no address"))
}

