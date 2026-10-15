//! Debug-Level System — 1:1 kompatibel mit C++ Bareos d_msg/Dmsg.
//!
//! C++ Original (baconfig.h):
//!   #define Dmsg0(lvl, ...)
//!     if ((lvl) <= debug_level) d_msg(__FILE__, __LINE__, lvl, __VA_ARGS__)
//!
//! d_msg Ausgabe-Format:
//!   [timestamp] daemon_name (level): file:line-jobid message
//!
//! Level-Konvention (aus C++ Quellen):
//!   1-49    Basis (Startup, Shutdown, Config)
//!   50-99   Ausfuehrlich (Commands, Session-Lifecycle)
//!   100     Protokoll (TLS, Auth, DoTlsHandshake)
//!   110     Daemon-Messages (>stored:, <stored:)
//!   116     Wire (>filed:, <filed: — gesendete/empfangene Strings)
//!   150     Detail (einzelne Schritte in Auth, Config-Parse)
//!   200     Trace (Memory, Locks, jeden Funktionsaufruf)

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::SystemTime;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::os::unix::fs::OpenOptionsExt;

/// One synchronized sink for protocol debug messages and daemon diagnostics.
/// Opening a new path is atomic with respect to writers and preserves the old
/// trace if opening fails. Trace files are append-only and never follow symlinks.
struct TraceOutput {
    file: Mutex<Option<(PathBuf, File)>>,
    process_id: AtomicU32,
}

impl TraceOutput {
    const fn new() -> Self { Self { file: Mutex::new(None), process_id: AtomicU32::new(0) } }

