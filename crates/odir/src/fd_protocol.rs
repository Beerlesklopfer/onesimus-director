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

use tracing::debug;

use bareos_bnet::{BnetStream, BnetSignal, BnetMessage, CramMd5Auth};
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
    if let Some((mut fd, _)) = crate::fd_grpc::try_connect(address, port, password, director_name, Some(Duration::from_secs(30))) {
        if let Err(error) = fd.send_str("status") { return fail(client_name, &error.to_string()); }
        let output = read_listing_reply(&mut fd);
        let _ = fd.send_signal(BnetSignal::Terminate);
        return match output {
            Ok(status_text) => FdStatusResult { client_name: client_name.into(), version: "54".into(), status_text, success: true },
            Err(error) => fail(client_name, &error),
        };
    }
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
    let (mut bnet, version) = connect_control_fd(address, port, password, director_name)?;
    bnet.send_str(&command(version)).map_err(|e| e.to_string())?;
    let result = read_control_reply(&mut bnet);
    let _ = bnet.send_signal(BnetSignal::Terminate);
    result
}

fn connect_control_fd(address: &str, port: u16, password: &str, director_name: &str)
    -> Result<(BnetStream<bareos_bnet::stream::AnyStream>, u32), String> {
    connect_fd_with_timeout(address, port, password, director_name, Some(Duration::from_secs(15)))
}

/// Authenticated Director→FD connection (TLS-PSK, Hello, CRAM-MD5) with the
/// given read timeout; returns the stream and the FD protocol version.
pub(crate) fn connect_fd_with_timeout(address: &str, port: u16, password: &str, director_name: &str,
    read_timeout: Option<Duration>) -> Result<(BnetStream<bareos_bnet::stream::AnyStream>, u32), String> {
    if let Some((stream, _)) = crate::fd_grpc::try_connect(address, port, password, director_name, read_timeout) {
        return Ok((stream, 54));
    }
    use std::net::ToSocketAddrs;
    let socket = (address, port).to_socket_addrs().map_err(|e| format!("DNS: {e}"))?
        .next().ok_or("no FD address")?;
    let stream = TcpStream::connect_timeout(&socket, Duration::from_secs(5))
        .map_err(|e| format!("connect: {e}"))?;
    stream.set_read_timeout(read_timeout).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
    let stream = tls_psk::psk_connect(stream, &format!("R_DIRECTOR\x1e{director_name}"),
        &tls_psk::password_to_psk(password)).map_err(|e| format!("TLS-PSK: {e}"))?;
    let mut bnet = BnetStream::new(bareos_bnet::AnyStream::Psk(stream));
    bnet.send_str(&make_director_hello(director_name)).map_err(|e| e.to_string())?;
    let auth = CramMd5Auth::new(password, director_name, 2);
    if !auth.two_way_authenticate(&mut bnet, false).map_err(|e| e.to_string())?.success {
        return Err("FD authentication failed".into());
    }
    let hello = bnet.recv_str().map_err(|e| format!("FD hello: {e}"))?;
    let hello = hello.strip_prefix(FD_OK_HELLO).ok_or("invalid FD hello")?;
    let version = hello.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
    Ok((bnet, version))
}

/// Handshake mode of a console connection to a File Daemon
/// (C++ ClientConnectionHandshakeMode, dird/fd_cmds.cc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandshakeMode { TlsFirst, CleartextFirst }

/// Per client, the handshake mode that succeeded last; kept until the
/// configuration is reloaded (`connection_successful_handshake_`).
static SUCCESSFUL_HANDSHAKE: std::sync::Mutex<Option<std::collections::HashMap<String, HandshakeMode>>>
    = std::sync::Mutex::new(None);

/// Forget the stored handshake modes (configuration reload).
pub fn reset_handshake_modes() {
    if let Ok(mut modes) = SUCCESSFUL_HANDSHAKE.lock() { *modes = None; }
}

fn stored_handshake(client: &str) -> Option<HandshakeMode> {
    SUCCESSFUL_HANDSHAKE.lock().ok()?.as_ref()?.get(client).copied()
}

/// A non-console connect (e.g. estimate) that succeeded with immediate TLS
/// records the mode silently, like ConnectToFileDaemon without UaContext.
pub(crate) fn note_tls_handshake(client: &str) {
    if stored_handshake(client).is_none() {
        store_handshake(client, HandshakeMode::TlsFirst);
    }
}

