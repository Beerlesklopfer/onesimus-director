// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! NVMe drives: the *SMART / Health Information* log page.
//!
//! The admin command *Get Log Page* (opcode `0x02`, log identifier `0x02`,
//! namespace `0xFFFFFFFF`, 512 bytes) is sent with the ioctl
//! `NVME_IOCTL_ADMIN_CMD` on the controller device (`/dev/nvme0`; the
//! namespace device `/dev/nvme0n1` takes the admin ioctl as well). It needs
//! `CAP_SYS_ADMIN` or root.
//!
//! The identity (model, serial, firmware) comes from *Identify Controller*
//! (opcode `0x06`, CNS 1, 4096 bytes): model at bytes 24–63, serial at 4–23,
//! firmware at 64–71.
//!
//! [`parse_health_log`] and [`parse_identify_controller`] take the raw
//! answers and need no hardware.

use crate::{Error, Health, Report};

/// The SMART / Health Information log of an NVMe drive (NVMe base spec, figure "SMART / Health Information Log Page").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthLog {
    /// Critical warning bits: 0 spare below threshold, 1 temperature, 2 reliability degraded, 3 read-only, 4 volatile memory backup failed, 5 persistent memory region read-only.
    pub critical_warning: u8,
    /// Composite temperature in °C (the log has Kelvin).
    pub temperature: i32,
    /// Available spare in percent.
    pub available_spare: u8,
    /// Available spare threshold in percent.
    pub spare_threshold: u8,
    /// Percentage used of the drive's endurance (may exceed 100).
    pub percentage_used: u8,
    /// Data units read (1 unit = 1000 × 512 bytes).
    pub data_units_read: u128,
    /// Data units written.
    pub data_units_written: u128,
    /// Power cycles.
    pub power_cycles: u128,
    /// Power-on hours.
    pub power_on_hours: u128,
    /// Unsafe shutdowns.
    pub unsafe_shutdowns: u128,
    /// Media and data integrity errors.
    pub media_errors: u128,
    /// Entries in the error information log.
    pub error_log_entries: u128,
}

impl HealthLog {
    /// A critical warning is set, or the spare is below its threshold.
    pub fn failing(&self) -> bool {
        self.critical_warning & 0x1F != 0 || self.available_spare < self.spare_threshold
    }
}

/// Reads a 16-byte little endian counter.
fn counter(log: &[u8], at: usize) -> u128 {
    let mut b = [0u8; 16];
    b.copy_from_slice(&log[at..at + 16]);
    u128::from_le_bytes(b)
}

/// Parses the 512-byte SMART / Health Information log.
pub fn parse_health_log(log: &[u8]) -> Result<HealthLog, Error> {
    if log.len() < 512 {
        return Err(Error::Device(format!("the SMART/Health log has {} bytes, not 512", log.len())));
    }
    let kelvin = i32::from(u16::from_le_bytes([log[1], log[2]]));
    Ok(HealthLog {
        critical_warning: log[0],
        temperature: kelvin - 273,
        available_spare: log[3],
        spare_threshold: log[4],
        percentage_used: log[5],
        data_units_read: counter(log, 32),
        data_units_written: counter(log, 48),
        power_cycles: counter(log, 112),
        power_on_hours: counter(log, 128),
        unsafe_shutdowns: counter(log, 144),
        media_errors: counter(log, 160),
        error_log_entries: counter(log, 176),
    })
}

/// Model, serial number and firmware revision from the 4096-byte *Identify Controller* answer.
pub fn parse_identify_controller(data: &[u8]) -> Result<(String, String, String), Error> {
    if data.len() < 72 {
        return Err(Error::Device(format!("Identify Controller has {} bytes", data.len())));
    }
    let text = |from: usize, to: usize| String::from_utf8_lossy(&data[from..to]).trim().to_string();
    Ok((text(24, 64), text(4, 24), text(64, 72)))
}

