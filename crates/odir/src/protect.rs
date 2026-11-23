// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The latest successful backup chain of every (client, FileSet name) is
//! never pruned or purged (decision of the user, 19.11.2026; deliberate
//! deviation from bareos-dir, see docs/wiki/betrieb-odir.md "Abweichungen
//! von Bareos").
//!
//! bareos-dir protects in PruneJobs only the chain of a job, FileSet and
//! client that are still configured (ua_prune.cc:1077-1114, JobSelectHandler
//! and AccurateGetJobids) and prunes the File records of that chain after
//! the File Retention. Here:
//!
//! 1. the chain is the last Full with JobStatus T/W, the last Differential
//!    after it (JobFiles > 0) and the Incrementals after both, like
//!    AccurateGetJobids with level Incremental (sql_get.cc), per (ClientId,
//!    FileSet name) of the catalog. The FileSet is compared by name over all
//!    its FileSetIds, also for the Full (AccurateGetJobids takes the Full of
//!    one FileSetId only). Whether job, FileSet or client are still
//!    configured does not matter. Base jobs of the chain are kept as well.
//! 2. Neither its Job nor its File records are purged (prune jobs, prune
//!    files, AutoPrune, purge jobs/files).
//! 3. A volume with JobMedia of such a job is neither pruned, purged,
//!    recycled, truncated nor deleted.
//! 4. When the chain cannot be determined (catalog error), nothing is
//!    purged (like J-11).

use crate::commands::bvfs::Db;

/// One protected chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    pub client_id: i64,
    pub client: String,
    pub fileset: String,
    /// Full, Differential, Incrementals (and base jobs), ascending.
    pub jobids: Vec<u32>,
}

/// The protected chains relevant for a set of jobs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Protected {
    pub chains: Vec<Chain>,
}

/// A backup job row of one (client, FileSet name).
#[derive(Debug, Clone, PartialEq)]
pub struct JobRow {
    pub jobid: u32,
    pub level: char,
    pub job_files: i64,
    pub start: String,
    pub end: String,
    pub job_tdate: i64,
    pub has_base: bool,
}

/// The chain of one (client, FileSet name) from its successful backup jobs
/// (all rows have JobStatus T/W and Type B): AccurateGetJobids level
/// Incremental. Times are compared as the catalog text
/// `YYYY-MM-DD HH:MM:SS`.
pub fn chain_of(rows: &[JobRow]) -> Vec<u32> {
    // Full: ORDER BY JobId DESC, JobTDate DESC LIMIT 1
    let Some(full) = rows.iter().filter(|r| r.level == 'F').max_by_key(|r| (r.jobid, r.job_tdate)) else {
        return Vec::new();
    };
    let mut chain = vec![full.jobid];
    let mut after = full.end.clone();
    // Differential: JobFiles > 0, StartTime > EndTime of the Full, ORDER BY JobTDate DESC LIMIT 1
    if let Some(diff) = rows.iter().filter(|r| r.level == 'D' && r.job_files > 0 && r.start > full.end)
        .max_by_key(|r| (r.job_tdate, r.jobid)) {
        chain.push(diff.jobid);
        if diff.end > after {
            after = diff.end.clone();
        }
    }
    // Incrementals after the latest EndTime of Full/Differential
    chain.extend(rows.iter().filter(|r| r.level == 'I' && r.start > after).map(|r| r.jobid));
    chain.sort_unstable();
    chain
}

