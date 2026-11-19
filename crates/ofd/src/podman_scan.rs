//! The host's scan handle for volume data (docs/plans/PODMAN-PLUGIN.md,
//! "Named Volumes", "Erweiterung von FileEntry", V-2, V-3).
//!
//! # Purpose
//!
//! `ofd` reads the bytes **and** the attributes of volume files itself,
//! rootful and rootless: it opens the graphroot of the account policy and
//! from there `volumes/<name>/_data`, and every entry below, with
//! `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS | RESOLVE_NO_XDEV)`. Root
//! can read the sub-UIDs' files; no user namespace is needed for reading.
//! The scan yields the real stat from disk, i.e. host IDs; nothing an
//! account or its broker says can invent owners, special bits or devices.
//! `onesimus_podman_core::sanitize::classify` decides what goes into the
//! stream and what into `ownership.tsv`; rejected entries are listed.
//!
//! The guest receives only path, kind, size and a reference
//! (`FileEntry.source_ref`). A reference is valid once, in this job and
//! this instance; its path must be the volume's prefix plus the entry's
//! relative path, and its kind must match. `ReadFile` is never called for
//! such an entry: the host reopens the file by its relative path below the
//! `_data` descriptor, checks device and inode against the scan and sends
//! the bytes itself.
//!
//! # Data flow
//!
//! ```mermaid
//! sequenceDiagram
//!     participant G as WASM guest
//!     participant P as PodmanPort
//!     participant S as ScanTable (this module)
//!     participant D as disk (_data)
//!     participant B as backup.rs
//!     G->>P: volume-scan {volume, cursor}
//!     P->>S: scan(volume) once
//!     S->>D: openat2 BENEATH|NO_SYMLINKS|NO_XDEV, fstatat, xattrs
//!     S->>S: sanitize::classify, ownership.tsv
//!     S-->>P: entries (relpath, kind, size, source_ref), rejected
//!     P-->>G: VolumeScanResponse
//!     G->>B: NextFile: FileEntry{path, kind, source_ref}
//!     B->>S: take(path, kind, source_ref) (once, path and kind checked)
//!     S->>D: reopen below _data, compare dev/ino
//!     S-->>B: stat in host view, mode & 0o777, user.* xattrs, ACL, reader
//!     B->>B: attributes and data into the SD stream (no ReadFile)
//! ```
//!
//! # Counterpart
//!
//! `podman_port.rs` (the guest-facing `volume-scan`), `backup.rs`
//! (`save_scan_file`), the online restore (applies `ownership.tsv`).

use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use onesimus_podman_core::ownership::{self, Table};
use onesimus_podman_core::sanitize::{self, AclEntry, Domain, Outcome, RawEntry, RawKind, Reason};
use onesimus_proto_rs::onesimus::filed::plugin::v1 as pb;

use crate::attribs::Stat;

/// `struct open_how` of openat2(2) (libc's is non-exhaustive).
#[repr(C)]
struct OpenHow {
    /// O_* flags.
    flags: u64,
    /// Mode for O_CREAT (unused).
    mode: u64,
    /// RESOLVE_* flags.
    resolve: u64,
}

/// The resolve flags of every open below a volume.
const RESOLVE: u64 = libc::RESOLVE_BENEATH | libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_XDEV;

/// openat2(2) relative to `dir`; `path` must be relative.
pub fn openat2(dir: i32, path: &[u8], flags: i32, resolve: u64) -> std::io::Result<OwnedFd> {
    let c = CString::new(if path.is_empty() { &b"."[..] } else { path }).map_err(|_| std::io::Error::other("NUL in path"))?;
    let how = OpenHow { flags: (flags | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64, mode: 0, resolve };
    // SAFETY: valid descriptor, NUL-terminated path, open_how of the size passed.
    let fd = unsafe { libc::syscall(libc::SYS_openat2, dir, c.as_ptr(), &how as *const OpenHow, std::mem::size_of::<OpenHow>()) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: the kernel returned a new descriptor we own.
    Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) })
}

