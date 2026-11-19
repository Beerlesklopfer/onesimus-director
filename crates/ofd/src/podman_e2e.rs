//! End-to-end tests of the Podman plugin, phase P0
//! (docs/plans/PODMAN-PLUGIN.md, "Testaufbau", exit criteria P0).
//!
//! # Purpose
//!
//! Real Podman 5.4.2, strictly isolated: every run has its own `--root`,
//! `--runroot`, `--tmpdir`, `HOME` and `XDG_RUNTIME_DIR` (0700) below
//! `/tmp/podimpl`, `lock_type = "file"`, never `podman system reset`.
//! Cleanup only with `rm -af`, `rmi -a`, `volume rm -a`, removing the own
//! directories (files of sub-UIDs via `podman unshare rm -rf`) and ending
//! the own pause process (checked by its environment). The user's Podman
//! store, pause processes and `podman.socket` are never touched.
//!
//! Run (needs podman 5.4.2, util-linux `unshare --map-auto`, sub-IDs of the
//! test user, network once for busybox):
//!
//! ```sh
//! cargo build -p onesimus-podman-broker
//! cargo test -p ofd -- --ignored podman_e2e --nocapture --test-threads=1
//! ```
//!
//! - [`volume_restore_to_files_is_clean`]: exit criterion P0 (1) with
//!   `ofd`: fixture volume with an ns-UID-0 setuid file and a capability
//!   file, scanned by the host, backed up through a podman test plugin
//!   (contract 2), restored to files without plugin code:
//!   `find -perm /7000` and `getcap -r` empty, the stream carries host IDs.
//!   The same stream restored by a Bareos FD is not part of this run (no
//!   bareos-fd on the development host).
//! - [`broker_in_root_owned_namespace`]: exit criteria P0 (3) and (4) in a
//!   nested user namespace: the test binary runs as root of an outer
//!   namespace (`unshare --map-root-user --map-auto`), `ofd`'s
//!   `launch_in_userns` creates the broker's namespace as that root for the
//!   account "uid 1"; Podman (info, ps, inspect, volume ls, create,
//!   run --rm) works under NNP with the three variables and without a
//!   pause process, and the account cannot `ptrace` the broker. The run
//!   with real host root (and with an existing pause process of the
//!   account) needs a test VM: `sudo -E cargo test -p ofd -- --ignored
//!   podman_e2e::broker_in_root_owned_namespace` with a real account.
//!
//! # Data flow
//!
//! ```mermaid
//! sequenceDiagram
//!     participant T as test
//!     participant P as podman (isolated)
//!     participant B as onesimus-podman-broker
//!     participant O as ofd (PodmanPort, ScanTable, backup, restore)
//!     T->>P: volume create, unshare: setuid, setcap, ACL, FIFO, hard link
//!     T->>B: start (podman unshare, socket on fd 0/1)
//!     O->>B: engine-identify (graphroot checked)
//!     O->>O: volume-scan (host reads disk)
//!     T->>O: guest lists references, backup to a stream
//!     O->>O: restore to files without plugin code
//!     T->>T: find -perm /7000, getcap -r: empty
//! ```
//!
//! # Counterpart
//!
//! The unit tests in `podman_port/tests.rs`, `podman_scan.rs` and
//! `onesimus-podman-core`.

use std::io::Cursor;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::AtomicBool;

use onesimus_plugins::wasm::{BrokerFunction, BrokerPort, PodmanFunction, RuntimeLimits};
use onesimus_podman_core::idmap::{self, IdMap};
use onesimus_podman_core::policy::{AccountEntry, DomainEntry};
use onesimus_podman_core::sanitize::Domain;
use onesimus_proto_rs::onesimus::filed::broker::podman::v1 as pb;
use onesimus_proto_rs::onesimus::filed::plugin::v1 as fpb;
use prost::Message;

use crate::plugin::guest::{as_read_back, Duplex};
use crate::plugin::testkit::*;
use crate::plugin::{parse_line, Session};
use crate::podman_port::{BrokerClient, PodmanPort};

/// Base of every isolated run.
const BASE: &str = "/tmp/podimpl";

/// An isolated Podman environment below [`BASE`].
struct Iso {
    /// The run's directory.
    t: PathBuf,
}

