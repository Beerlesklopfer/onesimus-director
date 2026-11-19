//! The host side of the Podman imports (docs/plans/PODMAN-PLUGIN.md,
//! "Zusammenspiel von Gast, Host und Broker").
//!
//! # Purpose
//!
//! All decisions that concern security are made here, in the host, never
//! in the guest or the broker: the plugin line is checked
//! (`onesimus_podman_core::line`), the account must be released in the
//! root-owned account policy, the grants the line needs must be in the
//! verified manifest, and the Podman imports are linked only for the
//! plugin id `org.onesimus.podman` with contract 2 (V-11, fixed in this
//! code in addition to `onesimus_plugins::verify`). Rootless lines get a
//! broker as the account in a root-owned user namespace
//! (`broker::launch_in_userns`); `user=root` gets the rootful backend in
//! `ofd` (`podman_rootful`), never a broker. Every answer of the broker is
//! the account's claim: the reported graphroot must be the policy's (path
//! and inode), and the guest only sees the positive-list filtered JSON.
//! Volume data is scanned by the host itself (`podman_scan`); the guest
//! gets references.
//!
//! Phase P0 implements `engine-identify`, `inventory` and `volume-scan`;
//! `inspect` and `member-state` answer "not before P1". A line with
//! `consistency=stop` is refused until the lifecycle (P2) exists: it never
//! falls back silently to a copy while running.
//!
//! # Data flow
//!
//! ```mermaid
//! flowchart TB
//!     L[Plugin line podman:user=…] --> C{prepare}
//!     C -- plugin id, api_version, grants --> X[plan error]
//!     C -- account policy, subuid/subgid --> D[Domain + IdMap]
//!     D -- user=root --> R[RootfulBackend in ofd]
//!     D -- rootless --> B[broker in root-owned userns]
//!     R --> P[PodmanPort]
//!     B --> P
//!     P -- engine-identify: graphroot path + inode --> P
//!     G[guest] -- engine-identify / inventory --> P
//!     P -- positive lists --> G
//!     G -- volume-scan --> P
//!     P -- podman_scan::scan_volume --> S[(ScanTable)]
//!     S -- references --> G
//! ```
//!
//! ```mermaid
//! classDiagram
//!     class Backend { <<trait>> request(BrokerRequest) BrokerResponse }
//!     class BrokerClient { frames over the broker socket }
//!     class RootfulBackend { exec protocol }
//!     class PodmanPort { line; account; domain; graphroot; scans }
//!     Backend <|.. BrokerClient
//!     Backend <|.. RootfulBackend
//!     PodmanPort --> Backend
//!     PodmanPort --> ScanTable
//! ```
//!
//! # Counterpart
//!
//! The guest (`plugins/podman`, WIT world `podman-plugin`) and
//! `onesimus-podman-broker`.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use onesimus_plugins::wasm::{BrokerFunction, BrokerPort, PodmanFunction, PODMAN_INVENTORY, PODMAN_VOLUME_READ};
use onesimus_podman_core::exec::Storage;
use onesimus_podman_core::idmap::{self, IdMap};
use onesimus_podman_core::line::{Account, Consistency, PodmanLine};
use onesimus_podman_core::policy::{AccountEntry, AccountPolicy};
use onesimus_podman_core::positive;
use onesimus_podman_core::sanitize::Domain;
use onesimus_podman_core::{read_frame, write_frame, MAX_INSPECT};
use onesimus_proto_rs::onesimus::filed::broker::podman::v1 as pb;
use prost::Message;

use crate::broker::UserNs;
use crate::plugin::{check_trusted, BrokerContext, PluginError, PluginLine, PreparedBroker};
use crate::podman_scan::{self, ScanTable, SharedScans};

/// The broker's path below the plugin directory.
pub const BROKER_PART: &str = "podman/onesimus-podman-broker";

/// Encoded scan entries per answer (fits the guest's broker answer limit,
/// chunk + 64 KiB, chunk at least 4 KiB).
const PAGE_BYTES: usize = 60 * 1024;
/// Entries per answer (the plan's limit).
const PAGE_ENTRIES: usize = 1024;

