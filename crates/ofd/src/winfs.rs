// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The file tree layer of ofd on Windows: `stat`, file system type, times.
//!
//! Zweck: Auf Unix liefert `lstat` die Werte, die `EncodeStat` in den
//! Katalog schreibt. Windows hat kein `lstat`; Bareos füllt die `Stat`
//! dort aus `WIN32_FILE_ATTRIBUTE_DATA` (`win32/compat/compat.cc`,
//! `stat2`). Dieses Modul tut dasselbe mit einem Handle ohne Zugriffsrechte
//! (`CreateFileW` mit `FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT`,
//! so folgt es keinem Symlink und öffnet auch Verzeichnisse und gesperrte
//! Dateien) und `GetFileInformationByHandle(Ex)`.
//!
//! Datenfluss: `find.rs` (`stat_of`) → [`stat`] → `Stat` → `encode_stat` →
//! Attributsatz im SD-Strom. Beim Wiederherstellen setzt [`set_times`] die
//! Zeiten aus der `Stat` des Katalogs.
//!
//! Abweichungen von Bareos (nicht gegen einen Bareos-Windows-FD geprüft):
//! `st_ctime` ist die Änderungszeit des MFT-Eintrags (`ChangeTime`), nicht
//! die Erstellungszeit; Besitzer und Gruppe sind 0 (`st_uid`, `st_gid`), wie
//! Bareos sie auf Win32 schreibt; Modus `0777`, schreibgeschützte Dateien
//! `0555`, Verzeichnisse immer `0777`.

use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bareos_format::lstat::Stat;
use windows_sys::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, GetVolumeInformationW,
    GetVolumePathNameW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT,
};

use crate::platform::OsStrExt;
use crate::winattr::{self, WinMeta};

/// `FILE_WRITE_ATTRIBUTES`: enough to set times and attributes.
const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
/// `FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE`.
const SHARE_ALL: u32 = 0x7;
/// 100-ns ticks between 1601-01-01 and 1970-01-01.
const EPOCH_DIFF_TICKS: i64 = 116_444_736_000_000_000;

/// The path as an `OsStr` (names on the wire are UTF-8 with `/`).
fn os(path: &[u8]) -> &std::ffi::OsStr {
    std::ffi::OsStr::from_bytes(path)
}

/// The Win32 error of an `io::Error` (0 when it has none).
fn code(e: std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(0)
}

/// FILETIME ticks to Unix seconds (rounded down).
pub fn ticks_to_unix(ticks: i64) -> i64 {
    (ticks - EPOCH_DIFF_TICKS).div_euclid(10_000_000)
}

/// Unix seconds to FILETIME ticks.
pub fn unix_to_ticks(secs: i64) -> i64 {
    secs * 10_000_000 + EPOCH_DIFF_TICKS
}

/// The `Stat` of `path` without following a symbolic link or junction
/// (`lstat`); the error is the Win32 error code.
#[allow(dead_code)]
pub fn stat(path: &[u8]) -> Result<Stat, i32> {
    stat_ex(path).map(|(st, _)| st)
}

/// [`stat`] and the Win32 data of the file (`attribsEx`).
pub fn stat_ex(path: &[u8]) -> Result<(Stat, WinMeta), i32> {
    let link = std::fs::symlink_metadata(os(path)).map_err(code)?.file_type().is_symlink();
    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(os(path))
        .map_err(code)?;
    let handle = file.as_raw_handle();
    // SAFETY: zeroed plain-data structs, a valid handle that lives through both calls.
    let (info, basic) = unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(handle, &mut info) == 0 {
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        let mut basic: FILE_BASIC_INFO = std::mem::zeroed();
        let ok = GetFileInformationByHandleEx(
            handle,
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        );
        if ok == 0 {
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        (info, basic)
    };
    let attributes = info.dwFileAttributes;
    // the tag tells a junction (mount point) from a symbolic link
    let mut reparse_tag = 0;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        // SAFETY: zeroed plain-data struct, a valid handle, the size of the struct.
        unsafe {
            let mut tag: FILE_ATTRIBUTE_TAG_INFO = std::mem::zeroed();
            let ok = GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                (&mut tag as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
                std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            );
            if ok != 0 {
                reparse_tag = tag.ReparseTag;
            }
        }
    }
    let size = (i64::from(info.nFileSizeHigh) << 32) | i64::from(info.nFileSizeLow);
    let mode = if link {
        crate::stmode::S_IFLNK | 0o777
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        crate::stmode::S_IFDIR | 0o777
    } else if attributes & FILE_ATTRIBUTE_READONLY != 0 {
        crate::stmode::S_IFREG | 0o555
    } else {
        crate::stmode::S_IFREG | 0o777
    };
    let size = if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 { 0 } else { size };
    let meta = WinMeta {
        attributes,
        creation: basic.CreationTime,
        access: basic.LastAccessTime,
        write: basic.LastWriteTime,
        size: size.max(0) as u64,
        reparse_tag,
    };
    let st = Stat {
        dev: u64::from(info.dwVolumeSerialNumber),
        ino: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        mode,
        nlink: u64::from(info.nNumberOfLinks.max(1)),
        uid: 0,
        gid: 0,
        rdev: 0,
        size,
        blksize: 4096,
        blocks: (size + 511) / 512,
        atime: ticks_to_unix(basic.LastAccessTime),
        mtime: ticks_to_unix(basic.LastWriteTime),
        ctime: ticks_to_unix(basic.ChangeTime),
        flags: 0,
    };
    Ok((st, meta))
}

