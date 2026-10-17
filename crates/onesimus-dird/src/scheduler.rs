//! Job scheduler — dir:scheduler thread (dird/scheduler.cc,
//! scheduler_private.cc, scheduler_job_item_queue.cc).
//!
//! Scheduler::Run(): FillSchedulerJobQueueOrSleep() adds the runs of this
//! and the next hour once the queue is empty, WaitForJobsToRun() takes the
//! due items out. A run is never queued twice: AddJobToQueue() skips a run
//! whose runtime lies within 61 seconds of the time it was last taken out
//! (RunResource::scheduled_last) and runs older than a minute. Duplicate
//! jobs (Allow Duplicate Jobs) are checked when the job starts, not here.

use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering as CmpOrdering;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::{info, debug};

use crate::director_config::{DirectorConfig, SharedConfig};

/// scheduler_private.cc seconds_per_hour.
const SECONDS_PER_HOUR: i64 = 3600;
/// DefaultSchedulerTimeAdapter default_wait_interval_ (seconds_per_minute).
const DEFAULT_WAIT_INTERVAL: i64 = 60;
/// SchedulerJobItemQueue::default_priority (items without run resource).
#[cfg(test)]
const DEFAULT_PRIORITY: i32 = 10;

/// A scheduled job item in the priority queue (SchedulerJobItem).
#[derive(Debug, Clone)]
pub struct ScheduledJob {
    pub job_name: String,
    /// The run resource: (schedule name, index of the Run directive).
    pub run: Option<RunKey>,
    pub level: String,
    pub runtime: i64,       // Unix timestamp
    pub priority: i32,      // Lower = higher priority
}

/// Identifies a Run directive (C++ RunResource*) across scans.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RunKey {
    pub job: String,
    pub schedule: String,
    pub index: usize,
}

// BinaryHeap is max-heap; we want earliest runtime first (PrioritiseJobItems).
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

/// Scheduler state (visible to the watchdog).
pub struct SchedulerState {
    /// Unix time the scheduler thread last showed life (0: not running).
    pub last_heartbeat: AtomicI64,
    pub jobs_dispatched: AtomicI64,
}

impl SchedulerState {
    pub const fn new() -> Self {
        Self { last_heartbeat: AtomicI64::new(0), jobs_dispatched: AtomicI64::new(0) }
    }

    pub fn heartbeat(&self) {
        self.last_heartbeat.store(now_unix(), Ordering::Relaxed);
    }
}

impl Default for SchedulerState {
    fn default() -> Self {
        Self::new()
    }
}

/// The state of the dir:scheduler thread.
pub static STATE: SchedulerState = SchedulerState::new();

/// SchedulerTimeAdapter's time source: SystemTime() and SleepFor().
pub trait TimeSource {
    fn system_time(&self) -> i64;
    /// Sleeps `seconds`; false when the scheduler is to stop.
    fn sleep_for(&self, seconds: i64) -> bool;
}

/// SystemTimeSource: wall clock, sleep interrupted by shutdown.
struct SystemTimeSource<'a> {
    shutdown: &'a AtomicBool,
}

impl TimeSource for SystemTimeSource<'_> {
    fn system_time(&self) -> i64 {
        now_unix()
    }

    fn sleep_for(&self, seconds: i64) -> bool {
        sleep_interruptible(Duration::from_secs(seconds.max(0) as u64), self.shutdown);
        STATE.heartbeat();
        !self.shutdown.load(Ordering::Relaxed)
    }
}

/// SchedulerPrivate: the queue and the scheduled_last of the run resources.
pub struct Scheduler<T: TimeSource> {
    time: T,
    queue: BinaryHeap<ScheduledJob>,
    /// RunResource::scheduled_last (0 until the run was taken out once).
    scheduled_last: HashMap<RunKey, i64>,
}

impl<T: TimeSource> Scheduler<T> {
    pub fn new(time: T) -> Self {
        Self { time, queue: BinaryHeap::new(), scheduled_last: HashMap::new() }
    }

    /// One cycle of Scheduler::Run(); false when the scheduler stops.
    pub fn cycle(&mut self, config: &SharedConfig, execute: &mut dyn FnMut(ScheduledJob)) -> bool {
        self.fill_queue_or_sleep(config) && self.wait_for_jobs_to_run(config, execute)
    }

