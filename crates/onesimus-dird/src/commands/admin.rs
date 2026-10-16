//! Admin commands: label, mount, unmount, release, update,
//! estimate, prune, purge, truncate, reload
//!
//! These commands communicate with the Storage Daemon and/or catalog.

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;

/// Send a command to a named Storage Daemon (resolves from config).
fn sd_command(ctx: &ExecContext, storage_name: &str, command: &str) -> Result<String, String> {
    let config = ctx.config.as_ref()
        .and_then(|c| c.read().ok())
        .ok_or("No config")?;
    let stor = config.storages.iter().find(|s| s.name == storage_name)
        .ok_or(format!("Storage '{storage_name}' not found"))?;
    let dir_name = config.director.name.clone();
    let addr = stor.address.clone();
    let port = stor.port;
    let password = stor.password.clone();
    drop(config);

    bareos_stored::command::sd_command(&addr, port, &password, &dir_name, command)
}

pub fn cmd_label(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    label_dialog(&mut ua, ast, false, label_execute);
    ua.finish(true)
}

/// A complete label/relabel request (ua_label.cc do_label).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LabelRequest {
    Barcodes { storage: String },
    Label { storage: String, volume: String, pool: String, old_volume: Option<String> },
}

/// do_label (ua_label.cc:393): OpenClientDb, storage (get_storage_resource
/// with the default keyword), for relabel the old volume (`oldvolume=` or
/// SelectMediaDbr; it must be Purged or Recycled), the new name (`volume=`
/// or "Enter new Volume name: " until legal and unused), the slot for
/// autochangers and the pool (SelectPoolDbr). `act` runs only for a
/// complete request.
pub(crate) fn label_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst, relabel: bool,
    act: impl FnOnce(&mut crate::ua_select::Ua, LabelRequest)) {
    use crate::ua_select::{self, find_arg_with_value};
    let ctx = ua.ctx;
    if !ua.open_client_db(ast, false) {
        return;
    }
    let barcodes = !relabel && ast.arguments.iter()
        .any(|a| a.key.eq_ignore_ascii_case("barcode") || a.key.eq_ignore_ascii_case("barcodes"));
    let Some(storage) = ua_select::get_storage_resource(ua, ast, true, barcodes).map(|s| s.name) else { return };
    let autochanger = ctx.config.as_ref().and_then(|c| c.read().ok())
        .is_some_and(|c| ua_select::is_autochanger(&c, &storage));
    if barcodes {
        act(ua, LabelRequest::Barcodes { storage });
        return;
    }
    let mut old_volume = None;
    if relabel {
        let mut old = None;
        if let Some(name) = find_arg_with_value(ast, "oldvolume") {
            match ua_select::get_media_record(ctx, 0, name) {
                Ok(record) => old = Some(record),
                Err(error) => {
                    ua.error_msg(error.clone());
                    ua.db_error = error;
                }
            }
        }
        let old = match old {
            Some(old) => old,
            None => match ua_select::select_media_dbr(ua, ast) {
                Some(old) => old,
                None => return,
            },
        };
        if old.vol_status != "Purged" && old.vol_status != "Recycle" {
            ua.error_msg(format!("Volume \"{}\" has VolStatus {}. It must be Purged or Recycled before relabeling.\n",
                old.volume_name, old.vol_status));
            return;
        }
        old_volume = Some(old.volume_name);
    }
    let mut given = find_arg_with_value(ast, "volume").map(str::to_string);
    let volume = loop {
        let name = match given.take() {
            Some(name) => name,
            None => match ua.get_cmd("Enter new Volume name: ") {
                Some(name) => name,
                None => return,
            },
        };
        if !ua_select::is_volume_name_legal(ua, &name) {
            continue;
        }
        // An existing record is only reused while it is not labeled (VolBytes 0).
        let used = ctx.catalog.as_ref()
            .and_then(|cat| cat.query_rows(&format!("SELECT VolBytes FROM Media WHERE VolumeName='{}'",
                name.replace('\'', "''"))).ok())
            .and_then(|t| t.rows.into_iter().next())
            .and_then(|row| row.into_iter().next().flatten())
            .is_some_and(|bytes| bytes != "0");
        if used {
            ua.error_msg(format!("Media record for new Volume \"{name}\" already exists.\n"));
            continue;
        }
        break name;
    };
    if find_arg_with_value(ast, "slot").is_none() && autochanger
        && ua.get_pint("Enter slot (0 or Enter for none): ").is_none() {
        return;
    }
    let Some(pool) = ua_select::select_pool_dbr(ua, ast, "pool") else { return };
    act(ua, LabelRequest::Label { storage, volume, pool, old_volume });
}

/// SendLabelRequest and the catalog/automount work of a complete label.
fn label_execute(ua: &mut crate::ua_select::Ua, request: LabelRequest) {
    let ctx = ua.ctx;
    let (stor_name, v, pool_name) = match request {
        LabelRequest::Barcodes { storage } => {
            ua.send_msg(format!("Sending barcode label command to Storage {storage}...\n"));
            return;
        }
        LabelRequest::Label { storage, volume, pool, .. } => (storage, volume, pool),
    };
    let mut output = format!("Connecting to Storage daemon {stor_name} ...\n");
    let cmd = format!("label Volume=\"{v}\" PoolName=\"{pool_name}\" MediaType=\"File\" Slot=0 drive=0");
    match sd_command(ctx, &stor_name, &cmd) {
        Ok(resp) => {
            output.push_str(&format!("Label Volume \"{v}\" Pool \"{pool_name}\":\n"));
            output.push_str(&resp);
            // Create media record in catalog
            if let Some(cat) = &ctx.catalog {
                let _ = cat.raw_query(&format!(
                    "INSERT INTO media (volumename, mediatype, poolid, volstatus, volbytes, voljobs, volfiles, maxvolbytes) \
                     SELECT '{v}', 'File', poolid, 'Append', 0, 0, 0, 0 FROM pool WHERE name = '{pool_name}' \
                     ON CONFLICT DO NOTHING"
                ));
            }
            output.push_str(&format!("\nVolume \"{v}\" labeled and added to catalog.\n"));
            // C++ ua_label.cc:557: mount the new volume unless `automount off`.
            if ctx.session_mgr.as_ref().is_none_or(|mgr| mgr.session_automount(ctx.session_id)) {
                output.push_str(&automount_after_label(ctx, &stor_name));
            }
        }
        Err(e) => {
            output.push_str(&format!("SD label failed: {e}\n"));
        }
    }
    ua.send_msg(output);
}

/// C++ ua_label.cc:557 automount: "Requesting to mount <dev> ..." and
/// `mount <dev> drive=0` (spaces bashed to 0x01) to the SD, echoing its reply.
fn automount_after_label(ctx: &ExecContext, stor_name: &str) -> String {
    let device = ctx.config.as_ref().and_then(|c| c.read().ok())
        .and_then(|cfg| cfg.find_storage(stor_name).map(|s| s.device.clone()))
        .unwrap_or_default();
    let mut output = format!("Requesting to mount {device} ...\n");
    match sd_command(ctx, stor_name, &format!("mount {} drive=0", device.replace(' ', "\x01"))) {
        Ok(resp) => output.push_str(&resp),
        Err(e) => output.push_str(&format!("mount {stor_name}: {e}\n")),
    }
    output
}

pub fn cmd_mount(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    do_mount_cmd(ast, ctx, "mount")
}

pub fn cmd_unmount(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    do_mount_cmd(ast, ctx, "unmount")
}

pub fn cmd_release(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    do_mount_cmd(ast, ctx, "release")
}

/// C++ ua_cmds.cc DoMountCmd(): OpenClientDb(), storage selection,
/// drive/slot, SD request.
fn do_mount_cmd(ast: &CommandAst, ctx: &ExecContext, cmd: &str) -> CommandResult {
    use crate::ua_storage_select::{Env, NetSd};
    let mut ua = crate::ua_select::Ua::new(ctx);
    if !ua.open_client_db(ast, false) {
        return ua.finish(false);
    }
    let Some(env) = Env::load(ctx) else { return CommandResult::error("No config\n".into()) };
    let director = ctx.config.as_ref().and_then(|c| c.read().ok())
        .map(|c| c.director.name.clone()).unwrap_or_default();
    let mut sd = NetSd { director };
    crate::ua_storage_select::do_mount_cmd(&mut ua, &ast.arguments, &env, &mut sd, cmd);
    ua.finish(true)
}

pub fn cmd_update(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    update_dialog(&mut ua, ast, update_execute);
    ua.finish(true)
}