/// `path` as a NUL-terminated wide string for a Win32 call: `/` becomes `\`
/// and a long absolute path gets the `\\?\` prefix (no 260-character limit).
fn wide(path: &[u8]) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt as _;
    let mut w: Vec<u16> = os(path).encode_wide().map(|c| if c == u16::from(b'/') { u16::from(b'\\') } else { c }).collect();
    let drive = w.len() >= 3 && w[1] == u16::from(b':') && w[2] == u16::from(b'\\');
    let unc = w.len() >= 2 && w[0] == u16::from(b'\\') && w[1] == u16::from(b'\\') && !w.starts_with(&[92, 92, 63, 92]);
    if w.len() >= 248 && drive {
        let mut v: Vec<u16> = r"\\?\".encode_utf16().collect();
        v.extend(w);
        w = v;
    } else if w.len() >= 248 && unc {
        let mut v: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        v.extend(&w[2..]);
        w = v;
    }
    w.push(0);
    w
}

/// SystemTime of FILETIME ticks.
fn system_time(ticks: i64) -> SystemTime {
    let since_unix = ticks - EPOCH_DIFF_TICKS;
    if since_unix >= 0 {
        UNIX_EPOCH + Duration::from_nanos(since_unix as u64 * 100)
    } else {
        UNIX_EPOCH - Duration::from_nanos(since_unix.unsigned_abs() * 100)
    }
}

/// Sets the Win32 attributes `meta` carries on a restored file or
/// directory, after its times: the exact times (with the creation time)
/// and the settable attribute bits ([`winattr::SETTABLE`]).
pub fn apply_meta(path: &[u8], meta: &WinMeta) -> Result<(), i32> {
    use std::os::windows::fs::FileTimesExt as _;
    use windows_sys::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_NORMAL};
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(os(path))
        .map_err(code)?;
    let times = std::fs::FileTimes::new()
        .set_created(system_time(meta.creation))
        .set_accessed(system_time(meta.access))
        .set_modified(system_time(meta.write));
    file.set_times(times).map_err(code)?;
    drop(file);
    let wanted = meta.attributes & winattr::SETTABLE;
    let wanted = if wanted == 0 { FILE_ATTRIBUTE_NORMAL } else { wanted };
    // SAFETY: a NUL-terminated wide path.
    if unsafe { SetFileAttributesW(wide(path).as_ptr(), wanted) } == 0 {
        return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
    }
    Ok(())
}

/// The name of the file system the volume of `path` has ("NTFS", "ReFS",
/// "FAT32" …), `None` when the volume cannot be asked.
pub fn fstype(path: &[u8]) -> Option<String> {
    use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
    let wide: Vec<u16> = os(path).encode_wide().map(|c| if c == u16::from(b'/') { u16::from(b'\\') } else { c }).chain(Some(0)).collect();
    let mut root = vec![0u16; 1024];
    // SAFETY: NUL-terminated path, a buffer of the length given.
    if unsafe { GetVolumePathNameW(wide.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return None;
    }
    let mut name = vec![0u16; 64];
    // SAFETY: NUL-terminated root from the call above, a buffer of the length given.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    Some(std::ffi::OsString::from_wide(&name[..end]).to_string_lossy().into_owned())
}

/// Sets the access and modification time of `path` (a directory too, a
/// link itself and not its target). Errors are the Win32 error code.
pub fn set_times(path: &[u8], atime: i64, mtime: i64) -> Result<(), i32> {
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(os(path))
        .map_err(code)?;
    let at = |secs: i64| {
        if secs >= 0 { UNIX_EPOCH + Duration::from_secs(secs as u64) } else { UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs()) }
    };
    let times = std::fs::FileTimes::new().set_accessed(at(atime)).set_modified(at(mtime));
    file.set_times(times).map_err(code)
}

