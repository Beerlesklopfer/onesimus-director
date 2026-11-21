use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Throw-away configuration directory under the temp directory.
pub(crate) fn config_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("onesimus-config-edit-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["director", "messages", "catalog", "client", "storage", "pool", "fileset", "job", "jobdefs", "schedule", "console"] {
        std::fs::create_dir_all(dir.join("bareos-dir.d").join(sub)).unwrap();
    }
    let write = |sub: &str, file: &str, text: &str| std::fs::write(dir.join("bareos-dir.d").join(sub).join(file), text).unwrap();
    write("director", "bareos-dir.conf", "Director {\n  Name = bareos-dir\n  Password = secret\n  Messages = Daemon\n}\n");
    write("messages", "Daemon.conf", "Messages {\n  Name = Daemon\n  Console = all\n}\n");
    write("messages", "Standard.conf", "Messages {\n  Name = Standard\n  Console = all\n}\n");
    write("catalog", "MyCatalog.conf", "Catalog {\n  Name = MyCatalog\n  DbName = nosuchdb\n  DbUser = x\n}\n");
    write("client", "fd.conf", "Client {\n  Name = fd\n  Address = localhost\n  Password = p\n}\n");
    write("storage", "File.conf", "Storage {\n  Name = File\n  Address = localhost\n  Password = p\n  Device = FileStorage\n  Media Type = File\n}\n");
    write("pool", "Full.conf", "Pool {\n  Name = Full\n  Pool Type = Backup\n}\n");
    write("fileset", "fs.conf", "FileSet {\n  Name = fs\n  Include {\n    File = /etc\n  }\n}\n");
    write("job", "a.conf", JOB_A);
    dir
}

pub(crate) const JOB_A: &str = "Job {\n  Name = a\n  Type = Backup\n  Client = fd\n  FileSet = fs\n  Storage = File\n  Pool = Full\n  Messages = Standard\n}\n";

pub(crate) const WEEKLY: &str = "Schedule {\n  Name = \"WeeklyCycle\"\n  Run = Full 1st sat at 21:00\n}\n";

/// Editor with a counting reload (fails while `fail` is set) and the
/// Director messages collected.
pub(crate) struct Harness {
    pub editor: ConfigEditor,
    pub dir: PathBuf,
    pub reloads: Arc<AtomicUsize>,
    pub fail: Arc<AtomicUsize>,
    pub messages: Arc<Mutex<Vec<String>>>,
}

pub(crate) fn harness(name: &str) -> Harness {
    let dir = config_dir(name);
    let reloads = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicUsize::new(0));
    let messages = Arc::new(Mutex::new(Vec::new()));
    let (r, f, m, d) = (reloads.clone(), fail.clone(), messages.clone(), dir.clone());
    let editor = ConfigEditor::new(dir.clone(), Box::new(move || {
        r.fetch_add(1, Ordering::SeqCst);
        if f.load(Ordering::SeqCst) > 0 {
            f.fetch_sub(1, Ordering::SeqCst);
            return Err("simulated reload failure".into());
        }
        // what the reload of the Director checks first (without the catalog)
        let config = DirectorConfig::load(&d).map_err(|e| e.to_string())?;
        crate::config_check::check(&config.raw, &crate::config_check::base_config_path(&d))
            .map_err(|diags| diags[0].text().to_string())
    }), Box::new(move |text: &str| m.lock().unwrap().push(text.to_string())));
    Harness { editor, dir, reloads, fail, messages }
}

fn all(_: &str) -> bool {
    true
}

impl Harness {
    fn file(&self, sub: &str, name: &str) -> PathBuf {
        self.dir.join("bareos-dir.d").join(sub).join(name)
    }
    fn create(&self, rtype: RType, text: &str) -> Outcome {
        self.editor.create(rtype, text, false, "admin", &all).unwrap()
    }
    fn read(&self, rtype: RType, name: &str) -> Outcome {
        self.editor.read(rtype, name, &all).unwrap()
    }
    fn update(&self, rtype: RType, name: &str, sha: &str, text: &str) -> Outcome {
        self.editor.update(rtype, name, sha, text, false, "admin", &all).unwrap()
    }
    fn delete(&self, rtype: RType, name: &str, sha: &str) -> Outcome {
        self.editor.delete(rtype, name, sha, false, "admin", &all).unwrap()
    }
    /// Contents of every file of the configuration.
    fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        for sub in std::fs::read_dir(self.dir.join("bareos-dir.d")).unwrap() {
            for f in std::fs::read_dir(sub.unwrap().path()).unwrap() {
                let p = f.unwrap().path();
                files.push((p.clone(), std::fs::read(&p).unwrap()));
            }
        }
        files.sort();
        files
    }
}

