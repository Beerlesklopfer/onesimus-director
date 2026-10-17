//! Job messages of the File Daemon (lib/message.cc Jmsg()/Qmsg()) and the
//! job status bookkeeping of the jcr they update.

use std::io::{Read, Write};

use bareos_bnet::BnetStream;

// lib/message_severity.h
pub const M_FATAL: i32 = 3;
pub const M_ERROR: i32 = 4;
pub const M_WARNING: i32 = 5;
pub const M_INFO: i32 = 6;
pub const M_SAVED: i32 = 7;
pub const M_SKIPPED: i32 = 9;
pub const M_RESTORED: i32 = 13;

// include/job_status.h
pub const JS_BLOCKED: u8 = b'B';
pub const JS_CANCELED: u8 = b'A';
pub const JS_ERROR_TERMINATED: u8 = b'E';
pub const JS_FATAL_ERROR: u8 = b'f';
pub const JS_RUNNING: u8 = b'R';
pub const JS_TERMINATED: u8 = b'T';
pub const JS_WARNINGS: u8 = b'W';
pub const JS_CREATED: u8 = b'C';

/// lib/jcr.cc GetStatusPriority()
fn status_priority(s: u8) -> i32 {
    match s {
        b'I' => 10,
        JS_ERROR_TERMINATED | JS_FATAL_ERROR | JS_CANCELED => 9,
        b'e' => 8,
        b'D' => 7,
        _ => 0,
    }
}

/// The counters and messages of one job (the FD side of the jcr).
pub struct JobMsgs {
    /// FD resource name (my_name).
    pub my_name: String,
    pub job_id: u32,
    /// Unique job name (jcr->Job), "*Director*" before the JobId command.
    pub job: String,
    status: u8,
    pub job_errors: u32,
    pub job_warnings: u32,
    /// Complete "Jmsg Job=..." lines not yet sent to the Director.
    queued: Vec<String>,
    /// Message types for the Director (Messages resource `director =`).
    pub director_types: u64,
    /// Messages resource `append = file = types`.
    pub append: Vec<(String, u64)>,
}

/// Without a Messages resource: all, !skipped, !restored, !saved.
pub const DEFAULT_DIRECTOR_TYPES: u64 = ((1u64 << 18) - 2) & !(1 << M_SKIPPED) & !(1 << M_RESTORED) & !(1 << M_SAVED);

impl JobMsgs {
    pub fn new(my_name: &str) -> Self {
        Self {
            my_name: my_name.to_string(),
            job_id: 0,
            job: "*Director*".to_string(),
            status: JS_CREATED,
            job_errors: 0,
            job_warnings: 0,
            queued: Vec::new(),
            director_types: DEFAULT_DIRECTOR_TYPES,
            append: Vec::new(),
        }
    }

    /// The Messages resource of the Client.
    pub fn with_messages(mut self, m: Option<&crate::config::MessagesRes>) -> Self {
        if let Some(m) = m {
            self.director_types = m.director_types;
            self.append = m.append.clone();
        }
        self
    }

    pub fn status(&self) -> u8 {
        self.status
    }

    /// setJobStatusWithPriorityCheck()
    pub fn set_status(&mut self, s: u8) {
        let old = status_priority(self.status);
        let new = status_priority(s);
        if new > old || (new == 0 && old == 0) {
            self.status = s;
        }
    }

    pub fn is_canceled(&self) -> bool {
        self.status == JS_CANCELED
    }

    /// Jmsg(jcr, type, 0, text): prefix, counters, and the message for the
    /// Director (types of the Messages resource) and the append files.
    pub fn jmsg(&mut self, msg_type: i32, text: &str) {
        let prefix = match msg_type {
            M_FATAL => {
                self.set_status(JS_FATAL_ERROR);
                if self.job_errors == 0 {
                    self.job_errors = 1;
                }
                format!("{} JobId {}: Fatal error: ", self.my_name, self.job_id)
            }
            M_ERROR => {
                self.job_errors += 1;
                format!("{} JobId {}: Error: ", self.my_name, self.job_id)
            }
            M_WARNING => {
                self.job_warnings += 1;
                format!("{} JobId {}: Warning: ", self.my_name, self.job_id)
            }
            _ => format!("{} JobId {}: ", self.my_name, self.job_id),
        };
        tracing::debug!("jmsg type={msg_type}: {}", text.trim_end());
        for (file, types) in &self.append {
            if types & (1u64 << msg_type) != 0 {
                use std::io::Write as _;
                let dt = chrono::Local::now().format("%d-%b %H:%M ");
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                    let _ = write!(f, "{dt}{prefix}{text}");
                }
            }
        }
        if !(0..64).contains(&msg_type) || self.director_types & (1u64 << msg_type) == 0 {
            return;
        }
        let mtime = chrono::Utc::now().timestamp();
        self.queued.push(format!("Jmsg Job={} type={msg_type} level={mtime} {prefix}{text}", self.job));
    }

    /// Send the queued messages to the Director.
    pub fn flush<S: Read + Write>(&mut self, dir: &mut BnetStream<S>) {
        for m in self.queued.drain(..) {
            if dir.send_str(&m).is_err() {
                break;
            }
        }
        self.queued.clear();
    }

    #[cfg(test)]
    pub fn take_queued(&mut self) -> Vec<String> {
        std::mem::take(&mut self.queued)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes_and_counters() {
        let mut j = JobMsgs::new("client-fd");
        j.job_id = 7;
        j.job = "RestoreFiles.2026-10-17_10.00.00_01".into();
        j.jmsg(M_ERROR, "boom\n");
        j.jmsg(M_WARNING, "hm\n");
        j.jmsg(M_RESTORED, "-rw-r--r-- ...\n");
        let q = j.take_queued();
        assert_eq!(q.len(), 2);
        assert!(q[0].starts_with("Jmsg Job=RestoreFiles.2026-10-17_10.00.00_01 type=4 level="));
        assert!(q[0].ends_with(" client-fd JobId 7: Error: boom\n"));
        assert_eq!((j.job_errors, j.job_warnings), (1, 1));
        j.set_status(JS_RUNNING);
        j.jmsg(M_FATAL, "x\n");
        assert_eq!(j.status(), JS_FATAL_ERROR);
        j.set_status(JS_TERMINATED);
        assert_eq!(j.status(), JS_FATAL_ERROR);
        j.set_status(JS_ERROR_TERMINATED);
        assert_eq!(j.status(), JS_FATAL_ERROR);
    }
}
