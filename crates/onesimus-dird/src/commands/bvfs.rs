//! BVFS — the Bareos virtual file system used by restore browsers.
//!
//! Port of C++ dird/ua_dotcmds.cc (DotBvfs*Cmd, BvfsParseArg,
//! BvfsValidateJobids, BvfsResultHandler, BvfsStat) and cats/bvfs.cc at 25.1.2 (Bvfs,
//! BareosDb::BvfsUpdatePathHierarchyCache, AccurateGetJobids) with the
//! PostgreSQL queries of cats/postgresql_queries.inc. All statements run on
//! one reserved catalog connection, like the C++ private connection
//! (OpenClientDb(ua, true)).

use std::collections::HashSet;

use serde_json::{json, Map, Value};

use crate::acl::{self, Kind};
use crate::catalog_worker::{CatalogConnection, SqlTable};
use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;

// --- catalog access -------------------------------------------------------

/// The statements BVFS needs from its catalog connection.
pub trait Db {
    /// C++ SqlQuery/QueryDb: rows with text values.
    fn rows(&self, sql: &str) -> Result<SqlTable, String>;
    /// C++ UpdateDb/InsertDb/DeleteDb, SqlAffectedRows.
    fn exec(&self, sql: &str) -> Result<u64, String>;
}

impl Db for CatalogConnection {
    fn rows(&self, sql: &str) -> Result<SqlTable, String> {
        self.query_rows(sql)
    }
    fn exec(&self, sql: &str) -> Result<u64, String> {
        self.execute(sql)
    }
}

/// First column of every row (NULL as empty string).
fn column0(db: &dyn Db, sql: &str) -> Result<Vec<String>, String> {
    Ok(db.rows(sql)?.rows.into_iter()
        .map(|r| r.into_iter().next().flatten().unwrap_or_default())
        .collect())
}

/// C++ EscapeString (PQescapeStringConn, standard_conforming_strings on).
fn escape(s: &str) -> String {
    s.replace('\'', "''")
}

// --- C++ lib/edit.cc helpers ----------------------------------------------

fn str_to_uint64(s: &str) -> u64 {
    let s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let s = s.strip_prefix('+').unwrap_or(s);
    s.bytes().take_while(u8::is_ascii_digit)
        .fold(0u64, |v, d| v.wrapping_mul(10).wrapping_add(u64::from(d - b'0')))
}

fn str_to_int64(s: &str) -> i64 {
    let s = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    match s.strip_prefix('-') {
        Some(rest) if !rest.starts_with('+') => (str_to_uint64(rest) as i64).wrapping_neg(),
        Some(_) => 0,
        None => str_to_uint64(s) as i64,
    }
}

/// C++ Is_a_number(): optional sign, digits, optional fraction and exponent.
fn is_a_number(n: &str) -> bool {
    let b = n.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut i = 0;
    if at(i) == b'-' || at(i) == b'+' {
        i += 1;
    }
    let mut digit_seen = false;
    while at(i).is_ascii_digit() {
        digit_seen = true;
        i += 1;
    }
    if digit_seen && at(i) == b'.' {
        i += 1;
        while at(i).is_ascii_digit() {
            i += 1;
        }
    }
    if digit_seen && (at(i) == b'e' || at(i) == b'E')
        && (at(i + 1).is_ascii_digit() || ((at(i + 1) == b'-' || at(i + 1) == b'+') && at(i + 2).is_ascii_digit())) {
        i += 2;
        while at(i).is_ascii_digit() {
            i += 1;
        }
    }
    digit_seen && i == b.len()
}

/// C++ Is_a_number_list(): digits separated by commas (trailing comma ok).
fn is_a_number_list(n: &str) -> bool {
    let mut previous_digit = false;
    let mut digit_seen = false;
    for c in n.bytes() {
        if c.is_ascii_digit() {
            previous_digit = true;
            digit_seen = true;
        } else if c == b',' && previous_digit {
            previous_digit = false;
        } else {
            return false;
        }
    }
    digit_seen
}

/// C++ IsAnInteger().
fn is_an_integer(n: &str) -> bool {
    !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit())
}

/// C++ GetNextJobidFromList()/GetNextIdFromList(): Some(Ok(id)), Some(Err)
/// for a non-number, None at the end of the list.
fn next_id<'a>(p: &mut &'a str) -> Option<Result<i64, ()>> {
    let bytes = p.as_bytes();
    let mut len = 0;
    let mut consumed = 0;
    while len < 30 && consumed < bytes.len() {
        if bytes[consumed] == b',' {
            consumed += 1;
            break;
        }
        consumed += 1;
        len += 1;
    }
    let id = &p[..len];
    if id.is_empty() {
        return None;
    }
    if !is_a_number(id) {
        return Some(Err(()));
    }
    *p = &p[consumed..];
    Some(Ok(str_to_int64(id)))
}

// --- C++ ua_select.cc argument helpers --------------------------------------

/// C++ FindArg(): index of the keyword (case-insensitive).
fn find_arg(ast: &CommandAst, key: &str) -> Option<usize> {
    ast.arguments.iter().position(|a| a.key.eq_ignore_ascii_case(key))
}

/// C++ FindArgWithValue(): the first matching keyword must carry a value.
fn find_arg_with_value<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(key)).and_then(|a| a.value.as_deref())
}

// --- path helpers (cats/bvfs.cc) ----------------------------------------------

/// C++ bvfs_parent_dir(): parent directory with trailing "/".
pub fn parent_dir(path: &str) -> String {
    let mut p: Vec<u8> = path.as_bytes().to_vec();
    let mut len = p.len() as isize - 1;
    if len == 2 && p[0].is_ascii_alphabetic() && p[1] == b':' && p[2] == b'/' {
        len = 0;
        p.truncate(0);
    }
    if len >= 0 && p.get(len as usize) == Some(&b'/') {
        p.truncate(len as usize);
    }
    if len > 0 {
        let mut i = len as usize;
        let at = |p: &Vec<u8>, i: usize| p.get(i).copied().unwrap_or(0);
        while i > 0 && at(&p, i) != b'/' {
            i -= 1;
        }
        if at(&p, i) == b'/' {
            p.truncate(i + 1);
        } else {
            p.truncate(i);
        }
    }
    String::from_utf8_lossy(&p).into_owned()
}

/// C++ bvfs_basename_dir(): last component, with its trailing "/".
pub fn basename_dir(path: &str) -> &str {
    let b = path.as_bytes();
    let mut len = b.len() as isize - 1;
    if len >= 0 && b[len as usize] == b'/' {
        len -= 1;
    }
    let mut i = 0usize;
    if len > 0 {
        i = len as usize;
        while i > 0 && b[i] != b'/' {
            i -= 1;
        }
        if b[i] == b'/' {
            i += 1;
        }
    }
    &path[i..]
}

// --- lstat decoding (lib/attribs.cc DecodeStat, lib/base64.cc) -------------

fn base64_value(c: u8) -> u64 {
    match c {
        b'A'..=b'Z' => u64::from(c - b'A'),
        b'a'..=b'z' => u64::from(c - b'a') + 26,
        b'0'..=b'9' => u64::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 0,
    }
}

/// C++ FromBase64(): value at `*i`, advancing over it.
fn from_base64(b: &[u8], i: &mut usize) -> i64 {
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut neg = false;
    if at(*i) == b'-' {
        *i += 1;
        neg = true;
    }
    let mut val: u64 = 0;
    while at(*i) != 0 && at(*i) != b' ' {
        val = (val << 6) + base64_value(at(*i));
        *i += 1;
    }
    if neg { (val as i64).wrapping_neg() } else { val as i64 }
}

/// Decoded stat fields as the C++ JSON output sees them (struct stat member
/// types, widened to uint64_t and printed as json_int_t), plus LinkFI.
#[derive(Debug, Default, PartialEq)]
pub struct Stat {
    pub dev: i64,
    pub ino: i64,
    pub mode: i64,
    pub nlink: i64,
    pub uid: u32,
    pub gid: u32,
    pub rdev: i64,
    pub size: i64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub link_fi: i64,
}

