// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Zweige (Branch): ein Haupt-Job und seine Zweig-Jobs aus einem FD-Lauf.
//!
//! Zweck: 3-2-1 im laufenden Betrieb (Plan `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`
//! „Zweige“, Entwurf `docs/plans/P2B-ZWEIGE-ENTWURF.md`). Steht im Overlay
//! `<overlay>/odir.d/job/<Haupt-Job>.conf` ein `Branch = <Job>` (mehrfach), liest der
//! FD die Daten einmal und osd schreibt sie für den Haupt-Job und für jeden Zweig. Dieses
//! Modul liegt im Director:
//!
//! - [`combine_since`]: Level und Basis des gemeinsamen Streams (D-10).
//! - [`prepare`]: legt für jeden Zweig den `Job`-Satz an (eigene JobId, eigener Pool,
//!   eigenes Storage), die Zuordnung in `onesimus.job_branch` und die Worker-Parameter.
//! - [`Prepared::enqueue`]: stellt die Zweig-Jobs in die Warteschlange des `JobManager`;
//!   sie zählen nicht gegen die Grenzen von Client und Director (sie gehören zum
//!   laufenden Haupt-Job).
//!
//! Der Worker eines Zweigs ist `backup_worker::run_branch`; die Daten gehen über osd
//! (`crates/osd/daemon/src/branch.rs`), die Katalogsätze (`File`, `JobMedia`, `Job`)
//! schreibt der Director je Zweig-JobId wie bei jedem Backup-Job.
//!
//! Bareos-Gegenstück: keins (Onesimus-Erweiterung); nur odir und osd über gRPC.

use bareos_config::resources::JobResource;

use crate::backup::{LevelPolicy, LevelSince};
use crate::director_config::DirectorConfig;
use crate::job_worker::{JobParams, WorkerJobType};

/// The level and the basis of the stream the FD reads for a main job and its
/// branches (D-10).
///
/// - A Full for one of them is a Full stream for all (a branch with no Full of its own
///   gets one, and the main job with it).
/// - Otherwise the stream has the highest level of the two kinds (Differential over
///   Incremental) and the **oldest** basis, so that a branch that missed a run gets
///   everything since its own last backup; the others store a little more than needed.
///
/// `main` is the result of the main job's own `get_level_since_time`, `branches` those of
/// the branch jobs (name and result), all for the level the run asked for.
pub(crate) fn combine_since(main: LevelSince, branches: &[(String, LevelSince)], requested: char) -> LevelSince {
    let mut all: Vec<&LevelSince> = std::iter::once(&main).chain(branches.iter().map(|(_, s)| s)).collect();
    all.retain(|s| s.level != 'V');
    if all.iter().any(|s| s.level == 'F') {
        if main.level == 'F' {
            return main;
        }
        let mut messages = main.messages.clone();
        for (name, _) in branches.iter().filter(|(_, s)| s.level == 'F') {
            messages.push(format!("Branch job {name} needs a Full backup: the stream is Full for all jobs.\n"));
        }
        return LevelSince {
            level: 'F',
            since: if requested == 'F' { String::new() } else { format!(" (upgraded from {})", crate::backup::level_to_string(requested)) },
            stime: String::new(),
            prev_job: String::new(),
            pool_updated: requested != 'F',
            messages,
        };
    }
    let oldest = all
        .iter()
        .filter(|s| !s.stime.is_empty())
        .min_by_key(|s| crate::btime::str_to_utime(&s.stime))
        .copied()
        .unwrap_or(&main);
    let mut out = main.clone();
    if all.iter().any(|s| s.level == 'D') {
        out.level = 'D';
    }
    if oldest.stime != main.stime {
        let who = branches.iter().find(|(_, s)| s.stime == oldest.stime).map(|(n, _)| n.as_str()).unwrap_or("a branch job");
        out.messages.push(format!(
            "Branch job {who} has an older basis ({}): the stream starts there for all jobs.\n", oldest.stime
        ));
        out.stime = oldest.stime.clone();
        out.prev_job = oldest.prev_job.clone();
        out.since = format!(", since={}", oldest.stime);
    }
    out
}

