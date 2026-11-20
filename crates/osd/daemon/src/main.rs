//! # osd – der Onesimus Storage Daemon (Programm)
//!
//! Zweck: osd soll bareos-sd ersetzen (Plan
//! `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`). Stand P0: Das Programm
//! nimmt die Argumente von bareos-sd an, liest `bareos-sd.d` mit
//! `bareos-config` und prüft die Konfiguration mit `-t`. Ohne `-t` meldet
//! es „osd: P1 pending“ und endet mit 2; Netz, Geräte und Volumes kommen
//! in P1.
//!
//! Argumente (wie `stored/stored.cc`):
//!
//! | Argument | Bedeutung |
//! |---|---|
//! | `-c <Pfad>` | Konfiguration: Datei, Verzeichnis mit `bareos-sd.d` oder `bareos-sd.d` selbst (Vorgabe `/etc/bareos`) |
//! | `-f` | im Vordergrund bleiben |
//! | `-d <n>` | Debug-Level; ab 1 Logging auf `debug` |
//! | `-t` | Konfiguration prüfen, Exit 0 (gut) oder 1 (Fehler) |
//! | `-v` | ausführliche Meldungen |
//!
//! Exit-Codes: 0 Konfiguration gut (`-t`), 1 Konfigurationsfehler (`-t`),
//! 2 Betrieb noch nicht umgesetzt (P1), 41 falsche Argumente.
//!
//! Datenfluss: Argumente → [`onesimus_daemon::logging`] →
//! [`bareos_config::sd::load`] → mit `-t` Bericht und Ende, sonst Ende mit 2.
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

use std::path::PathBuf;

use clap::Parser;

/// Exit code while the daemon itself is not implemented (P1).
const EXIT_P1_PENDING: i32 = 2;

/// Command line of osd, the arguments of bareos-sd that P0 knows.
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

    let loaded = bareos_config::sd::load(&cli.config)
        .map_err(|e| format!("osd: configuration error in {}: {e}", cli.config.display()));

    if cli.test_config {
        let code = onesimus_daemon::config_check::report(loaded.as_ref().map(summary).map_err(Clone::clone), 1);
        std::process::exit(code);
    }

    match loaded {
        Ok(config) => tracing::debug!("{}", summary(&config)),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(onesimus_daemon::exit::EXIT_CONFIG_ERROR);
        }
    }
    tracing::debug!("foreground={}", cli.foreground);
    eprintln!("osd: P1 pending");
    std::process::exit(EXIT_P1_PENDING);
}
