//! The host side of the PostgreSQL broker import
//! (docs/plans/POSTGRESQL-PHASE2.md, D).
//!
//! The guest asks; ofd decides. Before any request reaches the broker
//! process it is checked here: databases only from the allowed set (from
//! `databases=` / `exclude=` of the plugin line, never templates, never
//! databases that refuse connections, never the Bareos catalog `bareos`),
//! globals only with `globals=yes` (default), reads only on handles this
//! port opened and at most one chunk. The info answer the guest sees lists
//! only the allowed databases.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

use onesimus_plugins::wasm::{BrokerFunction, BrokerPort, PostgresqlFunction, POSTGRESQL_READ};
use onesimus_proto_rs::onesimus::filed::broker::postgresql::v1 as pb;
use prost::Message;

/// The Bareos catalog database: backed up by BackupCatalog, never here.
pub const CATALOG_DATABASE: &str = "bareos";
const MAX_FRAME: usize = 2 * 1024 * 1024;

/// `*` matches any run of characters; everything else literally.
fn glob(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || !name[first.len()..].ends_with(last) || name.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for middle in &parts[1..parts.len() - 1] {
        match rest.find(middle) {
            Some(i) => rest = &rest[i + middle.len()..],
            None => return false,
        }
    }
    true
}

fn list(value: Option<&String>) -> Vec<String> {
    value.map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

/// The databases a plugin line may touch, from the server's list.
pub fn allowed_databases(info: &pb::InfoResponse, options: &BTreeMap<String, String>) -> Result<BTreeSet<String>, String> {
    let wanted = list(options.get("databases"));
    let wanted: Vec<String> = if wanted.is_empty() { vec!["*".into()] } else { wanted };
    let exclude = list(options.get("exclude"));
    // The Director names its catalog (catalog_database=, added by odir);
    // "bareos" is excluded in any case.
    let catalog = options.get("catalog_database").map(String::as_str).unwrap_or(CATALOG_DATABASE);
    let usable = |d: &pb::Database| d.allow_connections && !d.is_template && d.name != CATALOG_DATABASE && d.name != catalog;
    let mut allowed = BTreeSet::new();
    for pattern in &wanted {
        let matches: Vec<&pb::Database> = info.databases.iter().filter(|d| glob(pattern, &d.name)).collect();
        if !pattern.contains('*') && matches.is_empty() {
            return Err(format!("database \"{pattern}\" does not exist"));
        }
        if !pattern.contains('*') && matches.iter().any(|d| !usable(d)) {
            return Err(format!("database \"{pattern}\" cannot be backed up here (template, no connections or the catalog)"));
        }
        for d in matches.into_iter().filter(|d| usable(d)) {
            if !exclude.iter().any(|e| glob(e, &d.name)) {
                allowed.insert(d.name.clone());
            }
        }
    }
    Ok(allowed)
}

/// The checked port over a broker connection (the broker's socket).
pub struct PgPort<T: Read + Write + Send> {
    transport: T,
    allowed: BTreeSet<String>,
    globals: bool,
    open: BTreeSet<u32>,
    chunk: u32,
}

fn exchange<T: Read + Write>(t: &mut T, request: pb::broker_request::Request) -> Result<pb::broker_response::Response, String> {
    let bytes = pb::BrokerRequest { request: Some(request) }.encode_to_vec();
    t.write_all(&(bytes.len() as u32).to_be_bytes()).and_then(|()| t.write_all(&bytes)).and_then(|()| t.flush())
        .map_err(|e| format!("broker: {e}"))?;
    let mut len = [0u8; 4];
    t.read_exact(&mut len).map_err(|e| format!("broker: {e}"))?;
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err("broker: frame too large".into());
    }
    let mut buf = vec![0u8; n];
    t.read_exact(&mut buf).map_err(|e| format!("broker: {e}"))?;
    match pb::BrokerResponse::decode(buf.as_slice()).map_err(|e| format!("broker: {e}"))?.response {
        Some(pb::broker_response::Response::Error(e)) => Err(e.message),
        Some(r) => Ok(r),
        None => Err("broker: empty response".into()),
    }
}

impl<T: Read + Write + Send> PgPort<T> {
    /// Asks the broker for the server's databases and fixes the allowed set.
    pub fn new(mut transport: T, options: &BTreeMap<String, String>, chunk: u32) -> Result<Self, String> {
        let pb::broker_response::Response::Info(info) = exchange(&mut transport, pb::broker_request::Request::Info(pb::InfoRequest {}))? else {
            return Err("broker: unexpected answer to info".into());
        };
        let allowed = allowed_databases(&info, options)?;
        let globals = options.get("globals").map(|v| !matches!(v.as_str(), "no" | "false" | "0")).unwrap_or(true);
        Ok(Self { transport, allowed, globals, open: BTreeSet::new(), chunk })
    }

    pub fn allowed(&self) -> &BTreeSet<String> {
        &self.allowed
    }

