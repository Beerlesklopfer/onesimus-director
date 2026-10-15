//! File Daemon protocol handler.
//!
//! Bareos 18.2+ Director→FD connection sequence:
//! 1. TCP connect
//! 2. TLS-PSK handshake (BEFORE any BNET messages!)
//! 3. Hello: "Hello Director <name> calling\n" (encrypted)
//! 4. CRAM-MD5 Two-Way (encrypted)
//! 5. FD response: "2000 OK Hello [version]"
//! 6. Commands on encrypted channel

use std::net::TcpStream;
use std::time::Duration;

use tracing::{debug, warn};

use bareos_bnet::{BnetStream, BnetSignal, BnetMessage, BnetError, CramMd5Auth};
use bareos_bnet::hello::{make_director_hello, FD_OK_HELLO};
use bareos_bnet::tls_psk;

/// Result of an FD status query.
#[derive(Debug)]
pub struct FdStatusResult {
    pub client_name: String,
    pub version: String,
    pub status_text: String,
    pub success: bool,
}

/// Connect to a File Daemon and get its status.
pub fn query_fd_status(
    address: &str,
    port: u16,
    password: &str,
    director_name: &str,
    client_name: &str,
) -> FdStatusResult {
    let addr = format!("{address}:{port}");

    // 1. TCP Connect
    let stream = match TcpStream::connect_timeout(
        &{
            use std::net::ToSocketAddrs;
            addr.to_socket_addrs().ok().and_then(|mut a| a.next())
                .unwrap_or_else(|| format!("127.0.0.1:{port}").parse().unwrap())
        },
        Duration::from_secs(3),
    ) {
        Ok(s) => s,
        Err(e) => return fail(client_name, &format!("Connect to {addr} failed: {e}")),
    };
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

    // 2. TLS-PSK handshake FIRST (Bareos 18.2+ requirement)
    // Identity format: "R_DIRECTOR\x1e<name>" with Record Separator
    let psk_identity = format!("R_DIRECTOR\x1e{director_name}");
    let psk = tls_psk::password_to_psk(password);
    let psk_result = tls_psk::psk_connect(stream, &psk_identity, &psk);

    // Branch based on TLS result — different stream types
    match psk_result {
        Ok(psk_stream) => {
            debug!("FD {client_name}: TLS-PSK handshake OK");
            return do_fd_status(BnetStream::new(psk_stream), password, director_name, client_name);
        }
        Err(e) => {
            debug!("FD {client_name}: TLS-PSK failed ({e}), trying cleartext");
            use std::net::ToSocketAddrs;
            let sock_addr = match addr.to_socket_addrs().ok().and_then(|mut a| a.next()) {
                Some(a) => a,
                None => return fail(client_name, &format!("DNS resolve {addr} failed")),
            };
            let stream2 = match TcpStream::connect_timeout(&sock_addr, Duration::from_secs(3)) {
                Ok(s) => s,
                Err(e2) => return fail(client_name, &format!("Reconnect failed: {e2}")),
            };
            stream2.set_read_timeout(Some(Duration::from_secs(30))).ok();
            return do_fd_status(BnetStream::new(stream2), password, director_name, client_name);
        }
    }

}

/// Common FD status flow after stream is established (TLS or plaintext).
fn do_fd_status<S: std::io::Read + std::io::Write>(
    mut bnet: BnetStream<S>,
    password: &str,
    director_name: &str,
    client_name: &str,
) -> FdStatusResult {
    // 3. Send Hello
    let hello = make_director_hello(director_name);
    if let Err(e) = bnet.send_str(&hello) {
        return fail(client_name, &format!("Hello send failed: {e}"));
    }

    // 4. CRAM-MD5 Two-Way (Director is client = initiated_by_remote=false)
    let auth = CramMd5Auth::new(password, director_name, 2);
    match auth.two_way_authenticate(&mut bnet, false) {
        Ok(r) if r.success => debug!("FD {client_name}: CRAM-MD5 auth OK"),
        Ok(_) => return fail(client_name, "CRAM-MD5 authentication failed"),
        Err(e) => return fail(client_name, &format!("CRAM-MD5 error: {e}")),
    };

    // 5. Receive FD hello response: "2000 OK Hello [version]"
    let fd_hello = match bnet.recv_str() {
        Ok(msg) => msg,
        Err(e) => return fail(client_name, &format!("FD hello recv failed: {e}")),
    };

    let version = if fd_hello.starts_with(FD_OK_HELLO) {
        fd_hello.trim_start_matches(FD_OK_HELLO).trim().to_string()
    } else {
        debug!("FD {client_name}: unexpected response: {fd_hello}");
        String::new()
    };

    // 6. Send "status" command
    if let Err(e) = bnet.send_str("status") {
        return fail(client_name, &format!("Status send failed: {e}"));
    }

    let mut status_text = String::new();
    loop {
        match bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                status_text.push_str(&String::from_utf8_lossy(&data));
            }
            Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
            Ok(BnetMessage::Signal(_)) => continue,
            Err(_) => break,
        }
    }

    let _ = bnet.send_signal(BnetSignal::Terminate);

    FdStatusResult {
        client_name: client_name.to_string(),
        version,
        status_text,
        success: true,
    }
}

