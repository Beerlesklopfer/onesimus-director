//! Port of `ovfs-core/src/tree.rs` of the OVFS PostgreSQL extension
//! (https://github.com/Beerlesklopfer/oenesimus-postgres-plugin, commit
//! `4041335`), the Director-side fallback when the extension is not installed.
//! Keep it in step with the extension: the fallback must produce the same
//! JSON text byte for byte.
//!
//! Directory trees from the Bareos catalog rows of a job chain, built while
//! the rows stream in, so memory does not grow with the size of the backup
//! (the database may run on a single-board computer with 1-2 GB RAM).
//!
//! Catalog conventions (Bareos schema 2250):
//! - `Path.Path` is a directory and always ends with `/` (`/`, `/etc/`, `C:/`).
//! - A directory's own attributes are a `File` row in that path with `Name = ''`.
//! - `FileIndex = 0` marks an entry deleted since the previous job (accurate
//!   mode). It hides older versions of the entry and is not shown itself.
//! - `DeltaSeq > 0` marks a delta part of a file (plugin backups); a restore
//!   needs the base version (`DeltaSeq = 0`) and every part in sequence.
//! - Path and name are bytes in a `SQL_ASCII` database and need not be UTF-8.
//!
//! A job chain (Full + Differential + Incrementals, with base jobs) is
//! resolved as Bareos does for a restore (`GetFileList` with
//! `select_recent_version_with_basejob_and_delta`, `ua_tree.cc`
//! `InsertTreeHandler`): per path, name and DeltaSeq the newest job
//! (`JobTDate`) wins, winning deletions drop out, and the remaining versions
//! are applied in `JobTDate` order, delta parts only on top of their
//! predecessor.
//!
//! Input order: [`TreeWriter`] and [`DirLister`] need the rows sorted by the
//! bytes of path, then name (`ORDER BY path, name` in a `C` collation). That
//! is depth-first order, and all versions of an entry are adjacent. Children
//! therefore come files first (by name bytes), then directories (by the
//! bytes of name + `/`, the order of their paths).

use std::cmp::Ordering;

use base64::Engine;

use super::lstat;

/// One `File` row joined with its `Path` and `Job`.
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: Vec<u8>,
    pub name: Vec<u8>,
    pub jobid: i32,
    pub jobtdate: i64,
    pub fileid: i64,
    pub fileindex: i32,
    pub delta_seq: i32,
    pub lstat: String,
}

#[derive(Debug, Clone)]
struct Attrs {
    jobid: i32,
    jobtdate: i64,
    fileid: i64,
    fileindex: i32,
    delta_seq: i32,
    stat: Option<lstat::Stat>,
}

impl Attrs {
    fn is_live(&self) -> bool {
        self.fileindex > 0
    }

    fn is_hard_link(&self) -> bool {
        self.stat.is_some_and(|s| s.link_fi != 0)
    }

    /// Total order of versions with the same DeltaSeq; the greatest one wins.
    ///
    /// The newest job wins (`JobTDate`, then `JobId` where Bareos leaves the
    /// choice open). Within one job Bareos keeps the last copy of a file but
    /// the first copy of a hard link, so the FileIndex counts the other way
    /// round for hard links. `FileId` only makes the order total.
    fn precedence(&self, other: &Self) -> Ordering {
        let rank = |a: &Self| if a.is_hard_link() { -i64::from(a.fileindex) } else { i64::from(a.fileindex) };
        (self.jobtdate, self.jobid)
            .cmp(&(other.jobtdate, other.jobid))
            .then_with(|| rank(self).cmp(&rank(other)))
            .then_with(|| self.fileid.cmp(&other.fileid))
    }
}

/// The winning version of each DeltaSeq of one entry.
#[derive(Debug, Default)]
struct Versions(Vec<Attrs>);

/// The version a restore uses, with the delta parts it is applied on.
#[derive(Debug, Clone)]
struct Resolved {
    attrs: Attrs,
    /// Earlier parts in sequence, starting with the base version.
    parts: Vec<Attrs>,
}

impl Versions {
    fn add(&mut self, attrs: Attrs) {
        match self.0.iter_mut().find(|old| old.delta_seq == attrs.delta_seq) {
            Some(old) if attrs.precedence(old) == Ordering::Greater => *old = attrs,
            Some(_) => {}
            None => self.0.push(attrs),
        }
    }

    /// Apply the live versions in `JobTDate` order (`ORDER BY JobTDate,
    /// FileIndex`). A delta part extends the version of the previous
    /// sequence number; without that base the entry is dropped, and a part
    /// out of sequence is skipped.
    fn resolve(mut self) -> Option<Resolved> {
        self.0.retain(Attrs::is_live);
        self.0.sort_by_key(|a| (a.jobtdate, a.fileindex, a.jobid, a.fileid));
        let mut current: Option<Resolved> = None;
        for attrs in self.0 {
            if attrs.delta_seq <= 0 {
                current = Some(Resolved { attrs, parts: Vec::new() });
            } else if let Some(resolved) = current.as_mut() {
                if attrs.delta_seq == resolved.attrs.delta_seq + 1 {
                    let previous = std::mem::replace(&mut resolved.attrs, attrs);
                    resolved.parts.push(previous);
                }
            }
        }
        current
    }
}