/// fstat of a descriptor.
fn fstat(fd: i32) -> std::io::Result<libc::stat> {
    // SAFETY: zeroed stat is a valid out-parameter.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid descriptor and out-parameter.
    if unsafe { libc::fstat(fd, &mut st) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(st)
}

/// fstatat(dir, name, AT_SYMLINK_NOFOLLOW).
fn fstatat(dir: i32, name: &[u8]) -> std::io::Result<libc::stat> {
    let c = CString::new(name).map_err(|_| std::io::Error::other("NUL in name"))?;
    // SAFETY: zeroed stat is a valid out-parameter.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid descriptor, name and out-parameter; no link followed.
    if unsafe { libc::fstatat(dir, c.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(st)
}

/// Extended attributes through a path that is not followed at its end
/// (`l*xattr` on `/proc/self/fd/<dir>/<name>`, or an open descriptor).
fn read_xattrs(target: XattrTarget<'_>) -> std::io::Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let list = |buf: *mut libc::c_char, len: usize| -> isize {
        match target {
            // SAFETY: valid descriptor and buffer of `len` bytes.
            XattrTarget::Fd(fd) => unsafe { libc::flistxattr(fd, buf, len) },
            XattrTarget::Path(p) => unsafe { libc::llistxattr(p.as_ptr(), buf, len) },
        }
    };
    let get = |name: &CString, buf: *mut libc::c_void, len: usize| -> isize {
        match target {
            // SAFETY: valid descriptor, name and buffer.
            XattrTarget::Fd(fd) => unsafe { libc::fgetxattr(fd, name.as_ptr(), buf, len) },
            XattrTarget::Path(p) => unsafe { libc::lgetxattr(p.as_ptr(), name.as_ptr(), buf, len) },
        }
    };
    let len = list(std::ptr::null_mut(), 0);
    if len < 0 {
        let e = std::io::Error::last_os_error();
        return if matches!(e.raw_os_error(), Some(libc::ENOTSUP) | Some(libc::EPERM)) { Ok(Vec::new()) } else { Err(e) };
    }
    if len == 0 {
        return Ok(Vec::new());
    }
    if len as usize > MAX_XATTR_LIST {
        return Err(std::io::Error::other("xattr list too long"));
    }
    let mut names = vec![0u8; len as usize];
    let len = list(names.as_mut_ptr().cast(), names.len());
    if len < 0 {
        return Err(std::io::Error::last_os_error());
    }
    names.truncate(len as usize);
    let mut out = Vec::new();
    for name in names.split(|&b| b == 0).filter(|n| !n.is_empty()) {
        let c = CString::new(name).map_err(|_| std::io::Error::other("NUL"))?;
        let vlen = get(&c, std::ptr::null_mut(), 0);
        if vlen < 0 {
            continue;
        }
        if vlen as usize > MAX_XATTR_VALUE {
            return Err(std::io::Error::other("xattr value too large"));
        }
        let mut value = vec![0u8; vlen as usize];
        let n = get(&c, value.as_mut_ptr().cast(), value.len());
        if n < 0 {
            continue;
        }
        value.truncate(n as usize);
        out.push((name.to_vec(), value));
    }
    Ok(out)
}

/// Where xattrs are read from.
#[derive(Clone, Copy)]
enum XattrTarget<'a> {
    /// An open descriptor (regular files, directories).
    Fd(i32),
    /// A `/proc/self/fd/<dir>/<name>` path, not followed at its end.
    Path(&'a CString),
}

/// Largest xattr name list (64 KiB, the plan's xattr limit).
const MAX_XATTR_LIST: usize = 64 * 1024;
/// Largest single xattr value.
const MAX_XATTR_VALUE: usize = 64 * 1024;
/// Longest relative path.
const MAX_PATH: usize = 4096;
/// Default limit of entries per volume scan.
pub const DEFAULT_MAX_ENTRIES: usize = 1_000_000;

/// What a scan entry is in the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// Regular file with data.
    Regular,
    /// Directory (FT_DIREND).
    Directory,
    /// Symbolic link with its target (never followed).
    Symlink(Vec<u8>),
    /// FIFO (FT_SPEC).
    Fifo,
    /// Hard link to the entry with this relative path (FT_LNKSAVED).
    Hardlink(Vec<u8>),
    /// ownership.tsv generated by the host.
    Ownership(Vec<u8>),
}

impl EntryKind {
    /// The FileKind a guest must give for this entry.
    pub fn file_kind(&self) -> pb::FileKind {
        match self {
            Self::Regular | Self::Ownership(_) => pb::FileKind::Regular,
            Self::Directory => pb::FileKind::Directory,
            Self::Symlink(_) => pb::FileKind::Symlink,
            Self::Fifo => pb::FileKind::Fifo,
            Self::Hardlink(_) => pb::FileKind::Hardlink,
        }
    }
}

/// One entry of a volume scan.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Relative path below `data/` (empty: `data/` itself); for the
    /// ownership file `ownership.tsv` relative to the volume.
    pub relpath: Vec<u8>,
    /// Kind.
    pub kind: EntryKind,
    /// The stat as read (host IDs), with the mode already cleaned.
    pub stat: Stat,
    /// `user.*` xattrs for STREAM_XATTR_LINUX.
    pub xattrs: Vec<(Vec<u8>, Vec<u8>)>,
    /// Access ACL with host IDs (non-trivial only).
    pub acl_access: Vec<AclEntry>,
    /// Default ACL with host IDs (directories).
    pub acl_default: Vec<AclEntry>,
    /// The reference handed to the guest.
    pub source_ref: [u8; 16],
    /// The full path the guest must use.
    pub path: Vec<u8>,
}

