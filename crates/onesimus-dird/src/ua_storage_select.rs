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
//! Messages and the dialogs (several storages, several drives, autochanger
//! slot) go through the command's [`Ua`] (compatibility log C-011).

use std::collections::HashMap;
use std::sync::Mutex;

use bareos_bnet::{BnetMessage, BnetSignal};
use bareos_config::resources::StorageResource;

use crate::acl::{self, Kind};
use crate::command_ast::Argument;
use crate::command_executor::ExecContext;
use crate::ua_select::Ua;

/// Everything the selection needs, copied out of the configuration lock.
pub struct Env {
    /// Storages in resource order: (resource, autochanger, Storage ACL ok).
    pub storages: Vec<(StorageResource, bool, bool)>,
    /// Running jobs: (JobId, unique Job name, write storage name).
    pub running: Vec<(u32, String, String)>,
}

impl Env {
    pub fn load(ctx: &ExecContext) -> Option<Self> {
        let config = ctx.config.as_ref()?.read().ok()?;
        let storages = config.storages.iter()
            .map(|s| (s.clone(), crate::ua_select::is_autochanger(&config, &s.name),
                acl::access(ctx, Kind::Storage, &s.name)))
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
        Some(Self { storages, running })
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

/// C++ get_storage_resource() (ua_select.cc:1320) on the arguments after
/// the command name.
pub fn select(ua: &mut Ua, args: &[Argument], env: &Env, use_default: bool, autochangers_only: bool)
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
                ua.error_msg("Storage name given twice.\n");
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
                        ua.error_msg(format!("Expecting jobid=nn command, got: {}\n", arg.key));
                        return None;
                    }
                    // jobid is an int in upstream.
                    let jobid = jobid as i32;
                    match env.running.iter().find(|j| j.0 as i64 == jobid as i64) {
                        Some(job) => store = Some(job.2.clone()),
                        None => {
                            ua.error_msg(format!("JobId {jobid} is not running.\n"));
                            return None;
                        }
                    }
                    break;
                }
                "job" | "jobname" | "ujobid" => {
                    let Some(name) = arg.value.as_deref() else {
                        let what = if key == "ujobid" { "ujobid" } else { "job" };
                        ua.error_msg(format!("Expecting {what}=xxx, got: {}.\n", arg.key));
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
                            ua.error_msg(format!("Job \"{name}\" is not running.\n"));
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
            // GetStoreResWithName(name) audits a denied name (ua_acl.cc:309).
            if acl::access_audited(ua.ctx, Kind::Storage, name) {
                found = env.get_store_res_with_name(name).cloned();
            }
            if found.is_none() {
                ua.error_msg(format!("Storage resource \"{name}\": not found\n"));
            }
        }
    }
    // No keywords found, so present a selection list
    if found.is_none() {
        found = select_storage_resource(ua, env, autochangers_only);
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
pub fn select_storage_resource(ua: &mut Ua, env: &Env, autochanger_only: bool) -> Option<StorageResource> {
    ua.start_prompt(if autochanger_only {
        "The defined Autochanger Storage resources are:\n"
    } else {
        "The defined Storage resources are:\n"
    });
    for name in crate::director_config::sort_case_insensitive(env.storages.iter()
        .filter(|s| s.2 && (!autochanger_only || s.1))
        .map(|s| s.0.name.as_str())) {
        ua.add_prompt(name);
    }
    let (_, name) = ua.do_prompt("Storage", "Select Storage resource")?;
    env.get_store_res_with_name(&name).cloned()
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
fn open_and_exchange(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, command: &str)
    -> Option<Vec<String>> {
    ua.send_msg(format!("Connecting to Storage daemon {} at {}:{} ...\n", store.name, store.address, store.port));
    match sd.exchange(store, command) {
        Ok(messages) => Some(messages),
        Err(_) => {
            ua.error_msg("Failed to connect to Storage daemon.\n");
            None
        }
    }
}

/// runtime_storage_status->drives: the drive count is asked once per storage.
static DRIVES: Mutex<Option<HashMap<String, i16>>> = Mutex::new(None);

/// GetNumDrives() (storage.cc:668) / NativeGetNumDrives() (sd_cmds.cc:558).
fn get_num_drives(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource) -> i16 {
    if let Some(drives) = DRIVES.lock().ok().and_then(|d| d.as_ref()?.get(&store.name).copied()) {
        if drives > 0 { return drives; }
    }
    let mut drives = 0;
    if let Some(messages) = open_and_exchange(ua, sd, store, &format!("autochanger drives {}\n", dev_name(store))) {
        for message in messages {
            match message.strip_prefix("drives=").and_then(|rest| {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<i16>().ok()
            }) {
                Some(n) => { drives = n; break; }
                None => ua.send_msg(message),
            }
        }
    }
    if let Ok(mut cache) = DRIVES.lock() {
        cache.get_or_insert_with(HashMap::new).insert(store.name.clone(), drives);
    }
    drives
}

/// GetStorageDrive() (ua_select.cc:1429); None is kInvalidDriveNumber.
fn get_storage_drive(ua: &mut Ua, args: &[Argument], env: &Env, sd: &mut impl SdLink, store: &StorageResource)
    -> Option<i16> {
    if let Some(drive) = arg_number(args, "drive") {
        return Some(drive);
    }
    if !env.autochanger(&store.name) {
        return Some(0);
    }
    let drives = get_num_drives(ua, sd, store);
    if drives == 1 {
        return Some(0);
    }
    ua.start_prompt("Select Drive:\n");
    for n in 0..drives.max(0) {
        ua.add_prompt(&format!("Drive {n}"));
    }
    let (_, choice) = ua.do_prompt("Drive", "Select drive")?;
    choice.strip_prefix("Drive ").and_then(|n| n.parse().ok())
}

/// GetStorageSlot() (ua_select.cc:1471); -1 means no slot. None: the slot
/// question was cancelled. Upstream then continues without a slot; here a
/// cancelled dialog ends the command without contacting the SD.
fn get_storage_slot(ua: &mut Ua, args: &[Argument], env: &Env, store: &StorageResource) -> Option<i16> {
    if let Some(slot) = arg_number(args, "slot") {
        return Some(slot);
    }
    if env.autochanger(&store.name) {
        return ua.get_cmd("Enter autochanger slot: ").map(|answer| str_to_int64(&answer) as i32 as i16);
    }
    Some(-1)
}

/// NativeAutochangerVolumeOperation() (sd_cmds.cc:793): the SD answer is
/// relayed unchanged.
fn volume_operation(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, operation: &str, drive: i16,
    slot: i16) {
    // IsSlotNumberValid(): slots are counted from 1.
    let command = if slot > 0 {
        format!("{operation} {} drive={drive} slot={slot}\n", dev_name(store))
    } else {
        format!("{operation} {} drive={drive}\n", dev_name(store))
    };
    if let Some(messages) = open_and_exchange(ua, sd, store, &command) {
        for message in messages {
            ua.send_msg(message);
        }
    }
}

/// C++ DoMountCmd() (ua_cmds.cc:2498) after OpenClientDb(): `cmd` is
/// "mount", "unmount" or "release". The command always succeeds.
pub fn do_mount_cmd(ua: &mut Ua, args: &[Argument], env: &Env, sd: &mut impl SdLink, cmd: &str) {
    let do_alldrives = (cmd == "release" || cmd == "unmount")
        && args.iter().any(|a| a.key.eq_ignore_ascii_case("alldrives"));

    let Some(store) = select(ua, args, env, true, false) else { return };
    let mut drive = -1;
    if !do_alldrives {
        match get_storage_drive(ua, args, env, sd, &store) {
            Some(d) => drive = d,
            None => return,
        }
    }
    let mut slot = -1;
    if cmd == "mount" {
        match get_storage_slot(ua, args, env, &store) {
            Some(s) => slot = s,
            None => return,
        }
    }

    if !do_alldrives {
        volume_operation(ua, sd, &store, cmd, drive, slot);
    } else {
        let drives = get_num_drives(ua, sd, &store);
        for i in 0..drives.max(0) {
            volume_operation(ua, sd, &store, cmd, i, slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_executor::ApiMode;
    use crate::command_queue::CommandResult;
    use crate::ua_select::tests::{context, scripted};

    const CONFIG: &str = "Director { Name=dir; Password=secret }";

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

    /// Output of a command without console in emission order.
    fn output(result: CommandResult) -> String {
        if result.errors_first {
            format!("{}{}", result.error_message, result.text_output)
        } else {
            format!("{}{}", result.text_output, result.error_message)
        }
    }

    fn sel_in(mode: ApiMode, line: &str, env: &Env, use_default: bool) -> (Option<String>, String) {
        let ctx = context(CONFIG, mode);
        let mut ua = Ua::new(&ctx);
        let store = select(&mut ua, &parse(line), env, use_default, false);
        (store.map(|s| s.name), output(ua.finish(true)))
    }

    fn sel(line: &str, env: &Env, use_default: bool) -> (Option<String>, String) {
        sel_in(ApiMode::Human, line, env, use_default)
    }

    fn mount(line: &str, env: &Env, sd: &mut FakeSd, cmd: &str) -> String {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut ua = Ua::new(&ctx);
        do_mount_cmd(&mut ua, &parse(line), env, sd, cmd);
        output(ua.finish(true))
    }

    /// do_mount_cmd with a console answering `answers`; the transcript.
    fn mount_dialog(answers: &[&str], line: &str, env: &Env, sd: &mut FakeSd, cmd: &str) -> String {
        let ctx = context(CONFIG, ApiMode::Human);
        let (_, console) = scripted(answers, || {
            let mut ua = Ua::new(&ctx);
            do_mount_cmd(&mut ua, &parse(line), env, sd, cmd);
            ua.finish(true)
        });
        console.transcript()
    }

    #[test]
    fn only_permitted_storage_is_selected_automatically() {
        let e = env(&[("File", false, true), ("Hidden", false, false)]);
        assert_eq!(sel("", &e, true), (Some("File".into()), "Automatically selected Storage: File\n".into()));
        assert_eq!(sel("nosuch", &e, false), (Some("File".into()), "Automatically selected Storage: File\n".into()));
        let quiet = env(&[("File", false, true)]);
        assert_eq!(sel_in(ApiMode::Api, "", &quiet, true), (Some("File".into()), String::new()));
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
    fn several_storages_are_offered_in_the_dialog() {
        let e = env(&[("b", false, true), ("A", false, true)]);
        let ctx = context(CONFIG, ApiMode::Human);
        let (store, console) = scripted(&["2"], || select(&mut Ua::new(&ctx), &[], &e, true, false));
        assert_eq!(store.map(|s| s.name).as_deref(), Some("b"));
        assert_eq!(console.transcript(),
            "The defined Storage resources are:\n1: A\n2: b\nSelect Storage resource (1-2): 2\n");
        let (store, console) = scripted(&["."], || {
            let mut ua = Ua::new(&ctx);
            let store = select(&mut ua, &[], &e, true, false);
            ua.finish(true);
            store
        });
        assert!(store.is_none());
        assert_eq!(console.transcript(), "The defined Storage resources are:\n1: A\n2: b\n\
            Select Storage resource (1-2): .\nSelection aborted, nothing done.\n");
        // Without a console (batch mode) the choices are listed.
        assert_eq!(sel("", &e, true), (None, "The defined Storage resources are:\n1: A\n2: b\n\
            Your request has multiple choices for \"Storage\". Selection is not possible in batch mode.\n".into()));
        let none = env(&[("Hidden", false, false)]);
        let ((store, result), _) = scripted(&[], || {
            let mut ua = Ua::new(&ctx);
            let store = select(&mut ua, &[], &none, true, false);
            (store, ua.finish(false))
        });
        assert!(store.is_none());
        assert_eq!(output(result), "The defined Storage resources are:\nSelection list for \"Storage\" is empty!\n");
        let mut ua = Ua::new(&ctx);
        let ac = select_storage_resource(&mut ua, &env(&[("File", false, true), ("Tape", true, true)]), true);
        assert_eq!((ac.map(|s| s.name), output(ua.finish(true))),
            (Some("Tape".into()), "Automatically selected Storage: Tape\n".into()));
    }

    #[test]
    fn mount_release_and_unmount_talk_to_the_selected_sd() {
        let e = env(&[("File", false, true)]);
        let mut sd = fake(|_| Ok(vec!["3906 File device \"FileDev\" is always mounted.\n".into()]));
        assert_eq!(mount("", &e, &mut sd, "mount"),
            "Automatically selected Storage: File\nConnecting to Storage daemon File at sd.example:9103 ...\n\
             3906 File device \"FileDev\" is always mounted.\n");
        assert_eq!(sd.sent, vec!["File: mount FileDev drive=0\n"]);

        let mut sd = fake(|_| Ok(vec![]));
        mount("File drive=2 slot=5", &e, &mut sd, "mount");
        mount("storage=File slot=0", &e, &mut sd, "mount");
        mount("File slot=3", &e, &mut sd, "release");
        mount("umountish", &e, &mut sd, "unmount");
        assert_eq!(sd.sent, vec!["File: mount FileDev drive=2 slot=5\n", "File: mount FileDev drive=0\n",
            "File: release FileDev drive=0\n", "File: unmount FileDev drive=0\n"]);
    }

    #[test]
    fn device_names_are_bashed_and_failures_reported() {
        let mut e = env(&[("File", false, true)]);
        e.storages[0].0.device = "My Device".into();
        let mut sd = fake(|_| Err("refused".into()));
        assert_eq!(mount("storage=File", &e, &mut sd, "release"),
            "Connecting to Storage daemon File at sd.example:9103 ...\nFailed to connect to Storage daemon.\n");
        assert_eq!(sd.sent, vec!["File: release My\u{1}Device drive=0\n"]);
    }

    #[test]
    fn alldrives_asks_the_sd_for_its_drives() {
        let e = env(&[("Changer13", true, true)]);
        let mut sd = fake(|cmd| Ok(if cmd.starts_with("autochanger") { vec!["drives=2\n".into()] }
            else { vec![format!("3922 {cmd}")] }));
        let out = mount("Changer13 alldrives", &e, &mut sd, "unmount");
        assert_eq!(sd.sent, vec!["Changer13: autochanger drives Changer13Dev\n",
            "Changer13: unmount Changer13Dev drive=0\n", "Changer13: unmount Changer13Dev drive=1\n"]);
        assert_eq!(out.matches("Connecting to Storage daemon Changer13").count(), 3);
        // Cached: the next command needs no drive query; two drives → dialog.
        let mut sd = fake(|_| Ok(vec![]));
        assert_eq!(mount_dialog(&["."], "Changer13", &e, &mut sd, "release"),
            "Select Drive:\n1: Drive 0\n2: Drive 1\nSelect drive (1-2): .\nSelection aborted, nothing done.\n");
        assert!(sd.sent.is_empty(), "a cancelled drive selection contacts no SD");
        mount_dialog(&["2"], "Changer13", &e, &mut sd, "release");
        assert_eq!(sd.sent, vec!["Changer13: release Changer13Dev drive=1\n"]);
    }

    #[test]
    fn autochanger_with_one_drive_asks_for_the_slot_on_mount() {
        let e = env(&[("Changer11", true, true)]);
        let mut sd = fake(|cmd| Ok(if cmd.starts_with("autochanger") { vec!["3998 note\n".into(), "drives=1\n".into()] }
            else { vec![] }));
        assert_eq!(mount_dialog(&["."], "Changer11", &e, &mut sd, "mount"),
            "Connecting to Storage daemon Changer11 at sd.example:9103 ...\n3998 note\nEnter autochanger slot: .\n");
        assert_eq!(sd.sent, vec!["Changer11: autochanger drives Changer11Dev\n"], "cancelled: no mount");
        let mut sd = fake(|_| Ok(vec![]));
        mount_dialog(&["4"], "Changer11", &e, &mut sd, "mount");
        assert_eq!(sd.sent, vec!["Changer11: mount Changer11Dev drive=0 slot=4\n"]);
        let mut sd = fake(|_| Ok(vec![]));
        mount("Changer11 slot=4", &e, &mut sd, "mount");
        assert_eq!(sd.sent, vec!["Changer11: mount Changer11Dev drive=0 slot=4\n"]);
    }

    #[test]
    fn str_to_int64_like_upstream() {
        assert_eq!((str_to_int64("12x"), str_to_int64("-3"), str_to_int64("abc")), (12, -3, 0));
    }
}
