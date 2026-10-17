//! lib/breg.cc: the `!from!to!opts` file name manipulation of restores
//! (`restore ... regexwhere=`) with POSIX regcomp/regexec like the C
//! original. Same code as onesimus-dird/src/breg.rs, on bytes (file names
//! need not be UTF-8).

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
pub fn apply_bregexps(fname: &[u8], list: &[Bregexp]) -> Vec<u8> {
    let mut name = fname.to_vec();
    for reg in list {
        name = reg.replace(&name).0;
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regexps_are_parsed_and_applied_like_breg_cc() {
        let list = get_bregexps("!a!b!").unwrap();
        assert_eq!(apply_bregexps(b"/tmp/a/a", &list), b"/tmp/b/a");
        assert!(get_bregexps("bad").is_none());
        assert!(get_bregexps("!a!b").is_none());
        let list = get_bregexps("!/tmp!!i,!([^/])$!$1.rst!,!^!/new!").unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(apply_bregexps(b"/TMP/a/b", &list), b"/new/a/b.rst");
        let list = get_bregexps("!(a)(b)!$2\\1!").unwrap();
        assert_eq!(apply_bregexps(b"xaby", &list), b"xbay");
    }
}