pub fn decode_stat(lstat: &str) -> Stat {
    let b = lstat.as_bytes();
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let mut i = 0;
    let mut v = [0i64; 13];
    for (n, value) in v.iter_mut().enumerate() {
        *value = from_base64(b, &mut i);
        if n < 12 {
            i += 1;
        }
    }
    let mut link_fi = 0;
    if at(i) == b' ' || (at(i) != 0 && at(i + 1) == b' ') {
        i += 1;
        link_fi = i64::from(from_base64(b, &mut i) as u32 as i32);
    }
    Stat {
        dev: v[0],
        ino: v[1],
        mode: i64::from(v[2] as u32),
        nlink: v[3],
        uid: v[4] as u32,
        gid: v[5] as u32,
        rdev: v[6],
        size: v[7],
        atime: v[10],
        mtime: v[11],
        ctime: v[12],
        link_fi,
    }
}

/// C++ guid_list uid_to_name()/gid_to_name(): name or the number.
pub(crate) fn uid_name(uid: u32) -> String {
    nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).ok().flatten()
        .map(|u| u.name).unwrap_or_else(|| uid.to_string())
}

pub(crate) fn gid_name(gid: u32) -> String {
    nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid)).ok().flatten()
        .map(|g| g.name).unwrap_or_else(|| gid.to_string())
}

/// C++ BvfsStat(): the "stat" object (JSON only).
fn stat_json(stat: &Stat) -> Value {
    json!({
        "dev": stat.dev, "ino": stat.ino, "mode": stat.mode, "nlink": stat.nlink,
        "uid": stat.uid, "gid": stat.gid, "user": uid_name(stat.uid), "group": gid_name(stat.gid),
        "rdev": stat.rdev, "size": stat.size, "atime": stat.atime, "mtime": stat.mtime, "ctime": stat.ctime,
    })
}

// --- output -------------------------------------------------------------------

/// Text lines and JSON array items of one command (ua->send).
struct Output {
    json: bool,
    text: String,
    items: Vec<Value>,
}

impl Output {
    fn new(ctx: &ExecContext) -> Self {
        Self { json: ctx.api_mode == ApiMode::Json, text: String::new(), items: Vec::new() }
    }

    /// C++ BvfsResultHandler(): one row of ls_dirs/ls_files/versions.
    fn row(&mut self, row: &[Option<String>]) {
        let get = |i: usize| row.get(i).cloned().flatten();
        let kind = get(0).unwrap_or_default();
        let mut fileid = get(5).unwrap_or_default();
        let mut lstat = get(4).unwrap_or_default();
        let mut jobid = get(3).unwrap_or_default();
        // We need to deal with non existent path
        if !is_a_number(&fileid) {
            lstat = "A A A A A A A A A A A A A A".into();
            jobid = "0".into();
            fileid = "0".into();
        }
        let pathid = str_to_uint64(&get(1).unwrap_or_default());
        let (fileid, jobid) = (str_to_uint64(&fileid), str_to_uint64(&jobid));
        let name = get(2).unwrap_or_default();
        let stat = decode_stat(&lstat);
        let mut obj = Map::new();
        obj.insert("type".into(), json!(kind));
        obj.insert("pathid".into(), json!(pathid as i64));
        obj.insert("fileid".into(), json!(fileid as i64));
        obj.insert("jobid".into(), json!(jobid as i64));
        obj.insert("lstat".into(), json!(lstat));
        match kind.as_bytes().first() {
            Some(b'D') => {
                let base = basename_dir(&name);
                self.text.push_str(&format!("{pathid}\t{fileid}\t{jobid}\t{lstat}\t{base}\n"));
                obj.insert("name".into(), json!(base));
                obj.insert("fullpath".into(), json!(name));
                obj.insert("stat".into(), stat_json(&stat));
                obj.insert("linkfileindex".into(), json!(stat.link_fi));
            }
            Some(b'V') => {
                let md5 = get(6).unwrap_or_default();
                let volume = get(7).unwrap_or_default();
                let inchanger = str_to_uint64(&get(8).unwrap_or_default());
                self.text.push_str(&format!("{pathid}\t{fileid}\t{jobid}\t{lstat}\t{md5}\t{volume}\t{inchanger}\n"));
                obj.insert("md5".into(), json!(md5));
                obj.insert("volumename".into(), json!(volume));
                obj.insert("volumeinchanger".into(), json!(inchanger as i64));
                obj.insert("stat".into(), stat_json(&stat));
            }
            Some(b'F') => {
                self.text.push_str(&format!("{pathid}\t{fileid}\t{jobid}\t{lstat}\t{name}\n"));
                obj.insert("name".into(), json!(name));
                obj.insert("stat".into(), stat_json(&stat));
                obj.insert("linkfileindex".into(), json!(stat.link_fi));
            }
            _ => return,
        }
        if self.json {
            self.items.push(Value::Object(obj));
        }
    }

    fn result(self, prefix: String, array: &str) -> CommandResult {
        CommandResult::ok(prefix + &self.text).with_json(json!({ array: self.items }))
    }
}

/// Result with the success flag of the C++ command's return value.
fn finish(text: String, error: String, success: bool) -> CommandResult {
    CommandResult { success, text_output: text, error_message: error, ..Default::default() }
}

const ERR_ARGS: &str = "Can't find jobid, pathid or path argument\n";
const ERR_UNAUTHORIZED: &str = "Unauthorized command from this console.\n";

/// Reserve the catalog connection after OpenClientDb succeeded.
fn connection(ctx: &ExecContext) -> Option<CatalogConnection> {
    ctx.catalog.as_ref()?.dedicated().ok()
}

// --- argument parsing -----------------------------------------------------------

/// C++ BvfsParseArg() results.
struct Args {
    pathid: i64,
    path: Option<String>,
    jobid: Option<String>,
    limit: i32,
    offset: i32,
}

/// C++ BvfsParseArg() without the OpenClientDb() at its end.
fn parse_args(ast: &CommandAst) -> Option<Args> {
    let mut args = Args { pathid: 0, path: None, jobid: None, limit: 2000, offset: 0 };
    for a in &ast.arguments {
        let value = a.value.as_deref();
        let key = a.key.as_str();
        if key.eq_ignore_ascii_case("pathid") {
            if let Some(v) = value.filter(|v| is_a_number(v)) {
                args.pathid = str_to_int64(v);
            }
        }
        if key.eq_ignore_ascii_case("path") {
            args.path = value.map(str::to_string);
        }
        if key.eq_ignore_ascii_case("jobid") {
            if let Some(v) = value.filter(|v| is_a_number_list(v)) {
                args.jobid = Some(v.to_string());
            }
        }
        if key.eq_ignore_ascii_case("limit") {
            if let Some(v) = value.filter(|v| is_a_number(v)) {
                args.limit = str_to_int64(v) as i32;
            }
        }
        if key.eq_ignore_ascii_case("offset") {
            if let Some(v) = value.filter(|v| is_a_number(v)) {
                args.offset = str_to_int64(v) as i32;
            }
        }
    }
    ((args.pathid != 0 || args.path.is_some()) && args.jobid.is_some()).then_some(args)
}

/// C++ BvfsParseArg() incl. OpenClientDb(ua, true): Ok((args, prefix)) or
/// the messages up to the "Can't find ..." error.
fn parse_arg(ast: &CommandAst, ctx: &ExecContext) -> Result<(Args, String), (String, String)> {
    let Some(args) = parse_args(ast) else { return Err((String::new(), ERR_ARGS.into())) };
    match crate::ua_db::open_client_db(ast, ctx, true) {
        Ok(prefix) => Ok((args, prefix)),
        Err(r) => Err((r.text_output, r.error_message + ERR_ARGS)),
    }
}

