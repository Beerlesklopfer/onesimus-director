//! gRPC control transport for the native File Daemon engine. The adapter keeps
//! its synchronous message API local; only protobuf frames cross the network.
use std::io::{self, Read, Write};
use std::time::Duration;

use onesimus_proto_rs::onesimus::filed::{
    ControlFrame, control_frame::Frame, file_daemon_client::FileDaemonClient,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};

const QUEUE: usize = 8;
const MAX_FRAME: usize = crate::signal::MAX_PACKET_SIZE;

pub struct ControlIo {
    incoming: flume::Receiver<io::Result<ControlFrame>>,
    outgoing: mpsc::Sender<ControlFrame>,
    read_buffer: std::io::Cursor<Vec<u8>>,
    write_buffer: Vec<u8>,
    read_timeout: Option<Duration>,
    pub cancel: Option<&'static std::sync::atomic::AtomicBool>,
    pub authenticated: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

pub struct ControlPeer {
    pub incoming: flume::Sender<io::Result<ControlFrame>>,
    pub outgoing: mpsc::Receiver<ControlFrame>,
}

pub fn pair(timeout: Option<Duration>) -> (ControlIo, ControlPeer) {
    let (incoming_tx, incoming_rx) = flume::bounded(QUEUE);
    let (outgoing_tx, outgoing_rx) = mpsc::channel(QUEUE);
    (
        ControlIo {
            incoming: incoming_rx,
            outgoing: outgoing_tx,
            read_buffer: std::io::Cursor::new(Vec::new()),
            write_buffer: Vec::new(),
            read_timeout: timeout,
            cancel: None,
            authenticated: Default::default(),
        },
        ControlPeer {
            incoming: incoming_tx,
            outgoing: outgoing_rx,
        },
    )
}

impl ControlIo {
    pub fn set_read_timeout(&mut self, timeout: Option<Duration>) {
        self.read_timeout = timeout;
        self.authenticated
            .store(timeout.is_none(), std::sync::atomic::Ordering::Relaxed);
    }
}

impl Read for ControlIo {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.read_buffer.position() as usize == self.read_buffer.get_ref().len() {
            let timeout = if self
                .authenticated
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                None
            } else {
                self.read_timeout
            };
            let started = std::time::Instant::now();
            let frame = loop {
                if self
                    .cancel
                    .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "gRPC control canceled",
                    ));
                }
                let wait = timeout
                    .map(|limit| limit.saturating_sub(started.elapsed()))
                    .unwrap_or(Duration::from_millis(100))
                    .min(Duration::from_millis(100));
                match self.incoming.recv_timeout(wait) {
                    Ok(frame) => break frame?,
                    Err(flume::RecvTimeoutError::Disconnected) => return Ok(0),
                    Err(flume::RecvTimeoutError::Timeout) => {
                        if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                            return Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "gRPC control read timeout",
                            ));
                        }
                    }
                }
            };
            let bytes = match frame.frame {
                Some(Frame::Payload(payload)) if payload.len() <= MAX_FRAME => {
                    let mut bytes = (payload.len() as i32).to_be_bytes().to_vec();
                    bytes.extend(payload);
                    bytes
                }
                Some(Frame::Signal(signal))
                    if signal < 0 && crate::BnetSignal::from_i32(signal).is_some() =>
                {
                    signal.to_be_bytes().to_vec()
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid gRPC control frame",
                    ));
                }
            };
            self.read_buffer = std::io::Cursor::new(bytes);
        }
        self.read_buffer.read(buf)
    }
}

