//! Job scheduler — dir:scheduler thread.
//!
//! Scans schedule resources hourly, fills a priority queue,
//! dispatches jobs when their runtime arrives.

use std::collections::BinaryHeap;
use std::cmp::Ordering as CmpOrdering;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::{info, debug, warn};

use crate::director_config::SharedConfig;

/// A scheduled job item in the priority queue.
#[derive(Debug, Clone)]
pub struct ScheduledJob {
    pub job_name: String,
    pub level: String,
    pub pool: Option<String>,
    pub storage: Option<String>,
    pub runtime: i64,       // Unix timestamp
    pub priority: i32,      // Lower = higher priority
}

// BinaryHeap is max-heap; we want earliest runtime first.
impl Eq for ScheduledJob {}
impl PartialEq for ScheduledJob {
    fn eq(&self, other: &Self) -> bool {
        self.runtime == other.runtime && self.priority == other.priority
    }
}

impl PartialOrd for ScheduledJob {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledJob {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Reverse: earliest runtime first, then lowest priority number
        other.runtime.cmp(&self.runtime)
            .then(other.priority.cmp(&self.priority))
    }
}

/// Scheduler state (visible to watchdog).
pub struct SchedulerState {
    pub last_heartbeat: AtomicI64,
    pub jobs_dispatched: AtomicI64,
}

impl SchedulerState {
    pub fn new() -> Self {
        Self {
            last_heartbeat: AtomicI64::new(now_unix()),
            jobs_dispatched: AtomicI64::new(0),
        }
    }

    pub fn heartbeat(&self) {
        self.last_heartbeat.store(now_unix(), Ordering::Relaxed);
    }
}

/// Main scheduler loop. Runs in the dir:scheduler named thread.
pub fn scheduler_loop(config: Option<SharedConfig>, shutdown: &AtomicBool) {
    let state = SchedulerState::new();
    let mut queue: BinaryHeap<ScheduledJob> = BinaryHeap::new();

    info!("scheduler started");

    while !shutdown.load(Ordering::Relaxed) {
        state.heartbeat();

        // Scan config for scheduled jobs
        if let Some(ref cfg) = config {
            fill_queue_from_config(cfg, &mut queue);
        }

        // Dispatch ready jobs
        dispatch_ready_jobs(&mut queue, &state, shutdown);

        // Sleep until next check (max 60 seconds)
        let sleep_duration = next_check_interval(&queue);
        sleep_interruptible(sleep_duration, shutdown);
    }

    info!("scheduler stopped, dispatched {} jobs total",
          state.jobs_dispatched.load(Ordering::Relaxed));
}

/// Scan config for jobs with schedules and add to queue.
fn fill_queue_from_config(config: &SharedConfig, queue: &mut BinaryHeap<ScheduledJob>) {
    let cfg = match config.read() {
        Ok(c) => c,
        Err(_) => return,
    };

    let now = now_unix();

    for job in &cfg.jobs {
        if !job.enabled || job.schedule.is_empty() {
            continue;
        }

        // Find matching schedule
        let schedule = cfg.schedules.iter().find(|s| s.name == job.schedule);
        let schedule = match schedule {
            Some(s) if s.enabled => s,
            _ => continue,
        };

        for run in &schedule.runs {
            // Local-time day/hour masks like C++ TriggersOnDayAndHour(); the
            // run is due within the minute of its run time.
            let runtime = crate::run_time::run_time_in_hour(run, now as libc::time_t) as i64;
            if crate::run_time::triggers_on_day_and_hour(&run.mask, now as libc::time_t)
                && runtime <= now && now - runtime < 60
            {
                let level = run.level
                    .map(|l| format!("{:?}", l))
                    .unwrap_or_else(|| format!("{:?}", job.level));

                // Avoid duplicate scheduling (check if already in queue)
                let already_queued = false; // TODO: proper dedup

                if !already_queued {
                    debug!("scheduling job '{}' level={} at {:02}:{:02}",
                           job.name, level, run.hour, run.minute);

                    queue.push(ScheduledJob {
                        job_name: job.name.clone(),
                        level,
                        pool: run.pool.clone(),
                        storage: run.storage.clone(),
                        runtime,
                        priority: run.priority.unwrap_or(job.priority),
                    });
                }
            }
        }
    }
}

