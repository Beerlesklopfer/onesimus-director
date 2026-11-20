//! Das Gerüst als Ganzes: anlegen, starten, befragen, abräumen.
//!
//! Zweck: [`Rig`] bündelt Verzeichnis, Katalog und Daemons eines Laufs und
//! bietet die Schritte, die ein Test braucht (Job starten, auf das Ende
//! warten, Katalog lesen, Restore vergleichen). `Drop` räumt in umgekehrter
//! Reihenfolge ab: Director, FD, SD, Katalog, Verzeichnis – auch wenn der
//! Test vorher mit einer Panik endet.
//!
//! Datenfluss: siehe Klassendiagramm an [`Rig`].
//!
//! Bareos-Gegenstück: `systemtests/scripts/functions` (`start_test`,
//! `run_bareos`, `check_for_zombie_jobs`, `end_test`).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::catalog::Cluster;
use crate::config::{CLIENT, CONSOLE_PASSWORD, Layout, Ports};
use crate::console::{Console, queued_job_id};
use crate::prereq::Programs;
use crate::process::{Daemon, port_free, wait_port};

/// Job states that end a job (`Job.JobStatus`).
const FINAL_STATES: &[char] = &['T', 'W', 'E', 'e', 'f', 'A', 'D'];

/// How a rig is set up.
#[derive(Debug, Clone)]
pub struct RigOptions {
    /// First of the four ports (Director, FD, SD, catalog).
    pub base_port: u16,
    /// Keep the directory after the run (`ONESIMUS_RIG_KEEP=1`).
    pub keep: bool,
    /// Debug level of odir and ofd (`-d`).
    pub debug_level: u32,
    /// Start the rig's own `ofd` (client `vfd`). `false` when the test brings
    /// its File Daemon itself, e.g. a packaged ofd in a container
    /// (`tests/ldap_o7.rs`); the FD port is then not checked either.
    pub start_fd: bool,
    /// Additional or replacing Director resources as (relative path, text),
    /// see [`crate::config::write_director_resources`].
    pub extra_director: Vec<(String, String)>,
    /// `Auditing = yes` in the Director; audit messages go to
    /// `dir-work/audit.log` ([`Rig::daemon_log`] with `audit`).
    pub auditing: bool,
    /// The Storage Daemon to start; takes precedence over `ONESIMUS_RIG_SD`
    /// (e.g. `target/debug/osd` in `tests/osd.rs`). `None`: the variable or
    /// `/usr/sbin/bareos-sd`.
    pub sd: Option<PathBuf>,
    /// `Passive = yes` on the Client: the SD connects to the FD instead of
    /// the FD to the SD (`passive client address=… port=… ssl=…`).
    pub passive_client: bool,
}

impl RigOptions {
    /// Options for `base_port`; `keep` from `ONESIMUS_RIG_KEEP`.
    pub fn new(base_port: u16) -> RigOptions {
        RigOptions {
            base_port,
            keep: std::env::var_os("ONESIMUS_RIG_KEEP").is_some_and(|v| v != "0"),
            debug_level: 100,
            start_fd: true,
            extra_director: Vec::new(),
            auditing: false,
            sd: None,
            passive_client: false,
        }
    }
}

/// One row of the catalog table `Job`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    /// `Job.JobId`.
    pub id: u32,
    /// `Job.Type` (`B` backup, `R` restore, `V` verify).
    pub kind: char,
    /// `Job.Level` (`F` full, `O` VolumeToCatalog …).
    pub level: char,
    /// `Job.JobStatus` (glossary: Job-Status).
    pub status: char,
    /// `Job.JobFiles`.
    pub files: u64,
    /// `Job.JobBytes`.
    pub bytes: u64,
    /// `Job.JobErrors`.
    pub errors: u64,
}

/// The `2800 End Job` report of the FD for one job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdEndJob {
    /// `TermCode=` as job status letter (`T`, `E`, `A` …).
    pub term_code: char,
    /// `ReadBytes=`: bytes the FD read from disk.
    pub read_bytes: u64,
    /// `JobBytes=`: bytes the FD sent to the SD.
    pub job_bytes: u64,
    /// `JobFiles=`.
    pub files: u64,
}

