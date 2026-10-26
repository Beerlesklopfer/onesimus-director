//! Jobs the scheduler starts — ExecuteJob() of the default scheduler
//! (dird/scheduler_private.cc TryCreateJobControlRecord, SetJcrFromRunResource;
//! dird/job.cc ExecuteJob → RunJob).
//!
//! The original creates the JCR with SetJcrDefaults() from the Job resource,
//! applies the overrides of the Run directive and calls RunJob(), the same
//! function `run` ends in (ua_run.cc DoRunCmd). Here the scheduler builds the
//! confirmed run request (`run job=… level=… yes`) and hands it to
//! `commands::job::start_job`, the start of the `run` command; the parts of
//! the Run directive a command line cannot express (job trigger
//! "Scheduler", the level pool overrides and their pool source, the
//! Messages override, Accurate, SpoolData) are the [`RunOverrides`] of the current
//! thread while the job starts. The Next Pool goes in the request
//! (`nextpool=`); MaxRunSchedTime reaches the job queue's checks
//! (job_manager/limits.rs) through [`start_flags`].
//!
//! Unlike an interactive run, a scheduled run writes no "Job queued" audit
//! event and sends no console output (ua_run.cc:500 only audits runs with
//! IgnoreLevelPoolOverrides).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bareos_config::resources::{JobResource, JobType, RunDirective};

use crate::command_ast::{Argument, CommandAst};
use crate::command_executor::{ApiMode, ExecContext};
use crate::director_config::{DirectorConfig, SharedConfig};

/// The overrides of a Run directive (SetJcrFromRunResource) for the job the
/// current thread starts.
#[derive(Debug, Clone, Default)]
pub struct RunOverrides {
    /// JobTriggerToString(jcr->dir_impl->job_trigger).
    pub trigger: &'static str,
    pub pool: Option<String>,
    pub full_pool: Option<String>,
    pub incremental_pool: Option<String>,
    pub differential_pool: Option<String>,
    pub storage: Option<String>,
    pub messages: Option<String>,
    pub priority: Option<i32>,
    pub accurate: Option<bool>,
    /// jcr->dir_impl->spool_data.
    pub spool_data: Option<bool>,
    /// jcr->dir_impl->MaxRunSchedTime (seconds).
    pub max_run_sched_time: Option<u64>,
    /// jcr->dir_impl->res.pool_source after ApplyPoolOverrides().
    pool_source: Option<String>,
}

/// JobTriggerToString(JobTrigger::kScheduler).
pub const TRIGGER_SCHEDULER: &str = "Scheduler";

