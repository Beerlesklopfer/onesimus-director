//! Catalog work of `prune` and `purge` (ua_prune.cc, ua_purge.cc and the
//! catalog functions of cats/sql_delete.cc, sql_create.cc, sql_update.cc).
//! The dialogs live in admin.rs; everything here runs after the request is
//! complete and asks only the confirmations upstream asks at this point
//! ("Purge (yes/no)? ", "Cleanup orphaned path records (yes/no):").
//! Compatibility log C-030.

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::commands::bvfs::Db;
use crate::ua_select::{find_arg_with_value, Ua};

/// C++ EscapeString (standard_conforming_strings on).
fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// First column of every row (DbListHandler; NULL rows are skipped).
fn column0(db: &dyn Db, sql: &str) -> Result<Vec<String>, String> {
    Ok(db.rows(sql)?.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect())
}

/// `ua->db->strerror()` after a failed statement.
fn db_error(error: &str) -> String {
    if error.ends_with('\n') { error.to_string() } else { format!("{error}\n") }
}

/// Seconds since the epoch (C++ time(NULL)).
fn now() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// C++ bstrutime(): local time "%Y-%m-%d %H:%M:%S".
fn bstrutime(t: i64) -> String {
    crate::localtime::strftime("%Y-%m-%d %H:%M:%S", t as libc::time_t)
}

/// C++ StrToUtime(): "YYYY-MM-DD HH:MM:SS" in local time, 0 when empty or
/// not parseable.
fn str_to_utime(text: &str) -> i64 {
    let numbers: Vec<i32> = text.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty())
        .take(6).filter_map(|p| p.parse().ok()).collect();
    if numbers.len() < 6 {
        return 0;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = numbers[0] - 1900;
    tm.tm_mon = numbers[1] - 1;
    tm.tm_mday = numbers[2];
    tm.tm_hour = numbers[3];
    tm.tm_min = numbers[4];
    tm.tm_sec = numbers[5];
    tm.tm_isdst = -1;
    let t = unsafe { libc::mktime(&mut tm) };
    if t < 0 { 0 } else { t as i64 }
}

/// lib/edit.cc edit_uint64_with_commas().
fn with_commas(v: u64) -> String {
    crate::sql_list::add_commas(&v.to_string())
}

/// JobIds of the jobs the Director is running (foreach_jcr).
fn running_jobids(ctx: &ExecContext) -> Vec<u32> {
    ctx.job_manager.as_ref()
        .and_then(|m| m.lock().ok().map(|m| m.running_jobs().into_iter().map(|(id, _, _)| id).collect()))
        .unwrap_or_default()
}

/// C++ ExcludeRunningJobsFromList(): running jobs and JobId 0 are dropped.
pub(crate) fn exclude_running_jobs(ctx: &ExecContext, list: &mut Vec<u32>) -> usize {
    let running = running_jobids(ctx);
    list.retain(|id| *id != 0 && !running.contains(id));
    list.len()
}

/// C++ PrepareJobidsTobedeleted(): the console's own JobId (0) and zeros go.
fn prepare_jobids(list: &mut Vec<u32>) -> String {
    list.retain(|id| *id != 0);
    list.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

fn parse_ids(values: Vec<String>) -> Vec<u32> {
    values.iter().map(|v| crate::commands::job::str_to_int64(v) as u32).collect()
}

/// BareosDb::PurgeFiles().
pub(crate) fn purge_files(db: &dyn Db, jobids: &str) {
    if jobids.is_empty() {
        return;
    }
    let _ = db.exec(&format!("DELETE FROM File WHERE JobId IN ({jobids})"));
    let _ = db.exec(&format!("DELETE FROM BaseFiles WHERE JobId IN ({jobids})"));
    let _ = db.exec(&format!("UPDATE Job SET PurgedFiles=1 WHERE JobId IN ({jobids})"));
}

/// BareosDb::PurgeJobs(): all records of the jobs, copies upgraded
/// (UpgradeCopies) before the Job rows go.
pub(crate) fn purge_jobs(db: &dyn Db, jobids: &str) {
    if jobids.is_empty() {
        return;
    }
    purge_files(db, jobids);
    for table in ["JobMedia", "Log", "RestoreObject", "PathVisibility", "NDMPJobEnvironment", "JobStats"] {
        let _ = db.exec(&format!("DELETE FROM {table} WHERE JobId IN ({jobids})"));
    }
    // UpgradeCopies (sql_update.cc:553, query 0040_uap_upgrade_copies_oldest_job).
    let _ = db.exec(&format!(
        "CREATE TEMPORARY TABLE cpy_tmp AS\nSELECT MIN(JobId) AS JobId\n  FROM Job\n WHERE Type = 'C'\n   AND (\n          \
         PriorJobId IN ({jobids})\n       OR PriorJobId IN (\n         SELECT PriorJobId\n           FROM Job\n          \
         WHERE JobId IN ({jobids})\n            AND Type = 'B'\n          )\n       )\n GROUP BY PriorJobId\n"));
    let _ = db.exec("UPDATE Job SET Type='B' WHERE JobId IN ( SELECT JobId FROM cpy_tmp )");
    let _ = db.exec("DELETE FROM Job WHERE Type='c' AND priorjobid IN ( SELECT JobId FROM cpy_tmp )");
    let _ = db.exec("DROP TABLE IF EXISTS cpy_tmp");
    let _ = db.exec(&format!("DELETE FROM Job WHERE JobId IN ({jobids})"));
}

/// C++ PurgeJobListFromCatalog().
pub(crate) fn purge_job_list_from_catalog(ua: &mut Ua, db: &dyn Db, list: &mut Vec<u32>) {
    let jobids = prepare_jobids(list);
    if list.is_empty() {
        ua.send_msg("No jobids found to be purged\n");
        return;
    }
    ua.send_msg(format!("Purging the following {} JobIds: {jobids}\n", list.len()));
    purge_jobs(db, &jobids);
}

/// C++ PurgeFilesFromJobList().
fn purge_files_from_job_list(db: &dyn Db, list: &mut Vec<u32>) {
    let jobids = prepare_jobids(list);
    purge_files(db, &jobids);
}

/// The Media record fields prune and purge work with (MediaDbRecord).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Volume {
    pub media_id: i64,
    pub name: String,
    pub pool_id: i64,
    pub status: String,
    pub retention: u64,
    pub enabled: i64,
    pub vol_jobs: i64,
    pub vol_files: i64,
    pub vol_bytes: u64,
    pub max_vol_bytes: u64,
    pub max_vol_jobs: i64,
    pub max_vol_files: i64,
    pub use_duration: u64,
    pub recycle_pool_id: i64,
    pub first_written: i64,
    pub last_written: i64,
    /// cLastWritten (text as stored).
    pub c_last_written: String,
}

