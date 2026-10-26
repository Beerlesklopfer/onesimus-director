//! Integration test of the typed gRPC API (DirectorApi, S-34).
//!
//! Starts target/debug/odir with the parity harness configuration
//! (tests/parity/parity.py setup) on a throw-away copy of the catalog
//! template `bareos_cppref`, calls every read RPC with a tonic client and
//! compares the typed result with the console's `.api 2` JSON (or plain
//! text) of the same command, fetched over BNET with the bnet_capture
//! example. Also checks console ACLs, UNIMPLEMENTED write RPCs and that the
//! existing Director service (ExecuteCommand, ListCommands) is unchanged.
//!
//! Needs PostgreSQL (CREATEDB right), python3 and the built binaries:
//!   cargo build -p odir
//!   cargo build -p bareos-bnet --example bnet_capture
//!   cargo test -p odir --test grpc_typed_api -- --ignored --nocapture
//! Ports: TYPED_API_PORT (BNET, default 39240) and +1 (gRPC).
//! Only read commands are sent; the live File/Storage daemons are asked for
//! status only.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use bytes::{Buf, BufMut};
use hmac::{Hmac, Mac};
use hyper_util::rt::TokioIo;
use prost::Message;
use prost_types::field_descriptor_proto::{Label, Type};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tonic::codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder};
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, Endpoint, Uri};
use tonic::{Code, Request, Status};

use onesimus_proto_rs::onesimus::director::api::{self as pb, director_api_client::DirectorApiClient};
use onesimus_proto_rs::onesimus::director::commands as cmd;
use onesimus_proto_rs::onesimus::director::{
    director_client::DirectorClient, ChallengeRequest, CommandRequest, ListCommandsRequest, LoginRequest,
    ResponseFormat,
};
use onesimus_proto_rs::typed::{self, field_name, DIRECTOR_API_SERVICE};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const PASSWORD: &str = "parity-dir-pw";
const SESSION_HEADER: &str = "x-onesimus-session";
const LIMITED: &str = "typed-api-limited";
const LIMITED_PW: &str = "typed-api-limited-pw";

// ---------------------------------------------------------------------------
// Director under test
// ---------------------------------------------------------------------------

struct Director {
    child: Child,
    db: String,
    work: PathBuf,
    /// self-signed gRPC certificate (in the Director's working directory)
    cert: PathBuf,
    port: u16,
    grpc_port: u16,
    /// private daemons of the development-build test (stopped first)
    daemons: Vec<Child>,
}

impl Drop for Director {
    fn drop(&mut self) {
        for daemon in &mut self.daemons {
            unsafe { libc::kill(daemon.id() as i32, libc::SIGTERM) };
            let _ = daemon.wait();
        }
        unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        psql(&format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.db));
        if !std::thread::panicking() {
            let _ = std::fs::remove_dir_all(&self.work);  // kept for the log after a failure
        }
    }
}

fn psql(sql: &str) -> std::process::Output {
    Command::new("psql").args(["-d", "postgres", "-v", "ON_ERROR_STOP=1", "-Atc", sql]).output().expect("psql")
}

