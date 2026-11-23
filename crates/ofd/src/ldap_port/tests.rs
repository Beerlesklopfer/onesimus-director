// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
use super::*;
use pb::broker_request::Request as Q;
use pb::broker_response::Response as A;
use std::collections::VecDeque;
use std::os::unix::fs::PermissionsExt;

/// A broker stand-in: records requests, plays back responses.
#[derive(Default)]
struct Fake {
    responses: VecDeque<Vec<u8>>,
    out: Vec<u8>,
    pending: Vec<u8>,
    seen: Vec<Q>,
}
impl Fake {
    fn answer(&mut self, r: A) -> &mut Self {
        let bytes = pb::BrokerResponse { response: Some(r) }.encode_to_vec();
        let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend(bytes);
        self.responses.push_back(frame);
        self
    }
}
impl Write for Fake {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let n = u32::from_be_bytes(self.pending[..4].try_into().unwrap()) as usize;
        let req = pb::BrokerRequest::decode(&self.pending[4..4 + n]).unwrap();
        self.seen.push(req.request.unwrap());
        self.pending.clear();
        if self.out.is_empty() {
            self.out = self.responses.pop_front().unwrap_or_default();
        }
        Ok(())
    }
}
impl Read for Fake {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = b.len().min(self.out.len());
        b[..n].copy_from_slice(&self.out[..n]);
        self.out.drain(..n);
        Ok(n)
    }
}