const VOL_ARCHIVED: i64 = 2;

/// BareosDb::GetMediaRecord() by MediaId.
pub(crate) fn get_volume(db: &dyn Db, media_id: i64) -> Result<Volume, String> {
    let table = db.rows(&format!(
        "SELECT MediaId,VolumeName,PoolId,VolStatus,VolRetention,Enabled,VolJobs,VolFiles,VolBytes,MaxVolBytes,\
         MaxVolJobs,MaxVolFiles,VolUseDuration,RecyclePoolId,FirstWritten,LastWritten FROM Media WHERE MediaId={media_id}"))?;
    let Some(row) = table.rows.into_iter().next() else {
        return Err(format!("Media record MediaId={media_id} not found.\n"));
    };
    let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let int = |i: usize| text(i).parse::<i64>().unwrap_or(0);
    let uint = |i: usize| text(i).parse::<u64>().unwrap_or(0);
    Ok(Volume {
        media_id: int(0), name: text(1), pool_id: int(2), status: text(3), retention: uint(4), enabled: int(5),
        vol_jobs: int(6), vol_files: int(7), vol_bytes: uint(8), max_vol_bytes: uint(9), max_vol_jobs: int(10),
        max_vol_files: int(11), use_duration: uint(12), recycle_pool_id: int(13),
        first_written: str_to_utime(&text(14)), last_written: str_to_utime(&text(15)), c_last_written: text(15),
    })
}

/// MediaId of a volume name (GetMediaRecord by VolumeName).
pub(crate) fn get_volume_by_name(db: &dyn Db, name: &str) -> Result<Volume, String> {
    let ids = column0(db, &format!("SELECT MediaId FROM Media WHERE VolumeName='{}'", esc(name)))?;
    match ids.first() {
        Some(id) => get_volume(db, id.parse().unwrap_or(0)),
        None => Err(format!("Media record for Volume \"{name}\" not found.\n")),
    }
}

/// Pool name and counters (GetPoolRecord by PoolId).
fn pool_record(db: &dyn Db, pool_id: i64) -> Option<(String, i64, i64)> {
    let table = db.rows(&format!("SELECT Name,MaxVols,NumVols FROM Pool WHERE PoolId={pool_id}")).ok()?;
    let row = table.rows.into_iter().next()?;
    let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    Some((text(0), text(1).parse().unwrap_or(0), text(2).parse().unwrap_or(0)))
}

/// BareosDb::CreateClientRecord(): the ClientId, the record is created when
/// missing (Uname '', AutoPrune 0, retentions 0).
fn create_client_record(db: &dyn Db, name: &str) -> Option<i64> {
    let select = format!("SELECT ClientId FROM Client WHERE Name='{}'", esc(name));
    if let Some(id) = column0(db, &select).ok().and_then(|ids| ids.first().and_then(|id| id.parse().ok())) {
        return Some(id);
    }
    db.exec(&format!("INSERT INTO Client (Name,Uname,AutoPrune,FileRetention,JobRetention) VALUES ('{}','',0,0,0)",
        esc(name))).ok()?;
    column0(db, &select).ok().and_then(|ids| ids.first().and_then(|id| id.parse().ok()))
}

/// Name of the client's catalog (client->catalog->resource_name_).
fn client_catalog(ctx: &ExecContext, client: &str) -> String {
    ctx.config.as_ref().and_then(|c| c.read().ok()).map(|cfg| {
        cfg.find_client(client).map(|c| c.catalog.clone()).filter(|c| !c.is_empty())
            .or_else(|| cfg.catalogs.first().map(|c| c.name.clone())).unwrap_or_default()
    }).unwrap_or_default()
}

/// C++ GetUserJobStatusSelection(): None for an invalid `jobstatus=`.
fn job_status_selection(ast: &CommandAst) -> Option<Vec<char>> {
    let Some(value) = find_arg_with_value(ast, "jobstatus") else { return Some(Vec::new()) };
    if value.is_empty() {
        return None;
    }
    value.split(',').filter(|s| !s.is_empty()).map(|status| {
        let first = status.chars().next()?;
        if status.len() == 1 && ('A'..='z').contains(&first) {
            return Some(first);
        }
        [("terminated", 'T'), ("warnings", 'W'), ("canceled", 'A'), ("running", 'R'), ("error", 'E'), ("fatal", 'f')]
            .iter().find(|(name, _)| status.eq_ignore_ascii_case(name)).map(|(_, code)| *code)
    }).collect()
}

/// CreateDelimitedStringForSqlQueries(): 'A','B'.
fn status_list(statuses: &[char]) -> String {
    statuses.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")
}

// --- purge -------------------------------------------------------------------------

/// ua_purge.cc PurgeFilesFromClient().
pub(crate) fn purge_files_from_client(ua: &mut Ua, ast: &CommandAst, db: &dyn Db, client: &str) {
    let Some(client_id) = create_client_record(db, client) else { return };
    ua.info_msg(format!("Begin purging files for Client \"{client}\"\n"));
    let mut del = parse_ids(column0(db, &format!(
        "SELECT JobId FROM Job WHERE ClientId={client_id} AND PurgedFiles=0 ORDER BY JobId")).unwrap_or_default());
    let catalog = client_catalog(ua.ctx, client);
    if del.is_empty() {
        ua.warning_msg(format!("No Files found for client {client} to purge from {catalog} catalog.\n"));
    } else {
        ua.info_msg(format!("Found Files in {} Jobs for client \"{client}\" in catalog \"{catalog}\".\n", del.len()));
        if !ua.get_confirmation(ast, "Purge (yes/no)? ", false) {
            ua.info_msg("Purge canceled.\n");
        } else {
            purge_files_from_job_list(db, &mut del);
        }
    }
}

/// ua_purge.cc PurgeJobsFromClient().
pub(crate) fn purge_jobs_from_client(ua: &mut Ua, ast: &CommandAst, db: &dyn Db, client: &str) {
    let Some(client_id) = create_client_record(db, client) else { return };
    ua.info_msg(format!("Begin purging jobs from Client \"{client}\"\n"));
    let Some(statuses) = job_status_selection(ast) else {
        ua.error_msg("invalid jobstatus parameter\n");
        return;
    };
    let mut query = format!("SELECT JobId, PurgedFiles FROM Job WHERE ClientId={client_id} ");
    if !statuses.is_empty() {
        query.push_str(&format!("AND JobStatus IN ({}) ", status_list(&statuses)));
    }
    query.push_str("ORDER BY JobId");
    let mut list = parse_ids(column0(db, &query).unwrap_or_default());
    let catalog = client_catalog(ua.ctx, client);
    if list.is_empty() {
        ua.warning_msg(format!("No Jobs found for client {client} to purge from {catalog} catalog.\n"));
    } else {
        ua.info_msg(format!("Found {} Jobs for client \"{client}\" in catalog \"{catalog}\".\n", list.len()));
        if !ua.get_confirmation(ast, "Purge (yes/no)? ", false) {
            ua.info_msg("Purge canceled.\n");
        } else {
            purge_job_list_from_catalog(ua, db, &mut list);
            let n = list.len();
            ua.info_msg(format!("{n} Job{} on client \"{client}\" purged from catalog.\n", if n <= 1 { "" } else { "s" }));
        }
    }
}

