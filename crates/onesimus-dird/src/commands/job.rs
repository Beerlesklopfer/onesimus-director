//! Job control commands: run, cancel, enable, disable

use std::sync::atomic::{AtomicU32, Ordering};
use tracing::info;

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::job_worker::{self, JobParams, WorkerJobType};
use crate::message_bus::BusMessage;
use bareos_config::resources::JobType;

static NEXT_JOB_ID: AtomicU32 = AtomicU32::new(1);

pub fn cmd_run(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    let success = run_with_ua(&mut ua, ast, start_job);
    ua.finish(success)
}

/// Start the job of a confirmed run: the resolved request (job, level,
/// client, fileset, storage, pool and `yes`) goes through run_selected.
pub(crate) fn start_job(ast: &CommandAst, ctx: &ExecContext, job: bareos_config::resources::JobResource) -> CommandResult {
    match ctx.config.as_ref().map(|c| c.read()) {
        Some(Ok(config)) => run_selected(ast, ctx, config, job),
        Some(Err(_)) => CommandResult::error("Config read error\n".to_string()),
        None => CommandResult::error("No config loaded\n".to_string()),
    }
}

/// C++ ua_run.cc DoRunCmd(): OpenClientDb, ScanCommandLineArguments,
/// SetJcrDefaults, then ResetRestoreContext and — unless `yes` was given or
/// the console is in batch (JSON) mode — DisplayJobParameters and
/// "OK to run? (yes/mod/no): " with the ModifyJobParameters dialog. `start`
/// runs the confirmed job; it is never called on "no", on a cancelled
/// dialog or without a console (GetCmd fails: "Job not run."). Returns the
/// DoRunCmd result.
pub(crate) fn run_with_ua(
    ua: &mut crate::ua_select::Ua,
    ast: &CommandAst,
    start: impl FnOnce(&CommandAst, &ExecContext, bareos_config::resources::JobResource) -> CommandResult,
) -> bool {
    let ctx = ua.ctx;
    if !ua.open_client_db(ast, false) {
        return false;
    }
    let mut rc = RunContext::default();
    if !rc.scan(ua, ast) {
        return false;
    }
    let Some(job) = rc.job.clone() else { return false };
    let mut jcr = Jcr::new(ctx, &job);
    let mut do_pool_overrides = true;

    loop {
        // try_again:
        if !rc.reset(ua, &mut jcr) {
            break;
        }
        // Run without prompting? (`ua->batch` is the JSON mode here.)
        if ctx.api_mode == crate::command_executor::ApiMode::Json || crate::ua_select::find_arg(ast, "yes").is_some() {
            return run_start(ua, &jcr, start);
        }
        // Interactive runs perform the level pool overrides once, early.
        if do_pool_overrides {
            if jcr.job_type == JobType::Backup && jcr.level != L_VIRTUAL_FULL {
                jcr.apply_pool_overrides(ctx, false);
            }
            do_pool_overrides = false;
        }
        if !display_job_parameters(ua, &mut jcr, &mut rc) {
            break;
        }
        // Prompt User until we have a valid response.
        let answer = loop {
            let Some(answer) = ua.get_cmd("OK to run? (yes/mod/no): ") else {
                ua.send_msg("Job not run.\n");
                return false;
            };
            if run_answer_valid(&answer) {
                break answer;
            }
            ua.warning_msg(format!("Illegal response {answer}\n"));
        };
        let lower = answer.to_ascii_lowercase();
        // ".mod <args>" / "mod <args>": parse the new arguments, show again.
        if lower.starts_with(".mod ") || (lower.starts_with("mod ") && answer.len() > 6) {
            rc.is_mod = true;
            if !rc.scan(ua, &crate::session::parse_cli_command(&answer)) {
                return false;
            }
            continue;
        }
        match modify_job_parameters(ua, &mut jcr, &mut rc, &answer) {
            Modify::Again => continue,
            Modify::Done => {}
            Modify::Cancel => break,
        }
        // Interactive runs keep the pool they showed.
        jcr.ignore_level_pool_overrides = true;
        if answer.is_empty() || prefix_of(&lower, "yes") {
            return run_start(ua, &jcr, start);
        }
        break;
    }
    // bail_out:
    ua.send_msg("Job not run.\n");
    false
}

/// `bstrncasecmp(cmd, word, strlen(cmd))`: the answer is a prefix of word.
fn prefix_of(answer_lower: &str, word: &str) -> bool {
    word.starts_with(answer_lower)
}

/// C++ DoRunCmd valid_response: empty, or a prefix of ".mod "/"mod "
/// (compared over at most the keyword length) or of "yes"/"no".
fn run_answer_valid(answer: &str) -> bool {
    let lower = answer.to_ascii_lowercase();
    let head = |n: usize| lower.chars().take(n).collect::<String>();
    answer.is_empty()
        || ".mod ".starts_with(&head(5))
        || "mod ".starts_with(&head(4))
        || prefix_of(&lower, "yes")
        || prefix_of(&lower, "no")
}

fn run_start(
    ua: &mut crate::ua_select::Ua,
    jcr: &Jcr,
    start: impl FnOnce(&CommandAst, &ExecContext, bareos_config::resources::JobResource) -> CommandResult,
) -> bool {
    let result = start(&jcr.to_ast(), ua.ctx, jcr.job.clone());
    let success = result.success;
    ua.send_result(result);
    success
}

// Job levels (include/job_level.h) and replace options (dird_conf.cc:606).
const L_FULL: u8 = b'F';
const L_INCREMENTAL: u8 = b'I';
const L_DIFFERENTIAL: u8 = b'D';
const L_SINCE: u8 = b'S';
const L_VERIFY_CATALOG: u8 = b'C';
const L_VERIFY_INIT: u8 = b'V';
const L_VERIFY_VOLUME_TO_CATALOG: u8 = b'O';
const L_VERIFY_DISK_TO_CATALOG: u8 = b'd';
const L_VERIFY_DATA: u8 = b'A';
const L_VIRTUAL_FULL: u8 = b'f';
const L_NONE: u8 = b' ';

/// joblevels[] (dird_conf.cc:518): name and level.
const JOB_LEVELS: &[(&str, u8)] = &[
    ("Full", L_FULL), ("Base", b'B'), ("Incremental", L_INCREMENTAL), ("Differential", L_DIFFERENTIAL),
    ("Since", L_SINCE), ("VirtualFull", L_VIRTUAL_FULL), ("Catalog", L_VERIFY_CATALOG),
    ("InitCatalog", L_VERIFY_INIT), ("VolumeToCatalog", L_VERIFY_VOLUME_TO_CATALOG),
    ("DiskToCatalog", L_VERIFY_DISK_TO_CATALOG), ("Data", L_VERIFY_DATA), (" ", L_NONE),
];

/// ReplaceOptions[] (dird_conf.cc:606).
const REPLACE_OPTIONS: &[(&str, u8)] = &[("Always", b'a'), ("IfNewer", b'w'), ("IfOlder", b'o'), ("Never", b'n')];

/// GetLevelFromName(): the first level of that name (any job type).
fn level_from_name(name: &str) -> Option<u8> {
    JOB_LEVELS.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, level)| *level)
}

/// JobLevelToString() (dird_conf.cc:2090).
fn job_level_to_string(level: u8) -> String {
    JOB_LEVELS.iter().find(|(_, l)| *l == level).map(|(n, _)| n.to_string())
        .unwrap_or_else(|| format!("{} ({level})", level as char))
}

fn now_unix_time() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// bstrutime(): local time "%Y-%m-%d %H:%M:%S".
fn bstrutime(t: i64) -> String {
    crate::localtime::strftime("%Y-%m-%d %H:%M:%S", t as libc::time_t)
}

/// lib/btime.cc StrToUtime(): `sscanf` is Bareos' bsscanf (baconfig.h:455),
/// so "%u-%u-%u %u:%u:%u%15s" reads digits without skipping blanks (no
/// digit is a 0), the blank matches any amount of white space and %15s
/// reads non-blank characters, which must be none. Then DateIsValid and
/// mktime(); 0 for every invalid time.
fn str_to_utime(text: &str) -> i64 {
    let buf = text.as_bytes();
    let is_space = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r');
    if buf.is_empty() {
        return 0;
    }
    let mut pos = 0;
    let mut fields = [0i32; 6];
    for (i, field) in fields.iter_mut().enumerate() {
        let mut value: u64 = 0;
        while pos < buf.len() && buf[pos].is_ascii_digit() {
            value = value.wrapping_mul(10).wrapping_add(u64::from(buf[pos] - b'0'));
            pos += 1;
        }
        *field = value as u32 as i32;
        match b"-- ::".get(i) {
            Some(b' ') => while pos < buf.len() && is_space(buf[pos]) {
                pos += 1;
            },
            Some(&literal) => {
                if buf.get(pos) != Some(&literal) {
                    return 0;
                }
                pos += 1;
            }
            None => {}
        }
    }
    if buf.get(pos).is_some_and(|&c| !is_space(c)) {
        return 0;
    }
    let [year, month, day, hour, min, sec] = fields;
    let (year, month) = (year.wrapping_sub(1900), month.wrapping_sub(1));
    // DateIsValid()
    let leap = { let y = year.wrapping_add(1900); y % 4 == 0 && !(y % 100 == 0 && y % 400 != 0) };
    let days = match month {
        1 => if leap { 29 } else { 28 },
        3 | 5 | 8 | 10 => 30,
        _ => 31,
    };
    if year <= 0 || !(0..=11).contains(&month) || day <= 0 || day > days || !(0..=23).contains(&hour)
        || !(0..=59).contains(&min) || !(0..=59).contains(&sec)
    {
        return 0;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec) = (year, month, day, hour, min, sec);
    tm.tm_isdst = -1;
    match unsafe { libc::mktime(&mut tm) } {
        -1 => 0,
        time => time as i64,
    }
}

/// Value of an item of a resource's directives (last one wins, aliases of
/// the job items resolved).
fn item(dirs: &[bareos_config::raw::Directive], key: &str) -> Option<String> {
    dirs.iter().rev().find(|d| bareos_config::jobdefs::item_key(&d.key) == key)
        .and_then(|d| d.first()).map(str::to_string)
}

/// Directives of a Job (with its JobDefs) or another resource type.
fn resource_directives(ctx: &ExecContext, kind: &str, name: &str) -> Vec<bareos_config::raw::Directive> {
    with_config(ctx, |cfg| if kind == "job" {
        crate::commands::defaults::job_directives(cfg, name)
    } else {
        cfg.raw.iter().find(|r| r.norm_type() == kind && r.name() == Some(name))
            .map(|r| r.directives.clone()).unwrap_or_default()
    }).unwrap_or_default()
}

fn with_config<R>(ctx: &ExecContext, f: impl FnOnce(&crate::director_config::DirectorConfig) -> R) -> Option<R> {
    ctx.config.as_ref().and_then(|c| c.read().ok()).map(|c| f(&c))
}

/// A resource of that type and exact name exists.
fn resource_exists(ctx: &ExecContext, kind: &str, name: &str) -> bool {
    with_config(ctx, |cfg| match kind {
        "job" => cfg.find_job(name).is_some(),
        "pool" => cfg.find_pool(name).is_some(),
        "storage" => cfg.find_storage(name).is_some(),
        "client" => cfg.find_client(name).is_some(),
        "fileset" => cfg.find_fileset(name).is_some(),
        "catalog" => cfg.catalogs.iter().any(|c| c.name == name),
        _ => false,
    }).unwrap_or(false)
}

/// UaContext::Get…ResWithName(): audited ACL check, then the resource.
fn res_with_name(ua: &crate::ua_select::Ua, kind: &str, name: &str) -> Option<String> {
    use crate::acl::Kind;
    let acl = match kind {
        "job" => Kind::Job,
        "pool" => Kind::Pool,
        "storage" => Kind::Storage,
        "client" => Kind::Client,
        "fileset" => Kind::FileSet,
        _ => Kind::Catalog,
    };
    (crate::acl::access_audited(ua.ctx, acl, name) && resource_exists(ua.ctx, kind, name)).then(|| name.to_string())
}

