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
    // TODO: list actually running jobs
    output.push_str("No Jobs running.\n");
    output.push_str("====\n\n");
    output.push_str("Terminated Jobs:\n");
    output.push_str("====\n");

    output
}
