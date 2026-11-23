// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Reads S.M.A.R.T. health data of disks on Linux, in Rust, without running
//! `smartctl`.
//!
//! * **ATA/SATA** (also SATA disks behind SAT-capable USB bridges): `SG_IO`
//!   with the SCSI/ATA Translation command ATA PASS-THROUGH(16): `IDENTIFY
//!   DEVICE`, `SMART READ DATA` (attribute table, self test status),
//!   `SMART READ THRESHOLDS`, `SMART RETURN STATUS` (the health verdict).
//! * **SCSI / SAS**: `INQUIRY` and the log pages Informational Exceptions,
//!   Temperature, Self-Test Results and the error counters, and the grown
//!   defect list (not verified on a SAS drive).
//! * **NVMe**: the admin command *Get Log Page* with the SMART / Health
//!   information log (`NVME_IOCTL_ADMIN_CMD`).
//!
//! Reading needs `CAP_SYS_RAWIO` (ATA) or `CAP_SYS_ADMIN` (NVMe) or root, as
//! `smartctl` does. The calls only read; nothing is written to the drive and
//! no self test is started.
//!
//! ```no_run
//! let report = smartread::read("/dev/sda")?;
//! println!("{}: {:?}", report.model, report.health);
//! # Ok::<(), smartread::Error>(())
//! ```
//!
//! The parsers ([`ata::parse_identify`], [`ata::parse_smart_data`],
//! [`nvme::parse_health_log`]) take the raw sectors and work on every
//! platform; the device access is Linux and Windows (macOS: see the README).

#![deny(missing_docs)]

pub mod ata;
pub mod nvme;
pub mod scsi;
#[cfg(target_os = "linux")]
mod sg;
#[cfg(windows)]
mod windows;

use std::fmt;

/// What went wrong.
#[derive(Debug)]
pub enum Error {
    /// The device could not be opened or the request failed (permission denied, no such device, …).
    Io(std::io::Error),
    /// The drive did not answer a SMART command as expected (no SMART, a bridge that does not translate).
    Device(String),
    /// Not Linux or Windows (macOS is not implemented, see the README), or a drive type this crate cannot read.
    Unsupported(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Device(m) | Error::Unsupported(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

/// The overall verdict the drive gives itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// SMART says the drive is fine.
    Passed,
    /// SMART says the drive is failing (ATA: threshold exceeded; NVMe: a critical warning bit).
    Failed,
    /// The status could not be read (bridge without status translation).
    Unknown,
}

/// One ATA SMART attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Attribute ID (5 Reallocated sectors, 9 power-on hours, 194 temperature, 197 pending sectors, …).
    pub id: u8,
    /// Status flags (bit 0: pre-fail, bit 1: updated online).
    pub flags: u16,
    /// Normalised current value (1..253).
    pub value: u8,
    /// Worst value seen.
    pub worst: u8,
    /// The six raw bytes as a number (little endian).
    pub raw: u64,
    /// The failure threshold from `SMART READ THRESHOLDS` (0: none).
    pub threshold: Option<u8>,
}

impl Attribute {
    /// A pre-fail attribute (flag bit 0): reaching the threshold means failure.
    pub fn is_prefail(&self) -> bool {
        self.flags & 1 != 0
    }

    /// The value is at or below its threshold (failing now when pre-fail).
    pub fn failing(&self) -> bool {
        self.threshold.is_some_and(|t| t > 0 && self.value <= t)
    }

    /// The conventional name of the ID, if it is a well known one.
    pub fn name(&self) -> Option<&'static str> {
        ata::attribute_name(self.id)
    }
}

/// The result of the last ATA self test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfTest {
    /// The status nibble: 0 completed without error, 1 aborted by the host, 2 interrupted, 3 fatal error, 4 unknown failure,
    /// 5 electrical failure, 6 servo failure, 7 read failure, 8 handling damage, 15 in progress.
    pub status: u8,
    /// Percent of the running test still to do (status 15).
    pub remaining_percent: u8,
}

impl SelfTest {
    /// The test finished without an error (or none was run).
    pub fn passed(&self) -> bool {
        self.status == 0
    }

    /// The status in words.
    pub fn text(&self) -> &'static str {
        match self.status {
            0 => "completed without error",
            1 => "aborted by the host",
            2 => "interrupted by a reset",
            3 => "fatal error or unknown test error",
            4 => "completed with an unknown failure",
            5 => "completed with an electrical failure",
            6 => "completed with a servo or seek failure",
            7 => "completed with a read failure",
            8 => "completed with a handling damage",
            15 => "in progress",
            _ => "unknown status",
        }
    }
}

/// What a drive reports about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// `ata` or `nvme`.
    pub protocol: &'static str,
    /// The model string.
    pub model: String,
    /// The serial number.
    pub serial: String,
    /// The firmware revision.
    pub firmware: String,
    /// The verdict of the drive.
    pub health: Health,
    /// Temperature in °C.
    pub temperature: Option<i32>,
    /// Power-on hours.
    pub power_on_hours: Option<u64>,
    /// ATA: the attribute table (empty for NVMe).
    pub attributes: Vec<Attribute>,
    /// ATA: the last self test.
    pub self_test: Option<SelfTest>,
    /// NVMe: the SMART / Health log (`None` for ATA).
    pub nvme: Option<nvme::HealthLog>,
    /// SCSI / SAS: what the log pages say (`None` for ATA and NVMe).
    pub scsi: Option<scsi::Summary>,
}

impl Report {
    /// The attribute `id` (ATA).
    pub fn attribute(&self, id: u8) -> Option<&Attribute> {
        self.attributes.iter().find(|a| a.id == id)
    }
}

/// Reads the SMART data of the drive at `path`: on Linux `/dev/sda`, `/dev/nvme0`, `/dev/nvme0n1`;
/// on Windows `\\.\PhysicalDrive0`.
#[cfg(target_os = "linux")]
pub fn read(path: impl AsRef<std::path::Path>) -> Result<Report, Error> {
    let path = path.as_ref();
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with("nvme") {
        return nvme::read(path);
    }
    // SATA disks answer INQUIRY with the vendor "ATA" (SAT): they take the ATA commands; every other SCSI device the log pages
    let file = std::fs::OpenOptions::new().read(true).open(path)?;
    let sata = scsi::inquiry(&file, None).ok().and_then(|d| scsi::parse_inquiry(&d).ok()).is_some_and(|(vendor, _, _)| vendor == "ATA");
    if sata || scsi::inquiry(&file, None).is_err() {
        ata::read(path)
    } else {
        scsi::read(&file)
    }
}

/// Reads the drive at `path` through the SCSI log pages only (a SAS drive, or a SATA disk as a test of this path).
#[cfg(target_os = "linux")]
pub fn read_scsi(path: impl AsRef<std::path::Path>) -> Result<Report, Error> {
    let file = std::fs::OpenOptions::new().read(true).open(path.as_ref())?;
    scsi::read(&file)
}

/// Reads the SMART data of the drive at `path` (`\\.\PhysicalDrive0`); needs administrator rights.
#[cfg(windows)]
pub fn read(path: impl AsRef<std::path::Path>) -> Result<Report, Error> {
    windows::read(path.as_ref())
}

/// Reads the SMART data of the drive at `path` (not implemented on this platform).
#[cfg(not(any(target_os = "linux", windows)))]
pub fn read(_path: impl AsRef<std::path::Path>) -> Result<Report, Error> {
    Err(Error::Unsupported("smartread reads drives on Linux and Windows; macOS is not implemented (see the README of the crate)".into()))
}
