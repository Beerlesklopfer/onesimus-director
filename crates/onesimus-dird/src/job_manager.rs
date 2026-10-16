//! Job Manager — Director-side tracking of all running job workers.
//!
//! Maintains a Vec of active jobs (up to 1000+ concurrent).
//! One monitor thread polls all worker pipes and handles CatReq.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tracing::{info, debug, error};

use crate::catalog_worker::CatalogHandle;
use crate::commands::bvfs::Db;
use crate::job_worker::{JobHandle, PipeMessage};
use crate::message_bus::{MessageBus, BusMessage, MsgType};
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
}

struct JobEnd {
    term_code: i32, files: i64, bytes: i64, errors: i32,
    vol_session_id: i32, vol_session_time: i32,
}

/// Bareos lib/jcr.cc: retain the first equal-priority error; Incomplete wins.
fn merge_status(previous: char, next: char) -> char {
    fn priority(status: char) -> u8 { match status { 'I' => 10, 'E' | 'f' | 'A' => 9, 'e' => 8, 'D' => 7, _ => 0 } }
    if priority(next) > priority(previous) || (priority(next) == 0 && priority(previous) == 0) { next } else { previous }
}

/// Director-side job phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirJobPhase {
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

    /// Upstream JobStatus code of the phase (lib/jcr.h JS_*): a job whose
    /// worker has not reached its daemons yet is JS_Created; storage waits
    /// are JS_WaitMedia/JS_WaitMount, a paused job JS_Blocked.
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
    pub fn job_started(&self) -> bool {
        !matches!(self, Self::Queued | Self::Starting)
    }
}

/// Owned snapshot; callers release the manager lock before waiting or querying DB.
#[derive(Clone)]
pub struct ActiveJob {
    pub id: u32,
    pub name: String,
    pub unique_name: String,
    pub phase: DirJobPhase,
}

/// Manages all active job workers — Vec-based for up to 1000+ jobs.
pub struct JobManager {
    jobs: Vec<DirJob>,
    bus: Arc<MessageBus>,
    config: Option<SharedConfig>,
    catalog: Option<CatalogHandle>,
    /// Catalog state of the SD catalog requests per JobId (catreq.rs).
    catreq_jobs: std::collections::HashMap<u32, crate::catreq::CatReqJob>,
}

impl JobManager {
    pub fn new(bus: Arc<MessageBus>, config: Option<SharedConfig>, catalog: Option<CatalogHandle>) -> Self {
        Self { jobs: Vec::new(), bus, config, catalog, catreq_jobs: std::collections::HashMap::new() }
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
        });
        debug!("JobManager: added job {job_id} '{job_name}' (total: {})", self.jobs.len());
    }

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
            .map(|job| ActiveJob { id: job.job_id, name: job.job_name.clone(),
                unique_name: job.handle.unique_job.clone(), phase: job.phase }).collect()
    }

    /// Jobs that ran to an end since the Director started (still in the table).
    pub fn finished_count(&self) -> usize {
        self.jobs.iter()
            .filter(|j| matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error))
            .count()
    }

    /// JobIds of the jobs that ended since the Director started, in the
    /// order they were started (C++ RecentJobResultsList without state file).
    pub fn finished_job_ids(&self) -> Vec<u32> {
        self.jobs.iter()
            .filter(|j| matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error))
            .map(|j| j.job_id)
            .collect()
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

    #[cfg(test)]
    pub fn set_phase(&mut self, job_id: u32, phase: DirJobPhase) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.phase = phase;
        }
    }

    pub fn cancel_job(&mut self, job_id: u32) -> bool {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id
            && !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error)) {
            if !job.handle.cancel() { return false; }
            job.observed_status = merge_status(job.observed_status, 'A');
            job.phase = DirJobPhase::Canceling;
            true
        } else { false }
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
                            job.observed_status = merge_status(job.observed_status, status);
                            if job.observed_status == status {
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
                        job.pending_end = Some(JobEnd {term_code, files, bytes, errors,
                            vol_session_id: vol_session_id.unwrap_or(0), vol_session_time: vol_session_time.unwrap_or(0)});
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
                crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
                    job_id: job.job_id, job_name: job.job_name.clone(),
                    msg_type: if success { MsgType::Info } else { MsgType::Error },
                    text: format!("Job {status}: status={db_status} files={} bytes={} errors={}", ending.files, ending.bytes, ending.errors),
                    timestamp: now_unix() });
                self.bus.post(BusMessage::JobStatus {job_id: job.job_id, job_name: job.job_name.clone(), status: status.into(), timestamp: now_unix()});
                info!("JobManager: job {} worker exited (code={exit_code}), catalog status={db_status}", job.job_id);
            }
        }

        // Cleanup completed jobs older than 5 minutes
        self.jobs.retain(|job| !job.completed_at.is_some_and(|time| time.elapsed() >= std::time::Duration::from_secs(300)));
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

/// Generate bootstrap data for a restore from a given backup JobId.
pub fn generate_bootstrap(catalog: &Option<CatalogHandle>, job_id: u32) -> Option<String> {
    catalog.as_ref()?.generate_bootstrap(job_id)
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
            let before = mgr.active_count();
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
