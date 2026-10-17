//! Onesimus Director — Drop-in replacement for the Bareos Director daemon.
//!
//! Named threads, fork-per-job, crossbeam channels for core,
//! tokio only for gRPC.

mod accept;
mod acl;
mod audit;
mod breg;
mod btime;
mod catalog_worker;
mod check_catalog;
mod catreq;
mod command_ast;
mod console_msg;
mod command_executor;
mod command_queue;
mod command_registry;
mod commands;
mod config_check;
mod command_worker;
mod expand;
mod director_config;
mod fd_protocol;
mod fsm;
mod grpc;
mod internal_jobs;
mod job_manager;
mod job_messages;
mod job_worker;
mod json_output;
mod kill;
mod localtime;
mod logging;
mod mdns;
mod ndmp;
mod ovfs;
mod privileges;
mod message_bus;
mod reload;
mod run_time;
mod runscript;
mod scheduled_run;
mod scheduler;
mod migrate;
mod sd_copy;
mod sd_label;
mod scan;
mod sd_protocol;
mod session;
mod session_manager;
mod sql_list;
mod ua_commands_table;
mod ua_db;
mod ua_impexp;
mod ua_storage_select;
mod ua_select;
mod version_strings;
mod volume_key;
mod watchdog;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use clap::Parser;
use tracing::{info, error, warn, debug};
use tracing_subscriber::EnvFilter;

use crate::command_queue::CommandQueue;
use crate::director_config::{DirectorConfig, SharedConfig};
use crate::message_bus::MessageBus;
use crate::session_manager::SessionManager;

/// Global shutdown flag.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Onesimus Director — Drop-in replacement for the Bareos Director
#[derive(Parser, Debug)]
#[command(
    name = "onesimus-dird",
    version,
    about = "The Onesimus Director Daemon.\n\nCopyright (C) 2026 Onesimus.io\nDrop-in compatible with Bareos FD/SD/bconsole"
)]
struct Cli {
    /// Use <path> as configuration file or directory.
    #[arg(short, long, default_value = "/etc/bareos/bareos-dir.d")]
    config: PathBuf,

    /// Set debug level to <level>.
    #[arg(short, long, default_value_t = 0)]
    debug_level: u32,

    /// Print timestamps in debug output.
    #[arg(long = "dt", alias = "debug-timestamps")]
    debug_timestamps: bool,

    /// Run in foreground.
    #[arg(short, long)]
    foreground: bool,

    /// Run as given user (requires starting as root).
    #[arg(short, long)]
    user: Option<String>,

    /// Run as given group (requires starting as root).
    #[arg(short, long)]
    group: Option<String>,

    /// Test - read configuration and exit.
    #[arg(short, long)]
    test_config: bool,

    /// Stop the running Director (PID from -p, otherwise found via /proc) and exit.
    #[arg(short = 'k', long)]
    kill: bool,

    /// Full path to pidfile (default: none).
    #[arg(short, long)]
    pid_file: Option<PathBuf>,

    /// No signals (for debugging).
    #[arg(short = 's', long)]
    no_signals: bool,

    /// Verbose user messages.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Print all or specific configuration resources and exit.
    #[arg(long = "xc", alias = "export-config")]
    export_config: Option<Option<String>>,

    /// Print configuration schema in JSON format and exit.
    #[arg(long = "xs", alias = "export-schema")]
    export_schema: bool,

    /// Override listening port (default: from config, typically 9101).
    #[arg(long, short = 'P')]
    port: Option<u16>,

    /// Override bind address (default: 0.0.0.0).
    #[arg(long, short = 'b')]
    bind_address: Option<String>,

    /// Override director name (default: from config).
    #[arg(long)]
    name: Option<String>,

    /// Override working directory (default: /var/lib/bareos).
    #[arg(short = 'w', long)]
    working_directory: Option<PathBuf>,

    /// Restore only mode — only accept restore jobs.
    #[arg(short = 'r', long)]
    restore_only: bool,

    /// Maximum concurrent jobs override.
    #[arg(short = 'm', long)]
    max_jobs: Option<u32>,

    /// TLS certificate file (PEM)
    #[arg(long)]
    tls_cert: Option<String>,

    /// TLS private key file (PEM)
    #[arg(long)]
    tls_key: Option<String>,

    /// TLS CA certificate file (PEM)
    #[arg(long)]
    tls_ca: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    localtime::init_locale();
    localtime::mark_daemon_start();
    if cli.kill {
        std::process::exit(kill::kill_running(cli.pid_file.as_deref(), &cli.config));
    }

