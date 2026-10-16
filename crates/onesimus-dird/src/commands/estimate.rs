//! `estimate` — port of C++ ua_cmds.cc EstimateCmd().
//!
//! Connects to the File Daemon like a backup does (fd_cmds.cc
//! SendJobInfoToFileDaemon, SendLevelCommand, SendIncludeExcludeLists,
//! backup.cc SendAccurateCurrentFiles), sends `estimate listing=N` and relays
//! every FD message until the next signal. The level of an Incremental or
//! Differential estimate comes from the catalog (job.cc GetLevelSinceTime).

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};
use bareos_config::raw::Directive;
use bareos_config::resources::JobType;
use chrono::TimeZone;

use crate::acl::{self, Kind};
use crate::catalog_worker::CatalogHandle;
use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::commands::defaults::{bool_directive, job_directives, time_directive};
use crate::commands::show::{default_options, parse_options, FileOptions};
use crate::director_config::DirectorConfig;
use crate::fd_protocol;

/// C++ JobLevel codes used by estimate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    Full,
    Base,
    Incremental,
    Differential,
    Since,
    Other,
}

impl Level {
    /// ua_select.cc GetLevelFromName() over dird_conf.cc joblevels[].
    fn from_name(name: &str) -> Option<Self> {
        let level = match name.to_ascii_lowercase().as_str() {
            "full" => Self::Full,
            "base" => Self::Base,
            "incremental" => Self::Incremental,
            "differential" => Self::Differential,
            "since" => Self::Since,
            "virtualfull" | "catalog" | "initcatalog" | "volumetocatalog" | "disktocatalog" | "data" => Self::Other,
            _ => return None,
        };
        Some(level)
    }
}

/// Text output and ErrorMsg lines in C++ emission order.
#[derive(Default)]
struct Reply {
    out: String,
    err: String,
}

impl Reply {
    fn error(mut self, message: &str) -> CommandResult {
        self.err.push_str(message);
        let mut result = CommandResult::error(self.err);
        result.text_output = self.out;
        result.errors_first = true;
        result
    }

    fn done(self) -> CommandResult {
        let errors_first = !self.err.is_empty();
        let mut result = CommandResult::ok_with_errors(self.out, self.err);
        result.errors_first = errors_first;
        result
    }
}

/// Everything EstimateCmd takes from the configuration (copied out of the lock).
struct Plan {
    director: String,
    client: String,
    address: String,
    port: u16,
    password: String,
    fileset_name: String,
    fileset: Option<FilesetItems>,
    /// Options of the job's own FileSet (backup.cc IsChecksumNeededByFileset)
    job_fileset_opts: Vec<String>,
    job_name: String,
    job_accurate: bool,
    max_diff_interval: u64,
    max_full_interval: u64,
    max_vfull_interval: u64,
}

/// FilesetResource include/exclude items as SendIncludeExcludeItems sends them.
struct FilesetItems {
    enable_vss: bool,
    includes: Vec<Item>,
    excludes: Vec<Item>,
}

struct Item {
    ignoredir: Vec<String>,
    options: Vec<FileOptions>,
    files: Vec<String>,
    plugins: Vec<String>,
}

fn fileset_items(cfg: &DirectorConfig, name: &str) -> Option<FilesetItems> {
    let res = cfg.raw.iter().find(|r| r.norm_type() == "fileset" && r.name() == Some(name))?;
    let item = |body: &Vec<Directive>| {
        let values = |key: &str| -> Vec<String> {
            body.iter().filter(|d| d.norm_key() == key).flat_map(|d| d.values()).map(str::to_string).collect()
        };
        let mut options: Vec<FileOptions> = body.iter().filter(|d| d.norm_key() == "options")
            .filter_map(|d| d.block.as_ref()).map(|b| parse_options(b)).collect();
        if options.is_empty() {
            // inc_conf.cc StoreDefaultOptions()
            options.push(FileOptions { opts: default_options(&[]), ..Default::default() });
        }
        Item { ignoredir: values("excludedircontaining"), options, files: values("file"), plugins: values("plugin") }
    };
    let blocks = |key: &str| -> Vec<Item> {
        res.directives.iter().filter(|d| d.norm_key() == key).filter_map(|d| d.block.as_ref()).map(item).collect()
    };
    let includes = blocks("include");
    let excludes = blocks("exclude");
    if includes.is_empty() && excludes.is_empty() {
        return None; // !fileset->new_include: nothing is sent
    }
    let enable_vss = res.directives.iter().rev().find(|d| d.norm_key() == "enablevss")
        .and_then(|d| d.first())
        .is_some_and(|v| bareos_config::values::parse_bool(v).map(|(_, v)| v).unwrap_or(false));
    Some(FilesetItems { enable_vss, includes, excludes })
}