impl RunOverrides {
    /// SetJcrFromRunResource(): the overrides of `run` (none without a Run
    /// directive, as for AddJobWithNoRunResourceToQueue).
    pub fn from_run(run: Option<&RunDirective>) -> Self {
        let Some(run) = run else {
            return Self { trigger: TRIGGER_SCHEDULER, ..Default::default() };
        };
        Self {
            trigger: TRIGGER_SCHEDULER,
            pool: run.pool.clone(),
            full_pool: run.full_pool.clone(),
            incremental_pool: run.incremental_pool.clone(),
            differential_pool: run.differential_pool.clone(),
            storage: run.storage.clone(),
            messages: run.messages.clone(),
            // `if (run->Priority != 0) jcr->JobPriority = run->Priority;`
            priority: run.priority.filter(|p| *p != 0),
            accurate: run.accurate,
            spool_data: run.spool_data,
            max_run_sched_time: run.max_run_sched_time,
            pool_source: None,
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<RunOverrides>> = const { RefCell::new(None) };
    /// `ignoreduplicatecheck=` of the run the current thread starts.
    static IGNORE_DUPLICATE_CHECK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// reschedule_count of a job RescheduleJob() reruns as a new job.
    static RESCHEDULE_COUNT: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// RescheduleJob() of a job that wrote data (jobq.cc:640): RunJob() of a
/// new JCR with the job's level, pool, write storage and sched_time.
pub fn rerun(manager: &Arc<Mutex<crate::job_manager::JobManager>>, rerun: &crate::job_manager::Rerun) {
    let (config, catalog) = {
        let mgr = manager.lock().unwrap();
        (mgr.config(), mgr.catalog())
    };
    let Some(config) = config else { return };
    let Some(job) = config.read().ok().and_then(|c| c.find_job(&rerun.job_name).cloned()) else { return };
    let arg = |key: &str, value: &str| Argument { key: key.to_string(), value: Some(value.to_string()) };
    let mut arguments = vec![arg("job", &rerun.job_name), arg("level", &rerun.level)];
    if !rerun.pool.is_empty() { arguments.push(arg("pool", &rerun.pool)); }
    if !rerun.storage.is_empty() { arguments.push(arg("storage", &rerun.storage)); }
    arguments.push(arg("schedtime", &rerun.sched_time.to_string()));
    arguments.push(Argument { key: "yes".to_string(), value: None });
    let ast = CommandAst { command_name: "run".to_string(), arguments, ..Default::default() };
    let ctx = ExecContext { session_id: 0, config: Some(config), session_mgr: None, catalog,
        api_mode: ApiMode::Human, bus_rx: None, job_manager: Some(manager.clone()) };
    RESCHEDULE_COUNT.with(|c| c.set(rerun.reschedule_count));
    let result = crate::commands::job::start_job(&ast, &ctx, job);
    RESCHEDULE_COUNT.with(|c| c.set(0));
    tracing::info!("RescheduleJob: {} rerun: {}", rerun.job_name, result.text_output.trim_end());
}

/// Run `f` (a job start) with the run's `ignoreduplicatecheck=` flag.
pub fn with_ignore_duplicate_check<R>(ignore: bool, f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            IGNORE_DUPLICATE_CHECK.with(|c| c.set(false));
        }
    }
    IGNORE_DUPLICATE_CHECK.with(|c| c.set(ignore));
    let _reset = Reset;
    f()
}

/// The JCR fields of the job the current thread starts that the job
/// queue's duplicate and time checks need (S-22).
pub fn start_flags() -> crate::job_manager::StartFlags {
    crate::job_manager::StartFlags {
        ignore_duplicate_check: IGNORE_DUPLICATE_CHECK.with(|c| c.get()),
        max_run_sched_time: current(|o| o.max_run_sched_time).flatten().unwrap_or(0) as i64,
        reschedule_count: RESCHEDULE_COUNT.with(|c| c.get()),
    }
}

/// Run `f` with `overrides` as the Run directive of the job it starts.
pub fn with_overrides<R>(overrides: RunOverrides, f: impl FnOnce() -> R) -> R {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = None);
        }
    }
    CURRENT.with(|c| *c.borrow_mut() = Some(overrides));
    let _reset = Reset;
    f()
}

fn current<R>(f: impl FnOnce(&mut RunOverrides) -> R) -> Option<R> {
    CURRENT.with(|c| c.borrow_mut().as_mut().map(f))
}

/// The Run directive's SpoolData of the job the current thread starts.
pub fn spool_data() -> Option<bool> {
    current(|o| o.spool_data).flatten()
}

/// The Run directive's Accurate of the job the current thread starts.
pub fn accurate() -> Option<bool> {
    current(|o| o.accurate).flatten()
}

/// The job trigger of the job the current thread starts (None: a console
/// `run`, JobTrigger::kUser).
#[allow(dead_code)]
pub fn trigger() -> Option<&'static str> {
    current(|o| o.trigger)
}

