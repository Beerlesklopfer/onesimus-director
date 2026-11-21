//! What ofd needs from the platform beyond std, in one place.
//!
//! Unix: re-exports of `std::os::unix` (no behaviour of its own).
//!
//! Windows (docs/plans/PORTIERUNG-MACOS-WINDOWS.md): the file tree layer
//! (stat, times, restore of files and directories) is `winfs.rs` and the
//! Windows branches of `create_file.rs` and `attribs.rs`; owner, mode,
//! devices, POSIX ACLs and xattrs do not exist there. Only the byte access
//! to `OsStr` is needed from this module.

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