fn arg_is(key: &str, name: &str) -> bool {
    key.eq_ignore_ascii_case(name)
}

pub fn cmd_estimate(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut reply = Reply::default();
    let Some(config) = &ctx.config else {
        return reply.error("No job specified.\n");
    };

    // Argument loop of EstimateCmd(): resources are looked up with ACL checks.
    let mut job: Option<String> = None;
    let mut client: Option<String> = None;
    let mut fileset: Option<String> = None;
    let mut listing = false;
    let mut level = Level::Full;
    let mut accurate: Option<bool> = None;
    let need_job_selection;
    {
        let cfg = match config.read() {
            Ok(cfg) => cfg,
            Err(_) => return reply.error("Config read error\n"),
        };
        for arg in &ast.arguments {
            let key = arg.key.as_str();
            let value = arg.value.as_deref();
            if arg_is(key, "client") || arg_is(key, "fd") {
                let Some(name) = value else { return reply.error("Client name missing.\n") };
                if !acl::access(ctx, Kind::Client, name) || cfg.find_client(name).is_none() {
                    return reply.error(&format!("Client \"{name}\" not found.\n"));
                }
                client = Some(name.to_string());
            } else if arg_is(key, "job") {
                let Some(name) = value else { return reply.error("Job name missing.\n") };
                if !acl::access(ctx, Kind::Job, name) || cfg.find_job(name).is_none() {
                    return reply.error(&format!("Job \"{name}\" not found.\n"));
                }
                job = Some(name.to_string());
            } else if arg_is(key, "fileset") {
                let Some(name) = value else { return reply.error("Fileset name missing.\n") };
                if !acl::access(ctx, Kind::FileSet, name) || cfg.find_fileset(name).is_none() {
                    return reply.error(&format!("Fileset \"{name}\" not found.\n"));
                }
                fileset = Some(name.to_string());
            } else if arg_is(key, "listing") {
                listing = true;
            } else if arg_is(key, "level") {
                let Some(name) = value else { return reply.error("Level value missing.\n") };
                match Level::from_name(name) {
                    Some(l) => level = l,
                    None => reply.err.push_str(&format!("Level \"{name}\" not valid.\n")),
                }
            } else if arg_is(key, "accurate") {
                let Some(v) = value else { return reply.error("Accurate value missing.\n") };
                // ua_input.cc IsYesno(): false for anything but yes/no
                let yes = v.eq_ignore_ascii_case("yes");
                if !yes && !v.eq_ignore_ascii_case("no") {
                    reply.err.push_str("Invalid value for accurate. It must be yes or no.\n");
                }
                accurate = Some(yes);
            }
        }

        // select_job_resource() follows below, without the configuration lock.
        need_job_selection = job.is_none() && !(client.is_some() && fileset.is_some());
        if job.is_none() && !need_job_selection {
            // job = ua->GetJobResWithName(ua->argk[1])
            let first = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
            if acl::access(ctx, Kind::Job, first) && cfg.find_job(first).is_some() {
                job = Some(first.to_string());
            } else {
                return reply.error("No job specified.\n");
            }
        }
    }

    if !need_job_selection {
        return estimate_job(ast, ctx, reply, job, client, fileset, listing, level, accurate);
    }
    // select_job_resource() (ua_select.cc): the console dialog, after the
    // ErrorMsg lines of the argument loop.
    let mut ua = crate::ua_select::Ua::new(ctx);
    ua.send_result(reply.done());
    let Some(name) = crate::ua_select::select_job_resource(&mut ua) else { return ua.finish(false) };
    let result = estimate_job(ast, ctx, Reply::default(), Some(name), client, fileset, listing, level, accurate);
    let success = result.success;
    ua.send_result(result);
    ua.finish(success)
}

