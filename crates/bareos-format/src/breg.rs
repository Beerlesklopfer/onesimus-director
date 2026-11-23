// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Dateinamen beim Restore umschreiben: `regexwhere=!von!nach!optionen`.
//!
//! Zweck: Ein Restore kann Dateinamen per regulärem Ausdruck umschreiben
//! (`restore … regexwhere=`, `strip_prefix`, `add_prefix`, `add_suffix`).
//! Der Director baut den Ausdruck ([`build_where`]) und prüft ihn im
//! Dialog, der FD wendet ihn auf jeden Namen an ([`apply_bregexps`]).
//! Beide müssen dasselbe Ergebnis liefern, deshalb steht der Code hier
//! einmal. Gearbeitet wird auf Bytes, weil Dateinamen kein UTF-8 sein
//! müssen. Die Ausdrücke übersetzt POSIX `regcomp`/`regexec` aus der libc
//! wie im C-Original, auf Windows die Crate `regex` wie dort PCRE2
//! ([`crate::posix_regex`]).
//!
//! Datenfluss: Director (`restore`-Dialog) → `regexwhere=` im
//! Restore-Kommando → FD [`get_bregexps`] → [`apply_bregexps`] je Datei.
//!
//! Bareos-Gegenstück: `core/src/lib/breg.cc` (`NewBregexp`,
//! `ExtractRegexp`, `BareosRegex::replace`, `get_bregexps`,
//! `ApplyBregexps`, `bregexp_build_where`, `bregexp_escape_string`).
//!
//! Zusammengeführt (20.11.2026) aus `ofd/src/breg.rs` (Bytes) und
//! `odir/src/breg.rs` (Zeichenketten, dazu `build_where`); der Code war
//! bis auf diese Signaturen gleich.

use crate::posix_regex::{Options, Regex};

/// One compiled `BareosRegex`.
pub struct Bregexp {
    /// The compiled POSIX regular expression (`BREG_NREGS` registers).
    preg: Regex,
    /// Substitution with `$n`/`\n` back references.
    subst: Vec<u8>,
}

impl Bregexp {
    /// NewBregexp()/ExtractRegexp(): the regexp at the start of `motif` and
    /// the rest after its options (`eor`), None for a bad regexp.
    fn new(motif: &[u8]) -> Option<(Self, &[u8])> {
        let sep = *motif.first()?;
        if !b"!:;|,&%=~/#".contains(&sep) {
            return None;
        }
        let mut expr = Vec::new();
        let mut subst: Option<Vec<u8>> = None;
        let mut ok = false;
        // `i` follows the C pointer `search`: after `\<sep>` and `\\` it
        // stays on the copied character (`*dest++ = *++search`), which the
        // next round looks at again; the closing separator is left to the
        // options loop.
        let mut i = 1;
        let at = |i: usize| motif.get(i).copied().unwrap_or(0);
        while i < motif.len() && !ok {
            if at(i) == b'\\' && (at(i + 1) == sep || at(i + 1) == b'\\') {
                i += 1;
                subst.as_mut().unwrap_or(&mut expr).push(at(i));
            } else if at(i) == sep {
                if subst.is_some() {
                    ok = true;
                } else {
                    i += 1;
                    subst = Some(Vec::new());
                }
            } else {
                subst.as_mut().unwrap_or(&mut expr).push(at(i));
                i += 1;
            }
        }
        let subst = subst.filter(|_| ok)?;
        // REG_EXTENDED | REG_NEWLINE
        let mut options = Options { newline: true, ..Options::default() };
        let mut done = false;
        while i < motif.len() && !done {
            match motif[i] {
                b'i' => options.icase = true,
                b'g' => {}
                c if c == sep => {}
                _ => done = true,
            }
            i += 1;
        }
        let preg = Regex::new(&expr, options).ok()?;
        Some((Self { preg, subst }, &motif[i..]))
    }

