// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Restore of one file with a name that is not UTF-8 from a `SQL_ASCII` catalog (C-047).
//!
//! Start: `cargo test -p onesimus-rig --test restore_non_utf8 -- --ignored`.

#![deny(missing_docs)]

use std::os::unix::ffi::OsStrExt as _;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// `restore … fileregex=` selects the Latin-1 named file (`caf\xe9.txt`) in the tree and restores it byte-exactly;
/// the other files are not restored.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn fileregex_restores_a_file_with_a_latin1_name() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let rig = Rig::start(RigOptions::new(39961)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}\n{}", rig.diagnostics());

    let command = format!(
        "restore jobid={backup} client=vfd where={} fileregex=/caf.\\.txt$ all done yes",
        rig.layout.restore.display()
    );
    let reply = rig.command(&command).unwrap_or_else(|e| panic!("{e}"));
    let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("no JobId:\n{reply}\n{}", rig.diagnostics()));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    // 'W': the catalog has no match for the Latin-1 name, so odir sends the job like bareos-dir (all FileIndexes
    // and the FileRegex line) and warns like it: "File count mismatch: expected=13 , restored=1"
    // (compared in restore_fileregex_compare.rs)
    assert!(matches!(row.status, 'T' | 'W') && row.files == 1, "{row:?}\n{}\n{}", rig.job_log(id), rig.diagnostics());

    let restored = rig.layout.restored_data().join(std::ffi::OsStr::from_bytes(onesimus_rig::tree::NON_UTF8_NAME));
    assert_eq!(std::fs::read_to_string(&restored).unwrap_or_default(), "latin-1 name\n", "{}", restored.display());
    assert!(!rig.layout.restored_data().join("file0.txt").exists());
}