/// Collects adjacent rows of the same path and name.
#[derive(Debug, Default)]
struct Grouper {
    key: Option<(Vec<u8>, Vec<u8>)>,
    versions: Versions,
}

/// One resolved entry: its path, name and the version a restore uses.
type Group = (Vec<u8>, Vec<u8>, Option<Resolved>);

impl Grouper {
    /// Add a row; returns the previous entry when the path or name changes.
    fn add(&mut self, entry: Entry) -> Option<Group> {
        let Entry { path, name, jobid, jobtdate, fileid, fileindex, delta_seq, lstat } = entry;
        let same = matches!(&self.key, Some((p, n)) if *p == path && *n == name);
        let done = if same { None } else { self.finish() };
        if self.key.is_none() {
            self.key = Some((path, name));
        }
        self.versions.add(Attrs { jobid, jobtdate, fileid, fileindex, delta_seq, stat: lstat::decode(&lstat) });
        done
    }

    fn finish(&mut self) -> Option<Group> {
        let (path, name) = self.key.take()?;
        Some((path, name, std::mem::take(&mut self.versions).resolve()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TreeError {
    /// The root must be a directory path ending with `/`.
    InvalidRoot,
}

/// Normalize a directory path to the catalog form (trailing `/`).
pub fn normalize_root(root: &str) -> Result<Vec<u8>, TreeError> {
    if root.is_empty() || root.contains('\0') {
        return Err(TreeError::InvalidRoot);
    }
    let mut bytes = root.as_bytes().to_vec();
    if !bytes.ends_with(b"/") {
        bytes.push(b'/');
    }
    Ok(bytes)
}

/// Exclusive upper bound for a byte-wise range scan over all paths starting
/// with `prefix` (`prefix <= path < upper`, compared with `COLLATE "C"`).
/// `None` means there is no upper bound (prefix is all `0xff`).
pub fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut upper = prefix.to_vec();
    while let Some(last) = upper.pop() {
        if last < 0xff {
            upper.push(last + 1);
            return Some(upper);
        }
    }
    None
}

/// Path components of `path` below `root` (`/etc/ssh/` below `/` → etc, ssh).
fn components<'a>(root: &[u8], path: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
    path[root.len()..].split(|b| *b == b'/').filter(|c| !c.is_empty())
}

#[derive(Debug)]
struct Frame {
    name: Vec<u8>,
    path: Vec<u8>,
    attrs: Option<Resolved>,
    /// Header written: the directory or something below it is live.
    open: bool,
    children: usize,
    /// Live entries below a directory at the depth limit (written without children).
    entries: u64,
}

impl Frame {
    fn new(name: &[u8], path: Vec<u8>) -> Self {
        Frame { name: name.to_vec(), path, attrs: None, open: false, children: 0, entries: 0 }
    }
}

/// Writes the JSON tree below `root` down to `depth` levels while sorted
/// rows stream in. Memory is the output plus one entry and the stack of
/// open directories, however large the backup is.
///
/// Directories at the depth limit are written without `children` but with
/// `entries` (live entries below them, as `ls` counts them); a tree model
/// loads them with `ls` when they are expanded. Depth 1 is the root with its
/// direct children, depth 0 the root alone.
///
/// A directory is written only once something live is found at or below
/// it. Keys come in a fixed order with `children` last, so a client reading
/// the text as a stream knows a node before its children arrive.
#[derive(Debug)]
pub struct TreeWriter {
    root: Vec<u8>,
    depth: usize,
    out: Vec<u8>,
    stack: Vec<Frame>,
    group: Grouper,
    nodes: u64,
}

impl TreeWriter {
    pub fn new(root: &[u8], depth: usize) -> Self {
        let top = Frame::new(root, root.to_vec());
        TreeWriter { root: root.to_vec(), depth, out: Vec::new(), stack: vec![top], group: Grouper::default(), nodes: 0 }
    }

    /// Nodes written so far, to limit the size of the output.
    pub fn nodes(&self) -> u64 {
        self.nodes
    }

    /// Add one row (sorted by path, then name); rows outside the root are ignored.
    pub fn add(&mut self, entry: Entry) {
        if entry.path.starts_with(&self.root) {
            if let Some(group) = self.group.add(entry) {
                self.entry(group);
            }
        }
    }

    /// The tree as JSON text, `None` if no live entry remains below the root.
    pub fn finish(mut self) -> Option<String> {
        if let Some(group) = self.group.finish() {
            self.entry(group);
        }
        while !self.stack.is_empty() {
            self.close();
        }
        (!self.out.is_empty()).then(|| String::from_utf8(self.out).expect("JSON text is built from valid UTF-8"))
    }

    fn entry(&mut self, (path, name, resolved): Group) {
        let Some(resolved) = resolved else {
            return;
        };
        let level = components(&self.root, &path).count();
        self.descend(&path);
        let own_row = name.is_empty();
        if level > self.depth || (level == self.depth && !own_row) {
            // Below the depth limit: only counted for the directory at the limit
            self.stack.last_mut().expect("root frame").entries += 1;
        } else if own_row {
            let top = self.stack.last_mut().expect("root frame");
            // Rows of a directory start with its own (name ''); later is out of order
            if !top.open {
                top.attrs = Some(resolved);
            }
            if level < self.depth {
                self.open();
            }
        } else {
            self.open();
            self.separator();
            self.nodes += 1;
            write_file(&mut self.out, &name, &resolved);
        }
    }

    /// Close the directories not containing `path` and push the missing ones,
    /// down to the depth limit.
    fn descend(&mut self, path: &[u8]) {
        let wanted: Vec<&[u8]> = components(&self.root, path).take(self.depth).collect();
        let common = self.stack[1..].iter().zip(&wanted).take_while(|(frame, name)| frame.name == **name).count();
        while self.stack.len() > common + 1 {
            self.close();
        }
        for name in &wanted[common..] {
            let mut child_path = self.stack.last().expect("root frame").path.clone();
            child_path.extend_from_slice(name);
            child_path.push(b'/');
            self.stack.push(Frame::new(name, child_path));
        }
    }

    /// Comma before the next child of the directory on top of the stack.
    fn separator(&mut self) {
        let top = self.stack.last_mut().expect("root frame");
        if top.children > 0 {
            self.out.push(b',');
        }
        top.children += 1;
    }

    /// Write the headers of all directories on the stack not yet written.
    fn open(&mut self) {
        for i in 0..self.stack.len() {
            if self.stack[i].open {
                continue;
            }
            if i > 0 {
                let parent = &mut self.stack[i - 1];
                if parent.children > 0 {
                    self.out.push(b',');
                }
                parent.children += 1;
            }
            self.nodes += 1;
            let frame = &mut self.stack[i];
            frame.open = true;
            write_dir_head(&mut self.out, frame);
            self.out.extend_from_slice(b",\"children\":[");
        }
    }

    fn close(&mut self) {
        let at_limit = self.stack.len().checked_sub(1) == Some(self.depth);
        let frame = self.stack.pop().expect("frame to close");
        if frame.open {
            self.out.extend_from_slice(b"]}");
        } else if at_limit && (frame.attrs.is_some() || frame.entries > 0) {
            if !self.stack.is_empty() {
                self.open();
                self.separator();
            }
            self.nodes += 1;
            write_dir_head(&mut self.out, &frame);
            write_number(&mut self.out, "entries", frame.entries as i64);
            self.out.push(b'}');
        }
    }
}

/// `{` and the keys of a directory node up to (not including) its children.
fn write_dir_head(out: &mut Vec<u8>, frame: &Frame) {
    out.push(b'{');
    write_text(out, "name", &frame.name);
    out.push(b',');
    write_text(out, "path", &frame.path);
    out.extend_from_slice(b",\"type\":\"directory\"");
    if let Some(attrs) = &frame.attrs {
        write_attrs(out, attrs);
    }
}

/// Catalog row of one delta part (or the base version) of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub jobid: i32,
    pub fileindex: i32,
}

/// Version of an entry that a restore of the chain uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub jobid: i32,
    pub fileid: i64,
    pub fileindex: i32,
    pub delta_seq: i32,
    /// Base version and earlier delta parts, in sequence; empty without deltas.
    pub delta_parts: Vec<Part>,
    pub stat: Option<lstat::Stat>,
}

