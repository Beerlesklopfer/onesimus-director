// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Cleaning volume files for the Bareos stream (docs/plans/PODMAN-PLUGIN.md,
//! "Attribute der Volume-Dateien", V-1, V-3, S-13).
//!
//! # Purpose
//!
//! A restore to files runs without plugin code, often with another FD, and
//! knows only the attributes of the stream. An account that is root inside
//! its namespace could otherwise plant a file with ns-UID 0 and mode 4755
//! or `cap_setuid+ep`, which a later restore to files turns into a
//! root-owned setuid or capability file on the host. Protection must
//! therefore happen while backing up: this module decides, from what the
//! host itself read from disk (never from the broker or the guest), which
//! attributes go into the stream and which only into `ownership.tsv`.
//!
//! Rules for the stream (rootless and rootful alike):
//! - IDs in host view (what is on disk), checked against the domain's map;
//! - mode: only the permission bits `0o777`, never setuid/setgid/sticky;
//! - xattrs: only `user.*`; never `security.capability`, never `trusted.*`;
//! - kinds: regular, directory, symlink, FIFO (hard links are decided by the
//!   scan); character and block devices and sockets never;
//! - every ID outside the map, also in an ACL entry or a v3 capability
//!   root id, rejects the file (rootless).
//!
//! # Data flow
//!
//! ```mermaid
//! flowchart LR
//!     D[(disk: fstatat, flistxattr<br/>by ofd as root)] --> R[RawEntry]
//!     R --> C{classify}
//!     C -- device / socket / ID outside map --> X[Rejected: manifest 'rejected']
//!     C -- ok --> S[StreamAttrs<br/>host IDs, mode & 0o777, user.* xattrs]
//!     C -- special bits, caps, ACL, SELinux --> O[OwnershipRow<br/>ns view]
//!     S --> SD[(Bareos stream)]
//!     O --> T[ownership.tsv after data/]
//! ```
//!
//! ```mermaid
//! classDiagram
//!     class RawEntry { relpath; kind; mode; uid; gid; size; xattrs }
//!     class Domain { uid_map; gid_map; rootless }
//!     class StreamAttrs { uid; gid; perm; xattrs }
//!     class OwnershipRow { relpath; ns_uid; ns_gid; mode; caps; acl; selinux }
//!     class Outcome { Stream; Rejected }
//!     RawEntry --> Outcome
//!     Domain --> Outcome
//!     Outcome *-- StreamAttrs
//!     Outcome *-- OwnershipRow
//! ```
//!
//! # Counterpart
//!
//! The online restore applies `ownership.tsv` under host control (second
//! pass: special bits and capabilities after `chown`); a restore to files
//! never sees them.

use crate::idmap::IdMap;

/// File kinds as the host reads them with `fstatat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    /// S_IFREG.
    Regular,
    /// S_IFDIR.
    Directory,
    /// S_IFLNK.
    Symlink,
    /// S_IFIFO.
    Fifo,
    /// S_IFCHR.
    Char,
    /// S_IFBLK.
    Block,
    /// S_IFSOCK.
    Socket,
}

impl RawKind {
    /// The kind of an `st_mode`.
    pub fn of_mode(mode: u32) -> Option<Self> {
        // mode_t is u32 on Linux, u16 on macOS
        #[allow(clippy::unnecessary_cast)]
        let (fmt, kind) = (libc::S_IFMT as u32, |bits: libc::mode_t| bits as u32);
        let m = mode & fmt;
        Some(match m {
            _ if m == kind(libc::S_IFREG) => Self::Regular,
            _ if m == kind(libc::S_IFDIR) => Self::Directory,
            _ if m == kind(libc::S_IFLNK) => Self::Symlink,
            _ if m == kind(libc::S_IFIFO) => Self::Fifo,
            _ if m == kind(libc::S_IFCHR) => Self::Char,
            _ if m == kind(libc::S_IFBLK) => Self::Block,
            _ if m == kind(libc::S_IFSOCK) => Self::Socket,
            _ => return None,
        })
    }
}

/// What the host read from disk for one volume entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    /// Path relative to the volume's `_data` (empty for `_data` itself).
    pub relpath: Vec<u8>,
    /// File kind.
    pub kind: RawKind,
    /// `st_mode & 0o7777`.
    pub mode: u32,
    /// Owner, host view.
    pub uid: u32,
    /// Group, host view.
    pub gid: u32,
    /// Extended attributes as read (name, value).
    pub xattrs: Vec<(Vec<u8>, Vec<u8>)>,
}

