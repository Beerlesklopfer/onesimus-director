//! S-35 execution of the write, session and restore-tree RPCs (only with the
//! cargo feature `grpc-development`). Plans come from api_write.rs.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};
use prost::Message;
use rand::RngCore;
use serde_json::{json, Map, Value};
use tonic::{Request, Response, Status};

use bareos_bnet::BnetSignal;
use onesimus_proto_rs::onesimus::director::api as pb;
use onesimus_proto_rs::typed;

use super::api::DirectorApiImpl;
use super::api_write::{self, AnswerConsole, Follow, Mode, Plan};
use super::coverage::{LOGOUT, NOT_APPLICABLE};
use super::service::run_command;
use crate::command_ast::{CommandSource, ResponseFormat};
use crate::command_executor::{self, ApiMode};
use crate::command_queue::{CommandResult, MsgKind};
use crate::ua_select::Console;

/// Idle time after which a restore session ends (its dialog is cancelled).
pub const RESTORE_IDLE: Duration = Duration::from_secs(600);
/// Longest wait for one reply of the restore dialog.
const REPLY_WAIT: Duration = Duration::from_secs(600);

fn decode<Q: Message>(rpc: &str, request: &Q) -> Result<Map<String, Value>, Status> {
    let (input, _) = typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, rpc)
        .ok_or_else(|| Status::internal(format!("no descriptor for {rpc}")))?;
    match typed::message_to_json(&input, &request.encode_to_vec()) {
        Value::Object(m) => Ok(m),
        _ => Ok(Map::new()),
    }
}

fn encode<R: Message + Default>(rpc: &str, object: Map<String, Value>) -> Result<R, Status> {
    let (_, output) = typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, rpc)
        .ok_or_else(|| Status::internal(format!("no descriptor for {rpc}")))?;
    let encoded = typed::json_to_message(&output, &Value::Object(object));
    R::decode(encoded.bytes.as_slice()).map_err(|e| Status::internal(format!("{rpc}: {e}")))
}

