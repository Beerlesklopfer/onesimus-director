//! Instance workers for trusted WASM-host adapters. This is scheduling, not a
//! component engine: an adapter must enforce memory/fuel limits and provide a
//! nonblocking, thread-safe interrupt handle for both WASM and host I/O.
use crate::VerifiedComponent;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    mpsc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub instance_id: String,
    pub job_id: u64,
    pub device_id: String,
    pub worker_id: String,
}
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_pending_operations: usize,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub operation_timeout: Duration,
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("invalid plugin worker configuration")]
    Configuration,
    #[error("plugin worker or queue capacity exhausted")]
    Busy,
    #[error("request belongs to another plugin/job/device")]
    Binding,
    #[error("plugin request or response exceeds byte limit")]
    TooLarge,
    #[error("plugin operation cancelled")]
    Cancelled,
    #[error("plugin operation timed out")]
    Timeout,
    #[error("plugin worker stopped")]
    Stopped,
    #[error("plugin host adapter failed: {0}")]
    Backend(String),
    #[error("plugin worker did not stop before deadline")]
    Unresponsive,
}

/// Implemented by typed host requests/replies, never an unbounded raw socket.
pub trait BoundedMessage: Send + 'static {
    fn byte_size(&self) -> usize;
}
/// Must return promptly even while the worker is executing. An implementation
/// signals the engine and aborts broker I/O; it may not acquire a worker lock.
pub trait Interrupt: Send + Sync + 'static {
    fn interrupt(&self);
}
pub trait Backend: Send + 'static {
    type Request: BoundedMessage;
    type Reply: BoundedMessage;
    fn interrupt_handle(&self) -> Arc<dyn Interrupt>;
    fn invoke(&mut self, request: Self::Request, context: &Context) -> Result<Self::Reply, Error>;
}

const RUNNING: u8 = 0;
const CANCELLED: u8 = 1;
const TIMEOUT: u8 = 2;
const STOPPED: u8 = 3;
struct State {
    status: AtomicU8,
    deadline: Mutex<Option<Instant>>,
    changed: Condvar,
    interrupt: Arc<dyn Interrupt>,
    exited: AtomicBool,
}
impl State {
    fn error(&self) -> Option<Error> {
        match self.status.load(Ordering::Acquire) {
            RUNNING => None,
            CANCELLED => Some(Error::Cancelled),
            TIMEOUT => Some(Error::Timeout),
            _ => Some(Error::Stopped),
        }
    }
    fn stop(&self, reason: u8) {
        if self
            .status
            .compare_exchange(RUNNING, reason, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.interrupt.interrupt();
        }
        // Lock pairs notification with the watchdog's condition check.
        let _guard = self.deadline.lock().unwrap_or_else(|e| e.into_inner());
        self.changed.notify_all();
    }
}
#[derive(Clone)]
pub struct Context {
    state: Arc<State>,
}
impl Context {
    pub fn check(&self) -> Result<(), Error> {
        self.state.error().map_or(Ok(()), Err)
    }
}
struct Work<B: Backend> {
    request: B::Request,
    reply: mpsc::SyncSender<Result<B::Reply, Error>>,
}
pub struct Ticket<T> {
    reply: mpsc::Receiver<Result<T, Error>>,
}
impl<T> Ticket<T> {
    pub fn wait(self) -> Result<T, Error> {
        self.reply.recv().map_err(|_| Error::Stopped)?
    }
    pub fn wait_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Result<T, Error>, mpsc::RecvTimeoutError> {
        self.reply.recv_timeout(timeout)
    }
}

