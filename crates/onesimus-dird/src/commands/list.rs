//! List commands: list, llist
//!
//! Port of C++ ua_output.cc DoListCmd() and cats/sql_list.cc: the same SQL
//! (cats/postgresql_queries.inc), the same JSON keys (`.api 2`) and the same
//! table/vertical text rendering (see sql_list.rs).

use chrono::{Datelike, Local, TimeZone, Timelike};
use serde_json::{json, Map, Value};

use crate::catalog_worker::{CatalogHandle, SqlTable};
use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::{CommandResult, MsgKind};
use crate::acl::Kind;
use crate::sql_list::{add_commas, json_rows, render_shown, ListType};

pub fn cmd_list(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_output.cc DoListCmd: OpenClientDb() before anything else.
    match crate::ua_db::open_client_db(ast, ctx, true) {
        Ok(prefix) => cmd_list_db(ast, ctx, prefix),
        Err(result) => result,
    }
}

fn cmd_list_db(ast: &CommandAst, ctx: &ExecContext, prefix: String) -> CommandResult {
    let ty = if ast.command_name.eq_ignore_ascii_case("llist") { ListType::Vert } else { ListType::Horz };
    let Some(cat) = &ctx.catalog else {
        return crate::ua_db::with_prefix(prefix, CommandResult::error("Could not open catalog database.\n".to_string()));
    };
    // The catalog messages of OpenClientDb come first (also before a dialog).
    let mut out = ListOut { ctx: Some(ctx), text: prefix, ..Default::default() };
    let ok = do_list(&Args(ast), ctx, cat, ty, &mut out);
    out.finish(ok)
}

pub fn cmd_llist(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    cmd_list(ast, ctx)
}

// ---------------------------------------------------------------------------
// Output collection (text + JSON result object), like the OutputFormatter
// ---------------------------------------------------------------------------

/// Configured resource types the OutputFormatter filters look up (R_JOB, ...).
#[derive(Clone, Copy, Debug)]
enum Res {
    Job,
    Client,
    Pool,
    FileSet,
    Storage,
}

/// OutputFormatter filter tuples (lib/output_formatter.h) on a row column,
/// evaluated by ua_output.cc filterit().
#[derive(Clone, Copy, Debug)]
enum Filter {
    /// OF_FILTER_ACL (SetAclFilter): the console may access the resource
    /// (compatibility log C-026)
    Acl(usize, crate::acl::Kind),
    /// OF_FILTER_RESOURCE (`current`): the resource is configured
    Resource(usize, Res),
    /// OF_FILTER_ENABLED (`enabled`)
    Enabled(usize, Res),
    /// OF_FILTER_DISABLED (`disabled`)
    Disabled(usize, Res),
}

#[derive(Default)]
struct ListOut<'c> {
    ctx: Option<&'c ExecContext>,
    text: String,
    json: Map<String, Value>,
    errors: String,
    /// limit/offset filter → "meta": {"range": {...}} (JsonFinalizeResult)
    range: Option<(u64, Option<u64>)>,
    /// Row filters; any filter (or a range) makes HasFilters() true.
    filters: Vec<Filter>,
    /// get_num_rows_filtered()
    filtered: u64,
    /// WarningMsg/InfoMsg calls (their text is also in `text`).
    messages: Vec<(MsgKind, String)>,
    /// Console dialog (StartPrompt/DoPrompt) of this command: once started,
    /// all output goes through it in C++ order.
    ua: Option<crate::ua_select::Ua<'c>>,
}

impl<'c> ListOut<'c> {
    /// The dialog of this command, with the output so far handed over.
    fn dialog(&mut self) -> &mut crate::ua_select::Ua<'c> {
        let ctx = self.ctx.expect("a dialog needs the command context");
        let pending = self.take_pending();
        let ua = self.ua.get_or_insert_with(|| crate::ua_select::Ua::new(ctx));
        ua.send_result(pending);
        ua
    }

    /// The output collected so far (text, ErrorMsg, Warning/InfoMsg).
    fn take_pending(&mut self) -> CommandResult {
        CommandResult {
            text_output: std::mem::take(&mut self.text),
            error_message: std::mem::take(&mut self.errors),
            messages: std::mem::take(&mut self.messages),
            ..Default::default()
        }
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push_str(&msg.into());
    }

    /// SetAclFilter(): an ACL tuple only when the console's ACL of this
    /// type has restrictions (ua_output.cc:619).
    fn acl(&mut self, column: usize, kind: crate::acl::Kind) {
        if self.ctx.is_some_and(|ctx| crate::acl::has_restrictions(ctx, kind)) {
            self.filters.push(Filter::Acl(column, kind));
        }
    }

    /// ua->WarningMsg(): output text plus BNET_WARNING_MSG / "warning" entry.
    fn warning(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.text.push_str(&msg);
        self.messages.push((MsgKind::Warning, msg));
    }

    /// FilterData() for every row: which rows are shown. Suppressed rows
    /// are counted for meta.range.filtered.
    fn shown(&mut self, t: &SqlTable) -> Vec<bool> {
        if self.filters.is_empty() {
            return vec![true; t.rows.len()];
        }
        let config = self.ctx.and_then(|ctx| ctx.config.as_ref()).and_then(|c| c.read().ok()).map(|c| c.clone());
        let shown: Vec<bool> = t.rows.iter().map(|row| self.filter_data(config.as_ref(), row)).collect();
        self.filtered += shown.iter().filter(|s| !**s).count() as u64;
        shown
    }

    /// OutputFormatter::FilterData (lib/output_formatter.cc): any suppressing
    /// tuple hides the row; ACL tuples are tri-state, so a row whose ACL
    /// columns are all empty (unknown) and none explicitly allowed is hidden.
    fn filter_data(&self, config: Option<&crate::director_config::DirectorConfig>, row: &[Option<String>]) -> bool {
        let (mut acl_show, mut acl_unknown) = (0, 0);
        for filter in &self.filters {
            match *filter {
                Filter::Acl(column, kind) => {
                    // A column past the row is read out of bounds in C++; suppressed.
                    let Some(value) = row.get(column) else { return false };
                    match value.as_deref().filter(|v| !v.is_empty()) {
                        None => acl_unknown += 1,
                        Some(name) => {
                            // AclAccessOk(acltype, value, false)
                            if !self.ctx.is_some_and(|ctx| crate::acl::access(ctx, kind, name)) {
                                return false;
                            }
                            acl_show += 1;
                        }
                    }
                }
                f => {
                    if !self.keep(config, f, row) {
                        return false;
                    }
                }
            }
        }
        !(acl_unknown > 0 && acl_show == 0)
    }

    /// filterit(): false = OF_FILTER_STATE_SUPPRESS. An empty column value is
    /// OF_FILTER_STATE_UNKNOWN, which only matters for ACL tuples.
    fn keep(&self, config: Option<&crate::director_config::DirectorConfig>, filter: Filter, row: &[Option<String>]) -> bool {
        let (Filter::Resource(column, res) | Filter::Enabled(column, res) | Filter::Disabled(column, res)) = filter else {
            return true;
        };
        // A column past the row (enabled/disabled on the one-column count
        // result) is read out of bounds in C++; observed: suppressed.
        let Some(value) = row.get(column) else { return false };
        let Some(name) = value.as_deref().filter(|v| !v.is_empty()) else { return true };
        let (Some(ctx), Some(config)) = (self.ctx, config) else { return true };
        match filter {
            // my_config->GetResWithName(): configuration only, no ACL
            Filter::Resource(..) => match res {
                Res::Job => config.find_job(name).is_some(),
                Res::Client => config.find_client(name).is_some(),
                Res::Pool => config.find_pool(name).is_some(),
                Res::FileSet => config.find_fileset(name).is_some(),
                Res::Storage => config.find_storage(name).is_some(),
            },
            // ua->Get<Res>ResWithName(): with the console ACL
            Filter::Enabled(..) | Filter::Disabled(..) => {
                let want = matches!(filter, Filter::Enabled(..));
                use crate::acl::access;
                match res {
                    Res::Job => config.find_job(name).is_some_and(|j| j.enabled == want) && access(ctx, Kind::Job, name),
                    Res::Client => config.find_client(name).is_some_and(|c| c.enabled == want) && access(ctx, Kind::Client, name),
                    Res::Storage => config.find_storage(name).is_some_and(|s| s.enabled == want) && access(ctx, Kind::Storage, name),
                    Res::Pool | Res::FileSet => true,
                }
            }
            Filter::Acl(..) => true,
        }
    }

    /// ArrayStart(key) + ListResult + ArrayEnd(key)
    fn array(&mut self, key: &str, t: &SqlTable, ty: ListType) {
        let shown = self.shown(t);
        self.text.push_str(&render_shown(t, ty, &shown));
        self.json.insert(key.to_lowercase(), Value::Array(shown_json(t, &shown)));
    }

    fn finish(mut self, ok: bool) -> CommandResult {
        if let Some(mut ua) = self.ua.take() {
            ua.send_result(self.take_pending());
            // After a dialog the C++ return value alone decides (BNET_CMD_OK).
            return ua.finish(ok);
        }
        if ok && self.errors.is_empty() && (self.range.is_some() || !self.filters.is_empty()) {
            let mut range = Map::new();
            if let Some((limit, offset)) = self.range {
                range.insert("limit".into(), json!(limit));
                if let Some(o) = offset {
                    range.insert("offset".into(), json!(o));
                }
            }
            range.insert("filtered".into(), json!(self.filtered));
            self.json.insert("meta".into(), json!({ "range": range }));
        }
        let mut r = if ok && self.errors.is_empty() {
            let result = CommandResult::ok(self.text);
            // `.api 1` NF_LIST rows have no newline (Decoration): send as is
            if crate::sql_list::decorations() { result } else { result.verbatim() }
        } else {
            let mut r = CommandResult::error(self.errors);
            r.text_output = self.text;
            r
        };
        r.messages = self.messages;
        r.with_json(Value::Object(self.json))
    }
}

