//! Read-only conmon account discovery. A candidate is not an authenticated
//! Podman endpoint and grants no access to that user's engine or files.
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub boot_id: String,
    pub pid: u32,
    pub start_ticks: u64,
    pub real_uid: u32,
    pub effective_uid: u32,
    pub executable: PathBuf,
}
#[derive(Debug, Serialize)]
pub struct Candidate {
    pub host_uid: u32,
    pub state: &'static str,
    pub observations: Vec<Observation>,
}
#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub candidates: Vec<Candidate>,
    /// Discovery cannot establish its own host PID-namespace provenance.
    pub host_scope_verified: bool,
    pub issues: BTreeSet<String>,
    pub vanished_processes: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ExecutableId {
    device: u64,
    inode: u64,
}

pub struct Scanner {
    proc_root: PathBuf,
    trusted: BTreeSet<ExecutableId>,
}
impl Scanner {
    /// Trusted paths are administrator-provided, not read from process argv.
    /// Linux executable identity is checked by inode/device, not basename.
    #[cfg(unix)]
    pub fn new(proc_root: impl Into<PathBuf>, paths: &[PathBuf]) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;
        let mut trusted = BTreeSet::new();
        for path in paths {
            let meta = std::fs::metadata(path)?;
            if !meta.is_file()
                || meta.uid() != 0
                || meta.mode() & 0o022 != 0
                || meta.mode() & 0o111 == 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "conmon executable must be root-owned, executable and not writable by group/others",
                ));
            }
            trusted.insert(ExecutableId {
                device: meta.dev(),
                inode: meta.ino(),
            });
        }
        Ok(Self {
            proc_root: proc_root.into(),
            trusted,
        })
    }
    #[cfg(not(unix))]
    pub fn new(_: impl Into<PathBuf>, _: &[PathBuf]) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "conmon discovery requires Linux procfs",
        ))
    }
    pub fn scan(&self) -> io::Result<Report> {
        let mut report = Report::default();
        report
            .issues
            .insert("host_pid_namespace_not_verified".into());
        if self.trusted.is_empty() {
            report.issues.insert("no_trusted_conmon_executable".into());
        }
        let boot_id =
            match std::fs::read_to_string(self.proc_root.join("sys/kernel/random/boot_id")) {
                Ok(value) if !value.trim().is_empty() => value.trim().to_string(),
                _ => {
                    report.issues.insert("boot_id_unavailable".into());
                    String::new()
                }
            };
        match std::fs::read_to_string(self.proc_root.join("self/mountinfo")) {
            Ok(mounts) => {
                if mounts.contains("hidepid=") {
                    report.issues.insert("procfs_hidepid_configured".into());
                }
            }
            Err(_) => {
                report.issues.insert("procfs_visibility_unverified".into());
            }
        }
        let mut grouped: BTreeMap<u32, Vec<Observation>> = BTreeMap::new();
        for entry in std::fs::read_dir(&self.proc_root)? {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    report
                        .issues
                        .insert("process_enumeration_incomplete".into());
                    continue;
                }
            };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            match self.observe(pid, &boot_id) {
                Ok(Some(observation)) => grouped
                    .entry(observation.effective_uid)
                    .or_default()
                    .push(observation),
                Ok(None) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    report.vanished_processes += 1
                }
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    report.issues.insert("process_access_denied".into());
                }
                Err(_) => {
                    report
                        .issues
                        .insert("process_observation_incomplete".into());
                }
            }
        }
        report.candidates = grouped
            .into_iter()
            .map(|(host_uid, mut observations)| {
                observations.sort_by_key(|p| p.pid);
                Candidate {
                    host_uid,
                    state: "discovered_unconfirmed",
                    observations,
                }
            })
            .collect();
        Ok(report)
    }
    #[cfg(unix)]
    fn observe(&self, pid: u32, boot_id: &str) -> io::Result<Option<Observation>> {
        use std::os::unix::fs::MetadataExt;
        let dir = self.proc_root.join(pid.to_string());
        let first = std::fs::read_to_string(dir.join("stat"))?;
        let start = start_ticks(&first, pid)?;
        let meta = std::fs::metadata(dir.join("exe"))?;
        let identity = ExecutableId {
            device: meta.dev(),
            inode: meta.ino(),
        };
        if !self.trusted.contains(&identity) {
            return Ok(None);
        }
        let executable = std::fs::read_link(dir.join("exe"))?;
        let status = std::fs::read_to_string(dir.join("status"))?;
        let (real_uid, effective_uid) = uids(&status)?;
        // Read the tuple a second time; PID reuse or executable/UID transitions
        // during discovery invalidate the observation instead of changing scope.
        let second = std::fs::read_to_string(dir.join("stat"))?;
        let meta2 = std::fs::metadata(dir.join("exe"))?;
        let status2 = std::fs::read_to_string(dir.join("status"))?;
        if start != start_ticks(&second, pid)?
            || identity
                != (ExecutableId {
                    device: meta2.dev(),
                    inode: meta2.ino(),
                })
            || (real_uid, effective_uid) != uids(&status2)?
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "process changed during observation",
            ));
        }
        Ok(Some(Observation {
            boot_id: boot_id.into(),
            pid,
            start_ticks: start,
            real_uid,
            effective_uid,
            executable,
        }))
    }
    #[cfg(not(unix))]
    fn observe(&self, _: u32, _: &str) -> io::Result<Option<Observation>> {
        Ok(None)
    }
}
fn start_ticks(stat: &str, expected_pid: u32) -> io::Result<u64> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid proc stat");
    let (prefix, rest) = stat.rsplit_once(") ").ok_or_else(invalid)?;
    let pid = prefix
        .split_once(" (")
        .and_then(|(p, _)| p.parse::<u32>().ok())
        .ok_or_else(invalid)?;
    if pid != expected_pid {
        return Err(invalid());
    }
    // First remainder field is state (field 3); starttime is field 22.
    rest.split_whitespace()
        .nth(19)
        .and_then(|s| s.parse().ok())
        .ok_or_else(invalid)
}
fn uids(status: &str) -> io::Result<(u32, u32)> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid proc status Uid");
    let mut fields = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .ok_or_else(invalid)?
        .split_whitespace();
    let real = fields
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(invalid)?;
    let effective = fields
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(invalid)?;
    Ok((real, effective))
}

