// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Der Adapter: ein gRPC-Strom als `Read`/`Write` von BNET-Rahmen.
//!
//! Zweck: Die Daemons sprechen synchron (`BnetStream` über `Read`/`Write`),
//! gRPC läuft in tokio. [`pair`] liefert beide Enden: [`ControlIo`] für den
//! synchronen Code und [`ControlPeer`] für die Pumpe, die Rahmen zwischen
//! den Warteschlangen und dem gRPC-Strom bewegt ([`crate::client`],
//! [`crate::server`]). `write` zerlegt die Bytes, die `BnetStream` schreibt
//! (Längenkopf, dann Nutzdaten), in je einen [`Frame`]; `read` baut den
//! Längenkopf für `BnetStream::recv` wieder auf.
//!
//! Abbruch: Ein Lesen wartet in Schritten von 100 ms und prüft dazwischen
//! die Abbruchmerker ([`ControlIo::cancel`], [`ControlIo::close_handle`]);
//! dann scheitert es mit `ConnectionAborted` (nicht `Interrupted`, das
//! `read_exact` endlos wiederholen würde).
//! Das ersetzt `shutdown(Read)` auf dem TCP-Socket, das es bei gRPC nicht
//! gibt. Vor der Anmeldung gilt eine Lesegrenze
//! ([`ControlIo::set_read_timeout`]), danach keine: Jobs warten beliebig
//! lange (Mount-Anforderungen).
//!
//! Herkunft: `bareos-bnet/src/fd_grpc.rs` (bis 20.11.2026), unverändert bis
//! auf den Rahmentyp ([`Frame`] statt `filed::ControlFrame`) und den
//! zusätzlichen Schließmerker.
//!
//! Bareos-Gegenstück: `core/src/lib/bsock_tcp.cc` (`recv`, `send`).

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::frame::Frame;

/// Frames buffered in each direction.
const QUEUE: usize = 8;

/// Largest payload of one frame (`MAX_PACKET_SIZE` of BNET).
pub const MAX_FRAME: usize = bareos_bnet::signal::MAX_PACKET_SIZE;

/// Largest gRPC message: one frame plus protobuf overhead.
pub const MAX_MESSAGE: usize = MAX_FRAME + 64;

/// How long a `write` waits for room in a full queue.
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

#[cfg_attr(doc, aquamarine::aquamarine)]
/// The synchronous end of a gRPC stream: `Read`/`Write` of BNET frames.
///
/// ```mermaid
/// classDiagram
///     class ControlIo {
///         +cancel
///         +authenticated
///         +set_read_timeout(timeout)
///         +close_handle()
///         +read(buf)
///         +write(buf)
///     }
///     class ControlPeer { +incoming +outgoing }
///     class Frame { <<enum>> }
///     class WireFrame { <<trait>> }
///     class Target { +host +port +ca +server_name }
///     class Rpc { <<enum>> }
///     ControlIo .. ControlPeer : pair()
///     ControlPeer --> Frame : queues
///     WireFrame <|.. ControlFrame : filed
///     WireFrame <|.. StoredFrame : stored
///     Target --> Rpc : client connect
///     Rpc --> ControlIo : returns
/// ```
pub struct ControlIo {
    /// Frames from the peer (or the error that ended the stream).
    incoming: flume::Receiver<io::Result<Frame>>,
    /// Frames to the peer.
    outgoing: mpsc::Sender<Frame>,
    /// The rest of the frame being read, with its rebuilt length header.
    read_buffer: io::Cursor<Vec<u8>>,
    /// Bytes written but not yet a whole frame.
    write_buffer: Vec<u8>,
    /// Read limit until authenticated.
    read_timeout: Option<Duration>,
    /// Daemon-wide or worker-wide cancel flag (shutdown, job cancel).
    pub cancel: Option<&'static AtomicBool>,
    /// Set once the peer authenticated: reads then wait without limit.
    pub authenticated: Arc<AtomicBool>,
    /// Per-connection close flag ([`ControlIo::close_handle`]).
    closed: Arc<AtomicBool>,
}

/// The asynchronous end of a gRPC stream, driven by a pump.
pub struct ControlPeer {
    /// Where the pump puts the frames it received.
    pub incoming: flume::Sender<io::Result<Frame>>,
    /// Where the pump takes the frames to send.
    pub outgoing: mpsc::Receiver<Frame>,
}

/// Both ends of a new stream; `timeout` limits reads until
/// [`ControlIo::authenticated`] is set.
pub fn pair(timeout: Option<Duration>) -> (ControlIo, ControlPeer) {
    let (incoming_tx, incoming_rx) = flume::bounded(QUEUE);
    let (outgoing_tx, outgoing_rx) = mpsc::channel(QUEUE);
    (
        ControlIo {
            incoming: incoming_rx,
            outgoing: outgoing_tx,
            read_buffer: io::Cursor::new(Vec::new()),
            write_buffer: Vec::new(),
            read_timeout: timeout,
            cancel: None,
            authenticated: Default::default(),
            closed: Default::default(),
        },
        ControlPeer { incoming: incoming_tx, outgoing: outgoing_rx },
    )
}

