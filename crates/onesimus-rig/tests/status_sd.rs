// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `status storage` against bareos-sd and against osd (C-014): the same text, apart from the
//! values that differ by nature (version, times, rig path, binary, counters osd does not have).
//!
//! Start: `cargo test -p onesimus-rig --test status_sd -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// `text` with the parts replaced that differ by nature.
fn normalized(rig: &Rig, text: &str) -> String {
    let root = rig.layout.root.display().to_string();
    let mut out = Vec::new();
    for line in text.replace(&root, "<root>").lines() {
        let line = if line.starts_with("Connecting to Storage daemon") {
            "Connecting".to_string()
        } else if line.contains(" Version: ") {
            "<name> Version: <v>".to_string()
        } else if line.starts_with("Daemon started") {
            "Daemon started <t>".to_string()
        } else if line.trim_start().starts_with("Files=") {
            "    Files=<n> Bytes=<n> AveBytes/sec=<n> LastBytes/sec=<n>".to_string()
        } else if line.trim_start().starts_with("FDReadSeqNo=") || line.trim_start().starts_with("Total Bytes=")
            || line.trim_start().starts_with("Positioned at") || line.trim_start().starts_with("Health:") {
            continue;
        } else if line.starts_with("JobId=") {
            line.split_once(' ').map_or(String::new(), |(_, rest)| format!("JobId=<n> {rest}"))
        } else if line.starts_with("Attached Jobs:") {
            "Attached Jobs: <n>".to_string()
        } else if line.starts_with("  File=0 block=") {
            "  File=0 block=<n>".to_string()
        } else if line.starts_with("     ") && line.contains(" Full ") || line.contains(" VBackup") || line.contains(" BigBackup") {
            // a row of Terminated Jobs: the time and the byte count differ
            let mut f: Vec<&str> = line.split_whitespace().collect();
            f.drain(4..f.len().saturating_sub(1).max(4));
            f.remove(0); // the JobId differs: the second daemon runs after the first in one catalog
            f.join(" ")
        } else {
            line.to_string()
        };
        out.push(line);
    }
    out.join("\n")
}

/// Runs the same steps against both storage daemons and compares the text of `status storage`.
#[test]
#[ignore = "needs bareos-sd, osd and initdb: run with --ignored --nocapture"]
fn status_storage_is_the_same_for_both_storage_daemons() {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut rig = Rig::start(RigOptions::new(39971)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let status = |rig: &Rig| rig.command("status storage=vsd").unwrap_or_else(|e| panic!("{e}"));
    let mut seen = Vec::new();
    for daemon in ["bareos-sd", "osd"] {
        if daemon == "osd" {
            rig.replace_sd(&onesimus_rig::prereq::target_profile_dir().join("osd")).unwrap_or_else(|e| panic!("{e}"));
        }
        let idle = normalized(&rig, &status(&rig));
        let id = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
        rig.wait_job(id, Duration::from_secs(180)).unwrap_or_else(|e| panic!("{e}"));
        let after = normalized(&rig, &status(&rig));
        let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
        rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
        std::thread::sleep(Duration::from_secs(3));
        let running = normalized(&rig, &status(&rig));
        let _ = rig.command(&format!("cancel jobid={big} yes"));
        rig.wait_job(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
        seen.push((daemon, idle, after, running));
    }
    for (what, i) in [("idle", 1), ("after a backup", 2), ("running", 3)] {
        let pick = |n: usize| match i {
            1 => &seen[n].1,
            2 => &seen[n].2,
            _ => &seen[n].3,
        };
        let (a, b) = (pick(0), pick(1));
        if a != b {
            let diff: Vec<String> = a.lines().zip(b.lines()).enumerate()
                .filter(|(_, (x, y))| x != y).map(|(n, (x, y))| format!("{n}: bareos-sd {x:?}\n   osd {y:?}")).collect();
            panic!("status storage {what}: {} / {} lines\n{}\n=== bareos-sd ===\n{a}\n=== osd ===\n{b}", a.lines().count(), b.lines().count(), diff.join("\n"));
        }
    }
}