/// The stored handshake mode of a client: Some(true) cleartext, Some(false)
/// immediate TLS, None not connected yet (probing). S-27.
pub(crate) fn stored_cleartext(client: &str) -> Option<bool> {
    stored_handshake(client).map(|mode| mode == HandshakeMode::CleartextFirst)
}

/// A job connection succeeded (connection_successful_handshake_). S-27.
pub(crate) fn store_client_handshake(client: &str, cleartext: bool) {
    store_handshake(client, if cleartext { HandshakeMode::CleartextFirst } else { HandshakeMode::TlsFirst });
}

fn store_handshake(client: &str, mode: HandshakeMode) {
    if let Ok(mut modes) = SUCCESSFUL_HANDSHAKE.lock() {
        modes.get_or_insert_with(Default::default).insert(client.to_string(), mode);
    }
}

/// Connect to a File Daemon for a console command like the C++
/// ConnectToFileDaemon with a UaContext (dird/fd_cmds.cc:279). Returns the
/// console text it produces (OutputMessageForConnectionTry,
/// SendInfoSuccess, SendInfoChosenCipher) and the authenticated connection.
/// The text is also returned when the connection fails.
pub fn connect_fd_console(address: &str, port: u16, password: &str, director_name: &str, client_name: &str,
                          tls: (bool, bool))
    -> (String, Result<BnetStream<bareos_bnet::stream::AnyStream>, String>) {
    if let Some((stream, _)) = crate::fd_grpc::try_connect(address, port, password, director_name, Some(Duration::from_secs(15))) {
        return (" Handshake: gRPC TLS,\n".into(), Ok(stream));
    }
    let (tls_enable, tls_require) = tls;
    let stored = stored_handshake(client_name);
    let mut info = String::new();
    if stored.is_none() {
        info.push_str("Probing client protocol... (result will be saved until config reload)");
    }
    // SetConnectionHandshakeMode(): the stored mode, else immediate TLS for a
    // TLS-configured client (TLS Enable, default yes), else cleartext.
    let mut mode = stored.unwrap_or(if tls_enable { HandshakeMode::TlsFirst } else { HandshakeMode::CleartextFirst });
    let mut last_error;
    loop {
        match connect_fd_mode(address, port, password, director_name, mode) {
            Ok((bnet, cipher)) => {
                if stored.is_none() { info.push_str("\r\x0b"); }
                info.push_str(match mode {
                    HandshakeMode::TlsFirst => " Handshake: Immediate TLS,",
                    HandshakeMode::CleartextFirst => " Handshake: Cleartext,",
                });
                info.push_str(&tls_psk::cipher_message_string(cipher.as_deref()));
                info.push('\n');
                store_handshake(client_name, mode);
                return (info, Ok(bnet));
            }
            Err(error) => last_error = error,
        }
        if last_error.starts_with("connect:") || mode == HandshakeMode::CleartextFirst { break; }
        // UpdateFailedConnectionHandshakeMode(): no cleartext when the client
        // requires TLS (TLS Require, default yes). S-28.
        if tls_enable && tls_require { break; }
        mode = HandshakeMode::CleartextFirst;
    }
    (info, Err(last_error))
}

