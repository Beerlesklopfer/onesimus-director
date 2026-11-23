//! SCSI / SAS drives: `INQUIRY` and the SMART related log pages (`LOG SENSE`).
//!
//! SAS and Fibre Channel disks, and SCSI disks in general, do not have the ATA
//! SMART commands. They report their health in log pages (SPC-4):
//!
//! | Page | Content used here |
//! |---|---|
//! | `0x2F` Informational Exceptions | the *additional sense code* of a predicted failure (`ASC 0x5D`: failure prediction threshold exceeded) and the most recent temperature |
//! | `0x0D` Temperature | the current temperature |
//! | `0x10` Self-Test Results | the last self test |
//! | `0x02`, `0x03`, `0x05` Write, Read, Verify Error Counters | *total uncorrected errors* (parameter `0x0006`) |
//!
//! and the grown defect list (`READ DEFECT DATA(10)`, GLIST). Identity comes
//! from `INQUIRY` (vendor, product, revision) and the unit serial number VPD
//! page `0x80`.
//!
//! **Not verified on a SAS drive.** The author has no SAS hardware. The
//! parsers follow the byte layouts of SPC-4 and are tested on pages built from
//! those layouts; the commands were checked for their plumbing against SATA
//! disks (which answer `INQUIRY` through SAT). Reports from real SAS drives
//! are welcome (<contact@onesimus.io>).

use crate::{Error, Health, Report, SelfTest};

/// What the SCSI log pages say besides health, temperature and self test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Summary {
    /// Informational exceptions: additional sense code (`0x5D` = failure predicted), 0 = none.
    pub exception_asc: u8,
    /// Informational exceptions: additional sense code qualifier.
    pub exception_ascq: u8,
    /// Entries in the grown defect list.
    pub grown_defects: Option<u32>,
    /// Total uncorrected read errors.
    pub uncorrected_read: Option<u64>,
    /// Total uncorrected write errors.
    pub uncorrected_write: Option<u64>,
    /// Total uncorrected verify errors.
    pub uncorrected_verify: Option<u64>,
}

/// The parameters of a log page: (parameter code, value bytes).
pub fn log_parameters(page: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    if page.len() < 4 {
        return out;
    }
    let length = usize::from(u16::from_be_bytes([page[2], page[3]])).min(page.len() - 4);
    let body = &page[4..4 + length];
    let mut at = 0;
    while at + 4 <= body.len() {
        let code = u16::from_be_bytes([body[at], body[at + 1]]);
        let n = usize::from(body[at + 3]);
        if at + 4 + n > body.len() {
            break;
        }
        out.push((code, &body[at + 4..at + 4 + n]));
        at += 4 + n;
    }
    out
}

/// The value of a counter parameter (1 to 8 bytes, big endian).
fn counter(bytes: &[u8]) -> u64 {
    bytes.iter().take(8).fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
}

/// Informational Exceptions page `0x2F`: (ASC, ASCQ, most recent temperature in °C when valid).
pub fn parse_informational_exceptions(page: &[u8]) -> Option<(u8, u8, Option<i32>)> {
    let (_, v) = log_parameters(page).into_iter().find(|(c, _)| *c == 0)?;
    if v.len() < 2 {
        return None;
    }
    let temperature = v.get(2).copied().filter(|t| *t != 0xFF).map(i32::from);
    Some((v[0], v[1], temperature))
}

/// Temperature page `0x0D`: the current temperature in °C (parameter 0, byte 1; `0xFF` means invalid).
pub fn parse_temperature(page: &[u8]) -> Option<i32> {
    let (_, v) = log_parameters(page).into_iter().find(|(c, _)| *c == 0)?;
    v.get(1).copied().filter(|t| *t != 0xFF).map(i32::from)
}

/// Self-Test Results page `0x10`: the most recent entry with data (parameter 1 is the newest).
///
/// The result code is mapped to the ATA codes of [`SelfTest`]: 0 completed → 0,
/// 1 or 2 aborted → 1, 3 unknown error → 4, 4 to 7 failed in a segment → 7, 15 in progress → 15.
pub fn parse_self_test(page: &[u8]) -> Option<SelfTest> {
    let mut entries: Vec<(u16, &[u8])> = log_parameters(page).into_iter().filter(|(c, v)| (1..=20).contains(c) && v.len() >= 4).collect();
    entries.sort_by_key(|(c, _)| *c);
    for (_, v) in entries {
        // an unused entry is all zero
        if v.iter().all(|b| *b == 0) {
            continue;
        }
        let status = match v[0] & 0x0F {
            0 => 0,
            1 | 2 => 1,
            3 => 4,
            4..=7 => 7,
            0xF => 15,
            _ => 4,
        };
        return Some(SelfTest { status, remaining_percent: 0 });
    }
    None
}

/// Error counter pages `0x02`, `0x03`, `0x05`: *total uncorrected errors* (parameter `0x0006`).
pub fn parse_uncorrected_errors(page: &[u8]) -> Option<u64> {
    log_parameters(page).into_iter().find(|(c, _)| *c == 6).map(|(_, v)| counter(v))
}

