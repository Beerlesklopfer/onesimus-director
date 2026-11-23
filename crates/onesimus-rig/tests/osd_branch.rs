// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Zweige (Branch): 3-2-1 im laufenden Betrieb (Plan
//! `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`, P2b; Entwurf
//! `docs/plans/P2B-ZWEIGE-ENTWURF.md`).
//!
//! Der Haupt-Job `VBackup` (Storage `vsd`, Device `VDev`, Pool `VPool`) hat im
//! Overlay `odir.d/job/VBackup.conf` ein `Branch = VBranch`: der Zweig-Job
//! `VBranch` schreibt über das Storage `vsd2` auf das zweite Device `VDev2`
//! (MediaType `VFile2`, Volumes in `storage2/`) in den Pool `VBranchPool`.
//! Ein Lauf von `VBackup` liest den Client **einmal**; osd schreibt den Stream
//! zweimal.
//!
//! | Test | Ports | Inhalt |
//! |---|---|---|
//! | `one_stream_two_jobs` | ab 39900 | Full und Incremental: beide Jobs enden `T`, `Job`/`File`/`JobMedia` je Zweig, zwei Pools und Devices, der FD liest einmal (eine `2800 End Job`-Zeile je Lauf), Restore aus jedem Zweig allein (und die Kette des Zweigs) ergibt den Quellbaum |
//! | `a_branch_that_cannot_be_written_does_not_stop_the_main_job` | ab 39910 | Device des Zweigs nicht in der SD-Konfiguration (Just-in-time-Reservierung scheitert beim ersten Datensatz): der Zweig endet mit Fehler, der Haupt-Job `T`, Restore des Haupt-Jobs stimmt |
//! | `a_branch_that_fails_while_writing_does_not_stop_the_main_job` | ab 39920 | Verzeichnis des Zweig-Devices nicht beschreibbar: der Zweig scheitert beim Schreiben, der Haupt-Job `T` |
//! | `a_canceled_branch_leaves_the_main_job_alone` | ab 39930 | Abbruch des Zweig-Jobs mitten im Lauf: nur er endet `A`, der Haupt-Job `T` |
//! | `the_branch_consolidates_to_a_virtual_full` | ab 39940 | Konsolidierung des Zweigs (Always Incremental): Full und Incremental des Zweig-Jobs auf `VDev2` werden von einem VirtualFull-Job (mit ausdrücklicher JobId-Liste, siehe BACKLOG ZW-1) in den Pool `VBranchOut` (`VDev`) zusammengelegt; Restore des VirtualFull ergibt den Quellbaum einschließlich der Datei des Incrementals |
//!
//! Start: `cargo test -p onesimus-rig --test osd_branch -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Grpc, Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(240);

/// A rig with osd on gRPC and the branch `VBranch` of `VBackup`; `extra` Director files.
fn start(port: u16, extra: Vec<(String, String)>, extra_odir_d: Vec<(String, String)>) -> Rig {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(port);
    options.sd = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    options.fd_grpc = Grpc::On;
    options.sd_grpc = Grpc::On;
    options.branch = true;
    options.extra_director = extra;
    options.extra_odir_d = extra_odir_d;
    Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"))
}

/// The first value of the first row of `sql`.
fn value(rig: &Rig, sql: &str) -> String {
    let out = rig.catalog().query(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    out.lines().next().unwrap_or_default().split('|').next().unwrap_or_default().trim().to_string()
}

/// Runs the console command `command` (a `run`), returns the JobId.
fn run(rig: &Rig, what: &str, command: &str) -> u32 {
    let reply = rig.command(command).unwrap_or_else(|e| panic!("{e}"));
    onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("{what}: no JobId\n{reply}"))
}

/// The branch job of main job `main` (`onesimus.job_branch`).
fn branch_of(rig: &Rig, main: u32) -> u32 {
    let id = value(rig, &format!("SELECT branch_job_id FROM onesimus.job_branch WHERE main_job_id = {main}"));
    id.parse().unwrap_or_else(|_| panic!("no branch of job {main} in onesimus.job_branch\n{}", rig.diagnostics()))
}

