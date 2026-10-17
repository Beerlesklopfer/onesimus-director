//! The FileSet the Director sends (`fileset` command): filed/fileset.cc
//! (AddFileset, SetOptionsAndFlags, AddFileToFileset) and the matching of
//! findlib/find.cc (AcceptFile) and findlib/match.cc (ParseSizeMatch).

use std::ffi::CString;
use std::io::BufRead;

use crate::compression::{COMPRESS_FZ4H, COMPRESS_FZ4L, COMPRESS_FZFZ, COMPRESS_GZIP, COMPRESS_LZO1X};

// include/fileopts.h
pub const FO_MD5: u32 = 1;
pub const FO_COMPRESS: u32 = 2;
pub const FO_NO_RECURSION: u32 = 3;
pub const FO_MULTIFS: u32 = 4;
pub const FO_SPARSE: u32 = 5;
pub const FO_IF_NEWER: u32 = 6;
pub const FO_NOREPLACE: u32 = 7;
pub const FO_READFIFO: u32 = 8;
pub const FO_SHA1: u32 = 9;
pub const FO_PORTABLE: u32 = 10;
pub const FO_MTIMEONLY: u32 = 11;
pub const FO_KEEPATIME: u32 = 12;
pub const FO_EXCLUDE: u32 = 13;
pub const FO_ACL: u32 = 14;
pub const FO_NO_HARDLINK: u32 = 15;
pub const FO_IGNORECASE: u32 = 16;
pub const FO_HFSPLUS: u32 = 17;
pub const FO_SHA256: u32 = 19;
pub const FO_SHA512: u32 = 20;
pub const FO_NOATIME: u32 = 22;
pub const FO_ENHANCEDWILD: u32 = 23;
pub const FO_CHKCHANGES: u32 = 24;
pub const FO_STRIPPATH: u32 = 25;
pub const FO_HONOR_NODUMP: u32 = 26;
pub const FO_XATTR: u32 = 27;
pub const FO_OFFSETS: u32 = 30;
pub const FO_NO_AUTOEXCL: u32 = 31;
pub const FO_FORCE_ENCRYPT: u32 = 32;
pub const FO_XXH128: u32 = 33;

/// The FO_* bit set (BitIsSet/SetBit/ClearBit).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags(pub u64);

impl Flags {
    pub fn has(self, bit: u32) -> bool {
        self.0 & (1u64 << bit) != 0
    }
    pub fn set(&mut self, bit: u32) {
        self.0 |= 1u64 << bit;
    }
    pub fn clear(&mut self, bit: u32) {
        self.0 &= !(1u64 << bit);
    }
    pub fn any_digest(self) -> bool {
        [FO_MD5, FO_SHA1, FO_SHA256, FO_SHA512, FO_XXH128].iter().any(|b| self.has(*b))
    }
}

/// s_sz_matching
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeMatch {
    Approx(i64),
    Smaller(i64),
    Greater(i64),
    Range(i64, i64),
}

impl SizeMatch {
    /// CheckSizeMatching()
    pub fn accepts(self, size: i64) -> bool {
        match self {
            SizeMatch::Approx(b) => {
                let diff = (size - b).abs();
                diff < b / 100
            }
            SizeMatch::Smaller(b) => size < b,
            SizeMatch::Greater(b) => size > b,
            SizeMatch::Range(b, e) => size >= b && size <= e,
        }
    }
}

/// size_to_uint64() (lib/edit.cc)
fn size_to_u64(s: &str) -> Option<i64> {
    let s = s.trim();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let n: f64 = digits.parse().ok()?;
    let unit = s[digits.len()..].trim().to_ascii_lowercase();
    let mult: f64 = match unit.as_str() {
        "" | "*" => 1.0,
        "k" => 1024.0,
        "kb" => 1000.0,
        "m" => 1048576.0,
        "mb" => 1e6,
        "g" => 1073741824.0,
        "gb" => 1e9,
        "t" => 1099511627776.0,
        "tb" => 1e12,
        "p" => 1125899906842624.0,
        "pb" => 1e15,
        "e" => 1152921504606846976.0,
        "eb" => 1e18,
        _ => return None,
    };
    Some((n * mult) as i64)
}

