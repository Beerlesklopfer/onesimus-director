//! JobDefs inheritance, as dird_conf.cc PopulateJobdefaults() does it.
//!
//! - every JobDefs resource, in resource-chain order, takes the defaults of
//!   the JobDefs it names (`PropagateJobdefs(R_JOBDEFS, ...)`); a JobDefs
//!   later in the chain has not been propagated yet when an earlier one
//!   copies from it;
//! - then every Job takes the defaults of its JobDefs (`PropagateJobdefs(R_JOB, ...)`).
//!
//! `PropagateResource()` copies an item only when the destination does not
//! have it (`IsMemberPresent`, set by the parser for every directive written
//! in the resource — defaults never count) and the source has it. Lists
//! (`Storage`, `Run`, `*PluginOptions`) are therefore replaced as a whole,
//! never merged. `Name` (CFG_TYPE_NAME) and the run scripts
//! (CFG_TYPE_RUNSCRIPT / CFG_TYPE_SHRTRUNSCRIPT) are not copied by
//! `PropagateResource()`; instead `PropagateJobdefs()` appends every run
//! script of the JobDefs after the resource's own ones (`from_jobdef`).
//! A JobDefs naming itself propagates nothing.

use std::collections::HashSet;

use crate::raw::{normalize_key, ConfResource, Directive};
use crate::resources::{JobDefsResource, JobResource};

/// Item a Job/JobDefs directive key belongs to (normalized; aliases of
/// dird_conf.cc job_items resolved).
pub fn item_key(key: &str) -> String {
    let key = normalize_key(key);
    match key.as_str() {
        "verifyjob" => "jobtoverify".to_string(),
        _ => key,
    }
}

/// Run script items: RunScript {} and the short forms (all stored in RunScripts).
pub fn is_runscript(item: &str) -> bool {
    matches!(item, "runscript" | "runbeforejob" | "runafterjob" | "runafterfailedjob"
        | "clientrunbeforejob" | "clientrunafterjob")
}

/// Items PropagateResource() copies (everything but Name and the run scripts).
pub fn is_propagated(item: &str) -> bool {
    item != "name" && !is_runscript(item)
}

/// A directive of a Job/JobDefs after propagation.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub directive: Directive,
    /// Copied from the JobDefs (inherit_content_ bit / RunScript::from_jobdef)
    pub inherited: bool,
}

/// Job and JobDefs resources after PopulateJobdefaults(), in chain order.
#[derive(Debug, Clone, Default)]
pub struct Propagated {
    pub jobdefs: Vec<(String, Vec<Effective>)>,
    pub jobs: Vec<(String, Vec<Effective>)>,
}

impl Propagated {
    /// Directives of a Job (own first, then inherited ones).
    pub fn job(&self, name: &str) -> Option<&[Effective]> {
        self.jobs.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
    }

    /// Directives of a JobDefs after its own propagation.
    pub fn jobdefs(&self, name: &str) -> Option<&[Effective]> {
        self.jobdefs.iter().find(|(n, _)| n == name).map(|(_, d)| d.as_slice())
    }
}

/// Plain directive list of an effective resource.
pub fn directives(effective: &[Effective]) -> Vec<Directive> {
    effective.iter().map(|e| e.directive.clone()).collect()
}

fn jobdefs_name(dirs: &[Effective]) -> Option<String> {
    dirs.iter().rev()
        .find(|e| !e.inherited && item_key(&e.directive.key) == "jobdefs")
        .and_then(|e| e.directive.first())
        .map(str::to_string)
}

/// PropagateJobdefs() + PropagateResource() for one resource.
fn propagate(dest: &mut Vec<Effective>, source: &[Effective]) {
    let present: HashSet<String> = dest.iter().map(|e| item_key(&e.directive.key)).collect();
    // RunScripts: appended (DuplicateRunscript, from_jobdef = true)
    for e in source.iter().filter(|e| is_runscript(&item_key(&e.directive.key))) {
        dest.push(Effective { directive: e.directive.clone(), inherited: true });
    }
    // Every other item: only when the destination does not have it
    for e in source {
        let item = item_key(&e.directive.key);
        if is_propagated(&item) && !present.contains(&item) {
            dest.push(Effective { directive: e.directive.clone(), inherited: true });
        }
    }
}

