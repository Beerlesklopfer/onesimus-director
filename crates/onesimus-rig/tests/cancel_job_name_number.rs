// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! A canceled backup takes a job name number like bareos-dir (S-27).
//!
//! bareos-dir's job thread ends a canceled backup with
//! CancelStorageDaemonJob(jcr) (backup.cc:726), whose `*JobCancel*` control
//! JCR takes a number of the CreateUniqueJobName() sequence (sd_cmds.cc:675,
//! ua_server.cc new_control_jcr); the next job's unique name skips it.
//!
//! Start: `cargo test -p onesimus-rig --test cancel_job_name_number -- --ignored`.

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Longest time one console command may take.
const STEP: Duration = Duration::from_secs(60);

/// The number at the end of a unique job name (`VBackup.2026-11-27_10.00.00_07` → 7).
fn number(rig: &Rig, id: u32) -> u32 {
    let job = rig.catalog().query(&format!("SELECT Job FROM Job WHERE JobId = {id}")).unwrap_or_else(|e| panic!("{e}"));
    job.trim().rsplit('_').next().and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("job name {job:?}"))
}

/// Numbers run 1..59, then 0 (job.cc CreateUniqueJobName).
fn step(from: u32, to: u32) -> u32 {
    (to + 60 - from) % 60
}

/// In one console session: two backups take consecutive numbers; after a
/// canceled backup the next job's number is two further on.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn a_canceled_backup_takes_a_job_name_number() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let rig = Rig::start(RigOptions::new(40111)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let mut console = rig.console().unwrap_or_else(|e| panic!("{e}"));
    let mut run = |console: &mut onesimus_rig::console::Console, job: &str| {
        let reply = console.command(&format!("run job={job} yes"), STEP).unwrap_or_else(|e| panic!("{e}"));
        onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("no JobId:\n{reply}"))
    };

    let first = run(&mut console, "VBackup");
    let row = rig.wait_job(first, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}\n{}", rig.job_log(first));
    let second = run(&mut console, "VBackup");
    let row = rig.wait_job(second, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}\n{}", rig.job_log(second));
    assert_eq!(step(number(&rig, first), number(&rig, second)), 1, "a finished backup takes no extra number");

    let big = run(&mut console, "BigBackup");
    let row = rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'R', "{row:?}");
    std::thread::sleep(Duration::from_secs(3));
    let reply = console.command(&format!("cancel jobid={big} yes"), STEP).unwrap_or_else(|e| panic!("{e}"));
    assert!(reply.contains("marked to be canceled"), "{reply}");
    let row = rig.wait_job(big, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    // 'A' like bareos-dir; odir still writes 'f' (expected defect D16, tests/rig.rs)
    assert!(matches!(row.status, 'A' | 'f'), "{row:?}\n{}", rig.job_log(big));

    let after = run(&mut console, "VBackup");
    let row = rig.wait_job(after, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}\n{}", rig.job_log(after));
    assert_eq!(step(number(&rig, big), number(&rig, after)), 2, "the *JobCancel* control JCR takes one number\n{}",
        rig.job_log(big));
}
