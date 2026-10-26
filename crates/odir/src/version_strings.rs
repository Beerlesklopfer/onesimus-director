//! Central version strings, C++ lib/version.cc `kBareosVersionStrings` and
//! lib/osinfo_unix.cc `GetOsInfoString`.
//!
//! Every place the Director prints version information uses these values in
//! the Bareos format; only the version token itself is Onesimus' own
//! (workspace version `25.1.2+onesimus.<x>`). Dates come from build.rs.

use std::sync::OnceLock;

/// kBareosVersionStrings.Full (BAREOS_FULL_VERSION).
pub const FULL: &str = env!("CARGO_PKG_VERSION");
/// kBareosVersionStrings.Date, build date "%d %B %Y" (e.g. "09 October 2026").
pub const DATE: &str = env!("ONESIMUS_BUILD_DATE");
/// kBareosVersionStrings.ShortDate, "%d%b%y" (e.g. "09Oct26").
pub const SHORT_DATE: &str = env!("ONESIMUS_BUILD_SHORT_DATE");
/// kBareosVersionStrings.BinaryInfo. Own wording in the upstream position:
/// the status header appends " binary" ("..., Onesimus binary"), `status
/// subscriptions` prints "Binary info: Onesimus".
pub const BINARY_INFO: &str = "Onesimus";
/// kBareosVersionStrings.ServicesMessage, two lines without trailing newline
/// like upstream's "Bareos community build (UNSUPPORTED).\nGet professional
/// support from https://www.bareos.com".
pub const SERVICES_MESSAGE: &str = "Onesimus build.\nSupport: https://onesimus.io";
/// kBareosVersionStrings.JoblogMessage, ServicesMessage on one line
/// ("<line 1 without period>: <line 2>") as upstream.
pub const JOBLOG_MESSAGE: &str = "Onesimus build: Support: https://onesimus.io";
/// Configure-time BAREOS_PLATFORM (os-release ID of the build host, e.g. "debian").
pub const PLATFORM: &str = env!("ONESIMUS_BUILD_PLATFORM");
/// Configure-time DISTVER (os-release PRETTY_NAME of the build host).
pub const DISTVER: &str = env!("ONESIMUS_BUILD_DISTVER");

/// kBareosVersionStrings.GetOsInfo(): os-release PRETTY_NAME of the running
/// host, read once. Falls back to uname, then to "built on <DISTVER>".
pub fn os_info() -> &'static str {
    static INFO: OnceLock<String> = OnceLock::new();
    INFO.get_or_init(|| {
        let content = ["/etc/os-release", "/usr/lib/os-release", "/etc/initrd-release"]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok());
        let version = match content {
            Some(content) => parse_os_release(&content),
            None => uname_pretty(),
        };
        version.unwrap_or_else(|| format!("built on {DISTVER}"))
    })
}

/// C++ os_release::parse_file. Note the upstream quirk: VERSION is stored
/// into the NAME slot, so the "NAME VERSION" fallback never applies and a
/// file without PRETTY_NAME yields None ("built on DISTVER").
fn parse_os_release(content: &str) -> Option<String> {
    let mut name: Option<String> = None;
    let version: Option<String> = None;
    let mut pretty_name: Option<String> = None;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // split_first on the untrimmed line, like upstream
        let Some((key, value)) = line.split_once('=') else { continue };
        let place = match key {
            "NAME" | "VERSION" => &mut name,
            "PRETTY_NAME" => &mut pretty_name,
            _ => continue,
        };
        let real_value = parse_value(unquote(value));
        if real_value.is_empty() {
            continue;
        }
        *place = Some(real_value);
    }
    pretty_name.or_else(|| match (name, version) {
        (Some(n), Some(v)) => Some(format!("{n} {v}")),
        _ => None,
    })
}

