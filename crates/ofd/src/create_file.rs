//! findlib/create_file.cc CreateFile() and findlib/mkpath.cc makepath().

use std::collections::HashSet;
#[cfg(unix)]
use std::ffi::CString;
use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::attribs::*;
#[cfg(windows)]
use crate::platform::OsStrExt;

// include/baconfig.h
#[allow(dead_code)]
pub const REPLACE_ALWAYS: u8 = b'a';
pub const REPLACE_IFNEWER: u8 = b'w';
pub const REPLACE_NEVER: u8 = b'n';
pub const REPLACE_IFOLDER: u8 = b'o';

/// CreateFile() result.
pub enum Cf {
    /// CF_SKIP
    Skip,
    /// CF_ERROR
    Error,
    /// CF_EXTRACT: file created, data follows into this file.
    Extract(File),
    /// CF_CREATED: created, no data to restore.
    Created,
}

/// Restore-wide state CreateFile() keeps in the jcr.
#[derive(Default)]
pub struct CreateState {
    /// jcr->keep_path_list / jcr->path_list (replace=never): directories this
    /// restore created.
    pub keep_path_list: bool,
    pub path_list: HashSet<Vec<u8>>,
    /// jcr->cached_path (PathAlreadySeen)
    pub cached_path: Option<Vec<u8>>,
}

#[cfg(unix)]
fn cpath(p: &[u8]) -> CString {
    CString::new(p.to_vec()).unwrap_or_default()
}

#[cfg(unix)]
fn errno() -> crate::berrno::BErr {
    crate::berrno::BErr::last()
}

#[cfg(unix)]
fn lstat(p: &[u8]) -> Option<libc::stat> {
    let c = cpath(p);
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::lstat(c.as_ptr(), &mut st) } == 0).then_some(st)
}

#[cfg(unix)]
fn stat(p: &[u8]) -> Option<libc::stat> {
    let c = cpath(p);
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    (unsafe { libc::stat(c.as_ptr(), &mut st) } == 0).then_some(st)
}

#[cfg(unix)]
fn is_dir(st: &libc::stat) -> bool {
    st.st_mode & libc::S_IFMT == libc::S_IFDIR
}

#[cfg(unix)]
/// CreateFile(). Messages (Qmsg/Jmsg) go to `msg(type, text)`.
pub fn create_file(
    attr: &mut Attributes,
    replace: u8,
    state: &mut CreateState,
    msg: &mut dyn FnMut(i32, String),
) -> Cf {
    use crate::jmsg::{M_ERROR, M_INFO};
    let new_mode = attr.statp.mode;
    let parent_mode = crate::stmode::S_IWUSR | crate::stmode::S_IXUSR | new_mode;
    let (uid, gid) = (attr.statp.uid, attr.statp.gid);
    let oname = attr.ofname_str();

    let mut exists = false;
    if let Some(m) = lstat(&attr.ofname) {
        exists = true;
        match replace {
            REPLACE_IFNEWER if attr.statp.mtime <= m.st_mtime => {
                msg(M_INFO, format!("File skipped. Not newer: {oname}\n"));
                return Cf::Skip;
            }
            REPLACE_IFOLDER if attr.statp.mtime >= m.st_mtime => {
                msg(M_INFO, format!("File skipped. Not older: {oname}\n"));
                return Cf::Skip;
            }
            REPLACE_NEVER => {
                if !(attr.type_ == FT_DIREND && state.path_list.contains(&attr.ofname)) {
                    msg(M_INFO, format!("File skipped. Already exists: {oname}\n"));
                    return Cf::Skip;
                }
            }
            _ => {}
        }
    }

    match attr.type_ {
        FT_RAW | FT_FIFO | FT_LNKSAVED | FT_LNK | FT_SPEC | FT_REGE | FT_REG => {
            if exists && attr.type_ != FT_RAW && attr.type_ != FT_FIFO {
                // SecureErase() without a secure erase command: unlink()
                let c = cpath(&attr.ofname);
                if unsafe { libc::unlink(c.as_ptr()) } == -1 {
                    msg(M_ERROR, format!("File {oname} already exists and could not be replaced. ERR={}.\n", errno()));
                }
            }
            // SeparatePathAndFile()
            let Some(slash) = attr.ofname.iter().rposition(|&b| b == b'/') else {
                if attr.ofname.is_empty() {
                    msg(M_ERROR, format!("Zero length filename: {}\n", attr.fname_str()));
                    return Cf::Error;
                }
                return create_typed(attr, exists, msg);
            };
            if slash + 1 == attr.ofname.len() {
                msg(M_ERROR, format!("Zero length filename: {}\n", attr.fname_str()));
                return Cf::Error;
            }
            if slash > 0 {
                let dir = attr.ofname[..slash].to_vec();
                if state.cached_path.as_ref() != Some(&dir) {
                    state.cached_path = Some(dir.clone());
                    if !makepath(attr, &dir, parent_mode, parent_mode, uid, gid, true, state, msg) {
                        return Cf::Error;
                    }
                }
            }
            create_typed(attr, exists, msg)
        }
        FT_REPARSE | FT_JUNCTION | FT_DIRBEGIN | FT_DIREND => {
            let path = attr.ofname.clone();
            if !makepath(attr, &path, new_mode, parent_mode, uid, gid, false, state, msg) {
                return Cf::Error;
            }
            Cf::Created
        }
        FT_DELETED => {
            msg(M_INFO, format!("Original file {} have been deleted: type={}\n", attr.fname_str(), attr.type_));
            Cf::Error
        }
        FT_NOACCESS | FT_NOFOLLOW | FT_NOSTAT | FT_DIRNOCHG | FT_NOCHG | FT_ISARCH | FT_NORECURSE | FT_NOFSCHG | FT_NOOPEN => {
            msg(M_ERROR, format!("Original file {} not saved: type={}\n", attr.fname_str(), attr.type_));
            Cf::Error
        }
        t => {
            msg(M_ERROR, format!("Unknown file type {t}; not restored: {}\n", attr.fname_str()));
            Cf::Error
        }
    }
}