fn id_list(ids: &[u32]) -> String {
    ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

fn text(row: &[Option<String>], i: usize) -> String {
    row.get(i).cloned().flatten().unwrap_or_default()
}

/// The protected chains of every (client, FileSet name) that one of `jobids`
/// belongs to. Err on a catalog error: then nothing may be purged.
pub fn for_jobs(db: &dyn Db, jobids: &[u32]) -> Result<Protected, String> {
    let jobids: Vec<u32> = jobids.iter().copied().filter(|id| *id != 0).collect();
    if jobids.is_empty() {
        return Ok(Protected::default());
    }
    let list = id_list(&jobids);
    let table = db.rows(&format!(
        "SELECT Job.ClientId, COALESCE(Client.Name, ''), FileSet.FileSet, Job.JobId, Job.Level, Job.JobFiles, \
         Job.StartTime, Job.EndTime, Job.JobTDate, Job.HasBase \
         FROM Job JOIN FileSet ON (FileSet.FileSetId = Job.FileSetId) LEFT JOIN Client ON (Client.ClientId = Job.ClientId) \
         WHERE Job.Type = 'B' AND Job.JobStatus IN ('T','W') AND Job.Level IN ('F','D','I') \
         AND (Job.ClientId, FileSet.FileSet) IN (SELECT DISTINCT J.ClientId, F.FileSet FROM Job J \
         JOIN FileSet F ON (F.FileSetId = J.FileSetId) WHERE J.JobId IN ({list})) \
         ORDER BY Job.ClientId, FileSet.FileSet, Job.JobId"))?;
    // group by (client, fileset name)
    let mut groups: Vec<((i64, String), String, Vec<JobRow>)> = Vec::new();
    for row in &table.rows {
        let key = (text(row, 0).parse::<i64>().unwrap_or(0), text(row, 2));
        let job = JobRow {
            jobid: text(row, 3).parse().unwrap_or(0),
            level: text(row, 4).chars().next().unwrap_or(' '),
            job_files: text(row, 5).parse().unwrap_or(0),
            start: text(row, 6),
            end: text(row, 7),
            job_tdate: text(row, 8).parse().unwrap_or(0),
            has_base: matches!(text(row, 9).as_str(), "1" | "t" | "true"),
        };
        match groups.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, _, rows)) => rows.push(job),
            None => groups.push((key, text(row, 1), vec![job])),
        }
    }
    let mut chains = Vec::new();
    for ((client_id, fileset), client, rows) in groups {
        let mut ids = chain_of(&rows);
        if ids.is_empty() {
            continue;
        }
        // GetUsedBaseJobids of the chain
        let with_base: Vec<u32> = rows.iter().filter(|r| r.has_base && ids.contains(&r.jobid)).map(|r| r.jobid).collect();
        if !with_base.is_empty() {
            let base = db.rows(&format!("SELECT DISTINCT BaseJobId FROM BaseFiles WHERE JobId IN ({})", id_list(&with_base)))?;
            ids.extend(base.rows.iter().filter_map(|r| text(r, 0).parse::<u32>().ok()).filter(|id| *id != 0));
            ids.sort_unstable();
            ids.dedup();
        }
        chains.push(Chain { client_id, client, fileset, jobids: ids });
    }
    Ok(Protected { chains })
}

/// The protected chains with jobs on volume `media_id` (only those jobs
/// listed). Err on a catalog error.
pub fn for_volume(db: &dyn Db, media_id: i64) -> Result<Protected, String> {
    let jobs: Vec<u32> = db.rows(&format!("SELECT DISTINCT JobId FROM JobMedia WHERE MediaId={media_id}"))?
        .rows.iter().filter_map(|r| text(r, 0).parse().ok()).collect();
    let mut protected = for_jobs(db, &jobs)?;
    for chain in &mut protected.chains {
        chain.jobids.retain(|id| jobs.contains(id));
    }
    protected.chains.retain(|c| !c.jobids.is_empty());
    Ok(protected)
}

impl Protected {
    pub fn is_empty(&self) -> bool {
        self.chains.iter().all(|c| c.jobids.is_empty())
    }

    pub fn contains(&self, jobid: u32) -> bool {
        self.chains.iter().any(|c| c.jobids.contains(&jobid))
    }

    /// Every protected JobId.
    pub fn jobids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.chains.iter().flat_map(|c| c.jobids.iter().copied()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Removes the protected jobs from `list`; one message per chain that
    /// had candidates: "Keeping the latest Full chain of client … FileSet …
    /// (JobIds …); not pruned."
    pub fn remove_from(&self, list: &mut Vec<u32>, what: &str) -> Vec<String> {
        let mut messages = Vec::new();
        for chain in &self.chains {
            let kept: Vec<u32> = list.iter().copied().filter(|id| chain.jobids.contains(id)).collect();
            if kept.is_empty() {
                continue;
            }
            messages.push(format!("Keeping the latest Full chain of client \"{}\" FileSet \"{}\" (JobIds {}); \
                its {what} are not pruned.\n", chain.client, chain.fileset, id_list(&kept)));
        }
        list.retain(|id| !self.contains(*id));
        messages
    }

    /// Why volume `name` is not pruned, purged or recycled.
    pub fn volume_message(&self, name: &str, action: &str) -> String {
        let chains: Vec<String> = self.chains.iter().filter(|c| !c.jobids.is_empty())
            .map(|c| format!("client \"{}\" FileSet \"{}\" (JobIds {})", c.client, c.fileset, id_list(&c.jobids)))
            .collect();
        format!("Volume \"{name}\" holds the latest Full chain of {}; not {action}.\n", chains.join(" and of "))
    }
}

/// Added to the volume message where a job looks for a volume.
pub const WAIT_NOTE: &str = "If no other volume is available, the job waits for a volume as with a full pool \
    (label or add a volume, or raise Maximum Volumes).\n";

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::catalog_worker::{SqlColumn, SqlTable};
    use std::sync::Mutex;

