//! Auto prune of Jobs and Files at the end of a job (dird/autoprune.cc
//! DoAutoprune()). DELETES CATALOG RECORDS: with `AutoPrune = yes` on the
//! Client (or `Prune Jobs`/`Prune Files` on the Job) every job that ends
//! purges the Job records older than the Job Retention and the File
//! records older than the File Retention of its client, like `prune jobs`
//! and `prune files` on the console. The latest Full chain of every
//! configured client/FileSet stays (PruneJobs, ua_prune.cc:1077-1114).
//!
//! When it runs (dird/job.cc:479-614, JobThread): after DoX() returned
//! true and its cleanup wrote the report, before the AfterJob run scripts.
//! - Backup: DoNativeBackup() returns true only when WaitForJobTermination()
//!   gave JS_Terminated (backup.cc:153); the cleanup may still make it
//!   JS_Warnings. VirtualFull only with JS_Terminated (vbackup.cc:166).
//! - Restore and Verify: once the job ran to WaitForJobTermination()
//!   (restore.cc DoNativeRestore, verify.cc DoVerify), whatever the result.
//! - Admin: always (admin.cc:62); Copy/Migrate and Consolidate: when they
//!   succeeded.
//!
//! Every message goes to the job as M_INFO (ua_output.cc:1882: a UaContext
//! without console sends each SendMsg/ErrorMsg/InfoMsg via Jmsg M_INFO).

use crate::catalog_worker::CatalogHandle;
use crate::director_config::SharedConfig;
use crate::job_worker::WorkerJobType;
use crate::message_bus::MsgType;

/// The job that just ended, as far as DoAutoprune() needs it.
pub(crate) struct EndedJob<'a> {
    pub job_id: u32,
    pub job_name: &'a str,
    pub job_type: WorkerJobType,
    pub level: &'a str,
    /// jcr->dir_impl->res.client (empty: no client, nothing to do).
    pub client: &'a str,
    /// jcr->dir_impl->res.pool.
    pub pool: &'a str,
    /// Final JobStatus after the cleanup.
    pub status: char,
    /// The File daemon was reached (the job ran to WaitForJobTermination).
    pub fd_reached: bool,
}

/// Whether job.cc calls DoAutoprune() for this job (see the module
/// comment). Approximation: a backup that ended `W` is taken as
/// JS_Terminated with errors (the only way the cleanup makes it W).
pub(crate) fn runs_after(job: &EndedJob) -> bool {
    match job.job_type {
        WorkerJobType::Backup if job.level.eq_ignore_ascii_case("VirtualFull") => job.status == 'T',
        WorkerJobType::Backup | WorkerJobType::Copy | WorkerJobType::Migrate | WorkerJobType::Consolidate => {
            matches!(job.status, 'T' | 'W')
        }
        WorkerJobType::Restore | WorkerJobType::Verify => job.fd_reached && job.status != 'A',
        WorkerJobType::Admin => job.status != 'A',
    }
}

/// DoAutoprune() (autoprune.cc:44) of a job that ended: the job messages
/// to send (all M_INFO). Empty when it does not run or nothing is to prune.
pub(crate) fn after_job(config: Option<&SharedConfig>, catalog: Option<&CatalogHandle>, job: &EndedJob)
    -> Vec<(MsgType, String)> {
    if !runs_after(job) {
        return Vec::new();
    }
    let (Some(config), Some(catalog)) = (config, catalog) else { return Vec::new() };
    do_autoprune(config, catalog, job)
}

