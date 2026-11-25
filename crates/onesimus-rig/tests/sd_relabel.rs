// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `relabel` of a purged File volume: bareos-sd 25.1.3 (with bareos-dir and with odir) against osd.
//!
//! Measured 25.11.2026: bareos-sd writes the new label into the file of the old volume and keeps
//! its name (`VVol-0001` holds `NewVol`); the next backup finds no file `NewVol`, marks the volume
//! `Error` and waits for the operator. osd renames the file as well, so the relabeled volume is
//! usable (C-096). Reply and catalog after `relabel` are the same.
//!
//! Start: `cargo test -p onesimus-rig --test sd_relabel -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::path::PathBuf;
use std::time::Duration;

use onesimus_rig::{Rig, RigOptions};

/// Longest time the first backup may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// Time the backup after the relabel gets before it counts as waiting.
const SECOND_TIMEOUT: Duration = Duration::from_secs(45);

/// What one run shows.
#[derive(Debug, PartialEq)]
struct Outcome {
    /// Reply of `relabel` (without addresses, paths and TLS lines).
    reply: String,
    /// Files of the volume directory after `relabel`.
    files: Vec<String>,
    /// Media rows after `relabel`.
    media: String,
    /// Status of the backup after `relabel`, `None` while it still waits.
    second: Option<char>,
}

/// Backup, delete its job, purge the volume, relabel it to `NewVol`, back up again.
fn relabel(base: u16, sd: Option<PathBuf>, director: Option<PathBuf>) -> Outcome {
    onesimus_rig::build_workspace_packages(&["odir", "ofd", "osd-daemon"]).unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.sd = sd;
    options.dir_program = director;
    options.extra_director = vec![(
        "pool/VPool.conf".into(),
        "Pool {\n  Name = VPool\n  Pool Type = Backup\n  Label Format = \"VVol-\"\n  Maximum Volumes = 1\n  Recycle = yes\n  \
         AutoPrune = no\n  Maximum Volume Bytes = 1G\n}\n".into(),
    )];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let first = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(rig.wait_job(first, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}")).status, 'T');
    for command in [format!("delete jobid={first} yes"), "update volume=VVol-0001 volstatus=Used".into(), "purge volume=VVol-0001 yes".into()] {
        rig.command(&command).unwrap_or_else(|e| panic!("{e}"));
    }
    let reply = rig.command("relabel storage=vsd oldvolume=VVol-0001 volume=NewVol pool=VPool yes").unwrap_or_else(|e| panic!("{e}"));
    let mut files: Vec<String> = std::fs::read_dir(&rig.layout.storage)
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    files.sort();
    let media = rig.catalog()
        .query("SELECT VolumeName, VolStatus, VolJobs, VolBytes, PoolId, MediaType FROM Media ORDER BY MediaId")
        .unwrap_or_default();
    let second = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let second = rig.wait_job(second, SECOND_TIMEOUT).ok().map(|row| row.status);
    let reply = reply.lines()
        .map(|l| l.split(" (/tmp").next().unwrap_or("").to_string())
        .filter(|l| !l.contains("Connecting to Storage") && !l.contains("Encryption") && !l.starts_with("Handshake"))
        .filter(|l| !l.starts_with("Automatically selected") && !l.starts_with("Using Catalog"))
        .collect::<Vec<_>>().join("\n");
    Outcome { reply, files, media, second }
}

/// osd relabels like bareos-sd and renames the volume file.
#[test]
#[ignore = "needs bareos-dir, bareos-sd, osd and initdb: run with --ignored --nocapture"]
fn relabel_of_a_purged_volume() {
    let original = relabel(41541, None, Some(PathBuf::from("/usr/sbin/bareos-dir")));
    println!("=== bareos-dir + bareos-sd: {original:#?}");
    let bareos = relabel(41501, None, None);
    println!("=== odir + bareos-sd: {bareos:#?}");
    let onesimus = relabel(41521, Some(onesimus_rig::prereq::target_profile_dir().join("osd")), None);
    println!("=== odir + osd: {onesimus:#?}");
    // the original: the file keeps the old name, the next backup waits
    assert_eq!(original.files, vec!["VVol-0001".to_string()]);
    assert_eq!(original.second, None);
    assert_eq!((&bareos.reply, &bareos.files, bareos.second), (&original.reply, &original.files, original.second));
    // osd: same reply and catalog (the label's size differs with the program version and host name
    // written into it), the file renamed, the next backup runs
    let bytes = |s: &str| {
        let mut out = String::new();
        let mut digits = false;
        for c in s.chars() {
            if c.is_ascii_digit() && digits {
                continue;
            }
            digits = c == '=' || (digits && c.is_ascii_digit());
            out.push(c);
        }
        out
    };
    assert_eq!(bytes(&onesimus.reply), bytes(&bareos.reply));
    let media = |s: &str| s.split('|').enumerate().map(|(i, f)| if i == 3 { "<bytes>" } else { f }).collect::<Vec<_>>().join("|");
    assert_eq!(media(&onesimus.media), media(&bareos.media));
    assert_eq!(onesimus.files, vec!["NewVol".to_string()]);
    assert_eq!(onesimus.second, Some('T'));
}
