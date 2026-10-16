//! The catalog work of `update` (ua_update.cc) beyond single Media
//! columns: UpdatePool, UpdateVolPool, UpdateVolRecyclepool,
//! UpdateVolActiononpurge, UpdateVolenabled, UpdateVolStorage,
//! UpdateVolFromPool, UpdateAllVolsFromPool, UpdateAllVols, UpdateStats and
//! UpdateJob (compatibility log C-067/C-068).

use crate::catreq::{self, Cat, MediaRecord, PoolRecord};
use crate::commands::bvfs::Db;
use crate::commands::pool_db;
use crate::ua_select::Ua;

/// A catalog session with its own errmsg (ua->db->strerror()).
struct Session<'d> {
    db: &'d dyn Db,
    errmsg: String,
    msgs: catreq::Messages,
}

impl<'d> Session<'d> {
    fn new(db: &'d dyn Db) -> Self {
        Session { db, errmsg: String::new(), msgs: Vec::new() }
    }

    fn cat(&mut self) -> Cat<'_> {
        Cat::new(self.db, &mut self.errmsg, &mut self.msgs)
    }

    /// SqlQuery() with the "Query failed" text of a failed statement.
    fn sql_query(&mut self, sql: &str) -> bool {
        match self.db.exec(sql) {
            Ok(_) => true,
            Err(e) => {
                self.errmsg = format!("Query failed: {sql}: ERR={e}\n");
                false
            }
        }
    }

    fn pool_by_id(&mut self, pool_id: i64) -> Option<PoolRecord> {
        let mut pr = PoolRecord { pool_id, ..Default::default() };
        catreq::get_pool_record(&mut self.cat(), &mut pr).then_some(pr)
    }

    fn pool_by_name(&mut self, name: &str) -> Option<PoolRecord> {
        let mut pr = PoolRecord { name: name.to_string(), ..Default::default() };
        catreq::get_pool_record(&mut self.cat(), &mut pr).then_some(pr)
    }

    fn media(&mut self, media_id: i64) -> Option<MediaRecord> {
        let mut mr = MediaRecord { media_id, ..Default::default() };
        catreq::get_media_record(&mut self.cat(), &mut mr).then_some(mr)
    }
}

/// UpdatePool (ua_update.cc:956) after GetPoolDbr: the catalog pool `pool`
/// from the Pool resource `resource`, then list_pool (`SELECT *`).
pub(crate) fn update_pool(ua: &mut Ua, db: &dyn Db, resource: &str, pool: &str) {
    let settings = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|c| pool_db::pool_settings(&c, resource));
    let mut session = Session::new(db);
    let mut pr = session.pool_by_name(pool).unwrap_or_default();
    if let Some(settings) = settings {
        pool_db::set_pooldbr_from_poolres(&mut pr, &settings, false);
        pool_db::set_pooldbr_references(&mut session.cat(), &mut pr, &settings);
    }
    if !catreq::update_pool_record(&mut session.cat(), &mut pr) {
        ua.error_msg(format!("UpdatePoolRecord returned 0. ERR={}\n", session.errmsg));
    }
    // FillQuery(list_pool) + ListSqlQuery(HORZ_LIST)
    if let Ok(table) = db.rows(&format!("SELECT *\n  FROM Pool\n WHERE PoolId = {}\n", pr.pool_id)) {
        ua.send_msg(crate::sql_list::render(&table, crate::sql_list::ListType::Horz));
    }
    ua.info_msg("Pool DB record updated from resource.\n");
}

