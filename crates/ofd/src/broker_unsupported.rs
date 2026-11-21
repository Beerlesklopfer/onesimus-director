//! Native plugin brokers on platforms without them (Windows).
//!
//! The broker of a signed plugin is a native process started with
//! `fexecve` from a checked descriptor, under `PR_SET_NO_NEW_PRIVS` and
//! `PR_SET_PDEATHSIG`, talking over a Unix socket pair (`broker.rs`). None
//! of that exists on Windows, so no broker can be started: [`Broker`] and
//! [`BrokerSocket`] have no values, and a plugin whose verified manifest
//! needs a broker fails to load with "native plugin brokers are not
//! supported on this platform" (as on macOS, docs/plans/
//! PORTIERUNG-MACOS-WINDOWS.md). WASM plugins without a broker are not
//! affected by this module.

use std::path::Path;
use std::time::Duration;

use crate::plugin::PluginError;

/// The error text when a plugin needs a broker.
pub const UNSUPPORTED: &str = "native plugin brokers are not supported on this platform";

/// A running broker; there is none on this platform.
#[derive(Debug)]
pub enum Broker {}

impl Broker {
    /// Never called: no broker exists.
    pub fn stop(&mut self, _grace: Duration) -> Option<i32> {
        match *self {}
    }
}

/// The socket to a broker (a Unix stream on Linux); there is none here.
#[derive(Debug)]
pub enum BrokerSocket {}

impl BrokerSocket {
    /// Never called: no socket exists.
    pub fn shutdown(&self, _how: std::net::Shutdown) -> std::io::Result<()> {
        match *self {}
    }
}

/// For `ofd --check-plugin`: native parts cannot be checked here (and
/// [`crate::config::multiarch_triplet`] is `None`, so it is not called).
pub(crate) fn check_parts(plugin_dir: &Path, _manifest: &onesimus_plugins::Manifest, _triplet: &str)
    -> Result<usize, PluginError> {
    Err(PluginError::Untrusted { path: plugin_dir.to_path_buf(), reason: UNSUPPORTED.into() })
}