/// The number of entries in the answer of `READ DEFECT DATA(10)` (8 bytes each, list length in bytes 2 and 3).
pub fn parse_defect_count(data: &[u8]) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    Some(u32::from(u16::from_be_bytes([data[2], data[3]])) / 8)
}

/// A string from a fixed INQUIRY field.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Vendor, product and revision from a standard INQUIRY answer (bytes 8–15, 16–31, 32–35).
pub fn parse_inquiry(data: &[u8]) -> Result<(String, String, String), Error> {
    if data.len() < 36 {
        return Err(Error::Device(format!("INQUIRY answer has {} bytes", data.len())));
    }
    Ok((text(&data[8..16]), text(&data[16..32]), text(&data[32..36])))
}

/// The serial number from the Unit Serial Number VPD page `0x80` (bytes 4 on, length in byte 3).
pub fn parse_serial(page: &[u8]) -> String {
    if page.len() < 4 {
        return String::new();
    }
    let n = usize::from(page[3]).min(page.len() - 4);
    text(&page[4..4 + n])
}

/// A [`Report`] from the pages that could be read.
#[allow(clippy::too_many_arguments)]
pub fn report(
    vendor: &str,
    product: &str,
    revision: &str,
    serial: String,
    exceptions: Option<(u8, u8, Option<i32>)>,
    temperature: Option<i32>,
    self_test: Option<SelfTest>,
    summary: Summary,
) -> Report {
    let health = match exceptions {
        Some((0, _, _)) => Health::Passed,
        Some(_) => Health::Failed,
        None => Health::Unknown,
    };
    let temperature = temperature.or(exceptions.and_then(|e| e.2));
    Report {
        protocol: "scsi",
        model: format!("{vendor} {product}").trim().to_string(),
        serial,
        firmware: revision.to_string(),
        health,
        temperature,
        power_on_hours: None,
        attributes: Vec::new(),
        self_test,
        nvme: None,
        scsi: Some(summary),
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::sg;
    use std::fs::File;

    /// Sends a SCSI command with a data-in buffer of `len` bytes; the data only when the drive answered GOOD.
    fn command(file: &File, mut cdb: Vec<u8>, len: usize) -> Result<Vec<u8>, Error> {
        let mut data = vec![0u8; len];
        let done = sg::request(file, &mut cdb, Some(&mut data))?;
        if done.host_status != 0 || done.status != 0 {
            return Err(Error::Device(format!("SCSI command {:#04x} failed (status {:#x}, host {:#x})", cdb[0], done.status, done.host_status)));
        }
        Ok(data)
    }

    /// `INQUIRY`, standard or the VPD page `page` when `Some`.
    pub fn inquiry(file: &File, page: Option<u8>) -> Result<Vec<u8>, Error> {
        let mut cdb = vec![0x12, u8::from(page.is_some()), page.unwrap_or(0), 0x00, 0xFF, 0x00];
        cdb.truncate(6);
        command(file, cdb, 255)
    }

    /// `LOG SENSE` of `page` (cumulative values).
    fn log_sense(file: &File, page: u8) -> Result<Vec<u8>, Error> {
        let cdb = vec![0x4D, 0x00, 0x40 | page, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00];
        command(file, cdb, 4096)
    }

    /// `READ DEFECT DATA(10)` with the grown defect list (GLIST) in the format *bytes from index*.
    fn defects(file: &File) -> Result<Vec<u8>, Error> {
        let cdb = vec![0x37, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00];
        command(file, cdb, 4096)
    }

    /// Reads a SCSI / SAS drive.
    pub fn read(file: &File) -> Result<Report, Error> {
        let (vendor, product, revision) = parse_inquiry(&inquiry(file, None)?)?;
        let serial = inquiry(file, Some(0x80)).map(|p| parse_serial(&p)).unwrap_or_default();
        let exceptions = log_sense(file, 0x2F).ok().and_then(|p| parse_informational_exceptions(&p));
        let temperature = log_sense(file, 0x0D).ok().and_then(|p| parse_temperature(&p));
        let self_test = log_sense(file, 0x10).ok().and_then(|p| parse_self_test(&p));
        let summary = Summary {
            exception_asc: exceptions.map_or(0, |e| e.0),
            exception_ascq: exceptions.map_or(0, |e| e.1),
            grown_defects: defects(file).ok().and_then(|d| parse_defect_count(&d)),
            uncorrected_write: log_sense(file, 0x02).ok().and_then(|p| parse_uncorrected_errors(&p)),
            uncorrected_read: log_sense(file, 0x03).ok().and_then(|p| parse_uncorrected_errors(&p)),
            uncorrected_verify: log_sense(file, 0x05).ok().and_then(|p| parse_uncorrected_errors(&p)),
        };
        if exceptions.is_none() && temperature.is_none() && self_test.is_none() {
            return Err(Error::Device("the drive answers none of the SMART log pages (0x2F, 0x0D, 0x10)".into()));
        }
        Ok(report(&vendor, &product, &revision, serial, exceptions, temperature, self_test, summary))
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::{inquiry, read};

#[cfg(test)]
mod tests {
    use super::*;

    /// A log page with the parameters `(code, value)`.
    fn page(code: u8, params: &[(u16, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (c, v) in params {
            body.extend_from_slice(&c.to_be_bytes());
            body.push(0x03);
            body.push(v.len() as u8);
            body.extend_from_slice(v);
        }
        let mut p = vec![code, 0];
        p.extend_from_slice(&(body.len() as u16).to_be_bytes());
        p.extend(body);
        p
    }

    #[test]
    fn the_informational_exceptions_page_gives_asc_and_temperature() {
        let ok = page(0x2F, &[(0, &[0x00, 0x00, 38])]);
        assert_eq!(parse_informational_exceptions(&ok), Some((0, 0, Some(38))));
        let failing = page(0x2F, &[(0, &[0x5D, 0x10, 0xFF])]);
        assert_eq!(parse_informational_exceptions(&failing), Some((0x5D, 0x10, None)));
        assert_eq!(parse_informational_exceptions(&page(0x2F, &[])), None);
        assert_eq!(parse_informational_exceptions(&[1, 2]), None);
    }

    #[test]
    fn the_temperature_page_is_read() {
        assert_eq!(parse_temperature(&page(0x0D, &[(0, &[0, 41]), (1, &[0, 65])])), Some(41));
        assert_eq!(parse_temperature(&page(0x0D, &[(0, &[0, 0xFF])])), None);
    }

    #[test]
    fn the_newest_self_test_with_data_decides() {
        let used = [0x10u8; 0x10]; // result 0 (code bits 7-5 = 0), but not all zero
        let mut failed = [0u8; 0x10];
        failed[0] = 0x07; // failed in another segment
        failed[2] = 0x12;
        // entry 1 unused, entry 2 failed, entry 3 completed: entry 2 is the newest with data
        let p = page(0x10, &[(1, &[0u8; 0x10]), (2, &failed), (3, &used)]);
        assert_eq!(parse_self_test(&p), Some(SelfTest { status: 7, remaining_percent: 0 }));
        let mut ok = [0u8; 0x10];
        ok[2] = 0x05;
        assert_eq!(parse_self_test(&page(0x10, &[(1, &ok)])).unwrap().status, 0);
        let mut running = [0u8; 0x10];
        running[0] = 0x0F;
        assert_eq!(parse_self_test(&page(0x10, &[(1, &running)])).unwrap().status, 15);
        assert_eq!(parse_self_test(&page(0x10, &[(1, &[0u8; 0x10])])), None);
    }

    #[test]
    fn the_error_counter_and_defect_list_are_read() {
        let counters = page(0x03, &[(0, &[0, 0, 0, 5]), (6, &[0, 0, 0, 0, 0, 0, 1, 2])]);
        assert_eq!(parse_uncorrected_errors(&counters), Some(0x0102));
        assert_eq!(parse_uncorrected_errors(&page(0x03, &[(0, &[0])])), None);
        // header: 0, format, list length 24 bytes = 3 entries of 8
        assert_eq!(parse_defect_count(&[0, 0x04, 0, 24]), Some(3));
        assert_eq!(parse_defect_count(&[0, 0]), None);
    }

    #[test]
    fn inquiry_and_serial_are_decoded() {
        let mut d = vec![b' '; 36];
        d[8..11].copy_from_slice(b"SEA");
        d[16..26].copy_from_slice(b"ST4000NM00");
        d[32..36].copy_from_slice(b"0004");
        assert_eq!(parse_inquiry(&d).unwrap(), ("SEA".into(), "ST4000NM00".into(), "0004".into()));
        assert!(parse_inquiry(&d[..10]).is_err());
        assert_eq!(parse_serial(&[0, 0x80, 0, 4, b'Z', b'1', b'0', b' ']), "Z10");
    }

    #[test]
    fn the_report_follows_the_informational_exceptions() {
        let fine = report("SEAGATE", "ST4000NM", "0004", "Z10".into(), Some((0, 0, Some(40))), None, None, Summary::default());
        assert_eq!((fine.health, fine.temperature, fine.protocol), (Health::Passed, Some(40), "scsi"));
        assert_eq!(fine.model, "SEAGATE ST4000NM");
        let failing = report("SEAGATE", "ST4000NM", "0004", String::new(), Some((0x5D, 0, None)), Some(30), None, Summary { exception_asc: 0x5D, ..Default::default() });
        assert_eq!((failing.health, failing.temperature), (Health::Failed, Some(30)));
        let unknown = report("X", "Y", "1", String::new(), None, Some(30), None, Summary::default());
        assert_eq!(unknown.health, Health::Unknown);
    }
}