/// An entry rejected from the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// Relative path below `data/`.
    pub relpath: Vec<u8>,
    /// Fixed reason.
    pub reason: Reason,
}

/// The scan of one volume.
#[derive(Debug)]
pub struct VolumeScan {
    /// The `_data` descriptor every reopen starts from.
    data: OwnedFd,
    /// Entries in scan order (`data/` first, ownership.tsv last).
    pub entries: Vec<Entry>,
    /// Rejected entries.
    pub rejected: Vec<Rejection>,
    /// Rows with special bits (for the restore report).
    pub special_rows: usize,
    /// Rows with capabilities.
    pub caps_rows: usize,
}

/// A fresh random reference.
fn new_ref() -> std::io::Result<[u8; 16]> {
    let mut r = [0u8; 16];
    // SAFETY: buffer of 16 bytes.
    let n = unsafe { libc::getrandom(r.as_mut_ptr().cast(), r.len(), 0) };
    if n != 16 {
        return Err(std::io::Error::other("getrandom failed"));
    }
    Ok(r)
}

/// The Stat record of a libc stat with cleaned mode (`& 0o777` plus type).
fn stat_of(st: &libc::stat, perm: u32, uid: u32, gid: u32) -> Stat {
    Stat {
        dev: st.st_dev,
        ino: st.st_ino,
        mode: (st.st_mode & libc::S_IFMT) | (perm & 0o777),
        nlink: st.st_nlink,
        uid,
        gid,
        rdev: 0,
        size: st.st_size,
        blksize: st.st_blksize,
        blocks: st.st_blocks,
        atime: st.st_atime,
        mtime: st.st_mtime,
        ctime: st.st_ctime,
        flags: 0,
    }
}

/// Scan state while walking.
struct Walker<'a> {
    /// `_data` descriptor.
    data: &'a OwnedFd,
    /// Device of `_data` (RESOLVE_NO_XDEV keeps us on it).
    dev: u64,
    /// The domain's maps.
    domain: &'a Domain,
    /// Path prefix of `data/`, e.g. `/@podman/uid-1000/volumes/v/data`.
    prefix: Vec<u8>,
    /// Entries so far.
    entries: Vec<Entry>,
    /// Rejections so far.
    rejected: Vec<Rejection>,
    /// First relpath of each multiply linked inode.
    links: HashMap<(u64, u64), Vec<u8>>,
    /// ownership.tsv rows.
    table: Table,
    /// Entry limit.
    max_entries: usize,
}

