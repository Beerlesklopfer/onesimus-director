//! restore: argument handling of C++ ua_restore.cc RestoreCmd and
//! UserSelectJobidsOrFiles (compatibility log C-021).
//!
//! Everything upstream checks before the file selection is ported in the
//! upstream order: comment, fileregex, regexwhere/where ACL, OpenClientDb,
//! the Restore Job resources, the keyword loop and the JobId list. The
//! JobId menu and the Restore Job selection are console dialogs (C-011);
//! of the menu only "Enter list of comma separated JobIds" and "Cancel" are
//! ported. The other interactive parts (other menu items, file selection
//! tree, client/date questions, "OK to run?") are not available yet; they
//! end the command without creating a job. A restore is only started for a
//! complete selection with `client=… all done yes`.

use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicU32, Ordering};

use bareos_config::resources::JobType;

use crate::acl::{self, Kind};
use crate::command_ast::{Argument, CommandAst};
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::job_manager;
use crate::job_worker::{self, JobParams, WorkerJobType};
use crate::message_bus::{BusMessage, MsgType};
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

/// Everything the start of the restore needs once the selection is complete.
pub(crate) struct Launch {
    pub restore_jobid: u32,
    pub client: String,
    pub restore_client: String,
    pub where_path: Option<String>,
    pub replace: Option<String>,
}

pub fn cmd_restore(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    restore_with(ast, ctx, |launch| start_restore(ctx, launch))
}

/// FindArgWithValue(): first argument with that keyword and a value.
fn arg_with_value<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref())
}

/// FindArg(): keyword present, with or without value.
fn has_arg(ast: &CommandAst, key: &str) -> bool {
    ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key))
}

