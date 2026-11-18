//! Signed plugins in ofd (plan: docs/plans/PLUGIN-PHASE1.md).
//!
//! A `Plugin = "name:key=value:…"` line of a FileSet names a signed bundle
//! `<Plugin Directory>/<name>.pb`. Only plugins listed in `Plugin Names` run,
//! only with a `Plugin Policy`, and only when the policy, the directory and
//! the bundle are owned by root and not writable by group or others (like
//! sshd's StrictModes): whoever can write the policy could trust their own CA.
//! There is no way to run an unsigned plugin, not even for tests.

use std::collections::BTreeMap;
use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use onesimus_plugins::bundle;
use onesimus_plugins::executor::{Binding, Host, Limits, Worker};
use onesimus_plugins::wasm::{self, LoadOptions, Operation, Reply, RuntimeLimits, WasmBackend};
use onesimus_proto_rs::onesimus::filed::plugin::v1 as pb;

use crate::config::FdConfig;
use crate::secrets::{is_secret, MASK};

/// One `Plugin =` line: the plugin name and its options.
#[derive(Clone, PartialEq, Eq)]
pub struct PluginLine {
    pub name: String,
    pub options: BTreeMap<String, String>,
}

/// Debug output masks secret option values, so a line can be logged.
impl fmt::Debug for PluginLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let options: BTreeMap<&str, &str> = self
            .options
            .iter()
            .map(|(k, v)| (k.as_str(), if is_secret(k) { MASK } else { v.as_str() }))
            .collect();
        f.debug_struct("PluginLine").field("name", &self.name).field("options", &options).finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PluginError {
    /// The line is not `name[:key=value…]`.
    Syntax(String),
    /// No `Plugin Policy` or `Plugin Directory` is configured.
    NotConfigured,
    /// The name is not in `Plugin Names`.
    NotAllowed(String),
    /// A path fails the ownership and permission check.
    Untrusted { path: PathBuf, reason: String },
    /// Reading, verifying or starting the bundle failed; nothing ran.
    Load(String),
    /// A call into a running plugin failed (trap, timeout, refused reply).
    Runtime(String),
    /// The job was canceled while the plugin worked.
    Canceled,
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(why) => write!(f, "invalid plugin line: {why}"),
            Self::NotConfigured => write!(f, "no Plugin Policy and Plugin Directory configured"),
            Self::NotAllowed(name) => write!(f, "plugin \"{name}\" is not in Plugin Names"),
            Self::Untrusted { path, reason } => write!(f, "{}: {reason}", path.display()),
            Self::Load(why) => write!(f, "cannot load: {why}"),
            Self::Runtime(why) => write!(f, "{why}"),
            Self::Canceled => write!(f, "canceled"),
        }
    }
}

const MAX_NAME: usize = 64;
const MAX_OPTIONS: usize = 64;
const MAX_LINE: usize = 8192;

/// Splits a plugin line at unescaped `:`; `\:` and `\\` stand for `:` and `\`.
pub fn parse_line(text: &str) -> Result<PluginLine, PluginError> {
    if text.len() > MAX_LINE {
        return Err(PluginError::Syntax("line too long".into()));
    }
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next @ (':' | '\\')) => current.push(next),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => return Err(PluginError::Syntax("line ends in a backslash".into())),
            },
            ':' => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    parts.push(current);
    let mut parts = parts.into_iter();
    let name = parts.next().unwrap_or_default();
    let name_ok = !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'));
    if !name_ok {
        return Err(PluginError::Syntax("the name must be 1 to 64 characters of [a-z0-9_-]".into()));
    }
    let mut options = BTreeMap::new();
    for part in parts {
        let Some((key, value)) = part.split_once('=') else {
            return Err(PluginError::Syntax(format!("option without '=' after \"{name}\"")));
        };
        let key_ok = !key.is_empty()
            && key.len() <= MAX_NAME
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
        if !key_ok {
            return Err(PluginError::Syntax("an option name must be 1 to 64 characters of [A-Za-z0-9_.-]".into()));
        }
        if options.insert(key.to_string(), value.to_string()).is_some() {
            return Err(PluginError::Syntax(format!("option \"{key}\" given twice")));
        }
        if options.len() > MAX_OPTIONS {
            return Err(PluginError::Syntax("too many options".into()));
        }
    }
    Ok(PluginLine { name, options })
}

/// Paths of a plugin that passed every check before loading.
#[derive(Debug, PartialEq, Eq)]
pub struct Resolved {
    pub bundle: PathBuf,
    pub policy: PathBuf,
}

