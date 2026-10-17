//! restore: port of C++ ua_restore.cc RestoreCmd and UserSelectJobidsOrFiles
//! (compatibility log C-021, C-047).
//!
//! Upstream order: comment, fileregex, regexwhere/where ACL, OpenClientDb,
//! the Restore Job resources, the JobId selection (keywords or the menu
//! dialog, C-011), BuildDirectoryTree with the file selection tree
//! (restore_tree.rs), the bootstrap (restore_bsr.rs) and finally the run
//! command upstream builds ("run job=... bootstrap=... files=N ...") with its
//! "OK to run?" dialog (commands/job.rs). Nothing is started without the
//! run confirmation (or `yes`).
//!
//! Like the C++ private catalog connection (OpenClientDb(ua, true)), the
//! command uses one dedicated catalog connection: the temporary tables of
//! SelectBackupsBeforeDate and the file list cursor of BuildDirectoryTree
//! live on it.

use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicU32, Ordering};

use bareos_config::resources::JobType;

use crate::acl::{self, Kind};
use crate::catalog_worker::{CatalogConnection, CatalogHandle, SqlTable};
use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::commands::bvfs::Db;
use crate::commands::restore_bsr::Bsr;
use crate::commands::restore_tree::{user_select_files_from_tree, Tree};
use crate::job_worker::{JobParams, WorkerJobType};
use crate::sql_list::{add_commas, ListType};
use crate::ua_select::Ua;

/// UserSelectJobidsOrFiles kw[] (ua_restore.cc:511), in upstream order.
const KEYWORDS: [&str; 27] = [
    "jobid", "current", "before", "file", "directory", "select", "pool", "all", "fileregex",
    // handled by individual arg lookups
    "client", "storage", "fileset", "where", "yes", "bootstrap", "done", "strip_prefix",
    "add_prefix", "add_suffix", "regexwhere", "restoreclient", "copies", "comment",
    "restorejob", "replace", "pluginoptions", "archive",
];

/// JobId selection menu (ua_restore.cc:495).
const SELECT_LIST: [&str; 13] = [
    "List last 20 Jobs run",
    "List Jobs where a given File is saved",
    "Enter list of comma separated JobIds to select",
    "Enter SQL list command",
    "Select the most recent backup for a client",
    "Select backup for a client before a specified time",
    "Enter a list of files to restore",
    "Enter a list of files to restore before a specified time",
    "Find the JobIds of the most recent backup for a client",
    "Find the JobIds for a backup for a client before a specified time",
    "Enter a list of directories to restore for found JobIds",
    "Select full restore to a specified Job date",
    "Cancel",
];

const INTRO: &str = "\nFirst you select one or more JobIds that contain files\n\
    to be restored. You will be presented several methods\n\
    of specifying the JobIds. Then you will be allowed to\n\
    select which files from those JobIds are to be restored.\n\n";

/// include/baconfig.h MAX_NAME_LENGTH.
const MAX_NAME_LENGTH: usize = 128;

static NEXT_RESTORE_ID: AtomicU32 = AtomicU32::new(10000);

/// Everything the start of the restore needs once the run is confirmed.
pub(crate) struct Launch {
    pub restore_jobid: u32,
    pub client: String,
    pub restore_client: String,
    pub where_path: Option<String>,
    pub replace: Option<String>,
    /// The bootstrap file written for this restore.
    pub bootstrap_path: Option<String>,
    /// The Restore Job resource and the storage of the run (C-058).
    pub job: Option<String>,
    pub storage: Option<String>,
}

pub fn cmd_restore(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    restore_with(ast, ctx, |launch| start_restore(ctx, launch))
}

/// FindArgWithValue(): the first argument with that keyword; None when it
/// has no value.
fn arg_with_value<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref())
}

/// FindArg(): keyword present, with or without value.
fn has_arg(ast: &CommandAst, key: &str) -> bool {
    ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key))
}

/// The catalog connection of the command (C++ private UaContext db).
enum RestoreDb {
    Dedicated(CatalogConnection),
    Shared(CatalogHandle),
    None,
}

impl Db for RestoreDb {
    fn rows(&self, sql: &str) -> Result<SqlTable, String> {
        match self {
            RestoreDb::Dedicated(c) => c.query_rows(sql),
            RestoreDb::Shared(c) => c.query_rows(sql),
            RestoreDb::None => Err("No catalog\n".into()),
        }
    }
    fn exec(&self, sql: &str) -> Result<u64, String> {
        match self {
            RestoreDb::Dedicated(c) => c.execute(sql),
            RestoreDb::Shared(c) => c.query_rows(sql).map(|_| 0),
            RestoreDb::None => Err("No catalog\n".into()),
        }
    }
}

impl RestoreDb {
    fn open(ctx: &ExecContext) -> Self {
        match &ctx.catalog {
            Some(catalog) => match catalog.dedicated() {
                Ok(connection) => RestoreDb::Dedicated(connection),
                Err(_) => RestoreDb::Shared(catalog.clone()),
            },
            None => RestoreDb::None,
        }
    }

    /// BigSqlQuery(): the rows of `sql` through a cursor, a batch at a time,
    /// so a large file list never sits in memory as a whole.
    fn for_each_row(&self, sql: &str, mut f: impl FnMut(&[Option<String>])) -> Result<(), String> {
        let RestoreDb::Dedicated(connection) = self else {
            for row in self.rows(sql)?.rows {
                f(&row);
            }
            return Ok(());
        };
        connection.execute("BEGIN")?;
        let result = (|| {
            connection.execute(&format!("DECLARE _bac_cursor NO SCROLL CURSOR FOR {sql}"))?;
            loop {
                let batch = connection.query_rows("FETCH 1000 FROM _bac_cursor")?;
                if batch.rows.is_empty() {
                    break;
                }
                for row in &batch.rows {
                    f(row);
                }
            }
            connection.execute("CLOSE _bac_cursor")
        })();
        let end = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
        let _ = connection.execute(end);
        result.map(|_| ())
    }
}

/// RestoreContext (dird/ua.h).
struct Rx {
    db: RestoreDb,
    job_ids: String,
    base_job_ids: String,
    last_jobid: String,
    total_files: u64,
    all: bool,
    client_name: Option<String>,
    /// Pool resource of `pool=`.
    pool: Option<String>,
    bsr: Bsr,
    selected_files: u32,
    store: Option<String>,
    /// RestoreContext::FilterIdentifier(job_filter): 'B' or 'A'.
    filter: char,
    archive: bool,
}

/// RestoreCmd (ua_restore.cc:103). `start` runs a confirmed restore job.
pub(crate) fn restore_with(ast: &CommandAst, ctx: &ExecContext,
    launch: impl FnOnce(Launch) -> CommandResult) -> CommandResult {
    let mut ua = Ua::new(ctx);
    let mut rx = Rx { db: RestoreDb::None, job_ids: String::new(), base_job_ids: String::new(),
        last_jobid: String::new(), total_files: 0, all: false, client_name: None, pool: None,
        bsr: Bsr::default(), selected_files: 0, store: None, filter: 'B', archive: false };

    let comment = arg_with_value(ast, "comment");
    if let Some(comment) = comment {
        if let Err(message) = comment_legal(comment) {
            ua.error_msg(message);
            return ua.finish(false);
        }
    }
    let backup_format = arg_with_value(ast, "backupformat");
    let where_path = arg_with_value(ast, "where");
    let replace = arg_with_value(ast, "replace");
    let plugin_options = arg_with_value(ast, "pluginoptions");
    let strip_prefix = arg_with_value(ast, "strip_prefix");
    let add_prefix = arg_with_value(ast, "add_prefix");
    let add_suffix = arg_with_value(ast, "add_suffix");
    let mut regexwhere = arg_with_value(ast, "regexwhere").map(str::to_string);

    if let Some(regex) = arg_with_value(ast, "fileregex") {
        if !check_and_set_fileregex(&mut ua, &mut rx, regex) {
            ua.error_msg("Invalid \"FileRegex\" value.\n");
            return ua.finish(false);
        }
    }
    if has_arg(ast, "archive") {
        rx.archive = true;
        rx.filter = 'A';
    }

    if strip_prefix.is_some() || add_suffix.is_some() || add_prefix.is_some() {
        regexwhere = Some(build_where(strip_prefix, add_prefix, add_suffix));
    }

    if let Some(regexwhere) = &regexwhere {
        if !acl::access(ctx, Kind::Where, regexwhere) {
            ua.error_msg("\"RegexWhere\" specification not authorized.\n");
            return ua.finish(false);
        }
    }
    if let Some(where_path) = where_path {
        if !acl::access(ctx, Kind::Where, where_path) {
            ua.error_msg("\"where\" specification not authorized.\n");
            return ua.finish(false);
        }
    }

    if !ua.open_client_db(ast, true) {
        return ua.finish(false);
    }
    rx.db = RestoreDb::open(ctx);

    // Ensure there is at least one Restore Job
    let restore_jobs = restore_job_names(ctx);
    if restore_jobs.is_empty() {
        let path = crate::reload::config_path().map(|p| p.display().to_string()).unwrap_or_default();
        ua.error_msg(format!("No Restore Job Resource found in {path}.\n\
            You must create at least one before running this command.\n"));
        return ua.finish(false);
    }

    // Request user to select JobIds or files by various different methods.
    match user_select_jobids_or_files(&mut ua, ast, &mut rx) {
        0 => return ua.finish(false),
        1 => {
            get_and_display_basejobs(&mut ua, &mut rx);
            if !build_directory_tree(&mut ua, ast, &mut rx) {
                ua.send_msg("Restore not done.\n");
                return ua.finish(false);
            }
        }
        2 => {} // selected by filename, no tree needed
        _ => {
            // selected by fileregex only, add all findexes
            if !add_all_findex(&mut rx) {
                ua.error_msg("No JobId specified cannot continue.\n");
                ua.send_msg("Restore not done.\n");
                return ua.finish(false);
            }
        }
    }

    let job_name = if restore_jobs.len() == 1 {
        restore_jobs[0].clone()
    } else {
        match get_restore_job(&mut ua, ast, ctx, &restore_jobs) {
            Some(name) => name,
            None => return ua.finish(false),
        }
    };

    if rx.bsr.job_id() == 0 {
        ua.warning_msg("No files selected to be restored.\n");
        return ua.finish(false);
    }
    if !rx.bsr.add_volume_information(&mut ua) {
        ua.error_msg("Unable to construct a valid BootStrapRecord. Cannot continue.\n");
        return ua.finish(false);
    }
    let (count, bootstrap) = rx.bsr.write_file(&mut ua, ast, &rx.job_ids, &mut rx.store);
    rx.selected_files = count;
    if count == 0 {
        ua.warning_msg("No files selected to be restored.\n");
        return ua.finish(false);
    }
    rx.bsr.display_info(&mut ua, &rx.job_ids);
    if rx.selected_files == 1 {
        ua.info_msg("\n1 file selected to be restored.\n\n");
    } else {
        ua.info_msg(format!("\n{} files selected to be restored.\n\n", add_commas(&rx.selected_files.to_string())));
    }

    if !get_client_name(&mut ua, ast, &mut rx) {
        return ua.finish(false);
    }
    let Some(client) = rx.client_name.clone() else {
        ua.error_msg("No Client resource found!\n");
        return ua.finish(false);
    };
    // GetRestoreClientName (ua_restore.cc:452)
    let restore_client = match arg_with_value(ast, "restoreclient") {
        Some(name) => {
            let key = arg_key(ast, "restoreclient");
            if let Err(message) = name_valid(name) {
                ua.error_msg(format!("{key} argument: {message}"));
                return ua.finish(false);
            }
            if !client_resource_exists(ctx, name) {
                ua.error_msg(format!("invalid {key} argument: {name}\n"));
                return ua.finish(false);
            }
            name.to_string()
        }
        None => client.clone(),
    };

    // Build run command
    let catalog = crate::ua_db::selected_catalog(ctx).unwrap_or_default();
    let mut cmd = format!("run job=\"{job_name}\" client=\"{client}\" restoreclient=\"{restore_client}\" \
        storage=\"{}\" bootstrap=\"{}\" files={} catalog=\"{catalog}\"",
        rx.store.as_deref().unwrap_or(""), escape_filename(&bootstrap), rx.selected_files);
    if let Some(format) = backup_format {
        cmd.push_str(&format!(" backupformat={format}"));
    }
    if let Some(regexwhere) = &regexwhere {
        cmd.push_str(&format!(" regexwhere=\"{}\"", escape_filename(regexwhere)));
    } else if let Some(where_path) = where_path {
        cmd.push_str(&format!(" where=\"{}\"", escape_filename(where_path)));
    }
    if let Some(replace) = replace {
        cmd.push_str(&format!(" replace={replace}"));
    }
    if let Some(options) = plugin_options {
        cmd.push_str(&format!(" pluginoptions={options}"));
    }
    if let Some(comment) = comment {
        cmd.push_str(&format!(" comment=\"{comment}\""));
    }
    if has_arg(ast, "yes") {
        cmd.push_str(" yes"); // pass it on to the run command
    }
    let restore_jobid = jobid_list(&rx.job_ids).first().copied().unwrap_or(0);
    drop(rx);

    let mut run = crate::session::parse_cli_command(&cmd);
    run.raw_line = cmd;
    crate::commands::job::run_with_ua(&mut ua, &run, |confirmed, _, _| {
        // What the run dialog confirmed (possibly modified), else the
        // restore's own run command line.
        let value = |key: &str| arg_with_value(confirmed, key).or_else(|| arg_with_value(&run, key))
            .map(str::to_string);
        launch(Launch {
            restore_jobid,
            client: value("client").unwrap_or_default(),
            restore_client: value("restoreclient").unwrap_or_default(),
            where_path: value("where"),
            replace: value("replace"),
            bootstrap_path: value("bootstrap"),
            job: value("job"),
            storage: value("storage"),
        })
    });
    ua.finish(true)
}

