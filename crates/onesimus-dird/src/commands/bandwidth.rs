//! setbandwidth: apply daemon-wide limits, expressed in KiB/s at the console.
use crate::{command_ast::CommandAst, command_executor::ExecContext, command_queue::CommandResult};
use bareos_config::resources::{ClientResource, StorageResource};

enum Target { Client(ClientResource), Storage(StorageResource) }

pub fn cmd_setbandwidth(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    setbandwidth_with(ast, ctx, |target, director, command| match target {
        Target::Client(client) => crate::fd_protocol::fd_command(&client.address,
            client.port, &client.password, director, |_| command.to_string()),
        Target::Storage(storage) => bareos_stored::command::sd_control_command(&storage.address,
            storage.port, &storage.password, director, command),
    })
}

fn setbandwidth_with(ast: &CommandAst, ctx: &ExecContext,
    send: impl FnOnce(&Target, &str, &str) -> Result<String, String>,
) -> CommandResult {
    for argument in &ast.arguments {
        if !matches!(argument.key.to_ascii_lowercase().as_str(), "client" | "storage" | "limit" | "yes") {
            return CommandResult::error(format!("setbandwidth: unsupported selector {}; use client=<name> or storage=<name>\n", argument.key));
        }
    }
    let value = |key: &str| ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(key)).and_then(|a| a.value.as_deref());
    let Some(limit) = value("limit").and_then(|value| value.parse::<i64>().ok()).filter(|value| *value >= 0) else {
        return CommandResult::error("setbandwidth: limit=<nonnegative KiB/s> is required\n".into());
    };
    let Some(bytes) = limit.checked_mul(1024) else {
        return CommandResult::error("setbandwidth: limit is too large\n".into());
    };
    let selectors: Vec<_> = ast.arguments.iter().filter(|a| a.key.eq_ignore_ascii_case("client") || a.key.eq_ignore_ascii_case("storage")).collect();
    if selectors.len() != 1 || selectors[0].value.as_deref().is_none_or(str::is_empty) {
        return CommandResult::error("setbandwidth: specify exactly one client=<name> or storage=<name>\n".into());
    }
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else { return CommandResult::error("No config\n".into()); };
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
    let banner = match &target {
        Target::Client(client) => format!("Connecting to Client {} at {}:{}\n", client.name, client.address, client.port),
        Target::Storage(storage) => format!("Connecting to Storage daemon {} at {}:{}\n", storage.name, storage.address, storage.port),
    };
    // Job= is intentionally empty for a daemon-wide limit; job selectors are
    // never silently widened into a daemon-wide operation.
    let command = format!("setbandwidth={bytes} Job=\n");
    match send(&target, &director, &command) {
        Ok(reply) if reply.trim_end() == "2000 OK Bandwidth" =>
            CommandResult::ok(format!("{banner}OK Limiting bandwidth to {limit}kb/s \n")),
        Ok(reply) => {
            let mut result = CommandResult::error(format!("Failed to set bandwidth limit: {reply}"));
            result.text_output = banner; result
        }
        Err(error) => {
            let mut result = CommandResult::error(format!("Failed to set bandwidth limit: {error}\n"));
            result.text_output = banner; result
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
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }
    #[test]
    fn applies_scaled_limit_and_zero_unlimited_to_the_selected_daemon() {
        for (line, expected) in [("setbandwidth client=fd limit=12", "setbandwidth=12288 Job=\n"),
            ("setbandwidth storage=sd limit=0", "setbandwidth=0 Job=\n")] {
            let result = setbandwidth_with(&crate::session::parse_cli_command(line), &context(), |target, director, command| {
                assert_eq!(director, "dir"); assert_eq!(command, expected);
                assert_eq!(matches!(target, Target::Client(_)), line.contains("client="));
                Ok("2000 OK Bandwidth\n".into())
            });
            assert!(result.success, "{line}");
            assert!(result.text_output.contains("OK Limiting bandwidth"));
        }
    }
    #[test]
    fn invalid_requests_and_job_selectors_cannot_change_daemon_wide_limits() {
        for line in ["setbandwidth client=fd", "setbandwidth client=fd limit=-1",
            "setbandwidth client=fd limit=9223372036854775807", "setbandwidth limit=1 client=fd storage=sd",
            "setbandwidth limit=1 jobid=42", "setbandwidth limit=1 all", "setbandwidth client=missing limit=1"] {
            assert!(!setbandwidth_with(&crate::session::parse_cli_command(line), &context(), |_, _, _| panic!("no I/O permitted")).success, "{line}");
        }
    }
    #[test]
    fn rejected_limits_and_connection_errors_are_not_reported_as_success() {
        let ast = crate::session::parse_cli_command("setbandwidth client=fd limit=1");
        for reply in [Ok("2900 Error\n".into()), Err("connection reset".into())] {
            assert!(!setbandwidth_with(&ast, &context(), |_, _, _| reply).success);
        }
    }
}
