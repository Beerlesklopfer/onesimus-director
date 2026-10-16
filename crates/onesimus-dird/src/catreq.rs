//! Catalog services for the Storage daemon during a job.
//!
//! Port of dird/catreq.cc (CatalogRequest, CatalogUpdate/UpdateAttribute,
//! DespoolAttributesFromFile) and the dispatch of dird/getmsg.cc
//! BgetDirmsg() for `C`atalog requests, `U`pdCat attribute records and
//! `B`lastAttr spool files, together with what they call:
//! dird/next_vol.cc (FindNextVolumeForAppend, HasVolumeExpired,
//! CheckIfVolumeValidOrRecyclable, GetScratchVolume), dird/recycle.cc,
//! dird/newvol.cc and the catalog functions of cats/ (GetMediaRecord,
//! GetPoolRecord, FindNextVolume, UpdateMediaRecord, UpdatePoolRecord,
//! CreateMediaRecord, CreateJobmediaRecord, UpdateRunningJobRecord,
//! DeleteNullJobmediaRecords, the PostgreSQL batch file insert).
//! Messages are parsed with a port of lib/scan.cc bsscanf(), which is what
//! `sscanf` means in the C++ Director. Compatibility log C-055.
//!
//! Not ported (the C++ paths need the console prune/purge machinery):
//! PruneVolumes() (next_vol.cc step 5), purging/recycling the oldest volume
//! (PurgeOldestVolume/RecycleOldestVolume, steps 8/9), pruning the current
//! volume (RecycleCurrentVolume) and LabelFormat variable substitution.

use crate::catalog_worker::{CatalogHandle, SqlTable};
use crate::commands::bvfs::Db;
use crate::message_bus::MsgType;

use crate::scan::is_space;
pub(crate) use crate::scan::{bsscanf, Scanned};

fn scanned_u(v: &[Scanned], i: usize) -> u64 {
    match v.get(i) { Some(Scanned::U(x)) => *x, Some(Scanned::D(x)) => *x as u64, _ => 0 }
}

fn scanned_d(v: &[Scanned], i: usize) -> i64 {
    match v.get(i) { Some(Scanned::D(x)) => *x, Some(Scanned::U(x)) => *x as i64, _ => 0 }
}

fn scanned_s(v: &[Scanned], i: usize) -> String {
    match v.get(i) { Some(Scanned::S(s)) => String::from_utf8_lossy(s).into_owned(), _ => String::new() }
}

// ---------------------------------------------------------------------------
// Small C helpers
// ---------------------------------------------------------------------------

/// lib/util.cc BashSpaces().
pub(crate) fn bash_spaces(s: &str) -> String {
    s.replace(' ', "\x01")
}

/// lib/util.cc UnbashSpaces().
pub(crate) fn unbash_spaces(s: &str) -> String {
    s.replace('\x01', " ")
}

/// C string at `start` (up to the first NUL or the end of the buffer).
fn cstr(buf: &[u8], start: usize) -> &[u8] {
    let rest = buf.get(start..).unwrap_or(&[]);
    &rest[..rest.iter().position(|c| *c == 0).unwrap_or(rest.len())]
}

fn byte(buf: &[u8], i: usize) -> u8 {
    buf.get(i).copied().unwrap_or(0)
}

fn skip_spaces(buf: &[u8], mut p: usize) -> usize {
    while byte(buf, p) != 0 && is_space(byte(buf, p)) {
        p += 1;
    }
    p
}

fn skip_nonspaces(buf: &[u8], mut p: usize) -> usize {
    while byte(buf, p) != 0 && !is_space(byte(buf, p)) {
        p += 1;
    }
    p
}

/// lib/edit.cc str_to_uint64().
fn str_to_uint64(s: &[u8]) -> u64 {
    let mut p = 0;
    while is_space(byte(s, p)) {
        p += 1;
    }
    if byte(s, p) == b'+' {
        p += 1;
    }
    let mut value: u64 = 0;
    while byte(s, p).is_ascii_digit() {
        value = value.wrapping_mul(10).wrapping_add((byte(s, p) - b'0') as u64);
        p += 1;
    }
    value
}

/// lib/edit.cc str_to_int64() (NULL gives 0).
fn str_to_int64(s: &[u8]) -> i64 {
    let mut p = 0;
    while is_space(byte(s, p)) {
        p += 1;
    }
    let negative = byte(s, p) == b'-';
    if matches!(byte(s, p), b'+' | b'-') {
        p += 1;
    }
    let value = str_to_uint64(&s[p.min(s.len())..]) as i64;
    if negative { value.wrapping_neg() } else { value }
}

/// C++ EscapeString (standard_conforming_strings on).
pub(crate) fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

fn now() -> i64 {
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// C++ bstrutime(): local time "%Y-%m-%d %H:%M:%S".
pub(crate) fn bstrutime(t: i64) -> String {
    crate::localtime::strftime("%Y-%m-%d %H:%M:%S", t as libc::time_t)
}

use crate::btime::str_to_utime;

/// lib/edit.cc edit_uint64_with_commas().
fn with_commas(v: u64) -> String {
    crate::sql_list::add_commas(&v.to_string())
}

/// lib/base64.cc BinToBase64(compatible = true).
fn bin_to_base64(bin: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let (mut reg, mut rem): (u32, i32) = (0, 0);
    let mut i = 0;
    while i < bin.len() {
        if rem < 6 {
            reg = (reg << 8) | bin[i] as u32;
            i += 1;
            rem += 8;
        }
        out.push(DIGITS[((reg >> (rem - 6)) & 0x3f) as usize] as char);
        rem -= 6;
    }
    if rem != 0 {
        let mask = (1u32 << rem) - 1;
        out.push(DIGITS[((reg & mask) << (6 - rem)) as usize] as char);
    }
    out
}

/// findlib/bfile.cc stream_to_ascii() for the digest streams.
fn stream_to_ascii(stream: i32) -> String {
    match stream {
        STREAM_MD5_DIGEST => "MD5 digest".into(),
        STREAM_SHA1_DIGEST => "SHA1 digest".into(),
        STREAM_SHA256_DIGEST => "SHA256 digest".into(),
        STREAM_SHA512_DIGEST => "SHA512 digest".into(),
        STREAM_XXH128_DIGEST => "XXH128 digest".into(),
        other => format!("{other}"),
    }
}

const STREAM_UNIX_ATTRIBUTES: i32 = 1;
const STREAM_MD5_DIGEST: i32 = 3;
const STREAM_UNIX_ATTRIBUTES_EX: i32 = 5;
const STREAM_SHA1_DIGEST: i32 = 10;
const STREAM_SHA256_DIGEST: i32 = 17;
const STREAM_SHA512_DIGEST: i32 = 18;
const STREAM_RESTORE_OBJECT: i32 = 28;
const STREAM_XXH128_DIGEST: i32 = 40;
const FT_REG: i32 = 3;
const FT_DELETED: i32 = 23;
const FT_BASE: i32 = 24;
const VOL_ENABLED: i32 = 1;
const BATCH_FLUSH: u32 = 800_000;
/// Rows sent to the server-side batch table in one INSERT.
const BATCH_CHUNK: usize = 1000;

// ---------------------------------------------------------------------------
// Catalog access with the C++ error bookkeeping (cats/sql.cc)
// ---------------------------------------------------------------------------

/// Job messages produced by a request (Jmsg type, text without header).
pub type Messages = Vec<(MsgType, String)>;

/// BareosDb with `errmsg` and the Jmsg side effects of QueryDb/UpdateDb/...
pub(crate) struct Cat<'a> {
    db: &'a dyn Db,
    pub(crate) errmsg: &'a mut String,
    msgs: &'a mut Messages,
}

impl<'a> Cat<'a> {
    /// A catalog session of a console command (ua->db): `errmsg` is what
    /// strerror() returns; the Jmsg side effects end up in `msgs`.
    pub(crate) fn new(db: &'a dyn Db, errmsg: &'a mut String, msgs: &'a mut Messages) -> Self {
        Cat { db, errmsg, msgs }
    }

    fn jmsg(&mut self, t: MsgType, text: String) {
        self.msgs.push((t, text));
    }

    fn sql_error(error: &str) -> String {
        error.to_string()
    }

    /// QueryDb(): rows or None (errmsg set, M_FATAL).
    pub(crate) fn query_db(&mut self, sql: &str, loc: &str) -> Option<SqlTable> {
        match self.db.rows(sql) {
            Ok(t) => Some(t),
            Err(e) => {
                *self.errmsg = format!("{loc} query {sql} failed:\n{}\n", Self::sql_error(&e));
                let text = self.errmsg.clone();
                self.jmsg(MsgType::Fatal, text);
                None
            }
        }
    }

    /// UpdateDb(): affected rows or -1 (errmsg set, M_ERROR).
    pub(crate) fn update_db(&mut self, sql: &str, loc: &str) -> i64 {
        self.change_db(sql, loc, "update")
    }

    /// DeleteDb(): affected rows or -1 (errmsg set, M_ERROR).
    fn delete_db(&mut self, sql: &str, loc: &str) -> i64 {
        self.change_db(sql, loc, "delete")
    }

    fn change_db(&mut self, sql: &str, loc: &str, what: &str) -> i64 {
        match self.db.exec(sql) {
            Ok(n) => n as i64,
            Err(e) => {
                *self.errmsg = format!("{loc} {what} {sql} failed:\n{}\n", Self::sql_error(&e));
                let text = self.errmsg.clone();
                self.jmsg(MsgType::Error, text);
                -1
            }
        }
    }

    /// InsertDb(): affected rows or -1 (errmsg set, M_FATAL).
    fn insert_db(&mut self, sql: &str, loc: &str) -> i64 {
        match self.db.exec(sql) {
            Ok(n) => {
                if n != 1 {
                    *self.errmsg = format!("{loc} Insertion problem: affected_rows={n}\n");
                }
                n as i64
            }
            Err(e) => {
                *self.errmsg = format!("{loc} insert {sql} failed:\n{}\n", Self::sql_error(&e));
                let text = self.errmsg.clone();
                self.jmsg(MsgType::Fatal, text);
                -1
            }
        }
    }

