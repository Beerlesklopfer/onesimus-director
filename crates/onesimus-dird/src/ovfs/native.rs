//! Calls of the `ovfs` extension functions (schema `ovfs`).
//!
//! Arguments are checked here first with the extension's own messages, so an
//! invalid argument reads the same on both paths; errors raised by the
//! extension are reported with their primary message.

use super::fallback;
use super::{
    cell, cell_i32, cell_i64, check_tree_args, int_array, literal, normalize_dir, parse_int_array, server_message,
    Catalog, Latest, LsRow, OvfsError, Part, Result, VersionRow,
};

fn call_error(error: &str) -> OvfsError {
    OvfsError(server_message(error))
}

/// `SELECT ovfs.tree(jobids, root, depth, max_entries)`.
pub fn tree(db: &dyn Catalog, jobids: &[i32], root: &str, depth: i32, max_entries: i64) -> Result<Option<String>> {
    check_tree_args(root, depth, max_entries)?;
    let sql = format!(
        "SELECT ovfs.tree({}, {}, {depth}, {max_entries})",
        int_array(jobids),
        literal(root)
    );
    let table = super::read_only(db, || db.rows(&sql).map_err(|e| call_error(&e)))?;
    Ok(table.rows.into_iter().next().and_then(|row| row.into_iter().next().flatten()))
}

/// `SELECT … FROM ovfs.ls(jobids, dir)`.
pub fn ls(db: &dyn Catalog, jobids: &[i32], dir: &str) -> Result<Vec<LsRow>> {
    normalize_dir(dir)?;
    let sql = format!(
        "SELECT name, name_b64, path, path_b64, kind, jobid, fileid, fileindex, delta_seq, delta_parts::text, \
             mode, uid, gid, size, mtime, link_fileindex, entries FROM ovfs.ls({}, {})",
        int_array(jobids),
        literal(dir)
    );
    let table = super::read_only(db, || db.rows(&sql).map_err(|e| call_error(&e)))?;
    Ok(table
        .rows
        .iter()
        .map(|row| LsRow {
            name: cell(row, 0).unwrap_or_default().to_string(),
            name_b64: cell(row, 1).map(str::to_string),
            path: cell(row, 2).map(str::to_string),
            path_b64: cell(row, 3).map(str::to_string),
            kind: cell(row, 4).unwrap_or_default().to_string(),
            jobid: cell_i32(row, 5),
            fileid: cell_i64(row, 6),
            fileindex: cell_i32(row, 7),
            delta_seq: cell_i32(row, 8),
            delta_parts: cell(row, 9).map(parse_parts),
            mode: cell_i64(row, 10),
            uid: cell_i64(row, 11),
            gid: cell_i64(row, 12),
            size: cell_i64(row, 13),
            mtime: cell_i64(row, 14),
            link_fileindex: cell_i64(row, 15),
            entries: cell_i64(row, 16),
        })
        .collect())
}

/// `[{"jobid":10,"fileindex":5}, …]`.
fn parse_parts(text: &str) -> Vec<Part> {
    let value: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    value
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .map(|p| Part {
                    jobid: p["jobid"].as_i64().unwrap_or_default() as i32,
                    fileindex: p["fileindex"].as_i64().unwrap_or_default() as i32,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `SELECT ovfs.jobids_latest(client, fileset)`.
///
/// The extension reports a Full on no enabled volume as a WARNING notice,
/// which the catalog connection does not hand over; the Director recomputes
/// that condition with the extension's own query (only then).
pub fn jobids_latest(db: &dyn Catalog, client: &str, fileset: Option<&str>) -> Result<Latest> {
    let sql = format!(
        "SELECT ovfs.jobids_latest({}, {})",
        literal(client),
        fileset.map_or_else(|| "NULL".to_string(), literal)
    );
    let table = super::read_only(db, || db.rows(&sql).map_err(|e| call_error(&e)))?;
    let jobids = table.rows.first().and_then(|r| cell(r, 0)).map(parse_int_array).unwrap_or_default();
    let (clientid, fileset) = fallback::client_and_fileset(db, client, fileset)?;
    let (full, full_on_media, _) = fallback::latest_chain(db, clientid, &fileset)?;
    let warning = match full {
        Some(full) if !full_on_media => Some(fallback::full_warning(full, client, &jobids)),
        _ => None,
    };
    Ok(Latest { jobids, warning })
}

/// `SELECT … FROM ovfs.versions(client, path, name, copies)`.
pub fn versions(db: &dyn Catalog, client: &str, path: &str, name: &str, copies: bool) -> Result<Vec<VersionRow>> {
    normalize_dir(path)?;
    let sql = format!(
        "SELECT jobid, level, job_type, jobtdate, starttime, fileid, fileindex, delta_seq, md5, \
             mode, uid, gid, size, mtime, array_to_json(volumes)::text, inchanger \
         FROM ovfs.versions({}, {}, {}, {})",
        literal(client),
        literal(path),
        literal(name),
        if copies { "true" } else { "false" }
    );
    let table = super::read_only(db, || db.rows(&sql).map_err(|e| call_error(&e)))?;
    Ok(table
        .rows
        .iter()
        .map(|row| VersionRow {
            jobid: cell_i32(row, 0).unwrap_or_default(),
            level: cell(row, 1).unwrap_or_default().to_string(),
            job_type: cell(row, 2).unwrap_or_default().to_string(),
            jobtdate: cell_i64(row, 3).unwrap_or_default(),
            starttime: cell(row, 4).map(str::to_string),
            fileid: cell_i64(row, 5).unwrap_or_default(),
            fileindex: cell_i32(row, 6).unwrap_or_default(),
            delta_seq: cell_i32(row, 7).unwrap_or_default(),
            md5: cell(row, 8).unwrap_or_default().to_string(),
            mode: cell_i64(row, 9),
            uid: cell_i64(row, 10),
            gid: cell_i64(row, 11),
            size: cell_i64(row, 12),
            mtime: cell_i64(row, 13),
            volumes: cell(row, 14).and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default(),
            inchanger: cell(row, 15) == Some("t"),
        })
        .collect())
}
