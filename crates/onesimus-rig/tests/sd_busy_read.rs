// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! A restore while a backup writes on the only Device: bareos-sd and osd.
//!
//! Start: `cargo test -p onesimus-rig --test sd_busy_read -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(300);

/// VBackup, then BigBackup (slow) and during it a restore of VBackup; status and log of the restore,
/// and whether the restore ended before the backup.
fn restore_on_a_busy_device(base: u16, sd: Option<PathBuf>) -> (char, char, String) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.sd = sd;
    options.dir_client_lines = vec!["Maximum Concurrent Jobs = 10".into()];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let first = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rig.wait_job(first, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}")).status, 'T');
    let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
    rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    std::thread::sleep(Duration::from_secs(3));
    let restore = rig.restore(first).unwrap_or_else(|e| panic!("{e}"));
    let r = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let b = rig.wait_job(big, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    (b.status, r.status, rig.job_log(restore))
}

/// Records what bareos-sd and osd do with the restore.
#[test]
#[ignore = "needs bareos-sd, osd and initdb: run with --ignored --nocapture"]
fn restore_waits_for_the_busy_device() {
    let (big, restore, log) = restore_on_a_busy_device(41601, None);
    println!("=== bareos-sd: BigBackup {big}, restore {restore}\n{log}");
    let (big2, restore2, log2) = restore_on_a_busy_device(41621, Some(onesimus_rig::prereq::target_profile_dir().join("osd")));
    println!("=== osd: BigBackup {big2}, restore {restore2}\n{log2}");
    assert_eq!((big, restore), ('T', 'T'));
    assert_eq!((big2, restore2), ('T', 'T'), "osd: the restore must wait for the device, not fail");
}