    /// GetSqlRecordMax(): first value of the first row, -1 on failure.
    pub(crate) fn get_sql_record_max(&mut self, sql: &str) -> i64 {
        match self.query_db(sql, "sql.cc:323") {
            Some(t) => match t.rows.first() {
                Some(row) => str_to_int64(row.first().cloned().flatten().unwrap_or_default().as_bytes()),
                None => {
                    *self.errmsg = "error fetching row: \n".into();
                    -1
                }
            },
            None => {
                let error = self.errmsg.clone();
                *self.errmsg = format!("error fetching row: {error}\n");
                -1
            }
        }
    }

    /// SqlInsertAutokeyRecord(): INSERT plus currval of `<table>_<table>id_seq`
    /// on the same connection; 0 on failure.
    pub(crate) fn insert_autokey(&mut self, sql: &str, table: &str) -> (u64, Option<String>) {
        match self.db.rows(&format!("{sql}; SELECT currval('{table}_{table}id_seq')")) {
            Ok(t) => (t.rows.first().and_then(|r| r.first().cloned().flatten())
                .map(|v| str_to_uint64(v.as_bytes())).unwrap_or(0), None),
            Err(e) => (0, Some(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Media and Pool records
// ---------------------------------------------------------------------------

/// cats.h MediaDbRecord.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct MediaRecord {
    pub media_id: i64,
    pub volume_name: String,
    pub media_type: String,
    pub encr_key: String,
    pub pool_id: i64,
    pub first_written: i64,
    pub last_written: i64,
    pub label_date: i64,
    pub initial_write: i64,
    pub label_type: i32,
    pub vol_jobs: u32,
    pub vol_files: u32,
    pub vol_blocks: u32,
    pub vol_mounts: u32,
    pub vol_errors: u32,
    pub vol_writes: u32,
    pub vol_bytes: u64,
    pub max_vol_bytes: u64,
    pub vol_capacity_bytes: u64,
    pub vol_read_time: u64,
    pub vol_write_time: u64,
    pub vol_retention: i64,
    pub vol_use_duration: i64,
    pub action_on_purge: u32,
    pub max_vol_jobs: u32,
    pub max_vol_files: u32,
    pub recycle: i32,
    pub slot: i32,
    pub enabled: i32,
    pub in_changer: i32,
    pub storage_id: i64,
    pub end_file: u32,
    pub end_block: u32,
    pub recycle_count: u32,
    pub min_blocksize: u32,
    pub max_blocksize: u32,
    pub vol_status: String,
    pub device_id: i64,
    pub location_id: i64,
    pub scratch_pool_id: i64,
    pub recycle_pool_id: i64,
    pub set_first_written: bool,
    pub set_label_date: bool,
}

const MEDIA_COLUMNS: &str = "MediaId,VolumeName,VolJobs,VolFiles,VolBlocks,\
VolBytes,VolMounts,VolErrors,VolWrites,MaxVolBytes,VolCapacityBytes,\
MediaType,VolStatus,PoolId,VolRetention,VolUseDuration,MaxVolJobs,\
MaxVolFiles,Recycle,Slot,FirstWritten,LastWritten,InChanger,\
EndFile,EndBlock,LabelType,LabelDate,StorageId,\
Enabled,LocationId,RecycleCount,InitialWrite,\
ScratchPoolId,RecyclePoolId,VolReadTime,VolWriteTime,\
ActionOnPurge,EncryptionKey,MinBlocksize,MaxBlocksize ";

impl MediaRecord {
    /// The column assignments of GetMediaRecord()/FindNextVolume().
    fn fill(&mut self, row: &[Option<String>]) {
        let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        let i64_ = |i: usize| str_to_int64(text(i).as_bytes());
        let u64_ = |i: usize| str_to_uint64(text(i).as_bytes());
        self.media_id = i64_(0);
        self.volume_name = text(1);
        self.vol_jobs = i64_(2) as u32;
        self.vol_files = i64_(3) as u32;
        self.vol_blocks = i64_(4) as u32;
        self.vol_bytes = u64_(5);
        self.vol_mounts = i64_(6) as u32;
        self.vol_errors = i64_(7) as u32;
        self.vol_writes = i64_(8) as u32;
        self.max_vol_bytes = u64_(9);
        self.vol_capacity_bytes = u64_(10);
        self.media_type = text(11);
        self.vol_status = text(12);
        self.pool_id = i64_(13);
        self.vol_retention = u64_(14) as i64;
        self.vol_use_duration = u64_(15) as i64;
        self.max_vol_jobs = i64_(16) as u32;
        self.max_vol_files = i64_(17) as u32;
        self.recycle = i64_(18) as i32;
        self.slot = i64_(19) as i32;
        self.first_written = str_to_utime(&text(20));
        self.last_written = str_to_utime(&text(21));
        self.in_changer = u64_(22) as i32;
        self.end_file = u64_(23) as u32;
        self.end_block = u64_(24) as u32;
        self.label_type = i64_(25) as i32;
        self.label_date = str_to_utime(&text(26));
        self.storage_id = i64_(27);
        self.enabled = i64_(28) as i32;
        self.location_id = i64_(29);
        self.recycle_count = i64_(30) as u32;
        self.initial_write = str_to_utime(&text(31));
        self.scratch_pool_id = i64_(32);
        self.recycle_pool_id = i64_(33);
        self.vol_read_time = i64_(34) as u64;
        self.vol_write_time = i64_(35) as u64;
        self.action_on_purge = i64_(36) as u32;
        self.encr_key = text(37);
        self.min_blocksize = i64_(38) as u32;
        self.max_blocksize = i64_(39) as u32;
    }

    /// catreq.cc OK_media as sent by SendVolumeInfoToStorageDaemon().
    pub(crate) fn ok_media(&self) -> String {
        format!(
            "1000 OK VolName={} VolJobs={} VolFiles={} VolBlocks={} VolBytes={} VolMounts={} \
             VolErrors={} VolWrites={} MaxVolBytes={} VolCapacityBytes={} VolStatus={} Slot={} \
             MaxVolJobs={} MaxVolFiles={} InChanger={} VolReadTime={} VolWriteTime={} EndFile={} \
             EndBlock={} LabelType={} MediaId={} EncryptionKey={} MinBlocksize={} MaxBlocksize={}\n",
            bash_spaces(&self.volume_name), self.vol_jobs, self.vol_files, self.vol_blocks,
            self.vol_bytes, self.vol_mounts, self.vol_errors, self.vol_writes, self.max_vol_bytes,
            self.vol_capacity_bytes, self.vol_status, self.slot, self.max_vol_jobs, self.max_vol_files,
            self.in_changer, self.vol_read_time as i64, self.vol_write_time as i64, self.end_file,
            self.end_block, self.label_type, self.media_id as u64, self.encr_key,
            self.min_blocksize as i32, self.max_blocksize as i32)
    }
}

/// cats.h PoolDbRecord (the columns GetPoolRecord() reads).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PoolRecord {
    pub pool_id: i64,
    pub name: String,
    pub num_vols: u32,
    pub max_vols: u32,
    pub use_once: i32,
    pub use_catalog: i32,
    pub accept_any_volume: i32,
    pub auto_prune: i32,
    pub recycle: i32,
    pub vol_retention: i64,
    pub vol_use_duration: i64,
    pub max_vol_jobs: u32,
    pub max_vol_files: u32,
    pub max_vol_bytes: u64,
    pub pool_type: String,
    pub label_type: i32,
    pub label_format: String,
    pub recycle_pool_id: i64,
    pub scratch_pool_id: i64,
    pub action_on_purge: u32,
    pub min_blocksize: u32,
    pub max_blocksize: u32,
}

/// sql_get.cc GetMediaRecord() (by MediaId when set, else by VolumeName).
pub(crate) fn get_media_record(cat: &mut Cat, mr: &mut MediaRecord) -> bool {
    if mr.media_id == 0 && mr.volume_name.is_empty() {
        mr.media_id = cat.get_sql_record_max("SELECT count(*) from Media");
        return true;
    }
    let sql = if mr.media_id != 0 {
        format!("SELECT {MEDIA_COLUMNS}FROM Media WHERE MediaId={}", mr.media_id)
    } else {
        format!("SELECT {MEDIA_COLUMNS}FROM Media WHERE VolumeName='{}'", esc(&mr.volume_name))
    };
    let Some(table) = cat.query_db(&sql, "sql_get.cc:1106") else {
        *cat.errmsg = if mr.media_id != 0 {
            format!("Media record for MediaId={} not found in Catalog.\n", mr.media_id as u32)
        } else {
            format!("Media record for Vol={} not found in Catalog.\n", mr.volume_name)
        };
        return false;
    };
    match table.rows.len() {
        1 => {
            mr.fill(&table.rows[0]);
            true
        }
        0 => {
            *cat.errmsg = if mr.media_id != 0 {
                format!("Media record MediaId={} not found.\n", mr.media_id)
            } else {
                format!("Media record for Volume \"{}\" not found.\n", mr.volume_name)
            };
            false
        }
        n => {
            *cat.errmsg = format!("More than one Volume!: {n}\n");
            let text = cat.errmsg.clone();
            cat.jmsg(MsgType::Error, text);
            false
        }
    }
}

/// sql_get.cc GetPoolRecord() (by PoolId when set, else by Name), with the
/// NumVols correction.
pub(crate) fn get_pool_record(cat: &mut Cat, pr: &mut PoolRecord) -> bool {
    let columns = "SELECT PoolId,Name,NumVols,MaxVols,UseOnce,UseCatalog,AcceptAnyVolume,\
AutoPrune,Recycle,VolRetention,VolUseDuration,MaxVolJobs,MaxVolFiles,\
MaxVolBytes,PoolType,LabelType,LabelFormat,RecyclePoolId,ScratchPoolId,\
ActionOnPurge,MinBlocksize,MaxBlocksize FROM Pool WHERE ";
    let sql = if pr.pool_id != 0 {
        format!("{columns}Pool.PoolId={}", pr.pool_id)
    } else {
        format!("{columns}Pool.Name='{}'", esc(&pr.name))
    };
    let mut ok = false;
    if let Some(table) = cat.query_db(&sql, "sql_get.cc:613") {
        match table.rows.len() {
            1 => {
                let row = &table.rows[0];
                let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
                let i64_ = |i: usize| str_to_int64(text(i).as_bytes());
                pr.pool_id = i64_(0);
                pr.name = text(1);
                pr.num_vols = i64_(2) as u32;
                pr.max_vols = i64_(3) as u32;
                pr.use_once = i64_(4) as i32;
                pr.use_catalog = i64_(5) as i32;
                pr.accept_any_volume = i64_(6) as i32;
                pr.auto_prune = i64_(7) as i32;
                pr.recycle = i64_(8) as i32;
                pr.vol_retention = i64_(9);
                pr.vol_use_duration = i64_(10);
                pr.max_vol_jobs = i64_(11) as u32;
                pr.max_vol_files = i64_(12) as u32;
                pr.max_vol_bytes = str_to_uint64(text(13).as_bytes());
                pr.pool_type = text(14);
                pr.label_type = i64_(15) as i32;
                pr.label_format = text(16);
                pr.recycle_pool_id = i64_(17);
                pr.scratch_pool_id = i64_(18);
                pr.action_on_purge = i64_(19) as u32;
                pr.min_blocksize = i64_(20) as u32;
                pr.max_blocksize = i64_(21) as u32;
                ok = true;
            }
            0 => {}
            n => {
                *cat.errmsg = format!("More than one Pool!: {n}\n");
                let text = cat.errmsg.clone();
                cat.jmsg(MsgType::Error, text);
            }
        }
    }
    if ok {
        let num_vols = cat.get_sql_record_max(&format!("SELECT count(*) from Media WHERE PoolId={}", pr.pool_id)) as u32;
        if num_vols != pr.num_vols {
            pr.num_vols = num_vols;
            ok = update_pool_record(cat, pr);
        }
    } else {
        *cat.errmsg = "Pool record not found in Catalog.\n".into();
    }
    ok
}

/// sql_update.cc UpdatePoolRecord().
pub(crate) fn update_pool_record(cat: &mut Cat, pr: &mut PoolRecord) -> bool {
    pr.num_vols = cat.get_sql_record_max(&format!("SELECT count(*) from Media WHERE PoolId={}", pr.pool_id)) as u32;
    let sql = format!(
        "UPDATE Pool SET NumVols={},MaxVols={},UseOnce={},UseCatalog={},\
AcceptAnyVolume={},VolRetention='{}',VolUseDuration='{}',\
MaxVolJobs={},MaxVolFiles={},MaxVolBytes={},Recycle={},\
AutoPrune={},LabelType={},LabelFormat='{}',RecyclePoolId={},\
ScratchPoolId={},ActionOnPurge={},MinBlockSize={},MaxBlockSize={} \
WHERE PoolId={}",
        pr.num_vols, pr.max_vols, pr.use_once, pr.use_catalog, pr.accept_any_volume,
        pr.vol_retention as u64, pr.vol_use_duration as u64, pr.max_vol_jobs, pr.max_vol_files,
        pr.max_vol_bytes, pr.recycle, pr.auto_prune, pr.label_type, esc(&pr.label_format),
        pr.recycle_pool_id, pr.scratch_pool_id, pr.action_on_purge as i32, pr.min_blocksize as i32,
        pr.max_blocksize as i32, pr.pool_id);
    cat.update_db(&sql, "sql_update.cc:263") > 0
}

/// sql_update.cc UpdateMediaRecord() (with MakeInchangerUnique()).
pub(crate) fn update_media_record(cat: &mut Cat, mr: &MediaRecord) -> bool {
    let name = esc(&mr.volume_name);
    if mr.set_first_written {
        cat.update_db(&format!("UPDATE Media SET FirstWritten='{}' WHERE VolumeName='{name}'",
            bstrutime(mr.first_written)), "sql_update.cc:308");
    }
    if mr.set_label_date {
        let t = if mr.label_date == 0 { now() } else { mr.label_date };
        cat.update_db(&format!("UPDATE Media SET LabelDate='{}' WHERE VolumeName='{name}'", bstrutime(t)),
            "sql_update.cc:321");
    }
    if mr.last_written != 0 {
        cat.update_db(&format!("UPDATE Media Set LastWritten='{}' WHERE VolumeName='{name}'",
            bstrutime(mr.last_written)), "sql_update.cc:331");
    }
    let sql = format!(
        "UPDATE Media SET VolJobs={},\
VolFiles={},VolBlocks={},VolBytes={},VolMounts={},VolErrors={},\
VolWrites={},MaxVolBytes={},VolStatus='{}',\
Slot={},InChanger={},VolReadTime={},VolWriteTime={},\
LabelType={},StorageId={},PoolId={},VolRetention={},VolUseDuration={},\
MaxVolJobs={},MaxVolFiles={},Enabled={},LocationId={},\
ScratchPoolId={},RecyclePoolId={},RecycleCount={},Recycle={},\
ActionOnPurge={},EncryptionKey='{}',MinBlocksize={},MaxBlocksize={} \
WHERE VolumeName='{name}'",
        mr.vol_jobs, mr.vol_files, mr.vol_blocks, mr.vol_bytes, mr.vol_mounts, mr.vol_errors,
        mr.vol_writes, mr.max_vol_bytes, esc(&mr.vol_status), mr.slot, mr.in_changer,
        mr.vol_read_time as i64, mr.vol_write_time as i64, mr.label_type, mr.storage_id, mr.pool_id,
        mr.vol_retention as u64, mr.vol_use_duration as u64, mr.max_vol_jobs as i32,
        mr.max_vol_files as i32, mr.enabled, mr.location_id as u64, mr.scratch_pool_id as u64,
        mr.recycle_pool_id as u64, mr.recycle_count as i32, mr.recycle, mr.action_on_purge as i32,
        mr.encr_key, mr.min_blocksize, mr.max_blocksize);
    let mut ok = cat.update_db(&sql, "sql_update.cc:361") > 0;
    ok &= make_inchanger_unique(cat, mr);
    ok
}

/// sql_update.cc MakeInchangerUnique().
pub(crate) fn make_inchanger_unique(cat: &mut Cat, mr: &MediaRecord) -> bool {
    if mr.in_changer != 0 && mr.slot != 0 && mr.storage_id != 0 {
        let sql = if mr.media_id != 0 {
            format!("UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={} AND StorageId={} AND MediaId!={}",
                mr.slot, mr.storage_id, mr.media_id)
        } else if !mr.volume_name.is_empty() {
            format!("UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={} AND StorageId={} AND VolumeName!='{}'",
                mr.slot, mr.storage_id, esc(&mr.volume_name))
        } else {
            format!("UPDATE Media SET InChanger=0, Slot=0 WHERE Slot={} AND StorageId={}", mr.slot, mr.storage_id)
        };
        return cat.update_db(&sql, "sql_update.cc:445") >= 0;
    }
    true
}

/// sql_create.cc CreateMediaRecord().
fn create_media_record(cat: &mut Cat, mr: &mut MediaRecord) -> bool {
    let name = esc(&mr.volume_name);
    if let Some(t) = cat.query_db(&format!("SELECT MediaId FROM Media WHERE VolumeName='{name}'"), "sql_create.cc:401") {
        if !t.rows.is_empty() {
            *cat.errmsg = format!("Volume \"{}\" already exists.\n", mr.volume_name);
            return false;
        }
    }
    let sql = format!(
        "INSERT INTO Media (VolumeName,MediaType,MediaTypeId,PoolId,MaxVolBytes,\
VolCapacityBytes,Recycle,VolRetention,VolUseDuration,MaxVolJobs,MaxVolFiles,\
VolStatus,Slot,VolBytes,InChanger,VolReadTime,VolWriteTime,\
EndFile,EndBlock,LabelType,StorageId,DeviceId,LocationId,\
ScratchPoolId,RecyclePoolId,Enabled,ActionOnPurge,EncryptionKey,\
MinBlocksize,MaxBlocksize,VolFiles) \
VALUES ('{}','{}',0,{},{},{},{},{},{},{},{},'{}',{},{},{},{},{},0,0,{},{},\
{},{},{},{},{},{},'{}',{},{},{})",
        name, esc(&mr.media_type), mr.pool_id as u32, mr.max_vol_bytes, mr.vol_capacity_bytes,
        mr.recycle, mr.vol_retention as u64, mr.vol_use_duration as u64, mr.max_vol_jobs,
        mr.max_vol_files, esc(&mr.vol_status), mr.slot, mr.vol_bytes, mr.in_changer,
        mr.vol_read_time as i64, mr.vol_write_time as i64, mr.label_type, mr.storage_id,
        mr.device_id, mr.location_id, mr.scratch_pool_id, mr.recycle_pool_id, mr.enabled,
        mr.action_on_purge, mr.encr_key, mr.min_blocksize as i32, mr.max_blocksize as i32,
        mr.vol_files as i32);
    let (id, error) = cat.insert_autokey(&sql, "Media");
    mr.media_id = id as i64;
    if mr.media_id == 0 {
        *cat.errmsg = format!("Create DB Media record {sql} failed. ERR={}\n", error.unwrap_or_default());
        return false;
    }
    let mut ok = true;
    if mr.set_label_date {
        if mr.label_date == 0 {
            mr.label_date = now();
        }
        ok = cat.update_db(&format!("UPDATE Media SET LabelDate='{}' WHERE MediaId={}",
            bstrutime(mr.label_date), mr.media_id as i32), "sql_create.cc:463") > 0;
    }
    ok &= make_inchanger_unique(cat, mr);
    ok
}

/// sql_find.cc FindNextVolume(); `item == -1` finds the oldest volume.
/// Returns the C++ row count (non-zero means found).
fn find_next_volume(cat: &mut Cat, item: i32, in_changer: bool, mr: &mut MediaRecord, unwanted: Option<&str>) -> i64 {
    let find_oldest = item == -1;
    let mut item = if find_oldest { 1 } else { item };
    loop {
        let sql = if find_oldest {
            format!("SELECT {MEDIA_COLUMNS}FROM Media WHERE PoolId={} AND MediaType='{}' AND VolStatus IN \
('Unlabeled', 'Full','Recycle','Purged','Used','Append') AND Enabled=1 ORDER BY LastWritten LIMIT {item}",
                mr.pool_id, esc(&mr.media_type))
        } else {
            let changer = if in_changer { format!("AND InChanger=1 AND StorageId={}", mr.storage_id) } else { String::new() };
            let order = if mr.vol_status == "Recycle" || mr.vol_status == "Purged" {
                "AND Recycle=1 ORDER BY LastWritten ASC,MediaId"
            } else {
                "ORDER BY LastWritten IS NULL,LastWritten DESC,MediaId\n"
            };
            format!("SELECT {MEDIA_COLUMNS}FROM Media WHERE PoolId={} AND MediaType='{}' AND Enabled=1 \
AND VolStatus='{}' {changer} {order} LIMIT {item}",
                mr.pool_id, esc(&mr.media_type), esc(&mr.vol_status))
        };
        let Some(table) = cat.query_db(&sql, "sql_find.cc:512") else { return 0 };
        let mut num_rows = table.rows.len() as i64;
        if item as i64 > num_rows || item < 1 {
            *cat.errmsg = format!("Request for Volume item {item} greater than max {num_rows} or less than 1\n");
            return 0;
        }
        let mut found = false;
        for i in 0..item as usize {
            let Some(row) = table.rows.get(i) else {
                *cat.errmsg = format!("No Volume record found for item {i}.\n");
                return 0;
            };
            let name = row.get(1).cloned().flatten().unwrap_or_default();
            if let Some(list) = unwanted {
                if list.split(',').any(|v| v == name) {
                    num_rows -= 1;
                    if num_rows <= 0 {
                        return num_rows;
                    }
                    continue;
                }
            }
            mr.fill(row);
            found = true;
            break;
        }
        if !found && find_oldest {
            item += 1;
            continue;
        }
        return num_rows;
    }
}

/// ua_db.cc SetPoolDbrDefaultsInMediaDbr().
pub(crate) fn set_pool_defaults(mr: &mut MediaRecord, pr: &PoolRecord) {
    mr.pool_id = pr.pool_id;
    mr.vol_status = "Unlabeled".into();
    mr.recycle = pr.recycle;
    mr.vol_retention = pr.vol_retention;
    mr.vol_use_duration = pr.vol_use_duration;
    mr.action_on_purge = pr.action_on_purge;
    mr.recycle_pool_id = pr.recycle_pool_id;
    mr.max_vol_jobs = pr.max_vol_jobs;
    mr.max_vol_files = pr.max_vol_files;
    mr.max_vol_bytes = pr.max_vol_bytes;
    mr.label_type = pr.label_type;
    mr.min_blocksize = pr.min_blocksize;
    mr.max_blocksize = pr.max_blocksize;
    mr.enabled = VOL_ENABLED;
}

/// ua_label.cc IsVolumeNameLegal(NULL, name).
fn is_volume_name_legal(name: &str) -> bool {
    !name.starts_with('/') && !name.is_empty() && name.len() < 128
        && name.bytes().all(|c| c.is_ascii_alphanumeric() || b":.-_/".contains(&c))
}

// ---------------------------------------------------------------------------
// Job context
// ---------------------------------------------------------------------------

/// The parts of the C++ JobControlRecord the catalog requests use.
#[derive(Debug, Clone, Default)]
pub struct JobContext {
    /// jcr->JobId
    pub job_id: u32,
    /// jcr->start_time
    pub start_time: i64,
    /// jcr->dir_impl->jr.PoolId
    pub pool_id: i64,
    /// jcr->dir_impl->res.pool->resource_name_
    pub pool_name: String,
    /// jcr->dir_impl->res.pool->catalog_files
    pub catalog_files: bool,
    /// jcr->dir_impl->res.write_storage (None for restore/verify jobs)
    pub write_storage: Option<WriteStorage>,
}

#[derive(Debug, Clone, Default)]
pub struct WriteStorage {
    pub media_type: String,
    /// StorageResource::StorageId (catalog Storage record)
    pub storage_id: i64,
    pub autochanger: bool,
}

/// next_vol.cc SetStorageidInMr().
fn set_storage_id(store: Option<&WriteStorage>, mr: &mut MediaRecord) {
    if let Some(store) = store {
        mr.storage_id = store.storage_id;
    }
}

/// Build the job context from the catalog Job record and the configuration.
pub fn load_context(db: &dyn Db, config: Option<&crate::director_config::SharedConfig>, job_id: u32,
                    start_time: i64, storage: Option<&str>) -> JobContext {
    let first = |sql: &str| -> Vec<String> {
        db.rows(sql).ok().and_then(|t| t.rows.into_iter().next())
            .map(|r| r.into_iter().map(|v| v.unwrap_or_default()).collect()).unwrap_or_default()
    };
    let job = first(&format!("SELECT PoolId,Type FROM Job WHERE JobId={job_id}"));
    let pool_id = job.first().map(|v| str_to_int64(v.as_bytes())).unwrap_or(0);
    let job_type = job.get(1).cloned().unwrap_or_default();
    let pool_name = first(&format!("SELECT Name FROM Pool WHERE PoolId={pool_id}")).into_iter().next().unwrap_or_default();
    let config = config.and_then(|c| c.read().ok());
    let catalog_files = config.as_ref().and_then(|c| c.raw.iter()
        .find(|r| r.norm_type() == "pool" && r.name() == Some(pool_name.as_str()))
        .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == "catalogfiles"))
        .and_then(|d| d.first())
        .and_then(|v| bareos_config::values::parse_bool(v).ok().map(|(_, on)| on)))
        .unwrap_or(true);
    let write_storage = match (job_type.as_str(), storage) {
        ("R" | "V" | "D" | "U" | "I", _) | (_, None) => None,
        (_, Some(name)) => {
            let media_type = config.as_ref().and_then(|c| c.find_storage(name).map(|s| s.media_type.clone())).unwrap_or_default();
            let autochanger = config.as_ref().is_some_and(|c| crate::ua_select::is_autochanger(c, name));
            let storage_id = first(&format!("SELECT StorageId FROM Storage WHERE Name='{}'", esc(name)))
                .first().map(|v| str_to_int64(v.as_bytes())).unwrap_or(0);
            Some(WriteStorage { media_type, storage_id, autochanger })
        }
    };
    JobContext { job_id, start_time, pool_id, pool_name, catalog_files, write_storage }
}

// ---------------------------------------------------------------------------
// Attributes (UpdateAttribute) and the batch file insert
// ---------------------------------------------------------------------------

/// cats.h AttributesDbRecord (jcr->ar).
#[derive(Debug, Clone, Default)]
struct AttrRecord {
    file_index: u32,
    file_type: i32,
    fname: Vec<u8>,
    attr: Vec<u8>,
    stream: i32,
    job_id: u32,
    digest: Option<String>,
    delta_seq: u32,
    file_id: u64,
}

/// Opens the private connection of the batch insert (CloneDatabaseConnection).
pub type BatchOpener<'a> = &'a dyn Fn() -> Option<Box<dyn Db + Send>>;

/// jcr->db_batch with the COPY state of the PostgreSQL driver.
struct Batch {
    db: Box<dyn Db + Send>,
    errmsg: String,
    changes: u32,
    pending: Vec<String>,
}

impl Batch {
    /// Send the buffered rows to the temporary batch table.
    fn flush(&mut self) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let rows = std::mem::take(&mut self.pending).join(",");
        self.db.exec(&format!("INSERT INTO batch VALUES {rows}")).map(|_| ())
    }
}

