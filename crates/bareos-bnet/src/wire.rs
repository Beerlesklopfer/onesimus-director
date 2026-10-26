//! BNET wire protocol: length-prefixed framing over TCP.
//!
//! The Bareos network protocol uses a simple framing:
//!
//! ```text
//! [4 bytes: i32 big-endian length] [length bytes: payload]
//! ```
//!
//! - Positive length: data message (payload follows)
//! - Negative length: out-of-band signal (no payload)
//! - Zero length: empty message / soft EOF

use std::io::{self, Read, Write};

use crate::signal::{BnetSignal, MAX_MESSAGE_LEN, MAX_PACKET_SIZE};

/// A message on the BNET wire: either data or a signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BnetMessage {
    /// Data payload
    Data(Vec<u8>),
    /// Out-of-band signal
    Signal(BnetSignal),
}

/// Errors specific to the BNET protocol.
#[derive(Debug, thiserror::Error)]
pub enum BnetError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("unknown BNET signal: {0}")]
    UnknownSignal(i32),
    #[error("packet too large: {0} bytes (max {MAX_PACKET_SIZE})")]
    PacketTooLarge(usize),
    #[error("connection terminated by peer")]
    Terminated,
    #[error("hard EOF")]
    HardEof,
}

/// A BNET connection over any Read+Write stream.
pub struct BnetStream<S> {
    stream: S,
    /// Tracks whether the peer sent BNET_TERMINATE.
    terminated: bool,
}

