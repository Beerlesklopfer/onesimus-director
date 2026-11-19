//! Broker of the signed Podman plugin (docs/plans/PODMAN-PLUGIN.md,
//! "Broker und Rootful-Backend").
//!
//! # Purpose
//!
//! `ofd` starts one broker per (job, host uid, domain), from its checked
//! descriptor (`fexecve`), as the account's uid inside a user and mount
//! namespace **owned by root** (`ofd` calls `unshare` as root and writes
//! the maps, `podman_ns.rs`). Inside the namespace the broker is uid 0 and
//! has rights only over the account's IDs, like `podman unshare`. It
//! delivers **metadata only** (`podman … --format json`,
//! `quadlet -dryrun -user`); never bytes or attributes of volume files or
//! image layers, which the host reads itself. Its first action is
//! `PR_SET_DUMPABLE(0)` (`execve` sets dumpable back to 1, V-3).
//!
//! Arguments (only from the host, from the account policy):
//!
//! ```text
//! onesimus-podman-broker [--root <graphroot>] [--runroot <dir>] [--tmpdir <dir>] [--rootless]
//! ```
//!
//! Environment (set by `ofd`, passed to podman unchanged, nothing else):
//! `HOME`, `XDG_RUNTIME_DIR`, `PATH`, `LANG` and the three internal
//! variables `_CONTAINERS_USERNS_CONFIGURED`, `_CONTAINERS_ROOTLESS_UID`,
//! `_CONTAINERS_ROOTLESS_GID` (with only one of them Podman believes it is
//! rootful, Praxis 6). Never `CONTAINERS_CONF`, `CONTAINERS_STORAGE_CONF`,
//! `QUADLET_UNIT_DIRS`, `DOCKER_HOST`.
//!
//! # Data flow
//!
//! ```mermaid
//! sequenceDiagram
//!     participant G as WASM guest
//!     participant H as ofd (PodmanPort)
//!     participant B as onesimus-podman-broker
//!     participant P as podman
//!     G->>H: engine-identify (WIT podman)
//!     H->>B: BrokerRequest{engine_identify} (frame on fd 0)
//!     B->>P: podman [storage] info --format json (fixed argv, env from the list)
//!     P-->>B: JSON (stderr discarded)
//!     B-->>H: BrokerResponse{engine_identify: raw JSON} (frame on fd 1)
//!     H->>H: check graphroot, uid map; positive list
//!     H-->>G: filtered JSON
//! ```
//!
//! # Counterpart
//!
//! `ofd::podman_port` (checks and filters every answer; the rootful
//! backend `ofd::podman_rootful` answers the same requests without a
//! broker). Argument lists come only from
//! `onesimus_podman_core::exec::Operation::argv`.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
// The error answer is a whole BrokerResponse, built once per failed request.
#![allow(clippy::result_large_err)]

use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use onesimus_podman_core::exec::{Kind, Operation, Storage};
use onesimus_podman_core::{read_frame, write_frame, MAX_FRAME, MAX_INSPECT};
use onesimus_proto_rs::onesimus::filed::broker::podman::v1 as pb;
use prost::Message;

/// The only environment variables passed to podman.
const ENV_PASS: &[&str] = &[
    "HOME",
    "XDG_RUNTIME_DIR",
    "PATH",
    "LANG",
    "_CONTAINERS_USERNS_CONFIGURED",
    "_CONTAINERS_ROOTLESS_UID",
    "_CONTAINERS_ROOTLESS_GID",
];

/// Longest single podman run.
const TOOL_TIMEOUT: Duration = Duration::from_secs(120);

/// Largest output of a listing (inventory part, info).
const MAX_LISTING: usize = MAX_INSPECT;

/// The broker's settings from its arguments.
#[derive(Debug, Clone, Default)]
struct Settings {
    /// Storage options of the domain.
    storage: Storage,
    /// Rootless profile: `quadlet -dryrun -user`.
    rootless: bool,
}

