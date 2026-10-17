//! baReos File Daemon — Rust rewrite of the Bareos File Daemon.
//!
//! Named threads, TLS-PSK authentication, BNET wire protocol.
//! Drop-in replacement for the C++ bareos-fd.
//!
//! Thread model:
//! - fd:accept    — TCP listener, spawns session threads
//! - fd:session:N — one per Director connection (status, backup, restore)
//! - fd:heartbeat — periodic keepalive

mod accept;
mod accurate;
mod acl_xattr;
mod attribs;
mod backup;
mod berrno;
mod breg;
mod compression;
mod config;
mod create_file;
mod digest;
mod fileset;
mod find;
mod heartbeat;
mod jmsg;
mod job;
mod restore;
mod runscript;
mod sd;
mod status;
mod streams;
mod version;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use clap::Parser;
use tracing::{info, error, warn};
use tracing_subscriber::EnvFilter;

/// Global shutdown flag.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// baReos File Daemon — Backup Archiving REcovery Open Sourced
#[derive(Parser, Debug)]
#[command(
    name = "bareos-fd",
    version,
    about = "The baReos File Daemon.\n\nCopyright (C) 2026 Onesomus.io — Rust Version\nBased on Bareos by Bareos GmbH & Co. KG"
)]
struct Cli {
    /// Use <path> as configuration file or directory.
    #[arg(short, long, default_value = "/etc/bareos/bareos-fd.d")]
    config: PathBuf,

    /// Set debug level to <level>.
    #[arg(short, long, default_value_t = 0)]
    debug_level: u32,

    /// Run in foreground.
    #[arg(short, long)]
    foreground: bool,

    /// Run as given user.
    #[arg(short, long)]
    user: Option<String>,

    /// Run as given group.
    #[arg(short, long)]
    group: Option<String>,

    /// Test - read configuration and exit.
    #[arg(short, long)]
    test_config: bool,

    /// No signals (for debugging).
    #[arg(short = 's', long)]
    no_signals: bool,

    /// Verbose user messages.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() {
    let cli = Cli::parse();

    // Logging
    let filter = if cli.debug_level > 0 {
        "debug,hyper=info,h2=info".to_string()
    } else {
        "info,hyper=warn,h2=warn".to_string()
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&filter)),
        )
        .init();

    let _ = rustls::crypto::ring::default_provider().install_default();

    bareos_bnet::debug::set_debug_level(cli.debug_level);
    bareos_bnet::debug::set_daemon_name("bareos-fd");

    info!("baReos File Daemon Version {} starting", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let fd_config = match config::FdConfig::load(&cli.config) {
        Ok(c) => {
            info!("loaded configuration from {}", cli.config.display());
            c
        }
        Err(e) => {
            error!("failed to load config {}: {e}", cli.config.display());
            std::process::exit(1);
        }
    };

    if cli.test_config {
        println!("configuration OK: FD '{}', port {}, {} director(s)",
            fd_config.name, fd_config.port, fd_config.directors.len());
        std::process::exit(0);
    }

    // Privilege drop
    #[cfg(unix)]
    {
        if let Some(ref group_name) = cli.group {
            if let Ok(Some(group)) = nix::unistd::Group::from_name(group_name) {
                nix::unistd::setgid(group.gid).ok();
                info!("switched to group {group_name}");
            }
        }
        if let Some(ref user_name) = cli.user {
            if let Ok(Some(user)) = nix::unistd::User::from_name(user_name) {
                nix::unistd::setuid(user.uid).ok();
                info!("switched to user {user_name}");
            }
        }
    }

    // Daemonize
    if !cli.foreground {
        info!("daemonizing...");
        match daemonize::Daemonize::new()
            .working_directory("/var/lib/bareos")
            .start()
        {
            Ok(_) => info!("daemon started"),
            Err(e) => {
                error!("FATAL: daemonize failed: {e}");
                std::process::exit(1);
            }
        }
    }

    let port = fd_config.port;
    let shared_config = Arc::new(fd_config);

    info!("FD '{}' listening on port {}", shared_config.name, port);

    // fd:accept thread
    let cfg = shared_config.clone();
    let accept_handle = thread::Builder::new()
        .name("fd:accept".into())
        .spawn(move || {
            accept::accept_loop(&cfg, &SHUTDOWN);
        })
        .expect("failed to spawn fd:accept");

    // fd:heartbeat thread
    let heartbeat_handle = thread::Builder::new()
        .name("fd:heartbeat".into())
        .spawn(move || {
            heartbeat::heartbeat_loop(&SHUTDOWN);
        })
        .expect("failed to spawn fd:heartbeat");

    // Wait for shutdown
    wait_for_shutdown();

    info!("baReos File Daemon shutting down");
    SHUTDOWN.store(true, Ordering::SeqCst);

    accept_handle.join().ok();
    heartbeat_handle.join().ok();

    info!("baReos File Daemon stopped");
}

fn wait_for_shutdown() {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let _ = ctrlc::set_handler(move || { let _ = tx.send(()); });
    let _ = rx.recv();
}
