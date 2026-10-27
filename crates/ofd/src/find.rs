//! Walking the FileSet: findlib/find.cc FindFiles()/OurCallback() and
//! findlib/find_one.cc FindOneFile() (hard links, directories with the
//! DIRBEGIN/DIREND pair, special files, incremental change check, file
//! system crossing, IgnoreDir, size matching, keep atime).

use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

use crate::attribs::*;
use crate::fileset::*;

/// FT_NOCHG etc. of include/filetypes.h not already in attribs.
pub const FT_INVALIDFS: i32 = 19;
pub const FT_INVALIDDT: i32 = 20;
pub const FT_BASE: i32 = 24;

/// The per-file packet (FindFilesPacket).
#[derive(Clone, Default)]
pub struct FfPkt {
    pub fname: Vec<u8>,
    /// link_or_dir: link target, or the directory name with a trailing slash
    pub link: Vec<u8>,
    pub top_fname: Vec<u8>,
    pub statp: Stat,
    pub type_: i32,
    pub ff_errno: i32,
    pub link_fi: i32,
    pub file_index: i32,
    pub opts: FileOpts,
    pub delta_seq: i32,
    /// FT_LNKSAVED: the digest of the saved file (stream, bytes).
    pub digest: Option<(i32, Vec<u8>)>,
    /// Hard link key of a file whose data is saved now (ff_pkt->linked).
    pub linked: Option<(u64, u64)>,
    pub accurate_found: bool,
    // Values of the include block (FindFiles())
    pub verify_opts: String,
    pub accurate_opts: String,
    pub basejob_opts: String,
    pub strip_path: i32,
    pub size_match: Option<SizeMatch>,
    pub fstypes: Vec<String>,
    /// incremental / save_time (SetFindOptions)
    pub incremental: bool,
    pub save_time: i64,
}

/// CurLink of the hard link table.
pub struct CurLink {
    pub name: Vec<u8>,
    pub file_index: i32,
    pub digest: Option<(i32, Vec<u8>)>,
}

/// What FindFiles() calls back (SaveFile, TallyFile, VerifyFile) and the
/// accurate check (SetFindChangedFunction).
pub trait Handler {
    /// FileSave(): 1 ok, 0 error (stop), -1 ignore.
    fn handle(&mut self, ff: &mut FfPkt, links: &mut HashMap<(u64, u64), CurLink>, top_level: bool) -> i32;
    /// ff->CheckFct: Some(changed) when the handler decides (accurate).
    fn check_changes(&mut self, _ff: &mut FfPkt) -> Option<bool> {
        None
    }
    fn canceled(&self) -> bool {
        false
    }
    /// Jmsg of the find library.
    fn jmsg(&mut self, t: i32, text: String);
}

pub fn stat_from(st: &libc::stat) -> Stat {
    Stat {
        dev: st.st_dev,
        ino: st.st_ino,
        mode: st.st_mode,
        nlink: u64::from(st.st_nlink),
        uid: st.st_uid,
        gid: st.st_gid,
        rdev: st.st_rdev,
        size: st.st_size,
        blksize: i64::from(st.st_blksize),
        blocks: st.st_blocks,
        atime: st.st_atime,
        mtime: st.st_mtime,
        ctime: st.st_ctime,
        flags: 0,
    }
}

fn cpath(p: &[u8]) -> CString {
    CString::new(p.to_vec()).unwrap_or_default()
}

pub fn lstat(p: &[u8]) -> Result<libc::stat, i32> {
    let c = cpath(p);
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::lstat(c.as_ptr(), &mut st) } == 0 {
        Ok(st)
    } else {
        Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0))
    }
}

/// fstype() via the mount table (mntent_cache: the mount whose device is
/// st_dev).
pub fn fstype(fname: &[u8]) -> Option<String> {
    let st = lstat(fname).ok()?;
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let mut found = None;
    for line in mounts.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        if f.len() < 3 {
            continue;
        }
        let mp = f[1].replace("\\040", " ");
        if let Ok(m) = lstat(mp.as_bytes()) {
            if m.st_dev == st.st_dev {
                found = Some(f[2].to_string());
            }
        }
    }
    found
}

/// RestoreFileTimes() (lutimes)
fn restore_file_times(ff: &FfPkt, fname: &[u8]) {
    let c = cpath(fname);
    let times = [
        libc::timeval { tv_sec: ff.statp.atime as libc::time_t, tv_usec: 0 },
        libc::timeval { tv_sec: ff.statp.mtime as libc::time_t, tv_usec: 0 },
    ];
    unsafe { libc::lutimes(c.as_ptr(), times.as_ptr()) };
}

