// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Onesimus extension, not in Bareos: purge several volumes at once with a
//! mandatory preview (decision of the user, 2026-11-20).
//!
//! ```text
//! purge volume all pool=<pool>|allpools [volstatus=<s>[,<s>...]] dryrun|yes
//! ```
//!
//! The bare word `all` selects the extension; `purge volume pool=X` (volume
//! selection menu) and `purge volume action=…` (ActionOnPurge, also
//! `action=all`) keep their Bareos meaning. The dialog part (warning, the
//! full-ACL check of PurgeCmd, the catalog) is the one of every `purge`
//! (admin.rs); this module parses the request and does the work.
//!
//! Safety: `yes` purges every eligible volume through the same function
//! as `purge volume=<name> yes` ([`purge::purge_jobs_from_volume`]) with all
//! its checks; the dry run asks the same refusal checks
//! ([`purge::volume_purge_refused`]) and changes nothing. A volume the
//! single form refuses is skipped with that reason. Never interactive.

use serde_json::{json, Value};

use crate::command_ast::{Argument, CommandAst};
use crate::commands::bvfs::Db;
use crate::commands::purge::{self, Volume};
use crate::ua_select::Ua;

/// VolStatus values `volstatus=` accepts (canonical spelling).
pub(crate) const VOLSTATUS: [&str; 7] = ["Full", "Used", "Error", "Append", "Archive", "Disabled", "Read-Only"];

/// VolStatus values selected without `volstatus=`.
pub(crate) const DEFAULT_VOLSTATUS: [&str; 3] = ["Full", "Used", "Error"];

/// Arguments the bulk form accepts (`catalog=` is OpenClientDb's).
const ARGUMENTS: [&str; 7] = ["volume", "all", "pool", "allpools", "volstatus", "dryrun", "yes"];

/// A parsed `purge volume all …` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BulkPurge {
    /// `pool=<pool>`; None with `allpools`.
    pub pool: Option<String>,
    /// `allpools`: volumes of every pool the console may access.
    pub allpools: bool,
    /// Selected VolStatus values, canonical spelling, in the given order.
    pub volstatus: Vec<String>,
    /// `dryrun`: report only; otherwise `yes`: purge.
    pub dryrun: bool,
}

/// What happened to one selected volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BulkAction {
    /// Dry run: the volume would be purged.
    WouldPurge,
    /// The volume was purged (and marked Purged).
    Purged,
    /// The volume was left alone; the reason says why.
    Skipped,
}

impl BulkAction {
    /// The JSON value of `action`.
    fn json_name(self) -> &'static str {
        match self {
            BulkAction::WouldPurge => "would_purge",
            BulkAction::Purged => "purged",
            BulkAction::Skipped => "skipped",
        }
    }
}

/// One selected volume with its catalog counts and the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BulkVolume {
    /// Media.MediaId.
    pub media_id: i64,
    /// Media.VolumeName.
    pub name: String,
    /// Pool.Name of the volume when it was selected.
    pub pool: String,
    /// Media.PoolId when it was selected.
    pub pool_id: i64,
    /// Media.VolStatus when it was selected.
    pub status: String,
    /// JobIds with JobMedia records on the volume (distinct).
    pub jobids: Vec<u32>,
    /// Sum of Job.JobFiles of those jobs.
    pub files: u64,
    /// Media.VolBytes.
    pub bytes: u64,
    /// Outcome.
    pub action: BulkAction,
    /// Why the volume was skipped (None otherwise).
    pub reason: Option<String>,
}

/// Whether `ast` is the bulk form: a `volume` argument together with the
/// bare word `all` (`action=all` does not count). None: not the
/// extension (the Bareos forms run unchanged); Some(Err): the extension
/// with invalid arguments, the message to refuse it with.
pub(crate) fn bulk_request(ast: &CommandAst) -> Option<Result<BulkPurge, String>> {
    let bare = |key: &str| ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_none());
    if !bare("all") || !ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("volume")) {
        return None;
    }
    Some(parse(ast))
}