fn me() -> RunAs {
    // SAFETY: getuid/getgid have no preconditions.
    unsafe { RunAs { uid: libc::getuid(), gid: libc::getgid() } }
}
fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ofd-ldap-port-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
    d
}
fn db(number: i32, backend: &str, suffix: &str, directory: &str) -> pb::Database {
    pb::Database { number, backend: backend.into(), suffix: suffix.into(), directory: directory.into(), ..Default::default() }
}
fn info(d: &Path, kind: pb::ConfigKind, config: &Path) -> pb::InfoResponse {
    for sub in ["main", "second"] {
        std::fs::create_dir_all(d.join(sub)).unwrap();
    }
    pb::InfoResponse {
        server_version: "slapd 2.6.10".into(),
        kind: kind as i32,
        config_path: config.display().to_string(),
        databases: vec![
            db(-1, "frontend", "", ""),
            db(0, "config", "", ""),
            db(1, "mdb", "dc=example,dc=org", d.join("main").to_str().unwrap()),
            db(2, "mdb", "o=second,c=de", d.join("second").to_str().unwrap()),
            db(3, "ldap", "dc=example,dc=net", ""),
        ],
        config_files: vec![pb::ConfigFile { id: 0, path: "/etc/default/slapd".into(), size: 10, main: false }],
        tls_files: vec!["/etc/ldap/tls/server.key".into(), "/etc/ldap/tls/server.crt".into()],
        secret_kinds: vec!["rootpw".into()],
        config_selected: false,
    }
}
fn opts(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn only_mdb_databases_are_selected_and_patterns_follow_the_line() {
    let d = dir("select");
    let base = info(&d, pb::ConfigKind::SlapdD, Path::new("/etc/ldap/slapd.d"));
    let sel = |o: &[(&str, &str)]| {
        let mut i = base.clone();
        select(&mut i, &opts(o)).map(|s| s.into_iter().collect::<Vec<_>>())
    };
    assert_eq!(sel(&[]).unwrap(), ["dc=example,dc=org", "o=second,c=de"]);
    assert_eq!(sel(&[("databases", "DC=EXAMPLE,DC=ORG")]).unwrap(), ["dc=example,dc=org"]);
    assert_eq!(sel(&[("exclude", "o=*")]).unwrap(), ["dc=example,dc=org"]);
    assert_eq!(sel(&[("databases", "dc=example,dc=org;o=second,c=de")]).unwrap().len(), 2);
    assert!(sel(&[("databases", "dc=missing")]).unwrap_err().contains("does not exist"));
    assert!(sel(&[("databases", "dc=example,dc=net")]).unwrap_err().contains("without local storage"));
    let mut i = base.clone();
    select(&mut i, &opts(&[])).unwrap();
    let proxy = i.databases.iter().find(|x| x.backend == "ldap").unwrap();
    assert!(!proxy.selected && proxy.skipped_reason.contains("no local storage"));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn backup_requests_outside_the_selection_never_reach_the_broker() {
    let d = dir("backup");
    let config = PathBuf::from("/etc/ldap/slapd.d");
    let mut fake = Fake::default();
    fake.answer(A::Info(info(&d, pb::ConfigKind::SlapdD, &config)))
        .answer(A::DumpOpen(pb::DumpOpenResponse { handle: 7 }))
        .answer(A::DumpRead(pb::DumpReadResponse { data: b"dn: x\n".to_vec(), ..Default::default() }))
        .answer(A::DumpRead(pb::DumpReadResponse { eof: true, exit_ok: false, diagnostic: "slapcat: exit code 1\nx".into(), ..Default::default() }))
        .answer(A::DumpClose(pb::DumpCloseResponse {}));
    let covered = vec![b"/etc/ldap/tls/".to_vec()];
    let mut port = LdapPort::backup(fake, &opts(&[("exclude", "o=second,c=de")]), 1024, &ConfigPath::Dir(config.clone()), me(), &covered).unwrap();
    assert!(port.warnings.is_empty(), "the TLS directory is in the FileSet: {:?}", port.warnings);
    let call = |port: &mut LdapPort<Fake>, f: OpenldapFunction, req: Vec<u8>| port.call(BrokerFunction::Openldap(f), &req);

    // Info marks the selection.
    let shown = pb::InfoResponse::decode(call(&mut port, OpenldapFunction::Info, vec![]).unwrap().as_slice()).unwrap();
    let selected: Vec<&str> = shown.databases.iter().filter(|d| d.selected).map(|d| d.suffix.as_str()).collect();
    assert_eq!(selected, ["dc=example,dc=org"]);
    assert!(shown.config_selected);

    // Refused in the host.
    let open = |what| pb::DumpOpenRequest { what: Some(what) }.encode_to_vec();
    for (f, req) in [
        (OpenldapFunction::DumpOpen, open(pb::dump_open_request::What::Suffix("o=second,c=de".into()))),
        (OpenldapFunction::DumpOpen, open(pb::dump_open_request::What::Suffix("dc=example,dc=net".into()))),
        (OpenldapFunction::DumpOpen, open(pb::dump_open_request::What::Suffix("cn=monitor".into()))),
        (OpenldapFunction::ConfigFileOpen, pb::ConfigFileOpenRequest { id: 9 }.encode_to_vec()),
        (OpenldapFunction::DumpRead, pb::DumpReadRequest { handle: 7, max_bytes: 10 }.encode_to_vec()),
        (OpenldapFunction::LoadOpen, pb::LoadOpenRequest { what: Some(pb::load_open_request::What::Suffix("dc=example,dc=org".into())) }.encode_to_vec()),
    ] {
        assert!(call(&mut port, f, req).is_err(), "{f:?}");
    }
    assert!(port.call(BrokerFunction::Postgresql(onesimus_plugins::wasm::PostgresqlFunction::Info), &[]).is_err());

    // Allowed: open, read capped to the chunk, diagnostic bounded and clean, close.
    let o = pb::DumpOpenResponse::decode(call(&mut port, OpenldapFunction::DumpOpen,
        open(pb::dump_open_request::What::Suffix("dc=example,dc=org".into()))).unwrap().as_slice()).unwrap();
    assert_eq!(o.handle, 7);
    call(&mut port, OpenldapFunction::DumpRead, pb::DumpReadRequest { handle: 7, max_bytes: 999_999 }.encode_to_vec()).unwrap();
    let end = pb::DumpReadResponse::decode(call(&mut port, OpenldapFunction::DumpRead,
        pb::DumpReadRequest { handle: 7, max_bytes: 10 }.encode_to_vec()).unwrap().as_slice()).unwrap();
    assert!(end.eof && !end.exit_ok && !end.diagnostic.contains('\n'));
    call(&mut port, OpenldapFunction::DumpClose, pb::DumpCloseRequest { handle: 7 }.encode_to_vec()).unwrap();
    assert!(call(&mut port, OpenldapFunction::DumpClose, pb::DumpCloseRequest { handle: 7 }.encode_to_vec()).is_err());

    let seen = &port.transport.seen;
    assert_eq!(seen.len(), 5, "only allowed requests went out: {seen:?}");
    assert!(matches!(&seen[2], Q::DumpRead(r) if r.max_bytes == 1024), "read capped to the chunk");
    assert!(!port.serves(OPENLDAP_WRITE) && port.serves(OPENLDAP_READ));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn config_no_and_slapd_conf_limit_the_configuration() {
    let d = dir("config");
    let conf = PathBuf::from("/etc/ldap/slapd.conf");
    let mut fake = Fake::default();
    fake.answer(A::Info(info(&d, pb::ConfigKind::SlapdConf, &conf)));
    let mut port = LdapPort::backup(fake, &opts(&[]), 1024, &ConfigPath::File(conf.clone()), me(), &[]).unwrap();
    assert_eq!(port.warnings.len(), 2, "both TLS files are missing in the FileSet");
    assert!(port.warnings[0].contains("/etc/ldap/tls/server.key"));
    let cfg = pb::DumpOpenRequest { what: Some(pb::dump_open_request::What::Config(pb::ConfigDump {})) }.encode_to_vec();
    assert!(port.call(BrokerFunction::Openldap(OpenldapFunction::DumpOpen), &cfg).unwrap_err().contains("slapd.conf"));

    let mut fake = Fake::default();
    fake.answer(A::Info(info(&d, pb::ConfigKind::SlapdD, Path::new("/etc/ldap/slapd.d"))));
    let mut port = LdapPort::backup(fake, &opts(&[("config", "no")]), 1024, &ConfigPath::Dir("/etc/ldap/slapd.d".into()), me(), &[]).unwrap();
    assert!(port.call(BrokerFunction::Openldap(OpenldapFunction::DumpOpen), &cfg).unwrap_err().contains("config=no"));
    let file = pb::ConfigFileOpenRequest { id: 0 }.encode_to_vec();
    assert!(port.call(BrokerFunction::Openldap(OpenldapFunction::ConfigFileOpen), &file).is_err());
    assert_eq!(port.transport.seen.len(), 1);

    // The broker reporting another instance than ofd discovered.
    let mut fake = Fake::default();
    fake.answer(A::Info(info(&d, pb::ConfigKind::SlapdD, Path::new("/etc/ldap/other.d"))));
    assert!(LdapPort::backup(fake, &opts(&[]), 1024, &ConfigPath::Dir("/etc/ldap/slapd.d".into()), me(), &[]).is_err());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_database_directory_of_another_owner_is_refused() {
    let d = dir("owner");
    let mut fake = Fake::default();
    fake.answer(A::Info(info(&d, pb::ConfigKind::SlapdD, Path::new("/etc/ldap/slapd.d"))));
    let other = RunAs { uid: me().uid + 1, gid: me().gid };
    let e = LdapPort::backup(fake, &opts(&[]), 1024, &ConfigPath::Dir("/etc/ldap/slapd.d".into()), other, &[]).err().unwrap();
    assert!(e.contains("owned by uid"), "{e}");
    std::fs::remove_dir_all(&d).unwrap();
}

fn restore_port(d: &Path, target: Option<PathBuf>, fake: Fake) -> Result<LdapPort<Fake>, String> {
    LdapPort::restore(fake, 4096, &ConfigPath::Dir(d.join("slapd.d")), RestoreSettings {
        target,
        base: d.join("restore"),
        job_id: 42,
        run_as: me(),
        trust: Trust { owner: me().uid, top: d.to_path_buf() },
    })
}
fn production(d: &Path) -> pb::InfoResponse {
    std::fs::create_dir_all(d.join("slapd.d")).unwrap();
    info(d, pb::ConfigKind::SlapdD, &d.join("slapd.d"))
}

#[test]
fn a_restore_target_never_touches_the_running_instance() {
    let d = dir("target");
    let refused = |target: PathBuf| {
        let mut fake = Fake::default();
        fake.answer(A::Info(production(&d)));
        restore_port(&d, Some(target), fake).err().unwrap_or_else(|| panic!("accepted"))
    };
    // The production database directory, below it, above it, the configuration.
    assert!(refused(d.join("main")).contains("overlaps"));
    assert!(refused(d.join("main/restore")).contains("overlaps"));
    assert!(refused(d.clone()).contains("overlaps"));
    assert!(refused(d.join("slapd.d")).contains("overlaps"));
    assert!(refused(PathBuf::from("/")).contains("absolute") || refused(PathBuf::from("/")).contains("overlaps"));
    assert!(refused(d.join("x/../main")).contains("without . or .."));
    // A link, a file, a non-empty directory, an untrusted parent.
    std::os::unix::fs::symlink(d.join("main"), d.join("link")).unwrap();
    assert!(refused(d.join("link")).contains("symbolic link"));
    std::fs::write(d.join("file"), "x").unwrap();
    assert!(refused(d.join("file")).contains("not a directory"));
    std::fs::create_dir_all(d.join("full")).unwrap();
    std::fs::write(d.join("full/x"), "x").unwrap();
    assert!(refused(d.join("full")).contains("not empty"));
    std::fs::create_dir_all(d.join("open")).unwrap();
    std::fs::set_permissions(d.join("open"), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(refused(d.join("open/t")).contains("writable"));
    // The default <base>/<jobid> is fine and is created only on the first load.
    let mut fake = Fake::default();
    fake.answer(A::Info(production(&d)));
    let port = restore_port(&d, None, fake).unwrap();
    assert_eq!(port.target(), Some(d.join("restore/42").as_path()));
    assert!(!d.join("restore").exists());
    assert!(port.serves(OPENLDAP_WRITE));
    std::fs::remove_dir_all(&d).unwrap();
}

const CN_CONFIG: &str = "dn: cn=config\nobjectClass: olcGlobal\nolcPidFile: /run/slapd/slapd.pid\n\
olcArgsFile: /run/slapd/slapd.args\n\ndn: olcDatabase={0}config,cn=config\nolcDatabase: {0}config\n\n\
dn: olcDatabase={1}mdb,cn=config\nolcDatabase: {1}mdb\nolcDbDirectory: /var/lib/l\n dap\nolcSuffix: dc=example,dc=org\n\
olcRootPW: {SSHA}x\n\ndn: olcDatabase={2}mdb,cn=config\nolcDatabase: {2}mdb\nolcSuffix:: bz1zZWNvbmQsYz1kZQ==\n\n\
dn: olcDatabase={3}ldap,cn=config\nolcDatabase: {3}ldap\nolcSuffix: dc=example,dc=net\n";

#[test]
fn cn_config_is_rewritten_onto_the_target() {
    let target = Path::new("/var/lib/onesimus-ldap-restore/7");
    let (out, dbs) = rewrite_cn_config(CN_CONFIG.as_bytes(), target).unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("olcDbDirectory: /var/lib/onesimus-ldap-restore/7/db-1\n"), "{out}");
    assert!(out.contains("olcDbDirectory: /var/lib/onesimus-ldap-restore/7/db-2\n"), "a missing directory is added: {out}");
    assert!(out.contains("olcPidFile: /var/lib/onesimus-ldap-restore/7/run/slapd.pid"));
    assert!(out.contains("olcArgsFile: /var/lib/onesimus-ldap-restore/7/run/slapd.args"));
    assert!(!out.contains("/var/lib/ldap") && !out.contains("/run/slapd/"), "{out}");
    assert!(out.contains("olcRootPW: {SSHA}x"), "everything else is kept");
    let got: Vec<(i32, &str, &str)> = dbs.iter().map(|d| (d.number, d.backend.as_str(), d.suffix.as_str())).collect();
    assert_eq!(got, [(1, "mdb", "dc=example,dc=org"), (2, "mdb", "o=second,c=de")]);
    assert!(rewrite_cn_config(b"dn: cn=config\nolcDbDirectory: /x\n", target).is_err());
}

#[test]
fn slapd_conf_is_rewritten_onto_the_target() {
    let files: BTreeMap<PathBuf, Vec<u8>> = [
        ("/etc/ldap/slapd.conf", "include /etc/ldap/schema/core.schema\ninclude /etc/ldap/acl.conf\n\
include /usr/share/other.schema\npidfile /run/slapd/slapd.pid\nargsfile /run/slapd/slapd.args\n\
TLSCertificateKeyFile /etc/ldap/tls/server.key\ndatabase config\ndatabase mdb\nsuffix \"dc=example,dc=com\"\n\
directory /srv/ldap/main\nindex objectClass eq\ndatabase mdb\nsuffix \"o=zweit,c=de\"\ndatabase meta\nsuffix \"dc=remote,dc=test\"\n"),
        ("/etc/ldap/schema/core.schema", "attributetype ( 1 NAME 'a' )\n"),
        ("/etc/ldap/acl.conf", "access to *\n  by * read\n"),
        ("/etc/default/slapd", "SLAPD_CONF=/etc/ldap/slapd.conf\n"),
    ].into_iter().map(|(p, t)| (PathBuf::from(p), t.as_bytes().to_vec())).collect();
    let target = Path::new("/var/lib/onesimus-ldap-restore/8");
    let (out, dbs) = rewrite_slapd_conf(&files, Path::new("/etc/ldap/slapd.conf"), target).unwrap();
    let main = String::from_utf8(out[&target.join("conf/etc/ldap/slapd.conf")].clone()).unwrap();
    assert!(main.contains("include \"/var/lib/onesimus-ldap-restore/8/conf/etc/ldap/schema/core.schema\""), "{main}");
    assert!(main.contains("include /usr/share/other.schema"), "includes not restored stay: {main}");
    assert!(main.contains("pidfile \"/var/lib/onesimus-ldap-restore/8/run/slapd.pid\""));
    assert!(!main.contains("/srv/ldap/main") && !main.contains("/run/slapd/"), "{main}");
    assert_eq!(main.matches("directory \"/var/lib/onesimus-ldap-restore/8/db-").count(), 2, "{main}");
    assert!(main.contains("TLSCertificateKeyFile /etc/ldap/tls/server.key"));
    assert!(out.contains_key(&target.join("conf/etc/default/slapd")));
    assert_eq!(out[&target.join("conf/etc/ldap/acl.conf")], b"access to *\n  by * read\n");
    let got: Vec<(i32, &str, PathBuf)> = dbs.iter().map(|d| (d.number, d.suffix.as_str(), d.directory.clone())).collect();
    assert_eq!(got, [(1, "dc=example,dc=com", target.join("db-1")), (2, "o=zweit,c=de", target.join("db-2"))]);
    assert!(rewrite_slapd_conf(&files, Path::new("/etc/ldap/missing.conf"), target).is_err());
}

#[test]
fn an_online_restore_goes_into_the_new_target_and_a_failure_removes_it() {
    let d = dir("flow");
    let mut fake = Fake::default();
    fake.answer(A::Info(production(&d)))
        .answer(A::LoadOpen(pb::LoadOpenResponse { handle: 1 }))
        .answer(A::LoadWrite(pb::LoadWriteResponse {}))
        .answer(A::LoadClose(pb::LoadCloseResponse { ok: true, ..Default::default() }))
        .answer(A::LoadOpen(pb::LoadOpenResponse { handle: 2 }))
        .answer(A::LoadWrite(pb::LoadWriteResponse {}))
        .answer(A::LoadClose(pb::LoadCloseResponse { ok: true, ..Default::default() }));
    let mut port = restore_port(&d, None, fake).unwrap();
    let target = d.join("restore/42");
    let call = |port: &mut LdapPort<Fake>, f: OpenldapFunction, req: Vec<u8>| port.call(BrokerFunction::Openldap(f), &req);
    let open = |what| pb::LoadOpenRequest { what: Some(what) }.encode_to_vec();

    // Data before the configuration: refused.
    assert!(call(&mut port, OpenldapFunction::LoadOpen, open(pb::load_open_request::What::Suffix("dc=example,dc=org".into())))
        .unwrap_err().contains("configuration"));
    // Reading the instance is not part of a restore.
    assert!(call(&mut port, OpenldapFunction::DumpOpen,
        pb::DumpOpenRequest { what: Some(pb::dump_open_request::What::Suffix("dc=example,dc=org".into())) }.encode_to_vec()).is_err());

    let h = pb::LoadOpenResponse::decode(call(&mut port, OpenldapFunction::LoadOpen,
        open(pb::load_open_request::What::Config(pb::ConfigDump {}))).unwrap().as_slice()).unwrap().handle;
    assert!(target.join("run").is_dir());
    call(&mut port, OpenldapFunction::LoadWrite, pb::LoadWriteRequest { handle: h, data: CN_CONFIG.as_bytes().to_vec() }.encode_to_vec()).unwrap();
    let closed = pb::LoadCloseResponse::decode(call(&mut port, OpenldapFunction::LoadClose,
        pb::LoadCloseRequest { handle: h, abort: false }.encode_to_vec()).unwrap().as_slice()).unwrap();
    assert!(closed.ok);
    assert_eq!(closed.target, target.display().to_string());
    for sub in ["slapd.d", "db-1", "db-2"] {
        let m = std::fs::metadata(target.join(sub)).unwrap();
        assert!(m.is_dir() && m.mode() & 0o777 == 0o700, "{sub}");
    }
    // The broker got the rewritten LDIF against the target's slapd.d.
    let seen = &port.transport.seen;
    assert!(matches!(&seen[1], Q::LoadOpen(o) if o.against == Some(pb::broker_load_open::Against::ConfigDir(target.join("slapd.d").display().to_string()))));
    let Q::LoadWrite(w) = &seen[2] else { panic!("{seen:?}") };
    let sent = String::from_utf8_lossy(&w.data);
    assert!(sent.contains(&format!("olcDbDirectory: {}/db-1", target.display())) && !sent.contains("/var/lib/ldap"));

    // A suffix the restored configuration does not have, or a proxy: refused.
    for s in ["dc=other", "dc=example,dc=net"] {
        assert!(call(&mut port, OpenldapFunction::LoadOpen, open(pb::load_open_request::What::Suffix(s.into()))).is_err(), "{s}");
    }
    let h = pb::LoadOpenResponse::decode(call(&mut port, OpenldapFunction::LoadOpen,
        open(pb::load_open_request::What::Suffix("DC=example,dc=org".into()))).unwrap().as_slice()).unwrap().handle;
    call(&mut port, OpenldapFunction::LoadWrite, pb::LoadWriteRequest { handle: h, data: b"dn: dc=example,dc=org\n".to_vec() }.encode_to_vec()).unwrap();
    call(&mut port, OpenldapFunction::LoadClose, pb::LoadCloseRequest { handle: h, abort: false }.encode_to_vec()).unwrap();
    assert!(matches!(&port.transport.seen[4], Q::LoadOpen(o) if o.what == Some(pb::broker_load_open::What::Suffix("dc=example,dc=org".into()))));

    // Success keeps the target and names it; a failed job would remove it.
    let notes = port.finish(true);
    assert!(notes[0].contains("restored online into") && target.join("db-1").is_dir(), "{notes:?}");

    let mut fake = Fake::default();
    fake.answer(A::Info(production(&d)));
    let mut port = restore_port(&d, Some(d.join("restore/43")), fake).unwrap();
    let h = pb::LoadOpenResponse::decode(call(&mut port, OpenldapFunction::LoadOpen,
        open(pb::load_open_request::What::Config(pb::ConfigDump {}))).unwrap().as_slice()).unwrap().handle;
    assert!(d.join("restore/43").is_dir());
    assert!(call(&mut port, OpenldapFunction::LoadWrite, pb::LoadWriteRequest { handle: h, data: vec![b'x'; 5000] }.encode_to_vec())
        .unwrap_err().contains("chunk"));
    let notes = port.finish(true);
    assert!(notes[0].contains("removed") && !d.join("restore/43").exists(), "{notes:?}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_slapd_conf_restore_writes_the_rewritten_file_set() {
    let d = dir("conf");
    let mut fake = Fake::default();
    fake.answer(A::Info(production(&d)))
        .answer(A::LoadOpen(pb::LoadOpenResponse { handle: 5 }))
        .answer(A::LoadClose(pb::LoadCloseResponse { ok: false, diagnostic: "slapadd: exit code 1\n".into(), ..Default::default() }));
    let mut port = restore_port(&d, None, fake).unwrap();
    let target = d.join("restore/42");
    let call = |port: &mut LdapPort<Fake>, f: OpenldapFunction, req: Vec<u8>| port.call(BrokerFunction::Openldap(f), &req);
    for (path, main, text) in [("/etc/ldap/slapd.conf", true, "include /etc/ldap/schema/x.schema\ndatabase mdb\nsuffix \"dc=example,dc=com\"\ndirectory /srv/ldap/main\n"),
        ("/etc/ldap/schema/x.schema", false, "attributetype ( 1 NAME 'x' )\n")] {
        let what = pb::load_open_request::What::ConfigFile(pb::ConfigFileLoad { path: path.into(), main });
        let h = pb::LoadOpenResponse::decode(call(&mut port, OpenldapFunction::LoadOpen,
            pb::LoadOpenRequest { what: Some(what) }.encode_to_vec()).unwrap().as_slice()).unwrap().handle;
        call(&mut port, OpenldapFunction::LoadWrite, pb::LoadWriteRequest { handle: h, data: text.as_bytes().to_vec() }.encode_to_vec()).unwrap();
        call(&mut port, OpenldapFunction::LoadClose, pb::LoadCloseRequest { handle: h, abort: false }.encode_to_vec()).unwrap();
    }
    let bad = pb::load_open_request::What::ConfigFile(pb::ConfigFileLoad { path: "/etc/../shadow".into(), main: false });
    assert!(call(&mut port, OpenldapFunction::LoadOpen, pb::LoadOpenRequest { what: Some(bad) }.encode_to_vec()).is_err());
    let h = pb::LoadOpenResponse::decode(call(&mut port, OpenldapFunction::LoadOpen,
        pb::LoadOpenRequest { what: Some(pb::load_open_request::What::Suffix("dc=example,dc=com".into())) }.encode_to_vec())
        .unwrap().as_slice()).unwrap().handle;
    let main = std::fs::read_to_string(target.join("conf/etc/ldap/slapd.conf")).unwrap();
    assert!(main.contains(&format!("include \"{}/conf/etc/ldap/schema/x.schema\"", target.display())), "{main}");
    assert!(main.contains(&format!("directory \"{}/db-1\"", target.display())) && !main.contains("/srv/ldap"));
    assert_eq!(std::fs::metadata(target.join("conf/etc/ldap/slapd.conf")).unwrap().mode() & 0o777, 0o600);
    assert!(matches!(&port.transport.seen[1], Q::LoadOpen(o)
        if o.against == Some(pb::broker_load_open::Against::ConfigFile(target.join("conf/etc/ldap/slapd.conf").display().to_string()))));
    // slapadd fails: the answer says so, cleanly; the target goes at the end.
    let closed = pb::LoadCloseResponse::decode(call(&mut port, OpenldapFunction::LoadClose,
        pb::LoadCloseRequest { handle: h, abort: false }.encode_to_vec()).unwrap().as_slice()).unwrap();
    assert!(!closed.ok && !closed.diagnostic.contains('\n'));
    port.finish(true);
    assert!(!target.exists());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn an_openldap_line_with_a_secret_is_refused_not_masked() {
    let manifest = onesimus_plugins::Manifest {
        schema_version: 3,
        plugin_id: "test.openldap".into(),
        version: "1".into(),
        api_version: 1,
        artifact_sha256: String::new(),
        signing_key_id: String::new(),
        not_before: 0,
        not_after: u64::MAX,
        capabilities: vec!["openldap.read".into()],
        target: "wasm32-wasip2".into(),
        native: Default::default(),
        suite: None,
    };
    for line in ["openldap:password=x", "openldap:bind_pw=x", "openldap:instance=main:credentials=c"] {
        let line = crate::plugin::parse_line(line).unwrap();
        let e = prepare(&manifest, &line, Path::new("/nonexistent"), "t", 1024, 0, Path::new("/"),
            &crate::plugin::BrokerContext::backup()).err().unwrap();
        assert!(matches!(&e, PluginError::Syntax(why) if why.contains("no passwords")), "{e:?}");
        assert!(!e.to_string().contains("=x"), "the value is not shown: {e}");
    }
}

#[test]
fn file_set_coverage() {
    let names = vec![b"/etc/ldap/tls".to_vec(), b"/root/key.pem".to_vec()];
    assert!(covered("/etc/ldap/tls/server.key", &names));
    assert!(covered("/root/key.pem", &names));
    assert!(!covered("/etc/ldap/tlsx/server.key", &names));
    assert!(!covered("/etc/ldap/server.key", &names));
}
