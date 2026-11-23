//! Die Sitzung mit dem Director: Anmeldung und Befehle.
//!
//! Zweck: Ein Director ruft osd an, meldet sich mit CRAM-MD5 an und
//! schickt Befehle, eine Zeile je Befehl. Für einen Job: `JobId=…`
//! (Anmeldung, Antwort mit Sitzung und Schlüssel), `getSecureEraseCmd`,
//! `use storage=…`/`use device=…` (Schreiben mit `append=1`: Reservierung
//! erst mit den Daten, Just-In-Time; Lesen mit `append=0`: das Device wird
//! sofort reserviert), bei Restore und Verify `bootstrap` (vor oder nach
//! `use storage`), bei passivem Client `passive client …`, dann `run`
//! (wartet auf den FD, führt die Datenphase, meldet `3099`).
//! Ohne Job: `status`, `.status`, `label`, `mount`, `unmount`, `release`,
//! `cancel`, `setdebug`, `setbandwidth`. Ein unbekannter Befehl wird wie
//! bei bareos-sd mit `3900 Invalid command: …` beantwortet, danach endet
//! die Sitzung.
//!
//! Bareos-Gegenstück: `core/src/stored/dir_cmd.cc`
//! (`HandleDirectorConnection`, Befehlstabelle `cmds`, `LabelCmd`,
//! `MountCmd`, `UnmountCmd`, `ReleaseCmd`, `CancelCmd`, `SetdebugCmd`,
//! `SetbandwidthCmd`, `PassiveCmd`, `RunCmd`, `SecureerasereqCmd`),
//! `stored/authenticate.cc` (`AuthenticateDirector`), `stored/job.cc`
//! (`job_cmd`), `stored/reserve.cc` (`use_cmd`, `UseDeviceCmd`),
//! `stored/status.cc` (`StatusCmd`, `DotstatusCmd`), `stored/dir_cmd.cc`
//! (`BootstrapCmd`, `GetBootstrapFile`), `stored/reserve.cc`
//! (`ReserveDeviceForRead`).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bareos_bnet::{BnetMessage, BnetSignal};
use bareos_format::bash::{bash_spaces, unbash_spaces};
use bareos_format::msg::{M_FATAL, M_INFO};
use daemon_crate::accept::AnyBnet;
use daemon_crate::handshake::{authenticate_director, DirectorAuthError, HelloReplies};

use crate::daemon::Daemon;
use crate::device::{FileDevice, LabelState};
use crate::jobs::{Job, JobCommand, UseStorage};
use crate::transport::{lock, text, SharedChannel};

/// `OK_hello` of `stored/authenticate.cc`.
const OK_HELLO: &str = "3000 OK Hello\n";
/// `Dir_sorry`.
const DIR_SORRY: &str = "3999 No go\n";

/// What a command handler wants next.
enum Next {
    /// Read the next command.
    Continue,
    /// End the session (`quit`).
    Quit,
}

/// One Director connection after the Hello was read.
pub fn handle(d: Daemon, mut bnet: AnyBnet, hello: &str, session: u64) {
    let own = format!("R_STORAGE::{}", d.name());
    let replies = HelloReplies { ok: OK_HELLO, sorry: DIR_SORRY };
    let name = match authenticate_director(&mut bnet, hello, &own, replies, |n| d.config.find_director(n).map(|r| r.password.clone())) {
        Ok(name) => name,
        Err(DirectorAuthError::BadHello) => {
            tracing::warn!("session {session}: Bad Hello command from Director: {}", hello.trim_end());
            return;
        }
        Err(DirectorAuthError::UnknownDirector(n)) => {
            tracing::warn!("session {session}: Connection from unknown Director {n} rejected.");
            return;
        }
        Err(DirectorAuthError::AuthFailed(n)) => {
            tracing::warn!("session {session}: Unable to authenticate Director \"{n}\".");
            std::thread::sleep(Duration::from_secs(5));
            return;
        }
        Err(DirectorAuthError::SendFailed(_)) => return,
    };
    let monitor = d.config.find_director(&name).is_some_and(|r| r.monitor);
    tracing::debug!("session {session}: Director {name} authenticated");
    let dir: SharedChannel = Arc::new(Mutex::new(Box::new(bnet)));
    let mut s = DirSession { d, dir, monitor, job: None };
    s.run();
}