/// One request/response exchange with whatever answers broker requests.
pub trait Backend: Send {
    /// Sends `request`, returns the answer.
    fn request(&mut self, request: pb::BrokerRequest) -> Result<pb::BrokerResponse, String>;
}

/// The rootless broker over its socket (4-byte length + message).
pub struct BrokerClient<T: Read + Write + Send> {
    /// The socket.
    pub io: T,
}

impl<T: Read + Write + Send> Backend for BrokerClient<T> {
    fn request(&mut self, request: pb::BrokerRequest) -> Result<pb::BrokerResponse, String> {
        write_frame(&mut self.io, &request.encode_to_vec()).map_err(|e| format!("broker: {e}"))?;
        let frame = read_frame(&mut self.io).map_err(|e| format!("broker: {e}"))?.ok_or("broker closed the connection")?;
        pb::BrokerResponse::decode(frame.as_slice()).map_err(|_| "broker sent a malformed answer".into())
    }
}

/// Paths the host reads; production uses [`HostFiles::system`].
#[derive(Debug, Clone)]
pub struct HostFiles {
    /// The account policy.
    pub policy: PathBuf,
    /// `/etc/subuid`.
    pub subuid: PathBuf,
    /// `/etc/subgid`.
    pub subgid: PathBuf,
    /// Required owner of the policy (root).
    pub owner: u32,
    /// Top of the ownership walk (`/`).
    pub top: PathBuf,
}

impl HostFiles {
    /// The production paths.
    pub fn system() -> Self {
        Self {
            policy: PathBuf::from(onesimus_podman_core::policy::DEFAULT_PATH),
            subuid: PathBuf::from("/etc/subuid"),
            subgid: PathBuf::from("/etc/subgid"),
            owner: 0,
            top: PathBuf::from("/"),
        }
    }
}

/// A plan error (nothing ran).
fn plan(why: impl Into<String>) -> PluginError {
    PluginError::Load(format!("podman: {}", why.into()))
}

/// The domain id of an account: `root` or `uid-<n>`.
pub fn domain_id(account: &AccountEntry) -> String {
    if account.uid == 0 { "root".into() } else { format!("uid-{}", account.uid) }
}

/// The ID maps of an account (identity for root).
pub fn domain_of(account: &AccountEntry, files: &HostFiles) -> Result<(Domain, u32), PluginError> {
    if account.uid == 0 {
        return Ok((Domain::rootful(), 0));
    }
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| plan(format!("{}: {e}", p.display())));
    let gid = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(account.uid)).ok().flatten().map(|u| u.gid.as_raw())
        .ok_or_else(|| plan(format!("account {} has no passwd entry", account.name)))?;
    let uranges = idmap::sub_ranges(&read(&files.subuid)?, &account.name, account.uid).map_err(plan)?;
    let granges = idmap::sub_ranges(&read(&files.subgid)?, &account.name, account.uid).map_err(plan)?;
    if uranges.is_empty() || granges.is_empty() {
        return Err(plan(format!("account {} has no sub-IDs", account.name)));
    }
    let uid_map = IdMap::rootless(account.uid, &uranges[..1]).map_err(plan)?;
    let gid_map = IdMap::rootless(gid, &granges[..1]).map_err(plan)?;
    Ok((Domain { uid_map, gid_map, rootless: true }, gid))
}

/// The host side of the Podman imports for one plugin line.
pub struct PodmanPort {
    /// Broker or rootful backend.
    backend: Box<dyn Backend>,
    /// The checked line.
    line: PodmanLine,
    /// The released account.
    account: AccountEntry,
    /// Its ID maps.
    domain: Domain,
    /// `root` / `uid-<n>`.
    domain_id: String,
    /// The policy's graphroot, opened by the host.
    graphroot: OwnedFd,
    /// Grants of the verified manifest.
    granted: BTreeSet<String>,
    /// Host scans handed to the session.
    scans: SharedScans,
    /// engine-identify passed the host checks.
    confirmed: bool,
}

