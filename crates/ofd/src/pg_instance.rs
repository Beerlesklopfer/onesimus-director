//! A local PostgreSQL instance and the account it runs as
//! (docs/plans/POSTGRESQL-PHASE2.md, C).
//!
//! The broker of an instance runs as the instance's own user. That user is
//! read from the instance itself, never from plugin options: the socket, its
//! lock file (`.s.PGSQL.<port>.lock`, which names the postmaster PID and the
//! data directory), `<datadir>/postmaster.pid` and the running process must
//! all belong to the same account, and that account must not be root.

use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

use crate::broker::RunAs;
use crate::plugin::PluginError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub socket_dir: PathBuf,
    pub port: u16,
    pub data_dir: PathBuf,
    pub pid: i32,
    pub run_as: RunAs,
    /// Major version from PG_VERSION: selects /usr/lib/postgresql/<major>/bin.
    pub major: u32,
}

fn plan_error(path: &Path, reason: impl Into<String>) -> PluginError {
    PluginError::Untrusted { path: path.to_path_buf(), reason: reason.into() }
}

fn read_small(path: &Path) -> Result<String, PluginError> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| plan_error(path, e.to_string()))?;
    if !meta.is_file() || meta.len() > 4096 {
        return Err(plan_error(path, "not a small regular file"));
    }
    std::fs::read_to_string(path).map_err(|e| plan_error(path, e.to_string()))
}

/// The instance listening on `<socket_dir>/.s.PGSQL.<port>`.
pub fn discover(socket_dir: &Path, port: u16) -> Result<Instance, PluginError> {
    let socket = socket_dir.join(format!(".s.PGSQL.{port}"));
    let lock = socket_dir.join(format!(".s.PGSQL.{port}.lock"));
    let socket_meta = std::fs::symlink_metadata(&socket).map_err(|e| plan_error(&socket, e.to_string()))?;
    if !socket_meta.file_type().is_socket() {
        return Err(plan_error(&socket, "not a socket"));
    }
    let lock_text = read_small(&lock)?;
    let lock_uid = std::fs::symlink_metadata(&lock).map_err(|e| plan_error(&lock, e.to_string()))?.uid();
    let mut lines = lock_text.lines();
    let pid: i32 = lines.next().and_then(|l| l.trim().parse().ok())
        .ok_or_else(|| plan_error(&lock, "no postmaster PID"))?;
    let data_dir = PathBuf::from(lines.next().map(str::trim).filter(|l| l.starts_with('/'))
        .ok_or_else(|| plan_error(&lock, "no data directory"))?);
    let _start = lines.next();
    let lock_port: Option<u16> = lines.next().and_then(|l| l.trim().parse().ok());
    if lock_port != Some(port) {
        return Err(plan_error(&lock, format!("names port {lock_port:?}, not {port}")));
    }

    let pidfile = data_dir.join("postmaster.pid");
    let pid_text = read_small(&pidfile)?;
    let pidfile_uid = std::fs::symlink_metadata(&pidfile).map_err(|e| plan_error(&pidfile, e.to_string()))?.uid();
    if pid_text.lines().next().and_then(|l| l.trim().parse::<i32>().ok()) != Some(pid) {
        return Err(plan_error(&pidfile, "PID differs from the socket lock file"));
    }

    let proc_dir = PathBuf::from(format!("/proc/{pid}"));
    let comm = std::fs::read_to_string(proc_dir.join("comm")).map_err(|e| plan_error(&proc_dir, e.to_string()))?;
    if !matches!(comm.trim(), "postgres" | "postmaster") {
        return Err(plan_error(&proc_dir, format!("process is {:?}, not postgres", comm.trim())));
    }
    let status = std::fs::read_to_string(proc_dir.join("status")).map_err(|e| plan_error(&proc_dir, e.to_string()))?;
    let uids: Vec<u32> = status.lines().find_map(|l| l.strip_prefix("Uid:"))
        .map(|l| l.split_whitespace().filter_map(|v| v.parse().ok()).collect()).unwrap_or_default();
    let process_uid = match uids.as_slice() {
        [real, effective, ..] if real == effective => *real,
        _ => return Err(plan_error(&proc_dir, "real and effective uid differ")),
    };

    let owners = [socket_meta.uid(), lock_uid, pidfile_uid, process_uid];
    if owners.iter().any(|&u| u != owners[0]) {
        return Err(plan_error(socket_dir, format!(
            "owners differ: socket {}, lock {}, postmaster.pid {}, process {}", owners[0], owners[1], owners[2], owners[3])));
    }
    let uid = owners[0];
    if uid == 0 {
        return Err(plan_error(socket_dir, "the instance runs as root"));
    }
    let gid = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid)).ok().flatten()
        .map(|u| u.gid.as_raw()).unwrap_or(socket_meta.gid());
    let version = data_dir.join("PG_VERSION");
    let major: u32 = read_small(&version)?.trim().parse().map_err(|_| plan_error(&version, "not a number"))?;
    Ok(Instance { socket_dir: socket_dir.to_path_buf(), port, data_dir, pid, run_as: RunAs { uid, gid }, major })
}