/// A branch job ready to start: its Job record exists, the worker parameters are
/// built.
pub(crate) struct Prepared {
    /// `JobId` of the branch job.
    pub job_id: u32,
    /// Unique job name (`Job=`) of the branch job.
    pub unique_job: String,
    /// Name of the Job resource.
    pub job_name: String,
    /// What the job queue needs to start it.
    pub start: crate::job_manager::QueuedStart,
    /// Storage resource name.
    pub storage: String,
    /// The end-of-job report data.
    pub report: crate::job_messages::JobReport,
}

/// What [`prepare`] needs to know about the main job's run.
pub(crate) struct MainRun<'a> {
    /// The main job's Job resource.
    pub job: &'a JobResource,
    /// The main job's unique job name.
    pub unique_job: &'a str,
    /// The main job's JobId.
    pub job_id: u32,
    /// The client the FD of the stream belongs to.
    pub client: &'a bareos_config::resources::ClientResource,
    /// `ClientId` of the client.
    pub client_id: i64,
    /// `FileSetId` and name of the main job's FileSet and its MD5.
    pub fileset_id: i64,
    /// The FileSet name.
    pub fileset_name: &'a str,
    /// The FileSet's MD5 of the Job command.
    pub fileset_md5: &'a str,
    /// The level of the stream ([`combine_since`]).
    pub since: &'a LevelSince,
    /// Scheduled time of the run.
    pub sched_time: i64,
    /// Now.
    pub now: i64,
    /// The main job's priority (the branches run with it).
    pub priority: i32,
}

