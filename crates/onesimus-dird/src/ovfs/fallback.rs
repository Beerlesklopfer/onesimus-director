//! The Director fallback for catalogs without the `ovfs` extension: the
//! extension's SQL (`ovfs/src/lib.rs`, commit `4041335`) sent as plain
//! statements, the rows streamed through a server-side cursor in batches of
//! [`FETCH_ROWS`], resolved by the ported core ([`super::tree`]).
//!
//! Memory is that of the extension: one batch of rows, the entry being
//! resolved, and the output (the JSON text of `tree`, limited by
//! `max_entries`; the children of one directory for `ls`). Bareos semantics
//! (job chain, FileIndex 0, BaseFiles, DeltaSeq, hard links, restore option
//! 5) are documented in the extension wiki, page Developer-Bareos-Semantics.

use super::lstat;
use super::tree::{self, Entry};
use super::{
    cell, cell_i32, cell_i64, check_tree_args, int_array, literal, normalize_dir, parse_bytea, parse_int_array,
    Catalog, Latest, LsRow, OvfsError, Part, Result, VersionRow,
};

/// Rows fetched from the cursor at a time (as in the extension).
pub const FETCH_ROWS: usize = 10_000;

/// Name of the server-side cursor.
const CURSOR: &str = "ovfs_fallback_rows";

/// The extension's `ROWS_SQL`: catalog rows of the chain sorted by the bytes
/// of path and name; files of base jobs referenced by a job of the chain
/// count with the base job's JobId, FileIndex and JobTDate.
fn rows_sql(jobids: &str, filter: &str, path_key: &str, name_key: &str) -> String {
    format!(
        "SELECT textsend(p.path), textsend(f.name), f.jobid, j.jobtdate, f.fileid, f.fileindex, f.deltaseq::int, f.lstat, \
             {path_key} AS path_key, {name_key} AS name_key \
         FROM path p JOIN file f ON f.pathid = p.pathid JOIN job j ON j.jobid = f.jobid \
         WHERE f.jobid = ANY({jobids}) AND {filter} \
         UNION ALL \
         SELECT textsend(p.path), textsend(f.name), f.jobid, j.jobtdate, f.fileid, f.fileindex, f.deltaseq::int, f.lstat, \
             {path_key}, {name_key} \
         FROM basefiles b JOIN file f ON f.fileid = b.fileid JOIN path p ON p.pathid = f.pathid JOIN job j ON j.jobid = f.jobid \
         WHERE b.jobid = ANY({jobids}) AND {filter} \
         ORDER BY path_key, name_key"
    )
}

/// Byte-wise collation as Bareos creates the database (`LC_COLLATE 'C'`):
/// PostgreSQL 17+ (with the builtin provider) and 15/16; older servers or
/// any error mean "not byte-wise" (slower prefix match, same result).
const BYTEWISE_SQL: [&str; 2] = [
    "SELECT datlocprovider = 'c' AND datcollate IN ('C', 'POSIX') \
         OR datlocprovider = 'b' AND datlocale = 'C' \
     FROM pg_database WHERE datname = current_database()",
    "SELECT datlocprovider = 'c' AND datcollate IN ('C', 'POSIX') \
     FROM pg_database WHERE datname = current_database()",
];

fn bytewise(db: &dyn Catalog) -> bool {
    BYTEWISE_SQL.iter().find_map(|sql| db.rows(sql).ok()).is_some_and(|t| {
        t.rows.first().and_then(|r| cell(r, 0)) == Some("t")
    })
}

/// The extension's checks of the job list (`jobids must not be empty`,
/// `job N does not exist`, `jobs [N, M] do not exist`).
pub(crate) fn check_jobids(db: &dyn Catalog, jobids: &[i32]) -> Result<()> {
    if jobids.is_empty() {
        return Err(OvfsError("jobids must not be empty".into()));
    }
    let sql = format!(
        "SELECT array_agg(id ORDER BY id) FROM unnest({}) AS id WHERE NOT EXISTS (SELECT 1 FROM job WHERE jobid = id)",
        int_array(jobids)
    );
    let table = db.rows(&sql).map_err(|e| catalog_error(&e))?;
    let missing = table.rows.first().and_then(|r| cell(r, 0)).map(parse_int_array).unwrap_or_default();
    match missing.as_slice() {
        [] => Ok(()),
        [jobid] => Err(OvfsError(format!("job {jobid} does not exist"))),
        jobs => Err(OvfsError(format!("jobs {jobs:?} do not exist"))),
    }
}

