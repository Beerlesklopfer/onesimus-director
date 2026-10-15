//! Resolve hostnames locally or on the selected File/Storage Daemon.
use crate::{command_ast::CommandAst, command_executor::{ApiMode, ExecContext}, command_queue::CommandResult};
use bareos_config::resources::{ClientResource, StorageResource};
use std::net::{IpAddr, ToSocketAddrs};

pub fn cmd_resolve(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    resolve_with(ast, ctx,
        |host| (host, 0).to_socket_addrs().map(|addresses| addresses.map(|address| address.ip()).collect()).map_err(|e| e.to_string()),
        |client, director, commands| crate::fd_protocol::fd_query_commands(&client.address,
            client.port, &client.password, director, commands),
        |storage, director, commands| bareos_stored::command::sd_query_commands(&storage.address,
            storage.port, &storage.password, director, commands))
}

fn resolve_with(
    ast: &CommandAst, ctx: &ExecContext,
    mut local: impl FnMut(&str) -> Result<Vec<IpAddr>, String>,
    mut fd: impl FnMut(&ClientResource, &str, &[String]) -> Result<String, String>,
    mut sd: impl FnMut(&StorageResource, &str, &[String]) -> Result<String, String>,
) -> CommandResult {
    let mut client_name = None;
    let mut storage_name = None;
    let mut hosts = vec![];
    for argument in &ast.arguments {
        match argument.key.to_ascii_lowercase().as_str() {
            "client" | "fd" => {
                if client_name.is_some() { return CommandResult::error("resolve: duplicate client selector\n".into()); }
                let Some(name) = argument.value.as_deref().filter(|v| !v.is_empty()) else {
                    return CommandResult::error("Client name missing.\n".into());
                };
                client_name = Some(name);
            }
            "storage" => {
                if storage_name.is_some() { return CommandResult::error("resolve: duplicate storage selector\n".into()); }
                let Some(name) = argument.value.as_deref().filter(|v| !v.is_empty()) else {
                    return CommandResult::error("Storage name missing.\n".into());
                };
                storage_name = Some(name);
            }
            _ => {
                if argument.value.is_some() || argument.key.is_empty()
                    || argument.key.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    return CommandResult::error("resolve: invalid hostname argument\n".into());
                }
                hosts.push(argument.key.as_str());
            }
        }
    }
    let mut output = String::new();
    if client_name.is_none() && storage_name.is_none() {
        for host in hosts {
            match local(host) {
                Ok(addresses) if !addresses.is_empty() => {
                    let mut seen = std::collections::HashSet::new();
                    let formatted: String = addresses.into_iter().filter(|address| seen.insert(*address))
                        .map(|address| format!("host[{};{address}] ", if address.is_ipv4() { "ipv4" } else { "ipv6" })).collect();
                    output.push_str(&format!("bareos-dir resolves {host} to {formatted}\n"));
                }
                _ => {
                    let mut result = CommandResult::error(format!("bareos-dir Failed to resolve {host}\n"));
                    result.text_output = output; return result;
                }
            }
        }
        return CommandResult::ok(output);
    }
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else { return CommandResult::error("No config\n".into()); };
    let client = if let Some(name) = client_name {
        match config.find_client(name) {
            Some(client) => Some(client.clone()),
            None => return CommandResult::error(format!("Client \"{name}\" not found.\n")),
        }
    } else { None };
    let storage = if let Some(name) = storage_name {
        match config.find_storage(name) {
            Some(storage) => Some(storage.clone()),
            None => return CommandResult::error(format!("Storage \"{name}\" not found.\n")),
        }
    } else { None };
    let director = config.director.name.clone();
    drop(config);
    let commands: Vec<_> = hosts.into_iter().map(|host| format!("resolve {host}")).collect();
    let mut errors = String::new();
    if let Some(client) = client {
        if ctx.api_mode == ApiMode::Human {
            output.push_str(&format!("Connecting to Client {} at {}:{}\n", client.name, client.address, client.port));
        }
        match fd(&client, &director, &commands) {
            Ok(reply) => output.push_str(&reply),
            Err(error) => errors.push_str(&format!("Failed to connect to Client {}.\n{error}\n", client.name)),
        }
    }
    if let Some(storage) = storage {
        output.push_str(&format!("Connecting to Storage daemon {} at {}:{} ...\n", storage.name, storage.address, storage.port));
        match sd(&storage, &director, &commands) {
            Ok(reply) => output.push_str(&reply),
            Err(error) => errors.push_str(&format!("Failed to connect to Storage {}.\n{error}\n", storage.name)),
        }
    }
    if errors.is_empty() { CommandResult::ok(output) }
    else { let mut result = CommandResult::error(errors); result.text_output = output; result }
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
            session_mgr: None, catalog: None, api_mode: ApiMode::Human, bus_rx: None, job_manager: None }
    }
    #[test]
    fn local_addresses_use_original_ipv4_ipv6_layout_and_order() {
        let result = resolve_with(&crate::session::parse_cli_command("resolve example"), &context(),
            |_| Ok(vec!["::1".parse().unwrap(), "127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]),
            |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert_eq!(result.text_output, "bareos-dir resolves example to host[ipv6;::1] host[ipv4;127.0.0.1] \n");
    }
    #[test]
    fn remote_targets_receive_every_hostname_without_local_resolution() {
        let result = resolve_with(&crate::session::parse_cli_command("resolve client=fd storage=sd one.invalid two.invalid"), &context(),
            |_| panic!("must resolve at remote daemon"),
            |client, director, commands| { assert_eq!(client.name,"fd"); assert_eq!(director,"dir"); assert_eq!(commands, &["resolve one.invalid", "resolve two.invalid"]); Ok("FD result\r\n".into()) },
            |storage, director, commands| { assert_eq!(storage.name,"sd"); assert_eq!(director,"dir"); assert_eq!(commands.len(),2); Ok("SD result\n".into()) });
        assert_eq!(result.text_output, "Connecting to Client fd at localhost:9102\nFD result\r\nConnecting to Storage daemon sd at localhost:9103 ...\nSD result\n");
    }
    #[test]
    fn unknown_targets_do_not_fall_back_to_local_dns_and_failures_are_visible() {
        let invalid = resolve_with(&crate::session::parse_cli_command("resolve storage=unknown example"), &context(),
            |_| panic!("no DNS"), |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert!(!invalid.success);
        let failure = resolve_with(&crate::session::parse_cli_command("resolve invalid"), &context(),
            |_| Err("DNS failure".into()), |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert!(!failure.success);
        assert_eq!(failure.error_message, "bareos-dir Failed to resolve invalid\n");
    }
    #[test]
    fn literal_ip_addresses_resolve_without_dns_fixtures() {
        let result = cmd_resolve(&crate::session::parse_cli_command("resolve 127.0.0.1 ::1"), &context());
        assert!(result.success);
        assert_eq!(result.text_output, "bareos-dir resolves 127.0.0.1 to host[ipv4;127.0.0.1] \nbareos-dir resolves ::1 to host[ipv6;::1] \n");
    }
}
