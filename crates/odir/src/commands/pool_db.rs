// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Pool and Media catalog records from the Pool resource: ua_db.cc
//! CreatePool(), SetPooldbrFromPoolres(), SetPooldbrReferences(),
//! SetPoolDbrDefaultsInMediaDbr() and the catalog functions they call
//! (sql_create.cc CreatePoolRecord(), sql_update.cc UpdateMediaDefaults()).
//! Used by `create` (C-066) and `update pool`/`update volume` (C-067/C-068).

use crate::catreq::{self, Cat, PoolRecord, MediaRecord};
use crate::commands::defaults::{boolean, first, number, pint32, raw, SIZES, TIMES};
use crate::director_config::DirectorConfig;

/// The PoolResource members SetPooldbrFromPoolres()/SetPooldbrReferences()
/// read, with the dird_conf.cc defaults.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PoolSettings {
    pub name: String,
    pub pool_type: String,
    pub max_volumes: u32,
    pub label_type: i32,
    pub use_catalog: bool,
    pub recycle: bool,
    pub vol_retention: u64,
    pub vol_use_duration: u64,
    pub max_vol_jobs: u32,
    pub max_vol_files: u32,
    pub max_vol_bytes: u64,
    pub auto_prune: bool,
    pub action_on_purge: u32,
    pub min_blocksize: u32,
    pub max_blocksize: u32,
    pub label_format: Option<String>,
    pub recycle_pool: Option<String>,
    pub scratch_pool: Option<String>,
}

/// B_BAREOS_LABEL, B_ANSI_LABEL, B_IBM_LABEL (lib/parse_conf.cc tapelabels).
fn label_type(value: &str) -> i32 {
    match value.to_ascii_lowercase().as_str() {
        "ansi" => 1,
        "ibm" => 2,
        _ => 0,
    }
}

/// The Pool resource `name` of the configuration.
pub(crate) fn pool_settings(cfg: &DirectorConfig, name: &str) -> Option<PoolSettings> {
    let pool = cfg.find_pool(name)?;
    let dirs = raw(cfg, "pool", &pool.name).map(|r| r.directives.as_slice()).unwrap_or(&[]);
    let size32 = |key: &str| number(dirs, key, 0, SIZES).unwrap_or(0) as u32;
    Some(PoolSettings {
        name: pool.name.clone(),
        pool_type: first(dirs, "pooltype").map(str::to_string).unwrap_or_else(|| pool.pool_type.clone()),
        max_volumes: pint32(dirs, "maximumvolumes", pool.maximum_volumes).unwrap_or(0),
        label_type: first(dirs, "labeltype").map(label_type).unwrap_or(0),
        use_catalog: boolean(dirs, "usecatalog", true).unwrap_or(1) != 0,
        recycle: boolean(dirs, "recycle", true).unwrap_or(1) != 0,
        vol_retention: number(dirs, "volumeretention", 31_536_000, TIMES).unwrap_or(0),
        vol_use_duration: number(dirs, "volumeuseduration", 0, TIMES).unwrap_or(0),
        max_vol_jobs: pint32(dirs, "maximumvolumejobs", 0).unwrap_or(0),
        max_vol_files: pint32(dirs, "maximumvolumefiles", 0).unwrap_or(0),
        max_vol_bytes: number(dirs, "maximumvolumebytes", 0, SIZES).unwrap_or(0),
        auto_prune: boolean(dirs, "autoprune", true).unwrap_or(1) != 0,
        // ON_PURGE_TRUNCATE
        action_on_purge: dirs.iter().filter(|d| d.norm_key() == "actiononpurge")
            .flat_map(|d| d.values()).fold(0, |acc, v| if v.eq_ignore_ascii_case("truncate") { acc | 1 } else { acc }),
        min_blocksize: size32("minimumblocksize"),
        max_blocksize: size32("maximumblocksize"),
        label_format: first(dirs, "labelformat").map(str::to_string),
        recycle_pool: first(dirs, "recyclepool").map(str::to_string),
        scratch_pool: first(dirs, "scratchpool").map(str::to_string),
    })
}

