//! Linux: Ring als Datei in `/dev/shm`, Warten mit `futex`.
//!
//! Bareos-Gegenstück: keins; bareos-sd kennt keinen Ring.

use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::time::Duration;

/// Directory of the ring files.
const SHM_DIR: &str = "/dev/shm";

/// A shared mapping of a ring; unmapped on drop.
#[derive(Debug)]
pub struct Mapping {
    /// Start of the mapping.
    ptr: *mut u8,
    /// Length in bytes.
    len: usize,
}

// The mapping is plain shared memory; synchronisation is the caller's job
// (atomics in the ring header).
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}

impl Mapping {
    /// Start of the mapping.
    pub fn ptr(&self) -> *mut u8 {
        self.ptr
    }

    /// Length of the mapping.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True for an empty mapping (never for a ring).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: ptr/len come from a successful mmap.
        unsafe {
            libc::munmap(self.ptr.cast(), self.len);
        }
    }
}

/// The file of ring `name`.
fn path(name: &str) -> io::Result<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid ring name {name:?}")));
    }
    Ok(PathBuf::from(SHM_DIR).join(name))
}

/// Maps `len` bytes of `file` shared.
fn map(file: &std::fs::File, len: usize) -> io::Result<Mapping> {
    // SAFETY: a fresh shared mapping of an open file descriptor.
    let ptr = unsafe {
        libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, file.as_raw_fd(), 0)
    };
    if ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(Mapping { ptr: ptr.cast(), len })
}

/// Creates ring `name` with `len` zeroed bytes; fails if it exists.
pub fn create(name: &str, len: usize) -> io::Result<Mapping> {
    let file = OpenOptions::new().read(true).write(true).create_new(true).mode(0o600).open(path(name)?)?;
    file.set_len(len as u64)?;
    map(&file, len)
}

/// Opens the existing ring `name`.
pub fn open(name: &str) -> io::Result<Mapping> {
    let file = OpenOptions::new().read(true).write(true).open(path(name)?)?;
    let len = file.metadata()?.len() as usize;
    map(&file, len)
}

/// Removes ring `name`; mappings stay valid until dropped.
pub fn unlink(name: &str) -> io::Result<()> {
    std::fs::remove_file(path(name)?)
}

/// Names of the rings starting with `prefix`.
pub fn list(prefix: &str) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(SHM_DIR)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix) {
            names.push(name);
        }
    }
    Ok(names)
}

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

/// The calling process id.
pub fn current_pid() -> u64 {
    std::process::id().into()
}

/// Whether process `pid` exists (`kill(pid, 0)`; `EPERM` counts as alive).
pub fn pid_alive(pid: u64) -> bool {
    if pid == 0 || pid > i32::MAX as u64 {
        return false;
    }
    // SAFETY: signal 0 only checks for existence.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
