//! MPSC command queue with oneshot result delivery.
//!
//! Multiple producers (console sessions, gRPC handlers) submit commands.
//! Single consumer (executor thread) takes and processes them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::{Sender, Receiver, bounded, TrySendError};

use crate::command_ast::CommandAst;

/// Message kinds of UaContext::ErrorMsg/WarningMsg/InfoMsg (C++
/// MSG_TYPE_ERROR/WARNING/INFO; signals BNET_ERROR_MSG/WARNING_MSG/INFO_MSG).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgKind {
    Error,
    Warning,
    Info,
}

impl MsgKind {
    /// Key of the `.api 2` messages object (output_formatter.h MSG_TYPE_*).
    pub fn json_key(self) -> &'static str {
        match self {
            MsgKind::Error => "error",
            MsgKind::Warning => "warning",
            MsgKind::Info => "info",
        }
    }
}

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
    /// The ErrorMsg lines were emitted before the SendMsg text (C++ order).
    pub errors_first: bool,
    /// `error_message` is the text of a single ErrorMsg call spanning
    /// several lines (one BNET_ERROR_MSG, one JSON error entry).
    pub single_error: bool,
    /// Send `text_output` as is, without the terminating newline the
    /// session otherwise appends (C++ SendMsg of text without "\n").
    pub verbatim: bool,
    /// Ordered ErrorMsg/WarningMsg/InfoMsg calls (one entry per call). Their
    /// text is already part of `text_output`/`error_message`; the entries
    /// drive the message signals and the `.api 2` messages lists. When no
    /// Error entry is present, the errors come from `error_message`.
    pub messages: Vec<(MsgKind, String)>,
    /// `text_output` already contains the ErrorMsg text in call order
    /// (`error_message` is a copy for inspection, `messages` drive the
    /// signals); the session sends `text_output` only.
    pub errors_inline: bool,
}

impl CommandResult {
    pub fn ok(text: String) -> Self {
        Self { success: true, text_output: text, ..Default::default() }
    }

    pub fn error(msg: String) -> Self {
        Self { success: false, error_message: msg, ..Default::default() }
    }

    /// Successful command that also emitted ErrorMsg lines (C++ commands
    /// returning true after ua->ErrorMsg); `.api 2` reports it as failed.
    pub fn ok_with_errors(text: String, errors: String) -> Self {
        Self { success: true, text_output: text, error_message: errors, ..Default::default() }
    }

    /// Mark `error_message` as one multi-line ErrorMsg call.
    pub fn with_error_parts(mut self) -> Self {
        self.single_error = true;
        self
    }

    /// Send the text without an appended newline.
    pub fn verbatim(mut self) -> Self {
        self.verbatim = true;
        self
    }

    /// C++ ua->WarningMsg() after the text so far: the text goes to the
    /// output, the call is recorded for BNET_WARNING_MSG and the `.api 2`
    /// "warning" list.
    pub fn with_warning(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.text_output.push_str(&text);
        self.messages.push((MsgKind::Warning, text));
        self
    }

    /// C++ ua->InfoMsg() after the text so far (BNET_INFO_MSG, "info").
    pub fn with_info(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        self.text_output.push_str(&text);
        self.messages.push((MsgKind::Info, text));
        self
    }

    /// All ErrorMsg/WarningMsg/InfoMsg calls in order. Results without
    /// recorded Error entries take the errors from `error_message`, placed
    /// before or after the other messages like the error text itself.
    pub fn message_list(&self) -> Vec<(MsgKind, String)> {
        if self.messages.iter().any(|(kind, _)| *kind == MsgKind::Error) {
            return self.messages.clone();
        }
        let errors = self.error_lines().into_iter().map(|line| (MsgKind::Error, line));
        if self.errors_first {
            errors.chain(self.messages.iter().cloned()).collect()
        } else {
            self.messages.iter().cloned().chain(errors).collect()
        }
    }

    /// One entry per ErrorMsg call: each line keeps its trailing newline.
    pub fn error_lines(&self) -> Vec<String> {
        if self.single_error && !self.error_message.is_empty() {
            return vec![self.error_message.clone()];
        }
        // A blank line belongs to the message before it (one ErrorMsg such
        // as "...ERR=<errmsg>\n\n" is one JSON message).
        let mut lines: Vec<String> = Vec::new();
        for l in self.error_message.lines() {
            match lines.last_mut() {
                Some(last) if l.trim().is_empty() => last.push('\n'),
                _ if l.trim().is_empty() => {}
                _ => lines.push(format!("{l}\n")),
            }
        }
        lines
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
            raw_line: String::new(),
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