/// A catalog change of `update` (one SQL statement and its InfoMsg).
#[derive(Debug, Clone, PartialEq, Eq)]
enum UpdateAction {
    /// `UPDATE Media SET <column>=<value> WHERE MediaId=<id>`
    Media { media_id: i64, column: &'static str, value: String, message: String },
    /// Pool record from the Pool resource (UpdatePool).
    Pool(String),
    /// UpdateSlots (ua_update.cc:1075) on the selected storage with the
    /// command arguments (key, value): SD dialog and catalog work.
    Slots { storage: String, args: Vec<(String, Option<String>)> },
    /// An update kind without catalog support yet.
    Unsupported(&'static str),
}

type UpdateAct<'f> = &'f mut dyn FnMut(&mut crate::ua_select::Ua, UpdateAction);

/// UpdateCmd (ua_update.cc:71): OpenClientDb, then the keyword (media/volume,
/// pool, slots/slot, jobid, stats) or "Update choice:".
fn update_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    mut act: impl FnMut(&mut crate::ua_select::Ua, UpdateAction)) {
    const KEYWORDS: [&str; 7] = ["media", "volume", "pool", "slots", "slot", "jobid", "stats"];
    let act: UpdateAct = &mut act;
    if !ua.open_client_db(ast, false) {
        return;
    }
    let choice = match find_arg_keyword(ast, &KEYWORDS) {
        Some(0 | 1) => 0,
        Some(2) => 1,
        Some(3 | 4) => 2,
        Some(5) => {
            // UpdateJob
            if crate::ua_select::find_arg_with_value(ast, "jobid").is_none() {
                ua.error_msg("Expect JobId keyword, not found.\n");
            } else {
                act(ua, UpdateAction::Unsupported("update jobid"));
            }
            return;
        }
        Some(_) => 3,
        None => {
            ua.start_prompt("Update choice:\n");
            for item in ["Volume parameters", "Pool from resource", "Slots from autochanger", "Long term statistics"] {
                ua.add_prompt(item);
            }
            match ua.do_prompt("item", "Choose catalog item to update") {
                Some((choice, _)) => choice,
                None => return,
            }
        }
    };
    match choice {
        0 => { update_volume(ua, ast, act); }
        1 => update_pool(ua, ast, act),
        2 => {
            // UpdateSlots: get_storage_resource(ua, true, true).
            if let Some(store) = crate::ua_select::get_storage_resource(ua, ast, true, true) {
                let args = ast.arguments.iter().map(|a| (a.key.clone(), a.value.clone())).collect();
                act(ua, UpdateAction::Slots { storage: store.name, args });
            }
        }
        _ => act(ua, UpdateAction::Unsupported("update stats")),
    }
}

/// UpdatePool (ua_update.cc:940): pool resource, its catalog record, update.
fn update_pool(ua: &mut crate::ua_select::Ua, ast: &CommandAst, act: UpdateAct) {
    use crate::ua_select;
    let Some(pool) = ua_select::get_pool_resource(ua, ast) else { return };
    // GetPoolDbr with the name filled in, otherwise the catalog list.
    let pool = match ua_select::get_pool_record(ua.ctx, &pool) {
        Ok(_) => pool,
        Err(error) => {
            ua.error_msg(format!("Could not find Pool \"{pool}\": ERR={error}"));
            match ua_select::select_pool_dbr(ua, ast, "pool") {
                Some(pool) => pool,
                None => return,
            }
        }
    };
    act(ua, UpdateAction::Pool(pool));
}

/// C++ GetEnabled(): yes/true, no/false, archived or 0..2.
fn get_enabled(ua: &mut crate::ua_select::Ua, value: &str) -> Option<i64> {
    let enabled = if value.eq_ignore_ascii_case("yes") || value.eq_ignore_ascii_case("true") {
        1
    } else if value.eq_ignore_ascii_case("no") || value.eq_ignore_ascii_case("false") {
        0
    } else if value.eq_ignore_ascii_case("archived") {
        2
    } else if !value.is_empty() {
        value.parse::<i64>().unwrap_or(-1)
    } else {
        -1
    };
    if !(0..=2).contains(&enabled) {
        ua.error_msg("Invalid Enabled value, it must be yes, no, archived, 0, 1, or 2\n");
        return None;
    }
    Some(enabled)
}

/// A numeric constant (PostgreSQL lexer): side-effect free in SQL.
fn is_numeric_constant(value: &str) -> bool {
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    let (mantissa, exponent) = match digits.find(['e', 'E']) {
        Some(at) => (&digits[..at], Some(&digits[at + 1..])),
        None => (digits, None),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    (!int.is_empty() || !frac.is_empty()) && all_digits(int) && all_digits(frac)
        && exponent.is_none_or(|e| { let e = e.strip_prefix(['+', '-']).unwrap_or(e); !e.is_empty() && all_digits(e) })
}

/// UpdateVolmaxjobs/UpdateVolmaxfiles (ua_update.cc:201) put the value
/// verbatim into `UPDATE Media SET <column>=<value>`. A numeric constant is
/// written like that; anything else is only parsed (never run), so the
/// catalog error of the C++ statement is reported without its side effects
/// (C-012). Statements that would parse are refused with C-012's message.
fn verbatim_number(ua: &mut crate::ua_select::Ua, column: &str, key: &str, value: &str, media_id: i64) -> Option<String> {
    if is_numeric_constant(value) {
        return Some(value.to_string());
    }
    let sql = format!("UPDATE Media SET {column}={value} WHERE MediaId={media_id}");
    let checked = (!value.contains([';', '$'])).then(|| ua.ctx.catalog.as_ref().map(|cat| cat.check_statement(&sql))).flatten();
    match checked {
        Some(Err(error)) => ua.error_msg(format!("Query failed: {sql}: ERR={error}\n")),
        _ => ua.error_msg(format!("Invalid {key} value: {value}\n")),
    }
    None
}

/// The encryption key of a volume is set (MediaDbRecord EncrKey).
fn has_encryption_key(ctx: &ExecContext, media_id: i64) -> bool {
    ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows(&format!("SELECT EncryptionKey FROM Media WHERE MediaId={media_id}")).ok())
        .and_then(|t| t.rows.into_iter().next())
        .and_then(|row| row.into_iter().next().flatten())
        .is_some_and(|key| !key.is_empty())
}

/// UpdateVolEncryption (ua_update.cc:340): every check before a key is
/// generated or removed.
fn update_vol_encryption(ua: &mut crate::ua_select::Ua, value: &str, media: &crate::ua_select::MediaRecord)
    -> Option<UpdateAction> {
    let key_present = has_encryption_key(ua.ctx, media.media_id);
    let purged = media.vol_status == "Purged";
    let name = &media.volume_name;
    if value.eq_ignore_ascii_case("yes") {
        if key_present {
            ua.warning_msg(format!("Encryption key for volume '{name}' already configured.\n"));
            return None;
        }
    } else if value.eq_ignore_ascii_case("no") || value.eq_ignore_ascii_case("rotate") {
        let what = if value.eq_ignore_ascii_case("no") { "Removing encryption key from" } else { "Rotating encryption key for" };
        if !key_present {
            ua.warning_msg(format!("No encryption key for volume '{name}' configured.\n"));
            return None;
        }
        if !purged {
            ua.error_msg(format!("{what} volume '{name}' requires volstatus=Purged\n"));
            return None;
        }
    } else {
        ua.error_msg("Valid values for Encrypt are yes/no/rotate\n");
        return None;
    }
    Some(UpdateAction::Unsupported("update volume encrypt"))
}

/// The Update<Field> helpers of ua_update.cc for one volume.
fn update_field(ua: &mut crate::ua_select::Ua, ast: &CommandAst, act: UpdateAct, field: usize, value: &str,
    media: &crate::ua_select::MediaRecord) {
    use crate::ua_select::{edit_utime, get_pool_dbr, is_yesno};
    let set = |column: &'static str, value: String, message: String|
        UpdateAction::Media { media_id: media.media_id, column, value, message };
    let action = match field {
        0 => {
            const STATUS: [&str; 9] = ["Append", "Archive", "Disabled", "Full", "Used", "Cleaning", "Recycle", "Read-Only", "Error"];
            match STATUS.iter().find(|s| s.eq_ignore_ascii_case(value)) {
                Some(status) => set("VolStatus", format!("'{status}'"), format!("New Volume status is: {status}\n")),
                None => return ua.error_msg(format!("Invalid VolStatus specified: {value}\n")),
            }
        }
        1 | 2 => match crate::commands::show::parse_duration(value) {
            Some(seconds) if field == 1 =>
                set("VolRetention", seconds.to_string(), format!("New retention period is: {}\n", edit_utime(seconds))),
            Some(seconds) =>
                set("VolUseDuration", seconds.to_string(), format!("New use duration is: {}\n", edit_utime(seconds))),
            None if field == 1 => return ua.error_msg(format!("Invalid retention period specified: {value}\n")),
            None => return ua.error_msg(format!("Invalid use duration specified: {value}\n")),
        },
        3 => match verbatim_number(ua, "MaxVolJobs", "maxvoljobs", value, media.media_id) {
            Some(number) => set("MaxVolJobs", number, format!("New max jobs is: {value}\n")),
            None => return,
        },
        4 => match verbatim_number(ua, "MaxVolFiles", "maxvolfiles", value, media.media_id) {
            Some(number) => set("MaxVolFiles", number, format!("New max files is: {value}\n")),
            None => return,
        },
        5 => match crate::commands::show::parse_size(value) {
            Some(bytes) => set("MaxVolBytes", bytes.to_string(), format!("New Max bytes is: {bytes}\n")),
            None => return ua.error_msg(format!("Invalid max. bytes specification: {value}\n")),
        },
        6 | 7 => match is_yesno(value) {
            Some(flag) if field == 6 =>
                set("Recycle", i32::from(flag).to_string(), format!("New Recycle flag is: {}\n", if flag { "yes" } else { "no" })),
            Some(flag) =>
                set("InChanger", i32::from(flag).to_string(), format!("New InChanger flag is: {}\n", if flag { "yes" } else { "no" })),
            None => return ua.error_msg("Invalid value. It must be yes or no.\n"),
        },
        8 => {
            let slot = crate::commands::job::str_to_int64(value);
            let max_vols = ua.ctx.catalog.as_ref()
                .and_then(|cat| cat.query_rows(&format!("SELECT MaxVols FROM Pool WHERE PoolId={}", media.pool_id)).ok())
                .and_then(|t| t.rows.into_iter().next())
                .and_then(|row| row.into_iter().next().flatten())
                .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
            if (max_vols > 0 && slot > max_vols) || slot < 0 {
                return ua.error_msg(format!("Invalid slot, it must be between 0 and MaxVols={max_vols}\n"));
            }
            set("Slot", slot.to_string(), format!("New Slot is: {slot}\n"))
        }
        12 => match get_enabled(ua, value) {
            Some(enabled) => set("Enabled", enabled.to_string(), format!("New Enabled is: {enabled}\n")),
            None => return,
        },
        // UpdateVolPool / UpdateAllVolsFromPool: GetPoolDbr(<value>).
        9 => match get_pool_dbr(ua, ast, value, "pool") {
            Some(_) => UpdateAction::Unsupported("update volume pool"),
            None => return,
        },
        10 => UpdateAction::Unsupported("update volume frompool"),
        11 => match get_pool_dbr(ua, ast, value, "pool") {
            Some(_) => UpdateAction::Unsupported("update volume allfrompool"),
            None => return,
        },
        // UpdateVolRecyclepool: an empty value clears the RecyclePool.
        13 if value.is_empty() => UpdateAction::Unsupported("update volume recyclepool"),
        13 => match get_pool_dbr(ua, ast, value, "recyclepool") {
            Some(_) => UpdateAction::Unsupported("update volume recyclepool"),
            None => return,
        },
        14 => UpdateAction::Unsupported("update volume actiononpurge"),
        15 => UpdateAction::Unsupported("update volume storage"),
        _ => match update_vol_encryption(ua, value, media) {
            Some(action) => action,
            None => return,
        },
    };
    act(ua, action);
}

/// UpdateVolume (ua_update.cc:592): `<keyword>=<value>` arguments, else the
/// "Parameters to modify:" loop with its per-parameter questions.
fn update_volume(ua: &mut crate::ua_select::Ua, ast: &CommandAst, act: UpdateAct) -> bool {
    use crate::ua_select::{self, edit_utime, find_arg, find_arg_with_value};
    const KEYWORDS: [&str; 17] = ["VolStatus", "VolRetention", "VolUse", "MaxVolJobs", "MaxVolFiles", "MaxVolBytes",
        "Recycle", "InChanger", "Slot", "Pool", "FromPool", "AllFromPool", "Enabled", "RecyclePool",
        "ActionOnPurge", "Storage", "Encrypt"];
    // Argument field index -> update_field index.
    const FIELD: [usize; 17] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let mut done = false;
    for (index, keyword) in KEYWORDS.iter().enumerate() {
        let Some(value) = find_arg_with_value(ast, keyword) else { continue };
        let media = if index != 11 {
            match ua_select::select_media_dbr(ua, ast) {
                Some(media) => media,
                None => return false,
            }
        } else {
            ua_select::MediaRecord::default()
        };
        update_field(ua, ast, act, FIELD[index], value, &media);
        if index == 10 || index == 11 {
            return true;
        }
        done = true;
    }
    if find_arg(ast, "fromallpools").is_some() {
        act(ua, UpdateAction::Unsupported("update volume fromallpools"));
        return true;
    }
    while !done {
        ua.start_prompt("Parameters to modify:\n");
        for item in ["Volume Status", "Volume Retention Period", "Volume Use Duration", "Maximum Volume Jobs",
            "Maximum Volume Files", "Maximum Volume Bytes", "Recycle Flag", "Slot", "InChanger Flag",
            "Volume Files", "Pool", "Volume from Pool", "All Volumes from Pool", "All Volumes from all Pools",
            "Enabled", "RecyclePool", "Action On Purge", "Storage", "Done"] {
            ua.add_prompt(item);
        }
        let choice = ua.do_prompt("", "Select parameter to modify").map(|(i, _)| i as i64).unwrap_or(-1);
        let mut media = ua_select::MediaRecord::default();
        if choice != 12 && choice != 13 && choice != 18 {
            match ua_select::select_media_dbr(ua, ast) {
                Some(record) => media = record,
                None => return false,
            }
            ua.info_msg(format!("Updating Volume \"{}\"\n", media.volume_name));
        }
        match choice {
            0 => {
                ua.info_msg(format!("Current Volume status is: {}\n", media.vol_status));
                ua.start_prompt("Possible Values are:\n");
                for status in ["Append", "Archive", "Disabled", "Full", "Used", "Cleaning"] {
                    ua.add_prompt(status);
                }
                if media.vol_status == "Purged" {
                    ua.add_prompt("Recycle");
                }
                ua.add_prompt("Read-Only");
                let Some((_, status)) = ua.do_prompt("", "Choose new Volume Status") else { return true };
                update_field(ua, ast, act, 0, &status, &media);
            }
            1 => {
                ua.info_msg(format!("Current retention period is: {}\n", edit_utime(media.vol_retention)));
                let Some(value) = ua.get_cmd("Enter Volume Retention period: ") else { return false };
                update_field(ua, ast, act, 1, &value, &media);
            }
            2 => {
                ua.info_msg(format!("Current use duration is: {}\n", edit_utime(media.vol_use_duration)));
                let Some(value) = ua.get_cmd("Enter Volume Use Duration: ") else { return false };
                update_field(ua, ast, act, 2, &value, &media);
            }
            3 => {
                ua.info_msg(format!("Current max jobs is: {}\n", media.max_vol_jobs));
                if ua.get_pint("Enter new Maximum Jobs: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, ast, act, 3, &value, &media);
            }
            4 => {
                ua.info_msg(format!("Current max files is: {}\n", media.max_vol_files));
                if ua.get_pint("Enter new Maximum Files: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, ast, act, 4, &value, &media);
            }
            5 => {
                ua.info_msg(format!("Current value is: {}\n", media.max_vol_bytes));
                let Some(value) = ua.get_cmd("Enter new Maximum Bytes: ") else { return false };
                update_field(ua, ast, act, 5, &value, &media);
            }
            6 => {
                ua.info_msg(format!("Current recycle flag is: {}\n", if media.recycle == 1 { "yes" } else { "no" }));
                if ua.get_yesno("Enter new Recycle status: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, ast, act, 6, &value, &media);
            }
            7 => {
                ua.info_msg(format!("Current Slot is: {}\n", media.slot));
                if ua.get_pint("Enter new Slot: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, ast, act, 8, &value, &media);
            }
            8 => {
                ua.info_msg(format!("Current InChanger flag is: {}\n", media.in_changer));
                let question = format!("Set InChanger flag for Volume \"{}\": yes/no: ", media.volume_name);
                let Some(flag) = ua.get_yesno(&question) else { return false };
                act(ua, UpdateAction::Media { media_id: media.media_id, column: "InChanger",
                    value: i32::from(flag).to_string(), message: format!("New InChanger flag is: {}\n", i32::from(flag)) });
            }
            9 => {
                ua.warning_msg("Warning changing Volume Files can result\nin loss of data on your Volume\n\n");
                ua.info_msg(format!("Current Volume Files is: {}\n", media.vol_files));
                let Some(files) = ua.get_pint("Enter new number of Files for Volume: ") else { return false };
                if i64::from(files) != media.vol_files + 1 {
                    ua.warning_msg("Normally, you should only increase Volume Files by one!\n");
                    if ua.get_yesno("Increase Volume Files? (yes/no): ") != Some(true) {
                        continue;
                    }
                }
                act(ua, UpdateAction::Media { media_id: media.media_id, column: "VolFiles",
                    value: files.to_string(), message: format!("New Volume Files is: {files}\n") });
            }
            10 => {
                let pool = ua.ctx.catalog.as_ref()
                    .and_then(|cat| cat.query_rows(&format!("SELECT Name FROM Pool WHERE PoolId={}", media.pool_id)).ok())
                    .and_then(|t| t.rows.into_iter().next())
                    .and_then(|row| row.into_iter().next().flatten());
                let Some(pool) = pool else {
                    ua.error_msg("Pool record not found in Catalog.\n");
                    return false;
                };
                ua.info_msg(format!("Current Pool is: {pool}\n"));
                let Some(name) = ua.get_cmd("Enter new Pool name: ") else { return false };
                update_field(ua, ast, act, 9, &name, &media);
                return true;
            }
            11 => {
                act(ua, UpdateAction::Unsupported("update volume frompool"));
                return true;
            }
            12 => {
                if let Some(pool) = ua_select::select_pool_resource(ua) {
                    update_field(ua, ast, act, 11, &pool, &media);
                }
                return true;
            }
            13 => {
                act(ua, UpdateAction::Unsupported("update volume fromallpools"));
                return true;
            }
            14 => {
                ua.info_msg(format!("Current Enabled is: {}\n", media.enabled));
                let Some(value) = ua.get_cmd("Enter new Enabled: ") else { return false };
                update_field(ua, ast, act, 12, &value, &media);
            }
            15 => {
                let current = ua.ctx.catalog.as_ref()
                    .and_then(|cat| cat.query_rows(&format!("SELECT Name FROM Pool WHERE PoolId={}", media.recycle_pool_id)).ok())
                    .and_then(|t| t.rows.into_iter().next())
                    .and_then(|row| row.into_iter().next().flatten());
                match current {
                    Some(pool) => ua.info_msg(format!("Current RecyclePool is: {pool}\n")),
                    None => ua.info_msg("No current RecyclePool\n"),
                }
                if ua_select::select_pool_dbr(ua, ast, "recyclepool").is_none() { return false; }
                act(ua, UpdateAction::Unsupported("update volume recyclepool"));
                return true;
            }
            16 => {
                let current = if media.action_on_purge & 1 != 0 { "Truncate" } else { "None" };
                ua.info_msg(format!("Current ActionOnPurge is: {current}\n"));
                if ua.get_cmd("Enter new ActionOnPurge (one of: Truncate, None): ").is_none() { return false; }
                act(ua, UpdateAction::Unsupported("update volume actiononpurge"));
            }
            17 => {
                act(ua, UpdateAction::Unsupported("update volume storage"));
                return true;
            }
            _ => {
                ua.info_msg("Selection terminated.\n");
                return true;
            }
        }
    }
    true
}

/// The catalog work of `update`.
fn update_execute(ua: &mut crate::ua_select::Ua, action: UpdateAction) {
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    match action {
        UpdateAction::Media { media_id, column, value, message } => {
            // SqlQuery: "Query failed: <query>: ERR=<PQerrorMessage>\n".
            let sql = format!("UPDATE Media SET {column}={value} WHERE MediaId={media_id}");
            match cat.query_rows(&sql) {
                Ok(_) => ua.info_msg(message),
                Err(e) => ua.error_msg(format!("Query failed: {sql}: ERR={e}\n")),
            }
        }
        UpdateAction::Pool(pool_name) => {
            let pool = ua.ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|c| c.find_pool(&pool_name).cloned());
            if let Some(pool) = pool {
                let sql = format!(
                    "UPDATE pool SET pooltype='{}', maxvols={}, autoprune={}, volretention={} WHERE name='{}'",
                    pool.pool_type, pool.maximum_volumes, i32::from(pool.auto_prune),
                    pool.volume_retention.as_secs(), pool_name.replace('\'', "''"));
                if let Err(e) = cat.raw_query(&sql) {
                    ua.error_msg(format!("UpdatePoolRecord returned 0. ERR={e}\n"));
                }
            }
            if let Ok(table) = cat.query_rows(&format!(
                "SELECT PoolId,Name,NumVols,MaxVols,UseOnce,UseCatalog,AcceptAnyVolume,VolRetention,VolUseDuration,\
                 MaxVolJobs,MaxVolBytes,AutoPrune,Recycle,PoolType,LabelFormat,Enabled,ScratchPoolId,RecyclePoolId,\
                 LabelType FROM Pool WHERE Name='{}'", pool_name.replace('\'', "''"))) {
                ua.send_msg(crate::sql_list::render(&table, crate::sql_list::ListType::Horz));
            }
            ua.info_msg("Pool DB record updated from resource.\n");
        }
        UpdateAction::Slots { storage, args } => {
            use crate::ua_storage_select::{Env, NetSd};
            let Some(env) = Env::load(ua.ctx) else { return };
            let Some(store) = env.storages.iter().find(|s| s.0.name == storage).map(|s| s.0.clone()) else { return };
            let args: Vec<crate::command_ast::Argument> = args.into_iter()
                .map(|(key, value)| crate::command_ast::Argument { key, value }).collect();
            let director = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
                .map(|c| c.director.name.clone()).unwrap_or_default();
            update_slots(ua, &args, &env, &store, &mut NetSd { director });
        }
        UpdateAction::Unsupported(what) => ua.send_msg(format!("{what}: not supported\n")),
    }
}

/// C++ UpdateSlots() (ua_update.cc:1075) after the storage selection:
/// slot count and volume list from the SD (`scan`: the label of every
/// slot), then Slot/InChanger/StorageId of the catalog volumes
/// (compatibility log C-054).
fn update_slots(ua: &mut crate::ua_select::Ua, args: &[crate::command_ast::Argument],
    env: &crate::ua_storage_select::Env, store: &bareos_config::resources::StorageResource,
    sd: &mut impl crate::ua_storage_select::SdLink) {
    use crate::ua_storage_select as storage;
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    let scan = args.iter().any(|a| a.key.eq_ignore_ascii_case("scan"));
    // kInvalidDriveNumber (uint16_t max) is sent as %hd.
    let mut drive: i16 = -1;
    if scan {
        drive = storage::get_storage_drive(ua, args, env, sd, store).unwrap_or(-1);
    }
    let mut enabled = None;
    // FindArgWithValue(): the first "Enabled" argument decides.
    if let Some(value) = args.iter().find(|a| a.key.eq_ignore_ascii_case("enabled"))
        .and_then(|a| a.value.as_deref()) {
        match get_enabled(ua, value) {
            Some(value) => enabled = Some(value),
            None => return,
        }
    }

    // slot_number_t is uint16_t.
    let max_slots = storage::get_num_slots(ua, sd, store) as u16;
    if max_slots == 0 {
        ua.warning_msg("No slots in changer to scan.\n");
        return;
    }
    let Some(mut slot_list) = storage::get_user_slot_list(args, "slots", i32::from(max_slots)) else { return };

    let Some(vol_list) = storage::get_vol_list_from_storage(ua, sd, store, false, scan, false) else {
        ua.warning_msg("No Volumes found to update, or no barcodes.\n");
        return;
    };

    // store->StorageId: the catalog record of the storage (0 without one).
    let storage_id = crate::ua_select::get_storage_record(ua.ctx, &store.name).unwrap_or(0);
    let query = |sql: &str| cat.query_rows(sql);

    // First zap out any InChanger with StorageId=0
    let _ = query("UPDATE Media SET InChanger=0 WHERE StorageId=0");

    for vl in &vol_list {
        let slot = vl.slot;
        if slot > max_slots {
            ua.warning_msg(format!("Slot {slot} greater than max {max_slots} ignored.\n"));
            continue;
        }
        // Check if user wants us to look at this slot
        let index = usize::from(slot.wrapping_sub(1));
        if !slot_list.get(index).copied().unwrap_or(false) {
            continue;
        }
        // If scanning, we read the label rather than the barcode
        let vol_name = if scan {
            storage::get_volume_name_from_sd(ua, sd, store, slot, drive)
        } else {
            vl.vol_name.clone()
        };
        slot_list[index] = false;

        // MakeInchangerUnique with MediaId 0 (by VolumeName).
        let vol = vol_name.as_deref().unwrap_or("");
        let _ = make_inchanger_unique(&query, slot, storage_id, None, vol);

        let Some(vol) = vol_name else {
            ua.info_msg(format!("No VolName for Slot={slot} InChanger set to zero.\n"));
            continue;
        };

        // GetMediaRecord by VolumeName.
        let record = query(&format!(
            "SELECT MediaId,VolumeName,Slot,InChanger,StorageId FROM Media WHERE VolumeName='{}'", slots_sql_escape(&vol)))
            .ok().filter(|t| t.rows.len() == 1).and_then(|t| t.rows.into_iter().next());
        let Some(row) = record else {
            ua.warning_msg(format!("Volume \"{vol}\" not found in catalog. Slot={slot} InChanger set to zero.\n"));
            continue;
        };
        let field = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        let number = |i: usize| field(i).parse::<i64>().unwrap_or(0);
        let (media_id, name) = (number(0), field(1));
        if number(2) != i64::from(slot) || number(3) == 0 || number(4) != storage_id {
            // UpdateMediaRecord(): the changed columns, then MakeInchangerUnique.
            let enabled = enabled.map(|e| format!(",Enabled={e}")).unwrap_or_default();
            let sql = format!("UPDATE Media SET Slot={slot},InChanger=1,StorageId={storage_id}{enabled} \
                WHERE VolumeName='{}'", slots_sql_escape(&name));
            let result = query(&sql).map_err(|e| format!("update {sql} failed:\n{e}\n"))
                .and_then(|_| make_inchanger_unique(&query, slot, storage_id, Some(media_id), &name));
            match result {
                Ok(()) => ua.info_msg(format!(
                    "Catalog record for Volume \"{name}\" updated to reference slot {slot}.\n")),
                Err(error) => ua.error_msg(error),
            }
        } else {
            ua.info_msg(format!("Catalog record for Volume \"{name}\" is up to date.\n"));
        }
    }

    // Any slot not visited gets it Inchanger flag reset.
    for slot in 1..=max_slots {
        if slot_list.get(usize::from(slot - 1)).copied().unwrap_or(false)
            && make_inchanger_unique(&query, slot, storage_id, None, "").is_err() {
            ua.warning_msg(format!("Could not set inchanger to 0 for slot {slot}.\n"));
        }
    }
}

/// EscapeString() for PostgreSQL (standard_conforming_strings).
fn slots_sql_escape(text: &str) -> String {
    text.replace('\'', "''")
}

/// BareosDb::MakeInchangerUnique() (sql_update.cc:418) for a record with
/// InChanger=1: no other volume of this storage keeps the slot.
fn make_inchanger_unique(query: &impl Fn(&str) -> Result<crate::catalog_worker::SqlTable, String>, slot: u16,
    storage_id: i64, media_id: Option<i64>, vol_name: &str) -> Result<(), String> {
    if slot == 0 || storage_id == 0 {
        return Ok(());
    }
    let sql = match media_id {
        Some(id) if id != 0 => format!(
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={slot} AND StorageId={storage_id} AND MediaId!={id}"),
        _ if !vol_name.is_empty() => format!(
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={slot} AND StorageId={storage_id} AND VolumeName!='{}'",
            slots_sql_escape(vol_name)),
        _ => format!("UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={slot} AND StorageId={storage_id}"),
    };
    query(&sql).map(|_| ()).map_err(|e| format!("update {sql} failed:\n{e}\n"))
}

pub fn cmd_estimate(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let job = get_arg(ast, "job");
    let client = get_arg(ast, "client");
    let listing = has_flag(ast, "listing");

    let job_name = match job {
        Some(j) => j,
        None => {
            if let Some(config) = &ctx.config {
                if let Ok(cfg) = config.read() {
                    let names: Vec<_> = cfg.jobs.iter()
                        .filter(|j| matches!(j.job_type, bareos_config::resources::JobType::Backup))
                        .map(|j| j.name.as_str())
                        .collect();
                    return CommandResult::ok(format!(
                        "estimate: specify job=<name>\nBackup jobs: {}\n",
                        names.join(", ")
                    ));
                }
            }
            return CommandResult::ok("estimate: specify job=<name>\n".to_string());
        }
    };

    // Look up job
    let (client_name, fileset_name) = if let Some(config) = &ctx.config {
        if let Ok(cfg) = config.read() {
            if let Some(j) = cfg.find_job(job_name) {
                (j.client.clone(), j.fileset.clone())
            } else {
                return CommandResult::error(format!("Job '{job_name}' not found\n"));
            }
        } else {
            (String::new(), String::new())
        }
    } else {
        (String::new(), String::new())
    };

    let client_name = client.unwrap_or(&client_name);
    let prefix = match crate::ua_db::reopen_client_db(ast, ctx, client_name) {
        Ok(prefix) => prefix,
        Err(result) => return result,
    };

    let mut output = prefix + &format!(
        "Connecting to Client \"{client_name}\" for estimate...\n\
         Job: {job_name}\n\
         FileSet: {fileset_name}\n"
    );

    if listing {
        output.push_str("Listing mode enabled\n");
    }

    // TODO: Connect to FD and run estimate
    output.push_str("Estimate pending — requires FD connection for file listing\n");

    CommandResult::ok(output)
}

pub fn cmd_prune(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_prune.cc PruneCmd: OpenClientDb(ua, true) after the Job/Storage ACL restriction checks.
    // Job/Storage restrictions are refused before the catalog is opened.
    if let Some(denied) = denied_restriction(ctx, &[(crate::acl::Kind::Job, "job"), (crate::acl::Kind::Storage, "storage")]) {
        return denied;
    }
    let mut ua = crate::ua_select::Ua::new(ctx);
    let ok = prune_dialog(&mut ua, ast, prune_execute);
    ua.finish(ok)
}

/// `Permission denied: need full %s permission.` (ua_prune.cc:370, ua_purge.cc:71).
fn denied_restriction(ctx: &ExecContext, kinds: &[(crate::acl::Kind, &str)]) -> Option<CommandResult> {
    kinds.iter().find(|(kind, _)| crate::acl::has_restrictions(ctx, *kind)).map(|(_, name)|
        CommandResult::error(format!("Permission denied: need full {name} permission.\n")))
}

/// First argument naming one of `keywords` (FindArgKeyword, case-insensitive).
fn find_arg_keyword(ast: &CommandAst, keywords: &[&str]) -> Option<usize> {
    ast.arguments.iter().find_map(|a| keywords.iter().position(|k| a.key.eq_ignore_ascii_case(k)))
}

/// What a confirmed prune does.
#[derive(Debug, PartialEq, Eq)]
enum PruneAction {
    Files { client: String, pool: Option<String>, retention: u64 },
    Jobs { client: String, pool: Option<String>, retention: u64 },
    Volume(String),
    Stats(u64),
    Directory { client: Option<String>, path: String, recursive: bool },
}

/// PruneCmd (ua_prune.cc:362) dialogs: keyword menu, client/pool selection,
/// volume selection and ConfirmRetention ("Continue? (yes/mod/no): ", only
/// `yes` skips it). `act` runs only for a confirmed request.
fn prune_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    mut act: impl FnMut(&mut crate::ua_select::Ua, PruneAction) -> bool) -> bool {
    use crate::acl::{has_restrictions, Kind};
    use crate::ua_select::{self, find_arg, find_arg_with_value};
    const KEYWORDS: [&str; 5] = ["Files", "Jobs", "Volume", "Stats", "Directory"];
    let ctx = ua.ctx;
    let denied = |ua: &mut crate::ua_select::Ua, what: &str| {
        ua.error_msg(format!("Permission denied: need full {what} permission.\n"));
        false
    };
    if !ua.open_client_db(ast, true) {
        return false;
    }
    let kw = find_arg_keyword(ast, &KEYWORDS)
        .or_else(|| ast.arguments.iter().any(|a| a.key.len() >= 6 && a.key.as_bytes()[..6].eq_ignore_ascii_case(b"volume")).then_some(2))
        .or_else(|| ua.do_keyword_prompt("Choose item to prune", &KEYWORDS));
    // get_pool_resource when pool= is given or the Pool ACL is restricted.
    let pool_of = |ua: &mut crate::ua_select::Ua| {
        if find_arg_with_value(ast, "pool").is_some() || has_restrictions(ctx, Kind::Pool) {
            ua_select::get_pool_resource(ua, ast)
        } else {
            None
        }
    };
    match kw {
        Some(index @ (0 | 1)) => {
            let Some(client) = ua_select::get_client_resource(ua, ast) else { return false };
            let pool = pool_of(ua);
            let (what, directive, default) = if index == 0 {
                ("File", "fileretention", 60 * 86_400)
            } else {
                ("Job", "jobretention", 180 * 86_400)
            };
            // Pool retention takes precedence over the client's.
            let pool_retention = pool.as_deref()
                .map(|pool| ua_select::resource_retention(ctx, "pool", pool, directive, 0)).unwrap_or(0);
            let retention = if pool_retention > 0 {
                pool_retention
            } else {
                ua_select::resource_retention(ctx, "client", &client, directive, default)
            };
            let Some(retention) = ua.confirm_retention(ast, retention, what) else { return false };
            if index == 0 {
                act(ua, PruneAction::Files { client, pool, retention });
                true
            } else {
                act(ua, PruneAction::Jobs { client, pool, retention })
            }
        }
        Some(2) => {
            if find_arg(ast, "all").is_some() {
                // PruneAllVolumes (ua_prune.cc:326): every volume (of the
                // pool) with its own ConfirmRetention.
                let pool = pool_of(ua);
                let names = ctx.catalog.as_ref()
                    .and_then(|cat| cat.query_rows("SELECT DISTINCT Media.VolumeName FROM Media ORDER BY VolumeName").ok())
                    .map(|t| t.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>());
                let Some(names) = names else { return false };
                let mut result = true;
                for volume in names {
                    let Some(record) = select_media_dbr_by_name(ua, &volume) else { return false };
                    let Some(pool_name) = select_pool_for_media_dbr(ua, &record) else { return false };
                    if pool.as_deref().is_some_and(|p| p != pool_name) {
                        continue;
                    }
                    if record.enabled == 2 {
                        ua.error_msg(format!("Cannot prune Volume \"{}\" because it is archived.\n", record.volume_name));
                        result = false;
                        continue;
                    }
                    if ua.confirm_retention(ast, record.vol_retention, &format!("Volume ({volume})")).is_none() {
                        result = false;
                        break;
                    }
                    result &= act(ua, PruneAction::Volume(record.volume_name));
                }
                return result;
            }
            let mut volumes: Vec<String> = Vec::new();
            for arg in ast.arguments.iter().filter(|a| a.key.eq_ignore_ascii_case("volume")) {
                if let Some(name) = &arg.value {
                    if !volumes.contains(name) {
                        volumes.push(name.clone());
                    }
                }
            }
            let pool = if find_arg_with_value(ast, "pool").is_some() || has_restrictions(ctx, Kind::Pool) {
                match ua_select::get_pool_resource(ua, ast) {
                    Some(pool) => Some(pool),
                    None => return false,
                }
            } else {
                None
            };
            if has_restrictions(ctx, Kind::Client) {
                return denied(ua, "client");
            }
            if let (true, Some(pool)) = (volumes.is_empty(), pool.as_deref()) {
                match select_prunable_volumes(ua, pool) {
                    Some(selected) if selected.is_empty() => return true,
                    Some(selected) => volumes = selected,
                    None => return false,
                }
            }
            if !volumes.is_empty() {
                let mut result = true;
                for volume in volumes {
                    let Some(record) = select_media_dbr_by_name(ua, &volume) else { return false };
                    let Some(pool_name) = select_pool_for_media_dbr(ua, &record) else { return false };
                    if let Some(pool) = pool.as_deref().filter(|p| *p != pool_name) {
                        ua.error_msg(format!("Volume \"{}\" is not in Pool \"{pool}\".\n", record.volume_name));
                        result = false;
                        continue;
                    }
                    if record.enabled == 2 {
                        ua.error_msg(format!("Cannot prune Volume \"{}\" because it is archived.\n", record.volume_name));
                        result = false;
                        continue;
                    }
                    if ua.confirm_retention(ast, record.vol_retention, &format!("Volume ({volume})")).is_none() {
                        result = false;
                        continue;
                    }
                    result &= act(ua, PruneAction::Volume(record.volume_name));
                }
                return result;
            }
            let Some(record) = ua_select::select_media_dbr(ua, ast) else { return false };
            if select_pool_for_media_dbr(ua, &record).is_none() {
                return false;
            }
            if record.enabled == 2 {
                ua.error_msg(format!("Cannot prune Volume \"{}\" because it is archived.\n", record.volume_name));
                return false;
            }
            if ua.confirm_retention(ast, record.vol_retention, "Volume").is_none() {
                return false;
            }
            act(ua, PruneAction::Volume(record.volume_name))
        }
        Some(3) => {
            // Director "Statistics Retention", default 5 * 12 * 31 days.
            let retention = director_duration(ctx, "statisticsretention", 5 * 12 * 31 * 86_400);
            if retention == 0 {
                return false;
            }
            if has_restrictions(ctx, Kind::Client) {
                return denied(ua, "client");
            }
            if has_restrictions(ctx, Kind::Pool) {
                return denied(ua, "pool");
            }
            let Some(retention) = ua.confirm_retention(ast, retention, "Statistics") else { return false };
            act(ua, PruneAction::Stats(retention))
        }
        Some(4) => {
            if has_restrictions(ctx, Kind::Pool) {
                return denied(ua, "pool");
            }
            let mut client = None;
            if find_arg_with_value(ast, "client").is_some() || has_restrictions(ctx, Kind::Client) {
                client = Some(match ua_select::get_client_resource(ua, ast) {
                    Some(client) => client,
                    None => return false,
                });
            }
            // PruneDirectory (ua_prune.cc:684)
            if client.is_none() && ua.get_yesno(
                "No client restriction given really remove directory for all clients (yes/no): ") != Some(true) {
                client = Some(match ua_select::get_client_resource(ua, ast) {
                    Some(client) => client,
                    None => return false,
                });
            }
            let recursive = find_arg(ast, "recursive").is_some();
            let path = match find_arg_with_value(ast, "directory") {
                Some(path) => path.to_string(),
                None => {
                    let prompt = if recursive {
                        "Please enter the full path prefix to remove: "
                    } else {
                        "Please enter the full path to remove: "
                    };
                    let Some(path) = ua.get_cmd(prompt) else { return false };
                    path
                }
            };
            act(ua, PruneAction::Directory { client, path, recursive })
        }
        _ => true,
    }
}

/// C++ SelectMediaDbrByName(): invalid names fail silently, a missing
/// record is reported.
fn select_media_dbr_by_name(ua: &mut crate::ua_select::Ua, name: &str) -> Option<crate::ua_select::MediaRecord> {
    if !crate::acl::valid_name(name) {
        return None;
    }
    match crate::ua_select::get_media_record(ua.ctx, 0, name) {
        Ok(record) => Some(record),
        Err(error) => {
            ua.error_msg(error);
            None
        }
    }
}

/// C++ SelectPoolForMediaDbr(): the name of the volume's pool (Pool ACL).
fn select_pool_for_media_dbr(ua: &mut crate::ua_select::Ua, record: &crate::ua_select::MediaRecord) -> Option<String> {
    let name = ua.ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows(&format!("SELECT Name FROM Pool WHERE PoolId={}", record.pool_id)).ok())
        .and_then(|t| t.rows.into_iter().next())
        .and_then(|row| row.into_iter().next().flatten());
    let Some(name) = name else {
        ua.error_msg(format!("Pool record not found in Catalog.\n"));
        return None;
    };
    if !crate::acl::access(ua.ctx, crate::acl::Kind::Pool, &name) {
        ua.error_msg(format!("No access to Pool \"{name}\"\n"));
        return None;
    }
    Some(name)
}

/// `prune volume pool=<pool>` without volumes (ua_prune.cc:486): the
/// prunable volume report and the selection. Some(empty) when the pool has
/// no prunable volume, None when nothing is to be done.
fn select_prunable_volumes(ua: &mut crate::ua_select::Ua, pool: &str) -> Option<Vec<String>> {
    use super::purge;
    let pool_id = match crate::ua_select::get_pool_record(ua.ctx, pool) {
        Ok(id) => id,
        Err(_) => {
            ua.error_msg(format!("Pool {pool} doesn't exist.\n"));
            return None;
        }
    };
    let connection = ua.ctx.catalog.as_ref().and_then(|cat| cat.dedicated().ok());
    let report = connection.as_ref().and_then(|db| purge::pool_prune_report(ua, db, pool_id));
    let Some(report) = report else {
        ua.error_msg(format!("Failed to compute prune details for pool {pool}.\n"));
        return None;
    };
    drop(connection);
    if report.is_empty() {
        ua.send_msg(format!("No prunable volumes found in Pool \"{pool}\".\n"));
        return Some(Vec::new());
    }
    ua.send_msg(format!("Prunable volumes in Pool \"{pool}\" (oldest data first):\n"));
    for (index, volume) in report.iter().enumerate() {
        ua.send_msg(purge::report_line(index + 1, volume));
    }
    let selection = ua.get_cmd("Enter volume numbers or names separated by commas (or \"all\"): ")?;
    let selection = selection.trim();
    if selection.is_empty() {
        ua.error_msg("No volumes selected.\n");
        return None;
    }
    if selection.eq_ignore_ascii_case("all") {
        return Some(report.into_iter().map(|v| v.name).collect());
    }
    let mut selected: Vec<String> = Vec::new();
    for token in selection.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let mut name = token.to_string();
        if crate::ua_select::is_a_number(token) {
            let index = crate::commands::job::str_to_int64(token);
            if index < 1 || index > report.len() as i64 {
                ua.error_msg(format!("Invalid prune selection \"{token}\".\n"));
                return None;
            }
            name = report[index as usize - 1].name.clone();
        }
        if selected.contains(&name) {
            continue;
        }
        if !report.iter().any(|v| v.name == name) {
            ua.error_msg(format!("Volume \"{name}\" is not currently prunable in Pool \"{pool}\".\n"));
            return None;
        }
        selected.push(name);
    }
    Some(selected)
}

/// Duration directive of the Director resource (raw configuration).
fn director_duration(ctx: &ExecContext, directive: &str, default: u64) -> u64 {
    ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|config| {
        config.raw.iter().find(|r| r.norm_type() == "director")
            .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == directive))
            .and_then(|d| crate::commands::show::parse_duration(&d.values().join(" ")))
    }).unwrap_or(default)
}