fn job_resource(ctx: &ExecContext, name: &str) -> Option<bareos_config::resources::JobResource> {
    with_config(ctx, |cfg| cfg.find_job(name).cloned()).flatten()
}

/// The first storage of a Pool (`pool->storage->first()`).
fn pool_storage(ctx: &ExecContext, pool: &str) -> Option<String> {
    item(&resource_directives(ctx, "pool", pool), "storage")
}

/// BareosDb::FindJobById(): `SELECT JobId FROM Job WHERE JobId=<id>`.
fn find_job_by_id(ctx: &ExecContext, id: &str) -> bool {
    // The id goes into the statement as given (like the C original); a
    // second statement is never sent.
    if id.contains(';') {
        return false;
    }
    ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows(&format!("SELECT JobId FROM Job WHERE JobId={id}")).ok())
        .is_some_and(|table| !table.rows.is_empty())
}

/// std::getline() splitting of split_string(): no trailing empty part.
fn split_string(text: &str, delim: char) -> Vec<&str> {
    let mut parts: Vec<&str> = text.split(delim).collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}

/// The RunContext of ua_run.h: command line values and the resources
/// chosen for them; kept across `.mod` re-scans and the dialog rounds.
#[derive(Default)]
struct RunContext {
    // Reset by every ScanCommandLineArguments.
    job_name: Option<String>,
    pool_name: Option<String>,
    next_pool_name: Option<String>,
    store_name: Option<String>,
    client_name: Option<String>,
    restore_client_name: Option<String>,
    fileset_name: Option<String>,
    verify_job_name: Option<String>,
    previous_job_name: Option<String>,
    catalog_name: Option<String>,
    backup_format: Option<String>,
    accurate_set: bool,
    spool_data_set: bool,
    // Cleared by ResetRestoreContext once applied.
    jid: Option<String>,
    level_name: Option<String>,
    regexwhere: Option<String>,
    where_: Option<String>,
    bootstrap: Option<String>,
    replace: Option<String>,
    when: Option<String>,
    priority: i32,
    plugin_options: Option<String>,
    since: Option<String>,
    verify_list: Option<String>,
    ignoreduplicatecheck_set: bool,
    consolidate_job: Option<bareos_config::resources::JobResource>,
    // Resources
    job: Option<bareos_config::resources::JobResource>,
    pool: Option<String>,
    next_pool: Option<String>,
    store: Option<String>,
    store_source: String,
    client: Option<String>,
    fileset: Option<String>,
    verify_job: Option<String>,
    catalog: Option<String>,
    is_mod: bool,
    level_override: bool,
    pool_override: bool,
}

/// RUN keywords of ScanCommandLineArguments (ua_run.cc:1770).
const RUN_KEYWORDS: &[&str] = &[
    "job", "jobid", "client", "fd", "fileset", "level", "storage", "sd", "regexwhere", "where",
    "bootstrap", "replace", "when", "priority", "yes", "verifyjob", "files", "catalog", "since",
    "cloned", "verifylist", "migrationjob", "pool", "nextpool", "backupclient", "restoreclient",
    "pluginoptions", "spooldata", "comment", "ignoreduplicatecheck", "accurate", "backupformat",
    "consolidatejob",
];

impl RunContext {
    /// ScanCommandLineArguments (ua_run.cc:1764).
    fn scan(&mut self, ua: &mut crate::ua_select::Ua, ast: &CommandAst) -> bool {
        use crate::acl::Kind;
        let ctx = ua.ctx;
        self.catalog_name = None;
        self.job_name = None;
        self.pool_name = None;
        self.next_pool_name = None;
        self.store_name = None;
        self.client_name = None;
        self.restore_client_name = None;
        self.fileset_name = None;
        self.verify_job_name = None;
        self.previous_job_name = None;
        self.accurate_set = false;
        self.spool_data_set = false;
        self.backup_format = None;
        let mut comment: Option<String> = None;
        let mut consolidate_job_name: Option<String> = None;

        for arg in &ast.arguments {
            let mut kw_ok = false;
            if let Some(keyword) = RUN_KEYWORDS.iter().find(|k| k.eq_ignore_ascii_case(&arg.key)) {
                // Note, yes and run have no value, so do not fail
                let value = match (&arg.value, *keyword) {
                    (Some(value), _) => value.clone(),
                    (None, "yes") => String::new(),
                    (None, _) => {
                        ua.send_msg(format!("Value missing for keyword {}\n", arg.key));
                        return false;
                    }
                };
                macro_rules! once {
                    ($field:expr, $check:expr, $msg:expr) => {{
                        if $field.is_some() && $check {
                            ua.send_msg($msg);
                            return false;
                        }
                        $field = Some(value.clone());
                        kw_ok = true;
                    }};
                }
                let not_mod = !self.is_mod;
                match *keyword {
                    "job" => once!(self.job_name, true, "Job name specified twice.\n"),
                    "jobid" => {
                        if self.jid.is_some() && not_mod {
                            ua.send_msg("JobId specified twice.\n");
                            return false;
                        }
                        self.jid = Some(value.clone());
                        for jobid in split_string(&value, ',') {
                            if !find_job_by_id(ctx, jobid) {
                                ua.send_msg(format!("JobId {jobid} not found in catalog. \n"));
                                return false;
                            }
                        }
                        kw_ok = true;
                    }
                    "client" | "fd" | "backupclient" => once!(self.client_name, true, "Client specified twice.\n"),
                    "fileset" => once!(self.fileset_name, true, "FileSet specified twice.\n"),
                    "level" => once!(self.level_name, true, "Level specified twice.\n"),
                    "storage" | "sd" => once!(self.store_name, true, "Storage specified twice.\n"),
                    "regexwhere" => {
                        if (self.regexwhere.is_some() || self.where_.is_some()) && not_mod {
                            ua.send_msg("RegexWhere or Where specified twice.\n");
                            return false;
                        }
                        self.regexwhere = Some(value.clone());
                        if !crate::acl::access_audited(ctx, Kind::Where, &value) {
                            ua.send_msg("No authorization for \"regexwhere\" specification.\n");
                            return false;
                        }
                        kw_ok = true;
                    }
                    "where" => {
                        if (self.where_.is_some() || self.regexwhere.is_some()) && not_mod {
                            ua.send_msg("Where or RegexWhere specified twice.\n");
                            return false;
                        }
                        self.where_ = Some(value.clone());
                        if !crate::acl::access_audited(ctx, Kind::Where, &value) {
                            ua.send_msg("No authorization for \"where\" specification.\n");
                            return false;
                        }
                        kw_ok = true;
                    }
                    "bootstrap" => once!(self.bootstrap, not_mod, "Bootstrap specified twice.\n"),
                    "replace" => once!(self.replace, not_mod, "Replace specified twice.\n"),
                    "when" => once!(self.when, not_mod, "When specified twice.\n"),
                    "priority" => {
                        if self.priority != 0 && not_mod {
                            ua.send_msg("Priority specified twice.\n");
                            return false;
                        }
                        self.priority = atoi(&value);
                        if self.priority <= 0 {
                            ua.send_msg("Priority must be positive nonzero setting it to 10.\n");
                            self.priority = 10;
                        }
                        kw_ok = true;
                    }
                    "yes" => kw_ok = true,
                    "verifyjob" => once!(self.verify_job_name, true, "Verify Job specified twice.\n"),
                    "files" | "cloned" => kw_ok = true,
                    "catalog" => {
                        self.catalog_name = Some(value.clone());
                        kw_ok = true;
                    }
                    "since" => {
                        self.since = Some(value.clone());
                        kw_ok = true;
                    }
                    "verifylist" => {
                        self.verify_list = Some(value.clone());
                        kw_ok = true;
                    }
                    "migrationjob" => once!(self.previous_job_name, true, "Migration Job specified twice.\n"),
                    "pool" => once!(self.pool_name, true, "Pool specified twice.\n"),
                    "nextpool" => once!(self.next_pool_name, true, "NextPool specified twice.\n"),
                    "restoreclient" => once!(self.restore_client_name, not_mod, "Restore Client specified twice.\n"),
                    "pluginoptions" => {
                        if self.plugin_options.is_some() {
                            ua.send_msg("Plugin Options specified twice.\n");
                            return false;
                        }
                        self.plugin_options = Some(value.clone());
                        if !crate::acl::access_audited(ctx, Kind::PluginOptions, &value) {
                            ua.send_msg("No authorization for \"PluginOptions\" specification.\n");
                            return false;
                        }
                        kw_ok = true;
                    }
                    "spooldata" | "ignoreduplicatecheck" | "accurate" => {
                        let (set, twice, invalid) = match *keyword {
                            "spooldata" => (&mut self.spool_data_set, "Spool flag specified twice.\n", "Invalid spooldata flag.\n"),
                            "accurate" => (&mut self.accurate_set, "Accurate flag specified twice.\n", "Invalid accurate flag.\n"),
                            _ => (&mut self.ignoreduplicatecheck_set, "IgnoreDuplicateCheck flag specified twice.\n",
                                "Invalid ignoreduplicatecheck flag.\n"),
                        };
                        if *set {
                            ua.send_msg(twice);
                            return false;
                        }
                        if crate::ua_select::is_yesno(&value).is_some() {
                            *set = true;
                            kw_ok = true;
                        } else {
                            ua.send_msg(invalid);
                        }
                    }
                    "comment" => {
                        comment = Some(value.clone());
                        kw_ok = true;
                    }
                    "backupformat" => once!(self.backup_format, not_mod, "Backup Format specified twice.\n"),
                    "consolidatejob" => once!(consolidate_job_name, true, "Consolidate Job specified twice.\n"),
                    _ => {}
                }
            }
            // Not a keyword: the job name can be the first keyword without value.
            if !kw_ok {
                if self.job_name.is_none() && arg.value.is_none() {
                    self.job_name = Some(arg.key.clone());
                } else {
                    ua.send_msg(format!("Invalid keyword: {}\n", arg.key));
                    return false;
                }
            }
        }

        if let Some(comment) = &comment {
            if let Err(message) = comment_legal(comment) {
                ua.error_msg(message);
                return false;
            }
        }
        if let Some(name) = self.catalog_name.clone() {
            self.catalog = res_with_name(ua, "catalog", &name);
            if self.catalog.is_none() {
                ua.error_msg(format!("Catalog \"{name}\" not found\n"));
                return false;
            }
        }

        if let Some(name) = self.job_name.clone() {
            self.job = res_with_name(ua, "job", &name).and_then(|n| job_resource(ctx, &n));
            if self.job.is_none() {
                if !name.is_empty() {
                    ua.send_msg(format!("Job \"{name}\" not found\n"));
                }
                self.job = select_job(ua);
            }
        } else if self.job.is_none() {
            ua.send_msg("A job name must be specified.\n");
            self.job = select_job(ua);
        }
        let Some(job) = self.job.clone() else { return false };

        if let Some(name) = self.pool_name.clone() {
            self.pool = res_with_name(ua, "pool", &name);
            if self.pool.is_none() {
                if !name.is_empty() {
                    ua.warning_msg(format!("Pool \"{name}\" not found.\n"));
                }
                self.pool = crate::ua_select::select_pool_resource(ua);
            }
        } else if self.pool.is_none() {
            self.pool = (!job.pool.is_empty()).then(|| job.pool.clone());
        }
        let Some(pool) = self.pool.clone() else { return false };

        if let Some(name) = self.next_pool_name.clone() {
            self.next_pool = res_with_name(ua, "pool", &name);
            if self.next_pool.is_none() {
                if !name.is_empty() {
                    ua.warning_msg(format!("Pool \"{name}\" not found.\n"));
                }
                self.next_pool = crate::ua_select::select_pool_resource(ua);
            }
        } else if self.next_pool.is_none() {
            self.next_pool = item(&resource_directives(ctx, "job", &job.name), "nextpool")
                .or_else(|| item(&resource_directives(ctx, "pool", &pool), "nextpool"));
        }

        if let Some(name) = self.store_name.clone() {
            self.store = res_with_name(ua, "storage", &name);
            self.store_source = "command line".to_string();
            if self.store.is_none() {
                if !name.is_empty() {
                    ua.warning_msg(format!("Storage \"{name}\" not found.\n"));
                }
                self.store = crate::ua_select::select_storage_resource(ua, false);
                self.store_source = "user selection".to_string();
            }
        } else if self.store.is_none() {
            // GetJobStorage(): the Pool's storage first, then the Job's.
            if let Some(storage) = pool_storage(ctx, &job.pool) {
                self.store = Some(storage);
                self.store_source = "Pool resource".to_string();
            } else if let Some(storage) = item(&resource_directives(ctx, "job", &job.name), "storage") {
                self.store = Some(storage);
                self.store_source = "Job resource".to_string();
            }
        }
        // Copy and Migrate jobs determine the read storage when they run.
        if !matches!(job.job_type, JobType::Copy | JobType::Migrate) {
            let Some(store) = self.store.clone() else {
                ua.error_msg("No storage specified.\n");
                return false;
            };
            if !crate::acl::access_audited(ctx, Kind::Storage, &store) {
                ua.error_msg(format!("No authorization. Storage \"{store}\".\n"));
                return false;
            }
        }

        if let Some(name) = self.client_name.clone() {
            self.client = res_with_name(ua, "client", &name);
            if self.client.is_none() {
                // A restore also works for clients only known to the catalog.
                if job.job_type == JobType::Restore {
                    if !client_in_catalog(ctx, &name) || !crate::acl::access_audited(ctx, Kind::Client, &name) {
                        ua.error_msg(format!("Client \"{name}\" not found.\n"));
                        return false;
                    }
                } else {
                    if !name.is_empty() {
                        ua.warning_msg(format!("Client \"{name}\" not found.\n"));
                    }
                    self.client = crate::ua_select::select_client_resource(ua);
                    if self.client.is_none() {
                        return false;
                    }
                }
            }
        } else if self.client.is_none() {
            self.client = (!job.client.is_empty()).then(|| job.client.clone());
        }
        if !self.client_authorized(ua) {
            return false;
        }

        if let Some(name) = self.restore_client_name.clone() {
            self.client = res_with_name(ua, "client", &name);
            if self.client.is_none() {
                if !name.is_empty() {
                    ua.warning_msg(format!("Restore Client \"{name}\" not found.\n"));
                }
                self.client = crate::ua_select::select_client_resource(ua);
                if self.client.is_none() {
                    return false;
                }
            }
        } else if self.client.is_none() {
            self.client = (!job.client.is_empty()).then(|| job.client.clone());
        }
        if !self.client_authorized(ua) {
            return false;
        }

        if let Some(name) = self.fileset_name.clone() {
            self.fileset = res_with_name(ua, "fileset", &name);
            if self.fileset.is_none() {
                ua.send_msg(format!("FileSet \"{name}\" not found.\n"));
                self.fileset = crate::ua_select::select_fileset_resource(ua);
                if self.fileset.is_none() {
                    return false;
                }
            }
        } else if self.fileset.is_none() {
            self.fileset = (!job.fileset.is_empty()).then(|| job.fileset.clone());
        }

        if let Some(name) = self.verify_job_name.clone() {
            self.verify_job = res_with_name(ua, "job", &name);
            if self.verify_job.is_none() {
                ua.send_msg(format!("Verify Job \"{name}\" not found.\n"));
                self.verify_job = select_job(ua).map(|j| j.name);
            }
        } else if self.verify_job.is_none() {
            self.verify_job = item(&resource_directives(ctx, "job", &job.name), "jobtoverify");
        }

        if let Some(name) = self.previous_job_name.clone() {
            if res_with_name(ua, "job", &name).is_none() {
                ua.send_msg(format!("Migration Job \"{name}\" not found.\n"));
                select_job(ua);
            }
        }

        if let Some(name) = consolidate_job_name {
            self.consolidate_job = res_with_name(ua, "job", &name).and_then(|n| job_resource(ctx, &n));
            if self.consolidate_job.is_none() {
                ua.send_msg(format!("Consolidate Job \"{name}\" not found.\n"));
                self.consolidate_job = select_job(ua);
            }
            if let Some(consolidate) = &self.consolidate_job {
                if consolidate.job_type != JobType::Consolidate {
                    ua.error_msg(format!("Invalid Consolidate Job \"{}\". Job type is \"{}\" but expected \"O\".\n",
                        consolidate.name, job_type_char(consolidate.job_type)));
                    return false;
                }
            }
        }
        true
    }

