// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Tests of the Podman host port: line and grant checks, the engine
//! confirmation, volume scans, and the session path of scan references
//! (exit criteria P0 (2), (5), (7)).

use super::*;
use crate::backup::{Backup, JobEnv, SdSender};
use crate::find::FfPkt;
use crate::jmsg::JobMsgs;
use crate::plugin::testkit::*;
use crate::plugin::{parse_line, Session};
use crate::restore::Counters;
use bareos_bnet::BnetStream;
use onesimus_plugins::wasm::RuntimeLimits;
use onesimus_proto_rs::onesimus::filed::plugin::v1 as fpb;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::AtomicBool;

/// The test user's uid and gid.
fn me() -> (u32, u32) {
    // SAFETY: getuid/getgid have no preconditions.
    unsafe { (libc::getuid(), libc::getgid()) }
}

/// A fresh directory below the temp dir.
fn tmp(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!("ofd-podport-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    d
}

/// A manifest as `verify` would hand it over.
fn manifest(plugin_id: &str, api: u32, caps: &[&str]) -> onesimus_plugins::Manifest {
    onesimus_plugins::Manifest {
        schema_version: 3,
        plugin_id: plugin_id.into(),
        version: "1.0.0".into(),
        api_version: api,
        artifact_sha256: String::new(),
        signing_key_id: String::new(),
        not_before: 0,
        not_after: u64::MAX,
        capabilities: caps.iter().map(|c| c.to_string()).collect(),
        target: "wasm32-wasip2".into(),
        native: Default::default(),
        suite: None,
    }
}

/// Host files with a policy releasing the test user (as `forgejo`) and root.
fn files(d: &Path, graphroot: &Path) -> HostFiles {
    let (uid, _) = me();
    let policy = serde_json::json!({
        "format": "onesimus-podman-accounts/1",
        "accounts": [
            {"name": "root", "uid": 0, "rootful": true, "domains": [{"graphroot": "/var/lib/containers/storage"}]},
            {"name": "forgejo", "uid": uid, "domains": [{"graphroot": graphroot}]}
        ]
    });
    std::fs::write(d.join("accounts.json"), serde_json::to_vec(&policy).unwrap()).unwrap();
    std::fs::set_permissions(d.join("accounts.json"), std::fs::Permissions::from_mode(0o644)).unwrap();
    std::fs::write(d.join("subuid"), format!("forgejo:100000:65536\n{uid}:100000:65536\n")).unwrap();
    std::fs::write(d.join("subgid"), format!("forgejo:100000:65536\n{uid}:100000:65536\n")).unwrap();
    HostFiles { policy: d.join("accounts.json"), subuid: d.join("subuid"), subgid: d.join("subgid"), owner: me().0, top: d.to_path_buf() }
}

const ALL: &[&str] = &["file.read", "podman.inventory", "podman.volume.read", "podman.image.read", "podman.lifecycle", "podman.rootful"];

#[test]
fn the_line_is_bound_to_the_podman_plugin_and_its_grants() {
    let d = tmp("line");
    let f = files(&d, Path::new("/srv/g"));
    let ok_line = parse_line("podman:user=forgejo:pods=*:consistency=crash").unwrap();
    let (line, account) = check_line(&manifest("io.onesimus.podman", 2, ALL), &ok_line, &f).unwrap();
    assert_eq!((line.consistency, account.name.as_str()), (Consistency::Crash, "forgejo"));
    let refused = |m: onesimus_plugins::Manifest, l: &str| check_line(&m, &parse_line(l).unwrap(), &f).unwrap_err().to_string();
    // Another plugin id with the same grants, or contract 1: never linked (V-11).
    assert!(refused(manifest("io.onesimus.other", 2, ALL), "podman:user=forgejo:pods=*:consistency=crash").contains("bound"));
    assert!(refused(manifest("io.onesimus.podman", 1, ALL), "podman:user=forgejo:pods=*:consistency=crash").contains("bound"));
    // Missing grants are plan errors, never a silent fallback.
    let no_rootful = manifest("io.onesimus.podman", 2, &["podman.inventory", "podman.volume.read"]);
    assert!(refused(no_rootful.clone(), "podman:user=root:quadlet=x:consistency=crash").contains("podman.rootful"));
    assert!(refused(no_rootful.clone(), "podman:user=forgejo:pods=*").contains("podman.lifecycle"));
    assert!(refused(manifest("io.onesimus.podman", 2, &["podman.inventory"]), "podman:user=forgejo:pods=*:consistency=crash")
        .contains("podman.volume.read"));
    // consistency=stop needs P2; an unreleased account is a plan error.
    assert!(refused(manifest("io.onesimus.podman", 2, ALL), "podman:user=forgejo:pods=*").contains("P2"));
    assert!(refused(manifest("io.onesimus.podman", 2, ALL), "podman:user=www-data:pods=*:consistency=crash").contains("not released"));
    // A writable policy is not trusted.
    std::fs::set_permissions(d.join("accounts.json"), std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(check_line(&manifest("io.onesimus.podman", 2, ALL), &ok_line, &f).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

/// Scripted backend answers keyed by request kind.
struct FakeBackend {
    /// podman info.
    info: serde_json::Value,
    /// podman volume inspect of the volume.
    volume: serde_json::Value,
}

impl Backend for FakeBackend {
    fn request(&mut self, request: pb::BrokerRequest) -> Result<pb::BrokerResponse, String> {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let r = match request.request {
            Some(Q::EngineIdentify(_)) => A::EngineIdentify(pb::EngineIdentifyResponse {
                info_json: serde_json::to_vec(&self.info).unwrap(),
                version_json: br#"{"Client":{"Version":"5.4.2","GoVersion":"x"}}"#.to_vec(),
            }),
            Some(Q::Inventory(_)) => A::Inventory(pb::InventoryResponse {
                containers_json: br#"[{"Id":"c","Names":["web"],"Command":["sh","-c","PASSWORD=sentinel-123"]}]"#.to_vec(),
                pods_json: b"[]".to_vec(),
                networks_json: b"[]".to_vec(),
                volumes_json: br#"[{"Name":"v","Driver":"local","Mountpoint":"/x"}]"#.to_vec(),
                images_json: b"[]".to_vec(),
                secrets_json: b"[]".to_vec(),
            }),
            Some(Q::Inspect(_)) => A::Inspect(pb::InspectResponse { json: serde_json::to_vec(&self.volume).unwrap() }),
            _ => return Err("unexpected".into()),
        };
        Ok(pb::BrokerResponse { response: Some(r) })
    }
}

/// A graphroot with volume `v`: a setuid file, a hard link, a FIFO, a link.
fn graphroot(d: &Path) -> PathBuf {
    let g = d.join("graph");
    let data = g.join("volumes/v/_data");
    std::fs::create_dir_all(data.join("bin")).unwrap();
    std::fs::write(data.join("bin/su"), b"host bytes").unwrap();
    std::fs::set_permissions(data.join("bin/su"), std::fs::Permissions::from_mode(0o4755)).unwrap();
    std::fs::hard_link(data.join("bin/su"), data.join("su2")).unwrap();
    nix::unistd::mkfifo(&data.join("fifo"), nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
    std::os::unix::fs::symlink("../bin/su", data.join("link")).unwrap();
    g
}

/// A port over the fake backend for the test user as a rootless account.
fn port(d: &Path, g: &Path) -> PodmanPort {
    let (uid, gid) = me();
    let info = serde_json::json!({
        "host": {"arch": "amd64", "hostname": "secret-host", "security": {"rootless": true},
                 "idMappings": {"uidmap": [{"container_id": 0, "host_id": uid, "size": 1}]}},
        "store": {"graphRoot": g, "graphDriverName": "vfs", "configFile": "/home/x"},
        "version": {"Version": "5.4.2"}
    });
    let volume = serde_json::json!([{"Name": "v", "Driver": "local", "Options": {}, "Mountpoint": g.join("volumes/v/_data")}]);
    let f = files(d, g);
    let (line, account) = check_line(&manifest("io.onesimus.podman", 2, ALL), &parse_line("podman:user=forgejo:pods=*:consistency=crash").unwrap(), &f).unwrap();
    let domain = Domain { uid_map: IdMap::rootless(uid, &[(100000, 65536)]).unwrap(), gid_map: IdMap::rootless(gid, &[(100000, 65536)]).unwrap(), rootless: true };
    PodmanPort::new(Box::new(FakeBackend { info, volume }), line, account, domain, podman_scan::open_graphroot(g).unwrap(), &manifest("", 2, ALL).capabilities)
}

#[test]
fn engine_identify_checks_the_graphroot_and_filters() {
    let d = tmp("engine");
    let g = graphroot(&d);
    let mut p = port(&d, &g);
    let out = p.call(BrokerFunction::Podman(PodmanFunction::EngineIdentify), b"").unwrap();
    let r = pb::EngineIdentifyResponse::decode(out.as_slice()).unwrap();
    let info: serde_json::Value = serde_json::from_slice(&r.info_json).unwrap();
    assert_eq!(info.pointer("/store/graphRoot").and_then(|v| v.as_str()), Some(g.to_str().unwrap()));
    assert!(info.pointer("/host/hostname").is_none() && info.pointer("/store/configFile").is_none(), "positive list");
    // The inventory reaches the guest filtered: no command lines.
    let inv = pb::InventoryResponse::decode(p.call(BrokerFunction::Podman(PodmanFunction::Inventory), b"").unwrap().as_slice()).unwrap();
    assert!(!String::from_utf8_lossy(&inv.containers_json).contains("sentinel-123"));
    // Another graphroot (path or inode) than the policy's is refused.
    let other = d.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let mut p2 = port(&d, &g);
    p2.backend = Box::new(FakeBackend { info: serde_json::json!({"store": {"graphRoot": other}, "host": {"security": {"rootless": true}}}), volume: serde_json::json!([]) });
    assert!(p2.confirm().is_err());
    let mut p3 = port(&d, &g);
    p3.graphroot = podman_scan::open_graphroot(&other).unwrap();
    assert!(p3.confirm().unwrap_err().contains("inode"));
    // P1 functions are not there yet.
    assert!(p.call(BrokerFunction::Podman(PodmanFunction::Inspect), b"").is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn volume_scan_pages_and_refuses_unsupported_volumes() {
    let d = tmp("vscan");
    let g = graphroot(&d);
    let mut p = port(&d, &g);
    let scan = |p: &mut PodmanPort, cursor: u32| -> Result<pb::VolumeScanResponse, String> {
        let req = pb::VolumeScanRequest { volume: "v".into(), cursor }.encode_to_vec();
        p.call(BrokerFunction::Podman(PodmanFunction::VolumeScan), &req).map(|b| pb::VolumeScanResponse::decode(b.as_slice()).unwrap())
    };
    let r = scan(&mut p, 0).unwrap();
    assert!(r.done);
    assert_eq!(r.entries.len(), 6, "data/, bin, bin/su, fifo, link, su2");
    assert!(!r.ownership_ref.is_empty() && r.ownership_size > 0);
    assert!(scan(&mut p, 99).is_err());
    // Bad names, other drivers, options.
    let req = pb::VolumeScanRequest { volume: "../v".into(), cursor: 0 }.encode_to_vec();
    assert!(p.call(BrokerFunction::Podman(PodmanFunction::VolumeScan), &req).is_err());
    for volume in [
        serde_json::json!([{"Name": "v", "Driver": "nfs-plugin", "Options": {}, "Mountpoint": g.join("volumes/v/_data")}]),
        serde_json::json!([{"Name": "v", "Driver": "local", "Options": {"o": "bind", "device": "/etc"}, "Mountpoint": g.join("volumes/v/_data")}]),
        serde_json::json!([{"Name": "v", "Driver": "local", "Options": {}, "Mountpoint": "/elsewhere"}]),
    ] {
        let mut q = port(&d, &g);
        q.backend = Box::new(FakeBackend { info: serde_json::Value::Null, volume });
        q.confirmed = true;
        assert!(scan(&mut q, 0).is_err());
    }
    std::fs::remove_dir_all(&d).unwrap();
}

/// Runs `session` through the real backup send path; (ok, stream, messages).
fn backup(session: Session) -> (i32, Vec<u8>, Vec<String>) {
    let config = crate::config::FdConfig::parse("Director { Name = d; Password = p }").unwrap();
    let canceled = AtomicBool::new(false);
    let mut sd = BnetStream::new(std::io::Cursor::new(Vec::new()));
    let mut msgs = JobMsgs::new("test-fd");
    let mut counters = Counters::default();
    let mut flush = |_: &mut JobMsgs| {};
    let ok = Backup {
        sd: SdSender { sd: &mut sd, msgs: &mut msgs, counters: &mut counters, canceled: &canceled, progress: None },
        env: JobEnv { canceled: &canceled, accurate: None, level_full: true, rerunning: false, base_size: 0 },
        flush: &mut flush,
        buf_size: 64 * 1024,
        workers: 0,
        acl_errors: 0,
        xattr_errors: 0,
        xattr_disabled: false,
        config: &config,
        plugin_index: 0,
    }
    .run_plugin(&FfPkt::default(), "podman:user=forgejo", session);
    (ok, sd.into_inner().into_inner(), msgs.take_queued())
}

/// A podman test plugin (io.onesimus.podman, contract 2) running `script`.
fn podman_session(d: &Path, script: Script, scans: Option<SharedScans>, job_id: u64) -> Session {
    let resolved = signed_plugin_as(d, "podman", onesimus_plugins::PODMAN_PLUGIN_ID, &["file.read", "podman.inventory"],
        &Script { interface_version: 2, ..script });
    let job = fpb::JobContext { job_id, job_name: "podman".into(), operation: fpb::JobOperation::Backup as i32 };
    let runtime = RuntimeLimits { max_chunk_bytes: 64 * 1024, ..RuntimeLimits::default() };
    let mut s = Session::open_resolved(&resolved, &parse_line("podman:user=forgejo:pods=*").unwrap(), job, 0, runtime).unwrap();
    s.scans = scans;
    s
}

/// Scans volume `v` and returns the table and its entries.
fn scanned(d: &Path) -> (SharedScans, Vec<podman_scan::Entry>) {
    let g = graphroot(d);
    let mut p = port(d, &g);
    let req = pb::VolumeScanRequest { volume: "v".into(), cursor: 0 }.encode_to_vec();
    p.call(BrokerFunction::Podman(PodmanFunction::VolumeScan), &req).unwrap();
    let scans = p.scans();
    let entries = scans.lock().unwrap().get("v").unwrap().entries.clone();
    (scans, entries)
}

/// Exit criterion P0 (2), positive case: the guest lists references and
/// even offers bytes and a setuid mode; the stream carries the host's
/// bytes, host IDs and mode & 0o777 only, and ReadFile is never called.
#[test]
fn guest_listed_references_carry_host_bytes_and_attributes() {
    let d = tmp("refs-ok");
    let (scans, entries) = scanned(&d);
    let mut next: Vec<Step> = entries.iter().map(|e| ref_step(&e.path, e.kind.file_kind(), &e.source_ref, 999, 0o4777)).collect();
    next.push(complete_step());
    let script = Script { next, read: vec![data_step(b"GUEST-BYTES", true)], ..Default::default() };
    let (ok, stream, msgs) = backup(podman_session(&d, script, Some(scans), 7001));
    assert_eq!(ok, 1, "{msgs:?}");
    let has = |needle: &[u8]| stream.windows(needle.len()).any(|w| w == needle);
    assert!(has(b"host bytes"));
    assert!(!has(b"GUEST-BYTES"), "ReadFile of the guest was used");
    assert!(has(b"onesimus-podman-ownership/1"));
    // Every attribute record: no special bits; owner is the host uid.
    for (mode, uid) in attribute_modes(&stream) {
        assert_eq!(mode & 0o7000, 0, "special bits in the stream");
        assert_eq!(uid, me().0);
    }
    std::fs::remove_dir_all(&d).unwrap();
}

/// (mode, uid) of every UNIX attribute record in a FD->SD stream.
pub(crate) fn attribute_modes(stream: &[u8]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut b = stream;
    let mut want_attr = false;
    while b.len() >= 4 {
        let n = i32::from_be_bytes(b[..4].try_into().unwrap());
        b = &b[4..];
        if n < 0 {
            continue;
        }
        let rec = &b[..n as usize];
        b = &b[n as usize..];
        if want_attr {
            // "<fi> <type> <fname>\0<attribs>\0…"
            let mut parts = rec.splitn(3, |&c| c == b' ');
            let (_, _, rest) = (parts.next(), parts.next(), parts.next().unwrap_or_default());
            let mut fields = rest.split(|&c| c == 0);
            let _fname = fields.next();
            let attribs = fields.next().unwrap_or_default();
            let (st, _, _) = crate::attribs::decode_stat(attribs);
            out.push((st.mode, st.uid));
            want_attr = false;
        }
        let text = String::from_utf8_lossy(rec);
        let f: Vec<&str> = text.split(' ').collect();
        if f.len() == 3 && f[1] == "1" && f[2] == "0" {
            want_attr = true;
        }
    }
    out
}

/// A named case: the guest steps built from the scan's entries.
type Case = (&'static str, Box<dyn Fn(&[podman_scan::Entry]) -> Vec<Step>>);

/// Exit criterion P0 (2): foreign, wrong-path and duplicate references
/// fail the job before any byte of them is sent.
#[test]
fn foreign_wrong_and_duplicate_references_are_refused() {
    let d = tmp("refs-bad");
    let cases: Vec<Case> = vec![
        ("foreign", Box::new(|e| vec![ref_step(&e[2].path, fpb::FileKind::Regular, &[9u8; 16], 1, 0o644)])),
        ("wrong path", Box::new(|e| {
            let su = e.iter().find(|x| x.relpath == b"bin/su" || x.relpath == b"su2").unwrap();
            vec![ref_step(b"/@podman/uid-1/volumes/v/data/elsewhere", fpb::FileKind::Regular, &su.source_ref, 1, 0o644)]
        })),
        ("duplicate", Box::new(|e| {
            let f = e.iter().find(|x| x.kind == podman_scan::EntryKind::Fifo).unwrap();
            vec![ref_step(&f.path, fpb::FileKind::Fifo, &f.source_ref, 0, 0o600), ref_step(&f.path, fpb::FileKind::Fifo, &f.source_ref, 0, 0o600)]
        })),
        ("wrong kind", Box::new(|e| {
            let f = e.iter().find(|x| x.kind == podman_scan::EntryKind::Fifo).unwrap();
            vec![ref_step(&f.path, fpb::FileKind::Regular, &f.source_ref, 0, 0o600)]
        })),
    ];
    for (i, (name, steps)) in cases.into_iter().enumerate() {
        let (scans, entries) = scanned(&d.join(format!("c{i}")).tap_mkdir());
        let mut next = steps(&entries);
        next.push(complete_step());
        let (ok, _stream, msgs) = backup(podman_session(&d, Script { next, ..Default::default() }, Some(scans), 7100 + i as u64));
        assert_eq!(ok, 0, "{name}: {msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("scan reference") || m.contains("kind")), "{name}: {msgs:?}");
    }
    // A reference without any host scan (e.g. another instance's).
    let (_, entries) = scanned(&d.join("other").tap_mkdir());
    let next = vec![ref_step(&entries[0].path, fpb::FileKind::Directory, &entries[0].source_ref, 0, 0o755), complete_step()];
    let (ok, _, msgs) = backup(podman_session(&d, Script { next, ..Default::default() }, None, 7200));
    assert_eq!(ok, 0, "{msgs:?}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// Creates the directory and returns it.
trait TapMkdir {
    /// mkdir -p, then self.
    fn tap_mkdir(self) -> Self;
}
impl TapMkdir for PathBuf {
    fn tap_mkdir(self) -> Self {
        std::fs::create_dir_all(&self).unwrap();
        self
    }
}

/// Exit criterion P0 (7): a contract-1 plugin (PostgreSQL-like) and the
/// contract-2 Podman test plugin load and run side by side; a contract-1
/// plugin cannot list scan references.
#[test]
fn contract_one_and_two_plugins_load_side_by_side() {
    let d = tmp("side");
    let pg_dir = d.join("pg").tap_mkdir();
    let pg = signed_plugin_as(&pg_dir, "postgresql", "io.onesimus.postgresql", &["file.read"], &Script {
        next: vec![file_step(b"@postgresql/main/a.dump", 2), complete_step()],
        read: vec![data_step(b"pg", true)],
        ..Default::default()
    });
    let job = |id| fpb::JobContext { job_id: id, job_name: "j".into(), operation: fpb::JobOperation::Backup as i32 };
    let runtime = RuntimeLimits { max_chunk_bytes: 64 * 1024, ..RuntimeLimits::default() };
    let pg_session = Session::open_resolved(&pg, &parse_line("postgresql").unwrap(), job(7301), 0, runtime.clone()).unwrap();
    let pod_dir = d.join("pod").tap_mkdir();
    let (scans, entries) = scanned(&pod_dir);
    let next = vec![ref_step(&entries[0].path, fpb::FileKind::Directory, &entries[0].source_ref, 0, 0o755), complete_step()];
    let pod_session = podman_session(&pod_dir, Script { next, ..Default::default() }, Some(scans), 7302);
    // Both loaded at the same time.
    let (ok_pg, s1, m1) = backup(pg_session);
    let (ok_pod, _s2, m2) = backup(pod_session);
    assert_eq!((ok_pg, ok_pod), (1, 1), "{m1:?} {m2:?}");
    assert!(s1.windows(2).any(|w| w == b"pg"));
    // Contract 1 with a scan reference: refused by the host's contract check.
    let v1_dir = d.join("v1").tap_mkdir();
    let v1 = signed_plugin_as(&v1_dir, "postgresql", "io.onesimus.postgresql", &["file.read"], &Script {
        next: vec![ref_step(b"/@x/y", fpb::FileKind::Regular, &[1; 16], 1, 0o644), complete_step()],
        ..Default::default()
    });
    let s = Session::open_resolved(&v1, &parse_line("postgresql").unwrap(), job(7303), 0, runtime).unwrap();
    let (ok, _, _) = backup(s);
    assert_eq!(ok, 0);
    std::fs::remove_dir_all(&d).unwrap();
}
