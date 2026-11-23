// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Measurement for docs/STUBS.md S-48: what the job monitor and the
//! JobManager mutex cost the Director while jobs run.
//!
//! Start: `cargo test -p onesimus-rig --test perf_s48 -- --ignored --nocapture`
//!
//! Three phases on one rig: the Director idle, with `BigBackup` running
//! (throttled to 200 kb/s, so it runs for the whole phase) and with two more
//! jobs queued behind it. Per phase it prints the CPU seconds of the
//! Director per thread (`/proc/<pid>/task/*/stat`) and the answer times of
//! `status dir`. It asserts nothing about speed: the numbers decide whether
//! S-48 needs a change, and they go into docs/STUBS.md.

#![deny(missing_docs)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use onesimus_rig::{Rig, RigOptions};

/// Length of one measuring phase.
const PHASE: Duration = Duration::from_secs(20);
/// Calls of `status dir` per phase.
const STATUS_CALLS: usize = 20;

/// CPU seconds (user + system) per thread name of process `pid`.
fn cpu_by_thread(pid: u32) -> BTreeMap<String, f64> {
    // SAFETY: sysconf has no preconditions.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(format!("/proc/{pid}/task")).into_iter().flatten().flatten() {
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue };
        // "tid (name with spaces) S fields…": the name ends at the last ')'
        let (Some(open), Some(close)) = (stat.find('('), stat.rfind(')')) else { continue };
        let name = stat[open + 1..close].to_string();
        let fields: Vec<&str> = stat[close + 2..].split_whitespace().collect();
        // utime and stime are fields 14 and 15 of the line, 12 and 13 after the name
        let (Some(u), Some(s)) = (fields.get(11), fields.get(12)) else { continue };
        let seconds = (u.parse::<f64>().unwrap_or(0.0) + s.parse::<f64>().unwrap_or(0.0)) / ticks;
        *out.entry(name).or_insert(0.0) += seconds;
    }
    out
}

/// Difference of two [`cpu_by_thread`] samples.
fn delta(before: &BTreeMap<String, f64>, after: &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
    after.iter().map(|(k, v)| (k.clone(), v - before.get(k).copied().unwrap_or(0.0))).collect()
}

/// Prints one phase: CPU per thread and the `status dir` times.
fn report(phase: &str, cpu: &BTreeMap<String, f64>, mut times: Vec<Duration>) {
    times.sort();
    let total: f64 = cpu.values().sum();
    eprintln!("\n== {phase}: Director CPU {total:.2} s in {:.0} s ({:.1} % of one core)", PHASE.as_secs_f64(), total / PHASE.as_secs_f64() * 100.0);
    let mut threads: Vec<_> = cpu.iter().filter(|(_, v)| **v >= 0.01).collect();
    threads.sort_by(|a, b| b.1.total_cmp(a.1));
    for (name, seconds) in threads {
        eprintln!("   {seconds:6.2} s  {name}");
    }
    if let (Some(first), Some(last)) = (times.first(), times.last()) {
        eprintln!("   status dir: min {first:?}, median {:?}, max {last:?} ({} calls)", times[times.len() / 2], times.len());
    }
}

/// Calls `status dir` `STATUS_CALLS` times over `PHASE` and returns the times.
fn status_times(rig: &Rig) -> Vec<Duration> {
    command_times(rig, "status dir")
}

/// Calls `command` `STATUS_CALLS` times over `PHASE` and returns the times.
fn command_times(rig: &Rig, command: &str) -> Vec<Duration> {
    let mut times = Vec::new();
    let end = Instant::now() + PHASE;
    let gap = PHASE / STATUS_CALLS as u32;
    while Instant::now() < end {
        let start = Instant::now();
        rig.command(command).unwrap_or_else(|e| panic!("{e}"));
        times.push(start.elapsed());
        std::thread::sleep(gap.saturating_sub(start.elapsed()));
    }
    times
}

/// The three phases.
#[test]
#[ignore = "measurement, needs bareos-sd and initdb: run with --ignored --nocapture"]
fn director_cost_idle_running_and_queued() {
    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let rig = Rig::start(RigOptions::new(39491)).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));
    let pid = rig.pid_of("odir").expect("odir runs");
    std::thread::sleep(Duration::from_secs(3));

    let before = cpu_by_thread(pid);
    let times = status_times(&rig);
    report("idle", &delta(&before, &cpu_by_thread(pid)), times);

    // A command that asks the job manager for nothing: the cost of the
    // console connection alone.
    let before = cpu_by_thread(pid);
    let times = command_times(&rig, "version");
    report("idle, `version` (connection cost only)", &delta(&before, &cpu_by_thread(pid)), times);

    let big = rig.run_job("BigBackup").unwrap_or_else(|e| panic!("{e}"));
    rig.wait_running(big, Duration::from_secs(60)).unwrap_or_else(|e| panic!("{e}"));
    std::thread::sleep(Duration::from_secs(2));
    let before = cpu_by_thread(pid);
    let times = status_times(&rig);
    report("1 job running", &delta(&before, &cpu_by_thread(pid)), times);

    // The same without anything that asks the FD: separates the lock of the
    // JobManager from the progress query of `status dir` (#6).
    let before = cpu_by_thread(pid);
    let times = command_times(&rig, "version");
    report("1 job running, `version`", &delta(&before, &cpu_by_thread(pid)), times);

    // The FD's own answer time: `status client=` asks the same FD with the
    // same command as the progress query of `status dir` (S-54).
    let before = cpu_by_thread(pid);
    let times = command_times(&rig, "status client=vfd");
    report("1 job running, `status client=vfd`", &delta(&before, &cpu_by_thread(pid)), times);

    let queued = [rig.run_job("VBackup").ok(), rig.run_job("VBackup").ok()];
    eprintln!("queued behind it: {queued:?}");
    std::thread::sleep(Duration::from_secs(2));
    let before = cpu_by_thread(pid);
    let times = status_times(&rig);
    report("1 running + 2 queued", &delta(&before, &cpu_by_thread(pid)), times);

    let _ = rig.command(&format!("cancel jobid={big} yes"));
}
