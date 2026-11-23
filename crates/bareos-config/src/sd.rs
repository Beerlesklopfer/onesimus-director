//! Konfiguration des Storage Daemons (`bareos-sd.d`).
//!
//! Zweck: osd liest dieselbe Konfiguration wie bareos-sd, damit ein Wechsel
//! in beide Richtungen ohne Umschreiben geht. Dieses Modul macht aus den
//! Rohressourcen ([`crate::raw`]) die fünf Ressourcenarten des SD:
//! [`SdStorage`] (der Daemon selbst, genau eine), [`SdDirector`] (wer sich
//! anmelden darf), [`SdDevice`] (Ausgabegeräte), [`SdMessages`] (wohin
//! Meldungen gehen) und [`SdAutochanger`] (Geräte mit Wechsler).
//!
//! Geprüft wird, was bareos-sd beim Start ablehnt und osd in P1 braucht:
//! unbekannte Ressourcenarten (z. B. ein `Distributor` in `bareos-sd.d`,
//! siehe Plan D-8), fehlende Pflichtangaben (`Name`, `Password`,
//! `Media Type`, `Archive Device`, `Device` am Autochanger), nicht genau
//! eine Storage-Ressource, doppelte Namen, Verweise auf unbekannte
//! Messages- oder Device-Ressourcen, ungültige Zahlen und Wahrheitswerte.
//! Unbekannte Direktiven innerhalb einer bekannten Ressource prüft das
//! Modul noch nicht (bareos-sd lehnt sie ab); sie bleiben in `other`
//! erhalten (P1).
//!
//! Datenfluss: `osd -c <Pfad>` → [`load`] → [`crate::raw::load_tree`]
//! (`bareos-sd.d/*/*.conf`) → [`SdConfig::from_resources`] → optional
//! [`SdConfig::apply_overlay`] (`osd.d/*/*.conf`) → osd.
//!
//! **Erweiterung `osd.d/` (Plan D-8, „additiv, rückwärtskompatibel“):**
//! Onesimus-Angaben stehen nie in `bareos-sd.d`, sondern in einem eigenen
//! Baum `osd.d/<Typ>/<Name>.conf` neben `bareos-sd.d` ([`overlay_dir`]).
//! Ein Eintrag dort erweitert die Ressource gleichen Typs und gleichen
//! Namens aus `bareos-sd.d` („Namensgleichheit ist Bedingung“):
//!
//! - Er darf nur Onesimus-Direktiven setzen ([`ONESIMUS_DIRECTIVES`]);
//!   jede andere Direktive, also jede Bareos-Direktive, ist ein Fehler.
//! - Ohne gleichnamige Ressource in `bareos-sd.d` ist er ein Fehler.
//! - Reine osd-Ressourcen (`Distributor` mit `Route`) kommen in P2; bis
//!   dahin lehnt der Lader sie als noch nicht unterstützt ab.
//! - Ohne `osd.d/` ist das Ergebnis genau das von `bareos-sd.d` allein;
//!   ein bareos-sd liest denselben Baum unverändert.
//!
//! Onesimus-Direktiven heute: `Ring Slots` (Storage, Device) und für den
//! gRPC-Zugang (P1 Stufe 3) an der Storage `Grpc Enable` (Vorgabe yes),
//! `Grpc Port` (Vorgabe `SD Port` + 1000, wie ofd) und `Grpc Address`
//! (Vorgabe `SD Address`).
//!
//! Bareos-Gegenstück: `core/src/stored/stored_conf.cc` (`store_items`,
//! `dir_items`, `dev_items`, `changer_items`, Defaults), Zweig `bareos-25`.

use std::path::Path;

use crate::raw::{ConfResource, Directive};
use crate::values::parse_bool;

/// Default `SD Port` (`stored_conf.cc`).
pub const DEFAULT_SD_PORT: u16 = 9103;

/// Default `Maximum Concurrent Jobs` of the Storage resource.
pub const DEFAULT_MAX_CONCURRENT_JOBS: u32 = 20;

/// Default `Maximum Block Size` of a Device (`stored_conf.cc`).
pub const DEFAULT_MAX_BLOCK_SIZE: u32 = 1_048_576;

/// Default `Label Block Size` of a Device (`stored_conf.cc`).
pub const DEFAULT_LABEL_BLOCK_SIZE: u32 = 64_512;

/// Default `Client Connect Wait` in seconds (`stored_conf.cc`).
pub const DEFAULT_CLIENT_CONNECT_WAIT: u64 = 1800;

