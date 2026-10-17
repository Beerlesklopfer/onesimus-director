//! Version strings of the File Daemon (lib/version.cc kBareosVersionStrings).

/// kBareosVersionStrings.Full
pub const FULL: &str = env!("CARGO_PKG_VERSION");
/// kBareosVersionStrings.Date
pub const DATE: &str = "17 October 2026";
/// kBareosVersionStrings.ShortDate
pub const SHORT_DATE: &str = "17Oct26";

fn os_release(key: &str) -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
        .map(|v| v.trim_matches('"').to_string())
}

/// GetOsInfo(): the distribution name ("Debian GNU/Linux 13 (trixie)").
pub fn os_info() -> String {
    os_release("PRETTY_NAME").unwrap_or_else(|| std::env::consts::OS.to_string())
}

/// BAREOS_PLATFORM ("debian")
pub fn platform() -> String {
    os_release("ID").unwrap_or_else(|| std::env::consts::OS.to_string())
}
