//! Onesimus Console (oconsole) — Interactive console client for the Director.
//!
//! Connects via BNET (TCP + TLS + CRAM-MD5) or gRPC (TLS + pinned certificate).
//! With --mcp it serves the Model Context Protocol on stdio instead of a prompt.

#[cfg(all(feature = "mcp-production", feature = "mcp-development"))]
compile_error!("MCP production and development policies are mutually exclusive; use --no-default-features --features mcp-development for development");
#[cfg(not(any(feature = "mcp-production", feature = "mcp-development")))]
compile_error!("Select an MCP policy: mcp-production (default) or mcp-development");

mod color;
mod fishkeys;
mod grammar;

// Keep all console stdout/stderr decoration under the same NO_COLOR policy.
macro_rules! print { ($($arg:tt)*) => { std::print!("{}", crate::color::stdout(&format!($($arg)*))) }; }
macro_rules! println { () => { std::println!() }; ($($arg:tt)*) => { std::println!("{}", crate::color::stdout(&format!($($arg)*))) }; }
macro_rules! eprintln { () => { std::eprintln!() }; ($($arg:tt)*) => { std::eprintln!("{}", crate::color::stderr(&format!($($arg)*))) }; }

mod grpc_client;
mod mcp;
mod mcp_read;
mod mcp_mirror;
mod mcp_write;
mod mcp_config;
mod mcp_tree;
mod mcp_typed;
#[cfg(test)]
mod mcp_coverage;
mod mcp_token;
mod rate_guard;

use std::io::{self, BufRead, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Parser;
use tracing::debug;
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

    /// Use gRPC (default: tried first, classic BNET when the Director's gRPC port is
    /// not reachable); with this option there is no fallback
    #[arg(long, conflicts_with = "bnet")]
    grpc: bool,

    /// Use the classic Bareos console protocol (BNET) only
    #[arg(long)]
    bnet: bool,

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

    /// Development builds only: offer the write and restore tools over
    /// --mcp-http too (stdio offers them in every build)
    #[cfg(feature = "mcp-development")]
    #[arg(long, requires = "mcp")]
    mcp_allow_write: bool,

    /// MCP: offer only the read tools, also on stdio
    #[arg(long, requires = "mcp")]
    mcp_read_only: bool,

    /// Execute single command and exit
    #[arg(short = 'e', long)]
    command: Option<String>,
}

impl Cli {
    /// Write and restore tools for this MCP transport (C-087): stdio unless
    /// --mcp-read-only; HTTP only in a development build with --mcp-allow-write.
    fn mcp_write_enabled(&self) -> bool {
        #[cfg(feature = "mcp-development")]
        let allow = self.mcp_allow_write;
        #[cfg(feature = "mcp-production")]
        let allow = false;
        self.mcp && mcp::write_access(self.mcp_http.is_some(), allow, self.mcp_read_only)
    }
}

/// Parsed config from oconsole.conf / bconsole.conf
struct ConsoleConfig {
    address: String,
    port: u16,
    name: String,
    password: String,
    /// oconsole extensions (written by the Onesimus GUI): Grpc = yes,
    /// Grpc Port = <n>, Grpc Fingerprint = <sha256 hex>; `Grpc = no` selects BNET
    grpc: Option<bool>,
    grpc_port: Option<u16>,
    grpc_fingerprint: Option<String>,
    /// Every `Console { Name Password }` of the file, in file order: the
    /// human login (first one without the MCP suffix) and the MCP login
    /// (`<name>-mcp`, see [`mcp_login`]).
    consoles: Vec<(String, String)>,
}

/// Suffix of the console the MCP server logs in as (decision of the
/// maintainer, 19.11.2026): the MCP path MUST use its own Console resource
/// `<console>-mcp`, defined in the Director's configuration with its own
/// (narrower) ACLs and in oconsole.conf with its own password.
const MCP_CONSOLE_SUFFIX: &str = "-mcp";

/// The login (console, password) of the MCP server.
///
/// - `--name` ending in `-mcp`: that console, password from `--password` or
///   from its `Console` resource in the file.
/// - otherwise `<human console>-mcp`, where the human console is `--name` or
///   the first `Console` of the file without the suffix; its password comes
///   from the `Console` resource of that name in the file (never the
///   human console's password).
///
/// Errors when there is no named human console (`*UserAgent*` has no MCP
/// counterpart) or no password for the MCP console: the MCP server then does
/// not start instead of falling back to the human console's rights.
fn mcp_login(cfg: &ConsoleConfig, cli_name: Option<&str>, cli_password: Option<&str>) -> Result<(String, String), String> {
    let explicit = cli_name.filter(|n| n.ends_with(MCP_CONSOLE_SUFFIX));
    let target = match explicit {
        Some(n) => n.to_string(),
        None => {
            let base = cli_name.map(str::to_string).unwrap_or_else(|| cfg.name.clone());
            if base.is_empty() || base == "*UserAgent*" {
                return Err(format!("the MCP server logs in as its own console <name>{MCP_CONSOLE_SUFFIX}, but the \
                    configuration names no console (only *UserAgent*). Add `Console {{ Name = <name>{MCP_CONSOLE_SUFFIX}; \
                    Password = ... }}` to oconsole.conf and the same Console with narrower ACLs to the Director"));
            }
            format!("{base}{MCP_CONSOLE_SUFFIX}")
        }
    };
    let from_file = cfg.consoles.iter().find(|(n, _)| *n == target).map(|(_, p)| p.clone());
    let password = match (explicit, cli_password) {
        (Some(_), Some(p)) => Some(p.to_string()),
        _ => from_file,
    };
    match password {
        Some(p) if !p.is_empty() => Ok((target, p)),
        _ => Err(format!("the MCP server logs in as console \"{target}\", but oconsole.conf has no \
            `Console {{ Name = {target}; Password = ... }}`. Add it there and the Console \"{target}\" with \
            narrower ACLs to the Director's configuration (in the editor); the MCP never uses the rights of \
            the human console")),
    }
}

