// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Reading the configuration of a slapd instance: the `slapcat -n 0` LDIF
//! of slapd.d, or slapd.conf with its include files. Only what the plugin
//! needs is taken: databases (number, backend, suffix, directory, overlays),
//! TLS file paths and the *kinds* of secrets present. Secret values are
//! never kept.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Database {
    pub number: i32,
    pub backend: String,
    pub suffix: String,
    pub directory: String,
    pub overlays: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Parsed {
    pub databases: Vec<Database>,
    pub tls_files: Vec<String>,
    pub secret_kinds: BTreeSet<&'static str>,
    /// slapd.conf: every file read, the main file first.
    pub files: Vec<PathBuf>,
}

/// TLS directives (slapd.conf) and their cn=config names.
const TLS: &[(&str, &str)] = &[
    ("tlscertificatefile", "olctlscertificatefile"),
    ("tlscertificatekeyfile", "olctlscertificatekeyfile"),
    ("tlscacertificatefile", "olctlscacertificatefile"),
    ("tlscacertificatepath", "olctlscacertificatepath"),
    ("tlsdhparamfile", "olctlsdhparamfile"),
];

/// The default database directory of Debian's slapd when none is given.
pub const DEFAULT_DIRECTORY: &str = "/var/lib/ldap";

/// Standard base64 (RFC 4648), as LDIF uses for `attr:: value`.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// LDIF entries as (attribute, value) lists, continuation lines joined,
/// `attr:: base64` decoded (lossy UTF-8), comments skipped.
pub fn ldif_entries(text: &str) -> Vec<Vec<(String, String)>> {
    let mut entries = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    let mut logical: Vec<String> = Vec::new();
    let flush_line = |logical: &mut Vec<String>, current: &mut Vec<(String, String)>| {
        for line in logical.drain(..) {
            if line.starts_with('#') {
                continue;
            }
            let Some((attr, rest)) = line.split_once(':') else { continue };
            let value = if let Some(b64) = rest.strip_prefix(':') {
                base64_decode(b64.trim()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
            } else if rest.starts_with('<') {
                String::new()
            } else {
                rest.trim_start().to_string()
            };
            current.push((attr.to_string(), value));
        }
    };
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() {
            flush_line(&mut logical, &mut current);
            if !current.is_empty() {
                entries.push(std::mem::take(&mut current));
            }
        } else if let Some(more) = line.strip_prefix(' ') {
            if let Some(last) = logical.last_mut() {
                last.push_str(more);
            }
        } else {
            logical.push(line.to_string());
        }
    }
    flush_line(&mut logical, &mut current);
    if !current.is_empty() {
        entries.push(current);
    }
    entries
}

/// `{n}name` -> (n, name).
fn indexed(value: &str) -> Option<(i32, String)> {
    let rest = value.strip_prefix('{')?;
    let (n, name) = rest.split_once('}')?;
    Some((n.parse().ok()?, name.to_string()))
}

fn secret_of(attr: &str, value: &str, kinds: &mut BTreeSet<&'static str>) {
    let has_credentials = value.to_ascii_lowercase().contains("credentials=");
    match attr {
        "olcrootpw" | "rootpw" => {
            kinds.insert("rootpw");
        }
        "olcsyncrepl" | "syncrepl" if has_credentials => {
            kinds.insert("syncrepl-credentials");
        }
        "olcdbidassertbind" | "idassert-bind" if has_credentials => {
            kinds.insert("idassert-credentials");
        }
        "olcdbaclbind" | "acl-bind" if has_credentials => {
            kinds.insert("aclbind-credentials");
        }
        "olcdbaclpasswd" | "acl-passwd" => {
            kinds.insert("acl-passwd");
        }
        _ => {}
    }
}