/// ua_purge.cc PurgeJobsFromPool().
pub(crate) fn purge_jobs_from_pool(ua: &mut Ua, ast: &CommandAst, db: &dyn Db, pool: &str) {
    let Some(statuses) = job_status_selection(ast) else {
        ua.error_msg("invalid jobstatus parameter\n");
        return;
    };
    let pool_id = column0(db, &format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(pool))).ok()
        .and_then(|ids| ids.first().and_then(|id| id.parse::<i64>().ok())).unwrap_or(0);
    let mut query = format!("SELECT DISTINCT JobMedia.JobId FROM JobMedia, Job WHERE Job.JobId = JobMedia.JobId \
        AND Jobmedia.MediaId IN (SELECT MediaId FROM Media WHERE PoolId={pool_id}) ");
    if !statuses.is_empty() {
        query.push_str(&format!("AND Job.JobStatus IN ({}) ", status_list(&statuses)));
    }
    query.push_str("ORDER BY JobId");
    let mut list = parse_ids(column0(db, &query).unwrap_or_default());
    if list.is_empty() {
        ua.warning_msg(format!("No Jobs found for pool {pool} to purge from catalog.\n"));
    } else {
        ua.info_msg(format!("Found {} Jobs for pool \"{pool}\" in catalog.\n", list.len()));
        if !ua.get_confirmation(ast, "Purge (yes/no)? ", false) {
            ua.info_msg("Purge canceled.\n");
        } else {
            purge_job_list_from_catalog(ua, db, &mut list);
            let n = list.len();
            ua.info_msg(format!("{n} Job{} on Pool \"{pool}\" purged from catalog.\n", if n <= 1 { "" } else { "s" }));
        }
    }
    // GetMediaIdsInPool() returns true on success, and upstream warns then.
    match column0(db, &format!("SELECT DISTINCT MediaId FROM Media WHERE PoolId={pool_id}")) {
        Ok(ids) => {
            ua.warning_msg(format!("No Media found for pool {pool}.\n"));
            for id in ids {
                match get_volume(db, id.parse().unwrap_or(0)) {
                    Ok(mut volume) => { is_volume_purged(ua, db, &mut volume, false); }
                    Err(error) => ua.error_msg(db_error(&error)),
                }
            }
        }
        Err(_) => {}
    }
}

/// ua_purge.cc PurgeQuotaFromClient().
pub(crate) fn purge_quota_from_client(ua: &mut Ua, db: &dyn Db, client: &str) {
    let Some(client_id) = create_client_record(db, client) else { return };
    let exists = db.rows(&format!("SELECT ClientId FROM Quota WHERE ClientId='{client_id}'"))
        .is_ok_and(|t| t.rows.len() == 1);
    if !exists && db.exec(&format!("INSERT INTO Quota (ClientId, GraceTime, QuotaLimit) VALUES ('{client_id}', '0', 0)"))
        .map_or(true, |n| n != 1) {
        return;
    }
    if db.exec(&format!("UPDATE Quota SET GraceTime='0', QuotaLimit='0' WHERE ClientId='{client_id}'"))
        .map_or(true, |n| n == 0) {
        return;
    }
    ua.info_msg(format!("Purged quota for Client \"{client}\"\n"));
}

/// ua_purge.cc PurgeJobsFromVolume(); true when the volume is purged.
pub(crate) fn purge_jobs_from_volume(ua: &mut Ua, ast: &CommandAst, db: &dyn Db, volume: &mut Volume, force: bool) -> bool {
    if !matches!(volume.status.as_str(), "Append" | "Full" | "Used" | "Error") {
        ua.error_msg(format!("\nVolume \"{}\" has VolStatus \"{}\" and cannot be purged.\n\
            The VolStatus must be: Append, Full, Used or Error to be purged.\n", volume.name, volume.status));
        return false;
    }
    let Some(statuses) = job_status_selection(ast) else {
        ua.error_msg("invalid jobstatus parameter\n");
        return false;
    };
    // `jobid=<list>` names the jobs, but leaves the counted list empty
    // (upstream: "No Jobs found" and nothing is purged).
    let mut list: Vec<String> = Vec::new();
    let jobids = match find_arg_with_value(ast, "jobid").filter(|v| is_a_number_list(v)) {
        Some(jobids) => jobids.to_string(),
        None if !statuses.is_empty() => {
            list = column0(db, &format!("SELECT DISTINCT JobMedia.JobId FROM JobMedia, Job WHERE JobMedia.MediaId={} \
                AND Job.JobId = JobMedia.JobId AND Job.JobStatus in ({})", volume.media_id, status_list(&statuses)))
                .unwrap_or_default();
            list.join(",")
        }
        None => {
            match column0(db, &format!("SELECT DISTINCT JobId FROM JobMedia WHERE MediaId={}", volume.media_id)) {
                Ok(ids) => list = ids,
                Err(error) => {
                    ua.error_msg(db_error(&error));
                    return false;
                }
            }
            list.join(",")
        }
    };
    if list.is_empty() {
        ua.warning_msg(format!("No Jobs found for media {} to purge from catalog.\n", volume.name));
    } else {
        ua.info_msg(format!("Found {} Jobs for media \"{}\" in catalog.\n", list.len(), volume.name));
        if !ua.get_confirmation(ast, "Purge (yes/no)? ", true) {
            ua.info_msg("Purge canceled.\n");
        } else {
            purge_jobs(db, &jobids);
            let n = list.len();
            ua.info_msg(format!("{n} Jobs{} on Volume \"{}\" purged from catalog.\n", if n <= 1 { "" } else { "s" },
                volume.name));
        }
    }
    is_volume_purged(ua, db, volume, force)
}

/// C++ Is_a_number_list().
fn is_a_number_list(text: &str) -> bool {
    let (mut previous_digit, mut digit_seen) = (false, false);
    for c in text.chars() {
        if c.is_ascii_digit() {
            previous_digit = true;
            digit_seen = true;
        } else if c == ',' && previous_digit {
            previous_digit = false;
        } else {
            return false;
        }
    }
    digit_seen
}