#[test]
fn spans_follow_the_lexer() {
    let text = "# head\nSchedule {\n  Name = \"a } # not a comment\"\n  Run = Full sun at 1:00 # c }\n} # end\n\
                Job { Name = x; Include { File = \"/a\\\"b\" } }\n@include \"/etc/x\"\nJobDefs\n{\n Name = d\n}";
    let spans = resource_spans(text).unwrap();
    assert_eq!(spans.len(), 3);
    assert!(text[spans[0].0..spans[0].1].starts_with("Schedule {") && text[spans[0].0..spans[0].1].ends_with("} # end\n"));
    assert_eq!(&text[spans[1].0..spans[1].1], "Job { Name = x; Include { File = \"/a\\\"b\" } }\n");
    assert_eq!(&text[spans[2].0..spans[2].1], "JobDefs\n{\n Name = d\n}");
    let err = resource_spans("Schedule {\n  Name = x\n").unwrap_err();
    assert_eq!(err.offset, 0);
    assert_eq!(line_col("ab\ncd", 4), (2, 2));
}

#[test]
fn schedule_create_read_update_delete() {
    let h = harness("crud");
    let created = h.create(RType::Schedule, WEEKLY);
    assert_eq!(created.status, Status::Ok, "{created:?}");
    assert!(created.reloaded && !created.validated_only);
    let path = h.file("schedule", "WeeklyCycle.conf");
    assert_eq!(created.file, path.display().to_string());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), WEEKLY);
    assert_eq!(created.sha256, sha256_hex(WEEKLY));
    assert!(created.followups.is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // mode like the other files of the configuration
        let other = std::fs::metadata(h.file("job", "a.conf")).unwrap().mode() & 0o7777;
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, other);
    }

    let read = h.read(RType::Schedule, "WeeklyCycle");
    assert_eq!((read.status, read.text.as_str(), read.sha256.as_str(), read.shared_file), (Status::Ok, WEEKLY, created.sha256.as_str(), false));

    let changed = WEEKLY.replace("21:00", "22:30");
    let updated = h.update(RType::Schedule, "WeeklyCycle", &read.sha256, &changed);
    assert_eq!(updated.status, Status::Ok, "{updated:?}");
    assert_eq!((updated.old_sha256.as_str(), updated.sha256.clone()), (read.sha256.as_str(), sha256_hex(&changed)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), changed);

    let deleted = h.delete(RType::Schedule, "WeeklyCycle", &updated.sha256);
    assert_eq!(deleted.status, Status::Ok, "{deleted:?}");
    assert!(!path.exists());
    assert_eq!(h.read(RType::Schedule, "WeeklyCycle").status, Status::NotFound);
    assert_eq!(h.reloads.load(Ordering::SeqCst), 3);
    // journal/Director message per write
    let messages = h.messages.lock().unwrap().clone();
    assert_eq!(messages.len(), 3);
    assert!(messages[1].starts_with(&format!("Console [admin] config update Schedule \"WeeklyCycle\": old sha256 {}, new sha256 {}, file {}",
        read.sha256, updated.sha256, path.display())), "{messages:?}");
    assert!(messages[0].contains("old sha256 -,") && messages[2].contains("new sha256 -,"), "{messages:?}");
    // no temporary files left
    assert!(std::fs::read_dir(h.file("schedule", "")).unwrap().next().is_none());
}

