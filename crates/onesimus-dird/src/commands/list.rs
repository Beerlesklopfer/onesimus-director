//! List commands: list, llist
//!
//! Port of C++ ua_output.cc DoListCmd() and cats/sql_list.cc: the same SQL
//! (cats/postgresql_queries.inc), the same JSON keys (`.api 2`) and the same
//! table/vertical text rendering (see sql_list.rs).

use serde_json::{json, Map, Value};

use crate::catalog_worker::{CatalogHandle, SqlTable};
use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::sql_list::{json_rows, render, ListType};

pub fn cmd_list(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_output.cc DoListCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_list_db(ast, ctx))
}

fn cmd_list_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let ty = if ast.command_name.eq_ignore_ascii_case("llist") { ListType::Vert } else { ListType::Horz };
    let Some(cat) = &ctx.catalog else {
        return CommandResult::error("Could not open catalog database.\n".to_string());
    };
    let mut out = ListOut::default();
    let ok = do_list(&Args(ast), ctx, cat, ty, &mut out);
    out.finish(ok)
}

pub fn cmd_llist(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    cmd_list(ast, ctx)
}

// ---------------------------------------------------------------------------
// Output collection (text + JSON result object), like the OutputFormatter
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ListOut {
    text: String,
    json: Map<String, Value>,
    errors: String,
    /// limit/offset filter → "meta": {"range": {...}} (JsonFinalizeResult)
    range: Option<(u64, Option<u64>)>,
}

impl ListOut {
    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push_str(&msg.into());
    }

    /// ArrayStart(key) + ListResult + ArrayEnd(key)
    fn array(&mut self, key: &str, t: &SqlTable, ty: ListType) {
        self.text.push_str(&render(t, ty));
        self.json.insert(key.to_lowercase(), Value::Array(json_rows(t)));
    }

    fn finish(mut self, ok: bool) -> CommandResult {
        if let Some((limit, offset)) = self.range {
            let mut range = Map::new();
            range.insert("limit".into(), json!(limit));
            if let Some(o) = offset {
                range.insert("offset".into(), json!(o));
            }
            range.insert("filtered".into(), json!(0));
            self.json.insert("meta".into(), json!({ "range": range }));
        }
        if ok && self.errors.is_empty() {
            CommandResult::ok(self.text).with_json(Value::Object(self.json))
        } else {
            let mut r = CommandResult::error(self.errors);
            r.text_output = self.text;
            r.with_json(Value::Object(self.json))
        }
    }
}

// ---------------------------------------------------------------------------
// Argument access with C++ semantics (case-insensitive keywords)
// ---------------------------------------------------------------------------

struct Args<'a>(&'a CommandAst);

impl<'a> Args<'a> {
    /// argk[1] — the list keyword
    fn keyword(&self) -> Option<&'a str> {
        self.0.arguments.first().map(|a| a.key.as_str())
    }
    /// argv[1] — value of the list keyword
    fn keyword_value(&self) -> Option<&'a str> {
        self.0.arguments.first().and_then(|a| a.value.as_deref())
    }
    fn is(&self, word: &str) -> bool {
        self.keyword().is_some_and(|k| k.eq_ignore_ascii_case(word))
    }
    /// FindArg: keyword present (with or without value)
    fn has(&self, name: &str) -> bool {
        self.0.arguments.iter().skip(1).any(|a| a.key.eq_ignore_ascii_case(name))
            || self.is(name)
    }
    /// GetArgValue / FindArgWithValue
    fn value(&self, name: &str) -> Option<&'a str> {
        self.0.arguments.iter()
            .find(|a| a.key.eq_ignore_ascii_case(name) && a.value.is_some())
            .and_then(|a| a.value.as_deref())
    }
}

fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

fn query(cat: &CatalogHandle, sql: &str, out: &mut ListOut) -> Option<SqlTable> {
    match cat.query_rows(sql) {
        Ok(t) => Some(t),
        Err(e) => {
            out.error(format!("Query failed: {e}\n"));
            None
        }
    }
}

/// SetQueryRange(): " LIMIT n [OFFSET m]" and the meta.range filter info.
fn query_range(args: &Args, out: &mut ListOut) -> String {
    let Some(limit) = args.value("limit") else { return String::new() };
    let limit: u64 = limit.parse().unwrap_or(0);
    let mut range = format!(" LIMIT {limit}");
    let offset = args.value("offset").map(|o| o.parse::<u64>().unwrap_or(0));
    if let Some(o) = offset {
        range.push_str(&format!(" OFFSET {o}"));
    }
    out.range = Some((limit, offset));
    range
}

