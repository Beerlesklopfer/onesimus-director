// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Build script: writes the module and type tree for rustdoc
//! (`scripts/doc/module_tree.rs`, docs/wiki/dokumentation.md) and the build
//! date for [`version`](src/version.rs) (`ONESIMUS_DAEMON_BUILD_DATE`,
//! `%d %B %Y`, honouring `SOURCE_DATE_EPOCH` for reproducible builds).

include!("../../scripts/doc/module_tree.rs");

fn main() {
    write_module_tree();
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let mut cmd = std::process::Command::new("date");
    cmd.env("LC_ALL", "C").arg("-u");
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        cmd.arg("-d").arg(format!("@{epoch}"));
    }
    let date = cmd
        .arg("+%d %B %Y")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=ONESIMUS_DAEMON_BUILD_DATE={date}");
}