#[test]
fn create_refuses_existing_and_dangerous_names() {
    let h = harness("names");
    assert_eq!(h.create(RType::Schedule, WEEKLY).status, Status::Ok);
    let again = h.create(RType::Schedule, WEEKLY);
    assert_eq!(again.status, Status::Exists);
    assert_eq!(again.sha256, sha256_hex(WEEKLY));
    for bad in ["../x", "a/b", ".hidden", "a..b", "tab\there"] {
        let o = h.create(RType::Schedule, &format!("Schedule {{\n  Name = \"{bad}\"\n}}\n"));
        assert_eq!(o.status, Status::Invalid, "{bad}: {o:?}");
    }
    // wrong type, two resources, nothing to write
    assert_eq!(h.create(RType::Schedule, JOB_A).status, Status::Invalid);
    assert_eq!(h.create(RType::Schedule, &format!("{WEEKLY}{}", WEEKLY.replace("WeeklyCycle", "B"))).status, Status::Invalid);
    assert_eq!(h.create(RType::Schedule, "# only a comment\n").status, Status::Invalid);
    assert_eq!(h.reloads.load(Ordering::SeqCst), 1);
}

#[test]
fn conflict_returns_the_current_text_and_sha() {
    let h = harness("conflict");
    let created = h.create(RType::Schedule, WEEKLY);
    // someone else changed the file meanwhile
    let other = WEEKLY.replace("sat", "sun");
    std::fs::write(h.file("schedule", "WeeklyCycle.conf"), &other).unwrap();
    let o = h.update(RType::Schedule, "WeeklyCycle", &created.sha256, &WEEKLY.replace("21:00", "23:00"));
    assert_eq!(o.status, Status::Conflict);
    assert_eq!((o.text.as_str(), o.sha256.clone()), (other.as_str(), sha256_hex(&other)));
    assert!(o.file.ends_with("WeeklyCycle.conf"));
    let o = h.delete(RType::Schedule, "WeeklyCycle", "");
    assert_eq!((o.status, o.text.as_str()), (Status::Conflict, other.as_str()));
    assert_eq!(std::fs::read_to_string(h.file("schedule", "WeeklyCycle.conf")).unwrap(), other);
}

#[test]
fn invalid_text_reports_file_line_and_column() {
    let h = harness("invalid");
    let created = h.create(RType::Schedule, WEEKLY);
    let bad = "Schedule {\n  Name = \"WeeklyCycle\"\n  Frequently = often\n}\n";
    let o = h.update(RType::Schedule, "WeeklyCycle", &created.sha256, bad);
    assert_eq!(o.status, Status::Invalid, "{o:?}");
    let p = o.problem.unwrap();
    assert_eq!((p.file.as_str(), p.line, p.column), (h.file("schedule", "WeeklyCycle.conf").to_str().unwrap(), 3, 13));
    assert!(p.text.contains("Config error: Keyword \"Frequently\" not permitted in this resource."), "{}", p.text);
    assert_eq!(std::fs::read_to_string(h.file("schedule", "WeeklyCycle.conf")).unwrap(), WEEKLY);
    // a reference to a resource that does not exist (new file)
    let o = h.create(RType::Job, &JOB_A.replace("Name = a", "Name = b").replace("Pool = Full", "Pool = Nope"));
    assert_eq!(o.status, Status::Invalid);
    let p = o.problem.unwrap();
    assert_eq!((p.line, p.file.ends_with("job/b.conf")), (7, true), "{p:?}");
    assert!(!h.file("job", "b.conf").exists());
    // unbalanced braces: position in the submitted text
    let o = h.update(RType::Schedule, "WeeklyCycle", &created.sha256, "Schedule {\n  Name = \"WeeklyCycle\"\n");
    assert_eq!(o.status, Status::Invalid);
    assert_eq!(o.problem.map(|p| (p.line, p.column)), Some((1, 1)));
    // the name must stay
    let o = h.update(RType::Schedule, "WeeklyCycle", &created.sha256, &WEEKLY.replace("WeeklyCycle", "Other"));
    assert_eq!(o.status, Status::Invalid);
    assert_eq!(h.reloads.load(Ordering::SeqCst), 1);
}

#[test]
fn a_reference_prevents_the_delete() {
    let h = harness("referenced");
    let created = h.create(RType::Schedule, WEEKLY);
    std::fs::write(h.file("job", "a.conf"), JOB_A.replace("  Pool = Full\n", "  Pool = Full\n  Schedule = \"WeeklyCycle\"\n")).unwrap();
    std::fs::write(h.file("jobdefs", "d.conf"), "JobDefs {\n  Name = d\n  Schedule = WeeklyCycle\n}\n").unwrap();
    let o = h.delete(RType::Schedule, "WeeklyCycle", &created.sha256);
    assert_eq!(o.status, Status::Referenced, "{o:?}");
    let mut by = o.referenced_by.clone();
    by.sort();
    assert_eq!(by, vec![("job".to_string(), "a".to_string()), ("jobdefs".to_string(), "d".to_string())]);
    assert!(h.file("schedule", "WeeklyCycle.conf").exists());
    // validate_only gives the same answer
    let v = h.editor.delete(RType::Schedule, "WeeklyCycle", &created.sha256, true, "admin", &all).unwrap();
    assert_eq!(v.status, Status::Referenced);
}

