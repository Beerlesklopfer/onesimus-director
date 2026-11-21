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
    FileBasicInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, GetVolumeInformationW,
    GetVolumePathNameW, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY,
    FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
};

use crate::platform::OsStrExt;

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
pub fn stat(path: &[u8]) -> Result<Stat, i32> {
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
    Ok(Stat {
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
    })
}

/// The Win32 file attributes of `path` (`GetFileAttributes`, without
/// following a link).
pub fn attributes(path: &[u8]) -> Result<u32, i32> {
    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(os(path))
        .map_err(code)?;
    // SAFETY: zeroed plain-data struct, a valid handle.
    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        Ok(info.dwFileAttributes)
    }
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
