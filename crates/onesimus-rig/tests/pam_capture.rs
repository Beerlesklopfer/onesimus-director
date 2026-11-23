// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Capture of the console side of the PAM dialog: a fake Director (TLS-PSK,
//! Hello, CRAM-MD5) talks to the installed original `bconsole` and logs every
//! frame it receives after the authentication, to learn the dialog of
//! `dird/auth_pam.cc` / `console/auth_pam.cc` (docs/STUBS.md S-43).
//!
//! `PAM_SEND` (env) lists what the fake Director does after CRAM-MD5,
//! separated by `|`: `text` (send), `@signal:<n>` (send a signal) or `recv`
//! (read one frame and log it) or `hex:<bytes>` (send the raw bytes). Start:
//! `PAM_SEND='1001 PAM required' cargo test -p onesimus-rig --test pam_capture -- --ignored --nocapture`

#![deny(missing_docs)]

use std::io::Write;
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::Duration;

use bareos_bnet::cram_md5::CramMd5Auth;
use bareos_bnet::signal::BnetSignal;
use bareos_bnet::wire::{BnetMessage, BnetStream};

/// Hex and text of a received frame.
fn show(message: &BnetMessage) -> String {
    match message {
        BnetMessage::Data(data) => format!("DATA {:?} hex={}", String::from_utf8_lossy(data), data.iter().map(|b| format!("{b:02x}")).collect::<String>()),
        BnetMessage::Signal(signal) => format!("SIGNAL {}", *signal as i32),
    }
}

/// The capture.
#[test]
#[ignore = "needs the installed bconsole: run with --ignored --nocapture"]
fn bconsole_after_authentication() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dir = std::path::PathBuf::from("/tmp/claude-10001/pam");
    let conf = dir.join("bconsole.conf");
    std::fs::write(&conf, format!(
        "Director {{ Name = dir; DIRport = {port}; Address = 127.0.0.1; Password = \"dirpw\" }}\n\
         Console {{ Name = pamc; Password = \"pw\" }}\n")).unwrap();
    let credentials = dir.join("pam.cred");
    std::fs::write(&credentials, "testuser\ntestpass\n").unwrap();
    let mut command = Command::new("/usr/sbin/bconsole");
    command.args(["-d", "100", "-c"]).arg(&conf);
    if std::env::var_os("PAM_NOCRED").is_none() {
        command.arg("-p").arg(&credentials);
    }
    let mut child = command
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    if let Some(mut stdin) = child.stdin.take() {
        // answers for an interactive prompt
        let _ = stdin.write_all(b"testuser\ntestpass\n");
        std::mem::forget(stdin);
    }
    let (stream, _) = listener.accept().unwrap();
    let psk = bareos_bnet::tls_psk::password_to_psk("pw");
    let (tls, identity) = bareos_bnet::tls_psk::psk_accept_with(stream, move |_| Some(psk.clone())).unwrap();
    eprintln!("identity {identity:?}");
    let mut bnet = BnetStream::new(tls);
    let hello = bnet.recv_str().unwrap();
    eprintln!("hello {hello:?}");
    let ok = CramMd5Auth::new("pw", "pamc", 0).two_way_authenticate(&mut bnet, true).map(|r| r.success);
    eprintln!("cram {ok:?}");
    for item in std::env::var("PAM_SEND").unwrap_or_default().split('|').filter(|s| !s.is_empty()) {
        if item == "recv" {
            match bnet.recv() {
                Ok(m) => eprintln!("  recv {}", show(&m)),
                Err(e) => eprintln!("  recv ended: {e}"),
            }
            continue;
        }
        if let Some(hex) = item.strip_prefix("hex:") {
            let bytes: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
            bnet.send(&bytes).unwrap();
            eprintln!("sent hex {hex}");
            continue;
        }
        match item.strip_prefix("@signal:") {
            Some(n) => { let _ = bnet.send_signal(BnetSignal::from_i32(n.parse::<i32>().unwrap()).unwrap()); }
            None => { bnet.send_str(item).unwrap(); }
        }
        eprintln!("sent {item:?}");
    }
    std::thread::sleep(Duration::from_secs(1));
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    eprintln!("stdout: {}", String::from_utf8_lossy(&out.stdout));
    eprintln!("stderr: {}", String::from_utf8_lossy(&out.stderr));
    let _ = std::io::stderr().flush();
}
