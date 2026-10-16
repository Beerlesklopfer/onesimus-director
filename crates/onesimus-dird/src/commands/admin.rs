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
    let Some(storage) = ua_select::get_storage_resource(ua, ast, true, barcodes) else { return };
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
    // C++ ua_cmds.cc DoMountCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_mount_db(ast, ctx))
}

fn cmd_mount_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    do_mount_cmd(ast, ctx, "mount")
}

pub fn cmd_unmount(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc DoMountCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_unmount_db(ast, ctx))
}

fn cmd_unmount_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    do_mount_cmd(ast, ctx, "unmount")
}

pub fn cmd_release(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_cmds.cc DoMountCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, false, || cmd_release_db(ast, ctx))
}

fn cmd_release_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    do_mount_cmd(ast, ctx, "release")
}

/// C++ ua_cmds.cc DoMountCmd(): storage selection, drive/slot, SD request.
fn do_mount_cmd(ast: &CommandAst, ctx: &ExecContext, cmd: &str) -> CommandResult {
    use crate::ua_storage_select::{Env, NetSd};
    let Some(env) = Env::load(ctx) else { return CommandResult::error("No config\n".into()) };
    let director = ctx.config.as_ref().and_then(|c| c.read().ok())
        .map(|c| c.director.name.clone()).unwrap_or_default();
    let mut sd = NetSd { director };
    CommandResult::ok(crate::ua_storage_select::do_mount_cmd(&ast.arguments, &env, &mut sd, cmd))
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
            // UpdateSlots: an autochanger is required.
            if crate::ua_select::get_storage_resource(ua, ast, true, true).is_some() {
                act(ua, UpdateAction::Unsupported("update slots"));
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

/// The Update<Field> helpers of ua_update.cc for one volume.
fn update_field(ua: &mut crate::ua_select::Ua, act: UpdateAct, field: usize, value: &str,
    media: &crate::ua_select::MediaRecord) {
    use crate::ua_select::{edit_utime, is_yesno};
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
        3 => set("MaxVolJobs", crate::commands::job::str_to_int64(value).to_string(), format!("New max jobs is: {value}\n")),
        4 => set("MaxVolFiles", crate::commands::job::str_to_int64(value).to_string(), format!("New max files is: {value}\n")),
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
        9 => UpdateAction::Unsupported("update volume pool"),
        10 => UpdateAction::Unsupported("update volume frompool"),
        11 => UpdateAction::Unsupported("update volume allfrompool"),
        13 => UpdateAction::Unsupported("update volume recyclepool"),
        14 => UpdateAction::Unsupported("update volume actiononpurge"),
        15 => UpdateAction::Unsupported("update volume storage"),
        _ => UpdateAction::Unsupported("update volume encrypt"),
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
        update_field(ua, act, FIELD[index], value, &media);
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
                update_field(ua, act, 0, &status, &media);
            }
            1 => {
                ua.info_msg(format!("Current retention period is: {}\n", edit_utime(media.vol_retention)));
                let Some(value) = ua.get_cmd("Enter Volume Retention period: ") else { return false };
                update_field(ua, act, 1, &value, &media);
            }
            2 => {
                ua.info_msg(format!("Current use duration is: {}\n", edit_utime(media.vol_use_duration)));
                let Some(value) = ua.get_cmd("Enter Volume Use Duration: ") else { return false };
                update_field(ua, act, 2, &value, &media);
            }
            3 => {
                ua.info_msg(format!("Current max jobs is: {}\n", media.max_vol_jobs));
                if ua.get_pint("Enter new Maximum Jobs: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, act, 3, &value, &media);
            }
            4 => {
                ua.info_msg(format!("Current max files is: {}\n", media.max_vol_files));
                if ua.get_pint("Enter new Maximum Files: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, act, 4, &value, &media);
            }
            5 => {
                ua.info_msg(format!("Current value is: {}\n", media.max_vol_bytes));
                let Some(value) = ua.get_cmd("Enter new Maximum Bytes: ") else { return false };
                update_field(ua, act, 5, &value, &media);
            }
            6 => {
                ua.info_msg(format!("Current recycle flag is: {}\n", if media.recycle == 1 { "yes" } else { "no" }));
                if ua.get_yesno("Enter new Recycle status: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, act, 6, &value, &media);
            }
            7 => {
                ua.info_msg(format!("Current Slot is: {}\n", media.slot));
                if ua.get_pint("Enter new Slot: ").is_none() { return false; }
                let value = ua.cmd.clone();
                update_field(ua, act, 8, &value, &media);
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
                if ua.get_cmd("Enter new Pool name: ").is_none() { return false; }
                act(ua, UpdateAction::Unsupported("update volume pool"));
                return true;
            }
            11 => {
                act(ua, UpdateAction::Unsupported("update volume frompool"));
                return true;
            }
            12 => {
                if ua_select::select_pool_resource(ua).is_some() {
                    act(ua, UpdateAction::Unsupported("update volume allfrompool"));
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
                update_field(ua, act, 12, &value, &media);
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
            match cat.raw_query(&format!("UPDATE Media SET {column}={value} WHERE MediaId={media_id}")) {
                Ok(_) => ua.info_msg(message),
                Err(e) => ua.error_msg(format!("{e}\n")),
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
        UpdateAction::Unsupported(what) => ua.send_msg(format!("{what}: not supported\n")),
    }
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
    Files { client: String, retention: u64 },
    Jobs { client: String, retention: u64 },
    Volume(String),
    Stats(u64),
    Directory { client: Option<String>, path: String },
}

/// PruneCmd (ua_prune.cc:362) dialogs: keyword menu, client/pool selection,
/// volume selection and ConfirmRetention ("Continue? (yes/mod/no): ", only
/// `yes` skips it). `act` runs only for a confirmed request.
fn prune_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    act: impl FnOnce(&mut crate::ua_select::Ua, PruneAction) -> bool) -> bool {
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
        .or_else(|| ast.arguments.iter().any(|a| a.key.len() >= 6 && a.key[..6].eq_ignore_ascii_case("volume")).then_some(2))
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
                act(ua, PruneAction::Files { client, retention });
                true
            } else {
                act(ua, PruneAction::Jobs { client, retention })
            }
        }
        Some(2) => {
            if find_arg(ast, "all").is_some() {
                // PruneAllVolumes (ua_prune.cc:326): every volume (of the
                // pool) with its own ConfirmRetention.
                let pool = pool_of(ua);
                let pool_id = pool.as_deref().and_then(|p| ua_select::get_pool_record(ctx, p).ok());
                let names = ctx.catalog.as_ref()
                    .and_then(|cat| cat.query_rows("SELECT VolumeName FROM Media ORDER BY VolumeName").ok())
                    .map(|t| t.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>())
                    .unwrap_or_default();
                let mut result = true;
                for volume in names {
                    let record = match ua_select::get_media_record(ctx, 0, &volume) {
                        Ok(record) => record,
                        Err(error) => {
                            ua.error_msg(error);
                            return false;
                        }
                    };
                    if pool_id.is_some_and(|id| id != record.pool_id) {
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
                    result &= act_volume(ua, record.volume_name);
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
            if volumes.is_empty() && pool.is_some() {
                // GetPoolPruneReport and its volume selection are not ported.
                ua.error_msg("prune volume pool=: the prunable volume report is not supported yet\n");
                return false;
            }
            let mut result = true;
            if !volumes.is_empty() {
                for volume in volumes {
                    let record = match ua_select::get_media_record(ctx, 0, &volume) {
                        Ok(record) => record,
                        Err(error) => {
                            ua.error_msg(error);
                            return false;
                        }
                    };
                    if record.enabled == 2 {
                        ua.error_msg(format!("Cannot prune Volume \"{}\" because it is archived.\n", record.volume_name));
                        result = false;
                        continue;
                    }
                    if ua.confirm_retention(ast, record.vol_retention, &format!("Volume ({volume})")).is_none() {
                        result = false;
                        continue;
                    }
                    result &= act_volume(ua, record.volume_name);
                }
                return result;
            }
            let Some(record) = ua_select::select_media_dbr(ua, ast) else { return false };
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
            act(ua, PruneAction::Directory { client, path })
        }
        _ => true,
    }
}

/// One volume of `prune volume=a volume=b` (PruneVolume).
fn act_volume(ua: &mut crate::ua_select::Ua, volume: String) -> bool {
    prune_execute(ua, PruneAction::Volume(volume))
}

/// Duration directive of the Director resource (raw configuration).
fn director_duration(ctx: &ExecContext, directive: &str, default: u64) -> u64 {
    ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|config| {
        config.raw.iter().find(|r| r.norm_type() == "director")
            .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == directive))
            .and_then(|d| crate::commands::show::parse_duration(&d.values().join(" ")))
    }).unwrap_or(default)
}

/// The catalog work of a confirmed prune. Only job pruning by client and the
/// pool-wide volume pruning exist in the catalog layer; the other prune
/// kinds report that they are not supported (nothing is changed).
fn prune_execute(ua: &mut crate::ua_select::Ua, action: PruneAction) -> bool {
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return true;
    };
    match action {
        PruneAction::Jobs { client, retention } => {
            let days = retention / 86_400;
            match cat.raw_query(&format!("SELECT sp_prune_expired_jobs('{}', {days})", client.replace('\'', "''"))) {
                Ok(result) => {
                    ua.send_msg(format!("Pruned jobs: {}\n", result.trim()));
                    true
                }
                Err(e) => {
                    ua.error_msg(format!("Prune error: {e}\n"));
                    false
                }
            }
        }
        PruneAction::Files { .. } => { ua.send_msg("prune files: not supported\n"); false }
        PruneAction::Volume(_) => { ua.send_msg("prune volume: not supported\n"); false }
        PruneAction::Stats(_) => { ua.send_msg("prune stats: not supported\n"); false }
        PruneAction::Directory { .. } => { ua.send_msg("prune directory: not supported\n"); false }
    }
}

pub fn cmd_purge(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    let ok = purge_dialog(&mut ua, ast, purge_execute);
    ua.finish(ok)
}

/// What a purge does once its target is known (purge has no confirmation).
#[derive(Debug, PartialEq, Eq)]
enum PurgeAction {
    FilesOfJob(i64),
    FilesOfClient(String),
    JobsOfClient(String),
    JobsOfVolume(String),
    JobsOfPool(String),
    QuotaOfClient(String),
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
            let positions: Vec<usize> = ast.arguments.iter().enumerate()
                .filter(|(_, a)| a.key.eq_ignore_ascii_case("volume")).map(|(i, _)| i).collect();
            for position in positions {
                let rest = CommandAst { arguments: ast.arguments[position..].to_vec(), ..ast.clone() };
                if let Some(media) = ua_select::select_media_dbr(ua, &rest) {
                    act(ua, PurgeAction::JobsOfVolume(media.volume_name));
                }
                ua.send_msg("\n");
            }
            if ua_select::find_arg(ast, "action").is_some() {
                ua.send_msg("purge action=: ActionOnPurge is not supported\n");
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

/// The catalog work of a purge.
fn purge_execute(ua: &mut crate::ua_select::Ua, action: PurgeAction) {
    let Some(cat) = ua.ctx.catalog.clone() else {
        ua.send_msg("No catalog configured\n");
        return;
    };
    let result = match &action {
        PurgeAction::FilesOfJob(jobid) => cat.raw_query(&format!("DELETE FROM file WHERE jobid = {jobid}"))
            .map(|_| format!("Purged files from JobId {jobid}\n")),
        PurgeAction::JobsOfClient(client) => {
            let client = client.replace('\'', "''");
            cat.raw_query(&format!("DELETE FROM jobmedia WHERE jobid IN (SELECT jobid FROM job j JOIN client c ON j.clientid = c.clientid WHERE c.name = '{client}'); \
                 DELETE FROM job WHERE clientid IN (SELECT clientid FROM client WHERE name = '{client}')"))
                .map(|_| format!("Purged jobs for client {client}\n"))
        }
        PurgeAction::JobsOfVolume(volume) => cat.raw_query(&format!("SELECT sp_purge_volume('{}')", volume.replace('\'', "''")))
            .map(|result| format!("Volume \"{volume}\" purged ({} jobmedia records removed)\n", result.trim())),
        PurgeAction::FilesOfClient(_) => Ok("purge files client: not supported\n".to_string()),
        PurgeAction::JobsOfPool(_) => Ok("purge jobs pool: not supported\n".to_string()),
        PurgeAction::QuotaOfClient(_) => Ok("purge quota: not supported\n".to_string()),
    };
    match result {
        Ok(text) => ua.send_msg(text),
        Err(e) => ua.error_msg(format!("Purge error: {e}\n")),
    }
}

/// `truncate volstatus=Purged [storage=] [pool=] [volume=] [drive=] [yes]`
/// (ua_cmds.cc:1972 TruncateCmd). Arguments are validated exactly like
/// upstream before the catalog or a Storage Daemon is touched; truncating
/// needs the confirmation (`yes`), compatibility log C-012.
pub fn cmd_truncate(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    truncate_with(ast, ctx, |storage, command| sd_command(ctx, storage, command))
}

fn truncate_with(
    ast: &CommandAst,
    ctx: &ExecContext,
    mut send: impl FnMut(&str, &str) -> Result<String, String>,
) -> CommandResult {
    use super::prompt::{selection_required, usage};
    use crate::acl::{self, Kind};
    if ast.arguments.is_empty() {
        return CommandResult::error(usage("truncate", "missing parameter"));
    }
    let value = |key: &str| ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref());
    let yes = ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("yes"));
    match value("volstatus") {
        None => return CommandResult::error(usage("truncate", "required parameter 'volstatus' missing")),
        Some(status) if !status.eq_ignore_ascii_case("Purged") => return CommandResult::error(
            usage("truncate", "Invalid parameter. 'volstatus' must be 'Purged'.")),
        Some(_) => {}
    }
    // OpenDb (ua_cmds.cc:2008) precedes the remaining checks.
    let prefix = match crate::ua_db::open_client_db(ast, ctx, false) {
        Ok(prefix) => prefix,
        Err(result) => return result,
    };
    let done = |result: CommandResult| crate::ua_db::with_prefix(prefix.clone(), result);
    let mut parsed = 2; // the command itself and volstatus, as upstream counts
    let storage = value("storage");
    if storage.is_some() || acl::has_restrictions(ctx, Kind::Storage) {
        // SelectStorageDbr (ua_select.cc:823): "Defined Storages:" /
        // "Select Storage resource" dialog when no accessible storage is named.
        if !storage.is_some_and(|name| known_storage(ctx, name)) {
            return done(selection_required(String::new()));
        }
        parsed += 1;
    }
    let pool = value("pool");
    if pool.is_some() || acl::has_restrictions(ctx, Kind::Pool) {
        // SelectPoolDbr (ua_select.cc): "Defined Pools:" / "Select the Pool" dialog.
        if !pool.is_some_and(|name| acl::access(ctx, Kind::Pool, name)) {
            return done(selection_required(String::new()));
        }
        parsed += 1;
    }
    let volume = value("volume").filter(|name| acl::valid_name(name));
    if volume.is_some() { parsed += 1; }
    let mut text = String::new();
    if let Some(drive) = value("drive") {
        if drive.parse::<i32>().is_ok() && !drive.starts_with('+') {
            parsed += 1;
        } else {
            text.push_str(&usage("truncate", &format!("Drive number must be integer but was : {drive}\n")));
        }
    }
    if yes { parsed += 1; }
    if parsed != ast.arguments.len() + 1 {
        text.push_str(&usage("truncate", "Invalid parameter."));
        return done(CommandResult::error(text));
    }

    let Some(cat) = &ctx.catalog else {
        return done(CommandResult::error(format!("{text}Failed to open db.\n")));
    };
    let quote = |v: &str| v.replace('\'', "''");
    let mut query = String::from("SELECT m.volumename FROM media m JOIN pool p ON m.poolid=p.poolid \
        WHERE m.volstatus='Purged' AND m.recycle=1 AND m.enabled=1 AND m.volbytes>64512");
    if let Some(p) = pool { query.push_str(&format!(" AND p.name='{}'", quote(p))); }
    if let Some(s) = storage {
        query.push_str(&format!(" AND m.storageid=(SELECT storageid FROM storage WHERE name='{}')", quote(s)));
    }
    if let Some(v) = volume { query.push_str(&format!(" AND m.volumename='{}'", quote(v))); }
    let volumes: Vec<String> = match cat.query_rows(&format!("{query} ORDER BY m.mediaid")) {
        Ok(table) => table.rows.into_iter().filter_map(|row| row.into_iter().next().flatten()).collect(),
        Err(e) => return done(CommandResult::error(format!("{text}{e}\n"))),
    };
    if volumes.is_empty() {
        return done(CommandResult::ok(text));
    }
    if !yes {
        // GetConfirmation (ua_input.cc:165, ua_cmds.cc:2120): API sessions
        // decline silently; the console is asked "Truncate listed volumes
        // (yes/no)? " after the volume list (ListSqlQuery, C-011).
        if ctx.api_mode != crate::command_executor::ApiMode::Human {
            return done(CommandResult::ok(text));
        }
        return done(selection_required(format!("{text}Truncate listed volumes (yes/no)? ")));
    }
    let stor = match storage {
        Some(s) => s.to_string(),
        None => match ctx.config.as_ref().and_then(|c| c.read().ok())
            .filter(|cfg| cfg.storages.len() == 1).map(|cfg| cfg.storages[0].name.clone()) {
            Some(s) => s,
            None => return done(CommandResult::error(format!("{text}truncate: specify storage=<name>\n"))),
        },
    };
    for vol in volumes {
        match send(&stor, &format!("truncate Volume=\"{vol}\"")) {
            Ok(resp) => {
                let _ = cat.raw_query(&format!(
                    "UPDATE media SET volbytes=0, volfiles=0 WHERE volumename='{}'", quote(&vol)));
                text.push_str(&format!("Truncate Volume \"{vol}\" on Storage \"{stor}\":\n{resp}\n"));
            }
            Err(e) => text.push_str(&format!("truncate {vol}: {e}\n")),
        }
    }
    done(CommandResult::ok(text))
}

fn known_storage(ctx: &ExecContext, name: &str) -> bool {
    ctx.config.as_ref().and_then(|c| c.read().ok()).is_some_and(|cfg| cfg.find_storage(name).is_some())
        && crate::acl::access(ctx, crate::acl::Kind::Storage, name)
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
            let result = truncate_with(&crate::session::parse_cli_command(line), &ctx,
                |_, _| panic!("{line}: must not contact the SD"));
            assert!(!result.success, "{line}");
            assert_eq!(result.error_message, expected, "{line}");
        }
        // Unknown storage: upstream opens the storage selection; nothing is done.
        let result = truncate_with(&crate::session::parse_cli_command("truncate volstatus=Purged storage=Nope"),
            &ctx, |_, _| panic!("must not contact the SD"));
        assert!(result.success && result.error_message.is_empty());
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
            Some(PruneAction::Jobs { client: "fd1".into(), retention: 3 * 86_400 }));
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
        assert_eq!(acted, Some(PruneAction::Jobs { client: "fd1".into(), retention: 180 * 86_400 }));

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
            truncate_with(&ast(line), &ctx, |_, _| panic!("{line}: must not contact the SD"));
        }
        let writes = |log: &Vec<String>| log.iter().filter(|q| !q.trim_start().to_ascii_uppercase().starts_with("SELECT")).count();
        assert_eq!(writes(&log.lock().unwrap()), 0, "{:?}", log.lock().unwrap());
        scripted(&[], || cmd_purge(&ast("purge jobs client=fd1"), &ctx));
        assert_eq!(writes(&log.lock().unwrap()), 1, "control: a complete purge reaches the catalog");
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
        update_field(&mut ua, &mut |_, action| acted.push(action), 0, "full", &media);
        update_field(&mut ua, &mut |_, action| acted.push(action), 0, "bogus", &media);
        update_field(&mut ua, &mut |_, action| acted.push(action), 1, "3 days", &media);
        assert_eq!(acted, vec![
            UpdateAction::Media { media_id: 7, column: "VolStatus", value: "'Full'".into(), message: "New Volume status is: Full\n".into() },
            UpdateAction::Media { media_id: 7, column: "VolRetention", value: "259200".into(), message: "New retention period is: 3 days \n".into() },
        ]);
        assert_eq!(ua.finish(true).error_message, "Invalid VolStatus specified: bogus\n");
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

    #[test]
    fn truncate_unknown_storage_is_a_selection() {
        let ctx = context();
        let result = truncate_with(&crate::session::parse_cli_command("truncate volstatus=Purged storage=Nope"),
            &ctx, |_, _| panic!("must not contact the SD"));
        assert!(result.success && result.error_message.is_empty());
    }
}
