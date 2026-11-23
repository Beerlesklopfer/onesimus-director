// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! gRPC-first File Daemon connections with an unconditional BNET fallback.
use crate::director_config::DirectorConfig;
use bareos_bnet::{AnyStream, BnetStream, CramMd5Auth};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::Duration;

/// The gRPC endpoint of a Client or Storage resource (Onesimus directives
/// `Grpc Enable`, `Grpc Port`, `Grpc CA Certificate File`, `Grpc Server
/// Name`), snapshotted into job parameters.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Target {
    /// gRPC port (default: the BNET port + 1000).
    pub(crate) port: u16,
    /// PEM file of the CA (or pinned self-signed certificate) to trust.
    pub(crate) ca: String,
    /// Name the server certificate must carry (default: the address).
    pub(crate) server_name: String,
}

impl Target {
    /// The target of resource `name` of type `kind` (`client`, `storage`)
    /// at `address:port`; `None` unless `Grpc Enable = yes`.
    pub(crate) fn from_directives(config: &DirectorConfig, kind: &str, name: &str, address: &str, port: u16) -> Option<Target> {
        let value = |key: &str| crate::job_messages::raw_directive(config, kind, name, key);
        if !value("grpcenable").is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "yes" | "true" | "1")) {
            return None;
        }
        Some(Target {
            port: value("grpcport").and_then(|p| p.parse().ok()).unwrap_or(port.saturating_add(1000)),
            ca: value("grpccacertificatefile").unwrap_or_default(),
            server_name: value("grpcservername").unwrap_or_else(|| address.to_string()),
        })
    }

    /// The transport target at `host` (reads the CA file).
    pub(crate) fn resolve(&self, host: &str) -> Result<onesimus_bnet_grpc::Target, String> {
        let ca = std::fs::read(&self.ca).map_err(|e| format!("gRPC CA {}: {e}", self.ca))?;
        Ok(onesimus_bnet_grpc::Target { host: host.to_string(), port: self.port, ca, server_name: self.server_name.clone() })
    }
}

static TARGETS: LazyLock<RwLock<HashMap<(String, u16), Target>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Run on startup/reload, before job workers fork. A job inherits its endpoint
/// configuration; existing connections continue unaffected by a later reload.
pub fn configure(config: &DirectorConfig) {
    let mut targets = HashMap::new();
    for client in &config.clients {
        if let Some(target) = for_client(config, client) {
            targets.insert((client.address.clone(), client.port), target);
        }
    }
    *TARGETS.write().unwrap_or_else(|e| e.into_inner()) = targets;
}

/// Snapshot into job parameters before fork. Workers never acquire a parent
/// registry lock that another thread could have held at fork time.
pub fn for_client(
    config: &DirectorConfig,
    client: &bareos_config::ClientResource,
) -> Option<Target> {
    Target::from_directives(config, "client", &client.name, &client.address, client.port)
}

pub fn try_worker(
    params: &crate::job_worker::JobParams,
) -> Option<(BnetStream<AnyStream>, String)> {
    connect_target(
        params.fd_grpc.as_ref()?,
        &params.fd_address,
        params.fd_port,
        &params.fd_password,
        &params.director_name,
        None,
    )
}

/// Never probe gRPC for an unconfigured legacy client. TLS, service or login
/// failure logs the reason and returns None so callers take their BNET path.
pub fn try_connect(
    address: &str,
    port: u16,
    password: &str,
    director: &str,
    timeout: Option<Duration>,
) -> Option<(BnetStream<AnyStream>, String)> {
    let target = TARGETS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(&(address.to_string(), port))
        .cloned()?;
    connect_target(&target, address, port, password, director, timeout)
}

