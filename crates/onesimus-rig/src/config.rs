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

/// The four ports of one rig, all above 1024.
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
}

impl Ports {
    /// Four consecutive ports from `base`: Director, FD, SD, catalog.
    pub fn from_base(base: u16) -> Ports {
        Ports { dir: base, fd: base + 1, sd: base + 2, pg: base + 3 }
    }
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
