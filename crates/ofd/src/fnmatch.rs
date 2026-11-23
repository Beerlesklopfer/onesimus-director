// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `fnmatch()` of Bareos (`core/src/lib/fnmatch.cc`, OpenBSD 1.6) in Rust.
//!
//! Used on Windows, where the C runtime has no `fnmatch`; on Unix ofd keeps
//! calling the libc (unchanged). Bareos itself links this BSD version on
//! every platform. On Windows `IsPathSeparator` also accepts `\`
//! (`include/baconfig.h`), so it does here.
//!
//! Semantics as in the C code: `char` is signed (x86), so a range compares
//! bytes ≥ 0x80 as negative numbers; `FNM_CASEFOLD` folds ASCII only
//! (`B_ISUPPER`); the recursion stops after 64 levels (`FNM_MAX_RECUR`)
//! and counts as no match.

#![cfg_attr(unix, allow(dead_code))]

/// `FNM_PATHNAME`: a slash must be matched by a slash.
pub const FNM_PATHNAME: i32 = 0x01;
/// `FNM_NOESCAPE`: backslash is an ordinary character.
pub const FNM_NOESCAPE: i32 = 0x02;
/// `FNM_PERIOD`: a leading period must be matched by a period.
pub const FNM_PERIOD: i32 = 0x04;
/// `FNM_LEADING_DIR`: ignore `/<tail>` after a match.
pub const FNM_LEADING_DIR: i32 = 0x08;
/// `FNM_CASEFOLD`: case-insensitive (ASCII).
pub const FNM_CASEFOLD: i32 = 0x10;

/// `FNM_MAX_RECUR`.
const MAX_RECUR: i32 = 64;

/// Result of `r_fnmatch`: 0 match, 1 no match, -1 recursion limit.
const MATCH: i32 = 0;
/// `FNM_NOMATCH`.
const NOMATCH: i32 = 1;

/// `fnmatch(pattern, string, flags) == 0`.
pub fn fnmatch(pattern: &[u8], string: &[u8], flags: i32) -> bool {
    r_fnmatch(pattern, 0, string, 0, flags, MAX_RECUR) == MATCH
}

/// `IsPathSeparator`.
fn is_sep(c: u8) -> bool {
    c == b'/' || (cfg!(windows) && c == b'\\')
}

/// The byte at `i`, `EOS` (0) past the end (C strings).
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// `FOLD(c)`.
fn fold(c: u8, flags: i32) -> u8 {
    if flags & FNM_CASEFOLD != 0 { c.to_ascii_lowercase() } else { c }
}

/// `r_fnmatch()`: `p`/`s` are the C pointers into `pattern`/`string`.
fn r_fnmatch(pattern: &[u8], mut p: usize, string: &[u8], mut s: usize, flags: i32, mut recur: i32) -> i32 {
    if recur <= 0 {
        return -1;
    }
    recur -= 1;
    let start = s;
    let pathname = flags & FNM_PATHNAME != 0;
    let leading_period =
        |s: usize| at(string, s) == b'.' && flags & FNM_PERIOD != 0 && (s == start || (pathname && is_sep(at(string, s - 1))));
    loop {
        let mut c = at(pattern, p);
        p += 1;
        match c {
            0 => {
                if flags & FNM_LEADING_DIR != 0 && is_sep(at(string, s)) {
                    return MATCH;
                }
                return if at(string, s) == 0 { MATCH } else { NOMATCH };
            }
            b'?' => {
                if at(string, s) == 0 || (is_sep(at(string, s)) && pathname) || leading_period(s) {
                    return NOMATCH;
                }
                s += 1;
                continue;
            }
            b'*' => {
                c = at(pattern, p);
                while c == b'*' {
                    p += 1;
                    c = at(pattern, p);
                }
                if leading_period(s) {
                    return NOMATCH;
                }
                if c == 0 {
                    if pathname {
                        let rest = &string[s.min(string.len())..];
                        return if flags & FNM_LEADING_DIR != 0 || !rest.contains(&b'/') { MATCH } else { NOMATCH };
                    }
                    return MATCH;
                } else if is_sep(c) && pathname {
                    match string[s.min(string.len())..].iter().position(|&b| b == b'/') {
                        Some(off) => s += off,
                        None => return NOMATCH,
                    }
                    continue;
                }
                while at(string, s) != 0 {
                    let test = at(string, s);
                    let e = r_fnmatch(pattern, p, string, s, flags & !FNM_PERIOD, recur);
                    if e != NOMATCH {
                        return e;
                    }
                    if test == b'/' && pathname {
                        break;
                    }
                    s += 1;
                }
                return NOMATCH;
            }
            b'[' => {
                if at(string, s) == 0 || (is_sep(at(string, s)) && pathname) || leading_period(s) {
                    return NOMATCH;
                }
                match rangematch(pattern, p, at(string, s), flags) {
                    Range::Error => {
                        // not a good range, treat as normal text
                        if fold(c, flags) != fold(at(string, s), flags) {
                            return NOMATCH;
                        }
                    }
                    Range::Match(np) => p = np,
                    Range::NoMatch => return NOMATCH,
                }
                s += 1;
                continue;
            }
            b'\\' if flags & FNM_NOESCAPE == 0 => {
                c = at(pattern, p);
                p += 1;
                if c == 0 {
                    c = b'\\';
                    p -= 1;
                }
            }
            _ => {}
        }
        if fold(c, flags) != fold(at(string, s), flags) {
            return NOMATCH;
        }
        s += 1;
    }
}

