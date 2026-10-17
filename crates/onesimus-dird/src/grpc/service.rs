//! DirectorService implementation for tonic gRPC.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use tracing::{info, warn};

use onesimus_proto_rs::onesimus::director::{
    director_server::Director,
    command_argument, session_message,
    ChallengeRequest, ChallengeResponse, CommandInfo, CommandRequest, CommandResponse,
    ListCommandsRequest, ListCommandsResponse, LoginRequest, LoginResponse,
    ResponseFormat as ProtoFormat, ServerInfo, ServerInfoRequest, SessionMessage,
};

use super::auth::AuthState;
use crate::command_ast::{Argument, CommandAst, CommandSource, ResponseFormat};
use crate::command_queue::{CommandQueue, CommandResult};
use crate::message_bus::MessageBus;
use crate::session::parse_cli_command;

pub struct DirectorServiceImpl {
    bus: Arc<MessageBus>,
    cmd_queue: Arc<CommandQueue>,
    auth: Arc<AuthState>,
    director_name: String,
    bnet_port: u16,
}

impl DirectorServiceImpl {
    pub fn new(
        bus: Arc<MessageBus>,
        cmd_queue: Arc<CommandQueue>,
        auth: Arc<AuthState>,
        director_name: String,
        bnet_port: u16,
    ) -> Self {
        Self { bus, cmd_queue, auth, director_name, bnet_port }
    }
}

/// Convert a CommandRequest into a CommandAst.
/// Clients may send the whole command line in `command_name` (as typed in a
/// console); it is then parsed exactly like a BNET console line. A single
/// word with `=` (`.jobstatus=T`, `status=…`) is a console line, too.
fn command_request_to_ast(req: &CommandRequest, session_id: &str) -> CommandAst {
    let line = req.command_name.trim();
    let mut ast = if req.arguments.is_empty() && (line.contains(char::is_whitespace) || line.contains('=')) {
        parse_cli_command(line)
    } else {
        CommandAst {
            command_name: req.command_name.clone(),
            arguments: req.arguments.iter().map(|a| {
                let value = match &a.value {
                    Some(command_argument::Value::StringValue(s)) => Some(s.clone()),
                    Some(command_argument::Value::JsonValue(b)) => Some(String::from_utf8_lossy(b).into_owned()),
                    None => None,
                };
                Argument { key: a.key.clone(), value }
            }).collect(),
            ..Default::default()
        }
    };
    ast.session_id = session_id.to_string();
    ast.source = CommandSource::Grpc;
    ast.response_format = match req.response_format() {
        ProtoFormat::Text => ResponseFormat::Text,
        ProtoFormat::Json => ResponseFormat::Json,
        ProtoFormat::Api => ResponseFormat::Api,
        ProtoFormat::Grpc => ResponseFormat::Grpc,
        ProtoFormat::Default => ResponseFormat::Default,
    };
    ast
}

fn to_response(result: CommandResult) -> CommandResponse {
    CommandResponse {
        success: result.success,
        text_output: result.text_output,
        json_output: Vec::new(),
        error_message: result.error_message,
    }
}

/// Run a command through the shared queue without blocking the tokio runtime.
pub(super) async fn run_command(cmd_queue: &Arc<CommandQueue>, ast: CommandAst, session_id: u64) -> Result<CommandResult, Status> {
    let rx = cmd_queue.submit(ast, session_id)
        .ok_or_else(|| Status::resource_exhausted("command queue full"))?;
    tokio::task::spawn_blocking(move || rx.recv())
        .await
        .map_err(|_| Status::internal("command execution failed"))?
        .map_err(|_| Status::internal("command execution failed"))
}

#[tonic::async_trait]
impl Director for DirectorServiceImpl {
    async fn get_server_info(
        &self,
        _request: Request<ServerInfoRequest>,
    ) -> Result<Response<ServerInfo>, Status> {
        Ok(Response::new(ServerInfo {
            director_name: self.director_name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            product: "onesimus-dird".to_string(),
            bnet_port: self.bnet_port as i32,
        }))
    }

    async fn get_challenge(
        &self,
        request: Request<ChallengeRequest>,
    ) -> Result<Response<ChallengeResponse>, Status> {
        let nonce = self.auth.challenge(&request.into_inner().console_name);
        Ok(Response::new(ChallengeResponse { nonce }))
    }