/// The domain whose map decides which IDs are legitimate.
#[derive(Debug, Clone)]
pub struct Domain {
    /// Host → namespace map of user IDs.
    pub uid_map: IdMap,
    /// Host → namespace map of group IDs.
    pub gid_map: IdMap,
    /// Rootless: IDs outside the map reject a file.
    pub rootless: bool,
}

impl Domain {
    /// The rootful profile: identity maps.
    pub fn rootful() -> Self {
        Self { uid_map: IdMap::identity(), gid_map: IdMap::identity(), rootless: false }
    }
}

/// The attributes of an entry in the Bareos stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamAttrs {
    /// Owner, host view.
    pub uid: u32,
    /// Group, host view.
    pub gid: u32,
    /// Permission bits only (`& 0o777`).
    pub perm: u32,
    /// Only `user.*` extended attributes.
    pub xattrs: Vec<(Vec<u8>, Vec<u8>)>,
}

/// One line of `ownership.tsv`: what the stream leaves out, in namespace
/// view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnershipRow {
    /// Path relative to `data/`.
    pub relpath: Vec<u8>,
    /// Owner, namespace view.
    pub ns_uid: u32,
    /// Group, namespace view.
    pub ns_gid: u32,
    /// Full mode `0o7777`.
    pub mode: u32,
    /// Capability text and raw v2 value in namespace view.
    pub caps: Option<String>,
    /// ACL text with namespace IDs (access, then `default:` entries).
    pub acl: Option<String>,
    /// SELinux label, informative only.
    pub selinux: Option<String>,
}

/// Why an entry stays out of the stream (fixed texts for the manifest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Character or block device (rootless always, rootful in the MVP).
    Device,
    /// Socket: never saved.
    Socket,
    /// Owner or group outside the domain's map.
    IdOutsideMap,
    /// An ACL entry names an ID outside the map.
    AclIdOutsideMap,
    /// The root id of a v3 capability is outside the map.
    CapRootOutsideMap,
    /// An ACL or capability value that cannot be parsed.
    Malformed,
}

impl Reason {
    /// The fixed text of the manifest's `rejected` list.
    pub fn text(self) -> &'static str {
        match self {
            Self::Device => "device",
            Self::Socket => "socket",
            Self::IdOutsideMap => "id-outside-map",
            Self::AclIdOutsideMap => "acl-id-outside-map",
            Self::CapRootOutsideMap => "cap-rootid-outside-map",
            Self::Malformed => "malformed-attribute",
        }
    }
}

/// The decision for one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Goes into the stream; `row` goes into ownership.tsv.
    Stream {
        /// Stream attributes.
        attrs: StreamAttrs,
        /// Namespace view of what the stream leaves out, if anything.
        row: Option<OwnershipRow>,
    },
    /// Stays out; listed under `rejected`.
    Rejected(Reason),
}

/// xattr name of POSIX access ACLs.
const ACL_ACCESS: &[u8] = b"system.posix_acl_access";
/// xattr name of POSIX default ACLs.
const ACL_DEFAULT: &[u8] = b"system.posix_acl_default";
/// xattr name of file capabilities.
const CAPABILITY: &[u8] = b"security.capability";
/// xattr name of SELinux labels.
const SELINUX: &[u8] = b"security.selinux";
/// Prefix of the only xattrs the stream carries.
const USER_PREFIX: &[u8] = b"user.";

/// Decides stream attributes, ownership row or rejection for `raw`.
pub fn classify(raw: &RawEntry, domain: &Domain) -> Outcome {
    match raw.kind {
        RawKind::Char | RawKind::Block => return Outcome::Rejected(Reason::Device),
        RawKind::Socket => return Outcome::Rejected(Reason::Socket),
        _ => {}
    }
    let (Some(ns_uid), Some(ns_gid)) = (domain.uid_map.to_ns(raw.uid), domain.gid_map.to_ns(raw.gid)) else {
        return Outcome::Rejected(Reason::IdOutsideMap);
    };
    let mut acl_parts = Vec::new();
    let mut caps = None;
    let mut selinux = None;
    let mut xattrs = Vec::new();
    for (name, value) in &raw.xattrs {
        let name = name.as_slice();
        if name == ACL_ACCESS || name == ACL_DEFAULT {
            let entries = match parse_acl(value) {
                Some(e) => e,
                None => return Outcome::Rejected(Reason::Malformed),
            };
            match acl_text(&entries, domain) {
                Ok(text) if name == ACL_DEFAULT => acl_parts.push(prefix_default(&text)),
                Ok(text) => acl_parts.insert(0, text),
                Err(r) => return Outcome::Rejected(r),
            }
        } else if name == CAPABILITY {
            match caps_text(value, &domain.uid_map) {
                Ok(text) => caps = Some(text),
                Err(r) => return Outcome::Rejected(r),
            }
        } else if name == SELINUX {
            let label = String::from_utf8_lossy(value.strip_suffix(&[0]).unwrap_or(value)).into_owned();
            selinux = Some(label);
        } else if name.starts_with(USER_PREFIX) {
            xattrs.push((name.to_vec(), value.clone()));
        }
        // trusted.*, other security.* and system.* never reach the stream.
    }
    let acl = (!acl_parts.is_empty()).then(|| acl_parts.join(","));
    let special = raw.mode & 0o7000 != 0;
    let row = (special || caps.is_some() || acl.is_some() || selinux.is_some()).then(|| OwnershipRow {
        relpath: raw.relpath.clone(),
        ns_uid,
        ns_gid,
        mode: raw.mode & 0o7777,
        caps,
        acl,
        selinux,
    });
    Outcome::Stream { attrs: StreamAttrs { uid: raw.uid, gid: raw.gid, perm: raw.mode & 0o777, xattrs }, row }
}