/// A private catalog connection for the catalog work (temporary tables of
/// PruneJobs and UpgradeCopies need one connection).
fn private_db(ua: &mut crate::ua_select::Ua) -> Option<crate::catalog_worker::CatalogConnection> {
    let connection = ua.ctx.catalog.as_ref().map(|cat| cat.dedicated());
    match connection {
        Some(Ok(connection)) => Some(connection),
        Some(Err(error)) => {
            ua.error_msg(format!("{error}\n"));
            None
        }
        None => {
            ua.send_msg("No catalog configured\n");
            None
        }
    }
}

/// The catalog work of a confirmed prune (commands/purge.rs).
fn prune_execute(ua: &mut crate::ua_select::Ua, action: PruneAction) -> bool {
    use super::purge;
    let Some(db) = private_db(ua) else { return false };
    match action {
        PruneAction::Files { client, pool, retention } => {
            purge::prune_files(ua, &db, &client, pool.as_deref(), retention);
            true
        }
        PruneAction::Jobs { client, pool, retention } => purge::prune_jobs(ua, &db, &client, pool.as_deref(), retention),
        PruneAction::Volume(name) => match purge::get_volume_by_name(&db, &name) {
            Ok(mut volume) => purge::prune_volume(ua, &db, &mut volume),
            Err(error) => {
                ua.error_msg(error);
                false
            }
        },
        PruneAction::Stats(retention) => {
            purge::prune_stats(ua, &db, retention);
            true
        }
        PruneAction::Directory { client, path, recursive } => purge::prune_directory(ua, &db, client.as_deref(), &path, recursive),
    }
}