/// ua_db.cc SetPooldbrFromPoolres(); `create` is POOL_OP_CREATE.
pub(crate) fn set_pooldbr_from_poolres(pr: &mut PoolRecord, pool: &PoolSettings, create: bool) {
    pr.pool_type = pool.pool_type.clone();
    if create {
        pr.max_vols = pool.max_volumes;
        pr.num_vols = 0;
    } else {
        if pr.max_vols != pool.max_volumes {
            pr.max_vols = pool.max_volumes;
        }
        if pr.max_vols != 0 && pr.max_vols < pr.num_vols {
            pr.max_vols = pr.num_vols;
        }
    }
    pr.label_type = pool.label_type;
    pr.use_once = 0;
    pr.use_catalog = i32::from(pool.use_catalog);
    pr.recycle = i32::from(pool.recycle);
    pr.vol_retention = pool.vol_retention as i64;
    pr.vol_use_duration = pool.vol_use_duration as i64;
    pr.max_vol_jobs = pool.max_vol_jobs;
    pr.max_vol_files = pool.max_vol_files;
    pr.max_vol_bytes = pool.max_vol_bytes;
    pr.auto_prune = i32::from(pool.auto_prune);
    pr.action_on_purge = pool.action_on_purge;
    pr.min_blocksize = pool.min_blocksize;
    pr.max_blocksize = pool.max_blocksize;
    pr.label_format = pool.label_format.clone().unwrap_or_else(|| "*".into());
}

/// ua_db.cc SetPooldbrReferences(): RecyclePoolId/ScratchPoolId of the
/// referenced pools (the Jmsg warning for a missing record is collected
/// in the Cat messages).
pub(crate) fn set_pooldbr_references(cat: &mut Cat, pr: &mut PoolRecord, pool: &PoolSettings) -> bool {
    let mut ok = true;
    for (reference, what) in [(&pool.recycle_pool, "RecyclePool"), (&pool.scratch_pool, "ScratchPool")] {
        let id = match reference {
            Some(name) => {
                let mut rpool = PoolRecord { name: name.clone(), ..Default::default() };
                if catreq::get_pool_record(cat, &mut rpool) {
                    Some(rpool.pool_id)
                } else {
                    cat.jmsg_pub(crate::message_bus::MsgType::Warning, format!(
                        "Can't set {0} {what} to {name}, {name} is not in database.\nTry to update it with 'update pool={0}'\n",
                        pool.name));
                    ok = false;
                    None
                }
            }
            None => Some(0),
        };
        if let Some(id) = id {
            if what == "RecyclePool" { pr.recycle_pool_id = id } else { pr.scratch_pool_id = id }
        }
    }
    ok
}

/// sql_create.cc CreatePoolRecord().
pub(crate) fn create_pool_record(cat: &mut Cat, pr: &mut PoolRecord) -> bool {
    let name = catreq::esc(&pr.name);
    if let Some(table) = cat.query_db(&format!("SELECT PoolId,Name FROM Pool WHERE Name='{name}'"), "sql_create.cc:164") {
        if !table.rows.is_empty() {
            *cat.errmsg = format!("cats/sql_create.cc:167 pool record {} already exists\n", pr.name);
            return false;
        }
    }
    let sql = format!(
        "INSERT INTO Pool (Name,NumVols,MaxVols,UseOnce,UseCatalog,\
AcceptAnyVolume,AutoPrune,Recycle,VolRetention,VolUseDuration,\
MaxVolJobs,MaxVolFiles,MaxVolBytes,PoolType,LabelType,LabelFormat,\
RecyclePoolId,ScratchPoolId,ActionOnPurge,MinBlocksize,MaxBlocksize) \
VALUES ('{name}',{},{},{},{},{},{},{},{},{},{},{},{},'{}',{},'{}',{},{},{},{},{})",
        pr.num_vols, pr.max_vols, pr.use_once, pr.use_catalog, pr.accept_any_volume, pr.auto_prune,
        pr.recycle, pr.vol_retention as u64, pr.vol_use_duration as u64, pr.max_vol_jobs, pr.max_vol_files,
        pr.max_vol_bytes, pr.pool_type, pr.label_type, catreq::esc(&pr.label_format), pr.recycle_pool_id,
        pr.scratch_pool_id, pr.action_on_purge as i32, pr.min_blocksize as i32, pr.max_blocksize as i32);
    let (id, error) = cat.insert_autokey(&sql, "Pool");
    pr.pool_id = id as i64;
    if pr.pool_id == 0 {
        *cat.errmsg = format!("cats/sql_create.cc:202 Create db Pool record {sql} failed: ERR={}\n",
            error.unwrap_or_default());
        return false;
    }
    true
}