/// EstimateCmd() once the job is known.
#[allow(clippy::too_many_arguments)]
fn estimate_job(ast: &CommandAst, ctx: &ExecContext, mut reply: Reply, job: Option<String>, client: Option<String>,
    fileset: Option<String>, listing: bool, mut level: Level, accurate: Option<bool>) -> CommandResult {
    let Some(config) = &ctx.config else {
        return reply.error("No job specified.\n");
    };
    let plan = {
        let cfg = config.read().unwrap_or_else(|e| e.into_inner());
        let job_res = cfg.find_job(job.as_deref().unwrap_or("")).cloned().unwrap_or_default();
        if job_res.job_type != JobType::Backup {
            return reply.error(&format!("Wrong job specified of type {:?}.\n", job_res.job_type));
        }
        let client_name = client.unwrap_or_else(|| job_res.client.clone());
        let fileset_name = fileset.unwrap_or_else(|| job_res.fileset.clone());
        let Some(client_res) = cfg.find_client(&client_name) else {
            return reply.error("No client specified or selected.\n");
        };
        if cfg.find_fileset(&fileset_name).is_none() {
            return reply.error("No fileset specified or selected.\n");
        }
        let dirs = job_directives(&cfg, &job_res.name);
        let job_fileset_opts = fileset_items(&cfg, &job_res.fileset)
            .map(|f| f.includes.iter().flat_map(|i| i.options.iter().map(|o| o.opts.clone())).collect())
            .unwrap_or_default();
        Plan {
            director: cfg.director.name.clone(),
            client: client_res.name.clone(),
            address: client_res.address.clone(),
            port: client_res.port,
            password: client_res.password.clone(),
            fileset: fileset_items(&cfg, &fileset_name),
            fileset_name,
            job_fileset_opts,
            job_name: job_res.name.clone(),
            job_accurate: bool_directive(&dirs, "accurate", false),
            max_diff_interval: time_directive(&dirs, "maxdiffinterval"),
            max_full_interval: time_directive(&dirs, "maxfullinterval"),
            max_vfull_interval: time_directive(&dirs, "maxvirtualfullinterval"),
        }
    };

    // CloseDb(ua); ua->catalog = ...; OpenDb(ua)
    match crate::ua_db::reopen_client_db(ast, ctx, &plan.client) {
        Ok(prefix) => reply.out.push_str(&prefix),
        Err(mut result) => {
            reply.err.push_str(&result.error_message);
            result.text_output.insert_str(0, &reply.out);
            result.error_message = reply.err;
            result.errors_first = true;
            return result;
        }
    }

    let start_time = chrono::Local::now().timestamp();
    let catalog = ctx.catalog.as_ref();
    // GetOrCreateClientRecord / GetOrCreateFilesetRecord (lookup only).
    let client_id = catalog.and_then(|c| catalog_id(c, &format!(
        "SELECT ClientId FROM Client WHERE Name='{}'", escape(&plan.client))));
    let fileset_id = catalog.and_then(|c| catalog_id(c, &format!(
        "SELECT FileSetId FROM FileSet WHERE FileSet='{}' ORDER BY CreateTime DESC, FileSetId DESC LIMIT 1",
        escape(&plan.fileset_name))));
    let (Some(catalog), Some(client_id), Some(fileset_id)) = (catalog, client_id, fileset_id) else {
        return reply.done();
    };

    let since = level_since_time(catalog, &plan, &mut level, client_id, fileset_id);

    reply.out.push_str(&format!("Connecting to Client {} at {}:{}\n", plan.client, plan.address, plan.port));
    let mut fd = match fd_protocol::connect_fd_with_timeout(&plan.address, plan.port, &plan.password, &plan.director, None) {
        Ok((fd, _)) => fd,
        Err(error) => {
            tracing::warn!("estimate: connect to client {}: {error}", plan.client);
            return reply.error("Failed to connect to Client.\n");
        }
    };
    let accurate = accurate.unwrap_or(plan.job_accurate);
    let result = estimate_on_fd(&mut fd, catalog, &plan, level, accurate, since, listing, start_time,
        client_id, fileset_id, &mut reply);
    // bail_out: signal(BNET_TERMINATE)
    let _ = fd.send_signal(BnetSignal::Terminate);
    match result {
        Err(message) if message.is_empty() => reply.done(),
        Err(message) => reply.error(&message),
        Ok(()) => reply.done(),
    }
}

