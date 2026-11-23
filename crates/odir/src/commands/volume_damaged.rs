// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Onesimus extension, not in Bareos: the flow for a damaged Volume (tape tear,
//! disk failure, a broken file Volume), docs/plans/OSD-GERAETE-UND-VERTEILUNG.md
//! D-19, backlog V-1.
//!
//! ```text
//! .volume_damaged volume=<name> [dryrun | yes] [full]
//! ```
//!
//! Without `yes` (or with `dryrun`) the command only reports:
//!
//! * the Volume and its pool,
//! * the jobs with JobMedia on it (a job over several Volumes is lost as a
//!   whole, the other Volumes are named),
//! * for each job whether a usable copy exists: a job of type Backup or Copy
//!   whose `PriorJobId` links it to the same original, that does not lie on
//!   the damaged Volume and not on a Volume with status Error,
//! * the jobs without a copy and the `run … level=Full` that would make a new
//!   backup of them,
//! * the later Incremental and Differential jobs of a damaged Full (their
//!   restore chain is broken).
//!
//! With `yes` it
//!
//! 1. sets the Volume to `VolStatus = Error`,
//! 2. purges the damaged jobs that have a usable copy; the catalog function
//!    behind `purge jobid=…` upgrades the oldest copy to a Backup job
//!    (`UpgradeCopies`, cats/sql_update.cc: the copy takes over, the job
//!    record of the damaged original goes),
//! 3. with `full`, starts a Full of every job that has no copy.
//!
//! Jobs without a copy stay in the catalog: what can still be read of them
//! (`bls`, `bextract`) is up to the operator.

use crate::command_ast::{Argument, CommandAst};
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::commands::bvfs::Db;
use crate::ua_select::Ua;

/// A job with JobMedia on the damaged Volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Damaged {
    /// JobId.
    pub id: u32,
    /// Name of the Job resource.
    pub name: String,
    /// `B` backup, `C` copy, `A` archive, …
    pub kind: String,
    /// `F`, `I`, `D`, …
    pub level: String,
    /// Client name.
    pub client: String,
    /// Start time as text.
    pub started: String,
    /// JobBytes.
    pub bytes: String,
    /// Other Volumes of the job (the job is lost as a whole).
    pub other_volumes: Vec<String>,
    /// JobIds of usable copies (or of the original when this job is a copy).
    pub copies: Vec<u32>,
}

/// SQL string literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// `jobids` in SQL (`1,2,3`).
fn id_list(jobs: &[Damaged]) -> String {
    jobs.iter().map(|j| j.id.to_string()).collect::<Vec<_>>().join(",")
}

/// The jobs on `media_id` with their other Volumes and usable copies.
pub(crate) fn damaged_jobs(db: &dyn Db, media_id: &str) -> Result<Vec<Damaged>, String> {
    let rows = db.rows(&format!(
        "SELECT DISTINCT j.JobId, j.Name, j.Type, j.Level, c.Name, COALESCE(j.StartTime::text, ''), j.JobBytes \
         FROM JobMedia jm JOIN Job j ON j.JobId = jm.JobId LEFT JOIN Client c ON c.ClientId = j.ClientId \
         WHERE jm.MediaId = {media_id} ORDER BY j.JobId"
    ))?;
    let cell = |row: &Vec<Option<String>>, i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let mut jobs: Vec<Damaged> = rows
        .rows
        .iter()
        .map(|r| Damaged {
            id: cell(r, 0).parse().unwrap_or(0),
            name: cell(r, 1),
            kind: cell(r, 2),
            level: cell(r, 3),
            client: cell(r, 4),
            started: cell(r, 5),
            bytes: cell(r, 6),
            other_volumes: Vec::new(),
            copies: Vec::new(),
        })
        .filter(|j| j.id != 0)
        .collect();
    if jobs.is_empty() {
        return Ok(jobs);
    }
    let ids = id_list(&jobs);
    let others = db.rows(&format!(
        "SELECT DISTINCT jm.JobId, m.VolumeName FROM JobMedia jm JOIN Media m ON m.MediaId = jm.MediaId \
         WHERE jm.JobId IN ({ids}) AND jm.MediaId <> {media_id} ORDER BY 1, 2"
    ))?;
    for r in &others.rows {
        let (id, volume) = (cell(r, 0).parse::<u32>().unwrap_or(0), cell(r, 1));
        if let Some(j) = jobs.iter_mut().find(|j| j.id == id) {
            j.other_volumes.push(volume);
        }
    }
    // a copy is a finished Backup/Copy job of the same original that lies on no damaged Volume and no Volume with status Error
    let copies = db.rows(&format!(
        "SELECT DISTINCT j.JobId, c.JobId FROM Job j JOIN Job c \
           ON (c.PriorJobId = j.JobId OR (j.PriorJobId <> 0 AND (c.JobId = j.PriorJobId OR c.PriorJobId = j.PriorJobId))) \
         WHERE j.JobId IN ({ids}) AND c.JobId <> j.JobId AND c.Type IN ('B','C') AND c.JobStatus IN ('T','W') \
           AND EXISTS (SELECT 1 FROM JobMedia x WHERE x.JobId = c.JobId) \
           AND NOT EXISTS (SELECT 1 FROM JobMedia x JOIN Media mx ON mx.MediaId = x.MediaId \
                            WHERE x.JobId = c.JobId AND (x.MediaId = {media_id} OR mx.VolStatus = 'Error')) \
         ORDER BY 1, 2"
    ))?;
    for r in &copies.rows {
        let (id, copy) = (cell(r, 0).parse::<u32>().unwrap_or(0), cell(r, 1).parse::<u32>().unwrap_or(0));
        if let Some(j) = jobs.iter_mut().find(|j| j.id == id) {
            if !j.copies.contains(&copy) {
                j.copies.push(copy);
            }
        }
    }
    Ok(jobs)
}

