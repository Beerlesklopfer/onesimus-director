//! Broker of the signed OpenLDAP plugin (docs/plans/OPENLDAP-PLUGIN.md).
//!
//! ofd starts it from its checked descriptor as the user slapd runs as
//! (never root), with the socket on fd 0/1, an empty environment and these
//! arguments only:
//!
//! ```text
//! onesimus-ldap-broker --sbin /usr/sbin (--config-dir <slapd.d> | --config-file <slapd.conf>) [--target <dir>]
//! ```
//!
//! It answers the requests of proto/openldap_broker.proto, framed as a
//! 4-byte big-endian length and the message. It runs nothing but
//! `slapd -VV`, `slapcat` and `slapadd` from `--sbin` with fixed arguments.
//! Without `--target` it only reads (info, dumps, configuration files by the
//! id of its own info list); with `--target` (an online restore into a new
//! directory the host created) it only loads, and only below that target.
//! No tool output reaches the host verbatim (see sanitize.rs).

mod config;
mod sanitize;

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread::JoinHandle;

use onesimus_proto_rs::onesimus::filed::broker::openldap::v1 as pb;
use prost::Message;

const MAX_FRAME: usize = 1024 * 1024;
const MAX_READ: u32 = 1024 * 1024;
const MAX_STREAMS: usize = 4;
/// Largest `slapcat -n 0` output info parses.
const MAX_CONFIG: u64 = 32 * 1024 * 1024;
/// stderr kept per tool run (only scanned, never passed on).
const MAX_STDERR: u64 = 256 * 1024;
/// Directories configuration files may come from (besides the directory of
/// the main slapd.conf). TLS files are never configuration files.
const CONFIG_PREFIXES: &[&str] = &["/etc/ldap", "/etc/openldap", "/usr/share/openldap", "/usr/local/etc/openldap"];
const DEFAULTS_FILE: &str = "/etc/default/slapd";

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigRef {
    Dir(PathBuf),
    File(PathBuf),
}

#[derive(Debug)]
struct Args {
    sbin: PathBuf,
    config: ConfigRef,
    target: Option<PathBuf>,
}

fn args_from(list: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut it = list;
    let (mut sbin, mut config, mut target) = (None, None, None);
    while let Some(key) = it.next() {
        let value = PathBuf::from(it.next().ok_or_else(|| format!("{key} needs a value"))?);
        if !config::is_plain_absolute(&value) {
            return Err(format!("{key}: path must be absolute and plain"));
        }
        match key.as_str() {
            "--sbin" => sbin = Some(value),
            "--config-dir" if config.is_none() => config = Some(ConfigRef::Dir(value)),
            "--config-file" if config.is_none() => config = Some(ConfigRef::File(value)),
            "--target" => target = Some(value),
            other => return Err(format!("unknown or repeated argument {other}")),
        }
    }
    Ok(Args { sbin: sbin.ok_or("--sbin missing")?, config: config.ok_or("--config-dir or --config-file missing")?, target })
}

/// A tool's stderr, drained by a thread so a chatty tool never blocks.
fn drain(stderr: Option<std::process::ChildStderr>) -> Option<JoinHandle<Vec<u8>>> {
    stderr.map(|e| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut e = e;
            let _ = e.by_ref().take(MAX_STDERR).read_to_end(&mut bytes);
            // Keep reading (and dropping) so the tool never blocks on a full pipe.
            let _ = std::io::copy(&mut e, &mut std::io::sink());
            bytes
        })
    })
}

enum Stream {
    Tool { child: Child, stdout: ChildStdout, stderr: Option<JoinHandle<Vec<u8>>>, tool: &'static str },
    File(std::fs::File),
}

struct Load {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

struct Broker {
    args: Args,
    streams: BTreeMap<u32, Stream>,
    load: Option<(u32, Load)>,
    next_handle: u32,
    /// The configuration files of the last info, by id.
    files: Vec<pb::ConfigFile>,
}

fn under(path: &Path, dir: &Path) -> bool {
    config::is_plain_absolute(path) && path.starts_with(dir) && path != dir
}

impl Broker {
    fn tool(&self, name: &str) -> Command {
        let mut c = Command::new(self.args.sbin.join(name));
        c.env_clear().stdin(Stdio::null());
        c
    }

    /// `-F <dir>` or `-f <file>` of a configuration.
    fn config_args(c: &mut Command, config: &ConfigRef) {
        match config {
            ConfigRef::Dir(d) => c.arg("-F").arg(d),
            ConfigRef::File(f) => c.arg("-f").arg(f),
        };
    }

