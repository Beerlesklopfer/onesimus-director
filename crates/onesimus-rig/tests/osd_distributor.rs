// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Der Verteiler (Distributor) im osd: Routen über zwei File-Devices
//! (Plan `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`, P2).
//!
//! Der Director sieht einen Storage `vsd` mit `Device = VDist` und dem
//! MediaType `VDistMT`. Im SD (`bareos-sd.d`) gibt es zwei Devices `VDev`
//! (MediaType `VFile`, Volumes in `storage/`) und `VDev2` (`VFile2`,
//! `storage2/`); der Verteiler steht in `<root>/onesimus/osd.d/distributor/VDist.conf`:
//! Full nach `VDev`, alles Übrige nach `VDev2`.
//!
//! | Test | Ports | Inhalt |
//! |---|---|---|
//! | `full_goes_to_one_device_incremental_to_the_other` | ab 39801 | Full → `VDev`, Incremental → `VDev2` (Route nach Level), Volumes mit dem MediaType des Ziels; Restore des Incrementals allein (Volume auf `VDev2`) und der Kette aus beiden Devices, verglichen mit dem Quellbaum |
//! | `restore_waits_for_a_busy_target` | ab 39881 | Kette Full (`VDev`) + Incremental (`VDev2`); während BigBackup per Job-Route nach `VDev2` schreibt, wartet der Restore der Kette auf `VDev2` und ist danach vollständig (VT-1) |
//! | `copy_and_migrate_routes` | ab 39841 | Routen nach `Type = Copy`/`Migrate` → `VDev2`, Backup → `VDev`; Copy und Migrate im selben SD über den Verteiler (Schreib-Storage `vdist2`, eine zweite Director-Ressource auf `VDist`) (lesen von `VDev`, schreiben nach `VDev2`); Restore der Kopie über einen Director-Storage mit `Device = VDev, VDev2` (VT-3) |
//!
//! Start: `cargo test -p onesimus-rig --test osd_distributor -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Grpc, Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(240);

/// Runs the console command `command` (a `run` or `restore`), waits for the job; its id.
fn run(rig: &Rig, what: &str, command: &str) -> u32 {
    let reply = rig.command(command).unwrap_or_else(|e| panic!("{e}"));
    let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("{what}: no JobId\n{reply}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{what}: {row:?}\n{}\n{}", rig.job_log(id), rig.diagnostics());
    id
}

/// The media (`VolumeName|MediaType`) the job `job` wrote to.
fn media_of(rig: &Rig, job: u32) -> String {
    rig.catalog()
        .query(&format!("SELECT m.VolumeName, m.MediaType FROM JobMedia jm JOIN Media m ON m.MediaId = jm.MediaId WHERE jm.JobId = {job} ORDER BY 1"))
        .unwrap_or_else(|e| panic!("{e}"))
}

/// Full → `VDev`, Incremental → `VDev2`; restore of the chain from both.
#[test]
#[ignore = "needs the osd binary and initdb"]
fn full_goes_to_one_device_incremental_to_the_other() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39801);
    options.sd = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    options.fd_grpc = Grpc::On;
    options.sd_grpc = Grpc::On;
    options.distributor = true;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    // osd -t knows the distributor (the same files the daemon reads)
    let osd = onesimus_rig::prereq::target_profile_dir().join("osd");
    let check = std::process::Command::new(&osd)
        .arg("-t")
        .arg("-c")
        .arg(&rig.layout.sd_conf)
        .arg("--overlay")
        .arg(rig.layout.root.join("onesimus"))
        .output()
        .unwrap_or_else(|e| panic!("{}: {e}", osd.display()));
    assert!(check.status.success(), "osd -t: {}{}", String::from_utf8_lossy(&check.stdout), String::from_utf8_lossy(&check.stderr));

    let full = run(&rig, "full", "run job=VBackup level=Full yes");
    std::thread::sleep(Duration::from_secs(2));
    std::fs::write(rig.layout.data.join("added-for-incremental.txt"), "added\n").unwrap_or_else(|e| panic!("{e}"));
    let incremental = run(&rig, "incremental", "run job=VBackup level=Incremental yes");

    // each job wrote a volume of its target's media type, into the target's directory
    let (full_media, inc_media) = (media_of(&rig, full), media_of(&rig, incremental));
    eprintln!("Full {full}: {full_media}Incremental {incremental}: {inc_media}");
    let full_volume = full_media.split('|').next().unwrap_or_default().trim().to_string();
    let inc_volume = inc_media.split('|').next().unwrap_or_default().trim().to_string();
    assert!(full_media.trim().ends_with("|VFile") && inc_media.trim().ends_with("|VFile2"), "media types:\n{full_media}{inc_media}");
    assert!(rig.layout.storage.join(&full_volume).is_file(), "the Full volume is not in storage/");
    assert!(rig.layout.root.join("storage2").join(&inc_volume).is_file(), "the Incremental volume is not in storage2/");
    assert!(!rig.layout.storage.join(&inc_volume).exists() && !rig.layout.root.join("storage2").join(&full_volume).exists());
    let log = rig.job_log(incremental);
    assert!(log.contains("Using Device \"VDev2\""), "Incremental log:\n{log}");
    assert!(rig.job_log(full).contains("Using Device \"VDev\""), "Full log:\n{}", rig.job_log(full));

    // status storage shows the distributor and its routes
    let status = rig.command("status storage=vsd").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("{status}");
    assert!(status.contains("Distributor \"VDist\"") && status.contains("Route 1: Level = Full -> Device \"VDev\""), "{status}");
    assert!(status.contains("Route 2: always -> Device \"VDev2\""), "{status}");

    // the Incremental alone: its volume is on VDev2, the Director names the distributor
    let alone = rig.layout.root.join("restore-incremental");
    run(&rig, "restore of the incremental", &format!("restore jobid={incremental} client=vfd where={} all done yes", alone.display()));
    let restored = alone.join(rig.layout.data.strip_prefix("/").unwrap_or(&rig.layout.data)).join("added-for-incremental.txt");
    assert_eq!(std::fs::read_to_string(&restored).unwrap_or_else(|e| panic!("{}: {e}", restored.display())), "added\n");

    // the chain: the Full from VDev, the Incremental from VDev2 in one restore job
    let chain = run(&rig, "restore of the chain", &format!("restore jobid={full},{incremental} client=vfd where={} all done yes", rig.layout.restore.display()));
    let chain_log = rig.job_log(chain);
    eprintln!("restore {chain}:\n{chain_log}");
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the chain differs from the source:\n{diff}\n{}", rig.diagnostics());
    assert!(chain_log.contains("Media Type change"), "the job log shows no change of the read device:\n{chain_log}");
}