fn wait_port(port: u16, seconds: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn start_director() -> Director {
    let root = Path::new(ROOT);
    let binary = root.join("target/debug/odir");
    assert!(binary.exists(), "build first: cargo build -p odir");
    assert!(root.join("target/debug/examples/bnet_capture").exists(),
            "build first: cargo build -p bareos-bnet --example bnet_capture");
    let port: u16 = std::env::var("TYPED_API_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(39240);
    let grpc_port = port + 1;
    let db = format!("typed_api_{}", std::process::id());
    let work = std::env::temp_dir().join(format!("onesimus-typed-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);

    // Throw-away catalog copy; never the template itself
    let created = psql(&format!("CREATE DATABASE \"{db}\" TEMPLATE \"bareos_cppref\""));
    assert!(created.status.success(), "createdb: {}", String::from_utf8_lossy(&created.stderr));
    let setup = Command::new("python3")
        .args(["tests/parity/parity.py", "setup", "--work"]).arg(&work)
        .args(["--port", &port.to_string(), "--db", &db])
        .current_dir(root).status().expect("parity.py setup");
    assert!(setup.success());
    let conf = work.join("conf/bareos-dir.d");
    let dir_conf = conf.join("director/bareos-dir.conf");
    let text = std::fs::read_to_string(&dir_conf).unwrap();
    std::fs::write(&dir_conf, text).unwrap();
    // Restricted console for the ACL checks (only in this test's copy)
    std::fs::write(conf.join("console/typed-api-limited.conf"), format!(
        "Console {{\n  Name = {LIMITED}\n  Password = \"{LIMITED_PW}\"\n\
         CommandACL = .api, list, llist, show, .jobs, .clients, .pools\n\
         JobACL = backup-bareos-fd\n  ClientACL = bareos-fd\n  PoolACL = Full, Incremental\n\
         StorageACL = File\n  CatalogACL = MyCatalog\n  FileSetACL = SelfTest\n}}\n")).unwrap();

    let log = std::fs::File::create(work.join("rust.log")).unwrap();
    let child = Command::new(&binary).arg("-f").arg("--grpc-port").arg(grpc_port.to_string()).arg("-c").arg(&conf)
        .env("LC_ALL", "C").env("LANG", "C").env("TZ", "UTC").env("NO_COLOR", "1")
        .stdout(Stdio::from(log.try_clone().unwrap())).stderr(Stdio::from(log))
        .spawn().expect("start odir");
    let cert = work.join("conf/work/onesimus-grpc.crt");
    let director = Director { child, db, work, cert, port, grpc_port, daemons: Vec::new() };
    assert!(wait_port(port, 30) && wait_port(grpc_port, 30), "Director did not open its ports");
    director
}

// ---------------------------------------------------------------------------
// Console side (BNET, bnet_capture)
// ---------------------------------------------------------------------------

fn console(port: u16, name: &str, password: &str, steps: &[&str]) -> Vec<String> {
    let out = Command::new(Path::new(ROOT).join("target/debug/examples/bnet_capture"))
        .arg(format!("127.0.0.1:{port}")).arg(name).arg(password).args(steps)
        .env("LC_ALL", "C").env("TZ", "UTC").env("CAPTURE_TIMEOUT_MS", "20000")
        .output().expect("bnet_capture");
    assert!(out.status.success(), "bnet_capture: {}", String::from_utf8_lossy(&out.stderr));
    let data: Value = serde_json::from_slice(&out.stdout).unwrap();
    data["steps"].as_array().unwrap().iter().map(|step| {
        let mut bytes = Vec::new();
        for frame in step["frames"].as_array().unwrap() {
            if let Some(d) = frame["data"].as_str() {
                bytes.extend(base64::engine::general_purpose::STANDARD.decode(d).unwrap());
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }).collect()
}

/// `.api 2` JSON envelope of a console command.
fn console_json(port: u16, name: &str, password: &str, line: &str) -> Value {
    let replies = console(port, name, password, &[".api 2", line]);
    serde_json::from_str(&replies[1]).unwrap_or_else(|e| panic!("{line}: no JSON ({e}): {}", replies[1]))
}

// ---------------------------------------------------------------------------
// gRPC client (pinned TLS, login, raw-bytes codec for data-driven calls)
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Pin(String);

impl ServerCertVerifier for Pin {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: &ServerName<'_>,
                          _: &[u8], _: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        if hex::encode(Sha256::digest(end_entity.as_ref())) == self.0 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("fingerprint mismatch".into()))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct)
        -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &rustls::crypto::ring::default_provider().signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider().signature_verification_algorithms.supported_schemes()
    }
}

async fn channel(director: &Director) -> Channel {
    // The Director keeps its self-signed certificate in the working directory
    let pem = std::fs::read(&director.cert).unwrap();
    let der = rustls_pemfile::certs(&mut pem.as_slice()).next().unwrap().unwrap();
    let fingerprint = hex::encode(Sha256::digest(der.as_ref()));
    let port = director.grpc_port;
    let connector = tower::service_fn(move |_: Uri| {
        let fingerprint = fingerprint.clone();
        async move {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let mut config = rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions().map_err(std::io::Error::other)?
                .dangerous().with_custom_certificate_verifier(Arc::new(Pin(fingerprint)))
                .with_no_client_auth();
            config.alpn_protocols = vec![b"h2".to_vec()];
            let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
            let tls = tokio_rustls::TlsConnector::from(Arc::new(config))
                .connect(ServerName::try_from("localhost").unwrap(), tcp).await?;
            Ok::<_, std::io::Error>(TokioIo::new(tls))
        }
    });
    Endpoint::from_shared(format!("http://127.0.0.1:{port}")).unwrap()
        .connect_with_connector(connector).await.expect("gRPC connect")
}

fn password_key(password: &str) -> String {
    hex::encode(md5::Md5::digest(password.as_bytes()))
}

async fn login(channel: &Channel, console: &str, password: &str) -> MetadataValue<tonic::metadata::Ascii> {
    let mut client = DirectorClient::new(channel.clone());
    let nonce = client.get_challenge(ChallengeRequest { console_name: console.into() }).await.unwrap().into_inner().nonce;
    let mut mac = Hmac::<Sha256>::new_from_slice(password_key(password).as_bytes()).unwrap();
    mac.update(nonce.as_bytes());
    let proof = hex::encode(mac.finalize().into_bytes());
    let login = client.login(LoginRequest { console_name: console.into(), nonce, proof }).await.unwrap().into_inner();
    assert!(login.success, "login {console}: {}", login.error_message);
    login.session_token.parse().unwrap()
}

/// Codec passing pre-encoded protobuf bytes through.
#[derive(Default, Clone)]
struct RawCodec;
struct RawEncoder;
struct RawDecoder;

impl Encoder for RawEncoder {
    type Item = Vec<u8>;
    type Error = Status;
    fn encode(&mut self, item: Vec<u8>, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        dst.put_slice(&item);
        Ok(())
    }
}

impl Decoder for RawDecoder {
    type Item = Vec<u8>;
    type Error = Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Vec<u8>>, Status> {
        Ok(Some(src.copy_to_bytes(src.remaining()).to_vec()))
    }
}

impl Codec for RawCodec {
    type Encode = Vec<u8>;
    type Decode = Vec<u8>;
    type Encoder = RawEncoder;
    type Decoder = RawDecoder;
    fn encoder(&mut self) -> RawEncoder { RawEncoder }
    fn decoder(&mut self) -> RawDecoder { RawDecoder }
}

/// Call a DirectorApi RPC with a JSON request; returns the response as JSON
/// (proto field names, see typed::message_to_json).
async fn call(channel: &Channel, token: &MetadataValue<tonic::metadata::Ascii>, rpc: &str, request: &Value)
    -> Result<Value, Status> {
    let (input, output) = typed::registry().rpc_types(DIRECTOR_API_SERVICE, rpc).unwrap_or_else(|| panic!("no RPC {rpc}"));
    let encoded = typed::json_to_message(&input, request);
    assert!(encoded.unmapped.is_empty(), "{rpc}: request fields {:?} unknown", encoded.unmapped);
    let mut grpc = tonic::client::Grpc::new(channel.clone());
    grpc.ready().await.unwrap();
    let mut req = Request::new(encoded.bytes);
    req.metadata_mut().insert(SESSION_HEADER, token.clone());
    let path = format!("/{DIRECTOR_API_SERVICE}/{rpc}").parse().unwrap();
    let response = grpc.unary(req, path, RawCodec).await?;
    Ok(typed::message_to_json(&output, &response.into_inner()))
}

// ---------------------------------------------------------------------------
// Comparison typed result <-> console JSON
// ---------------------------------------------------------------------------

struct Cmp<'a> {
    ignore: &'a [&'a str],
    errors: Vec<String>,
}

fn scalar_matches(field: &prost_types::FieldDescriptorProto, expected: &Value, typed: Option<&Value>) -> bool {
    let zero = || match expected {
        Value::String(s) => s.is_empty() || s == "0",
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::Bool(b) => !b,
        _ => false,
    };
    match field.r#type() {
        Type::String => typed.and_then(Value::as_str).unwrap_or("") == expected.as_str().unwrap_or("\u{0}"),
        Type::Bool => typed::parse_bool(expected).ok() == Some(typed.and_then(Value::as_bool).unwrap_or(false)),
        Type::Int32 | Type::Int64 => match typed {
            None => zero(),
            Some(t) => typed::parse_i64(expected).ok() == t.as_i64(),
        },
        Type::Uint32 | Type::Uint64 => match typed {
            None => zero(),
            Some(t) => typed::parse_u64(expected).ok() == t.as_u64(),
        },
        Type::Double => typed::parse_f64(expected).ok() == Some(typed.and_then(Value::as_f64).unwrap_or(0.0)),
        Type::Enum => typed::resolve_enum(typed::registry(), field.type_name(), expected).ok()
            == typed.and_then(Value::as_i64).map(|n| n as i32),
        _ => false,
    }
}