/// Later Incremental/Differential jobs of the same job resource and client that depend on the damaged Full `job`.
fn dependents(db: &dyn Db, job: &Damaged) -> Vec<u32> {
    let sql = format!(
        "SELECT j.JobId FROM Job j, Job f WHERE f.JobId = {} AND j.Name = f.Name AND j.ClientId = f.ClientId \
         AND j.FileSetId = f.FileSetId AND j.Level IN ('I','D') AND j.JobId > f.JobId AND j.Type = 'B' \
         AND j.JobStatus IN ('T','W') \
         AND NOT EXISTS (SELECT 1 FROM Job n WHERE n.Name = f.Name AND n.ClientId = f.ClientId AND n.FileSetId = f.FileSetId \
                          AND n.Level = 'F' AND n.Type = 'B' AND n.JobStatus IN ('T','W') AND n.JobId > f.JobId AND n.JobId < j.JobId) \
         ORDER BY j.JobId",
        job.id
    );
    db.rows(&sql)
        .map(|t| t.rows.iter().filter_map(|r| r.first().cloned().flatten().and_then(|v| v.parse().ok())).collect())
        .unwrap_or_default()
}

/// The arguments `.volume_damaged` accepts.
const ARGUMENTS: [&str; 5] = ["volume", "dryrun", "yes", "full", "catalog"];

/// Parsed arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// `volume=`.
    pub volume: String,
    /// Report only (no `yes`, or `dryrun`).
    pub dryrun: bool,
    /// `full`: start a Full of the jobs without a copy.
    pub full: bool,
}

/// Parses the arguments; the error is the message.
pub(crate) fn parse(args: &[Argument]) -> Result<Request, String> {
    let mut request = Request { volume: String::new(), dryrun: true, full: false };
    let (mut yes, mut dryrun) = (false, false);
    for a in args {
        let key = a.key.to_ascii_lowercase();
        if !ARGUMENTS.contains(&key.as_str()) {
            return Err(format!("unknown argument \"{}\"\n", a.key));
        }
        match key.as_str() {
            "volume" => request.volume = a.value.clone().unwrap_or_default(),
            "yes" => yes = true,
            "dryrun" => dryrun = true,
            "full" => request.full = true,
            _ => {}
        }
    }
    if request.volume.is_empty() {
        return Err("usage: .volume_damaged volume=<name> [dryrun | yes] [full]\n".into());
    }
    request.dryrun = dryrun || !yes;
    Ok(request)
}

/// `.volume_damaged`.
pub fn cmd_volume_damaged(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = Ua::new(ctx);
    let request = match parse(&ast.arguments) {
        Ok(r) => r,
        Err(e) => {
            ua.error_msg(e);
            return ua.finish(false);
        }
    };
    let Some(db) = ctx.catalog.as_ref().map(|c| c.dedicated()) else {
        ua.send_msg("No catalog configured\n");
        return ua.finish(false);
    };
    let db = match db {
        Ok(db) => db,
        Err(e) => {
            ua.error_msg(format!("{e}\n"));
            return ua.finish(false);
        }
    };
    let ok = run(&mut ua, ast, &db, &request);
    ua.finish(ok)
}

