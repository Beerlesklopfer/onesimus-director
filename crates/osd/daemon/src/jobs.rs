// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Die Jobs des SD: Job-Kommando, Tabelle, Warten auf den File Daemon.
//!
//! Zweck: Der Director meldet jeden Job mit einer Zeile `JobId=… job=…`
//! an. osd vergibt Sitzung (`VolSessionId`/`VolSessionTime`) und einen
//! Schlüssel, mit dem sich der FD später ausweist, und merkt sich den Job
//! in einer Tabelle unter seinem eindeutigen Namen. Die FD-Verbindung
//! (aktiv: der FD ruft an; passiv: osd ruft den FD an) landet im Job, und
//! `run` wartet darauf.
//!
//! Bareos-Gegenstück: `core/src/stored/job.cc` (`job_cmd`, `WaitFD`,
//! `DoJobRun`), `lib/jcr.cc` (`get_jcr_by_full_name`), `stored/stored.cc`
//! (`NewVolSessionId`, `vol_session_time`), `lib/util.cc`
//! (`MakeSessionKey`).

use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bareos_format::bash::unbash_spaces;

use crate::transport::{Channel, SharedChannel};

/// `use storage` of a job: what the Director wants written where.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UseStorage {
    /// `use storage=`.
    pub storage: String,
    /// `media_type=`.
    pub media_type: String,
    /// `pool_name=`.
    pub pool_name: String,
    /// `pool_type=`.
    pub pool_type: String,
    /// `append=`: 1 for writing.
    pub append: bool,
    /// The `use device=` names.
    pub devices: Vec<String>,
}

/// The fields of the Job command (`stored/job.cc` `jobcmd`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobCommand {
    /// `JobId=`.
    pub job_id: u32,
    /// `job=`: the unique job name.
    pub job: String,
    /// `job_name=` (unbashed).
    pub job_name: String,
    /// `client_name=` (unbashed).
    pub client_name: String,
    /// `type=` letter.
    pub job_type: char,
    /// `level=` letter.
    pub level: char,
    /// `FileSet=` (unbashed).
    pub fileset: String,
    /// `NoAttr=`.
    pub no_attributes: bool,
    /// `SpoolAttr=`.
    pub spool_attributes: bool,
    /// `FileSetMD5=`.
    pub fileset_md5: String,
    /// `SpoolData=`.
    pub spool_data: bool,
    /// `rerunning=`.
    pub rerunning: bool,
    /// `VolSessionId=` (only meaningful when rerunning).
    pub vol_session_id: u32,
    /// `VolSessionTime=`.
    pub vol_session_time: u32,
    /// `Quota=`.
    pub quota: u64,
    /// `Protocol=`.
    pub protocol: i32,
    /// `BackupFormat=`.
    pub backup_format: String,
}

impl JobCommand {
    /// `sscanf(msg, jobcmd, …) == 19`; `None` otherwise (the caller answers
    /// `3915 Bad Job command. stat=<n> CMD: …`, Bareos reports the number of
    /// converted fields, osd reports 0).
    pub fn parse(msg: &str) -> Option<JobCommand> {
        let keys = [
            "JobId=", "job=", "job_name=", "client_name=", "type=", "level=", "FileSet=", "NoAttr=", "SpoolAttr=",
            "FileSetMD5=", "SpoolData=", "PreferMountedVols=", "SpoolSize=", "rerunning=", "VolSessionId=",
            "VolSessionTime=", "Quota=", "Protocol=", "BackupFormat=",
        ];
        let words: Vec<&str> = msg.split_whitespace().collect();
        if words.len() < keys.len() {
            return None;
        }
        let mut v = Vec::with_capacity(keys.len());
        for (w, k) in words.iter().zip(keys) {
            v.push(w.strip_prefix(k)?);
        }
        let int = |i: usize| v[i].parse::<i64>().ok();
        let letter = |i: usize| int(i).and_then(|n| char::from_u32(n as u32));
        Some(JobCommand {
            job_id: int(0)? as u32,
            job: v[1].to_string(),
            job_name: unbash_spaces(v[2]),
            client_name: unbash_spaces(v[3]),
            job_type: letter(4)?,
            level: letter(5)?,
            fileset: unbash_spaces(v[6]),
            no_attributes: int(7)? != 0,
            spool_attributes: int(8)? != 0,
            fileset_md5: v[9].to_string(),
            spool_data: int(10)? != 0,
            rerunning: int(13)? != 0,
            vol_session_id: int(14)? as u32,
            vol_session_time: int(15)? as u32,
            quota: v[16].parse().ok()?,
            protocol: int(17)? as i32,
            backup_format: unbash_spaces(v[18]),
        })
    }
}

