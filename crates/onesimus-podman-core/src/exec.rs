//! The fixed argument lists (docs/plans/PODMAN-PLUGIN.md, "Broker und
//! Rootful-Backend", operation table).
//!
//! # Purpose
//!
//! Neither the broker nor the rootful backend in `ofd` runs anything but
//! the operations of the plan's table, each with a fixed argument list:
//! no shell, no generic exec, never `--showsecret`, never
//! `system reset|migrate|renumber`. Names and IDs that enter an argument
//! list are checked ([`valid_name`], [`valid_unit`]) so no value can turn
//! into an option. The rootful backend writes every argument list it runs
//! into its exec protocol; [`classify`] maps such a list back to its table
//! row, and a list that fits no row is a test failure (exit criterion P0
//! (6)).
//!
//! # Data flow
//!
//! request (host decision) → [`Operation`] → [`Operation::argv`] with the
//! domain's [`Storage`] → `Command` with an empty environment → exec
//! protocol → [`classify`] in tests.
//!
//! # Counterpart
//!
//! `onesimus-podman-broker` (rootless, metadata rows only) and
//! `ofd::podman_rootful` (rootful, every row) build their commands only
//! through [`Operation::argv`].

use std::path::PathBuf;

/// The properties of `systemctl show` for `unit-info` (V-6): no
/// `Environment`, `ExecStart*`, `ExecStop*`, credentials.
pub const UNIT_PROPERTIES: &[&str] = &[
    "Id", "Names", "SourcePath", "FragmentPath", "DropInPaths", "LoadState", "ActiveState", "SubState", "UnitFileState",
    "InvocationID", "ActiveEnterTimestampMonotonic", "Result", "ExecMainCode", "ExecMainStatus", "NRestarts", "Restart",
    "TimeoutStopUSec", "KillMode", "Requires", "Wants", "BindsTo", "PartOf", "After", "Before", "WantedBy",
    "EnvironmentFiles",
];

/// The Podman program of the certified profile.
pub const PODMAN: &str = "/usr/bin/podman";
/// The Quadlet generator.
pub const QUADLET: &str = "/usr/libexec/podman/quadlet";
/// systemctl.
pub const SYSTEMCTL: &str = "/usr/bin/systemctl";
/// systemd-run.
pub const SYSTEMD_RUN: &str = "/usr/bin/systemd-run";

/// Storage options of a domain from the account policy (non-default
/// locations only; an empty value leaves Podman's default).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Storage {
    /// `--root`.
    pub root: Option<PathBuf>,
    /// `--runroot`.
    pub runroot: Option<PathBuf>,
    /// `--tmpdir`.
    pub tmpdir: Option<PathBuf>,
}

impl Storage {
    /// The global Podman options for this storage.
    fn args(&self) -> Vec<String> {
        let mut a = Vec::new();
        for (flag, v) in [("--root", &self.root), ("--runroot", &self.runroot), ("--tmpdir", &self.tmpdir)] {
            if let Some(p) = v {
                a.push(flag.to_string());
                a.push(p.display().to_string());
            }
        }
        a
    }
}

/// Inspect kinds (`podman <kind> inspect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// container.
    Container,
    /// pod.
    Pod,
    /// network.
    Network,
    /// volume.
    Volume,
    /// image.
    Image,
    /// secret (metadata; never --showsecret).
    Secret,
}

impl Kind {
    /// The Podman object word.
    fn word(self) -> &'static str {
        match self {
            Self::Container => "container",
            Self::Pod => "pod",
            Self::Network => "network",
            Self::Volume => "volume",
            Self::Image => "image",
            Self::Secret => "secret",
        }
    }
}