/// ApplyPoolOverrides(jcr, false) of a scheduled backup (job.cc:1213) for
/// the effective level `level` ("F", "I", "D", "f"): the pool and its
/// source, when the Run directive overrides a pool; None leaves the pool
/// selection of `run` (Job resource and its level pools) as it is.
pub fn backup_pool(job: &JobResource, level: &str) -> Option<String> {
    current(|o| {
        let level_pools = [&o.full_pool, &o.incremental_pool, &o.differential_pool];
        if o.pool.is_none() && level_pools.iter().all(|p| p.is_none()) {
            return None;
        }
        // If only a pool override and no level overrides are given in the
        // run entry, choose this pool.
        if let Some(pool) = o.pool.as_ref().filter(|_| level_pools.iter().all(|p| p.is_none())) {
            o.pool_source = Some("Run Pool override".to_string());
            return Some(pool.clone());
        }
        let nonempty = |s: &String| (!s.is_empty()).then(|| s.clone());
        let (run_pool, job_pool, kind) = match level {
            "F" => (&o.full_pool, nonempty(&job.full_backup_pool), "FullPool"),
            "I" => (&o.incremental_pool, nonempty(&job.incremental_backup_pool), "IncPool"),
            "D" => (&o.differential_pool, nonempty(&job.differential_backup_pool), "DiffPool"),
            _ => (&None, None, ""),
        };
        if let Some(pool) = run_pool {
            o.pool_source = Some(format!("Run {kind} override"));
            return Some(pool.clone());
        }
        if let Some(pool) = job_pool {
            o.pool_source = Some(format!("Job {kind} override"));
            return Some(pool);
        }
        // res.pool (the Run's Pool, else the Job's); SetJcrDefaults source.
        o.pool_source = Some("Job resource".to_string());
        Some(o.pool.clone().unwrap_or_else(|| job.pool.clone()))
    }).flatten()
}

/// The JCR of a job the current thread queues: JobPriority, the trigger
/// and the Run directive's sources in the job report, the Messages
/// override. Does nothing outside a scheduled start.
pub fn apply(job_id: u32, priority: &mut i32, report: &mut crate::job_messages::JobReport) {
    current(|o| {
        report.trigger = o.trigger.to_string();
        if let Some(p) = o.priority {
            *priority = p;
            report.priority = p;
        }
        if let Some(source) = o.pool_source.take() {
            report.pool_source = source;
        }
        if o.storage.is_some() {
            // SetRwstorage(jcr, &store) with store_source "run override".
            report.storage_source = "run override".to_string();
        }
        if let Some(accurate) = o.accurate {
            report.accurate = accurate;
        }
        if let Some(messages) = &o.messages {
            set_job_messages(job_id, messages);
        }
    });
}

/// The Messages resource of the job the current thread starts (before its
/// JobId is registered with [`apply`]).
fn current_messages() -> Option<String> {
    current(|o| o.messages.clone()).flatten()
}

/// `jcr->dir_impl->res.messages` of running jobs whose Run directive
/// overrides the Job's Messages resource.
static JOB_MESSAGES: Mutex<BTreeMap<u32, String>> = Mutex::new(BTreeMap::new());
/// Overrides kept (the oldest JobIds are dropped beyond).
const MAX_JOB_MESSAGES: usize = 1024;

fn set_job_messages(job_id: u32, messages: &str) {
    let mut map = JOB_MESSAGES.lock().unwrap_or_else(|e| e.into_inner());
    map.insert(job_id, messages.to_string());
    while map.len() > MAX_JOB_MESSAGES {
        map.pop_first();
    }
}

/// The Messages resource overriding the Job's one for JobId `job_id`.
pub fn job_messages(job_id: u32) -> Option<String> {
    if job_id == 0 {
        return None;
    }
    JOB_MESSAGES.lock().unwrap_or_else(|e| e.into_inner()).get(&job_id).cloned().or_else(current_messages)
}

/// What a scheduled job needs to start like `run`.
#[derive(Clone)]
pub struct Launcher {
    pub config: SharedConfig,
    pub session_mgr: Option<Arc<crate::session_manager::SessionManager>>,
    pub catalog: Option<crate::catalog_worker::CatalogHandle>,
    pub job_manager: Option<Arc<Mutex<crate::job_manager::JobManager>>>,
}

/// JobLevel names (joblevels[], dird_conf.cc:518).
fn level_name(level: bareos_config::resources::JobLevel) -> String {
    format!("{level:?}")
}

