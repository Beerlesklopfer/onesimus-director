//! Verbindung des Directors zum Storage Daemon: TLS-PSK, Hello, CRAM-MD5, OK-Hello.
//!
//! Zweck: Jeder Weg vom Director zum SD (Job, `status storage`, `label`,
//! `setbandwidth` …) beginnt mit demselben Handshake. Er steht nur hier:
//! [`connect_sd`] für Befehle (Adresse und Port, Lese-Timeout bleibt
//! gesetzt) und [`connect_sd_on`] für einen schon verbundenen Socket (der
//! Job-Pfad von odir, der den Socket für den Abbruch registriert).
//!
//! Ablauf (Bareos `dird/authenticate.cc` `AuthenticateWithStorageDaemon`):
//!
//! 1. TLS-PSK mit der Identität `R_DIRECTOR\x1e<Director>` und dem
//!    SD-Passwort als Schlüssel.
//! 2. `Hello Director <Name> calling` ([`make_director_hello`]).
//! 3. CRAM-MD5 in beide Richtungen.
//! 4. Der SD antwortet `3000 OK Hello`; jede andere Antwort ist eine
//!    Ablehnung ([`SdConnectError::Rejected`]).
//!
//! Während des Handshakes gilt ein Lese-Timeout ([`HANDSHAKE_TIMEOUT`]).
//! Danach setzt der Aufrufer den Timeout, den er braucht (Befehle: 60 s;
//! Jobs: keiner, weil ein SD auf Mount-Anforderungen beliebig lange warten
//! kann).
//!
//! Zusammengeführt (20.11.2026) aus `osd/src/connect.rs` `connect_sd`,
//! `osd/src/status.rs` `query_sd_status` und `odir/src/job_worker.rs`
//! `connect_sd_on`. Die odir-Fassung verwarf die OK-Hello-Antwort und lief
//! ohne Lese-Timeout (Defekt D12 des ofd-Gutachtens): Ein SD, der das Hello
//! ablehnt, fiel erst beim nächsten Befehl auf, ein stummer SD hielt den Job
//! für immer fest.

use std::net::TcpStream;
use std::time::Duration;

use tracing::debug;

use bareos_bnet::hello::{SD_OK_HELLO, make_director_hello};
use bareos_bnet::tls_psk::{self as psk, PskStream};
use bareos_bnet::{BnetStream, CramMd5Auth};

/// Read timeout while the handshake runs (TLS-PSK, CRAM-MD5, OK-Hello).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);

/// Read timeout [`connect_sd`] leaves on the socket for the commands that
/// follow.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// How long [`connect_sd`] waits for the TCP connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The step of the handshake that failed. `Display` gives the texts the
/// Director's job path has always shown after "Failed to connect to Storage
/// daemon: ".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdConnectError {
    /// TLS-PSK failed (wrong password, no TLS).
    Tls(String),
    /// The Hello could not be sent.
    Hello(String),
    /// CRAM-MD5 broke off (I/O error, protocol error).
    Cram(String),
    /// CRAM-MD5 finished but the SD refused us.
    CramFailed,
    /// No answer to the Hello (closed, timeout).
    HelloResponse(String),
    /// The SD answered, but not with `3000 OK Hello` (D12).
    Rejected(String),
}

impl std::fmt::Display for SdConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tls(e) => write!(f, "TLS-PSK: {e}"),
            Self::Hello(e) => write!(f, "hello: {e}"),
            Self::Cram(e) => write!(f, "CRAM: {e}"),
            Self::CramFailed => write!(f, "CRAM failed"),
            Self::HelloResponse(e) => write!(f, "hello resp: {e}"),
            Self::Rejected(resp) => write!(f, "SD hello: {resp}"),
        }
    }
}

impl std::error::Error for SdConnectError {}

/// The handshake on a connected socket. [`HANDSHAKE_TIMEOUT`] applies
/// while it runs, `read_timeout_after` once it succeeded (`None`: block).
/// Returns the authenticated connection and the TLS cipher name.
pub fn connect_sd_on(
    sock: TcpStream,
    password: &str,
    director_name: &str,
    read_timeout_after: Option<Duration>,
) -> Result<(BnetStream<PskStream>, String), SdConnectError> {
    connect_sd_on_with(sock, password, director_name, HANDSHAKE_TIMEOUT, read_timeout_after)
}

/// [`connect_sd_on`] with an explicit handshake timeout (tests).
pub fn connect_sd_on_with(
    sock: TcpStream,
    password: &str,
    director_name: &str,
    handshake_timeout: Duration,
    read_timeout_after: Option<Duration>,
) -> Result<(BnetStream<PskStream>, String), SdConnectError> {
    sock.set_read_timeout(Some(handshake_timeout)).ok();
    let tcp = sock.try_clone().ok();

    let key = psk::password_to_psk(password);
    let stream = psk::psk_connect(sock, &format!("R_DIRECTOR\x1e{director_name}"), &key)
        .map_err(|e| SdConnectError::Tls(e.to_string()))?;
    let cipher = stream.cipher_name();
    let mut bnet = BnetStream::new(stream);

    bnet.send_str(&make_director_hello(director_name)).map_err(|e| SdConnectError::Hello(e.to_string()))?;
    let auth = CramMd5Auth::new(password, director_name, 2);
    let r = auth.two_way_authenticate(&mut bnet, false).map_err(|e| SdConnectError::Cram(e.to_string()))?;
    if !r.success {
        return Err(SdConnectError::CramFailed);
    }
    let resp = bnet.recv_str().map_err(|e| SdConnectError::HelloResponse(e.to_string()))?;
    if !resp.starts_with(SD_OK_HELLO) {
        return Err(SdConnectError::Rejected(resp));
    }

    if let Some(tcp) = tcp {
        tcp.set_read_timeout(read_timeout_after).ok();
    }
    Ok((bnet, cipher))
}