#[cfg(unix)]
/// The type specific part of CreateFile() for non-directories.
fn create_typed(attr: &mut Attributes, exists: bool, msg: &mut dyn FnMut(i32, String)) -> Cf {
    use crate::jmsg::M_ERROR;
    let oname = attr.ofname_str();
    let mode = attr.statp.mode;
    let fmt = mode & crate::stmode::S_IFMT;
    let open_write = |create: bool| -> std::io::Result<File> {
        let mut o = std::fs::OpenOptions::new();
        o.write(true);
        if create {
            o.create(true).truncate(true).mode(0o600);
        }
        o.open(attr.ofname_path())
    };
    match attr.type_ {
        FT_REGE | FT_REG => match open_write(true) {
            Ok(f) => Cf::Extract(f),
            Err(e) => {
                msg(M_ERROR, format!("Could not create {oname}: ERR={}\n", crate::berrno::s(&e)));
                Cf::Error
            }
        },
        FT_RAW | FT_FIFO | FT_SPEC => {
            let c = cpath(&attr.ofname);
            let is_on_root = attr.fname == attr.ofname;
            let mut create = false;
            if fmt == crate::stmode::S_IFIFO {
                if unsafe { libc::mkfifo(c.as_ptr(), mode as libc::mode_t) } != 0 && errno().raw_os_error() != Some(libc::EEXIST) {
                    msg(M_ERROR, format!("Cannot make fifo {oname}: ERR={}\n", errno()));
                    return Cf::Error;
                }
            } else if fmt == crate::stmode::S_IFSOCK {
                // Skipping restore of socket
            } else if (fmt == crate::stmode::S_IFBLK || fmt == crate::stmode::S_IFCHR) && !exists && is_on_root {
                msg(M_ERROR, format!("Device restore on root failed, device {} missing.\n", attr.fname_str()));
                return Cf::Error;
            } else if fmt == crate::stmode::S_IFBLK || fmt == crate::stmode::S_IFCHR {
                // Restoring a device as a file
                create = true;
            } else if unsafe { libc::mknod(c.as_ptr(), mode as libc::mode_t, attr.statp.rdev as libc::dev_t) } != 0
                && errno().raw_os_error() != Some(libc::EEXIST)
            {
                msg(M_ERROR, format!("Cannot make node {oname}: ERR={}\n", errno()));
                return Cf::Error;
            }
            if attr.type_ == FT_RAW || attr.type_ == FT_FIFO {
                // The original times the FIFO open out after 60 seconds;
                // a non-blocking open fails at once without a reader.
                let r = if attr.type_ == FT_FIFO {
                    let mut o = std::fs::OpenOptions::new();
                    o.write(true).custom_flags(libc::O_NONBLOCK);
                    o.open(attr.ofname_path()).and_then(|f| {
                        let fd = std::os::fd::AsRawFd::as_raw_fd(&f);
                        unsafe { libc::fcntl(fd, libc::F_SETFL, 0) };
                        Ok(f)
                    })
                } else {
                    open_write(create)
                };
                return match r {
                    Ok(f) => Cf::Extract(f),
                    Err(e) => {
                        msg(M_ERROR, format!("Could not open {oname}: ERR={}\n", crate::berrno::s(&e)));
                        Cf::Error
                    }
                };
            }
            Cf::Created
        }
        FT_LNKSAVED => {
            let from = cpath(&attr.olname);
            let to = cpath(&attr.ofname);
            if unsafe { libc::link(from.as_ptr(), to.as_ptr()) } != 0 {
                msg(M_ERROR, format!("Could not hard link {oname} -> {}: ERR={}\n", String::from_utf8_lossy(&attr.olname), errno()));
                return Cf::Error;
            }
            Cf::Created
        }
        FT_LNK => {
            let target = cpath(&attr.olname);
            let to = cpath(&attr.ofname);
            if unsafe { libc::symlink(target.as_ptr(), to.as_ptr()) } != 0 && errno().raw_os_error() != Some(libc::EEXIST) {
                msg(M_ERROR, format!("Could not symlink {oname} -> {}: ERR={}\n", String::from_utf8_lossy(&attr.olname), errno()));
                return Cf::Error;
            }
            Cf::Created
        }
        t => {
            msg(M_ERROR, format!("unhandled type: {t}\n"));
            Cf::Error
        }
    }
}