/// The confirmed run request of a scheduled job: SetJcrDefaults (job,
/// level, client, fileset, storage, pool of the Job resource) with the
/// level, pool and storage of the Run directive.
pub fn run_request(cfg: &DirectorConfig, job: &JobResource, run: Option<&RunDirective>, sched_time: i64) -> CommandAst {
    let arg = |key: &str, value: String| Argument { key: key.to_string(), value: Some(value) };
    let mut arguments = vec![arg("job", job.name.clone())];
    // The level of the Run, else the Job's Level directive; a job without
    // one keeps the default of its type (run_selected / run_admin).
    let job_level = crate::commands::defaults::job_directives(cfg, &job.name).iter()
        .any(|d| d.norm_key() == "level").then_some(job.level);
    if let Some(level) = run.and_then(|r| r.level).or(job_level) {
        arguments.push(arg("level", level_name(level)));
    }
    if let Some(storage) = run.and_then(|r| r.storage.clone()) {
        arguments.push(arg("storage", storage));
    }
    // Backups take their pool from backup_pool() (ApplyPoolOverrides).
    if job.job_type != JobType::Backup {
        if let Some(pool) = run.and_then(|r| r.pool.clone()) {
            arguments.push(arg("pool", pool));
        }
    }
    if let Some(pool) = run.and_then(|r| r.next_pool.clone()) {
        arguments.push(arg("nextpool", pool));
    }
    arguments.push(arg("schedtime", sched_time.to_string()));
    arguments.push(Argument { key: "yes".to_string(), value: None });
    CommandAst { command_name: "run".to_string(), arguments, ..Default::default() }
}

