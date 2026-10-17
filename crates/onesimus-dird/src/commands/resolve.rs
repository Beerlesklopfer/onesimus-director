//! Resolve hostnames locally or on the selected File/Storage Daemon
//! (C++ ua_cmds.cc ResolveCmd, fd_cmds.cc DoClientResolve, sd_cmds.cc DoStorageResolve).
use crate::{command_ast::CommandAst, command_executor::{ApiMode, ExecContext}, command_queue::CommandResult};
use bareos_config::resources::{ClientResource, StorageResource};
use std::net::IpAddr;

pub fn cmd_resolve(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    resolve_with(ast, ctx, host_to_ip_addrs,
        |client, director, commands| {
            let (info, fd) = crate::fd_protocol::connect_fd_console(&client.address,
                client.port, &client.password, director, &client.name,
            (client.tls_enable, client.tls_require));
            (info, fd.and_then(|fd| crate::fd_protocol::fd_query_commands(fd, commands)))
        },
        |storage, director, commands| bareos_stored::command::sd_query_commands(&storage.address,
            storage.port, &storage.password, director, commands))
}

// glibc address conversion functions (not exported by the libc crate).
unsafe extern "C" {
    fn inet_aton(cp: *const libc::c_char, inp: *mut libc::in_addr) -> libc::c_int;
    fn inet_pton(af: libc::c_int, src: *const libc::c_char, dst: *mut libc::c_void) -> libc::c_int;
    fn inet_ntop(af: libc::c_int, src: *const libc::c_void, dst: *mut libc::c_char, size: libc::socklen_t) -> *const libc::c_char;
}

/// C++ BnetHost2IpAddrs(host, 0, &errstr) (lib/bnet.cc:293): numeric IPv4
/// (inet_aton) or IPv6 (inet_pton) literals directly, otherwise getaddrinfo
/// for AF_INET6 and then AF_INET, keeping order and duplicates. The error is
/// the gai_strerror text of the AF_INET lookup.
fn host_to_ip_addrs(host: &str) -> Result<Vec<IpAddr>, String> {
    let Ok(c_host) = std::ffi::CString::new(host) else { return Err("invalid hostname".into()) };
    let mut ipv4: libc::in_addr = unsafe { std::mem::zeroed() };
    if unsafe { inet_aton(c_host.as_ptr(), &mut ipv4) } != 0 {
        return Ok(vec![IpAddr::V4(u32::from_be(ipv4.s_addr).into())]);
    }
    let mut ipv6: libc::in6_addr = unsafe { std::mem::zeroed() };
    if unsafe { inet_pton(libc::AF_INET6, c_host.as_ptr(), (&mut ipv6 as *mut libc::in6_addr).cast()) } == 1 {
        return Ok(vec![IpAddr::V6(ipv6.s6_addr.into())]);
    }
    let mut addresses = vec![];
    let _ = resolv_host(libc::AF_INET6, &c_host, &mut addresses);
    let error = resolv_host(libc::AF_INET, &c_host, &mut addresses);
    match error {
        Err(error) if addresses.is_empty() => Err(error),
        _ => Ok(addresses),
    }
}

/// C++ resolv_host (lib/bnet.cc:235): getaddrinfo with SOCK_STREAM/IPPROTO_TCP.
fn resolv_host(family: libc::c_int, host: &std::ffi::CStr, addresses: &mut Vec<IpAddr>) -> Result<(), String> {
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = family;
    hints.ai_socktype = libc::SOCK_STREAM;
    hints.ai_protocol = libc::IPPROTO_TCP;
    let mut list: *mut libc::addrinfo = std::ptr::null_mut();
    let result = unsafe { libc::getaddrinfo(host.as_ptr(), std::ptr::null(), &hints, &mut list) };
    if result != 0 {
        return Err(unsafe { std::ffi::CStr::from_ptr(libc::gai_strerror(result)) }.to_string_lossy().into_owned());
    }
    let mut entry = list;
    while let Some(info) = unsafe { entry.as_ref() } {
        match info.ai_family {
            libc::AF_INET => {
                let address = unsafe { &*(info.ai_addr as *const libc::sockaddr_in) };
                addresses.push(IpAddr::V4(u32::from_be(address.sin_addr.s_addr).into()));
            }
            libc::AF_INET6 => {
                let address = unsafe { &*(info.ai_addr as *const libc::sockaddr_in6) };
                addresses.push(IpAddr::V6(address.sin6_addr.s6_addr.into()));
            }
            _ => {}
        }
        entry = info.ai_next;
    }
    unsafe { libc::freeaddrinfo(list) };
    Ok(())
}

