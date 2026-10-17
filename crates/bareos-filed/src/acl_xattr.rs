//! ACL and extended attribute restore on Linux: findlib/acl.cc
//! (generic_set_acl_on_os: acl_from_text + acl_set_file) and findlib/xattr.cc
//! (UnSerializeXattrStream + generic_parse_xattr_streams: lsetxattr).
//!
//! The ACL text (acl_to_text() of the backup) is turned into the kernel's
//! `system.posix_acl_access` / `system.posix_acl_default` xattr value
//! directly, which is what acl_set_file() writes on Linux; no libacl needed.

use std::ffi::CString;

use crate::streams::*;

/// Result of one ACL/xattr stream (bacl_exit_code / BxattrExitCode).
#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    Ok,
    /// Non-fatal error (ACL: reported as a warning up to
    /// REPORT_ERR_MAX_PER_JOB times; xattr: M_ERROR).
    Error(String),
    /// xattr BxattrExitCode::kWarning (M_WARNING, not counted).
    Warning(String),
    /// Fatal: the restore is aborted.
    #[allow(dead_code)]
    Fatal(String),
}

/// Per-job ACL/xattr state (AclData / XattrData).
#[derive(Default)]
pub struct AclXattrState {
    pub acl_errors: u32,
    pub xattr_errors: u32,
    /// The filesystem said ENOTSUP for xattrs: skip them (and warn once).
    pub xattr_disabled_warned: bool,
}

/// ACL_REPORT_ERR_MAX_PER_JOB / XATTR_REPORT_ERR_MAX_PER_JOB
pub const REPORT_ERR_MAX_PER_JOB: u32 = 25;

const ACL_USER_OBJ: u16 = 0x01;
const ACL_USER: u16 = 0x02;
const ACL_GROUP_OBJ: u16 = 0x04;
const ACL_GROUP: u16 = 0x08;
const ACL_MASK: u16 = 0x10;
const ACL_OTHER: u16 = 0x20;
const ACL_UNDEFINED_ID: u32 = u32::MAX;
const POSIX_ACL_XATTR_VERSION: u32 = 2;

/// acl_from_text() + acl_valid(): the `posix_acl_xattr` value.
pub fn acl_text_to_xattr(text: &str) -> Result<Vec<u8>, std::io::Error> {
    let einval = || std::io::Error::from_raw_os_error(libc::EINVAL);
    let mut entries: Vec<(u16, u16, u32)> = Vec::new();
    for raw in text.split(['\n', ',']) {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split(':').map(str::trim).collect();
        let (tag, qualifier, perms) = match parts.as_slice() {
            [t, q, p] => (*t, *q, *p),
            // "other:rwx" / "mask:rwx" (no qualifier field)
            [t, p] if matches!(*t, "other" | "o" | "mask" | "m") => (*t, "", *p),
            _ => return Err(einval()),
        };
        let mut perm = 0u16;
        for c in perms.chars() {
            match c {
                'r' => perm |= 4,
                'w' => perm |= 2,
                'x' => perm |= 1,
                '-' => {}
                _ => return Err(einval()),
            }
        }
        let entry = match tag {
            "user" | "u" if qualifier.is_empty() => (ACL_USER_OBJ, perm, ACL_UNDEFINED_ID),
            "user" | "u" => (ACL_USER, perm, user_id(qualifier).ok_or_else(einval)?),
            "group" | "g" if qualifier.is_empty() => (ACL_GROUP_OBJ, perm, ACL_UNDEFINED_ID),
            "group" | "g" => (ACL_GROUP, perm, group_id(qualifier).ok_or_else(einval)?),
            "mask" | "m" if qualifier.is_empty() => (ACL_MASK, perm, ACL_UNDEFINED_ID),
            "other" | "o" if qualifier.is_empty() => (ACL_OTHER, perm, ACL_UNDEFINED_ID),
            _ => return Err(einval()),
        };
        entries.push(entry);
    }
    // acl_valid(): exactly one USER_OBJ, GROUP_OBJ, OTHER; MASK required
    // when named entries exist; no duplicate qualifiers.
    let count = |t: u16| entries.iter().filter(|e| e.0 == t).count();
    let named = count(ACL_USER) + count(ACL_GROUP);
    if count(ACL_USER_OBJ) != 1 || count(ACL_GROUP_OBJ) != 1 || count(ACL_OTHER) != 1
        || count(ACL_MASK) > 1 || (named > 0 && count(ACL_MASK) != 1)
    {
        return Err(einval());
    }
    entries.sort_by_key(|e| (e.0, e.2));
    if entries.windows(2).any(|w| w[0].0 == w[1].0 && w[0].2 == w[1].2) {
        return Err(einval());
    }
    let mut out = POSIX_ACL_XATTR_VERSION.to_le_bytes().to_vec();
    for (tag, perm, id) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&perm.to_le_bytes());
        out.extend_from_slice(&id.to_le_bytes());
    }
    Ok(out)
}

