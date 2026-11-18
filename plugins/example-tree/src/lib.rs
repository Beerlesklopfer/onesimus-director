//! Test plugin for ofd (phase 1 of docs/plans/PLUGIN-PHASE1.md).
//!
//! Backup: a fixed tree under `/@example/<instance>/` with a directory, an
//! empty file, a small file, a file larger than one block and a name with a
//! non-UTF-8 byte. Online restore: every file is checksummed in memory and
//! CloseFile answers `bytes=<n> fnv=<hex>` in its diagnostic. No imports: the
//! guest sees nothing of the host but the protobuf requests.
//!
//! `no_std`: Rust's std pulls WASI imports (clocks, streams, polling) into
//! the component, and the host refuses every import. The guest brings its
//! own allocator and panic handler instead.

#![no_std]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use prost::Message;

extern crate onesimus_guest_runtime as _;

/// Guest state; the component runs single-threaded.
struct Global(RefCell<State>);
// SAFETY: the component runs single-threaded.
unsafe impl Sync for Global {}
static STATE: Global = Global(RefCell::new(State::new()));

wit_bindgen::generate!({ path: "../../crates/onesimus-plugins/wit", world: "plugin" });

mod pb {
    use alloc::string::String;
    use alloc::vec::Vec;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PluginMetadata {
        #[prost(string, tag = "5")]
        pub version: String,
        #[prost(uint32, tag = "8")]
        pub interface_version: u32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct OperationResult {
        #[prost(int32, tag = "1")]
        pub code: i32,
        #[prost(string, tag = "2")]
        pub diagnostic: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct InitializeRequest {
        #[prost(btree_map = "string, string", tag = "6")]
        pub options: alloc::collections::BTreeMap<alloc::string::String, alloc::string::String>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct InitializeResponse {
        #[prost(message, optional, tag = "1")]
        pub result: Option<OperationResult>,
        #[prost(message, optional, tag = "2")]
        pub metadata: Option<PluginMetadata>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct FileEntry {
        #[prost(bytes = "vec", tag = "1")]
        pub path: Vec<u8>,
        #[prost(int32, tag = "2")]
        pub kind: i32,
        #[prost(uint64, tag = "3")]
        pub size: u64,
        #[prost(int64, tag = "4")]
        pub modified_unix_nanos: i64,
        #[prost(uint32, tag = "5")]
        pub mode: u32,
        #[prost(btree_map = "string, string", tag = "7")]
        pub attributes: alloc::collections::BTreeMap<alloc::string::String, alloc::string::String>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ScanComplete {}
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Item {
        #[prost(message, tag = "2")]
        File(FileEntry),
        #[prost(message, tag = "3")]
        Complete(ScanComplete),
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct NextFileResponse {
        #[prost(message, optional, tag = "1")]
        pub result: Option<OperationResult>,
        #[prost(oneof = "Item", tags = "2, 3")]
        pub item: Option<Item>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ReadFileRequest {
        #[prost(bytes = "vec", tag = "3")]
        pub path: Vec<u8>,
        #[prost(uint64, tag = "4")]
        pub offset: u64,
        #[prost(uint32, tag = "5")]
        pub max_bytes: u32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ReadFileResponse {
        #[prost(message, optional, tag = "1")]
        pub result: Option<OperationResult>,
        #[prost(bytes = "vec", tag = "2")]
        pub data: Vec<u8>,
        #[prost(bool, tag = "3")]
        pub eof: bool,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct WriteFileRequest {
        #[prost(message, optional, tag = "3")]
        pub file: Option<FileEntry>,
        #[prost(uint64, tag = "4")]
        pub offset: u64,
        #[prost(bytes = "vec", tag = "5")]
        pub data: Vec<u8>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CloseFileRequest {
        #[prost(bytes = "vec", tag = "3")]
        pub path: Vec<u8>,
        #[prost(bool, tag = "4")]
        pub abort: bool,
    }
}

const RC_OK: i32 = 1;
const RC_ERROR: i32 = 3;
const KIND_DIRECTORY: i32 = 2;
const KIND_VIRTUAL: i32 = 4;
/// Size of the large file: more than one 256 KiB block.
pub const BIG: usize = 300 * 1024;

fn result(code: i32, diagnostic: impl Into<String>) -> pb::OperationResult {
    pb::OperationResult { code, diagnostic: diagnostic.into() }
}
fn ok() -> Option<pb::OperationResult> {
    Some(result(RC_OK, ""))
}

/// Content of the large file: a pattern that shows misplaced blocks.
fn big_content() -> Vec<u8> {
    (0..BIG).map(|i| (i % 251) as u8).collect()
}

struct Entry {
    path: Vec<u8>,
    kind: i32,
    content: Vec<u8>,
}

struct State {
    instance: String,
    tree: Vec<Entry>,
    next: usize,
    /// Online restore: path -> (bytes, FNV-1a so far, next offset)
    restored: BTreeMap<Vec<u8>, (u64, u64, u64)>,
}

impl State {
    const fn new() -> Self {
        Self { instance: String::new(), tree: Vec::new(), next: 0, restored: BTreeMap::new() }
    }
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut STATE.0.borrow_mut())
}

fn tree(instance: &str) -> Vec<Entry> {
    let base = format!("/@example/{instance}");
    let p = |s: &str| format!("{base}/{s}").into_bytes();
    let mut odd = p("latin1-");
    odd.push(0xE9); // é in Latin-1: not UTF-8
    vec![
        Entry { path: base.clone().into_bytes(), kind: KIND_DIRECTORY, content: Vec::new() },
        Entry { path: p("empty"), kind: KIND_VIRTUAL, content: Vec::new() },
        Entry { path: p("small.txt"), kind: KIND_VIRTUAL, content: b"hello from the example plugin\n".to_vec() },
        Entry { path: p("big.bin"), kind: KIND_VIRTUAL, content: big_content() },
        Entry { path: odd, kind: KIND_VIRTUAL, content: b"latin-1 name".to_vec() },
    ]
}

fn fnv(mut h: u64, data: &[u8]) -> u64 {
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}
const FNV_START: u64 = 0xcbf2_9ce4_8422_2325;

struct Tree;

impl Guest for Tree {
    fn initialize(request: Vec<u8>) -> Vec<u8> {
        let req = pb::InitializeRequest::decode(request.as_slice()).unwrap_or_default();
        let instance = req.options.get("instance").cloned().unwrap_or_else(|| "main".into());
        let valid = !instance.is_empty() && instance.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        let result = if valid { ok() } else { Some(result(RC_ERROR, "instance must be [A-Za-z0-9_-]+")) };
        with_state(|s| {
            s.tree = tree(&instance);
            s.instance = instance;
        });
        pb::InitializeResponse {
            result,
            metadata: Some(pb::PluginMetadata { version: env!("CARGO_PKG_VERSION").into(), interface_version: 1 }),
        }
        .encode_to_vec()
    }

    fn handle_event(_request: Vec<u8>) -> Vec<u8> {
        result(RC_OK, "").encode_to_vec()
    }

    fn next_file(_request: Vec<u8>) -> Vec<u8> {
        with_state(|s| {
            let item = match s.tree.get(s.next) {
                Some(e) => {
                    let mut attributes = BTreeMap::new();
                    attributes.insert("instance".to_string(), s.instance.clone());
                    pb::Item::File(pb::FileEntry {
                        path: e.path.clone(),
                        kind: e.kind,
                        size: e.content.len() as u64,
                        modified_unix_nanos: 1_700_000_000_000_000_000,
                        mode: if e.kind == KIND_DIRECTORY { 0o750 } else { 0o640 },
                        attributes,
                    })
                }
                None => pb::Item::Complete(pb::ScanComplete {}),
            };
            s.next += 1;
            pb::NextFileResponse { result: ok(), item: Some(item) }.encode_to_vec()
        })
    }

    fn read_file(request: Vec<u8>) -> Vec<u8> {
        let Ok(req) = pb::ReadFileRequest::decode(request.as_slice()) else {
            return pb::ReadFileResponse { result: Some(result(RC_ERROR, "bad request")), ..Default::default() }.encode_to_vec();
        };
        with_state(|s| {
            let Some(e) = s.tree.iter().find(|e| e.path == req.path) else {
                return pb::ReadFileResponse { result: Some(result(RC_ERROR, "no such file")), ..Default::default() }
                    .encode_to_vec();
            };
            let start = (req.offset as usize).min(e.content.len());
            let end = start.saturating_add(req.max_bytes as usize).min(e.content.len());
            pb::ReadFileResponse { result: ok(), data: e.content[start..end].to_vec(), eof: end == e.content.len() }
                .encode_to_vec()
        })
    }

    fn write_file(request: Vec<u8>) -> Vec<u8> {
        let Ok(req) = pb::WriteFileRequest::decode(request.as_slice()) else {
            return result(RC_ERROR, "bad request").encode_to_vec();
        };
        let Some(file) = req.file else { return result(RC_ERROR, "no file").encode_to_vec() };
        with_state(|s| {
            let entry = s.restored.entry(file.path).or_insert((0, FNV_START, 0));
            if req.offset != entry.2 {
                return result(RC_ERROR, format!("write at {} but expected {}", req.offset, entry.2)).encode_to_vec();
            }
            entry.0 += req.data.len() as u64;
            entry.1 = fnv(entry.1, &req.data);
            entry.2 += req.data.len() as u64;
            result(RC_OK, "").encode_to_vec()
        })
    }

    fn close_file(request: Vec<u8>) -> Vec<u8> {
        let Ok(req) = pb::CloseFileRequest::decode(request.as_slice()) else {
            return result(RC_ERROR, "bad request").encode_to_vec();
        };
        with_state(|s| {
            if req.abort {
                s.restored.remove(&req.path);
                return result(RC_OK, "aborted").encode_to_vec();
            }
            let (bytes, hash, _) = s.restored.get(&req.path).copied().unwrap_or((0, FNV_START, 0));
            result(RC_OK, format!("bytes={bytes} fnv={hash:016x}")).encode_to_vec()
        })
    }

    fn shutdown(_request: Vec<u8>) -> Vec<u8> {
        result(RC_OK, "").encode_to_vec()
    }
}

export!(Tree);
