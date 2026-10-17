//! Onesimus Console (oconsole) — Interactive console client for the Director.
//!
//! Connects via BNET (TCP + TLS + CRAM-MD5) or gRPC (TLS + pinned certificate).
//! With --mcp it serves the Model Context Protocol on stdio instead of a prompt.

#[cfg(all(feature = "mcp-production", feature = "mcp-development"))]
compile_error!("MCP production and development policies are mutually exclusive; use --no-default-features --features mcp-development for development");
#[cfg(not(any(feature = "mcp-production", feature = "mcp-development")))]
compile_error!("Select an MCP policy: mcp-production (default) or mcp-development");

mod color;

// Keep all console stdout/stderr decoration under the same NO_COLOR policy.
macro_rules! print { ($($arg:tt)*) => { std::print!("{}", crate::color::output(&format!($($arg)*))) }; }
macro_rules! println { () => { std::println!() }; ($($arg:tt)*) => { std::println!("{}", crate::color::output(&format!($($arg)*))) }; }
macro_rules! eprintln { () => { std::eprintln!() }; ($($arg:tt)*) => { std::eprintln!("{}", crate::color::output(&format!($($arg)*))) }; }

mod grpc_client;
mod mcp;
mod mcp_read;
mod mcp_mirror;
#[cfg(feature = "mcp-development")]
mod mcp_write;
mod mcp_token;
mod rate_guard;

use std::io::{self, BufRead, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use tracing::{info, error, debug};
use tracing_subscriber::EnvFilter;

/// Global CTRL-C flag — set by signal handler, checked in recv loop.
static CTRL_C: AtomicBool = AtomicBool::new(false);
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Editor, Helper, Context};

use bareos_bnet::{BnetStream, BnetSignal, BnetMessage, BnetError};
use bareos_bnet::hello::make_console_hello;
use bareos_bnet::cram_md5::CramMd5Auth;

/// Onesimus Console — Interactive Director Client
#[derive(Parser, Debug)]
#[command(
    name = "oconsole",
    version,
    about = "The Onesimus Console.\n\nCopyright (C) 2026 Onesimus.io"
)]
struct Cli {
    /// Use <path> as configuration file [default: /etc/bareos/oconsole.conf,
    /// then /etc/bareos/bconsole.conf].
    #[arg(short, long)]
    config: Option<String>,

    /// Director address (overrides config)
    #[arg(short = 'a', long)]
    address: Option<String>,

    /// Director port (overrides config)
    #[arg(short, long)]
    port: Option<u16>,

    /// Console name (overrides config)
    #[arg(short, long)]
    name: Option<String>,

    /// Password (overrides config)
    #[arg(short = 'w', long)]
    password: Option<String>,

    /// Use gRPC protocol (onesimus-dird only)
    #[arg(long)]
    grpc: bool,

    /// gRPC port (default: Director port + 1000)
    #[arg(long)]
    grpc_port: Option<u16>,

    /// Expected SHA-256 fingerprint of the Director's gRPC certificate
    /// (default: ~/.config/oconsole/known_directors, asked on first use)
    #[arg(long)]
    grpc_fingerprint: Option<String>,

    /// Serve the Model Context Protocol on stdin/stdout (for AI assistants)
    #[arg(long)]
    mcp: bool,

    /// MCP: serve Streamable HTTP at /mcp instead of stdio (e.g. 127.0.0.1:9104)
    #[arg(long, requires = "mcp")]
    mcp_http: Option<std::net::SocketAddr>,

    /// MCP HTTP bearer-token file (created with mode 0600 if missing)
    #[arg(long, requires = "mcp_http")]
    mcp_token_file: Option<std::path::PathBuf>,

    /// Additional HTTP Host authority to accept, e.g. mcp.example.org
    #[arg(long, requires = "mcp_http")]
    mcp_host: Vec<String>,

    /// Development builds only: also offer the state-changing execute tool
    #[cfg(feature = "mcp-development")]
    #[arg(long, requires = "mcp")]
    mcp_allow_write: bool,

    /// Execute single command and exit
    #[arg(short = 'e', long)]
    command: Option<String>,
}

impl Cli {
    fn mcp_write_enabled(&self) -> bool {
        #[cfg(feature = "mcp-development")]
        { self.mcp_allow_write }
        #[cfg(feature = "mcp-production")]
        { false }
    }
}

/// Parsed config from oconsole.conf / bconsole.conf
struct ConsoleConfig {
    address: String,
    port: u16,
    name: String,
    password: String,
    /// oconsole extensions (written by the Onesimus GUI): Grpc = yes,
    /// Grpc Port = <n>, Grpc Fingerprint = <sha256 hex>
    grpc: bool,
    grpc_port: Option<u16>,
    grpc_fingerprint: Option<String>,
}

/// Effective CLI — resolved from config file + CLI overrides
struct EffCli {
    address: String,
    port: u16,
    name: String,
    password: String,
    grpc: bool,
    grpc_port: u16,
    grpc_fingerprint: Option<String>,
    command: Option<String>,
    mcp: bool,
    mcp_allow_write: bool,
    mcp_http: Option<std::net::SocketAddr>,
    mcp_token_file: Option<std::path::PathBuf>,
    mcp_host: Vec<String>,
}

/// Default config files, in order: oconsole's own, then bconsole's (the same
/// format, already set up wherever a Bareos console works).
const DEFAULT_CONFIGS: [&str; 2] = ["/etc/bareos/oconsole.conf", "/etc/bareos/bconsole.conf"];

fn load_console_config(path: Option<&str>) -> ConsoleConfig {
    let candidates: Vec<&str> = match path {
        Some(p) => vec![p],
        None => DEFAULT_CONFIGS.to_vec(),
    };
    let mut last_error = None;
    for candidate in &candidates {
        match std::fs::read_to_string(candidate) {
            Ok(content) => return parse_console_config(&content),
            Err(e) => last_error = Some((candidate.to_string(), e)),
        }
    }
    if let Some((p, e)) = last_error {
        eprintln!("\x1b[33mWarnung: Config {} nicht lesbar: {e}\x1b[0m", p);
        eprintln!("\x1b[33mTipp: Benutzer zur Gruppe 'bareos' hinzufuegen oder sudo verwenden\x1b[0m");
    }
    parse_console_config("")
}

