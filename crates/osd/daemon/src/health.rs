//! Gesundheit der Platten unter den File-Devices (D-21, erste Umsetzung).
//!
//! Zweck: osd findet zu jedem File-Device die Platte, auf der sein
//! `Archive Device` liegt (Einhängepunkt in `/proc/self/mountinfo`, bei einer
//! Partition die Platte darüber in `/sys/class/block`), fragt SMART über
//! `smartctl --json` (smartmontools) ab und bewertet das Ergebnis mit
//! `ok`, `warning` oder `critical`. `status storage` zeigt den Zustand je
//! Device, `.status health` liefert ihn zeilenweise zum Auswerten (odir).
//! Die Abfrage läuft höchstens alle zehn Minuten je Platte.
//!
//! Grenzen der ersten Umsetzung: nur Platten, die `smartctl` kennt (ATA über
//! SATA/USB mit passendem Treiber, NVMe, SAS); eine Platte hinter md, LVM
//! oder btrfs über mehrere Platten bleibt `unknown`; SMART braucht root oder
//! `CAP_SYS_RAWIO`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How long a result is reused.
const CACHE: Duration = Duration::from_secs(600);

/// How long `smartctl` may run.
const SMARTCTL_TIMEOUT: Duration = Duration::from_secs(30);

/// Temperature (°C) from which a disk is `warning` and `critical`.
const TEMP_WARNING: i64 = 55;
const TEMP_CRITICAL: i64 = 60;

/// The verdict on a disk (D-21: ok / warnung / kritisch / ausgefallen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// No finding.
    Ok,
    /// Growing defects, high temperature, error log entries.
    Warning,
    /// SMART says FAILED, a failed self test, a pre-fail attribute below its threshold.
    Critical,
}

impl Level {
    /// The word `status` and `.status health` print.
    pub fn name(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warning => "warning",
            Level::Critical => "critical",
        }
    }
}

/// What SMART says about one disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Health {
    /// The disk, e.g. `sdb`.
    pub disk: String,
    /// The model.
    pub model: String,
    /// The verdict.
    pub level: Level,
    /// Why not `ok`.
    pub reasons: Vec<String>,
    /// Temperature in °C.
    pub temperature: Option<i64>,
    /// Power-on hours.
    pub power_on_hours: Option<i64>,
    /// Current_Pending_Sector (ID 197).
    pub pending: Option<i64>,
    /// Reallocated_Sector_Ct (ID 5).
    pub reallocated: Option<i64>,
    /// Offline_Uncorrectable (ID 198).
    pub offline_uncorrectable: Option<i64>,
    /// UDMA_CRC_Error_Count (ID 199).
    pub crc_errors: Option<i64>,
    /// The result of the last self test.
    pub self_test: String,
}

/// The raw value of the ATA attribute `id`.
fn attribute(json: &Value, id: i64) -> Option<i64> {
    json.pointer("/ata_smart_attributes/table")?.as_array()?.iter().find(|a| a["id"].as_i64() == Some(id))?.pointer("/raw/value")?.as_i64()
}

/// Evaluates the output of `smartctl --json -a` for `disk`.
pub fn evaluate(disk: &str, json: &Value) -> Health {
    let model = json["model_name"].as_str().unwrap_or("").to_string();
    let mut reasons = Vec::new();
    let mut level = Level::Ok;
    let mut raise = |to: Level, why: String, level: &mut Level| {
        if to > *level {
            *level = to;
        }
        reasons.push(why);
    };
    let pending = attribute(json, 197);
    let reallocated = attribute(json, 5);
    let offline = attribute(json, 198);
    let crc = attribute(json, 199);
    let temperature = json.pointer("/temperature/current").and_then(Value::as_i64);
    let power_on_hours = json.pointer("/power_on_time/hours").and_then(Value::as_i64);
    if json.pointer("/smart_status/passed").and_then(Value::as_bool) == Some(false) {
        raise(Level::Critical, "SMART overall health: FAILED".into(), &mut level);
    }
    let self_test = json.pointer("/ata_smart_data/self_test/status/string").and_then(Value::as_str).unwrap_or("").to_string();
    if json.pointer("/ata_smart_data/self_test/status/passed").and_then(Value::as_bool) == Some(false) {
        raise(Level::Critical, format!("self test failed: {self_test}"), &mut level);
    }
    // the exit status of smartctl: bit 3 = disk failing, bit 4 = a pre-fail attribute at or below its threshold
    let exit = json.pointer("/smartctl/exit_status").and_then(Value::as_i64).unwrap_or(0);
    if exit & 0x10 != 0 && level < Level::Critical {
        raise(Level::Critical, "a pre-fail attribute is at or below its threshold".into(), &mut level);
    }
    for (value, what) in [(pending, "pending sectors"), (reallocated, "reallocated sectors"), (offline, "offline uncorrectable sectors")] {
        if let Some(n) = value.filter(|n| *n > 0) {
            raise(Level::Warning, format!("{n} {what}"), &mut level);
        }
    }
    if let Some(n) = crc.filter(|n| *n > 0) {
        raise(Level::Warning, format!("{n} interface CRC errors (cable or controller)"), &mut level);
    }
    if exit & 0x40 != 0 {
        raise(Level::Warning, "the device error log has entries".into(), &mut level);
    }
    if exit & 0x80 != 0 {
        raise(Level::Warning, "the self test log has errors".into(), &mut level);
    }
    match temperature {
        Some(t) if t >= TEMP_CRITICAL => raise(Level::Critical, format!("temperature {t} °C"), &mut level),
        Some(t) if t >= TEMP_WARNING => raise(Level::Warning, format!("temperature {t} °C"), &mut level),
        _ => {}
    }
    Health {
        disk: disk.to_string(),
        model,
        level,
        reasons,
        temperature,
        power_on_hours,
        pending,
        reallocated,
        offline_uncorrectable: offline,
        crc_errors: crc,
        self_test,
    }
}

