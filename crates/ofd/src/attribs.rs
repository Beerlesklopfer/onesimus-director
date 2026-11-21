//! Attribute records of a restore: lib/attr.cc (UnpackAttributesRecord,
//! BuildAttrOutputFnames, PrintLsOutput), lib/attribs.cc (DecodeStat),
//! lib/base64.cc (FromBase64) and findlib/attribs.cc (SetAttributes).

#[cfg(unix)]
use std::ffi::CString;
use std::ffi::OsStr;
use crate::platform::OsStrExt;
use std::path::Path;

use bareos_format::breg::Bregexp;

// include/filetypes.h (bareos-format, one definition for ofd, osd and odir)
pub use bareos_format::filetype::{
    AR_DATA_STREAM, FT_DELETED, FT_DIRBEGIN, FT_DIREND, FT_DIRNOCHG, FT_FIFO, FT_ISARCH, FT_JUNCTION, FT_LNK, FT_LNKSAVED,
    FT_MASK, FT_NOACCESS, FT_NOCHG, FT_NOFOLLOW, FT_NOFSCHG, FT_NOOPEN, FT_NORECURSE, FT_NOSTAT, FT_PLUGIN_CONFIG,
    FT_RAW, FT_REG, FT_REGE, FT_REPARSE, FT_RESTORE_FIRST, FT_SPEC, is_ft_object,
};
// lib/attribs.cc EncodeStat/DecodeStat, lib/base64.cc (bareos-format)
pub use bareos_format::base64::from_base64;
pub use bareos_format::lstat::{Stat, decode_stat, encode_stat};

/// One unpacked attribute record (struct Attributes).
#[derive(Debug, Clone, Default)]
pub struct Attributes {
    #[allow(dead_code)]
    pub stream: i32,
    #[allow(dead_code)]
    pub file_index: i32,
    pub type_: i32,
    pub data_stream: i32,
    pub fname: Vec<u8>,
    pub attr: Vec<u8>,
    pub lname: Vec<u8>,
    pub attr_ex: Vec<u8>,
    pub delta_seq: i32,
    pub statp: Stat,
    pub link_fi: i32,
    /// Output file name (where/regexwhere applied).
    pub ofname: Vec<u8>,
    /// Output link name.
    pub olname: Vec<u8>,
}

impl Attributes {
    pub fn ofname_path(&self) -> &Path {
        Path::new(OsStr::from_bytes(&self.ofname))
    }
    pub fn ofname_str(&self) -> String {
        String::from_utf8_lossy(&self.ofname).into_owned()
    }
    pub fn fname_str(&self) -> String {
        String::from_utf8_lossy(&self.fname).into_owned()
    }
}

/// UnpackAttributesRecord(): Err(message) for "Error scanning attributes".
pub fn unpack_attributes_record(stream: i32, rec: &[u8]) -> Result<Attributes, String> {
    let bad = || format!("Error scanning attributes: {}\n", String::from_utf8_lossy(rec.split(|&b| b == 0).next().unwrap_or(&[])));
    let v = crate::scan::scan(rec, "%d %d", 2).ok_or_else(bad)?;
    let file_index = v[0].int() as i32;
    let mut type_ = v[1].int() as i32;
    // skip record file index and type (up to and including their blanks)
    let mut fields = rec.splitn(3, |&b| b == b' ');
    fields.next();
    fields.next();
    let rest = fields.next().unwrap_or(&[]);
    let mut a = Attributes { stream, file_index, ..Default::default() };
    a.data_stream = i32::from(type_ & AR_DATA_STREAM != 0);
    type_ &= FT_MASK;
    a.type_ = type_;
    let mut parts = rest.splitn(4, |&b| b == 0);
    a.fname = parts.next().unwrap_or(&[]).to_vec();
    a.attr = parts.next().unwrap_or(&[]).to_vec();
    a.lname = parts.next().unwrap_or(&[]).to_vec();
    let tail = parts.next().unwrap_or(&[]);
    if type_ == FT_RESTORE_FIRST {
        a.attr_ex = tail.to_vec();
    } else {
        let mut t = tail.splitn(2, |&b| b == 0);
        a.attr_ex = t.next().unwrap_or(&[]).to_vec();
        let after = t.next().unwrap_or(&[]);
        if a.data_stream != 0 {
            a.data_stream = from_base64(after).0 as i32;
        } else if !after.is_empty() {
            let s: Vec<u8> = after.iter().take_while(|&&b| b != 0).copied().collect();
            a.delta_seq = String::from_utf8_lossy(&s).trim().parse().unwrap_or(0);
        }
    }
    Ok(a)
}

