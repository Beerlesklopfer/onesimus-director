//! Konfigurationen von Director, FD und SD für ein Gerüst.
//!
//! Zweck: Alle drei Daemons bekommen eine Bareos-Konfiguration mit eigenen
//! hohen Ports auf `127.0.0.1` und Arbeitsverzeichnissen unter dem
//! Gerüstverzeichnis. Die Konfiguration ist dieselbe, die `bareos-dir`,
//! `bareos-fd` und `bareos-sd` lesen; die Daemons sind dadurch austauschbar.
//!
//! Datenfluss: [`Layout::new`] legt die Pfade fest, [`write_all`] schreibt
//! `fd/client/vfd.conf` (ofd liest ein Verzeichnis), `sd/bareos-sd.conf`
//! (eine Datei) und `dir/bareos-dir.d/<Ressource>/<Name>.conf`.
//! [`write_director_resources`] ergänzt danach Ressourcen eines Tests, etwa
//! Client, FileSet und Job für einen FD außerhalb des Gerüsts (ein ofd in
//! einem Container); [`auditing_resources`] schaltet das Audit-Log ein.
//!
//! Bareos-Gegenstück: die Konfigurationsvorlagen der `systemtests`
//! (`systemtests/tests/*/etc/bareos/`). Die Ressourcen hier entsprechen dem
//! Gerüst `ofdrig.py` des Gutachtens.

use std::path::{Path, PathBuf};

/// Password of the default console (`*UserAgent*`) in the Director.
pub const CONSOLE_PASSWORD: &str = "rig-dir-pw";

/// Name of the client (FD) in all configurations.
pub const CLIENT: &str = "vfd";

/// The ports of one rig, all above 1024.
#[derive(Debug, Clone, Copy)]
pub struct Ports {
    /// Director port (console and daemons connect here).
    pub dir: u16,
    /// File Daemon port.
    pub fd: u16,
    /// Storage Daemon port.
    pub sd: u16,
    /// Catalog cluster port on `127.0.0.2`.
    pub pg: u16,
    /// gRPC port of the FD (ofd `--grpc-listen`), used with [`Grpc::On`].
    pub fd_grpc: u16,
    /// gRPC port of the SD (osd.d `Grpc Port`), used with [`Grpc::On`].
    pub sd_grpc: u16,
    /// Port of the second SD, the writing side of a Copy ([`crate::RigOptions::copy_target`]).
    pub copy_sd: u16,
}

impl Ports {
    /// Six consecutive ports from `base`: Director, FD, SD, catalog, FD
    /// gRPC, SD gRPC (the tests space their bases by 10).
    pub fn from_base(base: u16) -> Ports {
        Ports { dir: base, fd: base + 1, sd: base + 2, pg: base + 3, fd_grpc: base + 4, sd_grpc: base + 5, copy_sd: base + 6 }
    }
}

/// How a daemon of the rig offers gRPC and whether the Director uses it
/// (Plan P1 stage 3, interop matrix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grpc {
    /// No gRPC: the daemon runs with `--disable-grpc`, the Director's
    /// resource has no `Grpc Enable` (BNET as before stage 3).
    Off,
    /// The daemon listens on its rig gRPC port, the Director's resource
    /// has `Grpc Enable = yes` with port and pinned certificate.
    On,
    /// The Director's resource as with [`Grpc::On`], but the daemon runs
    /// with `--disable-grpc`: the Director must fall back to BNET.
    Disabled,
}

impl Grpc {
    /// Whether the Director's resource carries the gRPC directives.
    pub fn configured(self) -> bool {
        self != Grpc::Off
    }

    /// Whether the daemon is started with `--disable-grpc`.
    pub fn disabled(self) -> bool {
        self != Grpc::On
    }
}

/// File name of ofd's generated gRPC certificate in its working directory.
pub const FD_GRPC_CERT: &str = "onesimus-fd-grpc.crt";

