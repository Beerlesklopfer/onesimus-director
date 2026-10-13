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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_parts() {
        // 2021-01-02 is far enough from any timezone edge for the year
        assert_eq!(strftime("%Y", 1_609_588_800), "2021");
        assert_eq!(strftime("%m", 1_609_588_800).len(), 2);
    }
}
