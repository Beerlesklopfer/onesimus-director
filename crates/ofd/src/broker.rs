// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
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

#[cfg(target_os = "linux")]
use std::ffi::CString;
use std::fs::File;
use std::io::Read;
#[cfg(target_os = "linux")]
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

/// The socket to a broker (one end of a Unix stream pair).
pub type BrokerSocket = UnixStream;

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
pub(crate) fn launch_checked(plugin_dir: &Path, manifest: &onesimus_plugins::Manifest, triplet: &str, part: &str, run_as: RunAs,
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
    let file = open_part(plugin_dir, entry, part, owner, top)?;
    spawn(&file, run_as, part, args).map_err(|e| PluginError::Load(format!("starting {part}: {e}")))
}

/// Opens the native part once, checks owner, mode and the signed SHA-256
/// on the open descriptor; every later step uses this descriptor.
fn open_part(plugin_dir: &Path, entry: &onesimus_plugins::NativePart, part: &str, owner: u32, top: &Path)
    -> Result<File, PluginError> {
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
    Ok(file)
}

/// The user namespace of a rootless Podman broker
/// (docs/plans/PODMAN-PLUGIN.md, "Namespace und Start").
#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct UserNs {
    /// Text for `/proc/<pid>/uid_map` (`0 <uid> 1`, `1 <subuid> <count>`).
    pub uid_map: String,
    /// Text for `/proc/<pid>/gid_map`.
    pub gid_map: String,
    /// The broker's environment (HOME, XDG_RUNTIME_DIR, PATH, LANG and the
    /// three `_CONTAINERS_*` variables), nothing else.
    pub env: Vec<(String, String)>,
}

/// Starts a native part inside a user and mount namespace that **root
/// owns**: the child calls `unshare(CLONE_NEWUSER | CLONE_NEWNS)` while
/// still root, `ofd` writes the maps from `/etc/subuid`/`/etc/subgid`, the
/// child switches to ns-UID/GID 0 (= the account), drops supplementary
/// groups, makes its mounts private, sets `PR_SET_NO_NEW_PRIVS`,
/// `PR_SET_PDEATHSIG`, `RLIMIT_CORE` 0 and execs the checked descriptor.
/// The account has no capability in a namespace it does not own, so it
/// cannot `ptrace` the broker (V-3); the broker also sets
/// `PR_SET_DUMPABLE(0)` itself. A pause process of the account is never
/// joined: its namespace belongs to the account.
#[allow(clippy::too_many_arguments)]
pub(crate) fn launch_in_userns(plugin_dir: &Path, manifest: &onesimus_plugins::Manifest, triplet: &str, part: &str,
    ns: &UserNs, args: &[String], owner: u32, top: &Path) -> Result<Broker, PluginError> {
    let entry = manifest.native_part(triplet, part)
        .ok_or_else(|| PluginError::Load(format!("the signed manifest lists no {part} for {triplet}")))?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } != 0 {
        return Err(PluginError::Load("a root-owned user namespace needs ofd as root".into()));
    }
    if ns.uid_map.lines().next().is_some_and(|l| l.split_whitespace().nth(1) == Some("0")) {
        return Err(PluginError::Load("a broker never runs as root".into()));
    }
    let file = open_part(plugin_dir, entry, part, owner, top)?;
    spawn_userns(&file, part, args, ns).map_err(|e| PluginError::Load(format!("starting {part}: {e}")))
}

