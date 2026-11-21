//! Signed test plugins: a scripted Component Model guest, the shared test
//! hierarchy of onesimus-plugins (feature test-ca) and a local policy. Private keys stay in memory; the bundle
//! and the policy hold only public certificates. Test use only.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use onesimus_plugins::testca;
use onesimus_proto_rs::onesimus::{ccc::{PluginMetadata, ReturnCode}, filed::plugin::v1 as pb};
use prost::Message;

use super::Resolved;

/// What one call of an export does.
#[derive(Clone)]
pub enum Step {
    Reply(Vec<u8>),
    Trap,
    Loop,
}

/// Replies of every export, used in order; the last one repeats.
#[derive(Clone, Default)]
pub struct Script {
    pub event: Vec<Step>,
    pub next: Vec<Step>,
    pub read: Vec<Step>,
    pub write: Vec<Step>,
    pub close: Vec<Step>,
    /// Contract version the guest reports (0 = 1).
    pub interface_version: u32,
}

pub fn ok() -> pb::OperationResult {
    pb::OperationResult { code: ReturnCode::RcOk as i32, diagnostic: String::new() }
}
pub fn ok_step() -> Step {
    Step::Reply(ok().encode_to_vec())
}
pub fn file_step(path: &[u8], size: u64) -> Step {
    Step::Reply(pb::NextFileResponse {
        result: Some(ok()),
        item: Some(pb::next_file_response::Item::File(pb::FileEntry {
            path: path.to_vec(),
            kind: pb::FileKind::Virtual as i32,
            size,
            mode: 0o600,
            ..Default::default()
        })),
    }
    .encode_to_vec())
}
pub fn complete_step() -> Step {
    Step::Reply(pb::NextFileResponse {
        result: Some(ok()),
        item: Some(pb::next_file_response::Item::Complete(pb::ScanComplete {})),
    }
    .encode_to_vec())
}
pub fn data_step(data: &[u8], eof: bool) -> Step {
    Step::Reply(pb::ReadFileResponse { result: Some(ok()), data: data.to_vec(), eof }.encode_to_vec())
}

const PAGES: u32 = 16;
const DATA_START: usize = 512 * 1024;