/// Catalog state of one running job (jcr->db errmsg, jcr->ar, jcr->db_batch).
pub struct CatReqJob {
    pub ctx: JobContext,
    errmsg: String,
    ar: AttrRecord,
    cached_attribute: bool,
    batch: Option<Batch>,
    batch_started: bool,
}

/// What a request does besides the catalog: the bytes for the SD (empty:
/// no answer), job messages and the new job counters (UpdateJobRecord).
#[derive(Debug, Default, PartialEq)]
pub struct Reply {
    pub to_sd: String,
    pub messages: Messages,
    pub job_counts: Option<(u32, u64)>,
}

impl CatReqJob {
    pub fn new(ctx: JobContext) -> Self {
        Self { ctx, errmsg: String::new(), ar: AttrRecord::default(), cached_attribute: false, batch: None, batch_started: false }
    }

    /// getmsg.cc BgetDirmsg(): a `C`, `U` or `B` message of the SD.
    pub fn dispatch(&mut self, db: Option<&dyn Db>, open_batch: BatchOpener, msg: &[u8]) -> Reply {
        let mut reply = Reply::default();
        if bsscanf(msg, "%020s Job=%127s ").0 != 2 {
            reply.messages.push((MsgType::Error, format!("Malformed message: {}\n", String::from_utf8_lossy(cstr(msg, 0)))));
            return reply;
        }
        match msg.first() {
            Some(b'C') => self.catalog_request(db, open_batch, msg, &mut reply),
            Some(b'U') => self.catalog_update(db, open_batch, msg, &mut reply),
            Some(b'B') => self.blast_attributes(db, open_batch, msg, &mut reply),
            _ => {}
        }
        reply
    }

