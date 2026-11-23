//! ATA/SATA drives: `IDENTIFY DEVICE` and the SMART commands through
//! `SG_IO`, and the parsers of their 512-byte answers.
//!
//! # How the drive is asked
//!
//! The ATA commands travel inside the SCSI command *ATA PASS-THROUGH(16)*
//! (opcode `0x85`, SAT-5). SATA disks behind a SAS HBA or an AHCI controller
//! answer it directly; USB bridges answer it when they implement SAT (many
//! do, some need the 12-byte variant or vendor commands, which this crate
//! does not try).
//!
//! | Command | ATA | Feature | Data |
//! |---|---|---|---|
//! | IDENTIFY DEVICE | `0xEC` | – | 512 bytes in |
//! | SMART READ DATA | `0xB0` | `0xD0` | 512 bytes in |
//! | SMART READ THRESHOLDS | `0xB0` | `0xD1` | 512 bytes in |
//! | SMART RETURN STATUS | `0xB0` | `0xDA` | none; the answer is in the registers |
//!
//! `SMART RETURN STATUS` puts `0x4F`/`0xC2` into the LBA mid/high registers
//! when the drive is fine and `0xF4`/`0x2C` when a threshold is exceeded. The
//! registers come back in the sense data (*ATA Status Return* descriptor),
//! so the command is sent with `CK_COND` set.
//!
//! # The parsers
//!
//! [`parse_identify`], [`parse_smart_data`] and [`parse_thresholds`] take the
//! raw sectors and need no hardware; the tests use sectors read from real
//! drives.

use crate::{Attribute, Error, Health, Report, SelfTest};

/// The 12 attribute slots of 12 bytes in a SMART data sector start at byte 2.
const ATTRIBUTE_OFFSET: usize = 2;
/// Number of attribute slots.
const ATTRIBUTE_SLOTS: usize = 30;
/// The self-test execution status byte of the SMART data sector.
const SELF_TEST_STATUS: usize = 363;

/// What `IDENTIFY DEVICE` says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Model (words 27–46).
    pub model: String,
    /// Serial number (words 10–19).
    pub serial: String,
    /// Firmware revision (words 23–26).
    pub firmware: String,
    /// SMART is supported (word 82 bit 0).
    pub smart_supported: bool,
    /// SMART is enabled (word 85 bit 0).
    pub smart_enabled: bool,
}

/// The conventional name of attribute `id` (the common, vendor independent ones).
pub fn attribute_name(id: u8) -> Option<&'static str> {
    Some(match id {
        1 => "Raw_Read_Error_Rate",
        3 => "Spin_Up_Time",
        4 => "Start_Stop_Count",
        5 => "Reallocated_Sector_Ct",
        7 => "Seek_Error_Rate",
        9 => "Power_On_Hours",
        10 => "Spin_Retry_Count",
        12 => "Power_Cycle_Count",
        177 => "Wear_Leveling_Count",
        187 => "Reported_Uncorrect",
        188 => "Command_Timeout",
        190 => "Airflow_Temperature_Cel",
        192 => "Power-Off_Retract_Count",
        193 => "Load_Cycle_Count",
        194 => "Temperature_Celsius",
        196 => "Reallocated_Event_Count",
        197 => "Current_Pending_Sector",
        198 => "Offline_Uncorrectable",
        199 => "UDMA_CRC_Error_Count",
        200 => "Multi_Zone_Error_Rate",
        _ => return None,
    })
}

/// Text of an ATA string: the words are stored with the two bytes of each word swapped, padded with spaces.
fn ata_string(words: &[u8]) -> String {
    let swapped: Vec<u8> = words.chunks(2).flat_map(|w| [w.get(1).copied().unwrap_or(b' '), w[0]]).collect();
    String::from_utf8_lossy(&swapped).trim().to_string()
}

