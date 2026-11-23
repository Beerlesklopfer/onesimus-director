// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Local time formatting like C++ bstrftime(): libc strftime with the
//! process locale, so "%a %d-%b-%Y" reads "Di 13-Okt-2026" on a German
//! system exactly as with bareos-dir. `init_locale()` runs once at startup.

use std::ffi::CString;

/// setlocale(LC_TIME, "") — call once at startup, before threads are spawned.
pub fn init_locale() {
    let empty = CString::new("").unwrap();
    unsafe {
        libc::setlocale(libc::LC_TIME, empty.as_ptr());
    }
}

static DAEMON_STARTED: std::sync::OnceLock<libc::time_t> = std::sync::OnceLock::new();

/// Remember the daemon start time (for `status director`).
pub fn mark_daemon_start() {
    let _ = DAEMON_STARTED.set(unsafe { libc::time(std::ptr::null_mut()) });
}

pub fn daemon_started() -> libc::time_t {
    *DAEMON_STARTED.get().unwrap_or(&0)
}

pub fn strftime_now(fmt: &str) -> String {
    strftime(fmt, unsafe { libc::time(std::ptr::null_mut()) })
}

pub fn strftime(fmt: &str, t: libc::time_t) -> String {
    let Ok(cfmt) = CString::new(fmt) else { return String::new() };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let mut buf = [0u8; 128];
    let len = unsafe {
        libc::localtime_r(&t, &mut tm);
        libc::strftime(buf.as_mut_ptr() as *mut libc::c_char, buf.len(), cfmt.as_ptr(), &tm)
    };
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

/// Unix seconds now.
pub fn now() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// `time` (Unix seconds) in the local zone as `fmt` (strftime, process locale).
pub fn format(time: i64, fmt: &str) -> String {
    strftime(fmt, time as libc::time_t)
}

/// `time` broken down in the local zone (`localtime_r`), as bareos-dir's
/// `struct tm`: `tm_year` counts from 1900, `tm_mon` and `tm_yday` from 0.
pub fn local_tm(time: i64) -> libc::tm {
    let t = time as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    tm
}

/// Unix seconds of the local time `year`-`month`-`day` `hour`:00:00
/// (`mktime`, daylight saving decided by the system; `month` counts from 1).
pub fn local_time(year: i32, month: i32, day: i32, hour: i32) -> i64 {
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = year - 1900;
    tm.tm_mon = month - 1;
    tm.tm_mday = day;
    tm.tm_hour = hour;
    tm.tm_isdst = -1;
    unsafe { libc::mktime(&mut tm) as i64 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_parts() {
        // 2021-01-02 is far enough from any timezone edge for the year
        assert_eq!(strftime("%Y", 1_609_588_800), "2021");
        assert_eq!(strftime("%m", 1_609_588_800).len(), 2);
    }

    #[test]
    fn local_time_and_broken_down_time_agree() {
        let t = local_time(2026, 10, 15, 12);
        let tm = local_tm(t);
        assert_eq!((tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour), (126, 9, 15, 12));
        assert_eq!(format(t, "%Y-%m-%d %H"), "2026-10-15 12");
    }
}
