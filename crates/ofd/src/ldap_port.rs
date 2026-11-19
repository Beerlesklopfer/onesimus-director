//! The host side of the OpenLDAP broker imports
//! (docs/plans/OPENLDAP-PLUGIN.md).
//!
//! The guest asks; ofd decides. In a backup job ofd fixes from the broker's
//! info which databases this plugin line dumps (`databases=`/`exclude=`,
//! only `mdb`, each directory owned by the slapd user) and whether the
//! configuration goes along (`config=`), and checks every request: suffix in
//! that set, configuration file ids from the info list, reads only on its
//! own handles and at most one block. TLS files the FileSet misses give a
//! warning (E-9).
//!
//! In an online restore job (`restore_mode=online`, capability
//! `openldap.write`) everything goes into a *new* target ofd creates
//! (default /var/lib/onesimus-ldap-restore/<jobid>), never next to or over
//! the running instance. ofd, not the guest, rewrites the restored
//! configuration (olcDbDirectory / directory, pid and args files, include
//! paths) onto the target and creates its directories before slapadd runs.
//! A failed restore removes the target.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use onesimus_plugins::wasm::{BrokerFunction, BrokerPort, OpenldapFunction, OPENLDAP_READ, OPENLDAP_WRITE};
use onesimus_proto_rs::onesimus::filed::broker::openldap::v1 as pb;
use onesimus_proto_rs::onesimus::filed::plugin::v1 as plugin_pb;
use prost::Message;

use crate::broker::RunAs;
use crate::ldap_instance::ConfigPath;
use crate::plugin::{check_trusted, PluginError, PluginLine, PreparedBroker};

/// Where online restores go unless the restore job names a `target=`.
pub const RESTORE_BASE: &str = "/var/lib/onesimus-ldap-restore";
const MAX_FRAME: usize = 2 * 1024 * 1024;
/// Largest cn=config LDIF / single configuration file held for rewriting.
const MAX_CONFIG_LDIF: usize = 16 * 1024 * 1024;
const MAX_CONFIG_FILE: usize = 4 * 1024 * 1024;
const MAX_CONFIG_FILES: usize = 64;
const MAX_DIAGNOSTIC: usize = 512;

