//! Job Manager — Director-side tracking of all running job workers.
//!
//! Maintains a Vec of active jobs (up to 1000+ concurrent).
//! One monitor thread polls all worker pipes and handles CatReq.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tracing::{info, debug, warn, error};

use crate::catalog_worker::CatalogHandle;
use crate::commands::bvfs::Db;
use crate::job_worker::{JobHandle, PipeMessage};
use crate::message_bus::{MessageBus, BusMessage};
use crate::director_config::SharedConfig;

/// Director-side view of a running job.
pub struct DirJob {
    pub job_id: u32,
    pub job_name: String,
    pub phase: DirJobPhase,
    pub handle: JobHandle,
    pub start_time: i64,
    pub last_files: i64,
    pub last_bytes: i64,
    pub completion_error: Option<String>,
    completed_at: Option<std::time::Instant>,
    observed_status: char,
    worker_exit: Option<i32>,
    pending_end: Option<JobEnd>,
    /// Client and (write or read) Storage the job uses, for `cancel`
    /// (C++ jcr->dir_impl->res.client / write_storage).
    resources: Option<(String, String)>,
    /// JCR details beside the worker phase. Compatibility log C-048.
    jcr: JcrState,
}

/// JCR fields the Director keeps for a job (C++ JobControlRecord and
/// director_jcr_impl.h): queue position, SD status, daemon connections.
/// Compatibility log C-048.
#[derive(Default)]
struct JcrState {
    /// Worker parameters of a queued job, kept for the job codes of its
    /// run scripts afterwards.
    params: Option<crate::job_worker::JobParams>,
    /// JobPriority / AllowMixedPriority / sched_time of the jobq.
    priority: i32,
    allow_mixed_priority: bool,
    sched_time: i64,
    /// jobq.cc AcquireResources() succeeded (acquired_resource_locks).
    holds_resources: bool,
    /// Order in which the jobs left the queue (jobq running_jobs list).
    started_seq: u64,
    /// jcr->job_started (setJobStarted, before the FD's backup command).
    job_started: bool,
    /// jcr->file_bsock / jcr->store_bsock are set.
    fd_connected: bool,
    sd_connected: bool,
    /// jcr->dir_impl->SDJobStatus ('\0' none).
    sd_status: char,
    /// jcr->post_scripts_ran (RunScripts "AfterJob" happens once).
    post_scripts_ran: bool,
    /// Counted in num_jobs_run / RecentJobResultsList.
    recorded: bool,
    /// Job report data (resources and their sources), set when the job is
    /// started; None for jobs without an end-of-job report. C-051.
    report: Option<crate::job_messages::JobReport>,
}

/// What a queued job needs to start (jobq.cc JobqAdd). Compatibility log C-048.
pub struct QueuedStart {
    pub params: crate::job_worker::JobParams,
    pub priority: i32,
    /// jcr->sched_time: the job waits for it (JS_WaitStartTime).
    pub sched_time: i64,
}

/// Forks a job worker (job_worker::spawn_job_worker; a stand-in in tests).
pub type Spawner = fn(u32, &str, &crate::job_worker::JobParams) -> std::io::Result<JobHandle>;

/// Job data the run scripts need for their job codes (edit_job_codes).
pub struct ScriptJob {
    pub job_id: u32,
    pub job_name: String,
    pub unique_job: String,
    pub status: char,
    pub files: i64,
    pub bytes: i64,
    pub client: String,
    pub storage: String,
    pub params: Option<crate::job_worker::JobParams>,
}

struct JobEnd {
    term_code: i32, files: i64, bytes: i64, errors: i32,
    vol_session_id: i32, vol_session_time: i32,
}

/// Bareos lib/jcr.cc: retain the first equal-priority error; Incomplete wins.
fn merge_status(previous: char, next: char) -> char {
    let priority = status_priority;
    if priority(next) > priority(previous) || (priority(next) == 0 && priority(previous) == 0) { next } else { previous }
}

/// lib/jcr.cc GetStatusPriority().
fn status_priority(status: char) -> u8 { match status { 'I' => 10, 'E' | 'f' | 'A' => 9, 'e' => 8, 'D' => 7, _ => 0 } }

/// jcr.h IsJobCanceled().
pub fn is_job_canceled(status: char) -> bool { matches!(status, 'A' | 'E' | 'f') }

/// job.cc CancelJob(): the states of a job that has not left the job queue.
pub fn is_queued_status(status: char) -> bool { matches!(status, 'C' | 'j' | 'c' | 's' | 'p' | 'd' | 't') }

/// Director-side job phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DirJobPhase {
    #[default]
    Queued,
    Starting,
    Running,
    Paused,
    Canceling,
    WaitingCatReq,
    WaitingMedia,
    WaitingMount,
    Finishing,
    Completed,
    Error,
}

impl DirJobPhase {
    pub fn name(&self) -> &str {
        match self {
            Self::Queued => "Queued",
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Paused => "Paused",
            Self::Canceling => "Canceling",
            Self::WaitingCatReq => "WaitingCatReq",
            Self::WaitingMedia => "WaitingMedia",
            Self::WaitingMount => "WaitingMount",
            Self::Finishing => "Finishing",
            Self::Completed => "Completed",
            Self::Error => "Error",
        }
    }

    /// (Test helper since C-048: ActiveJob carries the JCR status.)
    /// Upstream JobStatus code of the phase (lib/jcr.h JS_*): a job whose
    /// worker has not reached its daemons yet is JS_Created; storage waits
    /// are JS_WaitMedia/JS_WaitMount, a paused job JS_Blocked.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn job_status(&self) -> char {
        match self {
            Self::Queued | Self::Starting => 'C',
            Self::Running | Self::WaitingCatReq | Self::Finishing => 'R',
            Self::Paused => 'B',
            Self::Canceling => 'A',
            Self::WaitingMedia => 'm',
            Self::WaitingMount => 'M',
            Self::Completed => 'T',
            Self::Error => 'f',
        }
    }

    /// C++ `jcr->job_started`: the job left the queue and runs.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn job_started(&self) -> bool {
        !matches!(self, Self::Queued | Self::Starting)
    }
}

/// Owned snapshot; callers release the manager lock before waiting or querying DB.
#[derive(Clone, Default)]
pub struct ActiveJob {
    pub id: u32,
    pub name: String,
    pub unique_name: String,
    pub phase: DirJobPhase,
    /// jcr->JobStatus (C-048).
    pub status: char,
    /// jcr->dir_impl->SDJobStatus, '\0' when the SD reported nothing.
    pub sd_status: char,
    /// jcr->job_started.
    pub job_started: bool,
    /// jcr->file_bsock / jcr->store_bsock.
    pub fd_connected: bool,
    pub sd_connected: bool,
    /// Client and Storage resource names ("" when unknown).
    pub client: String,
    pub storage: String,
    /// jcr->sched_time.
    pub sched_time: i64,
}

impl ActiveJob {
    /// director_jcr_impl.h JobWaiting().
    pub fn waiting(&self) -> bool {
        self.job_started && (matches!(self.status, 'F' | 'S' | 'm' | 'M' | 's' | 'j' | 'c' | 'd' | 'p')
            || matches!(self.sd_status, 'm' | 'M' | 'q' | 'd'))
    }
}

