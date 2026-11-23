// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! O-7 des OpenLDAP-Plans (`docs/plans/OPENLDAP-PLUGIN.md`): das Plugin Ende
//! zu Ende mit dem echten, als Debian-Paket installierten `ofd`-Daemon in
//! einem slapd-Container, gesteuert von `odir`, `bareos-sd` und einem
//! Wegwerf-Katalog aus dem Testgerüst.
//!
//! Start (braucht podman, Netz für den Image-Bau, bareos-sd, initdb, das
//! gebaute WASM-Plugin; Dauer einige Minuten):
//!
//! ```sh
//! (cd plugins/openldap && CARGO_TARGET_DIR=../../target/plugins cargo +1.99.1 build --release --target wasm32-wasip2)
//! cargo test -p onesimus-rig --test ldap_o7 -- --ignored --nocapture
//! ```
//!
//! Ablauf:
//!
//! 1. Eine Wegwerf-Plugin-CA (Ed25519-Wurzel und -Generation, RSA-4096-Signierer
//!    mit den Grants `io.onesimus.openldap`, `file.read`, `file.write`,
//!    `openldap.read`, `openldap.write`) entsteht im Speicher; nur die
//!    öffentlichen Zertifikate landen in `<work>/ca`. Kein privater Schlüssel
//!    wird geschrieben.
//! 2. `ofd` wird mit `ONESIMUS_PLUGIN_CA=<work>/ca` in einem eigenen
//!    Zielverzeichnis `target/ldap-o7` gebaut (nie in `target/debug` oder
//!    `target/release`, damit kein Paket mit der Wegwerf-CA entstehen kann)
//!    und mit `cargo deb` verpackt; das Plugin-Paket baut
//!    `packaging/ofd-plugin-openldap/build-deb.sh` aus dem hier signierten Bundle.
//! 3. Das Gerüst startet odir, bareos-sd und den Katalog auf dem Host, ohne
//!    eigenen ofd, mit Client/FileSet/Job je Container.
//! 4. Je Konfigurationsart ein Container (`ldaptest-o7-olc`, `ldaptest-o7-conf`,
//!    `--network host`): `dpkg -i` beider Pakete, `ofd --check-plugin`, der
//!    echte Daemon auf `127.0.0.2` und einem hohen Port; Backup, Katalog,
//!    Lecktest, Restore in Dateien und online, Audit/Joblog. Danach die
//!    Negativfälle im olc-Container.
//!
//! Podman läuft ausschließlich über `tests/openldap/o7-podman.sh` (eigener
//! Speicher unter `/tmp/ldaptest-o7-*`, nie `system reset`).
//!
//! Jede Messung steht als Zeile `O7 <Punkt>: …` in der Ausgabe.

#![deny(missing_docs)]

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use onesimus_plugins::NativePart;
use onesimus_plugins::testca::{self, TestCa};
use onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact;
use onesimus_rig::{JobRow, Rig, RigOptions};
use prost::Message;
use sha2::Digest;

/// First port of the rig (Director, -, SD, catalog).
const BASE_PORT: u16 = 39451;
/// Password between odir and the container FDs.
const FD_PASSWORD: &str = "ldap-o7-fd-pw";
/// Longest time one job may take (debug ofd, the WASM is compiled per job).
const JOB_TIMEOUT: Duration = Duration::from_secs(600);
/// Plugin id the signing certificate grants.
const PLUGIN_ID: &str = "io.onesimus.openldap";
/// Capabilities of the bundle (as the package README names them).
const CAPABILITIES: [&str; 4] = ["file.read", "file.write", "openldap.read", "openldap.write"];
/// The plugin line of both FileSets.
const PLUGIN_LINE: &str = "openldap:instance=main";
/// The ofd configuration directory the package uses.
const FD_CONF: &str = "/etc/bareos/bareos-fd.d";
/// The client file the ofd postinst creates from the default configuration.
const FD_CLIENT_FILE: &str = "/etc/bareos/bareos-fd.d/client/myself.conf";
/// The local plugin policy (it may only restrict; `{}` restricts nothing).
const POLICY: &str = "/etc/onesimus/plugin-policy.json";
/// Log file of the ofd daemon inside a container.
const OFD_LOG: &str = "/var/log/bareos/ofd-o7.log";
/// Secrets of both test instances that must never leave the volume: the
/// rootpw hash plain and in LDIF base64, anna's password plain and base64.
const COMMON_SECRETS: [&str; 4] =
    ["mzk4xLlCA0uZhM9pGxDsMizeHI7GpkgQ", "e1NTSEF9bXprNHhMbENBMHVaaE05cEd4RHNNaXplSEk3R3BrZ1E=", "annaPass1", "YW5uYVBhc3Mx"];

/// One slapd configuration kind and everything the test needs to know of it.
#[derive(Debug, Clone, Copy)]
struct Kind {
    /// `olc` (slapd.d) or `conf` (slapd.conf); also the setup script suffix.
    name: &'static str,
    /// FD port of the container's ofd on 127.0.0.2.
    fd_port: u16,
    /// Main suffix of the instance.
    suffix: &'static str,
    /// Second mdb database.
    second: &'static str,
    /// `-F …` or `-f …` for slapcat/slapd.
    config_flag: &'static str,
    /// The production `data.mdb` of the main suffix.
    data_mdb: &'static str,
    /// A clear-text proxy credential of the configuration (`credentials=`).
    plain_secret: &'static str,
}