/// Parses the host's arguments; anything unknown ends the broker.
fn settings(args: &[String]) -> Result<Settings, String> {
    let mut s = Settings::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut path = || -> Result<Option<PathBuf>, String> {
            let p = PathBuf::from(it.next().ok_or(format!("{a} needs a value"))?);
            if !p.is_absolute() {
                return Err(format!("{a} must be absolute"));
            }
            Ok(Some(p))
        };
        match a.as_str() {
            "--root" => s.storage.root = path()?,
            "--runroot" => s.storage.runroot = path()?,
            "--tmpdir" => s.storage.tmpdir = path()?,
            "--rootless" => s.rootless = true,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(s)
}

/// A fixed error answer.
fn error(code: pb::ErrorCode, message: &str, exit_status: i32) -> pb::BrokerResponse {
    pb::BrokerResponse {
        response: Some(pb::broker_response::Response::Error(pb::BrokerError { code: code as i32, message: message.into(), exit_status })),
    }
}

/// Runs one operation; its stdout up to `limit`. stderr is discarded, never
/// passed on (it may quote values, V-6).
fn run(settings: &Settings, op: &Operation, limit: usize) -> Result<Vec<u8>, pb::BrokerResponse> {
    let argv = op.argv(&settings.storage).map_err(|_| error(pb::ErrorCode::Refused, "request refused", 0))?;
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).env_clear().stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    for name in ENV_PASS {
        if let Some(v) = std::env::var_os(name) {
            cmd.env(name, v);
        }
    }
    let mut child = cmd.spawn().map_err(|_| error(pb::ErrorCode::NotStarted, "podman could not be started", 0))?;
    let mut stdout = child.stdout.take().expect("piped");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let r = (&mut stdout).take(limit as u64 + 1).read_to_end(&mut out);
        // Drain the rest so the tool does not block on a full pipe.
        let _ = std::io::copy(&mut stdout, &mut std::io::sink());
        r.map(|_| out)
    });
    let deadline = Instant::now() + TOOL_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error(pb::ErrorCode::ToolFailed, "podman timed out", -1));
            }
        }
    };
    let out = reader.join().map_err(|_| error(pb::ErrorCode::ToolFailed, "reading podman failed", 0))?
        .map_err(|_| error(pb::ErrorCode::ToolFailed, "reading podman failed", 0))?;
    if !status.success() {
        return Err(error(pb::ErrorCode::ToolFailed, "podman failed", status.code().unwrap_or(-1)));
    }
    if out.len() > limit {
        return Err(error(pb::ErrorCode::TooLarge, "answer too large", 0));
    }
    Ok(out)
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

/// Answers one request.
fn handle(settings: &Settings, request: pb::BrokerRequest) -> pb::BrokerResponse {
    use pb::broker_request::Request;
    use pb::broker_response::Response;
    let answer = || -> Result<Response, pb::BrokerResponse> {
        Ok(match request.request {
            Some(Request::EngineIdentify(_)) => Response::EngineIdentify(pb::EngineIdentifyResponse {
                info_json: run(settings, &Operation::Info, MAX_LISTING)?,
                version_json: run(settings, &Operation::Version, MAX_LISTING)?,
            }),
            Some(Request::Inventory(_)) => Response::Inventory(pb::InventoryResponse {
                containers_json: run(settings, &Operation::Containers, MAX_LISTING)?,
                pods_json: run(settings, &Operation::Pods, MAX_LISTING)?,
                networks_json: run(settings, &Operation::Networks, MAX_LISTING)?,
                volumes_json: run(settings, &Operation::Volumes, MAX_LISTING)?,
                images_json: run(settings, &Operation::Images, MAX_LISTING)?,
                secrets_json: run(settings, &Operation::Secrets, MAX_LISTING)?,
            }),
            Some(Request::Inspect(r)) => {
                let k = kind(r.kind).ok_or_else(|| error(pb::ErrorCode::Refused, "unknown kind", 0))?;
                let op = if r.spec {
                    if !matches!(k, Kind::Container | Kind::Pod) || r.ids.len() != 1 {
                        return Err(error(pb::ErrorCode::Refused, "spec of one container or pod", 0));
                    }
                    Operation::GenerateSpec(r.ids[0].clone())
                } else {
                    Operation::Inspect(k, r.ids)
                };
                Response::Inspect(pb::InspectResponse { json: run(settings, &op, MAX_INSPECT)? })
            }
            Some(Request::QuadletResolve(_)) => Response::QuadletResolve(pb::QuadletResolveResponse {
                output: run(settings, &Operation::QuadletDryrun { user: settings.rootless }, MAX_LISTING)?,
            }),
            None => return Err(error(pb::ErrorCode::Refused, "empty request", 0)),
        })
    };
    match answer() {
        Ok(r) => pb::BrokerResponse { response: Some(r) },
        Err(e) => e,
    }
}

