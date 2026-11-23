//! The Director asks osd for the health of the disks under its devices
//! (`Health Check Interval` of the Storage resource, `.status health`),
//! writes the history to `onesimus.medium_health` and sends a Director
//! message about the verdict.
//!
//! The rig's devices lie in `/tmp` where an unprivileged test has no SMART
//! access, so the verdict is `unknown` (a warning); that exercises the whole
//! path: question, answer, row, message.
//!
//! Start: `cargo test -p onesimus-rig --test osd_health -- --ignored --nocapture`

#![deny(missing_docs)]

use std::time::{Duration, Instant};

use onesimus_rig::{Rig, RigOptions};

/// The osd of the workspace asked every minute; the Director looks every 30 s.
#[test]
#[ignore = "needs the osd binary and takes about a minute"]
fn the_director_records_and_reports_the_health_of_a_storage() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39701);
    options.sd = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    // the Onesimus directive goes into odir.d, bareos-dir.d stays standard
    options.extra_odir_d = vec![("storage/vsd.conf".into(), "Storage {\n  Name = vsd\n  Health Check Interval = 1 min\n}\n".into())];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let deadline = Instant::now() + Duration::from_secs(120);
    let rows = loop {
        let rows = rig
            .catalog()
            .query("SELECT storage, device, level, reasons FROM onesimus.medium_health ORDER BY checked_at")
            .unwrap_or_default();
        if !rows.trim().is_empty() {
            break rows;
        }
        assert!(Instant::now() < deadline, "no health row after two minutes\n{}", rig.diagnostics());
        std::thread::sleep(Duration::from_secs(3));
    };
    eprintln!("health history:\n{rows}");
    assert!(rows.starts_with("vsd|VDev|"), "{rows}");
    let level = rows.split('|').nth(2).unwrap_or_default();
    assert!(matches!(level, "unknown" | "ok" | "warning" | "critical"), "{rows}");

    // a message in the Daemon messages when the verdict is not ok
    if level != "ok" {
        let log = std::fs::read_to_string(rig.layout.dir_work.join("daemon.log")).unwrap_or_default();
        assert!(log.contains("Storage \"vsd\", device \"VDev\"") && log.contains(&format!("is {level}")), "no health message in the daemon log:\n{log}");
    }
}

/// The same through the console: `configure update storage=vsd … overlay` writes the
/// Onesimus directive to `odir.d/`, reloads, and the check starts.
#[test]
#[ignore = "needs the osd binary and takes about a minute"]
fn configure_update_with_overlay_switches_the_health_check_on() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39711);
    options.sd = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    // a Bareos directive with `overlay` is refused, an Onesimus one is written to odir.d
    let refused = rig.command("configure update storage=vsd password=x overlay").unwrap_or_else(|e| panic!("{e}"));
    assert!(refused.contains("is not an Onesimus directive"), "{refused}");
    let reply = rig.command("configure update storage=vsd healthcheckinterval=\"1 min\" overlay").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("configure: {reply}");
    assert!(reply.contains("overlay updated"), "{reply}");
    let file = rig.layout.root.join("onesimus/odir.d/storage/vsd.conf");
    let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert!(text.contains("Name = vsd") && text.contains("healthcheckinterval"), "{text}");
    // bareos-dir.d stays untouched
    let bareos = std::fs::read_to_string(rig.layout.dir_conf.join("storage/vsd.conf")).unwrap_or_default();
    assert!(!bareos.to_ascii_lowercase().contains("healthcheck"), "{bareos}");

    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let rows = rig.catalog().query("SELECT storage, device, level FROM onesimus.medium_health").unwrap_or_default();
        if rows.starts_with("vsd|VDev|") {
            break;
        }
        assert!(Instant::now() < deadline, "no health row after two minutes\n{}", rig.diagnostics());
        std::thread::sleep(Duration::from_secs(3));
    }
}