/// Effective CLI — resolved from config file + CLI overrides
struct EffCli {
    address: String,
    port: u16,
    name: String,
    password: String,
    grpc: bool,
    /// gRPC was asked for (--grpc, `Grpc = yes`): no fallback to BNET.
    grpc_required: bool,
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
        grpc: None,
        grpc_port: None,
        grpc_fingerprint: None,
        consoles: Vec::new(),
    };
    let mut director_password = String::new();
    // the Console resource being read
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
                if block.as_deref() == Some("console") {
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
            if depth == 0 {
                if block.as_deref() == Some("console")
                    && let Some(c) = console.take().filter(|c| !c.0.is_empty()) {
                    cfg.consoles.push(c);
                }
                block = None;
            }
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
            (_, "grpc") => cfg.grpc = Some(matches!(val.to_ascii_lowercase().as_str(), "yes" | "true" | "1" | "on")),
            (_, "grpc port" | "grpcport") => cfg.grpc_port = val.parse().ok(),
            (_, "grpc fingerprint" | "grpcfingerprint") => {
                cfg.grpc_fingerprint = Some(val.replace(':', "").to_ascii_lowercase())
            }
            _ => {}
        }
    }

    // the human login: the first Console without the MCP suffix
    match cfg.consoles.iter().find(|(n, _)| !n.ends_with(MCP_CONSOLE_SUFFIX)).cloned() {
        Some((name, password)) => {
            cfg.name = name;
            cfg.password = password;
        }
        None => cfg.password = director_password,
    }
    cfg
}

fn main() {
    let cli = Cli::parse();
    let mcp_allow_write = cli.mcp_write_enabled();

    // Load config file, then override with CLI args
    let file_cfg = load_console_config(cli.config.as_deref());

    // CLI args override config file
    let address = cli.address.clone().unwrap_or(file_cfg.address.clone());
    let port = cli.port.unwrap_or(file_cfg.port);
    let (name, password) = if cli.mcp {
        match mcp_login(&file_cfg, cli.name.as_deref(), cli.password.as_deref()) {
            Ok(login) => login,
            Err(e) => {
                eprintln!("MCP: {e}");
                std::process::exit(2);
            }
        }
    } else {
        (cli.name.clone().unwrap_or(file_cfg.name.clone()), cli.password.clone().unwrap_or(file_cfg.password.clone()))
    };

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
        grpc: !cli.bnet && (cli.grpc || file_cfg.grpc != Some(false)),
        grpc_required: !cli.bnet && (cli.grpc || file_cfg.grpc == Some(true)),
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
        libc::signal(libc::SIGINT, sigint_handler as *const () as libc::sighandler_t);
    }

    let addr = format!("{}:{}", eff.address, eff.port);

    // gRPC is the default; a Director without it (classic Bareos) is reached by BNET.
    let grpc_addr = format!("{}:{}", eff.address, eff.grpc_port);
    if eff.grpc && !eff.grpc_required && !port_reachable(&grpc_addr) {
        eprintln!("oconsole: no gRPC on {grpc_addr}, using the classic console protocol (BNET)");
        run_bnet_mode(&EffCli { grpc: false, ..eff }, &addr);
    } else if eff.grpc {
        run_grpc_mode(&eff);
    } else {
        run_bnet_mode(&eff, &addr);
    }
}

/// A TCP connect to `addr` works within a few seconds.
fn port_reachable(addr: &str) -> bool {
    use std::net::ToSocketAddrs;
    addr.to_socket_addrs().ok().is_some_and(|mut all| {
        all.any(|a| TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok())
    })
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
                let bnet = BnetStream::new(psk_stream);
                run_bnet_session(cli, bnet);
                return;
            }
            Err(e) => {
                debug!("TLS-PSK failed ({e}), reconnecting plaintext");
                let stream2 = TcpStream::connect(addr).unwrap();
                stream2.set_read_timeout(Some(Duration::from_secs(30))).ok();
                let bnet = BnetStream::new(stream2);
                run_bnet_session(cli, bnet);
                return;
            }
        }
    }
    let bnet = BnetStream::new(stream);
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
    let mut director_name = "bareos-dir".to_string();
    for _ in 0..2 {
        match bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                let text = String::from_utf8_lossy(&data);
                if let Some(name) = director_from_banner(&text) {
                    director_name = name;
                }
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
        eprintln!("oconsole: serving MCP{}", if cli.mcp_allow_write { " (read and write tools)" } else { " (read-only tools)" });
        mcp::McpServer::new(BnetRunner { bnet, dialog: false }, cli.mcp_allow_write, "bareos-dir".to_string()).serve_transport(cli.mcp_http, cli.mcp_token_file.clone(), cli.mcp_host.clone()).unwrap_or_else(|e| { eprintln!("MCP: {e}"); std::process::exit(1) });
        return;
    }

    // Single command mode
    if let Some(cmd) = &cli.command {
        execute_single_command(&mut bnet, cmd);
        return;
    }

    // Interactive mode
    interactive_loop(Box::new(BnetConn { bnet }), &cli.name, &director_name);
}

/// The Director's name from the BNET banner (`1000 OK: <name> Version: …`,
/// authenticate_console.cc:129, also with the record separator after 1000);
/// None for a banner without it.
fn director_from_banner(text: &str) -> Option<String> {
    let rest = text.split_once("OK: ")?.1;
    let name = rest.split_once(" Version:")?.0.trim();
    (!name.is_empty() && !name.contains(char::is_whitespace)).then(|| name.to_string())
}