/// ParseSizeMatch()
pub fn parse_size_match(p: &str) -> Option<SizeMatch> {
    if let Some((a, b)) = p.split_once('-') {
        return Some(SizeMatch::Range(size_to_u64(a)?, size_to_u64(b)?));
    }
    match p.as_bytes().first() {
        Some(b'<') => Some(SizeMatch::Smaller(size_to_u64(&p[1..])?)),
        Some(b'>') => Some(SizeMatch::Greater(size_to_u64(&p[1..])?)),
        _ => Some(SizeMatch::Approx(size_to_u64(p)?)),
    }
}

/// A compiled POSIX regex (regcomp REG_EXTENDED).
pub struct Regex(Box<libc::regex_t>);

impl Drop for Regex {
    fn drop(&mut self) {
        unsafe { libc::regfree(&mut *self.0) };
    }
}

impl Regex {
    pub fn new(pattern: &str, icase: bool) -> Result<Self, String> {
        let c = CString::new(pattern).map_err(|e| e.to_string())?;
        let mut preg: Box<libc::regex_t> = Box::new(unsafe { std::mem::zeroed() });
        let flags = libc::REG_EXTENDED | if icase { libc::REG_ICASE } else { 0 };
        let rc = unsafe { libc::regcomp(&mut *preg, c.as_ptr(), flags) };
        if rc != 0 {
            let mut buf = [0u8; 500];
            unsafe { libc::regerror(rc, &*preg, buf.as_mut_ptr().cast(), buf.len()) };
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            return Err(String::from_utf8_lossy(&buf[..end]).into_owned());
        }
        Ok(Regex(preg))
    }
    pub fn is_match(&self, s: &[u8]) -> bool {
        let Ok(c) = CString::new(s) else { return false };
        unsafe { libc::regexec(&*self.0, c.as_ptr(), 0, std::ptr::null_mut(), 0) == 0 }
    }
}

const FNM_PATHNAME: i32 = 1 << 0;
const FNM_CASEFOLD: i32 = 1 << 4;

unsafe extern "C" {
    fn fnmatch(pattern: *const libc::c_char, string: *const libc::c_char, flags: libc::c_int) -> libc::c_int;
}

/// fnmatch() == 0
pub fn fn_match(pattern: &[u8], s: &[u8], flags: i32) -> bool {
    let (Ok(p), Ok(s)) = (CString::new(pattern), CString::new(s)) else { return false };
    unsafe { fnmatch(p.as_ptr(), s.as_ptr(), flags) == 0 }
}

/// findFOPTS
#[derive(Default)]
pub struct FOpts {
    pub flags: Flags,
    pub compress_algo: u32,
    pub compress_level: i32,
    pub strip_path: i32,
    pub accurate_opts: String,
    pub verify_opts: String,
    pub size_match: Option<SizeMatch>,
    pub regex: Vec<Regex>,
    pub regexdir: Vec<Regex>,
    pub regexfile: Vec<Regex>,
    pub wild: Vec<Vec<u8>>,
    pub wilddir: Vec<Vec<u8>>,
    pub wildfile: Vec<Vec<u8>>,
    pub wildbase: Vec<Vec<u8>>,
    pub fstype: Vec<String>,
    pub drivetype: Vec<String>,
    pub plugin: Option<String>,
    /// An Encryption cipher is set (Encryption_cipher != NONE).
    pub encryption: bool,
}

/// findIncludeExcludeItem
#[derive(Default)]
pub struct IncExe {
    pub opts: Vec<FOpts>,
    /// The option block being filled (start_options()).
    current_opts: Option<usize>,
    pub names: Vec<Vec<u8>>,
    pub plugins: Vec<String>,
    pub ignoredir: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    None,
    Include,
    Options,
    Error,
}

/// findFILESET
pub struct FileSet {
    pub include: Vec<IncExe>,
    pub exclude: Vec<IncExe>,
    state: State,
    /// incexe: (is_include, index)
    current: Option<(bool, usize)>,
}

impl Default for FileSet {
    fn default() -> Self {
        Self { include: Vec::new(), exclude: Vec::new(), state: State::None, current: None }
    }
}