impl Kind {
    /// The slapd.d instance (Debian default plus test content).
    const OLC: Kind = Kind {
        name: "olc",
        fd_port: BASE_PORT + 10,
        suffix: "dc=example,dc=org",
        second: "o=second,c=de",
        config_flag: "-F /etc/ldap/slapd.d",
        data_mdb: "/var/lib/ldap/data.mdb",
        plain_secret: "admin123",
    };
    /// The slapd.conf instance (with include files and TLS).
    const CONF: Kind = Kind {
        name: "conf",
        fd_port: BASE_PORT + 11,
        suffix: "dc=example,dc=com",
        second: "o=zweit,c=de",
        config_flag: "-f /etc/ldap/slapd.conf",
        data_mdb: "/srv/ldap/main/data.mdb",
        plain_secret: "MetaSecret-4711",
    };

    /// Container name.
    fn container(&self) -> String {
        format!("ldaptest-o7-{}", self.name)
    }
    /// Director name of the container's client.
    fn client(&self) -> String {
        format!("ldap-{}-fd", self.name)
    }
    /// Name of the FileSet with the plugin line.
    fn fileset(&self) -> String {
        format!("Ldap{}Set", capitalized(self.name))
    }
    /// Name of the backup job.
    fn job(&self) -> String {
        format!("Ldap{}Backup", capitalized(self.name))
    }
    /// Every secret of this instance.
    fn secrets(&self) -> Vec<&'static str> {
        let mut all = COMMON_SECRETS.to_vec();
        all.push(self.plain_secret);
        all
    }
    /// The Director resources (Client, FileSet, Job) for this container.
    fn director_resources(&self) -> Vec<(String, String)> {
        let tls = if self.name == "conf" { "    File = /etc/ldap/tls\n" } else { "" };
        vec![
            (
                format!("client/{}.conf", self.client()),
                format!(
                    "Client {{\n  Name = {}\n  Address = 127.0.0.2\n  FD Port = {}\n  Password = \"{FD_PASSWORD}\"\n}}\n",
                    self.client(),
                    self.fd_port
                ),
            ),
            (
                format!("fileset/{}.conf", self.fileset()),
                format!(
                    "FileSet {{\n  Name = {}\n  Include {{\n    Options {{\n      Signature = MD5\n    }}\n    \
                     Plugin = \"{PLUGIN_LINE}\"\n{tls}  }}\n}}\n",
                    self.fileset()
                ),
            ),
            (
                format!("job/{}.conf", self.job()),
                format!(
                    "Job {{\n  Name = {}\n  JobDefs = VDefs\n  Type = Backup\n  Level = Full\n  Client = {}\n  FileSet = {}\n}}\n",
                    self.job(),
                    self.client(),
                    self.fileset()
                ),
            ),
        ]
    }
}

/// `olc` → `Olc`.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
}

/// The workspace root.
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

/// Runs `command`, panics with its output unless it succeeds; returns stdout.
fn run(command: &mut Command) -> String {
    let out = command.output().unwrap_or_else(|e| panic!("{command:?}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{command:?}: {}\n{stdout}{}", out.status, String::from_utf8_lossy(&out.stderr));
    stdout
}

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Seconds since the epoch.
fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs()
}

/// The scratch directory of one run below `/tmp`: CA certificates, packages,
/// the Podman store. Removed on drop, the Podman store always (even with
/// `ONESIMUS_RIG_KEEP=1`).
struct Work {
    /// `/tmp/ldaptest-o7-<pid>`.
    dir: PathBuf,
    /// `tests/openldap/o7-podman.sh`.
    script: PathBuf,
}