/// One row of the operation table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operation {
    /// engine-identify: `podman info --format json`.
    Info,
    /// engine-identify: `podman version --format json`.
    Version,
    /// inventory: `podman ps -a --format json`.
    Containers,
    /// inventory: `podman pod ps --format json`.
    Pods,
    /// inventory: `podman network ls --format json`.
    Networks,
    /// inventory: `podman volume ls --format json`.
    Volumes,
    /// inventory: `podman images --format json`.
    Images,
    /// inventory: `podman secret ls --format json`.
    Secrets,
    /// inspect(kind, ids).
    Inspect(Kind, Vec<String>),
    /// `podman generate spec <id>` (container or pod).
    GenerateSpec(String),
    /// quadlet-resolve: `quadlet -dryrun [-user]`.
    QuadletDryrun {
        /// rootless (`-user`).
        user: bool,
    },
    /// unit-info: `systemctl [--user] show -p <fixed list> <units>`.
    UnitShow {
        /// `--user`.
        user: bool,
        /// Units.
        units: Vec<String>,
    },
    /// lifecycle: `systemctl [--user] stop <unit>`.
    UnitStop {
        /// `--user`.
        user: bool,
        /// Unit.
        unit: String,
    },
    /// lifecycle: `systemctl [--user] mask --runtime <unit>`.
    UnitMaskRuntime {
        /// `--user`.
        user: bool,
        /// Unit.
        unit: String,
    },
    /// lifecycle: `systemctl [--user] unmask --runtime <unit>`.
    UnitUnmaskRuntime {
        /// `--user`.
        user: bool,
        /// Unit.
        unit: String,
    },
    /// lifecycle: `systemctl [--user] start <unit>`.
    UnitStart {
        /// `--user`.
        user: bool,
        /// Unit.
        unit: String,
    },
    /// lifecycle: `podman stop -t <seconds> <id>` (container without unit).
    ContainerStop {
        /// Seconds.
        timeout: u64,
        /// Container id.
        id: String,
    },
    /// lifecycle: `systemd-run [--user] --wait --collect podman start <id>`.
    ContainerStart {
        /// `--user`.
        user: bool,
        /// Container id.
        id: String,
    },
    /// The dead man's switch: a transient timer that runs the recovery.
    DeadMan {
        /// `<domain>/<jobid>`, checked.
        key: String,
        /// Seconds until it fires (`max_downtime`).
        seconds: u64,
    },
    /// Removes the dead man's switch: `systemctl stop <unit>.timer`.
    DeadManCancel {
        /// The timer unit.
        unit: String,
    },
}

/// A Podman name or ID: `[A-Za-z0-9][A-Za-z0-9_.-]{0,254}` (also sha256 IDs).
pub fn valid_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 255 && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

/// A systemd unit name of the kinds Podman uses.
pub fn valid_unit(s: &str) -> bool {
    let b = s.as_bytes();
    let kind_ok = [".service", ".target", ".timer", ".socket"].iter().any(|k| s.ends_with(k));
    kind_ok && b.len() <= 255 && b[0].is_ascii_alphanumeric()
        && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-' | b'@' | b':' | b'\\'))
}

/// A dead man's key `<domain>/<jobid>`: `root` or `uid-<n>`, a slash, digits.
pub fn valid_key(s: &str) -> bool {
    let Some((domain, job)) = s.split_once('/') else { return false };
    let domain_ok = domain == "root" || domain.strip_prefix("uid-").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    domain_ok && !job.is_empty() && job.len() <= 20 && job.bytes().all(|b| b.is_ascii_digit())
}

/// The unit name of the dead man's switch of `key`.
pub fn deadman_unit(key: &str) -> String {
    format!("onesimus-podman-recover-{}", key.replace('/', "-"))
}

/// Strings of a fixed list.
fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| x.to_string()).collect()
}

