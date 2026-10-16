//! Bootstrap of a console restore (compatibility log C-047): port of
//! dird/bsr.cc (AddFindex, AddFindexAll, AddVolumeInformationToBsr,
//! WriteBsrFile, DisplayBsrInfo) and FindStorageResource (ua_restore.cc).

use std::sync::atomic::{AtomicU32, Ordering};

use crate::command_ast::CommandAst;
use crate::ua_select::Ua;

/// One JobMedia record of a job (cats VolumeParameters).
#[derive(Debug, Clone, Default)]
pub(crate) struct VolParam {
    volume_name: String,
    media_type: String,
    storage: String,
    first_index: u32,
    last_index: u32,
    start_addr: u64,
    end_addr: u64,
    slot: i32,
    in_changer: i32,
}

/// RestoreBootstrapRecord: the FileIndexes of one JobId.
#[derive(Debug, Default)]
struct Record {
    job_id: u32,
    file_ids: Vec<i32>,
    all_files: bool,
    fileregex: Option<String>,
    vol_session_id: u32,
    vol_session_time: u32,
    vols: Vec<VolParam>,
}

impl Record {
    fn empty(&self) -> bool {
        !self.all_files && self.file_ids.is_empty()
    }

    /// RestoreBootstrapRecordFileIndex::GetRanges()
    fn ranges(&mut self) -> Vec<(i32, i32)> {
        if self.all_files {
            return vec![(1, i32::MAX)];
        }
        if !self.file_ids.is_sorted() {
            self.file_ids.sort_unstable();
        }
        let mut ranges = Vec::new();
        let (mut begin, mut end) = (0i32, 0i32);
        for &id in &self.file_ids {
            if begin == 0 {
                begin = id;
                end = id;
            } else if id > end + 1 {
                ranges.push((begin, end));
                begin = id;
                end = id;
            } else {
                end = id;
            }
        }
        ranges.push((begin, end));
        ranges
    }
}

/// The chain of bootstrap records (rx.bsr); the head always exists.
#[derive(Debug)]
pub(crate) struct Bsr {
    records: Vec<Record>,
}

impl Default for Bsr {
    fn default() -> Self {
        Self { records: vec![Record::default()] }
    }
}

static UNIQ: AtomicU32 = AtomicU32::new(0);

impl Bsr {
    /// JobId of the head record (`rx.bsr->JobId`).
    pub fn job_id(&self) -> u32 {
        self.records[0].job_id
    }

    /// CheckAndSetFileregex(): the head record's fileregex.
    pub fn set_fileregex(&mut self, regex: &str) {
        self.records[0].fileregex = Some(regex.to_string());
    }

    /// The record of `job_id` as AddFindex/AddFindexAll find or append it.
    fn record(&mut self, job_id: u32) -> &mut Record {
        if self.records[0].empty() {
            self.records[0].job_id = job_id;
        }
        if self.records[0].job_id == job_id {
            return &mut self.records[0];
        }
        match self.records.iter().skip(1).position(|r| r.job_id == job_id) {
            Some(i) => &mut self.records[i + 1],
            None => {
                self.records.push(Record { job_id, ..Default::default() });
                self.records.last_mut().unwrap()
            }
        }
    }

    /// AddFindex()
    pub fn add_findex(&mut self, job_id: u32, findex: i32) {
        if findex == 0 {
            return; // probably a dummy directory
        }
        self.record(job_id).file_ids.push(findex);
    }

    /// AddFindexAll(): a new record inherits the head's fileregex.
    pub fn add_findex_all(&mut self, job_id: u32) {
        let regex = self.records[0].fileregex.clone();
        let new = !self.records[0].empty() && self.records[0].job_id != job_id
            && !self.records.iter().skip(1).any(|r| r.job_id == job_id);
        let record = self.record(job_id);
        if new {
            record.fileregex = regex;
        }
        record.all_files = true;
    }

    /// AddVolumeInformationToBsr()
    pub fn add_volume_information(&mut self, ua: &mut Ua) -> bool {
        let Some(catalog) = ua.ctx.catalog.clone() else { return false };
        for record in &mut self.records {
            let sql = format!("SELECT VolSessionId,VolSessionTime,JobFiles FROM Job WHERE JobId={}", record.job_id);
            let row = match catalog.query_rows(&sql) {
                Ok(table) => table.rows.into_iter().next(),
                Err(e) => {
                    ua.error_msg(format!("Unable to get Job record. ERR={e}\n"));
                    return false;
                }
            };
            let Some(row) = row else {
                ua.error_msg(format!("Unable to get Job record. ERR=cats/sql_get.cc:275 No Job found for JobId {}\n", record.job_id));
                return false;
            };
            let value = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
            record.vol_session_id = to_u64(&value(0)) as u32;
            record.vol_session_time = to_u64(&value(1)) as u32;
            if to_u64(&value(2)) == 0 {
                // zero files is OK, not an error, but there are no volumes
                record.vols.clear();
                continue;
            }
            match job_volume_parameters(&catalog, record.job_id) {
                Ok(vols) => record.vols = vols,
                Err(e) => {
                    ua.error_msg(format!("Unable to get Job Volume Parameters. ERR={e}"));
                    return false;
                }
            }
        }
        true
    }

