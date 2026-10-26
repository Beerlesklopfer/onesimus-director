//! OVFS — Onesimus restore trees from the Bareos catalog.
//!
//! The PostgreSQL extension `ovfs` (repository `oenesimus-postgres-plugin`,
//! commit `4041335`, documentation in its GitHub wiki) resolves the directory
//! tree of a job or job chain as JSON (`ovfs.tree`), lists one directory
//! (`ovfs.ls`), finds the latest job chain of a client (`ovfs.jobids_latest`)
//! and the versions of one file (`ovfs.versions`).
//!
//! The Director checks at connect time whether the extension is usable
//! ([`capability`]). If it is, the functions below call it; otherwise the
//! Director computes the same result itself ([`fallback`]): the same catalog
//! rows, read through a server-side cursor in batches, resolved by a port of
//! the extension's core ([`tree`], [`lstat`]). Both paths return the same
//! values; `tests::equivalence` compares them against a catalog with the
//! extension installed.
//!
//! BVFS (`commands/bvfs.rs`) is not affected: it stays the byte-identical
//! Bareos interface for Bareos clients.

pub mod capability;
pub mod fallback;
pub mod lstat;
pub mod native;
pub mod tree;

#[cfg(test)]
mod tests;

pub use capability::Capability;

use crate::catalog_worker::{CatalogConnection, SqlTable};

/// The statements OVFS needs from one reserved catalog connection.
pub trait Catalog {
    /// Rows with text values (simple query protocol).
    fn rows(&self, sql: &str) -> std::result::Result<SqlTable, String>;
    /// Statement(s) without result.
    fn exec(&self, sql: &str) -> std::result::Result<u64, String>;
}

impl Catalog for CatalogConnection {
    fn rows(&self, sql: &str) -> std::result::Result<SqlTable, String> {
        self.query_rows(sql)
    }
    fn exec(&self, sql: &str) -> std::result::Result<u64, String> {
        self.execute(sql)
    }
}

/// An error of an OVFS call, worded like the extension's error messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OvfsError(pub String);

impl std::fmt::Display for OvfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for OvfsError {
    fn from(message: String) -> Self {
        OvfsError(message)
    }
}

pub type Result<T> = std::result::Result<T, OvfsError>;

/// One delta part (or the base version) of a delta file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub jobid: i32,
    pub fileindex: i32,
}

/// One row of `ovfs.ls`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LsRow {
    pub name: String,
    pub name_b64: Option<String>,
    pub path: Option<String>,
    pub path_b64: Option<String>,
    pub kind: String,
    pub jobid: Option<i32>,
    pub fileid: Option<i64>,
    pub fileindex: Option<i32>,
    pub delta_seq: Option<i32>,
    pub delta_parts: Option<Vec<Part>>,
    pub mode: Option<i64>,
    pub uid: Option<i64>,
    pub gid: Option<i64>,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub link_fileindex: Option<i64>,
    pub entries: Option<i64>,
}

/// One row of `ovfs.versions`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionRow {
    pub jobid: i32,
    pub level: String,
    pub job_type: String,
    pub jobtdate: i64,
    pub starttime: Option<String>,
    pub fileid: i64,
    pub fileindex: i32,
    pub delta_seq: i32,
    pub md5: String,
    pub mode: Option<i64>,
    pub uid: Option<i64>,
    pub gid: Option<i64>,
    pub size: Option<i64>,
    pub mtime: Option<i64>,
    pub volumes: Vec<String>,
    pub inchanger: bool,
}

/// The job chain of a client's latest state (`ovfs.jobids_latest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    pub jobids: Vec<i32>,
    /// The extension's WARNING: the Full is on no enabled volume.
    pub warning: Option<String>,
}

/// Default of the extension's `max_entries` argument.
#[cfg_attr(not(test), allow(dead_code))]
pub const DEFAULT_MAX_ENTRIES: i64 = 1_000_000;

/// The restore-tree functions on one catalog connection, through the
/// extension (`native == true`) or the Director fallback.
pub struct Ovfs<'a> {
    db: &'a dyn Catalog,
    native: bool,
}

impl<'a> Ovfs<'a> {
    pub fn new(db: &'a dyn Catalog, capability: &Capability) -> Self {
        Ovfs { db, native: capability.usable() }
    }

