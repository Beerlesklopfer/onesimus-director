// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `Exclude` inside `Options` (overlay of `odir.d`) with a Bareos File Daemon: the job does not
//! start and says why (docs/wiki/betrieb-odir.md § FileSet).
//!
//! Needs a real `bareos-fd`: `ONESIMUS_BAREOS_FD=/path/to/bareos-fd` (an extracted package is
//! enough). Start: `cargo test -p onesimus-rig --test bareos_fd_exclude -- --ignored`.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// A backup with a Bareos FD works; with the overlay `Exclude` in `Options` the job ends with an error
/// that names the cause and the FD is not given the include list that it would not understand.
#[test]
#[ignore = "needs bareos-sd, initdb and a bareos-fd (ONESIMUS_BAREOS_FD): run with --ignored"]
fn bareos_fd_refuses_the_overlay_exclude_with_a_clear_message() {
    let Some(fd) = std::env::var_os("ONESIMUS_BAREOS_FD").map(PathBuf::from) else {
        panic!("ONESIMUS_BAREOS_FD is not set (path of a bareos-fd binary)");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(39991);
    options.fd_program = Some(fd);
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    let id = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "a Bareos FD backs the tree up: {row:?}\n{}\n{}", rig.job_log(id), rig.diagnostics());

    let overlay = rig.layout.root.join("onesimus").join("odir.d").join("fileset");
    std::fs::create_dir_all(&overlay).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(
        overlay.join("VSet.conf"),
        format!("FileSet {{\n  Name = VSet\n  Include {{\n    Options {{\n      Index = 1\n      Exclude {{\n        File = {}\n      }}\n    }}\n  }}\n}}\n", rig.layout.data.join("sub").display()),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let reload = rig.command("reload").unwrap_or_else(|e| panic!("{e}"));
    assert!(!reload.to_lowercase().contains("error"), "reload: {reload}");

    let id = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(id);
    assert_ne!(row.status, 'T', "the job must not succeed with data that should be excluded: {row:?}\n{log}");
    assert!(log.contains("that needs an Onesimus ofd"), "{log}");
}