/// The Onesimus directives `osd.d/` may add, per resource type (normalized
/// type, normalized key). Everything else in `osd.d/` is an error.
pub const ONESIMUS_DIRECTIVES: &[(&str, &str)] = &[
    ("storage", "ringslots"),
    ("device", "ringslots"),
    ("storage", "grpcenable"),
    ("storage", "grpcport"),
    ("storage", "grpcaddress"),
];

/// Offset of the default gRPC port from the `SD Port` (as ofd: FD port + 1000).
pub const GRPC_PORT_OFFSET: u16 = 1000;

/// The `Storage` resource: the daemon itself.
#[derive(Debug, Clone, PartialEq)]
pub struct SdStorage {
    /// `Name`: the SD's name (the Director's `Storage` resource names it).
    pub name: String,
    /// `Description`.
    pub description: Option<String>,
    /// `SD Port` (default 9103).
    pub port: u16,
    /// `SD Address`: one listen address; `None` listens on all.
    pub address: Option<String>,
    /// `Working Directory`.
    pub working_directory: Option<String>,
    /// `Pid Directory`.
    pub pid_directory: Option<String>,
    /// `Plugin Directory`.
    pub plugin_directory: Option<String>,
    /// `Maximum Concurrent Jobs` (default 20).
    pub maximum_concurrent_jobs: u32,
    /// `Messages`: name of the Messages resource for daemon messages.
    pub messages: Option<String>,
    /// `Just In Time Reservation` (default yes since Bareos 23): reserve
    /// the device only when the first data arrives.
    pub just_in_time_reservation: bool,
    /// `Client Connect Wait` in seconds (default 1800): how long a job waits
    /// for its File Daemon.
    pub client_connect_wait: u64,
    /// osd.d: `Ring Slots`, default slot count of the job rings (Onesimus).
    pub ring_slots: Option<u32>,
    /// `TLS Enable` (default yes). Off: osd tells its peers `ssl=0` after the
    /// CRAM-MD5 of a clear-text connection, so a Bareos peer does not start
    /// TLS-PSK on it (a Copy with `ssl=0`).
    pub tls_enable: bool,
    /// osd.d: `Grpc Enable` (default yes): start the TLS gRPC listener
    /// (`onesimus.stored.StorageDaemon`) next to the BNET port (Onesimus).
    pub grpc_enable: bool,
    /// osd.d: `Grpc Port` (default `SD Port` + 1000, see [`SdStorage::grpc_port`]).
    pub grpc_port: Option<u16>,
    /// osd.d: `Grpc Address` (default the `SD Address`, else all).
    pub grpc_address: Option<String>,
    /// Every other directive, unchecked.
    pub other: Vec<Directive>,
}

impl SdStorage {
    /// The gRPC port: `Grpc Port`, else `SD Port` + [`GRPC_PORT_OFFSET`].
    pub fn grpc_port(&self) -> u16 {
        self.grpc_port.unwrap_or(self.port.saturating_add(GRPC_PORT_OFFSET))
    }

    /// The gRPC listen address: `Grpc Address`, else `SD Address`, else
    /// `0.0.0.0`.
    pub fn grpc_address(&self) -> &str {
        self.grpc_address.as_deref().or(self.address.as_deref()).unwrap_or("0.0.0.0")
    }
}

/// A `Director` resource: a Director allowed to connect.
#[derive(Debug, Clone, PartialEq)]
pub struct SdDirector {
    /// `Name` of the Director.
    pub name: String,
    /// `Password` (plain or `[md5]…`).
    pub password: String,
    /// `Description`.
    pub description: Option<String>,
    /// `Monitor = yes`: may only ask for status.
    pub monitor: bool,
    /// `Maximum Bandwidth Per Job` as written (e.g. `10 mb/s`).
    pub maximum_bandwidth_per_job: Option<String>,
    /// Every other directive, unchecked.
    pub other: Vec<Directive>,
}