pub fn cmd_purge(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    let ok = purge_dialog(&mut ua, ast, |ua, action| purge_execute(ua, ast, action));
    ua.finish(ok)
}

/// What a purge does once its target is known (the confirmation follows
/// in purge_execute once the jobs are counted).
#[derive(Debug, PartialEq, Eq)]
enum PurgeAction {
    FilesOfJob(i64),
    FilesOfClient(String),
    JobsOfClient(String),
    JobsOfVolume(String),
    JobsOfPool(String),
    QuotaOfClient(String),
    /// ActionOnPurgeCmd on the restored arguments (key, value).
    ActionOnPurge(Vec<(String, Option<String>)>),
}

/// The console line of the command (C++ `ua->cmd`); commands that did not
/// come from a text line get it rebuilt from their arguments.
fn command_line(ast: &CommandAst) -> String {
    if !ast.raw_line.is_empty() {
        return ast.raw_line.clone();
    }
    let quote = |text: &str| if text.contains(|c: char| c.is_ascii_whitespace()) { format!("\"{text}\"") } else { text.to_string() };
    let mut line = ast.command_name.clone();
    for arg in &ast.arguments {
        line.push(' ');
        line.push_str(&quote(&arg.key));
        if let Some(value) = &arg.value {
            line.push('=');
            line.push_str(&quote(value));
        }
    }
    line
}

/// PurgeCmd (ua_purge.cc:65): warning, ACL checks, OpenClientDb, then the
/// keyword cases; anything incomplete ends in "Choose item to purge". Every
/// target is named or selected before `act` runs.
fn purge_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    mut act: impl FnMut(&mut crate::ua_select::Ua, PurgeAction)) -> bool {
    use crate::acl::{has_restrictions, Kind};
    use crate::ua_select;
    const KEYWORDS: [&str; 4] = ["files", "jobs", "volume", "quota"];
    let ctx = ua.ctx;
    ua.warning_msg("\nThis command can be DANGEROUS!!!\n\n\
        It purges (deletes) all Files from a Job,\n\
        JobId, Client or Volume; or it purges (deletes)\n\
        all Jobs from a Client or Volume without regard\n\
        to retention periods. Normally you should use the\n\
        PRUNE command, which respects retention periods.\n\
        This command requires full access to all resources.\n");
    for (kind, name) in [(Kind::Client, "client"), (Kind::Job, "job"), (Kind::Pool, "pool"), (Kind::Storage, "storage")] {
        if has_restrictions(ctx, kind) {
            ua.error_msg(format!("Permission denied: need full {name} permission.\n"));
            return false;
        }
    }
    if !ua.open_client_db(ast, true) {
        return true;
    }
    match find_arg_keyword(ast, &KEYWORDS) {
        Some(0) => match find_arg_keyword(ast, &["Job", "JobId", "Client", "Volume"]) {
            Some(0 | 1) => {
                if let Some(jobid) = get_job_dbr(ua, ast) {
                    act(ua, PurgeAction::FilesOfJob(jobid));
                }
                return true;
            }
            Some(2) => {
                if let Some(client) = ua_select::get_client_resource(ua, ast) {
                    act(ua, PurgeAction::FilesOfClient(client));
                }
                return true;
            }
            Some(_) => return true, // Volume: not implemented upstream
            None => {}
        },
        Some(1) => match find_arg_keyword(ast, &["Client", "Volume", "Pool"]) {
            Some(0) => {
                if let Some(client) = ua_select::get_client_resource(ua, ast) {
                    act(ua, PurgeAction::JobsOfClient(client));
                }
                return true;
            }
            Some(1) => {
                if let Some(media) = ua_select::select_media_dbr(ua, ast) {
                    act(ua, PurgeAction::JobsOfVolume(media.volume_name));
                }
                return true;
            }
            Some(_) => {
                if let Some(pool) = ua_select::select_pool_dbr(ua, ast, "pool") {
                    act(ua, PurgeAction::JobsOfPool(pool));
                }
                return true;
            }
            None => {}
        },
        Some(2) => {
            // One SelectMediaDbr per `volume` keyword (each seen keyword is
            // zapped, so the next lookup starts behind it).
            // cmd_holder: the command line, plus " volume=<name>" for every
            // selection once ua->cmd holds an answer instead of the line.
            let mut holder = command_line(ast);
            ua.cmd = holder.clone();
            let positions: Vec<usize> = ast.arguments.iter().enumerate()
                .filter(|(_, a)| a.key.eq_ignore_ascii_case("volume")).map(|(i, _)| i).collect();
            for position in positions {
                let rest = CommandAst { arguments: ast.arguments[position..].to_vec(), ..ast.clone() };
                let name = match ua_select::select_media_dbr_named(ua, &rest) {
                    Ok(media) => {
                        let name = media.volume_name.clone();
                        act(ua, PurgeAction::JobsOfVolume(media.volume_name));
                        name
                    }
                    Err(name) => name,
                };
                ua.send_msg("\n");
                if ua.cmd != holder {
                    holder.push_str(" volume=");
                    holder.push_str(&name);
                }
            }
            // ParseArgs(cmd_holder); ActionOnPurgeCmd (action=truncate).
            let restored = crate::session::parse_cli_command(&holder);
            if ua_select::find_arg(&restored, "action").is_some() {
                act(ua, PurgeAction::ActionOnPurge(restored.arguments.iter()
                    .map(|a| (a.key.clone(), a.value.clone())).collect()));
            }
            return true;
        }
        Some(_) => {
            if find_arg_keyword(ast, &["Client"]).is_some() {
                if let Some(client) = ua_select::get_client_resource(ua, ast) {
                    act(ua, PurgeAction::QuotaOfClient(client));
                }
                return true;
            }
        }
        None => {}
    }
    match ua.do_keyword_prompt("Choose item to purge", &KEYWORDS) {
        Some(0) => {
            if let Some(client) = ua_select::get_client_resource(ua, ast) {
                act(ua, PurgeAction::FilesOfClient(client));
            }
        }
        Some(1) => {
            if let Some(client) = ua_select::get_client_resource(ua, ast) {
                act(ua, PurgeAction::JobsOfClient(client));
            }
        }
        Some(2) => {
            if let Some(media) = ua_select::select_media_dbr(ua, ast) {
                act(ua, PurgeAction::JobsOfVolume(media.volume_name));
            }
        }
        Some(_) => {
            if let Some(client) = ua_select::get_client_resource(ua, ast) {
                act(ua, PurgeAction::QuotaOfClient(client));
            }
        }
        None => {}
    }
    true
}

/// C++ ua_select.cc GetJobDbr(): `ujobid=`/`jobid=` name a Job record,
/// otherwise SelectJobDbr lists the jobs (of `job=`) and asks
/// "Enter the JobId to select: ". Returns the JobId.
fn get_job_dbr(ua: &mut crate::ua_select::Ua, ast: &CommandAst) -> Option<i64> {
    let ctx = ua.ctx;
    let lookup = |filter: String| ctx.catalog.as_ref()
        .and_then(|cat| cat.query_rows(&format!("SELECT JobId FROM Job WHERE {filter}")).ok())
        .and_then(|t| t.rows.into_iter().next())
        .and_then(|row| row.into_iter().next().flatten())
        .and_then(|id| id.parse::<i64>().ok());
    for arg in &ast.arguments {
        let Some(value) = arg.value.as_deref() else { continue };
        // GetJobRecord's error text (cats/sql_get.cc of the installed 25.1.2).
        let (filter, error) = if arg.key.eq_ignore_ascii_case("ujobid") {
            (format!("Job='{}'", value.replace('\'', "''")), format!("cats/sql_get.cc:275 No Job found for Job name {value}\n"))
        } else if arg.key.eq_ignore_ascii_case("jobid") {
            let id = crate::commands::job::str_to_int64(value);
            (format!("JobId={id}"), format!("cats/sql_get.cc:275 No Job found for JobId {id}\n"))
        } else {
            continue;
        };
        match lookup(filter) {
            Some(id) => return Some(id),
            None => {
                ua.error_msg(format!("Could not find Job \"{value}\": ERR={error}"));
                break;
            }
        }
    }
    // SelectJobDbr: ListJobRecords (for job=<name>) and GetPint.
    let name = ast.arguments.iter()
        .find(|a| (a.key.eq_ignore_ascii_case("jobname") || a.key.eq_ignore_ascii_case("job")) && a.value.is_some())
        .and_then(|a| a.value.clone());
    let list = crate::session::parse_cli_command(&match &name {
        Some(name) => format!("list jobs job=\"{name}\""),
        None => "list jobs".to_string(),
    });
    let listing = crate::commands::list::cmd_list(&list, ctx);
    ua.send_msg(listing.text_output);
    let jobid = i64::from(ua.get_pint("Enter the JobId to select: ")?);
    match lookup(format!("JobId={jobid}")) {
        Some(id) => Some(id),
        None => {
            ua.error_msg(format!("cats/sql_get.cc:275 No Job found for JobId {jobid}\n"));
            None
        }
    }
}

/// The catalog work of a purge (commands/purge.rs); the confirmations
/// ("Purge (yes/no)? ") are asked there once the jobs are counted.
fn purge_execute(ua: &mut crate::ua_select::Ua, ast: &CommandAst, action: PurgeAction) {
    use super::purge;
    let Some(db) = private_db(ua) else { return };
    match action {
        PurgeAction::FilesOfJob(jobid) => purge::purge_files(&db, &jobid.to_string()),
        PurgeAction::FilesOfClient(client) => purge::purge_files_from_client(ua, ast, &db, &client),
        PurgeAction::JobsOfClient(client) => purge::purge_jobs_from_client(ua, ast, &db, &client),
        PurgeAction::JobsOfVolume(volume) => match purge::get_volume_by_name(&db, &volume) {
            Ok(mut volume) => { purge::purge_jobs_from_volume(ua, ast, &db, &mut volume, true); }
            Err(error) => ua.error_msg(error),
        },
        PurgeAction::JobsOfPool(pool) => purge::purge_jobs_from_pool(ua, ast, &db, &pool),
        PurgeAction::QuotaOfClient(client) => purge::purge_quota_from_client(ua, &db, &client),
        PurgeAction::ActionOnPurge(arguments) => {
            let restored = CommandAst {
                arguments: arguments.into_iter().map(|(key, value)| crate::command_ast::Argument { key, value }).collect(),
                ..ast.clone()
            };
            let director = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
                .map(|c| c.director.name.clone()).unwrap_or_default();
            purge::action_on_purge(ua, &restored, &db, &mut purge::NetSdSession::new(director));
        }
    }
}

/// `truncate volstatus=Purged [storage=] [pool=] [volume=] [drive=] [yes]`
/// (ua_cmds.cc:1974 TruncateCmd). Arguments are validated exactly like
/// upstream before the catalog or a Storage Daemon is touched; truncating
/// needs the confirmation (`yes`), compatibility log C-012. Every selected
/// volume is truncated by an SD `relabel` (DoTruncate -> SendLabelRequest),
/// compatibility log C-053.
pub fn cmd_truncate(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let director = ctx.config.as_ref().and_then(|c| c.read().ok())
        .map(|c| c.director.name.clone()).unwrap_or_default();
    // Terminated when dropped at the end of the command (TruncateCmd bail_out).
    let mut sd = super::purge::NetSdSession::new(director);
    truncate_with(ast, ctx, &mut sd, |ua| private_db(ua).map(|db| Box::new(db) as Box<dyn super::bvfs::Db>))
}

/// The Media record fields DoTruncate/SendLabelRequest use (MediaDbRecord).
#[derive(Debug, Clone, Default)]
struct TruncateMedia {
    media_id: i64,
    volume_name: String,
    pool_id: i64,
    storage_id: i64,
    vol_status: String,
    media_type: String,
    slot: i32,
    min_blocksize: u32,
    max_blocksize: u32,
}

/// GetMediaRecord() by MediaId (only the fields truncate needs).
fn truncate_media(db: &dyn super::bvfs::Db, media_id: i64) -> Option<TruncateMedia> {
    let table = db.rows(&format!("SELECT MediaId,VolumeName,PoolId,StorageId,VolStatus,MediaType,Slot,\
        MinBlocksize,MaxBlocksize FROM Media WHERE MediaId={media_id}")).ok()?;
    if table.rows.len() != 1 { return None; }
    let row = &table.rows[0];
    let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let number = |i: usize| crate::commands::job::str_to_int64(&text(i));
    Some(TruncateMedia {
        media_id: number(0), volume_name: text(1), pool_id: number(2), storage_id: number(3),
        vol_status: text(4), media_type: text(5), slot: number(6) as i32,
        min_blocksize: number(7) as u32, max_blocksize: number(8) as u32,
    })
}