/// autoprune.cc:44-75.
fn do_autoprune(config: &SharedConfig, catalog: &CatalogHandle, job: &EndedJob) -> Vec<(MsgType, String)> {
    // if (!jcr->dir_impl->res.client) return;
    if job.client.is_empty() {
        return Vec::new();
    }
    let (client, prune_jobs, prune_files, pool_job_retention, pool_file_retention) = {
        let Ok(cfg) = config.read() else { return Vec::new() };
        let client = crate::backup::client_retention(&cfg, job.client);
        let job_dirs = crate::commands::defaults::job_directives(&cfg, job.job_name);
        let pool_dirs: Vec<bareos_config::raw::Directive> = crate::commands::defaults::raw(&cfg, "pool", job.pool)
            .map(|r| r.directives.clone()).unwrap_or_default();
        (client,
         crate::commands::defaults::bool_directive(&job_dirs, "prunejobs", false),
         crate::commands::defaults::bool_directive(&job_dirs, "prunefiles", false),
         crate::commands::defaults::time_directive(&pool_dirs, "jobretention"),
         crate::commands::defaults::time_directive(&pool_dirs, "fileretention"))
    };
    let run_jobs = prune_jobs || client.auto_prune;
    let run_files = prune_files || client.auto_prune;
    if !run_jobs && !run_files {
        return Vec::new();
    }
    let ctx = crate::command_executor::ExecContext {
        session_id: 0,
        config: Some(config.clone()),
        session_mgr: None,
        catalog: Some(catalog.clone()),
        api_mode: crate::command_executor::ApiMode::Human,
        bus_rx: None,
        job_manager: None,
    };
    let mut ua = crate::ua_select::Ua::new(&ctx);
    // ua->db = jcr->db: one connection for the temporary tables.
    match catalog.dedicated() {
        Ok(db) => {
            let pool = (!job.pool.is_empty()).then_some(job.pool);
            if run_jobs {
                // PruneJobs(): pool->JobRetention > 0 wins over the client's.
                let period = if pool.is_some() && pool_job_retention > 0 { pool_job_retention } else { client.job_retention };
                crate::commands::purge::prune_jobs_of(&mut ua, &db, job.client, pool, period, job.job_id);
            }
            if run_files {
                // PruneFiles(): pool->FileRetention > 0 wins over the client's.
                let period = if pool.is_some() && pool_file_retention > 0 { pool_file_retention } else { client.file_retention };
                crate::commands::purge::prune_files_of(&mut ua, &db, job.client, pool, period, job.job_id);
            }
        }
        Err(error) => ua.error_msg(format!("{}\n", error.trim_end())),
    }
    let mut messages: Vec<(MsgType, String)> =
        ua.into_job_messages().into_iter().map(|text| (MsgType::Info, text)).collect();
    // if (pruned) Jmsg(jcr, M_INFO, 0, "End auto prune.\n\n");
    messages.push((MsgType::Info, "End auto prune.\n\n".to_string()));
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogRequest, CatalogResponse, ConnectionOp, SqlColumn, SqlTable};
    use std::sync::{Arc, Mutex, RwLock};

    const CONFIG: &str = "Director { Name = dir; Password = x }\nCatalog { Name = MyCatalog; DbName = x }\n\
        Client { Name = fd1; Password = x; AutoPrune = yes; File Retention = 60 days; Job Retention = 180 days }\n\
        Client { Name = fd2; Password = x }\n\
        Pool { Name = Full; Pool Type = Backup }\nFileSet { Name = fs; Include { File = /etc } }\n\
        Storage { Name = File; Address = sd; Password = x; Device = d; Media Type = File }\n\
        Messages { Name = Standard }\n\
        Job { Name = j1; Type = Backup; Client = fd1; FileSet = fs; Pool = Full; Storage = File; Messages = Standard }\n\
        Job { Name = j2; Type = Backup; Client = fd2; FileSet = fs; Pool = Full; Storage = File; Messages = Standard }\n\
        Job { Name = j3; Type = Backup; Client = fd2; FileSet = fs; Pool = Full; Storage = File; Messages = Standard; \
              Prune Files = yes }\n";

    fn config() -> SharedConfig {
        Arc::new(RwLock::new(crate::director_config::DirectorConfig::from_str(CONFIG).unwrap()))
    }

    fn ended<'a>(name: &'a str, client: &'a str, job_type: WorkerJobType, status: char) -> EndedJob<'a> {
        EndedJob { job_id: 50, job_name: name, job_type, level: "Incremental", client, pool: "Full", status,
            fd_reached: true }
    }

    /// Mock catalog: every statement of the dedicated connection is logged;
    /// DelCandidates holds 41 and 50 (the job itself), File candidates 42 and 50.
    fn catalog(log: Arc<Mutex<Vec<String>>>) -> CatalogHandle {
        CatalogHandle::mock(move |request| match request {
            CatalogRequest::Dedicated { ops } => {
                let log = log.clone();
                std::thread::spawn(move || while let Ok((op, reply)) = ops.recv() {
                    let response = match op {
                        ConnectionOp::Rows(sql) => {
                            let rows: Vec<&str> = if sql.contains("DelCandidates.JobId") {
                                vec!["41", "50"]
                            } else if sql.contains("SELECT COUNT(1)") {
                                vec!["2"]
                            } else if sql.starts_with("SELECT JobId FROM Job") {
                                vec!["42", "50"]
                            } else {
                                vec![]
                            };
                            log.lock().unwrap().push(sql);
                            CatalogResponse::Rows(SqlTable {
                                columns: vec![SqlColumn { name: "c".into(), numeric: false }],
                                rows: rows.into_iter().map(|v| vec![Some(v.to_string())]).collect(),
                            })
                        }
                        ConnectionOp::Execute(sql) => {
                            log.lock().unwrap().push(sql);
                            CatalogResponse::Ok(0)
                        }
                    };
                    let _ = reply.send(response);
                });
                CatalogResponse::Ok(0)
            }
            _ => CatalogResponse::Error("pool not used".into()),
        })
    }

    /// Against a throwaway PostgreSQL catalog (ODIR_AUTOPRUNE_PG=postgres://…):
    /// DoAutoprune after the job `pdc-system.now` of client pdc-fd. The
    /// caller counts the rows before and after.
    #[test]
    #[ignore = "needs a throwaway PostgreSQL catalog in ODIR_AUTOPRUNE_PG"]
    fn autoprune_against_postgresql() {
        let url = std::env::var("ODIR_AUTOPRUNE_PG").expect("ODIR_AUTOPRUNE_PG");
        static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let (cat, _join) = crate::catalog_worker::spawn_catalog_worker(url.parse().expect("catalog url"), &SHUTDOWN);
        let job_id: u32 = cat.query_rows("SELECT JobId FROM Job WHERE Job='pdc-system.now'").unwrap()
            .rows[0][0].clone().unwrap().parse().unwrap();
        let config: SharedConfig = Arc::new(RwLock::new(crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir; Password = x }\nCatalog { Name = MyCatalog; DbName = x }\n\
             Client { Name = pdc-fd; Password = x; AutoPrune = yes; File Retention = 60 days; Job Retention = 180 days }\n\
             Pool { Name = Full; Pool Type = Backup }\n\
             FileSet { Name = PdcSystem; Include { File = /etc } }\nFileSet { Name = PdcData; Include { File = /srv } }\n\
             Storage { Name = File; Address = sd; Password = x; Device = d; Media Type = File }\nMessages { Name = Standard }\n\
             Job { Name = pdc-system; Type = Backup; Client = pdc-fd; FileSet = PdcSystem; Pool = Full; Storage = File; Messages = Standard }\n\
             Job { Name = pdc-data; Type = Backup; Client = pdc-fd; FileSet = PdcData; Pool = Full; Storage = File; Messages = Standard }\n"
        ).unwrap()));
        let job = EndedJob { job_id, job_name: "pdc-system", job_type: WorkerJobType::Backup, level: "Incremental",
            client: "pdc-fd", pool: "Full", status: 'T', fd_reached: true };
        for (msg_type, text) in after_job(Some(&config), Some(&cat), &job) {
            let shown = if text.len() > 300 { format!("{}…\n", &text[..300]) } else { text };
            print!("{msg_type:?}: {shown}");
        }
        assert!(cat.query_rows(&format!("SELECT JobId FROM Job WHERE JobId={job_id}")).unwrap().rows.len() == 1,
            "the job itself stays");
    }

    #[test]
    fn when_bareos_calls_do_autoprune() {
        let job = |job_type, status, fd_reached, level| EndedJob { job_id: 1, job_name: "j", job_type, level,
            client: "c", pool: "p", status, fd_reached };
        assert!(runs_after(&job(WorkerJobType::Backup, 'T', true, "Full")));
        assert!(runs_after(&job(WorkerJobType::Backup, 'W', true, "Incremental")));
        assert!(!runs_after(&job(WorkerJobType::Backup, 'f', true, "Full")));
        assert!(!runs_after(&job(WorkerJobType::Backup, 'A', true, "Full")));
        assert!(!runs_after(&job(WorkerJobType::Backup, 'W', true, "VirtualFull")));
        assert!(runs_after(&job(WorkerJobType::Restore, 'f', true, "Full")));
        assert!(!runs_after(&job(WorkerJobType::Restore, 'f', false, "Full")));
        assert!(runs_after(&job(WorkerJobType::Admin, 'T', false, "Full")));
    }

    #[test]
    fn client_autoprune_prunes_jobs_then_files_without_the_job_itself() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let messages = after_job(Some(&config()), Some(&catalog(log.clone())), &ended("j1", "fd1", WorkerJobType::Backup, 'T'));
        let texts: Vec<&str> = messages.iter().map(|(t, m)| { assert_eq!(*t, MsgType::Info); m.as_str() }).collect();
        assert_eq!(texts, vec![
            "Begin pruning Jobs older than 6 months .\n",
            "Purging the following 1 JobIds: 41\n",
            "Pruned 1 Job for client fd1 from catalog.\n",
            "Begin pruning Files.\n",
            "Pruned Files from 1 Jobs for client fd1 from catalog.\n",
            "End auto prune.\n\n",
        ]);
        let log = log.lock().unwrap();
        // jobs first, the job itself (JobId 50) never goes
        let jobs = log.iter().position(|s| s == "DELETE FROM Job WHERE JobId IN (41)").unwrap();
        let files = log.iter().position(|s| s == "DELETE FROM File WHERE JobId IN (42)").unwrap();
        assert!(jobs < files);
        assert!(!log.iter().any(|s| s.contains("(50)") || s.contains(",50)")), "{log:?}");
        // Client.Name and the retention cut-off of 180 days
        assert!(log.iter().any(|s| s.starts_with("INSERT INTO DelCandidates") && s.contains("Client.Name = 'fd1'")));
    }

    #[test]
    fn no_autoprune_without_the_directives_or_after_a_failed_backup() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let cat = catalog(log.clone());
        assert!(after_job(Some(&config()), Some(&cat), &ended("j2", "fd2", WorkerJobType::Backup, 'T')).is_empty());
        assert!(after_job(Some(&config()), Some(&cat), &ended("j1", "fd1", WorkerJobType::Backup, 'f')).is_empty());
        assert!(after_job(Some(&config()), Some(&cat), &ended("j1", "", WorkerJobType::Backup, 'T')).is_empty());
        assert!(log.lock().unwrap().is_empty());
        // Prune Files = yes on the job: files only
        let messages = after_job(Some(&config()), Some(&cat), &ended("j3", "fd2", WorkerJobType::Backup, 'T'));
        assert_eq!(messages.first().map(|m| m.1.as_str()), Some("Begin pruning Files.\n"));
        assert!(!log.lock().unwrap().iter().any(|s| s.contains("DelCandidates")));
    }
}
