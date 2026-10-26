use super::*;
use crate::executor::Error;

#[derive(Clone, Copy)]
enum Behavior {
    Complete,
    Loop,
    Grow,
    Trap,
    WrongResult,
}
fn escaped(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}
fn fixture(
    behavior: Behavior,
    unknown_import: bool,
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
    let next = pb::NextFileResponse {
        result: Some(result.clone()),
        item: Some(pb::next_file_response::Item::Complete(pb::ScanComplete {})),
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
    let import = if unknown_import {
        "(import \"ungranted-host-access\" (func))"
    } else {
        ""
    };
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
    }
}
fn load(host: &Host, behavior: Behavior, opts: LoadOptions) -> Worker<WasmBackend> {
    let (bundle, policy) = crate::tests::signed_component(fixture(behavior, false, false, false));
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
        crate::tests::signed_component(fixture(Behavior::Complete, false, false, false));
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
    for (imports, metadata) in [(true, false), (false, true)] {
        let (bundle, policy) =
            crate::tests::signed_component(fixture(Behavior::Complete, imports, metadata, false));
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
        crate::tests::signed_component(fixture(Behavior::Complete, false, false, true));
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
        crate::tests::signed_component(fixture(Behavior::Complete, false, false, true));
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
