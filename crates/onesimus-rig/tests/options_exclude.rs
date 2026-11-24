// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `Exclude` inside `Options` (Onesimus extension of `odir.d`, protocol line `Y`) against the real `ofd`.
//!
//! Start: `cargo test -p onesimus-rig --test options_exclude -- --ignored`
//! (docs/wiki/testgeruest.md, docs/wiki/betrieb-odir.md § FileSet).

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Entries of the source tree (see `onesimus_rig::tree::build`): `data`, `sub`, `file0..2`, `inner`, `empty`, `large`, `link0`, `hard1`, `hard2`, the non-UTF-8 name and `sparse`.
const ENTRIES: u64 = 13;

/// Entries below `data/sub`: the directory itself, `inner.txt` and `hard2`.
const BELOW_SUB: u64 = 3;

/// A Full backup of `VBackup`: its catalog row once it has ended.
fn full_backup(rig: &Rig) -> onesimus_rig::JobRow {
    let id = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "backup: {row:?}\n{}\n{}", rig.job_log(id), rig.diagnostics());
    row
}

/// The overlay excludes `data/sub` for the one Options block only: the backup after the reload has
/// the entries below `sub` less, the one before it has all of them, and `estimate` shows the same.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn exclude_inside_options_is_applied_by_the_ofd() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let rig = Rig::start(RigOptions::new(39941)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let before = full_backup(&rig);
    assert_eq!(before.files, ENTRIES, "{before:?}\n{}", rig.diagnostics());

    let sub = rig.layout.data.join("sub");
    let overlay = rig.layout.root.join("onesimus").join("odir.d").join("fileset");
    std::fs::create_dir_all(&overlay).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(
        overlay.join("VSet.conf"),
        format!("FileSet {{\n  Name = VSet\n  Include {{\n    Options {{\n      Index = 1\n      Exclude {{\n        File = {}\n      }}\n    }}\n  }}\n}}\n", sub.display()),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let reload = rig.command("reload").unwrap_or_else(|e| panic!("{e}"));
    assert!(!reload.to_lowercase().contains("error"), "reload: {reload}\n{}", rig.diagnostics());

    let estimate = rig.command("estimate job=VBackup listing").unwrap_or_else(|e| panic!("{e}"));
    assert!(estimate.contains("file0.txt") && !estimate.contains("inner.txt") && !estimate.contains("hard2"), "estimate:\n{estimate}");

    let after = full_backup(&rig);
    assert_eq!(after.files, ENTRIES - BELOW_SUB, "{after:?}\n{}", rig.diagnostics());
}