/// Case 9 of UpdateVolume (GetPoolRecord of the current pool) and
/// UpdateVolPool after GetPoolDbr.
pub(crate) fn update_vol_pool(ua: &mut Ua, db: &dyn Db, media_id: i64, old_pool_id: i64, pool: &str) {
    let mut session = Session::new(db);
    let Some(mut opr) = session.pool_by_id(old_pool_id) else {
        ua.error_msg(session.errmsg.clone());
        return;
    };
    let mut pr = session.pool_by_name(pool).unwrap_or_default();
    if !session.sql_query(&format!("UPDATE Media SET PoolId={} WHERE MediaId={media_id}", pr.pool_id)) {
        ua.error_msg(session.errmsg.clone());
        return;
    }
    ua.info_msg(format!("New Pool is: {}\n", pr.name));
    opr.num_vols = opr.num_vols.wrapping_sub(1);
    if !catreq::update_pool_record(&mut session.cat(), &mut opr) {
        ua.error_msg(session.errmsg.clone());
    }
    pr.num_vols = pr.num_vols.wrapping_add(1);
    if !catreq::update_pool_record(&mut session.cat(), &mut pr) {
        ua.error_msg(session.errmsg.clone());
    }
}

/// UpdateVolRecyclepool: None for an empty value ("*None*"); the *None*
/// entry of SelectPoolDbr leaves PoolId 0 and an empty name.
pub(crate) fn update_vol_recyclepool(ua: &mut Ua, db: &dyn Db, media_id: i64, pool: Option<&str>) {
    let mut session = Session::new(db);
    let (pool_id, name) = match pool {
        None => (0, "*None*".to_string()),
        Some("*None*") => (0, String::new()),
        Some(name) => session.pool_by_name(name).map_or((0, name.to_string()), |pr| (pr.pool_id, pr.name)),
    };
    if session.sql_query(&format!("UPDATE Media SET RecyclePoolId={pool_id} WHERE MediaId={media_id}")) {
        ua.info_msg(format!("New RecyclePool is: {name}\n"));
    } else {
        ua.error_msg(session.errmsg.clone());
    }
}

/// UpdateVolActiononpurge / UpdateVolenabled: the whole Media record
/// through UpdateMediaRecord.
pub(crate) fn update_vol_record(ua: &mut Ua, db: &dyn Db, media_id: i64, change: impl FnOnce(&mut MediaRecord),
    what: &str, done: impl FnOnce(&MediaRecord) -> String) {
    let mut session = Session::new(db);
    let mut mr = session.media(media_id).unwrap_or(MediaRecord { media_id, ..Default::default() });
    change(&mut mr);
    if catreq::update_media_record(&mut session.cat(), &mr) {
        ua.info_msg(done(&mr));
    } else {
        ua.error_msg(format!("Error updating media record {what}: ERR={}", session.errmsg));
    }
}

/// UpdateVolStorage after GetStorageDbr (the dialog announces the new
/// storage afterwards).
pub(crate) fn update_vol_storage(ua: &mut Ua, db: &dyn Db, media_id: i64, storage: &str, announce: bool) {
    let storage_id = crate::ua_select::get_storage_record(ua.ctx, storage).unwrap_or(0);
    let mut session = Session::new(db);
    if !session.sql_query(&format!("UPDATE Media SET StorageId={storage_id} WHERE MediaId={media_id}")) {
        ua.error_msg(session.errmsg.clone());
    }
    if announce {
        ua.info_msg(format!("New Storage is: {storage}\n"));
    }
}

/// UpdateVolFromPool: the pool defaults of the volume's own pool.
pub(crate) fn update_vol_from_pool(ua: &mut Ua, db: &dyn Db, media_id: i64) {
    let mut session = Session::new(db);
    let mut mr = session.media(media_id).unwrap_or(MediaRecord { media_id, ..Default::default() });
    let Some(pr) = session.pool_by_id(mr.pool_id) else { return };
    if !crate::acl::access(ua.ctx, crate::acl::Kind::Pool, &pr.name) {
        return;
    }
    catreq::set_pool_defaults(&mut mr, &pr);
    if pool_db::update_media_defaults(&mut session.cat(), &mr) {
        ua.info_msg(format!("Volume defaults updated from \"{}\" Pool record.\n", pr.name));
    } else {
        ua.error_msg(format!("Error updating Volume record: ERR={}", session.errmsg));
    }
}

