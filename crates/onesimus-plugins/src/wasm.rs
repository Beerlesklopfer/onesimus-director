//! Verified Component Model host. No ambient WASI or raw engine imports are
//! linked. All guest exports use the protobuf contract carried by the WIT ABI.
//! The only import a guest may get is a typed broker interface (e.g.
//! `onesimus:filed-plugin/postgresql`), linked only when the verified
//! manifest carries its capability and the host supplies a broker port.
use crate::{
    TrustPolicy, VerifiedComponent, VerifyError,
    executor::{self, Backend, Binding, BoundedMessage, Context, Host, Interrupt, Worker},
};
use onesimus_proto_rs::onesimus::{ccc::ReturnCode, filed::plugin::v1 as pb};
use prost::Message;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker, TypedFunc},
};

type Function = TypedFunc<(Vec<u8>,), (Vec<u8>,)>;
const MAX_PATH: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct RuntimeLimits {
    pub memory_bytes: usize,
    pub max_component_bytes: usize,
    pub fuel_per_operation: u64,
    pub max_chunk_bytes: u32,
    pub initialization_timeout: Duration,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 32 * 1024 * 1024,
            max_component_bytes: 16 * 1024 * 1024,
            fuel_per_operation: 10_000_000,
            max_chunk_bytes: 256 * 1024,
            initialization_timeout: Duration::from_secs(10),
        }
    }
}
#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub binding: Binding,
    pub job: pb::JobContext,
    pub options: BTreeMap<String, String>,
    pub worker: executor::Limits,
    pub runtime: RuntimeLimits,
    /// The host side of a broker import; None: the guest gets no import.
    pub broker: Option<BrokerHandle>,
}

/// One function of a typed broker import, namespaced by broker. Each takes
/// and returns one encoded message of the broker's protobuf contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrokerFunction {
    Postgresql(PostgresqlFunction),
    Openldap(OpenldapFunction),
}
/// proto/postgresql_broker.proto, import `postgresql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostgresqlFunction {
    Info,
    DatabaseInfo,
    DumpOpen,
    DumpRead,
    DumpClose,
}
/// proto/openldap_broker.proto, imports `openldap` and `openldap-restore`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenldapFunction {
    Info,
    DumpOpen,
    DumpRead,
    DumpClose,
    ConfigFileOpen,
    LoadOpen,
    LoadWrite,
    LoadClose,
}

/// The host's implementation of a broker import. It checks every request
/// before passing it to the broker process.
pub trait BrokerPort: Send {
    /// The broker family: the capability the manifest must carry for any
    /// import of this broker to be linked (e.g. "postgresql.read").
    fn capability(&self) -> &'static str;
    /// Whether this port serves `capability` in the current job. A granted
    /// capability of the family that the port does not serve (e.g.
    /// "openldap.write" in a backup job) gets functions that only fail.
    fn serves(&self, capability: &str) -> bool {
        capability == self.capability()
    }
    fn call(&mut self, function: BrokerFunction, request: &[u8]) -> Result<Vec<u8>, String>;
    /// The end of the job; notes for the job log (e.g. where a restore went).
    fn finish(&mut self, _succeeded: bool) -> Vec<String> {
        Vec::new()
    }
}

/// A shareable broker port (LoadOptions stays Clone).
#[derive(Clone)]
pub struct BrokerHandle(pub Arc<Mutex<Box<dyn BrokerPort>>>);
impl std::fmt::Debug for BrokerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BrokerHandle")
    }
}

/// The capability of the PostgreSQL broker import.
pub const POSTGRESQL_READ: &str = "postgresql.read";
/// The OpenLDAP broker: reading (backup), and loading into a new target
/// (online restore only).
pub const OPENLDAP_READ: &str = "openldap.read";
pub const OPENLDAP_WRITE: &str = "openldap.write";
/// Capabilities whose import only a running broker can serve.
pub const BROKER_CAPABILITIES: &[&str] = &[POSTGRESQL_READ, OPENLDAP_READ, OPENLDAP_WRITE];