/// The Name of GetStorageRecord()/GetPoolRecord() by id (by the empty name
/// when the id is 0); None unless exactly one row.
fn truncate_record_name(db: &dyn super::bvfs::Db, table: &str, id: i64) -> Option<String> {
    let filter = if id != 0 { format!("{table}Id={id}") } else { "Name=''".to_string() };
    let rows = db.rows(&format!("SELECT {table}Id,Name FROM {table} WHERE {filter}")).ok()?.rows;
    if rows.len() != 1 { return None; }
    rows[0].get(1).cloned().flatten()
}

/// `ua->GetStoreResWithName(name)`: Storage ACL, then the configured resource.
fn truncate_store(ctx: &ExecContext, name: &str) -> Option<bareos_config::resources::StorageResource> {
    if !crate::acl::access(ctx, crate::acl::Kind::Storage, name) { return None; }
    ctx.config.as_ref()?.read().ok()?.storages.iter().find(|s| s.name == name).cloned()
}

fn truncate_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    sd: &mut dyn super::purge::SdSession,
    open: impl FnOnce(&mut crate::ua_select::Ua) -> Option<Box<dyn super::bvfs::Db>>,
) -> CommandResult {
    use super::prompt::usage;
    use crate::acl::{self, Kind};
    if ast.arguments.is_empty() {
        return CommandResult::error(usage("truncate", "missing parameter"));
    }
    // FindArgWithValue (ua_select.cc:119): the first argument with this
    // keyword decides; without a value it counts as missing.
    let value = |key: &str| ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .and_then(|a| a.value.as_deref());
    let yes = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes"));
    match value("volstatus") {
        None => return CommandResult::error(usage("truncate", "required parameter 'volstatus' missing")),
        Some(status) if !status.eq_ignore_ascii_case("Purged") => return CommandResult::error(
            usage("truncate", "Invalid parameter. 'volstatus' must be 'Purged'.")),
        Some(_) => {}
    }
    // OpenDb (ua_cmds.cc:2008) precedes the remaining checks.
    let mut ua = crate::ua_select::Ua::new(ctx);
    match crate::ua_db::open_db(ast, ctx) {
        Ok(prefix) => ua.send_msg(prefix),
        Err(result) => {
            ua.send_result(result);
            ua.error_msg("Failed to open db.\n");
            return ua.finish(false);
        }
    }
    let mut parsed = 2; // the command itself and volstatus, as upstream counts
    let mut storage = value("storage").map(str::to_string);
    if storage.is_some() || acl::has_restrictions(ctx, Kind::Storage) {
        if ctx.catalog.is_none() {
            ua.error_msg("Failed to open db.\n");
            return ua.finish(false);
        }
        // SelectStorageDbr (ua_select.cc:823): `storage=` or the
        // "Defined Storages:" / "Select the Storage" dialog.
        match crate::ua_select::select_storage_dbr(&mut ua, ast, "storage") {
            Some(name) => storage = Some(name),
            None => return ua.finish(false),
        }
        parsed += 1;
    }
    let mut pool = value("pool").map(str::to_string);
    if pool.is_some() || acl::has_restrictions(ctx, Kind::Pool) {
        if ctx.catalog.is_none() {
            ua.error_msg("Failed to open db.\n");
            return ua.finish(false);
        }
        // SelectPoolDbr (ua_select.cc:726): `pool=` or the "Defined Pools:"
        // / "Select the Pool" dialog.
        let given = pool.is_some();
        match crate::ua_select::select_pool_dbr(&mut ua, ast, "pool") {
            Some(name) => pool = Some(name),
            None => return ua.finish(false),
        }
        if given { parsed += 1; }
    }
    let volume = value("volume").filter(|name| acl::valid_name(name));
    if volume.is_some() { parsed += 1; }
    let mut drive: u16 = 0;
    if let Some(text) = value("drive") {
        // IsAnInteger(): digits only; atoi() into a drive_number_t.
        if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
            drive = crate::commands::job::str_to_int64(text) as i32 as u16;
            parsed += 1;
        } else {
            ua.error_msg(usage("truncate", &format!("Drive number must be integer but was : {text}\n")));
        }
    }
    if yes { parsed += 1; }
    if parsed != ast.arguments.len() + 1 {
        ua.error_msg(usage("truncate", "Invalid parameter."));
        return ua.finish(false);
    }

    if ctx.catalog.is_none() {
        ua.error_msg("Failed to open db.\n");
        return ua.finish(false);
    }
    let Some(db) = open(&mut ua) else { return ua.finish(false) };
    let db = db.as_ref();
    let quote = |v: &str| v.replace('\'', "''");
    // PrepareMediaSqlQuery (sql_get.cc:1789) with Recycle=1, Enabled=1,
    // VolBytes > 64512 and VolStatus Purged.
    let mut query = String::from("SELECT DISTINCT Media.MediaId,Media.VolumeName,Pool.Name AS Pool,\
        Storage.Name AS Storage,Media.MediaType,Media.LastWritten,Media.VolFiles,Media.VolBytes,\
        Media.VolStatus,Media.ActionOnPurge,Media.Comment FROM Media LEFT JOIN Pool USING(PoolId) \
        LEFT JOIN Storage USING(StorageId) WHERE Media.Recycle=1 AND Media.Enabled=1 ");
    if let Some(s) = &storage {
        query.push_str(&format!("AND Media.StorageId=(SELECT StorageId FROM Storage WHERE Name='{}') ", quote(s)));
    }
    if let Some(p) = &pool {
        query.push_str(&format!("AND Media.PoolId=(SELECT PoolId FROM Pool WHERE Name='{}') ", quote(p)));
    }
    query.push_str("AND Media.VolBytes > 64512 AND Media.VolStatus = 'Purged' ");
    if let Some(v) = volume { query.push_str(&format!("AND Media.VolumeName IN ('{}') ", quote(v))); }
    let table = match db.rows(&query) {
        Ok(table) => table,
        Err(e) => {
            ua.error_msg(format!("{e}\n"));
            return ua.finish(false);
        }
    };
    // ListSqlQuery (HORZ_LIST "volumes") under the filter tuples
    // AddAclFilterTuple(2, Pool_ACL) and (3, Storage_ACL) (ua_cmds.cc:2013):
    // a row is hidden if the console may not access its Pool or Storage, or
    // if both are empty (tri-state FilterData).
    let shown: Vec<bool> = table.rows.iter().map(|row| {
        let (mut show, mut unknown) = (0, 0);
        for (column, kind) in [(2, Kind::Pool), (3, Kind::Storage)] {
            match row.get(column).cloned().flatten().filter(|v| !v.is_empty()) {
                None => unknown += 1,
                Some(name) if acl::access(ctx, kind, &name) => show += 1,
                Some(_) => return false,
            }
        }
        !(unknown > 0 && show == 0)
    }).collect();
    ua.send_msg(crate::sql_list::render_shown(&table, crate::sql_list::ListType::Horz, &shown));
    // GetQueryDbids: every MediaId of the query (the filters only hide rows).
    let media_ids: Vec<i64> = table.rows.iter()
        .filter_map(|row| row.first().cloned().flatten())
        .map(|id| crate::commands::job::str_to_int64(&id)).collect();
    if media_ids.is_empty() {
        return ua.finish(true);
    }
    // VerifyMediaIdsFromSingleStorage (sql_get.cc:1874).
    let mut single = true;
    let mut first_storage = None;
    for &id in &media_ids {
        match truncate_media(db, id) {
            None => { single = false; break; }
            Some(media) => match first_storage {
                None => first_storage = Some(media.storage_id),
                Some(storage_id) if storage_id != media.storage_id => { single = false; break; }
                Some(_) => {}
            },
        }
    }
    if !single {
        ua.error_msg("Selected volumes are from different storages. This is not supported. \
            Please choose only volumes from a single storage.\n");
        return ua.finish(false);
    }
    let Some(first) = truncate_media(db, media_ids[0]) else { return ua.finish(false) };
    // ua->GetStoreResWithId(mr.StorageId)->Protocol: upstream dereferences
    // the resource unchecked (a missing one crashes the Director); here the
    // command ends instead.
    let Some(_store) = truncate_record_name(db, "Storage", first.storage_id)
        .and_then(|name| truncate_store(ctx, &name)) else { return ua.finish(false) };
    // TODO(C-050): Storage Protocol; every storage is native until the
    // configuration carries it (otherwise WarningMsg "Storage uses a
    // non-native protocol. Truncate is only supported for native protocols.\n").
    // GetConfirmation (ua_input.cc:165, ua_cmds.cc:2120): `yes`, otherwise
    // the console is asked; API and batch sessions decline silently.
    if !ua.get_confirmation(ast, "Truncate listed volumes (yes/no)? ", false) {
        return ua.finish(false);
    }
    // Loop over the candidate Volumes and actually truncate them; the SD
    // connection (ua->jcr->store_bsock) is opened once.
    let mut connected = false;
    for &id in &media_ids {
        if let Some(media) = truncate_media(db, id) {
            do_truncate(&mut ua, db, sd, &mut connected, media, drive);
        }
    }
    ua.finish(true)
}

/// ua_cmds.cc:2145 DoTruncate(): relabel the volume to itself.
fn do_truncate(ua: &mut crate::ua_select::Ua, db: &dyn super::bvfs::Db, sd: &mut dyn super::purge::SdSession,
    connected: &mut bool, mut mr: TruncateMedia, drive: u16) -> bool {
    let Some(storage_name) = truncate_record_name(db, "Storage", mr.storage_id) else {
        ua.error_msg(format!("failed to determine storage for id {}\n", mr.storage_id));
        return false;
    };
    let Some(pool_name) = truncate_record_name(db, "Pool", mr.pool_id) else {
        ua.error_msg(format!("failed to determine pool for id {}\n", mr.pool_id));
        return false;
    };
    let Some(store) = truncate_store(ua.ctx, &storage_name) else {
        ua.error_msg(format!("failed to determine storage resource by name {storage_name}\n"));
        return false;
    };
    // SendLabelRequest(media_record_exists = true, relabel = true, slot 0)
    // -> native_send_label_request (ua_label.cc:104), omr == mr.
    let mut vol_files = 0u32;
    let mut vol_bytes = 0u64;
    let mut ok = false;
    // open_sd_bsock() (sd_cmds.cc:159): connect while there is no
    // connection (ua->jcr->store_bsock), otherwise reuse it.
    if !*connected {
        ua.send_msg(format!("Connecting to Storage daemon {} at {}:{} ...\n", store.name, store.address, store.port));
        if sd.connect(&store).is_err() {
            ua.error_msg("Failed to connect to Storage daemon.\n");
        } else {
            *connected = true;
        }
    }
    if *connected {
        let bash = crate::sd_label::bash_spaces;
        let command = format!("relabel {} OldName={} NewName={} PoolName={} MediaType={} Slot={} drive={} \
            MinBlocksize={} MaxBlocksize={}", crate::ua_storage_select::dev_name(&store), bash(&mr.volume_name),
            bash(&mr.volume_name), bash(&pool_name), bash(&mr.media_type), mr.slot as i16, drive as i16,
            mr.min_blocksize as i32, mr.max_blocksize as i32);
        let messages = sd.exchange(&command);
        ua.send_msg(format!("Sending relabel command from \"{0}\" to \"{0}\" ...\n", bash(&mr.volume_name)));
        for message in messages {
            ua.send_msg(message.clone());
            if let Some((files, bytes)) = crate::sd_label::parse_label_ok(&message) {
                if let Some(files) = files { vol_files = files; }
                vol_bytes = bytes;
                ok = true;
            }
        }
    }
    if !ok {
        ua.error_msg(format!("Label command failed for Volume {}.\n", mr.volume_name));
        return false;
    }
    // update_database() (ua_label.cc:57) for an existing record, then
    // UpdateMediaRecord (sql_update.cc:284): LabelDate, VolFiles, VolBytes,
    // VolStatus, InChanger and StorageId change; the other columns are
    // written back as read.
    if vol_bytes > 0 && mr.vol_status == "Unlabeled" {
        mr.vol_status = "Append".into();
    }
    let in_changer = i32::from(mr.slot > 0);
    let quote = |v: &str| v.replace('\'', "''");
    let name = quote(&mr.volume_name);
    let label_date = crate::localtime::strftime("%Y-%m-%d %H:%M:%S", unsafe { libc::time(std::ptr::null_mut()) });
    let updated = db.exec(&format!("UPDATE Media SET LabelDate='{label_date}' WHERE VolumeName='{name}'"))
        .and_then(|_| db.exec(&format!("UPDATE Media SET VolFiles={vol_files},VolBytes={vol_bytes},VolStatus='{}',\
            InChanger={in_changer},StorageId={},EncryptionKey=COALESCE(EncryptionKey,'') WHERE VolumeName='{name}'",
            quote(&mr.vol_status), mr.storage_id)))
        .and_then(|affected| {
            // MakeInchangerUnique()
            if in_changer != 0 && mr.slot != 0 && mr.storage_id != 0 {
                db.exec(&format!("UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={} AND StorageId={} AND MediaId!={}",
                    mr.slot, mr.storage_id, mr.media_id))?;
            }
            Ok(affected > 0)
        });
    match updated {
        Ok(true) => {}
        Ok(false) => return false,
        Err(error) => {
            ua.error_msg(if error.ends_with('\n') { error } else { format!("{error}\n") });
            return false;
        }
    }
    ua.send_msg(format!("The volume '{}' has been truncated.\n", mr.volume_name));
    true
}

/// `reload` — re-read the configuration (C++ ReloadCmd: {"reload": {"success": bool}}).
pub fn cmd_reload(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    match crate::reload::reload_now() {
        Ok(()) => CommandResult::ok("reloaded\n".to_string())
            .with_json(serde_json::json!({ "reload": { "success": true } })),
        Err(e) => CommandResult::error(format!("failed to reload\n{e}\n"))
            .with_json(serde_json::json!({ "reload": { "success": false } })),
    }
}

fn get_arg<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key == key)
        .and_then(|a| a.value.as_deref())
}

fn has_flag(ast: &CommandAst, key: &str) -> bool {
    ast.arguments.iter().any(|a| a.key == key && a.value.is_none())
}

