// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! A local slapd instance and the account it runs as
//! (docs/plans/OPENLDAP-PLUGIN.md, "Instanz erkennen"; L-11).
//!
//! The broker runs as the user slapd runs as. That user is read from the
//! running process, never from plugin options:
//! - a process with `comm == "slapd"` (one; several need `config_path=`),
//! - real and effective uid/gid equal, uid not 0 (slapd without `-u`),
//! - the configuration from its command line (`-F <dir>` slapd.d, `-f <file>`
//!   slapd.conf); without either, slapd's built-in default as the Debian
//!   unit relies on: `/etc/ldap/slapd.d` if present, else `/etc/ldap/slapd.conf`,
//! - the configuration path absolute, plain, not a link; slapd.d owned by the
//!   instance user, slapd.conf by root or the instance user, neither
//!   writable by others.
//!
//! The database directories are checked later against the broker's info
//! (ldap_port.rs), by ofd as root.

use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use crate::broker::RunAs;
use crate::plugin::PluginError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigPath {
    /// slapd.d (cn=config)
    Dir(PathBuf),
    /// slapd.conf
    File(PathBuf),
}

impl ConfigPath {
    pub fn path(&self) -> &Path {
        match self {
            Self::Dir(p) | Self::File(p) => p,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub pid: i32,
    pub run_as: RunAs,
    pub config: ConfigPath,
}

fn plan_error(path: &Path, reason: impl Into<String>) -> PluginError {
    PluginError::Untrusted { path: path.to_path_buf(), reason: reason.into() }
}

fn plain_absolute(path: &Path) -> bool {
    path.is_absolute() && path.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

/// The `-F`/`-f` argument of a slapd command line (`-F dir` or `-Fdir`).
pub fn config_flag(cmdline: &[u8]) -> Option<ConfigPath> {
    let args: Vec<String> = cmdline.split(|&b| b == 0).filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned()).collect();
    let mut found = None;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        for (flag, dir) in [("-F", true), ("-f", false)] {
            if let Some(rest) = a.strip_prefix(flag) {
                let value = if rest.is_empty() { it.next().cloned() } else { Some(rest.to_string()) };
                if let Some(v) = value {
                    found = Some(if dir { ConfigPath::Dir(v.into()) } else { ConfigPath::File(v.into()) });
                }
            }
        }
    }
    found
}

/// Real and effective uid and gid from /proc/<pid>/status; None when they differ.
fn ids(status: &str) -> Option<(u32, u32)> {
    let field = |name: &str| -> Option<Vec<u32>> {
        status.lines().find_map(|l| l.strip_prefix(name)).map(|l| l.split_whitespace().filter_map(|v| v.parse().ok()).collect())
    };
    let (uid, gid) = (field("Uid:")?, field("Gid:")?);
    match (uid.as_slice(), gid.as_slice()) {
        ([ru, eu, ..], [rg, eg, ..]) if ru == eu && rg == eg => Some((*ru, *rg)),
        _ => None,
    }
}

/// The running slapd and its configuration, from the real /proc and the
/// Debian default directory /etc/ldap. `selector`: `config_path=` of the
/// plugin line, needed when several slapd run.
pub fn discover(selector: Option<&str>) -> Result<Instance, PluginError> {
    discover_in(Path::new("/proc"), Path::new("/etc/ldap"), selector)
}

/// `discover` over a given proc tree and default directory (tests).
pub fn discover_in(proc_root: &Path, default_dir: &Path, selector: Option<&str>) -> Result<Instance, PluginError> {
    let entries = std::fs::read_dir(proc_root).map_err(|e| plan_error(proc_root, e.to_string()))?;
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let dir = entry.path();
        if std::fs::read_to_string(dir.join("comm")).map(|c| c.trim() == "slapd").unwrap_or(false) {
            found.push((pid, dir));
        }
    }
    found.sort();
    if found.is_empty() {
        return Err(plan_error(proc_root, "no slapd is running (the broker takes its user from the running process)"));
    }
    let mut candidates = Vec::new();
    for (pid, dir) in &found {
        let cmdline = std::fs::read(dir.join("cmdline")).map_err(|e| plan_error(dir, e.to_string()))?;
        let config = match config_flag(&cmdline) {
            Some(c) => c,
            None if default_dir.join("slapd.d").is_dir() => ConfigPath::Dir(default_dir.join("slapd.d")),
            None => ConfigPath::File(default_dir.join("slapd.conf")),
        };
        candidates.push((*pid, dir.clone(), config));
    }
    let (pid, dir, config) = match selector {
        Some(want) => candidates.into_iter().find(|(_, _, c)| c.path() == Path::new(want))
            .ok_or_else(|| plan_error(Path::new(want), "no running slapd uses this configuration"))?,
        None if candidates.len() == 1 => candidates.remove(0),
        None => {
            let list: Vec<String> = candidates.iter().map(|(p, _, c)| format!("{p}: {}", c.path().display())).collect();
            return Err(plan_error(proc_root, format!("several slapd run ({}); choose one with config_path=", list.join(", "))));
        }
    };
    let status = std::fs::read_to_string(dir.join("status")).map_err(|e| plan_error(&dir, e.to_string()))?;
    let (uid, gid) = ids(&status).ok_or_else(|| plan_error(&dir, "real and effective uid or gid differ"))?;
    if uid == 0 {
        return Err(plan_error(&dir, "slapd runs as root; start it with -u <user> (the broker never runs as root)"));
    }
    let path = config.path();
    if !plain_absolute(path) {
        return Err(plan_error(path, "configuration path is not absolute and plain"));
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| plan_error(path, e.to_string()))?;
    if meta.file_type().is_symlink() {
        return Err(plan_error(path, "configuration path is a symbolic link"));
    }
    if meta.mode() & 0o002 != 0 {
        return Err(plan_error(path, "configuration is writable by others"));
    }
    match &config {
        ConfigPath::Dir(_) if !meta.is_dir() => return Err(plan_error(path, "-F names no directory")),
        ConfigPath::Dir(_) if meta.uid() != uid => {
            return Err(plan_error(path, format!("owned by uid {}, slapd runs as uid {uid}", meta.uid())));
        }
        ConfigPath::File(_) if !meta.is_file() => return Err(plan_error(path, "-f names no regular file")),
        ConfigPath::File(_) if meta.uid() != uid && meta.uid() != 0 => {
            return Err(plan_error(path, format!("owned by uid {}, neither root nor the slapd user {uid}", meta.uid())));
        }
        _ => {}
    }
    Ok(Instance { pid, run_as: RunAs { uid, gid }, config })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me() -> (u32, u32) {
        // SAFETY: getuid/getgid have no preconditions.
        unsafe { (libc::getuid(), libc::getgid()) }
    }
    struct Fake {
        root: PathBuf,
    }
    impl Fake {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!("ofd-ldap-discover-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("proc")).unwrap();
            std::fs::create_dir_all(root.join("etc/ldap")).unwrap();
            Fake { root }
        }
        fn process(&self, pid: i32, comm: &str, uid: u32, gid: u32, args: &[&str]) {
            let d = self.root.join("proc").join(pid.to_string());
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(d.join("status"), format!("Name:\t{comm}\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\nGid:\t{gid}\t{gid}\t{gid}\t{gid}\n")).unwrap();
            let mut cmd = Vec::new();
            for a in std::iter::once("/usr/sbin/slapd").chain(args.iter().copied()) {
                cmd.extend_from_slice(a.as_bytes());
                cmd.push(0);
            }
            std::fs::write(d.join("cmdline"), cmd).unwrap();
        }
        fn discover(&self, selector: Option<&str>) -> Result<Instance, PluginError> {
            discover_in(&self.root.join("proc"), &self.root.join("etc/ldap"), selector)
        }
        fn path(&self, p: &str) -> PathBuf {
            self.root.join(p)
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn the_command_line_names_the_configuration() {
        let c = |args: &[&str]| {
            let mut v = Vec::new();
            for a in args {
                v.extend_from_slice(a.as_bytes());
                v.push(0);
            }
            config_flag(&v)
        };
        assert_eq!(c(&["slapd", "-h", "ldapi:///", "-u", "openldap", "-F", "/etc/ldap/slapd.d"]), Some(ConfigPath::Dir("/etc/ldap/slapd.d".into())));
        assert_eq!(c(&["slapd", "-f/etc/ldap/slapd.conf"]), Some(ConfigPath::File("/etc/ldap/slapd.conf".into())));
        assert_eq!(c(&["slapd", "-h", "ldap:/// ldapi:///", "-u", "openldap", "-g", "openldap"]), None);
    }

    #[test]
    fn finds_slapd_and_its_user_with_the_default_configuration() {
        let f = Fake::new("default");
        let (uid, gid) = me();
        f.process(10, "bash", uid, gid, &[]);
        f.process(11, "slapd", uid, gid, &["-h", "ldapi:///", "-u", "openldap"]);
        // No slapd.d: slapd.conf is the default.
        std::fs::write(f.path("etc/ldap/slapd.conf"), "database mdb\n").unwrap();
        let i = f.discover(None).unwrap();
        assert_eq!((i.pid, i.run_as, i.config.clone()), (11, RunAs { uid, gid }, ConfigPath::File(f.path("etc/ldap/slapd.conf"))));
        // slapd.d comes before slapd.conf.
        std::fs::create_dir(f.path("etc/ldap/slapd.d")).unwrap();
        assert_eq!(f.discover(None).unwrap().config, ConfigPath::Dir(f.path("etc/ldap/slapd.d")));
    }

    #[test]
    fn foreign_root_or_mismatched_instances_are_refused() {
        let (uid, gid) = me();
        // No slapd at all (only another process).
        let f = Fake::new("none");
        f.process(10, "postgres", uid, gid, &[]);
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("no slapd")));
        // slapd as root.
        let f = Fake::new("root");
        std::fs::create_dir(f.path("etc/ldap/slapd.d")).unwrap();
        f.process(12, "slapd", 0, 0, &[]);
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("runs as root")));
        // Configuration owned by someone else.
        let f = Fake::new("owner");
        std::fs::create_dir(f.path("etc/ldap/slapd.d")).unwrap();
        f.process(13, "slapd", uid + 1, gid, &[]);
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("owned by")));
        // Relative -F, a link as -F, -F naming a file.
        let f = Fake::new("paths");
        f.process(14, "slapd", uid, gid, &["-F", "etc/ldap/slapd.d"]);
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("absolute")));
        std::fs::create_dir(f.path("etc/ldap/real.d")).unwrap();
        std::os::unix::fs::symlink(f.path("etc/ldap/real.d"), f.path("etc/ldap/link.d")).unwrap();
        f.process(14, "slapd", uid, gid, &["-F", f.path("etc/ldap/link.d").to_str().unwrap()]);
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("link")));
        std::fs::write(f.path("etc/ldap/file"), "").unwrap();
        f.process(14, "slapd", uid, gid, &["-F", f.path("etc/ldap/file").to_str().unwrap()]);
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("no directory")));
        // Differing real and effective uid.
        let d = f.path("proc/14/status");
        std::fs::write(&d, format!("Uid:\t{uid}\t0\t0\t0\nGid:\t{gid}\t{gid}\t{gid}\t{gid}\n")).unwrap();
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("differ")));
    }

    #[test]
    fn several_slapd_need_a_selector() {
        let f = Fake::new("several");
        let (uid, gid) = me();
        std::fs::create_dir(f.path("etc/ldap/a.d")).unwrap();
        std::fs::create_dir(f.path("etc/ldap/b.d")).unwrap();
        let (a, b) = (f.path("etc/ldap/a.d"), f.path("etc/ldap/b.d"));
        f.process(20, "slapd", uid, gid, &["-F", a.to_str().unwrap()]);
        f.process(21, "slapd", uid, gid, &["-F", b.to_str().unwrap()]);
        assert!(matches!(f.discover(None), Err(PluginError::Untrusted { reason, .. }) if reason.contains("config_path")));
        assert_eq!(f.discover(Some(b.to_str().unwrap())).unwrap().pid, 21);
        assert!(f.discover(Some("/etc/ldap/other.d")).is_err());
    }
}