/// C++ BvfsValidateJobid(): Job ACL on the job name, Client ACL on its client.
fn validate_jobid(db: &dyn Db, ctx: &ExecContext, jobid: &str, audit: bool) -> bool {
    // AclAccessOk(…, audit_event): denials go to the audit log when set.
    let allowed = |kind, name: &str| if audit { acl::access_audited(ctx, kind, name) } else { acl::access(ctx, kind, name) };
    let id = str_to_int64(jobid);
    let sql = if id == 0 {
        "SELECT Name, ClientId FROM Job WHERE Job=''".to_string()
    } else {
        format!("SELECT Name, ClientId FROM Job WHERE JobId={id}")
    };
    let Ok(table) = db.rows(&sql) else { return false };
    let Some(row) = table.rows.first() else { return false };
    let name = row.first().cloned().flatten().unwrap_or_default();
    if !allowed(Kind::Job, &name) {
        return false;
    }
    let client_id = str_to_int64(&row.get(1).cloned().flatten().unwrap_or_default());
    if client_id != 0 {
        if let Ok(names) = column0(db, &format!("SELECT Name FROM Client WHERE Client.ClientId={client_id}")) {
            if let Some(client) = names.first() {
                if !allowed(Kind::Client, client) {
                    return false;
                }
            }
        }
    }
    true
}

/// C++ BvfsValidateJobids(): the permitted jobids of the list ("" if none).
fn validate_jobids(db: &dyn Db, ctx: &ExecContext, jobids: &str, audit: bool) -> String {
    jobids.split(',').take_while(|id| !id.is_empty())
        .filter(|id| validate_jobid(db, ctx, id, audit))
        .collect::<Vec<_>>()
        .join(",")
}

// --- cats/bvfs.cc -----------------------------------------------------------------

/// C++ GetPathRecord(): PathId of a path, 0 if unknown.
fn path_id(db: &dyn Db, path: &str) -> i64 {
    column0(db, &format!("SELECT PathId FROM Path WHERE Path='{}'", escape(path)))
        .ok().and_then(|ids| ids.first().map(|id| str_to_int64(id))).unwrap_or(0)
}

/// C++ BareosDb::BuildPathHierarchy().
fn build_path_hierarchy(db: &dyn Db, cache: &mut HashSet<i64>, mut pathid: i64, path: &str) {
    let mut path = path.to_string();
    while !path.is_empty() {
        if cache.contains(&pathid) {
            return;
        }
        let Ok(found) = db.rows(&format!("SELECT PPathId FROM PathHierarchy WHERE PathId = {pathid}")) else { return };
        if !found.rows.is_empty() {
            cache.insert(pathid);
            return;
        }
        // Search or create parent PathId in Path table
        path = parent_dir(&path);
        let Some(parent) = create_path_record(db, &path) else { return };
        cache.insert(pathid);
        if db.exec(&format!("INSERT INTO PathHierarchy (PathId, PPathId) VALUES ({pathid},{parent})")).is_err() {
            return;
        }
        pathid = parent;
    }
}

/// C++ BareosDb::CreatePathRecord().
fn create_path_record(db: &dyn Db, path: &str) -> Option<i64> {
    let escaped = escape(path);
    if let Ok(ids) = column0(db, &format!("SELECT PathId FROM Path WHERE Path='{escaped}'")) {
        if let Some(id) = ids.first() {
            return Some(str_to_int64(id));
        }
    }
    // SqlInsertAutokeyRecord(): INSERT, then the new key.
    let ids = column0(db, &format!("INSERT INTO Path (Path) VALUES ('{escaped}') RETURNING PathId")).ok()?;
    ids.first().map(|id| str_to_int64(id)).filter(|id| *id != 0)
}

