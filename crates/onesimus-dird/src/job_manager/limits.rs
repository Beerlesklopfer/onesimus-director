//! Job time limits and duplicate checks of the job queue (S-22).
//!
//! - AllowDuplicateJob() (dird/job.cc:883), done in SetupJob() through the
//!   job type's Init before JobqAdd(): it never refuses the job; a refused
//!   duplicate is the new job set JS_Canceled with "JobId N already
//!   running. Duplicate job not allowed.", queued in front of the ready
//!   queue and ended by job_thread() (report "… Canceled"). With Cancel
//!   Queued/Running/Lower Level Duplicates the other job is canceled
//!   ("Cancelling duplicate JobId=N." and CancelJob()'s "JobId N, Job …
//!   marked to be canceled.") and the new one runs.
//! - job_thread() (job.cc:433): Max Start Delay against jcr->sched_time and
//!   the Run directive's MaxRunSchedTime cancel the job when it leaves the
//!   queue ("Job canceled because max start delay time exceeded." /
//!   "… max run sched time exceeded."). The Job resource's MaxRunSchedTime
//!   is not copied into the JCR by SetJcrDefaults(): only a Run sets it.
//! - JobMonitorWatchdog (job.cc:717), every 60 seconds from the Director's
//!   start: Max Wait Time (time in wait states after the job started), Max
//!   Run Time / Full/Incremental/Differential Max Runtime, Max Run Sched
//!   Time; a hit is "Fatal error: Max … time exceeded. Job canceled." and
//!   CancelJob().

use super::*;

/// The run request's part of the JCR the queue checks need (ua_run.cc
/// `ignoreduplicatecheck=`, SetJcrFromRunResource MaxRunSchedTime).
#[derive(Debug, Clone, Copy, Default)]
pub struct StartFlags {
    pub ignore_duplicate_check: bool,
    /// Seconds; 0 = none.
    pub max_run_sched_time: i64,
}

/// The directives of the Job resource (with JobDefs) the checks use.
#[derive(Debug, Clone)]
pub(super) struct JobLimits {
    max_wait_time: i64,
    max_run_time: i64,
    full_max_run_time: i64,
    inc_max_run_time: i64,
    diff_max_run_time: i64,
    max_start_delay: i64,
    allow_duplicate_jobs: bool,
    cancel_lower_level: bool,
    cancel_queued: bool,
    cancel_running: bool,
    duplicate_job_proximity: i64,
    always_incremental: bool,
    backup: bool,
    fileset: String,
}

impl Default for JobLimits {
    fn default() -> Self {
        Self { max_wait_time: 0, max_run_time: 0, full_max_run_time: 0, inc_max_run_time: 0, diff_max_run_time: 0,
            max_start_delay: 0, allow_duplicate_jobs: true, cancel_lower_level: false, cancel_queued: false,
            cancel_running: false, duplicate_job_proximity: 0, always_incremental: false, backup: false,
            fileset: String::new() }
    }
}

/// Per-job state of the checks (JCR fields).
#[derive(Debug, Clone, Default)]
pub(super) struct LimitState {
    limits: JobLimits,
    flags: StartFlags,
    /// jcr->getJobLevel() ('F', 'I', 'D', 'f', …).
    level: char,
    /// jcr->initial_sched_time.
    initial_sched_time: i64,
    /// jcr->wait_time / wait_time_sum.
    wait_since: Option<i64>,
    wait_sum: i64,
}

/// The 60-second JobMonitorWatchdog timer.
#[derive(Debug, Default)]
pub(super) struct Monitor {
    next: Option<std::time::Instant>,
}

/// JobMonitorWatchdog interval (job.cc InitJobServer: wd->interval = 60).
const MONITOR_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

fn seconds(value: Option<&str>) -> i64 {
    value.and_then(|v| bareos_config::values::parse_time(v).ok()).map_or(0, |(_, d)| d.as_secs() as i64)
}

fn yes(value: Option<&str>, default: bool) -> bool {
    value.map_or(default, |v| matches!(v.to_ascii_lowercase().as_str(), "yes" | "true" | "1" | "on"))
}

