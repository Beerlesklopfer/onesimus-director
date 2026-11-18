//! Broker of the signed PostgreSQL plugin (docs/plans/POSTGRESQL-PHASE2.md, E).
//!
//! ofd starts it from its checked descriptor as the instance's user, with
//! the socket on fd 0/1, an empty environment and these arguments only:
//!
//! ```text
//! onesimus-pg-broker --socket-dir <dir> --port <port> --bin <postgresql bin dir>
//! ```
//!
//! It answers the requests of proto/postgresql_broker.proto, framed as a
//! 4-byte big-endian length and the message. It runs nothing but `psql` with
//! fixed SQL and `pg_dump -Fc` / `pg_dumpall --globals-only` with fixed
//! arguments from `--bin`, over the local socket. Database names reach the
//! client programs only as quoted `dbname='…'` connection strings, so a name
//! like `host=elsewhere` stays a name.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};

use onesimus_proto_rs::onesimus::filed::broker::postgresql::v1 as pb;
use prost::Message;

const MAX_FRAME: usize = 1024 * 1024;
const MAX_READ: u32 = 1024 * 1024;
const MAX_DUMPS: usize = 4;
/// Info queries never wait long for a lock or run long.
const QUERY_SETTINGS: &str = "SET lock_timeout = '10s'; SET statement_timeout = '120s';";
/// pg_dump waits at most this long for a table lock, then fails with a
/// message instead of blocking the backup without end.
const DUMP_LOCK_WAIT: &str = "--lock-wait-timeout=300s";

struct Args {
    socket_dir: PathBuf,
    port: u16,
    bin: PathBuf,
}