/// File name of osd's generated gRPC certificate in its working directory.
pub const SD_GRPC_CERT: &str = "onesimus-sd-grpc.crt";

/// The Onesimus gRPC directives of a Director resource (`Grpc Enable`,
/// port, pinned certificate, server name `localhost`), indented.
fn grpc_directives(port: u16, cert: &Path) -> String {
    format!(
        "  Grpc Enable = yes\n  Grpc Port = {port}\n  Grpc CA Certificate File = \"{}\"\n  Grpc Server Name = localhost\n",
        cert.display()
    )
}

/// The Director's Client resource of the rig FD: `Passive = yes` with
/// `passive`, the gRPC directives unless `grpc` is [`Grpc::Off`].
pub fn client_resource(layout: &Layout, ports: Ports, passive: bool, grpc: Grpc) -> (String, String) {
    let mut text = format!(
        "Client {{\n  Name = {CLIENT}\n  Address = 127.0.0.1\n  FD Port = {fd}\n  Password = \"vfd-pw\"\n",
        fd = ports.fd
    );
    if passive {
        text += "  Passive = yes\n";
    }
    text += "}\n";
    (format!("client/{CLIENT}.conf"), text)
}

/// The `odir.d` extension of the rig FD's Client resource: the gRPC
/// directives (Onesimus, not Bareos: they stand outside `bareos-dir.d`).
pub fn client_extension(layout: &Layout, ports: Ports) -> (String, String) {
    (
        format!("client/{CLIENT}.conf"),
        format!("Client {{\n  Name = {CLIENT}\n{}}}\n", grpc_directives(ports.fd_grpc, &layout.fd_work.join(FD_GRPC_CERT))),
    )
}

/// The `odir.d` extension of the rig SD's Storage resource (gRPC directives).
pub fn storage_extension(layout: &Layout, ports: Ports) -> (String, String) {
    ("storage/vsd.conf".into(), format!("Storage {{\n  Name = vsd\n{}}}\n", grpc_directives(ports.sd_grpc, &layout.sd_work.join(SD_GRPC_CERT))))
}

/// The Director's Storage resource of the rig SD, with the gRPC
/// directives unless `grpc` is [`Grpc::Off`].
pub fn storage_resource(layout: &Layout, ports: Ports, grpc: Grpc) -> (String, String) {
    let mut text = format!(
        "Storage {{\n  Name = vsd\n  Address = 127.0.0.1\n  SD Port = {sd}\n  Password = \"vsd-pw\"\n  Device = VDev\n  \
         Media Type = VFile\n  Maximum Concurrent Jobs = 10\n",
        sd = ports.sd
    );
    text += "}\n";
    ("storage/vsd.conf".into(), text)
}

/// Writes a self-signed certificate for `localhost` to `path` unless it
/// exists. Used where the Director is told to pin a daemon's certificate
/// that the daemon never generates ([`Grpc::Disabled`], bareos-sd): the
/// Director then really tries the gRPC port, is refused and falls back,
/// instead of failing early on a missing file.
pub fn write_stand_in_certificate(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).map_err(std::io::Error::other)?;
    write(path, &cert.cert.pem())
}

/// Writes `osd.d/storage/vsd.conf` next to the SD configuration: the gRPC
/// port of the rig as an osd.d extension (D-8). bareos-sd never reads it.
pub fn write_sd_grpc_extension(layout: &Layout, ports: Ports) -> std::io::Result<()> {
    let dir = layout.root.join("onesimus").join("osd.d").join("storage");
    write(
        &dir.join("vsd.conf"),
        &format!("Storage {{\n  Name = vsd\n  Grpc Port = {}\n  Grpc Address = 127.0.0.1\n}}\n", ports.sd_grpc),
    )
}