/// The report and, with `yes`, the actions.
fn run(ua: &mut Ua, ast: &CommandAst, db: &dyn Db, request: &Request) -> bool {
    let media = match db.rows(&format!(
        "SELECT m.MediaId, m.VolStatus, COALESCE(p.Name, '') FROM Media m LEFT JOIN Pool p ON p.PoolId = m.PoolId WHERE m.VolumeName = {}",
        quote(&request.volume)
    )) {
        Ok(t) => t,
        Err(e) => {
            ua.error_msg(format!("{e}\n"));
            return false;
        }
    };
    let Some(row) = media.rows.first() else {
        ua.error_msg(format!("Volume \"{}\" not found in the catalog.\n", request.volume));
        return false;
    };
    let cell = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let (media_id, status, pool) = (cell(0), cell(1), cell(2));
    if !pool.is_empty() && !crate::acl::access(ua.ctx, crate::acl::Kind::Pool, &pool) {
        ua.error_msg(format!("Access to pool \"{pool}\" is denied.\n"));
        return false;
    }
    let jobs = match damaged_jobs(db, &media_id) {
        Ok(j) => j,
        Err(e) => {
            ua.error_msg(format!("{e}\n"));
            return false;
        }
    };
    ua.send_msg(format!(
        "Volume \"{}\" (MediaId {media_id}, pool \"{pool}\", status {status}): {} job(s) with data on it{}\n",
        request.volume,
        jobs.len(),
        if request.dryrun { " [dry run: nothing is changed, add yes]" } else { "" }
    ));
    let mut with_copy = Vec::new();
    let mut without_copy = Vec::new();
    for j in &jobs {
        let denied = !crate::acl::access(ua.ctx, crate::acl::Kind::Job, &j.name) || !crate::acl::access(ua.ctx, crate::acl::Kind::Client, &j.client);
        let state = if denied {
            "access denied (job or client ACL)".to_string()
        } else if j.copies.is_empty() {
            "no usable copy".to_string()
        } else {
            format!("copy: JobId {}", j.copies.iter().map(u32::to_string).collect::<Vec<_>>().join(","))
        };
        let spread = if j.other_volumes.is_empty() { String::new() } else { format!(", also on {}", j.other_volumes.join(", ")) };
        ua.send_msg(format!(
            "  JobId {:>6} {:<24} client {:<14} {} level {} type {} {} bytes{spread} — {state}\n",
            j.id, j.name, j.client, j.started, j.level, j.kind, j.bytes
        ));
        if !denied {
            if j.copies.is_empty() { without_copy.push(j) } else { with_copy.push(j) }
        }
        if j.level == "F" && j.kind == "B" {
            let later = dependents(db, j);
            if !later.is_empty() {
                ua.send_msg(format!("    later Incremental/Differential jobs depend on it: {}\n", later.iter().map(u32::to_string).collect::<Vec<_>>().join(",")));
            }
        }
    }
    if !without_copy.is_empty() {
        let mut names: Vec<&str> = without_copy.iter().filter(|j| j.kind == "B").map(|j| j.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        ua.send_msg("Jobs without a copy lose their data. New Full backups:\n".to_string());
        for n in &names {
            ua.send_msg(format!("  run job={n} level=Full yes\n"));
        }
    }
    if request.dryrun {
        return true;
    }
    // 1. the Volume is Error
    if let Err(e) = db.exec(&format!("UPDATE Media SET VolStatus = 'Error' WHERE MediaId = {media_id}")) {
        ua.error_msg(format!("{e}\n"));
        return false;
    }
    ua.send_msg(format!("Volume \"{}\" is now in status Error.\n", request.volume));
    // 2. the damaged jobs with a copy go; the oldest copy becomes the Backup job
    if !with_copy.is_empty() {
        let ids = with_copy.iter().map(|j| j.id.to_string()).collect::<Vec<_>>().join(",");
        match crate::commands::purge::purge_jobs(db, &ids) {
            Ok(()) => ua.send_msg(format!("Purged the damaged jobs {ids}; their copies are Backup jobs now.\n")),
            Err(e) => {
                ua.error_msg(format!("{e}\n"));
                return false;
            }
        }
    }
    // 3. new Fulls
    if request.full {
        let mut names: Vec<String> = without_copy.iter().filter(|j| j.kind == "B").map(|j| j.name.clone()).collect();
        names.sort_unstable();
        names.dedup();
        for name in names {
            let run = CommandAst {
                command_name: "run".into(),
                arguments: vec![
                    Argument { key: "job".into(), value: Some(name.clone()) },
                    Argument { key: "level".into(), value: Some("Full".into()) },
                    Argument { key: "yes".into(), value: None },
                ],
                raw_line: String::new(),
                ..ast.clone()
            };
            let result = crate::commands::job::cmd_run(&run, ua.ctx);
            ua.send_msg(format!("run job={name} level=Full: {}\n", result.text_output.trim_end()));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<Argument> {
        line.split(' ').map(|t| match t.split_once('=') {
            Some((k, v)) => Argument { key: k.into(), value: Some(v.into()) },
            None => Argument { key: t.into(), value: None },
        }).collect()
    }

    #[test]
    fn a_report_is_the_default_and_yes_acts() {
        let r = parse(&args("volume=Vol-0001")).unwrap();
        assert_eq!((r.volume.as_str(), r.dryrun, r.full), ("Vol-0001", true, false));
        assert!(!parse(&args("volume=V yes")).unwrap().dryrun);
        assert!(parse(&args("volume=V yes dryrun")).unwrap().dryrun, "dryrun wins");
        assert!(parse(&args("volume=V yes full")).unwrap().full);
    }

    #[test]
    fn the_volume_is_required_and_unknown_arguments_are_refused() {
        assert!(parse(&args("yes")).unwrap_err().starts_with("usage:"));
        assert!(parse(&args("volume=V purge")).unwrap_err().contains("unknown argument \"purge\""));
    }
}
