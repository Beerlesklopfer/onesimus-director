//! Live progress of running jobs: the Director asks the File daemon's
//! `status` (ListRunningJobs of the FD) and reads the counters of each job
//! that moves data (issue #6).

use std::collections::HashMap;

use bareos_config::resources::ClientResource;

/// What the FD reports for one running job.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FdProgress {
    pub files: u64,
    pub bytes: u64,
    pub bytes_per_sec: u64,
    pub errors: u64,
    pub files_examined: u64,
    pub current_file: String,
    /// "26-Oct-26 21:25:01" as the FD prints it; empty when it does not say.
    pub last_update: String,
    /// Seconds since the FD last published counters; `None` when it does not say.
    pub seconds_since_progress: Option<u64>,
}

fn number(text: &str) -> u64 {
    text.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)
}

/// `Key=value` of a counters line ("Files=1,234 Bytes=5 Bytes/sec=6 Errors=0").
fn field(line: &str, key: &str) -> Option<u64> {
    let rest = line.split(key).nth(1)?;
    Some(number(rest.split_whitespace().next()?))
}

/// The "Running Jobs:" part of an FD status text, by JobId.
pub fn parse(status: &str) -> HashMap<u32, FdProgress> {
    let mut jobs = HashMap::new();
    let mut current: Option<u32> = None;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("JobId ") {
            current = rest.split_whitespace().next().and_then(|id| id.parse().ok());
            if let Some(id) = current {
                jobs.insert(id, FdProgress::default());
            }
            continue;
        }
        let Some(p) = current.and_then(|id| jobs.get_mut(&id)) else { continue };
        let line = line.trim();
        if line.starts_with("Files=") {
            p.files = field(line, "Files=").unwrap_or(0);
            p.bytes = field(line, "Bytes=").unwrap_or(0);
            p.bytes_per_sec = field(line, "Bytes/sec=").unwrap_or(0);
            p.errors = field(line, "Errors=").unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("Files Examined=") {
            p.files_examined = number(v);
        } else if let Some(v) = line.strip_prefix("Processing file: ") {
            p.current_file = v.to_string();
        } else if let Some(v) = line.strip_prefix("Seconds since last progress: ") {
            p.seconds_since_progress = Some(number(v));
        } else if let Some(v) = line.strip_prefix("Last progress update: ") {
            p.last_update = v.to_string();
        } else if line.is_empty() || line.starts_with("====") {
            current = None;
        }
    }
    jobs
}

/// Asks the FD of `client` for its running jobs; empty when it cannot be
/// reached (the job list stays usable).
pub fn query(client: &ClientResource, director: &str) -> HashMap<u32, FdProgress> {
    if !client.enabled || client.protocol.is_ndmp() {
        return HashMap::new();
    }
    let (_, fd) = crate::fd_protocol::connect_fd_console(
        &client.address, client.port, &client.password, director, &client.name,
        (client.tls_enable, client.tls_require));
    match fd.and_then(|fd| crate::fd_protocol::fd_query_commands(fd, &["status".to_string()])) {
        Ok(text) => parse(&text),
        Err(_) => HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "x-fd Version: 1\n\nRunning Jobs:\n\
JobId 862 Job forgejo.2026-10-26_20.21.09_09 is running.\n\
    Full Backup Job started: 26-Oct-26 20:21:09\n\
    Files=24,506 Bytes=4,134,111,858 Bytes/sec=17,000,000 Errors=1\n\
    Files Examined=25,010\n\
    Processing file: /srv/forgejo/data/a b.bin\n\
    Last progress update: 26-Oct-26 20:25:01\n\
    Seconds since last progress: 4\n\
JobId 863 Job other.2026-10-26_20.21.10_10 is running.\n\
    Incremental Backup Job started: 26-Oct-26 20:21:10\n\
    Files=3 Bytes=40 Bytes/sec=1 Errors=0\n\
====\n\nTerminated Jobs:\n";

    #[test]
    fn counters_of_each_running_job() {
        let jobs = parse(STATUS);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[&862], FdProgress {
            files: 24506, bytes: 4134111858, bytes_per_sec: 17000000, errors: 1, files_examined: 25010,
            current_file: "/srv/forgejo/data/a b.bin".into(), last_update: "26-Oct-26 20:25:01".into(),
            seconds_since_progress: Some(4) });
        assert_eq!((jobs[&863].files, jobs[&863].bytes), (3, 40));
    }

    #[test]
    fn no_running_jobs_gives_nothing() {
        assert!(parse("Running Jobs:\nNo Jobs running.\n====\n").is_empty());
    }
}
