// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Meldungsarten (`M_*`): Schwere und Ziel einer Job- oder Daemon-Meldung.
//!
//! Zweck: Jede Meldung, die ein Daemon an den Director schickt
//! (`Jmsg Job=… type=<M_*> level=…`), trägt eine Meldungsart. Sie entscheidet,
//! ob die Meldung den Job-Status ändert (`M_FATAL`, `M_ERROR`) und in
//! welche Ziele der `Messages`-Ressource sie geht (`all, !skipped, …`).
//!
//! Datenfluss: FD/SD `Jmsg` → Director `job_messages` → Logdateien,
//! Katalog-Tabelle `Log`, Konsole.
//!
//! Bareos-Gegenstück: `core/src/lib/message_severity.h`.
//!
//! Zusammengeführt (19.11.2026) aus `ofd/src/jmsg.rs`, `ofd/src/backup.rs`
//! (`M_NOTSAVED`) und den fünf Kopien in odir (`verify.rs`, `sd_copy.rs`,
//! `vbackup.rs`, `backup_worker.rs`, `job_worker.rs`), die die Werte als
//! `u32` führten.

/// Abort: the daemon ends with a core dump.
pub const M_ABORT: i32 = 1;
/// Debug output.
pub const M_DEBUG: i32 = 2;
/// Fatal error: the job ends with status `f`.
pub const M_FATAL: i32 = 3;
/// Error: the job goes on, JobErrors grows.
pub const M_ERROR: i32 = 4;
/// Warning.
pub const M_WARNING: i32 = 5;
/// Information.
pub const M_INFO: i32 = 6;
/// A file was saved.
pub const M_SAVED: i32 = 7;
/// A file was not saved.
pub const M_NOTSAVED: i32 = 8;
/// A file was skipped (unchanged, excluded).
pub const M_SKIPPED: i32 = 9;
/// Volume mount request.
pub const M_MOUNT: i32 = 10;
/// Error, the daemon terminates.
pub const M_ERROR_TERM: i32 = 11;
/// Terminating normally.
pub const M_TERM: i32 = 12;
/// A file was restored.
pub const M_RESTORED: i32 = 13;
/// Security violation.
pub const M_SECURITY: i32 = 14;
/// Tape alert.
pub const M_ALERT: i32 = 15;
/// Volume management.
pub const M_VOLMGMT: i32 = 16;
/// Audit of console commands.
pub const M_AUDIT: i32 = 17;
/// Highest message type (`M_MAX = M_AUDIT`).
pub const M_MAX: i32 = M_AUDIT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_the_replaced_constants() {
        // ofd/src/jmsg.rs, ofd/src/backup.rs, odir (u32 copies), odir show.rs M_MAX
        assert_eq!((M_FATAL, M_ERROR, M_WARNING, M_INFO, M_SAVED, M_NOTSAVED, M_SKIPPED, M_RESTORED), (3, 4, 5, 6, 7, 8, 9, 13));
        assert_eq!(M_MAX, 17);
    }
}
