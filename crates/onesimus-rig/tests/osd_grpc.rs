//! Interop-Matrix gRPC/BNET zwischen odir, ofd und osd (Plan
//! `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`, P1 Stufe 3).
//!
//! Start: `cargo test -p onesimus-rig --test osd_grpc -- --ignored --test-threads=1`
//! (docs/wiki/testgeruest.md, docs/wiki/osd.md). Jeder Fall macht Backup,
//! Verify (VolumeToCatalog) und Restore mit Vergleich und belegt aus den
//! Logs, welcher Transport tatsächlich lief:
//!
//! - Director ↔ SD: odir-Log `SD …: connected over gRPC TLS` bzw.
//!   `… falling back to BNET`, osd-Log `Director connects over …`, Job-Log
//!   `Connected Storage daemon at …, encryption: gRPC TLS`.
//! - Director ↔ FD: odir-Log `FD …: connected over gRPC TLS`.
//! - FD ↔ SD aktiv: ofd-Log `Connected Storage daemon at … over gRPC TLS`
//!   bzw. `over BNET`, osd-Log `File daemon of job … connects over …`.
//! - FD ↔ SD passiv: osd-Log `connected to File daemon … over gRPC TLS
//!   (passive client)` bzw. `over BNET`.
//!
//! | Test | Ports | odir | ofd | osd |
//! |---|---|---|---|---|
//! | `a_all_grpc_active` | ab 39511 | gRPC | gRPC | gRPC |
//! | `a_all_grpc_passive` | ab 39521 | gRPC | gRPC | gRPC, `Passive = yes` |
//! | `b_osd_disable_grpc_active` | ab 39531 | gRPC konfiguriert | gRPC | `--disable-grpc` |
//! | `b_osd_disable_grpc_passive` | ab 39541 | gRPC konfiguriert | gRPC | `--disable-grpc`, `Passive = yes` |
//! | `c_ofd_disable_grpc_active` | ab 39551 | gRPC konfiguriert | `--disable-grpc` | gRPC |
//! | `c_ofd_disable_grpc_passive` | ab 39561 | gRPC konfiguriert | `--disable-grpc` | gRPC, `Passive = yes` |
//!
//! Mit `ONESIMUS_RIG_SD=/usr/sbin/bareos-sd` laufen dieselben Fälle gegen
//! bareos-sd (Fall d): Die Storage-Ressource bleibt gRPC-konfiguriert, odir
//! fällt für den SD auf BNET zurück, ofd bekommt keine gRPC-Angaben.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Grpc, Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Entries of the rig's test tree (see `tests/rig.rs`).
const ENTRIES: u64 = 13;

/// The SD of this test: `ONESIMUS_RIG_SD` or the workspace's `osd`.
fn storage_daemon() -> PathBuf {
    std::env::var_os("ONESIMUS_RIG_SD")
        .map(PathBuf::from)
        .unwrap_or_else(|| onesimus_rig::prereq::target_profile_dir().join("osd"))
}

/// Panics with `what` and the rig diagnostics unless `ok`.
fn check(rig: &Rig, ok: bool, what: &str) {
    assert!(ok, "{what}\n{}", rig.diagnostics());
}

/// Lines of `log` that contain `needle`, for the evidence printed by the
/// test.
fn lines_with(log: &str, needle: &str) -> Vec<String> {
    log.lines().filter(|l| l.contains(needle)).map(str::to_string).collect()
}

/// Asserts that some line of daemon log `daemon` contains `needle`, and
/// prints the first such line as evidence.
fn evidence(rig: &Rig, daemon: &str, needle: &str) {
    let log = rig.daemon_log(daemon);
    let found = lines_with(&log, needle);
    check(rig, !found.is_empty(), &format!("{daemon}-Log ohne „{needle}“"));
    eprintln!("Beleg {daemon}: {}", found[0].trim());
}