impl Walker<'_> {
    /// Full guest path of a relative path.
    fn full(&self, rel: &[u8]) -> Vec<u8> {
        let mut p = self.prefix.clone();
        if !rel.is_empty() {
            p.push(b'/');
            p.extend_from_slice(rel);
        }
        p
    }

    /// Scans `rel` (whose stat is `st`; `parent` is its directory's
    /// descriptor, None for `_data` itself) and below.
    fn visit(&mut self, parent: Option<&OwnedFd>, name: &[u8], rel: Vec<u8>, st: libc::stat) -> Result<(), String> {
        if self.entries.len() + self.rejected.len() >= self.max_entries {
            return Err(format!("volume has more than {} entries", self.max_entries));
        }
        if rel.len() > MAX_PATH {
            return Err("path too long in volume".into());
        }
        let kind = RawKind::of_mode(st.st_mode).ok_or("unknown file type")?;
        // Directories and regular files: open below _data, compare inode.
        let opened = match kind {
            RawKind::Directory | RawKind::Regular => {
                let flags = if kind == RawKind::Directory { libc::O_RDONLY | libc::O_DIRECTORY } else { libc::O_RDONLY | libc::O_NONBLOCK };
                let fd = openat2(self.data.as_raw_fd(), &rel, flags, RESOLVE).map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&rel)))?;
                let now = fstat(fd.as_raw_fd()).map_err(|e| e.to_string())?;
                if (now.st_dev, now.st_ino) != (st.st_dev, st.st_ino) {
                    return Err(format!("{} changed during the scan", String::from_utf8_lossy(&rel)));
                }
                Some(fd)
            }
            _ => None,
        };
        let xattrs = match (&opened, parent, kind) {
            (Some(fd), _, _) => read_xattrs(XattrTarget::Fd(fd.as_raw_fd())),
            (None, Some(dir), RawKind::Fifo | RawKind::Symlink) => {
                let mut p = format!("/proc/self/fd/{}/", dir.as_raw_fd()).into_bytes();
                p.extend_from_slice(name);
                let c = CString::new(p).map_err(|_| "NUL")?;
                read_xattrs(XattrTarget::Path(&c))
            }
            _ => Ok(Vec::new()),
        }
        .map_err(|e| format!("{}: xattrs: {e}", String::from_utf8_lossy(&rel)))?;
        let raw = RawEntry { relpath: rel.clone(), kind, mode: st.st_mode & 0o7777, uid: st.st_uid, gid: st.st_gid, xattrs: xattrs.clone() };
        let (attrs, row) = match sanitize::classify(&raw, self.domain) {
            Outcome::Rejected(reason) => {
                self.rejected.push(Rejection { relpath: rel, reason });
                return Ok(());
            }
            Outcome::Stream { attrs, row } => (attrs, row),
        };
        let (acl_access, acl_default) = acls(&xattrs);
        let entry_kind = match kind {
            RawKind::Regular if st.st_nlink > 1 => match self.links.get(&(st.st_dev, st.st_ino)) {
                Some(first) => EntryKind::Hardlink(first.clone()),
                None => {
                    self.links.insert((st.st_dev, st.st_ino), rel.clone());
                    EntryKind::Regular
                }
            },
            RawKind::Regular => EntryKind::Regular,
            RawKind::Directory => EntryKind::Directory,
            RawKind::Fifo => EntryKind::Fifo,
            RawKind::Symlink => {
                let dir = parent.ok_or("the volume root is a link")?;
                EntryKind::Symlink(readlinkat(dir.as_raw_fd(), name).map_err(|e| e.to_string())?)
            }
            RawKind::Char | RawKind::Block | RawKind::Socket => unreachable!("classify rejects these"),
        };
        if let Some(row) = row {
            self.table.push(row)?;
        }
        let path = self.full(&rel);
        self.entries.push(Entry {
            relpath: rel.clone(),
            kind: entry_kind,
            stat: stat_of(&st, attrs.perm, attrs.uid, attrs.gid),
            xattrs: attrs.xattrs,
            acl_access,
            acl_default,
            source_ref: new_ref().map_err(|e| e.to_string())?,
            path,
        });
        if kind == RawKind::Directory {
            let dir = opened.expect("opened above");
            let mut names = list_dir(&dir).map_err(|e| format!("{}: {e}", String::from_utf8_lossy(&rel)))?;
            names.sort();
            for child in names {
                let cst = fstatat(dir.as_raw_fd(), &child).map_err(|e| e.to_string())?;
                if cst.st_dev != self.dev {
                    // Another filesystem below the volume: not followed.
                    continue;
                }
                let mut crel = rel.clone();
                if !crel.is_empty() {
                    crel.push(b'/');
                }
                crel.extend_from_slice(&child);
                self.visit(Some(&dir), &child, crel, cst)?;
            }
        }
        Ok(())
    }
}

/// The non-trivial access ACL and the default ACL (host IDs) of an xattr list.
fn acls(xattrs: &[(Vec<u8>, Vec<u8>)]) -> (Vec<AclEntry>, Vec<AclEntry>) {
    let get = |n: &[u8]| xattrs.iter().find(|(k, _)| k == n).and_then(|(_, v)| sanitize::parse_acl(v)).unwrap_or_default();
    let access = get(b"system.posix_acl_access");
    let trivial = access.iter().all(|e| matches!(e.tag, 0x01 | 0x04 | 0x20));
    (if trivial { Vec::new() } else { access }, get(b"system.posix_acl_default"))
}