/// The bootstrap of a read job (`bootstrap` command of the Director).
#[derive(Debug, Clone)]
pub struct Bootstrap {
    /// The file it was written to (`<Working Directory>/<Storage>.<Job>.<n>.bootstrap`);
    /// the device reader reads it again, it is removed with the job.
    pub path: std::path::PathBuf,
    /// The parsed bootstrap.
    pub bsr: bareos_format::bsr::Bsr,
}

/// How a cancel breaks the FD connection of a job.
pub enum Breaker {
    /// Nothing to break (tests).
    None,
    /// BNET: shut the socket down.
    Socket(TcpStream),
    /// gRPC: set the stream's close flag (`ControlIo::close_handle`).
    Grpc(std::sync::Arc<AtomicBool>),
}

impl Breaker {
    /// The breaker of a BNET connection with socket `socket`, if any.
    pub fn socket(socket: Option<TcpStream>) -> Breaker {
        socket.map(Breaker::Socket).unwrap_or(Breaker::None)
    }

    /// Breaks the connection: blocked reads return an error.
    pub fn break_connection(&self) {
        match self {
            Breaker::None => {}
            Breaker::Socket(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            Breaker::Grpc(close) => close.store(true, Ordering::Release),
        }
    }
}

/// The FD side of a job once connected and authenticated.
pub struct FdConnection {
    /// The data channel.
    pub channel: Box<dyn Channel>,
    /// How a cancel breaks the connection.
    pub breaker: Breaker,
    /// The peer is the reading SD of a Copy/Migrate (`Hello Start Storage Job`), not an FD.
    pub sd: bool,
    /// The connection osd made to the writing SD of a Copy (`replicate`); osd sends the data.
    pub replicate: bool,
}

/// How a branch job ended, handed from the main job's append to the
/// branch's control session, which then reports `3099` for it.
#[derive(Default)]
pub struct BranchEnd {
    /// The status letter of the branch once it ended.
    state: Mutex<Option<char>>,
    /// Signalled when [`BranchEnd::state`] is set.
    cond: Condvar,
}

impl BranchEnd {
    /// Marks the branch as ended with `status` (the first call wins).
    pub fn complete(&self, status: char) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.is_none() {
            *state = Some(status);
        }
        self.cond.notify_all();
    }