impl Cmp<'_> {
    fn fail(&mut self, path: &str, what: String) {
        self.errors.push(format!("{path}: {what}"));
    }

    /// Every key of `expected` (console JSON) must be in `typed`: as typed
    /// field with the same value, in extra_fields as text, or in directives.
    fn object(&mut self, expected: &Map<String, Value>, typed: &Map<String, Value>, type_name: &str, path: &str) {
        let registry = typed::registry();
        let desc = registry.message(type_name).unwrap_or_else(|| panic!("no message {type_name}"));
        if desc.field.iter().any(|f| f.name() == "directives") {
            if typed.get("directives") != Some(&Value::Object(expected.clone())) {
                self.fail(path, format!("directives differ: {:?}", typed.get("directives")));
            }
            return;
        }
        let count_field = desc.field.iter().any(|f| f.name() == "count");
        for (key, value) in expected {
            let p = format!("{path}.{key}");
            if value.is_null() || self.ignore.contains(&key.as_str()) {
                continue;
            }
            if let Some(extra) = typed.get("extra_fields").and_then(|e| e.get(key)) {
                if extra.as_str() != Some(&typed::extra_text(value)) {
                    self.fail(&p, format!("extra {extra} != {value}"));
                }
                continue;
            }
            let name = field_name(key);
            let Some(field) = desc.field.iter().find(|f| f.name() == name) else {
                self.fail(&p, "no field and not in extra_fields".into());
                continue;
            };
            if let (Value::Array(items), true) = (value, count_field) {
                if let [Value::Object(o)] = items.as_slice() {
                    if o.len() == 1 && o.contains_key("count") {
                        if typed::parse_u64(&o["count"]).ok() != typed.get("count").and_then(Value::as_u64) {
                            self.fail(&p, format!("count {:?} != {}", typed.get("count"), o["count"]));
                        }
                        continue;
                    }
                }
            }
            self.field(field, value, typed.get(&name), &p);
        }
    }

    fn value(&mut self, field: &prost_types::FieldDescriptorProto, expected: &Value, typed: Option<&Value>, path: &str) {
        if field.r#type() != Type::Message {
            if !scalar_matches(field, expected, typed) {
                self.fail(path, format!("typed {typed:?} != console {expected}"));
            }
            return;
        }
        match field.type_name() {
            ".google.protobuf.Struct" | ".google.protobuf.Value" | ".google.protobuf.ListValue" => {
                if typed != Some(expected) {
                    self.fail(path, format!("typed {typed:?} != console {expected}"));
                }
            }
            type_name => {
                let empty = Map::new();
                let typed_obj = typed.and_then(Value::as_object).unwrap_or(&empty);
                match expected {
                    Value::Object(o) => self.object(o, typed_obj, type_name, path),
                    Value::Array(_) if type_name.ends_with("List") => {
                        let desc = typed::registry().message(type_name).unwrap();
                        let first = desc.field.iter().find(|f| f.label() == Label::Repeated).unwrap();
                        let mut wrapped = Map::new();
                        wrapped.insert(first.name().to_string(), expected.clone());
                        self.object(&wrapped, typed_obj, type_name, path);
                    }
                    other => self.fail(path, format!("console value {other} for message {type_name}")),
                }
            }
        }
    }

    fn field(&mut self, field: &prost_types::FieldDescriptorProto, expected: &Value, typed: Option<&Value>, path: &str) {
        let registry = typed::registry();
        if field.label() != Label::Repeated {
            return self.value(field, expected, typed, path);
        }
        let entry = registry.message(field.type_name()).filter(|m| m.options.as_ref().is_some_and(|o| o.map_entry()));
        if let Some(entry) = entry {
            let value_field = entry.field.iter().find(|f| f.number() == 2).unwrap();
            let Value::Object(expected) = expected else { return self.fail(path, "console value is no object".into()) };
            let empty = Map::new();
            let typed = typed.and_then(Value::as_object).unwrap_or(&empty);
            if expected.len() != typed.len() {
                self.fail(path, format!("{} entries, typed {}", expected.len(), typed.len()));
            }
            for (key, value) in expected {
                self.value(value_field, value, typed.get(key), &format!("{path}[{key}]"));
            }
            return;
        }
        let Value::Array(expected) = expected else { return self.fail(path, "console value is no array".into()) };
        let empty = Vec::new();
        let typed = typed.and_then(Value::as_array).unwrap_or(&empty);
        if expected.len() != typed.len() {
            return self.fail(path, format!("{} records, typed {}", expected.len(), typed.len()));
        }
        for (i, (e, t)) in expected.iter().zip(typed).enumerate() {
            self.value(field, e, Some(t), &format!("{path}[{i}]"));
        }
    }
}

/// How the console JSON result becomes the typed response (the Director's
/// per-RPC renames).
#[derive(Clone, Copy)]
enum Tf {
    Same,
    Rename(&'static str, &'static str),
    WrapCommands,
    Pools,
    Msgs,
}

fn transform(tf: Tf, mut result: Map<String, Value>) -> Map<String, Value> {
    match tf {
        Tf::Same => {}
        Tf::Rename(from, to) => {
            if let Some(v) = result.remove(from) {
                result.insert(to.into(), v);
            }
        }
        Tf::WrapCommands => {
            let mut o = Map::new();
            o.insert("commands".into(), Value::Object(result));
            return o;
        }
        Tf::Pools => {
            if result.get("volumes").is_some_and(Value::is_object) {
                let v = result.remove("volumes").unwrap();
                result.insert("pools".into(), v);
            }
        }
        Tf::Msgs => {
            if let Some(mut items) = result.remove("messages") {
                for item in items.as_array_mut().unwrap() {
                    let o = item.as_object_mut().unwrap();
                    if let Some(t) = o.remove("text") {
                        o.insert("name".into(), t);
                    }
                }
                result.insert("items".into(), items);
            }
        }
    }
    result
}

/// One read RPC compared with the console.
struct Case {
    rpc: &'static str,
    request: Value,
    line: &'static str,
    tf: Tf,
    /// Text mode: compare `text` with the plain console output (first
    /// `lines` lines; 0 = all)
    text: Option<usize>,
    ignore: &'static [&'static str],
}

fn j(rpc: &'static str, request: Value, line: &'static str) -> Case {
    Case { rpc, request, line, tf: Tf::Same, text: None, ignore: &[] }
}

impl Case {
    fn tf(mut self, tf: Tf) -> Self { self.tf = tf; self }
    fn text(mut self, lines: usize) -> Self { self.text = Some(lines); self }
    fn ignore(mut self, keys: &'static [&'static str]) -> Self { self.ignore = keys; self }
}

fn items(rpc: &'static str, line: &'static str, key: &'static str) -> Case {
    j(rpc, json!({}), line).tf(Tf::Rename(key, "items"))
}

