//! Windows: `\\.\PhysicalDriveN` with `DeviceIoControl`.
//!
//! * **ATA/SATA**: `IOCTL_ATA_PASS_THROUGH` with an `ATA_PASS_THROUGH_EX`
//!   header and the data buffer behind it; the ATA task file registers are
//!   the same as in the Linux SAT path, so the parsers of [`crate::ata`]
//!   apply unchanged. Needs administrator rights.
//! * **NVMe**: `IOCTL_STORAGE_QUERY_PROPERTY` with
//!   `StorageDeviceProtocolSpecificProperty` and `ProtocolTypeNvme`:
//!   *Identify Controller* and the SMART / Health log page 0x02.
//!
//! The drive is chosen by its path: `\\.\PhysicalDrive0` or just `PhysicalDrive0`.

use crate::{ata, nvme, Error, Health, Report};
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::IO::DeviceIoControl;

/// `IOCTL_ATA_PASS_THROUGH`.
const IOCTL_ATA_PASS_THROUGH: u32 = 0x0004_D02C;
/// `IOCTL_STORAGE_QUERY_PROPERTY`.
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
/// `ATA_FLAGS_DRDY_REQUIRED`.
const ATA_FLAGS_DRDY_REQUIRED: u16 = 0x01;
/// `ATA_FLAGS_DATA_IN`.
const ATA_FLAGS_DATA_IN: u16 = 0x02;

/// `ATA_PASS_THROUGH_EX`.
#[repr(C)]
#[derive(Default)]
struct AtaPassThroughEx {
    length: u16,
    ata_flags: u16,
    path_id: u8,
    target_id: u8,
    lun: u8,
    reserved_as_uchar: u8,
    data_transfer_length: u32,
    time_out_value: u32,
    reserved_as_ulong: u32,
    data_buffer_offset: usize,
    previous_task_file: [u8; 8],
    current_task_file: [u8; 8],
}

/// An open drive.
struct Drive(HANDLE);