/// `*` matches any run of characters; ASCII case is ignored (suffixes).
fn glob(pattern: &str, name: &str) -> bool {
    let (pattern, name) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if name.len() < first.len() + last.len() || !name.starts_with(first) || !name.ends_with(last) {
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

/// Suffix patterns of an option: `;`-separated (a suffix holds commas).
fn patterns(value: Option<&String>) -> Vec<String> {
    value.map(|v| v.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

fn yes(options: &BTreeMap<String, String>, key: &str, default: bool) -> bool {
    options.get(key).map(|v| !matches!(v.as_str(), "no" | "false" | "0")).unwrap_or(default)
}

/// Marks the databases this plugin line dumps; the others get a reason.
/// Only `mdb` holds data slapcat can read (L-6, L-12).
pub fn select(info: &mut pb::InfoResponse, options: &BTreeMap<String, String>) -> Result<BTreeSet<String>, String> {
    let wanted = patterns(options.get("databases"));
    let wanted = if wanted.is_empty() { vec!["*".to_string()] } else { wanted };
    let exclude = patterns(options.get("exclude"));
    for pattern in wanted.iter().filter(|p| !p.contains('*')) {
        match info.databases.iter().find(|d| glob(pattern, &d.suffix)) {
            None => return Err(format!("suffix \"{pattern}\" does not exist")),
            Some(d) if d.backend != "mdb" => {
                return Err(format!("suffix \"{pattern}\" is a {} database without local storage", d.backend));
            }
            Some(_) => {}
        }
    }
    let mut selected = BTreeSet::new();
    for d in &mut info.databases {
        let matched = wanted.iter().any(|p| glob(p, &d.suffix));
        let reason = if d.backend == "frontend" || d.backend == "config" {
            Some("configuration, not data")
        } else if d.backend != "mdb" {
            Some("no local storage (only mdb is backed up)")
        } else if d.suffix.is_empty() {
            Some("no suffix")
        } else if !matched {
            Some("not selected by databases=")
        } else if exclude.iter().any(|e| glob(e, &d.suffix)) {
            Some("excluded by exclude=")
        } else {
            None
        };
        d.selected = reason.is_none();
        d.skipped_reason = reason.unwrap_or_default().to_string();
        if d.selected {
            selected.insert(d.suffix.clone());
        }
    }
    Ok(selected)
}

/// Whether `path` is backed up by one of the FileSet's `File =` names.
fn covered(path: &str, names: &[Vec<u8>]) -> bool {
    let p = path.as_bytes();
    names.iter().any(|n| {
        let n = n.strip_suffix(b"/").unwrap_or(n);
        p == n || (p.starts_with(n) && p.get(n.len()) == Some(&b'/'))
    })
}

fn plain_absolute(path: &Path) -> bool {
    path.is_absolute() && path.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

/// Same path, or one below the other.
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Standard base64 (LDIF `attr:: value`).
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace()) {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// A database of the restored configuration and its directory in the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDb {
    pub number: i32,
    pub backend: String,
    pub suffix: String,
    pub directory: PathBuf,
}

fn attr_of(line: &str) -> &str {
    line.split(':').next().unwrap_or("")
}

/// The cn=config LDIF of a restore, rewritten onto `target`: every
/// olcDbDirectory to `<target>/db-<n>` (added where missing for mdb),
/// olcPidFile/olcArgsFile to `<target>/run/`. Returns the LDIF and the
/// databases with their new directories.
pub fn rewrite_cn_config(ldif: &[u8], target: &Path) -> Result<(Vec<u8>, Vec<TargetDb>), String> {
    let text = std::str::from_utf8(ldif).map_err(|_| "the cn=config LDIF is not UTF-8".to_string())?;
    // Unfold continuation lines; entries split at blank lines.
    let mut entries: Vec<Vec<String>> = vec![Vec::new()];
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() {
            if !entries.last().is_some_and(Vec::is_empty) {
                entries.push(Vec::new());
            }
        } else if let Some(more) = line.strip_prefix(' ') {
            let last = entries.last_mut().and_then(|e| e.last_mut()).ok_or("LDIF starts with a continuation line")?;
            last.push_str(more);
        } else if !line.starts_with('#') {
            entries.last_mut().expect("one entry").push(line.to_string());
        }
    }
    let value = |line: &str| -> String {
        let rest = &line[attr_of(line).len() + 1..];
        match rest.strip_prefix(':') {
            Some(b64) => base64_decode(b64.trim()).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default(),
            None => rest.trim_start().to_string(),
        }
    };
    let mut out = String::new();
    let mut dbs = Vec::new();
    for entry in entries.into_iter().filter(|e| !e.is_empty()) {
        let dn = entry.first().filter(|l| attr_of(l).eq_ignore_ascii_case("dn")).map(|l| value(l))
            .ok_or("an LDIF entry without dn")?;
        let rdns: Vec<&str> = dn.split(',').collect();
        let database = match rdns.as_slice() {
            [db, config] if config.eq_ignore_ascii_case("cn=config") => db.split_once('=')
                .filter(|(k, _)| k.eq_ignore_ascii_case("olcDatabase"))
                .and_then(|(_, v)| v.strip_prefix('{')?.split_once('}'))
                .and_then(|(n, backend)| Some((n.parse::<i32>().ok()?, backend.to_ascii_lowercase()))),
            _ => None,
        };
        let global = dn.eq_ignore_ascii_case("cn=config");
        let mut lines = Vec::new();
        let mut has_directory = false;
        let mut suffix = String::new();
        for line in &entry {
            let attr = attr_of(line).to_ascii_lowercase();
            match (attr.as_str(), &database) {
                ("olcdbdirectory", Some((n, _))) => {
                    has_directory = true;
                    lines.push(format!("olcDbDirectory: {}/db-{n}", target.display()));
                }
                ("olcdbdirectory", None) => return Err(format!("olcDbDirectory outside a database entry ({dn})")),
                ("olcpidfile", _) if global => lines.push(format!("olcPidFile: {}/run/slapd.pid", target.display())),
                ("olcargsfile", _) if global => lines.push(format!("olcArgsFile: {}/run/slapd.args", target.display())),
                ("olcsuffix", Some(_)) => {
                    if suffix.is_empty() {
                        suffix = value(line);
                    }
                    lines.push(line.clone());
                }
                _ => lines.push(line.clone()),
            }
        }
        if let Some((n, backend)) = &database {
            if backend == "mdb" && !has_directory {
                lines.push(format!("olcDbDirectory: {}/db-{n}", target.display()));
                has_directory = true;
            }
            if has_directory {
                dbs.push(TargetDb { number: *n, backend: backend.clone(), suffix, directory: target.join(format!("db-{n}")) });
            }
        }
        for l in lines {
            out.push_str(&l);
            out.push('\n');
        }
        out.push('\n');
    }
    Ok((out.into_bytes(), dbs))
}

/// slapd.conf tokens: white space separated, `"…"` quoted with `\` escapes.
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut token = String::new();
        if c == '"' {
            chars.next();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => token.extend(chars.next()),
                    '"' => break,
                    c => token.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                token.push(c);
                chars.next();
            }
        }
        out.push(token);
    }
    out
}

/// A configuration file's place in the target: `<target>/conf/<original>`.
fn conf_path(target: &Path, original: &Path) -> PathBuf {
    target.join("conf").join(original.strip_prefix("/").unwrap_or(original))
}