/// Whether the plugin imports a broker interface (and so cannot be
/// instantiated without a broker, e.g. by `ofd --check-plugin`).
pub fn needs_broker(manifest: &crate::Manifest) -> bool {
    manifest.capabilities.iter().any(|c| BROKER_CAPABILITIES.contains(&c.as_str()))
}

/// One WIT import of a broker: linked only when the verified manifest
/// carries `capability` and the port belongs to `family`.
pub struct BrokerImport {
    pub capability: &'static str,
    pub family: &'static str,
    pub import: &'static str,
    pub functions: &'static [(&'static str, BrokerFunction)],
}

/// Capability -> import -> functions, for every broker the host knows.
pub const BROKER_IMPORTS: &[BrokerImport] = &[
    BrokerImport {
        capability: POSTGRESQL_READ,
        family: POSTGRESQL_READ,
        import: "onesimus:filed-plugin/postgresql@1.0.0",
        functions: &[
            ("info", BrokerFunction::Postgresql(PostgresqlFunction::Info)),
            ("database-info", BrokerFunction::Postgresql(PostgresqlFunction::DatabaseInfo)),
            ("dump-open", BrokerFunction::Postgresql(PostgresqlFunction::DumpOpen)),
            ("dump-read", BrokerFunction::Postgresql(PostgresqlFunction::DumpRead)),
            ("dump-close", BrokerFunction::Postgresql(PostgresqlFunction::DumpClose)),
        ],
    },
    BrokerImport {
        capability: OPENLDAP_READ,
        family: OPENLDAP_READ,
        import: "onesimus:filed-plugin/openldap@1.0.0",
        functions: &[
            ("info", BrokerFunction::Openldap(OpenldapFunction::Info)),
            ("dump-open", BrokerFunction::Openldap(OpenldapFunction::DumpOpen)),
            ("dump-read", BrokerFunction::Openldap(OpenldapFunction::DumpRead)),
            ("dump-close", BrokerFunction::Openldap(OpenldapFunction::DumpClose)),
            ("config-file-open", BrokerFunction::Openldap(OpenldapFunction::ConfigFileOpen)),
        ],
    },
    BrokerImport {
        capability: OPENLDAP_WRITE,
        family: OPENLDAP_READ,
        import: "onesimus:filed-plugin/openldap-restore@1.0.0",
        functions: &[
            ("load-open", BrokerFunction::Openldap(OpenldapFunction::LoadOpen)),
            ("load-write", BrokerFunction::Openldap(OpenldapFunction::LoadWrite)),
            ("load-close", BrokerFunction::Openldap(OpenldapFunction::LoadClose)),
        ],
    },
];
/// Largest broker request a guest may send (a load-write carries one block).
const MAX_BROKER_REQUEST: usize = 64 * 1024 + 512 * 1024;

/// Store data: memory limits and the broker port of this instance.
pub struct HostState {
    limits: StoreLimits,
    broker: Option<BrokerHandle>,
    max_broker_response: usize,
}

fn broker_call(state: &mut HostState, function: BrokerFunction, request: Vec<u8>) -> Result<Vec<u8>, String> {
    if request.len() > MAX_BROKER_REQUEST {
        return Err("broker request too large".into());
    }
    let handle = state.broker.as_ref().ok_or("no broker")?;
    let response = handle.0.lock().map_err(|_| "broker unavailable".to_string())?.call(function, &request)?;
    if response.len() > state.max_broker_response {
        return Err("broker response too large".into());
    }
    Ok(response)
}

/// Which imports a port of `family` gets for a manifest with `granted`
/// capabilities, as (import, live). Live imports reach the port; the others
/// (granted, but not served in this job) only fail. Imports of other
/// families, or whose capability is not granted, are not linked at all, so
/// a guest importing them cannot be instantiated.
pub fn import_plan(granted: &[String], family: &str, serves: &dyn Fn(&str) -> bool)
    -> Result<Vec<(&'static BrokerImport, bool)>, String> {
    let has = |c: &str| granted.iter().any(|g| g == c);
    if !has(family) || !BROKER_IMPORTS.iter().any(|i| i.capability == family && i.family == family) {
        return Err(format!("broker {family} not granted to this plugin"));
    }
    Ok(BROKER_IMPORTS.iter()
        .filter(|i| i.family == family && has(i.capability))
        .map(|i| (i, serves(i.capability)))
        .collect())
}

