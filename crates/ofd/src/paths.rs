// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Default paths and names of ofd per platform, each in one constant.
//!
//! Unix: the Bareos layout (`/etc/bareos/bareos-fd.d`, `/var/lib/bareos`)
//! and the Onesimus log file, unchanged.
//!
//! Windows: below `C:\ProgramData\Onesimus`, as the MSI installer of the
//! GUI creates it (`packaging/windows/config.wxi` in the GUI repository:
//! `DataDirName`, `OfdConfigDirName`, `LogDirName`, `OfdLogFileName`,
//! `OfdServiceName`). The installer passes `-c` and `--log-file`
//! explicitly; the defaults here are for a start without them. These
//! values are a proposal the maintainer has not confirmed yet (open point
//! in docs/plans/PORTIERUNG-MACOS-WINDOWS.md). Bareos itself uses
//! `C:\ProgramData\Bareos\bareos-fd.d` and the service name `Bareos-fd`
//! (`win32/filed/who.h`).

/// Default configuration directory (`-c`).
#[cfg(unix)]
pub const CONFIG_DIR: &str = "/etc/bareos/bareos-fd.d";
/// Default configuration directory (`-c`).
#[cfg(windows)]
pub const CONFIG_DIR: &str = r"C:\ProgramData\Onesimus\bareos-fd.d";

/// Default log file (`--log-file`).
#[cfg(unix)]
pub const LOG_FILE: &str = "/var/log/onesimus/ofd.log";
/// Default log file (`--log-file`).
#[cfg(windows)]
pub const LOG_FILE: &str = r"C:\ProgramData\Onesimus\log\ofd.log";

/// Default `Working Directory` (gRPC certificate and key live there).
#[cfg(unix)]
pub const WORKING_DIRECTORY: &str = "/var/lib/bareos";
/// Default `Working Directory` (gRPC certificate and key live there);
/// created at start for SYSTEM and Administrators only ([`ensure_private_dir`]).
#[cfg(windows)]
pub const WORKING_DIRECTORY: &str = r"C:\ProgramData\Onesimus\working";

/// Name of the Windows service (`OfdServiceName` of the MSI).
#[cfg(windows)]
pub const SERVICE_NAME: &str = "ofd";

/// The access rule of the MSI for the configuration and log directory:
/// owner Administrators, full access for SYSTEM and Administrators only,
/// inherited by files and subdirectories, no inheritance from above.
#[cfg(windows)]
const PRIVATE_DIR_SDDL: &str = "O:BAG:SYD:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";

/// Creates `path` (and missing parents) unless it exists; the directory
/// itself gets [`PRIVATE_DIR_SDDL`], parents the default ACL. An existing
/// directory is left as it is (its ACL is the administrator's choice).
#[cfg(windows)]
pub fn ensure_private_dir(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let sddl: Vec<u16> = PRIVATE_DIR_SDDL.encode_utf16().chain(Some(0)).collect();
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut sd = std::ptr::null_mut();
    // SAFETY: NUL-terminated SDDL; on success `sd` is freed with LocalFree below.
    if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    // SAFETY: NUL-terminated path, valid security attributes.
    let ok = unsafe { CreateDirectoryW(wide.as_ptr(), &sa) } != 0;
    let err = std::io::Error::last_os_error();
    // SAFETY: `sd` was allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(sd as _) };
    if ok || path.is_dir() { Ok(()) } else { Err(err) }
}