/// Global instance admission limit. Admission does not serialize plugin work.
pub struct Host {
    active: Arc<AtomicUsize>,
    instances: Arc<Mutex<std::collections::BTreeSet<String>>>,
    max_instances: usize,
}
impl Host {
    pub fn new(max_instances: usize) -> Result<Self, Error> {
        if max_instances == 0 {
            return Err(Error::Configuration);
        }
        Ok(Self {
            active: Arc::new(AtomicUsize::new(0)),
            instances: Default::default(),
            max_instances,
        })
    }
    pub fn active_instances(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
    /// The verified bytes move into a dedicated worker. The factory is trusted
    /// host code, not a way to load native/Python plugins. It creates the store
    /// on its owning thread and applies the engine's memory/fuel policy.
    pub fn spawn<B, F>(
        &self,
        component: VerifiedComponent,
        binding: Binding,
        limits: Limits,
        factory: F,
    ) -> Result<Worker<B>, Error>
    where
        B: Backend,
        F: FnOnce(VerifiedComponent) -> Result<B, Error> + Send + 'static,
    {
        let timeout = limits.operation_timeout;
        self.spawn_with_initialization_timeout(component, binding, limits, timeout, factory)
    }

    /// Initialization/compilation has its own admission deadline. An
    /// unresponsive adapter retains its instance slot until its thread exits.
    pub fn spawn_with_initialization_timeout<B, F>(
        &self,
        component: VerifiedComponent,
        binding: Binding,
        limits: Limits,
        initialization_timeout: Duration,
        factory: F,
    ) -> Result<Worker<B>, Error>
    where
        B: Backend,
        F: FnOnce(VerifiedComponent) -> Result<B, Error> + Send + 'static,
    {
        if initialization_timeout.is_zero()
            || binding.instance_id.is_empty()
            || binding.job_id == 0
            || binding.device_id.is_empty()
            || binding.worker_id.is_empty()
            || limits.max_pending_operations == 0
            || limits.max_request_bytes == 0
            || limits.max_response_bytes == 0
            || limits.operation_timeout.is_zero()
            || Instant::now()
                .checked_add(limits.operation_timeout)
                .is_none()
        {
            return Err(Error::Configuration);
        }
        let mut instances = self.instances.lock().unwrap_or_else(|e| e.into_inner());
        if instances.contains(&binding.instance_id) {
            return Err(Error::Binding);
        }
        self.active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max_instances).then_some(n + 1)
            })
            .map_err(|_| Error::Busy)?;
        instances.insert(binding.instance_id.clone());
        let permit = Permit {
            active: self.active.clone(),
            instances: self.instances.clone(),
            id: binding.instance_id.clone(),
        };
        drop(instances);
        let (send, receive) = mpsc::sync_channel::<Work<B>>(limits.max_pending_operations);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker_limits = limits.clone();
        let handle = thread::Builder::new()
            .name("plugin:instance".into())
            .spawn(move || {
                let _permit = permit;
                let backend =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| factory(component)));
                let mut backend = match backend {
                    Ok(Ok(backend)) => backend,
                    Ok(Err(error)) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                    Err(_) => {
                        let _ = ready_tx.send(Err(Error::Backend("initialization panic".into())));
                        return;
                    }
                };
                let state = Arc::new(State {
                    status: AtomicU8::new(RUNNING),
                    deadline: Mutex::new(None),
                    changed: Condvar::new(),
                    interrupt: backend.interrupt_handle(),
                    exited: AtomicBool::new(false),
                });
                let exit = ExitState(state.clone());
                let watch = state.clone();
                let watchdog = match thread::Builder::new()
                    .name("plugin:deadline".into())
                    .spawn(move || watchdog(watch))
                {
                    Ok(handle) => handle,
                    Err(error) => {
                        let _ = ready_tx.send(Err(Error::Backend(error.to_string())));
                        return;
                    }
                };
                if ready_tx.send(Ok(state.clone())).is_err() {
                    state.stop(STOPPED);
                }
                let context = Context {
                    state: state.clone(),
                };
                loop {
                    if state.error().is_some() {
                        break;
                    }
                    let work = match receive.recv_timeout(Duration::from_millis(10)) {
                        Ok(work) => work,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let mut deadline = state.deadline.lock().unwrap_or_else(|e| e.into_inner());
                    *deadline = Some(Instant::now() + worker_limits.operation_timeout);
                    state.changed.notify_all();
                    drop(deadline);
                    let result = match context.check() {
                        Err(error) => Err(error),
                        Ok(()) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            backend.invoke(work.request, &context)
                        }))
                        .unwrap_or_else(|_| Err(Error::Backend("operation panic".into()))),
                    };
                    let mut deadline = state.deadline.lock().unwrap_or_else(|e| e.into_inner());
                    let expired = deadline.is_some_and(|time| Instant::now() >= time);
                    *deadline = None;
                    state.changed.notify_all();
                    drop(deadline);
                    if expired {
                        state.stop(TIMEOUT);
                    }
                    let result = context.check().and(result).and_then(|reply| {
                        if reply.byte_size() > worker_limits.max_response_bytes {
                            Err(Error::TooLarge)
                        } else {
                            Ok(reply)
                        }
                    });
                    let failed = result.is_err();
                    let _ = work.reply.send(result);
                    if failed {
                        state.stop(STOPPED);
                        break;
                    }
                }
                let reason = state.error().unwrap_or(Error::Stopped);
                while let Ok(work) = receive.try_recv() {
                    let _ = work.reply.send(Err(reason.clone()));
                }
                drop(backend);
                drop(exit);
                let _ = watchdog.join();
            })
            .map_err(|e| Error::Backend(e.to_string()))?;
        let state =
            ready_rx
                .recv_timeout(initialization_timeout)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => Error::Unresponsive,
                    mpsc::RecvTimeoutError::Disconnected => Error::Stopped,
                })??;
        Ok(Worker {
            binding,
            limits,
            send,
            state,
            handle: Some(handle),
        })
    }
}
struct Permit {
    active: Arc<AtomicUsize>,
    instances: Arc<Mutex<std::collections::BTreeSet<String>>>,
    id: String,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.instances
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}
struct ExitState(Arc<State>);
impl Drop for ExitState {
    fn drop(&mut self) {
        self.0.exited.store(true, Ordering::Release);
        let _guard = self.0.deadline.lock().unwrap_or_else(|e| e.into_inner());
        self.0.changed.notify_all();
    }
}
fn watchdog(state: Arc<State>) {
    let mut deadline = state.deadline.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if state.exited.load(Ordering::Acquire) {
            return;
        }
        if state.status.load(Ordering::Acquire) != RUNNING {
            deadline = state
                .changed
                .wait(deadline)
                .unwrap_or_else(|e| e.into_inner());
            continue;
        }
        match *deadline {
            Some(time) if Instant::now() >= time => {
                drop(deadline);
                state.stop(TIMEOUT);
                deadline = state.deadline.lock().unwrap_or_else(|e| e.into_inner());
            }
            Some(time) => {
                deadline = state
                    .changed
                    .wait_timeout(deadline, time.saturating_duration_since(Instant::now()))
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            None => {
                deadline = state
                    .changed
                    .wait(deadline)
                    .unwrap_or_else(|e| e.into_inner())
            }
        }
    }
}

