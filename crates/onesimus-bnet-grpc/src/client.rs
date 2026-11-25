// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Der Client: eine TLS-Verbindung zu FileDaemon oder StorageDaemon.
//!
//! Zweck: [`connect`] öffnet einen der vier Ströme ([`Rpc`]) und gibt ihn
//! als [`ControlIo`] zurück. TLS ist Pflicht. Vertraut wird genau der CA
//! aus [`Target::ca`] (beim selbstsignierten Zertifikat eines Daemons ist
//! das sein eigenes Zertifikat, also festgehalten), geprüft wird der Name
//! [`Target::server_name`]. Danach folgen im Strom Hello und CRAM-MD5 wie
//! über BNET; erst sie melden die Gegenstelle an.
//!
//! Jede Verbindung bekommt einen eigenen Thread mit einer
//! `current_thread`-Laufzeit, die den Strom pumpt; sie endet, wenn eine
//! Seite schließt oder der [`ControlIo`] fallen gelassen wird.
//!
//! Herkunft: `bareos-bnet/src/fd_grpc.rs` `connect` (nur FileDaemon/Control),
//! seit 20.11.2026 für alle vier Ströme.
//!
//! Bareos-Gegenstück: keins; der Aufbau ersetzt `BareosSocketTCP::connect`
//! und den TLS-PSK-Handshake (`lib/bsock.cc`).

use std::future::Future;
use std::io;
use std::time::Duration;

use onesimus_proto_rs::onesimus::filed::{ControlFrame, file_daemon_client::FileDaemonClient};
use onesimus_proto_rs::onesimus::stored::{StoredFrame, storage_daemon_client::StorageDaemonClient};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

use crate::frame::{Frame, WireFrame};
use crate::io::{ControlIo, MAX_MESSAGE, pair};

/// How long the TCP connection may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long TCP, TLS and the RPC call together may take.
const SETUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The gRPC stream to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rpc {
    /// `onesimus.filed.FileDaemon/Control`: Director → FD.
    FileDaemonControl,
    /// `onesimus.filed.FileDaemon/Data`: SD → FD (passive client).
    FileDaemonData,
    /// `onesimus.stored.StorageDaemon/Control`: Director → SD.
    StorageDaemonControl,
    /// `onesimus.stored.StorageDaemon/Data`: FD → SD (active client).
    StorageDaemonData,
}

impl Rpc {
    /// The gRPC path, for log lines.
    pub fn path(self) -> &'static str {
        match self {
            Rpc::FileDaemonControl => "onesimus.filed.FileDaemon/Control",
            Rpc::FileDaemonData => "onesimus.filed.FileDaemon/Data",
            Rpc::StorageDaemonControl => "onesimus.stored.StorageDaemon/Control",
            Rpc::StorageDaemonData => "onesimus.stored.StorageDaemon/Data",
        }
    }
}

/// Where and whom to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Host name or address (IPv6 without brackets).
    pub host: String,
    /// The gRPC port.
    pub port: u16,
    /// PEM of the CA (or the pinned self-signed certificate) to trust.
    pub ca: Vec<u8>,
    /// The name the server certificate must carry.
    pub server_name: String,
}

impl Target {
    /// The tokens odir appends to `storage address=…` (to ofd) and
    /// `passive client address=…` (to osd) when the other side offers
    /// gRPC: `grpcport=<port> grpcname=<server name> grpcca=<PEM, base64>`.
    /// The host is the address of the command itself. Only Onesimus
    /// daemons get them; bareos-fd and bareos-sd never see them.
    pub fn to_tokens(&self) -> String {
        use base64::Engine;
        format!(
            "grpcport={} grpcname={} grpcca={}",
            self.port,
            self.server_name,
            base64::engine::general_purpose::STANDARD.encode(&self.ca)
        )
    }

    /// The target of the tokens in `line` ([`Target::to_tokens`]) at
    /// `host`; `None` unless all three are there and valid.
    pub fn from_tokens(host: &str, line: &str) -> Option<Target> {
        use base64::Engine;
        let field = |key: &str| line.split_whitespace().find_map(|w| w.strip_prefix(key));
        let port = field("grpcport=")?.parse().ok().filter(|&p: &u16| p != 0)?;
        let server_name = field("grpcname=").filter(|n| !n.is_empty())?.to_string();
        let ca = base64::engine::general_purpose::STANDARD.decode(field("grpcca=")?).ok()?;
        Some(Target { host: host.to_string(), port, ca, server_name })
    }
}

/// The outgoing message stream of one RPC.
type Outgoing<M> = tokio_stream::adapters::Map<ReceiverStream<Frame>, fn(Frame) -> M>;