    /// FillSchedulerJobQueueOrSleep(): only while the queue is empty.
    fn fill_queue_or_sleep(&mut self, config: &SharedConfig) -> bool {
        while self.queue.is_empty() {
            if let Ok(cfg) = config.read() {
                self.add_jobs_for_this_and_next_hour(&cfg);
            }
            if self.queue.is_empty() && !self.time.sleep_for(DEFAULT_WAIT_INTERVAL) {
                return false;
            }
        }
        true
    }

    /// WaitForJobsToRun(): due items are taken out and run (their run's
    /// scheduled_last becomes now), else sleep until the next one (at most
    /// the default wait interval).
    fn wait_for_jobs_to_run(&mut self, config: &SharedConfig, execute: &mut dyn FnMut(ScheduledJob)) -> bool {
        while let Some(next) = self.queue.peek() {
            let now = self.time.system_time();
            if now >= next.runtime {
                let job = self.queue.pop().expect("peeked");
                let enabled = config.read().ok()
                    .and_then(|cfg| cfg.find_job(&job.job_name).map(|j| j.enabled)).unwrap_or(true);
                if !enabled {
                    continue; // check queue again
                }
                if let Some(run) = &job.run {
                    self.scheduled_last.insert(run.clone(), now);
                }
                debug!("Scheduler: Running job {}.", job.job_name);
                execute(job);
            } else {
                let wait = DEFAULT_WAIT_INTERVAL.min(next.runtime - now);
                debug!("Scheduler: WaitForJobsToRun is sleeping for {wait} seconds. Next job: {}.", next.job_name);
                if !self.time.sleep_for(wait) {
                    return false;
                }
            }
        }
        true
    }

    /// AddJobsForThisAndNextHourToQueue()
    fn add_jobs_for_this_and_next_hour(&mut self, cfg: &DirectorConfig) {
        let now = self.time.system_time();
        let next_hour = now + SECONDS_PER_HOUR;
        for job in &cfg.jobs {
            let Some(schedule) = automatic_scheduler_schedule(cfg, job) else { continue };
            for (index, run) in schedule.runs.iter().enumerate() {
                let run_this_hour = crate::run_time::triggers_on_day_and_hour(&run.mask, now as libc::time_t);
                let run_next_hour = crate::run_time::triggers_on_day_and_hour(&run.mask, next_hour as libc::time_t);
                if !run_this_hour && !run_next_hour {
                    continue;
                }
                let runtime = crate::run_time::run_time_in_hour(run, now as libc::time_t) as i64;
                let item = |runtime| ScheduledJob {
                    job_name: job.name.clone(),
                    run: Some(RunKey { job: job.name.clone(), schedule: schedule.name.clone(), index }),
                    level: format!("{:?}", run.level.unwrap_or(job.level)),
                    runtime,
                    priority: run.priority.filter(|p| *p != 0).unwrap_or(job.priority),
                };
                if run_this_hour {
                    self.add_job_to_queue(now, item(runtime));
                }
                if run_next_hour {
                    self.add_job_to_queue(now, item(runtime + SECONDS_PER_HOUR));
                }
            }
        }
    }

    /// AddJobToQueue(): a run taken out within the last 61 seconds of its
    /// runtime, or a runtime more than 59 seconds past, is not queued.
    fn add_job_to_queue(&mut self, now: i64, item: ScheduledJob) {
        if let Some(run) = &item.run {
            let last = self.scheduled_last.get(run).copied().unwrap_or(0);
            if item.runtime - last < 61 {
                return;
            }
        }
        if item.runtime + 59 < now {
            return;
        }
        debug!("Scheduler: Put job {} into queue.", item.job_name);
        self.queue.push(item);
    }
}

/// IsAutomaticSchedulerJob(): the job's enabled schedule, when the job and
/// its client are enabled.
fn automatic_scheduler_schedule<'a>(cfg: &'a DirectorConfig, job: &bareos_config::resources::JobResource)
    -> Option<&'a bareos_config::resources::ScheduleResource> {
    if job.schedule.is_empty() || !job.enabled {
        return None;
    }
    let schedule = cfg.schedules.iter().find(|s| s.name == job.schedule).filter(|s| s.enabled)?;
    if cfg.find_client(&job.client).is_some_and(|c| !c.enabled) {
        return None;
    }
    Some(schedule)
}

/// The Run directive of a queued item in the current configuration.
pub fn run_directive(cfg: &DirectorConfig, run: &RunKey) -> Option<bareos_config::resources::RunDirective> {
    cfg.schedules.iter().find(|s| s.name == run.schedule).and_then(|s| s.runs.get(run.index)).cloned()
}