    fn permit(&self, database: &str) -> Result<(), String> {
        if self.allowed.contains(database) {
            Ok(())
        } else {
            Err(format!("database \"{database}\" is not allowed for this plugin line"))
        }
    }

    fn checked(&mut self, function: BrokerFunction, request: &[u8]) -> Result<Vec<u8>, String> {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let bad = |e: prost::DecodeError| format!("bad request: {e}");
        match function {
            BrokerFunction::Postgresql(PostgresqlFunction::Info) => {
                pb::InfoRequest::decode(request).map_err(bad)?;
                let A::Info(mut info) = exchange(&mut self.transport, Q::Info(pb::InfoRequest {}))? else {
                    return Err("broker: unexpected answer".into());
                };
                info.databases.retain(|d| self.allowed.contains(&d.name));
                Ok(info.encode_to_vec())
            }
            BrokerFunction::Postgresql(PostgresqlFunction::DatabaseInfo) => {
                let r = pb::DatabaseInfoRequest::decode(request).map_err(bad)?;
                self.permit(&r.database)?;
                match exchange(&mut self.transport, Q::DatabaseInfo(r))? {
                    A::DatabaseInfo(i) => Ok(i.encode_to_vec()),
                    _ => Err("broker: unexpected answer".into()),
                }
            }
            BrokerFunction::Postgresql(PostgresqlFunction::DumpOpen) => {
                let r = pb::DumpOpenRequest::decode(request).map_err(bad)?;
                match &r.what {
                    Some(pb::dump_open_request::What::Database(db)) => self.permit(db)?,
                    Some(pb::dump_open_request::What::Globals(_)) if self.globals => {}
                    Some(pb::dump_open_request::What::Globals(_)) => return Err("globals are off for this plugin line".into()),
                    None => return Err("dump of what?".into()),
                }
                match exchange(&mut self.transport, Q::DumpOpen(r))? {
                    A::DumpOpen(o) => {
                        self.open.insert(o.handle);
                        Ok(o.encode_to_vec())
                    }
                    _ => Err("broker: unexpected answer".into()),
                }
            }
            BrokerFunction::Postgresql(PostgresqlFunction::DumpRead) => {
                let mut r = pb::DumpReadRequest::decode(request).map_err(bad)?;
                if !self.open.contains(&r.handle) {
                    return Err("unknown dump handle".into());
                }
                r.max_bytes = r.max_bytes.clamp(1, self.chunk);
                match exchange(&mut self.transport, Q::DumpRead(r))? {
                    A::DumpRead(d) if d.data.len() <= self.chunk as usize => Ok(d.encode_to_vec()),
                    _ => Err("broker: unexpected answer".into()),
                }
            }
            BrokerFunction::Postgresql(PostgresqlFunction::DumpClose) => {
                let r = pb::DumpCloseRequest::decode(request).map_err(bad)?;
                if !self.open.remove(&r.handle) {
                    return Err("unknown dump handle".into());
                }
                match exchange(&mut self.transport, Q::DumpClose(r))? {
                    A::DumpClose(c) => Ok(c.encode_to_vec()),
                    _ => Err("broker: unexpected answer".into()),
                }
            }
            BrokerFunction::Openldap(_) => Err("not a function of the PostgreSQL broker".into()),
        }
    }
}

