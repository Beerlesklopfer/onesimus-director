//! Reading, creating, changing and deleting configuration resources as raw
//! text (DirectorApi CRUD, docs/plans/CONFIG-CRUD-GRPC.md; Onesimus
//! extension without a bareos-dir counterpart).
//!
//! - The text of a resource is the byte range of its file from the type
//!   word to the closing brace, blanks or a comment after the brace on the
//!   same line and the line end (CRLF stays CRLF; principle 12). The range
//!   is found with the lexer rules of `bareos_config::raw` (`#` comments,
//!   `;`, quoted strings with backslash escapes, nested blocks, several
//!   resources per file); `@include` lines are skipped like there. `sha256`
//!   is the hex SHA-256 of exactly these bytes.
//! - A change is checked before anything is written: the whole changed
//!   configuration (every file, the range replaced or removed, or the new
//!   file added) is parsed in memory and checked like `odir -t`
//!   (config_test/config_check: parser, CheckResources, references,
//!   listen addresses) without the catalog connection.
//! - Writing is atomic (temporary file in the same directory, fsync,
//!   rename; mode and owner of the old file), then the configuration is
//!   reloaded like the `reload` command. A failed reload writes the old
//!   state back and reloads again (RELOAD_FAILED).
//! - One lock serializes read-check-write-reload of all changes.
//!
//! First stage (principle 17): Schedule, Job and JobDefs; then Client and FileSet.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use bareos_config::raw::{ConfResource, Directive};
use sha2::{Digest, Sha256};

use crate::director_config::DirectorConfig;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Resource types this stage edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RType {
    Schedule,
    Job,
    JobDefs,
    Client,
    /// FileSet; deleting one that a Job or JobDefs names is REFERENCED.
    FileSet,
}

impl RType {
    pub const ALL: [RType; 5] = [RType::Schedule, RType::Job, RType::JobDefs, RType::Client, RType::FileSet];

    /// Type word as written in new files and messages.
    pub fn word(self) -> &'static str {
        match self {
            RType::Schedule => "Schedule",
            RType::Job => "Job",
            RType::JobDefs => "JobDefs",
            RType::Client => "Client",
            RType::FileSet => "FileSet",
        }
    }

    /// Normalized type (raw view) and directory below `bareos-dir.d`.
    pub fn norm(self) -> &'static str {
        match self {
            RType::Schedule => "schedule",
            RType::Job => "job",
            RType::JobDefs => "jobdefs",
            RType::Client => "client",
            RType::FileSet => "fileset",
        }
    }

    /// R_ code of the item tables (references).
    fn rcode(self) -> &'static str {
        match self {
            RType::Schedule => "R_SCHEDULE",
            RType::Job => "R_JOB",
            RType::JobDefs => "R_JOBDEFS",
            RType::Client => "R_CLIENT",
            RType::FileSet => "R_FILESET",
        }
    }

    /// Resource ACL like `show` (JobDefs use the Job ACL, Client the Client ACL,
    /// FileSet the FileSet ACL).
    pub fn acl(self) -> crate::acl::Kind {
        match self {
            RType::Schedule => crate::acl::Kind::Schedule,
            RType::Job | RType::JobDefs => crate::acl::Kind::Job,
            RType::Client => crate::acl::Kind::Client,
            RType::FileSet => crate::acl::Kind::FileSet,
        }
    }
}

/// Result status (ConfigStatus of the proto; DENIED is decided by the caller's ACL closure).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Ok,
    Conflict,
    Invalid,
    Referenced,
    Denied,
    NotFound,
    Exists,
    ReloadFailed,
}

/// A rejected configuration: where and the text `odir -t` prints.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Problem {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub text: String,
}

/// Answer of one operation (ResourceResponse of the proto).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    pub status: Status,
    pub message: String,
    pub name: String,
    pub file: String,
    pub text: String,
    pub sha256: String,
    pub shared_file: bool,
    /// (normalized resource type, name)
    pub referenced_by: Vec<(String, String)>,
    pub problem: Option<Problem>,
    pub reloaded: bool,
    pub validated_only: bool,
    pub old_sha256: String,
    pub followups: Vec<String>,
}

impl Outcome {
    fn new(status: Status, name: &str, message: String) -> Self {
        Outcome { status, name: name.to_string(), message, ..Default::default() }
    }
}

/// One line of ListResources.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub name: String,
    pub file: String,
    pub sha256: String,
    pub shared_file: bool,
}

/// Hex SHA-256 of `text`.
pub fn sha256_hex(text: &str) -> String {
    hex::encode(Sha256::digest(text.as_bytes()))
}

// ---------------------------------------------------------------------------
// Resource ranges in a file
// ---------------------------------------------------------------------------