/// Every directory and file of one rig below its root.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The rig root, a fresh directory in `/tmp`.
    pub root: PathBuf,
    /// Source tree of the main backup.
    pub data: PathBuf,
    /// Source of the cancel case (one large file).
    pub big: PathBuf,
    /// Target of the restore.
    pub restore: PathBuf,
    /// Configuration directory of the FD (`-c`).
    pub fd_conf: PathBuf,
    /// Working directory of the FD.
    pub fd_work: PathBuf,
    /// Configuration file of the SD (`-c`).
    pub sd_conf: PathBuf,
    /// Working directory of the SD.
    pub sd_work: PathBuf,
    /// Directory of the file volumes (SD Archive Device).
    pub storage: PathBuf,
    /// Configuration directory of the Director (`-c`).
    pub dir_conf: PathBuf,
    /// Working directory of the Director (job.log, daemon.log).
    pub dir_work: PathBuf,
}

impl Layout {
    /// The layout below `root`; nothing is created yet.
    pub fn new(root: &Path) -> Layout {
        Layout {
            root: root.to_path_buf(),
            data: root.join("data"),
            big: root.join("big"),
            restore: root.join("restore"),
            fd_conf: root.join("fd"),
            fd_work: root.join("fd-work"),
            sd_conf: root.join("sd").join("bareos-sd.conf"),
            sd_work: root.join("sd-work"),
            storage: root.join("storage"),
            dir_conf: root.join("dir").join("bareos-dir.d"),
            dir_work: root.join("dir-work"),
        }
    }

    /// Where the restore puts the source tree (`where` + absolute source path).
    pub fn restored_data(&self) -> PathBuf {
        let relative = self.data.strip_prefix("/").unwrap_or(&self.data);
        self.restore.join(relative)
    }
}

/// Writes the configurations of FD, SD and Director.
pub fn write_all(layout: &Layout, ports: Ports, catalog_user: &str) -> std::io::Result<()> {
    for dir in [&layout.fd_work, &layout.sd_work, &layout.storage, &layout.dir_work, &layout.restore] {
        std::fs::create_dir_all(dir)?;
    }
    write(&layout.fd_conf.join("client").join("vfd.conf"), &fd_config(layout, ports))?;
    write(&layout.sd_conf, &sd_config(layout, ports))?;
    for (relative, text) in director_config(layout, ports, catalog_user) {
        write(&layout.dir_conf.join(relative), &text)?;
    }
    Ok(())
}

/// Writes additional or replacing Director resources (relative path below
/// the Director configuration directory, text), e.g. a Client, FileSet and
/// Job for a File Daemon outside the rig ([`crate::RigOptions::extra_director`]).
/// A path that [`director_config`] also writes replaces that file.
pub fn write_director_resources(layout: &Layout, resources: &[(String, String)]) -> std::io::Result<()> {
    for (relative, text) in resources {
        if relative.split('/').any(|part| part.is_empty() || part == "..") {
            return Err(std::io::Error::other(format!("Director resource path {relative:?} is not a plain relative path")));
        }
        write(&layout.dir_conf.join(relative), text)?;
    }
    Ok(())
}

/// The Director resource with `Auditing = yes` and a Messages resource
/// `Daemon` that also appends the audit messages to `dir-work/audit.log`
/// ([`crate::RigOptions::auditing`]); written over the defaults.
pub fn auditing_resources(layout: &Layout, ports: Ports, catalog_user: &str) -> Vec<(String, String)> {
    let work = layout.dir_work.display();
    director_config(layout, ports, catalog_user)
        .into_iter()
        .filter_map(|(path, text)| match path.as_str() {
            "director/bareos-dir.conf" => Some((path, text.replacen("\n}\n", "\n  Auditing = yes\n}\n", 1))),
            "messages/Daemon.conf" => Some((
                path,
                format!(
                    "Messages {{\n  Name = Daemon\n  append = \"{work}/daemon.log\" = all, !skipped, !saved, !audit\n  \
                     append = \"{work}/audit.log\" = audit\n}}\n"
                ),
            )),
            _ => None,
        })
        .collect()
}