fn shown_json(t: &SqlTable, shown: &[bool]) -> Vec<Value> {
    json_rows(t).into_iter().zip(shown).filter(|(_, s)| **s).map(|(v, _)| v).collect()
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
    /// FindArg(ua, name) > 0: present after the list keyword
    fn has_after_keyword(&self, name: &str) -> bool {
        self.0.arguments.iter().skip(1).any(|a| a.key.eq_ignore_ascii_case(name))
    }
    /// GetArgValue / FindArgWithValue
    fn value(&self, name: &str) -> Option<&'a str> {
        self.0.arguments.iter()
            .find(|a| a.key.eq_ignore_ascii_case(name))
            .and_then(|a| a.value.as_deref())
    }
}

fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// lib/edit.cc str_to_int64: optional blanks and sign, then leading digits.
fn str_to_int64(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let value = digits.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |acc, b| acc.wrapping_mul(10).wrapping_add((b - b'0') as i64));
    if neg { -value } else { value }
}

/// lib/edit.cc Is_a_number_list: "1,2,3" (a trailing comma is accepted).
fn is_a_number_list(s: &str) -> bool {
    let mut previous_digit = false;
    let mut digit_seen = false;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            previous_digit = true;
            digit_seen = true;
        } else if b == b',' && previous_digit {
            previous_digit = false;
        } else {
            return false;
        }
    }
    digit_seen
}

