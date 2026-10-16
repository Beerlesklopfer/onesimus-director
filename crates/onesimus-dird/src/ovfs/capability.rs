//! Detection of the OVFS PostgreSQL extension in the catalog database.
//!
//! The Director checks on every new catalog connection, on `reload` and on
//! `.ovfs_status refresh` whether the extension `ovfs` is installed in a
//! supported version and whether every function the Director calls exists
//! and may be executed by the catalog role. The result is kept in the
//! catalog handle; the restore-tree commands use the extension when it is
//! usable and the Director fallback (plain SQL, `ovfs::fallback`) otherwise.

use std::fmt;

use crate::catalog_worker::SqlTable;

/// Versions of the extension the Director knows: `0.1.x` (the API of
/// `oenesimus-postgres-plugin` commit `4041335`). For 0.x a minor bump may
/// break the API (Cargo semver); the extension has no upgrade scripts and
/// its version was not bumped for every API change so far, so the
/// signature and result-type check below is the real guard (policy agreed
/// with the OVFS session, 16.10.2026).
pub const MIN_VERSION: (u64, u64, u64) = (0, 1, 0);
pub const MAX_VERSION_EXCLUSIVE: (u64, u64, u64) = (0, 2, 0);

/// The function signatures the Director calls with their result types
/// (`pg_get_function_result`), in the column order of [`detect_sql`].
pub const REQUIRED_FUNCTIONS: [(&str, &str); 6] = [
    ("ovfs.tree(integer,text,integer,bigint)", "json"),
    ("ovfs.tree(integer[],text,integer,bigint)", "json"),
    ("ovfs.tree_latest(text,text,integer,text,bigint)", "json"),
    ("ovfs.jobids_latest(text,text)", "integer[]"),
    ("ovfs.ls(integer[],text)", "TABLE(name text, name_b64 text, path text, path_b64 text, kind text, \
        jobid integer, fileid bigint, fileindex integer, delta_seq integer, delta_parts json, mode bigint, \
        uid bigint, gid bigint, size bigint, mtime bigint, link_fileindex bigint, entries bigint)"),
    ("ovfs.versions(text,text,text,boolean)", "TABLE(jobid integer, level text, job_type text, \
        jobtdate bigint, starttime timestamp without time zone, fileid bigint, fileindex integer, \
        delta_seq integer, md5 text, mode bigint, uid bigint, gid bigint, size bigint, mtime bigint, \
        volumes text[], inchanger boolean)"),
];

/// One row if the extension is installed: version, schema, USAGE on the
/// schema, then per required function `missing`, `result` (other result
/// type), `denied` (no EXECUTE) or `ok` (`to_regprocedure` is NULL for a
/// missing signature). No row without the extension.
pub fn detect_sql() -> String {
    let mut sql = String::from(
        "SELECT e.extversion::text, n.nspname::text, \
         CASE WHEN has_schema_privilege(n.oid, 'USAGE') THEN 't' ELSE 'f' END",
    );
    for (function, result) in REQUIRED_FUNCTIONS {
        sql.push_str(&format!(
            ", CASE WHEN to_regprocedure('{function}') IS NULL THEN 'missing' \
             WHEN pg_get_function_result(to_regprocedure('{function}')) <> '{result}' THEN 'result' \
             WHEN has_function_privilege(to_regprocedure('{function}'), 'EXECUTE') THEN 'ok' \
             ELSE 'denied' END"
        ));
    }
    sql.push_str(" FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace WHERE e.extname = 'ovfs'");
    sql
}

/// What the Director knows about the extension in its catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Capability {
    /// Not checked yet (no catalog connection so far).
    #[default]
    Unknown,
    /// The detection query failed; the fallback is used.
    CheckFailed { error: String },
    /// `ovfs` is not installed in the catalog database.
    NotInstalled,
    /// Installed, but not usable: unsupported version, missing function or
    /// missing privilege. The fallback is used.
    Unusable { version: String, reason: String },
    /// Installed and usable.
    Available { version: String, schema: String },
}

impl Capability {
    /// True if the restore-tree commands call the extension.
    pub fn usable(&self) -> bool {
        matches!(self, Capability::Available { .. })
    }

