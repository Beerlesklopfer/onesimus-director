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

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};
use tracing::{error, info};

pub const SERVICE_TYPE: &str = "_onesimus-dir._tcp.local.";

pub struct Announcement {
    pub director_name: String,
    pub bnet_port: u16,
    /// gRPC port and certificate fingerprint, if gRPC is enabled
    pub grpc: Option<(u16, String)>,
}

/// Virtual bridges/links of VMs and containers (libvirt, docker, podman,
/// LXC, veth pairs, VM taps). Their addresses are not reachable from the LAN,
/// so announcing them makes clients pick a dead address.
pub fn is_virtual_interface(name: &str) -> bool {
    const PREFIXES: &[&str] = &["virbr", "docker", "veth", "vnet", "lxc", "lxdbr", "podman", "cni", "flannel", "tap"];
    // Docker user networks are "br-" + 12 hex digits; LAN bridges such as
    // br0 or br-lan stay announced.
    let docker_bridge = name.strip_prefix("br-")
        .is_some_and(|id| id.len() == 12 && id.bytes().all(|b| b.is_ascii_hexdigit()));
    docker_bridge || PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Names of virtual interfaces present on this host (from /sys/class/net).
fn virtual_interfaces() -> Vec<String> {
    std::fs::read_dir("/sys/class/net")
        .map(|d| d.flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| is_virtual_interface(n))
            .collect())
        .unwrap_or_default()
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

    let skipped = virtual_interfaces();
    if !skipped.is_empty() {
        info!("mDNS: not announcing on virtual interfaces {}", skipped.join(", "));
        let kinds: Vec<IfKind> = skipped.into_iter().map(IfKind::Name).collect();
        if let Err(e) = daemon.disable_interface(kinds) {
            error!("mDNS: cannot exclude virtual interfaces: {e}");
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_interface_names() {
        for n in ["virbr0", "docker0", "br-0a1b2c3d4e5f", "veth12ab", "vnet0", "lxcbr0", "podman0"] {
            assert!(is_virtual_interface(n), "{n}");
        }
        for n in ["eth0", "enp3s0", "br0", "br-lan", "br-wan", "wlan0", "bond0", "eno1"] {
            assert!(!is_virtual_interface(n), "{n}");
        }
    }
}
