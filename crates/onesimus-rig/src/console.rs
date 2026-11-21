//! Konsolensitzung zum Director, wie `bconsole` sie aufbaut.
//!
//! Zweck: Der Test gibt dem Director Befehle (`run`, `wait`, `restore`,
//! `cancel`, `estimate`) und liest die Antworten als Text.
//!
//! Datenfluss: TCP → TLS-PSK mit der Identität `R_CONSOLE\x1e*UserAgent*` →
//! Hello → CRAM-MD5 in beide Richtungen → Begrüßung lesen. Danach je Befehl
//! eine BNET-Nachricht senden und Antwortrahmen sammeln, bis der Director
//! `EOD` oder eine Eingabeaufforderung signalisiert. Der Ablauf entspricht
//! `crates/bareos-bnet/examples/bnet_capture.rs`.
//!
//! Bareos-Gegenstück: `console/console.cc` (`bconsole`) mit
//! `lib/bsock.cc` und `lib/cram_md5.cc`.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use bareos_bnet::cram_md5::CramMd5Auth;
use bareos_bnet::hello::make_console_hello;
use bareos_bnet::signal::BnetSignal;
use bareos_bnet::tls_psk::PskStream;
use bareos_bnet::wire::{BnetError, BnetMessage, BnetStream};

/// Signals after which the Director waits for console input.
const INPUT_SIGNALS: [BnetSignal; 6] = [
    BnetSignal::Prompt,
    BnetSignal::SubPrompt,
    BnetSignal::SelectInput,
    BnetSignal::YesNo,
    BnetSignal::TextInput,
    BnetSignal::MainPrompt,
];

/// An authenticated console session with the default console.
pub struct Console {
    /// The BNET stream over TLS-PSK.
    bnet: BnetStream<PskStream>,
    /// A clone of the socket to change the read timeout.
    socket: TcpStream,
}

impl std::fmt::Debug for Console {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Console").field("peer", &self.socket.peer_addr().ok()).finish()
    }
}

impl Console {
    /// Connects to the Director at `address` as `*UserAgent*` with `password`.
    pub fn connect(address: SocketAddr, password: &str) -> Result<Console, String> {
        Self::connect_as(address, "*UserAgent*", password)
    }

    /// Connects as the console `name` (a `Console` resource of the Director,
    /// e.g. `admin-mcp`), the way `bconsole -c` does.
    pub fn connect_as(address: SocketAddr, name: &str, password: &str) -> Result<Console, String> {
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
            .map_err(|e| format!("Konsole: connect {address}: {e}"))?;
        let socket = stream.try_clone().map_err(|e| e.to_string())?;
        let psk = bareos_bnet::tls_psk::password_to_psk(password);
        let tls = bareos_bnet::tls_psk::psk_connect(stream, &format!("R_CONSOLE\x1e{name}"), &psk)
            .map_err(|e| format!("Konsole: TLS-PSK: {e}"))?;
        let mut bnet = BnetStream::new(tls);
        let version = if name == "*UserAgent*" { "" } else { "25.1.3" };
        let hello = make_console_hello(name, version, false);
        bnet.send_str(&hello).map_err(|e| format!("Konsole: Hello: {e}"))?;
        let authenticated = CramMd5Auth::new(password, name, 0)
            .two_way_authenticate(&mut bnet, false)
            .is_ok_and(|result| result.success);
        if !authenticated {
            return Err("Konsole: Anmeldung am Director gescheitert".into());
        }
        let mut console = Console { bnet, socket };
        // The banner has no end signal: first frame, then silence.
        console.set_timeout(Duration::from_secs(30));
        if console.bnet.recv().is_ok() {
            console.set_timeout(Duration::from_millis(300));
            let _ = console.read_reply();
        }
        Ok(console)
    }

    /// Sets the read timeout of the session.
    fn set_timeout(&self, timeout: Duration) {
        let _ = self.socket.set_read_timeout(Some(timeout));
    }

    /// Reads data frames until `EOD`, an input signal or the timeout.
    fn read_reply(&mut self) -> Result<String, String> {
        let mut text = Vec::new();
        loop {
            match self.bnet.recv() {
                Ok(BnetMessage::Data(data)) => text.extend_from_slice(&data),
                Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
                Ok(BnetMessage::Signal(signal)) if INPUT_SIGNALS.contains(&signal) => break,
                Ok(BnetMessage::Signal(_)) => {}
                Err(BnetError::Io(error))
                    if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                {
                    break;
                }
                Err(error) => return Err(format!("Konsole: Verbindung beendet: {error}")),
            }
        }
        Ok(String::from_utf8_lossy(&text).into_owned())
    }

    /// Sends `command` and returns the reply text; waits at most `timeout`.
    pub fn command(&mut self, command: &str, timeout: Duration) -> Result<String, String> {
        self.set_timeout(timeout);
        self.bnet.send_str(command).map_err(|e| format!("Konsole: senden '{command}': {e}"))?;
        self.read_reply()
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.bnet.send_signal(BnetSignal::Terminate);
    }
}

/// The JobId of a "Job queued. JobId=N" reply.
pub fn queued_job_id(reply: &str) -> Option<u32> {
    let rest = &reply[reply.find("JobId=")? + "JobId=".len()..];
    rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_job_id_is_read() {
        assert_eq!(queued_job_id("Using Catalog \"MyCatalog\"\nJob queued. JobId=17\n"), Some(17));
        assert_eq!(queued_job_id("no job"), None);
    }
}
