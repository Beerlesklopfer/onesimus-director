// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The cancel of a running backup with odir and with bareos-dir in the same rig (S-27, D16).
//!
//! bareos-dir's CancelJob() sets JS_Canceled first (job.cc:667), cuts the job's FD connection once the FD
//! has answered (fd_cmds.cc:1000-1001: no FD job end is read, "No Job status returned from FD.") and its
//! SD connection after the SD's answer ("Director's comm line to SD dropped."); the job ends "Backup
//! Canceled" with JobStatus A. The report of odir is the same, but for versions.
//!
//! bareos-dir's job thread ends a canceled backup with
//! CancelStorageDaemonJob(jcr) (backup.cc:726), whose `*JobCancel*` control
//! JCR takes a number of the CreateUniqueJobName() sequence (sd_cmds.cc:675,
//! ua_server.cc new_control_jcr); the next job's unique name skips it.
//!
//! Needs `/usr/sbin/bareos-dir`. Start: `cargo test -p onesimus-rig --test cancel_job_name_number -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;
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

/// In one console session: two backups, a canceled BigBackup, another backup. Returns the steps of the
/// job name numbers (second after first, the one after the cancel after BigBackup), the canceled job's
/// status and its log, without dates and host-specific prefixes.
fn scenario(base: u16, director: Option<PathBuf>) -> (u32, u32, char, String) {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.dir_program = director;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let mut console = rig.console().unwrap_or_else(|e| panic!("{e}"));
    let run = |console: &mut onesimus_rig::console::Console, job: &str| {
        let reply = console.command(&format!("run job={job} yes"), STEP).unwrap_or_else(|e| panic!("{e}"));
        onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("no JobId:\n{reply}"))
    };
    let done = |id: u32| {
        let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(row.status, 'T', "{row:?}\n{}", rig.job_log(id));
    };

    let first = run(&mut console, "VBackup");
    done(first);
    let second = run(&mut console, "VBackup");
    done(second);

    let big = run(&mut console, "BigBackup");
    let row = rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'R', "{row:?}");
    std::thread::sleep(Duration::from_secs(3));
    let reply = console.command(&format!("cancel jobid={big} yes"), STEP).unwrap_or_else(|e| panic!("{e}"));
    assert!(reply.contains("marked to be canceled"), "{reply}");
    let canceled = rig.wait_job(big, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));

    let after = run(&mut console, "VBackup");
    done(after);
    let log = rig.job_log(big).lines()
        .map(|l| match l.split_once(": ") {
            Some((head, rest)) if head.contains(" JobId ") => rest.trim_end().to_string(),
            _ => l.trim_end().to_string(),
        })
        .collect::<Vec<_>>().join("\n");
    (step(number(&rig, first), number(&rig, second)), step(number(&rig, big), number(&rig, after)), canceled.status, log)
}

/// After a canceled backup the next job's number is two further on (one for the `*JobCancel*` control
/// JCR), with odir as with bareos-dir; a finished backup takes none. The canceled job's status and report
/// are those of the original.
#[test]
#[ignore = "needs bareos-dir, bareos-sd and initdb: run with --ignored --nocapture"]
fn a_canceled_backup_takes_a_job_name_number() {
    let original = scenario(40111, Some(PathBuf::from("/usr/sbin/bareos-dir")));
    let onesimus = scenario(40131, None);
    println!("bareos-dir: {} {} {}\n{}\n\nodir: {} {} {}\n{}", original.0, original.1, original.2, original.3,
        onesimus.0, onesimus.1, onesimus.2, onesimus.3);
    assert_eq!((original.0, original.1), (1, 2), "bareos-dir");
    assert_eq!((onesimus.0, onesimus.1), (1, 2), "odir");
    assert_eq!((original.2, onesimus.2), ('A', 'A'));
    assert_eq!(comparable(&onesimus.3), comparable(&original.3));
}

/// The lines of a job log that both Directors must write alike: digits as `N`, without the version and
/// build lines, sorted (the daemons' messages arrive in either order).
fn comparable(log: &str) -> Vec<String> {
    let mut lines: Vec<String> = log.lines()
        .filter(|l| !l.starts_with("Version:") && !l.starts_with("Bareos bareos-dir") && !l.contains("Bareos binary info"))
        .map(|l| l.chars().map(|c| if c.is_ascii_digit() { 'N' } else { c }).collect::<String>())
        .map(|l| { let mut out = String::new(); for c in l.chars() { if !(c == 'N' && out.ends_with('N')) { out.push(c) } } out })
        .collect();
    lines.sort();
    lines
}

/// A backup, a restore queued for 2099 and canceled before it starts, another backup: the step of the job
/// name numbers from the restore to that backup, and the restore's status.
fn queued_restore(base: u16, director: Option<PathBuf>) -> (u32, char) {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.dir_program = director;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let mut console = rig.console().unwrap_or_else(|e| panic!("{e}"));
    let mut queued = |console: &mut onesimus_rig::console::Console, line: &str| {
        let reply = console.command(line, STEP).unwrap_or_else(|e| panic!("{e}"));
        onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("no JobId:\n{reply}"))
    };
    let backup = queued(&mut console, "run job=VBackup yes");
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}");
    // `restore` has no when= (bareos-dir: "Unknown keyword: when"); `run` of the Restore job has
    let restore = queued(&mut console, &format!("run job=VRestore jobid={backup} when=\"2099-01-01 00:00:00\" yes"));
    std::thread::sleep(Duration::from_secs(2));
    // a job waiting for its start time is only marked (job.cc CancelJob, JS_WaitStartTime)
    let reply = console.command(&format!("cancel jobid={restore} yes"), STEP).unwrap_or_else(|e| panic!("{e}"));
    assert!(reply.contains("marked to be canceled"), "port {base}: {reply:?}");
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let after = queued(&mut console, "run job=VBackup yes");
    let done = rig.wait_job(after, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(done.status, 'T', "{done:?}");
    (step(number(&rig, restore), number(&rig, after)), row.status)
}

/// A restore canceled before it starts ends with NativeRestoreCleanup(JS_Canceled) (job.cc), which
/// cancels the SD job through a `*JobCancel*` control JCR: one job name number, as with bareos-dir.
#[test]
#[ignore = "needs bareos-dir, bareos-sd and initdb: run with --ignored --nocapture"]
fn a_restore_canceled_before_its_start_takes_a_job_name_number() {
    let original = queued_restore(40151, Some(PathBuf::from("/usr/sbin/bareos-dir")));
    let onesimus = queued_restore(40171, None);
    println!("bareos-dir: {original:?}\nodir:       {onesimus:?}");
    assert_eq!(onesimus, original);
    assert_eq!(original, (2, 'A'));
}