    /// "No authorization. Client …" check of the chosen client.
    fn client_authorized(&self, ua: &mut crate::ua_select::Ua) -> bool {
        match &self.client {
            Some(client) if !crate::acl::access_audited(ua.ctx, crate::acl::Kind::Client, client) => {
                ua.error_msg(format!("No authorization. Client \"{client}\".\n"));
                false
            }
            _ => true,
        }
    }

    /// ResetRestoreContext (ua_run.cc:878): the run context into the jcr.
    fn reset(&mut self, ua: &mut crate::ua_select::Ua, jcr: &mut Jcr) -> bool {
        let ctx = ua.ctx;
        jcr.verify_job = self.verify_job.clone();
        jcr.pool = self.pool.clone();
        jcr.next_pool = self.next_pool.clone();

        if self.pool_name.is_some() {
            jcr.pool_source = "command line".to_string();
            jcr.ignore_level_pool_overrides = true;
        } else if !self.level_override && jcr.pool.as_deref() != Some(jcr.job.pool.as_str()) {
            jcr.pool_source = "user input".to_string();
        }
        jcr.set_rwstorage(self.store.as_deref(), &self.store_source);

        let pool_next_pool = jcr.pool.as_deref().and_then(|p| item(&resource_directives(ctx, "pool", p), "nextpool"));
        if self.next_pool_name.is_some() {
            jcr.npool_source = "command line".to_string();
        } else if jcr.next_pool != pool_next_pool {
            jcr.npool_source = "user input".to_string();
        }

        jcr.client = self.client.clone();
        jcr.fileset = self.fileset.clone();
        if self.catalog.is_some() {
            jcr.catalog = self.catalog.clone();
        }
        if let Some(where_) = self.where_.take() {
            jcr.where_ = Some(where_);
        }
        if let Some(regexwhere) = self.regexwhere.take() {
            jcr.regex_where = Some(regexwhere);
        }
        if let Some(when) = self.when.take() {
            jcr.sched_time = str_to_utime(&when);
            if jcr.sched_time == 0 {
                ua.send_msg("Invalid time, using current time.\n");
                jcr.sched_time = now_unix_time();
            }
        }
        if let Some(bootstrap) = self.bootstrap.take() {
            jcr.restore_bootstrap = Some(bootstrap);
        }
        if let Some(options) = self.plugin_options.take() {
            jcr.plugin_options = Some(options);
        }

        if let Some(replace) = self.replace.take() {
            jcr.replace = 0;
            for (name, token) in REPLACE_OPTIONS {
                if name.eq_ignore_ascii_case(&replace) {
                    jcr.replace = *token;
                }
            }
            if jcr.replace == 0 {
                ua.send_msg(format!("Invalid replace option: {replace}\n"));
                return false;
            }
        } else {
            jcr.replace = job_replace(ctx, &jcr.job);
        }

        if self.priority != 0 {
            jcr.priority = self.priority;
            self.priority = 0;
        }

        if let Some(since) = self.since.clone() {
            if str_to_utime(&since) != 0 {
                self.since = None;
            } else {
                ua.send_msg(format!("Since option expects string in format \"yyyy-mm-dd hh:mm:ss\", but is: {since}\n"));
                return false;
            }
        }

        // If pool changed, update migration write storage
        if matches!(jcr.job_type, JobType::Migrate | JobType::Copy)
            || (jcr.job_type == JobType::Backup && jcr.level == L_VIRTUAL_FULL)
        {
            // SetMigrationWstorage(): the Next Pool and its storage are needed
            // (a missing one is a fatal job message, Jmsg M_FATAL, in C++).
            let Some(storage) = self.next_pool.as_deref().and_then(|p| pool_storage(ctx, p)) else {
                return false;
            };
            jcr.write_storage = Some(storage);
            jcr.wstore_source = Some("Storage from Run NextPool override".to_string());
        }
        self.replace = Some(REPLACE_OPTIONS.iter().find(|(_, token)| *token == jcr.replace)
            .unwrap_or(&REPLACE_OPTIONS[0]).0.to_string());

        if let Some(level) = self.level_name.take() {
            match level_from_name(&level) {
                Some(code) => jcr.level = code,
                None => {
                    ua.send_msg(format!("Level \"{level}\" not valid.\n"));
                    return false;
                }
            }
        }

        if let Some(jid) = self.jid.take() {
            if jcr.job_type == JobType::Backup && jcr.level == L_VIRTUAL_FULL {
                jcr.vf_jobids = Some(jid);
            } else {
                // Note, this is also MigrateJobId and a VerifyJobId
                jcr.restore_job_id = str_to_int64(&jid) as u32;
            }
        }

        if let Some(format) = self.backup_format.take() {
            jcr.backup_format = format;
        }

        if self.consolidate_job.is_some() && jcr.level != L_VIRTUAL_FULL {
            ua.send_msg("Consolidate Job option is only valid for virtual full jobs.\n");
            return false;
        }
        true
    }
}

/// C atoi().
fn atoi(text: &str) -> i32 {
    str_to_int64(text) as i32
}

/// JobType character (JT_*).
fn job_type_char(job_type: JobType) -> char {
    match job_type {
        JobType::Backup => 'B',
        JobType::Restore => 'R',
        JobType::Verify => 'V',
        JobType::Admin => 'D',
        JobType::Archive => 'A',
        JobType::Copy => 'c',
        JobType::Migrate => 'g',
        JobType::Consolidate => 'O',
    }
}

/// IsCommentLegal (ua_input.cc:227).
fn comment_legal(comment: &str) -> Result<(), String> {
    if let Some(c) = comment.chars().find(|c| "'<>&\\\"".contains(*c)) {
        return Err(format!("Illegal character \"{c}\" in a comment.\n"));
    }
    if comment.len() >= 128 {
        return Err("Comment too long.\n".to_string());
    }
    if comment.is_empty() {
        return Err("Comment must be at least one character long.\n".to_string());
    }
    Ok(())
}

/// select_job_resource() with the resource.
fn select_job(ua: &mut crate::ua_select::Ua) -> Option<bareos_config::resources::JobResource> {
    let name = crate::ua_select::select_job_resource(ua)?;
    job_resource(ua.ctx, &name)
}

/// BareosDb::GetClientRecord() by name.
fn client_in_catalog(ctx: &ExecContext, name: &str) -> bool {
    ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows(&format!("SELECT ClientId FROM Client WHERE Name='{}'",
            name.replace('\'', "''"))).ok())
        .is_some_and(|table| !table.rows.is_empty())
}

/// The Job's Replace item (default Always).
fn job_replace(ctx: &ExecContext, job: &bareos_config::resources::JobResource) -> u8 {
    item(&resource_directives(ctx, "job", &job.name), "replace")
        .and_then(|value| REPLACE_OPTIONS.iter().find(|(n, _)| n.eq_ignore_ascii_case(&value)))
        .map(|(_, token)| *token).unwrap_or(b'a')
}

