//! The `st_mode` bits of libc as `u32`, the type of `Stat::mode`.
//!
//! `mode_t` is `u32` on Linux and `u16` on macOS; the bit values are the
//! same (POSIX), so the catalog/stream encoding does not change.
//!
//! Windows: the C runtime knows only a few of these; the values are the
//! POSIX ones, as Bareos defines them for Win32 (`win32/compat/include/
//! compat.h`), because the stream carries POSIX modes on every platform.

#![allow(clippy::unnecessary_cast)]

/// S_IFMT: file type mask.
#[cfg(unix)]
pub const S_IFMT: u32 = libc::S_IFMT as u32;
/// S_IFMT: file type mask.
#[cfg(windows)]
pub const S_IFMT: u32 = 0o170000;
/// S_IFREG: regular file.
#[cfg(unix)]
pub const S_IFREG: u32 = libc::S_IFREG as u32;
/// S_IFREG: regular file.
#[cfg(windows)]
pub const S_IFREG: u32 = 0o100000;
/// S_IFDIR: directory.
#[cfg(unix)]
pub const S_IFDIR: u32 = libc::S_IFDIR as u32;
/// S_IFDIR: directory.
#[cfg(windows)]
pub const S_IFDIR: u32 = 0o040000;
/// S_IFLNK: symbolic link.
#[cfg(unix)]
pub const S_IFLNK: u32 = libc::S_IFLNK as u32;
/// S_IFLNK: symbolic link.
#[cfg(windows)]
pub const S_IFLNK: u32 = 0o120000;
/// S_IFBLK: block device.
#[cfg(unix)]
pub const S_IFBLK: u32 = libc::S_IFBLK as u32;
/// S_IFBLK: block device.
#[cfg(windows)]
pub const S_IFBLK: u32 = 0o060000;
/// S_IFCHR: character device.
#[cfg(unix)]
pub const S_IFCHR: u32 = libc::S_IFCHR as u32;
/// S_IFCHR: character device.
#[cfg(windows)]
pub const S_IFCHR: u32 = 0o020000;
/// S_IFIFO: FIFO.
#[cfg(unix)]
pub const S_IFIFO: u32 = libc::S_IFIFO as u32;
/// S_IFIFO: FIFO.
#[cfg(windows)]
pub const S_IFIFO: u32 = 0o010000;
/// S_IFSOCK: socket.
#[cfg(unix)]
pub const S_IFSOCK: u32 = libc::S_IFSOCK as u32;
/// S_IFSOCK: socket.
#[cfg(windows)]
pub const S_IFSOCK: u32 = 0o140000;
/// S_ISUID: set-user-id.
#[cfg(unix)]
pub const S_ISUID: u32 = libc::S_ISUID as u32;
/// S_ISUID: set-user-id.
#[cfg(windows)]
pub const S_ISUID: u32 = 0o4000;
/// S_ISGID: set-group-id.
#[cfg(unix)]
pub const S_ISGID: u32 = libc::S_ISGID as u32;
/// S_ISGID: set-group-id.
#[cfg(windows)]
pub const S_ISGID: u32 = 0o2000;
/// S_ISVTX: sticky.
#[cfg(unix)]
pub const S_ISVTX: u32 = libc::S_ISVTX as u32;
/// S_ISVTX: sticky.
#[cfg(windows)]
pub const S_ISVTX: u32 = 0o1000;
/// S_IRUSR: owner read.
#[cfg(unix)]
pub const S_IRUSR: u32 = libc::S_IRUSR as u32;
/// S_IRUSR: owner read.
#[cfg(windows)]
pub const S_IRUSR: u32 = 0o400;
/// S_IWUSR: owner write.
#[cfg(unix)]
pub const S_IWUSR: u32 = libc::S_IWUSR as u32;
/// S_IWUSR: owner write.
#[cfg(windows)]
pub const S_IWUSR: u32 = 0o200;
/// S_IXUSR: owner execute.
#[cfg(unix)]
pub const S_IXUSR: u32 = libc::S_IXUSR as u32;
/// S_IXUSR: owner execute.
#[cfg(windows)]
pub const S_IXUSR: u32 = 0o100;
/// S_IRGRP: group read.
#[cfg(unix)]
pub const S_IRGRP: u32 = libc::S_IRGRP as u32;
/// S_IRGRP: group read.
#[cfg(windows)]
pub const S_IRGRP: u32 = 0o040;
/// S_IWGRP: group write.
#[cfg(unix)]
pub const S_IWGRP: u32 = libc::S_IWGRP as u32;
/// S_IWGRP: group write.
#[cfg(windows)]
pub const S_IWGRP: u32 = 0o020;
/// S_IXGRP: group execute.
#[cfg(unix)]
pub const S_IXGRP: u32 = libc::S_IXGRP as u32;
/// S_IXGRP: group execute.
#[cfg(windows)]
pub const S_IXGRP: u32 = 0o010;
/// S_IROTH: others read.
#[cfg(unix)]
pub const S_IROTH: u32 = libc::S_IROTH as u32;
/// S_IROTH: others read.
#[cfg(windows)]
pub const S_IROTH: u32 = 0o004;
/// S_IWOTH: others write.
#[cfg(unix)]
pub const S_IWOTH: u32 = libc::S_IWOTH as u32;
/// S_IWOTH: others write.
#[cfg(windows)]
pub const S_IWOTH: u32 = 0o002;
/// S_IXOTH: others execute.
#[cfg(unix)]
pub const S_IXOTH: u32 = libc::S_IXOTH as u32;
/// S_IXOTH: others execute.
#[cfg(windows)]
pub const S_IXOTH: u32 = 0o001;

/// A `u32` mode as the platform's `mode_t` (for chmod/mkfifo/mknod).
#[cfg(unix)]
pub fn to_mode_t(mode: u32) -> libc::mode_t {
    mode as libc::mode_t
}
