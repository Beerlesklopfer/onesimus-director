//! Build date for `version` (C++ kBareosVersionStrings.Date, e.g. "09 October 2026").
//! Honours SOURCE_DATE_EPOCH for reproducible Debian builds.

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let mut cmd = std::process::Command::new("date");
    cmd.env("LC_ALL", "C").arg("-u");
    if let Ok(epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        cmd.arg("-d").arg(format!("@{epoch}"));
    }
    let date = cmd.arg("+%d %B %Y").output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=ONESIMUS_BUILD_DATE={date}");
}