/// A lexer error at a byte offset.
#[derive(Debug, Clone, PartialEq)]
pub struct SpanError {
    pub offset: usize,
    pub message: String,
}

fn skip_comment(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// Index after the quoted string starting at `i` (backslash escapes).
fn skip_string(b: &[u8], start: usize) -> Result<usize, SpanError> {
    let mut i = start + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    Err(SpanError { offset: start, message: "unterminated quoted string".into() })
}

/// Byte ranges `(start, end)` of the resources of a configuration file, in
/// file order: from the type word to the closing brace, blanks or a comment
/// after it on the same line and the line end (`\n` or `\r\n`). Lexer rules
/// of `bareos_config::raw`; `@include` lines are skipped there too.
pub fn resource_spans(text: &str) -> Result<Vec<(usize, usize)>, SpanError> {
    let b = text.as_bytes();
    let n = b.len();
    let mut i = 0;
    let mut out = Vec::new();
    loop {
        while i < n {
            match b[i] {
                b'#' => i = skip_comment(b, i),
                b';' => i += 1,
                c if c.is_ascii_whitespace() => i += 1,
                _ => break,
            }
        }
        if i >= n {
            return Ok(out);
        }
        let start = i;
        if b[i] == b'@' {
            i = skip_comment(b, i);
            continue;
        }
        let mut depth = 0usize;
        loop {
            if i >= n {
                return Err(SpanError { offset: start, message: "unexpected end of file inside a resource".into() });
            }
            match b[i] {
                b'"' if depth > 0 => i = skip_string(b, i)?,
                b'#' => i = skip_comment(b, i),
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    if depth == 0 {
                        return Err(SpanError { offset: i, message: "unexpected '}'".into() });
                    }
                    depth -= 1;
                    i += 1;
                    if depth == 0 {
                        break;
                    }
                }
                b'"' | b'=' | b',' | b';' if depth == 0 => {
                    return Err(SpanError { offset: i, message: "expected '{' after the resource type".into() });
                }
                _ => i += 1,
            }
        }
        // blanks and a comment after the brace, then the line end
        let mut j = i;
        while j < n && (b[j] == b' ' || b[j] == b'\t') {
            j += 1;
        }
        if j < n && b[j] == b'#' {
            j = skip_comment(b, j);
        }
        let end = if b[j..].starts_with(b"\r\n") {
            j + 2
        } else if j < n && b[j] == b'\n' {
            j + 1
        } else if j >= n {
            j
        } else {
            i
        };
        out.push((start, end));
        i = end;
    }
}

/// 1-based line and column (characters) of byte `offset` in `text`.
fn line_col(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line as u32, col as u32)
}

/// Line end used by a file ("\r\n" when its first line end is one).
fn line_end_of(text: &str) -> &'static str {
    match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => "\r\n",
        _ => "\n",
    }
}

/// The value of a Schedule `Run` directive as one line for
/// [`bareos_config::parser::check_run`] (tokens separated by blanks, `=`
/// and `,` as written).
fn run_value(d: &Directive) -> String {
    d.tokens.iter().filter_map(|t| match t {
        bareos_config::raw::Tok::Word(s) | bareos_config::raw::Tok::Quoted(s) => Some(s.clone()),
        bareos_config::raw::Tok::Eq => Some("=".into()),
        bareos_config::raw::Tok::Comma => Some(",".into()),
        bareos_config::raw::Tok::Block(_) => None,
    }).collect::<Vec<_>>().join(" ")
}

/// Text without anything but blanks, line ends, `;` and comments.
fn only_comments(text: &str) -> bool {
    text.lines().all(|l| {
        let t = l.trim();
        t.is_empty() || t.starts_with('#') || t.chars().all(|c| c == ';' || c.is_whitespace())
    })
}

// ---------------------------------------------------------------------------
// The configuration files
// ---------------------------------------------------------------------------

/// All configuration files with their contents, in glob order (read in
/// reverse, like `bareos_config::raw::load`).
#[derive(Debug, Clone)]
struct Tree {
    /// `bareos-dir.d` (None: a single configuration file)
    include_dir: Option<PathBuf>,
    files: Vec<(PathBuf, String)>,
}

/// A resource found in the tree.
#[derive(Debug, Clone)]
struct Found {
    file: usize,
    start: usize,
    end: usize,
    shared: bool,
}