/// The command loop of one Director connection (its JCR).
struct DirSession {
    /// The daemon.
    d: Daemon,
    /// The Director connection.
    dir: SharedChannel,
    /// `Monitor = yes`: only status commands.
    monitor: bool,
    /// The job this connection set up with `JobId=`.
    job: Option<Arc<Job>>,
}

/// Command names in the order of `dir_cmd.cc` `cmds` and whether a monitor
/// may use them.
const COMMANDS: &[(&str, bool)] = &[
    ("autochanger", false),
    ("bootstrap", false),
    ("cancel", false),
    ("finish", false),
    ("JobId=", false),
    ("label", false),
    ("listen", false),
    ("mount", false),
    ("nextrun", false),
    ("passive", false),
    ("pluginoptions", false),
    ("readlabel", false),
    ("relabel", false),
    ("release", false),
    ("resolve", false),
    ("replicate", false),
    ("run", false),
    ("getSecureEraseCmd", false),
    ("setbandwidth=", false),
    ("setdebug=", false),
    ("setdevice", false),
    ("stats", false),
    ("status", true),
    (".status", true),
    ("unmount", false),
    ("use storage=", false),
];

impl DirSession {
    /// Sends `text` to the Director.
    fn send(&self, text: &str) {
        let _ = lock(&self.dir).send_str(text);
    }

    /// Sends an EOD signal.
    fn eod(&self) {
        let _ = lock(&self.dir).signal(BnetSignal::Eod);
    }

