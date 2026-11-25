// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
use super::*;
use crate::executor::Error;

#[derive(Clone, Copy)]
enum Behavior {
    Complete,
    Loop,
    Grow,
    Trap,
    WrongResult,
    VirtualFile,
    BadAttribute,
}
const UNKNOWN_IMPORT: &str = "(import \"ungranted-host-access\" (func))";
/// The PostgreSQL broker import, declared (one function) but not called.
const PG_IMPORT: &str = "(import \"onesimus:filed-plugin/postgresql@1.0.0\" (instance \
    (export \"info\" (func (param \"request\" (list u8)) (result (result (list u8) (error string)))))))";

/// A broker port that answers every call with the request reversed.
struct EchoPort(&'static str);
impl BrokerPort for EchoPort {
    fn capability(&self) -> &'static str {
        self.0
    }
    fn call(&mut self, _function: BrokerFunction, request: &[u8]) -> Result<Vec<u8>, String> {
        Ok(request.iter().rev().copied().collect())
    }
}
fn port(capability: &'static str) -> Option<BrokerHandle> {
    Some(BrokerHandle(Arc::new(Mutex::new(Box::new(EchoPort(capability))))))
}

fn escaped(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}
fn fixture(
    behavior: Behavior,
    import: &str,
    wrong_metadata: bool,
    core_start_loop: bool,
) -> Vec<u8> {
    use onesimus_proto_rs::onesimus::ccc::PluginMetadata;
    let result = pb::OperationResult {
        code: ReturnCode::RcOk as i32,
        diagnostic: String::new(),
    };
    let initialize = pb::InitializeResponse {
        result: Some(result.clone()),
        metadata: Some(PluginMetadata {
            version: if wrong_metadata {
                "wrong".into()
            } else {
                "1.0.0".into()
            },
            interface_version: 1,
            ..Default::default()
        }),
    }
    .encode_to_vec();
    let item = match behavior {
        Behavior::VirtualFile | Behavior::BadAttribute => {
            let key = if matches!(behavior, Behavior::BadAttribute) { "bad key" } else { "database" };
            pb::next_file_response::Item::File(pb::FileEntry {
                path: b"@postgresql/main/db/EleLa/EleLa.dump".to_vec(),
                kind: pb::FileKind::Virtual as i32,
                size: 12,
                attributes: [(key.to_string(), "EleLa".to_string())].into(),
                ..Default::default()
            })
        }
        _ => pb::next_file_response::Item::Complete(pb::ScanComplete {}),
    };
    let next = pb::NextFileResponse {
        result: Some(result.clone()),
        item: Some(item),
    }
    .encode_to_vec();
    let read = pb::ReadFileResponse {
        result: Some(result.clone()),
        data: b"fixture-data".to_vec(),
        eof: true,
    }
    .encode_to_vec();
    let results = [initialize, result.encode_to_vec(), next, read];
    let mut core = String::new();
    for (i, bytes) in results.iter().enumerate() {
        let ptr = 64 + i * 256;
        core.push_str(&format!(
            "(data (i32.const {ptr}) \"{}\")\n",
            escaped(bytes)
        ));
        let prefix = if i == 2 {
            match behavior {
                Behavior::Loop => "(loop $forever (br $forever))",
                Behavior::Grow => "(drop (memory.grow (i32.const 1)))",
                Behavior::Trap => "unreachable",
                _ => "",
            }
        } else {
            ""
        };
        let length = if i == 2 && matches!(behavior, Behavior::WrongResult) {
            0
        } else {
            bytes.len()
        };
        core.push_str(&format!(
            r#"(func (export "f{i}") (param i32 i32) (result i32)
            {prefix}
            (i32.store (i32.const 0) (i32.const {ptr}))
            (i32.store (i32.const 4) (i32.const {length})) (i32.const 0))"#
        ));
    }
    let mut exports = String::new();
    for (name, i) in [
        ("initialize", 0),
        ("handle-event", 1),
        ("next-file", 2),
        ("read-file", 3),
        ("write-file", 1),
        ("close-file", 1),
        ("shutdown", 1),
    ] {
        exports.push_str(&format!(
            r#"
            (alias core export $i "f{i}" (core func ${name}))
            (func (export "{name}") (param "request" (list u8)) (result (list u8))
                (canon lift (core func ${name}) (memory $mem) (realloc $realloc)))"#
        ));
    }

    let start = if core_start_loop {
        "(func $start (loop $forever (br $forever))) (start $start)"
    } else {
        ""
    };
    wat::parse_str(format!(
        r#"(component {import}
        (core module $m
            (memory (export "memory") 1)
            (func (export "realloc") (param i32 i32 i32 i32) (result i32) (i32.const 4096))
            {core} {start})
        (core instance $i (instantiate $m))
        (alias core export $i "memory" (core memory $mem))
        (alias core export $i "realloc" (core func $realloc))
        {exports})"#
    ))
    .unwrap()
}
fn options(id: &str) -> LoadOptions {
    LoadOptions {
        binding: Binding {
            instance_id: id.into(),
            job_id: 42,
            device_id: format!("device-{id}"),
            worker_id: format!("worker-{id}"),
        },
        job: pb::JobContext {
            job_id: 42,
            job_name: "test-job".into(),
            operation: pb::JobOperation::Backup as i32,
        },
        options: BTreeMap::new(),
        worker: executor::Limits {
            max_pending_operations: 1,
            max_request_bytes: 4096,
            max_response_bytes: 4096,
            operation_timeout: Duration::from_secs(5),
        },
        runtime: RuntimeLimits {
            memory_bytes: 65536,
            max_chunk_bytes: 1024,
            ..Default::default()
        },
        broker: None,
    }
}
fn load(host: &Host, behavior: Behavior, opts: LoadOptions) -> Worker<WasmBackend> {
    let (bundle, policy) = crate::tests::signed_component(fixture(behavior, "", false, false));
    load_signed_at(host, bundle, &policy, opts, 150).unwrap()
}
fn start(worker: &Worker<WasmBackend>, opts: &LoadOptions) {
    worker
        .submit(
            &opts.binding,
            Operation::Event(pb::EventRequest {
                instance_id: opts.binding.instance_id.clone(),
                job: Some(opts.job.clone()),
                event: Some(pb::event_request::Event::Started(pb::JobStarted {})),
            }),
        )
        .unwrap()
        .wait()
        .unwrap();
}
fn next(opts: &LoadOptions) -> Operation {
    Operation::NextFile(pb::NextFileRequest {
        instance_id: opts.binding.instance_id.clone(),
        job_id: opts.job.job_id,
    })
}
fn read(opts: &LoadOptions, count: u32) -> Operation {
    Operation::ReadFile(pb::ReadFileRequest {
        instance_id: opts.binding.instance_id.clone(),
        job_id: opts.job.job_id,
        path: b"logical-file".to_vec(),
        offset: 0,
        max_bytes: count,
    })
}
#[test]
fn signed_real_component_initializes_and_serves_typed_protobuf_calls() {
    let host = Host::new(1).unwrap();
    let opts = options("real");
    let mut worker = load(&host, Behavior::Complete, opts.clone());
    start(&worker, &opts);
    assert!(matches!(
        worker
            .submit(&opts.binding, next(&opts))
            .unwrap()
            .wait()
            .unwrap(),
        Reply::NextFile(_)
    ));
    match worker
        .submit(&opts.binding, read(&opts, 1024))
        .unwrap()
        .wait()
        .unwrap()
    {
        Reply::ReadFile(reply) => assert_eq!(reply.data, b"fixture-data"),
        _ => panic!("wrong response kind"),
    }
    worker.shutdown(Duration::from_secs(1)).unwrap();
    assert_eq!(host.active_instances(), 0);
}
#[test]
fn signature_verification_happens_before_component_compilation() {
    let host = Host::new(1).unwrap();
    let (mut bundle, policy) =
        crate::tests::signed_component(fixture(Behavior::Complete, "", false, false));
    bundle.artifact_bytes.push(0);
    assert!(matches!(
        load_signed_at(&host, bundle, &policy, options("tampered"), 150),
        Err(LoadError::Verification(VerifyError::Artifact))
    ));
    assert_eq!(host.active_instances(), 0);
}
#[test]
fn unknown_imports_bad_metadata_and_core_modules_are_rejected() {
    let host = Host::new(1).unwrap();
    for (import, metadata) in [(UNKNOWN_IMPORT, false), ("", true)] {
        let (bundle, policy) =
            crate::tests::signed_component(fixture(Behavior::Complete, import, metadata, false));
        assert!(matches!(
            load_signed_at(&host, bundle, &policy, options("rejected"), 150),
            Err(LoadError::Runtime(_))
        ));
    }
    let (bundle, policy) = crate::tests::signed_component(wat::parse_str("(module)").unwrap());
    assert!(matches!(
        load_signed_at(&host, bundle, &policy, options("core"), 150),
        Err(LoadError::Verification(VerifyError::Compatibility))
    ));
}
#[test]
fn fuel_memory_and_traps_are_confined_to_the_instance() {
    let host = Host::new(2).unwrap();
    for (i, behavior) in [
        Behavior::Loop,
        Behavior::Grow,
        Behavior::Trap,
        Behavior::WrongResult,
    ]
    .into_iter()
    .enumerate()
    {
        let mut opts = options(&format!("bounded-{i}"));
        opts.runtime.fuel_per_operation = 100_000;
        let mut worker = load(&host, behavior, opts.clone());
        start(&worker, &opts);
        assert!(matches!(
            worker.submit(&opts.binding, next(&opts)).unwrap().wait(),
            Err(Error::Backend(_))
        ));
        worker.shutdown(Duration::from_secs(1)).unwrap();
    }
    let opts = options("healthy");
    let mut healthy = load(&host, Behavior::Complete, opts.clone());
    start(&healthy, &opts);
    healthy
        .submit(&opts.binding, next(&opts))
        .unwrap()
        .wait()
        .unwrap();
    healthy.shutdown(Duration::from_secs(1)).unwrap();
}
#[test]
fn cancel_bypasses_a_full_queue_and_does_not_interrupt_another_wasm_engine() {
    let host = Host::new(2).unwrap();
    let mut a = options("a");
    a.runtime.fuel_per_operation = u64::MAX;
    let mut worker_a = load(&host, Behavior::Loop, a.clone());
    start(&worker_a, &a);
    let running = worker_a.submit(&a.binding, next(&a)).unwrap();
    let start_wait = std::time::Instant::now();
    while !worker_a.is_executing() {
        assert!(start_wait.elapsed() < Duration::from_secs(1));
        std::thread::yield_now();
    }
    let queued = worker_a.submit(&a.binding, read(&a, 1024)).unwrap();
    assert!(matches!(
        worker_a.submit(&a.binding, next(&a)),
        Err(Error::Busy)
    ));
    let b = options("b");
    let mut worker_b = load(&host, Behavior::Complete, b.clone());
    start(&worker_b, &b);
    worker_b
        .submit(&b.binding, next(&b))
        .unwrap()
        .wait()
        .unwrap();
    worker_a.cancel();
    assert!(matches!(
        running.wait_timeout(Duration::from_secs(1)).unwrap(),
        Err(Error::Cancelled)
    ));
    assert!(matches!(queued.wait(), Err(Error::Cancelled)));
    worker_b
        .submit(&b.binding, read(&b, 1024))
        .unwrap()
        .wait()
        .unwrap();
    worker_a.shutdown(Duration::from_secs(1)).unwrap();
    worker_b.shutdown(Duration::from_secs(1)).unwrap();
}
#[test]
fn deadline_interrupts_an_infinite_wasm_loop() {
    let host = Host::new(1).unwrap();
    let mut opts = options("deadline");
    opts.worker.operation_timeout = Duration::from_millis(50);
    opts.runtime.fuel_per_operation = u64::MAX;
    let mut worker = load(&host, Behavior::Loop, opts.clone());
    start(&worker, &opts);
    assert!(matches!(
        worker
            .submit(&opts.binding, next(&opts))
            .unwrap()
            .wait_timeout(Duration::from_secs(2))
            .unwrap(),
        Err(Error::Timeout)
    ));
    worker.shutdown(Duration::from_secs(1)).unwrap();
}
#[test]
fn initialization_start_function_is_fuel_limited() {
    let host = Host::new(1).unwrap();
    let mut opts = options("init-loop");
    opts.runtime.fuel_per_operation = 100_000;
    let (bundle, policy) =
        crate::tests::signed_component(fixture(Behavior::Complete, "", false, true));
    assert!(matches!(
        load_signed_at(&host, bundle, &policy, opts, 150),
        Err(LoadError::Runtime(_))
    ));
}
#[test]
fn job_phase_binding_and_response_chunk_limits_are_enforced() {
    let host = Host::new(1).unwrap();
    let opts = options("phase");
    let mut worker = load(&host, Behavior::Complete, opts.clone());
    assert!(
        worker
            .submit(&opts.binding, next(&opts))
            .unwrap()
            .wait()
            .is_err()
    );
    worker.shutdown(Duration::from_secs(1)).unwrap();
    let mut worker = load(&host, Behavior::Complete, opts.clone());
    start(&worker, &opts);
    let foreign = pb::NextFileRequest {
        instance_id: opts.binding.instance_id.clone(),
        job_id: opts.job.job_id + 1,
    };
    assert!(matches!(
        worker
            .submit(&opts.binding, Operation::NextFile(foreign.clone()))
            .unwrap()
            .wait(),
        Err(Error::Binding)
    ));
    worker.shutdown(Duration::from_secs(1)).unwrap();
    let mut worker = load(&host, Behavior::Complete, opts.clone());
    start(&worker, &opts);
    assert!(matches!(
        worker.submit(&opts.binding, read(&opts, 1)).unwrap().wait(),
        Err(Error::TooLarge)
    ));
    worker.shutdown(Duration::from_secs(1)).unwrap();
}

