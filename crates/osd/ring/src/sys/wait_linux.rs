//! Linux: Warten und Wecken mit `futex` auf einem Wort im gemeinsamen
//! Speicher (ohne `FUTEX_PRIVATE_FLAG`, also prozessübergreifend).

use std::sync::atomic::AtomicU32;
use std::time::Duration;

/// Waits while `word` is `expected`, at most `timeout` (spurious wake-ups
/// possible; the caller rechecks its condition).
pub fn wait(word: &AtomicU32, expected: u32, timeout: Duration) {
    let ts = libc::timespec { tv_sec: timeout.as_secs() as libc::time_t, tv_nsec: timeout.subsec_nanos() as libc::c_long };
    // SAFETY: FUTEX_WAIT on a valid, aligned u32 in shared memory.
    unsafe {
        libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAIT, expected, &ts as *const libc::timespec);
    }
}

/// Wakes every process waiting on `word`.
pub fn wake(word: &AtomicU32) {
    // SAFETY: FUTEX_WAKE on a valid, aligned u32 in shared memory.
    unsafe {
        libc::syscall(libc::SYS_futex, word.as_ptr(), libc::FUTEX_WAKE, i32::MAX);
    }
}