#[test]
fn job_references() {
    let raw = bareos_config::raw::parse_str("Job {\n Name = v\n Job To Verify = a\n}\nJob {\n Name = r\n RunScript {\n  Console = \"run job=a yes\"\n }\n}\n\
        Job {\n Name = x\n RunScript {\n  Console = \"run job=%n yes\"\n }\n Run = \"job=a level=Full\"\n}\nJob {\n Name = a\n JobDefs = d\n}\n", None).unwrap();
    assert_eq!(references(&raw, RType::Job, "a"), vec![("job".into(), "v".into()), ("job".into(), "r".into()), ("job".into(), "x".into())]);
    assert_eq!(references(&raw, RType::JobDefs, "d"), vec![("job".into(), "a".into())]);
    assert_eq!(run_job_name("run job=\"a b\" yes").as_deref(), Some("a b"));
    assert_eq!(run_job_name("run job=$x"), None);
    assert_eq!(run_job_name("run myjob=a"), None);
}

#[test]
fn shared_file_keeps_the_other_resource_byte_for_byte() {
    let h = harness("shared");
    let other = "Schedule {\n  Name = Other   # keep\n  Run = Incremental mon-fri at 23:10\n}\n";
    let file = h.file("schedule", "all.conf");
    let original = format!("# two schedules\n{WEEKLY}\n{other}# tail\n");
    std::fs::write(&file, &original).unwrap();
    let read = h.read(RType::Schedule, "WeeklyCycle");
    assert_eq!((read.text.as_str(), read.shared_file), (WEEKLY, true));
    let list = h.editor.list(RType::Schedule, &all).unwrap();
    assert!(list.iter().all(|s| s.shared_file && s.file == file.display().to_string()));
    let changed = WEEKLY.replace("21:00", "20:00");
    assert_eq!(h.update(RType::Schedule, "WeeklyCycle", &read.sha256, &changed).status, Status::Ok);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("# two schedules\n{changed}\n{other}# tail\n"));
    let o = h.delete(RType::Schedule, "WeeklyCycle", &sha256_hex(&changed));
    assert_eq!(o.status, Status::Ok, "{o:?}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("# two schedules\n\n{other}# tail\n"));
    assert_eq!(h.read(RType::Schedule, "Other").text, other);
    // the last resource: a file with only comments left is removed
    let o = h.delete(RType::Schedule, "Other", &sha256_hex(other));
    assert_eq!(o.status, Status::Ok);
    assert!(!file.exists());
}

#[test]
fn comments_and_directives_the_gui_does_not_know_are_kept() {
    let h = harness("comments");
    let text = "Schedule {\n  # managed by hand\n  Name = \"WeeklyCycle\"   ; Description = \"x \\\"y\\\"\"\n  Enabled = no\n  Run = Level=Full Pool=Full 1st sat at 21:00  # monthly\n}\n";
    let o = h.create(RType::Schedule, text);
    assert_eq!(o.status, Status::Ok, "{o:?}");
    assert_eq!(std::fs::read_to_string(h.file("schedule", "WeeklyCycle.conf")).unwrap(), text);
    assert_eq!(h.read(RType::Schedule, "WeeklyCycle").text, text);
    // a comment at the start of a file stays when the resource changes
    std::fs::write(h.file("job", "a.conf"), format!("# job a\n{JOB_A}")).unwrap();
    let read = h.read(RType::Job, "a");
    let changed = JOB_A.replace("  Type = Backup\n", "  Type = Backup\n  # nightly\n  Priority = 11\n");
    assert_eq!(h.update(RType::Job, "a", &read.sha256, &changed).status, Status::Ok);
    assert_eq!(std::fs::read_to_string(h.file("job", "a.conf")).unwrap(), format!("# job a\n{changed}"));
}