/// Waits for job `id`, returns its status letter.
fn finish(rig: &Rig, id: u32) -> char {
    rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}")).status
}

/// `Name|Level|JobStatus|JobFiles|JobBytes|Pool` of a job.
fn job_line(rig: &Rig, id: u32) -> String {
    rig.catalog()
        .query(&format!(
            "SELECT j.Name, j.Level, j.JobStatus, j.JobFiles, j.JobBytes, p.Name FROM Job j JOIN Pool p ON p.PoolId = j.PoolId WHERE j.JobId = {id}"
        ))
        .unwrap_or_else(|e| panic!("{e}"))
        .trim()
        .to_string()
}

/// The volumes (`VolumeName|MediaType`) the job `id` wrote to.
fn media_of(rig: &Rig, id: u32) -> String {
    rig.catalog()
        .query(&format!("SELECT m.VolumeName, m.MediaType FROM JobMedia jm JOIN Media m ON m.MediaId = jm.MediaId WHERE jm.JobId = {id} GROUP BY 1, 2 ORDER BY 1"))
        .unwrap_or_else(|e| panic!("{e}"))
        .trim()
        .to_string()
}

/// Number of `File` records of job `id`.
fn file_count(rig: &Rig, id: u32) -> i64 {
    value(rig, &format!("SELECT count(*) FROM File WHERE JobId = {id}")).parse().unwrap_or(-1)
}

/// How many times the FD ended a job of the Job resource `name` (`2800 End Job` in its log).
fn ends_of(rig: &Rig, name: &str) -> usize {
    let marker = format!("({name}.");
    rig.daemon_log("ofd").lines().filter(|l| l.contains(&marker) && l.contains("2800 End Job")).count()
}

/// Restores `jobids` (all files) into the rig's restore directory, which is empty before;
/// the restore job's status.
fn restore(rig: &Rig, jobids: &str) -> char {
    let _ = std::fs::remove_dir_all(&rig.layout.restore);
    std::fs::create_dir_all(&rig.layout.restore).unwrap_or_else(|e| panic!("{e}"));
    let id = run(rig, "restore", &format!("restore jobid={jobids} client=vfd where={} all done yes", rig.layout.restore.display()));
    finish(rig, id)
}

