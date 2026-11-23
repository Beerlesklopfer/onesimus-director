// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Masking of secrets in text odir passes on: job messages (catalog
//! `Log`, journal, mail, console, gRPC push events) and the RunScript
//! commands `show` prints.
//!
//! Bareos logs every RunScript command line verbatim (the client sends
//! `shell command: run ClientBeforeJob "<command>"`), so a database
//! password given as `--password=…` in a Job's RunScript ends up in every
//! `list joblog` and in `show`. Existing configurations stay as they are
//! (backward compatible); odir masks the values instead. Deviation from
//! Bareos, see docs/wiki/betrieb-odir.md.
//!
//! Recognised (case-insensitive), the value replaced by `***`:
//! - `<name>=<value>` where `<name>` (leading dashes, `_`, `-`, `.`
//!   allowed) ends in `password`, `passwd`, `pwd`, `passphrase`, `secret`,
//!   `token` or `apikey`/`api_key`/`api-key`: `--password=x`,
//!   `PGPASSWORD=x`, `MYSQL_PWD=x`, `--db-password=x`, `client_secret=x`;
//! - `--<name> <value>` (long option, value as the next word not starting
//!   with `-`) with such a name: `--password x`;
//! - credentials in URLs: `scheme://user:<value>@host`.
//!
//! A value runs to the next blank or double quote (in a plugin option
//! list, `…:password=x:host=h`, also to the next `:`); a value starting
//! with a quote runs to the matching quote (the quotes stay).

use std::borrow::Cow;

/// Name endings that mark a secret (lower case, separators removed).
const SECRET_SUFFIXES: &[&str] = &["password", "passwd", "pwd", "passphrase", "secret", "token", "apikey"];

/// The replacement of a masked value.
pub const MASK: &str = "***";

/// Whether `name` (an option or variable name, any case, with dashes,
/// underscores or dots) names a secret.
fn is_secret_name(name: &str) -> bool {
    let norm: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
    !norm.is_empty() && SECRET_SUFFIXES.iter().any(|s| norm.ends_with(s))
}

/// Characters of an option or variable name.
fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.')
}

/// End (exclusive) of the value starting at `start`: to the matching quote
/// when it starts with one (the closing quote excluded), else to the next
/// blank or double quote.
fn value_end(b: &[u8], start: usize, colon_list: bool) -> (usize, usize) {
    match b.get(start) {
        Some(&q) if q == b'\'' || q == b'"' => {
            let inner = start + 1;
            let end = b[inner..].iter().position(|&c| c == q).map_or(b.len(), |p| inner + p);
            (inner, end)
        }
        _ => {
            let end = b[start..].iter().position(|&c| c.is_ascii_whitespace() || c == b'"' || (colon_list && c == b':'))
                .map_or(b.len(), |p| start + p);
            (start, end)
        }
    }
}

/// `text` with every recognised secret value replaced by [`MASK`];
/// borrowed when nothing was found.
pub fn mask(text: &str) -> Cow<'_, str> {
    let b = text.as_bytes();
    // (start, end) byte ranges of values to mask, ascending, disjoint.
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        // URL credentials: "://user:" … "@"
        if b[i..].starts_with(b"://") {
            let auth_start = i + 3;
            let auth_end = b[auth_start..].iter()
                .position(|&c| c.is_ascii_whitespace() || matches!(c, b'/' | b'"' | b'\''))
                .map_or(b.len(), |p| auth_start + p);
            if let Some(at) = b[auth_start..auth_end].iter().rposition(|&c| c == b'@') {
                let at = auth_start + at;
                if let Some(colon) = b[auth_start..at].iter().position(|&c| c == b':') {
                    let v = auth_start + colon + 1;
                    if v < at {
                        ranges.push((v, at));
                    }
                }
            }
            i = auth_end;
            continue;
        }
        if !is_name_char(b[i]) || (i > 0 && is_name_char(b[i - 1])) {
            i += 1;
            continue;
        }
        // A name starts at i.
        let name_end = b[i..].iter().position(|&c| !is_name_char(c)).map_or(b.len(), |p| i + p);
        let name = &text[i..name_end];
        // Plugin option lists ("python:…:password=x:host=h"): the value ends at ':'.
        let colon_list = i > 0 && b[i - 1] == b':';
        if is_secret_name(name) {
            if b.get(name_end) == Some(&b'=') {
                let (s, e) = value_end(b, name_end + 1, colon_list);
                if e > s {
                    ranges.push((s, e));
                }
                i = e;
                continue;
            }
            if name.starts_with("--") && b.get(name_end).is_some_and(|c| *c == b' ' || *c == b'\t') {
                let start = name_end + b[name_end..].iter().take_while(|c| **c == b' ' || **c == b'\t').count();
                if b.get(start).is_some_and(|c| *c != b'-' && *c != b'"') {
                    let (s, e) = value_end(b, start, false);
                    if e > s {
                        ranges.push((s, e));
                    }
                    i = e;
                    continue;
                }
            }
        }
        i = name_end;
    }
    if ranges.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (s, e) in ranges {
        out.push_str(&text[last..s]);
        out.push_str(MASK);
        last = e;
    }
    out.push_str(&text[last..]);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runscript_log_line_of_the_client_is_masked() {
        // As CLOUD-fd sent it (JobId 1150, 2026-11-19), with a made-up password.
        let line = "CLOUD-fd JobId 1150: shell command: run ClientBeforeJob \"/etc/bareos/scripts/dump-database.sh \
                    --database=nextcloud --user=www-data --password=s3cr3t\"\n";
        assert_eq!(mask(line), "CLOUD-fd JobId 1150: shell command: run ClientBeforeJob \"/etc/bareos/scripts/dump-database.sh \
                    --database=nextcloud --user=www-data --password=***\"\n");
    }

    #[test]
    fn special_characters_in_the_value_are_masked_to_the_end() {
        assert_eq!(mask(r#"dump-globals.sh --user=postgres --password=x\(:%9z.Q$ab_C"#),
                   "dump-globals.sh --user=postgres --password=***");
    }

    #[test]
    fn option_and_variable_forms() {
        assert_eq!(mask("pg_dump --password secret db"), "pg_dump --password *** db");
        assert_eq!(mask("PGPASSWORD=abc pg_dump"), "PGPASSWORD=*** pg_dump");
        assert_eq!(mask("MYSQL_PWD=abc mysqldump"), "MYSQL_PWD=*** mysqldump");
        assert_eq!(mask("x --db-password='a b c' y"), "x --db-password='***' y");
        assert_eq!(mask("curl -H token=abc https://u:pw@host/x"), "curl -H token=*** https://u:***@host/x");
        assert_eq!(mask("--Password=X"), "--Password=***");
    }

    #[test]
    fn plugin_option_lists_end_at_the_colon() {
        assert_eq!(mask("plugin=python:module_name=ldap:bind_password=geheim:host=h"),
                   "plugin=python:module_name=ldap:bind_password=***:host=h");
    }

    #[test]
    fn ordinary_text_is_untouched() {
        for text in ["Backup OK -- Password=", "--password -v", "passwordless login", "user=www-data --database=x",
                     "Bareos bareos-dir 25.1.2", "https://host/path", "--pass=x"] {
            assert!(matches!(mask(text), Cow::Borrowed(_)), "{text}");
        }
    }
}
