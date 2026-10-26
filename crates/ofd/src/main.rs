//! Onesimus File Daemon — Rust rewrite of the Bareos File Daemon.
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
mod grpc;
mod jmsg;
mod job;
mod restore;
mod runscript;
mod scan;
mod sd;
mod status;
mod streams;
mod version;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use clap::Parser;
use tracing::{info, error};
use tracing_subscriber::EnvFilter;

/// Global shutdown flag.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum PluginCheckOperation { Backup, Restore }

/// Onesimus File Daemon — Backup Archiving REcovery Open Sourced
#[derive(Parser, Debug)]
#[command(
    name = "ofd",
    version,
    about = "The Onesimus File Daemon.\n\nCopyright (C) 2026 Onesimus.io — Rust Version\nBased on Bareos by Bareos GmbH & Co. KG"
)]
struct Cli {
    /// Verify and initialize a signed WASM bundle, then exit without running a job.
    #[arg(long, requires = "plugin_policy", conflicts_with_all = ["discover_podman", "test_config", "foreground", "grpc_listen"])]
    check_plugin: Option<PathBuf>,

    /// Public X.509 trust policy JSON for --check-plugin (no private keys).
    #[arg(long, requires = "check_plugin")]
    plugin_policy: Option<PathBuf>,

    /// Job profile for --check-plugin initialization (default: backup).
    #[arg(long, requires = "check_plugin", value_enum)]
    plugin_check_operation: Option<PluginCheckOperation>,

    /// Read-only host-UID discovery via conmon; print JSON and exit.
    #[arg(long)]
    discover_podman: bool,

    /// Trusted root-owned conmon executable (repeatable); used for discovery only.
    #[arg(long, requires = "discover_podman")]
    conmon_executable: Vec<PathBuf>,

    /// Additional TLS gRPC control listener; BNET always remains available.
    #[arg(long, requires_all = ["grpc_cert", "grpc_key"])]
    grpc_listen: Option<std::net::SocketAddr>,

    /// PEM server certificate for --grpc-listen.
    #[arg(long, requires = "grpc_listen")]
    grpc_cert: Option<PathBuf>,

    /// PEM private key for --grpc-listen.
    #[arg(long, requires = "grpc_listen")]
    grpc_key: Option<PathBuf>,

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
    if let Some(path) = cli.check_plugin.as_deref() {
        if let Err(error) = check_plugin(path, cli.plugin_policy.as_deref().unwrap(),
            cli.plugin_check_operation.unwrap_or(PluginCheckOperation::Backup)) {
            eprintln!("Plugin check: {error}"); std::process::exit(1);
        }
        return;
    }
    if cli.discover_podman {
        let paths = if cli.conmon_executable.is_empty() {
            onesimus_plugins::discovery::installed_conmon_paths()
        } else { cli.conmon_executable.clone() };
        let result = onesimus_plugins::discovery::Scanner::new("/proc", &paths).and_then(|scanner| scanner.scan());
        match result {
            Ok(report) => println!("{}", serde_json::to_string_pretty(&report).expect("discovery JSON")),
            Err(error) => { eprintln!("Podman discovery: {error}"); std::process::exit(1); }
        }
        return;
    }

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
    bareos_bnet::debug::set_daemon_name("ofd");

    info!("Onesimus File Daemon Version {} starting", env!("CARGO_PKG_VERSION"));

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

    let grpc_handle = cli.grpc_listen.map(|address| {
        grpc::start(shared_config.clone(), address, cli.grpc_cert.as_deref().unwrap(),
            cli.grpc_key.as_deref().unwrap(), &SHUTDOWN).unwrap_or_else(|error| {
                error!("cannot start FD gRPC listener: {error}");
                std::process::exit(1);
            })
    });

    info!("FD '{}' listening on BNET port {} (fallback always enabled)", shared_config.name, port);

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

    info!("Onesimus File Daemon shutting down");
    SHUTDOWN.store(true, Ordering::SeqCst);

    accept_handle.join().ok();
    heartbeat_handle.join().ok();
    if let Some((_, handle)) = grpc_handle { handle.join().ok(); }

    info!("Onesimus File Daemon stopped");
}

fn wait_for_shutdown() {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    let _ = ctrlc::set_handler(move || { let _ = tx.send(()); });
    let _ = rx.recv();
}

fn check_plugin(path: &std::path::Path, policy: &std::path::Path,
    operation: PluginCheckOperation) -> Result<(), Box<dyn std::error::Error>> {
    use onesimus_plugins::{bundle, executor::{Binding, Host, Limits}, wasm::{self, LoadOptions, RuntimeLimits}};
    use onesimus_proto_rs::onesimus::filed::plugin::v1::{JobContext, JobOperation};
    let runtime = RuntimeLimits::default();
    let bundle = bundle::read_bundle(path, runtime.max_component_bytes)?;
    let policy = bundle::read_policy(policy)?;
    let manifest: onesimus_plugins::Manifest = serde_json::from_slice(&bundle.manifest_bytes)?;
    let host = Host::new(1)?;
    let mut worker = wasm::load_signed(&host, bundle, &policy, LoadOptions {
        binding: Binding { instance_id: "plugin-check".into(), job_id: 1,
            device_id: "plugin-check".into(), worker_id: "plugin-check".into() },
        job: JobContext { job_id: 1, job_name: "plugin-check".into(), operation: match operation {
            PluginCheckOperation::Backup => JobOperation::Backup, PluginCheckOperation::Restore => JobOperation::Restore,
        } as i32 }, options: Default::default(),
        worker: Limits { max_pending_operations: 1, max_request_bytes: 256 * 1024,
            max_response_bytes: 256 * 1024, operation_timeout: std::time::Duration::from_secs(5) }, runtime,
    })?;
    worker.shutdown(std::time::Duration::from_secs(2))?;
    println!("Plugin {} {}: X.509, signature, component ABI and initialization OK", manifest.plugin_id, manifest.version);
    Ok(())
}

#[cfg(test)]
mod plugin_cli_tests {
    use super::*;
    #[test]
    fn plugin_check_requires_policy_and_cannot_start_a_daemon_or_discovery() {
        assert!(Cli::try_parse_from(["ofd", "--check-plugin", "bundle.pb"]).is_err());
        assert!(Cli::try_parse_from(["ofd", "--plugin-policy", "policy.json"]).is_err());
        for conflicting in ["--discover-podman", "--foreground", "--test-config"] {
            assert!(Cli::try_parse_from(["ofd", "--check-plugin", "bundle.pb", "--plugin-policy", "policy.json", conflicting]).is_err());
        }
        let cli = Cli::try_parse_from(["ofd", "--check-plugin", "bundle.pb", "--plugin-policy", "policy.json",
            "--plugin-check-operation", "restore"]).unwrap();
        assert!(matches!(cli.plugin_check_operation, Some(PluginCheckOperation::Restore)));
    }
}
