//! The lifecycle journal with recovery (docs/plans/PODMAN-PLUGIN.md,
//! "Journal mit Recovery", "consistency_scope=job").
//!
//! # Purpose
//!
//! Before every mutation (stop, mask, unmask, start) the host writes the
//! journal of the writer group: a root-owned file
//! `/var/lib/onesimus/ofd/podman-journal/<domain>/<jobid>.json`, written
//! with `fsync` and `rename`. If `ofd` dies or hangs, the recovery at its
//! next start, before the next Podman job of the domain, or by the dead
//! man's switch after `max_downtime` reads the journal and decides per
//! member from what it observes *now* whether it may restart it. It only
//! ever undoes its **own**, **unchanged** stops; anything else is
//! `CONFLICT` for a human.
//!
//! Decided procedure (user decision 19.11.2026): `stop` with the intact
//! unit, then `mask --runtime`, verified; restart with `unmask --runtime`
//! (never plain `unmask`, which would also drop an administrator's
//! persistent mask), then `start`.
//!
//! # Data flow
//!
//! ```mermaid
//! stateDiagram-v2
//!     [*] --> PREPARED: inventory and inspect done,<br/>dead man's switch armed
//!     PREPARED --> STOP_REQUESTED: per member, before stop<br/>(mask_before recorded)
//!     PREPARED --> ABORTED: plan error, nothing mutated
//!     STOP_REQUESTED --> MASK_REQUESTED: stopped, before mask --runtime
//!     MASK_REQUESTED --> STOP_REQUESTED: next member
//!     MASK_REQUESTED --> QUIESCED: all stopped and masked, verified
//!     QUIESCED --> CAPTURING: before the job's first file
//!     CAPTURING --> RESUME_PENDING: after the last file
//!     STOP_REQUESTED --> RESUME_PENDING: cancel, error, SIGTERM, max_downtime
//!     MASK_REQUESTED --> RESUME_PENDING: cancel, error, SIGTERM, max_downtime
//!     QUIESCED --> RESUME_PENDING: cancel, error
//!     RESUME_PENDING --> RESUMED: all members back in their original state
//!     RESUME_PENDING --> CONFLICT: a member changed under us
//!     STOP_REQUESTED --> CONFLICT
//!     MASK_REQUESTED --> CONFLICT
//!     RESUMED --> [*]: timer removed, archived
//!     ABORTED --> [*]
//!     CONFLICT --> [*]: ofd --podman-journal --resolve
//! ```
//!
//! ```mermaid
//! sequenceDiagram
//!     participant H as ofd (host)
//!     participant J as journal file
//!     participant S as systemd (user or system manager)
//!     H->>J: PREPARED (members, fingerprints, dead man's switch)
//!     loop each member, dependents first
//!         H->>J: STOP_REQUESTED (mask_before)
//!         H->>S: stop unit
//!         H->>J: MASK_REQUESTED (stop result)
//!         H->>S: mask --runtime unit
//!     end
//!     H->>S: verify masked, inactive, no new InvocationID
//!     H->>J: QUIESCED, then CAPTURING
//!     Note over H: the whole job's files (consistency_scope=job)
//!     H->>J: RESUME_PENDING
//!     loop each member, reverse order
//!         H->>S: unmask --runtime, start
//!     end
//!     H->>J: RESUMED
//! ```
//!
//! # Counterpart
//!
//! `ofd` (lifecycle guard, `--podman-recover`, recovery at start) writes and
//! reads it through [`Store`]; [`recover_member`] is the single decision
//! function both use.

use std::ffi::CString;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use serde::{Deserialize, Serialize};

/// Format marker of a journal file.
pub const FORMAT: &str = "onesimus-podman-journal/1";

/// Default base directory.
pub const DEFAULT_BASE: &str = "/var/lib/onesimus/ofd/podman-journal";

/// Largest journal file.
pub const MAX_JOURNAL: usize = 1024 * 1024;

/// States of the journal (and of a member).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum State {
    /// Inventory and inspect of all members done, dead man's switch armed.
    Prepared,
    /// Before the stop of a member.
    StopRequested,
    /// After the stop, before `mask --runtime`.
    MaskRequested,
    /// All members stopped and masked, verified.
    Quiesced,
    /// Before the first file of the job.
    Capturing,
    /// After the last file, or on cancel, error, SIGTERM, max_downtime.
    ResumePending,
    /// Everything back in its original state.
    Resumed,
    /// Needs a human.
    Conflict,
    /// Ended before any mutation.
    Aborted,
}