/// Why `smartctl` gave no SMART data (no permission, no SMART on the device), `None` when it did.
fn unreadable(json: &Value) -> Option<String> {
    if json.pointer("/ata_smart_attributes/table").is_some() || json.get("smart_status").is_some() || json.get("nvme_smart_health_information_log").is_some() {
        return None;
    }
    let messages: Vec<&str> = json
        .pointer("/smartctl/messages")
        .and_then(Value::as_array)
        .map(|m| m.iter().filter_map(|x| x["string"].as_str()).collect())
        .unwrap_or_default();
    Some(if messages.is_empty() { "smartctl returned no SMART data".to_string() } else { messages.join("; ") })
}

/// The mount source (`/dev/sdb`) of the longest mount point that holds `path`.
pub fn mount_source(mountinfo: &str, path: &Path) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for line in mountinfo.lines() {
        let Some((before, after)) = line.split_once(" - ") else { continue };
        let (Some(mount_point), Some(source)) = (before.split_whitespace().nth(4), after.split_whitespace().nth(1)) else { continue };
        let mount_point = mount_point.replace("\\040", " ");
        if path.starts_with(&mount_point) && source.starts_with("/dev/") && best.as_ref().is_none_or(|(len, _)| mount_point.len() >= *len) {
            best = Some((mount_point.len(), source.to_string()));
        }
    }
    best.map(|(_, s)| s)
}

/// The whole disk under block device `name` (`sdb1` → `sdb`, `nvme0n1p2` → `nvme0n1`), by `/sys/class/block`.
fn whole_disk(name: &str) -> Option<String> {
    let link = std::fs::canonicalize(Path::new("/sys/class/block").join(name)).ok()?;
    if link.join("partition").exists() {
        return link.parent()?.file_name()?.to_str().map(str::to_string);
    }
    link.file_name()?.to_str().map(str::to_string)
}

/// The disk under `path`, `None` when it is not a plain disk or partition (md, LVM, tmpfs, network).
pub fn disk_of(path: &Path) -> Option<String> {
    let path = std::fs::canonicalize(path).ok()?;
    let source = mount_source(&std::fs::read_to_string("/proc/self/mountinfo").ok()?, &path)?;
    let name = source.strip_prefix("/dev/")?;
    if name.starts_with("mapper/") || name.starts_with("md") || name.starts_with("dm-") {
        return None;
    }
    whole_disk(name)
}

