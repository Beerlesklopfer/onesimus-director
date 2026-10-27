//! FD status command — returns daemon status information.
//!
//! Compatible with C++ bareos-fd status output format.

use crate::config::FdConfig;

/// Generate status output (same format as C++ FD).
pub fn fd_status(_config: &FdConfig) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let hostname = nix::unistd::gethostname()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string());
    let now = chrono::Local::now();

    let mut output = String::new();

    // Header — same format as C++ FD
    output.push_str(&format!(
        "{hostname}-fd Version: {version} ({}) {}\n",
        now.format("%d %B %Y"),
        std::env::consts::OS,
    ));
    output.push_str(&format!(
        "Daemon started {}. Jobs: run=0 running=0, Bareos Rust binary\n",
        now.format("%d-%b-%Y %H:%M"),
    ));

    // System info
    output.push_str(&format!(
        " Sizeof: boffset_t=8 size_t=8 debug=0 trace=0 bwlimit=0kB/s\n"
    ));

    output.push('\n');
    output.push_str("Running Jobs:\n");
    output.push_str(&running_jobs());
    output.push_str("====\n\n");
    output.push_str("Terminated Jobs:\n");
    output.push_str("====\n");

    output
}

/// 1234567 -> "1,234,567" (edit_uint64_with_commas).
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// ListRunningJobs() of the FD: the data transfers in progress, with the
/// counters the backup publishes (`sd::Progress`).
fn running_jobs() -> String {
    use std::sync::atomic::Ordering::Relaxed;
    let running = crate::sd::running();
    if running.is_empty() {
        return "No Jobs running.\n".to_string();
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let time = |t: u64| chrono::DateTime::from_timestamp(t as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%d-%b-%y %H:%M:%S").to_string()).unwrap_or_default();
    let mut out = String::new();
    for job in running {
        let p = &job.progress;
        let started = p.started.load(Relaxed);
        let bytes = p.bytes.load(Relaxed);
        let rate = bytes / now.saturating_sub(started).max(1);
        out.push_str(&format!("JobId {} Job {} is running.\n", p.job_id.load(Relaxed), job.job));
        out.push_str(&format!("    {} Job started: {}\n", p.kind.lock().unwrap_or_else(|e| e.into_inner()), time(started)));
        out.push_str(&format!("    Files={} Bytes={} Bytes/sec={} Errors={}\n",
            commas(p.files.load(Relaxed)), commas(bytes), commas(rate), p.errors.load(Relaxed)));
        out.push_str(&format!("    Files Examined={}\n", commas(p.files_examined.load(Relaxed))));
        let file = p.current_file.lock().unwrap_or_else(|e| e.into_inner());
        if !file.is_empty() {
            out.push_str(&format!("    Processing file: {file}\n"));
        }
        out.push_str(&format!("    Last progress update: {}\n", time(p.last_update.load(Relaxed))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_separators() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(24506), "24,506");
        assert_eq!(commas(4134111858), "4,134,111,858");
    }

    #[test]
    fn running_job_shows_live_counters() {
        let shared = crate::sd::register("Live.2026-10-27_01.00.00_01", "");
        shared.progress.start(7, "Full Backup");
        shared.progress.update(24506, 4134111858, 25000, 2);
        shared.progress.set_file("/srv/data/file.bin");
        let text = running_jobs();
        crate::sd::unregister("Live.2026-10-27_01.00.00_01");
        assert!(text.contains("JobId 7 Job Live.2026-10-27_01.00.00_01 is running.\n"), "{text}");
        assert!(text.contains("    Full Backup Job started: "), "{text}");
        assert!(text.contains("    Files=24,506 Bytes=4,134,111,858 Bytes/sec="), "{text}");
        assert!(text.contains(" Errors=2\n    Files Examined=25,000\n    Processing file: /srv/data/file.bin\n"), "{text}");
        assert!(text.contains("    Last progress update: "), "{text}");
    }
}
