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

/// What the Director knows about a console connection before authentication.
pub struct ConsoleConnection {
    /// Console version from the Hello (C++ BareosVersionNumber, major * 100 + minor).
    pub version: u32,
    /// Negotiated TLS cipher (C++ TlsCipherGetName); None on a cleartext connection.
    pub tls_cipher: Option<String>,
}

impl Default for ConsoleConnection {
    fn default() -> Self {
        Self { version: bareos_bnet::hello::BAREOS_VERSION_RELEASE_18_2, tls_cipher: None }
    }
}

/// Handle a console session (BNET mode) of a console from 18.2 on, without TLS.
pub fn handle_console_session<S: Read + Write>(
    bnet: BnetStream<S>,
    session_id: u64,
    console_name: &str,
    password: &str,
    session_mgr: &Arc<SessionManager>,
    cmd_queue: &CommandQueue,
    config: &Option<SharedConfig>,
    job_manager: &Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
    catalog: &Option<CatalogHandle>,
) {
    handle_console_session_with(bnet, session_id, console_name, password, &ConsoleConnection::default(),
        session_mgr, cmd_queue, config, job_manager, catalog);
}

/// Connection banner after successful authentication, one string per BNET frame.
/// C++ authenticate_console.cc: ConsoleAuthenticatorBefore_18_2::
/// SendOkMessageWithSpaceSeparators (one frame with cipher) for consoles before
/// 18.2, otherwise ConsoleAuthenticatorFrom_18_2::SendResponseMessage (1000) and
/// SendInfoMessage (1002), fields separated by the record separator.
pub fn console_banner(director: &str, console_name: &str, connection: &ConsoleConnection) -> Vec<String> {
    use crate::version_strings::{DATE, FULL, SERVICES_MESSAGE};
    if connection.version < bareos_bnet::hello::BAREOS_VERSION_RELEASE_18_2 {
        let cipher = bareos_bnet::tls_psk::cipher_message_string(connection.tls_cipher.as_deref());
        return vec![format!("1000 OK: {director} Version: {FULL} ({DATE}) -- {cipher}\n")];
    }
    // snprintf(version_info, 100, ...): at most 99 bytes
    let mut version_info = format!("OK: {director} Version: {FULL} ({DATE})");
    if version_info.len() > 99 {
        let mut end = 99;
        while !version_info.is_char_boundary(end) { end -= 1; }
        version_info.truncate(end);
    }
    let who = if console_name == "*UserAgent*" {
        "connected using the default console".to_string()
    } else {
        format!("logged in as: {console_name}")
    };
    vec![format!("1000\x1e{version_info}"), format!("1002\x1e{SERVICES_MESSAGE}\nYou are {who}")]
}