    /// msgchan.cc MsgThreadCleanup() EndTransaction() and backup.cc: flush
    /// the cached attribute, then WriteBatchFileRecords().
    pub fn end_of_job(&mut self, db: Option<&dyn Db>, open_batch: BatchOpener) -> Messages {
        let mut msgs = Messages::new();
        if let Some(db) = db {
            if self.cached_attribute {
                let ar = self.ar.clone();
                if !self.create_attributes_record(db, open_batch, &ar, &mut msgs) {
                    msgs.push((MsgType::Fatal, format!("Attribute create error. {}", self.errmsg)));
                }
                self.cached_attribute = false;
            }
        }
        if self.batch_started {
            self.write_batch_file_records(&mut msgs);
        }
        msgs
    }

    /// catreq.cc CatalogRequest().
    fn catalog_request(&mut self, db: Option<&dyn Db>, open_batch: BatchOpener, msg: &[u8], reply: &mut Reply) {
        let text = String::from_utf8_lossy(cstr(msg, 0)).into_owned();
        let Some(db) = db else {
            reply.to_sd = format!("1990 Invalid Catalog Request: {text}");
            reply.messages.push((MsgType::Fatal, format!("Invalid Catalog request; DB not open: {text}")));
            return;
        };
        let ctx = self.ctx.clone();
        let (n, v) = bsscanf(msg, "CatReq Job=%127s FindMedia=%d pool_name=%127s media_type=%127s unwanted_volumes=%s\n");
        if n == 5 {
            let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs: &mut reply.messages };
            let mut pr = PoolRecord { name: unbash_spaces(&scanned_s(&v, 2)), ..Default::default() };
            let mut mr = MediaRecord { media_type: scanned_s(&v, 3), ..Default::default() };
            let unwanted = scanned_s(&v, 4);
            let mut ok = get_pool_record(&mut cat, &mut pr);
            if ok {
                mr.pool_id = pr.pool_id;
                set_storage_id(ctx.write_storage.as_ref(), &mut mr);
                mr.scratch_pool_id = pr.scratch_pool_id;
                ok = find_next_volume_for_append(&mut cat, &ctx, &mut mr, scanned_d(&v, 1) as i32, &unwanted);
            }
            reply.to_sd = if ok { mr.ok_media() } else { "1901 No Media.\n".into() };
            return;
        }
        let (n, v) = bsscanf(msg, "CatReq Job=%127s GetVolInfo VolName=%127s write=%d\n");
        if n == 3 {
            let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs: &mut reply.messages };
            let mut mr = MediaRecord { volume_name: unbash_spaces(&scanned_s(&v, 1)), ..Default::default() };
            if get_media_record(&mut cat, &mut mr) {
                let mut reason: Option<String> = None;
                if scanned_d(&v, 2) != 0 {
                    let media_type = ctx.write_storage.as_ref().map(|s| s.media_type.as_str()).unwrap_or("");
                    if mr.pool_id != ctx.pool_id {
                        reason = Some("not in Pool".into());
                    } else if mr.media_type != media_type {
                        reason = Some("not correct MediaType".into());
                    } else {
                        reason = check_if_volume_valid_or_recyclable(&mut cat, &mut mr);
                    }
                }
                if reason.is_none() && mr.enabled != VOL_ENABLED {
                    reason = Some("is not Enabled".into());
                }
                reply.to_sd = match reason {
                    None => mr.ok_media(),
                    Some(reason) => format!("1998 Volume \"{}\" catalog status is {}, {reason}.\n", mr.volume_name, mr.vol_status),
                };
            } else {
                reply.to_sd = format!("1997 Volume \"{}\" not in catalog.\n", mr.volume_name);
            }
            return;
        }
        let (n, v) = bsscanf(msg, "CatReq Job=%127s UpdateMedia VolName=%s \
VolJobs=%u VolFiles=%u VolBlocks=%u VolBytes=%lld VolMounts=%u \
VolErrors=%u VolWrites=%u MaxVolBytes=%lld EndTime=%lld VolStatus=%10s \
Slot=%d relabel=%d InChanger=%d VolReadTime=%lld VolWriteTime=%lld \
VolFirstWritten=%lld\n");
        if n == 18 {
            let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs: &mut reply.messages };
            reply.to_sd = update_media(&mut cat, &ctx, &v);
            return;
        }
        let (n, v) = bsscanf(msg, "CatReq Job=%127s CreateJobMedia  FirstIndex=%u LastIndex=%u StartFile=%u EndFile=%u  \
StartBlock=%u EndBlock=%u Copy=%d Strip=%d MediaId=%lld\n");
        if n == 10 {
            let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs: &mut reply.messages };
            if !create_jobmedia_record(&mut cat, ctx.job_id, &v) {
                let error = cat.errmsg.clone();
                cat.jmsg(MsgType::Fatal, format!("Catalog error creating JobMedia record. {error}\n"));
                reply.to_sd = "1992 Create JobMedia error\n".into();
            } else {
                reply.to_sd = "1000 OK CreateJobMedia\n".into();
            }
            return;
        }
        let (n, _) = bsscanf(msg, "Catreq Job=%127s UpdateFileList\n");
        if n == 1 {
            if self.batch.is_some() && !self.write_batch_file_records(&mut reply.messages) {
                let error = self.batch.as_ref().map(|b| b.errmsg.clone()).unwrap_or_default();
                reply.messages.push((MsgType::Fatal, format!("Catalog error updating File table. {error}\n")));
                reply.to_sd = "1992 Update File table error\n".into();
            }
            let _ = open_batch;
            return;
        }
        // PRIu32/PRIu64 are "u"/"lu": bsscanf stores %lu as 32 bits.
        let (n, v) = bsscanf(msg, "Catreq Job=%127s UpdateJobRecord JobFiles=%u JobBytes=%lu\n");
        if n == 3 {
            let (files, bytes) = (scanned_u(&v, 1) as u32, scanned_u(&v, 2) as u32 as u64);
            reply.job_counts = Some((files, bytes));
            let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs: &mut reply.messages };
            if cat.update_db(&format!("UPDATE Job SET JobBytes={bytes},JobFiles={files} WHERE JobId={}", ctx.job_id),
                "sql_update.cc:119") <= 0 {
                let error = cat.errmsg.clone();
                cat.jmsg(MsgType::Fatal, format!("Catalog error updating Job record. {error}\n"));
                reply.to_sd = "1992 Update job record error\n".into();
            }
            return;
        }
        let (n, v) = bsscanf(msg, "CatReq Job=%127s DeleteNullJobmediaRecords jobid=%u\n");
        if n == 2 {
            let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs: &mut reply.messages };
            let rows = cat.delete_db(&format!("DELETE FROM jobmedia WHERE jobid={} AND firstindex=0 AND lastindex=0",
                scanned_u(&v, 1)), "sql_delete.cc:138");
            reply.to_sd = if rows == -1 { "1992 DeleteNullJobMediaRecords error\n".into() } else { "1000 OK DeleteNullJobmediaRecords\n".into() };
            return;
        }
        reply.to_sd = format!("1990 Invalid Catalog Request: {text}");
        reply.messages.push((MsgType::Fatal, format!("Invalid Catalog request: {text}")));
    }

    /// catreq.cc CatalogUpdate().
    fn catalog_update(&mut self, db: Option<&dyn Db>, open_batch: BatchOpener, msg: &[u8], reply: &mut Reply) {
        if !self.ctx.catalog_files {
            return;
        }
        let Some(db) = db else {
            let text = String::from_utf8_lossy(cstr(msg, 0)).into_owned();
            reply.to_sd = format!("1994 Invalid Catalog Update: {text}");
            reply.messages.push((MsgType::Fatal, format!("Invalid Catalog Update; DB not open: {text}")));
            return;
        };
        self.update_attribute(db, open_batch, msg, &mut reply.messages);
    }

    /// getmsg.cc `BlastAttr Job=%127s File=%255s` with DespoolAttributesFromFile().
    fn blast_attributes(&mut self, db: Option<&dyn Db>, open_batch: BatchOpener, msg: &[u8], reply: &mut Reply) {
        let (n, v) = bsscanf(msg, "BlastAttr Job=%127s File=%255s");
        if n != 2 {
            reply.messages.push((MsgType::Error, format!("Malformed message: {}\n", String::from_utf8_lossy(cstr(msg, 0)))));
            return;
        }
        let file = unbash_spaces(&scanned_s(&v, 1));
        let ok = self.despool_attributes_from_file(db, open_batch, &file, &mut reply.messages);
        reply.to_sd = if ok { "1000 OK BlastAttr\n".into() } else { "1990 ERROR BlastAttr\n".into() };
    }

    /// catreq.cc DespoolAttributesFromFile().
    fn despool_attributes_from_file(&mut self, db: Option<&dyn Db>, open_batch: BatchOpener, file: &str, msgs: &mut Messages) -> bool {
        let Some(db) = db.filter(|_| self.ctx.catalog_files) else { return false };
        let Ok(data) = std::fs::read(file) else { return false };
        let mut pos = 0;
        let mut msg: Vec<u8> = Vec::new();
        while pos + 4 <= data.len() {
            let length = i32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            pos += 4;
            if length > 0 {
                let length = length as usize;
                if pos + length > data.len() {
                    msgs.push((MsgType::Fatal, "read attr spool error. ERR=Success\n".into()));
                    return false;
                }
                msg = data[pos..pos + length].to_vec();
                pos += length;
            }
            let length = length.max(0) as usize;
            let record = msg[..length.min(msg.len())].to_vec();
            self.update_attribute(db, open_batch, &record, msgs);
        }
        true
    }

    /// catreq.cc UpdateAttribute().
    fn update_attribute(&mut self, db: &dyn Db, open_batch: BatchOpener, msg: &[u8], msgs: &mut Messages) {
        let mut p = skip_nonspaces(msg, 0);
        p = skip_spaces(msg, p);
        p = skip_nonspaces(msg, p);
        p = skip_spaces(msg, p);
        p = skip_nonspaces(msg, p);
        p += 1;
        let be = |at: usize| u32::from_be_bytes([byte(msg, at), byte(msg, at + 1), byte(msg, at + 2), byte(msg, at + 3)]);
        let file_index = be(p + 8) as i32;
        let stream = be(p + 12) as i32;
        p += 20;
        match stream {
            STREAM_UNIX_ATTRIBUTES | STREAM_UNIX_ATTRIBUTES_EX => {
                if self.cached_attribute {
                    let ar = self.ar.clone();
                    if !self.create_attributes_record(db, open_batch, &ar, msgs) {
                        msgs.push((MsgType::Fatal, format!("Attribute create error: ERR={}", self.errmsg)));
                    }
                    self.cached_attribute = false;
                }
                let mut q = skip_nonspaces(msg, p);
                q = skip_spaces(msg, q);
                let file_type = str_to_int64(cstr(msg, q)) as i32;
                q = skip_nonspaces(msg, q);
                q = skip_spaces(msg, q);
                let fname = cstr(msg, q).to_vec();
                let attr_at = q + fname.len() + 1;
                let attr = cstr(msg, attr_at).to_vec();
                let mut delta_seq = 0;
                if file_type == FT_REG {
                    let mut d = attr_at + attr.len() + 1;
                    d += cstr(msg, d).len() + 1;
                    d += cstr(msg, d).len() + 1;
                    if d < msg.len() {
                        delta_seq = str_to_int64(cstr(msg, d)) as u32;
                    }
                }
                self.ar = AttrRecord {
                    file_index: if file_type == FT_DELETED { 0 } else { file_index as u32 },
                    file_type, fname, attr, stream, job_id: self.ctx.job_id, digest: None, delta_seq, file_id: self.ar.file_id,
                };
                self.cached_attribute = true;
            }
            STREAM_RESTORE_OBJECT => self.create_restore_object(db, msg, p, file_index, stream, msgs),
            STREAM_MD5_DIGEST | STREAM_SHA1_DIGEST | STREAM_SHA256_DIGEST | STREAM_SHA512_DIGEST | STREAM_XXH128_DIGEST => {
                if self.ar.file_index != file_index as u32 {
                    msgs.push((MsgType::Warning, format!("{} not same File={} as attributes={}\n",
                        stream_to_ascii(stream), file_index, self.ar.file_index as i32)));
                    return;
                }
                let len = match stream {
                    STREAM_MD5_DIGEST | STREAM_XXH128_DIGEST => 16,
                    STREAM_SHA1_DIGEST => 20,
                    STREAM_SHA256_DIGEST => 32,
                    _ => 64,
                };
                let raw: Vec<u8> = (0..len).map(|i| byte(msg, p + i)).collect();
                let digest = bin_to_base64(&raw);
                if self.cached_attribute {
                    let mut ar = self.ar.clone();
                    ar.digest = Some(digest);
                    self.ar.digest = ar.digest.clone();
                    if !self.create_attributes_record(db, open_batch, &ar, msgs) {
                        msgs.push((MsgType::Fatal, format!("attribute create error. {}\n", self.errmsg)));
                    }
                    self.cached_attribute = false;
                } else {
                    let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs };
                    if cat.update_db(&format!("UPDATE File SET MD5='{}' WHERE FileId={}", esc(&digest), self.ar.file_id),
                        "sql_update.cc:63") <= 0 {
                        let error = cat.errmsg.clone();
                        cat.jmsg(MsgType::Error, format!("Catalog error updating file digest. {error}\n"));
                    }
                }
            }
            _ => {}
        }
    }

    /// The STREAM_RESTORE_OBJECT branch of UpdateAttribute() with
    /// sql_create.cc CreateRestoreObjectRecord().
    fn create_restore_object(&mut self, db: &dyn Db, msg: &[u8], p: usize, file_index: i32, stream: i32, msgs: &mut Messages) {
        let mut q = skip_nonspaces(msg, p);
        q = skip_spaces(msg, q);
        let mut field = || {
            let value = str_to_int64(cstr(msg, q)) as i32;
            q = skip_nonspaces(msg, q);
            q = skip_spaces(msg, q);
            value
        };
        let file_type = field();
        let object_index = field();
        let object_len = field();
        let object_full_len = field();
        let object_compression = field();
        let plugin_name = cstr(msg, q).to_vec();
        let name_at = q + plugin_name.len() + 1;
        let object_name = cstr(msg, name_at).to_vec();
        let object_at = name_at + object_name.len() + 1;
        let object: Vec<u8> = (0..object_len.max(0) as usize).map(|i| byte(msg, object_at + i)).collect();
        let sql = format!(
            "INSERT INTO RestoreObject (ObjectName,PluginName,RestoreObject,\
ObjectLength,ObjectFullLength,ObjectIndex,ObjectType,\
ObjectCompression,FileIndex,JobId) \
VALUES ('{}','{}','\\x{}',{},{},{},{},{},{},{})",
            esc(&String::from_utf8_lossy(&object_name)), esc(&String::from_utf8_lossy(&plugin_name)),
            hex::encode(&object), object_len, object_full_len, object_index, file_type, object_compression,
            file_index, self.ctx.job_id);
        let _ = stream;
        let mut cat = Cat { db, errmsg: &mut self.errmsg, msgs };
        let (id, error) = cat.insert_autokey(&sql, "RestoreObject");
        if id == 0 {
            *cat.errmsg = format!("Create db Object record {sql} failed. ERR={}", error.unwrap_or_default());
            let text = format!("Restore object create error. {}", cat.errmsg);
            cat.jmsg(MsgType::Fatal, text);
        }
    }

    /// sql_create.cc CreateAttributesRecord() in batch mode
    /// (CreateBatchFileAttributesRecord()).
    fn create_attributes_record(&mut self, db: &dyn Db, open_batch: BatchOpener, ar: &AttrRecord, msgs: &mut Messages) -> bool {
        let _ = db;
        self.errmsg.clear();
        if !(ar.stream == STREAM_UNIX_ATTRIBUTES || ar.stream == STREAM_UNIX_ATTRIBUTES_EX) {
            self.errmsg = format!("Attempt to put non-attributes into catalog. Stream={}\n", ar.stream);
            msgs.push((MsgType::Fatal, self.errmsg.clone()));
            return false;
        }
        if ar.file_type == FT_BASE {
            self.errmsg = "Cannot Copy/Migrate job using BaseJob.\n".into();
            msgs.push((MsgType::Fatal, self.errmsg.clone()));
            return true;
        }
        if self.batch_started && self.batch.as_ref().is_some_and(|b| b.changes > BATCH_FLUSH) {
            self.write_batch_file_records(msgs);
        }
        if !self.batch_started {
            if self.batch.is_none() {
                match open_batch() {
                    Some(db) => self.batch = Some(Batch { db, errmsg: String::new(), changes: 0, pending: Vec::new() }),
                    None => {
                        self.errmsg = "Could not init database batch connection\n".into();
                        msgs.push((MsgType::Fatal, self.errmsg.clone()));
                        return false;
                    }
                }
            }
            let batch = self.batch.as_mut().expect("batch connection");
            if let Err(error) = batch.db.exec("CREATE TEMPORARY TABLE batch (FileIndex int,JobId int,Path varchar,\
Name varchar,LStat varchar,Md5 varchar,DeltaSeq smallint,Fhinfo NUMERIC(20),Fhnode NUMERIC(20))") {
                batch.errmsg = format!("error starting batch mode: {error}");
                self.errmsg = format!("Can't start batch mode: ERR={}", batch.errmsg);
                msgs.push((MsgType::Fatal, self.errmsg.clone()));
                return false;
            }
            self.batch_started = true;
        }
        // SplitPathAndFile(): everything up to the last '/' is the path.
        let split = ar.fname.iter().rposition(|c| *c == b'/').map(|i| i + 1).unwrap_or(ar.fname.len());
        let (path, name) = ar.fname.split_at(split);
        if path.is_empty() {
            msgs.push((MsgType::Error, format!("Path length is zero. File={}\n", String::from_utf8_lossy(name))));
        }
        let text = |b: &[u8]| esc(&String::from_utf8_lossy(b));
        let digest = ar.digest.as_deref().filter(|d| !d.is_empty()).unwrap_or("0");
        let batch = self.batch.as_mut().expect("batch connection");
        batch.pending.push(format!("({},{},'{}','{}','{}','{}',{},0,0)", ar.file_index, ar.job_id as i64,
            text(path), text(name), text(&ar.attr), esc(digest), ar.delta_seq));
        batch.changes += 1;
        if batch.pending.len() >= BATCH_CHUNK {
            if let Err(error) = batch.flush() {
                batch.errmsg = format!("error copying in batch mode: {error}");
            }
        }
        true
    }

    /// sql_create.cc WriteBatchFileRecords().
    fn write_batch_file_records(&mut self, msgs: &mut Messages) -> bool {
        if !self.batch_started {
            return true;
        }
        let Some(batch) = self.batch.as_mut() else { return true };
        msgs.push((MsgType::Info, format!("Insert of attributes batch table with {} entries start\n", batch.changes)));
        let mut ok = false;
        let query = |batch: &mut Batch, sql: &str| -> bool {
            match batch.db.exec(sql) {
                Ok(_) => true,
                Err(e) => {
                    batch.errmsg = format!("Query failed: {sql}: ERR={e}\n");
                    false
                }
            }
        };
        'run: {
            if let Err(error) = batch.flush() {
                batch.errmsg = format!("error ending batch mode: {error}");
                msgs.push((MsgType::Fatal, format!("Batch end {}\n", batch.errmsg)));
                break 'run;
            }
            if !query(batch, "BEGIN; LOCK TABLE Path IN SHARE ROW EXCLUSIVE MODE\n") {
                msgs.push((MsgType::Fatal, format!("Lock Path table {}\n", batch.errmsg)));
                break 'run;
            }
            if !query(batch, "INSERT INTO Path (Path)\nSELECT a.Path\n  FROM (\n      SELECT DISTINCT Path\n        FROM batch\n       ) AS a\n WHERE NOT EXISTS (\n      SELECT Path\n        FROM Path\n       WHERE Path = a.Path\n       )\n") {
                msgs.push((MsgType::Fatal, format!("Fill Path table {}\n", batch.errmsg)));
                query(batch, "COMMIT\n");
                break 'run;
            }
            if !query(batch, "COMMIT\n") {
                msgs.push((MsgType::Fatal, format!("Unlock Path table {}\n", batch.errmsg)));
                break 'run;
            }
            if !query(batch, "INSERT INTO File (FileIndex, JobId, PathId, Name, LStat, MD5, DeltaSeq, Fhinfo, Fhnode) \
SELECT batch.FileIndex, batch.JobId, Path.PathId, \
batch.Name, batch.LStat, batch.MD5, batch.DeltaSeq, batch.Fhinfo, batch.Fhnode \
FROM batch \
JOIN Path ON (batch.Path = Path.Path) ") {
                msgs.push((MsgType::Fatal, format!("Fill File table {}\n", batch.errmsg)));
                break 'run;
            }
            msgs.push((MsgType::Info, "Insert of attributes batch table done\n".into()));
            ok = true;
        }
        let _ = batch.db.exec("DROP TABLE IF EXISTS batch");
        batch.pending.clear();
        batch.changes = 0;
        self.batch_started = false;
        ok
    }
}

