//! Signed ofd plugin "postgresql": logical backups of a PostgreSQL instance
//! (docs/plans/POSTGRESQL-PLUGIN.md, class `logical`).
//!
//! The guest has no access to anything but its broker import
//! `onesimus:filed-plugin/postgresql`. ofd decides which databases it may
//! see (plugin line `databases=`, `exclude=`, never templates or the Bareos
//! catalog) and runs the broker as the instance's user.
//!
//! Tree (virtual files, restored to files by default):
//!
//! ```text
//! /@postgresql/<instance>/                     directory
//! /@postgresql/<instance>/globals.sql          pg_dumpall --globals-only (globals=yes)
//! /@postgresql/<instance>/db/<name>/           directory
//! /@postgresql/<instance>/db/<name>/_info.json owner, encoding, size, extensions, tables
//! /@postgresql/<instance>/db/<name>/<name>.dump pg_dump -Fc
//! /@postgresql/<instance>/_info.json           server, databases, time of the backup (last)
//! ```

#![no_std]

extern crate alloc;
extern crate onesimus_guest_runtime as _;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use prost::Message;

wit_bindgen::generate!({ path: "../../crates/onesimus-plugins/wit", world: "postgresql-plugin" });

use onesimus::filed_plugin::postgresql as broker;

mod pb {
    use alloc::string::String;
    use alloc::vec::Vec;

    // filed_plugin.proto (subset)
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
        pub options: alloc::collections::BTreeMap<String, String>,
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
        pub attributes: alloc::collections::BTreeMap<String, String>,
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

