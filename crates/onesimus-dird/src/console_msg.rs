//! Console message queue (lib/message.cc InitConsoleMsg / `con_fd`,
//! `console_msg_pending`; dird/ua_output.cc DoMessages).
//!
//! Messages routed to a `console` destination of a Messages resource are
//! appended to one file, `<WorkingDirectory>/<Director name>.conmsg`, shared
//! by all consoles. Whoever fetches them first (`messages`, `.messages`,
//! autodisplay) gets them; the file is truncated afterwards. A non-empty file
//! left from an earlier run makes the messages pending again at startup.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::command_executor::ExecContext;
use crate::director_config::DirectorConfig;
use crate::message_bus::MsgType;

/// One console message file (C++ `con_fname`/`con_fd`/`console_msg_pending`).
pub struct ConsoleQueue {
    path: PathBuf,
    file: Mutex<Option<File>>,
    pending: AtomicBool,
}

impl ConsoleQueue {
    /// InitConsoleMsg(): create the file (0600); pending when not empty.
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).append(true).create(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&path)?;
        let pending = file.metadata().map(|m| m.len() > 0).unwrap_or(false);
        Ok(Self { path, file: Mutex::new(Some(file)), pending: AtomicBool::new(pending) })
    }

    /// `console_msg_pending`.
    pub fn pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    /// DispatchMessage() kConsole: date prefix, message, a newline when the
    /// message lacks one (an empty message is a lone newline).
    pub fn append(&self, dt: &str, msg: &str) {
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        if file.is_none() {
            // C++ reopens a lost con_fd with fopen(con_fname, "a+b")
            *file = OpenOptions::new().read(true).append(true).create(true).open(&self.path).ok();
        }
        let Some(file) = file.as_mut() else { return };
        let mut data = dt.as_bytes().to_vec();
        data.extend_from_slice(msg.as_bytes());
        if !msg.ends_with('\n') {
            data.push(b'\n');
        }
        let _ = file.write_all(&data).and_then(|_| file.flush());
        self.pending.store(true, Ordering::SeqCst);
    }

    /// DoMessages(): the queued messages as fgets() hands them out (one
    /// line, at most 1999 bytes, cut at a NUL byte per chunk); each chunk is
    /// one message to the console. The file is truncated when something was
    /// read, and nothing is pending afterwards.
    pub fn take(&self) -> Vec<Vec<u8>> {
        let mut guard = self.file.lock().unwrap_or_else(|e| e.into_inner());
        let mut chunks = Vec::new();
        if let Some(file) = guard.as_mut() {
            let mut data = Vec::new();
            if file.seek(SeekFrom::Start(0)).is_ok() && file.read_to_end(&mut data).is_ok() {
                chunks = fgets_chunks(&data);
            }
            if !chunks.is_empty() {
                let _ = file.set_len(0);
            }
        }
        self.pending.store(false, Ordering::SeqCst);
        chunks
    }
}

/// Split like repeated `fgets(msg, 2000, fp)` followed by `strlen(msg)`.
fn fgets_chunks(data: &[u8]) -> Vec<Vec<u8>> {
    const SIZE: usize = 2000;
    let mut chunks = Vec::new();
    let mut rest = data;
    while !rest.is_empty() {
        let limit = rest.len().min(SIZE - 1);
        let end = rest[..limit].iter().position(|&b| b == b'\n').map_or(limit, |p| p + 1);
        let chunk = &rest[..end];
        let len = chunk.iter().position(|&b| b == 0).unwrap_or(chunk.len());
        chunks.push(chunk[..len].to_vec());
        rest = &rest[end..];
    }
    chunks
}

static QUEUE: Mutex<Option<std::sync::Arc<ConsoleQueue>>> = Mutex::new(None);

fn queue() -> Option<std::sync::Arc<ConsoleQueue>> {
    #[cfg(test)]
    return TEST_QUEUE.with(|t| t.borrow().clone());
    #[cfg(not(test))]
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// InitConsoleMsg(working_directory) with `my_name` = the Director name.
pub fn init(working_directory: &Path, director_name: &str) {
    let path = working_directory.join(format!("{director_name}.conmsg"));
    match ConsoleQueue::open(path.clone()) {
        Ok(q) => *QUEUE.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(q)),
        Err(error) => tracing::error!("Could not open console message file {}: ERR={error}", path.display()),
    }
}

/// `console_msg_pending` of the Director.
pub fn pending() -> bool {
    queue().is_some_and(|q| q.pending())
}