impl State {
    /// Whether the journal-level transition `self → to` is allowed.
    pub fn may_go_to(self, to: State) -> bool {
        use State::*;
        matches!(
            (self, to),
            (Prepared, StopRequested | Aborted | ResumePending)
                | (StopRequested, MaskRequested | ResumePending | Conflict)
                | (MaskRequested, StopRequested | Quiesced | ResumePending | Conflict)
                | (Quiesced, Capturing | ResumePending | Conflict)
                | (Capturing, ResumePending)
                | (ResumePending, Resumed | Conflict)
        )
    }

    /// Final states: the journal is done.
    pub fn is_final(self) -> bool {
        matches!(self, State::Resumed | State::Conflict | State::Aborted)
    }
}

/// The mask of a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mask {
    /// No mask.
    None,
    /// `/run/systemd/…` link to /dev/null.
    Runtime,
    /// `/etc/systemd/…` link to /dev/null (the administrator's).
    Persistent,
}

/// Kind of a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    /// A systemd unit (Quadlet or other unit with PODMAN_SYSTEMD_UNIT).
    Unit,
    /// A container without unit.
    Container,
}

/// Result of a stop.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopResult {
    /// Exit code of the main process.
    pub exit_code: Option<i32>,
    /// In `clean_exit_codes`, no SIGKILL, `Result=success`.
    pub clean: bool,
    /// When the stop finished (Unix seconds).
    pub at_unix: u64,
}

/// One member of the writer group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// Unit or container.
    pub kind: MemberKind,
    /// Unit name or container name.
    pub name: String,
    /// Unit: `InvocationID` before the stop.
    pub invocation_id: Option<String>,
    /// Unit: `ActiveEnterTimestampMonotonic`.
    pub active_enter: Option<u64>,
    /// Container: ID before the stop.
    pub container_id: Option<String>,
    /// Container: `StartedAt` before the stop.
    pub started_at: Option<String>,
    /// SHA-256 over Quadlet source, drop-ins and generated unit (or spec).
    pub fingerprint: String,
    /// Active before our stop.
    pub was_active: bool,
    /// The mask before our stop (V-5); a mask present before is never ours.
    pub mask_before: Mask,
    /// We set a runtime mask.
    pub own_mask: bool,
    /// The member's own progress.
    pub state: State,
    /// Result of our stop.
    pub stop: Option<StopResult>,
}

/// The journal of one writer group in one job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// [`FORMAT`].
    pub format: String,
    /// `/proc/sys/kernel/random/boot_id` when written.
    pub boot_id: String,
    /// Job id.
    pub job_id: u64,
    /// Plugin instance id.
    pub instance: String,
    /// Host uid of the domain.
    pub host_uid: u32,
    /// Domain id (`root`, `uid-<n>`).
    pub domain: String,
    /// libpod tmpdir (part of the domain identity).
    pub tmpdir: Option<String>,
    /// Unit of the dead man's switch.
    pub deadman: Option<String>,
    /// First stop (Unix seconds).
    pub first_stop_unix: Option<u64>,
    /// Upper bound of the downtime (seconds).
    pub max_downtime: u64,
    /// Journal-level state.
    pub state: State,
    /// Members, in stop order.
    pub members: Vec<Member>,
}

impl Journal {
    /// A new journal in `PREPARED`.
    pub fn prepared(boot_id: &str, job_id: u64, instance: &str, host_uid: u32, domain: &str, max_downtime: u64,
        members: Vec<Member>) -> Self {
        Self {
            format: FORMAT.into(),
            boot_id: boot_id.into(),
            job_id,
            instance: instance.into(),
            host_uid,
            domain: domain.into(),
            tmpdir: None,
            deadman: None,
            first_stop_unix: None,
            max_downtime,
            state: State::Prepared,
            members,
        }
    }

    /// Moves the journal to `to`; refuses transitions the plan does not have.
    pub fn advance(&mut self, to: State) -> Result<(), String> {
        if !self.state.may_go_to(to) {
            return Err(format!("journal: {:?} -> {:?} is not allowed", self.state, to));
        }
        self.state = to;
        Ok(())
    }

