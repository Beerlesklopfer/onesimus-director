// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `SG_IO` (`<scsi/sg.h>`): the Linux SCSI generic request used for ATA PASS-THROUGH.

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// `SG_IO`.
const SG_IO: u64 = 0x2285;
/// `SG_DXFER_NONE`.
pub const DXFER_NONE: i32 = -1;
/// `SG_DXFER_FROM_DEV`.
pub const DXFER_FROM_DEV: i32 = -3;

/// `struct sg_io_hdr`.
#[repr(C)]
struct SgIoHdr {
    interface_id: libc::c_int,
    dxfer_direction: libc::c_int,
    cmd_len: u8,
    mx_sb_len: u8,
    iovec_count: u16,
    dxfer_len: libc::c_uint,
    dxferp: *mut libc::c_void,
    cmdp: *mut u8,
    sbp: *mut u8,
    timeout: libc::c_uint,
    flags: libc::c_uint,
    pack_id: libc::c_int,
    usr_ptr: *mut libc::c_void,
    status: u8,
    masked_status: u8,
    msg_status: u8,
    sb_len_wr: u8,
    host_status: u16,
    driver_status: u16,
    resid: libc::c_int,
    duration: libc::c_uint,
    info: libc::c_uint,
}

/// What a request returned: the SCSI status and the sense bytes.
pub struct Done {
    /// SCSI status (0 good, 2 check condition).
    pub status: u8,
    /// The host (transport) status; non-zero is a transport error.
    pub host_status: u16,
    /// The driver status.
    pub driver_status: u16,
    /// The sense data the device returned.
    pub sense: Vec<u8>,
}

/// Sends the SCSI command `cdb` with `data` as the data-in buffer (or none).
pub fn request(file: &File, cdb: &mut [u8], data: Option<&mut [u8]>) -> io::Result<Done> {
    let mut sense = [0u8; 64];
    let (direction, ptr, len) = match data {
        Some(d) => (DXFER_FROM_DEV, d.as_mut_ptr().cast(), d.len()),
        None => (DXFER_NONE, std::ptr::null_mut(), 0),
    };
    let mut hdr = SgIoHdr {
        interface_id: i32::from(b'S'),
        dxfer_direction: direction,
        cmd_len: cdb.len() as u8,
        mx_sb_len: sense.len() as u8,
        iovec_count: 0,
        dxfer_len: len as libc::c_uint,
        dxferp: ptr,
        cmdp: cdb.as_mut_ptr(),
        sbp: sense.as_mut_ptr(),
        timeout: 20_000,
        flags: 0,
        pack_id: 0,
        usr_ptr: std::ptr::null_mut(),
        status: 0,
        masked_status: 0,
        msg_status: 0,
        sb_len_wr: 0,
        host_status: 0,
        driver_status: 0,
        resid: 0,
        duration: 0,
        info: 0,
    };
    // SAFETY: `hdr` points at buffers that live until the call returns; the ioctl only reads and writes through them.
    let rc = unsafe { libc::ioctl(file.as_raw_fd(), SG_IO as _, &mut hdr as *mut SgIoHdr) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    let n = usize::from(hdr.sb_len_wr).min(sense.len());
    Ok(Done { status: hdr.status, host_status: hdr.host_status, driver_status: hdr.driver_status, sense: sense[..n].to_vec() })
}