/// Manages all active job workers — Vec-based for up to 1000+ jobs.
pub struct JobManager {
    jobs: Vec<DirJob>,
    bus: Arc<MessageBus>,
    config: Option<SharedConfig>,
    catalog: Option<CatalogHandle>,
    /// Catalog state of the SD catalog requests per JobId (catreq.rs).
    catreq_jobs: std::collections::HashMap<u32, crate::catreq::CatReqJob>,
    /// jobq.cc: jobs waiting for their start time, the priority-sorted
    /// waiting queue and the ready queue (JobIds). Compatibility log C-048.
    delayed: Vec<u32>,
    waiting: Vec<u32>,
    ready: std::collections::VecDeque<u32>,
    spawner: Spawner,
    next_seq: u64,
    /// lib/jcr.cc num_jobs_run and RecentJobResultsList (10 JobIds, end order).
    jobs_run: usize,
    recent: std::collections::VecDeque<u32>,
    /// jobq_server() looks at the queue every 2 seconds, or when a job ended.
    last_schedule: std::time::Instant,
    /// jobq_server(): a job moved from the waiting to the ready queue runs
    /// after the server's "pause for 2 seconds" (jobq.cc:550); per JobId the
    /// earliest start. Compatibility log C-058.
    ready_at: std::collections::HashMap<u32, std::time::Instant>,
    start_delay: std::time::Duration,
    /// sched_wait(): next look at a delayed job (every 30 seconds).
    wait_check: std::collections::HashMap<u32, std::time::Instant>,
    wait_interval: std::time::Duration,
    /// sched_wait() "waiting N seconds" job messages not yet posted.
    wait_messages: Vec<(u32, std::time::Instant, String)>,
}

/// RecentJobResultsList max_count_recent_job_results.
const MAX_RECENT_JOB_RESULTS: usize = 10;

impl JobManager {
    pub fn new(bus: Arc<MessageBus>, config: Option<SharedConfig>, catalog: Option<CatalogHandle>) -> Self {
        Self { jobs: Vec::new(), bus, config, catalog, catreq_jobs: std::collections::HashMap::new(),
            delayed: Vec::new(), waiting: Vec::new(), ready: std::collections::VecDeque::new(),
            spawner: crate::job_worker::spawn_job_worker, next_seq: 0,
            jobs_run: 0, recent: std::collections::VecDeque::new(), last_schedule: std::time::Instant::now(),
            ready_at: std::collections::HashMap::new(),
            start_delay: std::time::Duration::from_secs(if cfg!(test) { 0 } else { 2 }),
            wait_check: std::collections::HashMap::new(),
            wait_interval: std::time::Duration::from_secs(if cfg!(test) { 0 } else { 30 }),
            wait_messages: Vec::new() }
    }

    /// The jobq_server pause before a ready job runs (tests).
    #[cfg(test)]
    pub fn set_start_delay(&mut self, delay: std::time::Duration) {
        self.start_delay = delay;
    }

    pub fn add_job(&mut self, job_id: u32, job_name: &str, handle: JobHandle) {
        self.jobs.push(DirJob {
            job_id,
            job_name: job_name.to_string(),
            phase: DirJobPhase::Starting,
            handle,
            start_time: now_unix(),
            last_files: 0,
            last_bytes: 0,
            completion_error: None, completed_at: None, observed_status: 'C',
            worker_exit: None, pending_end: None, resources: None,
            jcr: JcrState { holds_resources: true, ..Default::default() },
        });
        debug!("JobManager: added job {job_id} '{job_name}' (total: {})", self.jobs.len());
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn active_count(&self) -> usize {
        self.jobs.iter().filter(|j| !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error)).count()
    }

    pub fn get_job(&self, job_id: u32) -> Option<&DirJob> {
        self.jobs.iter().find(|j| j.job_id == job_id)
    }