/// Bundle and policy for `line`, after the configuration and ownership checks.
/// Called for every job: nothing is cached between jobs.
pub fn resolve(config: &FdConfig, line: &PluginLine) -> Result<Resolved, PluginError> {
    resolve_checked(config, line, 0, Path::new("/"))
}

/// `resolve` with the owner and the top of the walk as parameters, so tests
/// can check a tree of their own. Production always passes root and `/`.
fn resolve_checked(config: &FdConfig, line: &PluginLine, owner: u32, top: &Path) -> Result<Resolved, PluginError> {
    let (Some(policy), Some(directory)) = (&config.plugin_policy, &config.plugin_directory) else {
        return Err(PluginError::NotConfigured);
    };
    if !config.plugin_names.contains(&line.name) {
        return Err(PluginError::NotAllowed(line.name.clone()));
    }
    let bundle = directory.join(format!("{}.pb", line.name));
    check_trusted(policy, owner, false, top)?;
    // The CA files the policy names decide whom to trust, like the policy.
    for ca in policy_ca_files(policy)? {
        check_trusted(&ca, owner, false, top)?;
    }
    check_trusted(directory, owner, true, top)?;
    check_trusted(&bundle, owner, false, top)?;
    Ok(Resolved { bundle, policy: policy.clone() })
}