#[cfg(test)]
pub(crate) mod side_effect_tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    pub(crate) fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nStorage { Name = File; Address = 192.0.2.1; Port = 9103 }\n\
             Pool { Name = Full; Pool Type = Backup }\nPool { Name = Incremental; Pool Type = Backup }"
        ).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))),
            session_mgr: None, catalog: None, api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None, job_manager: None }
    }

    /// Catalog double that records every request; job creation and any
    /// write are visible in the log. `job_name` answers the rerun lookup.
    pub(crate) fn recording_catalog(job_name: &'static str)
        -> (crate::catalog_worker::CatalogHandle, Arc<std::sync::Mutex<Vec<String>>>) {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse};
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        let handle = crate::catalog_worker::CatalogHandle::mock(move |request| {
            let (entry, response) = match request {
                CatalogRequest::RawQuery { sql } => (sql, CatalogResponse::Text(format!("name\n----\n{job_name}\n"))),
                CatalogRequest::TextRows { sql } => (sql, CatalogResponse::Error("no rows in the double".into())),
                CatalogRequest::CreateJob { .. } => ("CreateJob".to_string(), CatalogResponse::Error("double".into())),
                _ => ("other request".to_string(), CatalogResponse::Error("double".into())),
            };
            seen.lock().unwrap().push(entry);
            response
        });
        (handle, log)
    }

    const USAGE: &str = "USAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]\n";

    #[test]
    fn truncate_rejects_like_upstream_without_touching_the_sd() {
        let ctx = context();
        for (line, expected) in [
            ("truncate", format!("missing parameter\n{USAGE}")),
            ("truncate yes", format!("required parameter 'volstatus' missing\n{USAGE}")),
            ("truncate volstatus", format!("required parameter 'volstatus' missing\n{USAGE}")),
            ("truncate volstatus=Full", format!("Invalid parameter. 'volstatus' must be 'Purged'.\n{USAGE}")),
            ("truncate volstatus=purged bogus", format!("Invalid parameter.\n{USAGE}")),
            ("truncate volstatus=Purged volume=bad*name", format!("Invalid parameter.\n{USAGE}")),
            ("truncate volstatus=Purged drive=x", format!("Drive number must be integer but was : x\n\n{USAGE}Invalid parameter.\n{USAGE}")),
            ("truncate volstatus=Purged storage=File yes", "Failed to open db.\n".to_string()),
        ] {
            let result = truncate_untouched(&crate::session::parse_cli_command(line), &ctx);
            assert!(!result.success, "{line}");
            assert_eq!(result.error_message, expected, "{line}");
        }
        // The storage selection needs the catalog (OpenDb).
        let result = truncate_untouched(&crate::session::parse_cli_command("truncate volstatus=Purged storage=Nope"), &ctx);
        assert_eq!((result.success, result.error_message.as_str()), (false, "Failed to open db.\n"));
    }

    /// PurgeCmd volume branch (ua_purge.cc:156): cmd_holder gets
    /// " volume=<name>" once ua->cmd holds an answer, then ActionOnPurgeCmd
    /// runs on the restored arguments (C-049).
    #[test]
    fn purge_volume_action_restores_the_command_line() {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
        use crate::ua_select::tests::scripted;
        let mut ctx = context();
        ctx.catalog = Some(crate::catalog_worker::CatalogHandle::mock(|request| {
            let rows: Vec<Vec<&str>> = match &request {
                CatalogRequest::TextRows { sql } if sql == "SELECT Name FROM Pool" => vec![vec!["Full"]],
                CatalogRequest::TextRows { sql } if sql.starts_with("SELECT PoolId FROM Pool") => vec![vec!["1"]],
                CatalogRequest::TextRows { .. } => vec![],
                _ => return CatalogResponse::Error("double".into()),
            };
            CatalogResponse::Rows(SqlTable {
                columns: vec![SqlColumn { name: "c".into(), numeric: false }],
                rows: rows.into_iter().map(|r| r.into_iter().map(|v| Some(v.to_string())).collect()).collect(),
            })
        }));
        let purge = |line: &str, answers: &[&str]| {
            let mut acted = Vec::new();
            scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                purge_dialog(&mut ua, &crate::session::parse_cli_command(line), |_, action| acted.push(action));
                ua.finish(true);
            });
            acted
        };
        let args = |list: &[(&str, Option<&str>)]| PurgeAction::ActionOnPurge(
            list.iter().map(|(k, v)| (k.to_string(), v.map(str::to_string))).collect());
        // Named volumes (unknown here): no prompt, the line stays as typed.
        assert_eq!(purge("purge volume=v1 volume=v2 action=truncate", &[]),
            vec![args(&[("volume", Some("v1")), ("volume", Some("v2")), ("action", Some("truncate"))])]);
        // Selected interactively: the typed name is appended.
        assert_eq!(purge("purge volume action=truncate storage=File", &["v9"]),
            vec![args(&[("volume", None), ("action", Some("truncate")), ("storage", Some("File")), ("volume", Some("v9"))])]);
        // A cancelled selection appends an empty name.
        assert_eq!(purge("purge volume action=truncate", &["."]),
            vec![args(&[("volume", None), ("action", Some("truncate")), ("volume", Some(""))])]);
        // Without action= nothing follows the volume purges.
        assert!(purge("purge volume=v1", &[]).is_empty());
    }

    #[test]
    fn prune_and_purge_need_target_and_confirmation() {
        use crate::ua_select::tests::scripted;
        let ctx = context();
        ctx.config.as_ref().unwrap().write().unwrap().clients.push(bareos_config::resources::ClientResource {
            name: "fd1".into(), ..Default::default() });
        let ast = |line: &str| crate::session::parse_cli_command(line);
        // Scripted prune: the transcript and the action (if any).
        let prune = |line: &str, answers: &[&str]| {
            let mut acted = None;
            let (_, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                prune_dialog(&mut ua, &ast(line), |_, action| { acted = Some(action); true });
                ua.finish(true);
            });
            (console.transcript(), acted)
        };
        let menu = "You have the following choices:\n1: Files\n2: Jobs\n3: Volume\n4: Stats\n5: Directory\n";
        assert_eq!(prune("prune", &["."]), (format!("{menu}Choose item to prune (1-5): .\nSelection aborted, nothing done.\n"), None));
        assert_eq!(prune("prune jobs", &["."]), ("Automatically selected Client: fd1\nThe current Job retention period is: 6 months \n\
            Continue? (yes/mod/no): .\n".to_string(), None));
        assert_eq!(prune("prune files client=fd1", &["no"]).1, None);
        assert_eq!(prune("prune", &["2", "mod", "3 days", "yes"]).1,
            Some(PruneAction::Jobs { client: "fd1".into(), pool: None, retention: 3 * 86_400 }));
        assert_eq!(prune("prune stats", &["."]).0,
            "The current Statistics retention period is: 5 years 1 month 5 days \nContinue? (yes/mod/no): .\n");
        assert_eq!(prune("prune directory", &["no", "."]).0,
            "No client restriction given really remove directory for all clients (yes/no): no\n\
             Automatically selected Client: fd1\nPlease enter the full path to remove: .\n");
        // Without console: the menu cannot be answered, nothing happens.
        let mut ua = crate::ua_select::Ua::new(&ctx);
        prune_dialog(&mut ua, &ast("prune jobs"), |_, _| panic!("no confirmation without console"));
        // `yes` skips ConfirmRetention.
        let mut ua = crate::ua_select::Ua::new(&ctx);
        let mut acted = None;
        prune_dialog(&mut ua, &ast("prune jobs client=fd1 yes"), |_, action| { acted = Some(action); true });
        assert_eq!(acted, Some(PruneAction::Jobs { client: "fd1".into(), pool: None, retention: 180 * 86_400 }));

        // Purge: the warning, then the keyword menu for incomplete requests.
        let purge = |line: &str, answers: &[&str]| {
            let mut acted = Vec::new();
            let (_, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                purge_dialog(&mut ua, &ast(line), |_, action| acted.push(action));
                ua.finish(true);
            });
            (console.transcript(), acted)
        };
        let menu = "You have the following choices:\n1: files\n2: jobs\n3: volume\n4: quota\nChoose item to purge (1-4): .\n\
            Selection aborted, nothing done.\n";
        for line in ["purge", "purge jobs", "purge files", "purge quota", "purge bogus"] {
            let (text, acted) = purge(line, &["."]);
            assert!(text.starts_with("\nThis command can be DANGEROUS!!!\n"), "{line}");
            assert!(text.ends_with(menu), "{line}: {text}");
            assert!(acted.is_empty(), "{line}");
        }
        assert_eq!(purge("purge jobs client=fd1", &[]).1, vec![PurgeAction::JobsOfClient("fd1".into())]);
        assert_eq!(purge("purge", &["2"]).1, vec![PurgeAction::JobsOfClient("fd1".into())]);

        // The handlers send nothing to the catalog on any cancelled path.
        let mut ctx = ctx;
        let (catalog, log) = recording_catalog("b1");
        ctx.catalog = Some(catalog);
        for line in ["prune", "prune jobs", "prune files", "prune volume", "prune volumes pool=Full", "prune stats"] {
            scripted(&["."], || cmd_prune(&ast(line), &ctx));
        }
        for line in ["purge", "purge jobs", "purge files", "purge files jobid=0", "purge volume", "purge jobs pool=Full"] {
            scripted(&["."], || cmd_purge(&ast(line), &ctx));
        }
        for line in ["truncate", "truncate volstatus=Purged bogus=1"] {
            truncate_untouched(&ast(line), &ctx);
        }
        let writes = |log: &Vec<String>| log.iter().filter(|q| !q.trim_start().to_ascii_uppercase().starts_with("SELECT")).count();
        assert_eq!(writes(&log.lock().unwrap()), 0, "{:?}", log.lock().unwrap());
        scripted(&[], || cmd_purge(&ast("purge jobs client=fd1"), &ctx));
        assert!(log.lock().unwrap().iter().any(|q| q == "other request"),
            "control: a complete purge asks for the private catalog connection");
    }

    #[test]
    fn update_dialog_changes_nothing_until_answered() {
        use crate::ua_select::tests::scripted;
        let ctx = context();
        let update = |line: &str, answers: &[&str]| {
            let mut acted = Vec::new();
            let (_, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                update_dialog(&mut ua, &crate::session::parse_cli_command(line), |_, action| acted.push(action));
                ua.finish(true);
            });
            (console.transcript(), acted)
        };
        let (text, acted) = update("update", &["."]);
        assert_eq!(text, "Update choice:\n1: Volume parameters\n2: Pool from resource\n3: Slots from autochanger\n\
            4: Long term statistics\nChoose catalog item to update (1-4): .\nSelection aborted, nothing done.\n");
        assert!(acted.is_empty());
        let (text, acted) = update("update volume", &["19"]);
        assert!(text.ends_with("Select parameter to modify (1-19): 19\nSelection terminated.\n"), "{text}");
        assert!(acted.is_empty());
        // Cancelled parameter menu: SelectMediaDbr still runs (no catalog: no pools).
        assert!(update("update volume", &["."]).1.is_empty());
        let (text, acted) = update("update pool", &["."]);
        assert!(text.ends_with("Select Pool resource (1-2): .\nSelection aborted, nothing done.\n"), "{text}");
        assert!(acted.is_empty());
        // The long term statistics are never computed here.
        assert_eq!(update("update stats", &[]).1, vec![UpdateAction::Unsupported("update stats")]);
        assert!(update("update jobid", &[]).1.is_empty());
        let mut media = crate::ua_select::MediaRecord { media_id: 7, ..Default::default() };
        media.volume_name = "v".into();
        let mut ua = crate::ua_select::Ua::new(&ctx);
        let mut acted = Vec::new();
        update_field(&mut ua, &crate::session::parse_cli_command("update volume"), &mut |_, action| acted.push(action), 0, "full", &media);
        update_field(&mut ua, &crate::session::parse_cli_command("update volume"), &mut |_, action| acted.push(action), 0, "bogus", &media);
        update_field(&mut ua, &crate::session::parse_cli_command("update volume"), &mut |_, action| acted.push(action), 1, "3 days", &media);
        assert_eq!(acted, vec![
            UpdateAction::Media { media_id: 7, column: "VolStatus", value: "'Full'".into(), message: "New Volume status is: Full\n".into() },
            UpdateAction::Media { media_id: 7, column: "VolRetention", value: "259200".into(), message: "New retention period is: 3 days \n".into() },
        ]);
        assert_eq!(ua.finish(true).error_message, "Invalid VolStatus specified: bogus\n");
    }

    /// Catalog double for `update volume`: volumes v1 (MediaId 7, Error, no
    /// key) and v2 (MediaId 8, Append, with key), pools Full (MaxVols 100)
    /// and Inc. Prepare fails like PostgreSQL for "abc"; every write and
    /// every request is logged.
    fn update_catalog() -> (crate::catalog_worker::CatalogHandle, Arc<std::sync::Mutex<Vec<String>>>) {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        let table = |rows: &[&[&str]]| SqlTable {
            columns: (0..rows.first().map_or(1, |r| r.len())).map(|i| SqlColumn { name: format!("c{i}"), numeric: false }).collect(),
            rows: rows.iter().map(|r| r.iter().map(|v| Some(v.to_string())).collect()).collect() };
        let handle = crate::catalog_worker::CatalogHandle::mock(move |request| {
            let (sql, prepare) = match request {
                CatalogRequest::TextRows { sql } | CatalogRequest::RawQuery { sql } => (sql, false),
                CatalogRequest::Prepare { sql } => (sql, true),
                _ => ("other request".to_string(), false),
            };
            seen.lock().unwrap().push(if prepare { format!("PREPARE {sql}") } else { sql.clone() });
            if prepare {
                return if sql.contains("=abc ") {
                    CatalogResponse::Error("ERROR:  column \"abc\" does not exist\nLINE 1: x\n        ^\n".into())
                } else {
                    CatalogResponse::Ok(0)
                };
            }
            let media = |id: &str, name: &str, status: &str| [id, name, "3", status, "0", "1", "0", "0", "0", "0", "1",
                "0", "0", "0", "0", "0", "0"].map(str::to_string);
            CatalogResponse::Rows(if sql.starts_with("SELECT MediaId, VolumeName") {
                let row = if sql.contains("'v1'") { Some(media("7", "v1", "Error")) }
                    else if sql.contains("'v2'") { Some(media("8", "v2", "Append")) } else { None };
                SqlTable { columns: vec![SqlColumn { name: "c".into(), numeric: false }; 17],
                    rows: row.into_iter().map(|r| r.into_iter().map(Some).collect()).collect() }
            } else if sql.starts_with("SELECT EncryptionKey") {
                table(&[&[if sql.ends_with("=8") { "key" } else { "" }]])
            } else if sql.starts_with("SELECT MaxVols") {
                table(&[&["100"]])
            } else if sql.starts_with("SELECT VolBytes FROM Media") {
                if sql.contains("'v1'") { table(&[&["123"]]) } else { table(&[]) }
            } else if sql.starts_with("SELECT Name FROM Pool WHERE PoolId=") {
                table(&[&["Full"]])
            } else if sql == "SELECT Name FROM Pool" {
                table(&[&["Full"], &["Inc"]])
            } else if sql.starts_with("SELECT PoolId FROM Pool") {
                if sql.contains("'Full'") || sql.contains("'Inc'") { table(&[&["3"]]) } else { table(&[]) }
            } else {
                table(&[])
            })
        });
        (handle, log)
    }

    /// Every validation message of UpdateVolume and its Update<Field>
    /// helpers (ua_update.cc) on the catalog double: nothing is written.
    #[test]
    fn update_volume_validation_messages() {
        use crate::ua_select::tests::scripted;
        let mut ctx = context();
        let (catalog, log) = update_catalog();
        ctx.catalog = Some(catalog);
        let run = |line: &str, answers: &[&str]| {
            let mut acted = Vec::new();
            let (result, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                update_dialog(&mut ua, &crate::session::parse_cli_command(line), |_, action| acted.push(action));
                ua.finish(true)
            });
            // Messages not yet flushed to the console come back in the result.
            let rest: String = result.messages.iter().map(|(_, text)| text.as_str()).collect();
            (console.transcript() + &rest, acted)
        };
        let error = |line: &str, expected: &str| {
            let (text, acted) = run(line, &[]);
            assert_eq!(text, expected, "{line}");
            assert!(acted.is_empty(), "{line}: {acted:?}");
        };
        error("update volume=v1 volstatus=Bogus", "Invalid VolStatus specified: Bogus\n");
        // SelectMediaDbr comes first: a missing volume ends before any value check.
        error("update volume=nosuch volstatus=Bogus maxvoljobs=abc",
            "cats/sql_get.cc:1177 Media record for Volume \"nosuch\" not found.\n");
        error("update volume=v1 volretention=bogus", "Invalid retention period specified: bogus\n");
        error("update volume=v1 voluse=bogus", "Invalid use duration specified: bogus\n");
        error("update volume=v1 maxvolbytes=xyz", "Invalid max. bytes specification: xyz\n");
        error("update volume=v1 recycle=maybe", "Invalid value. It must be yes or no.\n");
        error("update volume=v1 inchanger=maybe", "Invalid value. It must be yes or no.\n");
        error("update volume=v1 slot=-1", "Invalid slot, it must be between 0 and MaxVols=100\n");
        error("update volume=v1 slot=101", "Invalid slot, it must be between 0 and MaxVols=100\n");
        error("update volume=v1 enabled=7", "Invalid Enabled value, it must be yes, no, archived, 0, 1, or 2\n");
        error("update volume=v1 enabled=maybe", "Invalid Enabled value, it must be yes, no, archived, 0, 1, or 2\n");
        // MaxVolJobs/MaxVolFiles: the C++ statement's catalog error, never run.
        error("update volume=v1 maxvoljobs=abc", "Query failed: UPDATE Media SET MaxVolJobs=abc WHERE MediaId=7: \
            ERR=ERROR:  column \"abc\" does not exist\nLINE 1: x\n        ^\n\n");
        // Statements that would parse (or carry several commands) are refused (C-012).
        error("update volume=v1 maxvolfiles=1;DROP", "Invalid maxvolfiles value: 1;DROP\n");
        error("update volume=v1 maxvoljobs=1+1", "Invalid maxvoljobs value: 1+1\n");
        error("update volume=v1 maxvoljobs=$1", "Invalid maxvoljobs value: $1\n");
        error("update volume=v1 volstatus=Bogus maxvolbytes=xyz",
            "Invalid VolStatus specified: Bogus\nInvalid max. bytes specification: xyz\n");
        // Encrypt (UpdateVolEncryption).
        error("update volume=v1 encrypt=bogus", "Valid values for Encrypt are yes/no/rotate\n");
        error("update volume=v1 encrypt=true", "Valid values for Encrypt are yes/no/rotate\n");
        error("update volume=v1 encrypt=no", "No encryption key for volume 'v1' configured.\n");
        error("update volume=v1 encrypt=ROTATE", "No encryption key for volume 'v1' configured.\n");
        error("update volume=v2 encrypt=yes", "Encryption key for volume 'v2' already configured.\n");
        error("update volume=v2 encrypt=no", "Removing encryption key from volume 'v2' requires volstatus=Purged\n");
        error("update volume=v2 encrypt=rotate", "Rotating encryption key for volume 'v2' requires volstatus=Purged\n");
        // GetPoolDbr: unknown pools lead to the pool selection.
        let (text, acted) = run("update volume=v1 pool=nosuch", &["."]);
        assert_eq!(text, "Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\n\
            Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\n\
            Defined Pools:\n1: Full\n2: Inc\nSelect the Pool (1-2): .\nSelection aborted, nothing done.\n");
        assert!(acted.is_empty());
        let (text, acted) = run("update volume=v1 recyclepool=nosuch", &["."]);
        assert_eq!(text, "Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\n\
            Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\n\
            Defined Pools:\n1: *None*\n2: Full\n3: Inc\nSelect the Pool (1-3): .\nSelection aborted, nothing done.\n");
        assert!(acted.is_empty());
        let (text, acted) = run("update volume allfrompool=nosuch", &["."]);
        assert_eq!(text, "Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\n\
            Defined Pools:\n1: Full\n2: Inc\nSelect the Pool (1-2): .\nSelection aborted, nothing done.\n");
        assert!(acted.is_empty());
        // Dialog values go through the same checks.
        let (text, acted) = run("update volume=v1", &["2", "bogus", "."]);
        assert!(text.contains("Enter Volume Retention period: bogus\nInvalid retention period specified: bogus\n"), "{text}");
        assert!(acted.is_empty());
        let (text, acted) = run("update volume=v1", &["11", "nosuch", "."]);
        assert!(text.contains("Current Pool is: Full\nEnter new Pool name: nosuch\n\
            Could not find Pool \"nosuch\": ERR=Pool record not found in Catalog.\nDefined Pools:\n"), "{text}");
        assert!(acted.is_empty());
        let (text, acted) = run("update volume=v1", &["15", "7", "."]);
        assert!(text.contains("Enter new Enabled: 7\nInvalid Enabled value, it must be yes, no, archived, 0, 1, or 2\n"), "{text}");
        assert!(acted.is_empty());
        let writes: Vec<_> = log.lock().unwrap().iter().filter(|q| !q.starts_with("SELECT") && !q.starts_with("PREPARE")).cloned().collect();
        assert!(writes.is_empty(), "{writes:?}");
        // A numeric constant is written verbatim, like the C++ statement.
        assert_eq!(run("update volume=v1 maxvoljobs=-5", &[]).1, vec![UpdateAction::Media { media_id: 7,
            column: "MaxVolJobs", value: "-5".into(), message: "New max jobs is: -5\n".into() }]);
        assert_eq!(run("update volume=v1 maxvolfiles=1.5e1", &[]).1, vec![UpdateAction::Media { media_id: 7,
            column: "MaxVolFiles", value: "1.5e1".into(), message: "New max files is: 1.5e1\n".into() }]);
    }

    /// label/relabel validation of do_label (ua_label.cc:393, C-012
    /// 8943332) on the catalog double: nothing reaches the SD or the catalog.
    #[test]
    fn label_relabel_validation_messages() {
        use crate::ua_select::tests::scripted;
        let mut ctx = context();
        let (catalog, log) = update_catalog();
        ctx.catalog = Some(catalog);
        let run = |line: &str, relabel: bool, answers: &[&str]| {
            let mut acted = None;
            let (result, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                label_dialog(&mut ua, &crate::session::parse_cli_command(line), relabel, |_, request| acted = Some(request));
                ua.finish(true)
            });
            let rest: String = result.messages.iter().map(|(_, text)| text.as_str()).collect();
            assert_eq!(acted, None, "{line}");
            console.transcript() + &rest
        };
        // The old volume must be Purged or Recycled.
        assert_eq!(run("relabel storage=File oldvolume=v1 volume=new", true, &[]),
            "Volume \"v1\" has VolStatus Error. It must be Purged or Recycled before relabeling.\n");
        assert_eq!(run("relabel storage=File oldvolume=v2 volume=new", true, &[]),
            "Volume \"v2\" has VolStatus Append. It must be Purged or Recycled before relabeling.\n");
        // An unknown old volume: the catalog error, then SelectMediaDbr
        // (which takes volume=, here unknown as well, or offers the pools).
        assert_eq!(run("relabel storage=File oldvolume=nosuch volume=new", true, &[]),
            "cats/sql_get.cc:1177 Media record for Volume \"nosuch\" not found.\n\
            cats/sql_get.cc:1177 Media record for Volume \"new\" not found.\n");
        assert_eq!(run("relabel storage=File oldvolume=nosuch", true, &["."]),
            "cats/sql_get.cc:1177 Media record for Volume \"nosuch\" not found.\n\
            Defined Pools:\n1: Full\n2: Inc\nSelect the Pool (1-2): .\nSelection aborted, nothing done.\n\
            cats/sql_get.cc:1177 Media record for Volume \"nosuch\" not found.\n");
        // A labeled (VolBytes > 0) volume name is refused, then asked again.
        assert_eq!(run("label storage=File volume=v1", false, &["."]),
            "Media record for new Volume \"v1\" already exists.\nEnter new Volume name: .\n");
        assert_eq!(run("label storage=File volume=/ab", false, &["."]),
            "Volume name can not start with \"/\".\nEnter new Volume name: .\n");
        assert_eq!(run("label storage=File volume=a!b", false, &["."]),
            "Illegal character \"!\" in a volume name.\nEnter new Volume name: .\n");
        // No default pool: a legal new name leads to the pool selection.
        assert_eq!(run("label storage=File volume=fresh", false, &["."]),
            "Defined Pools:\n1: Full\n2: Inc\nSelect the Pool (1-2): .\nSelection aborted, nothing done.\n");
        let writes: Vec<_> = log.lock().unwrap().iter().filter(|q| !q.starts_with("SELECT")).cloned().collect();
        assert!(writes.is_empty(), "{writes:?}");
    }

    #[test]
    fn numeric_constants_are_side_effect_free_sql() {
        for value in ["0", "42", "-1", "+7", "1.5", ".5", "5.", "1e3", "2E-2"] {
            assert!(is_numeric_constant(value), "{value}");
        }
        for value in ["", "-", ".", "e3", "1e", "1e+", "abc", "12x", "1;DROP", "1 --", "1+1", "0x10", "(1)", " 1"] {
            assert!(!is_numeric_constant(value), "{value}");
        }
    }

    /// update_execute reports a failed statement like SqlQuery.
    #[test]
    fn update_execute_reports_query_failed() {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse};
        let mut ctx = context();
        ctx.catalog = Some(crate::catalog_worker::CatalogHandle::mock(|request| match request {
            CatalogRequest::TextRows { .. } => CatalogResponse::Error("ERROR:  integer out of range\n".into()),
            _ => CatalogResponse::Error("unexpected".into()),
        }));
        let mut ua = crate::ua_select::Ua::new(&ctx);
        update_execute(&mut ua, UpdateAction::Media { media_id: 7, column: "MaxVolJobs", value: "99999999999".into(),
            message: "New max jobs is: 99999999999\n".into() });
        assert_eq!(ua.finish(true).error_message,
            "Query failed: UPDATE Media SET MaxVolJobs=99999999999 WHERE MediaId=7: ERR=ERROR:  integer out of range\n\n");
    }

    #[test]
    fn label_selects_instead_of_defaulting() {
        use crate::ua_select::tests::scripted;
        let ctx = context();
        let label = |line: &str, relabel: bool, answers: &[&str]| {
            let mut acted = None;
            let (_, console) = scripted(answers, || {
                let mut ua = crate::ua_select::Ua::new(&ctx);
                label_dialog(&mut ua, &crate::session::parse_cli_command(line), relabel, |_, request| acted = Some(request));
                ua.finish(true);
            });
            (console.transcript(), acted)
        };
        assert_eq!(label("label", false, &["."]),
            ("Automatically selected Storage: File\nEnter new Volume name: .\n".to_string(), None));
        assert_eq!(label("label", false, &["/x", "a b", "."]).0, "Automatically selected Storage: File\n\
            Enter new Volume name: /x\nVolume name can not start with \"/\".\nEnter new Volume name: a b\n\
            Illegal character \" \" in a volume name.\nEnter new Volume name: .\n");
        // No catalog: no pool list, nothing labeled.
        assert_eq!(label("label volume=v1", false, &["."]).1, None);
        assert_eq!(label("label storage=Nope volume=v1", false, &["."]).1, None);
        // barcodes: only autochangers qualify (none here).
        assert_eq!(label("label barcodes", false, &[]).1, None);
        // relabel needs the old volume first.
        assert_eq!(label("relabel", true, &["."]).1, None);
        let mut ua = crate::ua_select::Ua::new(&ctx);
        label_dialog(&mut ua, &crate::session::parse_cli_command("label volume=v1"), false, |_, _| panic!("batch"));
    }

    /// Catalog with the storages File and Tape (no StorageId records) and
    /// one purged volume v1; every query is logged.
    fn truncate_catalog() -> (crate::catalog_worker::CatalogHandle, Arc<std::sync::Mutex<Vec<String>>>) {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        let table = |rows: &[&str]| SqlTable { columns: vec![SqlColumn { name: "name".into(), numeric: false }],
            rows: rows.iter().map(|r| vec![Some(r.to_string())]).collect() };
        let handle = crate::catalog_worker::CatalogHandle::mock(move |request| {
            let sql = match request {
                CatalogRequest::TextRows { sql } | CatalogRequest::RawQuery { sql } => sql,
                _ => "other request".to_string(),
            };
            seen.lock().unwrap().push(sql.clone());
            CatalogResponse::Rows(if sql == "SELECT Name FROM Storage" {
                table(&["File", "Tape"])
            } else if sql.starts_with("SELECT StorageId FROM Storage") {
                table(&if sql.contains("'File'") { vec!["1"] } else { vec![] })
            } else if sql.starts_with("SELECT DISTINCT Media.MediaId,Media.VolumeName") {
                SqlTable { columns: vec![SqlColumn { name: "mediaid".into(), numeric: true },
                    SqlColumn { name: "volumename".into(), numeric: false }],
                    rows: vec![vec![Some("7".into()), Some("v1".into())]] }
            } else {
                table(&[])
            })
        });
        (handle, log)
    }

    /// Storage daemon double for truncate: refuses the first `refuse`
    /// connections, records connects and commands, answers from `replies`.
    #[derive(Default)]
    struct TruncSd {
        refuse: usize,
        connects: Vec<String>,
        sent: Vec<String>,
        replies: std::collections::VecDeque<Vec<&'static str>>,
    }

    impl super::super::purge::SdSession for TruncSd {
        fn connect(&mut self, store: &bareos_config::resources::StorageResource) -> Result<(), String> {
            self.connects.push(store.name.clone());
            if self.refuse > 0 {
                self.refuse -= 1;
                return Err("refused".into());
            }
            Ok(())
        }
        fn exchange(&mut self, command: &str) -> Vec<String> {
            self.sent.push(command.to_string());
            self.replies.pop_front().unwrap_or_default().into_iter().map(String::from).collect()
        }
    }

    /// An SD that must never be contacted.
    struct PanicSd;

    impl super::super::purge::SdSession for PanicSd {
        fn connect(&mut self, store: &bareos_config::resources::StorageResource) -> Result<(), String> {
            panic!("must not contact the SD: {}", store.name)
        }
        fn exchange(&mut self, command: &str) -> Vec<String> {
            panic!("must not contact the SD: {command}")
        }
    }

    /// truncate where neither the catalog work nor the SD may be reached.
    fn truncate_untouched(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
        truncate_with(ast, ctx, &mut PanicSd, |_| panic!("must not reach the catalog work"))
    }

    /// Catalog double of the truncate work: `answer` gives the rows of a
    /// query; every statement is logged; UPDATEs affect one row.
    struct TruncDb {
        log: Arc<std::sync::Mutex<Vec<String>>>,
        answer: fn(&str) -> Vec<Vec<Option<&'static str>>>,
    }

    impl super::super::bvfs::Db for TruncDb {
        fn rows(&self, sql: &str) -> Result<crate::catalog_worker::SqlTable, String> {
            use crate::catalog_worker::{SqlColumn, SqlTable};
            self.log.lock().unwrap().push(sql.to_string());
            let rows = (self.answer)(sql);
            let names: Vec<String> = if sql.starts_with("SELECT DISTINCT Media.MediaId") {
                ["mediaid", "volumename", "pool", "storage", "mediatype", "lastwritten", "volfiles", "volbytes",
                 "volstatus", "actiononpurge", "comment"].iter().map(|s| s.to_string()).collect()
            } else {
                (0..rows.first().map_or(1, Vec::len)).map(|i| format!("c{i}")).collect()
            };
            Ok(SqlTable {
                columns: names.into_iter().map(|name| SqlColumn { numeric: name == "mediaid", name }).collect(),
                rows: rows.into_iter().map(|r| r.into_iter().map(|v| v.map(String::from)).collect()).collect(),
            })
        }
        fn exec(&self, sql: &str) -> Result<u64, String> {
            self.log.lock().unwrap().push(sql.to_string());
            Ok(1)
        }
    }

    /// Volumes 7 (v1) and 8 (v2, slot 3), both Purged on Storage 1 (File)
    /// in Pool 1 (Full); the listing row of v2 has neither Pool nor Storage.
    fn truncate_rows(sql: &str) -> Vec<Vec<Option<&'static str>>> {
        let row = |v: &[Option<&'static str>]| v.to_vec();
        if sql.starts_with("SELECT DISTINCT Media.MediaId") {
            vec![row(&[Some("7"), Some("v1"), Some("Full"), Some("File"), Some("File"), None, Some("3"),
                        Some("1000000"), Some("Purged"), Some("1"), Some("")]),
                 row(&[Some("8"), Some("v2"), None, None, Some("File"), None, Some("1"),
                        Some("2000000"), Some("Purged"), Some("1"), Some("")])]
        } else if sql.ends_with("FROM Media WHERE MediaId=7") {
            vec![row(&[Some("7"), Some("v1"), Some("1"), Some("1"), Some("Purged"), Some("File"), Some("0"), Some("0"), Some("0")])]
        } else if sql.ends_with("FROM Media WHERE MediaId=8") {
            vec![row(&[Some("8"), Some("v2"), Some("1"), Some("1"), Some("Purged"), Some("File"), Some("3"), Some("0"), Some("0")])]
        } else if sql == "SELECT StorageId,Name FROM Storage WHERE StorageId=1" {
            vec![row(&[Some("1"), Some("File")])]
        } else if sql == "SELECT PoolId,Name FROM Pool WHERE PoolId=1" {
            vec![row(&[Some("1"), Some("Full")])]
        } else {
            vec![]
        }
    }

    /// Context with the storage File (Device FileStorage) and the catalog
    /// double of the selections.
    fn truncate_context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nStorage { Name = File; Address = 192.0.2.1; Port = 9103; Device = FileStorage; \
             Media Type = File }\nPool { Name = Full; Pool Type = Backup }").unwrap();
        let (catalog, _) = truncate_catalog();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))), session_mgr: None,
            catalog: Some(catalog), api_mode: crate::command_executor::ApiMode::Human, bus_rx: None, job_manager: None }
    }

    /// One truncate with console answers: (success, transcript, catalog writes).
    fn run_truncate(line: &str, answers: &[&str], sd: &mut TruncSd, answer: fn(&str) -> Vec<Vec<Option<&'static str>>>)
        -> (bool, String, Vec<String>) {
        let ctx = truncate_context();
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let db = TruncDb { log: log.clone(), answer };
        let (result, console) = crate::ua_select::tests::scripted(answers, || truncate_with(
            &crate::session::parse_cli_command(line), &ctx, sd, move |_| Some(Box::new(db))));
        let writes = log.lock().unwrap().iter().filter(|q| !q.starts_with("SELECT")).cloned().collect();
        // Without a dialog the output comes back in the result.
        let output = if result.errors_inline {
            result.text_output.clone()
        } else if result.errors_first {
            format!("{}{}", result.error_message, result.text_output)
        } else {
            format!("{}{}", result.text_output, result.error_message)
        };
        (result.success, format!("{}{output}", console.transcript()), writes)
    }

    #[test]
    fn truncate_dialogs_change_nothing_when_cancelled() {
        use crate::ua_select::tests::scripted;
        let mut ctx = context();
        let (catalog, _) = truncate_catalog();
        ctx.catalog = Some(catalog);
        let ast = |line: &str| crate::session::parse_cli_command(line);
        // Unknown storage: SelectStorageDbr offers the catalog storages.
        let (result, console) = scripted(&["."], || truncate_untouched(&ast("truncate volstatus=Purged storage=Nope"), &ctx));
        assert!(!result.success);
        assert_eq!(console.transcript(), "Could not find Storage \"Nope\": ERR=Defined Storages:\n1: File\n2: Tape\n\
            Select the Storage (1-2): .\nSelection aborted, nothing done.\n");
        // The confirmation: "no" and a cancel decline.
        for answer in ["no", "."] {
            let (ok, transcript, writes) = run_truncate("truncate volstatus=Purged storage=File", &[answer],
                &mut TruncSd::default(), truncate_rows);
            assert!(!ok);
            assert!(transcript.ends_with(&format!("Truncate listed volumes (yes/no)? {answer}\n")), "{transcript}");
            assert!(writes.is_empty(), "{writes:?}");
        }
        // Without console (and in API mode) nothing is asked and nothing done.
        let ctx = truncate_context();
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let db = TruncDb { log: log.clone(), answer: truncate_rows };
        let result = truncate_with(&ast("truncate volstatus=Purged storage=File"), &ctx, &mut PanicSd,
            move |_| Some(Box::new(db)));
        assert!(!result.success);
        assert!(log.lock().unwrap().iter().all(|q| q.starts_with("SELECT")));
    }

    /// DoTruncate (ua_cmds.cc:2145): one relabel per volume over one SD
    /// connection; the 18be77e6 reply `3000 OK label. VolFiles=.. VolBytes=..`
    /// counts as success and the catalog gets the new size (C-053).
    #[test]
    fn truncate_relabels_every_volume_and_updates_the_catalog() {
        let mut sd = TruncSd::default();
        sd.replies.push_back(vec!["3000 OK label. VolFiles=0 VolBytes=225 Volume=\"v1\" Device=\"FileStorage\" (/s)\n"]);
        sd.replies.push_back(vec!["3000 OK label. VolBytes=226 Volume=\"v2\" Device=x\n"]);
        let (ok, transcript, writes) = run_truncate("truncate volstatus=Purged storage=File drive=2", &["yes"], &mut sd, truncate_rows);
        assert!(ok);
        assert_eq!(sd.connects, vec!["File"]);
        assert_eq!(sd.sent, vec![
            "relabel FileStorage OldName=v1 NewName=v1 PoolName=Full MediaType=File Slot=0 drive=2 MinBlocksize=0 MaxBlocksize=0",
            "relabel FileStorage OldName=v2 NewName=v2 PoolName=Full MediaType=File Slot=3 drive=2 MinBlocksize=0 MaxBlocksize=0",
        ]);
        // The row without Pool and Storage is hidden by the ACL filter tuples
        // but truncated all the same (GetQueryDbids is unfiltered).
        assert!(transcript.contains("| v1 ") && !transcript.contains("| v2 "), "{transcript}");
        assert!(transcript.ends_with("Truncate listed volumes (yes/no)? yes\n\
            Connecting to Storage daemon File at 192.0.2.1:9103 ...\n\
            Sending relabel command from \"v1\" to \"v1\" ...\n\
            3000 OK label. VolFiles=0 VolBytes=225 Volume=\"v1\" Device=\"FileStorage\" (/s)\n\
            The volume 'v1' has been truncated.\n\
            Sending relabel command from \"v2\" to \"v2\" ...\n\
            3000 OK label. VolBytes=226 Volume=\"v2\" Device=x\n\
            The volume 'v2' has been truncated.\n"), "{transcript}");
        assert_eq!(writes.len(), 5, "{writes:?}");
        assert!(writes[0].starts_with("UPDATE Media SET LabelDate='") && writes[0].ends_with("' WHERE VolumeName='v1'"));
        assert_eq!(writes[1], "UPDATE Media SET VolFiles=0,VolBytes=225,VolStatus='Purged',InChanger=0,StorageId=1,\
            EncryptionKey=COALESCE(EncryptionKey,'') WHERE VolumeName='v1'");
        assert_eq!(writes[3], "UPDATE Media SET VolFiles=0,VolBytes=226,VolStatus='Purged',InChanger=1,StorageId=1,\
            EncryptionKey=COALESCE(EncryptionKey,'') WHERE VolumeName='v2'");
        // MakeInchangerUnique for the volume in slot 3.
        assert_eq!(writes[4], "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=3 AND StorageId=1 AND MediaId!=8");
    }

    #[test]
    fn truncate_failures_leave_the_catalog_alone() {
        // Connection refused for v1 (so "Connecting" again for v2), a
        // Storage daemon error for v2.
        let mut sd = TruncSd { refuse: 1, ..Default::default() };
        sd.replies.push_back(vec!["3912 Failed to label Volume: ERR=busy\n"]);
        let (ok, transcript, writes) = run_truncate("truncate volstatus=Purged storage=File yes", &[], &mut sd, truncate_rows);
        assert!(ok);
        assert_eq!((sd.connects.len(), sd.sent.len()), (2, 1));
        assert!(writes.is_empty(), "{writes:?}");
        assert!(transcript.ends_with("Connecting to Storage daemon File at 192.0.2.1:9103 ...\n\
            Failed to connect to Storage daemon.\nLabel command failed for Volume v1.\n\
            Connecting to Storage daemon File at 192.0.2.1:9103 ...\n\
            Sending relabel command from \"v2\" to \"v2\" ...\n3912 Failed to label Volume: ERR=busy\n\
            Label command failed for Volume v2.\n"), "{transcript}");
        // Volumes of different storages are refused before the confirmation.
        fn two_storages(sql: &str) -> Vec<Vec<Option<&'static str>>> {
            if sql.ends_with("FROM Media WHERE MediaId=8") {
                vec![vec![Some("8"), Some("v2"), Some("1"), Some("2"), Some("Purged"), Some("File"), Some("0"), Some("0"), Some("0")]]
            } else {
                truncate_rows(sql)
            }
        }
        let (ok, transcript, writes) = run_truncate("truncate volstatus=Purged storage=File", &[], &mut TruncSd::default(), two_storages);
        assert!(!ok && writes.is_empty());
        assert!(transcript.ends_with("Selected volumes are from different storages. This is not supported. \
            Please choose only volumes from a single storage.\n"), "{transcript}");
        // No candidates: the empty listing, success, nothing else.
        let (ok, transcript, _) = run_truncate("truncate volstatus=Purged storage=File", &[], &mut TruncSd::default(), |_| vec![]);
        assert!(ok);
        assert_eq!(transcript, "No results to list.\n");
    }
}

