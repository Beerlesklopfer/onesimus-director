//! Plattformteil des Rings: gemeinsamer Speicher und Warten/Wecken.
//!
//! Zweck: Nur hier stehen Betriebssystem-Aufrufe. Der Rest des Crates kennt
//! einen Ring nur beim Namen und benutzt [`Mapping`], [`wait`] und [`wake`].
//!
//! | Plattform | gemeinsamer Speicher | Warten/Wecken | Stand |
//! |---|---|---|---|
//! | Linux | Datei in `/dev/shm` (`open`, `ftruncate`, `mmap MAP_SHARED`) | `futex` ohne `FUTEX_PRIVATE_FLAG` (prozessübergreifend) | umgesetzt (Stufe 1) |
//! | macOS | `shm_open` + `mmap` | `os_sync_wait_on_address` (shared) oder POSIX-Semaphore | Platzhalter |
//! | Windows | benanntes `CreateFileMapping` | benannte Events | Platzhalter |
//!
//! Die Platzhalter übersetzen, enden aber zur Laufzeit mit `unimplemented!()`.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(target_os = "linux"))]
mod other;
#[cfg(not(target_os = "linux"))]
pub use other::*;