/// The Job resource's limits (JobDefs included; the last directive wins).
pub(super) fn job_limits(config: &crate::director_config::DirectorConfig, job_name: &str) -> JobLimits {
    let effective = bareos_config::jobdefs::effective(&config.raw, "job", job_name).unwrap_or_default();
    let get = |key: &str| effective.iter().rev().find(|e| e.directive.norm_key() == key)
        .and_then(|e| e.directive.first().map(str::to_string));
    let job = config.find_job(job_name);
    JobLimits {
        max_wait_time: seconds(get("maxwaittime").as_deref()),
        max_run_time: seconds(get("maxruntime").as_deref()),
        full_max_run_time: seconds(get("fullmaxruntime").as_deref()),
        inc_max_run_time: seconds(get("incrementalmaxruntime").as_deref()),
        diff_max_run_time: seconds(get("differentialmaxruntime").as_deref()),
        max_start_delay: seconds(get("maxstartdelay").as_deref()),
        allow_duplicate_jobs: yes(get("allowduplicatejobs").as_deref(), true),
        cancel_lower_level: yes(get("cancellowerlevelduplicates").as_deref(), false),
        cancel_queued: yes(get("cancelqueuedduplicates").as_deref(), false),
        cancel_running: yes(get("cancelrunningduplicates").as_deref(), false),
        duplicate_job_proximity: seconds(get("duplicatejobproximity").as_deref()),
        always_incremental: yes(get("alwaysincremental").as_deref(), false),
        backup: job.is_some_and(|j| j.job_type == bareos_config::resources::JobType::Backup),
        fileset: job.map(|j| j.fileset.clone()).unwrap_or_default(),
    }
}

/// The level code of a worker's level ("full", "F", "Incremental", " ", …).
pub(super) fn level_code(level: &str) -> char {
    if level.chars().count() == 1 {
        return level.chars().next().unwrap_or(' ');
    }
    match level.to_ascii_lowercase().as_str() {
        "full" | "f" => 'F',
        "incremental" | "i" => 'I',
        "differential" | "d" => 'D',
        "virtualfull" => 'f',
        "since" | "s" => 'S',
        _ => level.chars().next().unwrap_or(' '),
    }
}

/// What AllowDuplicateJob() decides for a new job.
#[derive(Debug, PartialEq, Eq)]
enum Duplicate {
    Allow,
    /// The new job is canceled (fatal message).
    CancelMe(String),
    /// The other job is canceled.
    CancelOther(u32),
}

/// The other job as AllowDuplicateJob() sees it.
struct Other<'a> {
    job_id: u32,
    job_name: &'a str,
    status: char,
    level: char,
    backup: bool,
    always_incremental: bool,
    client: &'a str,
    fileset: &'a str,
    ignore_duplicate_check: bool,
    /// jcr->start_time (0 before job_thread()).
    start_time: i64,
}

/// AllowDuplicateJob() (job.cc:883) for the new job `me` among `others`.
#[allow(clippy::too_many_arguments)]
fn allow_duplicate(me_name: &str, me: &LimitState, me_client: &str, others: &[Other], now: i64) -> Duplicate {
    let l = &me.limits;
    let consolidation = me.level == 'f' && l.always_incremental;
    if !consolidation && (l.allow_duplicate_jobs || me.flags.ignore_duplicate_check) {
        return Duplicate::Allow;
    }
    for d in others {
        if d.job_id == 0 {
            continue;
        }
        if consolidation && d.level == 'f' && d.always_incremental && d.client == me_client && d.fileset == l.fileset {
            return Duplicate::CancelMe(format!(
                "JobId {} already running. Duplicate consolidation job of Client \"{me_client}\" and FileSet \"{}\" not allowed.\n",
                d.job_id, l.fileset));
        }
        if d.ignore_duplicate_check || d.job_name != me_name {
            continue;
        }
        if l.duplicate_job_proximity > 0 && now - d.start_time > l.duplicate_job_proximity {
            continue; // not really a duplicate
        }
        let mut cancel_dup = false;
        if l.cancel_lower_level && d.backup && l.backup {
            let cancel_me = match me.level {
                'F' => { cancel_dup = matches!(d.level, 'D' | 'I'); false }
                'D' => { cancel_dup = d.level == 'I'; d.level == 'F' }
                'I' => matches!(d.level, 'F' | 'D'),
                _ => false,
            };
            if cancel_me {
                return Duplicate::CancelMe(format!("JobId {} already running. Duplicate job not allowed.\n", d.job_id));
            }
        }
        // JS_Created, WaitJobRes/ClientRes/StoreRes/Priority/MaxJobs/StartTime
        if l.cancel_queued && matches!(d.status, 'C' | 'j' | 'c' | 's' | 'p' | 'd' | 't') {
            cancel_dup = true;
        }
        return if cancel_dup || l.cancel_running {
            Duplicate::CancelOther(d.job_id)
        } else {
            Duplicate::CancelMe(format!("JobId {} already running. Duplicate job not allowed.\n", d.job_id))
        };
    }
    Duplicate::Allow
}