/// Full and Incremental: one FD run feeds both jobs.
#[test]
#[ignore = "needs the osd binary and initdb"]
fn one_stream_two_jobs() {
    let rig = start(39900, Vec::new(), Vec::new());

    // osd -t and odir -t know the branch
    let full = run(&rig, "full", "run job=VBackup level=Full yes");
    let branch_full = branch_of(&rig, full);
    assert_eq!(finish(&rig, full), 'T', "main job\n{}\n{}", rig.job_log(full), rig.diagnostics());
    assert_eq!(finish(&rig, branch_full), 'T', "branch job\n{}\n{}", rig.job_log(branch_full), rig.diagnostics());
    let (main_line, branch_line) = (job_line(&rig, full), job_line(&rig, branch_full));
    eprintln!("Full: {main_line}\n      {branch_line}");
    let fields = |line: &str| line.split('|').map(str::to_string).collect::<Vec<_>>();
    let (m, b) = (fields(&main_line), fields(&branch_line));
    assert_eq!((m[0].as_str(), m[1].as_str(), m[2].as_str(), m[5].as_str()), ("VBackup", "F", "T", "VPool"), "{main_line}");
    assert_eq!((b[0].as_str(), b[1].as_str(), b[2].as_str(), b[5].as_str()), ("VBranch", "F", "T", "VBranchPool"), "{branch_line}");
    assert!(m[3].parse::<i64>().unwrap_or(0) > 0, "{main_line}");
    // the main job's bytes are the FD's, the branch's those of the SD (record headers on top)
    let bytes = |f: &[String]| f[4].parse::<i64>().unwrap_or(0);
    assert_eq!(m[3], b[3], "both jobs carry the same files");
    assert!(bytes(&m[..]) > 0 && (bytes(&b[..]) - bytes(&m[..])).abs() * 100 < bytes(&m[..]), "bytes: {main_line} / {branch_line}");
    // every job has its own File records and its own volumes, on its own device
    let (main_files, branch_files) = (file_count(&rig, full), file_count(&rig, branch_full));
    assert!(main_files > 0 && main_files == branch_files, "File records: main {main_files}, branch {branch_files}");
    let (main_media, branch_media) = (media_of(&rig, full), media_of(&rig, branch_full));
    eprintln!("media: main {main_media}  branch {branch_media}");
    assert!(main_media.ends_with("|VFile") && branch_media.ends_with("|VFile2"), "{main_media} / {branch_media}");
    let volume = |media: &str| media.split('|').next().unwrap_or_default().to_string();
    assert!(rig.layout.storage.join(volume(&main_media)).is_file());
    assert!(rig.layout.root.join("storage2").join(volume(&branch_media)).is_file());
    assert!(!rig.layout.root.join("storage2").join(volume(&main_media)).exists());
    assert!(branch_media.starts_with("VBr-") && main_media.starts_with("VVol-"), "{main_media} / {branch_media}");
    // the pair is recorded in the Onesimus schema, not in a standard table
    assert_eq!(value(&rig, &format!("SELECT main_job_id FROM onesimus.job_branch WHERE branch_job_id = {branch_full}")), full.to_string());
    // the FD read once: one 2800 End Job for the run, none for the branch
    assert!(rig.fd_end_job(full).is_some(), "no End Job of the FD for the main job");
    assert!(rig.fd_end_job(branch_full).is_none(), "the branch job has no FD session of its own");
    assert_eq!(ends_of(&rig, "VBackup"), 1, "the FD ends one backup job for one run:\n{}", rig.daemon_log("ofd"));
    assert_eq!(ends_of(&rig, "VBranch"), 0, "the FD never sees the branch job");
    // each job log says what happened to it
    let (main_log, branch_log) = (rig.job_log(full), rig.job_log(branch_full));
    assert!(main_log.contains("takes this stream"), "main log:\n{main_log}");
    assert!(branch_log.contains("Using Device \"VDev2\" ("), "branch log:\n{branch_log}");
    assert!(main_log.contains("Using Device \"VDev\" ("), "main log:\n{main_log}");

    // restore from each job alone: the source tree
    assert_eq!(restore(&rig, &full.to_string()), 'T');
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the main job differs from the source:\n{diff}\n{}", rig.diagnostics());
    assert_eq!(restore(&rig, &branch_full.to_string()), 'T', "{}", rig.diagnostics());
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the branch job differs from the source:\n{diff}\n{}", rig.diagnostics());

    // an Incremental: both jobs take the new file
    std::thread::sleep(Duration::from_secs(2));
    std::fs::write(rig.layout.data.join("added-for-incremental.txt"), "added\n").unwrap_or_else(|e| panic!("{e}"));
    let inc = run(&rig, "incremental", "run job=VBackup level=Incremental yes");
    let branch_inc = branch_of(&rig, inc);
    assert_eq!(finish(&rig, inc), 'T', "{}\n{}", rig.job_log(inc), rig.diagnostics());
    assert_eq!(finish(&rig, branch_inc), 'T', "{}\n{}", rig.job_log(branch_inc), rig.diagnostics());
    let (m, b) = (fields(&job_line(&rig, inc)), fields(&job_line(&rig, branch_inc)));
    assert_eq!((m[1].as_str(), b[1].as_str(), m[0].as_str(), b[0].as_str()), ("I", "I", "VBackup", "VBranch"));
    assert_eq!(m[3], b[3], "the same files for both");
    assert_eq!((ends_of(&rig, "VBackup"), ends_of(&rig, "VBranch")), (2, 0), "one FD session for each of the two runs");
    // the restore of the branch's own chain: its Full and its Incremental
    assert_eq!(restore(&rig, &format!("{branch_full},{branch_inc}")), 'T', "{}", rig.diagnostics());
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the branch chain differs from the source:\n{diff}\n{}", rig.diagnostics());
    let added = rig.layout.restored_data().join("added-for-incremental.txt");
    assert_eq!(std::fs::read_to_string(&added).unwrap_or_else(|e| panic!("{}: {e}", added.display())), "added\n");
}