    /// Parses and checks a journal file.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_JOURNAL {
            return Err("journal too large".into());
        }
        let j: Self = serde_json::from_slice(bytes).map_err(|e| format!("journal: {e}"))?;
        if j.format != FORMAT || !valid_domain(&j.domain) {
            return Err("journal: wrong format or domain".into());
        }
        Ok(j)
    }

    /// Whether `max_downtime` is reached at `now_unix`.
    pub fn downtime_exceeded(&self, now_unix: u64) -> bool {
        self.first_stop_unix.is_some_and(|t| now_unix.saturating_sub(t) >= self.max_downtime)
    }
}

/// What the recovery sees now for one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// Current boot id.
    pub boot_id: String,
    /// Unit active / container running.
    pub active: bool,
    /// Unit: current InvocationID.
    pub invocation_id: Option<String>,
    /// Unit: current mask.
    pub mask: Mask,
    /// Current definition fingerprint.
    pub fingerprint: String,
    /// Container: current ID.
    pub container_id: Option<String>,
    /// Container: `StoppedByUser`.
    pub stopped_by_user: bool,
    /// Container: `FinishedAt` equals the end of our own stop.
    pub finished_by_our_stop: bool,
    /// Container: has a restart policy (new boot: systemd/podman-restart).
    pub restart_policy: bool,
}

/// The recovery decision for one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Leave it as it is.
    Nothing,
    /// Start it (no mask of ours present).
    Start,
    /// `unmask --runtime`, then start (own mask present).
    UnmaskAndStart,
    /// Remove only our runtime mask, do not start (the member runs again).
    UnmaskOnly,
    /// Changed under us: a human decides.
    Conflict(&'static str),
}

/// The decision for `member` of `journal` given what is observed now.
/// One function for the start of ofd, `--podman-recover` and the guard.
pub fn recover_member(journal: &Journal, member: &Member, now: &Observed) -> Action {
    use State::*;
    let new_boot = now.boot_id != journal.boot_id;
    if journal.state == Prepared || member.state == Prepared {
        // Nothing of this member was mutated.
        return Action::Nothing;
    }
    if !member.was_active {
        return Action::Nothing;
    }
    if new_boot {
        // The runtime mask is gone with the reboot; systemd started units
        // by WantedBy. Units are only checked; plain containers without
        // restart policy start only with an unchanged definition.
        return match member.kind {
            MemberKind::Unit => Action::Nothing,
            MemberKind::Container if now.active || now.restart_policy => Action::Nothing,
            MemberKind::Container if now.fingerprint == member.fingerprint => Action::Start,
            MemberKind::Container => Action::Conflict("definition changed after reboot"),
        };
    }
    if now.fingerprint != member.fingerprint {
        return Action::Conflict("definition changed");
    }
    let own_mask_present = now.mask == Mask::Runtime && member.mask_before == Mask::None;
    match member.kind {
        MemberKind::Container => {
            if now.container_id != member.container_id {
                return Action::Conflict("container replaced");
            }
            if now.active {
                // Someone started it; ours was a stop: if our stop never ran, nothing happened.
                return if member.state == StopRequested { Action::Nothing } else { Action::Conflict("started by someone else") };
            }
            if member.state == StopRequested && member.stop.is_none() && !now.stopped_by_user {
                return Action::Conflict("stopped by someone else");
            }
            if now.stopped_by_user && (member.stop.is_none() || now.finished_by_our_stop) {
                Action::Start
            } else {
                Action::Conflict("not our stop")
            }
        }
        MemberKind::Unit => {
            let same_invocation = now.invocation_id == member.invocation_id;
            match member.state {
                StopRequested => {
                    if now.active && same_invocation {
                        Action::Nothing
                    } else if !now.active && now.mask == Mask::None {
                        Action::Start
                    } else {
                        Action::Conflict("unit changed while stopping")
                    }
                }
                MaskRequested | Quiesced | Capturing | ResumePending => {
                    if now.active {
                        // Started in the window between stop and mask (or by
                        // a human): consistency violated, no second stop.
                        if own_mask_present { Action::UnmaskOnly } else { Action::Conflict("started during the backup") }
                    } else if own_mask_present {
                        Action::UnmaskAndStart
                    } else if now.mask == Mask::None && member.state == MaskRequested {
                        Action::Start
                    } else {
                        Action::Conflict("mask is not ours")
                    }
                }
                _ => Action::Nothing,
            }
        }
    }
}