/// The JobControlRecord fields of a run that the dialog shows and changes.
struct Jcr {
    job: bareos_config::resources::JobResource,
    job_type: JobType,
    level: u8,
    pool: Option<String>,
    pool_source: String,
    full_pool: Option<String>,
    inc_pool: Option<String>,
    diff_pool: Option<String>,
    vfull_pool: Option<String>,
    next_pool: Option<String>,
    npool_source: String,
    read_storage: Option<String>,
    rstore_source: Option<String>,
    write_storage: Option<String>,
    wstore_source: Option<String>,
    client: Option<String>,
    fileset: Option<String>,
    catalog: Option<String>,
    priority: i32,
    sched_time: i64,
    backup_format: String,
    restore_bootstrap: Option<String>,
    verify_job: Option<String>,
    where_: Option<String>,
    regex_where: Option<String>,
    plugin_options: Option<String>,
    replace: u8,
    /// RestoreJobId (= MigrateJobId = VerifyJobId)
    restore_job_id: u32,
    vf_jobids: Option<String>,
    ignore_level_pool_overrides: bool,
}

impl Jcr {
    fn new(ctx: &ExecContext, job: &bareos_config::resources::JobResource) -> Self {
        let mut jcr = Self {
            job: job.clone(), job_type: job.job_type, level: 0, pool: None,
            pool_source: "unknown source".to_string(), full_pool: None, inc_pool: None, diff_pool: None,
            vfull_pool: None, next_pool: None, npool_source: "unknown source".to_string(), read_storage: None,
            rstore_source: None, write_storage: None, wstore_source: None, client: None, fileset: None,
            catalog: None, priority: 0, sched_time: now_unix_time(), backup_format: String::new(),
            restore_bootstrap: None, verify_job: None, where_: None, regex_where: None, plugin_options: None,
            replace: 0, restore_job_id: 0, vf_jobids: None, ignore_level_pool_overrides: false,
        };
        jcr.set_defaults(ctx, job);
        jcr
    }

    /// SetJcrDefaults (job.cc:1651).
    fn set_defaults(&mut self, ctx: &ExecContext, job: &bareos_config::resources::JobResource) {
        let dirs = resource_directives(ctx, "job", &job.name);
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        self.job = job.clone();
        self.job_type = job.job_type;
        self.level = match job.job_type {
            JobType::Admin | JobType::Archive => L_NONE,
            _ => item(&dirs, "level").and_then(|l| level_from_name(&l)).unwrap_or(0),
        };
        self.priority = job.priority;
        // Copy storage definitions
        if let Some(storage) = item(&dirs, "storage") {
            self.copy_rwstorage(storage, "Job resource");
        } else if let Some(storage) = some(&job.pool).and_then(|p| pool_storage(ctx, &p)) {
            self.copy_rwstorage(storage, "Pool resource");
        }
        self.client = some(&job.client);
        self.pool_source = "Job resource".to_string();
        self.pool = some(&job.pool);
        self.full_pool = some(&job.full_backup_pool);
        self.inc_pool = some(&job.incremental_backup_pool);
        self.diff_pool = some(&job.differential_backup_pool);
        self.vfull_pool = item(&dirs, "virtualfullbackuppool");
        let pool_catalog = self.pool.as_deref().and_then(|p| item(&resource_directives(ctx, "pool", p), "catalog"));
        self.catalog = pool_catalog.or_else(|| item(&dirs, "catalog")).or_else(|| match &self.client {
            Some(client) => client_catalog(ctx, client),
            None => first_catalog(ctx),
        });
        self.fileset = some(&job.fileset);
        self.backup_format = item(&dirs, "backupformat").unwrap_or_else(|| "Native".to_string());
        self.restore_bootstrap = item(&dirs, "bootstrap");
        self.verify_job = item(&dirs, "jobtoverify");
        if self.level == 0 {
            self.level = match job.job_type {
                JobType::Verify => L_VERIFY_CATALOG,
                JobType::Backup => L_INCREMENTAL,
                JobType::Restore | JobType::Admin => L_NONE,
                _ => L_FULL,
            };
        }
    }

    /// JobControlRecord::JobReads().
    fn job_reads(&self) -> bool {
        match self.job_type {
            JobType::Verify | JobType::Restore | JobType::Copy | JobType::Migrate => true,
            JobType::Backup => self.level == L_VIRTUAL_FULL,
            _ => false,
        }
    }

    /// CopyRwstorage(): the first storage of the list, read and write.
    fn copy_rwstorage(&mut self, storage: String, source: &str) {
        if self.job_reads() {
            self.read_storage = Some(storage.clone());
            self.rstore_source = Some(source.to_string());
        }
        self.write_storage = Some(storage);
        self.wstore_source = Some(source.to_string());
    }

    /// SetRwstorage(): a missing store changes nothing.
    fn set_rwstorage(&mut self, storage: Option<&str>, source: &str) {
        if let Some(storage) = storage {
            self.copy_rwstorage(storage.to_string(), source);
        }
    }

    /// ApplyPoolOverrides (job.cc:1213) without Run overrides.
    fn apply_pool_overrides(&mut self, ctx: &ExecContext, force: bool) {
        if !force && self.ignore_level_pool_overrides {
            return;
        }
        let (pool, source) = match self.level {
            L_FULL => (&self.full_pool, "Job FullPool override"),
            L_VIRTUAL_FULL => (&self.vfull_pool, "Job VFullPool override"),
            L_INCREMENTAL => (&self.inc_pool, "Job IncPool override"),
            L_DIFFERENTIAL => (&self.diff_pool, "Job DiffPool override"),
            _ => return,
        };
        let Some(pool) = pool.clone() else { return };
        self.pool_source = source.to_string();
        if let Some(catalog) = item(&resource_directives(ctx, "pool", &pool), "catalog") {
            self.catalog = Some(catalog);
        }
        self.pool = Some(pool);
    }

    /// The confirmed request for run_selected.
    fn to_ast(&self) -> CommandAst {
        use crate::command_ast::Argument;
        let arg = |key: &str, value: &str| Argument { key: key.to_string(), value: Some(value.to_string()) };
        let mut arguments = vec![arg("job", &self.job.name), arg("level", &job_level_to_string(self.level))];
        for (key, value) in [("client", &self.client), ("fileset", &self.fileset), ("storage", &self.write_storage)] {
            if let Some(value) = value {
                arguments.push(arg(key, value));
            }
        }
        // Interactive runs and `pool=` keep the pool (IgnoreLevelPoolOverrides).
        if let Some(pool) = self.pool.as_ref().filter(|_| self.ignore_level_pool_overrides) {
            arguments.push(arg("pool", pool));
        }
        arguments.push(Argument { key: "yes".to_string(), value: None });
        CommandAst { command_name: "run".to_string(), arguments, ..Default::default() }
    }
}

/// The Catalog of a Client resource (dird_conf.cc:2316: else the first one).
fn client_catalog(ctx: &ExecContext, client: &str) -> Option<String> {
    with_config(ctx, |cfg| cfg.find_client(client).map(|c| c.catalog.clone()).filter(|c| !c.is_empty()))
        .flatten().or_else(|| first_catalog(ctx))
}

fn first_catalog(ctx: &ExecContext) -> Option<String> {
    with_config(ctx, |cfg| cfg.catalogs.first().map(|c| c.name.clone())).flatten()
}

fn nprt(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("*None*")
}

/// printf("%s") of a possibly NULL string (glibc prints "(null)").
fn or_null(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("(null)")
}