/// GetJobidFromCmdline(): 0 = no jobid argument, -1 = not found, else JobId.
fn jobid_from_cmdline(args: &Args, cat: &CatalogHandle) -> i64 {
    let sql = if let Some(ujobid) = args.value("ujobid") {
        format!("SELECT JobId FROM Job WHERE Job='{}'", esc(ujobid))
    } else if let Some(jobid) = args.value("jobid") {
        let id: i64 = jobid.trim().parse().unwrap_or(0);
        format!("SELECT JobId FROM Job WHERE JobId={id}")
    } else {
        return 0;
    };
    match cat.query_rows(&sql) {
        Ok(t) => t.rows.first()
            .and_then(|r| r[0].as_deref())
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1),
        Err(_) => -1,
    }
}

/// FindClientArgFromDatabase(): Ok(None) no client=, Ok(Some) valid, Err on error.
fn client_from_db<'a>(args: &Args<'a>, cat: &CatalogHandle, out: &mut ListOut) -> Result<Option<&'a str>, ()> {
    let Some(name) = args.value("client") else { return Ok(None) };
    match cat.query_rows(&format!("SELECT ClientId FROM Client WHERE Name='{}'", esc(name))) {
        Ok(t) if !t.rows.is_empty() => Ok(Some(name)),
        _ => {
            out.error(format!("invalid client argument: {name}\n"));
            Err(())
        }
    }
}

const JOBID_NOT_FOUND: &str =
    "jobid not found in db, access to job or client denied by ACL, or client not found in db\n";

// ---------------------------------------------------------------------------
// DoListCmd
// ---------------------------------------------------------------------------

