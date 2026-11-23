// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The real ofd binary with a correctly signed bundle (test hierarchy).
//! A binary built without ONESIMUS_PLUGIN_CA has no trust base and must
//! refuse every plugin; with ONESIMUS_PLUGIN_CA it trusts only that CA,
//! never the test hierarchy. The positive path with the test hierarchy is
//! covered by the unit tests in src/main.rs (cfg(test) trust base).
use onesimus_plugins::testca;
use prost::Message;
use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn a_build_without_its_own_trust_base_loads_no_plugin() {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let root = std::env::temp_dir().join(format!("onesimus-plugin-cli-{}-{}", std::process::id(), now.as_nanos()));
    std::fs::create_dir_all(&root).unwrap();
    let component = wat::parse_str("(component)").unwrap();
    let bundle = testca::signed_bundle(component, "test.cli", &[], Default::default(), now.as_secs());
    std::fs::write(root.join("bundle.pb"), bundle.encode_to_vec()).unwrap();
    std::fs::write(root.join("policy.json"), b"{}").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ofd"))
        .arg("--check-plugin")
        .arg(root.join("bundle.pb"))
        .arg("--plugin-policy")
        .arg(root.join("policy.json"))
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "the test hierarchy must not be trusted by a real build");
    assert!(output.stdout.is_empty());
    if option_env!("ONESIMUS_PLUGIN_CA").is_none_or(str::is_empty) {
        assert!(stderr.contains("built without ONESIMUS_PLUGIN_CA"), "{stderr}");
    } else {
        assert!(stderr.contains("untrusted"), "{stderr}");
    }
    std::fs::remove_dir_all(root).unwrap();
}
