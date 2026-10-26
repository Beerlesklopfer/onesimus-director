//! Status commands: status, .status (C++ dird/ua_status.cc)

use bareos_config::resources::{JobLevel, JobType, RunDirective};
use serde_json::{json, Map, Value};

use crate::acl::{self, Kind};
use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;
use crate::director_config::DirectorConfig;

/// C++ GetArgValue()/FindArgWithValue(): value of the first `key=value`.
fn arg_value<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref())
}

/// C++ FindArg(): keyword present (with or without value).
fn has_arg(ast: &CommandAst, key: &str) -> bool {
    ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key))
}

/// C atoi(): optional sign, leading digits, 0 if none.
fn atoi(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let n = digits.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |acc, b| acc.saturating_mul(10).saturating_add((b - b'0') as i64));
    if neg { -n } else { n }
}

fn now() -> libc::time_t {
    unsafe { libc::time(std::ptr::null_mut()) }
}

/// lib/btime.cc bstrftime_nc(): "dd-Mon-yy hh:mm"
fn bstrftime_nc(t: libc::time_t) -> String {
    crate::localtime::strftime("%d-%b-%y %H:%M", t)
}

/// SendMsg/ErrorMsg in call order. Outside JSON mode both are plain text
/// frames; in JSON mode SendMsg text is dropped and ErrorMsg lines are
/// reported as messages.
struct Ua {
    api: ApiMode,
    text: String,
    errors: String,
    json: Map<String, Value>,
}

impl Ua {
    fn new(ctx: &ExecContext) -> Self {
        Self { api: ctx.api_mode, text: String::new(), errors: String::new(), json: Map::new() }
    }

    fn human(&self) -> bool {
        self.api == ApiMode::Human
    }

    fn send(&mut self, s: &str) {
        self.text.push_str(s);
    }

    /// OutputFormatter::Decoration(): only for the human console.
    fn decoration(&mut self, s: &str) {
        if self.human() {
            self.text.push_str(s);
        }
    }

    fn error(&mut self, s: &str) {
        if self.api == ApiMode::Json {
            self.errors.push_str(s);
        } else {
            self.text.push_str(s);
        }
    }

    fn finish(self, success: bool) -> CommandResult {
        let mut result = CommandResult::ok(self.text);
        result.success = success;
        result.error_message = self.errors;
        result.with_json(Value::Object(self.json))
    }
}

pub fn cmd_status(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ StatusCmd(): the first argument decides, every branch returns.
    let Some(first) = ast.arguments.first() else {
        return status_prompt(ast, ctx);
    };
    let key = first.key.to_ascii_lowercase();
    if key == "all" {
        status_all(ast, ctx)
    } else if key.starts_with("dir") {
        status_director(ast, ctx)
    } else if key == "client" {
        status_client_selected(ast, ctx, None)
    } else if key.starts_with("sched") {
        status_scheduler(ast, ctx)
    } else if key.starts_with("sub") {
        status_subscriptions(ast, ctx)
    } else if key.starts_with("conf") {
        status_configuration(ast, ctx)
    } else {
        status_storage_selected(ast, ctx)
    }
}

/// StatusCmd without arguments (ua_status.cc:227): "Status available for:"
/// selection; a failed DoPrompt (batch mode, `.api 2`) does nothing and
/// still succeeds.
fn status_prompt(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::ua_select;
    let mut ua = ua_select::Ua::new(ctx);
    ua.start_prompt("Status available for:\n");
    for item in ["Director", "Storage", "Client", "Scheduler", "All"] {
        ua.add_prompt(item);
    }
    let Some((item, _)) = ua.do_prompt("", "Select daemon type for status") else {
        return ua.finish(true);
    };
    let config = || ctx.config.as_ref().and_then(|c| c.read().ok());
    match item {
        0 => ua.send_result(status_director(ast, ctx)),
        1 => {
            let store = ua_select::select_storage_resource(&mut ua, false)
                .and_then(|name| config().and_then(|cfg| cfg.storages.iter().find(|s| s.name == name).cloned()));
            if let Some(store) = store {
                ua.send_msg(storage_status(&store, None, ctx));
            }
        }
        2 => {
            let found = ua_select::select_client_resource(&mut ua).and_then(|name| config().and_then(|cfg| {
                Some((cfg.clients.iter().find(|c| c.name == name)?.clone(), cfg.director.name.clone()))
            }));
            if let Some((client, director)) = found {
                ua.send_msg(client_status(&client, None, ctx, &director));
            }
        }
        3 => ua.send_result(status_scheduler(ast, ctx)),
        _ => ua.send_result(status_all(ast, ctx)),
    }
    ua.finish(true)
}

/// C++ DotStatusCmd(): `.status dir header|scheduled|running|terminated|
/// current|last`, `.status client=…`, `.status storage=…`.
pub fn cmd_dot_status(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let arg = |i: usize| ast.arguments.get(i).map(|a| a.key.to_ascii_lowercase());
    let bad = |msg: &str| {
        let mut ua = Ua::new(ctx);
        ua.send(msg);
        ua.finish(false)
    };
    let Some(first) = arg(0) else {
        return bad("1900 Bad .status command, missing arguments.\n");
    };
    match first.as_str() {
        "dir" => {
            let mut ua = Ua::new(ctx);
            match arg(1).as_deref() {
                Some("current") => dot_status_current(&mut ua, ctx),
                Some("last") => dot_status_last(&mut ua, ctx),
                Some("header") => return dir_status_header(ctx),
                Some("scheduled") => list_scheduled_jobs(&mut ua, ast, ctx),
                Some("running") => list_running_jobs(&mut ua, ctx),
                Some("terminated") => list_terminated_jobs(&mut ua, ctx),
                _ => return bad("1900 Bad .status command, wrong argument.\n"),
            }
            ua.finish(true)
        }
        // `if (ua->argc == 3) statuscmd = ua->argk[2]`
        "client" => status_client_selected(ast, ctx, dot_status_cmd(ast)),
        "storage" => dot_status_storage(ast, ctx, dot_status_cmd(ast)),
        _ => bad("1900 Bad .status command, wrong argument.\n"),
    }
}

/// C++ DoAllStatus(): Director, every Storage daemon and every File daemon
/// once per address/port.
fn status_all(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut result = status_director(ast, ctx);
    let (storages, clients, director) = match ctx.config.as_ref().and_then(|c| c.read().ok()) {
        Some(cfg) => {
            let mut storages: Vec<bareos_config::resources::StorageResource> = Vec::new();
            for s in cfg.storages.iter().filter(|s| acl::access(ctx, Kind::Storage, &s.name)) {
                if !storages.iter().any(|u| u.address == s.address && u.port == s.port) {
                    storages.push(s.clone());
                }
            }
            let mut clients: Vec<bareos_config::resources::ClientResource> = Vec::new();
            for c in cfg.clients.iter().filter(|c| acl::access(ctx, Kind::Client, &c.name)) {
                if !clients.iter().any(|u| u.address == c.address && u.port == c.port) {
                    clients.push(c.clone());
                }
            }
            (storages, clients, cfg.director.name.clone())
        }
        None => (Vec::new(), Vec::new(), String::new()),
    };
    for store in &storages {
        result.text_output.push_str(&storage_status(store, None, ctx));
    }
    for client in &clients {
        result.text_output.push_str(&client_status(client, None, ctx, &director));
    }
    result
}

/// DotStatusCmd(): `if (ua->argc == 3) statuscmd = ua->argk[2]`.
fn dot_status_cmd(ast: &CommandAst) -> Option<&str> {
    (ast.arguments.len() == 2).then(|| ast.arguments[1].key.as_str())
}

// ---------------------------------------------------------------------------
// status director
// ---------------------------------------------------------------------------

/// C++ DoDirectorStatus().
fn status_director(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let header = dir_status_header(ctx);
    let mut ua = Ua::new(ctx);
    ua.send(&header.text_output);
    if let Some(Value::Object(h)) = header.json {
        ua.json.extend(h);
    }
    list_scheduled_jobs(&mut ua, ast, ctx);
    list_running_jobs(&mut ua, ctx);
    list_terminated_jobs(&mut ua, ctx);
    list_connected_clients(&mut ua, ctx);
    ua.send("====\n");
    ua.finish(true)
}

/// Scheduling packet (ua_status.cc sched_pkt).
struct SchedPkt {
    job: String,
    job_type: JobType,
    level: JobLevel,
    priority: i32,
    runtime: libc::time_t,
    /// Run directive pool override
    run_pool: Option<String>,
    /// Pool the job would use (override or Job Pool)
    pool: String,
    store: String,
}

fn job_type_str(t: JobType) -> &'static str {
    match t {
        JobType::Backup => "Backup",
        JobType::Restore => "Restore",
        JobType::Verify => "Verify",
        JobType::Admin => "Admin",
        JobType::Archive => "Archive",
        JobType::Copy => "Copy",
        JobType::Migrate => "Migrate",
        JobType::Consolidate => "Consolidate",
    }
}

/// dird_conf.cc JobLevelToString(): the joblevels[] names.
fn job_level_str(level: JobLevel) -> &'static str {
    match level {
        JobLevel::Full => "Full",
        JobLevel::Incremental => "Incremental",
        JobLevel::Differential => "Differential",
        JobLevel::VirtualFull => "VirtualFull",
        JobLevel::InitCatalog => "InitCatalog",
        JobLevel::Catalog => "Catalog",
        JobLevel::VolumeToCatalog => "VolumeToCatalog",
        JobLevel::DiskToCatalog => "DiskToCatalog",
        JobLevel::Data => "Data",
    }
}

/// Job and client enabled (`job->enabled && !(job->client && !client->enabled)`).
fn job_enabled(cfg: &DirectorConfig, job: &bareos_config::resources::JobResource) -> bool {
    job.enabled && !cfg.clients.iter().any(|c| c.name == job.client && !c.enabled)
}