    /// `HandleDirectorConnection`'s loop.
    fn run(&mut self) {
        loop {
            let msg = match lock(&self.dir).recv() {
                Ok(BnetMessage::Data(data)) => text(&data),
                _ => break,
            };
            tracing::debug!("<dird: {}", msg.trim_end());
            let Some(&(cmd, monitor_ok)) = COMMANDS.iter().find(|(c, _)| msg.starts_with(c)) else {
                self.send(&format!("3900 Invalid command: {msg}\n"));
                break;
            };
            if self.monitor && !monitor_ok {
                self.send("3997 Invalid command for a Director with Monitor directive enabled.\n");
                self.eod();
                continue;
            }
            let next = match cmd {
                "JobId=" => self.job_cmd(&msg),
                "getSecureEraseCmd" => {
                    self.send("2000 OK SDSecureEraseCmd *None* \n");
                    Next::Continue
                }
                "use storage=" => self.use_cmd(&msg),
                "run" => self.run_cmd(),
                "listen" => self.listen_cmd(),
                "passive" => self.passive_cmd(&msg),
                "status" => self.status_cmd(&msg),
                ".status" => self.dot_status_cmd(&msg),
                "label" => self.label_cmd(&msg),
                "relabel" => {
                    self.send("3912 Failed to label Volume: ERR=relabel is not supported by osd yet\n");
                    self.eod();
                    Next::Continue
                }
                "mount" => self.device_cmd(&msg, "mount"),
                "unmount" => self.device_cmd(&msg, "unmount"),
                "release" => self.device_cmd(&msg, "release"),
                "cancel" => self.cancel_cmd(&msg),
                "setdebug=" => self.setdebug_cmd(&msg),
                "setbandwidth=" => self.setbandwidth_cmd(&msg),
                "pluginoptions" => {
                    self.send("2000 OK plugin options\n");
                    Next::Continue
                }
                "bootstrap" => self.bootstrap_cmd(),
                _ => {
                    // Known to bareos-sd, not implemented in osd yet.
                    self.send(&format!("3900 Invalid command: {msg}\n"));
                    Next::Quit
                }
            };
            if matches!(next, Next::Quit) {
                break;
            }
        }
        let _ = lock(&self.dir).signal(BnetSignal::Terminate);
        if let Some(job) = self.job.take() {
            if let Some(dev) = self.d.reserved_device(&job.cmd.job) {
                self.d.release_device(&dev);
            }
            if let Some(b) = job.bootstrap.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = std::fs::remove_file(b.path);
            }
            self.d.jobs.remove(&job.cmd.job);
        }
    }

    /// `job_cmd`.
    fn job_cmd(&mut self, msg: &str) -> Next {
        let Some(cmd) = JobCommand::parse(msg) else {
            self.send(&format!("3915 Bad Job command. stat=0 CMD: {msg}\n"));
            return Next::Quit;
        };
        let job = self.d.jobs.create(cmd, self.d.start_time);
        self.send(&format!(
            "3000 OK Job SDid={} SDtime={} Authorization={}\n",
            job.vol_session_id, job.vol_session_time, job.auth_key
        ));
        self.job = Some(job);
        Next::Continue
    }

    /// `use_cmd` / `UseDeviceCmd`.
    fn use_cmd(&mut self, first: &str) -> Next {
        let Some(job) = self.job.clone() else {
            self.send(&format!("3913 Bad use command: {first}\n"));
            return Next::Quit;
        };
        let mut storages: Vec<UseStorage> = Vec::new();
        let mut line = first.to_string();
        let parsed = loop {
            let Some(mut st) = parse_use_storage(&line) else { break Err(line) };
            // the devices up to an EOD
            let mut bad = None;
            while let Ok(BnetMessage::Data(data)) = lock(&self.dir).recv() {
                let l = text(&data);
                match l.trim_end().strip_prefix("use device=").and_then(|n| n.split_whitespace().next()) {
                    Some(dev) => st.devices.push(unbash_spaces(dev)),
                    None => {
                        bad = Some(l);
                        break;
                    }
                }
            }
            storages.push(st);
            if let Some(l) = bad {
                break Err(l);
            }
            match lock(&self.dir).recv() {
                Ok(BnetMessage::Data(data)) => line = text(&data),
                _ => break Ok(()),
            }
        };
        if let Err(l) = parsed {
            let l = unbash_spaces(&l);
            self.d.jmsg(&self.dir, &job, M_FATAL, &format!("Failed command: {l}\n"));
            self.send(&format!("3913 Bad use command: {l}\n"));
            job.set_status('E');
            return Next::Quit;
        }
        let Some(storage) = storages.iter().find(|s| s.append).cloned() else {
            return self.use_for_read(&job, storages);
        };
        let device = if self.d.config.storage.just_in_time_reservation {
            self.d.jmsg(&self.dir, &job, M_INFO, &format!("Using just in time reservation for job {}\n", job.cmd.job_id));
            Some("JustInTime Device".to_string())
        } else {
            // reserved now, freed at the end of the session; the append
            // reserves again from the same list
            let r = self.d.reserve_device(&storage, &job.cmd.job).map(|r| r.device.name);
            if let Some(name) = &r {
                self.d.release_device(name);
            }
            r
        };
        let wanted = storage.devices.first().cloned().unwrap_or_default();
        *job.storage.lock().unwrap_or_else(|e| e.into_inner()) = Some(storage);
        match device {
            Some(name) => {
                self.send(&format!("3000 OK use device device={}\n", bash_spaces(&name)));
                Next::Continue
            }
            None => {
                self.d.jmsg(&self.dir, &job, M_FATAL, &format!("Device reservation failed for JobId={}: no suitable device\n", job.cmd.job_id));
                self.send(&format!("3924 Device \"{wanted}\" not in SD Device resources or no matching Media Type.\n"));
                job.set_status('E');
                Next::Quit
            }
        }
    }

    /// `TryReserveAfterUse(jcr, false)` → `ReserveDeviceForRead`: a read
    /// job (restore, verify) reserves a device now; there is no
    /// just-in-time reservation for reading.
    fn use_for_read(&mut self, job: &Arc<Job>, storages: Vec<UseStorage>) -> Next {
        let reserved = storages.iter().find_map(|st| self.d.reserve_device(st, &job.cmd.job).map(|r| (st.clone(), r)));
        match reserved {
            Some((storage, r)) => {
                *job.storage.lock().unwrap_or_else(|e| e.into_inner()) = Some(storage);
                self.send(&format!("3000 OK use device device={}\n", bash_spaces(&r.device.name)));
                Next::Continue
            }
            None => {
                let wanted = storages.first().and_then(|s| s.devices.first()).cloned().unwrap_or_default();
                let reason = match storages.first().and_then(|s| s.devices.first()).and_then(|n| self.d.device_user(n)) {
                    Some(_) => format!("3602 JobId={} device {wanted} is busy (already reading/writing).", job.cmd.job_id),
                    None => format!("3924 Device \"{wanted}\" not in SD Device resources or no matching Media Type."),
                };
                self.d.jmsg(&self.dir, job, M_FATAL, &format!("Device reservation failed for JobId={}: {reason}\n", job.cmd.job_id));
                self.send(&format!("3924 Device \"{wanted}\" not in SD Device resources or no matching Media Type.\n"));
                job.set_status('E');
                Next::Quit
            }
        }
    }

    /// `RunCmd` → `DoJobRun`: waits for the FD, runs the job, ends the session.
    fn run_cmd(&mut self) -> Next {
        let Some(job) = self.job.clone() else { return Next::Quit };
        self.d.send_status(&self.dir, &job, 'F');
        let wait = Duration::from_secs(match self.d.config.storage.client_connect_wait {
            0 => 1800,
            n => n,
        });
        if job.wait_fd(wait) && !job.canceled.load(Ordering::Acquire) {
            self.d.jobs_run.fetch_add(1, Ordering::AcqRel);
            crate::fd_session::run_job(&self.d, &job, &self.dir);
        }
        Next::Quit
    }

    /// `ListenCmd` → `DoListenRun`: the writing SD of a Copy/Migrate waits
    /// for the reading SD (it calls with `Hello Start Storage Job` and the
    /// job's key after the Director sent it `replicate`) without a time
    /// limit, then takes its records like an FD's. The Director expects no
    /// answer to `listen`; the job ends with `3099`.
    fn listen_cmd(&mut self) -> Next {
        let Some(job) = self.job.clone() else { return Next::Quit };
        self.d.send_status(&self.dir, &job, 'S');
        if job.wait_fd(Duration::from_secs(30 * 24 * 3600)) && !job.canceled.load(Ordering::Acquire) {
            self.d.jobs_run.fetch_add(1, Ordering::AcqRel);
            crate::fd_session::run_job(&self.d, &job, &self.dir);
        } else {
            // authentication failed or canceled: the job ends without data
            if !job.canceled.load(Ordering::Acquire) {
                job.set_status('E');
            }
            let mut dir = lock(&self.dir);
            let _ = dir.send_str(&format!("3099 Job {} end JobStatus={} JobFiles=0 JobBytes=0 JobErrors=0\n", job.cmd.job, job.status() as u32));
            let _ = dir.signal(BnetSignal::Eod);
        }
        Next::Quit
    }

    /// `PassiveCmd`: osd connects to the FD (over gRPC when the command
    /// carries the FD's gRPC target, else BNET) and authenticates with the
    /// job's key; then `2000 OK passive client`.
    fn passive_cmd(&mut self, msg: &str) -> Next {
        let Some(job) = self.job.clone() else { return Next::Quit };
        let Some((addr, port, ssl)) = parse_passive(msg) else {
            self.d.jmsg(&self.dir, &job, M_FATAL, &format!("Bad passiveclientcmd command: {msg}"));
            return Next::Quit;
        };
        // odir appends the FD's gRPC target when the FD offers gRPC.
        let grpc = onesimus_bnet_grpc::Target::from_tokens(&addr, msg);
        match crate::server::connect_passive_fd(&self.d, &job, &addr, port, ssl, grpc.as_ref()) {
            Ok((conn, _transport)) => {
                job.connect_fd(conn);
                self.send("2000 OK passive client\n");
                Next::Continue
            }
            Err(e) => {
                tracing::warn!("{}: {e}", job.cmd.job);
                self.d.jmsg(&self.dir, &job, M_FATAL, &format!("Failed to connect to File daemon: {addr}:{port}\n"));
                Next::Quit
            }
        }
    }

    /// `status <devices>`.
    fn status_cmd(&mut self, msg: &str) -> Next {
        if msg.split_whitespace().nth(1).is_none() {
            self.send(&format!("3900 No arg in status command: {msg}\n"));
            self.eod();
            return Next::Quit;
        }
        self.send("\n");
        for line in crate::status::status_text(&self.d) {
            self.send(&line);
        }
        self.eod();
        Next::Continue
    }

    /// `.status <what>`.
    fn dot_status_cmd(&mut self, msg: &str) -> Next {
        let Some(what) = msg.split_whitespace().nth(1) else {
            self.send(&format!("3900 No arg in .status command: {msg}\n"));
            self.eod();
            return Next::Quit;
        };
        match crate::status::dot_status(&self.d, what) {
            Some(lines) => {
                for line in lines {
                    self.send(&line);
                }
                self.eod();
                Next::Continue
            }
            None => {
                self.send(&format!("3900 Unknown arg in .status command: {msg}\n"));
                self.eod();
                Next::Quit
            }
        }
    }

    /// `LabelCmd` / `LabelVolumeIfOk` for a File device.
    fn label_cmd(&mut self, msg: &str) -> Next {
        let reply = match parse_label(msg) {
            None => format!("3903 Error scanning label command: {msg}\n"),
            Some(l) => match self.d.config.find_device(&l.device) {
                None => format!("3999 Device \"{}\" not found or could not be opened.\n", l.device),
                // a job holds the device: refused unless it waits for the
                // operator (Bareos labels a BST_WAITING_FOR_SYSOP device)
                Some(_) if self.d.device_user(&l.device).is_some() && !self.d.waiting_for_sysop(&l.device) => {
                    let dev = self.d.config.find_device(&l.device).map(FileDevice::from_config);
                    format!("3937 Device {} is busy with writers=1 reserved=0.\n", dev.map(|d| d.print_name()).unwrap_or_default())
                }
                Some(conf) => {
                    let dev = FileDevice::from_config(conf);
                    let reply = label_volume(&self.d, &dev, &l.volume, &l.pool);
                    if reply.starts_with("3000 ") {
                        self.d.media_event(&dev.name);
                    }
                    reply
                }
            },
        };
        self.send(&reply);
        self.eod();
        Next::Continue
    }

    /// `mount`, `unmount`, `release` for a File device.
    fn device_cmd(&mut self, msg: &str, what: &str) -> Next {
        let name = msg.split_whitespace().nth(1).map(unbash_spaces);
        let reply = match name.as_deref().and_then(|n| self.d.config.find_device(n)) {
            None if name.is_none() => match what {
                "mount" => format!("3909 Error scanning mount command: {msg}\n"),
                "unmount" => format!("3907 Error scanning unmount command: {msg}\n"),
                _ => format!("3927 Error scanning release command: {msg}\n"),
            },
            None => format!("3999 Device \"{}\" not found or could not be opened.\n", name.unwrap_or_default()),
            Some(conf) => {
                let dev = FileDevice::from_config(conf);
                let busy = self.d.device_user(&dev.name).is_some();
                match (what, busy) {
                    ("mount", _) => {
                        self.d.media_event(&dev.name);
                        format!("3906 File device {} is always mounted.\n", dev.print_name())
                    }
                    (_, true) => format!("3937 Device {} is busy with writers=1 reserved=0.\n", dev.print_name()),
                    ("unmount", false) => format!("3901 Device \"{}\" is already unmounted.\n", dev.print_name()),
                    _ => format!("3921 Device \"{}\" already released.\n", dev.print_name()),
                }
            }
        };
        self.send(&reply);
        self.eod();
        Next::Continue
    }

    /// `CancelCmd`.
    fn cancel_cmd(&mut self, msg: &str) -> Next {
        match msg.trim_end().strip_prefix("cancel Job=").and_then(|j| j.split_whitespace().next()) {
            None => self.send("3903 Error scanning cancel command.\n"),
            Some(name) => {
                let job = match name.parse::<u32>() {
                    Ok(id) => self.d.jobs.find_id(id),
                    Err(_) => self.d.jobs.find(name),
                };
                match job {
                    None => self.send(&format!("3904 Job {name} not found.\n")),
                    Some(job) => {
                        job.cancel();
                        self.send(&format!("3000 JobId={} Job=\"{}\" marked to be canceled.\n", job.cmd.job_id, job.cmd.job));
                    }
                }
            }
        }
        self.eod();
        Next::Continue
    }

    /// `SetdebugCmd`.
    fn setdebug_cmd(&mut self, msg: &str) -> Next {
        let num = |key: &str| msg.split_whitespace().find_map(|w| w.strip_prefix(key)).and_then(|v| v.parse::<i64>().ok());
        let (Some(level), Some(trace)) = (num("setdebug="), num("trace=")) else {
            self.send(&format!("3991 Bad setdebug command: {msg}\n"));
            return Next::Quit;
        };
        let tracefile = format!(
            "{}/{}.trace",
            self.d.config.storage.working_directory.as_deref().unwrap_or("/var/lib/bareos"),
            self.d.name()
        );
        match num("timestamp=") {
            Some(ts) => self.send(&format!("3000 OK setdebug={level} trace={trace} timestamp={ts} tracefile={tracefile}\n")),
            None => self.send(&format!("3000 OK setdebug={level} tracefile={tracefile}\n")),
        }
        Next::Continue
    }

    /// `SetbandwidthCmd` (accepted; osd does not throttle FD→SD yet).
    fn setbandwidth_cmd(&mut self, msg: &str) -> Next {
        let bw = msg.trim_start_matches("setbandwidth=").split_whitespace().next().and_then(|v| v.parse::<i64>().ok());
        match bw {
            Some(bw) if bw >= 0 => {
                self.send("2000 OK Bandwidth\n");
                Next::Continue
            }
            _ => {
                self.send(&format!("2991 Bad setbandwidth command: {msg}\n"));
                Next::Quit
            }
        }
    }

    /// `BootstrapCmd` → `GetBootstrapFile`: receives the bootstrap up to
    /// the EOD, writes it to `<Working Directory>/<Storage>.<Job>.<n>.bootstrap`
    /// and parses it (`parse_bsr`); `3000 OK bootstrap` or `3904 Error
    /// bootstrap`. bareos-sd erases the file right after parsing; osd keeps
    /// it until the job ends because the device reader (own process) reads
    /// it again.
    fn bootstrap_cmd(&mut self) -> Next {
        let mut text = String::new();
        while let Ok(BnetMessage::Data(data)) = lock(&self.dir).recv() {
            text.push_str(&String::from_utf8_lossy(&data));
        }
        let Some(job) = self.job.clone() else {
            self.send("3904 Error bootstrap\n");
            return Next::Quit;
        };
        let n = BOOTSTRAP_UNIQ.fetch_add(1, Ordering::AcqRel) + 1;
        let work = self.d.config.storage.working_directory.as_deref().unwrap_or("/var/lib/bareos");
        let path = std::path::PathBuf::from(format!("{work}/{}.{}.{n}.bootstrap", self.d.name(), job.cmd.job));
        if let Err(e) = std::fs::write(&path, &text) {
            self.d.jmsg(&self.dir, &job, M_FATAL, &format!("Could not create bootstrap file {}: ERR={e}\n", path.display()));
            self.send("3904 Error bootstrap\n");
            return Next::Quit;
        }
        match bareos_format::bsr::Bsr::parse(&text) {
            Ok(bsr) => {
                tracing::debug!("{}: bootstrap {} with {} section(s)", job.cmd.job, path.display(), bsr.entries.len());
                let old = job.bootstrap.lock().unwrap_or_else(|e| e.into_inner()).replace(crate::jobs::Bootstrap { path, bsr });
                if let Some(old) = old {
                    let _ = std::fs::remove_file(old.path);
                }
                self.send("3000 OK bootstrap\n");
                Next::Continue
            }
            Err(e) => {
                self.d.jmsg(&self.dir, &job, M_FATAL, &format!("{e}, col 0 of file {}\n", path.display()));
                self.d.jmsg(&self.dir, &job, M_FATAL, "Error parsing bootstrap file.\n");
                let _ = std::fs::remove_file(&path);
                self.send("3904 Error bootstrap\n");
                Next::Quit
            }
        }
    }
}