/// A domain directory name: `root` or `uid-<n>` (optionally `-<short>`).
pub fn valid_domain(s: &str) -> bool {
    if s == "root" {
        return true;
    }
    let Some(rest) = s.strip_prefix("uid-") else { return false };
    let (num, tag) = rest.split_once('-').unwrap_or((rest, ""));
    !num.is_empty() && num.len() <= 10 && num.bytes().all(|b| b.is_ascii_digit())
        && tag.len() <= 16 && tag.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The journal directory, opened through an `O_NOFOLLOW` chain.
#[derive(Debug)]
pub struct Store {
    /// Descriptor of the base directory.
    base: OwnedFd,
    /// Required owner of every directory and file.
    owner: u32,
}

/// `errno` as an error.
fn os_err(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

/// Opens (and with `create` makes, mode 0700) one directory below `parent`
/// without following links, then checks owner and mode.
pub fn open_dir_at(parent: Option<&OwnedFd>, name: &str, create: bool, owner: u32) -> Result<OwnedFd, String> {
    let c = CString::new(name).map_err(|_| "NUL in path")?;
    let at = parent.map_or(libc::AT_FDCWD, |p| p.as_raw_fd());
    if create {
        // SAFETY: valid descriptor and NUL-terminated name.
        let r = unsafe { libc::mkdirat(at, c.as_ptr(), 0o700) };
        if r < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
            return Err(os_err(name));
        }
    }
    // SAFETY: as above; O_NOFOLLOW refuses a link as last component.
    let fd = unsafe { libc::openat(at, c.as_ptr(), libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_RDONLY) };
    if fd < 0 {
        return Err(os_err(name));
    }
    // SAFETY: fd was just opened and is owned here.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: zeroed stat is a valid out-parameter for fstat.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid descriptor and out-parameter.
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } < 0 {
        return Err(os_err(name));
    }
    if st.st_uid != owner || st.st_mode & 0o022 != 0 {
        return Err(format!("{name}: owned by uid {} with mode {:o}, refused", st.st_uid, st.st_mode & 0o7777));
    }
    Ok(fd)
}

