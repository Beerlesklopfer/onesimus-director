//! Build-time version strings (C++ lib/version.cc kBareosVersionStrings and
//! the configure-time BAREOS_PLATFORM / DISTVER), see src/version_strings.rs.
//! Dates honour SOURCE_DATE_EPOCH for reproducible Debian builds.

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-changed=/etc/os-release");

    // One timestamp for all date strings: Date "%d %B %Y", ShortDate
    // "%d%b%y", ProgDateTime "%Y-%m-%d %H:%M:%S", Year "%Y".
    let mut cmd = std::process::Command::new("date");
    cmd.env("LC_ALL", "C").arg("-u");
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        cmd.arg("-d").arg(format!("@{epoch}"));
    }
    let stamp = cmd.arg("+%d %B %Y|%d%b%y|%Y-%m-%d %H:%M:%S|%Y").output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    let mut parts = stamp.split('|');
    for key in ["ONESIMUS_BUILD_DATE", "ONESIMUS_BUILD_SHORT_DATE",
                "ONESIMUS_BUILD_PROG_DATE_TIME", "ONESIMUS_BUILD_YEAR"] {
        println!("cargo:rustc-env={key}={}", parts.next().unwrap_or(""));
    }

    // Bareos' distname detection at configure time: BAREOS_PLATFORM is the
    // os-release ID (e.g. "debian"), DISTVER its PRETTY_NAME.
    let os_release = std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .unwrap_or_default();
    let field = |key: &str| os_release.lines()
        .find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
        .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=ONESIMUS_BUILD_PLATFORM={}", field("ID"));
    println!("cargo:rustc-env=ONESIMUS_BUILD_DISTVER={}", field("PRETTY_NAME"));
}