/// The FileSet walker.
pub struct Finder<'a> {
    pub fileset: &'a FileSet,
    incexe: usize,
    pub links: HashMap<(u64, u64), CurLink>,
}

impl<'a> Finder<'a> {
    pub fn new(fileset: &'a FileSet) -> Self {
        Self { fileset, incexe: 0, links: HashMap::new() }
    }

    /// FindFiles(): 1 ok, 0 error/canceled.
    pub fn find_files(&mut self, ff: &mut FfPkt, h: &mut dyn Handler) -> i32 {
        ff.opts.flags = Flags::default();
        for i in 0..self.fileset.include.len() {
            self.incexe = i;
            let incexe = &self.fileset.include[i];
            ff.verify_opts = "V".to_string();
            ff.accurate_opts = "Cmcs".to_string();
            ff.basejob_opts = "Jspug5".to_string();
            for fo in &incexe.opts {
                ff.opts.flags = fo.flags;
                ff.opts.compress_algo = fo.compress_algo;
                ff.opts.compress_level = fo.compress_level;
                ff.strip_path = fo.strip_path;
                ff.size_match = fo.size_match;
                ff.fstypes = fo.fstype.clone();
                ff.verify_opts.push_str(&fo.verify_opts);
                if !fo.accurate_opts.is_empty() {
                    ff.accurate_opts = fo.accurate_opts.clone();
                }
            }
            for name in &incexe.names {
                ff.top_fname = name.clone();
                if self.find_one_file(ff, h, name.clone(), u64::MAX, true) == 0 {
                    return 0;
                }
                if h.canceled() {
                    return 0;
                }
            }
            if !incexe.plugins.is_empty() {
                h.jmsg(crate::jmsg::M_FATAL, format!("Plugin: \"{}\" not found.\n", incexe.plugins[0]));
                return 0;
            }
        }
        1
    }

    /// OurCallback()
    fn callback(&mut self, ff: &mut FfPkt, h: &mut dyn Handler, top_level: bool) -> i32 {
        if top_level {
            return h.handle(ff, &mut self.links, top_level);
        }
        match ff.type_ {
            FT_NOACCESS | FT_NOFOLLOW | FT_NOSTAT | FT_NOCHG | FT_ISARCH | FT_NORECURSE | FT_NOFSCHG
            | FT_INVALIDFS | FT_INVALIDDT | FT_NOOPEN | FT_LNKSAVED | FT_REGE | FT_REG | FT_LNK | FT_DIRBEGIN
            | FT_DIREND | FT_RAW | FT_FIFO | FT_SPEC | FT_DIRNOCHG | FT_REPARSE | FT_JUNCTION => {
                let is_dir = ff.statp.mode & libc::S_IFMT == libc::S_IFDIR;
                let incexe = &self.fileset.include[self.incexe];
                let fname = ff.fname.clone();
                if accept_file(self.fileset, incexe, &fname, is_dir, &mut ff.opts) {
                    h.handle(ff, &mut self.links, top_level)
                } else {
                    -1
                }
            }
            _ => 0,
        }
    }

    /// CheckChanges()
    fn check_changes(ff: &mut FfPkt, h: &mut dyn Handler) -> bool {
        if let Some(changed) = h.check_changes(ff) {
            return changed;
        }
        !(ff.incremental
            && ff.statp.mtime < ff.save_time
            && (ff.opts.flags.has(FO_MTIMEONLY) || ff.statp.ctime < ff.save_time))
    }

    fn accept_fstype(ff: &FfPkt) -> bool {
        if ff.fstypes.is_empty() {
            return true;
        }
        match fstype(&ff.fname) {
            Some(fs) => ff.fstypes.iter().any(|t| *t == fs),
            None => false,
        }
    }