/// fork; the child unshares as root and waits; the parent writes the maps;
/// the child switches to ns-root and execs. Everything the child needs is
/// prepared before fork.
#[cfg(target_os = "linux")]
fn spawn_userns(file: &File, name: &str, args: &[String], ns: &UserNs) -> std::io::Result<Broker> {
    let (parent, child) = UnixStream::pair()?;
    let devnull = File::options().write(true).open("/dev/null")?;
    let cstr = |s: &str| CString::new(s).map_err(|_| std::io::Error::other("NUL"));
    let argv_owned: Vec<CString> = std::iter::once(name).chain(args.iter().map(String::as_str)).map(cstr).collect::<Result<_, _>>()?;
    let mut argv: Vec<*const libc::c_char> = argv_owned.iter().map(|a| a.as_ptr()).collect();
    argv.push(std::ptr::null());
    let env_owned: Vec<CString> = ns.env.iter().map(|(k, v)| cstr(&format!("{k}={v}"))).collect::<Result<_, _>>()?;
    let mut envp: Vec<*const libc::c_char> = env_owned.iter().map(|e| e.as_ptr()).collect();
    envp.push(std::ptr::null());
    // Two pipes: child -> parent "unshared", parent -> child "maps written".
    let (mut ready, mut go) = ([0i32; 2], [0i32; 2]);
    // SAFETY: arrays of two descriptors.
    if unsafe { libc::pipe2(ready.as_mut_ptr(), libc::O_CLOEXEC) } < 0 || unsafe { libc::pipe2(go.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (exe, sock, null) = (file.as_raw_fd(), child.as_raw_fd(), devnull.as_raw_fd());
    // SAFETY: the child only makes raw system calls on prepared data.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if pid == 0 {
        // SAFETY: child process between fork and exec.
        unsafe { child_userns(exe, sock, null, ready[1], go[0], &argv, &envp) }
    }
    // SAFETY: closing the child's ends in the parent.
    unsafe {
        libc::close(ready[1]);
        libc::close(go[0]);
    }
    let mut byte = [0u8; 1];
    // SAFETY: reading one byte from our pipe end.
    let n = unsafe { libc::read(ready[0], byte.as_mut_ptr().cast(), 1) };
    let result = (|| {
        if n != 1 {
            return Err(std::io::Error::other("the broker child did not unshare"));
        }
        std::fs::write(format!("/proc/{pid}/uid_map"), &ns.uid_map)?;
        std::fs::write(format!("/proc/{pid}/gid_map"), &ns.gid_map)?;
        // SAFETY: writing one byte to our pipe end.
        if unsafe { libc::write(go[1], b"g".as_ptr().cast(), 1) } != 1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    })();
    // SAFETY: closing our pipe ends.
    unsafe {
        libc::close(ready[0]);
        libc::close(go[1]);
    }
    drop(child);
    let broker = Broker { socket: parent, pid, reaped: false };
    result.map(|_| broker)
}

/// The child side of `spawn_userns`; never returns.
///
/// # Safety
/// Only between fork and exec, with pointers valid in the parent's image.
#[cfg(target_os = "linux")]
unsafe fn child_userns(exe: i32, sock: i32, null: i32, ready: i32, go: i32, argv: &[*const libc::c_char],
    envp: &[*const libc::c_char]) -> ! {
    // SAFETY: raw system calls on valid descriptors and pointers.
    unsafe {
        let fail = |_step: u8| -> ! { libc::_exit(127) };
        let keep = |fd: i32| if fd <= 2 { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) } else { fd };
        let (exe_fd, ready, go) = (keep(exe), keep(ready), keep(go));
        if libc::dup2(sock, 0) < 0 || libc::dup2(sock, 1) < 0 || libc::dup2(null, 2) < 0 {
            fail(1);
        }
        let max = libc::sysconf(libc::_SC_OPEN_MAX).clamp(64, 65536) as i32;
        for fd in 3..max {
            if fd != exe_fd && fd != ready && fd != go {
                libc::close(fd);
            }
        }
        if libc::setpgid(0, 0) < 0 || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) < 0 {
            fail(2);
        }
        // Still root: the new user namespace belongs to root.
        if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNS) < 0 {
            fail(3);
        }
        let mut b = [0u8; 1];
        if libc::write(ready, b"r".as_ptr().cast(), 1) != 1 || libc::read(go, b.as_mut_ptr().cast(), 1) != 1 {
            fail(4);
        }
        libc::close(ready);
        libc::close(go);
        // Mounts of podman stay in this namespace.
        if libc::mount(std::ptr::null(), c"/".as_ptr(), std::ptr::null(), libc::MS_REC | libc::MS_PRIVATE, std::ptr::null()) < 0 {
            fail(5);
        }
        if libc::setgroups(0, std::ptr::null()) < 0 || libc::setresgid(0, 0, 0) < 0 || libc::setresuid(0, 0, 0) < 0 {
            fail(6);
        }
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) < 0 {
            fail(7);
        }
        let core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        libc::setrlimit(libc::RLIMIT_CORE, &core);
        libc::fexecve(exe_fd, argv.as_ptr(), envp.as_ptr());
        fail(8)
    }
}

/// User namespaces exist only on Linux: no broker on other platforms.
#[cfg(not(target_os = "linux"))]
fn spawn_userns(_file: &File, _name: &str, _args: &[String], _ns: &UserNs) -> std::io::Result<Broker> {
    Err(unsupported())
}

/// The error of a broker start on a platform without the Linux process
/// controls the broker relies on (`PR_SET_NO_NEW_PRIVS`,
/// `PR_SET_PDEATHSIG`, `fexecve`, namespaces).
#[cfg(not(target_os = "linux"))]
fn unsupported() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Unsupported, "native plugin brokers are not supported on this platform")
}