impl From<&Resolved> for Version {
    fn from(resolved: &Resolved) -> Self {
        let attrs = &resolved.attrs;
        Version {
            jobid: attrs.jobid,
            fileid: attrs.fileid,
            fileindex: attrs.fileindex,
            delta_seq: attrs.delta_seq,
            delta_parts: resolved.parts.iter().map(|p| Part { jobid: p.jobid, fileindex: p.fileindex }).collect(),
            stat: attrs.stat,
        }
    }
}

/// One direct child of a directory, for lazy loading into a tree model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Child {
    pub name: Vec<u8>,
    /// Catalog path of a directory child (ends with `/`), `None` for files.
    pub path: Option<Vec<u8>>,
    pub kind: lstat::Kind,
    /// `None` for a directory without an own catalog row.
    pub version: Option<Version>,
    /// Live entries below a directory child, `None` for files.
    pub entries: Option<u64>,
}

#[derive(Debug)]
struct SubDir {
    name: Vec<u8>,
    version: Option<Version>,
    entries: u64,
}

/// Lists the direct children of `dir` while sorted rows stream in. Memory is
/// one entry plus the children of `dir`; deeper entries are only counted.
#[derive(Debug)]
pub struct DirLister {
    dir: Vec<u8>,
    files: Vec<Child>,
    /// Direct subdirectories in path order; the last one is being counted.
    dirs: Vec<SubDir>,
    group: Grouper,
}

impl DirLister {
    pub fn new(dir: &[u8]) -> Self {
        DirLister { dir: dir.to_vec(), files: Vec::new(), dirs: Vec::new(), group: Grouper::default() }
    }

    /// Add one row (sorted by path, then name); rows outside `dir` are ignored.
    pub fn add(&mut self, entry: Entry) {
        if entry.path.starts_with(&self.dir) {
            if let Some(group) = self.group.add(entry) {
                self.entry(group);
            }
        }
    }

    /// Files first, then directories, each in the input order. Directories
    /// with nothing live at or below them are left out.
    pub fn finish(mut self) -> Vec<Child> {
        if let Some(group) = self.group.finish() {
            self.entry(group);
        }
        let dir = self.dir;
        let dirs = self.dirs.into_iter().filter(|d| d.version.is_some() || d.entries > 0).map(|d| {
            let mut path = dir.clone();
            path.extend_from_slice(&d.name);
            path.push(b'/');
            Child { name: d.name, path: Some(path), kind: lstat::Kind::Directory, version: d.version, entries: Some(d.entries) }
        });
        self.files.into_iter().chain(dirs).collect()
    }