impl Tree {
    fn load(path: &Path) -> Result<Tree, String> {
        let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()));
        if path.is_file() {
            return Ok(Tree { include_dir: None, files: vec![(path.to_path_buf(), read(path)?)] });
        }
        let dir_d = path.join("bareos-dir.d");
        let base = if dir_d.is_dir() { dir_d } else { path.to_path_buf() };
        let mut paths = Vec::new();
        for sub in std::fs::read_dir(&base).map_err(|e| format!("readdir {}: {e}", base.display()))? {
            let sub = sub.map_err(|e| e.to_string())?.path();
            if !sub.is_dir() {
                continue;
            }
            for f in std::fs::read_dir(&sub).map_err(|e| format!("readdir {}: {e}", sub.display()))? {
                let f = f.map_err(|e| e.to_string())?.path();
                if f.extension().is_some_and(|e| e == "conf") && f.is_file() {
                    paths.push(f);
                }
            }
        }
        bareos_config::raw::sort_like_glob(&mut paths);
        let mut files = Vec::with_capacity(paths.len());
        for p in paths {
            let text = read(&p)?;
            files.push((p, text));
        }
        Ok(Tree { include_dir: Some(base), files })
    }

    /// Files in the order the parser reads them.
    fn read_order(&self) -> Vec<(PathBuf, String)> {
        self.files.iter().rev().cloned().collect()
    }

    fn content(&self, path: &Path) -> Option<String> {
        self.files.iter().find(|(p, _)| p == path).map(|(_, t)| t.clone())
    }

    /// The resources of type `rtype` in read order: (name, found).
    fn resources(&self, rtype: RType) -> Vec<(String, Found)> {
        let mut out = Vec::new();
        for index in (0..self.files.len()).rev() {
            let text = &self.files[index].1;
            let Ok(spans) = resource_spans(text) else { continue };
            let shared = spans.len() > 1;
            for (start, end) in spans {
                let Ok(parsed) = bareos_config::raw::parse_str(&text[start..end], None) else { continue };
                let Some(res) = parsed.first() else { continue };
                if res.norm_type() != rtype.norm() {
                    continue;
                }
                if let Some(name) = res.name() {
                    out.push((name.to_string(), Found { file: index, start, end, shared }));
                }
            }
        }
        out
    }

    fn find(&self, rtype: RType, name: &str) -> Option<Found> {
        self.resources(rtype).into_iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }

    fn text(&self, found: &Found) -> &str {
        &self.files[found.file].1[found.start..found.end]
    }

    fn path(&self, found: &Found) -> &Path {
        &self.files[found.file].0
    }

    fn insert(&mut self, path: PathBuf, text: String) {
        self.files.push((path, text));
        let mut paths: Vec<PathBuf> = self.files.iter().map(|(p, _)| p.clone()).collect();
        bareos_config::raw::sort_like_glob(&mut paths);
        let mut files = std::mem::take(&mut self.files);
        self.files = paths.into_iter().map(|p| {
            let i = files.iter().position(|(q, _)| *q == p).unwrap();
            files.swap_remove(i)
        }).collect();
    }
}

// ---------------------------------------------------------------------------
// Checking a changed configuration like `odir -t` (without the catalog)
// ---------------------------------------------------------------------------

/// Problem of a parser error "<file>: line N: message".
fn parse_problem(tree: &Tree, message: &str, text: String) -> Problem {
    let mut problem = Problem { text, ..Default::default() };
    let rest = message.strip_prefix("parse error: ").unwrap_or(message);
    for (path, _) in &tree.files {
        if let Some(after) = rest.strip_prefix(&format!("{}: ", path.display())) {
            problem.file = path.display().to_string();
            if let Some(line) = after.strip_prefix("line ").and_then(|l| l.split(':').next()).and_then(|n| n.parse().ok()) {
                problem.line = line;
            }
            break;
        }
    }
    problem
}

