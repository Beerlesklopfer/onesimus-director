//! Equivalence of the Director fallback and the `ovfs` extension.
//!
//! Runs only with `OVFS_EQUIV_URL` set to a PostgreSQL server (without
//! database name, e.g. `postgres://user@127.0.0.1:55439`) on which the
//! extension `ovfs` is available and a Bareos catalog template exists
//! (`OVFS_EQUIV_TEMPLATE`, default `bareos_cppref`). The test creates a
//! throw-away copy (`CREATE DATABASE … TEMPLATE …`; the role needs CREATEDB
//! and the right to `CREATE EXTENSION ovfs`), adds synthetic jobs (base job,
//! Full/Differential/Incrementals, deletions, hard links, delta chains,
//! JobTDate ties, a Full on a disabled volume, odd and non-UTF-8 names), and
//! compares every result of both paths byte for byte. The template itself is
//! never written; the copy is dropped at the end.
//!
//! `OVFS_FALLBACK_URL` (same form, server without the extension) runs the
//! fallback alone on a copy of the same template and checks it detects
//! "not installed" and answers.

use std::sync::atomic::AtomicBool;

use serde_json::Value;

use super::super::lstat::tests::encode;
use super::super::*;
use crate::catalog_worker::{spawn_catalog_worker, CatalogConnection, CatalogHandle};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

async fn admin_exec(base: &str, sql: &str) {
    use sqlx::Connection;
    let mut conn = sqlx::PgConnection::connect(&format!("{base}/postgres")).await.expect("connect to postgres");
    sqlx::raw_sql(sql).execute(&mut conn).await.unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// A throw-away copy of the template; dropped (WITH FORCE) on drop.
struct Scratch {
    base: String,
    name: String,
}

impl Scratch {
    fn create(base: &str, template: &str) -> Scratch {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let name = format!("ovfs_equiv_{}_{}", std::process::id(), nanos % 1_000_000_000);
        runtime().block_on(admin_exec(base, &format!("CREATE DATABASE {name} TEMPLATE {template}")));
        Scratch { base: base.to_string(), name }
    }

    fn url(&self) -> String {
        format!("{}/{}", self.base, self.name)
    }

    fn exec(&self, sql: &str) {
        use sqlx::Connection;
        let url = self.url();
        runtime().block_on(async {
            let mut conn = sqlx::PgConnection::connect(&url).await.expect("connect to scratch database");
            sqlx::raw_sql(sql).execute(&mut conn).await.unwrap_or_else(|e| panic!("setup failed: {e}"));
        });
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        runtime().block_on(admin_exec(&self.base, &format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name)));
    }
}

fn lstat(mode: i64, size: i64, mtime: i64, link_fi: i64) -> String {
    encode([2049, 1000 + size, mode, 1, 1000, 100, 0, size, 4096, 8, mtime, mtime, mtime, link_fi, 0, 1])
}

const DIR: i64 = 0o040755;
const FILE: i64 = 0o100644;
const LINK: i64 = 0o120777;

/// SQL literal; `\xNN` sequences become raw bytes (non-UTF-8 names).
fn lit(text: &str) -> String {
    if text.contains("\\x") {
        format!("E'{}'", text.replace('\'', "\\'"))
    } else {
        format!("'{}'", text.replace('\'', "''"))
    }
}

struct Data {
    paths: Vec<String>,
    files: Vec<String>,
}

impl Data {
    fn file(&mut self, jobid: i32, path: &str, name: &str, fileindex: i32, delta: i32, lstat: String) {
        if !self.paths.iter().any(|p| p == path) {
            self.paths.push(path.to_string());
        }
        self.files.push(format!("({fileindex}, {jobid}, {}, {delta}, '{lstat}', 'md5-{jobid}-{fileindex}', {})", lit(path), lit(name)));
    }
}

