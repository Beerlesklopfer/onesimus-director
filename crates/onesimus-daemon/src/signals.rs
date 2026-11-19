//! Signale: `SIGTERM`/`SIGINT` beenden, `SIGHUP` lädt neu.
//!
//! Zweck: Ein Daemon soll auf `systemctl stop` (`SIGTERM`) und Strg-C
//! (`SIGINT`) sauber enden und auf `systemctl reload` (`SIGHUP`) seine
//! Konfiguration neu lesen. Zwei Verfahren stehen bereit:
//!
//! - **Signalmaske + `sigwait`** (odir, Vorlage für osd): `main` blockiert
//!   die Signale, bevor ein Thread entsteht; alle Threads erben die Maske.
//!   `main` wartet mit [`wait_for_shutdown`], ein eigener Thread mit
//!   [`spawn_sighup_thread`]. Kein Signal unterbricht einen Arbeitsthread.
//! - **Handler** (ofd heute, [`wait_for_interrupt`]): ein `ctrlc`-Handler
//!   weckt `main`. Er fängt nur `SIGINT`; `SIGTERM` beendet ofd sofort mit
//!   der Standardaktion. Das ist ofds Verhalten vor dem 19.11.2026 und
//!   bleibt bis zu einer bewussten Umstellung (offener Punkt).
//!
//! Datenfluss: Kernel → blockiertes Signal → `sigwait` in `main` bzw. im
//! SIGHUP-Thread → Abschalt-Flag bzw. Neulade-Funktion des Daemons.
//!
//! Bareos-Gegenstück: `lib/signal.cc` `InitSignals` (Handler für
//! `SIGTERM` → `Terminate…`), `dird/dird.cc` `ReloadConfig` an `SIGHUP`.

use nix::sys::signal::{SigSet, Signal};

/// The shutdown signals `SIGINT` and `SIGTERM`.
pub fn shutdown_sigset() -> SigSet {
    let mut set = SigSet::empty();
    set.add(Signal::SIGINT);
    set.add(Signal::SIGTERM);
    set
}

/// Blocks `SIGINT`/`SIGTERM` in the calling thread and every thread it
/// starts later; call it in `main` before the first thread.
pub fn block_shutdown_signals() {
    if let Err(e) = shutdown_sigset().thread_block() {
        tracing::error!("cannot block SIGINT/SIGTERM: {e}");
    }
}

/// Waits for `SIGINT` or `SIGTERM` (blocked before with
/// [`block_shutdown_signals`]); returns the signal, `None` if `sigwait` fails.
pub fn wait_for_shutdown() -> Option<Signal> {
    match shutdown_sigset().wait() {
        Ok(sig) => {
            tracing::info!("{sig} received, shutting down");
            Some(sig)
        }
        Err(e) => {
            tracing::error!("sigwait failed: {e}");
            None
        }
    }
}

/// Blocks `SIGHUP` in the calling thread and its future threads, so that
/// only the thread of [`spawn_sighup_thread`] receives it.
pub fn block_sighup() {
    let mut set = SigSet::empty();
    set.add(Signal::SIGHUP);
    if let Err(e) = set.thread_block() {
        tracing::error!("cannot block SIGHUP: {e}");
    }
}

/// Starts thread `name` that calls `reload` for every `SIGHUP`.
pub fn spawn_sighup_thread<F>(name: &str, mut reload: F) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnMut() + Send + 'static,
{
    std::thread::Builder::new().name(name.into()).spawn(move || {
        let mut set = SigSet::empty();
        set.add(Signal::SIGHUP);
        set.thread_block().ok();
        loop {
            match set.wait() {
                Ok(Signal::SIGHUP) => {
                    tracing::info!("SIGHUP received");
                    reload();
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::error!("sigwait error: {e}");
                    break;
                }
            }
        }
    })
}

/// ofd's wait before 19.11.2026: a `ctrlc` handler (`SIGINT` only) wakes
/// the caller. Returns when the signal arrives.
pub fn wait_for_interrupt() {
    let (tx, rx) = std::sync::mpsc::channel();
    let _ = ctrlc::set_handler(move || {
        let _ = tx.send(());
    });
    let _ = rx.recv();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_set_has_int_and_term_only() {
        let set = shutdown_sigset();
        assert!(set.contains(Signal::SIGINT) && set.contains(Signal::SIGTERM));
        assert!(!set.contains(Signal::SIGHUP));
    }

    // No test sends a real SIGHUP: in the multi-threaded test runner a
    // process-directed signal may reach a thread that does not block it.
}
