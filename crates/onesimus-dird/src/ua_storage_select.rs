//! Storage selection and storage-daemon volume operations of the console.
//!
//! Port of C++ dird/ua_select.cc get_storage_resource() (:1320),
//! select_storage_resource() (:197), GetStorageDrive() (:1429) and
//! GetStorageSlot() (:1471) plus the SD side of dird/storage.cc
//! GetNumDrives() and dird/sd_cmds.cc open_sd_bsock(),
//! NativeGetNumDrives() and NativeAutochangerVolumeOperation(), as used by
//! `mount`, `unmount`/`umount`, `release` (ua_cmds.cc DoMountCmd) and the
//! storage branch of `status` (ua_status.cc StatusCmd).
//!
//! Upstream prints everything (SendMsg and ErrorMsg) into the console in
//! call order; the text is collected here in that order. Where upstream
//! waits for console input (several storages, several drives, autochanger
//! slot) the dialog of compatibility log C-011 is not available yet: those
//! paths go through [`prompt_select`] / [`prompt_input`], which print what
//! upstream prints before it waits and then cancel.

use std::collections::HashMap;
use std::sync::Mutex;

use bareos_bnet::{BnetMessage, BnetSignal};
use bareos_config::resources::StorageResource;

use crate::acl::{self, Kind};
use crate::command_ast::{Argument, CommandAst};
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;

/// Everything the selection needs, copied out of the configuration lock.
pub struct Env {
    /// Storages in resource order: (resource, autochanger, Storage ACL ok).
    pub storages: Vec<(StorageResource, bool, bool)>,
    /// Running jobs: (JobId, unique Job name, write storage name).
    pub running: Vec<(u32, String, String)>,
    /// `!ua->api`: DoPrompt reports an automatic selection.
    pub human: bool,
}

impl Env {
    pub fn load(ctx: &ExecContext) -> Option<Self> {
        let config = ctx.config.as_ref()?.read().ok()?;
        let storages = config.storages.iter()
            .map(|s| (s.clone(), is_autochanger(&config, &s.name), acl::access(ctx, Kind::Storage, &s.name)))
            .collect();
        // jcr->dir_impl->res.write_storage: Onesimus keeps no per-job storage
        // record; the job resource's Storage is what the job writes to.
        let running = ctx.job_manager.as_ref()
            .and_then(|m| m.lock().ok().map(|m| m.active_jobs()))
            .unwrap_or_default().into_iter()
            .map(|job| {
                let storage = config.find_job(&job.name).map(|j| j.storage.clone()).unwrap_or_default();
                (job.id, job.unique_name, storage)
            })
            .collect();
        Some(Self { storages, running, human: ctx.api_mode == ApiMode::Human })
    }

    /// `ua->GetStoreResWithName()` (ua_acl.cc:279): Storage ACL, exact name.
    fn get_store_res_with_name(&self, name: &str) -> Option<&StorageResource> {
        self.storages.iter().find(|s| s.0.name == name && s.2).map(|s| &s.0)
    }

    fn acl_ok(&self, name: &str) -> bool {
        self.storages.iter().any(|s| s.0.name == name && s.2)
    }

    pub fn autochanger(&self, name: &str) -> bool {
        self.storages.iter().any(|s| s.0.name == name && s.1)
    }
}

/// `Auto Changer = yes` in the Storage resource (dird_conf.cc "autochanger").
fn is_autochanger(config: &crate::director_config::DirectorConfig, name: &str) -> bool {
    config.raw.iter()
        .find(|r| r.norm_type() == "storage" && r.name() == Some(name))
        .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == "autochanger"))
        .and_then(|d| d.first())
        .is_some_and(|v| bareos_config::values::parse_bool(v).is_ok_and(|(_, on)| on))
}

/// `get_storage_resource(ua, use_default, autochangers_only)` for a session.
/// Ok carries the storage and the text printed while selecting it; Err the
/// result to return when no storage was selected.
pub fn get_storage_resource(ast: &CommandAst, ctx: &ExecContext, use_default: bool, autochangers_only: bool)
    -> Result<(StorageResource, String), CommandResult> {
    let Some(env) = Env::load(ctx) else {
        return Err(CommandResult::error("No config\n".into()));
    };
    let mut out = String::new();
    match select(&ast.arguments, &env, use_default, autochangers_only, &mut out) {
        Some(store) => Ok((store, out)),
        // A waiting dialog prompt ends without newline (nothing failed);
        // otherwise the last line is the ErrorMsg that ended the selection.
        None if !out.ends_with('\n') => Err(CommandResult::ok(out)),
        None => {
            let split = out[..out.len() - 1].rfind('\n').map_or(0, |i| i + 1);
            let mut result = CommandResult::error(out[split..].to_string());
            out.truncate(split);
            result.text_output = out;
            Err(result)
        }
    }
}

