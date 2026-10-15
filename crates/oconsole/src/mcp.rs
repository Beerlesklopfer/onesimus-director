//! MCP server mode (`oconsole --mcp`).
//!
//! Speaks the Model Context Protocol (JSON-RPC 2.0, one message per line) on
//! stdin/stdout so AI assistants can query the Director. The Director
//! connection (BNET or gRPC) is opened before serving, with the same config
//! and login as the interactive console. stdout carries protocol only; all
//! diagnostics go to stderr.
//!
//! Tools:
//!   query   — read-only commands (list, show, status, dot-commands, …)
//!   execute — any command; only with --mcp-allow-write

use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};

use serde_json::{Value, json};

/// Runs one command line against the Director and returns its text output.
pub trait CommandRunner: Send + 'static {
    fn run(&mut self, command: &str) -> anyhow::Result<String>;
}

pub struct McpServer<R: CommandRunner> {
    runner: Arc<Mutex<R>>,
    #[cfg(feature = "mcp-development")]
    allow_write: bool,
    director_name: String,
    mirror: crate::mcp_mirror::Mirror,
}

impl<R: CommandRunner> McpServer<R> {
    pub fn new(runner: R, _allow_write: bool, director_name: String) -> Self {
        #[cfg(feature = "mcp-development")]
        let allow_write = _allow_write;
        Self {
            runner: Arc::new(Mutex::new(runner)),
            #[cfg(feature = "mcp-development")]
            allow_write,
            director_name,
            mirror: crate::mcp_mirror::Mirror::stderr(),
        }
    }

    fn write_enabled(&self) -> bool {
        #[cfg(feature = "mcp-development")]
        {
            self.allow_write
        }
        #[cfg(feature = "mcp-production")]
        {
            false
        }
    }

