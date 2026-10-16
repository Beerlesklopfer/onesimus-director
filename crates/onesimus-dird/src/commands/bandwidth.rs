//! `setbandwidth` — C++ ua_cmds.cc SetbwlimitCmd (:935) with
//! SetbwlimitFiled (:842), setbwlimit_stored (:886), fd_cmds.cc
//! SendBwlimitToFd (:453) and ua_select.cc select_jobs (:1566).
//! Compatibility log C-056.
use bareos_bnet::{BnetMessage, BnetSignal};
use bareos_config::resources::{ClientResource, JobType, StorageResource};

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;
use crate::job_manager::ActiveJob;
use crate::ua_select::{self, Ua};

/// What ConnectToFileDaemon + SendBwlimitToFd produced.
pub enum FdOutcome {
    /// The connection failed.
    NotConnected,
    /// FDVersion < FD_VERSION_4: nothing is sent, the limit counts as set.
    NotSent,
    /// The FD's answer to `setbandwidth=…` (None: socket error).
    Reply(Option<String>),
}

/// The daemon connections `setbandwidth` opens (fakes in the unit tests).
pub trait BwLinks {
    /// ConnectToFileDaemon (no UaContext), then SendBwlimitToFd.
    fn file_daemon(&mut self, client: &ClientResource, director: &str, command: &str) -> FdOutcome;
    /// ConnectToStorageDaemon only: setbwlimit_stored() calls
    /// SendBwlimitToFd(), which talks to `jcr->file_bsock` and, with the
    /// console JCR's FDVersion 0, sends nothing to the Storage daemon.
    fn storage_daemon(&mut self, store: &StorageResource, director: &str) -> bool;
}

/// The real daemon connections.
pub struct NetLinks;

impl BwLinks for NetLinks {
    fn file_daemon(&mut self, client: &ClientResource, director: &str, command: &str) -> FdOutcome {
        let Ok((mut fd, version)) = crate::fd_protocol::connect_fd_with_timeout(
            &client.address, client.port, &client.password, director, Some(std::time::Duration::from_secs(15)))
        else {
            return FdOutcome::NotConnected;
        };
        // ConnectToFileDaemon stores the successful handshake mode (C-060).
        crate::fd_protocol::note_tls_handshake(&client.name);
        let outcome = if version < 4 {
            FdOutcome::NotSent
        } else if fd.send_str(command).is_err() {
            FdOutcome::Reply(None)
        } else {
            // response(): the first data frame (BgetDirmsg skips heartbeats).
            let reply = loop {
                match fd.recv() {
                    Ok(BnetMessage::Data(data)) => break Some(String::from_utf8_lossy(&data).into_owned()),
                    Ok(BnetMessage::Signal(BnetSignal::Heartbeat)) => {
                        let _ = fd.send_signal(BnetSignal::HeartbeatResponse);
                    }
                    Ok(BnetMessage::Signal(_)) | Err(_) => break None,
                }
            };
            FdOutcome::Reply(reply)
        };
        let _ = fd.send_signal(BnetSignal::Terminate);
        outcome
    }

    fn storage_daemon(&mut self, store: &StorageResource, director: &str) -> bool {
        match bareos_stored::connect::connect_sd(&store.address, store.port, &store.password, director) {
            Ok(mut sd) => {
                let _ = sd.send_signal(BnetSignal::Terminate);
                true
            }
            Err(_) => false,
        }
    }
}

pub fn cmd_setbandwidth(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let active = ctx.job_manager.as_ref()
        .map(|mgr| mgr.lock().unwrap().active_jobs()).unwrap_or_default();
    setbandwidth_with(ast, ctx, &active, &mut NetLinks)
}

/// C atoi(): `(int) strtol(text, NULL, 10)` with strtol's saturation.
fn atoi(text: &str) -> i32 {
    let text = text.trim_start_matches([' ', '\t', '\n', '\x0b', '\x0c', '\r']);
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let mut value: i64 = 0;
    for digit in digits.bytes().take_while(u8::is_ascii_digit) {
        let d = i64::from(digit - b'0');
        value = if negative {
            value.checked_mul(10).and_then(|v| v.checked_sub(d)).unwrap_or(i64::MIN)
        } else {
            value.checked_mul(10).and_then(|v| v.checked_add(d)).unwrap_or(i64::MAX)
        };
    }
    value as i32
}

