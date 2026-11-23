// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The plugin trust base of this ofd build and the suite it belongs to.
//!
//! Root, generation and revocation list are compiled in from
//! `ONESIMUS_PLUGIN_CA` (see build.rs). A build without it has no trust
//! base: every plugin load fails. Only this crate's own tests use the test
//! hierarchy instead (`cfg(test)`); there is no run-time switch.
use std::path::Path;

use onesimus_plugins::{TrustBase, TrustPolicy, bundle};

use super::PluginError;

#[cfg(onesimus_plugin_ca)]
const BUILTIN: Option<[&[u8]; 3]> = Some([
    include_bytes!(concat!(env!("OUT_DIR"), "/root.pem")),
    include_bytes!(concat!(env!("OUT_DIR"), "/generation.pem")),
    include_bytes!(concat!(env!("OUT_DIR"), "/revoked.json")),
]);
#[cfg(not(onesimus_plugin_ca))]
const BUILTIN: Option<[&[u8]; 3]> = None;

/// The compiled-in trust base, checked on every call (cheap, and a broken
/// build input shows up as a load error instead of a silent default).
pub fn builtin() -> Result<TrustBase, PluginError> {
    let [root, generation, revoked] = BUILTIN.ok_or_else(|| {
        PluginError::Load(
            "this ofd was built without ONESIMUS_PLUGIN_CA: it has no plugin trust base and loads no plugin".into(),
        )
    })?;
    TrustBase::from_pem(root, generation, revoked)
        .map_err(|e| PluginError::Load(format!("built-in plugin trust base is invalid: {e}")))
}

/// The trust base plugin loads use.
#[cfg(not(test))]
pub fn base() -> Result<TrustBase, PluginError> {
    builtin()
}
/// Tests of this crate: the shared test hierarchy.
#[cfg(test)]
pub fn base() -> Result<TrustBase, PluginError> {
    Ok(onesimus_plugins::testca::TestCa::shared().trust_base())
}

/// Suite version of this ofd: the Onesimus part of its version.
pub fn suite() -> Result<&'static str, PluginError> {
    onesimus_plugins::suite_of(env!("CARGO_PKG_VERSION"))
        .ok_or_else(|| PluginError::Load(format!("version {} has no Onesimus suite part", env!("CARGO_PKG_VERSION"))))
}

/// Trust base, local restrictions from `local` and suite, read for one load.
pub fn policy(local: &Path) -> Result<TrustPolicy, PluginError> {
    let base = base()?;
    let local = bundle::read_local_policy(local, &base).map_err(|e| PluginError::Load(format!("{}: {e}", local.display())))?;
    Ok(TrustPolicy { base, local, suite: suite()?.to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_onesimus_plugin_ca_there_is_no_trust_base() {
        if cfg!(onesimus_plugin_ca) {
            builtin().expect("the configured trust base is valid");
        } else {
            let error = builtin().unwrap_err().to_string();
            assert!(error.contains("ONESIMUS_PLUGIN_CA"), "{error}");
        }
    }

    #[test]
    fn suite_is_the_onesimus_part_of_the_version() {
        let version = env!("CARGO_PKG_VERSION");
        let suite = suite().unwrap();
        assert!(version.ends_with(&format!("+onesimus.{suite}")), "{version} / {suite}");
    }
}