/// A `Device` resource: one output device.
#[derive(Debug, Clone, PartialEq)]
pub struct SdDevice {
    /// `Name` of the device.
    pub name: String,
    /// `Description`.
    pub description: Option<String>,
    /// `Media Type`: must match the Director's Storage resource.
    pub media_type: String,
    /// `Archive Device`: directory, tape device or object store spec.
    pub archive_device: String,
    /// `Device Type` as written (`File`, `Tape`, `Fifo`, `droplet` …);
    /// `None`: bareos-sd guesses it from the archive device.
    pub device_type: Option<String>,
    /// `Label Media`: label blank volumes automatically.
    pub label_media: bool,
    /// `Random Access`.
    pub random_access: bool,
    /// `Automatic Mount` (default yes).
    pub automatic_mount: bool,
    /// `Removable Media` (default yes).
    pub removable_media: bool,
    /// `Always Open` (default yes).
    pub always_open: bool,
    /// `Autochanger = yes`: the device belongs to a changer.
    pub autochanger: bool,
    /// `Drive Index` within the changer.
    pub drive_index: u32,
    /// `Maximum Concurrent Jobs` (default 1).
    pub maximum_concurrent_jobs: u32,
    /// `Maximum Block Size` in bytes (default 1 048 576, `stored_conf.cc`).
    pub maximum_block_size: u32,
    /// `Label Block Size` in bytes (default 64 512).
    pub label_block_size: u32,
    /// `Maximum Volume Size` in bytes; 0 = no limit (deprecated in Bareos,
    /// the Pool's `Maximum Volume Bytes` is the usual limit).
    pub maximum_volume_size: u64,
    /// osd.d: `Ring Slots`, slot count of the ring of a job on this device
    /// (Onesimus, D-12).
    pub ring_slots: Option<u32>,
    /// Every other directive, unchecked.
    pub other: Vec<Directive>,
}

/// A `Messages` resource: where messages go.
#[derive(Debug, Clone, PartialEq)]
pub struct SdMessages {
    /// `Name` of the resource.
    pub name: String,
    /// `Description`.
    pub description: Option<String>,
    /// The destinations (`Director = …`, `Append = …`, `Mail = …`), as written.
    pub destinations: Vec<Directive>,
}

/// An `Autochanger` resource: devices served by one changer.
#[derive(Debug, Clone, PartialEq)]
pub struct SdAutochanger {
    /// `Name` of the changer.
    pub name: String,
    /// `Description`.
    pub description: Option<String>,
    /// `Device`: names of its Device resources (at least one).
    pub devices: Vec<String>,
    /// `Changer Device` (e.g. `/dev/sg0`).
    pub changer_device: Option<String>,
    /// `Changer Command` (e.g. `mtx-changer %c %o %S %a %d`).
    pub changer_command: Option<String>,
}

/// The whole `bareos-sd.d` configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct SdConfig {
    /// The one Storage resource.
    pub storage: SdStorage,
    /// Directors, at least one.
    pub directors: Vec<SdDirector>,
    /// Devices, at least one.
    pub devices: Vec<SdDevice>,
    /// Messages resources.
    pub messages: Vec<SdMessages>,
    /// Autochangers.
    pub autochangers: Vec<SdAutochanger>,
}

/// Loads `path` (a file, a directory containing `bareos-sd.d`, or the
/// `bareos-sd.d` directory itself) and checks it.
pub fn load(path: &Path) -> Result<SdConfig, String> {
    let resources = crate::raw::load_tree(path, "bareos-sd.d")?;
    let mut config = SdConfig::from_resources(&resources)?;
    let overlay = overlay_dir(path);
    if overlay.is_dir() {
        let extensions = crate::raw::load_tree(&overlay, "osd.d").map_err(|e| format!("osd.d: {e}"))?;
        config.apply_overlay(&extensions)?;
    }
    Ok(config)
}

/// Where `osd.d/` belongs for the configuration `path`: next to the file,
/// inside the directory that holds `bareos-sd.d`, or next to `bareos-sd.d`.
pub fn overlay_dir(path: &Path) -> std::path::PathBuf {
    if path.is_file() {
        return path.parent().unwrap_or(Path::new(".")).join("osd.d");
    }
    if path.file_name().is_some_and(|n| n == "bareos-sd.d") {
        return path.parent().unwrap_or(Path::new(".")).join("osd.d");
    }
    path.join("osd.d")
}

/// The directives of one resource, looked up by normalized key.
struct Res<'a> {
    /// The raw resource.
    raw: &'a ConfResource,
    /// What to call it in errors (`Device "FileStorage"`).
    label: String,
}

impl<'a> Res<'a> {
    /// Wraps `raw`; `kind` is the resource type as Bareos names it.
    fn new(raw: &'a ConfResource, kind: &str) -> Self {
        let label = match raw.name() {
            Some(name) => format!("{kind} \"{name}\""),
            None => kind.to_string(),
        };
        Self { raw, label }
    }