/// `bsr_uniq`: numbers the bootstrap files of this daemon.
static BOOTSTRAP_UNIQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Labels `volume` on `dev` unless it already carries a label
/// (`LabelVolumeIfOk`); the reply line for the Director.
pub fn label_volume(d: &Daemon, dev: &FileDevice, volume: &str, pool: &str) -> String {
    match dev.read_label(volume) {
        LabelState::Labeled(l) => format!("3920 Cannot label Volume because it is already labeled: \"{}\"\n", l.volume_name),
        LabelState::NoLabel | LabelState::Broken(_) => {
            let label = crate::device::new_label(volume, pool, &dev.media_type, d.name());
            match dev.write_label(&label, 0, 0) {
                Ok(bytes) => format!(
                    "3000 OK label. VolFiles=0 VolBytes={bytes} Volume=\"{volume}\" Device={}\n",
                    dev.print_name()
                ),
                Err(e) => format!("3912 Failed to label Volume: ERR={e}\n"),
            }
        }
    }
}

/// `use storage=%127s media_type=%127s pool_name=%127s pool_type=%127s
/// append=%d copy=%d stripe=%d`.
pub fn parse_use_storage(line: &str) -> Option<UseStorage> {
    let keys = ["use storage=", "media_type=", "pool_name=", "pool_type=", "append=", "copy=", "stripe="];
    let mut rest = line.trim_end();
    let mut v = Vec::new();
    for k in keys {
        rest = rest.trim_start().strip_prefix(k)?;
        let end = rest.find(' ').unwrap_or(rest.len());
        v.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    let int = |i: usize| v[i].parse::<i32>().ok();
    int(5)?;
    int(6)?;
    Some(UseStorage {
        storage: unbash_spaces(&v[0]),
        media_type: unbash_spaces(&v[1]),
        pool_name: unbash_spaces(&v[2]),
        pool_type: unbash_spaces(&v[3]),
        append: int(4)? != 0,
        devices: Vec::new(),
    })
}

/// `passive client address=%s port=%d ssl=%d`.
pub fn parse_passive(msg: &str) -> Option<(String, u16, i32)> {
    let rest = msg.trim_end().strip_prefix("passive client ")?;
    let field = |k: &str| rest.split_whitespace().find_map(|w| w.strip_prefix(k)).map(str::to_string);
    Some((field("address=")?, field("port=")?.parse().ok()?, field("ssl=")?.parse().ok()?))
}

/// The fields of a `label` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelCommand {
    /// Device name.
    pub device: String,
    /// `VolumeName=`.
    pub volume: String,
    /// `PoolName=`.
    pub pool: String,
    /// `MediaType=`.
    pub media_type: String,
    /// `Slot=`.
    pub slot: i32,
}