/// C++ unquote: strip matching quotes, honouring backslash-escaped quotes.
fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    let Some(&quote) = bytes.first() else { return value };
    if quote != b'\'' && quote != b'"' {
        return value;
    }
    let mut current = 1;
    while let Some(off) = value[current..].find(quote as char) {
        let pos = current + off;
        let backslashes = bytes[..pos].iter().rev().take_while(|&&b| b == b'\\').count();
        if backslashes % 2 == 0 {
            return &value[1..pos];
        }
        current = pos + 1;
    }
    value
}

/// C++ parse_value: drop escaping backslashes.
fn parse_value(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                result.push(next);
                continue;
            }
        }
        result.push(c);
    }
    result
}

/// "<sysname> <release> (<version>)" from uname(2).
fn uname_pretty() -> Option<String> {
    let mut uts: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut uts) } != 0 {
        return None;
    }
    let field = |f: &[libc::c_char]| unsafe { std::ffi::CStr::from_ptr(f.as_ptr()) }
        .to_string_lossy().into_owned();
    Some(format!("{} {} ({})", field(&uts.sysname), field(&uts.release), field(&uts.version)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_is_workspace_version() {
        assert!(FULL.starts_with("25.1.2+onesimus."), "{FULL}");
    }

    #[test]
    fn dates_use_bareos_formats() {
        // "%d %B %Y", "%d%b%y"
        let date: Vec<&str> = DATE.split(' ').collect();
        assert_eq!(date.len(), 3, "{DATE}");
        assert_eq!(date[0].len(), 2);
        assert!(date[1].chars().all(|c| c.is_ascii_alphabetic()));
        assert_eq!(date[2].len(), 4);
        assert_eq!(SHORT_DATE.len(), 7, "{SHORT_DATE}");
        assert_eq!(&SHORT_DATE[..2], date[0]);
        assert_eq!(&SHORT_DATE[2..5], &date[1][..3]);
        assert_eq!(&SHORT_DATE[5..], &date[2][2..]);
    }

    #[test]
    fn own_binary_info_in_upstream_shape() {
        assert!(!BINARY_INFO.contains("Bareos"));
        assert_eq!(SERVICES_MESSAGE.lines().count(), 2);
        assert!(!SERVICES_MESSAGE.ends_with('\n'));
        let (first, second) = SERVICES_MESSAGE.split_once('\n').unwrap();
        assert_eq!(JOBLOG_MESSAGE, format!("{}: {second}", first.trim_end_matches('.')));
    }

    #[test]
    fn os_release_pretty_name() {
        let content = "PRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\nNAME=\"Debian GNU/Linux\"\n\
                       VERSION_ID=\"13\"\nVERSION=\"13 (trixie)\"\nID=debian\n";
        assert_eq!(parse_os_release(content).as_deref(), Some("Debian GNU/Linux 13 (trixie)"));
    }

    #[test]
    fn os_release_without_pretty_name_falls_back() {
        // upstream quirk: VERSION overwrites NAME, no "NAME VERSION" result
        assert_eq!(parse_os_release("NAME=Foo\nVERSION=1\n"), None);
        assert_eq!(parse_os_release("# PRETTY_NAME=x\n PRETTY_NAME=y\n"), None);
        assert_eq!(parse_os_release("PRETTY_NAME=\"\"\n"), None);
    }

    #[test]
    fn os_release_quoting() {
        assert_eq!(parse_os_release("PRETTY_NAME='A b'").as_deref(), Some("A b"));
        assert_eq!(parse_os_release(r#"PRETTY_NAME="a\"b""#).as_deref(), Some("a\"b"));
        assert_eq!(parse_os_release(r#"PRETTY_NAME=a\$b"#).as_deref(), Some("a$b"));
        assert_eq!(unquote("'abcd"), "'abcd");
        assert_eq!(unquote(r#""ab\"cd""#), r#"ab\"cd"#);
    }

    #[test]
    fn os_info_is_never_empty() {
        assert!(!os_info().is_empty());
    }
}
