//! MCP server mode (`oconsole --mcp`).
//!
//! Speaks the Model Context Protocol (JSON-RPC 2.0, one message per line) on
//! stdin/stdout so AI assistants can query the Director. The Director
//! connection (BNET or gRPC) is opened before serving, with the same config
//! and login as the interactive console. stdout carries protocol only; all
//! diagnostics go to stderr.
//!
//! This module is the server, transport and protocol part; the tools live in
//! their modules and tools_list/tools_call only dispatch:
//!   mcp_read.rs  — read tools: `query` (read-only allowlist, mcp_read/query.rs),
//!                  `ovfs_*` (mcp_read/ovfs.rs) and the typed read tools
//!   mcp_write.rs — write and restore tools (typed, mcp_write/admin.rs) and,
//!                  development build only, the generic `execute`
//!   mcp_tree.rs  — the interactive restore file selection (restore_tree_*)
//!   mcp_typed.rs — the typed DirectorApi route used by every tool over gRPC
//!
//! Write access (C-087, S-36): local stdio offers the write and restore
//! tools in every build (`--mcp-read-only` turns them off); Streamable HTTP
//! is read-only unless the binary is a development build
//! (`mcp-development`) started with `--mcp-allow-write`.

use std::sync::{Arc, Mutex};

use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};

use serde_json::{Value, json};

/// Runs one command line against the Director and returns its text output.
pub trait CommandRunner: Send + 'static {
    fn run(&mut self, command: &str) -> anyhow::Result<String>;

    /// Runs one command under the Director's JSON API (`.api json compact=yes`,
    /// then `.api 0` again, so the session stays in plain console mode).
    fn run_json(&mut self, command: &str) -> anyhow::Result<String> {
        self.run(".api json compact=yes")?;
        let out = self.run(command);
        self.run(".api 0")?;
        out
    }

    /// Runs one command and answers the Director's questions: `answer`
    /// gets the output so far (ending with the question) and returns the
    /// reply, or None to cancel with ".". Runners without dialogs (gRPC)
    /// run the command plainly.
    fn run_dialog(
        &mut self,
        command: &str,
        _answer: &mut dyn FnMut(&str) -> Option<String>,
    ) -> anyhow::Result<String> {
        self.run(command)
    }

    /// The connection is gRPC: write tools that would need a dialog append
    /// the Director's `yes` keyword instead (ExecuteCommand cannot answer).
    fn is_grpc(&self) -> bool {
        false
    }

    /// Call one RPC of the typed `DirectorApi` (encoded request, encoded
    /// response). `None`: this connection has no typed API (BNET).
    fn typed_call(&mut self, _rpc: &str, _request: Vec<u8>) -> Option<Result<Vec<u8>, TypedError>> {
        None
    }

    /// Start a command that may stay in a dialog (BNET): returns the output
    /// up to the Director's next question, or to the end of the command.
    fn dialog_start(&mut self, _command: &str) -> anyhow::Result<Dialog> {
        anyhow::bail!("interactive Director dialogs need a BNET connection")
    }

    /// Answer the open question of a dialog started with dialog_start.
    fn dialog_send(&mut self, _reply: &str) -> anyhow::Result<Dialog> {
        anyhow::bail!("interactive Director dialogs need a BNET connection")
    }
}

/// Failure of a typed DirectorApi call.
#[derive(Debug, Clone, PartialEq)]
pub enum TypedError {
    /// The Director does not implement the RPC (production Director:
    /// write RPCs, S-35): use ExecuteCommand instead.
    Unimplemented,
    /// Any other gRPC status (code name, message).
    Status(String, String),
}

/// One step of a BNET dialog.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dialog {
    /// Output since the command or the last answer.
    pub output: String,
    /// The Director waits for an answer (its question ends `output`).
    pub waiting: bool,
}

/// Most Director questions answered within one command; later ones are cancelled.
pub const MAX_QUESTIONS: usize = 20;

/// Write tools over Streamable HTTP are compiled only into the development
/// build; the production build serves HTTP read-only whatever it is asked.
pub const HTTP_WRITE_COMPILED: bool = cfg!(feature = "mcp-development");

/// Whether the write and restore tools are offered (C-087): stdio unless
/// `--mcp-read-only`; HTTP only in the development build with
/// `--mcp-allow-write`.
pub fn write_access(http: bool, allow_write_flag: bool, read_only_flag: bool) -> bool {
    if read_only_flag {
        return false;
    }
    if http { HTTP_WRITE_COMPILED && allow_write_flag } else { true }
}