fn connect_target(
    target: &Target,
    address: &str,
    port: u16,
    password: &str,
    director: &str,
    timeout: Option<Duration>,
) -> Option<(BnetStream<AnyStream>, String)> {
    let connect = || -> Result<_, String> {
        let grpc = target.resolve(address)?;
        let mut io = onesimus_bnet_grpc::client::connect(
            &grpc,
            onesimus_bnet_grpc::Rpc::FileDaemonControl,
            Some(Duration::from_secs(15)),
        )
        .map_err(|e| e.to_string())?;
        io.cancel = Some(&crate::job_worker::WORKER_CANCEL);
        // The stream stays authenticated-with-limit until the FD's OK Hello.
        let mut stream = BnetStream::new(io);
        stream
            .send_str(&bareos_bnet::hello::make_director_hello(director))
            .map_err(|e| e.to_string())?;
        if !CramMd5Auth::new(password, director, 2)
            .two_way_authenticate(&mut stream, false)
            .map_err(|e| e.to_string())?
            .success
        {
            return Err("gRPC FD authentication failed".into());
        }
        let hello = stream.recv_str().map_err(|e| e.to_string())?;
        if !hello.starts_with(bareos_bnet::hello::FD_OK_HELLO) {
            return Err("invalid gRPC FD hello".into());
        }
        stream.inner_mut().set_read_timeout(timeout);
        let stream = BnetStream::new(AnyStream::External(Box::new(stream.into_inner())));
        Ok((stream, onesimus_bnet_grpc::TRANSPORT.to_string()))
    };
    match connect() {
        Ok(result) => {
            tracing::info!("FD {address}:{}: connected over gRPC TLS", target.port);
            Some(result)
        }
        Err(error) => {
            tracing::warn!(
                "FD {address}:{}: {error}; falling back to BNET on port {port}",
                target.port
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bareos_bnet::BnetSignal;
    use std::net::TcpListener;

    #[test]
    fn grpc_failure_falls_back_to_authenticated_bnet() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Configuration parsing must retain all Onesimus directives.
        let config = DirectorConfig::from_str(&format!("Director {{\n Name = fallback-dir\n Password = pw\n}}\nClient {{\n Name = grpc-fallback-test\n Address = 127.0.0.1\n Port = {port}\n Password = pw\n Grpc Enable = yes\n Grpc Port = 1\n Grpc CA Certificate File = /missing/test-ca.pem\n Grpc Server Name = localhost\n}}\n")).unwrap();
        // Install only this unique endpoint, leaving other tests' endpoints alone.
        let client = &config.clients[0];
        let value =
            crate::job_messages::raw_directive(&config, "client", &client.name, "grpcenable");
        assert_eq!(value.as_deref(), Some("yes"));
        TARGETS.write().unwrap().insert(
            (client.address.clone(), port),
            Target {
                port: 1,
                ca: "/missing/test-ca.pem".into(),
                server_name: "localhost".into(),
            },
        );
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bnet = BnetStream::new(socket);
            assert_eq!(
                bnet.recv_str().unwrap(),
                "Hello Director fallback-dir calling\n"
            );
            assert!(
                CramMd5Auth::new("pw", "fd", 0)
                    .two_way_authenticate(&mut bnet, true)
                    .unwrap()
                    .success
            );
            bnet.send_str("2000 OK Hello 54\n").unwrap();
            assert_eq!(bnet.recv_str().unwrap(), "status");
            bnet.send_str("fallback status\n").unwrap();
            bnet.send_signal(BnetSignal::Eod).unwrap();
            let _ = bnet.recv();
        });
        let (_, connection) = crate::fd_protocol::connect_fd_console(
            "127.0.0.1",
            port,
            "pw",
            "fallback-dir",
            "grpc-fallback-test",
            (false, false),
        );
        let output =
            crate::fd_protocol::fd_query_commands(connection.unwrap(), &["status".into()]).unwrap();
        assert_eq!(output, "fallback status\n");
        server.join().unwrap();
        TARGETS.write().unwrap().remove(&("127.0.0.1".into(), port));
    }
}
