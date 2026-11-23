// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Compiles the plugin trust base into ofd (plan: docs/plans/SIGNIER-INFRASTRUKTUR.md).
//!
//! `ONESIMUS_PLUGIN_CA` names a directory (absolute path) with
//! - `root.pem`: the offline root CA certificate (Ed25519),
//! - `generation.pem`: the generation certificate of this release
//!   ("Onesimus Plugin Signing G<n>"),
//! - `revoked.json`: the built-in revocation list (`{}` if empty).
//!
//! Without the variable the build has no trust base and loads no plugin.
//! ofd parses and checks the files at run time (`TrustBase::from_pem`).
use std::path::{Path, PathBuf};

const FILES: [&str; 3] = ["root.pem", "generation.pem", "revoked.json"];

fn main() {
    println!("cargo::rerun-if-env-changed=ONESIMUS_PLUGIN_CA");
    println!("cargo::rustc-check-cfg=cfg(onesimus_plugin_ca)");
    let Some(dir) = std::env::var_os("ONESIMUS_PLUGIN_CA").filter(|d| !d.is_empty()) else {
        return;
    };
    let dir = PathBuf::from(dir);
    if !dir.is_absolute() {
        panic!("ONESIMUS_PLUGIN_CA must be an absolute path, not {}", dir.display());
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    for name in FILES {
        let path = dir.join(name);
        println!("cargo::rerun-if-changed={}", path.display());
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("ONESIMUS_PLUGIN_CA: {}: {e}", path.display()));
        check(&path, &bytes);
        std::fs::write(out.join(name), &bytes).expect("write trust base into OUT_DIR");
    }
    println!("cargo::rustc-cfg=onesimus_plugin_ca");
}

/// Coarse checks only; the full check happens in ofd and its tests.
fn check(path: &Path, bytes: &[u8]) {
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if has(b"PRIVATE KEY") {
        panic!("{}: a private key must never be compiled into ofd", path.display());
    }
    if path.extension().is_some_and(|e| e == "pem") && !has(b"-----BEGIN CERTIFICATE-----") {
        panic!("{}: no PEM certificate", path.display());
    }
    if bytes.len() > 64 * 1024 {
        panic!("{}: larger than 64 KiB", path.display());
    }
}