impl Write for ControlIo {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // BnetStream writes a header and its bounded payload separately.
        if self.write_buffer.len().saturating_add(buf.len()) > MAX_FRAME + 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gRPC control frame too large",
            ));
        }
        self.write_buffer.extend_from_slice(buf);
        while self.write_buffer.len() >= 4 {
            let len = i32::from_be_bytes(self.write_buffer[..4].try_into().unwrap());
            let size = if len <= 0 { 4 } else { len as usize + 4 };
            if size > MAX_FRAME + 4 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "gRPC control frame too large",
                ));
            }
            if self.write_buffer.len() < size {
                break;
            }
            let frame = if len < 0 {
                Frame::Signal(len)
            } else {
                Frame::Payload(self.write_buffer[4..size].to_vec())
            };
            let mut message = ControlFrame { frame: Some(frame) };
            let started = std::time::Instant::now();
            loop {
                if self
                    .cancel
                    .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::Relaxed))
                {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "gRPC control canceled",
                    ));
                }
                match self.outgoing.try_send(message) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "gRPC control closed",
                        ));
                    }
                    Err(mpsc::error::TrySendError::Full(frame)) => message = frame,
                }
                if started.elapsed() >= Duration::from_secs(15) {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "gRPC control write timeout",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            self.write_buffer.drain(..size);
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// TLS is mandatory; CA and DNS identity are supplied by the Director's client
/// resource. Mutual CRAM authentication follows on the returned control stream.
pub fn connect(
    host: &str,
    port: u16,
    ca: Vec<u8>,
    server_name: &str,
    timeout: Option<Duration>,
) -> io::Result<ControlIo> {
    let authority = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let endpoint = Endpoint::from_shared(format!("https://{authority}:{port}"))
        .map_err(io::Error::other)?
        .connect_timeout(Duration::from_secs(5))
        .tls_config(
            ClientTlsConfig::new()
                .ca_certificate(Certificate::from_pem(ca))
                .domain_name(server_name),
        )
        .map_err(io::Error::other)?;
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("fd:grpc-client".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            rt.block_on(async move {
                let (io, peer) = pair(timeout);
                let setup = async {
                    let channel = endpoint.connect().await.map_err(io::Error::other)?;
                    let mut client = FileDaemonClient::new(channel)
                        .max_decoding_message_size(MAX_FRAME + 64)
                        .max_encoding_message_size(MAX_FRAME + 64);
                    client
                        .control(ReceiverStream::new(peer.outgoing))
                        .await
                        .map(|r| r.into_inner())
                        .map_err(io::Error::other)
                };
                let mut stream = match tokio::time::timeout(Duration::from_secs(10), setup).await {
                    Ok(Ok(stream)) => stream,
                    result => {
                        let error = match result {
                            Ok(Err(e)) => e,
                            _ => io::Error::new(
                                io::ErrorKind::TimedOut,
                                "gRPC control connect timeout",
                            ),
                        };
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                if ready_tx.send(Ok(io)).is_err() {
                    return;
                }
                loop {
                    // Dropping the synchronous stream cancels even a quiet RPC.
                    let result = tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {
                            if peer.incoming.is_disconnected() { break; }
                            continue;
                        },
                        result = stream.message() => result,
                    };
                    match result {
                        Ok(Some(frame)) => {
                            if peer.incoming.send_async(Ok(frame)).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(status) => {
                            let _ = peer
                                .incoming
                                .send_async(Err(io::Error::other(status)))
                                .await;
                            break;
                        }
                    }
                }
            });
        })?;
    ready_rx.recv().map_err(io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BnetMessage, BnetSignal, BnetStream};

    #[test]
    fn frames_preserve_binary_payload_signals_and_partial_reads() {
        let (io, mut peer) = pair(None);
        let mut stream = BnetStream::new(io);
        stream.send(&[0, 255, 1, 0, 128]).unwrap();
        assert_eq!(
            peer.outgoing.try_recv().unwrap().frame,
            Some(Frame::Payload(vec![0, 255, 1, 0, 128]))
        );
        stream.send_signal(BnetSignal::Eod).unwrap();
        assert_eq!(
            peer.outgoing.try_recv().unwrap().frame,
            Some(Frame::Signal(BnetSignal::Eod as i32))
        );
        peer.incoming
            .send(Ok(ControlFrame {
                frame: Some(Frame::Payload(vec![0, 255, 4])),
            }))
            .unwrap();
        assert_eq!(stream.recv().unwrap(), BnetMessage::Data(vec![0, 255, 4]));
        peer.incoming
            .send(Ok(ControlFrame {
                frame: Some(Frame::Signal(BnetSignal::Eod as i32)),
            }))
            .unwrap();
        assert_eq!(stream.recv().unwrap(), BnetMessage::Signal(BnetSignal::Eod));
    }

    #[test]
    fn malformed_frames_and_disconnect_are_errors() {
        for frame in [
            None,
            Some(Frame::Signal(1)),
            Some(Frame::Signal(-99999)),
            Some(Frame::Payload(vec![0; MAX_FRAME + 1])),
        ] {
            let (io, peer) = pair(None);
            peer.incoming.send(Ok(ControlFrame { frame })).unwrap();
            let mut stream = BnetStream::new(io);
            assert!(stream.recv().is_err());
        }
        let (io, peer) = pair(Some(Duration::from_millis(1)));
        let mut stream = BnetStream::new(io);
        assert!(stream.recv().is_err());
        drop(peer);
        assert!(stream.recv().is_err());
    }
}