fn fail(client_name: &str, msg: &str) -> FdStatusResult {
    FdStatusResult {
        client_name: client_name.to_string(),
        version: String::new(),
        status_text: msg.to_string(),
        success: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fd_status_connect_refused() {
        let result = query_fd_status("127.0.0.1", 19999, "pass", "dir", "test-fd");
        assert!(!result.success);
    }
}

/// Execute a single-response FD control command over TLS-PSK and CRAM-MD5.
/// The builder receives the negotiated protocol version (53 adds timestamp).
/// Existing status/backup connection paths are unaffected.
pub fn fd_command(
    address: &str, port: u16, password: &str, director_name: &str,
    command: impl FnOnce(u32) -> String,
) -> Result<String, String> {
    use std::net::ToSocketAddrs;
    let socket = (address, port).to_socket_addrs().map_err(|e| format!("DNS: {e}"))?
        .next().ok_or("no FD address")?;
    let stream = TcpStream::connect_timeout(&socket, Duration::from_secs(5))
        .map_err(|e| format!("connect: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
    let stream = tls_psk::psk_connect(stream, &format!("R_DIRECTOR\x1e{director_name}"),
        &tls_psk::password_to_psk(password)).map_err(|e| format!("TLS-PSK: {e}"))?;
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(director_name)).map_err(|e| e.to_string())?;
    let auth = CramMd5Auth::new(password, director_name, 2);
    if !auth.two_way_authenticate(&mut bnet, false).map_err(|e| e.to_string())?.success {
        return Err("FD authentication failed".into());
    }
    let hello = bnet.recv_str().map_err(|e| format!("FD hello: {e}"))?;
    let hello = hello.strip_prefix(FD_OK_HELLO).ok_or("invalid FD hello")?;
    let version = hello.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
    bnet.send_str(&command(version)).map_err(|e| e.to_string())?;
    let result = read_control_reply(&mut bnet);
    let _ = bnet.send_signal(BnetSignal::Terminate);
    result
}

fn read_control_reply<S: std::io::Read + std::io::Write>(bnet: &mut BnetStream<S>) -> Result<String, String> {
    loop {
        match bnet.recv().map_err(|e| format!("FD response: {e}"))? {
            BnetMessage::Data(bytes) => return String::from_utf8(bytes).map_err(|e| e.to_string()),
            BnetMessage::Signal(BnetSignal::Heartbeat) => {
                bnet.send_signal(BnetSignal::HeartbeatResponse).map_err(|e| e.to_string())?;
            }
            BnetMessage::Signal(BnetSignal::Eod | BnetSignal::Terminate) => return Err("FD ended command without a response".into()),
            BnetMessage::Signal(_) => {},
        }
    }
}

#[cfg(test)]
mod control_reply_tests {
    use super::*;
    use std::io::{Cursor, Read, Write};
    struct Stream { input: Cursor<Vec<u8>>, output: Vec<u8> }
    impl Read for Stream { fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> { self.input.read(b) } }
    impl Write for Stream {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> { self.output.extend_from_slice(b); Ok(b.len()) }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    #[test]
    fn control_reply_preserves_wire_bytes_and_rejects_premature_end() {
        let response = b"2000 OK setdebug\r\n";
        let mut bytes = (response.len() as i32).to_be_bytes().to_vec();
        bytes.extend_from_slice(response);
        let mut bnet = BnetStream::new(Stream { input: Cursor::new(bytes), output: vec![] });
        assert_eq!(read_control_reply(&mut bnet).unwrap().as_bytes(), response);
        let mut bnet = BnetStream::new(Stream { input: Cursor::new((-1i32).to_be_bytes().to_vec()), output: vec![] });
        assert!(read_control_reply(&mut bnet).is_err());
    }
}
