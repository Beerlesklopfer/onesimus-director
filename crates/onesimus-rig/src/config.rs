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
}