/// Main scheduler loop (Scheduler::Run). Runs in the dir:scheduler named
/// thread; ExecuteJob() starts each due job like `run` does
/// (scheduled_run.rs).
pub fn scheduler_loop(launcher: Option<crate::scheduled_run::Launcher>, shutdown: &AtomicBool) {
    let state = &STATE;
    state.heartbeat();
    info!("scheduler started");
    if let Some(launcher) = launcher {
        let config = launcher.config.clone();
        let mut scheduler = Scheduler::new(SystemTimeSource { shutdown });
        let mut execute = |job: ScheduledJob| {
            debug!("dispatching job '{}' (level={}, priority={})", job.job_name, job.level, job.priority);
            let run = job.run.as_ref().and_then(|run| config.read().ok().and_then(|cfg| run_directive(&cfg, run)));
            if launcher.execute(&job.job_name, run.as_ref(), now_unix()) != 0 {
                state.jobs_dispatched.fetch_add(1, Ordering::Relaxed);
            }
            state.heartbeat();
        };
        while !shutdown.load(Ordering::Relaxed) {
            state.heartbeat();
            if !scheduler.cycle(&config, &mut execute) {
                break;
            }
        }
    }
    state.last_heartbeat.store(0, Ordering::Relaxed);
    info!("scheduler stopped, dispatched {} jobs total",
          state.jobs_dispatched.load(Ordering::Relaxed));
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
    use std::cell::Cell;
    use std::sync::{Arc, RwLock};

    /// Fake clock (scheduler tests' SimulatedTimeSource): sleeping advances
    /// the time; it stops after `limit`.
    struct FakeClock<'a> {
        now: &'a Cell<i64>,
        limit: i64,
    }

    impl TimeSource for FakeClock<'_> {
        fn system_time(&self) -> i64 {
            self.now.get()
        }
        fn sleep_for(&self, seconds: i64) -> bool {
            self.now.set(self.now.get() + seconds.max(1));
            self.now.get() < self.limit
        }
    }

    fn config(text: &str) -> SharedConfig {
        Arc::new(RwLock::new(crate::director_config::DirectorConfig::from_str(text).unwrap()))
    }

    /// Local time of today's `hour`:`minute`.
    /// Local time of `hour`:`minute` on Wednesday 2026-10-14 (inside `tz`).
    fn local(hour: i32, minute: i32) -> i64 {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        (tm.tm_year, tm.tm_mon, tm.tm_mday) = (126, 9, 14);
        (tm.tm_hour, tm.tm_min, tm.tm_sec, tm.tm_isdst) = (hour, minute, 0, -1);
        unsafe { libc::mktime(&mut tm) as i64 }
    }

    /// Runs `f` in a fixed time zone (holding the TZ lock of the TZ tests).
    fn tz<R>(f: impl FnOnce() -> R) -> R {
        crate::run_time::tests::with_tz("Europe/Berlin", f)
    }

    const BASE: &str = r#"