/// Asserts that no line of daemon log `daemon` contains `needle`.
fn absent(rig: &Rig, daemon: &str, needle: &str) {
    let found = lines_with(&rig.daemon_log(daemon), needle);
    check(rig, found.is_empty(), &format!("{daemon}-Log enthält unerwartet „{needle}“: {found:?}"));
}

/// Runs job `name` and checks it ends with `T` and the 13 entries; the
/// JobId.
fn run_ok(rig: &Rig, name: &str) -> u32 {
    let id = rig.run_job(name).unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(id);
    eprintln!("Job-Log {id} ({name}):\n{log}");
    check(rig, row.status == 'T' && row.files == ENTRIES, &format!("{name}: {row:?}\n{log}"));
    id
}

/// Backup, Verify VolumeToCatalog and a restore compared with `diff -r`;
/// the job logs together.
fn backup_verify_restore(rig: &Rig) -> String {
    let backup = run_ok(rig, "VBackup");
    let mut logs = rig.job_log(backup);
    check(rig, logs.contains("Backup OK"), "kein „Backup OK“");
    let verify = run_ok(rig, "VVol");
    let verify_log = rig.job_log(verify);
    check(rig, verify_log.contains("Verify OK"), &format!("kein „Verify OK“:\n{verify_log}"));
    logs += &verify_log;
    let _ = std::fs::remove_dir_all(&rig.layout.restore);
    std::fs::create_dir_all(&rig.layout.restore).unwrap_or_else(|e| panic!("{e}"));
    let restore = rig.restore(backup).unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let restore_log = rig.job_log(restore);
    eprintln!("Job-Log {restore} (Restore):\n{restore_log}");
    check(rig, row.status == 'T' && row.files == ENTRIES, &format!("Restore: {row:?}\n{restore_log}"));
    check(rig, restore_log.contains("Restore OK"), "kein „Restore OK“");
    let (rc, diff) = rig.diff_restore();
    check(rig, rc == 0, &format!("diff -r rc={rc}:\n{diff}"));
    logs + &restore_log
}

