//! Health of the disks under the devices of a Storage Daemon (Onesimus,
//! docs/plans/OSD-GERAETE-UND-VERTEILUNG.md, D-21).
//!
//! osd reads SMART of the disk under each File device and answers
//! `.status health` with one `key=value` line per device. For every Storage
//! resource that has `Health Check Interval` (an Onesimus directive in
//! `odir.d/storage/<Name>.conf`, so that `bareos-dir.d` stays standard; off when
//! absent) the Director asks that question at the interval, and
//!
//! * writes one row per device and check to `onesimus.medium_health` (schema
//!   step v2256) so that trends can be seen,
//! * sends a Director message through the Messages resource of the Director
//!   (the `Daemon` resource: mail, operator) when the verdict of a device
//!   changes, and for a first reading that is not `ok`: a warning for
//!   `warning` and `unknown`, an error for `critical`, an info when the
//!   device is `ok` again.
//!
//! The Director does not probe a Storage Daemon otherwise (watchdog.rs): a
//! probe is a connection in its log, so it is opt-in per Storage.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::catalog_worker::CatalogHandle;
use bareos_config::resources::StorageResource;

use crate::director_config::SharedConfig;
use crate::message_bus::MsgType;

/// How often the watcher looks at the configuration for due checks.
const POLL: Duration = Duration::from_secs(30);

/// Time limit of the catalog write.
const CATALOG_TIMEOUT: Duration = Duration::from_secs(10);

/// What `.status health` says about one device.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceHealth {
    /// `Device=`.
    pub device: String,
    /// `Disk=`.
    pub disk: String,
    /// `Level=`: ok, warning, critical or unknown.
    pub level: String,
    /// `Model=`.
    pub model: String,
    /// `Temperature=` (`-` is none).
    pub temperature: Option<i64>,
    /// `PowerOnHours=`.
    pub power_on_hours: Option<i64>,
    /// `Pending=`.
    pub pending: Option<i64>,
    /// `Reallocated=`.
    pub reallocated: Option<i64>,
    /// `OfflineUncorrectable=`.
    pub offline_uncorrectable: Option<i64>,
    /// `CrcErrors=`.
    pub crc_errors: Option<i64>,
    /// `SelfTest=`.
    pub self_test: String,
    /// `Reasons=`.
    pub reasons: String,
}

/// Splits a `key=value key="quoted value"` line.
fn tokens(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 1..];
        let (value, tail) = if let Some(quoted) = after.strip_prefix('"') {
            match quoted.find('"') {
                Some(end) => (quoted[..end].to_string(), &quoted[end + 1..]),
                None => (quoted.to_string(), ""),
            }
        } else {
            match after.find(char::is_whitespace) {
                Some(end) => (after[..end].to_string(), &after[end..]),
                None => (after.to_string(), ""),
            }
        };
        out.push((key, value));
        rest = tail.trim_start();
    }
    out
}

/// One line of the answer; `None` for a line without `Device=` (the `3000 OK` header).
pub fn parse_line(line: &str) -> Option<DeviceHealth> {
    let mut h = DeviceHealth::default();
    let number = |v: &str| v.parse::<i64>().ok();
    for (key, value) in tokens(line) {
        match key.as_str() {
            "Device" => h.device = value,
            "Disk" => h.disk = value,
            "Level" => h.level = value,
            "Model" => h.model = value,
            "Temperature" => h.temperature = number(&value),
            "PowerOnHours" => h.power_on_hours = number(&value),
            "Pending" => h.pending = number(&value),
            "Reallocated" => h.reallocated = number(&value),
            "OfflineUncorrectable" => h.offline_uncorrectable = number(&value),
            "CrcErrors" => h.crc_errors = number(&value),
            "SelfTest" => h.self_test = value,
            "Reasons" => h.reasons = value,
            _ => {}
        }
    }
    (!h.device.is_empty()).then_some(h)
}

