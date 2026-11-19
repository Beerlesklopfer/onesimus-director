//! # onesimus-daemon – das gemeinsame Daemon-Gerüst
//!
//! Zweck: Was jeder Onesimus-Daemon beim Start und beim Ende gleich tut,
//! steht hier einmal: Logging einrichten, Konfiguration mit `-t` prüfen,
//! Rechte abgeben (`-u`/`-g`), in den Hintergrund gehen (ohne `-f`),
//! PID-Datei schreiben, auf `SIGTERM`/`SIGINT` warten, `SIGHUP` an einen
//! eigenen Thread leiten, mit den Bareos-Exit-Codes enden. ofd nutzt es seit
//! 19.11.2026; der neue osd (`crates/osd/daemon`) soll es von Anfang an nutzen.
//!
//! Datenfluss eines Starts (Reihenfolge, wie `main` sie aufruft):
//!
//! 1. [`logging::init`] – vor allem anderen, damit Fehler sichtbar sind.
//! 2. Konfiguration laden; mit `-t` [`config_check::report`] und Ende.
//! 3. [`signals::block_shutdown_signals`] / [`signals::block_sighup`] – bevor
//!    ein Thread entsteht, sonst erbt ihn jeder Thread unblockiert.
//! 4. [`privileges`] – vor dem Hintergrund-Start und vor Threads.
//! 5. [`background::daemonize`] oder [`pidfile::write`] im Vordergrund.
//! 6. Threads starten, dann [`signals::wait_for_shutdown`] bzw.
//!    [`signals::wait_for_interrupt`].
//!
//! Bareos-Gegenstück: `core/src/lib/daemon.cc` (`daemon_start`),
//! `lib/bsys.cc` (`drop`, `CreatePidFile`, `DeletePidFile`),
//! `lib/signal.cc` (`InitSignals`), `include/exit_codes.h`, `filed/filed.cc`
//! und `stored/stored.cc` (`main`).
//!
//! Herkunft: `ofd/src/main.rs` (Logging, Rechteabgabe, Hintergrund, Warten)
//! und `odir/src/privileges.rs`, `odir/src/main.rs` (PID-Datei,
//! Signalmasken, `-t`-Exit-Codes). Begriffe: [`glossary`].
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod background;
pub mod config_check;
pub mod exit;
pub mod glossary;
pub mod logging;
pub mod pidfile;
pub mod privileges;
#[cfg_attr(doc, aquamarine::aquamarine)]
/// Signale: Abschalten (`SIGTERM`, `SIGINT`) und Neuladen (`SIGHUP`).
///
/// ```mermaid
/// classDiagram
///     class main {
///         block_shutdown_signals()
///         block_sighup()
///         wait_for_shutdown() Signal
///     }
///     class SighupThread {
///         +spawn_sighup_thread(name, reload)
///         sigwait(SIGHUP)
///     }
///     class Daemonthreads {
///         erben die blockierte Maske
///     }
///     main --> SighupThread : startet
///     main --> Daemonthreads : startet nach dem Blockieren
///     main ..> main : sigwait(SIGTERM, SIGINT)
/// ```
pub mod signals;