/// Default installation paths are candidates for administrator trust, not
/// container-owned paths. Missing paths are ignored; a scan with no installed
/// executable reports incomplete discovery instead of asserting an empty host.
pub fn installed_conmon_paths() -> Vec<PathBuf> {
    [
        "/usr/bin/conmon",
        "/usr/libexec/podman/conmon",
        "/usr/libexec/crio/conmon",
    ]
    .into_iter()
    .map(Path::new)
    .filter(|p| p.exists())
    .map(Path::to_path_buf)
    .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};
    fn stat(pid: u32, start: u64) -> String {
        let mut fields = vec!["0".to_string(); 20];
        fields[0] = "S".into();
        fields[19] = start.to_string();
        format!("{pid} (conmon ) strange) {}", fields.join(" "))
    }
    #[test]
    fn stat_parser_handles_parentheses_and_rejects_wrong_pid() {
        assert_eq!(start_ticks(&stat(42, 999), 42).unwrap(), 999);
        assert!(start_ticks(&stat(42, 999), 43).is_err());
        assert!(start_ticks("42 bad", 42).is_err());
        assert_eq!(
            uids("Name: conmon\nUid:\t1000 1001 1001 1001\n").unwrap(),
            (1000, 1001)
        );
    }
    #[test]
    fn groups_by_host_uid_without_granting_engine_access_or_trusting_names() {
        let root =
            std::env::temp_dir().join(format!("onesimus-conmon-discovery-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sys/kernel/random")).unwrap();
        std::fs::create_dir_all(root.join("self")).unwrap();
        std::fs::write(root.join("sys/kernel/random/boot_id"), "test-boot\n").unwrap();
        std::fs::write(root.join("self/mountinfo"), "proc hidepid=2").unwrap();
        let trusted = root.join("trusted-conmon");
        let impostor = root.join("fake-conmon");
        std::fs::write(&trusted, b"fixture").unwrap();
        std::fs::write(&impostor, b"other").unwrap();
        for (pid, uid, exe) in [
            (10, 1000, &trusted),
            (11, 1000, &trusted),
            (12, 1001, &trusted),
            (13, 0, &impostor),
        ] {
            let dir = root.join(pid.to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("stat"), stat(pid, pid as u64 * 10)).unwrap();
            std::fs::write(
                dir.join("status"),
                format!("Name: conmon\nUid: {uid} {uid} {uid} {uid}\n"),
            )
            .unwrap();
            symlink(exe, dir.join("exe")).unwrap();
        }
        // Synthetic procfs fixtures explicitly supply an identity; production
        // Scanner::new additionally enforces root-owned installation policy.
        let meta = std::fs::metadata(&trusted).unwrap();
        let scanner = Scanner {
            proc_root: root.clone(),
            trusted: [ExecutableId {
                device: meta.dev(),
                inode: meta.ino(),
            }]
            .into(),
        };
        let report = scanner.scan().unwrap();
        assert_eq!(
            report
                .candidates
                .iter()
                .map(|c| c.host_uid)
                .collect::<Vec<_>>(),
            vec![1000, 1001]
        );
        assert_eq!(report.candidates[0].observations.len(), 2);
        assert!(
            report
                .candidates
                .iter()
                .all(|c| c.state == "discovered_unconfirmed")
        );
        assert!(!report.host_scope_verified);
        assert!(report.issues.contains("procfs_hidepid_configured"));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn empty_scan_is_not_a_claim_of_complete_inventory() {
        let root =
            std::env::temp_dir().join(format!("onesimus-conmon-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let report = Scanner {
            proc_root: root.clone(),
            trusted: BTreeSet::new(),
        }
        .scan()
        .unwrap();
        assert!(report.candidates.is_empty());
        assert!(report.issues.contains("host_pid_namespace_not_verified"));
        assert!(report.issues.contains("no_trusted_conmon_executable"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