    /// FindOneFile()
    fn find_one_file(&mut self, ff: &mut FfPkt, h: &mut dyn Handler, fname: Vec<u8>, parent_dev: u64, top_level: bool) -> i32 {
        ff.fname = fname.clone();
        ff.link = fname.clone();
        ff.type_ = 0xFFFF;
        match lstat(&fname) {
            Ok(st) => ff.statp = stat_from(&st),
            Err(e) => {
                ff.type_ = FT_NOSTAT;
                ff.ff_errno = e;
                return self.callback(ff, h, top_level);
            }
        }
        if top_level && !Self::accept_fstype(ff) {
            ff.type_ = FT_INVALIDFS;
            if ff.opts.flags.has(FO_KEEPATIME) {
                restore_file_times(ff, &fname);
            }
            let (t, fs) = match fstype(&ff.fname) {
                Some(fs) => (crate::jmsg::M_INFO, fs),
                None => (crate::jmsg::M_WARNING, "unknown".to_string()),
            };
            h.jmsg(t, format!("Top level directory \"{}\" has unlisted fstype \"{fs}\"\n", String::from_utf8_lossy(&fname)));
            return 1;
        }
        let fmt = ff.statp.mode & libc::S_IFMT;
        if fmt != libc::S_IFDIR {
            if fmt == libc::S_IFREG {
                if let Some(sm) = ff.size_match {
                    if !sm.accepts(ff.statp.size) {
                        return 1;
                    }
                }
            }
            if !Self::check_changes(ff, h) {
                ff.type_ = FT_NOCHG;
                return self.callback(ff, h, top_level);
            }
        }
        ff.link_fi = 0;
        ff.file_index = 0;
        ff.linked = None;
        ff.digest = None;
        if !ff.opts.flags.has(FO_NO_HARDLINK)
            && ff.statp.nlink > 1
            && matches!(fmt, libc::S_IFREG | libc::S_IFCHR | libc::S_IFBLK | libc::S_IFIFO | libc::S_IFSOCK)
        {
            // process_hardlink()
            let key = (ff.statp.dev, ff.statp.ino);
            let entry = self.links.entry(key).or_insert_with(|| CurLink { name: fname.clone(), file_index: 0, digest: None });
            if entry.file_index == 0 {
                ff.linked = Some(key);
            } else if entry.name == fname {
                return 1;
            } else {
                ff.link = entry.name.clone();
                ff.type_ = FT_LNKSAVED;
                ff.link_fi = entry.file_index;
                ff.linked = None;
                ff.digest = entry.digest.clone();
                return self.callback(ff, h, top_level);
            }
        }
        let r = match fmt {
            libc::S_IFREG => {
                // MODE_RALL: world readable empty files are not opened
                ff.type_ = if ff.statp.size == 0 && (ff.statp.mode & 0o444) == 0o444 { FT_REGE } else { FT_REG };
                let r = self.callback(ff, h, top_level);
                self.set_linked(ff);
                if ff.opts.flags.has(FO_KEEPATIME) {
                    restore_file_times(ff, &fname);
                }
                r
            }
            libc::S_IFLNK => {
                match std::fs::read_link(std::ffi::OsStr::from_bytes(&fname)) {
                    Ok(t) => {
                        ff.link = t.as_os_str().as_bytes().to_vec();
                        ff.type_ = FT_LNK;
                    }
                    Err(e) => {
                        ff.type_ = FT_NOFOLLOW;
                        ff.ff_errno = e.raw_os_error().unwrap_or(0);
                    }
                }
                let r = self.callback(ff, h, top_level);
                self.set_linked(ff);
                r
            }
            libc::S_IFDIR => self.process_directory(ff, h, fname, parent_dev, top_level),
            _ => {
                ff.type_ = if top_level && fmt == libc::S_IFBLK {
                    FT_RAW
                } else if top_level && fmt == libc::S_IFIFO && ff.opts.flags.has(FO_READFIFO) {
                    FT_FIFO
                } else {
                    FT_SPEC
                };
                let r = self.callback(ff, h, top_level);
                self.set_linked(ff);
                r
            }
        };
        r
    }

    /// `if (ff_pkt->linked) ff_pkt->linked->FileIndex = ff_pkt->FileIndex;`
    fn set_linked(&mut self, ff: &FfPkt) {
        if let Some(key) = ff.linked {
            if let Some(l) = self.links.get_mut(&key) {
                l.file_index = ff.file_index;
            }
        }
    }

    /// HaveIgnoredir()
    fn have_ignoredir(&self, fname: &[u8]) -> bool {
        self.fileset.include[self.incexe].ignoredir.iter().any(|i| {
            let mut p = fname.to_vec();
            p.push(b'/');
            p.extend_from_slice(i.as_bytes());
            std::fs::metadata(std::ffi::OsStr::from_bytes(&p)).is_ok()
        })
    }