/// Connects to a Storage Daemon at `address:port` and authenticates;
/// [`COMMAND_TIMEOUT`] stays set for the commands that follow.
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

    let stream = TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT)
        .map_err(|e| format!("connect {addr}: {e}"))?;

    // The texts of connect_sd before 20.11.2026 where they differ.
    let (bnet, _) = connect_sd_on(stream, password, director_name, Some(COMMAND_TIMEOUT)).map_err(|e| match e {
        SdConnectError::Tls(e) => format!("TLS-PSK {addr}: {e}"),
        SdConnectError::Cram(e) => format!("CRAM-MD5: {e}"),
        SdConnectError::CramFailed => "CRAM-MD5 auth failed".to_string(),
        other => other.to_string(),
    })?;

    debug!("SD connected: {addr}");
    Ok(bnet)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::net::TcpListener;

    /// What the fake Storage Daemon does after TLS-PSK and the Hello.
    #[derive(Clone, Copy)]
    pub(crate) enum FakeSd {
        /// CRAM-MD5, then this line instead of the Hello answer.
        Answer(&'static str),
        /// CRAM-MD5, then nothing (keeps the connection open).
        Silent,
    }

    /// A Storage Daemon on loopback for one connection: TLS-PSK with
    /// `password`, reads the Hello, CRAM-MD5 as server, then `mode`.
    /// Returns the address and the Hello it received.
    pub(crate) fn fake_sd(password: &'static str, mode: FakeSd) -> (String, u16, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let (stream, _) = psk::psk_accept_with(sock, move |_| Some(psk::password_to_psk(password))).unwrap();
            let mut bnet = BnetStream::new(stream);
            let hello = bnet.recv_str().unwrap();
            let auth = CramMd5Auth::new(password, "R_STORAGE::fake-sd", 2);
            let ok = auth.two_way_authenticate(&mut bnet, true).map(|r| r.success).unwrap_or(false);
            assert!(ok, "fake SD: CRAM-MD5 failed");
            match mode {
                FakeSd::Answer(line) => {
                    bnet.send_str(line).unwrap();
                    std::thread::sleep(Duration::from_millis(200));
                }
                FakeSd::Silent => std::thread::sleep(Duration::from_secs(3)),
            }
            hello
        });
        ("127.0.0.1".into(), port, handle)
    }

    #[test]
    fn ok_hello_is_accepted_with_the_bareos_hello() {
        let (addr, port, sd) = fake_sd("pw", FakeSd::Answer("3000 OK Hello\n"));
        let bnet = connect_sd(&addr, port, "pw", "My Dir").unwrap();
        drop(bnet);
        let hello = sd.join().unwrap();
        assert!(hello.starts_with("Hello Director "), "{hello}");
    }

    #[test]
    fn d12_wrong_hello_is_rejected_on_the_job_path() {
        let (addr, port, sd) = fake_sd("pw", FakeSd::Answer("3999 No go\n"));
        let sock = TcpStream::connect((addr.as_str(), port)).unwrap();
        let err = connect_sd_on(sock, "pw", "test-dir", None).err().unwrap();
        assert_eq!(err, SdConnectError::Rejected("3999 No go\n".into()));
        assert_eq!(err.to_string(), "SD hello: 3999 No go\n");
        sd.join().unwrap();
        // the command path reports the same
        let (addr, port, sd) = fake_sd("pw", FakeSd::Answer("2000 OK Hello 54\n"));
        assert_eq!(connect_sd(&addr, port, "pw", "test-dir").err().unwrap(), "SD hello: 2000 OK Hello 54\n");
        sd.join().unwrap();
    }

    #[test]
    fn d12_silent_sd_times_out_in_the_handshake() {
        let (addr, port, sd) = fake_sd("pw", FakeSd::Silent);
        let sock = TcpStream::connect((addr.as_str(), port)).unwrap();
        let start = std::time::Instant::now();
        let err = connect_sd_on_with(sock, "pw", "test-dir", Duration::from_millis(500), None).err().unwrap();
        assert!(matches!(err, SdConnectError::HelloResponse(_)), "{err:?}");
        assert!(start.elapsed() < Duration::from_secs(3));
        sd.join().unwrap();
    }

    #[test]
    fn wrong_password_fails_before_the_hello() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let sd = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let _ = psk::psk_accept_with(sock, |_| Some(psk::password_to_psk("right")));
        });
        let sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let err = connect_sd_on(sock, "wrong", "test-dir", None).err().unwrap();
        assert!(matches!(err, SdConnectError::Tls(_)), "{err:?}");
        assert!(err.to_string().starts_with("TLS-PSK: "));
        sd.join().unwrap();
    }
}
