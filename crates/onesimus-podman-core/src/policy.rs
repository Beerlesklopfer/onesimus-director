// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The account policy (docs/plans/PODMAN-PLUGIN.md, "Kontopolicy").
//!
//! # Purpose
//!
//! Which accounts may be backed up is decided by the administrator in a
//! root-owned file next to the `Plugin Policy` (its own file because the
//! plugin policy rejects unknown fields). Per account: name and uid, the
//! allowed storage domains with their real `graphroot` (plus `runroot` and
//! `tmpdir`, which belong to the identity of a domain), `rootful: true`
//! only for root, `volume_bind_roots` (allowed `device` prefixes of
//! `o=bind` volumes) and the registries a restore may pull from. An account
//! only the discovery knows is not released.
//!
//! Location (proposal, open): [`DEFAULT_PATH`]; `ofd` checks it like the
//! plugin policy (root-owned, not writable by group or others, up to `/`).
//!
//! # Data flow
//!
//! file → [`AccountPolicy::parse`] (strict JSON, `deny_unknown_fields`) →
//! [`AccountPolicy::account`] for the line's `user=` → graphroot compared
//! with what the broker reports (`engine-identify`) and opened by the host
//! for the scan.
//!
//! # Counterpart
//!
//! The discovery (`onesimus_plugins::discovery`) lists candidates; only this
//! file releases them.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::line::Account;

/// Default location (proposal, open).
pub const DEFAULT_PATH: &str = "/etc/onesimus/podman-accounts.json";

/// Largest policy file.
pub const MAX_POLICY: usize = 256 * 1024;

/// One storage domain of an account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainEntry {
    /// Real path of the storage's graphroot.
    pub graphroot: PathBuf,
    /// runroot; default: Podman's for the account.
    #[serde(default)]
    pub runroot: Option<PathBuf>,
    /// libpod tmpdir; part of the domain identity.
    #[serde(default)]
    pub tmpdir: Option<PathBuf>,
}

/// One released account.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountEntry {
    /// Login name.
    pub name: String,
    /// Host uid.
    pub uid: u32,
    /// Storage domains (at least one).
    pub domains: Vec<DomainEntry>,
    /// The rootful profile; only for root.
    #[serde(default)]
    pub rootful: bool,
    /// Allowed `device=` prefixes of `o=bind` volumes.
    #[serde(default)]
    pub volume_bind_roots: Vec<PathBuf>,
    /// Registries a restore may pull from.
    #[serde(default)]
    pub registries: Vec<String>,
}

/// The whole policy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPolicy {
    /// Format marker, `onesimus-podman-accounts/1`.
    pub format: String,
    /// Released accounts.
    pub accounts: Vec<AccountEntry>,
}

/// The only accepted format marker.
pub const FORMAT: &str = "onesimus-podman-accounts/1";

/// An absolute, normal path (no `.`, `..`, no empty parts).
fn clean_absolute(p: &Path) -> bool {
    p.is_absolute() && p.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_))) && p.as_os_str().len() <= 4096
}

impl AccountPolicy {
    /// Parses and checks a policy.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_POLICY {
            return Err("account policy larger than 256 KiB".into());
        }
        let policy: Self = serde_json::from_slice(bytes).map_err(|e| format!("account policy: {e}"))?;
        if policy.format != FORMAT {
            return Err(format!("account policy: format must be {FORMAT}"));
        }
        let mut names = std::collections::BTreeSet::new();
        let mut uids = std::collections::BTreeSet::new();
        for a in &policy.accounts {
            let bad = |why: &str| Err(format!("account policy, account {}: {why}", a.name.escape_default()));
            if a.name.is_empty() || a.name.len() > 32 || !a.name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-')) {
                return bad("invalid name");
            }
            if !names.insert(a.name.clone()) || !uids.insert(a.uid) {
                return bad("name or uid listed twice");
            }
            if a.rootful != (a.uid == 0) || (a.uid == 0) != (a.name == "root") {
                return bad("rootful: true exactly for root (uid 0)");
            }
            if a.domains.is_empty() || a.domains.len() > 8 {
                return bad("needs 1 to 8 domains");
            }
            for d in &a.domains {
                let paths = std::iter::once(&d.graphroot).chain(d.runroot.as_ref()).chain(d.tmpdir.as_ref());
                if paths.into_iter().any(|p| !clean_absolute(p)) {
                    return bad("domain paths must be absolute and normal");
                }
            }
            if a.volume_bind_roots.iter().any(|p| !clean_absolute(p) || p == Path::new("/")) {
                return bad("volume_bind_roots must be absolute, normal and not /");
            }
        }
        Ok(policy)
    }

    /// The released account of a line's `user=`; None: not released (a
    /// plan error, even if the discovery knows the account).
    pub fn account(&self, user: &Account) -> Option<&AccountEntry> {
        self.accounts.iter().find(|a| match user {
            Account::Root => a.uid == 0,
            Account::Name(n) => &a.name == n,
            Account::Uid(u) => a.uid == *u,
        })
    }
}

impl AccountEntry {
    /// Whether `device` of an `o=bind` volume lies under an allowed root.
    pub fn bind_allowed(&self, device: &Path) -> bool {
        clean_absolute(device) && self.volume_bind_roots.iter().any(|r| device.starts_with(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"format":"onesimus-podman-accounts/1","accounts":[
        {"name":"root","uid":0,"rootful":true,"domains":[{"graphroot":"/var/lib/containers/storage"}]},
        {"name":"forgejo","uid":1001,"domains":[{"graphroot":"/home/forgejo/.local/share/containers/storage",
          "runroot":"/run/user/1001/containers","tmpdir":"/run/user/1001/libpod/tmp"}],
          "volume_bind_roots":["/srv/forgejo"],"registries":["ghcr.io"]}]}"#;

    #[test]
    fn released_accounts_by_name_uid_or_root() {
        let p = AccountPolicy::parse(GOOD.as_bytes()).unwrap();
        assert_eq!(p.account(&Account::Root).unwrap().name, "root");
        assert_eq!(p.account(&Account::Name("forgejo".into())).unwrap().uid, 1001);
        assert_eq!(p.account(&Account::Uid(1001)).unwrap().name, "forgejo");
        assert!(p.account(&Account::Name("www-data".into())).is_none(), "discovered only: not released");
        let f = p.account(&Account::Uid(1001)).unwrap();
        assert!(f.bind_allowed(Path::new("/srv/forgejo/data")));
        assert!(!f.bind_allowed(Path::new("/srv/forgejo/../etc")));
        assert!(!f.bind_allowed(Path::new("/srv/other")));
    }

    #[test]
    fn bad_policies_are_refused() {
        for bad in [
            GOOD.replace("onesimus-podman-accounts/1", "x"),
            GOOD.replace(r#""rootful":true,"#, ""),
            GOOD.replace(r#""uid":1001,"#, r#""uid":1001,"rootful":true,"#),
            GOOD.replace("\"forgejo\",\"uid\":1001", "\"forgejo\",\"uid\":0"),
            GOOD.replace("/var/lib/containers/storage", "var/lib"),
            GOOD.replace("/var/lib/containers/storage", "/var/lib/../etc"),
            GOOD.replace("\"/srv/forgejo\"", "\"/\""),
            GOOD.replace("\"registries\"", "\"extra\":1,\"registries\""),
            GOOD.replace(r#"[{"graphroot":"/var/lib/containers/storage"}]"#, "[]"),
        ] {
            assert!(AccountPolicy::parse(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
