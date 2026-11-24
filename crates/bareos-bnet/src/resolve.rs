// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Host name resolution as the daemons do it for `resolve` (C++ lib/bnet.cc `BnetHost2IpAddrs`,
//! lib/address_conf.cc `IPADDR::build_address_str`). Used by odir (local `resolve`) and ofd
//! (`resolve` of the Director).

use std::net::IpAddr;

#[cfg(unix)]
mod unix {
    use std::net::IpAddr;

    // glibc address conversion functions (not exported by the libc crate).
    unsafe extern "C" {
        fn inet_aton(cp: *const libc::c_char, inp: *mut libc::in_addr) -> libc::c_int;
        fn inet_pton(af: libc::c_int, src: *const libc::c_char, dst: *mut libc::c_void) -> libc::c_int;
        fn inet_ntop(af: libc::c_int, src: *const libc::c_void, dst: *mut libc::c_char, size: libc::socklen_t) -> *const libc::c_char;
    }

    /// C++ BnetHost2IpAddrs(host, 0, &errstr) (lib/bnet.cc:293): numeric IPv4
    /// (inet_aton) or IPv6 (inet_pton) literals directly, otherwise getaddrinfo
    /// for AF_INET6 and then AF_INET, keeping order and duplicates. The error is
    /// the gai_strerror text of the AF_INET lookup.
    pub fn host_to_ip_addrs(host: &str) -> Result<Vec<IpAddr>, String> {
        let Ok(c_host) = std::ffi::CString::new(host) else { return Err("invalid hostname".into()) };
        let mut ipv4: libc::in_addr = unsafe { std::mem::zeroed() };
        if unsafe { inet_aton(c_host.as_ptr(), &mut ipv4) } != 0 {
            return Ok(vec![IpAddr::V4(u32::from_be(ipv4.s_addr).into())]);
        }
        let mut ipv6: libc::in6_addr = unsafe { std::mem::zeroed() };
        if unsafe { inet_pton(libc::AF_INET6, c_host.as_ptr(), (&mut ipv6 as *mut libc::in6_addr).cast()) } == 1 {
            return Ok(vec![IpAddr::V6(ipv6.s6_addr.into())]);
        }
        let mut addresses = vec![];
        let _ = resolv_host(libc::AF_INET6, &c_host, &mut addresses);
        let error = resolv_host(libc::AF_INET, &c_host, &mut addresses);
        match error {
            Err(error) if addresses.is_empty() => Err(error),
            _ => Ok(addresses),
        }
    }

    /// C++ resolv_host (lib/bnet.cc:235): getaddrinfo with SOCK_STREAM/IPPROTO_TCP.
    fn resolv_host(family: libc::c_int, host: &std::ffi::CStr, addresses: &mut Vec<IpAddr>) -> Result<(), String> {
        let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
        hints.ai_family = family;
        hints.ai_socktype = libc::SOCK_STREAM;
        hints.ai_protocol = libc::IPPROTO_TCP;
        let mut list: *mut libc::addrinfo = std::ptr::null_mut();
        let result = unsafe { libc::getaddrinfo(host.as_ptr(), std::ptr::null(), &hints, &mut list) };
        if result != 0 {
            return Err(unsafe { std::ffi::CStr::from_ptr(libc::gai_strerror(result)) }.to_string_lossy().into_owned());
        }
        let mut entry = list;
        while let Some(info) = unsafe { entry.as_ref() } {
            match info.ai_family {
                libc::AF_INET => {
                    let address = unsafe { &*(info.ai_addr as *const libc::sockaddr_in) };
                    addresses.push(IpAddr::V4(u32::from_be(address.sin_addr.s_addr).into()));
                }
                libc::AF_INET6 => {
                    let address = unsafe { &*(info.ai_addr as *const libc::sockaddr_in6) };
                    addresses.push(IpAddr::V6(address.sin6_addr.s6_addr.into()));
                }
                _ => {}
            }
            entry = info.ai_next;
        }
        unsafe { libc::freeaddrinfo(list) };
        Ok(())
    }

    /// IPADDR::build_address_str without port (lib/address_conf.cc), via inet_ntop.
    pub fn address_string(address: &IpAddr) -> String {
        let mut buffer = [0 as libc::c_char; 64];
        let (kind, text) = match address {
            IpAddr::V4(v4) => {
                let raw = libc::in_addr { s_addr: u32::from(*v4).to_be() };
                ("ipv4", unsafe { inet_ntop(libc::AF_INET, (&raw as *const libc::in_addr).cast(), buffer.as_mut_ptr(), 64) })
            }
            IpAddr::V6(v6) => {
                let raw = libc::in6_addr { s6_addr: v6.octets() };
                ("ipv6", unsafe { inet_ntop(libc::AF_INET6, (&raw as *const libc::in6_addr).cast(), buffer.as_mut_ptr(), 64) })
            }
        };
        let text = if text.is_null() { address.to_string() }
            else { unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy().into_owned() };
        format!("host[{kind};{text}] ")
    }
}

#[cfg(unix)]
pub use unix::{address_string, host_to_ip_addrs};

/// BnetHost2IpAddrs() without glibc: the addresses of `host` from the system resolver.
#[cfg(not(unix))]
pub fn host_to_ip_addrs(host: &str) -> Result<Vec<IpAddr>, String> {
    use std::net::ToSocketAddrs;
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(vec![address]);
    }
    let addresses: Vec<IpAddr> = (host, 0).to_socket_addrs().map_err(|e| e.to_string())?.map(|a| a.ip()).collect();
    if addresses.is_empty() { Err("no address".into()) } else { Ok(addresses) }
}

/// `host[ipv4;a.b.c.d] ` / `host[ipv6;…] ` of one address.
#[cfg(not(unix))]
pub fn address_string(address: &IpAddr) -> String {
    let kind = if address.is_ipv4() { "ipv4" } else { "ipv6" };
    format!("host[{kind};{address}] ")
}

/// BuildAddressesString(): the addresses one after the other.
pub fn addresses_string(addresses: &[IpAddr]) -> String {
    addresses.iter().map(address_string).collect()
}