/// Checks the configuration of `tree` like `odir -t` without the catalog
/// connection (config_test::run_with: parser, CheckResources, listen
/// addresses). The configuration on success.
fn check(tree: &Tree, base: &str) -> Result<DirectorConfig, Problem> {
    let files = tree.read_order();
    let config = DirectorConfig::from_files(&files).map_err(|e| {
        let message = e.to_string();
        parse_problem(tree, &message, crate::config_test::config_error("lib/parse_conf.cc", &message, None))
    })?;
    if let Err(diags) = crate::config_check::check(&config.raw, base) {
        let read = |p: &Path| tree.content(p);
        let (text, _, position) = crate::config_test::report_in(&config.raw, &diags, base, &read);
        let mut problem = Problem { text, ..Default::default() };
        if let Some(p) = position {
            problem.file = p.file;
            problem.line = p.line as u32;
            problem.column = p.col as u32;
        } else if let Some(crate::config_check::Diag::Config(error)) = diags.first()
            && let Some(source) = config.raw.get(error.resource).and_then(|r| r.source.as_ref()) {
            problem.file = source.display().to_string();
        }
        return Err(problem);
    }
    if let Err(e) = crate::listen::listen_addresses(&config.raw, None, config.director.port) {
        return Err(Problem { text: crate::config_test::config_error("lib/parse_conf.cc", &e, None), ..Default::default() });
    }
    Ok(config)
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

/// `job=<name>` of a console `run` command or a Job `Run` value, when the
/// name is literal (no variables); otherwise None (not decidable).
fn run_job_name(command: &str) -> Option<String> {
    run_arg(command, "job=")
}

/// The literal value of `<key>` (e.g. `client=`) in a console command line;
/// None when missing or when it contains variables.
fn run_arg(command: &str, key: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    let mut from = 0;
    let at = loop {
        let i = from + lower[from..].find(key)?;
        if i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace() {
            break i;
        }
        from = i + key.len();
    };
    let value = &command[at + key.len()..];
    let name = if let Some(quoted) = value.strip_prefix('"') {
        quoted.split('"').next()?
    } else {
        value.split_whitespace().next()?
    };
    (!name.is_empty() && !name.contains(['%', '$'])).then(|| name.to_string())
}

/// Every resource that names resource `name` of type `rtype`: directives
/// whose item refers to that type (Schedule =, JobDefs =, Job To Verify =,
/// Client = …, from the item tables of all resource types), and for Jobs
/// (Clients, FileSets) `run job=<name>` (`client=<name>`, `fileset=<name>`) in RunScript `Console =`
/// commands and Job `Run =` values when the name is literal. Console/Profile ACL entries are patterns,
/// not references.
pub fn references(raw: &[ConfResource], rtype: RType, name: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for res in raw {
        let Some(res_name) = res.name() else { continue };
        if res.norm_type() == rtype.norm() && res_name == name {
            continue;
        }
        let items = crate::config_check::items_of(&res.rtype);
        let names_it = |d: &Directive| -> bool {
            if let Some(item) = items.and_then(|items| crate::config_check::item_of(items, &d.key))
                && item.code == Some(rtype.rcode()) && d.values().contains(&name) {
                return true;
            }
            // `run job=<name>` / `run ... client=<name>` in Job `Run` values
            // and RunScript console commands
            let key = match rtype {
                RType::Job => Some("job="),
                RType::Client => Some("client="),
                RType::FileSet => Some("fileset="),
                _ => None,
            };
            if let Some(key) = key && matches!(res.norm_type().as_str(), "job" | "jobdefs") {
                if d.norm_key() == "run" && d.values().iter().any(|v| run_arg(v, key).as_deref() == Some(name)) {
                    return true;
                }
                if d.norm_key() == "runscript" {
                    return d.block.iter().flatten().filter(|c| c.norm_key() == "console").flat_map(|c| c.values())
                        .any(|v| v.trim_start().to_ascii_lowercase().starts_with("run ")
                            && run_arg(v, key).as_deref() == Some(name));
                }
            }
            false
        };
        if res.directives.iter().any(names_it) {
            let entry = (res.norm_type(), res_name.to_string());
            if !out.contains(&entry) {
                out.push(entry);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Mode and owner for a file.
#[derive(Debug, Clone, Copy)]
struct Owner {
    mode: u32,
    uid: u32,
    gid: u32,
}

#[cfg(unix)]
fn owner_of(path: &Path) -> Option<Owner> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some(Owner { mode: m.mode() & 0o7777, uid: m.uid(), gid: m.gid() })
}

/// Mode and owner of a new file in `dir`: like the other `.conf` files
/// there (or of the type directories next to it), else 0640 and the owner
/// of the directory.
#[cfg(unix)]
fn owner_for_new(dir: &Path) -> Option<Owner> {
    let sibling = |d: &Path| -> Option<PathBuf> {
        let mut confs: Vec<PathBuf> = std::fs::read_dir(d).ok()?.filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "conf") && p.is_file()).collect();
        confs.sort();
        confs.into_iter().next()
    };
    if let Some(owner) = sibling(dir).and_then(|p| owner_of(&p)) {
        return Some(owner);
    }
    if let Some(parent) = dir.parent() {
        let mut subs: Vec<PathBuf> = std::fs::read_dir(parent).ok()?.filter_map(|e| e.ok()).map(|e| e.path())
            .filter(|p| p.is_dir()).collect();
        subs.sort();
        if let Some(owner) = subs.iter().find_map(|s| sibling(s)).and_then(|p| owner_of(&p)) {
            return Some(owner);
        }
    }
    owner_of(dir).map(|o| Owner { mode: 0o640, ..o })
}

static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Writes `content` to `path` atomically: a temporary file in the same
/// directory (not `*.conf`, so never read as configuration), mode and owner
/// `owner`, fsync, rename, fsync of the directory.
fn atomic_write(path: &Path, content: &str, owner: Option<Owner>) -> Result<(), String> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| format!("{}: no directory", path.display()))?;
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.odir-{}-{}.tmp", std::process::id(),
        TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(content.as_bytes())?;
        #[cfg(unix)]
        if let Some(o) = owner {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(o.mode))?;
            if let Err(e) = std::os::unix::fs::fchown(&f, Some(o.uid), Some(o.gid)) {
                // only root may give a file away; the mode is kept anyway
                let current = f.metadata().map(|m| { use std::os::unix::fs::MetadataExt; (m.uid(), m.gid()) });
                if current.ok() != Some((o.uid, o.gid)) {
                    tracing::warn!("config edit: cannot set owner {}:{} of {}: {e}", o.uid, o.gid, path.display());
                }
            }
        }
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("write {}: {e}", path.display()));
    }
    Ok(())
}

