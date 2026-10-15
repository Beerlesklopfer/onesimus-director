//! Bareos wait: select a running job, then report its actual catalog outcome.
use std::time::{Duration, Instant};
use crate::{command_ast::CommandAst, command_executor::ExecContext, command_queue::CommandResult,
    job_manager::{ActiveJob, DirJobPhase}};

enum Selection { All, Id(u32), Name(String), Unique(String), Mount }
struct Options { selection: Selection, timeout: Option<Duration> }

trait Backend {
    fn require_catalog(&mut self) -> Result<(), String>;
    fn active_jobs(&mut self) -> Result<Vec<ActiveJob>, String>;
    fn status(&mut self, id: u32) -> Result<Option<char>, String>;
    fn sleep(&mut self, duration: Duration);
    fn elapsed(&self) -> Duration;
}

struct Director<'a> { ctx: &'a ExecContext, started: Instant }
impl Backend for Director<'_> {
    fn require_catalog(&mut self) -> Result<(), String> {
        self.ctx.catalog.as_ref().ok_or("ERR: Can't open db\n")?
            .query_rows("SELECT 1").map(|_| ()).map_err(|error| format!("ERR: Can't open db: {error}\n"))
    }
    fn active_jobs(&mut self) -> Result<Vec<ActiveJob>, String> {
        self.ctx.job_manager.as_ref().ok_or("ERR: Job manager is unavailable\n")?
            .lock().map(|manager| manager.active_jobs()).map_err(|_| "ERR: Job manager lock poisoned\n".into())
    }
    fn status(&mut self, id: u32) -> Result<Option<char>, String> {
        let table = self.ctx.catalog.as_ref().ok_or("ERR: Can't open db\n")?
            .query_rows(&format!("SELECT JobStatus FROM Job WHERE JobId={id}"))
            .map_err(|error| format!("ERR: Can't read JobStatus: {error}\n"))?;
        Ok(table.rows.first().and_then(|row| row.first()).and_then(|value| value.as_deref()).and_then(|value| value.chars().next()))
    }
    fn sleep(&mut self, duration: Duration) { std::thread::sleep(duration); }
    fn elapsed(&self) -> Duration { self.started.elapsed() }
}

pub fn cmd_wait(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    wait_with(ast, &mut Director { ctx, started: Instant::now() })
}

fn parse(ast: &CommandAst) -> Result<Options, String> {
    let mut selection = None;
    let mut timeout = None;
    for argument in &ast.arguments {
        let value = || argument.value.as_deref().filter(|value| !value.is_empty())
            .ok_or_else(|| format!("wait: {} requires a value\n", argument.key));
        let next = match argument.key.to_ascii_lowercase().as_str() {
            "jobid" => Selection::Id(value()?.parse::<u32>().ok().filter(|id| *id > 0)
                .ok_or("wait: jobid must be a positive integer\n")?),
            "jobname" | "job" => Selection::Name(value()?.to_string()),
            "ujobid" => Selection::Unique(value()?.to_string()),
            "mount" => {
                if argument.value.is_some() { return Err("wait: mount takes no value\n".into()); }
                Selection::Mount
            }
            "timeout" => {
                if timeout.is_some() { return Err("wait: duplicate timeout\n".into()); }
                let seconds = value()?.parse::<u64>().map_err(|_| "wait: invalid timeout\n")?;
                let duration = Duration::from_secs(seconds);
                if Instant::now().checked_add(duration).is_none() { return Err("wait: timeout is too large\n".into()); }
                timeout = Some(duration);
                continue;
            }
            _ => return Err(format!("wait: unknown argument {}\n", argument.key)),
        };
        if selection.is_some() { return Err("wait: specify only one job selector or mount\n".into()); }
        selection = Some(next);
    }
    if selection.is_none() && timeout.is_some() { return Err("wait: timeout requires a job selector or mount\n".into()); }
    Ok(Options { selection: selection.unwrap_or(Selection::All), timeout })
}