/// Asks the Storage Daemon `store` `.status health`.
pub fn probe(store: &StorageResource, director: &str) -> Result<Vec<DeviceHealth>, String> {
    use bareos_bnet::{BnetMessage, BnetSignal};
    let mut sd = osd::connect::connect_sd(&store.address, store.port, &store.password, director)?;
    sd.send_str(".status health\n").map_err(|e| e.to_string())?;
    let mut devices = Vec::new();
    while let Ok(BnetMessage::Data(data)) = sd.recv() {
        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
        let text = String::from_utf8_lossy(&data[..end]);
        if text.starts_with("3900") {
            let _ = sd.send_signal(BnetSignal::Terminate);
            return Err(format!("the Storage Daemon does not know `.status health`: {}", text.trim_end()));
        }
        devices.extend(text.lines().filter_map(parse_line));
    }
    let _ = sd.send_signal(BnetSignal::Terminate);
    Ok(devices)
}

/// SQL string literal.
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A number or NULL.
fn number(v: Option<i64>) -> String {
    v.map_or("NULL".to_string(), |n| n.to_string())
}

/// The `INSERT` of one check.
pub fn insert_sql(storage: &str, h: &DeviceHealth) -> String {
    format!(
        "INSERT INTO onesimus.medium_health (storage, device, disk, model, level, reasons, temperature, power_on_hours, \
         pending_sectors, reallocated_sectors, offline_uncorrectable, crc_errors, self_test) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        quote(storage),
        quote(&h.device),
        quote(&h.disk),
        quote(&h.model),
        quote(&h.level),
        quote(&h.reasons),
        number(h.temperature),
        number(h.power_on_hours),
        number(h.pending),
        number(h.reallocated),
        number(h.offline_uncorrectable),
        number(h.crc_errors),
        quote(&h.self_test)
    )
}

/// The Director message for a new reading, `None` when nothing is to be said:
/// the verdict changed, or the first reading is not `ok`.
pub fn change_message(storage: &str, previous: Option<&str>, h: &DeviceHealth) -> Option<(MsgType, String)> {
    let changed = match previous {
        Some(p) => p != h.level,
        None => h.level != "ok",
    };
    if !changed {
        return None;
    }
    let kind = match h.level.as_str() {
        "critical" => MsgType::Error,
        "ok" => MsgType::Info,
        _ => MsgType::Warning,
    };
    let mut text = format!("Storage \"{storage}\", device \"{}\": disk /dev/{} ({}) is {}", h.device, h.disk, h.model, h.level);
    if let Some(p) = previous {
        text += &format!(" (was {p})");
    }
    if !h.reasons.is_empty() {
        text += &format!(": {}", h.reasons);
    }
    let value = |v: Option<i64>| v.map_or("?".to_string(), |n| n.to_string());
    text += &format!(
        ". {} °C, {} h, pending {}, reallocated {}, uncorrectable {}, CRC {}\n",
        value(h.temperature),
        value(h.power_on_hours),
        value(h.pending),
        value(h.reallocated),
        value(h.offline_uncorrectable),
        value(h.crc_errors)
    );
    Some((kind, text))
}

/// The `Health Check Interval` of a Storage resource, when set and not zero.
fn interval(config: &crate::director_config::DirectorConfig, store: &StorageResource) -> Option<Duration> {
    let raw = config.extension("storage", &store.name, "healthcheckinterval")?;
    let (_, d) = bareos_config::values::parse_time(&raw).ok()?;
    (!d.is_zero()).then_some(d)
}

