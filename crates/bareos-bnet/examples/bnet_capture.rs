//! Raw console capture for Director parity tests (tests/parity/parity.py).
//!
//! Opens one authenticated console session, sends each step and records every
//! BNET frame of the reply without decoding, trimming or reassembly:
//!
//!   bnet_capture HOST:PORT NAME PASSWORD STEP...
//!
//! NAME `*UserAgent*` uses the default console. Output is one JSON object on
//! stdout: the banner frames plus, per step, the frame list and why reading
//! stopped (`eod`, `signal:<n>` for an input request, `timeout`, `closed`).

use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bareos_bnet::cram_md5::CramMd5Auth;
use bareos_bnet::hello::make_console_hello;
use bareos_bnet::signal::BnetSignal;
use bareos_bnet::wire::{BnetError, BnetMessage, BnetStream};

/// Signals after which the Director waits for console input.
const INPUT_SIGNALS: [BnetSignal; 6] = [BnetSignal::Prompt, BnetSignal::SubPrompt, BnetSignal::SelectInput,
    BnetSignal::YesNo, BnetSignal::TextInput, BnetSignal::MainPrompt];

fn frame(message: &BnetMessage) -> serde_json::Value {
    match message {
        BnetMessage::Data(data) => serde_json::json!({ "data": BASE64.encode(data) }),
        BnetMessage::Signal(signal) => serde_json::json!({ "signal": *signal as i32 }),
    }
}

/// Reads frames until the reply ends; `quiet` ends a reply that has no signal (banner).
fn read_reply<S: std::io::Read + std::io::Write>(bnet: &mut BnetStream<S>, quiet: bool)
    -> (Vec<serde_json::Value>, String) {
    let mut frames = Vec::new();
    loop {
        match bnet.recv() {
            Ok(message) => {
                frames.push(frame(&message));
                match message {
                    BnetMessage::Signal(BnetSignal::Eod) => return (frames, "eod".into()),
                    BnetMessage::Signal(signal) if INPUT_SIGNALS.contains(&signal) =>
                        return (frames, format!("signal:{}", signal as i32)),
                    _ => {}
                }
            }
            Err(BnetError::Io(error)) if matches!(error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) =>
                return (frames, if quiet { "quiet" } else { "timeout" }.into()),
            Err(error) => return (frames, format!("closed: {error}")),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        eprintln!("usage: bnet_capture HOST:PORT NAME PASSWORD STEP...");
        std::process::exit(2);
    }
    let (address, name, password) = (&args[0], &args[1], &args[2]);
    let timeout = Duration::from_millis(std::env::var("CAPTURE_TIMEOUT_MS").ok()
        .and_then(|value| value.parse().ok()).unwrap_or(30_000));
    let banner_quiet = Duration::from_millis(std::env::var("CAPTURE_BANNER_MS").ok()
        .and_then(|value| value.parse().ok()).unwrap_or(150));

    let stream = TcpStream::connect(address).unwrap_or_else(|error| {
        eprintln!("connect {address}: {error}");
        std::process::exit(1);
    });
    let socket = stream.try_clone().expect("clone socket");
    let identity = if name == "*UserAgent*" { "R_CONSOLE\x1e*UserAgent*".to_string() }
        else { format!("R_CONSOLE\x1e{name}") };
    let psk = bareos_bnet::tls_psk::password_to_psk(password);
    let tls = bareos_bnet::tls_psk::psk_connect(stream, &identity, &psk).unwrap_or_else(|error| {
        eprintln!("TLS-PSK: {error}");
        std::process::exit(1);
    });
    let mut bnet = BnetStream::new(tls);
    let hello = make_console_hello(name, env!("CARGO_PKG_VERSION"), false);
    let auth_name = if name == "*UserAgent*" { "*UserAgent*" } else { name.as_str() };
    let authenticated = bnet.send_str(&hello).is_ok()
        && CramMd5Auth::new(password, auth_name, 0).two_way_authenticate(&mut bnet, false)
            .is_ok_and(|result| result.success);
    if !authenticated {
        eprintln!("authentication failed");
        std::process::exit(1);
    }

    // The banner has no end signal: wait for its first frame, then for silence.
    let mut banner = Vec::new();
    let _ = socket.set_read_timeout(Some(timeout));
    if let Ok(message) = bnet.recv() {
        banner.push(frame(&message));
        let _ = socket.set_read_timeout(Some(banner_quiet));
        banner.extend(read_reply(&mut bnet, true).0);
    }
    let _ = socket.set_read_timeout(Some(timeout));
    let mut steps = Vec::new();
    for step in &args[3..] {
        let started = Instant::now();
        if let Err(error) = bnet.send_str(step) {
            steps.push(serde_json::json!({ "send": step, "frames": [], "end": format!("send failed: {error}") }));
            break;
        }
        let (frames, end) = read_reply(&mut bnet, false);
        let closed = end.starts_with("closed");
        steps.push(serde_json::json!({ "send": step, "frames": frames, "end": end,
            "elapsed_ms": started.elapsed().as_millis() as u64 }));
        if closed { break; }
    }
    let _ = bnet.send_signal(BnetSignal::Terminate);
    let output = serde_json::json!({ "banner": banner, "steps": steps });
    let mut stdout = std::io::stdout().lock();
    let _ = writeln!(stdout, "{output}");
}