fn user_id(q: &str) -> Option<u32> {
    if let Ok(n) = q.parse() {
        return Some(n);
    }
    nix::unistd::User::from_name(q).ok().flatten().map(|u| u.uid.as_raw())
}

fn group_id(q: &str) -> Option<u32> {
    if let Ok(n) = q.parse() {
        return Some(n);
    }
    nix::unistd::Group::from_name(q).ok().flatten().map(|g| g.gid.as_raw())
}

fn cpath(p: &[u8]) -> CString {
    CString::new(p.to_vec()).unwrap_or_default()
}

fn errno() -> crate::berrno::BErr {
    crate::berrno::BErr::last()
}

/// parse_acl_streams() for Linux. `content` is the ACL text with its
/// trailing NUL as sent by the backup.
pub fn restore_acl(fname: &[u8], filetype: i32, stream: i32, content: &[u8]) -> Exit {
    let name = String::from_utf8_lossy(fname);
    let default = match stream {
        STREAM_UNIX_ACCESS_ACL | STREAM_ACL_LINUX_ACCESS_ACL => false,
        STREAM_UNIX_DEFAULT_ACL | STREAM_ACL_LINUX_DEFAULT_ACL => true,
        _ => {
            return Exit::Error(format!(
                "Can't restore ACLs of {name} - incompatible acl stream encountered - {stream}\n"
            ));
        }
    };
    let text = String::from_utf8_lossy(content.split(|&b| b == 0).next().unwrap_or(&[])).into_owned();
    let path = cpath(fname);
    let attr = if default { c"system.posix_acl_default" } else { c"system.posix_acl_access" };
    if default && text.is_empty() {
        // acl_delete_def_file()
        if unsafe { libc::removexattr(path.as_ptr(), attr.as_ptr()) } == 0 {
            return Exit::Ok;
        }
        let e = errno();
        return match e.raw_os_error() {
            Some(libc::ENOENT) | Some(libc::ENODATA) => Exit::Ok,
            Some(libc::ENOTSUP) => Exit::Error(format!("acl_delete_def_file error on file \"{name}\": filesystem doesn't support ACLs\n")),
            _ => Exit::Error(format!("acl_delete_def_file error on file \"{name}\": ERR={e}\n")),
        };
    }
    let value = match acl_text_to_xattr(&text) {
        Ok(v) => v,
        Err(e) => return Exit::Error(format!("acl_from_text error on file \"{name}\": ERR={e}\n")),
    };
    if unsafe { libc::setxattr(path.as_ptr(), attr.as_ptr(), value.as_ptr().cast(), value.len(), 0) } != 0
        && filetype != crate::attribs::FT_LNK
    {
        let e = errno();
        return match e.raw_os_error() {
            Some(libc::ENOENT) => Exit::Ok,
            Some(libc::ENOTSUP) => Exit::Error(format!("acl_set_file error on file \"{name}\": filesystem doesn't support ACLs\n")),
            _ => Exit::Error(format!("acl_set_file error on file \"{name}\": ERR={e}\n")),
        };
    }
    Exit::Ok
}

/// XATTR_MAGIC (xattr.h:54)
const XATTR_MAGIC: u32 = 0x5C5884;

