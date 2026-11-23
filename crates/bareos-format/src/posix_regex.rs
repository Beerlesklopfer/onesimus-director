// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! POSIX-Regex je Plattform: `regcomp`/`regexec` der libc auf Unix, die
//! Crate `regex` auf Windows.
//!
//! Zweck: [`crate::breg`] (`regexwhere`) und [`crate::bsr`] (`FileRegex`)
//! brauchen dieselben drei Aufrufe wie das C-Original: übersetzen,
//! prüfen, Teilausdrücke abfragen. Auf Unix ruft dieses Modul unverändert
//! die libc (glibc bzw. libSystem), wie Bareos über `<regex.h>`.
//!
//! Windows: Bareos linkt dort keine GNU-regex, sondern den POSIX-Aufsatz
//! von PCRE2 (`core/src/lib/bregex.h`, `HAVE_PCRE2_POSIX_H`;
//! `core/src/lib/CMakeLists.txt`: `PCRE2::POSIX` unter `HAVE_WIN32`). Die
//! Ausdrücke haben dort also Perl-Semantik: `REG_EXTENDED` wird
//! ignoriert, Alternativen gelten „erste passende“ statt „längste“,
//! `REG_NEWLINE` wird zu `PCRE2_MULTILINE`. Dem kommt die reine
//! Rust-Crate `regex` am nächsten (ebenfalls „erste passende“, Syntax aus
//! der Perl-Familie, Byte-Modus mit `(?-u)`). Abweichungen zu Bareos auf
//! Windows: keine Rückverweise (`\1` im Ausdruck) und kein Lookaround;
//! solche Ausdrücke lehnt `regex` beim Übersetzen ab (der Restore meldet
//! dann einen ungültigen Ausdruck wie bei jedem anderen Syntaxfehler).
//! Groß-/Kleinschreibung (`REG_ICASE`) wirkt nur auf ASCII, wie PCRE2 ohne
//! UTF-Modus mit den Standardtabellen.

/// Number of match registers of [`Regex::captures`] (`BREG_NREGS`).
pub const NREGS: usize = 11;

/// One match register: start and end offset, `None` if the group did not
/// take part in the match (`rm_so == -1`).
pub type Span = Option<(usize, usize)>;

/// Compile options (the `REG_*` flags the callers use).
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// `REG_ICASE`.
    pub icase: bool,
    /// `REG_NEWLINE`.
    pub newline: bool,
    /// `REG_NOSUB`: only [`Regex::is_match`] is used.
    pub nosub: bool,
}

#[cfg(unix)]
pub use unix::Regex;
#[cfg(windows)]
pub use windows::Regex;

/// libc `regcomp`/`regexec` (unchanged from the code before the split).
#[cfg(unix)]
mod unix {
    use super::{NREGS, Options, Span};
    use std::ffi::CString;

    /// A compiled POSIX extended regular expression (`REG_EXTENDED`).
    pub struct Regex {
        /// The compiled expression.
        preg: libc::regex_t,
    }

    impl Regex {
        /// `regcomp(…, REG_EXTENDED | …)`; the text of `regerror` on failure.
        pub fn new(pattern: &[u8], opts: Options) -> Result<Regex, String> {
            let c = CString::new(pattern).map_err(|e| e.to_string())?;
            let mut flags = libc::REG_EXTENDED;
            if opts.icase {
                flags |= libc::REG_ICASE;
            }
            if opts.newline {
                flags |= libc::REG_NEWLINE;
            }
            if opts.nosub {
                flags |= libc::REG_NOSUB;
            }
            // SAFETY: regcomp initializes `preg`; it is freed in Drop only on success.
            unsafe {
                let mut preg: libc::regex_t = std::mem::zeroed();
                let rc = libc::regcomp(&mut preg, c.as_ptr(), flags);
                if rc != 0 {
                    let mut buf = [0u8; 500];
                    libc::regerror(rc, &preg, buf.as_mut_ptr().cast(), buf.len());
                    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                    return Err(String::from_utf8_lossy(&buf[..end]).into_owned());
                }
                Ok(Regex { preg })
            }
        }

