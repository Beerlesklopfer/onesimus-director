// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Recording of a client-initiated connection (`Connection From Client To Director`) between the
//! original bareos-dir and a Bareos FD, in clear text through a recording proxy (docs/plans/CLIENT-INITIIERT.md).
//!
//! Needs `/usr/sbin/bareos-dir` and `ONESIMUS_BAREOS_FD=/path/to/bareos-fd`.
//! Start: `cargo test -p onesimus-rig --test client_initiated_capture -- --ignored --nocapture`.
//! The recording is printed and written to `<rig root>/capture.txt` (`ONESIMUS_RIG_KEEP=1` keeps it).

#![deny(missing_docs)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use onesimus_rig::{Rig, RigOptions};

/// The recorded frames: (seconds since the start, direction, text of the frame).
type Log = Arc<Mutex<Vec<(f64, &'static str, String)>>>;

/// Copies BNET frames from `from` to `to` and records each one under `dir`.
fn pump(mut from: TcpStream, mut to: TcpStream, dir: &'static str, log: Log, start: Instant) {
    loop {
        let mut len = [0u8; 4];
        if from.read_exact(&mut len).is_err() {
            log.lock().unwrap().push((start.elapsed().as_secs_f64(), dir, "<closed>".into()));
            let _ = to.shutdown(std::net::Shutdown::Write);
            return;
        }
        let n = i32::from_be_bytes(len);
        let mut body = vec![0u8; n.max(0) as usize];
        if n > 0 && from.read_exact(&mut body).is_err() {
            return;
        }
        let text = if n < 0 { format!("<signal {n}>") } else { String::from_utf8_lossy(&body).trim_end().to_string() };
        log.lock().unwrap().push((start.elapsed().as_secs_f64(), dir, text));
        // one write per frame: bareos-dir peeks at the first frame to tell TLS from clear text
        let mut frame = len.to_vec();
        frame.extend_from_slice(&body);
        if to.write_all(&frame).is_err() {
            return;
        }
    }
}

/// A proxy on `listen` that forwards to `target` and records every frame of every connection.
fn proxy(listen: u16, target: u16, log: Log) {
    let listener = TcpListener::bind(("127.0.0.1", listen)).unwrap_or_else(|e| panic!("proxy: {e}"));
    let start = Instant::now();
    std::thread::spawn(move || {
        for (n, client) in listener.incoming().flatten().enumerate() {
            log.lock().unwrap().push((start.elapsed().as_secs_f64(), "--", format!("connection {n} from the FD")));
            let Ok(server) = TcpStream::connect(("127.0.0.1", target)) else { continue };
            let (c2, s2) = (client.try_clone().unwrap(), server.try_clone().unwrap());
            let (l1, l2) = (log.clone(), log.clone());
            std::thread::spawn(move || pump(client, server, "FD->DIR", l1, start));
            std::thread::spawn(move || pump(s2, c2, "DIR->FD", l2, start));
        }
    });
}

/// The FD connects to the Director itself and keeps the connection; an estimate and `status client`
/// run while it waits; a job with `Run On Incoming Connect Interval` shows whether the Director starts it.
#[test]
#[ignore = "needs bareos-dir, bareos-sd, initdb and a bareos-fd (ONESIMUS_BAREOS_FD): run with --ignored --nocapture"]
fn record_client_initiated_connection_with_the_originals() {
    let Some(fd) = std::env::var_os("ONESIMUS_BAREOS_FD").map(PathBuf::from) else {
        panic!("ONESIMUS_BAREOS_FD is not set (path of a bareos-fd binary)");
    };
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let base = 40101 + 300;
    let proxy_port = base + 9;
    let log: Log = Arc::default();
    proxy(proxy_port, base, log.clone());

    let mut options = RigOptions::new(base);
    options.dir_program = Some(PathBuf::from("/usr/sbin/bareos-dir"));
    options.fd_program = Some(fd);
    options.fd_plain = true;
    options.fd_director_lines = vec![
        "Connection From Client To Director = yes".into(),
        "Address = 127.0.0.1".into(),
        format!("Port = {proxy_port}"),
    ];
    options.dir_client_lines = vec!["Connection From Client To Director = yes".into(), "Connection From Director To Client = no".into()];
    options.extra_director = vec![(
        "job/VIncoming.conf".into(),
        "Job {\n  Name = VIncoming\n  JobDefs = VDefs\n  Type = Backup\n  Level = Full\n  Run On Incoming Connect Interval = 1 hour\n}\n".into(),
    )];
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    // the FD connected before the Director listened: it tries again after 60 s
    let deadline = Instant::now() + Duration::from_secs(150);
    while Instant::now() < deadline && !log.lock().unwrap().iter().any(|(_, d, t)| *d == "FD->DIR" && t.starts_with("Hello Client")) {
        std::thread::sleep(Duration::from_millis(500));
    }
    std::thread::sleep(Duration::from_secs(5));
    let mut notes = Vec::new();
    for command in ["status client=vfd", "estimate job=VBackup listing", "list jobs", "status dir"] {
        let reply = rig.command(command).unwrap_or_else(|e| format!("<error {e}>"));
        notes.push(format!("=== console: {command}\n{reply}"));
        std::thread::sleep(Duration::from_secs(3));
    }
    // Run On Incoming Connect Interval: a VIncoming job of five hours ago; the next connection of the FD
    // (it reconnects after each use, here after `status client`) should start the job
    let inserted = rig.catalog().query(
        "INSERT INTO Job (Job, Name, Type, Level, ClientId, JobStatus, SchedTime, StartTime, EndTime, JobTDate) \
         SELECT 'VIncoming.old', 'VIncoming', 'B', 'F', ClientId, 'T', now() - interval '5 hours', \
         now() - interval '5 hours', now() - interval '5 hours', extract(epoch from now() - interval '5 hours')::bigint \
         FROM Client WHERE Name = 'vfd' RETURNING JobId",
    );
    notes.push(format!("=== inserted an old VIncoming job: {inserted:?}"));
    for command in ["status client=vfd"] {
        let reply = rig.command(command).unwrap_or_else(|e| format!("<error {e}>"));
        notes.push(format!("=== console: {command}\n{reply}"));
    }
    // the job goes to the scheduler's queue (AddJobWithNoRunResourceToQueue) and starts at its next look
    std::thread::sleep(Duration::from_secs(75));
    for command in ["list jobs", "messages"] {
        let reply = rig.command(command).unwrap_or_else(|e| format!("<error {e}>"));
        notes.push(format!("=== console: {command}\n{reply}"));
    }
    std::thread::sleep(Duration::from_secs(5));
    let mut text = String::new();
    for (t, d, line) in log.lock().unwrap().iter() {
        text.push_str(&format!("{t:8.3} {d:8} {line}\n"));
    }
    text.push('\n');
    text.push_str(&notes.join("\n"));
    text.push_str(&format!("\n=== Director log (end)\n{}\n", rig.daemon_log("odir").lines().rev().take(80).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n")));
    let _ = std::fs::write(rig.layout.root.join("capture.txt"), &text);
    println!("{text}");
    assert!(text.contains("Hello Client"), "the FD never connected");
}