/// C++ BareosDb::UpdatePathHierarchyCache() for one job.
fn update_path_hierarchy_cache(db: &dyn Db, cache: &mut HashSet<i64>, jobid: i64) -> bool {
    // StartTransaction()/EndTransaction() of the private connection
    // (mult_db_connections: transactions allowed).
    let mut transaction = db.exec("BEGIN").is_ok();
    let end = |transaction: &mut bool| {
        if *transaction {
            let _ = db.exec("COMMIT");
            *transaction = false;
        }
    };
    let retval = 'bail: {
        // Claim the cache build atomically.
        let Ok(updated) = db.exec(&format!("UPDATE Job SET HasCache=-1 WHERE JobId={jobid} AND HasCache=0")) else { break 'bail false };
        if updated == 0 {
            let Ok(done) = db.rows(&format!("SELECT 1 FROM Job WHERE JobId = {jobid} AND HasCache=1")) else { break 'bail false };
            break 'bail !done.rows.is_empty();
        }
        end(&mut transaction);

        // Inserting path records for JobId
        if db.exec(&format!(
            "INSERT INTO PathVisibility (PathId, JobId) SELECT DISTINCT PathId, JobId FROM (SELECT PathId, JobId FROM File WHERE JobId = {jobid} UNION SELECT PathId, BaseFiles.JobId FROM BaseFiles JOIN File AS F USING (FileId) WHERE BaseFiles.JobId = {jobid}) AS B"
        )).is_err() {
            break 'bail false;
        }

        // Directory recursion for the not yet hierarchised directories.
        let Ok(paths) = db.rows(&format!(
            "SELECT PathVisibility.PathId, Path FROM PathVisibility JOIN Path ON (PathVisibility.PathId = Path.PathId) LEFT JOIN PathHierarchy ON (PathVisibility.PathId = PathHierarchy.PathId) WHERE PathVisibility.JobId = {jobid} AND PathHierarchy.PathId IS NULL ORDER BY Path"
        )) else { break 'bail false };
        if !paths.rows.is_empty() {
            if db.exec(BVFS_LOCK_PATHHIERARCHY).is_err() {
                break 'bail false;
            }
            for row in &paths.rows {
                let id = str_to_int64(&row.first().cloned().flatten().unwrap_or_default());
                let path = row.get(1).cloned().flatten().unwrap_or_default();
                build_path_hierarchy(db, cache, id, &path);
            }
            if db.exec("COMMIT").is_err() {
                break 'bail false;
            }
        }

        transaction = db.exec("BEGIN").is_ok();
        let visibility = format!(
            "INSERT INTO PathVisibility (PathId, JobId)\nSELECT a.PathId,{jobid}\nFROM (\n   SELECT DISTINCT h.PPathId AS PathId\n   FROM PathHierarchy AS h\n   JOIN PathVisibility AS p ON (h.PathId=p.PathId)\n   WHERE p.JobId={jobid}\n) AS a\nLEFT JOIN PathVisibility AS b\nON (b.JobId={jobid} AND a.PathId = b.PathId)\nWHERE b.PathId IS NULL\n"
        );
        let mut retval;
        loop {
            match db.exec(&visibility) {
                Ok(n) => {
                    retval = true;
                    if n == 0 {
                        break;
                    }
                }
                Err(_) => {
                    retval = false;
                    break;
                }
            }
        }
        let _ = db.exec(&format!("UPDATE Job SET HasCache=1 WHERE JobId={jobid}"));
        retval
    };
    end(&mut transaction);
    retval
}

const BVFS_LOCK_PATHHIERARCHY: &str = "BEGIN; LOCK TABLE PathHierarchy IN SHARE ROW EXCLUSIVE MODE\n";

/// C++ BareosDb::BvfsUpdatePathHierarchyCache(): jobid list "1,2,3".
fn update_cache_for(db: &dyn Db, jobids: &str) -> bool {
    let mut cache = HashSet::new();
    let mut retval = true;
    let mut p = jobids;
    while let Some(Ok(jobid)) = next_id(&mut p) {
        // JobId_t is 32 bits
        if !update_path_hierarchy_cache(db, &mut cache, i64::from(jobid as u32)) {
            retval = false;
        }
    }
    retval
}

/// C++ BareosDb::BvfsUpdateCache(): all jobs without cache.
fn update_cache_all(db: &dyn Db) {
    let jobids = column0(db, "SELECT JobId from Job WHERE HasCache = 0 AND Type IN ('B','A','a') AND JobStatus IN ('T', 'W', 'f', 'A') ORDER BY JobId")
        .unwrap_or_default();
    update_cache_for(db, &jobids.join(","));
    let transaction = db.exec("BEGIN").is_ok();
    let _ = db.exec("DELETE FROM PathVisibility WHERE NOT EXISTS (SELECT 1 FROM Job WHERE JobId=PathVisibility.JobId)");
    if transaction {
        let _ = db.exec("COMMIT");
    }
}

/// C++ Bvfs::ls_dirs() (bvfs_ls_special_dirs_3, bvfs_ls_sub_dirs_5,
/// bvfs_lsdirs_4) with _handlePath() dropping repeated directories.
fn ls_dirs(db: &dyn Db, out: &mut Output, pathid: i64, jobids: &str, limit: u32, offset: u32) {
    let special = format!(
        "SELECT DISTINCT ON (PathId) 'D', SpecialDir.PathId, SpecialDir.Path, JobId, LStat, FileId\nFROM (\n    SELECT {pathid} AS PathId, '.' AS Path\n    UNION\n    SELECT PPathId AS PathId, '..' AS Path\n    FROM   PathHierarchy\n    WHERE  PathId = {pathid}\n) AS SpecialDir\nLEFT JOIN (\n    SELECT PathId, JobId, LStat, FileId\n    FROM File\n    WHERE File.Name = ''\n      AND File.JobId IN ({jobids})\n    ORDER BY PathId ASC, JobId DESC\n) AS DirAttribute\nON (SpecialDir.PathId = DirAttribute.PathId)\n"
    );
    let filter = "";
    let sub = format!(
        "(\nSELECT DISTINCT ON (Path) 'D', PathId, Path, JobId, LStat, FileId\nFROM (\n   SELECT\n      Path1.PathId AS PathId,\n      Path1.Path AS Path,\n      lower(Path1.Path) AS lpath,\n      listfile1.JobId AS JobId,\n      listfile1.LStat AS LStat,\n      listfile1.FileId AS FileId\n   FROM (\n      SELECT listpath1.PathId AS PathId\n      FROM (\n         SELECT DISTINCT PathHierarchy1.PathId AS PathId\n         FROM PathHierarchy AS PathHierarchy1\n         INNER JOIN Path AS Path2\n         ON (PathHierarchy1.PathId = Path2.PathId)\n         INNER JOIN PathVisibility AS PathVisibility1\n         ON (PathHierarchy1.PathId = PathVisibility1.PathId)\n         WHERE PathHierarchy1.PPathId = {pathid}\n         AND PathVisibility1.JobId IN ({jobids})\n         ) AS listpath1\n      LEFT JOIN (\n          SELECT PVD1.PathId AS PathId\n          FROM (\n             SELECT PV1.PathId AS PathId, MAX(JobId) AS MaxJobId\n             FROM PathVisibility AS PV1 WHERE JobId IN ({jobids}) GROUP BY PathId\n             ) AS PVD1\n             INNER JOIN File AS F2\n             ON (F2.PathId = PVD1.PathId AND F2.JobId = PVD1.MaxJobId AND F2.FileIndex = 0 AND F2.Name = '')\n      ) AS listpath2\n      ON (listpath1.PathId = listpath2.PathId)\n      WHERE listpath2.PathId IS NULL\n      {filter}\n   ) AS listpath3\n   INNER JOIN Path AS Path1 ON (listpath3.PathId = Path1.PathId)\n   LEFT JOIN (\n      SELECT File1.PathId AS PathId, File1.JobId AS JobId, File1.LStat AS LStat, File1.FileId AS FileId\n      FROM File AS File1\n      WHERE File1.Name = ''\n        AND File1.JobId IN ({jobids})\n   ) AS listfile1\n   ON (listpath3.PathId = listfile1.PathId)\n) AS A\nORDER BY Path ASC, JobId DESC\n)\n"
    );
    // bvfs_lsdirs_4 prints limit/offset with %d.
    let query = format!("{special}\nUNION\n{sub}\nORDER BY Path ASC,JobId DESC\nLIMIT {}\nOFFSET {}\n", limit as i32, offset as i32);
    let Ok(table) = db.rows(&query) else { return };
    // The sql query displays same directory multiple time, take the first one
    let mut prev_dir = String::new();
    for row in &table.rows {
        let is_dir = row.first().cloned().flatten().is_some_and(|t| t.starts_with('D'));
        let name = row.get(2).cloned().flatten().unwrap_or_default();
        if is_dir && name != prev_dir {
            prev_dir = name;
            out.row(row);
        }
    }
}

/// C++ Bvfs::ls_files() with bvfs_list_files (PostgreSQL).
fn ls_files(db: &dyn Db, out: &mut Output, pathid: i64, jobids: &str, pattern: Option<&str>, limit: u32, offset: u32) {
    let pathid = if pathid == 0 { path_id(db, "") } else { pathid };
    // match_query2 refers to a table the query does not have (upstream).
    let filter = pattern.map(|p| format!(" AND Filename.Name ~ '{}'\n", escape(p))).unwrap_or_default();
    let query = format!(
        "SELECT TYPE,\n       PathId,\n       Name,\n       JobId,\n       LStat,\n       FileId\nFROM (\n   SELECT DISTINCT ON (Name) 'F' AS TYPE,\n                       PathId,\n                       FileName AS Name,\n                       JobId,\n                       LStat,\n                       FileId,\n                       FileIndex\n   FROM (\n      SELECT FileId,\n             JobId,\n             PathId,\n             Name AS FileName,\n             FileIndex,\n             LStat,\n             MD5\n      FROM File\n      WHERE JobId IN ({jobids})\n        AND PathId = {pathid}\n        UNION ALL\n        SELECT File.FileId,\n               File.JobId,\n               PathId,\n               File.Name AS FileName,\n               File.FileIndex,\n               LStat,\n               MD5\n        FROM BaseFiles\n        JOIN File USING (FileId) WHERE BaseFiles.JobId IN ({jobids})\n        AND File.PathId = {pathid}\n   ) AS T\n   JOIN Job USING (JobId)\n   WHERE\n      FileName != ''\n      {filter}\n   ORDER BY FileName,\n            StartTime DESC\n) AS A\nWHERE A.FileIndex > 0\nLIMIT {limit}\nOFFSET {offset}\n"
    );
    if let Ok(table) = db.rows(&query) {
        for row in &table.rows {
            out.row(row);
        }
    }
}

/// C++ Bvfs::GetAllFileVersions() with bvfs_versions_6.
fn file_versions(db: &dyn Db, out: &mut Output, pathid: i64, fname: &str, client: &str, copies: bool, limit: u32, offset: u32) {
    let filter = if copies { " AND Job.Type IN ('C', 'B', 'A', 'a') " } else { " AND Job.Type IN ('B', 'A', 'a') " };
    let query = format!(
        "SELECT 'V', File.PathId, File.Name, File.JobId, File.LStat, File.FileId, File.Md5, Media.VolumeName, Media.InChanger\nFROM File, Job, Client, JobMedia, Media\nWHERE File.Name = '{}'\n  AND File.PathId = {}\n  AND File.JobId = Job.JobId\n  AND Job.JobId = JobMedia.JobId\n  AND File.FileIndex >= JobMedia.FirstIndex\n  AND File.FileIndex <= JobMedia.LastIndex\n  AND JobMedia.MediaId = Media.MediaId\n  AND Job.ClientId = Client.ClientId\n  AND Client.Name = '{}'\n  {filter}\nORDER BY File.FileId\nLIMIT {}\nOFFSET {}\n",
        escape(fname), pathid as u64, escape(client), limit as i32, offset as i32
    );
    if let Ok(table) = db.rows(&query) {
        for row in &table.rows {
            out.row(row);
        }
    }
}

/// C++ CheckTemp(): restore list tables are named b2<number>.
fn check_temp(table: &str) -> bool {
    table.strip_prefix("b2").is_some_and(is_an_integer)
}

/// C++ Bvfs::DropRestoreList().
fn drop_restore_list(db: &dyn Db, table: &str) -> bool {
    if check_temp(table) {
        let _ = db.exec(&format!("DROP TABLE IF EXISTS {table}"));
        return true;
    }
    false
}

/// C++ Bvfs::compute_restore_list().
fn compute_restore_list(db: &dyn Db, jobids: &str, fileid: &str, dirid: &str, hardlink: &str, output_table: &str) -> bool {
    if (!fileid.is_empty() && !is_a_number_list(fileid))
        || (!dirid.is_empty() && !is_a_number_list(dirid))
        || (!hardlink.is_empty() && !is_a_number_list(hardlink))
        || (hardlink.is_empty() && fileid.is_empty() && dirid.is_empty()) {
        return false;
    }
    if !check_temp(output_table) {
        return false;
    }

    // Cleanup old tables first
    let _ = db.exec(&format!("DROP TABLE IF EXISTS btemp{output_table}"));
    let _ = db.exec(&format!("DROP TABLE IF EXISTS {output_table}"));

    let mut query = format!("CREATE TABLE btemp{output_table} AS ");
    let mut init = false;
    if !fileid.is_empty() {
        init = true;
        query.push_str(&format!("SELECT Job.JobId, JobTDate, FileIndex, File.Name, PathId, FileId FROM File JOIN Job USING (JobId) WHERE FileId IN ({fileid}) "));
    }

    let retval = 'bail: {
        // Add a directory content
        let mut tmp2 = String::new();
        let mut list = dirid;
        while let Some(Ok(id)) = next_id(&mut list) {
            let Ok(paths) = column0(db, &format!("SELECT Path FROM Path WHERE PathId={id}")) else { break 'bail false };
            if let Some(path) = paths.last() {
                tmp2 = path.clone();
            }
            if tmp2.is_empty() {
                break; // path not found
            }
            // escape % and _ for LIKE search
            let mut like = String::new();
            for c in tmp2.chars() {
                if matches!(c, '%' | '_' | '\\') {
                    like.push('\\');
                }
                like.push(c);
            }
            like.push('%');
            // 25.1.2 escapes into tmp2: a later unknown dirid reuses it.
            tmp2 = escape(&like);
            let like = tmp2.clone();
            if init {
                query.push_str(" UNION ");
            }
            query.push_str(&format!("SELECT Job.JobId, JobTDate, File.FileIndex, File.Name, File.PathId, FileId FROM Path JOIN File USING (PathId) JOIN Job USING (JobId) WHERE Path.Path LIKE '{like}' AND File.JobId IN ({jobids}) "));
            init = true;
            query.push_str(" UNION ");
            // A directory can have files from a BaseJob
            query.push_str(&format!("SELECT File.JobId, JobTDate, BaseFiles.FileIndex, File.Name, File.PathId, BaseFiles.FileId FROM BaseFiles JOIN File USING (FileId) JOIN Job ON (BaseFiles.JobId = Job.JobId) JOIN Path USING (PathId) WHERE Path.Path LIKE '{like}' AND BaseFiles.JobId IN ({jobids}) "));
        }

        // expect jobid,fileindex
        let mut prev_jobid = 0;
        let mut tmp = String::new();
        let mut list = hardlink;
        while let Some(Ok(jobid)) = next_id(&mut list) {
            let Some(Ok(id)) = next_id(&mut list) else { break 'bail false };
            if jobid != prev_jobid {
                if prev_jobid == 0 {
                    if init {
                        query.push_str(" UNION ");
                    }
                } else {
                    tmp.push_str(") UNION ");
                    query.push_str(&tmp);
                }
                tmp = format!("SELECT Job.JobId, JobTDate, FileIndex, Name, PathId, FileId FROM File JOIN Job USING (JobId) WHERE JobId = {jobid} AND FileIndex IN ({id}");
                prev_jobid = jobid;
            } else {
                tmp.push_str(&format!(", {id}"));
            }
        }
        if prev_jobid != 0 {
            tmp.push_str(") ");
            query.push_str(&tmp);
        }

        if db.exec(&query).is_err() {
            break 'bail false;
        }
        let select = format!(
            "CREATE TABLE {output_table} AS\n  (SELECT JobId,\n          FileIndex,\n          FileId\n   FROM\n     (SELECT DISTINCT ON (PathId,\n                          Name) JobId,\n                         FileIndex,\n                         FileId\n      FROM btemp{output_table}\n      ORDER BY PathId,\n               Name,\n               JobTDate DESC) AS T\n   WHERE FileIndex > 0)\n"
        );
        db.exec(&select).is_ok()
    };
    let _ = db.exec(&format!("DROP TABLE IF EXISTS btemp{output_table}"));
    retval
}

/// C++ BareosDb::AccurateGetJobids() for the console (jcr->JobId 0):
/// Full + Differential + Incrementals up to `date`.
pub(crate) fn accurate_get_jobids(db: &dyn Db, client_id: i64, job_type: char, date: &str, fileset_id: i64) -> Option<Vec<String>> {
    let jobid = 0;
    let retval = 'bail: {
        let create = format!(
            "CREATE TABLE btemp3{jobid} AS\nSELECT JobId,\n       StartTime,\n       EndTime,\n       JobTDate,\n       PurgedFiles\nFROM Job\nJOIN FileSet USING (FileSetId)\nWHERE ClientId = {client_id}\n  AND Level = 'F'\n  AND JobStatus IN ('T',\n                    'W')\n  AND Type = '{job_type}'\n  AND StartTime < '{date}'\n  AND FileSet.FileSetId = {fileset_id}\nORDER BY Job.JobId DESC,\n         Job.JobTDate DESC\nLIMIT 1\n"
        );
        if db.exec(&create).is_err() {
            break 'bail None;
        }
        // jr->JobLevel is L_INCREMENTAL here.
        let diff = format!("INSERT INTO btemp3{jobid} (JobId, StartTime, EndTime, JobTDate, PurgedFiles) SELECT JobId, StartTime, EndTime, JobTDate, PurgedFiles FROM Job JOIN FileSet USING (FileSetId) WHERE ClientId = {client_id} AND JobFiles > 0 AND Level='D' AND JobStatus IN ('T','W') AND Type='{job_type}' AND StartTime > (SELECT EndTime FROM btemp3{jobid} ORDER BY EndTime DESC LIMIT 1) AND StartTime < '{date}' AND FileSet.FileSet= (SELECT FileSet FROM FileSet WHERE FileSetId = {fileset_id}) ORDER BY Job.JobTDate DESC LIMIT 1 ");
        if db.exec(&diff).is_err() {
            break 'bail None;
        }
        let incr = format!("INSERT INTO btemp3{jobid} (JobId, StartTime, EndTime, JobTDate, PurgedFiles) SELECT JobId, StartTime, EndTime, JobTDate, PurgedFiles FROM Job JOIN FileSet USING (FileSetId) WHERE ClientId = {client_id} AND Level='I' AND JobStatus IN ('T','W') AND Type='{job_type}' AND StartTime > (SELECT EndTime FROM btemp3{jobid} ORDER BY EndTime DESC LIMIT 1) AND StartTime < '{date}' AND FileSet.FileSet= (SELECT FileSet FROM FileSet WHERE FileSetId = {fileset_id}) ORDER BY Job.JobTDate DESC ");
        if db.exec(&incr).is_err() {
            break 'bail None;
        }
        Some(column0(db, &format!("SELECT JobId FROM btemp3{jobid} ORDER by JobTDate ASC")).unwrap_or_default())
    };
    let _ = db.exec(&format!("DROP TABLE IF EXISTS btemp3{jobid}"));
    retval
}

// --- commands ------------------------------------------------------------------------

/// `.bvfs_update [jobid=<list>]`
pub fn cmd_bvfs_update(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let prefix = match crate::ua_db::open_client_db(ast, ctx, true) {
        Ok(prefix) => prefix,
        Err(mut r) => {
            r.success = true;
            return r;
        }
    };
    let Some(db) = connection(ctx) else { return CommandResult::ok(prefix) };
    let mut error = String::new();
    match find_arg_with_value(ast, "jobid").filter(|v| is_a_number_list(v)) {
        Some(jobids) => {
            if !update_cache_for(&db, jobids) {
                error = format!("ERROR: BVFS reported a problem for {jobids}\n");
            }
        }
        None => update_cache_all(&db),
    }
    finish(prefix, error, true)
}

/// `.bvfs_clear_cache yes`
pub fn cmd_bvfs_clear_cache(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let prefix = match crate::ua_db::open_client_db(ast, ctx, true) {
        Ok(prefix) => prefix,
        Err(mut r) => {
            r.success = true;
            return r;
        }
    };
    if find_arg(ast, "yes").is_none() {
        return finish(prefix, "Can't find 'yes' argument\n".into(), true);
    }
    if let Some(db) = connection(ctx) {
        let _ = db.exec("BEGIN;\n   UPDATE Job SET HasCache=0;\n   TRUNCATE PathHierarchy;\n   TRUNCATE PathVisibility;\nCOMMIT;\n");
    }
    finish(prefix, String::new(), true).with_info("OK\n") // InfoMsg
}

/// `.bvfs_cleanup path=b2<number>`
pub fn cmd_bvfs_cleanup(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some(path) = find_arg_with_value(ast, "path") else {
        return CommandResult::error("Can't find path argument\n".into());
    };
    let prefix = match crate::ua_db::open_client_db(ast, ctx, true) {
        Ok(prefix) => prefix,
        Err(r) => return r,
    };
    if let Some(db) = connection(ctx) {
        drop_restore_list(&db, path);
    }
    CommandResult::ok(prefix)
}

/// `.bvfs_restore path=b2<number> jobid=<list> [fileid=<list>] [dirid=<list>] [hardlink=<list>]`
pub fn cmd_bvfs_restore(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let (args, prefix) = match parse_arg(ast, ctx) {
        Ok(parsed) => parsed,
        Err((text, error)) => return finish(text, error, false),
    };
    let Some(db) = connection(ctx) else { return finish(prefix, ERR_UNAUTHORIZED.into(), false) };
    let jobids = validate_jobids(&db, ctx, args.jobid.as_deref().unwrap_or(""), true);
    if jobids.is_empty() {
        return finish(prefix, ERR_UNAUTHORIZED.into(), false);
    }
    let fileid = find_arg_with_value(ast, "fileid").unwrap_or("");
    let dirid = find_arg_with_value(ast, "dirid").unwrap_or("");
    let hardlink = find_arg_with_value(ast, "hardlink").unwrap_or("");
    let output_table = args.path.as_deref().unwrap_or("");
    if compute_restore_list(&db, &jobids, fileid, dirid, hardlink, output_table) {
        finish(prefix + "OK\n", String::new(), true)
    } else {
        finish(prefix, "Can't create restore list\n".into(), true)
    }
}

/// `.bvfs_lsfiles jobid=<list> path=<path> | pathid=<id> [limit=] [offset=] [pattern=]`
pub fn cmd_bvfs_lsfiles(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let (args, prefix) = match parse_arg(ast, ctx) {
        Ok(parsed) => parsed,
        Err((text, error)) => return finish(text, error, false),
    };
    let Some(db) = connection(ctx) else { return finish(prefix, ERR_UNAUTHORIZED.into(), false) };
    let jobids = validate_jobids(&db, ctx, args.jobid.as_deref().unwrap_or(""), true);
    if jobids.is_empty() {
        return finish(prefix, ERR_UNAUTHORIZED.into(), false);
    }
    let pattern = find_arg_with_value(ast, "pattern");
    let pathid = if args.pathid != 0 { args.pathid } else { path_id(&db, args.path.as_deref().unwrap_or("")) };
    let mut out = Output::new(ctx);
    ls_files(&db, &mut out, pathid, &jobids, pattern, args.limit as u32, args.offset as u32);
    out.result(prefix, "files")
}

/// `.bvfs_lsdirs jobid=<list> path=<path> | pathid=<id> [limit=] [offset=]`
pub fn cmd_bvfs_lsdirs(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let (args, prefix) = match parse_arg(ast, ctx) {
        Ok(parsed) => parsed,
        // C++ returns true here (unlike the other BVFS commands).
        Err((text, error)) => return finish(text, error, true),
    };
    let Some(db) = connection(ctx) else { return finish(prefix, ERR_UNAUTHORIZED.into(), false) };
    let jobids = validate_jobids(&db, ctx, args.jobid.as_deref().unwrap_or(""), true);
    if jobids.is_empty() {
        return finish(prefix, ERR_UNAUTHORIZED.into(), false);
    }
    let pathid = if args.pathid != 0 {
        args.pathid
    } else {
        match path_id(&db, args.path.as_deref().unwrap_or("")) {
            // path could not be found. Giving up.
            0 => return finish(prefix, String::new(), false),
            id => id,
        }
    };
    let mut out = Output::new(ctx);
    ls_dirs(&db, &mut out, pathid, &jobids, args.limit as u32, args.offset as u32);
    out.result(prefix, "directories")
}

/// `.bvfs_versions jobid=0 client=<name> pathid=<id> | path=<path> fname=<name> [copies] [versions]`
pub fn cmd_bvfs_versions(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let (args, prefix) = match parse_arg(ast, ctx) {
        Ok(parsed) => parsed,
        Err((text, error)) => return finish(text, error, false),
    };
    // C++ BvfsParseArgVersion(): the last matching keyword wins.
    let mut fname = None;
    let mut client = None;
    let mut copies = false;
    for a in &ast.arguments {
        let key = a.key.as_str();
        if ["name", "fname", "filename"].iter().any(|k| key.eq_ignore_ascii_case(k)) {
            fname = a.value.clone();
        }
        if key.eq_ignore_ascii_case("client") {
            client = a.value.clone();
        }
        if key.eq_ignore_ascii_case("copies") {
            copies = true;
        }
        // "versions" is set, but not used (upstream).
    }
    let (Some(client), Some(fname)) = (client, fname) else {
        return finish(prefix, "Can't find client or fname argument\n".into(), false);
    };
    if !acl::access(ctx, Kind::Client, &client) {
        return finish(prefix, ERR_UNAUTHORIZED.into(), false);
    }
    let mut out = Output::new(ctx);
    if let Some(db) = connection(ctx) {
        // Bvfs::GetAllFileVersions(path) (bvfs.cc:487, 18be77e6) hands the raw
        // path to GetPathRecord(), which escapes it once; fc2c79b6 escaped it
        // twice, so paths containing a quote never matched.
        let pathid = if args.pathid != 0 { args.pathid } else { path_id(&db, args.path.as_deref().unwrap_or("")) };
        file_versions(&db, &mut out, pathid, &fname, &client, copies, args.limit as u32, args.offset as u32);
    }
    out.result(prefix, "versions")
}

/// `.bvfs_get_jobids jobid=<id> | ujobid=<job> [all]`
pub fn cmd_bvfs_get_jobids(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let prefix = match crate::ua_db::open_client_db(ast, ctx, true) {
        Ok(prefix) => prefix,
        Err(mut r) => {
            r.success = true;
            return r;
        }
    };
    let (arg, by_name) = if let Some(job) = find_arg_with_value(ast, "ujobid") {
        (job, Some(job.to_string()))
    } else if let Some(id) = find_arg_with_value(ast, "jobid") {
        let id_value = str_to_int64(id);
        (id, (id_value == 0).then(String::new))
    } else {
        return finish(prefix, "Can't find ujobid or jobid argument\n".into(), false);
    };
    let Some(db) = connection(ctx) else { return CommandResult::ok(prefix) };

    // C++ GetJobRecord()
    let columns = "JobId,StartTime,Type,Level,ClientId,FileSetId";
    let (sql, not_found) = match &by_name {
        Some(job) => (format!("SELECT {columns} FROM Job WHERE Job='{}'", escape(job)),
            format!("cats/sql_get.cc:273 No Job found for JobName '{}'\n", escape(job))),
        None => {
            let id = str_to_int64(arg);
            (format!("SELECT {columns} FROM Job WHERE JobId={id}"),
                format!("cats/sql_get.cc:275 No Job found for JobId {id}\n"))
        }
    };
    let job = db.rows(&sql).ok().and_then(|t| t.rows.into_iter().next());
    let Some(job) = job else {
        return finish(prefix, format!("Unable to get Job record for JobId={arg}: ERR={not_found}\n"), false);
    };
    let field = |i: usize| job.get(i).cloned().flatten();
    let jobid = str_to_int64(&field(0).unwrap_or_default());
    let start_time = field(1).unwrap_or_default();
    let job_type = field(2).and_then(|t| t.chars().next()).unwrap_or('B');
    let level = field(3).and_then(|l| l.chars().next()).unwrap_or(' ');
    let client_id = str_to_int64(&field(4).unwrap_or_default());
    let fileset_id = str_to_int64(&field(5).unwrap_or_default());

    let mut jobids: Vec<String> = Vec::new();
    if level == 'B' {
        // When in level base, we don't rely on any Full/Incr/Diff
        jobids.push(jobid.to_string());
    } else {
        let ids: Vec<i64> = if find_arg(ast, "all").is_some() {
            column0(&db, &format!("SELECT MAX(FileSet.FileSetId)\n  FROM FileSet\n  JOIN Job\n USING (FileSetId)\n WHERE Job.ClientId = {client_id}\n GROUP BY FileSet\n"))
                .unwrap_or_default().iter().map(|id| str_to_uint64(id) as i64).collect()
        } else {
            vec![fileset_id]
        };
        let date = accurate_date(&start_time);
        let job_type = if job_type == 'A' { 'A' } else { 'B' };
        for id in ids {
            // Lookup the FileSet and make sure it is allowed under the ACLs.
            let sql = if id != 0 {
                format!("SELECT FileSet FROM FileSet WHERE FileSetId={id}")
            } else {
                "SELECT FileSet FROM FileSet WHERE FileSet='' ORDER BY CreateTime DESC LIMIT 1".to_string()
            };
            let Some(fileset) = column0(&db, &sql).ok().and_then(|n| n.into_iter().next()) else { continue };
            if !acl::access(ctx, Kind::FileSet, &fileset) {
                continue;
            }
            let Some(found) = accurate_get_jobids(&db, client_id, job_type, &date, id) else {
                return CommandResult::ok(prefix);
            };
            jobids.extend(found);
        }
    }

    let filtered = validate_jobids(&db, ctx, &jobids.join(","), false);
    let items: Vec<Value> = filtered.split(',').take_while(|id| !id.is_empty()).map(|id| json!({ "id": id })).collect();
    CommandResult::ok(format!("{prefix}{filtered}\n")).with_json(json!({ "jobids": items }))
}

/// cats/sql_get.cc AccurateGetJobids(): bstrutime(StartTime + 1), where
/// StartTime is StrToUtime() of the job's catalog StartTime (GetJobRecord)
/// and the current time when that is 0.
pub(crate) fn accurate_date(start_time: &str) -> String {
    let start = match crate::btime::str_to_utime(start_time) {
        0 => (unsafe { libc::time(std::ptr::null_mut()) }) as i64,
        time => time,
    };
    crate::localtime::strftime("%Y-%m-%d %H:%M:%S", (start + 1) as libc::time_t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use crate::catalog_worker::SqlColumn;

    /// Records every statement; answers from a list of (prefix, rows).
    struct Fake {
        log: Mutex<Vec<String>>,
        answers: Vec<(&'static str, Vec<Vec<&'static str>>)>,
        affected: Mutex<Vec<u64>>,
    }

    impl Fake {
        fn new(answers: Vec<(&'static str, Vec<Vec<&'static str>>)>) -> Self {
            Self { log: Mutex::new(Vec::new()), answers, affected: Mutex::new(Vec::new()) }
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl Db for Fake {
        fn rows(&self, sql: &str) -> Result<SqlTable, String> {
            self.log.lock().unwrap().push(sql.to_string());
            let rows = self.answers.iter().find(|(p, _)| sql.starts_with(p)).map(|(_, r)| r.clone()).unwrap_or_default();
            let width = rows.first().map_or(0, Vec::len);
            Ok(SqlTable {
                columns: (0..width).map(|i| SqlColumn { name: format!("c{i}"), numeric: false }).collect(),
                rows: rows.into_iter().map(|r| r.into_iter().map(|v| (v != "NULL").then(|| v.to_string())).collect()).collect(),
            })
        }
        fn exec(&self, sql: &str) -> Result<u64, String> {
            self.log.lock().unwrap().push(sql.to_string());
            if sql.starts_with("INSERT INTO PathVisibility (PathId, JobId)\nSELECT") {
                return Ok(self.affected.lock().unwrap().pop().unwrap_or(0));
            }
            Ok(if sql.starts_with("UPDATE Job SET HasCache=-1") { 1 } else { 0 })
        }
    }

    fn ctx(api_mode: ApiMode) -> ExecContext {
        ExecContext { session_id: 0, config: None, session_mgr: None, catalog: None, api_mode, bus_rx: None, job_manager: None }
    }

    fn ast(args: &[(&str, Option<&str>)]) -> CommandAst {
        CommandAst {
            command_name: ".bvfs_lsdirs".into(),
            arguments: args.iter().map(|(k, v)| crate::command_ast::Argument { key: k.to_string(), value: v.map(str::to_string) }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn parent_and_basename_like_cpp() {
        assert_eq!(parent_dir("/usr/sbin/"), "/usr/");
        assert_eq!(parent_dir("/usr/"), "/");
        assert_eq!(parent_dir("/"), "");
        assert_eq!(parent_dir(""), "");
        assert_eq!(parent_dir("c:/"), "");
        assert_eq!(parent_dir("c:/Programs/"), "c:/");
        assert_eq!(parent_dir("@bpipe@/data.dat"), "@bpipe@/");
        assert_eq!(parent_dir("abc/"), "");
        assert_eq!(basename_dir("/usr/sbin/"), "sbin/");
        assert_eq!(basename_dir("/"), "/");
        assert_eq!(basename_dir("."), ".");
        assert_eq!(basename_dir(".."), "..");
    }

    #[test]
    fn number_checks_like_cpp() {
        assert!(is_a_number_list("1,2,3"));
        assert!(is_a_number_list("1,"));
        assert!(!is_a_number_list(",1"));
        assert!(!is_a_number_list("1,,2"));
        assert!(!is_a_number_list(""));
        assert!(is_a_number("+5") && is_a_number("1.5") && is_a_number("1e3") && !is_a_number("1x"));
        assert_eq!(str_to_int64(" -12x"), -12);
        let mut p = "1,x";
        assert_eq!(next_id(&mut p), Some(Ok(1)));
        assert_eq!(next_id(&mut p), Some(Err(())));
        let mut p = "7,";
        assert_eq!(next_id(&mut p), Some(Ok(7)));
        assert_eq!(next_id(&mut p), None);
    }

    #[test]
    fn decodes_lstat() {
        let stat = decode_stat("d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C");
        assert_eq!(stat, Stat { dev: 29, ino: 1506345, mode: 33261, nlink: 1, uid: 0, gid: 0, rdev: 0,
            size: 110152, atime: 1791097158, mtime: 1785512076, ctime: 1788087228, link_fi: 0 });
        assert_eq!(decode_stat("A A A A A A A A A A A A A A"), Stat::default());
        assert_eq!(decode_stat("A A A A A A A A A A A A A F").link_fi, 5);
    }

    #[test]
    fn parse_args_needs_jobid_and_path() {
        assert!(parse_args(&ast(&[("jobid", Some("1")), ("path", Some(""))])).is_some());
        assert!(parse_args(&ast(&[("jobid", Some("1")), ("pathid", Some("0"))])).is_none());
        assert!(parse_args(&ast(&[("jobid", Some("x")), ("path", Some("/"))])).is_none());
        let args = parse_args(&ast(&[("JOBID", Some("1,2")), ("pathid", Some("4")), ("limit", Some("-1")), ("offset", Some("x"))])).unwrap();
        assert_eq!((args.pathid, args.limit, args.offset, args.jobid.as_deref()), (4, -1, 0, Some("1,2")));
    }

    #[test]
    fn text_and_json_rows() {
        let mut out = Output::new(&ctx(ApiMode::Json));
        let row = |v: &[&str]| v.iter().map(|s| (*s != "NULL").then(|| s.to_string())).collect::<Vec<_>>();
        out.row(&row(&["D", "3", ".", "NULL", "NULL", "NULL"]));
        out.row(&row(&["F", "1", "agetty", "1", "d Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C", "91"]));
        out.row(&row(&["V", "1", "agetty", "1", "A A A A A A A A A A A A A A", "91", "md5", "Vol", "0"]));
        assert_eq!(out.text, "3\t0\t0\tA A A A A A A A A A A A A A\t.\n\
            1\t91\t1\td Fvwp IHt B A A A a5I BAA DY BqwflG BqbMCM BqlAu8 A A C\tagetty\n\
            1\t91\t1\tA A A A A A A A A A A A A A\tmd5\tVol\t0\n");
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>().join(",");
        assert_eq!(keys(&out.items[0]), "type,pathid,fileid,jobid,lstat,name,fullpath,stat,linkfileindex");
        assert_eq!(keys(&out.items[1]), "type,pathid,fileid,jobid,lstat,name,stat,linkfileindex");
        assert_eq!(keys(&out.items[2]), "type,pathid,fileid,jobid,lstat,md5,volumename,volumeinchanger,stat");
        assert_eq!(keys(&out.items[1]["stat"]), "dev,ino,mode,nlink,uid,gid,user,group,rdev,size,atime,mtime,ctime");
        assert_eq!(out.items[1]["stat"]["size"], json!(110152));
    }

    #[test]
    fn update_builds_hierarchy_and_visibility() {
        let db = Fake::new(vec![
            ("SELECT PathVisibility.PathId, Path", vec![vec!["10", "/usr/sbin/"]]),
            ("SELECT PathId FROM Path WHERE Path='/usr/'", vec![]),
            ("INSERT INTO Path (Path) VALUES ('/usr/')", vec![vec!["11"]]),
            ("SELECT PathId FROM Path WHERE Path='/'", vec![vec!["12"]]),
            ("SELECT PPathId FROM PathHierarchy WHERE PathId = 12", vec![vec!["13"]]),
        ]);
        *db.affected.lock().unwrap() = vec![0, 2];
        assert!(update_cache_for(&db, "1,x"));
        let log = db.log();
        let expected = [
            "BEGIN",
            "UPDATE Job SET HasCache=-1 WHERE JobId=1 AND HasCache=0",
            "COMMIT",
            "INSERT INTO PathVisibility (PathId, JobId) SELECT DISTINCT",
            "SELECT PathVisibility.PathId, Path",
            "BEGIN; LOCK TABLE PathHierarchy",
            "SELECT PPathId FROM PathHierarchy WHERE PathId = 10",
            "SELECT PathId FROM Path WHERE Path='/usr/'",
            "INSERT INTO Path (Path) VALUES ('/usr/')",
            "INSERT INTO PathHierarchy (PathId, PPathId) VALUES (10,11)",
            "SELECT PPathId FROM PathHierarchy WHERE PathId = 11",
            "SELECT PathId FROM Path WHERE Path='/'",
            "INSERT INTO PathHierarchy (PathId, PPathId) VALUES (11,12)",
            "SELECT PPathId FROM PathHierarchy WHERE PathId = 12",
            "COMMIT",
            "BEGIN",
            "INSERT INTO PathVisibility (PathId, JobId)\nSELECT a.PathId,1",
            "INSERT INTO PathVisibility (PathId, JobId)\nSELECT a.PathId,1",
            "UPDATE Job SET HasCache=1 WHERE JobId=1",
            "COMMIT",
        ];
        assert_eq!(log.len(), expected.len(), "{log:#?}");
        for (sql, start) in log.iter().zip(expected) {
            assert!(sql.starts_with(start), "{sql} !~ {start}");
        }
    }

    #[test]
    fn update_of_cached_or_unknown_job() {
        struct Cached(Fake);
        impl Db for Cached {
            fn rows(&self, sql: &str) -> Result<SqlTable, String> { self.0.rows(sql) }
            fn exec(&self, sql: &str) -> Result<u64, String> {
                self.0.log.lock().unwrap().push(sql.to_string());
                Ok(0)
            }
        }
        let cached = Cached(Fake::new(vec![("SELECT 1 FROM Job WHERE JobId = 1 AND", vec![vec!["1"]])]));
        assert!(update_cache_for(&cached, "1"));
        assert_eq!(cached.0.log().last().unwrap(), "COMMIT");
        assert!(!update_cache_for(&cached, "2"));
    }

    #[test]
    fn restore_list_queries() {
        let db = Fake::new(vec![("SELECT Path FROM Path WHERE PathId=4", vec![vec!["/tmp/a_b/"]])]);
        assert!(compute_restore_list(&db, "1,2", "7", "4", "1,10,1,11,2,3", "b21"));
        let log = db.log();
        assert_eq!(log[0], "DROP TABLE IF EXISTS btempb21");
        assert_eq!(log[1], "DROP TABLE IF EXISTS b21");
        assert_eq!(log[3], "CREATE TABLE btempb21 AS SELECT Job.JobId, JobTDate, FileIndex, File.Name, PathId, FileId FROM File JOIN Job USING (JobId) WHERE FileId IN (7)  UNION \
SELECT Job.JobId, JobTDate, File.FileIndex, File.Name, File.PathId, FileId FROM Path JOIN File USING (PathId) JOIN Job USING (JobId) WHERE Path.Path LIKE '/tmp/a\\_b/%' AND File.JobId IN (1,2)  UNION \
SELECT File.JobId, JobTDate, BaseFiles.FileIndex, File.Name, File.PathId, BaseFiles.FileId FROM BaseFiles JOIN File USING (FileId) JOIN Job ON (BaseFiles.JobId = Job.JobId) JOIN Path USING (PathId) WHERE Path.Path LIKE '/tmp/a\\_b/%' AND BaseFiles.JobId IN (1,2)  UNION \
SELECT Job.JobId, JobTDate, FileIndex, Name, PathId, FileId FROM File JOIN Job USING (JobId) WHERE JobId = 1 AND FileIndex IN (10, 11) UNION \
SELECT Job.JobId, JobTDate, FileIndex, Name, PathId, FileId FROM File JOIN Job USING (JobId) WHERE JobId = 2 AND FileIndex IN (3) ");
        assert!(log[4].starts_with("CREATE TABLE b21 AS\n"));
        assert_eq!(log[5], "DROP TABLE IF EXISTS btempb21");

        let db = Fake::new(vec![]);
        assert!(!compute_restore_list(&db, "1", "", "", "", "b21"));
        assert!(!compute_restore_list(&db, "1", "1", "", "", "x21"));
        assert!(!compute_restore_list(&db, "1", "", "", "1", "b21"));
        assert!(db.log().iter().all(|s| !s.starts_with("CREATE TABLE b21")));
        assert!(drop_restore_list(&db, "b2123") && !drop_restore_list(&db, "b2x") && !drop_restore_list(&db, "Job"));
        assert_eq!(db.log().last().unwrap(), "DROP TABLE IF EXISTS b2123");
    }

    #[test]
    fn lsdirs_skips_repeated_directories() {
        let db = Fake::new(vec![("SELECT DISTINCT ON (PathId) 'D'", vec![
            vec!["D", "5", ".", "NULL", "NULL", "NULL"],
            vec!["D", "6", "/usr/", "1", "A A A A A A A A A A A A A A", "3"],
            vec!["D", "6", "/usr/", "5", "A A A A A A A A A A A A A A", "4"],
        ])]);
        let mut out = Output::new(&ctx(ApiMode::Human));
        ls_dirs(&db, &mut out, 5, "1,5", 2000, 0);
        assert_eq!(out.text, "5\t0\t0\tA A A A A A A A A A A A A A\t.\n6\t3\t1\tA A A A A A A A A A A A A A\tusr/\n");
        assert!(db.log()[0].ends_with("ORDER BY Path ASC,JobId DESC\nLIMIT 2000\nOFFSET 0\n"));
    }

    #[test]
    fn accurate_jobids_queries() {
        let db = Fake::new(vec![("SELECT JobId FROM btemp30", vec![vec!["600"], vec!["628"]])]);
        assert_eq!(accurate_get_jobids(&db, 1, 'B', "2026-07-10 21:00:03", 1), Some(vec!["600".into(), "628".into()]));
        let log = db.log();
        assert!(log[0].starts_with("CREATE TABLE btemp30 AS\n") && log[0].contains("StartTime < '2026-07-10 21:00:03'"));
        assert!(log[1].contains("Level='D'") && log[2].contains("Level='I'"));
        assert_eq!(log[4], "DROP TABLE IF EXISTS btemp30");
        assert_eq!(accurate_date("2026-07-10 21:00:02"), "2026-07-10 21:00:03");
        assert_eq!(accurate_date("2026-12-31 23:59:59"), "2027-01-01 00:00:00");
        // not a StrToUtime time: the current time
        let now = crate::localtime::strftime_now("%Y-%m-%d");
        assert!(accurate_date("2026-12-31 23:59:59.5").starts_with(&now[..4]));
    }
}