/// Synthetic jobs of client `ovfs-fd` (FileSet `OvfsSet`) and `ovfs-fd2`.
fn synthetic_sql() -> String {
    let mut d = Data { paths: Vec::new(), files: Vec::new() };
    let t0 = 1_700_000_000;
    // base job 9000: /base/ (referenced by the Full through BaseFiles)
    let mut fi = 0;
    for name in ["", "b1", "b2", "b3"] {
        fi += 1;
        d.file(9000, "/base/", name, fi, 0, lstat(if name.is_empty() { DIR } else { FILE }, 10 + fi as i64, t0, 0));
    }
    // Full 9001
    let mut fi = 0;
    let mut next = || {
        fi += 1;
        fi
    };
    for top in ["/", "/data/", "/vm/", "/data/links/"] {
        d.file(9001, top, "", next(), 0, lstat(DIR, 4096, t0, 0));
    }
    for i in 0..12 {
        let dir = format!("/data/d{i:02}/");
        d.file(9001, &dir, "", next(), 0, lstat(DIR, 4096, t0, 0));
        for j in 0..4 {
            let sub = format!("{dir}s{j}/");
            if (i + j) % 5 != 0 {
                d.file(9001, &sub, "", next(), 0, lstat(DIR, 4096, t0, 0));
            }
            for k in 0..15 {
                d.file(9001, &sub, &format!("f{k:02}.txt"), next(), 0, lstat(FILE, (i * 100 + j * 10 + k) as i64, t0 + k as i64, 0));
            }
        }
        d.file(9001, &dir, "readme", next(), 0, lstat(FILE, 1, t0, 0));
    }
    // names that sort differently by bytes and by name, quotes, unicode, non-UTF-8
    for (dir, name) in [
        ("/data/a b/", "x"), ("/data/a!x/", "x"), ("/data/a-b/", "x"), ("/data/a.b/", "x"), ("/data/a/", "x"),
        ("/data/a/b/", "deep"), ("/data/it's/", "it's"), ("/data/Übersicht mit Leerzeichen/", "日本語.txt"),
        ("/data/caf\\xe9/", "na\\xefve"), ("/data/pct%_/", "under_score%"), ("C:/Users/anna/", "ntuser.dat"),
    ] {
        d.file(9001, dir, name, next(), 0, lstat(FILE, 7, t0, 0));
    }
    d.file(9001, "/data/", "symlink", next(), 0, lstat(LINK, 12, t0, 0));
    d.file(9001, "/vm/", "disk.img", next(), 0, lstat(FILE, 1 << 30, t0, 0));
    d.file(9001, "/vm/", "gap.img", next(), 0, lstat(FILE, 1 << 20, t0, 0));
    d.file(9001, "/data/", "gone-later", next(), 0, lstat(FILE, 3, t0, 0));
    // Incremental 9002: changes, deletions (also of a whole directory), hard links, delta part 1
    let mut fi = 0;
    let mut next = || {
        fi += 1;
        fi
    };
    d.file(9002, "/data/", "", next(), 0, lstat(DIR, 4096, t0 + 50, 0));
    for i in 0..12 {
        d.file(9002, &format!("/data/d{i:02}/s1/"), &format!("f{:02}.txt", i % 15), next(), 0, lstat(FILE, 999, t0 + 100, 0));
        d.file(9002, &format!("/data/d{i:02}/s2/"), &format!("f{:02}.txt", (i + 3) % 15), 0, 0, lstat(FILE, 0, t0, 0));
    }
    d.file(9002, "/data/d03/s3/", "", 0, 0, lstat(DIR, 4096, t0, 0));
    for k in 0..15 {
        d.file(9002, "/data/d03/s3/", &format!("f{k:02}.txt"), 0, 0, lstat(FILE, 0, t0, 0));
    }
    d.file(9002, "/data/", "gone-later", 0, 0, lstat(FILE, 3, t0, 0));
    d.file(9002, "/data/links/", "h", 51, 0, lstat(FILE, 5, t0, 50));
    d.file(9002, "/data/links/", "h", 50, 0, lstat(FILE, 5, t0, 50));
    d.file(9002, "/data/links/", "dup", 60, 0, lstat(FILE, 6, t0, 0));
    d.file(9002, "/data/links/", "dup", 61, 0, lstat(FILE, 6, t0, 0));
    d.file(9002, "/vm/", "disk.img", next() + 100, 1, lstat(FILE, 1 << 30, t0 + 200, 0));
    // Differential 9003: delta part without base (dropped), a recreated file
    d.file(9003, "/vm/", "orphan.img", 7, 1, lstat(FILE, 1, t0, 0));
    d.file(9003, "/data/", "gone-later", 8, 0, lstat(FILE, 4, t0 + 300, 0));
    d.file(9003, "/data/d00/s1/", "f00.txt", 9, 0, lstat(FILE, 1234, t0 + 300, 0));
    // Incrementals 9004/9005 with the same JobTDate (tie, JobId decides)
    d.file(9004, "/vm/", "disk.img", 3, 2, lstat(FILE, 1 << 30, t0 + 400, 0));
    d.file(9004, "/vm/", "gap.img", 4, 2, lstat(FILE, 1 << 20, t0 + 400, 0));
    d.file(9004, "/data/", "tie.txt", 5, 0, lstat(FILE, 40, t0 + 400, 0));
    d.file(9005, "/data/", "tie.txt", 6, 0, lstat(FILE, 50, t0 + 400, 0));
    // copy job 9006 (versions with copies)
    d.file(9006, "/data/", "tie.txt", 6, 0, lstat(FILE, 50, t0 + 400, 0));
    // client ovfs-fd2: Full 9010 on a disabled volume, Incremental 9011
    d.file(9010, "/srv/", "", 1, 0, lstat(DIR, 4096, t0, 0));
    d.file(9010, "/srv/", "a", 2, 0, lstat(FILE, 1, t0, 0));
    d.file(9011, "/srv/", "b", 1, 0, lstat(FILE, 2, t0, 0));

    let paths: Vec<String> = d.paths.iter().map(|p| format!("({})", lit(p))).collect();
    format!(
        "INSERT INTO client (clientid, name, uname) VALUES (9001, 'ovfs-fd', ''), (9002, 'ovfs-fd2', ''); \
         INSERT INTO fileset (filesetid, fileset, md5, createtime) VALUES \
             (9001, 'OvfsSet', 'x', now()), (9002, 'OvfsSet', 'y', now()), (9003, 'OtherSet', 'z', now()); \
         INSERT INTO media (mediaid, volumename, mediatype, volstatus, enabled, inchanger) VALUES \
             (9001, 'OvfsVol1', 'File', 'Full', 1, 1), (9002, 'OvfsVol2', 'File', 'Full', 0, 0), (9003, 'OvfsVol3', 'File', 'Full', 1, 0); \
         INSERT INTO job (jobid, job, name, type, level, jobstatus, clientid, filesetid, jobtdate, starttime) VALUES \
             (9000, 'base.1', 'ovfs-base', 'B', 'B', 'T', 9001, 9001, 1000, now() - interval '9 day'), \
             (9001, 'full.1', 'ovfs-backup', 'B', 'F', 'T', 9001, 9001, 2000, now() - interval '8 day'), \
             (9002, 'inc.1', 'ovfs-backup', 'B', 'I', 'T', 9001, 9002, 3000, now() - interval '7 day'), \
             (9003, 'diff.1', 'ovfs-backup', 'B', 'D', 'T', 9001, 9002, 4000, now() - interval '6 day'), \
             (9004, 'inc.2', 'ovfs-backup', 'B', 'I', 'T', 9001, 9002, 5000, now() - interval '5 day'), \
             (9005, 'inc.3', 'ovfs-backup', 'B', 'I', 'W', 9001, 9002, 5000, now() - interval '5 day'), \
             (9006, 'copy.1', 'ovfs-copy', 'C', 'I', 'T', 9001, 9002, 5000, now() - interval '4 day'), \
             (9007, 'other.1', 'ovfs-other', 'B', 'F', 'T', 9001, 9003, 2500, now() - interval '8 day'), \
             (9010, 'full.2', 'ovfs-backup2', 'B', 'F', 'T', 9002, 9001, 2000, now() - interval '8 day'), \
             (9011, 'inc.4', 'ovfs-backup2', 'B', 'I', 'T', 9002, 9001, 3000, now() - interval '7 day'); \
         INSERT INTO jobmedia (jobid, mediaid, firstindex, lastindex) VALUES \
             (9000, 9001, 1, 100000), (9001, 9001, 1, 400), (9001, 9003, 401, 100000), (9002, 9001, 0, 100000), \
             (9003, 9001, 1, 100000), (9004, 9001, 1, 100000), (9005, 9001, 1, 100000), (9006, 9003, 1, 100000), \
             (9007, 9001, 1, 100000), (9010, 9002, 1, 100000), (9011, 9001, 1, 100000); \
         INSERT INTO path (path) SELECT v FROM (VALUES {paths}) t(v) WHERE NOT EXISTS (SELECT 1 FROM path WHERE path = v); \
         INSERT INTO file (fileindex, jobid, pathid, deltaseq, lstat, md5, name) \
             SELECT v.fi, v.jid, p.pathid, v.ds, v.ls, v.md, v.nm FROM (VALUES {files}) v(fi, jid, pth, ds, ls, md, nm) \
             JOIN path p ON p.path = v.pth; \
         INSERT INTO basefiles (jobid, fileid, fileindex, basejobid) \
             SELECT 9001, f.fileid, f.fileindex, 9000 FROM file f WHERE f.jobid = 9000;",
        paths = paths.join(", "),
        files = d.files.join(", "),
    )
}

