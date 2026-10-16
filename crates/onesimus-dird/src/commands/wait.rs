//! Bareos wait: select a running job, then report its actual catalog outcome.
use std::time::{Duration, Instant};
use crate::{command_ast::CommandAst, command_executor::ExecContext, command_queue::CommandResult,
    job_manager::{ActiveJob, DirJobPhase}};

enum Selection { All, Id(u32), Name(String), Unique(String), Mount }
struct Options { selection: Selection, timeout: Option<i64> }

trait Backend {
    /// C++ wait_cmd OpenClientDb(); Ok carries the catalog messages.
    fn require_catalog(&mut self, ast: &CommandAst) -> Result<String, String>;
    fn active_jobs(&mut self) -> Result<Vec<ActiveJob>, String>;
    fn status(&mut self, id: u32) -> Result<Option<char>, String>;
    fn completion_error(&mut self, _id: Option<u32>) -> Result<Option<String>, String> { Ok(None) }
    fn sleep(&mut self, duration: Duration);
    fn elapsed(&self) -> Duration;
}

struct Director<'a> { ctx: &'a ExecContext, started: Instant }
impl Backend for Director<'_> {
    fn require_catalog(&mut self, ast: &CommandAst) -> Result<String, String> {
        let prefix = crate::ua_db::open_client_db(ast, self.ctx, false)
            .map_err(|result| format!("{}{}ERR: Can't open db\n", result.text_output, result.error_message))?;
        self.ctx.catalog.as_ref().ok_or("ERR: Can't open db\n")?
            .query_rows("SELECT 1").map(|_| prefix).map_err(|error| format!("ERR: Can't open db: {error}\n"))
    }
    fn active_jobs(&mut self) -> Result<Vec<ActiveJob>, String> {
        self.ctx.job_manager.as_ref().ok_or("ERR: Job manager is unavailable\n")?
            .lock().map(|manager| manager.active_jobs()).map_err(|_| "ERR: Job manager lock poisoned\n".into())
    }
    fn completion_error(&mut self, id: Option<u32>) -> Result<Option<String>, String> {
        let manager = self.ctx.job_manager.as_ref().ok_or("ERR: Job manager is unavailable\n")?
            .lock().map_err(|_| "ERR: Job manager lock poisoned\n")?;
        Ok(manager.completion_error(id, self.started))
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

/// wait_cmd (ua_cmds.cc:2618): no arguments waits for all jobs; otherwise
/// `timeout=` (str_to_int64 seconds, mount only) and the first of
/// jobid/jobname/job/ujobid/mount decide. Anything else leaves JobId 0
/// ("Job was not found").
fn parse(ast: &CommandAst) -> Options {
    if ast.arguments.is_empty() {
        return Options { selection: Selection::All, timeout: None };
    }
    let timeout = ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case("timeout"))
        .and_then(|a| a.value.as_deref()).map(crate::commands::job::str_to_int64);
    let mut selection = Selection::Id(0);
    for argument in &ast.arguments {
        let value = argument.value.clone();
        selection = match argument.key.to_ascii_lowercase().as_str() {
            "jobid" => Selection::Id(value.map_or(0, |v| crate::commands::job::str_to_int64(&v) as u32)),
            "jobname" | "job" => value.map_or(Selection::Id(0), Selection::Name),
            "ujobid" => value.map_or(Selection::Id(0), Selection::Unique),
            "mount" => Selection::Mount,
            _ => continue,
        };
        break;
    }
    Options { selection, timeout }
}

fn wait_with(ast: &CommandAst, backend: &mut impl Backend) -> CommandResult {
    let mut prefix = String::new();
    let result = wait_selected(ast, backend, &mut prefix);
    crate::ua_db::with_prefix(prefix, result)
}