/// Parses the 512-byte answer of `IDENTIFY DEVICE`.
pub fn parse_identify(sector: &[u8]) -> Result<Identity, Error> {
    if sector.len() < 512 {
        return Err(Error::Device(format!("IDENTIFY DEVICE answer has {} bytes, not 512", sector.len())));
    }
    let word = |n: usize| u16::from_le_bytes([sector[2 * n], sector[2 * n + 1]]);
    Ok(Identity {
        serial: ata_string(&sector[20..40]),
        firmware: ata_string(&sector[46..54]),
        model: ata_string(&sector[54..94]),
        smart_supported: word(82) & 1 != 0,
        smart_enabled: word(85) & 1 != 0,
    })
}

/// Parses the attribute table and the self test status of a `SMART READ DATA` sector.
///
/// The thresholds are not in this sector; [`parse_thresholds`] adds them.
pub fn parse_smart_data(sector: &[u8]) -> Result<(Vec<Attribute>, SelfTest), Error> {
    if sector.len() < 512 {
        return Err(Error::Device(format!("SMART READ DATA answer has {} bytes, not 512", sector.len())));
    }
    let mut attributes = Vec::new();
    for slot in 0..ATTRIBUTE_SLOTS {
        let e = &sector[ATTRIBUTE_OFFSET + slot * 12..ATTRIBUTE_OFFSET + (slot + 1) * 12];
        if e[0] == 0 {
            continue;
        }
        let mut raw = [0u8; 8];
        raw[..6].copy_from_slice(&e[5..11]);
        attributes.push(Attribute {
            id: e[0],
            flags: u16::from_le_bytes([e[1], e[2]]),
            value: e[3],
            worst: e[4],
            raw: u64::from_le_bytes(raw),
            threshold: None,
        });
    }
    let byte = sector[SELF_TEST_STATUS];
    Ok((attributes, SelfTest { status: byte >> 4, remaining_percent: (byte & 0x0F) * 10 }))
}

/// Adds the thresholds of a `SMART READ THRESHOLDS` sector to `attributes`.
pub fn parse_thresholds(sector: &[u8], attributes: &mut [Attribute]) -> Result<(), Error> {
    if sector.len() < 512 {
        return Err(Error::Device(format!("SMART READ THRESHOLDS answer has {} bytes, not 512", sector.len())));
    }
    for slot in 0..ATTRIBUTE_SLOTS {
        let e = &sector[ATTRIBUTE_OFFSET + slot * 12..ATTRIBUTE_OFFSET + (slot + 1) * 12];
        if e[0] == 0 {
            continue;
        }
        if let Some(a) = attributes.iter_mut().find(|a| a.id == e[0]) {
            a.threshold = Some(e[1]);
        }
    }
    Ok(())
}

/// The temperature in °C from attribute 194 (or 190): the low byte of the raw value.
pub fn temperature(attributes: &[Attribute]) -> Option<i32> {
    let a = attributes.iter().find(|a| a.id == 194).or_else(|| attributes.iter().find(|a| a.id == 190))?;
    Some((a.raw & 0xFF) as i32)
}

/// The power-on hours from attribute 9 (the raw value counts hours for the common drives).
pub fn power_on_hours(attributes: &[Attribute]) -> Option<u64> {
    attributes.iter().find(|a| a.id == 9).map(|a| a.raw & 0xFFFF_FFFF)
}

/// The verdict from the registers `SMART RETURN STATUS` put into LBA mid and high.
pub fn health_from_registers(lba_mid: u8, lba_high: u8) -> Health {
    match (lba_mid, lba_high) {
        (0x4F, 0xC2) => Health::Passed,
        (0xF4, 0x2C) => Health::Failed,
        _ => Health::Unknown,
    }
}