fn connect_fd_mode(address: &str, port: u16, password: &str, director_name: &str, mode: HandshakeMode)
    -> Result<(BnetStream<bareos_bnet::stream::AnyStream>, Option<String>), String> {
    use bareos_bnet::stream::AnyStream;
    use std::net::ToSocketAddrs;
    let socket = (address, port).to_socket_addrs().map_err(|e| format!("connect: {e}"))?
        .next().ok_or("connect: no FD address")?;
    let stream = TcpStream::connect_timeout(&socket, Duration::from_secs(5))
        .map_err(|e| format!("connect: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(15))).map_err(|e| e.to_string())?;
    let (stream, cipher) = match mode {
        HandshakeMode::TlsFirst => {
            let tls = tls_psk::psk_connect(stream, &format!("R_DIRECTOR\x1e{director_name}"),
                &tls_psk::password_to_psk(password)).map_err(|e| format!("TLS-PSK: {e}"))?;
            let cipher = tls.cipher_name();
            (AnyStream::Psk(tls), Some(cipher))
        }
        HandshakeMode::CleartextFirst => (AnyStream::Plain(stream), None),
    };
    let mut bnet = BnetStream::new(stream);
    bnet.send_str(&make_director_hello(director_name)).map_err(|e| e.to_string())?;
    let auth = CramMd5Auth::new(password, director_name, 2);
    if !auth.two_way_authenticate(&mut bnet, false).map_err(|e| e.to_string())?.success {
        return Err("FD authentication failed".into());
    }
    let hello = bnet.recv_str().map_err(|e| format!("FD hello: {e}"))?;
    hello.strip_prefix(FD_OK_HELLO).ok_or("invalid FD hello")?;
    Ok((bnet, cipher))
}

/// Send each command on an authenticated FD connection and collect every
/// reply up to its EOD, then terminate the connection (DoClientResolve).
pub fn fd_query_commands<S: std::io::Read + std::io::Write>(mut fd: BnetStream<S>, commands: &[String]) -> Result<String, String> {
    let result = (|| {
        let mut output = String::new();
        for command in commands {
            fd.send_str(command).map_err(|e| e.to_string())?;
            output.push_str(&read_listing_reply(&mut fd)?);
        }
        Ok(output)
    })();
    let _ = fd.send_signal(BnetSignal::Terminate);
    result
}

/// C++ getmsg.cc BgetDirmsg(): the next numbered response line, answering
/// poll/status/btime signals and swallowing Jmsg/request lines. `None` when
/// the FD sent a terminating signal (EOD, EOD_POLL, TERMINATE, unknown).
pub(crate) fn get_dir_msg<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>) -> Result<Option<String>, String> {
    loop {
        match fd.recv().map_err(|error| format!("FD response: {error}"))? {
            BnetMessage::Data(bytes) => {
                let message = String::from_utf8_lossy(&bytes).into_owned();
                if message.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    return Ok(Some(message));
                }
                // Jmsg/CatReq/... from the FD: dispatched to the job messages in C++.
                debug!("FD message: {}", message.trim_end());
            }
            BnetMessage::Signal(BnetSignal::EodPoll) => {
                fd.send_str("1000 OK\n").map_err(|e| e.to_string())?;
                return Ok(None);
            }
            BnetMessage::Signal(BnetSignal::Poll) => fd.send_str("1000 OK\n").map_err(|e| e.to_string())?,
            BnetMessage::Signal(BnetSignal::Heartbeat | BnetSignal::HeartbeatResponse) => {}
            BnetMessage::Signal(BnetSignal::Status) => {
                fd.send_str("Status OK\n").map_err(|e| e.to_string())?;
                fd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
            }
            BnetMessage::Signal(BnetSignal::Btime) => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                fd.send_str(&format!("btime {}\n", now.as_micros())).map_err(|e| e.to_string())?;
            }
            BnetMessage::Signal(_) => return Ok(None),
        }
    }
}

fn read_listing_reply<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>) -> Result<String, String> {
    let mut output = String::new();
    loop {
        match fd.recv().map_err(|error| format!("FD response: {error}"))? {
            // ua->SendMsg("%s", fd->msg) ends each message at its first NUL.
            BnetMessage::Data(mut bytes) => {
                if let Some(end) = bytes.iter().position(|&b| b == 0) { bytes.truncate(end); }
                output.push_str(&String::from_utf8(bytes).map_err(|e| e.to_string())?)
            }
            BnetMessage::Signal(BnetSignal::Eod) => return Ok(output),
            BnetMessage::Signal(BnetSignal::Terminate) => return Err("FD terminated listing before EOD".into()),
            BnetMessage::Signal(BnetSignal::Heartbeat) => fd.send_signal(BnetSignal::HeartbeatResponse).map_err(|e| e.to_string())?,
            BnetMessage::Signal(_) => {},
        }
    }
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
    #[test]
    fn listing_collects_multiple_frames_without_changing_newlines() {
        let mut bytes = Vec::new();
        for data in [b"first\n".as_slice(), b"second\r\n\0".as_slice(), b"\0third\n".as_slice()] {
            bytes.extend_from_slice(&(data.len() as i32).to_be_bytes());
            bytes.extend_from_slice(data);
        }
        bytes.extend_from_slice(&(-1i32).to_be_bytes());
        let mut fd = BnetStream::new(Stream { input: Cursor::new(bytes), output: vec![] });
        assert_eq!(read_listing_reply(&mut fd).unwrap(), "first\nsecond\r\n");
    }

}
