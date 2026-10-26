//! The Storage daemon and catalog part of `label`/`relabel` (ua_label.cc):
//! SendLabelRequest / native_send_label_request, update_database, the
//! deletion of the relabeled volume, the automount on the same SD
//! connection and label_from_barcodes. Compatibility log C-084.

use crate::catreq::{self, Cat, MediaRecord, PoolRecord};
use crate::commands::admin::LabelRequest;
use crate::commands::bvfs::Db;
use crate::commands::purge::SdSession;
use crate::sd_label::{bash_spaces, parse_label_ok};
use crate::ua_select::Ua;
use crate::ua_storage_select::{self as storage, SdLink};
use bareos_config::resources::StorageResource;

/// ua->jcr->store_bsock: opened by the first request, kept for the
/// automount and the following labels, closed when dropped.
pub(crate) struct LabelSd<'s> {
    pub sd: &'s mut dyn SdSession,
    pub connected: bool,
}

impl LabelSd<'_> {
    /// open_sd_bsock() (sd_cmds.cc:159).
    fn open(&mut self, ua: &mut Ua, store: &StorageResource) -> bool {
        if !self.connected {
            ua.send_msg(format!("Connecting to Storage daemon {} at {}:{} ...\n", store.name, store.address, store.port));
            if self.sd.connect(store).is_err() {
                ua.error_msg("Failed to connect to Storage daemon.\n");
                return false;
            }
            self.connected = true;
        }
        true
    }
}

/// The catalog of the command with strerror().
struct Catalog<'d> {
    db: &'d dyn Db,
    errmsg: String,
    msgs: catreq::Messages,
}

impl<'d> Catalog<'d> {
    fn new(db: &'d dyn Db) -> Self {
        Catalog { db, errmsg: String::new(), msgs: Vec::new() }
    }

    fn cat(&mut self) -> Cat<'_> {
        Cat::new(self.db, &mut self.errmsg, &mut self.msgs)
    }
}

/// StorageResource::StorageId (the catalog record set at start).
fn storage_id(ua: &Ua, store: &StorageResource) -> i64 {
    crate::ua_select::get_storage_record(ua.ctx, &store.name).unwrap_or(0)
}

/// native_send_label_request() (ua_label.cc:106): true for a
/// `3000 OK label` answer; LabelDate, VolFiles and VolBytes are set.
#[allow(clippy::too_many_arguments)]
fn native_send_label_request(ua: &mut Ua, sd: &mut LabelSd, store: &StorageResource, mr: &mut MediaRecord,
    omr: Option<&MediaRecord>, pr: &PoolRecord, drive: i16) -> bool {
    if !sd.open(ua, store) {
        return false;
    }
    let dev_name = storage::dev_name(store);
    let volume = bash_spaces(&mr.volume_name);
    let command = match omr {
        Some(omr) => {
            let old = bash_spaces(&omr.volume_name);
            let command = format!("relabel {dev_name} OldName={old} NewName={volume} PoolName={} MediaType={} \
                Slot={} drive={drive} MinBlocksize={} MaxBlocksize={}", bash_spaces(&pr.name),
                bash_spaces(&mr.media_type), mr.slot as i16, omr.min_blocksize as i32, omr.max_blocksize as i32);
            ua.send_msg(format!("Sending relabel command from \"{old}\" to \"{volume}\" ...\n"));
            command
        }
        None => {
            let command = format!("label {dev_name} VolumeName={volume} PoolName={} MediaType={} \
                Slot={} drive={drive} MinBlocksize={} MaxBlocksize={}", bash_spaces(&pr.name),
                bash_spaces(&mr.media_type), mr.slot as i16, pr.min_blocksize as i32, pr.max_blocksize as i32);
            ua.send_msg(format!("Sending label command for Volume \"{volume}\" Slot {} ...\n", mr.slot as i16));
            command
        }
    };
    let mut ok = false;
    let (mut vol_files, mut vol_bytes) = (0u32, 0u64);
    for message in sd.sd.exchange(&command) {
        ua.send_msg(message.clone());
        if let Some((files, bytes)) = parse_label_ok(&message) {
            if let Some(files) = files {
                vol_files = files;
            }
            vol_bytes = bytes;
            ok = true;
        }
    }
    mr.label_date = unsafe { libc::time(std::ptr::null_mut()) } as i64;
    mr.set_label_date = true;
    mr.vol_files = vol_files;
    mr.vol_bytes = vol_bytes;
    ok
}