/// Since time of an Incremental/Differential estimate (`(stime, prev_job)`),
/// upgrading `level` to Full like job.cc GetLevelSinceTime().
fn level_since_time(catalog: &CatalogHandle, plan: &Plan, level: &mut Level, client_id: i64, fileset_id: i64)
    -> (String, String) {
    if !matches!(*level, Level::Incremental | Level::Differential) {
        return (String::new(), String::new());
    }
    let now = chrono::Local::now().timestamp();
    let name = escape(&plan.job_name);
    let query = |levels: &str| format!(
        "SELECT StartTime, Job FROM Job WHERE JobStatus IN ('T','W') AND Type='B' AND \
         Level IN ({levels}) AND Name='{name}' AND ClientId={client_id} AND FileSetId={fileset_id} \
         ORDER BY StartTime DESC LIMIT 1");
    let row = |sql: String| -> Option<(String, String)> {
        let table = catalog.query_rows(&sql).ok()?;
        let row = table.rows.first()?;
        Some((row.first()?.clone().unwrap_or_default(), row.get(1)?.clone().unwrap_or_default()))
    };
    let full = row(query("'F'"));
    // sql_find.cc FindJobStartTime(): Differential is since the last Full,
    // Incremental since the last Full/Incremental/Differential.
    let mut do_full = false;
    let since = match (*level, &full) {
        (_, None) => { do_full = true; None }
        (Level::Differential, Some(_)) => full.clone(),
        _ => row(query("'I','D','F'")),
    };
    if since.is_none() {
        do_full = true;
    }
    let last_full = full.as_ref().map(|(t, _)| str_to_utime(t)).unwrap_or(0);
    let mut do_diff = false;
    if full.is_some() && *level == Level::Incremental && plan.max_diff_interval > 0 {
        let last_diff = row(query("'D'")).map(|(t, _)| str_to_utime(&t)).unwrap_or(last_full).max(last_full);
        do_diff = now - last_diff >= plan.max_diff_interval as i64;
    }
    let mut do_vfull = false;
    if full.is_some() && plan.max_full_interval > 0 {
        do_full = now - last_full >= plan.max_full_interval as i64;
    } else if full.is_some() && plan.max_vfull_interval > 0 {
        do_vfull = now - last_full >= plan.max_vfull_interval as i64;
    }
    if do_full {
        *level = Level::Full;
    } else if do_vfull {
        *level = Level::Other;
    } else if do_diff {
        *level = Level::Differential;
    }
    since.unwrap_or_default()
}

/// lib/btime.cc StrToUtime(): catalog time as local time.
fn str_to_utime(text: &str) -> i64 {
    chrono::NaiveDateTime::parse_from_str(text.get(..19).unwrap_or(text), "%Y-%m-%d %H:%M:%S")
        .ok()
        .and_then(|t| chrono::Local.from_local_datetime(&t).earliest())
        .map(|t| t.timestamp())
        .unwrap_or(0)
}

fn escape(value: &str) -> String {
    value.replace('\'', "''")
}

fn catalog_id(catalog: &CatalogHandle, sql: &str) -> Option<i64> {
    catalog.query_rows(sql).ok()?.rows.first()?.first()?.as_deref()?.parse().ok()
}

fn send<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, text: &str) -> Result<(), String> {
    fd.send_str(text).map_err(|e| e.to_string())
}

