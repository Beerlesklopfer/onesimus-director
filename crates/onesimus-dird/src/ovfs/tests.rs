//! Tests of the OVFS paths with a catalog test double, plus the equivalence
//! test of fallback and extension against a real catalog (`equivalence`).

use std::cell::RefCell;
use std::collections::VecDeque;

use super::lstat::tests::encode;
use super::tree::{Entry, TreeWriter};
use super::*;
use crate::catalog_worker::SqlColumn;

type Row = Vec<Option<String>>;

/// Answers statements by prefix; `FETCH` takes the next queued batch.
#[derive(Default)]
struct Fake {
    log: RefCell<Vec<String>>,
    answers: Vec<(&'static str, Vec<Row>)>,
    batches: RefCell<VecDeque<Vec<Row>>>,
    fail: Vec<(&'static str, &'static str)>,
}

fn table(rows: Vec<Row>) -> SqlTable {
    let width = rows.first().map_or(0, Vec::len);
    SqlTable { columns: (0..width).map(|i| SqlColumn { name: format!("c{i}"), numeric: false }).collect(), rows }
}

fn row(values: &[&str]) -> Row {
    values.iter().map(|v| (*v != "NULL").then(|| v.to_string())).collect()
}

impl Fake {
    fn new(answers: Vec<(&'static str, Vec<Row>)>) -> Self {
        Fake { answers, ..Default::default() }
    }
    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
    fn failure(&self, sql: &str) -> Option<String> {
        self.fail.iter().find(|(p, _)| sql.starts_with(p)).map(|(_, e)| e.to_string())
    }
}

impl Catalog for Fake {
    fn rows(&self, sql: &str) -> std::result::Result<SqlTable, String> {
        self.log.borrow_mut().push(sql.to_string());
        if let Some(e) = self.failure(sql) {
            return Err(e);
        }
        if sql.starts_with("FETCH") {
            return Ok(table(self.batches.borrow_mut().pop_front().unwrap_or_default()));
        }
        Ok(table(self.answers.iter().find(|(p, _)| sql.starts_with(p)).map(|(_, r)| r.clone()).unwrap_or_default()))
    }
    fn exec(&self, sql: &str) -> std::result::Result<u64, String> {
        self.log.borrow_mut().push(sql.to_string());
        match self.failure(sql) {
            Some(e) => Err(e),
            None => Ok(0),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::from("\\x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// The cursor row of an entry (textsend → bytea text form).
fn cursor_row(e: &Entry) -> Row {
    vec![
        Some(hex(&e.path)), Some(hex(&e.name)), Some(e.jobid.to_string()), Some(e.jobtdate.to_string()),
        Some(e.fileid.to_string()), Some(e.fileindex.to_string()), Some(e.delta_seq.to_string()), Some(e.lstat.clone()),
    ]
}

fn lstat(mode: i64, size: i64, link_fi: i64) -> String {
    encode([0, 0, mode, 1, 0, 0, 0, size, 4096, 0, 0, 1_700_000_000, 0, link_fi, 0, 0])
}

fn entry(jobid: i32, path: &str, name: &str, fileid: i64, fileindex: i32, mode: i64) -> Entry {
    Entry {
        path: path.as_bytes().to_vec(),
        name: name.as_bytes().to_vec(),
        jobid,
        jobtdate: 1000 * i64::from(jobid),
        fileid,
        fileindex,
        delta_seq: 0,
        lstat: lstat(mode, fileid * 10, 0),
    }
}

/// A job chain with directory rows, a deletion, a hard link, a delta file
/// and a non-UTF-8 name, sorted by path and name as the cursor delivers it.
fn chain_rows() -> Vec<Entry> {
    let mut rows = vec![
        entry(10, "/", "", 1, 1, 0o040755),
        entry(10, "/etc/", "", 2, 2, 0o040755),
        entry(11, "/etc/", "", 20, 1, 0o040755),
        entry(10, "/etc/", "caf\u{e9}", 3, 3, 0o100644),
        entry(10, "/etc/", "hosts", 4, 4, 0o100644),
        entry(10, "/etc/", "passwd", 5, 5, 0o100644),
        entry(11, "/etc/", "passwd", 21, 2, 0o100600),
        entry(10, "/etc/ssh/", "", 6, 6, 0o040755),
        entry(10, "/etc/ssh/", "sshd_config", 7, 7, 0o100600),
        entry(11, "/etc/ssh/", "sshd_config", 22, 0, 0o100600),
        entry(10, "/vm/", "disk.img", 8, 8, 0o100644),
        Entry { delta_seq: 1, ..entry(11, "/vm/", "disk.img", 23, 3, 0o100644) },
        Entry { lstat: lstat(0o100644, 1, 4), ..entry(10, "/vm/", "link", 9, 9, 0o100644) },
        Entry { lstat: lstat(0o100644, 1, 4), ..entry(10, "/vm/", "link", 10, 12, 0o100644) },
    ];
    rows[3].name = b"caf\xe9".to_vec();
    rows.sort_by(|a, b| (&a.path, &a.name).cmp(&(&b.path, &b.name)));
    rows
}

fn expected_tree(rows: &[Entry], root: &[u8], depth: usize) -> Option<String> {
    let mut writer = TreeWriter::new(root, depth);
    for e in rows {
        writer.add(e.clone());
    }
    writer.finish()
}

fn streaming_fake(rows: &[Entry], batch: usize, bytewise: &'static str) -> Fake {
    let fake = Fake::new(vec![
        ("SELECT array_agg(id", vec![row(&["NULL"])]),
        ("SELECT datlocprovider", vec![row(&[bytewise])]),
    ]);
    for chunk in rows.chunks(batch) {
        fake.batches.borrow_mut().push_back(chunk.iter().map(cursor_row).collect());
    }
    fake
}

#[test]
fn fallback_tree_streams_through_a_cursor() {
    let rows = chain_rows();
    // batches of 3: the versions of one entry cross batch boundaries
    let fake = streaming_fake(&rows, 3, "t");
    let tree = Ovfs::fallback(&fake).tree(&[10, 11], "/", 9, 100).unwrap();
    assert_eq!(tree, expected_tree(&rows, b"/", 9));
    let text = tree.unwrap();
    // newest JobTDate wins, deletion drops sshd_config, hard link keeps the first copy
    assert!(text.contains(r#"{"name":"passwd","type":"file","jobid":11,"fileid":21,"fileindex":2,"#), "{text}");
    assert!(!text.contains("sshd_config"));
    assert!(text.contains(r#""name":"link","type":"file","jobid":10,"fileid":9,"fileindex":9,"#));
    assert!(text.contains(r#""link_fileindex":4"#));
    assert!(text.contains(r#""delta_seq":1,"delta_parts":[{"jobid":10,"fileindex":8}]"#));
    assert!(text.contains(r#""name_b64":"Y2Fm6Q==""#));

    let log = fake.log();
    let declare = log.iter().find(|s| s.starts_with("DECLARE ovfs_fallback_rows NO SCROLL CURSOR FOR SELECT textsend(p.path)")).unwrap();
    assert!(declare.contains("f.jobid = ANY('{10,11}'::int[]) AND p.path >= '/' AND p.path < '0'"), "{declare}");
    assert!(declare.contains("FROM basefiles b JOIN file f ON f.fileid = b.fileid"));
    assert!(declare.ends_with("ORDER BY path_key, name_key"));
    let fetches = log.iter().filter(|s| *s == "FETCH FORWARD 10000 FROM ovfs_fallback_rows").count();
    assert_eq!(fetches, rows.len().div_ceil(3) + 1);
    let tail: Vec<&str> = log.iter().rev().take(2).map(String::as_str).collect();
    assert_eq!(tail, ["COMMIT", "CLOSE ovfs_fallback_rows"]);
    assert!(log.contains(&"BEGIN READ ONLY".to_string()));
}

#[test]
fn fallback_tree_depth_and_root() {
    let rows = chain_rows();
    for (root, depth) in [("/", 0), ("/", 1), ("/etc", 1), ("/etc/", 2), ("/vm/", 5), ("/nothing/", 1)] {
        let fake = streaming_fake(&rows, 4, "t");
        let tree = Ovfs::fallback(&fake).tree(&[10, 11], root, depth, 100).unwrap();
        let root_bytes = normalize_dir(root).unwrap();
        let in_root: Vec<Entry> = rows.iter().filter(|e| e.path.starts_with(&root_bytes)).cloned().collect();
        assert_eq!(tree, expected_tree(&in_root, &root_bytes, depth as usize), "{root} {depth}");
    }
    let fake = streaming_fake(&rows, 4, "t");
    let depth1 = Ovfs::fallback(&fake).tree(&[10, 11], "/", 1, 100).unwrap().unwrap();
    // depth 1: directories at the limit carry entries instead of children
    assert!(depth1.contains(r#""path":"/etc/","type":"directory","jobid":11,"fileid":20,"fileindex":1,"mode":16877"#), "{depth1}");
    assert!(depth1.contains(r#""entries":4}"#), "{depth1}");
}

#[test]
fn sql_ascii_catalog_reads_raw_bytes() {
    let rows = chain_rows();
    let mut fake = streaming_fake(&rows, 100, "t");
    fake.answers.push(("SHOW server_encoding", vec![row(&["SQL_ASCII"])]));
    Ovfs::fallback(&fake).tree(&[10, 11], "/", 1, 100).unwrap();
    let log = fake.log();
    let begin = log.iter().position(|s| s == "BEGIN READ ONLY").unwrap();
    assert_eq!(log[begin + 1], "SET LOCAL client_encoding TO 'SQL_ASCII'");
    assert!(log[begin + 2].starts_with("DECLARE"));
}

#[test]
fn fallback_tree_limit_rolls_back() {
    let rows = chain_rows();
    let fake = streaming_fake(&rows, 100, "t");
    let err = Ovfs::fallback(&fake).tree(&[10, 11], "/", 9, 3).unwrap_err();
    assert_eq!(err.0, "the tree has more than 3 entries; choose a smaller depth or a deeper root");
    let log = fake.log();
    assert_eq!(log.last().unwrap(), "ROLLBACK");
    assert!(!log.contains(&"COMMIT".to_string()));
}

#[test]
fn fallback_other_collation_sorts_bytes() {
    let rows = chain_rows();
    let fake = streaming_fake(&rows, 100, "f");
    Ovfs::fallback(&fake).tree(&[10], "/etc", 1, 100).unwrap();
    let declare = fake.log().into_iter().find(|s| s.starts_with("DECLARE")).unwrap();
    assert!(declare.contains("starts_with(p.path, '/etc/')"), "{declare}");
    assert!(declare.contains("textsend(p.path) AS path_key, textsend(f.name) AS name_key"));
}

#[test]
fn fallback_job_list_errors() {
    let fake = Fake::new(vec![("SELECT array_agg(id", vec![row(&["{7,8}"])])]);
    assert_eq!(Ovfs::fallback(&fake).tree(&[7, 8, 1], "/", 1, 10).unwrap_err().0, "jobs [7, 8] do not exist");
    let fake = Fake::new(vec![("SELECT array_agg(id", vec![row(&["{7}"])])]);
    assert_eq!(Ovfs::fallback(&fake).ls(&[7], "/").unwrap_err().0, "job 7 does not exist");
    assert!(!fake.log().iter().any(|s| s.starts_with("BEGIN")));
    let fake = Fake::new(vec![]);
    assert_eq!(Ovfs::fallback(&fake).tree(&[], "/", 1, 10).unwrap_err().0, "jobids must not be empty");
    assert_eq!(Ovfs::fallback(&fake).tree(&[1], "", 1, 10).unwrap_err().0, "invalid directory: \"\"");
    assert!(fake.log().is_empty());
}

#[test]
fn fallback_cursor_error_is_reported_and_rolled_back() {
    let rows = chain_rows();
    let mut fake = streaming_fake(&rows, 100, "t");
    fake.fail = vec![("FETCH", "Query: error returned from database: permission denied for table file")];
    let err = Ovfs::fallback(&fake).tree(&[10], "/", 1, 10).unwrap_err();
    assert_eq!(err.0, "catalog query failed: permission denied for table file");
    assert_eq!(fake.log().last().unwrap(), "ROLLBACK");
}

#[test]
fn fallback_ls_lists_children_like_the_extension() {
    let rows = chain_rows();
    let fake = streaming_fake(&rows, 5, "t");
    let listed = Ovfs::fallback(&fake).ls(&[10, 11], "/etc").unwrap();
    let names: Vec<&str> = listed.iter().map(|r| r.name.as_str()).collect();
    // files first (by name bytes), then directories; deleted entries are gone
    assert_eq!(names, ["caf\u{fffd}", "hosts", "passwd", "ssh"]);
    assert_eq!(listed[0].name_b64.as_deref(), Some("Y2Fm6Q=="));
    assert_eq!((listed[2].jobid, listed[2].fileindex, listed[2].mode), (Some(11), Some(2), Some(0o100600)));
    let ssh = &listed[3];
    assert_eq!((ssh.kind.as_str(), ssh.path.as_deref(), ssh.entries, ssh.jobid), ("directory", Some("/etc/ssh/"), Some(0), Some(10)));
    assert_eq!(ssh.size, Some(60));

    let fake = streaming_fake(&rows, 5, "t");
    let vm = Ovfs::fallback(&fake).ls(&[10, 11], "/vm/").unwrap();
    assert_eq!(vm[0].delta_seq, Some(1));
    assert_eq!(vm[0].delta_parts, Some(vec![Part { jobid: 10, fileindex: 8 }]));
    assert_eq!((vm[1].fileindex, vm[1].link_fileindex), (Some(9), Some(4)));
    assert_eq!(vm[1].delta_parts, None);

    let fake = streaming_fake(&rows, 5, "t");
    let top = Ovfs::fallback(&fake).ls(&[10, 11], "/").unwrap();
    let summary: Vec<(&str, Option<i64>, Option<i32>)> = top.iter().map(|r| (r.name.as_str(), r.entries, r.jobid)).collect();
    assert_eq!(summary, [("etc", Some(4), Some(11)), ("vm", Some(2), None)]);
}

fn latest_fake(full: &str, on_media: &str, chain: &str, filesets: &str) -> Fake {
    Fake::new(vec![
        ("SELECT (SELECT clientid FROM client WHERE name = 'fd')", vec![row(&["3"])]),
        ("SELECT (SELECT clientid FROM client", vec![row(&["NULL"])]),
        ("SELECT array_to_json(array_agg(DISTINCT fs.fileset", vec![row(&[filesets])]),
        ("WITH candidate AS", vec![row(&[full, on_media, chain])]),
    ])
}

#[test]
fn fallback_latest_chain_like_restore_option_5() {
    let fake = latest_fake("20", "t", "{20,22,23}", "[\"LinuxAll\"]");
    let latest = Ovfs::fallback(&fake).jobids_latest("fd", None).unwrap();
    assert_eq!(latest, Latest { jobids: vec![20, 22, 23], warning: None });
    let chain_sql = fake.log().into_iter().find(|s| s.starts_with("WITH candidate")).unwrap();
    assert!(chain_sql.contains("j.clientid = 3 AND fs.fileset = 'LinuxAll' AND j.type = 'B'"), "{chain_sql}");
    assert!(chain_sql.contains("j.jobstatus IN ('T', 'W') AND j.starttime < localtimestamp"));
    assert!(chain_sql.contains("WHERE jm.jobid = j.jobid AND m.enabled = 1"));

    // Full on no enabled volume: the chain lacks it, with the WARNING
    let fake = latest_fake("20", "f", "{22,23}", "[\"LinuxAll\"]");
    let latest = Ovfs::fallback(&fake).jobids_latest("fd", Some("LinuxAll")).unwrap();
    assert_eq!(latest.jobids, [22, 23]);
    assert_eq!(latest.warning.as_deref(), Some(
        "Full backup job 20 of client \"fd\" is on no enabled volume; the chain [22, 23] lacks it and a restore would be incomplete"));
    assert!(!fake.log().iter().any(|s| s.contains("array_agg(DISTINCT fs.fileset")));
}

#[test]
fn fallback_latest_chain_errors() {
    let o = |fake: &Fake, client: &str, fileset: Option<&str>| Ovfs::fallback(fake).jobids_latest(client, fileset).unwrap_err().0;
    let fake = latest_fake("20", "t", "{20}", "[\"A\"]");
    assert_eq!(o(&fake, "nosuch", None), "client \"nosuch\" does not exist");
    let fake = latest_fake("20", "t", "{20}", "[\"A\", \"B\"]");
    assert_eq!(o(&fake, "fd", None), "client \"fd\" has backups of several FileSets [\"A\", \"B\"]; choose one");
    let fake = latest_fake("20", "t", "{20}", "NULL");
    assert_eq!(o(&fake, "fd", None), "client \"fd\" has no backups");
    let fake = latest_fake("NULL", "NULL", "{}", "[\"A\"]");
    assert_eq!(o(&fake, "fd", None), "no Full backup of client \"fd\" with FileSet \"A\" found");
    let fake = latest_fake("20", "f", "{}", "[\"A\"]");
    assert_eq!(o(&fake, "fd", None), "no job of client \"fd\" with FileSet \"A\" is on an enabled volume");
}

#[test]
fn fallback_versions_decode_lstat_and_volumes() {
    let file = lstat(0o100644, 4711, 0);
    let fake = Fake::new(vec![("SELECT f.jobid, j.level::text", vec![
        row(&["3", "I", "B", "1792195200", "2026-10-17 00:00:00", "13", "2", "0", "md5x", &file, "[\"Inc-1\"]", "f"]),
        row(&["1", "F", "B", "1792108800", "2026-10-16 00:00:00", "2", "2", "0", "md5y", &file, "[\"Full-1\",\"Full-2\"]", "t"]),
    ])]);
    let versions = Ovfs::fallback(&fake).versions("fd", "/etc", "passwd", false).unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!((versions[0].jobid, versions[0].size, versions[0].mode), (3, Some(4711), Some(0o100644)));
    assert_eq!(versions[1].volumes, ["Full-1", "Full-2"]);
    assert!(versions[1].inchanger && !versions[0].inchanger);
    let sql = &fake.log()[2];
    assert!(sql.contains("WHERE p.path = '/etc/' AND f.name = 'passwd' AND c.name = 'fd'"), "{sql}");
    assert!(sql.contains("OR false AND j.type = 'C'"));
    assert!(sql.ends_with("ORDER BY j.jobtdate DESC, f.fileid DESC"));
}

#[test]
fn native_calls_the_extension() {
    let fake = Fake::new(vec![("SELECT ovfs.tree(", vec![row(&["{\"name\":\"/\"}"])])]);
    assert_eq!(Ovfs::extension(&fake).tree(&[1, 5], "/it's", 2, 100).unwrap().as_deref(), Some("{\"name\":\"/\"}"));
    assert_eq!(fake.log(), ["SHOW server_encoding", "BEGIN READ ONLY", "SELECT ovfs.tree('{1,5}'::int[], '/it''s', 2, 100)", "COMMIT"]);

    let mut fake = Fake::new(vec![]);
    fake.fail = vec![("SELECT ovfs.tree(", "Query: error returned from database: job 7 does not exist")];
    assert_eq!(Ovfs::extension(&fake).tree(&[7], "/", 1, 10).unwrap_err().0, "job 7 does not exist");
    // argument errors are worded like the extension before any call
    assert_eq!(Ovfs::extension(&fake).tree(&[7], "/", -2, 10).unwrap_err().0, "depth must not be negative, got -2");

    let fake = Fake::new(vec![("SELECT name, name_b64", vec![row(&[
        "disk.img", "NULL", "NULL", "NULL", "file", "11", "23", "3", "1", "[{\"jobid\":10,\"fileindex\":8}]",
        "33188", "0", "0", "230", "1700000000", "NULL", "NULL"])])]);
    let listed = Ovfs::extension(&fake).ls(&[10, 11], "/vm").unwrap();
    assert_eq!(listed[0].delta_parts, Some(vec![Part { jobid: 10, fileindex: 8 }]));
    assert_eq!(fake.log()[2], "SELECT name, name_b64, path, path_b64, kind, jobid, fileid, fileindex, delta_seq, delta_parts::text, \
        mode, uid, gid, size, mtime, link_fileindex, entries FROM ovfs.ls('{10,11}'::int[], '/vm')");
}

#[test]
fn native_latest_recomputes_the_warning() {
    let mut fake = latest_fake("20", "f", "{22,23}", "[\"LinuxAll\"]");
    fake.answers.insert(0, ("SELECT ovfs.jobids_latest(", vec![row(&["{22,23}"])]));
    let latest = Ovfs::extension(&fake).jobids_latest("fd", None).unwrap();
    assert_eq!(latest.jobids, [22, 23]);
    assert!(latest.warning.unwrap().starts_with("Full backup job 20 of client \"fd\" is on no enabled volume"));
    assert_eq!(fake.log()[2], "SELECT ovfs.jobids_latest('fd', NULL)");
}

#[test]
fn capability_selects_the_path() {
    let fake = Fake::new(vec![]);
    assert_eq!(Ovfs::new(&fake, &Capability::NotInstalled).path(), "fallback");
    assert_eq!(Ovfs::new(&fake, &Capability::Unusable { version: "2.0.0".into(), reason: "x".into() }).path(), "fallback");
    assert_eq!(Ovfs::new(&fake, &Capability::Available { version: "0.1.0".into(), schema: "ovfs".into() }).path(), "ovfs");
}

/// Capability detection through a catalog handle (startup, reload, refresh).
#[test]
fn recheck_records_the_capability() {
    use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse};
    let installed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let flag = installed.clone();
    let handle = CatalogHandle::mock(move |request| match request {
        CatalogRequest::TextRows { sql } if sql.contains("pg_extension") => {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                CatalogResponse::Rows(table(vec![row(&["0.1.0", "ovfs", "t", "ok", "ok", "ok", "ok", "ok", "ok"])]))
            } else {
                CatalogResponse::Rows(table(vec![]))
            }
        }
        _ => CatalogResponse::Error("unexpected".into()),
    });
    assert_eq!(handle.ovfs_capability(), Capability::Unknown);
    assert!(handle.ovfs_recheck().usable());
    assert_eq!(handle.ovfs_capability().version(), Some("0.1.0"));
    installed.store(false, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(handle.ovfs_recheck(), Capability::NotInstalled);
    assert_eq!(handle.ovfs_capability(), Capability::NotInstalled);
}

mod equivalence;