    /// One catalog job for the fake catalog.
    #[derive(Clone)]
    pub(crate) struct Job {
        pub id: u32,
        pub client_id: i64,
        pub client: &'static str,
        /// documents the FileSetId (the queries compare by name)
        #[allow(dead_code)]
        pub fileset_id: i64,
        pub fileset: &'static str,
        pub level: char,
        pub status: char,
        pub files: i64,
        /// day of month (start 10:00, end 11:00)
        pub day: u32,
        pub media: Vec<i64>,
    }

    pub(crate) fn job(id: u32, client_id: i64, fileset_id: i64, fileset: &'static str, level: char, day: u32) -> Job {
        Job { id, client_id, client: if client_id == 1 { "fd1" } else { "fd2" }, fileset_id, fileset, level,
              status: 'T', files: 5, day, media: vec![] }
    }

    /// A fake catalog that answers the queries of this module from a job
    /// list (and logs every statement); `fail` makes every query fail.
    pub(crate) struct Catalog {
        pub jobs: Vec<Job>,
        pub fail: bool,
        pub log: Mutex<Vec<String>>,
    }

    impl Catalog {
        pub(crate) fn new(jobs: Vec<Job>) -> Self {
            Self { jobs, fail: false, log: Mutex::new(Vec::new()) }
        }
    }

    fn ids_in(sql: &str, after: &str) -> Vec<u32> {
        let rest = &sql[sql.find(after).unwrap() + after.len()..];
        let inner = &rest[..rest.find(')').unwrap()];
        inner.split(',').filter_map(|s| s.trim().parse().ok()).collect()
    }

    fn table(rows: Vec<Vec<String>>) -> SqlTable {
        SqlTable { columns: vec![SqlColumn { name: "c".into(), numeric: false }],
                   rows: rows.into_iter().map(|r| r.into_iter().map(Some).collect()).collect() }
    }

    impl Db for Catalog {
        fn rows(&self, sql: &str) -> Result<SqlTable, String> {
            self.log.lock().unwrap().push(sql.to_string());
            if self.fail {
                return Err("ERROR:  connection lost".into());
            }
            if sql.contains("WHERE J.JobId IN (") {
                let ids = ids_in(sql, "WHERE J.JobId IN (");
                let pairs: Vec<(i64, &str)> = self.jobs.iter().filter(|j| ids.contains(&j.id)).map(|j| (j.client_id, j.fileset)).collect();
                let mut rows: Vec<&Job> = self.jobs.iter().filter(|j| pairs.contains(&(j.client_id, j.fileset))
                    && matches!(j.status, 'T' | 'W') && matches!(j.level, 'F' | 'D' | 'I')).collect();
                rows.sort_by_key(|j| (j.client_id, j.fileset, j.id));
                return Ok(table(rows.into_iter().map(|j| vec![j.client_id.to_string(), j.client.into(), j.fileset.into(),
                    j.id.to_string(), j.level.to_string(), j.files.to_string(), format!("2026-11-{:02} 10:00:00", j.day),
                    format!("2026-11-{:02} 11:00:00", j.day), (j.day as i64 * 86400).to_string(), "0".into()]).collect()));
            }
            if let Some(at) = sql.find("FROM JobMedia WHERE MediaId=") {
                let digits: String = sql[at + 28..].chars().take_while(char::is_ascii_digit).collect();
                let media: i64 = digits.parse().unwrap();
                let ids: Vec<Vec<String>> = self.jobs.iter().filter(|j| j.media.contains(&media)).map(|j| vec![j.id.to_string()]).collect();
                if sql.starts_with("SELECT 1 ") {
                    return Ok(table(ids.into_iter().take(1).map(|_| vec!["1".to_string()]).collect()));
                }
                return Ok(table(ids));
            }
            Ok(table(vec![]))
        }
        fn exec(&self, sql: &str) -> Result<u64, String> {
            self.log.lock().unwrap().push(sql.to_string());
            if self.fail { Err("ERROR:  connection lost".into()) } else { Ok(1) }
        }
    }

