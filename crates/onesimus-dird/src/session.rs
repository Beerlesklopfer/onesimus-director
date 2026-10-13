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

    // CRAM-MD5 two-way authentication (Director is server = initiated_by_remote=true)
    if !password.is_empty() {
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
    } else {
        // No password configured — allow unauthenticated
        session_mgr.authenticate_session(mgr_session_id);
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
    let info_msg = format!("1002{rs}You are connected using the default console");
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
        let ast = parse_cli_command(cmd_str);

        // Check for quit
        if matches!(ast.command_name.as_str(), "quit" | "exit" | ".quit") {
            let _ = bnet.send_str("bye\n");
            let _ = bnet.send_signal(BnetSignal::Terminate);
            break;
        }

        // C++ ua_cmds.cc Do_a_command(): in API mode the command is framed by
        // CMD_BEGIN ... CMD_OK/CMD_FAILED; in JSON mode the json-rpc result
        // follows after that (FinalizeResult). `.api` switches the mode
        // before it produces output, so its own reply is already JSON.
        if api_mode != command_executor::ApiMode::Human {
            if bnet.send_signal(BnetSignal::CmdBegin).is_err() { break; }
        }
        if ast.command_name == ".api" {
            api_mode = parse_api_mode(&ast).unwrap_or(api_mode);
        }

        // Execute command
        let ctx = ExecContext {
            session_id,
            config: config.clone(),
            session_mgr: Some(session_mgr.clone()),
            catalog: catalog.clone(),
            api_mode,
            bus_rx: Some(bus_rx.clone()),
            job_manager: job_manager.clone(),
        };
        let result = command_executor::dispatch(&ast, &ctx);

        // Session-altering commands
        match ast.command_name.as_str() {
            "autodisplay" => {
                let flag = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
                auto_display = flag != "off";
            }
            "gui" => {
                let flag = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("on");
                gui_mode = flag != "off";
            }
            ".grpc" => {
                let on = ast.arguments.iter().any(|a| a.key == "on");
                let off = ast.arguments.iter().any(|a| a.key == "off");
                if on { grpc_mode = true; api_mode = command_executor::ApiMode::Json; }
                if off { grpc_mode = false; }
            }
            _ => {}
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

        // HandleMessages: drain pending bus messages (like C++ ua_server.cc).
        // In API mode messages never interleave with the command output;
        // the console is only told via MSGS_PENDING and fetches them itself.
        if auto_display {
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
    let mut parts = input.splitn(2, char::is_whitespace);
    let command_name = parts.next().unwrap_or("").to_string();
    let args_str = parts.next().unwrap_or("");

    let mut arguments = Vec::new();

    // Parse arguments: key=value or bare words
    for token in args_str.split_whitespace() {
        if let Some(eq_pos) = token.find('=') {
            let key = token[..eq_pos].to_string();
            let value = token[eq_pos + 1..].trim_matches('"').to_string();
            arguments.push(Argument {
                key,
                value: Some(value),
            });
        } else {
            arguments.push(Argument {
                key: token.to_string(),
                value: None,
            });
        }
    }

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