impl Work {
    /// Creates the directory (0700) and the isolated Podman store in it.
    fn create() -> Work {
        use std::os::unix::fs::PermissionsExt;
        let dir = PathBuf::from(format!("/tmp/ldaptest-o7-{}", std::process::id()));
        let script = workspace().join("tests/openldap/o7-podman.sh");
        let work = Work { dir, script };
        work.cleanup();
        std::fs::create_dir_all(&work.dir).unwrap();
        std::fs::set_permissions(&work.dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        run(Command::new(&work.script).arg("init").arg(work.podman_base()));
        work
    }

    /// The Podman store directory.
    fn podman_base(&self) -> PathBuf {
        self.dir.join("podman")
    }

    /// `podman <args>` on the isolated store: (success, stdout + stderr).
    fn podman(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(&self.script).arg("p").arg(self.podman_base()).args(args).output().expect("o7-podman.sh");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }

    /// Like [`Work::podman`], panics on failure.
    fn podman_ok(&self, args: &[&str]) -> String {
        let (ok, out) = self.podman(args);
        assert!(ok, "podman {args:?}:\n{out}");
        out
    }

    /// `sh -c script` as root in container `c`: (success, stdout + stderr).
    fn exec(&self, c: &str, script: &str) -> (bool, String) {
        self.podman(&["exec", c, "sh", "-c", script])
    }

    /// Like [`Work::exec`], panics on failure.
    fn exec_ok(&self, c: &str, script: &str) -> String {
        let (ok, out) = self.exec(c, script);
        assert!(ok, "{c}: {script}\n{out}");
        out
    }

    /// Removes containers, images and the store, then the directory.
    fn cleanup(&self) {
        let _ = Command::new(&self.script).arg("cleanup").arg(self.podman_base()).status();
        let keep = std::env::var_os("ONESIMUS_RIG_KEEP").is_some_and(|v| v != "0");
        if keep && self.dir.exists() {
            eprintln!("O7: Arbeitsverzeichnis behalten: {}", self.dir.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// A bundle of `wasm` for the OpenLDAP plugin with the broker hash for
/// `triplet`, signed by a fresh leaf of `ca` (RSA 4096, RSA-PSS).
fn signed_bundle(ca: &TestCa, wasm: &[u8], broker_sha256: &str, triplet: &str) -> SignedPluginArtifact {
    let t = now();
    let key = testca::rsa4096();
    let grants = testca::grants(&[PLUGIN_ID], &CAPABILITIES);
    let leaf = ca.leaf(key, &grants, t as i64 - 3600, t as i64 + 2 * 86400);
    let native = [(
        triplet.to_string(),
        vec![NativePart { path: "openldap/onesimus-ldap-broker".into(), sha256: broker_sha256.into() }],
    )]
    .into();
    let manifest = testca::manifest(wasm, &leaf, PLUGIN_ID, &CAPABILITIES, native, t - 3600, t + 2 * 86400);
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    SignedPluginArtifact {
        ed25519_signature: testca::sign(key, &manifest_bytes),
        manifest_bytes,
        artifact_bytes: wasm.to_vec(),
        x509_certificate_chain_der: vec![leaf.to_der().unwrap(), ca.generation.to_der().unwrap()],
    }
}

/// The packages and negative bundles one run builds.
struct Packages {
    /// The `ofd` package (built against the throwaway CA).
    ofd: PathBuf,
    /// The `ofd-plugin-openldap` package.
    plugin: PathBuf,
    /// The bundle with one flipped signature byte.
    bad_signature: PathBuf,
    /// A correctly signed bundle from another, untrusted CA.
    untrusted: PathBuf,
    /// Debian multiarch triplet of the build.
    triplet: String,
}

/// Steps 1 and 2 of the module documentation.
fn build_packages(work: &Work) -> Packages {
    let ws = workspace();
    let wasm_path = ws.join("target/plugins/wasm32-wasip2/release/onesimus_plugin_openldap.wasm");
    assert!(
        wasm_path.is_file(),
        "build plugins/openldap first (see the module documentation): {}",
        wasm_path.display()
    );
    let wasm = std::fs::read(&wasm_path).unwrap();
    let triplet = run(Command::new("dpkg-architecture").arg("-qDEB_HOST_MULTIARCH")).trim().to_string();

    // 1. Throwaway CA: public certificates only on disk.
    let ca = TestCa::new("O7 throwaway", now() as i64 - 86400, now() as i64 + 7 * 86400);
    let ca_dir = work.dir.join("ca");
    std::fs::create_dir_all(&ca_dir).unwrap();
    std::fs::write(ca_dir.join("root.pem"), ca.root.to_pem().unwrap()).unwrap();
    std::fs::write(ca_dir.join("generation.pem"), ca.generation.to_pem().unwrap()).unwrap();
    std::fs::write(ca_dir.join("revoked.json"), b"{}").unwrap();

    // 2. ofd against it, in its own target directory.
    let target = ws.join("target/ldap-o7");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let deb_dir = work.dir.join("deb");
    eprintln!("O7: building ofd with ONESIMUS_PLUGIN_CA={} in {}", ca_dir.display(), target.display());
    run(Command::new(&cargo)
        .current_dir(&ws)
        .env("CARGO_TARGET_DIR", &target)
        .env("ONESIMUS_PLUGIN_CA", &ca_dir)
        .args(["build", "-p", "ofd", "-p", "onesimus-ldap-broker"]));
    run(Command::new(&cargo)
        .current_dir(&ws)
        .env("CARGO_TARGET_DIR", &target)
        .env("ONESIMUS_PLUGIN_CA", &ca_dir)
        .args(["deb", "-p", "ofd", "--profile", "dev", "--no-build", "--strip", "--fast", "-q", "-o"])
        .arg(&deb_dir));
    let ofd = std::fs::read_dir(&deb_dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("ofd_")))
        .expect("cargo deb wrote no ofd package");

    // The broker: stripped copy, its hash goes into the signed manifest.
    let broker_dir = work.dir.join("broker");
    std::fs::create_dir_all(&broker_dir).unwrap();
    let broker = broker_dir.join("onesimus-ldap-broker");
    std::fs::copy(target.join("debug/onesimus-ldap-broker"), &broker).unwrap();
    run(Command::new("strip").arg(&broker));
    let broker_sha = sha256_hex(&std::fs::read(&broker).unwrap());

    let bundle = signed_bundle(&ca, &wasm, &broker_sha, &triplet);
    let bundle_path = work.dir.join("openldap.pb");
    std::fs::write(&bundle_path, bundle.encode_to_vec()).unwrap();
    let mut bad = bundle.clone();
    bad.ed25519_signature[10] ^= 0x01;
    let bad_signature = work.dir.join("bad-signature.pb");
    std::fs::write(&bad_signature, bad.encode_to_vec()).unwrap();
    let other = TestCa::new("O7 untrusted", now() as i64 - 86400, now() as i64 + 7 * 86400);
    let untrusted = work.dir.join("untrusted.pb");
    std::fs::write(&untrusted, signed_bundle(&other, &wasm, &broker_sha, &triplet).encode_to_vec()).unwrap();

    let out = run(Command::new(ws.join("packaging/ofd-plugin-openldap/build-deb.sh"))
        .current_dir(&ws)
        .arg(&bundle_path)
        .arg(&broker)
        .arg(&deb_dir));
    let plugin = PathBuf::from(out.lines().last().expect("build-deb.sh prints the package").trim());
    assert!(plugin.is_file(), "{}", plugin.display());
    eprintln!("O7: packages {} and {}; broker sha256 {broker_sha}", ofd.display(), plugin.display());
    Packages { ofd, plugin, bad_signature, untrusted, triplet }
}

/// Waits until `address` accepts TCP connections.
fn wait_tcp(address: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Starts the installed ofd in `kind`'s container as the unit would
/// (`ExecStart` of ofd.service plus `-d 100`), waits for its port.
fn start_ofd(work: &Work, kind: Kind) {
    let c = kind.container();
    work.podman_ok(&[
        "exec",
        "-d",
        &c,
        "sh",
        "-c",
        &format!("cd /var/lib/bareos && exec /usr/sbin/ofd -f -c {FD_CONF} -d 100 >>{OFD_LOG} 2>&1"),
    ]);
    let address = SocketAddr::from(([127, 0, 0, 2], kind.fd_port));
    if !wait_tcp(address, Duration::from_secs(30)) {
        let log = work.exec(&c, &format!("tail -30 {OFD_LOG}")).1;
        panic!("ofd in {c} does not listen on {address}:\n{log}");
    }
}

/// Stops the ofd in `kind`'s container and waits until it is gone.
fn stop_ofd(work: &Work, kind: Kind) {
    work.exec_ok(
        &kind.container(),
        "pkill -x ofd || true; for i in $(seq 1 100); do pgrep -x ofd >/dev/null || exit 0; sleep 0.1; done; exit 1",
    );
}

/// Runs the backup job of `kind` and waits for its end.
fn backup(rig: &Rig, kind: Kind) -> JobRow {
    let id = rig.run_job(&kind.job()).unwrap_or_else(|e| panic!("{e}"));
    rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"))
}

/// Starts a restore with `command` (a console line) and waits for its end.
fn restore(rig: &Rig, command: &str) -> JobRow {
    let reply = rig.console().and_then(|mut c| c.command(command, Duration::from_secs(120))).unwrap_or_else(|e| panic!("{e}"));
    let id = onesimus_rig::console::queued_job_id(&reply).unwrap_or_else(|| panic!("{command}: no JobId:\n{reply}"));
    rig.wait_job(id, JOB_TIMEOUT).unwrap_or_else(|e| panic!("{e}"))
}

/// `ldapi://` URL of a socket path.
fn ldapi(socket: &str) -> String {
    let encoded: String = socket
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("ldapi://{encoded}")
}

/// Panics with `what` and the rig diagnostics unless `ok`.
fn check(rig: &Rig, ok: bool, what: &str) {
    assert!(ok, "{what}\n{}", rig.diagnostics());
}

/// The plugin lines of a job log (for 7.7), without wasmtime's compiler
/// debug lines (`signatures[n]::wasm_to_array_trampoline`).
fn plugin_lines(log: &str) -> Vec<String> {
    log.lines()
        .filter(|l| !l.contains("wasmtime"))
        .filter(|l| {
            let l = l.to_ascii_lowercase();
            ["plugin", "broker", "signature", "signed", "verif", "openldap"].iter().any(|w| l.contains(w))
        })
        .map(str::to_string)
        .collect()
}

/// 7.1: both packages installed with dpkg, the FD configured, postinst effects.
fn install(work: &Work, packages: &Packages, kind: Kind) {
    let c = kind.container();
    work.podman_ok(&["cp", packages.ofd.to_str().unwrap(), &format!("{c}:/root/ofd.deb")]);
    work.podman_ok(&["cp", packages.plugin.to_str().unwrap(), &format!("{c}:/root/ofd-plugin-openldap.deb")]);
    let out = work.exec_ok(&c, "dpkg -i /root/ofd.deb 2>&1");
    eprintln!("O7 7.1 [{}]: dpkg -i ofd: {}", kind.name, out.lines().filter(|l| l.contains("installiert") || l.contains("Onesimus")).collect::<Vec<_>>().join(" | "));
    // The FD as the administrator configures it: name, address, port, the
    // Director's password and the plugin policy (the postinst sets neither).
    work.exec_ok(
        &c,
        &format!(
            "cat > {FD_CLIENT_FILE} <<'EOF'\nClient {{\n  Name = {client}\n  FD Port = {port}\n  FD Address = 127.0.0.2\n  \
             Plugin Policy = {POLICY}\n}}\nEOF\n\
             cat > {FD_CONF}/director/bareos-dir.conf <<'EOF'\nDirector {{\n  Name = bareos-dir\n  Password = \"{FD_PASSWORD}\"\n}}\nEOF\n\
             install -d -m 0755 -o root -g root /etc/onesimus && echo '{{}}' > {POLICY} && chmod 0644 {POLICY}",
            client = kind.client(),
            port = kind.fd_port
        ),
    );
    let out = work.exec_ok(&c, "dpkg -i /root/ofd-plugin-openldap.deb 2>&1");
    eprintln!("O7 7.1 [{}]: dpkg -i ofd-plugin-openldap postinst: {}", kind.name, out.lines().filter(|l| l.starts_with("ofd-plugin-openldap:")).collect::<Vec<_>>().join(" | "));
    assert!(out.contains("added Plugin Names = \"openldap\""), "postinst did not add Plugin Names:\n{out}");
    let line = work.exec_ok(&c, &format!("grep -n 'ofd-plugin-openldap' {FD_CLIENT_FILE}"));
    assert!(line.contains("Plugin Names = \"openldap\"   # ofd-plugin-openldap"), "{line}");
    eprintln!("O7 7.1 [{}]: {FD_CLIENT_FILE}:{}", kind.name, line.trim());
    let test = work.exec_ok(&c, &format!("ofd -t -c {FD_CONF}"));
    eprintln!("O7 7.1 [{}]: ofd -t: {}", kind.name, test.trim());

    // Ownership up to "/" (StrictModes): every path root-owned, not writable by group/others.
    let plugins = format!("/usr/lib/{}/onesimus/plugins", packages.triplet);
    let stat = work.exec_ok(
        &c,
        &format!(
            "for p in {plugins}/openldap.pb {plugins}/openldap/onesimus-ldap-broker {plugins}/openldap {plugins} \
             /usr/lib/{t}/onesimus /usr/lib/{t} /usr/lib /usr / {POLICY} /etc/onesimus /etc; do stat -c '%u %a %n' $p; done",
            t = packages.triplet
        ),
    );
    for l in stat.lines() {
        let mut f = l.split_whitespace();
        let (uid, mode) = (f.next().unwrap(), u32::from_str_radix(f.next().unwrap(), 8).unwrap());
        assert!(uid == "0" && mode & 0o022 == 0, "not trusted: {l}");
    }
    eprintln!("O7 7.1 [{}]: ownership up to /: {}", kind.name, stat.lines().collect::<Vec<_>>().join("; "));
    let check = work.exec_ok(&c, &format!("ofd --check-plugin {plugins}/openldap.pb --plugin-policy {POLICY} 2>&1"));
    assert!(check.contains(&format!("1 native part(s) for {} OK", packages.triplet)), "{check}");
    eprintln!("O7 7.1 [{}]: ofd --check-plugin: {}", kind.name, check.trim());
}

/// 7.2–7.7 for one configuration kind; returns the JobId of the backup.
fn end_to_end(rig: &Rig, work: &Work, kind: Kind) -> u32 {
    let c = kind.container();
    let name = kind.name;

    // 7.2 the real daemon, reached by odir.
    start_ofd(work, kind);
    let status = rig.command(&format!("status client={}", kind.client())).unwrap_or_else(|e| panic!("{e}"));
    check(rig, status.contains(&kind.client()) || status.contains("Daemon started"), &format!("status client:\n{status}"));
    eprintln!("O7 7.2 [{name}]: status client: {}", status.lines().take(3).collect::<Vec<_>>().join(" | "));

    // 7.3 backup.
    let row = backup(rig, kind);
    let id = row.id;
    let log = rig.job_log(id);
    check(rig, row.status == 'T', &format!("backup {row:?}\n{log}\n--- ofd ---\n{}", work.exec(&c, &format!("tail -40 {OFD_LOG}")).1));
    eprintln!("O7 7.3 [{name}]: backup JobId={id} status={} files={} bytes={} errors={}", row.status, row.files, row.bytes, row.errors);

    // 7.4 catalog.
    let fileset_text = rig
        .catalog()
        .query(&format!("SELECT FileSetText FROM FileSet WHERE FileSet = '{}'", kind.fileset()))
        .unwrap_or_else(|e| panic!("{e}"));
    check(rig, fileset_text.contains(PLUGIN_LINE), &format!("FileSetText:\n{fileset_text}"));
    let plugin_text_line = fileset_text.lines().find(|l| l.contains("openldap")).unwrap_or("").trim().to_string();
    eprintln!("O7 7.4 [{name}]: FileSetText plugin line: {plugin_text_line}");
    let files = rig.command(&format!("list files jobid={id}")).unwrap_or_else(|e| panic!("{e}"));
    let listed: Vec<&str> = files.lines().filter(|l| l.contains("/@openldap/main/")).collect();
    check(rig, !listed.is_empty(), &format!("list files shows no /@openldap/main/:\n{files}"));
    for want in ["_info.json".to_string(), format!("data/{}.ldif", kind.suffix), format!("data/{}.ldif", kind.second)] {
        check(rig, listed.iter().any(|l| l.contains(&want)), &format!("list files misses {want}:\n{files}"));
    }
    let config_entry = if name == "olc" { "config/cn=config.ldif" } else { "config/files/etc/ldap/slapd.conf" };
    check(rig, listed.iter().any(|l| l.contains(config_entry)), &format!("list files misses {config_entry}:\n{files}"));
    let sizes = work.exec_ok(
        &c,
        &format!(
            "for b in {s} {t}; do runuser -u openldap -- slapcat {f} -b $b -o ldif_wrap=no | wc -c; done",
            s = kind.suffix,
            t = kind.second,
            f = kind.config_flag
        ),
    );
    let ldif_bytes: u64 = sizes.split_whitespace().filter_map(|n| n.parse::<u64>().ok()).sum();
    let tls_files: u64 = if name == "conf" { work.exec_ok(&c, "find /etc/ldap/tls | wc -l").trim().parse().unwrap() } else { 0 };
    let plugin_entries = listed.len() as u64;
    check(
        rig,
        row.files == plugin_entries + tls_files && row.bytes >= ldif_bytes && row.bytes < ldif_bytes + 4 * 1024 * 1024,
        &format!("JobFiles={} (listed plugin entries {plugin_entries} + TLS {tls_files}), JobBytes={} (LDIF {ldif_bytes})", row.files, row.bytes),
    );
    eprintln!(
        "O7 7.4 [{name}]: list files: {plugin_entries} entries below /@openldap/main/ (+{tls_files} /etc/ldap/tls); \
         JobFiles={} JobBytes={} >= LDIF of both databases {ldif_bytes}",
        row.files, row.bytes
    );
    for l in &listed {
        eprintln!("O7 7.4 [{name}]:   {}", l.trim());
    }

    // 7.5 leak test.
    let dump = rig.catalog().dump().unwrap_or_else(|e| panic!("{e}"));
    let ofd_log = work.exec_ok(&c, &format!("cat {OFD_LOG}"));
    let job_log_file = rig.daemon_log("job");
    let audit = rig.daemon_log("audit");
    let odir_log = rig.daemon_log("odir");
    let volumes: Vec<u8> = std::fs::read_dir(&rig.layout.storage)
        .unwrap()
        .filter_map(|e| e.ok())
        .flat_map(|e| std::fs::read(e.path()).unwrap_or_default())
        .collect();
    let in_volume = |s: &str| volumes.windows(s.len()).any(|w| w == s.as_bytes());
    check(rig, in_volume(kind.plain_secret), "positive control: the proxy credential is on the volume");
    check(rig, in_volume("credentials="), "positive control: credentials= is on the volume");
    let path_names = rig.catalog().query("SELECT Path FROM Path").unwrap_or_default()
        + &rig.catalog().query("SELECT Name FROM File").unwrap_or_default();
    let log_table = rig.catalog().query("SELECT LogText FROM Log").unwrap_or_default();
    let mut needles = kind.secrets();
    needles.push("credentials=");
    for secret in &needles {
        for (what, text) in [
            ("Path/File names", &path_names),
            ("Log table", &log_table),
            ("catalog dump", &dump),
            ("job.log", &job_log_file),
            ("audit.log", &audit),
            ("odir log", &odir_log),
            ("ofd log (-d 100)", &ofd_log),
        ] {
            check(rig, !text.contains(secret), &format!("{secret} in {what}"));
        }
    }
    eprintln!(
        "O7 7.5 [{name}]: {} needles (rootpw hash plain/base64, annaPass1 plain/base64, {}, credentials=) absent from \
         Path/File ({} bytes), Log ({} bytes), pg_dump ({} bytes), job.log, audit.log ({} bytes), odir log, ofd log ({} bytes); \
         positive control: proxy credential and credentials= present on the volume ({} bytes)",
        needles.len(),
        kind.plain_secret,
        path_names.len(),
        log_table.len(),
        dump.len(),
        audit.len(),
        ofd_log.len(),
        volumes.len()
    );

    // 7.6 (a) restore to files with Where; content equal to slapcat.
    let row_a = restore(rig, &format!("restore jobid={id} client={} where=/root/o7-files all done yes", kind.client()));
    check(rig, row_a.status == 'T', &format!("restore to files {row_a:?}\n{}", rig.job_log(row_a.id)));
    let r = "/root/o7-files/@openldap/main";
    let mut compare = format!(
        "set -e; R={r}; for b in {s} {t}; do runuser -u openldap -- slapcat {f} -b $b -o ldif_wrap=no | cmp - \"$R/data/$b.ldif\"; done; \
         stat -c '%a %U' \"$R/data/{s}.ldif\"; ",
        s = kind.suffix,
        t = kind.second,
        f = kind.config_flag
    );
    if name == "olc" {
        compare += "runuser -u openldap -- slapcat -F /etc/ldap/slapd.d -n 0 -o ldif_wrap=no | cmp - \"$R/config/cn=config.ldif\"";
    } else {
        compare += "for f in etc/ldap/slapd.conf etc/ldap/schema/core.schema etc/ldap/schema/local/onesimus.schema etc/default/slapd; \
                    do cmp \"/$f\" \"$R/config/files/$f\"; done";
    }
    let mode = work.exec_ok(&c, &compare);
    check(rig, mode.trim() == "600 root", &format!("restored LDIF mode/owner: {mode}"));
    eprintln!(
        "O7 7.6a [{name}]: restore JobId={} status={} files={}; data LDIFs ({} and {}) and {} byte-equal (cmp) with slapcat/originals; mode {}",
        row_a.id,
        row_a.status,
        row_a.files,
        kind.suffix,
        kind.second,
        if name == "olc" { "cn=config" } else { "slapd.conf, core.schema, onesimus.schema, /etc/default/slapd" },
        mode.trim()
    );

    // 7.6 (b) online restore into a new target, second slapd, login, production unchanged.
    let mdb_before = work.exec_ok(&c, &format!("sha256sum {}", kind.data_mdb));
    let row_b = restore(
        rig,
        &format!("restore jobid={id} client={} pluginoptions=openldap:restore_mode=online all done yes", kind.client()),
    );
    let log_b = rig.job_log(row_b.id);
    check(rig, row_b.status == 'T', &format!("online restore {row_b:?}\n{log_b}"));
    let target = format!("/var/lib/onesimus-ldap-restore/{}", row_b.id);
    check(rig, log_b.contains(&format!("restored online into {target}")), &format!("online restore log:\n{log_b}"));
    let socket = format!("{target}/run/ldapi");
    let url = ldapi(&socket);
    let flag = if name == "olc" { format!("-F {target}/slapd.d") } else { format!("-f {target}/conf/etc/ldap/slapd.conf") };
    work.exec_ok(&c, &format!("slapd {flag} -h '{url}' -u openldap -g openldap"));
    let who = work.exec_ok(
        &c,
        &format!(
            "for i in $(seq 1 50); do ldapsearch -x -H '{url}' -b '' -s base >/dev/null 2>&1 && break; sleep 0.2; done; \
             ldapwhoami -x -H '{url}' -D uid=anna,ou=people,{s} -w annaPass1",
            s = kind.suffix
        ),
    );
    check(rig, who.contains("uid=anna"), &format!("login anna on the restored target: {who}"));
    let same = work.exec_ok(
        &c,
        &format!(
            "for b in {s} {t}; do \
               a=$(runuser -u openldap -- slapcat {f} -b $b -o ldif_wrap=no | sort | sha256sum); \
               b2=$(runuser -u openldap -- slapcat {flag} -b $b -o ldif_wrap=no | sort | sha256sum); \
               [ \"$a\" = \"$b2\" ] && echo \"$b equal\" || echo \"$b DIFFERENT\"; done; \
             kill $(cat {target}/run/slapd.pid); sleep 0.5",
            s = kind.suffix,
            t = kind.second,
            f = kind.config_flag
        ),
    );
    check(rig, !same.contains("DIFFERENT"), &format!("restored databases differ: {same}"));
    let mdb_after = work.exec_ok(&c, &format!("sha256sum {}", kind.data_mdb));
    check(rig, mdb_before == mdb_after, &format!("production data.mdb changed: {mdb_before} / {mdb_after}"));
    eprintln!(
        "O7 7.6b [{name}]: online restore JobId={} status={} into {target}; second slapd ({flag}) up; ldapwhoami anna/annaPass1: {}; \
         {}; {} sha256 before = after ({})",
        row_b.id,
        row_b.status,
        who.trim(),
        same.split_whitespace().collect::<Vec<_>>().join(" "),
        kind.data_mdb,
        mdb_before.split_whitespace().next().unwrap_or("")
    );

    // 7.7 what the job logs, the ofd log and the audit log say about the plugin.
    let backup_lines = plugin_lines(&log);
    let restore_lines = plugin_lines(&log_b);
    let ofd_lines = plugin_lines(&work.exec_ok(&c, &format!("cat {OFD_LOG}")));
    let audit_lines: Vec<&str> = audit.lines().filter(|l| l.contains(&kind.job()) || l.contains("pluginoptions")).collect();
    eprintln!("O7 7.7 [{name}]: backup job log, plugin lines ({}):", backup_lines.len());
    for l in &backup_lines {
        eprintln!("O7 7.7 [{name}]:   {l}");
    }
    eprintln!("O7 7.7 [{name}]: online restore job log, plugin lines ({}):", restore_lines.len());
    for l in &restore_lines {
        eprintln!("O7 7.7 [{name}]:   {l}");
    }
    eprintln!("O7 7.7 [{name}]: ofd log (-d 100), plugin/broker/signature lines ({}):", ofd_lines.len());
    for l in ofd_lines.iter().take(40) {
        eprintln!("O7 7.7 [{name}]:   {l}");
    }
    eprintln!("O7 7.7 [{name}]: odir audit.log lines for this job ({}):", audit_lines.len());
    for l in &audit_lines {
        eprintln!("O7 7.7 [{name}]:   {l}");
    }
    check(rig, restore_lines.iter().any(|l| l.contains("restoring online through io.onesimus.openldap")), "online restore names the plugin id");
    id
}

/// 7.8 in the olc container: tampered broker, wrong and untrusted signature,
/// plugin missing from Plugin Names; each must fail the job visibly.
fn negatives(rig: &Rig, work: &Work, packages: &Packages) {
    let kind = Kind::OLC;
    let c = kind.container();
    let plugins = format!("/usr/lib/{}/onesimus/plugins", packages.triplet);
    let failed = |what: &str, want: &str| {
        let row = backup(rig, kind);
        let log = rig.job_log(row.id);
        let hit = log.lines().find(|l| l.contains(want)).unwrap_or("").trim().to_string();
        check(rig, row.status != 'T' && !hit.is_empty(), &format!("{what}: {row:?}, wanted \"{want}\" in:\n{log}"));
        eprintln!("O7 7.8 [{what}]: JobId={} status={} errors={}: {hit}", row.id, row.status, row.errors);
    };

    // Tampered broker (hash mismatch).
    work.exec_ok(&c, &format!("cp -p {plugins}/openldap/onesimus-ldap-broker /root/broker.orig && printf X >> {plugins}/openldap/onesimus-ldap-broker"));
    failed("tampered broker", "SHA-256 does not match the signed manifest");
    work.exec_ok(&c, &format!("cp -p /root/broker.orig {plugins}/openldap/onesimus-ldap-broker"));

    // Bundles with a wrong signature and from an untrusted CA.
    work.exec_ok(&c, &format!("cp -p {plugins}/openldap.pb /root/openldap.pb.orig"));
    for (what, file) in [("wrong signature", &packages.bad_signature), ("untrusted CA", &packages.untrusted)] {
        work.podman_ok(&["cp", file.to_str().unwrap(), &format!("{c}:/root/neg.pb")]);
        work.exec_ok(&c, &format!("install -m 0644 -o root -g root /root/neg.pb {plugins}/openldap.pb"));
        failed(what, "Plugin: \"openldap:instance=main\":");
    }
    work.exec_ok(&c, &format!("install -m 0644 -o root -g root /root/openldap.pb.orig {plugins}/openldap.pb"));

    // Plugin not in Plugin Names (line removed, ofd restarted).
    work.exec_ok(&c, &format!("sed -i '/# ofd-plugin-openldap/d' {FD_CLIENT_FILE}"));
    stop_ofd(work, kind);
    start_ofd(work, kind);
    failed("not in Plugin Names", "is not in Plugin Names");

    // Back: the postinst adds the line again, the plugin works again.
    let out = work.exec_ok(&c, "/var/lib/dpkg/info/ofd-plugin-openldap:*.postinst configure 2>&1");
    stop_ofd(work, kind);
    start_ofd(work, kind);
    let row = backup(rig, kind);
    check(rig, row.status == 'T', &format!("positive control after the negatives: {row:?}\n{}", rig.job_log(row.id)));
    eprintln!("O7 7.8 [positive control]: postinst again ({}), backup JobId={} status={}", out.trim(), row.id, row.status);
}

/// O-7 end to end, both configuration kinds (see the module documentation).
#[test]
#[ignore = "needs podman, network, bareos-sd, initdb and the built openldap guest: run with --ignored"]
fn ldap_plugin_end_to_end_with_odir_and_bareos_sd() {
    for program in ["podman", "dpkg-architecture", "dpkg-deb", "strip"] {
        assert!(onesimus_rig::prereq::which(program).is_some(), "{program} is missing");
    }
    let work = Work::create();
    let packages = build_packages(&work);

    onesimus_rig::build_workspace_binaries().unwrap_or_else(|e| panic!("{e}"));
    let mut options = RigOptions::new(BASE_PORT);
    options.start_fd = false;
    options.auditing = true;
    for kind in [Kind::OLC, Kind::CONF] {
        options.extra_director.extend(kind.director_resources());
    }
    let rig = Rig::start(options).unwrap_or_else(|e| panic!("Gerüst startet nicht: {e}"));

    eprintln!("O7: building localhost/ldaptest-slapd (isolated store {})", work.podman_base().display());
    work.podman_ok(&["build", "-q", "-t", "localhost/ldaptest-slapd", workspace().join("tests/openldap").to_str().unwrap()]);
    for kind in [Kind::OLC, Kind::CONF] {
        let c = kind.container();
        work.podman_ok(&["run", "-d", "--init", "--network", "host", "--name", &c, "--hostname", kind.name, "localhost/ldaptest-slapd"]);
        work.podman_ok(&["exec", "-e", "LDAPTEST_SLAPD_SERVICES=ldapi:///", &c, &format!("/opt/ldaptest/setup-{}.sh", kind.name)]);
        install(&work, &packages, kind);
        end_to_end(&rig, &work, kind);
    }
    negatives(&rig, &work, &packages);
    for kind in [Kind::OLC, Kind::CONF] {
        stop_ofd(&work, kind);
    }
    eprintln!("O7: all checks passed");
}