/// The databases, TLS files and secret kinds of a `slapcat -n 0` LDIF.
pub fn parse_cn_config(ldif: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut overlays: Vec<(i32, i32, String)> = Vec::new();
    for entry in ldif_entries(ldif) {
        let dn = entry.iter().find(|(a, _)| a.eq_ignore_ascii_case("dn")).map(|(_, v)| v.clone()).unwrap_or_default();
        let rdns: Vec<&str> = dn.split(',').collect();
        let get = |name: &str| entry.iter().find(|(a, _)| a.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
        for (attr, value) in &entry {
            secret_of(&attr.to_ascii_lowercase(), value, &mut parsed.secret_kinds);
        }
        if dn.eq_ignore_ascii_case("cn=config") {
            for (_, olc) in TLS {
                if let Some(v) = get(olc) {
                    parsed.tls_files.push(v);
                }
            }
            continue;
        }
        let rdn_value = |rdn: &str, key: &str| -> Option<String> {
            let (k, v) = rdn.split_once('=')?;
            k.eq_ignore_ascii_case(key).then(|| v.to_string())
        };
        match rdns.as_slice() {
            [db, config] if config.eq_ignore_ascii_case("cn=config") => {
                if let Some((number, backend)) = rdn_value(db, "olcDatabase").and_then(|v| indexed(&v)) {
                    parsed.databases.push(Database {
                        number,
                        backend: backend.to_ascii_lowercase(),
                        suffix: get("olcSuffix").unwrap_or_default(),
                        directory: get("olcDbDirectory").unwrap_or_default(),
                        overlays: Vec::new(),
                    });
                }
            }
            [overlay, db, config] if config.eq_ignore_ascii_case("cn=config") => {
                if let (Some((i, name)), Some((n, _))) =
                    (rdn_value(overlay, "olcOverlay").and_then(|v| indexed(&v)), rdn_value(db, "olcDatabase").and_then(|v| indexed(&v)))
                {
                    overlays.push((n, i, name));
                }
            }
            _ => {}
        }
    }
    overlays.sort();
    for (n, _, name) in overlays {
        if let Some(db) = parsed.databases.iter_mut().find(|d| d.number == n) {
            db.overlays.push(name);
        }
    }
    parsed
}

/// slapd.conf tokens: whitespace separated, `"…"` quoted with `\` escapes.
pub fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut token = String::new();
        if c == '"' {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => token.extend(chars.next()),
                    '"' => break,
                    c => token.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
        }
        out.push(token);
    }
    out
}

/// Logical lines of a slapd.conf file: comments and blank lines dropped,
/// a line starting with white space continues the previous one.
pub fn logical_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }
        if raw.starts_with([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(raw.trim());
                continue;
            }
        }
        out.push(raw.trim_end().to_string());
    }
    out
}

