//! SD status query (Director → SD).

use std::net::TcpStream;
use std::time::Duration;

use tracing::debug;

use bareos_bnet::{BnetSignal, BnetMessage};

/// Result of an SD status query.
#[derive(Debug)]
pub struct SdStatusResult {
    /// Storage resource the query went to.
    pub storage_name: String,
    /// The SD's status output, or the error text.
    pub status_text: String,
    /// Whether the SD answered.
    pub success: bool,
}

/// Connect to SD and query status.
pub fn query_sd_status(
    address: &str,
    port: u16,
    password: &str,
    director_name: &str,
    storage_name: &str,
) -> SdStatusResult {
    let addr = format!("{address}:{port}");

    let stream = match TcpStream::connect_timeout(
        &{
            use std::net::ToSocketAddrs;
            addr.to_socket_addrs().ok().and_then(|mut a| a.next())
                .unwrap_or_else(|| format!("127.0.0.1:{port}").parse().unwrap())
        },
        Duration::from_secs(3),
    ) {
        Ok(s) => s,
        Err(e) => return fail(storage_name, &format!("connect {addr}: {e}")),
    };
    // Handshake in crate::connect (one implementation since 20.11.2026).
    let mut bnet = match crate::connect::connect_sd_on(stream, password, director_name, Some(Duration::from_secs(30))) {
        Ok((bnet, _)) => bnet,
        Err(e) => return fail(storage_name, &e.to_string()),
    };
    debug!("SD {storage_name}: connected");

    if let Err(e) = bnet.send_str("status") {
        return fail(storage_name, &format!("status send: {e}"));
    }

    let mut status_text = String::new();
    loop {
        match bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                status_text.push_str(&String::from_utf8_lossy(&data));
            }
            Ok(BnetMessage::Signal(BnetSignal::Eod | BnetSignal::Terminate)) => break,
            Ok(BnetMessage::Signal(_)) => continue,
            Err(_) => break,
        }
    }

    let _ = bnet.send_signal(BnetSignal::Terminate);

    SdStatusResult { storage_name: storage_name.to_string(), status_text, success: true }
}

fn fail(name: &str, msg: &str) -> SdStatusResult {
    SdStatusResult { storage_name: name.to_string(), status_text: msg.to_string(), success: false }
}