/// Runs the Copy or Migrate control job of `command` and waits for the jobs it queued; its id.
fn run_with_queued(rig: &Rig, what: &str, command: &str) -> u32 {
    let control = run(rig, what, command);
    let log = rig.job_log(control);
    for queued in log.lines().filter_map(|l| l.split("Job queued. JobId=").nth(1)).filter_map(|v| v.trim().parse::<u32>().ok()) {
        let row = rig.wait_job(queued, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(row.status, 'T', "{what}: job {queued} {row:?}\n{}\n{}", rig.job_log(queued), rig.diagnostics());
    }
    control
}

/// The new jobs of a Copy or Migrate: `JobId|Type|VolumeName|MediaType` of the jobs with a
/// PriorJobId (copies `C`, migrated `B`).
fn written_by_copies(rig: &Rig) -> String {
    rig.catalog()
        .query("SELECT DISTINCT j.JobId, j.Type, m.VolumeName, m.MediaType FROM Job j JOIN JobMedia jm USING (JobId) \
                JOIN Media m USING (MediaId) WHERE j.PriorJobId <> 0 ORDER BY j.JobId")
        .unwrap_or_else(|e| panic!("{e}"))
}

/// Copy and Migrate go by their `Type` route to `VDev2` while backups go to `VDev`; the copy is
/// restored through a Director Storage that names both devices (`Device = VDev, VDev2`).
#[test]
#[ignore = "needs the osd binary and initdb"]
fn copy_and_migrate_routes() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39841);
    options.sd = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    options.distributor = true;
    options.distributor_routes = vec![
        "Route { Type = Copy; Device = VDev2 }".into(),
        "Route { Type = Migrate; Device = VDev2 }".into(),
        "Route { Device = VDev }".into(),
    ];
    let both = format!(
        "Storage {{\n  Name = vboth\n  Address = 127.0.0.1\n  SD Port = {}\n  Password = \"vsd-pw\"\n  Device = VDev, VDev2\n  \
         Media Type = VFile2\n  Maximum Concurrent Jobs = 10\n}}\n",
        39841 + 2
    );
    // a second Director Storage on the Distributor: the Director refuses a Copy with the same
    // read and write Storage resource (like bareos-dir)
    let second = format!(
        "Storage {{\n  Name = vdist2\n  Address = 127.0.0.1\n  SD Port = {}\n  Password = \"vsd-pw\"\n  Device = VDist\n  \
         Media Type = VDistMT\n  Maximum Concurrent Jobs = 10\n}}\n",
        39841 + 2
    );
    options.extra_director = vec![
        ("storage/vboth.conf".into(), both),
        ("storage/vdist2.conf".into(), second),
        (
            "pool/VCopyPool.conf".into(),
            "Pool {\n  Name = VCopyPool\n  Pool Type = Backup\n  Label Format = \"VCopy-\"\n  Maximum Volume Bytes = 1G\n  Storage = vdist2\n}\n".into(),
        ),
        (
            "pool/VPool.conf".into(),
            "Pool {\n  Name = VPool\n  Pool Type = Backup\n  Label Format = \"VVol-\"\n  Maximum Volume Bytes = 1G\n  Next Pool = VCopyPool\n}\n".into(),
        ),
        ("job/VCopy.conf".into(), "Job {\n  Name = VCopy\n  JobDefs = VDefs\n  Type = Copy\n  Selection Type = PoolUncopiedJobs\n}\n".into()),
        (
            "job/VMigrate.conf".into(),
            "Job {\n  Name = VMigrate\n  JobDefs = VDefs\n  Type = Migrate\n  Selection Type = SQLQuery\n  \
             Selection Pattern = \"SELECT MAX(JobId) FROM Job WHERE Type='B' AND Name='VBackup'\"\n}\n".into(),
        ),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let first = run(&rig, "backup", "run job=VBackup level=Full yes");
    assert!(media_of(&rig, first).trim().ends_with("|VFile"), "backup: {}", media_of(&rig, first));
    let copy = run_with_queued(&rig, "copy", "run job=VCopy yes");
    let copied = written_by_copies(&rig);
    eprintln!("copy {copy}:\n{}\n{copied}", rig.job_log(copy + 1));
    assert!(copied.lines().count() == 1 && copied.contains("|C|VCopy-") && copied.trim().ends_with("|VFile2"), "copy wrote:\n{copied}");
    let copy_job: u32 = copied.split('|').next().and_then(|v| v.trim().parse().ok()).unwrap_or_else(|| panic!("{copied}"));
    let copy_volume = copied.split('|').nth(2).unwrap_or_default().to_string();
    assert!(rig.layout.root.join("storage2").join(&copy_volume).is_file(), "the copy's volume is not in storage2/");

    let second = run(&rig, "second backup", "run job=VBackup level=Full yes");
    let migrate = run_with_queued(&rig, "migrate", "run job=VMigrate yes");
    let written = written_by_copies(&rig);
    eprintln!("migrate {migrate}:\n{}\n{written}", rig.job_log(migrate));
    let migrated: Vec<&str> = written.lines().filter(|l| l.contains("|B|")).collect();
    assert!(migrated.len() == 1 && migrated[0].trim().ends_with("|VFile2"), "migrate wrote:\n{written}");
    let prior = rig.catalog().query(&format!("SELECT Type FROM Job WHERE JobId={second}")).unwrap_or_default();
    assert_eq!(prior.trim(), "M", "the migrated job is not marked migrated");

    // the copy through a Storage with `Device = VDev, VDev2` (Media Type of the copy's volume)
    run(&rig, "restore of the copy", &format!(
        "restore jobid={copy_job} client=vfd storage=vboth where={} all done yes", rig.layout.restore.display()));
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the copy differs from the source:\n{diff}\n{}", rig.diagnostics());
}

