// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Two backups at the same time on one SD Device (Maximum Concurrent Jobs of the Device: default 1):
//! the second waits for the device and runs after the first. bareos-sd and osd.
//!
//! Start: `cargo test -p onesimus-rig --test sd_busy_device -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(300);

/// BigBackup (slow) and then VBackup on the same device; status and log of VBackup.
fn second_job_on_a_busy_device(base: u16, sd: Option<PathBuf>) -> (char, char, String) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.sd = sd;
    // the Director lets both jobs of the one client run at once
    options.dir_client_lines = vec!["Maximum Concurrent Jobs = 10".into()];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
    rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    std::thread::sleep(Duration::from_secs(3));
    let small = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let small_row = rig.wait_job(small, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let big_row = rig.wait_job(big, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    (big_row.status, small_row.status, rig.job_log(small))
}

/// Records what bareos-sd and osd do with the second job.
#[test]
#[ignore = "needs bareos-sd, osd and initdb: run with --ignored --nocapture"]
fn second_backup_waits_for_the_busy_device() {
    let (big, small, log) = second_job_on_a_busy_device(41301, None);
    println!("=== bareos-sd: BigBackup {big}, VBackup {small}\n{log}");
    let (big2, small2, log2) = second_job_on_a_busy_device(41321, Some(onesimus_rig::prereq::target_profile_dir().join("osd")));
    println!("=== osd: BigBackup {big2}, VBackup {small2}\n{log2}");
    assert_eq!((big, small), ('T', 'T'));
    assert_eq!((big2, small2), ('T', 'T'), "osd: the second job must wait for the device, not fail");
}