/// DisplayJobParameters (ua_run.cc:1223).
fn display_job_parameters(ua: &mut crate::ua_select::Ua, jcr: &mut Jcr, rc: &mut RunContext) -> bool {
    use bareos_bnet::BnetSignal;
    let job_name = rc.job.as_ref().map(|j| j.name.clone()).unwrap_or_default();
    let when = bstrutime(jcr.sched_time);
    let api = ua.api();
    match jcr.job_type {
        JobType::Admin | JobType::Archive | JobType::Consolidate => {
            let kind = match jcr.job_type {
                JobType::Admin => "Admin",
                JobType::Archive => "Archive",
                _ => "Consolidate",
            };
            let head = if api {
                ua.send_signal(BnetSignal::RunCmd);
                format!("Type: {kind}\nTitle: Run {kind} Job\n")
            } else {
                format!("Run {kind} Job\n")
            };
            ua.send_msg(format!("{head}JobName:  {job_name}\nFileSet:  {}\nClient:   {}\nStorage:  {}\n\
                When:     {when}\nPriority: {}\n",
                or_null(&jcr.fileset), or_null(&jcr.client), nprt(&jcr.write_storage), jcr.priority));
            jcr.level = L_FULL;
        }
        JobType::Backup => {
            let is_virtual = jcr.level == L_VIRTUAL_FULL;
            let plugin = jcr.plugin_options.as_ref().map(|o| format!("Plugin Options: {o}\n")).unwrap_or_default();
            let level = job_level_to_string(jcr.level);
            if api {
                ua.send_signal(BnetSignal::RunCmd);
                let next = if is_virtual { format!("NextPool: {}\n", nprt(&jcr.next_pool)) } else { String::new() };
                ua.send_msg(format!("Type: Backup\nTitle: Run Backup Job\nJobName:  {job_name}\nLevel:    {level}\n\
                    Client:   {}\nFormat:   {}\nFileSet:  {}\nPool:     {}\n{next}Storage:  {}\nWhen:     {when}\n\
                    Priority: {}\n{plugin}",
                    or_null(&jcr.client), jcr.backup_format, or_null(&jcr.fileset), nprt(&jcr.pool),
                    nprt(&jcr.write_storage), jcr.priority));
            } else {
                let next = if is_virtual {
                    format!("NextPool: {} (From {})\n", nprt(&jcr.next_pool), jcr.npool_source)
                } else {
                    String::new()
                };
                ua.send_msg(format!("Run Backup job\nJobName:  {job_name}\nLevel:    {level}\nClient:   {}\n\
                    Format:   {}\nFileSet:  {}\nPool:     {} (From {})\n{next}Storage:  {} (From {})\n\
                    When:     {when}\nPriority: {}\n{plugin}",
                    or_null(&jcr.client), jcr.backup_format, or_null(&jcr.fileset), nprt(&jcr.pool),
                    jcr.pool_source, nprt(&jcr.write_storage), or_null(&jcr.wstore_source), jcr.priority));
            }
        }
        JobType::Verify => {
            let name = if let Some(verify_job) = &jcr.verify_job {
                verify_job.clone()
            } else if jcr.restore_job_id != 0 {
                // Display job name if jobid requested
                match job_record_name(ua.ctx, jcr.restore_job_id) {
                    Ok(name) => name,
                    Err(error) => {
                        ua.error_msg(format!("Could not get job record for selected JobId. ERR={error}"));
                        return false;
                    }
                }
            } else {
                String::new()
            };
            let verify_list = rc.verify_list.clone()
                .or_else(|| item(&resource_directives(ua.ctx, "job", &job_name), "writeverifylist"))
                .unwrap_or_default();
            let head = if api {
                ua.send_signal(BnetSignal::RunCmd);
                "Type: Verify\nTitle: Run Verify Job\n"
            } else {
                "Run Verify Job\n"
            };
            ua.send_msg(format!("{head}JobName:     {job_name}\nLevel:       {}\nClient:      {}\nFileSet:     {}\n\
                Pool:        {} (From {})\nStorage:     {} (From {})\nVerify Job:  {name}\nVerify List: {verify_list}\n\
                When:        {when}\nPriority:    {}\n",
                job_level_to_string(jcr.level), or_null(&jcr.client), or_null(&jcr.fileset), nprt(&jcr.pool),
                jcr.pool_source, or_null(&jcr.read_storage), or_null(&jcr.rstore_source), jcr.priority));
        }
        JobType::Restore => {
            if jcr.restore_job_id == 0 && jcr.restore_bootstrap.is_none() {
                if let Some(jid) = &rc.jid {
                    jcr.restore_job_id = str_to_int64(jid) as u32;
                } else {
                    if ua.get_pint("Please enter a JobId for restore: ").is_none() {
                        return false;
                    }
                    let jobid = int64_val(&ua.cmd).to_string();
                    if !find_job_by_id(ua.ctx, &jobid) {
                        ua.send_msg(format!("JobId {jobid} not found in catalog. \n"));
                        return false;
                    }
                    jcr.restore_job_id = int64_val(&ua.cmd) as u32;
                }
            }
            jcr.level = L_FULL; // default level
            let job_dirs = resource_directives(ua.ctx, "job", &job_name);
            let job_regexwhere = item(&job_dirs, "regexwhere");
            let job_where = item(&job_dirs, "where");
            // RegexWhere is taken before RestoreWhere
            let regex = jcr.regex_where.is_some() || (job_regexwhere.is_some() && jcr.where_.is_none());
            let regex_where = jcr.regex_where.clone().or(job_regexwhere);
            let where_ = jcr.where_.clone().or(job_where);
            let replace = rc.replace.clone().unwrap_or_default();
            if jcr.restore_job_id == 0 {
                let (head, location) = match (api, regex) {
                    (true, true) => ("Type: Restore\nTitle: Run Restore Job\n", format!("RegexWhere:      {}\n", or_null(&regex_where))),
                    (true, false) => ("Type: Restore\nTitle: Run Restore job\n", format!("Where:           {}\n", nprt(&where_))),
                    (false, true) => ("Run Restore job\n", format!("RegexWhere:      {}\n", or_null(&regex_where))),
                    (false, false) => ("Run Restore job\n", format!("Where:           {}\n", nprt(&where_))),
                };
                if api {
                    ua.send_signal(BnetSignal::RunCmd);
                }
                ua.send_msg(format!("{head}JobName:         {job_name}\nBootstrap:       {}\n{location}\
                    Replace:         {replace}\nFileSet:         {}\nBackup Client:   {}\nRestore Client:  {}\n\
                    Format:          {}\nStorage:         {}\nWhen:            {when}\nCatalog:         {}\n\
                    Priority:        {}\nPlugin Options:  {}\n",
                    nprt(&jcr.restore_bootstrap), or_null(&jcr.fileset), or_null(&rc.client_name),
                    or_null(&jcr.client), jcr.backup_format, or_null(&jcr.read_storage), or_null(&jcr.catalog),
                    jcr.priority, nprt(&jcr.plugin_options)));
            } else {
                if api {
                    ua.send_signal(BnetSignal::RunCmd);
                }
                let location = if regex {
                    format!("RegexWhere:      {}\n", or_null(&regex_where))
                } else {
                    format!("Where:           {}\n", nprt(&where_))
                };
                ua.send_msg(format!("Run Restore job\nJobName:         {job_name}\nBootstrap:       {}\n{location}\
                    Replace:         {replace}\nClient:          {}\nFormat:          {}\nStorage:         {}\n\
                    JobId:           {}\nWhen:            {when}\nCatalog:         {}\nPriority:        {}\n\
                    Plugin Options:  {}\n",
                    nprt(&jcr.restore_bootstrap), or_null(&jcr.client), jcr.backup_format,
                    or_null(&jcr.read_storage), jcr.restore_job_id, or_null(&jcr.catalog), jcr.priority,
                    nprt(&jcr.plugin_options)));
            }
        }
        JobType::Copy | JobType::Migrate => {
            jcr.level = L_FULL; // default level
            let copy = jcr.job_type == JobType::Copy;
            let jobid = if jcr.restore_job_id == 0 { "*None*".to_string() } else { jcr.restore_job_id.to_string() };
            if api {
                ua.send_signal(BnetSignal::RunCmd);
                let head = if copy { "Type: Copy\nTitle: Run Copy Job\n" } else { "Type: Migration\nTitle: Run Migration Job\n" };
                ua.send_msg(format!("{head}JobName:       {job_name}\nBootstrap:     {}\nRead Storage:  {}\n\
                    Pool:          {}\nWrite Storage: {}\nNextPool:      {}\nJobId:         {jobid}\n\
                    When:          {when}\nCatalog:       {}\nPriority:      {}\n",
                    nprt(&jcr.restore_bootstrap), nprt(&jcr.read_storage), nprt(&jcr.pool),
                    // The C original passes NextPool and Write Storage swapped.
                    nprt(&jcr.next_pool), nprt(&jcr.write_storage), or_null(&jcr.catalog), jcr.priority));
            } else {
                let head = if copy { "Run Copy job\n" } else { "Run Migration job\n" };
                ua.send_msg(format!("{head}JobName:       {job_name}\nBootstrap:     {}\n\
                    Read Storage:  {} (From {})\nPool:          {} (From {})\nWrite Storage: {} (From {})\n\
                    NextPool:      {} (From {})\nJobId:         {jobid}\nWhen:          {when}\n\
                    Catalog:       {}\nPriority:      {}\n",
                    nprt(&jcr.restore_bootstrap), nprt(&jcr.read_storage), or_null(&jcr.rstore_source),
                    nprt(&jcr.pool), jcr.pool_source, nprt(&jcr.write_storage), or_null(&jcr.wstore_source),
                    nprt(&jcr.next_pool), jcr.npool_source, or_null(&jcr.catalog), jcr.priority));
            }
        }
    }
    true
}

/// `ua->int64_val` of the last GetPint answer: (int64_t)strtod().
fn int64_val(answer: &str) -> i64 {
    let Ok(text) = std::ffi::CString::new(answer) else { return 0 };
    (unsafe { libc::strtod(text.as_ptr(), std::ptr::null_mut()) }) as i64
}

/// BareosDb::GetJobRecord() by JobId: the unique Job name.
fn job_record_name(ctx: &ExecContext, jobid: u32) -> Result<String, String> {
    let catalog = ctx.catalog.as_ref().ok_or_else(String::new)?;
    let table = catalog.query_rows(&format!("SELECT Job FROM Job WHERE JobId={jobid}"))?;
    match table.rows.first() {
        Some(row) => Ok(row.first().cloned().flatten().unwrap_or_default()),
        None => Err(format!("No Job found for JobId {jobid}\n")),
    }
}

/// Result of ModifyJobParameters: 0 (show again), 1 (not a mod answer),
/// -1 (cancelled).
enum Modify {
    Again,
    Done,
    Cancel,
}

/// ModifyJobParameters (ua_run.cc:531).
fn modify_job_parameters(ua: &mut crate::ua_select::Ua, jcr: &mut Jcr, rc: &mut RunContext, answer: &str) -> Modify {
    use crate::ua_select;
    if answer.is_empty() || !prefix_of(&answer.to_ascii_lowercase(), "mod") {
        return Modify::Done;
    }
    let ctx = ua.ctx;
    let job_type = jcr.job_type;
    let restore = job_type == JobType::Restore;
    let backup = job_type == JobType::Backup;
    let pooled = matches!(job_type, JobType::Backup | JobType::Copy | JobType::Migrate | JobType::Verify);
    let next_pool_menu = matches!(job_type, JobType::Migrate | JobType::Copy) || (backup && jcr.level == L_VIRTUAL_FULL);
    ua.start_prompt("Parameters to modify:\n");
    for item in ["Level", "Storage", "Job", "FileSet", if restore { "Restore Client" } else { "Client" },
        "Backup Format", "When", "Priority"]
    {
        ua.add_prompt(item);
    }
    if pooled {
        ua.add_prompt("Pool");
        if next_pool_menu {
            ua.add_prompt("NextPool");
            if backup {
                ua.add_prompt("Plugin Options");
            }
        } else if job_type == JobType::Verify {
            ua.add_prompt("Verify Job");
        } else if backup {
            ua.add_prompt("Plugin Options");
        }
    } else if restore {
        for item in ["Bootstrap", "Where", "File Relocation", "Replace", "JobId", "Plugin Options"] {
            ua.add_prompt(item);
        }
    }
    let Some((choice, _)) = ua.do_prompt("", "Select parameter to modify") else { return Modify::Cancel };
    match choice {
        0 => {
            select_job_level(ua, jcr);
            if backup && !rc.pool_override && jcr.level != L_VIRTUAL_FULL {
                jcr.apply_pool_overrides(ctx, true);
                rc.pool = jcr.pool.clone();
                rc.level_override = true;
            }
        }
        1 => {
            let Some(storage) = ua_select::select_storage_resource(ua, false) else { return Modify::Cancel };
            rc.store = Some(storage);
            rc.store_source = "user selection".to_string();
            jcr.set_rwstorage(rc.store.as_deref(), &rc.store_source);
        }
        2 => {
            let Some(job) = select_job(ua) else { return Modify::Cancel };
            rc.job = Some(job.clone());
            jcr.set_defaults(ctx, &job);
        }
        3 => {
            let Some(fileset) = ua_select::select_fileset_resource(ua) else { return Modify::Cancel };
            rc.fileset = Some(fileset);
        }
        4 => {
            let Some(client) = ua_select::select_client_resource(ua) else { return Modify::Cancel };
            rc.client = Some(client);
        }
        5 => {
            let Some(format) = ua.get_cmd("Please enter Backup Format: ") else { return Modify::Cancel };
            jcr.backup_format = format;
        }
        6 => {
            let Some(when) = ua.get_cmd("Please enter desired start time as YYYY-MM-DD HH:MM:SS (return for now): ")
            else {
                return Modify::Cancel;
            };
            jcr.sched_time = if when.is_empty() {
                now_unix_time()
            } else {
                match str_to_utime(&when) {
                    0 => {
                        ua.send_msg("Invalid time, using current time.\n");
                        now_unix_time()
                    }
                    t => t,
                }
            };
        }
        7 => {
            let Some(priority) = ua.get_pint("Enter new Priority: ") else { return Modify::Cancel };
            if priority == 0 {
                ua.send_msg("Priority must be a positive integer.\n");
            } else {
                jcr.priority = priority as i32;
            }
        }
        8 if pooled => {
            let Some(pool) = ua_select::select_pool_resource(ua) else { return Modify::Cancel };
            rc.pool = Some(pool);
            rc.level_override = false;
            rc.pool_override = true;
        }
        8 => {
            // Bootstrap
            let Some(bootstrap) = ua.get_cmd("Please enter the Bootstrap file name: ") else { return Modify::Cancel };
            jcr.restore_bootstrap = None;
            if !bootstrap.is_empty() {
                match std::fs::File::open(&bootstrap) {
                    Ok(_) => jcr.restore_bootstrap = Some(bootstrap),
                    Err(error) => ua.send_msg(format!("Warning cannot open {bootstrap}: ERR={}\n",
                        bstrerror(&error))),
                }
            }
        }
        9 if next_pool_menu => {
            let Some(pool) = ua_select::select_pool_resource(ua) else { return Modify::Cancel };
            rc.next_pool = Some(pool);
        }
        9 if job_type == JobType::Verify => {
            if let Some(job) = select_job(ua) {
                rc.verify_job = Some(job.name);
            }
        }
        9 if restore => {
            let Some(where_) = ua.get_cmd("Please enter the full path prefix for restore (/ for none): ") else {
                return Modify::Cancel;
            };
            if !crate::acl::access_audited(ctx, crate::acl::Kind::Where, &where_) {
                ua.send_msg("No authorization for \"where\" specification.\n");
            } else {
                // cannot use regexwhere and where; "/" is treated as no prefix.
                jcr.regex_where = None;
                jcr.where_ = Some(if where_ == "/" { String::new() } else { where_ });
            }
        }
        9 => get_plugin_options(ua, jcr),
        10 if restore => select_where_regexp(ua, jcr),
        10 if backup => get_plugin_options(ua, jcr),
        10 => return Modify::Cancel,
        11 => {
            ua.start_prompt("Replace:\n");
            for (name, _) in REPLACE_OPTIONS {
                ua.add_prompt(name);
            }
            if let Some((opt, name)) = ua.do_prompt("", "Select replace option") {
                rc.replace = Some(name);
                jcr.replace = REPLACE_OPTIONS[opt].1;
            }
        }
        12 => {
            rc.jid = None; // force reprompt
            jcr.restore_job_id = 0;
            if jcr.restore_bootstrap.is_some() {
                ua.send_msg("You must set the bootstrap file to NULL to be able to specify a JobId.\n");
            }
        }
        13 => get_plugin_options(ua, jcr),
        _ => {}
    }
    Modify::Again
}

