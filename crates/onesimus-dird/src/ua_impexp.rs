//! `import`, `export` and `move` (dird/ua_impexp.cc PerformMoveOperation):
//! slot count and `autochanger listall` from the Storage daemon, slot
//! selections, `autochanger transfer` per volume and the catalog update of
//! the visited slots (ua_update.cc UpdateSlotsFromVolList,
//! UpdateInchangerForExport). Compatibility log C-069/C-071.

use crate::command_ast::Argument;
use crate::ua_select::Ua;
use crate::ua_storage_select::{self as storage, Env, SdLink, VolListEntry};
use bareos_config::resources::StorageResource;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveOp {
    Import,
    Export,
    Move,
}

/// Slot selection, index `n` is slot `n + 1` (the C++ bit lists).
type Slots = Vec<bool>;

fn is_set(list: &[bool], slot: u16) -> bool {
    list.get(usize::from(slot.wrapping_sub(1))).copied().unwrap_or(false)
}

fn set(list: &mut [bool], slot: u16, value: bool) {
    if let Some(bit) = list.get_mut(usize::from(slot.wrapping_sub(1))) {
        *bit = value;
    }
}

fn count_enabled_slots(list: &[bool]) -> u16 {
    list.iter().filter(|b| **b).count() as u16
}

fn is_storage_or_import(vl: &VolListEntry) -> bool {
    vl.slot_type == 'S' || vl.slot_type == 'I'
}

fn loaded_in_drive(list: &[VolListEntry], slot: u16) -> bool {
    storage::vol_is_loaded_in_drive(list, slot).is_some()
}

/// validate_slot_list(): deselect slots without the wanted status
/// (`full`: 'F', else 'E'), considering volumes loaded in a drive.
fn validate_slot_list(ua: &mut Ua, list: &[VolListEntry], slots: &mut Slots, full: bool) {
    for vl in list.iter().filter(|vl| is_storage_or_import(vl)) {
        if !is_set(slots, vl.slot) {
            continue;
        }
        let wanted = if full { 'F' } else { 'E' };
        if vl.content == wanted {
            continue;
        }
        let loaded = vl.slot_type == 'S' && vl.content == 'E' && loaded_in_drive(list, vl.slot);
        if (full && loaded) || (!full && vl.slot_type == 'S' && vl.content == 'E' && !loaded) {
            continue;
        }
        ua.warning_msg(format!("Deselecting slot {} doesn't have wanted status.\n", vl.slot as i16));
        set(slots, vl.slot, false);
    }
}

/// find_slot_in_list(): the full entry holding `slot` (drives by their
/// loaded slot).
fn find_slot_in_list(list: &[VolListEntry], slot: u16) -> Option<usize> {
    list.iter().position(|vl| vl.content == 'F'
        && if vl.slot_type == 'D' { vl.loaded_slot() == slot } else { vl.slot == slot })
}

/// slot_lists_overlap().
fn slot_lists_overlap(src: &[bool], dst: &[bool]) -> bool {
    let mut other = false;
    for (s, d) in src.iter().zip(dst) {
        if *s && *d && !other {
            return true;
        }
        if *s || *d {
            other = true;
        }
    }
    false
}

/// The element address the vol list is sorted by.
fn element_address(vl: &VolListEntry) -> u16 {
    if vl.slot_type == 'D' { vl.slot } else { 100u16.wrapping_add(vl.slot) }
}