impl FileSet {
    fn incexe(&mut self) -> Option<&mut IncExe> {
        match self.current? {
            (true, i) => self.include.get_mut(i),
            (false, i) => self.exclude.get_mut(i),
        }
    }

    /// start_options()
    fn start_options(&mut self) -> Option<&mut FOpts> {
        let new = self.state != State::Options;
        self.state = State::Options;
        let incexe = self.incexe()?;
        if new || incexe.current_opts.is_none() {
            incexe.opts.push(FOpts::default());
            incexe.current_opts = Some(incexe.opts.len() - 1);
        }
        let i = incexe.current_opts?;
        incexe.opts.get_mut(i)
    }

    pub fn is_error(&self) -> bool {
        self.state == State::Error
    }

    /// AddFileset(): one line of the `fileset` command. Fatal messages go
    /// to `fatal`; `program` runs `|command` entries (edit_job_codes done).
    pub fn add(&mut self, line: &str, fatal: &mut dyn FnMut(String), program: &dyn Fn(&str) -> String) {
        let bytes = line.as_bytes();
        let code = bytes.first().copied().unwrap_or(0);
        let mut item = if code != 0 { &line[1..] } else { line };
        let mut subcode = b' ';
        if let Some(&c) = item.as_bytes().first() {
            if c != b' ' {
                subcode = c;
                item = &item[1..];
            }
        }
        if item.starts_with(' ') {
            item = &item[1..];
        }
        if self.state == State::Error {
            return;
        }
        let mut state = self.state;
        if subcode != b' ' {
            state = State::Error;
        }
        match code {
            b'I' => {
                self.include.push(IncExe::default());
                self.current = Some((true, self.include.len() - 1));
            }
            b'E' => {
                self.exclude.push(IncExe::default());
                self.current = Some((false, self.exclude.len() - 1));
            }
            b'N' => state = State::None,
            b'F' | b'P' => {
                state = State::Include;
                self.add_file(item, code == b'F', fatal, program);
            }
            b'R' => {
                let icase = self.start_options().map(|o| o.flags.has(FO_IGNORECASE)).unwrap_or(false);
                state = match Regex::new(item, icase) {
                    Ok(re) => {
                        let o = self.start_options();
                        match (subcode, o) {
                            (b' ', Some(o)) => { o.regex.push(re); State::Options }
                            (b'D', Some(o)) => { o.regexdir.push(re); State::Options }
                            (b'F', Some(o)) => { o.regexfile.push(re); State::Options }
                            _ => State::Error,
                        }
                    }
                    Err(e) => {
                        fatal(format!("REGEX {item} compile error. ERR={e}\n"));
                        State::Error
                    }
                };
            }
            b'X' => {
                state = State::Options;
                match (subcode, self.start_options()) {
                    (b' ', Some(o)) => o.fstype.push(item.to_string()),
                    (b'D', Some(o)) => o.drivetype.push(item.to_string()),
                    _ => state = State::Error,
                }
            }
            b'W' => {
                let w = item.as_bytes().to_vec();
                state = match (subcode, self.start_options()) {
                    (b' ', Some(o)) => { o.wild.push(w); State::Options }
                    (b'D', Some(o)) => { o.wilddir.push(w); State::Options }
                    (b'F', Some(o)) => { o.wildfile.push(w); State::Options }
                    (b'B', Some(o)) => { o.wildbase.push(w); State::Options }
                    _ => State::Error,
                };
            }
            b'O' => {
                if let Some(o) = self.start_options() {
                    set_options_and_flags(o, item);
                }
                state = State::Options;
            }
            b'Z' => {
                state = State::Include;
                if let Some(i) = self.incexe() {
                    i.ignoredir.push(item.to_string());
                }
            }
            b'D' | b'T' => {
                self.start_options();
                state = State::Options;
            }
            b'G' => {
                if let Some(o) = self.start_options() {
                    o.plugin = Some(item.to_string());
                }
                state = State::Options;
            }
            _ => {
                fatal(format!("Invalid FileSet command: {item}\n"));
                state = State::Error;
            }
        }
        self.state = state;
    }