/// The keyword as typed (C++ argk[i]) of the first argument `key`.
fn arg_key(ast: &CommandAst, key: &str) -> String {
    ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(key)).map(|a| a.key.clone())
        .unwrap_or_else(|| key.to_string())
}

/// escape_filename() (lib/bsys.cc): backslash before `"` and `\`.
fn escape_filename(path: &str) -> String {
    path.chars().fold(String::new(), |mut out, c| {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

/// get_restore_job (ua_select.cc:385)
fn get_restore_job(ua: &mut Ua, ast: &CommandAst, ctx: &ExecContext, restore_jobs: &[String]) -> Option<String> {
    let restorejob = arg_with_value(ast, "restorejob");
    if let Some(name) = restorejob {
        if restore_jobs.iter().any(|j| j == name) && acl::access(ctx, Kind::Job, name) {
            return Some(name.to_string());
        }
        ua.error_msg(format!("Error: Restore Job resource \"{name}\" does not exist.\n"));
    }
    crate::ua_select::select_restore_job_resource(ua)
}

/// CheckAndSetFileregex (ua_restore.cc:1141)
fn check_and_set_fileregex(ua: &mut Ua, rx: &mut Rx, regex: &str) -> bool {
    match regex_check(regex) {
        Ok(()) => {
            rx.bsr.set_fileregex(regex);
            true
        }
        Err(message) => {
            ua.send_msg(format!("Regex compile error: {message}\n"));
            false
        }
    }
}

/// GetAndDisplayBasejobs (ua_restore.cc:358)
fn get_and_display_basejobs(ua: &mut Ua, rx: &mut Rx) {
    let sql = format!("SELECT DISTINCT BaseJobId   FROM Job JOIN BaseFiles USING (JobId)  \
        WHERE Job.HasBase = 1    AND Job.JobId IN ({}) ", rx.job_ids);
    let jobids = match rx.db.rows(&sql) {
        Ok(table) => table.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>(),
        Err(e) => {
            ua.warning_msg(e);
            Vec::new()
        }
    };
    let jobids = jobids.join(",");
    if !jobids.is_empty() {
        ua.send_msg("The restore will use the following job(s) as Base\n");
        list_query(ua, rx, &format!("SELECT DISTINCT JobId, Level, JobFiles, JobBytes, StartTime, VolumeName\n  \
            FROM Job\n  JOIN JobMedia\n USING (JobId)\n  JOIN Media\n USING (MediaId)\n WHERE JobId IN ({jobids})\n \
            ORDER BY StartTime ASC\n"), false);
    }
    rx.base_job_ids = jobids;
}

/// ListSqlQuery(..., HORZ_LIST, verbose=true), with `jcr->gui` set for the
/// duration when `gui`. None of these lists reads the temporary tables, so
/// they run on the shared catalog connection, whose errors carry the libpq
/// PQerrorMessage text.
fn list_query(ua: &mut Ua, rx: &Rx, sql: &str, gui: bool) {
    let result = match &ua.ctx.catalog {
        Some(catalog) => catalog.query_rows(sql),
        None => rx.db.rows(sql),
    };
    match result {
        Ok(table) => {
            let text = crate::sql_list::with_gui(gui, || crate::sql_list::render(&table, ListType::Horz));
            ua.send_msg(text);
        }
        Err(e) => {
            if crate::sql_list::decorations() {
                ua.send_msg(format!("Query failed: {e}\n"));
            }
        }
    }
}

/// HasValue (ua_restore.cc:411).
fn has_value<'a>(ua: &mut Ua, key: &str, value: Option<&'a str>) -> Option<&'a str> {
    if value.is_none() {
        ua.error_msg(format!("Missing value for keyword: {key}\n"));
    }
    value
}

/// bstrutime(): local time "YYYY-MM-DD HH:MM:SS".
fn bstrutime(time: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(time, 0).single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default()
}

/// UserSelectJobidsOrFiles (ua_restore.cc:485): 0 error, 1 JobIds,
/// 2 file list made, 3 fileregex only.
fn user_select_jobids_or_files(ua: &mut Ua, ast: &CommandAst, rx: &mut Rx) -> i32 {
    let ctx = ua.ctx;
    // Include current second if using current time
    let now = chrono::Local::now().timestamp() + 1;
    let mut date = String::new();
    let mut have_date = false;
    let mut done = false;
    let mut files: Vec<String> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let mut use_select = false;
    let mut use_fileregex = false;
    rx.job_ids.clear();

    for arg in &ast.arguments {
        let Some(keyword) = KEYWORDS.iter().position(|k| k.eq_ignore_ascii_case(&arg.key)) else {
            ua.error_msg(format!("Unknown keyword: {}\n", arg.key));
            return 0;
        };
        let value = arg.value.as_deref();
        match keyword {
            0 => {
                // jobid
                let Some(value) = has_value(ua, &arg.key, value) else { return 0 };
                if !rx.job_ids.is_empty() {
                    rx.job_ids.push(',');
                }
                rx.job_ids.push_str(value);
                rx.last_jobid = value.chars().take(49).collect();
                done = true;
            }
            1 => {
                // current
                date = bstrutime(now);
                have_date = true;
            }
            2 => {
                // before: a second date ends the selection without a message
                if have_date {
                    return 0;
                }
                let Some(value) = has_value(ua, &arg.key, value) else { return 0 };
                let cpdate = compensate_short_date(value);
                if str_to_utime(&cpdate) == 0 {
                    ua.error_msg(format!("Improper date format: {cpdate}\n"));
                    return 0;
                }
                date = cpdate;
                have_date = true;
            }
            3 => {
                let Some(value) = has_value(ua, &arg.key, value) else { return 0 };
                files.push(value.to_string());
            }
            4 => {
                let Some(value) = has_value(ua, &arg.key, value) else { return 0 };
                dirs.push(value.to_string());
            }
            5 => use_select = true,
            6 => {
                // pool: GetPoolResWithName (with Pool ACL)
                let Some(value) = has_value(ua, &arg.key, value) else { return 0 };
                let exists = ctx.config.as_ref()
                    .and_then(|c| c.read().ok().map(|c| c.find_pool(value).is_some()))
                    .unwrap_or(false);
                if !exists || !acl::access(ctx, Kind::Pool, value) {
                    ua.error_msg(format!("Error: Pool resource \"{value}\" does not exist.\n"));
                    return 0;
                }
                rx.pool = Some(value.to_string());
            }
            7 => rx.all = true,
            8 => use_fileregex = true,
            _ => {} // handled elsewhere
        }
    }

    if !files.is_empty() || !dirs.is_empty() || use_fileregex {
        if !have_date {
            date = bstrutime(now);
        }
        if !get_client_name(ua, ast, rx) {
            return 0;
        }
        for file in &files {
            insert_one_file_or_dir(ua, rx, file, &date, false);
        }
        for dir in &dirs {
            insert_one_file_or_dir(ua, rx, dir, &date, true);
        }
        if files.is_empty() && dirs.is_empty() {
            // Only fileregex: select the jobids if none were specified.
            if rx.job_ids.is_empty() && !select_backups_before_date(ua, ast, rx, &date) {
                return 0;
            }
            return 3;
        }
        return 2;
    }

    if use_select {
        if !have_date {
            date = bstrutime(now);
        }
        if !select_backups_before_date(ua, ast, rx, &date) {
            return 0;
        }
        done = true;
    }

    if !done {
        ua.send_msg(INTRO);
    }

    let filter = rx.filter;
    while !done {
        ua.start_prompt("To select the JobIds, you have the following choices:\n");
        for item in SELECT_LIST {
            ua.add_prompt(item);
        }
        done = true;
        let Some((item, _)) = ua.do_prompt("", "Select item: ") else { return 0 };
        match item {
            0 => {
                // list last 20 Jobs run
                let sql = format!("SELECT JobId, Client.Name as Client, StartTime, Level as JobLevel, JobFiles, JobBytes\n  \
                    FROM Client, Job\n WHERE Client.ClientId = Job.ClientId\n   AND JobStatus IN ('T','W')\n   \
                    AND Type            = '{filter}'\n ORDER BY StartTime DESC LIMIT 20\n");
                if !acl::access(ctx, Kind::Command, "sqlquery") {
                    ua.error_msg("SQL query not authorized.\n");
                    return 0;
                }
                list_query(ua, rx, &sql, true);
                done = false;
            }
            1 => {
                // list where a file is saved
                if !get_client_name(ua, ast, rx) {
                    return 0;
                }
                let Some(fname) = ua.get_cmd("Enter Filename (no path):") else { return 0 };
                let client = rx.client_name.clone().unwrap_or_default();
                let sql = format!("SELECT Job.JobId AS JobId,\n       Path.Path||File.Name AS Name,\n       StartTime,\n       \
                    TYPE AS JobType,\n       JobStatus,\n       JobFiles,\n       JobBytes\nFROM Client,\n     Job,\n     \
                    File,\n     Path\nWHERE Client.Name='{}'\n  AND Client.ClientId=Job.ClientId\n  \
                    AND Job.JobId=File.JobId\n  AND File.FileIndex > 0\n  AND Path.PathId=File.PathId\n  \
                    AND File.Name='{}'\nORDER BY StartTime DESC\nLIMIT 20\n", client, escape(&fname));
                list_query(ua, rx, &sql, true);
                done = false;
            }
            2 => {
                // enter a list of JobIds
                let Some(answer) = ua.get_cmd("Enter JobId(s), comma separated, to restore: ") else { return 0 };
                rx.job_ids = answer;
            }
            3 => {
                // Enter an SQL list command
                if !acl::access(ctx, Kind::Command, "sqlquery") {
                    ua.error_msg("SQL query not authorized.\n");
                    return 0;
                }
                let Some(sql) = ua.get_cmd("Enter SQL list command: ") else { return 0 };
                list_query(ua, rx, &sql, true);
                done = false;
            }
            4 | 8 => {
                // Select the most recent backups / find the JobIds of them
                if !have_date {
                    date = bstrutime(now);
                }
                if !select_backups_before_date(ua, ast, rx, &date) {
                    return 0;
                }
                done = item == 4;
            }
            5 | 9 => {
                // select backup at specified time / find the JobIds
                if !have_date {
                    match get_date(ua) {
                        Some(d) => date = d,
                        None => return 0,
                    }
                }
                if !select_backups_before_date(ua, ast, rx, &date) {
                    return 0;
                }
                done = item == 5;
            }
            6 | 7 => {
                // Enter files (before a specified time)
                if !have_date {
                    if item == 6 {
                        date = bstrutime(now);
                    } else {
                        match get_date(ua) {
                            Some(d) => date = d,
                            None => return 0,
                        }
                    }
                }
                if !get_client_name(ua, ast, rx) {
                    return 0;
                }
                ua.send_msg("Enter file names with paths, or < to enter a filename\n\
                    containing a list of file names with paths, and Terminate\n\
                    them with a blank line.\n");
                loop {
                    let Some(file) = ua.get_cmd("Enter full filename: ") else { return 0 };
                    if file.is_empty() {
                        break;
                    }
                    insert_one_file_or_dir(ua, rx, &file, &date, false);
                }
                return 2;
            }
            10 => {
                // Enter directories
                if !rx.job_ids.is_empty() {
                    ua.send_msg(format!("You have already selected the following JobIds: {}\n", rx.job_ids));
                } else if let Some(answer) = ua.get_cmd("Enter JobId(s), comma separated, to restore: ") {
                    if !rx.job_ids.is_empty() && !answer.is_empty() {
                        rx.job_ids.push(',');
                    }
                    rx.last_jobid = answer.chars().take(49).collect();
                    rx.job_ids.push_str(&answer);
                }
                if rx.job_ids.is_empty() || rx.job_ids.starts_with('.') {
                    rx.job_ids.clear();
                    return 0; // nothing entered, return
                }
                if !have_date {
                    date = bstrutime(now);
                }
                if !get_client_name(ua, ast, rx) {
                    return 0;
                }
                ua.send_msg("Enter full directory names or start the name\n\
                    with a < to indicate it is a filename containing a list\n\
                    of directories and Terminate them with a blank line.\n");
                loop {
                    let Some(mut dir) = ua.get_cmd("Enter directory name: ") else { return 0 };
                    if dir.is_empty() {
                        break;
                    }
                    // Add trailing slash to end of directory names
                    if !dir.starts_with('<') && !dir.ends_with('/') {
                        dir.push('/');
                    }
                    insert_one_file_or_dir(ua, rx, &dir, &date, true);
                }
                return 2;
            }
            11 => {
                // Choose a jobid and select jobs
                let answer = ua.get_cmd("Enter JobId to get the state to restore: ");
                let Some(answer) = answer.filter(|a| is_an_integer(a)) else { return 0 };
                let jobid = str_to_int64(&answer) as u32;
                let record = match job_record(rx, jobid) {
                    Ok(record) => record,
                    Err(e) => {
                        ua.error_msg(format!("Unable to get Job record for JobId={answer}: ERR={e}\n"));
                        return 0;
                    }
                };
                ua.send_msg(format!("Selecting jobs to build the Full state at {}\n", record.start_time));
                let job_type = if record.job_type == "A" { 'A' } else { 'B' };
                let date = crate::commands::bvfs::accurate_date(&record.start_time);
                let Some(jobids) = crate::commands::bvfs::accurate_get_jobids(&rx.db, record.client_id,
                    job_type, &date, record.fileset_id) else { return 0 };
                rx.job_ids = jobids.join(",");
            }
            _ => return 0, // Cancel or quit
        }
    }

    // Find total number of files to be restored, and filter the JobId list
    // to contain only ones permitted by the ACL conditions.
    let mut selected: Vec<u32> = Vec::new();
    let mut last: u32 = 0;
    let list = rx.job_ids.clone();
    let mut rest = list.as_str();
    rx.total_files = 0;
    loop {
        let jobid = match next_jobid(&mut rest) {
            Err(()) => {
                ua.error_msg("Invalid JobId in list.\n");
                return 0;
            }
            Ok(None) => break,
            Ok(Some(jobid)) => jobid,
        };
        if jobid == last {
            continue; // duplicate of last JobId
        }
        last = jobid;
        let record = match job_record(rx, jobid) {
            Ok(record) => record,
            Err(message) => {
                ua.error_msg(format!("Unable to get Job record for JobId={jobid}: ERR={message}\n"));
                return 0;
            }
        };
        if !acl::access(ctx, Kind::Job, &record.name) {
            ua.error_msg(format!("Access to JobId={jobid} (Job \"{}\") not authorized. Not selected.\n", record.name));
            continue;
        }
        selected.push(jobid);
        rx.total_files += record.job_files;
    }
    rx.job_ids = selected.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    if rx.job_ids.is_empty() {
        ua.warning_msg("No Jobs selected.\n");
        return 0;
    }
    let plural = if rx.job_ids.contains(',') { "s" } else { "" };
    ua.info_msg(format!("You have selected the following JobId{plural}: {}\n", rx.job_ids));
    1
}

/// get_date (ua_restore.cc:887)
fn get_date(ua: &mut Ua) -> Option<String> {
    ua.send_msg("The restored files will the most current backup\nBEFORE the date you specify below.\n\n");
    loop {
        let answer = ua.get_cmd("Enter date as YYYY-MM-DD HH:MM:SS :")?;
        let date = compensate_short_date(&answer);
        if str_to_utime(&date) != 0 {
            return Some(date);
        }
        ua.error_msg("Improper date format.\n");
    }
}

/// The JobDbRecord fields the restore needs.
struct JobRecord {
    name: String,
    job_files: u64,
    start_time: String,
    job_type: String,
    client_id: i64,
    fileset_id: i64,
}

/// BareosDb::GetJobRecord() by JobId; the error text carries the source
/// position as the reference binary reports it.
fn job_record(rx: &Rx, jobid: u32) -> Result<JobRecord, String> {
    let table = rx.db.rows(&format!(
        "SELECT Name,JobFiles,StartTime,Type,ClientId,FileSetId FROM Job WHERE JobId={jobid}"))
        .map_err(|e| format!("{e}\n"))?;
    let Some(row) = table.rows.into_iter().next() else {
        return Err(format!("cats/sql_get.cc:275 No Job found for JobId {jobid}\n"));
    };
    let value = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    Ok(JobRecord {
        name: value(0),
        job_files: str_to_int64(&value(1)) as u64,
        start_time: value(2),
        job_type: value(3),
        client_id: str_to_int64(&value(4)),
        fileset_id: str_to_int64(&value(5)),
    })
}

/// SQL string literal content (PQescapeStringConn).
fn escape(text: &str) -> String {
    text.replace('\'', "''")
}

/// StripTrailingJunk(): trailing newlines, carriage returns and blanks.
fn strip_trailing_junk(text: &str) -> &str {
    text.trim_end_matches(['\n', '\r', ' '])
}

/// InsertOneFileOrDir (ua_restore.cc:955)
fn insert_one_file_or_dir(ua: &mut Ua, rx: &mut Rx, p: &str, date: &str, dir: bool) {
    if let Some(name) = p.strip_prefix('<') {
        let content = match std::fs::read(name) {
            Ok(content) => content,
            Err(e) => {
                ua.error_msg(format!("Cannot open file {name}: ERR={}\n", errno_text(&e)));
                return;
            }
        };
        let text = String::from_utf8_lossy(&content);
        for (index, line) in text.split_inclusive('\n').enumerate() {
            let ok = if dir {
                insert_dir_into_findex_list(ua, rx, line)
            } else {
                insert_file_into_findex_list(ua, rx, line, date)
            };
            if !ok {
                ua.error_msg(format!("Error occurred on line {} of file \"{name}\"\n", index + 1));
            }
        }
    } else if let Some(table) = p.strip_prefix('?') {
        let table = strip_trailing_junk(table);
        let sql = format!("SELECT JobId, FileIndex\n  FROM {table}\n ORDER BY JobId, FileIndex ASC\n");
        if !jobid_fileindex(ua, rx, &sql) {
            ua.error_msg(format!("No table found: {table}\n"));
        }
    } else if dir {
        insert_dir_into_findex_list(ua, rx, p);
    } else {
        insert_file_into_findex_list(ua, rx, p, date);
    }
}

/// The C++ BErrNo text of an I/O error (strerror).
pub(crate) fn errno_text(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        // SAFETY: strerror returns a valid static string.
        Some(code) => unsafe { CStr::from_ptr(libc::strerror(code)).to_string_lossy().into_owned() },
        None => e.to_string(),
    }
}

