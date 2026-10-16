//! Console auditing (dird/ua_audit.cc) and the daemon message delivery it
//! needs (lib/message.cc e_msg/DispatchMessage for `file`/`append`).
//!
//! With `Auditing = yes` in the Director resource, every audited console
//! command and every audited ACL failure becomes an M_AUDIT message, routed
//! through the Director's Messages resource like Emsg(M_AUDIT, ...).

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;

use bareos_config::raw::ConfResource;

use crate::acl::Kind;
use crate::command_executor::ExecContext;
use crate::director_config::DirectorConfig;
use crate::message_bus::MsgType;

/// ua_cmds.cc commands[]: keys whose `audit_event` column is true.
const AUDIT_EVENT_COMMANDS: &[&str] = &[
    ".sql", ".status", ".bvfs_lsdirs", ".bvfs_lsfiles", ".bvfs_update", ".bvfs_get_jobids",
    ".bvfs_versions", ".bvfs_restore", ".bvfs_cleanup", ".bvfs_clear_cache", "add", "automount",
    "cancel", "configure", "create", "delete", "disable", "enable", "estimate", "export", "import",
    "label", "list", "llist", "mount", "move", "prune", "purge", "query", "restore", "relabel",
    "release", "reload", "rerun", "resolve", "run", "status", "setbandwidth", "setdebug",
    "setdevice", "setip", "show", "sqlquery", "trace", "truncate", "unmount", "umount", "update",
    "use", "var",
];

thread_local! {
    /// Peer address of the console connection served by this thread
    /// (BareosSocket::host()); "unknown" without a socket.
    static PEER_HOST: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Remember the peer address of the connection this thread serves.
pub fn set_peer_host(host: String) {
    PEER_HOST.with(|peer| *peer.borrow_mut() = Some(host));
}

fn peer_host() -> String {
    PEER_HOST.with(|peer| peer.borrow().clone()).unwrap_or_else(|| "unknown".into())
}

fn director(config: &DirectorConfig) -> Option<&ConfResource> {
    config.raw.iter().find(|r| r.norm_type() == "director")
}

fn directive<'a>(res: &'a ConfResource, key: &str) -> Option<&'a str> {
    res.directives.iter().rev().find(|d| d.norm_key() == key).and_then(|d| d.first())
}

fn auditing(config: &DirectorConfig) -> bool {
    director(config)
        .and_then(|d| directive(d, "auditing"))
        .is_some_and(|v| v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true"))
}

/// Console name and host as in ua_audit.cc (no user ACL: "default").
fn who(ctx: &ExecContext) -> (String, String) {
    let console = ctx.session_mgr.as_ref()
        .and_then(|manager| manager.authenticated_console(ctx.session_id))
        .filter(|name| name != "*UserAgent*")
        .unwrap_or_else(|| "default".into());
    (console, peer_host())
}

fn with_config(ctx: &ExecContext, f: impl FnOnce(&DirectorConfig)) {
    if let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) {
        f(&config);
    }
}

/// UaContext::AuditEventWanted(): the per-command audit_event flag, narrowed
/// by `Audit Events` (compared with the word as typed, argk[0]).
fn audit_event_wanted(config: &DirectorConfig, key: &str, typed: &str) -> bool {
    if !AUDIT_EVENT_COMMANDS.contains(&key) {
        return false;
    }
    let events: Vec<&str> = director(config).map(|d| d.directives.iter()
        .filter(|d| d.norm_key() == "auditevents")
        .flat_map(|d| d.values())
        .collect()).unwrap_or_default();
    events.is_empty() || events.iter().any(|event| event.eq_ignore_ascii_case(typed))
}

/// Do_a_command(): LogAuditEventCmdline() for a permitted command `key`
/// typed as `typed`; `cmdline` is the received line (ua->cmd).
pub fn command(ctx: &ExecContext, key: &str, typed: &str, cmdline: &str) {
    with_config(ctx, |config| {
        if !auditing(config) || !audit_event_wanted(config, key, typed) {
            return;
        }
        let (console, host) = who(ctx);
        // ua->cmd is a C string
        let cmdline = cmdline.split('\0').next().unwrap_or("");
        daemon_message(config, MsgType::Audit, &format!("Console [{console}] from [{host}] cmdline {cmdline}\n"));
    });
}

/// UaContext::LogAuditEventAclFailure().
pub fn acl_failure(ctx: &ExecContext, kind: Kind, item: &str) {
    with_config(ctx, |config| {
        if !auditing(config) {
            return;
        }
        let (console, host) = who(ctx);
        let what = match kind {
            Kind::Job => "for Job",
            Kind::Client => "for Client",
            Kind::Storage => "for Storage",
            Kind::Schedule => "for Schedule",
            Kind::Pool => "for Pool",
            Kind::Command => "for Command",
            Kind::FileSet => "for Fileset",
            Kind::Catalog => "for Catalog",
            Kind::Where => "for Where restore location",
            Kind::PluginOptions => "for Plugin Options",
        };
        daemon_message(config, MsgType::Audit,
            &format!("Console [{console}] from [{host}], Audit acl failure {what} {item}\n"));
    });
}

/// UaContext::LogAuditEventInfoMsg().
#[allow(dead_code)]
pub fn info(ctx: &ExecContext, message: &str) {
    with_config(ctx, |config| {
        if !auditing(config) {
            return;
        }
        let (console, host) = who(ctx);
        daemon_message(config, MsgType::Audit, &format!("Console [{console}] from [{host}] info message {message}\n"));
    });
}

