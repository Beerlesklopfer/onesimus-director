//! Integration test: BNET console connection to Director.
//!
//! Starts a minimal Director accept loop on a random port,
//! connects as a bconsole client, sends commands, verifies responses.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use bareos_bnet::{BnetStream, BnetSignal, BnetMessage};
use bareos_bnet::hello::make_console_hello;

/// Find a free port.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Start a minimal accept loop that handles one console session.
fn start_test_director(port: u16, shutdown: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("test:director".into())
        .spawn(move || {
            let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
            listener.set_nonblocking(true).ok();

            while !shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).ok();
                        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

                        let mut bnet = BnetStream::new(stream);

                        // Read hello
                        let hello = bnet.recv_str().unwrap();
                        assert!(hello.contains("Hello"), "expected Hello, got: {hello}");

                        // Send banner
                        bnet.send_str("1000 OK: baReos-dir Version: 0.1.0\n").unwrap();

                        // Command loop
                        loop {
                            bnet.send_signal(BnetSignal::MainPrompt).unwrap();

                            match bnet.recv() {
                                Ok(BnetMessage::Data(data)) => {
                                    let cmd = String::from_utf8_lossy(&data);
                                    let cmd = cmd.trim();

                                    if cmd == "quit" || cmd == "exit" {
                                        // C++ ua_server.cc: no reply, socket closed
                                        break;
                                    }

                                    if cmd == "version" {
                                        bnet.send_str("baReos Director Version: 0.1.0\n").unwrap();
                                    } else if cmd.starts_with("status") {
                                        bnet.send_str("Status: running\n").unwrap();
                                    } else {
                                        bnet.send_str(&format!("echo: {cmd}\n")).unwrap();
                                    }
                                    bnet.send_signal(BnetSignal::Eod).unwrap();
                                }
                                Ok(BnetMessage::Signal(BnetSignal::Terminate)) => break,
                                _ => break,
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        })
        .unwrap()
}

#[test]
fn connect_send_version_quit() {
    let port = free_port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let dir_handle = start_test_director(port, shutdown.clone());

    // Wait for server to start
    thread::sleep(Duration::from_millis(100));

    // Connect as console
    let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut bnet = BnetStream::new(stream);

    // Send hello
    let hello = make_console_hello("*UserAgent*", "", false);
    bnet.send_str(&hello).unwrap();

    // Receive banner
    let banner = bnet.recv_str().unwrap();
    assert!(banner.contains("1000 OK"), "expected OK banner, got: {banner}");

    // Receive prompt
    let prompt = bnet.recv().unwrap();
    assert_eq!(prompt, BnetMessage::Signal(BnetSignal::MainPrompt));

    // Send version command
    bnet.send_str("version").unwrap();
    let response = bnet.recv_str().unwrap();
    assert!(response.contains("baReos"), "expected version, got: {response}");

    // Receive EOD
    let eod = bnet.recv().unwrap();
    assert_eq!(eod, BnetMessage::Signal(BnetSignal::Eod));

    // Receive next prompt
    let prompt = bnet.recv().unwrap();
    assert_eq!(prompt, BnetMessage::Signal(BnetSignal::MainPrompt));

    // Send quit
    bnet.send_str("quit").unwrap();
    assert!(bnet.recv().is_err(), "expected connection close without reply");

    // Cleanup
    shutdown.store(true, Ordering::SeqCst);
    dir_handle.join().unwrap();
}

#[test]
fn connect_send_status() {
    let port = free_port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let dir_handle = start_test_director(port, shutdown.clone());

    thread::sleep(Duration::from_millis(100));

    let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut bnet = BnetStream::new(stream);

    // Hello + banner
    bnet.send_str(&make_console_hello("admin", "25.0.0", false)).unwrap();
    let banner = bnet.recv_str().unwrap();
    assert!(banner.contains("1000 OK"));

    // Prompt
    bnet.recv().unwrap(); // MainPrompt

    // Status
    bnet.send_str("status dir").unwrap();
    let response = bnet.recv_str().unwrap();
    assert!(response.contains("Status"), "expected status, got: {response}");

    // EOD + Prompt
    bnet.recv().unwrap(); // EOD
    bnet.recv().unwrap(); // MainPrompt

    // Quit
    bnet.send_str("quit").unwrap();

    shutdown.store(true, Ordering::SeqCst);
    dir_handle.join().unwrap();
}

#[test]
fn multiple_commands_in_session() {
    let port = free_port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let dir_handle = start_test_director(port, shutdown.clone());

    thread::sleep(Duration::from_millis(100));

    let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut bnet = BnetStream::new(stream);

    // Hello + banner
    bnet.send_str(&make_console_hello("*UserAgent*", "", false)).unwrap();
    bnet.recv_str().unwrap(); // banner

    // Send 5 commands
    for i in 0..5 {
        bnet.recv().unwrap(); // MainPrompt
        bnet.send_str(&format!("echo cmd-{i}")).unwrap();
        let resp = bnet.recv_str().unwrap();
        assert!(resp.contains(&format!("cmd-{i}")), "response should echo cmd-{i}, got: {resp}");
        bnet.recv().unwrap(); // EOD
    }

    // Quit
    bnet.recv().unwrap(); // MainPrompt
    bnet.send_str("quit").unwrap();

    shutdown.store(true, Ordering::SeqCst);
    dir_handle.join().unwrap();
}

#[test]
fn bnet_signal_framing() {
    let port = free_port();
    let shutdown = Arc::new(AtomicBool::new(false));
    let dir_handle = start_test_director(port, shutdown.clone());

    thread::sleep(Duration::from_millis(100));

    let stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut bnet = BnetStream::new(stream);

    // Hello
    bnet.send_str(&make_console_hello("*UserAgent*", "", false)).unwrap();
    bnet.recv_str().unwrap(); // banner

    // Verify signal framing: MainPrompt is -18
    let msg = bnet.recv().unwrap();
    assert_eq!(msg, BnetMessage::Signal(BnetSignal::MainPrompt));

    // Send command, verify EOD signal
    bnet.send_str("version").unwrap();
    bnet.recv_str().unwrap(); // response
    let eod = bnet.recv().unwrap();
    assert_eq!(eod, BnetMessage::Signal(BnetSignal::Eod));

    // Quit
    bnet.recv().unwrap(); // prompt
    bnet.send_str("quit").unwrap();

    shutdown.store(true, Ordering::SeqCst);
    dir_handle.join().unwrap();
}
