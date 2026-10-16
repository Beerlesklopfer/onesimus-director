//! Job Manager — Director-side tracking of all running job workers.
//!
//! Maintains a Vec of active jobs (up to 1000+ concurrent).
//! One monitor thread polls all worker pipes and handles CatReq.

use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use tracing::{info, debug, warn, error};

use crate::catalog_worker::CatalogHandle;
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
}

impl JobManager {
    pub fn new(bus: Arc<MessageBus>, config: Option<SharedConfig>, catalog: Option<CatalogHandle>) -> Self {
        Self { jobs: Vec::new(), bus, config, catalog }
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
                        debug!("JobManager: job {} CatReq: {}", job.job_id, &request[..request.len().min(60)]);
                        let previous_phase = job.phase;
                        job.phase = DirJobPhase::WaitingCatReq;
                        let response = handle_cat_req(&request, &self.catalog, job.job_id);
                        job.handle.send_message(&PipeMessage::CatResp { response });
                        job.phase = previous_phase;
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

/// CatReq handler — queries DB via CatalogHandle (MPSC channel).
fn handle_cat_req(request: &str, catalog: &Option<CatalogHandle>, job_id: u32) -> String {
    let Some(cat) = catalog else {
        return "1000 OK\n".to_string();
    };

    if request.contains("FindMedia") {
        let pool = extract_field(request, "pool_name=").unwrap_or("Full".into());
        debug!("CatReq: FindMedia pool={pool}");
        match cat.find_appendable_volume(&pool) {
            Some(name) => cat.get_volume_info(&name).unwrap_or("1901 No Media.\n".into()),
            None => "1901 No Media.\n".to_string(),
        }
    } else if request.contains("GetVolInfo") {
        let vol = extract_field(request, "VolName=").unwrap_or("Full-0001".into());
        debug!("CatReq: GetVolInfo vol={vol}");
        cat.get_volume_info(&vol).unwrap_or("1901 No Media.\n".into())
    } else if request.contains("UpdateMedia") {
        let vol = extract_field(request, "VolName=").unwrap_or("Full-0001".into());
        debug!("CatReq: UpdateMedia vol={vol}");
        let mounts: i64 = extract_field(request, "VolMounts=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let bytes: i64 = extract_field(request, "VolBytes=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let jobs: i32 = extract_field(request, "VolJobs=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let files: i64 = extract_field(request, "VolFiles=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let errors: i64 = extract_field(request, "VolErrors=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let writes: i64 = extract_field(request, "VolWrites=").and_then(|s| s.parse().ok()).unwrap_or(0);
        cat.update_media(&vol, mounts, bytes, jobs, files, errors, writes);
        cat.get_volume_info(&vol).unwrap_or("1901 No Media.\n".into())
    } else if request.contains("CreateJobMedia") {
        // CatReq format: "CatReq Job=%s CreateJobMedia FirstIndex=%u LastIndex=%u
        //   StartFile=%u EndFile=%u StartBlock=%u EndBlock=%u Copy=%d Strip=%d MediaId=%s"
        let first_idx: i32 = extract_field(request, "FirstIndex=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let last_idx: i32 = extract_field(request, "LastIndex=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let start_file: i32 = extract_field(request, "StartFile=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let end_file: i32 = extract_field(request, "EndFile=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let start_block: i64 = extract_field(request, "StartBlock=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let end_block: i64 = extract_field(request, "EndBlock=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let copy: i32 = extract_field(request, "Copy=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let strip: i32 = extract_field(request, "Strip=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let media_id: i32 = extract_field(request, "MediaId=").and_then(|s| s.parse().ok()).unwrap_or(0);
        // job_id comes from the running job context (not from CatReq string,
        // which only has Job=unique_name, not the numeric JobId)
        let db_job_id = job_id as i32;

        debug!("CatReq: CreateJobMedia job={db_job_id} media={media_id} fi={first_idx}-{last_idx} blk={start_block}-{end_block}");

        match cat.create_jobmedia(db_job_id, media_id, first_idx, last_idx,
            start_file, end_file, start_block, end_block, copy, strip, 0) {
            Ok(jm_id) => {
                info!("CreateJobMedia: JobMediaId={jm_id} job={job_id} media={media_id}");
                "1000 OK CreateJobMedia\n".to_string()
            }
            Err(e) => {
                error!("CreateJobMedia failed: {e}");
                "1000 OK CreateJobMedia\n".to_string() // SD expects OK regardless
            }
        }
    } else if request.contains("UpdateJobRecord") {
        // "Catreq Job=%s UpdateJobRecord JobFiles=%u JobBytes=%llu"
        let files: i64 = extract_field(request, "JobFiles=").and_then(|s| s.parse().ok()).unwrap_or(0);
        let bytes: i64 = extract_field(request, "JobBytes=").and_then(|s| s.parse().ok()).unwrap_or(0);
        debug!("CatReq: UpdateJobRecord job={job_id} files={files} bytes={bytes}");
        if let Err(e) = cat.raw_query(&format!(
            "UPDATE job SET jobfiles={files}, jobbytes={bytes} WHERE jobid={job_id}")) {
            warn!("UpdateJobRecord: {e}");
        }
        "1000 OK\n".to_string()
    } else if request.contains("UpdateFileList") || request.contains("Filelist") {
        // Fire-and-forget — SD signals new files written, no action needed
        debug!("CatReq: UpdateFileList job={job_id}");
        "1000 OK\n".to_string()
    } else if request.contains("DeleteNullJobmediaRecords") {
        // "CatReq Job=%s DeleteNullJobmediaRecords jobid=%u"
        debug!("CatReq: DeleteNullJobmediaRecords job={job_id}");
        if let Err(e) = cat.raw_query(&format!(
            "DELETE FROM jobmedia WHERE jobid={job_id} AND firstindex=0 AND lastindex=0 \
             AND startblock=0 AND endblock=0")) {
            warn!("DeleteNullJobmediaRecords: {e}");
        }
        "1000 OK DeleteNullJobmediaRecords\n".to_string()
    } else {
        debug!("CatReq: unknown: {}", &request[..request.len().min(80)]);
        "1000 OK\n".to_string()
    }
}


fn extract_field(request: &str, prefix: &str) -> Option<String> {
    request.split_whitespace()
        .find(|s| s.starts_with(prefix))
        .map(|s| s[prefix.len()..].to_string())
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