/// update_database() (ua_label.cc:57).
fn update_database(ua: &mut Ua, catalog: &mut Catalog, store: &StorageResource, mr: &mut MediaRecord,
    pr: &mut PoolRecord, media_record_exists: bool) -> bool {
    let sid = storage_id(ua, store);
    if media_record_exists {
        if mr.vol_bytes > 0 && mr.vol_status == "Unlabeled" {
            mr.vol_status = "Append".into();
        }
        mr.in_changer = i32::from(mr.slot > 0);
        mr.storage_id = sid;
        if !catreq::update_media_record(&mut catalog.cat(), mr) {
            ua.error_msg(catalog.errmsg.clone());
            return false;
        }
        return true;
    }
    catreq::set_pool_defaults(mr, pr);
    mr.in_changer = i32::from(mr.slot > 0);
    mr.enabled = 1;
    mr.storage_id = sid;
    if mr.vol_bytes > 0 {
        mr.vol_status = "Append".into();
    }
    if catreq::create_media_record(&mut catalog.cat(), mr) {
        ua.info_msg(format!("Catalog record for Volume \"{}\", Slot {} successfully created.\n",
            mr.volume_name, mr.slot as i16));
        pr.num_vols = pr.num_vols.wrapping_add(1);
        if !catreq::update_pool_record(&mut catalog.cat(), pr) {
            ua.error_msg(catalog.errmsg.clone());
        }
        true
    } else {
        ua.error_msg(catalog.errmsg.clone());
        false
    }
}

/// SendLabelRequest() (ua_label.cc:186).
#[allow(clippy::too_many_arguments)]
fn send_label_request(ua: &mut Ua, sd: &mut LabelSd, catalog: &mut Catalog, store: &StorageResource,
    mr: &mut MediaRecord, omr: Option<&MediaRecord>, pr: &mut PoolRecord, media_record_exists: bool,
    drive: i16) -> bool {
    let ok = if store.protocol.is_ndmp() {
        // NdmpSendLabelRequest() needs the robot session (C-050).
        let autochanger = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
            .is_some_and(|c| crate::ua_select::is_autochanger(&c, &store.name));
        if let Some(message) = crate::ndmp::robot_session(store, autochanger) {
            ua.error_msg(message);
        }
        false
    } else {
        native_send_label_request(ua, sd, store, mr, omr, pr, drive)
    };
    if ok {
        update_database(ua, catalog, store, mr, pr, media_record_exists)
    } else {
        ua.error_msg(format!("Label command failed for Volume {}.\n", mr.volume_name));
        false
    }
}

/// DeleteMediaRecord(): DoMediaPurge unless Purged, then the record.
fn delete_media_record(db: &dyn Db, omr: &MediaRecord) -> Result<(), String> {
    if omr.vol_status != "Purged" {
        let jobids = db.rows(&format!("SELECT JobId from JobMedia WHERE MediaId={}", omr.media_id))
            .map(|t| t.rows.into_iter().filter_map(|r| r.into_iter().next().flatten()).collect::<Vec<_>>())
            .unwrap_or_default();
        for jobid in jobids {
            for table in ["Job", "File", "JobMedia"] {
                let _ = db.exec(&format!("DELETE FROM {table} WHERE JobId={jobid}"));
            }
        }
    }
    db.exec(&format!("DELETE FROM Media WHERE MediaId={}", omr.media_id as i32)).map(|_| ())
}

fn find_store(ua: &Ua, name: &str) -> Option<StorageResource> {
    ua.ctx.config.as_ref()?.read().ok()?.find_storage(name).cloned()
}

