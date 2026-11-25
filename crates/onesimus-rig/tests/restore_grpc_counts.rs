// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Restore with `replace=never` into a new directory, odir + ofd + osd over gRPC and over BNET:
//! the job counts the restored files and ends OK (seen on PDC 25.11.2026: over gRPC "Files
//! Restored: 0", an FD error and "SD termination status: Running", though every file was written).
//!
//! Start: `cargo test -p onesimus-rig --test restore_grpc_counts -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Grpc, Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Backup, then restore with replace=never; the restore's row and log.
fn restore_counts(base: u16, grpc: Grpc, fd: Option<std::path::PathBuf>) -> (onesimus_rig::JobRow, onesimus_rig::JobRow, String) {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    // ONESIMUS_OSD: another osd build, e.g. the static i686 one of a 32-bit host
    options.sd = Some(std::env::var_os("ONESIMUS_OSD").map(Into::into).unwrap_or_else(|| onesimus_rig::prereq::target_profile_dir().join("osd")));
    options.fd_grpc = grpc;
    options.sd_grpc = grpc;
    if fd.is_some() {
        options.fd_program = fd;
    }
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let b = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let reply = rig.command(&format!("restore jobid={backup} client=vfd where={} replace=never all done yes", rig.layout.restore.display()))
        .unwrap_or_else(|e| panic!("{e}"));
    let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("{reply}"));
    let r = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let (code, diff) = rig.diff_restore();
    assert_eq!(code, 0, "{diff}");
    (b, r, rig.job_log(id))
}

/// gRPC and BNET count the same.
#[test]
#[ignore = "needs osd and initdb: run with --ignored --nocapture"]
fn restore_counts_files_over_grpc_and_bnet() {
    let (b, r, log) = restore_counts(41901, Grpc::On, None);
    println!("=== gRPC: backup {b:?}\nrestore {r:?}\n{log}");
    let (b2, r2, log2) = restore_counts(41941, Grpc::Off, None);
    println!("=== BNET: backup {b2:?}\nrestore {r2:?}\n{log2}");
    assert_eq!((r2.status, r2.files), ('T', b2.files), "BNET");
    assert_eq!((r.status, r.files), ('T', b.files), "gRPC");
}

/// The installed ofd of this host (an older build) against the current odir and osd over gRPC.
/// Set ONESIMUS_OLD_OFD to its path (e.g. /usr/sbin/ofd).
#[test]
#[ignore = "needs an older ofd in ONESIMUS_OLD_OFD"]
fn restore_with_an_older_ofd_over_grpc() {
    let old = std::env::var_os("ONESIMUS_OLD_OFD").map(std::path::PathBuf::from).expect("ONESIMUS_OLD_OFD");
    let (b, r, log) = restore_counts(41981, Grpc::On, Some(old));
    println!("=== old ofd over gRPC: backup {b:?}\nrestore {r:?}\n{log}");
    assert_eq!((r.status, r.files), ('T', b.files));
}

/// A long restore (BigBackup, about 50 MB through the FD's bandwidth limit) over gRPC.
#[test]
#[ignore = "needs osd and initdb"]
fn a_long_restore_over_grpc_counts_its_files() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(42021);
    options.sd = Some(onesimus_rig::prereq::target_profile_dir().join("osd"));
    options.fd_grpc = Grpc::On;
    options.sd_grpc = Grpc::On;
    if let Some(old) = std::env::var_os("ONESIMUS_OLD_OFD") {
        options.fd_program = Some(old.into());
    }
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
    let b = rig.wait_job(big, Duration::from_secs(600)).unwrap_or_else(|e| panic!("{e}"));
    let target = rig.layout.root.join("restore-big");
    let started = std::time::Instant::now();
    let reply = rig.command(&format!("restore jobid={big} client=vfd where={} replace=never all done yes", target.display()))
        .unwrap_or_else(|e| panic!("{e}"));
    let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("{reply}"));
    let r = rig.wait_job(id, Duration::from_secs(600)).unwrap_or_else(|e| panic!("{e}"));
    println!("=== long restore ({:?}): backup {b:?}\nrestore {r:?}\n{}", started.elapsed(), rig.job_log(id));
    assert_eq!((r.status, r.files), ('T', b.files));
}