/// Handle a console session (BNET mode).
pub fn handle_console_session_with<S: Read + Write>(
    mut bnet: BnetStream<S>,
    session_id: u64,
    console_name: &str,
    password: &str,
    connection: &ConsoleConnection,
    session_mgr: &Arc<SessionManager>,
    cmd_queue: &CommandQueue,
    config: &Option<SharedConfig>,
    job_manager: &Option<Arc<std::sync::Mutex<crate::job_manager::JobManager>>>,
    catalog: &Option<CatalogHandle>,
) {
    // Register session
    let mgr_session_id = session_mgr.create_session(console_name);
    let before_18_2 = connection.version < bareos_bnet::hello::BAREOS_VERSION_RELEASE_18_2;

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
                // SendNotAuthorizedMessageWithSpaceSeparators: consoles before 18.2 only
                if before_18_2 { let _ = bnet.send_str("1999 You are not authorized.\n"); }
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

    let director = config.as_ref().and_then(|c| c.read().ok().map(|c| c.director.name.clone()))
        .unwrap_or_else(|| "bareos-dir".to_string());
    for message in console_banner(&director, console_name, connection) {
        if bnet.send_str(&message).is_err() {
            session_mgr.remove_session(mgr_session_id);
            return;
        }
    }

    let mut state = SessionState::CommandLoop;
    let mut api_mode = command_executor::ApiMode::Human;

    // Session flags (set by gui, trace, .grpc commands; autodisplay and the
    // "You have messages." flag live in the SessionManager)
    let mut gui_mode = false;     // gui on/off
    let mut grpc_mode = false;    // .grpc on/off
    let mut json_compact = false; // .api json compact=yes|no (OutputFormatter::SetCompact)

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
            // C++ works on raw bytes; skipping invalid UTF-8 would leave the
            // console without a reply.
            Ok(BnetMessage::Data(data)) => String::from_utf8_lossy(&data).into_owned(),
            Ok(BnetMessage::Signal(BnetSignal::Terminate)) => {
                debug!("session {session_id}: client sent TERMINATE");
                break;
            }
            Ok(BnetMessage::Signal(_)) => {
                // C++ ua_server.cc: any other signal instead of a command
                // is answered with BNET_POLL; no command, no EOD.
                if bnet.send_signal(BnetSignal::Poll).is_err() { break; }
                continue;
            }
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
        debug!("session {session_id}: command '{cmd_str}'");

        // Parse command into AST
        let mut ast = parse_cli_command(cmd_str);
        ast.raw_line = msg.clone();
        let mut ctx = ExecContext {
            session_id: mgr_session_id,
            config: config.clone(),
            session_mgr: Some(session_mgr.clone()),
            catalog: catalog.clone(),
            api_mode,
            bus_rx: None,
            job_manager: job_manager.clone(),
        };
        // C++ Do_a_command(): no arguments (empty or whitespace-only line)
        // runs nothing, but the loop still handles messages and sends EOD.
        if ast.command_name.is_empty() {
            if !finish_command(&mut bnet, &ctx, gui_mode, api_mode) { break; }
            continue;
        }
        let typed = ast.command_name.clone();
        if let Some(name) = crate::command_registry::resolve_command_name(
            command_executor::get_registry(), &ast.command_name) {
            ast.command_name = name.to_string();
        }

        let command_allowed=matches!(ast.command_name.as_str(),"quit"|"exit"|"whoami")
            || crate::acl::access(&ctx,crate::acl::Kind::Command,&ast.command_name);

        // C++ ua_cmds.cc Do_a_command(): in API mode the command is framed by
        // CMD_BEGIN ... CMD_OK/CMD_FAILED; in JSON mode the json-rpc result
        // follows after that (FinalizeResult). `.api` switches the mode
        // before it produces output, so its own reply is already JSON.
        // An unknown or ACL-denied command is answered by an ErrorMsg only,
        // without the CMD_BEGIN ... CMD_OK/CMD_FAILED frame.
        let invalid = command_executor::is_invalid_command(&ast, &ctx);
        if !invalid {
            // Do_a_command(): audit before the command runs (ua->cmd is the line as received)
            crate::audit::command(&ctx, &ast.command_name, &typed, &msg);
        }
        if api_mode != command_executor::ApiMode::Human && !invalid {
            if bnet.send_signal(BnetSignal::CmdBegin).is_err() { break; }
        }
        if command_allowed && ast.command_name == ".api" {
            if let Some((mode, compact)) = crate::commands::dot::parse_dot_api(&ast) {
                ctx.api_mode = mode;
                json_compact = compact.unwrap_or(json_compact);
            }
        }

        // C++ AutomountCmd: without exactly one argument the value is asked
        // for (GetCmd); no answer (lone dot, lost connection) keeps the flag.
        if command_allowed && ast.command_name == "automount" && ast.arguments.len() != 1 {
            let answer = get_cmd(&mut bnet, "Turn on or off? ", api_mode);
            // ua_input.cc:66: an answer ".messages" first runs DotMessagesCmd
            if answer.as_deref() == Some(".messages") {
                crate::console_msg::dot_messages_cmd(&ctx, ".messages",
                    |messages| messages.iter().all(|m| bnet.send(m).is_ok()));
            }
            ast.arguments = answer
                .map(|answer| vec![Argument { key: answer, value: None }])
                .unwrap_or_default();
        }

        // Execute command; only successful authorization commits local changes.
        // Dialogs (ua_select) talk to this console while the command runs.
        let result = crate::ua_select::with_console(&mut bnet, || command_executor::dispatch(&ast, &ctx));

        // Session-altering commands
        if result.success {
        api_mode=ctx.api_mode;
        match ast.command_name.as_str() {
            // C++ AutodisplayCmd/gui_cmd: without on/off the flag stays.
            "autodisplay" => {
                if let Some(on) = crate::commands::extra::on_off_keyword(&ast) {
                    session_mgr.set_session_auto_display(mgr_session_id, on);
                }
            }
            "gui" => {
                if let Some(on) = crate::commands::extra::on_off_keyword(&ast) {
                    gui_mode = on;
                    session_mgr.set_session_gui(mgr_session_id, gui_mode);
                }
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

        // UaContext::ErrorMsg: in API mode every error message is preceded
        // by BNET_ERROR_MSG (in JSON mode only the signal goes out directly).
        if api_mode != command_executor::ApiMode::Human
            && result.error_lines().iter().any(|_| bnet.send_signal(BnetSignal::ErrorMsg).is_err()) {
            break;
        }
        if api_mode != command_executor::ApiMode::Human && !invalid {
            let sig = if result.success { BnetSignal::CmdOk } else { BnetSignal::CmdFailed };
            if bnet.send_signal(sig).is_err() { break; }
        }

        // Send result
        if api_mode == command_executor::ApiMode::Json {
            if bnet.send_str(&crate::json_output::envelope(&result, json_compact)).is_err() { break; }
        } else {
            let with_newline = |text: &str| if text.ends_with('\n') || result.verbatim { text.to_string() } else { format!("{text}\n") };
            let mut parts = [&result.text_output, &result.error_message];
            if result.errors_first { parts.reverse(); }
            if parts.iter().filter(|part| !part.is_empty())
                .any(|part| bnet.send_str(&with_newline(part)).is_err()) {
                break;
            }
        }

        // C++ ua_cmds.cc quit_cmd() only sets ua->quit; ua_server.cc then
        // skips HandleMessages and the EOD and closes the socket without
        // sending BNET_TERMINATE.
        if command_allowed && result.success
            && matches!(ast.command_name.as_str(), "quit" | "exit" | ".quit") {
            break;
        }

        if !finish_command(&mut bnet, &ctx, gui_mode, api_mode) { break; }
    }

    // Cleanup
    session_mgr.remove_session(mgr_session_id);
    info!("session {session_id} ({console_name}) ended");
}

/// Tail of one console loop iteration (C++ ua_server.cc
/// HandleUserAgentClientRequest after Do_a_command): pending messages, then
/// EOD in non-API mode. Also runs after an empty line. False: socket gone.
///
/// Pending console messages (the Director-wide queue, console_msg) are
/// handled only when the console may run `messages`: with autodisplay on
/// they are sent (DotMessagesCmd); otherwise a console outside gui mode is
/// told once — "You have messages." or, in API mode, BNET_MSGS_PENDING —
/// until it fetches them.
fn finish_command<S: Read + Write>(
    bnet: &mut BnetStream<S>,
    ctx: &ExecContext,
    gui_mode: bool,
    api_mode: command_executor::ApiMode,
) -> bool {
    let api = api_mode != command_executor::ApiMode::Human;
    if crate::console_msg::pending() && crate::acl::access(ctx, crate::acl::Kind::Command, "messages") {
        let manager = ctx.session_mgr.as_ref();
        if crate::console_msg::auto_display(ctx) {
            crate::console_msg::dot_messages_cmd(ctx, "messages",
                |messages| messages.iter().all(|m| bnet.send(m).is_ok()));
            if let Some(manager) = manager { manager.set_session_notified(ctx.session_id, false); }
        } else if !gui_mode && !manager.is_some_and(|m| m.session_notified(ctx.session_id)) {
            if api {
                let _ = bnet.send_signal(BnetSignal::MsgsPending);
            } else {
                let _ = bnet.send_str("You have messages.\n");
            }
            if let Some(manager) = manager { manager.set_session_notified(ctx.session_id, true); }
        }
    }

    // C++ ua_session.cc HandleMessages(): EOD only in non-API mode.
    // bconsole uses api=0, so it expects EOD after each command response.
    if api_mode == command_executor::ApiMode::Human {
        if bnet.send_signal(BnetSignal::Eod).is_err() { return false; }
    }
    true
}

/// Split a command line like C++ lib/scan.cc next_arg(): whitespace
/// separates arguments, double quotes group (and are removed), a backslash
/// takes the next character literally (`fileset="My Set"` -> `fileset=My Set`).
fn split_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // A trailing backslash ends the line (next_arg copies the NUL).
                if let Some(next) = chars.next() { cur.push(next); }
                has_token = true;
            }
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_ascii_whitespace() && !in_quotes => {
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

/// C++ ua_input.cc GetCmd(): ask the console for one line of input. JSON
/// mode is batch mode (DotApiCmd sets ua->batch) and never asks. `.api 1`
/// clients get BNET_TEXT_INPUT before the prompt, plain consoles
/// BNET_SUB_PROMPT after it. Signals are ignored; trailing whitespace is
/// stripped; a lone dot or a lost connection returns None.
fn get_cmd<S: Read + Write>(
    bnet: &mut BnetStream<S>,
    prompt: &str,
    api_mode: command_executor::ApiMode,
) -> Option<String> {
    use command_executor::ApiMode;
    match api_mode {
        ApiMode::Json => return None,
        ApiMode::Api => {
            bnet.send_signal(BnetSignal::TextInput).ok()?;
            bnet.send_str(prompt).ok()?;
        }
        ApiMode::Human => {
            bnet.send_str(prompt).ok()?;
            bnet.send_signal(BnetSignal::SubPrompt).ok()?;
        }
    }
    let answer = loop {
        match bnet.recv().ok()? {
            BnetMessage::Data(data) => break String::from_utf8_lossy(&data).trim_end().to_string(),
            BnetMessage::Signal(BnetSignal::Terminate) => return None,
            BnetMessage::Signal(_) => continue,
        }
    };
    (answer != ".").then_some(answer)
}

/// Parse a CLI command line like C++ ParseArgs(): the first argument is the
/// command, the rest are key=value arguments split at the first '='.
/// An empty line (no arguments) gives an empty command name.
pub(crate) fn parse_cli_command(input: &str) -> CommandAst {
    // ParseArgsOnly() stops at the first empty argument (e.g. `""`).
    let mut tokens = split_args(input).into_iter().take_while(|token| !token.is_empty())
        .map(|token| match token.split_once('=') {
            Some((key, value)) => Argument { key: key.to_string(), value: Some(value.to_string()) },
            None => Argument { key: token, value: None },
        });
    // Do_a_command() matches argk[0] (the part before '=') as a prefix of
    // the command table; an empty keyword (`=`, `=x`) therefore matches its
    // first entry, the no-op ".".
    let command_name = match tokens.next() {
        Some(first) if first.key.is_empty() => ".".to_string(),
        Some(first) => first.key,
        None => String::new(),
    };

    CommandAst {
        command_name,
        arguments: tokens.collect(),
        session_id: String::new(),
        source: CommandSource::Cli,
        response_format: ResponseFormat::Default,
        raw_line: input.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs get_cmd on the server side of a TCP pair while `client` answers.
    fn prompt_pair(
        api_mode: command_executor::ApiMode,
        client: impl FnOnce(&mut BnetStream<std::net::TcpStream>) + Send + 'static,
    ) -> Option<String> {
        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = std::thread::spawn(move || {
            let stream = TcpStream::connect(address).unwrap();
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            client(&mut BnetStream::new(stream));
        });
        let (stream, _) = listener.accept().unwrap();
        let answer = get_cmd(&mut BnetStream::new(stream), "Turn on or off? ", api_mode);
        peer.join().unwrap();
        answer
    }

    #[test]
    fn get_cmd_prompts_like_cpp() {
        use command_executor::ApiMode;
        let answer = prompt_pair(ApiMode::Human, |bnet| {
            assert_eq!(bnet.recv_str().unwrap(), "Turn on or off? ");
            assert_eq!(bnet.recv().unwrap(), BnetMessage::Signal(BnetSignal::SubPrompt));
            bnet.send_signal(BnetSignal::Eod).unwrap();
            bnet.send_str("OFF \n").unwrap();
        });
        assert_eq!(answer.as_deref(), Some("OFF"));
        let answer = prompt_pair(ApiMode::Api, |bnet| {
            assert_eq!(bnet.recv().unwrap(), BnetMessage::Signal(BnetSignal::TextInput));
            assert_eq!(bnet.recv_str().unwrap(), "Turn on or off? ");
            bnet.send_str(".").unwrap();
        });
        assert_eq!(answer, None, "lone dot cancels");
        // JSON mode is batch mode: no prompt at all.
        assert_eq!(prompt_pair(ApiMode::Json, |_| {}), None);
    }

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

    #[test]
    fn parse_blank_lines_have_no_command() {
        for line in ["   ", "\t", "\r\n", " \t \n", "\"\"", "\"\" version"] {
            let ast = parse_cli_command(line);
            assert_eq!(ast.command_name, "", "{line:?}");
            assert!(ast.arguments.is_empty(), "{line:?}");
        }
    }

    #[test]
    fn parse_command_token_like_parse_args() {
        // Leading/trailing whitespace and tabs separate like spaces.
        let ast = parse_cli_command("\t  show\tclients   \n");
        assert_eq!(ast.command_name, "show");
        assert_eq!(ast.arguments.len(), 1);
        assert_eq!(ast.arguments[0].key, "clients");
        // argk[0] is the part before '='; quotes are removed.
        assert_eq!(parse_cli_command("show=x clients").command_name, "show");
        assert_eq!(parse_cli_command("\"show\" clients").command_name, "show");
        // An empty keyword matches the first command table entry ".".
        assert_eq!(parse_cli_command("=").command_name, ".");
        let ast = parse_cli_command("=show clients");
        assert_eq!(ast.command_name, ".");
        assert_eq!(ast.arguments[0].key, "clients");
        // Comments are not special: the console sends them as commands.
        assert_eq!(parse_cli_command("# version").command_name, "#");
        assert_eq!(parse_cli_command("#comment").command_name, "#comment");
    }

    #[test]
    fn parse_stops_at_empty_argument_and_honours_backslash() {
        let ast = parse_cli_command("show \"\" clients");
        assert_eq!(ast.command_name, "show");
        assert!(ast.arguments.is_empty());
        let ast = parse_cli_command(r#"run comment=a\ b\"c"#);
        assert_eq!(ast.arguments[0].value.as_deref(), Some("a b\"c"));
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    /// C++ ua_server.cc: an empty line gets EOD only, a signal gets POLL.
    #[test]
    fn empty_lines_get_eod_and_signals_get_poll() {
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
        for line in ["", "   ", "\t", "\r\n"] {
            bnet.send_str(line).unwrap();
            assert_eq!(bnet.recv().unwrap(),BnetMessage::Signal(BnetSignal::Eod),"{line:?}");
        }
        bnet.send_signal(BnetSignal::Eod).unwrap();
        assert_eq!(bnet.recv().unwrap(),BnetMessage::Signal(BnetSignal::Poll));
        bnet.send_str("  VERSION  ").unwrap();
        assert!(bnet.recv_str().unwrap().contains(" Version: "));
        assert_eq!(bnet.recv().unwrap(),BnetMessage::Signal(BnetSignal::Eod));
        bnet.send_str("quit").unwrap();drop(bnet);server.join().unwrap();
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
        assert!(bnet.recv_str().unwrap().starts_with("dir Version: "));
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

#[cfg(test)]
mod banner_tests {
    //! C++ authenticate_console.cc: banner per console version.
    use super::*;
    use crate::version_strings::{DATE, FULL, SERVICES_MESSAGE};

    fn connection(version: u32, cipher: Option<&str>) -> ConsoleConnection {
        ConsoleConnection { version, tls_cipher: cipher.map(str::to_string) }
    }

    #[test]
    fn before_18_2_one_frame_with_cipher() {
        let cipher = Some("TLS_CHACHA20_POLY1305_SHA256 TLSv1.3");
        assert_eq!(console_banner("bareos-dir", "*UserAgent*", &connection(1, cipher)),
            vec![format!("1000 OK: bareos-dir Version: {FULL} ({DATE}) --  Encryption: TLS_CHACHA20_POLY1305_SHA256 TLSv1.3\n")]);
        assert_eq!(console_banner("d", "admin", &connection(1702, None)),
            vec![format!("1000 OK: d Version: {FULL} ({DATE}) --  Encryption: None\n")]);
    }

    #[test]
    fn from_18_2_ok_and_info_message() {
        assert_eq!(console_banner("bareos-dir", "*UserAgent*", &connection(1802, None)), vec![
            format!("1000\x1eOK: bareos-dir Version: {FULL} ({DATE})"),
            format!("1002\x1e{SERVICES_MESSAGE}\nYou are connected using the default console")]);
        assert_eq!(console_banner("bareos-dir", "admin", &connection(2501, None))[1],
            format!("1002\x1e{SERVICES_MESSAGE}\nYou are logged in as: admin"));
    }

    #[test]
    fn version_info_truncated_like_snprintf_100() {
        let banner = console_banner(&"d".repeat(120), "admin", &connection(2501, None));
        assert_eq!(banner[0].len(), "1000\x1e".len() + 99);
    }

    #[test]
    fn failed_auth_of_old_console_gets_not_authorized() {
        use std::net::{TcpListener, TcpStream};
        let manager = Arc::new(SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_console_session_with(BnetStream::new(stream), 1, "admin", "pw", &connection(1702, None),
                &manager, &CommandQueue::new(16), &None, &None, &None);
        });
        let stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        let mut bnet = BnetStream::new(stream);
        assert!(!CramMd5Auth::new("wrong", "admin", 0).two_way_authenticate(&mut bnet, false).unwrap().success);
        server.join().unwrap();
        let mut rest = Vec::new();
        while let Ok(BnetMessage::Data(data)) = bnet.recv() { rest.push(String::from_utf8(data).unwrap()); }
        assert_eq!(rest.last().map(String::as_str), Some("1999 You are not authorized.\n"));
    }
}

#[cfg(test)]
mod console_message_tests {
    //! C++ ua_server.cc HandleUserAgentClientRequest() after Do_a_command:
    //! the Director-wide console message queue (console_msg) is announced
    //! once ("You have messages." / BNET_MSGS_PENDING), shown directly with
    //! autodisplay on, and only for consoles that may run `messages`.
    use super::*;
    use crate::console_msg::ConsoleQueue;
    use std::net::{TcpListener, TcpStream};
    use std::sync::RwLock;
    use std::time::Duration;

    fn session(queue: &Arc<ConsoleQueue>, console: &str, acl: &str) -> (BnetStream<TcpStream>, std::thread::JoinHandle<()>) {
        let config = crate::director_config::DirectorConfig::from_str(&format!(
            "Director {{Name=dir; Password=secret}}\nConsole {{Name={console}; Password=pw; CommandACL={acl}}}"
        )).unwrap();
        let config: Option<SharedConfig> = Some(Arc::new(RwLock::new(config)));
        let manager = Arc::new(SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let name = console.to_string();
        let queue = queue.clone();
        let server = std::thread::spawn(move || {
            crate::console_msg::use_test_queue(queue);
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

    /// Frames up to and including `until`: data as text, signals as "<Name>".
    fn reply(bnet: &mut BnetStream<TcpStream>, until: BnetSignal) -> Vec<String> {
        let mut frames = Vec::new();
        loop {
            match bnet.recv().unwrap() {
                BnetMessage::Data(data) => frames.push(String::from_utf8(data).unwrap()),
                BnetMessage::Signal(signal) => {
                    frames.push(format!("<{signal:?}>"));
                    if signal == until { return frames; }
                }
            }
        }
    }

    fn command(bnet: &mut BnetStream<TcpStream>, line: &str, until: BnetSignal) -> Vec<String> {
        bnet.send_str(line).unwrap();
        reply(bnet, until)
    }

    fn close(mut bnet: BnetStream<TcpStream>, server: std::thread::JoinHandle<()>) {
        bnet.send_str("quit").unwrap();
        drop(bnet);
        server.join().unwrap();
    }

    #[test]
    fn announced_once_until_fetched_in_order() {
        let queue = crate::console_msg::test_queue("announce");
        let (mut bnet, server) = session(&queue, "admin", "*all*");
        assert_eq!(command(&mut bnet, "", BnetSignal::Eod), ["<Eod>"]);
        queue.append("16-Oct 10:00 ", "dir JobId 1: first\n");
        queue.append("16-Oct 10:01 ", "dir JobId 2: second");
        assert_eq!(command(&mut bnet, "", BnetSignal::Eod), ["You have messages.\n", "<Eod>"]);
        assert_eq!(command(&mut bnet, "", BnetSignal::Eod), ["<Eod>"], "told only once");
        assert_eq!(command(&mut bnet, "messages", BnetSignal::Eod),
            ["16-Oct 10:00 dir JobId 1: first\n", "16-Oct 10:01 dir JobId 2: second\n", "<Eod>"]);
        assert_eq!(command(&mut bnet, "messages", BnetSignal::Eod), ["You have no messages.\n", "<Eod>"]);
        assert_eq!(command(&mut bnet, ".messages", BnetSignal::Eod), ["<Eod>"]);
        queue.append("", "third\n");
        assert_eq!(command(&mut bnet, "", BnetSignal::Eod), ["You have messages.\n", "<Eod>"], "told again after fetching");
        assert_eq!(command(&mut bnet, ".messages", BnetSignal::Eod), ["third\n", "<Eod>"],
            ".messages fetches without autodisplay");
        close(bnet, server);
    }

    #[test]
    fn autodisplay_shows_messages_after_the_command() {
        let queue = crate::console_msg::test_queue("autodisplay");
        let (mut bnet, server) = session(&queue, "admin", "*all*");
        assert_eq!(command(&mut bnet, "autodisplay on", BnetSignal::Eod), ["<Eod>"]);
        queue.append("", "first\n");
        assert_eq!(command(&mut bnet, "", BnetSignal::Eod), ["first\n", "<Eod>"]);
        assert!(!queue.pending());
        // GetCmd: an answer ".messages" shows them, then counts as the answer
        assert_eq!(command(&mut bnet, "automount", BnetSignal::SubPrompt), ["Turn on or off? ", "<SubPrompt>"]);
        queue.append("", "second\n");
        assert_eq!(command(&mut bnet, ".messages", BnetSignal::Eod), ["second\n", "<Eod>"]);
        close(bnet, server);
    }

    #[test]
    fn gui_mode_and_command_acl_suppress_the_announcement() {
        let queue = crate::console_msg::test_queue("gui");
        let (mut bnet, server) = session(&queue, "admin", "*all*");
        assert_eq!(command(&mut bnet, "gui on", BnetSignal::Eod), ["<Eod>"]);
        queue.append("", "first\n");
        assert_eq!(command(&mut bnet, "", BnetSignal::Eod), ["<Eod>"]);
        assert_eq!(command(&mut bnet, "gui off", BnetSignal::Eod), ["You have messages.\n", "<Eod>"]);
        close(bnet, server);

        let queue = crate::console_msg::test_queue("acl");
        let (mut bnet, server) = session(&queue, "limited", "version, autodisplay");
        queue.append("", "first\n");
        assert_eq!(command(&mut bnet, "", BnetSignal::Eod), ["<Eod>"]);
        assert_eq!(command(&mut bnet, "autodisplay on", BnetSignal::Eod), ["<Eod>"]);
        assert!(queue.pending(), "not shown without the messages command");
        close(bnet, server);
    }

    #[test]
    fn api_mode_signals_msgs_pending_once() {
        let queue = crate::console_msg::test_queue("api");
        let (mut bnet, server) = session(&queue, "admin", "*all*");
        command(&mut bnet, ".api 1", BnetSignal::MainPrompt);
        queue.append("", "first\n");
        assert_eq!(command(&mut bnet, "", BnetSignal::MainPrompt), ["<MsgsPending>", "<MainPrompt>"]);
        assert_eq!(command(&mut bnet, "", BnetSignal::MainPrompt), ["<MainPrompt>"]);
        assert_eq!(command(&mut bnet, "messages", BnetSignal::MainPrompt),
            ["<CmdBegin>", "first\n", "<CmdOk>", "<MainPrompt>"]);
        close(bnet, server);
    }
}
