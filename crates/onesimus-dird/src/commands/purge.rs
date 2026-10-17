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

use crate::btime::str_to_utime;

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
    // GeneratePluginEvent(bDirEventVolumePurged): Onesimus loads no Director
    // plugins (compatibility log C-030), so there is no receiver.
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
    let running = running_jobids(ua.ctx);
    match prune_candidates_db(db, volume.media_id, volume.retention, volume.enabled, &running) {
        Ok(list) => Some(list),
        Err(error) => {
            ua.error_msg(db_error(&error));
            None
        }
    }
}

/// GetPruneCandidatesForVolume() without a console: `running` are the
/// JobIds of the running jobs (ExcludeRunningJobsFromList). S-27.
pub(crate) fn prune_candidates_db(db: &dyn Db, media_id: i64, retention: u64, enabled: i64, running: &[u32])
    -> Result<Vec<u32>, String> {
    if enabled == VOL_ARCHIVED {
        return Ok(Vec::new());
    }
    let mut list = parse_ids(column0(db, &format!("SELECT DISTINCT JobMedia.JobId\n   FROM JobMedia, Job\n   WHERE MediaId      = {}\n   \
        AND Job.JobId    = JobMedia.JobId\n   AND Job.JobTDate < {}\n   ORDER BY JobId\n",
        media_id, now() - retention as i64))?);
    if !list.is_empty() {
        let ids = list.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        list.extend(parse_ids(column0(db, &format!("SELECT JobId from Job WHERE type in ('c','g') AND priorjobid in ({ids})"))?));
    }
    list.retain(|id| *id != 0 && !running.contains(id));
    Ok(list)
}

/// next_vol.cc HasVolumeExpired(): an Append volume past one of its limits
/// becomes Full/Used (catalog updated), announced with Jmsg(M_INFO) of the
/// console JCR (JobId 0, the Director's Messages resource).
fn has_volume_expired(ctx: &ExecContext, db: &dyn Db, volume: &mut Volume) -> bool {
    if volume.status != "Append" || volume.vol_jobs <= 0 {
        return false;
    }
    let commas = crate::job_messages::commas;
    let (status, text) = if volume.max_vol_bytes > 0 && volume.vol_bytes >= volume.max_vol_bytes {
        ("Full", format!("Max Volume bytes={} exceeded. Marking Volume \"{}\" as Full.\n",
            commas(volume.max_vol_bytes), volume.name))
    } else if volume.max_vol_jobs > 0 && volume.max_vol_jobs <= volume.vol_jobs {
        ("Used", format!("Max Volume jobs={} exceeded. Marking Volume \"{}\" as Used.\n",
            commas(volume.max_vol_jobs as u64), volume.name))
    } else if volume.max_vol_files > 0 && volume.max_vol_files <= volume.vol_files {
        ("Used", format!("Max Volume files={} exceeded. Marking Volume \"{}\" as Used.\n",
            commas(volume.max_vol_files as u64), volume.name))
    } else if volume.use_duration > 0 && volume.use_duration as i64 <= now() - volume.first_written {
        ("Used", format!("Max configured use duration={} sec. exceeded. Marking Volume \"{}\" as Used.\n",
            commas(volume.use_duration), volume.name))
    } else {
        return false;
    };
    console_jmsg(ctx, crate::message_bus::MsgType::Info, &text);
    volume.status = status.into();
    if let Err(error) = db.exec(&format!("UPDATE Media SET VolStatus='{status}' WHERE VolumeName='{}'", esc(&volume.name))) {
        console_jmsg(ctx, crate::message_bus::MsgType::Error,
            &format!("Catalog error updating volume \"{}\". ERR={}", volume.name, db_error(&error)));
    }
    true
}

/// Jmsg(ua->jcr, type, 0, text) of the console JCR (JobId 0).
fn console_jmsg(ctx: &ExecContext, msg_type: crate::message_bus::MsgType, text: &str) {
    if let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) {
        crate::console_msg::job_message(&config, "-Console-", 0, msg_type, 0, text);
    }
}