/// C++ ListScheduledJobs(): next run of every Run directive within `days`
/// (default 1) for the enabled jobs, sorted by run time and priority.
fn list_scheduled_jobs(ua: &mut Ua, ast: &CommandAst, ctx: &ExecContext) {
    let mut days: i64 = 1;
    if let Some(value) = arg_value(ast, "days") {
        days = atoi(value);
        if (!(0..=500).contains(&days)) && ua.human() {
            ua.send("Ignoring invalid value for days. Max is 500.\n");
            days = 1;
        }
    }
    // C++ casts a negative API value to uint32 and loops for ages.
    let days = days.clamp(0, 500) as u32;

    let mut sched = Vec::new();
    let now = now();
    if let Some(cfg) = ctx.config.as_ref().and_then(|c| c.read().ok()) {
        for job in &cfg.jobs {
            if !acl::access(ctx, Kind::Job, &job.name) || !job_enabled(&cfg, job) {
                continue;
            }
            let Some(schedule) = cfg.schedules.iter().find(|s| s.name == job.schedule) else { continue };
            for run in &schedule.runs {
                let Some(runtime) = crate::run_time::next_schedule_time(run, now, days) else { continue };
                if sched.is_empty() && ua.human() {
                    ua.send("\nScheduled Jobs:\n");
                    ua.send("Level          Type     Pri  Scheduled          Name               Volume\n");
                    ua.send(&format!("{}\n", "=".repeat(83)));
                }
                sched.push(SchedPkt {
                    job: job.name.clone(),
                    job_type: job.job_type,
                    level: run.level.unwrap_or(job.level),
                    priority: run.priority.filter(|p| *p != 0).unwrap_or(job.priority),
                    runtime,
                    run_pool: run.pool.clone(),
                    pool: run.pool.clone().unwrap_or_else(|| job.pool.clone()),
                    store: run.storage.clone().unwrap_or_else(|| job.storage.clone()),
                });
            }
        }
    }
    sched.sort_by_key(|p| (p.runtime, p.priority));

    let mut rows = Vec::new();
    for sp in &sched {
        let volume = if sp.job_type == JobType::Backup {
            next_volume(ctx, &sp.pool, &sp.store)
        } else {
            String::new()
        };
        let dt = bstrftime_nc(sp.runtime);
        let level = match sp.job_type {
            JobType::Admin | JobType::Archive | JobType::Restore => " ",
            _ => job_level_str(sp.level),
        };
        let job_type = job_type_str(sp.job_type);
        match ua.api {
            ApiMode::Json => {
                let mut row = json!({
                    "name": sp.job, "type": job_type, "level": level,
                    "priority": sp.priority, "scheduled": dt, "volume": volume,
                });
                if let Some(pool) = &sp.run_pool {
                    row["pool"] = json!(pool);
                }
                if !sp.store.is_empty() {
                    row["storage"] = json!(sp.store);
                }
                rows.push(row);
            }
            ApiMode::Api => ua.send(&format!("{level:<14}\t{job_type:<8}\t{:3}\t{dt:<18}\t{:<18}\t{volume}\n",
                                             sp.priority, sp.job)),
            ApiMode::Human => ua.send(&format!("{level:<14} {job_type:<8} {:3}  {dt:<18} {:<18} {volume}\n",
                                               sp.priority, sp.job)),
        }
    }
    if ua.api == ApiMode::Json {
        ua.json.insert("scheduled".into(), Value::Array(rows));
    }
    if sched.is_empty() && ua.human() {
        ua.send("No Scheduled Jobs.\n");
    }
    if ua.human() {
        ua.send("====\n");
    }
}

/// Local time of a catalog timestamp ("YYYY-MM-DD HH:MM:SS"), C++ StrToUtime.
fn db_time(s: Option<&str>) -> i64 {
    crate::btime::str_to_utime_opt(s)
}

/// Volume column of a scheduled backup: C++ PrtRuntime() →
/// FindNextVolumeForAppend(jcr, mr, 1, NULL, fnv_no_create_vol,
/// fnv_no_prune) (next_vol.cc), read-only: an expired Append volume is
/// skipped instead of being marked Used, a Purged volume is reported
/// without being recycled. "*unknown*" if nothing is found.
fn next_volume(ctx: &ExecContext, pool: &str, store: &str) -> String {
    const UNKNOWN: &str = "*unknown*";
    let Some(catalog) = &ctx.catalog else { return UNKNOWN.into() };
    let media_type = ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|cfg| cfg.storages.iter().find(|s| s.name == store).map(|s| s.media_type.clone()));
    let Some(media_type) = media_type else { return UNKNOWN.into() };
    let esc = |s: &str| s.replace('\'', "''");
    let pool_id = catalog.query_rows(&format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(pool))).ok()
        .and_then(|t| t.rows.first().and_then(|r| r[0].clone()));
    let Some(pool_id) = pool_id else { return UNKNOWN.into() };

    let mut skipped: Vec<String> = Vec::new();
    for _ in 0..200 {
        let find = |status: &str| {
            let order = if status == "Recycle" || status == "Purged" {
                "AND Recycle=1 ORDER BY LastWritten ASC,MediaId"
            } else {
                "ORDER BY LastWritten IS NULL,LastWritten DESC,MediaId"
            };
            let not_in = if skipped.is_empty() { String::new() } else { format!("AND MediaId NOT IN ({}) ", skipped.join(",")) };
            let sql = format!(
                "SELECT MediaId,VolumeName,VolStatus,VolJobs,VolFiles,VolBytes,MaxVolBytes,MaxVolJobs,\
                 MaxVolFiles,VolUseDuration,FirstWritten FROM Media WHERE PoolId={pool_id} AND \
                 MediaType='{}' AND Enabled=1 AND VolStatus='{status}' {not_in}{order} LIMIT 1",
                esc(&media_type));
            catalog.query_rows(&sql).ok().and_then(|t| t.rows.into_iter().next())
        };
        let Some(row) = ["Append", "Unlabeled", "Recycle", "Purged"].iter().find_map(|s| find(s)) else {
            return UNKNOWN.into();
        };
        let num = |i: usize| row[i].as_deref().map_or(0, atoi);
        let (vol_jobs, vol_files, vol_bytes) = (num(3), num(4), num(5));
        let (max_bytes, max_jobs, max_files, use_duration) = (num(6), num(7), num(8), num(9));
        let expired = row[2].as_deref() == Some("Append") && vol_jobs > 0
            && ((max_bytes > 0 && vol_bytes >= max_bytes)
                || (max_jobs > 0 && max_jobs <= vol_jobs)
                || (max_files > 0 && max_files <= vol_files)
                || (use_duration > 0 && use_duration <= now() as i64 - db_time(row[10].as_deref())));
        if expired {
            skipped.push(row[0].clone().unwrap_or_default());
            continue;
        }
        return row[1].clone().unwrap_or_default();
    }
    UNKNOWN.into()
}

/// C++ ListRunningJobs().
fn list_running_jobs(ua: &mut Ua, ctx: &ExecContext) {
    if ua.human() {
        ua.send("\nRunning Jobs:\n");
        if let Some(smgr) = &ctx.session_mgr {
            for t in smgr.console_connect_times() {
                ua.send(&format!("Console connected at {}\n", bstrftime_nc(t as libc::time_t)));
            }
        }
    }
    let jobs = ctx.job_manager.as_ref()
        .map(|m| m.lock().unwrap().active_jobs())
        .unwrap_or_default();
    if jobs.is_empty() {
        if ua.human() {
            ua.send("No Jobs running.\n====\n");
        }
        if ua.api == ApiMode::Json {
            ua.json.insert("running".into(), json!([]));
        }
        return;
    }
    if ua.human() {
        ua.send(" JobId Level   Name                       Status\n");
        ua.send(&format!("{}\n", "=".repeat(70)));
    }
    let cfg = ctx.config.as_ref().and_then(|c| c.read().ok().map(|c| c.clone()));
    let mut rows = Vec::new();
    for job in jobs.iter().filter(|j| acl::access(ctx, Kind::Job, &j.name)) {
        let msg = running_job_status(job);
        let res = cfg.as_ref().and_then(|c| c.jobs.iter().find(|j| j.name == job.name).cloned());
        let level = match res.as_ref().map(|r| r.job_type) {
            Some(JobType::Admin | JobType::Archive | JobType::Restore) => "      ".to_string(),
            _ => res.as_ref().map_or("", |r| job_level_str(r.level)).chars().take(7).collect(),
        };
        let unique = if job.unique_name.is_empty() { &job.name } else { &job.unique_name };
        match ua.api {
            ApiMode::Json => {
                let job_type = res.as_ref().map_or("Backup", |r| job_type_str(r.job_type));
                rows.push(json!({ "jobid": job.id, "name": unique, "level": level, "type": job_type,
                                  "status": msg, "files": 0, "bytes": 0 }));
            }
            ApiMode::Api => ua.send(&format!("{:6}\t{level:<6}\t{unique:<20}\t{msg}\t\n", job.id)),
            ApiMode::Human => ua.send(&format!("{:6} {level:<6}  {unique:<20} {msg}\n", job.id)),
        }
    }
    match ua.api {
        ApiMode::Json => { ua.json.insert("running".into(), Value::Array(rows)); }
        ApiMode::Human => ua.send("====\n"),
        ApiMode::Api => {}
    }
}

/// ListRunningJobs(): the status text of a job from its JobStatus and the
/// Storage daemon's status (Compatibility log C-048).
fn running_job_status(job: &crate::job_manager::ActiveJob) -> String {
    let client = (!job.client.is_empty()).then_some(job.client.as_str());
    let storage = (!job.storage.is_empty()).then_some(job.storage.as_str());
    let mut msg = match job.status {
        'C' => "is waiting execution".to_string(),
        'R' => "is running".into(),
        'B' => "is blocked".into(),
        'T' => "has terminated".into(),
        'W' => "has terminated with warnings".into(),
        'E' => "has erred".into(),
        'e' => "has errors".into(),
        'f' => "has a fatal error".into(),
        'D' => "has verify differences".into(),
        'A' => "has been canceled".into(),
        'F' => client.map_or("is waiting on Client".into(), |c| format!("is waiting on Client {c}")),
        'S' => storage.map_or("is waiting on Storage".into(), |s| format!("is waiting on Storage \"{s}\"")),
        's' => "is waiting on max Storage jobs".into(),
        'c' => "is waiting on max Client jobs".into(),
        'j' => "is waiting on max Job jobs".into(),
        'd' => "is waiting on max total jobs".into(),
        't' if job.sched_time != 0 => format!("is waiting for its start time at {}",
            bstrftime_nc(job.sched_time as libc::time_t)),
        't' => "is waiting for its start time".into(),
        'p' => "is waiting for higher priority jobs to finish".into(),
        'L' => "SD committing Data".into(),
        'l' => "SD despooling Data".into(),
        'a' => "SD despooling Attributes".into(),
        'i' => "Dir inserting Attributes".into(),
        other => format!("is in unknown state {other}"),
    };
    match job.sd_status {
        'M' => msg = "is waiting for a mount request".into(),
        'm' => msg = "is waiting for an appendable Volume".into(),
        'F' => msg = if !job.fd_connected {
            "is waiting for Client to connect (Client Initiated Connection)".into()
        } else if let (Some(c), Some(s)) = (client, storage) {
            format!("is waiting for Client {c} to connect to Storage {s}")
        } else {
            "is waiting for Client to connect to Storage daemon".into()
        },
        'L' => msg = "SD committing Data".into(),
        'l' => msg = "SD despooling Data".into(),
        'a' => msg = "SD despooling Attributes".into(),
        'i' => msg = "Dir inserting Attributes".into(),
        _ => {}
    }
    msg
}

/// lib/edit.cc edit_uint64_with_suffix(): "126.3 M", "0 ".
pub(crate) fn edit_uint64_with_suffix(v: u64) -> String {
    const SUFFIX: [&str; 10] = ["", "K", "M", "G", "T", "P", "E", "Z", "Y", "FIX ME"];
    let with_commas = crate::sql_list::add_commas(&v.to_string());
    let commas = with_commas.matches(',').count().min(SUFFIX.len() - 1);
    let mut num = match with_commas.split_once(',') {
        Some((head, tail)) => format!("{head}.{}", tail.split(',').next().unwrap_or("")),
        None => with_commas.clone(),
    };
    if commas > 0 {
        num.truncate(5);
    }
    format!("{num} {}", SUFFIX[commas])
}