/// The slapd.conf file set of a restore, rewritten onto `target`: include
/// paths of restored files point into `<target>/conf`, every database gets
/// exactly one `directory <target>/db-<n>` (right after its `database`
/// line), pidfile/argsfile go to `<target>/run/`. Files are walked in
/// slapd's order (includes inline) so database numbers match. Returns the
/// files by their place in the target and the databases.
pub fn rewrite_slapd_conf(files: &BTreeMap<PathBuf, Vec<u8>>, main: &Path, target: &Path)
    -> Result<(BTreeMap<PathBuf, Vec<u8>>, Vec<TargetDb>), String> {
    let base = main.parent().unwrap_or(Path::new("/")).to_path_buf();
    let mut out: BTreeMap<PathBuf, Vec<u8>> = BTreeMap::new();
    let mut dbs: Vec<TargetDb> = Vec::new();
    let mut next = 1;
    let mut current: Option<usize> = None;
    fn walk(file: &Path, files: &BTreeMap<PathBuf, Vec<u8>>, base: &Path, target: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>,
        dbs: &mut Vec<TargetDb>, next: &mut i32, current: &mut Option<usize>, depth: usize) -> Result<(), String> {
        if depth > 16 {
            return Err("includes nest too deep".into());
        }
        let dest = conf_path(target, file);
        if out.contains_key(&dest) {
            return Ok(());
        }
        out.insert(dest.clone(), Vec::new());
        let text = String::from_utf8_lossy(files.get(file).ok_or_else(|| format!("{} was not restored", file.display()))?).into_owned();
        let mut result = String::new();
        for line in text.lines() {
            let directive = !line.starts_with([' ', '\t']) && !line.trim_start().starts_with('#') && !line.trim().is_empty();
            let words = if directive { tokens(line) } else { Vec::new() };
            let keyword = words.first().map(|w| w.to_ascii_lowercase()).unwrap_or_default();
            let arg = words.get(1).cloned().unwrap_or_default();
            match keyword.as_str() {
                "include" => {
                    let original = if Path::new(&arg).is_absolute() { PathBuf::from(&arg) } else { base.join(&arg) };
                    if plain_absolute(&original) && files.contains_key(&original) {
                        result.push_str(&format!("include \"{}\"\n", conf_path(target, &original).display()));
                        // Walk the include where slapd reads it (database order).
                        walk(&original, files, base, target, out, dbs, next, current, depth + 1)?;
                    } else {
                        result.push_str(line);
                        result.push('\n');
                    }
                }
                "database" => {
                    let backend = arg.to_ascii_lowercase();
                    let number = match backend.as_str() {
                        "config" => 0,
                        "frontend" => -1,
                        _ => {
                            *next += 1;
                            *next - 1
                        }
                    };
                    result.push_str(line);
                    result.push('\n');
                    let directory = target.join(format!("db-{number}"));
                    if backend == "mdb" {
                        result.push_str(&format!("directory \"{}\"\n", directory.display()));
                    }
                    dbs.push(TargetDb { number, backend, suffix: String::new(), directory });
                    *current = Some(dbs.len() - 1);
                }
                "directory" => result.push_str("# directory: set by the ofd restore after the database line\n"),
                "pidfile" => result.push_str(&format!("pidfile \"{}/run/slapd.pid\"\n", target.display())),
                "argsfile" => result.push_str(&format!("argsfile \"{}/run/slapd.args\"\n", target.display())),
                "suffix" => {
                    if let Some(db) = current.and_then(|i| dbs.get_mut(i)) {
                        if db.suffix.is_empty() {
                            db.suffix = arg;
                        }
                    }
                    result.push_str(line);
                    result.push('\n');
                }
                _ => {
                    result.push_str(line);
                    result.push('\n');
                }
            }
        }
        out.insert(dest, result.into_bytes());
        Ok(())
    }
    walk(main, files, &base, target, &mut out, &mut dbs, &mut next, &mut current, 0)?;
    // Restored files no include reaches (e.g. /etc/default/slapd) are kept as they are.
    for (original, bytes) in files {
        out.entry(conf_path(target, original)).or_insert_with(|| bytes.clone());
    }
    dbs.retain(|d| d.backend == "mdb");
    Ok((out, dbs))
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
        Some(pb::broker_response::Response::Error(e)) => Err(bounded(&e.message)),
        Some(r) => Ok(r),
        None => Err("broker: empty response".into()),
    }
}

/// A broker text as it may reach the job log: bounded, no control characters.
fn bounded(text: &str) -> String {
    text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(MAX_DIAGNOSTIC).collect()
}

/// The ownership rule for what ofd creates and checks (root and `/` in
/// production; the test user and a temporary tree in tests).
#[derive(Debug, Clone)]
pub struct Trust {
    pub owner: u32,
    pub top: PathBuf,
}

enum Load {
    ConfigLdif(Vec<u8>),
    ConfigFile { path: PathBuf, main: bool, data: Vec<u8> },
    Data { broker: u32 },
}

struct Restore {
    target: PathBuf,
    /// The default base, created on demand.
    base: Option<PathBuf>,
    run_as: RunAs,
    trust: Trust,
    production: Vec<PathBuf>,
    created: bool,
    /// The configuration of the restore, once loaded: where slapadd reads it.
    against: Option<pb::broker_load_open::Against>,
    config_files: BTreeMap<PathBuf, Vec<u8>>,
    main_file: Option<PathBuf>,
    dbs: Vec<TargetDb>,
    loads: BTreeMap<u32, Load>,
    next: u32,
    failed: bool,
    loaded: usize,
}

enum Mode {
    Backup,
    Restore(Box<Restore>),
}

