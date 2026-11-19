//! End-to-end tests of the OpenLDAP plugin against a real slapd 2.6, for
//! both configuration kinds, in throwaway Podman containers
//! (docs/plans/OPENLDAP-PLUGIN.md, "Tests").
//!
//! Run (needs podman, network for the image build, and the built parts):
//!
//! ```sh
//! (cd plugins/openldap && CARGO_TARGET_DIR=../../target/plugins cargo +1.99.1 build --release --target wasm32-wasip2)
//! cargo build -p onesimus-ldap-broker
//! cargo test -p ofd -- --ignored ldap_e2e::podman_both_configurations --nocapture
//! ```
//!
//! `podman_both_configurations` runs tests/openldap/run-in-podman.sh, which
//! builds the image, sets up `ldaptest-olc` (slapd.d) and `ldaptest-conf`
//! (slapd.conf) and runs the `inside` tests of this very test binary in each
//! container as root, with slapd running as `openldap`. Outside a container
//! (no ONESIMUS_LDAP_E2E) the `inside` tests return at once.
//!
//! The volume, the restore and the job messages go through ofd's real
//! backup and restore code (BNET stream as the SD sees it); Director and SD
//! themselves are not part of it.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use onesimus_proto_rs::onesimus::filed::plugin::v1 as pb;

use crate::ldap_instance::ConfigPath;
use crate::plugin::guest::{as_read_back, Duplex};
use crate::plugin::{parse_line, prepare_broker_for, BrokerContext, PluginLine, Resolved, Session};
use onesimus_plugins::wasm::RuntimeLimits;

const TRIPLET: &str = "test-triplet";
const CHUNK: u32 = 64 * 1024;
const ONLINE_CHUNK: u32 = 256 * 1024;

