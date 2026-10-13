//! mDNS / DNS-SD announcement — dir:mdns thread.
//!
//! Announces `_onesimus-dir._tcp.local.` so oconsole and the Onesimus GUI
//! find Directors without manual host/port entry.
//!   instance name = Director name
//!   SRV port      = BNET port
//!   TXT           = product, version, grpc=<port>, tls=1, fp=<sha256 of the gRPC cert>
//! The TXT fingerprint is a hint only (mDNS is unauthenticated); clients still
//! pin the certificate they actually see on first connect.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tracing::{error, info};

pub const SERVICE_TYPE: &str = "_onesimus-dir._tcp.local.";

pub struct Announcement {
    pub director_name: String,
    pub bnet_port: u16,
    /// gRPC port and certificate fingerprint, if gRPC is enabled
    pub grpc: Option<(u16, String)>,
}

/// Hostname without domain part (mDNS names live in .local).
pub fn short_hostname() -> Option<String> {
    let host = nix::unistd::gethostname().ok()?.to_string_lossy().into_owned();
    host.split('.').next().filter(|h| !h.is_empty()).map(str::to_string)
}

pub fn advertise_loop(ann: Announcement, shutdown: &AtomicBool) {
    let daemon = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            error!("mDNS: cannot start responder: {e}");
            return;
        }
    };

    let host_name = format!("{}.local.", short_hostname().unwrap_or_else(|| "onesimus".to_string()));

    let mut txt = vec![
        ("product".to_string(), "onesimus-dird".to_string()),
        ("version".to_string(), env!("CARGO_PKG_VERSION").to_string()),
    ];
    if let Some((port, fp)) = &ann.grpc {
        txt.push(("grpc".to_string(), port.to_string()));
        txt.push(("tls".to_string(), "1".to_string()));
        txt.push(("fp".to_string(), fp.clone()));
    }

    let info = ServiceInfo::new(SERVICE_TYPE, &ann.director_name, &host_name, "", ann.bnet_port, &txt[..])
        .map(|i| i.enable_addr_auto());
    let info = match info {
        Ok(i) => i,
        Err(e) => {
            error!("mDNS: invalid service info: {e}");
            return;
        }
    };
    let fullname = info.get_fullname().to_string();

    if let Err(e) = daemon.register(info) {
        error!("mDNS: register failed: {e}");
        return;
    }
    info!("mDNS: announcing '{}' as {SERVICE_TYPE} on {host_name}", ann.director_name);

    while !shutdown.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(500));
    }

    // Goodbye packets so browsers drop us immediately
    if let Ok(rx) = daemon.unregister(&fullname) {
        let _ = rx.recv_timeout(Duration::from_secs(1));
    }
    let _ = daemon.shutdown();
    info!("mDNS: stopped");
}