/// UnSerializeXattrStream(): (name, value) pairs.
pub fn unserialize_xattr(fname: &str, mut content: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>, String> {
    let mut list = Vec::new();
    let u32_at = |c: &[u8]| -> Option<u32> { c.get(..4).map(|b| u32::from_be_bytes(b.try_into().unwrap())) };
    while !content.is_empty() {
        if u32_at(content) != Some(XATTR_MAGIC) {
            return Err(format!("Illegal xattr stream, no XATTR_MAGIC on file \"{fname}\"\n"));
        }
        content = &content[4..];
        let name_len = u32_at(content).unwrap_or(0) as usize;
        if name_len == 0 {
            return Err(format!("Illegal xattr stream, xattr name length <= 0 on file \"{fname}\"\n"));
        }
        content = &content[4..];
        let Some(name) = content.get(..name_len) else { return Err(format!("Illegal xattr stream, no XATTR_MAGIC on file \"{fname}\"\n")) };
        let name = name.to_vec();
        content = &content[name_len..];
        let value_len = u32_at(content).unwrap_or(0) as usize;
        content = content.get(4..).unwrap_or(&[]);
        let Some(value) = content.get(..value_len) else { return Err(format!("Illegal xattr stream, no XATTR_MAGIC on file \"{fname}\"\n")) };
        list.push((name, value.to_vec()));
        content = &content[value_len..];
    }
    Ok(list)
}

/// generic_parse_xattr_streams(): lsetxattr() every attribute.
pub fn restore_xattr(fname: &[u8], stream: i32, content: &[u8], state: &mut AclXattrState) -> Exit {
    let name = String::from_utf8_lossy(fname).into_owned();
    if stream != STREAM_XATTR_LINUX {
        return Exit::Error(format!(
            "Can't restore Extended Attributes of {name} - incompatible xattr stream encountered - {stream}\n"
        ));
    }
    if state.xattr_disabled_warned {
        return Exit::Ok;
    }
    let list = match unserialize_xattr(&name, content) {
        Ok(l) => l,
        Err(m) => return Exit::Error(m),
    };
    let path = cpath(fname);
    for (n, v) in list {
        let cn = CString::new(n).unwrap_or_default();
        if unsafe { libc::lsetxattr(path.as_ptr(), cn.as_ptr(), v.as_ptr().cast(), v.len(), 0) } != 0 {
            let e = errno();
            return match e.raw_os_error() {
                Some(libc::ENOENT) => Exit::Error(String::new()),
                Some(libc::ENOTSUP) => {
                    // warn_if_disabling_xattrs()
                    state.xattr_disabled_warned = true;
                    Exit::Warning(format!(
                        "Disabling restore of XATTRs on this filesystem, not supported. Current file: \"{name}\"\n"
                    ))
                }
                _ => Exit::Error(format!("lsetxattr error on file \"{name}\": ERR={e}\n")),
            };
        }
    }
    Exit::Ok
}

// ---------------------------------------------------------------------------
// Backup side: generic_build_acl_streams() / generic_build_xattr_streams()
// ---------------------------------------------------------------------------

/// libacl functions (dlopen; the original links libacl).
struct LibAcl {
    get_file: unsafe extern "C" fn(*const libc::c_char, u32) -> *mut libc::c_void,
    to_text: unsafe extern "C" fn(*mut libc::c_void, *mut isize) -> *mut libc::c_char,
    free: unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int,
    get_entry: unsafe extern "C" fn(*mut libc::c_void, libc::c_int, *mut *mut libc::c_void) -> libc::c_int,
    get_tag_type: unsafe extern "C" fn(*mut libc::c_void, *mut libc::c_int) -> libc::c_int,
}

fn libacl() -> Option<&'static LibAcl> {
    static L: std::sync::OnceLock<Option<LibAcl>> = std::sync::OnceLock::new();
    L.get_or_init(|| unsafe {
        let h = [c"libacl.so.1", c"libacl.so"].iter().map(|n| libc::dlopen(n.as_ptr(), libc::RTLD_NOW)).find(|h| !h.is_null())?;
        let sym = |n: &std::ffi::CStr| {
            let s = libc::dlsym(h, n.as_ptr());
            (!s.is_null()).then_some(s as usize)
        };
        Some(LibAcl {
            get_file: std::mem::transmute::<usize, _>(sym(c"acl_get_file")?),
            to_text: std::mem::transmute::<usize, _>(sym(c"acl_to_text")?),
            free: std::mem::transmute::<usize, _>(sym(c"acl_free")?),
            get_entry: std::mem::transmute::<usize, _>(sym(c"acl_get_entry")?),
            get_tag_type: std::mem::transmute::<usize, _>(sym(c"acl_get_tag_type")?),
        })
    })
    .as_ref()
}

const ACL_TYPE_ACCESS: u32 = 0x8000;
const ACL_TYPE_DEFAULT: u32 = 0x4000;