    /// AddFileToFileset(): `|program`, `<file` or a name.
    fn add_file(&mut self, fname: &str, is_file: bool, fatal: &mut dyn FnMut(String), program: &dyn Fn(&str) -> String) {
        let mut entries = Vec::new();
        match fname.as_bytes().first() {
            Some(b'|') => {
                let cmd = program(&fname[1..]);
                match crate::runscript::run_program(&cmd) {
                    Ok((out, 0)) => entries.extend(out.lines().map(|l| l.trim_end().to_string())),
                    Ok((out, status)) => {
                        entries.extend(out.lines().map(|l| l.trim_end().to_string()));
                        fatal(format!("Error running program: {}. status={status}: ERR=Child exited with code {status}\n", &fname[1..]));
                    }
                    Err(e) => {
                        fatal(format!("Cannot run program: {}. ERR={e}\n", &fname[1..]));
                        return;
                    }
                }
            }
            Some(b'<') => match std::fs::File::open(&fname[1..]) {
                Ok(f) => entries.extend(std::io::BufReader::new(f).lines().map_while(Result::ok).map(|l| l.trim_end().to_string())),
                Err(e) => {
                    fatal(format!("Cannot open FileSet input file: {}. ERR={}\n", &fname[1..], crate::berrno::s(&e)));
                    return;
                }
            },
            _ => entries.push(fname.to_string()),
        }
        for e in entries {
            if let Some(incexe) = self.incexe() {
                if is_file {
                    if !e.is_empty() {
                        incexe.names.push(e.into_bytes());
                    }
                } else {
                    incexe.plugins.push(e);
                }
            }
        }
    }
}