impl Operation {
    /// The complete argument list, program first; Err if a value fails
    /// its check (it could otherwise turn into an option).
    pub fn argv(&self, storage: &Storage) -> Result<Vec<String>, String> {
        let podman = |rest: &[&str]| {
            let mut a = vec![PODMAN.to_string()];
            a.extend(storage.args());
            a.extend(s(rest));
            a
        };
        let names = |ids: &[String]| -> Result<(), String> {
            if ids.is_empty() || ids.len() > 256 || ids.iter().any(|i| !valid_name(i)) {
                Err("invalid or empty object list".into())
            } else {
                Ok(())
            }
        };
        let unit = |u: &str| if valid_unit(u) { Ok(()) } else { Err(format!("invalid unit \"{}\"", u.escape_default())) };
        let systemctl = |user: bool, rest: Vec<String>| {
            let mut a = vec![SYSTEMCTL.to_string()];
            if user {
                a.push("--user".into());
            }
            a.extend(rest);
            a
        };
        Ok(match self {
            Self::Info => podman(&["info", "--format", "json"]),
            Self::Version => podman(&["version", "--format", "json"]),
            Self::Containers => podman(&["ps", "-a", "--format", "json"]),
            Self::Pods => podman(&["pod", "ps", "--format", "json"]),
            Self::Networks => podman(&["network", "ls", "--format", "json"]),
            Self::Volumes => podman(&["volume", "ls", "--format", "json"]),
            Self::Images => podman(&["images", "--format", "json"]),
            Self::Secrets => podman(&["secret", "ls", "--format", "json"]),
            Self::Inspect(kind, ids) => {
                names(ids)?;
                let mut a = podman(&[kind.word(), "inspect", "--format", "json", "--"]);
                a.extend(ids.iter().cloned());
                a
            }
            Self::GenerateSpec(id) => {
                names(std::slice::from_ref(id))?;
                podman(&["generate", "spec", "--compact", "--", id])
            }
            Self::QuadletDryrun { user } => {
                let mut a = s(&[QUADLET, "-dryrun"]);
                if *user {
                    a.push("-user".into());
                }
                a
            }
            Self::UnitShow { user, units } => {
                if units.is_empty() || units.len() > 64 {
                    return Err("invalid unit list".into());
                }
                units.iter().try_for_each(|u| unit(u))?;
                let mut rest = vec!["show".to_string(), "-p".to_string(), UNIT_PROPERTIES.join(","), "--".to_string()];
                rest.extend(units.iter().cloned());
                systemctl(*user, rest)
            }
            Self::UnitStop { user, unit: u } => {
                unit(u)?;
                systemctl(*user, s(&["stop", "--", u]))
            }
            Self::UnitMaskRuntime { user, unit: u } => {
                unit(u)?;
                systemctl(*user, s(&["mask", "--runtime", "--", u]))
            }
            Self::UnitUnmaskRuntime { user, unit: u } => {
                unit(u)?;
                systemctl(*user, s(&["unmask", "--runtime", "--", u]))
            }
            Self::UnitStart { user, unit: u } => {
                unit(u)?;
                systemctl(*user, s(&["start", "--", u]))
            }
            Self::ContainerStop { timeout, id } => {
                names(std::slice::from_ref(id))?;
                podman(&["stop", "-t", &timeout.to_string(), "--", id])
            }
            Self::ContainerStart { user, id } => {
                names(std::slice::from_ref(id))?;
                let mut a = vec![SYSTEMD_RUN.to_string()];
                if *user {
                    a.push("--user".into());
                }
                a.extend(s(&["--wait", "--collect", "--"]));
                a.extend(podman(&["start", id]));
                a
            }
            Self::DeadMan { key, seconds } => {
                if !valid_key(key) || *seconds == 0 {
                    return Err("invalid dead man's switch".into());
                }
                s(&[SYSTEMD_RUN, &format!("--unit={}", deadman_unit(key)), &format!("--on-active={seconds}"),
                    "--timer-property=AccuracySec=1s", "/usr/sbin/ofd", "--podman-recover", key])
            }
            Self::DeadManCancel { unit: u } => {
                if !u.starts_with("onesimus-podman-recover-") || !valid_unit(u) || !u.ends_with(".timer") {
                    return Err("not a dead man's switch".into());
                }
                s(&[SYSTEMCTL, "stop", "--", u])
            }
        })
    }
}