#[test]
fn crlf_files_keep_their_line_ends() {
    let h = harness("crlf");
    let crlf = WEEKLY.replace('\n', "\r\n");
    let file = h.file("schedule", "WeeklyCycle.conf");
    std::fs::write(&file, &crlf).unwrap();
    let read = h.read(RType::Schedule, "WeeklyCycle");
    assert_eq!((read.text.as_str(), read.sha256.clone()), (crlf.as_str(), sha256_hex(&crlf)));
    assert_eq!(h.read(RType::Schedule, "WeeklyCycle").sha256, read.sha256);
    // the text without its final line end gets the file's CRLF
    let changed = crlf.replace("21:00", "21:30");
    let o = h.update(RType::Schedule, "WeeklyCycle", &read.sha256, changed.trim_end());
    assert_eq!(o.status, Status::Ok, "{o:?}");
    assert_eq!(std::fs::read(&file).unwrap(), changed.as_bytes());
    assert_eq!(o.sha256, sha256_hex(&changed));
}

#[test]
fn validate_only_writes_nothing() {
    let h = harness("validate");
    let created = h.create(RType::Schedule, WEEKLY);
    let before = h.snapshot();
    let o = h.editor.create(RType::Schedule, &WEEKLY.replace("WeeklyCycle", "New"), true, "admin", &all).unwrap();
    assert_eq!((o.status, o.validated_only, o.reloaded), (Status::Ok, true, false), "{o:?}");
    let o = h.editor.update(RType::Schedule, "WeeklyCycle", &created.sha256, &WEEKLY.replace("sat", "sun"), true, "admin", &all).unwrap();
    assert_eq!((o.status, o.validated_only, o.old_sha256.as_str()), (Status::Ok, true, created.sha256.as_str()));
    let o = h.editor.delete(RType::Schedule, "WeeklyCycle", &created.sha256, true, "admin", &all).unwrap();
    assert_eq!((o.status, o.validated_only), (Status::Ok, true));
    // the same INVALID answer as a real call
    let o = h.editor.update(RType::Schedule, "WeeklyCycle", &created.sha256, "Schedule {\n  Name = WeeklyCycle\n  Bad = 1\n}\n", true, "admin", &all).unwrap();
    assert_eq!(o.status, Status::Invalid);
    assert_eq!(h.snapshot(), before);
    assert_eq!(h.reloads.load(Ordering::SeqCst), 1);
    assert_eq!(h.messages.lock().unwrap().len(), 1);
}

#[test]
fn a_failed_reload_restores_the_previous_state() {
    let h = harness("reload");
    let created = h.create(RType::Schedule, WEEKLY);
    let before = h.snapshot();
    h.fail.store(1, Ordering::SeqCst);
    let o = h.update(RType::Schedule, "WeeklyCycle", &created.sha256, &WEEKLY.replace("21:00", "05:00"));
    assert_eq!(o.status, Status::ReloadFailed, "{o:?}");
    assert!(o.message.contains("simulated reload failure"), "{}", o.message);
    assert_eq!((o.text.as_str(), o.sha256.as_str()), (WEEKLY, created.sha256.as_str()));
    assert_eq!(h.snapshot(), before);
    // reloaded twice: the failed one and the one after the restore
    assert_eq!(h.reloads.load(Ordering::SeqCst), 3);
    // create and delete are undone too
    h.fail.store(1, Ordering::SeqCst);
    assert_eq!(h.create(RType::Schedule, &WEEKLY.replace("WeeklyCycle", "N")).status, Status::ReloadFailed);
    h.fail.store(1, Ordering::SeqCst);
    assert_eq!(h.delete(RType::Schedule, "WeeklyCycle", &created.sha256).status, Status::ReloadFailed);
    assert_eq!(h.snapshot(), before);
    assert!(h.messages.lock().unwrap().iter().filter(|m| m.contains("reload failed")).count() == 3);
}