pub struct Worker<B: Backend> {
    binding: Binding,
    limits: Limits,
    send: mpsc::SyncSender<Work<B>>,
    state: Arc<State>,
    handle: Option<JoinHandle<()>>,
}
impl<B: Backend> Worker<B> {
    pub fn submit(
        &self,
        binding: &Binding,
        request: B::Request,
    ) -> Result<Ticket<B::Reply>, Error> {
        if binding != &self.binding {
            return Err(Error::Binding);
        }
        if request.byte_size() > self.limits.max_request_bytes {
            return Err(Error::TooLarge);
        }
        if let Some(error) = self.state.error() {
            return Err(error);
        }
        let (reply, receive) = mpsc::sync_channel(1);
        self.send
            .try_send(Work { request, reply })
            .map_err(|e| match e {
                mpsc::TrySendError::Full(_) => Error::Busy,
                mpsc::TrySendError::Disconnected(_) => Error::Stopped,
            })?;
        Ok(Ticket { reply: receive })
    }
    pub fn is_executing(&self) -> bool {
        self.state
            .deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
    pub fn cancel(&self) {
        self.state.stop(CANCELLED);
    }
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), Error> {
        self.state.stop(STOPPED);
        let start = Instant::now();
        let mut guard = self
            .state
            .deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while !self.state.exited.load(Ordering::Acquire) {
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(Error::Unresponsive);
            }
            guard = self
                .state
                .changed
                .wait_timeout(guard, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(guard);
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| Error::Backend("worker panic".into()))?;
        }
        Ok(())
    }
}
impl<B: Backend> Drop for Worker<B> {
    fn drop(&mut self) {
        self.state.stop(STOPPED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Message(Vec<u8>);
    impl BoundedMessage for Message {
        fn byte_size(&self) -> usize {
            self.0.len()
        }
    }
    enum Request {
        Echo(Vec<u8>),
        Block,
        Panic,
        Large,
    }
    impl BoundedMessage for Request {
        fn byte_size(&self) -> usize {
            match self {
                Self::Echo(bytes) => bytes.len(),
                _ => 1,
            }
        }
    }
    struct Gate {
        stopped: Mutex<bool>,
        changed: Condvar,
    }
    impl Interrupt for Gate {
        fn interrupt(&self) {
            *self.stopped.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }
    struct TestBackend {
        gate: Arc<Gate>,
        started: mpsc::SyncSender<()>,
    }
    impl Backend for TestBackend {
        type Request = Request;
        type Reply = Message;
        fn interrupt_handle(&self) -> Arc<dyn Interrupt> {
            self.gate.clone()
        }
        fn invoke(&mut self, request: Request, context: &Context) -> Result<Message, Error> {
            match request {
                Request::Echo(bytes) => Ok(Message(bytes)),
                Request::Large => Ok(Message(vec![0; 1025])),
                Request::Panic => panic!("simulated host adapter failure"),
                Request::Block => {
                    self.started.send(()).unwrap();
                    let mut stopped = self.gate.stopped.lock().unwrap();
                    while !*stopped {
                        stopped = self.gate.changed.wait(stopped).unwrap();
                    }
                    context.check()?;
                    Ok(Message(Vec::new()))
                }
            }
        }
    }
    fn component() -> VerifiedComponent {
        VerifiedComponent {
            manifest: crate::Manifest {
                schema_version: crate::MANIFEST_SCHEMA_VERSION,
                plugin_id: "test".into(),
                version: "1".into(),
                api_version: 1,
                artifact_sha256: String::new(),
                signing_key_id: "test".into(),
                not_before: 0,
                not_after: u64::MAX,
                capabilities: vec![],
                target: "test".into(),
            },
            component: Vec::new(),
            certificate_sha256: "test".into(),
        }
    }
    fn binding(id: &str) -> Binding {
        Binding {
            instance_id: id.into(),
            job_id: 1,
            device_id: format!("device-{id}"),
            worker_id: format!("worker-{id}"),
        }
    }
    fn spawn(
        host: &Host,
        id: &str,
        timeout: Duration,
    ) -> (Worker<TestBackend>, mpsc::Receiver<()>) {
        let (started, receive) = mpsc::sync_channel(1);
        let worker = host
            .spawn(
                component(),
                binding(id),
                Limits {
                    max_pending_operations: 1,
                    max_request_bytes: 1024,
                    max_response_bytes: 1024,
                    operation_timeout: timeout,
                },
                move |_| {
                    Ok(TestBackend {
                        gate: Arc::new(Gate {
                            stopped: Mutex::new(false),
                            changed: Condvar::new(),
                        }),
                        started,
                    })
                },
            )
            .unwrap();
        (worker, receive)
    }
    #[test]
    fn blocked_instance_does_not_block_other_workers_and_cancel_bypasses_full_queue() {
        let host = Host::new(2).unwrap();
        let (mut a, started) = spawn(&host, "a", Duration::from_secs(5));
        let (mut b, _) = spawn(&host, "b", Duration::from_secs(5));
        let running = a.submit(&binding("a"), Request::Block).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        let queued = a.submit(&binding("a"), Request::Echo(vec![1])).unwrap();
        assert!(matches!(
            a.submit(&binding("a"), Request::Echo(vec![2])),
            Err(Error::Busy)
        ));
        assert_eq!(
            b.submit(&binding("b"), Request::Echo(vec![7]))
                .unwrap()
                .wait_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap()
                .0,
            vec![7]
        );
        a.cancel();
        assert!(matches!(running.wait(), Err(Error::Cancelled)));
        assert!(matches!(queued.wait(), Err(Error::Cancelled)));
        a.shutdown(Duration::from_secs(1)).unwrap();
        b.shutdown(Duration::from_secs(1)).unwrap();
        assert_eq!(host.active_instances(), 0);
    }
    #[test]
    fn deadline_interrupts_a_blocked_operation_without_a_queued_control_message() {
        let host = Host::new(1).unwrap();
        let (mut worker, started) = spawn(&host, "timeout", Duration::from_millis(50));
        let ticket = worker.submit(&binding("timeout"), Request::Block).unwrap();
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            ticket.wait_timeout(Duration::from_secs(1)).unwrap(),
            Err(Error::Timeout)
        ));
        worker.shutdown(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn binding_byte_limits_ordering_and_panics_are_isolated() {
        let host = Host::new(2).unwrap();
        let (mut worker, _) = spawn(&host, "limits", Duration::from_secs(1));
        assert!(matches!(
            worker.submit(&binding("other"), Request::Echo(vec![])),
            Err(Error::Binding)
        ));
        assert!(matches!(
            worker.submit(&binding("limits"), Request::Echo(vec![0; 1025])),
            Err(Error::TooLarge)
        ));
        for n in 0..10 {
            assert_eq!(
                worker
                    .submit(&binding("limits"), Request::Echo(vec![n]))
                    .unwrap()
                    .wait()
                    .unwrap()
                    .0,
                vec![n]
            );
        }
        assert!(matches!(
            worker
                .submit(&binding("limits"), Request::Large)
                .unwrap()
                .wait(),
            Err(Error::TooLarge)
        ));
        worker.shutdown(Duration::from_secs(1)).unwrap();
        let (mut panicking, _) = spawn(&host, "panic", Duration::from_secs(1));
        assert!(matches!(
            panicking
                .submit(&binding("panic"), Request::Panic)
                .unwrap()
                .wait(),
            Err(Error::Backend(_))
        ));
        panicking.shutdown(Duration::from_secs(1)).unwrap();
        assert_eq!(host.active_instances(), 0);
    }
    #[test]
    fn instance_admission_is_bounded() {
        let host = Host::new(1).unwrap();
        let (mut worker, _) = spawn(&host, "one", Duration::from_secs(1));
        let duplicate = host.spawn::<TestBackend, _>(
            component(),
            binding("one"),
            worker.limits.clone(),
            |_| unreachable!(),
        );
        assert!(matches!(duplicate, Err(Error::Binding)));
        let extra = host.spawn::<TestBackend, _>(
            component(),
            binding("two"),
            worker.limits.clone(),
            |_| unreachable!(),
        );
        assert!(matches!(extra, Err(Error::Busy)));
        worker.shutdown(Duration::from_secs(1)).unwrap();
    }
}