    /// Whether the branch has ended.
    pub fn is_complete(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// Waits until the branch ended; wakes at least every `step` to call
    /// `abandoned`, which returns a status when the wait should end (the
    /// branch was canceled, its main job is gone). The final status.
    pub fn wait(&self, step: Duration, mut abandoned: impl FnMut() -> Option<char>) -> char {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(status) = *state {
                return status;
            }
            if let Some(status) = abandoned() {
                *state = Some(status);
                return status;
            }
            state = self.cond.wait_timeout(state, step).unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}

/// A branch job that attached to a main job (`BranchAttach`): the stream of
/// the main job is also written for it.
#[derive(Clone)]
pub struct AttachedBranch {
    /// The branch job.
    pub job: Arc<Job>,
    /// The branch job's Director connection (its job messages, attributes
    /// and catalog requests go there, not to the main job's).
    pub dir: SharedChannel,
    /// Signalled when the branch ended.
    pub end: Arc<BranchEnd>,
}

/// The branches of a main job.
#[derive(Default)]
pub struct Branches {
    /// The unique names the Director announced (`BranchPlan`).
    planned: Mutex<Vec<String>>,
    /// The branches that attached.
    attached: Mutex<Vec<AttachedBranch>>,
    /// The stream began: later attaches are refused.
    closed: AtomicBool,
    /// Signalled when a branch attached or the stream began.
    changed: Condvar,
}

impl Branches {
    /// Records the names of the branch jobs the main job waits for.
    pub fn plan(&self, names: Vec<String>) {
        *self.planned.lock().unwrap_or_else(|e| e.into_inner()) = names;
    }

    /// The names announced.
    pub fn planned(&self) -> Vec<String> {
        self.planned.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// A branch attaches; false when the stream began already.
    pub fn attach(&self, branch: AttachedBranch) -> bool {
        let mut attached = self.attached.lock().unwrap_or_else(|e| e.into_inner());
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        attached.push(branch);
        self.changed.notify_all();
        true
    }

    /// The branches that attached up to now.
    pub fn attached(&self) -> Vec<AttachedBranch> {
        self.attached.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Waits until every planned branch attached or is `gone` (it was seen
    /// and left without attaching, e.g. its `use storage` failed) or until
    /// `timeout`; closes the set for later attaches. The branches that
    /// attached and the planned ones that did not, with the reason.
    pub fn collect(&self, timeout: Duration, mut gone: impl FnMut(&str) -> Option<String>) -> (Vec<AttachedBranch>, Vec<(String, String)>) {
        let planned = self.planned();
        let deadline = Instant::now() + timeout;
        let mut attached = self.attached.lock().unwrap_or_else(|e| e.into_inner());
        let missing = loop {
            let mut missing: Vec<(String, String)> = Vec::new();
            let mut waiting = false;
            for name in planned.iter().filter(|n| !attached.iter().any(|b| &b.job.cmd.job == *n)) {
                match gone(name) {
                    Some(why) => missing.push((name.clone(), why)),
                    None => {
                        waiting = true;
                        if Instant::now() >= deadline {
                            missing.push((name.clone(), "it did not attach in time".to_string()));
                        }
                    }
                }
            }
            if !waiting || Instant::now() >= deadline {
                break missing;
            }
            attached = self.changed.wait_timeout(attached, Duration::from_millis(100)).unwrap_or_else(|e| e.into_inner()).0;
        };
        self.closed.store(true, Ordering::Release);
        (attached.clone(), missing)
    }
}

/// One job of the SD (`JobControlRecord` of `stored`).
pub struct Job {
    /// The Job command.
    pub cmd: JobCommand,
    /// `VolSessionId`.
    pub vol_session_id: u32,
    /// `VolSessionTime`.
    pub vol_session_time: u32,
    /// Key the FD authenticates with (`sd_auth_key`).
    pub auth_key: String,
    /// `use storage` (set by the Director before `run`).
    pub storage: Mutex<Option<UseStorage>>,
    /// The device reserved for reading (a restore, verify, or the read side of a copy).
    pub read_device: Mutex<Option<String>>,
    /// The other targets of a Distributor a read job holds besides
    /// [`Job::read_device`]; the reader switches to them per volume.
    pub read_alternatives: Mutex<Vec<String>>,
    /// The write side of a copy inside this SD (`use storage append=1` after a read reservation).
    pub local_write: Mutex<Option<UseStorage>>,
    /// The bootstrap of a restore or verify (`bootstrap` command).
    pub bootstrap: Mutex<Option<Bootstrap>>,
    /// The FD connection, once there.
    pub fd: Mutex<Option<FdConnection>>,
    /// Signalled when the FD connected or the job was canceled.
    pub fd_arrived: Condvar,
    /// The FD authenticated (`jcr->authenticated`).
    pub authenticated: AtomicBool,
    /// The job was canceled.
    pub canceled: AtomicBool,
    /// Job status letter (`jcr->JobStatus`).
    pub status: AtomicU32,
    /// `JobFiles` so far.
    pub files: AtomicU32,
    /// `JobBytes` so far (low and high in one Mutex for simplicity).
    pub bytes: Mutex<u64>,
    /// When the job was created.
    pub created: Instant,
    /// The branch jobs of this (main) job.
    pub branches: Branches,
    /// For a branch job: the unique name of its main job (`BranchAttach`).
    pub branch_of: Mutex<Option<String>>,
}

impl Job {
    /// The job status letter.
    pub fn status(&self) -> char {
        char::from_u32(self.status.load(Ordering::Acquire)).unwrap_or('?')
    }

    /// Sets the status with `setJobStatusWithPriorityCheck`'s rule that
    /// errors and cancel are not overwritten by milder states.
    pub fn set_status(&self, status: char) {
        let priority = |c: char| match c {
            'f' | 'E' => 3,
            'A' => 2,
            _ => 0,
        };
        let old = self.status();
        if priority(status) >= priority(old) {
            self.status.store(status as u32, Ordering::Release);
        }
    }

    /// Waits until the FD is there or the job is canceled, at most
    /// `timeout` (`WaitFD`). True when authenticated.
    pub fn wait_fd(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut fd = self.fd.lock().unwrap_or_else(|e| e.into_inner());
        while fd.is_none() && !self.canceled.load(Ordering::Acquire) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            fd = self.fd_arrived.wait_timeout(fd, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        fd.is_some() && self.authenticated.load(Ordering::Acquire)
    }

    /// Hands the authenticated FD connection to the job and wakes `run`.
    pub fn connect_fd(&self, conn: FdConnection) {
        *self.fd.lock().unwrap_or_else(|e| e.into_inner()) = Some(conn);
        self.authenticated.store(true, Ordering::Release);
        self.fd_arrived.notify_all();
    }

    /// `CancelCmd`: marks the job canceled, breaks the FD connection and
    /// wakes a waiting `run`.
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
        self.set_status('A');
        if let Some(conn) = self.fd.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            conn.breaker.break_connection();
        }
        self.fd_arrived.notify_all();
    }
}

/// The table of jobs, by unique job name.
#[derive(Default)]
pub struct JobTable {
    /// The jobs.
    jobs: Mutex<HashMap<String, Arc<Job>>>,
    /// Last `VolSessionId` handed out.
    last_session: AtomicU32,
}

impl JobTable {
    /// `job_cmd`: registers a job; a job of the same name that never
    /// authenticated is replaced (`ojcr`).
    pub fn create(&self, cmd: JobCommand, vol_session_time: u32) -> Arc<Job> {
        let (sid, stime) = if cmd.rerunning {
            (cmd.vol_session_id, cmd.vol_session_time)
        } else {
            (self.last_session.fetch_add(1, Ordering::AcqRel) + 1, vol_session_time)
        };
        let job = Arc::new(Job {
            vol_session_id: sid,
            vol_session_time: stime,
            auth_key: make_session_key(),
            storage: Mutex::new(None),
            read_device: Mutex::new(None),
            read_alternatives: Mutex::new(Vec::new()),
            local_write: Mutex::new(None),
            bootstrap: Mutex::new(None),
            fd: Mutex::new(None),
            fd_arrived: Condvar::new(),
            authenticated: AtomicBool::new(false),
            canceled: AtomicBool::new(false),
            status: AtomicU32::new('C' as u32),
            files: AtomicU32::new(0),
            bytes: Mutex::new(0),
            created: Instant::now(),
            branches: Branches::default(),
            branch_of: Mutex::new(None),
            cmd,
        });
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).insert(job.cmd.job.clone(), job.clone());
        job
    }

    /// The job named `job` (`get_jcr_by_full_name`).
    pub fn find(&self, job: &str) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).get(job).cloned()
    }

    /// The job with JobId `id` (`get_jcr_by_id`).
    pub fn find_id(&self, id: u32) -> Option<Arc<Job>> {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).values().find(|j| j.cmd.job_id == id).cloned()
    }

