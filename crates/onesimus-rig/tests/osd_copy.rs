//! Copy between two Storage Daemons with osd as the writing side
//! (`listen`, `Hello Start Storage Job`, `start replicate`, `replicate
//! data`, `end replicate`): bareos-sd reads the volume of a backup and
//! replicates it to osd (Next Pool `VCopyPool`), then the copy is restored
//! from osd and compared with the source tree.
//!
//! A second test copies two backups with one control job (two replications to
//! one osd).
//!
//! Start: `cargo test -p onesimus-rig --test osd_copy -- --ignored --nocapture`
//! (needs `/usr/sbin/bareos-sd`; ports from 39561).

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(240);

/// Entries of the rig's test tree.
const ENTRIES: u64 = 13;

/// The osd binary of the workspace.
fn osd() -> std::path::PathBuf {
    onesimus_rig::prereq::target_profile_dir().join("osd")
}

/// Backup on the reading SD `reader`, Copy to the writing SD `writer`
/// (`copy_target`), Restore of the copy from the writer, compared with the
/// source tree. `media` is the media type the volume of the copy carries.
fn copy_and_restore(base_port: u16, reader: std::path::PathBuf, writer: std::path::PathBuf) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(reader);
    options.copy_target = Some(writer);
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert!(row.status == 'T' && row.files == ENTRIES, "Backup: {row:?}\n{}", rig.job_log(backup));

    let copy = rig.run_job("VCopy").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(copy, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(copy);
    eprintln!("copy control job {copy}: {row:?}\n{log}");
    assert_eq!(row.status, 'T', "Copy: {row:?}\n{log}\n{}", rig.diagnostics());
    assert!(log.contains("Copying OK"), "no \"Copying OK\":\n{log}");

    // The control job ends when the copy jobs are started: wait for the new backup job
    let deadline = std::time::Instant::now() + JOB_TIMEOUT;
    loop {
        let states = rig.catalog().query("SELECT JobStatus FROM Job WHERE JobId > 2 ORDER BY JobId").unwrap_or_default();
        if !states.is_empty() && states.lines().all(|l| matches!(l.trim(), "T" | "E" | "f" | "A" | "W")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the copy jobs do not end:\n{states}\n{}", rig.diagnostics());
        std::thread::sleep(Duration::from_secs(2));
    }
    // The new job (type C, copy of a backup) in the copy pool: all entries, on a volume of osd
    let jobs = rig
        .catalog()
        .query("SELECT j.JobId, j.JobStatus, j.JobFiles FROM Job j JOIN Pool p ON p.PoolId = j.PoolId WHERE p.Name = 'VCopyPool' AND j.Type = 'C' ORDER BY 1")
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("jobs of VCopyPool:\n{jobs}");
    let new_job: u32 = jobs.lines().next().and_then(|l| l.split('|').next()).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
    assert!(new_job != 0 && jobs.contains(&format!("|T|{ENTRIES}")), "no finished copy of {ENTRIES} files:\n{jobs}\n{}", rig.diagnostics());
    let media = rig
        .catalog()
        .query("SELECT VolumeName, MediaType, VolStatus, VolJobs FROM Media WHERE MediaType = 'VFile2'")
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("media of osd:\n{media}");
    let volume = media.split('|').next().unwrap_or_default().trim().to_string();
    assert!(!volume.is_empty(), "no volume of media type VFile2:\n{media}");
    let path = rig.layout.root.join("storage2").join(&volume);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let summary = bareos_format::volume::summarize(&bytes).unwrap_or_else(|e| panic!("Volume {}: {e}", path.display()));
    assert_eq!(summary.attribute_file_indexes, (1..=ENTRIES as i32).collect::<Vec<_>>(), "attributes: {summary}");

    // Restore of the copy from osd, compared with the source tree
    let reply = rig.command(&format!("restore jobid={new_job} client=vfd storage=vsd2 where={} all done yes", rig.layout.restore.display())).unwrap_or_else(|e| panic!("{e}"));
    let restore = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}\n{}", rig.job_log(restore), rig.diagnostics());
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the copy differs from the source:\n{diff}");
}

/// Backup with bareos-sd, Copy to osd, Restore of the copy from osd.
#[test]
#[ignore = "needs /usr/sbin/bareos-sd and the osd binary"]
fn bareos_sd_copies_to_osd_and_the_copy_restores() {
    copy_and_restore(39561, "/usr/sbin/bareos-sd".into(), osd());
}

/// Backup with osd, Copy osd to osd (osd is the reading SD: `replicate`),
/// Restore of the copy.
#[test]
#[ignore = "needs the osd binary"]
fn osd_copies_to_osd_and_the_copy_restores() {
    copy_and_restore(39581, osd(), osd());
}

/// Backup with osd, Copy to bareos-sd (osd replicates to the original SD),
/// Restore of the copy from bareos-sd.
#[test]
#[ignore = "needs /usr/sbin/bareos-sd and the osd binary"]
fn osd_copies_to_bareos_sd_and_the_copy_restores() {
    copy_and_restore(39591, osd(), "/usr/sbin/bareos-sd".into());
}

/// Two backups, one Copy control job: the two copy jobs replicate to osd
/// one after the other or overlapping (own job, own key, own connection each; overlap not checked).
#[test]
#[ignore = "needs /usr/sbin/bareos-sd and the osd binary"]
fn two_backups_are_copied_to_osd_by_one_control_job() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39571);
    options.sd = Some("/usr/sbin/bareos-sd".into());
    options.copy_target = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    for _ in 0..2 {
        let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
        let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
        assert!(row.status == 'T' && row.files == ENTRIES, "Backup: {row:?}\n{}", rig.job_log(backup));
    }
    let copy = rig.run_job("VCopy").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(copy, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "Copy: {row:?}\n{}", rig.job_log(copy));
    let log = rig.job_log(copy);
    assert!(log.contains("The following 2 JobIds were chosen to be copied"), "two backups not chosen:\n{log}");
    let deadline = std::time::Instant::now() + JOB_TIMEOUT;
    loop {
        let states = rig.catalog().query("SELECT JobStatus FROM Job WHERE JobId > 3 ORDER BY JobId").unwrap_or_default();
        if !states.is_empty() && states.lines().all(|l| matches!(l.trim(), "T" | "E" | "f" | "A" | "W")) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "the copy jobs do not end:\n{states}\n{}", rig.diagnostics());
        std::thread::sleep(Duration::from_secs(2));
    }
    let jobs = rig
        .catalog()
        .query("SELECT j.JobId, j.JobStatus, j.JobFiles FROM Job j JOIN Pool p ON p.PoolId = j.PoolId WHERE p.Name = 'VCopyPool' AND j.Type = 'C' ORDER BY 1")
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("jobs of VCopyPool:\n{jobs}");
    let done = jobs.lines().filter(|l| l.ends_with(&format!("|T|{ENTRIES}"))).count();
    assert_eq!(done, 2, "two finished copies expected:\n{jobs}\n{}", rig.diagnostics());
}
