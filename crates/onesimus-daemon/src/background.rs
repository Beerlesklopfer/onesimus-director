//! Vordergrund/Hintergrund: der Start ohne `-f`.
//!
//! Zweck: Ohne `-f` löst sich der Daemon vom Terminal (`fork`, neue
//! Sitzung) und wechselt ins Arbeitsverzeichnis. Das muss geschehen, bevor
//! ein Thread entsteht: `fork` behält nur den aufrufenden Thread.
//!
//! Datenfluss: `main` → [`daemonize()`] → Kindprozess läuft weiter, der
//! Elternprozess endet mit 0; die PID-Datei schreibt das Kind.
//!
//! Bareos-Gegenstück: `lib/daemon.cc` `daemon_start`.
//! Herkunft: `ofd/src/main.rs` und `odir/src/main.rs` (beide mit dem Crate
//! `daemonize`, ohne Pflege seit 2023, RUSTSEC-2025-0069); seit S-58 der
//! eigene Start mit `libc`: zweimal `fork` mit `setsid` dazwischen, `umask`
//! 027, Standardkanäle nach `/dev/null`, PID-Datei vom Enkel.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `Err` text of the last failed libc call, prefixed with its name.
fn os_error(call: &str) -> String {
    format!("{call}: {}", std::io::Error::last_os_error())
}

/// Goes to the background with `working_directory`; writes `pid_file` from
/// the child if given. `Err` is the text for `FATAL: daemonize failed: …`.
///
/// Must run while the process has one thread: `fork` keeps only the caller.
pub fn daemonize(working_directory: &Path, pid_file: Option<&Path>) -> Result<(), String> {
    let dir = CString::new(working_directory.as_os_str().as_bytes())
        .map_err(|_| "working directory contains a NUL byte".to_string())?;
    // SAFETY: only async-signal-safe libc calls run between fork and the
    // return; the process is single-threaded here (see above).
    unsafe {
        // First fork: the parent leaves, the child is no group leader.
        match libc::fork() {
            -1 => return Err(os_error("fork")),
            0 => {}
            _ => libc::_exit(0),
        }
        if libc::setsid() == -1 {
            return Err(os_error("setsid"));
        }
        // Second fork: the daemon can never get a controlling terminal again.
        match libc::fork() {
            -1 => return Err(os_error("fork")),
            0 => {}
            _ => libc::_exit(0),
        }
        libc::umask(0o027);
        if libc::chdir(dir.as_ptr()) == -1 {
            return Err(os_error("chdir"));
        }
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null == -1 {
            return Err(os_error("open /dev/null"));
        }
        for fd in 0..=2 {
            if libc::dup2(null, fd) == -1 {
                return Err(os_error("dup2"));
            }
        }
        if null > 2 {
            libc::close(null);
        }
    }
    if let Some(pid_file) = pid_file {
        std::fs::write(pid_file, format!("{}\n", std::process::id()))
            .map_err(|e| format!("unable to write pid file {}: {e}", pid_file.display()))?;
    }
    Ok(())
}
