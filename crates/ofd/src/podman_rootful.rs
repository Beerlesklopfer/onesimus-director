//! The rootful Podman backend in `ofd` (docs/plans/PODMAN-PLUGIN.md,
//! "Rootful", user decision 19.11.2026, variant B).
//!
//! # Purpose
//!
//! For `user=root` there is no broker process and no plugin code runs as
//! root: `ofd` itself answers the broker requests with fixed logic. Rootful
//! storage (0700 root) and `/run/podman/podman.sock` are root-equivalent; a
//! non-root broker with capabilities would only formally not be root. The
//! backend runs nothing but the rows of the operation table
//! (`onesimus_podman_core::exec`): fixed argument lists, an empty
//! environment plus a fixed `PATH` and `LANG=C`, stdin and stderr on
//! `/dev/null`, its own process group, `RLIMIT_CORE` 0, programs checked to
//! be root-owned and not writable. Every argument list it runs goes into
//! the **exec protocol** ([`exec_log`]); the P0 test suite compares the
//! protocol with the table (exit criterion P0 (6)).
//!
//! # Data flow
//!
//! ```mermaid
//! flowchart LR
//!     G[guest] -- WIT podman --> P[PodmanPort]
//!     P -- BrokerRequest --> R[RootfulBackend]
//!     R -- exec::Operation::argv --> L[(exec protocol)]
//!     R -- Runner --> C[/usr/bin/podman, quadlet, systemctl/]
//!     C -- stdout JSON --> R
//!     R -- BrokerResponse --> P
//! ```
//!
//! # Counterpart
//!
//! `onesimus-podman-broker` answers the same requests rootless; both build
//! argument lists only through `exec::Operation::argv`.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use onesimus_podman_core::exec::{Kind, Operation, Storage};
use onesimus_podman_core::MAX_INSPECT;
use onesimus_proto_rs::onesimus::filed::broker::podman::v1 as pb;

use crate::podman_port::Backend;

/// The exec protocol of this process: every argument list the rootful
/// backend ran (or tried to run).
static EXEC_LOG: Mutex<Vec<Vec<String>>> = Mutex::new(Vec::new());

/// A copy of the exec protocol.
pub fn exec_log() -> Vec<Vec<String>> {
    EXEC_LOG.lock().map(|l| l.clone()).unwrap_or_default()
}

/// Longest single program run.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Runs one checked argument list; its exit status and stdout (bounded).
pub trait Runner: Send {
    /// Runs `argv`; Err if it could not run.
    fn run(&mut self, argv: &[String], limit: usize) -> Result<(i32, Vec<u8>), String>;
}

/// The real runner: root only, programs root-owned and not writable.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, argv: &[String], limit: usize) -> Result<(i32, Vec<u8>), String> {
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            return Err("the rootful backend runs only in ofd as root".into());
        }
        let program = Path::new(&argv[0]);
        crate::plugin::check_trusted(program, 0, false, Path::new("/")).map_err(|e| e.to_string())?;
        let mut cmd = Command::new(program);
        cmd.args(&argv[1..])
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .env("LANG", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                let core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
                libc::setrlimit(libc::RLIMIT_CORE, &core);
                Ok(())
            });
        }
        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", argv[0]))?;
        let mut out = child.stdout.take().expect("piped");
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let r = (&mut out).take(limit as u64 + 1).read_to_end(&mut buf);
            let _ = std::io::copy(&mut out, &mut std::io::sink());
            r.map(|_| buf)
        });
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("timed out".into());
                }
            }
        };
        let out = reader.join().map_err(|_| "reader failed")?.map_err(|e| e.to_string())?;
        Ok((status.code().unwrap_or(-1), out))
    }
}

/// The rootful backend: the broker requests answered by `ofd`.
pub struct RootfulBackend {
    /// How programs run (the system, or a test double).
    runner: Box<dyn Runner>,
    /// Storage options from the account policy.
    storage: Storage,
}

impl RootfulBackend {
    /// A backend with `runner` and the domain's storage options.
    pub fn new(runner: Box<dyn Runner>, storage: Storage) -> Self {
        Self { runner, storage }
    }

    /// Runs one operation; records it in the exec protocol first.
    fn run(&mut self, op: &Operation, limit: usize) -> Result<Vec<u8>, pb::BrokerResponse> {
        let fail = |code: pb::ErrorCode, m: &str, st: i32| pb::BrokerResponse {
            response: Some(pb::broker_response::Response::Error(pb::BrokerError { code: code as i32, message: m.into(), exit_status: st })),
        };
        let argv = op.argv(&self.storage).map_err(|_| fail(pb::ErrorCode::Refused, "request refused", 0))?;
        if let Ok(mut log) = EXEC_LOG.lock() {
            log.push(argv.clone());
        }
        tracing::info!(argv = ?argv, "podman rootful backend");
        let (status, out) = self.runner.run(&argv, limit).map_err(|_| fail(pb::ErrorCode::NotStarted, "program could not be started", 0))?;
        if status != 0 {
            return Err(fail(pb::ErrorCode::ToolFailed, "program failed", status));
        }
        if out.len() > limit {
            return Err(fail(pb::ErrorCode::TooLarge, "answer too large", 0));
        }
        Ok(out)
    }
}