/// FindArgKeyword(): index in `list` of the first argument that is one of
/// its keywords.
fn find_arg_keyword(ast: &CommandAst, list: &[&str]) -> Option<usize> {
    ast.arguments.iter().find_map(|arg| list.iter().position(|key| arg.key.eq_ignore_ascii_case(key)))
}

/// SetbwlimitCmd().
pub fn setbandwidth_with(ast: &CommandAst, ctx: &ExecContext, active: &[ActiveJob], links: &mut dyn BwLinks)
    -> CommandResult {
    let mut ua = Ua::new(ctx);
    let mut limit: i64 = -1;
    if let Some(value) = ua_select::find_arg_with_value(ast, "limit") {
        limit = i64::from(atoi(value)) * 1024;
    }
    if limit < 0 {
        let Some(kbs) = ua.get_pint("Enter new bandwidth limit kb/s: ") else { return ua.finish(true) };
        limit = i64::from(kbs) * 1024;
    }

    let config = |f: &dyn Fn(&crate::director_config::DirectorConfig) -> Option<Resource>| {
        ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|c| f(&c))
    };
    // C++ `char Job[MAX_NAME_LENGTH]` is only set by a job selection; the
    // uninitialized buffer of the other paths is taken as "".
    let mut job = String::new();
    let mut client: Option<ClientResource> = None;
    let mut store: Option<StorageResource> = None;
    if find_arg_keyword(ast, &["job", "jobid", "ujobid", "all", "state"]).is_some_and(|i| i > 0) {
        let Some(selection) = crate::commands::cancel::select_jobs(&mut ua, ast, active, "limit") else {
            return ua.finish(true);
        };
        for id in selection {
            // get_jcr_by_id(): the job may have ended meanwhile.
            let Some(running) = active.iter().find(|j| j.id == id) else { continue };
            // jcr->max_bandwidth = limit: running jobs keep their own limits.
            job = running.unique_name.clone();
            let copy_or_migrate = config(&|c| c.find_job(&running.name)
                .filter(|j| matches!(j.job_type, JobType::Copy | JobType::Migrate))
                .map(|_| Resource::Job)).is_some();
            if copy_or_migrate {
                // jcr->dir_impl->res.read_storage
                store = config(&|c| c.find_storage(&running.storage).cloned().map(Resource::Storage))
                    .and_then(Resource::storage);
            } else {
                client = config(&|c| c.find_client(&running.client).cloned().map(Resource::Client))
                    .and_then(Resource::client);
            }
        }
    } else if ua_select::find_arg(ast, "storage").is_some() {
        store = ua_select::get_storage_resource(&mut ua, ast, false, false);
    } else if let Some(name) = ua_select::get_client_resource(&mut ua, ast) {
        client = config(&|c| c.find_client(&name).cloned().map(Resource::Client)).and_then(Resource::client);
    }

    let director = ctx.config.as_ref().and_then(|c| c.read().ok().map(|c| c.director.name.clone()))
        .unwrap_or_default();
    if let Some(client) = client {
        setbwlimit_filed(&mut ua, links, &client, &director, limit, &job);
    } else if let Some(store) = store {
        setbwlimit_stored(&mut ua, links, &store, &director, limit, &job);
    }
    ua.finish(true)
}

/// A resource copied out of the configuration.
enum Resource {
    Job,
    Client(ClientResource),
    Storage(StorageResource),
}

impl Resource {
    fn client(self) -> Option<ClientResource> {
        if let Resource::Client(c) = self { Some(c) } else { None }
    }
    fn storage(self) -> Option<StorageResource> {
        if let Resource::Storage(s) = self { Some(s) } else { None }
    }
}

/// SetbwlimitFiled() (ua_cmds.cc:842).
fn setbwlimit_filed(ua: &mut Ua, links: &mut dyn BwLinks, client: &ClientResource, director: &str,
    limit: i64, job: &str) {
    ua.send_msg(format!("Connecting to Client {} at {}:{}\n", client.name, client.address, client.port));
    // bandwidthcmd "setbandwidth=%lld Job=%s\n"
    let command = format!("setbandwidth={limit} Job={job}\n");
    match links.file_daemon(client, director, &command) {
        FdOutcome::NotConnected => ua.error_msg("Failed to connect to Client.\n"),
        FdOutcome::Reply(reply) if reply.as_deref() != Some("2000 OK Bandwidth\n") => {
            ua.error_msg("Failed to set bandwidth limit on Client.\n")
        }
        FdOutcome::NotSent | FdOutcome::Reply(_) => {
            ua.info_msg(format!("OK Limiting bandwidth to {}kb/s {job}\n", limit / 1024))
        }
    }
}