    // Before any thread exists: SIGHUP must only reach the dir:sighup thread,
    // SIGINT/SIGTERM only wait_for_shutdown() (clean stop for systemctl/-k)
    reload::block_sighup();
    block_shutdown_signals();

    // Debug-Level Hilfe
    if cli.debug_level == 999 {
        println!("Debug-Level Referenz (kompatibel mit C++ Bareos Dmsg):");
        println!();
        println!("  Level   Beschreibung");
        println!("  -----   ----------------------------------------");
        println!("  0       Aus (keine Debug-Ausgaben)");
        println!("  1-49    Basis (Startup, Shutdown, Config)");
        println!("  50-99   Ausfuehrlich (Commands, Sessions, Jobs)");
        println!("  100     Protokoll (TLS-PSK, CRAM-MD5)");
        println!("  110     Daemon-Messages (>stored:, <filed:)");
        println!("  116     Wire (gesendete/empfangene BNET-Strings)");
        println!("  150     Detail (Auth-Schritte, Config-Parse)");
        println!("  200     Trace (Memory, Locks, Funktionsaufrufe)");
        println!("  999     Diese Hilfe anzeigen");
        println!();
        println!("Beispiel: onesimus-dird -d 100 -f");
        std::process::exit(0);
    }

    // Logging
    let filter = if cli.debug_level > 0 {
        format!("debug,hyper=info,h2=info,tower=info")
    } else {
        "info,hyper=warn,h2=warn,tower=warn".to_string()
    };
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
    let (filter_layer, filter_handle) = tracing_subscriber::reload::Layer::new(
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&filter)));
    logging::install(filter_handle);
    tracing_subscriber::registry().with(filter_layer)
        .with(tracing_subscriber::fmt::layer().with_ansi(false)
            .with_writer(|| bareos_bnet::debug::DebugWriter))
        .init();

    // Install rustls CryptoProvider for TLS support
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Bareos-kompatibles Debug-Level System
    bareos_bnet::debug::set_debug_level(cli.debug_level);
    bareos_bnet::debug::set_debug_timestamps(cli.debug_timestamps);
    // Keep "bareos-dir" for BNET protocol compatibility (C++ FD/SD expect it)
    bareos_bnet::debug::set_daemon_name("bareos-dir");

    info!("Onesimus Director Version {} starting", env!("CARGO_PKG_VERSION"));
    if cli.debug_level > 0 {
        info!("debug level set to {}", cli.debug_level);
    }

    // Load configuration
    let mut config = if cli.config.exists() {
        match DirectorConfig::load(&cli.config) {
            Ok(c) => {
                info!("loaded configuration from {}", cli.config.display());
                c
            }
            Err(e) => {
                error!("failed to load config {}: {e}", cli.config.display());
                std::process::exit(1);
            }
        }
    } else {
        warn!("config file {} not found, using defaults", cli.config.display());
        DirectorConfig::from_str("Director { Name = bareos-dir\n Port = 9101\n Password = default\n }").unwrap()
    };

    // --test-config: validate and exit
    if cli.test_config {
        println!("configuration OK: Director '{}', {} clients, {} jobs, {} storages, {} pools, {} schedules",
              config.director.name, config.clients.len(), config.jobs.len(),
              config.storages.len(), config.pools.len(), config.schedules.len());
        std::process::exit(0);
    }

    // --export-schema: print JSON schema and exit
    if cli.export_schema {
        let schema_dir = std::path::Path::new("/usr/lib/bareos/scripts/ddl");
        let migrator = bareos_orm::migration::Migrator::new(schema_dir);
        match migrator.latest() {
            Ok(Some(schema)) => {
                println!("{}", serde_json::to_string_pretty(&schema).unwrap());
            }
            _ => {
                eprintln!("No schema found in {}", schema_dir.display());
            }
        }
        std::process::exit(0);
    }

    // --export-config: print config resources and exit
    if cli.export_config.is_some() {
        println!("Director {{");
        println!("  Name = \"{}\"", config.director.name);
        println!("  Port = {}", config.director.port);
        println!("  Maximum Concurrent Jobs = {}", config.director.max_concurrent_jobs);
        println!("}}");
        for c in &config.clients {
            println!("\nClient {{");
            println!("  Name = \"{}\"", c.name);
            println!("  Address = \"{}\"", c.address);
            println!("  Port = {}", c.port);
            println!("  Enabled = {}", if c.enabled { "yes" } else { "no" });
            println!("}}");
        }
        for j in &config.jobs {
            println!("\nJob {{");
            println!("  Name = \"{}\"", j.name);
            println!("  Type = {:?}", j.job_type);
            println!("  Client = \"{}\"", j.client);
            println!("  Storage = \"{}\"", j.storage);
            println!("  Pool = \"{}\"", j.pool);
            println!("}}");
        }
        for s in &config.storages {
            println!("\nStorage {{");
            println!("  Name = \"{}\"", s.name);
            println!("  Address = \"{}\"", s.address);
            println!("  Port = {}", s.port);
            println!("}}");
        }
        for p in &config.pools {
            println!("\nPool {{");
            println!("  Name = \"{}\"", p.name);
            println!("  Pool Type = \"{}\"", p.pool_type);
            println!("  Recycle = {}", if p.recycle { "yes" } else { "no" });
            println!("  Maximum Volumes = {}", p.maximum_volumes);
            println!("}}");
        }
        std::process::exit(0);
    }

    // CLI overrides
    if let Some(p) = cli.port { config.director.port = p; }
    if let Some(ref n) = cli.name { config.director.name = n.clone(); }
    if let Some(m) = cli.max_jobs { config.director.max_concurrent_jobs = m; }

    // Daemonize wenn NICHT -f/--foreground — before any thread exists (fork keeps
    // only the calling thread) and before privileges are dropped (PID file).
    if !cli.foreground && !cli.test_config {
        info!("daemonizing...");
        let mut daemon = daemonize::Daemonize::new()
            .working_directory(&config.director.working_directory);

        if let Some(ref pid_file) = cli.pid_file {
            daemon = daemon.pid_file(pid_file);
        }

        match daemon.start() {
            Ok(_) => info!("daemon started"),
            Err(e) => {
                error!("FATAL: daemonize failed: {e}");
                std::process::exit(1);
            }
        }
    }

    // -p/--pid-file in foreground mode (daemonize writes it itself)
    if cli.foreground {
        if let Some(ref pid_file) = cli.pid_file {
            if let Err(e) = std::fs::write(pid_file, format!("{}\n", std::process::id())) {
                error!("cannot write PID file {}: {e}", pid_file.display());
            }
        }
    }

    // -u/--user, -g/--group: drop privileges before the catalog connects
    // (peer authentication / ~/.pgpass as the target user) and before threads
    #[cfg(unix)]
    if let Err(e) = privileges::drop_privileges(cli.user.as_deref(), cli.group.as_deref()) {
        error!("FATAL: {e}");
        std::process::exit(1);
    }
    if cli.user.is_some() || cli.group.is_some() {
        info!("running as uid={} gid={}", nix::unistd::getuid(), nix::unistd::getgid());
    }

    // dird.cc: InitConsoleMsg(working_directory) — console message queue
    console_msg::init(std::path::Path::new(&config.director.working_directory), &config.director.name);

    // Build catalog URL and auto-migrate
    let catalog_db_url = config.catalogs.first().map(|cat| {
        if cat.db_address.is_empty() || cat.db_address == "localhost" || cat.db_address == "127.0.0.1" {
            format!("postgres://{}@localhost/{}?host=/var/run/postgresql", cat.db_user, cat.db_name)
        } else {
            format!("postgres://{}@{}:{}/{}", cat.db_user, cat.db_address, cat.db_port, cat.db_name)
        }
    });

    if let Some(ref db_url) = catalog_db_url {
        let cat = config.catalogs.first().unwrap();

        // Schema-Verzeichnis: relativ zum Binary (dev) oder /usr/lib/bareos/ddl/schema (installed)
        let schema_dir = commands::catalog::schema_dir();

        if schema_dir.exists() {
            let migrator = bareos_orm::migration::Migrator::new(&schema_dir);
            let rt = tokio::runtime::Runtime::new().unwrap();
            match rt.block_on(async {
                let pool = sqlx::postgres::PgPoolOptions::new()
                    .max_connections(1)
                    .connect(&db_url).await?;
                migrator.migrate(&pool).await
            }) {
                Ok(0) => info!("catalog '{}' schema up to date", cat.db_name),
                Ok(n) => info!("catalog '{}': applied {} schema migration(s)", cat.db_name, n),
                Err(e) => warn!("catalog migration failed: {e} (continuing without catalog)"),
            }
        } else {
            debug!("no schema directory found at {}, skipping migration", schema_dir.display());
        }
    }

    // Spawn catalog worker (single DB pool for all queries)
    let catalog_handle = if let Some(url) = catalog_db_url {
        let (handle, join) = catalog_worker::spawn_catalog_worker(url, &SHUTDOWN);
        // Store join handle for later
        Some((handle, join))
    } else {
        None
    };
    let catalog = catalog_handle.as_ref().map(|(h, _)| h.clone());
    if let Some(catalog) = &catalog {
        ovfs::register_for_reload(catalog.clone());
        reload::register_catalog(catalog.clone());
    }
    // CheckCatalog(): basic Client/Storage/MediaType records.
    if let Some(cat) = &catalog {
        check_catalog::ensure_basic_records(&config, cat);
    }
    // dird.cc: SetDbLogInsertCallback(DirDbLogInsert) — `catalog` message destination
    job_messages::set_catalog(catalog.clone());

    let port = config.director.port;
    let bind_addr = cli.bind_address.clone().unwrap_or_else(|| "0.0.0.0".to_string());
    let grpc_enabled = config.director.enable_grpc;
    let dir_name = config.director.name.clone();
    let dir_password = config.director.password.clone();

    // TLS config from CLI args
    let tls_config = if cli.tls_cert.is_some() && cli.tls_key.is_some() {
        Some(bareos_bnet::tls::TlsConfig {
            certfile: cli.tls_cert.clone(),
            keyfile: cli.tls_key.clone(),
            ca_certfile: cli.tls_ca.clone(),
            verify_peer: false,
        })
    } else {
        None
    };

    // Shared state
    let shared_config: SharedConfig = Arc::new(std::sync::RwLock::new(config));
    let bus = Arc::new(MessageBus::new(1024));
    let session_mgr = Arc::new(SessionManager::new(bus.clone()));
    let cmd_queue = Arc::new(CommandQueue::new(256));
    let job_mgr = Arc::new(std::sync::Mutex::new(
        job_manager::JobManager::new(bus.clone(), Some(shared_config.clone()), catalog.clone())
    ));

    info!("Director '{}' listening on port {}", dir_name, port);

    // Named threads
    let mut threads = spawn_named_threads(
        session_mgr.clone(), bus.clone(), cmd_queue.clone(),
        shared_config.clone(), port, bind_addr, grpc_enabled, dir_password, tls_config,
        job_mgr.clone(), catalog.clone(),
    );

    // dir:job-monitor — polls all job worker pipes
    {
        let mgr = job_mgr.clone();
        let handle = thread::Builder::new()
            .name("dir:job-monitor".into())
            .spawn(move || {
                job_manager::monitor_loop(mgr, &SHUTDOWN);
            })
            .expect("failed to spawn dir:job-monitor");
        threads.push(("dir:job-monitor".into(), handle));
    }

    // SIGHUP handler for config reload
    reload::setup_sighup_handler(cli.config.clone(), shared_config.clone());

    // Wait for shutdown signal (SIGTERM / SIGINT)
    wait_for_shutdown();

    info!("Onesimus Director shutting down");
    SHUTDOWN.store(true, Ordering::SeqCst);
    cmd_queue.shutdown();

    // Join all threads
    for (name, handle) in threads {
        if let Err(e) = handle.join() {
            error!("Thread {name} panicked: {e:?}");
        }
    }

    info!("Onesimus Director stopped");
}