impl PodmanPort {
    /// A port; `confirm` runs engine-identify and checks the graphroot.
    pub fn new(backend: Box<dyn Backend>, line: PodmanLine, account: AccountEntry, domain: Domain, graphroot: OwnedFd,
        granted: &[String]) -> Self {
        let domain_id = domain_id(&account);
        Self {
            backend,
            line,
            account,
            domain,
            domain_id,
            graphroot,
            granted: granted.iter().cloned().collect(),
            scans: Arc::new(Mutex::new(ScanTable::default())),
            confirmed: false,
        }
    }

    /// The shared scan table (for the session).
    pub fn scans(&self) -> SharedScans {
        self.scans.clone()
    }

    /// Sends a request; a broker error becomes a fixed text.
    fn ask(&mut self, request: pb::broker_request::Request) -> Result<pb::broker_response::Response, String> {
        match self.backend.request(pb::BrokerRequest { request: Some(request) })?.response {
            Some(pb::broker_response::Response::Error(e)) => Err(format!("podman: {} (exit {})", e.message, e.exit_status)),
            Some(r) => Ok(r),
            None => Err("podman: empty answer".into()),
        }
    }

    /// engine-identify with the host checks (graphroot path and inode,
    /// rootless flag, uid map); the raw answer.
    pub fn confirm(&mut self) -> Result<pb::EngineIdentifyResponse, String> {
        let Ok(pb::broker_response::Response::EngineIdentify(r)) = self.ask(pb::broker_request::Request::EngineIdentify(pb::EngineIdentifyRequest {})) else {
            return Err("engine-identify failed".into());
        };
        let info: serde_json::Value = serde_json::from_slice(&r.info_json).map_err(|_| "podman info is not JSON")?;
        let reported = info.pointer("/store/graphRoot").and_then(|v| v.as_str()).ok_or("podman info has no graphRoot")?;
        let expected = &self.account.domains[0].graphroot;
        if Path::new(reported) != expected {
            return Err(format!("graphroot {reported} is not the policy's {}", expected.display()));
        }
        let theirs = podman_scan::open_graphroot(Path::new(reported))?;
        let st = |fd: &OwnedFd| -> Result<(u64, u64), String> {
            let m = std::fs::metadata(format!("/proc/self/fd/{}", fd.as_raw_fd())).map_err(|e| e.to_string())?;
            use std::os::unix::fs::MetadataExt;
            Ok((m.dev(), m.ino()))
        };
        if st(&theirs)? != st(&self.graphroot)? {
            return Err("graphroot inode differs from the policy's".into());
        }
        let rootless = info.pointer("/host/security/rootless").and_then(|v| v.as_bool());
        if rootless != Some(self.account.uid != 0) {
            return Err("podman reports the wrong profile (rootless/rootful)".into());
        }
        if self.account.uid != 0 {
            let first = info.pointer("/host/idMappings/uidmap/0/host_id").and_then(|v| v.as_u64());
            if first != Some(u64::from(self.account.uid)) {
                return Err("podman reports another uid map than the account's".into());
            }
        }
        self.confirmed = true;
        Ok(r)
    }

    /// The guest's engine-identify: filtered.
    fn engine_identify(&mut self) -> Result<Vec<u8>, String> {
        let r = self.confirm()?;
        Ok(pb::EngineIdentifyResponse {
            info_json: positive::filter_bytes(&r.info_json, positive::ENGINE_INFO, MAX_INSPECT)?,
            version_json: positive::filter_bytes(&r.version_json, positive::ENGINE_VERSION, MAX_INSPECT)?,
        }
        .encode_to_vec())
    }

    /// The raw inventory.
    fn raw_inventory(&mut self) -> Result<pb::InventoryResponse, String> {
        match self.ask(pb::broker_request::Request::Inventory(pb::InventoryRequest {}))? {
            pb::broker_response::Response::Inventory(r) => Ok(r),
            _ => Err("unexpected answer to inventory".into()),
        }
    }

