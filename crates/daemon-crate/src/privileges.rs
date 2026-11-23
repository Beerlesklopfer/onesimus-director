// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Rechteabgabe: `-u <Benutzer>` / `-g <Gruppe>`.
//!
//! Zweck: Ein Daemon startet als root (Port, Dateien) und wechselt danach
//! zu einem Dienstbenutzer. Zwei Fassungen:
//!
//! - [`drop_privileges`] (streng, aus `odir/src/privileges.rs` unverändert
//!   übernommen): ergänzende Gruppen, Gruppe, Benutzer; jeder Fehler ist ein
//!   `Err`, und root darf danach nicht zurückzuholen sein. Muss laufen,
//!   solange der Prozess nur einen Thread hat (setzt `HOME`/`USER`).
//! - [`drop_privileges_lenient`] (nachsichtig, ofds Verhalten vor dem
//!   19.11.2026): Gruppe, dann Benutzer; unbekannte Namen und Fehler werden
//!   übergangen. Offener Punkt: ofd auf die strenge Fassung umstellen.
//!
//! Datenfluss: `main` (vor Hintergrund-Start und Threads) → setgroups/setgid/
//! setuid → Rest des Daemons läuft als Dienstbenutzer.
//!
//! Bareos-Gegenstück: `lib/bsys.cc` `drop()`.

use std::ffi::CString;

use nix::unistd::{Group, Uid, User};

/// Strict privilege drop (odir, osd): `-u`/`-g` switch user and group;
/// any failure is an `Err` – silently continuing as root is not an option.
/// Must run while the process is single-threaded (it sets `HOME`, `USER`).
pub fn drop_privileges(user: Option<&str>, group: Option<&str>) -> Result<(), String> {
    if user.is_none() && group.is_none() {
        return Ok(());
    }

    let target_user = match user {
        Some(name) => Some(
            User::from_name(name)
                .map_err(|e| format!("cannot look up user {name}: {e}"))?
                .ok_or_else(|| format!("unknown user {name}"))?,
        ),
        None => None,
    };
    // Group: -g, else the user's primary group (C++: pw_gid)
    let gid = match group {
        Some(name) => Group::from_name(name)
            .map_err(|e| format!("cannot look up group {name}: {e}"))?
            .ok_or_else(|| format!("unknown group {name}"))?
            .gid,
        None => target_user.as_ref().map(|u| u.gid).unwrap_or_else(nix::unistd::getgid),
    };
    let uid = target_user.as_ref().map(|u| u.uid).unwrap_or_else(nix::unistd::getuid);

    if !nix::unistd::geteuid().is_root() {
        if uid == nix::unistd::getuid() && gid == nix::unistd::getgid() {
            return Ok(()); // already running as the requested user/group
        }
        return Err(format!(
            "must be started as root to switch to {}{}",
            user.map(|u| format!("user {u}")).unwrap_or_default(),
            group.map(|g| format!(" group {g}")).unwrap_or_default()
        ));
    }

    // Supplementary groups first (root's groups must not survive), then gid, then uid
    match &target_user {
        Some(u) => {
            let cname = CString::new(u.name.as_str()).map_err(|_| "invalid user name".to_string())?;
            init_groups(&cname, gid).map_err(|e| format!("initgroups({}): {e}", u.name))?;
        }
        None => set_groups(gid).map_err(|e| format!("setgroups: {e}"))?,
    }
    nix::unistd::setgid(gid).map_err(|e| format!("setgid({gid}): {e}"))?;
    nix::unistd::setuid(uid).map_err(|e| format!("setuid({uid}): {e}"))?;

    // Paranoia: root must not be recoverable
    if !uid.is_root() && nix::unistd::setuid(Uid::from_raw(0)).is_ok() {
        return Err("privileges could not be dropped (setuid(0) still possible)".to_string());
    }

    if let Some(u) = &target_user {
        // HOME/USER for libpq-style lookups (~/.pgpass) and child processes.
        // SAFETY: called before any thread is spawned.
        unsafe {
            std::env::set_var("HOME", &u.dir);
            std::env::set_var("USER", &u.name);
            std::env::set_var("LOGNAME", &u.name);
        }
    }
    Ok(())
}

/// Lenient privilege drop (ofd before 19.11.2026): `setgid` to `group`,
/// then `setuid` to `user`; unknown names and failures are ignored, a
/// successful switch is logged.
pub fn drop_privileges_lenient(user: Option<&str>, group: Option<&str>) {
    if let Some(group_name) = group
        && let Ok(Some(group)) = Group::from_name(group_name)
    {
        nix::unistd::setgid(group.gid).ok();
        tracing::info!("switched to group {group_name}");
    }
    if let Some(user_name) = user
        && let Ok(Some(user)) = User::from_name(user_name)
    {
        nix::unistd::setuid(user.uid).ok();
        tracing::info!("switched to user {user_name}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_ignores_unknown_names() {
        drop_privileges_lenient(Some("no-such-user-onesimus-test"), Some("no-such-group-onesimus-test"));
        drop_privileges_lenient(None, None);
    }

    #[test]
    fn nothing_requested_is_a_noop() {
        assert!(drop_privileges(None, None).is_ok());
    }

    #[test]
    fn unknown_user_is_an_error() {
        assert!(drop_privileges(Some("no-such-user-onesimus-test"), None).is_err());
    }

    #[test]
    fn switching_without_root_fails_clearly() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        // "root" exists everywhere; switching to it as non-root must fail
        let err = drop_privileges(Some("root"), None).unwrap_err();
        assert!(err.contains("must be started as root"), "{err}");
    }

    #[test]
    fn same_user_without_root_is_ok() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let me = User::from_uid(nix::unistd::getuid()).unwrap().unwrap();
        let primary = Group::from_gid(me.gid).unwrap().unwrap();
        assert!(drop_privileges(Some(&me.name), Some(&primary.name)).is_ok());
    }
}

/// `initgroups(3)`: nix offers it everywhere but on Apple targets, where libc
/// takes the group as `c_int`.
#[cfg(not(target_vendor = "apple"))]
fn init_groups(user: &CString, gid: nix::unistd::Gid) -> nix::Result<()> {
    nix::unistd::initgroups(user, gid)
}

/// `initgroups(3)` on macOS (libc directly).
#[cfg(target_vendor = "apple")]
fn init_groups(user: &CString, gid: nix::unistd::Gid) -> nix::Result<()> {
    // SAFETY: `user` is a valid NUL-terminated string for the call.
    let rc = unsafe { nix::libc::initgroups(user.as_ptr(), gid.as_raw() as nix::libc::c_int) };
    nix::errno::Errno::result(rc).map(drop)
}

/// `setgroups(2)` with only `gid`.
#[cfg(not(target_vendor = "apple"))]
fn set_groups(gid: nix::unistd::Gid) -> nix::Result<()> {
    nix::unistd::setgroups(&[gid])
}

/// `setgroups(2)` with only `gid` on macOS (libc directly).
#[cfg(target_vendor = "apple")]
fn set_groups(gid: nix::unistd::Gid) -> nix::Result<()> {
    let groups = [gid.as_raw()];
    // SAFETY: one valid gid_t in `groups`.
    let rc = unsafe { nix::libc::setgroups(1, groups.as_ptr()) };
    nix::errno::Errno::result(rc).map(drop)
}
