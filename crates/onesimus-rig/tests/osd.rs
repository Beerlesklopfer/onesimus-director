//! Ende-zu-Ende-Läufe mit osd als Storage Daemon (Plan
//! `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`, P1 Stufe 1 bis 3).
//!
//! Seit Stufe 3 laufen alle Fälle auf dem Hauptweg gRPC: ofd mit
//! gRPC-Port, Client- und Storage-Ressource mit `Grpc Enable = yes`, osd
//! mit gRPC-Port aus `osd.d`. Gegen bareos-sd (`ONESIMUS_RIG_SD`) fällt
//! odir für den SD auf BNET zurück. Die BNET-Fälle stehen in der
//! Interop-Matrix `tests/osd_grpc.rs`.
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
//! `/usr/sbin/bls` installiert ist. Danach stellt ein Restore den Baum in
//! einem anderen Verzeichnis wieder her; verglichen werden Inhalt (`diff
//! -r`) und Attribute (Rechte, Eigentümer, Zeiten, Symlink-Ziele,
//! Hardlinks, Löcher der Sparse-Datei).
//!
//! | Test | Ports | Inhalt |
//! |---|---|---|
//! | `osd_backup_restore_active_client` | ab 39471 | Backup und Restore, der FD ruft den SD an |
//! | `osd_backup_restore_passive_client` | ab 39481 | dasselbe mit `Passive = yes`, der SD ruft den FD an |
//! | `osd_backup_verify_restore_over_several_volumes` | ab 39491 | `Maximum Volume Bytes = 5M`: Backup über mehrere Volumes, Verify VolumeToCatalog, Restore über alle |
//! | `osd_and_bareos_sd_read_each_others_volumes` | ab 39501 | Backup mit bareos-sd → Restore mit osd; Backup mit osd auf dasselbe Volume → Restore mit bareos-sd; `bls` liest das gemischte Volume |

#![deny(missing_docs)]

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use onesimus_rig::{Grpc, Rig, RigOptions};

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
    options.fd_grpc = Grpc::On;
    options.sd_grpc = Grpc::On;
    Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"))
}

/// Panics with `what` and the rig diagnostics unless `ok`.
fn check(rig: &Rig, ok: bool, what: &str) {
    assert!(ok, "{what}\n{}", rig.diagnostics());
}

/// One full backup and every check of the module comment; the JobId.
fn backup_and_check(rig: &Rig) -> u32 {
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
    backup
}

/// Restores job `backup` into the (emptied) restore directory and
/// compares the restored tree with the source: content, attributes,
/// hardlinks and holes.
fn restore_and_compare(rig: &Rig, backup: u32) {
    let _ = std::fs::remove_dir_all(&rig.layout.restore);
    std::fs::create_dir_all(&rig.layout.restore).unwrap_or_else(|e| panic!("{e}"));
    let restore = rig.restore(backup).unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(restore);
    eprintln!("Job-Log {restore} (Restore von {backup}):\n{log}");
    check(rig, row.status == 'T' && row.files == ENTRIES, &format!("Restore: {row:?}\n{log}"));
    check(rig, log.contains("Restore OK"), &format!("kein \"Restore OK\" im Job-Log:\n{log}"));
    let (rc, diff) = rig.diff_restore();
    check(rig, rc == 0, &format!("diff -r Quelle/Restore rc={rc}:\n{diff}"));
    let restored = rig.layout.restored_data();
    let diffs = onesimus_rig::tree::compare_trees(&rig.layout.data, &restored);
    check(rig, diffs.is_empty(), &format!("Attribute nach dem Restore verschieden:\n{}", diffs.join("\n")));
    check(rig, onesimus_rig::tree::hardlinks_preserved(&restored).unwrap_or(false), "Hardlink hard1/sub/hard2 ist keine gemeinsame Inode");
    let allocated = onesimus_rig::tree::allocated_bytes(&restored.join("sparse.bin")).unwrap_or(u64::MAX);
    check(rig, allocated < onesimus_rig::tree::SPARSE_FILE_BYTES / 4, &format!("Sparse-Datei ohne Löcher: {allocated} Bytes belegt"));
    eprintln!("Restore {restore}: diff -r rc=0, Attribute gleich ({ENTRIES} Einträge), Hardlinks und Löcher erhalten");
}