    /// WriteBsrFile(): the number of files in the bootstrap (0: nothing
    /// written or an error) and the bootstrap file name.
    pub fn write_file(&mut self, ua: &mut Ua, ast: &CommandAst, job_ids: &str, store: &mut Option<String>)
        -> (u32, String) {
        // MakeUniqueRestoreFilename()
        let fname = match crate::ua_select::find_arg_with_value(ast, "bootstrap") {
            Some(name) => name.to_string(),
            None => {
                let (dir, name) = ua.ctx.config.as_ref().and_then(|c| c.read().ok())
                    .map(|c| (c.director.working_directory.clone(), c.director.name.clone()))
                    .unwrap_or_default();
                let uniq = UNIQ.fetch_add(1, Ordering::SeqCst) + 1;
                format!("{dir}/{name}.restore.{uniq}.bsr")
            }
        };
        let mut buffer = String::new();
        let count = self.write(ua, ast, job_ids, store, &mut buffer);
        if let Err(e) = std::fs::write(&fname, &buffer) {
            if !std::path::Path::new(&fname).exists() {
                ua.error_msg(format!("Unable to create bootstrap file {fname}. ERR={}\n", crate::commands::restore::errno_text(&e)));
                return (0, fname);
            }
            if count != 0 {
                ua.error_msg("Error writing bsr file.\n");
                return (0, fname);
            }
        }
        if count == 0 {
            ua.info_msg("No files found to read. No bootstrap file written.\n");
            return (0, fname);
        }
        ua.send_msg(format!("Bootstrap records written to {fname}\n"));
        (count, fname)
    }

    /// WriteBsr(): records in the order of the JobId list.
    fn write(&mut self, ua: &mut Ua, ast: &CommandAst, job_ids: &str, store: &mut Option<String>,
        buffer: &mut String) -> u32 {
        let mut first = true;
        let mut last_index = 0u32;
        let mut total = 0u32;
        let mut last_job: Option<(u32, u32)> = None;
        let order: Vec<usize> = if job_ids.is_empty() {
            (0..self.records.len()).collect()
        } else {
            let mut order = Vec::new();
            for job_id in crate::commands::restore::jobid_list(job_ids) {
                order.extend(self.records.iter().enumerate().filter(|(_, r)| r.job_id == job_id).map(|(i, _)| i));
            }
            order
        };
        for i in order {
            let record = &mut self.records[i];
            let this_job = (record.vol_session_id, record.vol_session_time);
            if last_job != Some(this_job) {
                // cannot compare indices between jobs
                last_index = 0;
            }
            total = total.wrapping_add(write_bsr_item(record, ua, ast, store, buffer, &mut first, &mut last_index));
            last_job = Some(this_job);
        }
        total
    }

    /// DisplayBsrInfo(): the volumes needed.
    pub fn display_info(&self, ua: &mut Ua, job_ids: &str) {
        ua.send_msg("\n");
        ua.send_msg("The job will require the following\n   \
            Volume(s)                 Storage(s)                SD Device(s)\n\
            ===========================================================================\n");
        // Unique list of volumes (StartPrompt with an empty header)
        let mut prompts = vec![String::new()];
        let mut add = |ua: &Ua, record: &Record| {
            for vol in record.vols.iter().filter(|v| !v.volume_name.is_empty()) {
                let device = storage_device(ua, &vol.storage).unwrap_or_default();
                let online = if vol.in_changer != 0 && vol.slot != 0 { '*' } else { ' ' };
                let line = format!("{online}{:<25} {:<25} {device:<25}", vol.volume_name, vol.storage);
                if !prompts.iter().skip(1).any(|p| *p == line) {
                    prompts.push(line);
                }
            }
        };
        if job_ids.is_empty() {
            for record in &self.records {
                add(ua, record);
            }
        } else {
            for job_id in crate::commands::restore::jobid_list(job_ids) {
                for record in self.records.iter().filter(|r| r.job_id == job_id) {
                    add(ua, record);
                }
            }
        }
        for prompt in &prompts {
            ua.send_msg(format!("   {prompt}\n"));
        }
        // num_prompts counts the empty header too
        if prompts.is_empty() {
            ua.send_msg("No Volumes found to restore.\n");
        } else {
            ua.send_msg("\nVolumes marked with \"*\" are online.\n");
        }
        ua.send_msg("\n");
    }
}