/// Result of `Rangematch()`.
enum Range {
    /// `RANGE_MATCH` with the pattern position after `]`.
    Match(usize),
    /// `RANGE_NOMATCH`.
    NoMatch,
    /// `RANGE_ERROR`: no closing `]`.
    Error,
}

/// `Rangematch()`: the bracket expression at `p` (after `[`) against `test`.
fn rangematch(pattern: &[u8], mut p: usize, test: u8, flags: i32) -> Range {
    let negate = matches!(at(pattern, p), b'!' | b'^');
    if negate {
        p += 1;
    }
    // signed char comparison as in C
    let test = fold(test, flags) as i8;
    let mut ok = false;
    let mut c = at(pattern, p);
    p += 1;
    loop {
        if c == b'\\' && flags & FNM_NOESCAPE == 0 {
            c = at(pattern, p);
            p += 1;
        }
        if c == 0 {
            return Range::Error;
        }
        if c == b'/' && flags & FNM_PATHNAME != 0 {
            return Range::NoMatch;
        }
        let lo = fold(c, flags) as i8;
        let c2 = at(pattern, p + 1);
        if at(pattern, p) == b'-' && c2 != 0 && c2 != b']' {
            p += 2;
            let mut c2 = c2;
            if c2 == b'\\' && flags & FNM_NOESCAPE == 0 {
                c2 = at(pattern, p);
                p += 1;
            }
            if c2 == 0 {
                return Range::Error;
            }
            let hi = fold(c2, flags) as i8;
            if lo <= test && test <= hi {
                ok = true;
            }
        } else if lo == test {
            ok = true;
        }
        c = at(pattern, p);
        p += 1;
        if c == b']' {
            break;
        }
    }
    if ok == negate { Range::NoMatch } else { Range::Match(p) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cases where glibc and the BSD code agree, checked against both.
    #[test]
    fn agrees_with_libc_on_common_patterns() {
        let cases: &[(&[u8], &[u8], i32)] = &[
            (b"*.txt", b"a.txt", 0),
            (b"*.txt", b"dir/a.txt", 0),
            (b"*.txt", b"dir/a.txt", FNM_PATHNAME),
            (b"*/*.txt", b"dir/a.txt", FNM_PATHNAME),
            (b"/home/*", b"/home/a/b", 0),
            (b"/home/*", b"/home/a/b", FNM_PATHNAME),
            (b"/home/*/", b"/home/a/", FNM_PATHNAME),
            (b"a?c", b"abc", 0),
            (b"a?c", b"a/c", FNM_PATHNAME),
            (b"[a-c]x", b"bx", 0),
            (b"[!a-c]x", b"bx", 0),
            (b"[]]x", b"]x", 0),
            (b"*.TXT", b"a.txt", FNM_CASEFOLD),
            (b"\\*", b"*", 0),
            (b"\\*", b"a", 0),
            (b".*", b".hidden", FNM_PERIOD),
            (b"*", b".hidden", FNM_PERIOD),
            (b"/a", b"/a/b", FNM_LEADING_DIR),
            (b"**x", b"abx", 0),
            (b"", b"", 0),
            (b"", b"a", 0),
        ];
        for &(p, s, f) in cases {
            #[cfg(target_os = "linux")]
            {
                let (cp, cs) = (std::ffi::CString::new(p).unwrap(), std::ffi::CString::new(s).unwrap());
                unsafe extern "C" {
                    #[link_name = "fnmatch"]
                    fn c_fnmatch(pattern: *const libc::c_char, string: *const libc::c_char, flags: libc::c_int) -> libc::c_int;
                }
                // glibc uses the same flag values for these five bits
                let libc = unsafe { c_fnmatch(cp.as_ptr(), cs.as_ptr(), f) } == 0;
                assert_eq!(fnmatch(p, s, f), libc, "{:?} {:?} {f:#x}", String::from_utf8_lossy(p), String::from_utf8_lossy(s));
            }
            #[cfg(not(target_os = "linux"))]
            let _ = fnmatch(p, s, f);
        }
    }

    /// Where Bareos' BSD code and glibc differ, the BSD result counts: an
    /// unterminated bracket is an ordinary `[` (RANGE_ERROR → `normal`).
    #[test]
    fn bsd_specifics() {
        assert!(fnmatch(b"[a-", b"[a-", 0));
        assert!(fnmatch(b"x[", b"x[", 0));
    }

    #[test]
    fn recursion_limit_is_no_match() {
        let pattern = b"*a".repeat(70);
        let string = b"a".repeat(200);
        assert!(!fnmatch(&pattern, &string, 0));
    }
}