/// RestoreCmd (ua_restore.cc:103) up to the file selection. `launch` is
/// only called for a complete selection with `yes`.
pub(crate) fn restore_with(ast: &CommandAst, ctx: &ExecContext,
    launch: impl FnOnce(Launch) -> CommandResult) -> CommandResult {
    let mut ua = Ua::new(ctx);

    if let Some(comment) = arg_with_value(ast, "comment") {
        if let Err(message) = comment_legal(comment) {
            ua.error_msg(message);
            return ua.finish(false);
        }
    }

    let where_path = arg_with_value(ast, "where");
    let replace = arg_with_value(ast, "replace");
    let strip_prefix = arg_with_value(ast, "strip_prefix");
    let add_prefix = arg_with_value(ast, "add_prefix");
    let add_suffix = arg_with_value(ast, "add_suffix");
    let mut regexwhere = arg_with_value(ast, "regexwhere").map(str::to_string);

    if let Some(regex) = arg_with_value(ast, "fileregex") {
        if let Err(message) = regex_check(regex) {
            ua.send_msg(&format!("Regex compile error: {message}\n"));
            ua.error_msg("Invalid \"FileRegex\" value.\n");
            return ua.finish(false);
        }
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

    match crate::ua_db::open_client_db(ast, ctx, true) {
        Ok(prefix) => ua.send_msg(&prefix),
        Err(result) => return result,
    }

    // Ensure there is at least one Restore Job
    let restore_jobs = restore_job_names(ctx);
    if restore_jobs.is_empty() {
        let path = crate::reload::config_path().map(|p| p.display().to_string()).unwrap_or_default();
        ua.error_msg(&format!("No Restore Job Resource found in {path}.\n\
            You must create at least one before running this command.\n"));
        return ua.finish(false);
    }

    let jobids = match select_jobids(&mut ua, ast, ctx) {
        Selection::JobIds(jobids) => jobids,
        Selection::Abort => return ua.finish(false),
    };

    // BuildDirectoryTree (ua_restore.cc:1294): without `all` and `done` the
    // file selection tree would follow; with `all done` every file is
    // selected. Then RunCmd asks "OK to run?" unless `yes` was given.
    if !has_arg(ast, "all") || !has_arg(ast, "done") {
        return not_available(ua, "Interactive file selection");
    }
    if jobids.len() != 1 {
        return not_available(ua, "Restore from several JobIds");
    }
    if !has_arg(ast, "yes") {
        return not_available(ua, "Confirmation of the restore job");
    }

    // get_restore_job (ua_select.cc:385)
    if restore_jobs.len() > 1 {
        let restorejob = arg_with_value(ast, "restorejob");
        let found = restorejob.is_some_and(|name| restore_jobs.iter().any(|j| j == name)
            && acl::access(ctx, Kind::Job, name));
        if !found {
            if let Some(name) = restorejob {
                ua.error_msg(&format!("Error: Restore Job resource \"{name}\" does not exist.\n"));
            }
            if crate::ua_select::select_restore_job_resource(&mut ua).is_none() {
                return ua.finish(false);
            }
        }
    }

    // GetClientName (ua_restore.cc:421); the catalog selection list is a
    // dialog (C-011), so the backup client must be given.
    let Some(client) = client_name(&mut ua, ast, ctx) else { return ua.finish(false) };
    let Some(client) = client else {
        return not_available(ua, "Selection of the backup client");
    };

    // GetRestoreClientName (ua_restore.cc:452)
    let restore_client = match arg_with_value(ast, "restoreclient") {
        Some(name) => {
            if let Err(message) = name_valid(name) {
                ua.error_msg(&format!("restoreclient argument: {message}"));
                return ua.finish(false);
            }
            if !client_resource_exists(ctx, name) {
                ua.error_msg(&format!("invalid restoreclient argument: {name}\n"));
                return ua.finish(false);
            }
            name.to_string()
        }
        None => client.clone(),
    };

    // The restore worker only knows `where`.
    if regexwhere.is_some() {
        return not_available(ua, "Restore with regexwhere");
    }

    // RunCmd checks of the generated run command (ua_run.cc:2018,2229).
    if let Some(options) = arg_with_value(ast, "pluginoptions") {
        if !acl::access(ctx, Kind::PluginOptions, options) {
            ua.send_msg("No authorization for \"PluginOptions\" specification.\n");
            return ua.finish(false);
        }
    }
    if !acl::access(ctx, Kind::Client, &restore_client) {
        ua.error_msg(&format!("No authorization. Client \"{restore_client}\".\n"));
        return ua.finish(false);
    }

    let result = launch(Launch {
        restore_jobid: jobids[0],
        client,
        restore_client,
        where_path: where_path.map(str::to_string),
        replace: replace.map(str::to_string),
    });
    let success = result.success;
    ua.send_result(result);
    ua.finish(success)
}

/// End of the ported part: the dialog that follows upstream is not
/// available yet, so nothing is restored.
fn not_available(mut ua: Ua, what: &str) -> CommandResult {
    unavailable(&mut ua, what);
    ua.finish(false)
}

fn unavailable(ua: &mut Ua, what: &str) {
    ua.error_msg(&format!("{what} is not available yet; give jobid=<id> client=<name> all done yes.\n"));
    ua.send_msg("Restore not done.\n");
}

enum Selection {
    /// Selected JobIds (ACL filtered, duplicates of the previous one removed).
    JobIds(Vec<u32>),
    /// Error or cancel (UserSelectJobidsOrFiles returned 0).
    Abort,
}

/// HasValue (ua_restore.cc:411).
fn has_value<'a>(ua: &mut Ua, arg: &'a Argument) -> Option<&'a str> {
    if arg.value.is_none() {
        ua.error_msg(&format!("Missing value for keyword: {}\n", arg.key));
    }
    arg.value.as_deref()
}