/// console_conf.cc semantics: `Director { Name Address DirPort Password }`
/// names the Director to connect to; a `Console { Name Password }` resource is
/// the named console to log in as. Without one the console logs in as
/// "*UserAgent*" with the Director's password. Director/Console `Name` is never
/// the login of the other resource.
fn parse_console_config(content: &str) -> ConsoleConfig {
    let mut cfg = ConsoleConfig {
        address: "localhost".to_string(),
        port: 9101,
        name: "*UserAgent*".to_string(),
        password: String::new(),
        grpc: false,
        grpc_port: None,
        grpc_fingerprint: None,
    };
    let mut director_password = String::new();
    let mut console: Option<(String, String)> = None;
    // Resource we are in: None at top level (flat files of older oconsole
    // versions are read as Director keys).
    let mut block: Option<String> = None;
    let mut depth = 0usize;

    for raw in content.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() { continue; }
        if depth == 0 {
            if let Some(open) = line.find('{') {
                block = Some(line[..open].trim().to_ascii_lowercase());
                depth = 1;
                if block.as_deref() == Some("console") && console.is_none() {
                    console = Some((String::new(), String::new()));
                }
                continue;
            }
        } else if line.contains('{') {
            depth += 1;
            continue;
        }
        if line.starts_with('}') {
            depth = depth.saturating_sub(1);
            if depth == 0 { block = None; }
            continue;
        }
        let Some(pos) = line.find('=') else { continue };
        let key: String = line[..pos].split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
        let val = line[pos + 1..].trim().trim_matches('"').to_string();
        match (block.as_deref(), key.as_str()) {
            (Some("console"), "name") => { if let Some(c) = console.as_mut() { c.0 = val } }
            (Some("console"), "password") => { if let Some(c) = console.as_mut() { c.1 = val } }
            (Some("console"), _) => {}
            (_, "address") => cfg.address = val,
            (_, "port" | "dirport") => cfg.port = val.parse().unwrap_or(9101),
            (_, "password") => director_password = val,
            (_, "grpc") => cfg.grpc = matches!(val.to_ascii_lowercase().as_str(), "yes" | "true" | "1" | "on"),
            (_, "grpc port" | "grpcport") => cfg.grpc_port = val.parse().ok(),
            (_, "grpc fingerprint" | "grpcfingerprint") => {
                cfg.grpc_fingerprint = Some(val.replace(':', "").to_ascii_lowercase())
            }
            _ => {}
        }
    }

    match console {
        Some((name, password)) if !name.is_empty() => {
            cfg.name = name;
            cfg.password = password;
        }
        _ => cfg.password = director_password,
    }
    cfg
}

fn main() {
    let cli = Cli::parse();
    let mcp_allow_write = cli.mcp_write_enabled();

    // Load config file, then override with CLI args
    let file_cfg = load_console_config(cli.config.as_deref());

    // CLI args override config file
    let address = cli.address.unwrap_or(file_cfg.address);
    let port = cli.port.unwrap_or(file_cfg.port);
    let name = cli.name.unwrap_or(file_cfg.name);
    let password = cli.password.unwrap_or(file_cfg.password);

    // stderr: stdout belongs to command output (and to the protocol in --mcp mode)
    tracing_subscriber::fmt().with_ansi(color::enabled())
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    // Build effective CLI with resolved values
    let eff = EffCli {
        address: address.clone(),
        port,
        name: name.clone(),
        password: password.clone(),
        grpc: cli.grpc || file_cfg.grpc,
        grpc_port: cli.grpc_port.or(file_cfg.grpc_port).unwrap_or(port.saturating_add(1000)),
        grpc_fingerprint: cli.grpc_fingerprint.clone().or(file_cfg.grpc_fingerprint.clone()),
        command: cli.command.clone(),
        mcp: cli.mcp,
        mcp_allow_write,
        mcp_http: cli.mcp_http,
        mcp_token_file: cli.mcp_token_file.clone(),
        mcp_host: cli.mcp_host.clone(),
    };

    // SIGINT handler: set flag instead of killing process.
    // rustyline handles SIGINT at the prompt; we handle it during recv().
    unsafe {
        libc::signal(libc::SIGINT, sigint_handler as libc::sighandler_t);
    }

    let addr = format!("{}:{}", eff.address, eff.port);

    if eff.grpc {
        run_grpc_mode(&eff);
    } else {
        run_bnet_mode(&eff, &addr);
    }
}

fn run_bnet_mode(cli: &EffCli, addr: &str) {
    // Connect
    eprintln!("Connecting to Director {addr}");
    let stream = match TcpStream::connect(addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Connection failed: {e}");
            std::process::exit(1);
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();

    // Try TLS-PSK if password is set
    if !cli.password.is_empty() {
        let psk_identity = format!("R_CONSOLE\x1e{}", cli.name);
        let psk = bareos_bnet::tls_psk::password_to_psk(&cli.password);
        match bareos_bnet::tls_psk::psk_connect(stream, &psk_identity, &psk) {
            Ok(psk_stream) => {
                eprintln!(" Encryption: TLS-PSK");
                let mut bnet = BnetStream::new(psk_stream);
                run_bnet_session(cli, bnet);
                return;
            }
            Err(e) => {
                debug!("TLS-PSK failed ({e}), reconnecting plaintext");
                let stream2 = TcpStream::connect(addr).unwrap();
                stream2.set_read_timeout(Some(Duration::from_secs(30))).ok();
                let mut bnet = BnetStream::new(stream2);
                run_bnet_session(cli, bnet);
                return;
            }
        }
    }
    let mut bnet = BnetStream::new(stream);
    run_bnet_session(cli, bnet);
}

fn run_bnet_session<S: std::io::Read + std::io::Write + Send + 'static>(cli: &EffCli, mut bnet: BnetStream<S>) {
    // Send hello
    // Like bconsole (lib/bsock.cc:348) the version goes with every console's
    // Hello; the Director then answers with the 18.2+ banner (1000 + 1002).
    let hello = make_console_hello(&cli.name, env!("CARGO_PKG_VERSION"), cli.grpc);
    if let Err(e) = bnet.send_str(&hello) {
        eprintln!("Failed to send hello: {e}");
        std::process::exit(1);
    }

    // CRAM-MD5 authentication (if password provided)
    if !cli.password.is_empty() {
        let auth = CramMd5Auth::new(&cli.password, &cli.name, 0);
        match auth.two_way_authenticate(&mut bnet, false) {
            Ok(result) if result.success => {
                debug!("authentication successful");
            }
            Ok(_) => {
                eprintln!("Authentication failed");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("Authentication error: {e}");
                std::process::exit(1);
            }
        }
    }

    // Receive init messages: 1000 OK + 1002 Info.
    // C++ Director sends NO signal after init — goes straight to recv().
    // A Director answering in the pre-18.2 form (one frame "1000 OK: ... --
    // Encryption: ...", authenticate_console.cc:129) sends nothing more.
    for _ in 0..2 {
        match bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                let text = String::from_utf8_lossy(&data);
                if let Some(pos) = text.find('\x1e') {
                    eprintln!("{}", &text[pos+1..].trim());
                    if !text.starts_with("1000\x1e") { break; }
                } else {
                    eprintln!("{}", text.trim());
                    break;
                }
            }
            Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
            Ok(BnetMessage::Signal(BnetSignal::MainPrompt)) => break,
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => {
                eprintln!("Director terminated connection");
                std::process::exit(1);
            }
            Ok(BnetMessage::Signal(_)) => continue,
            Err(e) => {
                eprintln!("Failed to receive banner: {e}");
                std::process::exit(1);
            }
        }
    }

    // MCP server mode
    if cli.mcp {
        eprintln!("oconsole: serving MCP{}", if cli.mcp_allow_write { " (write access enabled)" } else { " (read-only)" });
        mcp::McpServer::new(BnetRunner { bnet }, cli.mcp_allow_write, "bareos-dir".to_string()).serve_transport(cli.mcp_http, cli.mcp_token_file.clone(), cli.mcp_host.clone()).unwrap_or_else(|e| { eprintln!("MCP: {e}"); std::process::exit(1) });
        return;
    }

    // Single command mode
    if let Some(cmd) = &cli.command {
        execute_single_command(&mut bnet, cmd);
        return;
    }

    // Interactive mode
    interactive_loop(Box::new(BnetConn { bnet }), &cli.name, "bareos-dir");
}