/// Links the broker imports of the port's family: only capabilities in the
/// verified manifest, real functions only for what the port serves.
fn link_brokers(linker: &mut Linker<HostState>, manifest: &crate::Manifest, broker: Option<&BrokerHandle>)
    -> Result<(), executor::Error> {
    let Some(handle) = broker else { return Ok(()) };
    let plan = {
        let port = handle.0.lock().map_err(|_| backend("broker unavailable"))?;
        import_plan(&manifest.capabilities, port.capability(), &|c| port.serves(c)).map_err(backend)?
    };
    for (import, live) in plan {
        let mut instance = linker.instance(import.import).map_err(backend)?;
        let capability = import.capability;
        for &(name, function) in import.functions {
            if live {
                instance
                    .func_wrap(name, move |mut caller: wasmtime::StoreContextMut<'_, HostState>, (request,): (Vec<u8>,)| {
                        Ok((broker_call(caller.data_mut(), function, request),))
                    })
                    .map_err(backend)?;
            } else {
                instance
                    .func_wrap(name, move |_caller: wasmtime::StoreContextMut<'_, HostState>, (_request,): (Vec<u8>,)| {
                        Ok((Err::<Vec<u8>, String>(format!("{capability} is not available in this job")),))
                    })
                    .map_err(backend)?;
            }
        }
    }
    Ok(())
}
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Verification(#[from] VerifyError),
    #[error(transparent)]
    Runtime(#[from] executor::Error),
    #[error("invalid host clock")]
    Clock,
}

/// Verify current public trust before compiling/initializing the component.
/// The bundle cannot add CA roots, keys, host imports or capabilities.
pub fn load_signed(
    host: &Host,
    bundle: pb::SignedPluginArtifact,
    policy: &TrustPolicy,
    options: LoadOptions,
) -> Result<Worker<WasmBackend>, LoadError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| LoadError::Clock)?
        .as_secs();
    load_signed_at(host, bundle, policy, options, now)
}
fn load_signed_at(
    host: &Host,
    bundle: pb::SignedPluginArtifact,
    policy: &TrustPolicy,
    options: LoadOptions,
    now: u64,
) -> Result<Worker<WasmBackend>, LoadError> {
    validate_options(&options)?;
    if bundle.artifact_bytes.len() > options.runtime.max_component_bytes {
        return Err(executor::Error::TooLarge.into());
    }
    let verified = crate::verify(
        &bundle.manifest_bytes,
        &bundle.ed25519_signature,
        bundle.artifact_bytes,
        &bundle.x509_certificate_chain_der,
        policy,
        now,
    )?;
    let binding = options.binding.clone();
    let worker_limits = options.worker.clone();
    let init_timeout = options.runtime.initialization_timeout;
    Ok(host.spawn_with_initialization_timeout(
        verified,
        binding,
        worker_limits,
        init_timeout,
        move |verified| WasmBackend::instantiate(verified, options),
    )?)
}
fn validate_options(options: &LoadOptions) -> Result<(), executor::Error> {
    let r = &options.runtime;
    if options.binding.job_id != options.job.job_id
        || options.job.job_name.is_empty()
        || !matches!(
            pb::JobOperation::try_from(options.job.operation),
            Ok(pb::JobOperation::Backup | pb::JobOperation::Restore)
        )
        || r.memory_bytes < 65536
        || r.memory_bytes > u32::MAX as usize
        || r.fuel_per_operation == 0
        || r.max_component_bytes == 0
        || r.max_chunk_bytes == 0
        || r.max_chunk_bytes as usize > r.memory_bytes
        || r.initialization_timeout.is_zero()
        || r.initialization_timeout > Duration::from_secs(300)
        || options.worker.max_pending_operations > u32::MAX as usize
        || options.worker.operation_timeout.as_millis() == 0
        || options.worker.operation_timeout.as_millis() > u64::MAX as u128
    {
        return Err(executor::Error::Configuration);
    }
    Ok(())
}