fn connect(url: &str) -> (CatalogHandle, CatalogConnection) {
    let (handle, _join) = spawn_catalog_worker(url.to_string(), &SHUTDOWN);
    let db = handle.dedicated().expect("dedicated connection");
    (handle, db)
}

/// Directories of a full tree (depth-first), for `ls`.
fn directories(node: &Value, out: &mut Vec<String>) {
    if node["type"] == "directory" && node.get("path_b64").is_none() {
        if let Some(path) = node["path"].as_str() {
            out.push(path.to_string());
        }
    }
    if let Some(children) = node["children"].as_array() {
        for child in children {
            directories(child, out);
        }
    }
}

/// Files with path (directories carry the path), for `versions`.
fn files(node: &Value, dir: &str, out: &mut Vec<(String, String)>) {
    let path = node["path"].as_str().unwrap_or(dir);
    if let Some(children) = node["children"].as_array() {
        for child in children {
            if child["type"] == "directory" {
                files(child, path, out);
            } else if child.get("name_b64").is_none() {
                out.push((path.to_string(), child["name"].as_str().unwrap_or("").to_string()));
            }
        }
    }
}

/// Key order and values of a JSON text, recursively sorted (the canonical form).
fn canonical(text: &str) -> String {
    fn sort(v: Value) -> Value {
        match v {
            Value::Object(map) => {
                let mut entries: Vec<(String, Value)> = map.into_iter().map(|(k, v)| (k, sort(v))).collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                Value::Object(entries.into_iter().collect())
            }
            Value::Array(items) => Value::Array(items.into_iter().map(sort).collect()),
            other => other,
        }
    }
    sort(serde_json::from_str(text).expect("valid JSON")).to_string()
}