    fn set_enabled(&self, path: &Path, enabled: bool) -> io::Result<()> {
        let pid = std::process::id();
        let owner = self.process_id.compare_exchange(0, pid, Ordering::Relaxed, Ordering::Relaxed).unwrap_or_else(|owner| owner);
        if owner != 0 && owner != pid {
            return Err(io::Error::other("cannot reconfigure an inherited trace sink"));
        }
        let mut current = self.file.lock().map_err(|_| io::Error::other("trace lock poisoned"))?;
        if !enabled {
            if let Some((_, file)) = current.as_mut() { file.flush()?; }
            *current = None;
            return Ok(());
        }
        if current.as_ref().is_some_and(|(active, _)| active == path) { return Ok(()); }
        let file = OpenOptions::new().append(true).create(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
        if !file.metadata()?.is_file() { return Err(io::Error::other("trace path is not a regular file")); }
        *current = Some((path.to_path_buf(), file));
        Ok(())
    }

    fn write(&self, bytes: &[u8]) -> io::Result<bool> {
        // Workers close inherited descriptors after fork. Never touch an
        // inherited lock, which may have been held by another parent thread.
        if self.process_id.load(Ordering::Relaxed) != std::process::id() { return Ok(false); }
        let mut current = self.file.lock().map_err(|_| io::Error::other("trace lock poisoned"))?;
        let Some((_, file)) = current.as_mut() else { return Ok(false); };
        if let Err(error) = file.write_all(bytes).and_then(|_| file.flush()) {
            *current = None;
            return Err(error);
        }
        Ok(true)
    }
}

static TRACE_OUTPUT: TraceOutput = TraceOutput::new();

pub fn set_trace(path: &Path, enabled: bool) -> io::Result<()> {
    TRACE_OUTPUT.set_enabled(path, enabled)
}

pub fn trace_enabled() -> bool {
    TRACE_OUTPUT.process_id.load(Ordering::Relaxed) == std::process::id()
        && TRACE_OUTPUT.file.lock().is_ok_and(|file| file.is_some())
}

pub fn debug_timestamps() -> bool { DEBUG_TIMESTAMPS.load(Ordering::Relaxed) }

/// Writer for tracing_subscriber; use the same destination as d_msg.
pub struct DebugWriter;

impl Write for DebugWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match TRACE_OUTPUT.write(bytes) {
            Ok(true) => Ok(bytes.len()),
            Ok(false) => io::stderr().write(bytes),
            Err(error) => {
                eprintln!("Trace output failed; reverting to stderr: {error}");
                io::stderr().write(bytes)
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> { io::stderr().flush() }
}


/// Globaler Debug-Level (gesetzt via -d CLI-Flag)
static DEBUG_LEVEL: AtomicU32 = AtomicU32::new(0);

/// Debug-Timestamps aktiviert (--dt Flag)
static DEBUG_TIMESTAMPS: AtomicBool = AtomicBool::new(false);

/// Daemon-Name fuer Debug-Ausgabe
static DAEMON_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Setze den globalen Debug-Level.
pub fn set_debug_level(level: u32) {
    DEBUG_LEVEL.store(level, Ordering::Relaxed);
}

/// Lese den aktuellen Debug-Level.
pub fn debug_level() -> u32 {
    DEBUG_LEVEL.load(Ordering::Relaxed)
}

/// Aktiviere/deaktiviere Timestamps in Debug-Ausgabe.
pub fn set_debug_timestamps(enabled: bool) {
    DEBUG_TIMESTAMPS.store(enabled, Ordering::Relaxed);
}

/// Setze den Daemon-Namen (z.B. "bareos-dir", "bareos-fd").
pub fn set_daemon_name(name: &str) {
    let _ = DAEMON_NAME.set(name.to_string());
}

/// Prüfe ob ein bestimmter Level aktiv ist.
#[inline]
pub fn is_debug(level: u32) -> bool {
    level <= DEBUG_LEVEL.load(Ordering::Relaxed)
}

/// Debug-Nachricht ausgeben — Äquivalent zu C++ d_msg().
///
/// Format: [timestamp] daemon (level): file:line message
pub fn d_msg(file: &str, line: u32, level: u32, msg: &str) {
    if !is_debug(level) {
        return;
    }

    let daemon = DAEMON_NAME.get().map(|s| s.as_str()).unwrap_or("bareos");

    let mut output = String::new();
    if DEBUG_TIMESTAMPS.load(Ordering::Relaxed) {
        let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        output.push_str(&format!("{}.{:06} ", now.as_secs(), now.subsec_micros()));
    }
    let basename = file.rsplit('/').next().unwrap_or(file);
    output.push_str(&format!("{daemon} ({level}): {basename}:{line} {msg}\n"));
    let _ = DebugWriter.write_all(output.as_bytes());
}

/// Bareos-kompatibles Dmsg Macro.
///
/// Verwendung: `dmsg!(100, "TLS handshake OK for {}", client_name);`
///
/// Gibt aus: `bareos-dir (100): accept.rs:42 TLS handshake OK for bareos-fd`
#[macro_export]
macro_rules! dmsg {
    ($level:expr, $($arg:tt)*) => {
        if $crate::debug::is_debug($level) {
            $crate::debug::d_msg(file!(), line!(), $level, &format!($($arg)*));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_level_default() {
        set_debug_level(0);
        assert_eq!(debug_level(), 0);
        assert!(!is_debug(1));
        assert!(is_debug(0)); // Level 0 = immer wenn debug_level >= 0
    }

    #[test]
    fn debug_level_set() {
        set_debug_level(100);
        assert!(is_debug(1));
        assert!(is_debug(50));
        assert!(is_debug(100));
        assert!(!is_debug(101));
        set_debug_level(0);
    }

    #[test]
    fn d_msg_output() {
        set_debug_level(100);
        set_daemon_name("test-dir");
        // Sollte nicht paniken
        d_msg("src/test.rs", 42, 100, "test message");
        d_msg("src/test.rs", 42, 200, "should not print"); // level too high
        set_debug_level(0);
    }

    #[test]
    fn timestamps() {
        set_debug_level(1);
        set_debug_timestamps(true);
        d_msg("test.rs", 1, 1, "with timestamp");
        set_debug_timestamps(false);
        d_msg("test.rs", 1, 1, "without timestamp");
        set_debug_level(0);
    }
}

#[cfg(test)]
mod trace_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn trace_appends_flushes_and_stops_without_global_test_state() {
        let dir = std::env::temp_dir().join(format!("onesimus-trace-{}-{}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("test.trace");
        let sink = TraceOutput::new();
        assert!(!sink.write(b"before").unwrap());
        sink.set_enabled(&path, true).unwrap();
        assert!(sink.write(b"first\n").unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"first\n");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        sink.set_enabled(&path, false).unwrap();
        assert!(!sink.write(b"after").unwrap());
        sink.set_enabled(&path, true).unwrap();
        sink.write(b"second\n").unwrap();
        // A failed replacement must not disable the existing sink.
        assert!(sink.set_enabled(&dir.join("missing/trace"), true).is_err());
        sink.write(b"third\n").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"first\nsecond\nthird\n");
        sink.set_enabled(&path, false).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(sink.set_enabled(&link, true).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn inherited_sink_bypasses_parent_mutex_and_descriptors() {
        let sink = TraceOutput::new();
        sink.process_id.store(std::process::id().wrapping_add(1), Ordering::Relaxed);
        let _held_by_parent = sink.file.lock().unwrap();
        assert!(!sink.write(b"child diagnostic").unwrap());
        assert!(sink.set_enabled(Path::new("unused"), true).is_err());
    }

}