#[cfg(unix)]
/// SetOwnMod(): warnings only when running as root.
fn set_own_mod(path: &[u8], owner: u32, group: u32, mode: u32, msg: &mut dyn FnMut(i32, String)) {
    let root = nix::unistd::getuid().is_root();
    let c = cpath(path);
    let p = String::from_utf8_lossy(path);
    if unsafe { libc::lchown(c.as_ptr(), owner, group) } != 0 && root {
        msg(crate::jmsg::M_WARNING, format!("Cannot change owner and/or group of {p}: ERR={}\n", errno()));
    }
    if unsafe { libc::chmod(c.as_ptr(), crate::stmode::to_mode_t(mode & 0o7777)) } != 0 && root {
        msg(crate::jmsg::M_WARNING, format!("Cannot change permissions of {p}: ERR={}\n", errno()));
    }
}

#[cfg(unix)]
/// makedir(): Some(created) or None on error.
fn makedir(path: &[u8], state: &mut CreateState, msg: &mut dyn FnMut(i32, String)) -> Option<bool> {
    let c = cpath(path);
    if unsafe { libc::mkdir(c.as_ptr(), 0o777) } != 0 {
        let e = errno();
        return match stat(path) {
            None => {
                msg(crate::jmsg::M_ERROR, format!("Cannot create directory {}: ERR={e}\n", String::from_utf8_lossy(path)));
                None
            }
            Some(st) if !is_dir(&st) => {
                msg(crate::jmsg::M_ERROR, format!("{} exists but is not a directory.\n", String::from_utf8_lossy(path)));
                None
            }
            Some(_) => Some(false),
        };
    }
    if state.keep_path_list {
        state.path_list.insert(path.to_vec());
    }
    Some(true)
}

#[cfg(unix)]
/// makepath(): create `apath` and missing parents; created parents get
/// `parent_mode`, the directory itself `mode` (unless keep_dir_modes).
#[allow(clippy::too_many_arguments)]
pub fn makepath(
    attr: &Attributes,
    apath: &[u8],
    mode: u32,
    parent_mode: u32,
    owner: u32,
    group: u32,
    keep_dir_modes: bool,
    state: &mut CreateState,
    msg: &mut dyn FnMut(i32, String),
) -> bool {
    if let Some(st) = stat(apath) {
        if !is_dir(&st) {
            msg(crate::jmsg::M_ERROR, format!("{} exists but is not a directory.\n", String::from_utf8_lossy(apath)));
            return false;
        }
        if !keep_dir_modes {
            set_own_mod(apath, owner, group, mode, msg);
        }
        return true;
    }
    let mut path = apath.to_vec();
    while path.len() > 1 && path.last() == Some(&b'/') {
        path.pop();
    }
    // every component, then the full path
    let mut created: Vec<(usize, bool)> = Vec::new();
    let mut i = 0;
    while i < path.len() && path[i] == b'/' {
        i += 1;
    }
    while let Some(off) = path[i..].iter().position(|&b| b == b'/') {
        let end = i + off;
        match makedir(&path[..end], state, msg) {
            Some(c) => created.push((end, c)),
            None => return false,
        }
        i = end;
        while i < path.len() && path[i] == b'/' {
            i += 1;
        }
    }
    if attr.type_ != FT_JUNCTION {
        match makedir(&path, state, msg) {
            Some(c) => created.push((path.len(), c)),
            None => return false,
        }
    }
    if !keep_dir_modes {
        let n = created.len();
        for (k, (end, was_created)) in created.into_iter().enumerate() {
            if was_created {
                set_own_mod(&path[..end], owner, group, if k + 1 == n { mode } else { parent_mode }, msg);
            }
        }
    }
    true
}