#[test]
fn list_in_resource_order_with_acl() {
    let h = harness("list");
    for name in ["A", "B", "C"] {
        assert_eq!(h.create(RType::Schedule, &WEEKLY.replace("WeeklyCycle", name)).status, Status::Ok);
    }
    let list = h.editor.list(RType::Schedule, &|n| n != "B").unwrap();
    // files are read in reverse glob order (C++ resource chain)
    assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["C", "A"]);
    assert_eq!(list[0].sha256, sha256_hex(&WEEKLY.replace("WeeklyCycle", "C")));
    assert!(list[0].file.ends_with("schedule/C.conf") && !list[0].shared_file);
    let jobs = h.editor.list(RType::Job, &all).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(h.editor.read(RType::Schedule, "B", &|n| n != "B").unwrap().status, Status::Denied);
}

#[test]
fn two_concurrent_updates_do_not_overtake_each_other() {
    let h = Arc::new(harness("concurrent"));
    let created = h.create(RType::Schedule, WEEKLY);
    let threads: Vec<_> = ["01:00", "02:00"].into_iter().map(|t| {
        let h = h.clone();
        let sha = created.sha256.clone();
        std::thread::spawn(move || h.update(RType::Schedule, "WeeklyCycle", &sha, &WEEKLY.replace("21:00", t)).status)
    }).collect();
    let mut statuses: Vec<Status> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    statuses.sort_by_key(|s| *s as u8);
    assert_eq!(statuses, [Status::Ok, Status::Conflict]);
}

pub(crate) const CLIENT_TLS: &str = "Client {\n  # laptop, TLS with PSK off\n  Name = \"laptop-fd\"   # name\n  Address = laptop.example   ; Port = 9102\n  Password = \"[md5]0123456789abcdef\"\n  TLS Enable = yes\n  TLS Require = yes\n  TLS CA Certificate File = \"/etc/bareos/tls/ca.pem\"\n  TLS Certificate = /etc/bareos/tls/dir.pem\n  TLS Key = /etc/bareos/tls/dir.key\n  TLS Allowed CN = \"laptop.example\"\n  File Retention = 30 days\n  Job Retention = 6 months\n  AutoPrune = no\n  Maximum Concurrent Jobs = 2\n  Heartbeat Interval = 60\n}\n";

#[test]
fn client_create_read_update_delete_keeps_every_directive() {
    let h = harness("client-crud");
    let created = h.create(RType::Client, CLIENT_TLS);
    assert_eq!(created.status, Status::Ok, "{created:?}");
    let path = h.file("client", "laptop-fd.conf");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CLIENT_TLS);
    let read = h.read(RType::Client, "laptop-fd");
    assert_eq!((read.status, read.text.as_str(), read.sha256.as_str()), (Status::Ok, CLIENT_TLS, created.sha256.as_str()));
    // only one value changes, every other byte (TLS, comments, `;`) stays
    // quoted like the MCP tool configure_update writes values with blanks
    let changed = CLIENT_TLS.replace("File Retention = 30 days", "File Retention = \"60 days\"");
    let updated = h.update(RType::Client, "laptop-fd", &read.sha256, &changed);
    assert_eq!(updated.status, Status::Ok, "{updated:?}");
    assert_eq!(std::fs::read(&path).unwrap(), changed.as_bytes());
    assert!(updated.followups.is_empty());
    let list = h.editor.list(RType::Client, &all).unwrap();
    assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["laptop-fd", "fd"]);
    // missing Address: INVALID like `odir -t`, nothing written
    let bad = changed.replace("  Address = laptop.example   ; Port = 9102\n", "");
    let o = h.update(RType::Client, "laptop-fd", &updated.sha256, &bad);
    assert_eq!(o.status, Status::Invalid, "{o:?}");
    assert_eq!(std::fs::read(&path).unwrap(), changed.as_bytes());
    let deleted = h.delete(RType::Client, "laptop-fd", &updated.sha256);
    assert_eq!(deleted.status, Status::Ok, "{deleted:?}");
    assert!(!path.exists());
    assert_eq!(h.reloads.load(Ordering::SeqCst), 3);
    assert!(h.messages.lock().unwrap()[1].starts_with("Console [admin] config update Client \"laptop-fd\""));
}