    /// List running jobs: (job_id, job_name, phase).
    pub fn running_jobs(&self) -> Vec<(u32, String, String)> {
        self.jobs.iter()
            .filter(|j| !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error))
            .map(|j| (j.job_id, j.job_name.clone(), j.phase.name().to_string()))
            .collect()
    }

    pub fn active_jobs(&self) -> Vec<ActiveJob> {
        self.jobs.iter().filter(|job| !matches!(job.phase, DirJobPhase::Completed | DirJobPhase::Error))
            .map(|job| {
                let (client, storage) = job.resources.clone().unwrap_or_default();
                ActiveJob { id: job.job_id, name: job.job_name.clone(),
                    unique_name: job.handle.unique_job.clone(), phase: job.phase, status: job.status(),
                    sd_status: job.jcr.sd_status, job_started: job.jcr.job_started,
                    fd_connected: job.jcr.fd_connected, sd_connected: job.jcr.sd_connected,
                    client, storage, sched_time: job.jcr.sched_time }
            }).collect()
    }

    /// lib/jcr.cc num_jobs_run: jobs that ended since the Director started.
    pub fn finished_count(&self) -> usize {
        self.jobs_run
    }

    /// JobIds of the last 10 jobs that ended since the Director started, in
    /// the order they ended (C++ RecentJobResultsList without state file).
    pub fn finished_job_ids(&self) -> Vec<u32> {
        self.recent.iter().copied().collect()
    }

    /// Queue a job (jobq.cc JobqAdd): it waits for its start time, its
    /// priority and the Director/Job/Client/Storage Maximum Concurrent Jobs
    /// before its worker is forked. Compatibility log C-048.
    pub fn enqueue_job(&mut self, job_id: u32, job_name: &str, start: QueuedStart, client: &str, storage: &str) {
        let handle = JobHandle::pending(job_id, job_name, &start.params.unique_job);
        let allow_mixed_priority = self.allow_mixed_priority(job_name);
        let delayed = start.sched_time > now_unix();
        self.jobs.push(DirJob {
            job_id, job_name: job_name.to_string(), phase: DirJobPhase::Queued, handle,
            start_time: now_unix(), last_files: 0, last_bytes: 0,
            completion_error: None, completed_at: None, observed_status: if delayed { 't' } else { 'C' },
            worker_exit: None, pending_end: None, resources: Some((client.to_string(), storage.to_string())),
            jcr: JcrState { priority: start.priority, allow_mixed_priority, sched_time: start.sched_time,
                params: Some(start.params), ..Default::default() },
        });
        if delayed {
            // jobq.cc sched_wait(): the message, then a look every 30 seconds.
            let wtime = self.get_job(job_id).map_or(0, |j| j.jcr.sched_time - now_unix()).max(0);
            let unique = self.get_job(job_id).map(|j| j.handle.unique_job.clone()).unwrap_or_default();
            // Posted by the monitor shortly after the run command has replied
            // (the sched_wait thread starts after JobqAdd returned).
            self.wait_messages.push((job_id, std::time::Instant::now() + std::time::Duration::from_millis(200),
                format!("Job {unique} waiting {wtime} seconds for scheduled start time.\n")));
            self.wait_check.insert(job_id, std::time::Instant::now() + self.wait_interval.min(std::time::Duration::from_secs(wtime as u64)));
            self.delayed.push(job_id)
        } else {
            self.insert_waiting(job_id)
        }
        debug!("JobManager: queued job {job_id} '{job_name}'");
        self.schedule();
    }

    /// Replace the worker fork (tests start stand-in processes).
    #[cfg(test)]
    pub fn set_spawner(&mut self, spawner: Spawner) {
        self.spawner = spawner;
    }

    /// AllowMixedPriority of the Job resource (with its JobDefs).
    fn allow_mixed_priority(&self, job_name: &str) -> bool {
        let Some(config) = self.config.as_ref().and_then(|c| c.read().ok()) else { return false };
        bareos_config::jobdefs::effective(&config.raw, "job", job_name).unwrap_or_default().iter().rev()
            .find(|e| e.directive.norm_key() == "allowmixedpriority")
            .and_then(|e| e.directive.first())
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "yes" | "true"))
    }

    /// JobqAdd(): insert before the first waiting job of a higher priority value.
    fn insert_waiting(&mut self, job_id: u32) {
        let priority = self.get_job(job_id).map_or(0, |j| j.jcr.priority);
        let at = self.waiting.iter().position(|id| self.get_job(*id).is_some_and(|j| j.jcr.priority > priority))
            .unwrap_or(self.waiting.len());
        self.waiting.insert(at, job_id);
    }

    /// Maximum Concurrent Jobs of the Director, Job, Client and Storage
    /// (unlimited without a configuration).
    fn limits(&self, job_name: &str, client: &str, storage: &str) -> (usize, usize, usize, usize) {
        let Some(config) = self.config.as_ref().and_then(|c| c.read().ok()) else {
            return (usize::MAX, usize::MAX, usize::MAX, usize::MAX);
        };
        (config.director.max_concurrent_jobs as usize,
         config.find_job(job_name).map_or(usize::MAX, |j| j.max_concurrent_jobs as usize),
         config.find_client(client).map_or(usize::MAX, |c| c.max_concurrent_jobs as usize),
         config.find_storage(storage).map_or(usize::MAX, |s| s.max_concurrent_jobs as usize))
    }

    /// Jobs whose engine runs (jobq running_jobs): forked and not ended.
    fn running(&self) -> impl Iterator<Item = &DirJob> {
        self.jobs.iter().filter(|j| j.handle.spawned() && !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error))
    }

    /// jobq_server(): start ready jobs while the Director has workers left,
    /// then move the waiting jobs whose priority and resources allow it to
    /// the ready queue (JS_WaitPriority/WaitStoreRes/WaitClientRes/WaitJobRes
    /// otherwise).
    fn schedule(&mut self) {
        self.last_schedule = std::time::Instant::now();
        let now = now_unix();
        // sched_wait(): a canceled job leaves its wait at the next 30-second look.
        let instant = std::time::Instant::now();
        let checks = self.wait_check.clone();
        let looked = |id: &u32| checks.get(id).is_none_or(|at| *at <= instant);
        let due: Vec<u32> = self.delayed.iter().copied()
            .filter(|id| self.get_job(*id).is_none_or(|j| j.jcr.sched_time <= now
                || (is_job_canceled(j.observed_status) && looked(id))))
            .collect();
        for id in self.delayed.clone() {
            if looked(&id) && !due.contains(&id) {
                let remaining = self.get_job(id).map_or(0, |j| j.jcr.sched_time - now).max(0) as u64;
                self.wait_check.insert(id, instant + self.wait_interval.min(std::time::Duration::from_secs(remaining)));
            }
        }
        for id in due {
            self.wait_check.remove(&id);
            self.delayed.retain(|d| *d != id);
            match self.get_job(id).map(|j| is_job_canceled(j.observed_status)) {
                // JobqAdd() of a canceled job: to the front of the ready queue.
                Some(true) => self.ready.push_front(id),
                Some(false) => self.insert_waiting(id),
                None => {}
            }
        }
        self.start_ready();
        if let Some(&first) = self.waiting.first() {
            let mut running: Vec<&DirJob> = self.running().collect();
            running.sort_by_key(|j| j.jcr.started_seq);
            let (wanted, running_allow_mix) = match running.first() {
                Some(job) => (job.jcr.priority, running.iter().all(|j| j.jcr.allow_mixed_priority)),
                None => (self.get_job(first).map_or(0, |j| j.jcr.priority), false),
            };
            for id in self.waiting.clone() {
                let Some(job) = self.get_job(id) else { self.waiting.retain(|w| *w != id); continue };
                let (priority, mix) = (job.jcr.priority, job.jcr.allow_mixed_priority);
                if !(priority == wanted || (priority < wanted && mix && running_allow_mix)) {
                    self.set_status(id, 'p');
                    break;
                }
                if !self.acquire_resources(id) { continue; }
                self.waiting.retain(|w| *w != id);
                self.ready.push_back(id);
                self.ready_at.insert(id, std::time::Instant::now() + self.start_delay);
            }
        }
        self.start_ready();
    }

    /// jobq.cc AcquireResources(): Storage, Client, then Job concurrency.
    fn acquire_resources(&mut self, id: u32) -> bool {
        let Some(job) = self.get_job(id) else { return false };
        let (client, storage) = job.resources.clone().unwrap_or_default();
        let name = job.job_name.clone();
        let (_, max_job, max_client, max_storage) = self.limits(&name, &client, &storage);
        let holders = |pred: &dyn Fn(&DirJob) -> bool| self.jobs.iter()
            .filter(|j| j.jcr.holds_resources && !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error) && pred(j))
            .count();
        let status = if !storage.is_empty()
            && holders(&|j| j.resources.as_ref().is_some_and(|r| r.1 == storage)) >= max_storage {
            's'
        } else if !client.is_empty()
            && holders(&|j| j.resources.as_ref().is_some_and(|r| r.0 == client)) >= max_client {
            'c'
        } else if holders(&|j| j.job_name == name) >= max_job {
            'j'
        } else {
            if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == id) { job.jcr.holds_resources = true; }
            return true;
        };
        self.set_status(id, status);
        false
    }

    /// setJobStatusWithPriorityCheck().
    fn set_status(&mut self, id: u32, status: char) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == id) {
            job.observed_status = merge_status(job.observed_status, status);
        }
    }

    /// Fork the workers of ready jobs while fewer than the Director's
    /// Maximum Concurrent Jobs run. A canceled job ends without a worker.
    fn start_ready(&mut self) {
        let mut children = Vec::new();
        self.start_ready_jobs(&mut children);
        // Jobs a Director-run job started (internal_jobs::ChildStart): JobqAdd.
        for child in children {
            self.bus.post(BusMessage::JobStatus { job_id: child.job_id, job_name: child.job_name.clone(),
                status: "Queued".to_string(), timestamp: now_unix() });
            self.enqueue_job(child.job_id, &child.job_name, child.start, &child.client, &child.storage);
            self.set_job_report(child.job_id, child.report);
        }
    }

    fn start_ready_jobs(&mut self, children: &mut Vec<crate::internal_jobs::ChildStart>) {
        while let Some(&id) = self.ready.front() {
            let canceled = self.get_job(id).is_some_and(|j| is_job_canceled(j.observed_status));
            if !canceled {
                let name = self.get_job(id).map(|j| j.job_name.clone()).unwrap_or_default();
                let (max_workers, ..) = self.limits(&name, "", "");
                if self.running().count() >= max_workers { break; }
                if self.ready_at.get(&id).is_some_and(|at| *at > std::time::Instant::now()) { break; }
            }
            self.ready.pop_front();
            self.ready_at.remove(&id);
            let spawner = self.spawner;
            let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == id) else { continue };
            if canceled {
                // job_thread() of a canceled job: no daemons, only its cleanup.
                // Job types whose "Version:" message comes from job_thread()
                // (not at queue time) get it here as well.
                if job.jcr.params.as_ref().is_some_and(|p| p.job_type == crate::job_worker::WorkerJobType::Admin
                    || p.job_type.internal().is_some()) {
                    crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                        job_id: id, job_name: job.job_name.clone(), msg_type: crate::message_bus::MsgType::Info,
                        text: crate::job_messages::version_text(), timestamp: now_unix() });
                }
                job.pending_end = Some(JobEnd { term_code: b'A' as i32, files: 0, bytes: 0, errors: 0,
                    vol_session_id: 0, vol_session_time: 0 });
                job.worker_exit = Some(0);
                continue;
            }
            let Some(params) = job.jcr.params.clone() else { continue };
            if let Some(kind) = params.job_type.internal() {
                // job_thread() of a job the Director runs itself (C-058):
                // JS_Running, UpdateJobStartRecord, "Version:", the job, its end.
                job.start_time = now_unix();
                if let Some(report) = job.jcr.report.as_mut() { report.start_time = job.start_time; }
                self.next_seq += 1;
                job.jcr.started_seq = self.next_seq;
                job.jcr.job_started = true;
                job.observed_status = merge_status(job.observed_status, 'R');
                let outcome = match self.config.as_ref().and_then(|c| c.read().ok()) {
                    Some(config) => crate::internal_jobs::run(&config, self.catalog.as_ref(), &crate::internal_jobs::Request {
                        kind, job_id: id, unique_job: &params.unique_job, job_name: &job.job_name, pool: &params.pool_name }),
                    None => crate::internal_jobs::Outcome { status: 'f', ..Default::default() },
                };
                if let Some(cat) = &self.catalog {
                    let start = job.start_time;
                    let pool = outcome.pool_id.as_deref().map(|p| format!(",PoolId={p}")).unwrap_or_default();
                    if let Err(e) = cat.raw_query(&format!(
                        "UPDATE Job SET JobStatus='{}',StartTime=to_timestamp({start}),JobTDate={start}{pool} WHERE JobId={id}",
                        job.observed_status)) {
                        warn!("Job {id}: UpdateJobStartRecord: {e}");
                    }
                }
                let mut messages = vec![(crate::message_bus::MsgType::Info, crate::job_messages::version_text())];
                messages.extend(outcome.messages);
                children.extend(outcome.children);
                // Messages of the worker jobs a control job started go
                // between its own (outcome.other_messages, C-058).
                let mut others = outcome.other_messages.into_iter().peekable();
                for (i, (msg_type, text)) in messages.into_iter().enumerate() {
                    while let Some((_, other_id, other_name, other_type, other_text)) = others.next_if(|o| o.0 + 1 <= i) {
                        crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                            job_id: other_id, job_name: other_name, msg_type: other_type, text: other_text, timestamp: now_unix() });
                    }
                    crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                        job_id: id, job_name: job.job_name.clone(), msg_type, text, timestamp: now_unix() });
                }
                for (_, other_id, other_name, msg_type, text) in others {
                    crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                        job_id: other_id, job_name: other_name, msg_type, text, timestamp: now_unix() });
                }
                if let Some(report) = job.jcr.report.as_mut() { report.selected = outcome.selected; }
                job.pending_end = Some(JobEnd { term_code: outcome.status as i32, files: 0, bytes: 0, errors: 0,
                    vol_session_id: 0, vol_session_time: 0 });
                job.worker_exit = Some(0);
                continue;
            }
            match spawner(id, &job.job_name, &params) {
                Ok(handle) => {
                    info!("Job {id} '{}': worker pid={}", job.job_name, handle.pid);
                    job.handle = handle;
                    job.phase = DirJobPhase::Starting;
                    job.start_time = now_unix();
                    // The report shows jr.StartTime (job_thread), not the queue time.
                    if let Some(report) = job.jcr.report.as_mut() { report.start_time = job.start_time; }
                    self.next_seq += 1;
                    job.jcr.started_seq = self.next_seq;
                    // job_thread(): JS_Running, then UpdateJobStartRecord().
                    job.observed_status = merge_status(job.observed_status, 'R');
                    if let Some(cat) = &self.catalog {
                        let start = job.start_time;
                        if let Err(e) = cat.raw_query(&format!(
                            "UPDATE Job SET JobStatus='{}',StartTime=to_timestamp({start}),JobTDate={start} WHERE JobId={id}",
                            job.observed_status)) {
                            warn!("Job {id}: UpdateJobStartRecord: {e}");
                        }
                    }
                }
                Err(e) => {
                    error!("Job {id}: fork failed: {e}");
                    job.pending_end = Some(JobEnd { term_code: b'E' as i32, files: 0, bytes: 0, errors: 1,
                        vol_session_id: 0, vol_session_time: 0 });
                    job.worker_exit = Some(1);
                }
            }
        }
    }

    /// RunScripts(jcr, ..., "AfterJob") runs once per job (post_scripts_ran):
    /// the job data for the scripts, None when they ran already.
    pub fn take_post_scripts(&mut self, job_id: u32) -> Option<ScriptJob> {
        let job = self.jobs.iter_mut().find(|j| j.job_id == job_id)?;
        if std::mem::replace(&mut job.jcr.post_scripts_ran, true) { return None; }
        let (client, storage) = job.resources.clone().unwrap_or_default();
        Some(ScriptJob { job_id, job_name: job.job_name.clone(), unique_job: job.handle.unique_job.clone(),
            status: job.status(), files: job.last_files, bytes: job.last_bytes, client, storage,
            params: job.jcr.params.clone() })
    }

    /// Record the Client and Storage resource names a started job uses.
    pub fn set_job_resources(&mut self, job_id: u32, client: &str, storage: &str) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.resources = Some((client.to_string(), storage.to_string()));
        }
    }

    /// Client and Storage resource names recorded by [`Self::set_job_resources`].
    pub fn job_resources(&self, job_id: u32) -> Option<(String, String)> {
        self.get_job(job_id)?.resources.clone()
    }

    /// Record the job report data of a started job (end-of-job summary).
    pub fn set_job_report(&mut self, job_id: u32, report: crate::job_messages::JobReport) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.jcr.report = Some(report);
        }
    }

    /// Put a job into the state of `phase` as its worker would (tests).
    #[cfg(test)]
    pub fn set_phase(&mut self, job_id: u32, phase: DirJobPhase) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.phase = phase;
            let started = phase.job_started();
            job.jcr.job_started = started;
            job.jcr.fd_connected = started;
            job.jcr.sd_connected = started;
            match phase.job_status() {
                status @ ('m' | 'M') => { job.jcr.sd_status = status; job.observed_status = 'R'; }
                status => job.observed_status = status,
            }
        }
    }

    /// Record the daemon connections and JobStatus of a job (tests).
    #[cfg(test)]
    pub fn set_connections(&mut self, job_id: u32, fd: bool, sd: bool, status: char) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.jcr.fd_connected = fd;
            job.jcr.sd_connected = sd;
            job.observed_status = status;
        }
    }

    /// job.cc CancelJob(): JS_Canceled. A queued job moves from the waiting
    /// to the front of the ready queue (JobqRemove) and ends there without a
    /// worker; a forked worker gets SIGTERM.
    pub fn cancel_job(&mut self, job_id: u32) -> bool {
        let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id
            && !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error)) else { return false };
        if !job.handle.spawned() {
            if job.worker_exit.is_some() { return false; }
            job.observed_status = merge_status(job.observed_status, 'A');
            job.phase = DirJobPhase::Canceling;
            if let Some(at) = self.waiting.iter().position(|id| *id == job_id) {
                self.waiting.remove(at);
                self.ready.push_front(job_id);
            }
            self.schedule();
            return true;
        }
        if !job.handle.cancel() { return false; }
        job.observed_status = merge_status(job.observed_status, 'A');
        job.phase = DirJobPhase::Canceling;
        true
    }


    pub fn completion_error(&self, id: Option<u32>, since: std::time::Instant) -> Option<String> {
        self.jobs.iter().find(|job| match id {
            Some(id) => job.job_id == id && job.completion_error.is_some(),
            None => job.completed_at.is_some_and(|time| time >= since) && job.completion_error.is_some(),
        }).and_then(|job| job.completion_error.as_ref().map(|error| format!("Job {} catalog completion failed: {error}", job.job_id)))
    }

    pub fn pause_job(&mut self, job_id: u32) -> bool {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.handle.pause();
            job.phase = DirJobPhase::Paused;
            true
        } else {
            false
        }
    }

    pub fn resume_job(&mut self, job_id: u32) -> bool {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.handle.resume();
            job.phase = DirJobPhase::Running;
            true
        } else {
            false
        }
    }

    /// Poll all active worker pipes. Non-blocking.
    /// Handles CatReq, status updates, state changes, job completion.
    pub fn poll_all(&mut self) {
        if !self.wait_messages.is_empty() {
            let now = std::time::Instant::now();
            let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.wait_messages).into_iter().partition(|m| m.1 <= now);
            self.wait_messages = later;
            for (job_id, _, text) in due {
                let job_name = self.get_job(job_id).map(|j| j.job_name.clone()).unwrap_or_default();
                crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                    job_id, job_name, msg_type: crate::message_bus::MsgType::Info, text, timestamp: now_unix() });
            }
        }
        for job in &mut self.jobs {
            if matches!(job.phase, DirJobPhase::Completed | DirJobPhase::Error) {
                job.handle.try_wait();
                continue;
            }

            // Reap first, but drain all final pipe messages before publishing completion.
            if job.worker_exit.is_none() { job.worker_exit = job.handle.try_wait(); }

            // Read pipe messages (non-blocking — try_read_message returns None if no data)
            while let Some(msg) = job.handle.try_read_message() {
                match msg {
                    PipeMessage::State { state, .. } => {
                        debug!("JobManager: job {} state={state}", job.job_id);
                        job.worker_state(&state);
                        if matches!(job.observed_status, 'A' | 'I' | 'E' | 'f' | 'e' | 'D')
                            && matches!(state.as_str(), "Running" | "Paused" | "WaitingMedia" | "WaitMedia" | "WaitingMount" | "WaitMount") { continue; }
                        job.phase = match state.as_str() {
                            "Running" => DirJobPhase::Running,
                            "Paused" => DirJobPhase::Paused,
                            "WaitingMedia" | "WaitMedia" => DirJobPhase::WaitingMedia,
                            "WaitingMount" | "WaitMount" => DirJobPhase::WaitingMount,
                            "Finishing" => DirJobPhase::Finishing,
                            // JobEnd commits the catalog outcome before the job disappears.
                            "Completed" | "Error" => DirJobPhase::Finishing,
                            _ => job.phase,
                        };
                    }

                    PipeMessage::JobStatus { status, .. } => {
                        if let Some(status) = char::from_u32(status as u32) {
                            // getmsg.cc SetJcrSdJobStatus(): the SD status is
                            // kept apart; only Incomplete and FatalError reach
                            // the job's own status.
                            job.jcr.sd_status = status;
                            if matches!(status, 'I' | 'f') {
                                job.observed_status = merge_status(job.observed_status, status);
                            }
                            if status_priority(job.observed_status) == 0 || job.observed_status == status {
                                match status {
                                    'M' => job.phase = DirJobPhase::WaitingMount,
                                    'm' => job.phase = DirJobPhase::WaitingMedia,
                                    'R' => job.phase = DirJobPhase::Running,
                                    _ => {},
                                }
                            }
                        }
                    }

                    PipeMessage::Status { files, bytes, errors, .. } => {
                        job.last_files = files;
                        job.last_bytes = bytes;
                    }

                    PipeMessage::CatReq { request, .. } => {
                        debug!("JobManager: job {} CatReq: {}", job.job_id, request.chars().take(60).collect::<String>());
                        let previous_phase = job.phase;
                        job.phase = DirJobPhase::WaitingCatReq;
                        let reply = catreq_session(&mut self.catreq_jobs, &self.catalog, &self.config, job)
                            .dispatch(self.catalog.as_ref().map(|c| c as &dyn Db), &batch_opener(&self.catalog), request.as_bytes());
                        if let Some((files, bytes)) = reply.job_counts {
                            job.last_files = files as i64;
                            job.last_bytes = bytes as i64;
                        }
                        post_catreq_messages(&self.bus, self.config.as_ref(), job, reply.messages);
                        job.handle.send_message(&PipeMessage::CatResp { response: reply.to_sd });
                        job.phase = previous_phase;
                    }

                    PipeMessage::UpdCat { data, .. } => {
                        let data = hex::decode(&data).unwrap_or_default();
                        let reply = catreq_session(&mut self.catreq_jobs, &self.catalog, &self.config, job)
                            .dispatch(self.catalog.as_ref().map(|c| c as &dyn Db), &batch_opener(&self.catalog), &data);
                        post_catreq_messages(&self.bus, self.config.as_ref(), job, reply.messages);
                    }

                    PipeMessage::JobEnd { term_code, files, bytes, errors,
                                          vol_session_id, vol_session_time, .. } => {
                        job.observed_status = merge_status(job.observed_status,
                            crate::job_worker::catalog_job_status(term_code).chars().next().unwrap());
                        job.last_files = files;
                        job.last_bytes = bytes;
                        job.phase = DirJobPhase::Finishing;
                        job.jcr.fd_connected = false;
                        job.jcr.sd_connected = false;
                        job.pending_end = Some(JobEnd {term_code, files, bytes, errors,
                            vol_session_id: vol_session_id.unwrap_or(0), vol_session_time: vol_session_time.unwrap_or(0)});
                    }

                    PipeMessage::Jmsg { msg_type, mtime, text, raw, .. } => {
                        if let Some(config) = self.config.as_ref().and_then(|c| c.read().ok()) {
                            if raw {
                                // getmsg.cc: a daemon's "Jmsg Job=.. type=.. level=.." line
                                crate::console_msg::dispatch_job_message(&config, &job.job_name, job.job_id, msg_type, mtime, &text);
                            } else if let Some(kind) = crate::job_messages::msg_type_of(msg_type) {
                                crate::console_msg::job_message(&config, &job.job_name, job.job_id, kind, mtime, &text);
                            }
                        }
                    }

                    _ => {}
                }
            }
            if let Some(exit_code) = job.worker_exit {
                // msgchan.cc EndTransaction() and backup.cc WriteBatchFileRecords().
                if let Some(mut state) = self.catreq_jobs.remove(&job.job_id) {
                    let messages = state.end_of_job(self.catalog.as_ref().map(|c| c as &dyn Db), &batch_opener(&self.catalog));
                    post_catreq_messages(&self.bus, self.config.as_ref(), job, messages);
                }
                let ending = job.pending_end.take().unwrap_or_else(|| JobEnd {
                    term_code: b'E' as i32, files: job.last_files, bytes: job.last_bytes,
                    errors: 1, vol_session_id: 0, vol_session_time: 0,
                });
                job.observed_status = merge_status(job.observed_status,
                    crate::job_worker::catalog_job_status(ending.term_code).chars().next().unwrap());
                if exit_code != 0 { job.observed_status = merge_status(job.observed_status, 'E'); }
                // MigrationCleanup() of a Copy/Migrate worker job (migrate.rs, C-058).
                if let Some(start_time) = job.jcr.report.as_ref()
                    .filter(|r| matches!(r.kind, crate::job_messages::ReportKind::MigrateWorker(_))).map(|r| r.start_time) {
                    let end_time = now_unix();
                    let end = crate::migrate::WorkerEnd {
                        status: job.observed_status, sd_status: job.jcr.sd_status,
                        sd_files: ending.files.max(0) as u64, sd_bytes: ending.bytes.max(0) as u64,
                        sd_errors: ending.errors.max(0) as u32, job_errors: 0,
                        vol_session_id: ending.vol_session_id, vol_session_time: ending.vol_session_time,
                        start_time, end_time };
                    let director = self.config.as_ref().and_then(|c| c.read().ok()).map(|c| c.director.name.clone()).unwrap_or_default();
                    if let Some((status, messages)) = crate::migrate::cleanup(&director, self.catalog.as_ref(), job.job_id, &end) {
                        job.observed_status = status;
                        for (msg_type, text) in messages {
                            crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                                job_id: job.job_id, job_name: job.job_name.clone(), msg_type, text, timestamp: end_time });
                        }
                        let success = matches!(status, 'T' | 'W');
                        job.phase = if success { DirJobPhase::Completed } else { DirJobPhase::Error };
                        job.completed_at = Some(std::time::Instant::now());
                        self.bus.post(BusMessage::JobStatus { job_id: job.job_id, job_name: job.job_name.clone(),
                            status: if success { "Success" } else { "Error" }.into(), timestamp: end_time });
                        continue;
                    }
                }
                // NativeBackupCleanup: terminated with errors is JS_Warnings
                if job.observed_status == 'T' && ending.errors > 0 { job.observed_status = 'W'; }
                let db_status = job.observed_status.to_string();
                let result = self.catalog.as_ref().ok_or_else(|| "catalog is unavailable".to_string()).and_then(|cat| cat.update_job_end(
                    job.job_id as i32, &db_status, now_unix(), ending.files, ending.bytes, ending.errors,
                    ending.vol_session_id, ending.vol_session_time));
                job.completion_error = result.err();
                if let Some(error) = &job.completion_error { error!("JobManager: job {} final catalog update failed: {error}", job.job_id); }
                let success = job.completion_error.is_none() && matches!(job.observed_status, 'T' | 'W');
                job.phase = if success { DirJobPhase::Completed } else { DirJobPhase::Error };
                job.completed_at = Some(std::time::Instant::now());
                let status = if success { "Success" } else { "Error" };
                // NativeBackupCleanup / NativeRestoreCleanup: the job report
                if let Some(report) = &job.jcr.report {
                    let end_time = now_unix();
                    let fd_status = crate::job_worker::catalog_job_status(ending.term_code).chars().next().unwrap_or('E');
                    let outcome = crate::job_messages::JobOutcome {
                        job_id: job.job_id, status: job.observed_status, end_time,
                        files: ending.files.max(0) as u64, bytes: ending.bytes.max(0) as u64,
                        read_bytes: ending.bytes.max(0) as u64, sd_files: ending.files.max(0) as u64,
                        sd_bytes: ending.bytes.max(0) as u64, errors: ending.errors.max(0) as u32, sd_errors: 0,
                        vol_session_id: ending.vol_session_id, vol_session_time: ending.vol_session_time,
                        fd_status, sd_status: job.jcr.sd_status, vss: false, encrypt: false,
                    };
                    let director = self.config.as_ref().and_then(|c| c.read().ok()).map(|c| c.director.name.clone()).unwrap_or_default();
                    for (msg_type, text) in crate::job_messages::end_of_job(&director, self.catalog.as_ref(), report, &outcome) {
                        crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                            job_id: job.job_id, job_name: job.job_name.clone(), msg_type, text, timestamp: end_time });
                    }
                }
                self.bus.post(BusMessage::JobStatus {job_id: job.job_id, job_name: job.job_name.clone(), status: status.into(), timestamp: now_unix()});
                info!("JobManager: job {} worker exited (code={exit_code}), catalog status={db_status}", job.job_id);
            }
        }

        // JcrCleanup(): num_jobs_run and RecentJobResultsList (C-048).
        let jobs_run = self.jobs_run;
        for job in self.jobs.iter_mut().filter(|j| j.completed_at.is_some() && !j.jcr.recorded) {
            job.jcr.recorded = true;
            job.jcr.fd_connected = false;
            job.jcr.sd_connected = false;
            self.jobs_run += 1;
            self.recent.push_back(job.job_id);
            if self.recent.len() > MAX_RECENT_JOB_RESULTS { self.recent.pop_front(); }
        }
        // Jobs that ended free their resources for the queue.
        let queued = !(self.waiting.is_empty() && self.ready.is_empty() && self.delayed.is_empty());
        if queued && (self.jobs_run != jobs_run || self.last_schedule.elapsed() >= std::time::Duration::from_secs(2)) {
            self.schedule();
        } else if !self.ready.is_empty() {
            // Ready jobs whose jobq_server pause has passed.
            self.start_ready();
        }

        // Cleanup completed jobs older than 5 minutes
        self.jobs.retain(|job| !job.completed_at.is_some_and(|time| time.elapsed() >= std::time::Duration::from_secs(300)));
    }
}

