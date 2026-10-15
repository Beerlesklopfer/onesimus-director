//! setbandwidth: apply daemon-wide limits, expressed in KiB/s at the console.
use crate::{command_ast::CommandAst, command_executor::ExecContext, command_queue::CommandResult};
use bareos_config::resources::{ClientResource, StorageResource};

enum Target {
    Client(ClientResource),
    Storage(StorageResource),
}

pub fn cmd_setbandwidth(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    setbandwidth_dialog_with(ast, ctx, |target, director, command| match target {
        Target::Client(client) => crate::fd_protocol::fd_command(
            &client.address,
            client.port,
            &client.password,
            director,
            |_| command.to_string(),
        ),
        Target::Storage(storage) => bareos_stored::command::sd_control_command(
            &storage.address,
            storage.port,
            &storage.password,
            director,
            command,
        ),
    })
}

/// C++ ua_cmds.cc SetbwlimitCmd() dialogs: without `limit=` the limit is
/// asked for (GetPint "Enter new bandwidth limit kb/s: "); without a job
/// selector or `storage` keyword the client comes from get_client_resource
/// (`client=`/`fd=` or the selection list). A cancelled dialog ends the
/// command silently. Job selectors and `storage` go to setbandwidth_with
/// unchanged.
fn setbandwidth_dialog_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    send: impl FnOnce(&Target, &str, &str) -> Result<String, String>,
) -> CommandResult {
    use crate::command_ast::Argument;
    use crate::ua_select::{self, Ua};
    let mut ua = Ua::new(ctx);
    let mut resolved = ast.clone();
    // C++: limit = atoi(value) * 1024; a negative or missing limit is asked.
    let given = ua_select::find_arg_with_value(ast, "limit")
        .map(|v| v.trim().parse::<i64>().unwrap_or(-1));
    if given.is_none_or(|v| v < 0) {
        let Some(limit) = ua.get_pint("Enter new bandwidth limit kb/s: ") else { return ua.finish(true) };
        resolved.arguments.retain(|a| !a.key.eq_ignore_ascii_case("limit"));
        resolved.arguments.push(Argument { key: "limit".into(), value: Some(limit.to_string()) });
    }
    let job_selector = ["job", "jobid", "ujobid", "all", "state"].iter()
        .any(|k| ua_select::find_arg(ast, k).is_some());
    if !job_selector && ua_select::find_arg(ast, "storage").is_none() {
        let Some(client) = ua_select::get_client_resource(&mut ua, ast) else { return ua.finish(true) };
        resolved.arguments.retain(|a| !a.key.eq_ignore_ascii_case("client") && !a.key.eq_ignore_ascii_case("fd"));
        resolved.arguments.push(Argument { key: "client".into(), value: Some(client) });
    }
    let result = setbandwidth_with(&resolved, ctx, send);
    let success = result.success;
    ua.send_result(result);
    ua.finish(success)
}