impl<S: Read + Write> BnetStream<S> {
    /// Wrap an existing stream.
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            terminated: false,
        }
    }

    /// Send a data message (may be split into multiple packets if > MAX_MESSAGE_LEN).
    pub fn send(&mut self, data: &[u8]) -> Result<(), BnetError> {
        if data.is_empty() {
            // Empty message: send zero-length header
            self.stream.write_all(&0i32.to_be_bytes())?;
            self.stream.flush()?;
            return Ok(());
        }

        let mut written = 0usize;
        while written < data.len() {
            let remaining = data.len() - written;
            let chunk_len = remaining.min(MAX_MESSAGE_LEN);
            let header = (chunk_len as i32).to_be_bytes();
            self.stream.write_all(&header)?;
            self.stream.write_all(&data[written..written + chunk_len])?;
            written += chunk_len;
        }
        self.stream.flush()?;
        Ok(())
    }

    /// Send a signal (negative length, no payload).
    pub fn send_signal(&mut self, signal: BnetSignal) -> Result<(), BnetError> {
        let header = (signal as i32).to_be_bytes();
        self.stream.write_all(&header)?;
        self.stream.flush()?;
        Ok(())
    }

    /// Receive the next message.
    pub fn recv(&mut self) -> Result<BnetMessage, BnetError> {
        let mut header_buf = [0u8; 4];
        match self.stream.read_exact(&mut header_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(BnetError::HardEof);
            }
            Err(e) => return Err(BnetError::Io(e)),
        }

        let len = i32::from_be_bytes(header_buf);

        if len < 0 {
            // Signal
            if len == BnetSignal::Terminate as i32 {
                self.terminated = true;
            }
            match BnetSignal::from_i32(len) {
                Some(sig) => Ok(BnetMessage::Signal(sig)),
                None => Err(BnetError::UnknownSignal(len)),
            }
        } else if len == 0 {
            // Empty message
            Ok(BnetMessage::Data(Vec::new()))
        } else {
            let len = len as usize;
            if len > MAX_PACKET_SIZE {
                return Err(BnetError::PacketTooLarge(len));
            }
            let mut buf = vec![0u8; len];
            self.stream.read_exact(&mut buf)?;
            Ok(BnetMessage::Data(buf))
        }
    }

    /// Send a UTF-8 string as data message.
    pub fn send_str(&mut self, s: &str) -> Result<(), BnetError> {
        self.send(s.as_bytes())
    }

    /// Receive a data message as UTF-8 string.
    /// Returns error on signal or invalid UTF-8.
    pub fn recv_str(&mut self) -> Result<String, BnetError> {
        match self.recv()? {
            BnetMessage::Data(data) => String::from_utf8(data).map_err(|e| {
                BnetError::Io(io::Error::new(io::ErrorKind::InvalidData, e))
            }),
            BnetMessage::Signal(BnetSignal::Terminate) => Err(BnetError::Terminated),
            BnetMessage::Signal(sig) => Err(BnetError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("expected data, got signal: {sig:?}"),
            ))),
        }
    }

    /// Whether the peer has sent BNET_TERMINATE.
    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// Access the underlying stream.
    pub fn inner(&self) -> &S {
        &self.stream
    }

    /// Access the underlying stream mutably.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    /// Consume and return the underlying stream.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    fn make_pair() -> (BnetStream<TcpStream>, BnetStream<TcpStream>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        (BnetStream::new(client), BnetStream::new(server))
    }

    #[test]
    fn data_roundtrip() {
        let (mut a, mut b) = make_pair();
        a.send(b"hello bareos").unwrap();
        assert_eq!(b.recv().unwrap(), BnetMessage::Data(b"hello bareos".to_vec()));
    }

    #[test]
    fn signal_roundtrip() {
        let (mut a, mut b) = make_pair();
        a.send_signal(BnetSignal::Eod).unwrap();
        assert_eq!(b.recv().unwrap(), BnetMessage::Signal(BnetSignal::Eod));
    }

    #[test]
    fn string_roundtrip() {
        let (mut a, mut b) = make_pair();
        a.send_str("status dir").unwrap();
        assert_eq!(b.recv_str().unwrap(), "status dir");
    }

    #[test]
    fn empty_message() {
        let (mut a, mut b) = make_pair();
        a.send(b"").unwrap();
        assert_eq!(b.recv().unwrap(), BnetMessage::Data(Vec::new()));
    }

    #[test]
    fn multiple_messages() {
        let (mut a, mut b) = make_pair();
        a.send_str("msg1").unwrap();
        a.send_str("msg2").unwrap();
        a.send_signal(BnetSignal::Eod).unwrap();

        assert_eq!(b.recv_str().unwrap(), "msg1");
        assert_eq!(b.recv_str().unwrap(), "msg2");
        assert_eq!(b.recv().unwrap(), BnetMessage::Signal(BnetSignal::Eod));
    }

    #[test]
    fn terminate_sets_flag() {
        let (mut a, mut b) = make_pair();
        assert!(!b.is_terminated());
        a.send_signal(BnetSignal::Terminate).unwrap();
        let msg = b.recv().unwrap();
        assert_eq!(msg, BnetMessage::Signal(BnetSignal::Terminate));
        assert!(b.is_terminated());
    }

    #[test]
    fn large_message_splitting() {
        let (mut a, mut b) = make_pair();
        // Send a message larger than MAX_MESSAGE_LEN to trigger splitting
        let large_data = vec![0x42u8; MAX_MESSAGE_LEN + 100];
        a.send(&large_data).unwrap();

        // Receiver should get two packets that together form the message
        let msg1 = b.recv().unwrap();
        let msg2 = b.recv().unwrap();

        match (msg1, msg2) {
            (BnetMessage::Data(d1), BnetMessage::Data(d2)) => {
                assert_eq!(d1.len(), MAX_MESSAGE_LEN);
                assert_eq!(d2.len(), 100);
                let mut combined = d1;
                combined.extend_from_slice(&d2);
                assert_eq!(combined, large_data);
            }
            _ => panic!("expected two data messages"),
        }
    }

    #[test]
    fn all_console_signals() {
        let (mut a, mut b) = make_pair();
        let signals = [
            BnetSignal::MainPrompt,
            BnetSignal::SubPrompt,
            BnetSignal::TextInput,
            BnetSignal::YesNo,
            BnetSignal::StartSelect,
            BnetSignal::EndSelect,
            BnetSignal::CmdOk,
            BnetSignal::CmdFailed,
            BnetSignal::ErrorMsg,
            BnetSignal::WarningMsg,
            BnetSignal::InfoMsg,
        ];
        for sig in &signals {
            a.send_signal(*sig).unwrap();
        }
        for sig in &signals {
            assert_eq!(b.recv().unwrap(), BnetMessage::Signal(*sig));
        }
    }

    #[test]
    fn hard_eof_on_disconnect() {
        let (a, mut b) = make_pair();
        drop(a); // Close sender
        match b.recv() {
            Err(BnetError::HardEof) => {} // expected
            Err(BnetError::Io(_)) => {}   // also acceptable
            other => panic!("expected HardEof or Io error, got {other:?}"),
        }
    }
}