/// Full backup and restore of the test tree with osd: the FD connects to
/// the SD (active client, the default).
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn osd_backup_restore_active_client() {
    let rig = start(39471, false);
    let backup = backup_and_check(&rig);
    restore_and_compare(&rig, backup);
}

/// Full backup and restore with `Passive = yes` on the Client: the Director
/// sends the SD `passive client address=… port=… ssl=…`, and the SD
/// connects to the FD, for the backup and for the restore.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn osd_backup_restore_passive_client() {
    let rig = start(39481, true);
    let backup = backup_and_check(&rig);
    restore_and_compare(&rig, backup);
}

/// Verify VolumeToCatalog with `Passive = yes` (S-59): the Director tells the SD
/// to connect to the FD (`passive client`), then starts the job in the SD;
/// before, the job waited for an FD that never came.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn osd_verify_passive_client() {
    let rig = start(39721, true);
    let backup = backup_and_check(&rig);
    let verify = rig.run_job("VVol").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(verify, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(verify);
    eprintln!("Job-Log {verify} (Verify, passive client):\n{log}");
    check(&rig, row.status == 'T' && row.files == ENTRIES, &format!("Verify: {row:?}\n{log}"));
    check(&rig, !log.contains("waiting 1800 sec"), &format!("the SD waited for the FD:\n{log}"));
    let _ = backup;
}

/// A pool with `Maximum Volume Bytes = 5M`: the 12 MiB file of the test
/// tree spreads the backup over three volumes. Verify VolumeToCatalog and
/// the restore read across them; the record cut at each volume end is
/// joined again.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn osd_backup_verify_restore_over_several_volumes() {
    let (sd, is_osd) = storage_daemon();
    let mut packages = vec!["odir", "ofd"];
    if is_osd {
        packages.push("osd-daemon");
    }
    onesimus_rig::build_workspace_packages(&packages).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39491);
    options.sd = Some(sd);
    options.fd_grpc = Grpc::On;
    options.sd_grpc = Grpc::On;
    options.extra_director = vec![(
        "pool/VPool.conf".into(),
        "Pool {\n  Name = VPool\n  Pool Type = Backup\n  Label Format = \"VVol-\"\n  Maximum Volume Bytes = 5M\n}\n".into(),
    )];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(backup);
    eprintln!("Job-Log {backup}:\n{log}");
    check(&rig, row.status == 'T' && row.files == ENTRIES, &format!("Backup: {row:?}\n{log}"));
    let media = rig
        .catalog()
        .query("SELECT MediaId, VolumeName, VolStatus, VolJobs, VolBlocks, VolBytes FROM Media ORDER BY MediaId")
        .unwrap_or_else(|e| panic!("{e}"));
    let jobmedia = rig
        .catalog()
        .query(&format!(
            "SELECT JobMediaId, MediaId, FirstIndex, LastIndex, StartFile, EndFile, StartBlock, EndBlock FROM JobMedia \
             WHERE JobId = {backup} ORDER BY JobMediaId"
        ))
        .unwrap_or_else(|e| panic!("{e}"));
    eprintln!("Media:\n{media}JobMedia:\n{jobmedia}");
    let volumes: Vec<String> = media.lines().filter_map(|l| l.split('|').nth(1).map(str::to_string)).collect();
    check(&rig, volumes.len() >= 2, &format!("Backup nicht über mehrere Volumes:\n{media}"));
    check(&rig, jobmedia.lines().count() >= volumes.len(), &format!("zu wenige JobMedia-Sätze:\n{jobmedia}"));
    check(&rig, media.lines().filter(|l| l.contains("|Full|")).count() == volumes.len() - 1, &format!("alle bis auf das letzte Volume Full:\n{media}"));
    for v in &volumes {
        let bytes = std::fs::read(rig.layout.storage.join(v)).unwrap_or_else(|e| panic!("{v}: {e}"));
        let contents = bareos_format::volume::read_volume(&bytes).unwrap_or_else(|e| panic!("Volume {v}: {e}"));
        eprintln!("Volume {v}: {} Bytes, {} Blöcke, {} Datensätze, Rest am Ende fehlt: {:?}", bytes.len(), contents.blocks.len(), contents.records.len(), contents.truncated);
    }

    let verify = rig.run_job("VVol").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(verify, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(verify);
    eprintln!("Job-Log {verify} (Verify):\n{log}");
    check(&rig, row.status == 'T' && row.files == ENTRIES, &format!("Verify: {row:?}\n{log}"));
    for v in &volumes {
        check(&rig, log.contains(&format!("Ready to read from volume \"{v}\"")), &format!("Verify liest {v} nicht:\n{log}"));
    }

    restore_and_compare(&rig, backup);
}

