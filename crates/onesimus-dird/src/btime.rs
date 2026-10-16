//! lib/btime.cc StrToUtime() — the one parser for catalog and console
//! timestamps (compatibility log C-059).
//!
//! The C++ `sscanf` is Bareos' own bsscanf (include/baconfig.h), so
//! `"%u-%u-%u %u:%u:%u%15s"` behaves unlike libc: `%u` skips no blanks and
//! accepts no sign, a field without digits converts to 0 (still counted),
//! digits wrap modulo 2^64 and are then truncated to `unsigned int`, the
//! blank matches any amount of white space (also none) and `%15s` reads
//! the non-blank characters up to the next white space (which must be
//! none, so `"… 12:34:56 text"` is accepted). DateIsValid() then checks the
//! ranges and mktime() reads the fields as local time.
//!
//! Expected values of the unit tests were taken from the installed
//! libbareos (25.1.2~pre43.18be77e68) through a small C++ harness calling
//! its StrToUtime() and bsscanf().

use crate::scan::{bsscanf, Scanned};

/// The C `int` a `%u` conversion stores (unsigned int bits).
fn int_field(values: &[Scanned], i: usize) -> Option<i32> {
    match values.get(i) {
        Some(Scanned::U(v)) => Some(*v as u32 as i32),
        _ => None,
    }
}

/// lib/btime.cc is_leap() on tm_year (C int arithmetic).
fn is_leap(tm_year: i32) -> bool {
    let year = tm_year.wrapping_add(1900);
    year % 4 == 0 && !(year % 100 == 0 && year % 400 != 0)
}

/// lib/btime.cc DateIsValid() on the struct tm fields.
fn date_is_valid(tm: &libc::tm) -> bool {
    if tm.tm_year <= 0
        || !(0..=11).contains(&tm.tm_mon)
        || tm.tm_mday <= 0
        || !(0..=23).contains(&tm.tm_hour)
        || !(0..=59).contains(&tm.tm_min)
        || !(0..=59).contains(&tm.tm_sec)
    {
        return false;
    }
    let days = match tm.tm_mon {
        1 => if is_leap(tm.tm_year) { 29 } else { 28 },
        3 | 5 | 8 | 10 => 30,
        _ => 31,
    };
    tm.tm_mday <= days
}

/// lib/btime.cc StrToUtime(): seconds since the epoch of a local
/// "YYYY-MM-DD HH:MM:SS"; 0 when empty, not parseable or invalid (and
/// when mktime() fails or yields -1). Times before 1970 are negative.
/// The text ends at the first NUL like the C string.
pub fn str_to_utime(text: &str) -> i64 {
    let buf = text.as_bytes();
    if buf.first().is_none_or(|c| *c == 0) {
        return 0;
    }
    let (count, values) = bsscanf(buf, "%u-%u-%u %u:%u:%u%15s");
    let garbage = matches!(values.get(6), Some(Scanned::S(s)) if !s.is_empty());
    if count != 7 || garbage {
        return 0;
    }
    let field = |i| int_field(&values, i).unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = field(0).wrapping_sub(1900);
    tm.tm_mon = field(1).wrapping_sub(1);
    tm.tm_mday = field(2);
    tm.tm_hour = field(3);
    tm.tm_min = field(4);
    tm.tm_sec = field(5);
    tm.tm_isdst = -1;
    if !date_is_valid(&tm) {
        return 0;
    }
    match unsafe { libc::mktime(&mut tm) } {
        -1 => 0,
        time => time as i64,
    }
}

/// [`str_to_utime`] of an optional catalog column (NULL is "").
pub fn str_to_utime_opt(text: Option<&str>) -> i64 {
    text.map_or(0, str_to_utime)
}