    /// The directive with normalized key `key`.
    fn get(&self, key: &str) -> Option<&'a Directive> {
        self.raw.directives.iter().find(|d| d.norm_key() == key)
    }

    /// The first value of `key`.
    fn text(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|d| d.first()).map(str::to_string)
    }

    /// The first value of `key`, an error when missing.
    fn required(&self, key: &str, written: &str) -> Result<String, String> {
        self.text(key).filter(|v| !v.is_empty())
            .ok_or_else(|| format!("{}: required directive \"{written}\" missing", self.label))
    }

    /// `key` as a boolean, `default` when missing.
    fn boolean(&self, key: &str, default: bool) -> Result<bool, String> {
        match self.text(key) {
            None => Ok(default),
            Some(v) => parse_bool(&v).map(|(_, b)| b).map_err(|e| format!("{}: {key}: {e}", self.label)),
        }
    }

    /// `key` as an unsigned number, `default` when missing.
    fn number<T: std::str::FromStr>(&self, key: &str, default: T) -> Result<T, String> {
        match self.text(key) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| format!("{}: {key}: invalid number \"{v}\"", self.label)),
        }
    }

    /// `key` as a size (`1M`, `64512`), `default` when missing.
    fn size(&self, key: &str, default: u64) -> Result<u64, String> {
        match self.get(key) {
            None => Ok(default),
            Some(d) => crate::values::parse_size(&d.values().join(" "))
                .map(|(_, v)| v)
                .map_err(|e| format!("{}: {key}: {e}", self.label)),
        }
    }

    /// `key` as a time in seconds (`30 minutes`), `default` when missing.
    fn duration(&self, key: &str, default: u64) -> Result<u64, String> {
        match self.get(key) {
            None => Ok(default),
            Some(d) => crate::values::parse_time(&d.values().join(" "))
                .map(|(_, v)| v.as_secs())
                .map_err(|e| format!("{}: {key}: {e}", self.label)),
        }
    }

    /// Directives whose keys are not in `known`.
    fn other(&self, known: &[&str]) -> Vec<Directive> {
        self.raw.directives.iter().filter(|d| !known.contains(&d.norm_key().as_str())).cloned().collect()
    }
}