    // postgresql_broker.proto (subset)
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct InfoRequest {}
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Database {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(string, tag = "2")]
        pub owner: String,
        #[prost(string, tag = "3")]
        pub encoding: String,
        #[prost(uint64, tag = "4")]
        pub size_bytes: u64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct InfoResponse {
        #[prost(string, tag = "1")]
        pub server_version: String,
        #[prost(uint32, tag = "2")]
        pub major: u32,
        #[prost(message, repeated, tag = "3")]
        pub databases: Vec<Database>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DatabaseInfoRequest {
        #[prost(string, tag = "1")]
        pub database: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Table {
        #[prost(string, tag = "1")]
        pub schema: String,
        #[prost(string, tag = "2")]
        pub name: String,
        #[prost(uint64, tag = "3")]
        pub estimated_rows: u64,
        #[prost(uint64, tag = "4")]
        pub size_bytes: u64,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DatabaseInfoResponse {
        #[prost(string, tag = "1")]
        pub database: String,
        #[prost(string, repeated, tag = "2")]
        pub extensions: Vec<String>,
        #[prost(string, repeated, tag = "3")]
        pub schemas: Vec<String>,
        #[prost(message, repeated, tag = "4")]
        pub tables: Vec<Table>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GlobalsOnly {}
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum What {
        #[prost(string, tag = "1")]
        Database(String),
        #[prost(message, tag = "2")]
        Globals(GlobalsOnly),
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DumpOpenRequest {
        #[prost(oneof = "What", tags = "1, 2")]
        pub what: Option<What>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DumpOpenResponse {
        #[prost(uint32, tag = "1")]
        pub handle: u32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DumpReadRequest {
        #[prost(uint32, tag = "1")]
        pub handle: u32,
        #[prost(uint32, tag = "2")]
        pub max_bytes: u32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DumpReadResponse {
        #[prost(bytes = "vec", tag = "1")]
        pub data: Vec<u8>,
        #[prost(bool, tag = "2")]
        pub eof: bool,
        #[prost(bool, tag = "3")]
        pub exit_ok: bool,
        #[prost(string, tag = "4")]
        pub diagnostic: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DumpCloseRequest {
        #[prost(uint32, tag = "1")]
        pub handle: u32,
    }
}

const RC_OK: i32 = 1;
const RC_ERROR: i32 = 3;
const KIND_DIRECTORY: i32 = 2;
const KIND_VIRTUAL: i32 = 4;

fn result(code: i32, diagnostic: impl Into<String>) -> pb::OperationResult {
    pb::OperationResult { code, diagnostic: diagnostic.into() }
}
fn ok() -> Option<pb::OperationResult> {
    Some(result(RC_OK, ""))
}

/// What a file's content comes from.
#[derive(Clone)]
enum Content {
    Directory,
    /// Generated once (JSON).
    Bytes(Vec<u8>),
    Dump(pb::What),
    /// The cluster _info.json, built after all databases were read.
    ClusterInfo,
}

struct Entry {
    path: Vec<u8>,
    content: Content,
    attributes: BTreeMap<String, String>,
}

struct OpenDump {
    path: Vec<u8>,
    handle: u32,
    offset: u64,
}

struct State {
    instance: String,
    /// The Director's catalog database (from odir), never backed up.
    catalog: String,
    globals: bool,
    tree: Option<Vec<Entry>>,
    next: usize,
    open: Option<OpenDump>,
    info: Option<pb::InfoResponse>,
    dumped: Vec<String>,
}

impl State {
    const fn new() -> Self {
        Self { instance: String::new(), catalog: String::new(), globals: true, tree: None, next: 0, open: None, info: None, dumped: Vec::new() }
    }
}

struct Global(RefCell<State>);
// SAFETY: the component runs single-threaded.
unsafe impl Sync for Global {}
static STATE: Global = Global(RefCell::new(State::new()));

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut STATE.0.borrow_mut())
}

/// A database name as one path component: `/`, `%` and control characters
/// percent-encoded, everything else kept (names with `-` stay readable).
fn component(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '/' | '%' => out.push_str(&format!("%{:02X}", c as u32)),
            c if c.is_control() => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c => out.push(c),
        }
    }
    if out == "." || out == ".." {
        out = out.replace('.', "%2E");
    }
    out
}

/// JSON string literal.
fn js(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn decode<M: Message + Default>(r: Result<Vec<u8>, String>) -> Result<M, String> {
    let bytes = r?;
    M::decode(bytes.as_slice()).map_err(|e| e.to_string())
}

fn database_json(db: &pb::Database, info: &pb::DatabaseInfoResponse) -> Vec<u8> {
    let list = |v: &[String]| v.iter().map(|s| js(s)).collect::<Vec<_>>().join(",");
    let tables = info.tables.iter().map(|t| format!(
        "{{\"schema\":{},\"name\":{},\"estimated_rows\":{},\"size_bytes\":{}}}",
        js(&t.schema), js(&t.name), t.estimated_rows, t.size_bytes)).collect::<Vec<_>>().join(",");
    format!(
        "{{\"database\":{},\"owner\":{},\"encoding\":{},\"size_bytes\":{},\"extensions\":[{}],\"schemas\":[{}],\"tables\":[{}],\"format\":\"pg_dump -Fc\"}}\n",
        js(&db.name), js(&db.owner), js(&db.encoding), db.size_bytes, list(&info.extensions), list(&info.schemas), tables
    ).into_bytes()
}

/// Builds the tree from the databases ofd allows this plugin line.
fn build(state: &mut State) -> Result<(), String> {
    let info: pb::InfoResponse = decode(broker::info(&pb::InfoRequest {}.encode_to_vec()))?;
    let base = format!("/@postgresql/{}", state.instance);
    let mut tree = vec![Entry { path: base.clone().into_bytes(), content: Content::Directory, attributes: BTreeMap::new() }];
    if state.globals {
        tree.push(Entry {
            path: format!("{base}/globals.sql").into_bytes(),
            content: Content::Dump(pb::What::Globals(pb::GlobalsOnly {})),
            attributes: [("content".to_string(), "globals".to_string())].into(),
        });
    }
    for db in &info.databases {
        let dir = format!("{base}/db/{}", component(&db.name));
        let details: pb::DatabaseInfoResponse =
            decode(broker::database_info(&pb::DatabaseInfoRequest { database: db.name.clone() }.encode_to_vec()))?;
        let attributes: BTreeMap<String, String> = [
            ("database".to_string(), db.name.clone()),
            ("owner".to_string(), db.owner.clone()),
            ("encoding".to_string(), db.encoding.clone()),
            ("size_bytes".to_string(), db.size_bytes.to_string()),
        ].into();
        tree.push(Entry { path: dir.clone().into_bytes(), content: Content::Directory, attributes: attributes.clone() });
        tree.push(Entry {
            path: format!("{dir}/_info.json").into_bytes(),
            content: Content::Bytes(database_json(db, &details)),
            attributes: attributes.clone(),
        });
        tree.push(Entry {
            path: format!("{dir}/{}.dump", component(&db.name)).into_bytes(),
            content: Content::Dump(pb::What::Database(db.name.clone())),
            attributes,
        });
    }
    tree.push(Entry { path: format!("{base}/_info.json").into_bytes(), content: Content::ClusterInfo, attributes: BTreeMap::new() });
    state.info = Some(info);
    state.tree = Some(tree);
    Ok(())
}

fn cluster_json(state: &State) -> Vec<u8> {
    let info = state.info.clone().unwrap_or_default();
    let dbs = info.databases.iter().map(|d| js(&d.name)).collect::<Vec<_>>().join(",");
    let dumped = state.dumped.iter().map(|d| js(d)).collect::<Vec<_>>().join(",");
    let mut excluded = vec![js("bareos")];
    if !state.catalog.is_empty() && state.catalog != "bareos" {
        excluded.push(js(&state.catalog));
    }
    format!(
        "{{\"instance\":{},\"server_version\":{},\"major\":{},\"class\":\"logical\",\"databases\":[{}],\"dumped\":[{}],\"globals\":{},\"excluded_catalogs\":[{}]}}\n",
        js(&state.instance), js(&info.server_version), info.major, dbs, dumped, state.globals, excluded.join(",")
    ).into_bytes()
}

struct Plugin;

impl Guest for Plugin {
    fn initialize(request: Vec<u8>) -> Vec<u8> {
        let req = pb::InitializeRequest::decode(request.as_slice()).unwrap_or_default();
        let instance = req.options.get("instance").cloned().unwrap_or_else(|| "main".into());
        let valid = !instance.is_empty() && instance.len() <= 64
            && instance.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        let globals = req.options.get("globals").map(|v| !matches!(v.as_str(), "no" | "false" | "0")).unwrap_or(true);
        let catalog = req.options.get("catalog_database").cloned().unwrap_or_default();
        with_state(|s| {
            s.instance = instance;
            s.globals = globals;
            s.catalog = catalog;
        });
        let result = if valid { ok() } else { Some(result(RC_ERROR, "instance must be [A-Za-z0-9_-]{1,64}")) };
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
            if s.tree.is_none() {
                if let Err(e) = build(s) {
                    return pb::NextFileResponse { result: Some(result(RC_ERROR, e)), item: None }.encode_to_vec();
                }
            }
            let tree = s.tree.as_ref().map(Vec::as_slice).unwrap_or(&[]);
            let item = match tree.get(s.next) {
                Some(e) => pb::Item::File(pb::FileEntry {
                    path: e.path.clone(),
                    kind: if matches!(e.content, Content::Directory) { KIND_DIRECTORY } else { KIND_VIRTUAL },
                    size: match &e.content {
                        Content::Bytes(b) => b.len() as u64,
                        _ => 0,
                    },
                    modified_unix_nanos: 0,
                    mode: if matches!(e.content, Content::Directory) { 0o700 } else { 0o600 },
                    attributes: e.attributes.clone(),
                }),
                None => pb::Item::Complete(pb::ScanComplete {}),
            };
            s.next += 1;
            pb::NextFileResponse { result: ok(), item: Some(item) }.encode_to_vec()
        })
    }

    fn read_file(request: Vec<u8>) -> Vec<u8> {
        let fail = |e: String| pb::ReadFileResponse { result: Some(result(RC_ERROR, e)), ..Default::default() }.encode_to_vec();
        let Ok(req) = pb::ReadFileRequest::decode(request.as_slice()) else { return fail("bad request".into()) };
        with_state(|s| {
            let Some(entry) = s.tree.as_ref().and_then(|t| t.iter().find(|e| e.path == req.path)) else {
                return fail("no such file".into());
            };
            let content = entry.content.clone();
            let slice = |bytes: &[u8]| {
                let start = (req.offset as usize).min(bytes.len());
                let end = start.saturating_add(req.max_bytes as usize).min(bytes.len());
                pb::ReadFileResponse { result: ok(), data: bytes[start..end].to_vec(), eof: end == bytes.len() }.encode_to_vec()
            };
            match content {
                Content::Directory => fail("a directory has no content".into()),
                Content::Bytes(b) => slice(&b),
                Content::ClusterInfo => slice(&cluster_json(s)),
                Content::Dump(what) => {
                    // Dumps are read front to back, one at a time.
                    if s.open.as_ref().is_some_and(|o| o.path != req.path) {
                        return fail("another dump is still open".into());
                    }
                    if s.open.is_none() {
                        if req.offset != 0 {
                            return fail("a dump starts at offset 0".into());
                        }
                        let opened: Result<pb::DumpOpenResponse, String> =
                            decode(broker::dump_open(&pb::DumpOpenRequest { what: Some(what.clone()) }.encode_to_vec()));
                        match opened {
                            Ok(o) => s.open = Some(OpenDump { path: req.path.clone(), handle: o.handle, offset: 0 }),
                            Err(e) => return fail(e),
                        }
                    }
                    let open = s.open.as_ref().map(|o| (o.handle, o.offset)).unwrap_or_default();
                    if req.offset != open.1 {
                        return fail(format!("dump read at {} but the stream is at {}", req.offset, open.1));
                    }
                    let read: Result<pb::DumpReadResponse, String> =
                        decode(broker::dump_read(&pb::DumpReadRequest { handle: open.0, max_bytes: req.max_bytes }.encode_to_vec()));
                    match read {
                        Ok(r) if r.eof => {
                            let _ = broker::dump_close(&pb::DumpCloseRequest { handle: open.0 }.encode_to_vec());
                            s.open = None;
                            if !r.exit_ok {
                                return fail(format!("dump failed: {}", r.diagnostic));
                            }
                            if let pb::What::Database(db) = what {
                                s.dumped.push(db);
                            }
                            pb::ReadFileResponse { result: ok(), data: Vec::new(), eof: true }.encode_to_vec()
                        }
                        Ok(r) => {
                            if let Some(o) = s.open.as_mut() {
                                o.offset += r.data.len() as u64;
                            }
                            pb::ReadFileResponse { result: ok(), data: r.data, eof: false }.encode_to_vec()
                        }
                        Err(e) => {
                            s.open = None;
                            fail(e)
                        }
                    }
                }
            }
        })
    }

    fn write_file(_request: Vec<u8>) -> Vec<u8> {
        // Online restore is phase 3 (new database through pg_restore).
        result(RC_ERROR, "online restore is not available yet; restore to files").encode_to_vec()
    }

    fn close_file(_request: Vec<u8>) -> Vec<u8> {
        result(RC_OK, "").encode_to_vec()
    }

    fn shutdown(_request: Vec<u8>) -> Vec<u8> {
        result(RC_OK, "").encode_to_vec()
    }
}

export!(Plugin);