/// UserSelectJobidsOrFiles (ua_restore.cc:485).
fn select_jobids(ua: &mut Ua, ast: &CommandAst, ctx: &ExecContext) -> Selection {
    let mut jobids = String::new();
    let mut done = false;
    let mut have_date = false;
    let mut files = 0;
    let mut use_select = false;
    let mut use_fileregex = false;

    for arg in &ast.arguments {
        let Some(keyword) = KEYWORDS.iter().position(|k| k.eq_ignore_ascii_case(&arg.key)) else {
            ua.error_msg(&format!("Unknown keyword: {}\n", arg.key));
            return Selection::Abort;
        };
        match keyword {
            0 => {
                // jobid
                let Some(value) = has_value(ua, arg) else { return Selection::Abort };
                if !jobids.is_empty() {
                    jobids.push(',');
                }
                jobids.push_str(value);
                done = true;
            }
            1 => have_date = true, // current
            2 => {
                // before: a second date ends the selection without a message
                if have_date {
                    return Selection::Abort;
                }
                let Some(value) = has_value(ua, arg) else { return Selection::Abort };
                let date = compensate_short_date(value);
                if str_to_utime(&date) == 0 {
                    ua.error_msg(&format!("Improper date format: {date}\n"));
                    return Selection::Abort;
                }
                have_date = true;
            }
            3 | 4 => {
                // file, directory
                if has_value(ua, arg).is_none() {
                    return Selection::Abort;
                }
                files += 1;
            }
            5 => use_select = true,
            6 => {
                // pool: GetPoolResWithName (with Pool ACL)
                let Some(value) = has_value(ua, arg) else { return Selection::Abort };
                let exists = ctx.config.as_ref()
                    .and_then(|c| c.read().ok().map(|c| c.find_pool(value).is_some()))
                    .unwrap_or(false);
                if !exists || !acl::access(ctx, Kind::Pool, value) {
                    ua.error_msg(&format!("Error: Pool resource \"{value}\" does not exist.\n"));
                    return Selection::Abort;
                }
            }
            8 => use_fileregex = true,
            _ => {} // all and the keywords handled elsewhere
        }
    }

    if files > 0 || use_fileregex {
        // Files are looked up for the backup client in the catalog.
        let Some(client) = client_name(ua, ast, ctx) else { return Selection::Abort };
        if client.is_none() {
            return not_available_selection(ua, "Selection of the backup client");
        }
        return not_available_selection(ua, "Restore by file, directory or fileregex");
    }

    if use_select {
        // SelectBackupsBeforeDate asks for client and fileset.
        return not_available_selection(ua, "Selection of the most recent backups");
    }

    if !done {
        ua.send_msg(INTRO);
        // The selection menu (ua_restore.cc:657).
        ua.start_prompt("To select the JobIds, you have the following choices:\n");
        for item in SELECT_LIST {
            ua.add_prompt(item);
        }
        match ua.do_prompt("", "Select item: ") {
            // error or cancel; 13: Cancel
            None | Some((12, _)) => return Selection::Abort,
            // enter a list of JobIds
            Some((2, _)) => {
                let Some(answer) = ua.get_cmd("Enter JobId(s), comma separated, to restore: ") else {
                    return Selection::Abort;
                };
                jobids = answer;
            }
            Some((_, item)) => return not_available_selection(ua, &format!("Menu item \"{item}\"")),
        }
    }

    filter_jobids(ua, ctx, &jobids)
}