/// C++ ListTerminatedJobs() over the jobs that ended since the Director
/// started (RecentJobResultsList, at most 10 kept).
fn list_terminated_jobs(ua: &mut Ua, ctx: &ExecContext) {
    let mut ids = ctx.job_manager.as_ref()
        .map(|m| m.lock().unwrap().finished_job_ids())
        .unwrap_or_default();
    if ids.len() > 10 {
        ids.drain(..ids.len() - 10);
    }
    let rows = match (&ctx.catalog, ids.is_empty()) {
        (Some(catalog), false) => {
            let list = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            catalog.query_rows(&format!(
                "SELECT JobId,Job,Type,Level,JobStatus,JobFiles,JobBytes,EndTime FROM Job WHERE JobId IN ({list})"))
                .map(|t| t.rows).unwrap_or_default()
        }
        _ => Vec::new(),
    };
    let mut jobs: Vec<&Vec<Option<String>>> = ids.iter()
        .filter_map(|id| rows.iter().find(|r| r[0].as_deref() == Some(&id.to_string())))
        .collect();
    if jobs.is_empty() {
        if ua.human() {
            ua.send("No Terminated Jobs.\n");
        }
        if ua.api == ApiMode::Json {
            ua.json.insert("terminated".into(), json!([]));
        }
        return;
    }
    if ua.human() {
        ua.send("\nTerminated Jobs:\n");
        ua.send(" JobId  Level    Files      Bytes   Status   Finished        Name \n");
        ua.send(&format!("{}\n", "=".repeat(68)));
    }
    let mut out = Vec::new();
    for row in jobs.drain(..) {
        let col = |i: usize| row[i].as_deref().unwrap_or("");
        // There are three periods after the Job name
        let mut name = col(1).to_string();
        for _ in 0..3 {
            if let Some(p) = name.rfind('.') {
                name.truncate(p);
            }
        }
        if !acl::access(ctx, Kind::Job, &name) {
            continue;
        }
        let dt = bstrftime_nc(db_time(row[7].as_deref()) as libc::time_t);
        let level: String = match col(2) {
            "D" | "A" | "R" => "    ".into(), // Admin, Archive, Restore
            _ => level_code_name(col(3)).chars().take(4).collect(),
        };
        let termstat = match col(4) {
            "C" => "Created",
            "f" | "E" => "Error",
            "D" => "Diffs",
            "A" => "Cancel",
            "T" => "OK",
            "W" => "OK -- with warnings",
            _ => "Other",
        };
        let (files, bytes) = (atoi(col(5)) as u64, atoi(col(6)) as u64);
        let jobid = atoi(col(0));
        match ua.api {
            ApiMode::Json => out.push(json!({
                "jobid": jobid, "name": name, "level": level, "type": job_type_code_name(col(2)),
                "status": termstat, "files": files, "bytes": bytes, "finished": dt,
            })),
            ApiMode::Api => ua.send(&format!("{jobid:6}\t{level:<6}\t{:>8}\t{:>10}\t{termstat:<7}\t{dt:<8}\t{name}\n",
                crate::sql_list::add_commas(&files.to_string()), edit_uint64_with_suffix(bytes))),
            ApiMode::Human => ua.send(&format!("{jobid:6}  {level:<6} {:>8} {:>10}  {termstat:<7}  {dt:<8} {name}\n",
                crate::sql_list::add_commas(&files.to_string()), edit_uint64_with_suffix(bytes))),
        }
    }
    match ua.api {
        ApiMode::Json => { ua.json.insert("terminated".into(), Value::Array(out)); }
        ApiMode::Human => ua.send("\n"),
        ApiMode::Api => {}
    }
}

/// Catalog level code to JobLevelToString() name.
fn level_code_name(code: &str) -> &'static str {
    match code {
        "F" => "Full",
        "B" => "Base",
        "I" => "Incremental",
        "D" => "Differential",
        "S" => "Since",
        "f" => "VirtualFull",
        "C" => "Catalog",
        "V" => "InitCatalog",
        "O" => "VolumeToCatalog",
        "d" => "DiskToCatalog",
        "A" => "Data",
        " " | "" => " ",
        _ => "?",
    }
}

/// Catalog job type code to job_type_to_str().
fn job_type_code_name(code: &str) -> &'static str {
    match code {
        "B" => "Backup",
        "M" => "Migrated Job",
        "V" => "Verify",
        "R" => "Restore",
        "U" => "Console",
        "I" => "System or Console",
        "D" => "Admin",
        "A" => "Archive",
        "C" => "Job Copy",
        "c" => "Copy",
        "g" => "Migrate",
        "S" => "Scan",
        "O" => "Consolidate",
        _ => "Unknown Type",
    }
}

/// C++ ListConnectedClients(): client initiated connections.
fn list_connected_clients(ua: &mut Ua, ctx: &ExecContext) {
    let sep = "=".repeat(20);
    ua.decoration("\n");
    ua.decoration("Client Initiated Connections (waiting for jobs):\n");
    ua.decoration(&format!("{:<20}{:<20}{:<20}{:<40}\n", "Connect time", "Protocol", "Authenticated", "Name"));
    ua.decoration(&format!("{sep}{sep}{sep}{sep}{sep}\n"));
    let mut rows = Vec::new();
    for fd in ctx.session_mgr.as_ref().map(|s| s.get_fd_connections()).unwrap_or_default() {
        let dt = bstrftime_nc(fd.connect_time as libc::time_t);
        match ua.api {
            ApiMode::Json => rows.push(json!({
                "connecttime": dt, "protocol_version": fd.protocol_version,
                "authenticated": true, "name": fd.client_name,
            })),
            _ => {
                ua.send(&format!("{dt:<20}{:<20}{:<20}{:<40}", fd.protocol_version, 1, fd.client_name));
                ua.decoration("\n");
            }
        }
    }
    if ua.api == ApiMode::Json {
        ua.json.insert("client-connection".into(), Value::Array(rows));
    }
}

/// `.status dir current`: running jobs as `JobId=… JobStatus=… JobErrors=…`.
fn dot_status_current(ua: &mut Ua, ctx: &ExecContext) {
    ua.send("1000 OK .status\n");
    let jobs = ctx.job_manager.as_ref().map(|m| m.lock().unwrap().active_jobs()).unwrap_or_default();
    for job in jobs.iter().filter(|j| acl::access(ctx, Kind::Job, &j.name)) {
        ua.send(&format!("JobId={} JobStatus={} JobErrors=0\n", job.id, job.status));
    }
}

/// `.status dir last`: the most recently terminated job.
fn dot_status_last(ua: &mut Ua, ctx: &ExecContext) {
    ua.send("1000 OK .status\n");
    let last = ctx.job_manager.as_ref().and_then(|m| m.lock().unwrap().finished_job_ids().last().copied());
    let (Some(id), Some(catalog)) = (last, &ctx.catalog) else { return };
    let Ok(t) = catalog.query_rows(&format!("SELECT Job,JobStatus,JobErrors FROM Job WHERE JobId={id}")) else { return };
    let Some(row) = t.rows.first() else { return };
    let col = |i: usize| row[i].clone().unwrap_or_default();
    // C++ checks the ACL against the unique Job name (ua_status.cc:129).
    if acl::access(ctx, Kind::Job, &col(0)) {
        ua.send(&format!("JobId={id} JobStatus={} JobErrors={}\n", col(1), atoi(&col(2))));
    }
}

// ---------------------------------------------------------------------------
// status scheduler
// ---------------------------------------------------------------------------

const DEFAULT_STATUS_SCHED_DAYS: i64 = 7;
const SECONDS_PER_DAY: i64 = 86400;
const SECONDS_PER_HOUR: i64 = 3600;

/// Override list of a preview line (show_scheduled_preview).
fn run_overrides(run: &RunDirective) -> String {
    let mut parts = Vec::new();
    if let Some(level) = run.level {
        parts.push(format!("Level={}", job_level_str(level)));
    }
    if let Some(p) = run.priority.filter(|p| *p != 0) {
        parts.push(format!("Priority={p}"));
    }
    if let Some(s) = run.spool_data {
        parts.push(format!("Spool Data={}", s as i32));
    }
    if let Some(a) = run.accurate {
        parts.push(format!("Accurate={}", a as i32));
    }
    if let Some(pool) = &run.pool {
        parts.push(format!("Pool={pool}"));
    }
    if let Some(storage) = &run.storage {
        parts.push(format!("Storage={storage}"));
    }
    if let Some(msgs) = &run.messages {
        parts.push(format!("Messages={msgs}"));
    }
    parts.join(" ")
}