/// generic_get_acl_from_os(): Ok(Some(text)) / Ok(None) (no or trivial
/// ACL) / Err(message).
fn get_acl_text(fname: &[u8], default: bool) -> Result<Option<String>, String> {
    let name = String::from_utf8_lossy(fname);
    let path = cpath(fname);
    let Some(l) = libacl() else { return get_acl_text_xattr(fname, default) };
    unsafe {
        let acl = (l.get_file)(path.as_ptr(), if default { ACL_TYPE_DEFAULT } else { ACL_TYPE_ACCESS });
        if acl.is_null() {
            let e = errno();
            return match e.raw_os_error() {
                Some(libc::ENOTSUP) | Some(libc::ENOENT) => Ok(None),
                _ => Err(format!("acl_get_file error on file \"{name}\": ERR={e}\n")),
            };
        }
        let mut count = 0;
        let mut trivial = true;
        let mut entry: *mut libc::c_void = std::ptr::null_mut();
        let mut avail = (l.get_entry)(acl, 0, &mut entry);
        while avail == 1 {
            count += 1;
            let mut tag = 0;
            if (l.get_tag_type)(entry, &mut tag) < 0 || !matches!(tag, 0x01 | 0x04 | 0x20) {
                trivial = false;
            }
            avail = (l.get_entry)(acl, 1, &mut entry);
        }
        let result = if count <= 0 || (!default && trivial) {
            Ok(None)
        } else {
            let text = (l.to_text)(acl, std::ptr::null_mut());
            if text.is_null() {
                Err(format!("acl_to_text error on file \"{name}\": ERR={}\n", errno()))
            } else {
                let s = std::ffi::CStr::from_ptr(text).to_string_lossy().into_owned();
                (l.free)(text.cast());
                Ok(Some(s))
            }
        };
        (l.free)(acl);
        result
    }
}

/// Without libacl: the ACL from its xattr, as acl_to_text() writes it.
fn get_acl_text_xattr(fname: &[u8], default: bool) -> Result<Option<String>, String> {
    let path = cpath(fname);
    let attr = if default { c"system.posix_acl_default" } else { c"system.posix_acl_access" };
    let mut buf = vec![0u8; 4096];
    let n = unsafe { libc::getxattr(path.as_ptr(), attr.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        return Ok(None);
    }
    buf.truncate(n as usize);
    if buf.len() < 4 {
        return Ok(None);
    }
    let entries: Vec<(u16, u16, u32)> = buf[4..]
        .chunks_exact(8)
        .map(|c| (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]), u32::from_le_bytes([c[4], c[5], c[6], c[7]])))
        .collect();
    if entries.is_empty() || (!default && entries.iter().all(|e| matches!(e.0, 0x01 | 0x04 | 0x20))) {
        return Ok(None);
    }
    let mask = entries.iter().find(|e| e.0 == ACL_MASK).map(|e| e.1);
    let perm = |p: u16| {
        format!("{}{}{}", if p & 4 != 0 { 'r' } else { '-' }, if p & 2 != 0 { 'w' } else { '-' }, if p & 1 != 0 { 'x' } else { '-' })
    };
    let mut out = String::new();
    for (tag, p, id) in entries {
        let (t, q) = match tag {
            ACL_USER_OBJ => ("user", String::new()),
            ACL_USER => ("user", nix::unistd::User::from_uid(id.into()).ok().flatten().map(|u| u.name).unwrap_or(id.to_string())),
            ACL_GROUP_OBJ => ("group", String::new()),
            ACL_GROUP => ("group", nix::unistd::Group::from_gid(id.into()).ok().flatten().map(|g| g.name).unwrap_or(id.to_string())),
            ACL_MASK => ("mask", String::new()),
            _ => ("other", String::new()),
        };
        out.push_str(&format!("{t}:{q}:{}", perm(p)));
        if let Some(m) = mask {
            if matches!(tag, ACL_USER | ACL_GROUP_OBJ | ACL_GROUP) && p & !m & 7 != 0 {
                out.push_str(&format!("\t#effective:{}", perm(p & m)));
            }
        }
        out.push('\n');
    }
    Ok(Some(out))
}

/// The ACL streams of one file (STREAM_ACL_LINUX_ACCESS_ACL, and for
/// directories STREAM_ACL_LINUX_DEFAULT_ACL), each with its trailing NUL;
/// errors are bacl_exit_error messages.
pub fn build_acl_streams(fname: &[u8], is_dir: bool) -> (Vec<(i32, Vec<u8>)>, Vec<String>) {
    let mut streams = Vec::new();
    let mut errors = Vec::new();
    for (default, stream) in [(false, STREAM_ACL_LINUX_ACCESS_ACL), (true, STREAM_ACL_LINUX_DEFAULT_ACL)] {
        if default && !is_dir {
            break;
        }
        match get_acl_text(fname, default) {
            Ok(Some(t)) if !t.is_empty() => {
                let mut v = t.into_bytes();
                v.push(0);
                streams.push((stream, v));
            }
            Ok(_) => {}
            Err(e) => errors.push(e),
        }
    }
    (streams, errors)
}