impl<T: Read + Write + Send> BrokerPort for PgPort<T> {
    fn capability(&self) -> &'static str {
        POSTGRESQL_READ
    }
    fn call(&mut self, function: BrokerFunction, request: &[u8]) -> Result<Vec<u8>, String> {
        self.checked(function, request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A broker stand-in: records requests, plays back responses.
    #[derive(Default)]
    struct Fake {
        responses: VecDeque<Vec<u8>>,
        out: Vec<u8>,
        pending: Vec<u8>,
        seen: Vec<pb::broker_request::Request>,
    }
    impl Fake {
        fn answer(&mut self, r: pb::broker_response::Response) -> &mut Self {
            let bytes = pb::BrokerResponse { response: Some(r) }.encode_to_vec();
            let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
            frame.extend(bytes);
            self.responses.push_back(frame);
            self
        }
    }
    impl Write for Fake {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.pending.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            let n = u32::from_be_bytes(self.pending[..4].try_into().unwrap()) as usize;
            let req = pb::BrokerRequest::decode(&self.pending[4..4 + n]).unwrap();
            self.seen.push(req.request.unwrap());
            self.pending.clear();
            if self.out.is_empty() {
                self.out = self.responses.pop_front().unwrap_or_default();
            }
            Ok(())
        }
    }
    impl Read for Fake {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            let n = b.len().min(self.out.len());
            b[..n].copy_from_slice(&self.out[..n]);
            self.out.drain(..n);
            Ok(n)
        }
    }

    fn db(name: &str, allow: bool, template: bool) -> pb::Database {
        pb::Database { name: name.into(), allow_connections: allow, is_template: template, ..Default::default() }
    }
    fn info() -> pb::InfoResponse {
        pb::InfoResponse {
            server_version: "17".into(),
            major: 17,
            databases: vec![db("EleLa", true, false), db("bareos", true, false), db("multi-gpt", true, false),
                db("multigpt", true, false), db("postgres", true, false), db("template0", false, true),
                db("template1", true, true), db("test_multigpt_core", true, false)],
        }
    }
    fn opts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn the_allowed_set_follows_the_line_and_never_includes_catalog_or_templates() {
        let all = allowed_databases(&info(), &opts(&[])).unwrap();
        assert_eq!(all.into_iter().collect::<Vec<_>>(), ["EleLa", "multi-gpt", "multigpt", "postgres", "test_multigpt_core"]);
        let some = allowed_databases(&info(), &opts(&[("databases", "*gpt*"), ("exclude", "test_*")])).unwrap();
        assert_eq!(some.into_iter().collect::<Vec<_>>(), ["multi-gpt", "multigpt"]);
        assert!(allowed_databases(&info(), &opts(&[("databases", "missing")])).unwrap_err().contains("does not exist"));
        assert!(allowed_databases(&info(), &opts(&[("databases", "bareos")])).is_err(), "catalog by name");
        assert!(allowed_databases(&info(), &opts(&[("databases", "template1")])).is_err());
        assert!(glob("a*b*c", "aXbYc") && !glob("a*b", "ab_") && glob("*", "") && !glob("x", "y"));
        // The Director's catalog under another name is excluded too.
        let named = allowed_databases(&info(), &opts(&[("catalog_database", "multigpt")])).unwrap();
        assert!(!named.contains("multigpt") && !named.contains("bareos") && named.contains("EleLa"));
        assert!(allowed_databases(&info(), &opts(&[("catalog_database", "EleLa"), ("databases", "EleLa")])).is_err());
    }

    #[test]
    fn requests_outside_the_allowed_set_never_reach_the_broker() {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let mut fake = Fake::default();
        fake.answer(A::Info(info()))
            .answer(A::Info(info()))
            .answer(A::DumpOpen(pb::DumpOpenResponse { handle: 7 }))
            .answer(A::DumpRead(pb::DumpReadResponse { data: b"PGDMP".to_vec(), ..Default::default() }))
            .answer(A::DumpClose(pb::DumpCloseResponse {}));
        let mut port = PgPort::new(fake, &opts(&[("databases", "EleLa,postgres"), ("globals", "no")]), 1024).unwrap();

        // Info shows only the allowed databases.
        let shown = pb::InfoResponse::decode(port.call(BrokerFunction::Postgresql(PostgresqlFunction::Info), &[]).unwrap().as_slice()).unwrap();
        assert_eq!(shown.databases.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["EleLa", "postgres"]);

        // Refused here, before the broker.
        let open = |what| pb::DumpOpenRequest { what: Some(what) }.encode_to_vec();
        for (f, req) in [
            (BrokerFunction::Postgresql(PostgresqlFunction::DumpOpen), open(pb::dump_open_request::What::Database("bareos".into()))),
            (BrokerFunction::Postgresql(PostgresqlFunction::DumpOpen), open(pb::dump_open_request::What::Database("multigpt".into()))),
            (BrokerFunction::Postgresql(PostgresqlFunction::DumpOpen), open(pb::dump_open_request::What::Globals(pb::GlobalsOnly {}))),
            (BrokerFunction::Postgresql(PostgresqlFunction::DatabaseInfo), pb::DatabaseInfoRequest { database: "template1".into() }.encode_to_vec()),
            (BrokerFunction::Postgresql(PostgresqlFunction::DumpRead), pb::DumpReadRequest { handle: 7, max_bytes: 10 }.encode_to_vec()),
        ] {
            assert!(port.call(f, &req).is_err(), "{f:?}");
        }

        // Allowed: open, read (capped to the chunk), close.
        let o = pb::DumpOpenResponse::decode(port.call(BrokerFunction::Postgresql(PostgresqlFunction::DumpOpen),
            &open(pb::dump_open_request::What::Database("EleLa".into()))).unwrap().as_slice()).unwrap();
        assert_eq!(o.handle, 7);
        port.call(BrokerFunction::Postgresql(PostgresqlFunction::DumpRead), &pb::DumpReadRequest { handle: 7, max_bytes: 999_999 }.encode_to_vec()).unwrap();
        port.call(BrokerFunction::Postgresql(PostgresqlFunction::DumpClose), &pb::DumpCloseRequest { handle: 7 }.encode_to_vec()).unwrap();
        assert!(port.call(BrokerFunction::Postgresql(PostgresqlFunction::DumpClose), &pb::DumpCloseRequest { handle: 7 }.encode_to_vec()).is_err());

        let seen = &port.transport.seen;
        assert_eq!(seen.len(), 5, "only allowed requests went out: {seen:?}");
        assert!(matches!(&seen[3], Q::DumpRead(r) if r.max_bytes == 1024), "read capped to the chunk");
    }
}