/// The ACL text of a stream (`acl_to_text` form, numeric IDs, never names:
/// the IDs are host IDs of this host).
pub fn acl_stream_text(entries: &[AclEntry]) -> String {
    let perm = |p: u16| [(4, 'r'), (2, 'w'), (1, 'x')].iter().map(|&(b, c)| if p & b != 0 { c } else { '-' }).collect::<String>();
    entries
        .iter()
        .map(|e| match e.tag {
            0x01 => format!("user::{}\n", perm(e.perm)),
            0x02 => format!("user:{}:{}\n", e.id, perm(e.perm)),
            0x04 => format!("group::{}\n", perm(e.perm)),
            0x08 => format!("group:{}:{}\n", e.id, perm(e.perm)),
            0x10 => format!("mask::{}\n", perm(e.perm)),
            _ => format!("other::{}\n", perm(e.perm)),
        })
        .collect()
}

/// readlinkat without following.
fn readlinkat(dir: i32, name: &[u8]) -> std::io::Result<Vec<u8>> {
    let c = CString::new(name).map_err(|_| std::io::Error::other("NUL"))?;
    let mut buf = vec![0u8; MAX_PATH + 1];
    // SAFETY: valid descriptor, name and buffer.
    let n = unsafe { libc::readlinkat(dir, c.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if n as usize > MAX_PATH {
        return Err(std::io::Error::other("link target too long"));
    }
    buf.truncate(n as usize);
    Ok(buf)
}

/// Names in a directory (without `.` and `..`), read through the open
/// descriptor (the `/proc/self/fd/<n>` link is the descriptor's own inode,
/// not a new path lookup).
fn list_dir(dir: &OwnedFd) -> std::io::Result<Vec<Vec<u8>>> {
    std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?
        .map(|e| e.map(|e| e.file_name().as_bytes().to_vec()))
        .collect()
}

/// Opens the graphroot of the account policy from `/` without following a
/// link anywhere (the policy names the real path).
pub fn open_graphroot(path: &Path) -> Result<OwnedFd, String> {
    if !path.is_absolute() {
        return Err("graphroot must be absolute".into());
    }
    let root = openat2(libc::AT_FDCWD, b"/", libc::O_RDONLY | libc::O_DIRECTORY, 0).map_err(|e| e.to_string())?;
    let rel = path.strip_prefix("/").map_err(|_| "graphroot")?;
    openat2(root.as_raw_fd(), rel.as_os_str().as_bytes(), libc::O_RDONLY | libc::O_DIRECTORY, libc::RESOLVE_NO_SYMLINKS)
        .map_err(|e| format!("graphroot {}: {e}", path.display()))
}

/// A volume name as Podman allows it.
pub fn valid_volume(name: &str) -> bool {
    onesimus_podman_core::exec::valid_name(name)
}

/// Scans `volumes/<name>/_data` below `graphroot`.
pub fn scan_volume(graphroot: &OwnedFd, name: &str, domain: &Domain, domain_id: &str, prefix: &[u8], max_entries: usize,
    max_rows: usize) -> Result<VolumeScan, String> {
    if !valid_volume(name) {
        return Err("invalid volume name".into());
    }
    let rel = format!("volumes/{name}/_data");
    let data = openat2(graphroot.as_raw_fd(), rel.as_bytes(), libc::O_RDONLY | libc::O_DIRECTORY, RESOLVE)
        .map_err(|e| format!("volume {name}: {e}"))?;
    let st = fstat(data.as_raw_fd()).map_err(|e| e.to_string())?;
    let mut w = Walker {
        data: &data,
        dev: st.st_dev,
        domain,
        prefix: [prefix, b"/data"].concat(),
        entries: Vec::new(),
        rejected: Vec::new(),
        links: HashMap::new(),
        table: Table::new(domain_id, name, domain.uid_map.clone(), domain.gid_map.clone(), max_rows),
        max_entries,
    };
    w.visit(None, b"", Vec::new(), st)?;
    let (special_rows, caps_rows) = (w.table.special_count(), w.table.caps_count());
    let tsv = w.table.render();
    let mut entries = std::mem::take(&mut w.entries);
    let rejected = std::mem::take(&mut w.rejected);
    // ownership.tsv: synthetic attributes like every plugin file (S-13):
    // 0600, owned by the euid/egid of ofd (root in production).
    // SAFETY: geteuid/getegid have no preconditions.
    let (euid, egid) = unsafe { (libc::geteuid(), libc::getegid()) };
    entries.push(Entry {
        relpath: b"ownership.tsv".to_vec(),
        stat: Stat {
            uid: euid,
            gid: egid,
            mode: libc::S_IFREG | 0o600,
            nlink: 1,
            size: tsv.len() as i64,
            blksize: 4096,
            mtime: st.st_mtime,
            atime: st.st_mtime,
            ctime: st.st_mtime,
            ..Default::default()
        },
        kind: EntryKind::Ownership(tsv),
        xattrs: Vec::new(),
        acl_access: Vec::new(),
        acl_default: Vec::new(),
        source_ref: new_ref().map_err(|e| e.to_string())?,
        path: [prefix, b"/ownership.tsv"].concat(),
    });
    Ok(VolumeScan { data, entries, rejected, special_rows, caps_rows })
}

/// What `backup.rs` gets for one reference.
#[derive(Debug)]
pub struct Taken {
    /// The entry (attributes from the host's own table).
    pub entry: Entry,
    /// For regular files: the reopened file, checked against the scan.
    pub file: Option<File>,
    /// For a hard link: the full path of its target.
    pub link_target: Option<Vec<u8>>,
}

/// All scans of one plugin instance: references are valid once.
#[derive(Debug, Default)]
pub struct ScanTable {
    /// Scans by volume name.
    volumes: HashMap<String, VolumeScan>,
    /// Reference → (volume, index); removed on use.
    refs: HashMap<[u8; 16], (String, usize)>,
    /// Full paths of entries already saved (hard link targets).
    saved: HashMap<Vec<u8>, i32>,
}

/// Shared between the port (hands out references) and the session (takes them).
pub type SharedScans = Arc<Mutex<ScanTable>>;

impl ScanTable {
    /// Adds a scan; its references become valid. A volume is scanned once.
    pub fn insert(&mut self, name: &str, scan: VolumeScan) -> Result<(), String> {
        if self.volumes.contains_key(name) {
            return Err(format!("volume {name} scanned twice"));
        }
        for (i, e) in scan.entries.iter().enumerate() {
            self.refs.insert(e.source_ref, (name.to_string(), i));
        }
        self.volumes.insert(name.to_string(), scan);
        Ok(())
    }

    /// The scan of a volume.
    pub fn get(&self, name: &str) -> Option<&VolumeScan> {
        self.volumes.get(name)
    }

    /// Takes the entry of `source_ref` for a guest entry with `path` and
    /// `kind`: unknown, reused, foreign path or wrong kind are refused.
    pub fn take(&mut self, source_ref: &[u8], path: &[u8], kind: i32) -> Result<Taken, String> {
        let key: [u8; 16] = source_ref.try_into().map_err(|_| "unknown scan reference")?;
        let (volume, index) = self.refs.get(&key).cloned().ok_or("unknown or already used scan reference")?;
        let scan = self.volumes.get(&volume).ok_or("unknown scan reference")?;
        let entry = scan.entries[index].clone();
        let guest = crate::backup::plugin_path(path).ok_or("invalid path for a scan reference")?;
        if guest != entry.path {
            return Err("path does not match its scan reference".into());
        }
        if kind != entry.kind.file_kind() as i32 {
            return Err("kind does not match its scan reference".into());
        }
        let link_target = match &entry.kind {
            EntryKind::Hardlink(first) => {
                let full = scan.entries.iter().find(|e| &e.relpath == first && e.kind == EntryKind::Regular).map(|e| e.path.clone())
                    .ok_or("hard link target missing")?;
                if !self.saved.contains_key(&full) {
                    return Err("hard link listed before its target".into());
                }
                Some(full)
            }
            _ => None,
        };
        let file = match entry.kind {
            EntryKind::Regular => {
                let fd = openat2(scan.data.as_raw_fd(), &entry.relpath, libc::O_RDONLY | libc::O_NONBLOCK, RESOLVE)
                    .map_err(|e| format!("reopening: {e}"))?;
                let st = fstat(fd.as_raw_fd()).map_err(|e| e.to_string())?;
                if (st.st_dev, st.st_ino) != (entry.stat.dev, entry.stat.ino) || st.st_mode & libc::S_IFMT != libc::S_IFREG {
                    return Err("file replaced since the scan".into());
                }
                Some(File::from(fd))
            }
            _ => None,
        };
        // Once only.
        self.refs.remove(&key);
        Ok(Taken { entry, file, link_target })
    }

    /// Records the file index of a saved entry (target of later hard links).
    pub fn mark_saved(&mut self, path: Vec<u8>, file_index: i32) {
        self.saved.insert(path, file_index);
    }

    /// The file index of a saved entry.
    pub fn saved_index(&self, path: &[u8]) -> Option<i32> {
        self.saved.get(path).copied()
    }
}

/// The relative path of an entry as text (for messages).
pub fn shown(rel: &[u8]) -> String {
    if rel.is_empty() { ".".into() } else { OsStr::from_bytes(rel).to_string_lossy().into_owned() }
}

/// ownership.tsv parsing re-exported for the restore and tests.
pub fn parse_ownership(bytes: &[u8]) -> Result<Vec<onesimus_podman_core::sanitize::OwnershipRow>, String> {
    ownership::parse(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use onesimus_podman_core::idmap::IdMap;
    use std::os::unix::fs::PermissionsExt;

    fn me() -> (u32, u32) {
        // SAFETY: getuid/getgid have no preconditions.
        unsafe { (libc::getuid(), libc::getgid()) }
    }
    /// A graphroot with one volume `v` holding a few kinds of entries.
    fn fixture(tag: &str) -> std::path::PathBuf {
        let g = std::env::temp_dir().join(format!("ofd-podscan-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let d = g.join("volumes/v/_data");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/file"), b"hello").unwrap();
        std::fs::set_permissions(d.join("sub/file"), std::fs::Permissions::from_mode(0o4755)).unwrap();
        std::fs::hard_link(d.join("sub/file"), d.join("hard")).unwrap();
        std::os::unix::fs::symlink("/etc/shadow", d.join("link")).unwrap();
        nix::unistd::mkfifo(&d.join("fifo"), nix::sys::stat::Mode::from_bits_truncate(0o600)).unwrap();
        let _ = xattr_set(&d.join("sub/file"), "user.k", b"v");
        g
    }
    fn xattr_set(p: &Path, name: &str, value: &[u8]) -> std::io::Result<()> {
        let cp = CString::new(p.as_os_str().as_bytes()).unwrap();
        let cn = CString::new(name).unwrap();
        // SAFETY: valid strings and buffer.
        let r = unsafe { libc::setxattr(cp.as_ptr(), cn.as_ptr(), value.as_ptr().cast(), value.len(), 0) };
        if r < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
    }
    fn domain() -> Domain {
        let (u, g) = me();
        Domain { uid_map: IdMap::rootless(u, &[(u + 1_000_000, 1000)]).unwrap(), gid_map: IdMap::rootless(g, &[(g + 1_000_000, 1000)]).unwrap(), rootless: true }
    }
    fn scan(g: &Path) -> VolumeScan {
        let root = open_graphroot(g).unwrap();
        scan_volume(&root, "v", &domain(), "uid-1", b"/@podman/uid-1/volumes/v", 1000, 1000).unwrap()
    }

    #[test]
    fn scan_reads_kinds_cleans_modes_and_writes_ownership() {
        let g = fixture("kinds");
        let s = scan(&g);
        let by = |r: &[u8]| s.entries.iter().find(|e| e.relpath == r).unwrap_or_else(|| panic!("{}", shown(r)));
        assert_eq!(s.entries[0].relpath, b"");
        assert_eq!(s.entries[0].path, b"/@podman/uid-1/volumes/v/data");
        let file = by(b"sub/file");
        assert_eq!(file.stat.mode, libc::S_IFREG | 0o755, "setuid never in the stream");
        assert_eq!(file.stat.uid, me().0, "host IDs");
        // The first name of the inode is the file, the second a hard link to it.
        assert!(matches!(by(b"hard").kind, EntryKind::Regular));
        assert_eq!(by(b"sub/file").kind, EntryKind::Hardlink(b"hard".to_vec()));
        assert_eq!(by(b"link").kind, EntryKind::Symlink(b"/etc/shadow".to_vec()));
        assert_eq!(by(b"fifo").kind, EntryKind::Fifo);
        let tsv = s.entries.last().unwrap();
        assert_eq!(tsv.path, b"/@podman/uid-1/volumes/v/ownership.tsv");
        let EntryKind::Ownership(bytes) = &tsv.kind else { panic!() };
        let rows = parse_ownership(bytes).unwrap();
        assert!(rows.iter().any(|r| r.relpath == b"sub/file" && r.mode == 0o4755 && r.ns_uid == 0), "{rows:?}");
        assert_eq!(s.special_rows, 2, "both names of the setuid inode");
        std::fs::remove_dir_all(&g).unwrap();
    }

    /// Exit criterion P0 (2): foreign, reused, mismatching references.
    #[test]
    fn references_are_checked_and_used_once() {
        let g = fixture("refs");
        let s = scan(&g);
        let hard = s.entries.iter().find(|e| e.relpath == b"hard").unwrap().clone();
        let file = s.entries.iter().find(|e| e.relpath == b"sub/file").unwrap().clone();
        let fifo = s.entries.iter().find(|e| e.relpath == b"fifo").unwrap().clone();
        let mut t = ScanTable::default();
        t.insert("v", s).unwrap();
        let reg = pb::FileKind::Regular as i32;
        // Foreign reference.
        assert!(t.take(&[7u8; 16], &hard.path, reg).is_err());
        assert!(t.take(b"short", &hard.path, reg).is_err());
        // Wrong path (another entry's, or outside the volume).
        assert!(t.take(&hard.source_ref, &fifo.path, reg).is_err());
        assert!(t.take(&hard.source_ref, b"/@podman/uid-1/volumes/v/data/../../x", reg).is_err());
        // Wrong kind.
        assert!(t.take(&hard.source_ref, &hard.path, pb::FileKind::Virtual as i32).is_err());
        // A hard link before its target.
        assert!(t.take(&file.source_ref, &file.path, pb::FileKind::Hardlink as i32).is_err());
        // Right: once.
        let taken = t.take(&hard.source_ref, &hard.path, reg).unwrap();
        let mut data = String::new();
        std::io::Read::read_to_string(&mut taken.file.unwrap(), &mut data).unwrap();
        assert_eq!(data, "hello");
        assert!(t.take(&hard.source_ref, &hard.path, reg).is_err(), "a reference is valid once");
        t.mark_saved(hard.path.clone(), 3);
        let link = t.take(&file.source_ref, &file.path, pb::FileKind::Hardlink as i32).unwrap();
        assert_eq!(link.link_target.unwrap(), hard.path);
        std::fs::remove_dir_all(&g).unwrap();
    }

    /// A symlink planted in place of _data or a path component is not followed.
    #[test]
    fn links_in_the_volume_path_are_not_followed() {
        let g = fixture("planted");
        std::fs::remove_dir_all(g.join("volumes/v/_data")).unwrap();
        std::os::unix::fs::symlink("/etc", g.join("volumes/v/_data")).unwrap();
        let root = open_graphroot(&g).unwrap();
        assert!(scan_volume(&root, "v", &domain(), "uid-1", b"/p", 10, 10).is_err());
        assert!(scan_volume(&root, "../v", &domain(), "uid-1", b"/p", 10, 10).is_err());
        std::fs::remove_dir_all(&g).unwrap();
        let link = std::env::temp_dir().join(format!("ofd-podscan-link-{}", std::process::id()));
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/tmp", &link).unwrap();
        assert!(open_graphroot(&link).is_err(), "a linked graphroot is refused");
        std::fs::remove_file(&link).unwrap();
    }

    /// Exit criterion P0 (5) on disk: an ID outside the map is rejected.
    #[test]
    fn foreign_ids_are_rejected_not_streamed() {
        let g = fixture("foreign");
        let (u, gid) = me();
        let narrow = Domain { uid_map: IdMap::rootless(u + 1, &[]).unwrap(), gid_map: IdMap::rootless(gid, &[]).unwrap(), rootless: true };
        let root = open_graphroot(&g).unwrap();
        let s = scan_volume(&root, "v", &narrow, "uid-1", b"/p", 100, 100).unwrap();
        assert_eq!(s.entries.len(), 1, "only ownership.tsv");
        assert_eq!(s.rejected, vec![Rejection { relpath: Vec::new(), reason: Reason::IdOutsideMap }]);
        std::fs::remove_dir_all(&g).unwrap();
    }

    #[test]
    fn acl_stream_text_is_numeric() {
        let e = |tag, perm, id| AclEntry { tag, perm, id };
        assert_eq!(acl_stream_text(&[e(1, 7, 0), e(2, 5, 100999), e(4, 5, 0), e(0x10, 5, 0), e(0x20, 0, 0)]),
            "user::rwx\nuser:100999:r-x\ngroup::r-x\nmask::r-x\nother::---\n");
    }
}