/// One POSIX ACL entry: tag, permissions, ID (only for USER and GROUP).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AclEntry {
    /// ACL_USER_OBJ 1, ACL_USER 2, ACL_GROUP_OBJ 4, ACL_GROUP 8, ACL_MASK 0x10, ACL_OTHER 0x20.
    pub tag: u16,
    /// rwx bits.
    pub perm: u16,
    /// Host ID for ACL_USER/ACL_GROUP.
    pub id: u32,
}

/// Tag of a named user entry.
const ACL_USER: u16 = 2;
/// Tag of a named group entry.
const ACL_GROUP: u16 = 8;
/// Version of the xattr ACL format.
const ACL_VERSION: u32 = 2;

/// Parses the `system.posix_acl_*` xattr format (version 2, entries of
/// tag u16, perm u16, id u32, little endian).
pub fn parse_acl(value: &[u8]) -> Option<Vec<AclEntry>> {
    let (head, body) = value.split_at_checked(4)?;
    if u32::from_le_bytes(head.try_into().ok()?) != ACL_VERSION || body.len() % 8 != 0 || body.len() > 8 * 1024 {
        return None;
    }
    Some(body.as_chunks::<8>().0.iter().map(|c| AclEntry {
        tag: u16::from_le_bytes([c[0], c[1]]),
        perm: u16::from_le_bytes([c[2], c[3]]),
        id: u32::from_le_bytes([c[4], c[5], c[6], c[7]]),
    }).collect())
}

/// `rwx` text of ACL permissions.
fn perm_text(p: u16) -> String {
    [(4, 'r'), (2, 'w'), (1, 'x')].iter().map(|&(b, c)| if p & b != 0 { c } else { '-' }).collect()
}

/// ACL text with namespace IDs; an ID outside the map rejects the file.
fn acl_text(entries: &[AclEntry], domain: &Domain) -> Result<String, Reason> {
    let mut out = Vec::new();
    for e in entries {
        let part = match e.tag {
            1 => format!("u::{}", perm_text(e.perm)),
            ACL_USER => {
                let ns = domain.uid_map.to_ns(e.id).ok_or(Reason::AclIdOutsideMap)?;
                format!("u:{ns}:{}", perm_text(e.perm))
            }
            4 => format!("g::{}", perm_text(e.perm)),
            ACL_GROUP => {
                let ns = domain.gid_map.to_ns(e.id).ok_or(Reason::AclIdOutsideMap)?;
                format!("g:{ns}:{}", perm_text(e.perm))
            }
            0x10 => format!("m::{}", perm_text(e.perm)),
            0x20 => format!("o::{}", perm_text(e.perm)),
            _ => return Err(Reason::Malformed),
        };
        out.push(part);
    }
    Ok(out.join(","))
}

/// Prefixes every entry of an ACL text with `d:`.
fn prefix_default(text: &str) -> String {
    text.split(',').map(|p| format!("d:{p}")).collect::<Vec<_>>().join(",")
}

/// Revision 2 of `vfs_cap_data` (no root id).
const VFS_CAP_REVISION_2: u32 = 0x0200_0000;
/// Revision 3 (with root id, written inside user namespaces).
const VFS_CAP_REVISION_3: u32 = 0x0300_0000;
/// Mask of the revision in `magic_etc`.
const VFS_CAP_REVISION_MASK: u32 = 0xFF00_0000;
/// The effective flag in `magic_etc`.
const VFS_CAP_FLAGS_EFFECTIVE: u32 = 0x0000_0001;

