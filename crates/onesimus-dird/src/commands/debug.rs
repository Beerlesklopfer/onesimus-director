//! setdebug: Director, client and storage targets using Bareos control packets.
use crate::{command_ast::CommandAst, command_executor::ExecContext, command_queue::CommandResult};
use bareos_config::resources::{ClientResource, StorageResource};

#[derive(Clone, Copy)]
struct Options {
    level: u32,
    trace: i32,
    hangup: i32,
    timestamp: i32,
}

impl Options {
    fn client_command(self, version: u32) -> String {
        let mut command = format!(
            "setdebug={} trace={} hangup={}",
            self.level, self.trace, self.hangup
        );
        if version >= 53 {
            command.push_str(&format!(" timestamp={}", self.timestamp));
        }
        command.push('\n');
        command
    }
    fn storage_command(self) -> String {
        format!(
            "setdebug={} trace={} timestamp={}\n",
            self.level, self.trace, self.timestamp
        )
    }
}

pub fn cmd_setdebug(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    setdebug_dialog_with(
        ast,
        ctx,
        |client, director, options| {
            crate::fd_protocol::fd_command(
                &client.address,
                client.port,
                &client.password,
                director,
                |version| options.client_command(version),
            )
        },
        |storage, director, options| {
            bareos_stored::command::sd_control_command(
                &storage.address,
                storage.port,
                &storage.password,
                director,
                &options.storage_command(),
            )
        },
    )
}

/// C++ atoi(): optional sign and leading digits, 0 otherwise.
fn atoi(text: &str) -> i64 {
    let text = text.trim_start();
    let (sign, digits) = match text.as_bytes().first() {
        Some(b'-') => (-1, &text[1..]),
        Some(b'+') => (1, &text[1..]),
        _ => (1, text),
    };
    let end = digits.bytes().take_while(u8::is_ascii_digit).count();
    sign * digits[..end].parse::<i64>().unwrap_or(0)
}

/// Daemon chosen by the SetdebugCmd dialog.
enum DebugTarget {
    Director,
    All,
    Client(String),
    Storage(String),
}

/// C++ ua_cmds.cc SetdebugCmd() argument scan and dialogs: the level is
/// asked for when missing or negative (GetPint "Enter new debug level: "),
/// a client/storage keyword without a known resource opens the selection
/// list, and without any daemon keyword the daemon type is asked for
/// ("Available daemons are: "). The resolved request then goes to
/// setdebug_with. Cancelled dialogs end the command silently (C++ returns
/// true without output).
fn setdebug_dialog_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    fd: impl FnMut(&ClientResource, &str, Options) -> Result<String, String>,
    sd: impl FnMut(&StorageResource, &str, Options) -> Result<String, String>,
) -> CommandResult {
    use crate::ua_select::{self, Ua};
    let mut ua = Ua::new(ctx);
    let mut level = ua_select::find_arg_with_value(ast, "level").map(atoi).unwrap_or(-1);
    if level < 0 {
        match ua.get_pint("Enter new debug level: ") {
            Some(value) => level = value as i64,
            None => return ua.finish(true),
        }
    }
    let exists = |storage: bool, name: &str| {
        ctx.config.as_ref().and_then(|c| c.read().ok()).is_some_and(|c| {
            if storage { c.find_storage(name).is_some() } else { c.find_client(name).is_some() }
        })
    };
    let mut target = None;
    for arg in &ast.arguments {
        let key = arg.key.to_ascii_lowercase();
        match key.as_str() {
            "all" => target = Some(DebugTarget::All),
            "dir" | "director" => target = Some(DebugTarget::Director),
            "client" | "fd" => {
                target = match arg.value.as_deref().filter(|name| exists(false, name)) {
                    Some(name) => Some(DebugTarget::Client(name.to_string())),
                    None => ua_select::select_client_resource(&mut ua).map(DebugTarget::Client),
                };
            }
            "store" | "storage" | "sd" => {
                target = match arg.value.as_deref().filter(|name| exists(true, name)) {
                    Some(name) => Some(DebugTarget::Storage(name.to_string())),
                    None => get_storage_resource(&mut ua, ast).map(DebugTarget::Storage),
                };
            }
            _ => {}
        }
        if target.is_some() {
            break;
        }
    }
    if target.is_none() {
        ua.start_prompt("Available daemons are: \n");
        for daemon in ["Director", "Storage", "Client", "All"] {
            ua.add_prompt(daemon);
        }
        target = match ua.do_prompt("", "Select daemon type to set debug level") {
            Some((0, _)) => Some(DebugTarget::Director),
            Some((1, _)) => get_storage_resource(&mut ua, ast).map(DebugTarget::Storage),
            Some((2, _)) => ua_select::select_client_resource(&mut ua).map(DebugTarget::Client),
            Some((3, _)) => Some(DebugTarget::All),
            _ => None,
        };
    }
    let Some(target) = target else { return ua.finish(true) };

    let mut resolved = CommandAst { command_name: ast.command_name.clone(), ..Default::default() };
    let argument = |key: &str, value: Option<String>| crate::command_ast::Argument { key: key.to_string(), value };
    resolved.arguments.push(argument("level", Some(level.to_string())));
    for key in ["trace", "hangup", "timestamp"] {
        if let Some(value) = ua_select::find_arg_with_value(ast, key) {
            resolved.arguments.push(argument(key, Some(atoi(value).to_string())));
        }
    }
    resolved.arguments.push(match target {
        DebugTarget::Director => argument("dir", None),
        DebugTarget::All => argument("all", None),
        DebugTarget::Client(name) => argument("client", Some(name)),
        DebugTarget::Storage(name) => argument("storage", Some(name)),
    });
    let result = setdebug_with(&resolved, ctx, fd, sd);
    let success = result.success;
    ua.send_result(result);
    ua.finish(success)
}