/// scan_slots_for_volnames(): read the label of every selected full slot
/// (or of the drive holding an empty slot's volume), then list the changer
/// again and carry the names over. None when the new list fails.
#[allow(clippy::too_many_arguments)]
fn scan_slots_for_volnames(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, drive: i16,
    mut list: Vec<VolListEntry>, src: Option<&Slots>) -> Option<Vec<VolListEntry>> {
    for index in 0..list.len() {
        let vl = list[index].clone();
        if vl.slot_type == 'D' || src.is_some_and(|src| !is_set(src, vl.slot)) {
            continue;
        }
        match vl.content {
            'F' => list[index].vol_name = storage::get_volume_name_from_sd(ua, sd, store, vl.slot, drive),
            'E' if vl.slot_type == 'S' => {
                if let Some(d) = list.iter().position(|e| e.slot_type == 'D' && e.loaded_slot() == vl.slot) {
                    list[d].vol_name = storage::get_volume_name_from_sd(ua, sd, store, vl.slot, drive);
                }
            }
            _ => {}
        }
    }
    let mut new_list = storage::get_vol_list_from_storage(ua, sd, store, true, true, false)?;
    for vl1 in new_list.iter_mut() {
        if vl1.content != 'F' {
            continue;
        }
        let old = match vl1.slot_type {
            'D' => {
                let same = list.iter().position(|e| element_address(e) == element_address(vl1));
                match same.filter(|&i| list[i].content == 'F' && list[i].loaded == vl1.loaded) {
                    Some(i) => Some(i),
                    None => find_slot_in_list(&list, vl1.loaded_slot()),
                }
            }
            'S' | 'I' => {
                if src.is_some_and(|src| !is_set(src, vl1.slot)) {
                    continue;
                }
                let same = list.iter().position(|e| element_address(e) == element_address(vl1));
                match same.filter(|&i| list[i].content == 'F' && list[i].slot == vl1.slot) {
                    Some(i) => Some(i),
                    None => find_slot_in_list(&list, vl1.slot),
                }
            }
            _ => None,
        };
        // Upstream: vl2->VolName = vl1->VolName; vl1->VolName = NULL. The
        // name moves into the old list, which is freed: the matched entry
        // of the returned list has no name.
        if let Some(i) = old {
            list[i].vol_name = vl1.vol_name.take();
        }
    }
    Some(new_list)
}

/// get_slot_list_using_volname(): mark the slot holding `name`.
fn slot_using_volname(ua: &mut Ua, list: &[VolListEntry], name: &str, wanted: Option<&Slots>, selected: &mut Slots)
    -> bool {
    if !crate::acl::valid_name(name) {
        ua.warning_msg(format!("Skipping illegal volumename {name}.\n"));
        return false;
    }
    let found = list.iter().filter(|vl| is_storage_or_import(vl))
        .filter(|vl| wanted.is_none_or(|w| is_set(w, vl.slot)))
        .find(|vl| match vl.content {
            'F' => vl.vol_name.as_deref() == Some(name),
            'E' => storage::vol_is_loaded_in_drive(list, vl.slot).is_some_and(|d| d.vol_name.as_deref() == Some(name)),
            _ => false,
        });
    match found {
        Some(vl) => {
            set(selected, vl.slot, true);
            true
        }
        None => {
            ua.warning_msg(format!("No volume named {name} in changer or in selected source slots.\n"));
            false
        }
    }
}

/// get_slot_list_using_volnames(): every `volume=a|b` argument from the
/// first one on.
fn slots_using_volnames(ua: &mut Ua, args: &[Argument], first: usize, list: &[VolListEntry], wanted: Option<&Slots>,
    selected: &mut Slots) -> u16 {
    let mut count = 0u16;
    for arg in &args[first..] {
        if !arg.key.eq_ignore_ascii_case("volume") {
            continue;
        }
        let value = arg.value.clone().unwrap_or_default();
        let mut tokens: Vec<&str> = value.split('|').collect();
        // The last token is only used when it is not empty.
        if tokens.last().is_some_and(|t| t.is_empty()) {
            tokens.pop();
        }
        for token in tokens {
            if slot_using_volname(ua, list, token, wanted, selected) {
                count = count.wrapping_add(1);
            }
        }
    }
    count
}

/// auto_fill_slot_selection().
fn auto_fill(list: &[VolListEntry], slots: &mut Slots, slot_type: char, content: char) -> u16 {
    let mut count = 0u16;
    for vl in list {
        if vl.slot_type != slot_type || vl.content != content {
            continue;
        }
        if slot_type == 'S' && content == 'E' && loaded_in_drive(list, vl.slot) {
            continue;
        }
        set(slots, vl.slot, true);
        count = count.wrapping_add(1);
    }
    count
}

