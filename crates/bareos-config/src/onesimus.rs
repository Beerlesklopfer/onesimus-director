//! The Onesimus configuration directory (`/etc/onesimus`).
//!
//! The Bareos configuration trees (`bareos-dir.d`, `bareos-sd.d`,
//! `bareos-fd.d`) stay exactly what Bareos reads: a Bareos daemon can be
//! pointed at the same files. Everything Onesimus adds to them lives in this
//! directory, in one overlay tree per daemon:
//!
//! | Daemon | Overlay | Extends |
//! |---|---|---|
//! | `odir` | `<dir>/odir.d/<Type>/<Name>.conf` | the resource of the same type and name in `bareos-dir.d` |
//! | `osd` | `<dir>/osd.d/<Type>/<Name>.conf` | the resource in `bareos-sd.d` |
//! | `ofd` | `<dir>/ofd.d/<Type>/<Name>.conf` | the resource in `bareos-fd.d` |
//!
//! Each daemon takes the directory from its option `--overlay`
//! (default [`DEFAULT_DIR`]); the daemon calls [`set_dir`] once at startup,
//! before it loads its configuration.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The Onesimus configuration directory when the command line names none.
pub const DEFAULT_DIR: &str = "/etc/onesimus";

/// The directory chosen at startup.
static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Sets the Onesimus configuration directory; only the first call counts.
pub fn set_dir(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// The Onesimus configuration directory: the one set at startup, else [`DEFAULT_DIR`].
pub fn dir() -> PathBuf {
    DIR.get().cloned().unwrap_or_else(|| Path::new(DEFAULT_DIR).to_path_buf())
}

/// The overlay tree of a daemon (`odir.d`, `osd.d`, `ofd.d`) below [`dir`].
pub fn overlay(tree: &str) -> PathBuf {
    dir().join(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_lies_below_the_onesimus_directory() {
        // nothing sets the directory in a test run
        assert_eq!(dir(), Path::new("/etc/onesimus"));
        assert_eq!(overlay("osd.d"), Path::new("/etc/onesimus/osd.d"));
    }
}