fn not_available_selection(ua: &mut Ua, what: &str) -> Selection {
    unavailable(ua, what);
    Selection::Abort
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

/// GetClientName (ua_restore.cc:421) for the `client=` argument: None on
/// error, Some(None) when no client was given (catalog selection dialog).
fn client_name(ua: &mut Ua, ast: &CommandAst, ctx: &ExecContext) -> Option<Option<String>> {
    let Some(name) = arg_with_value(ast, "client") else { return Some(None) };
    if let Err(message) = name_valid(name) {
        ua.error_msg(&format!("client argument: {message}"));
        return None;
    }
    let sql = format!("SELECT ClientId FROM Client WHERE Name='{name}'");
    let found = ctx.catalog.as_ref()
        .and_then(|catalog| catalog.query_rows(&sql).ok())
        .is_some_and(|table| table.rows.len() == 1);
    if !found {
        ua.error_msg(&format!("invalid client argument: {name}\n"));
        return None;
    }
    Some(Some(name.to_string()))
}

/// The JobId list check of UserSelectJobidsOrFiles (ua_restore.cc:839):
/// every JobId must be a number with a catalog record; JobIds of jobs the
/// console may not access are dropped.
fn filter_jobids(ua: &mut Ua, ctx: &ExecContext, list: &str) -> Selection {
    let mut selected: Vec<u32> = Vec::new();
    let mut last: u32 = 0;
    let mut rest = list;
    loop {
        let jobid = match next_jobid(&mut rest) {
            Err(()) => {
                ua.error_msg("Invalid JobId in list.\n");
                return Selection::Abort;
            }
            Ok(None) => break,
            Ok(Some(jobid)) => jobid,
        };
        if jobid == last {
            continue; // duplicate of last JobId
        }
        last = jobid;
        let name = match job_name(ctx, jobid) {
            Ok(name) => name,
            Err(message) => {
                ua.error_msg(&format!("Unable to get Job record for JobId={jobid}: ERR={message}\n"));
                return Selection::Abort;
            }
        };
        if !acl::access(ctx, Kind::Job, &name) {
            ua.error_msg(&format!("Access to JobId={jobid} (Job \"{name}\") not authorized. Not selected.\n"));
            continue;
        }
        selected.push(jobid);
    }
    if selected.is_empty() {
        ua.warning_msg("No Jobs selected.\n");
        return Selection::Abort;
    }
    let joined = selected.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    let plural = if selected.len() > 1 { "s" } else { "" };
    ua.info_msg(format!("You have selected the following JobId{plural}: {joined}\n"));
    Selection::JobIds(selected)
}

/// Job name of a catalog record (BareosDb::GetJobRecord); the error text
/// carries the source position as the reference binary reports it.
fn job_name(ctx: &ExecContext, jobid: u32) -> Result<String, String> {
    let catalog = ctx.catalog.as_ref().ok_or_else(|| "No catalog\n".to_string())?;
    let table = catalog.query_rows(&format!("SELECT Name FROM Job WHERE JobId={jobid}"))
        .map_err(|e| format!("{e}\n"))?;
    match table.rows.as_slice() {
        [row] => Ok(row.first().cloned().flatten().unwrap_or_default()),
        _ => Err(format!("cats/sql_get.cc:275 No Job found for JobId {jobid}\n")),
    }
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

/// One directive of the sscanf formats used by the date parsers.
#[derive(Clone, Copy)]
enum Scan {
    /// %u (stored in the next field)
    U,
    /// literal character
    Lit(u8),
    /// white space: any amount, also none
    Ws,
    /// %15s (or %s): trailing garbage
    S,
}

/// glibc sscanf for the directives above. `fields` receives the %u values
/// in order, `garbage` the %s value; both keep their previous content where
/// nothing is converted (as the C++ variables do). Returns the number of
/// conversions, -1 for an input failure before the first one.
fn sscanf(input: &str, format: &[Scan], fields: &mut [i64; 6], garbage: &mut String) -> i32 {
    let b = input.as_bytes();
    let mut pos = 0;
    let mut count = 0;
    let mut field = 0;
    let input_failure = |count: i32| if count == 0 { -1 } else { count };
    for directive in format {
        match *directive {
            Scan::Ws => {
                while b.get(pos).is_some_and(u8::is_ascii_whitespace) {
                    pos += 1;
                }
            }
            Scan::Lit(c) => match b.get(pos) {
                None => return input_failure(count),
                Some(&x) if x == c => pos += 1,
                Some(_) => return count,
            },
            Scan::U => {
                while b.get(pos).is_some_and(u8::is_ascii_whitespace) {
                    pos += 1;
                }
                if pos >= b.len() {
                    return input_failure(count);
                }
                let start = pos;
                if matches!(b[pos], b'+' | b'-') {
                    pos += 1;
                }
                let digits = pos;
                while b.get(pos).is_some_and(u8::is_ascii_digit) {
                    pos += 1;
                }
                if pos == digits {
                    return count;
                }
                // %u into an int: the unsigned value wraps to 32 bits.
                fields[field] = i64::from(str_to_int64(&input[start..pos]) as u32 as i32);
                field += 1;
                count += 1;
            }
            Scan::S => {
                while b.get(pos).is_some_and(u8::is_ascii_whitespace) {
                    pos += 1;
                }
                if pos >= b.len() {
                    return input_failure(count);
                }
                let start = pos;
                while pos < b.len() && pos - start < 15 && !b[pos].is_ascii_whitespace() {
                    pos += 1;
                }
                *garbage = String::from_utf8_lossy(&b[start..pos]).into_owned();
                count += 1;
            }
        }
    }
    count
}

use Scan::{Lit, Ws, S, U};
const FULL_DATE: &[Scan] = &[U, Lit(b'-'), U, Lit(b'-'), U, Ws, U, Lit(b':'), U, Lit(b':'), U, S];

/// CompensateShortDate (ua_restore.cc:913) as the reference binary
/// (25.1.2~pre43) behaves: four formats, each counted without the trailing
/// garbage conversion. The 18be77e6 source (unchanged since fc2c79b6) adds
/// year-month and year-only formats and counts the garbage conversion too,
/// which (with its StrToUtime) would reject every date; the binary
/// measurably accepts `2020-01-01`, so the observed behaviour is modelled.
fn compensate_short_date(cmd: &str) -> String {
    let mut tm = [0i64; 6];
    let mut garbage = String::new();
    let formats: [(&[Scan], i32); 4] = [
        (FULL_DATE, 6),
        (&[U, Lit(b'-'), U, Lit(b'-'), U, Ws, U, Lit(b':'), U, S], 5),
        (&[U, Lit(b'-'), U, Lit(b'-'), U, Ws, U, S], 4),
        (&[U, Lit(b'-'), U, Lit(b'-'), U, S], 3),
    ];
    let matched = formats.iter().any(|(format, n)| sscanf(cmd, format, &mut tm, &mut garbage) == *n);
    if !matched || !garbage.is_empty() {
        return cmd.to_string();
    }
    let [mut year, mut mon, mut mday, hour, min, sec] = tm;
    if mon == 0 {
        mon = 1;
    }
    if mday == 0 {
        mday = 1;
    }
    if year >= 1900 {
        year -= 1900;
    } else {
        return cmd.to_string();
    }
    if mon > 0 {
        mon -= 1;
    }
    format!("{}-{:02}-{:02} {:02}:{:02}:{:02}", year + 1900, mon + 1, mday, hour, min, sec)
}

/// StrToUtime (lib/btime.cc:161) of the reference binary: six fields, no
/// trailing garbage, DateIsValid. Returns 0 for a rejected date, otherwise
/// the local time (only the distinction matters here).
fn str_to_utime(s: &str) -> i64 {
    if s.is_empty() {
        return 0;
    }
    let mut tm = [0i64; 6];
    let mut garbage = String::new();
    if sscanf(s, FULL_DATE, &mut tm, &mut garbage) != 6 || !garbage.is_empty() {
        return 0;
    }
    let [year, mon, mday, hour, min, sec] = tm;
    // DateIsValid (lib/btime.cc:128)
    let valid_time = year > 1900 && (0..=23).contains(&hour) && (0..=59).contains(&min) && (0..=59).contains(&sec);
    let date = u32::try_from(mon).ok().zip(u32::try_from(mday).ok())
        .and_then(|(mon, mday)| chrono::NaiveDate::from_ymd_opt(year as i32, mon, mday));
    match date.and_then(|d| d.and_hms_opt(hour as u32, min as u32, sec as u32)) {
        Some(datetime) if valid_time => datetime.and_utc().timestamp().max(1),
        _ => 0,
    }
}

/// Start the restore worker for a complete selection.
fn start_restore(ctx: &ExecContext, launch: Launch) -> CommandResult {
    let Launch { restore_jobid, client: client_name, restore_client, where_path, replace } = launch;
    let where_path = where_path.unwrap_or_else(|| "/tmp/bareos-restore".to_string());
    let replace = replace.unwrap_or_else(|| "always".to_string());

    let config = match &ctx.config {
        Some(c) => match c.read() {
            Ok(c) => c,
            _ => return CommandResult::error("Config read error\n".to_string()),
        },
        None => return CommandResult::error("No config loaded\n".to_string()),
    };

    let client = match config.find_client(&restore_client) {
        Some(c) => c.clone(),
        None => return CommandResult::error(format!("Client '{restore_client}' not found\n")),
    };

    // Generate bootstrap from catalog
    let bootstrap = match job_manager::generate_bootstrap(&ctx.catalog, restore_jobid) {
        Some(bsr) => bsr,
        None => return CommandResult::error(format!(
            "Cannot generate bootstrap for JobId={restore_jobid} — no jobmedia records\n"
        )),
    };

    // Find a storage resource
    let storage = match config.storages.first() {
        Some(s) => s.clone(),
        None => return CommandResult::error("No storage configured\n".to_string()),
    };

    let dir_name = config.director.name.clone();
    let job_id = NEXT_RESTORE_ID.fetch_add(1, Ordering::Relaxed);
    let now = chrono::Local::now();
    let unique_job = format!("RestoreFiles.{}", now.format("%Y-%m-%d_%H.%M.%S_%f"));

    let params = JobParams {
        job_id,
        unique_job: unique_job.clone(),
        job_name: "RestoreFiles".to_string(),
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
        pool_name: "Full".to_string(),
        fileset_includes: vec![],
        fileset_excludes: vec![],
        since_utime: 0,
        prev_job: String::new(),
        where_path: Some(where_path.clone()),
        replace: Some(replace.clone()),
        restore_jobid: Some(restore_jobid),
        bootstrap: Some(bootstrap),
    };

    drop(config); // Release RwLock before fork

    // Post queued status
    if let Some(smgr) = &ctx.session_mgr {
        smgr.bus().post(BusMessage::JobStatus {
            job_id, job_name: "RestoreFiles".to_string(),
            status: "Queued".to_string(), timestamp: now_unix(),
        });
        crate::console_msg::post_job_message(smgr.bus(), ctx.config.as_ref(), BusMessage::JobMessage {
            job_id, job_name: "RestoreFiles".to_string(),
            msg_type: MsgType::Info,
            text: format!("Start Restore JobId={job_id} from Backup JobId={restore_jobid}, Where={where_path}"),
            timestamp: now_unix(),
        });
    }

    // Fork the autonomous restore worker
    match job_worker::spawn_job_worker(job_id, "RestoreFiles", &params) {
        Ok(handle) => {
            tracing::info!("Restore {job_id}: worker pid={}, from backup {restore_jobid}", handle.pid);
            if let Some(ref mgr) = ctx.job_manager {
                let mut mgr = mgr.lock().unwrap();
                mgr.add_job(job_id, "RestoreFiles", handle);
                mgr.set_job_resources(job_id, &client.name, &storage.name);
            }
        }
        Err(e) => {
            tracing::error!("Restore {job_id}: fork failed: {e}");
            return CommandResult::error(format!("Failed to start restore: {e}\n"));
        }
    }

    CommandResult::ok(format!(
        "Restore job queued. JobId={job_id}\n\
         Restore from:  Backup JobId={restore_jobid}\n\
         Client:        {client_name}\n\
         Storage:       {}\n\
         Where:         {where_path}\n\
         Replace:       {replace}\n",
        storage.name,
    ))
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
Job { Name=job1; Type=Backup; Client=fd1 }
Job { Name=RestoreFiles; Type=Restore; Client=fd1 }
Console { Name=limited; Password=pw; CommandACL=*all*; CatalogACL=*all*; ClientACL=*all*; PoolACL=*all*; JobACL=job1; WhereACL=/tmp/ok }
"#;

    fn table(rows: &[&str]) -> SqlTable {
        SqlTable {
            columns: vec![SqlColumn { name: "name".into(), numeric: false }],
            rows: rows.iter().map(|r| vec![Some(r.to_string())]).collect(),
        }
    }

    /// Catalog with client fd1 and the jobs 1 (job1) and 2 (secret); every
    /// request is logged.
    fn context_with(console: &str, config: &str) -> (ExecContext, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let catalog = CatalogHandle::mock(move |request| {
            let CatalogRequest::TextRows { sql } = request else {
                seen.lock().unwrap().push("other request".into());
                return CatalogResponse::Error("double".into());
            };
            seen.lock().unwrap().push(sql.clone());
            CatalogResponse::Rows(match sql.as_str() {
                "SELECT ClientId FROM Client WHERE Name='fd1'" => table(&["1"]),
                "SELECT Name FROM Job WHERE JobId=1" => table(&["job1"]),
                "SELECT Name FROM Job WHERE JobId=2" => table(&["secret"]),
                _ => table(&[]),
            })
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
        assert_eq!(run(&ctx, "restore before=2020").error_message, "Improper date format: 2020\n");
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
        // Without file selection the client is not looked at before the menu.
        assert!(run(&ctx, "restore client=nosuch").text_output.ends_with(MENU_BATCH));
        assert_eq!(run(&ctx, "restore jobid=1 client=nosuch all done yes").error_message,
            "invalid client argument: nosuch\n");
        assert_eq!(run(&ctx, "restore jobid=1 client=fd1 restoreclient=nosuch all done yes").error_message,
            "invalid restoreclient argument: nosuch\n");
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
        // Entered JobIds are checked like jobid=; nothing is restored without `all done yes`.
        let (_, transcript) = dialog(&fresh(), "restore", &["3", "1"]);
        assert!(transcript.ends_with("restore: 1\nYou have selected the following JobId: 1\n\
            Interactive file selection is not available yet; give jobid=<id> client=<name> all done yes.\n\
            Restore not done.\n"), "{transcript}");
        let (_, transcript) = dialog(&fresh(), "restore", &["1"]);
        assert!(transcript.ends_with("Select item:  (1-13): 1\nMenu item \"List last 20 Jobs run\" is not available yet; \
            give jobid=<id> client=<name> all done yes.\nRestore not done.\n"), "{transcript}");
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
        let config = CONFIG.replace("Job { Name=RestoreFiles; Type=Restore; Client=fd1 }", "");
        let (ctx, _) = context_with("*UserAgent*", &config);
        let result = run(&ctx, "restore help");
        assert!(result.error_message.starts_with("No Restore Job Resource found in "));
        assert!(result.error_message.ends_with(".\nYou must create at least one before running this command.\n"));
    }

    #[test]
    fn nothing_starts_without_complete_selection_and_yes() {
        let (ctx, log) = context("*UserAgent*");
        for line in ["restore", "restore help", "restore client=fd1", "restore jobid=1", "restore jobid=1 all",
            "restore jobid=1 done", "restore jobid=1 all done", "restore jobid=1 client=fd1 all done",
            "restore jobid=1 all done yes", "restore jobid=1,2 client=fd1 all done yes",
            "restore jobid=1 client=fd1 strip_prefix=/a all done yes",
            "restore file=/etc/passwd client=fd1 yes", "restore fileregex=x client=fd1 all done yes",
            "restore select all done yes", "restore current all done yes", "restore yes"] {
            run(&ctx, line);
        }
        // Only lookups were sent to the catalog.
        assert!(log.lock().unwrap().iter().all(|sql| sql.starts_with("SELECT ")), "{:?}", log.lock().unwrap());

        let result = run(&ctx, "restore jobid=1 client=fd1 all done");
        assert!(!result.success);
        assert!(result.text_output.ends_with("You have selected the following JobId: 1\n\
            Confirmation of the restore job is not available yet; give jobid=<id> client=<name> all done yes.\n\
            Restore not done.\n"), "{}", result.text_output);

        // Control: the complete request reaches the launch with the selection.
        let mut started = None;
        let result = restore_with(&crate::session::parse_cli_command(
            "restore jobid=1 client=fd1 where=/tmp/x all done yes"), &ctx, |launch| {
                started = Some((launch.restore_jobid, launch.client, launch.restore_client, launch.where_path));
                CommandResult::ok(String::new())
            });
        assert!(result.success);
        assert_eq!(started, Some((1, "fd1".into(), "fd1".into(), Some("/tmp/x".into()))));
    }

    #[test]
    fn several_restore_jobs_need_a_selection() {
        let config = format!("{CONFIG}\nJob {{ Name=Other; Type=Restore; Client=fd1 }}");
        let (ctx, _) = context_with("*UserAgent*", &config);
        let (result, transcript) = dialog(&ctx, "restore jobid=1 client=fd1 restorejob=nosuch all done yes", &["."]);
        assert!(!result.success);
        assert!(transcript.ends_with("You have selected the following JobId: 1\n\
            Error: Restore Job resource \"nosuch\" does not exist.\n\
            The defined Restore Job resources are:\n1: Other\n2: RestoreFiles\n\
            Select Restore Job (1-2): .\nSelection aborted, nothing done.\n"), "{transcript}");
        let mut started = false;
        restore_with(&crate::session::parse_cli_command("restore jobid=1 client=fd1 restorejob=Other all done yes"),
            &ctx, |_| { started = true; CommandResult::ok(String::new()) });
        assert!(started);
    }

    #[test]
    fn helpers() {
        assert_eq!(build_where(Some("/a!"), Some("/p"), Some(".s")), "!/a\\!!!i,!([^/])$!$1.s!,!^!/p!");
        assert_eq!(compensate_short_date("2020"), "2020");
        assert_eq!(compensate_short_date("2020-01-01"), "2020-01-01 00:00:00");
        assert_eq!(compensate_short_date("2020-1-2 3"), "2020-01-02 03:00:00");
        assert_eq!(compensate_short_date("2020-01-01x"), "2020-01-01x");
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
