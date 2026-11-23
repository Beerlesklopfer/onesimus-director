// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
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