    pub fn version(&self) -> Option<&str> {
        match self {
            Capability::Available { version, .. } | Capability::Unusable { version, .. } => Some(version),
            _ => None,
        }
    }

    pub fn installed(&self) -> bool {
        matches!(self, Capability::Available { .. } | Capability::Unusable { .. })
    }

    /// Short machine name of the state (`.api 2` / MCP).
    pub fn state(&self) -> &'static str {
        match self {
            Capability::Unknown => "unknown",
            Capability::CheckFailed { .. } => "check_failed",
            Capability::NotInstalled => "not_installed",
            Capability::Unusable { .. } => "unusable",
            Capability::Available { .. } => "available",
        }
    }

    /// Which implementation the restore-tree commands use.
    pub fn path(&self) -> &'static str {
        if self.usable() { "ovfs" } else { "fallback" }
    }

    /// Classify the result of [`detect_sql`].
    pub fn from_detection(result: Result<&SqlTable, &str>) -> Capability {
        let table = match result {
            Ok(table) => table,
            Err(error) => return Capability::CheckFailed { error: error.trim().to_string() },
        };
        let Some(row) = table.rows.first() else { return Capability::NotInstalled };
        let get = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        let version = get(0);
        let schema = get(1);
        if !supported_version(&version) {
            return Capability::Unusable {
                version: version.clone(),
                reason: format!(
                    "version {version} is not supported (need >= {}.{}.{} and < {}.{}.{})",
                    MIN_VERSION.0, MIN_VERSION.1, MIN_VERSION.2,
                    MAX_VERSION_EXCLUSIVE.0, MAX_VERSION_EXCLUSIVE.1, MAX_VERSION_EXCLUSIVE.2
                ),
            };
        }
        if get(2) != "t" {
            return Capability::Unusable { version, reason: format!("no USAGE privilege on schema {schema}") };
        }
        let mut missing = Vec::new();
        let mut result = Vec::new();
        let mut denied = Vec::new();
        for (i, (function, _)) in REQUIRED_FUNCTIONS.iter().enumerate() {
            match get(3 + i).as_str() {
                "ok" => {}
                "denied" => denied.push(*function),
                "result" => result.push(*function),
                _ => missing.push(*function),
            }
        }
        if !missing.is_empty() {
            return Capability::Unusable { version, reason: format!("missing function(s): {}", missing.join(", ")) };
        }
        if !result.is_empty() {
            return Capability::Unusable { version, reason: format!("unexpected result type of: {}", result.join(", ")) };
        }
        if !denied.is_empty() {
            return Capability::Unusable { version, reason: format!("no EXECUTE privilege on: {}", denied.join(", ")) };
        }
        Capability::Available { version, schema }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Unknown => write!(f, "not checked yet"),
            Capability::CheckFailed { error } => write!(f, "check failed: {error}"),
            Capability::NotInstalled => write!(f, "not installed"),
            Capability::Unusable { version, reason } => write!(f, "version {version} installed but not usable: {reason}"),
            Capability::Available { version, schema } => write!(f, "version {version} available (schema {schema})"),
        }
    }
}