impl DirJob {
    /// jcr->JobStatus; a paused worker (Onesimus `pause`) shows as blocked.
    fn status(&self) -> char {
        if self.phase == DirJobPhase::Paused && status_priority(self.observed_status) == 0 { 'B' } else { self.observed_status }
    }

    /// The worker's protocol state in JCR terms (backup.cc/restore.cc):
    /// JS_WaitFD while it connects to the FD, JS_WaitSD while it connects to
    /// and sets up the SD, JS_Running afterwards; job_started before the
    /// FD's backup/restore command; the daemon sockets it holds.
    /// Compatibility log C-048.
    fn worker_state(&mut self, state: &str) {
        let (fd, sd, status, started) = match state {
            "ConnectingFD" => (false, false, Some('F'), false),
            "ConnectingSD" => (true, false, Some('S'), false),
            "SDJobSetup" | "SDUseDevice" | "SDRun" | "SDBootstrap" => (true, true, None, false),
            "FDJobSetup" | "FDStorage" | "FDSession" => (true, true, Some('R'), false),
            "FDBackup" | "FDRestore" | "Running" => (true, true, Some('R'), true),
            "Paused" | "WaitingMedia" | "WaitMedia" | "WaitingMount" | "WaitMount" => (true, true, None, true),
            "Canceling" => (self.jcr.fd_connected, self.jcr.sd_connected, None, self.jcr.job_started),
            // Finishing/Completed/Error: the worker closed its connections.
            _ => (false, false, None, self.jcr.job_started),
        };
        self.jcr.fd_connected = fd;
        self.jcr.sd_connected = sd;
        self.jcr.job_started = started;
        if let Some(status) = status {
            self.observed_status = merge_status(self.observed_status, status);
        }
    }
}