fn args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let (mut socket_dir, mut port, mut bin) = (None, None, None);
    while let Some(key) = it.next() {
        let value = it.next().ok_or_else(|| format!("{key} needs a value"))?;
        match key.as_str() {
            "--socket-dir" => socket_dir = Some(PathBuf::from(value)),
            "--port" => port = Some(value.parse().map_err(|_| "bad --port")?),
            "--bin" => bin = Some(PathBuf::from(value)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let args = Args {
        socket_dir: socket_dir.ok_or("--socket-dir missing")?,
        port: port.ok_or("--port missing")?,
        bin: bin.ok_or("--bin missing")?,
    };
    if !args.socket_dir.is_absolute() || !args.bin.is_absolute() {
        return Err("paths must be absolute".into());
    }
    Ok(args)
}

/// `dbname='…'` with `\` and `'` escaped (libpq connection string rules).
fn conninfo(database: &str) -> String {
    let escaped = database.replace('\\', "\\\\").replace('\'', "\\'");
    format!("dbname='{escaped}'")
}

struct Dump {
    child: Child,
    stdout: ChildStdout,
    stderr: Option<ChildStderr>,
}

struct Broker {
    args: Args,
    dumps: BTreeMap<u32, Dump>,
    next_handle: u32,
}

impl Broker {
    fn client(&self, program: &str) -> Command {
        let mut c = Command::new(self.args.bin.join(program));
        c.env_clear()
            .arg("-h").arg(&self.args.socket_dir)
            .arg("-p").arg(self.args.port.to_string())
            .arg("--no-password");
        c
    }

    /// psql with fixed SQL; the single result value (JSON) as text.
    fn query(&self, database: &str, sql: &str) -> Result<String, String> {
        let out = self.client("psql")
            .args(["-X", "-A", "-t", "-q", "-v", "ON_ERROR_STOP=1"])
            .arg("-d").arg(conninfo(database))
            .arg("-c").arg(QUERY_SETTINGS)
            .arg("-c").arg(sql)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("psql: {e}"))?;
        if !out.status.success() {
            return Err(format!("psql: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn info(&self) -> Result<pb::InfoResponse, String> {
        let json = self.query("postgres", "SELECT json_build_object(
            'version', current_setting('server_version'),
            'num', current_setting('server_version_num')::int,
            'dbs', (SELECT coalesce(json_agg(json_build_object(
                'name', d.datname, 'owner', pg_get_userbyid(d.datdba),
                'encoding', pg_encoding_to_char(d.encoding),
                'size', CASE WHEN d.datallowconn AND has_database_privilege(d.oid, 'CONNECT')
                             THEN pg_database_size(d.oid) ELSE 0 END,
                'allow', d.datallowconn, 'template', d.datistemplate) ORDER BY d.datname), '[]')
              FROM pg_database d))")?;
        let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| format!("info JSON: {e}"))?;
        let databases = v["dbs"].as_array().cloned().unwrap_or_default().iter().map(|d| pb::Database {
            name: d["name"].as_str().unwrap_or_default().to_string(),
            owner: d["owner"].as_str().unwrap_or_default().to_string(),
            encoding: d["encoding"].as_str().unwrap_or_default().to_string(),
            size_bytes: d["size"].as_u64().unwrap_or(0),
            allow_connections: d["allow"].as_bool().unwrap_or(false),
            is_template: d["template"].as_bool().unwrap_or(false),
        }).collect();
        Ok(pb::InfoResponse {
            server_version: v["version"].as_str().unwrap_or_default().to_string(),
            major: (v["num"].as_u64().unwrap_or(0) / 10000) as u32,
            databases,
        })
    }

    fn database_info(&self, database: &str) -> Result<pb::DatabaseInfoResponse, String> {
        let json = self.query(database, "SELECT json_build_object(
            'ext', (SELECT coalesce(json_agg(extname ORDER BY extname), '[]') FROM pg_extension),
            'schemas', (SELECT coalesce(json_agg(nspname ORDER BY nspname), '[]') FROM pg_namespace
                        WHERE nspname NOT LIKE 'pg\\_%' AND nspname <> 'information_schema'),
            'tables', (SELECT coalesce(json_agg(json_build_object(
                    'schema', n.nspname, 'name', c.relname,
                    'rows', greatest(c.reltuples, 0)::bigint,
                    -- From the catalog: pg_total_relation_size would lock the table.
                    'size', c.relpages::bigint * current_setting('block_size')::bigint)
                    ORDER BY n.nspname, c.relname), '[]')
                FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE c.relkind IN ('r', 'p', 'm') AND n.nspname NOT LIKE 'pg\\_%'
                  AND n.nspname <> 'information_schema'))")?;
        let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| format!("database JSON: {e}"))?;
        let strings = |key: &str| v[key].as_array().cloned().unwrap_or_default().iter()
            .filter_map(|s| s.as_str().map(str::to_string)).collect::<Vec<_>>();
        let tables = v["tables"].as_array().cloned().unwrap_or_default().iter().map(|t| pb::Table {
            schema: t["schema"].as_str().unwrap_or_default().to_string(),
            name: t["name"].as_str().unwrap_or_default().to_string(),
            estimated_rows: t["rows"].as_u64().unwrap_or(0),
            size_bytes: t["size"].as_u64().unwrap_or(0),
        }).collect();
        Ok(pb::DatabaseInfoResponse { database: database.to_string(), extensions: strings("ext"), schemas: strings("schemas"), tables })
    }

    fn dump_open(&mut self, what: Option<pb::dump_open_request::What>) -> Result<pb::DumpOpenResponse, String> {
        if self.dumps.len() >= MAX_DUMPS {
            return Err("too many open dumps".into());
        }
        let mut cmd = match what {
            Some(pb::dump_open_request::What::Database(db)) => {
                let mut c = self.client("pg_dump");
                c.arg("-Fc").arg(DUMP_LOCK_WAIT).arg("-d").arg(conninfo(&db));
                c
            }
            Some(pb::dump_open_request::What::Globals(_)) => {
                let mut c = self.client("pg_dumpall");
                c.arg("--globals-only").arg(DUMP_LOCK_WAIT).arg("-d").arg(conninfo("postgres"));
                c
            }
            None => return Err("dump of what?".into()),
        };
        let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
            .map_err(|e| format!("start: {e}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = child.stderr.take();
        let handle = self.next_handle;
        self.next_handle += 1;
        self.dumps.insert(handle, Dump { child, stdout, stderr });
        Ok(pb::DumpOpenResponse { handle })
    }

    fn dump_read(&mut self, handle: u32, max: u32) -> Result<pb::DumpReadResponse, String> {
        let dump = self.dumps.get_mut(&handle).ok_or("unknown handle")?;
        let mut buf = vec![0u8; max.clamp(1, MAX_READ) as usize];
        let mut filled = 0;
        while filled < buf.len() {
            match dump.stdout.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("read: {e}")),
            }
        }
        buf.truncate(filled);
        if filled > 0 {
            return Ok(pb::DumpReadResponse { data: buf, ..Default::default() });
        }
        // End of output: the program's result.
        let mut diag = String::new();
        if let Some(mut e) = dump.stderr.take() {
            let mut bytes = Vec::new();
            let _ = e.by_ref().take(64 * 1024).read_to_end(&mut bytes);
            diag = String::from_utf8_lossy(&bytes).trim().to_string();
        }
        let status = dump.child.wait().map_err(|e| format!("wait: {e}"))?;
        Ok(pb::DumpReadResponse { data: Vec::new(), eof: true, exit_ok: status.success(), diagnostic: diag })
    }

    fn dump_close(&mut self, handle: u32) -> Result<pb::DumpCloseResponse, String> {
        let mut dump = self.dumps.remove(&handle).ok_or("unknown handle")?;
        let _ = dump.child.kill();
        let _ = dump.child.wait();
        Ok(pb::DumpCloseResponse {})
    }

    fn handle(&mut self, request: pb::BrokerRequest) -> pb::BrokerResponse {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let result = match request.request {
            Some(Q::Info(_)) => self.info().map(A::Info),
            Some(Q::DatabaseInfo(r)) => self.database_info(&r.database).map(A::DatabaseInfo),
            Some(Q::DumpOpen(r)) => self.dump_open(r.what).map(A::DumpOpen),
            Some(Q::DumpRead(r)) => self.dump_read(r.handle, r.max_bytes).map(A::DumpRead),
            Some(Q::DumpClose(r)) => self.dump_close(r.handle).map(A::DumpClose),
            None => Err("empty request".into()),
        };
        pb::BrokerResponse {
            response: Some(result.unwrap_or_else(|message| A::Error(pb::BrokerError { message }))),
        }
    }
}

fn read_frame(input: &mut impl Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut buf = vec![0u8; n];
    input.read_exact(&mut buf)?;
    Ok(Some(buf))
}

fn write_frame(output: &mut impl Write, message: &[u8]) -> std::io::Result<()> {
    output.write_all(&(message.len() as u32).to_be_bytes())?;
    output.write_all(message)?;
    output.flush()
}

fn main() {
    let args = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("onesimus-pg-broker: {e}");
            std::process::exit(2);
        }
    };
    // SAFETY: getuid has no preconditions; a broker never runs as root.
    if unsafe { libc::getuid() } == 0 {
        eprintln!("onesimus-pg-broker: refusing to run as root");
        std::process::exit(2);
    }
    // SAFETY: fd 0 and 1 are the socket ofd passed; owned by this process.
    let (mut input, mut output) = unsafe { (std::fs::File::from_raw_fd(0), std::fs::File::from_raw_fd(1)) };
    let mut broker = Broker { args, dumps: BTreeMap::new(), next_handle: 1 };
    loop {
        let frame = match read_frame(&mut input) {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                eprintln!("onesimus-pg-broker: {e}");
                std::process::exit(1);
            }
        };
        let response = match pb::BrokerRequest::decode(frame.as_slice()) {
            Ok(request) => broker.handle(request),
            Err(e) => pb::BrokerResponse {
                response: Some(pb::broker_response::Response::Error(pb::BrokerError { message: format!("decode: {e}") })),
            },
        };
        if write_frame(&mut output, &response.encode_to_vec()).is_err() {
            break;
        }
    }
    for (_, mut dump) in std::mem::take(&mut broker.dumps) {
        let _ = dump.child.kill();
        let _ = dump.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_names_stay_names() {
        assert_eq!(conninfo("app"), "dbname='app'");
        assert_eq!(conninfo("host=evil port=1"), "dbname='host=evil port=1'");
        assert_eq!(conninfo("a'b\\c"), "dbname='a\\'b\\\\c'");
    }
}