/// SetOptionsAndFlags()
pub fn set_options_and_flags(fo: &mut FOpts, opts: &str) {
    let b = opts.as_bytes();
    let mut i = 0;
    // up to ':' (or end), returns the text
    let take_until_colon = |i: &mut usize| -> String {
        let start = *i;
        while *i < b.len() && b[*i] != b':' {
            *i += 1;
        }
        String::from_utf8_lossy(&b[start..*i]).into_owned()
    };
    while i < b.len() {
        match b[i] {
            b'A' => fo.flags.set(FO_ACL),
            b'a' | b'0' => {}
            b'C' => {
                fo.accurate_opts = take_until_colon(&mut i);
                if i >= b.len() {
                    break;
                }
            }
            b'c' => fo.flags.set(FO_CHKCHANGES),
            b'd' => {
                if matches!(b.get(i + 1), Some(b'1'..=b'4')) {
                    i += 1;
                }
            }
            b'e' => fo.flags.set(FO_EXCLUDE),
            b'E' => match b.get(i + 1) {
                Some(b'3') => {
                    fo.encryption = true;
                    i += 1;
                }
                Some(b'a') | Some(b'c') | Some(b'h') => {
                    let max = if b.get(i + 1) == Some(&b'h') { b'2' } else { b'3' };
                    if matches!(b.get(i + 2), Some(c) if (b'1'..=max).contains(c)) {
                        fo.encryption = true;
                        i += 2;
                    }
                }
                Some(b'f') => {
                    fo.flags.set(FO_FORCE_ENCRYPT);
                    i += 1;
                }
                _ => {}
            },
            b'f' => fo.flags.set(FO_MULTIFS),
            b'H' => fo.flags.set(FO_NO_HARDLINK),
            b'h' => fo.flags.set(FO_NO_RECURSION),
            b'i' => fo.flags.set(FO_IGNORECASE),
            b'K' => fo.flags.set(FO_NOATIME),
            b'k' => fo.flags.set(FO_KEEPATIME),
            b'M' => fo.flags.set(FO_MD5),
            b'm' => fo.flags.set(FO_MTIMEONLY),
            b'N' => fo.flags.set(FO_HONOR_NODUMP),
            b'n' => fo.flags.set(FO_NOREPLACE),
            b'P' => {
                i += 1;
                let s = take_until_colon(&mut i);
                fo.strip_path = s.trim().parse().unwrap_or(0);
                fo.flags.set(FO_STRIPPATH);
                if i >= b.len() {
                    break;
                }
            }
            b'p' => fo.flags.set(FO_PORTABLE),
            b'R' => fo.flags.set(FO_HFSPLUS),
            b'r' => fo.flags.set(FO_READFIFO),
            b'S' => match b.get(i + 1) {
                Some(b'1') => {
                    fo.flags.set(FO_SHA1);
                    i += 1;
                }
                Some(b'2') => {
                    fo.flags.set(FO_SHA256);
                    i += 1;
                }
                Some(b'3') => {
                    fo.flags.set(FO_SHA512);
                    i += 1;
                }
                Some(b'4') => {
                    fo.flags.set(FO_XXH128);
                    i += 1;
                }
                _ => fo.flags.set(FO_SHA1),
            },
            b's' => fo.flags.set(FO_SPARSE),
            b'V' => {
                fo.verify_opts = take_until_colon(&mut i);
                if i >= b.len() {
                    break;
                }
            }
            b'W' => fo.flags.set(FO_ENHANCEDWILD),
            b'w' => fo.flags.set(FO_IF_NEWER),
            b'x' => fo.flags.set(FO_NO_AUTOEXCL),
            b'X' => fo.flags.set(FO_XATTR),
            b'Z' => {
                i += 1;
                match b.get(i) {
                    Some(d @ b'0'..=b'9') => {
                        fo.flags.set(FO_COMPRESS);
                        fo.compress_algo = COMPRESS_GZIP;
                        fo.compress_level = i32::from(d - b'0');
                    }
                    Some(b'o') => {
                        fo.flags.set(FO_COMPRESS);
                        fo.compress_algo = COMPRESS_LZO1X;
                        fo.compress_level = 1;
                    }
                    Some(b'f') => {
                        i += 1;
                        let algo = match b.get(i) {
                            Some(b'f') => Some(COMPRESS_FZFZ),
                            Some(b'4') => Some(COMPRESS_FZ4L),
                            Some(b'h') => Some(COMPRESS_FZ4H),
                            _ => None,
                        };
                        if let Some(a) = algo {
                            fo.flags.set(FO_COMPRESS);
                            fo.compress_algo = a;
                            fo.compress_level = 1;
                        }
                    }
                    _ => {}
                }
            }
            b'z' => {
                i += 1;
                let s = take_until_colon(&mut i);
                fo.size_match = parse_size_match(&s);
                if fo.size_match.is_none() {
                    tracing::error!("Unparsable size option: {s}");
                }
                if i >= b.len() {
                    break;
                }
            }
            c => tracing::error!("Unknown include/exclude option: {}", c as char),
        }
        i += 1;
    }
}

/// FileIsExcluded() with the (empty) excluded_files_list: the FileSet
/// excludes are applied in AcceptFile().
pub fn file_is_excluded(_name: &[u8]) -> bool {
    false
}

/// The per-file option values AcceptFile() copies into the ff packet.
#[derive(Debug, Clone, Default)]
pub struct FileOpts {
    pub flags: Flags,
    pub compress_algo: u32,
    pub compress_level: i32,
}

