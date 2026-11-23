// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Reading waits for a Volume that is not there (`DirAskSysopToMountVolume`):
//! the Volume file is moved away, the restore starts, the job asks the
//! operator to mount it (message in the job log), the file
//! comes back, `mount` wakes the job, the restore ends with the right data.
//!
//! Start: `cargo test -p onesimus-rig --test osd_read_wait -- --ignored --test-threads=1`
//! (bareos-sd for the reference test: ports from 39661).

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(120);

/// Backup, volume away, restore waits, volume back, `mount`, restore ends.
fn read_waits_for_the_volume(base_port: u16, sd: PathBuf) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(sd);
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "Backup: {row:?}\n{}", rig.job_log(backup));
    let media = rig.catalog().query("SELECT VolumeName FROM Media ORDER BY MediaId").unwrap_or_else(|e| panic!("{e}"));
    let volume = media.lines().next().unwrap_or_default().trim().to_string();
    assert!(!volume.is_empty(), "no volume in the catalog");
    let path = rig.layout.storage.join(&volume);
    let away = rig.layout.storage.join(format!("{volume}.away"));
    std::fs::rename(&path, &away).unwrap_or_else(|e| panic!("{e}"));

    let restore = rig.restore(backup).unwrap_or_else(|e| panic!("{e}"));
    // the job waits for the mount: the mount request is in its log (the catalog row stays `R`)
    let deadline = Instant::now() + Duration::from_secs(60);
    let wanted = format!("Please mount read Volume \"{volume}\"");
    loop {
        let log = rig.job_log(restore);
        if log.contains(&wanted) {
            break;
        }
        let row = rig.job_row(restore).unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("no job {restore}"));
        assert!(
            Instant::now() < deadline && matches!(row.status, 'C' | 'R'),
            "the restore does not ask for the volume: {row:?}\n{log}\n{}",
            rig.diagnostics()
        );
        std::thread::sleep(Duration::from_millis(300));
    }
    // still waiting: the job has not ended
    let row = rig.job_row(restore).unwrap_or_else(|e| panic!("{e}")).unwrap_or_else(|| panic!("no job {restore}"));
    assert!(matches!(row.status, 'C' | 'R'), "the restore ended without the volume: {row:?}");

    std::fs::rename(&away, &path).unwrap_or_else(|e| panic!("{e}"));
    let reply = rig.command("mount storage=vsd slot=0").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("mount: {reply}");
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "restore after the mount: {row:?}\n{}\n{}", rig.job_log(restore), rig.diagnostics());
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore differs from the source:\n{diff}");
}

/// osd.
#[test]
#[ignore = "needs the osd binary"]
fn osd_read_waits_for_the_volume() {
    read_waits_for_the_volume(39671, onesimus_rig::prereq::target_profile_dir().join("osd"));
}

/// bareos-sd (reference).
#[test]
#[ignore = "needs /usr/sbin/bareos-sd"]
fn bareos_sd_read_waits_for_the_volume() {
    read_waits_for_the_volume(39661, "/usr/sbin/bareos-sd".into());
}