/// MaxRunTime / Full/Inc/Diff Max Runtime reached (JobCheckMaxruntime).
fn run_time_exceeded(state: &LimitState, run_time: i64) -> bool {
    let l = &state.limits;
    match state.level {
        'F' if l.full_max_run_time != 0 && run_time >= l.full_max_run_time => true,
        'D' if l.diff_max_run_time != 0 && run_time >= l.diff_max_run_time => true,
        'I' if l.inc_max_run_time != 0 && run_time >= l.inc_max_run_time => true,
        _ => l.max_run_time > 0 && run_time >= l.max_run_time,
    }
}

/// JobCheckMaxrunschedtime().
fn run_sched_time_exceeded(state: &LimitState, now: i64) -> bool {
    state.flags.max_run_sched_time != 0 && now - state.initial_sched_time >= state.flags.max_run_sched_time
}

/// UpdateWaitTime(): the wait states of JobWaiting().
fn wait_state(status: char, sd_status: char) -> bool {
    matches!(status, 'F' | 'S' | 'm' | 'M' | 's' | 'j' | 'c' | 'd' | 'p') || matches!(sd_status, 'm' | 'M' | 'q' | 'd')
}

impl JobManager {
    fn post_info(&self, job_id: u32, job_name: &str, msg_type: crate::message_bus::MsgType, text: String) {
        crate::console_msg::post_job_message(&self.bus, self.config.as_ref(), BusMessage::JobMessage {
            job_id, job_name: job_name.to_string(), msg_type, text, timestamp: now_unix() });
    }

    /// SetupJob() for a job entering the queue: its limits and
    /// AllowDuplicateJob(). True when the new job is to be canceled.
    pub(super) fn admit_job(&mut self, job_id: u32, flags: StartFlags) -> bool {
        let Some(job) = self.get_job(job_id) else { return false };
        let name = job.job_name.clone();
        let client = job.resources.as_ref().map(|r| r.0.clone()).unwrap_or_default();
        let level = job.jcr.params.as_ref().map_or(' ', |p| level_code(&p.level));
        let limits = match self.config.as_ref().and_then(|c| c.read().ok()) {
            Some(config) => job_limits(&config, &name),
            None => JobLimits::default(),
        };
        let state = LimitState { limits, flags, level, initial_sched_time: job.jcr.sched_time, ..Default::default() };
        let now = now_unix();
        let decision = {
            let others: Vec<Other> = self.jobs.iter()
                .filter(|j| j.job_id != job_id && !matches!(j.phase, DirJobPhase::Completed | DirJobPhase::Error)
                    && j.worker_exit.is_none())
                .map(|j| Other {
                    job_id: j.job_id, job_name: &j.job_name, status: j.status(), level: j.jcr.limits.level,
                    backup: j.jcr.limits.limits.backup, always_incremental: j.jcr.limits.limits.always_incremental,
                    client: j.resources.as_ref().map_or("", |r| r.0.as_str()),
                    fileset: &j.jcr.limits.limits.fileset,
                    ignore_duplicate_check: j.jcr.limits.flags.ignore_duplicate_check,
                    start_time: if j.jcr.started_seq > 0 { j.start_time } else { 0 },
                }).collect();
            allow_duplicate(&name, &state, &client, &others, now)
        };
        if let Some(job) = self.jobs.iter_mut().find(|j| j.job_id == job_id) {
            job.jcr.limits = state;
        }
        match decision {
            Duplicate::Allow => false,
            Duplicate::CancelMe(text) => {
                self.set_status(job_id, 'A');
                self.post_info(job_id, &name, crate::message_bus::MsgType::Fatal, text);
                true
            }
            Duplicate::CancelOther(other) => {
                use crate::message_bus::MsgType::Info;
                self.post_info(job_id, &name, Info, format!("Cancelling duplicate JobId={other}.\n"));
                let unique = self.get_job(other).map(|j| j.handle.unique_job.clone()).unwrap_or_default();
                if self.cancel_job(other) {
                    self.post_info(job_id, &name, Info, format!("JobId {other}, Job {unique} marked to be canceled.\n"));
                }
                false
            }
        }
    }