/// Validates the arguments of the bulk form.
fn parse(ast: &CommandAst) -> Result<BulkPurge, String> {
    const USAGE: &str = "Usage: purge volume all pool=<pool>|allpools [volstatus=<status>[,<status>...]] dryrun|yes\n";
    let refuse = |why: String| Err(format!("purge volume all: {why}\n{USAGE}"));
    for arg in &ast.arguments {
        let key = arg.key.to_ascii_lowercase();
        if key == "catalog" {
            continue;
        }
        if !ARGUMENTS.contains(&key.as_str()) {
            return refuse(format!("argument \"{}\" does not apply to this form.", arg.key));
        }
        let wants_value = matches!(key.as_str(), "pool" | "volstatus");
        if wants_value != arg.value.is_some() {
            return refuse(if key == "volume" {
                "\"all\" cannot be combined with volume=<name>.".to_string()
            } else if wants_value {
                format!("{key} needs a value.")
            } else {
                format!("{key} takes no value.")
            });
        }
        if ast.arguments.iter().filter(|a| a.key.eq_ignore_ascii_case(&key)).count() > 1 {
            return refuse(format!("{key} given more than once."));
        }
    }
    let find = |key: &str| ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(key));
    let pool = find("pool").and_then(|a| a.value.clone());
    let allpools = find("allpools").is_some();
    match (&pool, allpools) {
        (Some(p), false) if p.is_empty() => return refuse("pool needs a value.".into()),
        (Some(_), false) | (None, true) => {}
        _ => return refuse("give exactly one of pool=<pool> or allpools.".into()),
    }
    let dryrun = find("dryrun").is_some();
    if dryrun == find("yes").is_some() {
        return refuse("give exactly one of dryrun (preview, changes nothing) or yes (purge); \
            this form never asks for confirmation.".into());
    }
    let volstatus = match find("volstatus").and_then(|a| a.value.as_deref()) {
        None => DEFAULT_VOLSTATUS.iter().map(|s| s.to_string()).collect(),
        Some(list) => {
            let mut statuses: Vec<String> = Vec::new();
            for item in list.split(',') {
                let Some(status) = VOLSTATUS.iter().find(|s| s.eq_ignore_ascii_case(item.trim())) else {
                    return refuse(format!("invalid volstatus \"{item}\"; allowed: {}.", VOLSTATUS.join(", ")));
                };
                if !statuses.iter().any(|s| s == status) {
                    statuses.push(status.to_string());
                }
            }
            statuses
        }
    };
    Ok(BulkPurge { pool, allpools, volstatus, dryrun })
}

/// SQL string literal content.
fn esc(text: &str) -> String {
    text.replace('\'', "''")
}