fn to_u64(s: &str) -> u64 {
    s.trim().parse::<u64>().unwrap_or_else(|_| s.trim().parse::<i64>().map(|v| v as u64).unwrap_or(0))
}

/// BareosDb::GetJobVolumeParameters()
fn job_volume_parameters(catalog: &crate::catalog_worker::CatalogHandle, job_id: u32) -> Result<Vec<VolParam>, String> {
    let table = catalog.query_rows(&format!(
        "SELECT VolumeName,MediaType,FirstIndex,LastIndex,StartFile,JobMedia.EndFile,StartBlock,JobMedia.EndBlock,\
         Slot,StorageId,InChanger,JobBytes FROM JobMedia,Media WHERE JobMedia.JobId={job_id} \
         AND JobMedia.MediaId=Media.MediaId ORDER BY VolIndex,JobMediaId"))?;
    if table.rows.is_empty() {
        return Err(format!("cats/sql_get.cc:414 No volumes found for JobId={job_id}\n"));
    }
    let mut vols = Vec::new();
    for row in &table.rows {
        let value = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        let n = |i: usize| to_u64(&value(i));
        let storage_id = n(9);
        let storage = if storage_id != 0 {
            catalog.query_rows(&format!("SELECT Name from Storage WHERE StorageId={storage_id}")).ok()
                .and_then(|t| t.rows.into_iter().next())
                .and_then(|r| r.into_iter().next().flatten())
                .unwrap_or_default()
        } else {
            String::new()
        };
        vols.push(VolParam {
            volume_name: value(0),
            media_type: value(1),
            storage,
            first_index: n(2) as u32,
            last_index: n(3) as u32,
            start_addr: (n(4) as u32 as u64) << 32 | n(6) as u32 as u64,
            end_addr: (n(5) as u32 as u64) << 32 | n(7) as u32 as u64,
            slot: n(8) as i32,
            in_changer: n(10) as i32,
        });
    }
    Ok(vols)
}

/// GetStorageDevice(): the first Device of the Storage resource.
fn storage_device(ua: &Ua, storage: &str) -> Option<String> {
    if storage.is_empty() {
        return None;
    }
    let config = ua.ctx.config.as_ref()?.read().ok()?;
    let store = config.storages.iter().find(|s| s.name == storage)?;
    (!store.device.is_empty()).then(|| store.device.clone())
}

/// is_volume_selected()
fn is_volume_selected(ranges: &[(i32, i32)], first: u32, last: u32) -> bool {
    ranges.iter().any(|&(begin, end)| begin as i64 <= last as i32 as i64 && end as i64 >= first as i32 as i64)
}

/// write_findex(): FileIndex lines of one volume, constrained to its range.
fn write_findex(ranges: &[(i32, i32)], first_index: &mut u32, last_index: &mut u32, buffer: &mut String) -> u32 {
    let mut count = 0u32;
    let mut min_max: Option<(u32, u32)> = None;
    for &(begin, end) in ranges {
        let first = (begin as u32).max(*first_index);
        let last = (end as u32).min(*last_index);
        if first <= last {
            match &mut min_max {
                None => min_max = Some((first, last)),
                Some((_, max)) => *max = last,
            }
            if first == last {
                buffer.push_str(&format!("FileIndex={first}\n"));
                count += 1;
            } else {
                buffer.push_str(&format!("FileIndex={first}-{last}\n"));
                count += last - first + 1;
            }
        }
    }
    if let Some((min, max)) = min_max {
        *first_index = min;
        *last_index = max;
    }
    count
}