/// Runs a query with JobidFileindexHandler: every row adds its FileIndex
/// to the bootstrap and its JobId to the list. Returns whether a row was
/// found.
fn jobid_fileindex(ua: &mut Ua, rx: &mut Rx, sql: &str) -> bool {
    let table = match rx.db.rows(sql) {
        Ok(table) => table,
        Err(e) => {
            ua.error_msg(format!("Query failed: {sql}. ERR={e}"));
            return false;
        }
    };
    let found = !table.rows.is_empty();
    for row in table.rows {
        let value = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        let jobid = value(0);
        rx.bsr.add_findex(str_to_int64(&jobid) as u32, str_to_int64(&value(1)) as i32);
        rx.selected_files += 1;
        // JobidHandler
        if rx.last_jobid != jobid {
            rx.last_jobid = jobid.chars().take(49).collect();
            if !rx.job_ids.is_empty() {
                rx.job_ids.push(',');
            }
            rx.job_ids.push_str(&jobid);
        }
    }
    found
}

/// InsertFileIntoFindexList (ua_restore.cc:1006)
fn insert_file_into_findex_list(ua: &mut Ua, rx: &mut Rx, file: &str, date: &str) -> bool {
    let file = file.trim_end_matches(['\n', '\r']);
    // SplitPathAndFilename(): everything after the last / is the file name
    let split = file.rfind('/').map_or(0, |p| p + 1);
    let (path, fname) = (escape(&file[..split]), escape(&file[split..]));
    let client = rx.client_name.clone().unwrap_or_default();
    let filter = rx.filter;
    let sql = if rx.job_ids.is_empty() {
        format!("SELECT Job.JobId,\n       File.FileIndex\nFROM Job,\n     File,\n     Path,\n     Client\n\
            WHERE Job.JobId=File.JobId\n  AND Job.StartTime<='{date}'\n  AND Path.Path='{path}'\n  \
            AND File.Name='{fname}'\n  AND Client.Name='{client}'\n  AND Job.ClientId=Client.ClientId\n  \
            AND Path.PathId=File.PathId\n  AND JobStatus IN ('T',\n                    'W')\n  \
            AND TYPE='{filter}'\nORDER BY Job.StartTime DESC\nLIMIT 1\n")
    } else {
        format!("SELECT Job.JobId,\n       File.FileIndex\nFROM Job,\n     File,\n     Path,\n     Client\n\
            WHERE Job.JobId IN ({})\n  AND Job.JobId=File.JobId\n  AND Job.StartTime<='{date}'\n  \
            AND Path.Path='{path}'\n  AND File.Name='{fname}'\n  AND Client.Name='{client}'\n  \
            AND Job.ClientId=Client.ClientId\n  AND Path.PathId=File.PathId\nORDER BY Job.StartTime DESC\nLIMIT 1\n",
            rx.job_ids)
    };
    if !jobid_fileindex(ua, rx, &sql) {
        ua.error_msg(format!("No database record found for: {file}\n"));
    }
    true
}

/// InsertDirIntoFindexList (ua_restore.cc:1044)
fn insert_dir_into_findex_list(ua: &mut Ua, rx: &mut Rx, dir: &str) -> bool {
    let dir = strip_trailing_junk(dir);
    if rx.job_ids.is_empty() {
        ua.error_msg("No JobId specified cannot continue.\n");
        return false;
    }
    let client = rx.client_name.clone().unwrap_or_default();
    let sql = format!("SELECT Job.JobId,\n       File.FileIndex\nFROM Job,\n     File,\n     Path,\n     Client\n\
        WHERE Job.JobId IN ({})\n  AND Job.JobId=File.JobId\n  AND Path.Path='{dir}'\n  AND Client.Name='{client}'\n  \
        AND Job.ClientId=Client.ClientId\n  AND Path.PathId=File.Pathid\n", rx.job_ids);
    if !jobid_fileindex(ua, rx, &sql) {
        ua.error_msg(format!("No database record found for: {dir}\n"));
    }
    true
}

/// AskForFileregex (ua_restore.cc:1165)
fn ask_for_fileregex(ua: &mut Ua, ast: &CommandAst, rx: &mut Rx) -> bool {
    // if user enters all on command line select everything
    if has_arg(ast, "all") || arg_with_value(ast, "fileregex").is_some() {
        return true;
    }
    ua.send_msg("\n\nFor one or more of the JobIds selected, no files were found,\n\
        so file selection is not possible.\nMost likely your retention policy pruned the files.\n");
    if let Some(yes) = ua.get_yesno("\nDo you want to restore all the files? (yes|no): ") {
        if yes {
            return true;
        }
        while let Some(answer) = ua.get_cmd("\nRegexp matching files to restore? (empty to abort): ") {
            if answer.is_empty() {
                break;
            } else if check_and_set_fileregex(ua, rx, &answer) {
                return true;
            }
        }
    }
    false
}

/// AddAllFindex (ua_restore.cc:1215)
fn add_all_findex(rx: &mut Rx) -> bool {
    let mut has_jobid = false;
    for jobid in jobid_list(&rx.job_ids) {
        // last_JobId is never updated upstream: only 0 is skipped
        if jobid == 0 {
            continue;
        }
        rx.bsr.add_findex_all(jobid);
        has_jobid = true;
    }
    has_jobid
}

/// GetNextJobidFromList over the whole list (stops at the first invalid
/// entry, like the `> 0` loops upstream).
pub(crate) fn jobid_list(list: &str) -> Vec<u32> {
    let mut rest = list;
    let mut jobids = Vec::new();
    while let Ok(Some(jobid)) = next_jobid(&mut rest) {
        jobids.push(jobid);
    }
    jobids
}