        /// `regexec(…) == 0`.
        pub fn is_match(&self, text: &[u8]) -> bool {
            let Ok(c) = CString::new(text) else { return false };
            // SAFETY: valid compiled expression and NUL-terminated input.
            unsafe { libc::regexec(&self.preg, c.as_ptr(), 0, std::ptr::null_mut(), 0) == 0 }
        }

        /// `regexec` with [`NREGS`] registers; `None` if there is no match
        /// (or the text contains a NUL byte, which C cannot pass).
        pub fn captures(&self, text: &[u8]) -> Option<[Span; NREGS]> {
            let c = CString::new(text).ok()?;
            let mut regs = [libc::regmatch_t { rm_so: -1, rm_eo: -1 }; NREGS];
            // SAFETY: valid compiled expression, NUL-terminated input, NREGS registers.
            if unsafe { libc::regexec(&self.preg, c.as_ptr(), NREGS, regs.as_mut_ptr(), 0) } != 0 {
                return None;
            }
            let mut out = [None; NREGS];
            for (o, r) in out.iter_mut().zip(regs) {
                if r.rm_so >= 0 && r.rm_eo >= 0 {
                    *o = Some((r.rm_so as usize, r.rm_eo as usize));
                }
            }
            Some(out)
        }
    }

    impl Drop for Regex {
        fn drop(&mut self) {
            // SAFETY: compiled by regcomp in `new`.
            unsafe { libc::regfree(&mut self.preg) };
        }
    }
}

/// The `regex` crate in byte mode, configured like PCRE2's `regcomp`.
#[cfg(windows)]
mod windows {
    use super::{NREGS, Options, Span};

    /// A compiled expression (`regex::bytes::Regex`).
    pub struct Regex {
        /// The compiled expression.
        re: regex::bytes::Regex,
    }

    impl Regex {
        /// Compiles `pattern` in byte mode (`(?-u)`); `REG_ICASE` →
        /// `(?i)`, `REG_NEWLINE` → `(?m)` (`PCRE2_MULTILINE`). The pattern
        /// must be UTF-8 (file names in a `regexwhere` arrive as UTF-8
        /// from the Director).
        pub fn new(pattern: &[u8], opts: Options) -> Result<Regex, String> {
            let pattern = std::str::from_utf8(pattern).map_err(|e| e.to_string())?;
            let re = regex::bytes::RegexBuilder::new(pattern)
                .unicode(false)
                .case_insensitive(opts.icase)
                .multi_line(opts.newline)
                .build()
                .map_err(|e| e.to_string())?;
            Ok(Regex { re })
        }

        /// Whether the expression matches somewhere in `text`.
        pub fn is_match(&self, text: &[u8]) -> bool {
            !text.contains(&0) && self.re.is_match(text)
        }

        /// The first match and its groups 1..[`NREGS`]-1.
        pub fn captures(&self, text: &[u8]) -> Option<[Span; NREGS]> {
            if text.contains(&0) {
                return None;
            }
            let caps = self.re.captures(text)?;
            let mut out = [None; NREGS];
            for (i, o) in out.iter_mut().enumerate() {
                *o = caps.get(i).map(|m| (m.start(), m.end()));
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_icase_and_classes() {
        let re = Regex::new(b"([a-z]+)/([0-9]+)$", Options { icase: true, newline: true, nosub: false }).unwrap();
        let caps = re.captures(b"x/ABC/123").unwrap();
        assert_eq!(caps[0], Some((2, 9)));
        assert_eq!(caps[1], Some((2, 5)));
        assert_eq!(caps[2], Some((6, 9)));
        assert_eq!(caps[3], None);
        let re = Regex::new(b"^[[:alpha:]]+\\.txt$", Options { nosub: true, ..Options::default() }).unwrap();
        assert!(re.is_match(b"abc.txt"));
        assert!(!re.is_match(b"a1.txt"));
        assert!(Regex::new(b"(", Options::default()).is_err());
    }

    #[test]
    fn non_utf8_text() {
        let re = Regex::new(b"^/a/.", Options::default()).unwrap();
        assert!(re.is_match(b"/a/\xff"));
    }
}