fn cases() -> Vec<Case> {
    vec![
        j("GetVersion", json!({}), "version"),
        j("GetTime", json!({}), "time").ignore(&["full", "minute", "second", "hour"]),
        j("Whoami", json!({}), "whoami"),
        j("GetMemory", json!({}), "memory"),
        j("Noop", json!({}), "."),
        j("Help", json!({}), "help").tf(Tf::WrapCommands),
        j("Help", json!({"command": "list"}), "help list").tf(Tf::WrapCommands),
        j("DotHelp", json!({}), ".help").tf(Tf::WrapCommands),
        j("DotHelp", json!({"item": "list"}), ".help item=list").tf(Tf::WrapCommands),
        j("GetMessages", json!({}), "messages").text(0),
        j("DotMessages", json!({}), ".messages").text(0),
        j("ExpandVariables", json!({"text": "Job"}), "var Job").text(0),
        j("Resolve", json!({"hostname": "localhost"}), "resolve localhost").text(0),
        // list / llist
        j("ListJobs", json!({"limit": 3}), "list jobs limit=3"),
        j("ListJobs", json!({"limit": 2, "offset": 1, "llist": true}), "llist jobs limit=2 offset=1"),
        j("ListJobs", json!({"job": "backup-bareos-fd", "jobstatus": "T", "limit": 2}),
          "list jobs job=backup-bareos-fd jobstatus=T limit=2"),
        j("ListJobs", json!({"count": true}), "list jobs count"),
        j("ListJobs", json!({}), "list jobs"),
        j("GetJob", json!({"jobid": 628}), "list jobid=628"),
        j("GetJob", json!({"jobid": 629, "llist": true}), "llist jobid=629"),
        j("ListBackups", json!({"client": "bareos-fd", "limit": 2}), "list backups client=bareos-fd limit=2")
            .tf(Tf::Rename("backups", "jobs")),
        j("ListBackups", json!({"client": "bareos-fd", "limit": 1, "llist": true}),
          "llist backups client=bareos-fd limit=1").tf(Tf::Rename("backups", "jobs")),
        j("ListCopies", json!({}), "list copies"),
        j("ListJobLog", json!({"jobid": 628}), "list joblog jobid=628"),
        j("ListJobLog", json!({"jobid": 628, "count": true}), "list joblog jobid=628 count"),
        j("ListJobMedia", json!({"jobid": 629}), "list jobmedia jobid=629"),
        j("ListJobMedia", json!({"jobid": 629, "llist": true}), "llist jobmedia jobid=629"),
        j("ListJobTotals", json!({}), "list jobtotals"),
        j("ListJobStatistics", json!({"jobid": 628}), "list jobstatistics jobid=628"),
        j("ListFiles", json!({"jobid": 5}), "list files jobid=5"),
        j("ListBaseFiles", json!({"jobid": 5}), "list basefiles jobid=5").tf(Tf::Rename("files", "filenames")),
        j("ListLog", json!({"limit": 5}), "list log limit=5"),
        j("ListVolumes", json!({}), "list volumes").tf(Tf::Pools),
        j("ListVolumes", json!({"llist": true}), "llist volumes").tf(Tf::Pools),
        j("ListVolumes", json!({"count": true}), "list volumes count").tf(Tf::Pools),
        j("ListVolumes", json!({"pool": "Full"}), "list volumes pool=Full").tf(Tf::Pools),
        j("ListVolumes", json!({"jobid": 629}), "list volumes jobid=629").tf(Tf::Pools),
        j("ListMedia", json!({"jobid": 629, "llist": true}), "llist media jobid=629").tf(Tf::Pools),
        j("GetVolume", json!({"volume": "Full-0001"}), "list volume=Full-0001"),
        j("GetVolume", json!({"mediaid": 47318, "llist": true}), "llist mediaid=47318"),
        j("ListPools", json!({}), "list pools"),
        j("ListPools", json!({"llist": true}), "llist pools"),
        j("ListPools", json!({"pool": "Full"}), "list pool=Full"),
        j("ListClients", json!({}), "list clients"),
        j("ListClients", json!({"client": "bareos-fd", "llist": true}), "llist client=bareos-fd"),
        j("ListFilesets", json!({}), "list filesets"),
        j("ListFilesets", json!({"filesetid": 1}), "list fileset filesetid=1"),
        j("ListStorages", json!({}), "list storages"),
        j("ListNextVolume", json!({"job": "backup-bareos-fd"}), "list nextvolume job=backup-bareos-fd").text(0),
        // status
        j("GetDirectorStatus", json!({}), "status dir"),
        j("GetDirectorStatus", json!({"days": 3}), "status dir days=3"),
        j("GetStatusAll", json!({}), "status all"),
        j("GetSchedulerStatus", json!({}), "status scheduler"),
        j("GetSchedulerStatus", json!({"schedule": "WeeklyCycle"}), "status scheduler schedule=WeeklyCycle"),
        j("GetClientStatus", json!({"client": "bareos-fd"}), "status client=bareos-fd").text(1),
        j("GetStorageStatus", json!({"storage": "File"}), "status storage=File").text(1),
        j("GetSubscriptions", json!({}), "status subscriptions").ignore(&["report-time", "checksum"]),
        j("GetConfigurationStatus", json!({}), "status configuration").text(0),
        j("DotStatusDirector", json!({"header": true}), ".status dir header"),
        j("DotStatusDirector", json!({"scheduled": true}), ".status dir scheduled"),
        j("DotStatusDirector", json!({"running": true}), ".status dir running"),
        j("DotStatusDirector", json!({"terminated": true}), ".status dir terminated"),
        j("DotStatusClient", json!({"client": "bareos-fd", "header": true}), ".status client=bareos-fd header").text(1),
        j("DotStatusStorage", json!({"storage": "File", "header": true}), ".status storage=File header").text(1),
        // show
        j("ShowCatalogs", json!({}), "show catalog"),
        j("ShowCatalogs", json!({"catalog": "MyCatalog"}), "show catalog=MyCatalog"),
        j("ShowClients", json!({}), "show clients"),
        j("ShowClients", json!({"client": "bareos-fd"}), "show client=bareos-fd"),
        j("ShowConsoles", json!({}), "show consoles"),
        j("ShowDirectors", json!({}), "show directors"),
        j("ShowFilesets", json!({}), "show filesets"),
        j("ShowFilesets", json!({"fileset": "LinuxAll"}), "show fileset=LinuxAll"),
        j("ShowJobdefs", json!({}), "show jobdefs"),
        j("ShowJobs", json!({}), "show jobs"),
        j("ShowJobs", json!({"job": "backup-bareos-fd"}), "show job=backup-bareos-fd"),
        j("ShowMessages", json!({}), "show messages"),
        j("ShowPools", json!({}), "show pools"),
        j("ShowPools", json!({"pool": "Full"}), "show pool=Full"),
        j("ShowProfiles", json!({}), "show profiles"),
        j("ShowSchedules", json!({}), "show schedules"),
        j("ShowSchedules", json!({"schedule": "WeeklyCycle"}), "show schedule=WeeklyCycle"),
        j("ShowStorages", json!({}), "show storages"),
        j("ShowStorages", json!({"storage": "File"}), "show storage=File"),
        j("ShowUsers", json!({}), "show users"),
        j("ShowCounters", json!({}), "show counters"),
        j("ShowAll", json!({}), "show all"),
        j("ShowDisabled", json!({}), "show disabled"),
        // dot listings
        items("DotJobs", ".jobs", "jobs"),
        j("DotJobs", json!({"type": "B"}), ".jobs type=B").tf(Tf::Rename("jobs", "items")),
        items("DotJobdefs", ".jobdefs", "jobdefs"),
        items("DotClients", ".clients", "clients"),
        items("DotPools", ".pools", "pools"),
        items("DotStorages", ".storages", "storages"),
        items("DotFilesets", ".filesets", "filesets"),
        items("DotCatalogs", ".catalogs", "catalogs"),
        items("DotConsoles", ".consoles", "consoles"),
        items("DotUsers", ".users", "users"),
        items("DotSchedule", ".schedule", "schedules"),
        j("DotMsgs", json!({}), ".msgs").tf(Tf::Msgs),
        items("DotProfiles", ".profiles", "profiles"),
        items("DotTypes", ".types", "jobtypes"),
        items("DotVolstatus", ".volstatus", "volstatus"),
        items("DotActiononpurge", ".actiononpurge", "actiononpurge"),
        items("DotMedia", ".media", "media"),
        items("DotMediatypes", ".mediatypes", "mediatypes"),
        items("DotLocations", ".locations", "locations"),
        j("DotLevels", json!({}), ".levels"),
        j("DotJobstatus", json!({}), ".jobstatus"),
        j("DotDefaults", json!({"job": "backup-bareos-fd"}), ".defaults job=backup-bareos-fd"),
        j("DotDefaults", json!({"client": "bareos-fd"}), ".defaults client=bareos-fd"),
        j("DotAuthorized", json!({"job": "backup-bareos-fd"}), ".authorized job=backup-bareos-fd"),
        j("DotSqlTables", json!({}), ".sqltables"),
        j("DotSqlCount", json!({}), ".sqlcount"),
        j("DotSqlSchema", json!({"table": "job"}), ".sqlschema table=job"),
        j("DotAclList", json!({}), ".acl"),
        j("DotProfileList", json!({}), ".profile").tf(Tf::Rename("profiles", "items")),
        // BVFS / OVFS
        j("DotBvfsGetJobids", json!({"jobid": 5}), ".bvfs_get_jobids jobid=5"),
        j("DotBvfsLsdirs", json!({"jobid": 5, "path": "/"}), ".bvfs_lsdirs jobid=5 path=/"),
        j("DotBvfsLsfiles", json!({"jobid": 5, "path": "/usr/sbin/"}), ".bvfs_lsfiles jobid=5 path=/usr/sbin/"),
        j("DotBvfsVersions", json!({"client": "bareos-fd", "pathid": "1", "filename": "x"}),
          ".bvfs_versions client=bareos-fd pathid=1 filename=x jobid=0"),
        j("DotOvfsStatus", json!({}), ".ovfs_status"),
        j("DotOvfsLatest", json!({"client": "bareos-fd", "fileset": "SelfTest"}), ".ovfs_latest client=bareos-fd fileset=SelfTest"),
        j("DotOvfsTree", json!({"client": "bareos-fd", "fileset": "SelfTest"}), ".ovfs_tree client=bareos-fd fileset=SelfTest"),
        j("DotOvfsLs", json!({"client": "bareos-fd", "fileset": "SelfTest", "path": "/"}),
          ".ovfs_ls client=bareos-fd fileset=SelfTest path=/"),
        j("DotOvfsVersions", json!({"client": "bareos-fd", "path": "/usr/sbin/", "name": "bareos-fd"}),
          ".ovfs_versions client=bareos-fd path=/usr/sbin/ name=bareos-fd"),
    ]
}