/// A [`Report`] from the two answers.
pub fn report(identify: &[u8], log: &[u8]) -> Result<Report, Error> {
    let (model, serial, firmware) = parse_identify_controller(identify)?;
    let health_log = parse_health_log(log)?;
    Ok(Report {
        protocol: "nvme",
        model,
        serial,
        firmware,
        health: if health_log.failing() { Health::Failed } else { Health::Passed },
        temperature: Some(health_log.temperature),
        power_on_hours: Some(health_log.power_on_hours.min(u128::from(u64::MAX)) as u64),
        attributes: Vec::new(),
        self_test: None,
        nvme: Some(health_log),
        scsi: None,
    })
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;
    use std::path::Path;

    /// `struct nvme_admin_cmd` (`<linux/nvme_ioctl.h>`).
    #[repr(C)]
    #[derive(Default)]
    struct AdminCmd {
        opcode: u8,
        flags: u8,
        rsvd1: u16,
        nsid: u32,
        cdw2: u32,
        cdw3: u32,
        metadata: u64,
        addr: u64,
        metadata_len: u32,
        data_len: u32,
        cdw10: u32,
        cdw11: u32,
        cdw12: u32,
        cdw13: u32,
        cdw14: u32,
        cdw15: u32,
        timeout_ms: u32,
        result: u32,
    }

    /// `NVME_IOCTL_ADMIN_CMD` = `_IOWR('N', 0x41, struct nvme_admin_cmd)`.
    pub(super) const NVME_IOCTL_ADMIN_CMD: u64 = (3 << 30) | ((std::mem::size_of::<AdminCmd>() as u64) << 16) | ((b'N' as u64) << 8) | 0x41;

    /// Sends an admin command with a data-in buffer of `len` bytes.
    fn admin(file: &std::fs::File, opcode: u8, nsid: u32, cdw10: u32, len: usize) -> Result<Vec<u8>, Error> {
        let mut buf = vec![0u8; len];
        let mut cmd = AdminCmd {
            opcode,
            nsid,
            addr: buf.as_mut_ptr() as usize as u64,
            data_len: len as u32,
            cdw10,
            timeout_ms: 20_000,
            ..Default::default()
        };
        // SAFETY: `cmd` and `buf` stay alive for the call; the kernel reads `cmd` and writes `buf` and `cmd.result`.
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), NVME_IOCTL_ADMIN_CMD as _, &mut cmd as *mut AdminCmd) };
        if rc < 0 {
            return Err(Error::Io(std::io::Error::last_os_error()));
        }
        if rc > 0 {
            return Err(Error::Device(format!("the NVMe command {opcode:#x} failed with status {rc:#x}")));
        }
        Ok(buf)
    }

    /// Reads the identity and the SMART / Health log of the NVMe drive at `path`.
    pub fn read(path: &Path) -> Result<Report, Error> {
        let file = OpenOptions::new().read(true).open(path)?;
        let identify = admin(&file, 0x06, 0, 1, 4096)?;
        // Get Log Page: log id 0x02, NUMD = 512 / 4 - 1 in bits 27:16 of CDW10
        let log = admin(&file, 0x02, 0xFFFF_FFFF, (127 << 16) | 0x02, 512)?;
        super::report(&identify, &log)
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::read;

#[cfg(test)]
mod tests {
    use super::*;

    /// A health log of a drive that is 36 °C warm, has run 1234 hours and logged 3 media errors.
    fn log() -> Vec<u8> {
        let mut l = vec![0u8; 512];
        l[1..3].copy_from_slice(&(36u16 + 273).to_le_bytes());
        l[3] = 100;
        l[4] = 10;
        l[5] = 7;
        l[128..132].copy_from_slice(&1234u32.to_le_bytes());
        l[160] = 3;
        l[176] = 9;
        l[112] = 55;
        l
    }

    #[test]
    fn the_health_log_is_decoded() {
        let h = parse_health_log(&log()).unwrap();
        assert_eq!((h.temperature, h.available_spare, h.spare_threshold, h.percentage_used), (36, 100, 10, 7));
        assert_eq!((h.power_on_hours, h.media_errors, h.error_log_entries, h.power_cycles), (1234, 3, 9, 55));
        assert!(!h.failing());
        assert!(parse_health_log(&[0; 10]).is_err());
    }

    #[test]
    fn a_critical_warning_or_low_spare_is_failing() {
        let mut l = log();
        l[0] = 0x04; // reliability degraded
        assert!(parse_health_log(&l).unwrap().failing());
        let mut l = log();
        l[3] = 5; // spare below the threshold of 10
        assert!(parse_health_log(&l).unwrap().failing());
        let mut l = log();
        l[0] = 0x80; // reserved bit: not a failure
        assert!(!parse_health_log(&l).unwrap().failing());
    }

    #[test]
    fn identify_controller_gives_the_strings() {
        let mut d = vec![b' '; 4096];
        d[4..4 + 8].copy_from_slice(b"S1234567");
        d[24..24 + 12].copy_from_slice(b"Samsung 980 ");
        d[64..64 + 8].copy_from_slice(b"1B2QGXA7");
        let r = report(&d, &log()).unwrap();
        assert_eq!((r.model.as_str(), r.serial.as_str(), r.firmware.as_str()), ("Samsung 980", "S1234567", "1B2QGXA7"));
        assert_eq!((r.protocol, r.health, r.temperature, r.power_on_hours), ("nvme", Health::Passed, Some(36), Some(1234)));
    }

    #[test]
    fn the_ioctl_number_matches_linux() {
        // _IOWR('N', 0x41, struct nvme_admin_cmd) with a 72-byte struct is 0xC0484E41
        #[cfg(target_os = "linux")]
        assert_eq!(super::linux::NVME_IOCTL_ADMIN_CMD, 0xC048_4E41);
    }
}