    fn entry(&mut self, (path, name, resolved): Group) {
        let mut below = components(&self.dir, &path);
        let Some(first) = below.next() else {
            // A file in `dir`; the row of `dir` itself (name '') is not listed
            if let (false, Some(resolved)) = (name.is_empty(), &resolved) {
                let kind = resolved.attrs.stat.map_or(lstat::Kind::Other, |s| s.kind());
                self.files.push(Child { name, path: None, kind, version: Some(resolved.into()), entries: None });
            }
            return;
        };
        if self.dirs.last().is_none_or(|d| d.name != first) {
            self.dirs.push(SubDir { name: first.to_vec(), version: None, entries: 0 });
        }
        let sub = self.dirs.last_mut().expect("pushed above");
        let own_row = name.is_empty() && below.next().is_none();
        match resolved {
            Some(resolved) if own_row => sub.version = Some((&resolved).into()),
            Some(_) => sub.entries += 1,
            None => {}
        }
    }
}

#[cfg(test)]
/// Sort rows as the database delivers them to [`TreeWriter`] and [`DirLister`].
pub fn sort_rows(rows: &mut [Entry]) {
    rows.sort_by(|a, b| (&a.path, &a.name).cmp(&(&b.path, &b.name)));
}

#[cfg(test)]
/// Build the tree below `root` from unsorted rows (sorts them first).
/// Returns `None` if no live entry remains at or below `root`.
pub fn build(root: &[u8], entries: impl IntoIterator<Item = Entry>) -> Option<serde_json::Value> {
    build_to_depth(root, usize::MAX, entries)
}

#[cfg(test)]
/// Like [`build`], down to `depth` levels; see [`TreeWriter`].
pub fn build_to_depth(root: &[u8], depth: usize, entries: impl IntoIterator<Item = Entry>) -> Option<serde_json::Value> {
    let mut rows: Vec<Entry> = entries.into_iter().collect();
    sort_rows(&mut rows);
    let mut writer = TreeWriter::new(root, depth);
    rows.into_iter().for_each(|entry| writer.add(entry));
    writer.finish().map(|text| serde_json::from_str(&text).expect("TreeWriter writes valid JSON"))
}

#[cfg(test)]
/// Direct children of `dir` from unsorted rows; see [`DirLister`].
pub fn list(dir: &[u8], entries: impl IntoIterator<Item = Entry>) -> Vec<Child> {
    let mut rows: Vec<Entry> = entries.into_iter().collect();
    sort_rows(&mut rows);
    let mut lister = DirLister::new(dir);
    rows.into_iter().for_each(|entry| lister.add(entry));
    lister.finish()
}

fn write_str(out: &mut Vec<u8>, text: &str) {
    serde_json::to_writer(&mut *out, text).expect("writing to a Vec cannot fail");
}