fn spawn_named_threads(
    session_mgr: Arc<SessionManager>,
    bus: Arc<MessageBus>,
    cmd_queue: Arc<CommandQueue>,
    config: SharedConfig,
    port: u16,
    bind_address: String,
    grpc_enabled: bool,
    password: String,
    tls_config: Option<bareos_bnet::tls::TlsConfig>,
    job_mgr: Arc<std::sync::Mutex<job_manager::JobManager>>,
    catalog: Option<catalog_worker::CatalogHandle>,
) -> Vec<(String, thread::JoinHandle<()>)> {
    let mut threads = Vec::new();

    // dir:accept — TCP connection listener
    {
        let smgr = session_mgr.clone();
        let q = cmd_queue.clone();
        let bind_address = bind_address.clone();
        let pw = password.clone();
        let tls_cfg = tls_config.clone();
        let shared_cfg = config.clone();
        let jmgr = job_mgr.clone();
        let cat = catalog.clone();
        let handle = thread::Builder::new()
            .name("dir:accept".into())
            .spawn(move || {
                let config = accept::AcceptConfig {
                    bind_address: bind_address,
                    port,
                    password: pw,
                    tls_config: tls_cfg,
                    shared_config: Some(shared_cfg),
                    job_manager: Some(jmgr),
                    catalog: cat,
                };
                accept::accept_loop(&smgr, &q, &config, &SHUTDOWN);
            })
            .expect("failed to spawn dir:accept");
        threads.push(("dir:accept".into(), handle));
    }

    // dir:scheduler — job scheduling
    {
        let launcher = scheduled_run::Launcher {
            config: config.clone(),
            session_mgr: Some(session_mgr.clone()),
            catalog: catalog.clone(),
            job_manager: Some(job_mgr.clone()),
        };
        let handle = thread::Builder::new()
            .name("dir:scheduler".into())
            .spawn(move || {
                scheduler::scheduler_loop(Some(launcher), &SHUTDOWN);
            })
            .expect("failed to spawn dir:scheduler");
        threads.push(("dir:scheduler".into(), handle));
    }

    // dir:watchdog — health monitoring
    {
        let handle = thread::Builder::new()
            .name("dir:watchdog".into())
            .spawn(move || {
                watchdog::watchdog_loop(&SHUTDOWN);
            })
            .expect("failed to spawn dir:watchdog");
        threads.push(("dir:watchdog".into(), handle));
    }

    // dir:grpc — tonic gRPC server over TLS (optional), dir:mdns — DNS-SD announcement
    let (dir_name, grpc_port, mdns_advertise, grpc_identity) = {
        let cfg = config.read().unwrap();
        let d = &cfg.director;
        let identity = if grpc_enabled {
            match grpc::tls::load_or_generate(
                d.grpc_tls_certificate.as_deref(),
                d.grpc_tls_key.as_deref(),
                &d.working_directory,
                &d.name,
            ) {
                Ok(id) => Some(id),
                Err(e) => {
                    error!("gRPC disabled: no TLS certificate: {e}");
                    None
                }
            }
        } else {
            None
        };
        (d.name.clone(), d.grpc_port.unwrap_or(port.saturating_add(1000)), d.mdns_advertise, identity)
    };
    let grpc_fingerprint = grpc_identity.as_ref().map(|id| id.fingerprint.clone());

    if let Some(identity) = grpc_identity {
        let b = bus.clone();
        let q = cmd_queue.clone();
        let settings = grpc::GrpcSettings {
            bind_address: bind_address.clone(),
            port: grpc_port,
            bnet_port: port,
            director_name: dir_name.clone(),
            identity,
            auth: Arc::new(grpc::auth::AuthState::new(config.clone(), session_mgr.clone())),
        };
        let handle = thread::Builder::new()
            .name("dir:grpc".into())
            .spawn(move || {
                grpc::run_grpc_server(&b, &q, settings, &SHUTDOWN);
            })
            .expect("failed to spawn dir:grpc");
        threads.push(("dir:grpc".into(), handle));
    }

    if mdns_advertise {
        let ann = mdns::Announcement {
            director_name: dir_name,
            bnet_port: port,
            grpc: grpc_fingerprint.map(|fp| (grpc_port, fp)),
        };
        let handle = thread::Builder::new()
            .name("dir:mdns".into())
            .spawn(move || {
                mdns::advertise_loop(ann, &SHUTDOWN);
            })
            .expect("failed to spawn dir:mdns");
        threads.push(("dir:mdns".into(), handle));
    }

    // dir:cmdworker — processes commands from gRPC and other queue submitters
    {
        let q = cmd_queue.clone();
        let cfg = config.clone();
        let smgr = session_mgr.clone();
        let cat = catalog.clone();
        let jmgr = job_mgr.clone();
        let handle = thread::Builder::new()
            .name("dir:cmdworker".into())
            .spawn(move || {
                command_worker::command_worker_loop(&q, Some(cfg), Some(smgr), cat, Some(jmgr), &SHUTDOWN);
            })
            .expect("failed to spawn dir:cmdworker");
        threads.push(("dir:cmdworker".into(), handle));
    }

    threads
}

fn shutdown_sigset() -> nix::sys::signal::SigSet {
    use nix::sys::signal::{SigSet, Signal};
    let mut set = SigSet::empty();
    set.add(Signal::SIGINT);
    set.add(Signal::SIGTERM);
    set
}

/// Block SIGINT/SIGTERM in all threads (inherited mask); main waits for them.
fn block_shutdown_signals() {
    if let Err(e) = shutdown_sigset().thread_block() {
        error!("cannot block SIGINT/SIGTERM: {e}");
    }
}

fn wait_for_shutdown() {
    match shutdown_sigset().wait() {
        Ok(sig) => info!("{sig} received, shutting down"),
        Err(e) => error!("sigwait failed: {e}"),
    }
}