/// ua_db.cc CreatePool(): -1 error, 0 the record exists (updated for
/// POOL_OP_UPDATE), 1 created.
pub(crate) fn create_pool(cat: &mut Cat, pool: &PoolSettings, create: bool) -> i32 {
    let mut pr = PoolRecord { name: pool.name.clone(), ..Default::default() };
    if catreq::get_pool_record(cat, &mut pr) {
        if !create {
            set_pooldbr_from_poolres(&mut pr, pool, false);
            set_pooldbr_references(cat, &mut pr, pool);
            catreq::update_pool_record(cat, &mut pr);
        }
        return 0;
    }
    set_pooldbr_from_poolres(&mut pr, pool, create);
    set_pooldbr_references(cat, &mut pr, pool);
    if !create_pool_record(cat, &mut pr) {
        return -1;
    }
    1
}

/// job.cc GetOrCreatePoolRecord() and the pool part of ua_output.cc
/// CompleteJcrForJob() (C-088): the PoolId of `name`. A missing record is
/// created from the Pool resource (CreatePool, POOL_OP_CREATE) and
/// `created` (with `{}` for the name) goes to `msgs` as M_INFO, followed by
/// the Jmsg side effects of the catalog session. Err: the M_FATAL text
/// (`not_in_db`, `{name}` and `{err}` replaced) when the record can neither
/// be read nor created.
pub(crate) fn get_or_create_pool_record(cfg: &DirectorConfig, catalog: &dyn crate::commands::bvfs::Db, name: &str,
                                        created: &str, not_in_db: &str,
                                        msgs: &mut Vec<(crate::message_bus::MsgType, String)>) -> Result<i64, String> {
    use crate::message_bus::MsgType;
    let (mut errmsg, mut side) = (String::new(), Vec::new());
    let mut cat = Cat::new(catalog, &mut errmsg, &mut side);
    let mut pr = PoolRecord { name: name.to_string(), ..Default::default() };
    let mut result = Ok(0);
    // while (!GetPoolRecord) { CreatePool … } — one creation, then the read.
    for attempt in 0..2 {
        if catreq::get_pool_record(&mut cat, &mut pr) {
            result = Ok(pr.pool_id);
            break;
        }
        let created_ok = attempt == 0
            && pool_settings(cfg, name).is_some_and(|settings| create_pool(&mut cat, &settings, true) >= 0);
        if !created_ok {
            let err = cat.errmsg.clone();
            result = Err(not_in_db.replace("{name}", name).replace("{err}", &err));
            break;
        }
        cat.jmsg_pub(MsgType::Info, created.replace("{}", name));
    }
    drop(cat);
    msgs.extend(side);
    result
}

/// ua_db.cc UpdatePoolReferences(): only pools with a RecyclePool or
/// ScratchPool are rewritten.
pub(crate) fn update_pool_references(cat: &mut Cat, pool: &PoolSettings) -> bool {
    if pool.recycle_pool.is_none() && pool.scratch_pool.is_none() {
        return true;
    }
    let mut pr = PoolRecord { name: pool.name.clone(), ..Default::default() };
    if !catreq::get_pool_record(cat, &mut pr) {
        return false;
    }
    set_pooldbr_from_poolres(&mut pr, pool, false);
    set_pooldbr_references(cat, &mut pr, pool) && catreq::update_pool_record(cat, &mut pr)
}

