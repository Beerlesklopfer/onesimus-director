//! Native broker processes of signed plugins (docs/plans/POSTGRESQL-PHASE2.md, B).
//!
//! A broker is the one native part of a plugin package. It runs as a real
//! process outside the WASM sandbox, so it is started only when:
//! - the signed manifest (schema 2) lists it for this host's triplet,
//! - the file and every directory above it are owned by root and not
//!   writable by group or others,
//! - the SHA-256 read from the *open* descriptor matches the manifest.
//!
//! The process is started from that same descriptor (`fexecve`), never again
//! by path, so the file cannot be swapped between check and start. The child
//! gets an empty environment, the socket as fd 0 and 1, /dev/null as fd 2, no
//! other descriptors, `PR_SET_NO_NEW_PRIVS`, `PR_SET_PDEATHSIG` and its own
//! process group, and drops to the instance user (never root) before exec.

use std::ffi::CString;
use std::fs::File;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::plugin::{check_trusted, PluginError};

/// The account a broker runs as: the owner of the database instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunAs {
    pub uid: u32,
    pub gid: u32,
}

/// A running broker: its socket and process group. Dropping it stops it.
#[derive(Debug)]
pub struct Broker {
    pub socket: UnixStream,
    pid: libc::pid_t,
    reaped: bool,
}

fn refuse(path: &Path, reason: impl Into<String>) -> PluginError {
    PluginError::Untrusted { path: path.to_path_buf(), reason: reason.into() }
}

/// Starts the native part `part` of a plugin whose verified manifest is
/// `manifest`, from `plugin_dir`, as `run_as`, with `args` (argv[1..]).
pub fn launch(plugin_dir: &Path, manifest: &onesimus_plugins::Manifest, part: &str, run_as: RunAs, args: &[String])
    -> Result<Broker, PluginError> {
    let triplet = crate::config::multiarch_triplet()
        .ok_or_else(|| PluginError::Load("no multiarch triplet for this build".into()))?;
    launch_checked(plugin_dir, manifest, triplet, part, run_as, args, 0, Path::new("/"))
}

#[allow(clippy::too_many_arguments)]
fn launch_checked(plugin_dir: &Path, manifest: &onesimus_plugins::Manifest, triplet: &str, part: &str, run_as: RunAs,
    args: &[String], owner: u32, top: &Path) -> Result<Broker, PluginError> {
    let entry = manifest.native_part(triplet, part)
        .ok_or_else(|| PluginError::Load(format!("the signed manifest lists no {part} for {triplet}")))?;
    if run_as.uid == 0 {
        return Err(PluginError::Load("a broker never runs as root".into()));
    }
    // SAFETY: geteuid has no preconditions and cannot fail.
    let euid = unsafe { libc::geteuid() };
    if euid != 0 && euid != run_as.uid {
        return Err(PluginError::Load(format!("cannot switch from uid {euid} to uid {}", run_as.uid)));
    }
    let path = plugin_dir.join(part);
    check_trusted(&path, owner, false, top)?;

    // Open once; every later check and the exec use this descriptor.
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| refuse(&path, e.to_string()))?;
    let meta = file.metadata().map_err(|e| refuse(&path, e.to_string()))?;
    if !meta.is_file() || meta.uid() != owner || meta.mode() & 0o022 != 0 || meta.mode() & 0o111 == 0 {
        return Err(refuse(&path, format!("open file: uid {}, mode {:o}", meta.uid(), meta.mode() & 0o7777)));
    }
    let digest = sha256_of(&mut file).map_err(|e| refuse(&path, e.to_string()))?;
    if digest != entry.sha256 {
        return Err(refuse(&path, "SHA-256 does not match the signed manifest"));
    }
    spawn(&file, run_as, part, args).map_err(|e| PluginError::Load(format!("starting {part}: {e}")))
}

