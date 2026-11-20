//! Plattformteil des Rings: gemeinsamer Speicher und Warten/Wecken.
//!
//! Zweck: Nur hier stehen Betriebssystem-Aufrufe. Der Rest des Crates kennt
//! einen Ring nur beim Namen und benutzt [`Mapping`], [`wait`] und [`wake`].
//!
//! Gemeinsamer Speicher ist auf jeder Plattform eine gewöhnliche Datei, die
//! osd selbst anlegt und mit `memmap2` gemeinsam einblendet (Recherche
//! `docs/plans/OSD-RECHERCHE-SPEICHER-UND-RING.md`, Teil B). So läuft auch
//! die Suche nach verwaisten Ringen überall gleich.
//!
//! | Plattform | Verzeichnis der Ringe | Warten/Wecken | Stand |
//! |---|---|---|---|
//! | Linux | `/dev/shm` | `futex` ohne `FUTEX_PRIVATE_FLAG` (prozessübergreifend) | umgesetzt (Stufe 1) |
//! | macOS | `$TMPDIR/osd-rings` (privat, 0700) | `os_sync_wait_on_address` mit `OS_SYNC_WAIT_ON_ADDRESS_SHARED` (ab 14.4) | Platzhalter |
//! | Windows | `%TEMP%\osd-rings`, `FILE_ATTRIBUTE_TEMPORARY` | benannte Auto-Reset-Events je Wartendem (`WaitOnAddress` wirkt nicht prozessübergreifend) | Platzhalter |
//!
//! Gewartet wird immer mit Zeitgrenze (der Ring nimmt 100 ms), damit die
//! Lebendprüfungen auch nach einem verlorenen Wecken laufen. Die
//! Platzhalter übersetzen, `wait`/`wake` enden dort aber mit
//! `unimplemented!()`.

use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod wait_linux;
#[cfg(target_os = "linux")]
pub use wait_linux::{wait, wake};

#[cfg(not(target_os = "linux"))]
mod wait_other;
#[cfg(not(target_os = "linux"))]
pub use wait_other::{wait, wake};

/// A shared mapping of a ring file.
#[derive(Debug)]
pub struct Mapping {
    /// The memory map.
    map: memmap2::MmapMut,
}

impl Mapping {
    /// Start of the mapping.
    pub fn ptr(&self) -> *mut u8 {
        self.map.as_ptr() as *mut u8
    }

    /// Length of the mapping.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True for an empty mapping (never for a ring).
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Directory of the ring files on this platform.
fn ring_dir() -> io::Result<PathBuf> {
    if cfg!(target_os = "linux") {
        return Ok(PathBuf::from("/dev/shm"));
    }
    let dir = std::env::temp_dir().join("osd-rings");
    if !dir.is_dir() {
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(dir)
}

/// The file of ring `name`.
fn path(name: &str) -> io::Result<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid ring name {name:?}")));
    }
    Ok(ring_dir()?.join(name))
}

/// Maps `file` shared, read and write.
fn map(file: &std::fs::File) -> io::Result<Mapping> {
    // SAFETY: the file is ours; concurrent access by other processes is the
    // point of the ring and is synchronised through the header atomics.
    let map = unsafe { memmap2::MmapOptions::new().map_mut(file)? };
    Ok(Mapping { map })
}

/// Creates ring `name` with `len` zeroed bytes; fails if it exists.
pub fn create(name: &str, len: usize) -> io::Result<Mapping> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.attributes(0x100); // FILE_ATTRIBUTE_TEMPORARY
    }
    let file = options.open(path(name)?)?;
    file.set_len(len as u64)?;
    map(&file)
}

/// Opens the existing ring `name`.
pub fn open(name: &str) -> io::Result<Mapping> {
    let file = OpenOptions::new().read(true).write(true).open(path(name)?)?;
    map(&file)
}

/// Removes ring `name`; mappings stay valid until dropped.
pub fn unlink(name: &str) -> io::Result<()> {
    std::fs::remove_file(path(name)?)
}

/// Names of the rings starting with `prefix`.
pub fn list(prefix: &str) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(ring_dir()?)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix) {
            names.push(name);
        }
    }
    Ok(names)
}

/// The calling process id.
pub fn current_pid() -> u64 {
    std::process::id().into()
}

/// Whether process `pid` exists (`kill(pid, 0)`; `EPERM` counts as alive).
#[cfg(unix)]
pub fn pid_alive(pid: u64) -> bool {
    if pid == 0 || pid > i32::MAX as u64 {
        return false;
    }
    // SAFETY: signal 0 only checks for existence.
    let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
    r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether process `pid` exists (not implemented on this platform).
#[cfg(not(unix))]
pub fn pid_alive(_pid: u64) -> bool {
    unimplemented!("osd ring: process check on this platform (OpenProcess)")
}