/// GetPluginOptions (ua_run.cc).
fn get_plugin_options(ua: &mut crate::ua_select::Ua, jcr: &mut Jcr) {
    if let Some(options) = ua.get_cmd("Please enter Plugin Options string: ") {
        if !crate::acl::access_audited(ua.ctx, crate::acl::Kind::PluginOptions, &options) {
            ua.send_msg("No authorization for \"PluginOptions\" specification.\n");
            return;
        }
        jcr.plugin_options = Some(options);
    }
}

/// SelectJobLevel (ua_run.cc).
fn select_job_level(ua: &mut crate::ua_select::Ua, jcr: &mut Jcr) {
    let levels: &[(&str, u8)] = match jcr.job_type {
        JobType::Backup => &[("Full", L_FULL), ("Incremental", L_INCREMENTAL), ("Differential", L_DIFFERENTIAL),
            ("Since", L_SINCE), ("VirtualFull", L_VIRTUAL_FULL)],
        JobType::Verify => &[("Initialize Catalog", L_VERIFY_INIT), ("Verify Catalog", L_VERIFY_CATALOG),
            ("Verify Volume to Catalog", L_VERIFY_VOLUME_TO_CATALOG),
            ("Verify Disk to Catalog", L_VERIFY_DISK_TO_CATALOG),
            ("Verify Volume Data (not yet implemented)", L_VERIFY_DATA)],
        _ => {
            ua.warning_msg("Level not appropriate for this Job. Cannot be changed.\n");
            return;
        }
    };
    ua.start_prompt("Levels:\n");
    for (name, _) in levels {
        ua.add_prompt(name);
    }
    if let Some((index, _)) = ua.do_prompt("", "Select level") {
        jcr.level = levels[index].1;
    }
}

/// SelectWhereRegexp (ua_run.cc:1090): build a RegexWhere from strip
/// prefix, add prefix, add suffix or an own regexp.
fn select_where_regexp(ua: &mut crate::ua_select::Ua, jcr: &mut Jcr) {
    let (mut strip_prefix, mut add_prefix, mut add_suffix, mut rwhere) =
        (None::<String>, None::<String>, None::<String>, None::<String>);
    loop {
        // try_again_reg:
        ua.send_msg(format!("strip_prefix={} add_prefix={} add_suffix={}\n",
            nprt(&strip_prefix), nprt(&add_prefix), nprt(&add_suffix)));
        ua.start_prompt("This will replace your current Where value\n");
        for item in ["Strip prefix", "Add prefix", "Add file suffix", "Enter a regexp", "Test filename manipulation",
            "Use this ?"]
        {
            ua.add_prompt(item);
        }
        let Some((choice, _)) = ua.do_prompt("", "Select parameter to modify") else { return };
        match choice {
            0 => {
                if let Some(text) = ua.get_cmd("Please enter the path prefix to strip: ") {
                    strip_prefix = Some(text);
                }
                continue;
            }
            1 => {
                if let Some(text) = ua.get_cmd("Please enter the path prefix to add (/ for none): ") {
                    add_prefix = Some(if text == "/" { String::new() } else { text });
                }
                continue;
            }
            2 => {
                if let Some(text) = ua.get_cmd("Please enter the file suffix to add: ") {
                    add_suffix = Some(text);
                }
                continue;
            }
            3 => {
                if let Some(text) = ua.get_cmd("Please enter a valid regexp (!from!to!): ") {
                    if !crate::acl::access_audited(ua.ctx, crate::acl::Kind::Where, &text) {
                        ua.send_msg("Denied by \"WhereACL\" configuration.\n");
                        continue;
                    }
                    rwhere = Some(text);
                }
                continue;
            }
            4 => {
                let regs = match rwhere.as_deref().filter(|r| !r.is_empty()) {
                    Some(rwhere) => {
                        ua.send_msg(format!("regexwhere={rwhere}\n"));
                        crate::breg::get_bregexps(rwhere)
                    }
                    None => {
                        let regexp = crate::breg::build_where(strip_prefix.as_deref(), add_prefix.as_deref(),
                            add_suffix.as_deref());
                        ua.send_msg(format!("strip_prefix={} add_prefix={} add_suffix={} result={regexp}\n",
                            nprt(&strip_prefix), nprt(&add_prefix), nprt(&add_suffix)));
                        crate::breg::get_bregexps(&regexp)
                    }
                };
                let Some(regs) = regs else {
                    ua.send_msg("Cannot use your regexp\n");
                    continue;
                };
                ua.send_msg("Enter a period (.) to stop this test\n");
                while let Some(name) = ua.get_cmd("Please enter filename to test: ") {
                    let result = crate::breg::apply_bregexps(&name, &regs);
                    ua.send_msg(format!("{name} -> {result}\n"));
                }
                continue;
            }
            5 => {}
            _ => continue,
        }
        // Use this: replace the existing where and regexwhere
        jcr.where_ = None;
        jcr.regex_where = None;
        if let Some(rwhere) = &rwhere {
            jcr.regex_where = Some(rwhere.clone());
        } else if strip_prefix.is_some() || add_prefix.is_some() || add_suffix.is_some() {
            let regexp = crate::breg::build_where(strip_prefix.as_deref(), add_prefix.as_deref(), add_suffix.as_deref());
            if !crate::acl::access_audited(ua.ctx, crate::acl::Kind::Where, &regexp) {
                ua.send_msg(format!("Regex ({regexp}) denied by \"WhereACL\" configuration.\n"));
                continue;
            }
            jcr.regex_where = Some(regexp);
        }
        if jcr.regex_where.as_deref().and_then(crate::breg::get_bregexps).is_none() {
            jcr.regex_where = None;
            ua.send_msg("Cannot use your regexp.\n");
        }
        return;
    }
}

/// BErrNo::bstrerror() of an I/O error.
fn bstrerror(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(errno) => unsafe { std::ffi::CStr::from_ptr(libc::strerror(errno)) }.to_string_lossy().into_owned(),
        None => error.to_string(),
    }
}

fn run_selected(
    ast: &CommandAst,
    ctx: &ExecContext,
    config: std::sync::RwLockReadGuard<'_, crate::director_config::DirectorConfig>,
    job: bareos_config::resources::JobResource,
) -> CommandResult {
    let job_name = job.name.as_str();
    let yes = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes"));

    // Only Backup jobs have a job worker; other types are never started as
    // (bogus) backups.
    if job.job_type != JobType::Backup {
        return CommandResult::error(format!("Running {:?} jobs is not available yet.\n", job.job_type));
    }
    if !job.enabled {
        return CommandResult::error(format!("Job '{job_name}' is disabled\n"));
    }

    let default_level = format!("{:?}", job.level);
    let requested_level = ast.arguments.iter()
        .find(|a| a.key == "level").and_then(|a| a.value.as_deref()).unwrap_or(&default_level);

    let client_name = ast.arguments.iter()
        .find(|a| a.key == "client")
        .and_then(|a| a.value.as_deref())
        .unwrap_or(&job.client);

    let storage_name = ast.arguments.iter()
        .find(|a| a.key == "storage")
        .and_then(|a| a.value.as_deref())
        .unwrap_or(&job.storage);

    let fileset_name = ast.arguments.iter().find(|a| a.key == "fileset")
        .and_then(|a| a.value.as_deref()).unwrap_or(&job.fileset);
    let client_res = config.find_client(client_name);
    let storage_res = config.find_storage(storage_name);
    let fileset_res = config.find_fileset(fileset_name);
    if client_res.is_none() { return CommandResult::error(format!("Client '{client_name}' not found\n")); }
    if storage_res.is_none() { return CommandResult::error(format!("Storage '{storage_name}' not found\n")); }
    if fileset_res.is_none() { return CommandResult::error(format!("FileSet '{fileset_name}' not found\n")); }

    // ua_run.cc:408 — without `yes` the run dialog (run_with_ua) asks first;
    // run_selected only starts confirmed requests.
    if !yes {
        return CommandResult::error("Job not run.\n".to_string());
    }

    let (level, level_char, since_utime, prev_job) = match backup_level(requested_level, |level| {
        let cat = ctx.catalog.as_ref().ok_or("A catalog is required for incremental/differential backups")?;
        cat.since_utime(client_name, &job.name, fileset_name, level)
    }) {
        Ok(value) => value,
        Err(error) => return CommandResult::error(format!("run: {error}\n")),
    };
    // Use the effective level's pool after any missing-base upgrade to Full.
    let default_pool = match level_char {
        "F" if !job.full_backup_pool.is_empty() => &job.full_backup_pool,
        "D" if !job.differential_backup_pool.is_empty() => &job.differential_backup_pool,
        "I" if !job.incremental_backup_pool.is_empty() => &job.incremental_backup_pool,
        _ => &job.pool,
    };
    let pool = ast.arguments.iter().find(|a| a.key == "pool")
        .and_then(|a| a.value.as_deref()).unwrap_or(default_pool);

    let client = client_res.unwrap();
    let storage = storage_res.unwrap();
    let dir_name = config.director.name.clone();

    // Unique job name
    let now = chrono::Local::now();
    let unique_job = format!("{}.{}", job.name, now.format("%Y-%m-%d_%H.%M.%S_%f"));

    // Create job record in DB — JobId comes from the database
    let job_type_str = match job.job_type { JobType::Backup => "B", JobType::Restore => "R", _ => "B" };
    let job_id = if let Some(cat) = &ctx.catalog {
        match cat.create_job(
            &unique_job, &job.name, job_type_str, level_char,
            client_name, pool, fileset_name, now.timestamp(),
        ) {
            Ok(id) => id as u32,
            Err(e) => {
                tracing::error!("Failed to create job record: {e}");
                return CommandResult::error(format!("Failed to create job record: {e}\n"));
            }
        }
    } else {
        NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed)
    };

    // Build JobParams for the autonomous worker
    let params = JobParams {
        job_id,
        unique_job: unique_job.clone(),
        job_name: job.name.clone(),
        job_type: WorkerJobType::Backup,
        level: level.clone(),
        fd_address: client.address.clone(),
        fd_port: client.port,
        fd_password: client.password.clone(),
        sd_address: storage.address.clone(),
        sd_port: storage.port,
        sd_password: storage.password.clone(),
        sd_device: storage.device.clone(),
        sd_media_type: storage.media_type.clone(),
        director_name: dir_name,
        pool_name: pool.to_string(),
        fileset_includes: fileset_res.map(|f| f.include_blocks.clone()).unwrap_or_default(),
        fileset_excludes: fileset_res.map(|f| f.exclude_blocks.clone()).unwrap_or_default(),
        since_utime,
        prev_job,
        // Write Bootstrap from job config
        where_path: None,
        replace: None,
        restore_jobid: None,
        bootstrap: if !job.write_bootstrap.is_empty() { Some(job.write_bootstrap.clone()) } else { None },
    };

    let resources = (client.name.clone(), storage.name.clone());
    drop(config); // Release RwLock before fork

    // Post queued status
    if let Some(smgr) = &ctx.session_mgr {
        smgr.bus().post(BusMessage::JobStatus {
            job_id, job_name: job.name.clone(), status: "Queued".to_string(),
            timestamp: now_unix(),
        });
        crate::console_msg::post_job_message(smgr.bus(), ctx.config.as_ref(), BusMessage::JobMessage {
            job_id, job_name: job.name.clone(),
            msg_type: crate::message_bus::MsgType::Info,
            text: format!("Start Backup JobId={job_id}, Job={}, Level={level}", job.name),
            timestamp: now_unix(),
        });
    }

    // RunJob(): the job queue forks the worker once its priority and the
    // Maximum Concurrent Jobs allow it (Compatibility log C-048).
    if let Some(ref mgr) = ctx.job_manager {
        mgr.lock().unwrap().enqueue_job(job_id, &job.name, crate::job_manager::QueuedStart {
            params, priority: job.priority, sched_time: now.timestamp(),
        }, &resources.0, &resources.1);
    } else {
        match job_worker::spawn_job_worker(job_id, &job.name, &params) {
            Ok(handle) => tracing::info!("Job {job_id} '{}': worker pid={}", job.name, handle.pid),
            Err(e) => {
                tracing::error!("Job {job_id}: fork failed: {e}");
                return CommandResult::error(format!("Failed to start job: {e}\n"));
            }
        }
    }

    let output = format!(
        "Job queued. JobId={job_id}\n\
         Run {:?} job\n\
         Job:        {}\n\
         Level:      {level}\n\
         Client:     {client_name}\n\
         Storage:    {storage_name}\n\
         Pool:       {pool}\n\
         FileSet:    {}\n\
         Priority:   {}\n\
         When:       now\n",
        job.job_type, job.name, fileset_name, job.priority,
    );

    CommandResult::ok(output)
}