    /// job_thread()'s checks when job `id` leaves the queue: the fatal
    /// texts (the job is then JS_Canceled), empty when it may run.
    pub(super) fn start_checks(&mut self, id: u32) -> Vec<String> {
        let now = now_unix();
        let Some(job) = self.get_job(id) else { return Vec::new() };
        let state = &job.jcr.limits;
        let mut fatals = Vec::new();
        if state.limits.max_start_delay != 0 && state.limits.max_start_delay < now - job.jcr.sched_time {
            fatals.push("Job canceled because max start delay time exceeded.\n".to_string());
        }
        if run_sched_time_exceeded(state, now) {
            fatals.push("Job canceled because max run sched time exceeded.\n".to_string());
        }
        if !fatals.is_empty() {
            self.set_status(id, 'A');
        }
        fatals
    }

    /// Post the job_thread() fatal texts of a job canceled at its start.
    pub(super) fn post_start_fatals(&self, id: u32, job_name: &str, fatals: Vec<String>) {
        for text in fatals {
            self.post_info(id, job_name, crate::message_bus::MsgType::Fatal, text);
        }
    }

    /// UpdateWaitTime() for every job, and every 60 seconds the
    /// JobMonitorWatchdog checks (called from poll_all).
    pub(super) fn job_monitor(&mut self) {
        let now = now_unix();
        for job in &mut self.jobs {
            let waiting = wait_state(job.status(), job.jcr.sd_status);
            let state = &mut job.jcr.limits;
            match (waiting, state.wait_since) {
                (true, None) => state.wait_since = Some(now),
                (false, Some(since)) => { state.wait_sum += now - since; state.wait_since = None; }
                _ => {}
            }
        }
        let instant = std::time::Instant::now();
        let next = *self.monitor.next.get_or_insert(instant + MONITOR_INTERVAL);
        if instant < next {
            return;
        }
        self.monitor.next = Some(instant + MONITOR_INTERVAL);
        let mut cancel = Vec::new();
        for job in &self.jobs {
            if job.job_id == 0 || matches!(job.phase, DirJobPhase::Completed | DirJobPhase::Error)
                || job.worker_exit.is_some() || is_job_canceled(job.status()) {
                continue;
            }
            let state = &job.jcr.limits;
            let text = if job.jcr.job_started && wait_state(job.status(), job.jcr.sd_status)
                && state.limits.max_wait_time != 0
                && state.wait_since.map_or(0, |s| now - s) + state.wait_sum >= state.limits.max_wait_time {
                "Max wait time exceeded. Job canceled.\n"
            } else if job.jcr.job_started && run_time_exceeded(state, now - job.start_time) {
                "Max run time exceeded. Job canceled.\n"
            } else if run_sched_time_exceeded(state, now) {
                "Max run sched time exceeded. Job canceled.\n"
            } else {
                continue;
            };
            cancel.push((job.job_id, job.job_name.clone(), text));
        }
        for (id, name, text) in cancel {
            self.set_status(id, 'A');
            self.post_info(id, &name, crate::message_bus::MsgType::Fatal, text.to_string());
            self.cancel_job(id);
        }
    }

    /// Run the JobMonitorWatchdog check now (tests).
    #[cfg(test)]
    pub(super) fn fire_job_monitor(&mut self) {
        self.monitor.next = Some(std::time::Instant::now());
        self.job_monitor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn me(limits: JobLimits, level: char) -> LimitState {
        LimitState { limits, level, ..Default::default() }
    }

    fn other(id: u32, status: char, level: char) -> Other<'static> {
        Other { job_id: id, job_name: "j", status, level, backup: true, always_incremental: false, client: "c",
            fileset: "fs", ignore_duplicate_check: false, start_time: 0 }
    }

    fn no_dup() -> JobLimits {
        JobLimits { allow_duplicate_jobs: false, backup: true, fileset: "fs".into(), ..Default::default() }
    }

    #[test]
    fn duplicates_allowed_by_default_or_ignored() {
        assert_eq!(allow_duplicate("j", &me(JobLimits::default(), 'F'), "c", &[other(1, 'R', 'F')], 0), Duplicate::Allow);
        let mut s = me(no_dup(), 'F');
        s.flags.ignore_duplicate_check = true;
        assert_eq!(allow_duplicate("j", &s, "c", &[other(1, 'R', 'F')], 0), Duplicate::Allow);
        // other job names and ignoring others do not count
        let mut o = other(2, 'R', 'F');
        o.ignore_duplicate_check = true;
        assert_eq!(allow_duplicate("x", &me(no_dup(), 'F'), "c", &[other(1, 'R', 'F'), o], 0), Duplicate::Allow);
    }

