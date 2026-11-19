//! The FileSet plugin line `podman:…` (docs/plans/PODMAN-PLUGIN.md,
//! "Parameter der Plugin-Zeile").
//!
//! # Purpose
//!
//! Selection and scope of one line, for exactly one account (user decision
//! 19.11.2026). Unknown keys are an error; without a selection
//! (`pods`/`containers`/`quadlet`) the line is a plan error, so an empty
//! line never backs up everything by accident. Decided defaults:
//! `env=omit`, `consistency_scope=job` with a mandatory `max_downtime`
//! (default 7200 s, `0` not allowed), `secrets=reference` only, no
//! `endpoint` in the MVP.
//!
//! # Data flow
//!
//! `ofd` `plugin::parse_line` (escapes, `key=value`) → options map →
//! [`PodmanLine::parse`] → checked against the account policy (`policy`)
//! and the grants (`required_grants`) before any broker starts.
//!
//! # Counterpart
//!
//! The guest receives the same options in `InitializeRequest.options` but
//! decides nothing that concerns security from them.

use std::collections::BTreeMap;

/// Default and upper bound of the downtime (V-4, user decision 19.11.2026).
pub const DEFAULT_MAX_DOWNTIME: u64 = 7200;

/// Every key the line may carry.
pub const KEYS: &[&str] = &[
    "user", "pods", "containers", "quadlet", "exclude", "volumes", "images", "networks", "quadlet_sources", "env",
    "consistency", "consistency_scope", "stop_timeout", "max_downtime", "clean_exit_codes", "on_unclean", "external",
    "secrets", "endpoint",
];

/// The account of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Account {
    /// `user=root`: the rootful profile (needs `podman.rootful`).
    Root,
    /// An account by name.
    Name(String),
    /// An account by numeric uid.
    Uid(u32),
}

/// `images=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Images {
    /// Inspect only, restore by pull.
    Reference,
    /// Config, manifests and layers as single files.
    Blobs,
}

/// `env=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Env {
    /// Names only (default).
    Omit,
    /// Values on the volume only.
    Keep,
}

/// `consistency=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consistency {
    /// Authorized stop of the writer group with journal.
    Stop,
    /// Copy while running, marked as weakened.
    Crash,
}

/// `consistency_scope=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Stop before the job's first file, restart after its last (default).
    Job,
    /// Only for this line's files.
    Plugin,
}

/// `on_unclean=` and `external=` share warn/fail semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A warning in the job log.
    Warn,
    /// A job error.
    Error,
}

/// A checked `podman:` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodmanLine {
    /// The account.
    pub user: Account,
    /// `pods=` patterns.
    pub pods: Vec<String>,
    /// `containers=` patterns.
    pub containers: Vec<String>,
    /// `quadlet=` patterns.
    pub quadlet: Vec<String>,
    /// `exclude=` patterns.
    pub exclude: Vec<String>,
    /// `volumes=yes|no`.
    pub volumes: bool,
    /// `images=`.
    pub images: Images,
    /// `networks=yes|no`.
    pub networks: bool,
    /// `quadlet_sources=yes|no`.
    pub quadlet_sources: bool,
    /// `env=`.
    pub env: Env,
    /// `consistency=`.
    pub consistency: Consistency,
    /// `consistency_scope=`.
    pub scope: Scope,
    /// `stop_timeout=` seconds, None: the unit's or container's own.
    pub stop_timeout: Option<u64>,
    /// `max_downtime=` seconds.
    pub max_downtime: u64,
    /// `clean_exit_codes=`.
    pub clean_exit_codes: Vec<i32>,
    /// `on_unclean=` (fail = Error).
    pub on_unclean: Severity,
    /// `external=`.
    pub external: Severity,
}

/// A list option: `,`-separated, trimmed, empty parts dropped.
fn list(options: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    options.get(key).map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

/// A name or glob: `[A-Za-z0-9_.*?-]{1,128}` (no path, no shell syntax).
fn valid_pattern(p: &str) -> bool {
    !p.is_empty() && p.len() <= 128 && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'*' | b'?'))
}

