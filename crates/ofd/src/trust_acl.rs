// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Decision of the plugin trust check on Windows: who may change a file.
//!
//! Zweck: Auf Unix gilt für Richtlinie, CA-Dateien, Plugin-Verzeichnis und
//! Bundle (und jedes Verzeichnis darüber): Eigentümer root, nicht schreibbar
//! für Gruppe oder Andere (wie `StrictModes` von sshd), siehe
//! `plugin::check_trusted`. Windows kennt keine Modusbits, sondern einen
//! Besitzer-SID und eine DACL aus erlaubenden und verweigernden Einträgen.
//! Dieses Modul beantwortet die Frage "darf ein anderer als SYSTEM,
//! Administratoren oder TrustedInstaller das Objekt verändern?" rein aus
//! diesen Daten. Es enthält keinen Windows-Aufruf und läuft daher auch im
//! Linux-Test; `win_trust` liest die Beschreiber und fragt dieses Modul.
//!
//! Datenfluss: `win_trust::check` → [`Descriptor`] je Pfadbestandteil →
//! [`verdict`] → Ablehnungsgrund oder `Ok`.
//!
//! Regeln (alle schließen im Zweifel aus):
//! - Der Besitzer ist SYSTEM, Administratoren oder TrustedInstaller. Der
//!   Besitzer darf die DACL immer ändern; ein anderer Besitzer macht jede
//!   DACL wertlos.
//! - Eine NULL-DACL gibt jedem alles: abgelehnt. Ein Eintrag, der weder
//!   erlaubt noch verweigert (Callback-, Objekt-, Audit-Typ), ist abgelehnt.
//! - Kein anderer Principal darf [`LEAF_DANGER`] haben (Datei, Plugin-
//!   Verzeichnis: Daten, Anhängen, Attribute, Löschen, DACL und Besitzer
//!   ändern) beziehungsweise [`COMPONENT_DANGER`] (jedes Verzeichnis darüber:
//!   Löschen, DACL und Besitzer ändern, dazu bei Verzeichnissen
//!   `FILE_DELETE_CHILD`). In einem Verzeichnis darüber darf man Einträge
//!   hinzufügen (`FILE_ADD_FILE`, `FILE_ADD_SUBDIRECTORY`): ein bestehender
//!   Eintrag lässt sich damit nicht ersetzen; das Windows-Standardlaufwerk
//!   erlaubt "Authenticated Users" genau das an der Wurzel. Das ist die
//!   Abweichung von Unix, wo `w` am Verzeichnis alles zugleich erlaubt.
//! - Die Rechte eines Principals werden wie bei `AccessCheck` Eintrag für
//!   Eintrag gebildet (erlaubt: `granted |= maske & !verweigert`,
//!   verweigert: `denied |= maske & !gewährt`). Ein Verweigern für dieselbe
//!   SID oder für "Jeder" (S-1-1-0) zählt; ein Verweigern für eine andere
//!   Gruppe nicht, weil unbekannt ist, ob der Principal ihr angehört.
//!   Einträge, die nur vererbt werden (`INHERIT_ONLY_ACE`), gelten nicht für
//!   das Objekt selbst.
//!
//! Bareos-Gegenstück: keines; Bareos lädt keine signierten Plugins.

/// SID of the local system account.
pub const SYSTEM: &str = "S-1-5-18";
/// SID of the built-in Administrators group.
pub const ADMINISTRATORS: &str = "S-1-5-32-544";
/// SID of `NT SERVICE\TrustedInstaller`, the owner of the Windows files.
pub const TRUSTED_INSTALLER: &str = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
/// SID of "Everyone": a deny entry for it applies to every principal.
pub const EVERYONE: &str = "S-1-1-0";

/// `FILE_WRITE_DATA` (`FILE_ADD_FILE` on a directory).
pub const FILE_WRITE_DATA: u32 = 0x0000_0002;
/// `FILE_APPEND_DATA` (`FILE_ADD_SUBDIRECTORY` on a directory).
pub const FILE_APPEND_DATA: u32 = 0x0000_0004;
/// `FILE_WRITE_EA`.
pub const FILE_WRITE_EA: u32 = 0x0000_0010;
/// `FILE_DELETE_CHILD`: delete or rename the entries of a directory.
pub const FILE_DELETE_CHILD: u32 = 0x0000_0040;
/// `FILE_WRITE_ATTRIBUTES`.
pub const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
/// `DELETE`.
pub const DELETE: u32 = 0x0001_0000;
/// `WRITE_DAC`: change the permissions.
pub const WRITE_DAC: u32 = 0x0004_0000;
/// `WRITE_OWNER`: take ownership.
pub const WRITE_OWNER: u32 = 0x0008_0000;
/// `GENERIC_ALL` (not yet mapped to specific rights).
pub const GENERIC_ALL: u32 = 0x1000_0000;
/// `GENERIC_WRITE` (not yet mapped to specific rights).
pub const GENERIC_WRITE: u32 = 0x4000_0000;

