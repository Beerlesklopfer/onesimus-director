//! lib/berrno.cc BErrNo::bstrerror(): the plain strerror() text in the
//! messages ("No such file or directory", without Rust's "(os error 2)").

use std::fmt;

/// An errno value displayed like bstrerror().
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BErr(pub i32);

impl BErr {
    /// errno of the last failed call.
    pub fn last() -> Self {
        BErr(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
    pub fn raw_os_error(&self) -> Option<i32> {
        Some(self.0)
    }
}

/// The strerror() text of the Win32 error codes a file operation meets, as
/// the C runtime words it (Bareos calls `strerror` on the errno it maps
/// the Win32 error to, `win32/compat/compat.cc` `statDir`/`b_errno_win32`).
/// `None` for a code without an entry: the text of the system, in the
/// language of the system, is used then.
#[cfg(any(windows, test))]
fn win32_text(code: i32) -> Option<&'static str> {
    Some(match code {
        2 | 3 | 15 | 53 | 67 => "No such file or directory",
        5 | 19 | 32 | 33 | 1314 => "Permission denied",
        80 | 183 => "File exists",
        87 => "Invalid argument",
        39 | 112 => "No space left on device",
        4 => "Too many open files",
        145 => "Directory not empty",
        206 | 267 => "File name too long",
        _ => return None,
    })
}

#[cfg(windows)]
impl fmt::Display for BErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match win32_text(self.0) {
            Some(text) => f.write_str(text),
            None => {
                // "<text> (os error N)": the number is no part of bstrerror()
                let text = std::io::Error::from_raw_os_error(self.0).to_string();
                let text = text.rsplit_once(" (os error").map_or(text.as_str(), |(t, _)| t);
                f.write_str(text.trim())
            }
        }
    }
}

#[cfg(unix)]
impl fmt::Display for BErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = unsafe { libc::strerror(self.0) };
        if p.is_null() {
            return write!(f, "Unknown error {}", self.0);
        }
        let s = unsafe { std::ffi::CStr::from_ptr(p) };
        f.write_str(&s.to_string_lossy())
    }
}

/// bstrerror() of an io::Error.
pub fn s(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(n) => BErr(n).to_string(),
        None => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn win32_codes_read_like_strerror() {
        assert_eq!(super::win32_text(2), Some("No such file or directory"));
        assert_eq!(super::win32_text(5), Some("Permission denied"));
        assert_eq!(super::win32_text(183), Some("File exists"));
        assert_eq!(super::win32_text(99999), None);
    }

    #[cfg(unix)]
    #[test]
    fn plain_text() {
        assert_eq!(super::BErr(libc::ENOENT).to_string(), "No such file or directory");
        assert_eq!(super::s(&std::io::Error::from_raw_os_error(libc::EACCES)), "Permission denied");
    }
}
