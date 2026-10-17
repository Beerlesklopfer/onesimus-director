//! Watchdog — dir:watchdog thread (Onesimus).
//!
//! The original Director's watchdog thread (lib/watchdog.cc) runs timer
//! callbacks: JcrTimeoutCheck (lib/jcr.cc, socket timers) and
//! JobMonitorWatchdog (dird/job.cc, Max Wait/Run/Run Sched Time). Socket
//! timeouts are per connection here; the job time limits belong to the job
//! queue (job_manager.rs) and are not implemented yet (docs/STUBS.md).
//!
//! What this thread does, specific to Onesimus' process model:
//! - Orphaned children: job workers are forked processes that the job
//!   manager reaps with waitpid(pid, WNOHANG) while it tracks the job. A
//!   child that stays a zombie for a whole watchdog interval is no longer
//!   waited for by anyone (a worker started without a job manager, a job
//!   dropped from the job list before its process ended); the watchdog
//!   reaps exactly that pid. It never calls waitpid(-1): that would steal
//!   the exit status of workers and commands others still wait for.
//! - Stuck scheduler: the dir:scheduler thread beats at least once per
//!   minute (scheduler.rs); a heartbeat older than [`HEARTBEAT_TIMEOUT`]
//!   is logged as a warning (once per stall), and its recovery as info.
//!
//! The File and Storage daemons are not probed: the original Director only
//! contacts them for jobs and console commands, and periodic connections
//! would show up in their logs and take their connection slots.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tracing::{info, warn};

/// Watchdog check interval (seconds).
const WATCHDOG_INTERVAL: u64 = 30;

/// Maximum time without scheduler heartbeat before warning (seconds): two
/// scheduler wait intervals.
const HEARTBEAT_TIMEOUT: i64 = 120;

pub fn watchdog_loop(shutdown: &AtomicBool) {
    info!("watchdog started (interval={}s, timeout={}s)", WATCHDOG_INTERVAL, HEARTBEAT_TIMEOUT);
    let mut watchdog = Watchdog::default();
    let me = std::process::id() as i32;
    loop {
        // Sleep the interval in short steps to stop quickly on shutdown.
        for _ in 0..WATCHDOG_INTERVAL * 2 {
            if shutdown.load(Ordering::Relaxed) {
                info!("watchdog stopped");
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        watchdog.check_scheduler(crate::scheduler::STATE.last_heartbeat.load(Ordering::Relaxed), now_unix());
        for (pid, status) in watchdog.reap_orphans(&zombie_children(me), reap) {
            warn!("watchdog: reaped orphaned child process {pid} ({status})");
        }
    }
}

/// What the watchdog remembers between two checks.
#[derive(Default)]
struct Watchdog {
    /// Zombie children seen at the previous check.
    zombies: HashSet<i32>,
    /// The scheduler stall already reported.
    scheduler_stalled: bool,
}

impl Watchdog {
    /// Warns once when the scheduler heartbeat is older than the timeout
    /// (0: scheduler not running). Returns whether it is stalled.
    fn check_scheduler(&mut self, heartbeat: i64, now: i64) -> bool {
        let stalled = heartbeat != 0 && now - heartbeat > HEARTBEAT_TIMEOUT;
        if stalled && !self.scheduler_stalled {
            warn!("watchdog: scheduler thread shows no life for {} seconds", now - heartbeat);
        } else if !stalled && self.scheduler_stalled {
            info!("watchdog: scheduler thread is running again");
        }
        self.scheduler_stalled = stalled;
        stalled
    }

    /// Reaps the zombie children that were zombies at the previous check
    /// already; `zombies` are the current ones. Returns (pid, status) of
    /// each child reaped.
    fn reap_orphans(&mut self, zombies: &[i32], mut reap: impl FnMut(i32) -> Option<String>) -> Vec<(i32, String)> {
        let mut reaped = Vec::new();
        let mut still = HashSet::new();
        for &pid in zombies {
            if self.zombies.contains(&pid) {
                if let Some(status) = reap(pid) {
                    reaped.push((pid, status));
                    continue;
                }
            }
            still.insert(pid);
        }
        self.zombies = still;
        reaped
    }
}

/// waitpid(pid, WNOHANG) of one zombie child: its exit status text.
fn reap(pid: i32) -> Option<String> {
    use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
    match waitpid(nix::unistd::Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)) {
        Ok(WaitStatus::Exited(_, code)) => Some(format!("exit status {code}")),
        Ok(WaitStatus::Signaled(_, signal, _)) => Some(format!("killed by {signal}")),
        _ => None,
    }
}

/// The child processes of `parent` in state Z (from /proc/<pid>/stat).
fn zombie_children(parent: i32) -> Vec<i32> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries.flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse::<i32>().ok()))
        .filter(|pid| std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()
            .and_then(|stat| parse_stat(&stat)).is_some_and(|(state, ppid)| state == 'Z' && ppid == parent))
        .collect()
}