/// BuildDirectoryTree (ua_restore.cc:1229)
fn build_directory_tree(ua: &mut Ua, ast: &CommandAst, rx: &mut Rx) -> bool {
    let mut tree = Tree::new(rx.all);

    // Use first JobId as estimate of the number of files to restore
    if let Some(&first) = jobid_list(&rx.job_ids).first() {
        match rx.db.rows(&format!("SELECT JobFiles\n  FROM Job\n WHERE JobId = {first}\n")) {
            Ok(table) => {
                if let Some(row) = table.rows.first() {
                    let files = str_to_int64(row.first().cloned().flatten().as_deref().unwrap_or("")) as u32;
                    tree.delta_count = files / 50; // print 50 ticks
                }
            }
            Err(e) => ua.error_msg(format!("{e}\n")),
        }
    }

    ua.info_msg(format!("\nBuilding directory tree for JobId(s) {} ...  ", rx.job_ids));
    crate::audit::info(ua.ctx, &format!("Building directory tree for JobId(s) {}", rx.job_ids));

    // GetFileList(jobids, use_md5=false, use_delta=true)
    let jobids = &rx.job_ids;
    let sql = format!("SELECT Path.Path, T1.Name, T1.FileIndex, T1.JobId, LStat, DeltaSeq, Fhinfo, Fhnode \
        FROM ( SELECT DISTINCT ON (Name, PathId, DeltaSeq) JobTDate, JobId, FileId, FileIndex, PathId, \
        Filename AS Name, LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM (SELECT FileId, JobId, PathId, Name AS FileName, \
        FileIndex, LStat, MD5, DeltaSeq, Fhinfo, Fhnode FROM File WHERE JobId IN ({jobids}) UNION ALL \
        SELECT File.FileId, File.JobId, PathId, File.Name AS FileName, File.FileIndex, LStat, MD5, DeltaSeq, Fhinfo, \
        Fhnode FROM BaseFiles JOIN File USING (FileId) WHERE BaseFiles.JobId IN ({jobids}) ) AS T \
        JOIN Job USING (JobId) ORDER BY Name, PathId, DeltaSeq, JobTDate DESC ) AS T1 \
        JOIN Path ON (Path.PathId = T1.PathId) WHERE FileIndex > 0 ORDER BY T1.JobTDate, FileIndex ASC");
    let result = rx.db.for_each_row(&sql, |row| {
        let value = |i: usize| row.get(i).and_then(|v| v.as_deref()).unwrap_or("");
        tree.insert_row(ua, value(0), value(1), str_to_int64(value(2)) as i32, str_to_int64(value(3)) as u32,
            value(4), str_to_int64(value(5)) as i32);
    });
    if let Err(e) = result {
        ua.error_msg(e);
    }

    if !rx.base_job_ids.is_empty() {
        rx.job_ids.push(',');
        rx.job_ids.push_str(&rx.base_job_ids);
    }

    // A purged Job makes the tree incomplete: no manual selection.
    if tree.file_count != 0 {
        match rx.db.rows(&format!("SELECT SUM(PurgedFiles) FROM Job WHERE JobId IN ({})", rx.job_ids)) {
            Ok(table) => {
                if let Some(row) = table.rows.first() {
                    if str_to_int64(row.first().cloned().flatten().as_deref().unwrap_or("")) as u32 > 0 {
                        tree.file_count = 0;
                    }
                }
            }
            Err(e) => ua.error_msg(format!("{e}\n")),
        }
    }

    if tree.file_count == 0 {
        let ok = ask_for_fileregex(ua, ast, rx);
        if ok {
            add_all_findex(rx);
        }
        return ok;
    }
    if tree.all {
        ua.info_msg(format!("\n{} files inserted into the tree and marked for extraction.\n",
            add_commas(&tree.file_count.to_string())));
    } else {
        ua.info_msg(format!("\n{} files inserted into the tree.\n", add_commas(&tree.cnt.to_string())));
    }

    let mut ok = true;
    if !has_arg(ast, "done") {
        // Let the user interact in selecting which files to restore. The
        // tree has a UaContext of its own: under `.api 2` it is not in
        // batch mode and writes plain text.
        ok = if ua.ctx.api_mode == crate::command_executor::ApiMode::Json {
            let mut tree_ua = Ua::new_dialog(ua.ctx);
            user_select_files_from_tree(&mut tree_ua, &mut tree)
        } else {
            user_select_files_from_tree(ua, &mut tree)
        };
    }
    // Walk down through the tree finding all files marked to be extracted
    if ok {
        let (bsr, selected) = (&mut rx.bsr, &mut rx.selected_files);
        tree.for_each_extracted(|job_id, file_index, deltas, counted| {
            for &(delta_job, delta_index) in deltas {
                bsr.add_findex(delta_job, delta_index);
            }
            bsr.add_findex(job_id, file_index);
            if counted {
                *selected += 1;
            }
        });
    }
    ok
}

/// GetClientName (ua_restore.cc:421): `client=`/`backupclient=` or the
/// catalog selection (GetClientDbr).
fn get_client_name(ua: &mut Ua, ast: &CommandAst, rx: &mut Rx) -> bool {
    if rx.client_name.is_some() {
        return true;
    }
    let key = if arg_with_value(ast, "client").is_some() { "client" } else { "backupclient" };
    if let Some(name) = arg_with_value(ast, key) {
        let key = arg_key(ast, key);
        if let Err(message) = name_valid(name) {
            ua.error_msg(format!("{key} argument: {message}"));
            return false;
        }
        let sql = format!("SELECT ClientId,Name,Uname,AutoPrune,FileRetention,JobRetention FROM Client WHERE Client.Name='{}'",
            escape(name));
        if !rx.db.rows(&sql).is_ok_and(|t| t.rows.len() == 1) {
            ua.error_msg(format!("invalid {key} argument: {name}\n"));
            return false;
        }
        rx.client_name = Some(name.to_string());
        return true;
    }
    match get_client_dbr(ua, ast, rx) {
        Some((_, name)) => {
            rx.client_name = Some(name);
            true
        }
        None => false,
    }
}

/// GetClientDbr (ua_select.cc:572) without a preset name: `client=`/`fd=`
/// or SelectClientDbr. Returns (ClientId, Name).
fn get_client_dbr(ua: &mut Ua, ast: &CommandAst, rx: &Rx) -> Option<(i64, String)> {
    get_client_dbr_with(ua, ast, &rx.db)
}

/// GetClientDbr without a preset name on any catalog connection.
pub(crate) fn get_client_dbr_with(ua: &mut Ua, ast: &CommandAst, db: &dyn Db) -> Option<(i64, String)> {
    let ctx = ua.ctx;
    let record = |name: &str| db.rows(&format!(
        "SELECT ClientId,Name,Uname,AutoPrune,FileRetention,JobRetention FROM Client WHERE Client.Name='{}'",
        escape(name))).ok().filter(|t| t.rows.len() == 1)
        .and_then(|t| t.rows.into_iter().next())
        .map(|row| (str_to_int64(row[0].as_deref().unwrap_or("")), row[1].clone().unwrap_or_default()));
    for arg in &ast.arguments {
        let (true, Some(value)) = (arg.key.eq_ignore_ascii_case("client") || arg.key.eq_ignore_ascii_case("fd"),
            arg.value.as_deref()) else { continue };
        if !acl::access(ctx, Kind::Client, value) {
            break;
        }
        match record(value) {
            Some(found) => return Some(found),
            None => {
                ua.error_msg(format!("Could not find Client \"{value}\": ERR=Client record not found in Catalog.\n"));
                break;
            }
        }
    }
    // SelectClientDbr
    let names = match db.rows("SELECT Name FROM Client ORDER BY Name") {
        Ok(table) => table.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>(),
        Err(e) => {
            ua.error_msg(format!("Error obtaining client ids. ERR={e}\n"));
            return None;
        }
    };
    if names.is_empty() {
        ua.error_msg("No clients defined. You must run a job before using this command.\n");
        return None;
    }
    ua.start_prompt("Defined Clients:\n");
    for name in names.iter().filter(|name| acl::access(ctx, Kind::Client, name)) {
        ua.add_prompt(name);
    }
    let (_, name) = ua.do_prompt("Client", "Select the Client")?;
    match record(&name) {
        Some(found) => Some(found),
        None => {
            ua.error_msg(format!("Could not find Client \"{name}\": ERR=Client record not found in Catalog.\n"));
            None
        }
    }
}

/// SelectBackupsBeforeDate (ua_restore.cc:1385): the most recent Full,
/// Differential and Incrementals of a client and FileSet before `date`.
fn select_backups_before_date(ua: &mut Ua, ast: &CommandAst, rx: &mut Rx, date: &str) -> bool {
    let ok = select_backups(ua, ast, rx, date);
    let _ = rx.db.exec("DROP TABLE IF EXISTS DelCandidates\n");
    let _ = rx.db.exec("DROP TABLE IF EXISTS temp1\n");
    // The C++ private connection ends with the command and its temporary
    // tables with it; this pooled connection outlives the command.
    let _ = rx.db.exec("DROP TABLE IF EXISTS temp\n");
    ok
}