impl Iso {
    /// Creates `<BASE>/<tag>-<pid>` with home, config, runtime dir and storage.
    fn new(tag: &str) -> Self {
        std::fs::create_dir_all(BASE).unwrap();
        std::fs::set_permissions(BASE, std::fs::Permissions::from_mode(0o711)).unwrap();
        let t = Path::new(BASE).join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        for d in ["home/.config/containers", "xdg-run", "root", "run", "tmp", "restore"] {
            std::fs::create_dir_all(t.join(d)).unwrap();
        }
        std::fs::set_permissions(t.join("xdg-run"), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(t.join("home/.config/containers/containers.conf"), "[engine]\nlock_type = \"file\"\n").unwrap();
        Self { t }
    }

    /// The storage options of this run.
    fn storage_args(&self) -> Vec<String> {
        ["--root", "root", "--runroot", "run", "--tmpdir", "tmp"].chunks(2)
            .flat_map(|c| [c[0].to_string(), self.t.join(c[1]).display().to_string()])
            .chain(["--storage-driver".to_string(), "vfs".to_string()])
            .collect()
    }

    /// A podman command with only this run's environment.
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new("podman");
        c.env_clear()
            .env("HOME", self.t.join("home"))
            .env("XDG_RUNTIME_DIR", self.t.join("xdg-run"))
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C")
            .args(self.storage_args())
            .args(args);
        c
    }

