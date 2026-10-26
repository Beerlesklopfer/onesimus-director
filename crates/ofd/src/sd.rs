//! Storage daemon connections of a job: filed/dir_cmd.cc StorageCmd()
//! (the FD connects to the SD) and filed/sd_cmds.cc
//! handle_stored_connection() (passive client: the SD connects to the FD),
//! plus the table of running jobs (jcr list) both of them and `cancel` use.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use bareos_bnet::{BnetStream, CramMd5Auth, tls_psk};

/// Any byte stream a BNET connection can run over.
pub trait Rw: Read + Write + Send {}
impl<T: Read + Write + Send> Rw for T {}

pub type SdStream = BnetStream<Box<dyn Rw>>;

/// The part of a running job other threads see (passive SD connection,
/// cancel).
pub struct JobShared {
    pub job: String,
    pub sd_auth_key: Mutex<String>,
    /// Storage daemon connection delivered by a passive connect.
    passive_sd: Mutex<Option<SdStream>>,
    cv: Condvar,
    pub canceled: AtomicBool,
    /// Raw socket of the SD connection, to break a blocked read on cancel.
    pub sd_socket: Mutex<Option<TcpStream>>,
    /// Bandwidth limit of the SD connection (setbandwidth Job=).
    pub bw: Arc<BwState>,
}

/// BareosSocket bwlimit state (SetBwlimit, ControlBwlimit).
#[derive(Default)]
pub struct BwState {
    /// bytes per second, 0: no limit
    pub limit: AtomicU64,
    pub bursting: AtomicBool,
    /// (nb_bytes, last_tick in microseconds)
    ticks: Mutex<(i64, i64)>,
}

fn now_us() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

impl BwState {
    /// ControlBwlimit()
    pub fn control(&self, bytes: usize) {
        let limit = self.limit.load(Ordering::Relaxed);
        if limit == 0 || bytes == 0 {
            return;
        }
        let rate = limit as f64 / 1_000_000.0;
        let bursting = self.bursting.load(Ordering::Relaxed);
        let mut t = self.ticks.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_us();
        if t.1 == 0 {
            *t = (bytes as i64, now);
            return;
        }
        let temp = now - t.1;
        if temp < 100 {
            t.0 += bytes as i64;
            return;
        }
        t.0 += bytes as i64;
        t.1 = now;
        if temp > 10_000_000 {
            return;
        }
        t.0 -= (temp as f64 * rate) as i64;
        if t.0 < 0 {
            if !bursting {
                t.0 = 0;
            }
            return;
        }
        let mut usec_sleep = (t.0 as f64 / rate) as i64;
        if usec_sleep > 100 {
            loop {
                std::thread::sleep(Duration::from_micros(usec_sleep as u64));
                let now = now_us();
                if now - t.1 < usec_sleep {
                    usec_sleep -= now - t.1;
                    continue;
                }
                t.1 = now;
                break;
            }
            if bursting {
                t.0 -= (usec_sleep as f64 * rate) as i64;
            } else {
                t.0 = 0;
            }
        }
    }
}

/// A stream whose reads and writes are bandwidth limited.
pub struct Limited {
    pub inner: Box<dyn Rw>,
    pub bw: Arc<BwState>,
}

impl Read for Limited {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(b)?;
        self.bw.control(n);
        Ok(n)
    }
}

impl Write for Limited {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(b)?;
        self.bw.control(n);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

static JOBS: LazyLock<Mutex<HashMap<String, Arc<JobShared>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn register(job: &str, sd_auth_key: &str) -> Arc<JobShared> {
    let shared = Arc::new(JobShared {
        job: job.to_string(),
        sd_auth_key: Mutex::new(sd_auth_key.to_string()),
        passive_sd: Mutex::new(None),
        cv: Condvar::new(),
        canceled: AtomicBool::new(false),
        sd_socket: Mutex::new(None),
        bw: Arc::new(BwState::default()),
    });
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).insert(job.to_string(), shared.clone());
    shared
}

pub fn unregister(job: &str) {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).remove(job);
}

/// get_jcr_by_full_name()
pub fn find(job: &str) -> Option<Arc<JobShared>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).get(job).cloned()
}

/// The TLS-PSK key for the identity "R_JOB\x1e<Job>": the job's SD
/// authorization key (GetTlsPskByFullyQualifiedResourceName).
pub fn psk_for_job(job: &str) -> Option<Vec<u8>> {
    let shared = find(job)?;
    let key = shared.sd_auth_key.lock().unwrap_or_else(|e| e.into_inner()).clone();
    (!key.is_empty()).then(|| key.into_bytes())
}