/// CreateFile() on Windows through `std::fs`: the same decisions as the
/// Unix one (replace policy, parents, one type per branch), no owner, mode
/// or device nodes. Symbolic links need the privilege to create them
/// (an administrator or developer mode); without it the file is reported
/// as "Could not symlink". Hard links need an NTFS volume.
#[cfg(windows)]
pub fn create_file(
    attr: &mut Attributes,
    replace: u8,
    state: &mut CreateState,
    msg: &mut dyn FnMut(i32, String),
) -> Cf {
    use crate::jmsg::{M_ERROR, M_INFO};
    let oname = attr.ofname_str();
    let mtime_of = |m: &std::fs::Metadata| {
        m.modified().ok().map(|t| match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(e) => -(e.duration().as_secs() as i64),
        })
    };
    let existing = std::fs::symlink_metadata(attr.ofname_path()).ok();
    let exists = existing.is_some();
    if let Some(m) = &existing {
        let theirs = mtime_of(m).unwrap_or(0);
        match replace {
            REPLACE_IFNEWER if attr.statp.mtime <= theirs => {
                msg(M_INFO, format!("File skipped. Not newer: {oname}\n"));
                return Cf::Skip;
            }
            REPLACE_IFOLDER if attr.statp.mtime >= theirs => {
                msg(M_INFO, format!("File skipped. Not older: {oname}\n"));
                return Cf::Skip;
            }
            REPLACE_NEVER => {
                if !(attr.type_ == FT_DIREND && state.path_list.contains(&attr.ofname)) {
                    msg(M_INFO, format!("File skipped. Already exists: {oname}\n"));
                    return Cf::Skip;
                }
            }
            _ => {}
        }
    }
    match attr.type_ {
        FT_LNKSAVED | FT_LNK | FT_REGE | FT_REG => {
            if exists {
                if let Err(e) = remove_existing(attr.ofname_path()) {
                    msg(M_ERROR, format!("File {oname} already exists and could not be replaced. ERR={}.\n", crate::berrno::s(&e)));
                }
            }
            let Some(slash) = attr.ofname.iter().rposition(|&b| b == b'/') else {
                if attr.ofname.is_empty() {
                    msg(M_ERROR, format!("Zero length filename: {}\n", attr.fname_str()));
                    return Cf::Error;
                }
                return create_typed(attr, msg);
            };
            if slash + 1 == attr.ofname.len() {
                msg(M_ERROR, format!("Zero length filename: {}\n", attr.fname_str()));
                return Cf::Error;
            }
            if slash > 0 {
                let dir = attr.ofname[..slash].to_vec();
                if state.cached_path.as_ref() != Some(&dir) {
                    state.cached_path = Some(dir.clone());
                    if !makepath(&dir, state, msg) {
                        return Cf::Error;
                    }
                }
            }
            create_typed(attr, msg)
        }
        FT_REPARSE | FT_JUNCTION | FT_DIRBEGIN | FT_DIREND => {
            let path = attr.ofname.clone();
            if !makepath(&path, state, msg) {
                return Cf::Error;
            }
            Cf::Created
        }
        FT_RAW | FT_FIFO | FT_SPEC => {
            msg(M_ERROR, format!("Cannot restore {oname}: devices, FIFOs and sockets do not exist on Windows.\n"));
            Cf::Error
        }
        FT_DELETED => {
            msg(M_INFO, format!("Original file {} have been deleted: type={}\n", attr.fname_str(), attr.type_));
            Cf::Error
        }
        FT_NOACCESS | FT_NOFOLLOW | FT_NOSTAT | FT_DIRNOCHG | FT_NOCHG | FT_ISARCH | FT_NORECURSE | FT_NOFSCHG | FT_NOOPEN => {
            msg(M_ERROR, format!("Original file {} not saved: type={}\n", attr.fname_str(), attr.type_));
            Cf::Error
        }
        t => {
            msg(M_ERROR, format!("Unknown file type {t}; not restored: {}\n", attr.fname_str()));
            Cf::Error
        }
    }
}

