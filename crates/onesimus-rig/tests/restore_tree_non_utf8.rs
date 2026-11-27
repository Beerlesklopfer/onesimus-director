// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The restore tree shows and marks a file whose name is not UTF-8, read from a `SQL_ASCII` catalog (C-047).
//!
//! Start: `cargo test -p onesimus-rig --test restore_tree_non_utf8 -- --ignored`.

#![deny(missing_docs)]

use std::os::unix::ffi::OsStrExt as _;
use std::time::Duration;

use onesimus_rig::tree::NON_UTF8_NAME;
use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Longest time one dialog step may take.
const STEP: Duration = Duration::from_secs(60);

/// `ls` in the data directory of the tree lists `caf\xe9.txt` with its bytes, as bareos-dir sends them;
/// `mark caf*` selects it and the restore brings back only that file.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn tree_lists_and_marks_a_latin1_name() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let rig = Rig::start(RigOptions::new(40071)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let backup = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(backup, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}\n{}", rig.diagnostics());

    // The catalog has the bytes of the file system, as bareos-dir writes them (not U+FFFD).
    let names = rig.catalog().query(&format!("SELECT encode(textsend(Name), 'hex') FROM File WHERE JobId = {backup}"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(names.contains("636166e92e747874"), "{names}");

    let mut console = rig.console().unwrap_or_else(|e| panic!("{e}"));
    let step = |console: &mut onesimus_rig::console::Console, line: &str| {
        console.command_bytes(line, STEP).unwrap_or_else(|e| panic!("{line}: {e}\n{}", rig.diagnostics()))
    };
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();

    let opened = step(&mut console, &format!("restore jobid={backup} client=vfd where={}", rig.layout.restore.display()));
    assert!(text(&opened).contains("You are now entering file selection mode"), "{}", text(&opened));

    let cd = step(&mut console, &format!("cd {}", rig.layout.data.display()));
    assert!(text(&cd).contains("cwd is:"), "{}", text(&cd));

    let ls = step(&mut console, "ls");
    let mut wanted = NON_UTF8_NAME.to_vec();
    wanted.push(b'\n');
    assert!(ls.windows(wanted.len()).any(|w| w == wanted), "ls:\n{}", text(&ls));

    let marked = step(&mut console, "mark caf*");
    assert!(text(&marked).contains("1 file newly marked."), "{}", text(&marked));

    let done = step(&mut console, "done");
    assert!(text(&done).contains("OK to run?"), "{}", text(&done));
    let queued = text(&step(&mut console, "yes"));
    let id = onesimus_rig::console::queued_job_id(&queued).unwrap_or_else(|| panic!("no JobId:\n{queued}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert!(row.status == 'T' && row.files == 1, "{row:?}\n{}\n{}", rig.job_log(id), rig.diagnostics());

    let restored = rig.layout.restored_data().join(std::ffi::OsStr::from_bytes(NON_UTF8_NAME));
    assert_eq!(std::fs::read_to_string(&restored).unwrap_or_default(), "latin-1 name\n", "{}", restored.display());
    assert!(!rig.layout.restored_data().join("file0.txt").exists());
}