/// verify_slot_list().
fn verify_slot_list(list: &[VolListEntry], slots: &[bool], slot_type: char, content: char) -> bool {
    for vl in list.iter().filter(|vl| is_storage_or_import(vl)) {
        if !is_set(slots, vl.slot) || (vl.slot_type == slot_type && vl.content == content) {
            continue;
        }
        if vl.slot_type == 'S' && content == 'F' && loaded_in_drive(list, vl.slot) {
            continue;
        }
        return false;
    }
    true
}

/// update_internal_slot_list(): the volume of `source` is now in
/// `destination`.
fn update_internal_slot_list(list: &mut [VolListEntry], source: u16, destination: u16) {
    let from = list.iter().position(|vl| vl.slot_type != 'D' && vl.slot == source);
    let to = list.iter().position(|vl| vl.slot_type != 'D' && vl.slot == destination);
    if let (Some(from), Some(to)) = (from, to) {
        list[to].vol_name = list[from].vol_name.take();
        list[to].content = 'F';
        list[from].content = 'E';
    }
}

/// transfer_volume() / NativeTransferVolume(): `autochanger transfer`;
/// a `39xx` answer is a failure.
fn transfer_volume(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, from: u16, to: u16) -> bool {
    if store.protocol.is_ndmp() {
        // NdmpTransferVolume() needs the robot session (C-050).
        storage::ndmp_robot_session(ua, store);
        return false;
    }
    let command = format!("autochanger transfer {} {} {} \n", storage::dev_name(store), from as i16, to as i16);
    let Some(messages) = storage::open_and_exchange(ua, sd, store, &command) else { return false };
    let mut ok = true;
    for message in messages {
        let msg = message.trim_end_matches([' ', '\n', '\r', '\t']);
        let b = msg.as_bytes();
        if b.len() >= 5 && b[0] == b'3' && b[1..4].iter().all(u8::is_ascii_digit) && b[4] == b' ' && b[1] == b'9' {
            ok = false;
        }
        ua.send_msg(format!("{msg}\n"));
    }
    ok
}

/// release_loaded_volume(): `release` of the drive, then the volume is
/// back in its slot.
fn release_loaded_volume(ua: &mut Ua, sd: &mut impl SdLink, store: &StorageResource, drive: u16,
    list: &mut [VolListEntry]) -> bool {
    if !storage::volume_operation(ua, sd, store, "release", drive as i16, -1) {
        return false;
    }
    // The loop stops at the drive or at the first non-drive entry.
    let Some(d) = list.iter().position(|vl| vl.slot_type != 'D' || vl.slot == drive) else { return false };
    let loaded = list[d].loaded_slot();
    let Some(s) = list.iter().position(|vl| vl.slot_type != 'D' && vl.slot == loaded) else { return false };
    list[s].vol_name = list[d].vol_name.take();
    list[s].content = 'F';
    list[d].content = 'E';
    list[d].loaded = Some(0);
    true
}

/// The catalog side of the move (StorageId of the storage record).
pub trait MoveCatalog {
    /// The storage of the command (store->StorageId is its record).
    fn set_storage(&mut self, _ctx: &crate::command_executor::ExecContext, _storage: &str) {}
    /// MakeInchangerUnique() for slot/volume (InChanger 1).
    fn make_inchanger_unique(&mut self, slot: u16, vol_name: &str);
    /// GetMediaRecord(VolumeName) + UpdateMediaRecord when slot, InChanger
    /// or StorageId differ: Err(None) when not found, Err(Some(e)) on a
    /// failed update.
    fn update_media(&mut self, vol_name: &str, slot: u16) -> Result<(), Option<String>>;
}

/// The Director catalog through catreq.
struct CatalogMoves<'a> {
    db: &'a dyn crate::commands::bvfs::Db,
    storage_id: i64,
}