/// C++ DoSchedulerStatus(): which schedule triggers which jobs, and a
/// preview of the run times (`days=N` or `days=FROM,TO`, `schedule=`,
/// `client=`, `job=`).
fn status_scheduler(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = Ua::new(ctx);
    let Some(cfg) = ctx.config.as_ref().and_then(|c| c.read().ok().map(|c| c.clone())) else {
        return ua.finish(true);
    };
    let mut days = DEFAULT_STATUS_SCHED_DAYS;
    let (mut json_from, mut json_to) = (0, DEFAULT_STATUS_SCHED_DAYS);
    if let Some(val) = arg_value(ast, "days") {
        if let Some((from, to)) = val.split_once(',') {
            // Range format: days=FROM,TO (both relative to now)
            json_from = atoi(from);
            json_to = atoi(to);
            if json_from > json_to {
                std::mem::swap(&mut json_from, &mut json_to);
            }
            if ua.api == ApiMode::Human && (json_from < -366 || json_to > 366) {
                ua.send("Ignoring invalid days range. Allowed: -366..366.\n");
                json_from = 0;
                json_to = DEFAULT_STATUS_SCHED_DAYS;
            }
            days = json_from.abs().max(json_to.abs());
        } else {
            days = atoi(val);
            if !(-366..=366).contains(&days) && ua.api == ApiMode::Human {
                ua.send("Ignoring invalid value for days. Allowed is -366 < days < 366.\n");
                days = DEFAULT_STATUS_SCHED_DAYS;
            }
            (json_from, json_to) = if days >= 0 { (0, days) } else { (days, 0) };
        }
    }
    let schedule_name = arg_value(ast, "schedule");
    // A client or job that is not found would be asked for interactively
    // (compatibility log C-011); it is ignored here.
    let client = arg_value(ast, "client")
        .and_then(|name| cfg.clients.iter().find(|c| c.name == name && acl::access(ctx, Kind::Client, &c.name)));
    let job = arg_value(ast, "job")
        .and_then(|name| cfg.jobs.iter().find(|j| j.name == name && acl::access(ctx, Kind::Job, &j.name)));
    let schedule_of = |name: &str| cfg.schedules.iter().find(|s| s.name == name);
    let sched_ok = |s: &bareos_config::resources::ScheduleResource| {
        s.enabled && acl::access(ctx, Kind::Schedule, &s.name) && schedule_name.is_none_or(|n| n == s.name)
    };

    // Schedule -> jobs
    let mut overview: Vec<(String, Vec<(String, bool)>)> = Vec::new();
    for sched in cfg.schedules.iter().filter(|s| sched_ok(s)) {
        let jobs: Vec<(String, bool)> = match job {
            Some(job) => (job.schedule == sched.name).then(|| (job.name.clone(), job_enabled(&cfg, job))).into_iter().collect(),
            None => cfg.jobs.iter()
                .filter(|j| acl::access(ctx, Kind::Job, &j.name))
                .filter(|j| client.is_none_or(|c| j.client == c.name))
                .filter(|j| j.schedule == sched.name)
                .map(|j| (j.name.clone(), job_enabled(&cfg, j)))
                .collect(),
        };
        if !jobs.is_empty() {
            overview.push((sched.name.clone(), jobs));
        }
    }

    // Runs in the preview, per checked hour in output order
    // Schedules in the preview, in output order per checked hour
    let mut previewed: Vec<&bareos_config::resources::ScheduleResource> = Vec::new();
    if let Some(job) = job {
        if let Some(s) = schedule_of(&job.schedule).filter(|s| s.enabled && job.enabled) {
            if job_enabled(&cfg, job) {
                previewed.push(s);
            }
        }
    } else if let Some(client) = client {
        // C++ previews every job here (a quirk: only a disabled client of
        // the job's own is skipped).
        for j in cfg.jobs.iter().filter(|j| acl::access(ctx, Kind::Job, &j.name)) {
            let Some(s) = schedule_of(&j.schedule).filter(|s| s.enabled && j.enabled) else { continue };
            if j.client == client.name && !client.enabled {
                continue;
            }
            previewed.push(s);
        }
    } else {
        previewed.extend(cfg.schedules.iter().filter(|s| sched_ok(s)));
    }
    let runs_at = |t: libc::time_t| -> Vec<(&str, &RunDirective)> {
        previewed.iter()
            .flat_map(|s| s.runs.iter().map(move |run| (s.name.as_str(), run)))
            .filter(|(_, run)| crate::run_time::triggers_on_day_and_hour(&run.mask, t))
            .collect()
    };

    let now = now() as i64;
    if ua.api == ApiMode::Json {
        let schedules: Vec<Value> = cfg.schedules.iter().filter(|s| sched_ok(s))
            .filter_map(|s| overview.iter().find(|o| o.0 == s.name))
            .map(|(name, jobs)| json!({
                "name": name,
                "enabled": true,
                "jobs": jobs.iter().map(|(j, e)| json!({ "name": j, "enabled": e })).collect::<Vec<_>>(),
            }))
            .collect();
        let mut preview = Vec::new();
        let mut t = now + json_from * SECONDS_PER_DAY;
        while t < now + json_to * SECONDS_PER_DAY {
            for (sched, run) in runs_at(t as libc::time_t) {
                let runtime = crate::run_time::run_time_in_hour(run, t as libc::time_t);
                let mut row = json!({
                    "datetime": crate::localtime::strftime("%a %d-%b-%Y %H:%M", runtime),
                    "runtime": runtime as i64,
                    "schedule": sched,
                });
                if let Some(level) = run.level {
                    row["level"] = json!(job_level_str(level));
                }
                if let Some(p) = run.priority.filter(|p| *p != 0) {
                    row["priority"] = json!(p);
                }
                if let Some(pool) = &run.pool {
                    row["pool"] = json!(pool);
                }
                if let Some(storage) = &run.storage {
                    row["storage"] = json!(storage);
                }
                if let Some(msgs) = &run.messages {
                    row["messages"] = json!(msgs);
                }
                if let Some(s) = run.spool_data {
                    row["spool_data"] = json!(s);
                }
                if let Some(a) = run.accurate {
                    row["accurate"] = json!(a);
                }
                preview.push(row);
            }
            t += SECONDS_PER_HOUR;
        }
        ua.json.insert("schedules".into(), Value::Array(schedules));
        ua.json.insert("preview".into(), Value::Array(preview));
        return ua.finish(true);
    }

    ua.send("Scheduler Jobs:\n\n");
    ua.send("Schedule               Jobs Triggered\n");
    ua.send("===========================================================\n");
    for (name, jobs) in &overview {
        ua.send(&format!("{name}\n"));
        for (job, enabled) in jobs {
            let disabled = if *enabled { "" } else { " (disabled)" };
            ua.send(&format!("                       {job}{disabled}\n"));
        }
        ua.send("\n");
    }

    // Build the overview hour by hour
    let (start, stop) = if days > 0 {
        (now, now + days * SECONDS_PER_DAY)
    } else {
        (now + days * SECONDS_PER_DAY, now)
    };
    let mut max_date_len = 0usize;
    let mut lines = String::new();
    'start_again: loop {
        lines.clear();
        let mut t = start;
        while t < stop {
            for (sched, run) in runs_at(t as libc::time_t) {
                let dt = crate::localtime::strftime("%a %d-%b-%Y %H:%M",
                    crate::run_time::run_time_in_hour(run, t as libc::time_t));
                if dt.len() > max_date_len {
                    let first = max_date_len == 0;
                    max_date_len = dt.len();
                    // A changing date length restarts once from scratch.
                    if first {
                        continue 'start_again;
                    }
                }
                let name: String = sched.chars().take(22).collect();
                lines.push_str(&format!("{dt:<w$}  {name:<22}  {}\n", run_overrides(run), w = max_date_len));
            }
            t += SECONDS_PER_HOUR;
        }
        break;
    }
    ua.send("====\n\n");
    ua.send(&format!("Scheduler Preview for {days} days:\n\n"));
    ua.send(&format!("{:<w$}  {:<22}  {}\n", "Date", "Schedule", "Overrides", w = max_date_len));
    ua.send("==============================================================\n");
    ua.send(&lines);
    ua.send("====\n");
    ua.finish(true)
}

// ---------------------------------------------------------------------------
// status subscriptions
// ---------------------------------------------------------------------------

/// cats subscription_with_clause_0 (PostgreSQL).
const SUBSCRIPTION_WITH: &str = include_str!("subscription_with.sql");

/// cats subscription_units_3: (with, client column, order column)
fn subscription_units(client_col: &str, order_col: &str) -> String {
    format!("{SUBSCRIPTION_WITH}\nSELECT {client_col} as client, plugin, count, size_gb::NUMERIC(20,2) FROM (
  SELECT
    client_name,
    client_anon,
    client_id,
    plugin,
    SUM(count) AS count,
    SUM(size_gb) AS size_gb,
    1 AS order
  FROM client_aggregated
  GROUP BY client_name, client_anon, client_id, plugin
  UNION
  SELECT
    'TOTAL' AS client_name,
    'TOTAL' AS client_anon,
    0 as client_id,
    NULL AS plugin,
    SUM(count) AS count,
    SUM(size_gb) AS size_gb,
    2 AS order
  FROM client_aggregated
) t
ORDER BY t.order, t.{order_col}, t.plugin;
")
}

/// cats subscription_units_client_total_3
fn subscription_units_client_total(client_col: &str, order_col: &str) -> String {
    format!("{SUBSCRIPTION_WITH}\nSELECT {client_col} as client, count, size_gb::NUMERIC(20,2) FROM (
  SELECT
    client_name,
    client_anon,
    client_id,
    SUM(count) AS count,
    SUM(size_gb) AS size_gb,
    1 AS order
  FROM client_aggregated
  GROUP BY client_name, client_anon, client_id
  UNION
  SELECT
    'TOTAL' AS client_name,
    'TOTAL' AS client_anon,
    0 as client_id,
    SUM(count) AS count,
    SUM(size_gb) AS size_gb,
    2 AS order
  FROM client_aggregated
) t
ORDER BY t.order, t.{order_col};
")
}