/// getmsg.cc response(): the next response must be exactly `expected`.
fn response<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, expected: &str) -> Result<bool, String> {
    Ok(fd_protocol::get_dir_msg(fd)?.is_some_and(|m| m == expected))
}

#[allow(clippy::too_many_arguments)]
fn estimate_on_fd<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, catalog: &CatalogHandle, plan: &Plan,
    level: Level, accurate: bool, since: (String, String), listing: bool, start_time: i64, client_id: i64,
    fileset_id: i64, reply: &mut Reply) -> Result<(), String> {
    const CONNECT_FAILED: &str = "Failed to connect to Client.\n";
    // SendJobInfoToFileDaemon(): JobId 0, the console's unique job name.
    let job = format!("-Console-.{}_{:02}",
        chrono::Local.timestamp_opt(start_time, 0).single().map(|t| t.format("%Y-%m-%d_%H.%M.%S").to_string())
            .unwrap_or_default(), 1);
    send(fd, &format!("JobId=0 Job={job} SDid=0 SDtime=0 Authorization=dummy ssl=4\n"))
        .map_err(|_| CONNECT_FAILED.to_string())?;
    match fd_protocol::get_dir_msg(fd).map_err(|_| CONNECT_FAILED.to_string())? {
        Some(message) if message.starts_with("2000 OK Job") => {}
        _ => return Err(CONNECT_FAILED.into()),
    }

    // SendLevelCommand()
    let accurate_prefix = if accurate { "accurate_" } else { "" };
    let level_cmd = |prefix: &str, name: &str| format!("level = {prefix}{name}  mtime_only=0 \n");
    match level {
        Level::Base => send(fd, &level_cmd("", "base"))?,
        Level::Full => send(fd, &level_cmd("", "full"))?,
        Level::Differential | Level::Incremental => {
            let name = if level == Level::Differential { "differential" } else { "incremental" };
            send(fd, &level_cmd(accurate_prefix, name))?;
            // SendSinceTime()
            send(fd, &format!("level =  since_utime {} mtime_only=0 prev_job={}\n", str_to_utime(&since.0).max(0), since.1))?;
            while fd_protocol::get_dir_msg(fd)?.is_some() {}
        }
        // L_SINCE and verify/virtual levels: Jmsg "Unimplemented backup level", bail out
        Level::Since | Level::Other => return Err(String::new()),
    }
    if !response(fd, "2000 OK level\n")? {
        return Err(String::new());
    }

    // SendIncludeExcludeLists()
    if let Some(items) = &plan.fileset {
        send(fd, &format!("fileset{}\n", if items.enable_vss { " vss=1" } else { "" }))?;
        for (code, list) in [("I\n", &items.includes), ("E\n", &items.excludes)] {
            for item in list {
                send(fd, code)?;
                send_item(fd, item)?;
            }
        }
        fd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())?;
        if !response(fd, "2000 OK include\n")? {
            return Err("Error sending include and exclude lists.\n".into());
        }
    }

    send_accurate_files(fd, catalog, plan, level, accurate, start_time, client_id, fileset_id)?;

    send(fd, &format!("estimate listing={}\n", listing as i32))?;
    loop {
        match fd.recv() {
            Ok(BnetMessage::Data(bytes)) => reply.out.push_str(&String::from_utf8_lossy(&bytes)),
            _ => return Ok(()),
        }
    }
}

/// fd_sendfileset.cc SendFilesetIgnoredir/SendFilesetOptions/SendFilesetFileAndPlugin.
fn send_item<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, item: &Item) -> Result<(), String> {
    for dir in &item.ignoredir {
        send(fd, &format!("Z {dir}\n"))?;
    }
    for fo in &item.options {
        let enhanced_wild = fo.opts.contains('W');
        send(fd, &format!("O {}\n", fo.opts))?;
        let lists: [(&str, &Vec<String>); 9] = [("R", &fo.regex), ("RD", &fo.regexdir), ("RF", &fo.regexfile),
            ("W", &fo.wild), ("WD", &fo.wilddir), ("WF", &fo.wildfile),
            (if enhanced_wild { "WB" } else { "WF" }, &fo.wildbase), ("X", &fo.fstype), ("XD", &fo.drivetype)];
        for (code, values) in lists {
            for value in values {
                send(fd, &format!("{code} {value}\n"))?;
            }
        }
        if let Some(plugin) = &fo.plugin {
            send(fd, &format!("G {plugin}\n"))?;
        }
        send(fd, "N\n")?;
    }
    for file in &item.files {
        send_list_item(fd, "F ", file)?;
    }
    send(fd, "N\n")?;
    for plugin in &item.plugins {
        send_list_item(fd, "P ", plugin)?;
    }
    send(fd, "N\n")
}