/// Text without the per-session catalog notice (printed on the first
/// catalog use of a console session), first `lines` lines (0 = all).
fn first_lines(text: &str, lines: usize) -> String {
    let kept = text.trim_end().lines()
        .filter(|l| !l.starts_with("Automatically selected Catalog: ") && !l.starts_with("Using Catalog \""));
    if lines == 0 { kept.collect::<Vec<_>>().join("\n") } else { kept.take(lines).collect::<Vec<_>>().join("\n") }
}

/// Compare one case; returns the error list (empty = match).
async fn check_case(director: &Director, channel: &Channel, token: &MetadataValue<tonic::metadata::Ascii>,
                    console_name: &str, password: &str, case: &Case) -> Vec<String> {
    let typed = call(channel, token, case.rpc, &case.request).await;
    let label = format!("{} {} [{}]", case.rpc, case.request, case.line);
    if let Some(lines) = case.text {
        let replies = console(director.port, console_name, password, &[case.line]);
        let typed = match typed {
            Ok(t) => t,
            Err(e) => return vec![format!("{label}: {e:?}")],
        };
        let t = first_lines(typed.get("text").and_then(Value::as_str).unwrap_or(""), lines);
        let c = first_lines(&replies[0], lines);
        return if t == c { Vec::new() } else { vec![format!("{label}: text differs\n typed:   {t:?}\n console: {c:?}")] };
    }
    let envelope = console_json(director.port, console_name, password, case.line);
    match (typed, envelope.get("result")) {
        (Ok(typed), Some(Value::Object(result))) => {
            let expected = transform(case.tf, result.clone());
            let (_, output) = typed::registry().rpc_types(DIRECTOR_API_SERVICE, case.rpc).unwrap();
            let mut cmp = Cmp { ignore: case.ignore, errors: Vec::new() };
            cmp.object(&expected, typed.as_object().unwrap(), &output, "");
            cmp.errors.into_iter().map(|e| format!("{label}: {e}")).collect()
        }
        (Err(status), None) => {
            // the console failed, too: same error text
            let messages: String = envelope["error"]["data"]["messages"]["error"].as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if status.code() == Code::FailedPrecondition && status.message() == messages.trim_end() {
                Vec::new()
            } else {
                vec![format!("{label}: typed error {status:?}, console {envelope}")]
            }
        }
        (typed, _) => vec![format!("{label}: typed {typed:?}, console {envelope}")],
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs PostgreSQL with bareos_cppref and the built binaries"]
async fn typed_api_matches_console_api2() {
    let director = start_director();
    let channel = channel(&director).await;
    let token = login(&channel, "*UserAgent*", PASSWORD).await;

    // --- every read RPC against the console's `.api 2` result -------------
    let cases = cases();
    let mut failures = Vec::new();
    let mut covered = std::collections::BTreeSet::new();
    for case in &cases {
        covered.insert(case.rpc);
        failures.extend(check_case(&director, &channel, &token, "*UserAgent*", PASSWORD, case).await);
    }

    // Error path: same message as the console
    failures.extend(check_case(&director, &channel, &token, "*UserAgent*", PASSWORD,
        &j("GetJobStatus", json!({"job": "backup-bareos-fd"}), "status job=backup-bareos-fd")).await);
    covered.insert("GetJobStatus");

    // --- console ACLs: a restricted console sees what list/show show it ---
    let limited = login(&channel, LIMITED, LIMITED_PW).await;
    for case in [
        j("ListJobs", json!({"limit": 20}), "list jobs limit=20"),
        j("ListClients", json!({}), "list clients"),
        j("ListPools", json!({}), "list pools"),
        j("ListVolumes", json!({}), "list volumes").tf(Tf::Pools),
        j("ShowJobs", json!({}), "show jobs"),
        j("ShowPools", json!({}), "show pools"),
        items("DotJobs", ".jobs", "jobs"),
        items("DotClients", ".clients", "clients"),
        items("DotPools", ".pools", "pools"),
    ] {
        failures.extend(check_case(&director, &channel, &limited, LIMITED, LIMITED_PW, &case).await);
    }
    let jobs = call(&channel, &limited, "ListJobs", &json!({"limit": 50})).await.unwrap();
    assert!(jobs["jobs"].as_array().unwrap().iter().all(|j| j["name"] == "backup-bareos-fd"), "{jobs}");
    let denied = call(&channel, &limited, "DotSqlTables", &json!({})).await;
    assert_eq!(denied.unwrap_err().code(), Code::PermissionDenied);
    // no session token: UNAUTHENTICATED
    let mut grpc = DirectorApiClient::new(channel.clone());
    let anonymous = grpc.get_version(cmd::VersionCommand {}).await;
    assert_eq!(anonymous.unwrap_err().code(), Code::Unauthenticated);

    // --- typed client as the GUI would use it -----------------------------
    let mut req = Request::new(pb::ListJobsCommand { limit: 2, llist: true, ..Default::default() });
    req.metadata_mut().insert(SESSION_HEADER, token.clone());
    let jobs = grpc.list_jobs(req).await.unwrap().into_inner();
    assert_eq!(jobs.jobs.len(), 2);
    assert!(jobs.jobs.iter().all(|j| j.jobid > 0 && j.r#type() != pb::JobType::Unspecified
        && j.jobstatus() != pb::JobStatus::Unspecified));
    assert_eq!(jobs.meta.unwrap().range.unwrap().limit, 2);

    // --- every RPC is either implemented (and tested) or UNIMPLEMENTED ----
    let rpcs = typed::registry().rpc_names(DIRECTOR_API_SERVICE);
    let mut unimplemented = Vec::new();
    for rpc in &rpcs {
        if covered.contains(rpc.as_str()) || (cfg!(feature = "grpc-development") && rpc != "Estimate") {
            // development build: write RPCs are tested by typed_api_writes_on_private_daemons
            continue;
        }
        let reply = call(&channel, &token, rpc, &json!({})).await;
        if rpc == "Estimate" {
            // implemented, not compared: it would ask the live File Daemon
            assert!(!matches!(&reply, Err(s) if s.code() == Code::Unimplemented), "Estimate: {reply:?}");
            continue;
        }
        match reply {
            Err(status) if status.code() == Code::Unimplemented => unimplemented.push(rpc.clone()),
            other => failures.push(format!("{rpc}: neither tested nor UNIMPLEMENTED: {other:?}")),
        }
    }
    if !cfg!(feature = "grpc-development") {
        assert!(unimplemented.contains(&"Run".to_string()) && unimplemented.contains(&"Restore".to_string()));
    }

    // --- existing Director service unchanged ------------------------------
    let mut director_client = DirectorClient::new(channel.clone());
    let mut req = Request::new(CommandRequest { command_name: "version".into(), ..Default::default() });
    req.metadata_mut().insert(SESSION_HEADER, token.clone());
    let version = director_client.execute_command(req).await.unwrap().into_inner();
    assert!(version.success && version.text_output.contains("Version"), "{version:?}");
    let mut req = Request::new(CommandRequest {
        command_name: "list pools".into(), response_format: ResponseFormat::Json as i32, ..Default::default()
    });
    req.metadata_mut().insert(SESSION_HEADER, token.clone());
    let pools = director_client.execute_command(req).await.unwrap().into_inner();
    let grpc_json: Value = serde_json::from_str(&pools.text_output).unwrap();
    assert_eq!(grpc_json, console_json(director.port, "*UserAgent*", PASSWORD, "list pools"));
    let mut req = Request::new(ListCommandsRequest {});
    req.metadata_mut().insert(SESSION_HEADER, token.clone());
    let commands = director_client.list_commands(req).await.unwrap().into_inner();
    assert!(commands.commands.iter().any(|c| c.name == "list"));

    println!("typed API: {} cases, {} RPCs implemented and compared, {} UNIMPLEMENTED (S-35)",
             cases.len(), covered.len(), unimplemented.len());
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

// ---------------------------------------------------------------------------
// Development build (S-35): write and restore RPCs against PRIVATE daemons
// ---------------------------------------------------------------------------

/// Director with its own catalog (Bareos DDL), a private bareos-fd and
/// bareos-sd (own config, ports, storage and working directories under a
/// short directory: SD device paths are limited) and a file device.
/// Ports: TYPED_API_DEV_PORT (Director, default 39240), +1 FD, +2 SD,
/// +3 gRPC. Never the live daemons.
#[cfg(feature = "grpc-development")]
fn start_private_director(base: &Path) -> Director {
    let root = Path::new(ROOT);
    let binary = root.join("target/debug/odir");
    assert!(binary.exists(), "build first: cargo build -p odir --features grpc-development");
    let port: u16 = std::env::var("TYPED_API_DEV_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(39240);
    let (fd_port, sd_port, grpc_port) = (port + 1, port + 2, port + 3);
    let _ = std::fs::remove_dir_all(base);
    let dir = |name: &str| { let p = base.join(name); std::fs::create_dir_all(&p).unwrap(); p };
    let (data, store, work) = (dir("data"), dir("storage"), dir("dirwork"));
    let (fd_work, sd_work) = (dir("fd-work"), dir("sd-work"));
    for i in 0..3 {
        std::fs::write(data.join(format!("file{i}.txt")), format!("content {i}\n").repeat(i + 1)).unwrap();
    }
    std::fs::write(dir("fd").join("bareos-fd.conf"), format!(
        "Client {{ Name = tfd; FD Port = {fd_port}; FD Address = 127.0.0.1; Working Directory = {}; }}\n\
         Director {{ Name = bareos-dir; Password = \"tfd-pw\" }}\n\
         Messages {{ Name = Standard; Director = bareos-dir = all, !skipped, !restored }}\n", fd_work.display())).unwrap();
    std::fs::write(dir("sd").join("bareos-sd.conf"), format!(
        "Storage {{ Name = tsd; SD Port = {sd_port}; SD Address = 127.0.0.1; Working Directory = {}; }}\n\
         Director {{ Name = bareos-dir; Password = \"tsd-pw\" }}\n\
         Device {{ Name = FileStorage; Media Type = File; Archive Device = {}; LabelMedia = yes; Random Access = yes;\n\
           AutomaticMount = yes; RemovableMedia = no; AlwaysOpen = no; }}\n\
         Messages {{ Name = Standard; Director = bareos-dir = all }}\n", sd_work.display(), store.display())).unwrap();

    let db = format!("typed_api_dev_{}", std::process::id());
    psql(&format!("DROP DATABASE IF EXISTS \"{db}\" WITH (FORCE)"));
    let created = psql(&format!("CREATE DATABASE \"{db}\" ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0"));
    assert!(created.status.success(), "createdb: {}", String::from_utf8_lossy(&created.stderr));
    let ddl = Command::new("psql").args(["-q", "-d", &db, "-v", "ON_ERROR_STOP=1", "-f",
        "/usr/lib/bareos/scripts/ddl/creates/postgresql.sql"]).output().unwrap();
    assert!(ddl.status.success(), "DDL: {}", String::from_utf8_lossy(&ddl.stderr));

    let conf = base.join("dir/bareos-dir.d");
    let user = std::env::var("USER").unwrap_or_else(|_| "bareos".into());
    let files = [
        ("director/bareos-dir.conf", format!("Director {{\n  Name = bareos-dir\n  Password = \"{PASSWORD}\"\n  Messages = Daemon\n\
            Working Directory = {}\n  DirPort = {port}\n\
            Maximum Concurrent Jobs = 10\n  QueryFile = \"/usr/lib/bareos/scripts/query.sql\"\n}}\n", work.display())),
        ("catalog/MyCatalog.conf", format!("Catalog {{\n  Name = MyCatalog\n  dbname = \"{db}\"\n  dbuser = \"{user}\"\n  dbpassword = \"\"\n}}\n")),
        ("messages/Daemon.conf", format!("Messages {{\n  Name = Daemon\n  append = \"{}/daemon.log\" = all, !skipped, !saved, !audit\n}}\n", work.display())),
        ("messages/Standard.conf", format!("Messages {{\n  Name = Standard\n  append = \"{}/job.log\" = all, !skipped, !saved, !audit\n\
            catalog = all, !skipped, !saved, !audit\n}}\n", work.display())),
        ("client/tfd.conf", format!("Client {{\n  Name = tfd\n  Address = 127.0.0.1\n  FD Port = {fd_port}\n  Password = \"tfd-pw\"\n}}\n")),
        ("storage/tsd.conf", format!("Storage {{\n  Name = tsd\n  Address = 127.0.0.1\n  SD Port = {sd_port}\n  Password = \"tsd-pw\"\n\
            Device = FileStorage\n  Media Type = File\n}}\n")),
        ("pool/TA.conf", "Pool {\n  Name = TA\n  Pool Type = Backup\n  Recycle = yes\n  AutoPrune = no\n  Volume Retention = 30 days\n}\n".into()),
        ("pool/Scratch.conf", "Pool {\n  Name = Scratch\n  Pool Type = Scratch\n}\n".into()),
        ("fileset/TSet.conf", format!("FileSet {{\n  Name = TSet\n  Include {{\n    Options {{\n      Signature = MD5\n    }}\n    File = {}\n  }}\n}}\n",
            data.display())),
        ("job/Bk.conf", "Job {\n  Name = Bk\n  Type = Backup\n  Level = Full\n  Client = tfd\n  FileSet = TSet\n  Storage = tsd\n\
            Pool = TA\n  Messages = Standard\n}\n".into()),
        ("job/RestoreFiles.conf", format!("Job {{\n  Name = RestoreFiles\n  Type = Restore\n  Client = tfd\n  FileSet = TSet\n  Storage = tsd\n\
            Pool = TA\n  Messages = Standard\n  Where = {}/default-restore\n}}\n", base.display())),
    ];
    for (rel, text) in files {
        let path = conf.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    let spawn = |cmd: &[&str], log: &str| {
        let log = std::fs::File::create(base.join(log)).unwrap();
        let mut c = Command::new(cmd[0]);
        c.args(&cmd[1..]).env("LC_ALL", "C").env("LANG", "C").env("TZ", "UTC")
            .stdout(Stdio::from(log.try_clone().unwrap())).stderr(Stdio::from(log));
        c.spawn().unwrap_or_else(|e| panic!("start {}: {e}", cmd[0]))
    };
    let fd_conf = base.join("fd/bareos-fd.conf");
    let sd_conf = base.join("sd/bareos-sd.conf");
    let fd = spawn(&["/usr/sbin/bareos-fd", "-f", "-c", fd_conf.to_str().unwrap()], "fd.out");
    let sd = spawn(&["/usr/sbin/bareos-sd", "-f", "-c", sd_conf.to_str().unwrap()], "sd.out");
    let child = spawn(&[binary.to_str().unwrap(), "-f", "--grpc-port", &grpc_port.to_string(), "-c", conf.to_str().unwrap()], "dir.out");
    let director = Director { child, db, cert: work.join("onesimus-grpc.crt"), work: base.to_path_buf(), port, grpc_port,
                              daemons: vec![fd, sd] };
    assert!(wait_port(fd_port, 20) && wait_port(sd_port, 20), "private daemons did not start");
    assert!(wait_port(port, 30) && wait_port(grpc_port, 30), "Director did not open its ports");
    director
}

/// Call a write RPC that must succeed; returns the ActionResponse as JSON.
#[cfg(feature = "grpc-development")]
async fn act(channel: &Channel, token: &MetadataValue<tonic::metadata::Ascii>, rpc: &str, request: Value) -> Value {
    let reply = call(channel, token, rpc, &request).await.unwrap_or_else(|e| panic!("{rpc} {request}: {e:?}"));
    assert_eq!(reply["success"], json!(true), "{rpc} {request}: {reply:#}");
    reply
}

#[cfg(feature = "grpc-development")]
async fn wait_job(channel: &Channel, token: &MetadataValue<tonic::metadata::Ascii>, jobid: u64) -> Value {
    act(channel, token, "Wait", json!({"jobid": jobid})).await;
    let job = call(channel, token, "GetJob", &json!({"jobid": jobid, "llist": true})).await.unwrap();
    assert_eq!(job["jobs"][0]["jobstatus"], json!(84), "job {jobid} not terminated OK: {job:#}");  // 'T'
    job
}

#[cfg(feature = "grpc-development")]
fn first_jobid(reply: &Value) -> u64 {
    reply["jobids"][0].as_u64().unwrap_or_else(|| panic!("no jobid: {reply:#}"))
}

#[cfg(feature = "grpc-development")]
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs PostgreSQL, /usr/sbin/bareos-fd and bareos-sd (started privately) and the built binaries"]
async fn typed_api_writes_on_private_daemons() {
    let base = PathBuf::from(std::env::var("TYPED_API_DEV_DIR").unwrap_or_else(|_| "/tmp/ta39240".into()));
    // restore targets (e.g. the agent's scratchpad); default under the base directory
    let restore_dir = PathBuf::from(std::env::var("TYPED_API_RESTORE_DIR")
        .unwrap_or_else(|_| base.join("restored").display().to_string()));
    let _ = std::fs::remove_dir_all(&restore_dir);
    let director = start_private_director(&base);
    let data = base.join("data");
    let channel = channel(&director).await;
    let token = login(&channel, "*UserAgent*", PASSWORD).await;

    // label a volume on the file device; typed volume record afterwards
    let label = act(&channel, &token, "Label", json!({"storage": "tsd", "volume": "TA-0001", "pool": "TA"})).await;
    assert_eq!(label["volumes"][0]["volumename"], json!("TA-0001"), "{label:#}");
    assert_eq!(label["command"], json!("label storage=tsd volume=TA-0001 pool=TA yes"));

    // backup via Run, typed job record of the queued job
    let run = act(&channel, &token, "Run", json!({"job": "Bk", "level": "Full"})).await;
    let backup = first_jobid(&run);
    assert_eq!(run["jobs"][0]["jobid"], json!(backup));
    let job = wait_job(&channel, &token, backup).await;
    assert_eq!(job["jobs"][0]["jobfiles"], json!(4), "{job:#}");  // data directory + 3 files

    // restore via the Restore RPC: all files
    let target_a = restore_dir.join("a");
    let restore = act(&channel, &token, "Restore", json!({"client": "tfd", "jobid": [backup], "all": true,
        "where": target_a.display().to_string()})).await;
    wait_job(&channel, &token, first_jobid(&restore)).await;
    for i in 0..3 {
        let restored = target_a.join(data.strip_prefix("/").unwrap()).join(format!("file{i}.txt"));
        assert_eq!(std::fs::read_to_string(&restored).unwrap_or_default(), format!("content {i}\n").repeat(i + 1),
                   "{}", restored.display());
    }

    // interactive restore: select one file in the tree
    let target_b = restore_dir.join("b");
    let session = call(&channel, &token, "StartRestoreSession", &json!({"client": "tfd", "jobid": [backup],
        "where": target_b.display().to_string()})).await.expect("StartRestoreSession");
    let id = session["session_id"].as_str().unwrap().to_string();
    assert_eq!(session["files"], json!(4), "{session:#}");
    let s = |extra: Value| { let mut v = json!({"session_id": id}); v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone()); v };
    let cd = call(&channel, &token, "RestoreTreeCd", &s(json!({"path": data.display().to_string()}))).await.unwrap();
    assert_eq!(cd["cwd"], json!(format!("{}/", data.display())), "{cd:#}");
    let ls = call(&channel, &token, "RestoreTreeLs", &s(json!({}))).await.unwrap();
    let names: Vec<&str> = ls["entries"].as_array().unwrap().iter().filter_map(|e| e["name"].as_str()).collect();
    assert_eq!(names, ["file0.txt", "file1.txt", "file2.txt"], "{ls:#}");
    let mark = call(&channel, &token, "RestoreTreeMark", &s(json!({"path": "file1.txt"}))).await.unwrap();
    assert_eq!(mark["marked"], json!(1), "{mark:#}");
    let count = call(&channel, &token, "RestoreTreeCount", &s(json!({}))).await.unwrap();
    assert_eq!(count["marked"], json!(1), "{count:#}");
    let lsmark = call(&channel, &token, "RestoreTreeLsMark", &s(json!({}))).await.unwrap();
    assert!(lsmark["entries"].as_array().unwrap().iter().any(|e| e["name"] == "file1.txt" && e["marked"] == true), "{lsmark:#}");
    let find = call(&channel, &token, "RestoreTreeFind", &s(json!({"path": "file2*"}))).await.unwrap();
    assert!(find["text"].as_str().unwrap().contains("file2.txt"), "{find:#}");
    let pwd = call(&channel, &token, "RestoreTreePwd", &s(json!({}))).await.unwrap();
    assert_eq!(pwd["cwd"], cd["cwd"]);
    call(&channel, &token, "RestoreTreeEstimate", &s(json!({}))).await.unwrap();
    let help = call(&channel, &token, "RestoreTreeHelp", &s(json!({}))).await.unwrap();
    assert!(help["text"].as_str().unwrap().contains("markdir"), "{help:#}");
    let done = act(&channel, &token, "RestoreTreeDone", s(json!({}))).await;
    wait_job(&channel, &token, first_jobid(&done)).await;
    let restored_b = target_b.join(data.strip_prefix("/").unwrap());
    assert!(restored_b.join("file1.txt").exists() && !restored_b.join("file0.txt").exists(), "{}", restored_b.display());
    let gone = call(&channel, &token, "RestoreTreePwd", &s(json!({}))).await.unwrap_err();
    assert_eq!(gone.code(), Code::NotFound);

    // a second session, aborted: no job
    let session = call(&channel, &token, "StartRestoreSession", &json!({"client": "tfd", "jobid": [backup]})).await.unwrap();
    let abort = act(&channel, &token, "RestoreTreeAbort", json!({"session_id": session["session_id"]})).await;
    assert!(abort.get("jobids").is_none() && abort["text"].as_str().unwrap().contains("Restore not done"), "{abort:#}");

    // storage commands on the file device
    for rpc in ["Unmount", "Mount", "Release"] {
        act(&channel, &token, rpc, json!({"storage": "tsd"})).await;
    }
    // volume update, purge (confirm), truncate, delete
    let update = act(&channel, &token, "UpdateVolume", json!({"volume": "TA-0001", "volstatus": "Used"})).await;
    assert_eq!(update["volumes"][0]["volstatus"], json!(3), "{update:#}");  // Used
    let unconfirmed = call(&channel, &token, "PurgeVolume", &json!({"volume": ["TA-0001"]})).await.unwrap_err();
    assert_eq!(unconfirmed.code(), Code::InvalidArgument);
    let purge = act(&channel, &token, "PurgeVolume", json!({"volume": ["TA-0001"], "confirm": true})).await;
    assert_eq!(purge["volumes"][0]["volstatus"], json!(5), "{purge:#}");  // Purged
    act(&channel, &token, "Truncate", json!({"volume": "TA-0001", "storage": "tsd", "pool": "TA", "confirm": true})).await;
    act(&channel, &token, "DeleteVolume", json!({"volume": "TA-0001", "confirm": true})).await;
    let deleted = call(&channel, &token, "GetVolume", &json!({"volume": "TA-0001"})).await;
    assert!(deleted.is_err() || deleted.as_ref().unwrap()["volume"].get("volumename").is_none(), "{deleted:?}");

    // cancel a job that waits for its start time
    let later = act(&channel, &token, "Run", json!({"job": "Bk", "when": "2099-01-01 00:00:00"})).await;
    let cancel = act(&channel, &token, "Cancel", json!({"jobid": first_jobid(&later)})).await;
    assert!(cancel["text"].as_str().unwrap().contains("cancel"), "{cancel:#}");

    // catalog and resource administration
    act(&channel, &token, "DeleteJob", json!({"jobid": [backup], "confirm": true})).await;
    act(&channel, &token, "Disable", json!({"job": "Bk"})).await;
    act(&channel, &token, "Enable", json!({"job": "Bk"})).await;
    act(&channel, &token, "UpdatePool", json!({"pool": "TA"})).await;
    act(&channel, &token, "PruneJobs", json!({"client": "tfd", "confirm": true})).await;
    act(&channel, &token, "ConfigureAdd", json!({"resource_type": 1, "name": "typed-api-client",
        "directives": {"address": "127.0.0.2", "password": "x"}})).await;
    act(&channel, &token, "Reload", json!({})).await;
    let sql = call(&channel, &token, "DotSql", &json!({"query": "SELECT count(*) AS n FROM Job"})).await.unwrap();
    assert!(sql["query"][0]["extra_fields"]["n"].is_string(), "{sql:#}");
    let sqlquery = call(&channel, &token, "SqlQuery", &json!({"query": "SELECT 42 AS answer"})).await.unwrap();
    assert!(sqlquery["text"].as_str().unwrap().contains("42"), "{sqlquery:#}");
    let not_applicable = call(&channel, &token, "Gui", &json!({"on": true})).await.unwrap_err();
    assert_eq!(not_applicable.code(), Code::FailedPrecondition);

    // logout
    act(&channel, &token, "Quit", json!({})).await;
    let after = call(&channel, &token, "GetVersion", &json!({})).await.unwrap_err();
    assert_eq!(after.code(), Code::Unauthenticated);
}