/// IPADDR::build_address_str without port (lib/address_conf.cc), via inet_ntop.
fn address_string(address: &IpAddr) -> String {
    let mut buffer = [0 as libc::c_char; 64];
    let (kind, text) = match address {
        IpAddr::V4(v4) => {
            let raw = libc::in_addr { s_addr: u32::from(*v4).to_be() };
            ("ipv4", unsafe { inet_ntop(libc::AF_INET, (&raw as *const libc::in_addr).cast(), buffer.as_mut_ptr(), 64) })
        }
        IpAddr::V6(v6) => {
            let raw = libc::in6_addr { s6_addr: v6.octets() };
            ("ipv6", unsafe { inet_ntop(libc::AF_INET6, (&raw as *const libc::in6_addr).cast(), buffer.as_mut_ptr(), 64) })
        }
    };
    let text = if text.is_null() { address.to_string() }
        else { unsafe { std::ffi::CStr::from_ptr(text) }.to_string_lossy().into_owned() };
    format!("host[{kind};{text}] ")
}

fn resolve_with(
    ast: &CommandAst, ctx: &ExecContext,
    mut local: impl FnMut(&str) -> Result<Vec<IpAddr>, String>,
    mut fd: impl FnMut(&ClientResource, &str, &[String]) -> (String, Result<String, String>),
    mut sd: impl FnMut(&StorageResource, &str, &[String]) -> Result<String, String>,
) -> CommandResult {
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else { return CommandResult::error("No config\n".into()); };
    let mut client = None;
    let mut storage = None;
    let mut hosts = vec![];
    // Selector keywords with a value are consumed ("zapped"); every other
    // keyword is a hostname, including the key of `name=value`.
    for argument in &ast.arguments {
        let key = argument.key.to_ascii_lowercase();
        if key == "client" || key == "fd" {
            let Some(name) = argument.value.as_deref() else {
                return CommandResult::error("Client name missing.\n".into());
            };
            match config.find_client(name).filter(|c| crate::acl::access_audited(ctx, crate::acl::Kind::Client, &c.name)) {
                Some(found) => client = Some(found.clone()),
                None => return CommandResult::error(format!("Client \"{name}\" not found.\n")),
            }
        } else if key == "storage" {
            let Some(name) = argument.value.as_deref() else {
                return CommandResult::error("Storage name missing.\n".into());
            };
            match config.find_storage(name).filter(|s| crate::acl::access_audited(ctx, crate::acl::Kind::Storage, &s.name)) {
                Some(found) => storage = Some(found.clone()),
                None => return CommandResult::error(format!("Storage \"{name}\" not found.\n")),
            }
        } else {
            hosts.push(argument.key.as_str());
        }
    }
    let director = config.director.name.clone();
    drop(config);
    let mut output = String::new();
    let commands: Vec<_> = hosts.iter().map(|host| format!("resolve {host}")).collect();
    if let Some(client) = &client {
        if ctx.api_mode == ApiMode::Human {
            output.push_str(&format!("Connecting to Client {} at {}:{}\n", client.name, client.address, client.port));
        }
        let (info, reply) = fd(client, &director, &commands);
        output.push_str(&info);
        match reply {
            Ok(reply) => output.push_str(&reply),
            Err(_) => output.push_str(&format!("\nFailed to connect to Client {}.\n====\n", client.name)),
        }
    }
    if let Some(storage) = &storage {
        output.push_str(&format!("Connecting to Storage daemon {} at {}:{} ...\n", storage.name, storage.address, storage.port));
        match sd(storage, &director, &commands) {
            Ok(reply) => output.push_str(&reply),
            Err(_) => {
                let mut result = CommandResult::error("Failed to connect to Storage daemon.\n".into());
                result.text_output = output; return result;
            }
        }
    }
    if client.is_none() && storage.is_none() {
        for host in hosts {
            match local(host) {
                Ok(addresses) => {
                    let formatted: String = addresses.iter().map(address_string).collect();
                    output.push_str(&format!("{director} resolves {host} to {formatted}\n"));
                }
                Err(_) => {
                    let mut result = CommandResult::error(format!("{director} Failed to resolve {host}\n"));
                    result.text_output = output; return result;
                }
            }
        }
    }
    CommandResult::ok(output)
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
    fn restricted_context(policy: &str) -> ExecContext {
        let mut ctx = context();
        let extra = crate::director_config::DirectorConfig::from_str(&format!("Director {{Name=dir}}\nConsole {{Name=limited; Password=x; {policy}}}")).unwrap();
        ctx.config.as_ref().unwrap().write().unwrap().raw.extend(extra.raw.into_iter().filter(|r| r.norm_type() == "console"));
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        ctx.session_id = manager.create_session("limited");
        manager.authenticate_session(ctx.session_id);
        ctx.session_mgr = Some(manager);
        ctx
    }
    #[test]
    fn resolve_checks_every_resource_before_any_daemon_request() {
        for (policy, expected) in [("StorageACL=sd", "Client"), ("ClientACL=fd", "Storage")] {
            let ctx = restricted_context(policy);
            let result = resolve_with(&crate::session::parse_cli_command("resolve client=fd storage=sd localhost"), &ctx,
                |_| panic!("no DNS"), |_,_,_| panic!("no FD before all ACL checks"), |_,_,_| panic!("no SD"));
            assert!(!result.success);
            assert!(result.error_message.starts_with(expected));
            assert!(result.text_output.is_empty());
        }
        let ctx = restricted_context("ClientACL=fd; StorageACL=sd");
        let result = resolve_with(&crate::session::parse_cli_command("resolve client=fd storage=sd localhost"), &ctx,
            |_| panic!("no DNS"), |_,_,_| (String::new(), Ok("FD\n".into())), |_,_,_| Ok("SD\n".into()));
        assert!(result.success);
        assert!(result.text_output.contains("FD\n"));
        assert!(result.text_output.ends_with("SD\n"));
    }
    #[test]
    fn local_addresses_keep_resolver_order_and_duplicates() {
        let result = resolve_with(&crate::session::parse_cli_command("resolve example"), &context(),
            |_| Ok(vec!["::1".parse().unwrap(), "127.0.0.1".parse().unwrap(), "::1".parse().unwrap()]),
            |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert_eq!(result.text_output, "dir resolves example to host[ipv6;::1] host[ipv4;127.0.0.1] host[ipv6;::1] \n");
    }
    #[test]
    fn remote_targets_receive_every_hostname_without_local_resolution() {
        let result = resolve_with(&crate::session::parse_cli_command("resolve client=fd storage=sd one.invalid two.invalid"), &context(),
            |_| panic!("must resolve at remote daemon"),
            |client, director, commands| { assert_eq!(client.name,"fd"); assert_eq!(director,"dir"); assert_eq!(commands, &["resolve one.invalid", "resolve two.invalid"]); (" Handshake: Immediate TLS, Encryption: None\n".into(), Ok("FD result\r\n".into())) },
            |storage, director, commands| { assert_eq!(storage.name,"sd"); assert_eq!(director,"dir"); assert_eq!(commands.len(),2); Ok("SD result\n".into()) });
        assert_eq!(result.text_output, "Connecting to Client fd at localhost:9102\n Handshake: Immediate TLS, Encryption: None\nFD result\r\nConnecting to Storage daemon sd at localhost:9103 ...\nSD result\n");
    }
    #[test]
    fn unknown_targets_do_not_fall_back_to_local_dns_and_failures_are_visible() {
        let invalid = resolve_with(&crate::session::parse_cli_command("resolve storage=unknown example"), &context(),
            |_| panic!("no DNS"), |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert!(!invalid.success);
        let failure = resolve_with(&crate::session::parse_cli_command("resolve invalid"), &context(),
            |_| Err("DNS failure".into()), |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert!(!failure.success);
        assert_eq!(failure.error_message, "dir Failed to resolve invalid\n");
    }
    #[test]
    fn literal_ip_addresses_resolve_without_dns_fixtures() {
        let result = cmd_resolve(&crate::session::parse_cli_command("resolve 127.0.0.1 ::1"), &context());
        assert!(result.success);
        assert_eq!(result.text_output, "dir resolves 127.0.0.1 to host[ipv4;127.0.0.1] \ndir resolves ::1 to host[ipv6;::1] \n");
    }
    #[test]
    fn selector_values_are_consumed_and_other_keys_are_hostnames() {
        let result = resolve_with(&crate::session::parse_cli_command("resolve foo=bar client=nosuch"), &context(),
            |_| panic!("no DNS"), |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert_eq!(result.error_message, "Client \"nosuch\" not found.\n");
        let result = resolve_with(&crate::session::parse_cli_command("resolve client=nosuch client=fd foo=bar"), &context(),
            |_| panic!("no DNS"), |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert_eq!(result.error_message, "Client \"nosuch\" not found.\n");
        let result = resolve_with(&crate::session::parse_cli_command("resolve foo=bar"), &context(),
            |host| { assert_eq!(host, "foo"); Err("Name or service not known".into()) }, |_,_,_| panic!("no FD"), |_,_,_| panic!("no SD"));
        assert_eq!(result.error_message, "dir Failed to resolve foo\n");
    }
    #[test]
    fn daemon_connection_failures_use_original_texts() {
        let result = resolve_with(&crate::session::parse_cli_command("resolve client=fd storage=sd x"), &context(),
            |_| panic!("no DNS"), |_,_,_| ("Probing client protocol... (result will be saved until config reload)".into(), Err("refused".into())),
            |_,_,_| Err("refused".into()));
        assert_eq!(result.text_output, "Connecting to Client fd at localhost:9102\nProbing client protocol... (result will be saved until config reload)\nFailed to connect to Client fd.\n====\nConnecting to Storage daemon sd at localhost:9103 ...\n");
        assert_eq!(result.error_message, "Failed to connect to Storage daemon.\n");
    }
    #[test]
    fn localhost_uses_ipv6_lookup_before_ipv4_lookup() {
        let addresses = host_to_ip_addrs("localhost").unwrap();
        if let Some(first_v4) = addresses.iter().position(|a| a.is_ipv4()) {
            assert!(addresses[first_v4..].iter().all(|a| a.is_ipv4()));
        }
        assert_eq!(host_to_ip_addrs("127.1").unwrap(), vec!["127.0.0.1".parse::<IpAddr>().unwrap()]);
        assert!(host_to_ip_addrs("nonexistent.invalid").is_err());
        assert_eq!(address_string(&"::ffff:1.2.3.4".parse().unwrap()), "host[ipv6;::ffff:1.2.3.4] ");
    }
}
