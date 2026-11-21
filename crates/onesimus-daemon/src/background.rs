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
//! eigene Start mit `nix`: zweimal `fork` mit `setsid` dazwischen, `umask`
//! 027, Standardkanäle nach `/dev/null`, PID-Datei vom Enkel.

use std::os::fd::AsRawFd;
use std::path::Path;

use nix::fcntl::{open, OFlag};
use nix::sys::stat::{umask, Mode};
use nix::unistd::{chdir, dup2, fork, setsid, ForkResult};

/// Goes to the background with `working_directory`; writes `pid_file` from
/// the child if given. `Err` is the text for `FATAL: daemonize failed: …`.
///
/// Must run while the process has one thread: `fork` keeps only the caller.
pub fn daemonize(working_directory: &Path, pid_file: Option<&Path>) -> Result<(), String> {
    // SAFETY: the process is single-threaded here (see above); between the
    // forks only async-signal-safe calls run.
    let fork_once = || match unsafe { fork() } {
        Ok(ForkResult::Parent { .. }) => std::process::exit(0),
        Ok(ForkResult::Child) => Ok(()),
        Err(e) => Err(format!("fork: {e}")),
    };
    // First fork: the parent leaves, the child is no group leader.
    fork_once()?;
    setsid().map_err(|e| format!("setsid: {e}"))?;
    // Second fork: the daemon can never get a controlling terminal again.
    fork_once()?;
    umask(Mode::from_bits_truncate(0o027));
    chdir(working_directory).map_err(|e| format!("chdir: {e}"))?;
    let null = open("/dev/null", OFlag::O_RDWR, Mode::empty())
        .map_err(|e| format!("open /dev/null: {e}"))?;
    for fd in 0..=2 {
        dup2(null.as_raw_fd(), fd).map_err(|e| format!("dup2: {e}"))?;
    }
    if let Some(pid_file) = pid_file {
        std::fs::write(pid_file, format!("{}\n", std::process::id()))
            .map_err(|e| format!("unable to write pid file {}: {e}", pid_file.display()))?;
    }
    Ok(())
}