#[cfg(test)]
mod banner_tests {
    #[test]
    fn director_name_from_the_banner() {
        use super::director_from_banner as d;
        assert_eq!(d("1000 OK: PDC-dir Version: 25.1.2+onesimus.0.2.0 (19 November 2026)").as_deref(), Some("PDC-dir"));
        assert_eq!(d("1000\x1eOK: bareos-dir Version: 25.1.2 (09 October 2026) -- TLS").as_deref(), Some("bareos-dir"));
        assert_eq!(d("1000 OK"), None);
        assert_eq!(d("1002 fake Director for rate-guard tests"), None);
    }
}

/// MCP command runner over an authenticated BNET session (api=0: response ends with EOD).
struct BnetRunner<S: std::io::Read + std::io::Write> {
    bnet: BnetStream<S>,
    /// A dialog (restore file selection) waits for an answer.
    dialog: bool,
}

impl<S: std::io::Read + std::io::Write> BnetRunner<S> {
    /// Read up to the next question (SubPrompt) or the end of the command.
    fn read_step(&mut self) -> anyhow::Result<mcp::Dialog> {
        let mut output = String::new();
        loop {
            match self.bnet.recv() {
                Ok(BnetMessage::Data(data)) => output.push_str(&String::from_utf8_lossy(&data)),
                Ok(BnetMessage::Signal(BnetSignal::SubPrompt)) => {
                    self.dialog = true;
                    return Ok(mcp::Dialog { output, waiting: true });
                }
                Ok(BnetMessage::Signal(BnetSignal::Eod)) | Ok(BnetMessage::Signal(BnetSignal::MainPrompt)) => {
                    self.dialog = false;
                    return Ok(mcp::Dialog { output, waiting: false });
                }
                Err(e) if is_read_timeout(&e) => continue,
                Ok(BnetMessage::Signal(BnetSignal::Terminate)) => anyhow::bail!("Director closed the connection"),
                Ok(BnetMessage::Signal(_)) => continue,
                Err(e) => anyhow::bail!("connection error: {e}"),
            }
        }
    }
}

impl<S: std::io::Read + std::io::Write + Send + 'static> mcp::CommandRunner for BnetRunner<S> {
    /// Any question of the Director is cancelled with "." so that a command
    /// never leaves the session waiting for input.
    fn run(&mut self, command: &str) -> anyhow::Result<String> {
        self.run_dialog(command, &mut |_| None)
    }

    fn dialog_start(&mut self, command: &str) -> anyhow::Result<mcp::Dialog> {
        if self.dialog {
            anyhow::bail!("a Director dialog is still open");
        }
        self.bnet.send_str(command)?;
        self.read_step()
    }

    fn dialog_send(&mut self, reply: &str) -> anyhow::Result<mcp::Dialog> {
        if !self.dialog {
            anyhow::bail!("no Director dialog is open");
        }
        self.bnet.send_str(reply)?;
        self.read_step()
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
                // The socket's 30 s read timeout only means the command is still
                // running (wait, a long restore selection): keep reading, or the
                // late reply would become the answer to the next tool call.
                Err(e) if is_read_timeout(&e) => continue,
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

    /// The reply of `cmd` in JSON (API mode 2), as text.
    fn query_json(&mut self, cmd: &str) -> String {
        self.query(".api 2");
        let lines = self.query(cmd);
        self.query(".api 0");
        lines.join("\n")
    }
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

/// rustyline Helper: completion, fish-style suggestions and coloring from the
/// Director's console grammar ([`grammar`]).
struct BareosHelper {
    /// The grammar: from `.grammar`, else rebuilt from `.help`
    grammar: grammar::Grammar,
    rate_guard: rate_guard::CommandRateGuard,
    /// Cached values from Director for tab-completion (loaded at connect)
    /// Maps: dot-command → list of values (e.g. ".clients" → ["bareos-fd", "remote-fd"])
    value_cache: grammar::ValueCache,
    /// Lines the Director refused in this session: never suggested
    failed: Vec<String>,
    /// Esc: no suggestion while the line is this text (set by the key handler)
    hidden_for: fishkeys::HiddenHint,
}

impl BareosHelper {
    fn new() -> Self {
        Self {
            grammar: grammar::Grammar::default(),
            rate_guard: rate_guard::CommandRateGuard::default(),
            value_cache: grammar::ValueCache::new(),
            failed: Vec::new(),
            hidden_for: fishkeys::HiddenHint::default(),
        }
    }

    /// Reload everything from the Director (after `reload`/`configure`).
    fn refresh(&mut self, dir: &mut dyn DirectorConn) {
        self.value_cache.clear();
        self.load_from_director(dir);
    }

    /// Load the grammar and the cached values at connect time.
    /// All queries happen sequentially over one connection before readline starts.
    fn load_from_director(&mut self, dir: &mut dyn DirectorConn) {
        self.grammar = load_grammar(dir);
        // Cache all dot-command values that are used for live-completion.
        // Query each unique dot-command once at connect time.
        for source in self.grammar.value_sources() {
            let values = dir.query(&source);
            self.value_cache.insert(source, values);
        }
    }

    /// One line for the connect and refresh messages.
    fn summary(&self) -> String {
        format!("{} commands, {} subcommands and arguments{}, {} cached values for tab-completion",
            self.grammar.commands.len(), self.grammar.element_count(),
            if self.grammar.typed { "" } else { " (from .help)" },
            self.value_cache.values().map(|v| v.len()).sum::<usize>())
    }
}

/// `.api 2; .grammar; .api 0`; a Director without `.grammar` (Bareos, older
/// odir: "is an invalid command") gets the model rebuilt from `.help`.
fn load_grammar(dir: &mut dyn DirectorConn) -> grammar::Grammar {
    let reply = dir.query_json(".grammar");
    if let Some(typed) = serde_json::from_str(&reply).ok().and_then(|v| grammar::Grammar::from_grammar_json(&v)) {
        return typed;
    }
    // Bareos API1 .help carries descriptions and original usage;
    // .help all intentionally contains only non-dot command names.
    dir.query(".api 1");
    let lines = dir.query(".help");
    dir.query(".api 0");
    let fallback = grammar::Grammar::from_help_lines(&lines);
    if fallback.commands.is_empty() { grammar::Grammar::minimal() } else { fallback }
}

/// Words of a command's grammar that are colored as known keywords.
fn known_words(node: &grammar::Node, out: &mut Vec<String>) {
    out.extend(node.args.iter().map(|a| a.name.clone()));
    for sub in &node.subs {
        out.push(sub.word.clone());
        out.extend(sub.aliases.iter().cloned());
        known_words(sub, out);
    }
}

/// The suggestion shown after the cursor; `completion` None for a usage hint.
struct ConsoleHint {
    display: String,
    completion: Option<String>,
}

impl rustyline::hint::Hint for ConsoleHint {
    fn display(&self) -> &str {
        &self.display
    }

    fn completion(&self) -> Option<&str> {
        self.completion.as_deref()
    }
}

impl Completer for BareosHelper {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, candidates) = grammar::complete(&self.grammar, &self.value_cache, line, pos);
        Ok((start, candidates.into_iter().map(|c| Pair { display: c.display, replacement: c.replacement }).collect()))
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

        let entry = self.grammar.command(cmd);

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
        let mut known = Vec::new();
        if let Some(node) = entry {
            known_words(node, &mut known);
        }
        let known_completions: Vec<&str> = known.iter().map(String::as_str).collect();

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
    type Hint = ConsoleHint;

    /// fish autosuggestion (history, a single completion, usage), only at the end of the line.
    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<ConsoleHint> {
        if self.hidden_for.is_hidden(line) {
            return None;
        }
        let history = ctx.history();
        let newest_first: Vec<String> = (0..history.len()).rev()
            .filter_map(|i| history.get(i, rustyline::history::SearchDirection::Forward).ok().flatten())
            .map(|r| r.entry.into_owned())
            .collect();
        grammar::suggest(&self.grammar, &self.value_cache, &newest_first, &self.failed, line, pos)
            .map(|s| ConsoleHint { display: s.display, completion: s.completion })
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

/// Output mode of the session as switched with `.api` (C++ DotApiCmd).
/// BNET keeps it in the Director's session; gRPC calls are stateless, so
/// the console sends it with every ExecuteCommand request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ApiMode {
    /// API 0: human-readable text
    #[default]
    Text,
    /// API 1: framed text for programs
    Api,
    /// API 2: JSON-RPC envelope
    Json,
}

impl ApiMode {
    /// The mode a `.api` line switches to, with the Director's rules
    /// (odir `parse_dot_api`): no argument means 1, more than two arguments
    /// or an unknown keyword fail. None for every other line.
    fn from_line(line: &str) -> Option<ApiMode> {
        let mut words = line.split_whitespace();
        if !words.next()?.eq_ignore_ascii_case(".api") {
            return None;
        }
        let args: Vec<&str> = words.collect();
        if args.len() > 2 {
            return None;
        }
        let Some(first) = args.first() else { return Some(ApiMode::Api) };
        match first.to_ascii_lowercase().as_str() {
            "off" | "0" => Some(ApiMode::Text),
            "on" | "1" => Some(ApiMode::Api),
            "json" | "2" => Some(ApiMode::Json),
            _ => None,
        }
    }

    /// The `.api` line that selects this mode again.
    fn command(self) -> &'static str {
        match self {
            ApiMode::Text => ".api 0",
            ApiMode::Api => ".api 1",
            ApiMode::Json => ".api 2",
        }
    }

    /// The per-request format of a gRPC ExecuteCommand call in this mode.
    fn grpc_format(self) -> onesimus_proto_rs::onesimus::director::ResponseFormat {
        use onesimus_proto_rs::onesimus::director::ResponseFormat;
        match self {
            ApiMode::Text => ResponseFormat::Text,
            ApiMode::Api => ResponseFormat::Api,
            ApiMode::Json => ResponseFormat::Json,
        }
    }
}