/// Decide the effective backup level before creating its catalog record or worker.
fn backup_level(
    requested: &str,
    mut lookup: impl FnMut(&str) -> Result<Option<(i64, String)>, String>,
) -> Result<(String, &'static str, i64, String), String> {
    let (name, code) = match requested.to_ascii_lowercase().as_str() {
        "full" | "f" => ("full", "F"),
        "incremental" | "i" => ("incremental", "I"),
        "differential" | "d" => ("differential", "D"),
        _ => return Err(format!("Unsupported backup level: {requested}")),
    };
    if code == "F" { return Ok((name.into(), code, 0, String::new())); }
    match lookup(code)? {
        Some((timestamp, previous)) if timestamp > 0 => Ok((name.into(), code, timestamp, previous)),
        _ => {
            info!("No matching Full backup chain for {name}; upgrading to Full");
            Ok(("full".into(), "F", 0, String::new()))
        }
    }
}

/// Bareos str_to_int64(): leading blanks, optional sign, leading digits.
pub(crate) fn str_to_int64(value: &str) -> i64 {
    let value = value.trim_start();
    let (negative, digits) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let number = digits.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |n, d| n.wrapping_mul(10).wrapping_add(i64::from(d - b'0')));
    if negative { -number } else { number }
}

pub fn cmd_enable(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    en_disable(ast, ctx, true)
}

pub fn cmd_disable(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    en_disable(ast, ctx, false)
}

/// DoEnDisableCmd (ua_cmds.cc:1022): `schedule`, else `client`, else a job.
/// There is no storage branch — `storage=` and missing names lead to the
/// select_enable_disable_*_resource selection (ua_select.cc), never to a
/// toggle of something else (C-012).
fn en_disable(ast: &CommandAst, ctx: &ExecContext, enable: bool) -> CommandResult {
    use crate::acl::{access, access_audited, Kind};
    let config = match &ctx.config {
        Some(c) => c,
        None => return CommandResult::error("No config loaded\n".to_string()),
    };
    let has = |key: &str| ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key));
    let value = |key: &str| crate::ua_select::find_arg_with_value(ast, key);
    let (res_type, label, kind) = if has("schedule") { ("schedule", "Schedule", Kind::Schedule) }
        else if has("client") { ("client", "Client", Kind::Client) }
        else { ("job", "Job", Kind::Job) };
    // Get{Schedule,Client,Job}ResWithName(): the ACL check is audited.
    let (name, shown) = match value(res_type) {
        Some(name) if access_audited(ctx, kind, name) => (name.to_string(), String::new()),
        Some(name) => return CommandResult::ok_with_errors(String::new(), format!("{label} \"{name}\" not found.\n")),
        None => {
            let candidates: Vec<String> = {
                let cfg = config.read().unwrap();
                let states: Vec<(&str, bool)> = match res_type {
                    "schedule" => cfg.schedules.iter().map(|s| (s.name.as_str(), s.enabled)).collect(),
                    "client" => cfg.clients.iter().map(|c| (c.name.as_str(), c.enabled)).collect(),
                    _ => cfg.jobs.iter().map(|j| (j.name.as_str(), j.enabled)).collect(),
                };
                crate::director_config::sort_case_insensitive(states.into_iter()
                    .filter(|(name, enabled)| *enabled != enable && access(ctx, kind, name)).map(|(name, _)| name))
                    .into_iter().map(str::to_string).collect()
            };
            // select_enable_disable_*_resource: DoPrompt dialog (C-011).
            let mut ua = crate::ua_select::Ua::new(ctx);
            ua.start_prompt(&format!("The defined {label} resources are:\n"));
            for name in &candidates {
                ua.add_prompt(name);
            }
            let msg = if res_type == "job" { "Select Job resource".to_string() } else { format!("Select {label} resource") };
            // EnableCmd/DisableCmd return true even when nothing was selected.
            let Some((_, name)) = ua.do_prompt(label, &msg) else { return ua.finish(true) };
            ua.send_result(toggle_enabled(config, res_type, &name, enable));
            return ua.finish(true);
        }
    };
    crate::ua_db::with_prefix(shown, toggle_enabled(config, res_type, &name, enable))
}

/// ua_cmds.cc DoEnDisableCmd(): an unknown resource is an ErrorMsg, the
/// command itself still returns true.
fn toggle_enabled(config: &crate::director_config::SharedConfig, res_type: &str, name: &str, enable: bool) -> CommandResult {
    let action = if enable { "enabled" } else { "disabled" };
    let mut cfg = config.write().unwrap();
    let (label, found) = match res_type {
        "client" => ("Client", cfg.clients.iter_mut().find(|c| c.name == name).map(|c| c.enabled = enable)),
        "job" => ("Job", cfg.jobs.iter_mut().find(|j| j.name == name).map(|j| j.enabled = enable)),
        "schedule" => ("Schedule", cfg.schedules.iter_mut().find(|s| s.name == name).map(|s| s.enabled = enable)),
        _ => return CommandResult::ok(format!("{res_type}: not supported\n")),
    };
    match found {
        // ObjectStart(action) + ArrayStart("<type>s") with the one name.
        Some(()) => CommandResult {
            json: Some(serde_json::json!({ &action[..action.len() - 1]: { format!("{res_type}s"): [name] } })),
            ..CommandResult::ok(format!("{label} \"{name}\" {action}\n"))
        }.with_warning(format!(
            "{}abling is a temporary operation until the director reloads.\n\
             For a permanent setting, please set the value of the \"Enabled\"\n\
             directive in the relevant configuration resource file.\n",
            if enable { "En" } else { "Dis" })),
        None => CommandResult::ok_with_errors(String::new(), format!("{label} \"{name}\" not found.\n")),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs() as i64
}

#[cfg(test)]
mod error_text_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str("Director { Name = dir }").unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    #[test]
    fn enable_disable_unknown_resource_is_an_error_message_of_a_successful_command() {
        let ctx = context();
        for (line, expected) in [("enable job=nosuch", "Job \"nosuch\" not found.\n"),
            ("disable client=nosuch", "Client \"nosuch\" not found.\n")] {
            let ast = crate::session::parse_cli_command(line);
            let result = if line.starts_with("enable") { cmd_enable(&ast, &ctx) } else { cmd_disable(&ast, &ctx) };
            assert!(result.success, "{line}");
            assert_eq!(result.error_message, expected);
        }
    }

    #[test]
    fn enable_disable_success_warns_about_the_temporary_change() {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nJob { Name = a; Type = Backup }").unwrap();
        let ctx = ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let warning = "abling is a temporary operation until the director reloads.\n\
            For a permanent setting, please set the value of the \"Enabled\"\n\
            directive in the relevant configuration resource file.\n";
        let result = cmd_disable(&crate::session::parse_cli_command("disable job=a"), &ctx);
        assert_eq!(result.text_output, format!("Job \"a\" disabled\nDis{warning}"));
        assert_eq!(result.messages, vec![(crate::command_queue::MsgKind::Warning, format!("Dis{warning}"))]);
        assert_eq!(result.json, Some(serde_json::json!({"disable": {"jobs": ["a"]}})));
        let result = cmd_enable(&crate::session::parse_cli_command("enable job=a"), &ctx);
        assert_eq!(result.text_output, format!("Job \"a\" enabled\nEn{warning}"));
    }

    #[test]
    fn enable_disable_never_toggle_storages_or_guess_a_resource() {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nStorage { Name = File; Address = localhost; Enabled = no }\n\
             Job { Name = a; Type = Backup }\nJob { Name = b; Type = Backup }").unwrap();
        let ctx = ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None };
        let storage_state = ctx.config.as_ref().unwrap().read().unwrap().find_storage("File").unwrap().enabled;
        use crate::ua_select::tests::scripted;
        for line in ["enable storage=File", "enable", "enable job"] {
            let (result, _) = scripted(&[], || cmd_enable(&crate::session::parse_cli_command(line), &ctx));
            assert_eq!(result.text_output, "The defined Job resources are:\n", "{line}");
            assert_eq!(result.error_message, "Selection list for \"Job\" is empty!\n", "{line}");
        }
        let (_, console) = scripted(&["."], || cmd_disable(&crate::session::parse_cli_command("disable storage=File"), &ctx));
        assert_eq!(console.transcript(), "The defined Job resources are:\n1: a\n2: b\nSelect Job resource (1-2): .\n\
            Selection aborted, nothing done.\n");
        let cfg = ctx.config.as_ref().unwrap().read().unwrap();
        assert_eq!(cfg.find_storage("File").unwrap().enabled, storage_state);
        assert!(cfg.jobs.iter().all(|j| j.enabled));
    }
}