fn select_backups(ua: &mut Ua, ast: &CommandAst, rx: &mut Rx, date: &str) -> bool {
    let filter = rx.filter;
    // Create temp tables
    let _ = rx.db.exec("DROP TABLE IF EXISTS temp\n");
    let _ = rx.db.exec("DROP TABLE IF EXISTS temp1\n");
    if let Err(e) = rx.db.exec("CREATE TEMPORARY TABLE\n       temp (\n          JobId INTEGER NOT NULL, JobTDate BIGINT, \
        ClientId INTEGER, Level CHAR, JobFiles INTEGER, JobBytes BIGINT, StartTime\n          TEXT, VolumeName TEXT, \
        StartFile INTEGER, VolSessionId INTEGER, VolSessionTime INTEGER\n       )\n") {
        ua.error_msg(format!("{e}\n"));
    }
    if let Err(e) = rx.db.exec("CREATE TEMPORARY TABLE temp1 ( JobId INTEGER NOT NULL, JobTDate BIGINT)\n") {
        ua.error_msg(format!("{e}\n"));
    }
    // Select Client from the Catalog
    let Some((client_id, client_name)) = get_client_dbr(ua, ast, rx) else { return false };
    rx.client_name = Some(client_name.clone());

    // Get FileSet
    let mut fileset = String::new();
    let mut have_fileset = false;
    if let Some(name) = arg_with_value(ast, "FileSet") {
        match name_valid(name) {
            Ok(()) => match fileset_record(rx, name) {
                Ok(()) => {
                    fileset = name.to_string();
                    have_fileset = true;
                }
                Err(e) => ua.error_msg(format!("Error getting FileSet \"{name}\": ERR={e}\n")),
            },
            Err(message) => ua.error_msg(format!("FileSet argument: {message}\n")),
        }
    }
    if !have_fileset {
        ua.start_prompt("The defined FileSet resources are:\n");
        let sql = format!("SELECT DISTINCT FileSet.FileSet\n  FROM Job, Client, FileSet\n \
            WHERE Job.FileSetId   = FileSet.FileSetId\n   AND Job.ClientId    = {client_id}\n   \
            AND Client.ClientId = {client_id}\n ORDER BY FileSet.FileSet\n");
        match rx.db.rows(&sql) {
            Ok(table) => {
                for name in table.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()) {
                    ua.add_prompt(&name);
                }
            }
            Err(e) => ua.error_msg(format!("{e}\n")),
        }
        let Some((_, name)) = ua.do_prompt("FileSet", "Select FileSet resource") else {
            ua.error_msg(format!("No FileSet found for client \"{client_name}\".\n"));
            return false;
        };
        if let Err(e) = fileset_record(rx, &name) {
            ua.warning_msg(format!("Error getting FileSet record: {e}\n"));
            ua.send_msg("This probably means you modified the FileSet.\nContinuing anyway.\n");
        }
        fileset = name;
    }
    let fileset = escape_fileset(&fileset);

    // If Pool specified, add PoolId specification
    let mut pool_select = String::new();
    if let Some(pool) = rx.pool.clone() {
        match crate::ua_select::get_pool_record(ua.ctx, &pool) {
            Ok(pool_id) => pool_select = format!("AND Media.PoolId={pool_id} "),
            Err(_) => ua.warning_msg(format!("Pool \"{pool}\" not found, using any pool.\n")),
        }
    }

    if !insert_last_full(ua, rx, filter, client_id, date, &fileset, &pool_select) {
        return false;
    }
    let mut job_tdate = last_full(rx, "SELECT *\n  FROM temp1\n", ua);
    if job_tdate == 0 {
        let archive = if filter == 'B' { "" } else { " archive" };
        ua.error_msg(format!("No Full backup{archive} before {date} found.\n"));
        // Is there a full backup of the opposite type?
        let opposite = if filter == 'B' { 'A' } else { 'B' };
        if insert_last_full(ua, rx, opposite, client_id, date, &fileset, &pool_select) {
            match rx.db.rows("SELECT *\n  FROM temp1\n") {
                Ok(table) => {
                    let tdate = table.rows.last().and_then(|r| r.get(1).cloned().flatten())
                        .map_or(0, |v| str_to_int64(&v));
                    if tdate != 0 {
                        let (addition, command) = if opposite == 'B' { ("", "normal restore") }
                            else { (" archive", "restore archive") };
                        ua.send_msg(format!("A suitable full backup{addition} was found. Try {command} <...> instead.\n"));
                    }
                }
                Err(_) => return false,
            }
        }
        return false;
    }

    // Now find most recent Differential Job after Full save, if any
    let sql = format!("INSERT INTO temp\nSELECT Job.JobId, Job.JobTDate, Job.ClientId, Job.Level, Job.JobFiles, \
        Job.JobBytes, Job.StartTime, Media.VolumeName,\n       JobMedia.StartFile, Job.VolSessionId, Job.VolSessionTime\n  \
        FROM Job, JobMedia, Media, FileSet\n WHERE Job.JobTDate     > {job_tdate}\n   AND Job.StartTime    < '{date}'\n   \
        AND Job.ClientId     = {client_id}\n   AND JobMedia.JobId   = Job.JobId\n   AND Media.Enabled    = 1\n   \
        AND JobMedia.MediaId = Media.MediaId\n   AND Job.Level        = 'D'\n   AND JobStatus IN ('T','W')\n   \
        AND Type             = '{filter}'\n   AND Job.FileSetId    = FileSet.FileSetId\n   \
        AND FileSet.FileSet  = '{fileset}' {pool_select}\n ORDER BY Job.JobTDate DESC LIMIT 1\n");
    if let Err(e) = rx.db.exec(&sql) {
        ua.warning_msg(format!("{e}\n"));
    }
    // Now update JobTDate to look into Differential, if any
    let tdate = last_full(rx, "SELECT *\n  FROM temp\n", ua);
    if tdate != 0 {
        job_tdate = tdate;
    }
    // Now find all Incremental Jobs after Full/dif save
    let sql = format!("INSERT INTO temp\nSELECT Job.JobId, Job.JobTDate, Job.ClientId, Job.Level, Job.JobFiles, \
        Job.JobBytes, Job.StartTime, Media.VolumeName,\n       JobMedia.StartFile, Job.VolSessionId, Job.VolSessionTime\n  \
        FROM Job, JobMedia, Media, FileSet\n WHERE Job.JobTDate     > {job_tdate}\n   AND Job.StartTime    < '{date}'\n   \
        AND Job.ClientId     = {client_id}\n   AND Media.Enabled    = 1\n   AND JobMedia.JobId   = Job.JobId\n   \
        AND JobMedia.MediaId = Media.MediaId\n   AND Job.Level        = 'I'\n   AND JobStatus IN ('T','W')\n   \
        AND Type             = '{filter}'\n   AND Job.FileSetId    = FileSet.FileSetId\n   \
        AND FileSet.FileSet  = '{fileset}' {pool_select}\n");
    if let Err(e) = rx.db.exec(&sql) {
        ua.warning_msg(format!("{e}\n"));
    }

    // Get the JobIds from that list
    rx.last_jobid.clear();
    rx.job_ids.clear();
    jobid_handler(ua, rx, "SELECT DISTINCT JobId, StartTime\n  FROM temp\n ORDER BY StartTime ASC\n");

    if rx.job_ids.is_empty() {
        ua.warning_msg("No jobs found.\n");
        return false;
    }
    if has_arg(ast, "copies") {
        // Display a list of all copies (ListCopiesRecords)
        let sql = format!("SELECT DISTINCT Job.PriorJobId AS JobId, Job.Job, Job.JobId AS CopyJobId, Media.MediaType \
            FROM Job JOIN JobMedia USING (JobId) JOIN Media USING (MediaId) WHERE Job.Type = 'C'  \
            AND (Job.PriorJobId IN ({0}) OR Job.JobId IN ({0}))  ORDER BY Job.PriorJobId DESC  ", rx.job_ids);
        if let Ok(table) = rx.db.rows(&sql) {
            if !table.rows.is_empty() {
                if crate::sql_list::decorations() {
                    ua.send_msg("These JobIds have copies as follows:\n");
                }
                ua.send_msg(crate::sql_list::render(&table, ListType::Horz));
            }
        }
        let restore_copies = if has_arg(ast, "yes") {
            true
        } else {
            ua.get_yesno("\nDo you want to restore from these copies? (yes|no): ").unwrap_or(false)
        };
        if restore_copies {
            let jobids = std::mem::take(&mut rx.job_ids);
            rx.last_jobid.clear();
            jobid_handler(ua, rx, &format!("SELECT DISTINCT JobId, StartTime\n  FROM Job\n WHERE Type = 'C'\n   \
                AND (PriorJobId IN ({jobids}))\n ORDER BY StartTime ASC\n"));
        }
    }
    // Display a list of Jobs selected for this restore
    list_query(ua, rx, &format!("SELECT DISTINCT Job.JobId, Job.Level, Job.JobFiles, Job.JobBytes, Job.StartTime, \
        Media.VolumeName\n  FROM Job, JobMedia, Media\n WHERE Job.JobId IN ({})\n   AND JobMedia.JobId   = Job.JobId\n   \
        AND Media.Enabled    = 1\n   AND JobMedia.MediaId = Media.MediaId\n ORDER BY StartTime ASC\n", rx.job_ids), false);
    true
}

/// The FileSet name as it goes into the queries (EscapeString is not
/// applied upstream: the name passed IsNameValid or came from the catalog).
fn escape_fileset(name: &str) -> String {
    name.to_string()
}

/// BareosDb::GetFilesetRecord() by name.
fn fileset_record(rx: &Rx, name: &str) -> Result<(), String> {
    let table = rx.db.rows(&format!("SELECT FileSetId,FileSet,MD5,CreateTime FROM FileSet WHERE FileSet='{}' \
        ORDER BY CreateTime DESC LIMIT 1", escape(name))).map_err(|_| "FileSet record not found in Catalog.\n".to_string())?;
    if table.rows.is_empty() {
        return Err(format!("cats/sql_get.cc:877 FileSet record \"{}\" not found.\n", escape(name)));
    }
    Ok(())
}

/// InsertLastFullBackupOfType (ua_restore.cc:1340)
fn insert_last_full(ua: &mut Ua, rx: &Rx, filter: char, client_id: i64, date: &str, fileset: &str,
    pool_select: &str) -> bool {
    let sql = if !pool_select.is_empty() {
        format!("INSERT INTO temp1\nSELECT Job.JobId, JobTdate\n  FROM Job, JobMedia, Media, FileSet\n \
            WHERE Job.ClientId     = {client_id}\n   AND Job.StartTime    < '{date}'\n   AND Level            = 'F'\n   \
            AND JobStatus IN ('T','W')\n   AND Type             = '{filter}'\n   AND JobMedia.JobId   = Job.JobId\n   \
            AND Media.Enabled    = 1\n   AND JobMedia.MediaId = Media.MediaId\n   AND Job.FileSetId    = FileSet.FileSetId\n   \
            AND FileSet.FileSet  = '{fileset}' {pool_select}\n ORDER BY Job.JobTDate DESC LIMIT 1\n")
    } else {
        format!("INSERT INTO temp1\nSELECT Job.JobId, JobTdate\n  FROM Job, FileSet\n WHERE Job.ClientId    = {client_id}\n   \
            AND Job.StartTime   < '{date}'\n   AND Level           = 'F'\n   AND JobStatus IN ('T','W')\n   \
            AND Type            = '{filter}'\n   AND Job.FileSetId   = FileSet.FileSetId\n   \
            AND FileSet.FileSet = '{fileset}'\n ORDER BY Job.JobTDate DESC LIMIT 1\n")
    };
    if let Err(e) = rx.db.exec(&sql) {
        ua.error_msg(format!("{e}\n"));
        return false;
    }
    // Find all Volumes used by that JobId
    let sql = format!("INSERT INTO temp\nSELECT Job.JobId, Job.JobTDate, Job.ClientId, Job.Level, Job.JobFiles, \
        Job.JobBytes, StartTime, VolumeName, JobMedia.StartFile,\n       VolSessionId, VolSessionTime\n  \
        FROM temp1, Job, JobMedia, Media\n WHERE temp1.JobId      = Job.JobId\n   AND Level            = 'F'\n   \
        AND JobStatus IN ('T','W')\n   AND Type             = '{filter}'\n   AND Media.Enabled    = 1\n   \
        AND JobMedia.JobId   = Job.JobId\n   AND JobMedia.MediaId = Media.MediaId\n");
    if let Err(e) = rx.db.exec(&sql) {
        ua.error_msg(format!("{e}\n"));
        return false;
    }
    true
}

/// LastFullHandler over `sql`: JobTDate (row[1]) of the last row, 0 if none.
fn last_full(rx: &Rx, sql: &str, ua: &mut Ua) -> i64 {
    match rx.db.rows(sql) {
        Ok(table) => table.rows.last().and_then(|r| r.get(1).cloned().flatten()).map_or(0, |v| str_to_int64(&v)),
        Err(e) => {
            ua.warning_msg(format!("{e}\n"));
            0
        }
    }
}

/// JobidHandler over `sql`: appends new JobIds (row[0]) to the list.
fn jobid_handler(ua: &mut Ua, rx: &mut Rx, sql: &str) {
    match rx.db.rows(sql) {
        Ok(table) => {
            for row in table.rows {
                let jobid = row.into_iter().next().flatten().unwrap_or_default();
                if rx.last_jobid == jobid {
                    continue; // duplicate id
                }
                rx.last_jobid = jobid.chars().take(49).collect();
                if !rx.job_ids.is_empty() {
                    rx.job_ids.push(',');
                }
                rx.job_ids.push_str(&jobid);
            }
        }
        Err(e) => ua.warning_msg(format!("{e}\n")),
    }
}

/// IsAnInteger (lib/edit.cc): digits only.
fn is_an_integer(n: &str) -> bool {
    !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
}

fn restore_job_names(ctx: &ExecContext) -> Vec<String> {
    ctx.config.as_ref()
        .and_then(|c| c.read().ok().map(|c| c.jobs.iter()
            .filter(|j| j.job_type == JobType::Restore)
            .map(|j| j.name.clone())
            .collect()))
        .unwrap_or_default()
}

fn client_resource_exists(ctx: &ExecContext, name: &str) -> bool {
    let exists = ctx.config.as_ref()
        .and_then(|c| c.read().ok().map(|c| c.find_client(name).is_some()))
        .unwrap_or(false);
    exists && acl::access(ctx, Kind::Client, name)
}

/// GetNextJobidFromList (lib/jcr.cc:959): Ok(None) at the end, Err for a
/// token that is not a number. Tokens are at most 30 characters long.
fn next_jobid(rest: &mut &str) -> Result<Option<u32>, ()> {
    let bytes = rest.as_bytes();
    let mut token = String::new();
    let mut consumed = 0;
    for _ in 0..30 {
        match bytes.get(consumed) {
            None => break,
            Some(b',') => {
                consumed += 1;
                break;
            }
            Some(&b) => {
                token.push(b as char);
                consumed += 1;
            }
        }
    }
    if token.is_empty() {
        return Ok(None);
    }
    if !is_a_number(&token) {
        return Err(());
    }
    *rest = &rest[consumed..];
    Ok(Some(str_to_int64(&token) as u32))
}

/// Is_a_number (lib/edit.cc:496).
fn is_a_number(n: &str) -> bool {
    let b = n.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    let start = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let digit_seen = i > start;
    if digit_seen && b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if digit_seen && matches!(b.get(i), Some(b'e' | b'E'))
        && (b.get(i + 1).is_some_and(u8::is_ascii_digit)
            || matches!(b.get(i + 1), Some(b'-' | b'+')) && b.get(i + 2).is_some_and(u8::is_ascii_digit)) {
        i += 2;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    digit_seen && i == b.len()
}

/// str_to_int64 (lib/edit.cc): optional sign and leading digits.
fn str_to_int64(s: &str) -> i64 {
    let s = s.trim_start();
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let value = digits.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |acc, d| acc.wrapping_mul(10).wrapping_add(i64::from(d - b'0')));
    if negative { value.wrapping_neg() } else { value }
}