/// The branch's Storage names a device the SD does not have: its job fails, the main job does not.
#[test]
#[ignore = "needs the osd binary and initdb"]
fn a_branch_that_cannot_be_written_does_not_stop_the_main_job() {
    let broken = (
        "storage/vsd2.conf".to_string(),
        "Storage {\n  Name = vsd2\n  Address = 127.0.0.1\n  SD Port = 39914\n  Password = \"vsd-pw\"\n  Device = NoSuchDevice\n  \
         Media Type = VFile2\n  Maximum Concurrent Jobs = 10\n}\n"
            .to_string(),
    );
    let rig = start(39910, vec![broken], Vec::new());
    let full = run(&rig, "full", "run job=VBackup level=Full yes");
    let branch = branch_of(&rig, full);
    assert_eq!(finish(&rig, full), 'T', "the main job must not notice:\n{}\n{}", rig.job_log(full), rig.diagnostics());
    let status = finish(&rig, branch);
    assert!(matches!(status, 'E' | 'f'), "branch status {status}\n{}\n{}", rig.job_log(branch), rig.diagnostics());
    let branch_log = rig.job_log(branch);
    assert!(branch_log.contains("Could not reserve any device"), "the branch log says why:\n{branch_log}");
    assert!(file_count(&rig, full) > 0 && file_count(&rig, branch) == 0);
    assert_eq!(restore(&rig, &full.to_string()), 'T');
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the main job differs from the source:\n{diff}");
}

/// The branch's device directory is not writable: the branch fails while writing, the main job goes on.
#[test]
#[ignore = "needs the osd binary and initdb"]
fn a_branch_that_fails_while_writing_does_not_stop_the_main_job() {
    use std::os::unix::fs::PermissionsExt;
    let rig = start(39920, Vec::new(), Vec::new());
    let dir = rig.layout.root.join("storage2");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap_or_else(|e| panic!("{e}"));
    if std::fs::write(dir.join("probe"), b"x").is_ok() {
        // root ignores the permissions: nothing to test here
        let _ = std::fs::remove_file(dir.join("probe"));
        eprintln!("skipped: the user can write into a 0500 directory");
        return;
    }
    let full = run(&rig, "full", "run job=VBackup level=Full yes");
    let branch = branch_of(&rig, full);
    assert_eq!(finish(&rig, full), 'T', "the main job must not notice:\n{}\n{}", rig.job_log(full), rig.diagnostics());
    let status = finish(&rig, branch);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(status, 'E' | 'f'), "branch status {status}\n{}\n{}", rig.job_log(branch), rig.diagnostics());
    assert_eq!(restore(&rig, &full.to_string()), 'T');
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the main job differs from the source:\n{diff}");
}