/// Replies are coloured only for a person: text mode (never API 1/2, whose
/// output programs parse) on a terminal, and not under NO_COLOR.
fn reply_coloured(mode: ApiMode, stdout_is_tty: bool, color_enabled: bool) -> bool {
    mode == ApiMode::Text && stdout_is_tty && color_enabled
}

/// One reply line, coloured by keyword when `coloured`.
fn reply_line(line: &str, coloured: bool) -> std::borrow::Cow<'_, str> {
    if !coloured {
        return std::borrow::Cow::Borrowed(line);
    }
    let colour = if line.contains("Error") || line.contains("FEHLER") || line.contains("failed") {
        "\x1b[31m"
    } else if line.contains("Warning") || line.contains("WARNUNG") {
        "\x1b[33m"
    } else if line.contains("OK") || line.contains("success") {
        "\x1b[32m"
    } else {
        return std::borrow::Cow::Borrowed(line);
    };
    std::borrow::Cow::Owned(format!("{colour}{line}\x1b[0m"))
}

/// Reply text, coloured by keyword in text mode on a terminal only.
fn print_reply(text: &str, mode: ApiMode) {
    use std::io::IsTerminal;
    let coloured = reply_coloured(mode, io::stdout().is_terminal(), color::enabled());
    for line in text.lines() {
        println!("{}", reply_line(line, coloured));
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
    eprintln!("\x1b[2m({})\x1b[0m", helper.summary());
    let hidden_hint = helper.hidden_for.clone();

    let rl_config = rustyline::Config::builder()
        .completion_type(rustyline::CompletionType::List)
        .edit_mode(rustyline::EditMode::Emacs)
        .auto_add_history(false)
        .build();
    let mut rl = Editor::with_config(rl_config).unwrap();
    rl.set_helper(Some(helper));
    let shared_history = fishkeys::SharedHistory::default();
    fishkeys::bind(&mut rl, hidden_hint, shared_history.clone());
    let refresh = if color::enabled() {
        rl.create_external_printer().map(rate_guard::PromptRefresh::new).unwrap_or_default()
    } else { rate_guard::PromptRefresh::default() };

    let history_file = dirs_home().join(".oconsole_history");
    let _ = rl.load_history(&history_file);
    shared_history.set(rl.history().iter().cloned());

    let version = env!("CARGO_PKG_VERSION");
    let prompt = format!("{}@{}[{}]> ", console_name, director_name, version);
    // The session's output mode, as last switched with `.api`
    let mut api_mode = ApiMode::default();

    loop {
        // Kein drain() hier — der State ist nach Init/Command-Response synchron.
        // Das naechste MainPrompt wird vom Director gesendet wenn wir einen Command senden.

        match terminal_readline(&mut rl, &prompt, &guard, &refresh) {
            Ok(line) => {
                let cmd = line.trim();
                if cmd.is_empty() { continue; }

                // help <command> from the grammar (odir's man page text), not sent
                if let Some(text) = rl.helper().and_then(|h| grammar::local_help(&h.grammar, cmd)) {
                    print!("{text}");
                    rl.add_history_entry(cmd).ok();
                    shared_history.push(cmd);
                    continue;
                }

                match send_terminal_line(dir.as_mut(), &guard, cmd) {
                    None => continue,
                    Some(false) => { eprintln!("Connection lost"); break; }
                    Some(true) => { rl.add_history_entry(cmd).ok(); shared_history.push(cmd); }
                }
                if let Some(mode) = ApiMode::from_line(cmd) {
                    api_mode = mode;
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
                // the Director refused the line: not suggested again
                let mut refused = false;
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
                                print_reply(&before, api_mode);
                                if !read_guarded_dialog(&mut rl, dir.as_mut(), &guard, &refresh,
                                    &dialog_prompt, false) {
                                    let _ = rl.save_history(&history_file);
                                    return;
                                }
                                continue;
                            }
                            refused |= text.contains("is an invalid command");
                            print_reply(&text, api_mode);
                        }
                        Some(RecvResult::Signal(BnetSignal::CmdFailed)) => refused = true,
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

                if refused {
                    if let Some(helper) = rl.helper_mut() {
                        helper.failed.push(cmd.to_string());
                    }
                }

                // The Director's resources may have changed: refresh the
                // completion data (grammar, job/client/pool names, ...)
                let first = cmd.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                if matches!(first.as_str(), "reload" | "configure") {
                    if let Some(helper) = rl.helper_mut() {
                        helper.refresh(dir.as_mut());
                        // the loader leaves the Director in API 0
                        if api_mode != ApiMode::Text {
                            dir.query(api_mode.command());
                        }
                        eprintln!("\x1b[2m(tab-completion refreshed: {})\x1b[0m", helper.summary());
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

/// A read that timed out (EAGAIN/EWOULDBLOCK, ETIMEDOUT) or was interrupted
/// before data arrived: not fatal, read again.
fn is_read_timeout(e: &BnetError) -> bool {
    match e {
        BnetError::Io(io_err) => matches!(
            io_err.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
        ),
        other => other.to_string().contains("os error 11"),
    }
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
    use onesimus_proto_rs::onesimus::director::ResponseFormat;

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
        eprintln!("oconsole: serving MCP{}", if cli.mcp_allow_write { " (read and write tools)" } else { " (read-only tools)" });
        let name = session.director_name.clone();
        let runner = GrpcRunner { rt, session, unimplemented: Default::default() };
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
    interactive_loop(Box::new(GrpcConn { rt, session, pending: Vec::new(), api_mode: ApiMode::default() }), &cli.name, &name);
}

/// gRPC implementation of DirectorConn: each command is one ExecuteCommand
/// call; the reply is handed to the interactive loop as Data + EOD.
struct GrpcConn {
    rt: tokio::runtime::Runtime,
    session: grpc_client::GrpcSession,
    pending: Vec<RecvResult>,
    /// The mode set with `.api`; ExecuteCommand is stateless, so every
    /// command carries it as its response format.
    api_mode: ApiMode,
}

impl GrpcConn {
    fn execute(&mut self, cmd: &str, format: onesimus_proto_rs::onesimus::director::ResponseFormat) -> Result<String, String> {
        let out = self.rt.block_on(self.session.execute(cmd, format)).map_err(|e| format!("gRPC error: {e}"))?;
        // A JSON reply carries its errors in the envelope; appending them would break the JSON
        let json = format == onesimus_proto_rs::onesimus::director::ResponseFormat::Json && !out.text.is_empty();
        Ok(if out.error.is_empty() || json { out.text } else { format!("{}{}", out.text, out.error) })
    }
}

impl DirectorConn for GrpcConn {
    fn query(&mut self, cmd: &str) -> Vec<String> {
        use onesimus_proto_rs::onesimus::director::ResponseFormat;
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

    fn query_json(&mut self, cmd: &str) -> String {
        use onesimus_proto_rs::onesimus::director::ResponseFormat;
        self.execute(cmd, ResponseFormat::Json).unwrap_or_default()
    }

    fn send_cmd(&mut self, cmd: &str) -> bool {
        if matches!(cmd, "quit" | "exit" | ".quit") {
            // Like the BNET Director: quit produces no output
            self.pending = vec![RecvResult::Closed];
            return true;
        }
        // As on BNET, `.api` answers in the new mode and sets it for what follows
        if let Some(mode) = ApiMode::from_line(cmd) {
            self.api_mode = mode;
        }
        let text = match self.execute(cmd, self.api_mode.grpc_format()) {
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
    /// RPCs the Director answered with UNIMPLEMENTED (asked once).
    unimplemented: std::collections::HashSet<String>,
}

impl mcp::CommandRunner for GrpcRunner {
    fn run(&mut self, command: &str) -> anyhow::Result<String> {
        use onesimus_proto_rs::onesimus::director::ResponseFormat;
        let out = self.rt.block_on(self.session.execute(command, ResponseFormat::Text))?;
        if !out.success && out.text.is_empty() {
            anyhow::bail!("{}", out.error.trim_end());
        }
        Ok(if out.error.is_empty() { out.text } else { format!("{}{}", out.text, out.error) })
    }

    fn run_json(&mut self, command: &str) -> anyhow::Result<String> {
        use onesimus_proto_rs::onesimus::director::ResponseFormat;
        let out = self.rt.block_on(self.session.execute(command, ResponseFormat::Json))?;
        if !out.success && out.text.is_empty() {
            anyhow::bail!("{}", out.error.trim_end());
        }
        Ok(out.text)
    }

    fn is_grpc(&self) -> bool {
        true
    }

    fn typed_call(&mut self, rpc: &str, request: Vec<u8>) -> Option<Result<Vec<u8>, mcp::TypedError>> {
        if self.unimplemented.contains(rpc) {
            return Some(Err(mcp::TypedError::Unimplemented));
        }
        Some(match self.rt.block_on(self.session.call_typed(rpc, request)) {
            Ok(bytes) => Ok(bytes),
            Err(status) if status.code() == tonic::Code::Unimplemented => {
                self.unimplemented.insert(rpc.to_string());
                Err(mcp::TypedError::Unimplemented)
            }
            Err(status) => Err(mcp::TypedError::Status(format!("{:?}", status.code()), status.message().to_string())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Director side for BnetRunner: canned reads (Err = a read timeout), recorded writes.
    struct Scripted {
        reads: std::collections::VecDeque<Result<Vec<u8>, std::io::ErrorKind>>,
        written: Vec<u8>,
    }
    impl std::io::Read for Scripted {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self.reads.pop_front() {
                None => Ok(0),
                Some(Err(kind)) => Err(kind.into()),
                Some(Ok(mut chunk)) => {
                    let n = chunk.len().min(buf.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    if n < chunk.len() {
                        self.reads.push_front(Ok(chunk.split_off(n)));
                    }
                    Ok(n)
                }
            }
        }
    }
    impl std::io::Write for Scripted {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    fn frame(text: &str) -> Result<Vec<u8>, std::io::ErrorKind> {
        let mut f = (text.len() as i32).to_be_bytes().to_vec();
        f.extend_from_slice(text.as_bytes());
        Ok(f)
    }
    fn signal(sig: BnetSignal) -> Result<Vec<u8>, std::io::ErrorKind> {
        Ok((sig as i32).to_be_bytes().to_vec())
    }
    fn sent_lines(written: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        let mut rest = written;
        while rest.len() >= 4 {
            let len = i32::from_be_bytes(rest[..4].try_into().unwrap());
            rest = &rest[4..];
            if len > 0 {
                lines.push(String::from_utf8_lossy(&rest[..len as usize]).to_string());
                rest = &rest[len as usize..];
            }
        }
        lines
    }

    #[test]
    fn mcp_runner_waits_through_read_timeouts() {
        use mcp::CommandRunner;
        let reads = vec![
            Err(std::io::ErrorKind::WouldBlock),
            Err(std::io::ErrorKind::TimedOut),
            frame("JobId=7\nJobStatus=OK (T)\n"),
            signal(BnetSignal::Eod),
        ];
        let mut runner = BnetRunner { bnet: BnetStream::new(Scripted { reads: reads.into(), written: vec![] }), dialog: false };
        assert_eq!(runner.run("wait jobid=7").unwrap(), "JobId=7\nJobStatus=OK (T)\n");
    }

    #[test]
    fn mcp_runner_answers_or_cancels_questions() {
        use mcp::CommandRunner;
        let reads = vec![
            frame("Found 1 Jobs\n"), frame("Purge (yes/no)? "), signal(BnetSignal::SubPrompt),
            frame("Select Pool (1-2): "), signal(BnetSignal::SubPrompt),
            frame("done\n"), signal(BnetSignal::Eod),
        ];
        let mut runner = BnetRunner { bnet: BnetStream::new(Scripted { reads: reads.into(), written: vec![] }), dialog: false };
        let out = runner
            .run_dialog("purge volume=A", &mut |out| out.ends_with("Purge (yes/no)? ").then(|| "yes".to_string()))
            .unwrap();
        assert_eq!(out, "Found 1 Jobs\nPurge (yes/no)? yes\nSelect Pool (1-2): .\ndone\n");
        assert_eq!(sent_lines(&runner.bnet.into_inner().written), vec!["purge volume=A", "yes", "."]);
    }

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

    /// The MCP server logs in only as `<console>-mcp` (19.11.2026).
    #[test]
    fn mcp_logs_in_as_its_own_console() {
        let file = concat!("Director {\n  Name = PDC-dir\n  Address = h\n  Password = \"dirpw\"\n}\n",
            "Console {\n  Name = bconsole-mcp\n  Password = \"mcppw\"\n}\n",
            "Console {\n  Name = bconsole\n  Password = \"conpw\"\n}\n");
        let cfg = parse_console_config(file);
        // the human login skips the MCP console even when it comes first
        assert_eq!((cfg.name.as_str(), cfg.password.as_str()), ("bconsole", "conpw"));
        assert_eq!(mcp_login(&cfg, None, None), Ok(("bconsole-mcp".into(), "mcppw".into())));
        // --name of the human console: still its -mcp console, never its password
        assert_eq!(mcp_login(&cfg, Some("bconsole"), Some("conpw")), Ok(("bconsole-mcp".into(), "mcppw".into())));
        // --name/--password of an -mcp console
        assert_eq!(mcp_login(&cfg, Some("x-mcp"), Some("p")), Ok(("x-mcp".into(), "p".into())));
        // no -mcp console in the file: no start
        let human_only = parse_console_config("Console {\n  Name = bconsole\n  Password = \"conpw\"\n}\n");
        assert!(mcp_login(&human_only, None, None).unwrap_err().contains("bconsole-mcp"));
        // *UserAgent* has no MCP counterpart
        let agent = parse_console_config("Director {\n  Password = \"dirpw\"\n}\n");
        assert!(mcp_login(&agent, None, None).unwrap_err().contains("*UserAgent*"));
    }

    #[test]
    fn flat_file_and_grpc_keys_still_work() {
        let cfg = parse_console_config("address = h\nport = 1\npassword = p\nGrpc = yes\nGrpc Port = 7\n");
        assert_eq!((cfg.address.as_str(), cfg.port, cfg.name.as_str(), cfg.password.as_str()), ("h", 1, "*UserAgent*", "p"));
        assert_eq!(cfg.grpc, Some(true));
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

    fn help_grammar(lines: &[&str]) -> grammar::Grammar {
        grammar::Grammar::from_help_lines(&lines.iter().map(|l| l.to_string()).collect::<Vec<_>>())
    }

    fn arg_words(node: &grammar::Node) -> Vec<String> {
        node.args.iter().map(|a| match a.kind {
            grammar::Kind::Keyword => a.name.clone(),
            grammar::Kind::Value => format!("{}=", a.name),
        }).collect()
    }

    #[test]
    fn completion_consumes_original_help_usage_instead_of_names_only() {
        let g = help_grammar(&["status Report status -- status [dir | director | client=<client-name> | storage=<storage-name>]"]);
        let status = g.command("status").unwrap();
        assert_eq!(status.description, "Report status");
        for value in ["dir", "director", "client=", "storage="] { assert!(arg_words(status).contains(&value.to_string())); }
        assert!(status.args.iter().any(|a| a.name == "client" && a.values == grammar::Values::Source(".clients".into())));
        assert!(status.args.iter().any(|a| a.name == "storage" && a.values == grammar::Values::Source(".storages".into())));
        let g = help_grammar(&["run Run a job -- run [job=<job-name> level=<level> yes]", "run"]);
        assert_eq!(g.commands.len(), 1, "a line without usage is no command");
        assert!(arg_words(&g.commands[0]).contains(&"job=".to_string()));
        assert!(arg_words(&g.commands[0]).contains(&"yes".to_string()));
        let legacy = help_grammar(&["status\tReport status\tstatus [dir]\t!dir,client"]);
        assert!(arg_words(&legacy.commands[0]).contains(&"dir".to_string()));
    }

    #[test]
    fn completion_accepts_every_director_help_entry() {
        mod source {
            include!("../../odir/src/ua_commands_table.rs");
        }
        // query() trims responses, including the space after an empty usage.
        let lines: Vec<String> = source::UA_COMMANDS.iter()
            .map(|(name, description, usage)| format!("{name} {description} -- {usage}").trim().to_string()).collect();
        let g = grammar::Grammar::from_help_lines(&lines);
        assert_eq!(g.commands.len(), source::UA_COMMANDS.len());
        for (command, keywords) in [
            ("list", vec!["jobs", "backups", "volumes", "jobstatistics"]),
            ("show", vec!["disabled", "clients", "all"]),
            ("status", vec!["scheduler", "subscriptions", "configuration"]),
            ("update", vec!["slots", "stats", "enabled=", "recyclepool="]),
            ("purge", vec!["volume=", "quota", "action="]),
        ] {
            let words = arg_words(g.command(command).unwrap());
            for keyword in keywords {
                assert!(words.iter().any(|c| c == keyword), "{command}: {keyword}");
            }
        }
    }

    #[test]
    fn completion_parses_adjacent_groups_and_spaced_assignments() {
        let g = help_grammar(&[
            "update Update volumes -- [maxvoljobs=<nb>][enabled=<yes/no>] | volume[=<volume>] [ drive = <drivenum>] trace=0/1"
        ]);
        let words = arg_words(&g.commands[0]);
        for key in ["maxvoljobs=", "enabled=", "volume=", "drive=", "trace="] {
            assert!(words.iter().any(|c| c == key), "missing {key}");
        }
        assert_eq!(words.len(), 5);
    }

    #[test]
    fn completion_keeps_partial_keys_and_handles_tabs() {
        let mut helper = BareosHelper::new();
        helper.grammar = help_grammar(&["run Run job -- job=<job-name> client=<client-name> yes"]);
        let history = rustyline::history::DefaultHistory::new();
        let ctx = Context::new(&history);
        for line in ["run client", "run\tclient"] {
            let (start, matches) = helper.complete(line, line.len(), &ctx).unwrap();
            assert_eq!(start, 4);
            assert_eq!(matches[0].replacement, "client=");
        }
        let line = "run client=example\t";
        let (_, matches) = helper.complete(line, line.len(), &ctx).unwrap();
        assert!(!matches.iter().any(|p| p.replacement == "client="));
        assert!(matches.iter().any(|p| p.replacement == "job="));
    }

    /// The loader takes `.grammar` (API 2) and falls back to `.help` when the
    /// Director answers "is an invalid command" (Bareos).
    struct FakeDirector {
        grammar: Option<String>,
        sent: Vec<String>,
    }

    impl DirectorConn for FakeDirector {
        fn query(&mut self, cmd: &str) -> Vec<String> {
            self.sent.push(cmd.to_string());
            match cmd {
                ".grammar" => match &self.grammar {
                    Some(json) => vec![json.clone()],
                    None => vec![r#"{"jsonrpc":"2.0","id":null,"error":{"code":1,"message":"failed","data":{"result":{},"messages":{"error":[".grammar: is an invalid command.\n"]}}}}"#.to_string()],
                },
                ".help" => vec!["run Run a job -- job=<job-name> yes".to_string()],
                ".pools" => vec!["Full".to_string()],
                _ => Vec::new(),
            }
        }
        fn send_cmd(&mut self, _: &str) -> bool { true }
        fn recv_msg(&mut self) -> Option<RecvResult> { None }
        fn drain(&mut self) {}
        fn send_cancel(&mut self) {}
    }

    #[test]
    fn loader_prefers_grammar_and_falls_back_to_help() {
        let json = r#"{"jsonrpc":"2.0","id":null,"result":{"grammar":{"schema":1,"commands":[
            {"word":"purge","aliases":[],"description":"Purge","args":[],"subs":[
              {"word":"volume","description":"v","args":[{"name":"pool","kind":"value","type":"resource","source":".pools"}],"subs":[]}]}]}}}"#;
        let mut odir = FakeDirector { grammar: Some(json.replace('\n', " ")), sent: Vec::new() };
        let mut helper = BareosHelper::new();
        helper.load_from_director(&mut odir);
        assert!(helper.grammar.typed);
        assert_eq!(odir.sent[..3], [".api 2", ".grammar", ".api 0"]);
        assert!(!odir.sent.contains(&".help".to_string()));
        assert_eq!(helper.value_cache[".pools"], ["Full"]);
        let mut bareos = FakeDirector { grammar: None, sent: Vec::new() };
        helper.refresh(&mut bareos);
        assert!(!helper.grammar.typed);
        assert!(helper.grammar.command("run").is_some());
        assert_eq!(bareos.sent[3..6], [".api 1", ".help", ".api 0"]);
        assert!(helper.summary().contains("(from .help)"));
    }

    /// The volstatus enum of `.grammar` contains the word "Error": in API 2
    /// and on a pipe the reply must stay byte-exact JSON.
    #[test]
    fn replies_are_coloured_only_in_text_mode_on_a_terminal() {
        let json = r#"{"jsonrpc":"2.0","result":{"values":["Append","Error","Full"]}}"#;
        let pretty = "  \"values\": [\n    \"Error\",\n    \"Full\"\n  ]";
        for mode in [ApiMode::Api, ApiMode::Json] {
            assert!(!reply_coloured(mode, true, true), "{mode:?}");
        }
        assert!(!reply_coloured(ApiMode::Text, false, true), "not on a pipe");
        assert!(!reply_coloured(ApiMode::Text, true, false), "NO_COLOR");
        assert!(reply_coloured(ApiMode::Text, true, true));
        for text in [json, pretty] {
            for line in text.lines() {
                assert_eq!(reply_line(line, reply_coloured(ApiMode::Json, true, true)), line);
                assert_eq!(reply_line(line, reply_coloured(ApiMode::Text, false, true)), line);
            }
        }
        assert_eq!(reply_line("Error: no such job", true), "\x1b[31mError: no such job\x1b[0m");
        assert_eq!(reply_line("plain", true), "plain");
    }

    /// `.api` over gRPC: the console keeps the mode and sends it as the
    /// response format of every following ExecuteCommand call (C++ DotApiCmd rules).
    #[test]
    fn dot_api_sets_the_grpc_response_format() {
        use onesimus_proto_rs::onesimus::director::ResponseFormat;
        assert_eq!(ApiMode::from_line(".api 2"), Some(ApiMode::Json));
        assert_eq!(ApiMode::from_line(".API json compact=yes"), Some(ApiMode::Json));
        assert_eq!(ApiMode::from_line(".api"), Some(ApiMode::Api));
        assert_eq!(ApiMode::from_line(".api on"), Some(ApiMode::Api));
        assert_eq!(ApiMode::from_line(".api off"), Some(ApiMode::Text));
        assert_eq!(ApiMode::from_line(".api 0"), Some(ApiMode::Text));
        assert_eq!(ApiMode::from_line(".api 3"), None);
        assert_eq!(ApiMode::from_line(".api 2 compact=yes x"), None);
        assert_eq!(ApiMode::from_line(".apis"), None);
        assert_eq!(ApiMode::from_line(".grammar"), None);
        assert_eq!(ApiMode::default().grpc_format(), ResponseFormat::Text);
        assert_eq!(ApiMode::Api.grpc_format(), ResponseFormat::Api);
        assert_eq!(ApiMode::Json.grpc_format(), ResponseFormat::Json);
        for mode in [ApiMode::Text, ApiMode::Api, ApiMode::Json] {
            assert_eq!(ApiMode::from_line(mode.command()), Some(mode));
        }
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

    /// C-087: stdio offers the write tools in every build; --mcp-read-only
    /// turns them off.
    #[test]
    fn stdio_writes_unless_read_only() {
        assert!(Cli::parse_from(["oconsole", "--mcp"]).mcp_write_enabled());
        assert!(!Cli::parse_from(["oconsole", "--mcp", "--mcp-read-only"]).mcp_write_enabled());
        assert!(Cli::try_parse_from(["oconsole", "--mcp-read-only"]).is_err());
    }

    #[cfg(feature = "mcp-development")]
    #[test]
    fn mcp_allow_write_requires_mcp() {
        assert!(Cli::try_parse_from(["oconsole", "--mcp-allow-write"]).is_err());
        let cli = Cli::parse_from(["oconsole", "--mcp", "--mcp-allow-write", "--grpc"]);
        assert!(cli.mcp && cli.mcp_write_enabled() && cli.grpc);
        // HTTP: write only with the flag
        assert!(Cli::parse_from(["oconsole", "--mcp", "--mcp-http", "127.0.0.1:9104", "--mcp-allow-write"]).mcp_write_enabled());
        assert!(!Cli::parse_from(["oconsole", "--mcp", "--mcp-http", "127.0.0.1:9104"]).mcp_write_enabled());
    }

    #[cfg(feature = "mcp-production")]
    #[test]
    fn production_http_is_read_only() {
        assert!(Cli::try_parse_from(["oconsole", "--mcp", "--mcp-allow-write"]).is_err());
        assert!(!Cli::parse_from(["oconsole", "--mcp", "--mcp-http", "127.0.0.1:9104"]).mcp_write_enabled());
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
