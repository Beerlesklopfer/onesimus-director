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
fn copy_and_restore(base_port: u16, reader: std::path::PathBuf, writer: Option<std::path::PathBuf>) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(reader);
    options.copy_same_sd = writer.is_none();
    options.copy_target = writer;
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
    copy_and_restore(39561, "/usr/sbin/bareos-sd".into(), Some(osd()));
}

/// Backup with osd, Copy osd to osd (osd is the reading SD: `replicate`),
/// Restore of the copy.
#[test]
#[ignore = "needs the osd binary"]
fn osd_copies_to_osd_and_the_copy_restores() {
    copy_and_restore(39581, osd(), Some(osd()));
}

/// Backup and Copy inside one osd (read device `VDev`, write device
/// `VDev2`: the local copy), Restore of the copy.
#[test]
#[ignore = "needs the osd binary"]
fn osd_copies_inside_one_sd_and_the_copy_restores() {
    copy_and_restore(39621, osd(), None);
}

/// The same inside one bareos-sd (reference).
#[test]
#[ignore = "needs /usr/sbin/bareos-sd"]
fn bareos_sd_copies_inside_one_sd_and_the_copy_restores() {
    copy_and_restore(39631, "/usr/sbin/bareos-sd".into(), None);
}

/// Backup with osd, Copy to bareos-sd (osd replicates to the original SD),
/// Restore of the copy from bareos-sd.
#[test]
#[ignore = "needs /usr/sbin/bareos-sd and the osd binary"]
fn osd_copies_to_bareos_sd_and_the_copy_restores() {
    copy_and_restore(39591, osd(), Some("/usr/sbin/bareos-sd".into()));
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

/// Full and Incremental backup, then a VirtualFull job inside one SD (read
/// device `VDev`, write device `VDev2` of the Next Pool), restore of the
/// VirtualFull job compared with the source tree (including the file
/// added for the Incremental job).
fn virtual_full(base_port: u16, sd: std::path::PathBuf) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(sd);
    options.copy_same_sd = true;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let run = |what: &str, command: &str| -> u32 {
        let reply = rig.command(command).unwrap_or_else(|e| panic!("{e}"));
        let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("{what}: no JobId\n{reply}"));
        let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(row.status, 'T', "{what}: {row:?}\n{}\n{}", rig.job_log(id), rig.diagnostics());
        id
    };
    run("full", "run job=VBackup level=Full yes");
    std::thread::sleep(Duration::from_secs(2));
    std::fs::write(rig.layout.data.join("added-for-incremental.txt"), "added\n").unwrap_or_else(|e| panic!("{e}"));
    run("incremental", "run job=VBackup level=Incremental yes");
    std::thread::sleep(Duration::from_secs(2));
    let vfull = run("virtual full", "run job=VFull level=VirtualFull yes");
    let log = rig.job_log(vfull);
    eprintln!("VirtualFull {vfull}:\n{log}");
    let reply = rig
        .command(&format!("restore jobid={vfull} client=vfd storage=vsd2 where={} all done yes", rig.layout.restore.display()))
        .unwrap_or_else(|e| panic!("{e}"));
    let restore = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}\n{}", rig.job_log(restore), rig.diagnostics());
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the VirtualFull job differs from the source:\n{diff}");
}

/// VirtualFull with bareos-sd (reference).
#[test]
#[ignore = "needs /usr/sbin/bareos-sd"]
fn virtual_full_with_bareos_sd() {
    virtual_full(39601, "/usr/sbin/bareos-sd".into());
}

/// VirtualFull with osd.
#[test]
#[ignore = "needs the osd binary"]
fn virtual_full_with_osd() {
    virtual_full(39611, osd());
}

/// Copy with clear text between the SDs (`ssl=0`): the writing SD has no
/// TLS, the reading SD connects without TLS-PSK and authenticates by CRAM-MD5
/// only.
fn copy_in_clear_text(base_port: u16, reader: std::path::PathBuf) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(reader);
    options.copy_target = Some(osd());
    options.copy_plain = true;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert!(row.status == 'T' && row.files == ENTRIES, "Backup: {row:?}\n{}", rig.job_log(backup));
    let copy = rig.run_job("VCopy").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(copy, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "Copy: {row:?}\n{}\n{}", rig.job_log(copy), rig.diagnostics());
    let deadline = std::time::Instant::now() + JOB_TIMEOUT;
    loop {
        let states = rig.catalog().query("SELECT JobStatus FROM Job WHERE JobId > 2 ORDER BY JobId").unwrap_or_default();
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
    assert!(jobs.contains(&format!("|T|{ENTRIES}")), "no finished copy of {ENTRIES} files:\n{jobs}\n{}", rig.diagnostics());
    let log = rig.job_log(copy);
    assert!(log.contains("Copying OK"), "{log}");
}

/// bareos-sd reads and replicates to osd without TLS.
#[test]
#[ignore = "needs /usr/sbin/bareos-sd and the osd binary"]
fn bareos_sd_copies_to_osd_in_clear_text() {
    copy_in_clear_text(39641, "/usr/sbin/bareos-sd".into());
}