fn pool_record(catalog: &mut Catalog, name: &str) -> PoolRecord {
    let mut pr = PoolRecord { name: name.to_string(), ..Default::default() };
    catreq::get_pool_record(&mut catalog.cat(), &mut pr);
    pr
}

/// The rest of do_label (ua_label.cc:530) after the dialog.
pub(crate) fn label_execute(ua: &mut Ua, request: LabelRequest, sd: &mut dyn SdSession, link: &mut impl SdLink) {
    let Some(db) = ua.ctx.catalog.clone() else { return };
    let mut catalog = Catalog::new(&db);
    let mut sd = LabelSd { sd, connected: false };
    match request {
        LabelRequest::Barcodes { storage, drive, yes, args } => {
            let Some(store) = find_store(ua, &storage) else { return };
            let args: Vec<_> = args.into_iter().map(|(key, value)| crate::command_ast::Argument { key, value }).collect();
            label_from_barcodes(ua, &mut sd, &mut catalog, link, &store, drive, yes, &args);
        }
        LabelRequest::Label { storage, volume, pool, old_volume, slot, drive, existing } => {
            let Some(store) = find_store(ua, &storage) else { return };
            let mut mr = MediaRecord { volume_name: volume.clone(), ..Default::default() };
            if existing {
                catreq::get_media_record(&mut catalog.cat(), &mut mr);
            }
            mr.slot = slot;
            mr.in_changer = i32::from(slot > 0);
            mr.storage_id = storage_id(ua, &store);
            mr.media_type = store.media_type.clone();
            let mut pr = pool_record(&mut catalog, &pool);
            let omr = old_volume.map(|name| {
                let mut omr = MediaRecord { volume_name: name, ..Default::default() };
                catreq::get_media_record(&mut catalog.cat(), &mut omr);
                omr
            });
            let ok = send_label_request(ua, &mut sd, &mut catalog, &store, &mut mr, omr.as_ref(), &mut pr, existing, drive);
            let mut print_reminder = true;
            if ok {
                if let Some(omr) = &omr {
                    match delete_media_record(&db, omr) {
                        Err(error) => ua.error_msg(format!("Delete of Volume \"{}\" failed. ERR={error}", omr.volume_name)),
                        Ok(()) => {
                            ua.info_msg(format!("Old volume \"{}\" deleted from catalog.\n", omr.volume_name));
                            pr.num_vols = pr.num_vols.wrapping_sub(1);
                            if !catreq::update_pool_record(&mut catalog.cat(), &mut pr) {
                                ua.error_msg(catalog.errmsg.clone());
                            }
                        }
                    }
                }
                let automount = ua.ctx.session_mgr.as_ref().is_none_or(|mgr| mgr.session_automount(ua.ctx.session_id));
                if automount {
                    let dev_name = storage::dev_name(&store);
                    ua.info_msg(format!("Requesting to mount {} ...\n", dev_name.replace('\u{1}', " ")));
                    for message in sd.sd.exchange(&format!("mount {dev_name} drive={drive}")) {
                        ua.send_msg(message.clone());
                        if ["3001 ", "3002 ", "3906 "].iter().any(|p| message.starts_with(p)) {
                            print_reminder = false;
                        }
                    }
                }
            }
            if print_reminder {
                ua.info_msg("Do not forget to mount the drive!!!\n");
            }
        }
    }
}

/// IsCleaningTape() (ua_label.cc:226): the CleaningPrefix of the pool
/// resource; None when the resource is missing (error printed).
fn is_cleaning_tape(ua: &mut Ua, volume: &str, pool: &str) -> Option<bool> {
    let prefix = ua.ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|cfg| {
        cfg.find_pool(pool)?;
        Some(crate::commands::defaults::raw(&cfg, "pool", pool)
            .and_then(|r| crate::commands::defaults::first(&r.directives, "cleaningprefix").map(str::to_string))
            .unwrap_or_else(|| "CLN".to_string()))
    });
    match prefix {
        Some(prefix) => Some(volume.starts_with(&prefix)),
        None => {
            ua.error_msg(format!("Pool \"{pool}\" resource not found for volume \"{volume}\"!\n"));
            None
        }
    }
}

