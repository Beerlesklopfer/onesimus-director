// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Bareos-Base64: Zahlen und Binärdaten im Bareos-eigenen Base64.
//!
//! Zweck: Bareos kodiert zwei Dinge mit demselben Alphabet
//! (`A–Z a–z 0–9 + /`), aber nicht nach RFC 4648:
//!
//! - **Zahlen** ([`to_base64`], [`from_base64`]): eine ganze Zahl als
//!   Folge von 6-Bit-Ziffern, höchstwertige zuerst, ohne Auffüllung,
//!   negatives Vorzeichen als führendes `-`. So stehen die Felder der
//!   LStat-Zeichenkette im Katalog ([`crate::lstat`]).
//! - **Binärdaten** ([`bin_to_base64`]): Prüfsummen und Schlüssel. Mit
//!   `compatible = true` entspricht das RFC-Base64 ohne `=`-Auffüllung; mit
//!   `compatible = false` werden die Bytes als vorzeichenbehaftet gelesen
//!   (Bareos-Eigenheit, gebraucht in CRAM-MD5).
//!
//! Datenfluss: FD → (LStat, Prüfsumme) → SD → Director → Katalog; Director →
//! (CRAM-MD5-Antwort) → FD/SD.
//!
//! Bareos-Gegenstück: `core/src/lib/base64.cc` (`ToBase64`, `FromBase64`,
//! `BinToBase64`).
//!
//! Zusammengeführt (19.11.2026) aus `ofd/src/attribs.rs` (`from_base64`,
//! `to_base64`, `base64_value`), `odir/src/commands/bvfs.rs`
//! (`from_base64`, `base64_value`), `odir/src/ovfs/lstat.rs` (`from_base64`,
//! `digit`), `bareos-bnet/src/cram_md5.rs` (`bareos_bin_to_base64`),
//! `odir/src/catreq.rs` und `odir/src/volume_key.rs` (`bin_to_base64`).

/// The 64 digits of Bareos base64, value = index.
pub const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Value of one base64 digit; any other byte counts as 0 (as in Bareos).
pub fn digit_value(c: u8) -> u64 {
    match c {
        b'A'..=b'Z' => u64::from(c - b'A'),
        b'a'..=b'z' => u64::from(c - b'a') + 26,
        b'0'..=b'9' => u64::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 0,
    }
}

/// `FromBase64()`: the number at the start of `s` and the bytes it used.
///
/// The number ends at a blank, a NUL or the end of the slice; a leading `-`
/// negates it. Overflow wraps like the C++ `uint64_t` arithmetic.
pub fn from_base64(s: &[u8]) -> (i64, usize) {
    let mut i = 0;
    let negative = s.first() == Some(&b'-');
    if negative {
        i += 1;
    }
    let mut value: u64 = 0;
    while i < s.len() && s[i] != 0 && s[i] != b' ' {
        value = (value << 6).wrapping_add(digit_value(s[i]));
        i += 1;
    }
    (if negative { (value as i64).wrapping_neg() } else { value as i64 }, i)
}

/// `ToBase64()`: `value` as Bareos base64 number (`0` is `"A"`).
pub fn to_base64(value: i64) -> String {
    let mut out = Vec::new();
    let mut rest = if value < 0 { value.wrapping_neg() as u64 } else { value as u64 };
    loop {
        out.push(DIGITS[(rest & 0x3f) as usize]);
        rest >>= 6;
        if rest == 0 {
            break;
        }
    }
    if value < 0 {
        out.push(b'-');
    }
    out.reverse();
    out.into_iter().map(char::from).collect()
}

