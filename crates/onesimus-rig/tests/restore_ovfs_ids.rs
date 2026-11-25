// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `restore ovfs=<JobId>:<FileIndex>,…` (Onesimus): the restore IDs of OVFS nodes select exactly those
//! files.
//!
//! Start: `cargo test -p onesimus-rig --test restore_ovfs_ids -- --ignored`.

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Two files chosen by their restore IDs are restored, nothing else; a JobId of another client is refused.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn restore_ids_of_ovfs_nodes_select_exactly_those_files() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let rig = Rig::start(RigOptions::new(41201)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}");

    // the restore IDs as .ovfs_ls shows them: jobid and fileindex of the node
    let ls = rig.command(&format!(".ovfs_ls jobid={backup} path={}/", rig.layout.data.display())).unwrap_or_default();
    let findex = |name: &str| -> i32 {
        rig.catalog()
            .query(&format!("SELECT FileIndex FROM File WHERE JobId={backup} AND Name='{name}'"))
            .unwrap_or_default().lines().next().and_then(|l| l.trim().parse().ok())
            .unwrap_or_else(|| panic!("no FileIndex of {name}\n{ls}"))
    };
    let ids = format!("{backup}:{},{backup}:{}", findex("file0.txt"), findex("inner.txt"));
    let reply = rig.command(&format!("restore client=vfd ovfs={ids} where={} yes", rig.layout.restore.display()))
        .unwrap_or_else(|e| panic!("{e}"));
    let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("no JobId:\n{reply}\n{}", rig.diagnostics()));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((row.status, row.files), ('T', 2), "{row:?}\n{}", rig.job_log(id));
    let restored = rig.layout.restored_data();
    assert_eq!(std::fs::read_to_string(restored.join("file0.txt")).unwrap_or_default(), "content 0\n");
    assert_eq!(std::fs::read_to_string(restored.join("sub").join("inner.txt")).unwrap_or_default(), "inner\n");
    assert!(!restored.join("file1.txt").exists() && !restored.join("large.bin").exists());

    let refused = rig.command(&format!("restore client=vfd ovfs=999999:1 where={} yes", rig.layout.restore.display()))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(refused.contains("JobId 999999 is not a job of client \"vfd\""), "{refused}");
    let bad = rig.command("restore client=vfd ovfs=abc yes").unwrap_or_else(|e| panic!("{e}"));
    assert!(bad.contains("Invalid restore ID"), "{bad}");
}