    #[test]
    fn duplicate_cancels_me_or_the_other() {
        let msg = "JobId 1 already running. Duplicate job not allowed.\n".to_string();
        assert_eq!(allow_duplicate("j", &me(no_dup(), 'F'), "c", &[other(1, 't', 'F')], 0), Duplicate::CancelMe(msg.clone()));
        let queued = JobLimits { cancel_queued: true, ..no_dup() };
        assert_eq!(allow_duplicate("j", &me(queued.clone(), 'F'), "c", &[other(1, 't', 'F')], 0), Duplicate::CancelOther(1));
        assert_eq!(allow_duplicate("j", &me(queued, 'F'), "c", &[other(1, 'R', 'F')], 0), Duplicate::CancelMe(msg.clone()));
        let running = JobLimits { cancel_running: true, ..no_dup() };
        assert_eq!(allow_duplicate("j", &me(running, 'F'), "c", &[other(1, 'R', 'F')], 0), Duplicate::CancelOther(1));
        let lower = JobLimits { cancel_lower_level: true, ..no_dup() };
        assert_eq!(allow_duplicate("j", &me(lower.clone(), 'F'), "c", &[other(1, 'R', 'I')], 0), Duplicate::CancelOther(1));
        assert_eq!(allow_duplicate("j", &me(lower.clone(), 'I'), "c", &[other(1, 'R', 'F')], 0), Duplicate::CancelMe(msg.clone()));
        assert_eq!(allow_duplicate("j", &me(lower, 'F'), "c", &[other(1, 'R', 'F')], 0), Duplicate::CancelMe(msg));
        // proximity: a job started long ago is not a duplicate
        let near = JobLimits { duplicate_job_proximity: 60, ..no_dup() };
        let mut o = other(1, 'R', 'F');
        o.start_time = 1000;
        assert_eq!(allow_duplicate("j", &me(near.clone(), 'F'), "c", &[o], 1061), Duplicate::Allow);
        let mut o = other(1, 'R', 'F');
        o.start_time = 1000;
        assert!(matches!(allow_duplicate("j", &me(near, 'F'), "c", &[o], 1030), Duplicate::CancelMe(_)));
    }

    #[test]
    fn consolidation_virtual_full_is_never_duplicated() {
        let ai = JobLimits { always_incremental: true, fileset: "fs".into(), ..Default::default() };
        let mut s = me(ai, 'f');
        s.flags.ignore_duplicate_check = true;
        let mut o = other(7, 'R', 'f');
        o.always_incremental = true;
        o.job_name = "other-job";
        assert_eq!(allow_duplicate("j", &s, "c", &[o], 0), Duplicate::CancelMe(
            "JobId 7 already running. Duplicate consolidation job of Client \"c\" and FileSet \"fs\" not allowed.\n".into()));
    }

    #[test]
    fn run_time_limits_by_level() {
        let l = JobLimits { max_run_time: 100, full_max_run_time: 50, ..Default::default() };
        assert!(run_time_exceeded(&me(l.clone(), 'F'), 50));
        assert!(!run_time_exceeded(&me(l.clone(), 'I'), 50));
        assert!(run_time_exceeded(&me(l, 'I'), 100));
        assert!(!run_time_exceeded(&me(JobLimits::default(), 'F'), 1_000_000));
    }

    #[test]
    fn run_sched_time_counts_from_initial_sched_time() {
        let mut s = me(JobLimits::default(), 'F');
        s.initial_sched_time = 1000;
        assert!(!run_sched_time_exceeded(&s, 5000));
        s.flags.max_run_sched_time = 60;
        assert!(!run_sched_time_exceeded(&s, 1059));
        assert!(run_sched_time_exceeded(&s, 1060));
    }

    #[test]
    fn level_codes() {
        assert_eq!(level_code("full"), 'F');
        assert_eq!(level_code("Incremental"), 'I');
        assert_eq!(level_code("D"), 'D');
        assert_eq!(level_code("VirtualFull"), 'f');
        assert_eq!(level_code("f"), 'f');
        assert_eq!(level_code(" "), ' ');
        assert_eq!(level_code(""), ' ');
    }
}
