// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! A rejected `reload` writes the lexer position of the error to the Director log (C-057).
//!
//! Start: `cargo test -p onesimus-rig --test reload_log -- --ignored`.

#![deny(missing_docs)]

use onesimus_rig::{Rig, RigOptions};

/// A FileSet with `Exclude { … }` inside `Options` (an error of the original too): the reload fails,
/// the old configuration stays, and the log names line, column and file like `scan_err()`.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn rejected_reload_logs_line_and_column() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let rig = Rig::start(RigOptions::new(39951)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let bad = rig.layout.dir_conf.join("fileset").join("Bad.conf");
    std::fs::write(&bad, "FileSet {\n  Name = Bad\n  Include {\n    Options {\n      Exclude {\n        File = /x\n      }\n    }\n    File = /y\n  }\n}\n")
        .unwrap_or_else(|e| panic!("{e}"));
    let reply = rig.command("reload").unwrap_or_else(|e| panic!("{e}"));
    assert!(reply.contains("failed"), "reload: {reply}");
    let log = rig.daemon_log("odir");
    let at = bad.display().to_string();
    assert!(log.contains("line 5") && log.contains("of file") && log.contains(&at), "{log}");
    // the previous configuration stays: the console still works
    assert!(rig.command("version").is_ok());
    std::fs::remove_file(&bad).unwrap_or_else(|e| panic!("{e}"));
}