impl Drop for Drive {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// Opens `\\.\PhysicalDriveN` for the control codes.
fn open(path: &std::path::Path) -> Result<Drive, Error> {
    let text = path.to_string_lossy();
    let full = if text.starts_with(r"\\.\") { text.to_string() } else { format!(r"\\.\{text}") };
    let wide: Vec<u16> = std::ffi::OsStr::new(&full).encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is NUL terminated and outlives the call.
    let handle = unsafe {
        CreateFileW(wide.as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    Ok(Drive(handle))
}

/// `DeviceIoControl` with one buffer used for input and output.
fn control(drive: &Drive, code: u32, buffer: &mut [u8]) -> Result<u32, Error> {
    let mut returned = 0u32;
    // SAFETY: `buffer` is valid for its length for the whole call; no overlapped I/O.
    let ok = unsafe {
        DeviceIoControl(drive.0, code, buffer.as_ptr().cast(), buffer.len() as u32, buffer.as_mut_ptr().cast(), buffer.len() as u32, &mut returned, std::ptr::null_mut())
    };
    if ok == 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    Ok(returned)
}

/// One ATA command through `IOCTL_ATA_PASS_THROUGH`: the 512 data bytes (or none) and the returned registers.
fn ata_command(drive: &Drive, feature: u8, count: u8, lba_mid: u8, lba_high: u8, command: u8, data_in: bool) -> Result<(Vec<u8>, [u8; 8]), Error> {
    let header = std::mem::size_of::<AtaPassThroughEx>();
    let data_len = if data_in { 512 } else { 0 };
    let mut buffer = vec![0u8; header + data_len];
    let mut ex = AtaPassThroughEx {
        length: header as u16,
        ata_flags: ATA_FLAGS_DRDY_REQUIRED | if data_in { ATA_FLAGS_DATA_IN } else { 0 },
        data_transfer_length: data_len as u32,
        time_out_value: 20,
        data_buffer_offset: header,
        ..Default::default()
    };
    // task file: features, sector count, LBA low, LBA mid, LBA high, device, command
    ex.current_task_file = [feature, count, 0, lba_mid, lba_high, 0xA0, command, 0];
    // SAFETY: `ex` is a plain repr(C) struct copied byte for byte into the buffer that is large enough.
    unsafe { std::ptr::copy_nonoverlapping(&ex as *const AtaPassThroughEx as *const u8, buffer.as_mut_ptr(), header) };
    control(drive, IOCTL_ATA_PASS_THROUGH, &mut buffer)?;
    let mut regs = [0u8; 8];
    regs.copy_from_slice(&buffer[header - 8..header]);
    Ok((buffer[header..].to_vec(), regs))
}

/// The ATA drive at `drive`.
fn read_ata(drive: &Drive) -> Result<Report, Error> {
    let identity = ata::parse_identify(&ata_command(drive, 0, 1, 0, 0, 0xEC, true)?.0)?;
    if !identity.smart_supported {
        return Err(Error::Device("the drive does not support SMART".into()));
    }
    let (mut attributes, self_test) = ata::parse_smart_data(&ata_command(drive, 0xD0, 1, 0x4F, 0xC2, 0xB0, true)?.0)?;
    if let Ok((t, _)) = ata_command(drive, 0xD1, 1, 0x4F, 0xC2, 0xB0, true) {
        ata::parse_thresholds(&t, &mut attributes)?;
    }
    let health = match ata_command(drive, 0xDA, 0, 0x4F, 0xC2, 0xB0, false) {
        Ok((_, regs)) => ata::health_from_registers(regs[3], regs[4]),
        Err(_) => Health::Unknown,
    };
    Ok(Report {
        protocol: "ata",
        model: identity.model,
        serial: identity.serial,
        firmware: identity.firmware,
        health,
        temperature: ata::temperature(&attributes),
        power_on_hours: ata::power_on_hours(&attributes),
        attributes,
        self_test: Some(self_test),
        nvme: None,
    })
}

/// `STORAGE_PROPERTY_QUERY` (8 bytes) + `STORAGE_PROTOCOL_SPECIFIC_DATA` (40 bytes) + room for the answer.
fn nvme_query(drive: &Drive, data_type: u32, request_value: u32, len: usize) -> Result<Vec<u8>, Error> {
    const QUERY: usize = 8;
    const SPECIFIC: usize = 40;
    let mut buffer = vec![0u8; QUERY + SPECIFIC + len];
    let put = |buf: &mut Vec<u8>, at: usize, v: u32| buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut buffer, 0, 49); // StorageDeviceProtocolSpecificProperty
    put(&mut buffer, 4, 0); // PropertyStandardQuery
    put(&mut buffer, QUERY, 3); // ProtocolTypeNvme
    put(&mut buffer, QUERY + 4, data_type);
    put(&mut buffer, QUERY + 8, request_value);
    put(&mut buffer, QUERY + 12, 0); // sub value (namespace 0)
    put(&mut buffer, QUERY + 16, SPECIFIC as u32); // ProtocolDataOffset
    put(&mut buffer, QUERY + 20, len as u32); // ProtocolDataLength
    control(drive, IOCTL_STORAGE_QUERY_PROPERTY, &mut buffer)?;
    // the answer: STORAGE_PROTOCOL_DATA_DESCRIPTOR (Version, Size) + the specific data + the data
    let offset = u32::from_le_bytes(buffer[8 + 16..8 + 20].try_into().unwrap()) as usize;
    let start = 8 + offset;
    buffer.get(start..start + len).map(<[u8]>::to_vec).ok_or_else(|| Error::Device("the NVMe answer is shorter than asked".into()))
}

/// The NVMe drive at `drive`.
fn read_nvme(drive: &Drive) -> Result<Report, Error> {
    let identify = nvme_query(drive, 1, 1, 4096)?; // NVMeDataTypeIdentify, CNS controller
    let log = nvme_query(drive, 2, 2, 512)?; // NVMeDataTypeLogPage, SMART / Health
    nvme::report(&identify, &log)
}

/// Reads the SMART data of `\\.\PhysicalDriveN`: NVMe when the drive answers the protocol query, else ATA.
pub fn read(path: &std::path::Path) -> Result<Report, Error> {
    let drive = open(path)?;
    match read_nvme(&drive) {
        Ok(report) => Ok(report),
        Err(_) => read_ata(&drive),
    }
}