/// A throwaway PostgreSQL cluster of the test user: own data directory and
/// socket directory, no TCP. Stopped and removed when dropped. The socket
/// directory lies directly under /tmp: a Unix socket path holds at most 107
/// bytes, and a long TMPDIR would exceed it.
#[cfg(test)]
pub(crate) mod testcluster {
    use std::path::PathBuf;
    use std::process::Command;

    pub struct Cluster {
        pub root: PathBuf,
        pub sock: PathBuf,
        pub port: u16,
        pub bin: PathBuf,
    }

    impl Cluster {
        pub fn data(&self) -> PathBuf {
            self.root.join("data")
        }
        pub fn sockets(&self) -> PathBuf {
            self.sock.clone()
        }
        /// Runs SQL as the test user against database `db`.
        pub fn sql(&self, db: &str, sql: &str) -> String {
            let out = Command::new(self.bin.join("psql"))
                .args(["-X", "-A", "-t", "-v", "ON_ERROR_STOP=1", "-h"]).arg(self.sockets())
                .args(["-p", &self.port.to_string(), "-d", db, "-c", sql])
                .output().expect("psql");
            assert!(out.status.success(), "{sql}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    }

    impl Drop for Cluster {
        fn drop(&mut self) {
            let _ = Command::new(self.bin.join("pg_ctl")).arg("-D").arg(self.data()).args(["-m", "immediate", "stop"])
                .output();
            let _ = std::fs::remove_dir_all(&self.root);
            let _ = std::fs::remove_dir_all(&self.sock);
        }
    }

    /// None when no PostgreSQL server binaries are installed.
    pub fn start(tag: &str, port: u16) -> Option<Cluster> {
        let bin = std::fs::read_dir("/usr/lib/postgresql").ok()?
            .filter_map(|e| e.ok().map(|e| e.path().join("bin")))
            .filter(|b| b.join("initdb").is_file())
            .max()?;
        let root = std::env::temp_dir().join(format!("ofd-pg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok()?;
        let sock = PathBuf::from(format!("/tmp/ofd-pgs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sock);
        std::fs::create_dir_all(&sock).ok()?;
        let cluster = Cluster { root, sock, port, bin };
        let init = Command::new(cluster.bin.join("initdb"))
            .arg("-D").arg(cluster.data()).args(["-A", "peer", "-E", "UTF8", "--no-sync", "-U"])
            .arg(nix::unistd::User::from_uid(nix::unistd::getuid()).ok()??.name)
            .output().ok()?;
        assert!(init.status.success(), "initdb: {}", String::from_utf8_lossy(&init.stderr));
        let opts = format!("-k {} -p {port} -c listen_addresses=''", cluster.sockets().display());
        let up = Command::new(cluster.bin.join("pg_ctl")).arg("-D").arg(cluster.data())
            .args(["-w", "-l"]).arg(cluster.root.join("log")).args(["-o", &opts, "start"])
            .output().ok()?;
        assert!(up.status.success(), "pg_ctl start: {}", String::from_utf8_lossy(&up.stderr));
        Some(cluster)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_a_running_cluster_and_its_user() {
        let Some(c) = testcluster::start("discover", 55432) else {
            eprintln!("no PostgreSQL server binaries; skipped");
            return;
        };
        let i = discover(&c.sockets(), c.port).unwrap();
        // SAFETY: getuid has no preconditions.
        assert_eq!(i.run_as.uid, unsafe { libc::getuid() });
        assert_eq!(std::fs::canonicalize(&i.data_dir).unwrap(), std::fs::canonicalize(c.data()).unwrap());
        assert!(i.major >= 12);
        assert!(i.pid > 1);
        // Wrong port: no socket.
        assert!(discover(&c.sockets(), c.port + 1).is_err());
    }

    #[test]
    fn a_forged_lock_file_is_refused() {
        // Under /tmp, not TMPDIR: the socket path must stay within 107 bytes.
        let d = PathBuf::from(format!("/tmp/ofd-pg-forged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("data")).unwrap();
        let _sock = std::os::unix::net::UnixListener::bind(d.join(".s.PGSQL.5999")).unwrap();
        // Our own PID is not a postgres process.
        std::fs::write(d.join(".s.PGSQL.5999.lock"),
            format!("{}\n{}\n0\n5999\n{}\n", std::process::id(), d.join("data").display(), d.display())).unwrap();
        std::fs::write(d.join("data/postmaster.pid"), format!("{}\n", std::process::id())).unwrap();
        std::fs::write(d.join("data/PG_VERSION"), "17\n").unwrap();
        let e = discover(&d, 5999).unwrap_err();
        assert!(matches!(&e, PluginError::Untrusted { reason, .. } if reason.contains("not postgres")), "{e:?}");
        // Lock file naming another port.
        std::fs::write(d.join(".s.PGSQL.5999.lock"), format!("1\n{}\n0\n6000\n", d.join("data").display())).unwrap();
        assert!(matches!(discover(&d, 5999), Err(PluginError::Untrusted { reason, .. }) if reason.contains("port")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