impl MoveCatalog for CatalogMoves<'_> {
    fn set_storage(&mut self, ctx: &crate::command_executor::ExecContext, storage: &str) {
        self.storage_id = crate::ua_select::get_storage_record(ctx, storage).unwrap_or(0);
    }

    fn make_inchanger_unique(&mut self, slot: u16, vol_name: &str) {
        let (mut errmsg, mut msgs) = (String::new(), Vec::new());
        let mut cat = crate::catreq::Cat::new(self.db, &mut errmsg, &mut msgs);
        let mr = crate::catreq::MediaRecord { slot: i32::from(slot), in_changer: 1, volume_name: vol_name.to_string(),
            storage_id: self.storage_id, ..Default::default() };
        crate::catreq::make_inchanger_unique(&mut cat, &mr);
    }

    fn update_media(&mut self, vol_name: &str, slot: u16) -> Result<(), Option<String>> {
        let (mut errmsg, mut msgs) = (String::new(), Vec::new());
        let mut cat = crate::catreq::Cat::new(self.db, &mut errmsg, &mut msgs);
        let mut mr = crate::catreq::MediaRecord { volume_name: vol_name.to_string(), ..Default::default() };
        if !crate::catreq::get_media_record(&mut cat, &mut mr) {
            return Err(None);
        }
        if mr.slot != i32::from(slot) || mr.in_changer == 0 || mr.storage_id != self.storage_id {
            mr.slot = i32::from(slot);
            mr.in_changer = 1;
            mr.storage_id = self.storage_id;
            if !crate::catreq::update_media_record(&mut cat, &mr) {
                drop(cat);
                return Err(Some(errmsg));
            }
        }
        Ok(())
    }
}

/// UpdateInchangerForExport(): InChanger 0 for the selected normal slots.
fn update_inchanger_for_export(ua: &mut Ua, ast: &crate::command_ast::CommandAst, cat: &mut dyn MoveCatalog,
    list: &[VolListEntry], slots: &[bool]) {
    if !ua.open_client_db(ast, false) {
        return;
    }
    for vl in list.iter().filter(|vl| vl.slot_type == 'S' && is_set(slots, vl.slot)) {
        cat.make_inchanger_unique(vl.slot, vl.vol_name.as_deref().unwrap_or(""));
    }
}

/// UpdateSlotsFromVolList(): the catalog records of the visited slots.
fn update_slots_from_vol_list(ua: &mut Ua, ast: &crate::command_ast::CommandAst, cat: &mut dyn MoveCatalog,
    list: &[VolListEntry], slots: &[bool]) {
    if !ua.open_client_db(ast, false) {
        return;
    }
    for vl in list.iter().filter(|vl| vl.slot_type == 'S' && is_set(slots, vl.slot)) {
        cat.make_inchanger_unique(vl.slot, vl.vol_name.as_deref().unwrap_or(""));
        if vl.content != 'F' {
            continue;
        }
        let Some(name) = vl.vol_name.as_deref() else { continue };
        match cat.update_media(name, vl.slot) {
            Ok(()) => {}
            Err(Some(error)) => ua.error_msg(error),
            Err(None) => ua.warning_msg(format!(
                "Volume \"{name}\" not found in catalog. Slot={} InChanger set to zero.\n", vl.slot)),
        }
    }
}

/// move_volumes_in_autochanger(): the visited slots, None for nothing to do.
#[allow(clippy::too_many_arguments)]
fn move_volumes(ua: &mut Ua, ast: &crate::command_ast::CommandAst, sd: &mut impl SdLink, cat: &mut dyn MoveCatalog,
    op: MoveOp, store: &StorageResource, list: &mut [VolListEntry], src: &[bool], dst: &[bool], max_slots: u16)
    -> Option<Slots> {
    if count_enabled_slots(src) == 0 || count_enabled_slots(dst) == 0 {
        ua.warning_msg("Nothing to do\n");
        return None;
    }
    if op == MoveOp::Export {
        update_inchanger_for_export(ua, ast, cat, list, src);
    }
    let mut visited = vec![false; usize::from(max_slots)];
    let mut to: u16 = 1;
    for from in 1..=max_slots {
        if !is_set(src, from) {
            continue;
        }
        while to <= max_slots && !is_set(dst, to) {
            to += 1;
        }
        if to > max_slots {
            ua.warning_msg("Failed to find suitable destination slot in slot range.\n");
            break;
        }
        if op == MoveOp::Export {
            if let Some(drive) = storage::vol_is_loaded_in_drive(list, from).map(|d| d.slot) {
                if !release_loaded_volume(ua, sd, store, drive, list) {
                    ua.warning_msg(format!("Failed to release volume in drive {}\n", drive as i16));
                    // `continue` skips the transfer_to++ of this source.
                    continue;
                }
            }
        }
        if transfer_volume(ua, sd, store, from, to) {
            update_internal_slot_list(list, from, to);
            set(&mut visited, from, true);
            set(&mut visited, to, true);
        } else {
            ua.warning_msg(format!("Failed to move volume from source slot {} to destination slot {}\n",
                from as i16, to as i16));
            if op == MoveOp::Export {
                set(&mut visited, from, true);
            }
        }
        to += 1;
    }
    Some(visited)
}