/// A restore of a chain whose Incremental is on a target another job writes to waits for it (VT-1).
#[test]
#[ignore = "needs the osd binary and initdb"]
fn restore_waits_for_a_busy_target() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39881);
    options.sd = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    options.distributor = true;
    options.dir_client_lines = vec!["Maximum Concurrent Jobs = 10".into()];
    options.distributor_routes = vec![
        "Route { Job = BigBackup; Device = VDev2 }".into(),
        "Route { Level = Full; Device = VDev }".into(),
        "Route { Device = VDev2 }".into(),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let full = run(&rig, "full", "run job=VBackup level=Full yes");
    std::thread::sleep(Duration::from_secs(2));
    std::fs::write(rig.layout.data.join("added-for-incremental.txt"), "added\n").unwrap_or_else(|e| panic!("{e}"));
    let incremental = run(&rig, "incremental", "run job=VBackup level=Incremental yes");
    assert!(media_of(&rig, incremental).trim().ends_with("|VFile2"), "{}", media_of(&rig, incremental));

    let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
    rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    std::thread::sleep(Duration::from_secs(3));
    let chain = run(&rig, "restore of the chain", &format!("restore jobid={full},{incremental} client=vfd where={} all done yes", rig.layout.restore.display()));
    let big_row = rig.wait_job(big, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(big_row.status, 'T', "{}", rig.job_log(big));
    assert!(media_of(&rig, big).trim().ends_with("|VFile2"), "BigBackup went to VDev2: {}", media_of(&rig, big));
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore of the chain differs:\n{diff}\n{}", rig.job_log(chain));
    let end = |id: u32| rig.catalog().query(&format!("SELECT EndTime FROM Job WHERE JobId={id}")).unwrap_or_default();
    assert!(end(chain) >= end(big), "the restore waited for BigBackup: restore {} BigBackup {}", end(chain), end(big));
}