/// `"key":` followed by the text form of `bytes`, plus `key_b64` with the
/// exact bytes if they are not UTF-8 (the readable form is then lossy).
fn write_text(out: &mut Vec<u8>, key: &str, bytes: &[u8]) {
    write_str(out, key);
    out.push(b':');
    match std::str::from_utf8(bytes) {
        Ok(text) => write_str(out, text),
        Err(_) => {
            write_str(out, &String::from_utf8_lossy(bytes));
            out.push(b',');
            write_str(out, &format!("{key}_b64"));
            out.push(b':');
            write_str(out, &base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }
}

fn write_number(out: &mut Vec<u8>, key: &str, value: i64) {
    out.push(b',');
    write_str(out, key);
    out.push(b':');
    out.extend_from_slice(value.to_string().as_bytes());
}

/// Restore identity (`jobid`, `fileindex`, delta parts) and the decoded
/// LStat. `link_fileindex` refers to a FileIndex in the same job.
fn write_attrs(out: &mut Vec<u8>, resolved: &Resolved) {
    let attrs = &resolved.attrs;
    write_number(out, "jobid", attrs.jobid.into());
    write_number(out, "fileid", attrs.fileid);
    write_number(out, "fileindex", attrs.fileindex.into());
    if attrs.delta_seq > 0 {
        write_number(out, "delta_seq", attrs.delta_seq.into());
        out.extend_from_slice(b",\"delta_parts\":[");
        for (i, part) in resolved.parts.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            out.extend_from_slice(format!("{{\"jobid\":{},\"fileindex\":{}}}", part.jobid, part.fileindex).as_bytes());
        }
        out.push(b']');
    }
    if let Some(stat) = attrs.stat {
        write_number(out, "mode", stat.mode);
        write_number(out, "uid", stat.uid);
        write_number(out, "gid", stat.gid);
        write_number(out, "size", stat.size);
        write_number(out, "mtime", stat.mtime);
        if stat.link_fi != 0 {
            write_number(out, "link_fileindex", stat.link_fi);
        }
    }
}

fn write_file(out: &mut Vec<u8>, name: &[u8], resolved: &Resolved) {
    out.push(b'{');
    write_text(out, "name", name);
    let kind = resolved.attrs.stat.map_or(lstat::Kind::Other, |s| s.kind());
    out.extend_from_slice(format!(",\"type\":\"{}\"", kind.as_str()).as_bytes());
    write_attrs(out, resolved);
    out.push(b'}');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use super::super::lstat::tests::encode;

    fn stat(mode: i64, size: i64) -> String {
        encode([0, 0, mode, 1, 0, 0, 0, size, 4096, 0, 0, 1_700_000_000, 0, 0, 0, 0])
    }

    fn row(path: &str, name: &str, fileid: i64, fileindex: i32, mode: i64) -> Entry {
        job_row(1, path, name, fileid, fileindex, mode)
    }

    /// Row of job `jobid`; jobs run in JobId order (`JobTDate = 1000 * JobId`).
    fn job_row(jobid: i32, path: &str, name: &str, fileid: i64, fileindex: i32, mode: i64) -> Entry {
        Entry {
            path: path.as_bytes().to_vec(),
            name: name.as_bytes().to_vec(),
            jobid,
            jobtdate: 1000 * i64::from(jobid),
            fileid,
            fileindex,
            delta_seq: 0,
            lstat: stat(mode, fileid * 10),
        }
    }

    fn hard_link(mut entry: Entry, link_fi: i64) -> Entry {
        entry.lstat = encode([0, 0, 0o100644, 2, 0, 0, 0, 1, 4096, 0, 0, 1_700_000_000, 0, link_fi, 0, 0]);
        entry
    }

    fn child<'a>(node: &'a Value, name: &str) -> &'a Value {
        node["children"].as_array().unwrap().iter().find(|c| c["name"] == name).unwrap()
    }

    fn names(node: &Value) -> Vec<&str> {
        node["children"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect()
    }

    #[test]
    fn root_normalization() {
        assert_eq!(normalize_root("/etc").unwrap(), b"/etc/");
        assert_eq!(normalize_root("/").unwrap(), b"/");
        assert_eq!(normalize_root("C:/").unwrap(), b"C:/");
        assert_eq!(normalize_root(""), Err(TreeError::InvalidRoot));
        assert_eq!(normalize_root("/a\0b"), Err(TreeError::InvalidRoot));
    }

    #[test]
    fn upper_bound() {
        assert_eq!(prefix_upper_bound(b"/etc/").unwrap(), b"/etc0");
        assert_eq!(prefix_upper_bound(b"/a\xff").unwrap(), b"/b");
        assert_eq!(prefix_upper_bound(b"\xff\xff"), None);
    }

    #[test]
    fn builds_nested_tree() {
        let tree = build(
            b"/etc/",
            [
                row("/etc/", "", 1, 1, 0o040755),
                row("/etc/", "passwd", 2, 2, 0o100644),
                row("/etc/ssh/", "", 3, 3, 0o040755),
                row("/etc/ssh/", "sshd_config", 4, 4, 0o100600),
                row("/etc/", "localtime", 5, 5, 0o120777),
                row("/var/", "log", 6, 6, 0o100644),
            ],
        )
        .unwrap();

        assert_eq!(tree["path"], "/etc/");
        assert_eq!(tree["fileid"], 1);
        assert_eq!(names(&tree), ["localtime", "passwd", "ssh"]);
        let children = tree["children"].as_array().unwrap();
        assert_eq!(children[0]["type"], "symlink");
        assert_eq!(children[1]["type"], "file");
        assert_eq!(children[1]["size"], 20);
        assert_eq!(children[2]["type"], "directory");
        assert_eq!(children[2]["path"], "/etc/ssh/");
        assert_eq!(names(&children[2]), ["sshd_config"]);
    }

    #[test]
    fn creates_intermediate_directories() {
        // Only a deep path was backed up; "/" and "/home/" have no own rows
        let tree = build(b"/", [row("/home/user/", "notes.txt", 1, 1, 0o100644)]).unwrap();
        assert!(tree.get("fileid").is_none());
        let home = &tree["children"][0];
        assert_eq!(home["path"], "/home/");
        assert!(home.get("fileid").is_none());
        assert_eq!(names(&home["children"][0]), ["notes.txt"]);
    }

    #[test]
    fn skips_deleted_entries() {
        let tree = build(
            b"/data/",
            [
                row("/data/", "kept", 1, 1, 0o100644),
                row("/data/", "deleted", 2, 0, 0o100644),
                row("/data/gone/", "", 3, 0, 0o040755),
            ],
        )
        .unwrap();
        assert_eq!(names(&tree), ["kept"]);
    }

    #[test]
    fn empty_result_is_none() {
        assert!(build(b"/nothing/", [row("/etc/", "passwd", 1, 1, 0o100644)]).is_none());
        assert!(build(b"/etc/", [row("/etc/", "deleted", 1, 0, 0o100644)]).is_none());
    }

    #[test]
    fn prefix_must_match_whole_component() {
        // "/etc/" must not match "/etcetera/" — guaranteed by the trailing slash
        assert!(build(b"/etc/", [row("/etcetera/", "x", 1, 1, 0o100644)]).is_none());
    }

    #[test]
    fn keeps_non_utf8_names_exactly() {
        let tree = build(
            b"/",
            [Entry { name: b"caf\xe9".to_vec(), ..row("/", "x", 1, 1, 0o100644) }],
        )
        .unwrap();
        let file = &tree["children"][0];
        assert_eq!(file["name"], "caf\u{fffd}");
        assert_eq!(file["name_b64"], "Y2Fm6Q==");
    }

    #[test]
    fn unicode_and_spaces() {
        let tree = build(b"/", [row("/Übersicht mit Leerzeichen/", "日本語.txt", 1, 1, 0o100644)]).unwrap();
        let dir = &tree["children"][0];
        assert_eq!(dir["name"], "Übersicht mit Leerzeichen");
        assert!(dir.get("name_b64").is_none());
        assert_eq!(names(dir), ["日本語.txt"]);
    }

    #[test]
    fn output_is_deterministic() {
        let rows = vec![row("/b/", "z", 1, 1, 0o100644), row("/a/", "y", 2, 2, 0o100644), row("/", "x", 3, 3, 0o100644)];
        let mut reversed = rows.clone();
        reversed.reverse();
        assert_eq!(build(b"/", rows), build(b"/", reversed));
    }

    #[test]
    fn newest_job_wins() {
        let full = job_row(10, "/etc/", "passwd", 1, 5, 0o100644);
        let incr = job_row(11, "/etc/", "passwd", 2, 3, 0o100600);
        let tree = build(b"/etc/", [incr.clone(), full.clone()]).unwrap();
        let passwd = child(&tree, "passwd");
        assert_eq!((passwd["jobid"].as_i64(), passwd["fileindex"].as_i64()), (Some(11), Some(3)));
        assert_eq!(passwd["mode"], 0o100600);
        assert_eq!(build(b"/etc/", [full, incr]), Some(tree));
    }

    #[test]
    fn jobtdate_decides_not_jobid() {
        // A copy or migration job can have a higher JobId but an older JobTDate
        let mut copy = job_row(20, "/etc/", "passwd", 2, 1, 0o100600);
        copy.jobtdate = 500;
        let tree = build(b"/etc/", [job_row(10, "/etc/", "passwd", 1, 1, 0o100644), copy]).unwrap();
        assert_eq!(child(&tree, "passwd")["jobid"], 10);
    }

    #[test]
    fn unchanged_files_come_from_older_jobs() {
        let tree = build(
            b"/",
            [
                job_row(10, "/etc/", "", 1, 1, 0o040755),
                job_row(10, "/etc/", "hosts", 2, 2, 0o100644),
                job_row(10, "/etc/", "passwd", 3, 3, 0o100644),
                job_row(11, "/etc/", "", 4, 1, 0o040755),
                job_row(11, "/etc/", "passwd", 5, 2, 0o100644),
            ],
        )
        .unwrap();
        let etc = child(&tree, "etc");
        assert_eq!(etc["jobid"], 11);
        assert_eq!(child(etc, "hosts")["jobid"], 10);
        assert_eq!(child(etc, "passwd")["jobid"], 11);
    }

    #[test]
    fn deletion_in_newer_job_removes_entry() {
        let tree = build(
            b"/",
            [
                job_row(10, "/data/", "kept", 1, 1, 0o100644),
                job_row(10, "/data/", "removed", 2, 2, 0o100644),
                job_row(10, "/data/old/", "", 3, 3, 0o040755),
                job_row(10, "/data/old/", "x", 4, 4, 0o100644),
                job_row(11, "/data/", "removed", 5, 0, 0o100644),
                job_row(11, "/data/old/", "", 6, 0, 0o040755),
                job_row(11, "/data/old/", "x", 7, 0, 0o100644),
            ],
        )
        .unwrap();
        assert_eq!(names(child(&tree, "data")), ["kept"]);
    }

    #[test]
    fn recreated_after_deletion() {
        let tree = build(
            b"/data/",
            [
                job_row(10, "/data/", "f", 1, 1, 0o100644),
                job_row(11, "/data/", "f", 2, 0, 0o100644),
                job_row(12, "/data/", "f", 3, 7, 0o100644),
            ],
        )
        .unwrap();
        assert_eq!(child(&tree, "f")["jobid"], 12);
    }

    #[test]
    fn whole_chain_deleted_is_none() {
        let rows = [job_row(10, "/data/", "f", 1, 1, 0o100644), job_row(11, "/data/", "f", 2, 0, 0o100644)];
        assert!(build(b"/data/", rows).is_none());
    }

    #[test]
    fn deleted_directory_keeps_live_children() {
        // Only the directory's own row is marked deleted; its content is not
        let tree = build(
            b"/",
            [
                job_row(10, "/d/", "", 1, 1, 0o040755),
                job_row(10, "/d/", "f", 2, 2, 0o100644),
                job_row(11, "/d/", "", 3, 0, 0o040755),
            ],
        )
        .unwrap();
        let d = child(&tree, "d");
        assert!(d.get("jobid").is_none());
        assert_eq!(names(d), ["f"]);
    }

    #[test]
    fn same_job_keeps_last_copy_of_file_and_first_of_hard_link() {
        let tree = build(
            b"/",
            [
                job_row(10, "/", "file", 1, 4, 0o100644),
                job_row(10, "/", "file", 2, 9, 0o100644),
                hard_link(job_row(10, "/", "link", 3, 5, 0o100644), 2),
                hard_link(job_row(10, "/", "link", 4, 8, 0o100644), 2),
            ],
        )
        .unwrap();
        assert_eq!(child(&tree, "file")["fileindex"], 9);
        let link = child(&tree, "link");
        assert_eq!(link["fileindex"], 5);
        assert_eq!((link["jobid"].as_i64(), link["link_fileindex"].as_i64()), (Some(10), Some(2)));
    }

    #[test]
    fn list_shows_direct_children_with_counts() {
        let rows = [
            job_row(10, "/etc/", "", 1, 1, 0o040755),
            job_row(10, "/etc/", "passwd", 2, 2, 0o100644),
            job_row(10, "/etc/ssh/", "", 3, 3, 0o040755),
            job_row(10, "/etc/ssh/", "sshd_config", 4, 4, 0o100600),
            job_row(10, "/etc/ssh/keys/", "host", 5, 5, 0o100600),
            job_row(10, "/etc/gone/", "x", 6, 6, 0o100644),
            job_row(11, "/etc/gone/", "x", 7, 0, 0o100644),
            job_row(11, "/etc/", "passwd", 8, 1, 0o100600),
        ];
        let children = list(b"/etc/", rows.clone());
        let names: Vec<&[u8]> = children.iter().map(|c| c.name.as_slice()).collect();
        assert_eq!(names, [b"passwd".as_slice(), b"ssh"]);
        let passwd = &children[0];
        assert_eq!((passwd.kind, passwd.path.as_deref(), passwd.entries), (lstat::Kind::File, None, None));
        assert_eq!(passwd.version.as_ref().map(|v| (v.jobid, v.fileindex)), Some((11, 1)));
        let ssh = &children[1];
        assert_eq!(ssh.path.as_deref(), Some(b"/etc/ssh/".as_slice()));
        // sshd_config, keys/ (no own row) and keys/host
        assert_eq!(ssh.entries, Some(2));
        assert_eq!(ssh.version.as_ref().map(|v| v.fileid), Some(3));

        let in_ssh = list(b"/etc/ssh/", rows.clone());
        let names: Vec<&[u8]> = in_ssh.iter().map(|c| c.name.as_slice()).collect();
        assert_eq!(names, [b"sshd_config".as_slice(), b"keys"]);
        let keys = &in_ssh[1];
        assert_eq!((keys.name.as_slice(), keys.version.is_none(), keys.entries), (b"keys".as_slice(), true, Some(1)));
        assert!(list(b"/etc/gone/", rows).is_empty());
    }

    #[test]
    fn list_matches_first_level_of_build() {
        let rows = vec![
            job_row(10, "/", "", 1, 1, 0o040755),
            job_row(10, "/a/", "", 2, 2, 0o040755),
            job_row(10, "/a/b/", "f", 3, 3, 0o100644),
            job_row(10, "/", "z", 4, 4, 0o100644),
            job_row(11, "/a/b/", "f", 5, 0, 0o100644),
        ];
        let tree = build(b"/", rows.clone()).unwrap();
        let listed: Vec<String> = list(b"/", rows).iter().map(|c| String::from_utf8(c.name.clone()).unwrap()).collect();
        assert_eq!(names(&tree), listed.iter().map(String::as_str).collect::<Vec<_>>());
        // files first, then directories
        assert_eq!(names(&tree), ["z", "a"]);
    }

    fn delta(mut entry: Entry, seq: i32) -> Entry {
        entry.delta_seq = seq;
        entry
    }

    fn parts(node: &Value) -> Vec<(i64, i64)> {
        node["delta_parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["jobid"].as_i64().unwrap(), p["fileindex"].as_i64().unwrap()))
            .collect()
    }

    #[test]
    fn delta_parts_are_applied_on_their_base() {
        let tree = build(
            b"/vm/",
            [
                delta(job_row(10, "/vm/", "disk.img", 1, 4, 0o100644), 0),
                delta(job_row(11, "/vm/", "disk.img", 2, 7, 0o100644), 1),
                delta(job_row(12, "/vm/", "disk.img", 3, 2, 0o100644), 2),
            ],
        )
        .unwrap();
        let disk = child(&tree, "disk.img");
        assert_eq!((disk["jobid"].as_i64(), disk["fileindex"].as_i64(), disk["delta_seq"].as_i64()), (Some(12), Some(2), Some(2)));
        assert_eq!(parts(disk), [(10, 4), (11, 7)]);
    }

    #[test]
    fn new_base_version_restarts_the_delta_chain() {
        let tree = build(
            b"/vm/",
            [
                delta(job_row(10, "/vm/", "disk.img", 1, 1, 0o100644), 0),
                delta(job_row(11, "/vm/", "disk.img", 2, 1, 0o100644), 1),
                delta(job_row(12, "/vm/", "disk.img", 3, 1, 0o100644), 0),
                delta(job_row(13, "/vm/", "disk.img", 4, 1, 0o100644), 1),
            ],
        )
        .unwrap();
        let disk = child(&tree, "disk.img");
        assert_eq!(disk["jobid"], 13);
        assert_eq!(parts(disk), [(12, 1)]);

        // Only the new base: the old part 1 (job 11) is older and drops out
        let rows = [
            delta(job_row(10, "/vm/", "disk.img", 1, 1, 0o100644), 0),
            delta(job_row(11, "/vm/", "disk.img", 2, 1, 0o100644), 1),
            delta(job_row(12, "/vm/", "disk.img", 3, 1, 0o100644), 0),
        ];
        let disk = build(b"/vm/", rows).unwrap()["children"][0].clone();
        assert_eq!(disk["jobid"], 12);
        assert!(disk.get("delta_parts").is_none());
    }

    #[test]
    fn delta_without_base_or_out_of_sequence() {
        // Part 1 without base version: the file cannot be restored
        assert!(build(b"/vm/", [delta(job_row(11, "/vm/", "a", 1, 1, 0o100644), 1)]).is_none());
        // Part 3 after part 1: skipped, part 1 stays the restored version
        let tree = build(
            b"/vm/",
            [
                delta(job_row(10, "/vm/", "a", 1, 1, 0o100644), 0),
                delta(job_row(11, "/vm/", "a", 2, 1, 0o100644), 1),
                delta(job_row(13, "/vm/", "a", 3, 1, 0o100644), 3),
            ],
        )
        .unwrap();
        assert_eq!(child(&tree, "a")["jobid"], 11);
    }

    #[test]
    fn deleted_after_deltas() {
        let rows = [
            delta(job_row(10, "/vm/", "a", 1, 1, 0o100644), 0),
            delta(job_row(11, "/vm/", "a", 2, 1, 0o100644), 1),
            delta(job_row(12, "/vm/", "a", 3, 0, 0o100644), 0),
        ];
        assert!(build(b"/vm/", rows).is_none());
    }

    #[test]
    fn list_carries_delta_parts() {
        let rows = [
            delta(job_row(10, "/vm/", "a", 1, 5, 0o100644), 0),
            delta(job_row(11, "/vm/", "a", 2, 6, 0o100644), 1),
        ];
        let version = list(b"/vm/", rows)[0].version.clone().unwrap();
        assert_eq!((version.jobid, version.delta_seq), (11, 1));
        assert_eq!(version.delta_parts, [Part { jobid: 10, fileindex: 5 }]);
    }

    #[test]
    fn json_keys_come_before_children() {
        let mut writer = TreeWriter::new(b"/etc/", usize::MAX);
        writer.add(row("/etc/", "", 1, 1, 0o040755));
        writer.add(row("/etc/", "passwd", 2, 2, 0o100644));
        let text = writer.finish().unwrap();
        assert!(text.starts_with(r#"{"name":"/etc/","path":"/etc/","type":"directory","jobid":1,"fileid":1,"#), "{text}");
        assert!(text.ends_with(r#""children":[{"name":"passwd","type":"file","jobid":1,"fileid":2,"fileindex":2,"mode":33188,"uid":0,"gid":0,"size":20,"mtime":1700000000}]}"#), "{text}");
    }

    fn depth_rows() -> Vec<Entry> {
        vec![
            job_row(10, "/", "", 1, 1, 0o040755),
            job_row(10, "/", "vmlinuz", 2, 2, 0o100644),
            job_row(10, "/etc/", "", 3, 3, 0o040755),
            job_row(10, "/etc/", "passwd", 4, 4, 0o100644),
            job_row(10, "/etc/ssh/", "", 5, 5, 0o040755),
            job_row(10, "/etc/ssh/", "sshd_config", 6, 6, 0o100600),
            job_row(10, "/home/user/", "notes", 7, 7, 0o100644),
            job_row(10, "/gone/", "x", 8, 8, 0o100644),
            job_row(11, "/gone/", "x", 9, 0, 0o100644),
        ]
    }

    #[test]
    fn depth_limits_the_tree() {
        let tree = build_to_depth(b"/", 1, depth_rows()).unwrap();
        assert_eq!(names(&tree), ["vmlinuz", "etc", "home"]);
        let etc = child(&tree, "etc");
        assert!(etc.get("children").is_none());
        assert_eq!((etc["entries"].as_i64(), etc["fileid"].as_i64()), (Some(3), Some(3)));
        // home/ has no own row, user/ neither: one entry below
        let home = child(&tree, "home");
        assert_eq!((home["entries"].as_i64(), home.get("fileid")), (Some(1), None));

        let two = build_to_depth(b"/", 2, depth_rows()).unwrap();
        let etc = child(&two, "etc");
        assert_eq!(names(etc), ["passwd", "ssh"]);
        assert_eq!(child(etc, "ssh")["entries"], 1);
        assert!(child(etc, "ssh").get("children").is_none());

        let root_only = build_to_depth(b"/", 0, depth_rows()).unwrap();
        assert_eq!((root_only["entries"].as_i64(), root_only.get("children")), (Some(6), None));
    }

    #[test]
    fn depth_one_matches_list() {
        let tree = build_to_depth(b"/", 1, depth_rows()).unwrap();
        let listed = list(b"/", depth_rows());
        let tree_children = tree["children"].as_array().unwrap();
        assert_eq!(tree_children.len(), listed.len());
        for (node, child) in tree_children.iter().zip(&listed) {
            assert_eq!(node["name"].as_str().unwrap().as_bytes(), child.name.as_slice());
            assert_eq!(node["entries"].as_u64(), child.entries);
        }
    }

    #[test]
    fn full_depth_equals_unlimited() {
        assert_eq!(build_to_depth(b"/", 50, depth_rows()), build(b"/", depth_rows()));
        assert!(build(b"/", depth_rows()).unwrap()["children"][1].get("entries").is_none());
    }
}
