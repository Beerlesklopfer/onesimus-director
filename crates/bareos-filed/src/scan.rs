//! lib/scan.cc bsscanf(): the `sscanf` of all Bareos daemons
//! (include/baconfig.h `#define sscanf bsscanf`). Unlike the C library:
//! a blank in the format eats zero or more blanks, `%s` takes the
//! (possibly empty) run of non-blank characters, a number without digits
//! is 0 and still counts, text after the format is ignored; -1 on a
//! literal mismatch or an unknown conversion.

/// One converted value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    Int(i64),
    Str(Vec<u8>),
    Char(u8),
}

impl Val {
    pub fn int(&self) -> i64 {
        match self {
            Val::Int(v) => *v,
            _ => 0,
        }
    }
    pub fn text(&self) -> String {
        match self {
            Val::Str(s) => String::from_utf8_lossy(s).into_owned(),
            Val::Char(c) => (*c as char).to_string(),
            Val::Int(v) => v.to_string(),
        }
    }
}

fn is_space(c: u8) -> bool {
    c.is_ascii() && (c as char).is_ascii_whitespace() || c == 0x0b
}

/// bsscanf(): (count, values); count -1 on an error.
pub fn bsscanf(buf: &[u8], fmt: &str) -> (i32, Vec<Val>) {
    let f = fmt.as_bytes();
    let mut vals = Vec::new();
    let mut count = 0;
    let mut b = 0usize;
    let mut i = 0usize;
    let at = |b: usize| buf.get(b).copied().unwrap_or(0);
    while i < f.len() {
        if f[i] == b'%' {
            i += 1;
            let mut max_len = usize::MAX;
            // modifiers: h, l, ll, q, width
            loop {
                match f.get(i).copied() {
                    Some(b'h') | Some(b'l') | Some(b'q') => i += 1,
                    Some(d) if d.is_ascii_digit() => {
                        let start = i;
                        while i < f.len() && f[i].is_ascii_digit() {
                            i += 1;
                        }
                        max_len = std::str::from_utf8(&f[start..i]).ok().and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
                        if f.get(i) != Some(&b's') {
                            return (-1, vals);
                        }
                    }
                    _ => break,
                }
            }
            match f.get(i).copied() {
                Some(b'u') => {
                    let mut v: u64 = 0;
                    while at(b).is_ascii_digit() {
                        v = v.wrapping_mul(10).wrapping_add(u64::from(at(b) - b'0'));
                        b += 1;
                    }
                    vals.push(Val::Int(v as i64));
                    count += 1;
                }
                Some(b'd') => {
                    let neg = at(b) == b'-';
                    if neg {
                        b += 1;
                    }
                    let mut v: i64 = 0;
                    while at(b).is_ascii_digit() {
                        v = v.wrapping_mul(10).wrapping_add(i64::from(at(b) - b'0'));
                        b += 1;
                    }
                    vals.push(Val::Int(if neg { -v } else { v }));
                    count += 1;
                }
                Some(b's') => {
                    let start = b;
                    let mut n = 0;
                    while b < buf.len() && buf[b] != 0 && !is_space(buf[b]) && n < max_len {
                        b += 1;
                        n += 1;
                    }
                    vals.push(Val::Str(buf[start..b].to_vec()));
                    count += 1;
                }
                Some(b'c') => {
                    vals.push(Val::Char(at(b)));
                    b += 1;
                    count += 1;
                }
                Some(b'%') => {
                    if at(b) != b'%' {
                        return (-1, vals);
                    }
                    b += 1;
                }
                _ => return (-1, vals),
            }
            i += 1;
        } else if is_space(f[i]) {
            i += 1;
            while b < buf.len() && is_space(buf[b]) {
                b += 1;
            }
        } else {
            if at(b) != f[i] {
                return (-1, vals);
            }
            b += 1;
            i += 1;
        }
    }
    (count, vals)
}

/// sscanf(...) == n: the values when exactly `n` were converted.
pub fn scan(buf: &[u8], fmt: &str, n: i32) -> Option<Vec<Val>> {
    let (c, v) = bsscanf(buf, fmt);
    (c == n).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_bsscanf() {
        // two blanks, the volume name "1", the 7th number missing -> 0
        let v = scan(b"session  1 1792223388 0 0 0 0", "session %127s %ld %ld %ld %ld %ld %ld\n", 7).unwrap();
        assert_eq!(v[0], Val::Str(b"1".to_vec()));
        assert_eq!(v[1].int(), 1792223388);
        assert_eq!(v[6].int(), 0);
        let v = scan(b"restore replace=a prelinks=0 where=/tmp/x\n", "restore replace=%c prelinks=%d where=%s\n", 3).unwrap();
        assert_eq!((v[0].clone(), v[1].int(), v[2].text()), (Val::Char(b'a'), 0, "/tmp/x".to_string()));
        assert_eq!(bsscanf(b"restore replace=a prelinks=0 regexwhere=!a!b!\n", "restore replace=%c prelinks=%d where=%s\n").0, -1);
        assert_eq!(bsscanf(b"rechdr 1 2 -3 4 5", "rechdr %u %u %d %d %u").1[2].int(), -3);
        assert_eq!(scan(b"3000 OK open ticket = 7\n", "3000 OK open ticket = %d\n", 1).unwrap()[0].int(), 7);
    }
}