/// Serves frames until the host closes the socket.
fn serve(settings: &Settings, input: &mut impl Read, output: &mut impl Write) -> std::io::Result<()> {
    while let Some(frame) = read_frame(input)? {
        let response = match pb::BrokerRequest::decode(frame.as_slice()) {
            Ok(r) => handle(settings, r),
            Err(_) => error(pb::ErrorCode::Refused, "malformed request", 0),
        };
        let mut bytes = response.encode_to_vec();
        if bytes.len() > MAX_FRAME {
            bytes = error(pb::ErrorCode::TooLarge, "answer too large", 0).encode_to_vec();
        }
        write_frame(output, &bytes)?;
    }
    Ok(())
}

fn main() {
    // First action (V-3): not dumpable, so the account cannot ptrace or read
    // /proc/<pid>/mem of the broker even inside the namespace.
    // SAFETY: prctl with constant arguments.
    unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let settings = match settings(&args) {
        Ok(s) => s,
        Err(_) => std::process::exit(2),
    };
    // SAFETY: fd 0 and 1 are the socket the host passed; owned from here on.
    let (mut input, mut output) = unsafe { (std::fs::File::from_raw_fd(0), std::fs::File::from_raw_fd(1)) };
    if serve(&settings, &mut input, &mut output).is_err() {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_strict() {
        let s = settings(&["--root".into(), "/r".into(), "--rootless".into()]).unwrap();
        assert_eq!(s.storage.root, Some(PathBuf::from("/r")));
        assert!(s.rootless);
        assert!(settings(&["--root".into(), "rel".into()]).is_err());
        assert!(settings(&["--root".into()]).is_err());
        assert!(settings(&["--env".into(), "X=1".into()]).is_err());
    }

    #[test]
    fn malformed_and_refused_requests_get_fixed_errors() {
        let s = Settings::default();
        let mut input = Vec::new();
        write_frame(&mut input, b"\xff\xff\xff").unwrap();
        let bad = pb::BrokerRequest {
            request: Some(pb::broker_request::Request::Inspect(pb::InspectRequest { kind: 1, ids: vec!["--all".into()], spec: false })),
        };
        write_frame(&mut input, &bad.encode_to_vec()).unwrap();
        let spec = pb::BrokerRequest {
            request: Some(pb::broker_request::Request::Inspect(pb::InspectRequest { kind: 3, ids: vec!["n".into()], spec: true })),
        };
        write_frame(&mut input, &spec.encode_to_vec()).unwrap();
        let mut output = Vec::new();
        serve(&s, &mut input.as_slice(), &mut output).unwrap();
        let mut r = output.as_slice();
        for _ in 0..3 {
            let resp = pb::BrokerResponse::decode(read_frame(&mut r).unwrap().unwrap().as_slice()).unwrap();
            let Some(pb::broker_response::Response::Error(e)) = resp.response else { panic!("{resp:?}") };
            assert_eq!(e.code, pb::ErrorCode::Refused as i32);
        }
    }
}
