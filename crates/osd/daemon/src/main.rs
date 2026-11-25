// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! # osd – der Onesimus Storage Daemon (Programm)
//!
//! Zweck: osd ersetzt bareos-sd (Plan `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`).
//! Stand P1 (abgeschlossen): Backup, Restore und Verify auf bzw. von
//! File-Devices, mit aktivem und passivem Client, im Bareos-Volume-Format;
//! zu odir und ofd über gRPC mit TLS ([`grpc`], `onesimus.stored.StorageDaemon`),
//! zu bareos-dir und bareos-fd über BNET. Anleitung und Abweichungen:
//! `docs/wiki/osd.md`.
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
//! | `--disable-grpc` | keinen gRPC-Port öffnen, nur BNET (wie ofd) |
//! | `--grpc-listen <Adresse:Port>` | gRPC-Port statt osd.d `Grpc Address`/`Grpc Port` (Vorgabe `SD Port` + 1000); muss starten |
//! | `--grpc-cert <PEM>`, `--grpc-key <PEM>` | eigenes TLS-Zertifikat statt des erzeugten `onesimus-sd-grpc.crt` |
//! | `--device-writer <Ring>` | intern: Geräteschreiber eines Jobs (Kindprozess, D-13) |
//! | `--device-reader <Ring>` | intern: Geräteleser eines Restore- oder Verify-Jobs (Kindprozess) |
//!
//! Exit-Codes: 0 Ende nach Signal bzw. Konfiguration gut (`-t`),
//! 1 Konfigurationsfehler (`-t`) oder Port nicht zu binden,
//! 41 falsche Argumente, 42 Konfigurationsfehler beim Start.
//!
//! Datenfluss: Argumente → [`daemon_crate::logging`] →
//! [`bareos_config::sd::load`] → verwaiste Ringe löschen → Port binden →
//! gRPC-Port ([`grpc`]) → [`daemon_crate::accept::serve`] → je
//! Verbindung (BNET oder gRPC-Strom) [`server`] →
//! [`director`] bzw. FD-Anschluss → `run` → [`fd_session`] (Erzeuger) →
//! Ring (`osd-ring`) → [`writer`] (Kindprozess) → Volume ([`device`]).
//! Lesen: `read data` → [`read_session`] (Verbraucher) ← Ring ←
//! [`reader`] (Kindprozess) ← Volume.
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
mod grpc;
mod health;
mod jobs;
mod read_session;
mod reader;
mod ringproto;
mod server;
mod share;
mod status;
mod transport;
mod writer;

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

/// Set on shutdown: the accept loop, the gRPC server and every gRPC
/// stream end.
pub(crate) static SHUTDOWN: AtomicBool = AtomicBool::new(false);

use clap::Parser;

/// Command line of osd: the arguments of bareos-sd plus the internal
/// writer mode.
#[derive(Parser, Debug)]
#[command(name = "osd", about = "Onesimus Storage Daemon", disable_version_flag = true)]
struct Cli {
    /// Use <path> as configuration file or directory.
    #[arg(short, long, default_value = "/etc/bareos")]
    config: PathBuf,

    /// Overlay directory: its `osd.d/` extends the Bareos files (`-c`) with Onesimus directives.
    #[arg(long, value_name = "DIR", default_value = "/etc/onesimus")]
    overlay: PathBuf,

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

    /// Log file (also stderr); `none` for stderr only.
    #[arg(long, value_name = "PATH", default_value = "/var/log/onesimus/osd.log")]
    log_file: PathBuf,

    /// Do not start the gRPC listener (BNET only).
    #[arg(long, conflicts_with = "grpc_listen")]
    disable_grpc: bool,

    /// TLS gRPC listener (default: osd.d Grpc Address/Grpc Port, else
    /// SD Address and SD Port + 1000); BNET always remains available.
    #[arg(long, value_name = "ADDRESS:PORT")]
    grpc_listen: Option<SocketAddr>,

    /// PEM server certificate for gRPC (default: generated in the working directory).
    #[arg(long, requires = "grpc_key")]
    grpc_cert: Option<PathBuf>,

    /// PEM private key for gRPC.
    #[arg(long, requires = "grpc_cert")]
    grpc_key: Option<PathBuf>,

    /// Internal: run as the device writer of ring <RING> (started by osd).
    #[arg(long, value_name = "RING", hide = true)]
    device_writer: Option<String>,

    /// Internal: run as the device reader of ring <RING> (started by osd).
    #[arg(long, value_name = "RING", hide = true)]
    device_reader: Option<String>,
}