/// MCP command runner over an authenticated BNET session (api=0: response ends with EOD).
struct BnetRunner<S: std::io::Read + std::io::Write> {
    bnet: BnetStream<S>,
}

impl<S: std::io::Read + std::io::Write + Send + 'static> mcp::CommandRunner for BnetRunner<S> {
    /// Any question of the Director is cancelled with "." so that a command
    /// never leaves the session waiting for input.
    fn run(&mut self, command: &str) -> anyhow::Result<String> {
        self.run_dialog(command, &mut |_| None)
    }

    /// Plain console: a question is the prompt text followed by
    /// BNET_SUB_PROMPT, then the Director waits for one answer line.
    fn run_dialog(&mut self, command: &str, answer: &mut dyn FnMut(&str) -> Option<String>) -> anyhow::Result<String> {
        self.bnet.send_str(command)?;
        let mut output = String::new();
        let mut questions = 0;
        loop {
            match self.bnet.recv() {
                Ok(BnetMessage::Data(data)) => output.push_str(&String::from_utf8_lossy(&data)),
                Ok(BnetMessage::Signal(BnetSignal::SubPrompt)) => {
                    questions += 1;
                    let reply = if questions > mcp::MAX_QUESTIONS { None } else { answer(&output) };
                    let reply = reply.unwrap_or_else(|| ".".into());
                    output.push_str(&format!("{reply}\n"));
                    self.bnet.send_str(&reply)?;
                }
                Ok(BnetMessage::Signal(BnetSignal::Eod)) | Ok(BnetMessage::Signal(BnetSignal::MainPrompt)) => break,
                Ok(BnetMessage::Signal(BnetSignal::Terminate)) => anyhow::bail!("Director closed the connection"),
                Ok(BnetMessage::Signal(_)) => continue,
                Err(e) => anyhow::bail!("connection error: {e}"),
            }
        }
        Ok(output)
    }
}

fn execute_single_command<S: std::io::Read + std::io::Write>(bnet: &mut BnetStream<S>, cmd: &str) {
    // Init messages already drained in run_bnet_session.
    if bnet.send_str(cmd).is_err() {
        return;
    }

    // Read response until EOD or MainPrompt
    loop {
        match bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                let text = String::from_utf8_lossy(&data);
                print!("{text}");
            }
            Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
            Ok(BnetMessage::Signal(BnetSignal::MainPrompt)) => break,
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
            Ok(BnetMessage::Signal(_)) => continue,
            Err(_) => break,
        }
    }

    let _ = bnet.send_str("quit");
    let _ = bnet.send_signal(BnetSignal::Terminate);
}

/// Abstraction for Director communication (BNET or gRPC).
/// Used by both the command loop and tab-completion.
trait DirectorConn: Send {
    /// Send a command and return the full response as text lines.
    fn query(&mut self, cmd: &str) -> Vec<String>;

    /// Send a command string. Returns false on error.
    fn send_cmd(&mut self, cmd: &str) -> bool;

    /// Receive one message. Returns (is_data, text) or None on EOD/prompt/error.
    fn recv_msg(&mut self) -> Option<RecvResult>;

    /// Drain until MainPrompt or EOD.
    fn drain(&mut self);

    /// Send a signal string (e.g. "." for cancel).
    fn send_cancel(&mut self);
}

/// Result from recv_msg
enum RecvResult {
    Data(String),
    Signal(BnetSignal),
    Error(String),
    Closed,
}

/// BNET implementation of DirectorConn.
struct BnetConn<S: std::io::Read + std::io::Write + Send> {
    bnet: BnetStream<S>,
}