/// `update slots` against a fake SD and a catalog double (C-054).
#[cfg(test)]
mod update_slots_tests {
    use super::*;
    use std::sync::Arc;

    /// Fake SD for `update slots` (reply formats of stored/dir_cmd.cc
    /// ChangerCmd/ReadlabelCmd and stored/autochanger.cc): records commands.
    struct SlotsSd { sent: Vec<String>, answer: fn(&str) -> Vec<String> }

    impl crate::ua_storage_select::SdLink for SlotsSd {
        fn exchange(&mut self, _store: &bareos_config::resources::StorageResource, command: &str)
            -> Result<Vec<String>, String> {
            self.sent.push(command.to_string());
            Ok((self.answer)(command).into_iter().map(|m| format!("{m}\n")).collect())
        }
    }

    /// Catalog double: StorageId 3 for "Changer…" (none for other storages),
    /// media vol1 (MediaId 1, slot 1, in changer, StorageId 3) and vol2
    /// (MediaId 2, slot 0, not in changer, StorageId 0); statements with
    /// "Slot=4 " fail. Every statement is logged.
    fn slots_catalog() -> (crate::catalog_worker::CatalogHandle, Arc<std::sync::Mutex<Vec<String>>>) {
        use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        let handle = crate::catalog_worker::CatalogHandle::mock(move |request| {
            let sql = match request {
                CatalogRequest::TextRows { sql } | CatalogRequest::RawQuery { sql } => sql,
                _ => "other request".to_string(),
            };
            seen.lock().unwrap().push(sql.clone());
            let table = |columns: &[&str], rows: Vec<Vec<&str>>| CatalogResponse::Rows(SqlTable {
                columns: columns.iter().map(|c| SqlColumn { name: c.to_string(), numeric: false }).collect(),
                rows: rows.into_iter().map(|r| r.into_iter().map(|v| Some(v.to_string())).collect()).collect() });
            if sql.contains("Slot=4 ") {
                CatalogResponse::Error("ERROR:  simulated failure".into())
            } else if sql.starts_with("SELECT StorageId FROM Storage") {
                table(&["storageid"], if sql.contains("'Changer") { vec![vec!["3"]] } else { vec![] })
            } else if sql.starts_with("SELECT MediaId,VolumeName,Slot,InChanger,StorageId FROM Media") {
                let columns = ["mediaid", "volumename", "slot", "inchanger", "storageid"];
                if sql.ends_with("'vol1'") {
                    table(&columns, vec![vec!["1", "vol1", "1", "1", "3"]])
                } else if sql.ends_with("'vol2'") {
                    table(&columns, vec![vec!["2", "vol2", "0", "0", "0"]])
                } else {
                    table(&columns, vec![])
                }
            } else {
                table(&[], vec![])
            }
        });
        (handle, log)
    }