/// cats subscription_units_plugin_total_1
fn subscription_units_plugin_total() -> String {
    format!("{SUBSCRIPTION_WITH}\nSELECT
  plugin,
  count,
  size_gb::NUMERIC(20,2)
FROM (
  SELECT
    plugin,
    count,
    size_gb,
    1 AS order 
  FROM plugin_aggregated
  UNION
  SELECT
    'TOTAL' as plugin,
    SUM(count) as count,
    SUM(size_gb) AS size_gb,
    2 AS order
  FROM plugin_aggregated
) t
ORDER BY t.order, t.plugin;
")
}

/// cats subscription_units_total_2
fn subscription_units_total(configured: i64) -> String {
    format!("{SUBSCRIPTION_WITH}\nSELECT
  CASE
    WHEN t.size_units >= t.count_units THEN 'volume-based (' || t.size_units || ' volume-based units >= ' || t.count_units || ' count-based units)'
    ELSE 'count-based (' || t.size_units || ' volume-based < ' || t.count_units || ' count-based)'
  END AS accounting_mode,
  GREATEST(t.count_units, t.size_units) AS used,
  configured_units as configured,
  configured_units - GREATEST(t.count_units, t.size_units) as remaining
FROM (
  SELECT
    COALESCE(SUM(count), 0) AS count_units,
    CEIL(COALESCE(SUM(size_gb)/1000, 0.0)) AS size_units,
    {configured} AS configured_units
  FROM client_aggregated
) t
")
}

/// cats subscription_client_detail_2
fn subscription_client_detail(client: &str) -> String {
    format!("{SUBSCRIPTION_WITH}\nSELECT
  fileset,
  COALESCE(plugin, 'none (file data)') as plugin,
  size_gb::NUMERIC(20,2)
FROM
  client_detail
WHERE
  client_name = '{client}'
ORDER BY
  fileset;
")
}

/// lib/crypto_openssl.cc compute_hash(): "{SHA256}" and the digest in hex,
/// only the first byte padded to two digits (std::setw applies once).
fn compute_hash(source: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(source.as_bytes());
    let mut out = String::from("{SHA256}");
    for (i, b) in digest.iter().enumerate() {
        if i == 0 {
            out.push_str(&format!("{b:02x}"));
        } else {
            out.push_str(&format!("{b:x}"));
        }
    }
    out
}

/// ListSqlQuery(): table text or JSON rows under `key`.
fn list_sql(ua: &mut Ua, ctx: &ExecContext, sql: &str, ty: crate::sql_list::ListType, key: &str) {
    let Some(catalog) = &ctx.catalog else { return };
    match catalog.query_rows(sql) {
        Ok(table) => {
            if ua.api == ApiMode::Json {
                let rows = crate::sql_list::json_rows(&table);
                ua.json.insert(key.into(), Value::Array(rows));
            } else if table.rows.is_empty() {
                ua.decoration("No results to list.\n");
            } else {
                ua.send(&crate::sql_list::render(&table, ty));
            }
        }
        Err(e) => ua.decoration(&format!("Query failed: {e}\n")),
    }
}

/// C++ DoSubscriptionStatus(): backup units from the catalog against the
/// Director's `Subscriptions`.
fn status_subscriptions(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    use crate::sql_list::ListType;
    let mut ua = Ua::new(ctx);
    let words = || {
        let first = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
        format!("{} {first}", ast.command_name)
    };
    if !acl::no_restrictions(ctx, Kind::Client) || !acl::no_restrictions(ctx, Kind::Job)
        || !acl::no_restrictions(ctx, Kind::FileSet)
    {
        ua.error(&format!("{}: needs access to all client, job and fileset resources.\n", words()));
        return ua.finish(false);
    }
    match crate::ua_db::open_db(ast, ctx) {
        Ok(prefix) => ua.send(&prefix),
        Err(result) => {
            ua.send(&result.text_output);
            ua.error(&result.error_message);
            ua.error("Failed to open db.\n");
            return ua.finish(false);
        }
    }

    let kw_detail = has_arg(ast, "detail");
    let kw_unknown = has_arg(ast, "unknown");
    let kw_all = has_arg(ast, "all");
    let kw_anon = has_arg(ast, "anonymize");
    let kw_plugins = has_arg(ast, "plugins");
    let kw_clients = has_arg(ast, "clients");
    let client = arg_value(ast, "client");

    if kw_unknown {
        ua.error("Parameter 'unknown' ignored. There is no unknown data anymore.\n");
    } else if kw_detail {
        ua.error("Parameter 'detail' ignored. This is now the default.\n");
    }
    if let Some(client) = client {
        if kw_clients || kw_plugins || kw_anon || kw_all {
            ua.error("Parameter 'client=' provided. Ignoring other parameters.\n");
        }
        if let Err(msg) = name_valid(client) {
            ua.error(&format!("Client name invalid: {msg}"));
            return ua.finish(false);
        }
        ua.send(&format!("\nDetailed backup unit report for client '{client}':\n"));
        list_sql(&mut ua, ctx, &subscription_client_detail(client), ListType::Horz, "unit-detail");
        ua.send("All plugin jobs and the first non-plugin job are counted.\n");
        return ua.finish(true);
    }

    let now = crate::localtime::strftime_now("%F %T");
    let (client_col, order_col) = if kw_anon { ("client_anon", "client_id") } else { ("client_name", "client_name") };
    if kw_all || (!kw_clients && !kw_plugins) {
        let (full, os, binary) = (crate::version_strings::FULL, crate::version_strings::os_info(),
                                  crate::version_strings::BINARY_INFO);
        ua.send(&format!("Bareos version: {full} ({os})\nBinary info: {binary}\nReport time: {now}\n"));
        ua.json.insert("version".into(), json!(full));
        ua.json.insert("os".into(), json!(os));
        ua.json.insert("binary-info".into(), json!(binary));
        ua.json.insert("report-time".into(), json!(now));
        ua.send("\nDetailed backup unit report:\n");
        list_sql(&mut ua, ctx, &subscription_units(client_col, order_col), ListType::Horz, "unit-detail");
    }
    if kw_all || kw_clients {
        ua.send("\nBackup unit report aggregated by client:\n");
        list_sql(&mut ua, ctx, &subscription_units_client_total(client_col, order_col), ListType::Horz, "unit-clients");
        ua.send("For a detailed per-client report run 'status subscriptions client=<client-name>'\n");
    }
    if kw_all || kw_plugins {
        ua.send("\nBackup unit report aggregated by plugin:\n");
        list_sql(&mut ua, ctx, &subscription_units_plugin_total(), ListType::Horz, "unit-plugins");
    }

    ua.send("\nBackup unit summary:\n");
    let total = subscription_units_total(director_subscriptions(ctx));
    let summary = ctx.catalog.as_ref().map(|c| c.query_rows(&total));
    let vert = match &summary {
        Some(Ok(table)) => crate::sql_list::render(table, ListType::Vert),
        _ => String::new(),
    };
    match &summary {
        Some(Ok(table)) if ua.api == ApiMode::Json => {
            // CollapseMode::Collapse: a single row becomes an object
            let mut rows = crate::sql_list::json_rows(table);
            let value = if rows.len() == 1 { rows.remove(0) } else { Value::Array(rows) };
            ua.json.insert("unit-summary".into(), value);
        }
        Some(Ok(table)) if !table.rows.is_empty() => ua.send(&vert),
        Some(Ok(_)) => ua.decoration("No results to list.\n"),
        Some(Err(e)) => ua.decoration(&format!("Query failed: {e}\n")),
        _ => {}
    }
    // The checksum covers the summary as text (get_subscription_status_checksum_source_text)
    let checksum = compute_hash(&format!("SECRETSALT\n{now}\n{vert}"));
    ua.send(&format!("Checksum: {checksum}\n"));
    ua.json.insert("checksum".into(), json!(checksum));
    ua.finish(true)
}

/// lib/edit.cc IsNameValid() error texts.
fn name_valid(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name is empty\n".into());
    }
    if let Some(c) = name.chars().find(|c| !(c.is_ascii_alphanumeric() || ":.-_/ ".contains(*c))) {
        return Err(format!("Illegal character \"{c}\" in name.\n"));
    }
    if name.len() >= 128 {
        return Err("Name too long.\n".into());
    }
    if name.starts_with(' ') {
        return Err("Name cannot start with space.\n".into());
    }
    if name.ends_with(' ') {
        return Err("Name cannot end with space.\n".into());
    }
    Ok(())
}

/// Director `Subscriptions` directive (default 0).
fn director_subscriptions(ctx: &ExecContext) -> i64 {
    ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|cfg| cfg.raw.iter().find(|r| r.norm_type() == "director")
            .and_then(|r| r.directives.iter().find(|d| d.norm_key() == "subscriptions"))
            .and_then(|d| d.first().map(atoi)))
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// status configuration
// ---------------------------------------------------------------------------

/// C++ DoConfigurationStatus(). The Rust configuration parser records no
/// deprecation warnings, so the list is always empty.
fn status_configuration(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = Ua::new(ctx);
    if !acl::access(ctx, Kind::Command, "configure") {
        let first = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
        ua.error(&format!("{} {first}: is an invalid command or needs access right to the \"configure\" command.\n",
                          ast.command_name));
    } else {
        ua.send("No deprecated configuration settings detected.\n");
    }
    ua.finish(true)
}

/// C++ ua_status.cc ListDirStatusHeader (also `memory`). Plugin list,
/// secure erase command and configuration warnings are not supported by the
/// Rust Director and therefore never printed.
pub fn dir_status_header(ctx: &ExecContext) -> CommandResult {
    let header = dir_status_header_json(ctx);
    let text = format!(
        "{} Version: {} ({}) {}\n\
         Daemon started {}. Jobs: run={}, running={} db:postgresql, {} binary\n",
        header["director"].as_str().unwrap_or_default(),
        crate::version_strings::FULL,
        crate::version_strings::DATE,
        crate::version_strings::os_info(),
        header["daemon_started"].as_str().unwrap_or_default(),
        header["jobs_run"], header["jobs_running"],
        crate::version_strings::BINARY_INFO,
    );
    CommandResult::ok(text).with_json(serde_json::json!({ "header": header }))
}

/// JSON object of ListDirStatusHeader.
fn dir_status_header_json(ctx: &ExecContext) -> serde_json::Value {
    let name = ctx.config.as_ref()
        .and_then(|c| c.read().ok().map(|c| c.director.name.clone()))
        .unwrap_or_default();
    let (jobs_run, jobs_running) = ctx.job_manager.as_ref()
        .map(|mgr| {
            let jobs = mgr.lock().unwrap();
            (jobs.finished_count(), jobs.running_jobs().len())
        })
        .unwrap_or((0, 0));
    serde_json::json!({
        "director": name,
        "version": crate::version_strings::FULL,
        "release_date": crate::version_strings::DATE,
        "os": crate::version_strings::os_info(),
        "binary_info": crate::version_strings::BINARY_INFO,
        "daemon_started": crate::localtime::strftime("%d-%b-%y %H:%M", crate::localtime::daemon_started()),
        "jobs_run": jobs_run,
        "jobs_running": jobs_running,
        "config_warnings": false,
    })
}

/// C++ ClientStatus() (ua_status.cc:79) -> DoNativeClientStatus()
/// (fd_cmds.cc:999); `cmd` is the `.status client=… <cmd>` argument.
fn client_status(client: &bareos_config::resources::ClientResource, cmd: Option<&str>, ctx: &ExecContext,
                 director: &str) -> String {
    if !client.enabled {
        return format!("Client \"{}\" is disabled; skipping status call\n", client.name);
    }
    // case APT_NDMPV2..4: DoNdmpClientStatus() (compatibility log C-050)
    if client.protocol.is_ndmp() {
        return crate::ndmp::client_status(client);
    }
    let mut output = String::new();
    if ctx.api_mode == ApiMode::Human {
        output.push_str(&format!("Connecting to Client {} at {}:{}\n", client.name, client.address, client.port));
    }
    let (info, fd) = crate::fd_protocol::connect_fd_console(
        &client.address, client.port, &client.password, director, &client.name,
            (client.tls_enable, client.tls_require));
    output.push_str(&info);
    let command = match cmd {
        Some(cmd) => format!(".status {cmd}"),
        None => "status".to_string(),
    };
    match fd.and_then(|fd| crate::fd_protocol::fd_query_commands(fd, &[command])) {
        Ok(status) => output.push_str(&status),
        Err(_) => output.push_str(&format!("\nFailed to connect to Client {}.\n====\n", client.name)),
    }
    output
}

/// `status client[=<name>]` and `.status client[=<name>] [<cmd>]`: C++
/// get_client_resource() (ua_select.cc:506), then ClientStatus().
fn status_client_selected(ast: &CommandAst, ctx: &ExecContext, cmd: Option<&str>) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    if let Some(name) = crate::ua_select::get_client_resource(&mut ua, ast) {
        let found = ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|cfg| {
            let client = cfg.clients.iter().find(|c| c.name == name)?.clone();
            Some((client, cfg.director.name.clone()))
        });
        if let Some((client, director)) = found {
            ua.send_msg(client_status(&client, cmd, ctx, &director));
        }
    }
    ua.finish(true)
}

/// Every `Device` of a Storage resource joined with "," (C++
/// DoNativeStorageStatus builds `devicenames` from `store->device`).
fn device_names(config: &DirectorConfig, store: &bareos_config::resources::StorageResource) -> String {
    let devices: Vec<String> = config.raw.iter()
        .find(|r| r.norm_type() == "storage" && r.name() == Some(store.name.as_str()))
        .map(|r| r.directives.iter().filter(|d| d.norm_key() == "device")
            .flat_map(|d| d.values().iter().map(|v| v.to_string()).collect::<Vec<_>>()).collect())
        .unwrap_or_else(|| store.device.split(',').map(|d| d.trim().trim_matches('"').to_string()).collect());
    devices.join(",")
}

/// C++ StorageStatus() (storage.cc:501): native storages, or for NDMP
/// DoNdmpStorageStatus() (ndmp_dma_storage.cc:216): the paired native
/// storage daemon if there is one, else the NDMP query (C-050).
fn storage_status(store: &bareos_config::resources::StorageResource, cmd: Option<&str>, ctx: &ExecContext) -> String {
    if !store.protocol.is_ndmp() {
        return native_storage_status(store, cmd, ctx);
    }
    let paired = store.paired_storage.as_deref().and_then(|name| {
        ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|cfg| cfg.find_storage(name).cloned())
    });
    match paired {
        Some(paired) => native_storage_status(&paired, cmd, ctx),
        None => crate::ndmp::native_storage_status(store),
    }
}