impl Store {
    /// Opens `base` component by component with `O_NOFOLLOW`. Components
    /// up to `top` are only opened (no link followed); every component
    /// below `top` is created if missing (0700) and must belong to `owner`
    /// and not be writable by group or others. Production: `top` = `/`,
    /// `owner` = root.
    pub fn open(base: &std::path::Path, owner: u32, top: &std::path::Path) -> Result<Self, String> {
        if !base.is_absolute() || !base.starts_with(top) {
            return Err("journal base must be absolute and below the trusted top".into());
        }
        let normal = |p: &std::path::Path| p.components().all(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_)));
        if !normal(base) || !normal(top) {
            return Err("journal base must be normal".into());
        }
        let depth = top.components().count();
        let c = CString::new("/").map_err(|_| "NUL")?;
        // SAFETY: opening / as a directory.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_RDONLY) };
        if fd < 0 {
            return Err(os_err("/"));
        }
        // SAFETY: fd just opened and owned here.
        let mut fd = unsafe { OwnedFd::from_raw_fd(fd) };
        for (i, part) in base.components().enumerate().skip(1) {
            let name = part.as_os_str().to_string_lossy().into_owned();
            fd = if i < depth { open_link_free(&fd, &name)? } else { open_dir_at(Some(&fd), &name, true, owner)? };
        }
        Ok(Self { base: fd, owner })
    }

    /// The directory of `domain`, created if needed.
    fn domain_dir(&self, domain: &str) -> Result<OwnedFd, String> {
        if !valid_domain(domain) {
            return Err("invalid domain".into());
        }
        open_dir_at(Some(&self.base), domain, true, self.owner)
    }

    /// Writes `journal` atomically: temp file (O_EXCL, O_NOFOLLOW, 0600),
    /// fsync, renameat, fsync of the directory.
    pub fn write(&self, journal: &Journal) -> Result<(), String> {
        let dir = self.domain_dir(&journal.domain)?;
        let name = format!("{}.json", journal.job_id);
        let tmp = format!(".{}.json.tmp", journal.job_id);
        let bytes = serde_json::to_vec_pretty(journal).map_err(|e| e.to_string())?;
        let ctmp = CString::new(tmp.clone()).map_err(|_| "NUL")?;
        // SAFETY: valid descriptor and name; a stale temp file is removed first.
        unsafe { libc::unlinkat(dir.as_raw_fd(), ctmp.as_ptr(), 0) };
        // SAFETY: valid descriptor and name.
        let fd = unsafe {
            libc::openat(dir.as_raw_fd(), ctmp.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600)
        };
        if fd < 0 {
            return Err(os_err(&tmp));
        }
        // SAFETY: fd just opened, owned by the File.
        let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        let cname = CString::new(name.clone()).map_err(|_| "NUL")?;
        // SAFETY: both names relative to the same valid directory.
        if unsafe { libc::renameat(dir.as_raw_fd(), ctmp.as_ptr(), dir.as_raw_fd(), cname.as_ptr()) } < 0 {
            return Err(os_err(&name));
        }
        // SAFETY: fsync on a valid directory descriptor.
        unsafe { libc::fsync(dir.as_raw_fd()) };
        Ok(())
    }

    /// Reads the journal of `domain`/`job_id`, None if there is none.
    pub fn read(&self, domain: &str, job_id: u64) -> Result<Option<Journal>, String> {
        let dir = self.domain_dir(domain)?;
        let c = CString::new(format!("{job_id}.json")).map_err(|_| "NUL")?;
        // SAFETY: valid descriptor and name; no link is followed.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
        if fd < 0 {
            return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) { Ok(None) } else { Err(os_err("journal")) };
        }
        // SAFETY: fd just opened, owned by the File.
        let f = unsafe { std::fs::File::from_raw_fd(fd) };
        let meta = f.metadata().map_err(|e| e.to_string())?;
        if std::os::unix::fs::MetadataExt::uid(&meta) != self.owner || !meta.is_file() {
            return Err("journal file has a foreign owner".into());
        }
        let mut bytes = Vec::new();
        f.take(MAX_JOURNAL as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        Journal::parse(&bytes).map(Some)
    }

    /// Job ids with a journal in `domain`.
    pub fn jobs(&self, domain: &str) -> Result<Vec<u64>, String> {
        let dir = self.domain_dir(domain)?;
        // SAFETY: duplicate of a valid descriptor for fdopendir.
        let dup = unsafe { libc::fcntl(dir.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if dup < 0 {
            return Err(os_err("dup"));
        }
        let path = format!("/proc/self/fd/{dup}");
        let entries = std::fs::read_dir(&path).map_err(|e| e.to_string());
        // SAFETY: closing our own duplicate.
        unsafe { libc::close(dup) };
        let mut jobs: Vec<u64> = entries?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json")?.parse().ok())
            .collect();
        jobs.sort_unstable();
        Ok(jobs)
    }

    /// An exclusive `flock` on `<domain>/<job>.lock`, so the recovery runs
    /// once (timer, ofd start and guard may race). None if someone holds it.
    pub fn lock(&self, domain: &str, job_id: u64) -> Result<Option<OwnedFd>, String> {
        let dir = self.domain_dir(domain)?;
        let c = CString::new(format!("{job_id}.lock")).map_err(|_| "NUL")?;
        // SAFETY: valid descriptor and name.
        let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o600) };
        if fd < 0 {
            return Err(os_err("lock"));
        }
        // SAFETY: fd just opened and owned here.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: flock on a valid descriptor.
        if unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } < 0 {
            return Ok(None);
        }
        Ok(Some(fd))
    }
}