impl ControlIo {
    /// Sets the read limit; `None` also marks the stream authenticated
    /// (no limit from now on).
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.read_timeout = timeout;
        self.authenticated.store(timeout.is_none(), Ordering::Relaxed);
    }

    /// A flag that, once set, makes every further `read` and `write` fail
    /// with `ConnectionAborted`, like `shutdown` on a socket (job cancel).
    pub fn close_handle(&self) -> Arc<AtomicBool> {
        self.closed.clone()
    }

    /// `Err(ConnectionAborted)` when a cancel or close flag is set.
    ///
    /// Not `Interrupted`: `read_exact`/`write_all` retry that kind at once,
    /// so a canceled stream would spin forever instead of failing (the
    /// adapter in `bareos-bnet/src/fd_grpc.rs` until 20.11.2026 did so).
    fn check_cancel(&self) -> io::Result<()> {
        if self.closed.load(Ordering::Relaxed) || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "gRPC stream canceled"));
        }
        Ok(())
    }

    /// The next frame from the peer as BNET bytes (header + payload);
    /// `Ok(None)` when the peer closed the stream.
    fn next_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        let timeout = if self.authenticated.load(Ordering::Relaxed) { None } else { self.read_timeout };
        let started = Instant::now();
        let frame = loop {
            self.check_cancel()?;
            let wait = timeout
                .map(|limit| limit.saturating_sub(started.elapsed()))
                .unwrap_or(Duration::from_millis(100))
                .min(Duration::from_millis(100));
            match self.incoming.recv_timeout(wait) {
                Ok(frame) => break frame?,
                Err(flume::RecvTimeoutError::Disconnected) => return Ok(None),
                Err(flume::RecvTimeoutError::Timeout) => {
                    if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "gRPC read timeout"));
                    }
                }
            }
        };
        match frame {
            Frame::Payload(payload) if payload.len() <= MAX_FRAME => {
                let mut bytes = (payload.len() as i32).to_be_bytes().to_vec();
                bytes.extend(payload);
                Ok(Some(bytes))
            }
            Frame::Signal(signal) if signal < 0 && bareos_bnet::BnetSignal::from_i32(signal).is_some() => {
                Ok(Some(signal.to_be_bytes().to_vec()))
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "invalid gRPC frame")),
        }
    }

    /// Queues `frame`, waiting up to [`WRITE_TIMEOUT`] for room.
    fn queue(&self, mut frame: Frame) -> io::Result<()> {
        let started = Instant::now();
        loop {
            self.check_cancel()?;
            match self.outgoing.try_send(frame) {
                Ok(()) => return Ok(()),
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "gRPC stream closed"));
                }
                Err(mpsc::error::TrySendError::Full(back)) => frame = back,
            }
            if started.elapsed() >= WRITE_TIMEOUT {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "gRPC write timeout"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Read for ControlIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.read_buffer.position() as usize == self.read_buffer.get_ref().len() {
            match self.next_frame()? {
                Some(bytes) => self.read_buffer = io::Cursor::new(bytes),
                None => return Ok(0),
            }
        }
        self.read_buffer.read(buf)
    }
}

impl Write for ControlIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // BnetStream writes a header and its bounded payload separately.
        if self.write_buffer.len().saturating_add(buf.len()) > MAX_FRAME + 4 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "gRPC frame too large"));
        }
        self.write_buffer.extend_from_slice(buf);
        while self.write_buffer.len() >= 4 {
            let len = i32::from_be_bytes(self.write_buffer[..4].try_into().unwrap_or_default());
            let size = if len <= 0 { 4 } else { len as usize + 4 };
            if size > MAX_FRAME + 4 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "gRPC frame too large"));
            }
            if self.write_buffer.len() < size {
                break;
            }
            let frame = if len < 0 { Frame::Signal(len) } else { Frame::Payload(self.write_buffer[4..size].to_vec()) };
            self.queue(frame)?;
            self.write_buffer.drain(..size);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl bareos_bnet::stream::ExternalStream for ControlIo {
    fn transport(&self) -> &str {
        crate::TRANSPORT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};

    #[test]
    fn frames_preserve_binary_payload_signals_and_partial_reads() {
        let (io, mut peer) = pair(None);
        let mut stream = BnetStream::new(io);
        stream.send(&[0, 255, 1, 0, 128]).unwrap();
        assert_eq!(peer.outgoing.try_recv().unwrap(), Frame::Payload(vec![0, 255, 1, 0, 128]));
        stream.send_signal(BnetSignal::Eod).unwrap();
        assert_eq!(peer.outgoing.try_recv().unwrap(), Frame::Signal(BnetSignal::Eod as i32));
        peer.incoming.send(Ok(Frame::Payload(vec![0, 255, 4]))).unwrap();
        assert_eq!(stream.recv().unwrap(), BnetMessage::Data(vec![0, 255, 4]));
        peer.incoming.send(Ok(Frame::Signal(BnetSignal::Eod as i32))).unwrap();
        assert_eq!(stream.recv().unwrap(), BnetMessage::Signal(BnetSignal::Eod));
    }

    #[test]
    fn malformed_frames_and_disconnect_are_errors() {
        for frame in [Frame::Signal(1), Frame::Signal(-99999), Frame::Payload(vec![0; MAX_FRAME + 1])] {
            let (io, peer) = pair(None);
            peer.incoming.send(Ok(frame)).unwrap();
            let mut stream = BnetStream::new(io);
            assert!(stream.recv().is_err());
        }
        let (io, peer) = pair(Some(Duration::from_millis(1)));
        let mut stream = BnetStream::new(io);
        assert!(stream.recv().is_err());
        drop(peer);
        assert!(stream.recv().is_err());
    }

    #[test]
    fn close_handle_breaks_a_blocked_read() {
        let (io, _peer) = pair(None);
        let closer = io.close_handle();
        let reader = std::thread::spawn(move || BnetStream::new(io).recv().map_err(|e| e.to_string()));
        std::thread::sleep(Duration::from_millis(150));
        closer.store(true, Ordering::Relaxed);
        let result = reader.join().unwrap();
        assert!(result.is_err(), "{result:?}");
    }
}