/// Whether `t` is a time the file system can hold (not before 1601).
#[allow(dead_code)]
pub fn representable(t: SystemTime) -> bool {
    t.duration_since(UNIX_EPOCH).is_ok() || UNIX_EPOCH.duration_since(t).map(|d| d.as_secs() < 11_644_473_600).unwrap_or(false)
}

/// `IO_REPARSE_TAG_MOUNT_POINT`: the tag of a junction.
pub const TAG_MOUNT_POINT: u32 = 0xA000_0003;

/// Creates the junction `link` to the directory `target` (an absolute
/// path): an empty directory with a mount-point reparse point
/// (`FSCTL_SET_REPARSE_POINT`). A junction needs no special privilege.
pub fn create_junction(link: &[u8], target: &[u8]) -> Result<(), i32> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_REPARSE_POINT;
    std::fs::create_dir(os(link)).map_err(code)?;
    let result = (|| {
        let target_path: Vec<u16> = os(target).encode_wide().map(|c| if c == u16::from(b'/') { u16::from(b'\\') } else { c }).collect();
        let mut substitute: Vec<u16> = r"\??\".encode_utf16().collect();
        substitute.extend(&target_path);
        let (sub_bytes, print_bytes) = (substitute.len() * 2, target_path.len() * 2);
        let data_len = 8 + sub_bytes + 2 + print_bytes + 2;
        let mut buf: Vec<u8> = Vec::with_capacity(8 + data_len);
        buf.extend(TAG_MOUNT_POINT.to_le_bytes());
        buf.extend((data_len as u16).to_le_bytes());
        buf.extend(0u16.to_le_bytes());
        buf.extend(0u16.to_le_bytes());
        buf.extend((sub_bytes as u16).to_le_bytes());
        buf.extend(((sub_bytes + 2) as u16).to_le_bytes());
        buf.extend((print_bytes as u16).to_le_bytes());
        for unit in substitute.iter().chain(Some(&0u16)).chain(target_path.iter()).chain(Some(&0u16)) {
            buf.extend(unit.to_le_bytes());
        }
        let dir = std::fs::OpenOptions::new()
            .write(true)
            .share_mode(SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(os(link))
            .map_err(code)?;
        let mut returned = 0u32;
        // SAFETY: a valid handle, a buffer of the length given, no output buffer.
        let ok = unsafe {
            DeviceIoControl(
                dir.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                buf.as_ptr().cast(),
                buf.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)) } else { Ok(()) }
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir(os(link));
    }
    result
}

// ---------------------------------------------------------------------------
// BackupRead / BackupWrite: the Win32 data stream
// ---------------------------------------------------------------------------

/// Enables the backup privileges of the process token once: read any file
/// (`SeBackupPrivilege`), write any file and set any owner
/// (`SeRestorePrivilege`), take ownership (`SeTakeOwnershipPrivilege`).
/// A privilege the token does not have stays off; the calls then work for
/// the files the account may access anyway. Bareos: `win32/compat`
/// `enable_backup_privileges`.
pub fn enable_backup_privileges() {
    use std::sync::Once;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LUID};
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
        TOKEN_PRIVILEGES,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo handle of this process, an out pointer for the token.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token) } == 0 {
            return;
        }
        for name in ["SeBackupPrivilege", "SeRestorePrivilege", "SeTakeOwnershipPrivilege"] {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let mut luid = LUID { LowPart: 0, HighPart: 0 };
            // SAFETY: NUL-terminated name, out pointer for the LUID.
            if unsafe { LookupPrivilegeValueW(std::ptr::null(), wide.as_ptr(), &mut luid) } == 0 {
                continue;
            }
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
            };
            // SAFETY: a valid token with TOKEN_ADJUST_PRIVILEGES and a one-entry privilege set.
            unsafe { AdjustTokenPrivileges(token, 0, &privileges, 0, std::ptr::null_mut(), std::ptr::null_mut()) };
        }
        // SAFETY: the token handle opened above.
        unsafe { CloseHandle(token) };
    });
}