/// IsCommentLegal (ua_input.cc:227).
fn comment_legal(comment: &str) -> Result<(), &'static str> {
    if let Some(c) = comment.chars().find(|c| "'<>&\\\"".contains(*c)) {
        return Err(match c {
            '\'' => "Illegal character \"'\" in a comment.\n",
            '<' => "Illegal character \"<\" in a comment.\n",
            '>' => "Illegal character \">\" in a comment.\n",
            '&' => "Illegal character \"&\" in a comment.\n",
            '\\' => "Illegal character \"\\\" in a comment.\n",
            _ => "Illegal character \"\"\" in a comment.\n",
        });
    }
    if comment.len() >= MAX_NAME_LENGTH {
        return Err("Comment too long.\n");
    }
    if comment.is_empty() {
        return Err("Comment must be at least one character long.\n");
    }
    Ok(())
}

/// IsNameValid(name, msg) (lib/edit.cc:559).
fn name_valid(name: &str) -> Result<(), String> {
    if name.starts_with(' ') {
        return Err("Name cannot start with space.\n".into());
    }
    if let Some(c) = name.bytes().find(|b| !b.is_ascii_alphanumeric() && !b":.-_/ ".contains(b)) {
        return Err(format!("Illegal character \"{}\" in name.\n", c as char));
    }
    if name.len() >= MAX_NAME_LENGTH {
        return Err("Name too long.\n".into());
    }
    if name.is_empty() {
        return Err("Name must be at least one character long.\n".into());
    }
    if name.ends_with(' ') {
        return Err("Name cannot end with space.\n".into());
    }
    Ok(())
}

