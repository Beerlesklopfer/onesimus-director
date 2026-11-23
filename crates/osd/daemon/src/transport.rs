// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Transport der Steuer- und Datenverbindungen.
//!
//! Zweck: Director-Sitzung und FD-Datenphase sprechen über [`Channel`],
//! nicht über einen konkreten Socket. Zwei Umsetzungen, beide über
//! [`bareos_bnet::BnetStream`]: BNET (TLS-PSK oder Klartext auf dem
//! SD-Port) und seit Stufe 3 gRPC (`onesimus.stored.StorageDaemon`
//! Control/Data bzw. `onesimus.filed.FileDaemon/Data` beim passiven
//! Client), wo `onesimus_bnet_grpc::ControlIo` den Strom als `Read`/`Write`
//! liefert ([`crate::grpc`]). Befehlsschleife, Append- und Lese-Logik
//! wissen nicht, welche es ist. Schreiben und Lesen, aktiv und passiv
//! laufen so über beide.
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

/// One end of an in-process connection (two [`MemChannel`]s are a socket
/// pair): the copy inside one SD runs the reading and the writing side of
/// the copy protocol against each other without a network.
pub struct MemChannel {
    /// Messages for the other end; `None` after TERMINATE (hang up).
    tx: Option<std::sync::mpsc::Sender<Message>>,
    /// Messages from the other end.
    rx: std::sync::mpsc::Receiver<Message>,
}

impl MemChannel {
    /// Two connected ends.
    pub fn pair() -> (MemChannel, MemChannel) {
        let (a_tx, b_rx) = std::sync::mpsc::channel();
        let (b_tx, a_rx) = std::sync::mpsc::channel();
        (MemChannel { tx: Some(a_tx), rx: a_rx }, MemChannel { tx: Some(b_tx), rx: b_rx })
    }
}

impl Channel for MemChannel {
    fn recv(&mut self) -> Result<Message, String> {
        self.rx.recv().map_err(|_| "connection closed".to_string())
    }

    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        self.tx.as_ref().ok_or("connection closed")?.send(Message::Data(data.to_vec())).map_err(|_| "connection closed".to_string())
    }

    fn signal(&mut self, signal: BnetSignal) -> Result<(), String> {
        if signal == BnetSignal::Terminate {
            // like a hang-up: the other end's recv fails
            self.tx = None;
            return Ok(());
        }
        self.tx.as_ref().ok_or("connection closed")?.send(Message::Signal(signal)).map_err(|_| "connection closed".to_string())
    }
}
