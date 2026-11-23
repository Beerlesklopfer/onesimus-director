// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The broker against a throwaway cluster of the test user (initdb, own
//! socket directory, no TCP). Skipped without PostgreSQL server binaries.
//! The socket directory lies directly under /tmp: a Unix socket path holds at
//! most 107 bytes, and a long TMPDIR would exceed it.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use onesimus_proto_rs::onesimus::filed::broker::postgresql::v1 as pb;
use prost::Message;

struct Cluster {
    root: PathBuf,
    sock: PathBuf,
    port: u16,
    bin: PathBuf,
}
impl Cluster {
    fn start(port: u16) -> Option<Self> {
        let bin = std::fs::read_dir("/usr/lib/postgresql").ok()?
            .filter_map(|e| e.ok().map(|e| e.path().join("bin")))
            .filter(|b| b.join("initdb").is_file()).max()?;
        let root = std::env::temp_dir().join(format!("pg-broker-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sock = PathBuf::from(format!("/tmp/pg-broker-sock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sock);
        std::fs::create_dir_all(&sock).unwrap();
        let c = Cluster { root, sock, port, bin };
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".into());
        let user = Command::new("id").arg("-un").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or(user);
        assert!(Command::new(c.bin.join("initdb")).arg("-D").arg(c.root.join("data"))
            .args(["-A", "peer", "-E", "UTF8", "--no-sync", "-U", &user]).output().unwrap().status.success());
        let opts = format!("-k {} -p {port} -c listen_addresses=''", c.sock.display());
        assert!(Command::new(c.bin.join("pg_ctl")).arg("-D").arg(c.root.join("data")).args(["-w", "-l"])
            .arg(c.root.join("log")).args(["-o", &opts, "start"]).output().unwrap().status.success());
        Some(c)
    }
    fn sql(&self, db: &str, sql: &str) {
        let out = Command::new(self.bin.join("psql")).args(["-X", "-q", "-v", "ON_ERROR_STOP=1", "-h"])
            .arg(&self.sock).args(["-p", &self.port.to_string(), "-d", db, "-c", sql]).output().unwrap();
        assert!(out.status.success(), "{sql}: {}", String::from_utf8_lossy(&out.stderr));
    }
}
impl Drop for Cluster {
    fn drop(&mut self) {
        let _ = Command::new(self.bin.join("pg_ctl")).arg("-D").arg(self.root.join("data")).args(["-m", "immediate", "stop"]).output();
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.sock);
    }
}