fn catalog_error(error: &str) -> OvfsError {
    OvfsError(format!("catalog query failed: {}", super::server_message(error)))
}

/// Stream the catalog rows of the chain at or below `root` (all versions),
/// sorted by path and name, into `add`; one batch of rows is in memory at a
/// time. `add` may stop the stream with an error.
pub fn fetch(db: &dyn Catalog, jobids: &[i32], root: &[u8], mut add: impl FnMut(Entry) -> Result<()>) -> Result<()> {
    check_jobids(db, jobids)?;
    let jobids = int_array(jobids);
    // root is valid UTF-8 ending with '/': the upper bound only bumps that byte
    let root_text = String::from_utf8_lossy(root).into_owned();
    let sql = match (bytewise(db), tree::prefix_upper_bound(root)) {
        (true, Some(upper)) => {
            let upper = String::from_utf8_lossy(&upper).into_owned();
            let filter = format!("p.path >= {} AND p.path < {}", literal(&root_text), literal(&upper));
            rows_sql(&jobids, &filter, "p.path", "f.name")
        }
        _ => {
            let filter = format!("starts_with(p.path, {})", literal(&root_text));
            rows_sql(&jobids, &filter, "textsend(p.path)", "textsend(f.name)")
        }
    };

    super::read_only(db, || {
        db.exec(&format!("DECLARE {CURSOR} NO SCROLL CURSOR FOR {sql}")).map_err(|e| catalog_error(&e))?;
        loop {
            let batch = db.rows(&format!("FETCH FORWARD {FETCH_ROWS} FROM {CURSOR}")).map_err(|e| catalog_error(&e))?;
            if batch.rows.is_empty() {
                break;
            }
            for row in batch.rows {
                add(Entry {
                    path: cell(&row, 0).map(parse_bytea).unwrap_or_default(),
                    name: cell(&row, 1).map(parse_bytea).unwrap_or_default(),
                    jobid: cell_i32(&row, 2).unwrap_or_default(),
                    jobtdate: cell_i64(&row, 3).unwrap_or_default(),
                    fileid: cell_i64(&row, 4).unwrap_or_default(),
                    fileindex: cell_i32(&row, 5).unwrap_or_default(),
                    delta_seq: cell_i32(&row, 6).unwrap_or_default(),
                    lstat: cell(&row, 7).unwrap_or_default().to_string(),
                })?;
            }
        }
        db.exec(&format!("CLOSE {CURSOR}")).map_err(|e| catalog_error(&e))?;
        Ok(())
    })
}

/// `ovfs.tree(jobids, root, depth, max_entries)`.
pub fn tree(db: &dyn Catalog, jobids: &[i32], root: &str, depth: i32, max_entries: i64) -> Result<Option<String>> {
    let (root, depth) = check_tree_args(root, depth, max_entries)?;
    let mut writer = tree::TreeWriter::new(&root, depth);
    fetch(db, jobids, &root, |entry| {
        writer.add(entry);
        if writer.nodes() as i64 > max_entries {
            return Err(OvfsError(format!(
                "the tree has more than {max_entries} entries; choose a smaller depth or a deeper root"
            )));
        }
        Ok(())
    })?;
    Ok(writer.finish())
}

/// `ovfs.ls(jobids, dir)`.
pub fn ls(db: &dyn Catalog, jobids: &[i32], dir: &str) -> Result<Vec<LsRow>> {
    let dir = normalize_dir(dir)?;
    let mut lister = tree::DirLister::new(&dir);
    fetch(db, jobids, &dir, |entry| {
        lister.add(entry);
        Ok(())
    })?;
    Ok(lister.finish().into_iter().map(ls_row).collect())
}

/// Text form (lossy if not UTF-8) and, only if not UTF-8, the exact bytes.
fn text_and_b64(bytes: &[u8]) -> (String, Option<String>) {
    use base64::Engine;
    match std::str::from_utf8(bytes) {
        Ok(text) => (text.to_string(), None),
        Err(_) => (
            String::from_utf8_lossy(bytes).into_owned(),
            Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        ),
    }
}

