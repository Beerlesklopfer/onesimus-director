// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Console auditing (dird/ua_audit.cc) and the daemon message delivery it
//! needs (lib/message.cc e_msg/DispatchMessage; mail and operator in
//! `mail.rs`).
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
        // `.config create/update`: the resource text may carry passwords
        let masked;
        let cmdline = if key == ".config" {
            masked = crate::commands::config_crud::mask_cmdline(cmdline);
            masked.as_str()
        } else {
            cmdline
        };
        daemon_message(config, MsgType::Audit, &format!("Console [{console}] from [{host}] cmdline {cmdline}\n"));
    });
}

/// UserSelectFilesFromTree(): AuditEventWanted(audit_event) and
/// LogAuditEventCmdline() of a restore tree command. The tree runs on a new
/// UaContext without user ACL, so the console is always "default".
pub fn tree_command(ctx: &ExecContext, audit_event: bool, typed: &str, cmdline: &str) {
    with_config(ctx, |config| {
        if !auditing(config) || !audit_event {
            return;
        }
        let events: Vec<&str> = director(config).map(|d| d.directives.iter()
            .filter(|d| d.norm_key() == "auditevents")
            .flat_map(|d| d.values())
            .collect()).unwrap_or_default();
        if !events.is_empty() && !events.iter().any(|event| event.eq_ignore_ascii_case(typed)) {
            return;
        }
        let cmdline = cmdline.split('\0').next().unwrap_or("");
        daemon_message(config, MsgType::Audit,
            &format!("Console [default] from [{}] cmdline {cmdline}\n", peer_host()));
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
pub(crate) fn code(msg_type: MsgType) -> u32 {
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

/// `mtime` in the Messages resource's TimestampFormat (default
/// log_timestamp_format "%d-%b %H:%M"), without the separating blank.
fn timestamp_at(res: &ConfResource, mtime: i64) -> String {
    let format = directive(res, "timestampformat").filter(|f| !f.is_empty()).unwrap_or("%d-%b %H:%M");
    crate::localtime::strftime(format, mtime as libc::time_t)
}

fn now() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// Destination files kept open like MessageDestinationInfo::file_pointer_.
static FILES: Mutex<Option<HashMap<(&'static str, String), File>>> = Mutex::new(None);

/// e_msg() for message types without a prefix of their own ("<name>: ")
/// delivered by DispatchMessage() through the daemon Messages resource
/// (the Director's `Messages`).
pub fn daemon_message(config: &DirectorConfig, msg_type: MsgType, text: &str) {
    let Some(res) = daemon_messages(config) else { return };
    dispatch(res, code(msg_type), 0, &format!("{}: {text}", config.director.name), None);
}

/// The Director's Messages resource (`daemon_msgs`).
pub(crate) fn daemon_messages(config: &DirectorConfig) -> Option<&ConfResource> {
    let name = director(config).and_then(|d| directive(d, "messages"))?;
    messages_resource(config, name)
}

/// The Messages resource `name`.
pub(crate) fn messages_resource<'a>(config: &'a DirectorConfig, name: &str) -> Option<&'a ConfResource> {
    // my_name for %d of mail and operator commands without a job
    crate::mail::set_director(&config.director.name);
    config.raw.iter()
        .find(|r| r.norm_type() == "messages" && r.name().is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

/// DispatchMessage(jcr, type, mtime, msg) over the destinations of `res`
/// that want message type `code` (lib/message.h M_*). `mtime` 0 is now, 1
/// is now without the date prefix. Served destinations: `console` (the
/// console message queue), `file`, `append` (date prefix, then the message)
/// `catalog` (a Log row through DirDbLogInsert, only with a JCR that
/// has a catalog: `job_id`), `mail`, `mailonerror` and `mailonsuccess`
/// (collected for the job `job_id`, None or 0: the daemon, and sent when
/// it ends, message.cc:780-808) and `operator` (sent at once,
/// message.cc:760-779), see `mail.rs`, `stdout` and `stderr` (date prefix,
/// then the message) and `syslog` (LOG_DAEMON | LOG_ERR, the message alone).
pub(crate) fn dispatch(res: &ConfResource, code: u32, mtime: i64, msg: &str, job_id: Option<u32>) {
    // Secrets in RunScript command lines never reach a destination
    // (catalog Log, journal, mail, console): crate::secrets.
    let masked = crate::secrets::mask(msg);
    let msg = masked.as_ref();
    let (mtime, dt_conversion) = match mtime {
        0 => (now(), true),
        1 => (now(), false),
        t => (t, true),
    };
    let Some(bit) = 1u32.checked_shl(code) else { return };
    let mut files = FILES.lock().unwrap_or_else(|e| e.into_inner());
    let files = files.get_or_insert_with(HashMap::new);
    for dest in crate::commands::show::destinations(res) {
        if dest.types & bit == 0 {
            continue;
        }
        let dt = if dt_conversion { format!("{} ", timestamp_at(res, mtime)) } else { String::new() };
        match dest.code {
            "console" => {
                crate::console_msg::append(&dt, msg);
                continue;
            }
            "catalog" => {
                if let Some(job_id) = job_id {
                    if !crate::job_messages::log_insert(job_id, mtime, msg) {
                        tracing::warn!("Msg delivery error: Unable to store data in database.");
                    }
                }
                continue;
            }
            "operator" => {
                crate::mail::operator(res, &dest.where_, &format!("{dt}{msg}"), job_id);
                continue;
            }
            "mail" | "mailonerror" | "mailonsuccess" => {
                crate::mail::collect(res, dest.code, &dest.where_, &format!("{dt}{msg}"), job_id);
                continue;
            }
            "stdout" => {
                use std::io::Write as _;
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(format!("{dt}{msg}").as_bytes()).and_then(|()| out.flush());
                continue;
            }
            "stderr" => {
                use std::io::Write as _;
                let mut err = std::io::stderr().lock();
                let _ = err.write_all(format!("{dt}{msg}").as_bytes()).and_then(|()| err.flush());
                continue;
            }
            "syslog" => {
                // message.cc: syslog(LOG_DAEMON | LOG_ERR, "%s", msg), without the date prefix
                syslog_daemon_err(msg);
                continue;
            }
            "file" | "append" => {}
            _ => continue,
        }
        let line = format!("{dt}{msg}");
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

/// The message as the C string `syslog(3)` takes: an interior NUL ends it
/// there, like it ends a C++ `char*`.
fn syslog_text(msg: &str) -> std::ffi::CString {
    let end = msg.find('\0').unwrap_or(msg.len());
    std::ffi::CString::new(&msg[..end]).unwrap_or_default()
}

/// `syslog(LOG_DAEMON | LOG_ERR, "%s", msg)` as in message.cc (which
/// does not call `openlog` either).
fn syslog_daemon_err(msg: &str) {
    let text = syslog_text(msg);
    // SAFETY: both strings are NUL-terminated and "%s" takes exactly one.
    unsafe { libc::syslog(libc::LOG_DAEMON | libc::LOG_ERR, c"%s".as_ptr(), text.as_ptr()) };
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

    /// A RunScript command line the client reports (`shell command: run
    /// ClientBeforeJob "…--password=…"`) reaches neither the catalog `Log`
    /// nor a file destination with the password (crate::secrets).
    #[test]
    fn secrets_in_job_messages_are_masked_for_every_destination() {
        use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse, SqlTable};
        let dir = std::env::temp_dir().join(format!("onesimus-audit-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raw = bareos_config::raw::parse_str(&format!(
            "Messages {{ Name = Standard; catalog = all; append = \"{}/job.log\" = all }}\n", dir.display()), None).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        crate::job_messages::set_catalog(Some(CatalogHandle::mock(move |request| {
            if let CatalogRequest::TextRows { sql } | CatalogRequest::RawQuery { sql } | CatalogRequest::Execute { sql } = request {
                log.lock().unwrap().push(sql);
            }
            CatalogResponse::Rows(SqlTable { columns: vec![], rows: vec![] })
        })));
        let msg = "CLOUD-fd JobId 7: shell command: run ClientBeforeJob \"/etc/bareos/scripts/dump-database.sh \
                   --user=www-data --password=Geheim-42\"\n";
        dispatch(&raw[0], 1, 1, msg, Some(7));
        crate::job_messages::set_catalog(None);
        let sql = seen.lock().unwrap().join("\n");
        assert!(sql.contains("INSERT INTO Log") && sql.contains("--password=***"), "{sql}");
        let file = std::fs::read_to_string(dir.join("job.log")).unwrap();
        assert!(file.contains("--user=www-data --password=***\""), "{file}");
        assert!(!sql.contains("Geheim-42") && !file.contains("Geheim-42"));
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn syslog_text_ends_at_a_nul() {
        assert_eq!(syslog_text("abc\0def").to_str().unwrap(), "abc");
        assert_eq!(syslog_text("plain").to_str().unwrap(), "plain");
    }
}