fn remove_file(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display()))?;
    if let Some(dir) = path.parent()
        && let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// One file change and how to undo it.
#[derive(Debug, Clone)]
enum Change {
    /// New content; `old` None: the file is new
    Write { path: PathBuf, content: String, old: Option<String> },
    /// The file is removed
    Remove { path: PathBuf, old: String },
}

impl Change {
    fn path(&self) -> &Path {
        match self {
            Change::Write { path, .. } | Change::Remove { path, .. } => path,
        }
    }

    fn apply(&self, owner: Option<Owner>) -> Result<(), String> {
        match self {
            Change::Write { path, content, .. } => atomic_write(path, content, owner),
            Change::Remove { path, .. } => remove_file(path),
        }
    }

    fn revert(&self, owner: Option<Owner>) -> Result<(), String> {
        match self {
            Change::Write { path, old: None, .. } => remove_file(path),
            Change::Write { path, old: Some(old), .. } | Change::Remove { path, old } => atomic_write(path, old, owner),
        }
    }
}

// ---------------------------------------------------------------------------
// The editor
// ---------------------------------------------------------------------------

/// Reloads the configuration (production: `reload::reload_now`, the path of
/// the `reload` command).
pub type ReloadFn = Box<dyn Fn() -> Result<(), String> + Send + Sync>;
/// Sends a Director message (production: the Director's Messages resource).
pub type NotifyFn = Box<dyn Fn(&str) + Send + Sync>;

/// Which operation (log, messages).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Create,
    Update,
    Delete,
}

impl Op {
    fn word(self) -> &'static str {
        match self {
            Op::Create => "create",
            Op::Update => "update",
            Op::Delete => "delete",
        }
    }
}

pub struct ConfigEditor {
    /// The `-c` path (directory with `bareos-dir.d`, `bareos-dir.d` itself or a file)
    path: PathBuf,
    reload: ReloadFn,
    notify: NotifyFn,
    /// Read-check-write-reload of one change at a time
    lock: Mutex<()>,
}