fn update_pool_volumes(ua: &mut Ua, session: &mut Session, pr: &PoolRecord) {
    let mut mr = MediaRecord::default();
    catreq::set_pool_defaults(&mut mr, pr);
    if pool_db::update_media_defaults(&mut session.cat(), &mr) {
        ua.info_msg(format!("All Volume defaults updated from \"{}\" Pool record.\n", pr.name));
    } else {
        ua.error_msg(format!("Error updating Volume records: ERR={}", session.errmsg));
    }
}

/// UpdateAllVolsFromPool after GetPoolDbr.
pub(crate) fn update_all_vols_from_pool(ua: &mut Ua, db: &dyn Db, pool: &str) {
    let mut session = Session::new(db);
    let Some(pr) = session.pool_by_name(pool) else { return };
    update_pool_volumes(ua, &mut session, &pr);
}

/// UpdateAllVols (`fromallpools`): every catalog pool in GetPoolIds order.
pub(crate) fn update_all_vols(ua: &mut Ua, db: &dyn Db) {
    let ids = match db.rows("SELECT PoolId FROM Pool") {
        Ok(table) => table.rows.into_iter()
            .map(|row| crate::commands::job::str_to_int64(row.first().cloned().flatten().as_deref().unwrap_or("")))
            .collect::<Vec<_>>(),
        Err(e) => {
            ua.error_msg(format!("Error obtaining pool ids. ERR=Pool id select failed: ERR={e}\n\n"));
            return;
        }
    };
    let mut session = Session::new(db);
    for id in ids {
        let Some(pr) = session.pool_by_id(id) else {
            ua.warning_msg(format!("Updating all pools, but skipped PoolId={id}. ERR={}\n", session.errmsg));
            continue;
        };
        if !crate::acl::access(ua.ctx, crate::acl::Kind::Pool, &pr.name) {
            continue;
        }
        update_pool_volumes(ua, &mut session, &pr);
    }
}

/// postgresql_queries.inc 0004_fill_jobhisto.
const FILL_JOBHISTO: &str = "INSERT INTO JobHisto (
\t       JobId, Job, Name, Type, Level, ClientId, JobStatus, SchedTime, StartTime, EndTime, RealEndTime, JobTDate,
\t       VolSessionId, VolSessionTime, JobFiles, JobBytes, ReadBytes, JobErrors, JobMissingFiles, PoolId, FileSetId,
\t       PriorJobId, PurgedFiles,
\t       HasBase, Reviewed, Comment
       )
SELECT JobId, Job, Name, Type, Level, ClientId, JobStatus, SchedTime, StartTime, EndTime, RealEndTime, JobTDate, VolSessionId,
       VolSessionTime, JobFiles, JobBytes, ReadBytes, JobErrors, JobMissingFiles, PoolId, FileSetId, PriorJobId, PurgedFiles,
       HasBase,
       Reviewed, Comment
  FROM Job
 WHERE JobStatus IN ('T','W','f','A','E')
   AND NOT EXISTS (
\t\tSELECT JobHisto.JobId
\t\t  FROM JobHisto
\t\t WHERE JobHisto.Jobid = Job.JobId
       )
   AND JobTDate       < ";

/// UpdateStats / BareosDb::UpdateStats(): the jobs older than `since`
/// seconds into JobHisto, "Updating %i job(s)." with the affected rows.
pub(crate) fn update_stats(ua: &mut Ua, db: &dyn Db, since: i64) {
    let now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
    let sql = format!("{FILL_JOBHISTO}{}\n", now.wrapping_sub(since) as u64);
    let rows = db.exec(&sql).map_or(-1, |n| n as i64);
    ua.info_msg(format!("Updating {} job(s).\n", rows as i32));
}