/// `label %127s VolumeName=%127s PoolName=%127s MediaType=%127s Slot=%hd
/// drive=%hd MinBlocksize=%d MaxBlocksize=%d`.
pub fn parse_label(msg: &str) -> Option<LabelCommand> {
    let w: Vec<&str> = msg.split_whitespace().collect();
    if w.len() < 9 || w[0] != "label" {
        return None;
    }
    let get = |i: usize, k: &str| w[i].strip_prefix(k).map(unbash_spaces);
    let cmd = LabelCommand {
        device: unbash_spaces(w[1]),
        volume: get(2, "VolumeName=")?,
        pool: get(3, "PoolName=")?,
        media_type: get(4, "MediaType=")?,
        slot: get(5, "Slot=")?.parse().ok()?,
    };
    get(6, "drive=")?.parse::<i32>().ok()?;
    get(7, "MinBlocksize=")?.parse::<i64>().ok()?;
    get(8, "MaxBlocksize=")?.parse::<i64>().ok()?;
    Some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_storage_is_scanned_like_use_device_cmd() {
        let s = parse_use_storage("use storage=vsd media_type=V\u{1}File pool_name=VPool pool_type=Backup append=1 copy=0 stripe=0\n").unwrap();
        assert_eq!((s.storage.as_str(), s.media_type.as_str(), s.pool_name.as_str(), s.append), ("vsd", "V File", "VPool", true));
        assert_eq!(parse_use_storage("use storage=x"), None);
    }

    #[test]
    fn passive_and_label_commands() {
        assert_eq!(parse_passive("passive client address=127.0.0.1 port=39482 ssl=4\n"), Some(("127.0.0.1".into(), 39482, 4)));
        let l = parse_label("label VDev VolumeName=Vol1 PoolName=VPool MediaType=VFile Slot=0 drive=0 MinBlocksize=0 MaxBlocksize=0").unwrap();
        assert_eq!((l.device.as_str(), l.volume.as_str(), l.pool.as_str(), l.media_type.as_str()), ("VDev", "Vol1", "VPool", "VFile"));
        assert_eq!(parse_label("label VDev"), None);
    }

    #[test]
    fn commands_match_first_in_table_order() {
        let find = |m: &str| COMMANDS.iter().find(|(c, _)| m.starts_with(c)).map(|(c, _)| *c);
        assert_eq!(find("release VDev drive=0"), Some("release"));
        assert_eq!(find("relabel VDev OldName=a"), Some("relabel"));
        assert_eq!(find(".status header"), Some(".status"));
        assert_eq!(find("use storage=x"), Some("use storage="));
        assert_eq!(find("frobnicate"), None);
    }
}
