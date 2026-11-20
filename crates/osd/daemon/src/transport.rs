//! Transport der Steuer- und Datenverbindungen.
//!
//! Zweck: Director-Sitzung und FD-Datenphase sprechen über [`Channel`],
//! nicht über einen konkreten Socket. Stufe 1 hat genau eine Umsetzung,
//! BNET ([`bareos_bnet::BnetStream`] über TLS-PSK oder Klartext). In Stufe 3
//! kommt gRPC (`onesimus-proto-stored`) als zweite Umsetzung dazu, ohne
//! dass Befehlsschleife oder Append-Logik sich ändern.
//!
//! Bareos-Gegenstück: `core/src/lib/bsock.h` (`BareosSocket` mit `recv`,
//! `send`, `signal`).

use std::io::{Read, Write};

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};

/// One message of a channel: data or a BNET signal.
pub type Message = BnetMessage;

/// A connection the daemon reads commands or data from and answers on.
pub trait Channel: Send {
    /// The next message; `Err` on a broken or closed connection.
    fn recv(&mut self) -> Result<Message, String>;
    /// Sends one data message.
    fn send(&mut self, data: &[u8]) -> Result<(), String>;
    /// Sends a signal (`EOD`, `TERMINATE` …).
    fn signal(&mut self, signal: BnetSignal) -> Result<(), String>;
    /// Sends a text message.
    fn send_str(&mut self, text: &str) -> Result<(), String> {
        self.send(text.as_bytes())
    }
}

impl<S: Read + Write + Send> Channel for BnetStream<S> {
    fn recv(&mut self) -> Result<Message, String> {
        BnetStream::recv(self).map_err(|e| e.to_string())
    }

    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        BnetStream::send(self, data).map_err(|e| e.to_string())
    }

    fn signal(&mut self, signal: BnetSignal) -> Result<(), String> {
        self.send_signal(signal).map_err(|e| e.to_string())
    }
}

/// A shared channel: the Director connection of a job is used by the
/// command loop, the FD session (attributes, job messages) and the mailbox
/// server (catalog requests) of the device writer.
pub type SharedChannel = std::sync::Arc<std::sync::Mutex<Box<dyn Channel>>>;

/// Locks `shared`, ignoring poisoning (a panicked user leaves the socket
/// in a defined state: whole messages only).
pub fn lock(shared: &SharedChannel) -> std::sync::MutexGuard<'_, Box<dyn Channel>> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
}

/// Text of a data message, lossy.
pub fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}
