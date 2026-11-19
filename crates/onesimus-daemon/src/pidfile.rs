//! PID-Datei.
//!
//! Zweck: Die PID-Datei sagt `-k`, Init-Skripten und Administratoren, unter
//! welcher Prozessnummer der Daemon läuft. Im Hintergrund schreibt sie
//! [`crate::background::daemonize`] nach dem `fork`; im Vordergrund
//! [`write()`].
//!
//! Datenfluss: `main` → [`write()`] → Datei `<pid>\n`; beim Ende [`remove`].
//!
//! Bareos-Gegenstück: `lib/bsys.cc` `CreatePidFile`, `DeletePidFile`.
//! Herkunft: `odir/src/main.rs` (`-p` im Vordergrund).

use std::path::Path;

/// Writes `<pid>\n` of this process to `path`.
pub fn write(path: &Path) -> std::io::Result<()> {
    std::fs::write(path, format!("{}\n", std::process::id()))
}

/// The PID stored in `path`, if the file exists and holds a number.
pub fn read(path: &Path) -> Option<i32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Removes `path` if it still names this process (another start may have
/// replaced it).
pub fn remove(path: &Path) {
    if read(path) == Some(std::process::id() as i32) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_remove() {
        let path = std::env::temp_dir().join(format!("onesimus-daemon-pid-{}", std::process::id()));
        write(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{}\n", std::process::id()));
        assert_eq!(read(&path), Some(std::process::id() as i32));
        remove(&path);
        assert!(!path.exists());
    }

    #[test]
    fn foreign_pid_file_is_kept() {
        let path = std::env::temp_dir().join(format!("onesimus-daemon-pid-other-{}", std::process::id()));
        std::fs::write(&path, "1\n").unwrap();
        remove(&path);
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();
    }
}