    /// Always the Director fallback.
    pub fn fallback(db: &'a dyn Catalog) -> Self {
        Ovfs { db, native: false }
    }

    /// Always the extension (tests; callers use [`Ovfs::new`]).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn extension(db: &'a dyn Catalog) -> Self {
        Ovfs { db, native: true }
    }

    pub fn path(&self) -> &'static str {
        if self.native { "ovfs" } else { "fallback" }
    }

    /// `ovfs.tree(jobids, root, depth, max_entries)`: the JSON text, `None`
    /// if nothing live remains at or below `root`.
    pub fn tree(&self, jobids: &[i32], root: &str, depth: i32, max_entries: i64) -> Result<Option<String>> {
        if self.native {
            native::tree(self.db, jobids, root, depth, max_entries)
        } else {
            fallback::tree(self.db, jobids, root, depth, max_entries)
        }
    }

    /// `ovfs.ls(jobids, dir)`.
    pub fn ls(&self, jobids: &[i32], dir: &str) -> Result<Vec<LsRow>> {
        if self.native {
            native::ls(self.db, jobids, dir)
        } else {
            fallback::ls(self.db, jobids, dir)
        }
    }

    /// `ovfs.jobids_latest(client, fileset)` with its WARNING.
    pub fn jobids_latest(&self, client: &str, fileset: Option<&str>) -> Result<Latest> {
        if self.native {
            native::jobids_latest(self.db, client, fileset)
        } else {
            fallback::jobids_latest(self.db, client, fileset)
        }
    }

    /// `ovfs.versions(client, path, name, copies)`.
    pub fn versions(&self, client: &str, path: &str, name: &str, copies: bool) -> Result<Vec<VersionRow>> {
        if self.native {
            native::versions(self.db, client, path, name, copies)
        } else {
            fallback::versions(self.db, client, path, name, copies)
        }
    }
}

// --- re-check on reload ------------------------------------------------------

/// The Director's catalog handle, for the re-check after a reload.
static RELOAD_CATALOG: std::sync::OnceLock<crate::catalog_worker::CatalogHandle> = std::sync::OnceLock::new();

/// Register the catalog whose OVFS capability `reload` checks again.
pub fn register_for_reload(catalog: crate::catalog_worker::CatalogHandle) {
    let _ = RELOAD_CATALOG.set(catalog);
}

/// Check the OVFS extension again after a configuration reload.
pub fn recheck_after_reload() {
    if let Some(catalog) = RELOAD_CATALOG.get() {
        let capability = catalog.ovfs_recheck();
        tracing::info!("reload: OVFS extension {capability}; restore trees use the {} path", capability.path());
    }
}

// --- helpers shared by both paths -------------------------------------------

/// SQL string literal (standard_conforming_strings on, as in Bareos).
pub(crate) fn literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// `'{1,2,3}'::int[]`.
pub(crate) fn int_array(jobids: &[i32]) -> String {
    let list: Vec<String> = jobids.iter().map(i32::to_string).collect();
    format!("'{{{}}}'::int[]", list.join(","))
}

/// The directory in catalog form or the extension's error.
pub(crate) fn normalize_dir(dir: &str) -> Result<Vec<u8>> {
    tree::normalize_root(dir).map_err(|_| OvfsError(format!("invalid directory: {dir:?}")))
}

/// The extension's argument checks of `tree` (root, depth, max_entries).
pub(crate) fn check_tree_args(root: &str, depth: i32, max_entries: i64) -> Result<(Vec<u8>, usize)> {
    let root = normalize_dir(root)?;
    let Ok(depth) = usize::try_from(depth) else {
        return Err(OvfsError(format!("depth must not be negative, got {depth}")));
    };
    if max_entries < 1 {
        return Err(OvfsError(format!("max_entries must be positive, got {max_entries}")));
    }
    Ok((root, depth))
}