/// The table row an argument list belongs to, by rebuilding it from its
/// parsed values; None if it fits no row (exit criterion P0 (6)).
pub fn classify(argv: &[String]) -> Option<&'static str> {
    let prog = argv.first()?.as_str();
    let rest = &argv[1..];
    // Recognise and strip the storage options of podman.
    let (storage, body) = if prog == PODMAN {
        let mut st = Storage::default();
        let mut i = 0;
        while i + 1 < rest.len() {
            let v = Some(PathBuf::from(&rest[i + 1]));
            match rest[i].as_str() {
                "--root" if st.root.is_none() => st.root = v,
                "--runroot" if st.runroot.is_none() => st.runroot = v,
                "--tmpdir" if st.tmpdir.is_none() => st.tmpdir = v,
                _ => break,
            }
            i += 2;
        }
        (st, &rest[i..])
    } else {
        (Storage::default(), rest)
    };
    let b: Vec<&str> = body.iter().map(String::as_str).collect();
    let tail = |n: usize| body.get(n..).map(<[String]>::to_vec).unwrap_or_default();
    let user = b.first() == Some(&"--user");
    let ub = if user { &b[1..] } else { &b[..] };
    let utail = |n: usize| tail(n + usize::from(user));
    let candidates: Vec<(&'static str, Operation)> = match (prog, b.as_slice()) {
        (PODMAN, ["info", ..]) => vec![("engine-identify", Operation::Info)],
        (PODMAN, ["version", ..]) => vec![("engine-identify", Operation::Version)],
        (PODMAN, ["ps", ..]) => vec![("inventory", Operation::Containers)],
        (PODMAN, ["pod", "ps", ..]) => vec![("inventory", Operation::Pods)],
        (PODMAN, ["network", "ls", ..]) => vec![("inventory", Operation::Networks)],
        (PODMAN, ["volume", "ls", ..]) => vec![("inventory", Operation::Volumes)],
        (PODMAN, ["images", ..]) => vec![("inventory", Operation::Images)],
        (PODMAN, ["secret", "ls", ..]) => vec![("inventory", Operation::Secrets)],
        (PODMAN, [kind, "inspect", ..]) => {
            let k = [Kind::Container, Kind::Pod, Kind::Network, Kind::Volume, Kind::Image, Kind::Secret]
                .into_iter().find(|k| k.word() == *kind)?;
            vec![("inspect", Operation::Inspect(k, tail(5)))]
        }
        (PODMAN, ["generate", "spec", "--compact", "--", id]) => vec![("inspect", Operation::GenerateSpec(id.to_string()))],
        (PODMAN, ["stop", "-t", t, "--", id]) => vec![("lifecycle", Operation::ContainerStop { timeout: t.parse().ok()?, id: id.to_string() })],
        (QUADLET, _) => vec![("quadlet-resolve", Operation::QuadletDryrun { user: b.contains(&"-user") })],
        (SYSTEMCTL, _) => match ub {
            ["show", ..] => vec![("unit-info", Operation::UnitShow { user, units: utail(4) })],
            ["stop", "--", u] if u.starts_with("onesimus-podman-recover-") && !user =>
                vec![("lifecycle", Operation::DeadManCancel { unit: u.to_string() })],
            ["stop", "--", u] => vec![("lifecycle", Operation::UnitStop { user, unit: u.to_string() })],
            ["mask", "--runtime", "--", u] => vec![("lifecycle", Operation::UnitMaskRuntime { user, unit: u.to_string() })],
            ["unmask", "--runtime", "--", u] => vec![("lifecycle", Operation::UnitUnmaskRuntime { user, unit: u.to_string() })],
            ["start", "--", u] => vec![("lifecycle", Operation::UnitStart { user, unit: u.to_string() })],
            _ => return None,
        },
        (SYSTEMD_RUN, _) => match ub {
            ["--wait", "--collect", "--", ..] => {
                let id = argv.last()?.to_string();
                vec![("lifecycle", Operation::ContainerStart { user, id })]
            }
            [unit, _, _, "/usr/sbin/ofd", "--podman-recover", key] if !user && unit.starts_with("--unit=") => {
                let seconds = b.get(1)?.strip_prefix("--on-active=")?.parse().ok()?;
                vec![("lifecycle", Operation::DeadMan { key: key.to_string(), seconds })]
            }
            _ => return None,
        },
        _ => return None,
    };
    candidates.into_iter().find_map(|(row, op)| {
        let st = if matches!(op, Operation::ContainerStart { .. }) { extract_storage(argv) } else { storage.clone() };
        (op.argv(&st).ok()?.as_slice() == argv).then_some(row)
    })
}