/// PopulateJobdefaults() over the raw configuration.
pub fn propagate_raw(raw: &[ConfResource]) -> Propagated {
    let own = |r: &ConfResource| -> Vec<Effective> {
        r.directives.iter().map(|d| Effective { directive: d.clone(), inherited: false }).collect()
    };
    let mut out = Propagated::default();
    for r in raw.iter().filter(|r| r.norm_type() == "jobdefs") {
        out.jobdefs.push((r.name().unwrap_or_default().to_string(), own(r)));
    }
    for i in 0..out.jobdefs.len() {
        let Some(parent) = jobdefs_name(&out.jobdefs[i].1) else { continue };
        // Don't allow the JobDefs pointing to itself.
        if parent == out.jobdefs[i].0 {
            continue;
        }
        let Some(source) = out.jobdefs(&parent).map(<[Effective]>::to_vec) else { continue };
        propagate(&mut out.jobdefs[i].1, &source);
    }
    for r in raw.iter().filter(|r| r.norm_type() == "job") {
        let mut dirs = own(r);
        if let Some(source) = jobdefs_name(&dirs).and_then(|p| out.jobdefs(&p)) {
            let source = source.to_vec();
            propagate(&mut dirs, &source);
        }
        out.jobs.push((r.name().unwrap_or_default().to_string(), dirs));
    }
    out
}

/// Effective directives of one Job (`kind` "job") or JobDefs (`kind` "jobdefs").
pub fn effective(raw: &[ConfResource], kind: &str, name: &str) -> Option<Vec<Effective>> {
    let all = propagate_raw(raw);
    match kind {
        "jobdefs" => all.jobdefs(name),
        _ => all.job(name),
    }
    .map(<[Effective]>::to_vec)
}

/// PropagateJobdefs() + PropagateResource() for the typed fields of a JobResource.
pub fn propagate_typed_one(dest: &mut JobResource, src: &JobResource) {
    macro_rules! items {
        ($($item:literal => $field:ident),* $(,)?) => {$(
            if !dest.present.contains($item) && src.present.contains($item) {
                dest.$field = src.$field.clone();
                dest.present.insert($item.to_string());
            }
        )*};
    }
    items! {
        "type" => job_type,
        "level" => level,
        "client" => client,
        "fileset" => fileset,
        "schedule" => schedule,
        "storage" => storage,
        "pool" => pool,
        "fullbackuppool" => full_backup_pool,
        "differentialbackuppool" => differential_backup_pool,
        "incrementalbackuppool" => incremental_backup_pool,
        "messages" => messages,
        "priority" => priority,
        "maximumconcurrentjobs" => max_concurrent_jobs,
        "enabled" => enabled,
        "writebootstrap" => write_bootstrap,
    }
    // Typed fields hold one short run script each; the run scripts of a
    // JobDefs are appended after the Job's own, the first one is kept here.
    if dest.run_before_job.is_empty() { dest.run_before_job = src.run_before_job.clone(); }
    if dest.run_after_job.is_empty() { dest.run_after_job = src.run_after_job.clone(); }
}

/// PopulateJobdefaults() over the typed resources (chain order).
pub fn propagate_typed(jobdefs: &mut [JobDefsResource], jobs: &mut [JobResource]) {
    for i in 0..jobdefs.len() {
        let parent = jobdefs[i].job.jobdefs.clone();
        if parent.is_empty() || parent == jobdefs[i].name {
            continue;
        }
        let Some(src) = jobdefs.iter().find(|d| d.name == parent).map(|d| d.job.clone()) else { continue };
        propagate_typed_one(&mut jobdefs[i].job, &src);
    }
    for job in jobs.iter_mut() {
        propagate_job(jobdefs, job);
    }
}