/// osd reads and replicates to osd without TLS.
#[test]
#[ignore = "needs the osd binary"]
fn osd_copies_to_osd_in_clear_text() {
    copy_in_clear_text(39651, osd());
}

/// Backup, Copy to a second device of the same SD (media type `VFile2`),
/// then a restore of the copy with `storage=vsd` (the first device, media
/// type `VFile`): the SD reads from the device that has the volume
/// (`Changing read device`).
fn restore_of_the_copy_changes_the_read_device(base_port: u16, sd: std::path::PathBuf) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(sd);
    options.copy_same_sd = true;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "Backup: {row:?}\n{}", rig.job_log(backup));
    let copy = rig.run_job("VCopy").unwrap_or_else(|e| panic!("{e}"));
    rig.wait_job(copy, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let deadline = std::time::Instant::now() + JOB_TIMEOUT;
    let new_job: u32 = loop {
        let jobs = rig
            .catalog()
            .query("SELECT j.JobId, j.JobStatus FROM Job j JOIN Pool p ON p.PoolId = j.PoolId WHERE p.Name = 'VCopyPool' AND j.Type = 'C' ORDER BY 1")
            .unwrap_or_default();
        if let Some(line) = jobs.lines().next().filter(|l| l.ends_with("|T")) {
            break line.split('|').next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        }
        assert!(std::time::Instant::now() < deadline, "the copy does not end:\n{jobs}\n{}", rig.diagnostics());
        std::thread::sleep(Duration::from_secs(2));
    };
    let reply = rig
        .command(&format!("restore jobid={new_job} client=vfd storage=vsd where={} all done yes", rig.layout.restore.display()))
        .unwrap_or_else(|e| panic!("{e}"));
    let restore = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(restore);
    eprintln!("restore {restore}: {row:?}\n{log}");
    assert_eq!(row.status, 'T', "restore: {row:?}\n{log}\n{}", rig.diagnostics());
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore differs from the source:\n{diff}");
}

/// bareos-sd (reference).
#[test]
#[ignore = "needs /usr/sbin/bareos-sd"]
fn bareos_sd_changes_the_read_device() {
    restore_of_the_copy_changes_the_read_device(39681, "/usr/sbin/bareos-sd".into());
}

/// osd.
#[test]
#[ignore = "needs the osd binary"]
fn osd_changes_the_read_device() {
    restore_of_the_copy_changes_the_read_device(39691, osd());
}

/// `.volume_damaged`: a backup and its copy on a second volume; the report shows the
/// copy, `yes` sets the Volume to Error and promotes the copy (the original job goes).
#[test]
#[ignore = "needs the osd binary"]
fn a_damaged_volume_promotes_the_copy() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39731);
    options.sd = Some(osd());
    options.copy_same_sd = true;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}")).status, 'T');
    let copy = rig.run_job("VCopy").unwrap_or_else(|e| panic!("{e}"));
    rig.wait_job(copy, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let deadline = std::time::Instant::now() + JOB_TIMEOUT;
    let new_job: u32 = loop {
        let jobs = rig.catalog().query("SELECT JobId, JobStatus FROM Job WHERE Type = 'C' ORDER BY 1").unwrap_or_default();
        if let Some(line) = jobs.lines().next().filter(|l| l.ends_with("|T")) {
            break line.split('|').next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        }
        assert!(std::time::Instant::now() < deadline, "the copy does not end:\n{jobs}\n{}", rig.diagnostics());
        std::thread::sleep(Duration::from_secs(2));
    };
    let volume = rig.catalog().query("SELECT m.VolumeName FROM JobMedia jm JOIN Media m ON m.MediaId = jm.MediaId WHERE jm.JobId = 1 LIMIT 1").unwrap_or_default().trim().to_string();
    assert!(volume.starts_with("VVol-"), "{volume}");

    // the report changes nothing
    let report = rig.command(&format!(".volume_damaged volume={volume}")).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("{report}");
    assert!(report.contains("JobId      1") && report.contains(&format!("copy: JobId {new_job}")), "{report}");
    assert!(report.contains("dry run"), "{report}");
    let status = rig.catalog().query(&format!("SELECT VolStatus FROM Media WHERE VolumeName = '{volume}'")).unwrap_or_default();
    assert!(!status.contains("Error"), "the dry run changed the volume: {status}");

    // yes: Error, the original goes, the copy is a Backup job
    let done = rig.command(&format!(".volume_damaged volume={volume} yes")).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("{done}");
    assert!(done.contains("is now in status Error") && done.contains("their copies are Backup jobs now"), "{done}");
    let status = rig.catalog().query(&format!("SELECT VolStatus FROM Media WHERE VolumeName = '{volume}'")).unwrap_or_default();
    assert!(status.contains("Error"), "{status}");
    let jobs = rig.catalog().query(&format!("SELECT JobId, Type FROM Job WHERE JobId IN (1, {new_job}) ORDER BY 1")).unwrap_or_default();
    assert_eq!(jobs.trim(), format!("{new_job}|B"), "the original is gone, the copy is a backup:\n{jobs}");
}