/// CheckAndSetFileregex (ua_restore.cc:1141): POSIX extended regex, the
/// error text is regerror()'s.
fn regex_check(regex: &str) -> Result<(), String> {
    let Ok(pattern) = CString::new(regex) else { return Err("embedded NUL".into()) };
    unsafe {
        let mut compiled = std::mem::MaybeUninit::<libc::regex_t>::zeroed();
        let rc = libc::regcomp(compiled.as_mut_ptr(), pattern.as_ptr(),
            libc::REG_EXTENDED | libc::REG_NOSUB);
        if rc != 0 {
            let mut buffer = [0 as libc::c_char; 500];
            libc::regerror(rc, compiled.as_ptr(), buffer.as_mut_ptr(), buffer.len());
            return Err(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
        }
        libc::regfree(compiled.as_mut_ptr());
    }
    Ok(())
}

/// bregexp_build_where (lib/breg.cc:356).
fn build_where(strip_prefix: Option<&str>, add_prefix: Option<&str>, add_suffix: Option<&str>) -> String {
    let escape = |s: &str| s.chars().fold(String::new(), |mut out, c| {
        if c == '!' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
        out
    });
    let mut parts = Vec::new();
    if let Some(strip) = strip_prefix {
        parts.push(format!("!{}!!i", escape(strip)));
    }
    if let Some(suffix) = add_suffix {
        parts.push(format!("!([^/])$!$1{}!", escape(suffix)));
    }
    if let Some(prefix) = add_prefix {
        parts.push(format!("!^!{}!", escape(prefix)));
    }
    parts.join(",")
}

/// CompensateShortDate (ua_restore.cc:913): `sscanf` is Bareos' bsscanf,
/// so a missing trailing field still counts (`%15s` converts nothing at the
/// end) and every attempt leaves its converted fields in the shared `tm`
/// for the next one. The first format whose count matches decides; its
/// trailing garbage must be empty. The fields are printed by strftime()
/// without normalisation. Verified against the installed libbareos bsscanf
/// (compatibility log C-059).
fn compensate_short_date(cmd: &str) -> String {
    const FORMATS: [(&str, i32); 6] = [
        ("%u-%u-%u %u:%u:%u%15s", 7),
        ("%u-%u-%u %u:%u%15s", 6),
        ("%u-%u-%u %u%15s", 5),
        ("%u-%u-%u%15s", 4),
        ("%u-%u%15s", 3),
        ("%u%s", 2),
    ];
    let mut fields = [0i32; 6];
    let mut garbage = Vec::new();
    let mut matched = false;
    for (format, expected) in FORMATS {
        let (count, values) = crate::scan::bsscanf(cmd.as_bytes(), format);
        for (i, value) in values.into_iter().enumerate() {
            match value {
                crate::scan::Scanned::U(v) => fields[i] = v as u32 as i32,
                crate::scan::Scanned::S(s) => garbage = s,
                crate::scan::Scanned::D(_) => {}
            }
        }
        if count == expected {
            matched = true;
            break;
        }
    }
    if !matched || !garbage.is_empty() {
        return cmd.to_string();
    }
    let [year, mut mon, mut mday, hour, min, sec] = fields;
    if mon == 0 {
        mon = 1;
    }
    if mday == 0 {
        mday = 1;
    }
    if year < 1900 {
        return cmd.to_string();
    }
    if mon > 0 {
        mon -= 1;
    }
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    (tm.tm_year, tm.tm_mon, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec) = (year - 1900, mon, mday, hour, min, sec);
    crate::btime::strftime_tm("%Y-%m-%d %H:%M:%S", &tm)
}

use crate::btime::str_to_utime;

/// Start a confirmed restore (the `restore` command and `run` of a Restore
/// job, job.cc RunJob): the Job record (Type 'R', Level 'F', the restore
/// client), the job queue and the reply "Job queued. JobId=N". The worker
/// reads the bootstrap file of the run (`bootstrap=`, else the Job's
/// Bootstrap directive, job.cc SetJcrDefaults); without one, or when it
/// does not open, the job fails like restore.cc DoNativeRestore.
/// Compatibility log C-058.
pub(crate) fn start_restore(ctx: &ExecContext, launch: Launch) -> CommandResult {
    let Launch { restore_jobid, client: _, restore_client, where_path, replace, bootstrap_path, job, storage } = launch;

    let config = match &ctx.config {
        Some(c) => match c.read() {
            Ok(c) => c,
            _ => return CommandResult::error("Config read error\n".to_string()),
        },
        None => return CommandResult::error("No config loaded\n".to_string()),
    };
    let job = match job.as_deref().and_then(|name| config.find_job(name))
        .or_else(|| config.jobs.iter().find(|j| j.job_type == JobType::Restore)) {
        Some(job) => job.clone(),
        None => return CommandResult::error("No Restore Job resource found!\n".to_string()),
    };
    let job_dirs = crate::commands::defaults::job_directives(&config, &job.name);
    let job_value = |key: &str| job_dirs.iter().rev().find(|d| d.norm_key() == key).and_then(|d| d.first())
        .map(str::to_string);
    let where_path = where_path.or_else(|| job_value("where")).unwrap_or_else(|| "/tmp/bareos-restores".to_string());
    let replace = replace.or_else(|| job_value("replace")).unwrap_or_else(|| "always".to_string()).to_ascii_lowercase();

    let client = match config.find_client(&restore_client) {
        Some(c) => c.clone(),
        None => return CommandResult::error(format!("Client '{restore_client}' not found\n")),
    };

    // jcr->RestoreBootstrap: the run's bootstrap=, else the Job's Bootstrap;
    // the job reads it (job_worker restore_bootstrap).
    let bootstrap = bootstrap_path.or_else(|| job_value("bootstrap"));

    // The read storage of the run, else the Job's, else the first one.
    let storage = match storage.as_deref().and_then(|s| config.find_storage(s))
        .or_else(|| config.find_storage(job.storage.split(',').next().unwrap_or("").trim()))
        .or_else(|| config.storages.first()) {
        Some(s) => s.clone(),
        None => return CommandResult::error("No storage configured\n".to_string()),
    };

    let dir_name = config.director.name.clone();
    drop(config); // Release RwLock before fork

    let now = now_unix();
    let unique_job = crate::commands::job::unique_job_name(&job.name, now);
    let job_id = if ctx.catalog.is_some() {
        match crate::commands::job::create_job_record(ctx, &unique_job, &job.name, 'R', 'F', Some(&client.name), now, "") {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("Failed to create job record: {e}");
                return CommandResult::error("Job failed.\n".to_string());
            }
        }
    } else {
        NEXT_RESTORE_ID.fetch_add(1, Ordering::Relaxed)
    };

    let params = JobParams {
        job_id,
        unique_job: unique_job.clone(),
        job_name: job.name.clone(),
        job_type: WorkerJobType::Restore,
        level: "full".to_string(),
        fd_address: client.address.clone(),
        fd_port: client.port,
        fd_password: client.password.clone(),
        sd_address: storage.address.clone(),
        sd_port: storage.port,
        sd_password: storage.password.clone(),
        sd_device: storage.device.clone(),
        sd_media_type: storage.media_type.clone(),
        director_name: dir_name,
        pool_name: job.pool.clone(),
        fileset_includes: vec![],
        fileset_excludes: vec![],
        since_utime: 0,
        prev_job: String::new(),
        where_path: Some(where_path),
        replace: Some(replace),
        restore_jobid: Some(restore_jobid),
        bootstrap,
        client_name: client.name.clone(),
        client_first_connection: crate::job_messages::first_client_connection(&client.name),
        copy: None,
    };
    let report = crate::job_messages::JobReport {
        kind: crate::job_messages::ReportKind::Restore, job: unique_job.clone(), client: client.name.clone(),
        sched_time: now, start_time: now,
        trigger: crate::job_messages::TRIGGER_USER.to_string(), ..Default::default()
    };

    crate::commands::job::queue_job(ctx, &job, params, report, &client.name, &storage.name, now)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default().as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_executor::ApiMode;
    use crate::ua_select::tests::scripted;
    use std::sync::{Arc, Mutex, RwLock};

    use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
    use crate::director_config::DirectorConfig;
    use crate::session_manager::SessionManager;

    const CONFIG: &str = r#"
Director { Name=dir; Password=secret }
Catalog { Name=MyCatalog; DbName=bareos }
Client { Name=fd1; Password=x }
Pool { Name=Full; Pool Type=Backup }
Storage { Name=File; Address=localhost; Password=x; Device=FileStorage; Media Type=File }
Job { Name=job1; Type=Backup; Client=fd1 }
Job { Name=RestoreFiles; Type=Restore; Client=fd1; Pool=Full; Storage=File }
Console { Name=limited; Password=pw; CommandACL=*all*; CatalogACL=*all*; ClientACL=*all*; PoolACL=*all*; JobACL=job1; WhereACL=/tmp/ok }
"#;

    /// One row per entry, the values separated by `|`.
    fn table(rows: &[&str]) -> SqlTable {
        let width = rows.first().map_or(1, |r| r.split('|').count());
        SqlTable {
            columns: (0..width).map(|i| SqlColumn { name: format!("c{i}"), numeric: false }).collect(),
            rows: rows.iter().map(|r| r.split('|').map(|v| Some(v.to_string())).collect()).collect(),
        }
    }

    const LSTAT: &str = "P0C BHt B A A A ByK BAA Q BqvQnm BqOal9 BqUv/U A A C";

    /// The catalog of the mock: client fd1, the jobs 1 (job1, two files in
    /// /usr/sbin/ on volume Vol1) and 2 (secret).
    fn answer(sql: &str) -> SqlTable {
        let file_list = format!("/usr/sbin/|zic|1|1|{LSTAT}|0|0|0");
        let file_list2 = format!("/usr/sbin/|ls|2|1|{LSTAT}|0|0|0");
        match sql {
            "SELECT ClientId,Name,Uname,AutoPrune,FileRetention,JobRetention FROM Client WHERE Client.Name='fd1'" =>
                table(&["1|fd1|u|1|0|0"]),
            "SELECT Name,JobFiles,StartTime,Type,ClientId,FileSetId FROM Job WHERE JobId=1" =>
                table(&["job1|2|2026-01-01 00:00:00|B|1|1"]),
            "SELECT Name,JobFiles,StartTime,Type,ClientId,FileSetId FROM Job WHERE JobId=2" =>
                table(&["secret|0|2026-01-01 00:00:00|B|1|1"]),
            "SELECT JobFiles\n  FROM Job\n WHERE JobId = 1\n" => table(&["2"]),
            "SELECT SUM(PurgedFiles) FROM Job WHERE JobId IN (1)" => table(&["0"]),
            "SELECT VolSessionId,VolSessionTime,JobFiles FROM Job WHERE JobId=1" => table(&["1|100|2"]),
            "SELECT Name from Storage WHERE StorageId=1" => table(&["File"]),
            "SELECT Name FROM Client ORDER BY Name" => table(&["fd1"]),
            sql if sql.starts_with("SELECT VolumeName,MediaType,FirstIndex,LastIndex") && sql.contains("JobId=1 ") =>
                table(&["Vol1|File|1|2|0|0|10|20|0|1|0|99"]),
            sql if sql.starts_with("SELECT Path.Path, T1.Name") && sql.contains("JobId IN (1)") =>
                table(&[&file_list, &file_list2]),
            _ => table(&[]),
        }
    }

    /// Catalog with client fd1 and the jobs 1 (job1) and 2 (secret); every
    /// request is logged.
    fn context_with(console: &str, config: &str) -> (ExecContext, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let catalog = CatalogHandle::mock(move |request| {
            let CatalogRequest::TextRows { sql } = request else {
                // no dedicated connection: the command uses the shared one
                return CatalogResponse::Error("mock".into());
            };
            seen.lock().unwrap().push(sql.clone());
            CatalogResponse::Rows(answer(&sql))
        });
        let manager = Arc::new(SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session(console);
        manager.authenticate_session(id);
        let ctx = ExecContext {
            session_id: id,
            config: Some(Arc::new(RwLock::new(DirectorConfig::from_str(config).unwrap()))),
            session_mgr: Some(manager),
            catalog: Some(catalog),
            api_mode: ApiMode::Human,
            bus_rx: None,
            job_manager: None,
        };
        (ctx, log)
    }

    fn context(console: &str) -> (ExecContext, Arc<Mutex<Vec<String>>>) {
        context_with(console, CONFIG)
    }

    /// Runs the command; a launch fails the test.
    fn run(ctx: &ExecContext, line: &str) -> CommandResult {
        restore_with(&crate::session::parse_cli_command(line), ctx,
            |_| panic!("restore started for incomplete input: {line}"))
    }

    const USING: &str = "Automatically selected Catalog: MyCatalog\nUsing Catalog \"MyCatalog\"\n";
    /// DoPrompt without console (batch mode) for the JobId menu.
    const MENU_BATCH: &str = "13: Cancel\nYour request has multiple choices for \"\". \
        Selection is not possible in batch mode.\n";

    /// A bootstrap file path of this test run.
    fn bootstrap_file(name: &str) -> String {
        std::env::temp_dir().join(format!("onesimus-test-{}-{name}.bsr", std::process::id())).display().to_string()
    }

    /// Runs the command with a console answering `answers`: the result and
    /// the transcript. A launch fails the test.
    fn dialog(ctx: &ExecContext, line: &str, answers: &[&str]) -> (CommandResult, String) {
        let (result, console) = scripted(answers, || run(ctx, line));
        (result, console.transcript())
    }

    #[test]
    fn unknown_keywords_are_rejected_after_opening_the_catalog() {
        let (ctx, _) = context("*UserAgent*");
        let result = run(&ctx, "restore help");
        assert!(!result.success);
        assert_eq!(result.text_output, USING);
        assert_eq!(result.error_message, "Unknown keyword: help\n");
        // backupformat and backupclient are read by RestoreCmd, but are not in kw[].
        assert_eq!(run(&ctx, "restore backupformat=x").error_message, "Unknown keyword: backupformat\n");
        assert_eq!(run(&ctx, "restore jobid=1 BackupClient=fd1").error_message, "Unknown keyword: BackupClient\n");
        // Keywords are case-insensitive.
        assert_eq!(run(&ctx, "restore JOBID=nope").error_message, "Invalid JobId in list.\n");
    }

    #[test]
    fn missing_values() {
        let (ctx, _) = context("*UserAgent*");
        for keyword in ["jobid", "before", "file", "directory", "pool"] {
            let result = run(&ctx, &format!("restore {keyword}"));
            assert!(!result.success);
            assert_eq!(result.error_message, format!("Missing value for keyword: {keyword}\n"));
        }
    }

    #[test]
    fn checks_before_the_catalog_is_opened() {
        let (ctx, log) = context("*UserAgent*");
        let result = run(&ctx, "restore comment=a<b help");
        assert_eq!((result.text_output.as_str(), result.error_message.as_str()),
            ("", "Illegal character \"<\" in a comment.\n"));
        assert_eq!(run(&ctx, "restore comment=").error_message, "Comment must be at least one character long.\n");
        assert_eq!(run(&ctx, &format!("restore comment={}", "x".repeat(128))).error_message, "Comment too long.\n");

        let result = run(&ctx, "restore fileregex=( help");
        assert!(!result.success);
        assert!(result.text_output.starts_with("Regex compile error: "), "{}", result.text_output);
        assert_eq!(result.error_message, "Invalid \"FileRegex\" value.\n");
        assert!(log.lock().unwrap().is_empty());
        // Nothing of this opened the catalog connection yet.
        assert!(run(&ctx, "restore help").text_output.contains("Using Catalog"));
    }

    #[test]
    fn where_acl() {
        let (ctx, log) = context("limited");
        let result = run(&ctx, "restore where=/etc help");
        assert_eq!((result.text_output.as_str(), result.error_message.as_str()),
            ("", "\"where\" specification not authorized.\n"));
        assert_eq!(run(&ctx, "restore regexwhere=!a!b!").error_message,
            "\"RegexWhere\" specification not authorized.\n");
        assert_eq!(run(&ctx, "restore add_prefix=/x where=/tmp/ok").error_message,
            "\"RegexWhere\" specification not authorized.\n");
        assert!(log.lock().unwrap().is_empty());
        assert_eq!(run(&ctx, "restore where=/tmp/ok help").error_message, "Unknown keyword: help\n");
    }

    #[test]
    fn dates() {
        let (ctx, _) = context("*UserAgent*");
        // bsscanf: a year alone is 2020-01-01 00:00:00 (C-059)
        assert!(run(&ctx, "restore before=2020").text_output.ends_with(MENU_BATCH));
        assert_eq!(run(&ctx, "restore before=2020x").error_message, "Improper date format: 2020x\n");
        assert_eq!(run(&ctx, "restore before=1899-01-01").error_message, "Improper date format: 1899-01-01\n");
        assert_eq!(run(&ctx, "restore before=\"2020-02-30 10:00:00\"").error_message,
            "Improper date format: 2020-02-30 10:00:00\n");
        // Valid dates lead to the selection menu.
        assert!(run(&ctx, "restore before=2020-01-01").text_output.ends_with(MENU_BATCH));
        // A second date ends the command without a message.
        let (ctx, _) = context("*UserAgent*");
        let result = run(&ctx, "restore current before=x");
        assert!(!result.success);
        assert_eq!((result.text_output.as_str(), result.error_message.as_str()), (USING, ""));
    }

    #[test]
    fn pool_must_exist_and_be_permitted() {
        let (ctx, _) = context("*UserAgent*");
        assert_eq!(run(&ctx, "restore pool=nosuch").error_message, "Error: Pool resource \"nosuch\" does not exist.\n");
        assert!(run(&ctx, "restore pool=Full").text_output.ends_with(MENU_BATCH));
    }

    #[test]
    fn jobid_list() {
        let (ctx, _) = context("*UserAgent*");
        assert_eq!(run(&ctx, "restore jobid=abc").error_message, "Invalid JobId in list.\n");
        assert_eq!(run(&ctx, "restore jobid=1,x").error_message, "Invalid JobId in list.\n");
        assert_eq!(run(&ctx, "restore jobid=99").error_message,
            "Unable to get Job record for JobId=99: ERR=cats/sql_get.cc:275 No Job found for JobId 99\n\n");
        // JobId 0 counts as duplicate of the empty record.
        let (ctx, _) = context("*UserAgent*");
        let result = run(&ctx, "restore jobid=0");
        assert_eq!((result.text_output.as_str(), result.success), (&*format!("{USING}No Jobs selected.\n"), false));
        let result = run(&ctx, "restore jobid=1 jobid=1,2");
        assert!(result.text_output.contains("You have selected the following JobIds: 1,2\n"));
    }

    #[test]
    fn job_acl_filters_jobids() {
        let (ctx, _) = context("limited");
        let result = run(&ctx, "restore jobid=2,1");
        assert!(result.text_output.contains("Access to JobId=2 (Job \"secret\") not authorized. Not selected.\n\
            You have selected the following JobId: 1\n"), "{}", result.text_output);
        let result = run(&ctx, "restore jobid=2");
        assert!(result.text_output.ends_with("No Jobs selected.\n"));
    }

    #[test]
    fn client_argument() {
        let (ctx, _) = context("*UserAgent*");
        assert_eq!(run(&ctx, "restore client=nosuch file=/x").error_message, "invalid client argument: nosuch\n");
        assert_eq!(run(&ctx, "restore client=a!b file=/x").error_message,
            "client argument: Illegal character \"!\" in name.\n");
        // FindArgWithValue: the first `client` decides.
        // GetClientDbr then takes any `client=`/`fd=` with a value.
        assert_eq!(run(&ctx, "restore client client=fd1 file=/x").error_message, "No database record found for: /x\n");
        // Without file selection the client is not looked at before the menu.
        assert!(run(&ctx, "restore client=nosuch").text_output.ends_with(MENU_BATCH));
        let bootstrap = bootstrap_file("client_argument");
        assert_eq!(run(&ctx, &format!("restore jobid=1 client=nosuch all done yes bootstrap={bootstrap}")).error_message,
            "invalid client argument: nosuch\n");
        assert_eq!(run(&ctx, &format!("restore jobid=1 client=fd1 restoreclient=nosuch all done yes bootstrap={bootstrap}"))
            .error_message, "invalid restoreclient argument: nosuch\n");
        let _ = std::fs::remove_file(&bootstrap);
    }

    #[test]
    fn selection_menu() {
        // Each run in a new session: "Using Catalog" is shown once per session.
        let (mut ctx, log) = context("*UserAgent*");
        let fresh = || context("*UserAgent*").0;
        let menu = format!("{USING}{INTRO}\
            To select the JobIds, you have the following choices:\n \
            1: List last 20 Jobs run\n \
            2: List Jobs where a given File is saved\n \
            3: Enter list of comma separated JobIds to select\n \
            4: Enter SQL list command\n \
            5: Select the most recent backup for a client\n \
            6: Select backup for a client before a specified time\n \
            7: Enter a list of files to restore\n \
            8: Enter a list of files to restore before a specified time\n \
            9: Find the JobIds of the most recent backup for a client\n\
            10: Find the JobIds for a backup for a client before a specified time\n\
            11: Enter a list of directories to restore for found JobIds\n\
            12: Select full restore to a specified Job date\n\
            13: Cancel\n\
            Select item:  (1-13): ");
        let (result, transcript) = dialog(&ctx, "restore", &["."]);
        assert!(!result.success);
        assert_eq!(transcript, format!("{menu}.\nSelection aborted, nothing done.\n"));
        let (_, transcript) = dialog(&fresh(), "restore", &["13"]);
        assert_eq!(transcript, format!("{menu}13\n"), "Cancel ends without a message");
        let (_, transcript) = dialog(&fresh(), "restore", &["3", "."]);
        assert_eq!(transcript, format!("{menu}3\nEnter JobId(s), comma separated, to restore: .\n"));
        // Entered JobIds are checked like jobid=, then the tree is built.
        let (_, transcript) = dialog(&fresh(), "restore", &["3", "1", "quit"]);
        assert!(transcript.contains("restore: 1\nYou have selected the following JobId: 1\n\n\
            Building directory tree for JobId(s) 1 ...  \n2 files inserted into the tree.\n"), "{transcript}");
        assert!(transcript.ends_with("$ quit\nRestore not done.\n"), "{transcript}");
        // List last 20 Jobs, then the menu again.
        let (_, transcript) = dialog(&fresh(), "restore", &["1", "13"]);
        assert!(transcript.contains("Select item:  (1-13): 1\nNo results to list.\nTo select the JobIds"), "{transcript}");
        assert!(log.lock().unwrap().iter().all(|sql| sql.starts_with("SELECT ")));

        let result = run(&fresh(), "restore");
        assert_eq!(result.text_output, format!("{}{MENU_BATCH}", menu.strip_suffix("13: Cancel\nSelect item:  (1-13): ").unwrap()));
        ctx.api_mode = ApiMode::Json;
        let result = run(&ctx, "restore");
        assert!(!result.success);
        assert!(result.text_output.ends_with(MENU_BATCH));
    }

    #[test]
    fn no_restore_job_resource() {
        let config = CONFIG.replace("Job { Name=RestoreFiles; Type=Restore; Client=fd1; Pool=Full; Storage=File }", "");
        let (ctx, _) = context_with("*UserAgent*", &config);
        let result = run(&ctx, "restore help");
        assert!(result.error_message.starts_with("No Restore Job Resource found in "));
        assert!(result.error_message.ends_with(".\nYou must create at least one before running this command.\n"));
    }

    #[test]
    fn nothing_starts_without_confirmation() {
        let (ctx, log) = context("*UserAgent*");
        let bootstrap = bootstrap_file("nothing_starts");
        for line in ["restore", "restore help", "restore client=fd1", "restore jobid=1", "restore jobid=1 all",
            "restore jobid=1 done", "restore jobid=1 all done", "restore jobid=1 client=fd1 all done",
            "restore jobid=1,2 client=fd1 strip_prefix=/a all done", "restore file=/etc/passwd client=fd1",
            "restore fileregex=x client=fd1 all done", "restore select all done", "restore current all done"] {
            run(&ctx, &format!("{line} bootstrap={bootstrap}"));
        }
        // The catalog was only read (temporary tables aside).
        assert!(log.lock().unwrap().iter().all(|sql| !sql.contains("INSERT INTO Job") && !sql.starts_with("UPDATE")
            && !sql.starts_with("DELETE")), "{:?}", log.lock().unwrap());

        // Without console the run confirmation fails: "Job not run."
        let result = run(&ctx, &format!("restore jobid=1 client=fd1 all done bootstrap={bootstrap}"));
        assert!(result.text_output.contains("Bootstrap records written to "), "{}", result.text_output);
        assert!(result.text_output.contains("\n2 files selected to be restored.\n\n"), "{}", result.text_output);
        assert!(std::fs::read_to_string(&bootstrap).unwrap().starts_with("Storage=\"File\"\nVolume=\"Vol1\"\n\
            MediaType=\"File\"\nDevice=\"FileStorage\"\nVolSessionId=1\nVolSessionTime=100\nVolAddr=10-20\nFileIndex=1-2\nCount=2\n"));

        // Control: `yes` reaches the launch with the selection.
        let mut started = None;
        let result = restore_with(&crate::session::parse_cli_command(
            &format!("restore jobid=1 client=fd1 where=/tmp/x all done yes bootstrap={bootstrap}")), &ctx, |launch| {
                started = Some((launch.restore_jobid, launch.client, launch.restore_client, launch.where_path,
                    launch.bootstrap_path));
                CommandResult::ok(String::new())
            });
        assert!(result.success);
        assert_eq!(started, Some((1, "fd1".into(), "fd1".into(), Some("/tmp/x".into()), Some(bootstrap.clone()))));
        let _ = std::fs::remove_file(&bootstrap);
    }

    /// `.api 2`: a complete selection starts the restore without a question
    /// (batch mode) and the result carries DoRunCmd's "run" object, also
    /// after the tree dialog on the console (C-072, S-09).
    #[test]
    fn api2_restore_result_has_the_jobid() {
        let (mut ctx, _) = context("*UserAgent*");
        ctx.api_mode = ApiMode::Json;
        let bootstrap = bootstrap_file("api2_jobid");
        let queued = || CommandResult::ok("Job queued. JobId=630\n".to_string())
            .with_json(serde_json::json!({"run": {"jobid": "630"}}));
        for (line, answers) in [
            (format!("restore jobid=1 client=fd1 all done bootstrap={bootstrap}"), vec![]),
            (format!("restore jobid=1 client=fd1 bootstrap={bootstrap}"), vec!["mark *", "done"]),
        ] {
            let mut started = false;
            let (result, transcript) = scripted(&answers, || restore_with(&crate::session::parse_cli_command(&line),
                &ctx, |_| { started = true; queued() }));
            let transcript = transcript.transcript();
            assert!(started, "{line}: {transcript}");
            assert!(result.success, "{line}");
            assert_eq!(result.json, Some(serde_json::json!({"run": {"jobid": "630"}})), "{line}: {transcript}");
            assert!(!transcript.contains("OK to run?"), "{line}: {transcript}");
        }
        let _ = std::fs::remove_file(&bootstrap);
    }

    /// The whole command under `.api 2` (start_restore, queue_job with a
    /// spawner that never forks): the json-rpc result is DoRunCmd's
    /// {"run":{"jobid":"N"}} (C-072, S-09).
    #[test]
    fn api2_restore_envelope_has_the_jobid() {
        let (mut ctx, _) = context("*UserAgent*");
        ctx.api_mode = ApiMode::Json;
        ctx.catalog = Some(CatalogHandle::mock(|request| match request {
            CatalogRequest::TextRows { sql } if sql.starts_with("SELECT ClientId FROM Client") =>
                CatalogResponse::Rows(table(&["1"])),
            CatalogRequest::TextRows { sql } if sql.starts_with("INSERT INTO Job ") =>
                CatalogResponse::Rows(table(&["630"])),
            CatalogRequest::TextRows { sql } => CatalogResponse::Rows(answer(&sql)),
            _ => CatalogResponse::Error("mock".into()),
        }));
        fn no_fork(_: u32, _: &str, _: &crate::job_worker::JobParams) -> std::io::Result<crate::job_worker::JobHandle> {
            Err(std::io::Error::other("no fork in this test"))
        }
        let mut manager = crate::job_manager::JobManager::new(
            Arc::new(crate::message_bus::MessageBus::new(16)), ctx.config.clone(), ctx.catalog.clone());
        manager.set_spawner(no_fork);
        ctx.job_manager = Some(Arc::new(Mutex::new(manager)));
        let bootstrap = bootstrap_file("api2_envelope");
        let result = cmd_restore(&crate::session::parse_cli_command(
            &format!("restore jobid=1 client=fd1 all done bootstrap={bootstrap}")), &ctx);
        let envelope: serde_json::Value = serde_json::from_str(&crate::json_output::envelope(&result, false)).unwrap();
        assert_eq!(envelope["result"], serde_json::json!({"run": {"jobid": "630"}}), "{result:?}");
        let _ = std::fs::remove_file(&bootstrap);
    }

    #[test]
    fn file_selection_tree_dialog() {
        let (ctx, _) = context("*UserAgent*");
        let bootstrap = bootstrap_file("tree_dialog");
        let line = format!("restore jobid=1 bootstrap={bootstrap}");
        let (result, transcript) = dialog(&ctx, &line, &["ls", "cd /usr/sbin", "mark z*", "lsmark", "count",
            "done", "no"]);
        assert!(result.success);
        assert!(transcript.contains("cwd is: /\n$ ls\nusr/\n$ cd /usr/sbin\ncwd is: /usr/sbin/\n$ mark z*\n\
            1 file newly marked.\n$ lsmark\n*zic\n$ count\n2 total files/dirs. 1 marked to be restored.\n$ done\n\
            Bootstrap records written to "), "{transcript}");
        assert!(transcript.contains("\n1 file selected to be restored.\n\n"), "{transcript}");
        assert!(transcript.ends_with("OK to run? (yes/mod/no): no\nJob not run.\n"), "{transcript}");
        assert!(std::fs::read_to_string(&bootstrap).unwrap().contains("FileIndex=1\nCount=1\n"));
        // quit leaves without bootstrap
        let (_, transcript) = dialog(&ctx, &line, &["quit"]);
        assert!(transcript.ends_with("$ quit\nRestore not done.\n"), "{transcript}");
        let _ = std::fs::remove_file(&bootstrap);
    }

    #[test]
    fn several_restore_jobs_need_a_selection() {
        let config = format!("{CONFIG}\nJob {{ Name=Other; Type=Restore; Client=fd1; Pool=Full; Storage=File }}");
        let (ctx, _) = context_with("*UserAgent*", &config);
        let bootstrap = bootstrap_file("several_restore_jobs");
        let (result, transcript) = dialog(&ctx,
            &format!("restore jobid=1 client=fd1 restorejob=nosuch all done yes bootstrap={bootstrap}"), &["."]);
        assert!(!result.success);
        assert!(transcript.ends_with("2 files inserted into the tree and marked for extraction.\n\
            Error: Restore Job resource \"nosuch\" does not exist.\n\
            The defined Restore Job resources are:\n1: Other\n2: RestoreFiles\n\
            Select Restore Job (1-2): .\nSelection aborted, nothing done.\n"), "{transcript}");
        let mut started = false;
        restore_with(&crate::session::parse_cli_command(
            &format!("restore jobid=1 client=fd1 restorejob=Other all done yes bootstrap={bootstrap}")),
            &ctx, |_| { started = true; CommandResult::ok(String::new()) });
        assert!(started);
        let _ = std::fs::remove_file(&bootstrap);
    }

    /// Expected values from CompensateShortDate() compiled against the
    /// installed libbareos bsscanf (C-059).
    #[test]
    fn compensate_short_date_like_bsscanf() {
        for (input, expected) in [
            ("2020", "2020-01-01 00:00:00"),
            ("2020-01", "2020-01-01 00:00:00"),
            ("2020-1", "2020-01-01 00:00:00"),
            ("2020-01-01", "2020-01-01 00:00:00"),
            ("2020-1-2 3", "2020-01-02 03:00:00"),
            ("2026-10-16 12:34", "2026-10-16 12:34:00"),
            ("2026-10-16 ::", "2026-10-16 00:00:00"),
            ("2026--16 12:34:56", "2026-01-16 12:34:56"),
            ("2026-10- 12:34:56", "2026-10-01 12:34:56"),
            ("2020-00-00", "2020-01-01 00:00:00"),
            ("2026-00-01 00:00:00", "2026-01-01 00:00:00"),
            ("2026-1-2 3:4:5", "2026-01-02 03:04:05"),
            ("02026-010-016 012:034:056", "2026-10-16 12:34:56"),
            ("2026-10-16 12:34:56 trailing", "2026-10-16 12:34:56"),
            ("2026-10-16\t12:34:56", "2026-10-16 12:34:56"),
            ("2026-10-1612:34:56", "2026-10-1612 00:34:56"),
            ("2026-4294967297-16 12:34:56", "2026-01-16 12:34:56"),
            ("2026-10-16 12:34:56123456789012345678", "2026-10-16 12:34:1141961550"),
            ("18446744073709553642-01-01 00:00:00", "2026-01-01 00:00:00"),
            ("2026-10-16 12:34:56\0garbage", "2026-10-16 12:34:56"),
            // unchanged: garbage, a sign, a short year, other separators
            ("2020x", "2020x"),
            ("2020-01-01x", "2020-01-01x"),
            ("2020-01-01 garbage", "2020-01-01 garbage"),
            ("2026-10-16 12:34:56x", "2026-10-16 12:34:56x"),
            ("2026-10-16 12:34:56.123", "2026-10-16 12:34:56.123"),
            ("1899-01-01", "1899-01-01"),
            ("26-10-16 12:34:56", "26-10-16 12:34:56"),
            ("-10-16 12:34:56", "-10-16 12:34:56"),
            ("+2026-10-16 12:34:56", "+2026-10-16 12:34:56"),
            (" 2026-10-16 12:34:56", " 2026-10-16 12:34:56"),
            ("2026/10/16 12:34:56", "2026/10/16 12:34:56"),
            ("2026-10-16T12:34:56", "2026-10-16T12:34:56"),
            ("4294967297-10-16 12:34:56", "4294967297-10-16 12:34:56"),
            ("x", "x"),
            ("", ""),
            // no range check here (StrToUtime rejects them afterwards)
            ("2026-13-01 00:00:00", "2026-13-01 00:00:00"),
            ("2020-13-45 99:99:99", "2020-13-45 99:99:99"),
            ("2023-02-29 00:00:00", "2023-02-29 00:00:00"),
        ] {
            assert_eq!(compensate_short_date(input), expected, "{input:?}");
        }
    }

    #[test]
    fn helpers() {
        assert_eq!(build_where(Some("/a!"), Some("/p"), Some(".s")), "!/a\\!!!i,!([^/])$!$1.s!,!^!/p!");
        assert!(str_to_utime("2020-01-01 10:00:00") > 0);
        assert_eq!(str_to_utime("2020-01-01 10:00:00x"), 0);
        assert_eq!(str_to_utime("2021-02-29 00:00:00"), 0);
        assert!(is_a_number("+1.5e-3") && !is_a_number("1e") && !is_a_number(""));
        let mut rest = "1,2.5,,x";
        assert_eq!(next_jobid(&mut rest), Ok(Some(1)));
        assert_eq!(next_jobid(&mut rest), Ok(Some(2)));
        assert_eq!(next_jobid(&mut rest), Ok(None));
        assert_eq!(name_valid(" a"), Err("Name cannot start with space.\n".into()));
        assert_eq!(name_valid("a "), Err("Name cannot end with space.\n".into()));
        assert!(regex_check("a(b").is_err() && regex_check("^a.*$").is_ok());
    }
}