/// The catreq.rs state of a job, created with its context on first use.
fn catreq_session<'a>(sessions: &'a mut std::collections::HashMap<u32, crate::catreq::CatReqJob>,
                      catalog: &Option<CatalogHandle>, config: &Option<SharedConfig>, job: &DirJob)
                      -> &'a mut crate::catreq::CatReqJob {
    sessions.entry(job.job_id).or_insert_with(|| {
        let context = match catalog {
            Some(cat) => crate::catreq::load_context(cat, config.as_ref(), job.job_id, job.start_time,
                job.resources.as_ref().map(|(_, storage)| storage.as_str())),
            None => crate::catreq::JobContext { job_id: job.job_id, start_time: job.start_time, catalog_files: true,
                ..Default::default() },
        };
        crate::catreq::CatReqJob::new(context)
    })
}

/// A private catalog connection for the batch file insert (jcr->db_batch).
fn batch_opener(catalog: &Option<CatalogHandle>) -> impl Fn() -> Option<Box<dyn Db + Send>> + '_ {
    move || catalog.as_ref()?.dedicated().ok().map(|c| Box::new(c) as Box<dyn Db + Send>)
}

/// Jmsg() of the catalog requests to the job's messages.
fn post_catreq_messages(bus: &MessageBus, config: Option<&SharedConfig>, job: &DirJob, messages: crate::catreq::Messages) {
    for (msg_type, text) in messages {
        crate::console_msg::post_job_message(bus, config, BusMessage::JobMessage {
            job_id: job.job_id, job_name: job.job_name.clone(), msg_type, text, timestamp: now_unix() });
    }
}