impl<S: std::io::Read + std::io::Write + Send> DirectorConn for BnetConn<S> {
    fn query(&mut self, cmd: &str) -> Vec<String> {
        if cmd.is_empty() {
            self.drain();
            return Vec::new();
        }
        // api=0: send cmd → read Data → EOD marks end
        // api>0: CMD_BEGIN, CMD_OK/CMD_FAILED, Data → MainPrompt marks end
        // (C++ ua_cmds.cc framing); those framing signals are skipped
        if self.bnet.send_str(cmd).is_err() { return Vec::new(); }
        let mut output = String::new();
        loop {
            match self.bnet.recv() {
                Ok(BnetMessage::Data(data)) => {
                    output.push_str(&String::from_utf8_lossy(&data));
                }
                Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
                Ok(BnetMessage::Signal(BnetSignal::MainPrompt)) => break,
                Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
                Ok(BnetMessage::Signal(_)) => continue,
                Err(_) => break,
            }
        }
        output.lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn send_cmd(&mut self, cmd: &str) -> bool {
        self.bnet.send_str(cmd).is_ok()
    }

    fn recv_msg(&mut self) -> Option<RecvResult> {
        match self.bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                Some(RecvResult::Data(String::from_utf8_lossy(&data).to_string()))
            }
            Ok(BnetMessage::Signal(sig)) => Some(RecvResult::Signal(sig)),
            Err(BnetError::HardEof) | Err(BnetError::Terminated) => Some(RecvResult::Closed),
            Err(e) if e.to_string().contains("os error 11") || e.to_string().contains("Interrupted") => {
                // EAGAIN or EINTR — not fatal, just retry
                None
            }
            Err(e) => Some(RecvResult::Error(e.to_string())),
        }
    }

    fn drain(&mut self) {
        // Read until EOD (api=0) or MainPrompt (api>0) — whichever comes first.
        loop {
            match self.bnet.recv() {
                Ok(BnetMessage::Signal(BnetSignal::Eod)) => break,
                Ok(BnetMessage::Signal(BnetSignal::MainPrompt)) => break,
                Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    }

    fn send_cancel(&mut self) {
        let _ = self.bnet.send_str(".");
    }
}

/// Parsed command entry from Director's .help response.
struct CmdEntry {
    name: String,
    description: String,
    usage: String,
    /// Static completions from arguments field (bare keywords with !, key= args)
    completions: Vec<String>,
    /// Dot-command to query Director for dynamic values (e.g. ".clients" for client=)
    /// Maps: argument key → dot-command
    live_queries: Vec<(String, String)>,
}

/// rustyline Helper with tab-completion (from Director .help) and syntax coloring.
/// Holds a shared reference to the Director connection for live queries.
struct BareosHelper {
    /// Commands loaded from Director via .help at session start
    entries: Vec<CmdEntry>,
    rate_guard: rate_guard::CommandRateGuard,
            /// Cached values from Director for tab-completion (loaded at connect)
    /// Maps: dot-command → list of values (e.g. ".clients" → ["bareos-fd", "remote-fd"])
    value_cache: std::collections::HashMap<String, Vec<String>>,
}

impl BareosHelper {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            rate_guard: rate_guard::CommandRateGuard::default(),
            value_cache: std::collections::HashMap::new(),
        }
    }

    fn command_names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    fn find_command(&self, name: &str) -> Option<&CmdEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Load commands + cached values from Director at connect time.
    /// All queries happen sequentially over one connection before readline starts.
    /// Reload everything from the Director (after `reload`/`configure`).
    fn refresh(&mut self, dir: &mut dyn DirectorConn) {
        self.entries.clear();
        self.value_cache.clear();
        self.load_from_director(dir);
    }

    fn load_from_director(&mut self, dir: &mut dyn DirectorConn) {
        // Bareos API1 .help carries descriptions and original usage;
        // .help all intentionally contains only non-dot command names.
        dir.query(".api 1");
        let lines = dir.query(".help");
        dir.query(".api 0");

        for line in &lines {
            if let Some(entry) = Self::parse_help_line(line) { self.entries.push(entry); }
        }

        if self.entries.is_empty() {
            for name in &["version", "time", "quit", "exit", "help", ".help",
                          "status", "run", "cancel", "list", "reload"] {
                self.entries.push(CmdEntry {
                    name: name.to_string(),
                    description: String::new(),
                    usage: String::new(),
                    completions: Vec::new(),
                    live_queries: Vec::new(),
                });
            }
        }

        // Cache all dot-command values that are used for live-completion.
        // Query each unique dot-command once at connect time.
        let mut dot_cmds: Vec<String> = self.entries.iter()
            .flat_map(|e| e.live_queries.iter().map(|(_, cmd)| cmd.clone()))
            .collect();
        dot_cmds.sort();
        dot_cmds.dedup();

        for dot_cmd in &dot_cmds {
            let values = dir.query(dot_cmd);
            self.value_cache.insert(dot_cmd.clone(), values);
        }
    }

    fn parse_help_line(line: &str) -> Option<CmdEntry> {
        let (name, description, usage, arguments) = if line.contains('\t') {
            // Compatibility with older Onesimus metadata tables.
            let mut fields = line.split('\t');
            (fields.next()?.trim(), fields.next().unwrap_or(""), fields.next().unwrap_or(""),
                Some(fields.next().unwrap_or("")))
        } else {
            let (header, usage) = line.split_once(" -- ")?;
            let (name, description) = header.split_once(' ')?;
            (name.trim(), description, usage, None)
        };
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_') { return None; }
        let source_arguments;
        let arguments = match arguments {
            Some(arguments) => arguments,
            None => {
                source_arguments = Self::usage_arguments(usage, name);
                &source_arguments
            }
        };
        let (completions, live_queries) = Self::parse_arguments(arguments);
        Some(CmdEntry {name:name.into(),description:description.into(),usage:usage.into(),completions,live_queries})
    }

    fn usage_arguments(usage: &str, command: &str) -> String {
        let mut arguments=Vec::new();
        for token in usage.split_whitespace() {
            let token=token.trim_matches(|c:char| matches!(c,'['|']'|'('|')'|'|'|','));
            if token.is_empty() || token==command { continue; }
            let argument = if let Some((key,_))=token.split_once('=') {
                if key.is_empty() || !key.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_') { continue; }
                key.to_string()
            } else {
                if !token.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_' || b==b'-') { continue; }
                format!("!{token}")
            };
            if !arguments.contains(&argument) { arguments.push(argument); }
        }
        arguments.join(",")
    }

    /// Parse the arguments field from .help into completions and live-query mappings.
    ///
    /// Conventions:
    /// - "!dir"        → bare keyword "dir" (no "=")
    /// - "job"         → key=value arg "job=" — also triggers live query ".jobs" for values
    /// - "client"      → key=value arg "client=" — triggers ".clients"
    /// - "storage"     → key=value arg "storage=" — triggers ".storages"
    /// - "pool"        → key=value arg "pool=" — triggers ".pools"
    /// - "fileset"     → key=value arg "fileset=" — triggers ".filesets"
    /// - "schedule"    → key=value arg "schedule=" — triggers ".schedule"
    /// - "level"       → key=value arg "level=" — triggers ".levels"
    /// - "type"        → key=value arg "type=" — triggers ".types"
    /// - "catalog"     → key=value arg "catalog=" — triggers ".catalogs"
    fn parse_arguments(arguments: &str) -> (Vec<String>, Vec<(String, String)>) {
        let mut completions = Vec::new();
        let mut live_queries = Vec::new();

        if arguments.is_empty() { return (completions, live_queries); }

        for arg in arguments.split(',') {
            let arg = arg.trim();
            if arg.is_empty() { continue; }

            if let Some(bare) = arg.strip_prefix('!') {
                // Bare keyword — no "="
                completions.push(bare.to_string());
            } else {
                // key=value argument
                completions.push(format!("{arg}="));

                // Map known argument keys to Director dot-commands for live values
                let dot_cmd = match arg {
                    "job" => Some(".jobs"),
                    "client" => Some(".clients"),
                    "storage" => Some(".storages"),
                    "pool" => Some(".pools"),
                    "fileset" => Some(".filesets"),
                    "schedule" => Some(".schedule"),
                    "level" => Some(".levels"),
                    "type" => Some(".types"),
                    "catalog" => Some(".catalogs"),
                    "volume" => Some(".media"),
                    _ => None,
                };
                if let Some(cmd) = dot_cmd {
                    live_queries.push((arg.to_string(), cmd.to_string()));
                }
            }
        }

        (completions, live_queries)
    }
}

impl Completer for BareosHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let line = &line[..pos];
        let parts: Vec<&str> = line.split_whitespace().collect();
        let arg_pos = if line.ends_with(' ') { parts.len() } else { parts.len().saturating_sub(1) };
        let prefix = if line.ends_with(' ') { "" } else { parts.last().copied().unwrap_or("") };
        let start = if line.ends_with(' ') { pos } else { pos - prefix.len() };

        // === Position 0: Command name ===
        if arg_pos == 0 {
            let matches: Vec<Pair> = self.entries.iter()
                .filter(|e| e.name.starts_with(prefix))
                .map(|e| {
                    if e.description.is_empty() {
                        Pair { display: e.name.clone(), replacement: e.name.clone() }
                    } else {
                        // fish-style: "command  -- description"
                        Pair {
                            display: format!("{:<22} -- {}", e.name, e.description),
                            replacement: e.name.clone(),
                        }
                    }
                })
                .collect();
            return Ok((0, matches));
        }

        let cmd = parts[0];

        // "help <TAB>" / ".help <TAB>" → complete with command names + descriptions
        if cmd == "help" || cmd == ".help" {
            let matches: Vec<Pair> = self.entries.iter()
                .filter(|e| e.name.starts_with(prefix))
                .map(|e| Pair {
                    display: format!("{:<22} -- {}", e.name, e.description),
                    replacement: e.name.clone(),
                })
                .collect();
            return Ok((start, matches));
        }

        let entry = match self.find_command(cmd) {
            Some(e) => e,
            None => return Ok((pos, Vec::new())),
        };

        // === Value after "key=" (e.g. "run client=[TAB]") ===
        if prefix.contains('=') {
            let (key, val_prefix) = prefix.split_once('=').unwrap_or(("", ""));
            let val_start = start + key.len() + 1;

            // Cached values from Director
            if let Some((_, dot_cmd)) = entry.live_queries.iter().find(|(k, _)| k == key) {
                let empty = Vec::new();
                let values = self.value_cache.get(dot_cmd.as_str()).unwrap_or(&empty);
                let matches: Vec<Pair> = values.iter()
                    .filter(|v| v.starts_with(val_prefix))
                    .map(|v| Pair { display: v.to_string(), replacement: v.to_string() })
                    .collect();
                return Ok((val_start, matches));
            }

            return Ok((pos, Vec::new()));
        }

        // === Position 1+: Arguments (bare keywords + key= args) ===
        // Filter out args already typed
        let already_typed: Vec<&str> = parts[1..].iter()
            .map(|p| p.split('=').next().unwrap_or(""))
            .collect();

        let matches: Vec<Pair> = entry.completions.iter()
            .filter(|c| {
                let key = c.trim_end_matches('=');
                // Don't suggest key= args already used; bare keywords can repeat
                !already_typed.contains(&key) || !c.ends_with('=')
            })
            .filter(|c| c.starts_with(prefix))
            .map(|c| Pair { display: c.to_string(), replacement: c.to_string() })
            .collect();

        Ok((start, matches))
    }
}