/// Opens `rpc` at `target`; `timeout` limits reads until the caller marks
/// the stream authenticated ([`ControlIo::set_read_timeout`] with `None`).
pub fn connect(target: &Target, rpc: Rpc, timeout: Option<Duration>) -> io::Result<ControlIo> {
    crate::install_crypto_provider();
    let authority = if target.host.contains(':') { format!("[{}]", target.host) } else { target.host.clone() };
    let endpoint = Endpoint::from_shared(format!("https://{authority}:{}", target.port))
        .map_err(io::Error::other)?
        .connect_timeout(CONNECT_TIMEOUT)
        .tls_config(
            ClientTlsConfig::new().ca_certificate(Certificate::from_pem(&target.ca)).domain_name(&target.server_name),
        )
        .map_err(io::Error::other)?;
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new().name("grpc-client".into()).spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        rt.block_on(async move {
            match rpc {
                Rpc::FileDaemonControl => {
                    run::<ControlFrame, _, _>(endpoint, timeout, ready_tx, |ch, out| async move {
                        FileDaemonClient::new(ch)
                            .max_decoding_message_size(MAX_MESSAGE)
                            .max_encoding_message_size(MAX_MESSAGE)
                            .control(out)
                            .await
                    })
                    .await
                }
                Rpc::FileDaemonData => {
                    run::<ControlFrame, _, _>(endpoint, timeout, ready_tx, |ch, out| async move {
                        FileDaemonClient::new(ch)
                            .max_decoding_message_size(MAX_MESSAGE)
                            .max_encoding_message_size(MAX_MESSAGE)
                            .data(out)
                            .await
                    })
                    .await
                }
                Rpc::StorageDaemonControl => {
                    run::<StoredFrame, _, _>(endpoint, timeout, ready_tx, |ch, out| async move {
                        StorageDaemonClient::new(ch)
                            .max_decoding_message_size(MAX_MESSAGE)
                            .max_encoding_message_size(MAX_MESSAGE)
                            .control(out)
                            .await
                    })
                    .await
                }
                Rpc::StorageDaemonData => {
                    run::<StoredFrame, _, _>(endpoint, timeout, ready_tx, |ch, out| async move {
                        StorageDaemonClient::new(ch)
                            .max_decoding_message_size(MAX_MESSAGE)
                            .max_encoding_message_size(MAX_MESSAGE)
                            .data(out)
                            .await
                    })
                    .await
                }
            }
        });
    })?;
    ready_rx.recv().map_err(io::Error::other)?
}

/// `error` with all its sources ("transport error: tcp connect error:
/// Connection refused …"), so a log line says why gRPC failed.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        let part = e.to_string();
        if !text.contains(&part) {
            text = format!("{text}: {part}");
        }
        source = e.source();
    }
    text
}

/// Connects, calls the RPC with `call`, hands the [`ControlIo`] to the
/// caller through `ready` and pumps the incoming messages until either
/// side closes.
async fn run<M, F, Fut>(
    endpoint: Endpoint,
    timeout: Option<Duration>,
    ready: std::sync::mpsc::SyncSender<io::Result<ControlIo>>,
    call: F,
) where
    M: WireFrame,
    F: FnOnce(Channel, Outgoing<M>) -> Fut,
    Fut: Future<Output = Result<tonic::Response<tonic::Streaming<M>>, tonic::Status>>,
{
    let (io, peer) = pair(timeout);
    let outgoing: Outgoing<M> = ReceiverStream::new(peer.outgoing).map(M::from_frame as fn(Frame) -> M);
    let setup = async {
        let channel = endpoint.connect().await.map_err(|e| io::Error::other(chain(&e)))?;
        call(channel, outgoing).await.map(|r| r.into_inner()).map_err(|e| io::Error::other(chain(&e)))
    };
    let stream = match tokio::time::timeout(SETUP_TIMEOUT, setup).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            let _ = ready.send(Err(e));
            return;
        }
        Err(_) => {
            let _ = ready.send(Err(io::Error::new(io::ErrorKind::TimedOut, "gRPC connect timeout")));
            return;
        }
    };
    if ready.send(Ok(io)).is_err() {
        return;
    }
    crate::server::pump_incoming(stream, peer.incoming).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip_and_ignore_the_bareos_part() {
        let target = Target { host: "sd.example".into(), port: 10103, ca: b"-----BEGIN CERTIFICATE-----\nx\n".to_vec(), server_name: "sd.example".into() };
        let line = format!("storage address=sd.example port=9103 ssl=4 Authorization=KEY {}\n", target.to_tokens());
        assert_eq!(Target::from_tokens("sd.example", &line), Some(target));
        assert_eq!(Target::from_tokens("x", "storage address=x port=9103 ssl=4 Authorization=KEY"), None);
        assert_eq!(Target::from_tokens("x", "passive client address=x port=1 ssl=4 grpcport=0 grpcname=a grpcca=AA=="), None);
        assert_eq!(Target::from_tokens("x", "grpcport=5 grpcname=a grpcca=!!"), None);
    }

    #[test]
    fn a_closed_port_fails_fast() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let target = Target { host: "127.0.0.1".into(), port, ca: Vec::new(), server_name: "localhost".into() };
        let started = std::time::Instant::now();
        assert!(connect(&target, Rpc::StorageDaemonControl, None).is_err());
        assert!(started.elapsed() < SETUP_TIMEOUT + Duration::from_secs(1));
    }
}