/// Removes a file or link that is in the way; a read-only file loses its
/// read-only attribute first (Windows refuses to delete it otherwise).
#[cfg(windows)]
fn remove_existing(path: &std::path::Path) -> std::io::Result<()> {
    if let Ok(m) = std::fs::symlink_metadata(path) {
        let mut perm = m.permissions();
        if perm.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(path, perm);
        }
        if m.file_type().is_dir() || (m.file_type().is_symlink() && std::os::windows::fs::FileTypeExt::is_symlink_dir(&m.file_type())) {
            return std::fs::remove_dir(path);
        }
    }
    std::fs::remove_file(path)
}

/// The type specific part of CreateFile() for non-directories.
#[cfg(windows)]
fn create_typed(attr: &mut Attributes, msg: &mut dyn FnMut(i32, String)) -> Cf {
    use crate::jmsg::M_ERROR;
    let oname = attr.ofname_str();
    match attr.type_ {
        FT_REGE | FT_REG => match std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(attr.ofname_path()) {
            Ok(f) => Cf::Extract(f),
            Err(e) => {
                msg(M_ERROR, format!("Could not create {oname}: ERR={}\n", crate::berrno::s(&e)));
                Cf::Error
            }
        },
        FT_LNKSAVED => {
            let from = std::ffi::OsStr::from_bytes(&attr.olname);
            if let Err(e) = std::fs::hard_link(from, attr.ofname_path()) {
                msg(M_ERROR, format!("Could not hard link {oname} -> {}: ERR={}\n", String::from_utf8_lossy(&attr.olname), crate::berrno::s(&e)));
                return Cf::Error;
            }
            Cf::Created
        }
        FT_LNK => {
            let target = std::path::Path::new(std::ffi::OsStr::from_bytes(&attr.olname));
            // a link to a directory is a different kind of link on Windows
            let resolved = match attr.ofname_path().parent() {
                Some(parent) if target.is_relative() => parent.join(target),
                _ => target.to_path_buf(),
            };
            let result = if resolved.is_dir() {
                std::os::windows::fs::symlink_dir(target, attr.ofname_path())
            } else {
                std::os::windows::fs::symlink_file(target, attr.ofname_path())
            };
            if let Err(e) = result {
                msg(M_ERROR, format!("Could not symlink {oname} -> {}: ERR={}\n", String::from_utf8_lossy(&attr.olname), crate::berrno::s(&e)));
                return Cf::Error;
            }
            Cf::Created
        }
        t => {
            msg(M_ERROR, format!("unhandled type: {t}\n"));
            Cf::Error
        }
    }
}

/// makepath() on Windows: `apath` and every missing parent; the directories
/// this restore created are remembered for replace=never.
#[cfg(windows)]
pub fn makepath(apath: &[u8], state: &mut CreateState, msg: &mut dyn FnMut(i32, String)) -> bool {
    use crate::jmsg::M_ERROR;
    let path = std::path::Path::new(std::ffi::OsStr::from_bytes(apath));
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => return true,
        Ok(_) => {
            msg(M_ERROR, format!("{} exists but is not a directory.\n", String::from_utf8_lossy(apath)));
            return false;
        }
        Err(_) => {}
    }
    // missing parents first
    let mut missing: Vec<&std::path::Path> = Vec::new();
    let mut cur = Some(path);
    while let Some(p) = cur {
        if p.as_os_str().is_empty() || p.is_dir() {
            break;
        }
        missing.push(p);
        cur = p.parent();
    }
    for p in missing.into_iter().rev() {
        if let Err(e) = std::fs::create_dir(p) {
            if !p.is_dir() {
                msg(M_ERROR, format!("Cannot create directory {}: ERR={}\n", p.display(), crate::berrno::s(&e)));
                return false;
            }
        } else if state.keep_path_list {
            state.path_list.insert(p.as_os_str().as_bytes().to_vec());
        }
    }
    true
}