    /// BareosRegex::replace(): the file name with the substitution applied,
    /// and whether it matched.
    fn replace(&self, fname: &[u8]) -> (Vec<u8>, bool) {
        let Some(regs) = self.preg.captures(fname) else { return (fname.to_vec(), false) };
        let Some((so, eo)) = regs[0] else { return (fname.to_vec(), false) };
        let mut result = fname[..so].to_vec();
        let mut p = 0;
        while p < self.subst.len() {
            let c = self.subst[p];
            let next = self.subst.get(p + 1).copied().unwrap_or(0);
            if (c == b'$' || c == b'\\') && next.is_ascii_digit() {
                if let Some((rso, reo)) = regs[usize::from(next - b'0')] {
                    result.extend_from_slice(&fname[rso..reo]);
                }
                p += 2;
            } else {
                result.push(c);
                p += 1;
            }
        }
        result.extend_from_slice(&fname[eo..]);
        (result, true)
    }
}

/// `get_bregexps()`: every regexp of a `where` list, `None` when there is none.
pub fn get_bregexps(regexwhere: &str) -> Option<Vec<Bregexp>> {
    let mut list = Vec::new();
    let mut rest = regexwhere.as_bytes();
    while let Some((reg, eor)) = Bregexp::new(rest) {
        list.push(reg);
        rest = eor;
    }
    (!list.is_empty()).then_some(list)
}

/// `ApplyBregexps()`: all regexps applied one after the other.
pub fn apply_bregexps(fname: &[u8], list: &[Bregexp]) -> Vec<u8> {
    let mut name = fname.to_vec();
    for reg in list {
        name = reg.replace(&name).0;
    }
    name
}

/// `bregexp_escape_string()` with the `!` separator.
fn escape(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, c| {
        if c == '!' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

/// `bregexp_build_where()`: the `regexwhere` list of `strip_prefix`,
/// `add_suffix` and `add_prefix` (in this order, as Bareos builds it).
pub fn build_where(strip_prefix: Option<&str>, add_prefix: Option<&str>, add_suffix: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(strip) = strip_prefix {
        parts.push(format!("!{}!!i", escape(strip)));
    }
    if let Some(suffix) = add_suffix {
        parts.push(format!("!([^/])$!$1{}!", escape(suffix)));
    }
    if let Some(prefix) = add_prefix {
        parts.push(format!("!^!{}!", escape(prefix)));
    }
    parts.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests of `ofd/src/breg.rs` and `odir/src/breg.rs`, joined.
    #[test]
    fn regexps_are_parsed_and_applied_like_breg_cc() {
        let list = get_bregexps("!a!b!").unwrap();
        assert_eq!(apply_bregexps(b"/tmp/a/a", &list), b"/tmp/b/a");
        assert!(get_bregexps("bad").is_none());
        assert!(get_bregexps("!a!b").is_none());
        let built = build_where(Some("/tmp"), Some("/new"), Some(".rst"));
        assert_eq!(built, "!/tmp!!i,!([^/])$!$1.rst!,!^!/new!");
        let list = get_bregexps(&built).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(apply_bregexps(b"/TMP/a/b", &list), b"/new/a/b.rst");
        // `\!` copies the separator and then ends the expression there
        // (breg.cc keeps `search` on the copied separator).
        let list = get_bregexps("!x\\!y!z!").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(apply_bregexps(b"ax!b", &list), b"ayb");
        let list = get_bregexps("!(a)(b)!$2\\1!").unwrap();
        assert_eq!(apply_bregexps(b"xaby", &list), b"xbay");
    }

    #[test]
    fn non_utf8_names_pass_through() {
        let list = get_bregexps("!^/a!/b!").unwrap();
        assert_eq!(apply_bregexps(b"/a/\xff\xfe", &list), b"/b/\xff\xfe");
    }

    #[test]
    fn build_where_escapes_separator_and_backslash() {
        assert_eq!(build_where(Some("a!b\\c"), None, None), "!a\\!b\\\\c!!i");
        assert_eq!(build_where(None, None, None), "");
    }
}