    /// process_directory()
    fn process_directory(&mut self, ff: &mut FfPkt, h: &mut dyn Handler, fname: Vec<u8>, parent_dev: u64, top_level: bool) -> i32 {
        let our_dev = ff.statp.dev;
        if self.have_ignoredir(&fname) {
            return 1;
        }
        let mut link = fname.clone();
        while link.last() == Some(&b'/') {
            link.pop();
        }
        link.push(b'/');
        ff.type_ = FT_DIREND;
        ff.link = link.clone();
        ff.type_ = if !Self::check_changes(ff, h) { FT_DIRNOCHG } else { FT_DIRBEGIN };
        let rtn = self.callback(ff, h, top_level);
        if rtn < 1 || ff.type_ == FT_REPARSE {
            return rtn;
        }
        if ff.type_ == FT_DIRBEGIN {
            ff.type_ = FT_DIREND;
        }
        let mut dir_ff = ff.clone();
        let mut recurse = true;
        if !top_level && ff.opts.flags.has(FO_NO_RECURSION) {
            ff.type_ = FT_NORECURSE;
            recurse = false;
        } else if !top_level && parent_dev != ff.statp.dev {
            if !ff.opts.flags.has(FO_MULTIFS) {
                ff.type_ = FT_NOFSCHG;
                recurse = false;
            } else if !Self::accept_fstype(ff) {
                ff.type_ = FT_INVALIDFS;
                recurse = false;
            }
        }
        if !recurse {
            let r = self.callback(ff, h, top_level);
            self.set_linked(ff);
            ff.link = ff.fname.clone();
            if ff.opts.flags.has(FO_KEEPATIME) {
                restore_file_times(ff, &fname);
            }
            return r;
        }
        ff.link = ff.fname.clone();
        let dir = match std::fs::read_dir(std::ffi::OsStr::from_bytes(&fname)) {
            Ok(d) => d,
            Err(e) => {
                ff.type_ = FT_NOOPEN;
                ff.ff_errno = e.raw_os_error().unwrap_or(0);
                let r = self.callback(ff, h, top_level);
                self.set_linked(ff);
                return r;
            }
        };
        let mut rtn = 1;
        for entry in dir {
            if h.canceled() {
                break;
            }
            let Ok(entry) = entry else { break };
            let name = entry.file_name();
            let mut child = link.clone();
            child.extend_from_slice(name.as_bytes());
            if !file_is_excluded(&child) {
                rtn = self.find_one_file(ff, h, child, our_dev, false);
                self.set_linked(ff);
            }
        }
        // The directory entry itself after its contents (dir_ff_pkt).
        h.handle(&mut dir_ff, &mut self.links, top_level);
        self.set_linked(&dir_ff);
        if ff.opts.flags.has(FO_KEEPATIME) {
            restore_file_times(&dir_ff, &fname);
        }
        rtn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Collect(Vec<(i32, String, String)>);
    impl Handler for Collect {
        fn handle(&mut self, ff: &mut FfPkt, _l: &mut HashMap<(u64, u64), CurLink>, _top: bool) -> i32 {
            if ff.type_ == FT_DIRBEGIN {
                return 1;
            }
            self.0.push((ff.type_, String::from_utf8_lossy(&ff.fname).into(), String::from_utf8_lossy(&ff.link).into()));
            ff.file_index = self.0.len() as i32;
            1
        }
        fn jmsg(&mut self, _t: i32, _text: String) {}
    }

    #[test]
    fn walks_like_find_one() {
        let tmp = std::env::temp_dir().join(format!("ofd-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("d/sub")).unwrap();
        std::fs::write(tmp.join("d/a.txt"), b"x").unwrap();
        std::fs::write(tmp.join("d/skip.tmp"), b"x").unwrap();
        std::fs::hard_link(tmp.join("d/a.txt"), tmp.join("d/sub/b.txt")).unwrap();
        std::os::unix::fs::symlink("a.txt", tmp.join("d/l")).unwrap();
        let top = tmp.join("d");
        let mut fs = FileSet::default();
        for l in ["I", "O e", "W *.tmp", "N", &format!("F {}", top.display()), "N"] {
            fs.add(l, &mut |m| panic!("{m}"), &|s| s.to_string());
        }
        let mut c = Collect(Vec::new());
        let mut ff = FfPkt::default();
        assert_eq!(Finder::new(&fs).find_files(&mut ff, &mut c), 1);
        let names: Vec<_> = c.0.iter().map(|(t, n, _)| (*t, n.rsplit('/').next().unwrap().to_string())).collect();
        assert!(!names.iter().any(|(_, n)| n == "skip.tmp"));
        assert!(names.contains(&(FT_LNK, "l".into())));
        assert_eq!(names.last().unwrap().0, FT_DIREND);
        assert_eq!(c.0.last().unwrap().2, format!("{}/", top.display()));
        let saved = names.iter().filter(|(t, _)| *t == FT_LNKSAVED).count();
        let regs = names.iter().filter(|(t, n)| *t == FT_REG && (n == "a.txt" || n == "b.txt")).count();
        assert_eq!((saved, regs), (1, 1));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