/// Append one message to the console queue (no queue: dropped, as C++
/// without a console message file).
pub fn append(dt: &str, msg: &str) {
    if let Some(q) = queue() {
        q.append(dt, msg);
    }
}

/// Fetch and remove all queued messages (DoMessages).
pub fn take() -> Vec<Vec<u8>> {
    queue().map(|q| q.take()).unwrap_or_default()
}

/// DoMessages(ua): hand all queued messages to `send` (raw, one console
/// message each) and clear the session's "notified" flag.
pub fn do_messages(ctx: &ExecContext, send: impl FnOnce(&[Vec<u8>]) -> bool) -> bool {
    let messages = take();
    let sent = send(&messages);
    if let Some(manager) = &ctx.session_mgr {
        manager.set_session_notified(ctx.session_id, false);
    }
    sent
}

/// `ua->auto_display_messages` of the session.
pub fn auto_display(ctx: &ExecContext) -> bool {
    ctx.session_mgr.as_ref().is_some_and(|m| m.session_auto_display(ctx.session_id))
}

/// DotMessagesCmd(ua, cmd): only with pending messages, `cmd` permitted by
/// the Command ACL and autodisplay on.
pub fn dot_messages_cmd(ctx: &ExecContext, cmd: &str, send: impl FnOnce(&[Vec<u8>]) -> bool) -> bool {
    if pending() && crate::acl::access(ctx, crate::acl::Kind::Command, cmd) && auto_display(ctx) {
        return do_messages(ctx, send);
    }
    true
}

/// Jmsg() header for a job message (message.cc: "%s JobId %u: " plus the
/// type word for fatal errors, errors and warnings).
pub fn job_message_text(director: &str, job_id: u32, msg_type: MsgType, text: &str) -> String {
    let kind = match msg_type {
        MsgType::Fatal => "Fatal error: ",
        MsgType::Error => "Error: ",
        MsgType::Warning => "Warning: ",
        _ => "",
    };
    format!("{director} JobId {job_id}: {kind}{text}")
}

/// Jmsg(jcr, type, mtime, ...) of job `job_name`: the header of
/// [`job_message_text`] and DispatchMessage() through the job's Messages
/// resource (InitJobMsg). A JCR without one of its own — the console JCR
/// (JobId 0) or a job unknown to the configuration — uses the Director's
/// Messages resource (`daemon_msgs`).
pub fn job_message(config: &DirectorConfig, job_name: &str, job_id: u32, msg_type: MsgType, mtime: i64, text: &str) {
    // "Dequeue messages to keep the original order" (message.cc Jmsg).
    dequeue_messages(config, job_id);
    let msg = job_message_text(&config.director.name, job_id, msg_type, text);
    dispatch_job_message(config, job_name, job_id, crate::audit::code(msg_type), mtime, &msg);
}

/// A job message queued with [`qmsg`]: (job name, type, time, text).
type Queued = (String, MsgType, i64, String);

/// jcr->msg_queue of the jobs (Qmsg), per JobId.
static MSG_QUEUE: std::sync::Mutex<std::collections::BTreeMap<u32, Vec<Queued>>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Qmsg(jcr, type, mtime, ...): a message of a thread that may not send
/// it itself (the watchdog); it goes out before the job's next Director
/// message (Jmsg dequeues first) or with its end (UpdateJobEnd).
pub fn qmsg(bus: &crate::message_bus::MessageBus, job_id: u32, job_name: &str, msg_type: MsgType, text: String) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    MSG_QUEUE.lock().unwrap_or_else(|e| e.into_inner()).entry(job_id).or_default()
        .push((job_name.to_string(), msg_type, now, text.clone()));
    bus.post(crate::message_bus::BusMessage::JobMessage { job_id, job_name: job_name.to_string(), msg_type, text,
        timestamp: now });
}

/// DequeueMessages(jcr): dispatch the queued messages of JobId `job_id`.
pub fn dequeue_messages(config: &DirectorConfig, job_id: u32) {
    if job_id == 0 {
        return;
    }
    let queued = MSG_QUEUE.lock().unwrap_or_else(|e| e.into_inner()).remove(&job_id);
    for (job_name, msg_type, mtime, text) in queued.unwrap_or_default() {
        let msg = job_message_text(&config.director.name, job_id, msg_type, &text);
        dispatch_job_message(config, &job_name, job_id, crate::audit::code(msg_type), mtime, &msg);
    }
}

