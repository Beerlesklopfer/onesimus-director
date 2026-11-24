// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `setdebug` on the real daemons: ofd against bareos-fd, osd against bareos-sd; the replies the
//! Director shows are the same.
//!
//! Needs a real `bareos-fd`: `ONESIMUS_BAREOS_FD=/path/to/bareos-fd`.
//! Start: `cargo test -p onesimus-rig --test setdebug_daemons -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;

use onesimus_rig::{Rig, RigOptions};

/// The console commands sent to each pair of daemons.
const COMMANDS: &[&str] = &[
    "setdebug level=150 trace=0 client=vfd",
    "setdebug level=0 client=vfd",
    "setdebug level=7 trace=1 timestamp=1 client=vfd",
    "setdebug level=0 trace=0 timestamp=0 client=vfd",
    "setdebug level=120 storage=vsd",
    "setdebug level=0 trace=0 storage=vsd",
];

/// The replies of [`COMMANDS`] with the File daemon `fd` (None: ofd) and the Storage daemon `sd`
/// (None: the rig's default, bareos-sd).
fn replies(port: u16, fd: Option<PathBuf>, sd: Option<PathBuf>) -> Vec<String> {
    let mut options = RigOptions::new(port);
    options.fd_program = fd;
    options.sd = sd;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let root = rig.layout.root.display().to_string();
    COMMANDS
        .iter()
        .map(|c| {
            let reply = rig.command(c).unwrap_or_else(|e| panic!("{c}: {e}"));
            // ports and the rig path differ between the two rigs
            reply
                .replace(&root, "<root>")
                .lines()
                .map(|l| if l.starts_with("Connecting to ") { "Connecting".to_string() } else { l.to_string() })
                .filter(|l| !l.contains("Handshake") && !l.contains("Encryption"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

/// ofd and osd answer `setdebug` like bareos-fd and bareos-sd.
#[test]
#[ignore = "needs bareos-sd, osd, initdb and a bareos-fd (ONESIMUS_BAREOS_FD): run with --ignored"]
fn setdebug_replies_match_the_bareos_daemons() {
    let Some(fd) = std::env::var_os("ONESIMUS_BAREOS_FD").map(PathBuf::from) else {
        panic!("ONESIMUS_BAREOS_FD is not set (path of a bareos-fd binary)");
    };
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let bareos = replies(40061, Some(fd), None);
    let onesimus = replies(40071, None, Some(onesimus_rig::prereq::target_profile_dir().join("osd")));
    let mut diffs = Vec::new();
    for ((command, b), o) in COMMANDS.iter().zip(&bareos).zip(&onesimus) {
        println!("=== {command}\n--- bareos\n{b}\n--- onesimus\n{o}");
        if b != o {
            diffs.push(*command);
        }
    }
    assert!(diffs.is_empty(), "different replies: {diffs:?}");
}