/// The inspect kind of the proto enum.
fn kind(k: i32) -> Option<Kind> {
    Some(match pb::ObjectKind::try_from(k).ok()? {
        pb::ObjectKind::Container => Kind::Container,
        pb::ObjectKind::Pod => Kind::Pod,
        pb::ObjectKind::Network => Kind::Network,
        pb::ObjectKind::Volume => Kind::Volume,
        pb::ObjectKind::Image => Kind::Image,
        pb::ObjectKind::Secret => Kind::Secret,
        pb::ObjectKind::Unspecified => return None,
    })
}

impl Backend for RootfulBackend {
    fn request(&mut self, request: pb::BrokerRequest) -> Result<pb::BrokerResponse, String> {
        use pb::broker_request::Request;
        use pb::broker_response::Response;
        let mut answer = || -> Result<Response, pb::BrokerResponse> {
            Ok(match request.request {
                Some(Request::EngineIdentify(_)) => Response::EngineIdentify(pb::EngineIdentifyResponse {
                    info_json: self.run(&Operation::Info, MAX_INSPECT)?,
                    version_json: self.run(&Operation::Version, MAX_INSPECT)?,
                }),
                Some(Request::Inventory(_)) => Response::Inventory(pb::InventoryResponse {
                    containers_json: self.run(&Operation::Containers, MAX_INSPECT)?,
                    pods_json: self.run(&Operation::Pods, MAX_INSPECT)?,
                    networks_json: self.run(&Operation::Networks, MAX_INSPECT)?,
                    volumes_json: self.run(&Operation::Volumes, MAX_INSPECT)?,
                    images_json: self.run(&Operation::Images, MAX_INSPECT)?,
                    secrets_json: self.run(&Operation::Secrets, MAX_INSPECT)?,
                }),
                Some(Request::Inspect(r)) => {
                    let k = kind(r.kind).ok_or_else(|| refused())?;
                    let op = if r.spec { Operation::GenerateSpec(r.ids.first().cloned().unwrap_or_default()) } else { Operation::Inspect(k, r.ids) };
                    Response::Inspect(pb::InspectResponse { json: self.run(&op, MAX_INSPECT)? })
                }
                Some(Request::QuadletResolve(_)) => {
                    Response::QuadletResolve(pb::QuadletResolveResponse { output: self.run(&Operation::QuadletDryrun { user: false }, MAX_INSPECT)? })
                }
                None => return Err(refused()),
            })
        };
        Ok(match answer() {
            Ok(r) => pb::BrokerResponse { response: Some(r) },
            Err(e) => e,
        })
    }
}

/// A fixed "refused" answer.
fn refused() -> pb::BrokerResponse {
    pb::BrokerResponse {
        response: Some(pb::broker_response::Response::Error(pb::BrokerError {
            code: pb::ErrorCode::Refused as i32,
            message: "request refused".into(),
            exit_status: 0,
        })),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A runner that answers JSON and records nothing itself (the backend
    /// writes the exec protocol).
    pub(crate) struct FakeRunner(pub String);
    impl Runner for FakeRunner {
        fn run(&mut self, argv: &[String], _limit: usize) -> Result<(i32, Vec<u8>), String> {
            Ok((0, if argv.iter().any(|a| a == "info") { self.0.clone().into_bytes() } else { b"[]".to_vec() }))
        }
    }

    #[test]
    fn the_real_runner_refuses_without_root() {
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        assert!(SystemRunner.run(&["/usr/bin/podman".into(), "info".into()], 10).is_err());
    }

    #[test]
    fn requests_map_to_table_rows_only() {
        let mut b = RootfulBackend::new(Box::new(FakeRunner("{}".into())), Storage::default());
        let before = exec_log().len();
        let r = |req| pb::BrokerRequest { request: Some(req) };
        use pb::broker_request::Request;
        b.request(r(Request::EngineIdentify(pb::EngineIdentifyRequest {}))).unwrap();
        b.request(r(Request::Inventory(pb::InventoryRequest {}))).unwrap();
        b.request(r(Request::Inspect(pb::InspectRequest { kind: pb::ObjectKind::Container as i32, ids: vec!["web".into()], spec: false }))).unwrap();
        b.request(r(Request::QuadletResolve(pb::QuadletResolveRequest {}))).unwrap();
        // A value that would be an option is refused before anything runs.
        let bad = b.request(r(Request::Inspect(pb::InspectRequest { kind: 1, ids: vec!["--all".into()], spec: false }))).unwrap();
        assert!(matches!(bad.response, Some(pb::broker_response::Response::Error(_))));
        let log = exec_log();
        assert!(log.len() >= before + 10);
        for argv in &log {
            assert!(onesimus_podman_core::exec::classify(argv).is_some(), "not in the table: {argv:?}");
        }
    }
}