/// C++ DoNativeStorageStatus() (sd_cmds.cc:692): connect, print the TLS
/// line, send `status <devices>` or `.status <cmd>` and relay every
/// message of the SD unchanged.
fn native_storage_status(store: &bareos_config::resources::StorageResource, cmd: Option<&str>, ctx: &ExecContext) -> String {
    use bareos_bnet::{BnetMessage, BnetSignal};
    let Some((director, devices)) = ctx.config.as_ref().and_then(|c| c.read().ok())
        .map(|cfg| (cfg.director.name.clone(), device_names(&cfg, store))) else {
        return String::new();
    };
    let mut output = String::new();
    if ctx.api_mode == ApiMode::Human {
        output.push_str(&format!("Connecting to Storage daemon {} at {}:{}\n", store.name, store.address, store.port));
    }
    let mut sd = match osd::connect::connect_sd(&store.address, store.port, &store.password, &director) {
        Ok(sd) => sd,
        Err(_) => {
            output.push_str(&format!("\nFailed to connect to Storage daemon {}.\n====\n", store.name));
            return output;
        }
    };
    let cipher = sd.inner().cipher_name();
    output.push_str(&bareos_bnet::tls_psk::cipher_message_string(Some(&cipher).filter(|c| !c.is_empty()).map(|c| c.as_str())));
    output.push('\n');
    let command = match cmd {
        Some(cmd) => format!(".status {cmd}\n"),
        // BashSpaces()
        None => format!("status {}\n", devices.replace(' ', "\u{1}")),
    };
    // fsend errors are silent upstream; the receive loop then ends.
    if sd.send_str(&command).is_ok() {
        // while (store_bsock->recv() >= 0): every signal ends the loop.
        while let Ok(BnetMessage::Data(data)) = sd.recv() {
            // SendMsg("%s", msg): the text up to the first NUL.
            let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
            output.push_str(&String::from_utf8_lossy(&data[..end]));
        }
    }
    // TerminateAndCloseJcrStoreSocket()
    let _ = sd.send_signal(BnetSignal::Terminate);
    output
}

/// `.status storage[=<name>] [<cmd>]`: get_storage_resource(ua) and
/// StorageStatus().
fn dot_status_storage(ast: &CommandAst, ctx: &ExecContext, cmd: Option<&str>) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    // DotStatusCmd returns true whatever the selection printed.
    if let Some(store) = crate::ua_select::get_storage_resource(&mut ua, ast, false, false) {
        ua.send_msg(storage_status(&store, cmd, ctx));
    }
    ua.finish(true)
}

/// Storage branch of C++ StatusCmd(): `status storage=<name>` or any other
/// first argument (storage selection: compatibility log C-013).
fn status_storage_selected(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let slots = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("slots"));
    let mut ua = crate::ua_select::Ua::new(ctx);
    let mut json = None;
    // StatusCmd returns true whatever the selection printed.
    if let Some(store) = crate::ua_select::get_storage_resource(&mut ua, ast, false, slots) {
        if slots {
            let director = ctx.config.as_ref().and_then(|c| c.read().ok())
                .map(|c| c.director.name.clone()).unwrap_or_default();
            let mut sd = crate::ua_storage_select::NetSd { director };
            json = status_slots(&mut ua, ast, &store, &mut sd);
        } else {
            ua.send_msg(storage_status(&store, None, ctx));
        }
    }
    let result = ua.finish(true);
    match json {
        Some(json) => result.with_json(json),
        None => result,
    }
}

/// The fields of a catalog Media record used by the slot views (C++
/// MediaDbRecord after GetMediaRecord(), PoolDbRecord::Name after
/// GetPoolRecord(), "?" when the pool is unknown).
#[derive(Default)]
struct SlotMedia {
    volume_name: String,
    vol_bytes: u64,
    vol_status: String,
    media_type: String,
    slot: i32,
    last_written: i64,
    vol_retention: u64,
    pool: String,
}

/// GetMediaRecord() by VolumeName (sql_get.cc:1065) + GetPoolRecord() by
/// PoolId. An empty name succeeds with an empty record (upstream then only
/// counts the media); None: no single matching Media row.
fn slot_media(ctx: &ExecContext, vol_name: &str) -> Option<SlotMedia> {
    let unknown_pool = || "?".to_string();
    if vol_name.is_empty() {
        return Some(SlotMedia { pool: unknown_pool(), ..Default::default() });
    }
    let catalog = ctx.catalog.as_ref()?;
    let table = catalog.query_rows(&format!(
        "SELECT VolumeName,VolBytes,MediaType,VolStatus,PoolId,VolRetention,Slot,LastWritten \
         FROM Media WHERE VolumeName='{}'", vol_name.replace('\'', "''"))).ok()?;
    let [row] = table.rows.as_slice() else { return None };
    let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    // str_to_int64()/str_to_uint64()
    let number = |i: usize| atoi(&text(i));
    let pool_id = number(4);
    let pool = (pool_id != 0).then(|| catalog.query_rows(&format!("SELECT Name FROM Pool WHERE Pool.PoolId={pool_id}")).ok())
        .flatten()
        .and_then(|t| match t.rows.as_slice() { [row] => Some(row.first().cloned().flatten().unwrap_or_default()), _ => None })
        .unwrap_or_else(unknown_pool);
    Some(SlotMedia {
        volume_name: text(0),
        vol_bytes: number(1) as u64,
        media_type: text(2),
        vol_status: text(3),
        vol_retention: number(5) as u64,
        slot: number(6) as i32,
        last_written: db_time(row.get(7).cloned().flatten().as_deref()),
        pool,
    })
}

/// `status storage=… slots`: C++ StatusSlots() (ua_status.cc:1917) without
/// API mode, StatusContentApi() (:1703) with `.api 1`, StatusContentJson()
/// (:1804) with `.api 2` (returns the "contents" result). The autochanger
/// is asked through `sd`; volumes are looked up in the catalog.
fn status_slots(ua: &mut crate::ua_select::Ua, ast: &CommandAst, store: &bareos_config::resources::StorageResource,
    sd: &mut impl crate::ua_storage_select::SdLink) -> Option<Value> {
    use crate::ua_storage_select as uss;
    let ctx = ua.ctx;
    if !ua.open_client_db(ast, false) {
        return None;
    }
    let mut max_slots = 0;
    let mut slot_list = Vec::new();
    if ctx.api_mode == ApiMode::Human {
        max_slots = uss::get_num_slots(ua, sd, store) as u16;
        if max_slots == 0 {
            ua.warning_msg("No slots in changer to scan.\n");
            return None;
        }
        slot_list = uss::get_user_slot_list(&ast.arguments, "slots", i32::from(max_slots))?;
    }
    let Some(vol_list) = uss::get_vol_list_from_storage(ua, sd, store, true, true, true) else {
        ua.warning_msg("No Volumes found, or no barcodes.\n");
        return None;
    };
    match ctx.api_mode {
        ApiMode::Human => {
            slot_table(ua, &vol_list, max_slots, &slot_list);
            None
        }
        ApiMode::Api => {
            content_api(ua, &vol_list);
            None
        }
        ApiMode::Json => Some(json!({ "contents": content_json(ctx, &vol_list) })),
    }
}

/// The slot table of StatusSlots() (ua_status.cc:1947).
fn slot_table(ua: &mut crate::ua_select::Ua, vol_list: &[crate::ua_storage_select::VolListEntry], max_slots: u16,
    slot_list: &[bool]) {
    // Slot | Volume | Status | MediaType | Pool
    // printf "%Ns": padded by bytes, never truncated.
    let pad = |s: &str, width: usize| format!("{}{s}", " ".repeat(width.saturating_sub(s.len())));
    let line = |slot: u16, flag: char, vol: &str, status: &str, media_type: &str, pool: &str| {
        format!(" {slot:>4}{flag}| {} | {} | {} | {} |\n", pad(vol, 16), pad(status, 9), pad(media_type, 14), pad(pool, 24))
    };
    ua.send_msg(" Slot |   Volume Name    |   Status  |  Media Type    |         Pool            |\n");
    ua.send_msg("------+------------------+-----------+----------------+--------------------------|\n");
    for vl1 in vol_list {
        if !matches!(vl1.slot_type, 'S' | 'I') {
            // We are not interested in drive slots.
            continue;
        }
        let import = vl1.slot_type == 'I';
        let other = if import { '@' } else { '*' };
        if vl1.slot > max_slots {
            ua.warning_msg(format!("Slot {} greater than max {} ignored.\n", vl1.slot as i16, max_slots as i16));
            continue;
        }
        // Check if user wants us to look at this slot
        if !slot_list.get(usize::from(vl1.slot).wrapping_sub(1)).copied().unwrap_or(false) {
            continue;
        }
        let mut vl2 = None;
        let name = match vl1.content {
            'E' => {
                if import {
                    ua.send_msg(line(vl1.slot, '@', "?", "?", "?", "?"));
                    continue;
                }
                // See if this empty slot is empty because the volume is
                // loaded in one of the drives.
                vl2 = crate::ua_storage_select::vol_is_loaded_in_drive(vol_list, vl1.slot);
                match vl2.and_then(|d| d.vol_name.as_deref()) {
                    Some(name) => name,
                    None => {
                        ua.send_msg(line(vl1.slot, '*', "?", "?", "?", "?"));
                        continue;
                    }
                }
            }
            'F' => match vl1.vol_name.as_deref() {
                Some(name) => name,
                None => {
                    ua.send_msg(line(vl1.slot, other, "?", "?", "?", "?"));
                    continue;
                }
            },
            _ => continue,
        };
        // bstrncpy(mr.VolumeName, …, MAX_NAME_LENGTH)
        let name: String = name.chars().take(127).collect();
        match (!name.is_empty()).then(|| slot_media(ua.ctx, &name)).flatten() {
            Some(mr) => {
                let flag = if import {
                    '@'
                } else if i32::from(vl1.slot) == mr.slot {
                    if vl2.is_some() { '%' } else { ' ' }
                } else {
                    '*'
                };
                ua.send_msg(line(vl1.slot, flag, &mr.volume_name, &mr.vol_status, &mr.media_type, &mr.pool));
            }
            None => ua.send_msg(line(vl1.slot, other, &name, "?", "?", "?")),
        }
    }
}

/// The volume of a slot: its own or the one loaded in a drive from it
/// (vol_is_loaded_in_drive()); None: an empty slot.
fn slot_volume<'a>(vol_list: &'a [crate::ua_storage_select::VolListEntry],
    vl1: &'a crate::ua_storage_select::VolListEntry) -> Option<Option<&'a str>> {
    match vl1.content {
        'F' => Some(vl1.vol_name.as_deref()),
        'E' => crate::ua_storage_select::vol_is_loaded_in_drive(vol_list, vl1.slot).map(|d| d.vol_name.as_deref()),
        _ => None,
    }
}