/// ua_purge.cc IsVolumePurged().
pub(crate) fn is_volume_purged(ua: &mut Ua, db: &dyn Db, volume: &mut Volume, force: bool) -> bool {
    if !force && (volume.first_written == 0 || volume.last_written == 0) {
        return false;
    }
    if volume.status == "Purged" {
        return true;
    }
    let count = match db.rows(&format!("SELECT 1 FROM JobMedia WHERE MediaId={} LIMIT 1", volume.media_id)) {
        Ok(table) => table.rows.len(),
        Err(error) => {
            ua.error_msg(db_error(&error));
            return false;
        }
    };
    if count != 0 {
        return false;
    }
    ua.warning_msg(format!("There are no more Jobs associated with Volume \"{}\". Marking it purged.\n", volume.name));
    let purged = mark_media_purged(ua, db, volume);
    purged
}

/// ua_purge.cc MarkMediaPurged() (console: no JobId, no message of its own).
fn mark_media_purged(ua: &mut Ua, db: &dyn Db, volume: &mut Volume) -> bool {
    if !matches!(volume.status.as_str(), "Append" | "Full" | "Used" | "Error") {
        ua.error_msg(format!("Cannot purge Volume with VolStatus={}\n", volume.status));
        return volume.status == "Purged";
    }
    volume.status = "Purged".into();
    // UpdateMediaRecord writes back the record read before; only VolStatus changed.
    if let Err(error) = db.exec(&format!("UPDATE Media SET VolStatus='Purged' WHERE VolumeName='{}'", esc(&volume.name))) {
        ua.error_msg(db_error(&error));
        return false;
    }
    // TODO(C-030): GeneratePluginEvent(bDirEventVolumePurged) - no Director plugins.
    if volume.recycle_pool_id != 0 && volume.recycle_pool_id != volume.pool_id {
        match (pool_record(db, volume.pool_id), pool_record(db, volume.recycle_pool_id)) {
            (Some(_), Some((name, max_vols, num_vols))) => {
                if max_vols > 0 && num_vols >= max_vols {
                    ua.error_msg(format!("Unable move recycled Volume in full Pool \"{name}\" MaxVols={max_vols}\n"));
                } else {
                    update_vol_pool(ua, db, volume, &name);
                }
            }
            _ => ua.error_msg("\n"),
        }
    }
    true
}

/// ua_update.cc UpdateVolPool(): move the volume, recount both pools
/// (UpdatePoolRecord sets NumVols from the Media rows).
fn update_vol_pool(ua: &mut Ua, db: &dyn Db, volume: &mut Volume, pool: &str) {
    let old = volume.pool_id;
    let Some(new) = column0(db, &format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(pool))).ok()
        .and_then(|ids| ids.first().and_then(|id| id.parse::<i64>().ok())) else { return };
    volume.pool_id = new;
    if let Err(error) = db.exec(&format!("UPDATE Media SET PoolId={new} WHERE MediaId={}", volume.media_id)) {
        ua.error_msg(db_error(&error));
        return;
    }
    ua.info_msg(format!("New Pool is: {pool}\n"));
    for id in [old, new] {
        if let Err(error) = db.exec(&format!(
            "UPDATE Pool SET NumVols=(SELECT count(*) FROM Media WHERE PoolId={id}) WHERE PoolId={id}")) {
            ua.error_msg(db_error(&error));
        }
    }
}

// --- prune -------------------------------------------------------------------------

/// prune_set_filter(): FROM joins and WHERE conditions.
fn prune_filter(client: Option<&str>, pool: Option<&str>, period: u64) -> (String, String) {
    let mut from = String::new();
    let mut filter = format!(" AND JobTDate < {} ", now() - period as i64);
    if let Some(client) = client {
        filter.push_str(&format!(" AND Client.Name = '{}' ", esc(client)));
        from.push_str(" JOIN Client USING (ClientId) ");
    }
    if let Some(pool) = pool {
        filter.push_str(&format!(" AND Pool.Name = '{}' ", esc(pool)));
        from.push_str(" JOIN Pool USING(PoolId) ");
    }
    (from, filter)
}

/// ua_prune.cc PruneFiles() with the confirmed retention.
pub(crate) fn prune_files(ua: &mut Ua, db: &dyn Db, client: &str, pool: Option<&str>, period: u64) {
    let (from, filter) = prune_filter(Some(client), pool, period);
    ua.send_msg("Begin pruning Files.\n");
    let count = match column0(db, &format!("SELECT COUNT(1) FROM Job {from} WHERE PurgedFiles=0 {filter}")) {
        Ok(rows) => rows.first().map(|c| crate::commands::job::str_to_int64(c)).unwrap_or(0),
        Err(error) => {
            ua.error_msg(db_error(&error));
            return;
        }
    };
    if count == 0 {
        ua.warning_msg("No Files found to prune.\n");
        return;
    }
    let mut list = parse_ids(column0(db, &format!("SELECT JobId FROM Job {from} WHERE PurgedFiles=0 {filter} ORDER BY JobId"))
        .unwrap_or_default());
    purge_files_from_job_list(db, &mut list);
    ua.info_msg(format!("Pruned Files from {} Jobs for client {client} from catalog.\n", with_commas(list.len() as u64)));
}