#[test]
fn client_crlf_and_conflict() {
    let h = harness("client-crlf");
    let crlf = CLIENT_TLS.replace('\n', "\r\n");
    let file = h.file("client", "laptop-fd.conf");
    std::fs::write(&file, &crlf).unwrap();
    let read = h.read(RType::Client, "laptop-fd");
    assert_eq!(read.sha256, sha256_hex(&crlf));
    let o = h.update(RType::Client, "laptop-fd", "deadbeef", &crlf);
    assert_eq!((o.status, o.text.as_str()), (Status::Conflict, crlf.as_str()));
    let changed = crlf.replace("Heartbeat Interval = 60", "Heartbeat Interval = 120");
    assert_eq!(h.update(RType::Client, "laptop-fd", &read.sha256, &changed).status, Status::Ok);
    assert_eq!(std::fs::read(&file).unwrap(), changed.as_bytes());
}

#[test]
fn a_job_using_the_client_prevents_the_delete() {
    let h = harness("client-referenced");
    // JOB_A uses Client = fd
    let read = h.read(RType::Client, "fd");
    let o = h.delete(RType::Client, "fd", &read.sha256);
    assert_eq!(o.status, Status::Referenced, "{o:?}");
    assert_eq!(o.referenced_by, vec![("job".to_string(), "a".to_string())]);
    assert!(h.file("client", "fd.conf").exists());
    // JobDefs, `run ... client=` in a Run value and in a RunScript console command
    assert_eq!(h.create(RType::Client, CLIENT_TLS).status, Status::Ok);
    std::fs::write(h.file("jobdefs", "d.conf"), "JobDefs {\n  Name = d\n  Client = \"laptop-fd\"\n}\n").unwrap();
    let raw = bareos_config::raw::parse_str("Job {\n Name = r\n Run = \"job=a client=laptop-fd level=Full\"\n}\n\
        Job {\n Name = s\n RunScript {\n  Console = \"run job=a client=\\\"laptop-fd\\\" yes\"\n }\n}\n\
        Job {\n Name = t\n Run = \"job=a client=%c\"\n RunScript {\n  Console = \"run job=a myclient=laptop-fd\"\n }\n}\n\
        Console {\n Name = c\n Password = p\n ClientACL = laptop-fd\n}\n", None).unwrap();
    assert_eq!(references(&raw, RType::Client, "laptop-fd"), vec![("job".into(), "r".into()), ("job".into(), "s".into())]);
    let read = h.read(RType::Client, "laptop-fd");
    let o = h.delete(RType::Client, "laptop-fd", &read.sha256);
    assert_eq!((o.status, o.referenced_by.clone()), (Status::Referenced, vec![("jobdefs".to_string(), "d".to_string())]));
    assert_eq!(run_arg("run job=a Client=x yes", "client=").as_deref(), Some("x"));
}

/// FileSet with Options, Include and Exclude blocks and a comment the GUI does not know.
const FILESET_ORPHAN: &str = "FileSet {\n  Name = orphan\n  # kept as written\n  Include {\n    Options {\n      Signature = XXH128\n    }\n    File = /srv\n  }\n  Exclude {\n    File = /srv/tmp\n  }\n}\n";

#[test]
fn fileset_create_read_update_delete_and_referenced() {
    let h = harness("fileset");
    let created = h.create(RType::FileSet, FILESET_ORPHAN);
    assert_eq!(created.status, Status::Ok, "{created:?}");
    assert_eq!(std::fs::read_to_string(h.file("fileset", "orphan.conf")).unwrap(), FILESET_ORPHAN);
    let read = h.read(RType::FileSet, "orphan");
    assert_eq!((read.status, read.text.as_str()), (Status::Ok, FILESET_ORPHAN));
    let changed = FILESET_ORPHAN.replace("File = /srv\n", "File = /srv\n    File = /opt\n");
    let updated = h.update(RType::FileSet, "orphan", &read.sha256, &changed);
    assert_eq!(updated.status, Status::Ok, "{updated:?}");
    let o = h.delete(RType::FileSet, "orphan", &updated.sha256);
    assert_eq!(o.status, Status::Ok, "{o:?}");
    assert!(!h.file("fileset", "orphan.conf").exists());
    // JOB_A uses FileSet = fs
    let read = h.read(RType::FileSet, "fs");
    let o = h.delete(RType::FileSet, "fs", &read.sha256);
    assert_eq!((o.status, o.referenced_by.clone()), (Status::Referenced, vec![("job".to_string(), "a".to_string())]));
    assert!(h.file("fileset", "fs.conf").exists());
    // `run job=… fileset=<name>` in a Job Run value names it too
    let raw = bareos_config::raw::parse_str("Job {\n Name = r\n Run = \"job=a fileset=orphan level=Full\"\n}\n", None).unwrap();
    assert_eq!(references(&raw, RType::FileSet, "orphan"), vec![("job".into(), "r".into())]);
}

