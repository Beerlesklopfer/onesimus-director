// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Ende-zu-Ende-Läufe odir + ofd + bareos-sd (später osd).
//!
//! Start: `cargo test -p onesimus-rig -- --ignored`
//! (docs/wiki/testgeruest.md). Jeder Test startet ein eigenes Gerüst auf
//! eigenen Ports und räumt es auch bei einem Fehlschlag ab (`Drop` von
//! [`Rig`]).
//!
//! Gemessene Defekte aus dem Gutachten `docs/audits/ofd-struktur-2026-11-19.md`
//! stehen als eigene `#[should_panic]`-Tests: Sie bestehen, solange der
//! Defekt besteht, und schlagen fehl, sobald er behoben ist. Dann wird aus
//! dem `should_panic`-Test eine gewöhnliche Prüfung.

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Builds the workspace binaries and starts a rig on `base_port`.
fn start(base_port: u16) -> Rig {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    Rig::start(RigOptions::new(base_port)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"))
}

/// Panics with `what` and the rig diagnostics unless `ok`.
fn check(rig: &Rig, ok: bool, what: &str) {
    assert!(ok, "{what}\n{}", rig.diagnostics());
}

/// Starts `BigBackup`, waits until it runs and has sent data for a while,
/// cancels it and returns its JobId once it has ended.
fn cancel_big_backup(rig: &Rig) -> u32 {
    let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    check(rig, row.status == 'R', &format!("BigBackup läuft nicht: {row:?}"));
    std::thread::sleep(Duration::from_secs(3));
    let reply = rig.command(&format!("cancel jobid={big} yes")).unwrap_or_else(|e| panic!("{e}"));
    check(rig, reply.contains("marked to be canceled"), &format!("cancel: {reply}"));
    rig.wait_job(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    big
}

/// Backup of the special-case tree, Verify VolumeToCatalog, restore with
/// bit-exact comparison, Estimate, and the cancel of a running backup.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn backup_verify_restore_estimate_cancel() {
    let rig = start(39391);
    // directories data, sub; files file0..2, inner, empty, large, link0,
    // hard1, hard2, café, sparse
    let entries = 13;

    // Backup
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    check(&rig, row.status == 'T' && row.files == entries, &format!("Backup: {row:?}\n{}", rig.job_log(backup)));

    // Estimate (listing) after the first backup: the FD walks the FileSet
    // without saving (before it, see expected_defect_d15_…).
    let estimate = rig.command("estimate job=VBackup listing").unwrap_or_else(|e| panic!("{e}"));
    check(
        &rig,
        estimate.contains("file0.txt") && estimate.contains("sparse.bin"),
        &format!("estimate listet den Testbaum nicht:\n{estimate}"),
    );

    // Verify VolumeToCatalog
    let verify = rig.run_job("VVol").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(verify, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    check(&rig, row.status == 'T' && row.files == entries, &format!("Verify: {row:?}\n{}", rig.job_log(verify)));

    // Restore and bit-exact comparison
    let restore = rig.restore(backup).unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    check(&rig, row.status == 'T' && row.files == entries, &format!("Restore: {row:?}\n{}", rig.job_log(restore)));
    let (rc, diff) = rig.diff_restore();
    check(&rig, rc == 0, &format!("diff -r Quelle/Restore rc={rc}:\n{diff}"));
    let restored = rig.layout.restored_data();
    check(
        &rig,
        onesimus_rig::tree::hardlinks_preserved(&restored).unwrap_or(false),
        "Hardlink hard1/sub/hard2 ist nach dem Restore keine gemeinsame Inode",
    );
    let allocated = onesimus_rig::tree::allocated_bytes(&restored.join("sparse.bin")).unwrap_or(u64::MAX);
    check(
        &rig,
        allocated < onesimus_rig::tree::SPARSE_FILE_BYTES / 4,
        &format!("Sparse-Datei ohne Löcher wiederhergestellt: {allocated} Bytes belegt"),
    );

    // Cancel: the job ends early and not as a success. Which letter the
    // catalog shows is expected_defect_d16_…; what the FD reports is
    // expected_defect_d1_….
    let big = cancel_big_backup(&rig);
    let row = rig.job_row(big).ok().flatten();
    check(
        &rig,
        row.as_ref().is_some_and(|r| {
            r.status != 'T' && r.bytes < onesimus_rig::tree::BIG_FILE_BYTES as u64
        }),
        &format!("Abbruch wirkt nicht: {row:?}\n{}", rig.job_log(big)),
    );
}

/// Erwarteter Defekt D15 (neu, gemessen 19.11.2026 im Gerüst): `estimate`
/// vor dem ersten Backup eines FileSets antwortet nur mit
/// `Using Catalog "MyCatalog"` und spricht den FD nicht an. odir sucht den
/// FileSet-Satz im Katalog nur (`crates/odir/src/commands/estimate.rs`
/// `estimate_job`, „GetOrCreateFilesetRecord (lookup only)“) und gibt still
/// auf, wenn er fehlt; Bareos legt ihn an. Das ist die Ursache des Befunds
/// „Estimate-Pfad nicht erreicht“ im Gutachten §3.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
#[should_panic(expected = "D15:")]
fn expected_defect_d15_estimate_before_first_backup_is_silent() {
    let rig = start(39421);
    let estimate = rig.command("estimate job=VBackup listing").unwrap_or_else(|e| panic!("{e}"));
    check(&rig, estimate.contains("file0.txt"), &format!("D15: estimate vor dem ersten Backup listet nichts:\n{estimate}"));
}

/// Erwarteter Defekt D16 (neu, gemessen 19.11.2026 im Gerüst): Ein
/// abgebrochenes Backup steht im Katalog mit `f` (fatal) statt `A`. odirs
/// Job-Worker endet nach dem `cancel` mit Code 1, und der JobManager trägt
/// `f` ein („worker exited (code=1), catalog status=f“). Der Fachmann hatte
/// im Gutachten §3 bei anderem Zeitverlauf (200 kB/s, Abbruch nach 4 s) `A`
/// gemessen; der Endstatus hängt also vom Zeitpunkt des Abbruchs ab.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
#[should_panic(expected = "D16:")]
fn expected_defect_d16_canceled_backup_is_fatal_in_catalog() {
    let rig = start(39431);
    let big = cancel_big_backup(&rig);
    let row = rig.job_row(big).ok().flatten();
    check(&rig, row.as_ref().is_some_and(|r| r.status == 'A'), &format!("D16: Katalog nach cancel: {row:?}"));
}

/// Erwarteter Defekt D1 (Gutachten §7): Nach `cancel` meldet ofd dem
/// Director `2800 End Job TermCode=69` (E) statt 65 (A). Der Katalog zeigt
/// trotzdem A, weil odir den Status selbst setzt; das prüft
/// [`backup_verify_restore_estimate_cancel`]. Ist D1 behoben, schlägt dieser
/// Test fehl und wird zur gewöhnlichen Prüfung.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
#[should_panic(expected = "D1:")]
fn expected_defect_d1_fd_reports_cancel_as_error() {
    let rig = start(39401);
    let big = cancel_big_backup(&rig);
    let end = rig.fd_end_job(big);
    check(&rig, end.is_some(), "kein 2800 End Job im ofd-Log");
    let end = end.unwrap_or_else(|| unreachable!());
    check(&rig, end.term_code == 'A', &format!("D1: ofd meldet TermCode {} statt A: {end:?}", end.term_code));
}

/// Erwarteter Defekt D14 (neu, gemessen 19.11.2026 im Gerüst): Die
/// Job-Direktive `Maximum Bandwidth` kommt als `setbandwidth=200000 Job=…`
/// beim FD an, wirkt aber nicht. `SetbandwidthCmd` setzt in ofd nur das
/// Limit der Verbindung, `storage_cmd` überschreibt es danach mit dem
/// Client-Wert (`crates/ofd/src/job/backup_cmds.rs` `setbandwidth_cmd`,
/// `crates/ofd/src/job.rs` `storage_cmd`). Bareos setzt
/// `jcr->max_bandwidth` (`filed/dir_cmd.cc` Zeile 751, 1412–1420). Im
/// Gerüst gilt deshalb 4 MB/s (Client) statt 200 kB/s (Job).
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
#[should_panic(expected = "D14:")]
fn expected_defect_d14_job_bandwidth_is_overwritten() {
    let rig = start(39411);
    let big = cancel_big_backup(&rig);
    let end = rig.fd_end_job(big);
    check(&rig, end.is_some(), "kein 2800 End Job im ofd-Log");
    let end = end.unwrap_or_else(|| unreachable!());
    // About 5 s at 200 kB/s are 1 MB; allow twice that for start and burst.
    check(
        &rig,
        end.job_bytes < 2_000_000,
        &format!("D14: {} Bytes in rund 5 s gesendet, Limit 200 kB/s greift nicht: {end:?}", end.job_bytes),
    );
}