/// What no untrusted principal may hold on any component of the path:
/// removing or replacing the object itself, or changing who may.
pub const COMPONENT_DANGER: u32 = DELETE | WRITE_DAC | WRITE_OWNER | GENERIC_ALL | GENERIC_WRITE;
/// What none may hold on the object itself (file, plugin directory): also
/// changing its content.
pub const LEAF_DANGER: u32 =
    COMPONENT_DANGER | FILE_WRITE_DATA | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES;

/// The kind of an access control entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AceKind {
    /// `ACCESS_ALLOWED_ACE_TYPE`.
    Allow,
    /// `ACCESS_DENIED_ACE_TYPE`.
    Deny,
    /// Any other type (callback, object, audit, mandatory label): not understood.
    Other(u8),
}

/// One entry of a DACL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    /// Allow or deny.
    pub kind: AceKind,
    /// The principal, as a string SID (`S-1-5-32-545`).
    pub sid: String,
    /// The access mask.
    pub mask: u32,
    /// `INHERIT_ONLY_ACE`: the entry is for new children, not for this object.
    pub inherit_only: bool,
}

/// Owner and DACL of one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descriptor {
    /// The owner as a string SID.
    pub owner: String,
    /// The entries in order; `None` is a NULL DACL (everybody may do everything).
    pub dacl: Option<Vec<Ace>>,
}

/// How strictly a component is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The object the check is about (policy, CA file, bundle, plugin
    /// directory): content must not change.
    Leaf,
    /// A directory above it: it must not be replaced or re-permissioned.
    Ancestor,
}

/// Whether `sid` may change what the check protects.
pub fn is_trusted_sid(sid: &str) -> bool {
    matches!(sid, SYSTEM | ADMINISTRATORS | TRUSTED_INSTALLER)
}

/// The names of the rights in `mask`, for the refusal text.
fn names(mask: u32) -> String {
    const ALL: [(u32, &str); 10] = [
        (FILE_WRITE_DATA, "write/add file"), (FILE_APPEND_DATA, "append/add subdirectory"),
        (FILE_WRITE_EA, "write EA"), (FILE_DELETE_CHILD, "delete child"),
        (FILE_WRITE_ATTRIBUTES, "write attributes"), (DELETE, "delete"), (WRITE_DAC, "change permissions"),
        (WRITE_OWNER, "take ownership"), (GENERIC_ALL, "generic all"), (GENERIC_WRITE, "generic write"),
    ];
    ALL.iter().filter(|(bit, _)| mask & bit != 0).map(|(_, n)| *n).collect::<Vec<_>>().join(", ")
}

/// The rights the entries give `principal`, evaluated in order like `AccessCheck`.
fn effective(entries: &[Ace], principal: &str) -> u32 {
    let (mut granted, mut denied) = (0u32, 0u32);
    for ace in entries.iter().filter(|a| !a.inherit_only && (a.sid == principal || a.sid == EVERYONE)) {
        match ace.kind {
            AceKind::Allow => granted |= ace.mask & !denied,
            AceKind::Deny => denied |= ace.mask & !granted,
            AceKind::Other(_) => {}
        }
    }
    granted
}