/// The CA certificate files a policy names, resolved like `bundle::read_policy`
/// does (relative to the policy's directory).
fn policy_ca_files(policy: &Path) -> Result<Vec<PathBuf>, PluginError> {
    let untrusted = |reason: String| PluginError::Untrusted { path: policy.to_path_buf(), reason };
    let bytes = onesimus_plugins::bundle::read_bounded(policy, 1024 * 1024).map_err(|e| untrusted(e.to_string()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| untrusted("invalid policy JSON".into()))?;
    let base = policy.parent().unwrap_or(Path::new("/"));
    value
        .get("ca_certificates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| untrusted("no ca_certificates".into()))?
        .iter()
        .map(|v| v.as_str().map(|s| base.join(s)).ok_or_else(|| untrusted("ca_certificates holds a non-string".into())))
        .collect()
}

/// `path` and every directory above it up to `top` (as written and after
/// resolving links) are owned by `owner` and not writable by group or others.
pub(crate) fn check_trusted(path: &Path, owner: u32, directory: bool, top: &Path) -> Result<(), PluginError> {
    let untrusted = |p: &Path, reason: String| PluginError::Untrusted { path: p.to_path_buf(), reason };
    if !path.is_absolute() {
        return Err(untrusted(path, "path is not absolute".into()));
    }
    let real = std::fs::canonicalize(path).map_err(|e| untrusted(path, e.to_string()))?;
    let meta = std::fs::metadata(&real).map_err(|e| untrusted(&real, e.to_string()))?;
    if directory != meta.is_dir() || (!directory && !meta.is_file()) {
        let want = if directory { "a directory" } else { "a regular file" };
        return Err(untrusted(&real, format!("is not {want}")));
    }
    for p in path.ancestors().chain(real.ancestors()).filter(|p| p.starts_with(top)) {
        // As written: a link's own directory must be trusted too.
        let meta = std::fs::symlink_metadata(p).map_err(|e| untrusted(p, e.to_string()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.uid() != owner {
            return Err(untrusted(p, format!("owned by uid {}, not {owner}", meta.uid())));
        }
        if meta.mode() & 0o022 != 0 {
            return Err(untrusted(p, format!("writable by group or others (mode {:o})", meta.mode() & 0o7777)));
        }
    }
    Ok(())
}

/// Most plugin instances one File Daemon runs at the same time.
const MAX_INSTANCES: usize = 16;
/// How often a waiting call looks at the job's cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(100);
/// Longest single plugin operation (one NextFile, one block of ReadFile).
const OPERATION_TIMEOUT: Duration = Duration::from_secs(300);

/// The admission limit shared by every job of this process.
fn host() -> Result<&'static Host, PluginError> {
    static HOST: OnceLock<Result<Host, String>> = OnceLock::new();
    HOST.get_or_init(|| Host::new(MAX_INSTANCES).map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| PluginError::Load(e.clone()))
}

/// One running plugin instance, bound to one job and one `Plugin =` line.
/// Dropping it stops the worker; `finish` reports the job result first.
pub struct Session {
    worker: Worker<WasmBackend>,
    binding: Binding,
    job: pb::JobContext,
    pub line: PluginLine,
    pub plugin_id: String,
    /// Largest block ReadFile/WriteFile may carry.
    pub chunk: u32,
    started: bool,
}

impl Session {
    /// Checks, verifies and starts the plugin of `line` for this job.
    /// `index` numbers the plugin lines of the job (instance identity).
    pub fn open(config: &FdConfig, line: &PluginLine, job: pb::JobContext, index: usize, chunk: u32)
        -> Result<Self, PluginError> {
        let resolved = resolve(config, line)?;
        let runtime = RuntimeLimits { max_chunk_bytes: chunk, ..RuntimeLimits::default() };
        Self::open_resolved(&resolved, line, job, index, runtime)
    }

    pub(crate) fn open_resolved(resolved: &Resolved, line: &PluginLine, job: pb::JobContext, index: usize, runtime: RuntimeLimits)
        -> Result<Self, PluginError> {
        let load = |e: &dyn fmt::Display| PluginError::Load(e.to_string());
        let chunk = runtime.max_chunk_bytes;
        let bundle = bundle::read_bundle(&resolved.bundle, runtime.max_component_bytes).map_err(|e| load(&e))?;
        let policy = bundle::read_policy(&resolved.policy).map_err(|e| load(&e))?;
        let manifest: onesimus_plugins::Manifest =
            serde_json::from_slice(&bundle.manifest_bytes).map_err(|e| load(&e))?;
        let binding = Binding {
            instance_id: format!("{}-{index}-{}", job.job_id, line.name),
            job_id: job.job_id,
            device_id: format!("plugin:{}", line.name),
            worker_id: format!("ofd-{}-{index}", job.job_id),
        };
        let overhead = 64 * 1024;
        let worker = wasm::load_signed(host()?, bundle, &policy, LoadOptions {
            binding: binding.clone(),
            job: job.clone(),
            options: line.options.clone(),
            worker: Limits {
                max_pending_operations: 1,
                max_request_bytes: chunk as usize + overhead,
                max_response_bytes: chunk as usize + overhead,
                operation_timeout: OPERATION_TIMEOUT,
            },
            runtime,
        })
        .map_err(|e| load(&e))?;
        Ok(Self { worker, binding, job, line: line.clone(), plugin_id: manifest.plugin_id, chunk, started: false })
    }

    /// One call, waiting until it answers or `canceled()` turns true.
    fn call(&self, op: Operation, canceled: &dyn Fn() -> bool) -> Result<Reply, PluginError> {
        let ticket = self.worker.submit(&self.binding, op).map_err(|e| PluginError::Runtime(e.to_string()))?;
        loop {
            match ticket.wait_timeout(CANCEL_POLL) {
                Ok(reply) => return reply.map_err(|e| PluginError::Runtime(e.to_string())),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if canceled() {
                        self.worker.cancel();
                        return Err(PluginError::Canceled);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(PluginError::Runtime("plugin worker stopped".into()));
                }
            }
        }
    }

    fn event(&self, event: pb::event_request::Event, canceled: &dyn Fn() -> bool) -> Result<(), PluginError> {
        self.call(Operation::Event(pb::EventRequest {
            instance_id: self.binding.instance_id.clone(),
            job: Some(self.job.clone()),
            event: Some(event),
        }), canceled)
        .map(drop)
    }

    /// JobStarted: from here on file operations are allowed.
    pub fn start(&mut self, canceled: &dyn Fn() -> bool) -> Result<(), PluginError> {
        self.event(pb::event_request::Event::Started(pb::JobStarted {}), canceled)?;
        self.started = true;
        Ok(())
    }

    /// The next file of a backup, `None` when the plugin has no more.
    pub fn next_file(&self, canceled: &dyn Fn() -> bool) -> Result<Option<pb::FileEntry>, PluginError> {
        let op = Operation::NextFile(pb::NextFileRequest {
            instance_id: self.binding.instance_id.clone(),
            job_id: self.job.job_id,
        });
        match self.call(op, canceled)? {
            Reply::NextFile(pb::NextFileResponse { item: Some(pb::next_file_response::Item::File(f)), .. }) => Ok(Some(f)),
            Reply::NextFile(pb::NextFileResponse { item: Some(pb::next_file_response::Item::Complete(_)), .. }) => Ok(None),
            _ => Err(PluginError::Runtime("unexpected reply to NextFile".into())),
        }
    }

    /// Up to `chunk` bytes of `path` from `offset`; `true` at the end.
    pub fn read(&self, path: &[u8], offset: u64, canceled: &dyn Fn() -> bool) -> Result<(Vec<u8>, bool), PluginError> {
        let op = Operation::ReadFile(pb::ReadFileRequest {
            instance_id: self.binding.instance_id.clone(),
            job_id: self.job.job_id,
            path: path.to_vec(),
            offset,
            max_bytes: self.chunk,
        });
        match self.call(op, canceled)? {
            Reply::ReadFile(r) => Ok((r.data, r.eof)),
            _ => Err(PluginError::Runtime("unexpected reply to ReadFile".into())),
        }
    }

    /// One block of a file during an online restore.
    pub fn write(&self, file: &pb::FileEntry, offset: u64, data: &[u8], canceled: &dyn Fn() -> bool)
        -> Result<(), PluginError> {
        self.call(Operation::WriteFile(pb::WriteFileRequest {
            instance_id: self.binding.instance_id.clone(),
            job_id: self.job.job_id,
            file: Some(file.clone()),
            offset,
            data: data.to_vec(),
        }), canceled)
        .map(drop)
    }

    /// Completes (or with `abort` discards) one restored file.
    pub fn close(&self, path: &[u8], abort: bool, canceled: &dyn Fn() -> bool) -> Result<(), PluginError> {
        self.call(Operation::CloseFile(pb::CloseFileRequest {
            instance_id: self.binding.instance_id.clone(),
            job_id: self.job.job_id,
            path: path.to_vec(),
            abort,
        }), canceled)
        .map(drop)
    }

    /// Reports the end of the job to the plugin and stops the worker.
    pub fn finish(mut self, succeeded: bool) -> Result<(), PluginError> {
        let never = || false;
        let reported = if self.started {
            let event = if succeeded {
                pb::event_request::Event::Finished(pb::JobFinished { succeeded })
            } else {
                pb::event_request::Event::Cancelled(pb::JobCancelled { reason: "job failed or canceled".into() })
            };
            self.event(event, &never)
        } else {
            Ok(())
        };
        let stopped = self.worker.shutdown(Duration::from_secs(5)).map_err(|e| PluginError::Runtime(e.to_string()));
        reported.and(stopped)
    }
}

#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn line_names_options_and_escapes() {
        let l = parse_line("postgresql:instance=main:databases=*:exclude=test_*").unwrap();
        assert_eq!(l.name, "postgresql");
        assert_eq!(l.options["databases"], "*");
        let l = parse_line(r"openldap:uri=ldaps\://h\:636:bind_dn=cn=a,dc=b:password=x\\y").unwrap();
        assert_eq!(l.options["uri"], "ldaps://h:636");
        assert_eq!(l.options["bind_dn"], "cn=a,dc=b");
        assert_eq!(l.options["password"], r"x\y");
        assert_eq!(parse_line("postgresql").unwrap().options.len(), 0);
    }

    #[test]
    fn bad_lines_are_refused() {
        for bad in ["", "Postgres", "../x", "pg:noequals", "pg:=v", "pg:a=1:a=2", "pg:k=v\\", "pg:bad key=v"] {
            assert!(matches!(parse_line(bad), Err(PluginError::Syntax(_))), "{bad}");
        }
    }

    #[test]
    fn debug_output_masks_secrets() {
        let l = parse_line("pg:user=bob:password=hunter2:db_pass=x").unwrap();
        let shown = format!("{l:?}");
        assert!(!shown.contains("hunter2") && shown.contains("bob") && shown.contains(MASK));
    }

    fn dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("ofd-plugin-test-{}-{n}", std::process::id()));
        std::fs::create_dir(&d).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        d
    }
    fn mode(p: &Path, mode: u32) {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
    }
    /// A complete trusted tree: policy, plugin directory, one bundle.
    fn tree() -> (PathBuf, FdConfig) {
        let d = dir();
        std::fs::create_dir(d.join("plugins")).unwrap();
        mode(&d.join("plugins"), 0o755);
        std::fs::write(d.join("policy.json"), br#"{"ca_certificates":["ca.pem"]}"#).unwrap();
        mode(&d.join("policy.json"), 0o644);
        std::fs::write(d.join("ca.pem"), b"ca").unwrap();
        mode(&d.join("ca.pem"), 0o644);
        std::fs::write(d.join("plugins/postgresql.pb"), b"x").unwrap();
        mode(&d.join("plugins/postgresql.pb"), 0o644);
        let mut c = FdConfig::parse("Director { Name = d; Password = p }").unwrap();
        c.plugin_directory = Some(d.join("plugins"));
        c.plugin_policy = Some(d.join("policy.json"));
        c.plugin_names = vec!["postgresql".into()];
        (d, c)
    }
    fn me() -> u32 {
        // SAFETY: geteuid has no preconditions and cannot fail.
        unsafe { libc::geteuid() }
    }

    #[test]
    fn configuration_and_names_are_checked_first() {
        let (d, mut c) = tree();
        let line = parse_line("postgresql").unwrap();
        assert!(resolve_checked(&c, &line, me(), &d).is_ok());
        assert_eq!(resolve_checked(&c, &parse_line("openldap").unwrap(), me(), &d),
            Err(PluginError::NotAllowed("openldap".into())));
        c.plugin_policy = None;
        assert_eq!(resolve_checked(&c, &line, me(), &d), Err(PluginError::NotConfigured));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn writable_or_foreign_parts_are_refused() {
        let line = parse_line("postgresql").unwrap();
        let untrusted = |r: Result<Resolved, PluginError>, part: &str| {
            matches!(r, Err(PluginError::Untrusted { path, .. }) if path.ends_with(part))
        };
        let (d, c) = tree();
        assert_eq!(resolve_checked(&c, &line, me(), &d).unwrap().bundle, d.join("plugins/postgresql.pb"));
        // Someone else's tree: wrong owner.
        assert!(matches!(resolve_checked(&c, &line, me() + 1, &d), Err(PluginError::Untrusted { .. })));
        for (part, bad) in [("policy.json", 0o664), ("ca.pem", 0o666), ("plugins/postgresql.pb", 0o646), ("plugins", 0o775)] {
            mode(&d.join(part), bad);
            assert!(untrusted(resolve_checked(&c, &line, me(), &d), part), "{part} {bad:o}");
            mode(&d.join(part), if part == "plugins" { 0o755 } else { 0o644 });
        }
        // The top itself is part of the walk.
        mode(&d, 0o777);
        assert!(matches!(resolve_checked(&c, &line, me(), &d), Err(PluginError::Untrusted { .. })));
        mode(&d, 0o755);
        // A missing bundle, a directory where the policy should be, a relative path.
        std::fs::remove_file(d.join("plugins/postgresql.pb")).unwrap();
        assert!(untrusted(resolve_checked(&c, &line, me(), &d), "postgresql.pb"));
        assert!(matches!(check_trusted(&d.join("plugins"), me(), false, &d),
            Err(PluginError::Untrusted { reason, .. }) if reason.contains("regular file")));
        assert!(matches!(check_trusted(Path::new("relative/p"), me(), false, &d), Err(PluginError::Untrusted { .. })));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_ca_file_outside_the_trusted_tree_is_refused() {
        let (d, c) = tree();
        let line = parse_line("postgresql").unwrap();
        std::fs::create_dir(d.join("open")).unwrap();
        mode(&d.join("open"), 0o777);
        std::fs::write(d.join("open/evil.pem"), b"x").unwrap();
        std::fs::write(d.join("policy.json"), br#"{"ca_certificates":["ca.pem","open/evil.pem"]}"#).unwrap();
        assert!(matches!(resolve_checked(&c, &line, me(), &d), Err(PluginError::Untrusted { path, .. }) if path.ends_with("open")));
        std::fs::write(d.join("policy.json"), b"{}").unwrap();
        assert!(matches!(resolve_checked(&c, &line, me(), &d), Err(PluginError::Untrusted { reason, .. }) if reason.contains("ca_certificates")));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_link_is_checked_as_written_and_as_resolved() {
        let (d, mut c) = tree();
        let line = parse_line("postgresql").unwrap();
        // Policy reached through a link in a writable directory: refused.
        std::fs::create_dir(d.join("open")).unwrap();
        mode(&d.join("open"), 0o777);
        std::os::unix::fs::symlink(d.join("policy.json"), d.join("open/policy.json")).unwrap();
        // CA paths resolve next to the policy as named, here through a link too.
        std::os::unix::fs::symlink(d.join("ca.pem"), d.join("open/ca.pem")).unwrap();
        c.plugin_policy = Some(d.join("open/policy.json"));
        assert!(matches!(resolve_checked(&c, &line, me(), &d), Err(PluginError::Untrusted { path, .. }) if path.ends_with("open")));
        mode(&d.join("open"), 0o755);
        assert!(resolve_checked(&c, &line, me(), &d).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn production_walks_to_the_root() {
        // Below a world-writable /tmp nothing passes, whoever owns it.
        let (d, c) = tree();
        assert!(matches!(resolve(&c, &parse_line("postgresql").unwrap()), Err(PluginError::Untrusted { .. })));
        std::fs::remove_dir_all(&d).unwrap();
        // Root-owned system paths pass.
        assert_eq!(check_trusted(Path::new("/usr/lib"), 0, true, Path::new("/")), Ok(()));
    }

    mod session {
        use super::super::testkit::*;
        use super::super::*;
        use std::time::Instant;

        /// A fresh job id per call: the host refuses two instances with the
        /// same identity, as two jobs never share one.
        fn job(op: pb::JobOperation) -> pb::JobContext {
            use std::sync::atomic::{AtomicU64, Ordering};
            static ID: AtomicU64 = AtomicU64::new(1000);
            pb::JobContext { job_id: ID.fetch_add(1, Ordering::Relaxed), job_name: "pg-test".into(), operation: op as i32 }
        }
        fn limits() -> RuntimeLimits {
            RuntimeLimits { max_chunk_bytes: 1024, ..RuntimeLimits::default() }
        }
        fn open_with(script: &Script, op: pb::JobOperation, runtime: RuntimeLimits) -> (PathBuf, Result<Session, PluginError>) {
            let d = super::dir();
            let resolved = signed_plugin(&d, "postgresql", script);
            let line = parse_line("postgresql:instance=main:password=secret").unwrap();
            let s = Session::open_resolved(&resolved, &line, job(op), 0, runtime);
            (d, s)
        }
        fn open(script: &Script, op: pb::JobOperation) -> (PathBuf, Result<Session, PluginError>) {
            open_with(script, op, limits())
        }
        const NEVER: fn() -> bool = || false;

        #[test]
        fn backup_lists_and_reads_files() {
            let script = Script {
                next: vec![file_step(b"@postgresql/main/db/a/a.dump", 5), complete_step()],
                read: vec![data_step(b"hello", true)],
                ..Default::default()
            };
            let (d, s) = open(&script, pb::JobOperation::Backup);
            let mut s = s.unwrap();
            assert_eq!(s.plugin_id, "test.postgresql");
            s.start(&NEVER).unwrap();
            let f = s.next_file(&NEVER).unwrap().unwrap();
            assert_eq!(f.path, b"@postgresql/main/db/a/a.dump");
            assert_eq!(s.read(&f.path, 0, &NEVER).unwrap(), (b"hello".to_vec(), true));
            assert_eq!(s.next_file(&NEVER).unwrap(), None);
            s.finish(true).unwrap();
            std::fs::remove_dir_all(&d).unwrap();
        }

        #[test]
        fn restore_writes_and_closes() {
            let (d, s) = open(&Script::default(), pb::JobOperation::Restore);
            let mut s = s.unwrap();
            s.start(&NEVER).unwrap();
            let f = pb::FileEntry { path: b"x".to_vec(), kind: pb::FileKind::Virtual as i32, ..Default::default() };
            s.write(&f, 0, b"data", &NEVER).unwrap();
            s.close(b"x", false, &NEVER).unwrap();
            // A backup call in a restore job is refused by the host.
            assert!(matches!(s.next_file(&NEVER), Err(PluginError::Runtime(_))));
            let _ = s.finish(false);
            std::fs::remove_dir_all(&d).unwrap();
        }

        #[test]
        fn a_changed_bundle_is_refused_before_any_guest_code_runs() {
            let d = super::dir();
            let resolved = signed_plugin(&d, "postgresql", &Script::default());
            let mut bytes = std::fs::read(&resolved.bundle).unwrap();
            let last = bytes.len() - 1;
            bytes[last] ^= 0x01;
            std::fs::write(&resolved.bundle, &bytes).unwrap();
            let line = parse_line("postgresql").unwrap();
            assert!(matches!(Session::open_resolved(&resolved, &line, job(pb::JobOperation::Backup), 0, limits()),
                Err(PluginError::Load(_))));
            std::fs::remove_dir_all(&d).unwrap();
        }

        #[test]
        fn an_endless_loop_runs_out_of_fuel() {
            let script = Script { next: vec![Step::Loop], ..Default::default() };
            let (d, s) = open(&script, pb::JobOperation::Backup);
            let mut s = s.unwrap();
            s.start(&NEVER).unwrap();
            assert!(matches!(s.next_file(&NEVER), Err(PluginError::Runtime(_))));
            let _ = s.finish(false);
            std::fs::remove_dir_all(&d).unwrap();
        }

        #[test]
        fn a_trap_ends_only_this_session() {
            let script = Script { next: vec![Step::Trap], ..Default::default() };
            let (d, s) = open(&script, pb::JobOperation::Backup);
            let mut s = s.unwrap();
            s.start(&NEVER).unwrap();
            assert!(matches!(s.next_file(&NEVER), Err(PluginError::Runtime(_))));
            let _ = s.finish(false);
            // A second session of the same process still works.
            let ok = Script { next: vec![complete_step()], ..Default::default() };
            let (d2, s2) = open(&ok, pb::JobOperation::Backup);
            let mut s2 = s2.unwrap();
            s2.start(&NEVER).unwrap();
            assert_eq!(s2.next_file(&NEVER).unwrap(), None);
            s2.finish(true).unwrap();
            std::fs::remove_dir_all(&d).unwrap();
            std::fs::remove_dir_all(&d2).unwrap();
        }

        #[test]
        fn cancel_stops_a_hanging_plugin() {
            // Enough fuel that the loop runs until the cancel, not out of fuel.
            let script = Script { next: vec![Step::Loop], ..Default::default() };
            let runtime = RuntimeLimits { fuel_per_operation: u64::MAX / 4, ..limits() };
            let (d, s) = open_with(&script, pb::JobOperation::Backup, runtime);
            let mut s = s.unwrap();
            s.start(&NEVER).unwrap();
            let begin = Instant::now();
            let cancel = move || begin.elapsed() > Duration::from_millis(300);
            assert_eq!(s.next_file(&cancel), Err(PluginError::Canceled));
            assert!(begin.elapsed() < Duration::from_secs(5));
            let _ = s.finish(false);
            std::fs::remove_dir_all(&d).unwrap();
        }
    }
}

/// End to end with the real Rust guest `plugins/example-tree`: backup into a
/// BNET stream, the stream as the SD returns it, restore to files and online.
/// Build the guest first (see plugins/example-tree/README.md), then
/// `cargo test -p ofd -- --ignored example_tree`.
#[cfg(test)]
mod guest {
    use super::testkit::signed_artifact;
    use super::*;
    use crate::backup::{Backup, JobEnv, SdSender};
    use crate::find::FfPkt;
    use crate::jmsg::JobMsgs;
    use crate::restore::{Counters, Restore, RestoreParams};
    use bareos_bnet::BnetStream;
    use std::io::{Cursor, Read, Write};
    use std::sync::atomic::AtomicBool;

    fn guest_wasm() -> Vec<u8> {
        let path = std::env::var_os("ONESIMUS_EXAMPLE_TREE_WASM").map(PathBuf::from).unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/plugins/wasm32-wasip2/release/onesimus_plugin_example_tree.wasm")
        });
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}; build plugins/example-tree first", path.display()))
    }

    fn session(dir: &Path, op: pb::JobOperation, job_id: u64) -> Session {
        let resolved = signed_artifact(dir, "example", guest_wasm());
        let line = parse_line("example:instance=main:password=geheim").unwrap();
        let job = pb::JobContext { job_id, job_name: "example".into(), operation: op as i32 };
        let runtime = RuntimeLimits { max_chunk_bytes: 64 * 1024, ..RuntimeLimits::default() };
        Session::open_resolved(&resolved, &line, job, 0, runtime).unwrap()
    }

    /// FD->SD frames ("<fi> <stream> 0", data, EOD) as the SD sends them back
    /// on restore ("rechdr <vsid> <vst> <fi> <stream> <len>", data).
    fn as_read_back(written: &[u8]) -> Vec<u8> {
        let mut frames = Vec::new();
        let mut b = written;
        while b.len() >= 4 {
            let n = i32::from_be_bytes(b[..4].try_into().unwrap());
            b = &b[4..];
            if n < 0 {
                frames.push(None);
            } else {
                frames.push(Some(b[..n as usize].to_vec()));
                b = &b[n as usize..];
            }
        }
        let mut out = Vec::new();
        let mut put = |m: &[u8]| {
            out.extend_from_slice(&(m.len() as i32).to_be_bytes());
            out.extend_from_slice(m);
        };
        let mut i = 0;
        while i < frames.len() {
            let Some(header) = frames[i].clone() else { i += 1; continue };
            let text = String::from_utf8(header).unwrap();
            let mut f = text.split(' ');
            let (fi, stream) = (f.next().unwrap().to_string(), f.next().unwrap().to_string());
            i += 1;
            while let Some(Some(data)) = frames.get(i) {
                put(format!("rechdr 1 2 {fi} {stream} {}", data.len()).as_bytes());
                put(data);
                i += 1;
            }
        }
        out.extend_from_slice(&(bareos_bnet::BnetSignal::Eod as i32).to_be_bytes());
        out
    }

    struct Duplex(Cursor<Vec<u8>>);
    impl Read for Duplex {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(b)
        }
    }
    impl Write for Duplex {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    #[ignore = "needs the built plugins/example-tree guest"]
    fn example_tree_backs_up_and_restores() {
        let dir = std::env::temp_dir().join(format!("ofd-example-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("restore")).unwrap();
        let never = || false;

        // The guest lists its tree; the large file comes in several blocks.
        let mut s = session(&dir, pb::JobOperation::Backup, 9001);
        s.start(&never).unwrap();
        let mut names = Vec::new();
        while let Some(f) = s.next_file(&never).unwrap() {
            assert_eq!(f.attributes.get("instance").map(String::as_str), Some("main"));
            names.push(f.path);
        }
        assert_eq!(names.len(), 5);
        assert!(names.iter().any(|n| n.ends_with(b"latin1-\xE9")));
        s.finish(true).unwrap();

        // Backup through the real send path.
        let config = crate::config::FdConfig::parse("Director { Name = d; Password = p }").unwrap();
        let canceled = AtomicBool::new(false);
        let mut sd = BnetStream::new(Cursor::new(Vec::new()));
        let mut msgs = JobMsgs::new("test-fd");
        let mut counters = Counters::default();
        let mut flush = |_: &mut JobMsgs| {};
        let shown = crate::secrets::mask_plugin_secrets("example:instance=main:password=geheim");
        let ok = Backup {
            sd: SdSender { sd: &mut sd, msgs: &mut msgs, counters: &mut counters, canceled: &canceled, progress: None },
            env: JobEnv { canceled: &canceled, accurate: None, level_full: true, rerunning: false, base_size: 0 },
            flush: &mut flush,
            buf_size: 64 * 1024,
            workers: 0,
            acl_errors: 0,
            xattr_errors: 0,
            xattr_disabled: false,
            config: &config,
            plugin_index: 0,
        }
        .run_plugin(&FfPkt::default(), &shown, session(&dir, pb::JobOperation::Backup, 9002));
        assert_eq!(ok, 1, "{:?}", msgs.take_queued());
        assert_eq!(counters.job_files, 5);
        let volume = sd.into_inner().into_inner();
        assert!(!volume.windows(6).any(|w| w == b"geheim"), "a secret reached the volume");

        // Restore to files, without the plugin.
        let restore = |where_: Vec<u8>, options: &[String], hook: bool| {
            let mut sd = BnetStream::new(Duplex(Cursor::new(as_read_back(&volume))));
            let params = RestoreParams { where_: crate::attribs::Where::Prefix(where_), replace: b'a', prefix_links: false };
            let mut msgs = JobMsgs::new("test-fd");
            msgs.job_id = 9003;
            let mut counters = Counters::default();
            let mut sent = Vec::new();
            if hook {
                let d = dir.clone();
                crate::restore::test_hook::set(Box::new(move |_line, job, index| {
                    let resolved = signed_artifact(&d, "example", guest_wasm());
                    let line = parse_line("example:instance=main").unwrap();
                    let runtime = RuntimeLimits { max_chunk_bytes: 64 * 1024, ..RuntimeLimits::default() };
                    Session::open_resolved(&resolved, &line, job, index, runtime)
                }));
            }
            Restore { params: &params, msgs: &mut msgs, counters: &mut counters,
                create: &mut crate::create_file::CreateState::default(), canceled: &canceled,
                config: Some(&config), plugin_options: options }
                .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
            (msgs.status(), counters, sent)
        };
        let target = dir.join("restore");
        let (status, c, sent) = restore(target.as_os_str().as_encoded_bytes().to_vec(), &[], false);
        assert_eq!(status, crate::jmsg::JS_TERMINATED, "{sent:?}");
        assert_eq!(c.job_files, 5);
        let base = target.join("@example/main");
        let big = std::fs::read(base.join("big.bin")).unwrap();
        assert_eq!(big.len(), 300 * 1024);
        assert!(big.iter().enumerate().all(|(i, b)| *b == (i % 251) as u8));
        assert_eq!(std::fs::read(base.join("small.txt")).unwrap(), b"hello from the example plugin\n");
        assert_eq!(std::fs::read(base.join("empty")).unwrap(), b"");
        use std::os::unix::ffi::OsStrExt;
        let odd = base.join(std::ffi::OsStr::from_bytes(b"latin1-\xE9"));
        assert_eq!(std::fs::read(odd).unwrap(), b"latin-1 name");

        // Online restore through the guest.
        let (status, c, sent) = restore(Vec::new(), &["example:restore_mode=online".to_string()], true);
        assert_eq!(status, crate::jmsg::JS_TERMINATED, "{sent:?}");
        assert_eq!(c.job_files, 5);
        assert!(sent.iter().any(|m| m.contains("restoring online through test.example")), "{sent:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