pub enum Operation {
    Event(pb::EventRequest),
    NextFile(pb::NextFileRequest),
    ReadFile(pb::ReadFileRequest),
    WriteFile(pb::WriteFileRequest),
    CloseFile(pb::CloseFileRequest),
}
impl BoundedMessage for Operation {
    fn byte_size(&self) -> usize {
        match self {
            Self::Event(r) => r.encoded_len(),
            Self::NextFile(r) => r.encoded_len(),
            Self::ReadFile(r) => r.encoded_len(),
            Self::WriteFile(r) => r.encoded_len(),
            Self::CloseFile(r) => r.encoded_len(),
        }
    }
}
#[derive(Debug)]
pub enum Reply {
    Result(pb::OperationResult),
    NextFile(pb::NextFileResponse),
    ReadFile(pb::ReadFileResponse),
}
impl BoundedMessage for Reply {
    fn byte_size(&self) -> usize {
        match self {
            Self::Result(r) => r.encoded_len(),
            Self::NextFile(r) => r.encoded_len(),
            Self::ReadFile(r) => r.encoded_len(),
        }
    }
}

struct EngineInterrupt {
    engine: Engine,
    stopped: AtomicBool,
}
impl Interrupt for EngineInterrupt {
    fn interrupt(&self) {
        self.stopped.store(true, Ordering::Release);
        self.engine.increment_epoch();
    }
}
/// Initialization must not escape interruption merely because the worker has
/// not published its ready state yet. The timer also bounds core start funcs.
struct InitializationTimer {
    done: Arc<(Mutex<bool>, Condvar)>,
    handle: Option<std::thread::JoinHandle<()>>,
}
impl InitializationTimer {
    fn start(interrupt: Arc<EngineInterrupt>, timeout: Duration) -> Result<Self, executor::Error> {
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let state = done.clone();
        let handle = std::thread::Builder::new()
            .name("plugin:wasm-init".into())
            .spawn(move || {
                let (lock, changed) = &*state;
                let finished = lock.lock().unwrap_or_else(|e| e.into_inner());
                let (finished, _) = changed
                    .wait_timeout_while(finished, timeout, |done| !*done)
                    .unwrap_or_else(|e| e.into_inner());
                if !*finished {
                    interrupt.interrupt();
                }
            })
            .map_err(|e| executor::Error::Backend(e.to_string()))?;
        Ok(Self {
            done,
            handle: Some(handle),
        })
    }
}
impl Drop for InitializationTimer {
    fn drop(&mut self) {
        let (lock, changed) = &*self.done;
        {
            *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
            changed.notify_all();
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
#[derive(PartialEq, Eq)]
enum Phase {
    Ready,
    Started,
    Finished,
}
pub struct WasmBackend {
    store: Store<HostState>,
    interrupt: Arc<EngineInterrupt>,
    event: Function,
    next: Function,
    read: Function,
    write: Function,
    close: Function,
    binding: Binding,
    job: pb::JobContext,
    phase: Phase,
    limits: RuntimeLimits,
    max_response: usize,
    capabilities: Vec<String>,
}
fn backend(error: impl std::fmt::Display) -> executor::Error {
    executor::Error::Backend(error.to_string())
}
/// Bounds of `FileEntry.attributes`: display metadata, not a data channel.
pub const MAX_ATTRIBUTES: usize = 32;
pub const MAX_ATTRIBUTE_KEY: usize = 64;
pub const MAX_ATTRIBUTE_VALUE: usize = 1024;
fn attributes(map: &std::collections::HashMap<String, String>) -> Result<(), executor::Error> {
    if map.len() > MAX_ATTRIBUTES {
        return Err(backend("too many file attributes"));
    }
    for (key, value) in map {
        let key_ok = !key.is_empty()
            && key.len() <= MAX_ATTRIBUTE_KEY
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
        if !key_ok {
            return Err(backend("invalid file attribute name"));
        }
        if value.len() > MAX_ATTRIBUTE_VALUE || value.chars().any(char::is_control) {
            return Err(backend("invalid file attribute value"));
        }
    }
    Ok(())
}
/// Longest plugin diagnostic passed on to the job log.
const MAX_DIAGNOSTIC: usize = 512;
fn ok(result: Option<&pb::OperationResult>) -> Result<(), executor::Error> {
    match result {
        Some(r) if r.code == ReturnCode::RcOk as i32 => Ok(()),
        Some(r) if !r.diagnostic.is_empty() => {
            // The plugin's own words, bounded and without control characters.
            let text: String = r.diagnostic.chars().map(|c| if c.is_control() { ' ' } else { c })
                .take(MAX_DIAGNOSTIC).collect();
            Err(backend(format!("plugin: {}", text.trim())))
        }
        _ => Err(backend("plugin returned an unsuccessful or missing result")),
    }
}
impl WasmBackend {
    fn instantiate(
        component: VerifiedComponent,
        options: LoadOptions,
    ) -> Result<Self, executor::Error> {
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .epoch_interruption(true)
            .max_wasm_stack(256 * 1024)
            .memory_reservation(options.runtime.memory_bytes as u64);
        let engine = Engine::new(&config).map_err(backend)?;
        let interrupt = Arc::new(EngineInterrupt {
            engine: engine.clone(),
            stopped: AtomicBool::new(false),
        });
        let timer =
            InitializationTimer::start(interrupt.clone(), options.runtime.initialization_timeout)?;
        // Compile portable WASM bytes only. Never deserialize native/precompiled
        // engine artifacts from a plugin bundle, even when they are signed.
        let compiled = Component::new(&engine, component.bytes()).map_err(backend)?;
        if interrupt.stopped.load(Ordering::Acquire) {
            return Err(executor::Error::Timeout);
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(options.runtime.memory_bytes)
            .memories(1)
            .tables(4)
            .table_elements(4096)
            .instances(16)
            .trap_on_grow_failure(true)
            .build();
        let max_broker_response = options.runtime.max_chunk_bytes as usize + 64 * 1024;
        let mut store = Store::new(&engine, HostState { limits, broker: options.broker.clone(), max_broker_response });
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(options.runtime.fuel_per_operation)
            .map_err(backend)?;
        store.set_epoch_deadline(1);
        // Deliberately no WASI or other host imports; at most a granted broker.
        let mut linker = Linker::new(&engine);
        link_brokers(&mut linker, component.manifest(), options.broker.as_ref())?;
        let instance = linker.instantiate(&mut store, &compiled).map_err(backend)?;
        let initialize = instance
            .get_typed_func::<(Vec<u8>,), (Vec<u8>,)>(&mut store, "initialize")
            .map_err(backend)?;
        let event = instance
            .get_typed_func(&mut store, "handle-event")
            .map_err(backend)?;
        let next = instance
            .get_typed_func(&mut store, "next-file")
            .map_err(backend)?;
        let read = instance
            .get_typed_func(&mut store, "read-file")
            .map_err(backend)?;
        let write = instance
            .get_typed_func(&mut store, "write-file")
            .map_err(backend)?;
        let close = instance
            .get_typed_func(&mut store, "close-file")
            .map_err(backend)?;
        let _shutdown: Function = instance
            .get_typed_func(&mut store, "shutdown")
            .map_err(backend)?;
        let manifest = component.manifest();
        let request = pb::InitializeRequest {
            instance_id: options.binding.instance_id.clone(),
            plugin_id: manifest.plugin_id.clone(),
            version: manifest.version.clone(),
            api_version: manifest.api_version,
            granted_capabilities: manifest.capabilities.clone(),
            options: options.options.into_iter().collect(),
            limits: Some(pb::ResourceLimits {
                memory_bytes: options.runtime.memory_bytes as u64,
                operation_timeout_millis: options.worker.operation_timeout.as_millis() as u64,
                max_open_files: 1,
                max_chunk_bytes: options.runtime.max_chunk_bytes,
                max_pending_operations: options.worker.max_pending_operations as u32,
            }),
            binding: Some(pb::ExecutionBinding {
                job: Some(options.job.clone()),
                device_id: options.binding.device_id.clone(),
                worker_id: options.binding.worker_id.clone(),
            }),
        };
        if request.encoded_len() > options.worker.max_request_bytes {
            return Err(executor::Error::TooLarge);
        }
        let (bytes,) = initialize
            .call(&mut store, (request.encode_to_vec(),))
            .map_err(backend)?;
        if bytes.len() > options.worker.max_response_bytes {
            return Err(executor::Error::TooLarge);
        }
        let response = pb::InitializeResponse::decode(bytes.as_slice()).map_err(backend)?;
        ok(response.result.as_ref())?;
        let metadata = response
            .metadata
            .ok_or_else(|| backend("missing plugin metadata"))?;
        if metadata.version != manifest.version
            || metadata.interface_version != manifest.api_version
        {
            return Err(backend(
                "plugin metadata does not match the certified version",
            ));
        }
        drop(timer);
        if interrupt.stopped.load(Ordering::Acquire) {
            return Err(executor::Error::Timeout);
        }
        Ok(Self {
            store,
            interrupt,
            event,
            next,
            read,
            write,
            close,
            binding: options.binding,
            job: options.job,
            phase: Phase::Ready,
            limits: options.runtime,
            max_response: options.worker.max_response_bytes,
            capabilities: manifest.capabilities.clone(),
        })
    }
    fn bound(&self, instance: &str, job: u64) -> Result<(), executor::Error> {
        if instance != self.binding.instance_id || job != self.binding.job_id {
            Err(executor::Error::Binding)
        } else {
            Ok(())
        }
    }
    fn file_access(&self, restore: bool) -> Result<(), executor::Error> {
        let operation = if restore {
            pb::JobOperation::Restore
        } else {
            pb::JobOperation::Backup
        };
        let capability = if restore { "file.write" } else { "file.read" };
        if self.phase != Phase::Started
            || self.job.operation != operation as i32
            || !self.capabilities.iter().any(|c| c == capability)
        {
            return Err(backend(
                "file operation is not authorized in this job phase",
            ));
        }
        Ok(())
    }
    fn path(&self, path: &[u8]) -> Result<(), executor::Error> {
        if path.is_empty() || path.len() > MAX_PATH || path.contains(&0) {
            Err(backend("invalid logical file path"))
        } else {
            Ok(())
        }
    }
    fn call<M: Message, R: Message + Default>(
        &mut self,
        function: Function,
        request: &M,
    ) -> Result<R, executor::Error> {
        self.store
            .set_fuel(self.limits.fuel_per_operation)
            .map_err(backend)?;
        self.store.set_epoch_deadline(1);
        // Interrupts issued before setting the new deadline must not be lost.
        if self.interrupt.stopped.load(Ordering::Acquire) {
            return Err(executor::Error::Cancelled);
        }
        let (bytes,) = function
            .call(&mut self.store, (request.encode_to_vec(),))
            .map_err(backend)?;
        if bytes.len() > self.max_response {
            return Err(executor::Error::TooLarge);
        }
        R::decode(bytes.as_slice()).map_err(backend)
    }
}
impl Backend for WasmBackend {
    type Request = Operation;
    type Reply = Reply;
    fn interrupt_handle(&self) -> Arc<dyn Interrupt> {
        self.interrupt.clone()
    }
    fn invoke(&mut self, request: Operation, context: &Context) -> Result<Reply, executor::Error> {
        context.check()?;
        match request {
            Operation::Event(request) => {
                let job = request.job.as_ref().ok_or(executor::Error::Binding)?;
                self.bound(&request.instance_id, job.job_id)?;
                if job != &self.job {
                    return Err(executor::Error::Binding);
                }
                let next_phase = match &request.event {
                    Some(pb::event_request::Event::Started(_)) if self.phase == Phase::Ready => {
                        Phase::Started
                    }
                    Some(
                        pb::event_request::Event::Finished(_)
                        | pb::event_request::Event::Cancelled(_),
                    ) if self.phase == Phase::Started => Phase::Finished,
                    _ => return Err(backend("invalid plugin event transition")),
                };
                let result: pb::OperationResult = self.call(self.event, &request)?;
                ok(Some(&result))?;
                self.phase = next_phase;
                Ok(Reply::Result(result))
            }
            Operation::NextFile(request) => {
                self.bound(&request.instance_id, request.job_id)?;
                self.file_access(false)?;
                let response: pb::NextFileResponse = self.call(self.next, &request)?;
                ok(response.result.as_ref())?;
                match &response.item {
                    Some(pb::next_file_response::Item::File(file)) => {
                        self.path(&file.path)?;
                        if !matches!(
                            pb::FileKind::try_from(file.kind),
                            Ok(pb::FileKind::Regular
                                | pb::FileKind::Directory
                                | pb::FileKind::Symlink
                                | pb::FileKind::Virtual)
                        ) {
                            return Err(backend("unsupported file kind"));
                        }
                        attributes(&file.attributes)?;
                    }
                    Some(pb::next_file_response::Item::Complete(_)) => {}
                    None => return Err(backend("missing scan item")),
                }
                Ok(Reply::NextFile(response))
            }
            Operation::ReadFile(request) => {
                self.bound(&request.instance_id, request.job_id)?;
                self.file_access(false)?;
                self.path(&request.path)?;
                if request.max_bytes == 0
                    || request.max_bytes > self.limits.max_chunk_bytes
                    || request
                        .offset
                        .checked_add(request.max_bytes as u64)
                        .is_none()
                {
                    return Err(executor::Error::TooLarge);
                }
                let response: pb::ReadFileResponse = self.call(self.read, &request)?;
                ok(response.result.as_ref())?;
                if response.data.len() > request.max_bytes as usize {
                    return Err(executor::Error::TooLarge);
                }
                if response.data.is_empty() && !response.eof {
                    return Err(backend("read made no progress without EOF"));
                }
                Ok(Reply::ReadFile(response))
            }
            Operation::WriteFile(request) => {
                self.bound(&request.instance_id, request.job_id)?;
                self.file_access(true)?;
                self.path(
                    &request
                        .file
                        .as_ref()
                        .ok_or_else(|| backend("missing file metadata"))?
                        .path,
                )?;
                if let Some(file) = request.file.as_ref() {
                    attributes(&file.attributes)?;
                }
                if request.data.len() > self.limits.max_chunk_bytes as usize
                    || request
                        .offset
                        .checked_add(request.data.len() as u64)
                        .is_none()
                {
                    return Err(executor::Error::TooLarge);
                }
                let result: pb::OperationResult = self.call(self.write, &request)?;
                ok(Some(&result))?;
                Ok(Reply::Result(result))
            }
            Operation::CloseFile(request) => {
                self.bound(&request.instance_id, request.job_id)?;
                self.file_access(true)?;
                self.path(&request.path)?;
                let result: pb::OperationResult = self.call(self.close, &request)?;
                ok(Some(&result))?;
                Ok(Reply::Result(result))
            }
        }
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn plugin_diagnostics_reach_the_log_bounded_and_clean() {
        let r = |code: ReturnCode, d: &str| pb::OperationResult { code: code as i32, diagnostic: d.into() };
        assert!(ok(Some(&r(ReturnCode::RcOk, "fine"))).is_ok());
        let e = ok(Some(&r(ReturnCode::RcError, "dump failed:\nconnection lost"))).unwrap_err().to_string();
        assert!(e.contains("plugin: dump failed: connection lost"), "{e}");
        let long = "x".repeat(5000);
        assert!(ok(Some(&r(ReturnCode::RcError, &long))).unwrap_err().to_string().len() < MAX_DIAGNOSTIC + 64);
        assert!(ok(None).unwrap_err().to_string().contains("missing"));
    }
}