/// `GENERIC_READ`, `GENERIC_WRITE`, `WRITE_DAC`, `WRITE_OWNER`.
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;

/// The rights a file needs to receive a `BackupWrite` stream with its
/// security descriptor.
pub const RESTORE_ACCESS: u32 = GENERIC_WRITE | WRITE_DAC | WRITE_OWNER;

/// The Win32 data stream of a file as `BackupRead` produces it
/// (`WIN32_STREAM_ID` headers, names, the data stream, alternate data
/// streams, the security descriptor): what Bareos sends as
/// `STREAM_WIN32_DATA`.
pub struct BackupReader {
    file: std::fs::File,
    context: usize,
    done: bool,
}

/// Opens `path` for [`BackupReader`] (no link is followed, directories and
/// files held open by others for reading work, files locked against
/// sharing do not).
pub fn open_backup_reader(path: &[u8]) -> std::io::Result<BackupReader> {
    enable_backup_privileges();
    let file = std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT | 0x0800_0000) // | FILE_FLAG_SEQUENTIAL_SCAN
        .open(os(path))?;
    Ok(BackupReader { file, context: 0, done: false })
}

impl std::io::Read for BackupReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use windows_sys::Win32::Storage::FileSystem::BackupRead;
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        let mut read = 0u32;
        let mut context = self.context as *mut std::ffi::c_void;
        let wanted = buf.len().min(u32::MAX as usize) as u32;
        // SAFETY: a valid handle, a buffer of `wanted` bytes, the context of this reader only.
        let ok = unsafe { BackupRead(self.file.as_raw_handle(), buf.as_mut_ptr(), wanted, &mut read, 0, 1, &mut context) };
        self.context = context as usize;
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        if read == 0 {
            self.done = true;
        }
        Ok(read as usize)
    }
}

impl Drop for BackupReader {
    fn drop(&mut self) {
        use windows_sys::Win32::Storage::FileSystem::BackupRead;
        if self.context != 0 {
            let mut context = self.context as *mut std::ffi::c_void;
            let mut read = 0u32;
            // SAFETY: the same handle and context; abort = TRUE frees the context.
            unsafe { BackupRead(self.file.as_raw_handle(), std::ptr::null_mut(), 0, &mut read, 1, 0, &mut context) };
            self.context = 0;
        }
    }
}

/// Writes a `BackupRead` stream back into a file (`BackupWrite`), with its
/// security descriptor and alternate data streams.
#[derive(Default)]
pub struct BackupWriter {
    context: usize,
}

impl BackupWriter {
    /// Writes `buf` into `file`, which must have been opened with
    /// [`RESTORE_ACCESS`].
    pub fn process(&mut self, buf: &[u8], file: &mut std::fs::File) -> std::io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::BackupWrite;
        enable_backup_privileges();
        let mut rest = buf;
        while !rest.is_empty() {
            let mut written = 0u32;
            let mut context = self.context as *mut std::ffi::c_void;
            let wanted = rest.len().min(u32::MAX as usize) as u32;
            // SAFETY: a valid handle, a buffer of `wanted` bytes, the context of this writer only.
            let ok = unsafe { BackupWrite(file.as_raw_handle(), rest.as_ptr(), wanted, &mut written, 0, 1, &mut context) };
            self.context = context as usize;
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if written == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "BackupWrite wrote nothing"));
            }
            rest = &rest[written as usize..];
        }
        Ok(())
    }

    /// Ends the stream (frees the context of `BackupWrite`).
    pub fn finish(&mut self, file: &std::fs::File) {
        use windows_sys::Win32::Storage::FileSystem::BackupWrite;
        if self.context != 0 {
            let mut context = self.context as *mut std::ffi::c_void;
            let mut written = 0u32;
            // SAFETY: the same handle and context; abort = TRUE frees the context.
            unsafe { BackupWrite(file.as_raw_handle(), std::ptr::null(), 0, &mut written, 1, 0, &mut context) };
            self.context = 0;
        }
    }
}
