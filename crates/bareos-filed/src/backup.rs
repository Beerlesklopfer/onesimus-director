//! Backup protocol handler — FD side (filed/backup.cc). Not implemented
//! yet: `backup` ends the job with a fatal error instead of reporting a
//! successful empty backup, `estimate` answers with zero files.

use std::io::{Read, Write};

use bareos_bnet::{BnetSignal, BnetStream};

use crate::jmsg::M_FATAL;
use crate::job::Session;

/// BackupCmd(): "2000 OK backup", then the job fails.
pub fn backup<S: Read + Write>(session: &mut Session<'_>, dir: &mut BnetStream<S>) -> bool {
    session.send_reply(dir, "2000 OK backup\n");
    session.job_msg(M_FATAL, "Backup is not supported by this File Daemon yet.\n");
    false
}

/// EstimateCmd()
pub fn estimate<S: Read + Write>(session: &mut Session<'_>, dir: &mut BnetStream<S>) -> bool {
    session.send_reply(dir, "2000 OK estimate files=0 bytes=0\n");
    dir.send_signal(BnetSignal::Eod).is_ok()
}