/// FindArgWithValue() for the first of `keys` that is given.
fn arg_with_value<'a>(args: &'a [Argument], keys: &[&str]) -> Option<(usize, &'a str)> {
    keys.iter().find_map(|key| {
        let i = args.iter().position(|a| a.key.eq_ignore_ascii_case(key))?;
        args[i].value.as_deref().map(|v| (i, v))
    })
}

/// PerformMoveOperation() after get_storage_resource(ua, false, true).
#[allow(clippy::too_many_arguments)]
pub fn perform_move_operation(ua: &mut Ua, ast: &crate::command_ast::CommandAst, env: &Env, sd: &mut impl SdLink,
    cat: &mut dyn MoveCatalog, store: &StorageResource, op: MoveOp) -> bool {
    let args = &ast.arguments;
    cat.set_storage(ua.ctx, &store.name);
    let scan = args.iter().any(|a| a.key.eq_ignore_ascii_case("scan"));
    let mut drive: i16 = -1;
    if scan {
        drive = storage::get_storage_drive(ua, args, env, sd, store).unwrap_or(-1);
    }
    let max_slots = storage::get_num_slots(ua, sd, store) as u16;
    if max_slots == 0 {
        ua.warning_msg("No slots in changer.\n");
        return false;
    }
    let Some(mut list) = storage::get_vol_list_from_storage(ua, sd, store, true, true, true) else {
        ua.warning_msg("No Volumes found, or no barcodes.\n");
        return false;
    };
    let (mut nr_src, mut nr_dst) = (0u16, 0u16);
    let mut src: Option<Slots> = None;
    if arg_with_value(args, &["srcslots", "srcslot"]).is_some() {
        let Some(mut slots) = storage::get_user_slot_list(args, "srcslots", i32::from(max_slots)) else { return false };
        if scan {
            let Some(new_list) = scan_slots_for_volnames(ua, sd, store, drive, list, Some(&slots)) else { return false };
            list = new_list;
        }
        validate_slot_list(ua, &list, &mut slots, true);
        nr_src = count_enabled_slots(&slots);
        src = Some(slots);
    } else if scan {
        let Some(new_list) = scan_slots_for_volnames(ua, sd, store, drive, list, None) else { return false };
        list = new_list;
    }
    let mut dst: Option<Slots> = None;
    if arg_with_value(args, &["dstslots", "dstslot"]).is_some() {
        let Some(mut slots) = storage::get_user_slot_list(args, "dstslots", i32::from(max_slots)) else { return false };
        if op != MoveOp::Move {
            validate_slot_list(ua, &list, &mut slots, false);
        }
        nr_dst = count_enabled_slots(&slots);
        dst = Some(slots);
    }
    if op != MoveOp::Move {
        // FindArgWithValue(ua, "volume") > 0
        if let Some((first, _)) = arg_with_value(args, &["volume"]) {
            let mut selected = vec![false; usize::from(max_slots)];
            nr_src = slots_using_volnames(ua, args, first, &list, src.as_ref(), &mut selected);
            src = Some(selected);
        }
    }
    let empty = vec![false; usize::from(max_slots)];
    if nr_src != 0 && nr_dst != 0 && nr_src > nr_dst {
        ua.warning_msg("Source slot selection doesn't fit into destination slot selection.\n");
        return false;
    }
    if nr_src != 0 && nr_dst != 0
        && slot_lists_overlap(src.as_deref().unwrap_or(&empty), dst.as_deref().unwrap_or(&empty)) {
        ua.warning_msg("Source slot selection and destination slot selection overlap.\n");
        return false;
    }
    match op {
        MoveOp::Export if nr_src == 0 => {
            ua.warning_msg("Cannot perform an export operation without source slot selection\n");
            return false;
        }
        MoveOp::Move if nr_src == 0 || nr_dst == 0 => {
            ua.warning_msg("Cannot perform a move operation without source and/or destination selection\n");
            return false;
        }
        _ => {}
    }
    let mut src = src.unwrap_or_else(|| empty.clone());
    let mut dst = dst.unwrap_or_else(|| empty.clone());
    let visited = match op {
        MoveOp::Import => {
            if nr_src == 0 {
                src = empty.clone();
                nr_src = auto_fill(&list, &mut src, 'I', 'F');
            } else if !verify_slot_list(&list, &src, 'I', 'F') {
                ua.warning_msg("Not all slots in source selection are import slots and filled.\n");
                return false;
            }
            if nr_dst == 0 {
                dst = empty.clone();
                nr_dst = auto_fill(&list, &mut dst, 'S', 'E');
                if nr_src > nr_dst {
                    ua.warning_msg(format!("Not enough free slots available to import {} volumes\n", nr_src as i16));
                    return false;
                }
            } else if !verify_slot_list(&list, &dst, 'S', 'E') {
                ua.warning_msg("Not all slots in destination selection are normal slots and empty.\n");
                return false;
            }
            move_volumes(ua, ast, sd, cat, op, store, &mut list, &src, &dst, max_slots)
        }
        MoveOp::Export => {
            if !verify_slot_list(&list, &src, 'S', 'F') {
                ua.warning_msg("Not all slots in source selection are normal slots and filled.\n");
                return false;
            }
            if nr_dst == 0 {
                dst = empty.clone();
                nr_dst = auto_fill(&list, &mut dst, 'I', 'E');
                if nr_src > nr_dst {
                    ua.warning_msg(format!("Not enough free export slots available to export {} volume{}\n",
                        nr_src as i16, if nr_src > 1 { "s" } else { "" }));
                    return false;
                }
            } else if !verify_slot_list(&list, &dst, 'I', 'E') {
                ua.warning_msg("Not all slots in destination selection are export slots and empty.\n");
                return false;
            }
            move_volumes(ua, ast, sd, cat, op, store, &mut list, &src, &dst, max_slots)
        }
        MoveOp::Move => move_volumes(ua, ast, sd, cat, op, store, &mut list, &src, &dst, max_slots),
    };
    if let Some(visited) = visited.filter(|v| count_enabled_slots(v) > 0) {
        update_slots_from_vol_list(ua, ast, cat, &list, &visited);
    }
    true
}