/// Opens an existing directory below `parent` without following a link
/// and without owner check (components above the trusted top).
fn open_link_free(parent: &OwnedFd, name: &str) -> Result<OwnedFd, String> {
    let c = CString::new(name).map_err(|_| "NUL in path")?;
    // SAFETY: valid descriptor and NUL-terminated name.
    let fd = unsafe { libc::openat(parent.as_raw_fd(), c.as_ptr(), libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_RDONLY) };
    if fd < 0 {
        return Err(os_err(name));
    }
    // SAFETY: fd just opened and owned here.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The current boot id.
pub fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map(|s| s.trim().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(kind: MemberKind, state: State) -> Member {
        Member {
            kind,
            name: "web.service".into(),
            invocation_id: Some("inv1".into()),
            active_enter: Some(5),
            container_id: Some("c1".into()),
            started_at: Some("t".into()),
            fingerprint: "fp".into(),
            was_active: true,
            mask_before: Mask::None,
            own_mask: state != State::StopRequested,
            state,
            stop: Some(StopResult { exit_code: Some(0), clean: true, at_unix: 10 }),
        }
    }
    fn journal(state: State) -> Journal {
        let mut j = Journal::prepared("boot1", 42, "42-0-podman", 1000, "uid-1000", 7200, vec![]);
        j.state = state;
        j
    }
    fn seen(active: bool, mask: Mask) -> Observed {
        Observed {
            boot_id: "boot1".into(),
            active,
            invocation_id: Some(if active { "inv2" } else { "inv1" }.into()),
            mask,
            fingerprint: "fp".into(),
            container_id: Some("c1".into()),
            stopped_by_user: true,
            finished_by_our_stop: true,
            restart_policy: false,
        }
    }

    #[test]
    fn transitions_follow_the_plan() {
        use State::*;
        let mut j = journal(Prepared);
        for to in [StopRequested, MaskRequested, StopRequested, MaskRequested, Quiesced, Capturing, ResumePending, Resumed] {
            j.advance(to).unwrap();
        }
        assert!(j.state.is_final());
        for (from, to) in [(Prepared, Capturing), (Prepared, Quiesced), (Capturing, Quiesced), (Resumed, StopRequested),
            (Conflict, Resumed), (Aborted, StopRequested), (StopRequested, Capturing)] {
            assert!(!from.may_go_to(to), "{from:?} -> {to:?}");
        }
    }

    /// The recovery table of the plan, one case per row.
    #[test]
    fn recovery_decisions_per_state() {
        use State::*;
        let unit = MemberKind::Unit;
        // PREPARED: nothing mutated.
        assert_eq!(recover_member(&journal(Prepared), &member(unit, Prepared), &seen(true, Mask::None)), Action::Nothing);
        // STOP_REQUESTED: active with the same invocation -> nothing happened.
        let mut same = seen(true, Mask::None);
        same.invocation_id = Some("inv1".into());
        assert_eq!(recover_member(&journal(StopRequested), &member(unit, StopRequested), &same), Action::Nothing);
        // inactive, no mask, unchanged -> start.
        assert_eq!(recover_member(&journal(StopRequested), &member(unit, StopRequested), &seen(false, Mask::None)), Action::Start);
        // active with a new invocation -> conflict.
        assert!(matches!(recover_member(&journal(StopRequested), &member(unit, StopRequested), &seen(true, Mask::None)), Action::Conflict(_)));
        // MASK_REQUESTED: own runtime mask -> like RESUME_PENDING; no mask -> like STOP_REQUESTED.
        assert_eq!(recover_member(&journal(MaskRequested), &member(unit, MaskRequested), &seen(false, Mask::Runtime)), Action::UnmaskAndStart);
        assert_eq!(recover_member(&journal(MaskRequested), &member(unit, MaskRequested), &seen(false, Mask::None)), Action::Start);
        // QUIESCED/CAPTURING/RESUME_PENDING: own mask, inactive, unchanged -> unmask and start.
        for s in [Quiesced, Capturing, ResumePending] {
            assert_eq!(recover_member(&journal(s), &member(unit, Quiesced), &seen(false, Mask::Runtime)), Action::UnmaskAndStart);
        }
        // A persistent mask is never ours.
        assert!(matches!(recover_member(&journal(ResumePending), &member(unit, Quiesced), &seen(false, Mask::Persistent)), Action::Conflict(_)));
        // A mask that existed before is never ours.
        let mut pre = member(unit, Quiesced);
        pre.mask_before = Mask::Runtime;
        assert!(matches!(recover_member(&journal(ResumePending), &pre, &seen(false, Mask::Runtime)), Action::Conflict(_)));
        // Changed definition.
        let mut changed = seen(false, Mask::Runtime);
        changed.fingerprint = "other".into();
        assert!(matches!(recover_member(&journal(ResumePending), &member(unit, Quiesced), &changed), Action::Conflict(_)));
        // Started in the window between stop and mask: only remove our mask.
        assert_eq!(recover_member(&journal(ResumePending), &member(unit, Quiesced), &seen(true, Mask::Runtime)), Action::UnmaskOnly);
    }

    #[test]
    fn new_boot_and_containers() {
        use State::*;
        let mut reboot = seen(false, Mask::None);
        reboot.boot_id = "boot2".into();
        assert_eq!(recover_member(&journal(Capturing), &member(MemberKind::Unit, Quiesced), &reboot), Action::Nothing);
        assert_eq!(recover_member(&journal(Capturing), &member(MemberKind::Container, Quiesced), &reboot), Action::Start);
        reboot.restart_policy = true;
        assert_eq!(recover_member(&journal(Capturing), &member(MemberKind::Container, Quiesced), &reboot), Action::Nothing);
        let c = MemberKind::Container;
        assert_eq!(recover_member(&journal(ResumePending), &member(c, Quiesced), &seen(false, Mask::None)), Action::Start);
        let mut replaced = seen(false, Mask::None);
        replaced.container_id = Some("c2".into());
        assert!(matches!(recover_member(&journal(ResumePending), &member(c, Quiesced), &replaced), Action::Conflict(_)));
        let mut foreign = seen(false, Mask::None);
        foreign.finished_by_our_stop = false;
        assert!(matches!(recover_member(&journal(ResumePending), &member(c, Quiesced), &foreign), Action::Conflict(_)));
        let mut inactive_before = member(c, Quiesced);
        inactive_before.was_active = false;
        assert_eq!(recover_member(&journal(ResumePending), &inactive_before, &seen(false, Mask::None)), Action::Nothing);
    }

    fn me() -> u32 {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() }
    }
    fn tmpbase(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("podcore-journal-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn store_writes_atomically_and_locks() {
        let base = tmpbase("store");
        let s = Store::open(&base, me(), &std::env::temp_dir()).unwrap();
        let mut j = journal(State::Prepared);
        j.members.push(member(MemberKind::Unit, State::Prepared));
        s.write(&j).unwrap();
        j.advance(State::StopRequested).unwrap();
        s.write(&j).unwrap();
        assert_eq!(s.read("uid-1000", 42).unwrap().unwrap(), j);
        assert_eq!(s.read("uid-1000", 43).unwrap(), None);
        assert_eq!(s.jobs("uid-1000").unwrap(), [42]);
        let held = s.lock("uid-1000", 42).unwrap().unwrap();
        assert!(s.lock("uid-1000", 42).unwrap().is_none(), "second lock on another descriptor fails");
        drop(held);
        assert!(s.lock("uid-1000", 42).unwrap().is_some());
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(base.join("uid-1000/42.json")).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(s.write(&Journal { domain: "../x".into(), ..j.clone() }).is_err());
        // Another owner than required is refused.
        assert!(Store::open(&base, me() + 1, &std::env::temp_dir()).is_err());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn store_refuses_links_and_writable_directories() {
        use std::os::unix::fs::PermissionsExt;
        let base = tmpbase("links");
        std::fs::create_dir_all(base.join("real")).unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("uid-1000")).unwrap();
        let s = Store::open(&base, me(), &std::env::temp_dir()).unwrap();
        assert!(s.write(&journal(State::Prepared)).is_err(), "a planted link is not followed");
        std::fs::remove_file(base.join("uid-1000")).unwrap();
        std::fs::create_dir(base.join("uid-1000")).unwrap();
        std::fs::set_permissions(base.join("uid-1000"), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(s.write(&journal(State::Prepared)).is_err(), "a world-writable directory is refused");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn domains_and_downtime() {
        for ok in ["root", "uid-1000", "uid-1000-b"] {
            assert!(valid_domain(ok), "{ok}");
        }
        for bad in ["", "uid-", "uid-x", "../root", "uid-1/2", "uid-1-a.b"] {
            assert!(!valid_domain(bad), "{bad}");
        }
        let mut j = journal(State::Capturing);
        assert!(!j.downtime_exceeded(100));
        j.first_stop_unix = Some(100);
        assert!(!j.downtime_exceeded(7299) && j.downtime_exceeded(7300));
    }
}