/// PropagateJobdefs(R_JOB, job) against already propagated JobDefs
/// (also used by `configure add job`).
pub fn propagate_job(jobdefs: &[JobDefsResource], job: &mut JobResource) {
    if job.jobdefs.is_empty() {
        return;
    }
    if let Some(def) = jobdefs.iter().find(|d| d.name == job.jobdefs) {
        let src = def.job.clone();
        propagate_typed_one(job, &src);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::{JobLevel, JobType};

    fn raw(text: &str) -> Vec<ConfResource> {
        crate::raw::parse_str(text, None).unwrap()
    }

    fn values(dirs: &[Effective], key: &str) -> Vec<(String, bool)> {
        dirs.iter().filter(|e| item_key(&e.directive.key) == key)
            .map(|e| (e.directive.values().join(","), e.inherited)).collect()
    }

    fn typed(text: &str) -> (Vec<JobDefsResource>, Vec<JobResource>) {
        let mut defs = Vec::new();
        let mut jobs = Vec::new();
        for r in crate::parse_config(text).unwrap() {
            match r {
                crate::Resource::JobDefs(d) => defs.push(d),
                crate::Resource::Job(j) => jobs.push(j),
                _ => {}
            }
        }
        propagate_typed(&mut defs, &mut jobs);
        (defs, jobs)
    }

    const DEFAULTS: &str = r#"
JobDefs {
  Name = "DefaultJob"
  Type = Backup
  Level = Incremental
  Client = bareos-fd
  FileSet = "SelfTest"
  Schedule = "WeeklyCycle"
  Storage = File
  Messages = Standard
  Pool = Incremental
  Priority = 10
  Write Bootstrap = "/var/lib/bareos/%c.bsr"
  Full Backup Pool = Full
  Differential Backup Pool = Differential
  Incremental Backup Pool = Incremental
  RunBeforeJob = "defs-before"
}
Job {
  Name = "BackupCatalog"
  JobDefs = "DefaultJob"
  Level = Full
  FileSet = "Catalog"
  Priority = 11
  RunBeforeJob = "own-before"
}
Job {
  Name = "backup-bareos-fd"
  JobDefs = "DefaultJob"
  Client = "bareos-fd"
}
"#;

    #[test]
    fn explicit_level_full_wins_over_jobdefs() {
        let p = propagate_raw(&raw(DEFAULTS));
        let job = p.job("BackupCatalog").unwrap();
        assert_eq!(values(job, "level"), vec![("Full".into(), false)]);
        assert_eq!(values(job, "fileset"), vec![("Catalog".into(), false)]);
        assert_eq!(values(job, "pool"), vec![("Incremental".into(), true)]);
        let (_, jobs) = typed(DEFAULTS);
        assert_eq!(jobs[0].level, JobLevel::Full);
        assert_eq!(jobs[0].fileset, "Catalog");
        assert_eq!(jobs[0].priority, 11);
        assert_eq!(jobs[0].pool, "Incremental");
        assert_eq!(jobs[1].level, JobLevel::Incremental);
        assert_eq!(jobs[1].schedule, "WeeklyCycle");
    }

    #[test]
    fn explicit_default_values_win() {
        // Values equal to the built-in defaults are still "present".
        let text = r#"
JobDefs { Name = D; Type = Restore; Level = Differential; Priority = 5; Enabled = no; Maximum Concurrent Jobs = 4 }
Job { Name = J; JobDefs = D; Type = Backup; Level = Full; Priority = 10; Enabled = yes; Maximum Concurrent Jobs = 1 }
Job { Name = K; JobDefs = D }
"#;
        let (_, jobs) = typed(text);
        assert_eq!(jobs[0].job_type, JobType::Backup);
        assert_eq!(jobs[0].level, JobLevel::Full);
        assert_eq!(jobs[0].priority, 10);
        assert!(jobs[0].enabled);
        assert_eq!(jobs[0].max_concurrent_jobs, 1);
        assert_eq!(jobs[1].job_type, JobType::Restore);
        assert_eq!(jobs[1].level, JobLevel::Differential);
        assert_eq!(jobs[1].priority, 5);
        assert!(!jobs[1].enabled);
        assert_eq!(jobs[1].max_concurrent_jobs, 4);
    }

    #[test]
    fn lists_are_replaced_not_merged() {
        let text = r#"
JobDefs { Name = D; Storage = A, B; Run = "x"; Run = "y"; FdPluginOptions = "p" }
Job { Name = J; JobDefs = D; Storage = C; Run = "z" }
Job { Name = K; JobDefs = D }
"#;
        let p = propagate_raw(&raw(text));
        let j = p.job("J").unwrap();
        assert_eq!(values(j, "storage"), vec![("C".into(), false)]);
        assert_eq!(values(j, "run"), vec![("z".into(), false)]);
        assert_eq!(values(j, "fdpluginoptions"), vec![("p".into(), true)]);
        let k = p.job("K").unwrap();
        assert_eq!(values(k, "storage"), vec![("A,B".into(), true)]);
        assert_eq!(values(k, "run"), vec![("x".into(), true), ("y".into(), true)]);
    }

    #[test]
    fn runscripts_are_appended() {
        let p = propagate_raw(&raw(DEFAULTS));
        let job = p.job("BackupCatalog").unwrap();
        let scripts: Vec<_> = job.iter().filter(|e| is_runscript(&item_key(&e.directive.key)))
            .map(|e| (e.directive.first().unwrap().to_string(), e.inherited)).collect();
        assert_eq!(scripts, vec![("own-before".into(), false), ("defs-before".into(), true)]);
        let text = r#"
JobDefs { Name = D; RunScript { Command = "c"; RunsWhen = After } }
Job { Name = J; JobDefs = D; RunAfterJob = "a" }
"#;
        let p = propagate_raw(&raw(text));
        let keys: Vec<_> = p.job("J").unwrap().iter().map(|e| (item_key(&e.directive.key), e.inherited)).collect();
        assert_eq!(keys, vec![("name".into(), false), ("jobdefs".into(), false),
            ("runafterjob".into(), false), ("runscript".into(), true)]);
    }

    #[test]
    fn name_and_aliases() {
        let text = r#"
JobDefs { Name = D; Description = "d"; VerifyJob = X }
Job { Name = J; JobDefs = D; Job To Verify = Y }
"#;
        let p = propagate_raw(&raw(text));
        let j = p.job("J").unwrap();
        assert_eq!(values(j, "name"), vec![("J".into(), false)]);
        assert_eq!(values(j, "description"), vec![("d".into(), true)]);
        assert_eq!(values(j, "jobtoverify"), vec![("Y".into(), false)]);
    }

    #[test]
    fn jobdefs_chain_in_resource_order() {
        // A before B: A copies B before B got C's items.
        let text = r#"
JobDefs { Name = A; JobDefs = B; Level = Full }
JobDefs { Name = B; JobDefs = C; Pool = PB; RunBeforeJob = "b" }
JobDefs { Name = C; Pool = PC; Schedule = SC; Client = CC; RunBeforeJob = "c" }
JobDefs { Name = E; JobDefs = B }
JobDefs { Name = S; JobDefs = S; Pool = PS }
Job { Name = JA; JobDefs = A }
Job { Name = JE; JobDefs = E }
Job { Name = JS; JobDefs = S }
"#;
        let p = propagate_raw(&raw(text));
        let ja = p.job("JA").unwrap();
        assert_eq!(values(ja, "level"), vec![("Full".into(), true)]);
        assert_eq!(values(ja, "pool"), vec![("PB".into(), true)]);
        assert!(values(ja, "schedule").is_empty());
        assert_eq!(values(ja, "jobdefs"), vec![("A".into(), false)]);
        let je = p.job("JE").unwrap();
        assert_eq!(values(je, "pool"), vec![("PB".into(), true)]);
        assert_eq!(values(je, "schedule"), vec![("SC".into(), true)]);
        assert_eq!(values(je, "runbeforejob"), vec![("b".into(), true), ("c".into(), true)]);
        assert_eq!(values(p.job("JS").unwrap(), "pool"), vec![("PS".into(), true)]);
        // JobDefs own view: B inherited C's
        let b = p.jobdefs("B").unwrap();
        assert_eq!(values(b, "client"), vec![("CC".into(), true)]);
        assert_eq!(values(b, "pool"), vec![("PB".into(), false)]);

        let (defs, jobs) = typed(text);
        assert_eq!(jobs[0].pool, "PB");
        assert_eq!(jobs[0].schedule, "");
        assert_eq!(jobs[0].level, JobLevel::Full);
        assert_eq!(jobs[1].schedule, "SC");
        assert_eq!(jobs[1].client, "CC");
        assert_eq!(jobs[2].pool, "PS");
        assert_eq!(defs[1].job.client, "CC");
    }

    #[test]
    fn typed_and_raw_agree_on_every_typed_item() {
        let text = r#"
JobDefs { Name = D; Type = Verify; Level = Differential; Client = c; FileSet = f; Schedule = s; Storage = st; Pool = p
  Full Backup Pool = fp; Differential Backup Pool = dp; Incremental Backup Pool = ip; Messages = m; Priority = 7
  Maximum Concurrent Jobs = 3; Enabled = no; Write Bootstrap = "/b" }
Job { Name = J; JobDefs = D }
"#;
        let p = propagate_raw(&raw(text));
        let j = p.job("J").unwrap();
        let (_, jobs) = typed(text);
        let t = &jobs[0];
        for (key, v) in [("client", &t.client), ("fileset", &t.fileset), ("schedule", &t.schedule),
            ("storage", &t.storage), ("pool", &t.pool), ("fullbackuppool", &t.full_backup_pool),
            ("differentialbackuppool", &t.differential_backup_pool),
            ("incrementalbackuppool", &t.incremental_backup_pool), ("messages", &t.messages),
            ("writebootstrap", &t.write_bootstrap)] {
            assert_eq!(values(j, key), vec![(v.clone(), true)], "{key}");
        }
        assert_eq!(t.job_type, JobType::Verify);
        assert_eq!(t.level, JobLevel::Differential);
        assert_eq!(t.priority, 7);
        assert_eq!(t.max_concurrent_jobs, 3);
        assert!(!t.enabled);
    }
}