/// Runs the inside tests in two containers (see the module documentation).
#[test]
#[ignore = "needs podman, network and the built openldap guest and broker"]
fn podman_both_configurations() {
    if Command::new("podman").arg("--version").output().map(|o| !o.status.success()).unwrap_or(true) {
        eprintln!("podman is not installed; skipped");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let wasm = root.join("target/plugins/wasm32-wasip2/release/onesimus_plugin_openldap.wasm");
    let broker = root.join("target/debug/onesimus-ldap-broker");
    assert!(wasm.is_file(), "build plugins/openldap first: {}", wasm.display());
    assert!(broker.is_file(), "cargo build -p onesimus-ldap-broker first");
    let me = std::env::current_exe().unwrap();
    let status = Command::new(root.join("tests/openldap/run-in-podman.sh")).arg(&me).arg(&wasm).arg(&broker).status().unwrap();
    assert!(status.success(), "container tests failed ({status})");
}

fn sh(script: &str) -> (bool, String, String) {
    let out = Command::new("sh").arg("-c").arg(script).output().expect("sh");
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}
fn sh_ok(script: &str) -> String {
    let (ok, out, err) = sh(script);
    assert!(ok, "{script}: {err}{out}");
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Olc,
    Conf,
}

struct Setup {
    kind: Kind,
    dir: PathBuf,
    plugins: PathBuf,
    resolved: Resolved,
    line: PluginLine,
}

impl Setup {
    /// None outside the test container.
    fn new(tag: &str) -> Option<Setup> {
        let kind = match std::env::var("ONESIMUS_LDAP_E2E").ok()?.as_str() {
            "olc" => Kind::Olc,
            "conf" => Kind::Conf,
            other => panic!("ONESIMUS_LDAP_E2E={other}"),
        };
        use std::os::unix::fs::PermissionsExt;
        let wasm = std::fs::read(std::env::var("ONESIMUS_LDAP_WASM").unwrap()).unwrap();
        let broker_bin = PathBuf::from(std::env::var("ONESIMUS_LDAP_BROKER").unwrap());
        let dir = PathBuf::from(format!("/root/ldaptest-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        let plugins = dir.join("plugins");
        std::fs::create_dir_all(plugins.join("openldap")).unwrap();
        for p in [&dir, &plugins, &plugins.join("openldap")] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let broker_path = plugins.join("openldap/onesimus-ldap-broker");
        std::fs::copy(&broker_bin, &broker_path).unwrap();
        std::fs::set_permissions(&broker_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sha: String = {
            use sha2::Digest;
            sha2::Sha256::digest(std::fs::read(&broker_path).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
        };
        let native = [(TRIPLET.to_string(), vec![onesimus_plugins::NativePart { path: "openldap/onesimus-ldap-broker".into(), sha256: sha }])].into();
        let resolved = crate::plugin::testkit::signed_artifact_with(&plugins, "openldap", wasm,
            &["file.read", "file.write", "openldap.read", "openldap.write"], native);
        let line = parse_line("openldap:instance=main").unwrap();
        Some(Setup { kind, dir, plugins, resolved, line })
    }

    fn suffix(&self) -> &'static str {
        match self.kind {
            Kind::Olc => "dc=example,dc=org",
            Kind::Conf => "dc=example,dc=com",
        }
    }
    fn second(&self) -> &'static str {
        match self.kind {
            Kind::Olc => "o=second,c=de",
            Kind::Conf => "o=zweit,c=de",
        }
    }
    fn config(&self) -> ConfigPath {
        match self.kind {
            Kind::Olc => ConfigPath::Dir("/etc/ldap/slapd.d".into()),
            Kind::Conf => ConfigPath::File("/etc/ldap/slapd.conf".into()),
        }
    }
    fn config_flag(&self) -> String {
        match self.config() {
            ConfigPath::Dir(d) => format!("-F {}", d.display()),
            ConfigPath::File(f) => format!("-f {}", f.display()),
        }
    }
    fn data_mdb(&self) -> PathBuf {
        match self.kind {
            Kind::Olc => "/var/lib/ldap/data.mdb".into(),
            Kind::Conf => "/srv/ldap/main/data.mdb".into(),
        }
    }
    fn admin(&self) -> String {
        format!("-x -D cn=admin,{} -w admin123 -H ldapi:///", self.suffix())
    }
    /// Secrets of the instance: on the volume, never in metadata or messages.
    fn secrets(&self) -> Vec<&'static str> {
        // The rootpw hash plain (slapd.conf) and as LDIF base64 (cn=config), userPassword likewise.
        vec!["mzk4xLlCA0uZhM9pGxDsMizeHI7GpkgQ", "e1NTSEF9bXprNHhMbENBMHVaaE05cEd4RHNNaXplSEk3R3BrZ1E=",
            "annaPass1", "YW5uYVBhc3Mx", self.plain_secret()]
    }
    /// A clear-text proxy credential of the configuration (credentials=).
    fn plain_secret(&self) -> &'static str {
        match self.kind {
            Kind::Olc => "admin123",
            Kind::Conf => "MetaSecret-4711",
        }
    }

    fn session(&self, op: pb::JobOperation, job_id: u64, line: &PluginLine, covered: &[Vec<u8>], chunk: u32)
        -> Result<Session, crate::plugin::PluginError> {
        let context = BrokerContext { operation: op, job_id, covered };
        let prepared = prepare_broker_for(&self.resolved, line, &self.plugins, TRIPLET, chunk, 0, &self.dir, &context)?
            .expect("a broker");
        let job = pb::JobContext { job_id, job_name: "ldap".into(), operation: op as i32 };
        let runtime = RuntimeLimits { max_chunk_bytes: chunk, ..RuntimeLimits::default() };
        Session::open_with(&self.resolved, line, job, 0, runtime, Some(prepared))
    }

    /// A backup into a BNET buffer: (ok, volume, messages incl. warnings).
    fn backup(&self, job_id: u64, covered: &[Vec<u8>]) -> (i32, Vec<u8>, Vec<String>) {
        Self::backup_with(self.session(pb::JobOperation::Backup, job_id, &self.line, covered, CHUNK).unwrap())
    }

    /// The backup of an opened session (the slapcat runs here).
    fn backup_with(session: Session) -> (i32, Vec<u8>, Vec<String>) {
        let mut sent: Vec<String> = session.warnings.clone();
        let canceled = AtomicBool::new(false);
        let config = crate::config::FdConfig::parse("Director { Name = d; Password = p }").unwrap();
        let mut sd = bareos_bnet::BnetStream::new(Cursor::new(Vec::new()));
        let mut msgs = crate::jmsg::JobMsgs::new("test-fd");
        let mut counters = crate::restore::Counters::default();
        let mut flush = |_: &mut crate::jmsg::JobMsgs| {};
        let shown = crate::secrets::mask_plugin_secrets("openldap:instance=main");
        let ok = crate::backup::Backup {
            sd: crate::backup::SdSender { sd: &mut sd, msgs: &mut msgs, counters: &mut counters, canceled: &canceled, progress: None },
            env: crate::backup::JobEnv { canceled: &canceled, accurate: None, level_full: true, rerunning: false, base_size: 0 },
            flush: &mut flush, buf_size: CHUNK as usize, workers: 0, acl_errors: 0, xattr_errors: 0, xattr_disabled: false,
            config: &config, plugin_index: 0,
        }
        .run_plugin(&crate::find::FfPkt::default(), &shown, session);
        sent.extend(msgs.take_queued());
        (ok, sd.into_inner().into_inner(), sent)
    }

    /// Restore of `volume`: to files below `where_` (no plugin code), or
    /// online with these plugin options. (job status, messages)
    fn restore(&self, volume: &[u8], job_id: u32, where_: Option<&Path>, options: &[String]) -> (u8, Vec<String>) {
        let canceled = AtomicBool::new(false);
        let mut rsd = bareos_bnet::BnetStream::new(Duplex(Cursor::new(as_read_back(volume))));
        let where_bytes = where_.map(|w| w.as_os_str().as_encoded_bytes().to_vec()).unwrap_or_default();
        let params = crate::restore::RestoreParams { where_: crate::attribs::Where::Prefix(where_bytes), replace: b'a', prefix_links: false };
        let mut msgs = crate::jmsg::JobMsgs::new("test-fd");
        msgs.job_id = job_id;
        let mut sent = Vec::new();
        let config = crate::config::FdConfig::parse("Director { Name = d; Password = p }").unwrap();
        if !options.is_empty() {
            let resolved = Resolved { bundle: self.resolved.bundle.clone(), policy: self.resolved.policy.clone() };
            let (plugins, dir) = (self.plugins.clone(), self.dir.clone());
            crate::restore::test_hook::set(Box::new(move |line, job, index| {
                let context = BrokerContext { operation: pb::JobOperation::Restore, job_id: job.job_id, covered: &[] };
                let prepared = prepare_broker_for(&resolved, line, &plugins, TRIPLET, ONLINE_CHUNK, 0, &dir, &context)?;
                let runtime = RuntimeLimits { max_chunk_bytes: ONLINE_CHUNK, ..RuntimeLimits::default() };
                Session::open_with(&resolved, line, job, index, runtime, prepared)
            }));
        }
        crate::restore::Restore {
            params: &params, msgs: &mut msgs, counters: &mut crate::restore::Counters::default(),
            create: &mut crate::create_file::CreateState::default(), canceled: &canceled,
            config: if options.is_empty() { None } else { Some(&config) }, plugin_options: options,
        }
        .run(&mut rsd, &mut |m: &mut crate::jmsg::JobMsgs| sent.extend(m.take_queued()));
        let _ = crate::restore::test_hook::take();
        (msgs.status(), sent)
    }

    fn slapcat(&self, args: &str) -> String {
        sh_ok(&format!("runuser -u openldap -- slapcat {} {args} -o ldif_wrap=no", self.config_flag()))
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn sha256(path: &Path) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(std::fs::read(path).unwrap()).iter().map(|b| format!("{b:02x}")).collect()
}

/// The frames of an FD->SD stream by stream type (header "<fi> <stream> 0").
fn frames_by_stream(volume: &[u8]) -> Vec<(i32, Vec<u8>)> {
    let mut out = Vec::new();
    let mut b = volume;
    let mut stream: Option<i32> = None;
    while b.len() >= 4 {
        let n = i32::from_be_bytes(b[..4].try_into().unwrap());
        b = &b[4..];
        if n < 0 {
            stream = None;
            continue;
        }
        let data = b[..n as usize].to_vec();
        b = &b[n as usize..];
        match stream {
            None => stream = String::from_utf8_lossy(&data).split(' ').nth(1).and_then(|s| s.parse().ok()),
            Some(s) => out.push((s, data)),
        }
    }
    out
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// LDIF as sorted entries of sorted lines, without `skip` attributes.
fn normalized(ldif: &str, skip: &[&str]) -> Vec<Vec<String>> {
    let mut entries: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in ldif.lines() {
        if line.is_empty() {
            if !current.is_empty() {
                entries.push(std::mem::take(&mut current));
            }
        } else if let Some(more) = line.strip_prefix(' ') {
            if let Some(last) = current.last_mut() {
                last.push_str(more);
            }
        } else if !line.starts_with('#') {
            current.push(line.to_string());
        }
    }
    if !current.is_empty() {
        entries.push(current);
    }
    for e in &mut entries {
        e.retain(|l| !skip.iter().any(|s| l.split(':').next().is_some_and(|a| a.eq_ignore_ascii_case(s))));
        e.sort();
    }
    entries.sort();
    entries
}

fn ldapi(socket: &Path) -> String {
    let enc: String = socket.display().to_string().bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("ldapi://{enc}")
}

fn openldap_uid() -> u32 {
    sh_ok("id -u openldap").trim().parse().unwrap()
}

mod inside {
    use super::*;

    /// Backup without load, restore to files without plugin code, the
    /// content against direct slapcat, the leak test, the online restore
    /// into a new target with a second slapd, the refused production target
    /// and the filtered slapadd diagnostic.
    #[test]
    #[ignore = "runs inside the ldaptest containers (see ldap_e2e)"]
    fn a_backup_restore_and_no_leak() {
        let Some(s) = Setup::new("main") else { return };
        let mdb_before = sha256(&s.data_mdb());

        // The broker runs as openldap, never as root.
        let probe = s.session(pb::JobOperation::Backup, 100, &s.line, &[], CHUNK).unwrap();
        let brokers = sh_ok("for p in /proc/[0-9]*; do c=$(cat $p/comm 2>/dev/null); case $c in onesimus-ldap*) awk '/^Uid:/{print $2, $3}' $p/status;; esac; done");
        let uid = openldap_uid();
        assert_eq!(brokers.trim(), format!("{uid} {uid}"), "broker uids: {brokers}");
        assert_ne!(uid, 0);
        probe.finish(true).unwrap();

        // Backup (quiet instance).
        let (ok, volume, sent) = s.backup(101, &[]);
        assert_eq!(ok, 1, "{sent:?}");
        match s.kind {
            Kind::Conf => {
                assert!(sent.iter().any(|m| m.contains("/etc/ldap/tls/server.key") && m.contains("not in the FileSet")), "{sent:?}");
                let (_, _, covered) = s.backup(102, &[b"/etc/ldap/tls".to_vec()]);
                assert!(!covered.iter().any(|m| m.contains("not in the FileSet")), "{covered:?}");
            }
            Kind::Olc => assert!(!sent.iter().any(|m| m.contains("TLS")), "{sent:?}"),
        }

        // Leak test: secrets are on the volume (file data) only.
        let frames = frames_by_stream(&volume);
        assert!(contains(&volume, s.plain_secret()), "positive control: the proxy credential is backed up");
        assert!(contains(&volume, "mzk4xLlCA0uZhM9pGxDsMizeHI7GpkgQ") || contains(&volume, "e1NTSEF9bXprNHhMbENBMHVaaE05cEd4RHNNaXplSEk3R3BrZ1E="),
            "positive control: the rootpw hash is backed up");
        for secret in s.secrets() {
            for (stream, data) in &frames {
                if *stream != crate::streams::STREAM_FILE_DATA {
                    assert!(!contains(data, secret), "{secret} in stream {stream} (catalog metadata)");
                }
            }
            assert!(!sent.iter().any(|m| m.contains(secret)), "{secret} in the job log: {sent:?}");
        }

        // Restore to files: no plugin code (config None), mode 0600.
        let target = s.dir.join("files");
        let (status, rsent) = s.restore(&volume, 103, Some(&target), &[]);
        assert_eq!(status, crate::jmsg::JS_TERMINATED, "{rsent:?}");
        for secret in s.secrets() {
            assert!(!rsent.iter().any(|m| m.contains(secret)), "{secret} in the restore log");
        }
        let base = target.join("@openldap/main");
        let info = std::fs::read_to_string(base.join("_info.json")).unwrap();
        let data = std::fs::read_to_string(base.join(format!("data/{}.ldif", s.suffix()))).unwrap();
        use std::os::unix::fs::MetadataExt;
        assert_eq!(std::fs::metadata(base.join(format!("data/{}.ldif", s.suffix()))).unwrap().mode() & 0o777, 0o600);
        assert_eq!(data, s.slapcat(&format!("-b {}", s.suffix())), "the data LDIF is what slapcat gives");
        assert_eq!(std::fs::read_to_string(base.join(format!("data/{}.ldif", s.second()))).unwrap(), s.slapcat(&format!("-b {}", s.second())));
        assert!(data.contains("dn: uid=anna,ou=people,") && data.split("\n\n").any(|e| e.contains("uid=anna,") && e.contains("userPassword")),
            "password hashes are in the backup");
        assert!(info.contains("\"contains_secrets\":[") && info.contains("\"rootpw\"") && !info.contains("mzk4x"), "{info}");
        assert!(info.contains("no local storage"), "the proxy database is listed as not backed up: {info}");
        match s.kind {
            Kind::Olc => {
                assert!(info.contains("\"kind\":\"slapd.d\""), "{info}");
                assert_eq!(std::fs::read_to_string(base.join("config/cn=config.ldif")).unwrap(), s.slapcat("-n 0"));
                assert!(info.contains("idassert-credentials"), "{info}");
            }
            Kind::Conf => {
                assert!(info.contains("\"kind\":\"slapd.conf\""), "{info}");
                for f in ["etc/ldap/slapd.conf", "etc/ldap/schema/core.schema", "etc/ldap/schema/local/onesimus.schema", "etc/default/slapd"] {
                    let restored = std::fs::read(base.join("config/files").join(f)).unwrap();
                    assert_eq!(restored, std::fs::read(Path::new("/").join(f)).unwrap(), "{f}");
                }
                assert!(!base.join("config/files/etc/ldap/tls").exists(), "TLS files are not plugin content");
                assert!(info.contains("/etc/ldap/tls/server.key"), "{info}");
            }
        }

        // Online restore into a new target; a second slapd serves it.
        let (status, osent) = s.restore(&volume, 104, None, &["openldap:restore_mode=online".to_string()]);
        assert_eq!(status, crate::jmsg::JS_TERMINATED, "{osent:?}");
        assert!(osent.iter().any(|m| m.contains("restored online into /var/lib/onesimus-ldap-restore/104")), "{osent:?}");
        for secret in s.secrets() {
            assert!(!osent.iter().any(|m| m.contains(secret)), "{secret} in the online restore log");
        }
        let t = PathBuf::from("/var/lib/onesimus-ldap-restore/104");
        let socket = t.join("run/ldapi");
        let flag = match s.kind {
            Kind::Olc => format!("-F {}", t.join("slapd.d").display()),
            Kind::Conf => format!("-f {}", t.join("conf/etc/ldap/slapd.conf").display()),
        };
        sh_ok(&format!("slapd {flag} -h '{}' -u openldap -g openldap", ldapi(&socket)));
        let url = ldapi(&socket);
        let mut up = false;
        for _ in 0..50 {
            if sh(&format!("ldapsearch -x -H '{url}' -b '' -s base >/dev/null 2>&1")).0 {
                up = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        assert!(up, "the second slapd did not start on the restored target");
        let who = sh_ok(&format!("ldapwhoami -x -H '{url}' -D uid=anna,ou=people,{} -w annaPass1", s.suffix()));
        assert!(who.contains("uid=anna"), "login with the old password: {who}");
        let member = sh_ok(&format!("ldapsearch -LLL -x -H '{url}' -D uid=anna,ou=people,{0} -w annaPass1 -b uid=anna,ou=people,{0} -s base memberOf", s.suffix()));
        assert!(member.contains("cn=admins,ou=groups"), "memberOf survived: {member}");
        let restored_flag = flag.clone();
        for suffix in [s.suffix(), s.second()] {
            let restored = sh_ok(&format!("runuser -u openldap -- slapcat {restored_flag} -b {suffix} -o ldif_wrap=no"));
            assert_eq!(normalized(&restored, &[]), normalized(&s.slapcat(&format!("-b {suffix}")), &[]), "{suffix}");
        }
        match s.kind {
            Kind::Olc => {
                let skip = ["olcDbDirectory", "olcPidFile", "olcArgsFile"];
                let restored = sh_ok(&format!("runuser -u openldap -- slapcat {restored_flag} -n 0 -o ldif_wrap=no"));
                assert_eq!(normalized(&restored, &skip), normalized(&s.slapcat("-n 0"), &skip), "cn=config incl. overlays and ACLs");
                for overlay in ["memberof", "ppolicy", "syncprov"] {
                    assert!(restored.contains(&format!("olcOverlay: {{")) && restored.contains(overlay), "{overlay}");
                }
                assert!(restored.contains(&format!("olcDbDirectory: {}/db-1", t.display())));
            }
            Kind::Conf => {
                let conf = std::fs::read_to_string(t.join("conf/etc/ldap/slapd.conf")).unwrap();
                assert!(conf.contains("overlay memberof") && conf.contains("access to attrs=userPassword"), "{conf}");
                assert!(conf.contains(&format!("include \"{}/conf/etc/ldap/schema/local/onesimus.schema\"", t.display())));
                assert!(!conf.contains("/srv/ldap/"), "{conf}");
            }
        }
        sh_ok(&format!("kill $(cat {})", t.join("run/slapd.pid").display()));
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(sha256(&s.data_mdb()), mdb_before, "the production database was not touched");

        // An online restore aimed at the production directory is refused.
        let prod_dir = s.data_mdb().parent().unwrap().to_path_buf();
        let (status, bad) = s.restore(&volume, 105, None, &[format!("openldap:restore_mode=online:target={}", prod_dir.display())]);
        assert_ne!(status, crate::jmsg::JS_TERMINATED, "{bad:?}");
        assert!(bad.iter().any(|m| m.contains("overlaps")), "{bad:?}");
        assert!(!Path::new("/var/lib/onesimus-ldap-restore/105").exists());
        assert_eq!(sha256(&s.data_mdb()), mdb_before, "the production database was never overwritten");

        // slapadd fails on a broken LDIF holding a secret: the diagnostic is filtered.
        let line = parse_line("openldap:instance=main:restore_mode=online:target=/var/lib/onesimus-ldap-restore/bad").unwrap();
        let context = BrokerContext { operation: pb::JobOperation::Restore, job_id: 106, covered: &[] };
        let prepared = prepare_broker_for(&s.resolved, &line, &s.plugins, TRIPLET, ONLINE_CHUNK, 0, &s.dir, &context).unwrap().unwrap();
        let port = prepared.handle.clone();
        let call = |f: onesimus_plugins::wasm::OpenldapFunction, msg: Vec<u8>| -> Result<Vec<u8>, String> {
            port.0.lock().unwrap().call(onesimus_plugins::wasm::BrokerFunction::Openldap(f), &msg)
        };
        use onesimus_plugins::wasm::OpenldapFunction as F;
        use onesimus_proto_rs::onesimus::filed::broker::openldap::v1 as lpb;
        use prost::Message;
        let load = |what: lpb::load_open_request::What, data: &[u8]| -> lpb::LoadCloseResponse {
            let h = lpb::LoadOpenResponse::decode(call(F::LoadOpen, lpb::LoadOpenRequest { what: Some(what) }.encode_to_vec()).unwrap().as_slice()).unwrap().handle;
            for block in data.chunks(ONLINE_CHUNK as usize) {
                call(F::LoadWrite, lpb::LoadWriteRequest { handle: h, data: block.to_vec() }.encode_to_vec()).unwrap();
            }
            lpb::LoadCloseResponse::decode(call(F::LoadClose, lpb::LoadCloseRequest { handle: h, abort: false }.encode_to_vec()).unwrap().as_slice()).unwrap()
        };
        match s.kind {
            Kind::Olc => {
                let config = std::fs::read(base.join("config/cn=config.ldif")).unwrap();
                assert!(load(lpb::load_open_request::What::Config(lpb::ConfigDump {}), &config).ok);
            }
            Kind::Conf => {
                for (f, main) in [("etc/ldap/slapd.conf", true), ("etc/ldap/schema/core.schema", false), ("etc/ldap/schema/cosine.schema", false),
                    ("etc/ldap/schema/nis.schema", false), ("etc/ldap/schema/inetorgperson.schema", false), ("etc/ldap/schema/local/onesimus.schema", false)] {
                    let bytes = std::fs::read(base.join("config/files").join(f)).unwrap();
                    let what = lpb::load_open_request::What::ConfigFile(lpb::ConfigFileLoad { path: format!("/{f}"), main });
                    assert!(load(what, &bytes).ok, "{f}");
                }
            }
        }
        let broken = format!("dn: {0}\nobjectClass: dcObject\nobjectClass: organization\no: x\ndc: example\n\n\
dn: uid=geheim,{0}\nobjectClass: inetOrgPerson\nuid: geheim\nuserPassword: Geheim-0815\nnoSuchAttribute: Geheim-0815\n\n", s.suffix());
        let closed = load(lpb::load_open_request::What::Suffix(s.suffix().into()), broken.as_bytes());
        assert!(!closed.ok, "slapadd must refuse the broken LDIF");
        assert!(closed.diagnostic.contains("slapadd") && !closed.diagnostic.contains("Geheim") && !closed.diagnostic.contains("uid=geheim"),
            "filtered diagnostic: {}", closed.diagnostic);
        let notes = port.0.lock().unwrap().finish(false);
        assert!(notes.iter().any(|n| n.contains("removed")), "{notes:?}");
        assert!(!Path::new("/var/lib/onesimus-ldap-restore/bad").exists());
        drop(prepared);
    }

    /// slapcat is a snapshot: under constant writes (add q_i, delete p_i)
    /// the backup holds N or N+1 of them, never a mix of two states.
    #[test]
    #[ignore = "runs inside the ldaptest containers (see ldap_e2e)"]
    fn b_backup_under_write_load_is_consistent() {
        let Some(s) = Setup::new("load") else { return };
        const N: usize = 20000;
        let admin = s.admin();
        let suffix = s.suffix();
        let mut add = String::new();
        for i in 0..N {
            add.push_str(&format!("dn: cn=p{i},ou=policies,{suffix}\nobjectClass: device\ncn: p{i}\n\n"));
        }
        std::fs::write(s.dir.join("p.ldif"), add).unwrap();
        sh_ok(&format!("ldapadd {admin} -f {} >/dev/null", s.dir.join("p.ldif").display()));
        let mut churn = String::new();
        for i in 0..N {
            churn.push_str(&format!("dn: cn=q{i},ou=groups,{suffix}\nchangetype: add\nobjectClass: device\ncn: q{i}\n\n\
dn: cn=p{i},ou=policies,{suffix}\nchangetype: delete\n\n"));
        }
        std::fs::write(s.dir.join("churn.ldif"), churn).unwrap();
        // Plugin and broker are ready before the writes start: the dumps run in backup_with.
        let session = s.session(pb::JobOperation::Backup, 201, &s.line, &[b"/etc/ldap/tls".to_vec()], CHUNK).unwrap();
        let mut load = Command::new("sh").arg("-c")
            .arg(format!("ldapmodify {admin} -f {} >/dev/null", s.dir.join("churn.ldif").display())).spawn().unwrap();
        // Back up once the writes are well under way (q500 exists).
        for _ in 0..200 {
            if sh(&format!("ldapsearch {admin} -b cn=q500,ou=groups,{suffix} -s base dn >/dev/null 2>&1")).0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (ok, volume, sent) = Setup::backup_with(session);
        let still_running = load.try_wait().unwrap().is_none();
        assert!(load.wait().unwrap().success());
        assert_eq!(ok, 1, "{sent:?}");
        let target = s.dir.join("files");
        let (status, rsent) = s.restore(&volume, 202, Some(&target), &[]);
        assert_eq!(status, crate::jmsg::JS_TERMINATED, "{rsent:?}");
        let data = std::fs::read_to_string(target.join(format!("@openldap/main/data/{suffix}.ldif"))).unwrap();
        let p = data.lines().filter(|l| l.starts_with("dn: cn=p") && l.contains(",ou=policies,")).count();
        let q = data.lines().filter(|l| l.starts_with("dn: cn=q") && l.contains(",ou=groups,")).count();
        eprintln!("under load ({}): p={p} q={q} p+q={}", if still_running { "writes were running" } else { "writes had ended" }, p + q);
        assert!(p + q == N || p + q == N + 1, "not a snapshot: p={p} q={q}");
        assert!(still_running && q > 0 && p > 0, "the backup must overlap the writes (p={p} q={q})");
        // Clean up for the other tests.
        let mut del = String::new();
        for i in 0..N {
            del.push_str(&format!("dn: cn=q{i},ou=groups,{suffix}\nchangetype: delete\n\n"));
        }
        std::fs::write(s.dir.join("del.ldif"), del).unwrap();
        sh_ok(&format!("ldapmodify -c {admin} -f {} >/dev/null 2>&1 || true", s.dir.join("del.ldif").display()));
    }

    /// slapd started as root (no -u): the plugin refuses before any broker runs.
    #[test]
    #[ignore = "runs inside the ldaptest containers (see ldap_e2e)"]
    fn z_slapd_as_root_is_refused() {
        let Some(s) = Setup::new("root") else { return };
        sh_ok("ldap-ctl stop");
        sh_ok(&format!("mkdir -p /run/slapd && slapd {} -h ldapi:///", s.config_flag()));
        std::thread::sleep(std::time::Duration::from_millis(500));
        let refused = s.session(pb::JobOperation::Backup, 301, &s.line, &[], CHUNK).err().expect("refused");
        let shown = refused.to_string();
        sh_ok("ldap-ctl stop");
        sh_ok("chown -R openldap:openldap /run/slapd && ldap-ctl start");
        assert!(shown.contains("runs as root"), "{shown}");
        // Back to normal: the plugin works again.
        let (ok, _, sent) = s.backup(302, &[b"/etc/ldap/tls".to_vec()]);
        assert_eq!(ok, 1, "{sent:?}");
        let _ = BTreeMap::<u8, u8>::new();
    }
}