/// The UpdateMedia branch of CatalogRequest().
fn update_media(cat: &mut Cat, ctx: &JobContext, v: &[Scanned]) -> String {
    let sd = MediaRecord {
        volume_name: scanned_s(v, 1),
        vol_jobs: scanned_u(v, 2) as u32,
        vol_files: scanned_u(v, 3) as u32,
        vol_blocks: scanned_u(v, 4) as u32,
        vol_bytes: scanned_d(v, 5) as u64,
        vol_mounts: scanned_u(v, 6) as u32,
        vol_errors: scanned_u(v, 7) as u32,
        vol_writes: scanned_u(v, 8) as u32,
        max_vol_bytes: scanned_d(v, 9) as u64,
        vol_status: scanned_s(v, 11),
        slot: scanned_d(v, 12) as i32,
        in_changer: scanned_d(v, 14) as i32,
        vol_read_time: scanned_d(v, 15) as u64,
        vol_write_time: scanned_d(v, 16) as u64,
        ..Default::default()
    };
    let mut sd = sd;
    let vol_last_written = scanned_d(v, 10);
    let label = scanned_d(v, 13) as i32;
    let vol_first_written = scanned_d(v, 17);
    let mut mr = MediaRecord { volume_name: unbash_spaces(&sd.volume_name), ..Default::default() };
    if !get_media_record(cat, &mut mr) {
        let error = cat.errmsg.clone();
        cat.jmsg(MsgType::Error, format!("Unable to get Media record for Volume {}: ERR={error}\n", mr.volume_name));
        return format!("1991 Catalog Request for vol={} failed: {error}", mr.volume_name);
    }
    if mr.first_written == 0 {
        mr.first_written = if vol_first_written == 0 { ctx.start_time } else { vol_first_written };
        mr.set_first_written = true;
    }
    if label != 0 || mr.label_date == 0 {
        mr.label_date = ctx.start_time;
        mr.set_label_date = true;
        if mr.initial_write == 0 {
            mr.initial_write = ctx.start_time;
        }
    } else {
        // Sanity checks: VolFiles and VolBlocks (on a filemark) only grow.
        if sd.vol_files < mr.vol_files {
            cat.jmsg(MsgType::Info, format!("Ignoring Volume Files at {} being set to {} for Volume \"{}\".\n",
                mr.vol_files, sd.vol_files, mr.volume_name));
            sd.vol_files = mr.vol_files;
            sd.vol_blocks = mr.vol_blocks;
        } else if sd.vol_files == mr.vol_files && sd.vol_blocks < mr.vol_blocks {
            cat.jmsg(MsgType::Info, format!("Ignoring Volume Blocks at {} being set to {} for Volume \"{}\".\n",
                mr.vol_blocks, sd.vol_blocks, mr.volume_name));
            sd.vol_blocks = mr.vol_blocks;
        }
        if sd.vol_bytes < mr.vol_bytes {
            cat.jmsg(MsgType::Info, format!("Ignoring Volume Bytes at {} being set to {} for Volume \"{}\".\n",
                mr.vol_bytes, sd.vol_bytes, mr.volume_name));
            sd.vol_bytes = mr.vol_bytes;
        }
    }
    if mr.vol_blocks != sd.vol_blocks && vol_last_written != 0 {
        mr.last_written = vol_last_written;
    }
    if ctx.write_storage.is_some() && sd.vol_writes > mr.vol_writes {
        set_storage_id(ctx.write_storage.as_ref(), &mut mr);
    }
    mr.vol_jobs = sd.vol_jobs;
    mr.vol_files = sd.vol_files;
    mr.vol_blocks = sd.vol_blocks;
    mr.vol_bytes = sd.vol_bytes;
    mr.vol_mounts = sd.vol_mounts;
    mr.vol_errors = sd.vol_errors;
    mr.vol_writes = sd.vol_writes;
    mr.slot = sd.slot;
    mr.in_changer = sd.in_changer;
    mr.vol_status = sd.vol_status;
    mr.vol_read_time = sd.vol_read_time;
    mr.vol_write_time = sd.vol_write_time;
    if !update_media_record(cat, &mr) {
        let error = cat.errmsg.clone();
        cat.jmsg(MsgType::Fatal, format!("Catalog error updating Media record. {error}"));
        return "1993 Update Media error\n".into();
    }
    has_volume_expired(cat, &mut mr);
    mr.ok_media()
}

