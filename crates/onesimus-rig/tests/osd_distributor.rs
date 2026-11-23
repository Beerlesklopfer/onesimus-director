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
