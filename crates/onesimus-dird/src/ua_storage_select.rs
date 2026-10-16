//! Storage selection and storage-daemon volume operations of the console.
//!
//! Port of C++ dird/ua_select.cc get_storage_resource() (:1320),
//! select_storage_resource() (:197), GetStorageDrive() (:1429) and
//! GetStorageSlot() (:1471) plus the SD side of dird/storage.cc
//! GetNumDrives() and dird/sd_cmds.cc open_sd_bsock(),
//! NativeGetNumDrives() and NativeAutochangerVolumeOperation(), as used by
//! `mount`, `unmount`/`umount`, `release` (ua_cmds.cc DoMountCmd) and the
//! storage branch of `status` (ua_status.cc StatusCmd), and the shared
//! autochanger helpers: GetNumSlots(), native_get_vol_list(),
//! get_vol_list_from_storage() with its cache, InvalidateVolList(),
//! vol_is_loaded_in_drive() and GetUserSlotList() (compatibility log C-045).
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
    args.iter().find(|a| a.key.eq_ignore_ascii_case(key))
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
                    Ok(BnetMessage::Data(data)) => {
                        // SendMsg("%s", msg) / sscanf: the text up to the first NUL.
                        let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                        messages.push(String::from_utf8_lossy(&data[..end]).into_owned())
                    }
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
pub(crate) fn dev_name(store: &StorageResource) -> String {
    let first = store.device.split(',').next().unwrap_or("").trim().trim_matches('"');
    // BashSpaces()
    first.replace(' ', "\u{1}")
}

/// open_sd_bsock() (sd_cmds.cc:165) + one exchange; None if the connection
/// failed (the error has been printed).
pub(crate) fn open_and_exchange(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, command: &str)
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

/// runtime_storage_status->slots: reused once it is positive.
static SLOTS: Mutex<Option<HashMap<String, i16>>> = Mutex::new(None);

/// GetNumSlots() (storage.cc:640) / NativeGetNumSlots() (sd_cmds.cc:530).
pub fn get_num_slots(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource) -> i16 {
    if let Some(slots) = SLOTS.lock().ok().and_then(|d| d.as_ref()?.get(&store.name).copied()) {
        if slots > 0 { return slots; }
    }
    let mut slots = 0;
    if let Some(messages) = open_and_exchange(ua, sd, store, &format!("autochanger slots {}\n", dev_name(store))) {
        for message in messages {
            match message.strip_prefix("slots=").and_then(|rest| {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<i16>().ok()
            }) {
                Some(n) => { slots = n; break; }
                None => ua.send_msg(message),
            }
        }
        ua.send_msg(format!("Device \"{}\" has {slots} slots.\n", dev_name(store).replace('\u{1}', " ")));
    }
    if let Ok(mut cache) = SLOTS.lock() {
        cache.get_or_insert_with(HashMap::new).insert(store.name.clone(), slots);
    }
    slots
}

/// One autochanger list entry (C++ vol_list_t): type `D`/`S`/`I` (other
/// letters: `?`), drive or slot number, content `E`/`F` (other: `?`), the
/// slot loaded in a drive and the volume name.
#[derive(Debug, Clone, PartialEq)]
pub struct VolListEntry {
    pub slot_type: char,
    pub slot: u16,
    pub content: char,
    pub loaded: Option<u16>,
    pub vol_name: Option<String>,
}

impl VolListEntry {
    /// vol_list_t::currently_loaded_slot_number (kInvalidSlotNumber when unset).
    pub fn loaded_slot(&self) -> u16 {
        self.loaded.unwrap_or(u16::MAX)
    }
}

