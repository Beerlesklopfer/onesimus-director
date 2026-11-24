// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `resolve client=…` answered by ofd and by a Bareos FD: the same text.
//!
//! Needs a real `bareos-fd`: `ONESIMUS_BAREOS_FD=/path/to/bareos-fd`.
//! Start: `cargo test -p onesimus-rig --test fd_resolve -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;

use onesimus_rig::{Rig, RigOptions};

/// The reply of `resolve client=vfd …` from the FD started with `fd_program` (None: ofd).
fn resolve_reply(port: u16, fd_program: Option<PathBuf>) -> String {
    let mut options = RigOptions::new(port);
    options.fd_program = fd_program;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    rig.command("resolve client=vfd localhost 127.0.0.1 ::1 nonexistent.invalid").unwrap_or_else(|e| panic!("{e}"))
}

/// ofd answers `resolve` like bareos-fd 25.
#[test]
#[ignore = "needs bareos-sd, initdb and a bareos-fd (ONESIMUS_BAREOS_FD): run with --ignored"]
fn ofd_resolves_like_the_bareos_fd() {
    let Some(fd) = std::env::var_os("ONESIMUS_BAREOS_FD").map(PathBuf::from) else {
        panic!("ONESIMUS_BAREOS_FD is not set (path of a bareos-fd binary)");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let bareos = resolve_reply(40011, Some(fd));
    let ofd = resolve_reply(40021, None);
    let strip = |s: &str| s.lines().filter(|l| !l.starts_with("Connecting to Client") && !l.contains("Handshake") && !l.contains("Encryption")).collect::<Vec<_>>().join("\n");
    println!("=== bareos-fd ===\n{bareos:?}\n=== ofd ===\n{ofd:?}");
    assert_eq!(strip(&ofd), strip(&bareos));
}
