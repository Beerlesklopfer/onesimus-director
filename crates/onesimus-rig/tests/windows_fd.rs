//! Backup and restore of a File Daemon on Windows (`ofd.exe`) from a rig
//! whose Director and Storage Daemon run on this host.
//!
//! The test needs a Windows host (no Wine) with `D:\odir-test\bin\ofd.exe`
//! built by `scripts/windows-vm/ship.sh`: `ONESIMUS_WINDOWS_FD=<address>`.
//! It copies the helper scripts of `scripts/windows-vm` there, lets
//! `prep.ps1` create the test tree and start ofd (port 19102, Director
//! `bareos-dir`, password `vfd-pw`), backs the tree up, restores it and lets
//! `cmp.ps1` compare every file by SHA-256 on the Windows host. The Storage
//! Daemon connects to the File Daemon (a passive client), so the Windows
//! host never needs to reach this host.
//!
//! Start: `ONESIMUS_WINDOWS_FD=192.168.24.95 cargo test -p onesimus-rig --test windows_fd -- --ignored --nocapture`

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::console::queued_job_id;
use onesimus_rig::{Rig, RigOptions};

/// Source tree on the Windows host.
const DATA: &str = "D:/odir-test/testdata";
/// Restore target on the Windows host.
const RESTORE: &str = "D:/odir-test/restore";

/// Runs `script` of `scripts/windows-vm` on the Windows host (copied there
/// first) and returns its output.
fn vm(host: &str, script: &str) -> String {
    let local = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/windows-vm").join(script);
    let copy = std::process::Command::new("scp")
        .args(["-o", "BatchMode=yes"])
        .arg(&local)
        .arg(format!("{host}:D:/odir-test/{script}"))
        .output()
        .expect("scp starts");
    assert!(copy.status.success(), "scp {script}: {}", String::from_utf8_lossy(&copy.stderr));
    let run = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(format!("D:\\odir-test\\{script}"))
        .output()
        .expect("ssh starts");
    format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr))
}

/// Backup of the Windows tree, then a restore to `RESTORE`.
#[test]
#[ignore = "needs a running ofd.exe on a Windows host (ONESIMUS_WINDOWS_FD)"]
fn backup_and_restore_through_the_windows_fd() {
    let Some(host) = std::env::var_os("ONESIMUS_WINDOWS_FD").and_then(|h| h.into_string().ok()) else {
        panic!("ONESIMUS_WINDOWS_FD is not set");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    eprintln!("deploy:\n{}", vm(&host, "deploy.ps1"));
    eprintln!("prep:\n{}", vm(&host, "prep.ps1"));
    let mut options = RigOptions::new(39691);
    options.start_fd = false;
    options.extra_director = vec![
        ("client/vfd.conf".into(), format!("Client {{\n  Name = vfd\n  Address = {host}\n  FD Port = 19102\n  Password = \"vfd-pw\"\n  Passive = yes\n}}\n")),
        ("fileset/VSet.conf".into(), format!("FileSet {{\n  Name = VSet\n  Include {{\n    Options {{\n      Signature = MD5\n      Compression = GZIP\n    }}\n    File = {DATA}\n  }}\n}}\n")),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let status = rig.command("status client=vfd").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("status client:\n{status}");

    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("backup: {row:?}\n{}", rig.job_log(backup));
    assert_eq!(row.status, 'T', "backup: {row:?}\n{}", rig.job_log(backup));

    let reply = rig
        .command(&format!("restore jobid={backup} client=vfd where={RESTORE} all done yes"))
        .unwrap_or_else(|e| panic!("{e}"));
    let restore = queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("restore: {row:?}\n{}", rig.job_log(restore));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}", rig.job_log(restore));

    let compare = vm(&host, "cmp.ps1");
    eprintln!("compare:\n{compare}");
    assert!(compare.contains("files: source 5, restored 5"), "{compare}");
    assert!(compare.contains("bad=0"), "{compare}");
    let _ = vm(&host, "stop.ps1");
}