/// generic_build_xattr_streams(): Ok(Some(serialized STREAM_XATTR_LINUX))
/// / Ok(None) / Err((is_info, message)).
pub fn build_xattr_stream(fname: &[u8], skip_acl_names: bool) -> Result<Option<Vec<u8>>, (bool, String)> {
    const ACL_SKIP: [&[u8]; 2] = [b"system.posix_acl_access", b"system.posix_acl_default"];
    const SKIP: [&[u8]; 8] = [b"ceph.dir.entries", b"ceph.dir.files", b"ceph.dir.rbytes", b"ceph.dir.rctime",
        b"ceph.dir.rentries", b"ceph.dir.rfiles", b"ceph.dir.rsubdirs", b"ceph.dir.subdirs"];
    let name = String::from_utf8_lossy(fname).into_owned();
    let path = cpath(fname);
    let len = unsafe { libc::llistxattr(path.as_ptr(), std::ptr::null_mut(), 0) };
    if len < 0 {
        let e = errno();
        return match e.raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            Some(libc::ENOTSUP) => Err((true, format!(
                "Disabling restore of XATTRs on this filesystem, not supported. Current file: \"{name}\"\n"))),
            _ => Err((false, format!("llistxattr error on file \"{name}\": ERR={e}\n"))),
        };
    }
    if len == 0 {
        return Ok(None);
    }
    let mut list = vec![0u8; len as usize + 1];
    let len = unsafe { libc::llistxattr(path.as_ptr(), list.as_mut_ptr().cast(), len as usize) };
    if len < 0 {
        let e = errno();
        return match e.raw_os_error() {
            Some(libc::ENOENT) => Ok(None),
            _ => Err((false, format!("llistxattr error on file \"{name}\": ERR={e}\n"))),
        };
    }
    list.truncate(len as usize);
    let mut out = Vec::new();
    let mut count = 0;
    for xname in list.split(|&b| b == 0) {
        if xname.is_empty() || (skip_acl_names && ACL_SKIP.contains(&xname)) || SKIP.contains(&xname) {
            continue;
        }
        let cn = CString::new(xname.to_vec()).unwrap_or_default();
        let vlen = unsafe { libc::lgetxattr(path.as_ptr(), cn.as_ptr(), std::ptr::null_mut(), 0) };
        if vlen < 0 {
            let e = errno();
            return match e.raw_os_error() {
                Some(libc::ENOENT) => Ok(None),
                _ => Err((false, format!("lgetxattr error on file \"{name}\": ERR={e}\n"))),
            };
        }
        let mut value = vec![0u8; vlen as usize];
        if vlen > 0 {
            let r = unsafe { libc::lgetxattr(path.as_ptr(), cn.as_ptr(), value.as_mut_ptr().cast(), value.len()) };
            if r < 0 {
                let e = errno();
                return match e.raw_os_error() {
                    Some(libc::ENOENT) => Ok(None),
                    _ => Err((false, format!("lgetxattr error on file \"{name}\": ERR={e}\n"))),
                };
            }
            value.truncate(r as usize);
        }
        out.extend_from_slice(&XATTR_MAGIC.to_be_bytes());
        out.extend_from_slice(&(xname.len() as u32).to_be_bytes());
        out.extend_from_slice(xname);
        out.extend_from_slice(&(value.len() as u32).to_be_bytes());
        out.extend_from_slice(&value);
        count += 1;
    }
    Ok((count > 0).then_some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acl_text_conversion() {
        let v = acl_text_to_xattr("user::rw-\nuser:0:r--\ngroup::r--\nmask::r--\nother::---\n").unwrap();
        assert_eq!(&v[..4], &2u32.to_le_bytes());
        assert_eq!(v.len(), 4 + 5 * 8);
        // sorted: USER_OBJ, USER(0), GROUP_OBJ, MASK, OTHER
        let tags: Vec<u16> = v[4..].chunks(8).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(tags, vec![1, 2, 4, 0x10, 0x20]);
        // named entry without mask is invalid
        assert!(acl_text_to_xattr("user::rw-\nuser:0:r--\ngroup::r--\nother::---\n").is_err());
        assert!(acl_text_to_xattr("bogus").is_err());
    }

    #[test]
    fn xattr_unserialize() {
        let mut s = Vec::new();
        for (n, v) in [(&b"user.a"[..], &b"1"[..]), (b"user.empty", b"")] {
            s.extend_from_slice(&XATTR_MAGIC.to_be_bytes());
            s.extend_from_slice(&(n.len() as u32).to_be_bytes());
            s.extend_from_slice(n);
            s.extend_from_slice(&(v.len() as u32).to_be_bytes());
            s.extend_from_slice(v);
        }
        let l = unserialize_xattr("f", &s).unwrap();
        assert_eq!(l, vec![(b"user.a".to_vec(), b"1".to_vec()), (b"user.empty".to_vec(), Vec::new())]);
        assert!(unserialize_xattr("f", &[0, 0, 0, 1]).is_err());
    }
}
