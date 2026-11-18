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
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(why) => write!(f, "invalid plugin line: {why}"),
            Self::NotConfigured => write!(f, "no Plugin Policy and Plugin Directory configured"),
            Self::NotAllowed(name) => write!(f, "plugin \"{name}\" is not in Plugin Names"),
            Self::Untrusted { path, reason } => write!(f, "{}: {reason}", path.display()),
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
    if !config.plugin_names.iter().any(|n| *n == line.name) {
        return Err(PluginError::NotAllowed(line.name.clone()));
    }
    let bundle = directory.join(format!("{}.pb", line.name));
    check_trusted(policy, owner, false, top)?;
    check_trusted(directory, owner, true, top)?;
    check_trusted(&bundle, owner, false, top)?;
    Ok(Resolved { bundle, policy: policy.clone() })
}

/// `path` and every directory above it up to `top` (as written and after
/// resolving links) are owned by `owner` and not writable by group or others.
fn check_trusted(path: &Path, owner: u32, directory: bool, top: &Path) -> Result<(), PluginError> {
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
        std::fs::write(d.join("policy.json"), b"{}").unwrap();
        mode(&d.join("policy.json"), 0o644);
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
        for (part, bad) in [("policy.json", 0o664), ("plugins/postgresql.pb", 0o646), ("plugins", 0o775)] {
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
    fn a_link_is_checked_as_written_and_as_resolved() {
        let (d, mut c) = tree();
        let line = parse_line("postgresql").unwrap();
        // Policy reached through a link in a writable directory: refused.
        std::fs::create_dir(d.join("open")).unwrap();
        mode(&d.join("open"), 0o777);
        std::os::unix::fs::symlink(d.join("policy.json"), d.join("open/policy.json")).unwrap();
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
}