fn sha256_of(file: &mut File) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// fork + exec of the open descriptor. Everything the child needs is
/// prepared before fork; the child only makes raw system calls.
fn spawn(file: &File, run_as: RunAs, name: &str, args: &[String]) -> std::io::Result<Broker> {
    let (parent, child) = UnixStream::pair()?;
    let devnull = File::options().write(true).open("/dev/null")?;
    let argv_owned: Vec<CString> = std::iter::once(name)
        .chain(args.iter().map(String::as_str))
        .map(|a| CString::new(a).map_err(|_| std::io::Error::other("argument with NUL")))
        .collect::<Result<_, _>>()?;
    let mut argv: Vec<*const libc::c_char> = argv_owned.iter().map(|a| a.as_ptr()).collect();
    argv.push(std::ptr::null());
    let envp: [*const libc::c_char; 1] = [std::ptr::null()];
    // SAFETY: geteuid has no preconditions and cannot fail.
    let switch = unsafe { libc::geteuid() } == 0;
    let (exe, sock, null) = (file.as_raw_fd(), child.as_raw_fd(), devnull.as_raw_fd());

    // SAFETY: after fork the child calls only async-signal-safe functions
    // on data prepared before the fork, and ends in exec or _exit.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if pid == 0 {
        // SAFETY: child process; see above.
        unsafe { child_exec(exe, sock, null, switch, run_as, &argv, &envp) }
    }
    drop(child);
    Ok(Broker { socket: parent, pid, reaped: false })
}

/// The child side; never returns.
///
/// # Safety
/// Only between fork and exec, with pointers valid in the parent's image.
unsafe fn child_exec(exe: i32, sock: i32, null: i32, switch: bool, run_as: RunAs,
    argv: &[*const libc::c_char], envp: &[*const libc::c_char; 1]) -> ! {
    // SAFETY: raw system calls on valid descriptors and pointers.
    unsafe {
        let fail = |_step: u8| -> ! { libc::_exit(127) };
        // The executable must survive the descriptor cleanup below.
        let exe_fd = if exe <= 2 { libc::fcntl(exe, libc::F_DUPFD_CLOEXEC, 3) } else { exe };
        if libc::dup2(sock, 0) < 0 || libc::dup2(sock, 1) < 0 || libc::dup2(null, 2) < 0 {
            fail(1);
        }
        // Close everything from 3 up except the executable (it is O_CLOEXEC
        // and closes at exec).
        let max = libc::sysconf(libc::_SC_OPEN_MAX).clamp(64, 65536) as i32;
        for fd in 3..max {
            if fd != exe_fd {
                libc::close(fd);
            }
        }
        if libc::setpgid(0, 0) < 0
            || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) < 0
            || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0
        {
            fail(2);
        }
        let core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &core);
        if switch
            && (libc::setgroups(0, std::ptr::null()) < 0
                || libc::setgid(run_as.gid) < 0
                || libc::setuid(run_as.uid) < 0)
        {
            fail(3);
        }
        if libc::getuid() == 0 || libc::geteuid() == 0 {
            fail(4);
        }
        libc::fexecve(exe_fd, argv.as_ptr(), envp.as_ptr());
        fail(5)
    }
}