impl SdConfig {
    /// Builds and checks the configuration from raw resources.
    pub fn from_resources(resources: &[ConfResource]) -> Result<Self, String> {
        let mut storages = Vec::new();
        let mut directors = Vec::new();
        let mut devices = Vec::new();
        let mut messages = Vec::new();
        let mut autochangers = Vec::new();

        for raw in resources {
            match raw.norm_type().as_str() {
                "storage" => {
                    let r = Res::new(raw, "Storage");
                    storages.push(SdStorage {
                        name: r.required("name", "Name")?,
                        description: r.text("description"),
                        port: r.number("sdport", DEFAULT_SD_PORT)?,
                        address: r.text("sdaddress"),
                        working_directory: r.text("workingdirectory"),
                        pid_directory: r.text("piddirectory"),
                        plugin_directory: r.text("plugindirectory"),
                        maximum_concurrent_jobs: r.number("maximumconcurrentjobs", DEFAULT_MAX_CONCURRENT_JOBS)?,
                        messages: r.text("messages"),
                        just_in_time_reservation: r.boolean("justintimereservation", true)?,
                        client_connect_wait: r.duration("clientconnectwait", DEFAULT_CLIENT_CONNECT_WAIT)?,
                        ring_slots: None,
                        tls_enable: r.boolean("tlsenable", true)?,
                        grpc_enable: true,
                        grpc_port: None,
                        grpc_address: None,
                        other: r.other(&["name", "description", "sdport", "sdaddress", "workingdirectory",
                            "piddirectory", "plugindirectory", "maximumconcurrentjobs", "messages",
                            "justintimereservation", "clientconnectwait"]),
                    });
                }
                "director" => {
                    let r = Res::new(raw, "Director");
                    directors.push(SdDirector {
                        name: r.required("name", "Name")?,
                        password: r.required("password", "Password")?,
                        description: r.text("description"),
                        monitor: r.boolean("monitor", false)?,
                        maximum_bandwidth_per_job: r.get("maximumbandwidthperjob")
                            .map(|d| d.values().join(" ")),
                        other: r.other(&["name", "password", "description", "monitor", "maximumbandwidthperjob"]),
                    });
                }
                "device" => {
                    let r = Res::new(raw, "Device");
                    devices.push(SdDevice {
                        name: r.required("name", "Name")?,
                        description: r.text("description"),
                        media_type: r.required("mediatype", "Media Type")?,
                        archive_device: r.required("archivedevice", "Archive Device")?,
                        device_type: r.text("devicetype"),
                        label_media: r.boolean("labelmedia", false)?,
                        random_access: r.boolean("randomaccess", false)?,
                        automatic_mount: r.boolean("automaticmount", true)?,
                        removable_media: r.boolean("removablemedia", true)?,
                        always_open: r.boolean("alwaysopen", true)?,
                        autochanger: r.boolean("autochanger", false)?,
                        drive_index: r.number("driveindex", 0)?,
                        maximum_concurrent_jobs: r.number("maximumconcurrentjobs", 1)?,
                        maximum_block_size: r.size("maximumblocksize", DEFAULT_MAX_BLOCK_SIZE.into())? as u32,
                        label_block_size: r.size("labelblocksize", DEFAULT_LABEL_BLOCK_SIZE.into())? as u32,
                        maximum_volume_size: r.size("maximumvolumesize", 0)?,
                        ring_slots: None,
                        other: r.other(&["name", "description", "mediatype", "archivedevice", "devicetype",
                            "labelmedia", "randomaccess", "automaticmount", "removablemedia", "alwaysopen",
                            "autochanger", "driveindex", "maximumconcurrentjobs", "maximumblocksize",
                            "labelblocksize", "maximumvolumesize"]),
                    });
                }
                "messages" => {
                    let r = Res::new(raw, "Messages");
                    messages.push(SdMessages {
                        name: r.required("name", "Name")?,
                        description: r.text("description"),
                        destinations: r.other(&["name", "description"]),
                    });
                }
                "autochanger" => {
                    let r = Res::new(raw, "Autochanger");
                    let devices: Vec<String> = raw.directives.iter().filter(|d| d.norm_key() == "device")
                        .flat_map(|d| d.values().into_iter().map(str::to_string)).collect();
                    if devices.is_empty() {
                        return Err(format!("{}: required directive \"Device\" missing", r.label));
                    }
                    autochangers.push(SdAutochanger {
                        name: r.required("name", "Name")?,
                        description: r.text("description"),
                        devices,
                        changer_device: r.text("changerdevice"),
                        changer_command: r.text("changercommand"),
                    });
                }
                _ => {
                    let at = raw.source.as_ref().map(|p| format!(" in {}", p.display())).unwrap_or_default();
                    return Err(format!("unknown resource type \"{}\"{at}", raw.rtype));
                }
            }
        }

        let storage = match storages.len() {
            1 => storages.remove(0),
            0 => return Err("no Storage resource defined".to_string()),
            n => return Err(format!("only one Storage resource permitted, {n} defined")),
        };
        if directors.is_empty() {
            return Err("no Director resource defined".to_string());
        }
        if devices.is_empty() {
            return Err("no Device resource defined".to_string());
        }
        duplicates("Director", directors.iter().map(|d| &d.name))?;
        duplicates("Device", devices.iter().map(|d| &d.name))?;
        duplicates("Messages", messages.iter().map(|m| &m.name))?;
        duplicates("Autochanger", autochangers.iter().map(|a| &a.name))?;
        if let Some(m) = &storage.messages
            && !messages.iter().any(|r| &r.name == m)
        {
            return Err(format!("Storage \"{}\": Messages resource \"{m}\" not found", storage.name));
        }
        for changer in &autochangers {
            for dev in &changer.devices {
                if !devices.iter().any(|d| &d.name == dev) {
                    return Err(format!("Autochanger \"{}\": Device \"{dev}\" not found", changer.name));
                }
            }
        }
        Ok(Self { storage, directors, devices, messages, autochangers })
    }

