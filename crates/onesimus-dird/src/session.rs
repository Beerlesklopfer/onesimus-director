//! Console session — BNET command loop with FSM.
//!
//! Each console session runs in its own named thread (dir:session:N).
//! The session FSM handles authentication, command loop, and optional
//! gRPC upgrade.

use std::io::{Read, Write};
use std::sync::Arc;

use tracing::{info, debug, warn, error};

use bareos_bnet::{BnetStream, BnetSignal, BnetMessage, BnetError, CramMd5Auth};
use bareos_bnet::tls::TlsOrPlain;

use crate::catalog_worker::CatalogHandle;
use crate::command_ast::{CommandAst, Argument, CommandSource, ResponseFormat};
use crate::command_executor::{self, ExecContext};
use crate::command_queue::CommandQueue;
use crate::director_config::SharedConfig;
use crate::fsm::SessionState;
use crate::session_manager::SessionManager;

/// Handle a console session (BNET mode).
pub fn handle_console_session<S: Read + Write>(
    mut bnet: BnetStream<S>,
    session_id: u64,
    console_name: &str,
    password: &str,
    session_mgr: &Arc<SessionManager>,
    cmd_queue: &CommandQueue,
    config: &Option<SharedConfig>,
    job_manager: &Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
    catalog: &Option<CatalogHandle>,
) {
    // Register session
    let mgr_session_id = session_mgr.create_session(console_name);

    // CRAM-MD5 two-way authentication (Director is server = initiated_by_remote=true).
    // The caller only gets here with a configured password; never skip auth.
    if password.is_empty() {
        warn!("session {session_id}: no password for console '{console_name}', rejected");
        session_mgr.remove_session(mgr_session_id);
        return;
    }
    {
        let auth = CramMd5Auth::new(password, "bareos-dir", 0);
        match auth.two_way_authenticate(&mut bnet, true) {
            Ok(result) if result.success => {
                debug!("session {session_id}: CRAM-MD5 auth OK");
                session_mgr.authenticate_session(mgr_session_id);
            }
            Ok(_) => {
                warn!("session {session_id}: CRAM-MD5 auth failed for '{console_name}'");
                session_mgr.remove_session(mgr_session_id);
                return;
            }
            Err(e) => {
                warn!("session {session_id}: CRAM-MD5 error: {e}");
                session_mgr.remove_session(mgr_session_id);
                return;
            }
        }
    }

    // Post-18.2 protocol: send MessageID-based responses
    // Format: "<id><RS><payload>" where RS = 0x1e (Record Separator)
    let version = env!("CARGO_PKG_VERSION");
    let rs = "\x1e";

    // 1. kMessageIdOk (1000) with version info
    let ok_msg = format!("1000{rs}OK: bareos-dir Version: {version}");
    if bnet.send_str(&ok_msg).is_err() {
        session_mgr.remove_session(mgr_session_id);
        return;
    }

    // 2. kMessageIdInfoMessage (1002) with connection info
    // C++ ConsoleAuthenticatorFrom_18_2::SendInfoMessage()
    let info_msg = if console_name == "*UserAgent*" {
        format!("1002{rs}You are connected using the default console")
    } else {
        format!("1002{rs}You are logged in as: {console_name}")
    };
    if bnet.send_str(&info_msg).is_err() {
        session_mgr.remove_session(mgr_session_id);
        return;
    }

    let mut state = SessionState::CommandLoop;
    let mut api_mode = command_executor::ApiMode::Human;

    // Subscribe to MessageBus for job messages, status updates, etc.
    let bus_rx = session_mgr.bus().subscribe();

    // Session flags (set by autodisplay, gui, trace, .grpc commands)
    let mut auto_display = true;  // autodisplay on/off
    let mut gui_mode = false;     // gui on/off
    let mut grpc_mode = false;    // .grpc on/off

    // EOD after init messages — signals "ready for commands" to the console.
    // Command loop
    // C++ Director: NO signal after init messages, goes straight to recv().
    // Console must send command without waiting for prompt/EOD.
    // C++ ua_session.cc CommandLoop():
    //   api=true:  MainPrompt before recv, NO EOD after response
    //   api=false: NO MainPrompt, EOD after response (HandleMessages)
    // bconsole uses api=0 (default), so: NO MainPrompt, EOD after response.
    loop {
        // api mode: send MainPrompt before recv
        if api_mode != command_executor::ApiMode::Human {
            if bnet.send_signal(BnetSignal::MainPrompt).is_err() { break; }
        }

        // Receive command
        let msg = match bnet.recv() {
            Ok(BnetMessage::Data(data)) => {
                match String::from_utf8(data) {
                    Ok(s) => s,
                    Err(_) => continue,
                }
            }
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => {
                debug!("session {session_id}: client sent TERMINATE");
                break;
            }
            Ok(BnetMessage::Signal(_)) => continue,
            Err(BnetError::HardEof) | Err(BnetError::Terminated) => {
                debug!("session {session_id}: connection closed");
                break;
            }
            Err(e) => {
                // Connection reset / broken pipe = normales Session-Ende (kein Fehler)
                debug!("session {session_id}: recv: {e}");
                break;
            }
        };

        let cmd_str = msg.trim();
        if cmd_str.is_empty() {
            continue;
        }

        debug!("session {session_id}: command '{cmd_str}'");

        // Parse command into AST
        let mut ast = parse_cli_command(cmd_str);
        if let Some(name) = crate::command_registry::resolve_command_name(
            command_executor::get_registry(), &ast.command_name) {
            ast.command_name = name.to_string();
        }

        let mut ctx = ExecContext {
            session_id: mgr_session_id,
            config: config.clone(),
            session_mgr: Some(session_mgr.clone()),
            catalog: catalog.clone(),
            api_mode,
            bus_rx: Some(bus_rx.clone()),
            job_manager: job_manager.clone(),
        };
        let command_allowed=matches!(ast.command_name.as_str(),"quit"|"exit"|"whoami")
            || crate::acl::access(&ctx,crate::acl::Kind::Command,&ast.command_name);

        // C++ ua_cmds.cc Do_a_command(): in API mode the command is framed by
        // CMD_BEGIN ... CMD_OK/CMD_FAILED; in JSON mode the json-rpc result
        // follows after that (FinalizeResult). `.api` switches the mode
        // before it produces output, so its own reply is already JSON.
        if api_mode != command_executor::ApiMode::Human {
            if bnet.send_signal(BnetSignal::CmdBegin).is_err() { break; }
        }
        if command_allowed && ast.command_name == ".api" {
            ctx.api_mode = parse_api_mode(&ast).unwrap_or(api_mode);
        }

        // Execute command; only successful authorization commits local changes.
        let result = command_executor::dispatch(&ast, &ctx);

        // Session-altering commands
        if result.success {
        api_mode=ctx.api_mode;
        match ast.command_name.as_str() {
            "autodisplay" => {
                let flag = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
                auto_display = flag != "off";
            }
            "gui" => {
                let flag = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
                gui_mode = flag != "off";
                session_mgr.set_session_gui(mgr_session_id, gui_mode);
            }
            ".grpc" => {
                let on = ast.arguments.iter().any(|a| a.key == "on");
                let off = ast.arguments.iter().any(|a| a.key == "off");
                if on { grpc_mode = true; api_mode = command_executor::ApiMode::Json; }
                if off { grpc_mode = false; }
            }
            _ => {}
        }

        }

        if api_mode != command_executor::ApiMode::Human {
            let sig = if result.success { BnetSignal::CmdOk } else { BnetSignal::CmdFailed };
            if bnet.send_signal(sig).is_err() { break; }
        }

        // Send result
        if api_mode == command_executor::ApiMode::Json {
            if bnet.send_str(&crate::json_output::envelope(&result)).is_err() { break; }
        } else {
            if !result.text_output.is_empty() {
                let out = if result.text_output.ends_with('\n') {
                    result.text_output.clone()
                } else {
                    format!("{}\n", result.text_output)
                };
                if bnet.send_str(&out).is_err() { break; }
            }
            if !result.error_message.is_empty() {
                let err = if result.error_message.ends_with('\n') {
                    result.error_message.clone()
                } else {
                    format!("{}\n", result.error_message)
                };
                if bnet.send_str(&err).is_err() { break; }
            }
        }

        // C++ ua_cmds.cc quit_cmd() only sets ua->quit; ua_server.cc then
        // skips HandleMessages and the EOD and closes the socket without
        // sending BNET_TERMINATE.
        if command_allowed && result.success
            && matches!(ast.command_name.as_str(), "quit" | "exit" | ".quit") {
            break;
        }

        // HandleMessages: drain pending bus messages (like C++ ua_server.cc).
        // In API mode messages never interleave with the command output;
        // the console is only told via MSGS_PENDING and fetches them itself.
        if auto_display && api_mode != command_executor::ApiMode::Human {
            // API clients fetch messages explicitly; do not consume their queue.
            if !bus_rx.is_empty() && !gui_mode {
                let _ = bnet.send_signal(BnetSignal::MsgsPending);
            }
        } else if auto_display {
            let mut has_messages = false;
            while let Ok(msg) = bus_rx.try_recv() {
                let text = match msg {
                    crate::message_bus::BusMessage::ConsoleText { text } => text,
                    crate::message_bus::BusMessage::JobMessage { job_id, text, .. } if job_id > 0 => text,
                    _ => continue,
                };
                has_messages = true;
                if api_mode == command_executor::ApiMode::Human {
                    let _ = bnet.send_str(&format!("{text}\n"));
                }
            }
            if has_messages && !gui_mode {
                if api_mode == command_executor::ApiMode::Human {
                    let _ = bnet.send_str("You have messages.\n");
                } else {
                    let _ = bnet.send_signal(BnetSignal::MsgsPending);
                }
            }
        }

        // C++ ua_session.cc HandleMessages(): EOD only in non-API mode.
        // bconsole uses api=0, so it expects EOD after each command response.
        if api_mode == command_executor::ApiMode::Human {
            if bnet.send_signal(BnetSignal::Eod).is_err() { break; }
        }
    }

    // Cleanup
    session_mgr.remove_session(mgr_session_id);
    info!("session {session_id} ({console_name}) ended");
}