#[test]
fn fallback_equals_extension() {
    let Ok(base) = std::env::var("OVFS_EQUIV_URL") else {
        eprintln!("ovfs equivalence: skipped (OVFS_EQUIV_URL not set)");
        return;
    };
    let template = std::env::var("OVFS_EQUIV_TEMPLATE").unwrap_or_else(|_| "bareos_cppref".into());
    let scratch = Scratch::create(&base, &template);
    scratch.exec("CREATE EXTENSION ovfs");
    scratch.exec(&synthetic_sql());

    let (handle, db) = connect(&scratch.url());
    let capability = handle.ovfs_recheck();
    assert!(capability.usable(), "{capability}");
    assert_eq!(Ovfs::new(&db, &capability).path(), "ovfs");
    let native = Ovfs::extension(&db);
    let fallback = Ovfs::fallback(&db);
    let mut compared = (0, 0, 0, 0, 0);

    // job chains: real catalog jobs, latest states, synthetic chains, single jobs
    let mut chains: Vec<Vec<i32>> = vec![vec![1], vec![5], vec![1, 5], vec![5, 1], vec![9000], vec![9001], vec![9002],
        vec![9001, 9002], vec![9001, 9002, 9003], vec![9001, 9002, 9003, 9004, 9005], vec![9005, 9004, 9003, 9002, 9001],
        vec![9001, 9003], vec![9010, 9011], vec![9001, 9006], vec![9002, 9004]];
    for (client, fileset) in [("bareos-fd", Some("SelfTest")), ("bareos-fd", Some("Catalog")), ("bareos-fd", None),
        ("ovfs-fd", Some("OvfsSet")), ("ovfs-fd", Some("OtherSet")), ("ovfs-fd", None), ("ovfs-fd2", None), ("nosuch", None)] {
        let a = native.jobids_latest(client, fileset);
        let b = fallback.jobids_latest(client, fileset);
        assert_eq!(a, b, "jobids_latest({client}, {fileset:?})");
        compared.0 += 1;
        if let Ok(latest) = a {
            chains.push(latest.jobids);
        }
    }
    let warned = fallback.jobids_latest("ovfs-fd2", None).unwrap();
    assert_eq!(warned.jobids, [9011]);
    assert!(warned.warning.is_some());

    for chain in &chains {
        // the whole tree, then depths and roots
        let full = native.tree(chain, "/", 1000, 1_000_000);
        assert_eq!(full, fallback.tree(chain, "/", 1000, 1_000_000), "tree({chain:?}, /, 1000)");
        let mut roots = vec!["/".to_string(), "C:/".to_string(), "/nothing/".to_string()];
        if let Ok(Some(text)) = &full {
            let value: Value = serde_json::from_str(text).unwrap();
            directories(&value, &mut roots);
        }
        let windows = native.tree(chain, "C:/", 1000, 1_000_000).ok().flatten();
        if let Some(text) = windows {
            directories(&serde_json::from_str(&text).unwrap(), &mut roots);
        }
        roots.dedup();
        for root in roots.iter().take(80) {
            for depth in [0, 1, 2, 3, 1000] {
                let a = native.tree(chain, root, depth, 1_000_000);
                let b = fallback.tree(chain, root, depth, 1_000_000);
                assert_eq!(a, b, "tree({chain:?}, {root}, {depth})");
                if let (Ok(Some(a)), Ok(Some(b))) = (&a, &b) {
                    assert_eq!(canonical(a), canonical(b));
                }
                compared.1 += 1;
            }
            let a = native.ls(chain, root);
            assert_eq!(a, fallback.ls(chain, root), "ls({chain:?}, {root})");
            compared.2 += 1;
        }
        // max_entries counts result nodes
        for max in [1, 2, 5, 50] {
            assert_eq!(native.tree(chain, "/", 1000, max), fallback.tree(chain, "/", 1000, max), "max_entries {max} {chain:?}");
            compared.1 += 1;
        }
    }

    // versions of every file of the latest states (and directories: name '')
    let mut wanted: Vec<(String, String, String)> = Vec::new();
    for (client, fileset) in [("bareos-fd", Some("SelfTest")), ("ovfs-fd", Some("OvfsSet"))] {
        let chain = native.jobids_latest(client, fileset).unwrap().jobids;
        let tree: Value = serde_json::from_str(&native.tree(&chain, "/", 1000, 1_000_000).unwrap().unwrap()).unwrap();
        let mut found = Vec::new();
        files(&tree, "/", &mut found);
        wanted.extend(found.into_iter().map(|(p, n)| (client.to_string(), p, n)));
    }
    wanted.push(("ovfs-fd".into(), "/data/".into(), String::new()));
    wanted.push(("ovfs-fd".into(), "/data/".into(), "tie.txt".into()));
    wanted.push(("ovfs-fd".into(), "/nothing".into(), "x".into()));
    for (client, path, name) in wanted.iter().step_by(3).chain(wanted.iter().rev().take(3)) {
        for copies in [false, true] {
            let a = native.versions(client, path, name, copies);
            assert_eq!(a, fallback.versions(client, path, name, copies), "versions({client}, {path}, {name}, {copies})");
            compared.3 += 1;
        }
    }

    // errors read the same
    for (chain, root, depth, max) in [(vec![424242], "/", 1, 10), (vec![424242, 424243, 1], "/", 1, 10), (vec![], "/", 1, 10),
        (vec![1], "", 1, 10), (vec![1], "/", -1, 10), (vec![1], "/", 1, 0)] {
        let a = native.tree(&chain, root, depth, max);
        assert!(a.is_err());
        assert_eq!(a, fallback.tree(&chain, root, depth, max), "error case {chain:?} {root:?} {depth} {max}");
        compared.4 += 1;
    }
    assert_eq!(native.ls(&[1], ""), fallback.ls(&[1], ""));
    assert_eq!(native.versions("ovfs-fd", "", "x", false), fallback.versions("ovfs-fd", "", "x", false));

    // the extension dropped: the next check falls back
    drop(db);
    scratch.exec("DROP EXTENSION ovfs");
    assert_eq!(handle.ovfs_recheck(), Capability::NotInstalled);
    eprintln!(
        "ovfs equivalence: {} jobids_latest, {} tree, {} ls, {} versions, {} error comparisons over {} chains: identical",
        compared.0, compared.1, compared.2, compared.3, compared.4, chains.len()
    );
    drop(handle);
}