fn component(script: &Script) -> Vec<u8> {
    let init = pb::InitializeResponse {
        result: Some(ok()),
        metadata: Some(PluginMetadata { version: "1.0.0".into(), interface_version: script.interface_version.max(1), ..Default::default() }),
    }
    .encode_to_vec();
    let exports: [(&str, Vec<Step>); 7] = [
        ("initialize", vec![Step::Reply(init)]),
        ("handle-event", script.event.clone()),
        ("next-file", script.next.clone()),
        ("read-file", script.read.clone()),
        ("write-file", script.write.clone()),
        ("close-file", script.close.clone()),
        ("shutdown", vec![ok_step()]),
    ];
    let mut core = String::new();
    let mut lifts = String::new();
    let mut at = DATA_START;
    for (e, (name, steps)) in exports.iter().enumerate() {
        let steps = if steps.is_empty() { vec![ok_step()] } else { steps.clone() };
        // Table: per step (kind, ptr, len) as three i32.
        let table = at;
        at += steps.len() * 12;
        let mut table_bytes = Vec::new();
        for step in &steps {
            let (kind, ptr, len) = match step {
                Step::Reply(bytes) => {
                    core.push_str(&format!("(data (i32.const {at}) \"{}\")\n", escaped(bytes)));
                    let r = (0u32, at as u32, bytes.len() as u32);
                    at += bytes.len().max(1);
                    r
                }
                Step::Trap => (1, 0, 0),
                Step::Loop => (2, 0, 0),
            };
            for v in [kind, ptr, len] {
                table_bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        core.push_str(&format!("(data (i32.const {table}) \"{}\")\n", escaped(&table_bytes)));
        let last = steps.len() - 1;
        core.push_str(&format!(
            r#"(global $c{e} (mut i32) (i32.const 0))
            (func (export "f{e}") (param i32 i32) (result i32) (local $n i32) (local $row i32)
              (local.set $n (global.get $c{e}))
              (if (i32.lt_u (local.get $n) (i32.const {last}))
                (then (global.set $c{e} (i32.add (local.get $n) (i32.const 1)))))
              (local.set $row (i32.add (i32.const {table}) (i32.mul (local.get $n) (i32.const 12))))
              (if (i32.eq (i32.load (local.get $row)) (i32.const 1)) (then unreachable))
              (if (i32.eq (i32.load (local.get $row)) (i32.const 2)) (then (loop $forever (br $forever))))
              (i32.store (i32.const 0) (i32.load offset=4 (local.get $row)))
              (i32.store (i32.const 4) (i32.load offset=8 (local.get $row)))
              (i32.const 0))
            "#
        ));
        lifts.push_str(&format!(
            r#"(alias core export $i "f{e}" (core func $f{e}))
            (func (export "{name}") (param "request" (list u8)) (result (list u8))
              (canon lift (core func $f{e}) (memory $mem) (realloc $realloc)))
            "#
        ));
    }
    assert!(at < PAGES as usize * 65536, "script too large for the test guest");
    wat::parse_str(format!(
        r#"(component
          (core module $m
            (memory (export "memory") {PAGES})
            (func (export "realloc") (param i32 i32 i32 i32) (result i32) (i32.const 4096))
            {core})
          (core instance $i (instantiate $m))
          (alias core export $i "memory" (core memory $mem))
          (alias core export $i "realloc" (core func $realloc))
          {lifts})"#
    ))
    .expect("test guest WAT")
}

fn escaped(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// Writes `<dir>/<name>.pb`, `<dir>/ca.pem` and `<dir>/policy.json` for a
/// guest running `script`, signed by a fresh test CA, with capabilities
/// file.read and file.write.
pub fn signed_plugin(dir: &Path, name: &str, script: &Script) -> Resolved {
    signed_artifact(dir, name, component(script))
}

/// A scripted guest signed for `plugin_id` (not `test.<name>`) with these
/// capabilities, e.g. io.onesimus.podman with contract 2.
pub fn signed_plugin_as(dir: &Path, name: &str, plugin_id: &str, caps: &[&str], script: &Script) -> Resolved {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let bundle = testca::signed_bundle(component(script), plugin_id, caps, Default::default(), now);
    let bundle_path = dir.join(format!("{name}.pb"));
    std::fs::write(&bundle_path, bundle.encode_to_vec()).unwrap();
    std::fs::write(dir.join("ca.pem"), testca::TestCa::shared().root.to_pem().unwrap()).unwrap();
    let policy = serde_json::json!({ "ca_certificates": ["ca.pem"], "target": "wasm32-wasip2" });
    let policy_path = dir.join("policy.json");
    std::fs::write(&policy_path, serde_json::to_vec(&policy).unwrap()).unwrap();
    Resolved { bundle: bundle_path, policy: policy_path }
}

/// A NextFile step with a scan reference (contract 2).
pub fn ref_step(path: &[u8], kind: pb::FileKind, source_ref: &[u8], size: u64, mode: u32) -> Step {
    Step::Reply(pb::NextFileResponse {
        result: Some(ok()),
        item: Some(pb::next_file_response::Item::File(pb::FileEntry {
            path: path.to_vec(),
            kind: kind as i32,
            size,
            mode,
            source_ref: source_ref.to_vec(),
            ..Default::default()
        })),
    }
    .encode_to_vec())
}

/// Like `signed_plugin` for a given component (e.g. a built Rust guest).
pub fn signed_artifact(dir: &Path, name: &str, artifact: Vec<u8>) -> Resolved {
    signed_artifact_with(dir, name, artifact, &["file.read", "file.write"], Default::default())
}

/// A signed bundle with these capabilities and native parts (schema 3, the
/// suite of this build), signed by the shared test hierarchy with an RSA 4096
/// key whose certificate grants exactly this plugin and these capabilities.
/// The policy pins the test root (compiled in for tests) and restricts
/// nothing else.
pub fn signed_artifact_with(dir: &Path, name: &str, artifact: Vec<u8>, caps: &[&str],
    native: std::collections::BTreeMap<String, Vec<onesimus_plugins::NativePart>>) -> Resolved {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let bundle = testca::signed_bundle(artifact, &format!("test.{name}"), caps, native, now);
    let bundle_path = dir.join(format!("{name}.pb"));
    std::fs::write(&bundle_path, bundle.encode_to_vec()).unwrap();
    std::fs::write(dir.join("ca.pem"), testca::TestCa::shared().root.to_pem().unwrap()).unwrap();
    let policy = serde_json::json!({ "ca_certificates": ["ca.pem"], "target": "wasm32-wasip2" });
    let policy_path = dir.join("policy.json");
    std::fs::write(&policy_path, serde_json::to_vec(&policy).unwrap()).unwrap();
    Resolved { bundle: bundle_path, policy: policy_path }
}