    /// The guest's inventory: filtered by the positive lists.
    fn inventory(&mut self) -> Result<Vec<u8>, String> {
        let r = self.raw_inventory()?;
        let f = |b: &[u8], l: &[&str]| positive::filter_bytes(b, l, MAX_INSPECT);
        Ok(pb::InventoryResponse {
            containers_json: f(&r.containers_json, positive::INVENTORY_CONTAINERS)?,
            pods_json: f(&r.pods_json, positive::INVENTORY_PODS)?,
            networks_json: f(&r.networks_json, positive::INVENTORY_NETWORKS)?,
            volumes_json: f(&r.volumes_json, positive::INVENTORY_VOLUMES)?,
            images_json: f(&r.images_json, positive::INVENTORY_IMAGES)?,
            secrets_json: f(&r.secrets_json, positive::INVENTORY_SECRETS)?,
        }
        .encode_to_vec())
    }

    /// Checks a volume through inspect: driver `local` without options, its
    /// mountpoint the standard place below the policy's graphroot.
    fn check_volume(&mut self, name: &str) -> Result<(), String> {
        let req = pb::InspectRequest { kind: pb::ObjectKind::Volume as i32, ids: vec![name.to_string()], spec: false };
        let pb::broker_response::Response::Inspect(r) = self.ask(pb::broker_request::Request::Inspect(req))? else {
            return Err("unexpected answer to inspect".into());
        };
        let v: serde_json::Value = serde_json::from_slice(&r.json).map_err(|_| "volume inspect is not JSON")?;
        let v = v.get(0).ok_or("volume not found")?;
        if v.get("Name").and_then(|n| n.as_str()) != Some(name) {
            return Err("volume inspect names another volume".into());
        }
        if v.get("Driver").and_then(|d| d.as_str()) != Some("local") {
            return Err(format!("volume {name}: driver not supported (plan error)"));
        }
        if v.get("Options").and_then(|o| o.as_object()).is_some_and(|o| !o.is_empty()) {
            return Err(format!("volume {name}: options (type, o=bind, device) not supported in P0 (plan error)"));
        }
        let expected = self.account.domains[0].graphroot.join("volumes").join(name).join("_data");
        if v.get("Mountpoint").and_then(|m| m.as_str()).map(Path::new) != Some(expected.as_path()) {
            return Err(format!("volume {name}: not at the standard place below the graphroot (plan error)"));
        }
        Ok(())
    }

    /// volume-scan: scan once (host), then answer pages of references.
    fn volume_scan(&mut self, request: &[u8]) -> Result<Vec<u8>, String> {
        let req = pb::VolumeScanRequest::decode(request).map_err(|_| "malformed volume-scan")?;
        if !podman_scan::valid_volume(&req.volume) {
            return Err("invalid volume name".into());
        }
        if !self.confirmed {
            self.confirm()?;
        }
        let scanned = self.scans.lock().map_err(|_| "scan table")?.get(&req.volume).is_some();
        if !scanned {
            if req.cursor != 0 {
                return Err("a scan starts at cursor 0".into());
            }
            self.check_volume(&req.volume)?;
            let prefix = format!("/@podman/{}/volumes/{}", self.domain_id, req.volume);
            let scan = podman_scan::scan_volume(&self.graphroot, &req.volume, &self.domain, &self.domain_id, prefix.as_bytes(),
                podman_scan::DEFAULT_MAX_ENTRIES, onesimus_podman_core::ownership::DEFAULT_MAX_ROWS)?;
            self.scans.lock().map_err(|_| "scan table")?.insert(&req.volume, scan)?;
        }
        let table = self.scans.lock().map_err(|_| "scan table")?;
        let scan = table.get(&req.volume).ok_or("scan missing")?;
        let start = req.cursor as usize;
        if start > scan.entries.len() {
            return Err("cursor beyond the scan".into());
        }
        let mut resp = pb::VolumeScanResponse::default();
        let mut bytes = 0;
        let mut i = start;
        // The last entry is ownership.tsv: it goes into ownership_ref.
        let data_end = scan.entries.len() - 1;
        while i < data_end && resp.entries.len() < PAGE_ENTRIES {
            let e = &scan.entries[i];
            let item = pb::ScanEntry { relpath: e.relpath.clone(), kind: e.kind.file_kind() as i32, size: e.stat.size as u64,
                source_ref: e.source_ref.to_vec() };
            bytes += item.encoded_len() + 4;
            if bytes > PAGE_BYTES && !resp.entries.is_empty() {
                break;
            }
            resp.entries.push(item);
            i += 1;
        }
        resp.next_cursor = i as u32;
        if i >= data_end {
            resp.done = true;
            let tsv = &scan.entries[data_end];
            resp.ownership_ref = tsv.source_ref.to_vec();
            resp.ownership_size = tsv.stat.size as u64;
            resp.rejected = scan.rejected.iter().map(|r| pb::Rejected { relpath: r.relpath.clone(), reason: r.reason.text().into() }).collect();
        }
        Ok(resp.encode_to_vec())
    }
}