/// Starts the watcher thread.
pub fn spawn_watcher(config: SharedConfig, catalog: Option<CatalogHandle>) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name("dir:storage-health".into()).spawn(move || {
        let mut last: HashMap<String, Instant> = HashMap::new();
        let mut levels: HashMap<(String, String), String> = HashMap::new();
        loop {
            std::thread::sleep(POLL);
            let due: Vec<(StorageResource, String)> = match config.read() {
                Ok(c) => c
                    .storages
                    .iter()
                    .filter(|s| s.enabled)
                    .filter_map(|s| {
                        let every = interval(&c, s)?;
                        last.get(&s.name).is_none_or(|t| t.elapsed() >= every).then(|| (s.clone(), c.director.name.clone()))
                    })
                    .collect(),
                Err(_) => continue,
            };
            for (store, director) in due {
                last.insert(store.name.clone(), Instant::now());
                let devices = match probe(&store, &director) {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!("health check of Storage {}: {e}", store.name);
                        continue;
                    }
                };
                for h in devices {
                    let key = (store.name.clone(), h.device.clone());
                    if let Some((kind, text)) = change_message(&store.name, levels.get(&key).map(String::as_str), &h)
                        && let Ok(c) = config.read()
                    {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
                        crate::console_msg::job_message(&c, "*StorageHealth*", 0, kind, now, &text);
                    }
                    if let Some(catalog) = &catalog
                        && let Err(e) = catalog.execute_within(&insert_sql(&store.name, &h), CATALOG_TIMEOUT)
                    {
                        tracing::debug!("health history of {}/{} not written: {e}", store.name, h.device);
                    }
                    levels.insert(key, h.level);
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "Device=H340File1 Disk=sdd Level=warning Temperature=36 PowerOnHours=54844 Pending=232 Reallocated=0 \
                        OfflineUncorrectable=- CrcErrors=1 SelfTest=\"completed without error\" Model=\"WDC WD20EFRX-68EUZN0\" \
                        Reasons=\"232 pending sectors; 1 interface CRC errors (cable or controller)\"\n";

    #[test]
    fn a_line_of_the_answer_is_parsed() {
        let h = parse_line(LINE).unwrap();
        assert_eq!((h.device.as_str(), h.disk.as_str(), h.level.as_str()), ("H340File1", "sdd", "warning"));
        assert_eq!((h.temperature, h.power_on_hours, h.pending, h.reallocated, h.offline_uncorrectable, h.crc_errors),
                   (Some(36), Some(54844), Some(232), Some(0), None, Some(1)));
        assert_eq!(h.model, "WDC WD20EFRX-68EUZN0");
        assert_eq!(h.self_test, "completed without error");
        assert_eq!(h.reasons, "232 pending sectors; 1 interface CRC errors (cable or controller)");
        assert!(parse_line("3000 OK .status\n").is_none());
        let unknown = parse_line("Device=H340File2 Level=unknown Reasons=\"Permission denied\"\n").unwrap();
        assert_eq!((unknown.level.as_str(), unknown.disk.as_str(), unknown.pending), ("unknown", "", None));
    }

    #[test]
    fn the_history_row_is_quoted() {
        let mut h = parse_line(LINE).unwrap();
        h.model = "O'Brien".into();
        let sql = insert_sql("H340", &h);
        assert!(sql.starts_with("INSERT INTO onesimus.medium_health (storage, device,"), "{sql}");
        assert!(sql.contains("'H340', 'H340File1', 'sdd', 'O''Brien', 'warning'"), "{sql}");
        assert!(sql.contains("36, 54844, 232, 0, NULL, 1,"), "{sql}");
    }

    #[test]
    fn messages_follow_the_changes_only() {
        let h = parse_line(LINE).unwrap();
        // the first reading of a warning is reported, of an ok device it is not
        let (kind, text) = change_message("H340", None, &h).unwrap();
        assert_eq!(kind, MsgType::Warning);
        assert!(text.contains("is warning") && text.contains("232 pending sectors") && !text.contains("was "), "{text}");
        let mut ok = h.clone();
        ok.level = "ok".into();
        assert!(change_message("H340", None, &ok).is_none());
        // no change, no message
        assert!(change_message("H340", Some("warning"), &h).is_none());
        // a change is reported with the old verdict; critical is an error, ok again an info
        let mut critical = h.clone();
        critical.level = "critical".into();
        let (kind, text) = change_message("H340", Some("warning"), &critical).unwrap();
        assert_eq!(kind, MsgType::Error);
        assert!(text.contains("(was warning)"), "{text}");
        let (kind, _) = change_message("H340", Some("critical"), &ok).unwrap();
        assert_eq!(kind, MsgType::Info);
    }
}