/// Where the restored files go (jcr->where / jcr->where_bregexp).
pub enum Where {
    Prefix(Vec<u8>),
    Regex(Vec<Bregexp>),
}

/// BuildAttrOutputFnames().
pub fn build_attr_output_fnames(attr: &mut Attributes, where_: &Where, prefix_links: bool) {
    let is_link = attr.type_ == FT_LNKSAVED || attr.type_ == FT_LNK;
    match where_ {
        Where::Regex(list) => {
            attr.ofname = bareos_format::breg::apply_bregexps(&attr.fname, list);
            if is_link {
                attr.olname = if attr.type_ == FT_LNKSAVED || prefix_links {
                    bareos_format::breg::apply_bregexps(&attr.lname, list)
                } else {
                    attr.lname.clone()
                };
            }
        }
        Where::Prefix(w) if w.is_empty() => {
            attr.ofname = attr.fname.clone();
            attr.olname = attr.lname.clone();
        }
        Where::Prefix(w) => {
            // Windows: `C:/data/a` below a prefix is `<prefix>/C/data/a`, the
            // colon of the drive letter cannot be part of a path component
            let fname = restore_path(&attr.fname);
            let lname = restore_path(&attr.lname);
            let mut of = w.clone();
            if w.last() != Some(&b'/') && fname.first() != Some(&b'/') {
                of.push(b'/');
            }
            of.extend_from_slice(&fname);
            attr.ofname = of;
            if is_link {
                let absolute = lname.first() == Some(&b'/') || (cfg!(windows) && has_drive(&attr.lname));
                let allow_prefix = attr.type_ == FT_LNKSAVED || prefix_links;
                let add_link = absolute && allow_prefix;
                let mut ol = if add_link { w.clone() } else { Vec::new() };
                if add_link && w.last() != Some(&b'/') && lname.first() != Some(&b'/') {
                    ol.push(b'/');
                }
                ol.extend_from_slice(if add_link { &lname } else { &attr.lname });
                attr.olname = ol;
            }
        }
    }
}

/// Whether `name` starts with a Windows drive (`C:/…`, `C:\\…`, `C:`).
pub fn has_drive(name: &[u8]) -> bool {
    name.len() >= 2 && name[0].is_ascii_alphabetic() && name[1] == b':' && name.get(2).is_none_or(|c| matches!(c, b'/' | b'\\'))
}

/// The name as it is appended to a restore prefix: on Windows the drive
/// `C:/x` becomes `C/x`; on Unix and for names without a drive unchanged.
pub fn restore_path(name: &[u8]) -> Vec<u8> {
    if cfg!(windows) && has_drive(name) {
        let mut out = vec![name[0]];
        out.extend_from_slice(&name[2..]);
        out
    } else {
        name.to_vec()
    }
}