    /// Runs podman; panics on failure.
    fn podman(&self, args: &[&str]) -> Output {
        let out = self.cmd(args).output().unwrap();
        assert!(out.status.success(), "podman {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out
    }

    /// Cleanup: only rm -af, volume rm -a, rmi -a, the own directories and
    /// the own pause process.
    fn cleanup(&self) {
        for args in [&["rm", "-af"][..], &["volume", "rm", "-a"], &["rmi", "-a"]] {
            let _ = self.cmd(args).output();
        }
        let _ = self.cmd(&["unshare", "rm", "-rf", &self.t.join("root").display().to_string(), &self.t.join("run").display().to_string(),
            &self.t.join("restore").display().to_string()]).output();
        kill_own_pause(&self.t.join("xdg-run"));
        let _ = std::fs::remove_dir_all(&self.t);
    }
}

/// Ends the pause process of an isolated runtime dir, after checking that
/// its environment names exactly that dir (never the user's own).
fn kill_own_pause(runtime: &Path) {
    let Ok(pid) = std::fs::read_to_string(runtime.join("libpod/tmp/pause.pid")) else { return };
    let Ok(pid) = pid.trim().parse::<i32>() else { return };
    let marker = format!("XDG_RUNTIME_DIR={}", runtime.display());
    let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
    if environ.split(|&b| b == 0).any(|e| e == marker.as_bytes()) {
        // SAFETY: signalling a process we verified to be ours.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
}

/// Whether podman is usable at all.
fn podman_available() -> bool {
    Command::new("podman").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// The broker binary built by `cargo build -p onesimus-podman-broker`.
fn broker_binary() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/onesimus-podman-broker");
    assert!(p.is_file(), "cargo build -p onesimus-podman-broker first");
    p.canonicalize().unwrap()
}

/// The test user's uid/gid.
fn me() -> (u32, u32) {
    // SAFETY: getuid/getgid have no preconditions.
    unsafe { (libc::getuid(), libc::getgid()) }
}

/// Exit criterion P0 (1) with ofd (see the module documentation).
#[test]
#[ignore = "needs podman, sub-IDs and the built broker; isolated below /tmp/podimpl"]
fn volume_restore_to_files_is_clean() {
    if !podman_available() {
        eprintln!("podman not installed; skipped");
        return;
    }
    let iso = Iso::new("p0-restore");
    let result = std::panic::catch_unwind(|| restore_case(&iso));
    iso.cleanup();
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

/// The body of [`volume_restore_to_files_is_clean`].
fn restore_case(iso: &Iso) {
    let (uid, gid) = me();
    iso.podman(&["volume", "create", "fix"]);
    let data = iso.t.join("root/volumes/fix/_data");
    // Inside the account's namespace: ns-UID 0 setuid, a capability, a
    // setgid directory of ns-UID 1000 with an ACL, FIFO, hard link, link.
    let script = format!(
        "set -e; cd {d}; mkdir -p bin data; echo su > bin/su; chmod 4755 bin/su; echo ping > bin/ping; \
         setcap cap_net_raw+ep bin/ping; echo f > data/f; chown -R 1000:1000 data; chmod 2755 data; chmod 0644 data/f; \
         setfacl -m u:1000:r data/f; mkfifo fifo; ln bin/su hard; ln -s bin/su link; \
         python3 -c \"import os; os.setxattr('data/f', 'user.note', b'kept')\"",
        d = data.display()
    );
    iso.podman(&["unshare", "sh", "-c", &script]);
    let caps = Command::new("getcap").arg(data.join("bin/ping")).output().unwrap();
    assert!(String::from_utf8_lossy(&caps.stdout).contains("cap_net_raw"), "fixture has a capability");

    // The real broker in the account's namespace (podman unshare).
    let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
    let theirs_out = theirs.try_clone().unwrap();
    let mut broker_args: Vec<String> = vec![broker_binary().display().to_string()];
    broker_args.extend(["--root", "--runroot", "--tmpdir"].iter().zip(["root", "run", "tmp"])
        .flat_map(|(f, d)| [f.to_string(), iso.t.join(d).display().to_string()]));
    broker_args.push("--rootless".into());
    let mut unshare = vec!["unshare".to_string()];
    unshare.extend(broker_args);
    let refs: Vec<&str> = unshare.iter().map(String::as_str).collect();
    let mut child = iso.cmd(&refs).stdin(Stdio::from(std::os::fd::OwnedFd::from(theirs)))
        .stdout(Stdio::from(std::os::fd::OwnedFd::from(theirs_out))).stderr(Stdio::null()).spawn().unwrap();

    let name = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).unwrap().unwrap().name;
    let sub = |file: &str| idmap::sub_ranges(&std::fs::read_to_string(file).unwrap(), &name, uid).unwrap();
    let domain = Domain {
        uid_map: IdMap::rootless(uid, &sub("/etc/subuid")[..1]).unwrap(),
        gid_map: IdMap::rootless(gid, &sub("/etc/subgid")[..1]).unwrap(),
        rootless: true,
    };
    let account = AccountEntry {
        name: name.clone(),
        uid,
        domains: vec![DomainEntry { graphroot: iso.t.join("root"), runroot: Some(iso.t.join("run")), tmpdir: Some(iso.t.join("tmp")) }],
        rootful: false,
        volume_bind_roots: vec![],
        registries: vec![],
    };
    let line = onesimus_podman_core::line::PodmanLine::parse(&parse_line("podman:user=x:pods=*:consistency=crash").unwrap().options).unwrap();
    let caps_granted: Vec<String> = ["file.read", "podman.inventory", "podman.volume.read"].iter().map(|s| s.to_string()).collect();
    let graphroot = crate::podman_scan::open_graphroot(&iso.t.join("root")).unwrap();
    let mut port = PodmanPort::new(Box::new(BrokerClient { io: ours }), line, account, domain, graphroot, &caps_granted);
    // engine-identify through the real broker: rootless, graphroot checked.
    port.confirm().expect("engine-identify through the broker");
    let r = pb::VolumeScanResponse::decode(port.call(BrokerFunction::Podman(PodmanFunction::VolumeScan),
        &pb::VolumeScanRequest { volume: "fix".into(), cursor: 0 }.encode_to_vec()).unwrap().as_slice()).unwrap();
    assert!(r.done && r.rejected.is_empty(), "{:?}", r.rejected);
    let scans = port.scans();
    let entries = scans.lock().unwrap().get("fix").unwrap().entries.clone();
    let _ = child.kill();
    let _ = child.wait();

    // The podman test plugin lists every reference (hard link after its target).
    let mut next: Vec<Step> = entries.iter().map(|e| ref_step(&e.path, e.kind.file_kind(), &e.source_ref, 0, 0o7777)).collect();
    next.push(complete_step());
    let plugins = iso.t.join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let resolved = signed_plugin_as(&plugins, "podman", onesimus_plugins::PODMAN_PLUGIN_ID, &["file.read", "podman.inventory", "podman.volume.read"],
        &Script { next, interface_version: 2, ..Default::default() });
    let job = fpb::JobContext { job_id: 8101, job_name: "podman".into(), operation: fpb::JobOperation::Backup as i32 };
    let runtime = RuntimeLimits { max_chunk_bytes: 64 * 1024, ..RuntimeLimits::default() };
    let mut session = Session::open_resolved(&resolved, &parse_line("podman:user=x:pods=*").unwrap(), job, 0, runtime).unwrap();
    session.scans = Some(scans);
    let stream = backup_stream(session);

    // The stream: never special bits, never security.capability, host IDs.
    let attrs = crate::podman_port::tests::attribute_modes(&stream);
    assert!(attrs.iter().all(|(mode, _)| mode & 0o7000 == 0), "{attrs:?}");
    let sub_uid = domain_host(1000);
    assert!(attrs.iter().any(|(_, u)| *u == uid) && attrs.iter().any(|(_, u)| *u == sub_uid), "host IDs in the stream: {attrs:?}");
    assert!(!stream.windows(19).any(|w| w == b"security.capability"));
    assert!(stream.windows(9).any(|w| w == b"user.note"), "user.* xattrs are kept");

    // Restore to files without plugin code.
    let target = iso.t.join("restore");
    let config = crate::config::FdConfig::parse("Director { Name = d; Password = p }").unwrap();
    let canceled = AtomicBool::new(false);
    let mut sd = bareos_bnet::BnetStream::new(Duplex(Cursor::new(as_read_back(&stream))));
    let params = crate::restore::RestoreParams { where_: crate::attribs::Where::Prefix(target.as_os_str().as_encoded_bytes().to_vec()),
        replace: b'a', prefix_links: false };
    let mut msgs = crate::jmsg::JobMsgs::new("test-fd");
    msgs.job_id = 8102;
    let mut counters = crate::restore::Counters::default();
    let mut sent = Vec::new();
    crate::restore::Restore { params: &params, msgs: &mut msgs, counters: &mut counters,
        create: &mut crate::create_file::CreateState::default(), canceled: &canceled, config: Some(&config), plugin_options: &[] }
        .run(&mut sd, &mut |m: &mut crate::jmsg::JobMsgs| sent.extend(m.take_queued()));
    assert!(counters.job_files as usize >= entries.len(), "{sent:?}");
    let base = target.join("@podman").join(format!("uid-{uid}")).join("volumes/fix");
    assert_eq!(std::fs::read(base.join("data/bin/su")).unwrap(), b"su\n");
    let find = Command::new("find").arg(&target).args(["-perm", "/7000"]).output().unwrap();
    assert!(find.stdout.is_empty(), "special bits after restore: {}", String::from_utf8_lossy(&find.stdout));
    let getcap = Command::new("getcap").arg("-r").arg(&target).output().unwrap();
    assert!(getcap.stdout.is_empty(), "capabilities after restore: {}", String::from_utf8_lossy(&getcap.stdout));
    let tsv = std::fs::read_to_string(base.join("ownership.tsv")).unwrap();
    assert!(tsv.contains("bin/su\t0\t0\t4755"), "{tsv}");
    assert!(tsv.contains("permitted=0x2000"), "cap_net_raw in ownership.tsv: {tsv}");
    assert!(std::fs::symlink_metadata(base.join("data/fifo")).unwrap().file_type().is_fifo_like());
    eprintln!("P0 (1) with ofd: {} entries, find -perm /7000 and getcap -r empty", entries.len());
}

/// The host uid of ns uid `n` of the test user (first sub range).
fn domain_host(n: u32) -> u32 {
    let (uid, _) = me();
    let name = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).unwrap().unwrap().name;
    let r = idmap::sub_ranges(&std::fs::read_to_string("/etc/subuid").unwrap(), &name, uid).unwrap();
    IdMap::rootless(uid, &r[..1]).unwrap().to_host(n).unwrap()
}

/// FIFO check on a file type.
trait FifoLike {
    /// Whether it is a FIFO.
    fn is_fifo_like(&self) -> bool;
}
impl FifoLike for std::fs::FileType {
    fn is_fifo_like(&self) -> bool {
        std::os::unix::fs::FileTypeExt::is_fifo(self)
    }
}

/// A session's files through the real backup send path.
fn backup_stream(session: Session) -> Vec<u8> {
    let config = crate::config::FdConfig::parse("Director { Name = d; Password = p }").unwrap();
    let canceled = AtomicBool::new(false);
    let mut sd = bareos_bnet::BnetStream::new(Cursor::new(Vec::new()));
    let mut msgs = crate::jmsg::JobMsgs::new("test-fd");
    let mut counters = crate::restore::Counters::default();
    let mut flush = |_: &mut crate::jmsg::JobMsgs| {};
    let ok = crate::backup::Backup {
        sd: crate::backup::SdSender { sd: &mut sd, msgs: &mut msgs, counters: &mut counters, canceled: &canceled, progress: None },
        env: crate::backup::JobEnv { canceled: &canceled, accurate: None, level_full: true, rerunning: false, base_size: 0 },
        flush: &mut flush,
        buf_size: 64 * 1024,
        workers: 0,
        acl_errors: 0,
        xattr_errors: 0,
        xattr_disabled: false,
        config: &config,
        plugin_index: 0,
    }
    .run_plugin(&crate::find::FfPkt::default(), "podman:user=x", session);
    assert_eq!(ok, 1, "{:?}", msgs.take_queued());
    sd.into_inner().into_inner()
}

/// Exit criteria P0 (3) and (4) in a nested user namespace (see the
/// module documentation): re-runs this test binary as root of an outer
/// namespace, where [`nested_inside`] does the work.
#[test]
#[ignore = "needs podman, unshare --map-auto, sub-IDs and the built broker; isolated below /tmp/podimpl"]
fn broker_in_root_owned_namespace() {
    if !podman_available() {
        eprintln!("podman not installed; skipped");
        return;
    }
    // busybox once into an isolated store, saved as an archive for the inside.
    let iso = Iso::new("p0-image");
    let archive = Path::new(BASE).join(format!("busybox-{}.tar", std::process::id()));
    let prepared = (|| {
        iso.podman(&["pull", "-q", "docker.io/library/busybox:1.36"]);
        iso.podman(&["save", "-o", archive.to_str().unwrap(), "docker.io/library/busybox:1.36"]);
    })();
    iso.cleanup();
    let _ = prepared;
    std::fs::set_permissions(&archive, std::fs::Permissions::from_mode(0o644)).unwrap();
    let me_exe = std::env::current_exe().unwrap();
    let out = Command::new("unshare")
        .args(["--user", "--map-root-user", "--map-auto", "--mount"])
        .arg(&me_exe)
        .args(["--exact", "podman_e2e::nested_inside", "--ignored", "--nocapture", "--test-threads=1"])
        .env("ONESIMUS_PODMAN_NESTED", &archive)
        .env("ONESIMUS_PODMAN_BROKER", broker_binary())
        .output()
        .unwrap();
    let _ = std::fs::remove_file(&archive);
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    eprintln!("{text}");
    assert!(out.status.success() && text.contains("1 passed"), "inside test failed");
    assert!(text.contains("NESTED-OK"), "inside test did not run");
}

/// The inside part of [`broker_in_root_owned_namespace`]; returns at once
/// outside its outer namespace.
#[test]
#[ignore = "runs only inside broker_in_root_owned_namespace"]
fn nested_inside() {
    let Some(archive) = std::env::var_os("ONESIMUS_PODMAN_NESTED").map(PathBuf::from) else { return };
    let broker = PathBuf::from(std::env::var_os("ONESIMUS_PODMAN_BROKER").unwrap());
    // SAFETY: geteuid has no preconditions.
    assert_eq!(unsafe { libc::geteuid() }, 0, "root of the outer namespace");
    let n = Path::new(BASE).join(format!("nested-{}", std::process::id()));
    let result = std::panic::catch_unwind(|| nested_case(&n, &archive, &broker));
    let _ = std::fs::remove_dir_all(&n);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
    println!("NESTED-OK");
}

/// The account of the nested run: outer uid/gid 1, sub-IDs outer 2..65535.
const ACCOUNT: u32 = 1;

/// The body of [`nested_inside`].
fn nested_case(n: &Path, archive: &Path, broker: &Path) {
    use crate::broker::UserNs;
    let _ = std::fs::remove_dir_all(n);
    for d in ["plugins/podman", "home/.config/containers", "xdg", "root", "run", "tmp"] {
        std::fs::create_dir_all(n.join(d)).unwrap();
    }
    std::fs::write(n.join("home/.config/containers/containers.conf"),
        "[engine]\nlock_type = \"file\"\ncgroup_manager = \"cgroupfs\"\nevents_logger = \"file\"\n").unwrap();
    for d in ["home", "home/.config", "home/.config/containers", "home/.config/containers/containers.conf", "xdg", "root", "run", "tmp"] {
        std::os::unix::fs::chown(n.join(d), Some(ACCOUNT), Some(ACCOUNT)).unwrap();
    }
    std::fs::set_permissions(n.join("xdg"), std::fs::Permissions::from_mode(0o700)).unwrap();
    for d in [n.to_path_buf(), n.join("plugins"), n.join("plugins/podman")] {
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let plugin_dir = n.join("plugins");
    std::fs::copy(broker, plugin_dir.join("podman/onesimus-podman-broker")).unwrap();
    std::fs::copy("/bin/sh", plugin_dir.join("podman/sh")).unwrap();
    std::fs::copy(archive, n.join("busybox.tar")).unwrap();
    std::fs::set_permissions(n.join("busybox.tar"), std::fs::Permissions::from_mode(0o644)).unwrap();
    let hash = |p: &Path| {
        use sha2::Digest;
        sha2::Sha256::digest(std::fs::read(p).unwrap()).iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    let parts = vec![
        onesimus_plugins::NativePart { path: "podman/onesimus-podman-broker".into(), sha256: hash(&plugin_dir.join("podman/onesimus-podman-broker")) },
        onesimus_plugins::NativePart { path: "podman/sh".into(), sha256: hash(&plugin_dir.join("podman/sh")) },
    ];
    for p in ["podman/onesimus-podman-broker", "podman/sh"] {
        std::fs::set_permissions(plugin_dir.join(p), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let manifest = onesimus_plugins::Manifest {
        schema_version: 3,
        plugin_id: onesimus_plugins::PODMAN_PLUGIN_ID.into(),
        version: "1.0.0".into(),
        api_version: 2,
        artifact_sha256: String::new(),
        signing_key_id: String::new(),
        not_before: 0,
        not_after: u64::MAX,
        capabilities: vec!["podman.inventory".into()],
        target: "wasm32-wasip2".into(),
        native: [("test-triplet".to_string(), parts)].into(),
        suite: None,
    };
    let map = IdMap::rootless(ACCOUNT, &[(2, 65534)]).unwrap();
    let env = vec![
        ("HOME".to_string(), n.join("home").display().to_string()),
        ("XDG_RUNTIME_DIR".to_string(), n.join("xdg").display().to_string()),
        ("PATH".to_string(), "/usr/sbin:/usr/bin:/sbin:/bin".to_string()),
        ("LANG".to_string(), "C".to_string()),
        ("_CONTAINERS_USERNS_CONFIGURED".to_string(), "done".to_string()),
        ("_CONTAINERS_ROOTLESS_UID".to_string(), ACCOUNT.to_string()),
        ("_CONTAINERS_ROOTLESS_GID".to_string(), ACCOUNT.to_string()),
    ];
    let ns = UserNs { uid_map: map.proc_map(), gid_map: map.proc_map(), env };
    let storage = format!("--root {r} --runroot {rr} --tmpdir {t} --storage-driver vfs",
        r = n.join("root").display(), rr = n.join("run").display(), t = n.join("tmp").display());
    // (3) Podman under NNP with the three variables, without a pause process.
    let script = format!(
        "P='podman {storage}'; exec 2>&1; echo uid=$(id -u); grep NoNewPrivs /proc/self/status; \
         $P info --format 'rootless={{{{.Host.Security.Rootless}}}} graphroot={{{{.Store.GraphRoot}}}}' && echo INFO-OK; \
         $P load -q -i {img} >/dev/null && echo LOAD-OK; \
         $P ps -a --format json >/dev/null && echo PS-OK; \
         $P volume ls >/dev/null && echo VOLUME-OK; \
         $P create --name c1 --network none docker.io/library/busybox:1.36 true >/dev/null && echo CREATE-OK; \
         $P inspect c1 --format '{{{{.Name}}}}' && echo INSPECT-OK; \
         $P run --rm --network none docker.io/library/busybox:1.36 true && echo RUN-OK; \
         test ! -e {xdg}/libpod/tmp/pause.pid && echo NO-PAUSE",
        img = n.join("busybox.tar").display(), xdg = n.join("xdg").display()
    );
    let mut sh = crate::broker::launch_in_userns(&plugin_dir, &manifest, "test-triplet", "podman/sh", &ns,
        &["-c".to_string(), script], 0, n).unwrap();
    let mut out = String::new();
    std::io::Read::read_to_string(&mut sh.socket, &mut out).unwrap();
    sh.wait();
    eprintln!("--- podman in the root-owned namespace ---\n{out}");
    assert!(out.contains("uid=0") && out.contains("NoNewPrivs:\t1"), "{out}");
    assert!(out.contains(&format!("rootless=true graphroot={}", n.join("root").display())), "{out}");
    for marker in ["INFO-OK", "LOAD-OK", "PS-OK", "VOLUME-OK", "CREATE-OK", "INSPECT-OK", "RUN-OK"] {
        assert!(out.contains(marker), "{marker} missing:\n{out}");
    }

    // The real broker in the same kind of namespace.
    let args: Vec<String> = ["--root", "root", "--runroot", "run", "--tmpdir", "tmp"].chunks(2)
        .flat_map(|c| [c[0].to_string(), n.join(c[1]).display().to_string()]).chain(["--rootless".to_string()]).collect();
    let b = crate::broker::launch_in_userns(&plugin_dir, &manifest, "test-triplet", "podman/onesimus-podman-broker", &ns, &args, 0, n).unwrap();
    let mut client = BrokerClient { io: b.socket.try_clone().unwrap() };
    use crate::podman_port::Backend;
    let r = client.request(pb::BrokerRequest { request: Some(pb::broker_request::Request::EngineIdentify(pb::EngineIdentifyRequest {})) }).unwrap();
    let Some(pb::broker_response::Response::EngineIdentify(e)) = r.response else { panic!("{r:?}") };
    let info: serde_json::Value = serde_json::from_slice(&e.info_json).unwrap();
    assert_eq!(info.pointer("/host/security/rootless"), Some(&serde_json::Value::Bool(true)));
    assert_eq!(info.pointer("/store/graphRoot").and_then(|v| v.as_str()), n.join("root").to_str());
    let r = client.request(pb::BrokerRequest { request: Some(pb::broker_request::Request::Inventory(pb::InventoryRequest {})) }).unwrap();
    let Some(pb::broker_response::Response::Inventory(inv)) = r.response else { panic!("{r:?}") };
    assert!(String::from_utf8_lossy(&inv.containers_json).contains("c1"));

    // (4) The account tries to ptrace the broker: EPERM (namespace owned by
    // root, broker not dumpable).
    let pid = b.pid();
    // SAFETY: fork; the child only makes raw system calls and exits.
    let child = unsafe { libc::fork() };
    if child == 0 {
        // SAFETY: child process: become the account, try to attach, report.
        unsafe {
            if libc::setresgid(ACCOUNT, ACCOUNT, ACCOUNT) < 0 || libc::setresuid(ACCOUNT, ACCOUNT, ACCOUNT) < 0 {
                libc::_exit(3);
            }
            let r = libc::ptrace(libc::PTRACE_ATTACH, pid, std::ptr::null_mut::<libc::c_void>(), std::ptr::null_mut::<libc::c_void>());
            let e = *libc::__errno_location();
            libc::_exit(if r < 0 && e == libc::EPERM { 0 } else { 1 });
        }
    }
    let mut status = 0;
    // SAFETY: waiting for our child.
    unsafe { libc::waitpid(child, &mut status, 0) };
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "ptrace by the account was not refused (status {status})");
    drop(client);
    drop(b);
    // Cleanup inside: the containers of the nested store (rm -af, rmi -a).
    let cleanup = format!("P='podman {storage}'; $P rm -af; $P volume rm -a; $P rmi -a");
    let mut c = crate::broker::launch_in_userns(&plugin_dir, &manifest, "test-triplet", "podman/sh", &ns, &["-c".to_string(), cleanup], 0, n).unwrap();
    let mut sink = String::new();
    let _ = std::io::Read::read_to_string(&mut c.socket, &mut sink);
    c.wait();
}

/// Exit criterion P0 (3), last part: `systemctl --user show` as the
/// account's uid with NNP and only XDG_RUNTIME_DIR (here: the test user,
/// read-only, a unit unrelated to Podman).
#[test]
#[ignore = "needs a running user manager of the test user"]
fn systemctl_user_show_as_account() {
    let (uid, gid) = me();
    let runtime = PathBuf::from(format!("/run/user/{uid}"));
    if !runtime.join("bus").exists() {
        eprintln!("no user manager; skipped");
        return;
    }
    let out = crate::podman_port::unit_show_as_account(uid, gid, &runtime, &["default.target".to_string()]).unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("Id=default.target") && text.contains("ActiveState="), "{text}");
    assert!(!text.contains("Environment="), "the fixed property list has no Environment");
}