impl Highlighter for BareosHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(&'s self, prompt: &'p str, default: bool) -> std::borrow::Cow<'b, str> {
        if !default { return std::borrow::Cow::Borrowed(prompt); }
        std::borrow::Cow::Owned(self.rate_guard.prompt(prompt, color::enabled()))
    }
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        if !color::enabled() { return std::borrow::Cow::Borrowed(line); }
        if line.is_empty() {
            return std::borrow::Cow::Borrowed(line);
        }

        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        let cmd = parts[0];
        let args_str = if parts.len() > 1 { parts[1] } else { "" };

        let entry = self.find_command(cmd);

        // Command color: green=regular, yellow=dot, red=unknown
        let cmd_color = match entry {
            Some(_) if cmd.starts_with('.') => "\x1b[33m", // yellow
            Some(_) => "\x1b[32m",                          // green
            None => "\x1b[31m",                              // red
        };

        if args_str.is_empty() {
            return std::borrow::Cow::Owned(format!("{cmd_color}{cmd}\x1b[0m"));
        }

        // Color arguments: known args = cyan, values = white, bare keywords = magenta
        let mut colored_args = String::new();
        let known_completions: Vec<&str> = entry
            .map(|e| e.completions.iter().map(|c| c.as_str()).collect())
            .unwrap_or_default();

        for (i, token) in args_str.split_whitespace().enumerate() {
            if i > 0 { colored_args.push(' '); }

            if let Some((key, val)) = token.split_once('=') {
                // key=value: key in cyan, value in white
                let key_known = known_completions.iter().any(|c| c.trim_end_matches('=') == key);
                if key_known {
                    colored_args.push_str(&format!("\x1b[36m{key}\x1b[0m=\x1b[35m{val}\x1b[0m"));
                } else {
                    colored_args.push_str(&format!("\x1b[36m{key}\x1b[0m=\x1b[35m{val}\x1b[0m"));
                }
            } else if known_completions.iter().any(|c| *c == token) {
                // Known bare keyword: magenta
                colored_args.push_str(&format!("\x1b[35m{token}\x1b[0m"));
            } else {
                // Unknown argument: no color
                colored_args.push_str(token);
            }
        }

        // Preserve trailing space
        if args_str.ends_with(' ') { colored_args.push(' '); }

        std::borrow::Cow::Owned(format!("{cmd_color}{cmd}\x1b[0m {colored_args}"))
    }


    fn highlight_hint<'h>(&self, hint: &'h str) -> std::borrow::Cow<'h, str> {
        if !color::enabled() { return color::output(hint); }
        // Ghost-Text in grau (dim) — wie fish autosuggestion
        std::borrow::Cow::Owned(format!("\x1b[2m{hint}\x1b[0m"))
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: rustyline::highlight::CmdKind) -> bool {
        true
    }
}

impl Hinter for BareosHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<String> {
        if pos < line.len() { return None; } // nur am Zeilenende

        let parts: Vec<&str> = line.split_whitespace().collect();

        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 && !line.ends_with(' ') {
            // Ghost-Text fuer Command-Name: "sta" → grau "tus"
            let prefix = parts[0];
            let best = self.command_names()
                .filter(|c| c.starts_with(prefix) && *c != prefix)
                .next()?;
            return Some(best[prefix.len()..].to_string());
        }

        if parts.len() >= 1 && line.ends_with(' ') {
            // Nach Command + Space: Ghost-Text fuer erstes Argument
            let cmd = parts[0];
            if let Some(entry) = self.find_command(cmd) {
                // Zeige Usage als Ghost-Hint
                let usage_hint = &entry.usage;
                // Nur den Teil nach dem Command-Namen
                if let Some(rest) = usage_hint.strip_prefix(cmd) {
                    let rest = rest.trim_start();
                    if !rest.is_empty() {
                        return Some(color::output(&format!("\x1b[2m{rest}\x1b[0m")).into_owned());
                    }
                }
            }
        }

        None
    }
}

impl Validator for BareosHelper {}

impl Helper for BareosHelper {}