/// label_from_barcodes() (ua_label.cc:257).
#[allow(clippy::too_many_arguments)]
fn label_from_barcodes(ua: &mut Ua, sd: &mut LabelSd, catalog: &mut Catalog, link: &mut impl SdLink,
    store: &StorageResource, drive: i16, yes: bool, args: &[crate::command_ast::Argument]) {
    let max_slots = storage::get_num_slots(ua, link, store);
    if max_slots <= 0 {
        ua.warning_msg("No slots in changer to scan.\n");
        return;
    }
    let Some(slots) = storage::get_user_slot_list(args, "slots", i32::from(max_slots)) else { return };
    let Some(list) = storage::get_vol_list_from_storage(ua, link, store, false, false, true) else {
        ua.warning_msg("No Volumes found to label, or no barcodes.\n");
        return;
    };
    let selected = |slot: u16| slots.get(usize::from(slot.wrapping_sub(1))).copied().unwrap_or(false);
    ua.send_msg("The following Volumes will be labeled:\nSlot  Volume\n==============\n");
    for vl in &list {
        if let (Some(name), true) = (&vl.vol_name, selected(vl.slot)) {
            ua.send_msg(format!("{:4}  {name}\n", vl.slot as i32));
        }
    }
    if !yes && ua.get_yesno("Do you want to label these Volumes? (yes|no): ") != Some(true) {
        return;
    }
    let ast = crate::command_ast::CommandAst { arguments: args.to_vec(), ..Default::default() };
    let Some(pool) = crate::ua_select::select_pool_dbr(ua, &ast, "pool") else { return };
    let mut pr = pool_record(catalog, &pool);
    let sid = storage_id(ua, store);
    for vl in &list {
        let Some(name) = vl.vol_name.clone().filter(|_| selected(vl.slot)) else { continue };
        let mut mr = MediaRecord { volume_name: name, ..Default::default() };
        let mut exists = false;
        if catreq::get_media_record(&mut catalog.cat(), &mut mr) {
            if mr.vol_bytes != 0 {
                ua.warning_msg(format!("Media record for Slot {} Volume \"{}\" already exists.\n",
                    vl.slot as i16, mr.volume_name));
                mr.slot = i32::from(vl.slot);
                mr.in_changer = i32::from(mr.slot > 0);
                mr.storage_id = sid;
                if !catreq::update_media_record(&mut catalog.cat(), &mr) {
                    ua.error_msg(format!("Error setting InChanger: ERR={}", catalog.errmsg));
                }
                continue;
            }
            exists = true;
        }
        mr.in_changer = i32::from(mr.slot > 0);
        mr.storage_id = sid;
        match is_cleaning_tape(ua, &mr.volume_name, &pr.name) {
            Some(true) => {
                if exists {
                    mr.vol_bytes = 1;
                    mr.vol_status = "Cleaning".into();
                    mr.media_type.clear();
                    if !catreq::update_media_record(&mut catalog.cat(), &mr) {
                        ua.error_msg(catalog.errmsg.clone());
                    }
                } else {
                    if pr.max_vols > 0 && pr.num_vols >= pr.max_vols {
                        ua.error_msg(format!("Maximum pool Volumes={} reached.\n", pr.max_vols));
                        return;
                    }
                    catreq::set_pool_defaults(&mut mr, &pr);
                    mr.vol_status = "Cleaning".into();
                    mr.media_type.clear();
                    mr.storage_id = sid;
                    if catreq::create_media_record(&mut catalog.cat(), &mut mr) {
                        ua.send_msg(format!("Catalog record for cleaning tape \"{}\" successfully created.\n",
                            mr.volume_name));
                        pr.num_vols = pr.num_vols.wrapping_add(1);
                        if !catreq::update_pool_record(&mut catalog.cat(), &mut pr) {
                            ua.error_msg(catalog.errmsg.clone());
                        }
                    } else {
                        ua.error_msg(format!("Catalog error on cleaning tape: {}", catalog.errmsg));
                    }
                }
                continue;
            }
            // IsCleaningTape() false (also for a missing pool resource).
            _ => {}
        }
        mr.media_type = store.media_type.clone();
        mr.slot = i32::from(vl.slot);
        send_label_request(ua, sd, catalog, store, &mut mr, None, &mut pr, exists, drive);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
    use crate::command_executor::{ApiMode, ExecContext};
    use std::sync::{Arc, Mutex, RwLock};

    /// Fake SD session: answers by command prefix, records connects/commands.
    struct FakeSd { connects: usize, sent: Vec<String>, answer: fn(&str) -> Vec<&'static str> }

    impl SdSession for FakeSd {
        fn connect(&mut self, _store: &StorageResource) -> Result<(), String> {
            self.connects += 1;
            Ok(())
        }
        fn exchange(&mut self, command: &str) -> Vec<String> {
            self.sent.push(command.to_string());
            (self.answer)(command).into_iter().map(|m| format!("{m}\n")).collect()
        }
    }

    struct NoLink;

    impl SdLink for NoLink {
        fn exchange(&mut self, _store: &StorageResource, command: &str) -> Result<Vec<String>, String> {
            panic!("{command}: no slot query expected")
        }
    }

    /// Catalog double: pool Full (PoolId 3, Min/MaxBlocksize 0/0), storage
    /// File (StorageId 1), old volume old1 (MediaId 9, Purged); every
    /// statement is logged.
    fn catalog() -> (CatalogHandle, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let handle = CatalogHandle::mock(move |request| {
            let (sql, execute) = match request {
                CatalogRequest::TextRows { sql } | CatalogRequest::RawQuery { sql } => (sql, false),
                CatalogRequest::Execute { sql } => (sql, true),
                _ => (String::new(), false),
            };
            seen.lock().unwrap().push(sql.clone());
            if execute {
                return CatalogResponse::Ok(1);
            }
            let rows = |rows: Vec<Vec<&str>>| CatalogResponse::Rows(SqlTable {
                columns: (0..rows.first().map_or(1, |r| r.len())).map(|i| SqlColumn { name: format!("c{i}"), numeric: false }).collect(),
                rows: rows.into_iter().map(|r| r.into_iter().map(|v| Some(v.to_string())).collect()).collect() });
            if sql.starts_with("SELECT StorageId FROM Storage") {
                rows(vec![vec!["1"]])
            } else if sql.starts_with("SELECT PoolId,Name,NumVols") {
                rows(vec![vec!["3", "Full", "1", "100", "0", "1", "0", "1", "1", "31536000", "0", "0", "0", "0",
                    "Backup", "0", "Full-", "0", "0", "0", "0", "0"]])
            } else if sql.starts_with("SELECT count(*) from Media") {
                rows(vec![vec!["1"]])
            } else if sql.contains("FROM Media WHERE VolumeName='old1'") {
                let mut row = vec!["0"; 40];
                row[0] = "9"; row[1] = "old1"; row[11] = "Tape"; row[12] = "Purged"; row[13] = "3";
                row[20] = ""; row[21] = ""; row[26] = ""; row[31] = ""; row[37] = "";
                row[38] = "1024"; row[39] = "2048";
                rows(vec![row])
            } else if sql.contains("currval") {
                rows(vec![vec!["12"]])
            } else {
                rows(vec![])
            }
        });
        (handle, log)
    }

    fn context() -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(
            "Director { Name = dir }\nStorage { Name = File; Address = sd.example; Port = 9103; \
             Device = \"File Storage\"; Media Type = McpwFile }\nPool { Name = Full; Pool Type = Backup }").unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))), session_mgr: None,
            catalog: Some(catalog().0), api_mode: ApiMode::Human, bus_rx: None, job_manager: None }
    }

    fn run(request: LabelRequest, answer: fn(&str) -> Vec<&'static str>) -> (String, FakeSd, Vec<String>) {
        let mut ctx = context();
        let (handle, log) = catalog();
        ctx.catalog = Some(handle);
        let mut sd = FakeSd { connects: 0, sent: vec![], answer };
        let mut ua = Ua::new(&ctx);
        label_execute(&mut ua, request, &mut sd, &mut NoLink);
        let result = ua.finish(true);
        let output = format!("{}{}", result.text_output, if result.errors_inline { "" } else { &result.error_message });
        let writes = log.lock().unwrap().iter().filter(|q| !q.starts_with("SELECT")).cloned().collect();
        (output, sd, writes)
    }

    fn label(volume: &str) -> LabelRequest {
        LabelRequest::Label { storage: "File".into(), volume: volume.into(), pool: "Full".into(), old_volume: None,
            slot: 0, drive: 0, existing: false }
    }

    #[test]
    fn label_sends_the_original_command_and_records_after_ok() {
        let (output, sd, writes) = run(label("new1"), |cmd| if cmd.starts_with("label ") {
            vec!["3000 OK label. VolBytes=219 VolABI=0 Volume=\"new1\" Device=\"File Storage\" (/srv)"]
        } else {
            vec!["3001 Device \"File Storage\" (/srv) is mounted with Volume \"new1\""]
        });
        assert_eq!(sd.connects, 1);
        assert_eq!(sd.sent, ["label File\u{1}Storage VolumeName=new1 PoolName=Full MediaType=McpwFile Slot=0 drive=0 \
            MinBlocksize=0 MaxBlocksize=0", "mount File\u{1}Storage drive=0"]);
        assert_eq!(output, "Connecting to Storage daemon File at sd.example:9103 ...\n\
            Sending label command for Volume \"new1\" Slot 0 ...\n\
            3000 OK label. VolBytes=219 VolABI=0 Volume=\"new1\" Device=\"File Storage\" (/srv)\n\
            Catalog record for Volume \"new1\", Slot 0 successfully created.\n\
            Requesting to mount File Storage ...\n\
            3001 Device \"File Storage\" (/srv) is mounted with Volume \"new1\"\n");
        assert!(writes.iter().any(|q| q.starts_with("INSERT INTO Media") && q.contains("'new1','McpwFile'")
            && q.contains("'Append'")), "{writes:?}");
        assert!(writes.iter().any(|q| q.starts_with("UPDATE Pool SET NumVols=")), "{writes:?}");
    }

    #[test]
    fn failed_label_changes_no_catalog_record() {
        let (output, sd, writes) = run(label("new2"), |_| vec!["3903 Error scanning label command: label x"]);
        assert_eq!(sd.sent.len(), 1, "no mount after a failed label");
        assert!(output.ends_with("3903 Error scanning label command: label x\n\
            Label command failed for Volume new2.\nDo not forget to mount the drive!!!\n"), "{output}");
        assert!(writes.is_empty(), "{writes:?}");
    }

    #[test]
    fn relabel_keeps_the_old_blocksizes_and_deletes_the_old_record() {
        let request = LabelRequest::Label { storage: "File".into(), volume: "new 3".into(), pool: "Full".into(),
            old_volume: Some("old1".into()), slot: 0, drive: 1, existing: false };
        let (output, sd, writes) = run(request, |cmd| if cmd.starts_with("relabel ") {
            vec!["3000 OK label. VolFiles=0 VolBytes=219 VolABI=0 Volume=\"new 3\""]
        } else {
            vec!["3906 File device \"File Storage\" is always mounted."]
        });
        assert_eq!(sd.sent[0], "relabel File\u{1}Storage OldName=old1 NewName=new\u{1}3 PoolName=Full MediaType=McpwFile \
            Slot=0 drive=1 MinBlocksize=1024 MaxBlocksize=2048");
        assert!(output.contains("Sending relabel command from \"old1\" to \"new\u{1}3\" ...\n"), "{output}");
        assert!(output.contains("Old volume \"old1\" deleted from catalog.\n"), "{output}");
        assert!(!output.contains("Do not forget"), "{output}");
        assert!(writes.iter().any(|q| q == "DELETE FROM Media WHERE MediaId=9"), "{writes:?}");
        // Purged: no DoMediaPurge.
        assert!(!writes.iter().any(|q| q.starts_with("DELETE FROM Job")), "{writes:?}");
    }
}