/// The Client resource with `Passive = yes` ([`crate::RigOptions::passive_client`]):
/// the Director tells the SD to connect to the FD; written over the default.
pub fn passive_client_resources(ports: Ports) -> Vec<(String, String)> {
    vec![(
        format!("client/{CLIENT}.conf"),
        format!(
            "Client {{\n  Name = {CLIENT}\n  Address = 127.0.0.1\n  FD Port = {fd}\n  Password = \"vfd-pw\"\n  Passive = yes\n}}\n",
            fd = ports.fd
        ),
    )]
}

/// Creates the parent directory of `path` and writes `text`.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)
}

/// Client, Director and Messages resource of the FD.
pub fn fd_config(layout: &Layout, ports: Ports) -> String {
    format!(
        "Client {{\n  Name = {CLIENT}\n  FD Port = {fd}\n  FD Address = 127.0.0.1\n  Working Directory = {work}\n  \
         Pid Directory = {work}\n  Maximum Bandwidth Per Job = 4 mb/s\n}}\n\
         Director {{\n  Name = bareos-dir\n  Password = \"vfd-pw\"\n}}\n\
         Messages {{\n  Name = Standard\n  Director = bareos-dir = all, !skipped, !restored\n}}\n",
        fd = ports.fd,
        work = layout.fd_work.display()
    )
}

/// The second device of the rig SD (`VDev2`, media type `VFile2`, volumes
/// below `root/storage2`), appended to its configuration for a Copy or a
/// VirtualFull inside one SD.
pub fn second_device(root: &Path) -> String {
    format!(
        "Device {{\n  Name = VDev2\n  Media Type = VFile2\n  Archive Device = {root}/storage2\n  LabelMedia = yes\n  \
         Random Access = yes\n  AutomaticMount = yes\n  RemovableMedia = no\n  AlwaysOpen = no\n}}\n",
        root = root.display()
    )
}

/// Configuration of the second SD (`vsd2`, device `VDev2`, media type
/// `VFile2`, volumes below `root/storage2`).
pub fn copy_sd_config(root: &Path, ports: Ports, plain: bool) -> String {
    let tls = if plain { "  TLS Enable = no\n  TLS Require = no\n" } else { "" };
    format!(
        "Storage {{\n  Name = vsd2\n  SD Port = {sd}\n  SD Address = 127.0.0.1\n  Working Directory = {root}/sd2-work\n{tls}}}\n\
         Director {{\n  Name = bareos-dir\n  Password = \"vsd2-pw\"\n{tls}}}\n\
         Device {{\n  Name = VDev2\n  Media Type = VFile2\n  Archive Device = {root}/storage2\n  LabelMedia = yes\n  \
         Random Access = yes\n  AutomaticMount = yes\n  RemovableMedia = no\n  AlwaysOpen = no\n}}\n\
         Messages {{\n  Name = Standard\n  Director = bareos-dir = all\n}}\n",
        sd = ports.copy_sd,
        root = root.display()
    )
}

/// The Director resources of a Copy to the second SD: its Storage, the
/// pool `VCopyPool`, `VPool` with it as Next Pool, and the Copy job `VCopy`
/// (uncopied jobs of `VPool`, read from `vsd`).
pub fn copy_resources(ports: Ports, same_sd: bool, plain: bool) -> Vec<(String, String)> {
    let tls = if plain { "  TLS Enable = no\n  TLS Require = no\n" } else { "" };
    // a second SD process (own port and password), or a second device of the rig SD
    let (port, password) = if same_sd { (ports.sd, "vsd-pw") } else { (ports.copy_sd, "vsd2-pw") };
    vec![
        (
            "storage/vsd2.conf".into(),
            format!(
                "Storage {{\n  Name = vsd2\n  Address = 127.0.0.1\n  SD Port = {port}\n  Password = \"{password}\"\n  Device = VDev2\n  \
                 Media Type = VFile2\n  Maximum Concurrent Jobs = 10\n{tls}}}\n"
            ),
        ),
        (
            "pool/VCopyPool.conf".into(),
            "Pool {\n  Name = VCopyPool\n  Pool Type = Backup\n  Label Format = \"VCopy-\"\n  Maximum Volume Bytes = 1G\n  Storage = vsd2\n}\n".into(),
        ),
        (
            "pool/VPool.conf".into(),
            "Pool {\n  Name = VPool\n  Pool Type = Backup\n  Label Format = \"VVol-\"\n  Maximum Volume Bytes = 1G\n  Next Pool = VCopyPool\n}\n".into(),
        ),
        (
            "job/VFull.conf".into(),
            "Job {\n  Name = VFull\n  JobDefs = VDefs\n  Type = Backup\n  Level = VirtualFull\n  Accurate = yes\n}\n".into(),
        ),
        (
            "job/VCopy.conf".into(),
            "Job {\n  Name = VCopy\n  JobDefs = VDefs\n  Type = Copy\n  Selection Type = PoolUncopiedJobs\n}\n".into(),
        ),
    ]
}

