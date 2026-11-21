//! What ofd needs from the platform beyond std, in one place.
//!
//! Unix: re-exports of `std::os::unix` (no behaviour of its own).
//!
//! Windows (W1, docs/plans/PORTIERUNG-MACOS-WINDOWS.md): ofd builds and runs
//! as a service, but the file tree layer (stat, owner, mode, devices, ACLs,
//! xattrs) is the POSIX one and has no Windows counterpart yet. Bareos uses
//! a separate Win32 path there (`BackupRead`/`BackupWrite`,
//! `STREAM_WIN32_DATA`, `win32/compat/compat.cc`), planned as W2. Until
//! then backup, estimate, verify and restore jobs are refused at their
//! start ([`FILE_JOBS_UNSUPPORTED`]), and the POSIX helpers below only exist
//! so that the shared code compiles; each fails with [`unsupported`]
//! instead of doing something different from Bareos.

/// Byte access to `OsStr` (Unix: `std::os::unix::ffi::OsStrExt`).
#[cfg(unix)]
pub use std::os::unix::ffi::OsStrExt;

/// Byte access to `OsStr` on Windows, where file names on the wire are
/// UTF-8 (Bareos converts UTF-16 names to UTF-8 on Win32,
/// `win32/compat/compat.cc` `wchar_2_UTF8`).
#[cfg(windows)]
pub trait OsStrExt {
    /// The UTF-8 bytes as an `OsStr`; a name that is not UTF-8 becomes
    /// the empty name (every file operation on it then fails).
    fn from_bytes(bytes: &[u8]) -> &Self;
    /// The name as bytes (WTF-8, which is UTF-8 for every valid name).
    fn as_bytes(&self) -> &[u8];
}

#[cfg(windows)]
impl OsStrExt for std::ffi::OsStr {
    fn from_bytes(bytes: &[u8]) -> &Self {
        std::ffi::OsStr::new(std::str::from_utf8(bytes).unwrap_or(""))
    }
    fn as_bytes(&self) -> &[u8] {
        self.as_encoded_bytes()
    }
}

/// The job log text for a backup, estimate, verify or restore job on a
/// platform without the file tree layer (Windows until W2).
#[cfg(windows)]
pub const FILE_JOBS_UNSUPPORTED: &str =
    "file backup and restore are not supported on Windows by this version of ofd \
     (the Win32 BackupRead/BackupWrite path is not implemented yet)";

/// The error of a POSIX file operation that has no Windows implementation.
#[cfg(windows)]
pub fn unsupported() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, FILE_JOBS_UNSUPPORTED)
}

/// `ENOSYS` for the helpers that report errors as an errno.
#[cfg(windows)]
pub const ENOSYS: i32 = 40;
