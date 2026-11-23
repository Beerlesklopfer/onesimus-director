// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Zweige (Branch): ein Stream, mehrere Jobs.
//!
//! Zweck: 3-2-1 im laufenden Betrieb. Der FD liest die Daten einmal; der
//! Haupt-Job gibt jeden Datensatz an seine Zweige weiter. Ein Zweig ist ein
//! eigener Job des Directors (eigene JobId, Pool, Device, Volumes,
//! Katalogsätze), der keine FD-Sitzung hat, sondern sich an den Haupt-Job
//! hängt. Dieses Modul ist die Steuerseite: die Nachrichten des Directors
//! (`BranchPlan` am Haupt-Job, `BranchAttach` an einem Zweig, beide nur über
//! gRPC, [`onesimus_bnet_grpc::branch`]) und die Sitzung eines Zweigs, die
//! bis zum Ende des Haupt-Jobs wartet und dann `3099` meldet. Die
//! Datenseite (jede Senke mit eigenem Ring und Geräteschreiber) steht in
//! [`crate::fd_session`].
//!
//! Fehlerisolation (D-11): Scheitert ein Zweig (Device nicht vorhanden,
//! Schreiber fällt aus, Abbruch), endet nur dessen Job mit Fehler; der
//! Haupt-Job und die übrigen Zweige schreiben weiter.
//!
//! Bareos-Gegenstück: keins (Onesimus-Erweiterung); die Sitzung ist `RunCmd`
//! (`stored/dir_cmd.cc`) ohne `WaitFD`.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bareos_bnet::BnetSignal;
use bareos_format::msg::M_FATAL;
use onesimus_bnet_grpc::branch::BranchMessage;

use crate::daemon::Daemon;
use crate::jobs::{AttachedBranch, BranchEnd, Job};
use crate::transport::{lock, SharedChannel};

/// Slots of the ring of a job that is part of a branch group when neither
/// the Device nor the Storage says `Ring Slots` (D-25): the slowest branch
/// sets the pace, a larger ring absorbs fluctuations.
pub const BRANCH_RING_SLOTS: u32 = 512;

/// How often a waiting branch looks whether it was canceled or its main job
/// is gone.
const WATCH_STEP: Duration = Duration::from_millis(200);

/// `BranchPlan` on a main job's connection: remembers the branch jobs it
/// waits for. The answer for the Director.
pub fn plan(job: &Job, names: Vec<String>) -> BranchMessage {
    job.branches.plan(names);
    BranchMessage::Reply(Ok(()))
}

/// `BranchAttach` on a branch job's connection: the branch hangs itself on
/// `main`, the job named by the Director. The branch job must have its
/// `use storage` (it is written like a job of its own). Waits up to `wait`
/// for the main job (its Job command may come later). `Ok` with what the
/// main job needs, or the refusal.
pub fn attach(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, main: &str, wait: Duration) -> Result<(Arc<Job>, Arc<BranchEnd>), String> {
    if job.storage.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_none_or(|s| !s.append) {
        return Err("a branch job needs its use storage (append) before it attaches".into());
    }
    if main == job.cmd.job {
        return Err("a job cannot be its own branch".into());
    }
    let deadline = Instant::now() + wait;
    let main_job = loop {
        if let Some(j) = d.jobs.find(main) {
            break j;
        }
        if job.canceled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err(format!("main job {main} is not known to this Storage Daemon"));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let end = Arc::new(BranchEnd::default());
    let branch = AttachedBranch { job: job.clone(), dir: dir.clone(), end: end.clone() };
    if !main_job.branches.attach(branch) {
        return Err(format!("main job {main} already writes: the branch came too late"));
    }
    *job.branch_of.lock().unwrap_or_else(|e| e.into_inner()) = Some(main.to_string());
    Ok((main_job, end))
}

/// The branch job's `run`: it reports its start, waits until the main job
/// wrote its stream (or the branch is canceled, the main job is gone) and
/// reports the end of the job like `run_job` does (`3099`).
pub fn run_branch(d: &Daemon, job: &Arc<Job>, dir: &SharedChannel, main: &Arc<Job>, end: &BranchEnd) {
    let name = job.cmd.job.clone();
    let _ = lock(dir).send_str(&format!("3010 Job {name} start\n"));
    d.send_status(dir, job, 'R');
    let status = end.wait(WATCH_STEP, || {
        if job.canceled.load(Ordering::Acquire) {
            return Some('A');
        }
        // the main job left the table without handing the stream on
        d.jobs.find(&main.cmd.job).is_none().then_some('E')
    });
    if !matches!(status, 'T' | 'W') && !job.canceled.load(Ordering::Acquire) && job.status() != 'E' {
        d.jmsg(dir, job, M_FATAL, &format!("The main job {} ended before it wrote this branch.\n", main.cmd.job));
    }
    job.set_status(status);
    let bytes = *job.bytes.lock().unwrap_or_else(|e| e.into_inner());
    let mut dir_lock = lock(dir);
    let _ = dir_lock.send_str(&format!(
        "3099 Job {name} end JobStatus={} JobFiles={} JobBytes={bytes} JobErrors=0\n",
        job.status() as u32,
        job.files.load(Ordering::Acquire)
    ));
    let _ = dir_lock.signal(BnetSignal::Eod);
}

/// The main job's session ended: a branch that is still waiting for its
/// stream ends with an error (`finish_sink` ended the others already).
pub fn abandon(d: &Daemon, main: &Job) {
    for b in main.branches.attached() {
        if !b.end.is_complete() {
            b.job.set_status('E');
            d.jmsg(&b.dir, &b.job, M_FATAL, &format!("The main job {} ended before it wrote this branch.\n", main.cmd.job));
            b.end.complete('E');
        }
    }
}
