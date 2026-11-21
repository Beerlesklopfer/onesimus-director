//! Logging-Einrichtung (`tracing`).
//!
//! Zweck: Alle Daemons loggen über `tracing` nach stderr. Der Debug-Level
//! `-d` schaltet von `info` auf `debug`; laute Bibliotheken (hyper, h2 …)
//! bleiben eine Stufe darunter. `RUST_LOG` überschreibt den Filter.
//!
//! Datenfluss: `main` → [`init`] → globaler `tracing`-Subscriber → stderr
//! (unter systemd ins Journal) und, wenn angegeben, eine Log-Datei unter
//! [`LOG_DIR`] (`/var/log/onesimus/<daemon>.log`, Option `--log-file`).
//! Farbcodes nur, wenn ausschließlich auf ein Terminal geschrieben wird.
//!
//! Bareos-Gegenstück: `lib/message.cc` (`InitMsg`, `debug_level`) – Bareos
//! hat eigene Debug-Ausgaben; die Bareos-kompatible `Dmsg`-Ausgabe der
//! Daemons liegt in `bareos_bnet::debug` und bleibt dort.

/// The default filter: `debug` from debug level 1 on, else `info`; the
/// crates in `quiet` one step lower (`info` resp. `warn`).
pub fn default_filter(debug_level: u32, quiet: &[&str]) -> String {
    let (main, lower) = if debug_level > 0 { ("debug", "info") } else { ("info", "warn") };
    let mut filter = main.to_string();
    for name in quiet {
        filter.push_str(&format!(",{name}={lower}"));
    }
    filter
}

/// Directory of the daemons' log files.
pub const LOG_DIR: &str = "/var/log/onesimus";

/// The default log file of `daemon`: `/var/log/onesimus/<daemon>.log`.
pub fn default_log_file(daemon: &str) -> std::path::PathBuf {
    std::path::Path::new(LOG_DIR).join(format!("{daemon}.log"))
}

/// The `--log-file` value as a path; `none`, `-` or empty: no log file.
pub fn log_file_arg(value: &std::path::Path) -> Option<std::path::PathBuf> {
    match value.to_str() {
        Some("none" | "-" | "") => None,
        _ => Some(value.to_path_buf()),
    }
}

/// An open log file shared by every writer of the subscriber.
pub type LogFile = std::sync::Arc<std::sync::Mutex<std::fs::File>>;

/// Opens `path` for appending (the directory is created); without the right
/// to do so a warning on stderr and None: the daemon logs to stderr only.
pub fn open_log_file(path: &std::path::Path) -> Option<LogFile> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => Some(std::sync::Arc::new(std::sync::Mutex::new(file))),
        Err(e) => {
            eprintln!("warning: cannot open log file {}: {e}; logging to stderr only", path.display());
            None
        }
    }
}

/// A log line goes to `inner` (stderr or a trace file) and to the log file.
pub struct TeeWriter<W> {
    /// The usual destination.
    pub inner: W,
    /// The log file, if one is open.
    pub file: Option<LogFile>,
}

impl<W: std::io::Write> std::io::Write for TeeWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(file) = &self.file {
            let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
            let _ = file.write_all(bytes);
        }
        self.inner.write_all(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = &self.file {
            let _ = file.lock().unwrap_or_else(|e| e.into_inner()).flush();
        }
        self.inner.flush()
    }
}

/// Colour only when the log goes to a terminal and nowhere else.
pub fn use_ansi(file: Option<&LogFile>) -> bool {
    use std::io::IsTerminal;
    file.is_none() && std::io::stderr().is_terminal()
}

/// Installs the global subscriber: `RUST_LOG` if set, else `filter`; lines
/// to stderr and to `log_file` (see [`open_log_file`]).
pub fn init(filter: &str, log_file: Option<&std::path::Path>) {
    use tracing_subscriber::EnvFilter;
    let file = log_file.and_then(open_log_file);
    let ansi = use_ansi(file.as_ref());
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)))
        .with_ansi(ansi)
        .with_writer(move || TeeWriter { inner: std::io::stderr(), file: file.clone() })
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_paths() {
        assert_eq!(default_log_file("osd"), std::path::PathBuf::from("/var/log/onesimus/osd.log"));
        assert_eq!(log_file_arg(std::path::Path::new("none")), None);
        assert_eq!(log_file_arg(std::path::Path::new("-")), None);
        assert_eq!(log_file_arg(std::path::Path::new("/tmp/x.log")), Some("/tmp/x.log".into()));
    }

    #[test]
    fn tee_writes_both_and_a_file_means_no_colour() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("onesimus-log-{}", std::process::id()));
        let path = dir.join("sub").join("t.log");
        let file = open_log_file(&path).unwrap();
        let mut tee = TeeWriter { inner: Vec::new(), file: Some(file.clone()) };
        tee.write_all(b"line one\n").unwrap();
        tee.flush().unwrap();
        assert_eq!(tee.inner, b"line one\n");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "line one\n");
        assert!(!use_ansi(Some(&file)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ofd_filters_are_unchanged() {
        // ofd/src/main.rs before 19.11.2026
        assert_eq!(default_filter(100, &["hyper", "h2"]), "debug,hyper=info,h2=info");
        assert_eq!(default_filter(0, &["hyper", "h2"]), "info,hyper=warn,h2=warn");
    }
}