/// A branch job canceled while the stream runs ends `A`; the main job ends `T`.
#[test]
#[ignore = "needs the osd binary and initdb"]
fn a_canceled_branch_leaves_the_main_job_alone() {
    // BigBackup is slow (Maximum Bandwidth): time to cancel; its branch is VBranch2, a second
    // job of the same kind as VBranch
    let branch2 = (
        "job/VBranch2.conf".to_string(),
        "Job {\n  Name = VBranch2\n  JobDefs = VDefs\n  Type = Backup\n  Level = Incremental\n  Storage = vsd2\n  Pool = VBranchPool\n}\n".to_string(),
    );
    let overlay = ("job/BigBackup.conf".to_string(), "Job {\n  Name = BigBackup\n  Branch = VBranch2\n}\n".to_string());
    let rig = start(39930, vec![branch2], vec![overlay]);
    let main = run(&rig, "big", "run job=BigBackup level=Full yes");
    let branch = branch_of(&rig, main);
    rig.wait_running(branch, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    std::thread::sleep(Duration::from_secs(3));
    let reply = rig.command(&format!("cancel jobid={branch} yes")).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("{reply}");
    let status = finish(&rig, branch);
    assert_eq!(status, 'A', "branch\n{}\n{}", rig.job_log(branch), rig.diagnostics());
    let main_status = finish(&rig, main);
    assert_eq!(main_status, 'T', "main\n{}\n{}", rig.job_log(main), rig.diagnostics());
}

/// Always Incremental on the branch: the Full and the Incremental of the branch job
/// (volumes on `VDev2`) are consolidated by a VirtualFull job of the branch's pool into the
/// pool `VBranchOut` (device `VDev`); the main job's chain is not touched.
#[test]
#[ignore = "needs the osd binary and initdb"]
fn the_branch_consolidates_to_a_virtual_full() {
    let pools = [
        (
            "pool/VBranchPool.conf".to_string(),
            "Pool {\n  Name = VBranchPool\n  Pool Type = Backup\n  Label Format = \"VBr-\"\n  Maximum Volume Bytes = 1G\n  Storage = vsd2\n  \
             Next Pool = VBranchOut\n}\n"
                .to_string(),
        ),
        (
            "pool/VBranchOut.conf".to_string(),
            "Pool {\n  Name = VBranchOut\n  Pool Type = Backup\n  Label Format = \"VOut-\"\n  Maximum Volume Bytes = 1G\n  Storage = vsd\n}\n".to_string(),
        ),
        (
            "job/VBranchFull.conf".to_string(),
            "Job {\n  Name = VBranchFull\n  JobDefs = VDefs\n  Type = Backup\n  Level = VirtualFull\n  Accurate = yes\n  Storage = vsd2\n  \
             Pool = VBranchPool\n}\n"
                .to_string(),
        ),
    ];
    let rig = start(39940, pools.to_vec(), Vec::new());
    let full = run(&rig, "full", "run job=VBackup level=Full yes");
    let branch_full = branch_of(&rig, full);
    assert_eq!((finish(&rig, full), finish(&rig, branch_full)), ('T', 'T'), "{}", rig.diagnostics());
    std::thread::sleep(Duration::from_secs(2));
    std::fs::write(rig.layout.data.join("added-for-incremental.txt"), "added\n").unwrap_or_else(|e| panic!("{e}"));
    let inc = run(&rig, "incremental", "run job=VBackup level=Incremental yes");
    let branch_inc = branch_of(&rig, inc);
    assert_eq!((finish(&rig, inc), finish(&rig, branch_inc)), ('T', 'T'), "{}", rig.diagnostics());
    std::thread::sleep(Duration::from_secs(2));
    // the jobs to consolidate are named: the automatic choice (client and FileSet, as in Bareos)
    // would take the main job's Incremental for the chain too (BACKLOG ZW-1)
    let vfull = run(&rig, "virtual full", &format!("run job=VBranchFull level=VirtualFull jobid={branch_full},{branch_inc} yes"));
    assert_eq!(finish(&rig, vfull), 'T', "VirtualFull\n{}\n{}", rig.job_log(vfull), rig.diagnostics());
    let media = media_of(&rig, vfull);
    eprintln!("VirtualFull {vfull}: {media}");
    assert!(media.starts_with("VOut-") && media.ends_with("|VFile"), "the consolidated job lies in the Next Pool on VDev: {media}");
    let _ = std::fs::remove_dir_all(&rig.layout.restore);
    std::fs::create_dir_all(&rig.layout.restore).unwrap_or_else(|e| panic!("{e}"));
    let restore_id = run(&rig, "restore", &format!("restore jobid={vfull} client=vfd storage=vsd where={} all done yes", rig.layout.restore.display()));
    assert_eq!(finish(&rig, restore_id), 'T', "{}", rig.diagnostics());
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the VirtualFull differs from the source:\n{diff}\n{}", rig.diagnostics());
}