// --- daemon messages (lib/message.cc) ---------------------------------------

/// lib/message.h M_* code of a bus message type.
fn code(msg_type: MsgType) -> u32 {
    match msg_type {
        MsgType::Fatal => 3,
        MsgType::Error => 4,
        MsgType::Warning => 5,
        MsgType::Info => 6,
        MsgType::Saved => 7,
        MsgType::Skipped => 9,
        MsgType::Mount => 10,
        MsgType::Restored => 13,
        MsgType::Audit => 17,
    }
}

/// Destination files kept open like MessageDestinationInfo::file_pointer_.
static FILES: Mutex<Option<HashMap<(&'static str, String), File>>> = Mutex::new(None);

/// e_msg() for message types without a prefix of their own ("<name>: ")
/// delivered by DispatchMessage() through the daemon Messages resource
/// (the Director's `Messages`). Only `file` and `append` destinations are
/// served; each line gets the "%d-%b %H:%M " timestamp (or the resource's
/// TimestampFormat).
pub fn daemon_message(config: &DirectorConfig, msg_type: MsgType, text: &str) {
    let Some(name) = director(config).and_then(|d| directive(d, "messages")) else { return };
    let Some(res) = config.raw.iter()
        .find(|r| r.norm_type() == "messages" && r.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
    else { return };
    let format = directive(res, "timestampformat").filter(|f| !f.is_empty()).unwrap_or("%d-%b %H:%M");
    let line = format!("{} {}: {text}", crate::localtime::strftime_now(format), config.director.name);
    let bit = 1u32 << code(msg_type);
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let files = files.get_or_insert_with(HashMap::new);
    for dest in crate::commands::show::destinations(res) {
        if dest.types & bit == 0 || !matches!(dest.code, "file" | "append") {
            continue;
        }
        let key = (dest.code, dest.where_.clone());
        if !files.contains_key(&key) {
            // DispatchMessage(): kAppend opens "ab", kFile "w+b"
            let mut options = OpenOptions::new();
            if dest.code == "append" { options.append(true); } else { options.write(true).truncate(true); }
            match options.create(true).open(&dest.where_) {
                Ok(file) => { files.insert(key.clone(), file); }
                Err(error) => {
                    tracing::warn!("cannot open message destination {}: {error}", dest.where_);
                    continue;
                }
            }
        }
        if let Some(file) = files.get_mut(&key) {
            if file.write_all(line.as_bytes()).and_then(|_| file.flush()).is_err() {
                files.remove(&key); // reopened with the next message (log rotation)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn context(dir: &std::path::Path, extra: &str, console: &str) -> ExecContext {
        let config = DirectorConfig::from_str(&format!(r#"
Director {{ Name = audit-dir; Password = secret; Messages = Daemon; Auditing = yes {extra} }}
Messages {{ Name = Daemon; append = "{0}/audit.log" = audit; append = "{0}/all.log" = all, !audit }}
Console {{ Name = limited; Password = pw; CommandACL = list, show; CatalogACL = !*all* }}
"#, dir.display())).unwrap();
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session(console);
        manager.authenticate_session(id);
        ExecContext { session_id: id, config: Some(Arc::new(RwLock::new(config))), session_mgr: Some(manager),
            catalog: None, api_mode: crate::command_executor::ApiMode::Human, bus_rx: None, job_manager: None }
    }

    fn lines(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path).unwrap_or_default().lines()
            .map(|l| l.splitn(3, ' ').nth(2).unwrap_or("").to_string()).collect()
    }

    #[test]
    fn audited_commands_are_original_command_keys() {
        for key in AUDIT_EVENT_COMMANDS {
            assert!(crate::ua_commands_table::UA_COMMANDS.iter().any(|entry| entry.0 == *key), "{key}");
        }
    }

    #[test]
    fn audit_lines_follow_ua_audit_and_the_messages_resource() {
        let dir = std::env::temp_dir().join(format!("onesimus-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        set_peer_host("127.0.0.1".into());
        let ctx = context(&dir, "", "*UserAgent*");
        command(&ctx, "list", "list", "list pools");
        command(&ctx, "whoami", "whoami", "whoami");
        command(&ctx, "show", "SHO", "SHO clients\n");
        let limited = context(&dir, "", "limited");
        assert!(!crate::acl::access_audited(&limited, Kind::Command, "sqlquery"));
        assert!(!crate::acl::access_audited(&limited, Kind::Catalog, "MyCatalog"));
        assert!(crate::acl::access_audited(&limited, Kind::Command, "list"));
        let filtered = context(&dir, "; Audit Events = show", "*UserAgent*");
        command(&filtered, "list", "list", "list jobs");
        command(&filtered, "show", "show", "show pools");
        assert_eq!(lines(&dir.join("audit.log")), [
            "audit-dir: Console [default] from [127.0.0.1] cmdline list pools",
            "audit-dir: Console [default] from [127.0.0.1] cmdline SHO clients",
            "",
            "audit-dir: Console [limited] from [127.0.0.1], Audit acl failure for Command sqlquery",
            "audit-dir: Console [limited] from [127.0.0.1], Audit acl failure for Catalog MyCatalog",
            "audit-dir: Console [default] from [127.0.0.1] cmdline show pools",
        ]);
        assert!(!dir.join("all.log").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