/// ImportCmd/ExportCmd/move_cmd: get_storage_resource(ua, false, true),
/// then PerformMoveOperation with `sd` and `cat`.
pub fn move_command(ua: &mut Ua, ast: &crate::command_ast::CommandAst, op: MoveOp, sd: &mut impl SdLink,
    cat: &mut dyn MoveCatalog) -> bool {
    let Some(store) = crate::ua_select::get_storage_resource(ua, ast, false, true) else { return false };
    let Some(env) = Env::load(ua.ctx) else { return false };
    perform_move_operation(ua, ast, &env, sd, cat, &store, op)
}

/// The real Storage daemon and the Director catalog.
pub fn move_cmd(ua: &mut Ua, ast: &crate::command_ast::CommandAst, op: MoveOp) -> bool {
    let ctx = ua.ctx;
    let director = ctx.config.as_ref().and_then(|c| c.read().ok()).map(|c| c.director.name.clone()).unwrap_or_default();
    let mut sd = storage::NetSd { director };
    match ctx.catalog.clone() {
        Some(db) => move_command(ua, ast, op, &mut sd, &mut CatalogMoves { db: &db, storage_id: 0 }),
        None => move_command(ua, ast, op, &mut sd, &mut NoCatalog),
    }
}

/// Without a catalog nothing is recorded.
pub struct NoCatalog;