/// One of `choices` for `key`, or `default`.
fn choice<'a>(options: &'a BTreeMap<String, String>, key: &str, choices: &[&str], default: &'a str) -> Result<&'a str, String> {
    let v = options.get(key).map(String::as_str).unwrap_or(default);
    if choices.contains(&v) { Ok(v) } else { Err(format!("{key}={v}: allowed are {}", choices.join(", "))) }
}

impl PodmanLine {
    /// Checks the options of a `podman:` line.
    pub fn parse(options: &BTreeMap<String, String>) -> Result<Self, String> {
        if let Some(k) = options.keys().find(|k| !KEYS.contains(&k.as_str())) {
            return Err(format!("unknown key \"{k}\""));
        }
        if options.contains_key("endpoint") {
            return Err("endpoint= is not supported (remote comes in P4)".into());
        }
        let user = match options.get("user").map(String::as_str) {
            None | Some("") => return Err("user= is required (one line per account)".into()),
            Some("root") | Some("0") => Account::Root,
            Some(u) if u.bytes().all(|b| b.is_ascii_digit()) => Account::Uid(u.parse().map_err(|_| "user= uid too large")?),
            Some(u) if u.len() <= 32 && u.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
                && !u.starts_with('-') => Account::Name(u.into()),
            Some(u) => return Err(format!("user=\"{}\" is not an account name", u.escape_default())),
        };
        let (pods, containers, quadlet, exclude) =
            (list(options, "pods"), list(options, "containers"), list(options, "quadlet"), list(options, "exclude"));
        if pods.is_empty() && containers.is_empty() && quadlet.is_empty() {
            return Err("at least one of pods=, containers=, quadlet= is required".into());
        }
        if let Some(p) = pods.iter().chain(&containers).chain(&quadlet).chain(&exclude).find(|p| !valid_pattern(p)) {
            return Err(format!("\"{}\" is not a name or glob", p.escape_default()));
        }
        let yes = |key: &str| -> Result<bool, String> { Ok(choice(options, key, &["yes", "no"], "yes")? == "yes") };
        let consistency = match choice(options, "consistency", &["stop", "crash"], "stop")? {
            "stop" => Consistency::Stop,
            _ => Consistency::Crash,
        };
        let scope = match choice(options, "consistency_scope", &["job", "plugin"], "job")? {
            "job" => Scope::Job,
            _ => Scope::Plugin,
        };
        let seconds = |key: &str| -> Result<Option<u64>, String> {
            options.get(key).map(|v| v.parse::<u64>().map_err(|_| format!("{key}={v} is not a number of seconds"))).transpose()
        };
        let max_downtime = seconds("max_downtime")?.unwrap_or(DEFAULT_MAX_DOWNTIME);
        if max_downtime == 0 {
            return Err("max_downtime=0 (unlimited) is not allowed".into());
        }
        let clean_exit_codes = match options.get("clean_exit_codes") {
            None => vec![0, 143],
            Some(v) => v.split(',').map(|c| c.trim().parse::<i32>().map_err(|_| format!("clean_exit_codes: \"{c}\""))).collect::<Result<_, _>>()?,
        };
        choice(options, "secrets", &["reference"], "reference")?;
        Ok(Self {
            user,
            pods,
            containers,
            quadlet,
            exclude,
            volumes: yes("volumes")?,
            images: if choice(options, "images", &["reference", "blobs"], "reference")? == "blobs" { Images::Blobs } else { Images::Reference },
            networks: yes("networks")?,
            quadlet_sources: yes("quadlet_sources")?,
            env: if choice(options, "env", &["omit", "keep"], "omit")? == "keep" { Env::Keep } else { Env::Omit },
            consistency,
            scope,
            stop_timeout: seconds("stop_timeout")?,
            max_downtime,
            clean_exit_codes,
            on_unclean: if choice(options, "on_unclean", &["fail", "warn"], "fail")? == "warn" { Severity::Warn } else { Severity::Error },
            external: if choice(options, "external", &["warn", "error"], "warn")? == "error" { Severity::Error } else { Severity::Warn },
        })
    }