    fn run_update_slots(storage: &str, line: &str, answer: fn(&str) -> Vec<String>)
        -> (String, Vec<String>, Vec<String>) {
        let mut ctx = side_effect_tests::context();
        let (catalog, log) = slots_catalog();
        ctx.catalog = Some(catalog);
        let store = bareos_config::resources::StorageResource { name: storage.into(), address: "sd.example".into(),
            port: 9103, device: "Dev".into(), ..Default::default() };
        let env = crate::ua_storage_select::Env { storages: vec![(store.clone(), true, true)], running: vec![] };
        let ast = crate::session::parse_cli_command(line);
        let mut sd = SlotsSd { sent: vec![], answer };
        let mut ua = crate::ua_select::Ua::new(&ctx);
        update_slots(&mut ua, &ast.arguments, &env, &store, &mut sd);
        let result = ua.finish(true);
        let output = format!("{}{}", result.text_output, if result.errors_inline { "" } else { &result.error_message });
        let sql = log.lock().unwrap().clone();
        (output, sd.sent, sql)
    }

    fn connect(storage: &str) -> String {
        format!("Connecting to Storage daemon {storage} at sd.example:9103 ...\n")
    }

    fn writes(sql: &[String]) -> Vec<&str> {
        sql.iter().filter(|q| q.starts_with("UPDATE")).map(String::as_str).collect()
    }

    #[test]
    fn update_slots_from_barcodes() {
        let (output, sent, sql) = run_update_slots("ChangerBar", "update slots", |cmd| match cmd {
            c if c.starts_with("autochanger slots") => vec!["slots=4".into()],
            c if c.starts_with("autochanger list") => vec!["1:vol1".into(), "2:vol2".into(), "3:volX".into(),
                "5:vol5".into(), "3995 Bad autochanger \"list\" command".into()],
            _ => vec![],
        });
        assert_eq!(sent, ["autochanger slots Dev\n", "autochanger list Dev \n"]);
        let c = connect("ChangerBar");
        assert_eq!(output, format!("{c}Device \"Dev\" has 4 slots.\n{c}\
            3995 Bad autochanger \"list\" command\n\
            Catalog record for Volume \"vol1\" is up to date.\n\
            Catalog record for Volume \"vol2\" updated to reference slot 2.\n\
            Volume \"volX\" not found in catalog. Slot=3 InChanger set to zero.\n\
            Slot 5 greater than max 4 ignored.\n\
            Could not set inchanger to 0 for slot 4.\n"));
        assert_eq!(writes(&sql), [
            "UPDATE Media SET InChanger=0 WHERE StorageId=0",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=1 AND StorageId=3 AND VolumeName!='vol1'",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=2 AND StorageId=3 AND VolumeName!='vol2'",
            "UPDATE Media SET Slot=2,InChanger=1,StorageId=3 WHERE VolumeName='vol2'",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=2 AND StorageId=3 AND MediaId!=2",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=3 AND StorageId=3 AND VolumeName!='volX'",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=4 AND StorageId=3",
        ]);
    }

    #[test]
    fn update_slots_scan_reads_the_labels() {
        let (output, sent, sql) = run_update_slots("ChangerScan", "update slots=1-2 scan drive=1 enabled=no", |cmd| match cmd {
            c if c.starts_with("autochanger slots") => vec!["slots=3".into()],
            c if c.starts_with("autochanger list") => vec!["1:".into(), "2:vol2".into(), "3:vol3".into()],
            c if c.starts_with("readlabel Dev Slot=1 ") => vec!["3301 Issuing autochanger \"loaded? drive 1\" command.".into(),
                "3001 Volume=vol2 Slot=1".into(), "3001 not shown".into()],
            _ => vec!["3902 Cannot mount Volume on Storage Device \"Dev\".".into()],
        });
        assert_eq!(sent, ["autochanger slots Dev\n", "autochanger list Dev \n",
            "readlabel Dev Slot=1 drive=1\n", "readlabel Dev Slot=2 drive=1\n"]);
        let c = connect("ChangerScan");
        assert_eq!(output, format!("{c}Device \"Dev\" has 3 slots.\n{c}\
            {c}3301 Issuing autochanger \"loaded? drive 1\" command.\n3001 Volume=vol2 Slot=1\n\
            Catalog record for Volume \"vol2\" updated to reference slot 1.\n\
            {c}3902 Cannot mount Volume on Storage Device \"Dev\".\n\
            No VolName for Slot=2 InChanger set to zero.\n"));
        assert_eq!(writes(&sql), [
            "UPDATE Media SET InChanger=0 WHERE StorageId=0",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=1 AND StorageId=3 AND VolumeName!='vol2'",
            "UPDATE Media SET Slot=1,InChanger=1,StorageId=3,Enabled=0 WHERE VolumeName='vol2'",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=1 AND StorageId=3 AND MediaId!=2",
            "UPDATE Media SET InChanger=0, Slot=0 WHERE Slot=2 AND StorageId=3",
        ], "slot 3 is outside slots=1-2");
    }

    #[test]
    fn update_slots_stops_before_the_sd_on_bad_arguments() {
        let none = |_: &str| -> Vec<String> { panic!("must not contact the SD") };
        let (output, sent, _) = run_update_slots("ChangerArgs", "update slots enabled=bogus", none);
        assert_eq!(output, "Invalid Enabled value, it must be yes, no, archived, 0, 1, or 2\n");
        assert!(sent.is_empty());
        // A bad slot list is silent upstream (Dmsg only).
        let (output, sent, sql) = run_update_slots("ChangerList", "update slots=3-1", |_| vec!["slots=4".into()]);
        assert_eq!(output, format!("{}Device \"Dev\" has 4 slots.\n", connect("ChangerList")));
        assert_eq!(sent, ["autochanger slots Dev\n"]);
        assert!(writes(&sql).is_empty());
    }

    #[test]
    fn update_slots_on_a_plain_device_and_without_storage_record() {
        let (output, _, _) = run_update_slots("PlainSlots", "update slots",
            |_| vec!["3998 Device \"Dev\" is not an autochanger.".into()]);
        assert_eq!(output, format!("{}3998 Device \"Dev\" is not an autochanger.\nDevice \"Dev\" has 0 slots.\n\
            No slots in changer to scan.\n", connect("PlainSlots")));
        // Empty list: warning, no catalog change.
        let (output, _, sql) = run_update_slots("EmptySlots", "update slots", |cmd|
            if cmd.starts_with("autochanger slots") { vec!["slots=2".into()] } else { vec![] });
        assert!(output.ends_with("No Volumes found to update, or no barcodes.\n"), "{output}");
        assert!(writes(&sql).is_empty());
        // StorageId 0: MakeInchangerUnique does nothing.
        let (output, _, sql) = run_update_slots("NoRecord", "update slots", |cmd|
            if cmd.starts_with("autochanger slots") { vec!["slots=2".into()] } else { vec!["1:vol1".into()] });
        assert!(output.ends_with("Catalog record for Volume \"vol1\" updated to reference slot 1.\n"), "{output}");
        assert_eq!(writes(&sql), ["UPDATE Media SET InChanger=0 WHERE StorageId=0",
            "UPDATE Media SET Slot=1,InChanger=1,StorageId=0 WHERE VolumeName='vol1'"]);
    }
}