impl BrokerPort for PodmanPort {
    fn capability(&self) -> &'static str {
        PODMAN_INVENTORY
    }

    fn serves(&self, capability: &str) -> bool {
        match capability {
            PODMAN_INVENTORY => true,
            PODMAN_VOLUME_READ => self.line.volumes && self.granted.contains(PODMAN_VOLUME_READ),
            _ => false,
        }
    }

    fn call(&mut self, function: BrokerFunction, request: &[u8]) -> Result<Vec<u8>, String> {
        let BrokerFunction::Podman(f) = function else { return Err("not a function of the Podman broker".into()) };
        match f {
            PodmanFunction::EngineIdentify => self.engine_identify(),
            PodmanFunction::Inventory => self.inventory(),
            PodmanFunction::VolumeScan => {
                if !self.serves(PODMAN_VOLUME_READ) {
                    return Err("volume data not allowed for this line".into());
                }
                self.volume_scan(request)
            }
            PodmanFunction::Inspect | PodmanFunction::MemberState => Err("not available before phase P1".into()),
        }
    }
}

/// The checks of a `podman:` line before anything runs: plugin id and
/// contract (V-11, fixed here), line syntax, grants, P0 limits, account
/// policy. Returns the line and the released account.
pub fn check_line(manifest: &onesimus_plugins::Manifest, line: &PluginLine, files: &HostFiles)
    -> Result<(PodmanLine, AccountEntry), PluginError> {
    if manifest.plugin_id != onesimus_plugins::PODMAN_PLUGIN_ID || manifest.api_version < onesimus_plugins::API_VERSION_SOURCE_REF {
        return Err(plan("the Podman imports are bound to org.onesimus.podman with contract 2"));
    }
    let parsed = PodmanLine::parse(&line.options).map_err(plan)?;
    if let Some(g) = parsed.missing_grant(&manifest.capabilities) {
        return Err(plan(format!("the line needs the grant {g}")));
    }
    if parsed.consistency == Consistency::Stop {
        return Err(plan("consistency=stop needs the lifecycle of phase P2; use consistency=crash"));
    }
    check_trusted(&files.policy, files.owner, false, &files.top)?;
    let bytes = onesimus_plugins::bundle::read_bounded(&files.policy, onesimus_podman_core::policy::MAX_POLICY)
        .map_err(|e| plan(format!("account policy: {e}")))?;
    let policy = AccountPolicy::parse(&bytes).map_err(plan)?;
    let account = policy.account(&parsed.user).cloned().ok_or_else(|| plan("the account is not released in the account policy"))?;
    if (parsed.user == Account::Root) != account.rootful {
        return Err(plan("user=root is the rootful profile"));
    }
    Ok((parsed, account))
}

