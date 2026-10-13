//! `-u <user>` / `-g <group>`: drop root privileges (like C++ lib/bsys drop()).
//!
//! Must run while the process is still single-threaded and before any
//! database connection: the catalog connects as the target user (peer
//! authentication, ~/.pgpass), and setenv() is not thread-safe.
//! Any failure is fatal — silently continuing as root is not an option.

use std::ffi::CString;

use nix::unistd::{Group, Uid, User};

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
            nix::unistd::initgroups(&cname, gid).map_err(|e| format!("initgroups({}): {e}", u.name))?;
        }
        None => nix::unistd::setgroups(&[gid]).map_err(|e| format!("setgroups: {e}"))?,
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

#[cfg(test)]
mod tests {
    use super::*;

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
