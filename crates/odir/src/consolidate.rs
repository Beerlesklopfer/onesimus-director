//! ConsolidateJobs() (dird/consolidate.cc), Compatibility log C-058, S-06:
//! for every Always Incremental Job resource the Consolidate job selects the
//! jobs to consolidate (AlwaysIncrementalJobRetention, KeepNumber,
//! MaxFullAge, MaxFullConsolidations), purges empty jobs and starts a
//! VirtualFull job with the selected JobIds (StartNewConsolidationJob).

use crate::catalog_worker::CatalogHandle;
use crate::commands::defaults::{bool_directive, job_directives, pint32, time_directive};
use crate::director_config::DirectorConfig;
use crate::internal_jobs::{Outcome, Request};
use crate::message_bus::MsgType;

fn sql_esc(text: &str) -> String {
    text.replace('\'', "''")
}

/// The Consolidate job's own settings (jcr of the consolidate job).
struct Control {
    max_full_consolidations: u32,
    priority: i32,
    accurate: bool,
    spool_data: bool,
}

/// ConsolidateJobs(): false when the job fails.
pub fn consolidate_jobs(config: &DirectorConfig, catalog: &CatalogHandle, request: &Request, out: &mut Outcome) -> bool {
    let own = job_directives(config, request.job_name);
    let control = Control {
        max_full_consolidations: pint32(&own, "maxfullconsolidations", 0).unwrap_or(0),
        priority: config.find_job(request.job_name).map(|j| j.priority).unwrap_or(10),
        accurate: bool_directive(&own, "accurate", false),
        spool_data: bool_directive(&own, "spooldata", false),
    };
    let now = (unsafe { libc::time(std::ptr::null_mut()) }) as i64;
    let mut full_consolidations_started = 0u32;
    let info = |out: &mut Outcome, text: String| out.messages.push((MsgType::Info, text));

    for job in &config.jobs {
        let dirs = job_directives(config, &job.name);
        if !bool_directive(&dirs, "alwaysincremental", false) {
            continue;
        }
        let name = &job.name;
        info(out, format!("Looking at always incremental job {name}\n"));

        // GetOrCreateFilesetRecord / GetOrCreateClientRecord
        let fileset_id: i64 = catalog.query_rows(&format!(
            "SELECT FileSetId FROM FileSet WHERE FileSet='{}' ORDER BY CreateTime DESC, FileSetId DESC LIMIT 1",
            sql_esc(&job.fileset))).ok()
            .and_then(|t| t.rows.into_iter().next()).and_then(|r| r.into_iter().next().flatten())
            .and_then(|v| v.parse().ok()).unwrap_or(0);
        let client_id: i64 = match catalog.query_rows(&format!("SELECT ClientId FROM Client WHERE Name='{}'",
            sql_esc(&job.client))).ok().and_then(|t| t.rows.into_iter().next()).and_then(|r| r.into_iter().next().flatten()) {
            Some(id) => id.parse().unwrap_or(0),
            None => match catalog.query_rows(&format!(
                "INSERT INTO Client (Name,Uname,AutoPrune,FileRetention,JobRetention) \
                 VALUES ('{}','',0,5184000,15552000) RETURNING ClientId", sql_esc(&job.client))).ok()
                .and_then(|t| t.rows.into_iter().next()).and_then(|r| r.into_iter().next().flatten()) {
                Some(id) => id.parse().unwrap_or(0),
                None => {
                    out.messages.push((MsgType::Fatal, format!("JobId={} no ClientId\n", request.job_id)));
                    return false;
                }
            },
        };

        let jobids = |start: i64, limit: u64| crate::vbackup::accurate_jobids(catalog, request.job_id, client_id,
            fileset_id, start, limit);
        // First determine the number of total incrementals
        let all_jobids = jobids(0, 0);

        let retention = time_directive(&dirs, "alwaysincrementaljobretention") as i64;
        let mut start_time = 0;
        if retention != 0 {
            start_time = now - retention;
            info(out, format!("{name}: considering jobs older than {} for consolidation.\n",
                crate::job_messages::bstrftimes(start_time)));
        }
        let mut candidates = jobids(start_time, 0);

        // FilterZeroFileJobs() and PurgeJobs()
        let zero_file_jobs: Vec<String> = if candidates.is_empty() {
            Vec::new()
        } else {
            match catalog.query_rows(&format!("SELECT JobId FROM Job WHERE JobFiles = 0 AND JobId IN ({}) ORDER BY JobId",
                candidates.join(","))) {
                Ok(t) => t.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect(),
                Err(e) => {
                    out.messages.push((MsgType::Fatal, format!("{e}\n")));
                    return false;
                }
            }
        };
        candidates.retain(|id| !zero_file_jobs.contains(id));
        if !zero_file_jobs.is_empty() {
            info(out, format!("{name}: purging empty jobids {}\n", zero_file_jobs.join(", ")));
            crate::commands::purge::purge_jobs(catalog, &zero_file_jobs.join(","));
        }

        // all jobs - any empty jobs - the full backup
        let incrementals_total = all_jobids.len() as i64 - zero_file_jobs.len() as i64 - 1;
        if incrementals_total < 1 {
            info(out, format!("{name}: less than two jobs to consolidate found, doing nothing.\n"));
            continue;
        }
        let keep_number = pint32(&dirs, "alwaysincrementalkeepnumber", 0).unwrap_or(0) as i64;
        if incrementals_total <= keep_number {
            info(out, format!("{name}: less incrementals than required, not consolidating\n"));
            continue;
        }
        let max_incrementals_to_consolidate = incrementals_total - keep_number;
        let mut selected = jobids(start_time, (max_incrementals_to_consolidate + 1) as u64);
        let incrementals_to_consolidate = selected.len() as i64 - 1;
        if incrementals_to_consolidate < 1 {
            info(out, format!("{name}: After limited query: less incrementals than required, not consolidating\n"));
            continue;
        }

        // Check if we need to skip the first (full) job from consolidation
        let max_full_age = time_directive(&dirs, "alwaysincrementalmaxfullage") as i64;
        if max_full_age != 0 {
            if incrementals_to_consolidate < 2 {
                info(out, format!("{name}: less incrementals than required to consolidate without full, not consolidating\n"));
                continue;
            }
            info(out, format!("before ConsolidateFull: jobids: {}\n", selected.join(",")));
            let oldest = selected[0].clone();
            let tdate = match catalog.query_rows(&format!("SELECT JobTDate FROM Job WHERE JobId={oldest}")).ok()
                .and_then(|t| t.rows.into_iter().next()).and_then(|r| r.into_iter().next().flatten()) {
                Some(tdate) => tdate.parse::<i64>().unwrap_or(0),
                None => {
                    out.messages.push((MsgType::Fatal, format!(
                        "Error getting Job record for first Job: ERR=No Job found for JobId {oldest}\n")));
                    return true;
                }
            };
            let oldest_allowed = now - max_full_age;
            info(out, format!("check full age: full is {}, allowed is {}\n",
                crate::job_messages::bstrftimes(tdate), crate::job_messages::bstrftimes(oldest_allowed)));
            if tdate > oldest_allowed {
                info(out, format!("Full is newer than AlwaysIncrementalMaxFullAge -> skipping first jobid {oldest} because of age\n"));
                selected.remove(0);
            } else if control.max_full_consolidations != 0 && full_consolidations_started >= control.max_full_consolidations {
                info(out, format!("{} AlwaysIncrementalFullConsolidations reached -> skipping first jobid {oldest} independent of age\n",
                    control.max_full_consolidations));
                selected.remove(0);
            } else {
                info(out, format!("Full is older than AlwaysIncrementalMaxFullAge -> also consolidating Full jobid {oldest}\n"));
                full_consolidations_started += 1;
            }
            info(out, format!("after ConsolidateFull: jobids: {}\n", selected.join(",")));
        }

        info(out, format!("{name}: Start new consolidation\n"));
        // StartNewConsolidationJob(): "run job=.. jobid=.. level=VirtualFull
        // priority=.. accurate=.. spooldata=.. ignoreduplicatecheck=yes
        // consolidatejob=.." in batch mode.
        let vf = crate::vbackup::Request {
            job: job.clone(), jobids: Some(selected.join(",")), priority: Some(control.priority),
            accurate: Some(control.accurate), spool_data: Some(control.spool_data),
            consolidate_job: Some(request.job_name.to_string()), sched_time: now,
            trigger: crate::job_messages::TRIGGER_USER.to_string(), ..Default::default()
        };
        match crate::vbackup::start(config, Some(catalog), &vf) {
            Ok(child) => {
                // The batch UaContext of DoRunCmd() has no console: its
                // messages (OpenClientDb, "Job queued.") are job messages.
                info(out, format!("Using Catalog \"{}\"\n", child.report.catalog));
                info(out, format!("Job queued. JobId={}\n", child.job_id));
                info(out, format!("Consolidating JobId {} started.\n", child.job_id));
                out.children.push(child);
            }
            Err(messages) => {
                tracing::warn!("consolidate: VirtualFull of {name} not started: {messages:?}");
                out.messages.push((MsgType::Error, "Could not start Consolidating job.\n".into()));
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
    use crate::internal_jobs::Kind;

    fn rows(values: &[&str]) -> CatalogResponse {
        CatalogResponse::Rows(SqlTable {
            columns: vec![SqlColumn { name: "x".into(), numeric: false }],
            rows: values.iter().map(|v| vec![Some(v.to_string())]).collect(),
        })
    }

    /// Two Always Incremental jobs (FileSets fs1/fs2) with old Full jobs.
    fn catalog() -> CatalogHandle {
        let mut fileset = 0;
        CatalogHandle::mock(move |request| match request {
            CatalogRequest::Execute { sql } => {
                if let Some(at) = sql.find("FileSet.FileSetId = ") {
                    fileset = sql[at + 20..].chars().next().unwrap().to_digit(10).unwrap();
                }
                CatalogResponse::Ok(0)
            }
            CatalogRequest::TextRows { sql } => {
                if sql.contains("FileSet='fs1'") {
                    rows(&["1"])
                } else if sql.contains("FileSet='fs2'") {
                    rows(&["2"])
                } else if sql.starts_with("SELECT ClientId") {
                    rows(&["1"])
                } else if sql.starts_with("SELECT JobId FROM btemp3") {
                    let all: &[&str] = if fileset == 1 { &["10", "11", "12", "13"] } else { &["20", "21", "22"] };
                    let limit = sql.split("LIMIT ").nth(1).map(|l| l.trim().parse().unwrap()).unwrap_or(all.len());
                    rows(&all[..limit.min(all.len())])
                } else if sql.starts_with("SELECT JobId FROM Job WHERE JobFiles = 0") {
                    rows(if fileset == 1 { &["13"] } else { &[] })
                } else if sql.starts_with("SELECT JobTDate") {
                    rows(&["1000"])
                } else {
                    rows(&[])
                }
            }
            _ => CatalogResponse::Error("unexpected".into()),
        })
    }

    #[test]
    fn selection_with_max_full_age_and_max_full_consolidations() {
        let job = |name: &str, fileset: &str| format!("Job {{ Name = {name}; Type = Backup; Client = c; FileSet = {fileset}; \
            Pool = p; Always Incremental = yes; Always Incremental Max Full Age = 1 day }}\n");
        let config = DirectorConfig::from_str(&format!("Director {{ Name = dir }}\nClient {{ Name = c; Address = x; Password = y }}\n\
            Pool {{ Name = p; Pool Type = Backup }}\n{}{}Job {{ Name = cons; Type = Consolidate; Max Full Consolidations = 1 }}\n",
            job("a1", "fs1"), job("a2", "fs2"))).unwrap();
        let request = Request { kind: Kind::Consolidate, job_id: 9, unique_job: "cons.x", job_name: "cons", pool: "p" };
        let mut out = Outcome::default();
        assert!(consolidate_jobs(&config, &catalog(), &request, &mut out));
        let texts: Vec<&str> = out.messages.iter().map(|(_, t)| t.as_str()).collect();
        assert!(texts[3].starts_with("check full age: full is "), "{texts:?}");
        assert_eq!(texts, [
            "Looking at always incremental job a1\n",
            "a1: purging empty jobids 13\n",
            "before ConsolidateFull: jobids: 10,11,12\n",
            texts[3],
            "Full is older than AlwaysIncrementalMaxFullAge -> also consolidating Full jobid 10\n",
            "after ConsolidateFull: jobids: 10,11,12\n",
            "a1: Start new consolidation\n",
            "Could not start Consolidating job.\n",
            "Looking at always incremental job a2\n",
            "before ConsolidateFull: jobids: 20,21,22\n",
            texts[10],
            "1 AlwaysIncrementalFullConsolidations reached -> skipping first jobid 20 independent of age\n",
            "after ConsolidateFull: jobids: 21,22\n",
            "a2: Start new consolidation\n",
            "Could not start Consolidating job.\n",
        ]);
    }
}