Director { Name=dir; Password=secret }
Catalog { Name=MyCatalog; DbName=bareos }
Client { Name=fd1; Password=x }
Client { Name=off; Password=x; Enabled=no }
Storage { Name=File; Address=localhost; Password=x; Device=FileStorage; Media Type=File }
Pool { Name=Full; Pool Type=Backup }
FileSet { Name=fs; Include { File=/etc } }
Messages { Name=Standard }
"#;

    /// Runs the scheduler from `start` until `limit`: the jobs started
    /// as (name, time, level, priority).
    fn run_scheduler(cfg: &SharedConfig, start: i64, limit: i64) -> Vec<(String, i64, String, i32)> {
        let now = Cell::new(start);
        let mut scheduler = Scheduler::new(FakeClock { now: &now, limit });
        let mut started = Vec::new();
        let mut execute = |job: ScheduledJob| started.push((job.job_name, now.get(), job.level, job.priority));
        while scheduler.cycle(cfg, &mut execute) {}
        started
    }

    fn job(name: &str, schedule: &str, extra: &str) -> String {
        format!("Job {{ Name={name}; Type=Backup; Client=fd1; FileSet=fs; Storage=File; Pool=Full; \
            Messages=Standard; Schedule={schedule}; {extra} }}\n")
    }

    #[test]
    fn hourly_run_starts_once_per_hour() {
        tz(|| {
            let cfg = config(&format!("{BASE}Schedule {{ Name=hourly; Run = Full hourly at 0:10 }}\n{}",
                job("a", "hourly", "")));
            let start = local(9, 0);
            let started = run_scheduler(&cfg, start, start + 3 * 3600);
            let times: Vec<i64> = started.iter().map(|s| s.1).collect();
            assert_eq!(times, vec![local(9, 10), local(10, 10), local(11, 10)]);
        })
    }

    #[test]
    fn same_run_is_not_queued_twice() {
        tz(|| {
            // daily run; the scheduler refills the queue every minute after it
            // ran, without starting the job a second time.
            let cfg = config(&format!("{BASE}Schedule {{ Name=daily; Run = Full daily at 10:30 }}\n{}",
                job("a", "daily", "")));
            let start = local(10, 0);
            let started = run_scheduler(&cfg, start, start + 2 * 3600);
            assert_eq!(started, vec![("a".to_string(), local(10, 30), "Full".to_string(), 10)]);
        })
    }

    #[test]
    fn missed_run_older_than_a_minute_is_skipped() {
        tz(|| {
            let cfg = config(&format!("{BASE}Schedule {{ Name=daily; Run = Full daily at 10:30 }}\n{}",
                job("a", "daily", "")));
            assert!(run_scheduler(&cfg, local(10, 31), local(10, 59)).is_empty());
            // within the minute it still runs
            let started = run_scheduler(&cfg, local(10, 30) + 59, local(10, 59));
            assert_eq!(started.len(), 1);
        })
    }

    #[test]
    fn earlier_runtime_then_lower_priority_runs_first() {
        tz(|| {
            let cfg = config(&format!("{BASE}Schedule {{ Name=s; Run = Full daily at 10:30 }}\n\
                Schedule {{ Name=p; Run = Incremental Priority=5 daily at 10:30 }}\n\
                Schedule {{ Name=e; Run = Full daily at 10:20 }}\n{}{}{}",
                job("low", "s", "Priority=20"), job("high", "p", ""), job("early", "e", "Priority=50")));
            let started = run_scheduler(&cfg, local(10, 0), local(11, 0));
            let order: Vec<(&str, &str, i32)> = started.iter().map(|s| (s.0.as_str(), s.2.as_str(), s.3)).collect();
            assert_eq!(order, vec![("early", "Full", 50), ("high", "Incremental", 5), ("low", "Full", 20)]);
        })
    }

    #[test]
    fn disabled_job_schedule_or_client_is_not_scheduled() {
        tz(|| {
            let cfg = config(&format!("{BASE}Schedule {{ Name=s; Run = Full daily at 10:30 }}\n\
                Schedule {{ Name=off; Enabled=no; Run = Full daily at 10:30 }}\n{}{}{}",
                job("job_off", "s", "Enabled=no"), job("sched_off", "off", ""),
                job("client_off", "s", "").replace("Client=fd1", "Client=off")));
            assert!(run_scheduler(&cfg, local(10, 0), local(11, 0)).is_empty());
        })
    }

    #[test]
    fn job_disabled_while_queued_does_not_run() {
        tz(|| {
            let cfg = config(&format!("{BASE}Schedule {{ Name=s; Run = Full daily at 10:30 }}\n{}", job("a", "s", "")));
            let now = Cell::new(local(10, 0));
            let mut scheduler = Scheduler::new(FakeClock { now: &now, limit: local(10, 1) });
            let mut started = 0;
            scheduler.fill_queue_or_sleep(&cfg);
            assert_eq!(scheduler.queue.len(), 1);
            cfg.write().unwrap().jobs[0].enabled = false;
            now.set(local(10, 30));
            scheduler.wait_for_jobs_to_run(&cfg, &mut |_| started += 1);
            assert_eq!(started, 0);
            assert!(scheduler.queue.is_empty());
        })
    }

    #[test]
    fn scheduled_job_ordering_earliest_first() {
        let item = |name: &str, runtime| ScheduledJob {
            job_name: name.into(), run: None, level: "Full".into(), runtime, priority: DEFAULT_PRIORITY,
        };
        let mut heap = BinaryHeap::new();
        heap.push(item("late", 2000));
        heap.push(item("early", 1000));
        heap.push(item("mid", 1500));
        assert_eq!(heap.pop().unwrap().job_name, "early");
        assert_eq!(heap.pop().unwrap().job_name, "mid");
        assert_eq!(heap.pop().unwrap().job_name, "late");
    }

    #[test]
    fn scheduler_state_heartbeat() {
        let state = SchedulerState::new();
        assert_eq!(state.last_heartbeat.load(Ordering::Relaxed), 0);
        state.heartbeat();
        assert!(state.last_heartbeat.load(Ordering::Relaxed) > 0);
    }
}