/// fd_sendfileset.cc SendListItem(): `|program`, `<file` or a plain name.
fn send_list_item<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, code: &str, item: &str) -> Result<(), String> {
    let lines = |text: String| -> Vec<String> { text.split_inclusive('\n').map(|l| format!("{code}{l}")).collect() };
    let messages = if let Some(program) = item.strip_prefix('|') {
        let output = std::process::Command::new("/bin/sh").arg("-c").arg(program).output()
            .map_err(|e| format!("Cannot run program: {program}. ERR={e}"))?;
        if !output.status.success() {
            return Err(String::new());
        }
        lines(String::from_utf8_lossy(&output.stdout).into_owned())
    } else if let Some(path) = item.strip_prefix('<') {
        lines(std::fs::read_to_string(path).map_err(|_| String::new())?)
    } else {
        vec![format!("{code}{}", item.strip_prefix('\\').unwrap_or(item))]
    };
    for message in messages {
        send(fd, &message)?;
    }
    Ok(())
}

/// backup.cc IsChecksumNeededByFileset() for a non-base job.
fn checksum_needed(opts: &[String], level: Level) -> bool {
    for opts in opts {
        let mut in_block = false;
        for c in opts.chars() {
            match c {
                'V' => in_block = false, // only for verify jobs
                'C' => in_block = level != Level::Full,
                ':' => in_block = false,
                '5' | '1' if in_block => return true,
                _ => {}
            }
        }
    }
    false
}