    fn version(&self) -> String {
        let Ok(out) = self.tool("slapd").arg("-VV").stdout(Stdio::piped()).stderr(Stdio::piped()).output() else {
            return String::new();
        };
        let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
        // "@(#) $OpenLDAP: slapd 2.6.10+dfsg-1 (May 29 2025 23:41:48) $"
        text.lines()
            .find_map(|l| l.find("slapd ").map(|i| l[i..].split(" (").next().unwrap_or("").trim().to_string()))
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_control())
            .take(128)
            .collect()
    }

    fn config_file_entry(id: u32, path: &Path, main: bool) -> Result<pb::ConfigFile, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{}: not a regular file", path.display()));
        }
        Ok(pb::ConfigFile { id, path: path.display().to_string(), size: meta.len(), main })
    }

    /// Whether a configuration file may be read: below the fixed prefixes
    /// or the main file's directory (after resolving links), or the
    /// defaults file; never a TLS file.
    fn allowed_config_file(path: &Path, main_dir: Option<&Path>, tls: &[String]) -> Result<(), String> {
        let real = std::fs::canonicalize(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if tls.iter().any(|t| Path::new(t) == path || std::fs::canonicalize(t).ok().as_deref() == Some(real.as_path())) {
            return Err(format!("{}: a TLS file is no configuration file", path.display()));
        }
        let ok = |p: &Path| {
            p == Path::new(DEFAULTS_FILE)
                || CONFIG_PREFIXES.iter().any(|d| under(p, Path::new(d)))
                || main_dir.is_some_and(|d| under(p, d))
        };
        if ok(path) && ok(&real) {
            Ok(())
        } else {
            Err(format!("{}: outside the configuration directories", path.display()))
        }
    }

    fn info(&mut self) -> Result<pb::InfoResponse, String> {
        let (kind, parsed) = match self.args.config.clone() {
            ConfigRef::Dir(dir) => {
                let mut c = self.tool("slapcat");
                c.arg("-F").arg(&dir).args(["-n", "0", "-o", "ldif_wrap=no"]);
                let mut child = c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("slapcat: {e}"))?;
                let stderr = drain(child.stderr.take());
                let mut ldif = Vec::new();
                child.stdout.take().ok_or("no stdout")?.take(MAX_CONFIG).read_to_end(&mut ldif).map_err(|e| format!("slapcat: {e}"))?;
                let status = child.wait().ok();
                let err = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
                if !status.is_some_and(|s| s.success()) {
                    return Err(sanitize::diagnostic("slapcat -n 0", status, &err));
                }
                (pb::ConfigKind::SlapdD, config::parse_cn_config(&String::from_utf8_lossy(&ldif)))
            }
            ConfigRef::File(file) => {
                let mut read = |p: &Path| -> Result<String, String> {
                    let meta = std::fs::metadata(p).map_err(|e| format!("{}: {e}", p.display()))?;
                    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
                        return Err(format!("{}: not a regular file of at most 4 MiB", p.display()));
                    }
                    std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
                };
                (pb::ConfigKind::SlapdConf, config::parse_slapd_conf(&file, &mut read)?)
            }
        };
        let main_dir = match &self.args.config {
            ConfigRef::File(f) => f.parent().map(Path::to_path_buf),
            ConfigRef::Dir(_) => None,
        };
        let mut files = Vec::new();
        for (i, path) in parsed.files.iter().enumerate() {
            Self::allowed_config_file(path, main_dir.as_deref(), &parsed.tls_files)?;
            files.push(Self::config_file_entry(i as u32, path, i == 0)?);
        }
        if Path::new(DEFAULTS_FILE).is_file() {
            files.push(Self::config_file_entry(files.len() as u32, Path::new(DEFAULTS_FILE), false)?);
        }
        self.files = files.clone();
        let config_path = match &self.args.config {
            ConfigRef::Dir(d) | ConfigRef::File(d) => d.display().to_string(),
        };
        Ok(pb::InfoResponse {
            server_version: self.version(),
            kind: kind as i32,
            config_path,
            databases: parsed.databases.into_iter().map(|d| pb::Database {
                number: d.number,
                backend: d.backend,
                suffix: d.suffix,
                directory: d.directory,
                overlays: d.overlays,
                selected: false,
                skipped_reason: String::new(),
            }).collect(),
            config_files: files,
            tls_files: parsed.tls_files,
            secret_kinds: parsed.secret_kinds.into_iter().map(str::to_string).collect(),
            config_selected: false,
        })
    }

    fn reading(&self) -> Result<(), String> {
        if self.args.target.is_some() {
            return Err("this broker loads into a restore target; it does not read".into());
        }
        if self.streams.len() >= MAX_STREAMS {
            return Err("too many open streams".into());
        }
        Ok(())
    }

    fn add_stream(&mut self, stream: Stream) -> pb::DumpOpenResponse {
        let handle = self.next_handle;
        self.next_handle += 1;
        self.streams.insert(handle, stream);
        pb::DumpOpenResponse { handle }
    }

    fn dump_open(&mut self, what: Option<pb::dump_open_request::What>) -> Result<pb::DumpOpenResponse, String> {
        self.reading()?;
        let mut c = self.tool("slapcat");
        Self::config_args(&mut c, &self.args.config);
        match what {
            Some(pb::dump_open_request::What::Config(_)) => {
                if !matches!(self.args.config, ConfigRef::Dir(_)) {
                    return Err("slapd.conf has no cn=config to dump (-n 0)".into());
                }
                c.args(["-n", "0"]);
            }
            Some(pb::dump_open_request::What::Suffix(suffix)) => {
                if suffix.is_empty() || suffix.starts_with('-') || suffix.chars().any(char::is_control) {
                    return Err("invalid suffix".into());
                }
                c.arg("-b").arg(suffix);
            }
            None => return Err("dump of what?".into()),
        }
        c.args(["-o", "ldif_wrap=no"]);
        let mut child = c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| format!("slapcat: {e}"))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let stderr = drain(child.stderr.take());
        Ok(self.add_stream(Stream::Tool { child, stdout, stderr, tool: "slapcat" }))
    }

    fn config_file_open(&mut self, id: u32) -> Result<pb::DumpOpenResponse, String> {
        self.reading()?;
        if self.files.is_empty() {
            self.info()?;
        }
        let entry = self.files.iter().find(|f| f.id == id).ok_or("unknown configuration file id")?;
        let file = std::fs::File::open(&entry.path).map_err(|e| format!("{}: {e}", entry.path))?;
        Ok(self.add_stream(Stream::File(file)))
    }

    fn dump_read(&mut self, handle: u32, max: u32) -> Result<pb::DumpReadResponse, String> {
        let stream = self.streams.get_mut(&handle).ok_or("unknown handle")?;
        let mut buf = vec![0u8; max.clamp(1, MAX_READ) as usize];
        let mut filled = 0;
        {
            let reader: &mut dyn Read = match stream {
                Stream::Tool { stdout, .. } => stdout,
                Stream::File(f) => f,
            };
            while filled < buf.len() {
                match reader.read(&mut buf[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(format!("read: {e}")),
                }
            }
        }
        buf.truncate(filled);
        if filled > 0 {
            return Ok(pb::DumpReadResponse { data: buf, ..Default::default() });
        }
        match stream {
            Stream::File(_) => Ok(pb::DumpReadResponse { data: Vec::new(), eof: true, exit_ok: true, diagnostic: String::new() }),
            Stream::Tool { child, stderr, tool, .. } => {
                let status = child.wait().ok();
                let err = stderr.take().and_then(|h| h.join().ok()).unwrap_or_default();
                let ok = status.is_some_and(|s| s.success());
                let diagnostic = if ok { String::new() } else { sanitize::diagnostic(tool, status, &err) };
                Ok(pb::DumpReadResponse { data: Vec::new(), eof: true, exit_ok: ok, diagnostic })
            }
        }
    }

    fn dump_close(&mut self, handle: u32) -> Result<pb::DumpCloseResponse, String> {
        match self.streams.remove(&handle).ok_or("unknown handle")? {
            Stream::Tool { mut child, .. } => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Stream::File(_) => {}
        }
        Ok(pb::DumpCloseResponse {})
    }

    fn load_open(&mut self, r: pb::BrokerLoadOpen) -> Result<pb::LoadOpenResponse, String> {
        let Some(target) = self.args.target.clone() else {
            return Err("this broker only reads; loading needs a restore target".into());
        };
        if self.load.is_some() {
            return Err("a load is already open".into());
        }
        let against = match r.against {
            Some(pb::broker_load_open::Against::ConfigDir(d)) => ConfigRef::Dir(PathBuf::from(d)),
            Some(pb::broker_load_open::Against::ConfigFile(f)) => ConfigRef::File(PathBuf::from(f)),
            None => return Err("load against which configuration?".into()),
        };
        let (ConfigRef::Dir(p) | ConfigRef::File(p)) = &against;
        if !under(p, &target) {
            return Err("the configuration of a load must lie inside the restore target".into());
        }
        let mut c = self.tool("slapadd");
        Self::config_args(&mut c, &against);
        match r.what {
            Some(pb::broker_load_open::What::Config(_)) => {
                if !matches!(against, ConfigRef::Dir(_)) {
                    return Err("cn=config loads only into a slapd.d directory".into());
                }
                c.args(["-n", "0"]);
            }
            Some(pb::broker_load_open::What::Suffix(suffix)) => {
                if suffix.is_empty() || suffix.starts_with('-') || suffix.chars().any(char::is_control) {
                    return Err("invalid suffix".into());
                }
                c.arg("-b").arg(suffix).arg("-q");
            }
            None => return Err("load of what?".into()),
        }
        let mut child = c.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn()
            .map_err(|e| format!("slapadd: {e}"))?;
        let stdin = child.stdin.take();
        let stderr = drain(child.stderr.take());
        let handle = self.next_handle;
        self.next_handle += 1;
        self.load = Some((handle, Load { child, stdin, stderr }));
        Ok(pb::LoadOpenResponse { handle })
    }

    fn load_write(&mut self, handle: u32, data: &[u8]) -> Result<pb::LoadWriteResponse, String> {
        let (h, load) = self.load.as_mut().ok_or("no load is open")?;
        if *h != handle {
            return Err("unknown load handle".into());
        }
        let stdin = load.stdin.as_mut().ok_or("load input closed")?;
        // A failing slapadd closes its input: report its diagnostic at close.
        stdin.write_all(data).map_err(|_| "slapadd stopped reading; see the result at close".to_string())?;
        Ok(pb::LoadWriteResponse {})
    }

    fn load_close(&mut self, handle: u32, abort: bool) -> Result<pb::LoadCloseResponse, String> {
        if self.load.as_ref().map(|(h, _)| *h) != Some(handle) {
            return Err("unknown load handle".into());
        }
        let (_, mut load) = self.load.take().ok_or("no load is open")?;
        drop(load.stdin.take());
        if abort {
            let _ = load.child.kill();
            let _ = load.child.wait();
            return Ok(pb::LoadCloseResponse { ok: false, diagnostic: "aborted".into(), target: String::new() });
        }
        let status = load.child.wait().ok();
        let err = load.stderr.take().and_then(|h| h.join().ok()).unwrap_or_default();
        let ok = status.is_some_and(|s| s.success());
        let diagnostic = if ok { String::new() } else { sanitize::diagnostic("slapadd", status, &err) };
        Ok(pb::LoadCloseResponse { ok, diagnostic, target: String::new() })
    }

    fn handle(&mut self, request: pb::BrokerRequest) -> pb::BrokerResponse {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let result = match request.request {
            Some(Q::Info(_)) => self.info().map(A::Info),
            Some(Q::DumpOpen(r)) => self.dump_open(r.what).map(A::DumpOpen),
            Some(Q::ConfigFileOpen(r)) => self.config_file_open(r.id).map(A::DumpOpen),
            Some(Q::DumpRead(r)) => self.dump_read(r.handle, r.max_bytes).map(A::DumpRead),
            Some(Q::DumpClose(r)) => self.dump_close(r.handle).map(A::DumpClose),
            Some(Q::LoadOpen(r)) => self.load_open(r).map(A::LoadOpen),
            Some(Q::LoadWrite(r)) => self.load_write(r.handle, &r.data).map(A::LoadWrite),
            Some(Q::LoadClose(r)) => self.load_close(r.handle, r.abort).map(A::LoadClose),
            None => Err("empty request".into()),
        };
        pb::BrokerResponse { response: Some(result.unwrap_or_else(|message| A::Error(pb::BrokerError { message }))) }
    }

    fn stop_all(&mut self) {
        for (_, stream) in std::mem::take(&mut self.streams) {
            if let Stream::Tool { mut child, .. } = stream {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Some((_, mut load)) = self.load.take() {
            let _ = load.child.kill();
            let _ = load.child.wait();
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
    let args = match args_from(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("onesimus-ldap-broker: {e}");
            std::process::exit(2);
        }
    };
    // SAFETY: getuid/geteuid have no preconditions; a broker never runs as root.
    if unsafe { libc::getuid() == 0 || libc::geteuid() == 0 } {
        eprintln!("onesimus-ldap-broker: refusing to run as root");
        std::process::exit(2);
    }
    // A write to a dead slapadd must be an error, not a signal.
    // SAFETY: setting a signal disposition before any thread exists.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    // SAFETY: fd 0 and 1 are the socket ofd passed; owned by this process.
    let (mut input, mut output) = unsafe { (std::fs::File::from_raw_fd(0), std::fs::File::from_raw_fd(1)) };
    let mut broker = Broker { args, streams: BTreeMap::new(), load: None, next_handle: 1, files: Vec::new() };
    loop {
        let frame = match read_frame(&mut input) {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                eprintln!("onesimus-ldap-broker: {e}");
                broker.stop_all();
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
    broker.stop_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        args_from(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn arguments_are_fixed_and_plain() {
        let a = args(&["--sbin", "/usr/sbin", "--config-dir", "/etc/ldap/slapd.d"]).unwrap();
        assert_eq!(a.config, ConfigRef::Dir("/etc/ldap/slapd.d".into()));
        assert!(a.target.is_none());
        assert!(args(&["--sbin", "/usr/sbin"]).is_err(), "a configuration is required");
        assert!(args(&["--sbin", "usr/sbin", "--config-dir", "/x"]).is_err(), "relative");
        assert!(args(&["--sbin", "/usr/sbin", "--config-dir", "/etc/../x"]).is_err(), "dots");
        assert!(args(&["--sbin", "/usr/sbin", "--config-dir", "/a", "--config-file", "/b"]).is_err(), "both");
        assert!(args(&["--sbin", "/usr/sbin", "--config-dir", "/a", "--uri", "/b"]).is_err(), "unknown");
    }

    fn broker(target: Option<&str>) -> Broker {
        Broker {
            args: Args { sbin: "/nonexistent".into(), config: ConfigRef::File("/etc/ldap/slapd.conf".into()), target: target.map(PathBuf::from) },
            streams: BTreeMap::new(),
            load: None,
            next_handle: 1,
            files: Vec::new(),
        }
    }

    #[test]
    fn a_reading_broker_never_loads_and_a_loading_broker_never_reads() {
        let mut b = broker(None);
        let load = |dir: &str| pb::BrokerLoadOpen {
            against: Some(pb::broker_load_open::Against::ConfigDir(dir.into())),
            what: Some(pb::broker_load_open::What::Suffix("dc=x".into())),
        };
        assert!(b.load_open(load("/var/lib/onesimus-ldap-restore/1/slapd.d")).unwrap_err().contains("only reads"));
        let mut b = broker(Some("/var/lib/onesimus-ldap-restore/1"));
        assert!(b.dump_open(Some(pb::dump_open_request::What::Suffix("dc=x".into()))).unwrap_err().contains("does not read"));
        assert!(b.config_file_open(0).unwrap_err().contains("does not read"));
        // Only configurations inside the target.
        assert!(b.load_open(load("/etc/ldap/slapd.d")).unwrap_err().contains("inside the restore target"));
        assert!(b.load_open(load("/var/lib/onesimus-ldap-restore/1/../../ldap")).unwrap_err().contains("inside"));
        assert!(b.load_open(load("/var/lib/onesimus-ldap-restore/1")).unwrap_err().contains("inside"));
        let bad = pb::BrokerLoadOpen {
            against: Some(pb::broker_load_open::Against::ConfigDir("/var/lib/onesimus-ldap-restore/1/slapd.d".into())),
            what: Some(pb::broker_load_open::What::Suffix("-n0".into())),
        };
        assert!(b.load_open(bad).unwrap_err().contains("invalid suffix"));
        let conf = pb::BrokerLoadOpen {
            against: Some(pb::broker_load_open::Against::ConfigFile("/var/lib/onesimus-ldap-restore/1/conf/slapd.conf".into())),
            what: Some(pb::broker_load_open::What::Config(pb::ConfigDump {})),
        };
        assert!(b.load_open(conf).unwrap_err().contains("slapd.d"));
    }

    #[test]
    fn configuration_files_come_only_from_configuration_directories() {
        let tls = vec!["/etc/ldap/tls/server.key".to_string()];
        assert!(Broker::allowed_config_file(Path::new("/etc/shadow"), None, &tls).is_err());
        assert!(Broker::allowed_config_file(Path::new("/etc/passwd"), Some(Path::new("/etc/ldap")), &tls).is_err());
        assert!(Broker::allowed_config_file(Path::new("/etc/default/slapd"), None, &tls).is_ok()
            || !Path::new("/etc/default/slapd").exists());
        assert!(under(Path::new("/etc/ldap/schema/core.schema"), Path::new("/etc/ldap")));
        assert!(!under(Path::new("/etc/ldap"), Path::new("/etc/ldap")) && !under(Path::new("/etc/ldapx/a"), Path::new("/etc/ldap")));
    }
}