#[cfg(test)]
mod run_confirmation_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn context(jobs: &str) -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(&format!(
            "Director {{ Name = dir }}\nClient {{ Name = fd; Address = localhost; Password = x }}\n\
             Storage {{ Name = File; Address = localhost; Password = x; Device = d; Media Type = File }}\n\
             FileSet {{ Name = fs; Include {{ File = /etc }} }}\nPool {{ Name = Full; Pool Type = Backup }}\n{jobs}"
        )).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    const JOB: &str = "Job { Name = b1; Type = Backup; Level = Incremental; Client = fd; FileSet = fs; Storage = File; Pool = Full; Messages = m; Priority = 10 }";

    fn run(line: &str, ctx: &ExecContext) -> CommandResult {
        cmd_run(&crate::session::parse_cli_command(line), ctx)
    }

    /// Runs `line` with scripted answers; the start hook must never fire.
    fn dialog(line: &str, ctx: &ExecContext, answers: &[&str]) -> (bool, crate::ua_select::tests::Scripted) {
        crate::ua_select::tests::scripted(answers, || {
            let mut ua = crate::ua_select::Ua::new(ctx);
            let ok = run_with_ua(&mut ua, &crate::session::parse_cli_command(line),
                |_, _, _| panic!("{line}: {answers:?} must not start a job"));
            ua.finish(ok);
            ok
        })
    }

    #[test]
    fn missing_or_unknown_job_ends_in_the_selection() {
        let ctx = context(&format!("{JOB}\n{}", JOB.replace("b1", "A2")));
        // Without console: batch mode lists the choices.
        assert_eq!(run("run", &ctx).text_output,
            "A job name must be specified.\nThe defined Job resources are:\n1: A2\n2: b1\n\
             Your request has multiple choices for \"Job\". Selection is not possible in batch mode.\n");
        let (ok, console) = dialog("run job=nosuch yes", &ctx, &["0", "x", "."]);
        assert!(!ok);
        assert_eq!(console.transcript(),
            "Job \"nosuch\" not found\nThe defined Job resources are:\n1: A2\n2: b1\nSelect Job resource (1-2): 0\n\
             Please enter a number between 1 and 2\nSelect Job resource (1-2): x\n\
             Expected a positive integer, got: x\nSelect Job resource (1-2): .\nSelection aborted, nothing done.\n");
    }

    /// Context whose catalog and job manager record what a run would do.
    pub(crate) fn recording_context(jobs: &str) -> (ExecContext, Arc<std::sync::Mutex<Vec<String>>>) {
        let mut ctx = context(jobs);
        let (catalog, log) = crate::commands::admin::side_effect_tests::recording_catalog("b1");
        ctx.catalog = Some(catalog);
        ctx.job_manager = Some(Arc::new(std::sync::Mutex::new(crate::job_manager::JobManager::new(
            Arc::new(crate::message_bus::MessageBus::new(8)), ctx.config.clone(), None))));
        (ctx, log)
    }

    #[test]
    fn run_rerun_without_yes_create_and_start_nothing() {
        let (ctx, log) = recording_context(JOB);
        // No console: GetCmd fails, "Job not run.".
        for line in ["run", "run job=b1", "run job=b1 level=Full pool=Full", "run job=nosuch"] {
            let result = cmd_run(&crate::session::parse_cli_command(line), &ctx);
            assert!(result.text_output.ends_with("Job not run.\n"), "{line}: {}", result.text_output);
        }
        // Console answers that never start.
        for answers in [&["no"][..], &["n"], &["."], &["mod", "."], &["maybe", "no"], &["mod", "8", "."]] {
            let (ctx2, log2) = recording_context(JOB);
            let (ok, console) = crate::ua_select::tests::scripted(answers, || cmd_run(
                &crate::session::parse_cli_command("run job=b1"), &ctx2).success);
            assert!(!ok, "{answers:?}");
            assert!(console.text().ends_with("Job not run.\n"), "{answers:?}: {}", console.text());
            assert!(!log2.lock().unwrap().iter().any(|request| request == "CreateJob"), "{answers:?}");
        }
        let rerun = crate::commands::extra::cmd_rerun(&crate::session::parse_cli_command("rerun jobid=1"), &ctx);
        // The double has no Job record: reRunJob stops after the announcement.
        assert_eq!(rerun.text_output, "rerunning jobid 1\n");
        assert!(!log.lock().unwrap().iter().any(|request| request == "CreateJob"), "{:?}", log.lock().unwrap());
        assert!(ctx.job_manager.as_ref().unwrap().lock().unwrap().running_jobs().is_empty());
        // Control: with `yes` the double sees the job record request (and
        // refuses it, so nothing is forked).
        let started = cmd_run(&crate::session::parse_cli_command("run job=b1 level=Full yes"), &ctx);
        assert!(!started.success);
        assert!(log.lock().unwrap().iter().any(|request| request == "CreateJob"));
    }

    #[test]
    fn job_without_yes_is_displayed_and_asked() {
        let ctx = context(JOB);
        let result = run("run job=b1", &ctx);
        assert!(!result.success);
        assert!(result.text_output.starts_with("Run Backup job\nJobName:  b1\nLevel:    Incremental\nClient:   fd\n\
            Format:   Native\nFileSet:  fs\nPool:     Full (From Job resource)\nStorage:  File (From Job resource)\nWhen:     "),
            "{}", result.text_output);
        assert!(result.text_output.ends_with("Priority: 10\nJob not run.\n"));
        // A single job is selected automatically, then the question.
        let (ok, console) = dialog("run", &ctx, &["no"]);
        assert!(!ok);
        let text = console.transcript();
        assert!(text.starts_with("A job name must be specified.\nAutomatically selected Job: b1\nRun Backup job\n"), "{text}");
        assert!(text.ends_with("Priority: 10\nOK to run? (yes/mod/no): no\nJob not run.\n"), "{text}");
        assert_eq!(console.signals(), vec![bareos_bnet::BnetSignal::SubPrompt]);
    }

    #[test]
    fn illegal_answers_and_modify_dialog() {
        let ctx = context(JOB);
        let (_, console) = dialog("run job=b1", &ctx, &["yes please", "mod", "1", "1", "mod", "8", "2", "no"]);
        let text = console.transcript();
        assert!(text.contains("OK to run? (yes/mod/no): yes please\nIllegal response yes please\n"), "{text}");
        assert!(text.contains("Parameters to modify:\n 1: Level\n 2: Storage\n 3: Job\n 4: FileSet\n 5: Client\n\
            \x206: Backup Format\n 7: When\n 8: Priority\n 9: Pool\n10: Plugin Options\n\
            Select parameter to modify (1-10): 1\nLevels:\n1: Full\n2: Incremental\n3: Differential\n4: Since\n\
            5: VirtualFull\nSelect level (1-5): 1\nRun Backup job\nJobName:  b1\nLevel:    Full\n"), "{text}");
        assert!(text.contains("Select parameter to modify (1-10): 8\nEnter new Priority: 2\nRun Backup job\n"), "{text}");
        assert!(text.contains("Priority: 2\nOK to run? (yes/mod/no): no\nJob not run.\n"), "{text}");
        // Cancel inside the menu ends the run.
        let (_, console) = dialog("run job=b1", &ctx, &["mod", "."]);
        assert!(console.transcript().ends_with("Select parameter to modify (1-10): .\n\
            Selection aborted, nothing done.\nJob not run.\n"));
        // Single storage/pool are selected automatically; ".mod" re-scans.
        let (_, console) = dialog("run job=b1", &ctx, &["mod", "9", ".mod level=Differential", "no"]);
        let text = console.transcript();
        assert!(text.contains("Select parameter to modify (1-10): 9\nAutomatically selected Pool: Full\nRun Backup job\n"), "{text}");
        // The same pool as the Job's keeps its source (ResetRestoreContext).
        assert!(text.contains("Pool:     Full (From Job resource)\n"), "{text}");
        assert!(text.contains(".mod level=Differential\nRun Backup job\nJobName:  b1\nLevel:    Differential\n"), "{text}");
        assert!(run_answer_valid("") && run_answer_valid("y") && run_answer_valid("Ye") && run_answer_valid("n")
            && run_answer_valid("m") && run_answer_valid(".mo") && run_answer_valid("mod x"));
        assert!(!run_answer_valid("yess") && !run_answer_valid("x") && !run_answer_valid("modx"));
    }

    #[test]
    fn api_mode_signals_run_cmd() {
        let mut ctx = context(JOB);
        ctx.api_mode = crate::command_executor::ApiMode::Api;
        let (_, console) = dialog("run job=b1", &ctx, &["no"]);
        use bareos_bnet::BnetSignal::*;
        assert_eq!(console.signals(), vec![RunCmd, TextInput]);
        assert!(console.text().starts_with("Type: Backup\nTitle: Run Backup Job\nJobName:  b1\n"));
    }

    const RESTORE: &str = "Job { Name = r1; Type = Restore; Client = fd; FileSet = fs; Storage = File; Pool = Full; \
        Messages = m; Where = /tmp/r }";

    #[test]
    fn restore_jobs_ask_for_a_jobid_first() {
        let ctx = context(RESTORE);
        // C-043: "no" is not a number; without a catalog every JobId is unknown.
        let (ok, console) = dialog("run job=r1", &ctx, &["no", "7"]);
        assert!(!ok);
        assert_eq!(console.transcript(), "Please enter a JobId for restore: no\n\
            Expected a positive integer, got: no\nPlease enter a JobId for restore: 7\n\
            JobId 7 not found in catalog. \nJob not run.\n");
        // With a bootstrap no JobId is needed: the Where layout.
        let (_, console) = dialog("run job=r1 bootstrap=/x.bsr replace=never", &ctx, &["mod", "11", ".", "no"]);
        let text = console.transcript();
        assert!(text.starts_with("Run Restore job\nJobName:         r1\nBootstrap:       /x.bsr\n\
            Where:           /tmp/r\nReplace:         Never\nFileSet:         fs\nBackup Client:   (null)\n\
            Restore Client:  fd\nFormat:          Native\nStorage:         File\nWhen:            "), "{text}");
        assert!(text.contains("Priority:        10\nPlugin Options:  *None*\nOK to run? (yes/mod/no): mod\n\
            Parameters to modify:\n 1: Level\n 2: Storage\n 3: Job\n 4: FileSet\n 5: Restore Client\n\
            \x206: Backup Format\n 7: When\n 8: Priority\n 9: Bootstrap\n10: Where\n11: File Relocation\n\
            12: Replace\n13: JobId\n14: Plugin Options\nSelect parameter to modify (1-14): 11\n\
            strip_prefix=*None* add_prefix=*None* add_suffix=*None*\n"), "{text}");
        assert!(text.contains("Select parameter to modify (1-6): .\nSelection aborted, nothing done.\nRun Restore job\n"),
            "{text}");
        assert!(text.ends_with("OK to run? (yes/mod/no): no\nJob not run.\n"), "{text}");
    }

    #[test]
    fn other_job_types_are_shown_and_never_started_as_backups() {
        let ctx = context(&format!("{JOB}\nJob {{ Name = a1; Type = Admin; Client = fd; FileSet = fs; Storage = File; \
            Pool = Full; Messages = m }}"));
        let (ok, console) = dialog("run job=a1", &ctx, &["no"]);
        assert!(!ok);
        let text = console.transcript();
        assert!(text.starts_with("Run Admin Job\nJobName:  a1\nFileSet:  fs\nClient:   fd\nStorage:  File\nWhen:     "),
            "{text}");
        assert!(text.ends_with("Priority: 10\nOK to run? (yes/mod/no): no\nJob not run.\n"), "{text}");
        let job = ctx.config.as_ref().unwrap().read().unwrap().find_job("a1").cloned().unwrap();
        let result = start_job(&crate::session::parse_cli_command("run job=a1 yes"), &ctx, job);
        assert!(!result.success);
        assert_eq!(result.error_message, "Running Admin jobs is not available yet.\n");
    }

    #[test]
    fn times_are_read_like_bsscanf_and_date_is_valid() {
        // Reference values: libbareos.so StrToUtime() of the installed Director.
        let base = str_to_utime("2030-01-02 03:04:05");
        assert!(base > 0);
        for same in ["2030-1-2 3:4:5", "2030-01-02 03:04:05 x", "2030-01-02 03:04:05 ", "2030-001-02 03:04:05",
            "2030-01-02  03:04:05", "2030-01-02 03:04:05\n"]
        {
            assert_eq!(str_to_utime(same), base, "{same}");
        }
        assert_eq!(str_to_utime("2030-01-02 03:04:056"), base + 51);
        for invalid in ["", "2030-01-02 03:04:05x", "2030-02-30 03:04:05", "2023-02-29 00:00:00", "2030-01-02 23:04:60",
            "2030-01-02T03:04:05", "+2030-01-02 03:04:05", " 2030-01-02 03:04:05", "2030-01-02 03:04",
            "999-01-02 03:04:05", "2030-00-02 03:04:05", "2030-01-02 24:00:00", "2030-01-02 03:04:05.5"]
        {
            assert_eq!(str_to_utime(invalid), 0, "{invalid}");
        }
        assert!(str_to_utime("2024-02-29 00:00:00") > 0);
    }
}

#[cfg(test)]
mod backup_level_tests {
    use super::*;

    #[test]
    fn valid_incremental_and_differential_keep_their_reference() {
        for (requested, code) in [("Incremental", "I"), ("D", "D")] {
            let (_, effective, since, previous) = backup_level(requested, |selected| {
                assert_eq!(selected, code);
                Ok(Some((123456, "previous-job".into())))
            }).unwrap();
            assert_eq!((effective, since, previous.as_str()), (code, 123456, "previous-job"));
        }
    }

    #[test]
    fn missing_chain_upgrades_but_errors_do_not_become_full_backups() {
        assert_eq!(backup_level("I", |_| Ok(None)).unwrap().1, "F");
        assert!(backup_level("D", |_| Err("catalog unavailable".into())).is_err());
        assert!(backup_level("VirtualFull", |_| panic!("invalid level must not query")).is_err());
        assert_eq!(backup_level("Full", |_| panic!("full must not query")).unwrap().1, "F");
    }
}
