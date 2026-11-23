// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `oscan` und `oextract` gegen ein Volume, das osd im Gerüst geschrieben
//! hat: Der Katalogeintrag des Jobs wird gelöscht, `oscan -s` baut ihn aus
//! dem Volume neu auf, danach läuft die Rücksicherung durch den Director und
//! `oextract` stellt dieselbe Struktur ohne Director her.
//!
//! Start: `cargo test -p onesimus-rig --test outils -- --ignored --nocapture`
//! (braucht `initdb`; Ports ab 39751).

#![deny(missing_docs)]

use std::process::Command;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Längste Zeit eines Jobs im Gerüst.
const JOB_TIMEOUT: Duration = Duration::from_secs(240);

/// Das Programm `name` aus dem Build-Verzeichnis.
fn tool(name: &str) -> std::path::PathBuf {
    onesimus_rig::prereq::target_profile_dir().join(name)
}

#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn oscan_rebuilds_the_catalog_and_oextract_restores_without_the_director() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon", "outils"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39751);
    options.sd = Some(tool("osd"));
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{}", rig.job_log(backup));
    let volume = rig.root().join("storage/VVol-0001");
    assert!(volume.is_file(), "{}", volume.display());

    let q = |sql: &str| rig.catalog().query(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    let facts = |id: u32| q(&format!("SELECT Job, Name, Type, Level, JobStatus, JobFiles, JobErrors FROM Job WHERE JobId = {id}"));
    let (before, files_before) = (facts(backup), q(&format!("SELECT count(*) FROM File WHERE JobId = {backup}")));
    assert!(files_before.trim().parse::<u32>().unwrap() > 10, "{files_before}");

    let latin1 = |id: u32| q(&format!("SELECT encode(convert_to(Name, 'SQL_ASCII'), 'hex') FROM File WHERE JobId = {id} AND Name LIKE 'caf%'"));
    let name_before = latin1(backup);
    assert!(name_before.starts_with("636166"), "the Latin-1 test file is in the catalog: {name_before}");
    let jobmedia = q(&format!("SELECT FirstIndex, LastIndex, StartFile, EndFile, StartBlock, EndBlock, VolIndex FROM JobMedia WHERE JobId = {backup}"));
    eprintln!("original JobMedia: {jobmedia}");
    // the catalog loses the job, its files and the volume
    q(&format!("DELETE FROM File WHERE JobId = {backup}; DELETE FROM JobMedia WHERE JobId = {backup}; DELETE FROM Job WHERE JobId = {backup}; DELETE FROM Media WHERE VolumeName = 'VVol-0001'"));
    assert_eq!(q(&format!("SELECT count(*) FROM Job WHERE JobId = {backup}")).trim(), "0");

    // oscan prints the script; it changes nothing by itself
    let scan = |args: &[&str]| {
        let out = Command::new(tool("oscan")).args(args).arg(&volume).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let script = scan(&["-s"]);
    assert_eq!(q(&format!("SELECT count(*) FROM Job WHERE JobId = {backup}")).trim(), "0");
    q(&script);
    assert_eq!(facts(backup), before, "the job record differs from the original");
    // JobBytes comes from the end label (what the SD wrote), the FD counted a little less
    assert!(q(&format!("SELECT JobBytes FROM Job WHERE JobId = {backup}")).trim().parse::<u64>().unwrap() >= 12_588_910);
    assert_eq!(q(&format!("SELECT count(*) FROM File WHERE JobId = {backup}")), files_before);
    assert_eq!(q("SELECT VolStatus FROM Media WHERE VolumeName = 'VVol-0001'").trim(), "Full");
    assert_eq!(q(&format!("SELECT count(*) FROM JobMedia WHERE JobId = {backup}")).trim(), "1");
    assert_eq!(latin1(backup), name_before, "the file name that is not UTF-8 changed");
    eprintln!("scanned JobMedia: {}", q(&format!("SELECT FirstIndex, LastIndex, StartFile, EndFile, StartBlock, EndBlock, VolIndex FROM JobMedia WHERE JobId = {backup}")));
    // a second run changes nothing
    q(&script);
    assert_eq!(q(&format!("SELECT count(*) FROM File WHERE JobId = {backup}")), files_before);
    assert_eq!(q(&format!("SELECT count(*) FROM JobMedia WHERE JobId = {backup}")).trim(), "1");

    // the Director restores from the rebuilt catalog
    let restore = rig.restore(backup).unwrap_or_else(|e| panic!("{e}"));
    let restored = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(restored.status, 'T', "{}", rig.job_log(restore));
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "restore from the scanned catalog differs:\n{diff}");

    // oextract without the Director
    let out = rig.root().join("extracted");
    let run = Command::new(tool("oextract")).arg(&out).arg(&volume).output().unwrap();
    assert!(run.status.success(), "{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    let source = rig.root().join("data");
    let diff = Command::new("diff").args(["-r", "--no-dereference"]).arg(&source).arg(out.join(source.strip_prefix("/").unwrap())).output().unwrap();
    assert!(diff.status.success(), "oextract differs from the source:\n{}", String::from_utf8_lossy(&diff.stdout));
}