/// `major.minor.patch` (missing parts are 0, a suffix like `-dev` is ignored)
/// within the supported range.
pub fn supported_version(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let mut parts = core.split('.');
    let mut next = || -> Option<u64> {
        match parts.next() {
            None => Some(0),
            Some(p) => p.parse().ok(),
        }
    };
    let (Some(major), Some(minor), Some(patch)) = (next(), next(), next()) else { return false };
    if parts.next().is_some() || core.is_empty() {
        return false;
    }
    let v = (major, minor, patch);
    v >= MIN_VERSION && v < MAX_VERSION_EXCLUSIVE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::SqlColumn;

    fn table(rows: Vec<Vec<&str>>) -> SqlTable {
        let width = rows.first().map_or(0, Vec::len);
        SqlTable {
            columns: (0..width).map(|i| SqlColumn { name: format!("c{i}"), numeric: false }).collect(),
            rows: rows.into_iter().map(|r| r.into_iter().map(|v| (v != "NULL").then(|| v.to_string())).collect()).collect(),
        }
    }

    fn row(version: &str, usage: &str, functions: [&str; 6]) -> SqlTable {
        let mut r = vec![version, "ovfs", usage];
        r.extend(functions);
        table(vec![r])
    }

    #[test]
    fn detection_sql_checks_every_function() {
        let sql = detect_sql();
        for (f, result) in REQUIRED_FUNCTIONS {
            assert!(sql.contains(&format!("to_regprocedure('{f}')")), "{f}");
            assert!(sql.contains(&format!("<> '{result}'")), "{f}");
        }
        assert!(sql.contains("e.extname = 'ovfs'"));
    }

    #[test]
    fn without_extension() {
        assert_eq!(Capability::from_detection(Ok(&table(vec![]))), Capability::NotInstalled);
        assert!(!Capability::NotInstalled.usable());
        assert_eq!(Capability::NotInstalled.path(), "fallback");
    }

    #[test]
    fn with_extension() {
        let cap = Capability::from_detection(Ok(&row("0.1.0", "t", ["ok"; 6])));
        assert_eq!(cap, Capability::Available { version: "0.1.0".into(), schema: "ovfs".into() });
        assert!(cap.usable());
        assert_eq!((cap.path(), cap.version(), cap.state()), ("ovfs", Some("0.1.0"), "available"));
        assert_eq!(cap.to_string(), "version 0.1.0 available (schema ovfs)");
    }

    #[test]
    fn wrong_version() {
        for v in ["0.0.9", "0.2.0", "1.0.0", "2.3", "abc", ""] {
            let cap = Capability::from_detection(Ok(&row(v, "t", ["ok"; 6])));
            assert!(matches!(&cap, Capability::Unusable { reason, .. } if reason.contains("not supported")), "{v}: {cap:?}");
            assert!(!cap.usable());
            assert!(cap.installed());
        }
        for v in ["0.1.0", "0.1", "0.1.7", "0.1.9-dev", "0.1.0+build1"] {
            assert!(supported_version(v), "{v}");
        }
    }

    #[test]
    fn missing_function() {
        let mut functions = ["ok"; 6];
        functions[4] = "missing";
        let cap = Capability::from_detection(Ok(&row("0.1.0", "t", functions)));
        assert_eq!(cap, Capability::Unusable { version: "0.1.0".into(), reason: "missing function(s): ovfs.ls(integer[],text)".into() });
        // NULL (column absent) counts as missing as well
        let cap = Capability::from_detection(Ok(&row("0.1.0", "t", ["ok", "ok", "ok", "ok", "ok", "NULL"])));
        assert!(matches!(cap, Capability::Unusable { reason, .. } if reason.contains("ovfs.versions")));
    }

    #[test]
    fn changed_result_type() {
        let mut functions = ["ok"; 6];
        functions[5] = "result";
        let cap = Capability::from_detection(Ok(&row("0.1.0", "t", functions)));
        assert_eq!(cap, Capability::Unusable { version: "0.1.0".into(),
            reason: "unexpected result type of: ovfs.versions(text,text,text,boolean)".into() });
    }

    #[test]
    fn missing_privileges() {
        let cap = Capability::from_detection(Ok(&row("0.1.0", "f", ["ok"; 6])));
        assert!(matches!(cap, Capability::Unusable { reason, .. } if reason == "no USAGE privilege on schema ovfs"));
        let mut functions = ["ok"; 6];
        functions[0] = "denied";
        let cap = Capability::from_detection(Ok(&row("0.1.0", "t", functions)));
        assert!(matches!(cap, Capability::Unusable { reason, .. } if reason.starts_with("no EXECUTE privilege on: ovfs.tree(integer,")));
    }

    #[test]
    fn failed_check_uses_fallback() {
        let cap = Capability::from_detection(Err("ERROR:  permission denied\n"));
        assert_eq!(cap, Capability::CheckFailed { error: "ERROR:  permission denied".into() });
        assert_eq!(cap.path(), "fallback");
        assert_eq!(Capability::Unknown.path(), "fallback");
    }
}
