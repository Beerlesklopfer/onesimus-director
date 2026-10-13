//! `onesimus-dird -k`: stop a running Director.
//!
//! The PID comes from `-p <pidfile>` if given, otherwise from /proc
//! (processes named onesimus-dird, our own excluded; with several candidates
//! only those started with the same `-c` config). SIGTERM first for a clean
//! shutdown, SIGKILL if the process is still alive after the grace period.

use std::path::Path;
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

const GRACE: Duration = Duration::from_secs(10);

/// Returns the process exit code: 0 stopped, 1 nothing found/error.
pub fn kill_running(pid_file: Option<&Path>, config: &Path) -> i32 {
    let pids = match pid_file {
        Some(f) => match std::fs::read_to_string(f) {
            Ok(s) => match s.trim().parse::<i32>() {
                Ok(p) => vec![p],
                Err(_) => {
                    eprintln!("invalid PID in {}", f.display());
                    return 1;
                }
            },
            Err(e) => {
                eprintln!("cannot read PID file {}: {e}", f.display());
                return 1;
            }
        },
        None => find_directors(config),
    };

    if pids.is_empty() {
        eprintln!("no running onesimus-dird found");
        return 1;
    }

    let mut rc = 0;
    for pid in pids {
        if !stop(pid) {
            rc = 1;
        }
    }
    rc
}

fn stop(pid: i32) -> bool {
    let p = Pid::from_raw(pid);
    match kill(p, Signal::SIGTERM) {
        Ok(()) => {}
        Err(nix::errno::Errno::ESRCH) => {
            eprintln!("process {pid} is not running");
            return false;
        }
        Err(nix::errno::Errno::EPERM) => {
            eprintln!("no permission to stop process {pid} (run as root or as its user)");
            return false;
        }
        Err(e) => {
            eprintln!("cannot signal process {pid}: {e}");
            return false;
        }
    }
    println!("sent SIGTERM to onesimus-dird (pid {pid}), waiting for shutdown ...");

    let start = Instant::now();
    while start.elapsed() < GRACE {
        if !alive(pid) {
            println!("onesimus-dird (pid {pid}) stopped");
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    eprintln!("pid {pid} still running after {}s, sending SIGKILL", GRACE.as_secs());
    let _ = kill(p, Signal::SIGKILL);
    std::thread::sleep(Duration::from_millis(200));
    if alive(pid) {
        eprintln!("pid {pid} did not terminate");
        false
    } else {
        println!("onesimus-dird (pid {pid}) killed");
        true
    }
}

/// Process exists and is not a zombie.
fn alive(pid: i32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => !stat.rsplit(')').next().is_some_and(|rest| rest.trim_start().starts_with('Z')),
        Err(_) => false,
    }
}

/// Running onesimus-dird processes except ourselves. With more than one
/// candidate, keep only those using the same config path.
fn find_directors(config: &Path) -> Vec<i32> {
    let me = std::process::id() as i32;
    let mut found: Vec<(i32, Vec<String>)> = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    for e in entries.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<i32>() else { continue };
        if pid == me {
            continue;
        }
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
        // comm is truncated to 15 characters: "onesimus-dird" fits
        if comm.trim() != "onesimus-dird" {
            continue;
        }
        let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let args: Vec<String> = cmdline.split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect();
        if args.iter().any(|a| a == "-k" || a == "--kill") {
            continue;
        }
        found.push((pid, args));
    }

    if found.len() > 1 {
        let want = config.to_string_lossy();
        found.retain(|(_, args)| config_arg(args).as_deref().unwrap_or("/etc/bareos/bareos-dir.d") == want);
    }
    found.into_iter().map(|(pid, _)| pid).collect()
}

/// Value of -c/--config in a command line.
fn config_arg(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-c" || a == "--config" {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix("--config=") {
            return Some(v.to_string());
        }
        if let Some(v) = a.strip_prefix("-c").filter(|v| !v.is_empty()) {
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn config_arg_forms() {
        assert_eq!(config_arg(&v(&["onesimus-dird", "-c", "/a", "-f"])).as_deref(), Some("/a"));
        assert_eq!(config_arg(&v(&["onesimus-dird", "--config=/b"])).as_deref(), Some("/b"));
        assert_eq!(config_arg(&v(&["onesimus-dird", "-c/c"])).as_deref(), Some("/c"));
        assert_eq!(config_arg(&v(&["onesimus-dird", "-f"])), None);
    }

    #[test]
    fn own_process_is_alive_and_bogus_pid_is_not() {
        assert!(alive(std::process::id() as i32));
        assert!(!alive(i32::MAX));
    }
}
