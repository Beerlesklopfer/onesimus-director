//! Signed ofd plugin "openldap": slapcat backups of a local slapd and the
//! online restore into a new target (docs/plans/OPENLDAP-PLUGIN.md).
//!
//! The guest has no access to anything but its broker imports
//! `onesimus:filed-plugin/openldap` (openldap.read) and
//! `onesimus:filed-plugin/openldap-restore` (openldap.write, real only in an
//! online restore). ofd decides which databases and files it may see, runs
//! the broker as the slapd user, and owns the restore target.
//!
//! Tree (virtual files, restored to files by default):
//!
//! ```text
//! /@openldap/<instance>/                         directory
//! /@openldap/<instance>/_info.json               first: kind, version, databases, TLS files, secret kinds
//! /@openldap/<instance>/config/cn=config.ldif    slapd.d: slapcat -n 0
//! /@openldap/<instance>/config/files/<path>      slapd.conf, its includes, /etc/default/slapd
//! /@openldap/<instance>/data/<suffix>.ldif       slapcat -b <suffix>, one file per mdb database
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

wit_bindgen::generate!({ path: "../../crates/onesimus-plugins/wit", world: "openldap-plugin" });

use onesimus::filed_plugin::openldap as broker;
use onesimus::filed_plugin::openldap_restore as loader;

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

    // openldap_broker.proto (subset)
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct InfoRequest {}
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Database {
        #[prost(int32, tag = "1")]
        pub number: i32,
        #[prost(string, tag = "2")]
        pub backend: String,
        #[prost(string, tag = "3")]
        pub suffix: String,
        #[prost(string, tag = "4")]
        pub directory: String,
        #[prost(string, repeated, tag = "5")]
        pub overlays: Vec<String>,
        #[prost(bool, tag = "6")]
        pub selected: bool,
        #[prost(string, tag = "7")]
        pub skipped_reason: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ConfigFile {
        #[prost(uint32, tag = "1")]
        pub id: u32,
        #[prost(string, tag = "2")]
        pub path: String,
        #[prost(uint64, tag = "3")]
        pub size: u64,
        #[prost(bool, tag = "4")]
        pub main: bool,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct InfoResponse {
        #[prost(string, tag = "1")]
        pub server_version: String,
        #[prost(int32, tag = "2")]
        pub kind: i32,
        #[prost(string, tag = "3")]
        pub config_path: String,
        #[prost(message, repeated, tag = "4")]
        pub databases: Vec<Database>,
        #[prost(message, repeated, tag = "5")]
        pub config_files: Vec<ConfigFile>,
        #[prost(string, repeated, tag = "6")]
        pub tls_files: Vec<String>,
        #[prost(string, repeated, tag = "7")]
        pub secret_kinds: Vec<String>,
        #[prost(bool, tag = "8")]
        pub config_selected: bool,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ConfigDump {}
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum What {
        #[prost(message, tag = "1")]
        Config(ConfigDump),
        #[prost(string, tag = "2")]
        Suffix(String),
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
    pub struct ConfigFileOpenRequest {
        #[prost(uint32, tag = "1")]
        pub id: u32,
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
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ConfigFileLoad {
        #[prost(string, tag = "1")]
        pub path: String,
        #[prost(bool, tag = "2")]
        pub main: bool,
    }
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum LoadWhat {
        #[prost(message, tag = "1")]
        Config(ConfigDump),
        #[prost(message, tag = "2")]
        ConfigFile(ConfigFileLoad),
        #[prost(string, tag = "3")]
        Suffix(String),
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct LoadOpenRequest {
        #[prost(oneof = "LoadWhat", tags = "1, 2, 3")]
        pub what: Option<LoadWhat>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct LoadOpenResponse {
        #[prost(uint32, tag = "1")]
        pub handle: u32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct LoadWriteRequest {
        #[prost(uint32, tag = "1")]
        pub handle: u32,
        #[prost(bytes = "vec", tag = "2")]
        pub data: Vec<u8>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct LoadCloseRequest {
        #[prost(uint32, tag = "1")]
        pub handle: u32,
        #[prost(bool, tag = "2")]
        pub abort: bool,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct LoadCloseResponse {
        #[prost(bool, tag = "1")]
        pub ok: bool,
        #[prost(string, tag = "2")]
        pub diagnostic: String,
        #[prost(string, tag = "3")]
        pub target: String,
    }
}

const RC_OK: i32 = 1;
const RC_ERROR: i32 = 3;
const KIND_DIRECTORY: i32 = 2;
const KIND_VIRTUAL: i32 = 4;
const SLAPD_D: i32 = 1;

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
    Bytes(Vec<u8>),
    Dump(pb::What),
    ConfigFile(u32),
}

struct Entry {
    path: Vec<u8>,
    content: Content,
    attributes: BTreeMap<String, String>,
}

struct OpenStream {
    path: Vec<u8>,
    handle: u32,
    offset: u64,
}

/// The file being restored online.
enum Sink {
    /// `_info.json`: kept to learn the main configuration file.
    Info(Vec<u8>),
    Load(u32),
    Ignore,
}
struct Restoring {
    path: Vec<u8>,
    sink: Sink,
    offset: u64,
}

struct State {
    instance: String,
    tree: Option<Vec<Entry>>,
    next: usize,
    open: Option<OpenStream>,
    restoring: Option<Restoring>,
    /// The main slapd.conf, from a restored `_info.json`.
    main_config: Option<String>,
}

impl State {
    const fn new() -> Self {
        Self { instance: String::new(), tree: None, next: 0, open: None, restoring: None, main_config: None }
    }
}

struct Global(RefCell<State>);
// SAFETY: the component runs single-threaded.
unsafe impl Sync for Global {}
static STATE: Global = Global(RefCell::new(State::new()));

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    f(&mut STATE.0.borrow_mut())
}

/// A suffix as one path component: `/`, `%` and control characters
/// percent-encoded, everything else kept (`dc=example,dc=org` stays readable).
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

/// The reverse of `component`.
fn uncomponent(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
fn js_list(v: &[String]) -> String {
    v.iter().map(|s| js(s)).collect::<Vec<_>>().join(",")
}

/// The value of `"key":"…"` in JSON this plugin wrote.
fn json_string(json: &str, key: &str) -> Option<String> {
    let start = json.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let mut out = String::new();
    let mut chars = json[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                c => out.push(c),
            },
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

fn decode<M: Message + Default>(r: Result<Vec<u8>, String>) -> Result<M, String> {
    let bytes = r?;
    M::decode(bytes.as_slice()).map_err(|e| e.to_string())
}

fn info_json(instance: &str, info: &pb::InfoResponse) -> Vec<u8> {
    let kind = if info.kind == SLAPD_D { "slapd.d" } else { "slapd.conf" };
    let dbs = info.databases.iter().map(|d| format!(
        "{{\"number\":{},\"backend\":{},\"suffix\":{},\"directory\":{},\"overlays\":[{}],\"backed_up\":{},\"reason\":{}}}",
        d.number, js(&d.backend), js(&d.suffix), js(&d.directory), js_list(&d.overlays), d.selected, js(&d.skipped_reason)))
        .collect::<Vec<_>>().join(",");
    let files = info.config_files.iter().map(|f| format!("{{\"path\":{},\"size\":{},\"main\":{}}}", js(&f.path), f.size, f.main))
        .collect::<Vec<_>>().join(",");
    format!(
        "{{\"plugin\":\"openldap\",\"instance\":{},\"kind\":{},\"config_path\":{},\"server_version\":{},\
\"consistency\":\"per-database\",\"config\":{},\"databases\":[{}],\"config_files\":[{}],\"tls_files\":[{}],\
\"contains_secrets\":[{}],\"format\":\"slapcat LDIF, slapadd -b <suffix> -q\"}}\n",
        js(instance), js(kind), js(&info.config_path), js(&info.server_version), info.config_selected, dbs, files,
        js_list(&info.tls_files), js_list(&info.secret_kinds)
    ).into_bytes()
}

/// Builds the tree from what ofd allows this plugin line.
fn build(state: &mut State) -> Result<(), String> {
    let info: pb::InfoResponse = decode(broker::info(&pb::InfoRequest {}.encode_to_vec()))?;
    let base = format!("/@openldap/{}", state.instance);
    let dir = |path: String| Entry { path: path.into_bytes(), content: Content::Directory, attributes: BTreeMap::new() };
    let mut tree = vec![dir(base.clone())];
    let kind = if info.kind == SLAPD_D { "slapd.d" } else { "slapd.conf" };
    tree.push(Entry {
        path: format!("{base}/_info.json").into_bytes(),
        content: Content::Bytes(info_json(&state.instance, &info)),
        attributes: [("kind".to_string(), kind.to_string())].into(),
    });
    if info.config_selected {
        tree.push(dir(format!("{base}/config")));
        if info.kind == SLAPD_D {
            tree.push(Entry {
                path: format!("{base}/config/cn=config.ldif").into_bytes(),
                content: Content::Dump(pb::What::Config(pb::ConfigDump {})),
                attributes: [("content".to_string(), "cn=config".to_string())].into(),
            });
        }
        for f in &info.config_files {
            if !f.path.starts_with('/') {
                return Err(format!("configuration file {} is not absolute", f.path));
            }
            tree.push(Entry {
                path: format!("{base}/config/files{}", f.path).into_bytes(),
                content: Content::ConfigFile(f.id),
                attributes: [("content".to_string(), if f.main { "main configuration" } else { "configuration" }.to_string())].into(),
            });
        }
    }
    let selected: Vec<&pb::Database> = info.databases.iter().filter(|d| d.selected).collect();
    if !selected.is_empty() {
        tree.push(dir(format!("{base}/data")));
    }
    for d in selected {
        tree.push(Entry {
            path: format!("{base}/data/{}.ldif", component(&d.suffix)).into_bytes(),
            content: Content::Dump(pb::What::Suffix(d.suffix.clone())),
            attributes: [("suffix".to_string(), d.suffix.clone()), ("backend".to_string(), d.backend.clone())].into(),
        });
    }
    state.tree = Some(tree);
    Ok(())
}

/// The part of a restored path after `/@openldap/<instance>/`.
fn relative(path: &[u8]) -> Option<String> {
    let text = core::str::from_utf8(path).ok()?;
    let rest = text.strip_prefix("/@openldap/")?;
    let (_, rest) = rest.split_once('/')?;
    Some(rest.to_string())
}

/// Opens the sink of a restored file: what it is decides where it goes.
fn open_sink(state: &State, path: &[u8]) -> Result<Sink, String> {
    let Some(rel) = relative(path) else { return Ok(Sink::Ignore) };
    let what = if rel == "_info.json" {
        return Ok(Sink::Info(Vec::new()));
    } else if rel == "config/cn=config.ldif" {
        pb::LoadWhat::Config(pb::ConfigDump {})
    } else if let Some(file) = rel.strip_prefix("config/files/") {
        let original = format!("/{file}");
        let main = match &state.main_config {
            Some(m) => *m == original,
            None => original.ends_with("/slapd.conf"),
        };
        pb::LoadWhat::ConfigFile(pb::ConfigFileLoad { path: original, main })
    } else if let Some(name) = rel.strip_prefix("data/").and_then(|n| n.strip_suffix(".ldif")) {
        pb::LoadWhat::Suffix(uncomponent(name))
    } else {
        return Ok(Sink::Ignore);
    };
    let opened: pb::LoadOpenResponse = decode(loader::load_open(&pb::LoadOpenRequest { what: Some(what) }.encode_to_vec()))?;
    Ok(Sink::Load(opened.handle))
}

struct Plugin;

impl Guest for Plugin {
    fn initialize(request: Vec<u8>) -> Vec<u8> {
        let req = pb::InitializeRequest::decode(request.as_slice()).unwrap_or_default();
        let instance = req.options.get("instance").cloned().unwrap_or_else(|| "main".into());
        let valid = !instance.is_empty() && instance.len() <= 64
            && instance.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        with_state(|s| s.instance = instance);
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
            match content {
                Content::Directory => fail("a directory has no content".into()),
                Content::Bytes(b) => {
                    let start = (req.offset as usize).min(b.len());
                    let end = start.saturating_add(req.max_bytes as usize).min(b.len());
                    pb::ReadFileResponse { result: ok(), data: b[start..end].to_vec(), eof: end == b.len() }.encode_to_vec()
                }
                Content::Dump(_) | Content::ConfigFile(_) => {
                    // Streams are read front to back, one at a time.
                    if s.open.as_ref().is_some_and(|o| o.path != req.path) {
                        return fail("another stream is still open".into());
                    }
                    if s.open.is_none() {
                        if req.offset != 0 {
                            return fail("a stream starts at offset 0".into());
                        }
                        let opened: Result<pb::DumpOpenResponse, String> = match &content {
                            Content::Dump(what) => decode(broker::dump_open(&pb::DumpOpenRequest { what: Some(what.clone()) }.encode_to_vec())),
                            Content::ConfigFile(id) => decode(broker::config_file_open(&pb::ConfigFileOpenRequest { id: *id }.encode_to_vec())),
                            _ => Err("not a stream".into()),
                        };
                        match opened {
                            Ok(o) => s.open = Some(OpenStream { path: req.path.clone(), handle: o.handle, offset: 0 }),
                            Err(e) => return fail(e),
                        }
                    }
                    let (handle, offset) = s.open.as_ref().map(|o| (o.handle, o.offset)).unwrap_or_default();
                    if req.offset != offset {
                        return fail(format!("stream read at {} but it is at {offset}", req.offset));
                    }
                    let read: Result<pb::DumpReadResponse, String> =
                        decode(broker::dump_read(&pb::DumpReadRequest { handle, max_bytes: req.max_bytes }.encode_to_vec()));
                    match read {
                        Ok(r) if r.eof => {
                            let _ = broker::dump_close(&pb::DumpCloseRequest { handle }.encode_to_vec());
                            s.open = None;
                            if !r.exit_ok {
                                return fail(format!("dump failed: {}", r.diagnostic));
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
                            let _ = broker::dump_close(&pb::DumpCloseRequest { handle }.encode_to_vec());
                            s.open = None;
                            fail(e)
                        }
                    }
                }
            }
        })
    }

    fn write_file(request: Vec<u8>) -> Vec<u8> {
        let Ok(req) = pb::WriteFileRequest::decode(request.as_slice()) else {
            return result(RC_ERROR, "bad request").encode_to_vec();
        };
        let Some(file) = req.file else { return result(RC_ERROR, "no file").encode_to_vec() };
        if file.kind == KIND_DIRECTORY {
            return result(RC_OK, "").encode_to_vec();
        }
        with_state(|s| {
            if s.restoring.as_ref().is_some_and(|r| r.path != file.path) {
                return result(RC_ERROR, "another file is still being restored").encode_to_vec();
            }
            if s.restoring.is_none() {
                match open_sink(s, &file.path) {
                    Ok(sink) => s.restoring = Some(Restoring { path: file.path.clone(), sink, offset: 0 }),
                    Err(e) => return result(RC_ERROR, e).encode_to_vec(),
                }
            }
            let Some(r) = s.restoring.as_mut() else { return result(RC_ERROR, "no file").encode_to_vec() };
            if req.offset != r.offset {
                return result(RC_ERROR, format!("write at {} but expected {}", req.offset, r.offset)).encode_to_vec();
            }
            r.offset += req.data.len() as u64;
            if req.data.is_empty() {
                return result(RC_OK, "").encode_to_vec();
            }
            match &mut r.sink {
                Sink::Info(buf) if buf.len() < 1024 * 1024 => buf.extend_from_slice(&req.data),
                Sink::Info(_) | Sink::Ignore => {}
                Sink::Load(handle) => {
                    let w = pb::LoadWriteRequest { handle: *handle, data: req.data };
                    if let Err(e) = loader::load_write(&w.encode_to_vec()) {
                        return result(RC_ERROR, e).encode_to_vec();
                    }
                }
            }
            result(RC_OK, "").encode_to_vec()
        })
    }

    fn close_file(request: Vec<u8>) -> Vec<u8> {
        let Ok(req) = pb::CloseFileRequest::decode(request.as_slice()) else {
            return result(RC_ERROR, "bad request").encode_to_vec();
        };
        with_state(|s| {
            // A file without data (or a directory) may close without a write.
            if s.restoring.as_ref().is_none_or(|r| r.path != req.path) {
                if req.abort || relative(&req.path).is_none() || req.path.ends_with(b"/config") || req.path.ends_with(b"/data") {
                    return result(RC_OK, "").encode_to_vec();
                }
                match open_sink(s, &req.path) {
                    Ok(sink) => s.restoring = Some(Restoring { path: req.path.clone(), sink, offset: 0 }),
                    Err(e) => return result(RC_ERROR, e).encode_to_vec(),
                }
            }
            let Some(r) = s.restoring.take() else { return result(RC_OK, "").encode_to_vec() };
            match r.sink {
                Sink::Ignore => result(RC_OK, "").encode_to_vec(),
                Sink::Info(buf) => {
                    s.main_config = json_string(&String::from_utf8_lossy(&buf), "config_path");
                    result(RC_OK, "").encode_to_vec()
                }
                Sink::Load(handle) => {
                    let closed: Result<pb::LoadCloseResponse, String> =
                        decode(loader::load_close(&pb::LoadCloseRequest { handle, abort: req.abort }.encode_to_vec()));
                    match closed {
                        Ok(c) if c.ok || req.abort => result(RC_OK, "").encode_to_vec(),
                        Ok(c) => result(RC_ERROR, format!("load into {} failed: {}", c.target, c.diagnostic)).encode_to_vec(),
                        Err(e) => result(RC_ERROR, e).encode_to_vec(),
                    }
                }
            }
        })
    }

    fn shutdown(_request: Vec<u8>) -> Vec<u8> {
        result(RC_OK, "").encode_to_vec()
    }
}

export!(Plugin);