/// Name usable as a file name `<name>.conf` and as a Bareos resource name.
pub fn name_error(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("the name is empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.starts_with('.')
        || name.chars().any(char::is_control) {
        return Some(format!("the name \"{}\" is not allowed (/, \\, .., leading dot, control characters)", name.escape_default()));
    }
    if !crate::acl::valid_name(name) {
        return Some(format!("the name \"{name}\" is not a valid resource name"));
    }
    None
}

/// The production editor shared by the gRPC RPCs and the `.config` console
/// command (one lock for both paths); None without a configuration path.
pub fn shared_editor(config: &crate::director_config::SharedConfig) -> Option<std::sync::Arc<ConfigEditor>> {
    static SHARED: std::sync::OnceLock<std::sync::Arc<ConfigEditor>> = std::sync::OnceLock::new();
    if let Some(editor) = SHARED.get() {
        return Some(editor.clone());
    }
    let path = crate::reload::config_path()?;
    Some(SHARED.get_or_init(|| std::sync::Arc::new(ConfigEditor::production(path, config.clone()))).clone())
}

/// The ACL decision of the caller for a resource name.
pub type Allow<'a> = &'a dyn Fn(&str) -> bool;

impl ConfigEditor {
    pub fn new(path: PathBuf, reload: ReloadFn, notify: NotifyFn) -> Self {
        Self { path, reload, notify, lock: Mutex::new(()) }
    }

    /// Production editor: the configuration path of the Director, the
    /// reload of the `reload` command and Director messages through the
    /// Director's Messages resource of the active configuration.
    pub fn production(path: PathBuf, config: crate::director_config::SharedConfig) -> Self {
        Self::new(path, Box::new(crate::reload::reload_now), Box::new(move |text: &str| {
            if let Ok(cfg) = config.read() {
                crate::audit::daemon_message(&cfg, crate::message_bus::MsgType::Info, text);
            }
        }))
    }

    fn base(&self) -> String {
        crate::config_check::base_config_path(&self.path)
    }

    fn tree(&self) -> Result<Tree, String> {
        Tree::load(&self.path)
    }

    fn found_outcome(tree: &Tree, found: &Found, name: &str, status: Status, message: String) -> Outcome {
        let text = tree.text(found).to_string();
        Outcome {
            status,
            message,
            name: name.to_string(),
            file: tree.path(found).display().to_string(),
            sha256: sha256_hex(&text),
            text,
            shared_file: found.shared,
            ..Default::default()
        }
    }

    /// ReadResource.
    pub fn read(&self, rtype: RType, name: &str, allow: Allow) -> Result<Outcome, String> {
        if !allow(name) {
            return Ok(denied(rtype, name));
        }
        let tree = self.tree()?;
        Ok(match tree.find(rtype, name) {
            Some(found) => Self::found_outcome(&tree, &found, name, Status::Ok, String::new()),
            None => not_found(rtype, name),
        })
    }

    /// ListResources: the allowed resources of `rtype` in resource order.
    pub fn list(&self, rtype: RType, allow: Allow) -> Result<Vec<Summary>, String> {
        let tree = self.tree()?;
        Ok(tree.resources(rtype).into_iter().filter(|(name, _)| allow(name)).map(|(name, found)| Summary {
            sha256: sha256_hex(tree.text(&found)),
            file: tree.path(&found).display().to_string(),
            shared_file: found.shared,
            name,
        }).collect())
    }

    /// The submitted text of one resource of `rtype`: its name and the text
    /// to write (a missing final line end is added in the file's style).
    #[allow(clippy::result_large_err)]
    fn submitted(rtype: RType, text: &str, file: &str, line_end: &str) -> Result<(String, String), Outcome> {
        let invalid = |message: String, problem: Option<Problem>| {
            let mut o = Outcome::new(Status::Invalid, "", message);
            o.problem = problem;
            o
        };
        let mut text = text.to_string();
        if !text.ends_with('\n') {
            text.push_str(line_end);
        }
        let spans = resource_spans(&text).map_err(|e| {
            let (line, column) = line_col(&text, e.offset);
            let message = format!("text: {} (line {line}, column {column})", e.message);
            invalid(message.clone(), Some(Problem { file: file.to_string(), line, column, text: message }))
        })?;
        if spans.len() != 1 || spans[0] != (0, text.len()) {
            return Err(invalid(format!("text must be exactly one {} resource, from the type word to the closing \
                brace and its line end", rtype.word()), None));
        }
        let parsed = bareos_config::raw::parse_str(&text, None)
            .map_err(|e| invalid(format!("text: {e}"), None))?;
        let res = &parsed[0];
        if res.norm_type() != rtype.norm() {
            return Err(invalid(format!("text is a {} resource, not {}", res.rtype, rtype.word()), None));
        }
        let name = res.name().ok_or_else(|| invalid(format!("{} resource without Name", rtype.word()), None))?;
        if rtype == RType::Schedule {
            for d in res.directives.iter().filter(|d| d.norm_key() == "run") {
                let value = run_value(d);
                if let Err(e) = bareos_config::parser::check_run(&value) {
                    let message = format!("Run = {value}: {e}");
                    return Err(invalid(message.clone(), Some(Problem { file: file.to_string(), text: message, ..Default::default() })));
                }
            }
        }
        Ok((name.to_string(), text))
    }

    /// CreateResource: `bareos-dir.d/<type>/<Name>.conf`.
    pub fn create(&self, rtype: RType, text: &str, validate_only: bool, console: &str, allow: Allow)
        -> Result<Outcome, String> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut tree = self.tree()?;
        let Some(include_dir) = tree.include_dir.clone() else {
            return Ok(Outcome::new(Status::Invalid, "", "the configuration is a single file; CreateResource needs \
                the bareos-dir.d directory layout".into()));
        };
        let directory = include_dir.join(rtype.norm());
        let (name, text) = match Self::submitted(rtype, text, "", "\n") {
            Ok(v) => v,
            Err(o) => return Ok(o),
        };
        if let Some(e) = name_error(&name) {
            return Ok(Outcome::new(Status::Invalid, &name, e));
        }
        if !allow(&name) {
            return Ok(denied(rtype, &name));
        }
        let path = directory.join(format!("{name}.conf"));
        if let Some(found) = tree.find(rtype, &name) {
            return Ok(Self::found_outcome(&tree, &found, &name, Status::Exists,
                format!("{} \"{name}\" already exists", rtype.word())));
        }
        if path.exists() {
            let mut o = Outcome::new(Status::Exists, &name, format!("file {} already exists", path.display()));
            o.file = path.display().to_string();
            return Ok(o);
        }
        tree.insert(path.clone(), text.clone());
        if let Err(problem) = check(&tree, &self.base()) {
            return Ok(invalid_outcome(&name, problem));
        }
        let mut outcome = Outcome {
            name: name.clone(),
            file: path.display().to_string(),
            sha256: sha256_hex(&text),
            text: text.clone(),
            ..Default::default()
        };
        if validate_only {
            outcome.validated_only = true;
            outcome.message = format!("{} \"{name}\" can be created", rtype.word());
            return Ok(outcome);
        }
        if !directory.is_dir() {
            std::fs::create_dir_all(&directory).map_err(|e| format!("mkdir {}: {e}", directory.display()))?;
        }
        let change = Change::Write { path: path.clone(), content: text, old: None };
        #[cfg(unix)]
        let owner = owner_for_new(&directory);
        #[cfg(not(unix))]
        let owner = None;
        self.commit(Op::Create, rtype, console, change, owner, outcome)
    }

    /// UpdateResource.
    #[allow(clippy::too_many_arguments)]
    pub fn update(&self, rtype: RType, name: &str, expected_sha256: &str, text: &str, validate_only: bool,
                  console: &str, allow: Allow) -> Result<Outcome, String> {
        if !allow(name) {
            return Ok(denied(rtype, name));
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut tree = self.tree()?;
        let Some(found) = tree.find(rtype, name) else { return Ok(not_found(rtype, name)) };
        let current = tree.text(&found).to_string();
        let current_sha = sha256_hex(&current);
        if !expected_sha256.eq_ignore_ascii_case(&current_sha) {
            return Ok(Self::found_outcome(&tree, &found, name, Status::Conflict,
                format!("{} \"{name}\" changed since it was read", rtype.word())));
        }
        let path = tree.path(&found).to_path_buf();
        let file_text = tree.files[found.file].1.clone();
        let (new_name, text) = match Self::submitted(rtype, text, &path.display().to_string(), line_end_of(&file_text)) {
            Ok(v) => v,
            Err(mut o) => {
                o.name = name.to_string();
                return Ok(o);
            }
        };
        if new_name != name {
            return Ok(Outcome::new(Status::Invalid, name, format!("the Name in the text (\"{new_name}\") must be \
                \"{name}\"; renaming is not supported")));
        }
        let content = format!("{}{}{}", &file_text[..found.start], text, &file_text[found.end..]);
        tree.files[found.file].1 = content.clone();
        if let Err(problem) = check(&tree, &self.base()) {
            return Ok(invalid_outcome(name, problem));
        }
        let shared = resource_spans(&content).map(|s| s.len() > 1).unwrap_or(found.shared);
        let outcome = Outcome {
            name: name.to_string(),
            file: path.display().to_string(),
            sha256: sha256_hex(&text),
            text,
            shared_file: shared,
            old_sha256: current_sha,
            ..Default::default()
        };
        if validate_only {
            return Ok(Outcome { validated_only: true,
                message: format!("{} \"{name}\" can be changed", rtype.word()), ..outcome });
        }
        #[cfg(unix)]
        let owner = owner_of(&path);
        #[cfg(not(unix))]
        let owner = None;
        let change = Change::Write { path, content, old: Some(file_text) };
        self.commit(Op::Update, rtype, console, change, owner, outcome)
    }

    /// DeleteResource. In a shared file only the range of the resource is
    /// removed; a file left with nothing but blanks and comments is removed
    /// (a comment-only `<Name>.conf` would block a later CreateResource of
    /// the same name and keeps describing a resource that is gone).
    pub fn delete(&self, rtype: RType, name: &str, expected_sha256: &str, validate_only: bool, console: &str,
                  allow: Allow) -> Result<Outcome, String> {
        if !allow(name) {
            return Ok(denied(rtype, name));
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut tree = self.tree()?;
        let Some(found) = tree.find(rtype, name) else { return Ok(not_found(rtype, name)) };
        let current = tree.text(&found).to_string();
        let current_sha = sha256_hex(&current);
        if !expected_sha256.eq_ignore_ascii_case(&current_sha) {
            return Ok(Self::found_outcome(&tree, &found, name, Status::Conflict,
                format!("{} \"{name}\" changed since it was read", rtype.word())));
        }
        // references in the current configuration
        let raw: Vec<ConfResource> = tree.read_order().iter()
            .flat_map(|(p, t)| bareos_config::raw::parse_str(t, Some(p)).unwrap_or_default()).collect();
        let referenced_by = references(&raw, rtype, name);
        if !referenced_by.is_empty() {
            let mut o = Self::found_outcome(&tree, &found, name, Status::Referenced,
                format!("{} \"{name}\" is used by {} resource{}", rtype.word(), referenced_by.len(),
                    if referenced_by.len() == 1 { "" } else { "s" }));
            o.referenced_by = referenced_by;
            return Ok(o);
        }
        let path = tree.path(&found).to_path_buf();
        let file_text = tree.files[found.file].1.clone();
        let rest = format!("{}{}", &file_text[..found.start], &file_text[found.end..]);
        let change = if only_comments(&rest) {
            tree.files.remove(found.file);
            Change::Remove { path: path.clone(), old: file_text }
        } else {
            tree.files[found.file].1 = rest.clone();
            Change::Write { path: path.clone(), content: rest, old: Some(file_text) }
        };
        if let Err(problem) = check(&tree, &self.base()) {
            return Ok(invalid_outcome(name, problem));
        }
        let outcome = Outcome {
            name: name.to_string(),
            file: path.display().to_string(),
            shared_file: found.shared,
            old_sha256: current_sha,
            ..Default::default()
        };
        if validate_only {
            return Ok(Outcome { validated_only: true,
                message: format!("{} \"{name}\" can be deleted", rtype.word()), ..outcome });
        }
        #[cfg(unix)]
        let owner = owner_of(&path);
        #[cfg(not(unix))]
        let owner = None;
        self.commit(Op::Delete, rtype, console, change, owner, outcome)
    }

    /// Writes the change, reloads, logs; on a failed reload the old state is
    /// written back and loaded again.
    fn commit(&self, op: Op, rtype: RType, console: &str, change: Change, owner: Option<Owner>, mut outcome: Outcome)
        -> Result<Outcome, String> {
        change.apply(owner)?;
        let file = change.path().display().to_string();
        let audit = |result: &str| format!("Console [{console}] config {} {} \"{}\": old sha256 {}, new sha256 {}, file {file}{result}",
            op.word(), rtype.word(), outcome.name, empty_dash(&outcome.old_sha256), empty_dash(&outcome.sha256));
        match (self.reload)() {
            Ok(()) => {
                let line = audit("");
                tracing::info!("{line}");
                (self.notify)(&format!("{line}\n"));
                outcome.reloaded = true;
                outcome.message = format!("{} \"{}\" {}", rtype.word(), outcome.name, match op {
                    Op::Create => "created",
                    Op::Update => "changed",
                    Op::Delete => "deleted",
                });
                // Principle 15: what the reload does not bring into line (Pool
                // resources: volumes keep their values until `update volume …
                // frompool`) goes into `followups`; Schedule/Job/JobDefs/Client/FileSet
                // have nothing of that kind (the catalog Client record takes
                // AutoPrune and the retentions at the next job, as in Bareos), so the list stays empty in this stage.
                outcome.followups = Vec::new();
                Ok(outcome)
            }
            Err(reason) => {
                let restored = change.revert(owner);
                let reloaded = (self.reload)();
                let line = audit(&format!(" - reload failed ({reason}), previous state {}",
                    match (&restored, &reloaded) {
                        (Ok(()), Ok(())) => "restored and reloaded".to_string(),
                        (Ok(()), Err(e)) => format!("restored, reload failed again: {e}"),
                        (Err(e), _) => format!("NOT restored: {e}"),
                    }));
                tracing::warn!("{line}");
                (self.notify)(&format!("{line}\n"));
                let mut o = Outcome::new(Status::ReloadFailed, &outcome.name,
                    format!("reload failed: {reason}; previous state restored"));
                o.file = file;
                o.old_sha256 = outcome.old_sha256.clone();
                // current (restored) state
                if let Ok(tree) = self.tree()
                    && let Some(found) = tree.find(rtype, &outcome.name) {
                    let current = Self::found_outcome(&tree, &found, &outcome.name, Status::ReloadFailed, String::new());
                    o.text = current.text;
                    o.sha256 = current.sha256;
                    o.shared_file = current.shared_file;
                }
                if let Err(e) = restored {
                    o.message = format!("reload failed: {reason}; restoring the previous state failed: {e}");
                }
                Ok(o)
            }
        }
    }
}

fn empty_dash(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}

fn denied(rtype: RType, name: &str) -> Outcome {
    Outcome::new(Status::Denied, name, format!("{} \"{name}\" not allowed by the console's ACL", rtype.word()))
}

fn not_found(rtype: RType, name: &str) -> Outcome {
    Outcome::new(Status::NotFound, name, format!("{} \"{name}\" not found", rtype.word()))
}

fn invalid_outcome(name: &str, problem: Problem) -> Outcome {
    let first = problem.text.lines().find(|l| l.starts_with("Config error: ")).map(|l| l.trim_start_matches("Config error: "))
        .or_else(|| problem.text.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("invalid configuration").to_string();
    let mut o = Outcome::new(Status::Invalid, name, format!("configuration invalid: {first}"));
    o.problem = Some(problem);
    o
}

#[cfg(test)]
pub(crate) mod tests;
