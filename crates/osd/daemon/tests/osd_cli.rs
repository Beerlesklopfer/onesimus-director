// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `osd -t`, the osd.d overlay and the running daemon seen from a Director
//! (the interface crate `osd` connects as odir does).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("osd-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The shipped `config/bareos-sd.d` with the placeholders the package's
/// postinst fills in, copied to `<dir>/bareos-sd.d`.
fn shipped_config(dir: &Path) -> PathBuf {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../config/bareos-sd.d");
    let tree = dir.join("bareos-sd.d");
    for sub in std::fs::read_dir(&src).unwrap() {
        let sub = sub.unwrap().path();
        let to = tree.join(sub.file_name().unwrap());
        std::fs::create_dir_all(&to).unwrap();
        for f in std::fs::read_dir(&sub).unwrap() {
            let f = f.unwrap().path();
            let text = std::fs::read_to_string(&f).unwrap()
                .replace("@DIRECTOR_NAME@", "bareos-dir")
                .replace("@STORAGE_NAME@", "test-sd")
                .replace("CHANGEME", "secret");
            std::fs::write(to.join(f.file_name().unwrap()), text).unwrap();
        }
    }
    tree
}

/// Runs `osd` with `args`.
fn osd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_osd")).args(args).env_remove("RUST_LOG").output().unwrap()
}

