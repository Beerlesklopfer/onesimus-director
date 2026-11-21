//! The `st_mode` bits of libc as `u32`, the type of `Stat::mode`.
//!
//! `mode_t` is `u32` on Linux and `u16` on macOS; the bit values are the
//! same (POSIX), so the catalog/stream encoding does not change.

#![allow(clippy::unnecessary_cast)]

/// S_IFMT: file type mask.
pub const S_IFMT: u32 = libc::S_IFMT as u32;
/// S_IFREG: regular file.
pub const S_IFREG: u32 = libc::S_IFREG as u32;
/// S_IFDIR: directory.
pub const S_IFDIR: u32 = libc::S_IFDIR as u32;
/// S_IFLNK: symbolic link.
pub const S_IFLNK: u32 = libc::S_IFLNK as u32;
/// S_IFBLK: block device.
pub const S_IFBLK: u32 = libc::S_IFBLK as u32;
/// S_IFCHR: character device.
pub const S_IFCHR: u32 = libc::S_IFCHR as u32;
/// S_IFIFO: FIFO.
pub const S_IFIFO: u32 = libc::S_IFIFO as u32;
/// S_IFSOCK: socket.
pub const S_IFSOCK: u32 = libc::S_IFSOCK as u32;
/// S_ISUID: set-user-id.
pub const S_ISUID: u32 = libc::S_ISUID as u32;
/// S_ISGID: set-group-id.
pub const S_ISGID: u32 = libc::S_ISGID as u32;
/// S_ISVTX: sticky.
pub const S_ISVTX: u32 = libc::S_ISVTX as u32;
/// S_IRUSR: owner read.
pub const S_IRUSR: u32 = libc::S_IRUSR as u32;
/// S_IWUSR: owner write.
pub const S_IWUSR: u32 = libc::S_IWUSR as u32;
/// S_IXUSR: owner execute.
pub const S_IXUSR: u32 = libc::S_IXUSR as u32;
/// S_IRGRP: group read.
pub const S_IRGRP: u32 = libc::S_IRGRP as u32;
/// S_IWGRP: group write.
pub const S_IWGRP: u32 = libc::S_IWGRP as u32;
/// S_IXGRP: group execute.
pub const S_IXGRP: u32 = libc::S_IXGRP as u32;
/// S_IROTH: others read.
pub const S_IROTH: u32 = libc::S_IROTH as u32;
/// S_IWOTH: others write.
pub const S_IWOTH: u32 = libc::S_IWOTH as u32;
/// S_IXOTH: others execute.
pub const S_IXOTH: u32 = libc::S_IXOTH as u32;

/// A `u32` mode as the platform's `mode_t` (for chmod/mkfifo/mknod).
pub fn to_mode_t(mode: u32) -> libc::mode_t {
    mode as libc::mode_t
}