/// Find the most recent successful backup job for a client.
pub fn find_latest_backup_jobid(catalog: &Option<CatalogHandle>, client_name: &str) -> Option<u32> {
    catalog.as_ref()?.find_latest_backup_jobid(client_name)
}

/// Job monitor thread — polls all worker pipes periodically.
pub fn monitor_loop(
    manager: Arc<Mutex<JobManager>>,
    shutdown: &AtomicBool,
) {
    debug!("job monitor started");
    while !shutdown.load(Ordering::Relaxed) {
        let had_work;
        {
            let mut mgr = manager.lock().unwrap();
            // Queued jobs need no fast polling (jobq looks every 2 seconds).
            let before = mgr.running().count();
            mgr.poll_all();
            had_work = before > 0;
        }
        // Adaptive sleep: 0.1ms when jobs running (low latency CatReq),
        // 50ms when idle (save CPU)
        if had_work {
            thread::sleep(std::time::Duration::from_micros(100));
        } else {
            thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    debug!("job monitor stopped");
}


fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs() as i64
}

#[cfg(test)]
mod queue_tests {
    //! jobq.cc queue, JCR states and RecentJobResultsList with stand-in
    //! worker processes (`sleep`). Compatibility log C-048.
    use super::*;
    use crate::job_worker::{JobParams, WorkerJobType};

    const CONFIG: &str = r#"
Director { Name = dir; Password = x; Maximum Concurrent Jobs = 10 }
Client { Name = fd1; Address = fd.example; Password = x; Maximum Concurrent Jobs = 10 }
Client { Name = fd2; Address = fd2.example; Password = x; Maximum Concurrent Jobs = 1 }
Storage { Name = File; Address = sd.example; Password = y; Device = D; Media Type = File; Maximum Concurrent Jobs = 10 }
Storage { Name = Tape; Address = sd.example; Password = y; Device = T; Media Type = LTO; Maximum Concurrent Jobs = 1 }
Job { Name = a; Client = fd1; Storage = File; Maximum Concurrent Jobs = 10 }
Job { Name = b; Client = fd1; Storage = File; Maximum Concurrent Jobs = 10; Allow Mixed Priority = yes }
Job { Name = single; Client = fd1; Storage = File }
"#;

    fn sleeper(id: u32, name: &str, params: &JobParams) -> std::io::Result<JobHandle> {
        let child = std::process::Command::new("sleep").arg("30").spawn()?;
        Ok(JobHandle::detached(child.id() as i32, id, name, &params.unique_job))
    }

    fn failing(_: u32, _: &str, _: &JobParams) -> std::io::Result<JobHandle> {
        Err(std::io::Error::other("no fork in this test"))
    }

    fn manager(config: &str) -> JobManager {
        let config = Arc::new(std::sync::RwLock::new(crate::director_config::DirectorConfig::from_str(config).unwrap()));
        let mut manager = JobManager::new(Arc::new(MessageBus::new(10)), Some(config), None);
        manager.set_spawner(sleeper);
        manager
    }

    fn params(id: u32, name: &str) -> JobParams {
        JobParams { job_id: id, unique_job: format!("{name}.2026-10-16_10.00.00_{id:02}"), job_name: name.into(),
            job_type: WorkerJobType::Backup, level: "Full".into(), fd_address: String::new(), fd_port: 0,
            fd_password: String::new(), sd_address: String::new(), sd_port: 0, sd_password: String::new(),
            sd_device: String::new(), sd_media_type: String::new(), director_name: "dir".into(), pool_name: "Full".into(),
            fileset_includes: Vec::new(), fileset_excludes: Vec::new(), since_utime: 0, prev_job: String::new(),
            where_path: None, replace: None, restore_jobid: None, bootstrap: None,
            client_name: String::new(), client_first_connection: false, copy: None }
    }

    fn enqueue(m: &mut JobManager, id: u32, name: &str, priority: i32, client: &str, storage: &str) {
        m.enqueue_job(id, name, QueuedStart { params: params(id, name), priority, sched_time: now_unix() }, client, storage);
    }

    fn state(m: &JobManager, id: u32) -> (char, bool) {
        let job = m.active_jobs().into_iter().find(|j| j.id == id).unwrap();
        (job.status, m.get_job(id).unwrap().handle.spawned())
    }

    /// End a stand-in worker the test started and wait until the manager saw it.
    fn finish(m: &mut JobManager, id: u32) {
        let pid = m.get_job(id).unwrap().handle.pid;
        assert!(pid > 0);
        unsafe { libc::kill(pid, libc::SIGKILL); }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while m.get_job(id).is_some_and(|j| !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error)) {
            m.poll_all();
            assert!(std::time::Instant::now() < deadline, "job {id} did not end");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn kill_all(m: &mut JobManager) {
        let ids: Vec<u32> = m.jobs.iter().filter(|j| j.handle.spawned() && j.completed_at.is_none()).map(|j| j.job_id).collect();
        for id in ids { finish(m, id); }
    }

    #[test]
    fn director_maximum_concurrent_jobs_keeps_jobs_created() {
        let mut m = manager(&CONFIG.replacen("Maximum Concurrent Jobs = 10", "Maximum Concurrent Jobs = 1", 1));
        enqueue(&mut m, 1, "a", 10, "fd1", "File");
        enqueue(&mut m, 2, "a", 10, "fd1", "File");
        assert_eq!((state(&m, 1), state(&m, 2)), (('R', true), ('C', false)));
        assert_eq!(m.running_jobs().len(), 2);
        finish(&mut m, 1);
        assert_eq!(state(&m, 2), ('R', true));
        assert_eq!((m.finished_count(), m.finished_job_ids()), (1, vec![1]));
        kill_all(&mut m);
        assert_eq!(m.finished_job_ids(), vec![1, 2]);
    }

    #[test]
    fn resource_waits() {
        let mut m = manager(CONFIG);
        enqueue(&mut m, 1, "single", 10, "fd1", "File");
        enqueue(&mut m, 2, "single", 10, "fd1", "File");
        enqueue(&mut m, 3, "a", 10, "fd2", "File");
        enqueue(&mut m, 4, "a", 10, "fd2", "File");
        enqueue(&mut m, 5, "a", 10, "fd1", "Tape");
        enqueue(&mut m, 6, "a", 10, "fd1", "Tape");
        let states: Vec<_> = (1..=6).map(|id| state(&m, id)).collect();
        assert_eq!(states, vec![('R', true), ('j', false), ('R', true), ('c', false), ('R', true), ('s', false)]);
        // A job that ends frees its resources: the next one starts.
        finish(&mut m, 1);
        assert_eq!(state(&m, 2), ('R', true));
        kill_all(&mut m);
    }

    #[test]
    fn priority_waits_and_mixed_priority() {
        let mut m = manager(CONFIG);
        enqueue(&mut m, 1, "a", 10, "fd1", "File");
        enqueue(&mut m, 2, "a", 5, "fd1", "File");
        enqueue(&mut m, 3, "a", 10, "fd1", "File");
        // The waiting queue is sorted by priority: job 2 blocks job 3.
        assert_eq!((state(&m, 2), state(&m, 3)), (('p', false), ('C', false)));
        finish(&mut m, 1);
        assert_eq!((state(&m, 2), state(&m, 3)), (('R', true), ('p', false)));
        finish(&mut m, 2);
        assert_eq!(state(&m, 3), ('R', true));
        kill_all(&mut m);

        // Allow Mixed Priority: a more important job joins running ones that allow it.
        let mut m = manager(CONFIG);
        enqueue(&mut m, 1, "b", 10, "fd1", "File");
        enqueue(&mut m, 2, "b", 5, "fd1", "File");
        enqueue(&mut m, 3, "a", 1, "fd1", "File");
        assert_eq!((state(&m, 2), state(&m, 3)), (('R', true), ('p', false)));
        kill_all(&mut m);
    }

    #[test]
    fn start_time_wait() {
        let mut m = manager(CONFIG);
        m.enqueue_job(1, "a", QueuedStart { params: params(1, "a"), priority: 10, sched_time: now_unix() + 3600 }, "fd1", "File");
        let job = m.active_jobs().remove(0);
        assert_eq!((job.status, job.sched_time > now_unix(), job.job_started), ('t', true, false));
        assert!(m.cancel_job(1));
        m.poll_all();
        assert!(m.active_jobs().is_empty());
        assert_eq!(m.get_job(1).unwrap().observed_status, 'A');
        assert!(!m.get_job(1).unwrap().handle.spawned());
    }

    #[test]
    fn canceled_queued_job_ends_without_worker() {
        let mut m = manager(CONFIG);
        enqueue(&mut m, 1, "single", 10, "fd1", "File");
        enqueue(&mut m, 2, "single", 10, "fd1", "File");
        assert_eq!(state(&m, 2), ('j', false));
        assert!(m.cancel_job(2));
        assert_eq!(m.get_job(2).unwrap().phase, DirJobPhase::Canceling);
        m.poll_all();
        let job = m.get_job(2).unwrap();
        assert_eq!((job.phase, job.observed_status, job.handle.spawned()), (DirJobPhase::Error, 'A', false));
        assert!(!m.cancel_job(2));
        assert_eq!(m.finished_job_ids(), vec![2]);
        // Its post scripts run once.
        assert!(m.take_post_scripts(2).is_some_and(|j| j.status == 'A' && j.unique_job == "single.2026-10-16_10.00.00_02"));
        assert!(m.take_post_scripts(2).is_none());
        kill_all(&mut m);
    }

    #[test]
    fn fork_failure_ends_the_job_in_error() {
        let mut m = manager(CONFIG);
        m.set_spawner(failing);
        enqueue(&mut m, 1, "a", 10, "fd1", "File");
        m.poll_all();
        assert_eq!(m.get_job(1).unwrap().phase, DirJobPhase::Error);
        assert_eq!(m.get_job(1).unwrap().observed_status, 'E');
    }

    #[test]
    fn worker_states_set_wait_codes_and_connections() {
        let mut job_manager = manager(CONFIG);
        enqueue(&mut job_manager, 1, "a", 10, "fd1", "File");
        let job = job_manager.jobs.iter_mut().find(|j| j.job_id == 1).unwrap();
        let mut seen = Vec::new();
        for state in ["ConnectingFD", "ConnectingSD", "SDJobSetup", "FDJobSetup", "FDBackup", "Running", "Error"] {
            job.worker_state(state);
            seen.push((job.observed_status, job.jcr.fd_connected, job.jcr.sd_connected, job.jcr.job_started));
        }
        assert_eq!(seen, vec![('F', false, false, false), ('S', true, false, false), ('S', true, true, false),
            ('R', true, true, false), ('R', true, true, true), ('R', true, true, true), ('R', false, false, true)]);
        // A canceled job keeps JS_Canceled.
        job.observed_status = 'A';
        job.worker_state("ConnectingFD");
        assert_eq!(job.observed_status, 'A');
        kill_all(&mut job_manager);
    }
}
