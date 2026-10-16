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
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
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
        // The "Status available for:" selection needs the dialog framework
        // (compatibility log C-011); show the Director status instead.
        return status_director(ast, ctx);
    };
    let key = first.key.to_ascii_lowercase();
    if key == "all" {
        status_all(ast, ctx)
    } else if key.starts_with("dir") {
        status_director(ast, ctx)
    } else if key == "client" {
        let name = arg_value(ast, "client").unwrap_or("*");
        status_client(name, ctx)
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
        "client" => status_client(arg_value(ast, "client").unwrap_or("*"), ctx),
        "storage" => status_storage(arg_value(ast, "storage").unwrap_or("*"), ctx),
        _ => bad("1900 Bad .status command, wrong argument.\n"),
    }
}

/// C++ DoAllStatus(): Director, every Storage daemon and every File daemon
/// once per address/port.
fn status_all(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut result = status_director(ast, ctx);
    let (storages, clients) = match ctx.config.as_ref().and_then(|c| c.read().ok()) {
        Some(cfg) => {
            let mut storages: Vec<(String, String, u16)> = Vec::new();
            for s in cfg.storages.iter().filter(|s| acl::access(ctx, Kind::Storage, &s.name)) {
                if !storages.iter().any(|u| u.1 == s.address && u.2 == s.port) {
                    storages.push((s.name.clone(), s.address.clone(), s.port));
                }
            }
            let mut clients: Vec<(String, String, u16)> = Vec::new();
            for c in cfg.clients.iter().filter(|c| acl::access(ctx, Kind::Client, &c.name)) {
                if !clients.iter().any(|u| u.1 == c.address && u.2 == c.port) {
                    clients.push((c.name.clone(), c.address.clone(), c.port));
                }
            }
            (storages, clients)
        }
        None => (Vec::new(), Vec::new()),
    };
    for (name, ..) in &storages {
        result.text_output.push_str(&status_storage(name, ctx).text_output);
    }
    for (name, ..) in &clients {
        result.text_output.push_str(&status_client(name, ctx).text_output);
    }
    result
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
    let Some(s) = s else { return 0 };
    let parts: Vec<i32> = s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty())
        .map(|p| p.parse().unwrap_or(0)).collect();
    if parts.len() < 6 {
        return 0;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = parts[0] - 1900;
    tm.tm_mon = parts[1] - 1;
    tm.tm_mday = parts[2];
    tm.tm_hour = parts[3];
    tm.tm_min = parts[4];
    tm.tm_sec = parts[5];
    tm.tm_isdst = -1;
    unsafe { libc::mktime(&mut tm) as i64 }
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
        use crate::job_manager::DirJobPhase as P;
        let msg = match job.phase {
            P::Queued | P::Starting => "is waiting execution",
            P::Canceling => "has been canceled",
            P::WaitingMedia => "is waiting for an appendable Volume",
            P::WaitingMount => "is waiting for a mount request",
            P::Completed => "has terminated",
            P::Error => "has erred",
            P::Running | P::Paused | P::WaitingCatReq | P::Finishing => "is running",
        };
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
        use crate::job_manager::DirJobPhase as P;
        let status = match job.phase {
            P::Queued | P::Starting => 'C',
            P::Canceling => 'A',
            P::Error => 'f',
            P::Completed => 'T',
            _ => 'R',
        };
        ua.send(&format!("JobId={} JobStatus={status} JobErrors=0\n", job.id));
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

fn status_client(name: &str, ctx: &ExecContext) -> CommandResult {
    let config = match &ctx.config {
        Some(c) => c.read().ok(),
        None => return CommandResult::ok(format!("Client {name}: no config loaded\n")),
    };
    let config = match config {
        Some(c) => c,
        None => return CommandResult::ok(format!("Client {name}: config lock error\n")),
    };

    let dir_name = config.director.name.clone();

    // Find all matching clients
    let clients: Vec<_> = if name == "*" {
        config.clients.iter().collect()
    } else {
        config.clients.iter().filter(|c| c.name == name).collect()
    };

    if clients.is_empty() {
        return CommandResult::ok(format!("Client '{name}' not found in configuration\n"));
    }

    let mut output = String::new();
    for client in &clients {
        // DoNativeClientStatus (fd_cmds.cc:999)
        if ctx.api_mode == crate::command_executor::ApiMode::Human {
            output.push_str(&format!("Connecting to Client {} at {}:{}\n",
                                      client.name, client.address, client.port));
        }
        let (info, fd) = crate::fd_protocol::connect_fd_console(
            &client.address, client.port, &client.password, &dir_name, &client.name);
        output.push_str(&info);
        match fd.and_then(|fd| crate::fd_protocol::fd_query_commands(fd, &["status".to_string()])) {
            Ok(status) => output.push_str(&status),
            Err(_) => output.push_str(&format!("\nFailed to connect to Client {}.\n====\n", client.name)),
        }
    }

    CommandResult::ok(output)
}

fn status_storage(name: &str, ctx: &ExecContext) -> CommandResult {
    let config = match &ctx.config {
        Some(c) => match c.read() {
            Ok(c) => c,
            Err(_) => return CommandResult::ok(format!("Storage {name}: config error\n")),
        },
        None => return CommandResult::ok(format!("Storage {name}: no config\n")),
    };

    let dir_name = config.director.name.clone();
    let storages: Vec<_> = if name == "*" {
        config.storages.iter().collect()
    } else {
        config.storages.iter().filter(|s| s.name == name).collect()
    };

    if storages.is_empty() {
        return CommandResult::ok(format!("Storage '{name}' not found\n"));
    }

    let mut output = String::new();
    for stor in &storages {
        output.push_str(&format!("Connecting to Storage {} at {}:{}\n",
                                  stor.name, stor.address, stor.port));

        let result = crate::sd_protocol::query_sd_status(
            &stor.address, stor.port, &stor.password, &dir_name, &stor.name,
        );

        if result.success {
            output.push_str(&format!("Connected to Storage {}\n", stor.name));
            output.push_str(&result.status_text);
        } else {
            output.push_str(&format!("Failed: {}\n", result.status_text));
        }
        output.push('\n');
    }

    CommandResult::ok(output)
}

/// Storage branch of C++ StatusCmd(): `status storage=<name>` or any other
/// first argument (storage selection: compatibility log C-013).
fn status_storage_selected(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let slots = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("slots"));
    match crate::ua_storage_select::get_storage_resource(ast, ctx, false, slots) {
        Ok((store, text)) => crate::ua_db::with_prefix(text, status_storage(&store.name, ctx)),
        // StatusCmd returns true whatever the selection printed.
        Err(result) => CommandResult { success: true, ..result },
    }
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
}