/// write_bsr_item()
fn write_bsr_item(record: &mut Record, ua: &mut Ua, ast: &CommandAst, store: &mut Option<String>,
    buffer: &mut String, first: &mut bool, last_index: &mut u32) -> u32 {
    let ranges = record.ranges();
    let mut total = 0u32;
    for i in 0..record.vols.len() {
        let vol = &record.vols[i];
        if !is_volume_selected(&ranges, vol.first_index, vol.last_index) {
            record.vols[i].volume_name.clear(); // zap VolumeName
            continue;
        }
        if store.is_none() {
            *store = find_storage_resource(ua, ast, &vol.storage, &vol.media_type);
        }
        let vol = &record.vols[i];
        buffer.push_str(&format!("Storage=\"{}\"\nVolume=\"{}\"\nMediaType=\"{}\"\n",
            vol.storage, vol.volume_name, vol.media_type));
        if let Some(regex) = &record.fileregex {
            buffer.push_str(&format!("FileRegex={regex}\n"));
        }
        if let Some(device) = storage_device(ua, &vol.storage) {
            buffer.push_str(&format!("Device=\"{device}\"\n"));
        }
        if vol.slot > 0 {
            buffer.push_str(&format!("Slot={}\n", vol.slot));
        }
        buffer.push_str(&format!("VolSessionId={}\nVolSessionTime={}\nVolAddr={}-{}\n",
            record.vol_session_id, record.vol_session_time, vol.start_addr, vol.end_addr));
        let (mut start, mut end) = (vol.first_index, vol.last_index);
        let count = write_findex(&ranges, &mut start, &mut end, buffer);
        if count > 0 {
            buffer.push_str(&format!("Count={count}\n"));
        }
        total = total.wrapping_add(count);
        // A file continued on the next volume is counted once.
        if !*first && count > 0 && *last_index == start {
            total = total.wrapping_sub(1);
        }
        *first = false;
        *last_index = end;
    }
    total
}

/// FindStorageResource() (ua_restore.cc): the Storage resource of the
/// volumes, overridable by `storage=`, else by MediaType, else asked for.
fn find_storage_resource(ua: &mut Ua, ast: &CommandAst, storage: &str, media_type: &str) -> Option<String> {
    let ctx = ua.ctx;
    let storages: Vec<(String, String)> = ctx.config.as_ref().and_then(|c| c.read().ok())
        .map(|c| c.storages.iter().map(|s| (s.name.clone(), s.media_type.clone())).collect())
        .unwrap_or_default();
    // Try looking up Storage by name
    let mut found = None;
    if let Some((name, _)) = storages.iter().find(|(name, _)| name == storage) {
        if crate::acl::access(ctx, crate::acl::Kind::Storage, name) {
            found = Some(name.clone());
        }
    }
    if let Some(store) = found {
        // Check if an explicit storage resource is given
        let explicit = ast.arguments.iter().position(|a| a.key.eq_ignore_ascii_case("storage") && a.value.is_some())
            .and_then(|i| ast.arguments[i].value.clone())
            .filter(|name| storages.iter().any(|(n, _)| n == name)
                && crate::acl::access(ctx, crate::acl::Kind::Storage, name));
        if let Some(name) = explicit.filter(|name| *name != store) {
            ua.info_msg(format!("Warning default storage overridden by \"{name}\" on command line.\n"));
            return Some(name);
        }
        return Some(store);
    }
    // If no storage resource, try to find one from MediaType
    if let Some((name, _)) = storages.iter().find(|(_, mt)| mt == media_type) {
        if crate::acl::access(ctx, crate::acl::Kind::Storage, name) {
            ua.warning_msg(format!("Storage \"{storage}\" not found, using Storage \"{name}\" from MediaType \"{media_type}\".\n"));
            return Some(name.clone());
        }
        // the C++ code returns here without a storage
        return None;
    }
    ua.warning_msg(format!("\nUnable to find Storage resource for\nMediaType \"{media_type}\", needed by the Jobs you selected.\n"));
    // Take command line arg, or ask user if none
    crate::ua_select::get_storage_resource(ua, ast, false, false).map(|s| s.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn findex_records_and_ranges() {
        let mut bsr = Bsr::default();
        bsr.add_findex(1, 0);
        assert_eq!(bsr.job_id(), 0);
        bsr.add_findex(1, 5);
        bsr.add_findex(5, 2);
        bsr.add_findex(1, 3);
        bsr.add_findex(1, 4);
        bsr.add_findex(1, 9);
        assert_eq!(bsr.job_id(), 1);
        assert_eq!(bsr.records.len(), 2);
        assert_eq!(bsr.records[0].ranges(), [(3, 5), (9, 9)]);
        let mut buffer = String::new();
        let (mut first, mut last) = (4, 100);
        assert_eq!(write_findex(&bsr.records[0].ranges(), &mut first, &mut last, &mut buffer), 3);
        assert_eq!(buffer, "FileIndex=4-5\nFileIndex=9\n");
        assert_eq!((first, last), (4, 9));
        bsr.add_findex_all(7);
        assert_eq!(bsr.records[2].ranges(), [(1, i32::MAX)]);
        assert!(is_volume_selected(&[(3, 5)], 5, 10) && !is_volume_selected(&[(3, 5)], 6, 10));
    }
}