    fn row(jobid: u32, level: char, day: u32) -> JobRow {
        JobRow { jobid, level, job_files: 1, start: format!("2026-11-{day:02} 10:00:00"),
                 end: format!("2026-11-{day:02} 11:00:00"), job_tdate: day as i64, has_base: false }
    }

    #[test]
    fn chain_is_last_full_last_differential_and_incrementals_after() {
        let rows = vec![row(1, 'F', 1), row(2, 'I', 2), row(3, 'D', 3), row(4, 'I', 4), row(5, 'F', 5),
                        row(6, 'I', 6), row(7, 'D', 7), row(8, 'D', 8), row(9, 'I', 9), row(10, 'I', 10)];
        assert_eq!(chain_of(&rows), vec![5, 8, 9, 10]);
        // no Differential: all Incrementals after the Full
        assert_eq!(chain_of(&[row(1, 'F', 1), row(2, 'I', 2), row(3, 'I', 3)]), vec![1, 2, 3]);
        // a Differential without files does not count
        let mut empty = row(3, 'D', 3);
        empty.job_files = 0;
        assert_eq!(chain_of(&[row(1, 'F', 1), row(2, 'I', 2), empty, row(4, 'I', 4)]), vec![1, 2, 4]);
        // only Incrementals: no chain (nothing to protect)
        assert!(chain_of(&[row(2, 'I', 2)]).is_empty());
    }

    #[test]
    fn chains_per_client_and_fileset_name_over_all_filesetids() {
        // FileSet "fs" with two FileSetIds (changed include list): the Full of
        // FileSetId 11 and the Incremental of FileSetId 12 are one chain.
        let cat = Catalog::new(vec![job(1, 1, 11, "fs", 'F', 1), job(2, 1, 11, "fs", 'F', 3), job(3, 1, 12, "fs", 'I', 4),
                                    job(4, 2, 11, "fs", 'F', 2), job(5, 1, 13, "other", 'F', 1)]);
        let p = for_jobs(&cat, &[1]).unwrap();
        assert_eq!(p.chains, vec![Chain { client_id: 1, client: "fd1".into(), fileset: "fs".into(), jobids: vec![2, 3] }]);
        let mut list = vec![1, 2, 3];
        let messages = p.remove_from(&mut list, "jobs");
        assert_eq!(list, vec![1]);
        assert_eq!(messages, vec!["Keeping the latest Full chain of client \"fd1\" FileSet \"fs\" (JobIds 2,3); \
            its jobs are not pruned.\n".to_string()]);
        let p = for_jobs(&cat, &[1, 4, 5]).unwrap();
        assert_eq!(p.jobids(), vec![2, 3, 4, 5]);
        assert!(for_jobs(&cat, &[]).unwrap().is_empty());
        // a failed Full does not start a chain
        let mut failed = job(6, 1, 11, "fs", 'F', 5);
        failed.status = 'E';
        let cat = Catalog::new(vec![job(1, 1, 11, "fs", 'F', 1), failed]);
        assert_eq!(for_jobs(&cat, &[1, 6]).unwrap().jobids(), vec![1]);
    }

    #[test]
    fn volumes_and_catalog_errors() {
        let mut full = job(2, 1, 11, "fs", 'F', 3);
        full.media = vec![7];
        let mut old = job(1, 1, 11, "fs", 'F', 1);
        old.media = vec![7, 8];
        let cat = Catalog::new(vec![old, full]);
        let p = for_volume(&cat, 7).unwrap();
        assert_eq!(p.jobids(), vec![2]);
        assert_eq!(p.volume_message("Full-0007", "pruned"),
            "Volume \"Full-0007\" holds the latest Full chain of client \"fd1\" FileSet \"fs\" (JobIds 2); not pruned.\n");
        assert!(for_volume(&cat, 8).unwrap().is_empty());
        let mut cat = cat;
        cat.fail = true;
        assert!(for_jobs(&cat, &[1]).is_err());
        assert!(for_volume(&cat, 7).is_err());
    }
}