/// setbwlimit_stored() (ua_cmds.cc:886).
fn setbwlimit_stored(ua: &mut Ua, links: &mut dyn BwLinks, store: &StorageResource, director: &str,
    limit: i64, job: &str) {
    if store.protocol.is_ndmp() {
        ua.error_msg("Storage selected is NDMP storage which cannot have a bandwidth limit\n");
        return;
    }
    ua.send_msg(format!("Connecting to Storage daemon {} at {}:{}\n", store.name, store.address, store.port));
    if !links.storage_daemon(store, director) {
        ua.error_msg("Failed to connect to Storage daemon.\n");
        return;
    }
    ua.info_msg(format!("OK Limiting bandwidth to {}kb/s {job}\n", limit / 1024));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_executor::ApiMode;
    use crate::ua_select::tests::{context, scripted};

    const CONFIG: &str = "Director { Name = dir }\n\
        Client { Name = fd; Address = localhost }\n\
        Client { Name = fd2; Address = otherhost }\n\
        Storage { Name = sd; Address = localhost }\n\
        Job { Name = b; Client = fd }\n\
        Job { Name = c; Type = Copy; Client = fd }\n";

    #[derive(Default)]
    struct Fake {
        fd: Vec<String>,
        sd: Vec<String>,
        fd_outcome: Option<FdOutcome>,
    }

    impl BwLinks for Fake {
        fn file_daemon(&mut self, client: &ClientResource, _: &str, command: &str) -> FdOutcome {
            self.fd.push(format!("{}:{command}", client.name));
            self.fd_outcome.take().unwrap_or(FdOutcome::Reply(Some("2000 OK Bandwidth\n".into())))
        }
        fn storage_daemon(&mut self, store: &StorageResource, _: &str) -> bool {
            self.sd.push(store.name.clone());
            true
        }
    }

    fn run(line: &str, active: &[ActiveJob], fake: &mut Fake) -> CommandResult {
        let ctx = context(CONFIG, ApiMode::Human);
        setbandwidth_with(&crate::session::parse_cli_command(line), &ctx, active, fake)
    }

    fn running(id: u32, name: &str, client: &str, storage: &str) -> ActiveJob {
        ActiveJob { id, name: name.into(), unique_name: format!("{name}.2026-10-16_10.00.00_0{id}"),
            status: 'R', job_started: true, client: client.into(), storage: storage.into(), ..Default::default() }
    }

    #[test]
    fn atoi_like_c() {
        assert_eq!(atoi("12abc"), 12);
        assert_eq!(atoi(" -5"), -5);
        assert_eq!(atoi("abc"), 0);
        assert_eq!(atoi("99999999999999999999"), -1);
        assert_eq!(atoi("4294967297"), 1);
    }

    #[test]
    fn client_limit_is_sent_and_reported() {
        let mut fake = Fake::default();
        let r = run("setbandwidth client=fd limit=12", &[], &mut fake);
        assert_eq!(fake.fd, ["fd:setbandwidth=12288 Job=\n"]);
        assert!(r.text_output.contains("Connecting to Client fd at localhost:9102\n"), "{}", r.text_output);
        assert!(r.text_output.contains("OK Limiting bandwidth to 12kb/s \n"));
    }

    #[test]
    fn non_numeric_limit_is_zero_like_atoi() {
        let mut fake = Fake::default();
        run("setbandwidth client=fd limit=abc", &[], &mut fake);
        assert_eq!(fake.fd, ["fd:setbandwidth=0 Job=\n"]);
    }

    #[test]
    fn fd_failures_use_the_original_texts() {
        for (outcome, text) in [(FdOutcome::NotConnected, "Failed to connect to Client.\n"),
            (FdOutcome::Reply(Some("2901 Bad\n".into())), "Failed to set bandwidth limit on Client.\n"),
            (FdOutcome::Reply(None), "Failed to set bandwidth limit on Client.\n")] {
            let mut fake = Fake { fd_outcome: Some(outcome), ..Default::default() };
            let r = run("setbandwidth client=fd limit=1", &[], &mut fake);
            assert!(r.error_message.contains(text) || r.text_output.contains(text), "{r:?}");
            assert!(!r.text_output.contains("OK Limiting"));
        }
    }

    #[test]
    fn storage_connects_but_sends_nothing() {
        let mut fake = Fake::default();
        let r = run("setbandwidth storage=sd limit=3", &[], &mut fake);
        assert_eq!(fake.sd, ["sd"]);
        assert!(fake.fd.is_empty());
        assert!(r.text_output.contains("Connecting to Storage daemon sd at localhost:9103\n"));
        assert!(r.text_output.contains("OK Limiting bandwidth to 3kb/s \n"));
    }

    #[test]
    fn job_selection_without_jobs_never_connects() {
        for line in ["setbandwidth limit=1 all", "setbandwidth limit=1 state=running",
            "setbandwidth limit=1 jobid=42", "setbandwidth limit=1 ujobid=x"] {
            let mut fake = Fake::default();
            let r = run(line, &[], &mut fake);
            assert!(fake.fd.is_empty() && fake.sd.is_empty(), "{line}");
            assert!(r.text_output.contains("No Jobs running.\n"), "{line}: {r:?}");
        }
        let mut fake = Fake::default();
        let r = run("setbandwidth limit=1 jobid=42", &[], &mut fake);
        assert!(format!("{}{}", r.text_output, r.error_message)
            .contains("JobId 42 is not running. Use Job name to limit inactive jobs.\n"));
    }

    #[test]
    fn job_first_keyword_falls_back_to_the_client() {
        // FindArgKeyword() returns 0 for "job", which is not > 0.
        let mut fake = Fake::default();
        run("setbandwidth limit=1 job=b client=fd", &[running(5, "b", "fd", "sd")], &mut fake);
        assert_eq!(fake.fd, ["fd:setbandwidth=1024 Job=\n"]);
    }

    #[test]
    fn selected_jobs_limit_their_client_or_read_storage() {
        let jobs = [running(5, "b", "fd2", "sd"), running(6, "c", "fd", "sd")];
        let mut fake = Fake::default();
        let r = run("setbandwidth limit=2 jobid=5", &jobs, &mut fake);
        assert_eq!(fake.fd, ["fd2:setbandwidth=2048 Job=b.2026-10-16_10.00.00_05\n"]);
        assert!(r.text_output.contains("OK Limiting bandwidth to 2kb/s b.2026-10-16_10.00.00_05\n"));
        let mut fake = Fake::default();
        run("setbandwidth limit=2 jobid=6", &jobs, &mut fake);
        assert_eq!(fake.sd, ["sd"]);
        assert!(fake.fd.is_empty());
    }

    #[test]
    fn dialog_asks_limit_then_selects_client() {
        let ctx = context(CONFIG, ApiMode::Human);
        let line = |l: &str| crate::session::parse_cli_command(l);
        // Batch mode: no limit, nothing happens.
        let r = setbandwidth_with(&line("setbandwidth"), &ctx, &[], &mut Fake::default());
        assert!(r.success && r.text_output.is_empty() && r.error_message.is_empty());
        let (_, console) = scripted(&["."], || {
            setbandwidth_with(&line("setbandwidth"), &ctx, &[], &mut Fake::default())
        });
        assert_eq!(console.transcript(), "Enter new bandwidth limit kb/s: .\n");
        let mut fake = Fake::default();
        let (_, console) = scripted(&["x", "4", "2"], || {
            setbandwidth_with(&line("setbandwidth"), &ctx, &[], &mut fake)
        });
        assert!(console.transcript().starts_with(
            "Enter new bandwidth limit kb/s: x\nExpected a positive integer, got: x\n\
             Enter new bandwidth limit kb/s: 4\n"), "{}", console.transcript());
        assert_eq!(fake.fd, ["fd2:setbandwidth=4096 Job=\n"]);
        // Unknown client: original error, then the list (cancelled).
        let mut fake = Fake::default();
        let (_, console) = scripted(&["."], || {
            setbandwidth_with(&line("setbandwidth limit=1 client=nosuch"), &ctx, &[], &mut fake)
        });
        assert!(console.transcript().contains("Error: Client resource nosuch does not exist.\n"),
            "{}", console.transcript());
        assert!(fake.fd.is_empty());
    }
}