impl Broker {
    /// Stops the broker's process group: SIGTERM, after `grace` SIGKILL.
    pub fn stop(&mut self, grace: Duration) -> Option<i32> {
        if self.reaped {
            return None;
        }
        // SAFETY: signalling our own child's process group.
        unsafe { libc::kill(-self.pid, libc::SIGTERM) };
        let deadline = Instant::now() + grace;
        loop {
            let mut status = 0;
            // SAFETY: waiting for our own child.
            let r = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if r == self.pid || r < 0 {
                self.reaped = true;
                return (r == self.pid).then_some(status);
            }
            if Instant::now() >= deadline {
                // SAFETY: as above.
                unsafe {
                    libc::kill(-self.pid, libc::SIGKILL);
                    libc::waitpid(self.pid, &mut status, 0);
                }
                self.reaped = true;
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for the broker to exit by itself; its exit status.
    pub fn wait(&mut self) -> Option<i32> {
        if self.reaped {
            return None;
        }
        let mut status = 0;
        // SAFETY: waiting for our own child.
        let r = unsafe { libc::waitpid(self.pid, &mut status, 0) };
        self.reaped = true;
        (r == self.pid).then_some(status)
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.stop(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn me() -> RunAs {
        // SAFETY: getuid/getgid have no preconditions.
        unsafe { RunAs { uid: libc::getuid(), gid: libc::getgid() } }
    }
    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ofd-broker-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("plugin")).unwrap();
        for p in [&d, &d.join("plugin")] {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        d
    }
    /// A copy of a system tool as the "broker", and a manifest naming it.
    fn install(d: &Path, tool: &str) -> onesimus_plugins::Manifest {
        let target = d.join("plugin/broker");
        std::fs::copy(tool, &target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sha = sha256_of(&mut File::open(&target).unwrap()).unwrap();
        onesimus_plugins::Manifest {
            schema_version: 2,
            plugin_id: "test.broker".into(),
            version: "1".into(),
            api_version: 1,
            artifact_sha256: String::new(),
            signing_key_id: String::new(),
            not_before: 0,
            not_after: u64::MAX,
            capabilities: vec![],
            target: "wasm32-wasip2".into(),
            native: [("test-triplet".to_string(),
                vec![onesimus_plugins::NativePart { path: "broker".into(), sha256: sha }])].into(),
        }
    }
    fn start(d: &Path, m: &onesimus_plugins::Manifest, args: &[&str]) -> Result<Broker, PluginError> {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        launch_checked(&d.join("plugin"), m, "test-triplet", "broker", me(), &args, me().uid, d)
    }

    #[test]
    fn the_socket_is_stdin_and_stdout() {
        let d = dir("cat");
        let m = install(&d, "/bin/cat");
        let mut b = start(&d, &m, &[]).unwrap();
        b.socket.write_all(b"ping\n").unwrap();
        b.socket.shutdown(std::net::Shutdown::Write).unwrap();
        let mut out = String::new();
        b.socket.read_to_string(&mut out).unwrap();
        assert_eq!(out, "ping\n");
        assert_eq!(b.wait(), Some(0));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_environment_is_empty_and_no_descriptor_leaks() {
        let d = dir("env");
        let m = install(&d, "/bin/sh");
        let secret = std::fs::File::open("/etc/hostname").unwrap();
        let mut b = start(&d, &m, &["-c", "env; ls /proc/self/fd"]).unwrap();
        let mut out = String::new();
        b.socket.read_to_string(&mut out).unwrap();
        drop(secret);
        let lines: Vec<&str> = out.lines().collect();
        // sh sets PWD itself; nothing comes from the parent.
        assert!(lines.iter().all(|l| !l.contains('=') || l.starts_with("PWD=")), "{out}");
        // 0, 1, 2 and the descriptor ls opens for /proc/self/fd.
        let fds: Vec<&str> = lines.iter().copied().filter(|l| l.parse::<u32>().is_ok()).collect();
        assert!(fds.len() <= 4, "{out}");
        b.wait();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_changed_or_untrusted_file_is_not_started() {
        let d = dir("bad");
        let mut m = install(&d, "/bin/cat");
        // Wrong hash.
        let good = m.native["test-triplet"][0].sha256.clone();
        m.native.get_mut("test-triplet").unwrap()[0].sha256 = "0".repeat(64);
        assert!(matches!(start(&d, &m, &[]), Err(PluginError::Untrusted { reason, .. }) if reason.contains("SHA-256")));
        m.native.get_mut("test-triplet").unwrap()[0].sha256 = good;
        // Writable by group.
        std::fs::set_permissions(d.join("plugin/broker"), std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(matches!(start(&d, &m, &[]), Err(PluginError::Untrusted { .. })));
        std::fs::set_permissions(d.join("plugin/broker"), std::fs::Permissions::from_mode(0o755)).unwrap();
        // Writable directory above it.
        std::fs::set_permissions(d.join("plugin"), std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(matches!(start(&d, &m, &[]), Err(PluginError::Untrusted { .. })));
        std::fs::set_permissions(d.join("plugin"), std::fs::Permissions::from_mode(0o755)).unwrap();
        // Not listed for this triplet / root as instance user.
        let args: Vec<String> = Vec::new();
        assert!(matches!(launch_checked(&d.join("plugin"), &m, "other-triplet", "broker", me(), &args, me().uid, &d),
            Err(PluginError::Load(_))));
        assert!(matches!(launch_checked(&d.join("plugin"), &m, "test-triplet", "broker", RunAs { uid: 0, gid: 0 },
            &args, me().uid, &d), Err(PluginError::Load(r)) if r.contains("root")));
        // A link instead of the file.
        std::fs::rename(d.join("plugin/broker"), d.join("plugin/real")).unwrap();
        std::os::unix::fs::symlink(d.join("plugin/real"), d.join("plugin/broker")).unwrap();
        assert!(start(&d, &m, &[]).is_err(), "O_NOFOLLOW refuses a link");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn stop_ends_the_whole_process_group() {
        let d = dir("stop");
        let m = install(&d, "/bin/sh");
        let mut b = start(&d, &m, &["-c", "sleep 30 & sleep 30; wait"]).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let begin = Instant::now();
        b.stop(Duration::from_millis(500));
        assert!(begin.elapsed() < Duration::from_secs(3));
        // No member of the group is left.
        // SAFETY: probing a process group with signal 0.
        let alive = unsafe { libc::kill(-b.pid, 0) } == 0;
        assert!(!alive, "a child of the broker survived");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
