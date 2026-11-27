// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `restore … fileregex=` with odir and with the original bareos-dir in the same rig.
//!
//! bareos-dir sends every FileIndex of the job with a FileRegex line and lets the SD filter: it expects
//! all files (13) and ends "with warnings" (File count mismatch: expected=13 , restored=1). odir looks
//! the matching files up in the catalog on purpose (J-16, the SD's filter can pass data records of a
//! skipped file): it expects and restores 1 and ends OK. Where the catalog has no match, odir sends the
//! job like the original and expects all files as well. (A name that is not UTF-8 was such a case until
//! C-047: odir stored U+FFFD in the catalog; now it stores the bytes and finds the file like any other.)
//!
//! Needs `/usr/sbin/bareos-dir`. Start: `cargo test -p onesimus-rig --test restore_fileregex_compare -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Backup, then a restore of one file by `fileregex=`; the report lines that matter.
fn run(base: u16, director: Option<PathBuf>, regex: &str) -> (char, Vec<String>) {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.dir_program = director;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}\n{}", rig.job_log(backup));
    let command = format!("restore jobid={backup} client=vfd where={} fileregex={regex} all done yes", rig.layout.restore.display());
    let reply = rig.command(&command).unwrap_or_else(|e| panic!("{e}"));
    let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("no JobId:\n{reply}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(id);
    let lines = log.lines()
        .map(|l| l.split_once(": ").map_or(l, |(_, rest)| rest).trim().to_string())
        .filter(|l| l.starts_with("Files Expected") || l.starts_with("Files Restored") || l.contains("mismatch")
            || l.starts_with("Termination") || l.starts_with("Warning"))
        .collect();
    (row.status, lines)
}

/// The original expects every file and warns; odir expects only the matching file (deliberate).
#[test]
#[ignore = "needs bareos-dir, bareos-sd and initdb: run with --ignored --nocapture"]
fn fileregex_restore_expects_the_matching_file_where_the_original_expects_all() {
    let original = run(41101, Some(PathBuf::from("/usr/sbin/bareos-dir")), "/file0\\.txt$");
    let onesimus = run(41121, None, "/file0\\.txt$");
    println!("bareos-dir: {original:?}\nodir:       {onesimus:?}");
    assert_eq!(original, ('W', vec!["Warning: File count mismatch: expected=13 , restored=1".to_string()]));
    assert_eq!(onesimus, ('T', Vec::new()));
}

/// No catalog match: odir sends the job like the original and reports the same.
#[test]
#[ignore = "needs bareos-dir, bareos-sd and initdb: run with --ignored --nocapture"]
fn fileregex_without_catalog_match_reports_like_the_original() {
    let original = run(41141, Some(PathBuf::from("/usr/sbin/bareos-dir")), "/no-such-file\\.txt$");
    let onesimus = run(41161, None, "/no-such-file\\.txt$");
    println!("bareos-dir: {original:?}\nodir:       {onesimus:?}");
    assert_eq!(onesimus, original);
}
