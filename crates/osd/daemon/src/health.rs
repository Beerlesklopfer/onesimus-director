//! Gesundheit der Platten unter den File-Devices (D-21, erste Umsetzung).
//!
//! Zweck: osd findet zu jedem File-Device die Platte, auf der sein
//! `Archive Device` liegt (Einhängepunkt in `/proc/self/mountinfo`, bei einer
//! Partition die Platte darüber in `/sys/class/block`), fragt SMART über
//! das Crate `smartread` ab (SG_IO/SAT und NVMe, ohne `smartctl`) und bewertet
//! das Ergebnis mit `ok`, `warning` oder `critical`. `status storage` zeigt den
//! Zustand je Device, `.status health` liefert ihn zeilenweise zum Auswerten
//! (odir). Die Abfrage läuft höchstens alle zehn Minuten je Platte.
//!
//! Grenzen der ersten Umsetzung: nur ATA/SATA und NVMe (SAS liest `smartread`
//! noch nicht); eine Platte hinter md, LVM oder btrfs über mehrere Platten
//! bleibt `unknown`; SMART braucht root oder `CAP_SYS_RAWIO` (NVMe
//! `CAP_SYS_ADMIN`). Fehlerprotokoll und Selbsttestprotokoll der Platte
//! liest `smartread` nicht; die Bewertung stützt sich auf Attribute,
//! Gesamturteil und Ergebnis des letzten Selbsttests.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a result is reused.
const CACHE: Duration = Duration::from_secs(600);

/// Temperature (°C) from which a disk is `warning` and `critical`.
const TEMP_WARNING: i64 = 55;
const TEMP_CRITICAL: i64 = 60;

/// The verdict on a disk (D-21: ok / warnung / kritisch / ausgefallen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// No finding.
    Ok,
    /// Growing defects, high temperature.
    Warning,
    /// SMART says FAILED, a failed self test, a pre-fail attribute at its threshold.
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

