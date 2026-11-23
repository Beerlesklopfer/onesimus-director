// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Diagnostics of slapcat/slapadd without their words (S-7.2).
//!
//! An slapadd error quotes the LDIF it failed on (`could not add entry
//! dn="…"`, `value #0 invalid per syntax`, sometimes the value itself), and
//! LDIF holds userPassword hashes, olcRootPW and credentials=. So no line
//! of a tool's stderr is passed on. The diagnostic is built from fixed
//! texts only: the exit status, the first LDIF line number (`line=N`) and
//! the known error classes found.

/// Known fragments of slapcat/slapadd/slapd messages and the fixed text
/// reported for each.
const KNOWN: &[(&str, &str)] = &[
    ("could not add entry", "an entry could not be added"),
    ("invalid path", "a configured path is invalid"),
    ("No such file or directory", "a file or directory is missing"),
    ("Permission denied", "permission denied"),
    ("doesn't support necessary operations", "the database has no local storage"),
    ("out of range", "database number out of range"),
    ("could not open database", "the database could not be opened"),
    ("could not open config", "the configuration could not be opened"),
    ("already exists", "an entry already exists"),
    ("Already exists", "an entry already exists"),
    ("parent does not exist", "a parent entry is missing"),
    ("invalid per syntax", "a value violates its attribute syntax"),
    ("AttributeDescription", "an attribute type is not defined"),
    ("attribute type undefined", "an attribute type is not defined"),
    ("unrecognized objectClass", "an object class is not defined"),
    ("object class violation", "an object class rule is violated"),
    ("no structuralObjectClass", "an entry has no structural object class"),
    ("Invalid DN", "an invalid DN"),
    ("unknown database type", "an unknown database type"),
    ("MDB_MAP_FULL", "the database is full (maxsize)"),
    ("is not empty", "the target is not empty"),
    ("bad configuration", "the configuration could not be read"),
    ("config error", "the configuration could not be read"),
    ("str2entry", "an entry could not be parsed"),
];

/// `line=N` or `line N` (slapadd), the first number after it.
fn line_number(text: &str) -> Option<u64> {
    for marker in ["line=", "line "] {
        let mut rest = text;
        while let Some(i) = rest.find(marker) {
            let after = &rest[i + marker.len()..];
            let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(n) = digits.parse() {
                return Some(n);
            }
            rest = after;
        }
    }
    None
}

/// The exit status as text.
pub fn status_text(status: Option<std::process::ExitStatus>) -> String {
    use std::os::unix::process::ExitStatusExt;
    match status {
        Some(s) if s.success() => "success".into(),
        Some(s) => match (s.code(), s.signal()) {
            (Some(code), _) => format!("exit code {code}"),
            (None, Some(sig)) => format!("signal {sig}"),
            _ => "unknown status".into(),
        },
        None => "unknown status".into(),
    }
}

/// The filtered diagnostic of `tool` for its stderr bytes.
pub fn diagnostic(tool: &str, status: Option<std::process::ExitStatus>, stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let mut out = format!("{tool}: {}", status_text(status));
    if let Some(n) = line_number(&text) {
        out.push_str(&format!("; first error at LDIF line {n}"));
    }
    let mut classes: Vec<&str> = Vec::new();
    for (needle, class) in KNOWN {
        if text.contains(needle) && !classes.contains(class) {
            classes.push(class);
        }
    }
    if !classes.is_empty() {
        out.push_str("; ");
        out.push_str(&classes.join(", "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn no_word_of_the_tool_output_is_passed_on() {
        let stderr = b"str2entry: invalid value for attributeType userPassword #0 (syntax 1.3.6.1.4.1.1466.115.121.1.40)\n\
slapadd: could not add entry dn=\"uid=anna,ou=people,dc=example,dc=org\" (line=17): olcRootPW: {SSHA}geheim credentials=admin123\n";
        let d = diagnostic("slapadd", Some(std::process::ExitStatus::from_raw(1 << 8)), stderr);
        assert_eq!(d, "slapadd: exit code 1; first error at LDIF line 17; an entry could not be added, an entry could not be parsed");
        for secret in ["geheim", "admin123", "anna", "userPassword", "SSHA", "dc=example"] {
            assert!(!d.contains(secret), "{secret} leaked: {d}");
        }
        assert_eq!(diagnostic("slapcat", Some(std::process::ExitStatus::from_raw(0)), b""), "slapcat: success");
        assert_eq!(diagnostic("slapcat", Some(std::process::ExitStatus::from_raw(9)), b"x"), "slapcat: signal 9");
        let d = diagnostic("slapadd", None, b"olcDbDirectory: value #0: invalid path: No such file or directory\n");
        assert!(d.contains("a configured path is invalid") && d.contains("missing") && !d.contains("olcDbDirectory"), "{d}");
    }
}