    /// The grants this line needs; a missing one is a plan error, never a
    /// silent switch to `crash` (V-5, V-11).
    pub fn required_grants(&self) -> Vec<&'static str> {
        let mut g = vec!["podman.inventory"];
        if self.volumes {
            g.push("podman.volume.read");
        }
        if self.images == Images::Blobs {
            g.push("podman.image.read");
        }
        if self.consistency == Consistency::Stop {
            g.push("podman.lifecycle");
        }
        if self.user == Account::Root {
            g.push("podman.rootful");
        }
        g
    }

    /// The first required grant missing from `granted`.
    pub fn missing_grant(&self, granted: &[String]) -> Option<&'static str> {
        self.required_grants().into_iter().find(|g| !granted.iter().any(|c| c == g))
    }
}

/// `*` any run, `?` one character; whole-name match.
pub fn glob(pattern: &str, name: &str) -> bool {
    /// Recursive matcher on bytes.
    fn m(p: &[u8], n: &[u8]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some(b'*'), _) => m(&p[1..], n) || (!n.is_empty() && m(p, &n[1..])),
            (Some(b'?'), Some(_)) => m(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => m(&p[1..], &n[1..]),
            _ => false,
        }
    }
    m(pattern.as_bytes(), name.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn decided_defaults() {
        let l = PodmanLine::parse(&opts(&[("user", "forgejo"), ("pods", "*")])).unwrap();
        assert_eq!(l.user, Account::Name("forgejo".into()));
        assert_eq!((l.env, l.scope, l.consistency, l.max_downtime), (Env::Omit, Scope::Job, Consistency::Stop, 7200));
        assert_eq!((l.volumes, l.images, l.networks), (true, Images::Reference, true));
        assert_eq!(l.clean_exit_codes, [0, 143]);
        assert_eq!(l.required_grants(), ["podman.inventory", "podman.volume.read", "podman.lifecycle"]);
    }

    #[test]
    fn plan_examples_parse() {
        let l = PodmanLine::parse(&opts(&[("user", "1001"), ("containers", "keycloak,postgres"), ("images", "blobs")])).unwrap();
        assert_eq!((l.user, l.containers.len()), (Account::Uid(1001), 2));
        let l = PodmanLine::parse(&opts(&[("user", "root"), ("quadlet", "tapir-*"), ("images", "blobs"), ("max_downtime", "3600")])).unwrap();
        assert_eq!(l.user, Account::Root);
        assert!(l.required_grants().contains(&"podman.rootful"));
        assert_eq!(l.missing_grant(&["podman.inventory".into(), "podman.volume.read".into()]), Some("podman.image.read"));
    }

    #[test]
    fn plan_errors() {
        for bad in [
            vec![("pods", "*")],
            vec![("user", "a")],
            vec![("user", "a"), ("pods", "*"), ("colour", "x")],
            vec![("user", "a"), ("pods", "*"), ("endpoint", "tcp://x")],
            vec![("user", "a"), ("pods", "*"), ("max_downtime", "0")],
            vec![("user", "a"), ("pods", "*"), ("env", "plain")],
            vec![("user", "a"), ("pods", "*"), ("secrets", "content")],
            vec![("user", "a"), ("pods", "../x")],
            vec![("user", "-a"), ("pods", "*")],
            vec![("user", "a b"), ("pods", "*")],
            vec![("user", "a"), ("pods", "*"), ("clean_exit_codes", "0,x")],
        ] {
            assert!(PodmanLine::parse(&opts(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn globs() {
        assert!(glob("tapir-*", "tapir-redis"));
        assert!(glob("*", ""));
        assert!(glob("a?c", "abc") && !glob("a?c", "ac"));
        assert!(!glob("tapir-*", "xtapir-a"));
    }
}
