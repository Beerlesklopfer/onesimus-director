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
mod broker;
mod compression;
mod config;
mod create_file;
mod digest;
mod fileset;
mod find;
mod heartbeat;
mod grpc;
mod grpc_tls;
mod jmsg;
mod ldap_instance;
mod ldap_port;
#[cfg(test)]
mod ldap_e2e;
mod job;
mod pg_instance;
mod pg_port;
mod plugin;
#[cfg(test)]
mod podman_e2e;
mod podman_port;
mod podman_rootful;
mod podman_scan;
mod restore;
mod runscript;
mod scan;
mod secrets;
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

    /// TLS gRPC control listener (default: [::]:<port + 1000>); BNET always remains available.
    #[arg(long)]
    grpc_listen: Option<std::net::SocketAddr>,

    /// PEM server certificate for gRPC (default: generated in the working directory).
    #[arg(long, requires = "grpc_key")]
    grpc_cert: Option<PathBuf>,

    /// PEM private key for gRPC.
    #[arg(long, requires = "grpc_cert")]
    grpc_key: Option<PathBuf>,

    /// Do not start the gRPC listener (BNET only).
    #[arg(long, conflicts_with = "grpc_listen")]
    disable_grpc: bool,

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
        match check_plugin(path, cli.plugin_policy.as_deref().unwrap(),
            cli.plugin_check_operation.unwrap_or(PluginCheckOperation::Backup)) {
            Ok(report) => println!("{report}"),
            Err(error) => { eprintln!("Plugin check: {error}"); std::process::exit(1); }
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

    // gRPC is the default control channel; it is started with a certificate
    // generated once in the working directory unless one is given. A listener
    // asked for explicitly must start, the default one only warns.
    let grpc_handle = if cli.disable_grpc {
        None
    } else {
        let explicit = cli.grpc_listen.is_some();
        // Default: the BNET address (FD Address) on port + 1000.
        let address = match cli.grpc_listen {
            Some(a) => a,
            None => match shared_config.listen_addresses(port.saturating_add(1000)) {
                Ok(a) => a[0],
                Err(e) => {
                    error!("{e}");
                    std::process::exit(1);
                }
            },
        };
        let files = match (&cli.grpc_cert, &cli.grpc_key) {
            (Some(cert), Some(key)) => Ok((cert.clone(), key.clone())),
            _ => grpc_tls::load_or_generate(&shared_config.working_directory, &shared_config.name),
        };
        match files.and_then(|(cert, key)| grpc::start(shared_config.clone(), address, &cert, &key, &SHUTDOWN)) {
            Ok(started) => {
                info!("FD gRPC (TLS) listening on {}", started.0);
                Some(started)
            }
            Err(error) if explicit => {
                error!("cannot start FD gRPC listener: {error}");
                std::process::exit(1);
            }
            Err(error) => {
                warn!("FD gRPC listener on {address} not started, BNET only: {error}");
                None
            }
        }
    };

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

/// Checks a bundle as a job would: trust base, local policy, signature,
/// manifest (suite) and the hashes of its native parts for this triplet.
/// A plugin that needs a broker ends there (no broker in check mode);
/// any other plugin is also instantiated and initialized.
fn check_plugin(path: &std::path::Path, policy: &std::path::Path,
    operation: PluginCheckOperation) -> Result<String, Box<dyn std::error::Error>> {
    use onesimus_plugins::{bundle, executor::{Binding, Host, Limits}, wasm::{self, LoadOptions, RuntimeLimits}};
    use onesimus_proto_rs::onesimus::filed::plugin::v1::{JobContext, JobOperation};
    let runtime = RuntimeLimits::default();
    let bundle = bundle::read_bundle(path, runtime.max_component_bytes)?;
    let policy = plugin::trust::policy(policy).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let verified = onesimus_plugins::verify(&bundle.manifest_bytes, &bundle.ed25519_signature,
        bundle.artifact_bytes.clone(), &bundle.x509_certificate_chain_der, &policy, now)?;
    let manifest = verified.manifest().clone();
    drop(verified);
    let what = format!("Plugin {} {} (suite {}, {})", manifest.plugin_id, manifest.version, policy.suite,
        policy.base.generation_name());
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let triplet = config::multiarch_triplet();
    let parts = match triplet {
        Some(t) => broker::check_parts(dir, &manifest, t).map_err(|e| e.to_string())?,
        None => 0,
    };
    if wasm::needs_broker(&manifest) {
        let triplet = triplet.ok_or("no multiarch triplet for this build")?;
        if parts == 0 {
            return Err(format!("the signed manifest lists no native parts for {triplet}").into());
        }
        return Ok(format!("{what}: X.509, signature, manifest and {parts} native part(s) for {triplet} OK; \
            not initialized (needs a broker)"));
    }
    let host = Host::new(1)?;
    let mut worker = wasm::load_signed(&host, bundle, &policy, LoadOptions {
        binding: Binding { instance_id: "plugin-check".into(), job_id: 1,
            device_id: "plugin-check".into(), worker_id: "plugin-check".into() },
        job: JobContext { job_id: 1, job_name: "plugin-check".into(), operation: match operation {
            PluginCheckOperation::Backup => JobOperation::Backup, PluginCheckOperation::Restore => JobOperation::Restore,
        } as i32 }, options: Default::default(),
        worker: Limits { max_pending_operations: 1, max_request_bytes: 256 * 1024,
            max_response_bytes: 256 * 1024, operation_timeout: std::time::Duration::from_secs(5) }, runtime, broker: None,
    })?;
    worker.shutdown(std::time::Duration::from_secs(2))?;
    Ok(format!("{what}: X.509, signature, manifest, component ABI and initialization OK"))
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

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::path::PathBuf::from(format!("/tmp/ofd-check-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_broker_plugin_is_checked_up_to_its_native_parts() {
        use onesimus_plugins::NativePart;
        use sha2::{Digest, Sha256};
        let Some(triplet) = config::multiarch_triplet() else { return };
        let d = dir("broker");
        std::fs::create_dir(d.join("postgresql")).unwrap();
        let broker = d.join("postgresql/onesimus-pg-broker");
        std::fs::write(&broker, b"broker binary").unwrap();
        let sha: String = Sha256::digest(b"broker binary").iter().map(|b| format!("{b:02x}")).collect();
        let native = [(triplet.to_string(), vec![NativePart { path: "postgresql/onesimus-pg-broker".into(), sha256: sha }])].into();
        // A component that could not run without a broker; check mode never instantiates it.
        let component = wat::parse_str("(component)").unwrap();
        let resolved = plugin::testkit::signed_artifact_with(&d, "postgresql", component,
            &["file.read", "postgresql.read"], native);
        let report = check_plugin(&resolved.bundle, &resolved.policy, PluginCheckOperation::Backup).unwrap();
        assert!(report.contains("test.postgresql 1.0.0") && report.contains("1 native part(s)")
            && report.contains("not initialized"), "{report}");
        // A replaced broker fails the check.
        std::fs::write(&broker, b"other binary").unwrap();
        let error = check_plugin(&resolved.bundle, &resolved.policy, PluginCheckOperation::Backup).unwrap_err();
        assert!(error.to_string().contains("SHA-256"), "{error}");
        // A broker plugin without native parts for this triplet fails too.
        let resolved = plugin::testkit::signed_artifact_with(&d, "postgresql", wat::parse_str("(component)").unwrap(),
            &["postgresql.read"], Default::default());
        let error = check_plugin(&resolved.bundle, &resolved.policy, PluginCheckOperation::Backup).unwrap_err();
        assert!(error.to_string().contains("no native parts"), "{error}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_plain_plugin_is_initialized_and_a_bad_bundle_refused() {
        use prost::Message;
        let d = dir("plain");
        let resolved = plugin::testkit::signed_plugin(&d, "example", &Default::default());
        let report = check_plugin(&resolved.bundle, &resolved.policy, PluginCheckOperation::Restore).unwrap();
        assert!(report.contains("initialization OK") && report.contains("Onesimus Plugin Signing G1"), "{report}");
        let mut bundle = onesimus_plugins::bundle::read_bundle(&resolved.bundle, 1 << 24).unwrap();
        bundle.ed25519_signature[3] ^= 1;
        std::fs::write(&resolved.bundle, bundle.encode_to_vec()).unwrap();
        let error = check_plugin(&resolved.bundle, &resolved.policy, PluginCheckOperation::Backup).unwrap_err();
        assert!(error.to_string().contains("invalid plugin signature"), "{error}");
        // A local policy that tries to grant is a load error.
        std::fs::write(&resolved.policy, br#"{"signers":{}}"#).unwrap();
        let error = check_plugin(&resolved.bundle, &resolved.policy, PluginCheckOperation::Backup).unwrap_err();
        assert!(error.to_string().contains("cannot grant"), "{error}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