/// The namespace view of a `security.capability` value: text and the raw
/// v2 value as hex. A v3 root id outside the map rejects the file.
pub fn caps_text(value: &[u8], uid_map: &IdMap) -> Result<String, Reason> {
    let word = |i: usize| -> Option<u32> { Some(u32::from_le_bytes(value.get(i * 4..i * 4 + 4)?.try_into().ok()?)) };
    let magic = word(0).ok_or(Reason::Malformed)?;
    let revision = magic & VFS_CAP_REVISION_MASK;
    let (expected_len, rootid) = match revision {
        VFS_CAP_REVISION_2 => (20, None),
        VFS_CAP_REVISION_3 => (24, word(5)),
        _ => return Err(Reason::Malformed),
    };
    if value.len() != expected_len {
        return Err(Reason::Malformed);
    }
    let ns_root = match rootid {
        Some(r) => Some(uid_map.to_ns(r).ok_or(Reason::CapRootOutsideMap)?),
        None => None,
    };
    let (p0, i0, p1, i1) = (word(1).unwrap_or(0), word(2).unwrap_or(0), word(3).unwrap_or(0), word(4).unwrap_or(0));
    let permitted = u64::from(p0) | (u64::from(p1) << 32);
    let inheritable = u64::from(i0) | (u64::from(i1) << 32);
    let effective = magic & VFS_CAP_FLAGS_EFFECTIVE != 0;
    let mut v2 = Vec::with_capacity(20);
    v2.extend_from_slice(&(VFS_CAP_REVISION_2 | (magic & VFS_CAP_FLAGS_EFFECTIVE)).to_le_bytes());
    for w in [p0, i0, p1, i1] {
        v2.extend_from_slice(&w.to_le_bytes());
    }
    let root = ns_root.map(|r| format!(" rootid={r}")).unwrap_or_default();
    Ok(format!(
        "permitted=0x{permitted:x} inheritable=0x{inheritable:x} effective={}{root} raw={}",
        u8::from(effective),
        hex::encode(v2)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Account 1000 with sub-IDs 100000..165536.
    fn rootless() -> Domain {
        let m = IdMap::rootless(1000, &[(100000, 65536)]).unwrap();
        Domain { uid_map: m.clone(), gid_map: m, rootless: true }
    }
    fn raw(kind: RawKind, mode: u32, uid: u32, gid: u32) -> RawEntry {
        RawEntry { relpath: b"f".to_vec(), kind, mode, uid, gid, xattrs: Vec::new() }
    }
    fn acl(entries: &[(u16, u16, u32)]) -> Vec<u8> {
        let mut v = ACL_VERSION.to_le_bytes().to_vec();
        for &(t, p, i) in entries {
            v.extend_from_slice(&t.to_le_bytes());
            v.extend_from_slice(&p.to_le_bytes());
            v.extend_from_slice(&i.to_le_bytes());
        }
        v
    }
    fn cap_v3(permitted: u32, rootid: u32) -> Vec<u8> {
        let mut v = (VFS_CAP_REVISION_3 | VFS_CAP_FLAGS_EFFECTIVE).to_le_bytes().to_vec();
        for w in [permitted, 0, 0, 0, rootid] {
            v.extend_from_slice(&w.to_le_bytes());
        }
        v
    }

    /// The case of exit criterion P0 (1): an ns-UID-0 setuid file keeps its
    /// host owner, loses the setuid bit in the stream and is described in
    /// ownership.tsv.
    #[test]
    fn setuid_of_ns_root_never_reaches_the_stream() {
        let Outcome::Stream { attrs, row } = classify(&raw(RawKind::Regular, 0o4755, 1000, 1000), &rootless()) else {
            panic!("rejected")
        };
        assert_eq!((attrs.uid, attrs.gid, attrs.perm), (1000, 1000, 0o755));
        let row = row.unwrap();
        assert_eq!((row.ns_uid, row.ns_gid, row.mode), (0, 0, 0o4755));
        for mode in [0o2755, 0o1777, 0o7777] {
            let Outcome::Stream { attrs, row } = classify(&raw(RawKind::Directory, mode, 100005, 100005), &rootless()) else {
                panic!()
            };
            assert_eq!(attrs.perm & 0o7000, 0);
            assert_eq!(row.unwrap().ns_uid, 6);
        }
    }

    #[test]
    fn capabilities_only_in_the_ownership_row() {
        let mut r = raw(RawKind::Regular, 0o755, 1000, 1000);
        r.xattrs = vec![(CAPABILITY.to_vec(), cap_v3(1 << 7, 1000)), (b"user.k".to_vec(), b"v".to_vec()),
            (b"trusted.x".to_vec(), b"y".to_vec()), (b"security.ima".to_vec(), b"z".to_vec())];
        let Outcome::Stream { attrs, row } = classify(&r, &rootless()) else { panic!() };
        assert_eq!(attrs.xattrs, vec![(b"user.k".to_vec(), b"v".to_vec())]);
        let caps = row.unwrap().caps.unwrap();
        assert!(caps.starts_with("permitted=0x80 inheritable=0x0 effective=1 rootid=0 raw=01000002"), "{caps}");
        // A v3 root id outside the map (e.g. host root) rejects the file.
        r.xattrs = vec![(CAPABILITY.to_vec(), cap_v3(1, 0))];
        assert_eq!(classify(&r, &rootless()), Outcome::Rejected(Reason::CapRootOutsideMap));
        r.xattrs = vec![(CAPABILITY.to_vec(), vec![1, 2, 3])];
        assert_eq!(classify(&r, &rootless()), Outcome::Rejected(Reason::Malformed));
    }

    /// Exit criterion P0 (5): devices and foreign IDs are rejected.
    #[test]
    fn devices_sockets_and_foreign_ids_are_rejected() {
        let d = rootless();
        assert_eq!(classify(&raw(RawKind::Char, 0o666, 1000, 1000), &d), Outcome::Rejected(Reason::Device));
        assert_eq!(classify(&raw(RawKind::Block, 0o660, 1000, 1000), &d), Outcome::Rejected(Reason::Device));
        assert_eq!(classify(&raw(RawKind::Socket, 0o777, 1000, 1000), &d), Outcome::Rejected(Reason::Socket));
        assert_eq!(classify(&raw(RawKind::Char, 0o666, 0, 0), &Domain::rootful()), Outcome::Rejected(Reason::Device));
        assert_eq!(classify(&raw(RawKind::Regular, 0o644, 0, 1000), &d), Outcome::Rejected(Reason::IdOutsideMap));
        assert_eq!(classify(&raw(RawKind::Regular, 0o644, 1000, 165536), &d), Outcome::Rejected(Reason::IdOutsideMap));
        // An ACL naming host root, access or default.
        let mut r = raw(RawKind::Directory, 0o755, 1000, 1000);
        r.xattrs = vec![(ACL_DEFAULT.to_vec(), acl(&[(1, 7, 0), (ACL_USER, 5, 0), (4, 5, 0), (0x10, 5, 0), (0x20, 0, 0)]))];
        assert_eq!(classify(&r, &d), Outcome::Rejected(Reason::AclIdOutsideMap));
    }

    #[test]
    fn acls_are_written_in_namespace_ids() {
        let mut r = raw(RawKind::Directory, 0o750, 1000, 1000);
        r.xattrs = vec![
            (ACL_DEFAULT.to_vec(), acl(&[(1, 7, 0), (4, 5, 0), (0x20, 0, 0)])),
            (ACL_ACCESS.to_vec(), acl(&[(1, 7, 0), (ACL_USER, 5, 100999), (4, 5, 0), (ACL_GROUP, 4, 1000), (0x10, 5, 0), (0x20, 0, 0)])),
        ];
        let Outcome::Stream { row, .. } = classify(&r, &rootless()) else { panic!() };
        assert_eq!(row.unwrap().acl.unwrap(), "u::rwx,u:1000:r-x,g::r-x,g:0:r--,m::r-x,o::---,d:u::rwx,d:g::r-x,d:o::---");
        r.xattrs = vec![(ACL_ACCESS.to_vec(), vec![2, 0, 0, 0, 1])];
        assert_eq!(classify(&r, &rootless()), Outcome::Rejected(Reason::Malformed));
    }

    #[test]
    fn plain_files_need_no_row_and_rootful_keeps_host_ids() {
        let Outcome::Stream { attrs, row } = classify(&raw(RawKind::Regular, 0o640, 0, 33), &Domain::rootful()) else {
            panic!()
        };
        assert_eq!((attrs.uid, attrs.gid, attrs.perm), (0, 33, 0o640));
        assert!(row.is_none());
        let mut r = raw(RawKind::Fifo, 0o600, 100001, 100001);
        r.xattrs = vec![(SELINUX.to_vec(), b"system_u:object_r:container_file_t:s0\0".to_vec())];
        let Outcome::Stream { row, .. } = classify(&r, &rootless()) else { panic!() };
        assert_eq!(row.unwrap().selinux.unwrap(), "system_u:object_r:container_file_t:s0");
        assert_eq!(RawKind::of_mode(libc::S_IFIFO | 0o600), Some(RawKind::Fifo));
    }
}