/// `Ok` when only SYSTEM, Administrators and TrustedInstaller can change the
/// object (as `directory` says it is a directory, in role `role`); else why not.
pub fn verdict(d: &Descriptor, directory: bool, role: Role) -> Result<(), String> {
    if !is_trusted_sid(&d.owner) {
        return Err(format!("owned by {}, not by SYSTEM, Administrators or TrustedInstaller", d.owner));
    }
    let Some(entries) = &d.dacl else {
        return Err("has no DACL (NULL DACL: everybody has full access)".into());
    };
    if let Some(other) = entries.iter().find_map(|a| match a.kind {
        AceKind::Other(t) => Some(t),
        _ => None,
    }) {
        return Err(format!("has an access control entry of type {other}, which is not understood"));
    }
    let mut danger = if role == Role::Leaf { LEAF_DANGER } else { COMPONENT_DANGER };
    if directory {
        danger |= FILE_DELETE_CHILD;
    }
    let mut principals: Vec<&str> = entries
        .iter()
        .filter(|a| a.kind == AceKind::Allow && !a.inherit_only && !is_trusted_sid(&a.sid))
        .map(|a| a.sid.as_str())
        .collect();
    principals.sort_unstable();
    principals.dedup();
    for principal in principals {
        let held = effective(entries, principal) & danger;
        if held != 0 {
            return Err(format!("{principal} may {}", names(held)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const USERS: &str = "S-1-5-32-545";
    const AUTHENTICATED: &str = "S-1-5-11";
    const GUESTS: &str = "S-1-5-32-546";
    const ME: &str = "S-1-5-21-1-2-3-1001";
    const READ: u32 = 0x0012_00A9;
    const MODIFY: u32 = 0x0013_01BF;
    const FULL: u32 = 0x001F_01FF;

    fn ace(kind: AceKind, sid: &str, mask: u32) -> Ace {
        Ace { kind, sid: sid.into(), mask, inherit_only: false }
    }
    fn allow(sid: &str, mask: u32) -> Ace {
        ace(AceKind::Allow, sid, mask)
    }
    fn deny(sid: &str, mask: u32) -> Ace {
        ace(AceKind::Deny, sid, mask)
    }
    fn trusted(extra: Vec<Ace>) -> Descriptor {
        let mut dacl = vec![allow(SYSTEM, FULL), allow(ADMINISTRATORS, FULL)];
        dacl.extend(extra);
        Descriptor { owner: ADMINISTRATORS.into(), dacl: Some(dacl) }
    }
    fn refused(d: &Descriptor, directory: bool, role: Role) -> String {
        verdict(d, directory, role).expect_err("must be refused")
    }

    #[test]
    fn system_files_are_trusted() {
        // owner TrustedInstaller, Users read and execute: C:\Windows\System32
        let d = Descriptor {
            owner: TRUSTED_INSTALLER.into(),
            dacl: Some(vec![allow(TRUSTED_INSTALLER, FULL), allow(SYSTEM, FULL), allow(ADMINISTRATORS, FULL),
                            allow(USERS, READ)]),
        };
        assert!(verdict(&d, true, Role::Leaf).is_ok());
        assert!(verdict(&d, false, Role::Leaf).is_ok());
    }

    #[test]
    fn an_untrusted_owner_makes_every_dacl_worthless() {
        for owner in [ME, USERS, "S-1-1-0", "S-1-3-0"] {
            let mut d = trusted(vec![allow(USERS, READ)]);
            d.owner = owner.into();
            assert!(refused(&d, false, Role::Leaf).contains("owned by"), "{owner}");
            assert!(refused(&d, true, Role::Ancestor).contains("owned by"), "{owner}");
        }
    }

    #[test]
    fn a_null_dacl_is_refused() {
        let d = Descriptor { owner: SYSTEM.into(), dacl: None };
        assert!(refused(&d, false, Role::Leaf).contains("NULL DACL"));
        assert!(refused(&d, true, Role::Ancestor).contains("NULL DACL"));
    }

    #[test]
    fn write_for_users_everyone_or_authenticated_users_is_refused() {
        for sid in [USERS, EVERYONE, AUTHENTICATED, ME] {
            let d = trusted(vec![allow(sid, MODIFY)]);
            let why = refused(&d, false, Role::Leaf);
            assert!(why.contains(sid) && why.contains("delete"), "{why}");
            assert!(refused(&d, true, Role::Ancestor).contains(sid));
        }
    }

    #[test]
    fn single_dangerous_rights_are_each_enough() {
        for (bit, name) in [(FILE_WRITE_DATA, "write"), (FILE_APPEND_DATA, "append"), (FILE_WRITE_EA, "EA"),
                            (FILE_WRITE_ATTRIBUTES, "attributes"), (DELETE, "delete"), (WRITE_DAC, "permissions"),
                            (WRITE_OWNER, "ownership"), (GENERIC_WRITE, "generic"), (GENERIC_ALL, "generic")] {
            let d = trusted(vec![allow(USERS, bit | READ)]);
            assert!(verdict(&d, false, Role::Leaf).is_err(), "{name} on a file");
        }
        // only the three that replace or re-permission matter above the leaf
        for bit in [DELETE, WRITE_DAC, WRITE_OWNER, GENERIC_WRITE, GENERIC_ALL] {
            assert!(verdict(&trusted(vec![allow(USERS, bit)]), true, Role::Ancestor).is_err());
        }
        assert!(verdict(&trusted(vec![allow(USERS, FILE_DELETE_CHILD)]), true, Role::Ancestor).is_err());
    }

    #[test]
    fn read_only_rights_are_fine() {
        let d = trusted(vec![allow(USERS, READ), allow(AUTHENTICATED, READ), allow(EVERYONE, 0x0012_0089)]);
        assert!(verdict(&d, false, Role::Leaf).is_ok());
        assert!(verdict(&d, true, Role::Leaf).is_ok());
    }

    #[test]
    fn adding_entries_to_a_directory_above_is_allowed_but_not_in_the_plugin_directory() {
        // the Windows default at a volume root: Authenticated Users may add subdirectories
        let root = trusted(vec![allow(AUTHENTICATED, FILE_APPEND_DATA | READ)]);
        assert!(verdict(&root, true, Role::Ancestor).is_ok());
        assert!(refused(&root, true, Role::Leaf).contains("append"));
        let add_file = trusted(vec![allow(USERS, FILE_WRITE_DATA | READ)]);
        assert!(verdict(&add_file, true, Role::Ancestor).is_ok());
        assert!(verdict(&add_file, true, Role::Leaf).is_err());
    }

    #[test]
    fn delete_child_counts_only_for_a_directory() {
        let d = trusted(vec![allow(USERS, FILE_DELETE_CHILD | READ)]);
        assert!(verdict(&d, false, Role::Leaf).is_ok(), "bit 0x40 is not a right of a file");
        assert!(refused(&d, true, Role::Ancestor).contains("delete child"));
    }

    #[test]
    fn inherit_only_entries_do_not_apply_to_the_object() {
        // Authenticated Users: (OI)(CI)(IO)(M) at a volume root
        let mut inherit = allow(AUTHENTICATED, MODIFY);
        inherit.inherit_only = true;
        let d = trusted(vec![inherit.clone()]);
        assert!(verdict(&d, true, Role::Ancestor).is_ok());
        assert!(verdict(&d, true, Role::Leaf).is_ok());
        // the same entry without the flag is refused
        inherit.inherit_only = false;
        assert!(verdict(&trusted(vec![inherit]), true, Role::Ancestor).is_err());
    }

    #[test]
    fn a_deny_entry_before_the_allow_cancels_it_for_the_same_sid_or_everyone() {
        let same = trusted(vec![deny(USERS, MODIFY), allow(USERS, MODIFY)]);
        assert!(verdict(&same, false, Role::Leaf).is_ok());
        let everyone = trusted(vec![deny(EVERYONE, MODIFY), allow(USERS, MODIFY)]);
        assert!(verdict(&everyone, false, Role::Leaf).is_ok());
        // partial: the deny covers delete only, the write stays
        let partial = trusted(vec![deny(USERS, DELETE), allow(USERS, MODIFY)]);
        let why = refused(&partial, false, Role::Leaf);
        assert!(why.contains("write") && !why.contains("delete"), "{why}");
    }

    #[test]
    fn a_deny_after_the_allow_cancels_nothing() {
        // AccessCheck: bits granted earlier cannot be taken back by a later deny
        let d = trusted(vec![allow(USERS, MODIFY), deny(USERS, MODIFY)]);
        assert!(verdict(&d, false, Role::Leaf).is_err());
    }

    #[test]
    fn a_deny_for_another_group_cancels_nothing() {
        // the principal may or may not belong to Guests: not assumed
        let d = trusted(vec![deny(GUESTS, MODIFY), allow(USERS, MODIFY)]);
        assert!(verdict(&d, false, Role::Leaf).is_err());
    }

    #[test]
    fn an_everyone_allow_reaches_every_principal() {
        // Users has only read for itself, but Everyone may write: Everyone is refused
        let d = trusted(vec![allow(USERS, READ), allow(EVERYONE, MODIFY)]);
        assert!(refused(&d, false, Role::Leaf).contains(EVERYONE));
    }

    #[test]
    fn entries_that_are_not_understood_are_refused() {
        let d = trusted(vec![ace(AceKind::Other(9), USERS, READ)]);
        assert!(refused(&d, false, Role::Leaf).contains("type 9"));
    }

    #[test]
    fn trusted_principals_may_hold_anything() {
        let d = Descriptor {
            owner: TRUSTED_INSTALLER.into(),
            dacl: Some(vec![allow(TRUSTED_INSTALLER, FULL), allow(SYSTEM, GENERIC_ALL), allow(ADMINISTRATORS, FULL)]),
        };
        assert!(verdict(&d, true, Role::Leaf).is_ok());
    }
}