/// `SMART RETURN STATUS` and the registers in the sense data: the *ATA Status Return* descriptor (`0x09`) of descriptor format sense (`0x72`).
pub fn health_from_sense(sense: &[u8]) -> Health {
    // response code 0x72/0x73, additional sense length, descriptors from byte 8
    if sense.len() >= 20 && matches!(sense[0] & 0x7F, 0x72 | 0x73) && sense[8] == 0x09 {
        return health_from_registers(sense[8 + 9], sense[8 + 11]);
    }
    Health::Unknown
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::sg;
    use std::fs::{File, OpenOptions};
    use std::path::Path;

    /// ATA PASS-THROUGH(16): protocol `proto`, the ATA registers, optional data-in buffer.
    fn pass_through(
        file: &File,
        data: Option<&mut [u8]>,
        feature: u8,
        count: u8,
        lba_mid: u8,
        lba_high: u8,
        command: u8,
    ) -> Result<sg::Done, Error> {
        let (protocol, flags) = if data.is_some() {
            // PIO data-in; T_DIR = from device, BYT_BLOK = blocks, T_LENGTH = sector count field
            (4u8, 0x0Eu8)
        } else {
            // non-data; CK_COND: return the registers in the sense data
            (3u8, 0x20u8)
        };
        let mut cdb = [0u8; 16];
        cdb[0] = 0x85;
        cdb[1] = protocol << 1;
        cdb[2] = flags;
        cdb[4] = feature;
        cdb[6] = count;
        cdb[10] = lba_mid;
        cdb[12] = lba_high;
        cdb[14] = command;
        let done = sg::request(file, &mut cdb, data)?;
        if done.host_status != 0 || (done.driver_status & 0x07 != 0 && done.sense.first().map(|b| b & 0x7F) != Some(0x72)) {
            return Err(Error::Device(format!(
                "the transport refused the ATA command (host status {:#x}, driver status {:#x}); a bridge without SAT?",
                done.host_status, done.driver_status
            )));
        }
        Ok(done)
    }

    /// Reads one data sector with the SMART command `feature` (0xD0 data, 0xD1 thresholds).
    fn smart_sector(file: &File, feature: u8) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; 512];
        pass_through(file, Some(&mut buf), feature, 1, 0x4F, 0xC2, 0xB0)?;
        Ok(buf)
    }

    /// Reads identity, attributes, thresholds, self test and health of the ATA drive at `path`.
    pub fn read(path: &Path) -> Result<Report, Error> {
        let file = OpenOptions::new().read(true).open(path)?;
        let mut sector = vec![0u8; 512];
        pass_through(&file, Some(&mut sector), 0, 1, 0, 0, 0xEC)?;
        let identity = parse_identify(&sector)?;
        if !identity.smart_supported {
            return Err(Error::Device("the drive does not support SMART".into()));
        }
        let (mut attributes, self_test) = parse_smart_data(&smart_sector(&file, 0xD0)?)?;
        // the thresholds are optional (obsolete since ACS-4, still answered by most drives)
        if let Ok(t) = smart_sector(&file, 0xD1) {
            parse_thresholds(&t, &mut attributes)?;
        }
        let health = match pass_through(&file, None, 0xDA, 0, 0x4F, 0xC2, 0xB0) {
            Ok(done) => health_from_sense(&done.sense),
            Err(_) => Health::Unknown,
        };
        Ok(Report {
            protocol: "ata",
            model: identity.model,
            serial: identity.serial,
            firmware: identity.firmware,
            health,
            temperature: temperature(&attributes),
            power_on_hours: power_on_hours(&attributes),
            attributes,
            self_test: Some(self_test),
            nvme: None,
        })
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::read;

#[cfg(test)]
mod tests {
    use super::*;

    /// An `IDENTIFY DEVICE` sector with the strings of a WD Red.
    fn identify_sector() -> Vec<u8> {
        let mut s = vec![0u8; 512];
        let put = |s: &mut Vec<u8>, from: usize, len: usize, text: &str| {
            let mut padded = text.as_bytes().to_vec();
            padded.resize(len, b' ');
            for (i, pair) in padded.chunks(2).enumerate() {
                s[from + 2 * i] = pair[1];
                s[from + 2 * i + 1] = pair[0];
            }
        };
        put(&mut s, 20, 20, "WD-WCC4M0000000");
        put(&mut s, 46, 8, "82.00A82");
        put(&mut s, 54, 40, "WDC WD20EFRX-68EUZN0");
        s[2 * 82] = 1;
        s[2 * 85] = 1;
        s
    }

    /// A SMART data sector with the attributes of a disk with 232 pending sectors.
    fn smart_sector() -> Vec<u8> {
        let mut s = vec![0u8; 512];
        let mut put = |slot: usize, id: u8, flags: u16, value: u8, worst: u8, raw: u64| {
            let e = &mut s[2 + slot * 12..2 + (slot + 1) * 12];
            e[0] = id;
            e[1..3].copy_from_slice(&flags.to_le_bytes());
            e[3] = value;
            e[4] = worst;
            e[5..11].copy_from_slice(&raw.to_le_bytes()[..6]);
        };
        put(0, 5, 0x33, 200, 200, 0);
        put(1, 9, 0x32, 25, 25, 54844);
        put(2, 194, 0x22, 111, 100, 36);
        put(3, 197, 0x32, 200, 200, 232);
        put(4, 199, 0x32, 200, 200, 1);
        s[363] = 0x00;
        s
    }

    #[test]
    fn identify_strings_are_unswapped() {
        let id = parse_identify(&identify_sector()).unwrap();
        assert_eq!(id.model, "WDC WD20EFRX-68EUZN0");
        assert_eq!(id.serial, "WD-WCC4M0000000");
        assert_eq!(id.firmware, "82.00A82");
        assert!(id.smart_supported && id.smart_enabled);
        assert!(parse_identify(&[0; 100]).is_err());
    }

    #[test]
    fn the_attribute_table_is_read() {
        let (attrs, test) = parse_smart_data(&smart_sector()).unwrap();
        assert_eq!(attrs.len(), 5);
        let pending = attrs.iter().find(|a| a.id == 197).unwrap();
        assert_eq!((pending.value, pending.worst, pending.raw, pending.is_prefail()), (200, 200, 232, false));
        assert_eq!(attrs[0].name(), Some("Reallocated_Sector_Ct"));
        assert!(attrs[0].is_prefail());
        assert_eq!(temperature(&attrs), Some(36));
        assert_eq!(power_on_hours(&attrs), Some(54844));
        assert!(test.passed() && test.text() == "completed without error");
    }

    #[test]
    fn thresholds_are_matched_by_id() {
        let (mut attrs, _) = parse_smart_data(&smart_sector()).unwrap();
        let mut t = vec![0u8; 512];
        t[2] = 5;
        t[3] = 140;
        t[2 + 12] = 197;
        t[3 + 12] = 0;
        parse_thresholds(&t, &mut attrs).unwrap();
        assert_eq!(attrs.iter().find(|a| a.id == 5).unwrap().threshold, Some(140));
        assert!(!attrs.iter().find(|a| a.id == 5).unwrap().failing());
        let mut low = attrs[0].clone();
        low.value = 140;
        assert!(low.failing(), "at the threshold means failing");
        assert_eq!(attrs.iter().find(|a| a.id == 197).unwrap().threshold, Some(0));
    }

    #[test]
    fn a_running_or_failed_self_test_is_decoded() {
        let mut s = smart_sector();
        s[363] = 0x79; // read failure, 90 % left of nothing: status 7
        let (_, t) = parse_smart_data(&s).unwrap();
        assert_eq!((t.status, t.passed()), (7, false));
        s[363] = 0xF3; // in progress, 30 % to do
        let (_, t) = parse_smart_data(&s).unwrap();
        assert_eq!((t.status, t.remaining_percent), (15, 30));
    }

    #[test]
    fn the_status_registers_give_the_verdict() {
        assert_eq!(health_from_registers(0x4F, 0xC2), Health::Passed);
        assert_eq!(health_from_registers(0xF4, 0x2C), Health::Failed);
        assert_eq!(health_from_registers(0, 0), Health::Unknown);
        // descriptor format sense with an ATA Status Return descriptor
        let mut sense = vec![0u8; 22];
        sense[0] = 0x72;
        sense[8] = 0x09;
        sense[9] = 0x0C;
        sense[17] = 0xF4;
        sense[19] = 0x2C;
        assert_eq!(health_from_sense(&sense), Health::Failed);
        assert_eq!(health_from_sense(&[0x70; 18]), Health::Unknown);
    }
}