#[test]
fn fallback_without_extension() {
    let Ok(base) = std::env::var("OVFS_FALLBACK_URL") else {
        eprintln!("ovfs fallback: skipped (OVFS_FALLBACK_URL not set)");
        return;
    };
    let template = std::env::var("OVFS_EQUIV_TEMPLATE").unwrap_or_else(|_| "bareos_cppref".into());
    let scratch = Scratch::create(&base, &template);
    scratch.exec(&synthetic_sql());
    let (handle, db) = connect(&scratch.url());
    let capability = handle.ovfs_recheck();
    assert_eq!(capability, Capability::NotInstalled);
    let ovfs = Ovfs::new(&db, &capability);
    assert_eq!(ovfs.path(), "fallback");
    let latest = ovfs.jobids_latest("ovfs-fd", Some("OvfsSet")).unwrap();
    assert_eq!(latest.jobids, [9001, 9003, 9004, 9005]);
    let tree = ovfs.tree(&latest.jobids, "/", 2, 1000).unwrap().unwrap();
    assert!(tree.starts_with(r#"{"name":"/","path":"/","type":"directory""#), "{tree}");
    let listed = ovfs.ls(&latest.jobids, "/data/").unwrap();
    assert!(listed.iter().any(|r| r.name == "tie.txt" && r.jobid == Some(9005)));
    assert!(!listed.iter().any(|r| r.name == "gone-later" && r.jobid == Some(9001)));
    drop(db);
    // let the worker release the reserved connection before it stops
    let _ = handle.ping();
    drop(handle);
}