/// backup.cc SendAccurateCurrentFiles() with cats AccurateGetJobids() and
/// GetFileList(). Full level would need Base jobs, which are not configured.
#[allow(clippy::too_many_arguments)]
fn send_accurate_files<S: std::io::Read + std::io::Write>(fd: &mut BnetStream<S>, catalog: &CatalogHandle,
    plan: &Plan, level: Level, accurate: bool, start_time: i64, client_id: i64, fileset_id: i64) -> Result<(), String> {
    if !accurate || matches!(level, Level::Base | Level::Full) {
        return Ok(());
    }
    let date = chrono::Local.timestamp_opt(start_time + 1, 0).single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default();
    let ids = |sql: String| -> Result<Vec<String>, String> {
        Ok(catalog.query_rows(&sql)?.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect())
    };
    let mut jobids = ids(format!(
        "SELECT JobId FROM Job JOIN FileSet USING (FileSetId) WHERE ClientId = {client_id} AND Level = 'F' \
         AND JobStatus IN ('T','W') AND Type = 'B' AND StartTime < '{date}' AND FileSet.FileSetId = {fileset_id} \
         ORDER BY Job.JobId DESC, Job.JobTDate DESC LIMIT 1"))?;
    if level == Level::Incremental && !jobids.is_empty() {
        let later = |selected: &[String], extra: &str, limit: &str| format!(
            "SELECT JobId FROM Job JOIN FileSet USING (FileSetId) WHERE ClientId = {client_id} {extra} \
             AND JobStatus IN ('T','W') AND Type='B' \
             AND StartTime > (SELECT EndTime FROM Job WHERE JobId IN ({}) ORDER BY EndTime DESC LIMIT 1) \
             AND StartTime < '{date}' \
             AND FileSet.FileSet= (SELECT FileSet FROM FileSet WHERE FileSetId = {fileset_id}) \
             ORDER BY Job.JobTDate DESC {limit}", selected.join(","));
        let diff = ids(later(&jobids, "AND JobFiles > 0 AND Level='D'", "LIMIT 1"))?;
        jobids.extend(diff);
        let incrementals = ids(later(&jobids, "AND Level='I'", ""))?;
        jobids.extend(incrementals);
    }
    if jobids.is_empty() {
        // Jmsg(M_FATAL, "Cannot find previous jobids.")
        return Err(String::new());
    }
    let list = jobids.join(",");
    let jobids = ids(format!("SELECT JobId FROM Job WHERE JobId IN ({list}) ORDER BY JobTDate ASC"))?.join(",");
    let count = catalog.query_rows(&format!("SELECT sum(JobFiles) FROM Job WHERE JobId IN ({jobids})"))?
        .rows.first().and_then(|r| r.first().cloned().flatten()).unwrap_or_default();
    send(fd, &format!("accurate files={count}\n"))?;

    let use_md5 = checksum_needed(&plan.job_fileset_opts, level);
    let files = catalog.query_rows(&format!(
        "SELECT Path.Path, T1.Name, T1.FileIndex, T1.JobId, LStat, DeltaSeq, MD5 FROM ( \
         SELECT DISTINCT ON (Name, PathId) JobTDate, JobId, FileId, FileIndex, PathId, FileName AS Name, \
         LStat, MD5, DeltaSeq FROM (SELECT FileId, JobId, PathId, Name AS FileName, FileIndex, LStat, MD5, \
         DeltaSeq FROM File WHERE JobId IN ({jobids}) UNION ALL SELECT File.FileId, File.JobId, PathId, \
         File.Name AS FileName, File.FileIndex, LStat, MD5, DeltaSeq FROM BaseFiles JOIN File USING (FileId) \
         WHERE BaseFiles.JobId IN ({jobids}) ) AS T JOIN Job USING (JobId) ORDER BY Name, PathId, JobTDate DESC \
         ) AS T1 JOIN Path ON (Path.PathId = T1.PathId) WHERE FileIndex > 0 ORDER BY T1.JobTDate, FileIndex ASC"))?;
    for row in files.rows {
        let field = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        if field(2).starts_with('0') {
            continue;
        }
        let md5 = field(6);
        let checksum = if use_md5 && md5.len() > 1 { md5 } else { String::new() };
        let mut message = format!("{}{}", field(0), field(1)).into_bytes();
        for part in [field(4), checksum, field(5)] {
            message.push(0);
            message.extend_from_slice(part.as_bytes());
        }
        fd.send(&message).map_err(|e| e.to_string())?;
    }
    fd.send_signal(BnetSignal::Eod).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_names_follow_joblevels_table() {
        assert_eq!(Level::from_name("incremental"), Some(Level::Incremental));
        assert_eq!(Level::from_name("FULL"), Some(Level::Full));
        assert_eq!(Level::from_name("VirtualFull"), Some(Level::Other));
        assert_eq!(Level::from_name("bogus"), None);
    }

    #[test]
    fn checksum_only_inside_accurate_option_for_non_full() {
        let opts = vec!["S4C5:HAX".to_string()];
        assert!(checksum_needed(&opts, Level::Incremental));
        assert!(!checksum_needed(&opts, Level::Full));
        assert!(!checksum_needed(&["S4HAX".to_string()], Level::Incremental));
    }

    #[test]
    fn fileset_items_carry_default_options_and_vss() {
        let cfg = DirectorConfig::from_str(r#"
Director { Name = dir; Password = x }
FileSet { Name = fs; Enable VSS = yes
  Include { Options { Signature = XXH128; FS Type = ext4 } File = /usr/sbin }
  Exclude { File = /tmp }
}
"#).unwrap();
        let items = fileset_items(&cfg, "fs").unwrap();
        assert!(items.enable_vss);
        assert_eq!(items.includes[0].options[0].opts, "S4HAX");
        assert_eq!(items.includes[0].options[0].fstype, vec!["ext4"]);
        assert_eq!(items.excludes[0].options[0].opts, "HAX");
        assert_eq!(items.excludes[0].files, vec!["/tmp"]);
    }
}