/// A pool with a comment and a value the GUI changes (Volume Retention).
const POOL_SPARE: &str = "Pool {\n  Name = Spare\n  # kept as written\n  Pool Type = Backup\n  Volume Retention = 30 days\n  Label Format = \"Spare-\"\n}\n";

/// Pool in the CRUD: create, read, update with the follow-up for the
/// volumes, delete; REFERENCED by Job `Pool =`, Pool `Next Pool =` and a
/// Schedule `Run` override `Pool=`.
#[test]
fn pool_create_read_update_delete_and_referenced() {
    let h = harness("pool");
    let created = h.create(RType::Pool, POOL_SPARE);
    assert_eq!(created.status, Status::Ok, "{created:?}");
    assert_eq!(std::fs::read_to_string(h.file("pool", "Spare.conf")).unwrap(), POOL_SPARE);
    let read = h.read(RType::Pool, "Spare");
    assert_eq!((read.status, read.text.as_str()), (Status::Ok, POOL_SPARE));
    let changed = POOL_SPARE.replace("30 days", "3 months");
    let updated = h.update(RType::Pool, "Spare", &read.sha256, &changed);
    assert_eq!(updated.status, Status::Ok, "{updated:?}");
    assert_eq!(updated.followups, vec!["update volume allfrompool=Spare".to_string()]);
    assert!(std::fs::read_to_string(h.file("pool", "Spare.conf")).unwrap().contains("# kept as written"));
    let o = h.delete(RType::Pool, "Spare", &updated.sha256);
    assert_eq!(o.status, Status::Ok, "{o:?}");
    assert!(!h.file("pool", "Spare.conf").exists());
    assert_eq!(o.followups.len(), 1, "{o:?}");
    // JOB_A uses Pool = Full
    let read = h.read(RType::Pool, "Full");
    let o = h.delete(RType::Pool, "Full", &read.sha256);
    assert_eq!((o.status, o.referenced_by.clone()), (Status::Referenced, vec![("job".to_string(), "a".to_string())]));
    assert!(h.file("pool", "Full.conf").exists());
    // Next Pool of another pool, a Schedule Run override, a Job Run value
    let raw = bareos_config::raw::parse_str("Pool {\n Name = Daily\n Next Pool = Copy\n}\n\
        Schedule {\n Name = S\n Run = Level=Full Pool=Copy 1st sun at 23:05\n}\n\
        Job {\n Name = r\n Run = \"job=a fullpool=Copy level=Full\"\n}\n", None).unwrap();
    assert_eq!(references(&raw, RType::Pool, "Copy"),
        vec![("pool".into(), "Daily".into()), ("schedule".into(), "S".into()), ("job".into(), "r".into())]);
}

/// Storage in the CRUD: update keeps comments (gRPC directives of an osd),
/// REFERENCED by Job `Storage =`, Pool `Storage =` and a Run override.
#[test]
fn storage_update_and_referenced() {
    let h = harness("storage");
    let read = h.read(RType::Storage, "File");
    assert_eq!(read.status, Status::Ok, "{read:?}");
    let changed = read.text.replace("}\n", "  Grpc Enable = yes\n  Grpc Server Name = sd-01\n}\n");
    let updated = h.update(RType::Storage, "File", &read.sha256, &changed);
    assert_eq!(updated.status, Status::Ok, "{updated:?}");
    assert!(updated.followups.is_empty());
    let o = h.delete(RType::Storage, "File", &updated.sha256);
    assert_eq!((o.status, o.referenced_by.clone()), (Status::Referenced, vec![("job".to_string(), "a".to_string())]));
    let raw = bareos_config::raw::parse_str("Pool {\n Name = P\n Storage = Tape\n}\n\
        Schedule {\n Name = S\n Run = Level=Full Storage=Tape sun at 23:05\n}\n", None).unwrap();
    assert_eq!(references(&raw, RType::Storage, "Tape"), vec![("pool".into(), "P".into()), ("schedule".into(), "S".into())]);
}