/// State and parent pid of a /proc/<pid>/stat line ("pid (comm) S ppid …").
fn parse_stat(stat: &str) -> Option<(char, i32)> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let ppid = fields.next()?.parse().ok()?;
    Some((state, ppid))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_line_with_spaces_and_parens_in_comm() {
        assert_eq!(parse_stat("4711 (bareos (dir) x) Z 42 4711 0"), Some(('Z', 42)));
        assert_eq!(parse_stat("1 (init) S 0 1 1"), Some(('S', 0)));
        assert_eq!(parse_stat("garbage"), None);
    }

    #[test]
    fn only_zombies_seen_twice_are_reaped() {
        let mut wd = Watchdog::default();
        let mut calls = Vec::new();
        // first sighting: nobody is reaped (the owner may still wait for it)
        assert!(wd.reap_orphans(&[10, 11], |pid| { calls.push(pid); Some("exit status 0".into()) }).is_empty());
        assert!(calls.is_empty());
        // 10 was reaped by its owner in the meantime, 11 is still a zombie
        let reaped = wd.reap_orphans(&[11, 12], |pid| { calls.push(pid); Some("exit status 1".into()) });
        assert_eq!(reaped, vec![(11, "exit status 1".to_string())]);
        assert_eq!(calls, vec![11]);
        // a zombie that could not be reaped is tried again
        assert!(wd.reap_orphans(&[12], |_| None).is_empty());
        assert_eq!(wd.reap_orphans(&[12], |_| Some("x".into())).len(), 1);
    }

    #[test]
    fn real_orphan_is_reaped() {
        // A child nobody waits for becomes a zombie; the watchdog reaps it.
        let child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        std::mem::forget(child);
        let me = std::process::id() as i32;
        let mut zombie = false;
        for _ in 0..200 {
            if zombie_children(me).contains(&pid) { zombie = true; break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(zombie);
        // only this test's child: other tests' children are not ours to reap
        let mine = || zombie_children(me).into_iter().filter(|p| *p == pid).collect::<Vec<_>>();
        let mut wd = Watchdog::default();
        wd.reap_orphans(&mine(), reap);
        let reaped = wd.reap_orphans(&mine(), reap);
        assert!(reaped.contains(&(pid, "exit status 0".to_string())));
        assert!(!zombie_children(me).contains(&pid));
    }

    #[test]
    fn scheduler_stall_is_reported_once_and_recovers() {
        let mut wd = Watchdog::default();
        assert!(!wd.check_scheduler(0, 10_000)); // not running
        assert!(!wd.check_scheduler(10_000 - 60, 10_000));
        assert!(wd.check_scheduler(10_000 - 121, 10_000));
        assert!(wd.scheduler_stalled);
        assert!(wd.check_scheduler(10_000 - 200, 10_079));
        assert!(!wd.check_scheduler(10_100, 10_101));
        assert!(!wd.scheduler_stalled);
    }
}