/// `BinToBase64()`: binary data as base64 without padding.
///
/// `compatible = true` reads the bytes as `uint8_t` (RFC 4648 without `=`);
/// `compatible = false` reads them as `int8_t`, so bytes ≥ 0x80 are
/// sign-extended, and the last digit is not shifted (Bareos-specific).
pub fn bin_to_base64(bin: &[u8], compatible: bool) -> String {
    let mut buf = Vec::new();
    let mut reg: u32 = 0;
    let mut rem: u32 = 0;
    let mut i: usize = 0;
    while i < bin.len() {
        if rem < 6 {
            reg <<= 8;
            if compatible {
                reg |= u32::from(bin[i]);
            } else {
                // C++: reg |= (int8_t)bin[i];
                reg |= bin[i] as i8 as i32 as u32;
            }
            i += 1;
            rem += 8;
        }
        let save = reg;
        reg >>= rem - 6;
        buf.push(DIGITS[(reg & 0x3F) as usize]);
        reg = save;
        rem -= 6;
    }
    if rem > 0 {
        let mask: u32 = (1 << rem) - 1;
        if compatible {
            buf.push(DIGITS[((reg & mask) << (6 - rem)) as usize]);
        } else {
            buf.push(DIGITS[(reg & mask) as usize]);
        }
    }
    buf.into_iter().map(char::from).collect()
}

#[cfg(test)]
mod tests {
    //! Golden values: the outputs of the implementations this module
    //! replaces, kept verbatim in [`old`] and compared over many inputs.

    use super::*;

    /// The replaced implementations, copied unchanged (19.11.2026).
    mod old {
        /// ofd/src/attribs.rs `from_base64` + `base64_value`.
        pub fn ofd_from_base64(s: &[u8]) -> (i64, usize) {
            let mut i = 0;
            let neg = s.first() == Some(&b'-');
            if neg {
                i += 1;
            }
            let mut val: u64 = 0;
            while i < s.len() && s[i] != 0 && s[i] != b' ' {
                val = (val << 6).wrapping_add(ofd_base64_value(s[i]));
                i += 1;
            }
            (if neg { (val as i64).wrapping_neg() } else { val as i64 }, i)
        }

        /// ofd/src/attribs.rs `base64_value`.
        fn ofd_base64_value(c: u8) -> u64 {
            match c {
                b'A'..=b'Z' => u64::from(c - b'A'),
                b'a'..=b'z' => u64::from(c - b'a') + 26,
                b'0'..=b'9' => u64::from(c - b'0') + 52,
                b'+' => 62,
                b'/' => 63,
                _ => 0,
            }
        }

        /// ofd/src/attribs.rs `to_base64`.
        pub fn ofd_to_base64(value: i64) -> String {
            const MAP: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = Vec::new();
            let mut val = if value < 0 { value.wrapping_neg() as u64 } else { value as u64 };
            loop {
                out.push(MAP[(val & 0x3f) as usize]);
                val >>= 6;
                if val == 0 {
                    break;
                }
            }
            if value < 0 {
                out.push(b'-');
            }
            out.reverse();
            String::from_utf8(out).unwrap_or_default()
        }