/// Runs `smartctl --json=c -a /dev/<disk>`; the exit status bits are in the JSON.
fn smartctl(disk: &str) -> Result<Value, String> {
    let program = ["/usr/sbin/smartctl", "/usr/bin/smartctl", "/sbin/smartctl"]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .ok_or("smartctl (smartmontools) is not installed")?;
    let mut child = std::process::Command::new(program)
        .args(["--json=c", "-a", &format!("/dev/{disk}")])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if start.elapsed() > SMARTCTL_TIMEOUT => {
                let _ = child.kill();
                return Err("smartctl does not answer".into());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    serde_json::from_slice(&out.stdout).map_err(|e| format!("smartctl output: {e}"))
}

/// The health of the disk under `archive_device`, at most every ten minutes per disk.
pub fn of_device(archive_device: &Path) -> Result<Health, String> {
    static CACHE_MAP: Mutex<Option<HashMap<String, (Instant, Result<Health, String>)>>> = Mutex::new(None);
    let disk = disk_of(archive_device).ok_or("not a plain disk (md, LVM, network or memory file system)")?;
    let mut guard = CACHE_MAP.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some((at, result)) = map.get(&disk) && at.elapsed() < CACHE {
        return result.clone();
    }
    let result = smartctl(&disk).and_then(|json| match unreadable(&json) {
        Some(why) => Err(why),
        None => Ok(evaluate(&disk, &json)),
    });
    map.insert(disk, (Instant::now(), result.clone()));
    result
}

impl Health {
    /// One line for `status storage`.
    pub fn status_line(&self) -> String {
        let mut text = format!("Disk /dev/{} ({}): {}", self.disk, self.model, self.level.name());
        let value = |v: Option<i64>| v.map_or("?".to_string(), |n| n.to_string());
        text += &format!(
            ", {} °C, {} h, pending {}, reallocated {}, uncorrectable {}, CRC {}",
            value(self.temperature),
            value(self.power_on_hours),
            value(self.pending),
            value(self.reallocated),
            value(self.offline_uncorrectable),
            value(self.crc_errors)
        );
        if !self.reasons.is_empty() {
            text += &format!("; {}", self.reasons.join("; "));
        }
        text
    }

    /// The line `.status health` prints for `device`: `key=value` pairs, the reasons last.
    pub fn dot_line(&self, device: &str) -> String {
        let value = |v: Option<i64>| v.map_or("-".to_string(), |n| n.to_string());
        format!(
            "Device={device} Disk={} Level={} Temperature={} PowerOnHours={} Pending={} Reallocated={} OfflineUncorrectable={} CrcErrors={} SelfTest=\"{}\" Model=\"{}\" Reasons=\"{}\"\n",
            self.disk,
            self.level.name(),
            value(self.temperature),
            value(self.power_on_hours),
            value(self.pending),
            value(self.reallocated),
            value(self.offline_uncorrectable),
            value(self.crc_errors),
            self.self_test,
            self.model,
            self.reasons.join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `smartctl --json -a` of a healthy WD Red (the H340's `sdb`).
    fn healthy() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/smartctl-wd20efrx-sdb.json")).unwrap()
    }

    /// … of the H340's `sdd`: 232 pending sectors, one CRC error, error log entries.
    fn damaged() -> Value {
        serde_json::from_str(include_str!("../tests/fixtures/smartctl-wd20efrx-sdd.json")).unwrap()
    }

    #[test]
    fn a_healthy_disk_is_ok() {
        let h = evaluate("sdb", &healthy());
        assert_eq!(h.level, Level::Ok, "{h:?}");
        assert!(h.reasons.is_empty());
        assert_eq!((h.pending, h.reallocated, h.crc_errors), (Some(0), Some(0), Some(0)));
        assert_eq!(h.temperature, Some(32));
        assert!(h.model.starts_with("WDC WD20EFRX"));
    }

    #[test]
    fn pending_sectors_are_a_warning() {
        let h = evaluate("sdd", &damaged());
        assert_eq!(h.level, Level::Warning, "{h:?}");
        assert_eq!(h.pending, Some(232));
        assert!(h.reasons.iter().any(|r| r.contains("232 pending sectors")), "{:?}", h.reasons);
        assert!(h.reasons.iter().any(|r| r.contains("CRC")), "{:?}", h.reasons);
        assert!(h.status_line().contains("warning") && h.status_line().contains("pending 232"));
    }

    #[test]
    fn a_failed_smart_status_or_self_test_is_critical() {
        let mut j = healthy();
        j["smart_status"]["passed"] = Value::Bool(false);
        assert_eq!(evaluate("sdb", &j).level, Level::Critical);
        let mut j = healthy();
        j["ata_smart_data"]["self_test"]["status"]["passed"] = Value::Bool(false);
        assert_eq!(evaluate("sdb", &j).level, Level::Critical);
        let mut j = healthy();
        j["smartctl"]["exit_status"] = Value::from(16);
        assert_eq!(evaluate("sdb", &j).level, Level::Critical);
    }

    #[test]
    fn the_temperature_raises_the_level() {
        let mut j = healthy();
        j["temperature"]["current"] = Value::from(56);
        assert_eq!(evaluate("sdb", &j).level, Level::Warning);
        j["temperature"]["current"] = Value::from(61);
        assert_eq!(evaluate("sdb", &j).level, Level::Critical);
    }

    #[test]
    fn no_data_is_not_ok() {
        let j: Value = serde_json::from_str(r#"{"smartctl":{"exit_status":2,"messages":[{"string":"Smartctl open device: /dev/sdb failed: Permission denied","severity":"error"}]}}"#).unwrap();
        assert_eq!(unreadable(&j).as_deref(), Some("Smartctl open device: /dev/sdb failed: Permission denied"));
        assert_eq!(unreadable(&healthy()), None);
    }

    #[test]
    fn the_longest_mount_point_names_the_source() {
        let info = "25 1 8:1 / / rw - ext4 /dev/sda1 rw\n34 25 8:16 / /var/lib/bareos/storage rw,noatime - ext4 /dev/sdb rw\n35 25 0:30 / /var/lib/bareos rw - tmpfs tmpfs rw\n";
        assert_eq!(mount_source(info, Path::new("/var/lib/bareos/storage/Vol-0001")).as_deref(), Some("/dev/sdb"));
        assert_eq!(mount_source(info, Path::new("/home/x")).as_deref(), Some("/dev/sda1"));
        // a tmpfs has no /dev source
        assert_eq!(mount_source("1 0 0:1 / /mnt rw - tmpfs tmpfs rw\n", Path::new("/mnt/x")), None);
    }

    #[test]
    fn the_dot_line_is_key_value_pairs() {
        let line = evaluate("sdd", &damaged()).dot_line("H340File1");
        assert!(line.starts_with("Device=H340File1 Disk=sdd Level=warning "), "{line}");
        assert!(line.contains("Pending=232 ") && line.contains("Reasons=\"232 pending sectors"), "{line}");
    }
}