/// Creates the branch jobs of `run.job`: the `Job` record (own JobId), the pool record,
/// the pair in `onesimus.job_branch` and the worker parameters. The branches take the
/// client and FileSet of the main job (the FD reads once); pool, storage, retention and
/// messages are those of the branch job's own resource.
pub(crate) fn prepare(
    ctx: &crate::command_executor::ExecContext,
    config: &DirectorConfig,
    run: &MainRun<'_>,
) -> Result<Vec<Prepared>, String> {
    let Some(cat) = ctx.catalog.as_ref() else { return Err("no catalog".into()) };
    let mut out = Vec::new();
    for name in config.branches_of(&run.job.name) {
        let branch = config.find_job(&name).ok_or_else(|| format!("Branch job \"{name}\" not found"))?;
        let level = run.since.level;
        let level_text = level.to_string();
        let unique = crate::commands::job::unique_job_name(&branch.name, run.now);
        let job_id = crate::commands::job::create_job_record(ctx, &unique, &branch.name, 'B', level, Some(&run.client.name),
            run.sched_time, "").map_err(|e| format!("Branch job \"{name}\": {e}"))?;
        // the pair: Onesimus table, never a field of the Job record
        let pair = format!("INSERT INTO onesimus.job_branch (branch_job_id, main_job_id) VALUES ({job_id}, {})", run.job_id);
        if let Err(e) = cat.execute(&pair) {
            tracing::warn!("Branch job {job_id} of {}: cannot record the pair in onesimus.job_branch: {e}", run.job_id);
        }
        // ApplyPoolOverrides() of the branch's own resource for the stream's level
        let default_pool = match level {
            'F' if !branch.full_backup_pool.is_empty() => &branch.full_backup_pool,
            'D' if !branch.differential_backup_pool.is_empty() => &branch.differential_backup_pool,
            'I' if !branch.incremental_backup_pool.is_empty() => &branch.incremental_backup_pool,
            _ => &branch.pool,
        };
        let pool = default_pool.as_str();
        let mut messages = Vec::new();
        let pool_id = crate::commands::pool_db::get_or_create_pool_record(config, cat, pool, crate::backup::POOL_CREATED,
            crate::backup::POOL_NOT_IN_DB, &mut messages).map_err(|e| format!("Branch job \"{name}\": {e}"))?;
        let storage_name = branch.storage.split(',').next().unwrap_or("").trim().to_string();
        let storage = config.find_storage(&storage_name)
            .ok_or_else(|| format!("Branch job \"{name}\": Storage \"{storage_name}\" not found"))?;
        let job_dirs = crate::commands::defaults::job_directives(config, &branch.name);
        let catalog_files = crate::job_messages::raw_directive(config, "pool", pool, "catalogfiles")
            .and_then(|v| bareos_config::values::parse_bool(&v).ok().map(|(_, on)| on)).unwrap_or(true);
        let write = crate::sd_copy::StorageUse {
            name: storage.name.clone(), address: storage.address.clone(), port: storage.port,
            password: storage.password.clone(), media_type: storage.media_type.clone(),
            devices: storage.device.split(',').map(|d| d.trim().to_string()).filter(|d| !d.is_empty()).collect(),
            pool: pool.to_string(),
            pool_type: config.find_pool(pool).map(|p| p.pool_type.clone()).unwrap_or_else(|| "Backup".into()),
            grpc: crate::sd_grpc::for_storage(config, storage),
        };
        let director_dirs: Vec<bareos_config::raw::Directive> = config.raw.iter().find(|r| r.norm_type() == "director")
            .map(|r| r.directives.clone()).unwrap_or_default();
        let sd_connect_timeout = if director_dirs.iter().any(|d| d.norm_key() == "sdconnecttimeout") {
            crate::commands::defaults::time_directive(&director_dirs, "sdconnecttimeout")
        } else {
            1800
        };
        let session = crate::backup_worker::BackupSession {
            ids: crate::backup::JobIds { level, client_id: run.client_id, pool_id, fileset_id: run.fileset_id },
            since_utime: crate::btime::str_to_utime(&run.since.stime),
            prev_job: run.since.prev_job.clone(),
            sd_job: crate::sd_copy::CopySession {
                job_type: 'B', level, job_resource: branch.name.clone(), client: run.client.name.clone(),
                fileset: run.fileset_name.to_string(), fileset_md5: run.fileset_md5.to_string(),
                no_attributes: !catalog_files,
                prefer_mounted_volumes: crate::commands::defaults::bool_directive(&job_dirs, "prefermountedvolumes", true),
                ..Default::default()
            },
            write,
            branch_of: Some(run.unique_job.to_string()),
            sd_connect_timeout,
            ..Default::default()
        };
        let params = JobParams {
            job_id,
            unique_job: unique.clone(),
            job_name: branch.name.clone(),
            job_type: WorkerJobType::Backup,
            level: level_text.clone(),
            fd_address: String::new(),
            fd_port: 0,
            fd_password: String::new(),
            fd_grpc: None,
            sd_grpc: crate::sd_grpc::for_storage(config, storage),
            sd_address: storage.address.clone(),
            sd_port: storage.port,
            sd_password: storage.password.clone(),
            sd_device: storage.device.clone(),
            sd_media_type: storage.media_type.clone(),
            director_name: config.director.name.clone(),
            pool_name: pool.to_string(),
            fileset_name: run.fileset_name.to_string(),
            fileset_includes: Vec::new(),
            fileset_excludes: Vec::new(),
            since_utime: crate::btime::str_to_utime(&run.since.stime),
            prev_job: run.since.prev_job.clone(),
            where_path: None,
            regex_where: None,
            prefix_links: false,
            replace: None,
            restore_jobid: None,
            bootstrap: if branch.write_bootstrap.is_empty() { None } else { Some(branch.write_bootstrap.clone()) },
            client_name: run.client.name.clone(),
            client_first_connection: false,
            passive_client: false,
            copy: None,
            backup: Some(session),
            verify: None,
            plugin_options: Vec::new(),
        };
        let mut report = crate::job_messages::backup_report(config, branch, run.client, &storage.name, pool, default_pool,
            &level_text, run.fileset_name, run.now);
        report.sched_time = run.sched_time;
        report.job = unique.clone();
        report.level = crate::backup::level_to_string(level).to_string();
        report.since = run.since.since.clone();
        out.push(Prepared {
            job_id,
            unique_job: unique,
            job_name: branch.name.clone(),
            start: crate::job_manager::QueuedStart { params, priority: run.priority, sched_time: run.sched_time },
            storage: storage.name.clone(),
            report,
        });
    }
    Ok(out)
}