fn do_list(args: &Args, ctx: &ExecContext, cat: &CatalogHandle, ty: ListType, out: &mut ListOut) -> bool {
    let Some(_) = args.keyword() else {
        out.error(format!("{} command requires a keyword\n", args.0.command_name));
        return false;
    };

    let opts = match ListOptions::parse(args) {
        Ok(o) => o,
        Err(e) => {
            out.error(e);
            return false;
        }
    };

    if ["jobs", "job", "jobname", "jobid", "ujobid"].iter().any(|k| args.is(k)) {
        return list_jobs(args, cat, ty, &opts, out);
    }

    let range = query_range(args, out);

    if args.is("jobtotals") {
        let Some(t) = query(cat, "SELECT COUNT(*) AS Jobs, SUM(JobFiles) AS Files, SUM(JobBytes) AS Bytes, \
             Name AS Job FROM Job WHERE Type IN ('B','A','a','C') GROUP BY Name", out) else { return false };
        out.array("jobs", &t, ListType::Horz);
        let Some(t) = query(cat, "SELECT COUNT(*) AS Jobs, SUM(JobFiles) AS Files, SUM(JobBytes) AS Bytes \
             FROM Job WHERE Type IN ('B','A','a','C')", out) else { return false };
        out.text.push_str(&render(&t, ListType::Horz));
        let totals = json_rows(&t).into_iter().next().unwrap_or_else(|| json!({}));
        out.json.insert("jobtotals".into(), totals);
        return true;
    }

    if args.is("basefiles") || args.is("files") {
        let jobid = jobid_from_cmdline(args, cat);
        if jobid <= 0 {
            out.error(JOBID_NOT_FOUND);
            return false;
        }
        let (sql, key) = if args.is("files") {
            (format!("SELECT Path.Path||Name AS Filename \
                 FROM (SELECT PathId, Name FROM File WHERE JobId={jobid} \
                 UNION ALL SELECT PathId, Name FROM BaseFiles JOIN File ON (BaseFiles.FileId = File.FileId) \
                 WHERE BaseFiles.JobId = {jobid}) AS F, Path WHERE Path.PathId=F.PathId"), "filenames")
        } else {
            (format!("SELECT Path.Path||File.Name AS Filename FROM BaseFiles, File, Path \
                 WHERE BaseFiles.JobId={jobid} AND BaseFiles.BaseJobId = File.JobId \
                 AND BaseFiles.FileId = File.FileId AND Path.PathId=File.PathId"), "files")
        };
        let Some(mut t) = query(cat, &sql, out) else { return false };
        // No SQL range here: the C++ OutputFormatter applies limit/offset per row
        if let Some((limit, offset)) = out.range {
            let skip = offset.unwrap_or(0) as usize;
            t.rows = t.rows.into_iter().skip(skip).take(limit as usize).collect();
        }
        // NF_LIST: only rows, no "No results" decoration for big queries
        if !t.rows.is_empty() {
            out.text.push_str(&render(&t, ListType::Nf));
        }
        out.json.insert(key.into(), Value::Array(json_rows(&t)));
        return true;
    }

    if args.is("fileset") || args.is("filesets") {
        let select = "SELECT DISTINCT FileSet.FileSetId AS FileSetId, FileSet, MD5, CreateTime, FileSetText";
        let sql = if args.is("fileset") {
            let filesetid: i64 = args.value("filesetid").and_then(|v| v.parse().ok()).unwrap_or(0);
            let jobid = jobid_from_cmdline(args, cat);
            if jobid > 0 {
                format!("{select} FROM Job, FileSet WHERE Job.FileSetId = FileSet.FileSetId \
                     AND Job.JobId='{jobid}' {range}")
            } else if filesetid > 0 {
                format!("SELECT FileSetId, FileSet, MD5, CreateTime, FileSetText FROM FileSet WHERE FileSetId={filesetid} ")
            } else {
                out.error("jobid not found in db, access to job or client denied by ACL, or client not found in db or missing filesetid\n");
                return false;
            }
        } else {
            format!("{select} FROM FileSet ORDER BY FileSetId ASC {range}")
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("filesets", &t, ty);
        return true;
    }

    if args.is("jobmedia") {
        let jobid = jobid_from_cmdline(args, cat);
        if jobid < 0 {
            out.error(JOBID_NOT_FOUND);
            return false;
        }
        let filter = if jobid > 0 { format!("AND JobMedia.JobId={jobid} ") } else { String::new() };
        let sql = if ty == ListType::Vert {
            format!("SELECT JobMediaId,JobId,Media.MediaId,Media.VolumeName,FirstIndex,LastIndex,StartFile,\
                 JobMedia.EndFile,StartBlock,JobMedia.EndBlock FROM JobMedia,Media \
                 WHERE Media.MediaId=JobMedia.MediaId {filter}")
        } else {
            format!("SELECT JobId,Media.VolumeName,FirstIndex,LastIndex FROM JobMedia,Media \
                 WHERE Media.MediaId=JobMedia.MediaId {filter}")
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("jobmedia", &t, ty);
        return true;
    }

    if args.is("joblog") {
        let jobid = jobid_from_cmdline(args, cat);
        if jobid < 0 {
            out.error(JOBID_NOT_FOUND);
            return false;
        }
        if jobid == 0 {
            return true;
        }
        let (sql, lty) = if opts.count {
            (format!("SELECT COUNT(*) AS count FROM Log WHERE Log.JobId={jobid}"), ty)
        } else {
            (format!("SELECT Time, LogText FROM Log WHERE Log.JobId={jobid} ORDER BY Log.LogId {range}"),
             if ty == ListType::Vert { ListType::Vert } else { ListType::Raw })
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("joblog", &t, lty);
        return true;
    }

    if args.is("log") {
        let client = match client_from_db(args, cat, out) {
            Ok(c) => c,
            Err(()) => return false,
        };
        let filter = client.map(|c| format!("AND Client.Name = '{}' ", esc(c))).unwrap_or_default();
        let range = if range.is_empty() { " LIMIT 5".to_string() } else { range };
        let inner = format!("SELECT LogId, Job.Name AS JobName, Client.Name AS ClientName, Time, LogText \
             FROM Log JOIN Job USING (JobId) LEFT JOIN Client USING (ClientId) WHERE Job.Type != 'C' {filter}\
             ORDER BY Log.LogId DESC {range}");
        let sql = if args.has("reverse") {
            inner
        } else {
            format!("SELECT LogId, JobName, ClientName, Time, LogText FROM ({inner}) AS sub ORDER BY LogId ASC ")
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        // Text mode shows only the log text (other columns hidden), JSON gets all
        let text_only = SqlTable {
            columns: t.columns.iter().skip(4).cloned().collect(),
            rows: t.rows.iter().map(|r| r.iter().skip(4).cloned().collect()).collect(),
        };
        out.text.push_str(&render(&text_only, if ty == ListType::Vert { ListType::Vert } else { ListType::Raw }));
        out.json.insert("log".into(), Value::Array(json_rows(&t)));
        return true;
    }

    if args.is("pool") || args.is("pools") || (args.is("poolid") && args.keyword_value().is_some()) {
        let filter = if args.is("poolid") {
            let id: i64 = args.keyword_value().and_then(|v| v.parse().ok()).unwrap_or(0);
            format!("WHERE poolid={id}")
        } else if let Some(name) = args.keyword_value() {
            format!("WHERE Name='{}'", esc(name))
        } else {
            "ORDER BY PoolId".to_string()
        };
        let select = if ty == ListType::Vert {
            "SELECT PoolId,Name,NumVols,MaxVols,UseOnce,UseCatalog,AcceptAnyVolume,VolRetention,\
             VolUseDuration,MaxVolJobs,MaxVolBytes,AutoPrune,Recycle,PoolType,LabelFormat,Enabled,\
             ScratchPoolId,RecyclePoolId,LabelType "
        } else {
            "SELECT PoolId,Name,NumVols,MaxVols,PoolType,LabelFormat "
        };
        let Some(t) = query(cat, &format!("{select} FROM Pool {filter}"), out) else { return false };
        out.array("pools", &t, ty);
        return true;
    }

    if args.is("clients") || (args.is("client") && args.keyword_value().is_some()) {
        let filter = match args.keyword_value() {
            Some(name) if args.is("client") => format!("WHERE Name = '{}'", esc(name)),
            _ => String::new(),
        };
        let sql = if ty == ListType::Vert {
            format!("SELECT ClientId,Name,Uname,AutoPrune,FileRetention,JobRetention FROM Client {filter} ORDER BY ClientId ")
        } else {
            format!("SELECT ClientId,Name,FileRetention,JobRetention FROM Client {filter} ORDER BY ClientId ")
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("clients", &t, ty);
        return true;
    }

    if args.is("storages") {
        let Some(t) = query(cat, "SELECT * FROM Storage", out) else { return false };
        out.array("storages", &t, ty);
        return true;
    }

    if args.is("media") || args.is("volume") || args.is("volumes") {
        return list_media(args, cat, ty, &range, out);
    }

    if (args.is("mediaid") || args.is("volumeid")) && args.keyword_value().is_some() {
        let id: i64 = args.keyword_value().and_then(|v| v.parse().ok()).unwrap_or(0);
        let sql = format!("{} WHERE MediaId={id} ORDER BY MediaId {range}", volumes_select(ty));
        let Some(t) = query(cat, &sql, out) else { return false };
        out.text.push_str(&render(&t, ty));
        out.json.insert("volume".into(), json_rows(&t).into_iter().next().unwrap_or_else(|| json!({})));
        return true;
    }

    if args.is("copies") {
        let jobids = args.value("jobid");
        if let Some(ids) = jobids {
            if !ids.split(',').all(|i| !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit())) {
                out.error(format!("Unknown list keyword: {}\n", args.keyword().unwrap_or("")));
                return false;
            }
        }
        let filter = jobids.map(|ids| format!(" AND (Job.PriorJobId IN ({ids}) OR Job.JobId IN ({ids})) ")).unwrap_or_default();
        let sql = format!("SELECT DISTINCT Job.PriorJobId AS JobId, Job.Job, Job.JobId AS CopyJobId, Media.MediaType \
             FROM Job JOIN JobMedia USING (JobId) JOIN Media USING (MediaId) \
             WHERE Job.Type = 'c' {filter} ORDER BY Job.PriorJobId DESC {range} ");
        let Some(t) = query(cat, &sql, out) else { return false };
        if !t.rows.is_empty() {
            out.text.push_str(if jobids.is_some() { "These JobIds have copies as follows:\n" }
                              else { "The catalog contains copies as follows:\n" });
            out.array("copies", &t, ty);
        }
        return true;
    }

    // C++ falls through to "Unknown list keyword" when the backups arguments are invalid
    if args.is("backups") && list_backups(args, ctx, cat, ty, &range, out) {
        return true;
    }

    if args.is("jobstatistics") || args.is("jobstats") {
        let jobid = jobid_from_cmdline(args, cat);
        if jobid <= 0 {
            out.error("no jobid given\n");
            return false;
        }
        let sql = format!("SELECT DeviceId, SampleTime, JobId, JobFiles, JobBytes FROM JobStats \
             WHERE JobStats.JobId={jobid} ORDER BY JobStats.SampleTime ");
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("jobstats", &t, ty);
        return true;
    }

    out.error(format!("Unknown list keyword: {}\n", args.keyword().unwrap_or("")));
    false
}

// ---------------------------------------------------------------------------
// list jobs
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ListOptions {
    count: bool,
    last: bool,
    jobstatus: Vec<char>,
    joblevel: Vec<char>,
    jobtype: Vec<char>,
}

impl ListOptions {
    fn parse(args: &Args) -> Result<Self, String> {
        let mut o = ListOptions { count: args.has("count"), last: args.has("last"), ..Default::default() };
        if let Some(v) = args.value("jobstatus") {
            for s in v.split(',') {
                let c = match s.to_ascii_lowercase().as_str() {
                    _ if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => s.as_bytes()[0] as char,
                    "terminated" => 'T',
                    "warnings" => 'W',
                    "canceled" => 'A',
                    "running" => 'R',
                    "error" => 'E',
                    "fatal" => 'f',
                    _ => return Err("invalid jobstatus parameter\n".into()),
                };
                o.jobstatus.push(c);
            }
        }
        if let Some(v) = args.value("joblevel").or_else(|| args.value("level")) {
            for s in v.split(',') {
                if s.len() == 1 {
                    o.joblevel.push(s.as_bytes()[0] as char);
                } else {
                    let lv = LEVELS.iter().find(|(n, _)| n.eq_ignore_ascii_case(s))
                        .ok_or_else(|| "invalid joblevel parameter\n".to_string())?;
                    o.joblevel.push(lv.1);
                }
            }
        }
        if let Some(v) = args.value("jobtype") {
            for s in v.split(',') {
                let t = TYPES.iter().find(|(n, c)| n.eq_ignore_ascii_case(s) || (s.len() == 1 && s.starts_with(*c)))
                    .ok_or_else(|| "invalid jobtype parameter\n".to_string())?;
                o.jobtype.push(t.1);
            }
        }
        Ok(o)
    }
}

const LEVELS: &[(&str, char)] = &[
    ("Full", 'F'), ("Base", 'B'), ("Incremental", 'I'), ("Differential", 'D'), ("Since", 'S'),
    ("VirtualFull", 'f'), ("Catalog", 'C'), ("InitCatalog", 'V'), ("VolumeToCatalog", 'O'),
    ("DiskToCatalog", 'd'), ("Data", 'A'),
];

const TYPES: &[(&str, char)] = &[
    ("Backup", 'B'), ("Admin", 'D'), ("Archive", 'A'), ("Verify", 'V'), ("Restore", 'R'),
    ("Migrate", 'g'), ("Copy", 'c'), ("Consolidate", 'O'),
];

fn sql_char_list(v: &[char]) -> String {
    v.iter().map(|c| format!("'{c}'")).collect::<Vec<_>>().join(",")
}

const LIST_JOBS: &str = "SELECT DISTINCT Job.JobId,Job.Name, Client.Name as Client,
Job.StartTime,
CASE
    WHEN Job.endtime IS NOT NULL AND Job.endtime >= Job.starttime THEN Job.endtime - Job.starttime
    ELSE CURRENT_TIMESTAMP(0) - Job.starttime
    END as Duration,
Job.Type,Job.Level,Job.JobFiles,Job.JobBytes,Job.JobStatus
FROM Job
LEFT JOIN Client ON Client.ClientId=Job.ClientId
LEFT JOIN JobMedia ON JobMedia.JobId=Job.JobId
LEFT JOIN Media ON JobMedia.MediaId=Media.MediaId
LEFT JOIN FileSet ON FileSet.FileSetId=Job.FileSetId
";

const LIST_JOBS_LONG: &str = "SELECT DISTINCT Job.JobId, Job.Job, Job.Name, Job.PurgedFiles, Job.Type, Job.Level,
Job.ClientId, Client.Name as Client, Job.JobStatus, Job.SchedTime, Job.StartTime,
Job.EndTime,Job.RealEndtime,
CASE
    WHEN Job.endtime IS NOT NULL AND Job.endtime >= Job.starttime THEN Job.endtime - Job.starttime
    ELSE CURRENT_TIMESTAMP(0) - Job.starttime
    END as Duration,
Job.JobTDate, Job.VolSessionId, Job.VolSessionTime,
Job.JobFiles, Job.JobBytes, Job.JobErrors, Job.JobMissingFiles, Job.PoolId,
Pool.Name as PoolName, Job.PriorJobId, Job.FileSetId, FileSet.FileSet
FROM Job
LEFT JOIN Client ON Client.ClientId=Job.ClientId
LEFT JOIN Pool ON Pool.PoolId=Job.PoolId
LEFT JOIN JobMedia ON JobMedia.JobId=Job.JobId
LEFT JOIN Media ON JobMedia.MediaId=Media.MediaId
LEFT JOIN FileSet ON FileSet.FileSetId=Job.FileSetId
";

/// postgresql_queries.inc list_jobs / list_jobs_long (+ _last variants)
fn list_jobs_sql(long: bool, last: bool, selection: &str, order_range: &str) -> String {
    let base = if long { LIST_JOBS_LONG } else { LIST_JOBS };
    if last {
        format!("{base}INNER JOIN (
   SELECT MAX(Job.JobId) as MaxJobId
   FROM Job
   LEFT JOIN Client ON Client.ClientId=Job.ClientId
   LEFT JOIN Pool ON Pool.PoolId=Job.PoolId
   LEFT JOIN JobMedia ON JobMedia.JobId=Job.JobId
   LEFT JOIN Media ON JobMedia.MediaId=Media.MediaId
   LEFT JOIN FileSet ON FileSet.FileSetId=Job.FileSetId
   WHERE Job.JobId > 0
   {selection}
   GROUP BY Job.Name
) LastJob
ON Job.JobId = LastJob.MaxJobId
ORDER BY StartTime{order_range};
")
    } else {
        format!("{base}WHERE Job.JobId > 0
{selection}
ORDER BY StartTime{order_range};
")
    }
}

fn list_jobs(args: &Args, cat: &CatalogHandle, ty: ListType, opts: &ListOptions, out: &mut ListOut) -> bool {
    if args.is("jobs") && args.keyword_value().is_some() {
        out.error("'list jobs' does not allow an jobs parameter\n");
        return false;
    }

    let mut selection = String::new();
    if let Some(name) = args.value("jobname").or_else(|| args.value("job")) {
        selection.push_str(&format!("AND Job.Name = '{}' ", esc(name)));
    } else if args.is("job") || args.is("jobname") {
        out.error(format!("Missing {} parameter\n", args.keyword().unwrap_or("")));
        return false;
    }

    let jobid = jobid_from_cmdline(args, cat);
    if jobid > 0 {
        selection.insert_str(0, &format!("AND Job.JobId={jobid} "));
    } else if jobid < 0 {
        // Unknown jobid or no permission: empty object, not an error (C++)
        out.json.insert("jobs".into(), json!({}));
        return true;
    } else if args.is("jobid") || args.is("ujobid") {
        out.error(format!("Missing {} parameter\n", args.keyword().unwrap_or("")));
        return false;
    }

    let since = args.value("hours").and_then(|h| h.parse::<i64>().ok()).map(|h| h * 3600)
        .or_else(|| args.value("days").and_then(|d| d.parse::<i64>().ok()).map(|d| d * 86400));

    let client = match client_from_db(args, cat, out) {
        Ok(c) => c,
        Err(()) => return false,
    };
    if let Some(c) = client {
        selection.push_str(&format!("AND Client.Name = '{}' ", esc(c)));
    }
    if !opts.jobstatus.is_empty() {
        selection.push_str(&format!("AND Job.JobStatus in ({}) ", sql_char_list(&opts.jobstatus)));
    }
    if !opts.joblevel.is_empty() {
        selection.push_str(&format!("AND Job.Level in ({}) ", sql_char_list(&opts.joblevel)));
    }
    if !opts.jobtype.is_empty() {
        selection.push_str(&format!("AND Job.Type in ({}) ", sql_char_list(&opts.jobtype)));
    }
    if let Some(v) = args.value("volume") {
        selection.push_str(&format!("AND Media.Volumename = '{}' ", esc(v)));
    }
    if let Some(p) = args.value("pool") {
        selection.push_str(&format!("AND Job.poolid = (SELECT poolid FROM pool WHERE name = '{}' LIMIT 1) ", esc(p)));
    }
    if let Some(secs) = since {
        let t = chrono::Local::now() - chrono::Duration::seconds(secs);
        selection.push_str(&format!("AND Job.SchedTime > '{}' ", t.format("%Y-%m-%d %H:%M:%S")));
    }

    let range = query_range(args, out);
    let sql = if opts.count {
        format!("SELECT DISTINCT COUNT(DISTINCT Job.JobId) as count
FROM Job
LEFT JOIN Client ON Client.ClientId=Job.ClientId
LEFT JOIN JobMedia ON JobMedia.JobId=Job.JobId
LEFT JOIN Media ON JobMedia.MediaId=Media.MediaId
LEFT JOIN FileSet ON FileSet.FileSetId=Job.FileSetId
WHERE Job.JobId > 0
{selection}
{range};
")
    } else {
        list_jobs_sql(ty == ListType::Vert, opts.last, &selection, &range)
    };
    let Some(t) = query(cat, &sql, out) else { return false };
    out.array("jobs", &t, ty);
    true
}

// ---------------------------------------------------------------------------
// list backups
// ---------------------------------------------------------------------------

fn list_backups(args: &Args, ctx: &ExecContext, cat: &CatalogHandle, ty: ListType, range: &str, out: &mut ListOut) -> bool {
    let client = match client_from_db(args, cat, out) {
        Ok(Some(c)) => c,
        Ok(None) => {
            out.error("missing parameter: client\n");
            return false;
        }
        Err(()) => return false,
    };
    let mut selection = format!("AND Job.Type IN('B', 'A', 'a') AND Client.Name='{}' ", esc(client));

    // jobstatus= (default: terminated + warnings)
    let mut status = Vec::new();
    if let Some(v) = args.value("jobstatus") {
        for s in v.split(',') {
            let c = match s.to_ascii_lowercase().as_str() {
                _ if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => s.as_bytes()[0] as char,
                "terminated" => 'T',
                "warnings" => 'W',
                "canceled" => 'A',
                "running" => 'R',
                "error" => 'E',
                "fatal" => 'f',
                _ => continue,
            };
            status.push(c);
        }
    }
    if status.is_empty() {
        selection.push_str("AND JobStatus IN ('T','W') ");
    } else {
        selection.push_str(&format!(" AND JobStatus IN ({})", sql_char_list(&status)));
    }

    // level= (prefix match on backup levels)
    if let Some(v) = args.value("level") {
        let mut levels = Vec::new();
        for s in v.split(',') {
            for (name, code) in LEVELS.iter().take(6) {
                if name.to_ascii_lowercase().starts_with(&s.to_ascii_lowercase()) {
                    levels.push(*code);
                }
            }
        }
        if !levels.is_empty() {
            selection.push_str(&format!(" AND Level IN ({})", sql_char_list(&levels)));
        }
    }

    // fileset= / any: restrict to configured FileSets (listall)
    match args.value("fileset") {
        Some(fs) if !fs.eq_ignore_ascii_case("any") => {
            selection.push_str(&format!("AND FileSet='{}' ", esc(fs)));
        }
        _ => {
            let names: Vec<String> = ctx.config.as_ref()
                .and_then(|c| c.read().ok())
                .map(|cfg| cfg.filesets.iter().map(|f| f.name.clone()).collect())
                .unwrap_or_default();
            if !names.is_empty() {
                let ors: Vec<String> = names.iter().map(|n| format!("FileSet='{}'", esc(n))).collect();
                selection.push_str(&format!("AND ({}) ", ors.join(" OR ")));
            }
        }
    }

    let mut criteria = String::new();
    if let Some(order) = args.value("order") {
        let o = order.to_ascii_lowercase();
        if "ascending".starts_with(&o) {
            criteria.push_str(" ASC");
        } else if "descending".starts_with(&o) {
            criteria.push_str(" DESC");
        } else {
            out.error(format!("Unknown list keyword: {}\n", args.keyword().unwrap_or("")));
            return false;
        }
    }
    criteria.push_str(range);

    let sql = list_jobs_sql(ty == ListType::Vert, false, &selection, &criteria);
    let Some(t) = query(cat, &sql, out) else { return false };
    out.array("backups", &t, ty);
    true
}

// ---------------------------------------------------------------------------
// list media / volumes
// ---------------------------------------------------------------------------

fn volumes_select(ty: ListType) -> &'static str {
    if ty == ListType::Vert {
        "SELECT    Media.MediaId, Media.VolumeName, Media.Slot, Media.PoolId,
          (SELECT Name FROM Pool WHERE Pool.PoolId=Media.PoolId) AS Pool,
          Media.MediaType, Media.FirstWritten, Media.LastWritten, Media.LabelDate,
          Media.VolJobs, Media.VolFiles, Media.VolBlocks, Media.VolMounts, Media.VolBytes,
          Media.VolErrors, Media.VolWrites, Media.VolCapacityBytes, Media.VolStatus,
          Media.Enabled, Media.Recycle, Media.VolRetention, Media.VolUseDuration,
          Media.MaxVolJobs, Media.MaxVolFiles, Media.MaxVolBytes, Media.InChanger,
          Media.EndFile, Media.EndBlock, Media.LabelType, Media.StorageId, Media.DeviceId,
          Media.LocationId, Media.RecycleCount, Media.InitialWrite, Media.ScratchPoolId,
          (SELECT Name FROM Pool WHERE Pool.PoolId=Media.ScratchPoolId) AS ScratchPool,
          Media.RecyclePoolId,
          (SELECT Name FROM Pool WHERE Pool.PoolId=Media.RecyclePoolId) AS RecyclePool,
          CASE WHEN COALESCE(Media.EncryptionKey, '') = ''
               THEN 0
               ELSE 1
          END AS HasEncryptionKey,
          Media.Comment,
          Storage.Name AS Storage
FROM      Media
LEFT JOIN Storage USING(StorageId)
"
    } else {
        "SELECT    MediaId,VolumeName,VolStatus,Enabled,
          VolBytes,VolFiles,VolRetention,Recycle,Slot,InChanger,
          MediaType,LastWritten,Name AS Storage,
          CASE WHEN COALESCE(Media.EncryptionKey, '') = ''
               THEN 0
               ELSE 1
          END AS HasEncryptionKey
FROM      Media
LEFT JOIN Storage USING(StorageId)
"
    }
}

fn list_media(args: &Args, cat: &CatalogHandle, ty: ListType, range: &str, out: &mut ListOut) -> bool {
    let jobid = jobid_from_cmdline(args, cat);
    if jobid > 0 {
        let sql = if ty == ListType::Vert {
            format!("SELECT JobMediaId,JobId,Media.MediaId,Media.VolumeName FROM JobMedia,Media \
                 WHERE Media.MediaId=JobMedia.MediaId AND JobMedia.JobId={jobid} ")
        } else {
            format!("SELECT DISTINCT Media.VolumeName FROM JobMedia,Media \
                 WHERE Media.MediaId=JobMedia.MediaId AND JobMedia.JobId={jobid} ")
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("volumes", &t, ty);
        return true;
    }
    if jobid < 0 {
        return true;
    }

    // list volume=<name>
    if let Some(name) = args.keyword_value() {
        let sql = format!("{} WHERE VolumeName='{}'", volumes_select(ty), esc(name));
        let Some(t) = query(cat, &sql, out) else { return false };
        out.text.push_str(&render(&t, ty));
        out.json.insert("volume".into(), json_rows(&t).into_iter().next().unwrap_or_else(|| json!({})));
        return true;
    }

    // pool=<name>: flat array
    if let Some(pool) = args.value("pool") {
        let id = cat.query_rows(&format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(pool)))
            .ok().and_then(|t| t.rows.first().and_then(|r| r[0].clone()));
        let Some(id) = id else {
            out.error(format!("Pool {pool} doesn't exist.\n"));
            return true;
        };
        let sql = format!("{} WHERE PoolId={id} ORDER BY MediaId {range}", volumes_select(ty));
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("volumes", &t, ty);
        return true;
    }

    // all: flat over all pools
    if args.has("all") && !args.is("all") {
        let sql = format!("{} ORDER BY MediaId {range}", volumes_select(ty));
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("volumes", &t, ty);
        return true;
    }

    // grouped by pool: {"volumes": {"<pool>": [...]}}
    let Some(pools) = query(cat, "SELECT PoolId, Name FROM Pool", out) else { return false };
    let mut grouped = Map::new();
    for row in &pools.rows {
        let (Some(id), Some(name)) = (&row[0], &row[1]) else { continue };
        let sql = format!("{} WHERE PoolId={id} ORDER BY MediaId {range}", volumes_select(ty));
        let Some(t) = query(cat, &sql, out) else { return false };
        out.text.push_str(&format!("Pool: {name}\n"));
        out.text.push_str(&render(&t, ty));
        grouped.insert(name.to_lowercase(), Value::Array(json_rows(&t)));
    }
    out.json.insert("volumes".into(), Value::Object(grouped));
    true
}