        /// odir/src/ovfs/lstat.rs `from_base64` (whole field, no stop byte).
        pub fn ovfs_from_base64(field: &str) -> i64 {
            let (negative, digits) = match field.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, field),
            };
            let value = digits.bytes().fold(0u64, |acc, byte| acc.wrapping_shl(6).wrapping_add(super::digit_value(byte)));
            if negative { (value as i64).wrapping_neg() } else { value as i64 }
        }

        /// bareos-bnet/src/cram_md5.rs `bareos_bin_to_base64`.
        pub fn bnet_bin_to_base64(bin: &[u8], compatible: bool) -> String {
            const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut buf = Vec::new();
            let mut reg: u32 = 0;
            let mut rem: u32 = 0;
            let mut i: usize = 0;
            while i < bin.len() {
                if rem < 6 {
                    reg <<= 8;
                    if compatible {
                        reg |= bin[i] as u32;
                    } else {
                        reg |= bin[i] as i8 as i32 as u32;
                    }
                    i += 1;
                    rem += 8;
                }
                let save = reg;
                reg >>= rem - 6;
                buf.push(DIGITS[(reg & 0x3F) as usize]);
                reg = save;
                rem -= 6;
            }
            if rem > 0 {
                let mask: u32 = (1 << rem) - 1;
                if compatible {
                    buf.push(DIGITS[((reg & mask) << (6 - rem)) as usize]);
                } else {
                    buf.push(DIGITS[(reg & mask) as usize]);
                }
            }
            String::from_utf8(buf).unwrap()
        }

        /// odir/src/catreq.rs `bin_to_base64` (compatible = true).
        pub fn catreq_bin_to_base64(bin: &[u8]) -> String {
            const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            let (mut reg, mut rem): (u32, i32) = (0, 0);
            let mut i = 0;
            while i < bin.len() {
                if rem < 6 {
                    reg = (reg << 8) | bin[i] as u32;
                    i += 1;
                    rem += 8;
                }
                out.push(DIGITS[((reg >> (rem - 6)) & 0x3f) as usize] as char);
                rem -= 6;
            }
            if rem != 0 {
                let mask = (1u32 << rem) - 1;
                out.push(DIGITS[((reg & mask) << (6 - rem)) as usize] as char);
            }
            out
        }
    }

    /// Sample numbers around the interesting borders.
    fn numbers() -> Vec<i64> {
        let mut v = vec![0, 1, 63, 64, 4095, 4096, 0o100644, 1_700_000_000, -1, -5, -64, i64::MAX, i64::MIN, i64::MIN + 1];
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..500 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            v.push(x as i64);
            v.push((x >> 40) as i64);
        }
        v
    }

    /// Sample byte strings: empty, every length up to 20, all byte values.
    fn byte_strings() -> Vec<Vec<u8>> {
        let mut v: Vec<Vec<u8>> = (0..=20).map(|n| (0..n).map(|i| (i * 37 + 200) as u8).collect()).collect();
        v.push((0..=255u8).collect());
        v.push(b"abc".to_vec());
        v
    }

    #[test]
    fn to_base64_matches_ofd() {
        for n in numbers() {
            assert_eq!(to_base64(n), old::ofd_to_base64(n), "{n}");
        }
    }

    #[test]
    fn from_base64_matches_ofd_and_bvfs_and_ovfs() {
        for n in numbers() {
            let s = to_base64(n);
            assert_eq!(from_base64(s.as_bytes()), old::ofd_from_base64(s.as_bytes()));
            assert_eq!(from_base64(s.as_bytes()).0, old::ovfs_from_base64(&s));
            assert_eq!(from_base64(s.as_bytes()), (n, s.len()));
        }
        // stops at blank and NUL, unknown bytes count as 0
        for raw in [&b"BA C"[..], b"BA\0C", b"", b"-", b"*?", b"-B x"] {
            assert_eq!(from_base64(raw), old::ofd_from_base64(raw), "{raw:?}");
        }
    }

    #[test]
    fn number_goldens() {
        assert_eq!(to_base64(0), "A");
        assert_eq!(to_base64(64), "BA");
        assert_eq!(to_base64(-1), "-B");
        assert_eq!(to_base64(0o100644), "IGk");
    }

    #[test]
    fn bin_to_base64_matches_catreq_and_rfc() {
        use base64::Engine;
        for bin in byte_strings() {
            assert_eq!(bin_to_base64(&bin, true), old::catreq_bin_to_base64(&bin));
            assert_eq!(bin_to_base64(&bin, true), old::bnet_bin_to_base64(&bin, true));
            assert_eq!(bin_to_base64(&bin, false), old::bnet_bin_to_base64(&bin, false));
            // volume_key.rs used STANDARD_NO_PAD for compatible = true
            assert_eq!(bin_to_base64(&bin, true), base64::engine::general_purpose::STANDARD_NO_PAD.encode(&bin));
        }
    }

    #[test]
    fn bin_to_base64_goldens() {
        // bareos-bnet cram_md5 tests (compatible and Bareos form)
        assert_eq!(bin_to_base64(&[0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1, 0x7f, 0x72], true),
            "kAFQmDzST7DWlj99KOF/cg");
        assert_eq!(bin_to_base64(&[0xff], false), "/D");
        assert_eq!(bin_to_base64(&[0xff], true), "/w");
        assert_eq!(bin_to_base64(&[0x01, 0x02, 0x03], false), "AQID");
    }
}
