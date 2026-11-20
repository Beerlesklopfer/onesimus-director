//! macOS und Windows: Platzhalter, damit der Workspace dort übersetzt.
//!
//! Vorgesehen (siehe [`super`]): macOS `shm_open` + `mmap` und
//! `os_sync_wait_on_address` bzw. POSIX-Semaphore, Windows benanntes
//! `CreateFileMapping` und benannte Events. Jeder Aufruf endet heute mit
//! `unimplemented!()`.

use std::io;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

/// A shared mapping of a ring (not implemented on this platform).
#[derive(Debug)]
pub struct Mapping {
    /// Never constructed.
    _private: (),
}

impl Mapping {
    /// Start of the mapping.
    pub fn ptr(&self) -> *mut u8 {
        unimplemented!("osd ring: shared memory on this platform")
    }

    /// Length of the mapping.
    pub fn len(&self) -> usize {
        unimplemented!("osd ring: shared memory on this platform")
    }

    /// True for an empty mapping.
    pub fn is_empty(&self) -> bool {
        unimplemented!("osd ring: shared memory on this platform")
    }
}

/// Creates ring `name` (not implemented on this platform).
pub fn create(_name: &str, _len: usize) -> io::Result<Mapping> {
    unimplemented!("osd ring: shared memory on this platform")
}

/// Opens ring `name` (not implemented on this platform).
pub fn open(_name: &str) -> io::Result<Mapping> {
    unimplemented!("osd ring: shared memory on this platform")
}

/// Removes ring `name` (not implemented on this platform).
pub fn unlink(_name: &str) -> io::Result<()> {
    unimplemented!("osd ring: shared memory on this platform")
}

/// Names of the rings starting with `prefix` (not implemented).
pub fn list(_prefix: &str) -> io::Result<Vec<String>> {
    unimplemented!("osd ring: shared memory on this platform")
}

/// Cross-process wait (not implemented on this platform).
pub fn wait(_word: &AtomicU32, _expected: u32, _timeout: Duration) {
    unimplemented!("osd ring: cross-process wait on this platform")
}

/// Cross-process wake (not implemented on this platform).
pub fn wake(_word: &AtomicU32) {
    unimplemented!("osd ring: cross-process wake on this platform")
}

/// The calling process id.
pub fn current_pid() -> u64 {
    std::process::id().into()
}

/// Whether process `pid` exists (not implemented on this platform).
pub fn pid_alive(_pid: u64) -> bool {
    unimplemented!("osd ring: process check on this platform")
}