/// The columns of `ovfs.ls` for one child (as the extension maps them).
fn ls_row(child: tree::Child) -> LsRow {
    let (name, name_b64) = text_and_b64(&child.name);
    let (path, path_b64) = match &child.path {
        Some(path) => {
            let (text, b64) = text_and_b64(path);
            (Some(text), b64)
        }
        None => (None, None),
    };
    let version = child.version.as_ref();
    let stat = version.and_then(|v| v.stat);
    LsRow {
        name,
        name_b64,
        path,
        path_b64,
        kind: child.kind.as_str().to_string(),
        jobid: version.map(|v| v.jobid),
        fileid: version.map(|v| v.fileid),
        fileindex: version.map(|v| v.fileindex),
        delta_seq: version.filter(|v| v.delta_seq > 0).map(|v| v.delta_seq),
        delta_parts: version
            .filter(|v| !v.delta_parts.is_empty())
            .map(|v| v.delta_parts.iter().map(|p| Part { jobid: p.jobid, fileindex: p.fileindex }).collect()),
        mode: stat.map(|s| s.mode),
        uid: stat.map(|s| s.uid),
        gid: stat.map(|s| s.gid),
        size: stat.map(|s| s.size),
        mtime: stat.map(|s| s.mtime),
        link_fileindex: stat.map(|s| s.link_fi).filter(|fi| *fi != 0),
        entries: child.entries.map(|n| n as i64),
    }
}

/// The extension's `LATEST_CHAIN_SQL` (restore option 5: `uar_last_full_no_pool`,
/// `uar_full`, `uar_dif`, `uar_inc`): Full jobid, whether the Full is on an
/// enabled volume, and the chain ordered by JobTDate.
pub(crate) fn latest_chain_sql(clientid: i64, fileset: &str) -> String {
    format!(
        "WITH candidate AS ( \
             SELECT j.jobid, j.jobtdate, j.level, EXISTS ( \
                     SELECT 1 FROM jobmedia jm JOIN media m ON m.mediaid = jm.mediaid \
                     WHERE jm.jobid = j.jobid AND m.enabled = 1) AS on_media \
             FROM job j JOIN fileset fs ON fs.filesetid = j.filesetid \
             WHERE j.clientid = {clientid} AND fs.fileset = {fileset} AND j.type = 'B' \
                 AND j.jobstatus IN ('T', 'W') AND j.starttime < localtimestamp), \
         full_job AS (SELECT * FROM candidate WHERE level = 'F' ORDER BY jobtdate DESC LIMIT 1), \
         diff_job AS ( \
             SELECT c.* FROM candidate c, full_job f \
             WHERE c.level = 'D' AND c.on_media AND c.jobtdate > f.jobtdate \
             ORDER BY c.jobtdate DESC LIMIT 1), \
         incr_job AS ( \
             SELECT c.* FROM candidate c, full_job f \
             WHERE c.level = 'I' AND c.on_media \
                 AND c.jobtdate > greatest(f.jobtdate, (SELECT jobtdate FROM diff_job))) \
         SELECT (SELECT jobid FROM full_job), (SELECT on_media FROM full_job), \
             coalesce((SELECT array_agg(jobid ORDER BY jobtdate, jobid) FROM ( \
                 SELECT * FROM full_job WHERE on_media \
                 UNION ALL SELECT * FROM diff_job \
                 UNION ALL SELECT * FROM incr_job) chain), '{{}}')",
        fileset = literal(fileset)
    )
}

/// The client's id and the FileSet to use (given, or the only one the
/// client has backups of), with the extension's errors.
pub(crate) fn client_and_fileset(db: &dyn Catalog, client: &str, fileset: Option<&str>) -> Result<(i64, String)> {
    let sql = format!("SELECT (SELECT clientid FROM client WHERE name = {})", literal(client));
    let table = db.rows(&sql).map_err(|e| catalog_error(&e))?;
    let Some(clientid) = table.rows.first().and_then(|r| cell_i64(r, 0)) else {
        return Err(OvfsError(format!("client {client:?} does not exist")));
    };
    let fileset = match fileset {
        Some(fileset) => fileset.to_string(),
        None => {
            // uar_sel_fileset, limited to backups
            let sql = format!(
                "SELECT array_to_json(array_agg(DISTINCT fs.fileset ORDER BY fs.fileset))::text \
                 FROM job j JOIN fileset fs ON fs.filesetid = j.filesetid \
                 WHERE j.clientid = {clientid} AND j.type = 'B'"
            );
            let table = db.rows(&sql).map_err(|e| catalog_error(&e))?;
            let filesets: Vec<String> = table.rows.first().and_then(|r| cell(r, 0))
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or_default();
            match <[String; 1]>::try_from(filesets) {
                Ok([fileset]) => fileset,
                Err(filesets) if filesets.is_empty() => return Err(OvfsError(format!("client {client:?} has no backups"))),
                Err(filesets) => {
                    return Err(OvfsError(format!("client {client:?} has backups of several FileSets {filesets:?}; choose one")))
                }
            }
        }
    };
    Ok((clientid, fileset))
}

