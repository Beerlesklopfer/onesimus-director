//! # onesimus-daemon – das gemeinsame Daemon-Gerüst
//!
//! Zweck: Was jeder Onesimus-Daemon beim Start und beim Ende gleich tut,
//! steht hier einmal: Logging einrichten, Konfiguration mit `-t` prüfen,
//! Rechte abgeben (`-u`/`-g`), in den Hintergrund gehen (ohne `-f`),
//! PID-Datei schreiben, auf `SIGTERM`/`SIGINT` warten, `SIGHUP` an einen
//! eigenen Thread leiten, mit den Bareos-Exit-Codes enden. Dazu kommt seit
//! 20.11.2026 die Netzseite: Verbindungen annehmen ([`accept`], TLS-PSK mit
//! Schlüsselwahl nach PSK-Identität), den Director anmelden ([`handshake`]),
//! und die Bandbreitenbegrenzung ([`bwlimit`]). ofd nutzt es seit 19.11.2026; der neue osd
//! (`crates/osd/daemon`) nutzt es von Anfang an.
//!
//! **Folgt in P1** (zu eng mit ofd verflochten, bleibt vorerst dort):
//! die Tabelle laufender Jobs und der passive SD-Anschluss
//! (`ofd/src/sd.rs` `JobShared`, `handle_stored_connection`), das Binden des Ports mit den Fehlertexten der FD-Konfiguration
//! (`ofd/src/accept.rs` `accept_loop`), die TLS-Fassung des Directors
//! (`odir/src/grpc/tls.rs`, mit mDNS-Namen) sowie Signale, PID-Datei und
//! `-k` in odir.
//!
//! Der gRPC-Server mit Laufzeit und Strompumpe (früher `ofd/src/grpc.rs`
//! `start`) liegt seit 20.11.2026 in `onesimus-bnet-grpc` (`server::spawn`,
//! `server::accept`), gemeinsam für ofd und osd; die TLS-Einstellung
//! (früher `grpc_tls` hier) seit 21.11.2026 ebenfalls dort (`tls`), weil sie
//! keine Bareos-Ableitung ist und mit dem gRPC-Crate dual lizenziert ist
//! (docs/LIZENZEN.md).
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
//! Netzseite (`core/src/lib/bnet_server_tcp.cc`, `filed/authenticate.cc`,
//! `stored/authenticate.cc`, `lib/bsock.cc` `ControlBwlimit`): siehe die
//! Module.
//!
//! Herkunft: `ofd/src/accept.rs`, `ofd/src/grpc_tls.rs`, `ofd/src/sd.rs`
//! (`Rw`, `BwState`, `Limited`), `ofd/src/main.rs` (Logging, Rechteabgabe, Hintergrund, Warten)
//! und `odir/src/privileges.rs`, `odir/src/main.rs` (PID-Datei,
//! Signalmasken, `-t`-Exit-Codes). Begriffe: [`glossary`].
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod accept;
pub mod background;
pub mod bwlimit;
pub mod config_check;
pub mod exit;
pub mod glossary;
pub mod handshake;
pub mod logging;
pub mod pidfile;
pub mod privileges;
pub mod version;
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
