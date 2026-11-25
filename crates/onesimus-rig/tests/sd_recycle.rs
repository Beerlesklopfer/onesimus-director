// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Recycling of a purged volume (Pool `Maximum Volumes = 1`, `Recycle = yes`) with bareos-sd and with
//! osd: the second backup reuses the volume, its old data is gone, and the restore of the second
//! backup is complete.
//!
//! Start: `cargo test -p onesimus-rig --test sd_recycle -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Backup, delete the job and purge its volume, backup again; the SD's messages of the second backup
/// (without times and versions), its status, the volume's job count afterwards, and the restore check.
fn recycle(base: u16, sd: Option<PathBuf>) -> (char, Vec<String>, String, i32) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.sd = sd;
    options.extra_director = vec![(
        "pool/VPool.conf".into(),
        "Pool {\n  Name = VPool\n  Pool Type = Backup\n  Label Format = \"VVol-\"\n  Maximum Volumes = 1\n  Recycle = yes\n  \
         AutoPrune = no\n  Maximum Volume Bytes = 1G\n}\n".into(),
    )];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let first = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rig.wait_job(first, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}")).status, 'T');
    for command in [format!("delete jobid={first} yes"), "update volume=VVol-0001 volstatus=Used".into(), "purge volume=VVol-0001 yes".into()] {
        let reply = rig.command(&command).unwrap_or_else(|e| panic!("{e}"));
        println!("> {command}\n{reply}");
    }
    let second = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(second, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(second);
    let sd_lines: Vec<String> = log.lines().filter(|l| l.starts_with("vsd JobId"))
        .map(|l| l.split_once(": ").map_or(l, |(_, r)| r).to_string())
        .filter(|l| !l.starts_with("Version:") && !l.starts_with("Elapsed time"))
        .collect();
    let media = rig.catalog().query("SELECT VolumeName, VolStatus, VolJobs, VolFiles FROM Media ORDER BY MediaId").unwrap_or_default();
    let restore = rig.restore(second).unwrap_or_else(|e| panic!("{e}"));
    let r = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let (rc, diff) = rig.diff_restore();
    assert_eq!(r.status, 'T', "{}", rig.job_log(restore));
    assert_eq!(rc, 0, "{diff}");
    (row.status, sd_lines, media.replace(&rig.layout.root.display().to_string(), "<root>"), rc)
}

/// osd recycles a purged volume like bareos-sd.
#[test]
#[ignore = "needs bareos-sd, osd and initdb: run with --ignored --nocapture"]
fn purged_volume_is_recycled_like_bareos_sd() {
    let bareos = recycle(41401, None);
    println!("=== bareos-sd: {bareos:#?}");
    let onesimus = recycle(41421, Some(onesimus_rig::prereq::target_profile_dir().join("osd")));
    println!("=== osd: {onesimus:#?}");
    assert_eq!(onesimus.0, 'T');
    let strip = |l: &Vec<String>| l.iter().map(|s| s.replace("41403", "<p>").replace("41423", "<p>")).map(|s| s.split(" (/tmp").next().unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(strip(&onesimus.1), strip(&bareos.1));
    assert_eq!(onesimus.2, bareos.2);
}