/// UpdateJob (ua_update.cc:991): GetJobRecord, the starttime/client/
/// filesetid/jobname/jobtype arguments and one UPDATE of the Job record.
pub(crate) fn update_job(ua: &mut Ua, ast: &crate::command_ast::CommandAst, db: &dyn Db) {
    use crate::commands::job::str_to_int64;
    use crate::ua_select::find_arg_with_value;
    let mut job_id = str_to_int64(find_arg_with_value(ast, "jobid").unwrap_or(""));
    // GetJobRecord (JobId 0 searches by the empty Job name).
    let filter = if job_id == 0 { "Job=''".to_string() } else { format!("JobId={job_id}") };
    let sql = format!("SELECT VolSessionId,VolSessionTime,\
PoolId,StartTime,EndTime,JobFiles,JobBytes,JobTDate,Job,JobStatus,\
Type,Level,ClientId,Name,PriorJobId,RealEndTime,JobId,FileSetId,\
SchedTime,RealEndTime,ReadBytes,HasBase,PurgedFiles FROM Job WHERE {filter}");
    let row = match db.rows(&sql) {
        Ok(table) => table.rows.into_iter().next(),
        Err(e) => {
            ua.error_msg(format!("cats/sql_get.cc:269 query {sql} failed:\n{e}\n"));
            return;
        }
    };
    let Some(row) = row else {
        ua.error_msg(if job_id == 0 {
            "cats/sql_get.cc:273 No Job found for JobName ''\n".to_string()
        } else {
            format!("cats/sql_get.cc:275 No Job found for JobId {job_id}\n")
        });
        return;
    };
    let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    if job_id == 0 {
        job_id = str_to_int64(&text(16));
    }
    let mut c_start = text(3);
    let mut c_end = text(4);
    let mut c_sched = text(18);
    let mut job_tdate = str_to_int64(&text(7));
    let mut job_type = text(10).chars().next().unwrap_or('B');
    let mut client_id = str_to_int64(&text(12)) as u64 as i64;
    let mut name = text(13);
    let mut fileset_id = str_to_int64(&text(17)) as u64;
    let start = crate::btime::str_to_utime(&c_start);
    let sched = crate::btime::str_to_utime(&c_sched);
    let end = crate::btime::str_to_utime(&c_end);

    let start_time = find_arg_with_value(ast, "starttime");
    let client_name = find_arg_with_value(ast, "client");
    let new_fileset = find_arg_with_value(ast, "filesetid").map_or(0, |v| str_to_int64(v) as u64);
    let job_name = find_arg_with_value(ast, "jobname");
    let new_type = find_arg_with_value(ast, "jobtype").and_then(|v| v.chars().next());
    if client_name.is_none() && start_time.is_none() && new_fileset == 0 && job_name.is_none() && new_type.is_none() {
        ua.error_msg("Neither Client, StartTime, Filesetid, JobType nor Name specified.\n");
        return;
    }
    if client_name.is_some() {
        let Some((id, _)) = crate::commands::restore::get_client_dbr_with(ua, ast, db) else { return };
        client_id = id;
    }
    if new_fileset != 0 {
        fileset_id = new_fileset;
    }
    if let Some(job_name) = job_name {
        name = job_name.chars().take(127).collect();
    }
    if let Some(new_type) = new_type {
        job_type = new_type;
    }
    if let Some(start_time) = start_time {
        let new_start = crate::btime::str_to_utime(start_time);
        if new_start == 0 {
            // ua->argv[i] with i == 5 after the keyword loop.
            let argv5 = ast.arguments.get(4).and_then(|a| a.value.clone()).unwrap_or_else(|| "(null)".into());
            ua.error_msg(format!("Improper date format: {argv5}\n"));
            return;
        }
        let delta = new_start - start;
        job_tdate += delta;
        c_start = catreq::bstrutime(new_start);
        c_sched = catreq::bstrutime(sched + delta);
        c_end = catreq::bstrutime(end + delta);
    }
    let sql = format!(
        "UPDATE Job SET Name='{name}', ClientId={client_id},StartTime='{c_start}',SchedTime='{c_sched}',\
EndTime='{c_end}',JobTDate={}, FileSetId='{fileset_id}', Type='{job_type}' WHERE JobId={job_id}",
        job_tdate as u64);
    if let Err(e) = db.exec(&sql) {
        ua.error_msg(format!("Query failed: {sql}: ERR={e}\n"));
    }
}