/// ua_prune.cc PruneJobs() with the confirmed retention.
pub(crate) fn prune_jobs(ua: &mut Ua, db: &dyn Db, client: &str, pool: Option<&str>, period: u64) -> bool {
    let (from, filter) = prune_filter(Some(client), pool, period);
    let drop = |db: &dyn Db| { let _ = db.exec("DROP TABLE IF EXISTS DelCandidates\n"); };
    drop(db);
    for sql in ["CREATE TEMPORARY TABLE\n       DelCandidates ( JobId INTEGER NOT NULL, PurgedFiles SMALLINT, FileSetId INTEGER, JobFiles INTEGER, JobStatus char(1))\n",
                "CREATE INDEX DelInx1 ON DelCandidates (JobId)\n"] {
        if let Err(error) = db.exec(sql) {
            ua.error_msg(db_error(&error));
            drop(db);
            return true;
        }
    }
    ua.send_msg(format!("Begin pruning Jobs older than {}.\n", crate::ua_select::edit_utime(period)));
    if let Err(error) = db.exec(&format!("INSERT INTO DelCandidates SELECT JobId,PurgedFiles,FileSetId,JobFiles,JobStatus \
        FROM Job {from} WHERE Type NOT IN ('A')  {filter} ")) {
        ua.error_msg(db_error(&error));
        drop(db);
        return true;
    }
    // JobSelectHandler: jobs, filesets and clients still configured, no Verify jobs.
    let mut checks: Vec<(i64, i64)> = Vec::new();
    match db.rows("SELECT DISTINCT Job.Name, FileSet, Client.Name, Job.FileSetId, Job.ClientId, Job.Type FROM DelCandidates \
        JOIN Job USING (JobId) JOIN Client USING (ClientId) JOIN FileSet ON (Job.FileSetId = FileSet.FileSetId) \
        WHERE Job.Type IN ('B') AND Job.JobStatus IN ('T', 'W') ") {
        Ok(table) => {
            let config = ua.ctx.config.as_ref().and_then(|c| c.read().ok());
            for row in table.rows {
                let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                let configured = config.as_ref().is_some_and(|cfg| cfg.find_job(&text(0)).is_some()
                    && cfg.find_fileset(&text(1)).is_some() && cfg.find_client(&text(2)).is_some());
                if !configured || text(5).starts_with('V') {
                    continue;
                }
                checks.push((text(3).parse().unwrap_or(0), text(4).parse().unwrap_or(0)));
            }
        }
        Err(error) => ua.error_msg(db_error(&error)),
    }
    // Jobs needed for an accurate restore (AccurateGetJobids, level Incremental).
    let date = bstrutime(now() + 1);
    let mut keep: Vec<String> = Vec::new();
    for (fileset_id, client_id) in checks {
        keep.extend(crate::commands::bvfs::accurate_get_jobids(db, client_id, 'B', &date, fileset_id).unwrap_or_default());
    }
    // The latest Verify InitCatalog job.
    match column0(db, &format!("SELECT JobId, JobTDate FROM Job {from} WHERE Type='V'    AND Level='V'  {filter} \
        ORDER BY JobTDate DESC LIMIT 1")) {
        Ok(ids) => keep.extend(ids),
        Err(error) => ua.error_msg(db_error(&error)),
    }
    if !keep.is_empty() {
        let list = keep.join(",");
        // GetUsedBaseJobids
        if let Ok(base) = column0(db, &format!("SELECT DISTINCT BaseJobId   FROM Job JOIN BaseFiles USING (JobId)  \
            WHERE Job.HasBase = 1    AND Job.JobId IN ({list}) ")) {
            keep.extend(base);
        }
        if let Err(error) = db.exec(&format!("DELETE FROM DelCandidates WHERE JobId IN ({}) AND JobFiles!=0", keep.join(","))) {
            ua.error_msg(db_error(&error));
            drop(db);
            return true;
        }
    }
    let mut list = match column0(db, "SELECT DISTINCT DelCandidates.JobId FROM DelCandidates ORDER BY JobId") {
        Ok(ids) => parse_ids(ids),
        Err(error) => {
            ua.error_msg(db_error(&error));
            Vec::new()
        }
    };
    purge_job_list_from_catalog(ua, db, &mut list);
    if !list.is_empty() {
        ua.info_msg(format!("Pruned {} {} for client {client} from catalog.\n", list.len(),
            if list.len() == 1 { "Job" } else { "Jobs" }));
    }
    drop(db);
    true
}

/// GetPruneCandidatesForVolume(): expired jobs of the volume plus the
/// copy/migration jobs of them, running jobs excluded. None on a query error.
fn prune_candidates(ua: &mut Ua, db: &dyn Db, volume: &Volume) -> Option<Vec<u32>> {
    if volume.enabled == VOL_ARCHIVED {
        return Some(Vec::new());
    }
    let mut list = match column0(db, &format!("SELECT DISTINCT JobMedia.JobId\n   FROM JobMedia, Job\n   WHERE MediaId      = {}\n   \
        AND Job.JobId    = JobMedia.JobId\n   AND Job.JobTDate < {}\n   ORDER BY JobId\n",
        volume.media_id, now() - volume.retention as i64)) {
        Ok(ids) => parse_ids(ids),
        Err(error) => {
            ua.error_msg(db_error(&error));
            return None;
        }
    };
    if !list.is_empty() {
        let ids = list.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        match column0(db, &format!("SELECT JobId from Job WHERE type in ('c','g') AND priorjobid in ({ids})")) {
            Ok(more) => list.extend(parse_ids(more)),
            Err(error) => {
                ua.error_msg(db_error(&error));
                return None;
            }
        }
    }
    exclude_running_jobs(ua.ctx, &mut list);
    Some(list)
}

/// next_vol.cc HasVolumeExpired(): an Append volume past one of its limits
/// becomes Full/Used (catalog updated).
fn has_volume_expired(db: &dyn Db, volume: &mut Volume) -> bool {
    if volume.status != "Append" || volume.vol_jobs <= 0 {
        return false;
    }
    // TODO(C-030): the Jmsg(M_INFO) "Max Volume ... exceeded. Marking Volume
    // ..." of the console JCR is not delivered.
    let status = if volume.max_vol_bytes > 0 && volume.vol_bytes >= volume.max_vol_bytes {
        "Full"
    } else if (volume.max_vol_jobs > 0 && volume.max_vol_jobs <= volume.vol_jobs)
        || (volume.max_vol_files > 0 && volume.max_vol_files <= volume.vol_files)
        || (volume.use_duration > 0 && volume.use_duration as i64 <= now() - volume.first_written) {
        "Used"
    } else {
        return false;
    };
    volume.status = status.into();
    let _ = db.exec(&format!("UPDATE Media SET VolStatus='{status}' WHERE VolumeName='{}'", esc(&volume.name)));
    true
}

/// ua_prune.cc PruneVolume(); true when the volume is empty afterwards.
pub(crate) fn prune_volume(ua: &mut Ua, db: &dyn Db, volume: &mut Volume) -> bool {
    if volume.enabled == VOL_ARCHIVED {
        return false;
    }
    let empty = volume.first_written == 0 && volume.last_written == 0;
    if !(volume.status == "Full" || volume.status == "Used" || has_volume_expired(db, volume)) {
        ua.send_msg(format!("Pruning volume {}: cannot prune as Volstatus is {} but needs to be Full or Used.\n",
            volume.name, volume.status));
        return false;
    }
    if empty {
        ua.send_msg(format!("Volume \"{0}\" seems to be empty.  Make sure that it is not currently in use, \
            then call 'purge volume=\"{0}\"'", volume.name));
        return false;
    }
    // GetPruneListForVolume()
    let mut list = prune_candidates(ua, db, volume).unwrap_or_default();
    if !list.is_empty() {
        ua.send_msg(format!("Volume \"{}\" has Volume Retention of {} sec. and has {} jobs that will be pruned\n",
            volume.name, volume.retention as i64, list.len()));
    }
    let count = list.len();
    ua.send_msg(format!("Pruning volume {}: {count} Jobs have expired and can be pruned.\n", volume.name));
    if count != 0 {
        purge_job_list_from_catalog(ua, db, &mut list);
    }
    let now_empty = is_volume_purged(ua, db, volume, false);
    if now_empty {
        ua.send_msg(format!("Volume \"{}\" contains no jobs after pruning.\n", volume.name));
    } else {
        ua.send_msg(format!("Volume \"{}\" still contains jobs after pruning.\n", volume.name));
    }
    now_empty
}

