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
}

/// Director-side job phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirJobPhase {
    Queued,
    Starting,
    Running,
    Paused,
    WaitingCatReq,
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
            Self::WaitingCatReq => "WaitingCatReq",
            Self::Finishing => "Finishing",
            Self::Completed => "Completed",
            Self::Error => "Error",
        }
    }
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

    /// Jobs that ran to an end since the Director started (still in the table).
    pub fn finished_count(&self) -> usize {
        self.jobs.iter()
            .filter(|j| matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error))
            .count()
    }

    pub fn cancel_job(&mut self, job_id: u32) -> bool {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.handle.cancel();
            job.phase = DirJobPhase::Error;
            true
        } else {
            false
        }
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
                continue;
            }

            // Check if worker exited
            if let Some(exit_code) = job.handle.try_wait() {
                if exit_code == 0 {
                    job.phase = DirJobPhase::Completed;
                } else {
                    job.phase = DirJobPhase::Error;
                }
                info!("JobManager: job {} '{}' worker exited (code={})",
                    job.job_id, job.job_name, exit_code);
                continue;
            }

            // Read pipe messages (non-blocking — try_read_message returns None if no data)
            while let Some(msg) = job.handle.try_read_message() {
                match msg {
                    PipeMessage::State { state, .. } => {
                        debug!("JobManager: job {} state={state}", job.job_id);
                        job.phase = match state.as_str() {
                            "Running" => DirJobPhase::Running,
                            "Paused" => DirJobPhase::Paused,
                            "Finishing" => DirJobPhase::Finishing,
                            "Completed" => DirJobPhase::Completed,
                            "Error" => DirJobPhase::Error,
                            _ => job.phase,
                        };
                    }

                    PipeMessage::Status { files, bytes, errors, .. } => {
                        job.last_files = files;
                        job.last_bytes = bytes;
                    }

                    PipeMessage::CatReq { request, .. } => {
                        debug!("JobManager: job {} CatReq: {}", job.job_id, &request[..request.len().min(60)]);
                        job.phase = DirJobPhase::WaitingCatReq;
                        let response = handle_cat_req(&request, &self.catalog, job.job_id);
                        job.handle.send_message(&PipeMessage::CatResp { response });
                        job.phase = DirJobPhase::Running;
                    }

                    PipeMessage::JobEnd { term_code, files, bytes, errors,
                                          vol_session_id, vol_session_time, .. } => {
                        let status = if term_code == 0 { "Success" } else { "Error" };
                        let db_status = if term_code == 0 { "T" } else { "E" };
                        info!("JobManager: job {} '{}' ended — {status} files={files} bytes={bytes} errors={errors}",
                            job.job_id, job.job_name);
                        job.last_files = files;
                        job.last_bytes = bytes;
                        job.phase = if term_code == 0 { DirJobPhase::Completed } else { DirJobPhase::Error };

                        // Update job record in DB
                        if let Some(cat) = &self.catalog {
                            if let Err(e) = cat.update_job_end(
                                job.job_id as i32, db_status, now_unix(),
                                files, bytes, errors as i32,
                                vol_session_id.unwrap_or(0), vol_session_time.unwrap_or(0),
                            ) {
                                error!("JobManager: DB update_job_end failed for job {}: {e}", job.job_id);
                            }
                        }

                        // Post to message bus
                        let msg_type = if term_code == 0 { MsgType::Info } else { MsgType::Error };
                        self.bus.post(BusMessage::JobMessage {
                            job_id: job.job_id,
                            job_name: job.job_name.clone(),
                            msg_type,
                            text: format!("Backup {status}: files={files} bytes={bytes} errors={errors}"),
                            timestamp: now_unix(),
                        });
                        self.bus.post(BusMessage::JobStatus {
                            job_id: job.job_id,
                            job_name: job.job_name.clone(),
                            status: status.to_string(),
                            timestamp: now_unix(),
                        });
                    }

                    _ => {}
                }
            }
        }

        // Cleanup completed jobs older than 5 minutes
        let cutoff = now_unix() - 300;
        self.jobs.retain(|j| {
            if matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error) && j.start_time < cutoff {
                debug!("JobManager: removing finished job {}", j.job_id);
                false
            } else {
                true
            }
        });
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