fn wait_with(ast: &CommandAst, backend: &mut impl Backend) -> CommandResult {
    let options = match parse(ast) { Ok(options) => options, Err(error) => return CommandResult::error(error) };
    if !matches!(options.selection, Selection::All) {
        if let Err(error) = backend.require_catalog() { return CommandResult::error(error); }
    }
    let selected_id = match options.selection {
        Selection::Id(id) => Some(id),
        Selection::Name(ref name) | Selection::Unique(ref name) => {
            let jobs = match backend.active_jobs() { Ok(jobs) => jobs, Err(error) => return CommandResult::error(error) };
            let selected = jobs.iter().find(|job| match options.selection {
                Selection::Name(_) => job.unique_name.starts_with(name),
                _ => job.unique_name == *name,
            });
            match selected {
                Some(job) => Some(job.id),
                None => return CommandResult::error("ERR: Job was not found\n".into()),
            }
        }
        _ => None,
    };
    // The original allows briefly queued jobs to enter the running table.
    if !matches!(options.selection, Selection::Mount) { backend.sleep(Duration::from_millis(200)); }
    loop {
        let jobs = match backend.active_jobs() { Ok(jobs) => jobs, Err(error) => return CommandResult::error(error) };
        let finished = match options.selection {
            Selection::Mount => jobs.iter().any(|job| matches!(job.phase, DirJobPhase::WaitingMedia | DirJobPhase::WaitingMount)),
            Selection::All => jobs.is_empty(),
            _ => !jobs.iter().any(|job| Some(job.id) == selected_id),
        };
        if finished { break; }
        // As upstream, timeout applies to mount waits only.
        if matches!(options.selection, Selection::Mount) && options.timeout.is_some_and(|timeout| backend.elapsed() >= timeout) {
            return CommandResult::ok("Wait on mount timed out\n".into());
        }
        backend.sleep(Duration::from_secs(1));
    }
    let Some(id) = selected_id else { return CommandResult::ok(String::new()); };
    match backend.status(id) {
        Ok(status) => outcome(id, status.unwrap_or('?')),
        Err(error) => CommandResult::error(error),
    }
}