impl Prepared {
    /// Puts the branch job into the job queue (`client` empty: a branch does not read the
    /// client, so the client's `Maximum Concurrent Jobs` does not count it). Without a
    /// job manager (tests) the worker is forked at once.
    pub(crate) fn enqueue(self, ctx: &crate::command_executor::ExecContext) -> Result<(), String> {
        if let Some(smgr) = &ctx.session_mgr {
            smgr.bus().post(crate::message_bus::BusMessage::JobStatus {
                job_id: self.job_id, job_name: self.job_name.clone(), status: "Queued".to_string(),
                timestamp: crate::job_worker::now_unix(), sched_time: 0,
            });
        }
        match &ctx.job_manager {
            Some(mgr) => {
                let mut mgr = mgr.lock().unwrap_or_else(|e| e.into_inner());
                mgr.enqueue_job_with(self.job_id, &self.job_name, self.start, "", &self.storage, crate::scheduled_run::start_flags());
                mgr.set_job_report(self.job_id, self.report);
                Ok(())
            }
            None => crate::job_worker::spawn_job_worker(self.job_id, &self.job_name, &self.start.params).map(|_| ())
                .map_err(|e| format!("Failed to start branch job {}: {e}", self.job_name)),
        }
    }
}

/// The level policy of a branch job (its own `Max Full Interval` and so on), for the
/// branch's own `get_level_since_time`.
pub(crate) fn policy_of(config: &DirectorConfig, job: &str) -> LevelPolicy {
    LevelPolicy::of_job(&crate::commands::defaults::job_directives(config, job))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A since result of level `level` with basis `stime` (job `prev`).
    fn since(level: char, stime: &str, prev: &str) -> LevelSince {
        LevelSince { level, since: format!(", since={stime}"), stime: stime.into(), prev_job: prev.into(), ..Default::default() }
    }

    #[test]
    fn the_stream_takes_the_oldest_basis_of_the_jobs() {
        let main = since('I', "2026-11-23 10:00:00", "Main.1");
        let branches = [("Ai".to_string(), since('I', "2026-11-22 10:00:00", "Ai.1")), ("Off".to_string(), since('I', "2026-11-23 09:00:00", "Off.1"))];
        let out = combine_since(main, &branches, 'I');
        assert_eq!((out.level, out.stime.as_str(), out.prev_job.as_str()), ('I', "2026-11-22 10:00:00", "Ai.1"));
        assert_eq!(out.since, ", since=2026-11-22 10:00:00");
        assert!(out.messages.iter().any(|m| m.contains("Branch job Ai has an older basis")), "{:?}", out.messages);
    }

    #[test]
    fn the_main_basis_stays_when_it_is_the_oldest_and_a_differential_wins() {
        let main = since('I', "2026-11-20 10:00:00", "Main.1");
        let out = combine_since(main.clone(), &[("Ai".to_string(), since('I', "2026-11-23 10:00:00", "Ai.1"))], 'I');
        assert_eq!(out, main);
        let out = combine_since(main, &[("Ai".to_string(), since('D', "2026-11-23 10:00:00", "Ai.1"))], 'I');
        assert_eq!((out.level, out.stime.as_str()), ('D', "2026-11-20 10:00:00"));
    }

    #[test]
    fn a_full_for_one_job_is_a_full_stream_for_all() {
        let main = since('I', "2026-11-23 10:00:00", "Main.1");
        let out = combine_since(main, &[("New".to_string(), LevelSince { level: 'F', ..Default::default() })], 'I');
        assert_eq!((out.level, out.stime.as_str(), out.pool_updated), ('F', "", true));
        assert_eq!(out.since, " (upgraded from Incremental)");
        assert!(out.messages.iter().any(|m| m.contains("Branch job New needs a Full backup")), "{:?}", out.messages);
        // a Full run stays a Full run without remarks
        let full = LevelSince { level: 'F', ..Default::default() };
        let out = combine_since(full.clone(), &[("New".to_string(), full.clone())], 'F');
        assert_eq!(out, full);
        // the main job was upgraded itself: its own messages stay, nothing is added
        let upgraded = LevelSince { level: 'F', since: " (upgraded from Incremental)".into(), pool_updated: true,
            messages: vec!["No prior or suitable Full backup found in catalog. Doing FULL backup.\n".into()], ..Default::default() };
        let out = combine_since(upgraded.clone(), &[("Ai".to_string(), since('I', "2026-11-23 10:00:00", "Ai.1"))], 'I');
        assert_eq!(out.level, 'F');
        assert_eq!(out.messages, upgraded.messages);
    }
}