    pub fn serve_transport(
        self,
        address: Option<std::net::SocketAddr>,
        token_path: Option<std::path::PathBuf>,
        hosts: Vec<String>,
    ) -> anyhow::Result<()> {
        // Read/generate the credential before entering the async runtime.
        let http = address
            .map(|address| {
                let path = token_path
                    .map(Ok)
                    .unwrap_or_else(crate::mcp_token::default_path)?;
                let token = crate::mcp_token::load_or_create(&path)?;
                eprintln!(
                    "oconsole: MCP URL http://{address}/mcp; bearer token file {}",
                    path.display()
                );
                Ok::<_, anyhow::Error>((address, token))
            })
            .transpose()?;
        // Keep the runner alive outside the runtime (GrpcRunner owns a runtime).
        let runner = self.runner.clone();
        let result = tokio::runtime::Runtime::new()?.block_on(async move {
            if let Some((address, token)) = http {
                let listener = tokio::net::TcpListener::bind(address).await?;
                axum::serve(listener, self.http_router(token, hosts))
                    .with_graceful_shutdown(async {
                        loop {
                            if crate::CTRL_C.load(std::sync::atomic::Ordering::Relaxed) {
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    })
                    .await?;
            } else {
                let (read, write) = rmcp::transport::stdio();
                let access = self.mirror.access("stdio");
                let transport = (
                    crate::mcp_mirror::MirrorIo::new(read, access.clone(), "request"),
                    crate::mcp_mirror::MirrorIo::new(write, access, "response"),
                );
                let service = ServiceExt::serve(self, transport).await?;
                service.waiting().await?;
            }
            Ok(())
        });
        drop(runner);
        result
    }

    fn http_router(self, token: String, hosts: Vec<String>) -> axum::Router {
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        };
        let mut config = StreamableHttpServerConfig::default();
        config.allowed_hosts.extend(hosts);
        config = config.enforce_origin_validation();
        let mirror = self.mirror.clone();
        let service = StreamableHttpService::new(
            move || {
                Ok(Self {
                    runner: self.runner.clone(),
                    #[cfg(feature = "mcp-development")]
                    allow_write: self.allow_write,
                    director_name: self.director_name.clone(),
                    mirror: self.mirror.clone(),
                })
            },
            Arc::new(LocalSessionManager::default()),
            config,
        );
        axum::Router::new()
            .route_service("/mcp", service)
            .layer(axum::middleware::from_fn(
                move |request: axum::extract::Request, next: axum::middleware::Next| {
                    let token = token.clone();
                    let access = mirror.access("http");
                    async move {
                        access.event(
                            "request",
                            format!("{} {}", request.method(), request.uri().path()).as_bytes(),
                        );
                        let (parts, body) = request.into_parts();
                        let request = axum::extract::Request::from_parts(
                            parts,
                            access.body(body, "request-body"),
                        );
                        let header = request
                            .headers()
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|h| h.to_str().ok());
                        let response = if !crate::mcp_token::authorized(header, &token) {
                            use axum::response::IntoResponse;
                            (
                                axum::http::StatusCode::UNAUTHORIZED,
                                [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
                                "MCP bearer token required",
                            )
                                .into_response()
                        } else {
                            next.run(request).await
                        };
                        access.event("response", format!("HTTP {}", response.status()).as_bytes());
                        let (parts, body) = response.into_parts();
                        axum::response::Response::from_parts(
                            parts,
                            access.body(body, "response-body"),
                        )
                    }
                },
            ))
    }

    fn server_info(&self) -> ServerConfig {
        let mode = if self.write_enabled() {
            "read and write"
        } else {
            "read-only"
        };
        serde_json::from_value(json!({
            "protocolVersion": ProtocolVersion::default(),
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "oconsole", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!(
                "Connected to the Onesimus/Bareos Director '{}' ({mode}). \
                 Commands are bconsole command lines, e.g. 'status dir', 'list jobs', \
                 'list volumes', 'show client=NAME', 'llist jobid=42'. \
                 Use 'help' or '.help all' to discover commands.",
                self.director_name
            ),
        }))
        .expect("valid server metadata")
    }

    fn tools_list(&self) -> Value {
        let tools = vec![json!({
            "name": "query",
            "title": "Query the Director (read-only)",
            "description": "Run a read-only bconsole command on the Director and return its output. \
                Allowed: status, list, llist, show, version, help, time, messages, estimate, \
                and dot-commands such as .jobs, .clients, .pools, .storages, .filesets, .help. \
                Examples: 'status dir', 'list jobs', 'llist jobid=42', 'show fileset=Linux'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "bconsole command line" }
                },
                "required": ["command"]
            },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        })];
        #[cfg(feature = "mcp-development")]
        let mut tools = tools;
        #[cfg(feature = "mcp-development")]
        if self.write_enabled() {
            tools.push(json!({
                "name": "execute",
                "title": "Execute a Director command",
                "description": "Run any bconsole command on the Director, including ones that change \
                    state: run, cancel, label, mount, update, prune, purge, delete, configure. \
                    Prefer 'query' for read-only commands. Confirm destructive actions with the user first.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "bconsole command line" }
                    },
                    "required": ["command"]
                },
                "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false }
            }));
        }
        json!({ "tools": tools })
    }

    fn tools_call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let command = params
            .get("arguments")
            .and_then(|a| a.get("command"))
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");

        match name {
            "query" => {}
            #[cfg(feature = "mcp-development")]
            "execute" if self.write_enabled() => {}
            "execute" => {
                return Err((
                    -32602,
                    "tool 'execute' is unavailable in this build or disabled".into(),
                ));
            }
            _ => return Err((-32602, format!("unknown tool: {name}"))),
        }

        // Tool-level problems are reported as tool results so the model can react
        if let Err(msg) = validate_command(command) {
            return Ok(tool_result(&msg, true));
        }
        if name == "query" && !is_read_only(command) {
            return Ok(tool_result(
                &format!(
                    "'{command}' is not a read-only command. {}",
                    if self.write_enabled() {
                        "Use the 'execute' tool."
                    } else {
                        "Write access is unavailable in this build or disabled."
                    }
                ),
                true,
            ));
        }

        match self
            .runner
            .lock()
            .map_err(|_| (-32603, "Director runner unavailable".into()))?
            .run(command)
        {
            Ok(out) if out.trim().is_empty() => Ok(tool_result("(no output)", false)),
            Ok(out) => Ok(tool_result(&out, false)),
            Err(e) => Ok(tool_result(&format!("Director error: {e}"), true)),
        }
    }
}

fn tool_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

impl<R: CommandRunner> ServerHandler for McpServer<R> {
    fn get_info(&self) -> ServerConfig {
        self.server_info()
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(serde_json::from_value(self.tools_list()).expect("valid tool metadata"))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let server = Self {
            runner: self.runner.clone(),
            #[cfg(feature = "mcp-development")]
            allow_write: self.allow_write,
            director_name: self.director_name.clone(),
            mirror: self.mirror.clone(),
        };
        tokio::task::spawn_blocking(move || {
            let params = serde_json::to_value(request).expect("serializable tool request");
            let result = server
                .tools_call(&params)
                .map_err(|(_, message)| ErrorData::invalid_params(message, None))?;
            let result: CallToolResult = serde_json::from_value(result).expect("valid tool result");
            Ok(result.into())
        })
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
    }
}