/// DispatchMessage(jcr, type, mtime, msg) of a complete message (a job
/// message the File or Storage daemon sent, getmsg.cc) for job `job_name`.
pub fn dispatch_job_message(config: &DirectorConfig, job_name: &str, job_id: u32, code: u32, mtime: i64, msg: &str) {
    // A Run directive's Messages override (scheduled_run.rs, S-15).
    let run_messages = crate::scheduled_run::job_messages(job_id);
    let job_messages = run_messages.as_deref().or_else(|| config.jobs.iter().find(|j| j.name == job_name && job_id != 0)
        .map(|j| j.messages.as_str())).filter(|m| !m.is_empty());
    let res = match job_messages {
        Some(name) => crate::audit::messages_resource(config, name),
        None => crate::audit::daemon_messages(config),
    };
    if let Some(res) = res {
        crate::audit::dispatch(res, code, mtime, msg, Some(job_id));
    }
}

/// Post a job message on the bus (gRPC push events) and dispatch it like
/// Jmsg() through the job's Messages resource; the bus message's
/// `timestamp` is the message time.
pub fn post_job_message(bus: &crate::message_bus::MessageBus, config: Option<&crate::director_config::SharedConfig>,
                        message: crate::message_bus::BusMessage) {
    if let crate::message_bus::BusMessage::JobMessage { job_id, job_name, msg_type, text, timestamp } = &message {
        if let Some(config) = config.and_then(|c| c.read().ok()) {
            job_message(&config, job_name, *job_id, *msg_type, *timestamp, text);
        }
    }
    bus.post(message);
}

#[cfg(test)]
thread_local! {
    /// Tests never use the Director-wide queue: each installs its own queue
    /// on the threads it runs (parallel tests stay independent).
    static TEST_QUEUE: std::cell::RefCell<Option<std::sync::Arc<ConsoleQueue>>> = const { std::cell::RefCell::new(None) };
}

/// A fresh queue file named after `name`, installed for the current thread.
#[cfg(test)]
pub fn test_queue(name: &str) -> std::sync::Arc<ConsoleQueue> {
    let dir = std::env::temp_dir().join(format!("onesimus-conmsg-{}-thread-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let q = std::sync::Arc::new(ConsoleQueue::open(dir.join("bareos-dir.conmsg")).unwrap());
    use_test_queue(q.clone());
    q
}

/// Install `q` as the queue of the current thread.
#[cfg(test)]
pub fn use_test_queue(q: std::sync::Arc<ConsoleQueue>) {
    TEST_QUEUE.with(|t| *t.borrow_mut() = Some(q));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("onesimus-conmsg-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("bareos-dir.conmsg")
    }

    #[test]
    fn messages_are_delivered_once_in_order() {
        let path = temp("order");
        let q = ConsoleQueue::open(path.clone()).unwrap();
        assert!(!q.pending());
        q.append("16-Oct 10:00 ", "bareos-dir JobId 1: first\n");
        q.append("16-Oct 10:01 ", "second");
        q.append("", "");
        assert!(q.pending());
        assert_eq!(q.take(), vec![
            b"16-Oct 10:00 bareos-dir JobId 1: first\n".to_vec(),
            b"16-Oct 10:01 second\n".to_vec(),
            b"\n".to_vec(),
        ]);
        assert!(!q.pending());
        assert!(q.take().is_empty());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn leftover_file_is_pending_at_startup() {
        let path = temp("leftover");
        std::fs::write(&path, "old\n").unwrap();
        let q = ConsoleQueue::open(path.clone()).unwrap();
        assert!(q.pending());
        q.append("", "new\n");
        assert_eq!(q.take(), vec![b"old\n".to_vec(), b"new\n".to_vec()]);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn long_lines_are_split_like_fgets() {
        let mut data = vec![b'x'; 2500];
        data.push(b'\n');
        data.extend_from_slice(b"a\0b\n");
        let chunks = fgets_chunks(&data);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 1999);
        assert_eq!(chunks[1].len(), 502);
        assert_eq!(chunks[2], b"a");
    }

    #[test]
    fn job_message_header_follows_jmsg() {
        assert_eq!(job_message_text("bareos-dir", 7, MsgType::Warning, "w\n"), "bareos-dir JobId 7: Warning: w\n");
        assert_eq!(job_message_text("bareos-dir", 7, MsgType::Info, "i\n"), "bareos-dir JobId 7: i\n");
    }
}
