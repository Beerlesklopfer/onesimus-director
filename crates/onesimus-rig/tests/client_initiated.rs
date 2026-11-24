// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Client initiated connection between odir and ofd (`Connection From Client To Director`, S-46,
//! docs/plans/CLIENT-INITIIERT.md): the Director cannot reach the FD itself (wrong FD port in its
//! Client resource), so everything that talks to the FD has to take the connection the FD opened.
//!
//! Start: `cargo test -p onesimus-rig --test client_initiated -- --ignored --nocapture`.

#![deny(missing_docs)]

use std::time::{Duration, Instant};

use onesimus_rig::{Rig, RigOptions};

/// Longest time a job of the rig may take.
const JOB_TIMEOUT: Duration = Duration::from_secs(180);

/// A rig whose FD connects to the Director (TLS or clear text) and whose Director cannot connect to
/// the FD; `extra` are further Director resources.
fn start(base: u16, plain: bool, extra: Vec<(String, String)>) -> Rig {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(base);
    options.fd_plain = plain;
    options.fd_director_lines =
        vec!["Connection From Client To Director = yes".into(), "Address = 127.0.0.1".into(), format!("Port = {base}")];
    // port 1: every connection the Director makes itself is refused
    options.dir_client_lines = vec!["Connection From Client To Director = yes".into(), "FD Port = 1".into()];
    options.extra_director = extra;
    Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"))
}

/// Waits until `status dir` lists a waiting connection of the rig client (the FD started before the
/// Director and tries again after 60 s).
fn wait_for_connection(rig: &Rig) -> String {
    let deadline = Instant::now() + Duration::from_secs(100);
    loop {
        let status = rig.command("status dir").unwrap_or_default();
        let listed = status.split("Client Initiated Connections").nth(1).is_some_and(|s| s.contains(" vfd"));
        if listed || Instant::now() > deadline {
            return status;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// status client, a backup and an estimate run over the connections the FD opened; after each use the
/// FD connects again.
fn check_waiting_connection(base: u16, plain: bool) {
    let rig = start(base, plain, Vec::new());
    let status = wait_for_connection(&rig);
    assert!(status.split("Client Initiated Connections").nth(1).is_some_and(|s| s.contains(" vfd")),
        "no waiting connection:\n{status}\n{}", rig.diagnostics());

    let client = rig.command("status client=vfd").unwrap_or_else(|e| panic!("{e}"));
    assert!(client.contains("vfd Version:"), "status client over the waiting connection:\n{client}\n{}", rig.diagnostics());
    assert!(client.contains(if plain { "Handshake: Cleartext" } else { "Handshake: Immediate TLS" }), "{client}");

    wait_for_connection(&rig);
    let id = rig.run_job("VBackup").unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!((row.status, row.files), ('T', 13), "{row:?}\n{}\n{}", rig.job_log(id), rig.diagnostics());

    wait_for_connection(&rig);
    let estimate = rig.command("estimate job=VBackup listing").unwrap_or_else(|e| panic!("{e}"));
    assert!(estimate.contains("file0.txt"), "{estimate}\n{}", rig.diagnostics());

    wait_for_connection(&rig);
    let restore = rig.restore(id).unwrap_or_else(|e| panic!("{e}"));
    let row = rig.wait_job(restore, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(row.status, 'T', "{row:?}\n{}", rig.job_log(restore));
    let (rc, diff) = rig.diff_restore();
    assert_eq!(rc, 0, "{diff}");
}

/// The FD connects with TLS-PSK.
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn jobs_and_console_commands_use_the_connection_of_the_fd_with_tls() {
    check_waiting_connection(40501, false);
}

/// The FD connects in clear text (`TLS Enable = no` on both sides).
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn jobs_and_console_commands_use_the_connection_of_the_fd_in_clear_text() {
    check_waiting_connection(40521, true);
}

/// `Run On Incoming Connect Interval`: a job of the client that never ran starts when the FD
/// connects, triggered by "Client".
#[test]
#[ignore = "needs bareos-sd and initdb: run with --ignored"]
fn a_connection_starts_the_jobs_of_the_client_that_are_due() {
    let rig = start(40541, false, vec![(
        "job/VIncoming.conf".into(),
        "Job {\n  Name = VIncoming\n  JobDefs = VDefs\n  Type = Backup\n  Level = Full\n  Run On Incoming Connect Interval = 1 hour\n}\n".into(),
    )]);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut id = None;
    while id.is_none() && Instant::now() < deadline {
        let out = rig.catalog().query("SELECT JobId FROM Job WHERE Name='VIncoming' ORDER BY JobId LIMIT 1").unwrap_or_default();
        id = out.lines().next().and_then(|l| l.trim().parse::<u32>().ok());
        std::thread::sleep(Duration::from_secs(2));
    }
    let id = id.unwrap_or_else(|| panic!("VIncoming did not start\n{}", rig.diagnostics()));
    let row = rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"));
    let log = rig.job_log(id);
    assert_eq!(row.status, 'T', "{row:?}\n{log}");
    assert!(log.contains("Job triggered by:       Client"), "{log}");
}