fn setbandwidth_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    send: impl FnOnce(&Target, &str, &str) -> Result<String, String>,
) -> CommandResult {
    for argument in &ast.arguments {
        if !matches!(
            argument.key.to_ascii_lowercase().as_str(),
            "client" | "storage" | "limit" | "yes"
        ) {
            return CommandResult::error(format!(
                "setbandwidth: unsupported selector {}; use client=<name> or storage=<name>\n",
                argument.key
            ));
        }
    }
    let value = |key: &str| {
        ast.arguments
            .iter()
            .find(|a| a.key.eq_ignore_ascii_case(key))
            .and_then(|a| a.value.as_deref())
    };
    let Some(limit) = value("limit")
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 0)
    else {
        return CommandResult::error(
            "setbandwidth: limit=<nonnegative KiB/s> is required\n".into(),
        );
    };
    let Some(bytes) = limit.checked_mul(1024) else {
        return CommandResult::error("setbandwidth: limit is too large\n".into());
    };
    let selectors: Vec<_> = ast
        .arguments
        .iter()
        .filter(|a| a.key.eq_ignore_ascii_case("client") || a.key.eq_ignore_ascii_case("storage"))
        .collect();
    if selectors.len() != 1 || selectors[0].value.as_deref().is_none_or(str::is_empty) {
        return CommandResult::error(
            "setbandwidth: specify exactly one client=<name> or storage=<name>\n".into(),
        );
    }
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else {
        return CommandResult::error("No config\n".into());
    };
    let name = selectors[0].value.as_deref().unwrap();
    let target = if selectors[0].key.eq_ignore_ascii_case("client") {
        match config.find_client(name) {
            Some(client) => Target::Client(client.clone()),
            None => return CommandResult::error(format!("Client \"{name}\" not found.\n")),
        }
    } else {
        match config.find_storage(name) {
            Some(storage) => Target::Storage(storage.clone()),
            None => return CommandResult::error(format!("Storage \"{name}\" not found.\n")),
        }
    };
    let director = config.director.name.clone();
    drop(config);
    let (kind, resource, label) = match &target {
        Target::Client(client) => (crate::acl::Kind::Client, client.name.as_str(), "Client"),
        Target::Storage(storage) => (crate::acl::Kind::Storage, storage.name.as_str(), "Storage"),
    };
    if !crate::acl::access(ctx, kind, resource) {
        return CommandResult::error(format!("{label} \"{resource}\" not found.\n"));
    }
    let banner = match &target {
        Target::Client(client) => format!(
            "Connecting to Client {} at {}:{}\n",
            client.name, client.address, client.port
        ),
        Target::Storage(storage) => format!(
            "Connecting to Storage daemon {} at {}:{}\n",
            storage.name, storage.address, storage.port
        ),
    };
    // Job= is intentionally empty for a daemon-wide limit; job selectors are
    // never silently widened into a daemon-wide operation.
    let command = format!("setbandwidth={bytes} Job=\n");
    match send(&target, &director, &command) {
        Ok(reply) if reply.trim_end() == "2000 OK Bandwidth" => {
            CommandResult::ok(format!("{banner}OK Limiting bandwidth to {limit}kb/s \n"))
        }
        Ok(reply) => {
            let mut result =
                CommandResult::error(format!("Failed to set bandwidth limit: {reply}"));
            result.text_output = banner;
            result
        }
        Err(error) => {
            let mut result =
                CommandResult::error(format!("Failed to set bandwidth limit: {error}\n"));
            result.text_output = banner;
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};
    fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nClient { Name = fd; Address = localhost }\nStorage { Name = sd; Address = localhost }"
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
    fn resource_acl_denies_authenticated_and_unknown_sessions_before_network() {
        for authenticated in [true, false] {
            let ctx = restricted(authenticated);
            for line in [
                "setbandwidth client=fd limit=1",
                "setbandwidth storage=sd limit=1",
            ] {
                let r =
                    setbandwidth_with(&crate::session::parse_cli_command(line), &ctx, |_, _, _| {
                        panic!("ACL must prevent I/O")
                    });
                assert!(!r.success, "{line}");
                assert!(r.text_output.is_empty());
                assert!(r.error_message.contains("not found"));
            }
        }
    }
    #[test]
    fn authenticated_allowed_resource_reaches_only_its_transport() {
        let ctx = restricted(true);
        ctx.config.as_ref().unwrap().write().unwrap().raw = bareos_config::raw::parse_str(
            "Console {Name=restricted; Password=x; ClientACL=fd; StorageACL=none}",
            None,
        )
        .unwrap();
        let r = setbandwidth_with(
            &crate::session::parse_cli_command("setbandwidth client=fd limit=2"),
            &ctx,
            |target, _, command| {
                assert!(matches!(target, Target::Client(_)));
                assert_eq!(command, "setbandwidth=2048 Job=\n");
                Ok("2000 OK Bandwidth\n".into())
            },
        );
        assert!(r.success);
    }
    #[test]
    fn applies_scaled_limit_and_zero_unlimited_to_the_selected_daemon() {
        for (line, expected) in [
            (
                "setbandwidth client=fd limit=12",
                "setbandwidth=12288 Job=\n",
            ),
            ("setbandwidth storage=sd limit=0", "setbandwidth=0 Job=\n"),
        ] {
            let result = setbandwidth_with(
                &crate::session::parse_cli_command(line),
                &context(),
                |target, director, command| {
                    assert_eq!(director, "dir");
                    assert_eq!(command, expected);
                    assert_eq!(
                        matches!(target, Target::Client(_)),
                        line.contains("client=")
                    );
                    Ok("2000 OK Bandwidth\n".into())
                },
            );
            assert!(result.success, "{line}");
            assert!(result.text_output.contains("OK Limiting bandwidth"));
        }
    }
    #[test]
    fn invalid_requests_and_job_selectors_cannot_change_daemon_wide_limits() {
        for line in [
            "setbandwidth client=fd",
            "setbandwidth client=fd limit=-1",
            "setbandwidth client=fd limit=9223372036854775807",
            "setbandwidth limit=1 client=fd storage=sd",
            "setbandwidth limit=1 jobid=42",
            "setbandwidth limit=1 all",
            "setbandwidth client=missing limit=1",
        ] {
            assert!(
                !setbandwidth_with(
                    &crate::session::parse_cli_command(line),
                    &context(),
                    |_, _, _| panic!("no I/O permitted")
                )
                .success,
                "{line}"
            );
        }
    }
    #[test]
    fn rejected_limits_and_connection_errors_are_not_reported_as_success() {
        let ast = crate::session::parse_cli_command("setbandwidth client=fd limit=1");
        for reply in [Ok("2900 Error\n".into()), Err("connection reset".into())] {
            assert!(!setbandwidth_with(&ast, &context(), |_, _, _| reply).success);
        }
    }

    #[test]
    fn dialog_asks_limit_then_selects_client() {
        use crate::ua_select::tests::scripted;
        let ctx = context();
        // Batch mode: no limit, nothing happens.
        let r = setbandwidth_dialog_with(&crate::session::parse_cli_command("setbandwidth"), &ctx,
            |_, _, _| panic!("no I/O"));
        assert!(r.success && r.text_output.is_empty() && r.error_message.is_empty());
        let (r, console) = scripted(&["."], || setbandwidth_dialog_with(
            &crate::session::parse_cli_command("setbandwidth"), &ctx, |_, _, _| panic!("no I/O")));
        assert!(r.success);
        assert_eq!(console.transcript(), "Enter new bandwidth limit kb/s: .\n");
        // Single client: automatically selected.
        let (r, console) = scripted(&["x", "4"], || setbandwidth_dialog_with(
            &crate::session::parse_cli_command("setbandwidth"), &ctx, |target, _, command| {
                assert!(matches!(target, Target::Client(_)));
                assert_eq!(command, "setbandwidth=4096 Job=\n");
                Ok("2000 OK Bandwidth\n".into())
            }));
        assert!(r.success);
        assert_eq!(console.transcript(),
            "Enter new bandwidth limit kb/s: x\nExpected a positive integer, got: x\n\
             Enter new bandwidth limit kb/s: 4\nAutomatically selected Client: fd\n\
             Connecting to Client fd at localhost:9102\nOK Limiting bandwidth to 4kb/s \n");
        // Unknown client: original error, then the list.
        let r = setbandwidth_dialog_with(&crate::session::parse_cli_command("setbandwidth limit=1 client=nosuch"),
            &ctx, |_, _, _| Ok("2000 OK Bandwidth\n".into()));
        assert_eq!(r.error_message, "Error: Client resource nosuch does not exist.\n");
        assert!(r.text_output.starts_with("Automatically selected Client: fd\n"));
    }
}