/// Starts the gRPC listener unless disabled (`--disable-grpc`, osd.d
/// `Grpc Enable = no`). As in ofd: a listener asked for explicitly
/// (`--grpc-listen`) must start, the default one only warns.
fn start_grpc(cli: &Cli, d: &daemon::Daemon) {
    let storage = &d.config.storage;
    if cli.disable_grpc || !storage.grpc_enable {
        tracing::info!("osd: gRPC disabled, BNET only");
        return;
    }
    let explicit = cli.grpc_listen.is_some();
    let address = match cli.grpc_listen {
        Some(a) => Ok(a),
        None => format!("{}:{}", storage.grpc_address(), storage.grpc_port())
            .parse::<SocketAddr>()
            .map_err(|e| format!("Grpc Address {}: {e}", storage.grpc_address())),
    };
    let started = address.and_then(|address| {
        let (cert, key) = grpc::certificate(d, cli.grpc_cert.as_deref(), cli.grpc_key.as_deref())?;
        grpc::start(d.clone(), address, &cert, &key)
    });
    match started {
        Ok((address, _thread)) => tracing::info!("osd: gRPC (TLS) listening on {address}"),
        Err(e) if explicit => {
            eprintln!("osd: cannot start the gRPC listener: {e}");
            std::process::exit(daemon_crate::exit::EXIT_FAILURE);
        }
        Err(e) => tracing::warn!("osd: gRPC listener not started, BNET only: {e}"),
    }
}

/// The line `-t` prints for a good configuration.
fn summary(config: &bareos_config::sd::SdConfig) -> String {
    let distributors = match config.distributors.len() {
        0 => String::new(),
        n => format!(", {n} Distributor(s)"),
    };
    format!(
        "osd: configuration OK: Storage \"{}\" (port {}), {} Director(s), {} Device(s), {} Autochanger(s){distributors}",
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
            std::process::exit(daemon_crate::exit::EXIT_ARGUMENTS);
        }
        Err(e) => {
            let _ = e.print();
            std::process::exit(daemon_crate::exit::EXIT_SUCCESS);
        }
    };
    bareos_config::onesimus::set_dir(cli.overlay.clone());
    let level = if cli.verbose { cli.debug_level.max(1) } else { cli.debug_level };
    let log_file = daemon_crate::logging::log_file_arg(&cli.log_file);
    daemon_crate::logging::init(&daemon_crate::logging::default_filter(level, &["hyper", "h2"]), log_file.as_deref());

    if let Some(ring) = &cli.device_writer {
        std::process::exit(writer::run(ring));
    }
    if let Some(ring) = &cli.device_reader {
        std::process::exit(reader::run(ring));
    }

    let loaded = bareos_config::sd::load(&cli.config)
        .map_err(|e| format!("osd: configuration error in {}: {e}", cli.config.display()));

    if cli.test_config {
        let code = daemon_crate::config_check::report(loaded.as_ref().map(summary).map_err(Clone::clone), 1);
        std::process::exit(code);
    }

    let config = match loaded {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(daemon_crate::exit::EXIT_CONFIG_ERROR);
        }
    };
    tracing::debug!("{}", summary(&config));
    if !cli.foreground {
        tracing::info!("osd runs in the foreground (daemonizing follows with the packaging)");
    }
    daemon_crate::signals::block_shutdown_signals();

    let removed = osd_ring::Ring::remove_stale(&ringproto::ring_prefix(config.storage.port));
    for name in removed {
        tracing::info!("removed stale ring {name}");
    }
    // SD Addresses (every entry), SD Address, else every interface of both families (like bareos-sd)
    let mut listeners: Vec<TcpListener> = Vec::new();
    for addr in &config.storage.listen {
        match bareos_bnet::listen::bind(*addr) {
            Ok(l) => listeners.push(l),
            Err(e) => {
                eprintln!("osd: cannot bind {addr}: {e} (another Storage daemon running? ss -ltnp)");
                std::process::exit(daemon_crate::exit::EXIT_FAILURE);
            }
        }
        tracing::info!("osd {}: listening on {addr}", daemon_crate::version::VERSION);
    }
    let launcher = match std::env::current_exe() {
        Ok(exe) => daemon::Launcher::Process(exe),
        Err(_) => daemon::Launcher::Thread,
    };
    let d = daemon::Daemon::new(config, launcher);
    start_grpc(&cli, &d);
    // one accept loop per address; the session numbers stay unique across them
    for listener in listeners {
        let d = d.clone();
        std::thread::Builder::new()
            .name("accept".into())
            .spawn(move || {
                daemon_crate::accept::serve(listener, "sd:session", &SHUTDOWN, move |stream, n| {
                    server::handle_connection(d.clone(), stream, n);
                });
            })
            .unwrap_or_else(|e| {
                eprintln!("osd: cannot start the accept thread: {e}");
                std::process::exit(daemon_crate::exit::EXIT_FAILURE);
            });
    }
    let signal = daemon_crate::signals::wait_for_shutdown();
    tracing::info!("osd: shutting down ({signal:?})");
    SHUTDOWN.store(true, std::sync::atomic::Ordering::Relaxed);
    std::process::exit(daemon_crate::exit::EXIT_SUCCESS);
}