#[test]
fn initialization_epoch_deadline_reclaims_the_instance_slot() {
    let host = Host::new(1).unwrap();
    let mut opts = options("init-deadline");
    opts.runtime.fuel_per_operation = u64::MAX;
    opts.runtime.initialization_timeout = Duration::from_millis(50);
    let (bundle, policy) =
        crate::tests::signed_component(fixture(Behavior::Complete, "", false, true));
    assert!(matches!(
        load_signed_at(&host, bundle, &policy, opts, 150),
        Err(LoadError::Runtime(_))
    ));
    let start = std::time::Instant::now();
    while host.active_instances() != 0 {
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "initialization worker did not exit"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn virtual_file_entries_carry_bounded_attributes() {
    let host = Host::new(1).unwrap();
    let opts = options("virtual");
    let mut worker = load(&host, Behavior::VirtualFile, opts.clone());
    start(&worker, &opts);
    let reply = worker.submit(&opts.binding, next(&opts)).unwrap().wait().unwrap();
    let Reply::NextFile(pb::NextFileResponse { item: Some(pb::next_file_response::Item::File(file)), .. }) = reply
    else {
        panic!("expected a file entry");
    };
    assert_eq!(file.kind, pb::FileKind::Virtual as i32);
    assert_eq!(file.attributes.get("database").map(String::as_str), Some("EleLa"));
    worker.shutdown(Duration::from_secs(1)).unwrap();

    let mut worker = load(&host, Behavior::BadAttribute, opts.clone());
    start(&worker, &opts);
    assert!(matches!(
        worker.submit(&opts.binding, next(&opts)).unwrap().wait(),
        Err(Error::Backend(message)) if message.contains("attribute name")
    ));
    worker.shutdown(Duration::from_secs(1)).unwrap();
}

#[test]
fn attribute_bounds() {
    use std::collections::HashMap;
    let one = |k: &str, v: &str| HashMap::from([(k.to_string(), v.to_string())]);
    assert!(attributes(&one("db.name-1_x", "EleLa")).is_ok());
    assert!(attributes(&one("", "x")).is_err());
    assert!(attributes(&one("a/b", "x")).is_err());
    assert!(attributes(&one(&"k".repeat(MAX_ATTRIBUTE_KEY + 1), "x")).is_err());
    assert!(attributes(&one("k", &"v".repeat(MAX_ATTRIBUTE_VALUE + 1))).is_err());
    assert!(attributes(&one("k", "line\nbreak")).is_err());
    let many: HashMap<_, _> = (0..=MAX_ATTRIBUTES).map(|i| (format!("k{i}"), String::new())).collect();
    assert!(attributes(&many).is_err());
}

#[test]
fn the_broker_import_needs_the_capability_and_a_port() {
    let host = Host::new(1).unwrap();
    let load = |caps: &[&str], broker: Option<BrokerHandle>, id: &str| {
        let (bundle, policy) =
            crate::tests::signed_component_with(fixture(Behavior::Complete, PG_IMPORT, false, false), caps);
        let mut opts = options(id);
        opts.broker = broker;
        load_signed_at(&host, bundle, &policy, opts, 150).map(|mut w| w.shutdown(Duration::from_secs(1)).unwrap())
    };
    // Granted and supplied: the guest instantiates.
    assert!(load(&["file.read", POSTGRESQL_READ], port(POSTGRESQL_READ), "pg-ok").is_ok());
    // Without the capability the port is refused, without the port the import is missing.
    assert!(matches!(load(&["file.read"], port(POSTGRESQL_READ), "pg-nocap"), Err(LoadError::Runtime(_))));
    assert!(matches!(load(&["file.read", POSTGRESQL_READ], None, "pg-noport"), Err(LoadError::Runtime(_))));
    // A port of another capability is not linked as PostgreSQL.
    assert!(matches!(load(&["file.read", POSTGRESQL_READ], port("docker.read"), "pg-other"), Err(LoadError::Runtime(_))));
}

#[test]
fn broker_calls_are_bounded() {
    let mut state = HostState {
        limits: StoreLimitsBuilder::new().build(),
        broker: port(POSTGRESQL_READ),
        max_broker_response: 8,
    };
    assert_eq!(broker_call(&mut state, BrokerFunction::Postgresql(PostgresqlFunction::Info), b"abc".to_vec()).unwrap(), b"cba");
    assert!(broker_call(&mut state, BrokerFunction::Postgresql(PostgresqlFunction::Info), vec![0; 9]).unwrap_err().contains("response"));
    assert!(broker_call(&mut state, BrokerFunction::Postgresql(PostgresqlFunction::Info), vec![0; MAX_BROKER_REQUEST + 1]).unwrap_err().contains("request"));
    state.broker = None;
    assert!(broker_call(&mut state, BrokerFunction::Postgresql(PostgresqlFunction::Info), Vec::new()).is_err());
}

const LDAP_IMPORT: &str = "(import \"onesimus:filed-plugin/openldap@1.0.0\" (instance \
    (export \"info\" (func (param \"request\" (list u8)) (result (result (list u8) (error string)))))))";
const LDAP_RESTORE_IMPORT: &str = "(import \"onesimus:filed-plugin/openldap-restore@1.0.0\" (instance \
    (export \"load-open\" (func (param \"request\" (list u8)) (result (result (list u8) (error string)))))))";

/// An OpenLDAP port: family openldap.read, serving openldap.write only
/// when `restore` (an online restore job).
struct LdapEcho {
    restore: bool,
}
impl BrokerPort for LdapEcho {
    fn capability(&self) -> &'static str {
        OPENLDAP_READ
    }
    fn serves(&self, capability: &str) -> bool {
        capability == OPENLDAP_READ || (self.restore && capability == OPENLDAP_WRITE)
    }
    fn call(&mut self, _function: BrokerFunction, request: &[u8]) -> Result<Vec<u8>, String> {
        Ok(request.to_vec())
    }
}
fn ldap_port(restore: bool) -> Option<BrokerHandle> {
    Some(BrokerHandle(Arc::new(Mutex::new(Box::new(LdapEcho { restore })))))
}

#[test]
fn each_port_links_only_its_own_broker_imports() {
    let host = Host::new(1).unwrap();
    let load = |import: &str, caps: &[&str], broker: Option<BrokerHandle>, id: &str| {
        let (bundle, policy) =
            crate::tests::signed_component_with(fixture(Behavior::Complete, import, false, false), caps);
        let mut opts = options(id);
        opts.broker = broker;
        load_signed_at(&host, bundle, &policy, opts, 150).map(|mut w| w.shutdown(Duration::from_secs(1)).unwrap())
    };
    // The OpenLDAP guest with its own port.
    assert!(load(LDAP_IMPORT, &["file.read", OPENLDAP_READ], ldap_port(false), "ldap-ok").is_ok());
    // An openldap.read plugin never gets the PostgreSQL import, and the other way round.
    assert!(matches!(load(PG_IMPORT, &["file.read", OPENLDAP_READ], ldap_port(false), "ldap-pg"), Err(LoadError::Runtime(_))));
    assert!(matches!(load(PG_IMPORT, &["file.read", POSTGRESQL_READ, OPENLDAP_READ], ldap_port(false), "ldap-pg2"),
        Err(LoadError::Runtime(_))));
    assert!(matches!(load(LDAP_IMPORT, &["file.read", POSTGRESQL_READ, OPENLDAP_READ], port(POSTGRESQL_READ), "pg-ldap"),
        Err(LoadError::Runtime(_))));
    // Without openldap.read the port is refused.
    assert!(matches!(load(LDAP_IMPORT, &["file.read"], ldap_port(false), "ldap-nocap"), Err(LoadError::Runtime(_))));
    // The restore import needs openldap.write in the manifest; with it the
    // guest instantiates in both jobs (failing functions in a backup).
    assert!(matches!(load(LDAP_RESTORE_IMPORT, &["file.read", OPENLDAP_READ], ldap_port(true), "ldap-nowrite"),
        Err(LoadError::Runtime(_))));
    assert!(load(LDAP_RESTORE_IMPORT, &["file.read", OPENLDAP_READ, OPENLDAP_WRITE], ldap_port(false), "ldap-w-backup").is_ok());
    assert!(load(LDAP_RESTORE_IMPORT, &["file.read", OPENLDAP_READ, OPENLDAP_WRITE], ldap_port(true), "ldap-w-restore").is_ok());
}

#[test]
fn openldap_write_is_live_only_where_the_port_serves_it() {
    let caps = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let live = |granted: &[String], restore: bool| -> Vec<(&str, bool)> {
        let port = LdapEcho { restore };
        import_plan(granted, OPENLDAP_READ, &|c| port.serves(c)).unwrap().into_iter().map(|(i, l)| (i.capability, l)).collect()
    };
    let all = caps(&[OPENLDAP_READ, OPENLDAP_WRITE]);
    assert_eq!(live(&all, false), [(OPENLDAP_READ, true), (OPENLDAP_WRITE, false)]);
    assert_eq!(live(&all, true), [(OPENLDAP_READ, true), (OPENLDAP_WRITE, true)]);
    assert_eq!(live(&caps(&[OPENLDAP_READ]), true), [(OPENLDAP_READ, true)]);
    // openldap.write alone is no family; postgresql.read never brings openldap imports.
    assert!(import_plan(&caps(&[OPENLDAP_WRITE]), OPENLDAP_WRITE, &|_| true).is_err());
    let pg = import_plan(&caps(&[POSTGRESQL_READ, OPENLDAP_READ, OPENLDAP_WRITE]), POSTGRESQL_READ, &|_| true).unwrap();
    assert_eq!(pg.iter().map(|(i, _)| i.import).collect::<Vec<_>>(), ["onesimus:filed-plugin/postgresql@1.0.0"]);
}

/// Podman imports: `podman` with podman.inventory, `podman-volume` only
/// with podman.volume.read in addition; never for another family.
#[test]
fn podman_imports_follow_the_grants() {
    let caps = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let plan = |granted: &[String]| -> Vec<&str> {
        import_plan(granted, PODMAN_INVENTORY, &|_| true).unwrap().into_iter().map(|(i, _)| i.import).collect()
    };
    assert_eq!(plan(&caps(&[PODMAN_INVENTORY])), ["onesimus:filed-plugin/podman@1.0.0"]);
    assert_eq!(plan(&caps(&[PODMAN_INVENTORY, PODMAN_VOLUME_READ, PODMAN_ROOTFUL])),
        ["onesimus:filed-plugin/podman@1.0.0", "onesimus:filed-plugin/podman-volume@1.0.0"]);
    // podman.volume.read alone is no family; another family gets no podman import.
    assert!(import_plan(&caps(&[PODMAN_VOLUME_READ]), PODMAN_VOLUME_READ, &|_| true).is_err());
    let pg = import_plan(&caps(&[POSTGRESQL_READ, PODMAN_INVENTORY]), POSTGRESQL_READ, &|_| true).unwrap();
    assert_eq!(pg.len(), 1);
}

/// FileEntry v2: scan references only from contract 2 on; FIFO and
/// HARDLINK only with a reference.
#[test]
fn scan_references_need_contract_two() {
    let entry = |kind: pb::FileKind, source_ref: &[u8]| pb::FileEntry {
        path: b"/@podman/x".to_vec(), kind: kind as i32, source_ref: source_ref.to_vec(), ..Default::default()
    };
    assert!(entry_kind(&entry(pb::FileKind::Regular, b""), 1).is_ok());
    assert!(entry_kind(&entry(pb::FileKind::Regular, b"r"), 1).is_err());
    assert!(entry_kind(&entry(pb::FileKind::Regular, b"r"), 2).is_ok());
    assert!(entry_kind(&entry(pb::FileKind::Regular, &[0; 65]), 2).is_err());
    for kind in [pb::FileKind::Fifo, pb::FileKind::Hardlink] {
        assert!(entry_kind(&entry(kind, b""), 2).is_err());
        assert!(entry_kind(&entry(kind, b"r"), 2).is_ok());
        assert!(entry_kind(&entry(kind, b"r"), 1).is_err());
    }
    assert!(entry_kind(&entry(pb::FileKind::Unspecified, b"r"), 2).is_err());
}