/// StatusContentApi() (ua_status.cc:1703): `D|` drive and `S|`/`I|` slot
/// lines; full slots via ContentSendInfoApi() (:1615).
fn content_api(ua: &mut crate::ua_select::Ua, vol_list: &[crate::ua_storage_select::VolListEntry]) {
    for vl1 in vol_list {
        match (vl1.slot_type, vl1.content) {
            ('D', 'F') => ua.send_msg(format!("D|{}|{}|{}\n", vl1.slot as i16, vl1.loaded_slot() as i16,
                vl1.vol_name.as_deref().unwrap_or("(null)"))),
            ('D', 'E') => ua.send_msg(format!("D|{}||\n", vl1.slot as i16)),
            (t @ ('S' | 'I'), 'F' | 'E') => match slot_volume(vol_list, vl1) {
                // A NULL volume name is undefined behaviour upstream; it is
                // treated as an empty name.
                Some(name) => {
                    // bstrncpy(mr.VolumeName, vol_name, MAX_NAME_LENGTH)
                    let name: String = name.unwrap_or("").chars().take(127).collect();
                    let line = match slot_media(ua.ctx, &name) {
                        Some(mr) => format!("{t}|{}|{}|{}|{}|{}|{}|{}|{}|{}\n", vl1.slot as i16, mr.slot as i16,
                            mr.volume_name, mr.vol_bytes, mr.vol_status, mr.media_type, mr.pool, mr.last_written as u64,
                            (mr.last_written as u64).wrapping_add(mr.vol_retention)),
                        None => format!("{t}|{}|0|{name}|?|?|?|?|0|0\n", vl1.slot as i16),
                    };
                    ua.send_msg(line);
                }
                None => ua.send_msg(format!("{t}|{}||||||||\n", vl1.slot as i16)),
            },
            _ => {}
        }
    }
}

