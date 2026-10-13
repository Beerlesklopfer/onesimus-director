//! MPSC command queue with oneshot result delivery.
//!
//! Multiple producers (console sessions, gRPC handlers) submit commands.
//! Single consumer (executor thread) takes and processes them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::{Sender, Receiver, bounded, TrySendError};

use crate::command_ast::CommandAst;

/// Result of command execution.
///
/// `text_output` is the human/API-mode rendering. `json` is the structured
/// result for JSON API mode (`.api 2`); the session wraps it into the
/// json-rpc envelope (see json_output.rs). Commands without a JSON rendering
/// leave it `None` and yield an empty result object, as in C++.
#[derive(Debug, Clone, Default)]
pub struct CommandResult {
    pub success: bool,
    pub text_output: String,
    pub error_message: String,
    pub json: Option<serde_json::Value>,
}

impl CommandResult {
    pub fn ok(text: String) -> Self {
        Self { success: true, text_output: text, ..Default::default() }
    }

    pub fn error(msg: String) -> Self {
        Self { success: false, error_message: msg, ..Default::default() }
    }

    /// Attach the structured JSON result (used in `.api 2` mode).
    pub fn with_json(mut self, json: serde_json::Value) -> Self {
        self.json = Some(json);
        self
    }
}

/// A command queued for execution with a channel to send the result back.
pub struct QueuedCommand {
    pub ast: CommandAst,
    pub session_id: u64,
    pub result_tx: Sender<CommandResult>,
}

/// MPSC command queue.
pub struct CommandQueue {
    tx: Sender<QueuedCommand>,
    rx: Receiver<QueuedCommand>,
    shutdown: AtomicBool,
}

impl CommandQueue {
    pub fn new(capacity: usize) -> Self {
        let (tx, rx) = bounded(capacity);
        Self {
            tx,
            rx,
            shutdown: AtomicBool::new(false),
        }
    }

    /// Submit a command and get a Receiver for the result.
    /// Returns None if the queue is full or shut down.
    pub fn submit(&self, ast: CommandAst, session_id: u64) -> Option<Receiver<CommandResult>> {
        if self.shutdown.load(Ordering::Relaxed) {
            return None;
        }

        let (result_tx, result_rx) = bounded(1);
        let cmd = QueuedCommand { ast, session_id, result_tx };

        match self.tx.try_send(cmd) {
            Ok(()) => Some(result_rx),
            Err(TrySendError::Full(_)) => None,
            Err(TrySendError::Disconnected(_)) => None,
        }
    }

    /// Take the next command (blocking). Returns None on shutdown.
    pub fn take(&self) -> Option<QueuedCommand> {
        match self.rx.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => Some(cmd),
            Err(_) => {
                if self.shutdown.load(Ordering::Relaxed) {
                    None
                } else {
                    // Timeout, not shutdown — caller should retry
                    None
                }
            }
        }
    }

    /// Try to take without blocking.
    pub fn try_take(&self) -> Option<QueuedCommand> {
        self.rx.try_recv().ok()
    }

    /// Signal shutdown.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    /// Number of pending commands.
    pub fn len(&self) -> usize {
        self.rx.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rx.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_ast::{CommandSource, ResponseFormat};

    fn make_ast(name: &str) -> CommandAst {
        CommandAst {
            command_name: name.to_string(),
            arguments: Vec::new(),
            session_id: String::new(),
            source: CommandSource::Cli,
            response_format: ResponseFormat::Default,
        }
    }

    #[test]
    fn submit_and_take() {
        let queue = CommandQueue::new(16);
        let result_rx = queue.submit(make_ast("status"), 1).unwrap();

        let cmd = queue.try_take().unwrap();
        assert_eq!(cmd.ast.command_name, "status");
        assert_eq!(cmd.session_id, 1);

        // Send result back
        cmd.result_tx.send(CommandResult::ok("OK".into())).unwrap();

        let result = result_rx.recv().unwrap();
        assert!(result.success);
        assert_eq!(result.text_output, "OK");
    }

    #[test]
    fn multiple_commands() {
        let queue = CommandQueue::new(16);
        queue.submit(make_ast("cmd1"), 1).unwrap();
        queue.submit(make_ast("cmd2"), 2).unwrap();
        queue.submit(make_ast("cmd3"), 3).unwrap();

        assert_eq!(queue.len(), 3);

        let c1 = queue.try_take().unwrap();
        assert_eq!(c1.ast.command_name, "cmd1");

        let c2 = queue.try_take().unwrap();
        assert_eq!(c2.ast.command_name, "cmd2");

        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn bounded_queue_full() {
        let queue = CommandQueue::new(2);
        assert!(queue.submit(make_ast("a"), 1).is_some());
        assert!(queue.submit(make_ast("b"), 2).is_some());
        assert!(queue.submit(make_ast("c"), 3).is_none()); // Full
    }

    #[test]
    fn shutdown_rejects_submit() {
        let queue = CommandQueue::new(16);
        queue.shutdown();
        assert!(queue.submit(make_ast("x"), 1).is_none());
    }

    #[test]
    fn empty_queue() {
        let queue = CommandQueue::new(16);
        assert!(queue.is_empty());
        assert!(queue.try_take().is_none());
    }

    #[test]
    fn multi_producer() {
        use std::sync::Arc;

        let queue = Arc::new(CommandQueue::new(100));
        let mut handles = Vec::new();

        for i in 0..10 {
            let q = queue.clone();
            handles.push(std::thread::spawn(move || {
                for j in 0..10 {
                    q.submit(make_ast(&format!("cmd-{i}-{j}")), i as u64);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(queue.len(), 100);
    }

    #[test]
    fn result_delivery_error() {
        let queue = CommandQueue::new(16);
        let result_rx = queue.submit(make_ast("bad"), 1).unwrap();

        let cmd = queue.try_take().unwrap();
        cmd.result_tx.send(CommandResult::error("not found".into())).unwrap();

        let result = result_rx.recv().unwrap();
        assert!(!result.success);
        assert_eq!(result.error_message, "not found");
    }
}