impl JobShared {
    /// CancelCmd(): mark canceled and break the SD connection.
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::SeqCst);
        if let Some(s) = self.sd_socket.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            // SetTimedOut()/SetTerminated() + kTimeoutSignal: wake a blocked
            // read, but do not close the connection to the SD.
            let _ = s.shutdown(std::net::Shutdown::Read);
        }
        self.cv.notify_all();
    }

    /// Wait for the Storage daemon's passive connection.
    pub fn wait_passive_sd(&self, timeout: Duration) -> Option<SdStream> {
        let deadline = Instant::now() + timeout;
        let mut slot = self.passive_sd.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(s) = slot.take() {
                return Some(s);
            }
            if self.canceled.load(Ordering::SeqCst) {
                return None;
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            slot = self.cv.wait_timeout(slot, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
    }

    fn deliver_passive_sd(&self, sd: SdStream) {
        *self.passive_sd.lock().unwrap_or_else(|e| e.into_inner()) = Some(sd);
        self.cv.notify_all();
    }
}

/// The result of StorageCmd()'s connect: the stream and the job messages
/// it produces (Qmsg/Jmsg M_INFO).
pub struct Connected {
    pub sd: SdStream,
    pub infos: Vec<String>,
}

/// StorageCmd(): connect to the Storage daemon (retry every 10 s until
/// `timeout`), TLS-PSK first when the Director asked for TLS
/// (identity "R_JOB\x1e<Job>", key = SD authorization key), "Hello Start
/// Job <Job>" and CRAM-MD5 with the authorization key.
#[allow(clippy::too_many_arguments)]
pub fn connect_storage(
    addr: &str,
    port: u16,
    tls_policy: i32,
    job: &str,
    auth_key: &str,
    client_name: &str,
    timeout: Duration,
    shared: &JobShared,
) -> Result<Connected, String> {
    let deadline = Instant::now() + timeout;
    // A refused connection means nothing listens there (wrong address, e.g.
    // localhost seen from the client): give up after a few tries instead of
    // retrying until `timeout`.
    const MAX_REFUSED: u32 = 3;
    let mut refused = 0;
    let tcp = loop {
        let attempt = (addr, port).to_socket_addrs().map_err(|e| e.to_string()).and_then(|mut a| {
            let sa = a.next().ok_or_else(|| "no address".to_string())?;
            TcpStream::connect_timeout(&sa, Duration::from_secs(10)).map_err(|e| e.to_string())
        });
        match attempt {
            Ok(s) => break s,
            Err(e) => {
                refused += u32::from(e.to_ascii_lowercase().contains("connection refused"));
                if refused >= MAX_REFUSED {
                    return Err(format!("Unable to connect to Storage daemon on {addr}:{port}. ERR={e} \
                        (refused {refused} times; check the Storage Address as seen from this client, or set Client Passive = yes)"));
                }
                tracing::warn!("Could not connect to Storage daemon on {addr}:{port}. ERR={e} Retrying ...");
                if Instant::now() + Duration::from_secs(10) > deadline || shared.canceled.load(Ordering::SeqCst) {
                    return Err(e);
                }
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    };
    tcp.set_nodelay(true).ok();
    *shared.sd_socket.lock().unwrap_or_else(|e| e.into_inner()) = tcp.try_clone().ok();
    let mut infos = Vec::new();
    let (stream, cipher): (Box<dyn Rw>, Option<String>) = if tls_policy != 0 {
        let identity = format!("R_JOB\x1e{job}");
        let s = tls_psk::psk_connect(tcp, &identity, auth_key.as_bytes()).map_err(|e| format!("TLS: {e}"))?;
        let c = s.cipher_name();
        // TlsLogConninfo()
        infos.push(format!("Connected Storage daemon at {addr}:{port}, encryption: {}\n", if c.is_empty() { "Unknown" } else { &c }));
        (Box::new(s), Some(c))
    } else {
        (Box::new(tcp), None)
    };
    infos.push(format!("{}\n", tls_psk::cipher_message_string(cipher.as_deref())));
    let stream: Box<dyn Rw> = Box::new(Limited { inner: stream, bw: shared.bw.clone() });
    let mut sd: SdStream = BnetStream::new(stream);
    sd.send_str(&format!("Hello Start Job {job}\n")).map_err(|e| e.to_string())?;
    // CramMd5Handshake with the own qualified name and the local TLS policy
    // (TLS required: ssl=2)
    let auth = CramMd5Auth::new_raw(auth_key, &format!("R_CLIENT::{client_name}"), 2);
    let _ = tls_policy;
    match auth.two_way_authenticate(&mut sd, false) {
        Ok(r) if r.success => Ok(Connected { sd, infos }),
        _ => Err("auth".into()),
    }
}

/// handle_stored_connection(): "Hello Storage calling Start Job <Job>";
/// authenticate with the job's key and hand the connection to the job.
pub fn handle_stored_connection(mut sd: SdStream, hello: &str, client_name: &str) {
    let Some(job) = hello.trim_end().strip_prefix("Hello Storage calling Start Job ").map(|j| j.split_whitespace().next().unwrap_or("").to_string()) else {
        tracing::error!("Bad Hello command from Director: {hello}");
        return;
    };
    let Some(shared) = find(&job) else {
        tracing::error!("SD connect failed: Job name not found: {job}");
        return;
    };
    let key = shared.sd_auth_key.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let auth = CramMd5Auth::new_raw(&key, &format!("R_CLIENT::{client_name}"), 2);
    match auth.two_way_authenticate(&mut sd, true) {
        Ok(r) if r.success => {
            tracing::debug!("OK Authentication Job {job}");
            let inner = sd.into_inner();
            let sd: SdStream = BnetStream::new(Box::new(Limited { inner, bw: shared.bw.clone() }));
            shared.deliver_passive_sd(sd);
        }
        _ => tracing::error!("Unable to authenticate Storage daemon for job {job}"),
    }
}