/// A path without `.`/`..` components, absolute.
pub fn is_plain_absolute(path: &Path) -> bool {
    path.is_absolute() && path.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

/// Parses slapd.conf `main` and its include files (recursively, at most
/// 64 files). `read` returns a file's text. Relative include paths are
/// taken relative to the directory of the main file.
pub fn parse_slapd_conf(main: &Path, read: &mut dyn FnMut(&Path) -> Result<String, String>) -> Result<Parsed, String> {
    let mut parsed = Parsed::default();
    let base = main.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut next_number = 1;
    let mut current: Option<usize> = None;
    let mut stack: Vec<(PathBuf, Vec<String>, usize)> = Vec::new();
    let text = read(main)?;
    parsed.files.push(main.to_path_buf());
    stack.push((main.to_path_buf(), logical_lines(&text), 0));
    while let Some((file, lines, at)) = stack.last_mut() {
        let Some(line) = lines.get(*at).cloned() else {
            stack.pop();
            continue;
        };
        *at += 1;
        let file = file.clone();
        let words = tokens(&line);
        let Some(keyword) = words.first().map(|w| w.to_ascii_lowercase()) else { continue };
        let arg = words.get(1).cloned().unwrap_or_default();
        secret_of(&keyword, &line, &mut parsed.secret_kinds);
        match keyword.as_str() {
            "include" => {
                let path = if Path::new(&arg).is_absolute() { PathBuf::from(&arg) } else { base.join(&arg) };
                if !is_plain_absolute(&path) {
                    return Err(format!("{}: include path is not plain", file.display()));
                }
                if parsed.files.contains(&path) {
                    continue;
                }
                if parsed.files.len() >= 64 || stack.len() >= 16 {
                    return Err("too many include files".into());
                }
                let text = read(&path)?;
                parsed.files.push(path.clone());
                stack.push((path, logical_lines(&text), 0));
            }
            "database" => {
                let backend = arg.to_ascii_lowercase();
                let number = match backend.as_str() {
                    "config" => 0,
                    "frontend" => -1,
                    _ => {
                        next_number += 1;
                        next_number - 1
                    }
                };
                parsed.databases.push(Database { number, backend, ..Default::default() });
                current = Some(parsed.databases.len() - 1);
            }
            "suffix" => {
                if let Some(db) = current.and_then(|i| parsed.databases.get_mut(i)) {
                    if db.suffix.is_empty() {
                        db.suffix = arg;
                    }
                }
            }
            "directory" => {
                if let Some(db) = current.and_then(|i| parsed.databases.get_mut(i)) {
                    db.directory = arg;
                }
            }
            "overlay" => {
                if let Some(db) = current.and_then(|i| parsed.databases.get_mut(i)) {
                    db.overlays.push(arg);
                }
            }
            k => {
                if let Some((_, _)) = TLS.iter().find(|(name, _)| *name == k) {
                    parsed.tls_files.push(arg);
                }
            }
        }
    }
    for db in &mut parsed.databases {
        if db.backend == "mdb" && db.directory.is_empty() {
            db.directory = DEFAULT_DIRECTORY.to_string();
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const CN_CONFIG: &str = "dn: cn=config\nobjectClass: olcGlobal\ncn: config\nolcArgsFile: /run/slapd/slapd.args\n\
olcPidFile: /run/slapd/slapd.pid\nolcTLSCertificateFile: /etc/ldap/tls/server.crt\n\
olcTLSCertificateKeyFile: /etc/ldap/tls/se\n rver.key\n\n\
dn: olcDatabase={-1}frontend,cn=config\nolcDatabase: {-1}frontend\n\n\
dn: olcDatabase={0}config,cn=config\nolcDatabase: {0}config\nolcRootPW:: e1NTSEF9c2VjcmV0aGFzaA==\n\n\
dn: olcDatabase={1}mdb,cn=config\nolcDatabase: {1}mdb\nolcDbDirectory: /var/lib/ldap\nolcSuffix: dc=example,dc=org\n\
olcRootPW: {SSHA}abc\n\n\
dn: olcOverlay={1}ppolicy,olcDatabase={1}mdb,cn=config\nolcOverlay: {1}ppolicy\n\n\
dn: olcOverlay={0}memberof,olcDatabase={1}mdb,cn=config\nolcOverlay: {0}memberof\n\n\
dn: olcDatabase={2}mdb,cn=config\nolcDatabase: {2}mdb\nolcDbDirectory: /var/lib/ldap/second\n\
olcSuffix:: bz1Nw7xsbGVyLGM9ZGU=\n\n\
dn: olcDatabase={3}ldap,cn=config\nolcSuffix: dc=example,dc=net\n\
olcDbIDAssertBind: bindmethod=simple binddn=\"cn=admin\" credentials=admin123 mode=none\n";

    #[test]
    fn cn_config_gives_databases_overlays_tls_and_secret_kinds_only() {
        let p = parse_cn_config(CN_CONFIG);
        let dbs: Vec<(i32, &str, &str, &str)> =
            p.databases.iter().map(|d| (d.number, d.backend.as_str(), d.suffix.as_str(), d.directory.as_str())).collect();
        assert_eq!(dbs, [(-1, "frontend", "", ""), (0, "config", "", ""), (1, "mdb", "dc=example,dc=org", "/var/lib/ldap"),
            (2, "mdb", "o=Müller,c=de", "/var/lib/ldap/second"), (3, "ldap", "dc=example,dc=net", "")]);
        assert_eq!(p.databases[2].overlays, ["memberof", "ppolicy"]);
        assert_eq!(p.tls_files, ["/etc/ldap/tls/server.crt", "/etc/ldap/tls/server.key"]);
        assert_eq!(p.secret_kinds.iter().copied().collect::<Vec<_>>(), ["idassert-credentials", "rootpw"]);
        let shown = format!("{p:?}");
        assert!(!shown.contains("admin123") && !shown.contains("SSHA"), "no secret value is kept: {shown}");
    }

    #[test]
    fn slapd_conf_with_includes_continuations_and_quotes() {
        let files: BTreeMap<&str, &str> = [
            ("/etc/ldap/slapd.conf", "# test\ninclude /etc/ldap/schema/core.schema\ninclude schema/local/x.schema\n\
pidfile /run/slapd/slapd.pid\nTLSCertificateKeyFile /etc/ldap/tls/server.key\n\
database config\naccess to * by * none\n\
database mdb\nsuffix \"dc=example,dc=com\"\nrootpw {SSHA}x\ndirectory /srv/ldap/main\noverlay memberof\n\
access to attrs=userPassword\n  by self write\n  by * none\n\
database monitor\n\
database mdb\nsuffix \"o=zweit,c=de\"\n\
database meta\nsuffix \"dc=remote,dc=test\"\nacl-passwd geheim\n"),
            ("/etc/ldap/schema/core.schema", "attributetype ( 1.2 NAME 'x' )\n"),
            ("/etc/ldap/schema/local/x.schema", "include /etc/ldap/schema/core.schema\nobjectclass ( 1.3 NAME 'y' )\n"),
        ].into();
        let mut read = |p: &Path| files.get(p.to_str().unwrap()).map(|s| s.to_string()).ok_or_else(|| format!("{} missing", p.display()));
        let p = parse_slapd_conf(Path::new("/etc/ldap/slapd.conf"), &mut read).unwrap();
        assert_eq!(p.files, [PathBuf::from("/etc/ldap/slapd.conf"), "/etc/ldap/schema/core.schema".into(),
            "/etc/ldap/schema/local/x.schema".into()]);
        let dbs: Vec<(i32, &str, &str, &str)> =
            p.databases.iter().map(|d| (d.number, d.backend.as_str(), d.suffix.as_str(), d.directory.as_str())).collect();
        assert_eq!(dbs, [(0, "config", "", ""), (1, "mdb", "dc=example,dc=com", "/srv/ldap/main"), (2, "monitor", "", ""),
            (3, "mdb", "o=zweit,c=de", DEFAULT_DIRECTORY), (4, "meta", "dc=remote,dc=test", "")]);
        assert_eq!(p.databases[1].overlays, ["memberof"]);
        assert_eq!(p.tls_files, ["/etc/ldap/tls/server.key"]);
        assert_eq!(p.secret_kinds.iter().copied().collect::<Vec<_>>(), ["acl-passwd", "rootpw"]);
        // A missing include is an error, not a silent gap.
        let mut none = |p: &Path| if p.ends_with("slapd.conf") { Ok("include /etc/ldap/gone.schema\n".into()) } else { Err("missing".to_string()) };
        assert!(parse_slapd_conf(Path::new("/etc/ldap/slapd.conf"), &mut none).is_err());
        let mut dots = |_: &Path| Ok("include ../../etc/shadow\n".to_string());
        assert!(parse_slapd_conf(Path::new("/etc/ldap/slapd.conf"), &mut dots).is_err());
    }

    #[test]
    fn helpers() {
        assert_eq!(base64_decode("e1NTSEF9").unwrap(), b"{SSHA}");
        assert_eq!(base64_decode("YQ==").unwrap(), b"a");
        assert!(base64_decode("a$b").is_none());
        assert_eq!(tokens(r#"suffix "dc=a, dc=b" x\y "q\"t""#), ["suffix", "dc=a, dc=b", r"x\y", "q\"t"]);
        assert_eq!(logical_lines("a b\n  c\n# x\n\n d\ne"), ["a b c d", "e"]);
        assert!(is_plain_absolute(Path::new("/etc/ldap")) && !is_plain_absolute(Path::new("/etc/../x"))
            && !is_plain_absolute(Path::new("etc")));
    }
}