pub struct McpServer<R: CommandRunner> {
    runner: Arc<Mutex<R>>,
    write: bool,
    director_name: String,
    mirror: crate::mcp_mirror::Mirror,
    tree: Arc<Mutex<crate::mcp_tree::TreeState>>,
}

impl<R: CommandRunner> McpServer<R> {
    /// `write`: offer the write and restore tools (see [`write_access`]);
    /// over HTTP the production build drops it again.
    pub fn new(runner: R, write: bool, director_name: String) -> Self {
        Self {
            runner: Arc::new(Mutex::new(runner)),
            write,
            director_name,
            mirror: crate::mcp_mirror::Mirror::stderr(),
            tree: Arc::default(),
        }
    }

    fn write_enabled(&self) -> bool {
        self.write
    }

    fn share(&self) -> Self {
        Self {
            runner: self.runner.clone(),
            write: self.write,
            director_name: self.director_name.clone(),
            mirror: self.mirror.clone(),
            tree: self.tree.clone(),
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
        let runtime = tokio::runtime::Runtime::new()?;
        let result = runtime.block_on(async move {
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
                // End on Ctrl-D (end of stdin: the transport closes) and on Ctrl-C, also
                // while the client has not sent "initialize" yet.
                let interrupted = async {
                    while !crate::CTRL_C.load(std::sync::atomic::Ordering::Relaxed) {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                };
                tokio::pin!(interrupted);
                tokio::select! {
                    started = ServiceExt::serve(self, transport) => match started {
                        Ok(service) => {
                            tokio::select! {
                                result = service.waiting() => { result?; }
                                _ = &mut interrupted => {}
                            }
                        }
                        // stdin ended before the first request: a normal end
                        Err(e) if e.to_string().starts_with("connection closed") => {}
                        Err(e) => return Err(e.into()),
                    },
                    _ = &mut interrupted => {}
                }
            }
            Ok(())
        });
        drop(runner);
        // The blocking read of stdin cannot be interrupted: do not wait for it.
        runtime.shutdown_background();
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
                let mut server = self.share();
                server.write = server.write && HTTP_WRITE_COMPILED;
                Ok(server)
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
                 Use 'help' or '.help all' to discover commands. \
                 Prefer the typed tools (list_*, show, status_*, dot_*, version, time, messages{}); \
                 they return the Director's JSON (typed DirectorApi results over gRPC). \
                 Browse backup contents with the ovfs_* tools (ovfs_latest, ovfs_tree, ovfs_ls, ovfs_versions){}",
                self.director_name,
                if self.write_enabled() { ", and the write tools run_job, restore, label_volume, update_volume, …" } else { "" },
                if self.write_enabled() {
                    "; restore with 'restore' in one call or interactively with restore_tree_start … restore_tree_done. \
                     Destructive tools need confirm=true: ask the user first."
                } else { "." },
            ),
        }))
        .expect("valid server metadata")
    }

    /// All tools of this build and mode, in a fixed order: the read tools
    /// (mcp_read.rs: query, ovfs_*, typed), then — with write access — the
    /// write tools (mcp_write.rs: typed, admin, restore tree, and in the
    /// development build `execute`).
    fn tools_list(&self) -> Value {
        let mut tools = crate::mcp_read::all_tools();
        if self.write_enabled() {
            tools.extend(crate::mcp_write::all_tools());
        }
        json!({ "tools": tools })
    }

    /// Dispatch a tools/call to the module owning the tool; a name no module
    /// claims is a JSON-RPC error (-32602).
    fn tools_call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if self.write_enabled() {
            if let Some(result) = crate::mcp_tree::call(name, &arguments, &self.runner, &self.tree) {
                return result;
            }
        }
        // A BNET restore file selection holds the connection in its dialog.
        let open = self.tree.lock().map_err(|_| (-32603, "restore state unavailable".to_string()))?.bnet_session();
        if let Some(id) = open {
            if crate::mcp_write::is_tool(name) || crate::mcp_read::is_tool(name) {
                return Ok(tool_result(&format!(
                    "The restore file selection '{id}' is open on this BNET connection; finish it with \
                     restore_tree_done or restore_tree_abort before calling {name}."), true));
            }
        }
        if self.write_enabled() {
            if let Some(result) = crate::mcp_write::call(name, &arguments, &self.runner) {
                return result;
            }
        }
        if let Some(result) = crate::mcp_read::call(name, &arguments, self.write_enabled(), &self.runner) {
            return result;
        }
        match name {
            "execute" => Err((-32602, "tool 'execute' is unavailable in this build or disabled".into())),
            _ => Err((-32602, format!("unknown tool: {name}"))),
        }
    }
}