/// Environment of the rootless broker (V-8, Praxis 6).
pub fn broker_env(account: &AccountEntry, gid: u32, home: &Path, runtime: Option<&Path>) -> Vec<(String, String)> {
    let mut env = vec![
        ("HOME".to_string(), home.display().to_string()),
        ("PATH".to_string(), "/usr/sbin:/usr/bin:/sbin:/bin".to_string()),
        ("LANG".to_string(), "C".to_string()),
        ("_CONTAINERS_USERNS_CONFIGURED".to_string(), "done".to_string()),
        ("_CONTAINERS_ROOTLESS_UID".to_string(), account.uid.to_string()),
        ("_CONTAINERS_ROOTLESS_GID".to_string(), gid.to_string()),
    ];
    if let Some(r) = runtime {
        env.push(("XDG_RUNTIME_DIR".to_string(), r.display().to_string()));
    }
    env
}

/// Broker arguments from the account policy's domain.
pub fn broker_args(account: &AccountEntry) -> Vec<String> {
    let d = &account.domains[0];
    let mut a = vec!["--root".to_string(), d.graphroot.display().to_string()];
    for (flag, v) in [("--runroot", &d.runroot), ("--tmpdir", &d.tmpdir)] {
        if let Some(p) = v {
            a.push(flag.into());
            a.push(p.display().to_string());
        }
    }
    a.push("--rootless".into());
    a
}

/// `podman.inventory` granted to org.onesimus.podman: check the line, start
/// the broker (rootless) or the rootful backend, confirm the engine.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare(manifest: &onesimus_plugins::Manifest, line: &PluginLine, plugin_dir: &Path, triplet: &str,
    _chunk: u32, owner: u32, top: &Path, _context: &BrokerContext, files: &HostFiles) -> Result<PreparedBroker, PluginError> {
    use onesimus_plugins::wasm::BrokerHandle;
    let (parsed, account) = check_line(manifest, line, files)?;
    let (domain, gid) = domain_of(&account, files)?;
    let graphroot = podman_scan::open_graphroot(&account.domains[0].graphroot).map_err(plan)?;
    let (backend, broker, cancel): (Box<dyn Backend>, Option<crate::broker::Broker>, std::os::unix::net::UnixStream) = if account.uid == 0 {
        let d = &account.domains[0];
        let storage = Storage { root: Some(d.graphroot.clone()), runroot: d.runroot.clone(), tmpdir: d.tmpdir.clone() };
        let (a, _b) = std::os::unix::net::UnixStream::pair().map_err(|e| plan(e.to_string()))?;
        (Box::new(crate::podman_rootful::RootfulBackend::new(Box::new(crate::podman_rootful::SystemRunner), storage)), None, a)
    } else {
        let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(account.uid)).ok().flatten()
            .ok_or_else(|| plan("account without passwd entry"))?;
        let runtime = PathBuf::from(format!("/run/user/{}", account.uid));
        let ns = UserNs {
            uid_map: domain.uid_map.proc_map(),
            gid_map: domain.gid_map.proc_map(),
            env: broker_env(&account, gid, &user.dir, runtime.is_dir().then_some(runtime.as_path())),
        };
        let b = crate::broker::launch_in_userns(plugin_dir, manifest, triplet, BROKER_PART, &ns, &broker_args(&account), owner, top)?;
        let io = |e: std::io::Error| plan(format!("broker socket: {e}"));
        let client = BrokerClient { io: b.socket.try_clone().map_err(io)? };
        let cancel = b.socket.try_clone().map_err(io)?;
        (Box::new(client), Some(b), cancel)
    };
    let mut port = PodmanPort::new(backend, parsed, account, domain, graphroot, &manifest.capabilities);
    port.confirm().map_err(plan)?;
    let scans = port.scans();
    Ok(PreparedBroker {
        handle: BrokerHandle(Arc::new(Mutex::new(Box::new(port)))),
        broker,
        cancel,
        warnings: Vec::new(),
        scans: Some(scans),
    })
}

#[cfg(test)]
mod tests;
