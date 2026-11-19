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
//! `daemonize`).

use std::path::Path;

/// Goes to the background with `working_directory`; writes `pid_file` from
/// the child if given. `Err` is the text for `FATAL: daemonize failed: …`.
pub fn daemonize(working_directory: &Path, pid_file: Option<&Path>) -> Result<(), String> {
    let mut daemon = daemonize::Daemonize::new().working_directory(working_directory);
    if let Some(pid_file) = pid_file {
        daemon = daemon.pid_file(pid_file);
    }
    daemon.start().map_err(|e| e.to_string())
}
