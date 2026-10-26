//! SD connection: TLS-PSK + Hello + CRAM-MD5.

use std::net::TcpStream;
use std::time::Duration;

use tracing::debug;

use bareos_bnet::{BnetStream, CramMd5Auth};
use bareos_bnet::hello::{make_director_hello, SD_OK_HELLO};
use bareos_bnet::tls_psk::PskStream;

/// Connect to a Storage Daemon with TLS-PSK + CRAM-MD5.
///
/// Returns authenticated BnetStream ready for commands.
pub fn connect_sd(
    address: &str,
    port: u16,
    password: &str,
    director_name: &str,
) -> Result<BnetStream<PskStream>, String> {
    let addr = format!("{address}:{port}");

    // TCP connect
    let sock_addr = addr.parse()
        .or_else(|_| {
            use std::net::ToSocketAddrs;
            addr.to_socket_addrs()
                .map_err(|e| format!("DNS: {e}"))?
                .next()
                .ok_or_else(|| "no address".to_string())
        })
        .map_err(|e| format!("connect {addr}: {e}"))?;

    let stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(5))
        .map_err(|e| format!("connect {addr}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(60))).ok();

    // TLS-PSK
    let psk_identity = format!("R_DIRECTOR\x1e{director_name}");
    let psk = psk::password_to_psk(password);
    let psk_stream = psk::psk_connect(stream, &psk_identity, &psk)
        .map_err(|e| format!("TLS-PSK {addr}: {e}"))?;

    let mut bnet = BnetStream::new(psk_stream);

    // Hello
    let hello = make_director_hello(director_name);
    bnet.send_str(&hello).map_err(|e| format!("hello: {e}"))?;

    // CRAM-MD5
    let auth = CramMd5Auth::new(password, director_name, 2);
    let result = auth.two_way_authenticate(&mut bnet, false)
        .map_err(|e| format!("CRAM-MD5: {e}"))?;
    if !result.success {
        return Err("CRAM-MD5 auth failed".into());
    }

    // "3000 OK Hello"
    let resp = bnet.recv_str().map_err(|e| format!("hello resp: {e}"))?;
    if !resp.starts_with(SD_OK_HELLO) {
        return Err(format!("SD hello: {resp}"));
    }

    debug!("SD connected: {addr}");
    Ok(bnet)
}

// Re-export for convenience
use bareos_bnet::tls_psk as psk;