/// The checked port over a broker connection.
pub struct LdapPort<T: Read + Write + Send> {
    transport: T,
    chunk: u32,
    mode: Mode,
    /// The info the guest sees (selection marked).
    info: pb::InfoResponse,
    selected: BTreeSet<String>,
    config_ids: BTreeSet<u32>,
    open: BTreeSet<u32>,
    pub warnings: Vec<String>,
}

/// Restore settings: target (None: `<base>/<job_id>`), the slapd user.
pub struct RestoreSettings {
    pub target: Option<PathBuf>,
    pub base: PathBuf,
    pub job_id: u64,
    pub run_as: RunAs,
    pub trust: Trust,
}

impl<T: Read + Write + Send> LdapPort<T> {
    fn ask_info(transport: &mut T) -> Result<pb::InfoResponse, String> {
        match exchange(transport, pb::broker_request::Request::Info(pb::InfoRequest {}))? {
            pb::broker_response::Response::Info(i) => Ok(i),
            _ => Err("broker: unexpected answer to info".into()),
        }
    }

    /// A backup port: asks the broker for the instance, checks it against
    /// what ofd discovered and fixes the selection.
    pub fn backup(mut transport: T, options: &BTreeMap<String, String>, chunk: u32, config: &ConfigPath, run_as: RunAs,
        fileset: &[Vec<u8>]) -> Result<Self, String> {
        let mut info = Self::ask_info(&mut transport)?;
        Self::check_kind(&info, config)?;
        let selected = select(&mut info, options)?;
        for d in info.databases.iter().filter(|d| d.selected) {
            let dir = Path::new(&d.directory);
            let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            if !plain_absolute(dir) || meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(format!("{}: database directory of {} is not a plain directory", dir.display(), d.suffix));
            }
            if meta.uid() != run_as.uid {
                return Err(format!("{}: owned by uid {}, slapd runs as uid {}", dir.display(), meta.uid(), run_as.uid));
            }
        }
        info.config_selected = yes(options, "config", true);
        if !info.config_selected {
            info.config_files.clear();
        }
        let config_ids = info.config_files.iter().map(|f| f.id).collect();
        let warnings = info.tls_files.iter().filter(|t| !covered(t, fileset)).map(|t| format!(
            "TLS file {t} of the slapd configuration is not in the FileSet; back it up with a File = line (the plugin never reads key files)"))
            .collect();
        Ok(Self { transport, chunk, mode: Mode::Backup, info, selected, config_ids, open: BTreeSet::new(), warnings })
    }

    fn check_kind(info: &pb::InfoResponse, config: &ConfigPath) -> Result<(), String> {
        let kind_ok = match config {
            ConfigPath::Dir(_) => info.kind == pb::ConfigKind::SlapdD as i32,
            ConfigPath::File(_) => info.kind == pb::ConfigKind::SlapdConf as i32,
        };
        if !kind_ok || Path::new(&info.config_path) != config.path() {
            return Err("broker: the configuration differs from the running instance".into());
        }
        Ok(())
    }

    /// An online-restore port: checks the target against the running
    /// instance now (before anything is restored); creates it on the first load.
    pub fn restore(mut transport: T, chunk: u32, config: &ConfigPath, settings: RestoreSettings) -> Result<Self, String> {
        let info = Self::ask_info(&mut transport)?;
        Self::check_kind(&info, config)?;
        let mut production: Vec<PathBuf> = vec![config.path().to_path_buf()];
        production.extend(info.databases.iter().filter(|d| !d.directory.is_empty()).map(|d| PathBuf::from(&d.directory)));
        let (target, base) = match settings.target {
            Some(t) => (t, None),
            None => (settings.base.join(settings.job_id.to_string()), Some(settings.base.clone())),
        };
        let restore = Restore {
            target, base, run_as: settings.run_as, trust: settings.trust, production, created: false, against: None,
            config_files: BTreeMap::new(), main_file: None, dbs: Vec::new(), loads: BTreeMap::new(), next: 1,
            failed: false, loaded: 0,
        };
        restore.validate()?;
        Ok(Self { transport, chunk, mode: Mode::Restore(Box::new(restore)), info, selected: BTreeSet::new(),
            config_ids: BTreeSet::new(), open: BTreeSet::new(), warnings: Vec::new() })
    }

    pub fn target(&self) -> Option<&Path> {
        match &self.mode {
            Mode::Restore(r) => Some(&r.target),
            Mode::Backup => None,
        }
    }

    fn backup_only(&self) -> Result<(), String> {
        match self.mode {
            Mode::Backup => Ok(()),
            Mode::Restore(_) => Err("reading the instance is not available in a restore job".into()),
        }
    }

    fn checked(&mut self, function: OpenldapFunction, request: &[u8]) -> Result<Vec<u8>, String> {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let bad = |e: prost::DecodeError| format!("bad request: {e}");
        match function {
            OpenldapFunction::Info => {
                pb::InfoRequest::decode(request).map_err(bad)?;
                Ok(self.info.encode_to_vec())
            }
            OpenldapFunction::DumpOpen => {
                self.backup_only()?;
                let r = pb::DumpOpenRequest::decode(request).map_err(bad)?;
                match &r.what {
                    Some(pb::dump_open_request::What::Config(_)) => {
                        if !self.info.config_selected {
                            return Err("the configuration is off for this plugin line (config=no)".into());
                        }
                        if self.info.kind != pb::ConfigKind::SlapdD as i32 {
                            return Err("slapd.conf has no cn=config dump; its files are read by id".into());
                        }
                    }
                    Some(pb::dump_open_request::What::Suffix(s)) if self.selected.contains(s) => {}
                    Some(pb::dump_open_request::What::Suffix(s)) => {
                        return Err(format!("suffix \"{s}\" is not selected for this plugin line"));
                    }
                    None => return Err("dump of what?".into()),
                }
                let answer = exchange(&mut self.transport, Q::DumpOpen(r))?;
                self.opened(answer)
            }
            OpenldapFunction::ConfigFileOpen => {
                self.backup_only()?;
                let r = pb::ConfigFileOpenRequest::decode(request).map_err(bad)?;
                if !self.config_ids.contains(&r.id) {
                    return Err("unknown configuration file id".into());
                }
                let answer = exchange(&mut self.transport, Q::ConfigFileOpen(r))?;
                self.opened(answer)
            }
            OpenldapFunction::DumpRead => {
                let mut r = pb::DumpReadRequest::decode(request).map_err(bad)?;
                if !self.open.contains(&r.handle) {
                    return Err("unknown dump handle".into());
                }
                r.max_bytes = r.max_bytes.clamp(1, self.chunk);
                match exchange(&mut self.transport, Q::DumpRead(r))? {
                    A::DumpRead(mut d) if d.data.len() <= self.chunk as usize => {
                        d.diagnostic = bounded(&d.diagnostic);
                        Ok(d.encode_to_vec())
                    }
                    _ => Err("broker: unexpected answer".into()),
                }
            }
            OpenldapFunction::DumpClose => {
                let r = pb::DumpCloseRequest::decode(request).map_err(bad)?;
                if !self.open.remove(&r.handle) {
                    return Err("unknown dump handle".into());
                }
                match exchange(&mut self.transport, Q::DumpClose(r))? {
                    A::DumpClose(c) => Ok(c.encode_to_vec()),
                    _ => Err("broker: unexpected answer".into()),
                }
            }
            OpenldapFunction::LoadOpen | OpenldapFunction::LoadWrite | OpenldapFunction::LoadClose => {
                let chunk = self.chunk;
                let Mode::Restore(restore) = &mut self.mode else {
                    return Err(format!("{OPENLDAP_WRITE} is not available in a backup job"));
                };
                let result = restore.load(&mut self.transport, function, request, chunk);
                // A refused open changes nothing; a broken write or close spoils the restore.
                if result.is_err() && function != OpenldapFunction::LoadOpen {
                    restore.failed = true;
                }
                result
            }
        }
    }

    fn opened(&mut self, answer: pb::broker_response::Response) -> Result<Vec<u8>, String> {
        match answer {
            pb::broker_response::Response::DumpOpen(o) => {
                self.open.insert(o.handle);
                Ok(o.encode_to_vec())
            }
            _ => Err("broker: unexpected answer".into()),
        }
    }
}

