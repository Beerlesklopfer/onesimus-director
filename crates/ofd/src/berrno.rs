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
    fn plain_text() {
        assert_eq!(super::BErr(libc::ENOENT).to_string(), "No such file or directory");
        assert_eq!(super::s(&std::io::Error::from_raw_os_error(libc::EACCES)), "Permission denied");
    }
}
