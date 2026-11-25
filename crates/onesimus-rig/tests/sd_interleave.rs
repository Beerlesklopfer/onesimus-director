// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Two backups at the same time on one Device with `Maximum Concurrent Jobs = 2`: both write into
//! the same volume (their blocks interleaved), the short one ends while the long one still runs,
//! and both restore. bareos-sd and osd (OP-2).
//!
//! Start: `cargo test -p onesimus-rig --test sd_interleave -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(300);

/// What one run shows.
#[derive(Debug)]
struct Outcome {
    /// Status of BigBackup and VBackup.
    status: (char, char),
    /// VBackup ended while BigBackup still ran.
    overlapped: bool,
    /// Volumes of each job (`JobId|VolumeName`, distinct).
    media: String,
    /// The SD's messages of VBackup (no versions, paths, sizes).
    sd_lines: Vec<String>,
    /// Restore of VBackup equals the source; restore of BigBackup brought back all its files.
    restored: (bool, bool),
}

/// BigBackup (slow), during it VBackup; then both restored.
fn interleave(base: u16, sd: Option<PathBuf>, max_volume_bytes: &str) -> Outcome {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.sd = sd;
    options.dir_client_lines = vec!["Maximum Concurrent Jobs = 10".into()];
    options.sd_device_lines = vec!["Maximum Concurrent Jobs = 2".into()];
    options.extra_director = vec![(
        "pool/VPool.conf".into(),
        format!("Pool {{\n  Name = VPool\n  Pool Type = Backup\n  Label Format = \"VVol-\"\n  Maximum Volume Bytes = {max_volume_bytes}\n}}\n"),
    )];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
    rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    std::thread::sleep(Duration::from_secs(3));
    let small = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let small_row = rig.wait_job(small, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let overlapped = rig.job_row(big).ok().flatten().is_some_and(|r| r.status == 'R');
    let big_row = rig.wait_job(big, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let media = rig.catalog()
        .query("SELECT DISTINCT jm.JobId, m.VolumeName FROM JobMedia jm JOIN Media m USING (MediaId) ORDER BY 1, 2")
        .unwrap_or_default();
    let sd_lines = rig.job_log(small).lines()
        .filter_map(|l| l.strip_prefix(&format!("vsd JobId {small}: ")))
        .filter(|l| !l.starts_with("Version:") && !l.starts_with("Elapsed time"))
        .map(|l| l.split(" (/tmp").next().unwrap_or("").split(" size=").next().unwrap_or("").to_string())
        .collect();
    // VBackup: compared with the source tree
    let r = rig.restore(small).unwrap_or_else(|e| panic!("{e}"));
    let r_row = rig.wait_job(r, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let (code, diff) = rig.diff_restore();
    if code != 0 {
        eprintln!("restore of VBackup differs:\n{diff}\n{}", rig.job_log(r));
    }
    // BigBackup: every file comes back
    let where_big = rig.layout.root.join("restore-big");
    let reply = rig.command(&format!("restore jobid={big} client=vfd where={} all done yes", where_big.display())).unwrap_or_else(|e| panic!("{e}"));
    let rb = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("{reply}"));
    let rb_row = rig.wait_job(rb, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    if rb_row.status != 'T' || rb_row.files != big_row.files {
        eprintln!("restore of BigBackup: {rb_row:?} (backup {big_row:?})\n{}", rig.job_log(rb));
    }
    Outcome {
        status: (big_row.status, small_row.status),
        overlapped,
        media,
        sd_lines,
        restored: (r_row.status == 'T' && code == 0, rb_row.status == 'T' && rb_row.files == big_row.files),
    }
}

/// osd writes two jobs into one volume at the same time like bareos-sd.
#[test]
#[ignore = "needs bareos-sd, osd and initdb: run with --ignored --nocapture"]
fn two_jobs_write_into_one_volume_at_the_same_time() {
    let bareos = interleave(41701, None, "1G");
    println!("=== bareos-sd: {bareos:#?}");
    let onesimus = interleave(41741, Some(onesimus_rig::prereq::target_profile_dir().join("osd")), "1G");
    println!("=== osd: {onesimus:#?}");
    assert_eq!((bareos.status, bareos.overlapped, bareos.restored), (('T', 'T'), true, (true, true)));
    assert_eq!((onesimus.status, onesimus.overlapped, onesimus.restored), (('T', 'T'), true, (true, true)));
    assert_eq!(onesimus.media, bareos.media, "both jobs on the same volume");
    assert_eq!(onesimus.sd_lines, bareos.sd_lines);
}

/// The volume fills up while both jobs write (Maximum Volume Bytes 20 MB, BigBackup about 50 MB):
/// the writer that fills it mounts the next one, the other follows; both jobs restore completely.
#[test]
#[ignore = "needs bareos-sd, osd and initdb: run with --ignored --nocapture"]
fn the_volume_changes_while_two_jobs_write() {
    let bareos = interleave(41801, None, "20M");
    println!("=== bareos-sd: {bareos:#?}");
    let onesimus = interleave(41841, Some(onesimus_rig::prereq::target_profile_dir().join("osd")), "20M");
    println!("=== osd: {onesimus:#?}");
    assert_eq!((bareos.status, bareos.restored), (('T', 'T'), (true, true)));
    assert_eq!((onesimus.status, onesimus.restored), (('T', 'T'), (true, true)));
    assert!(onesimus.media.lines().count() > 2, "the jobs span more than one volume:\n{}", onesimus.media);
}