/// encode_mode() of lib/edit.cc ("drwxr-xr-x").
fn encode_mode(mode: u32) -> String {
    let fmt = mode & crate::stmode::S_IFMT;
    let mut s = String::with_capacity(10);
    s.push(match fmt {
        crate::stmode::S_IFDIR => 'd',
        crate::stmode::S_IFCHR => 'c',
        crate::stmode::S_IFBLK => 'b',
        crate::stmode::S_IFIFO => 'f',
        crate::stmode::S_IFLNK => 'l',
        crate::stmode::S_IFSOCK => 's',
        _ => '-',
    });
    let bit = |b: u32, c: char| if mode & b != 0 { c } else { '-' };
    s.push(bit(crate::stmode::S_IRUSR, 'r'));
    s.push(bit(crate::stmode::S_IWUSR, 'w'));
    s.push(if mode & crate::stmode::S_ISUID != 0 { if mode & crate::stmode::S_IXUSR != 0 { 's' } else { 'S' } } else { bit(crate::stmode::S_IXUSR, 'x') });
    s.push(bit(crate::stmode::S_IRGRP, 'r'));
    s.push(bit(crate::stmode::S_IWGRP, 'w'));
    s.push(if mode & crate::stmode::S_ISGID != 0 { if mode & crate::stmode::S_IXGRP != 0 { 's' } else { 'S' } } else { bit(crate::stmode::S_IXGRP, 'x') });
    s.push(bit(crate::stmode::S_IROTH, 'r'));
    s.push(bit(crate::stmode::S_IWOTH, 'w'));
    s.push(if mode & crate::stmode::S_ISVTX != 0 { if mode & crate::stmode::S_IXOTH != 0 { 't' } else { 'T' } } else { bit(crate::stmode::S_IXOTH, 'x') });
    s
}

#[cfg(unix)]
fn uid_name(uid: u32) -> String {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).ok().flatten().map(|u| u.name).unwrap_or_else(|| uid.to_string())
}

#[cfg(unix)]
fn gid_name(gid: u32) -> String {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid)).ok().flatten().map(|g| g.name).unwrap_or_else(|| gid.to_string())
}

/// Windows: no user database for POSIX uids; the number (as Bareos for an
/// unknown uid).
#[cfg(windows)]
fn uid_name(uid: u32) -> String {
    uid.to_string()
}

/// Windows: the number, see `uid_name`.
#[cfg(windows)]
fn gid_name(gid: u32) -> String {
    gid.to_string()
}

/// attr_to_ls_output(): the M_RESTORED line of PrintLsOutput().
pub fn ls_output(attr: &Attributes) -> String {
    let mut out = String::new();
    if attr.type_ == FT_DELETED {
        out.push_str("----------   - -        -                - ---------- --------");
    } else {
        let st = &attr.statp;
        let t = chrono::DateTime::from_timestamp(st.ctime, 0)
            .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();
        let trunc8 = |s: String| s.chars().take(8).collect::<String>();
        out.push_str(&format!("{}  {:2} {:<8} {:<8}{:>12} {}", encode_mode(st.mode), st.nlink as u32,
            trunc8(uid_name(st.uid)), trunc8(gid_name(st.gid)), st.size, t));
    }
    out.push_str("  ");
    out.push_str(&attr.ofname_str());
    if attr.type_ == FT_LNK {
        out.push_str(" -> ");
        out.push_str(&String::from_utf8_lossy(&attr.olname));
    }
    out.push('\n');
    out
}

#[cfg(unix)]
fn cpath(p: &[u8]) -> CString {
    CString::new(p.to_vec()).unwrap_or_default()
}