impl MoveCatalog for NoCatalog {
    fn make_inchanger_unique(&mut self, _slot: u16, _vol_name: &str) {}
    fn update_media(&mut self, _vol_name: &str, _slot: u16) -> Result<(), Option<String>> {
        Err(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_executor::{ApiMode, ExecContext};
    use std::sync::{Arc, RwLock};

    /// Fake SD: answers by command prefix and records every command.
    struct ChangerSd { sent: Vec<String>, answer: fn(&str) -> Vec<String> }

    impl SdLink for ChangerSd {
        fn exchange(&mut self, _store: &StorageResource, command: &str) -> Result<Vec<String>, String> {
            self.sent.push(command.to_string());
            Ok((self.answer)(command).into_iter().map(|m| format!("{m}\n")).collect())
        }
    }

    #[derive(Default)]
    struct Recorded { calls: Vec<String> }

    impl MoveCatalog for Recorded {
        fn make_inchanger_unique(&mut self, slot: u16, vol_name: &str) {
            self.calls.push(format!("unique {slot} {vol_name}"));
        }
        fn update_media(&mut self, vol_name: &str, slot: u16) -> Result<(), Option<String>> {
            self.calls.push(format!("media {vol_name} {slot}"));
            Ok(())
        }
    }

    fn context(storage: &str, autochanger: bool) -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(&format!(
            "Director {{ Name = dir }}\nCatalog {{ Name = MyCatalog; DB Name = bareos }}\n\
             Storage {{ Name = {storage}; Address = sd.example; Port = 9103; Device = Dev; Media Type = Tape; \
             Auto Changer = {} }}", if autochanger { "yes" } else { "no" })).unwrap();
        ExecContext { session_id: 0, config: Some(Arc::new(RwLock::new(config))), session_mgr: None,
            catalog: None, api_mode: ApiMode::Human, bus_rx: None, job_manager: None }
    }

    /// Drive 0 holds slot 1 (vol1), slot 2 vol2, slot 3 empty, import
    /// slot 4 vol4, import slot 5 empty.
    fn changer(cmd: &str) -> Vec<String> {
        match cmd {
            c if c.starts_with("autochanger slots") => vec!["slots=5".into()],
            c if c.starts_with("autochanger listall") => ["D:0:F:1:vol1", "S:1:E", "S:2:F:vol2", "S:3:E", "I:4:F:vol4", "I:5:E"]
                .iter().map(|s| s.to_string()).collect(),
            c if c.starts_with("autochanger transfer") => vec!["3308 Transfer done".into()],
            c if c.starts_with("release") => vec!["3022 Device \"Dev\" released.".into()],
            _ => vec!["3900 unexpected".into()],
        }
    }

    fn run(storage: &str, line: &str, answer: fn(&str) -> Vec<String>, autochanger: bool)
        -> (bool, String, Vec<String>, Vec<String>) {
        let ctx = context(storage, autochanger);
        let ast = crate::session::parse_cli_command(&format!("{line} storage={storage}"));
        let op = match ast.command_name.as_str() {
            "import" => MoveOp::Import,
            "export" => MoveOp::Export,
            _ => MoveOp::Move,
        };
        let mut sd = ChangerSd { sent: vec![], answer };
        let mut cat = Recorded::default();
        let mut ua = Ua::new(&ctx);
        let ok = move_command(&mut ua, &ast, op, &mut sd, &mut cat);
        let result = ua.finish(ok);
        let output = format!("{}{}", result.text_output, if result.errors_inline { "" } else { &result.error_message });
        (ok, output, sd.sent, cat.calls)
    }

    fn connect(storage: &str) -> String {
        format!("Connecting to Storage daemon {storage} at sd.example:9103 ...\n")
    }

    #[test]
    fn plain_device_stops_after_the_slot_query() {
        for (storage, line) in [("PlainMove", "move"), ("PlainExport", "export"), ("PlainImport", "import srcslots=1")] {
            let (ok, output, sent, calls) = run(storage, line,
                |_| vec!["3998 Device \"Dev\" is not an autochanger.".into()], false);
            assert!(!ok);
            assert_eq!(sent, ["autochanger slots Dev\n"], "{line}");
            assert_eq!(output, format!("{}3998 Device \"Dev\" is not an autochanger.\nDevice \"Dev\" has 0 slots.\n\
                No slots in changer.\n", connect(storage)), "{line}");
            assert!(calls.is_empty());
        }
    }

    #[test]
    fn move_transfers_and_updates_the_visited_slots() {
        let (ok, output, sent, calls) = run("ChangerMove", "move srcslots=2 dstslots=3", changer, true);
        assert!(ok, "{output}");
        assert_eq!(sent, ["autochanger slots Dev\n", "autochanger listall Dev \n", "autochanger transfer Dev 2 3 \n"]);
        assert!(output.contains("3308 Transfer done\n"), "{output}");
        assert_eq!(calls, ["unique 2 ", "unique 3 vol2", "media vol2 3"]);
    }

    #[test]
    fn move_selection_checks() {
        let (ok, output, sent, _) = run("ChangerOverlap", "move srcslots=2 dstslots=2", changer, true);
        assert!(!ok && sent.len() == 2);
        assert!(output.ends_with("Source slot selection and destination slot selection overlap.\n"), "{output}");
        let (_, output, _, _) = run("ChangerFit", "move srcslots=1-2 dstslots=3", changer, true);
        assert!(output.ends_with("Source slot selection doesn't fit into destination slot selection.\n"), "{output}");
        let (_, output, sent, _) = run("ChangerNoDst", "move srcslots=2", changer, true);
        assert_eq!(sent.len(), 2);
        assert!(output.ends_with("Cannot perform a move operation without source and/or destination selection\n"), "{output}");
        // Slot 3 is empty: deselected.
        let (_, output, _, _) = run("ChangerEmptySrc", "move srcslots=3 dstslots=5", changer, true);
        assert!(output.contains("Deselecting slot 3 doesn't have wanted status.\n"), "{output}");
        assert!(output.ends_with("Cannot perform a move operation without source and/or destination selection\n"), "{output}");
    }

    #[test]
    fn import_fills_free_normal_slots() {
        // Slot 1 is empty but its volume is in the drive: slot 3 is used.
        let (ok, _, sent, calls) = run("ChangerImport", "import", changer, true);
        assert!(ok);
        assert_eq!(sent.last().unwrap(), "autochanger transfer Dev 4 3 \n");
        assert_eq!(calls, ["unique 3 vol4", "media vol4 3"]);
    }

    #[test]
    fn export_needs_a_free_export_slot_and_releases_the_drive() {
        let (ok, output, _, _) = run("ChangerExportFull", "export srcslots=1-2", changer, true);
        assert!(!ok);
        assert!(output.ends_with("Not enough free export slots available to export 2 volumes\n"), "{output}");
        let (ok, output, sent, calls) = run("ChangerExport", "export volume=vol1", changer, true);
        assert!(ok, "{output}");
        assert_eq!(&sent[2..], ["release Dev drive=0\n", "autochanger transfer Dev 1 5 \n"]);
        // InChanger is cleared first, then the visited normal slot 1.
        assert_eq!(calls, ["unique 1 ", "unique 1 "]);
        let (_, output, _, _) = run("ChangerExportNone", "export volume=nope|", changer, true);
        assert!(output.contains("No volume named nope in changer or in selected source slots.\n"), "{output}");
        assert!(output.ends_with("Cannot perform an export operation without source slot selection\n"), "{output}");
    }

    #[test]
    fn failed_transfer_is_reported() {
        fn refuse(cmd: &str) -> Vec<String> {
            if cmd.starts_with("autochanger transfer") {
                vec!["3992 Bad autochanger \"transfer\" command".into()]
            } else {
                changer(cmd)
            }
        }
        let (ok, output, _, calls) = run("ChangerRefuse", "move srcslots=2 dstslots=3", refuse, true);
        assert!(ok);
        assert!(output.ends_with("3992 Bad autochanger \"transfer\" command\n\
            Failed to move volume from source slot 2 to destination slot 3\n"), "{output}");
        assert!(calls.is_empty());
    }
}