/// Evaluates what `smartread` read from `disk`.
pub fn evaluate(disk: &str, report: &smartread::Report) -> Health {
    let mut level = Level::Ok;
    let mut reasons = Vec::new();
    let mut raise = |to: Level, why: String, level: &mut Level| {
        if to > *level {
            *level = to;
        }
        reasons.push(why);
    };
    let raw = |id: u8| report.attribute(id).map(|a| a.raw as i64);
    let (pending, reallocated, offline, crc) = (raw(197), raw(5), raw(198), raw(199));
    if report.health == smartread::Health::Failed {
        raise(Level::Critical, "SMART overall health: FAILED".into(), &mut level);
    }
    let self_test = report.self_test.as_ref().map(|t| t.text().to_string()).unwrap_or_default();
    // an aborted (1), interrupted (2) or running (15) test is no failure
    if let Some(t) = report.self_test.as_ref().filter(|t| !t.passed() && !matches!(t.status, 1 | 2 | 15)) {
        raise(Level::Critical, format!("self test failed: {}", t.text()), &mut level);
    }
    for a in report.attributes.iter().filter(|a| a.is_prefail() && a.failing()) {
        raise(Level::Critical, format!("attribute {} ({}) is at its threshold", a.id, a.name().unwrap_or("?")), &mut level);
    }
    for (value, what) in [(pending, "pending sectors"), (reallocated, "reallocated sectors"), (offline, "offline uncorrectable sectors")] {
        if let Some(n) = value.filter(|n| *n > 0) {
            raise(Level::Warning, format!("{n} {what}"), &mut level);
        }
    }
    if let Some(n) = crc.filter(|n| *n > 0) {
        raise(Level::Warning, format!("{n} interface CRC errors (cable or controller)"), &mut level);
    }
    if let Some(n) = report.nvme.as_ref() {
        if n.failing() {
            raise(Level::Critical, format!("NVMe critical warning {:#04x} or spare below threshold", n.critical_warning), &mut level);
        }
        if n.media_errors > 0 {
            raise(Level::Warning, format!("{} media and data integrity errors", n.media_errors), &mut level);
        }
        if n.percentage_used >= 90 {
            raise(Level::Warning, format!("{} % of the endurance used", n.percentage_used), &mut level);
        }
    }
    let temperature = report.temperature.map(i64::from);
    match temperature {
        Some(t) if t >= TEMP_CRITICAL => raise(Level::Critical, format!("temperature {t} °C"), &mut level),
        Some(t) if t >= TEMP_WARNING => raise(Level::Warning, format!("temperature {t} °C"), &mut level),
        _ => {}
    }
    Health {
        disk: disk.to_string(),
        model: report.model.clone(),
        level,
        reasons,
        temperature,
        power_on_hours: report.power_on_hours.map(|h| h as i64),
        pending,
        reallocated,
        offline_uncorrectable: offline,
        crc_errors: crc,
        self_test,
    }
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

/// The health of the disk under `archive_device`, at most every ten minutes per disk.
pub fn of_device(archive_device: &Path) -> Result<Health, String> {
    static CACHE_MAP: Mutex<Option<HashMap<String, (Instant, Result<Health, String>)>>> = Mutex::new(None);
    let disk = disk_of(archive_device).ok_or("not a plain disk (md, LVM, network or memory file system)")?;
    let mut guard = CACHE_MAP.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some((at, result)) = map.get(&disk) && at.elapsed() < CACHE {
        return result.clone();
    }
    let result = smartread::read(format!("/dev/{disk}")).map(|report| evaluate(&disk, &report)).map_err(|e| e.to_string());
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
    use smartread::{Attribute, Report, SelfTest};

    /// An attribute with the common flags (pre-fail for 1, 3 and 5).
    fn attr(id: u8, value: u8, threshold: u8, raw: u64) -> Attribute {
        let prefail = matches!(id, 1 | 3 | 5);
        Attribute { id, flags: if prefail { 0x33 } else { 0x32 }, value, worst: value, raw, threshold: Some(threshold) }
    }

    /// A healthy WD Red as the H340's `sdb`.
    fn healthy() -> Report {
        Report {
            protocol: "ata",
            model: "WDC WD20EFRX-68EUZN0".into(),
            serial: String::new(),
            firmware: String::new(),
            health: smartread::Health::Passed,
            temperature: Some(32),
            power_on_hours: Some(54879),
            attributes: vec![attr(5, 200, 140, 0), attr(9, 25, 0, 54879), attr(194, 115, 0, 32), attr(197, 200, 0, 0), attr(198, 100, 0, 0), attr(199, 200, 0, 0)],
            self_test: Some(SelfTest { status: 0, remaining_percent: 0 }),
            nvme: None,
        }
    }

    /// … with the values of the H340's `sdd`: 232 pending sectors, one CRC error.
    fn damaged() -> Report {
        let mut r = healthy();
        r.attributes = vec![attr(5, 200, 140, 0), attr(197, 200, 0, 232), attr(199, 200, 0, 1), attr(194, 111, 0, 36)];
        r.temperature = Some(36);
        r
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
    fn a_failed_status_self_test_or_threshold_is_critical() {
        let mut r = healthy();
        r.health = smartread::Health::Failed;
        assert_eq!(evaluate("sdb", &r).level, Level::Critical);
        let mut r = healthy();
        r.self_test = Some(SelfTest { status: 7, remaining_percent: 0 });
        assert_eq!(evaluate("sdb", &r).level, Level::Critical);
        // an aborted test is no failure
        let mut r = healthy();
        r.self_test = Some(SelfTest { status: 1, remaining_percent: 0 });
        assert_eq!(evaluate("sdb", &r).level, Level::Ok);
        let mut r = healthy();
        r.attributes[0] = attr(5, 140, 140, 0); // reallocated sectors at the threshold
        assert_eq!(evaluate("sdb", &r).level, Level::Critical);
    }

    #[test]
    fn the_temperature_raises_the_level() {
        let mut r = healthy();
        r.temperature = Some(56);
        assert_eq!(evaluate("sdb", &r).level, Level::Warning);
        r.temperature = Some(61);
        assert_eq!(evaluate("sdb", &r).level, Level::Critical);
    }

    #[test]
    fn nvme_warnings_and_media_errors() {
        let mut sector = vec![0u8; 512];
        sector[1..3].copy_from_slice(&310u16.to_le_bytes());
        sector[3] = 100;
        sector[4] = 10;
        let mut log = smartread::nvme::parse_health_log(&sector).unwrap();
        let mut r = healthy();
        r.protocol = "nvme";
        r.attributes.clear();
        r.self_test = None;
        r.nvme = Some(log);
        assert_eq!(evaluate("nvme0n1", &r).level, Level::Ok);
        log.media_errors = 2;
        r.nvme = Some(log);
        assert_eq!(evaluate("nvme0n1", &r).level, Level::Warning);
        log.critical_warning = 0x04;
        r.nvme = Some(log);
        assert_eq!(evaluate("nvme0n1", &r).level, Level::Critical);
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