/// None means rejected locally; Some(false) means a connection failure.
fn send_terminal_line(
    dir: &mut dyn DirectorConn,
    guard: &rate_guard::CommandRateGuard,
    line: &str,
) -> Option<bool> {
    if line.contains(['\n', '\r']) {
        eprintln!("oconsole: Command Rate Guard violation: multiline input discarded.");
        return None;
    }
    if !guard.admit() {
        return None;
    }
    Some(dir.send_cmd(line))
}
fn send_terminal_cancel(dir: &mut dyn DirectorConn, guard: &rate_guard::CommandRateGuard) -> bool {
    if !guard.admit() {
        return false;
    }
    dir.send_cancel();
    true
}
fn terminal_readline(
    rl: &mut Editor<BareosHelper, rustyline::history::DefaultHistory>,
    prompt: &str,
    guard: &rate_guard::CommandRateGuard,
    refresh: &rate_guard::PromptRefresh,
) -> Result<String, ReadlineError> {
    refresh.arm(guard.remaining());
    let result = rl.readline(prompt);
    refresh.disarm();
    result
}
fn read_guarded_dialog(
    rl: &mut Editor<BareosHelper, rustyline::history::DefaultHistory>,
    dir: &mut dyn DirectorConn,
    guard: &rate_guard::CommandRateGuard,
    refresh: &rate_guard::PromptRefresh,
    prompt: &str,
    cancel_dot: bool,
) -> bool {
    loop {
        match terminal_readline(rl, prompt, guard, refresh) {
            Ok(input) if cancel_dot && input.trim() == "." => {
                if send_terminal_cancel(dir, guard) {
                    return true;
                }
            }
            Ok(input) => match send_terminal_line(dir, guard, input.trim()) {
                None => continue,
                Some(sent) => return sent,
            },
            Err(ReadlineError::Interrupted) => {
                eprintln!();
                if send_terminal_cancel(dir, guard) {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
}

/// Reply text, colored by keyword.
fn print_reply(text: &str) {
    for line in text.lines() {
        if line.contains("Error") || line.contains("FEHLER") || line.contains("failed") {
            println!("\x1b[31m{line}\x1b[0m");
        } else if line.contains("Warning") || line.contains("WARNUNG") {
            println!("\x1b[33m{line}\x1b[0m");
        } else if line.contains("OK") || line.contains("success") {
            println!("\x1b[32m{line}\x1b[0m");
        } else {
            println!("{line}");
        }
    }
}

/// API 1 input dialogs of the Director (C-076). DoPrompt() (ua_select.cc)
/// signals SELECT_INPUT, GetYesno() (ua_input.cc) YESNO; both then call
/// GetCmd(), which signals TEXT_INPUT and sends the prompt text as the
/// next data frame ("Select Job resource (1-2): ") — and then waits for
/// the answer without a SUB_PROMPT. The console therefore takes the data
/// frame after these signals as the prompt and reads the answer.
#[derive(Default)]
struct InputDialog {
    pending: Option<BnetSignal>,
}

impl InputDialog {
    fn signal(&mut self, signal: BnetSignal) {
        match signal {
            // TEXT_INPUT keeps the kind announced before it
            BnetSignal::TextInput if self.pending.is_some() => {}
            BnetSignal::SelectInput | BnetSignal::YesNo | BnetSignal::TextInput => self.pending = Some(signal),
            _ => {}
        }
    }

    /// For the data frame after an input signal: the text before the last
    /// line (printed as output) and the last line as the readline prompt.
    fn prompt(&mut self, text: &str) -> Option<(String, String)> {
        let kind = self.pending.take()?;
        let (before, last) = match text.rfind('\n') {
            Some(pos) if pos + 1 < text.len() => (&text[..=pos], &text[pos + 1..]),
            _ if text.ends_with('\n') => (text, ""),
            _ => ("", text),
        };
        let prompt = if last.is_empty() {
            match kind {
                BnetSignal::YesNo => "(yes/no): ",
                BnetSignal::SelectInput => "Select: ",
                _ => "input> ",
            }.to_string()
        } else {
            last.to_string()
        };
        Some((before.to_string(), prompt))
    }
}

fn interactive_loop(
    mut dir: Box<dyn DirectorConn>,
    console_name: &str,
    director_name: &str,
) {

    // Init messages already drained in run_bnet_session before entering interactive mode.
    let mut helper = BareosHelper::new();
    let guard = helper.rate_guard.clone();
    helper.load_from_director(dir.as_mut());
    let total_completions: usize = helper.entries.iter().map(|e| e.completions.len()).sum();
    let cached_values: usize = helper.value_cache.values().map(|v| v.len()).sum();
    eprintln!("\x1b[2m({} commands, {} subcommands, {} cached values for tab-completion)\x1b[0m",
        helper.entries.len(), total_completions, cached_values);

    let rl_config = rustyline::Config::builder()
        .completion_type(rustyline::CompletionType::List)
        .edit_mode(rustyline::EditMode::Emacs)
        .auto_add_history(false)
        .build();
    let mut rl = Editor::with_config(rl_config).unwrap();
    rl.set_helper(Some(helper));
    let refresh = if color::enabled() {
        rl.create_external_printer().map(rate_guard::PromptRefresh::new).unwrap_or_default()
    } else { rate_guard::PromptRefresh::default() };

    let history_file = dirs_home().join(".oconsole_history");
    let _ = rl.load_history(&history_file);

    let version = env!("CARGO_PKG_VERSION");
    let prompt = format!("{}@{}[{}]> ", console_name, director_name, version);

    loop {
        // Kein drain() hier — der State ist nach Init/Command-Response synchron.
        // Das naechste MainPrompt wird vom Director gesendet wenn wir einen Command senden.

        match terminal_readline(&mut rl, &prompt, &guard, &refresh) {
            Ok(line) => {
                let cmd = line.trim();
                if cmd.is_empty() { continue; }

                match send_terminal_line(dir.as_mut(), &guard, cmd) {
                    None => continue,
                    Some(false) => { eprintln!("Connection lost"); break; }
                    Some(true) => { rl.add_history_entry(cmd).ok(); }
                }

                // Quit
                if matches!(cmd, "quit" | "exit" | ".quit") {
                    // C++ Director (ua_cmds.cc quit_cmd) sends nothing and just
                    // closes the socket; show anything that still arrives.
                    while let Some(RecvResult::Data(text)) = dir.recv_msg() {
                        println!("{}", text.trim());
                    }
                    break;
                }

                // Read response
                CTRL_C.store(false, Ordering::SeqCst);
                let mut dialog = InputDialog::default();
                loop {
                    if CTRL_C.swap(false, Ordering::SeqCst) {
                        if send_terminal_cancel(dir.as_mut(), &guard) {
                            eprintln!("\x1b[33m<CTRL-C> — sending cancel to Director\x1b[0m");
                        }
                        continue;
                    }

                    let msg = dir.recv_msg();
                    match msg {
                        Some(RecvResult::Data(text)) => {
                            if let Some((before, dialog_prompt)) = dialog.prompt(&text) {
                                print_reply(&before);
                                if !read_guarded_dialog(&mut rl, dir.as_mut(), &guard, &refresh,
                                    &dialog_prompt, false) {
                                    let _ = rl.save_history(&history_file);
                                    return;
                                }
                                continue;
                            }
                            print_reply(&text);
                        }
                        // API 1 input dialogs: the prompt text follows (C-076)
                        Some(RecvResult::Signal(signal @ (BnetSignal::SelectInput | BnetSignal::YesNo | BnetSignal::TextInput))) => {
                            dialog.signal(signal);
                        }
                        Some(RecvResult::Signal(BnetSignal::Eod)) => {
                            // api=0: EOD marks end of response, no MainPrompt follows.
                            break;
                        }
                        Some(RecvResult::Signal(BnetSignal::MainPrompt)) => break,
                        Some(RecvResult::Signal(BnetSignal::Terminate)) => {
                            eprintln!("\x1b[31mDirector terminated connection\x1b[0m");
                            let _ = rl.save_history(&history_file);
                            return;
                        }
                        // API 0: the prompt text came before the signal
                        Some(RecvResult::Signal(BnetSignal::SubPrompt)) => {
                            if !read_guarded_dialog(&mut rl, dir.as_mut(), &guard, &refresh, "> ", true) {
                                let _ = rl.save_history(&history_file);
                                return;
                            }
                        }
                        None => {
                            // EAGAIN/EINTR — check CTRL-C
                            if CTRL_C.swap(false, Ordering::SeqCst) {
                                if send_terminal_cancel(dir.as_mut(), &guard) {
                                    eprintln!("\x1b[33m<CTRL-C> — sending cancel to Director\x1b[0m");
                                }
                            }
                            continue;
                        }
                        Some(RecvResult::Signal(sig)) => {
                            debug!("signal: {sig:?}");
                        }
                        Some(RecvResult::Closed) => {
                            eprintln!("Connection closed");
                            let _ = rl.save_history(&history_file);
                            return;
                        }
                        Some(RecvResult::Error(e)) => {
                            eprintln!("\x1b[31mError: {e}\x1b[0m");
                            let _ = rl.save_history(&history_file);
                            return;
                        }
                        None => break,
                    }
                }

                // The Director's resources may have changed: refresh the
                // completion data (command list, job/client/pool names, ...)
                let first = cmd.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                if matches!(first.as_str(), "reload" | "configure") {
                    if let Some(helper) = rl.helper_mut() {
                        helper.refresh(dir.as_mut());
                        eprintln!("\x1b[2m(tab-completion refreshed: {} commands, {} cached values)\x1b[0m",
                            helper.entries.len(), helper.value_cache.values().map(|v| v.len()).sum::<usize>());
                    }
                }
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => {
                // EOF closes the connection; do not send an unguarded command.
                break;
            }
            Err(e) => {
                eprintln!("\x1b[31mReadline error: {e}\x1b[0m");
                break;
            }
        }
    }

    let _ = rl.save_history(&history_file);
}

/// POSIX signal handler for SIGINT (CTRL-C).
/// Sets the atomic flag — the recv loop checks it.
extern "C" fn sigint_handler(_sig: libc::c_int) {
    CTRL_C.store(true, Ordering::SeqCst);
}

/// Check if a BnetError wraps an EINTR (interrupted by signal).
fn is_interrupted(e: &BnetError) -> bool {
    match e {
        BnetError::Io(io_err) => io_err.kind() == std::io::ErrorKind::Interrupted,
        _ => false,
    }
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
}

// drain_signals removed — now part of DirectorConn::drain()

fn run_grpc_mode(cli: &EffCli) {
    use bareos_proto_rs::bareos::director::ResponseFormat;

    let host = cli.address.clone();
    let port = cli.grpc_port;
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    // Certificate pin: --grpc-fingerprint, else known_directors, else ask (trust on first use)
    let mut known = grpc_client::KnownDirectors::load();
    let fingerprint = match cli.grpc_fingerprint.clone().or_else(|| known.get(&host, port).map(str::to_string)) {
        Some(fp) => fp,
        None => {
            let seen = match rt.block_on(grpc_client::probe_fingerprint(&host, port)) {
                Ok(fp) => fp,
                Err(e) => {
                    eprintln!("gRPC: {e}");
                    std::process::exit(1);
                }
            };
            if cli.mcp || cli.command.is_some() {
                eprintln!("gRPC: unknown Director certificate {host}:{port}\n  SHA-256 {seen}");
                eprintln!("Verify it and pass --grpc-fingerprint {seen}, or connect once interactively.");
                std::process::exit(1);
            }
            eprintln!("The Director {host}:{port} is not known yet.");
            eprintln!("Certificate SHA-256 fingerprint:\n  {seen}");
            eprintln!("Compare it with the Director log ('certificate SHA-256 …').");
            eprint!("Trust and remember this Director? [y/N] ");
            let _ = io::stderr().flush();
            let mut answer = String::new();
            let _ = io::stdin().lock().read_line(&mut answer);
            if !matches!(answer.trim(), "y" | "Y" | "yes" | "j" | "J" | "ja") {
                std::process::exit(1);
            }
            if let Err(e) = known.remember(&host, port, &seen) {
                eprintln!("Warning: cannot write {}: {e}", known.path().display());
            }
            seen
        }
    };

    let mut session = match rt.block_on(grpc_client::GrpcSession::connect(&host, port, &fingerprint, &cli.name, &cli.password)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("gRPC: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("Connected to Director {} {} via gRPC ({host}:{port}, TLS)", session.director_name, session.version);

    if cli.mcp {
        eprintln!("oconsole: serving MCP{}", if cli.mcp_allow_write { " (write access enabled)" } else { " (read-only)" });
        let name = session.director_name.clone();
        let runner = GrpcRunner { rt, session };
        mcp::McpServer::new(runner, cli.mcp_allow_write, name).serve_transport(cli.mcp_http, cli.mcp_token_file.clone(), cli.mcp_host.clone()).unwrap_or_else(|e| { eprintln!("MCP: {e}"); std::process::exit(1) });
        return;
    }

    let mut run = |cmd: &str| -> bool {
        match rt.block_on(session.execute(cmd, ResponseFormat::Text)) {
            Ok(out) => {
                print!("{}", out.text);
                if !out.error.is_empty() {
                    eprintln!("{}", out.error.trim_end());
                }
                out.success
            }
            Err(e) => {
                eprintln!("gRPC error: {e}");
                false
            }
        }
    };

    // Single command mode
    if let Some(cmd) = &cli.command {
        if !run(cmd) {
            std::process::exit(1);
        }
        return;
    }

    // Interactive mode: same line editor (history, tab completion) as BNET
    let name = session.director_name.clone();
    interactive_loop(Box::new(GrpcConn { rt, session, pending: Vec::new() }), &cli.name, &name);
}

/// gRPC implementation of DirectorConn: each command is one ExecuteCommand
/// call; the reply is handed to the interactive loop as Data + EOD.
struct GrpcConn {
    rt: tokio::runtime::Runtime,
    session: grpc_client::GrpcSession,
    pending: Vec<RecvResult>,
}

impl GrpcConn {
    fn execute(&mut self, cmd: &str, format: bareos_proto_rs::bareos::director::ResponseFormat) -> Result<String, String> {
        let out = self.rt.block_on(self.session.execute(cmd, format)).map_err(|e| format!("gRPC error: {e}"))?;
        Ok(if out.error.is_empty() { out.text } else { format!("{}{}", out.text, out.error) })
    }
}

impl DirectorConn for GrpcConn {
    fn query(&mut self, cmd: &str) -> Vec<String> {
        use bareos_proto_rs::bareos::director::ResponseFormat;
        // Stateless calls: .api has no effect; machine format is chosen per request
        if cmd.is_empty() || cmd.starts_with(".api") {
            return Vec::new();
        }
        self.execute(cmd, ResponseFormat::Api).unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect()
    }

    fn send_cmd(&mut self, cmd: &str) -> bool {
        use bareos_proto_rs::bareos::director::ResponseFormat;
        if matches!(cmd, "quit" | "exit" | ".quit") {
            // Like the BNET Director: quit produces no output
            self.pending = vec![RecvResult::Closed];
            return true;
        }
        let text = match self.execute(cmd, ResponseFormat::Text) {
            Ok(t) => t,
            // A failed call is shown like a command error; the session goes on
            Err(e) => format!("Error: {e}\n"),
        };
        // recv_msg pops from the end
        self.pending = vec![RecvResult::Signal(BnetSignal::Eod), RecvResult::Data(text)];
        true
    }

    fn recv_msg(&mut self) -> Option<RecvResult> {
        Some(self.pending.pop().unwrap_or(RecvResult::Signal(BnetSignal::Eod)))
    }

    fn drain(&mut self) {
        self.pending.clear();
    }

    fn send_cancel(&mut self) {}
}

/// MCP command runner over an authenticated gRPC session.
struct GrpcRunner {
    rt: tokio::runtime::Runtime,
    session: grpc_client::GrpcSession,
}

impl mcp::CommandRunner for GrpcRunner {
    fn run(&mut self, command: &str) -> anyhow::Result<String> {
        use bareos_proto_rs::bareos::director::ResponseFormat;
        let out = self.rt.block_on(self.session.execute(command, ResponseFormat::Text))?;
        if !out.success && out.text.is_empty() {
            anyhow::bail!("{}", out.error.trim_end());
        }
        Ok(if out.error.is_empty() { out.text } else { format!("{}{}", out.text, out.error) })
    }

    fn run_json(&mut self, command: &str) -> anyhow::Result<String> {
        use bareos_proto_rs::bareos::director::ResponseFormat;
        let out = self.rt.block_on(self.session.execute(command, ResponseFormat::Json))?;
        if !out.success && out.text.is_empty() {
            anyhow::bail!("{}", out.error.trim_end());
        }
        Ok(out.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn director_name_is_not_the_login() {
        // oconsole.conf as the package writes it: the Director's Name must not
        // become the console name ("unknown console 'bareos-dir'").
        let cfg = parse_console_config(
            "Director {\n  Name = bareos-dir\n  address = localhost\n  Port = 9101\n  Password = \"dirpw\"\n}\n");
        assert_eq!(cfg.name, "*UserAgent*");
        assert_eq!(cfg.password, "dirpw");
        assert_eq!(cfg.address, "localhost");
        assert_eq!(cfg.port, 9101);
    }

    #[test]
    fn console_resource_is_the_login() {
        // bconsole.conf with a named console, as on a typical Bareos host.
        let cfg = parse_console_config(concat!(
            "Director {\n  Name = PDC-dir\n  DIRport = 9102\n#  Address = old.example\n",
            "  Address = 192.0.2.10\n  Password = \"dirpw\"\n\n  # TLS\n  TLS Enable = yes\n}\n\n",
            "Console {\n  Name = bconsole\n  Password = \"conpw\"\n}\n"));
        assert_eq!(cfg.name, "bconsole");
        assert_eq!(cfg.password, "conpw");
        assert_eq!(cfg.address, "192.0.2.10");
        assert_eq!(cfg.port, 9102);
    }

    #[test]
    fn flat_file_and_grpc_keys_still_work() {
        let cfg = parse_console_config("address = h\nport = 1\npassword = p\nGrpc = yes\nGrpc Port = 7\n");
        assert_eq!((cfg.address.as_str(), cfg.port, cfg.name.as_str(), cfg.password.as_str()), ("h", 1, "*UserAgent*", "p"));
        assert!(cfg.grpc);
        assert_eq!(cfg.grpc_port, Some(7));
    }

    #[test]
    fn no_config_option_means_default_search() {
        // A clap default would always set --config and skip bconsole.conf.
        let cli = Cli::try_parse_from(["oconsole"]).unwrap();
        assert!(cli.config.is_none());
    }

    #[test]
    fn explicit_config_path_is_read() {
        let dir = std::env::temp_dir().join(format!("oconsole-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let only = dir.join("bconsole.conf");
        std::fs::write(&only, "Director {\n Address = x\n Password = \"q\"\n}\n").unwrap();
        let cfg = load_console_config(Some(only.to_str().unwrap()));
        assert_eq!((cfg.address.as_str(), cfg.password.as_str()), ("x", "q"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn select_input_takes_following_text_as_prompt() {
        // DoPrompt() at API 1: SELECT_INPUT, TEXT_INPUT, prompt text
        let mut dialog = InputDialog::default();
        assert_eq!(dialog.prompt("Header:\n"), None, "no input signal yet: plain output");
        dialog.signal(BnetSignal::SelectInput);
        dialog.signal(BnetSignal::TextInput);
        assert_eq!(dialog.prompt("Select Job resource (1-2): "),
            Some((String::new(), "Select Job resource (1-2): ".to_string())));
        // answered: the next frame is output again
        assert_eq!(dialog.prompt("Please enter a number between 1 and 2\n"), None);
    }

    #[test]
    fn yesno_and_text_input_prompts() {
        let mut dialog = InputDialog::default();
        dialog.signal(BnetSignal::YesNo);
        dialog.signal(BnetSignal::TextInput);
        assert_eq!(dialog.prompt(""), Some((String::new(), "(yes/no): ".to_string())));
        dialog.signal(BnetSignal::TextInput);
        assert_eq!(dialog.prompt("Enter new Volume name: "),
            Some((String::new(), "Enter new Volume name: ".to_string())));
    }

    #[test]
    fn multiline_prompt_prints_all_but_last_line() {
        let mut dialog = InputDialog::default();
        dialog.signal(BnetSignal::SelectInput);
        assert_eq!(dialog.prompt("Defined Storages:\nSelect Storage (1-3): "),
            Some(("Defined Storages:\n".to_string(), "Select Storage (1-3): ".to_string())));
        dialog.signal(BnetSignal::SelectInput);
        assert_eq!(dialog.prompt("text\n"), Some(("text\n".to_string(), "Select: ".to_string())));
    }

    #[test]
    fn other_signals_do_not_open_a_dialog() {
        let mut dialog = InputDialog::default();
        for signal in [BnetSignal::Eod, BnetSignal::MainPrompt, BnetSignal::SubPrompt] {
            dialog.signal(signal);
        }
        assert_eq!(dialog.prompt("x"), None);
    }

    #[test]
    fn completion_consumes_original_help_usage_instead_of_names_only() {
        let status=BareosHelper::parse_help_line("status Report status -- status [dir | director | client=<client-name> | storage=<storage-name>]").unwrap();
        assert_eq!(status.name,"status");assert_eq!(status.description,"Report status");
        for value in ["dir","director","client=","storage="] { assert!(status.completions.contains(&value.to_string())); }
        assert!(status.live_queries.contains(&("client".into(),".clients".into())));
        assert!(status.live_queries.contains(&("storage".into(),".storages".into())));
        let job=BareosHelper::parse_help_line("run Run a job -- run [job=<job-name> level=<level> yes]").unwrap();
        assert!(job.completions.contains(&"job=".to_string()));assert!(job.completions.contains(&"yes".to_string()));
        assert!(BareosHelper::parse_help_line("run").is_none());
        let legacy=BareosHelper::parse_help_line("status\tReport status\tstatus [dir]\t!dir,client").unwrap();
        assert!(legacy.completions.contains(&"dir".to_string()));
    }

    #[test]
    fn cli_defaults() {
        let cli = Cli::parse_from(["oconsole"]);
        assert!(cli.address.is_none());
        assert!(cli.port.is_none());
        assert!(cli.name.is_none());
        assert!(!cli.grpc);
        assert!(cli.command.is_none());
        assert!(!cli.mcp);
        assert!(!cli.mcp_write_enabled());
    }

    #[cfg(feature = "mcp-development")]
    #[test]
    fn mcp_allow_write_requires_mcp() {
        assert!(Cli::try_parse_from(["oconsole", "--mcp-allow-write"]).is_err());
        let cli = Cli::parse_from(["oconsole", "--mcp", "--mcp-allow-write", "--grpc"]);
        assert!(cli.mcp && cli.mcp_write_enabled() && cli.grpc);
    }

    #[cfg(feature = "mcp-production")]
    #[test]
    fn production_has_no_write_flag() {
        assert!(Cli::try_parse_from(["oconsole", "--mcp", "--mcp-allow-write"]).is_err());
        let cli = Cli::parse_from(["oconsole", "--mcp"]);
        assert!(!cli.mcp_write_enabled());
    }

    #[test]
    fn mcp_http_options_require_server_mode() {
        assert!(Cli::try_parse_from(["oconsole", "--mcp-http", "127.0.0.1:9104"]).is_err());
        assert!(Cli::try_parse_from(["oconsole", "--mcp", "--mcp-token-file", "/tmp/token"]).is_err());
        let cli = Cli::parse_from(["oconsole", "--mcp", "--mcp-http", "127.0.0.1:9104", "--mcp-token-file", "/tmp/token"]);
        assert_eq!(cli.mcp_http.unwrap().port(), 9104);
    }

    #[test]
    fn cli_with_args() {
        let cli = Cli::parse_from([
            "oconsole", "-a", "10.0.0.1", "-p", "9102",
            "-n", "admin", "-e", "status dir",
        ]);
        assert_eq!(cli.address, Some("10.0.0.1".to_string()));
        assert_eq!(cli.port, Some(9102));
        assert_eq!(cli.name, Some("admin".to_string()));
        assert_eq!(cli.command, Some("status dir".to_string()));
    }
}