    async fn login(
        &self,
        request: Request<LoginRequest>,
    ) -> Result<Response<LoginResponse>, Status> {
        let peer = request.remote_addr();
        let req = request.into_inner();
        let response = match self.auth.login(&req.console_name, &req.nonce, &req.proof) {
            Ok(token) => {
                info!("gRPC login: console '{}' from {:?}", req.console_name, peer);
                LoginResponse {
                    success: true,
                    session_token: token,
                    director_name: self.director_name.clone(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    ..Default::default()
                }
            }
            Err(e) => {
                warn!("gRPC login failed: console '{}' from {:?}: {e}", req.console_name, peer);
                LoginResponse { success: false, error_message: e, ..Default::default() }
            }
        };
        Ok(Response::new(response))
    }

    async fn execute_command(
        &self,
        request: Request<CommandRequest>,
    ) -> Result<Response<CommandResponse>, Status> {
        let session_id=self.auth.check_session(request.metadata())?;
        let req = request.into_inner();
        let ast = command_request_to_ast(&req, &session_id.to_string());
        let result = run_command(&self.cmd_queue, ast, session_id).await?;
        Ok(Response::new(to_response(result)))
    }

    async fn list_commands(
        &self,
        request: Request<ListCommandsRequest>,
    ) -> Result<Response<ListCommandsResponse>, Status> {
        let session_id=self.auth.check_session(request.metadata())?;
        let commands = KNOWN_COMMANDS.iter().filter(|(name,_,_)|self.auth.allowed(session_id,crate::acl::Kind::Command,name)).map(|(name, help, usage)| {
            CommandInfo {
                name: name.to_string(),
                help: help.to_string(),
                usage: usage.to_string(),
                allowed_in_runscript: false,
                audit_event: false,
            }
        }).collect();

        Ok(Response::new(ListCommandsResponse { commands }))
    }

    type InteractiveSessionStream = ReceiverStream<Result<SessionMessage, Status>>;

    async fn interactive_session(
        &self,
        request: Request<Streaming<SessionMessage>>,
    ) -> Result<Response<Self::InteractiveSessionStream>, Status> {
        let session_id=self.auth.check_session(request.metadata())?;
        let metadata=request.metadata().clone();
        let auth=self.auth.clone();
        let mut stream = request.into_inner();
        let (tx, rx) = mpsc::channel(32);

        let bus = self.bus.clone();
        let cmd_queue = self.cmd_queue.clone();

        tokio::spawn(async move {
            // Subscribe to bus for push events
            let bus_rx = bus.subscribe();

            loop {
                tokio::select! {
                    // Read from client
                    msg = stream.message() => {
                        match msg {
                            Ok(Some(session_msg)) => {
                                if session_msg.r#type() != session_message::Type::Command {
                                    continue;
                                }
                                if auth.check_session(&metadata).is_err() {
                                    let _=tx.send(Err(Status::unauthenticated("session expired"))).await;
                                    break;
                                }
                                let Some(cmd_req) = session_msg.command else { continue };
                                let ast = command_request_to_ast(&cmd_req, &session_id.to_string());
                                let response = match run_command(&cmd_queue, ast, session_id).await {
                                    Ok(result) => to_response(result),
                                    Err(status) => CommandResponse {
                                        success: false,
                                        error_message: status.message().to_string(),
                                        ..Default::default()
                                    },
                                };
                                let reply = SessionMessage {
                                    r#type: session_message::Type::Response as i32,
                                    session_id: session_msg.session_id,
                                    response: Some(response),
                                    ..Default::default()
                                };
                                if tx.send(Ok(reply)).await.is_err() {
                                    break;
                                }
                            }
                            Ok(None) => break, // Stream ended
                            Err(_) => break,    // Error
                        }
                    }

                    // Check for bus messages (non-blocking)
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        while let Ok(bus_msg) = bus_rx.try_recv() {
                            let push_event = super::push_events::to_push_event(&bus_msg);
                            let session_msg = SessionMessage {
                                r#type: session_message::Type::PushEvent as i32,
                                push_event: Some(push_event),
                                ..Default::default()
                            };
                            if tx.send(Ok(session_msg)).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Well-known Director commands (subset for ListCommands).
static KNOWN_COMMANDS: &[(&str, &str, &str)] = &[
    ("status", "Report status", "status [dir | client | storage | all]"),
    ("version", "Print Director version", "version"),
    ("help", "Print help on command", "help [command]"),
    (".help", "Print machine-readable help", ".help [command]"),
    ("run", "Run a job", "run [job=name] [client=name] [level=type]"),
    ("cancel", "Cancel a job", "cancel [jobid=id]"),
    ("list", "List catalog contents", "list [jobs | volumes | pools | clients]"),
    ("llist", "Long list catalog contents", "llist [jobs | volumes]"),
    ("restore", "Restore files", "restore [where=path]"),
    ("quit", "Terminate bconsole", "quit"),
    ("reload", "Reload configuration", "reload"),
    ("enable", "Enable resource", "enable [client | job | schedule | storage]"),
    ("disable", "Disable resource", "disable [client | job | schedule | storage]"),
    ("label", "Label a volume", "label [volume=name] [storage=name]"),
    ("mount", "Mount storage device", "mount [storage=name]"),
    ("unmount", "Unmount storage device", "unmount [storage=name]"),
    ("update", "Update catalog records", "update [pool | volume | slots]"),
    ("configure", "Configure resources", "configure [add | delete | update]"),
    (".clients", "List clients", ".clients"),
    (".jobs", "List jobs", ".jobs"),
    (".pools", "List pools", ".pools"),
    (".storage", "List storage", ".storage"),
    (".filesets", "List filesets", ".filesets"),
    (".defaults", "List defaults", ".defaults [job=name]"),
    (".jobstatus", "List job status codes", ".jobstatus"),
    (".sql", "Execute SQL", ".sql query=\"...\""),
    ("time", "Print current time", "time"),
    ("estimate", "Estimate backup size", "estimate [job=name] [listing]"),
    ("prune", "Prune catalog records", "prune [files | jobs | volumes]"),
    ("purge", "Purge catalog records", "purge [files | jobs | volumes]"),
    ("truncate", "Truncate purged volumes", "truncate [volume=name]"),
];

#[cfg(test)]
mod acl_binding_tests {
    use super::*;
    #[tokio::test]
    async fn client_supplied_session_id_cannot_impersonate_default_console() {
        let ctx=crate::acl::tests::context();
        let manager=ctx.session_mgr.as_ref().unwrap().clone();
        let root=manager.create_session("*UserAgent*");manager.authenticate_session(root);
        let auth=Arc::new(super::super::auth::AuthState::new(ctx.config.clone().unwrap(),manager));
        let nonce=auth.challenge("limited");
        let token=auth.login("limited",&nonce,&super::super::auth::compute_proof(&super::super::auth::password_key("pw"),&nonce)).unwrap();
        let queue=Arc::new(CommandQueue::new(8));
        let service=DirectorServiceImpl::new(Arc::new(MessageBus::new(16)),queue.clone(),auth,"dir".into(),9101);
        let mut request=Request::new(CommandRequest{command_name:"delete".into(),session_id:root.to_string(),..Default::default()});
        request.metadata_mut().insert(super::super::auth::SESSION_HEADER,token.parse().unwrap());
        let task=tokio::spawn(async move {service.execute_command(request).await.unwrap().into_inner()});
        let queued=tokio::task::spawn_blocking(move || queue.take().unwrap()).await.unwrap();
        assert_ne!(queued.session_id,root);assert_eq!(queued.ast.session_id,queued.session_id.to_string());
        let mut execute_ctx=ctx;execute_ctx.session_id=queued.session_id;
        let result=crate::command_executor::dispatch(&queued.ast,&execute_ctx);assert!(!result.success);
        queued.result_tx.send(result).unwrap();assert!(!task.await.unwrap().success);
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;

    #[test]
    fn single_word_with_value_is_parsed_like_a_console_line() {
        let req = CommandRequest { command_name: ".jobstatus=T".into(), ..Default::default() };
        let bnet = parse_cli_command(".jobstatus=T");
        let ast = command_request_to_ast(&req, "s");
        assert_eq!(ast.command_name, bnet.command_name);
        assert_eq!(ast.arguments.len(), bnet.arguments.len());
        let req = CommandRequest { command_name: "version".into(), ..Default::default() };
        assert_eq!(command_request_to_ast(&req, "s").command_name, "version");
    }
}