/// Text of column `i` of a catalog row.
fn cell(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

/// The volumes of the request: VolStatus in the filter, pool as selected,
/// pools the console may not access left out. Purged and Recycle volumes
/// are never selected (not in [`VOLSTATUS`]).
fn select_volumes(db: &dyn Db, request: &BulkPurge, pool_allowed: &dyn Fn(&str) -> bool)
    -> Result<Vec<BulkVolume>, String> {
    let statuses = request.volstatus.iter().map(|s| format!("'{}'", esc(s))).collect::<Vec<_>>().join(",");
    let pool = request.pool.as_ref().map(|p| format!(" AND Pool.Name='{}'", esc(p))).unwrap_or_default();
    let table = db.rows(&format!(
        "SELECT Media.MediaId,Media.VolumeName,Pool.Name,Media.VolStatus,Media.VolBytes,Media.PoolId \
         FROM Media JOIN Pool ON Pool.PoolId=Media.PoolId WHERE Media.VolStatus IN ({statuses}){pool} \
         ORDER BY Pool.Name,Media.VolumeName"))?;
    let mut volumes: Vec<BulkVolume> = table.rows.iter()
        .map(|row| BulkVolume {
            media_id: cell(row, 0).parse().unwrap_or(0), name: cell(row, 1), pool: cell(row, 2), status: cell(row, 3),
            bytes: cell(row, 4).parse().unwrap_or(0), pool_id: cell(row, 5).parse().unwrap_or(0),
            jobids: Vec::new(), files: 0, action: BulkAction::Skipped, reason: None,
        })
        // Defense in depth: PurgeCmd already demands full Pool ACL access.
        .filter(|v| pool_allowed(&v.pool) && VOLSTATUS.contains(&v.status.as_str()))
        .collect();
    if volumes.is_empty() {
        return Ok(volumes);
    }
    // Jobs per volume (JobMedia, distinct) and their files (Job.JobFiles).
    let media = volumes.iter().map(|v| v.media_id.to_string()).collect::<Vec<_>>().join(",");
    let pairs = db.rows(&format!("SELECT DISTINCT MediaId,JobId FROM JobMedia WHERE MediaId IN ({media})"))?;
    for row in &pairs.rows {
        let (media_id, jobid) = (cell(row, 0).parse::<i64>().unwrap_or(0), cell(row, 1).parse::<u32>().unwrap_or(0));
        if jobid == 0 {
            continue;
        }
        if let Some(volume) = volumes.iter_mut().find(|v| v.media_id == media_id && !v.jobids.contains(&jobid)) {
            volume.jobids.push(jobid);
        }
    }
    let mut all: Vec<u32> = volumes.iter().flat_map(|v| v.jobids.iter().copied()).collect();
    all.sort_unstable();
    all.dedup();
    if !all.is_empty() {
        let ids = all.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let files = db.rows(&format!("SELECT JobId,JobFiles FROM Job WHERE JobId IN ({ids})"))?;
        let files: Vec<(u32, u64)> = files.rows.iter()
            .map(|row| (cell(row, 0).parse().unwrap_or(0), cell(row, 1).parse().unwrap_or(0))).collect();
        for volume in &mut volumes {
            volume.files = volume.jobids.iter()
                .map(|id| files.iter().find(|(job, _)| job == id).map_or(0, |(_, n)| *n)).sum();
        }
    }
    Ok(volumes)
}

/// The messages a nested UaContext collected, as one line (the reason of a
/// skipped volume): errors first, else the last line of the text.
fn reason_of(result: &crate::command_queue::CommandResult) -> String {
    let join = |text: &str| text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    let errors = join(&result.error_message);
    if !errors.is_empty() {
        return errors;
    }
    result.text_output.lines().map(str::trim).rfind(|l| !l.is_empty())
        .map_or_else(|| "not purged".to_string(), str::to_string)
}

/// Why `volume` is skipped before the single-volume checks run: a running
/// job wrote to it (JobMedia of a running JobId).
fn in_use(volume: &BulkVolume, running: &[u32]) -> Option<String> {
    let busy: Vec<String> = volume.jobids.iter().filter(|id| running.contains(id)).map(u32::to_string).collect();
    (!busy.is_empty()).then(|| format!("in use by running job {}", busy.join(",")))
}

/// Dry run of one volume: the refusal checks of `purge volume=<name>`
/// on a nested UaContext; nothing is written.
fn preview(ua: &Ua, db: &dyn Db, volume: &mut BulkVolume, running: &[u32]) {
    if let Some(reason) = in_use(volume, running) {
        volume.reason = Some(reason);
        return;
    }
    let record = Volume { media_id: volume.media_id, name: volume.name.clone(), pool_id: volume.pool_id,
        status: volume.status.clone(), ..Default::default() };
    let mut nested = Ua::new(ua.ctx);
    if purge::volume_purge_refused(&mut nested, db, &record) {
        volume.reason = Some(reason_of(&nested.finish(false)));
    } else {
        volume.action = BulkAction::WouldPurge;
    }
}

/// `yes` for one volume: re-read the record (pool and VolStatus may have
/// changed since the selection), then `purge volume=<name> yes` through
/// [`purge::purge_jobs_from_volume`] on a nested UaContext.
fn purge_one(ua: &Ua, db: &dyn Db, request: &BulkPurge, volume: &mut BulkVolume, running: &[u32]) {
    if let Some(reason) = in_use(volume, running) {
        volume.reason = Some(reason);
        return;
    }
    let mut record = match purge::get_volume(db, volume.media_id) {
        Ok(record) => record,
        Err(error) => {
            volume.reason = Some(error.trim_end().to_string());
            return;
        }
    };
    if record.pool_id != volume.pool_id {
        volume.reason = Some(format!("no longer in pool {}", volume.pool));
        return;
    }
    if !request.volstatus.contains(&record.status) {
        volume.reason = Some(format!("VolStatus changed to {}", record.status));
        return;
    }
    let ast = CommandAst {
        command_name: "purge".into(),
        arguments: vec![Argument { key: "volume".into(), value: Some(record.name.clone()) },
                        Argument { key: "yes".into(), value: None }],
        ..Default::default()
    };
    let mut nested = Ua::new(ua.ctx);
    // force = true like PurgeAction::JobsOfVolume of `purge volume=<name>` (admin.rs)
    if purge::purge_jobs_from_volume(&mut nested, &ast, db, &mut record, true) {
        volume.action = BulkAction::Purged;
    } else {
        volume.reason = Some(reason_of(&nested.finish(false)));
    }
}

/// The text line of one volume.
fn line(volume: &BulkVolume) -> String {
    let outcome = match volume.action {
        BulkAction::WouldPurge => "would purge".to_string(),
        BulkAction::Purged => "purged".to_string(),
        BulkAction::Skipped => format!("skipped: {}", volume.reason.as_deref().unwrap_or("")),
    };
    format!("Volume \"{}\" (Pool {}, {}, {} jobs, {} files): {outcome}\n",
        volume.name, volume.pool, volume.status, volume.jobids.len(), volume.files)
}

/// Runs a bulk purge: the text lines and summary, and in API mode 2 the
/// `purge` object. `running`: JobIds of running jobs; `pool_allowed`: the
/// console's Pool ACL. False (error sent) when the pool is unknown or not
/// accessible, or the selection failed; a refused volume is no failure.
pub(crate) fn purge_volumes(ua: &mut Ua, db: &dyn Db, request: &BulkPurge, running: &[u32],
    pool_allowed: &dyn Fn(&str) -> bool) -> bool {
    if let Some(pool) = &request.pool {
        // A forbidden pool reads like an unknown one (GetPoolDbr).
        let known = pool_allowed(pool) && db.rows(&format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(pool)))
            .is_ok_and(|t| !t.rows.is_empty());
        if !known {
            ua.error_msg(format!("Could not find Pool \"{pool}\": ERR=Pool record not found in Catalog.\n"));
            return false;
        }
    }
    let mut volumes = match select_volumes(db, request, pool_allowed) {
        Ok(volumes) => volumes,
        Err(error) => {
            ua.error_msg(if error.ends_with('\n') { error } else { format!("{error}\n") });
            return false;
        }
    };
    for volume in &mut volumes {
        if request.dryrun {
            preview(ua, db, volume, running);
        } else {
            purge_one(ua, db, request, volume, running);
        }
    }
    let done = volumes.iter().filter(|v| v.action != BulkAction::Skipped).count();
    let skipped = volumes.len() - done;
    if volumes.is_empty() {
        ua.send_msg(format!("No volumes with VolStatus {} found in {}.\n", request.volstatus.join(", "),
            request.pool.as_ref().map_or_else(|| "any pool".to_string(), |p| format!("pool {p}"))));
    }
    for volume in &volumes {
        ua.send_msg(line(volume));
    }
    ua.send_msg(if request.dryrun {
        format!("Dry run: {} volumes selected, {done} would be purged, {skipped} skipped. Nothing was changed.\n",
            volumes.len())
    } else {
        format!("{} volumes selected, {done} purged, {skipped} skipped.\n", volumes.len())
    });
    ua.set_json(json!({ "purge": {
        "mode": if request.dryrun { "dryrun" } else { "yes" },
        "pool": request.pool,
        "allpools": request.allpools,
        "volstatus": request.volstatus,
        "volumes": volumes.iter().map(|v| json!({
            "volume": v.name, "pool": v.pool, "volstatus": v.status, "jobs": v.jobids.len(), "files": v.files,
            "bytes": v.bytes, "action": v.action.json_name(), "reason": v.reason,
        })).collect::<Vec<Value>>(),
        "summary": { "selected": volumes.len(), "purged": done, "skipped": skipped },
    }}));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{SqlColumn, SqlTable};
    use crate::protect::tests::{job, Catalog, Job};
    use std::sync::Mutex;

    fn ast(line: &str) -> CommandAst {
        crate::session::parse_cli_command(line)
    }

    #[test]
    fn only_the_bare_word_all_selects_the_extension() {
        // Bareos forms: not the extension.
        for line in ["purge volume=X yes", "purge volume pool=Full", "purge volume action=truncate allpools",
                     "purge volume action=all pool=Full", "purge jobs client=fd1", "purge", "purge volume all=1 pool=Full"] {
            assert_eq!(bulk_request(&ast(line)), None, "{line}");
        }
        assert_eq!(bulk_request(&ast("purge volume all pool=Full dryrun")), Some(Ok(BulkPurge {
            pool: Some("Full".into()), allpools: false, volstatus: vec!["Full".into(), "Used".into(), "Error".into()],
            dryrun: true })));
        assert_eq!(bulk_request(&ast("purge volume all allpools volstatus=full,read-only,Full yes")), Some(Ok(BulkPurge {
            pool: None, allpools: true, volstatus: vec!["Full".into(), "Read-Only".into()], dryrun: false })));
    }

    #[test]
    fn invalid_bulk_requests_are_refused() {
        let refused = |line: &str| match bulk_request(&ast(line)) {
            Some(Err(message)) => message,
            other => panic!("{line}: {other:?}"),
        };
        // neither dryrun nor yes, or both
        assert!(refused("purge volume all pool=Full").contains("give exactly one of dryrun"));
        assert!(refused("purge volume all pool=Full dryrun yes").contains("give exactly one of dryrun"));
        // pool and allpools, or neither
        assert!(refused("purge volume all pool=Full allpools dryrun").contains("exactly one of pool=<pool> or allpools"));
        assert!(refused("purge volume all dryrun").contains("exactly one of pool=<pool> or allpools"));
        // volstatus values
        assert!(refused("purge volume all pool=Full volstatus=Purged dryrun").contains("invalid volstatus \"Purged\""));
        assert!(refused("purge volume all pool=Full volstatus=Recycle yes").contains("invalid volstatus"));
        // foreign arguments and a volume name
        assert!(refused("purge volume all pool=Full action=truncate dryrun").contains("\"action\" does not apply"));
        assert!(refused("purge volume all pool=Full jobstatus=E yes").contains("\"jobstatus\" does not apply"));
        assert!(refused("purge volume=X all yes").contains("cannot be combined with volume=<name>"));
        assert!(refused("purge volume all pool=A pool=B dryrun").contains("more than once"));
        assert!(refused("purge volume all pool=Full").ends_with("dryrun|yes\n"));
    }

    /// Volume of the test catalog: MediaId, name, PoolId, pool, VolStatus, VolBytes.
    type Media = (i64, &'static str, i64, &'static str, String, u64);

    /// The protect.rs catalog (jobs, JobMedia, latest chains) plus Media
    /// and Pool rows; purges remove the jobs, `UPDATE Media` changes them.
    struct BulkDb {
        cat: Mutex<Catalog>,
        media: Mutex<Vec<Media>>,
        log: Mutex<Vec<String>>,
    }

    fn table(rows: Vec<Vec<String>>) -> SqlTable {
        let width = rows.first().map_or(1, Vec::len);
        SqlTable { columns: (0..width).map(|i| SqlColumn { name: format!("c{i}"), numeric: false }).collect(),
                   rows: rows.into_iter().map(|r| r.into_iter().map(Some).collect()).collect() }
    }

    fn ids_in(sql: &str, after: &str) -> Vec<i64> {
        let rest = &sql[sql.find(after).unwrap() + after.len()..];
        rest[..rest.find(')').unwrap()].split(',').filter_map(|s| s.trim().parse().ok()).collect()
    }

    impl BulkDb {
        /// fd1/fs: old chain 1 (Full) and 2 (Incremental) on Full-A, latest
        /// Full 3 on Full-B (protected); fd2 has Incrementals only (nothing
        /// protected): 5 on Full-C, 7 on Inc-F, 8 on Sec-1, 9 on Full-G.
        fn new() -> Self {
            let mut jobs: Vec<Job> = vec![job(1, 1, 11, "fs", 'F', 1), job(2, 1, 11, "fs", 'I', 2), job(3, 1, 11, "fs", 'F', 3),
                job(5, 2, 11, "fs", 'I', 5), job(7, 2, 11, "fs", 'I', 7), job(8, 2, 11, "fs", 'I', 8), job(9, 2, 11, "fs", 'I', 9)];
            for (job, media) in jobs.iter_mut().zip([10, 10, 11, 12, 15, 18, 16]) {
                job.media = vec![media];
            }
            jobs[1].files = 7;
            let m = |id, name, pool_id, pool, status: &str, bytes| (id, name, pool_id, pool, status.to_string(), bytes);
            Self {
                cat: Mutex::new(Catalog::new(jobs)),
                media: Mutex::new(vec![
                    m(10, "Full-A", 1, "Full", "Full", 1000), m(11, "Full-B", 1, "Full", "Full", 2000),
                    m(12, "Full-C", 1, "Full", "Used", 3000), m(13, "Full-D", 1, "Full", "Error", 0),
                    m(14, "Full-E", 1, "Full", "Append", 500), m(15, "Inc-F", 2, "Incremental", "Full", 4000),
                    m(16, "Full-G", 1, "Full", "Archive", 600), m(17, "Full-H", 1, "Full", "Purged", 0),
                    m(18, "Sec-1", 3, "Secret", "Full", 700)]),
                log: Mutex::new(Vec::new()),
            }
        }
        fn writes(&self) -> Vec<String> {
            self.log.lock().unwrap().iter().filter(|s| !s.starts_with("SELECT") && !s.starts_with("CREATE TEMPORARY")
                && !matches!(s.as_str(), "BEGIN" | "COMMIT")).cloned().collect()
        }
        fn status(&self, name: &str) -> String {
            self.media.lock().unwrap().iter().find(|m| m.1 == name).unwrap().4.clone()
        }
        fn jobs(&self) -> Vec<u32> {
            self.cat.lock().unwrap().jobs.iter().map(|j| j.id).collect()
        }
    }

    impl Db for BulkDb {
        fn rows(&self, sql: &str) -> Result<SqlTable, String> {
            self.log.lock().unwrap().push(sql.to_string());
            let media = self.media.lock().unwrap();
            if sql.contains("FROM Media JOIN Pool") {
                let pool = sql.find("Pool.Name='").map(|at| sql[at + 11..].split('\'').next().unwrap().to_string());
                let statuses = &sql[sql.find("VolStatus IN (").unwrap()..];
                let statuses = &statuses[..statuses.find(')').unwrap()];
                let mut rows: Vec<&Media> = media.iter()
                    .filter(|m| statuses.contains(&format!("'{}'", m.4)) && pool.as_deref().is_none_or(|p| p == m.3)).collect();
                rows.sort_by_key(|m| (m.3, m.1));
                return Ok(table(rows.into_iter().map(|m| vec![m.0.to_string(), m.1.into(), m.3.into(), m.4.clone(),
                    m.5.to_string(), m.2.to_string()]).collect()));
            }
            if let Some(rest) = sql.strip_prefix("SELECT PoolId FROM Pool WHERE Name='") {
                let name = rest.split('\'').next().unwrap();
                return Ok(table(media.iter().filter(|m| m.3 == name).take(1).map(|m| vec![m.2.to_string()]).collect()));
            }
            if sql.starts_with("SELECT MediaId,VolumeName,PoolId,VolStatus") {
                let id: i64 = sql.rsplit('=').next().unwrap().parse().unwrap();
                return Ok(table(media.iter().filter(|m| m.0 == id).map(|m| {
                    let mut row = vec![m.0.to_string(), m.1.into(), m.2.to_string(), m.4.clone()];
                    row.extend(["100", "1", "1", "1", "1", "0", "0", "0", "0", "0", "2026-01-01 10:00:00",
                        "2026-01-02 10:00:00"].map(String::from));
                    row
                }).collect()));
            }
            let cat = self.cat.lock().unwrap();
            if sql.contains("FROM JobMedia WHERE MediaId IN (") {
                let ids = ids_in(sql, "MediaId IN (");
                return Ok(table(cat.jobs.iter().flat_map(|j| j.media.iter().filter(|m| ids.contains(m))
                    .map(move |m| vec![m.to_string(), j.id.to_string()])).collect()));
            }
            if sql.starts_with("SELECT JobId,JobFiles FROM Job WHERE JobId IN (") {
                let ids = ids_in(sql, "JobId IN (");
                return Ok(table(cat.jobs.iter().filter(|j| ids.contains(&i64::from(j.id)))
                    .map(|j| vec![j.id.to_string(), j.files.to_string()]).collect()));
            }
            cat.rows(sql)
        }
        fn exec(&self, sql: &str) -> Result<u64, String> {
            self.log.lock().unwrap().push(sql.to_string());
            if sql.starts_with("DELETE FROM Job WHERE JobId IN (") {
                let ids = ids_in(sql, "JobId IN (");
                self.cat.lock().unwrap().jobs.retain(|j| !ids.contains(&i64::from(j.id)));
            }
            if let Some(name) = sql.strip_prefix("UPDATE Media SET VolStatus='Purged' WHERE VolumeName='") {
                let name = name.trim_end_matches('\'');
                self.media.lock().unwrap().iter_mut().filter(|m| m.1 == name).for_each(|m| m.4 = "Purged".into());
            }
            Ok(1)
        }
    }

    fn ctx(api: crate::command_executor::ApiMode) -> crate::command_executor::ExecContext {
        crate::ua_select::tests::context("Director { Name = dir }\nCatalog { Name = MyCatalog; DbName = x }", api)
    }

    /// Runs a request (`line`) on `db`; job 5 runs, pool Secret is forbidden.
    fn run(db: &BulkDb, line: &str, api: crate::command_executor::ApiMode) -> crate::command_queue::CommandResult {
        let ctx = ctx(api);
        let request = bulk_request(&ast(line)).unwrap().unwrap();
        let mut ua = Ua::new(&ctx);
        let ok = purge_volumes(&mut ua, db, &request, &[5], &|pool| pool != "Secret");
        ua.finish(ok)
    }

    use crate::command_executor::ApiMode;

    #[test]
    fn dryrun_changes_nothing_and_lists_volumes_and_counts() {
        let db = BulkDb::new();
        let result = run(&db, "purge volume all pool=Full dryrun", ApiMode::Human);
        assert!(result.success, "{}", result.error_message);
        assert_eq!(result.text_output, "\
            Volume \"Full-A\" (Pool Full, Full, 2 jobs, 12 files): would purge\n\
            Volume \"Full-B\" (Pool Full, Full, 1 jobs, 5 files): skipped: Volume \"Full-B\" holds the latest Full chain \
            of client \"fd1\" FileSet \"fs\" (JobIds 3); not purged.\n\
            Volume \"Full-C\" (Pool Full, Used, 1 jobs, 5 files): skipped: in use by running job 5\n\
            Volume \"Full-D\" (Pool Full, Error, 0 jobs, 0 files): would purge\n\
            Dry run: 4 volumes selected, 2 would be purged, 2 skipped. Nothing was changed.\n");
        assert!(db.writes().is_empty(), "{:?}", db.writes());
        assert_eq!(db.jobs(), vec![1, 2, 3, 5, 7, 8, 9]);
    }

    #[test]
    fn yes_purges_only_the_eligible_volumes() {
        let db = BulkDb::new();
        let result = run(&db, "purge volume all pool=Full yes", ApiMode::Human);
        assert!(result.success, "{}", result.error_message);
        assert!(result.text_output.contains("Volume \"Full-A\" (Pool Full, Full, 2 jobs, 12 files): purged\n"), "{}", result.text_output);
        assert!(result.text_output.contains("Volume \"Full-B\" (Pool Full, Full, 1 jobs, 5 files): skipped: Volume \"Full-B\" \
            holds the latest Full chain"), "{}", result.text_output);
        assert!(result.text_output.contains("(Pool Full, Used, 1 jobs, 5 files): skipped: in use by running job 5\n"));
        assert!(result.text_output.contains("Volume \"Full-D\" (Pool Full, Error, 0 jobs, 0 files): purged\n"));
        assert!(result.text_output.ends_with("4 volumes selected, 2 purged, 2 skipped.\n"), "{}", result.text_output);
        // jobs 1 and 2 are gone, protected 3 and running 5 stay
        assert_eq!(db.jobs(), vec![3, 5, 7, 8, 9]);
        assert_eq!((db.status("Full-A"), db.status("Full-B"), db.status("Full-C"), db.status("Full-D")),
            ("Purged".into(), "Full".into(), "Used".into(), "Purged".into()));
        // the same statements as `purge volume=Full-A yes`, one transaction per volume
        let writes = db.writes();
        assert!(writes.contains(&"DELETE FROM Job WHERE JobId IN (1,2)".to_string()), "{writes:?}");
        assert!(!writes.iter().any(|w| w.contains("(3)") || w.contains("(5)") || w.contains("Full-B") || w.contains("Full-C")),
            "{writes:?}");
        // Append and Archive volumes (not in the default filter) and Purged ones are untouched
        assert_eq!((db.status("Full-E"), db.status("Full-G"), db.status("Full-H")),
            ("Append".into(), "Archive".into(), "Purged".into()));
    }

    #[test]
    fn volstatus_filter_and_refusal_of_the_single_form() {
        let db = BulkDb::new();
        let result = run(&db, "purge volume all pool=Full volstatus=Append,Archive yes", ApiMode::Human);
        assert_eq!(result.text_output, "\
            Volume \"Full-E\" (Pool Full, Append, 0 jobs, 0 files): purged\n\
            Volume \"Full-G\" (Pool Full, Archive, 1 jobs, 5 files): skipped: Volume \"Full-G\" has VolStatus \"Archive\" \
            and cannot be purged. The VolStatus must be: Append, Full, Used or Error to be purged.\n\
            2 volumes selected, 1 purged, 1 skipped.\n");
        assert_eq!((db.status("Full-E"), db.status("Full-G"), db.status("Full-A")),
            ("Purged".into(), "Archive".into(), "Full".into()));
        assert_eq!(db.jobs(), vec![1, 2, 3, 5, 7, 8, 9]);
        // nothing matches: a message, no failure
        let result = run(&db, "purge volume all pool=Incremental volstatus=Error dryrun", ApiMode::Human);
        assert!(result.success);
        assert_eq!(result.text_output, "No volumes with VolStatus Error found in pool Incremental.\n\
            Dry run: 0 volumes selected, 0 would be purged, 0 skipped. Nothing was changed.\n");
    }

    #[test]
    fn allpools_versus_pool_and_the_pool_acl() {
        let db = BulkDb::new();
        let all = run(&db, "purge volume all allpools dryrun", ApiMode::Human).text_output;
        assert!(all.contains("Volume \"Inc-F\" (Pool Incremental, Full, 1 jobs, 5 files): would purge\n"), "{all}");
        assert!(all.contains("Volume \"Full-A\""), "{all}");
        // forbidden pool Secret: left out of allpools, unknown when named
        assert!(!all.contains("Sec-1") && !all.contains("Secret"), "{all}");
        assert!(all.ends_with("Dry run: 5 volumes selected, 3 would be purged, 2 skipped. Nothing was changed.\n"), "{all}");
        let one = run(&db, "purge volume all pool=Incremental dryrun", ApiMode::Human).text_output;
        assert_eq!(one, "Volume \"Inc-F\" (Pool Incremental, Full, 1 jobs, 5 files): would purge\n\
            Dry run: 1 volumes selected, 1 would be purged, 0 skipped. Nothing was changed.\n");
        for pool in ["Secret", "Nosuch"] {
            let result = run(&db, &format!("purge volume all pool={pool} yes"), ApiMode::Human);
            assert!(!result.success);
            assert_eq!(result.error_message, format!("Could not find Pool \"{pool}\": ERR=Pool record not found in Catalog.\n"));
        }
        assert!(db.writes().is_empty());
        assert_eq!(db.status("Sec-1"), "Full");
    }

    #[test]
    fn json_result_in_api_mode_2() {
        let db = BulkDb::new();
        let result = run(&db, "purge volume all pool=Full volstatus=Full dryrun", ApiMode::Json);
        assert_eq!(result.json.unwrap(), json!({ "purge": {
            "mode": "dryrun", "pool": "Full", "allpools": false, "volstatus": ["Full"],
            "volumes": [
                {"volume": "Full-A", "pool": "Full", "volstatus": "Full", "jobs": 2, "files": 12, "bytes": 1000,
                 "action": "would_purge", "reason": null},
                {"volume": "Full-B", "pool": "Full", "volstatus": "Full", "jobs": 1, "files": 5, "bytes": 2000,
                 "action": "skipped", "reason": "Volume \"Full-B\" holds the latest Full chain of client \"fd1\" \
                 FileSet \"fs\" (JobIds 3); not purged."}],
            "summary": {"selected": 2, "purged": 1, "skipped": 1}}}));
        let result = run(&db, "purge volume all allpools volstatus=Error yes", ApiMode::Json);
        let json = result.json.unwrap();
        assert_eq!(json["purge"]["mode"], "yes");
        assert_eq!(json["purge"]["pool"], Value::Null);
        assert_eq!(json["purge"]["allpools"], true);
        assert_eq!(json["purge"]["volumes"][0]["action"], "purged");
    }

    /// The selection changed between the query and the purge: skipped.
    #[test]
    fn a_volume_changed_since_the_selection_is_skipped() {
        let db = BulkDb::new();
        let ctx = ctx(ApiMode::Human);
        let request = bulk_request(&ast("purge volume all pool=Full yes")).unwrap().unwrap();
        let mut moved = BulkVolume { media_id: 10, name: "Full-A".into(), pool: "Full".into(), pool_id: 2,
            status: "Full".into(), jobids: vec![1, 2], files: 0, bytes: 0, action: BulkAction::Skipped, reason: None };
        let ua = Ua::new(&ctx);
        purge_one(&ua, &db, &request, &mut moved, &[]);
        assert_eq!(moved.reason.as_deref(), Some("no longer in pool Full"));
        db.media.lock().unwrap()[0].4 = "Append".into();
        let mut changed = BulkVolume { pool_id: 1, reason: None, ..moved };
        purge_one(&ua, &db, &request, &mut changed, &[]);
        assert_eq!(changed.reason.as_deref(), Some("VolStatus changed to Append"));
        assert!(db.writes().is_empty());
    }
}