impl Restore {
    /// The target is new or empty, a plain path whose parent is trusted,
    /// and touches no path of the running instance.
    fn validate(&self) -> Result<(), String> {
        let t = &self.target;
        if !plain_absolute(t) || t.parent().is_none() {
            return Err(format!("{}: restore target must be an absolute path without . or ..", t.display()));
        }
        if let Some(p) = self.production.iter().find(|p| overlaps(t, p)) {
            return Err(format!("{}: restore target overlaps {} of the running instance", t.display(), p.display()));
        }
        match std::fs::symlink_metadata(t) {
            Ok(m) if m.file_type().is_symlink() => return Err(format!("{}: restore target is a symbolic link", t.display())),
            Ok(m) if !m.is_dir() => return Err(format!("{}: restore target is not a directory", t.display())),
            Ok(m) if m.uid() != self.trust.owner || m.mode() & 0o022 != 0 => {
                return Err(format!("{}: restore target is not ofd's own directory (uid {}, mode {:o})", t.display(), m.uid(), m.mode() & 0o7777));
            }
            Ok(_) if !self.created && std::fs::read_dir(t).map(|mut d| d.next().is_some()).unwrap_or(true) => {
                return Err(format!("{}: restore target exists and is not empty", t.display()));
            }
            _ => {}
        }
        let parent = t.parent().expect("checked above");
        let base_pending = self.base.as_deref() == Some(parent) && !parent.exists();
        let trusted_dir = if base_pending { parent.parent().unwrap_or(Path::new("/")) } else { parent };
        check_trusted(trusted_dir, self.trust.owner, true, &self.trust.top).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn mkdir_for_slapd(&self, dir: &Path) -> Result<(), String> {
        std::fs::DirBuilder::new().mode(0o700).create(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::os::unix::fs::chown(dir, Some(self.run_as.uid), Some(self.run_as.gid)).map_err(|e| format!("{}: {e}", dir.display()))
    }

    /// Creates the target (0755, ofd's) and `run/` (slapd user, 0700).
    fn ensure_target(&mut self) -> Result<(), String> {
        if self.created {
            return Ok(());
        }
        self.validate()?;
        if let Some(base) = &self.base {
            if !base.exists() {
                std::fs::DirBuilder::new().mode(0o755).create(base).map_err(|e| format!("{}: {e}", base.display()))?;
            }
        }
        match std::fs::DirBuilder::new().mode(0o755).create(&self.target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", self.target.display())),
        }
        self.created = true;
        self.validate()?;
        self.mkdir_for_slapd(&self.target.join("run"))
    }

    fn handle(&mut self, load: Load) -> pb::LoadOpenResponse {
        let handle = self.next;
        self.next += 1;
        self.loads.insert(handle, load);
        pb::LoadOpenResponse { handle }
    }

    fn load<T: Read + Write>(&mut self, transport: &mut T, function: OpenldapFunction, request: &[u8], chunk: u32)
        -> Result<Vec<u8>, String> {
        let bad = |e: prost::DecodeError| format!("bad request: {e}");
        match function {
            OpenldapFunction::LoadOpen => {
                let r = pb::LoadOpenRequest::decode(request).map_err(bad)?;
                let response = match r.what {
                    Some(pb::load_open_request::What::Config(_)) => {
                        if self.against.is_some() || !self.config_files.is_empty() {
                            return Err("the restore already has a configuration".into());
                        }
                        self.ensure_target()?;
                        self.handle(Load::ConfigLdif(Vec::new()))
                    }
                    Some(pb::load_open_request::What::ConfigFile(f)) => {
                        let path = PathBuf::from(&f.path);
                        if self.against.is_some() || !plain_absolute(&path) || path == Path::new("/") {
                            return Err("a configuration file must come before the data, with an absolute plain path".into());
                        }
                        if self.config_files.len() >= MAX_CONFIG_FILES {
                            return Err("too many configuration files".into());
                        }
                        self.ensure_target()?;
                        self.handle(Load::ConfigFile { path, main: f.main, data: Vec::new() })
                    }
                    Some(pb::load_open_request::What::Suffix(suffix)) => {
                        self.finish_config_files()?;
                        let against = self.against.clone()
                            .ok_or("restore the configuration together with the data (select config/ too)")?;
                        let db = self.dbs.iter().find(|d| d.suffix.eq_ignore_ascii_case(&suffix))
                            .ok_or_else(|| format!("suffix \"{suffix}\" is no mdb database of the restored configuration"))?;
                        if !db.directory.starts_with(&self.target) || db.directory == self.target {
                            return Err("database directory outside the restore target".into());
                        }
                        let open = pb::BrokerLoadOpen {
                            against: Some(against),
                            what: Some(pb::broker_load_open::What::Suffix(db.suffix.clone())),
                        };
                        let pb::broker_response::Response::LoadOpen(o) = exchange(transport, pb::broker_request::Request::LoadOpen(open))? else {
                            return Err("broker: unexpected answer".into());
                        };
                        self.handle(Load::Data { broker: o.handle })
                    }
                    None => return Err("load of what?".into()),
                };
                Ok(response.encode_to_vec())
            }
            OpenldapFunction::LoadWrite => {
                let r = pb::LoadWriteRequest::decode(request).map_err(bad)?;
                if r.data.len() > chunk as usize {
                    return Err("load block larger than the chunk".into());
                }
                match self.loads.get_mut(&r.handle).ok_or("unknown load handle")? {
                    Load::ConfigLdif(buf) if buf.len() + r.data.len() <= MAX_CONFIG_LDIF => buf.extend_from_slice(&r.data),
                    Load::ConfigFile { data, .. } if data.len() + r.data.len() <= MAX_CONFIG_FILE => data.extend_from_slice(&r.data),
                    Load::Data { broker } => {
                        let w = pb::LoadWriteRequest { handle: *broker, data: r.data };
                        let pb::broker_response::Response::LoadWrite(_) = exchange(transport, pb::broker_request::Request::LoadWrite(w))? else {
                            return Err("broker: unexpected answer".into());
                        };
                    }
                    _ => return Err("configuration too large".into()),
                }
                Ok(pb::LoadWriteResponse {}.encode_to_vec())
            }
            OpenldapFunction::LoadClose => {
                let r = pb::LoadCloseRequest::decode(request).map_err(bad)?;
                let load = self.loads.remove(&r.handle).ok_or("unknown load handle")?;
                let target = self.target.display().to_string();
                if r.abort {
                    self.failed = true;
                    if let Load::Data { broker } = load {
                        let _ = exchange(transport, pb::broker_request::Request::LoadClose(pb::LoadCloseRequest { handle: broker, abort: true }));
                    }
                    return Ok(pb::LoadCloseResponse { ok: false, diagnostic: "aborted".into(), target }.encode_to_vec());
                }
                let result = match load {
                    Load::ConfigLdif(buf) => self.load_cn_config(transport, &buf, chunk)?,
                    Load::ConfigFile { path, main, data } => {
                        if main {
                            self.main_file = Some(path.clone());
                        }
                        self.config_files.insert(path, data);
                        pb::LoadCloseResponse { ok: true, diagnostic: String::new(), target: String::new() }
                    }
                    Load::Data { broker } => {
                        let close = pb::LoadCloseRequest { handle: broker, abort: false };
                        match exchange(transport, pb::broker_request::Request::LoadClose(close))? {
                            pb::broker_response::Response::LoadClose(c) => c,
                            _ => return Err("broker: unexpected answer".into()),
                        }
                    }
                };
                if !result.ok {
                    self.failed = true;
                } else {
                    self.loaded += 1;
                }
                Ok(pb::LoadCloseResponse { ok: result.ok, diagnostic: bounded(&result.diagnostic), target }.encode_to_vec())
            }
            _ => Err("not a load function".into()),
        }
    }

    /// slapd.d: rewrite, create the database directories, slapadd -n 0.
    fn load_cn_config<T: Read + Write>(&mut self, transport: &mut T, ldif: &[u8], chunk: u32) -> Result<pb::LoadCloseResponse, String> {
        let (rewritten, dbs) = rewrite_cn_config(ldif, &self.target)?;
        let config_dir = self.target.join("slapd.d");
        self.mkdir_for_slapd(&config_dir)?;
        for db in &dbs {
            self.mkdir_for_slapd(&db.directory)?;
        }
        let against = pb::broker_load_open::Against::ConfigDir(config_dir.display().to_string());
        let result = self.slapadd(transport, against.clone(), pb::broker_load_open::What::Config(pb::ConfigDump {}), &rewritten, chunk)?;
        if result.ok {
            self.against = Some(against);
            self.dbs = dbs.into_iter().filter(|d| d.backend == "mdb").collect();
        }
        Ok(result)
    }

    /// slapd.conf: once the data starts, rewrite and write the file set.
    fn finish_config_files(&mut self) -> Result<(), String> {
        if self.against.is_some() || self.config_files.is_empty() {
            return Ok(());
        }
        let main = self.main_file.clone()
            .or_else(|| self.config_files.keys().find(|p| p.file_name().is_some_and(|n| n == "slapd.conf")).cloned())
            .ok_or("the restored configuration has no main slapd.conf")?;
        let (files, dbs) = rewrite_slapd_conf(&self.config_files, &main, &self.target)?;
        let conf = self.target.join("conf");
        for (path, bytes) in &files {
            // Directories below <target>/conf belong to the slapd user (0700).
            let rel = path.strip_prefix(&self.target).map_err(|_| "configuration outside the target".to_string())?;
            let mut dir = self.target.clone();
            for part in rel.parent().into_iter().flat_map(Path::components) {
                dir.push(part);
                if !dir.exists() {
                    self.mkdir_for_slapd(&dir)?;
                }
            }
            if !path.starts_with(&conf) {
                return Err("configuration outside the target".into());
            }
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600)
                .custom_flags(libc::O_NOFOLLOW).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
            f.write_all(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            std::os::unix::fs::chown(path, Some(self.run_as.uid), Some(self.run_as.gid)).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        for db in &dbs {
            self.mkdir_for_slapd(&db.directory)?;
        }
        self.dbs = dbs;
        self.against = Some(pb::broker_load_open::Against::ConfigFile(conf_path(&self.target, &main).display().to_string()));
        Ok(())
    }

    fn slapadd<T: Read + Write>(&mut self, transport: &mut T, against: pb::broker_load_open::Against,
        what: pb::broker_load_open::What, data: &[u8], chunk: u32) -> Result<pb::LoadCloseResponse, String> {
        use pb::broker_request::Request as Q;
        use pb::broker_response::Response as A;
        let A::LoadOpen(o) = exchange(transport, Q::LoadOpen(pb::BrokerLoadOpen { against: Some(against), what: Some(what) }))? else {
            return Err("broker: unexpected answer".into());
        };
        for block in data.chunks(chunk.max(1) as usize) {
            if let Err(e) = exchange(transport, Q::LoadWrite(pb::LoadWriteRequest { handle: o.handle, data: block.to_vec() })) {
                // slapadd stopped reading: its result tells why.
                let _ = e;
                break;
            }
        }
        match exchange(transport, Q::LoadClose(pb::LoadCloseRequest { handle: o.handle, abort: false }))? {
            A::LoadClose(c) => Ok(c),
            _ => Err("broker: unexpected answer".into()),
        }
    }

    /// The end of the job: a failed or empty restore leaves nothing behind.
    fn finish(&mut self, succeeded: bool) -> Vec<String> {
        if !self.created {
            return Vec::new();
        }
        // A slapd.conf restore without data: the file set is written now.
        if succeeded && !self.failed && self.against.is_none() && !self.config_files.is_empty() {
            if let Err(e) = self.finish_config_files() {
                self.failed = true;
                let _ = std::fs::remove_dir_all(&self.target);
                return vec![format!("OpenLDAP online restore failed ({e}); the new target {} was removed", self.target.display())];
            }
        }
        if self.failed || !succeeded || self.against.is_none() {
            let removed = std::fs::remove_dir_all(&self.target);
            return vec![match removed {
                Ok(()) => format!("OpenLDAP online restore failed; the new target {} was removed", self.target.display()),
                Err(e) => format!("OpenLDAP online restore failed; removing {}: {e}", self.target.display()),
            }];
        }
        let socket: String = format!("{}/run/ldapi", self.target.display()).bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect();
        let config = match &self.against {
            Some(pb::broker_load_open::Against::ConfigDir(d)) => format!("-F {d}"),
            Some(pb::broker_load_open::Against::ConfigFile(f)) => format!("-f {f}"),
            None => String::new(),
        };
        vec![format!(
            "OpenLDAP restored online into {} ({} part(s)); the running slapd is unchanged. Check it with: \
             slapd {config} -h ldapi://{socket} -u <slapd user>; switching over stays a manual step",
            self.target.display(), self.loaded)]
    }
}

impl<T: Read + Write + Send> BrokerPort for LdapPort<T> {
    fn capability(&self) -> &'static str {
        OPENLDAP_READ
    }
    fn serves(&self, capability: &str) -> bool {
        capability == OPENLDAP_READ || (capability == OPENLDAP_WRITE && matches!(self.mode, Mode::Restore(_)))
    }
    fn call(&mut self, function: BrokerFunction, request: &[u8]) -> Result<Vec<u8>, String> {
        match function {
            BrokerFunction::Openldap(f) => self.checked(f, request),
            BrokerFunction::Postgresql(_) => Err("not a function of the OpenLDAP broker".into()),
        }
    }
    fn finish(&mut self, succeeded: bool) -> Vec<String> {
        match &mut self.mode {
            Mode::Restore(r) => r.finish(succeeded),
            Mode::Backup => Vec::new(),
        }
    }
}

/// `openldap.read`: discover the running slapd and its user, check the
/// tools, start `<name>/onesimus-ldap-broker` as that user, and wrap it in
/// the checked port (backup, or online restore into a new target).
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare(manifest: &onesimus_plugins::Manifest, line: &PluginLine, plugin_dir: &Path, triplet: &str,
    chunk: u32, owner: u32, top: &Path, context: &crate::plugin::BrokerContext) -> Result<PreparedBroker, PluginError> {
    use onesimus_plugins::wasm::BrokerHandle;
    // E-3: an openldap line carries no secret; the local broker needs none.
    if let Some(key) = line.options.keys().find(|k| crate::secrets::is_secret(k)) {
        return Err(PluginError::Syntax(format!(
            "option \"{key}\": openldap plugin lines carry no passwords (the broker reads the local instance as its user)")));
    }
    let instance = crate::ldap_instance::discover(line.options.get("config_path").map(String::as_str))?;
    let sbin = Path::new("/usr/sbin");
    for tool in ["slapcat", "slapadd", "slapd"] {
        check_trusted(&sbin.join(tool), 0, false, Path::new("/"))?;
    }
    let restore = context.operation == plugin_pb::JobOperation::Restore;
    let mut args = vec!["--sbin".to_string(), sbin.display().to_string()];
    match &instance.config {
        ConfigPath::Dir(d) => args.extend(["--config-dir".to_string(), d.display().to_string()]),
        ConfigPath::File(f) => args.extend(["--config-file".to_string(), f.display().to_string()]),
    }
    let target = line.options.get("target").map(PathBuf::from);
    let settings = RestoreSettings {
        target: target.clone(),
        base: PathBuf::from(RESTORE_BASE),
        job_id: context.job_id,
        run_as: instance.run_as,
        trust: Trust { owner: 0, top: PathBuf::from("/") },
    };
    if restore {
        let t = target.unwrap_or_else(|| settings.base.join(settings.job_id.to_string()));
        if !plain_absolute(&t) {
            return Err(PluginError::Syntax(format!("target \"{}\" is not an absolute plain path", t.display())));
        }
        args.extend(["--target".to_string(), t.display().to_string()]);
    }
    let part = format!("{}/onesimus-ldap-broker", line.name);
    let broker = crate::broker::launch_checked(plugin_dir, manifest, triplet, &part, instance.run_as, &args, owner, top)?;
    let io = |e: std::io::Error| PluginError::Load(format!("broker socket: {e}"));
    let cancel = broker.socket.try_clone().map_err(io)?;
    let transport = broker.socket.try_clone().map_err(io)?;
    let port = if restore {
        LdapPort::restore(transport, chunk, &instance.config, settings)
    } else {
        LdapPort::backup(transport, &line.options, chunk, &instance.config, instance.run_as, context.covered)
    }
    .map_err(PluginError::Load)?;
    let warnings = port.warnings.clone();
    Ok(PreparedBroker {
        handle: BrokerHandle(std::sync::Arc::new(std::sync::Mutex::new(Box::new(port)))),
        broker,
        cancel,
        warnings,
    })
}

#[cfg(test)]
mod tests;
