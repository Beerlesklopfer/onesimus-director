// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Die Serverseite: eingehende Ströme und der gRPC-Server eines Daemons.
//!
//! Zweck: Ein Dienst (ofd: FileDaemon, osd: StorageDaemon) bekommt in jeder
//! RPC einen eingehenden Strom. [`accept`] macht daraus einen
//! [`ControlIo`] für die synchrone Sitzung und den Antwortstrom, den die
//! RPC zurückgibt. [`spawn`] startet den `tonic`-Server mit TLS auf einem
//! eigenen Thread und hält ihn an, sobald der Abschaltmerker gesetzt ist.
//! Der Port wird vor dem Start gebunden, ein belegter Port ist also ein
//! Fehler des Aufrufs, nicht des Threads.
//!
//! Herkunft: `ofd/src/grpc.rs` (`Service::control`, `start`), am 20.11.2026
//! hierher verschoben, damit ofd und osd denselben Server haben.
//!
//! Bareos-Gegenstück: `core/src/lib/bnet_server_tcp.cc`
//! (`BnetThreadServerTcp`) für den BNET-Port.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
use tonic::transport::server::Router;
use tonic::transport::{Server, ServerTlsConfig};

use crate::frame::{Frame, WireFrame};
use crate::io::{ControlIo, pair};

/// The response stream of a bidirectional RPC.
pub type ResponseStream<M> = Pin<Box<dyn tokio_stream::Stream<Item = Result<M, Status>> + Send>>;

/// How often a waiting pump checks whether the synchronous side is gone.
const POLL: Duration = Duration::from_millis(100);

/// Turns the incoming stream of an RPC into a [`ControlIo`] (reads limited
/// to `timeout` until authenticated) and the response stream to return.
/// Must run inside the tokio runtime of the server (spawns the pump).
pub fn accept<M: WireFrame>(input: tonic::Streaming<M>, timeout: Option<Duration>) -> (ControlIo, ResponseStream<M>) {
    let (io, peer) = pair(timeout);
    tokio::spawn(pump_incoming(input, peer.incoming));
    let output = ReceiverStream::new(peer.outgoing).map(|frame| Ok(M::from_frame(frame)));
    (io, Box::pin(output))
}

/// Moves messages from `input` to `incoming` until either side ends; a
/// gRPC error or an empty message ends the stream with an error.
pub(crate) async fn pump_incoming<M: WireFrame>(
    mut input: tonic::Streaming<M>,
    incoming: flume::Sender<io::Result<Frame>>,
) {
    loop {
        let result = tokio::select! {
            _ = tokio::time::sleep(POLL) => {
                // Dropping the synchronous stream cancels even a quiet RPC.
                if incoming.is_disconnected() { break; }
                continue;
            },
            result = input.message() => result,
        };
        match result {
            Ok(Some(message)) => {
                let frame = message
                    .into_frame()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid gRPC frame"));
                let failed = frame.is_err();
                if incoming.send_async(frame).await.is_err() || failed {
                    break;
                }
            }
            Ok(None) => break,
            Err(status) => {
                let _ = incoming.send_async(Err(io::Error::other(status))).await;
                break;
            }
        }
    }
}

/// Starts a TLS gRPC server on `address` in thread `name`; `services`
/// adds the services. Returns the bound address and the thread, which ends
/// after `shutdown` is set.
pub fn spawn<F>(
    name: &str,
    address: SocketAddr,
    tls: ServerTlsConfig,
    shutdown: &'static AtomicBool,
    services: F,
) -> Result<(SocketAddr, std::thread::JoinHandle<()>), String>
where
    F: FnOnce(&mut Server) -> Router + Send + 'static,
{
    crate::install_crypto_provider();
    let mut builder = Server::builder().tls_config(tls).map_err(|e| e.to_string())?;
    // Bind before reporting a successful start (also catches port conflicts).
    let listener = bareos_bnet::listen::bind(address).map_err(|e| format!("{address}: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let label = name.to_string();
    let handle = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!("{label}: no runtime: {e}");
                    return;
                }
            };
            rt.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("{label}: {e}");
                        return;
                    }
                };
                let serve = services(&mut builder)
                    .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
                tokio::select! {
                    result = serve => if let Err(error) = result {
                        tracing::error!("{label}: {error}");
                    },
                    _ = async {
                        while !shutdown.load(Ordering::Relaxed) {
                            tokio::time::sleep(POLL).await;
                        }
                    } => {},
                }
            });
            rt.shutdown_timeout(Duration::from_secs(2));
        })
        .map_err(|e| e.to_string())?;
    Ok((address, handle))
}