/// Parse a CLI command string into a CommandAST.
/// Split an argument string at whitespace outside of double quotes; quotes
/// are removed (`fileset="My Set"` -> `fileset=My Set`).
fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for c in input.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    args.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        args.push(cur);
    }
    args
}

/// Mode argument of `.api` (C++ DotApiCmd: no argument means 1).
fn parse_api_mode(ast: &CommandAst) -> Option<command_executor::ApiMode> {
    match ast.arguments.first().map(|a| a.value.as_deref().unwrap_or(a.key.as_str())) {
        None => Some(command_executor::ApiMode::Api),
        Some("0") | Some("off") => Some(command_executor::ApiMode::Human),
        Some("1") | Some("on") => Some(command_executor::ApiMode::Api),
        Some("2") | Some("json") => Some(command_executor::ApiMode::Json),
        Some(_) => None,
    }
}

/// Simple parser: first word is command name, rest are key=value arguments.
pub(crate) fn parse_cli_command(input: &str) -> CommandAst {
    let mut parts = input.trim().splitn(2, char::is_whitespace);
    let command_name = parts.next().unwrap_or("").to_string();
    let args_str = parts.next().unwrap_or("");

    // Like C++ ParseArgs(): whitespace separates arguments, double quotes
    // group (and are removed), key=value pairs split at the first '='.
    let arguments = split_args(args_str).into_iter().map(|token| {
        match token.split_once('=') {
            Some((key, value)) => Argument { key: key.to_string(), value: Some(value.to_string()) },
            None => Argument { key: token, value: None },
        }
    }).collect();

    CommandAst {
        command_name,
        arguments,
        session_id: String::new(),
        source: CommandSource::Cli,
        response_format: ResponseFormat::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quoted_values() {
        let ast = parse_cli_command(r#"run job="My Job" client="fd one" level=Full yes"#);
        assert_eq!(ast.arguments.len(), 4);
        assert_eq!(ast.arguments[0].key, "job");
        assert_eq!(ast.arguments[0].value.as_deref(), Some("My Job"));
        assert_eq!(ast.arguments[1].value.as_deref(), Some("fd one"));
        assert_eq!(ast.arguments[3].key, "yes");
        assert!(ast.arguments[3].value.is_none());
    }

    #[test]
    fn parse_empty_quoted_value() {
        let ast = parse_cli_command("configure add console name=x password=\"\"");
        assert_eq!(ast.arguments[3].key, "password");
        assert_eq!(ast.arguments[3].value.as_deref(), Some(""));
    }

    #[test]
    fn parse_simple_command() {
        let ast = parse_cli_command("status dir");
        assert_eq!(ast.command_name, "status");
        assert_eq!(ast.arguments.len(), 1);
        assert_eq!(ast.arguments[0].key, "dir");
        assert!(ast.arguments[0].value.is_none());
    }

    #[test]
    fn parse_command_with_args() {
        let ast = parse_cli_command("run job=BackupJob level=Full");
        assert_eq!(ast.command_name, "run");
        assert_eq!(ast.arguments.len(), 2);
        assert_eq!(ast.arguments[0].key, "job");
        assert_eq!(ast.arguments[0].value.as_deref(), Some("BackupJob"));
        assert_eq!(ast.arguments[1].key, "level");
        assert_eq!(ast.arguments[1].value.as_deref(), Some("Full"));
    }

    #[test]
    fn parse_bare_command() {
        let ast = parse_cli_command("version");
        assert_eq!(ast.command_name, "version");
        assert!(ast.arguments.is_empty());
    }

    #[test]
    fn parse_dot_command() {
        let ast = parse_cli_command(".clients");
        assert_eq!(ast.command_name, ".clients");
        assert!(ast.arguments.is_empty());
    }

    #[test]
    fn parse_quoted_value_no_spaces() {
        let ast = parse_cli_command("run job=\"BackupJob\"");
        assert_eq!(ast.arguments[0].key, "job");
        assert_eq!(ast.arguments[0].value.as_deref(), Some("BackupJob"));
    }

    #[test]
    fn parse_empty_input() {
        let ast = parse_cli_command("");
        assert_eq!(ast.command_name, "");
        assert!(ast.arguments.is_empty());
    }
}

#[cfg(test)]
mod acl_transport_tests {
    use super::*;
    #[test]
    fn authenticated_bnet_uses_server_session_and_denied_modes_have_no_effect() {
        use std::net::{TcpListener,TcpStream};
        use std::time::Duration;
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
        let ctx=crate::acl::tests::context();
        let server=std::thread::spawn(move || {
            let (stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            handle_console_session(BnetStream::new(stream),u64::MAX,"limited","pw",ctx.session_mgr.as_ref().unwrap(),
                &CommandQueue::new(16),&ctx.config,&None,&None);
        });
        let stream=TcpStream::connect(address).unwrap();stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bnet=BnetStream::new(stream);
        let result=CramMd5Auth::new("pw","limited",0).two_way_authenticate(&mut bnet,false).unwrap();assert!(result.success);
        assert!(bnet.recv_str().unwrap().starts_with("1000"));assert!(bnet.recv_str().unwrap().starts_with("1002"));
        for command in [".api 2","gui on","autodisplay off",".grpc on"] {
            bnet.send_str(command).unwrap();
            assert!(bnet.recv_str().unwrap().contains("is an invalid command"),"{command}");
            assert_eq!(bnet.recv().unwrap(),BnetMessage::Signal(BnetSignal::Eod));
        }
        bnet.send_str("version").unwrap();
        assert!(bnet.recv_str().unwrap().contains("bareos-dir Version"));
        assert_eq!(bnet.recv().unwrap(),BnetMessage::Signal(BnetSignal::Eod));
        bnet.send_str("quit").unwrap();drop(bnet);server.join().unwrap();
    }
}

#[cfg(test)]
mod quit_tests {
    //! C++ ua_cmds.cc quit_cmd() / ua_server.cc HandleUserAgentClientRequest():
    //! quit, exit and .quit send no reply and no BNET_TERMINATE; the socket is
    //! just closed.
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::sync::RwLock;
    use std::time::Duration;

    fn session(console: &str, acl: &str) -> (BnetStream<TcpStream>, std::thread::JoinHandle<()>) {
        let config = crate::director_config::DirectorConfig::from_str(&format!(
            "Director {{Name=dir; Password=secret}}\nConsole {{Name={console}; Password=pw; CommandACL={acl}}}"
        )).unwrap();
        let config: Option<SharedConfig> = Some(Arc::new(RwLock::new(config)));
        let manager = Arc::new(SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let name = console.to_string();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            handle_console_session(BnetStream::new(stream), 1, &name, "pw", &manager,
                &CommandQueue::new(16), &config, &None, &None);
        });
        let stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bnet = BnetStream::new(stream);
        assert!(CramMd5Auth::new("pw", console, 0).two_way_authenticate(&mut bnet, false).unwrap().success);
        assert!(bnet.recv_str().unwrap().starts_with("1000"));
        assert!(bnet.recv_str().unwrap().starts_with("1002"));
        (bnet, server)
    }

    #[test]
    fn quit_exit_dot_quit_close_without_reply() {
        for command in ["quit", "exit", ".quit", "q"] {
            let (mut bnet, server) = session("admin", "*all*");
            bnet.send_str(command).unwrap();
            assert!(bnet.recv().is_err(), "{command}: expected close without frames");
            server.join().unwrap();
        }
    }

    #[test]
    fn quit_in_json_api_mode_finishes_command_then_closes() {
        let (mut bnet, server) = session("admin", "*all*");
        bnet.send_str(".api 2").unwrap();
        while bnet.recv().unwrap() != BnetMessage::Signal(BnetSignal::MainPrompt) {}
        bnet.send_str("quit").unwrap();
        assert_eq!(bnet.recv().unwrap(), BnetMessage::Signal(BnetSignal::CmdBegin));
        assert_eq!(bnet.recv().unwrap(), BnetMessage::Signal(BnetSignal::CmdOk));
        let json: serde_json::Value = serde_json::from_str(&bnet.recv_str().unwrap()).unwrap();
        assert_eq!(json["result"], serde_json::json!({}));
        assert!(bnet.recv().is_err(), "expected close without MainPrompt or TERMINATE");
        server.join().unwrap();
    }

    #[test]
    fn dot_quit_obeys_command_acl_but_quit_is_always_allowed() {
        let (mut bnet, server) = session("limited", "version");
        bnet.send_str(".quit").unwrap();
        assert!(bnet.recv_str().unwrap().contains("is an invalid command"));
        assert_eq!(bnet.recv().unwrap(), BnetMessage::Signal(BnetSignal::Eod));
        bnet.send_str("quit").unwrap();
        assert!(bnet.recv().is_err());
        server.join().unwrap();
    }
}