fn token(metadata: &tonic::metadata::MetadataMap) -> String {
    metadata.get(super::auth::SESSION_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
}

/// Output text of a finished command (console transcript first).
fn reply_text(console_text: &str, result: &CommandResult) -> String {
    let mut text = console_text.to_string();
    text.push_str(&result.text_output);
    if !result.errors_inline && !result.error_message.is_empty() && !text.contains(result.error_message.trim_end()) {
        text.push_str(&result.error_message);
    }
    text
}

fn kinds(result: &CommandResult, kind: MsgKind) -> Vec<Value> {
    result.message_list().into_iter().filter(|(k, _)| *k == kind)
        .map(|(_, t)| Value::String(t.trim_end().to_string())).collect()
}

// ---------------------------------------------------------------------------
// Restore sessions
// ---------------------------------------------------------------------------

enum Event {
    /// The dialog waits for input; `in_tree` while in file selection mode.
    Prompt { output: String, in_tree: bool },
    Finished { output: String, result: CommandResult },
}

/// Console of a restore session thread: output is collected until the
/// dialog asks for input; input comes from the RestoreTree* RPCs.
struct ChannelConsole {
    buffer: String,
    in_tree: bool,
    events: Sender<Event>,
    input: Receiver<Option<String>>,
}

impl Console for ChannelConsole {
    fn send(&mut self, text: &str) -> bool {
        self.buffer.push_str(text);
        true
    }
    fn signal(&mut self, signal: BnetSignal) -> bool {
        match signal {
            BnetSignal::StartRtree => self.in_tree = true,
            BnetSignal::EndRtree => self.in_tree = false,
            _ => {}
        }
        true
    }
    fn recv(&mut self) -> Option<String> {
        let mut output = std::mem::take(&mut self.buffer);
        // the file selection prompt is not part of a reply
        if self.in_tree && output.ends_with("$ ") {
            output.truncate(output.len() - 2);
        }
        if self.events.send(Event::Prompt { output, in_tree: self.in_tree }).is_err() {
            return None;
        }
        // Idle sessions end: no answer cancels the dialog.
        self.input.recv_timeout(RESTORE_IDLE).ok().flatten()
    }
}

pub struct RestoreSession {
    owner: u64,
    input: Sender<Option<String>>,
    events: Receiver<Event>,
    finished: Arc<AtomicBool>,
    last_used: Instant,
    jobids: Vec<u64>,
}

#[derive(Default)]
pub struct RestoreSessions {
    sessions: Mutex<HashMap<String, RestoreSession>>,
}

impl RestoreSessions {
    /// Drop ended and idle sessions.
    fn reap(&self) {
        self.sessions.lock().unwrap().retain(|_, s| {
            let keep = !s.finished.load(Ordering::Relaxed) && s.last_used.elapsed() < RESTORE_IDLE;
            if !keep {
                let _ = s.input.try_send(None);
            }
            keep
        });
    }

    /// End all restore sessions of a gRPC session (logout).
    pub fn close_owner(&self, owner: u64) {
        self.sessions.lock().unwrap().retain(|_, s| {
            if s.owner == owner {
                let _ = s.input.try_send(None);
                false
            } else {
                true
            }
        });
    }
}

async fn next_event(events: Receiver<Event>) -> Result<Event, Status> {
    tokio::task::spawn_blocking(move || events.recv_timeout(REPLY_WAIT))
        .await
        .map_err(|_| Status::internal("restore session failed"))?
        .map_err(|_| Status::deadline_exceeded("no reply from the restore dialog"))
}

fn quote(arg: &str) -> String {
    if arg.contains(char::is_whitespace) { format!("\"{arg}\"") } else { arg.to_string() }
}

/// RPC name -> tree command line (with the path/pattern argument).
fn tree_line(rpc: &str, path: &str) -> Result<String, Status> {
    let need = |cmd: &str| if path.is_empty() {
        Err(Status::invalid_argument(format!("{rpc}: path is required")))
    } else {
        Ok(format!("{cmd} {}", quote(path)))
    };
    let optional = |cmd: &str| if path.is_empty() { cmd.to_string() } else { format!("{cmd} {}", quote(path)) };
    if path.chars().any(|c| c.is_control() || c == '"') {
        return Err(Status::invalid_argument("path: control characters and '\"' are not allowed"));
    }
    Ok(match rpc {
        "RestoreTreeCd" => need("cd")?,
        "RestoreTreeLs" => optional("ls"),
        "RestoreTreeLsDir" => optional(".lsdir"),
        "RestoreTreeLsMark" => optional("lsmark"),
        "RestoreTreePwd" => "pwd".into(),
        "RestoreTreeMark" => need("mark")?,
        "RestoreTreeUnmark" => need("unmark")?,
        "RestoreTreeMarkDir" => need("markdir")?,
        "RestoreTreeUnmarkDir" => need("unmarkdir")?,
        "RestoreTreeCount" => "count".into(),
        "RestoreTreeFind" => need("find")?,
        "RestoreTreeEstimate" => "estimate".into(),
        "RestoreTreeHelp" => "help".into(),
        "RestoreTreeDone" => "done".into(),
        "RestoreTreeAbort" => "quit".into(),
        _ => return Err(Status::unimplemented(rpc.to_string())),
    })
}

impl DirectorApiImpl {
    fn exec_parts(&self) -> Result<&super::ExecParts, Status> {
        self.exec.as_ref().ok_or_else(|| Status::unavailable("no execution context"))
    }

    fn command_allowed(&self, session_id: u64, command: &str) -> Result<(), Status> {
        if self.auth.allowed(session_id, crate::acl::Kind::Command, command) {
            Ok(())
        } else {
            Err(Status::permission_denied(format!("command \"{command}\" not allowed")))
        }
    }

    /// Run a plan; returns (result, console transcript, cancelled question).
    async fn run_plan(&self, session_id: u64, plan: &Plan, own_thread: bool)
        -> Result<(CommandResult, String, Option<String>), Status> {
        self.command_allowed(session_id, &plan.ast.command_name)?;
        let mut ast = plan.ast.clone();
        ast.session_id = session_id.to_string();
        ast.source = CommandSource::Grpc;
        ast.response_format = if plan.mode == Mode::Json { ResponseFormat::Json } else { ResponseFormat::Text };
        let console = AnswerConsole::new(plan);
        let transcript = console.transcript.clone();
        let result = if own_thread {
            // blocking commands (wait) must not hold up the command worker
            let exec = self.exec_parts()?.clone();
            let mut console = console;
            tokio::task::spawn_blocking(move || {
                let ctx = exec.context(session_id, ApiMode::Human);
                crate::ua_select::with_console(&mut console, || command_executor::dispatch(&ast, &ctx))
            }).await.map_err(|_| Status::internal("command failed"))?
        } else if plan.mode == Mode::Json {
            run_command(&self.cmd_queue, ast, session_id).await?
        } else {
            let rx = self.cmd_queue.submit_with_console(ast, session_id, Some(Box::new(console)))
                .ok_or_else(|| Status::resource_exhausted("command queue full"))?;
            tokio::task::spawn_blocking(move || rx.recv())
                .await
                .map_err(|_| Status::internal("command execution failed"))?
                .map_err(|_| Status::internal("command execution failed"))?
        };
        let t = transcript.lock().unwrap();
        Ok((result, t.text.clone(), t.question.clone()))
    }

    /// One read command in JSON mode for the follow-up records.
    async fn query_json(&self, session_id: u64, line: &str) -> Option<Map<String, Value>> {
        let mut ast = crate::session::parse_cli_command(line);
        ast.session_id = session_id.to_string();
        ast.source = CommandSource::Grpc;
        ast.response_format = ResponseFormat::Json;
        let result = run_command(&self.cmd_queue, ast, session_id).await.ok()?;
        if !result.success {
            return None;
        }
        result.json.and_then(|v| v.as_object().cloned())
    }

    /// Typed records after the change.
    async fn follow_up(&self, session_id: u64, follow: &Follow, jobids: &[u64], response: &mut Map<String, Value>) {
        let mut jobs = Vec::new();
        let mut volumes = Vec::new();
        let mut pools = Vec::new();
        match follow {
            Follow::None => {}
            Follow::QueuedJobs | Follow::JobIds(_) => {
                for id in jobids {
                    if let Some(Value::Array(rows)) = self.query_json(session_id, &format!("llist jobid={id}")).await
                        .and_then(|mut o| o.remove("jobs")) {
                        jobs.extend(rows);
                    }
                }
            }
            Follow::Volumes(names) => {
                for name in names {
                    if let Some(v) = self.query_json(session_id, &format!("llist volume=\"{name}\"")).await
                        .and_then(|mut o| o.remove("volume")) {
                        volumes.push(v);
                    }
                }
            }
            Follow::Pools(names) => {
                for name in names {
                    if let Some(Value::Array(rows)) = self.query_json(session_id, &format!("llist pool=\"{name}\"")).await
                        .and_then(|mut o| o.remove("pools")) {
                        pools.extend(rows);
                    }
                }
            }
        }
        response.insert("jobs".into(), Value::Array(jobs));
        response.insert("volumes".into(), Value::Array(volumes));
        response.insert("pools".into(), Value::Array(pools));
    }

    /// ActionResponse of a finished command.
    async fn action_response(&self, session_id: u64, plan: &Plan, result: CommandResult, console_text: String,
                             question: Option<String>) -> Map<String, Value> {
        let text = reply_text(&console_text, &result);
        let errors = kinds(&result, MsgKind::Error);
        let success = result.success && errors.is_empty() && question.is_none();
        let jobids: Vec<u64> = match &plan.follow {
            Follow::QueuedJobs => api_write::queued_jobids(&text),
            Follow::JobIds(ids) => ids.clone(),
            _ => Vec::new(),
        };
        let mut response = Map::new();
        response.insert("success".into(), json!(success));
        response.insert("text".into(), json!(text));
        response.insert("errors".into(), Value::Array(errors));
        response.insert("warnings".into(), Value::Array(kinds(&result, MsgKind::Warning)));
        response.insert("jobids".into(), json!(jobids));
        if let Some(Value::Object(obj)) = &result.json {
            response.insert("result".into(), Value::Object(obj.clone()));
        }
        if let Some(q) = question {
            response.insert("question".into(), json!(q));
        }
        response.insert("command".into(), json!(plan.line()));
        if success {
            self.follow_up(session_id, &plan.follow, &jobids, &mut response).await;
        }
        response
    }

    /// Write and session RPCs returning ActionResponse.
    pub(super) async fn write<Q: Message>(&self, request: Request<Q>, rpc: &str) -> Result<Response<pb::ActionResponse>, Status> {
        let session_id = self.auth.check_session(request.metadata())?;
        if let Some((_, why)) = NOT_APPLICABLE.iter().find(|(name, _)| *name == rpc) {
            return Err(Status::failed_precondition(format!("{rpc} does not apply to the typed API: {why}")));
        }
        if LOGOUT.contains(&rpc) {
            let token = token(request.metadata());
            self.restore_sessions.close_owner(session_id);
            self.auth.logout(&token);
            return encode(rpc, serde_json::from_value(json!({"success": true, "text": "Session closed.\n"})).unwrap())
                .map(Response::new);
        }
        let fields = decode(rpc, request.get_ref())?;
        let plan = api_write::plan(rpc, &fields)
            .ok_or_else(|| Status::unimplemented(rpc.to_string()))?
            .map_err(Status::invalid_argument)?;
        let (result, console_text, question) = self.run_plan(session_id, &plan, rpc == "Wait").await?;
        let response = self.action_response(session_id, &plan, result, console_text, question).await;
        encode(rpc, response).map(Response::new)
    }

    /// DotSql (JSON rows), SqlQuery and Query (text).
    pub(super) async fn sql<Q: Message>(&self, request: Request<Q>, rpc: &str) -> Result<Response<pb::SqlQueryResponse>, Status> {
        let session_id = self.auth.check_session(request.metadata())?;
        let fields = decode(rpc, request.get_ref())?;
        let plan = api_write::plan(rpc, &fields)
            .ok_or_else(|| Status::unimplemented(rpc.to_string()))?
            .map_err(Status::invalid_argument)?;
        let (result, console_text, question) = self.run_plan(session_id, &plan, false).await?;
        let errors: Vec<String> = kinds(&result, MsgKind::Error).iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
        if !result.success || !errors.is_empty() {
            let text = if errors.is_empty() { reply_text(&console_text, &result) } else { errors.join("\n") };
            return Err(Status::failed_precondition(text.trim_end().to_string()));
        }
        let mut object = match (&plan.mode, &result.json) {
            (Mode::Json, Some(Value::Object(o))) => o.clone(),
            _ => Map::new(),
        };
        if plan.mode == Mode::Text {
            let mut text = reply_text(&console_text, &result);
            if let Some(q) = question {
                text.push_str(&format!("\n[cancelled question: {q}]\n"));
            }
            object.insert("text".into(), json!(text));
        }
        encode(rpc, object).map(Response::new)
    }

    // --- restore sessions --------------------------------------------------

    pub(super) async fn start_restore_session(&self, request: Request<pb::RestoreSessionCommand>)
        -> Result<Response<pb::RestoreSessionResponse>, Status> {
        let rpc = "StartRestoreSession";
        let session_id = self.auth.check_session(request.metadata())?;
        self.restore_sessions.reap();
        let fields = decode(rpc, request.get_ref())?;
        let plan = api_write::restore_session_plan(&fields).map_err(Status::invalid_argument)?;
        self.command_allowed(session_id, "restore")?;
        let exec = self.exec_parts()?.clone();
        let mut ast = plan.ast.clone();
        ast.session_id = session_id.to_string();
        ast.source = CommandSource::Grpc;
        ast.response_format = ResponseFormat::Text;
        let (event_tx, event_rx) = bounded::<Event>(4);
        let (input_tx, input_rx) = bounded::<Option<String>>(4);
        let finished = Arc::new(AtomicBool::new(false));
        let done = finished.clone();
        std::thread::Builder::new().name("grpc:restore".into()).spawn(move || {
            let ctx = exec.context(session_id, ApiMode::Human);
            let mut console = ChannelConsole { buffer: String::new(), in_tree: false, events: event_tx.clone(), input: input_rx };
            let result = crate::ua_select::with_console(&mut console, || command_executor::dispatch(&ast, &ctx));
            let output = std::mem::take(&mut console.buffer);
            done.store(true, Ordering::Relaxed);
            let _ = event_tx.send(Event::Finished { output, result });
        }).map_err(|e| Status::internal(format!("restore session: {e}")))?;

        match next_event(event_rx.clone()).await? {
            Event::Prompt { output, in_tree: true } => {
                let mut id = [0u8; 16];
                rand::thread_rng().fill_bytes(&mut id);
                let id = hex::encode(id);
                let jobids: Vec<u64> = fields.get("jobid").and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
                let files = output.lines().find(|l| l.contains("files inserted into the tree"))
                    .and_then(|l| api_write::numbers(l).first().copied()).unwrap_or(0);
                let cwd = output.lines().rev().find_map(|l| l.strip_prefix("cwd is: ")).unwrap_or("/").to_string();
                self.restore_sessions.sessions.lock().unwrap().insert(id.clone(), RestoreSession {
                    owner: session_id, input: input_tx, events: event_rx, finished, last_used: Instant::now(),
                    jobids: jobids.clone(),
                });
                encode(rpc, serde_json::from_value(json!({
                    "session_id": id, "jobids": jobids, "files": files, "cwd": cwd, "text": output,
                })).unwrap()).map(Response::new)
            }
            Event::Prompt { output, in_tree: false } => {
                let _ = input_tx.send(None);
                let rest = match next_event(event_rx).await {
                    Ok(Event::Finished { output, result }) => reply_text(&output, &result),
                    _ => String::new(),
                };
                Err(Status::failed_precondition(format!(
                    "the restore asked \"{}\" before the file selection; give the missing parameter (jobid, client, fileset)\n{output}{rest}",
                    api_write::question(&output))))
            }
            Event::Finished { output, result } => {
                let text = reply_text(&output, &result);
                Err(Status::failed_precondition(text.trim_end().to_string()))
            }
        }
    }

    /// Send one tree command; returns the reply text and whether the session
    /// is still in file selection mode (else the final event).
    async fn tree_command(&self, request_session: u64, id: &str, line: &str)
        -> Result<(String, Option<(String, CommandResult)>, Vec<u64>), Status> {
        let (input, events, jobids) = {
            let mut sessions = self.restore_sessions.sessions.lock().unwrap();
            let s = sessions.get_mut(id).filter(|s| s.owner == request_session && !s.finished.load(Ordering::Relaxed))
                .ok_or_else(|| Status::not_found("no such restore session (ended, timed out or of another login)"))?;
            s.last_used = Instant::now();
            (s.input.clone(), s.events.clone(), s.jobids.clone())
        };
        input.send(Some(line.to_string())).map_err(|_| Status::not_found("restore session ended"))?;
        match next_event(events).await? {
            Event::Prompt { output, in_tree: true } => Ok((output, None, jobids)),
            Event::Prompt { output, in_tree: false } => Ok((output.clone(), Some((output, CommandResult::ok(String::new()))), jobids)),
            Event::Finished { output, result } => {
                self.restore_sessions.sessions.lock().unwrap().remove(id);
                Ok((output.clone(), Some((output, result)), jobids))
            }
        }
    }

    fn session_request(fields: &Map<String, Value>) -> Result<(String, String), Status> {
        let id = fields.get("session_id").and_then(Value::as_str).unwrap_or("").to_string();
        if id.is_empty() {
            return Err(Status::invalid_argument("session_id is required"));
        }
        let path = fields.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        Ok((id, path))
    }

    /// cd, ls, .lsdir, lsmark, pwd, find
    pub(super) async fn tree_list<Q: Message>(&self, request: Request<Q>, rpc: &str)
        -> Result<Response<pb::RestoreTreeListResponse>, Status> {
        let session_id = self.auth.check_session(request.metadata())?;
        let (id, path) = Self::session_request(&decode(rpc, request.get_ref())?)?;
        let (text, ended, _) = self.tree_command(session_id, &id, &tree_line(rpc, &path)?).await?;
        if ended.is_some() {
            return Err(Status::failed_precondition(format!("restore session ended\n{text}")));
        }
        let cwd = if matches!(rpc, "RestoreTreeCd" | "RestoreTreePwd") {
            api_write::cwd_of(&text).unwrap_or_default()
        } else {
            let (pwd, _, _) = self.tree_command(session_id, &id, ".pwd").await?;
            api_write::cwd_of(&pwd).unwrap_or_default()
        };
        let entries: Vec<Value> = if matches!(rpc, "RestoreTreeCd" | "RestoreTreePwd") {
            Vec::new()
        } else {
            api_write::tree_entries(&text).into_iter()
                .map(|(name, is_dir, marked)| json!({"name": name, "is_dir": is_dir, "marked": marked})).collect()
        };
        encode(rpc, serde_json::from_value(json!({"cwd": cwd, "entries": entries, "text": text})).unwrap())
            .map(Response::new)
    }

    /// mark, unmark, markdir, unmarkdir, count, estimate
    pub(super) async fn tree_count<Q: Message>(&self, request: Request<Q>, rpc: &str)
        -> Result<Response<pb::RestoreTreeCountResponse>, Status> {
        let session_id = self.auth.check_session(request.metadata())?;
        let (id, path) = Self::session_request(&decode(rpc, request.get_ref())?)?;
        let (text, ended, _) = self.tree_command(session_id, &id, &tree_line(rpc, &path)?).await?;
        if ended.is_some() {
            return Err(Status::failed_precondition(format!("restore session ended\n{text}")));
        }
        let n = api_write::numbers(&text);
        let mut object = json!({"text": text}).as_object().cloned().unwrap();
        match rpc {
            "RestoreTreeCount" | "RestoreTreeEstimate" => {
                object.insert("total".into(), json!(n.first().copied().unwrap_or(0)));
                object.insert("marked".into(), json!(n.get(1).copied().unwrap_or(0)));
                object.insert("bytes".into(), json!(n.get(2).copied().unwrap_or(0)));
            }
            _ => {
                object.insert("marked".into(), json!(n.first().copied().unwrap_or(0)));
            }
        }
        encode(rpc, object).map(Response::new)
    }

    pub(super) async fn tree_help(&self, request: Request<pb::RestoreTreeSessionCommand>)
        -> Result<Response<pb::HelpResponse>, Status> {
        let rpc = "RestoreTreeHelp";
        let session_id = self.auth.check_session(request.metadata())?;
        let (id, _) = Self::session_request(&decode(rpc, request.get_ref())?)?;
        let (text, _, _) = self.tree_command(session_id, &id, "help").await?;
        encode(rpc, serde_json::from_value(json!({"text": text})).unwrap()).map(Response::new)
    }

    /// done (answers "OK to run?" with yes) and abort
    pub(super) async fn tree_finish(&self, request: Request<pb::RestoreTreeSessionCommand>, rpc: &str)
        -> Result<Response<pb::ActionResponse>, Status> {
        let session_id = self.auth.check_session(request.metadata())?;
        let (id, _) = Self::session_request(&decode(rpc, request.get_ref())?)?;
        let line = tree_line(rpc, "")?;
        let (mut text, mut ended, _) = self.tree_command(session_id, &id, &line).await?;
        let mut question = None;
        // questions after the selection: only "OK to run?" is answered
        while let Some((output, _)) = &ended {
            let finished = self.restore_sessions.sessions.lock().unwrap().get(&id).is_none();
            if finished {
                break;
            }
            let q = api_write::question(output);
            let answer = if rpc == "RestoreTreeDone" && q.contains("OK to run?") { Some("yes") } else { None };
            if answer.is_none() {
                question = Some(q);
            }
            let (input, events) = {
                let sessions = self.restore_sessions.sessions.lock().unwrap();
                let s = sessions.get(&id).ok_or_else(|| Status::not_found("restore session ended"))?;
                (s.input.clone(), s.events.clone())
            };
            let _ = input.send(answer.map(str::to_string));
            match next_event(events).await? {
                Event::Prompt { output, .. } => {
                    text.push_str(answer.unwrap_or(""));
                    text.push('\n');
                    text.push_str(&output);
                    ended = Some((output, CommandResult::ok(String::new())));
                }
                Event::Finished { output, result } => {
                    self.restore_sessions.sessions.lock().unwrap().remove(&id);
                    text.push_str(answer.unwrap_or(""));
                    text.push('\n');
                    ended = Some((output, result));
                    break;
                }
            }
        }
        let result = match ended {
            Some((output, result)) => {
                let t = reply_text(&output, &result);
                if !text.ends_with(&t) {
                    text.push_str(&t);
                }
                result
            }
            None => return Err(Status::failed_precondition(format!("{rpc}: the restore dialog did not end\n{text}"))),
        };
        let plan = Plan {
            ast: crate::command_ast::CommandAst::default(),
            yes_to: &[],
            answers: Vec::new(),
            follow: if rpc == "RestoreTreeDone" { Follow::QueuedJobs } else { Follow::None },
            mode: Mode::Text,
        };
        let mut response = self.action_response(session_id, &plan, result, text, question.clone()).await;
        response.insert("command".into(), json!(line));
        if rpc == "RestoreTreeAbort" && question.is_none() {
            response.insert("success".into(), json!(true));
        }
        encode(rpc, response).map(Response::new)
    }
}