/// osd reads what bareos-sd wrote and the other way round, on one volume:
/// a backup with bareos-sd is restored with osd; then osd appends a second
/// backup to the same volume, and bareos-sd restores that one. `bls` lists
/// both sessions of the mixed volume. Needs `/usr/sbin/bareos-sd`.
#[test]
#[ignore = "needs bareos-sd, initdb and the Bareos catalog schema: run with --ignored"]
fn osd_and_bareos_sd_read_each_others_volumes() {
    let bareos_sd = PathBuf::from("/usr/sbin/bareos-sd");
    if !bareos_sd.is_file() {
        eprintln!("{} nicht installiert (bareos-storage): Gegenprobe übersprungen", bareos_sd.display());
        return;
    }
    let osd = onesimus_rig::prereq::target_profile_dir().join("osd");
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39501);
    options.sd = Some(bareos_sd.clone());
    options.fd_grpc = Grpc::On;
    options.sd_grpc = Grpc::On;
    let mut rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let first = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(first, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    check(&rig, row.status == 'T' && row.files == ENTRIES, &format!("Backup mit bareos-sd: {row:?}\n{}", rig.job_log(first)));
    eprintln!("Backup {first} mit bareos-sd: T, {} Dateien", row.files);

    rig.replace_sd(&osd).unwrap_or_else(|e| panic!("osd startet nicht: {e}"));
    eprintln!("SD gewechselt: osd liest das Volume von bareos-sd");
    restore_and_compare(&rig, first);

    let second = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(second, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    check(&rig, row.status == 'T' && row.files == ENTRIES, &format!("Backup mit osd: {row:?}\n{}", rig.job_log(second)));
    eprintln!("Backup {second} mit osd auf dasselbe Volume: T, {} Dateien", row.files);

    rig.replace_sd(&bareos_sd).unwrap_or_else(|e| panic!("bareos-sd startet nicht: {e}"));
    eprintln!("SD gewechselt: bareos-sd liest, was osd angehängt hat");
    restore_and_compare(&rig, second);

    let media = rig.catalog().query("SELECT VolumeName, VolJobs FROM Media ORDER BY MediaId").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("Media:\n{media}");
    check(&rig, media.lines().count() == 1 && media.trim().ends_with("|2"), &format!("ein Volume mit zwei Jobs erwartet:\n{media}"));
    let volume = media.split('|').next().unwrap_or_default().to_string();
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
        check(&rig, out.status.success(), &format!("bls endet mit {}:\n{text}", out.status));
        check(&rig, text.contains(&format!("{} files and directories found", 2 * ENTRIES)), &format!("bls: nicht {} Dateien:\n{text}", 2 * ENTRIES));
    }
}
