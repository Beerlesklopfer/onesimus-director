//! Heartbeat thread — periodic keepalive.

use std::sync::atomic::{AtomicBool, Ordering};
use tracing::debug;

pub fn heartbeat_loop(shutdown: &AtomicBool) {
    debug!("heartbeat thread started");
    while !shutdown.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
    debug!("heartbeat thread stopped");
}
