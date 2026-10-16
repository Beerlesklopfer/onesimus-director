//! lib/scan.cc bsscanf(): what `sscanf` means in the C++ Director
//! (include/baconfig.h `#define sscanf bsscanf`). Shared by the catalog
//! requests of the Storage daemon (C-055) and lib/btime.cc StrToUtime()
//! (C-059).

/// One converted value of [`bsscanf`] (already narrowed to the C target
/// type of the conversion, e.g. `%u` to 32 bits).
#[derive(Debug, Clone, PartialEq)]
pub enum Scanned {
    U(u64),
    D(i64),
    S(Vec<u8>),
}

pub fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// lib/scan.cc bsscanf(): the number of conversions or -1 after a mismatch.
/// The input ends at the first NUL (C string).
pub fn bsscanf(buf: &[u8], fmt: &str) -> (i32, Vec<Scanned>) {
    const BIG: usize = 1000;
    let at = |i: usize| -> u8 { buf.get(i).copied().unwrap_or(0) };
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    let buf_at = |i: usize| -> u8 { if i < end { at(i) } else { 0 } };
    let fmt = fmt.as_bytes();
    let fmt_at = |i: usize| -> u8 { fmt.get(i).copied().unwrap_or(0) };
    let (mut b, mut f) = (0usize, 0usize);
    let mut out = Vec::new();
    let mut count = 0;
    let (mut l, mut h) = (0, 0);
    let mut max_len = BIG;
    let mut error = false;
    while f < fmt.len() && !error {
        if fmt[f] == b'%' {
            f += 1;
            loop {
                let c = fmt_at(f);
                f += 1;
                match c {
                    b'u' => {
                        let mut value: u64 = 0;
                        while buf_at(b).is_ascii_digit() {
                            value = value.wrapping_mul(10).wrapping_add((buf_at(b) - b'0') as u64);
                            b += 1;
                        }
                        out.push(Scanned::U(if h == 1 { value as u16 as u64 } else if l < 2 { value as u32 as u64 } else { value }));
                        count += 1;
                        h = 0;
                        l = 0;
                    }
                    b'd' => {
                        let negative = buf_at(b) == b'-';
                        if negative {
                            b += 1;
                        }
                        let mut value: i64 = 0;
                        while buf_at(b).is_ascii_digit() {
                            value = value.wrapping_mul(10).wrapping_add((buf_at(b) - b'0') as i64);
                            b += 1;
                        }
                        if negative {
                            value = value.wrapping_neg();
                        }
                        out.push(Scanned::D(if h == 1 { value as i16 as i64 } else if l < 2 { value as i32 as i64 } else { value }));
                        count += 1;
                        h = 0;
                        l = 0;
                    }
                    b'h' => {
                        h = 1;
                        if matches!(fmt_at(f), b'd' | b'u') {
                            continue;
                        }
                        error = true;
                    }
                    b'l' => {
                        l = 1;
                        if fmt_at(f) == b'l' {
                            l += 1;
                            f += 1;
                        }
                        if matches!(fmt_at(f), b'd' | b'u') {
                            continue;
                        }
                        error = true;
                    }
                    b'q' => {
                        l = 2;
                        if matches!(fmt_at(f), b'd' | b'u') {
                            continue;
                        }
                        error = true;
                    }
                    b's' => {
                        let mut s = Vec::new();
                        while buf_at(b) != 0 && !is_space(buf_at(b)) && max_len > 0 {
                            max_len -= 1;
                            s.push(buf_at(b));
                            b += 1;
                        }
                        out.push(Scanned::S(s));
                        count += 1;
                        max_len = BIG;
                    }
                    b'c' => {
                        out.push(Scanned::S(vec![buf_at(b)]));
                        b += 1;
                        count += 1;
                    }
                    b'%' => {
                        if buf_at(b) != b'%' {
                            error = true;
                        }
                        b += 1;
                    }
                    _ => {
                        f -= 1;
                        max_len = 0;
                        while fmt_at(f).is_ascii_digit() {
                            max_len = max_len * 10 + (fmt_at(f) - b'0') as usize;
                            f += 1;
                        }
                        if fmt_at(f) == b's' {
                            continue;
                        }
                        error = true;
                    }
                }
                break;
            }
        } else if is_space(fmt[f]) {
            f += 1;
            while is_space(buf_at(b)) {
                b += 1;
            }
        } else {
            let mismatch = buf_at(b) != fmt[f];
            b += 1;
            f += 1;
            if mismatch {
                error = true;
                break;
            }
        }
    }
    (if error { -1 } else { count }, out)
}
