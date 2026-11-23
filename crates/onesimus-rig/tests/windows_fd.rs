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
    put(host, script);
    let run = std::process::Command::new("ssh")
        .args(["-o", "BatchMode=yes", host, "powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(format!("D:\\odir-test\\{script}"))
        .output()
        .expect("ssh starts");
    format!("{}{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr))
}

/// Copies `script` of `scripts/windows-vm` to the Windows host.
fn put(host: &str, script: &str) {
    let local = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/windows-vm").join(script);
    let copy = std::process::Command::new("scp")
        .args(["-o", "BatchMode=yes"])
        .arg(&local)
        .arg(format!("{host}:D:/odir-test/{script}"))
        .output()
        .expect("scp starts");
    assert!(copy.status.success(), "scp {script}: {}", String::from_utf8_lossy(&copy.stderr));
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
        ("fileset/VSet.conf".into(), format!("FileSet {{\n  Name = VSet\n  EnableVSS = no\n  Include {{\n    Options {{\n      Signature = MD5\n      Compression = GZIP\n      HardLinks = yes\n    }}\n    File = {DATA}\n  }}\n}}\n")),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let status = rig.command("status client=vfd").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("status client:\n{status}");

    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("backup: {row:?}\n{}", rig.job_log(backup));
    assert_eq!(row.status, 'T', "backup: {row:?}\n{}", rig.job_log(backup));

    let rows = rig
        .catalog()
        .query(&format!("SELECT p.path || f.name, f.lstat FROM file f JOIN path p ON p.pathid = f.pathid WHERE f.jobid = {backup} AND (f.name = 'a.txt' OR f.name = 'hard1.txt') ORDER BY 1"))
        .unwrap_or_default();
    eprintln!("catalog rows of the hard link pair:\n{rows}");
    // the data of a Windows file is the BackupRead stream: STREAM_WIN32_COMPRESSED_DATA (31, "f")
    assert!(rows.lines().all(|l| l.trim_end().ends_with(" f")), "the data stream is not the Win32 one:\n{rows}");
    let reply = rig
        .command(&format!("restore jobid={backup} client=vfd where={RESTORE} all done yes"))
        .unwrap_or_else(|e| panic!("{e}"));
    let restore = queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("restore: {row:?}\n{}", rig.job_log(restore));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}", rig.job_log(restore));

    let compare = vm(&host, "cmp.ps1");
    eprintln!("compare:\n{compare}");
    let entries = compare.lines().find(|l| l.starts_with("entries: source ")).unwrap_or_default();
    let numbers: Vec<&str> = entries.split(|c: char| !c.is_ascii_digit()).filter(|n| !n.is_empty()).collect();
    assert!(numbers.len() == 2 && numbers[0] == numbers[1] && numbers[0] != "0", "{compare}");
    assert!(compare.contains("bad=0"), "{compare}");

    // A file another program holds open without sharing cannot be read (EnableVSS = no): the job reports it and saves the rest.
    put(&host, "hold.ps1");
    eprintln!("lock: {}", vm(&host, "lock.ps1"));
    let again = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(again, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(again);
    eprintln!("backup with a locked file: {row:?}\n{log}");
    let _ = vm(&host, "unlock.ps1");
    assert!(log.contains("locked.bin"), "the locked file is not named in the job log:\n{log}");
    assert!(row.files >= 20, "the other files were not saved: {row:?}");
    let _ = vm(&host, "stop.ps1");
}

/// Full backup, a change of the tree, an incremental backup that holds only the
/// change, an estimate of the File Daemon on Windows, then a
/// restore of both jobs that matches the changed tree.
#[test]
#[ignore = "needs a running ofd.exe on a Windows host (ONESIMUS_WINDOWS_FD)"]
fn incremental_and_estimate_through_the_windows_fd() {
    let Some(host) = std::env::var_os("ONESIMUS_WINDOWS_FD").and_then(|h| h.into_string().ok()) else {
        panic!("ONESIMUS_WINDOWS_FD is not set");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    eprintln!("deploy:\n{}", vm(&host, "deploy.ps1"));
    eprintln!("prep:\n{}", vm(&host, "prep.ps1"));
    let mut options = RigOptions::new(39791);
    options.start_fd = false;
    options.extra_director = vec![
        ("client/vfd.conf".into(), format!("Client {{\n  Name = vfd\n  Address = {host}\n  FD Port = 19102\n  Password = \"vfd-pw\"\n  Passive = yes\n}}\n")),
        ("fileset/VSet.conf".into(), format!("FileSet {{\n  Name = VSet\n  Include {{\n    Options {{\n      Signature = MD5\n      Compression = GZIP\n      HardLinks = yes\n      Verify = pins5\n    }}\n    File = {DATA}\n  }}\n}}\n")),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let run = |what: &str, command: &str| -> (u32, onesimus_rig::JobRow) {
        let reply = rig.command(command).unwrap_or_else(|e| panic!("{e}"));
        let id = queued_job_id(&reply).unwrap_or_else(|| panic!("{what}: no JobId\n{reply}"));
        let row = rig.wait_job(id, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
        eprintln!("{what}: {row:?}\n{}", rig.job_log(id));
        (id, row)
    };

    let (full, row) = run("full", "run job=VBackup level=Full yes");
    assert_eq!(row.status, 'T', "full: {row:?}\n{}", rig.job_log(full));
    let full_files = row.files;

    eprintln!("change: {}", vm(&host, "change.ps1"));
    // the estimate compares with the last backup: it names the two changed files
    // (and the hard link of one of them) before the incremental job, none after it
    let all = rig.command("estimate job=VBackup level=Full listing").unwrap_or_else(|e| panic!("{e}"));
    assert!(all.contains("big.bin") && all.contains("new.txt"), "the full estimate is incomplete:\n{all}");
    let estimate = rig.command("estimate job=VBackup level=Incremental listing").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("estimate before the incremental job:\n{estimate}");
    assert!(estimate.contains("new.txt") && estimate.contains("a.txt"), "the estimate does not list the changed files:\n{estimate}");
    assert!(!estimate.contains("big.bin"), "the estimate lists an unchanged file:\n{estimate}");

    let (incr, row) = run("incremental", "run job=VBackup level=Incremental yes");
    assert_eq!(row.status, 'T', "incremental: {row:?}\n{}", rig.job_log(incr));
    let names = rig
        .catalog()
        .query(&format!("SELECT f.name FROM file f WHERE f.jobid = {incr} ORDER BY 1"))
        .unwrap_or_default();
    eprintln!("files of the incremental job:\n{names}");
    assert!(names.contains("new.txt") && names.contains("a.txt"), "the changed files are missing:\n{names}");
    assert!(!names.contains("big.bin") && !names.contains("b.bin"), "unchanged files were saved again:\n{names}");
    assert!(row.files < full_files, "the incremental job saved as much as the full one: {row:?} against {full_files}");

    let after = rig.command("estimate job=VBackup level=Incremental").unwrap_or_else(|e| panic!("{e}"));
    assert!(after.contains("files=0"), "the estimate after the incremental job still finds changes:\n{after}");

    // A Verify job (VolumeToCatalog) against a passive client hangs: odir does not
    // tell the SD to connect to the FD (STUBS S-59), so it is not run here.

    let reply = rig
        .command(&format!("restore jobid={full},{incr} client=vfd where={RESTORE} all done yes"))
        .unwrap_or_else(|e| panic!("{e}"));
    let restore = queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("restore: {row:?}\n{}", rig.job_log(restore));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}", rig.job_log(restore));
    let compare = vm(&host, "cmp.ps1");
    eprintln!("compare:\n{compare}");
    assert!(compare.contains("bad=0"), "{compare}");
    let _ = vm(&host, "stop.ps1");
}

/// A file larger than 4 GiB (random blocks at the 32 bit limit and at the end)
/// survives backup and restore on Windows byte for byte.
#[test]
#[ignore = "needs a running ofd.exe on a Windows host (ONESIMUS_WINDOWS_FD) and 10 GB free below D:\\odir-test"]
fn a_file_over_four_gigabytes_through_the_windows_fd() {
    let Some(host) = std::env::var_os("ONESIMUS_WINDOWS_FD").and_then(|h| h.into_string().ok()) else {
        panic!("ONESIMUS_WINDOWS_FD is not set");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    eprintln!("deploy:\n{}", vm(&host, "deploy.ps1"));
    eprintln!("prep:\n{}", vm(&host, "prep.ps1"));
    eprintln!("big:\n{}", vm(&host, "big-prep.ps1"));
    let mut options = RigOptions::new(39891);
    options.start_fd = false;
    options.extra_director = vec![
        ("client/vfd.conf".into(), format!("Client {{\n  Name = vfd\n  Address = {host}\n  FD Port = 19102\n  Password = \"vfd-pw\"\n  Passive = yes\n}}\n")),
        ("fileset/VSet.conf".into(), "FileSet {\n  Name = VSet\n  Include {\n    Options {\n      Signature = MD5\n      Compression = GZIP\n    }\n    File = D:/odir-test/bigdata\n  }\n}\n".into()),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, Duration::from_secs(1200)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("backup: {row:?}\n{}", rig.job_log(backup));
    assert_eq!(row.status, 'T', "backup: {row:?}\n{}", rig.job_log(backup));
    // JobBytes counts what was stored (compressed zeros), the log what the FD read
    let log = rig.job_log(backup);
    let read: u64 = log
        .lines()
        .find_map(|l| l.trim().strip_prefix("FD Bytes Read:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.replace(',', "").parse().ok())
        .unwrap_or(0);
    assert!(read > 4 << 30, "the FD read {read} bytes, less than 4 GiB:\n{log}");
    let reply = rig
        .command(&format!("restore jobid={backup} client=vfd where={RESTORE} all done yes"))
        .unwrap_or_else(|e| panic!("{e}"));
    let restore = queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, Duration::from_secs(1200)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("restore: {row:?}\n{}", rig.job_log(restore));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}", rig.job_log(restore));
    let compare = vm(&host, "big-cmp.ps1");
    eprintln!("compare:\n{compare}");
    assert!(compare.contains("bad=0"), "{compare}");
    let _ = vm(&host, "stop.ps1");
}

/// A file another program holds open without sharing is saved from a VSS
/// shadow copy (`EnableVSS = yes`, the default) and restored with its content.
#[test]
#[ignore = "needs a running ofd.exe on a Windows host (ONESIMUS_WINDOWS_FD)"]
fn a_locked_file_is_saved_from_a_shadow_copy_on_windows() {
    let Some(host) = std::env::var_os("ONESIMUS_WINDOWS_FD").and_then(|h| h.into_string().ok()) else {
        panic!("ONESIMUS_WINDOWS_FD is not set");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    eprintln!("deploy:\n{}", vm(&host, "deploy.ps1"));
    eprintln!("prep:\n{}", vm(&host, "prep.ps1"));
    let mut options = RigOptions::new(39991);
    options.start_fd = false;
    options.extra_director = vec![
        ("client/vfd.conf".into(), format!("Client {{\n  Name = vfd\n  Address = {host}\n  FD Port = 19102\n  Password = \"vfd-pw\"\n  Passive = yes\n}}\n")),
        ("fileset/VSet.conf".into(), format!("FileSet {{\n  Name = VSet\n  EnableVSS = yes\n  Include {{\n    Options {{\n      Signature = MD5\n      Compression = GZIP\n      HardLinks = yes\n    }}\n    File = {DATA}\n  }}\n}}\n")),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    put(&host, "hold.ps1");
    let locked = vm(&host, "lock.ps1");
    eprintln!("lock: {locked}");
    assert!(locked.contains("locked: True"), "the file is not locked: {locked}");
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(backup);
    eprintln!("backup with VSS: {row:?}\n{log}");
    let _ = vm(&host, "unlock.ps1");
    assert!(log.contains("VSS: reading from the shadow copy of D:"), "no shadow copy was used:\n{log}");
    assert!(!log.contains("Cannot open") && !log.contains("locked.bin: "), "the locked file was not read:\n{log}");
    assert_eq!(row.status, 'T', "backup: {row:?}\n{log}");

    let reply = rig
        .command(&format!("restore jobid={backup} client=vfd where={RESTORE} all done yes"))
        .unwrap_or_else(|e| panic!("{e}"));
    let restore = queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}", rig.job_log(restore));
    let content = vm(&host, "show-locked.ps1");
    eprintln!("restored locked.bin: {content}");
    assert!(content.contains("locked content"), "the restored file has not the content of the locked one: {content}");
    let _ = vm(&host, "stop.ps1");
}

/// A UNC path (`//host/share`, a network drive reached by its share name) is
/// saved and restored: the share `odirunc` of `unc-prep.ps1` is on the
/// Windows host itself and read through SMB.
#[test]
#[ignore = "needs a running ofd.exe on a Windows host (ONESIMUS_WINDOWS_FD)"]
fn a_unc_path_through_the_windows_fd() {
    let Some(host) = std::env::var_os("ONESIMUS_WINDOWS_FD").and_then(|h| h.into_string().ok()) else {
        panic!("ONESIMUS_WINDOWS_FD is not set");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    eprintln!("deploy:\n{}", vm(&host, "deploy.ps1"));
    eprintln!("prep:\n{}", vm(&host, "prep.ps1"));
    let unc = vm(&host, "unc-prep.ps1");
    eprintln!("unc-prep:\n{unc}");
    assert!(unc.contains("read: unc a"), "the share does not work: {unc}");
    let mut options = RigOptions::new(40091);
    options.start_fd = false;
    options.extra_director = vec![
        ("client/vfd.conf".into(), format!("Client {{\n  Name = vfd\n  Address = {host}\n  FD Port = 19102\n  Password = \"vfd-pw\"\n  Passive = yes\n}}\n")),
        ("fileset/VSet.conf".into(), "FileSet {\n  Name = VSet\n  Include {\n    Options {\n      Signature = MD5\n    }\n    File = //localhost/odirunc\n  }\n}\n".to_string()),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(backup);
    eprintln!("backup: {row:?}\n{log}");
    let names = rig.catalog().query(&format!("SELECT p.path || f.name FROM file f JOIN path p ON p.pathid = f.pathid WHERE f.jobid = {backup} ORDER BY 1")).unwrap_or_default();
    eprintln!("catalog names:\n{names}");
    assert_eq!(row.status, 'T', "backup: {row:?}\n{log}");
    assert!(names.contains("//localhost/odirunc/sub/big.bin"), "the UNC names are not in the catalog as sent:\n{names}");
    let hex = rig.catalog().query(&format!("SELECT encode(convert_to(f.name, 'SQL_ASCII'), 'hex') FROM file f WHERE f.jobid = {backup} AND f.name LIKE '%.txt' AND f.name <> 'a.txt'")).unwrap_or_default();
    eprintln!("hex of the unicode name: {hex}");
    assert!(hex.contains("c3bc6ec3af2e747874"), "the unicode name is not UTF-8 in the catalog:\n{hex}");
    let reply = rig.command(&format!("restore jobid={backup} client=vfd where=D:/odir-test/uncrestore all done yes")).unwrap_or_else(|e| panic!("{e}"));
    let restore = queued_job_id(&reply).unwrap_or_else(|| panic!("restore: no JobId\n{reply}"));
    let row = rig.wait_job(restore, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("restore: {row:?}\n{}", rig.job_log(restore));
    assert_eq!(row.status, 'T', "restore: {row:?}\n{}", rig.job_log(restore));
    let list = vm(&host, "unc-list.ps1");
    eprintln!("{list}");
    // the same files by content (the console shows the unicode name garbled, so names are not compared)
    let hashes = |part: &str| -> Vec<String> {
        let mut v: Vec<String> = part.lines().filter(|l| l.starts_with('\\')).map(|l| l.trim_end().rsplit(' ').next().unwrap_or("").to_string()).collect();
        v.sort();
        v
    };
    let (source, restored) = list.split_once("== D:\\odir-test\\uncrestore").unwrap_or_else(|| panic!("{list}"));
    assert_eq!(hashes(source).len(), 3, "{list}");
    assert_eq!(hashes(source), hashes(restored), "source and restore differ:\n{list}");
    // restore onto the share itself: the files arrive below its directory on the host
    let reply = rig.command(&format!("restore jobid={backup} client=vfd where=//localhost/odirunc/back all done yes")).unwrap_or_else(|e| panic!("{e}"));
    let again = queued_job_id(&reply).unwrap_or_else(|| panic!("restore to the share: no JobId\n{reply}"));
    let row = rig.wait_job(again, Duration::from_secs(300)).unwrap_or_else(|e| panic!("{e}"));
    eprintln!("restore to the share: {row:?}\n{}", rig.job_log(again));
    assert_eq!(row.status, 'T', "restore to the share: {row:?}\n{}", rig.job_log(again));
    let back = vm(&host, "unc-back.ps1");
    eprintln!("{back}");
    assert!(back.contains("back: 3 files, same hashes: True"), "{back}");
    let _ = vm(&host, "stop.ps1");
}

/// A whole drive (`File = C:/`) with the big system and tool trees excluded (the VM's C: holds 90 GB): the walk
/// meets junctions, locked system files and `System Volume Information`.
/// `ONESIMUS_WINDOWS_DRIVE_BACKUP=0` runs only the estimate.
#[test]
#[ignore = "needs a running ofd.exe on a Windows host (ONESIMUS_WINDOWS_FD)"]
fn a_whole_drive_through_the_windows_fd() {
    let Some(host) = std::env::var_os("ONESIMUS_WINDOWS_FD").and_then(|h| h.into_string().ok()) else {
        panic!("ONESIMUS_WINDOWS_FD is not set");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    eprintln!("deploy:\n{}", vm(&host, "deploy.ps1"));
    eprintln!("prep:\n{}", vm(&host, "prep.ps1"));
    let mut options = RigOptions::new(40191);
    options.start_fd = false;
    options.extra_director = vec![
        ("client/vfd.conf".into(), format!("Client {{\n  Name = vfd\n  Address = {host}\n  FD Port = 19102\n  Password = \"vfd-pw\"\n  Passive = yes\n}}\n")),
        ("fileset/VSet.conf".into(), "FileSet {\n  Name = VSet\n  Include {\n    Options {\n      Signature = MD5\n      Compression = GZIP\n      fstype = ntfs\n    }\n    File = C:/\n  }\n  Exclude {\n    File = C:/Windows\n    File = \"C:/Program Files\"\n    File = \"C:/Program Files (x86)\"\n    File = C:/ProgramData\n    File = C:/Qt\n    File = C:/msys64\n    File = C:/vcpkg\n    File = C:/cmake\n    File = C:/Strawberry\n    File = C:/ESD\n    File = \"C:/$WINDOWS.~BT\"\n    File = C:/forgejo-runner\n    File = C:/v\n    File = C:/tmp\n  }\n}\n".to_string()),
    ];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let started = std::time::Instant::now();
    let estimate = rig.command("estimate job=VBackup level=Full").unwrap_or_else(|e| panic!("{e}"));
    eprintln!("estimate in {:?}:\n{estimate}", started.elapsed());
    if std::env::var("ONESIMUS_WINDOWS_DRIVE_BACKUP").as_deref() == Ok("0") {
        let _ = vm(&host, "stop.ps1");
        return;
    }
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, Duration::from_secs(3 * 3600)).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(backup);
    eprintln!("backup in {:?}: {row:?}\n{log}", started.elapsed());
    let _ = vm(&host, "stop.ps1");
}
