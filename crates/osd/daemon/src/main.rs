//! # osd – der Onesimus Storage Daemon (Programm)
//!
//! Zweck: osd ersetzt bareos-sd (Plan `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`).
//! Stand P1 Stufe 1: Backup über BNET auf ein File-Device, mit aktivem und
//! passivem Client, im Bareos-Volume-Format; Restore folgt in Stufe 2, gRPC
//! in Stufe 3. Anleitung und Abweichungen: `docs/wiki/osd.md`.
//!
//! Argumente (wie `stored/stored.cc`):
//!
//! | Argument | Bedeutung |
//! |---|---|
//! | `-c <Pfad>` | Konfiguration: Datei, Verzeichnis mit `bareos-sd.d` oder `bareos-sd.d` selbst (Vorgabe `/etc/bareos`); `osd.d` daneben erweitert sie (D-8) |
//! | `-f` | im Vordergrund bleiben |
//! | `-d <n>` | Debug-Level; ab 1 Logging auf `debug` |
//! | `-t` | Konfiguration prüfen, Exit 0 (gut) oder 1 (Fehler) |
//! | `-v` | ausführliche Meldungen |
//! | `--device-writer <Ring>` | intern: Geräteschreiber eines Jobs (Kindprozess, D-13) |
//!
//! Exit-Codes: 0 Ende nach Signal bzw. Konfiguration gut (`-t`),
//! 1 Konfigurationsfehler (`-t`) oder Port nicht zu binden,
//! 41 falsche Argumente, 42 Konfigurationsfehler beim Start.
//!
//! Datenfluss: Argumente → [`onesimus_daemon::logging`] →
//! [`bareos_config::sd::load`] → verwaiste Ringe löschen → Port binden →
//! [`onesimus_daemon::accept::serve`] → je Verbindung [`server`] →
//! [`director`] bzw. FD-Anschluss → `run` → [`fd_session`] (Erzeuger) →
//! Ring (`osd-ring`) → [`writer`] (Kindprozess) → Volume ([`device`]).
//!
//! Paket: `osd-daemon` (`crates/osd/daemon`), Programm `osd`; die
//! Schnittstelle zum SD ist das Paket `osd` (`crates/osd`). Mit `-p osd`
//! baut und testet Cargo nur die Schnittstelle (ofd-Gutachten M3/M4), der
//! Daemon braucht `-p osd-daemon`. Das Programm steht nicht in
//! `cargo doc` (`doc = false`, M9).
//!
//! Bareos-Gegenstück: `core/src/stored/stored.cc` (`main`, Argumente,
//! `test_config`).
#![deny(missing_docs)]

mod daemon;
mod device;
mod director;
mod fd_session;
mod jobs;
mod ringproto;
mod server;
mod status;
mod transport;
mod writer;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use clap::Parser;

/// Command line of osd: the arguments of bareos-sd plus the internal
/// writer mode.
#[derive(Parser, Debug)]
#[command(name = "osd", about = "Onesimus Storage Daemon", disable_version_flag = true)]
struct Cli {
    /// Use <path> as configuration file or directory.
    #[arg(short, long, default_value = "/etc/bareos")]
    config: PathBuf,

    /// Run in foreground.
    #[arg(short, long)]
    foreground: bool,

    /// Set debug level to <level>.
    #[arg(short, long, default_value_t = 0)]
    debug_level: u32,

    /// Test configuration and exit.
    #[arg(short, long)]
    test_config: bool,

    /// Print more messages.
    #[arg(short, long)]
    verbose: bool,

    /// Internal: run as the device writer of ring <RING> (started by osd).
    #[arg(long, value_name = "RING", hide = true)]
    device_writer: Option<String>,
}

/// The line `-t` prints for a good configuration.
fn summary(config: &bareos_config::sd::SdConfig) -> String {
    format!(
        "osd: configuration OK: Storage \"{}\" (port {}), {} Director(s), {} Device(s), {} Autochanger(s)",
        config.storage.name,
        config.storage.port,
        config.directors.len(),
        config.devices.len(),
        config.autochangers.len()
    )
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if e.use_stderr() => {
            let _ = e.print();
            std::process::exit(onesimus_daemon::exit::EXIT_ARGUMENTS);
        }
        Err(e) => {
            let _ = e.print();
            std::process::exit(onesimus_daemon::exit::EXIT_SUCCESS);
        }
    };
    let level = if cli.verbose { cli.debug_level.max(1) } else { cli.debug_level };
    onesimus_daemon::logging::init(&onesimus_daemon::logging::default_filter(level, &["hyper", "h2"]));

    if let Some(ring) = &cli.device_writer {
        std::process::exit(writer::run(ring));
    }

    let loaded = bareos_config::sd::load(&cli.config)
        .map_err(|e| format!("osd: configuration error in {}: {e}", cli.config.display()));

    if cli.test_config {
        let code = onesimus_daemon::config_check::report(loaded.as_ref().map(summary).map_err(Clone::clone), 1);
        std::process::exit(code);
    }

    let config = match loaded {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(onesimus_daemon::exit::EXIT_CONFIG_ERROR);
        }
    };
    tracing::debug!("{}", summary(&config));
    if !cli.foreground {
        tracing::info!("osd runs in the foreground (daemonizing follows with the packaging)");
    }
    onesimus_daemon::signals::block_shutdown_signals();

    let removed = osd_ring::Ring::remove_stale(&ringproto::ring_prefix(config.storage.port));
    for name in removed {
        tracing::info!("removed stale ring {name}");
    }
    let addr = format!("{}:{}", config.storage.address.as_deref().unwrap_or("0.0.0.0"), config.storage.port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("osd: cannot bind {addr}: {e} (another Storage daemon running? ss -ltnp)");
            std::process::exit(onesimus_daemon::exit::EXIT_FAILURE);
        }
    };
    tracing::info!("osd {}: listening on {addr}", onesimus_daemon::version::VERSION);
    let launcher = match std::env::current_exe() {
        Ok(exe) => daemon::Launcher::Process(exe),
        Err(_) => daemon::Launcher::Thread,
    };
    let d = daemon::Daemon::new(config, launcher);
    static SHUTDOWN: AtomicBool = AtomicBool::new(false);
    std::thread::Builder::new()
        .name("accept".into())
        .spawn(move || {
            onesimus_daemon::accept::serve(listener, "sd:session", &SHUTDOWN, move |stream, n| {
                server::handle_connection(d.clone(), stream, n);
            });
        })
        .unwrap_or_else(|e| {
            eprintln!("osd: cannot start the accept thread: {e}");
            std::process::exit(onesimus_daemon::exit::EXIT_FAILURE);
        });
    let signal = onesimus_daemon::signals::wait_for_shutdown();
    tracing::info!("osd: shutting down ({signal:?})");
    SHUTDOWN.store(true, std::sync::atomic::Ordering::Relaxed);
    std::process::exit(onesimus_daemon::exit::EXIT_SUCCESS);
}