/// strftime(3) of a broken-down time (no normalisation, like the C++
/// std::strftime calls on a filled `tm`).
pub fn strftime_tm(fmt: &str, tm: &libc::tm) -> String {
    let Ok(fmt) = std::ffi::CString::new(fmt) else { return String::new() };
    let mut buf = [0u8; 256];
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mktime() of local fields: what StrToUtime returns for valid input.
    fn local(year: i32, mon: i32, mday: i32, hour: i32, min: i32, sec: i32) -> i64 {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec) =
            (year - 1900, mon - 1, mday, hour, min, sec);
        tm.tm_isdst = -1;
        unsafe { libc::mktime(&mut tm) as i64 }
    }

    const T: (i32, i32, i32, i32, i32, i32) = (2026, 10, 16, 12, 34, 56);

    fn t() -> i64 {
        local(T.0, T.1, T.2, T.3, T.4, T.5)
    }

    #[test]
    fn plain_catalog_time() {
        assert_eq!(str_to_utime("2026-10-16 12:34:56"), t());
        assert_eq!(str_to_utime("2026-1-2 3:4:5"), local(2026, 1, 2, 3, 4, 5));
        assert_eq!(str_to_utime("02026-010-016 012:034:056"), t());
        assert_eq!(str_to_utime("2026-12-31 00:00:00"), local(2026, 12, 31, 0, 0, 0));
        assert_eq!(str_to_utime("2026-04-30 00:00:00"), local(2026, 4, 30, 0, 0, 0));
    }

    #[test]
    fn empty_and_missing_fields() {
        assert_eq!(str_to_utime(""), 0);
        assert_eq!(str_to_utime("x"), 0);
        assert_eq!(str_to_utime("2026"), 0);
        assert_eq!(str_to_utime("2026-10"), 0);
        assert_eq!(str_to_utime("2026-10-16"), 0);
        assert_eq!(str_to_utime("2026-10-16 12"), 0);
        assert_eq!(str_to_utime("2026-10-16 12:34"), 0);
        // a field without digits is 0: valid for the time, not for the date
        assert_eq!(str_to_utime("2026-10-16 ::"), local(2026, 10, 16, 0, 0, 0));
        assert_eq!(str_to_utime("2026-10-16 12::"), local(2026, 10, 16, 12, 0, 0));
        assert_eq!(str_to_utime("2026--16 12:34:56"), 0);
        assert_eq!(str_to_utime("2026-10- 12:34:56"), 0);
        assert_eq!(str_to_utime("-10-16 12:34:56"), 0);
        // no blank between date and time: the day swallows the hour
        assert_eq!(str_to_utime("2026-10-1612:34:56"), 0);
    }

    #[test]
    fn trailing_text() {
        // %15s stops at white space: text after a blank is ignored
        assert_eq!(str_to_utime("2026-10-16 12:34:56 trailing"), t());
        assert_eq!(str_to_utime("2026-10-16 12:34:56 "), t());
        assert_eq!(str_to_utime("2026-10-16 12:34:56\n"), t());
        assert_eq!(str_to_utime("2026-10-16 12:34:56\t"), t());
        assert_eq!(str_to_utime("2026-10-16 12:34:56\x0b"), t());
        // ... directly attached text is garbage
        assert_eq!(str_to_utime("2026-10-16 12:34:56x"), 0);
        assert_eq!(str_to_utime("2026-10-16 12:34:56.123"), 0);
        assert_eq!(str_to_utime("2026-10-16 12:34:56+02"), 0);
        assert_eq!(str_to_utime("2026-10-16 12:34:56%"), 0);
        assert_eq!(str_to_utime("2026-10-16 12:34:56ä"), 0);
        assert_eq!(str_to_utime("2026-10-16 12:34:56123456789012345678"), 0);
        // the C string ends at NUL
        assert_eq!(str_to_utime("2026-10-16 12:34:56\0garbage"), t());
        assert_eq!(str_to_utime("\x002026-10-16 12:34:56"), 0);
        // other separators
        assert_eq!(str_to_utime("2026/10/16 12:34:56"), 0);
        assert_eq!(str_to_utime("2026-10-16T12:34:56"), 0);
    }

    #[test]
    fn white_space() {
        assert_eq!(str_to_utime("2026-10-16  12:34:56"), t());
        for sep in ["\t", "\n", "\x0b", "\x0c", "\r", " \t\n "] {
            assert_eq!(str_to_utime(&format!("2026-10-16{sep}12:34:56")), t(), "{sep:?}");
        }
        // %u skips no leading blanks
        assert_eq!(str_to_utime(" 2026-10-16 12:34:56"), 0);
        assert_eq!(str_to_utime("2026- 10-16 12:34:56"), 0);
        assert_eq!(str_to_utime("2026-10-16 12: 34:56"), 0);
    }

    #[test]
    fn signs() {
        assert_eq!(str_to_utime("+2026-10-16 12:34:56"), 0);
        assert_eq!(str_to_utime("-2026-10-16 12:34:56"), 0);
        assert_eq!(str_to_utime("2026-+10-16 12:34:56"), 0);
        assert_eq!(str_to_utime("2026-10-16 -1:00:00"), 0);
    }

    #[test]
    fn ranges() {
        assert_eq!(str_to_utime("2026-13-01 00:00:00"), 0);
        assert_eq!(str_to_utime("2026-00-01 00:00:00"), 0);
        assert_eq!(str_to_utime("2026-01-00 00:00:00"), 0);
        assert_eq!(str_to_utime("2026-01-32 00:00:00"), 0);
        assert_eq!(str_to_utime("2026-04-31 00:00:00"), 0);
        assert_eq!(str_to_utime("2026-01-01 24:00:00"), 0);
        assert_eq!(str_to_utime("2026-01-01 23:60:00"), 0);
        assert_eq!(str_to_utime("2026-01-01 23:59:60"), 0);
        assert_eq!(str_to_utime("2020-13-45 99:99:99"), 0);
    }

    #[test]
    fn leap_years() {
        assert_eq!(str_to_utime("2024-02-29 00:00:00"), local(2024, 2, 29, 0, 0, 0));
        assert_eq!(str_to_utime("2000-02-29 00:00:00"), local(2000, 2, 29, 0, 0, 0));
        assert_eq!(str_to_utime("2023-02-29 00:00:00"), 0);
        assert_eq!(str_to_utime("2100-02-29 00:00:00"), 0);
        assert_eq!(str_to_utime("1900-02-29 00:00:00"), 0);
    }

    #[test]
    fn years() {
        // two-digit years are years 1..99, before 1901: invalid
        assert_eq!(str_to_utime("26-10-16 12:34:56"), 0);
        assert_eq!(str_to_utime("99-01-01 00:00:00"), 0);
        assert_eq!(str_to_utime("1900-01-01 00:00:00"), 0);
        // before 1970: negative, not 0
        assert_eq!(str_to_utime("1901-01-01 00:00:00"), local(1901, 1, 1, 0, 0, 0));
        assert!(str_to_utime("1901-01-01 00:00:00") < 0);
        assert_eq!(str_to_utime("1969-12-31 23:59:59"), local(1969, 12, 31, 23, 59, 59));
        assert_eq!(str_to_utime("2038-01-19 03:14:08"), local(2038, 1, 19, 3, 14, 8));
        assert_eq!(str_to_utime("9999-12-31 23:59:59"), local(9999, 12, 31, 23, 59, 59));
        assert_eq!(str_to_utime("10000-01-01 00:00:00"), local(10000, 1, 1, 0, 0, 0));
    }

    #[test]
    fn mktime_minus_one_is_zero() {
        // the second before the epoch in UTC terms
        let offset = local(1970, 1, 1, 0, 0, 0);
        let (h, m, s) = ((-1 - offset) / 3600, ((-1 - offset) % 3600) / 60, (-1 - offset) % 60);
        if (0..24).contains(&h) && m >= 0 && s >= 0 {
            let text = format!("1970-01-01 {h:02}:{m:02}:{s:02}");
            assert_eq!(local(1970, 1, 1, h as i32, m as i32, s as i32), -1);
            assert_eq!(str_to_utime(&text), 0);
        }
    }

    #[test]
    fn overflow_wraps_like_unsigned_int() {
        // 4294967297 = 2^32 + 1: the month is 1
        assert_eq!(str_to_utime("2026-4294967297-16 12:34:56"), local(2026, 1, 16, 12, 34, 56));
        assert_eq!(str_to_utime("2026-10-4294967312 12:34:56"), t());
        assert_eq!(str_to_utime("2026-10-16 4294967308:34:56"), t());
        assert_eq!(str_to_utime("4294967297-10-16 12:34:56"), 0);
        // the 64-bit accumulator wraps too: 2^64 + 1 is 1, 2^64 + 2026 is 2026
        assert_eq!(str_to_utime("18446744073709551617-01-01 00:00:00"), 0);
        assert_eq!(str_to_utime("18446744073709553642-01-01 00:00:00"), local(2026, 1, 1, 0, 0, 0));
        // years beyond i32 after the 1900 shift (C int wrap) are still mktime()d
        assert!(str_to_utime("2147483647-01-01 00:00:00") > 67_767_976_000_000_000);
        assert!(str_to_utime("2147483648-01-01 00:00:00") > 67_767_976_000_000_000);
    }

    #[test]
    fn optional_column() {
        assert_eq!(str_to_utime_opt(None), 0);
        assert_eq!(str_to_utime_opt(Some("2026-10-16 12:34:56")), t());
    }

    #[test]
    fn strftime_of_fields() {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour) = (126, 9, 1612, 0);
        assert_eq!(strftime_tm("%Y-%m-%d %H:%M:%S", &tm), "2026-10-1612 00:00:00");
    }
}