/// A tool result with one text item.
pub(crate) fn tool_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Run a command line that passed the tool's checks; its text is the result.
pub(crate) fn run_plain<R: CommandRunner>(runner: &Mutex<R>, command: &str) -> Result<Value, (i64, String)> {
    match runner
        .lock()
        .map_err(|_| (-32603, "Director runner unavailable".into()))?
        .run(command)
    {
        Ok(out) if out.trim().is_empty() => Ok(tool_result("(no output)", false)),
        Ok(out) => Ok(tool_result(&out, false)),
        Err(e) => Ok(tool_result(&format!("Director error: {e}"), true)),
    }
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
        let server = self.share();
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

    /// Compare with (or, with UPDATE_MCP_SNAPSHOTS set, write) a JSON file
    /// in crates/oconsole/testdata/.
    fn snapshot(file: &str, actual: &Value) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(file);
        let text = serde_json::to_string_pretty(actual).unwrap() + "\n";
        if std::env::var_os("UPDATE_MCP_SNAPSHOTS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(expected == text, "{file} changed; diff it against the snapshot");
    }

    /// The modes of this build: read-only (HTTP, --mcp-read-only) and with
    /// write access (stdio; development HTTP with --mcp-allow-write).
    fn modes() -> Vec<(&'static str, bool)> {
        #[cfg(feature = "mcp-production")]
        return vec![("production", false), ("production-stdio", true)];
        #[cfg(feature = "mcp-development")]
        return vec![("development", false), ("development-write", true)];
    }

    /// tools/list must stay byte-identical (names, schemas, annotations, order).
    #[test]
    fn tools_list_matches_snapshot() {
        for (mode, allow_write) in modes() {
            let srv = McpServer::new(Echo(vec![]), allow_write, "dir".into());
            snapshot(&format!("tools_list_{mode}.json"), &srv.tools_list());
        }
    }

    /// Results and Director command lines of a fixed set of calls (generic
    /// query/execute, ovfs, typed read and write tools, refusals).
    #[test]
    fn tool_calls_match_snapshot() {
        let calls = [
            tool_call("query", "list jobs"),
            tool_call("query", "delete volume=x yes"),
            tool_call("query", ".ovfs_status"),
            tool_call("query", "list jobs\nquit"),
            tool_call("query", "status fail"),
            tool_call("execute", "run job=Backup yes"),
            tool_call("execute", "quit"),
            json!({"method":"tools/call","params":{"name":"ovfs_tree","arguments":{"jobids":"1,2","depth":2}}}),
            json!({"method":"tools/call","params":{"name":"ovfs_ls","arguments":{"jobids":"1"}}}),
            json!({"method":"tools/call","params":{"name":"ovfs_versions","arguments":{"client":"fd","path":"/etc/","name":"passwd"}}}),
            json!({"method":"tools/call","params":{"name":"ovfs_nosuch","arguments":{}}}),
            json!({"method":"tools/call","params":{"name":"list_jobs","arguments":{"client":"fd"}}}),
            json!({"method":"tools/call","params":{"name":"show","arguments":{"resource":"job","name":"x;y"}}}),
            json!({"method":"tools/call","params":{"name":"run_job","arguments":{"job":"Backup","level":"Full"}}}),
            json!({"method":"tools/call","params":{"name":"purge","arguments":{"kind":"volume","volume":"V"}}}),
            json!({"method":"tools/call","params":{"name":"delete","arguments":{"kind":"jobid","jobids":[3],"confirm":true}}}),
            json!({"method":"tools/call","params":{"name":"nosuch","arguments":{}}}),
        ];
        for (mode, allow_write) in modes() {
            let mut srv = McpServer::new(Echo(vec![]), allow_write, "dir".into());
            let results: Vec<Value> = calls.iter().map(|c| call(&mut srv, c.clone())).collect();
            let ran = srv.runner.lock().unwrap().0.clone();
            snapshot(&format!("tool_calls_{mode}.json"),
                &json!({ "results": results, "director_commands": ran }));
        }
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
        assert_eq!(names[..6], [json!("query"), json!("ovfs_status"), json!("ovfs_latest"),
            json!("ovfs_tree"), json!("ovfs_ls"), json!("ovfs_versions")]);
        // then the typed read-only tools (mcp_read.rs)
        assert_eq!(names.len(), 6 + crate::mcp_read::tools().len());
        assert!(names.contains(&json!("list_jobs")) && names.contains(&json!("dot_jobstatus")));
        for tool in r["result"]["tools"].as_array().unwrap() {
            assert_eq!(tool["annotations"]["readOnlyHint"], true, "{}", tool["name"]);
        }
    }

    #[test]
    fn ovfs_tools_build_read_only_command_lines() {
        let mut srv = McpServer::new(Echo(vec![]), false, "dir".into());
        let ovfs = |tool: &str, args: Value| json!({"method":"tools/call","params":{"name":tool,"arguments":args}});
        for (tool, args, line) in [
            ("ovfs_status", json!({}), ".ovfs_status json"),
            ("ovfs_latest", json!({"client":"pc-anna"}), ".ovfs_latest client=\"pc-anna\" json"),
            ("ovfs_tree", json!({"jobids":"40, 41,42","root":"/home/a b/","depth":2}),
                ".ovfs_tree jobid=40,41,42 root=\"/home/a b/\" depth=2 json"),
            ("ovfs_ls", json!({"client":"fd","fileset":"Linux","path":"/it's \"x\"/"}),
                ".ovfs_ls client=\"fd\" fileset=\"Linux\" path=\"/it's \\\"x\\\"/\" json"),
            ("ovfs_versions", json!({"client":"fd","path":"/etc/","name":"passwd","copies":true}),
                ".ovfs_versions client=\"fd\" path=\"/etc/\" name=\"passwd\" copies json"),
        ] {
            let r = call(&mut srv, ovfs(tool, args));
            assert_eq!(r["result"]["isError"], false, "{tool}: {r}");
            assert_eq!(srv.runner.lock().unwrap().0.last().unwrap(), line);
        }
        let ran = srv.runner.lock().unwrap().0.len();
        for (tool, args) in [
            ("ovfs_tree", json!({})),
            ("ovfs_tree", json!({"jobids":"1;delete"})),
            ("ovfs_ls", json!({"jobids":"1"})),
            ("ovfs_ls", json!({"jobids":"1","path":"/a\nb/"})),
            ("ovfs_versions", json!({"client":"fd"})),
            ("ovfs_tree", json!({"jobids":"1","depth":"x"})),
        ] {
            let r = call(&mut srv, ovfs(tool, args.clone()));
            assert_eq!(r["result"]["isError"], true, "{tool} {args}");
        }
        assert_eq!(srv.runner.lock().unwrap().0.len(), ran);
        assert!(call(&mut srv, ovfs("ovfs_nosuch", json!({})))["error"].is_object());
        for cmd in [".ovfs_status", ".ovfs_tree jobid=1", ".ovfs_ls jobid=1 path=/"] {
            assert_eq!(call(&mut srv, tool_call("query", cmd))["result"]["isError"], false, "{cmd}");
        }
        // prefixes are not read-only names
        assert_eq!(call(&mut srv, tool_call("query", ".ovfs"))["result"]["isError"], true);
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

    /// C-087: production stdio offers the write and restore tools, but not
    /// the generic `execute`.
    #[cfg(feature = "mcp-production")]
    #[test]
    fn production_stdio_offers_write_tools_without_execute() {
        let mut srv = McpServer::new(Echo(vec![]), true, "dir".into());
        let tools = srv.tools_list();
        let names: Vec<&str> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        for tool in WRITE_TOOLS.iter().chain(["restore_tree_start", "add_volumes", "sqlquery"].iter()) {
            assert!(names.contains(tool), "{tool} on production stdio");
        }
        assert!(!names.contains(&"execute"));
        assert!(call(&mut srv, tool_call("execute", "run job=Backup yes"))["error"].is_object());
        assert!(srv.runner.lock().unwrap().0.is_empty());
        let r = call(&mut srv, json!({"method":"tools/call","params":{"name":"run_job","arguments":{"job":"Backup"}}}));
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_eq!(srv.runner.lock().unwrap().0, vec!["run job=Backup yes"]);
    }

    #[test]
    fn write_access_matrix() {
        // stdio: every build, unless --mcp-read-only
        assert!(write_access(false, false, false));
        assert!(!write_access(false, true, true));
        // HTTP: only the development build with --mcp-allow-write
        assert!(!write_access(true, false, false));
        assert_eq!(write_access(true, true, false), cfg!(feature = "mcp-development"));
        assert!(!write_access(true, true, true));
    }

    #[test]
    fn write_mode_lists_all_write_tools() {
        let names =|srv: &McpServer<Echo>| -> Vec<String> {
            srv.tools_list()["tools"].as_array().unwrap().iter()
                .map(|t| t["name"].as_str().unwrap().to_string()).collect()
        };
        let on = names(&McpServer::new(Echo(vec![]), true, "dir".into()));
        let off = names(&McpServer::new(Echo(vec![]), false, "dir".into()));
        let execute: &[&str] = if cfg!(feature = "mcp-development") { &["execute"] } else { &[] };
        for tool in WRITE_TOOLS.iter().chain(execute) {
            assert!(on.iter().any(|n| n == tool), "{tool} listed with write access");
            assert!(!off.iter().any(|n| n == tool), "{tool} hidden without write access");
        }
        assert_eq!(on.len(), off.len() + crate::mcp_write::all_tools().len());
        let without = McpServer::new(Echo(vec![]), false, "dir".into()).tools_list();
        assert!(without["tools"].as_array().unwrap().iter().all(|t| t["annotations"]["readOnlyHint"] == true));
    }

    /// The typed write/restore tools of mcp_write.rs (S-31). Spelled out here
    /// because the production build does not compile that module.
    const WRITE_TOOLS: [&str; 23] = [
        "run_job", "rerun_job", "cancel_job", "wait_job", "restore", "label_volume",
        "relabel_volume", "mount_storage", "unmount_storage", "release_storage",
        "update_volume", "update_pool", "prune", "purge", "delete", "truncate", "enable",
        "disable", "configure_add", "configure_export", "reload", "setdebug", "setbandwidth",
    ];

    /// Plausible arguments, so a refusal is about the tool, not the arguments.
    fn write_call(tool: &str) -> Value {
        json!({"method":"tools/call","params":{"name":tool,"arguments":{
            "job":"Backup","client":"fd","jobids":[1],"all":true,"storage":"File","volume":"V",
            "pool":"Full","kind":"volume","confirm":true,"resource":"job","name":"Backup",
            "level":1,"target":"dir","limit":1,"jobid":1}}})
    }

    /// Without write access (HTTP in production, --mcp-read-only): no write
    /// tool is listed, every call is refused before the Director.
    #[test]
    fn write_tools_absent_and_refused_without_write_access() {
        for allow_write in [false] {
            let mut srv = McpServer::new(Echo(vec![]), allow_write, "dir".into());
            let tools = srv.tools_list();
            for tool in WRITE_TOOLS {
                assert!(tools["tools"].as_array().unwrap().iter().all(|t| t["name"] != tool), "{tool} listed");
                let r = call(&mut srv, write_call(tool));
                assert!(r["error"]["message"].as_str().unwrap().contains("unknown tool"), "{tool}: {r}");
            }
            assert!(srv.runner.lock().unwrap().0.is_empty());
        }
    }

    #[test]
    fn write_tools_run_with_write_access() {
        let mut srv = McpServer::new(Echo(vec![]), true, "dir".into());
        let r = call(&mut srv, json!({"method":"tools/call","params":{"name":"run_job","arguments":{"job":"Backup"}}}));
        assert_eq!(r["result"]["isError"], false, "{r}");
        let r = call(&mut srv, json!({"method":"tools/call","params":{"name":"delete","arguments":{"kind":"jobid","jobids":[3]}}}));
        assert_eq!(r["result"]["isError"], true, "no confirm: {r}");
        let r = call(&mut srv, json!({"method":"tools/call","params":{"name":"delete","arguments":{"kind":"jobid","jobids":[3],"confirm":true}}}));
        assert_eq!(r["result"]["isError"], false, "{r}");
        assert_eq!(srv.runner.lock().unwrap().0, vec!["run job=Backup yes", "delete jobid=3"]);
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
                // query + 5 read-only OVFS tools + typed read-only tools
                2 => assert_eq!(reply["result"]["tools"].as_array().unwrap().len(), 6 + crate::mcp_read::tools().len()),
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
        // HTTP drops write access unless this is a development build.
        for allow_write in [false, true] {
            let http_write = allow_write && HTTP_WRITE_COMPILED;
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
                        http_write
                    );
                }
                if id == 2 {
                    let body = String::from_utf8_lossy(&bytes);
                    assert_eq!(body.contains("\"run_job\""), http_write, "write tools over HTTP");
                    assert_eq!(body.contains("\"restore_tree_start\""), http_write);
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