fn wait_selected(ast: &CommandAst, backend: &mut impl Backend, prefix: &mut String) -> CommandResult {
    // wait_cmd returns true on every path; the Job selection errors are
    // ErrorMsg only.
    let not_found = || { let mut r = CommandResult::error("ERR: Job was not found\n".into()); r.success = true; r };
    let options = parse(ast);
    if !matches!(options.selection, Selection::All) {
        match backend.require_catalog(ast) {
            Ok(messages) => *prefix = messages,
            Err(error) => { let mut r = CommandResult::error(error); r.success = true; return r; }
        }
    }
    let selected_id = match options.selection {
        Selection::Id(0) => return not_found(),
        Selection::Id(id) => Some(id),
        Selection::Name(ref name) | Selection::Unique(ref name) => {
            let jobs = match backend.active_jobs() { Ok(jobs) => jobs, Err(error) => return CommandResult::error(error) };
            let selected = jobs.iter().find(|job| match options.selection {
                Selection::Name(_) => job.unique_name.starts_with(name),
                _ => job.unique_name == *name,
            });
            match selected {
                Some(job) => Some(job.id),
                None => return not_found(),
            }
        }
        _ => None,
    };
    // The original allows briefly queued jobs to enter the running table.
    if !matches!(options.selection, Selection::Mount) { backend.sleep(Duration::from_millis(200)); }
    loop {
        if !matches!(options.selection, Selection::Mount) {
            match backend.completion_error(selected_id) {
                Ok(Some(error)) => return CommandResult::error(format!("ERR: {error}\n")),
                Err(error) => return CommandResult::error(error),
                _ => {},
            }
        }
        let jobs = match backend.active_jobs() { Ok(jobs) => jobs, Err(error) => return CommandResult::error(error) };
        let finished = match options.selection {
            Selection::Mount => jobs.iter().any(|job| matches!(job.phase, DirJobPhase::WaitingMedia | DirJobPhase::WaitingMount)),
            Selection::All => jobs.is_empty(),
            _ => !jobs.iter().any(|job| Some(job.id) == selected_id),
        };
        if finished {
            // The poller can finish a catalog update between the two snapshots.
            // Recheck after observing terminal state, before reporting success.
            if !matches!(options.selection, Selection::Mount) {
                match backend.completion_error(selected_id) {
                    Ok(Some(error)) => return CommandResult::error(format!("ERR: {error}\n")),
                    Err(error) => return CommandResult::error(error),
                    _ => {},
                }
            }
            break;
        }
        // As upstream, timeout applies to mount waits only.
        if matches!(options.selection, Selection::Mount) && options.timeout.is_some_and(|timeout| timeout <= 0 || backend.elapsed() >= Duration::from_secs(timeout as u64)) {
            return CommandResult::ok(String::new()).with_warning("Wait on mount timed out\n");
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
        completion_errors: VecDeque<Option<String>>,
    }
    impl Backend for Fake {
        fn require_catalog(&mut self, _: &CommandAst) -> Result<String, String> { self.catalog.clone().map(|_| String::new()) }
        fn active_jobs(&mut self) -> Result<Vec<ActiveJob>, String> { Ok(self.snapshots.pop_front().unwrap_or_default()) }
        fn status(&mut self, id: u32) -> Result<Option<char>, String> { self.queried.push(id); self.status.clone() }
        fn completion_error(&mut self, _: Option<u32>) -> Result<Option<String>, String> { Ok(self.completion_errors.pop_front().flatten()) }
        fn sleep(&mut self, duration: Duration) { self.elapsed += duration; }
        fn elapsed(&self) -> Duration { self.elapsed }
    }
    fn job(phase: DirJobPhase) -> ActiveJob { ActiveJob { id: 42, name: "Backup".into(), unique_name: "Backup.2026-10-15_14.00.00".into(), phase, ..Default::default() } }
    fn backend(snapshots: Vec<Vec<ActiveJob>>) -> Fake {
        Fake { snapshots: snapshots.into(), elapsed: Duration::ZERO, queried: vec![], status: Ok(Some('T')), catalog: Ok(()), completion_errors: VecDeque::new() }
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
    fn completion_failure_between_snapshots_cannot_be_reported_as_success() {
        for command in ["wait", "wait jobid=42"] {
            let mut fake = backend(vec![vec![]]);
            fake.completion_errors = vec![None, Some("final update failed".into())].into();
            let result = wait_with(&crate::session::parse_cli_command(command), &mut fake);
            assert!(!result.success); assert!(result.error_message.contains("final update failed"));
            assert!(fake.queried.is_empty());
        }
    }
    #[test]
    fn invalid_input_missing_jobs_and_catalog_errors_never_claim_completion() {
        // wait_cmd: JobId 0 (no usable selector) is "Job was not found",
        // an ErrorMsg of a command that still returns true.
        for line in ["wait jobid=0","wait jobid=bad","wait jobname","wait jobid","wait foo","wait timeout=5"] {
            let mut fake=backend(vec![vec![job(DirJobPhase::Running)]]);
            let result=wait_with(&crate::session::parse_cli_command(line),&mut fake);
            assert!(result.success,"{line}"); assert_eq!(result.error_message,"ERR: Job was not found\n","{line}");
            assert_eq!(fake.snapshots.len(),1); assert!(fake.queried.is_empty());
        }
        let mut fake=backend(vec![vec![]]);
        let result=wait_with(&crate::session::parse_cli_command("wait jobname=missing"),&mut fake);
        assert!(result.success); assert_eq!(result.error_message,"ERR: Job was not found\n");
        let mut fake=backend(vec![vec![]]);fake.catalog=Err("ERR: Can't open db\n".into());
        let result=wait_with(&crate::session::parse_cli_command("wait jobid=42"),&mut fake);
        assert!(result.success); assert_eq!(result.error_message,"ERR: Can't open db\n");
        assert_eq!(fake.snapshots.len(),1);
        // The first selector decides; `mount` ignores a value; a negative
        // timeout has already expired.
        let mut fake=backend(vec![vec![job(DirJobPhase::Running)],vec![]]);
        assert!(wait_with(&crate::session::parse_cli_command("wait jobid=42 jobid=2"),&mut fake).success);
        assert_eq!(fake.queried,vec![42]);
        let mut fake=backend(vec![vec![job(DirJobPhase::WaitingMount)]]);
        assert_eq!(wait_with(&crate::session::parse_cli_command("wait mount=yes"),&mut fake).text_output,"");
        let mut fake=backend(vec![vec![]]);
        assert_eq!(wait_with(&crate::session::parse_cli_command("wait mount timeout=-1"),&mut fake).text_output,
            "Wait on mount timed out\n");
        let mut fake=backend(vec![vec![]]);fake.status=Err("status query failed".into());
        let result=wait_with(&crate::session::parse_cli_command("wait jobid=42"),&mut fake);
        assert!(!result.success);assert_eq!(result.error_message,"status query failed");
    }
}