/// C++ get_storage_resource(ua) as SetdebugCmd uses it (no default
/// keyword): `storage=`/`sd=` names a resource, otherwise the selection
/// list. (Job based selection jobid=/job=/ujobid= is not supported here.)
fn get_storage_resource(ua: &mut crate::ua_select::Ua, ast: &CommandAst) -> Option<String> {
    let name = ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case("storage") || a.key.eq_ignore_ascii_case("sd"))
        .and_then(|a| a.value.clone());
    if let Some(name) = name.filter(|n| !n.is_empty()) {
        let found = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
            .is_some_and(|c| c.find_storage(&name).is_some());
        if found && crate::acl::access(ua.ctx, crate::acl::Kind::Storage, &name) {
            return Some(name);
        }
        if !found {
            ua.error_msg(format!("Storage resource \"{name}\": not found\n"));
        }
    }
    crate::ua_select::select_storage_resource(ua, false)
}

fn setdebug_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    mut fd: impl FnMut(&ClientResource, &str, Options) -> Result<String, String>,
    mut sd: impl FnMut(&StorageResource, &str, Options) -> Result<String, String>,
) -> CommandResult {
    let value = |key: &str| {
        ast.arguments
            .iter()
            .find(|a| a.key.eq_ignore_ascii_case(key))
            .and_then(|a| a.value.as_deref())
    };
    let level = match value("level")
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|level| *level <= i32::MAX as u32)
    {
        Some(level) => level,
        None => {
            return CommandResult::error(
                "setdebug: level=<nonnegative integer> is required\n".into(),
            );
        }
    };
    let flag = |key: &str| -> Result<i32, String> {
        match value(key) {
            None => Ok(-1),
            Some(s) => s
                .parse::<i32>()
                .map(|n| n.signum())
                .map_err(|_| format!("setdebug: invalid {key}\n")),
        }
    };
    let options = match (flag("trace"), flag("hangup"), flag("timestamp")) {
        (Ok(trace), Ok(hangup), Ok(timestamp)) => Options {
            level,
            trace,
            hangup,
            timestamp,
        },
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
            return CommandResult::error(error);
        }
    };
    let targets: Vec<_> = ast
        .arguments
        .iter()
        .filter(|a| {
            matches!(
                a.key.to_ascii_lowercase().as_str(),
                "dir" | "director" | "client" | "fd" | "storage" | "store" | "sd" | "all"
            )
        })
        .collect();
    for argument in &ast.arguments {
        if !matches!(
            argument.key.to_ascii_lowercase().as_str(),
            "level"
                | "trace"
                | "hangup"
                | "timestamp"
                | "dir"
                | "director"
                | "client"
                | "fd"
                | "storage"
                | "store"
                | "sd"
                | "all"
        ) {
            return CommandResult::error(format!("setdebug: unknown argument {}\n", argument.key));
        }
    }
    if targets.len() != 1 {
        return CommandResult::error(
            "setdebug: specify exactly one of dir, client=<name>, storage=<name>, all\n".into(),
        );
    }
    let target = targets[0];
    let kind = target.key.to_ascii_lowercase();
    let all = kind == "all";
    if (all || matches!(kind.as_str(), "dir" | "director")) && target.value.is_some() {
        return CommandResult::error("setdebug: dir/all does not take a value\n".into());
    }
    let mut clients = vec![];
    let mut storages = vec![];
    let director;
    if all || !matches!(kind.as_str(), "dir" | "director") {
        let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else {
            return CommandResult::error("No config\n".into());
        };
        director = config.director.name.clone();
        if all {
            clients.extend(config.clients.iter().cloned());
            storages.extend(config.storages.iter().cloned());
        } else {
            let Some(name) = target.value.as_deref().filter(|n| !n.is_empty()) else {
                return CommandResult::error("setdebug: target resource name missing\n".into());
            };
            match kind.as_str() {
                "client" | "fd" => match config.find_client(name) {
                    Some(client) => clients.push(client.clone()),
                    None => return CommandResult::error(format!("Client \"{name}\" not found.\n")),
                },
                _ => match config.find_storage(name) {
                    Some(storage) => storages.push(storage.clone()),
                    None => {
                        return CommandResult::error(format!("Storage \"{name}\" not found.\n"));
                    }
                },
            }
        }
    } else {
        director = String::new();
    }

    // Original AllStorageSetdebug/AllClientSetdebug enumerate resources without
    // resource ACL checks. CommandACL is enforced by the executor. Explicit
    // targets use Get*ResWithName and must pass their resource ACL.
    if all {
        let mut seen = std::collections::HashSet::new();
        clients.retain(|client| seen.insert((client.address.clone(), client.port)));
        seen.clear();
        storages.retain(|storage| seen.insert((storage.address.clone(), storage.port)));
    } else {
        for client in &clients {
            if !crate::acl::access(ctx, crate::acl::Kind::Client, &client.name) {
                return CommandResult::error(format!("Client \"{}\" not found.\n", client.name));
            }
        }
        for storage in &storages {
            if !crate::acl::access(ctx, crate::acl::Kind::Storage, &storage.name) {
                return CommandResult::error(format!("Storage \"{}\" not found.\n", storage.name));
            }
        }
    }

    let mut output = String::new();
    if all || matches!(kind.as_str(), "dir" | "director") {
        let path = super::extra::trace_path(ctx);
        if options.trace >= 0 {
            if let Err(error) = bareos_bnet::debug::set_trace(&path, options.trace > 0) {
                return CommandResult::error(format!("setdebug: {}: {error}\n", path.display()));
            }
        }
        if let Err(error) = crate::logging::set_level(level) {
            return CommandResult::error(format!("setdebug: {error}\n"));
        }
        if options.timestamp >= 0 {
            bareos_bnet::debug::set_debug_timestamps(options.timestamp > 0);
        }
        output.push_str(&format!(
            "level={} trace={} hangup=0 timestamp={} tracefilename={}\n",
            level,
            bareos_bnet::debug::trace_enabled() as u8,
            bareos_bnet::debug::debug_timestamps() as u8,
            path.display()
        ));
    }
    let mut errors = String::new();
    for storage in storages {
        if !all && !crate::acl::access(ctx, crate::acl::Kind::Storage, &storage.name) {
            errors.push_str(&format!("Storage \"{}\" not found.\n", storage.name));
            continue;
        }
        output.push_str(&format!(
            "Connecting to Storage daemon {} at {}:{}\n",
            storage.name, storage.address, storage.port
        ));
        match sd(&storage, &director, options) {
            Ok(reply) => output.push_str(&reply),
            Err(error) => {
                errors.push_str(&format!("Failed to connect to Storage daemon.\n{error}\n"))
            }
        }
    }
    for client in clients {
        if !all && !crate::acl::access(ctx, crate::acl::Kind::Client, &client.name) {
            errors.push_str(&format!("Client \"{}\" not found.\n", client.name));
            continue;
        }
        output.push_str(&format!(
            "Connecting to Client {} at {}:{}\n",
            client.name, client.address, client.port
        ));
        match fd(&client, &director, options) {
            Ok(reply) => output.push_str(&reply),
            Err(error) => errors.push_str(&format!("Failed to connect to Client.\n{error}\n")),
        }
    }
    if errors.is_empty() {
        CommandResult::ok(output)
    } else {
        let mut result = CommandResult::error(errors);
        result.text_output = output;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};
    fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nClient { Name = fd; Address = localhost }\nClient { Name = duplicate; Address = localhost }\nStorage { Name = sd; Address = localhost }"
        ).unwrap();
        ExecContext {
            session_id: 0,
            config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None,
            catalog: None,
            api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None,
            job_manager: None,
        }
    }
    fn restricted(authenticated: bool) -> ExecContext {
        let mut ctx = context();
        let extra=bareos_config::raw::parse_str("Console {Name=restricted; Password=x; CommandACL=setdebug,setbandwidth; ClientACL=duplicate; StorageACL=none}",None).unwrap();
        ctx.config
            .as_ref()
            .unwrap()
            .write()
            .unwrap()
            .raw
            .extend(extra);
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16),
        )));
        ctx.session_id = manager.create_session("restricted");
        if authenticated {
            manager.authenticate_session(ctx.session_id);
        }
        ctx.session_mgr = Some(manager);
        ctx
    }
    #[test]
    fn explicit_targets_and_aliases_obey_authenticated_resource_acl() {
        for authenticated in [true, false] {
            let ctx = restricted(authenticated);
            for key in ["client", "fd", "storage", "store", "sd"] {
                let name = if ["client", "fd"].contains(&key) {
                    "fd"
                } else {
                    "sd"
                };
                let ast =
                    crate::session::parse_cli_command(&format!("setdebug level=1 {key}={name}"));
                let r = setdebug_with(
                    &ast,
                    &ctx,
                    |_, _, _| panic!("denied FD"),
                    |_, _, _| panic!("denied SD"),
                );
                assert!(!r.success, "{key}");
                assert!(r.text_output.is_empty());
            }
        }
        let r = setdebug_with(
            &crate::session::parse_cli_command("setdebug level=1 fd=duplicate"),
            &restricted(true),
            |client, _, _| {
                assert_eq!(client.name, "duplicate");
                Ok("2000 OK setdebug\n".into())
            },
            |_, _, _| panic!("not SD"),
        );
        assert!(r.success);
    }
    #[test]
    fn original_all_special_case_uses_command_acl_not_resource_acl() {
        let ctx = restricted(true);
        assert!(crate::acl::access(
            &ctx,
            crate::acl::Kind::Command,
            "setdebug"
        ));
        assert!(!crate::acl::access(&ctx, crate::acl::Kind::Client, "fd"));
        assert!(!crate::acl::access(&ctx, crate::acl::Kind::Storage, "sd"));
        let calls = std::cell::RefCell::new(Vec::new());
        let original = bareos_bnet::debug::debug_level();
        let r = setdebug_with(
            &crate::session::parse_cli_command("setdebug level=1 all"),
            &ctx,
            |client, _, _| {
                calls.borrow_mut().push(client.name.clone());
                Ok("2000 OK setdebug\n".into())
            },
            |storage, _, _| {
                calls.borrow_mut().push(storage.name.clone());
                Ok("3000 OK setdebug\n".into())
            },
        );
        bareos_bnet::debug::set_debug_level(original);
        assert!(r.success);
        assert_eq!(*calls.borrow(), vec!["sd", "fd"]);
    }
    #[test]
    fn uses_original_fd_protocol_versions_and_sd_command() {
        let options = Options {
            level: 100,
            trace: 1,
            hangup: -1,
            timestamp: 0,
        };
        assert_eq!(
            options.client_command(52),
            "setdebug=100 trace=1 hangup=-1\n"
        );
        assert_eq!(
            options.client_command(53),
            "setdebug=100 trace=1 hangup=-1 timestamp=0\n"
        );
        assert_eq!(
            options.storage_command(),
            "setdebug=100 trace=1 timestamp=0\n"
        );
        let result = setdebug_with(
            &crate::session::parse_cli_command("setdebug client=fd level=100 trace=1 timestamp=0"),
            &context(),
            |client, director, actual| {
                assert_eq!(client.name, "fd");
                assert_eq!(director, "dir");
                assert_eq!(actual.client_command(53), options.client_command(53));
                Ok("2000 OK setdebug\n".into())
            },
            |_, _, _| panic!("must not contact SD"),
        );
        assert_eq!(
            result.text_output,
            "Connecting to Client fd at localhost:9102\n2000 OK setdebug\n"
        );
    }
    #[test]
    fn validates_requests_before_any_remote_effect() {
        for line in [
            "setdebug client=fd",
            "setdebug level=bad dir",
            "setdebug level=1 trace=bad dir",
            "setdebug level=1 client=missing",
            "setdebug level=1 client",
            "setdebug level=1 dir all",
        ] {
            let result = setdebug_with(
                &crate::session::parse_cli_command(line),
                &context(),
                |_, _, _| panic!("must not contact FD"),
                |_, _, _| panic!("must not contact SD"),
            );
            assert!(!result.success, "{line}");
        }
    }
    #[test]
    fn all_deduplicates_daemons_and_does_not_hide_remote_errors() {
        let count = std::cell::Cell::new(0);
        let original = bareos_bnet::debug::debug_level();
        let result = setdebug_with(
            &crate::session::parse_cli_command("setdebug level=1 all"),
            &context(),
            |_, _, _| {
                count.set(count.get() + 1);
                Err("offline".into())
            },
            |_, _, _| {
                count.set(count.get() + 1);
                Ok("3000 OK setdebug\n".into())
            },
        );
        bareos_bnet::debug::set_debug_level(original);
        assert_eq!(count.get(), 2);
        assert!(!result.success);
        assert!(result.error_message.contains("offline"));
        assert!(result.text_output.contains("3000 OK setdebug\n"));
    }

    #[test]
    fn dialog_asks_level_and_daemon_type() {
        use crate::ua_select::tests::scripted;
        let ctx = context();
        // Batch mode: no level, nothing happens.
        let r = setdebug_dialog_with(&crate::session::parse_cli_command("setdebug"), &ctx,
            |_, _, _| panic!("no FD"), |_, _, _| panic!("no SD"));
        assert!(r.success && r.text_output.is_empty() && r.error_message.is_empty());
        // Cancel at the level question.
        let (r, console) = scripted(&["."], || setdebug_dialog_with(
            &crate::session::parse_cli_command("setdebug"), &ctx,
            |_, _, _| panic!("no FD"), |_, _, _| panic!("no SD")));
        assert!(r.success);
        assert_eq!(console.transcript(), "Enter new debug level: .\n");
        // Level given, daemon type asked, client list (two clients), cancel.
        let (r, console) = scripted(&["3", "."], || setdebug_dialog_with(
            &crate::session::parse_cli_command("setdebug level=5"), &ctx,
            |_, _, _| panic!("no FD"), |_, _, _| panic!("no SD")));
        assert!(r.success);
        assert_eq!(console.transcript(),
            "Available daemons are: \n1: Director\n2: Storage\n3: Client\n4: All\n\
             Select daemon type to set debug level (1-4): 3\n\
             The defined Client resources are:\n1: duplicate\n2: fd\n\
             Select Client (File daemon) resource (1-2): .\nSelection aborted, nothing done.\n");
        // storage keyword without value: single storage selected automatically.
        let (r, _) = scripted(&[], || setdebug_dialog_with(
            &crate::session::parse_cli_command("setdebug level=2 storage"), &ctx,
            |_, _, _| panic!("no FD"), |storage, _, options| {
                assert_eq!((storage.name.as_str(), options.level), ("sd", 2));
                Ok("3000 OK setdebug\n".into())
            }));
        assert!(r.success);
        assert_eq!(r.text_output, "Automatically selected Storage: sd\nConnecting to Storage daemon sd at localhost:9103\n3000 OK setdebug\n");
        assert_eq!(atoi("12abc"), 12);
        assert_eq!(atoi("x"), 0);
        assert_eq!(atoi("-3"), -3);
    }
}