    /// Applies the `osd.d/` extensions `resources` (rules in the module
    /// comment): only Onesimus directives, only onto a same-named resource
    /// of the same type.
    pub fn apply_overlay(&mut self, resources: &[ConfResource]) -> Result<(), String> {
        for raw in resources {
            let rtype = raw.norm_type();
            let at = raw.source.as_ref().map(|p| format!(" ({})", p.display())).unwrap_or_default();
            let Some(name) = raw.name().map(str::to_string) else {
                return Err(format!("osd.d: {} without Name{at}", raw.rtype));
            };
            let known_type = matches!(rtype.as_str(), "storage" | "director" | "device" | "messages" | "autochanger");
            if !known_type {
                return Err(format!(
                    "osd.d: resource type \"{}\" is not supported yet{at} (Distributor and Route follow in P2)",
                    raw.rtype
                ));
            }
            let exists = match rtype.as_str() {
                "storage" => self.storage.name == name,
                "director" => self.directors.iter().any(|d| d.name == name),
                "device" => self.devices.iter().any(|d| d.name == name),
                "messages" => self.messages.iter().any(|m| m.name == name),
                _ => self.autochangers.iter().any(|a| a.name == name),
            };
            if !exists {
                return Err(format!("osd.d: {} \"{name}\" extends no {} of that name in bareos-sd.d{at}", raw.rtype, raw.rtype));
            }
            let r = Res::new(raw, &raw.rtype);
            for d in &raw.directives {
                let key = d.norm_key();
                if key == "name" {
                    continue;
                }
                if !ONESIMUS_DIRECTIVES.contains(&(rtype.as_str(), key.as_str())) {
                    return Err(format!(
                        "osd.d: {} \"{name}\": \"{}\" is not an Onesimus directive; osd.d may only add Onesimus \
                         directives and never set or override a Bareos directive{at}",
                        raw.rtype, d.key
                    ));
                }
            }
            let slots = match r.get("ringslots") {
                Some(_) => Some(r.number::<u32>("ringslots", 0)?).filter(|&n| n >= 2).map(Some)
                    .ok_or_else(|| format!("osd.d: {} \"{name}\": Ring Slots must be at least 2{at}", raw.rtype))?,
                None => None,
            };
            if rtype == "storage" {
                self.storage.grpc_enable = r.boolean("grpcenable", self.storage.grpc_enable)?;
                if r.get("grpcport").is_some() {
                    self.storage.grpc_port = Some(r.number::<u16>("grpcport", 0)?);
                }
                if let Some(address) = r.text("grpcaddress") {
                    self.storage.grpc_address = Some(address);
                }
            }
            match rtype.as_str() {
                "storage" => self.storage.ring_slots = slots.or(self.storage.ring_slots),
                "device" => {
                    if let Some(dev) = self.devices.iter_mut().find(|d| d.name == name) {
                        dev.ring_slots = slots.or(dev.ring_slots);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The Director resource named `name`.
    pub fn find_director(&self, name: &str) -> Option<&SdDirector> {
        self.directors.iter().find(|d| d.name == name)
    }

    /// The Device resource named `name`.
    pub fn find_device(&self, name: &str) -> Option<&SdDevice> {
        self.devices.iter().find(|d| d.name == name)
    }
}

/// An error naming the first name used twice.
fn duplicates<'a>(kind: &str, names: impl Iterator<Item = &'a String>) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for name in names {
        if !seen.insert(name.as_str()) {
            return Err(format!("{kind} \"{name}\" defined twice"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resources of a Bareos 25 default `bareos-sd.d` (one file).
    const SAMPLE: &str = r#"
Storage {
  Name = PDC-sd
  Maximum Concurrent Jobs = 20
  Working Directory = /var/lib/bareos
  Messages = Standard
}
Director {
  Name = PDC-dir
  Password = "[md5]0123456789abcdef"
  Description = "Director, who is permitted to contact this storage daemon."
}
Director {
  Name = PDC-mon
  Password = "secret"
  Monitor = yes
}
Device {
  Name = FileStorage
  Media Type = File
  Archive Device = /var/lib/bareos/storage
  LabelMedia = yes;
  Random Access = yes;
  AutomaticMount = yes;
  RemovableMedia = no;
  AlwaysOpen = no;
  Description = "File device. A connecting Director must have the same Name and MediaType."
}
Messages {
  Name = Standard
  Director = PDC-dir = all
}
"#;

    fn parse(text: &str) -> Result<SdConfig, String> {
        SdConfig::from_resources(&crate::raw::parse_str(text, None).unwrap())
    }

    #[test]
    fn default_config_is_read() {
        let c = parse(SAMPLE).unwrap();
        assert_eq!(c.storage.name, "PDC-sd");
        assert_eq!(c.storage.port, 9103);
        assert_eq!(c.storage.messages.as_deref(), Some("Standard"));
        assert_eq!(c.directors.len(), 2);
        assert_eq!(c.find_director("PDC-dir").unwrap().password, "[md5]0123456789abcdef");
        assert!(c.find_director("PDC-mon").unwrap().monitor);
        let dev = c.find_device("FileStorage").unwrap();
        assert_eq!((dev.media_type.as_str(), dev.archive_device.as_str()), ("File", "/var/lib/bareos/storage"));
        assert!(dev.label_media && dev.random_access && dev.automatic_mount);
        assert!(!dev.removable_media && !dev.always_open && !dev.autochanger);
        assert_eq!(c.messages[0].destinations.len(), 1);
    }

    #[test]
    fn autochanger_and_port() {
        let text = format!("{SAMPLE}\nAutochanger {{\n Name = Lib\n Device = FileStorage, Drive2\n \
            Changer Device = /dev/sg0\n Changer Command = \"mtx-changer %c %o %S %a %d\"\n}}\n\
            Device {{\n Name = Drive2\n Media Type = LTO\n Archive Device = /dev/nst0\n Autochanger = yes\n \
            Drive Index = 1\n}}\n").replace("Name = PDC-sd", "Name = PDC-sd\n  SD Port = 19103");
        let c = parse(&text).unwrap();
        assert_eq!(c.storage.port, 19103);
        assert_eq!(c.autochangers[0].devices, ["FileStorage", "Drive2"]);
        assert_eq!(c.autochangers[0].changer_command.as_deref(), Some("mtx-changer %c %o %S %a %d"));
        let d2 = c.find_device("Drive2").unwrap();
        assert!(d2.autochanger);
        assert_eq!(d2.drive_index, 1);
    }

    #[test]
    fn errors_bareos_sd_would_report() {
        let cases = [
            (SAMPLE.replace("Archive Device = /var/lib/bareos/storage", ""),
                "Device \"FileStorage\": required directive \"Archive Device\" missing"),
            (SAMPLE.replace("Password = \"secret\"", ""), "Director \"PDC-mon\": required directive \"Password\" missing"),
            (format!("{SAMPLE}\nDistributor {{\n Name = V\n}}\n"), "unknown resource type \"Distributor\""),
            (format!("{SAMPLE}\nStorage {{\n Name = two\n}}\n"), "only one Storage resource permitted, 2 defined"),
            (SAMPLE.replace("Messages = Standard", "Messages = Nope"), "Storage \"PDC-sd\": Messages resource \"Nope\" not found"),
            (SAMPLE.replace("Name = PDC-mon", "Name = PDC-dir"), "Director \"PDC-dir\" defined twice"),
            (SAMPLE.replace("LabelMedia = yes", "LabelMedia = maybe"), "Device \"FileStorage\": labelmedia: invalid boolean: maybe"),
            (SAMPLE.replace("Maximum Concurrent Jobs = 20", "Maximum Concurrent Jobs = many"),
                "Storage \"PDC-sd\": maximumconcurrentjobs: invalid number \"many\""),
            (format!("{SAMPLE}\nAutochanger {{\n Name = Lib\n Device = Missing\n}}\n"),
                "Autochanger \"Lib\": Device \"Missing\" not found"),
        ];
        for (text, want) in cases {
            assert_eq!(parse(&text).unwrap_err(), want);
        }
        assert_eq!(parse("Storage {\n Name = x\n}\n").unwrap_err(), "no Director resource defined");
    }

    #[test]
    fn new_bareos_directives_with_defaults() {
        let c = parse(SAMPLE).unwrap();
        assert!(c.storage.just_in_time_reservation);
        assert_eq!(c.storage.client_connect_wait, 1800);
        let d = c.find_device("FileStorage").unwrap();
        assert_eq!((d.maximum_block_size, d.label_block_size, d.maximum_volume_size), (1_048_576, 64_512, 0));
        let text = SAMPLE.replace("AlwaysOpen = no;", "AlwaysOpen = no;\n  Maximum Block Size = 262144\n  Maximum Volume Size = 5 GiB")
            .replace("Messages = Standard\n}", "Messages = Standard\n  Just In Time Reservation = no\n  Client Connect Wait = 5 minutes\n}");
        let c = parse(&text).unwrap();
        assert!(!c.storage.just_in_time_reservation);
        assert_eq!(c.storage.client_connect_wait, 300);
        let d = c.find_device("FileStorage").unwrap();
        assert_eq!((d.maximum_block_size, d.maximum_volume_size), (262_144, 5 << 30));
    }

    /// Applies the osd.d text `overlay` to [`SAMPLE`].
    fn overlay(overlay: &str) -> Result<SdConfig, String> {
        let mut c = parse(SAMPLE)?;
        c.apply_overlay(&crate::raw::parse_str(overlay, None).unwrap())?;
        Ok(c)
    }

    #[test]
    fn osd_d_extension_applies_to_the_same_named_resource() {
        let c = overlay("Device {\n Name = FileStorage\n Ring Slots = 32\n}\nStorage {\n Name = PDC-sd\n RingSlots = 8\n}\n").unwrap();
        assert_eq!(c.find_device("FileStorage").unwrap().ring_slots, Some(32));
        assert_eq!(c.storage.ring_slots, Some(8));
        // Bareos directives are untouched
        assert_eq!(c.find_device("FileStorage").unwrap().media_type, "File");
    }

    #[test]
    fn tls_enable_of_the_storage_defaults_to_yes() {
        assert!(parse(SAMPLE).unwrap().storage.tls_enable);
        let c = parse(&SAMPLE.replace("Maximum Concurrent Jobs = 20", "Maximum Concurrent Jobs = 20\n  TLS Enable = no")).unwrap();
        assert!(!c.storage.tls_enable);
    }

    #[test]
    fn osd_d_grpc_directives_of_the_storage() {
        let c = parse(SAMPLE).unwrap();
        assert!(c.storage.grpc_enable);
        assert_eq!(c.storage.grpc_port(), c.storage.port + GRPC_PORT_OFFSET);
        assert_eq!(c.storage.grpc_address(), c.storage.address.as_deref().unwrap_or("0.0.0.0"));
        let c = overlay("Storage {\n Name = PDC-sd\n Grpc Port = 19103\n Grpc Address = 127.0.0.1\n}\n").unwrap();
        assert_eq!((c.storage.grpc_port(), c.storage.grpc_address()), (19103, "127.0.0.1"));
        let c = overlay("Storage {\n Name = PDC-sd\n Grpc Enable = no\n}\n").unwrap();
        assert!(!c.storage.grpc_enable);
        let e = overlay("Storage {\n Name = PDC-sd\n Grpc Port = many\n}\n").unwrap_err();
        assert!(e.contains("grpcport"), "{e}");
        let e = overlay("Device {\n Name = FileStorage\n Grpc Port = 1\n}\n").unwrap_err();
        assert!(e.contains("not an Onesimus directive"), "{e}");
    }

    #[test]
    fn osd_d_override_of_a_bareos_directive_is_rejected() {
        let e = overlay("Device {\n Name = FileStorage\n Media Type = Other\n}\n").unwrap_err();
        assert!(e.contains("\"Media Type\" is not an Onesimus directive"), "{e}");
        let e = overlay("Device {\n Name = FileStorage\n Ring Slots = 1\n}\n").unwrap_err();
        assert!(e.contains("at least 2"), "{e}");
    }

    #[test]
    fn osd_d_orphan_and_new_types_are_rejected() {
        let e = overlay("Device {\n Name = NoSuchDevice\n Ring Slots = 4\n}\n").unwrap_err();
        assert!(e.contains("Device \"NoSuchDevice\" extends no Device of that name in bareos-sd.d"), "{e}");
        let e = overlay("Distributor {\n Name = V\n}\n").unwrap_err();
        assert!(e.contains("not supported yet"), "{e}");
        let e = overlay("Device {\n Ring Slots = 4\n}\n").unwrap_err();
        assert!(e.contains("without Name"), "{e}");
    }

    #[test]
    fn without_osd_d_the_tree_reads_like_bareos_sd() {
        let dir = std::env::temp_dir().join(format!("bareos-config-osd-d-{}", std::process::id()));
        let file = dir.join("bareos-sd.conf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, SAMPLE).unwrap();
        assert_eq!(overlay_dir(&file), dir.join("osd.d"));
        let plain = load(&file).unwrap();
        assert_eq!(plain, parse(SAMPLE).unwrap());
        assert_eq!(plain.find_device("FileStorage").unwrap().ring_slots, None);
        std::fs::create_dir_all(dir.join("osd.d").join("device")).unwrap();
        std::fs::write(dir.join("osd.d/device/FileStorage.conf"), "Device {\n Name = FileStorage\n Ring Slots = 4\n}\n").unwrap();
        assert_eq!(load(&file).unwrap().find_device("FileStorage").unwrap().ring_slots, Some(4));
        std::fs::write(dir.join("osd.d/device/FileStorage.conf"), "Device {\n Name = Other\n Ring Slots = 4\n}\n").unwrap();
        assert!(load(&file).unwrap_err().contains("extends no Device"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tree_is_loaded_from_bareos_sd_d() {
        let dir = std::env::temp_dir().join(format!("bareos-config-sd-{}", std::process::id()));
        let tree = dir.join("bareos-sd.d");
        for (sub, file, text) in [
            ("storage", "PDC-sd.conf", "Storage {\n Name = PDC-sd\n}\n"),
            ("director", "PDC-dir.conf", "Director {\n Name = PDC-dir\n Password = x\n}\n"),
            ("device", "FileStorage.conf", "Device {\n Name = FileStorage\n Media Type = File\n Archive Device = /tmp\n}\n"),
        ] {
            std::fs::create_dir_all(tree.join(sub)).unwrap();
            std::fs::write(tree.join(sub).join(file), text).unwrap();
        }
        let a = load(&dir).unwrap();
        let b = load(&tree).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.storage.name, "PDC-sd");
        std::fs::remove_dir_all(&dir).ok();
    }
}