/// For `ofd --check-plugin`: every native part the verified manifest lists
/// for `triplet` is a regular file (not a link) below `plugin_dir` with the
/// signed SHA-256. Returns how many parts were checked. Nothing is started.
pub(crate) fn check_parts(plugin_dir: &Path, manifest: &onesimus_plugins::Manifest, triplet: &str)
    -> Result<usize, PluginError> {
    let Some(parts) = manifest.native.get(triplet) else { return Ok(0) };
    for part in parts {
        let path = plugin_dir.join(&part.path);
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|e| refuse(&path, e.to_string()))?;
        let meta = file.metadata().map_err(|e| refuse(&path, e.to_string()))?;
        if !meta.is_file() {
            return Err(refuse(&path, "is not a regular file"));
        }
        if sha256_of(&mut file).map_err(|e| refuse(&path, e.to_string()))? != part.sha256 {
            return Err(refuse(&path, "SHA-256 does not match the signed manifest"));
        }
    }
    Ok(parts.len())
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
#[cfg(target_os = "linux")]
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
    // The child makes itself a group leader too; doing it here as well
    // means kill(-pid) reaches it even before the child got that far
    // (otherwise `stop` would wait out its grace for nothing). Errors are
    // harmless: EACCES once the child has exec'd, by then it did it itself.
    // SAFETY: setpgid on our own fresh child.
    unsafe { libc::setpgid(pid, pid) };
    drop(child);
    Ok(Broker { socket: parent, pid, reaped: false })
}

/// The child side; never returns.
///
/// # Safety
/// Only between fork and exec, with pointers valid in the parent's image.
#[cfg(target_os = "linux")]
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

/// Brokers need Linux process controls; see [`unsupported`].
#[cfg(not(target_os = "linux"))]
fn spawn(_file: &File, _run_as: RunAs, _name: &str, _args: &[String]) -> std::io::Result<Broker> {
    Err(unsupported())
}

impl Broker {
    /// The broker's process id (also its process group).
    #[cfg(test)]
    pub fn pid(&self) -> libc::pid_t {
        self.pid
    }