/// sql_update.cc UpdateMediaDefaults(): by VolumeName when set, else for
/// every volume of the pool.
pub(crate) fn update_media_defaults(cat: &mut Cat, mr: &MediaRecord) -> bool {
    let set = format!(
        "UPDATE Media SET ActionOnPurge={},Recycle={},VolRetention={},VolUseDuration={},\
MaxVolJobs={},MaxVolFiles={},MaxVolBytes={},RecyclePoolId={},MinBlocksize={},MaxBlocksize={} ",
        mr.action_on_purge as i32, mr.recycle, mr.vol_retention as u64, mr.vol_use_duration as u64,
        mr.max_vol_jobs, mr.max_vol_files, mr.max_vol_bytes, mr.recycle_pool_id, mr.min_blocksize as i32,
        mr.max_blocksize as i32);
    let sql = if mr.volume_name.is_empty() {
        format!("{set}WHERE PoolId={}", mr.pool_id)
    } else {
        format!("{set}WHERE VolumeName='{}'", catreq::esc(&mr.volume_name))
    };
    cat.update_db(&sql, "sql_update.cc:410") != -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_follow_the_resource_defaults() {
        let cfg = DirectorConfig::from_str(
            "Director { Name = dir }\nPool { Name = Full; Pool Type = Backup; Label Format = \"Full-\"; \
             Maximum Volume Bytes = 50G; Maximum Volumes = 100; Volume Retention = 365 days; \
             Action On Purge = Truncate; Recycle Pool = Scratch }\nPool { Name = Scratch; Pool Type = Scratch }").unwrap();
        let full = pool_settings(&cfg, "Full").unwrap();
        assert_eq!((full.max_vol_bytes, full.max_volumes, full.vol_retention), (53_687_091_200, 100, 31_536_000));
        assert_eq!((full.action_on_purge, full.recycle, full.auto_prune, full.use_catalog), (1, true, true, true));
        assert_eq!((full.label_format.as_deref(), full.recycle_pool.as_deref()), (Some("Full-"), Some("Scratch")));
        let mut pr = PoolRecord::default();
        set_pooldbr_from_poolres(&mut pr, &pool_settings(&cfg, "Scratch").unwrap(), true);
        assert_eq!((pr.label_format.as_str(), pr.pool_type.as_str(), pr.vol_retention), ("*", "Scratch", 31_536_000));
    }

    /// Pool table double: `Name -> PoolId`; INSERT … currval creates a row.
    struct PoolTable(std::sync::Mutex<Vec<(String, i64)>>);
    impl crate::commands::bvfs::Db for PoolTable {
        fn rows(&self, sql: &str) -> Result<crate::catalog_worker::SqlTable, String> {
            let mut pools = self.0.lock().unwrap();
            let quoted = |s: &str| s.split('\'').nth(1).unwrap_or("").to_string();
            let rows = if sql.starts_with("INSERT INTO Pool") {
                let id = pools.len() as i64 + 1;
                pools.push((quoted(sql), id));
                vec![vec![Some(id.to_string())]]
            } else if sql.contains(" FROM Pool WHERE ") {
                let name = quoted(sql);
                pools.iter().filter(|(n, _)| *n == name).map(|(n, id)| {
                    let mut row = vec![Some("0".to_string()); 22];
                    row[0] = Some(id.to_string());
                    row[1] = Some(n.clone());
                    row
                }).collect()
            } else {
                Vec::new()
            };
            Ok(crate::catalog_worker::SqlTable { columns: Vec::new(), rows })
        }
        fn exec(&self, _sql: &str) -> Result<u64, String> {
            Ok(1)
        }
    }

    /// C-088: job.cc GetOrCreatePoolRecord creates a missing record with
    /// M_INFO "Created database record for Pool"; an existing record gives
    /// its id without a message. The Next Pool is created under its own name
    /// (Bareos would create the job's pool and loop forever, U-001).
    #[test]
    fn get_or_create_pool_record_creates_a_missing_pool() {
        let cfg = DirectorConfig::from_str(
            "Director { Name = dir }\nPool { Name = Full; Pool Type = Backup; Label Format = \"Full-\" }\n\
             Pool { Name = Copy; Pool Type = Backup }").unwrap();
        let db = PoolTable(std::sync::Mutex::new(vec![("Full".into(), 1)]));
        let mut msgs = Vec::new();
        let created = crate::backup::POOL_CREATED;
        let missing = crate::backup::POOL_NOT_IN_DB;
        assert_eq!(get_or_create_pool_record(&cfg, &db, "Full", created, missing, &mut msgs), Ok(1));
        assert!(msgs.is_empty());
        assert_eq!(get_or_create_pool_record(&cfg, &db, "Copy", created, missing, &mut msgs), Ok(2));
        assert_eq!(msgs, vec![(crate::message_bus::MsgType::Info, "Created database record for Pool \"Copy\".\n".to_string())]);
        assert_eq!(db.0.lock().unwrap().iter().filter(|(n, _)| n == "Copy").count(), 1);
        // a pool without resource cannot be created: the M_FATAL text
        let err = get_or_create_pool_record(&cfg, &db, "Nosuch", created, missing, &mut msgs).unwrap_err();
        assert!(err.starts_with("Pool \"Nosuch\" not in database. ERR="), "{err}");
    }

    #[test]
    fn update_keeps_max_vols_at_least_num_vols() {
        let pool = PoolSettings { max_volumes: 2, ..Default::default() };
        let mut pr = PoolRecord { num_vols: 5, max_vols: 9, ..Default::default() };
        set_pooldbr_from_poolres(&mut pr, &pool, false);
        assert_eq!((pr.max_vols, pr.num_vols), (5, 5));
        set_pooldbr_from_poolres(&mut pr, &pool, true);
        assert_eq!((pr.max_vols, pr.num_vols), (2, 0));
    }
}
