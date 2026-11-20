//! Annahme von BNET-Verbindungen: Accept-Schleife, TLS-PSK oder Klartext.
//!
//! Zweck: Jeder Daemon wartet auf einem TCP-Port auf Gegenstellen
//! (Director, SD, FD). Je Verbindung entsteht ein eigener, benannter Thread
//! (`<präfix>:<n>`). Beginnt die Gegenstelle mit einem TLS-ClientHello
//! (erstes Byte `0x16`), folgt TLS-PSK; den Schlüssel wählt die
//! PSK-Identität ([`PskIdentity`]), z. B. `R_DIRECTOR\x1e<Name>` → Passwort
//! dieses Directors. Sonst läuft die Verbindung im Klartext weiter.
//!
//! Datenfluss: [`serve`] → je Verbindung `handler(stream, n)` im eigenen
//! Thread → dort [`open_bnet`] → [`crate::handshake`] → Sitzung des Daemons.
//!
//! Bareos-Gegenstück: `core/src/lib/bnet_server_tcp.cc`
//! (`BnetThreadServerTcp`), `lib/tls_openssl_private.cc`
//! (PSK-Server-Callback), `filed/socket_server.cc`,
//! `stored/socket_server.cc`.
//!
//! Herkunft: `ofd/src/accept.rs` (`accept_loop`, `handle_connection`,
//! `psk_for_identity`) und `ofd/src/sd.rs` (`Rw`), am 20.11.2026 ohne
//! Verhaltensänderung herausgelöst. Binden des Ports und die Fehlertexte
//! dazu bleiben im Daemon, weil sie dessen Konfiguration nennen.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use bareos_bnet::{BnetStream, tls_psk};
use tracing::{debug, info, warn};

/// Any byte stream a BNET connection can run over.
pub trait Rw: Read + Write + Send {}
impl<T: Read + Write + Send> Rw for T {}

/// A BNET connection over any stream (TLS-PSK, plain TCP, bandwidth limited …).
pub type AnyBnet = BnetStream<Box<dyn Rw>>;

/// The kind and name a peer announces in its TLS-PSK identity
/// (`<kind>\x1e<name>`, `lib/bsock.cc` / `lib/tls_psk_credentials.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PskIdentity<'a> {
    /// `R_DIRECTOR\x1e<name>`: a Director; the key is its password.
    Director(&'a str),
    /// `R_JOB\x1e<job>`: a job's data connection; the key is the job's
    /// authorization key.
    Job(&'a str),
    /// Any other kind (`R_CLIENT`, `R_STORAGE`, `R_CONSOLE` …) and its name.
    Other(&'a str, &'a str),
}

impl<'a> PskIdentity<'a> {
    /// Splits an identity at `\x1e`; `None` without a separator.
    pub fn parse(identity: &'a str) -> Option<Self> {
        let (kind, name) = identity.split_once('\x1e')?;
        Some(match kind {
            "R_DIRECTOR" => Self::Director(name),
            "R_JOB" => Self::Job(name),
            _ => Self::Other(kind, name),
        })
    }
}

/// Accepts connections on `listener` until `shutdown` is set and runs
/// `handler(stream, n)` for each in a thread named `<thread_prefix>:<n>`,
/// `n` counting from 1.
pub fn serve<F>(listener: TcpListener, thread_prefix: &str, shutdown: &AtomicBool, handler: F)
where
    F: Fn(TcpStream, u64) + Send + Sync + 'static,
{
    let handler = Arc::new(handler);
    let mut next_session = 1u64;
    listener.set_nonblocking(true).ok();

    while !shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, peer_addr)) => {
                debug!("new connection from {peer_addr}");
                stream.set_nonblocking(false).ok();

                let session_id = next_session;
                next_session += 1;
                let handler = handler.clone();

                thread::Builder::new()
                    .name(format!("{thread_prefix}:{session_id}"))
                    .spawn(move || {
                        handler(stream, session_id);
                    })
                    .ok();
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                warn!("accept error: {e}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    info!("accept loop stopped");
}

/// TLS-PSK when the peer starts with a ClientHello (first byte `0x16`),
/// the key chosen by `lookup(identity)`, else plain BNET. `None` when the
/// peer sent nothing or TLS-PSK failed (logged at debug level).
pub fn open_bnet<F>(stream: TcpStream, session_id: u64, lookup: F) -> Option<AnyBnet>
where
    F: Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static,
{
    let mut peek_buf = [0u8; 1];
    match stream.peek(&mut peek_buf) {
        Ok(1) if peek_buf[0] == 0x16 => match tls_psk::psk_accept_with(stream, lookup) {
            Ok((s, identity)) => {
                debug!("session {session_id}: TLS-PSK accept OK ({})", identity.replace('\x1e', ":"));
                Some(BnetStream::new(Box::new(s)))
            }
            Err(e) => {
                debug!("session {session_id}: TLS-PSK failed: {e}");
                None
            }
        },
        Ok(1) => {
            debug!("session {session_id}: plaintext connection");
            Some(BnetStream::new(Box::new(stream)))
        }
        _ => {
            debug!("session {session_id}: peek failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psk_identities_like_ofd() {
        assert_eq!(PskIdentity::parse("R_DIRECTOR\x1ePDC-dir"), Some(PskIdentity::Director("PDC-dir")));
        assert_eq!(PskIdentity::parse("R_JOB\x1eVBackup.2026-11-20_01.00.00_03"),
            Some(PskIdentity::Job("VBackup.2026-11-20_01.00.00_03")));
        assert_eq!(PskIdentity::parse("R_CLIENT\x1ex"), Some(PskIdentity::Other("R_CLIENT", "x")));
        assert_eq!(PskIdentity::parse("no separator"), None);
    }

    #[test]
    fn serve_hands_each_connection_to_a_named_thread() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        static STOP: AtomicBool = AtomicBool::new(false);
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        let server = thread::spawn(move || {
            serve(listener, "t:session", &STOP, move |mut s, n| {
                let mut b = [0u8; 2];
                s.read_exact(&mut b).unwrap();
                let name = thread::current().name().unwrap().to_string();
                tx.lock().unwrap().send((n, name, b)).unwrap();
            })
        });
        for _ in 0..2 {
            TcpStream::connect(addr).unwrap().write_all(b"hi").unwrap();
        }
        let mut got: Vec<_> = (0..2).map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap()).collect();
        got.sort();
        assert_eq!(got[0], (1, "t:session:1".to_string(), *b"hi"));
        assert_eq!(got[1].0, 2);
        STOP.store(true, Ordering::Relaxed);
        server.join().unwrap();
    }

    #[test]
    fn open_bnet_takes_plaintext_and_rejects_silent_peers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            let mut bnet = BnetStream::new(&mut c);
            bnet.send_str("Hello Director x calling\n").unwrap();
            drop(TcpStream::connect(addr).unwrap()); // closes without a byte
        });
        let (s, _) = listener.accept().unwrap();
        let mut bnet = open_bnet(s, 1, |_| None).expect("plaintext");
        assert_eq!(bnet.recv_str().unwrap(), "Hello Director x calling\n");
        client.join().unwrap();
        let (s, _) = listener.accept().unwrap();
        assert!(open_bnet(s, 2, |_| None).is_none());
    }
}