/// C++ IsAnInteger(): one or more digits and nothing else.
fn is_an_integer(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// C++ atoi() stored into a slot_number_t (uint16_t).
fn atoi_u16(text: &str) -> u16 {
    str_to_int64(text) as i32 as u16
}

/// C++ IsSlotNumberValid() (baconfig.h:243).
pub fn is_slot_number_valid(slot: u16) -> bool {
    slot != 0 && slot != u16::MAX
}

/// native_get_vol_list() (sd_cmds.cc:258): `autochanger listall <dev> ` or
/// `autochanger list <dev> `; SD messages (`3nnn `) are passed on, parse
/// errors are reported. `scan` (list only) keeps empty slots and checks
/// only the slot number. None when the connection failed or nothing was
/// parsed. Entries are in StorageCompareVolListEntry order (element
/// address: drives first, then slots); equal addresses are dropped like
/// dlist::binary_insert.
pub fn native_get_vol_list(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, listall: bool, scan: bool)
    -> Option<Vec<VolListEntry>> {
    const INDEX_SLOT_OFFSET: u16 = 100;
    const INDEX_MAX_DRIVES: u16 = 100;
    let command = if listall { "listall" } else { "list" };
    let messages = open_and_exchange(ua, sd, store, &format!("autochanger {command} {} \n", dev_name(store)))?;
    let mut list: Vec<(u16, VolListEntry)> = Vec::new();
    for message in messages {
        // StripTrailingJunk()
        let msg = message.trim_end_matches([' ', '\n', '\r', '\t']);
        let bytes = msg.as_bytes();
        if bytes.len() >= 5 && bytes[0] == b'3' && bytes[1..4].iter().all(u8::is_ascii_digit) && bytes[4] == b' ' {
            ua.send_msg(format!("{msg}\n"));
            continue;
        }
        // list gives at most 2 fields (the volume name keeps any further
        // ':'), listall at most 5.
        let fields: Vec<&str> = msg.splitn(if listall { 5 } else { 2 }, ':').collect();
        if fields.len() < 2 || (listall && fields.len() < 3) {
            // parse_error: the ':' replaced so far are restored.
            ua.error_msg(format!("Illegal output from autochanger {command}: {msg}\n"));
            continue;
        }
        let (field1, field2) = (fields[0], fields[1]);
        let mut entry = VolListEntry { slot_type: 'S', slot: u16::MAX, content: '?', loaded: None, vol_name: None };
        let address;
        if scan && !listall {
            // Scanning -- require only valid slot
            entry.slot = atoi_u16(field1);
            if entry.slot == 0 {
                // sd->msg ends after field1 here.
                ua.error_msg(format!("Invalid Slot number: {field1}\n"));
                continue;
            }
            if field2.is_empty() {
                entry.content = 'E';
            } else {
                entry.content = 'F';
                entry.vol_name = Some(field2.to_string());
            }
            address = INDEX_SLOT_OFFSET.wrapping_add(entry.slot);
        } else if !listall {
            if field2.is_empty() {
                continue;
            }
            entry.slot = atoi_u16(field1);
            if !is_an_integer(field1) || entry.slot == 0 {
                ua.error_msg(format!("Invalid Slot number: {field1}\n"));
                continue;
            }
            if !crate::ua_select::is_volume_name_legal(ua, field2) {
                ua.error_msg(format!("Invalid Volume name: {field2}\n"));
                continue;
            }
            entry.content = 'F';
            entry.vol_name = Some(field2.to_string());
            address = INDEX_SLOT_OFFSET.wrapping_add(entry.slot);
        } else {
            entry.slot_type = match field1.chars().next() {
                Some(c @ ('D' | 'S' | 'I')) => c,
                _ => '?',
            };
            entry.slot = atoi_u16(field2);
            if entry.slot_type == 'D' {
                if !is_an_integer(field2) || entry.slot == u16::MAX {
                    ua.error_msg(format!("Invalid Drive number: {field2}\n"));
                    continue;
                }
                address = entry.slot;
                if address >= INDEX_MAX_DRIVES {
                    ua.error_msg(format!(
                        "Drive number {} greater then INDEX_MAX_DRIVES({INDEX_MAX_DRIVES}) please increase define\n",
                        entry.slot));
                    continue;
                }
            } else {
                if !is_an_integer(field2) || entry.slot == 0 {
                    ua.error_msg(format!("Invalid Slot number: {field2}\n"));
                    continue;
                }
                // slot_number_t arithmetic wraps like upstream.
                address = INDEX_SLOT_OFFSET.wrapping_add(entry.slot);
            }
            match fields[2].chars().next() {
                Some('E') => entry.content = 'E',
                Some('F') => {
                    entry.content = 'F';
                    match entry.slot_type {
                        'S' | 'I' => entry.vol_name = fields.get(3).map(|v| v.to_string()),
                        'D' => {
                            entry.loaded = fields.get(3).map(|v| atoi_u16(v));
                            entry.vol_name = fields.get(4).map(|v| v.to_string());
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if let Err(pos) = list.binary_search_by(|(a, _)| a.cmp(&address)) {
            list.insert(pos, (address, entry));
        }
    }
    if list.is_empty() { None } else { Some(list.into_iter().map(|(_, e)| e).collect()) }
}

/// runtime_storage_status->vol_list per storage: (listall type, timestamp,
/// contents).
type VolListCache = HashMap<String, (bool, i64, Vec<VolListEntry>)>;
static VOL_LIST: Mutex<Option<VolListCache>> = Mutex::new(None);

/// CacheStatusInterval of a Storage resource (default 30 seconds).
fn cache_status_interval(ua: &Ua, store: &StorageResource) -> i64 {
    let Some(config) = ua.ctx.config.as_ref().and_then(|c| c.read().ok()) else { return 30 };
    config.raw.iter()
        .find(|r| r.norm_type() == "storage" && r.name() == Some(store.name.as_str()))
        .filter(|r| r.directives.iter().any(|d| d.norm_key() == "cachestatusinterval"))
        .map(|r| crate::commands::defaults::time_directive(&r.directives, "cachestatusinterval") as i64)
        .unwrap_or(30)
}

/// get_vol_list_from_storage() (storage.cc:553): the content of the
/// autochanger, served from the per-storage cache when `cached`, of the
/// same list type and not older than CacheStatusInterval; any other call
/// drops the cached list and asks the SD (a successful answer is cached).
pub fn get_vol_list_from_storage(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, listall: bool,
    scan: bool, cached: bool) -> Option<Vec<VolListEntry>> {
    let now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
    let interval = cache_status_interval(ua, store);
    if let Ok(mut cache) = VOL_LIST.lock() {
        let cache = cache.get_or_insert_with(HashMap::new);
        if let Some((kind, timestamp, contents)) = cache.get(&store.name) {
            if cached && *kind == listall && now - *timestamp <= interval {
                return Some(contents.clone());
            }
            cache.remove(&store.name);
        }
    }
    let contents = native_get_vol_list(ua, sd, store, listall, scan)?;
    if let Ok(mut cache) = VOL_LIST.lock() {
        cache.get_or_insert_with(HashMap::new).insert(store.name.clone(), (listall, now, contents.clone()));
    }
    Some(contents)
}

/// InvalidateVolList() (storage.cc:811): mount, unmount and release change
/// the content of the autochanger.
pub fn invalidate_vol_list(store_name: &str) {
    if let Ok(mut cache) = VOL_LIST.lock() {
        if let Some(cache) = cache.as_mut() {
            cache.remove(store_name);
        }
    }
}

/// vol_is_loaded_in_drive() (storage.cc:741): the drive holding the volume
/// of `slot`.
pub fn vol_is_loaded_in_drive(list: &[VolListEntry], slot: u16) -> Option<&VolListEntry> {
    list.iter().find(|vl| vl.slot_type == 'D' && vl.loaded_slot() == slot)
}

/// C++ atoi() of a digit string: strtol() (saturating) truncated to int.
fn atoi_digits(text: &str) -> i32 {
    text.bytes().try_fold(0i64, |acc, d| acc.checked_mul(10)?.checked_add(i64::from(d - b'0')))
        .unwrap_or(i64::MAX) as i32
}

/// GetUserSlotList() (ua_select.cc:1822): the slots selected by
/// `<argument>=` (or the argument without its trailing `s`), e.g.
/// `slots=1-3,7`; all slots without it. Index `n` is slot `n + 1`. None on
/// a malformed list (upstream logs the reason only at debug level).
pub fn get_user_slot_list(args: &[Argument], argument: &str, num_slots: i32) -> Option<Vec<bool>> {
    // FindArgWithValue(): the first argument with that keyword, if it has a value.
    let find = |key: &str| args.iter().find(|a| a.key.eq_ignore_ascii_case(key)).and_then(|a| a.value.as_deref());
    let mut search: String = argument.chars().take(19).collect();
    let mut value = find(&search);
    if value.is_none() && search.ends_with('s') {
        search.pop();
        value = find(&search);
    }
    let mut slots = vec![false; num_slots.max(0) as usize];
    let Some(value) = value else {
        slots.iter_mut().for_each(|s| *s = true);
        return Some(slots);
    };
    let skip_spaces = |p: &str| p.trim_start_matches(|c: char| c.is_ascii_whitespace()).to_string();
    // StripTrailingJunk(); the loop ends at an empty remainder ("1,,2" is "1").
    let mut rest = Some(value.trim_end_matches([' ', '\n', '\r', '\t']));
    while let Some(p) = rest.filter(|p| !p.is_empty()) {
        let (part, next) = match p.split_once(',') {
            Some((part, next)) => (part, Some(next)),
            None => (p, None),
        };
        rest = next;
        let (beg, end) = match part.find('-') {
            // Negative numbers not permitted
            Some(0) => return None,
            Some(h) => {
                let (start, end) = (skip_spaces(&part[..h]), &part[h + 1..]);
                if !is_an_integer(end) || !is_an_integer(&start) {
                    return None;
                }
                let (beg, end) = (atoi_digits(&start), atoi_digits(end));
                if end < beg {
                    return None;
                }
                (beg, end)
            }
            None => {
                let start = skip_spaces(part);
                if !is_an_integer(&start) {
                    return None;
                }
                (atoi_digits(&start), atoi_digits(&start))
            }
        };
        if beg <= 0 || end <= 0 || end > num_slots {
            return None;
        }
        for i in beg..=end {
            slots[(i - 1) as usize] = true;
        }
    }
    Some(slots)
}

/// GetStorageDrive() (ua_select.cc:1429); None is kInvalidDriveNumber.
pub fn get_storage_drive(ua: &mut Ua, args: &[Argument], env: &Env, sd: &mut impl SdLink, store: &StorageResource)
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
    // IsSlotNumberValid() on the slot_number_t.
    let command = if is_slot_number_valid(slot as u16) {
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

    invalidate_vol_list(&store.name);
}

/// get_volume_name_from_SD() (sd_cmds.cc:186): `readlabel <dev> Slot=<slot>
/// drive=<drive>`; every SD message up to the first `3001 Volume=<name>
/// Slot=<n>` is relayed. Used by `update slots scan` (C-054).
pub fn get_volume_name_from_sd(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, slot: u16, drive: i16)
    -> Option<String> {
    // readlabelcmd "readlabel %s Slot=%hd drive=%hd\n"
    let command = format!("readlabel {} Slot={} drive={drive}\n", dev_name(store), slot as i16);
    let Some(messages) = open_and_exchange(ua, sd, store, &command) else {
        ua.error_msg("Could not open SD socket.\n");
        return None;
    };
    for message in messages {
        ua.send_msg(message.clone());
        if let Some(name) = message.strip_prefix("3001 Volume=").and_then(scan_readlabel_response) {
            return Some(name);
        }
    }
    None
}

/// sscanf(msg + 12, "%s Slot=%hd") == 2.
fn scan_readlabel_response(rest: &str) -> Option<String> {
    let is_space = |c: char| c.is_ascii_whitespace();
    let rest = rest.trim_start_matches(is_space);
    let end = rest.find(is_space).unwrap_or(rest.len());
    if end == 0 { return None; }
    let (name, rest) = rest.split_at(end);
    let rest = rest.trim_start_matches(is_space).strip_prefix("Slot=")?.trim_start_matches(is_space);
    let digits = rest.strip_prefix(['+', '-']).unwrap_or(rest);
    digits.starts_with(|c: char| c.is_ascii_digit()).then(|| name.to_string())
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

    #[test]
    fn num_slots_of_a_plain_device() {
        let mut sd = fake(|_| Ok(vec!["3998 Device \"FileDev\" is not an autochanger.\n".into(), "slots=0\n".into()]));
        let ctx = context(CONFIG, ApiMode::Human);
        let mut ua = Ua::new(&ctx);
        let store = storage("SlotsPlain", "FileDev");
        assert_eq!(get_num_slots(&mut ua, &mut sd, &store), 0);
        assert_eq!(sd.sent, ["SlotsPlain: autochanger slots FileDev\n"]);
        assert_eq!(output(ua.finish(true)), "Connecting to Storage daemon SlotsPlain at sd.example:9103 ...\n\
            3998 Device \"FileDev\" is not an autochanger.\nDevice \"FileDev\" has 0 slots.\n");
    }

    #[test]
    fn vol_list_listall_parsing() {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut sd = fake(|_| Ok(vec!["3998 not an autochanger.\n".into()]));
        let mut ua = Ua::new(&ctx);
        assert_eq!(native_get_vol_list(&mut ua, &mut sd, &storage("Tape", "TapeDev"), true, false), None);
        assert_eq!(output(ua.finish(true)), "Connecting to Storage daemon Tape at sd.example:9103 ...\n3998 not an autochanger.\n");

        let mut sd = fake(|_| Ok(vec![
            "S:2:E\n".into(), "S:1:F:vol1\n".into(), "D:0:F:3:vol3\n".into(), "I:10:F:vol10\n".into(),
            "garbage\n".into(), "S:x:E\n".into(), "S:1:F:dup\n".into(),
        ]));
        let mut ua = Ua::new(&ctx);
        let list = native_get_vol_list(&mut ua, &mut sd, &storage("Tape", "TapeDev"), true, false).unwrap();
        assert_eq!(sd.sent, ["Tape: autochanger listall TapeDev \n"]);
        let result = ua.finish(true);
        assert_eq!(result.text_output, "Connecting to Storage daemon Tape at sd.example:9103 ...\n");
        assert_eq!(result.error_message, "Illegal output from autochanger listall: garbage\nInvalid Slot number: x\n");
        let got: Vec<(char, u16, char, Option<u16>, Option<&str>)> = list.iter()
            .map(|e| (e.slot_type, e.slot, e.content, e.loaded, e.vol_name.as_deref())).collect();
        assert_eq!(got, [
            ('D', 0, 'F', Some(3), Some("vol3")),
            ('S', 1, 'F', None, Some("vol1")),
            ('S', 2, 'E', None, None),
            ('I', 10, 'F', None, Some("vol10")),
        ]);
    }

    fn entries(list: &[VolListEntry]) -> Vec<(char, u16, char, Option<u16>, Option<&str>)> {
        list.iter().map(|e| (e.slot_type, e.slot, e.content, e.loaded, e.vol_name.as_deref())).collect()
    }

    #[test]
    fn vol_list_listall_errors() {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut sd = fake(|_| Ok(vec![
            "S:1\n".into(), "D:x:E\n".into(), "D:100:E\n".into(), "D:-1:E\n".into(), "X:5:F:v\n".into(),
            "S:6:Q\n".into(), "S:7:F:a:b:c\n".into(), "D:1:F\n".into(),
        ]));
        let mut ua = Ua::new(&ctx);
        let list = native_get_vol_list(&mut ua, &mut sd, &storage("Tape", "TapeDev"), true, false).unwrap();
        assert_eq!(ua.finish(true).error_message, "Illegal output from autochanger listall: S:1\n\
            Invalid Drive number: x\n\
            Drive number 100 greater then INDEX_MAX_DRIVES(100) please increase define\n\
            Invalid Drive number: -1\n");
        assert_eq!(entries(&list), [
            ('D', 1, 'F', None, None),
            ('?', 5, 'F', None, None),
            ('S', 6, '?', None, None),
            ('S', 7, 'F', None, Some("a")),
        ]);
    }

    #[test]
    fn vol_list_list_and_scan() {
        let ctx = context(CONFIG, ApiMode::Human);
        let answer = |_: &str| Ok(vec!["3306 Issuing autochanger \"list\" command.\n".into(), "2:vol2\n".into(),
            "1:\n".into(), "x:volx\n".into(), "0:vol0\n".into(), "3:bad vol\n".into(), "4:v:4\n".into(),
            "nocolon\n".into()]);
        let mut sd = fake(answer);
        let mut ua = Ua::new(&ctx);
        let list = native_get_vol_list(&mut ua, &mut sd, &storage("Tape", "TapeDev"), false, false).unwrap();
        assert_eq!(sd.sent, ["Tape: autochanger list TapeDev \n"]);
        let result = ua.finish(true);
        assert_eq!(result.text_output, "Connecting to Storage daemon Tape at sd.example:9103 ...\n\
            3306 Issuing autochanger \"list\" command.\n");
        assert_eq!(result.error_message, "Invalid Slot number: x\nInvalid Slot number: 0\n\
            Illegal character \" \" in a volume name.\nInvalid Volume name: bad vol\n\
            Illegal output from autochanger list: nocolon\n");
        assert_eq!(entries(&list), [('S', 2, 'F', None, Some("vol2")), ('S', 4, 'F', None, Some("v:4"))]);

        // Scan: empty slots are kept, only the slot number is checked.
        let mut sd = fake(answer);
        let mut ua = Ua::new(&ctx);
        let list = native_get_vol_list(&mut ua, &mut sd, &storage("Tape", "TapeDev"), false, true).unwrap();
        assert_eq!(ua.finish(true).error_message, "Invalid Slot number: x\nInvalid Slot number: 0\n\
            Illegal output from autochanger list: nocolon\n");
        assert_eq!(entries(&list), [
            ('S', 1, 'E', None, None),
            ('S', 2, 'F', None, Some("vol2")),
            ('S', 3, 'F', None, Some("bad vol")),
            ('S', 4, 'F', None, Some("v:4")),
        ]);
    }

    #[test]
    fn vol_list_cache() {
        let ctx = context(CONFIG, ApiMode::Human);
        let store = storage("CacheTape", "TapeDev");
        let mut sd = fake(|cmd| Ok(vec![if cmd.contains("listall") { "S:1:F:all\n" } else { "1:part\n" }.into()]));
        let mut ua = Ua::new(&ctx);
        let all = get_vol_list_from_storage(&mut ua, &mut sd, &store, true, true, true).unwrap();
        assert_eq!(get_vol_list_from_storage(&mut ua, &mut sd, &store, true, true, true).unwrap(), all);
        assert_eq!(sd.sent.len(), 1, "served from the cache");
        // Another list type or cached=false asks the SD again.
        let part = get_vol_list_from_storage(&mut ua, &mut sd, &store, false, false, true).unwrap();
        assert_eq!(part[0].vol_name.as_deref(), Some("part"));
        get_vol_list_from_storage(&mut ua, &mut sd, &store, false, false, false);
        get_vol_list_from_storage(&mut ua, &mut sd, &store, true, true, true);
        assert_eq!(sd.sent.len(), 4);
        // A failed retrieval leaves nothing cached.
        invalidate_vol_list("CacheTape");
        let mut failing = fake(|_| Err("refused".into()));
        assert!(get_vol_list_from_storage(&mut ua, &mut failing, &store, true, true, true).is_none());
        get_vol_list_from_storage(&mut ua, &mut sd, &store, true, true, true);
        assert_eq!(sd.sent.len(), 5);
    }

    #[test]
    fn slot_helpers() {
        let all = |n: i32| get_user_slot_list(&[], "slots", n);
        assert_eq!(all(3), Some(vec![true, true, true]));
        let sel = |line: &str, n: i32| get_user_slot_list(&parse(line), "slots", n)
            .map(|v| v.iter().enumerate().filter(|(_, on)| **on).map(|(i, _)| i + 1).collect::<Vec<_>>());
        assert_eq!(sel("slots=2,4-5", 6), Some(vec![2, 4, 5]));
        assert_eq!(sel("slot=3", 6), Some(vec![3]));
        assert_eq!(sel("slots=6", 6), Some(vec![6]));
        assert_eq!(sel("slots=", 6), Some(vec![]));
        assert_eq!(sel("slots slot=2", 6), Some(vec![2]));
        assert_eq!(sel("slots=99999999999", 6), None);
        for bad in ["slots=7", "slots=0", "slots=-1", "slots=1-", "slots=3-2", "slots=,1"] {
            assert_eq!(sel(bad, 6), None, "{bad}");
        }
        assert!(is_slot_number_valid(1) && !is_slot_number_valid(0) && !is_slot_number_valid(u16::MAX));
        let list = [
            VolListEntry { slot_type: 'D', slot: 0, content: 'E', loaded: None, vol_name: None },
            VolListEntry { slot_type: 'D', slot: 1, content: 'F', loaded: Some(4), vol_name: Some("v4".into()) },
        ];
        assert_eq!(vol_is_loaded_in_drive(&list, 4).map(|d| d.slot), Some(1));
        assert!(vol_is_loaded_in_drive(&list, 3).is_none());
    }
}
