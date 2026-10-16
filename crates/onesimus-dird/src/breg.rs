//! lib/breg.cc: the `!from!to!opts` file name manipulation of restores
//! (RegexWhere, strip_prefix/add_prefix/add_suffix) with POSIX regcomp/
//! regexec like the C original.

use std::ffi::CString;

/// BREG_NREGS (breg.h:44)
const NREGS: usize = 11;

/// One compiled BareosRegex.
pub struct Bregexp {
    preg: libc::regex_t,
    /// Substitution with `$n`/`\n` back references.
    subst: Vec<u8>,
}

impl Drop for Bregexp {
    fn drop(&mut self) {
        unsafe { libc::regfree(&mut self.preg) };
    }
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
        let mut options = libc::REG_EXTENDED | libc::REG_NEWLINE;
        let mut done = false;
        while i < motif.len() && !done {
            match motif[i] {
                b'i' => options |= libc::REG_ICASE,
                b'g' => {}
                c if c == sep => {}
                _ => done = true,
            }
            i += 1;
        }
        let pattern = CString::new(expr).ok()?;
        let mut preg: libc::regex_t = unsafe { std::mem::zeroed() };
        if unsafe { libc::regcomp(&mut preg, pattern.as_ptr(), options) } != 0 {
            return None;
        }
        Some((Self { preg, subst }, &motif[i..]))
    }

    /// BareosRegex::replace(): the file name with the substitution applied,
    /// and whether it matched.
    fn replace(&self, fname: &[u8]) -> (Vec<u8>, bool) {
        let Ok(cname) = CString::new(fname) else { return (fname.to_vec(), false) };
        let mut regs = [libc::regmatch_t { rm_so: -1, rm_eo: -1 }; NREGS];
        if unsafe { libc::regexec(&self.preg, cname.as_ptr(), NREGS, regs.as_mut_ptr(), 0) } != 0
            || regs[0].rm_so < 0
        {
            return (fname.to_vec(), false);
        }
        let mut result = fname[..regs[0].rm_so as usize].to_vec();
        let mut p = 0;
        while p < self.subst.len() {
            let c = self.subst[p];
            let next = self.subst.get(p + 1).copied().unwrap_or(0);
            if (c == b'$' || c == b'\\') && next.is_ascii_digit() {
                let reg = regs[usize::from(next - b'0')];
                if reg.rm_so >= 0 && reg.rm_eo >= 0 {
                    result.extend_from_slice(&fname[reg.rm_so as usize..reg.rm_eo as usize]);
                }
                p += 2;
            } else {
                result.push(c);
                p += 1;
            }
        }
        result.extend_from_slice(&fname[regs[0].rm_eo as usize..]);
        (result, true)
    }
}

/// get_bregexps(): every regexp of a `where` list, None when there is none.
pub fn get_bregexps(regexwhere: &str) -> Option<Vec<Bregexp>> {
    let mut list = Vec::new();
    let mut rest = regexwhere.as_bytes();
    while let Some((reg, eor)) = Bregexp::new(rest) {
        list.push(reg);
        rest = eor;
    }
    (!list.is_empty()).then_some(list)
}

/// ApplyBregexps(): all regexps applied one after the other.
pub fn apply_bregexps(fname: &str, list: &[Bregexp]) -> String {
    let mut name = fname.as_bytes().to_vec();
    for reg in list {
        name = reg.replace(&name).0;
    }
    String::from_utf8_lossy(&name).into_owned()
}

/// bregexp_escape_string() with the `!` separator.
fn escape(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, c| {
        if c == '!' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

/// bregexp_build_where().
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

    #[test]
    fn regexps_are_parsed_and_applied_like_breg_cc() {
        let list = get_bregexps("!a!b!").unwrap();
        assert_eq!(apply_bregexps("/tmp/a/a", &list), "/tmp/b/a");
        assert!(get_bregexps("bad").is_none());
        assert!(get_bregexps("!a!b").is_none());
        let built = build_where(Some("/tmp"), Some("/new"), Some(".rst"));
        assert_eq!(built, "!/tmp!!i,!([^/])$!$1.rst!,!^!/new!");
        let list = get_bregexps(&built).unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(apply_bregexps("/TMP/a/b", &list), "/new/a/b.rst");
        // `\!` copies the separator and then ends the expression there
        // (breg.cc keeps `search` on the copied separator).
        let list = get_bregexps("!x\\!y!z!").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(apply_bregexps("ax!b", &list), "ayb");
        let list = get_bregexps("!(a)(b)!$2\\1!").unwrap();
        assert_eq!(apply_bregexps("xaby", &list), "xbay");
    }
}