/// SetAttributes(): owner, mode and times of a restored file (the open
/// output file is closed). Returns the error messages (M_ERROR).
#[cfg(unix)]
pub fn set_attributes(attr: &Attributes, ofd: Option<std::fs::File>) -> Vec<String> {
    let mut errors = Vec::new();
    // suppress_errors = (debug_level >= 100 || my_uid != 0)
    let suppress = !nix::unistd::getuid().is_root();
    let name = cpath(&attr.ofname);
    let oname = attr.ofname_str();
    let errno = crate::berrno::BErr::last;
    let old_mask = unsafe { libc::umask(0) };

    let size = match &ofd {
        Some(f) => f.metadata().map(|m| m.len() as i64).ok(),
        None => std::fs::symlink_metadata(attr.ofname_path()).map(|m| m.len() as i64).ok(),
    };
    if let Some(fsize) = size {
        if attr.type_ == FT_REG && fsize > 0 && attr.statp.size > 0 && fsize != attr.statp.size {
            errors.push(format!("File size of restored file {oname} not correct. Original {}, restored {fsize}.\n", attr.statp.size));
        }
    }

    let is_sock = attr.type_ == FT_SPEC && attr.statp.mode & crate::stmode::S_IFMT == crate::stmode::S_IFSOCK;
    if is_sock {
        // We do not restore sockets.
    } else if attr.type_ == FT_LNK {
        if unsafe { libc::lchown(name.as_ptr(), attr.statp.uid, attr.statp.gid) } < 0 && !suppress {
            errors.push(format!("Unable to set file owner {oname}: ERR={}\n", errno()));
        }
        // No lchmod on Linux (HAVE_LCHMOD unset). Symlink times are not set
        // either (the original only uses futimes/utime on non-links).
        // macOS has lchmod (HAVE_LCHMOD): the full st_mode, like SetAttributes().
        #[cfg(target_vendor = "apple")]
        if unsafe { libc::lchmod(name.as_ptr(), crate::stmode::to_mode_t(attr.statp.mode)) } < 0 && !suppress {
            errors.push(format!("Unable to set file modes {oname}: ERR={}\n", errno()));
        }
    } else {
        use std::os::fd::AsRawFd;
        let fd = ofd.as_ref().map(|f| f.as_raw_fd());
        let r = match fd {
            Some(fd) => unsafe { libc::fchown(fd, attr.statp.uid, attr.statp.gid) },
            None => unsafe { libc::lchown(name.as_ptr(), attr.statp.uid, attr.statp.gid) },
        };
        if r < 0 && !suppress {
            errors.push(format!("Unable to set file owner {oname}: ERR={}\n", errno()));
        }
        let r = match fd {
            Some(fd) => unsafe { libc::fchmod(fd, crate::stmode::to_mode_t(attr.statp.mode & 0o7777)) },
            None => unsafe { libc::chmod(name.as_ptr(), crate::stmode::to_mode_t(attr.statp.mode & 0o7777)) },
        };
        if r < 0 && !suppress {
            errors.push(format!("Unable to set file modes {oname}: ERR={}\n", errno()));
        }
        let times = [
            libc::timeval { tv_sec: attr.statp.atime as libc::time_t, tv_usec: 0 },
            libc::timeval { tv_sec: attr.statp.mtime as libc::time_t, tv_usec: 0 },
        ];
        let r = match fd {
            Some(fd) => unsafe { libc::futimes(fd, times.as_ptr()) },
            None => unsafe { libc::utimes(name.as_ptr(), times.as_ptr()) },
        };
        if r < 0 && !suppress {
            errors.push(format!("Unable to set file times {oname}: ERR={}\n", errno()));
        }
        // HAVE_CHFLAGS (macOS): the BSD file flags last, after the times,
        // since an immutable flag would make utimes() fail.
        #[cfg(target_vendor = "apple")]
        if unsafe { libc::chflags(name.as_ptr(), attr.statp.flags as libc::c_uint) } < 0 && !suppress {
            errors.push(format!("Unable to set file flags {oname}: ERR={}\n", errno()));
        }
    }
    drop(ofd);
    unsafe { libc::umask(old_mask) };
    errors
}