    /// Removes the job `job` (`FreeJcr`).
    pub fn remove(&self, job: &str) {
        self.jobs.lock().unwrap_or_else(|e| e.into_inner()).remove(job);
    }

    /// All jobs, oldest first.
    pub fn all(&self) -> Vec<Arc<Job>> {
        let mut v: Vec<_> = self.jobs.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        v.sort_by_key(|j| j.created);
        v
    }
}

/// `MakeSessionKey`: eight groups of four letters `A`–`P`, like
/// `PMFF-PFGC-PEIG-FGCO-HNFP-PBHF-OLAA-ECII`, from 16 random bytes.
pub fn make_session_key() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut bytes);
    }
    let mut key = String::with_capacity(39);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 2 == 0 {
            key.push('-');
        }
        key.push((b'A' + (b >> 4)) as char);
        key.push((b'A' + (b & 0x0f)) as char);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Job command odir sent in the rig.
    const CMD: &str = "JobId=1 job=VBackup.2026-11-20_19.59.23_03 job_name=VBackup client_name=vfd type=66 level=70 \
        FileSet=VSet NoAttr=0 SpoolAttr=0 FileSetMD5=FE/BE0kIqW SpoolData=0 PreferMountedVols=1 SpoolSize=0 rerunning=0 \
        VolSessionId=0 VolSessionTime=0 Quota=0 Protocol=0 BackupFormat=Native\n";

    #[test]
    fn job_command_is_scanned_like_job_cmd() {
        let c = JobCommand::parse(CMD).unwrap();
        assert_eq!((c.job_id, c.job.as_str(), c.job_type, c.level), (1, "VBackup.2026-11-20_19.59.23_03", 'B', 'F'));
        assert_eq!((c.fileset.as_str(), c.fileset_md5.as_str(), c.client_name.as_str()), ("VSet", "FE/BE0kIqW", "vfd"));
        assert!(JobCommand::parse(&CMD.replace("job_name=VBackup", "job_name=V\u{1}Backup")).unwrap().job_name == "V Backup");
        assert_eq!(JobCommand::parse("JobId=1 job=x"), None);
    }

    #[test]
    fn sessions_count_up_and_keys_look_like_bareos() {
        let t = JobTable::default();
        let a = t.create(JobCommand::parse(CMD).unwrap(), 1000);
        let b = t.create(JobCommand::parse(&CMD.replace("_03", "_04").replace("JobId=1 ", "JobId=2 ")).unwrap(), 1000);
        assert_eq!((a.vol_session_id, b.vol_session_id, a.vol_session_time), (1, 2, 1000));
        assert_eq!(a.auth_key.len(), 39);
        assert!(a.auth_key.chars().all(|c| c == '-' || ('A'..='P').contains(&c)));
        assert!(t.find("VBackup.2026-11-20_19.59.23_04").is_some());
        assert_eq!(t.find_id(2).map(|j| j.cmd.job.clone()), Some("VBackup.2026-11-20_19.59.23_04".to_string()));
    }

    #[test]
    fn status_priority_keeps_errors() {
        let t = JobTable::default();
        let j = t.create(JobCommand::parse(CMD).unwrap(), 1);
        j.set_status('R');
        j.set_status('E');
        j.set_status('T');
        assert_eq!(j.status(), 'E');
    }

    /// A job named `name` with JobId `id` in `table`.
    fn job(table: &JobTable, name: &str, id: u32) -> Arc<Job> {
        let cmd = CMD.replace("JobId=1 ", &format!("JobId={id} ")).replace("VBackup.2026-11-20_19.59.23_03", name);
        table.create(JobCommand::parse(&cmd).unwrap(), 1)
    }

    /// A connection that goes nowhere (the Director side of a branch in tests).
    fn nowhere() -> SharedChannel {
        let (a, _b) = crate::transport::MemChannel::pair();
        Arc::new(Mutex::new(Box::new(a)))
    }

    #[test]
    fn a_main_job_collects_the_planned_branches_that_attach_and_names_the_gone_ones() {
        let t = JobTable::default();
        let main = job(&t, "Main.1", 1);
        let (b1, b2) = (job(&t, "B.1", 2), job(&t, "B.2", 3));
        main.branches.plan(vec![b1.cmd.job.clone(), b2.cmd.job.clone()]);
        let attaching = {
            let (main, b1) = (main.clone(), b1.clone());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                assert!(main.branches.attach(AttachedBranch { job: b1, dir: nowhere(), end: Arc::default() }));
            })
        };
        // B.2 was seen and left (its use storage failed): no waiting for it
        t.remove("B.2");
        let (attached, missing) = main.branches.collect(Duration::from_secs(10), |n| (n == "B.2").then(|| "its job ended".to_string()));
        attaching.join().unwrap();
        assert_eq!(attached.iter().map(|b| b.job.cmd.job.as_str()).collect::<Vec<_>>(), ["B.1"]);
        assert_eq!(missing, [("B.2".to_string(), "its job ended".to_string())]);
        // the stream began: a late branch is refused
        assert!(!main.branches.attach(AttachedBranch { job: b2, dir: nowhere(), end: Arc::default() }));
    }

    #[test]
    fn a_branch_that_never_comes_costs_the_wait_and_no_more() {
        let t = JobTable::default();
        let main = job(&t, "Main.1", 1);
        main.branches.plan(vec!["Never.1".into()]);
        let begin = Instant::now();
        let (attached, missing) = main.branches.collect(Duration::from_millis(300), |_| None);
        assert!(attached.is_empty() && begin.elapsed() >= Duration::from_millis(300) && begin.elapsed() < Duration::from_secs(5));
        assert_eq!(missing, [("Never.1".to_string(), "it did not attach in time".to_string())]);
        // no plan: nothing to wait for
        let alone = job(&t, "Alone.1", 2);
        let (attached, missing) = alone.branches.collect(Duration::from_secs(10), |_| None);
        assert!(attached.is_empty() && missing.is_empty());
    }

    #[test]
    fn a_branch_end_is_final_and_a_watcher_can_end_the_wait() {
        let end = BranchEnd::default();
        assert!(!end.is_complete());
        assert_eq!(end.wait(Duration::from_millis(20), || Some('A')), 'A', "the watcher ends the wait");
        end.complete('T');
        assert_eq!(end.wait(Duration::from_millis(20), || None), 'A', "the first end stays");
        let fresh = BranchEnd::default();
        fresh.complete('T');
        fresh.complete('E');
        assert_eq!(fresh.wait(Duration::from_millis(20), || None), 'T');
    }
}