/// ua_prune.cc PruneVolume(); true when the volume is empty afterwards.
pub(crate) fn prune_volume(ua: &mut Ua, db: &dyn Db, volume: &mut Volume) -> bool {
    if volume.enabled == VOL_ARCHIVED {
        return false;
    }
    let empty = volume.first_written == 0 && volume.last_written == 0;
    if !(volume.status == "Full" || volume.status == "Used" || has_volume_expired(ua.ctx, db, volume)) {
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

// ---------------------------------------------------------------------------
// ActionOnPurge (ua_purge.cc:690 ActionOnPurgeCmd, :604 do_truncate_on_purge),
// compatibility log C-049.
// ---------------------------------------------------------------------------

/// The Storage daemon connection of ActionOnPurgeCmd (`jcr->store_bsock`):
/// opened once, used for every volume, terminated at the end.
pub(crate) trait SdSession {
    /// ConnectToStorageDaemon(); Err when the connection failed.
    fn connect(&mut self, store: &bareos_config::resources::StorageResource) -> Result<(), String>;
    /// `sd->fsend(command)` and the `while (sd->recv() >= 0)` loop: every
    /// message until the next signal.
    fn exchange(&mut self, command: &str) -> Vec<String>;
}

/// The real Storage daemon (TLS-PSK as Director); BNET_TERMINATE on drop
/// (CloseSdBsock).
pub(crate) struct NetSdSession {
    director: String,
    sd: Option<bareos_bnet::BnetStream<bareos_bnet::tls_psk::PskStream>>,
}

impl NetSdSession {
    pub(crate) fn new(director: String) -> Self {
        Self { director, sd: None }
    }
}

impl SdSession for NetSdSession {
    fn connect(&mut self, store: &bareos_config::resources::StorageResource) -> Result<(), String> {
        self.sd = Some(bareos_stored::connect::connect_sd(&store.address, store.port, &store.password, &self.director)?);
        Ok(())
    }

    fn exchange(&mut self, command: &str) -> Vec<String> {
        use bareos_bnet::{BnetMessage, BnetSignal};
        let mut messages = Vec::new();
        let Some(sd) = self.sd.as_mut() else { return messages };
        // fsend errors are silent upstream; the receive loop then ends.
        if sd.send_str(command).is_err() {
            return messages;
        }
        loop {
            match sd.recv() {
                Ok(BnetMessage::Data(data)) => {
                    // SendMsg("%s", msg) / sscanf: the text up to the first NUL.
                    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                    messages.push(String::from_utf8_lossy(&data[..end]).into_owned());
                }
                Ok(BnetMessage::Signal(BnetSignal::Heartbeat)) => {
                    let _ = sd.send_signal(BnetSignal::HeartbeatResponse);
                }
                Ok(BnetMessage::Signal(_)) | Err(_) => break,
            }
        }
        messages
    }
}

impl Drop for NetSdSession {
    fn drop(&mut self) {
        if let Some(sd) = self.sd.as_mut() {
            let _ = sd.send_signal(bareos_bnet::BnetSignal::Terminate);
        }
    }
}

/// C atoi(): (int) strtol(text, NULL, 10).
fn atoi(text: &str) -> i32 {
    let text = text.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut value: i64 = 0;
    for d in digits.bytes().take_while(u8::is_ascii_digit) {
        value = match value.checked_mul(10).and_then(|v| v.checked_add((d - b'0') as i64)) {
            Some(v) => v,
            // strtol saturates at LONG_MIN/LONG_MAX.
            None => return if negative { i64::MIN as i32 } else { i64::MAX as i32 },
        };
    }
    (if negative { -value } else { value }) as i32
}

/// The Media fields do_truncate_on_purge uses (GetMediaRecord by MediaId).
struct PurgedMedia {
    media_id: i64,
    name: String,
    media_type: String,
    slot: i32,
    min_blocksize: u32,
    max_blocksize: u32,
    recycle: i64,
    action_on_purge: i64,
    in_changer: i64,
    storage_id: i64,
}

fn get_purged_media(db: &dyn Db, media_id: i64) -> Option<PurgedMedia> {
    let table = db.rows(&format!(
        "SELECT MediaId,VolumeName,MediaType,Slot,MinBlocksize,MaxBlocksize,Recycle,ActionOnPurge,InChanger,StorageId \
         FROM Media WHERE MediaId={media_id}")).ok()?;
    let row = table.rows.into_iter().next()?;
    let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let int = |i: usize| text(i).parse::<i64>().unwrap_or(0);
    Some(PurgedMedia {
        media_id: int(0), name: text(1), media_type: text(2), slot: int(3) as i32,
        min_blocksize: int(4) as u32, max_blocksize: int(5) as u32, recycle: int(6),
        action_on_purge: int(7), in_changer: int(8), storage_id: int(9),
    })
}

/// ON_PURGE_TRUNCATE (dird_conf.h).
const ON_PURGE_TRUNCATE: i64 = 1;

/// ua_purge.cc:604 do_truncate_on_purge(): relabel the volume under its own
/// name (which truncates it), accept the SD reply (crate::sd_label) and
/// store the new size.
fn do_truncate_on_purge(ua: &mut Ua, db: &dyn Db, sd: &mut dyn SdSession, media: &PurgedMedia,
    pool: &str, dev_name: &str, drive: u16) {
    use crate::sd_label::{bash_spaces, parse_label_ok};
    if media.recycle == 0 {
        return;
    }
    if media.action_on_purge & ON_PURGE_TRUNCATE == 0 {
        ua.error_msg(format!("\nThe option \"Action On Purge = Truncate\" was not defined in the Pool resource.\n\
            Unable to truncate volume \"{}\"\n", media.name));
        return;
    }
    let volume = bash_spaces(&media.name);
    // drive_number_t printed with PRIi16.
    let command = format!("relabel {dev_name} OldName={volume} NewName={volume} PoolName={} MediaType={} \
        Slot={} drive={} MinBlocksize={} MaxBlocksize={}\n",
        bash_spaces(pool), bash_spaces(&media.media_type), media.slot, drive as i16,
        media.min_blocksize, media.max_blocksize);
    let mut ok = false;
    let mut vol_bytes = 0;
    for message in sd.exchange(&command) {
        ua.send_msg(message.clone());
        if let Some((_, bytes)) = parse_label_ok(&message) {
            vol_bytes = bytes;
            ok = true;
        }
    }
    if !ok {
        ua.warning_msg(format!("Unable to truncate volume \"{}\"\n", media.name));
        return;
    }
    // UpdateMediaRecord writes back the record read before; only VolBytes
    // and VolFiles changed. MakeInchangerUnique follows.
    let mut updated = matches!(db.exec(&format!("UPDATE Media SET VolFiles=0,VolBytes={vol_bytes} WHERE VolumeName='{}'",
        esc(&media.name))), Ok(n) if n > 0);
    if media.in_changer != 0 && media.slot != 0 && media.storage_id != 0 {
        updated &= db.exec(&format!("UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={} AND StorageId={} AND MediaId!={}",
            media.slot, media.storage_id, media.media_id)).is_ok();
    }
    if !updated {
        ua.error_msg("Can't update volume size in the catalog\n");
    }
    ua.send_msg(format!("The volume \"{}\" has been truncated\n", media.name));
}

/// ua_purge.cc:690 ActionOnPurgeCmd(): `purge volume action=... [volume=]
/// [pool=|allpools] [storage=] [devicetype=] [drive=]` — the Purged,
/// recyclable, enabled volumes of more than 10000 bytes on the storage are
/// truncated through the Storage daemon. Always true, like upstream; every
/// path ends in CloseDb (the next command reopens the catalog).
pub(crate) fn action_on_purge(ua: &mut Ua, ast: &CommandAst, db: &dyn Db, sd: &mut dyn SdSession) -> bool {
    action_on_purge_work(ua, ast, db, sd);
    crate::ua_db::close_db(ua.ctx);
    true
}

fn action_on_purge_work(ua: &mut Ua, ast: &CommandAst, db: &dyn Db, sd: &mut dyn SdSession) {
    use crate::acl::valid_name;
    let mut allpools = false;
    let mut drive: u16 = u16::MAX; // kInvalidSlotNumber
    let mut volumes = String::new();
    let mut media_type = String::new();
    let mut action = "all".to_string();
    for arg in &ast.arguments {
        let key = arg.key.as_str();
        let value = arg.value.as_deref();
        if key.eq_ignore_ascii_case("allpools") {
            allpools = true;
        } else if key.eq_ignore_ascii_case("volume") && value.is_some_and(valid_name) {
            if !volumes.is_empty() {
                volumes.push(',');
            }
            volumes.push_str(&format!("'{}'", esc(value.unwrap_or_default())));
        } else if key.eq_ignore_ascii_case("devicetype") && value.is_some() {
            // bstrncpy into MediaType[MAX_NAME_LENGTH].
            let mut text = value.unwrap_or_default().to_string();
            while text.len() > 127 { text.pop(); }
            media_type = text;
        } else if key.eq_ignore_ascii_case("drive") && value.is_some() {
            drive = atoi(value.unwrap_or_default()) as u16;
        } else if key.eq_ignore_ascii_case("action") && value.is_some_and(valid_name) {
            action = value.unwrap_or_default().to_string();
        }
    }

    // Choose storage
    let Some(store) = crate::ua_select::get_storage_resource(ua, ast, false, false) else { return };
    if store.protocol.is_ndmp() {
        ua.warning_msg("Storage has non-native protocol.\n");
        return;
    }
    // OpenDb(ua): the shared connection ("Using Catalog" once more after
    // PurgeCmd's private one).
    match crate::ua_db::open_db(ast, ua.ctx) {
        Ok(prefix) => ua.send_msg(prefix),
        Err(result) => {
            ua.send_result(result);
            return;
        }
    }

    let mut pool_name = String::new();
    let mut pool_id = 0i64;
    if !allpools {
        // Force pool selection
        let Some(pool) = crate::ua_select::get_pool_resource(ua, ast) else { return };
        let Some(id) = column0(db, &format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(&pool))).ok()
            .and_then(|ids| ids.first().and_then(|id| id.parse::<i64>().ok())) else { return };
        pool_name = pool;
        pool_id = id;
    }
    // store->StorageId: the catalog record of the storage resource.
    let storage_id = column0(db, &format!("SELECT StorageId FROM Storage WHERE Name='{}'", esc(&store.name))).ok()
        .and_then(|ids| ids.first().and_then(|id| id.parse::<i64>().ok())).unwrap_or(0);

    // GetMediaIds / PrepareMediaSqlQuery: Purged volumes that can be
    // recycled, are enabled and have more than 10,000 bytes.
    let mut query = String::from("SELECT DISTINCT MediaId FROM Media WHERE Recycle=1 AND Enabled=1 ");
    if !media_type.is_empty() {
        query.push_str(&format!("AND MediaType='{}' ", esc(&media_type)));
    }
    if storage_id != 0 {
        query.push_str(&format!("AND StorageId={storage_id} "));
    }
    if pool_id != 0 {
        query.push_str(&format!("AND PoolId={pool_id} "));
    }
    query.push_str("AND VolBytes > 10000 AND VolStatus = 'Purged' ");
    if !volumes.is_empty() {
        query.push_str(&format!("AND VolumeName IN ({volumes}) "));
    }
    let Ok(ids) = column0(db, &query) else { return };
    if ids.is_empty() {
        ua.send_msg(format!("No Volumes found to perform {action} action.\n"));
        return;
    }

    // open_sd_bsock()
    ua.send_msg(format!("Connecting to Storage daemon {} at {}:{} ...\n", store.name, store.address, store.port));
    if sd.connect(&store).is_err() {
        ua.error_msg("Failed to connect to Storage daemon.\n");
        return;
    }
    let dev_name = crate::ua_storage_select::dev_name(&store);
    for id in ids {
        let Some(media) = get_purged_media(db, id.parse().unwrap_or(0)) else { continue };
        if action.eq_ignore_ascii_case("truncate") || action.eq_ignore_ascii_case("all") {
            do_truncate_on_purge(ua, db, sd, &media, &pool_name, &dev_name, drive);
        }
    }
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

    /// next_vol.cc HasVolumeExpired(): the first exceeded limit marks the
    /// volume and is announced by Jmsg(M_INFO) of the console JCR.
    #[test]
    fn expired_append_volume_is_marked_with_console_job_message() {
        let _queue = crate::console_msg::test_queue("purge-expired");
        let ctx = crate::ua_select::tests::context(
            "Director { Name = dir; Messages = Daemon }\nCatalog { Name = MyCatalog; DbName = x }\n\
             Messages { Name = Daemon; Console = all }", crate::command_executor::ApiMode::Human);
        let cases: [(&'static str, &[usize], &str, &str); 4] = [
            // MaxVolBytes=1000 reached by VolBytes (MaxVolJobs=1000 is not)
            ("1000", &[9, 10], "Full", "Max Volume bytes=1,000 exceeded. Marking Volume \"Full-0007\" as Full.\n"),
            ("3", &[10], "Used", "Max Volume jobs=3 exceeded. Marking Volume \"Full-0007\" as Used.\n"),
            ("1", &[11], "Used", "Max Volume files=1 exceeded. Marking Volume \"Full-0007\" as Used.\n"),
            ("3600", &[12], "Used", "Max configured use duration=3,600 sec. exceeded. Marking Volume \"Full-0007\" as Used.\n"),
        ];
        for (limit, columns, status, text) in cases {
            let mut row = volume_row("Append");
            for column in columns {
                row[*column] = limit;
            }
            let db = FakeDb::new(vec![(MEDIA, vec![row])]);
            let mut volume = get_volume(&db, 7).unwrap();
            assert!(has_volume_expired(&ctx, &db, &mut volume));
            assert_eq!(volume.status, status);
            assert_eq!(db.writes(), vec![format!("UPDATE Media SET VolStatus='{status}' WHERE VolumeName='Full-0007'")]);
            let messages: Vec<String> = crate::console_msg::take().iter()
                .map(|m| String::from_utf8_lossy(m).into_owned()).collect();
            assert_eq!(messages.len(), 1);
            assert!(messages[0].ends_with(&format!("dir JobId 0: {text}")), "{messages:?}");
        }
        // within all limits: nothing
        let db = FakeDb::new(vec![(MEDIA, vec![volume_row("Append")])]);
        let mut volume = get_volume(&db, 7).unwrap();
        assert!(!has_volume_expired(&ctx, &db, &mut volume));
        assert!(db.writes().is_empty() && !crate::console_msg::pending());
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
    /// Storage daemon double: records the commands, scripted replies, never
    /// touches the network.
    #[derive(Default)]
    struct FakeSd {
        refuse: bool,
        connects: usize,
        replies: std::collections::VecDeque<Vec<&'static str>>,
        sent: Vec<String>,
    }

    impl SdSession for FakeSd {
        fn connect(&mut self, _store: &bareos_config::resources::StorageResource) -> Result<(), String> {
            self.connects += 1;
            if self.refuse { Err("refused".into()) } else { Ok(()) }
        }
        fn exchange(&mut self, command: &str) -> Vec<String> {
            self.sent.push(command.to_string());
            self.replies.pop_front().unwrap_or_default().into_iter().map(str::to_string).collect()
        }
    }

    fn aop_ctx(protocol: &str) -> ExecContext {
        crate::ua_select::tests::context(&format!(
            "Director {{ Name = dir }}\nCatalog {{ Name = MyCatalog; DbName = x }}\n\
             Pool {{ Name = Full; Pool Type = Backup }}\nPool {{ Name = Inc; Pool Type = Backup }}\n\
             Storage {{ Name = File; Address = sd1; Password = x; Device = \"File Storage\"; Media Type = File{protocol} }}\n\
             Storage {{ Name = Tape; Address = sd2; Password = x; Device = LTO; Media Type = LTO }}"),
            crate::command_executor::ApiMode::Human)
    }

    /// (MediaId row): MediaId,VolumeName,MediaType,Slot,MinBlocksize,
    /// MaxBlocksize,Recycle,ActionOnPurge,InChanger,StorageId.
    fn aop_db(ids: Vec<Vec<&'static str>>) -> FakeDb {
        FakeDb::new(vec![
            ("FROM Pool WHERE Name='Full'", vec![vec!["3"]]),
            ("FROM Storage WHERE Name='File'", vec![vec!["2"]]),
            ("SELECT DISTINCT MediaId FROM Media", ids),
            ("WHERE MediaId=7", vec![vec!["7", "Full-0007", "File", "4", "0", "0", "1", "1", "1", "2"]]),
            ("WHERE MediaId=8", vec![vec!["8", "Full 0008", "File", "0", "512", "1024", "1", "0", "0", "2"]]),
            ("WHERE MediaId=9", vec![vec!["9", "Full-0009", "File", "0", "0", "0", "1", "1", "0", "2"]]),
        ])
    }

    fn run_aop(ctx: &ExecContext, line: &str, db: &FakeDb, sd: &mut FakeSd, answers: &[&str]) -> String {
        let (result, console) = crate::ua_select::tests::scripted(answers, || {
            let mut ua = Ua::new(ctx);
            action_on_purge(&mut ua, &ast(line), db, sd);
            ua.finish(true)
        });
        // Unflushed output: in call order (errors_inline) or as two blocks.
        let rest = if result.errors_inline {
            result.text_output
        } else if result.errors_first {
            format!("{}{}", result.error_message, result.text_output)
        } else {
            format!("{}{}", result.text_output, result.error_message)
        };
        console.transcript() + &rest
    }

    /// ActionOnPurgeCmd + do_truncate_on_purge (ua_purge.cc:604/690), C-049.
    #[test]
    fn action_on_purge_truncates_through_sd_relabel() {
        let ctx = aop_ctx("");
        let db = aop_db(vec![vec!["7"], vec!["8"], vec!["9"]]);
        let mut sd = FakeSd::default();
        sd.replies.push_back(vec!["3000 OK label. VolFiles=0 VolBytes=225 Volume=\"Full-0007\" Device=\"File Storage\" (/var/lib/bareos/storage)\n"]);
        sd.replies.push_back(vec!["3912 Failed to label Volume: ERR=busy\n"]);
        let text = run_aop(&ctx, "purge volume action=truncate storage=File pool=Full volume=Full-0007 volume=Full-0009", &db, &mut sd, &[]);
        assert_eq!(text, "Connecting to Storage daemon File at sd1:9103 ...\n\
            3000 OK label. VolFiles=0 VolBytes=225 Volume=\"Full-0007\" Device=\"File Storage\" (/var/lib/bareos/storage)\n\
            The volume \"Full-0007\" has been truncated\n\
            \nThe option \"Action On Purge = Truncate\" was not defined in the Pool resource.\n\
            Unable to truncate volume \"Full 0008\"\n\
            3912 Failed to label Volume: ERR=busy\n\
            Unable to truncate volume \"Full-0009\"\n");
        assert_eq!(sd.connects, 1, "one connection for all volumes");
        assert_eq!(sd.sent, vec![
            "relabel File\u{1}Storage OldName=Full-0007 NewName=Full-0007 PoolName=Full MediaType=File Slot=4 drive=-1 MinBlocksize=0 MaxBlocksize=0\n",
            "relabel File\u{1}Storage OldName=Full-0009 NewName=Full-0009 PoolName=Full MediaType=File Slot=0 drive=-1 MinBlocksize=0 MaxBlocksize=0\n",
        ]);
        let log = db.log.lock().unwrap().clone();
        assert!(log.contains(&"SELECT DISTINCT MediaId FROM Media WHERE Recycle=1 AND Enabled=1 AND StorageId=2 AND PoolId=3 \
            AND VolBytes > 10000 AND VolStatus = 'Purged' AND VolumeName IN ('Full-0007','Full-0009') ".to_string()), "{log:?}");
        assert_eq!(db.writes(), vec![
            "UPDATE Media SET VolFiles=0,VolBytes=225 WHERE VolumeName='Full-0007'".to_string(),
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=4 AND StorageId=2 AND MediaId!=7".to_string(),
        ]);

        // The older reply form, allpools (empty PoolName), devicetype and drive.
        let db = aop_db(vec![vec!["8"]]);
        let db = FakeDb::new(db.answers.into_iter().map(|(f, rows)| if f == "WHERE MediaId=8" {
            (f, vec![vec!["8", "Full 0008", "File", "0", "512", "1024", "1", "1", "0", "2"]])
        } else { (f, rows) }).collect());
        let mut sd = FakeSd::default();
        sd.replies.push_back(vec!["3000 OK label. VolBytes=64512 Volume=\"Full 0008\" Device=\"File Storage\"\n"]);
        let text = run_aop(&ctx, "purge volume allpools storage=File devicetype=File drive=1", &db, &mut sd, &[]);
        assert!(text.ends_with("The volume \"Full 0008\" has been truncated\n"), "{text}");
        assert_eq!(sd.sent, vec!["relabel File\u{1}Storage OldName=Full\u{1}0008 NewName=Full\u{1}0008 PoolName= MediaType=File \
            Slot=0 drive=1 MinBlocksize=512 MaxBlocksize=1024\n"]);
        assert!(db.log.lock().unwrap().iter().any(|q| q == "SELECT DISTINCT MediaId FROM Media WHERE Recycle=1 AND Enabled=1 \
            AND MediaType='File' AND StorageId=2 AND VolBytes > 10000 AND VolStatus = 'Purged' "));
        assert_eq!(db.writes(), vec!["UPDATE Media SET VolFiles=0,VolBytes=64512 WHERE VolumeName='Full 0008'".to_string()]);
    }

    #[test]
    fn action_on_purge_without_candidates_or_sd_changes_nothing() {
        let ctx = aop_ctx("");
        // No volume: the action name is reported, the SD is not contacted.
        let db = aop_db(vec![]);
        let mut sd = FakeSd::default();
        assert_eq!(run_aop(&ctx, "purge volume action=Truncate storage=File pool=Full", &db, &mut sd, &[]),
            "No Volumes found to perform Truncate action.\n");
        assert_eq!(run_aop(&ctx, "purge volume storage=File pool=Full", &db, &mut sd, &[]),
            "No Volumes found to perform all action.\n");
        assert_eq!((sd.connects, db.writes().len()), (0, 0));
        // An unknown action connects but sends nothing.
        let db = aop_db(vec![vec!["7"]]);
        let text = run_aop(&ctx, "purge volume action=bogus storage=File pool=Full", &db, &mut sd, &[]);
        assert_eq!(text, "Connecting to Storage daemon File at sd1:9103 ...\n");
        assert!(sd.sent.is_empty() && db.writes().is_empty());
        // Refused connection.
        let mut sd = FakeSd { refuse: true, ..FakeSd::default() };
        let text = run_aop(&ctx, "purge volume storage=File pool=Full", &db, &mut sd, &[]);
        assert_eq!(text, "Connecting to Storage daemon File at sd1:9103 ...\nFailed to connect to Storage daemon.\n");
        assert!(sd.sent.is_empty() && db.writes().is_empty());
        // Cancelled storage and pool selections.
        let mut sd = FakeSd::default();
        let text = run_aop(&ctx, "purge volume action=truncate", &db, &mut sd, &["."]);
        assert_eq!(text, "The defined Storage resources are:\n1: File\n2: Tape\nSelect Storage resource (1-2): .\n\
            Selection aborted, nothing done.\n");
        let text = run_aop(&ctx, "purge volume action=truncate storage=File pool=Nope", &db, &mut sd, &["."]);
        assert_eq!(text, "Error: Pool resource \"Nope\" does not exist.\nThe defined Pool resources are:\n1: Full\n2: Inc\n\
            Select Pool resource (1-2): .\nSelection aborted, nothing done.\n");
        // Unknown storage name, then the list.
        let text = run_aop(&ctx, "purge volume storage=Nope", &db, &mut sd, &["."]);
        assert!(text.starts_with("Storage resource \"Nope\": not found\nThe defined Storage resources are:\n"), "{text}");
        assert_eq!((sd.connects, sd.sent.len(), db.writes().len()), (0, 0, 0));
        // NDMP storage.
        let ctx = aop_ctx("; Protocol = NDMPv4");
        let text = run_aop(&ctx, "purge volume storage=File pool=Full", &db, &mut sd, &[]);
        assert_eq!(text, "Storage has non-native protocol.\n");
        assert_eq!(sd.connects, 0);
        assert_eq!(atoi(" -3x"), -3);
        assert_eq!(atoi("99999999999"), 1_215_752_191);
        assert_eq!(atoi("99999999999999999999"), i64::MAX as i32);
        assert_eq!(atoi("x"), 0);
    }
}