/// Full jobid, Full on an enabled volume, chain.
pub(crate) fn latest_chain(db: &dyn Catalog, clientid: i64, fileset: &str) -> Result<(Option<i32>, bool, Vec<i32>)> {
    let table = db.rows(&latest_chain_sql(clientid, fileset)).map_err(|e| catalog_error(&e))?;
    let row = table.rows.into_iter().next().unwrap_or_default();
    Ok((cell_i32(&row, 0), cell(&row, 1) == Some("t"), cell(&row, 2).map(parse_int_array).unwrap_or_default()))
}

/// The extension's WARNING text.
pub(crate) fn full_warning(full: i32, client: &str, jobids: &[i32]) -> String {
    format!(
        "Full backup job {full} of client {client:?} is on no enabled volume; \
         the chain {jobids:?} lacks it and a restore would be incomplete"
    )
}

/// `ovfs.jobids_latest(client, fileset)`.
pub fn jobids_latest(db: &dyn Catalog, client: &str, fileset: Option<&str>) -> Result<Latest> {
    let (clientid, fileset) = client_and_fileset(db, client, fileset)?;
    let (full, full_on_media, jobids) = latest_chain(db, clientid, &fileset)?;
    let Some(full) = full else {
        return Err(OvfsError(format!("no Full backup of client {client:?} with FileSet {fileset:?} found")));
    };
    if jobids.is_empty() {
        return Err(OvfsError(format!("no job of client {client:?} with FileSet {fileset:?} is on an enabled volume")));
    }
    let warning = (!full_on_media).then(|| full_warning(full, client, &jobids));
    Ok(Latest { jobids, warning })
}

/// `ovfs.versions(client, path, name, copies)`: the extension's
/// `VERSIONS_SQL` with LStat decoded here.
pub fn versions(db: &dyn Catalog, client: &str, path: &str, name: &str, copies: bool) -> Result<Vec<VersionRow>> {
    let path = String::from_utf8_lossy(&normalize_dir(path)?).into_owned();
    let sql = format!(
        "SELECT f.jobid, j.level::text, j.type::text, j.jobtdate, j.starttime, f.fileid, f.fileindex, \
             f.deltaseq::int, f.md5, f.lstat, \
             array_to_json(array_agg(DISTINCT m.volumename ORDER BY m.volumename))::text, bool_or(m.inchanger <> 0) \
         FROM path p \
         JOIN file f ON f.pathid = p.pathid \
         JOIN job j ON j.jobid = f.jobid \
         JOIN client c ON c.clientid = j.clientid \
         JOIN jobmedia jm ON jm.jobid = j.jobid AND f.fileindex BETWEEN jm.firstindex AND jm.lastindex \
         JOIN media m ON m.mediaid = jm.mediaid \
         WHERE p.path = {path} AND f.name = {name} AND c.name = {client} \
             AND (j.type IN ('B', 'A', 'a') OR {copies} AND j.type = 'C') \
         GROUP BY f.fileid, j.jobid \
         ORDER BY j.jobtdate DESC, f.fileid DESC",
        path = literal(&path),
        name = literal(name),
        client = literal(client),
        copies = if copies { "true" } else { "false" },
    );
    let table = super::read_only(db, || db.rows(&sql).map_err(|e| catalog_error(&e)))?;
    Ok(table
        .rows
        .iter()
        .map(|row| {
            let stat = lstat::decode(cell(row, 9).unwrap_or_default());
            VersionRow {
                jobid: cell_i32(row, 0).unwrap_or_default(),
                level: cell(row, 1).unwrap_or_default().to_string(),
                job_type: cell(row, 2).unwrap_or_default().to_string(),
                jobtdate: cell_i64(row, 3).unwrap_or_default(),
                starttime: cell(row, 4).map(str::to_string),
                fileid: cell_i64(row, 5).unwrap_or_default(),
                fileindex: cell_i32(row, 6).unwrap_or_default(),
                delta_seq: cell_i32(row, 7).unwrap_or_default(),
                md5: cell(row, 8).unwrap_or_default().to_string(),
                mode: stat.map(|s| s.mode),
                uid: stat.map(|s| s.uid),
                gid: stat.map(|s| s.gid),
                size: stat.map(|s| s.size),
                mtime: stat.map(|s| s.mtime),
                volumes: cell(row, 10).and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default(),
                inchanger: cell(row, 11) == Some("t"),
            }
        })
        .collect())
}