/// sql_create.cc CreateJobmediaRecord() for the CreateJobMedia request.
fn create_jobmedia_record(cat: &mut Cat, job_id: u32, v: &[Scanned]) -> bool {
    let (first_index, last_index) = (scanned_u(v, 1) as u32, scanned_u(v, 2) as u32);
    let (start_file, end_file) = (scanned_u(v, 3) as u32, scanned_u(v, 4) as u32);
    let (start_block, end_block) = (scanned_u(v, 5) as u32, scanned_u(v, 6) as u32);
    let media_id = scanned_d(v, 9);
    let mut count = cat.get_sql_record_max(&format!("SELECT count(*) from JobMedia WHERE JobId={job_id}")) as i32;
    if count < 0 {
        count = 0;
    }
    count += 1;
    let sql = format!("INSERT INTO JobMedia (JobId,MediaId,FirstIndex,LastIndex,\
StartFile,EndFile,StartBlock,EndBlock,VolIndex,JobBytes) \
VALUES ({job_id},{media_id},{first_index},{last_index},{start_file},{end_file},{start_block},{end_block},{},0)", count as u32);
    if cat.insert_db(&sql, "sql_create.cc:124") != 1 {
        let error = cat.errmsg.clone();
        *cat.errmsg = format!("Create JobMedia record {sql} failed: ERR={error}\n");
        return false;
    }
    let update = format!("UPDATE Media SET EndFile={end_file}, EndBlock={end_block} WHERE MediaId={media_id}");
    if cat.update_db(&update, "sql_create.cc:133") == -1 {
        let error = cat.errmsg.clone();
        *cat.errmsg = format!("Update Media record {update} failed: ERR={error}\n");
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// dird/next_vol.cc, recycle.cc, newvol.cc
// ---------------------------------------------------------------------------

/// next_vol.cc FindNextVolumeForAppend() with create and prune set (the
/// prune steps are not ported, see the module documentation).
fn find_next_volume_for_append(cat: &mut Cat, ctx: &JobContext, mr: &mut MediaRecord, index: i32, unwanted: &str) -> bool {
    let store = ctx.write_storage.as_ref();
    mr.media_type = store.map(|s| s.media_type.clone()).unwrap_or_default();
    let mut in_changer = store.is_some_and(|s| s.autochanger);
    let unwanted = Some(unwanted);
    let mut retry = 0;
    loop {
        set_storage_id(store, mr);
        mr.vol_status = "Append".into();
        let mut ok = find_next_volume(cat, index, in_changer, mr, unwanted) != 0;
        if !ok {
            mr.vol_status = "Unlabeled".into();
            ok = find_next_volume(cat, index, in_changer, mr, unwanted) != 0;
        }
        if !ok {
            mr.vol_status = "Append".into();
            // 3. a volume with status Recycle
            ok = find_recycled_volume(cat, in_changer, mr, store, unwanted);
            set_storage_id(store, mr);
            if !ok {
                // 4. the oldest Purged volume
                ok = recycle_oldest_purged_volume(cat, in_changer, mr, store, unwanted);
                set_storage_id(store, mr);
                if !ok {
                    // 5. PruneVolumes() is not ported.
                    ok = recycle_oldest_purged_volume(cat, in_changer, mr, store, unwanted);
                    set_storage_id(store, mr);
                    if !ok {
                        // 6. a volume of the Scratch pool
                        ok = get_scratch_volume(cat, ctx, in_changer, mr, store);
                        set_storage_id(store, mr);
                    }
                    if !ok && in_changer {
                        in_changer = false;
                        continue;
                    }
                }
            }
            if !ok {
                // 7. a new volume by LabelFormat
                ok = new_volume(cat, mr, store);
            }
            // 8./9. PurgeOldestVolume/RecycleOldestVolume are not ported.
        }
        if ok && mr.vol_status == "Append" && has_volume_expired(cat, mr) {
            let again = retry < 200;
            retry += 1;
            if again {
                continue;
            }
            cat.jmsg(MsgType::Error, "We seem to be looping trying to find the next volume. I give up.\n".into());
        }
        return ok;
    }
}

/// next_vol.cc HasVolumeExpired().
fn has_volume_expired(cat: &mut Cat, mr: &mut MediaRecord) -> bool {
    let mut expired = false;
    if mr.vol_status == "Append" && mr.vol_jobs > 0 {
        if mr.max_vol_bytes > 0 && mr.vol_bytes >= mr.max_vol_bytes {
            cat.jmsg(MsgType::Info, format!("Max Volume bytes={} exceeded. Marking Volume \"{}\" as Full.\n",
                with_commas(mr.max_vol_bytes), mr.volume_name));
            mr.vol_status = "Full".into();
            expired = true;
        } else if mr.max_vol_jobs > 0 && mr.max_vol_jobs <= mr.vol_jobs {
            cat.jmsg(MsgType::Info, format!("Max Volume jobs={} exceeded. Marking Volume \"{}\" as Used.\n",
                with_commas(mr.max_vol_jobs as u64), mr.volume_name));
            mr.vol_status = "Used".into();
            expired = true;
        } else if mr.max_vol_files > 0 && mr.max_vol_files <= mr.vol_files {
            cat.jmsg(MsgType::Info, format!("Max Volume files={} exceeded. Marking Volume \"{}\" as Used.\n",
                with_commas(mr.max_vol_files as u64), mr.volume_name));
            mr.vol_status = "Used".into();
            expired = true;
        } else if mr.vol_use_duration > 0 && mr.vol_use_duration <= now() - mr.first_written {
            cat.jmsg(MsgType::Info, format!("Max configured use duration={} sec. exceeded. Marking Volume \"{}\" as Used.\n",
                with_commas(mr.vol_use_duration as u64), mr.volume_name));
            mr.vol_status = "Used".into();
            expired = true;
        }
    }
    if expired && !update_media_record(cat, mr) {
        let error = cat.errmsg.clone();
        cat.jmsg(MsgType::Error, format!("Catalog error updating volume \"{}\". ERR={error}\n", mr.volume_name));
    }
    expired
}

/// next_vol.cc CheckIfVolumeValidOrRecyclable(): None or the reason.
fn check_if_volume_valid_or_recyclable(cat: &mut Cat, mr: &mut MediaRecord) -> Option<String> {
    if mr.vol_status == "Append" && has_volume_expired(cat, mr) {
        // keep going, the volume may be recyclable
    }
    if matches!(mr.vol_status.as_str(), "Unlabeled" | "Append" | "Recycle") {
        return None;
    }
    if mr.vol_status == "Purged" {
        if recycle_volume(cat, mr) {
            cat.jmsg(MsgType::Info, format!("Recycled current volume \"{}\"\n", mr.volume_name));
            return None;
        }
        return Some("and recycling of current volume failed".into());
    }
    if mr.recycle == 0 {
        return Some("volume has recycling disabled".into());
    }
    // Pruning the current volume (Recycle Current Volume) is not ported.
    Some("but should be Append, Purged or Recycle".into())
}

/// recycle.cc FindRecycledVolume().
fn find_recycled_volume(cat: &mut Cat, in_changer: bool, mr: &mut MediaRecord, store: Option<&WriteStorage>, unwanted: Option<&str>) -> bool {
    mr.vol_status = "Recycle".into();
    set_storage_id(store, mr);
    if find_next_volume(cat, 1, in_changer, mr, unwanted) != 0 {
        set_storage_id(store, mr);
        return true;
    }
    false
}

/// recycle.cc RecycleOldestPurgedVolume().
fn recycle_oldest_purged_volume(cat: &mut Cat, in_changer: bool, mr: &mut MediaRecord, store: Option<&WriteStorage>, unwanted: Option<&str>) -> bool {
    mr.vol_status = "Purged".into();
    set_storage_id(store, mr);
    if find_next_volume(cat, 1, in_changer, mr, unwanted) != 0 {
        set_storage_id(store, mr);
        if recycle_volume(cat, mr) {
            cat.jmsg(MsgType::Info, format!("Recycled volume \"{}\"\n", mr.volume_name));
            return true;
        }
    }
    false
}

/// recycle.cc RecycleVolume().
fn recycle_volume(cat: &mut Cat, mr: &mut MediaRecord) -> bool {
    mr.vol_status = "Recycle".into();
    mr.vol_jobs = 0;
    mr.vol_files = 0;
    mr.vol_blocks = 0;
    mr.vol_errors = 0;
    mr.vol_bytes = 1;
    mr.first_written = 0;
    mr.last_written = 0;
    mr.recycle_count = mr.recycle_count.wrapping_add(1);
    mr.set_first_written = true;
    update_media_record(cat, mr)
}

/// next_vol.cc GetScratchVolume().
fn get_scratch_volume(cat: &mut Cat, ctx: &JobContext, in_changer: bool, mr: &mut MediaRecord, store: Option<&WriteStorage>) -> bool {
    let mut spr = PoolRecord { name: "Scratch".into(), pool_id: mr.scratch_pool_id, ..Default::default() };
    if !get_pool_record(cat, &mut spr) {
        return false;
    }
    let mut smr = MediaRecord { pool_id: spr.pool_id, vol_status: "Append".into(), media_type: mr.media_type.clone(), ..Default::default() };
    if in_changer {
        smr.storage_id = mr.storage_id;
    }
    set_storage_id(store, &mut smr);
    let found = find_next_volume(cat, 1, in_changer, &mut smr, None) != 0
        || find_recycled_volume(cat, in_changer, &mut smr, store, None)
        || recycle_oldest_purged_volume(cat, in_changer, &mut smr, store, None);
    if !found {
        return false;
    }
    let mut pr = PoolRecord { name: ctx.pool_name.clone(), ..Default::default() };
    if !get_pool_record(cat, &mut pr) {
        let error = cat.errmsg.clone();
        cat.jmsg(MsgType::Warning, format!("Unable to get Pool record: ERR={error}\n"));
        return false;
    }
    if pr.max_vols > 0 && pr.num_vols >= pr.max_vols {
        cat.jmsg(MsgType::Warning, format!("Unable add Scratch Volume, Pool \"{}\" full MaxVols={}\n", ctx.pool_name, pr.max_vols as i32));
        return false;
    }
    *mr = smr.clone();
    set_storage_id(store, mr);
    set_pool_defaults(mr, &pr);
    mr.vol_status = smr.vol_status.clone();
    mr.recycle_pool_id = smr.recycle_pool_id;
    if !update_media_record(cat, mr) {
        let error = cat.errmsg.clone();
        cat.jmsg(MsgType::Warning, format!("Failed to move Scratch Volume. ERR={error}\n"));
        return false;
    }
    cat.jmsg(MsgType::Info, format!("Using Volume \"{}\" from 'Scratch' pool.\n", mr.volume_name));
    true
}

/// newvol.cc newVolume() (simple LabelFormat names only).
fn new_volume(cat: &mut Cat, mr: &mut MediaRecord, store: Option<&WriteStorage>) -> bool {
    let mut pr = PoolRecord { pool_id: mr.pool_id, ..Default::default() };
    if !get_pool_record(cat, &mut pr) {
        return false;
    }
    if !(pr.max_vols == 0 || pr.num_vols < pr.max_vols) {
        return false;
    }
    *mr = MediaRecord::default();
    set_pool_defaults(mr, &pr);
    mr.media_type = store.map(|s| s.media_type.clone()).unwrap_or_default();
    if pr.label_format.is_empty() || pr.label_format.starts_with('*') {
        return false;
    }
    if !is_volume_name_legal(&pr.label_format) {
        // PerformFullNameSubstitution() is not ported.
        return false;
    }
    if !create_simple_name(cat, mr, &pr) {
        return false;
    }
    pr.num_vols += 1;
    mr.enabled = VOL_ENABLED;
    set_storage_id(store, mr);
    if create_media_record(cat, mr) && update_pool_record(cat, &mut pr) {
        cat.jmsg(MsgType::Info, format!("Created new Volume \"{}\" in catalog.\n", mr.volume_name));
        return true;
    }
    let error = cat.errmsg.clone();
    cat.jmsg(MsgType::Error, error);
    false
}

/// newvol.cc CreateSimpleName().
fn create_simple_name(cat: &mut Cat, mr: &mut MediaRecord, pr: &PoolRecord) -> bool {
    mr.volume_name.clear();
    let sql = format!("SELECT MAX(MediaId) FROM Media,Pool WHERE Pool.PoolId={}", pr.pool_id);
    let value: i64 = match cat.db.rows(&sql) {
        Ok(t) => t.rows.iter().filter_map(|r| r.first().cloned().flatten()).last()
            .map(|v| str_to_int64(v.as_bytes())).unwrap_or(0),
        Err(e) => {
            *cat.errmsg = format!("Query failed: {sql}: ERR={e}\n");
            let error = cat.errmsg.clone();
            cat.jmsg(MsgType::Warning, format!("SQL failed, but ignored. ERR={error}\n"));
            pr.num_vols as i64 + 1
        }
    };
    let value = value as i32;
    for i in value.wrapping_add(1)..value.wrapping_add(100) {
        let name = format!("{}{i:04}", pr.label_format);
        let mut tmr = MediaRecord { volume_name: name.clone(), ..Default::default() };
        if get_media_record(cat, &mut tmr) {
            cat.jmsg(MsgType::Warning, format!("Wanted to create Volume \"{name}\", but it already exists. Trying again.\n"));
            continue;
        }
        mr.volume_name = name;
        break;
    }
    if mr.volume_name.is_empty() {
        cat.jmsg(MsgType::Error, "Too many failures. Giving up creating Volume name.\n".into());
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Catalog handle as Db
// ---------------------------------------------------------------------------

impl Db for CatalogHandle {
    fn rows(&self, sql: &str) -> Result<SqlTable, String> {
        self.query_rows(sql)
    }
    fn exec(&self, sql: &str) -> Result<u64, String> {
        self.execute(sql)
    }
}

#[cfg(test)]
mod tests;