/// Storage, Director, Device and Messages resource of the SD.
pub fn sd_config(layout: &Layout, ports: Ports) -> String {
    format!(
        "Storage {{\n  Name = vsd\n  SD Port = {sd}\n  SD Address = 127.0.0.1\n  Working Directory = {work}\n}}\n\
         Director {{\n  Name = bareos-dir\n  Password = \"vsd-pw\"\n}}\n\
         Device {{\n  Name = VDev\n  Media Type = VFile\n  Archive Device = {storage}\n  LabelMedia = yes\n  \
         Random Access = yes\n  AutomaticMount = yes\n  RemovableMedia = no\n  AlwaysOpen = no\n}}\n\
         Messages {{\n  Name = Standard\n  Director = bareos-dir = all\n}}\n",
        sd = ports.sd,
        work = layout.sd_work.display(),
        storage = layout.storage.display()
    )
}

/// The Director resources as (relative path, text) pairs.
pub fn director_config(layout: &Layout, ports: Ports, catalog_user: &str) -> Vec<(String, String)> {
    let work = layout.dir_work.display();
    let data = layout.data.display();
    let big = layout.big.display();
    let restore = layout.restore.display();
    let mut files = vec![
        (
            "director/bareos-dir.conf".to_string(),
            format!(
                "Director {{\n  Name = bareos-dir\n  Password = \"{CONSOLE_PASSWORD}\"\n  Messages = Daemon\n  \
                 Working Directory = {work}\n  DirPort = {dir}\n  DirAddress = 127.0.0.1\n  \
                 Maximum Concurrent Jobs = 10\n  QueryFile = \"/usr/lib/bareos/scripts/query.sql\"\n}}\n",
                dir = ports.dir
            ),
        ),
        (
            "catalog/MyCatalog.conf".into(),
            format!(
                "Catalog {{\n  Name = MyCatalog\n  DbName = {db}\n  DbUser = \"{catalog_user}\"\n  DbPassword = \"\"\n  \
                 DbAddress = {addr}\n  DbPort = {pg}\n}}\n",
                db = crate::catalog::DATABASE,
                addr = crate::catalog::LISTEN,
                pg = ports.pg
            ),
        ),
        (
            "messages/Daemon.conf".into(),
            format!("Messages {{\n  Name = Daemon\n  append = \"{work}/daemon.log\" = all, !skipped, !saved, !audit\n}}\n"),
        ),
        (
            "messages/Standard.conf".into(),
            format!(
                "Messages {{\n  Name = Standard\n  append = \"{work}/job.log\" = all, !skipped, !saved, !audit\n  \
                 catalog = all, !skipped, !saved, !audit\n}}\n"
            ),
        ),
        (
            format!("client/{CLIENT}.conf"),
            format!("Client {{\n  Name = {CLIENT}\n  Address = 127.0.0.1\n  FD Port = {fd}\n  Password = \"vfd-pw\"\n}}\n", fd = ports.fd),
        ),
        (
            "storage/vsd.conf".into(),
            format!(
                "Storage {{\n  Name = vsd\n  Address = 127.0.0.1\n  SD Port = {sd}\n  Password = \"vsd-pw\"\n  Device = VDev\n  \
                 Media Type = VFile\n  Maximum Concurrent Jobs = 10\n}}\n",
                sd = ports.sd
            ),
        ),
        (
            "pool/VPool.conf".into(),
            "Pool {\n  Name = VPool\n  Pool Type = Backup\n  Label Format = \"VVol-\"\n  Maximum Volume Bytes = 1G\n}\n".into(),
        ),
        (
            "fileset/VSet.conf".into(),
            format!(
                "FileSet {{\n  Name = VSet\n  Include {{\n    Options {{\n      Signature = MD5\n      Compression = GZIP\n      \
                 Sparse = yes\n      HardLinks = yes\n      Verify = pins5\n    }}\n    File = {data}\n  }}\n}}\n"
            ),
        ),
        (
            "fileset/BigSet.conf".into(),
            format!("FileSet {{\n  Name = BigSet\n  Include {{\n    Options {{\n      Signature = XXH128\n    }}\n    File = {big}\n  }}\n}}\n"),
        ),
        (
            "jobdefs/VDefs.conf".into(),
            format!(
                "JobDefs {{\n  Name = VDefs\n  Client = {CLIENT}\n  FileSet = VSet\n  Storage = vsd\n  Pool = VPool\n  \
                 Messages = Standard\n  Priority = 10\n}}\n"
            ),
        ),
    ];
    let jobs = [
        ("VBackup", "Type = Backup\n  Level = Full".to_string()),
        ("VRestore", format!("Type = Restore\n  Where = {restore}")),
        ("VVol", "Type = Verify\n  Level = VolumeToCatalog\n  Verify Job = VBackup".into()),
        ("BigBackup", "Type = Backup\n  Level = Full\n  FileSet = BigSet\n  Maximum Bandwidth = 200 kb/s".into()),
    ];
    for (name, body) in jobs {
        files.push((format!("job/{name}.conf"), format!("Job {{\n  Name = {name}\n  JobDefs = VDefs\n  {body}\n}}\n")));
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restored_data_appends_the_absolute_source_path() {
        let layout = Layout::new(Path::new("/tmp/r1"));
        assert_eq!(layout.restored_data(), Path::new("/tmp/r1/restore/tmp/r1/data"));
    }

    #[test]
    fn catalog_uses_tcp_on_the_rig_address() {
        let layout = Layout::new(Path::new("/tmp/r1"));
        let files = director_config(&layout, Ports::from_base(39391), "me");
        let catalog = &files.iter().find(|(p, _)| p.starts_with("catalog/")).unwrap().1;
        assert!(catalog.contains("DbAddress = 127.0.0.2") && catalog.contains("DbPort = 39394"), "{catalog}");
    }

    #[test]
    fn auditing_switches_on_auditing_and_an_audit_log() {
        let layout = Layout::new(Path::new("/tmp/r1"));
        let files = auditing_resources(&layout, Ports::from_base(39391), "me");
        let director = &files.iter().find(|(p, _)| p == "director/bareos-dir.conf").unwrap().1;
        assert!(director.contains("  Auditing = yes\n}\n") && director.contains("DirPort = 39391"), "{director}");
        let daemon = &files.iter().find(|(p, _)| p == "messages/Daemon.conf").unwrap().1;
        assert!(daemon.contains("/tmp/r1/dir-work/audit.log\" = audit"), "{daemon}");
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn extra_resource_paths_must_stay_below_the_configuration() {
        let layout = Layout::new(Path::new("/nonexistent-rig"));
        let error = write_director_resources(&layout, &[("../evil.conf".into(), String::new())]).unwrap_err();
        assert!(error.to_string().contains("not a plain relative path"), "{error}");
    }
}
