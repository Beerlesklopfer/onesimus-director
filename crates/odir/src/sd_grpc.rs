// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! gRPC-first Storage Daemon connections (osd) with an unconditional BNET
//! fallback, the counterpart of `fd_grpc` for Storage resources.
//!
//! A Storage resource opts in with the same Onesimus directives as a
//! Client (`Grpc Enable = yes`, `Grpc Port` (default SD Port + 1000),
//! `Grpc CA Certificate File`, `Grpc Server Name`). Then every path to the
//! SD first tries `onesimus.stored.StorageDaemon/Control` with TLS; Hello,
//! CRAM-MD5 and the OK-Hello run inside the stream
//! (`osd::connect::handshake`). Any failure (no listener, e.g. bareos-sd;
//! TLS; login) is logged and the caller takes BNET with its unchanged
//! error texts. Never probes a Storage without `Grpc Enable`.
//!
//! - Job paths (`job_worker::connect_sd`, `backup_worker`): [`try_connect`]
//!   with the target snapshotted into the job parameters before fork.
//! - Command paths (`status storage`, `label`, `setdebug` …): the dialer
//!   registered with `osd::connect::set_grpc_dialer` ([`configure`]).
//! - The data connection FD ↔ SD: [`tokens`] gives the SD's target to an
//!   ofd in the storage command (active client) and the FD's target to osd
//!   in the passive client command. Only Onesimus daemons on gRPC get them.
use crate::director_config::DirectorConfig;
use crate::fd_grpc::Target;
use bareos_bnet::{AnyStream, BnetStream};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::Duration;

static TARGETS: LazyLock<RwLock<HashMap<(String, u16), Target>>> = LazyLock::new(|| RwLock::new(HashMap::new()));

/// Run on startup/reload, before job workers fork (as `fd_grpc::configure`).
pub fn configure(config: &DirectorConfig) {
    let mut targets = HashMap::new();
    for storage in &config.storages {
        if let Some(target) = for_storage(config, storage) {
            targets.insert((storage.address.clone(), storage.port), target);
        }
    }
    *TARGETS.write().unwrap_or_else(|e| e.into_inner()) = targets;
    osd::connect::set_grpc_dialer(dial);
}

/// The gRPC target of `storage`, `None` without `Grpc Enable = yes`.
pub fn for_storage(config: &DirectorConfig, storage: &bareos_config::resources::StorageResource) -> Option<Target> {
    Target::from_directives(config, "storage", &storage.name, &storage.address, storage.port)
}

/// The dialer of `osd::connect::connect_sd` (command paths).
fn dial(address: &str, port: u16, password: &str, director: &str) -> Option<osd::connect::SdStream> {
    let target = TARGETS.read().unwrap_or_else(|e| e.into_inner()).get(&(address.to_string(), port)).cloned()?;
    try_connect(Some(&target), address, port, password, director, Some(osd::connect::COMMAND_TIMEOUT))
}

/// `StorageDaemon/Control` to `address` and the Director handshake;
/// `read_timeout_after` applies once authenticated (`None`: jobs wait for
/// mounts as long as it takes).
pub fn connect(
    target: &Target,
    address: &str,
    password: &str,
    director: &str,
    read_timeout_after: Option<Duration>,
) -> Result<osd::connect::SdStream, String> {
    let grpc = target.resolve(address)?;
    let mut io = onesimus_bnet_grpc::client::connect(
        &grpc,
        onesimus_bnet_grpc::Rpc::StorageDaemonControl,
        Some(osd::connect::HANDSHAKE_TIMEOUT),
    )
    .map_err(|e| e.to_string())?;
    io.cancel = Some(&crate::job_worker::WORKER_CANCEL);
    let mut bnet = BnetStream::new(io);
    osd::connect::handshake(&mut bnet, password, director).map_err(|e| e.to_string())?;
    let mut io = bnet.into_inner();
    io.set_read_timeout(read_timeout_after);
    Ok(BnetStream::new(AnyStream::External(Box::new(io))))
}

/// [`connect`] when `target` is set; logs a failure and returns `None` so
/// the caller takes its BNET path.
pub fn try_connect(
    target: Option<&Target>,
    address: &str,
    port: u16,
    password: &str,
    director: &str,
    read_timeout_after: Option<Duration>,
) -> Option<osd::connect::SdStream> {
    let target = target?;
    match connect(target, address, password, director, read_timeout_after) {
        Ok(stream) => {
            tracing::info!("SD {address}:{}: connected over gRPC TLS", target.port);
            Some(stream)
        }
        Err(error) => {
            tracing::warn!("SD {address}:{}: {error}; falling back to BNET on port {port}", target.port);
            None
        }
    }
}

/// The tokens that tell an ofd to reach the SD over gRPC
/// (`onesimus_bnet_grpc::Target::to_tokens`, with a leading space), for the
/// storage command at `host`; empty unless the SD connection itself runs
/// over gRPC (`sd_on_grpc`: the SD is osd) and the FD connection too
/// (`fd_on_grpc`: the FD is ofd and understands them). bareos-fd and
/// bareos-sd never see them.
pub fn tokens(target: Option<&Target>, host: &str, sd_on_grpc: bool, fd_on_grpc: bool) -> String {
    match target {
        Some(target) if sd_on_grpc && fd_on_grpc => match target.resolve(host) {
            Ok(t) => format!(" {}", t.to_tokens()),
            Err(e) => {
                tracing::warn!("gRPC target of {host}: {e}; the data connection uses BNET");
                String::new()
            }
        },
        _ => String::new(),
    }
}

/// Whether `stream` runs over gRPC.
pub fn on_grpc(stream: &BnetStream<AnyStream>) -> bool {
    stream.inner().is_external()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_directives_and_unconfigured_storage() {
        let config = DirectorConfig::from_str(
            "Director {\n Name = d\n Password = pw\n}\nStorage {\n Name = osd1\n Address = sd.example\n SD Port = 9103\n \
             Password = pw\n Device = D\n Media Type = F\n Grpc Enable = yes\n Grpc CA Certificate File = /x/ca.pem\n}\n\
             Storage {\n Name = plain\n Address = 127.0.0.1\n Password = pw\n Device = D\n Media Type = F\n}\n",
        )
        .unwrap();
        let target = for_storage(&config, &config.storages[0]).unwrap();
        assert_eq!(target, Target { port: 10103, ca: "/x/ca.pem".into(), server_name: "sd.example".into() });
        assert_eq!(for_storage(&config, &config.storages[1]), None);
        // An unreachable gRPC target falls back (None), never panics.
        let dead = Target { port: 1, ca: "/missing/ca.pem".into(), server_name: "localhost".into() };
        assert!(try_connect(Some(&dead), "127.0.0.1", 9, "pw", "d", None).is_none());
        assert!(try_connect(None, "127.0.0.1", 9, "pw", "d", None).is_none());
    }
}