#[test]
fn t_accepts_the_shipped_config() {
    let dir = scratch("good");
    let tree = shipped_config(&dir);
    for path in [&dir, &tree] {
        let out = osd(&["-t", "-c", path.to_str().unwrap()]);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(stdout.trim_end(),
            "osd: configuration OK: Storage \"test-sd\" (port 9103), 1 Director(s), 1 Device(s), 0 Autochanger(s)");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn t_rejects_a_broken_config_with_1() {
    let dir = scratch("bad");
    let tree = shipped_config(&dir);
    std::fs::write(tree.join("device/FileStorage.conf"), "Device {\n Name = FileStorage\n Media Type = File\n}\n").unwrap();
    let out = osd(&["-t", "-c", dir.to_str().unwrap(), "--overlay", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("Device \"FileStorage\": required directive \"Archive Device\" missing"), "{stderr}");
    // missing configuration
    let out = osd(&["-t", "-c", dir.join("nothing").to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn t_reports_an_orphaned_osd_d_extension() {
    let dir = scratch("orphan");
    shipped_config(&dir);
    std::fs::create_dir_all(dir.join("osd.d/device")).unwrap();
    std::fs::write(dir.join("osd.d/device/Nope.conf"), "Device {\n Name = Nope\n Ring Slots = 4\n}\n").unwrap();
    let out = osd(&["-t", "-c", dir.to_str().unwrap(), "--overlay", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Device \"Nope\" extends no Device of that name in bareos-sd.d"));
    std::fs::write(dir.join("osd.d/device/Nope.conf"), "Device {\n Name = FileStorage\n Ring Slots = 4\n}\n").unwrap();
    assert_eq!(osd(&["-t", "-c", dir.to_str().unwrap(), "--overlay", dir.to_str().unwrap()]).status.code(), Some(0));
    std::fs::remove_dir_all(&dir).ok();
}

/// `osd -t` accepts a Distributor of `osd.d/distributor/` and reports its errors (P2).
#[test]
fn t_checks_a_distributor() {
    let dir = scratch("distributor");
    shipped_config(&dir);
    std::fs::create_dir_all(dir.join("osd.d/distributor")).unwrap();
    let check = |text: &str| {
        std::fs::write(dir.join("osd.d/distributor/Verteiler.conf"), text).unwrap();
        osd(&["-t", "-c", dir.to_str().unwrap(), "--overlay", dir.to_str().unwrap()])
    };
    let out = check("Distributor {\n Name = Verteiler\n Media Type = Onesimus\n Route { Level = Full; Device = FileStorage }\n Route { Device = FileStorage }\n}\n");
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    for (text, want) in [
        ("Distributor {\n Name = Verteiler\n Media Type = O\n Route { Device = Nirgends }\n}\n", "Route target Device \"Nirgends\" not found"),
        ("Distributor {\n Name = Verteiler\n Media Type = O\n Route { Level = Full }\n}\n", "Route without Device"),
        ("Distributor {\n Name = FileStorage\n Media Type = O\n Route { Device = FileStorage }\n}\n", "already used by a Device"),
        ("Distributor {\n Name = Verteiler\n Media Type = O\n Route { Device = FileStorage }\n Route { Device = FileStorage }\n}\n", "more than one Route without conditions"),
        ("Distributor {\n Name = Verteiler\n Media Type = O\n Route { Stripe = yes; Device = FileStorage }\n}\n", "Stripe = yes is not supported yet"),
    ] {
        let out = check(text);
        assert_eq!(out.status.code(), Some(1), "{text}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(want), "want {want:?} in {stderr}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// A free TCP port on 127.0.0.1.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Sends `cmd` and collects the answer up to EOD.
fn ask<S: std::io::Read + std::io::Write>(sd: &mut bareos_bnet::BnetStream<S>, cmd: &str) -> String {
    sd.send_str(cmd).unwrap();
    let mut out = String::new();
    loop {
        match sd.recv() {
            Ok(bareos_bnet::BnetMessage::Data(d)) => out.push_str(&String::from_utf8_lossy(&d)),
            Ok(bareos_bnet::BnetMessage::Signal(bareos_bnet::BnetSignal::Eod)) => return out,
            Ok(bareos_bnet::BnetMessage::Signal(_)) | Err(_) => return out,
        }
    }
}

#[test]
fn daemon_answers_a_director_and_labels_a_volume() {
    let dir = scratch("run");
    let tree = shipped_config(&dir);
    let port = free_port();
    let storage = dir.join("storage");
    std::fs::create_dir_all(&storage).unwrap();
    let conf = std::fs::read_to_string(tree.join("storage/bareos-sd.conf")).unwrap()
        .replace("# SD Port = 9103", &format!("SD Port = {port}\n  SD Address = 127.0.0.1"))
        .replace("Working Directory = /var/lib/bareos", &format!("Working Directory = {}", dir.display()));
    std::fs::write(tree.join("storage/bareos-sd.conf"), conf).unwrap();
    let dev = std::fs::read_to_string(tree.join("device/FileStorage.conf")).unwrap()
        .replace("/var/lib/bareos/storage", storage.to_str().unwrap());
    std::fs::write(tree.join("device/FileStorage.conf"), dev).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_osd")).args(["-f", "-c", dir.to_str().unwrap(), "--overlay", dir.to_str().unwrap()]).spawn().unwrap();
    let mut sd = None;
    for _ in 0..100 {
        match osd::connect::connect_sd("127.0.0.1", port, "secret", "bareos-dir") {
            Ok(s) => {
                sd = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    let mut sd = sd.expect("osd answers on its SD port");
    let header = ask(&mut sd, ".status header");
    assert!(header.starts_with("test-sd Version: "), "{header}");
    let label = ask(&mut sd, "label FileStorage VolumeName=Vol-1 PoolName=Full MediaType=File Slot=0 drive=0 MinBlocksize=0 MaxBlocksize=0");
    assert!(label.starts_with("3000 OK label. VolFiles=0 VolBytes="), "{label}");
    assert!(label.contains("Volume=\"Vol-1\" Device=\"FileStorage\""), "{label}");
    let again = ask(&mut sd, "label FileStorage VolumeName=Vol-1 PoolName=Full MediaType=File Slot=0 drive=0 MinBlocksize=0 MaxBlocksize=0");
    assert_eq!(again, "3920 Cannot label Volume because it is already labeled: \"Vol-1\"\n");
    let volume = std::fs::read(storage.join("Vol-1")).unwrap();
    let summary = bareos_format::volume::summarize(&volume).unwrap();
    assert_eq!((summary.label.volume_name.as_str(), summary.label.label_prog.as_str()), ("Vol-1", "test-sd"));
    assert!(ask(&mut sd, "mount FileStorage drive=0").starts_with("3906 File device \"FileStorage\""));
    assert!(ask(&mut sd, "release FileStorage drive=0").starts_with("3921 Device"));
    let unknown = ask(&mut sd, "frobnicate");
    assert_eq!(unknown, "3900 Invalid command: frobnicate\n");
    // SIGTERM ends the daemon with 0
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    assert_eq!(child.wait().unwrap().code(), Some(0));
    std::fs::remove_dir_all(&dir).ok();
}

/// The Director's side of gRPC (`onesimus.stored.StorageDaemon`): Control
/// carries the same session as the SD port, Data takes only the FD of a
/// job, a wrong certificate is refused, `--disable-grpc` closes the port.
#[test]
fn daemon_answers_a_director_over_grpc() {
    let dir = scratch("grpc");
    let tree = shipped_config(&dir);
    let (port, grpc_port) = (free_port(), free_port());
    let conf = std::fs::read_to_string(tree.join("storage/bareos-sd.conf")).unwrap()
        .replace("# SD Port = 9103", &format!("SD Port = {port}\n  SD Address = 127.0.0.1"))
        .replace("Working Directory = /var/lib/bareos", &format!("Working Directory = {}", dir.display()));
    std::fs::write(tree.join("storage/bareos-sd.conf"), conf).unwrap();
    // D-8: the gRPC port is an osd.d extension of the same-named Storage.
    std::fs::create_dir_all(dir.join("osd.d/storage")).unwrap();
    std::fs::write(dir.join("osd.d/storage/test-sd.conf"), format!("Storage {{\n  Name = test-sd\n  Grpc Port = {grpc_port}\n}}\n")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_osd")).args(["-f", "-c", dir.to_str().unwrap(), "--overlay", dir.to_str().unwrap()]).spawn().unwrap();
    let cert = dir.join("onesimus-sd-grpc.crt");
    let target = |ca: Vec<u8>| onesimus_bnet_grpc::Target { host: "127.0.0.1".into(), port: grpc_port, ca, server_name: "localhost".into() };
    let mut io = None;
    for _ in 0..100 {
        if let Ok(ca) = std::fs::read(&cert)
            && let Ok(s) = onesimus_bnet_grpc::client::connect(&target(ca), onesimus_bnet_grpc::Rpc::StorageDaemonControl, Some(std::time::Duration::from_secs(10)))
        {
            io = Some(s);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut sd = bareos_bnet::BnetStream::new(io.expect("osd answers on its gRPC port"));
    osd::connect::handshake(&mut sd, "secret", "bareos-dir").unwrap();
    let header = ask(&mut sd, ".status header");
    assert!(header.starts_with("test-sd Version: "), "{header}");
    drop(sd);
    // Data takes only "Hello Start Job"; a Director is turned away.
    let ca = std::fs::read(&cert).unwrap();
    let io = onesimus_bnet_grpc::client::connect(&target(ca.clone()), onesimus_bnet_grpc::Rpc::StorageDaemonData, Some(std::time::Duration::from_secs(10))).unwrap();
    let mut data = bareos_bnet::BnetStream::new(io);
    assert!(osd::connect::handshake(&mut data, "secret", "bareos-dir").is_err());
    // Another certificate is not trusted (pinning).
    let other = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap().cert.pem().into_bytes();
    assert!(onesimus_bnet_grpc::client::connect(&target(other), onesimus_bnet_grpc::Rpc::StorageDaemonControl, None).is_err());
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    assert_eq!(child.wait().unwrap().code(), Some(0));

    let mut child = Command::new(env!("CARGO_BIN_EXE_osd")).args(["-f", "--disable-grpc", "-c", dir.to_str().unwrap(), "--overlay", dir.to_str().unwrap()]).spawn().unwrap();
    let mut bnet = None;
    for _ in 0..100 {
        if let Ok(s) = osd::connect::connect_sd("127.0.0.1", port, "secret", "bareos-dir") {
            bnet = Some(s);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(bnet.is_some(), "BNET stays with --disable-grpc");
    assert!(onesimus_bnet_grpc::client::connect(&target(ca), onesimus_bnet_grpc::Rpc::StorageDaemonControl, None).is_err());
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    assert_eq!(child.wait().unwrap().code(), Some(0));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unknown_argument_is_41() {
    assert_eq!(osd(&["--no-such-flag"]).status.code(), Some(41));
}