fn outcome(id: u32, status: char) -> CommandResult {
    let (description, exit_status) = match status {
        'T' => ("OK", 0),
        'W' => ("OK -- with warnings", 0),
        'e' => ("Error", 1),
        'E' => ("Error", 2),
        'f' => ("Fatal Error", 2),
        'A' => ("Canceled", 2),
        'D' => ("Differences", 3),
        _ => ("Unknown term code", 3),
    };
    // ExitStatus has no text-format argument in upstream; it is JSON-only.
    CommandResult::ok(format!("JobId={id}\nJobStatus={description} ({status})\n"))
        .with_json(serde_json::json!({"job": {
            "jobid": id, "jobstatuslong": description, "jobstatus": status.to_string(), "exitstatus": exit_status,
        }}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    struct Fake {
        snapshots: VecDeque<Vec<ActiveJob>>, elapsed: Duration, queried: Vec<u32>,
        status: Result<Option<char>, String>, catalog: Result<(), String>,
    }
    impl Backend for Fake {
        fn require_catalog(&mut self) -> Result<(), String> { self.catalog.clone() }
        fn active_jobs(&mut self) -> Result<Vec<ActiveJob>, String> { Ok(self.snapshots.pop_front().unwrap_or_default()) }
        fn status(&mut self, id: u32) -> Result<Option<char>, String> { self.queried.push(id); self.status.clone() }
        fn sleep(&mut self, duration: Duration) { self.elapsed += duration; }
        fn elapsed(&self) -> Duration { self.elapsed }
    }
    fn job(phase: DirJobPhase) -> ActiveJob { ActiveJob { id: 42, name: "Backup".into(), unique_name: "Backup.2026-10-15_14.00.00".into(), phase } }
    fn backend(snapshots: Vec<Vec<ActiveJob>>) -> Fake {
        Fake { snapshots: snapshots.into(), elapsed: Duration::ZERO, queried: vec![], status: Ok(Some('T')), catalog: Ok(()) }
    }
    #[test]
    fn bare_wait_is_silent_and_waits_until_the_running_set_is_empty() {
        let mut fake = backend(vec![vec![job(DirJobPhase::Running)], vec![]]);
        fake.catalog = Err("must not open catalog".into());
        let result = wait_with(&crate::session::parse_cli_command("wait"), &mut fake);
        assert!(result.success); assert!(result.text_output.is_empty()); assert!(fake.queried.is_empty());
        assert_eq!(fake.elapsed, Duration::from_millis(1200));
    }
    #[test]
    fn selected_job_outcomes_match_original_text_and_json_including_failure() {
        for (status, description, exit_status) in [('T',"OK",0),('W',"OK -- with warnings",0),
            ('e',"Error",1),('E',"Error",2),('f',"Fatal Error",2),('A',"Canceled",2),('D',"Differences",3),('?',"Unknown term code",3)] {
            let mut fake = backend(vec![vec![job(DirJobPhase::Running)],vec![]]); fake.status=Ok(Some(status));
            let result = wait_with(&crate::session::parse_cli_command("wait jobid=42"), &mut fake);
            assert_eq!(result.text_output,format!("JobId=42\nJobStatus={description} ({status})\n"));
            assert_eq!(result.json.unwrap(),serde_json::json!({"job":{"jobid":42,"jobstatuslong":description,"jobstatus":status.to_string(),"exitstatus":exit_status}}));
            assert_eq!(fake.queried,vec![42]);
        }
    }
    #[test]
    fn name_alias_and_unique_name_select_the_actual_running_job() {
        for selector in ["jobname=Back", "job=Backup", "ujobid=Backup.2026-10-15_14.00.00"] {
            let mut fake=backend(vec![vec![job(DirJobPhase::Running)],vec![]]);
            assert!(wait_with(&crate::session::parse_cli_command(&format!("wait {selector}")), &mut fake).success);
            assert_eq!(fake.queried,vec![42]);
        }
    }
    #[test]
    fn mount_wait_observes_media_state_and_uses_timeout() {
        let mut fake=backend(vec![vec![job(DirJobPhase::Running)],vec![job(DirJobPhase::WaitingMedia)]]);
        let result=wait_with(&crate::session::parse_cli_command("wait mount timeout=5"),&mut fake);
        assert!(result.success); assert!(result.text_output.is_empty()); assert!(fake.queried.is_empty());
        let mut fake=backend(vec![vec![],vec![]]);
        let result=wait_with(&crate::session::parse_cli_command("wait mount timeout=1"),&mut fake);
        assert_eq!(result.text_output,"Wait on mount timed out\n");
        assert_eq!(fake.elapsed,Duration::from_secs(1));
    }
    #[test]
    fn invalid_input_missing_jobs_and_catalog_errors_never_claim_completion() {
        for line in ["wait jobid=0","wait jobid=bad","wait jobname","wait mount=yes","wait jobid=1 jobid=2","wait mount timeout=-1"] {
            let mut fake=backend(vec![vec![job(DirJobPhase::Running)]]);
            assert!(!wait_with(&crate::session::parse_cli_command(line),&mut fake).success,"{line}");
            assert_eq!(fake.snapshots.len(),1); assert!(fake.queried.is_empty());
        }
        let mut fake=backend(vec![vec![]]);
        assert!(!wait_with(&crate::session::parse_cli_command("wait jobname=missing"),&mut fake).success);
        let mut fake=backend(vec![vec![]]);fake.catalog=Err("database unavailable".into());
        assert!(!wait_with(&crate::session::parse_cli_command("wait jobid=42"),&mut fake).success);
        assert_eq!(fake.snapshots.len(),1);
        let mut fake=backend(vec![vec![]]);fake.status=Err("status query failed".into());
        let result=wait_with(&crate::session::parse_cli_command("wait jobid=42"),&mut fake);
        assert!(!result.success);assert_eq!(result.error_message,"status query failed");
    }
}