/// AcceptFile(): whether `fname` is taken, with the option values of the
/// option block that decided (or the last one).
pub fn accept_file(fs: &FileSet, incexe: &IncExe, fname: &[u8], is_dir: bool, cur: &mut FileOpts) -> bool {
    let basename = |enhanced: bool| -> &[u8] {
        if enhanced {
            match fname.iter().rposition(|&b| b == b'/') {
                Some(p) => &fname[p + 1..],
                None => fname,
            }
        } else {
            fname
        }
    };
    for fo in &incexe.opts {
        cur.flags = fo.flags;
        cur.compress_algo = fo.compress_algo;
        cur.compress_level = fo.compress_level;
        let mut fnm = if fo.flags.has(FO_IGNORECASE) { FNM_CASEFOLD } else { 0 };
        let enhanced = fo.flags.has(FO_ENHANCEDWILD);
        if enhanced {
            fnm |= FNM_PATHNAME;
        }
        let exclude = fo.flags.has(FO_EXCLUDE);
        let decide = || !exclude;
        if is_dir {
            if fo.wilddir.iter().any(|w| fn_match(w, fname, fnm)) {
                return decide();
            }
        } else {
            if fo.wildfile.iter().any(|w| fn_match(w, fname, fnm)) {
                return decide();
            }
            let base = basename(enhanced);
            if fo.wildbase.iter().any(|w| fn_match(w, base, fnm)) {
                return decide();
            }
        }
        if fo.wild.iter().any(|w| fn_match(w, fname, fnm)) {
            return decide();
        }
        if is_dir {
            if fo.regexdir.iter().any(|r| r.is_match(fname)) {
                return decide();
            }
        } else if fo.regexfile.iter().any(|r| r.is_match(fname)) {
            return decide();
        }
        if fo.regex.iter().any(|r| r.is_match(fname)) {
            return decide();
        }
        if exclude && fo.regex.is_empty() && fo.wild.is_empty() && fo.regexdir.is_empty() && fo.wilddir.is_empty()
            && fo.regexfile.is_empty() && fo.wildfile.is_empty() && fo.wildbase.is_empty()
        {
            return false;
        }
    }
    for ex in &fs.exclude {
        for fo in &ex.opts {
            let fnm = if fo.flags.has(FO_IGNORECASE) { FNM_CASEFOLD } else { 0 };
            if fo.wild.iter().any(|w| fn_match(w, fname, fnm)) {
                return false;
            }
        }
        let fnm = match ex.current_opts.and_then(|i| ex.opts.get(i)) {
            Some(o) if o.flags.has(FO_IGNORECASE) => FNM_CASEFOLD,
            _ => 0,
        };
        if ex.names.iter().any(|n| fn_match(n, fname, fnm)) {
            return false;
        }
    }
    true
}

/// IsInFileset(): the name is an explicit Include or Exclude entry.
pub fn is_in_fileset(fs: &FileSet, fname: &[u8]) -> bool {
    fs.include.iter().chain(fs.exclude.iter()).any(|i| i.names.iter().any(|n| n == fname))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(lines: &[&str]) -> FileSet {
        let mut fs = FileSet::default();
        for l in lines {
            fs.add(l, &mut |m| panic!("{m}"), &|s| s.to_string());
        }
        fs
    }

    #[test]
    fn parses_options_and_matches() {
        let fs = build(&["I", "O MZ6sAXS4", "N", "O e", "W *.tmp", "N", "F /etc", "F /var/lib", "N", "E", "F /etc/shadow", "N"]);
        assert_eq!(fs.include.len(), 1);
        let inc = &fs.include[0];
        assert_eq!(inc.names, vec![b"/etc".to_vec(), b"/var/lib".to_vec()]);
        assert_eq!(inc.opts.len(), 2);
        let o = &inc.opts[0];
        for f in [FO_MD5, FO_COMPRESS, FO_SPARSE, FO_ACL, FO_XATTR, FO_XXH128] {
            assert!(o.flags.has(f), "{f}");
        }
        assert_eq!((o.compress_algo, o.compress_level), (COMPRESS_GZIP, 6));
        let mut cur = FileOpts::default();
        assert!(!accept_file(&fs, inc, b"/etc/a.tmp", false, &mut cur));
        assert!(accept_file(&fs, inc, b"/etc/a.conf", false, &mut cur));
        assert!(cur.flags.has(FO_EXCLUDE), "flags of the last block");
        assert!(!accept_file(&fs, inc, b"/etc/shadow", false, &mut cur));
        assert!(is_in_fileset(&fs, b"/var/lib"));
    }

    #[test]
    fn size_and_accurate_options() {
        let fs = build(&["I", "O Cmcs5:z<10k:P2:VpinsM:Zf4", "N", "F /x", "N"]);
        let o = &fs.include[0].opts[0];
        assert_eq!(o.accurate_opts, "Cmcs5");
        assert_eq!(o.size_match, Some(SizeMatch::Smaller(10240)));
        assert_eq!(o.strip_path, 2);
        assert_eq!(o.verify_opts, "VpinsM");
        assert_eq!(o.compress_algo, COMPRESS_FZ4L);
        assert!(SizeMatch::Range(5, 10).accepts(7));
        assert!(SizeMatch::Approx(1000).accepts(1005));
    }
}