/// One single command line, no control characters (BNET would take them literally).
fn validate_command(command: &str) -> Result<(), String> {
    if command.is_empty() {
        return Err("empty command".into());
    }
    if command.chars().any(|c| c.is_control()) {
        return Err("command must be a single line without control characters".into());
    }
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(first.as_str(), "quit" | "exit" | ".quit" | ".api" | "@exit")
        || first.starts_with('@')
    {
        return Err(format!("'{first}' is not available via MCP"));
    }
    Ok(())
}

/// Commands that only read Director/catalog state.
fn is_read_only(command: &str) -> bool {
    let first = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if first.starts_with('.') {
        // Full names only: a prefix could resolve to a mutating command.
        return matches!(
            first.as_str(),
            "." | ".actiononpurge"
                | ".authorized"
                | ".catalogs"
                | ".clients"
                | ".consoles"
                | ".defaults"
                | ".filesets"
                | ".help"
                | ".jobdefs"
                | ".jobs"
                | ".jobstatus"
                | ".levels"
                | ".locations"
                | ".messages"
                | ".media"
                | ".mediatypes"
                | ".msgs"
                | ".pools"
                | ".profiles"
                | ".schedule"
                | ".status"
                | ".storage"
                | ".storages"
                | ".types"
                | ".users"
                | ".volstatus"
                | ".bvfs_get_jobids"
                | ".bvfs_lsdirs"
                | ".bvfs_lsfiles"
                | ".bvfs_versions"
                | ".sqltables"
                | ".sqlschema"
                | ".sqlcount"
        );
    }
    matches!(
        first.as_str(),
        "status"
            | "list"
            | "llist"
            | "show"
            | "version"
            | "help"
            | "time"
            | "messages"
            | "estimate"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo(Vec<String>);
    impl CommandRunner for Echo {
        fn run(&mut self, command: &str) -> anyhow::Result<String> {
            self.0.push(command.to_string());
            if command == "status fail" {
                anyhow::bail!("test Director unavailable");
            }
            Ok(format!("ran {command}\n"))
        }
    }

    fn call(srv: &mut McpServer<Echo>, line: Value) -> Value {
        match line["method"].as_str().unwrap() {
            "tools/list" => json!({"result": srv.tools_list()}),
            "tools/call" => match srv.tools_call(&line["params"]) {
                Ok(result) => json!({"result": result}),
                Err((code, message)) => json!({"error": {"code": code, "message": message}}),
            },
            _ => unreachable!(),
        }
    }

    fn tool_call(tool: &str, cmd: &str) -> Value {
        json!({"method":"tools/call","params":{"name":tool,"arguments":{"command":cmd}}})
    }

    #[test]
    fn read_only_mode_lists_only_query() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(
            &mut srv,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
        );
        let names: Vec<_> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].clone())
            .collect();
        assert_eq!(names, vec![json!("query")]);
    }

    #[test]
    fn query_runs_read_only_commands() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(&mut srv, tool_call("query", "list jobs"));
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(r["result"]["content"][0]["text"], "ran list jobs\n");
        let r = call(&mut srv, tool_call("query", ".clients"));
        assert_eq!(r["result"]["isError"], false);
    }

    #[test]
    fn query_rejects_writes_without_running_them() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        for cmd in [
            "delete volume=Full-0001 yes",
            "run job=Backup yes",
            ".sql query=\"DELETE FROM job\"",
            "purge volume=x",
        ] {
            let r = call(&mut srv, tool_call("query", cmd));
            assert_eq!(r["result"]["isError"], true, "{cmd}");
        }
        assert!(srv.runner.lock().unwrap().0.is_empty());
    }

    #[test]
    fn query_rejects_mutating_dot_commands_and_prefixes() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        for cmd in [
            ".bvfs_update",
            ".bvfs_r",
            ".catalog delete",
            ".catalog reset yes confirm=bareos force",
            ".catalog drop table=job yes confirm=bareos",
            ".catalog create yes confirm=bareos",
            ".acl set",
            ".ap 2",
            ".grpc on",
            ".sql query=DELETE",
            ".unknown",
        ] {
            let r = call(&mut srv, tool_call("query", cmd));
            assert_eq!(r["result"]["isError"], true, "{cmd}");
        }
        assert!(srv.runner.lock().unwrap().0.is_empty());
    }

    #[test]
    fn execute_disabled_without_flag() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let r = call(&mut srv, tool_call("execute", "run job=Backup yes"));
        assert!(r["error"].is_object());
        assert!(srv.runner.lock().unwrap().0.is_empty());
    }

    #[cfg(feature = "mcp-development")]
    #[test]
    fn execute_allowed_with_flag() {
        let mut srv = McpServer::new(Echo(vec![]), true, "dir".into());
        let r = call(&mut srv, tool_call("execute", "run job=Backup yes"));
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(srv.runner.lock().unwrap().0, vec!["run job=Backup yes"]);
    }

    #[test]
    fn multi_line_and_session_commands_rejected() {
        let mut srv = McpServer::new(Echo(vec![]), true, "dir".into());
        for cmd in [
            "list jobs\ndelete volume=x yes",
            "quit",
            ".api 2",
            "@exit",
            "",
        ] {
            let r = call(&mut srv, tool_call("query", cmd));
            assert_eq!(r["result"]["isError"], true, "{cmd:?}");
        }
        assert!(srv.runner.lock().unwrap().0.is_empty());
    }

    #[cfg(feature = "mcp-production")]
    #[test]
    fn production_cannot_enable_execute_even_with_internal_write_request() {
        let mut srv = McpServer::new(Echo(vec![]), true, "dir".into());
        assert!(!srv.write_enabled());
        assert_eq!(srv.tools_list()["tools"].as_array().unwrap().len(), 1);
        assert!(call(&mut srv, tool_call("execute", "run job=Backup yes"))["error"].is_object());
        assert!(srv.runner.lock().unwrap().0.is_empty());
    }

    #[cfg(feature = "mcp-development")]
    #[test]
    fn development_lists_execute_only_after_runtime_opt_in() {
        let srv = McpServer::new(Echo(vec![]), true, "dir".into());
        assert_eq!(srv.tools_list()["tools"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn rmcp_stdio_handshake_and_tool_call() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (client, server) = tokio::io::duplex(16384);
        let capture = crate::mcp_mirror::tests::Capture::default();
        let mut handler = McpServer::new(Echo(vec![]), false, "dir".into());
        handler.mirror = capture.mirror();
        let access = handler.mirror.access("stdio");
        let (read, write) = tokio::io::split(server);
        let transport = (
            crate::mcp_mirror::MirrorIo::new(read, access.clone(), "request"),
            crate::mcp_mirror::MirrorIo::new(write, access, "response"),
        );
        let task = tokio::spawn(async move {
            ServiceExt::serve(handler, transport)
                .await
                .unwrap()
                .waiting()
                .await
                .unwrap();
        });
        let (read, mut write) = tokio::io::split(client);
        let mut lines = BufReader::new(read).lines();
        write.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"1\"}}}\n").await.unwrap();
        let reply: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(reply["result"]["serverInfo"]["name"], "oconsole");
        assert_eq!(reply["result"]["protocolVersion"], "2024-11-05");
        write
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .await
            .unwrap();
        for (id, method, params) in [
            (2, "tools/list", json!({})),
            (
                3,
                "tools/call",
                json!({"name":"query","arguments":{"command":"list jobs"}}),
            ),
            (
                4,
                "tools/call",
                json!({"name":"execute","arguments":{"command":"run job=Backup yes"}}),
            ),
        ] {
            let request = json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params});
            write
                .write_all(format!("{request}\n").as_bytes())
                .await
                .unwrap();
            let reply: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(reply["id"], id);
            match id {
                2 => assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 1),
                3 => assert_eq!(reply["result"]["content"][0]["text"], "ran list jobs\n"),
                4 => assert!(reply["error"].is_object()),
                _ => unreachable!(),
            }
        }
        drop(write);
        drop(lines);
        tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        let records = capture.records();
        assert!(
            records.iter().any(|r| r["direction"] == "request"
                && r["body"]["method"] == "notifications/initialized")
        );
        for id in [1, 2, 3, 4] {
            assert!(
                records
                    .iter()
                    .any(|r| r["direction"] == "request" && r["body"]["id"] == id)
            );
            assert!(
                records
                    .iter()
                    .any(|r| r["direction"] == "response" && r["body"]["id"] == id)
            );
        }
        assert!(
            records
                .iter()
                .any(|r| r["body"]["result"]["content"][0]["text"] == "ran list jobs\n")
        );
        assert!(
            records
                .iter()
                .any(|r| r["body"]["id"] == 4 && r["body"]["error"].is_object())
        );
    }
    #[tokio::test]
    async fn http_mirrors_handshake_tools_errors_and_notifications_without_auth_headers() {
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;
        #[cfg(feature = "mcp-production")]
        let modes = [false];
        #[cfg(feature = "mcp-development")]
        let modes = [false, true];
        for allow_write in modes {
            let capture = crate::mcp_mirror::tests::Capture::default();
            let mut server = McpServer::new(Echo(vec![]), allow_write, "dir".into());
            server.mirror = capture.mirror();
            let token = "private-token-not-for-terminal";
            let app = server.http_router(token.into(), vec![]);
            let request = |body: Value, session: Option<&str>| {
                let mut builder = axum::http::Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("host", "localhost")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream");
                if let Some(session) = session {
                    builder = builder.header("mcp-session-id", session);
                }
                builder
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap()
            };
            let response = app.clone().oneshot(request(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"mirror-test","version":"1"}}}), None)).await.unwrap();
            assert_eq!(response.status(), 200);
            let session = response
                .headers()
                .get("mcp-session-id")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let body = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                response.into_body().collect(),
            )
            .await
            .unwrap()
            .unwrap()
            .to_bytes();
            assert!(String::from_utf8_lossy(&body).contains("oconsole"));
            let response = app
                .clone()
                .oneshot(request(
                    json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                    Some(&session),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), 202);
            for (id, method, params) in [
                (2, "tools/list", json!({})),
                (
                    3,
                    "tools/call",
                    json!({"name":"query","arguments":{"command":"list jobs"}}),
                ),
                (
                    4,
                    "tools/call",
                    json!({"name":"execute","arguments":{"command":"run job=Backup yes"}}),
                ),
                (
                    5,
                    "tools/call",
                    json!({"name":"query","arguments":{"command":"delete volume=x"}}),
                ),
                (6, "no-such-method", json!({})),
                (
                    7,
                    "tools/call",
                    json!({"name":"query","arguments":{"command":"status fail"}}),
                ),
            ] {
                let response = app
                    .clone()
                    .oneshot(request(
                        json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
                        Some(&session),
                    ))
                    .await
                    .unwrap();
                // HTTP may return a direct JSON response or finite SSE stream.
                let bytes = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    response.into_body().collect(),
                )
                .await
                .unwrap()
                .unwrap()
                .to_bytes();
                assert!(!bytes.is_empty(), "missing response for {method}");
                if id == 4 {
                    assert_eq!(
                        String::from_utf8_lossy(&bytes).contains("ran run job=Backup yes"),
                        allow_write
                    );
                }
                let records = capture.records();
                assert!(
                    records
                        .iter()
                        .any(|r| r["direction"] == "request-body" && r["body"]["id"] == id)
                );
                assert!(records.iter().any(|r| {
                    r["direction"] == "response-body"
                        && (r["body"]["id"] == id
                            || r["body"]["text"]
                                .as_str()
                                .is_some_and(|text| text.contains(&format!("\"id\":{id}"))))
                }));
            }
            let denied = axum::http::Request::builder()
                .method("POST")
                .uri("/mcp")
                .header("authorization", "Bearer invalid-private-token")
                .body(axum::body::Body::empty())
                .unwrap();
            let response = app.oneshot(denied).await.unwrap();
            assert_eq!(response.status(), 401);
            response.into_body().collect().await.unwrap();
            let records = capture.records();
            assert!(records.iter().any(|r| r["direction"] == "response-body"
                && r["body"]["text"] == "MCP bearer token required"));
            assert!(
                !serde_json::to_string(&records)
                    .unwrap()
                    .contains("invalid-private-token")
            );
            assert!(
                records
                    .iter()
                    .any(|r| r["body"]["method"] == "notifications/initialized")
            );
            assert!(records.iter().any(|r| {
                r["direction"] == "response-body"
                    && serde_json::to_string(&r["body"])
                        .unwrap()
                        .contains("test Director unavailable")
            }));
            assert!(!serde_json::to_string(&records).unwrap().contains(token));
        }
    }

    #[tokio::test]
    async fn http_requires_token_and_accepts_authenticated_initialize() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let token = "a".repeat(64);
        let app =
            McpServer::new(Echo(vec![]), false, "dir".into()).http_router(token.clone(), vec![]);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        for (auth, expected) in [
            (String::new(), "401"),
            ("Authorization: Bearer wrong\r\n".into(), "401"),
            (format!("Authorization: Bearer {token}\r\n"), "200"),
        ] {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            let body = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}).to_string();
            let request = format!(
                "POST /mcp HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut bytes = [0u8; 8192];
            let size =
                tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut bytes))
                    .await
                    .unwrap()
                    .unwrap();
            let response = String::from_utf8_lossy(&bytes[..size]);
            assert!(
                response.starts_with(&format!("HTTP/1.1 {expected}")),
                "{response}"
            );
        }
        task.abort();
    }
}