struct Broker(Child);
impl Broker {
    fn start(c: &Cluster) -> Self {
        Broker(Command::new(env!("CARGO_BIN_EXE_onesimus-pg-broker"))
            .env_clear()
            .arg("--socket-dir").arg(&c.sock).args(["--port", &c.port.to_string()])
            .arg("--bin").arg(&c.bin)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap())
    }
    fn call(&mut self, request: pb::broker_request::Request) -> pb::broker_response::Response {
        let bytes = pb::BrokerRequest { request: Some(request) }.encode_to_vec();
        let stdin = self.0.stdin.as_mut().unwrap();
        stdin.write_all(&(bytes.len() as u32).to_be_bytes()).unwrap();
        stdin.write_all(&bytes).unwrap();
        stdin.flush().unwrap();
        let stdout = self.0.stdout.as_mut().unwrap();
        let mut len = [0u8; 4];
        stdout.read_exact(&mut len).unwrap();
        let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
        stdout.read_exact(&mut buf).unwrap();
        pb::BrokerResponse::decode(buf.as_slice()).unwrap().response.unwrap()
    }
    fn dump(&mut self, what: pb::dump_open_request::What) -> (Vec<u8>, bool, String) {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let A::DumpOpen(open) = self.call(Q::DumpOpen(pb::DumpOpenRequest { what: Some(what) })) else { panic!("open") };
        let mut data = Vec::new();
        loop {
            match self.call(Q::DumpRead(pb::DumpReadRequest { handle: open.handle, max_bytes: 4096 })) {
                A::DumpRead(r) if r.eof => return (data, r.exit_ok, r.diagnostic),
                A::DumpRead(r) => data.extend(r.data),
                other => panic!("{other:?}"),
            }
        }
    }
}
impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn info_database_info_and_dumps() {
    use pb::broker_request::Request as Q;
    use pb::broker_response::Response as A;
    let Some(c) = Cluster::start(55433) else {
        eprintln!("no PostgreSQL server binaries; skipped");
        return;
    };
    c.sql("postgres", "CREATE DATABASE \"app-1\"");
    c.sql("postgres", "CREATE DATABASE \"evil host=/nonexistent\"");
    c.sql("postgres", "CREATE DATABASE legacy ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C' TEMPLATE template0");
    c.sql("app-1", "CREATE TABLE t (id int primary key, v text); INSERT INTO t SELECT g, md5(g::text) FROM generate_series(1, 5000) g; ANALYZE t");
    c.sql("postgres", "CREATE ROLE reader NOLOGIN");
    let mut b = Broker::start(&c);

    let A::Info(info) = b.call(Q::Info(pb::InfoRequest {})) else { panic!("info") };
    assert!(info.major >= 12, "{info:?}");
    let names: Vec<&str> = info.databases.iter().map(|d| d.name.as_str()).collect();
    for want in ["app-1", "evil host=/nonexistent", "legacy", "postgres", "template0"] {
        assert!(names.contains(&want), "{names:?}");
    }
    let legacy = info.databases.iter().find(|d| d.name == "legacy").unwrap();
    assert_eq!(legacy.encoding, "SQL_ASCII");
    assert!(info.databases.iter().find(|d| d.name == "template0").unwrap().is_template);

    let A::DatabaseInfo(di) = b.call(Q::DatabaseInfo(pb::DatabaseInfoRequest { database: "app-1".into() })) else { panic!() };
    let t = di.tables.iter().find(|t| t.name == "t").unwrap();
    assert_eq!((t.schema.as_str(), t.estimated_rows), ("public", 5000));

    // A dump of app-1 restores to the same rows.
    let (dump, ok, diag) = b.dump(pb::dump_open_request::What::Database("app-1".into()));
    assert!(ok, "{diag}");
    assert!(dump.starts_with(b"PGDMP"), "custom format");
    let file = c.root.join("app.dump");
    std::fs::write(&file, &dump).unwrap();
    c.sql("postgres", "CREATE DATABASE restored");
    let restore = Command::new(c.bin.join("pg_restore")).arg("-h").arg(&c.sock)
        .args(["-p", &c.port.to_string(), "-d", "restored"]).arg(&file).output().unwrap();
    assert!(restore.status.success(), "{}", String::from_utf8_lossy(&restore.stderr));
    let count = Command::new(c.bin.join("psql")).args(["-X", "-A", "-t", "-h"]).arg(&c.sock)
        .args(["-p", &c.port.to_string(), "-d", "restored", "-c", "SELECT count(*), sum(id) FROM t"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&count.stdout).trim(), "5000|12502500");

    // A name that looks like connection options is dumped as that database.
    let (dump, ok, diag) = b.dump(pb::dump_open_request::What::Database("evil host=/nonexistent".into()));
    assert!(ok && dump.starts_with(b"PGDMP"), "{diag}");

    // Globals include the role; a missing database fails with a diagnostic.
    let (globals, ok, _) = b.dump(pb::dump_open_request::What::Globals(pb::GlobalsOnly {}));
    assert!(ok && String::from_utf8_lossy(&globals).contains("CREATE ROLE reader"));
    let (_, ok, diag) = b.dump(pb::dump_open_request::What::Database("missing".into()));
    assert!(!ok && diag.contains("missing"), "{diag}");

    // Unknown handle and an empty request are errors, not crashes.
    assert!(matches!(b.call(Q::DumpRead(pb::DumpReadRequest { handle: 999, max_bytes: 10 })), A::Error(_)));
}