/// One volume of GetPoolPruneReport().
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrunableVolume {
    pub name: String,
    pub last_written: String,
    pub jobs: usize,
    pub bytes: u64,
}

/// ua_prune.cc GetPoolPruneReport(): Full/Used volumes of the pool with
/// expired jobs, oldest data first. None on a catalog error.
pub(crate) fn pool_prune_report(ua: &mut Ua, db: &dyn Db, pool_id: i64) -> Option<Vec<PrunableVolume>> {
    let ids = column0(db, &format!("SELECT MediaId FROM Media WHERE PoolId={pool_id} ORDER BY MediaId")).ok()?;
    let mut volumes: Vec<(Volume, Vec<u32>)> = Vec::new();
    for id in ids {
        let volume = get_volume(db, id.parse().unwrap_or(0)).ok()?;
        if volume.enabled == VOL_ARCHIVED || !(volume.status == "Full" || volume.status == "Used") {
            continue;
        }
        let mut jobs = prune_candidates(ua, db, &volume)?;
        if jobs.is_empty() {
            continue;
        }
        jobs.sort_unstable();
        jobs.dedup();
        volumes.push((volume, jobs));
    }
    let mut bytes = std::collections::HashMap::new();
    let mut all: Vec<u32> = volumes.iter().flat_map(|(_, jobs)| jobs.iter().copied()).collect();
    all.sort_unstable();
    all.dedup();
    if !all.is_empty() {
        let list = all.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let table = db.rows(&format!("SELECT JobId, Name, JobBytes, StartTime FROM Job WHERE JobId IN ({list}) ORDER BY JobId"))
            .ok()?;
        for row in table.rows {
            let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
            bytes.insert(crate::commands::job::str_to_int64(&text(0)) as u32, text(2).parse::<u64>().unwrap_or(0));
        }
    }
    let mut report: Vec<PrunableVolume> = volumes.into_iter().map(|(volume, jobs)| PrunableVolume {
        bytes: jobs.iter().filter_map(|id| bytes.get(id)).sum(),
        name: volume.name,
        last_written: volume.c_last_written,
        jobs: jobs.len(),
    }).collect();
    report.sort_by(|a, b| match (a.last_written.is_empty(), b.last_written.is_empty()) {
        _ if a.last_written == b.last_written => a.name.cmp(&b.name),
        (true, _) => std::cmp::Ordering::Greater,
        (_, true) => std::cmp::Ordering::Less,
        _ => a.last_written.cmp(&b.last_written),
    });
    Some(report)
}

/// The report line of a prunable volume.
pub(crate) fn report_line(index: usize, volume: &PrunableVolume) -> String {
    format!("  {index}: {} | lastwritten={} | jobs={} | bytes={}\n", volume.name, volume.last_written,
        with_commas(volume.jobs as u64), crate::commands::status::edit_uint64_with_suffix(volume.bytes))
}

/// ua_prune.cc PruneStats().
pub(crate) fn prune_stats(ua: &mut Ua, db: &dyn Db, retention: u64) {
    let now = now();
    let _ = db.exec(&format!("DELETE FROM JobHisto WHERE JobTDate < {}", now - retention as i64));
    ua.info_msg("Pruned Jobs from JobHisto in catalog.\n");
    let dt = bstrutime(now - retention as i64);
    let _ = db.exec(&format!("DELETE FROM DeviceStats WHERE SampleTime < '{dt}'"));
    ua.info_msg("Pruned Statistics from DeviceStats in catalog.\n");
    let _ = db.exec(&format!("DELETE FROM JobStats WHERE SampleTime < '{dt}'"));
    ua.info_msg("Pruned Statistics from JobStats in catalog.\n");
}