/// One case of the matrix: starts the rig with `fd`/`sd` gRPC modes, runs
/// [`backup_verify_restore`] and checks from the logs which transport each
/// connection took.
fn case(base_port: u16, fd: Grpc, sd: Grpc, passive: bool) {
    let sd_program = storage_daemon();
    let is_osd = onesimus_rig::is_osd(&sd_program);
    let mut packages = vec!["odir", "ofd"];
    if is_osd {
        packages.push("osd-daemon");
    }
    onesimus_rig::build_workspace_packages(&packages).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base_port);
    options.sd = Some(sd_program);
    options.passive_client = passive;
    options.fd_grpc = fd;
    options.sd_grpc = sd;
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let jobs = backup_verify_restore(&rig);
    let ports = rig.ports;

    // What should have happened: gRPC wherever both ends offer it.
    let sd_grpc = is_osd && sd == Grpc::On;
    let fd_grpc = fd == Grpc::On;
    let data_grpc = sd_grpc && fd_grpc;
    eprintln!("Erwartet: Director↔SD {}, Director↔FD {}, FD↔SD {} ({})",
        if sd_grpc { "gRPC" } else { "BNET" }, if fd_grpc { "gRPC" } else { "BNET" },
        if data_grpc { "gRPC" } else { "BNET" }, if passive { "passiv" } else { "aktiv" });

    // Every fallback comes from a real attempt on the gRPC port (the rig
    // writes a stand-in certificate where no daemon creates one).
    absent(&rig, "odir", "gRPC CA");
    // Director ↔ SD
    if sd_grpc {
        evidence(&rig, "odir", &format!("SD 127.0.0.1:{}: connected over gRPC TLS", ports.sd_grpc));
        evidence(&rig, "sd", "Director connects over gRPC TLS");
        check(&rig, jobs.contains(&format!("Connected Storage daemon at 127.0.0.1:{}, encryption: gRPC TLS", ports.sd)),
            "Job-Log ohne gRPC-Verbindung zum SD");
        absent(&rig, "odir", &format!("falling back to BNET on port {}", ports.sd));
    } else {
        if sd.configured() {
            evidence(&rig, "odir", &format!("SD 127.0.0.1:{}: ", ports.sd_grpc));
            evidence(&rig, "odir", &format!("falling back to BNET on port {}", ports.sd));
        }
        if is_osd {
            evidence(&rig, "sd", "Director connects over BNET");
            absent(&rig, "sd", "over gRPC TLS");
        }
        absent(&rig, "odir", "SD 127.0.0.1: connected over gRPC");
    }
    // Director ↔ FD
    if fd_grpc {
        evidence(&rig, "odir", &format!("FD 127.0.0.1:{}: connected over gRPC TLS", ports.fd_grpc));
    } else if fd.configured() {
        evidence(&rig, "odir", &format!("FD 127.0.0.1:{}: ", ports.fd_grpc));
        evidence(&rig, "odir", &format!("falling back to BNET on port {}", ports.fd));
    }
    // FD ↔ SD
    if passive {
        if is_osd {
            let needle = if data_grpc {
                format!("connected to File daemon 127.0.0.1:{} over gRPC TLS (passive client)", ports.fd_grpc)
            } else {
                format!("connected to File daemon 127.0.0.1:{} over BNET (passive client)", ports.fd)
            };
            evidence(&rig, "sd", &needle);
        }
        if data_grpc {
            evidence(&rig, "ofd", "Storage daemon connects over gRPC (passive client)");
        }
    } else {
        let needle = if data_grpc {
            format!("Connected Storage daemon at 127.0.0.1:{} over gRPC TLS", ports.sd_grpc)
        } else {
            format!("Connected Storage daemon at 127.0.0.1:{} over BNET", ports.sd)
        };
        evidence(&rig, "ofd", &needle);
        if is_osd {
            let transport = if data_grpc { "connects over gRPC TLS" } else { "connects over BNET" };
            let log = rig.daemon_log("sd");
            let lines = lines_with(&log, "File daemon of job");
            check(&rig, !lines.is_empty() && lines.iter().all(|l| l.contains(transport)),
                &format!("osd: FD-Verbindungen nicht alle „{transport}“: {lines:?}"));
            eprintln!("Beleg sd: {}", lines[0].trim());
        }
        if !data_grpc {
            absent(&rig, "ofd", "over gRPC TLS");
        }
    }
}

/// (a) odir, ofd and osd all on gRPC, active client.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn a_all_grpc_active() {
    case(39511, Grpc::On, Grpc::On, false);
}

/// (a) odir, ofd and osd all on gRPC, passive client: osd calls ofd over
/// `FileDaemon/Data`.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn a_all_grpc_passive() {
    case(39521, Grpc::On, Grpc::On, true);
}

/// (b) osd with `--disable-grpc`: odir falls back to BNET for the SD, ofd
/// gets no gRPC target and connects over BNET; same results.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn b_osd_disable_grpc_active() {
    case(39531, Grpc::On, Grpc::Disabled, false);
}

/// (b) as above with a passive client: osd calls ofd over BNET.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn b_osd_disable_grpc_passive() {
    case(39541, Grpc::On, Grpc::Disabled, true);
}

/// (c) ofd with `--disable-grpc`: the FD runs on BNET (control and data),
/// Director ↔ SD stays on gRPC.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn c_ofd_disable_grpc_active() {
    case(39551, Grpc::Disabled, Grpc::On, false);
}

/// (c) as above with a passive client.
#[test]
#[ignore = "needs initdb and the Bareos catalog schema: run with --ignored"]
fn c_ofd_disable_grpc_passive() {
    case(39561, Grpc::Disabled, Grpc::On, true);
}
