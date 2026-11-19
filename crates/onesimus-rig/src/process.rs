//! Gestartete Daemons des Gerüsts und ihr sicheres Ende.
//!
//! Zweck: Jeder Daemon läuft in einer eigenen Prozessgruppe, damit ein
//! Signal auch seine Kinder erreicht (bareos-sd startet z. B. Hilfsprozesse).
//! [`Daemon`] beendet die Gruppe in `Drop`: erst `SIGTERM`, nach einer Frist
//! `SIGKILL`. So bleibt auch nach einer Panik im Test kein Prozess zurück.
//!
//! Datenfluss: [`Daemon::spawn`] startet das Programm mit Ausgabe in eine
//! Logdatei; [`wait_port`] wartet, bis der Daemon auf seinem Port annimmt.
//!
//! Bareos-Gegenstück: `systemtests/scripts/functions` (`start_test`,
//! `stop_bareos`), dort per Shell und PID-Datei.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long a daemon gets after `SIGTERM` before `SIGKILL`.
const TERM_GRACE: Duration = Duration::from_secs(10);

/// One running daemon in its own process group, ended on drop.
#[derive(Debug)]
pub struct Daemon {
    /// Short name for messages (`odir`, `ofd`, `sd`).
    pub name: String,
    /// The child process; `None` after [`Daemon::stop`].
    child: Option<Child>,
    /// File that receives stdout and stderr of the daemon.
    pub log: PathBuf,
}

impl Daemon {
    /// Starts `program args…` with output to `log`, in a new process group.
    pub fn spawn(name: &str, program: &Path, args: &[&str], log: &Path) -> Result<Daemon, String> {
        use std::os::unix::process::CommandExt;
        let out = std::fs::File::create(log).map_err(|e| format!("{}: {e}", log.display()))?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let child = Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .env("LANG", "C")
            .env("TZ", "UTC")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .process_group(0)
            .spawn()
            .map_err(|e| format!("{name}: {} startet nicht: {e}", program.display()))?;
        Ok(Daemon { name: name.into(), child: Some(child), log: log.to_path_buf() })
    }

    /// The exit status, if the daemon has ended already.
    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.as_mut().and_then(|c| c.try_wait().ok().flatten())
    }

    /// Ends the process group: `SIGTERM`, after 10 s (`TERM_GRACE`) `SIGKILL`.
    pub fn stop(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        let group = child.id() as libc::pid_t;
        // SAFETY: kill() with a negative pid signals our own child's group.
        unsafe { libc::kill(-group, libc::SIGTERM) };
        let deadline = Instant::now() + TERM_GRACE;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                // also take down helpers left in the group
                unsafe { libc::kill(-group, libc::SIGKILL) };
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        unsafe { libc::kill(-group, libc::SIGKILL) };
        let _ = child.wait();
    }

    /// The last `lines` lines of the daemon log, for failure messages.
    pub fn log_tail(&self, lines: usize) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let all: Vec<&str> = text.lines().collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Waits until `address` accepts TCP connections, or `timeout` passes or
/// the daemon ends; `Err` names the reason and shows the end of its log.
pub fn wait_port(daemon: &mut Daemon, address: SocketAddr, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some(status) = daemon.exited() {
            return Err(format!("{} ist beendet ({status}):\n{}", daemon.name, daemon.log_tail(30)));
        }
        if TcpStream::connect_timeout(&address, Duration::from_millis(300)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("{} nimmt auf {address} nicht an:\n{}", daemon.name, daemon.log_tail(30)))
}

/// Whether nothing listens on `address` (the port is free for the rig).
pub fn port_free(address: SocketAddr) -> bool {
    std::net::TcpListener::bind(address).is_ok()
}