/// Run `body` in a read-only transaction. In a `SQL_ASCII` catalog (as
/// Bareos creates it) the client encoding is set to `SQL_ASCII` for the
/// transaction: path and file names need not be UTF-8, and with the
/// connection's default `UTF8` the server refuses to send them (the
/// extension fails the same way). `SET LOCAL` ends with the transaction, so
/// the pooled connection keeps its settings.
pub(crate) fn read_only<T>(db: &dyn Catalog, body: impl FnOnce() -> Result<T>) -> Result<T> {
    let failed = |e: String| OvfsError(format!("catalog query failed: {}", server_message(&e)));
    let ascii = db.rows("SHOW server_encoding").ok()
        .and_then(|t| t.rows.first().and_then(|r| cell(r, 0)).map(|v| v == "SQL_ASCII"))
        .unwrap_or(false);
    db.exec("BEGIN READ ONLY").map_err(failed)?;
    let result = (|| {
        if ascii {
            db.exec("SET LOCAL client_encoding TO 'SQL_ASCII'").map_err(failed)?;
        }
        body()
    })();
    match result {
        Ok(value) => {
            db.exec("COMMIT").map_err(failed)?;
            Ok(value)
        }
        Err(error) => {
            let _ = db.exec("ROLLBACK");
            Err(error)
        }
    }
}

/// The primary message of a catalog error (without "Query: ", the sqlx
/// prefix and a localized severity word such as "ERROR:  "/"FEHLER:  ").
pub(crate) fn server_message(error: &str) -> String {
    let mut text = error.trim();
    for prefix in ["Query: ", "error returned from database: "] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest;
        }
    }
    let first = text.lines().next().unwrap_or("");
    match first.split_once(":  ") {
        Some((severity, message)) if !severity.contains(' ') && severity.chars().all(|c| c.is_alphabetic()) => message.to_string(),
        _ => first.to_string(),
    }
}

/// Text value of a column.
pub(crate) fn cell(row: &[Option<String>], i: usize) -> Option<&str> {
    row.get(i).and_then(|v| v.as_deref())
}

pub(crate) fn cell_i64(row: &[Option<String>], i: usize) -> Option<i64> {
    cell(row, i).and_then(|v| v.parse().ok())
}

pub(crate) fn cell_i32(row: &[Option<String>], i: usize) -> Option<i32> {
    cell(row, i).and_then(|v| v.parse().ok())
}

/// `{1,2,3}` (text form of an int[]).
pub(crate) fn parse_int_array(text: &str) -> Vec<i32> {
    text.trim_matches(|c| c == '{' || c == '}')
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}

/// `\x2f6574632f` (text form of a bytea) to bytes.
pub(crate) fn parse_bytea(text: &str) -> Vec<u8> {
    let hex = text.strip_prefix("\\x").unwrap_or(text).as_bytes();
    let nibble = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    hex.chunks_exact(2).map(|pair| nibble(pair[0]) << 4 | nibble(pair[1])).collect()
}

#[cfg(test)]
mod helper_tests {
    use super::*;

    #[test]
    fn literals_and_arrays() {
        assert_eq!(literal("/it's/"), "'/it''s/'");
        assert_eq!(int_array(&[1, 22]), "'{1,22}'::int[]");
        assert_eq!(parse_int_array("{600,615,1}"), [600, 615, 1]);
        assert!(parse_int_array("{}").is_empty());
        assert_eq!(parse_bytea("\\x2f6574632f"), b"/etc/");
        assert_eq!(parse_bytea("\\xff00"), [0xff, 0]);
    }

    #[test]
    fn server_messages() {
        assert_eq!(server_message("Query: error returned from database: job 7 does not exist"), "job 7 does not exist");
        assert_eq!(server_message("FEHLER:  job 7 does not exist\n"), "job 7 does not exist");
        assert_eq!(server_message("ERROR:  invalid directory: \"\"\nCONTEXT: x"), "invalid directory: \"\"");
        assert_eq!(server_message("catalog connection closed"), "catalog connection closed");
    }

    #[test]
    fn tree_argument_errors_like_the_extension() {
        assert_eq!(check_tree_args("", 1, 1).unwrap_err().0, "invalid directory: \"\"");
        assert_eq!(check_tree_args("/a\0b", 1, 1).unwrap_err().0, "invalid directory: \"/a\\0b\"");
        assert_eq!(check_tree_args("/", -1, 1).unwrap_err().0, "depth must not be negative, got -1");
        assert_eq!(check_tree_args("/", 1, 0).unwrap_err().0, "max_entries must be positive, got 0");
        assert_eq!(check_tree_args("/etc", 2, 5).unwrap(), (b"/etc/".to_vec(), 2));
    }
}
