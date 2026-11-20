//! macOS und Windows: Warten/Wecken als Platzhalter (siehe [`super`]).
//!
//! Vorgesehen: macOS `os_sync_wait_on_address`/`os_sync_wake_by_address_all`
//! mit `OS_SYNC_WAIT_ON_ADDRESS_SHARED`; Windows je Wartendem ein benanntes
//! Auto-Reset-Event, dessen Name im Ringkopf steht.

use std::sync::atomic::AtomicU32;
use std::time::Duration;

/// Cross-process wait (not implemented on this platform).
pub fn wait(_word: &AtomicU32, _expected: u32, _timeout: Duration) {
    unimplemented!("osd ring: cross-process wait on this platform")
}

/// Cross-process wake (not implemented on this platform).
pub fn wake(_word: &AtomicU32) {
    unimplemented!("osd ring: cross-process wake on this platform")
}