/// The storage options inside a `systemd-run … podman …` list.
fn extract_storage(argv: &[String]) -> Storage {
    let Some(pos) = argv.iter().position(|a| a == PODMAN) else { return Storage::default() };
    let mut st = Storage::default();
    let mut i = pos + 1;
    while i + 1 < argv.len() {
        let v = Some(PathBuf::from(&argv[i + 1]));
        match argv[i].as_str() {
            "--root" => st.root = v,
            "--runroot" => st.runroot = v,
            "--tmpdir" => st.tmpdir = v,
            _ => break,
        }
        i += 2;
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_ops() -> Vec<Operation> {
        let id = "abc123".to_string();
        let unit = "tapir-redis.service".to_string();
        vec![
            Operation::Info, Operation::Version, Operation::Containers, Operation::Pods, Operation::Networks,
            Operation::Volumes, Operation::Images, Operation::Secrets,
            Operation::Inspect(Kind::Container, vec![id.clone(), "web".into()]),
            Operation::Inspect(Kind::Secret, vec![id.clone()]),
            Operation::GenerateSpec(id.clone()),
            Operation::QuadletDryrun { user: true }, Operation::QuadletDryrun { user: false },
            Operation::UnitShow { user: true, units: vec![unit.clone()] },
            Operation::UnitStop { user: false, unit: unit.clone() },
            Operation::UnitMaskRuntime { user: true, unit: unit.clone() },
            Operation::UnitUnmaskRuntime { user: false, unit: unit.clone() },
            Operation::UnitStart { user: true, unit: unit.clone() },
            Operation::ContainerStop { timeout: 10, id: id.clone() },
            Operation::ContainerStart { user: true, id: id.clone() },
            Operation::DeadMan { key: "uid-1000/42".into(), seconds: 7200 },
            Operation::DeadManCancel { unit: "onesimus-podman-recover-uid-1000-42.timer".into() },
        ]
    }

    /// Every row's own argument list is recognised (also with storage).
    #[test]
    fn every_row_round_trips_through_classify() {
        let st = Storage { root: Some("/r".into()), runroot: Some("/rr".into()), tmpdir: Some("/t".into()) };
        for op in all_ops() {
            for storage in [Storage::default(), st.clone()] {
                let argv = op.argv(&storage).unwrap();
                assert!(classify(&argv).is_some(), "{argv:?}");
            }
        }
    }

    #[test]
    fn foreign_or_dangerous_lists_fit_no_row() {
        for bad in [
            vec![PODMAN, "system", "reset"],
            vec![PODMAN, "system", "migrate"],
            vec![PODMAN, "secret", "inspect", "--showsecret", "--format", "json", "--", "s"],
            vec![PODMAN, "secret", "inspect", "--format", "json", "--showsecret", "s"],
            vec![PODMAN, "ps", "-a", "--format", "json", "--filter", "x"],
            vec![PODMAN, "--root", "/r", "--root", "/s", "info", "--format", "json"],
            vec!["/bin/sh", "-c", "podman info"],
            vec![SYSTEMCTL, "unmask", "--", "x.service"],
            vec![SYSTEMCTL, "mask", "--", "x.service"],
            vec![PODMAN, "container", "inspect", "--format", "json", "--", "-x"],
        ] {
            let argv: Vec<String> = bad.iter().map(|s| s.to_string()).collect();
            assert_eq!(classify(&argv), None, "{argv:?}");
        }
    }

    #[test]
    fn values_never_become_options() {
        let st = Storage::default();
        assert!(Operation::Inspect(Kind::Container, vec!["--all".into()]).argv(&st).is_err());
        assert!(Operation::Inspect(Kind::Container, vec![]).argv(&st).is_err());
        assert!(Operation::UnitStop { user: false, unit: "-x.service".into() }.argv(&st).is_err());
        assert!(Operation::UnitStop { user: false, unit: "a;b.service".into() }.argv(&st).is_err());
        assert!(Operation::DeadMan { key: "../x/1".into(), seconds: 5 }.argv(&st).is_err());
        assert!(Operation::DeadMan { key: "root/1".into(), seconds: 0 }.argv(&st).is_err());
        assert!(Operation::DeadManCancel { unit: "sshd.service".into() }.argv(&st).is_err());
        assert_eq!(
            Operation::UnitMaskRuntime { user: true, unit: "web.service".into() }.argv(&st).unwrap(),
            [SYSTEMCTL, "--user", "mask", "--runtime", "--", "web.service"]
        );
        assert_eq!(deadman_unit("root/42"), "onesimus-podman-recover-root-42");
    }
}