/// C++ get_storage_resource() (ua_select.cc:1320) on the arguments after
/// the command name.
pub fn select(args: &[Argument], env: &Env, use_default: bool, autochangers_only: bool, out: &mut String)
    -> Option<StorageResource> {
    let mut store_name: Option<String> = None;
    let mut store: Option<String> = None;

    for arg in args {
        // Ignore any zapped keyword.
        if arg.key.is_empty() { continue; }
        if use_default && arg.value.is_none() {
            const IGNORED: [&str; 6] = ["barcode", "barcodes", "encrypt", "scan", "slots", "alldrives"];
            if IGNORED.iter().any(|k| k.eq_ignore_ascii_case(&arg.key)) { continue; }
            // Default argument is storage
            if store_name.is_some() {
                out.push_str("Storage name given twice.\n");
                return None;
            }
            if arg.key.starts_with('?') {
                store_name = Some(String::new());
                break;
            }
            store_name = Some(arg.key.clone());
        } else {
            let key = arg.key.to_ascii_lowercase();
            match key.as_str() {
                "storage" | "sd" => {
                    store_name = arg.value.clone();
                    break;
                }
                "jobid" => {
                    let jobid = arg.value.as_deref().map(str_to_int64).unwrap_or(0);
                    if jobid <= 0 {
                        out.push_str(&format!("Expecting jobid=nn command, got: {}\n", arg.key));
                        return None;
                    }
                    // jobid is an int in upstream.
                    let jobid = jobid as i32;
                    match env.running.iter().find(|j| j.0 as i64 == jobid as i64) {
                        Some(job) => store = Some(job.2.clone()),
                        None => {
                            out.push_str(&format!("JobId {jobid} is not running.\n"));
                            return None;
                        }
                    }
                    break;
                }
                "job" | "jobname" | "ujobid" => {
                    let Some(name) = arg.value.as_deref() else {
                        let what = if key == "ujobid" { "ujobid" } else { "job" };
                        out.push_str(&format!("Expecting {what}=xxx, got: {}.\n", arg.key));
                        return None;
                    };
                    let job = if key == "ujobid" {
                        env.running.iter().find(|j| j.1 == name)
                    } else {
                        env.running.iter().find(|j| j.1.starts_with(name))
                    };
                    match job {
                        Some(job) => store = Some(job.2.clone()),
                        None => {
                            out.push_str(&format!("Job \"{name}\" is not running.\n"));
                            return None;
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
    }

    let mut found = store.and_then(|name| env.storages.iter().find(|s| s.0.name == name).map(|s| s.0.clone()));
    if found.as_ref().is_some_and(|s| !env.acl_ok(&s.name)) {
        found = None;
    }
    if found.is_none() {
        if let Some(name) = store_name.as_deref().filter(|n| !n.is_empty()) {
            found = env.get_store_res_with_name(name).cloned();
            if found.is_none() {
                out.push_str(&format!("Storage resource \"{name}\": not found\n"));
            }
        }
    }
    // No keywords found, so present a selection list
    if found.is_none() {
        found = select_storage_resource(env, autochangers_only, out);
    }
    found
}

/// C++ str_to_int64(): optional sign and leading digits, otherwise 0.
fn str_to_int64(text: &str) -> i64 {
    let text = text.trim_start();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let value = digits.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |acc, d| acc.wrapping_mul(10).wrapping_add((d - b'0') as i64));
    if negative { -value } else { value }
}

/// C++ select_storage_resource() (ua_select.cc:197).
pub fn select_storage_resource(env: &Env, autochanger_only: bool, out: &mut String) -> Option<StorageResource> {
    let header = if autochanger_only {
        "The defined Autochanger Storage resources are:\n"
    } else {
        "The defined Storage resources are:\n"
    };
    let names: Vec<String> = crate::director_config::sort_case_insensitive(env.storages.iter()
        .filter(|s| s.2 && (!autochanger_only || s.1))
        .map(|s| s.0.name.as_str()))
        .into_iter().map(str::to_string).collect();
    let name = prompt_select(env.human, header, &names, "Storage", "Select Storage resource", out)?;
    env.get_store_res_with_name(&name).cloned()
}

/// DoPrompt (ua_select.cc:1210) over a prepared list.
///
/// One entry is selected automatically, an empty list fails. Several
/// entries need the console dialog (compatibility log C-011): until it
/// exists, this hook prints the list and the prompt like upstream and then
/// cancels the selection.
pub fn prompt_select(human: bool, header: &str, items: &[String], automsg: &str, msg: &str, out: &mut String)
    -> Option<String> {
    if items.len() == 1 {
        if human {
            out.push_str(&format!("Automatically selected {automsg}: {}\n", items[0]));
        }
        return Some(items[0].clone());
    }
    out.push_str(header);
    if items.is_empty() {
        out.push_str(&format!("Selection list for \"{automsg}\" is empty!\n"));
        return None;
    }
    // FormatPrompts() short form: "%*d: %s\n" with the width of num_prompts.
    let width = (items.len() + 1).to_string().len();
    for (index, item) in items.iter().enumerate() {
        out.push_str(&format!("{:>width$}: {item}\n", index + 1));
    }
    out.push_str(&format!("{msg} (1-{}): ", items.len()));
    None
}

/// GetCmd() (ua_input.cc): needs the console dialog (C-011). Prints the
/// prompt and reports no input. Upstream continues without a slot only when
/// reading the answer fails; here nobody could answer, so callers stop.
pub fn prompt_input(prompt: &str, out: &mut String) -> Option<String> {
    out.push_str(prompt);
    None
}

/// C++ FindArgWithValue() + atoi().
fn arg_number(args: &[Argument], key: &str) -> Option<i16> {
    args.iter().find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref()).map(|v| str_to_int64(v) as i32 as i16)
}

/// Transport to a Storage daemon: one connection per call, like
/// open_sd_bsock()/CloseSdBsock(). Err means the connection failed.
pub trait SdLink {
    /// Connect, send `command` and return every message until a signal.
    fn exchange(&mut self, store: &StorageResource, command: &str) -> Result<Vec<String>, String>;
}

/// The real Storage daemon connection (TLS-PSK as Director).
pub struct NetSd {
    pub director: String,
}

impl SdLink for NetSd {
    fn exchange(&mut self, store: &StorageResource, command: &str) -> Result<Vec<String>, String> {
        let mut sd = bareos_stored::connect::connect_sd(&store.address, store.port, &store.password, &self.director)?;
        let mut messages = Vec::new();
        // fsend errors are silent upstream; the receive loop then ends.
        if sd.send_str(command).is_ok() {
            loop {
                match sd.recv() {
                    Ok(BnetMessage::Data(data)) => messages.push(String::from_utf8_lossy(&data).into_owned()),
                    Ok(BnetMessage::Signal(BnetSignal::Heartbeat)) => {
                        let _ = sd.send_signal(BnetSignal::HeartbeatResponse);
                    }
                    Ok(BnetMessage::Signal(_)) | Err(_) => break,
                }
            }
        }
        let _ = sd.send_signal(BnetSignal::Terminate);
        Ok(messages)
    }
}

/// StorageResource::dev_name(): the first Device of the storage.
fn dev_name(store: &StorageResource) -> String {
    let first = store.device.split(',').next().unwrap_or("").trim().trim_matches('"');
    // BashSpaces()
    first.replace(' ', "\u{1}")
}

/// open_sd_bsock() (sd_cmds.cc:165) + one exchange; None if the connection
/// failed (the error has been printed).
fn open_and_exchange(sd: &mut impl SdLink, store: &StorageResource, command: &str, out: &mut String)
    -> Option<Vec<String>> {
    out.push_str(&format!("Connecting to Storage daemon {} at {}:{} ...\n", store.name, store.address, store.port));
    match sd.exchange(store, command) {
        Ok(messages) => Some(messages),
        Err(_) => {
            out.push_str("Failed to connect to Storage daemon.\n");
            None
        }
    }
}

/// runtime_storage_status->drives: the drive count is asked once per storage.
static DRIVES: Mutex<Option<HashMap<String, i16>>> = Mutex::new(None);

/// GetNumDrives() (storage.cc:668) / NativeGetNumDrives() (sd_cmds.cc:558).
fn get_num_drives(sd: &mut impl SdLink, store: &StorageResource, out: &mut String) -> i16 {
    if let Some(drives) = DRIVES.lock().ok().and_then(|d| d.as_ref()?.get(&store.name).copied()) {
        if drives > 0 { return drives; }
    }
    let mut drives = 0;
    if let Some(messages) = open_and_exchange(sd, store, &format!("autochanger drives {}\n", dev_name(store)), out) {
        for message in messages {
            match message.strip_prefix("drives=").and_then(|rest| {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<i16>().ok()
            }) {
                Some(n) => { drives = n; break; }
                None => out.push_str(&message),
            }
        }
    }
    if let Ok(mut cache) = DRIVES.lock() {
        cache.get_or_insert_with(HashMap::new).insert(store.name.clone(), drives);
    }
    drives
}

/// GetStorageDrive() (ua_select.cc:1429); None is kInvalidDriveNumber.
fn get_storage_drive(args: &[Argument], env: &Env, sd: &mut impl SdLink, store: &StorageResource, out: &mut String)
    -> Option<i16> {
    if let Some(drive) = arg_number(args, "drive") {
        return Some(drive);
    }
    if !env.autochanger(&store.name) {
        return Some(0);
    }
    let drives = get_num_drives(sd, store, out);
    if drives == 1 {
        return Some(0);
    }
    let items: Vec<String> = (0..drives.max(0)).map(|n| format!("Drive {n}")).collect();
    let choice = prompt_select(env.human, "Select Drive:\n", &items, "Drive", "Select drive", out)?;
    choice.strip_prefix("Drive ").and_then(|n| n.parse().ok())
}

/// GetStorageSlot() (ua_select.cc:1471); -1 means no slot, None that the
/// slot dialog is not available (see [`prompt_input`]).
fn get_storage_slot(args: &[Argument], env: &Env, store: &StorageResource, out: &mut String) -> Option<i16> {
    if let Some(slot) = arg_number(args, "slot") {
        return Some(slot);
    }
    if env.autochanger(&store.name) {
        return prompt_input("Enter autochanger slot: ", out).map(|answer| str_to_int64(&answer) as i32 as i16);
    }
    Some(-1)
}

/// NativeAutochangerVolumeOperation() (sd_cmds.cc:793): the SD answer is
/// relayed unchanged.
fn volume_operation(sd: &mut impl SdLink, store: &StorageResource, operation: &str, drive: i16, slot: i16,
    out: &mut String) {
    // IsSlotNumberValid(): slots are counted from 1.
    let command = if slot > 0 {
        format!("{operation} {} drive={drive} slot={slot}\n", dev_name(store))
    } else {
        format!("{operation} {} drive={drive}\n", dev_name(store))
    };
    if let Some(messages) = open_and_exchange(sd, store, &command, out) {
        for message in messages {
            out.push_str(&message);
        }
    }
}

/// C++ DoMountCmd() (ua_cmds.cc:2498) after OpenClientDb(): `cmd` is
/// "mount", "unmount" or "release". The command always succeeds.
pub fn do_mount_cmd(args: &[Argument], env: &Env, sd: &mut impl SdLink, cmd: &str) -> String {
    let mut out = String::new();
    let do_alldrives = (cmd == "release" || cmd == "unmount")
        && args.iter().any(|a| a.key.eq_ignore_ascii_case("alldrives"));

    let Some(store) = select(args, env, true, false, &mut out) else { return out };
    let mut drive = -1;
    if !do_alldrives {
        match get_storage_drive(args, env, sd, &store, &mut out) {
            Some(d) => drive = d,
            None => return out,
        }
    }
    let mut slot = -1;
    if cmd == "mount" {
        match get_storage_slot(args, env, &store, &mut out) {
            Some(s) => slot = s,
            None => return out,
        }
    }

    if !do_alldrives {
        volume_operation(sd, &store, cmd, drive, slot, &mut out);
    } else {
        let drives = get_num_drives(sd, &store, &mut out);
        for i in 0..drives.max(0) {
            volume_operation(sd, &store, cmd, i, slot, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arg(key: &str, value: Option<&str>) -> Argument {
        Argument { key: key.into(), value: value.map(str::to_string) }
    }

    fn parse(line: &str) -> Vec<Argument> {
        line.split_whitespace().map(|w| match w.split_once('=') {
            Some((k, v)) => arg(k, Some(v)),
            None => arg(w, None),
        }).collect()
    }

    fn storage(name: &str, device: &str) -> StorageResource {
        StorageResource { name: name.into(), address: "sd.example".into(), port: 9103,
            device: device.into(), ..Default::default() }
    }

    fn env(storages: &[(&str, bool, bool)]) -> Env {
        Env {
            storages: storages.iter().map(|(n, ac, ok)| (storage(n, &format!("{n}Dev")), *ac, *ok)).collect(),
            running: vec![(7, "Backup.2026-10-15_21.00.00_03".into(), "Tape".into())],
            human: true,
        }
    }

    /// Fake SD: records commands and answers from a script.
    struct FakeSd { sent: Vec<String>, answer: fn(&str) -> Result<Vec<String>, String> }

    impl SdLink for FakeSd {
        fn exchange(&mut self, store: &StorageResource, command: &str) -> Result<Vec<String>, String> {
            self.sent.push(format!("{}: {command}", store.name));
            (self.answer)(command)
        }
    }

    fn fake(answer: fn(&str) -> Result<Vec<String>, String>) -> FakeSd { FakeSd { sent: vec![], answer } }

    fn sel(line: &str, env: &Env, use_default: bool) -> (Option<String>, String) {
        let mut out = String::new();
        let store = select(&parse(line), env, use_default, false, &mut out);
        (store.map(|s| s.name), out)
    }

    #[test]
    fn only_permitted_storage_is_selected_automatically() {
        let e = env(&[("File", false, true), ("Hidden", false, false)]);
        assert_eq!(sel("", &e, true), (Some("File".into()), "Automatically selected Storage: File\n".into()));
        assert_eq!(sel("nosuch", &e, false), (Some("File".into()), "Automatically selected Storage: File\n".into()));
        let mut quiet = env(&[("File", false, true)]);
        quiet.human = false;
        assert_eq!(sel("", &quiet, true), (Some("File".into()), String::new()));
    }

    #[test]
    fn argument_forms() {
        let e = env(&[("File", false, true), ("Tape", true, true), ("Hidden", false, false)]);
        assert_eq!(sel("storage=Tape", &e, true).0.as_deref(), Some("Tape"));
        assert_eq!(sel("sd=File", &e, false).0.as_deref(), Some("File"));
        assert_eq!(sel("Tape drive=1", &e, true).0.as_deref(), Some("Tape"));
        assert_eq!(sel("barcodes slots alldrives Tape", &e, true).0.as_deref(), Some("Tape"));
        assert_eq!(sel("jobid=7", &e, true).0.as_deref(), Some("Tape"));
        assert_eq!(sel("job=Backup.2026", &e, true).0.as_deref(), Some("Tape"));
        assert_eq!(sel("ujobid=Backup.2026-10-15_21.00.00_03", &e, true).0.as_deref(), Some("Tape"));
        assert_eq!(sel("File Tape", &e, true), (None, "Storage name given twice.\n".into()));
        assert_eq!(sel("jobid=x", &e, true), (None, "Expecting jobid=nn command, got: jobid\n".into()));
        assert_eq!(sel("jobid=8", &e, true), (None, "JobId 8 is not running.\n".into()));
        assert_eq!(sel("job", &e, false), (None, "Expecting job=xxx, got: job.\n".into()));
        assert_eq!(sel("ujobid=Backup", &e, true), (None, "Job \"Backup\" is not running.\n".into()));
    }

    #[test]
    fn unknown_or_denied_name_falls_back_to_the_selection_list() {
        let e = env(&[("File", false, true), ("Hidden", false, false)]);
        assert_eq!(sel("storage=Hidden", &e, true), (Some("File".into()),
            "Storage resource \"Hidden\": not found\nAutomatically selected Storage: File\n".into()));
        // With use_default a bare "storage" keyword is a storage name.
        assert_eq!(sel("storage", &e, true).1,
            "Storage resource \"storage\": not found\nAutomatically selected Storage: File\n");
        assert_eq!(sel("storage", &e, false).1, "Automatically selected Storage: File\n");
        assert_eq!(sel("?", &e, true).1, "Automatically selected Storage: File\n");
    }

    #[test]
    fn several_or_no_storages_need_the_dialog() {
        let e = env(&[("b", false, true), ("A", false, true)]);
        assert_eq!(sel("", &e, true), (None,
            "The defined Storage resources are:\n1: A\n2: b\nSelect Storage resource (1-2): ".into()));
        let none = env(&[("Hidden", false, false)]);
        assert_eq!(sel("", &none, true), (None,
            "The defined Storage resources are:\nSelection list for \"Storage\" is empty!\n".into()));
        let mut out = String::new();
        let ac = select_storage_resource(&env(&[("File", false, true), ("Tape", true, true)]), true, &mut out);
        assert_eq!((ac.map(|s| s.name), out), (Some("Tape".into()), "Automatically selected Storage: Tape\n".into()));
    }

    #[test]
    fn mount_release_and_unmount_talk_to_the_selected_sd() {
        let e = env(&[("File", false, true)]);
        let mut sd = fake(|_| Ok(vec!["3906 File device \"FileDev\" is always mounted.\n".into()]));
        assert_eq!(do_mount_cmd(&[], &e, &mut sd, "mount"),
            "Automatically selected Storage: File\nConnecting to Storage daemon File at sd.example:9103 ...\n\
             3906 File device \"FileDev\" is always mounted.\n");
        assert_eq!(sd.sent, vec!["File: mount FileDev drive=0\n"]);

        let mut sd = fake(|_| Ok(vec![]));
        do_mount_cmd(&parse("File drive=2 slot=5"), &e, &mut sd, "mount");
        do_mount_cmd(&parse("storage=File slot=0"), &e, &mut sd, "mount");
        do_mount_cmd(&parse("File slot=3"), &e, &mut sd, "release");
        do_mount_cmd(&parse("umountish"), &e, &mut sd, "unmount");
        assert_eq!(sd.sent, vec!["File: mount FileDev drive=2 slot=5\n", "File: mount FileDev drive=0\n",
            "File: release FileDev drive=0\n", "File: unmount FileDev drive=0\n"]);
    }

    #[test]
    fn device_names_are_bashed_and_failures_reported() {
        let mut e = env(&[("File", false, true)]);
        e.storages[0].0.device = "My Device".into();
        let mut sd = fake(|_| Err("refused".into()));
        assert_eq!(do_mount_cmd(&parse("storage=File"), &e, &mut sd, "release"),
            "Connecting to Storage daemon File at sd.example:9103 ...\nFailed to connect to Storage daemon.\n");
        assert_eq!(sd.sent, vec!["File: release My\u{1}Device drive=0\n"]);
    }

    #[test]
    fn alldrives_asks_the_sd_for_its_drives() {
        let e = env(&[("Changer13", true, true)]);
        let mut sd = fake(|cmd| Ok(if cmd.starts_with("autochanger") { vec!["drives=2\n".into()] }
            else { vec![format!("3922 {cmd}")] }));
        let out = do_mount_cmd(&parse("Changer13 alldrives"), &e, &mut sd, "unmount");
        assert_eq!(sd.sent, vec!["Changer13: autochanger drives Changer13Dev\n",
            "Changer13: unmount Changer13Dev drive=0\n", "Changer13: unmount Changer13Dev drive=1\n"]);
        assert_eq!(out.matches("Connecting to Storage daemon Changer13").count(), 3);
        // Cached: the next command needs no drive query, two drives → dialog.
        let mut sd = fake(|_| Ok(vec![]));
        let out = do_mount_cmd(&parse("Changer13"), &e, &mut sd, "release");
        assert!(sd.sent.is_empty());
        assert_eq!(out, "Select Drive:\n1: Drive 0\n2: Drive 1\nSelect drive (1-2): ");
    }

    #[test]
    fn autochanger_with_one_drive_asks_for_the_slot_on_mount() {
        let e = env(&[("Changer11", true, true)]);
        let mut sd = fake(|cmd| Ok(if cmd.starts_with("autochanger") { vec!["3998 note\n".into(), "drives=1\n".into()] }
            else { vec![] }));
        let out = do_mount_cmd(&parse("Changer11"), &e, &mut sd, "mount");
        assert_eq!(out, "Connecting to Storage daemon Changer11 at sd.example:9103 ...\n3998 note\n\
            Enter autochanger slot: ");
        assert_eq!(sd.sent, vec!["Changer11: autochanger drives Changer11Dev\n"]);
        let mut sd = fake(|_| Ok(vec![]));
        do_mount_cmd(&parse("Changer11 slot=4"), &e, &mut sd, "mount");
        assert_eq!(sd.sent, vec!["Changer11: mount Changer11Dev drive=0 slot=4\n"]);
    }

    #[test]
    fn str_to_int64_like_upstream() {
        assert_eq!((str_to_int64("12x"), str_to_int64("-3"), str_to_int64("abc")), (12, -3, 0));
    }
}