impl Launcher {
    /// ExecuteJob(jcr): RunJob() of the job `job_name` with the overrides of
    /// `run` (None for a job queued without a Run directive). Returns the
    /// JobId, 0 when the job could not be queued.
    pub fn execute(&self, job_name: &str, run: Option<&RunDirective>, now: i64) -> u32 {
        let (job, ast) = {
            let Ok(cfg) = self.config.read() else { return 0 };
            let Some(job) = cfg.find_job(job_name).cloned() else { return 0 };
            let ast = run_request(&cfg, &job, run, now);
            (job, ast)
        };
        let ctx = ExecContext {
            session_id: 0,
            config: Some(self.config.clone()),
            session_mgr: self.session_mgr.clone(),
            catalog: self.catalog.clone(),
            api_mode: ApiMode::Human,
            bus_rx: None,
            job_manager: self.job_manager.clone(),
        };
        let result = with_overrides(RunOverrides::from_run(run),
            || crate::commands::job::start_job(&ast, &ctx, job));
        let job_id = result.json.as_ref().and_then(|j| j["run"]["jobid"].as_str().and_then(|id| id.parse().ok()))
            .or_else(|| result.text_output.strip_prefix("Job queued. JobId=")
                .and_then(|rest| rest.trim_end().parse().ok()))
            .unwrap_or(0);
        if job_id == 0 {
            tracing::warn!("Scheduler: job {job_name} not started: {}", result.text_output.trim_end());
        } else {
            tracing::info!("Scheduler: job {job_name} queued, JobId={job_id}");
        }
        job_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(extra: &str) -> (DirectorConfig, JobResource) {
        let cfg = DirectorConfig::from_str(&format!(
            "Director {{ Name=dir; Password=x }}\nCatalog {{ Name=c; DbName=b }}\nClient {{ Name=fd; Password=x }}\n\
             Storage {{ Name=File; Address=localhost; Password=x; Device=D; Media Type=File }}\n\
             Storage {{ Name=Other; Address=localhost; Password=x; Device=D; Media Type=File }}\n\
             Pool {{ Name=Full; Pool Type=Backup }}\nPool {{ Name=Inc; Pool Type=Backup }}\n\
             FileSet {{ Name=fs; Include {{ File=/etc }} }}\nMessages {{ Name=Standard }}\n\
             Job {{ Name=a; Type=Backup; Client=fd; FileSet=fs; Storage=File; Pool=Full; Messages=Standard; {extra} }}\n"))
            .unwrap();
        let job = cfg.find_job("a").unwrap().clone();
        (cfg, job)
    }

    fn run(spec: &str) -> RunDirective {
        match &bareos_config::parser::parse_config(&format!("Schedule {{\n Name = s\n Run = {spec}\n}}\n")).unwrap()[0] {
            bareos_config::Resource::Schedule(s) => s.runs[0].clone(),
            _ => unreachable!(),
        }
    }

    fn args(ast: &CommandAst) -> Vec<String> {
        ast.arguments.iter().filter(|a| a.key != "schedtime")
            .map(|a| match &a.value { Some(v) => format!("{}={v}", a.key), None => a.key.clone() }).collect()
    }

    #[test]
    fn request_takes_level_and_storage_of_the_run() {
        let (cfg, job) = job("");
        let r = run("Level=Incremental Storage=Other Pool=Inc daily at 1:00");
        assert_eq!(args(&run_request(&cfg, &job, Some(&r), 0)), ["job=a", "level=Incremental", "storage=Other", "yes"]);
        // without a level in Run or Job the type's default applies
        assert_eq!(args(&run_request(&cfg, &job, Some(&run("daily at 1:00")), 0)), ["job=a", "yes"]);
        let (cfg, job) = job_with_level();
        assert_eq!(args(&run_request(&cfg, &job, Some(&run("daily at 1:00")), 0)), ["job=a", "level=Differential", "yes"]);
    }

    fn job_with_level() -> (DirectorConfig, JobResource) {
        job("Level=Differential")
    }

    #[test]
    fn pool_overrides_follow_apply_pool_overrides() {
        let (_, job) = job("Full Backup Pool=Full");
        let pool = |spec: &str, level: &str| {
            with_overrides(RunOverrides::from_run(Some(&run(spec))), || {
                let pool = backup_pool(&job, level);
                let mut report = crate::job_messages::JobReport::default();
                let mut prio = 10;
                apply(0, &mut prio, &mut report);
                (pool, report.pool_source)
            })
        };
        assert_eq!(pool("daily", "F"), (None, String::new()));
        assert_eq!(pool("Pool=Inc daily", "F"), (Some("Inc".into()), "Run Pool override".into()));
        assert_eq!(pool("Pool=Inc IncrementalPool=Inc daily", "F"), (Some("Full".into()), "Job FullPool override".into()));
        assert_eq!(pool("IncrementalPool=Inc daily", "I"), (Some("Inc".into()), "Run IncPool override".into()));
        assert_eq!(pool("Pool=Inc FullPool=Full daily", "D"), (Some("Inc".into()), "Job resource".into()));
        // outside a scheduled start nothing changes
        assert_eq!(backup_pool(&job, "F"), None);
    }

    #[test]
    fn apply_sets_trigger_priority_sources_and_messages() {
        let mut report = crate::job_messages::JobReport { trigger: "User".into(), priority: 10, ..Default::default() };
        let mut prio = 10;
        with_overrides(RunOverrides::from_run(Some(&run("Priority=3 Storage=Other Accurate=yes Messages=M2 daily"))), || {
            assert_eq!(job_messages(4711), Some("M2".into()));
            apply(4711, &mut prio, &mut report)
        });
        assert_eq!((prio, report.priority, report.trigger.as_str()), (3, 3, "Scheduler"));
        assert_eq!(report.storage_source, "run override");
        assert!(report.accurate);
        assert_eq!(job_messages(4711), Some("M2".into()));
        assert_eq!(job_messages(4712), None);
        // Priority=0 keeps the Job's priority; no Run directive: trigger only
        let mut prio = 10;
        let mut report = crate::job_messages::JobReport::default();
        with_overrides(RunOverrides::from_run(None), || apply(1, &mut prio, &mut report));
        assert_eq!((prio, report.trigger.as_str()), (10, "Scheduler"));
    }
}