/// SetAttributes() on Windows: the access and modification time of the
/// restored file or directory (`SetFileTime`) and the attributes of a Windows
/// FD's `attribsEx` (read-only, hidden, system, archive, creation time),
/// else the times of the catalog's `Stat`; links keep the time they were
/// created with. Owner and mode do not exist on Windows.
#[cfg(windows)]
pub fn set_attributes(attr: &Attributes, ofd: Option<std::fs::File>) -> Vec<String> {
    let mut errors = Vec::new();
    let oname = attr.ofname_str();
    let size = match &ofd {
        Some(f) => f.metadata().map(|m| m.len() as i64).ok(),
        None => std::fs::symlink_metadata(attr.ofname_path()).map(|m| m.len() as i64).ok(),
    };
    if let Some(fsize) = size {
        if attr.type_ == FT_REG && fsize > 0 && attr.statp.size > 0 && fsize != attr.statp.size {
            errors.push(format!("File size of restored file {oname} not correct. Original {}, restored {fsize}.\n", attr.statp.size));
        }
    }
    drop(ofd);
    if !matches!(attr.type_, FT_LNK | FT_LNKSAVED) {
        // the exact times and attributes of a Windows FD's backup, else the
        // times of the catalog's Stat (whole seconds)
        let result = match crate::winattr::decode(&attr.attr_ex) {
            Some(meta) => crate::winfs::apply_meta(&attr.ofname, &meta),
            None => crate::winfs::set_times(&attr.ofname, attr.statp.atime, attr.statp.mtime),
        };
        if let Err(code) = result {
            errors.push(format!("Unable to set file attributes of {oname}: ERR={}\n", crate::berrno::BErr(code)));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    // ToBase64/FromBase64 and DecodeStat are tested in bareos-format
    // (base64.rs, lstat.rs decode_stat_like_original) since 19.11.2026.

    #[test]
    fn unpack_record_and_where() {
        let rec = b"7 3 /etc/hosts\0P0A BAA\0\0\x000";
        let mut a = unpack_attributes_record(1, rec).unwrap();
        assert_eq!(a.file_index, 7);
        assert_eq!(a.type_, FT_REG);
        assert_eq!(a.fname, b"/etc/hosts");
        assert_eq!(a.attr, b"P0A BAA");
        assert!(a.lname.is_empty());
        build_attr_output_fnames(&mut a, &Where::Prefix(b"/tmp/r".to_vec()), false);
        assert_eq!(a.ofname, b"/tmp/r/etc/hosts");
        build_attr_output_fnames(&mut a, &Where::Prefix(Vec::new()), false);
        assert_eq!(a.ofname, b"/etc/hosts");

        let rec = b"8 4 /a/link\0P0A\0/a/target\0\0";
        let mut a = unpack_attributes_record(1, rec).unwrap();
        build_attr_output_fnames(&mut a, &Where::Prefix(b"/r/".to_vec()), false);
        assert_eq!((a.ofname.as_slice(), a.olname.as_slice()), (&b"/r//a/link"[..], &b"/a/target"[..]));
        build_attr_output_fnames(&mut a, &Where::Prefix(b"/r".to_vec()), true);
        assert_eq!(a.olname, b"/r/a/target");
        // hard links always get the prefix
        let rec = b"9 1 /a/hard\0P0A\0/a/orig\0\0";
        let mut a = unpack_attributes_record(1, rec).unwrap();
        build_attr_output_fnames(&mut a, &Where::Prefix(b"/r".to_vec()), false);
        assert_eq!(a.olname, b"/r/a/orig");
        // regexwhere
        let list = bareos_format::breg::get_bregexps("!/a!/b!").unwrap();
        build_attr_output_fnames(&mut a, &Where::Regex(list), false);
        assert_eq!((a.ofname.as_slice(), a.olname.as_slice()), (&b"/b/hard"[..], &b"/b/orig"[..]));
        // bsscanf: numbers without digits are 0 and still count
        assert_eq!(unpack_attributes_record(1, b"x y").unwrap().file_index, 0);
    }

    #[test]
    fn drive_letters() {
        assert!(has_drive(b"C:/Users/x") && has_drive(b"d:\\x") && has_drive(b"C:"));
        assert!(!has_drive(b"/C:/x") && !has_drive(b"CC:/x") && !has_drive(b"C:x") && !has_drive(b"/etc"));
        // on Unix names are never rewritten
        if !cfg!(windows) {
            assert_eq!(restore_path(b"C:/Users/x"), b"C:/Users/x");
        }
    }

    #[test]
    fn mode_string() {
        assert_eq!(encode_mode(0o040755), "drwxr-xr-x");
        assert_eq!(encode_mode(0o104755), "-rwsr-xr-x");
        assert_eq!(encode_mode(0o120777), "lrwxrwxrwx");
    }
}