/// StatusContentJson() (ua_status.cc:1804): the "contents" array; full
/// slots via ContentSendInfoJson() (:1641).
fn content_json(ctx: &ExecContext, vol_list: &[crate::ua_storage_select::VolListEntry]) -> Value {
    let mut contents = Vec::new();
    for vl1 in vol_list {
        let mut object = Map::new();
        match vl1.slot_type {
            'D' => {
                object.insert("type".into(), json!("drive"));
                object.insert("slotnr".into(), json!(vl1.slot));
                match vl1.content {
                    'F' => {
                        object.insert("content".into(), json!("full"));
                        object.insert("loaded".into(), json!(vl1.loaded_slot()));
                        object.insert("volname".into(), json!(vl1.vol_name.as_deref().unwrap_or("")));
                    }
                    'E' => {
                        object.insert("content".into(), json!("empty"));
                    }
                    _ => {}
                }
            }
            'S' | 'I' if matches!(vl1.content, 'F' | 'E') => {
                let kind = if vl1.slot_type == 'S' { "slot" } else { "import_slot" };
                object.insert("type".into(), json!(kind));
                object.insert("slotnr".into(), json!(vl1.slot));
                object.insert("content".into(), json!(if slot_volume(vol_list, vl1).is_some() { "full" } else { "empty" }));
                if let Some(name) = slot_volume(vol_list, vl1) {
                    let name: String = name.unwrap_or("").chars().take(127).collect();
                    match slot_media(ctx, &name) {
                        Some(mr) => {
                            object.insert("mr_slotnr".into(), json!(mr.slot));
                            object.insert("mr_volname".into(), json!(mr.volume_name));
                            object.insert("mr_volbytes".into(), json!(mr.vol_bytes as i64));
                            object.insert("mr_volstatus".into(), json!(mr.vol_status));
                            object.insert("mr_mediatype".into(), json!(mr.media_type));
                            object.insert("pr_name".into(), json!(mr.pool));
                            object.insert("mr_lastwritten".into(), json!(mr.last_written));
                            object.insert("mr_expire".into(),
                                json!((mr.last_written as u64).wrapping_add(mr.vol_retention) as i64));
                        }
                        None => {
                            object.insert("mr_slotnr".into(), json!(0));
                            object.insert("mr_volname".into(), json!(name));
                            object.insert("mr_volbytes".into(), json!("?"));
                            object.insert("mr_volstatus".into(), json!("?"));
                            object.insert("mr_mediatype".into(), json!("?"));
                            object.insert("pr_name".into(), json!("?"));
                            object.insert("mr_lastwritten".into(), json!(0));
                            object.insert("mr_expire".into(), json!(0));
                        }
                    }
                }
            }
            _ => continue,
        }
        contents.push(Value::Object(object));
    }
    Value::Array(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_like_edit_uint64_with_suffix() {
        assert_eq!(edit_uint64_with_suffix(0), "0 ");
        assert_eq!(edit_uint64_with_suffix(999), "999 ");
        assert_eq!(edit_uint64_with_suffix(22_730), "22.73 K");
        assert_eq!(edit_uint64_with_suffix(126_300_000), "126.3 M");
        assert_eq!(edit_uint64_with_suffix(967_000), "967.0 K");
        assert_eq!(edit_uint64_with_suffix(1_500_000_000), "1.500 G");
    }

    #[test]
    fn checksum_pads_only_the_first_byte() {
        // Captured from bareos-dir 25.1.2 `status subscriptions`
        let summary = " accounting_mode: volume-based (1 volume-based units >= 1 count-based units)\n            used: 1\n      configured: 0\n       remaining: -1\n\n";
        assert_eq!(compute_hash(&format!("SECRETSALT\n2026-10-15 20:47:52\n{summary}")),
                   "{SHA256}7269ae9256a0a11a1a2e71393e4f81388a697cf018b329d841d79f2e4cfc142");
    }

    #[test]
    fn preview_overrides_in_cpp_order() {
        let input = "Schedule {\n Name = s\n Run = Level=Incremental Priority=5 SpoolData=yes Accurate=no Storage=File Messages=Standard Pool=Full daily at 12:45\n}\n";
        let run = match &bareos_config::parser::parse_config(input).unwrap()[0] {
            bareos_config::Resource::Schedule(s) => s.runs[0].clone(),
            _ => unreachable!(),
        };
        assert_eq!(run_overrides(&run),
                   "Level=Incremental Priority=5 Spool Data=1 Accurate=0 Pool=Full Storage=File Messages=Standard");
    }

    #[test]
    fn names_and_numbers() {
        assert_eq!(atoi(" -12x"), -12);
        assert_eq!(atoi("abc"), 0);
        assert!(name_valid("bareos-fd").is_ok());
        assert_eq!(name_valid("a*b"), Err("Illegal character \"*\" in name.\n".into()));
        assert_eq!(level_code_name("I"), "Incremental");
        assert_eq!(job_type_code_name("B"), "Backup");
    }

    // ---- status storage=… slots (compatibility log C-045) ----

    use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse, SqlTable};
    use crate::command_ast::Argument;
    use crate::ua_storage_select::{SdLink, invalidate_vol_list};
    use bareos_config::resources::StorageResource;

    /// Fake autochanger SD: answers like stored/dir_cmd.cc ChangerCmd() +
    /// autochanger.cc AutochangerCmd() with an mtx-changer listall output.
    struct ChangerSd { sent: Vec<String>, slots: u16, listall: Vec<&'static str> }

    impl SdLink for ChangerSd {
        fn exchange(&mut self, store: &StorageResource, command: &str) -> Result<Vec<String>, String> {
            self.sent.push(command.to_string());
            let dev = store.device.as_str();
            let issuing = |cmd: &str| format!("3306 Issuing autochanger \"{cmd}\" command.\n");
            Ok(if command == format!("autochanger slots {dev}\n") {
                vec![issuing("slots"), format!("slots={}", self.slots)]
            } else if command == format!("autochanger listall {dev} \n") {
                std::iter::once(issuing("listall")).chain(self.listall.iter().map(|l| l.to_string())).collect()
            } else {
                vec![format!("3908 Error scanning autochanger drives/list/slots command: {command}")]
            })
        }
    }

    const LISTALL: &[&str] = &["D:0:F:3:vol3\n", "D:1:E\n", "S:1:F:vol1\n", "S:2:E\n", "S:3:E\n",
        "S:4:F:unknown4\n", "S:5:F:vol5\n", "I:6:F:vol6\n", "I:7:E\n"];

    fn changer(listall: &[&'static str]) -> ChangerSd {
        ChangerSd { sent: vec![], slots: 6, listall: listall.to_vec() }
    }

    fn rows(rows: Vec<Vec<Option<&str>>>) -> CatalogResponse {
        CatalogResponse::Rows(SqlTable { columns: vec![],
            rows: rows.into_iter().map(|r| r.into_iter().map(|v| v.map(str::to_string)).collect()).collect() })
    }

    /// Catalog double: Media vol1/vol3/vol5/vol6, Pools 1 Full and 2 Scratch.
    fn slots_catalog() -> CatalogHandle {
        CatalogHandle::mock(|request| {
            let CatalogRequest::TextRows { sql } = request else { return CatalogResponse::Error("write".into()) };
            let media = |cols: [Option<&str>; 8]| rows(vec![cols.to_vec()]);
            const MEDIA: &str = "SELECT VolumeName,VolBytes,MediaType,VolStatus,PoolId,VolRetention,Slot,LastWritten \
                FROM Media WHERE VolumeName='";
            if let Some(name) = sql.strip_prefix(MEDIA) {
                match name.trim_end_matches('\'') {
                    "vol1" => media([Some("vol1"), Some("1000"), Some("File"), Some("Append"), Some("1"), Some("100"),
                        Some("1"), Some("2026-10-15 20:00:00")]),
                    "vol3" => media([Some("vol3"), Some("3000"), Some("File"), Some("Full"), Some("1"), Some("0"),
                        Some("3"), None]),
                    "vol5" => media([Some("vol5"), Some("5"), Some("LTO"), Some("Used"), Some("77"), Some("10"),
                        Some("9"), Some("2026-10-15 20:00:01")]),
                    "vol6" => media([Some("vol6"), Some("6"), Some("File"), Some("Append"), Some("2"), Some("0"),
                        Some("0"), None]),
                    _ => rows(vec![]),
                }
            } else {
                match sql.as_str() {
                    "SELECT Name FROM Pool WHERE Pool.PoolId=1" => rows(vec![vec![Some("Full")]]),
                    "SELECT Name FROM Pool WHERE Pool.PoolId=2" => rows(vec![vec![Some("Scratch")]]),
                    "SELECT Name FROM Pool WHERE Pool.PoolId=77" => rows(vec![]),
                    other => panic!("unexpected query {other}"),
                }
            }
        })
    }

    fn slots_ast(line: &str) -> CommandAst {
        let arguments = line.split_whitespace().map(|w| match w.split_once('=') {
            Some((k, v)) => Argument { key: k.into(), value: Some(v.into()) },
            None => Argument { key: w.into(), value: None },
        }).collect();
        CommandAst { command_name: "status".into(), arguments, ..Default::default() }
    }

    /// Run status_slots for the storage `name`: the output in call order
    /// and the JSON result.
    fn run_slots(mode: ApiMode, name: &str, line: &str, sd: &mut ChangerSd) -> (String, Option<Value>) {
        let mut ctx = crate::ua_select::tests::context("Director { Name=dir; Password=secret }", mode);
        ctx.catalog = Some(slots_catalog());
        let store = StorageResource { name: name.into(), address: "sd.example".into(), port: 9103,
            device: "ChangerDev".into(), ..Default::default() };
        let mut ua = crate::ua_select::Ua::new(&ctx);
        let json = status_slots(&mut ua, &slots_ast(line), &store, sd);
        let result = ua.finish(true);
        let out = if result.errors_inline || !result.errors_first {
            format!("{}{}", result.text_output, if result.errors_inline { "" } else { &result.error_message })
        } else {
            format!("{}{}", result.error_message, result.text_output)
        };
        (out, json)
    }

    fn connect(name: &str) -> String {
        format!("Connecting to Storage daemon {name} at sd.example:9103 ...\n")
    }

    fn lw(text: &str) -> i64 {
        db_time(Some(text))
    }

    #[test]
    fn slot_table_like_status_slots() {
        let mut sd = changer(LISTALL);
        let (out, json) = run_slots(ApiMode::Human, "SlotsTable", "storage=SlotsTable slots", &mut sd);
        assert!(json.is_none());
        assert_eq!(sd.sent, ["autochanger slots ChangerDev\n", "autochanger listall ChangerDev \n"]);
        // Lines as printf(" %4i%c| %16s | %9s | %14s | %24s |\n", …) prints them.
        let expected = [
            connect("SlotsTable").as_str(),
            "3306 Issuing autochanger \"slots\" command.\n",
            "Device \"ChangerDev\" has 6 slots.\n",
            &connect("SlotsTable"),
            "3306 Issuing autochanger \"listall\" command.\n",
            " Slot |   Volume Name    |   Status  |  Media Type    |         Pool            |\n",
            "------+------------------+-----------+----------------+--------------------------|\n",
            "    1 |             vol1 |    Append |           File |                     Full |\n",
            "    2*|                ? |         ? |              ? |                        ? |\n",
            "    3%|             vol3 |      Full |           File |                     Full |\n",
            "    4*|         unknown4 |         ? |              ? |                        ? |\n",
            "    5*|             vol5 |      Used |            LTO |                        ? |\n",
            "    6@|             vol6 |    Append |           File |                  Scratch |\n",
            "Slot 7 greater than max 6 ignored.\n",
        ].concat();
        assert_eq!(out, expected);

        // Slots and the volume list are cached: no SD contact at all.
        // FindArgWithValue("slots") stops at the bare "slots": all slots.
        assert_eq!(run_slots(ApiMode::Human, "SlotsTable", "SlotsTable slots slots=2", &mut sd).0,
            expected[expected.find(" Slot |").unwrap()..]);
        let (out, _) = run_slots(ApiMode::Human, "SlotsTable", "SlotsTable slots=2,4-5", &mut sd);
        assert_eq!(sd.sent.len(), 2);
        assert_eq!(out, [
            " Slot |   Volume Name    |   Status  |  Media Type    |         Pool            |\n",
            "------+------------------+-----------+----------------+--------------------------|\n",
            "    2*|                ? |         ? |              ? |                        ? |\n",
            "    4*|         unknown4 |         ? |              ? |                        ? |\n",
            "    5*|             vol5 |      Used |            LTO |                        ? |\n",
            "Slot 7 greater than max 6 ignored.\n",
        ].concat());

        // A malformed slot list ends the command silently.
        assert_eq!(run_slots(ApiMode::Human, "SlotsTable", "slots slot=7", &mut sd).0, "");
        for list in ["3-1", "0", "-2", "2-x", "x-2", "1,,2", "7", ",1", "a", "1,"] {
            if list == "1," {
                // A trailing ',' ends the list: only slot 1.
                assert!(run_slots(ApiMode::Human, "SlotsTable", "slots=1,", &mut sd).0.ends_with(
                    "|\n    1 |             vol1 |    Append |           File |                     Full |\n\
                     Slot 7 greater than max 6 ignored.\n"));
                continue;
            }
            assert_eq!(run_slots(ApiMode::Human, "SlotsTable", &format!("slots={list}"), &mut sd).0, "", "{list}");
        }

        // mount/unmount/release invalidate the list: asked again.
        invalidate_vol_list("SlotsTable");
        let (out, _) = run_slots(ApiMode::Human, "SlotsTable", "slots slots=1", &mut sd);
        assert_eq!(sd.sent.len(), 3);
        assert_eq!(&sd.sent[2], "autochanger listall ChangerDev \n");
        assert!(out.starts_with(&format!("{}3306 Issuing autochanger \"listall\" command.\n Slot |",
            connect("SlotsTable"))), "{out}");
    }

    #[test]
    fn slot_table_without_slots_or_volumes() {
        let mut sd = ChangerSd { sent: vec![], slots: 0, listall: vec![] };
        assert_eq!(run_slots(ApiMode::Human, "SlotsNone", "slots", &mut sd).0, [
            connect("SlotsNone").as_str(),
            "3306 Issuing autochanger \"slots\" command.\n",
            "Device \"ChangerDev\" has 0 slots.\n",
            "No slots in changer to scan.\n",
        ].concat());
        let mut sd = ChangerSd { sent: vec![], slots: 4,
            listall: vec!["3998 Autochanger error: ERR=Child exited with code 1\n"] };
        assert_eq!(run_slots(ApiMode::Human, "SlotsEmpty", "slots", &mut sd).0, [
            connect("SlotsEmpty").as_str(),
            "3306 Issuing autochanger \"slots\" command.\n",
            "Device \"ChangerDev\" has 4 slots.\n",
            &connect("SlotsEmpty"),
            "3306 Issuing autochanger \"listall\" command.\n",
            "3998 Autochanger error: ERR=Child exited with code 1\n",
            "No Volumes found, or no barcodes.\n",
        ].concat());
        // A full slot without volume name, an empty volume name, an import
        // slot without name.
        let mut sd = ChangerSd { sent: vec![], slots: 3, listall: vec!["S:1:F\n", "S:2:F:\n", "I:3:F\n"] };
        let (out, _) = run_slots(ApiMode::Human, "SlotsOdd", "slots", &mut sd);
        assert!(out.ends_with(&[
            "    1*|                ? |         ? |              ? |                        ? |\n",
            "    2*|                  |         ? |              ? |                        ? |\n",
            "    3@|                ? |         ? |              ? |                        ? |\n",
        ].concat()), "{out}");
    }

    #[test]
    fn content_api_lines() {
        let mut sd = changer(LISTALL);
        let (out, json) = run_slots(ApiMode::Api, "SlotsApi", "slots", &mut sd);
        assert!(json.is_none());
        // StatusContentApi asks for no slot count.
        assert_eq!(sd.sent, ["autochanger listall ChangerDev \n"]);
        let (lw1, lw5) = (lw("2026-10-15 20:00:00"), lw("2026-10-15 20:00:01"));
        assert_eq!(out, [
            connect("SlotsApi"),
            "3306 Issuing autochanger \"listall\" command.\n".into(),
            "D|0|3|vol3\n".into(),
            "D|1||\n".into(),
            format!("S|1|1|vol1|1000|Append|File|Full|{lw1}|{}\n", lw1 + 100),
            "S|2||||||||\n".into(),
            "S|3|3|vol3|3000|Full|File|Full|0|0\n".into(),
            "S|4|0|unknown4|?|?|?|?|0|0\n".into(),
            format!("S|5|9|vol5|5|Used|LTO|?|{lw5}|{}\n", lw5 + 10),
            "I|6|0|vol6|6|Append|File|Scratch|0|0\n".into(),
            "I|7||||||||\n".into(),
        ].concat());
    }

    #[test]
    fn content_json_objects() {
        let mut sd = changer(LISTALL);
        let (_, json) = run_slots(ApiMode::Json, "SlotsJson", "slots", &mut sd);
        let (lw1, lw5) = (lw("2026-10-15 20:00:00"), lw("2026-10-15 20:00:01"));
        #[allow(clippy::too_many_arguments)]
        fn full(kind: &str, slot: u16, mr_slot: i64, name: &str, bytes: i64, status: &str, mt: &str, pool: &str,
            lastw: i64, expire: i64) -> Value {
            json!({ "type": kind, "slotnr": slot, "content": "full", "mr_slotnr": mr_slot, "mr_volname": name,
                "mr_volbytes": bytes, "mr_volstatus": status, "mr_mediatype": mt, "pr_name": pool,
                "mr_lastwritten": lastw, "mr_expire": expire })
        }
        let expected = json!({ "contents": [
            { "type": "drive", "slotnr": 0, "content": "full", "loaded": 3, "volname": "vol3" },
            { "type": "drive", "slotnr": 1, "content": "empty" },
            full("slot", 1, 1, "vol1", 1000, "Append", "File", "Full", lw1, lw1 + 100),
            { "type": "slot", "slotnr": 2, "content": "empty" },
            full("slot", 3, 3, "vol3", 3000, "Full", "File", "Full", 0, 0),
            { "type": "slot", "slotnr": 4, "content": "full", "mr_slotnr": 0, "mr_volname": "unknown4",
              "mr_volbytes": "?", "mr_volstatus": "?", "mr_mediatype": "?", "pr_name": "?",
              "mr_lastwritten": 0, "mr_expire": 0 },
            full("slot", 5, 9, "vol5", 5, "Used", "LTO", "?", lw5, lw5 + 10),
            full("import_slot", 6, 0, "vol6", 6, "Append", "File", "Scratch", 0, 0),
            { "type": "import_slot", "slotnr": 7, "content": "empty" },
        ]});
        // Compared as jansson would print them (key order included).
        assert_eq!(crate::json_output::dumps(&json.unwrap(), false), crate::json_output::dumps(&expected, false));
        // Without volumes: the warning only, no "contents".
        let mut sd = ChangerSd { sent: vec![], slots: 4, listall: vec![] };
        let (out, json) = run_slots(ApiMode::Json, "SlotsJsonNone", "slots", &mut sd);
        assert!(json.is_none());
        assert!(out.ends_with("No Volumes found, or no barcodes.\n"), "{out}");
    }

    /// ListRunningJobs() status texts (Compatibility log C-048).
    #[test]
    fn running_job_status_texts() {
        use crate::job_manager::ActiveJob;
        let job = |status, sd_status, fd_connected| ActiveJob { status, sd_status, fd_connected,
            client: "fd1".into(), storage: "File".into(), ..Default::default() };
        let texts: Vec<String> = ['C', 'R', 'B', 'A', 'F', 'S', 's', 'c', 'j', 'd', 'p', 't', 'T', 'f', 'x']
            .iter().map(|status| running_job_status(&job(*status, '\0', false))).collect();
        assert_eq!(texts, ["is waiting execution", "is running", "is blocked", "has been canceled",
            "is waiting on Client fd1", "is waiting on Storage \"File\"", "is waiting on max Storage jobs",
            "is waiting on max Client jobs", "is waiting on max Job jobs", "is waiting on max total jobs",
            "is waiting for higher priority jobs to finish", "is waiting for its start time", "has terminated",
            "has a fatal error", "is in unknown state x"]);
        assert_eq!(running_job_status(&ActiveJob { status: 'S', ..Default::default() }), "is waiting on Storage");
        assert_eq!(running_job_status(&job('R', 'm', true)), "is waiting for an appendable Volume");
        assert_eq!(running_job_status(&job('A', 'M', true)), "is waiting for a mount request");
        assert_eq!(running_job_status(&job('R', 'F', true)), "is waiting for Client fd1 to connect to Storage File");
        assert_eq!(running_job_status(&job('R', 'F', false)), "is waiting for Client to connect (Client Initiated Connection)");
        let start = ActiveJob { status: 't', sched_time: 1_700_000_000, ..Default::default() };
        assert_eq!(running_job_status(&start), format!("is waiting for its start time at {}", bstrftime_nc(1_700_000_000)));
    }
}