#[cfg_attr(doc, aquamarine::aquamarine)]
/// A running rig: directory, catalog and the three daemons.
///
/// ```mermaid
/// classDiagram
///     class Rig {
///         +Layout layout
///         +Ports ports
///         +Programs programs
///         +start(RigOptions) Rig
///         +run_job(name) u32
///         +wait_job(id) JobRow
///         +restore(jobid) u32
///         +diff_restore() (i32, String)
///     }
///     class Cluster { +query(sql) String }
///     class Daemon { +stop() }
///     class Console { +command(text) String }
///     Rig *-- Cluster : Katalog
///     Rig *-- "3" Daemon : odir, ofd, SD
///     Rig ..> Console : je Befehl
///     Rig *-- Layout
///     Rig *-- Ports
///     Rig *-- Programs
/// ```
#[derive(Debug)]
pub struct Rig {
    /// Paths of this rig.
    pub layout: Layout,
    /// Ports of this rig.
    pub ports: Ports,
    /// The programs in use.
    pub programs: Programs,
    /// Options it was started with.
    options: RigOptions,
    /// Director, FD, SD – in start order, stopped in reverse.
    daemons: Vec<Daemon>,
    /// The catalog cluster.
    catalog: Option<Cluster>,
}

impl Rig {
    /// Checks the prerequisites, creates the rig and starts everything.
    ///
    /// Panics with a clear message if a prerequisite is missing (T-9); any
    /// other failure is returned and the partial rig is cleaned up.
    pub fn start(options: RigOptions) -> Result<Rig, String> {
        let programs = match Programs::find_with_sd(&crate::prereq::target_profile_dir(), options.sd.clone()) {
            Ok(programs) => programs,
            Err(missing) => panic!("{missing}"),
        };
        let ports = Ports::from_base(options.base_port);
        for (what, port, host) in [
            ("Director", ports.dir, "127.0.0.1"),
            ("FD", ports.fd, "127.0.0.1"),
            ("SD", ports.sd, "127.0.0.1"),
            ("Katalog", ports.pg, crate::catalog::LISTEN),
        ] {
            if what == "FD" && !options.start_fd {
                continue;
            }
            let address: SocketAddr = format!("{host}:{port}").parse().map_err(|e| format!("{e}"))?;
            if !port_free(address) {
                return Err(format!("Port {port} ({what}) ist belegt: läuft noch ein Gerüst? (ss -ltnp)"));
            }
        }
        let root = PathBuf::from(format!("/tmp/ofdrig-{}-{}", options.base_port, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
        let layout = Layout::new(&root);
        let mut rig = Rig { layout, ports, programs, options, daemons: Vec::new(), catalog: None };
        rig.setup()?;
        Ok(rig)
    }

    /// Writes trees and configurations, starts the catalog and the daemons.
    fn setup(&mut self) -> Result<(), String> {
        let io = |e: std::io::Error| e.to_string();
        crate::tree::build(&self.layout.data).map_err(io)?;
        crate::tree::build_big(&self.layout.big).map_err(io)?;
        let cluster = Cluster::create(&self.layout.root, self.ports.pg, &self.programs)?;
        crate::config::write_all(&self.layout, self.ports, &cluster.user).map_err(io)?;
        if self.options.auditing {
            let audit = crate::config::auditing_resources(&self.layout, self.ports, &cluster.user);
            crate::config::write_director_resources(&self.layout, &audit).map_err(io)?;
        }
        if self.options.passive_client {
            let passive = crate::config::passive_client_resources(self.ports);
            crate::config::write_director_resources(&self.layout, &passive).map_err(io)?;
        }
        crate::config::write_director_resources(&self.layout, &self.options.extra_director).map_err(io)?;
        self.catalog = Some(cluster);

        let debug = self.options.debug_level.to_string();
        let local = |port: u16| SocketAddr::from(([127, 0, 0, 1], port));
        let sd_conf = self.layout.sd_conf.display().to_string();
        let mut sd = Daemon::spawn("sd", &self.programs.sd, &["-f", "-c", &sd_conf, "-d", "50"], &self.layout.root.join("sd.log"))?;
        let started = wait_port(&mut sd, local(self.ports.sd), Duration::from_secs(20));
        self.daemons.push(sd);
        started?;
        if self.options.start_fd {
            let fd_conf = self.layout.fd_conf.display().to_string();
            let mut fd = Daemon::spawn(
                "ofd",
                &self.programs.ofd,
                &["-f", "--disable-grpc", "-c", &fd_conf, "-d", &debug],
                &self.layout.root.join("fd.log"),
            )?;
            let started = wait_port(&mut fd, local(self.ports.fd), Duration::from_secs(20));
            self.daemons.push(fd);
            started?;
        }

        let dir_conf = self.layout.dir_conf.display().to_string();
        let mut dir = Daemon::spawn(
            "odir",
            &self.programs.odir,
            &["-f", "--disable-grpc", "-c", &dir_conf, "-d", &debug],
            &self.layout.root.join("dir.log"),
        )?;
        let started = wait_port(&mut dir, local(self.ports.dir), Duration::from_secs(30));
        self.daemons.push(dir);
        started
    }

    /// A new console session with the Director.
    pub fn console(&self) -> Result<Console, String> {
        Console::connect(SocketAddr::from(([127, 0, 0, 1], self.ports.dir)), CONSOLE_PASSWORD)
    }

    /// Sends one console command and returns the reply.
    pub fn command(&self, command: &str) -> Result<String, String> {
        self.console()?.command(command, Duration::from_secs(60))
    }

    /// The catalog cluster.
    pub fn catalog(&self) -> &Cluster {
        self.catalog.as_ref().expect("catalog runs after start")
    }

    /// Starts job `name` (`run job=… yes`) and returns its JobId.
    pub fn run_job(&self, name: &str) -> Result<u32, String> {
        let reply = self.command(&format!("run job={name} yes"))?;
        queued_job_id(&reply).ok_or_else(|| format!("run job={name}: keine JobId in der Antwort:\n{reply}"))
    }

    /// Starts the restore of all files of `jobid` into the restore directory.
    pub fn restore(&self, jobid: u32) -> Result<u32, String> {
        let command = format!(
            "restore jobid={jobid} client={CLIENT} where={} all done yes",
            self.layout.restore.display()
        );
        let reply = self.console()?.command(&command, Duration::from_secs(120))?;
        queued_job_id(&reply).ok_or_else(|| format!("{command}: keine JobId in der Antwort:\n{reply}"))
    }

    /// The catalog row of job `id`, if it exists.
    pub fn job_row(&self, id: u32) -> Result<Option<JobRow>, String> {
        let out = self.catalog().query(&format!(
            "SELECT JobId, Type, Level, JobStatus, JobFiles, JobBytes, JobErrors FROM Job WHERE JobId = {id}"
        ))?;
        let Some(line) = out.lines().next() else { return Ok(None) };
        let f: Vec<&str> = line.split('|').collect();
        let ch = |i: usize| f.get(i).and_then(|s| s.chars().next()).unwrap_or('?');
        let num = |i: usize| f.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok(Some(JobRow {
            id: num(0) as u32,
            kind: ch(1),
            level: ch(2),
            status: ch(3),
            files: num(4),
            bytes: num(5),
            errors: num(6),
        }))
    }

    /// Waits until job `id` ends (catalog), at most `timeout`.
    pub fn wait_job(&self, id: u32, timeout: Duration) -> Result<JobRow, String> {
        let deadline = Instant::now() + timeout;
        let mut last = None;
        while Instant::now() < deadline {
            if let Some(row) = self.job_row(id)? {
                if FINAL_STATES.contains(&row.status) {
                    return Ok(row);
                }
                last = Some(row);
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        Err(format!("Job {id} endet nicht in {timeout:?} (zuletzt {last:?})\n{}", self.diagnostics()))
    }

    /// Waits until job `id` runs (`R`), at most `timeout`.
    pub fn wait_running(&self, id: u32, timeout: Duration) -> Result<JobRow, String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(row) = self.job_row(id)?
                && (row.status == 'R' || FINAL_STATES.contains(&row.status))
            {
                return Ok(row);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(format!("Job {id} startet nicht in {timeout:?}\n{}", self.diagnostics()))
    }

    /// The job log of `id` from the catalog table `Log`.
    pub fn job_log(&self, id: u32) -> String {
        self.catalog()
            .query(&format!("SELECT LogText FROM Log WHERE JobId = {id} ORDER BY LogId"))
            .unwrap_or_else(|e| e)
    }

    /// The whole log file of the daemon `name` (`odir`, `ofd`, `sd`), or the
    /// Director's `job.log` (`job`) or audit log (`audit`, with
    /// [`RigOptions::auditing`]) from its working directory.
    pub fn daemon_log(&self, name: &str) -> String {
        let file = match name {
            "odir" => self.layout.root.join("dir.log"),
            "ofd" => self.layout.root.join("fd.log"),
            "job" => self.layout.dir_work.join("job.log"),
            "audit" => self.layout.dir_work.join("audit.log"),
            _ => self.layout.root.join("sd.log"),
        };
        std::fs::read_to_string(file).unwrap_or_default()
    }

    /// The `2800 End Job` report of the FD for JobId `id`, read from the ofd
    /// log (`job <id> (<Job>) ended: 2800 End Job TermCode=…`).
    pub fn fd_end_job(&self, id: u32) -> Option<FdEndJob> {
        let marker = format!("job {id} (");
        let log = self.daemon_log("ofd");
        let line = log.lines().find(|l| l.contains(&marker) && l.contains("2800 End Job"))?;
        let field = |key: &str| -> u64 {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(key))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        let code = field("TermCode=");
        Some(FdEndJob {
            term_code: char::from_u32(code as u32).unwrap_or('?'),
            read_bytes: field("ReadBytes="),
            job_bytes: field("JobBytes="),
            files: field("JobFiles="),
        })
    }

    /// `diff -r --no-dereference` of source and restored tree: (exit code, output).
    pub fn diff_restore(&self) -> (i32, String) {
        let restored = self.layout.restored_data();
        match Command::new(&self.programs.diff)
            .args(["-r", "--no-dereference"])
            .arg(&self.layout.data)
            .arg(&restored)
            .output()
        {
            Ok(out) => (
                out.status.code().unwrap_or(-1),
                format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
            ),
            Err(e) => (-1, e.to_string()),
        }
    }

    /// The end of every daemon log and the job table, for failure messages.
    pub fn diagnostics(&self) -> String {
        let mut text = format!("Gerüstverzeichnis: {} (ONESIMUS_RIG_KEEP=1 behält es)\n", self.layout.root.display());
        if let Some(cluster) = &self.catalog {
            text += &format!(
                "Jobs:\n{}\n",
                cluster
                    .query("SELECT JobId, Name, Type, Level, JobStatus, JobFiles, JobBytes, JobErrors FROM Job ORDER BY JobId")
                    .unwrap_or_else(|e| e)
            );
        }
        for daemon in &self.daemons {
            text += &format!("--- {} ({}) ---\n{}\n", daemon.name, daemon.log.display(), daemon.log_tail(15));
        }
        text
    }

    /// The rig root directory.
    pub fn root(&self) -> &Path {
        &self.layout.root
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        while let Some(mut daemon) = self.daemons.pop() {
            daemon.stop();
        }
        if let Some(mut cluster) = self.catalog.take() {
            cluster.stop();
        }
        if self.options.keep {
            eprintln!("Testgerüst behalten: {}", self.layout.root.display());
        } else {
            let _ = std::fs::remove_dir_all(&self.layout.root);
        }
    }
}

/// Builds `odir` and `ofd` of the workspace with the cargo that runs the test,
/// unless `ONESIMUS_RIG_NO_BUILD` is set (then the existing binaries are used).
pub fn build_workspace_binaries() -> Result<(), String> {
    build_workspace_packages(&["odir", "ofd"])
}

/// Builds the workspace `packages` (e.g. `osd-daemon` for the program `osd`)
/// with the cargo that runs the test, unless `ONESIMUS_RIG_NO_BUILD` is set.
pub fn build_workspace_packages(packages: &[&str]) -> Result<(), String> {
    if std::env::var_os("ONESIMUS_RIG_NO_BUILD").is_some() {
        return Ok(());
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut args = vec!["build"];
    for package in packages {
        args.extend(["-p", package]);
    }
    let shown = format!("cargo {}", args.join(" "));
    let status = Command::new(cargo)
        .current_dir(&workspace)
        .args(&args)
        .status()
        .map_err(|e| format!("{shown}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{shown}: {status}")) }
}