/// Dispatch all jobs whose runtime has arrived.
fn dispatch_ready_jobs(
    queue: &mut BinaryHeap<ScheduledJob>,
    state: &SchedulerState,
    shutdown: &AtomicBool,
) {
    let now = now_unix();

    while let Some(job) = queue.peek() {
        if job.runtime > now || shutdown.load(Ordering::Relaxed) {
            break;
        }

        let job = queue.pop().unwrap();
        debug!("dispatching job '{}' (level={}, priority={})",
               job.job_name, job.level, job.priority);

        // TODO(R6-full): Fork worker process via job_worker::spawn_job_worker
        state.jobs_dispatched.fetch_add(1, Ordering::Relaxed);
    }
}

/// Calculate how long to sleep until the next job is due (max 60s).
fn next_check_interval(queue: &BinaryHeap<ScheduledJob>) -> Duration {
    let max_sleep = Duration::from_secs(60);

    if let Some(next_job) = queue.peek() {
        let now = now_unix();
        if next_job.runtime <= now {
            return Duration::ZERO;
        }
        let wait_secs = (next_job.runtime - now) as u64;
        Duration::from_secs(wait_secs).min(max_sleep)
    } else {
        max_sleep
    }
}

/// Sleep that can be interrupted by shutdown.
fn sleep_interruptible(duration: Duration, shutdown: &AtomicBool) {
    let interval = Duration::from_millis(500);
    let mut remaining = duration;

    while remaining > Duration::ZERO && !shutdown.load(Ordering::Relaxed) {
        let sleep_time = remaining.min(interval);
        std::thread::sleep(sleep_time);
        remaining = remaining.saturating_sub(sleep_time);
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduled_job_ordering_earliest_first() {
        let mut heap = BinaryHeap::new();
        heap.push(ScheduledJob {
            job_name: "late".into(), level: "Full".into(),
            pool: None, storage: None, runtime: 2000, priority: 10,
        });
        heap.push(ScheduledJob {
            job_name: "early".into(), level: "Full".into(),
            pool: None, storage: None, runtime: 1000, priority: 10,
        });
        heap.push(ScheduledJob {
            job_name: "mid".into(), level: "Full".into(),
            pool: None, storage: None, runtime: 1500, priority: 10,
        });

        assert_eq!(heap.pop().unwrap().job_name, "early");
        assert_eq!(heap.pop().unwrap().job_name, "mid");
        assert_eq!(heap.pop().unwrap().job_name, "late");
    }

    #[test]
    fn same_time_higher_priority_first() {
        let mut heap = BinaryHeap::new();
        heap.push(ScheduledJob {
            job_name: "low".into(), level: "Full".into(),
            pool: None, storage: None, runtime: 1000, priority: 20,
        });
        heap.push(ScheduledJob {
            job_name: "high".into(), level: "Full".into(),
            pool: None, storage: None, runtime: 1000, priority: 5,
        });

        assert_eq!(heap.pop().unwrap().job_name, "high");
        assert_eq!(heap.pop().unwrap().job_name, "low");
    }

    #[test]
    fn dispatch_ready_jobs_pops_due() {
        let state = SchedulerState::new();
        let shutdown = AtomicBool::new(false);
        let mut queue = BinaryHeap::new();

        let past = now_unix() - 100;
        let future = now_unix() + 3600;

        queue.push(ScheduledJob {
            job_name: "past-job".into(), level: "Full".into(),
            pool: None, storage: None, runtime: past, priority: 10,
        });
        queue.push(ScheduledJob {
            job_name: "future-job".into(), level: "Full".into(),
            pool: None, storage: None, runtime: future, priority: 10,
        });

        dispatch_ready_jobs(&mut queue, &state, &shutdown);

        // Past job dispatched, future job remains
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.peek().unwrap().job_name, "future-job");
        assert_eq!(state.jobs_dispatched.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn next_check_interval_empty_queue() {
        let queue = BinaryHeap::new();
        assert_eq!(next_check_interval(&queue), Duration::from_secs(60));
    }

    #[test]
    fn next_check_interval_due_now() {
        let mut queue = BinaryHeap::new();
        queue.push(ScheduledJob {
            job_name: "now".into(), level: "Full".into(),
            pool: None, storage: None, runtime: now_unix() - 10, priority: 10,
        });
        assert_eq!(next_check_interval(&queue), Duration::ZERO);
    }

    #[test]
    fn scheduler_state_heartbeat() {
        let state = SchedulerState::new();
        let t1 = state.last_heartbeat.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(10));
        state.heartbeat();
        let t2 = state.last_heartbeat.load(Ordering::Relaxed);
        assert!(t2 >= t1);
    }
}
