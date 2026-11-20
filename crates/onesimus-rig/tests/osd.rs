//! Ende-zu-Ende-Läufe mit osd als Storage Daemon (Plan
//! `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`, P1 Stufe 1).
//!
//! Start: `cargo test -p onesimus-rig --test osd -- --ignored`
//! (docs/wiki/testgeruest.md, docs/wiki/osd.md). Der Test baut `odir`, `ofd`
//! und `osd-daemon` und startet `target/debug/osd` als SD. Mit
//! `ONESIMUS_RIG_SD=/usr/sbin/bareos-sd` läuft derselbe Test gegen bareos-sd
//! (Gegenprobe, so entstand die Referenz).
//!
//! Geprüft wird je Lauf: Backup endet mit `T` und 13 Dateien, der Katalog hat
//! Job, Media und JobMedia, das Volume trägt das Bareos-Format (eigener
//! Leser aus `bareos-format`: Volume-Label, SOS- und EOS-Label, Attribute
//! jeder Datei, Prüfsumme jedes Blocks) und `bls` listet die Dateien, wenn
//! `/usr/sbin/bls` installiert ist.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Entries of the rig's test tree (see `tests/rig.rs`).
const ENTRIES: u64 = 13;

/// The Bareos volume lister (package `bareos-storage-tools`).
const BLS: &str = "/usr/sbin/bls";

/// The SD of this test: `ONESIMUS_RIG_SD` or the workspace's `osd`.
fn storage_daemon() -> (PathBuf, bool) {
    match std::env::var_os("ONESIMUS_RIG_SD") {
        Some(sd) => (PathBuf::from(sd), false),
        None => (onesimus_rig::prereq::target_profile_dir().join("osd"), true),
    }
}

/// Builds the binaries and starts a rig on `base_port`.
fn start(base_port: u16, passive_client: bool) -> Rig {
    let (sd, is_osd) = storage_daemon();
    let mut packages = vec!["odir", "ofd"];
    if is_osd {
        packages.push("osd-daemon");
    }
    onesimus_rig::build_workspace_packages(&packages).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(sd);
    options.passive_client = passive_client;
    Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"))
}

/// Panics with `what` and the rig diagnostics unless `ok`.
fn check(rig: &Rig, ok: bool, what: &str) {
    assert!(ok, "{what}\n{}", rig.diagnostics());
}

/// One full backup and every check of the module comment.
fn backup_and_check(rig: &Rig) {
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(backup);
    check(rig, row.status == 'T' && row.files == ENTRIES, &format!("Backup: {row:?}\n{log}"));
    check(rig, log.contains("Backup OK"), &format!("kein \"Backup OK\" im Job-Log:\n{log}"));
    eprintln!("Job-Log {backup}:\n{log}");

    let media = rig
        .catalog()
        .query("SELECT MediaId, VolumeName, MediaType, VolStatus, VolJobs, VolFiles, VolBlocks, VolBytes FROM Media ORDER BY MediaId")
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("Media:\n{media}");
    let jobmedia = rig
        .catalog()
        .query(&format!(
            "SELECT JobMediaId, JobId, MediaId, FirstIndex, LastIndex, StartFile, EndFile, StartBlock, EndBlock, \
             VolIndex FROM JobMedia WHERE JobId = {backup} ORDER BY JobMediaId"
        ))
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("JobMedia:\n{jobmedia}");
    let files = rig
        .catalog()
        .query(&format!("SELECT count(*) FROM File WHERE JobId = {backup}"))
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("File-Sätze: {}", files.trim());
    check(rig, media.lines().count() == 1, &format!("ein Media-Satz erwartet:\n{media}"));
    let volume = media.split('|').nth(1).unwrap_or_default().to_string();
    check(rig, media.contains("|VFile|Append|1|"), &format!("Media nicht Append mit einem Job:\n{media}"));
    check(rig, jobmedia.lines().any(|l| l.split('|').nth(3) == Some("1") && l.split('|').nth(4) == Some("13")),
        &format!("JobMedia ohne FirstIndex=1 LastIndex=13:\n{jobmedia}"));
    check(rig, files.trim() == ENTRIES.to_string(), &format!("File-Sätze: {files}"));

    // The volume read back with the own reader.
    let path = rig.layout.storage.join(&volume);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let catalog_bytes: u64 = media.trim().split('|').nth(7).and_then(|v| v.parse().ok()).unwrap_or(0);
    check(rig, catalog_bytes == bytes.len() as u64, &format!("VolBytes {catalog_bytes} != Dateigröße {}", bytes.len()));
    let summary = bareos_format::volume::summarize(&bytes).unwrap_or_else(|e| panic!("Volume {}: {e}", path.display()));
    eprintln!("Volume {volume} (eigener Leser):\n{summary}");
    check(rig, summary.label.volume_name == volume, &format!("Label nennt {:?}", summary.label.volume_name));
    check(rig, summary.label.media_type == "VFile" && summary.label.pool_name == "VPool", &format!("{:?}", summary.label));
    let session = summary.sessions.iter().find(|s| s.start.job_id == backup);
    check(rig, session.is_some_and(|s| s.end.as_ref().is_some_and(|e| e.job_files == Some(ENTRIES as u32) && e.job_status == Some('T' as u32))),
        &format!("SOS/EOS von Job {backup} fehlen oder falsch: {:?}", summary.sessions));
    check(rig, summary.attribute_file_indexes == (1..=ENTRIES as i32).collect::<Vec<_>>(),
        &format!("Attribute je Datei: {:?}", summary.attribute_file_indexes));

    // bls of the Bareos storage tools, if installed.
    if std::path::Path::new(BLS).is_file() {
        let out = Command::new(BLS)
            .arg("-c")
            .arg(&rig.layout.sd_conf)
            .arg("-V")
            .arg(&volume)
            .arg("VDev")
            .env("LC_ALL", "C")
            .output()
            .unwrap_or_else(|e| panic!("{BLS}: {e}"));
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        eprintln!("bls -V {volume}:\n{text}");
        check(rig, out.status.success(), &format!("bls endet mit {}:\n{text}", out.status));
        for name in ["file0.txt", "sparse.bin", "hard1", "link0"] {
            check(rig, text.contains(name), &format!("bls listet {name} nicht:\n{text}"));
        }
        check(rig, text.contains(&format!("{ENTRIES} files and directories found")), &format!("bls: nicht {ENTRIES} Dateien:\n{text}"));
    } else {
        eprintln!("{BLS} nicht installiert (bareos-storage-tools): nur der eigene Leser hat das Volume geprüft");
    }
}

/// Full backup of the test tree with osd: the FD connects to the SD
/// (active client, the default).
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn osd_full_backup_active_client() {
    let rig = start(39471, false);
    backup_and_check(&rig);
}

/// Full backup with `Passive = yes` on the Client: the Director sends the SD
/// `passive client address=… port=… ssl=…`, and the SD connects to the FD.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn osd_full_backup_passive_client() {
    let rig = start(39481, true);
    backup_and_check(&rig);
}
