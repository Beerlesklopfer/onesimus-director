// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `resolve client=…` answered by ofd and by a Bareos FD, with and without TLS: the same text.
//!
//! Needs a real `bareos-fd`: `ONESIMUS_BAREOS_FD=/path/to/bareos-fd`.
//! Start: `cargo test -p onesimus-rig --test fd_resolve -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;

use onesimus_rig::{Rig, RigOptions};

/// The reply of `resolve client=vfd …` from the FD started with `fd_program` (None: ofd).
fn resolve_reply(port: u16, fd_program: Option<PathBuf>, plain: bool) -> String {
    let mut options = RigOptions::new(port);
    options.fd_program = fd_program;
    options.fd_plain = plain;
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
    let bareos = resolve_reply(40011, Some(fd.clone()), false);
    let ofd = resolve_reply(40021, None, false);
    let strip = |s: &str| s.lines().filter(|l| !l.starts_with("Connecting to Client") && !l.contains("Handshake") && !l.contains("Encryption")).collect::<Vec<_>>().join("\n");
    println!("=== bareos-fd ===\n{bareos:?}\n=== ofd ===\n{ofd:?}");
    assert_eq!(strip(&ofd), strip(&bareos));

    // a FD without TLS (clear text, as older daemons): the same through odir
    let bareos_plain = resolve_reply(40031, Some(fd), true);
    let ofd_plain = resolve_reply(40041, None, true);
    println!("=== bareos-fd, no TLS ===\n{bareos_plain:?}\n=== ofd, no TLS ===\n{ofd_plain:?}");
    assert!(!bareos_plain.contains("Encryption: TLS"), "{bareos_plain}");
    assert_eq!(strip(&ofd_plain), strip(&bareos_plain));
    assert_eq!(strip(&bareos_plain), strip(&bareos));
}