    /// Stops the broker's process group: SIGTERM, after `grace` SIGKILL.
    /// The broker's wait status, or None when it is unknown (already reaped
    /// elsewhere, or waitpid failed; then it is not marked as reaped).
    pub fn stop(&mut self, grace: Duration) -> Option<i32> {
        if self.reaped {
            return None;
        }
        // SAFETY: signalling our own child's process group.
        unsafe { libc::kill(-self.pid, libc::SIGTERM) };
        let deadline = Instant::now() + grace;
        loop {
            match self.wait_flags(libc::WNOHANG) {
                Waited::Exited(status) => return Some(status),
                Waited::Gone => return None,
                Waited::Running | Waited::Failed => {}
            }
            if Instant::now() >= deadline {
                // SAFETY: as above.
                unsafe { libc::kill(-self.pid, libc::SIGKILL) };
                return match self.wait_flags(0) {
                    Waited::Exited(status) => Some(status),
                    Waited::Gone | Waited::Running | Waited::Failed => None,
                };
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Waits for the broker to exit by itself; its exit status (None when
    /// unknown, see `stop`).
    pub fn wait(&mut self) -> Option<i32> {
        if self.reaped {
            return None;
        }
        match self.wait_flags(0) {
            Waited::Exited(status) => Some(status),
            Waited::Gone | Waited::Running | Waited::Failed => None,
        }
    }

    /// One waitpid on the broker with `flags`, retried on EINTR; marks the
    /// broker reaped exactly when it was (its status, or ECHILD).
    fn wait_flags(&mut self, flags: i32) -> Waited {
        let pid = self.pid;
        let waited = wait_retrying(pid, |status| {
            // SAFETY: waiting for our own child.
            let r = unsafe { libc::waitpid(pid, status, flags) };
            if r < 0 { Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(0)) } else { Ok(r) }
        });
        if matches!(waited, Waited::Exited(_) | Waited::Gone) {
            self.reaped = true;
        }
        waited
    }
}

/// What one waitpid on the broker found.
#[derive(Debug, PartialEq, Eq)]
enum Waited {
    /// Reaped now, with this wait status.
    Exited(i32),
    /// Still running (WNOHANG).
    Running,
    /// ECHILD: no such child any more (reaped elsewhere); status unknown.
    Gone,
    /// Any other error: neither reaped nor known to be running.
    Failed,
}

/// Classifies `call` (a waitpid for `pid`: the returned pid or the errno),
/// calling it again as long as it is interrupted (EINTR).
fn wait_retrying(pid: libc::pid_t, mut call: impl FnMut(&mut i32) -> Result<libc::pid_t, i32>) -> Waited {
    loop {
        let mut status = 0;
        return match call(&mut status) {
            Ok(r) if r == pid => Waited::Exited(status),
            Ok(_) => Waited::Running,
            Err(libc::EINTR) => continue,
            Err(libc::ECHILD) => Waited::Gone,
            Err(_) => Waited::Failed,
        };
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
            schema_version: 3,
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
            suite: Some("0.0.0".into()),
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

    /// waitpid results: EINTR is retried, only the child's pid or ECHILD
    /// count as reaped, and no error is mistaken for an exit status.
    #[test]
    fn wait_retries_on_eintr_and_classifies_the_rest() {
        let mut answers = vec![Err(libc::EINTR), Err(libc::EINTR), Ok(42)].into_iter();
        let mut calls = 0;
        let waited = wait_retrying(42, |status| {
            calls += 1;
            *status = 9;
            answers.next().unwrap()
        });
        assert_eq!((waited, calls), (Waited::Exited(9), 3));
        assert_eq!(wait_retrying(42, |_| Ok(0)), Waited::Running);
        assert_eq!(wait_retrying(42, |_| Err(libc::ECHILD)), Waited::Gone);
        assert_eq!(wait_retrying(42, |_| Err(libc::EINVAL)), Waited::Failed);
    }

    /// A broker that ignores SIGTERM is killed; `stop` reports the real
    /// status (killed by SIGKILL), not a clean exit.
    #[test]
    fn stop_reports_the_status_after_sigkill() {
        let d = dir("kill");
        let m = install(&d, "/bin/sh");
        let mut b = start(&d, &m, &["-c", "trap '' TERM; sleep 30"]).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let status = b.stop(Duration::from_millis(300)).expect("a status");
        assert!(libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGKILL, "status {status:#x}");
        assert_eq!(b.stop(Duration::from_millis(10)), None, "already reaped");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A broker reaped elsewhere (ECHILD) counts as reaped without a status.
    #[test]
    fn stop_after_the_child_was_reaped_elsewhere() {
        let d = dir("echild");
        let m = install(&d, "/bin/sh");
        let mut b = start(&d, &m, &["-c", "exit 3"]).unwrap();
        let mut status = 0;
        // SAFETY: waiting for our own child.
        assert_eq!(unsafe { libc::waitpid(b.pid, &mut status, 0) }, b.pid);
        let begin = Instant::now();
        assert_eq!(b.stop(Duration::from_secs(2)), None);
        assert!(b.reaped);
        assert!(begin.elapsed() < Duration::from_secs(1), "no grace wait for a gone child");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The parent makes the broker a group leader itself: right after
    /// spawn its group is its pid, and an immediate stop ends it at once.
    #[test]
    fn the_broker_leads_its_group_from_the_start() {
        let d = dir("pgid");
        let m = install(&d, "/bin/sh");
        let mut b = start(&d, &m, &["-c", "sleep 30"]).unwrap();
        // SAFETY: getpgid on our own child.
        assert_eq!(unsafe { libc::getpgid(b.pid) }, b.pid);
        let begin = Instant::now();
        let status = b.stop(Duration::from_secs(2)).expect("a status");
        assert!(begin.elapsed() < Duration::from_secs(1), "stop took {:?}", begin.elapsed());
        assert!(libc::WIFSIGNALED(status), "status {status:#x}");
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
        // No living member of the group is left. Zombies do not count: an
        // orphaned grandchild goes to PID 1, and in a container (CI) PID 1
        // may never reap it, so `kill(-pgid, 0)` would still see it.
        let living = |pgid: i32| -> Vec<String> {
            std::fs::read_dir("/proc").into_iter().flatten().flatten().filter_map(|e| {
                let stat = std::fs::read_to_string(e.path().join("stat")).ok()?;
                // pid (comm) state ppid pgrp …; comm may contain spaces
                let rest = &stat[stat.rfind(')')? + 2..];
                let mut f = rest.split(' ');
                let state = f.next()?;
                let pgrp: i32 = f.nth(1)?.parse().ok()?;
                (pgrp == pgid && state != "Z" && state != "X").then(|| stat)
            }).collect()
        };
        let survivors = living(b.pid);
        assert!(survivors.is_empty(), "a child of the broker survived: {survivors:?}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