/// lib/edit.cc edit_uint64_with_suffix: "1.234 K", "0 ".
fn with_suffix(value: u64) -> String {
    const SUFFIX: &[&str] = &["", "K", "M", "G", "T", "P", "E", "Z", "Y", "FIX ME"];
    let mut mbuf = add_commas(&value.to_string());
    let commas = mbuf.matches(',').count();
    if let Some(first) = mbuf.find(',') {
        let rest: String = mbuf[first + 1..].split(',').next().unwrap_or("").to_string();
        mbuf = format!("{}.{}", &mbuf[..first], rest);
        mbuf.truncate(5);
    }
    format!("{mbuf} {}", SUFFIX[commas.min(SUFFIX.len() - 1)])
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

/// GetJobidFromCmdline(): 0 = no jobid argument, -1 = not found or no
/// access to the job's Job/Client (audited ACL checks), else JobId.
fn jobid_from_cmdline(args: &Args, ctx: &ExecContext, cat: &CatalogHandle) -> i64 {
    let select = "SELECT Job.JobId, Job.Name, Job.ClientId, Client.Name FROM Job LEFT JOIN Client ON Client.ClientId=Job.ClientId";
    let sql = if let Some(ujobid) = args.value("ujobid") {
        format!("{select} WHERE Job.Job='{}'", esc(ujobid))
    } else if let Some(jobid) = args.value("jobid") {
        let id = str_to_int64(jobid);
        format!("{select} WHERE Job.JobId={id}")
    } else {
        return 0;
    };
    let Ok(t) = cat.query_rows(&sql) else { return -1 };
    let Some(row) = t.rows.first() else { return -1 };
    let field = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let Ok(jobid) = field(0).parse::<i64>() else { return -1 };
    if !crate::acl::access_audited(ctx, Kind::Job, &field(1)) {
        return -1;
    }
    if str_to_int64(&field(2)) != 0 {
        // GetClientRecord fails when the client row is gone
        let Some(client) = row.get(3).cloned().flatten() else { return -1 };
        if !crate::acl::access_audited(ctx, Kind::Client, &client) {
            return -1;
        }
    }
    jobid
}

/// FindClientArgFromDatabase(): Ok(None) no client=, Ok(Some) valid, Err on error.
fn client_from_db<'a>(args: &Args<'a>, ctx: &ExecContext, cat: &CatalogHandle, out: &mut ListOut) -> Result<Option<&'a str>, ()> {
    let Some(name) = args.value("client") else { return Ok(None) };
    match cat.query_rows(&format!("SELECT ClientId FROM Client WHERE Name='{}'", esc(name))) {
        // AclAccessOk(Client_ACL, cr.Name): a denial fails silently
        Ok(t) if !t.rows.is_empty() => if crate::acl::access(ctx, Kind::Client, name) { Ok(Some(name)) } else { Err(()) },
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

    let opts = match ListOptions::parse(args, out) {
        Ok(o) => o,
        Err(e) => {
            out.error(e);
            return false;
        }
    };

    if ["jobs", "job", "jobname", "jobid", "ujobid"].iter().any(|k| args.is(k)) {
        return list_jobs(args, ctx, cat, ty, &opts, out);
    }

    let range = query_range(args, out);

    if args.is("jobtotals") {
        let Some(t) = query(cat, "SELECT COUNT(*) AS Jobs, SUM(JobFiles) AS Files, SUM(JobBytes) AS Bytes, \
             Name AS Job FROM Job WHERE Type IN ('B','A','a','C') GROUP BY Name", out) else { return false };
        out.array("jobs", &t, ListType::Horz);
        let Some(t) = query(cat, "SELECT COUNT(*) AS Jobs, SUM(JobFiles) AS Files, SUM(JobBytes) AS Bytes \
             FROM Job WHERE Type IN ('B','A','a','C')", out) else { return false };
        let shown = out.shown(&t);
        out.text.push_str(&render_shown(&t, ListType::Horz, &shown));
        let totals = shown_json(&t, &shown).into_iter().next().unwrap_or_else(|| json!({}));
        out.json.insert("jobtotals".into(), totals);
        return true;
    }

    if args.is("basefiles") || args.is("files") {
        let jobid = jobid_from_cmdline(args, ctx, cat);
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
        let Some(t) = query(cat, &sql, out) else { return false };
        // BigSqlQuery: limit/offset only reach meta.range (filterit ignores
        // them), and the per-row ListResult prints nothing for no rows.
        let shown = out.shown(&t);
        if !t.rows.is_empty() {
            out.text.push_str(&render_shown(&t, ListType::Nf, &shown));
        }
        out.json.insert(key.into(), Value::Array(shown_json(&t, &shown)));
        return true;
    }

    if args.is("fileset") {
        let filesetid = args.value("filesetid").map(str_to_int64).unwrap_or(0);
        let jobid = jobid_from_cmdline(args, ctx, cat);
        if jobid > 0 || filesetid > 0 {
            out.acl(1, Kind::FileSet);
            if opts.current {
                out.filters.push(Filter::Resource(1, Res::FileSet));
            }
            let sql = if jobid != 0 {
                format!("SELECT DISTINCT FileSet.FileSetId AS FileSetId, FileSet, MD5, CreateTime, FileSetText \
                     FROM Job, FileSet WHERE Job.FileSetId = FileSet.FileSetId \
                     AND Job.JobId='{}' {range}", jobid as u32)
            } else {
                format!("SELECT FileSetId, FileSet, MD5, CreateTime, FileSetText FROM FileSet WHERE FileSetId={filesetid} ")
            };
            let Some(t) = query(cat, &sql, out) else { return false };
            out.array("filesets", &t, ty);
            return true;
        }
        out.error("jobid not found in db, access to job or client denied by ACL, or client not found in db or missing filesetid\n");
        return false;
    }

    if args.is("filesets") {
        out.acl(1, Kind::FileSet);
        if opts.current {
            out.filters.push(Filter::Resource(1, Res::FileSet));
        }
        let sql = format!("SELECT DISTINCT FileSet.FileSetId AS FileSetId, FileSet, MD5, CreateTime, FileSetText \
             FROM FileSet ORDER BY FileSetId ASC {range}");
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("filesets", &t, ty);
        return true;
    }

    if args.is("jobmedia") {
        let jobid = jobid_from_cmdline(args, ctx, cat);
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
        let jobid = jobid_from_cmdline(args, ctx, cat);
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
        let client = match client_from_db(args, ctx, cat, out) {
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
        // SetHiddenColumnAclFilter / SetAclFilter on JobName and ClientName
        out.acl(1, Kind::Job);
        out.acl(2, Kind::Client);
        let Some(t) = query(cat, &sql, out) else { return false };
        let shown = out.shown(&t);
        // Outside JSON mode LogId, JobName, ClientName and Time are hidden columns
        let text_only = SqlTable {
            columns: t.columns.iter().skip(4).cloned().collect(),
            rows: t.rows.iter().map(|r| r.iter().skip(4).cloned().collect()).collect(),
        };
        out.text.push_str(&render_shown(&text_only, if ty == ListType::Vert { ListType::Vert } else { ListType::Raw }, &shown));
        out.json.insert("log".into(), Value::Array(shown_json(&t, &shown)));
        return true;
    }

    if args.is("pool") || args.is("pools") {
        return list_pools(args, ctx, cat, ty, &opts, out);
    }

    if args.is("poolid") && args.keyword_value().is_some() {
        out.acl(1, Kind::Pool);
        if opts.current {
            out.filters.push(Filter::Resource(1, Res::Pool));
        }
        let id = args.keyword_value().map(str_to_int64).unwrap_or(0);
        let filter = if id > 0 { format!("WHERE poolid={id}") } else { "ORDER BY PoolId".to_string() };
        let Some(t) = query(cat, &format!("{} FROM Pool {filter}", pool_select(ty)), out) else { return false };
        out.array("pools", &t, ty);
        return true;
    }

    if args.is("clients") || (args.is("client") && args.keyword_value().is_some()) {
        out.acl(1, Kind::Client);
        if opts.current {
            out.filters.push(Filter::Resource(1, Res::Client));
        }
        if opts.enabled {
            out.filters.push(Filter::Enabled(1, Res::Client));
        }
        if opts.disabled {
            out.filters.push(Filter::Disabled(1, Res::Client));
        }
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
        out.acl(1, Kind::Storage);
        if opts.current {
            out.filters.push(Filter::Resource(1, Res::Storage));
        }
        if opts.enabled {
            out.filters.push(Filter::Enabled(1, Res::Storage));
        }
        if opts.disabled {
            out.filters.push(Filter::Disabled(1, Res::Storage));
        }
        let Some(t) = query(cat, "SELECT * FROM Storage", out) else { return false };
        out.array("storages", &t, ty);
        return true;
    }

    if args.is("media") || args.is("volume") || args.is("volumes") {
        return list_media(args, ctx, cat, ty, &opts, &range, out);
    }

    if (args.is("mediaid") || args.is("volumeid")) && args.keyword_value().is_some() {
        let id = args.keyword_value().map(str_to_int64).unwrap_or(0);
        // ListMediaRecords in count mode only knows VolumeName and PoolId
        let sql = if opts.count {
            VOLUMES_COUNT.to_string()
        } else if id > 0 {
            format!("{} WHERE MediaId={id} ORDER BY MediaId {range}", volumes_select(ty))
        } else {
            format!("{} ORDER BY MediaId {range}", volumes_select(ty))
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        object(out, "volume", &t, ty);
        return true;
    }

    if args.is("nextvol") || args.is("nextvolume") {
        return list_nextvol(args, ctx, cat, out);
    }

    if args.is("copies") {
        match args.value("jobid") {
            Some(ids) if !is_a_number_list(ids) => {}
            jobids => {
                let filter = jobids.filter(|ids| !ids.is_empty())
                    .map(|ids| format!(" AND (Job.PriorJobId IN ({ids}) OR Job.JobId IN ({ids})) ")).unwrap_or_default();
                let sql = format!("SELECT DISTINCT Job.PriorJobId AS JobId, Job.Job, Job.JobId AS CopyJobId, Media.MediaType \
                     FROM Job JOIN JobMedia USING (JobId) JOIN Media USING (MediaId) \
                     WHERE Job.Type = 'c' {filter} ORDER BY Job.PriorJobId DESC {range} ");
                let Some(t) = query(cat, &sql, out) else { return false };
                if !t.rows.is_empty() {
                    if crate::sql_list::decorations() { // Decoration, sql_list.cc:367
                        out.text.push_str(if jobids.is_some() { "These JobIds have copies as follows:\n" }
                                          else { "The catalog contains copies as follows:\n" });
                    }
                    out.array("copies", &t, ty);
                }
                return true;
            }
        }
    }

    // C++ falls through to "Unknown list keyword" when the backups arguments are invalid
    if args.is("backups") {
        if let Some(sql) = list_backups_sql(args, ctx, cat, ty, &range, out) {
            match ty {
                ListType::Vert => {
                    out.acl(2, Kind::Job);
                    out.acl(22, Kind::Pool);
                    if opts.current {
                        out.filters.push(Filter::Resource(2, Res::Job));
                        out.filters.push(Filter::Resource(22, Res::Pool));
                    }
                    if opts.enabled {
                        out.filters.push(Filter::Enabled(2, Res::Job));
                    }
                    if opts.disabled {
                        out.filters.push(Filter::Disabled(2, Res::Job));
                    }
                }
                _ => {
                    out.acl(1, Kind::Job);
                    if opts.current {
                        out.filters.push(Filter::Resource(1, Res::Job));
                    }
                    if opts.enabled {
                        out.filters.push(Filter::Enabled(1, Res::Job));
                    }
                    if opts.disabled {
                        out.filters.push(Filter::Disabled(1, Res::Job));
                    }
                }
            }
            // ListSqlQuery(verbose=false): a failing query prints nothing
            let Ok(t) = cat.query_rows(&sql) else { return false };
            out.array("backups", &t, ty);
            return true;
        }
    }

    if args.is("jobstatistics") || args.is("jobstats") {
        let jobid = jobid_from_cmdline(args, ctx, cat);
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

/// ObjectStart(key) + ListResult + ObjectEnd(key): the row object(s) inside
/// a JSON object instead of an array.
fn object(out: &mut ListOut, key: &str, t: &SqlTable, ty: ListType) {
    let shown = out.shown(t);
    out.text.push_str(&render_shown(t, ty, &shown));
    out.json.insert(key.into(), shown_json(t, &shown).into_iter().next().unwrap_or_else(|| json!({})));
}

// ---------------------------------------------------------------------------
// list jobs
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ListOptions {
    count: bool,
    last: bool,
    current: bool,
    enabled: bool,
    disabled: bool,
    jobstatus: Vec<char>,
    joblevel: Vec<char>,
    jobtype: Vec<char>,
}

/// Single-character codes accepted as given ('A' <= c <= 'z', ua_select.cc).
fn status_char(s: &str) -> Option<char> {
    (s.len() == 1 && (b'A'..=b'z').contains(&s.as_bytes()[0])).then(|| s.as_bytes()[0] as char)
}

impl ListOptions {
    /// ListCmdOptions::parse (ua_output.cc) with GetUserJobStatusSelection,
    /// GetUserJobLevelSelection and GetUserJobTypeListSelection (ua_select.cc).
    fn parse(args: &Args, out: &mut ListOut) -> Result<Self, String> {
        let mut o = ListOptions {
            current: args.has("current"), enabled: args.has("enabled"), disabled: args.has("disabled"),
            count: args.has("count"), last: args.has("last"), ..Default::default()
        };
        if let Some(v) = args.value("jobstatus") {
            if v.is_empty() {
                return Err("invalid jobstatus parameter\n".into());
            }
            for s in v.split(',') {
                let c = match status_char(s) {
                    Some(c) => c,
                    None => match s.to_ascii_lowercase().as_str() {
                        "terminated" => 'T',
                        "warnings" => 'W',
                        "canceled" => 'A',
                        "running" => 'R',
                        "error" => 'E',
                        "fatal" => 'f',
                        _ => return Err("invalid jobstatus parameter\n".into()),
                    },
                };
                o.jobstatus.push(c);
            }
        }
        if let Some(v) = args.value("joblevel").or_else(|| args.value("level")) {
            for s in v.split(',') {
                let c = match status_char(s) {
                    Some(c) => c,
                    None => match s.to_ascii_lowercase().as_str() {
                        "full" => 'F',
                        "differential" => 'D',
                        "incremental" => 'I',
                        _ => return Err("invalid joblevel parameter\n".into()),
                    },
                };
                o.joblevel.push(c);
            }
        }
        if let Some(v) = args.value("jobtype") {
            for s in v.split(',') {
                // GetParsedJobType: one character is the code, else the exact (case-sensitive) name
                let t = TYPES.iter().find(|(n, c)| if s.len() == 1 { s.starts_with(*c) } else { *n == s });
                let Some(t) = t else {
                    out.warning(format!("Illegal jobtype {s}\n"));
                    return Err("invalid jobtype parameter\n".into());
                };
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

/// `days=`/`hours=` of ListJobs: `now - secs * n` computed in C `int`
/// (wraps for large values), 0 = no time selection.
fn schedtime(args: &Args, now: i64) -> i64 {
    let mut schedtime = 0;
    if let Some(v) = args.value("days") {
        schedtime = now - 86400i32.wrapping_mul(str_to_int64(v) as i32) as i64;
    }
    if let Some(v) = args.value("hours") {
        schedtime = now - 3600i32.wrapping_mul(str_to_int64(v) as i32) as i64;
    }
    schedtime
}

fn list_jobs(args: &Args, ctx: &ExecContext, cat: &CatalogHandle, ty: ListType, opts: &ListOptions, out: &mut ListOut) -> bool {
    if args.is("jobs") && args.keyword_value().is_some() {
        out.error("'list jobs' does not allow an jobs parameter\n");
        return false;
    }

    let mut name = None;
    if let Some(n) = args.value("jobname").or_else(|| args.value("job")) {
        name = Some(n);
    } else if args.is("job") || args.is("jobname") {
        out.error(format!("Missing {} parameter\n", args.keyword().unwrap_or("")));
        return false;
    }

    let jobid = jobid_from_cmdline(args, ctx, cat);
    if jobid < 0 {
        // Unknown jobid or no permission: empty object, not an error (C++)
        out.json.insert("jobs".into(), json!({}));
        return true;
    } else if jobid == 0 && (args.is("jobid") || args.is("ujobid")) {
        out.error(format!("Missing {} parameter\n", args.keyword().unwrap_or("")));
        return false;
    }

    let since = schedtime(args, chrono::Utc::now().timestamp());

    let client = match client_from_db(args, ctx, cat, out) {
        Ok(c) => c,
        Err(()) => return false,
    };

    // ListJobs: filter tuples on the job/client (pool, fileset) columns
    let (job_col, client_col) = if ty == ListType::Vert { (2, 7) } else { (1, 2) };
    // count result is one column, no filtering
    if !opts.count {
        out.acl(job_col, Kind::Job);
        out.acl(client_col, Kind::Client);
        if ty == ListType::Vert {
            out.acl(22, Kind::Pool);
            out.acl(25, Kind::FileSet);
        }
    }
    if !opts.count && opts.current {
        out.filters.push(Filter::Resource(job_col, Res::Job));
        out.filters.push(Filter::Resource(client_col, Res::Client));
        if ty == ListType::Vert {
            out.filters.push(Filter::Resource(22, Res::Pool));
            out.filters.push(Filter::Resource(25, Res::FileSet));
        }
    }
    // also on the one-column count result (see ListOut::keep)
    if opts.enabled {
        out.filters.push(Filter::Enabled(job_col, Res::Job));
    }
    if opts.disabled {
        out.filters.push(Filter::Disabled(job_col, Res::Job));
    }

    // BareosDb::ListJobRecords selection
    let mut selection = String::new();
    if jobid > 0 {
        selection.push_str(&format!("AND Job.JobId={jobid} "));
    }
    if let Some(n) = name {
        selection.push_str(&format!("AND Job.Name = '{}' ", esc(n)));
    }
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
    if since != 0 {
        let t = Local.timestamp_opt(since, 0).single().unwrap_or_else(Local::now);
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
        // "reverse": ORDER BY StartTime DESC
        let order_range = if args.has("reverse") { format!(" DESC{range}") } else { range };
        list_jobs_sql(ty == ListType::Vert, opts.last, &selection, &order_range)
    };
    let Some(t) = query(cat, &sql, out) else { return false };
    out.array("jobs", &t, ty);
    true
}

// ---------------------------------------------------------------------------
// list backups
// ---------------------------------------------------------------------------

/// ParseListBackupsCmd(): the list_jobs query, None when an argument is
/// invalid (the caller then reports "Unknown list keyword").
fn list_backups_sql(args: &Args, ctx: &ExecContext, cat: &CatalogHandle, ty: ListType, range: &str, out: &mut ListOut) -> Option<String> {
    let client = match client_from_db(args, ctx, cat, out) {
        Ok(Some(c)) => c,
        Ok(None) => {
            out.error("missing parameter: client\n");
            return None;
        }
        Err(()) => return None,
    };
    let mut selection = format!("AND Job.Type IN('B', 'A', 'a') AND Client.Name='{}' ", esc(client));

    // parse_jobstatus_selection_param (default: terminated + warnings);
    // unknown names are skipped
    let mut status = Vec::new();
    if let Some(v) = args.value("jobstatus") {
        for s in v.split(',') {
            let c = match status_char(s) {
                Some(c) => c,
                None => match s.to_ascii_lowercase().as_str() {
                    "terminated" => 'T',
                    "warnings" => 'W',
                    "canceled" => 'A',
                    "running" => 'R',
                    "error" => 'E',
                    "fatal" => 'f',
                    _ => continue,
                },
            };
            status.push(c);
        }
    }
    if status.is_empty() {
        selection.push_str("AND JobStatus IN ('T','W') ");
    } else {
        selection.push_str(&format!(" AND JobStatus IN ({})", sql_char_list(&status)));
    }

    // parse_level_selection_param: prefix match on the backup levels; every
    // argument counts, matched or not, which can yield invalid SQL like C++
    if let Some(v) = args.value("level") {
        let mut temp = String::new();
        let mut cnt = 0;
        for s in v.split(',') {
            for (name, code) in LEVELS.iter().take(6) {
                if name.len() >= s.len() && name[..s.len()].eq_ignore_ascii_case(s) {
                    if cnt == 0 {
                        temp.push_str(&format!(" AND Level IN ('{code}'"));
                    } else {
                        temp.push_str(&format!(",'{code}'"));
                    }
                }
            }
            cnt += 1;
        }
        if cnt > 0 {
            temp.push(')');
        }
        selection.push_str(&temp);
    }

    // parse_fileset_selection_param(listall = true)
    match args.value("fileset") {
        Some(fs) if !fs.eq_ignore_ascii_case("any") => {
            // AclAccessOk(FileSet_ACL, fileset, true): audited
            if !crate::acl::access_audited(ctx, Kind::FileSet, fs) {
                out.error("Access to specified FileSet not allowed.\n");
                return None;
            }
            selection.push_str(&format!("AND FileSet='{}' ", esc(fs)));
        }
        _ => {
            let names: Vec<String> = ctx.config.as_ref()
                .and_then(|c| c.read().ok())
                .map(|cfg| cfg.filesets.iter().map(|f| f.name.clone()).collect())
                .unwrap_or_default();
            let ors: Vec<String> = names.iter()
                .filter(|n| crate::acl::access(ctx, crate::acl::Kind::FileSet, n))
                .map(|n| format!("FileSet='{}'", esc(n))).collect();
            if !ors.is_empty() {
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
            return None;
        }
    }
    criteria.push_str(range);

    Some(list_jobs_sql(ty == ListType::Vert, false, &selection, &criteria))
}

// ---------------------------------------------------------------------------
// list pools
// ---------------------------------------------------------------------------

fn pool_select(ty: ListType) -> &'static str {
    if ty == ListType::Vert {
        POOL_FIELDS
    } else {
        "SELECT PoolId,Name,NumVols,MaxVols,PoolType,LabelFormat "
    }
}

/// QueryPoolListRows / ListPoolRecords(VERT_LIST)
const POOL_FIELDS: &str = "SELECT PoolId,Name,NumVols,MaxVols,UseOnce,UseCatalog,AcceptAnyVolume,VolRetention,\
     VolUseDuration,MaxVolJobs,MaxVolBytes,AutoPrune,Recycle,PoolType,LabelFormat,Enabled,\
     ScratchPoolId,RecyclePoolId,LabelType ";

/// `list pool[s][=name]`: the pool table plus the prune report of one named
/// pool; `llist` prints the records as "key: value" lines with the prune
/// summary (ua_output.cc EmitPoolFields).
fn list_pools(args: &Args, ctx: &ExecContext, cat: &CatalogHandle, ty: ListType, opts: &ListOptions, out: &mut ListOut) -> bool {
    let name = args.keyword_value().unwrap_or("");
    out.acl(1, Kind::Pool); // PoolName
    if opts.current {
        out.filters.push(Filter::Resource(1, Res::Pool));
    }
    let filter = if name.is_empty() { "ORDER BY PoolId".to_string() } else { format!("WHERE Name='{}'", esc(name)) };

    let json_detail = ty == ListType::Horz && ctx.api_mode == ApiMode::Json && !name.is_empty();
    if ty == ListType::Vert || json_detail {
        let Ok(t) = cat.query_rows(&format!("{POOL_FIELDS} FROM Pool {filter}")) else { return true };
        let shown = out.shown(&t);
        let mut pools = Vec::new();
        for (row, _) in t.rows.iter().zip(&shown).filter(|(_, s)| **s) {
            let field = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
            let Some(report) = pool_prune_report(ctx, cat, str_to_int64(&field(0))) else {
                let what = if json_detail { "details" } else { "summary" };
                out.error(format!("Failed to compute prune {what} for pool {}.\n", field(1)));
                out.json.insert("pools".into(), Value::Array(pools));
                return false;
            };
            let mut obj = Map::new();
            let mut emit = |key: &str, value: String, text: &mut String| {
                text.push_str(&format!("{key}: {value}\n"));
                obj.insert(key.into(), Value::String(value));
            };
            for (i, col) in t.columns.iter().enumerate() {
                emit(&col.name.to_lowercase(), field(i), &mut out.text);
            }
            emit("prunablevolumes", report.volumes.len().to_string(), &mut out.text);
            emit("prunablejobs", report.jobs.len().to_string(), &mut out.text);
            emit("prunablebytes", report.bytes().to_string(), &mut out.text);
            if json_detail {
                report.json(&mut obj);
            }
            pools.push(Value::Object(obj));
        }
        out.json.insert("pools".into(), Value::Array(pools));
        return true;
    }

    let Some(t) = query(cat, &format!("{} FROM Pool {filter}", pool_select(ty)), out) else { return false };
    out.array("pools", &t, ty);

    if ty == ListType::Horz && !name.is_empty() {
        let id = cat.query_rows(&format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(name))).ok()
            .and_then(|t| t.rows.first().and_then(|r| r[0].clone()));
        if let Some(id) = id {
            let Some(report) = pool_prune_report(ctx, cat, str_to_int64(&id)) else {
                out.error(format!("Failed to compute prune details for pool {name}.\n"));
                return false;
            };
            if crate::sql_list::decorations() {
                out.text.push_str(&report.text()); // Decoration only, ua_output.cc:228-280
            }
        }
    }
    true
}

/// ua_prune.h PoolPruneVolumeDetail
struct PruneVolume {
    name: String,
    status: String,
    last_written: String,
    job_ids: Vec<i64>,
    bytes: u64,
}

/// ua_prune.h PoolPruneReport: the volumes (Full/Used, not archived) whose
/// jobs are older than the volume retention, and those jobs.
struct PruneReport {
    volumes: Vec<PruneVolume>,
    /// (jobid, name, bytes, starttime) ordered by jobid
    jobs: Vec<(i64, String, u64, String)>,
    /// (status, volumes) in volume order
    statuses: Vec<(String, u64)>,
}

const PRUNE_REASON: &str = "volume_retention_expired";

impl PruneReport {
    fn reason(&self) -> &'static str {
        if self.volumes.is_empty() { "" } else { PRUNE_REASON }
    }

    fn bytes(&self) -> u64 {
        self.jobs.iter().map(|j| j.2).sum()
    }

    /// EmitPoolPruneReportText
    fn text(&self) -> String {
        let count = |n: u64| add_commas(&n.to_string());
        let mut s = String::from("\nPrune details:\n");
        s.push_str(&format!("  Reason: {}\n", self.reason()));
        s.push_str(&format!("  Prunable volumes: {}\n", count(self.volumes.len() as u64)));
        s.push_str(&format!("  Prunable jobs: {}\n", count(self.jobs.len() as u64)));
        s.push_str(&format!("  Prunable bytes: {}\n", with_suffix(self.bytes())));
        s.push_str("  Affected statuses:\n");
        if self.statuses.is_empty() {
            s.push_str("    none\n");
        }
        for (status, n) in &self.statuses {
            s.push_str(&format!("    - {status} ({})\n", count(*n)));
        }
        s.push_str("  Prunable volumes:\n");
        if self.volumes.is_empty() {
            s.push_str("    none\n");
        }
        for v in &self.volumes {
            s.push_str(&format!("    - {} | status={} | lastwritten={} | jobs={} | bytes={}\n",
                v.name, v.status, v.last_written, count(v.job_ids.len() as u64), with_suffix(v.bytes)));
        }
        s.push_str("  Prunable jobs:\n");
        if self.jobs.is_empty() {
            s.push_str("    none\n");
        }
        for (jobid, name, bytes, start) in &self.jobs {
            s.push_str(&format!("    - {} | {name} | {start} | {}\n", count(*jobid as u64), with_suffix(*bytes)));
        }
        s
    }

    /// EmitPoolPruneReportJson
    fn json(&self, obj: &mut Map<String, Value>) {
        obj.insert("reason".into(), json!(self.reason()));
        obj.insert("status_breakdown".into(), Value::Array(self.statuses.iter()
            .map(|(status, n)| json!({"status": status, "volumes": n})).collect()));
        obj.insert("volumes".into(), Value::Array(self.volumes.iter().map(|v| json!({
            "name": v.name, "status": v.status, "lastwritten": v.last_written, "reason": PRUNE_REASON,
            "prunablejobs": v.job_ids.len(), "prunablebytes": v.bytes, "jobids": v.job_ids,
        })).collect()));
        obj.insert("jobs".into(), Value::Array(self.jobs.iter().map(|(jobid, name, bytes, start)| json!({
            "jobid": jobid, "name": name, "bytes": bytes, "starttime": start,
        })).collect()));
    }
}

/// GetPoolPruneReport (ua_prune.cc): read-only catalog queries.
fn pool_prune_report(ctx: &ExecContext, cat: &CatalogHandle, pool_id: i64) -> Option<PruneReport> {
    let now = chrono::Utc::now().timestamp();
    let ids = |t: &SqlTable| -> Vec<i64> { t.rows.iter().filter_map(|r| r[0].as_deref().map(str_to_int64)).collect() };
    let running: Vec<i64> = ctx.job_manager.as_ref().and_then(|m| m.lock().ok())
        .map(|m| m.running_jobs().into_iter().map(|j| j.0 as i64).collect()).unwrap_or_default();
    let media = cat.query_rows(&format!("SELECT MediaId, VolumeName, VolStatus, Enabled, VolRetention, LastWritten \
         FROM Media WHERE PoolId={pool_id} ORDER BY MediaId")).ok()?;
    let mut volumes = Vec::new();
    for row in &media.rows {
        let field = |i: usize| row[i].clone().unwrap_or_default();
        if field(3) == "2" || !(field(2) == "Full" || field(2) == "Used") {
            continue; // VOL_ARCHIVED, or not prunable
        }
        // GetPruneCandidatesForVolume: sel_JobMedia + copy/migration children
        let retention = str_to_int64(&field(4));
        let mut jobs = ids(&cat.query_rows(&format!("SELECT DISTINCT JobMedia.JobId FROM JobMedia, Job \
             WHERE MediaId = {} AND Job.JobId = JobMedia.JobId AND Job.JobTDate < {} ORDER BY JobId",
             field(0), now - retention)).ok()?);
        if !jobs.is_empty() {
            let list = jobs.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            jobs.extend(ids(&cat.query_rows(&format!("SELECT JobId from Job WHERE type in ('c','g') \
                 AND priorjobid in ({list})")).ok()?));
        }
        jobs.retain(|j| !running.contains(j));
        if jobs.is_empty() {
            continue;
        }
        jobs.sort_unstable();
        jobs.dedup();
        volumes.push(PruneVolume { name: field(1), status: field(2), last_written: field(5), job_ids: jobs, bytes: 0 });
    }

    let mut all: Vec<i64> = volumes.iter().flat_map(|v| v.job_ids.iter().copied()).collect();
    all.sort_unstable();
    all.dedup();
    let mut jobs = Vec::new();
    if !all.is_empty() {
        let list = all.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        let t = cat.query_rows(&format!("SELECT JobId, Name, JobBytes, StartTime FROM Job WHERE JobId IN ({list}) \
             ORDER BY JobId")).ok()?;
        for row in &t.rows {
            let Some(id) = row[0].as_deref() else { continue };
            let field = |i: usize| row[i].clone().unwrap_or_default();
            jobs.push((str_to_int64(id), field(1), field(2).parse().unwrap_or(0), field(3)));
        }
    }
    for v in &mut volumes {
        v.bytes = v.job_ids.iter().filter_map(|id| jobs.iter().find(|j| j.0 == *id)).map(|j| j.2).sum();
    }

    // BuildPoolPruneReport: statuses in volume order, volumes by LastWritten (empty last), name
    let mut statuses: Vec<(String, u64)> = Vec::new();
    for v in &volumes {
        match statuses.iter_mut().find(|s| s.0 == v.status) {
            Some(s) => s.1 += 1,
            None => statuses.push((v.status.clone(), 1)),
        }
    }
    volumes.sort_by(|a, b| match (a.last_written.is_empty(), b.last_written.is_empty()) {
        _ if a.last_written == b.last_written => a.name.cmp(&b.name),
        (true, _) => std::cmp::Ordering::Greater,
        (_, true) => std::cmp::Ordering::Less,
        _ => a.last_written.cmp(&b.last_written),
    });
    Some(PruneReport { volumes, jobs, statuses })
}

// ---------------------------------------------------------------------------
// list media / volumes
// ---------------------------------------------------------------------------

pub(crate) fn volumes_select(ty: ListType) -> &'static str {
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

/// postgresql_queries.inc list_volumes_count_0
const VOLUMES_COUNT: &str = "SELECT COUNT(DISTINCT Media.MediaId) as count FROM Media;\n";

/// BareosDb::ListMediaRecords for a pool (count or records)
fn media_of_pool_sql(ty: ListType, count: bool, pool_id: &str, range: &str) -> String {
    if count {
        format!("SELECT COUNT(DISTINCT Media.MediaId) as count\nFROM Media\nWHERE Media.PoolId={pool_id}\n")
    } else {
        format!("{} WHERE PoolId={pool_id} ORDER BY MediaId {range}", volumes_select(ty))
    }
}

fn list_media(args: &Args, ctx: &ExecContext, cat: &CatalogHandle, ty: ListType, opts: &ListOptions, range: &str, out: &mut ListOut) -> bool {
    let jobid = jobid_from_cmdline(args, ctx, cat);
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
        let sql = if opts.count {
            format!("SELECT COUNT(DISTINCT Media.MediaId) as count\nFROM Media\nWHERE VolumeName='{}'\n", esc(name))
        } else {
            format!("{} WHERE VolumeName='{}'", volumes_select(ty), esc(name))
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        object(out, "volume", &t, ty);
        return true;
    }

    // pool=<name>: flat array; GetPoolDbr asks for a pool when it is unknown
    if let Some(pool) = args.value("pool") {
        let lookup = |name: &str| cat.query_rows(&format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(name)))
            .ok().and_then(|t| t.rows.first().and_then(|r| r[0].clone()));
        let mut id = lookup(pool).filter(|_| crate::acl::access(ctx, crate::acl::Kind::Pool, pool));
        if id.is_none() {
            out.error(format!("Could not find Pool \"{pool}\": ERR=Pool record not found in Catalog.\n"));
            // SelectPoolDbr: the pool= argument once more, then the dialog
            match crate::ua_select::select_pool_dbr(out.dialog(), args.0, "pool") {
                Some(name) => id = lookup(&name),
                None => {
                    out.error(format!("Pool {pool} doesn't exist.\n"));
                    return true;
                }
            }
        }
        let Some(id) = id else { return true };
        let Some(t) = query(cat, &media_of_pool_sql(ty, opts.count, &id, range), out) else { return false };
        out.array("volumes", &t, ty);
        return true;
    }

    // all: flat over all pools
    if args.has_after_keyword("all") {
        // The flat result has no pool column the Pool ACL could filter on
        // in the short listing (ua_output.cc ListMedia).
        if ty != ListType::Vert && crate::acl::has_restrictions(ctx, Kind::Pool) {
            out.error("Restricted permission. Use the commands 'list media' or 'llist media all' instead\n");
            return false;
        }
        out.acl(4, Kind::Pool); // PoolName
        if opts.current {
            // column 4 is the pool name in the llist query (VolBytes in the list query, as in C++)
            out.filters.push(Filter::Resource(4, Res::Pool));
        }
        let sql = if opts.count {
            VOLUMES_COUNT.to_string()
        } else {
            format!("{} ORDER BY MediaId {range}", volumes_select(ty))
        };
        let Some(t) = query(cat, &sql, out) else { return false };
        out.array("volumes", &t, ty);
        return true;
    }

    // grouped by pool (GetPoolIds order): {"volumes": {"<pool>": [...]}}
    let Some(pools) = query(cat, "SELECT PoolId, Name FROM Pool", out) else { return false };
    let mut grouped = Map::new();
    for row in &pools.rows {
        let (Some(id), Some(name)) = (&row[0], &row[1]) else { continue };
        if !crate::acl::access(ctx, Kind::Pool, name) {
            continue;
        }
        let Some(t) = query(cat, &media_of_pool_sql(ty, opts.count, id, range), out) else { return false };
        if crate::sql_list::decorations() {
            out.text.push_str(&format!("Pool: {name}\n")); // Decoration, ua_output.cc:813
        }
        let shown = out.shown(&t);
        out.text.push_str(&render_shown(&t, ty, &shown));
        grouped.insert(name.to_lowercase(), Value::Array(shown_json(&t, &shown)));
    }
    out.json.insert("volumes".into(), Value::Object(grouped));
    true
}

// ---------------------------------------------------------------------------
// list nextvolume
// ---------------------------------------------------------------------------

/// ListNextvol (ua_output.cc): for every Run of the job's schedule within
/// `days`, the volume FindNextVolumeForAppend would choose.
///
/// Read-only: upstream calls it with fnv_no_create_vol but fnv_prune, so it
/// may also recycle purged volumes, prune volumes, mark expired Append
/// volumes Full/Used and create a missing pool. Onesimus only looks the
/// volume up (Append, then Unlabeled; expired volumes are skipped instead of
/// being marked) and changes nothing in the catalog.
fn list_nextvol(args: &Args, ctx: &ExecContext, cat: &CatalogHandle, out: &mut ListOut) -> bool {
    use crate::acl::access;
    const MAX_DAYS: i64 = 50; // kDefaultNumberOfDays
    let mut days = 1;
    if let Some(v) = args.value("days") {
        days = str_to_int64(v) as i32 as i64;
        if !(0..=MAX_DAYS).contains(&days) {
            out.warning(format!("Ignoring invalid value for days. Max is {MAX_DAYS}.\n"));
            days = 1;
        }
    }
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()).map(|c| c.clone()) else { return false };

    // GetJobResWithName (with Job ACL), else select_job_resource
    let job = args.value("job").and_then(|name| config.find_job(name))
        .filter(|job| access(ctx, Kind::Job, &job.name)).cloned();
    let job = match job {
        Some(job) => job,
        None => {
            let Some(name) = crate::ua_select::select_job_resource(out.dialog()) else { return false };
            match config.find_job(&name) {
                Some(job) => job.clone(),
                None => return false,
            }
        }
    };

    let raw = |kind: &str, name: &str| config.raw.iter().find(|r| r.norm_type() == kind && r.name() == Some(name));
    let runs: Vec<_> = raw("schedule", &job.schedule).map(|s| s.directives.iter()
        .filter(|d| d.norm_key() == "run").map(|d| super::show::parse_run(&d.tokens)).collect()).unwrap_or_default();

    let now = chrono::Utc::now().timestamp();
    let mut found = false;
    for run in &runs {
        if !triggers_within(run, now, days) {
            continue;
        }
        // CompleteJcrForJob: the (run) pool must be in the catalog
        let pool = run.pool.clone().unwrap_or_else(|| job.pool.clone());
        let pool_id = cat.query_rows(&format!("SELECT PoolId FROM Pool WHERE Name='{}'", esc(&pool))).ok()
            .and_then(|t| t.rows.first().and_then(|r| r[0].clone()));
        let Some(pool_id) = pool_id else {
            // upstream creates the pool here (catalog write): not done
            found = false;
            break;
        };
        let level = run.level.and_then(|l| super::show::JOB_LEVELS.iter().find(|(_, c)| *c == l))
            .map(|(n, _)| *n).unwrap_or("");
        // GetJobStorage: run pool storage, run storage, pool storage, job storage
        let pool_storage = |name: &str| raw("pool", name)
            .and_then(|p| p.directives.iter().find(|d| d.norm_key() == "storage"))
            .and_then(|d| d.first().map(str::to_string));
        let storage = run.pool.as_deref().and_then(pool_storage)
            .or_else(|| run.storage.clone())
            .or_else(|| pool_storage(&job.pool))
            .unwrap_or_else(|| job.storage.clone());
        let Some(store) = config.find_storage(&storage) else { continue };
        let storage_id = cat.query_rows(&format!("SELECT StorageId FROM Storage WHERE Name='{}'", esc(&store.name))).ok()
            .and_then(|t| t.rows.first().and_then(|r| r[0].clone())).unwrap_or_else(|| "0".into());
        let in_changer = crate::ua_select::is_autochanger(&config, &store.name);
        match find_next_volume(cat, &pool_id, &store.media_type, &storage_id, in_changer) {
            Some(volume) => {
                out.text.push_str(&format!("The next Volume to be used by Job \"{}\" (Pool={pool}, Level={level}) will be {volume}\n",
                    job.name));
                found = true;
            }
            None => out.error(format!("Could not find next Volume for Job {} (Pool={pool}, Level={level}).\n", job.name)),
        }
    }
    if !found {
        out.error(format!("Could not find next Volume for Job {}.\n", job.name));
        return false;
    }
    true
}

/// FindNextVolumeForAppend steps 1 and 2 (BareosDb::FindNextVolume with
/// VolStatus Append, then Unlabeled), retried without the InChanger
/// restriction. HasVolumeExpired volumes are skipped (upstream marks them
/// Full/Used in the catalog and searches again).
fn find_next_volume(cat: &CatalogHandle, pool_id: &str, media_type: &str, storage_id: &str, mut in_changer: bool) -> Option<String> {
    let now = chrono::Utc::now().timestamp();
    loop {
        let changer = if in_changer { format!("AND InChanger=1 AND StorageId={storage_id}") } else { String::new() };
        for status in ["Append", "Unlabeled"] {
            let sql = format!("SELECT VolumeName,VolJobs,VolFiles,VolBytes,MaxVolBytes,MaxVolJobs,MaxVolFiles,\
                 VolUseDuration,FirstWritten FROM Media WHERE PoolId={pool_id} AND MediaType='{}' AND Enabled=1 \
                 AND VolStatus='{status}' {changer} ORDER BY LastWritten IS NULL,LastWritten DESC,MediaId",
                 esc(media_type));
            let t = cat.query_rows(&sql).ok()?;
            for row in &t.rows {
                let n = |i: usize| row[i].as_deref().map(str_to_int64).unwrap_or(0);
                let first_written = row[8].as_deref()
                    .and_then(|v| chrono::NaiveDateTime::parse_from_str(v, "%Y-%m-%d %H:%M:%S").ok())
                    .and_then(|v| Local.from_local_datetime(&v).single()).map(|v| v.timestamp()).unwrap_or(0);
                let expired = status == "Append" && n(1) > 0
                    && ((n(4) > 0 && n(3) >= n(4)) || (n(5) > 0 && n(5) <= n(1)) || (n(6) > 0 && n(6) <= n(2))
                        || (n(7) > 0 && n(7) <= now - first_written));
                if !expired {
                    return row[0].clone();
                }
            }
        }
        if !in_changer {
            return None;
        }
        in_changer = false;
    }
}

/// RunResource::NextScheduleTime(start, ndays).has_value(): a day within
/// `ndays` the run triggers on (DateTimeMask::TriggersOnDay) with a
/// scheduled hour (on the first day: from the current hour on).
fn triggers_within(run: &super::show::RunSpec, start: i64, ndays: i64) -> bool {
    let mut time = start;
    for d in 0..=ndays {
        if let Some(tm) = Local.timestamp_opt(time, 0).single() {
            if triggers_on_day(run, &tm) {
                let first = if d == 0 { tm.hour() as usize } else { 0 };
                if run.hour[first..].iter().any(|h| *h) {
                    return true;
                }
            }
        }
        time += 24 * 60 * 60;
    }
    false
}

/// dird/date_time_mask.cc TriggersOnDay with dird/date_time.cc DateTime.
fn triggers_on_day(run: &super::show::RunSpec, tm: &chrono::DateTime<Local>) -> bool {
    let mday = tm.day0() as usize;
    let yday = tm.ordinal0() as i64;
    // LastDayOfMonth uses tm_year (years since 1900) for the leap year test
    let year = tm.year() as i64 - 1900;
    let leap = year % 400 == 0 || (year % 100 != 0 && year % 4 == 0);
    let last_days: [i64; 12] = if leap {
        [30, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334, 365]
    } else {
        [30, 58, 89, 119, 150, 180, 211, 242, 272, 303, 333, 364]
    };
    let last_7days = last_days[tm.month0() as usize] - 7 < yday;
    run.mday[mday]
        && run.wday[tm.weekday().num_days_from_sunday() as usize]
        && run.month[tm.month0() as usize]
        && (run.wom.get(mday / 7).copied().unwrap_or(false) || (last_7days && run.last_7days))
        && run.woy.get(week_of_year(tm)).copied().unwrap_or(false)
}

/// lib/btime.cc TmWoy: ISO-like week of the year (week of 4 January = 1).
fn week_of_year(tm: &chrono::DateTime<Local>) -> usize {
    let yday = tm.ordinal0() as i64;
    let jan4 = chrono::NaiveDate::from_ymd_opt(tm.year(), 1, 4).map(|d| d.weekday().num_days_from_sunday() as i64)
        .unwrap_or(0);
    let mut fty = 1 - jan4;
    if fty <= 0 {
        fty += 7;
    }
    let woy = yday - fty + 4;
    if woy < 0 { 0 } else { (1 + woy / 7) as usize }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_number_helpers() {
        assert_eq!(str_to_int64(" 12abc"), 12);
        assert_eq!(str_to_int64("-3"), -3);
        assert_eq!(str_to_int64("abc"), 0);
        assert!(is_a_number_list("1,2"));
        assert!(is_a_number_list("1,"));
        assert!(!is_a_number_list(",1"));
        assert!(!is_a_number_list("x"));
        assert_eq!(with_suffix(0), "0 ");
        assert_eq!(with_suffix(999), "999 ");
        assert_eq!(with_suffix(1234), "1.234 K");
        assert_eq!(with_suffix(824021543), "824.0 M");
    }

    #[test]
    fn acl_filter_tri_state() {
        let ctx = crate::acl::tests::context(); // JobACL = !blocked, *all*; ClientACL = *all*
        let mut out = ListOut { ctx: Some(&ctx), ..Default::default() };
        out.acl(0, Kind::Job);
        out.acl(1, Kind::Client); // unrestricted: no tuple (SetAclFilter)
        assert_eq!(out.filters.len(), 1);
        let s = |v: &str| Some(v.to_string());
        let column = |name: &str| crate::catalog_worker::SqlColumn { name: name.into(), numeric: false };
        let mut t = SqlTable { columns: vec![column("job"), column("client")], rows: vec![
            vec![s("ok"), s("c")], vec![s("blocked"), s("c")], vec![None, s("c")], vec![s(""), None],
        ] };
        // an empty ACL column alone is unknown → suppressed
        assert_eq!(out.shown(&t), vec![true, false, false, false]);
        assert_eq!(out.filtered, 3);
        // a second ACL tuple that explicitly allows the row shows it
        out.filters.push(Filter::Acl(1, Kind::Client));
        t.rows.truncate(3);
        assert_eq!(out.shown(&t), vec![true, false, true]);
    }

    #[test]
    fn hours_wrap_like_c_int() {
        use crate::command_ast::{Argument, CommandAst};
        let ast = |key: &str, value: &str| CommandAst {
            command_name: "list".into(),
            arguments: vec![Argument { key: "jobs".into(), value: None }, Argument { key: key.into(), value: Some(value.into()) }],
            ..Default::default()
        };
        assert_eq!(schedtime(&Args(&ast("hours", "1")), 10_000), 6_400);
        // 3600 * 1000000 overflows a C int: the time lies in the future
        assert_eq!(schedtime(&Args(&ast("hours", "1000000")), 0), 694_967_296);
        assert_eq!(schedtime(&Args(&ast("days", "abc")), 5), 5);
    }

    #[test]
    fn week_of_year_like_tm_woy() {
        let at = |y, m, d| Local.from_local_datetime(&chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
            .and_hms_opt(12, 0, 0).unwrap()).single().unwrap();
        // 4 January 2026 is a Sunday: fty = 1, woy = yday + 3
        assert_eq!(week_of_year(&at(2026, 1, 1)), 1);
        assert_eq!(week_of_year(&at(2026, 10, 15)), 42);
    }

    #[test]
    fn schedule_runs_within_days() {
        use bareos_config::raw::Tok;
        let run = |spec: &str| super::super::show::parse_run(&spec.split(' ').map(|w| Tok::Word(w.into())).collect::<Vec<_>>());
        let at = |y, m, d, h| Local.from_local_datetime(&chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
            .and_hms_opt(h, 0, 0).unwrap()).single().unwrap().timestamp();
        // Thursday 15 October 2026
        assert!(triggers_within(&run("Incremental mon-fri at 21:00"), at(2026, 10, 15, 12), 0));
        assert!(!triggers_within(&run("Incremental mon-fri at 21:00"), at(2026, 10, 15, 22), 0));
        assert!(triggers_within(&run("Incremental mon-fri at 21:00"), at(2026, 10, 15, 22), 1));
        // Saturday 17 October is the 3rd Saturday of the month
        assert!(!triggers_within(&run("Full 1st sat at 21:00"), at(2026, 10, 15, 12), 3));
        assert!(triggers_within(&run("Differential 2nd-5th sat at 21:00"), at(2026, 10, 15, 12), 2));
        assert!(!triggers_within(&run("Differential 2nd-5th sat at 21:00"), at(2026, 10, 15, 12), 1));
    }
}