/// The SQL part of ua_prune.cc PruneDirectory() after the client and the
/// path are known.
pub(crate) fn prune_directory(ua: &mut Ua, db: &dyn Db, client: Option<&str>, path: &str, recursive: bool) -> bool {
    let mut path = path.to_string();
    if !path.ends_with('/') {
        path.push('/');
    }
    let topdir = esc(&path);
    let pattern = if recursive { format!("{topdir}%") } else { topdir };
    let mut query = format!("DELETE FROM file WHERE pathid IN (SELECT pathid FROM path WHERE path LIKE '{pattern}')");
    if let Some(client) = client {
        let Some(client_id) = create_client_record(db, client) else { return false };
        query.push_str(&format!(" AND JobId IN (SELECT JobId FROM Job WHERE ClientId={client_id})"));
    }
    let _ = db.exec(&query);
    if client.is_none() {
        if ua.get_yesno("Cleanup orphaned path records (yes/no):") != Some(true) {
            return true;
        }
        let _ = db.exec(&format!("DELETE FROM path WHERE path LIKE '{pattern}'"));
    }
    true
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::catalog_worker::{SqlColumn, SqlTable};
    use std::sync::Mutex;

    /// Catalog double: answers by the first matching SQL fragment, records
    /// every statement (writes too).
    pub(crate) struct FakeDb {
        pub answers: Vec<(&'static str, Vec<Vec<&'static str>>)>,
        pub log: Mutex<Vec<String>>,
    }

    impl FakeDb {
        pub(crate) fn new(answers: Vec<(&'static str, Vec<Vec<&'static str>>)>) -> Self {
            Self { answers, log: Mutex::new(Vec::new()) }
        }
        pub(crate) fn writes(&self) -> Vec<String> {
            self.log.lock().unwrap().iter().filter(|s| !s.trim_start().starts_with("SELECT")).cloned().collect()
        }
    }

    impl Db for FakeDb {
        fn rows(&self, sql: &str) -> Result<SqlTable, String> {
            self.log.lock().unwrap().push(sql.to_string());
            let rows = self.answers.iter().find(|(fragment, _)| sql.contains(fragment))
                .map(|(_, rows)| rows.clone()).unwrap_or_default();
            Ok(SqlTable {
                columns: vec![SqlColumn { name: "c".into(), numeric: false }],
                rows: rows.into_iter().map(|r| r.into_iter().map(|v| Some(v.to_string())).collect()).collect(),
            })
        }
        fn exec(&self, sql: &str) -> Result<u64, String> {
            self.log.lock().unwrap().push(sql.to_string());
            Ok(1)
        }
    }

    fn ctx() -> ExecContext {
        crate::ua_select::tests::context(
            "Director { Name = dir }\nCatalog { Name = MyCatalog; DbName = x }\nClient { Name = fd1; Password = x }\n\
             Pool { Name = Full; Pool Type = Backup }",
            crate::command_executor::ApiMode::Human)
    }

    fn ast(line: &str) -> CommandAst {
        crate::session::parse_cli_command(line)
    }

    const MEDIA: &str = "FROM Media WHERE MediaId=";

    fn volume_row(status: &'static str) -> Vec<&'static str> {
        vec!["7", "Full-0007", "1", status, "100", "1", "3", "1", "1000", "0", "0", "0", "0", "0",
             "2026-01-01 10:00:00", "2026-01-02 10:00:00"]
    }

    #[test]
    fn purge_jobs_from_client_asks_before_deleting() {
        let ctx = ctx();
        let run = |line: &str, answers: &[&str]| {
            let db = FakeDb::new(vec![("FROM Client WHERE Name", vec![vec!["4"]]),
                                      ("FROM Job WHERE ClientId=4", vec![vec!["11", "0"], vec!["12", "1"]])]);
            let (_, console) = crate::ua_select::tests::scripted(answers, || {
                let mut ua = Ua::new(&ctx);
                purge_jobs_from_client(&mut ua, &ast(line), &db, "fd1");
                ua.finish(true);
            });
            (console.transcript(), db.writes())
        };
        let head = "Begin purging jobs from Client \"fd1\"\nFound 2 Jobs for client \"fd1\" in catalog \"MyCatalog\".\n";
        // Cancelled, answered no, or no console: nothing is written.
        for (answers, tail) in [(&["."][..], "Purge (yes/no)? .\nPurge canceled.\n"),
                                (&["no"][..], "Purge (yes/no)? no\nPurge canceled.\n")] {
            let (text, writes) = run("purge jobs client=fd1", answers);
            assert_eq!(text, format!("{head}{tail}"));
            assert!(writes.is_empty(), "{writes:?}");
        }
        let db = FakeDb::new(vec![("FROM Client WHERE Name", vec![vec!["4"]]), ("FROM Job WHERE ClientId=4", vec![vec!["11"]])]);
        let mut ua = Ua::new(&ctx);
        purge_jobs_from_client(&mut ua, &ast("purge jobs client=fd1"), &db, "fd1");
        assert!(db.writes().is_empty(), "no console: GetConfirmation falls back to no");
        // Confirmed: all records of both jobs go.
        let (text, writes) = run("purge jobs client=fd1", &["yes"]);
        assert_eq!(text, format!("{head}Purge (yes/no)? yes\nPurging the following 2 JobIds: 11,12\n\
            2 Jobs on client \"fd1\" purged from catalog.\n"));
        assert!(writes.contains(&"DELETE FROM JobMedia WHERE JobId IN (11,12)".to_string()), "{writes:?}");
        assert_eq!(writes.last().unwrap(), "DELETE FROM Job WHERE JobId IN (11,12)");
        // `yes` on the command line.
        let (_, writes) = run("purge jobs client=fd1 yes", &[]);
        assert!(!writes.is_empty());
        // Invalid jobstatus: nothing happens.
        let db = FakeDb::new(vec![("FROM Client WHERE Name", vec![vec!["4"]])]);
        let mut ua = Ua::new(&ctx);
        purge_jobs_from_client(&mut ua, &ast("purge jobs client=fd1 jobstatus=bogus"), &db, "fd1");
        let result = ua.finish(true);
        assert_eq!((result.text_output.as_str(), result.error_message.as_str()),
            ("Begin purging jobs from Client \"fd1\"\n", "invalid jobstatus parameter\n"));
        assert!(db.writes().is_empty());
    }

    #[test]
    fn purge_files_and_pool_and_quota() {
        let ctx = ctx();
        let db = FakeDb::new(vec![("FROM Client WHERE Name", vec![vec!["4"]])]);
        let mut ua = Ua::new(&ctx);
        purge_files_from_client(&mut ua, &ast("purge files client=fd1"), &db, "fd1");
        assert_eq!(ua.finish(true).text_output,
            "Begin purging files for Client \"fd1\"\nNo Files found for client fd1 to purge from MyCatalog catalog.\n");
        assert!(db.writes().is_empty());

        let db = FakeDb::new(vec![("FROM Client WHERE Name", vec![vec!["4"]]), ("PurgedFiles=0", vec![vec!["5"]])]);
        let mut ua = Ua::new(&ctx);
        purge_files_from_client(&mut ua, &ast("purge files client=fd1 yes"), &db, "fd1");
        assert_eq!(db.writes(), vec!["DELETE FROM File WHERE JobId IN (5)", "DELETE FROM BaseFiles WHERE JobId IN (5)",
            "UPDATE Job SET PurgedFiles=1 WHERE JobId IN (5)"]);

        let db = FakeDb::new(vec![("FROM Pool WHERE Name", vec![vec!["2"]]), ("JobMedia.JobId FROM", vec![vec!["8"]]),
                                  ("SELECT DISTINCT MediaId", vec![])]);
        let (_, console) = crate::ua_select::tests::scripted(&["no"], || {
            let mut ua = Ua::new(&ctx);
            purge_jobs_from_pool(&mut ua, &ast("purge jobs pool=Full"), &db, "Full");
            ua.finish(true);
        });
        assert_eq!(console.transcript(), "Found 1 Jobs for pool \"Full\" in catalog.\nPurge (yes/no)? no\n\
            Purge canceled.\nNo Media found for pool Full.\n");
        assert!(db.writes().is_empty());

        let db = FakeDb::new(vec![("FROM Client WHERE Name", vec![vec!["4"]])]);
        let mut ua = Ua::new(&ctx);
        purge_quota_from_client(&mut ua, &db, "fd1");
        assert_eq!(ua.finish(true).text_output, "Purged quota for Client \"fd1\"\n");
        assert_eq!(db.writes(), vec!["INSERT INTO Quota (ClientId, GraceTime, QuotaLimit) VALUES ('4', '0', 0)",
            "UPDATE Quota SET GraceTime='0', QuotaLimit='0' WHERE ClientId='4'"]);
    }

    #[test]
    fn purge_volume_confirms_and_marks_purged() {
        let ctx = ctx();
        let db = FakeDb::new(vec![("FROM JobMedia WHERE MediaId=7", vec![vec!["21"], vec!["22"]])]);
        let mut volume = Volume { media_id: 7, name: "Full-0007".into(), status: "Full".into(), ..Default::default() };
        let (_, console) = crate::ua_select::tests::scripted(&["no"], || {
            let mut ua = Ua::new(&ctx);
            purge_jobs_from_volume(&mut ua, &ast("purge volume=Full-0007"), &db, &mut volume, true);
            ua.finish(true);
        });
        assert_eq!(console.transcript(), "Found 2 Jobs for media \"Full-0007\" in catalog.\nPurge (yes/no)? no\nPurge canceled.\n");
        assert!(db.writes().is_empty());

        // Confirmed; the volume has no JobMedia left afterwards in this double
        // for the LIMIT query, so it is marked purged.
        let db = FakeDb::new(vec![("SELECT DISTINCT JobId FROM JobMedia", vec![vec!["21"]])]);
        let mut ua = Ua::new(&ctx);
        assert!(purge_jobs_from_volume(&mut ua, &ast("purge volume=Full-0007 yes"), &db, &mut volume, true));
        assert_eq!(ua.finish(true).text_output, "Found 1 Jobs for media \"Full-0007\" in catalog.\n\
            1 Jobs on Volume \"Full-0007\" purged from catalog.\n\
            There are no more Jobs associated with Volume \"Full-0007\". Marking it purged.\n");
        assert_eq!(db.writes().last().unwrap(), "UPDATE Media SET VolStatus='Purged' WHERE VolumeName='Full-0007'");
        assert_eq!(volume.status, "Purged");

        let mut ua = Ua::new(&ctx);
        let mut archive = Volume { name: "A".into(), status: "Archive".into(), ..Default::default() };
        let db = FakeDb::new(vec![]);
        assert!(!purge_jobs_from_volume(&mut ua, &ast("purge volume=A yes"), &db, &mut archive, true));
        assert!(db.writes().is_empty());
    }

    #[test]
    fn prune_volume_and_stats_and_directory() {
        let ctx = ctx();
        // Append volume without limits: cannot be pruned, nothing written.
        let db = FakeDb::new(vec![(MEDIA, vec![volume_row("Append")])]);
        let mut volume = get_volume(&db, 7).unwrap();
        assert_eq!(volume.first_written, str_to_utime("2026-01-01 10:00:00"));
        let mut ua = Ua::new(&ctx);
        assert!(!prune_volume(&mut ua, &db, &mut volume));
        assert_eq!(ua.finish(true).text_output,
            "Pruning volume Full-0007: cannot prune as Volstatus is Append but needs to be Full or Used.\n");
        assert!(db.writes().is_empty());

        // Full volume with two expired jobs.
        let db = FakeDb::new(vec![(MEDIA, vec![volume_row("Full")]), ("Job.JobTDate <", vec![vec!["31"], vec!["32"]]),
                                  ("LIMIT 1", vec![vec!["1"]])]);
        let mut volume = get_volume(&db, 7).unwrap();
        let mut ua = Ua::new(&ctx);
        assert!(!prune_volume(&mut ua, &db, &mut volume));
        assert_eq!(ua.finish(true).text_output,
            "Volume \"Full-0007\" has Volume Retention of 100 sec. and has 2 jobs that will be pruned\n\
             Pruning volume Full-0007: 2 Jobs have expired and can be pruned.\n\
             Purging the following 2 JobIds: 31,32\nVolume \"Full-0007\" still contains jobs after pruning.\n");
        assert_eq!(db.writes().last().unwrap(), "DELETE FROM Job WHERE JobId IN (31,32)");

        let db = FakeDb::new(vec![]);
        let mut ua = Ua::new(&ctx);
        prune_stats(&mut ua, &db, 86_400);
        assert_eq!(ua.finish(true).text_output, "Pruned Jobs from JobHisto in catalog.\n\
            Pruned Statistics from DeviceStats in catalog.\nPruned Statistics from JobStats in catalog.\n");
        assert_eq!(db.writes().len(), 3);

        // Directory for all clients: the orphan cleanup is asked for.
        let db = FakeDb::new(vec![]);
        let (_, console) = crate::ua_select::tests::scripted(&["no"], || {
            let mut ua = Ua::new(&ctx);
            prune_directory(&mut ua, &db, None, "/tmp/x", true);
            ua.finish(true);
        });
        assert_eq!(console.transcript(), "Cleanup orphaned path records (yes/no):no\n");
        assert_eq!(db.writes(), vec!["DELETE FROM file WHERE pathid IN (SELECT pathid FROM path WHERE path LIKE '/tmp/x/%')"]);
        let db = FakeDb::new(vec![("FROM Client WHERE Name", vec![vec!["4"]])]);
        let mut ua = Ua::new(&ctx);
        prune_directory(&mut ua, &db, Some("fd1"), "/it's/", false);
        assert_eq!(db.writes(), vec!["DELETE FROM file WHERE pathid IN (SELECT pathid FROM path WHERE path LIKE '/it''s/') \
            AND JobId IN (SELECT JobId FROM Job WHERE ClientId=4)"]);
    }

    #[test]
    fn prune_files_and_jobs() {
        let ctx = ctx();
        let db = FakeDb::new(vec![("SELECT COUNT(1)", vec![vec!["0"]])]);
        let mut ua = Ua::new(&ctx);
        prune_files(&mut ua, &db, "fd1", None, 86_400);
        assert_eq!(ua.finish(true).text_output, "Begin pruning Files.\nNo Files found to prune.\n");
        assert!(db.writes().is_empty());
        let db = FakeDb::new(vec![("SELECT COUNT(1)", vec![vec!["2"]]), ("SELECT JobId FROM Job", vec![vec!["3"], vec!["4"]])]);
        let mut ua = Ua::new(&ctx);
        prune_files(&mut ua, &db, "fd1", Some("Full"), 86_400);
        assert_eq!(ua.finish(true).text_output, "Begin pruning Files.\nPruned Files from 2 Jobs for client fd1 from catalog.\n");
        assert_eq!(db.writes()[0], "DELETE FROM File WHERE JobId IN (3,4)");
        assert!(db.log.lock().unwrap()[0].contains("JOIN Client USING (ClientId)  JOIN Pool USING(PoolId) "));

        let db = FakeDb::new(vec![("SELECT DISTINCT DelCandidates.JobId", vec![vec!["9"]])]);
        let mut ua = Ua::new(&ctx);
        prune_jobs(&mut ua, &db, "fd1", None, 180 * 86_400);
        assert_eq!(ua.finish(true).text_output, "Begin pruning Jobs older than 6 months .\n\
            Purging the following 1 JobIds: 9\nPruned 1 Job for client fd1 from catalog.\n");
        let db = FakeDb::new(vec![]);
        let mut ua = Ua::new(&ctx);
        prune_jobs(&mut ua, &db, "fd1", None, 180 * 86_400);
        assert_eq!(ua.finish(true).text_output, "Begin pruning Jobs older than 6 months .\nNo jobids found to be purged\n");
    }

    #[test]
    fn job_status_selection_like_upstream() {
        assert_eq!(job_status_selection(&ast("purge")), Some(vec![]));
        assert_eq!(job_status_selection(&ast("purge jobstatus=T,error,f")), Some(vec!['T', 'E', 'f']));
        assert_eq!(job_status_selection(&ast("purge jobstatus=xx")), None);
        assert!(is_a_number_list("1,2,3") && !is_a_number_list("1,,2") && !is_a_number_list("x"));
    }
}
