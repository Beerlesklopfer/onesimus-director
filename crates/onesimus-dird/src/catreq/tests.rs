//! Requests in the exact stored/askdir.cc formats against a catalog double
//! (and, with ONESIMUS_CATREQ_DB set, against a real throw-away catalog).

use super::*;
use std::sync::{Arc, Mutex};

type RowsFn = dyn Fn(&str) -> Option<Result<Vec<Vec<Option<String>>>, String>> + Send + Sync;
type ExecFn = dyn Fn(&str) -> Result<u64, String> + Send + Sync;

/// Catalog double: answers by SQL text, logs every statement.
#[derive(Clone)]
struct MockDb {
    log: Arc<Mutex<Vec<String>>>,
    rows: Arc<RowsFn>,
    exec: Arc<ExecFn>,
}

impl MockDb {
    fn new(rows: impl Fn(&str) -> Option<Result<Vec<Vec<Option<String>>>, String>> + Send + Sync + 'static) -> Self {
        Self { log: Arc::default(), rows: Arc::new(rows), exec: Arc::new(|_| Ok(1)) }
    }
    fn with_exec(mut self, exec: impl Fn(&str) -> Result<u64, String> + Send + Sync + 'static) -> Self {
        self.exec = Arc::new(exec);
        self
    }
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Db for MockDb {
    fn rows(&self, sql: &str) -> Result<SqlTable, String> {
        self.log.lock().unwrap().push(sql.to_string());
        match (self.rows)(sql) {
            Some(Ok(rows)) => Ok(SqlTable { columns: Vec::new(), rows }),
            Some(Err(e)) => Err(e),
            None => Ok(SqlTable::default()),
        }
    }
    fn exec(&self, sql: &str) -> Result<u64, String> {
        self.log.lock().unwrap().push(sql.to_string());
        (self.exec)(sql)
    }
}

fn row(values: &[&str]) -> Vec<Option<String>> {
    values.iter().map(|v| Some(v.to_string())).collect()
}

/// A Media row in MEDIA_COLUMNS order.
fn media(id: i64, name: &str, status: &str, pool: i64, files: u32, blocks: u32, bytes: u64, writes: u32) -> Vec<Option<String>> {
    let mut r = vec![Some("0".to_string()); 40];
    let set = |r: &mut Vec<Option<String>>, i: usize, v: String| r[i] = Some(v);
    set(&mut r, 0, id.to_string());
    set(&mut r, 1, name.into());
    set(&mut r, 2, "1".into());
    set(&mut r, 3, files.to_string());
    set(&mut r, 4, blocks.to_string());
    set(&mut r, 5, bytes.to_string());
    set(&mut r, 6, "2".into());
    set(&mut r, 8, writes.to_string());
    set(&mut r, 9, "53687091200".into());
    set(&mut r, 11, "File".into());
    set(&mut r, 12, status.into());
    set(&mut r, 13, pool.to_string());
    set(&mut r, 14, "31536000".into());
    set(&mut r, 18, "1".into());
    r[20] = Some("2026-06-11 21:00:02".into());
    r[21] = Some("2026-06-12 21:00:02".into());
    r[26] = Some("2026-06-11 21:00:02".into());
    set(&mut r, 27, "1".into());
    set(&mut r, 28, "1".into());
    r[31] = None;
    r[37] = None;
    r
}

fn pool_row(id: i64, name: &str, num_vols: u32, max_vols: u32, label_format: &str) -> Vec<Option<String>> {
    row(&[&id.to_string(), name, &num_vols.to_string(), &max_vols.to_string(), "0", "1", "0", "1", "1",
        "31536000", "0", "1", "0", "53687091200", "Backup", "0", label_format, "0", "0", "0", "0", "0"])
}

fn context() -> JobContext {
    JobContext {
        job_id: 42, start_time: 1_781_000_000, pool_id: 3, pool_name: "Full".into(), catalog_files: true,
        write_storage: Some(WriteStorage { media_type: "File".into(), storage_id: 1, autochanger: false }),
    }
}

fn no_batch() -> Option<Box<dyn Db + Send>> {
    None
}

fn run(db: &MockDb, msg: &str) -> Reply {
    CatReqJob::new(context()).dispatch(Some(db), &no_batch, msg.as_bytes())
}

// --- bsscanf ---------------------------------------------------------------

#[test]
fn bsscanf_like_lib_scan() {
    let f = "CatReq Job=%127s FindMedia=%d pool_name=%127s media_type=%127s unwanted_volumes=%s\n";
    // First FindMedia of a job: empty unwanted list still counts (C sscanf: 4).
    assert_eq!(bsscanf(b"CatReq Job=a.b FindMedia=1 pool_name=Full media_type=File unwanted_volumes=\n", f).0, 5);
    let (n, v) = bsscanf(b"CatReq Job=a.b FindMedia=2 pool_name=Full media_type=File unwanted_volumes=Full-0001,Full-0002\n", f);
    assert_eq!((n, scanned_d(&v, 1), scanned_s(&v, 4)), (5, 2, "Full-0001,Full-0002".to_string()));
    // Mismatch of literal text: -1.
    assert_eq!(bsscanf(b"CatReq Job=a GetVolInfo VolName=x write=1\n", f).0, -1);
    // %10s stops after 10 characters; the following literal then mismatches.
    assert_eq!(bsscanf(b"S=Appendxxxxxxxxx X", "S=%10s X").0, -1);
    // %u of a 33-bit value is narrowed to 32 bits, %lu too (PRIu64 is "lu").
    assert_eq!(scanned_u(&bsscanf(b"x 4294967297", "x %u").1, 0), 1);
    assert_eq!(scanned_u(&bsscanf(b"x 4294967297", "x %lu").1, 0), 1);
    assert_eq!(scanned_d(&bsscanf(b"x -4294967297", "x %lld").1, 0), -4294967297);
    // Digits are optional for %u/%d.
    assert_eq!(bsscanf(b"x= y", "x=%u y").0, 1);
    assert_eq!(bsscanf(b"Jmsg Job=abc type=1", "%020s Job=%127s ").0, 2);
}

#[test]
fn base64_like_bareos() {
    assert_eq!(bin_to_base64(&[0u8; 16]), "AAAAAAAAAAAAAAAAAAAAAA");
    assert_eq!(bin_to_base64(&[0xff, 0x00, 0x10]), "/wAQ");
    assert_eq!(bin_to_base64(&[0xd4, 0x1d, 0x8c, 0xd9]), "1B2M2Q");
}

// --- getmsg dispatch, invalid requests --------------------------------------

#[test]
fn malformed_invalid_and_no_catalog() {
    let db = MockDb::new(|_| None);
    let reply = run(&db, "CatReq\n");
    assert_eq!(reply.to_sd, "");
    assert_eq!(reply.messages, vec![(MsgType::Error, "Malformed message: CatReq\n\n".to_string())]);

    let reply = run(&db, "CatReq Job=j.1 Unknown x=1\n");
    assert_eq!(reply.to_sd, "1990 Invalid Catalog Request: CatReq Job=j.1 Unknown x=1\n");
    assert_eq!(reply.messages, vec![(MsgType::Fatal, "Invalid Catalog request: CatReq Job=j.1 Unknown x=1\n".to_string())]);

    let reply = CatReqJob::new(context()).dispatch(None, &no_batch, b"CatReq Job=j.1 GetVolInfo VolName=a write=0\n");
    assert_eq!(reply.to_sd, "1990 Invalid Catalog Request: CatReq Job=j.1 GetVolInfo VolName=a write=0\n");
    assert_eq!(reply.messages[0].1, "Invalid Catalog request; DB not open: CatReq Job=j.1 GetVolInfo VolName=a write=0\n");
}

// --- FindMedia ----------------------------------------------------------------

#[test]
fn find_media_answers_ok_media_of_the_append_volume() {
    let db = MockDb::new(|sql| {
        if sql.contains("FROM Pool WHERE Pool.Name='Full'") {
            Some(Ok(vec![pool_row(3, "Full", 2, 100, "Full-")]))
        } else if sql.starts_with("SELECT count(*) from Media WHERE PoolId=3") {
            Some(Ok(vec![row(&["2"])]))
        } else if sql.contains("VolStatus='Append'") {
            assert!(sql.contains("FROM Media WHERE PoolId=3 AND MediaType='File' AND Enabled=1 AND VolStatus='Append'  \
ORDER BY LastWritten IS NULL,LastWritten DESC,MediaId\n LIMIT 1"), "{sql}");
            Some(Ok(vec![media(7, "Full-0007", "Append", 3, 0, 0, 1, 0)]))
        } else {
            None
        }
    });
    let reply = run(&db, "CatReq Job=backup.2026 FindMedia=1 pool_name=Full media_type=File unwanted_volumes=\n");
    assert_eq!(reply.to_sd, "1000 OK VolName=Full-0007 VolJobs=1 VolFiles=0 VolBlocks=0 VolBytes=1 VolMounts=2 \
VolErrors=0 VolWrites=0 MaxVolBytes=53687091200 VolCapacityBytes=0 VolStatus=Append Slot=0 MaxVolJobs=0 \
MaxVolFiles=0 InChanger=0 VolReadTime=0 VolWriteTime=0 EndFile=0 EndBlock=0 LabelType=0 MediaId=7 \
EncryptionKey= MinBlocksize=0 MaxBlocksize=0\n");
    assert!(reply.messages.is_empty());
}

#[test]
fn find_media_skips_unwanted_and_creates_a_new_volume() {
    let db = MockDb::new(|sql| {
        if sql.contains("FROM Pool WHERE Pool.Name='Full'") || sql.contains("FROM Pool WHERE Pool.PoolId=3") {
            Some(Ok(vec![pool_row(3, "Full", 1, 100, "Full-")]))
        } else if sql.starts_with("SELECT count(*) from Media WHERE PoolId=3") {
            Some(Ok(vec![row(&["1"])]))
        } else if sql.contains("VolStatus='Append'") {
            Some(Ok(vec![media(7, "Full-0007", "Append", 3, 0, 0, 1, 0)]))
        } else if sql.starts_with("SELECT MAX(MediaId) FROM Media,Pool WHERE Pool.PoolId=3") {
            Some(Ok(vec![row(&["7"])]))
        } else if sql.contains("WHERE VolumeName='Full-0008'") {
            Some(Ok(vec![media(8, "Full-0008", "Full", 3, 0, 0, 1, 0)]))
        } else if sql.starts_with("INSERT INTO Media") {
            Some(Ok(vec![row(&["9"])]))
        } else {
            None
        }
    });
    let reply = run(&db, "CatReq Job=backup.2026 FindMedia=1 pool_name=Full media_type=File unwanted_volumes=Full-0007\n");
    assert!(reply.to_sd.starts_with("1000 OK VolName=Full-0009 VolJobs=0 VolFiles=0 VolBlocks=0 VolBytes=0 "), "{}", reply.to_sd);
    assert!(reply.to_sd.contains(" VolStatus=Unlabeled Slot=0 MaxVolJobs=1 "), "{}", reply.to_sd);
    assert!(reply.to_sd.ends_with(" MediaId=9 EncryptionKey= MinBlocksize=0 MaxBlocksize=0\n"));
    assert_eq!(reply.messages, vec![
        (MsgType::Warning, "Wanted to create Volume \"Full-0008\", but it already exists. Trying again.\n".to_string()),
        (MsgType::Info, "Created new Volume \"Full-0009\" in catalog.\n".to_string()),
    ]);
    let log = db.log();
    let insert = log.iter().find(|s| s.starts_with("INSERT INTO Media")).unwrap();
    assert_eq!(insert, "INSERT INTO Media (VolumeName,MediaType,MediaTypeId,PoolId,MaxVolBytes,VolCapacityBytes,\
Recycle,VolRetention,VolUseDuration,MaxVolJobs,MaxVolFiles,VolStatus,Slot,VolBytes,InChanger,VolReadTime,\
VolWriteTime,EndFile,EndBlock,LabelType,StorageId,DeviceId,LocationId,ScratchPoolId,RecyclePoolId,Enabled,\
ActionOnPurge,EncryptionKey,MinBlocksize,MaxBlocksize,VolFiles) VALUES ('Full-0009','File',0,3,53687091200,0,1,\
31536000,0,1,0,'Unlabeled',0,0,0,0,0,0,0,0,1,0,0,0,0,1,0,'',0,0,0); SELECT currval('Media_Mediaid_seq')");
    assert!(log.iter().any(|s| s.starts_with("UPDATE Pool SET NumVols=1,MaxVols=100,")));
}

#[test]
fn find_media_without_volume_is_no_media() {
    let db = MockDb::new(|sql| {
        if sql.contains("FROM Pool WHERE") {
            Some(Ok(vec![pool_row(3, "Full", 1, 1, "Full-")]))
        } else if sql.starts_with("SELECT count(*) from Media WHERE PoolId=3") {
            Some(Ok(vec![row(&["1"])]))
        } else {
            None
        }
    });
    let reply = run(&db, "CatReq Job=b FindMedia=1 pool_name=Full media_type=File unwanted_volumes=\n");
    assert_eq!(reply.to_sd, "1901 No Media.\n");
}

// --- GetVolInfo ---------------------------------------------------------------

#[test]
fn get_vol_info_checks_pool_media_type_and_enabled() {
    let db = MockDb::new(|sql| {
        if sql.ends_with("WHERE VolumeName='Other Pool'") {
            Some(Ok(vec![media(5, "Other Pool", "Append", 2, 0, 0, 1, 0)]))
        } else if sql.ends_with("WHERE VolumeName='Full-0001'") {
            let mut r = media(1, "Full-0001", "Full", 3, 0, 0, 1, 0);
            r[18] = Some("0".into());
            Some(Ok(vec![r]))
        } else {
            None
        }
    });
    // Spaces are bashed (0x01) on the wire.
    assert_eq!(run(&db, "CatReq Job=b GetVolInfo VolName=Other\x01Pool write=1\n").to_sd,
        "1998 Volume \"Other Pool\" catalog status is Append, not in Pool.\n");
    assert!(run(&db, "CatReq Job=b GetVolInfo VolName=Other\x01Pool write=0\n").to_sd
        .starts_with("1000 OK VolName=Other\x01Pool VolJobs=1 "));
    assert_eq!(run(&db, "CatReq Job=b GetVolInfo VolName=Full-0001 write=1\n").to_sd,
        "1998 Volume \"Full-0001\" catalog status is Full, volume has recycling disabled.\n");
    assert_eq!(run(&db, "CatReq Job=b GetVolInfo VolName=Nope write=0\n").to_sd,
        "1997 Volume \"Nope\" not in catalog.\n");
}

#[test]
fn get_vol_info_recycles_a_purged_volume() {
    let db = MockDb::new(|sql| sql.ends_with("WHERE VolumeName='Full-0002'")
        .then(|| Ok(vec![media(2, "Full-0002", "Purged", 3, 4, 9, 100, 1)])));
    let reply = run(&db, "CatReq Job=b GetVolInfo VolName=Full-0002 write=1\n");
    assert!(reply.to_sd.starts_with("1000 OK VolName=Full-0002 VolJobs=0 VolFiles=0 VolBlocks=0 VolBytes=1 "), "{}", reply.to_sd);
    assert!(reply.to_sd.contains(" VolStatus=Recycle "));
    assert_eq!(reply.messages, vec![(MsgType::Info, "Recycled current volume \"Full-0002\"\n".to_string())]);
    assert!(db.log().iter().any(|s| s.starts_with("UPDATE Media SET VolJobs=0,VolFiles=0,VolBlocks=0,VolBytes=1,")
        && s.contains("VolStatus='Recycle'") && s.contains("RecycleCount=1,")));
}

// --- UpdateMedia (catreq.cc:258-280) -------------------------------------------

/// stored/askdir.cc Update_media.
fn update_media_msg(files: u32, blocks: u32, bytes: u64, writes: u32, relabel: i32) -> String {
    format!("CatReq Job=backup.2026 UpdateMedia VolName=Full-0007 VolJobs=2 VolFiles={files} VolBlocks={blocks} \
VolBytes={bytes} VolMounts=3 VolErrors=0 VolWrites={writes} MaxVolBytes=53687091200 EndTime=1781000100 \
VolStatus=Append Slot=0 relabel={relabel} InChanger=0 VolReadTime=0 VolWriteTime=12 VolFirstWritten=0\n")
}

fn update_media_db() -> MockDb {
    MockDb::new(|sql| sql.ends_with("WHERE VolumeName='Full-0007'")
        .then(|| Ok(vec![media(7, "Full-0007", "Append", 3, 5, 500, 10_000, 4)])))
}

fn main_update(db: &MockDb) -> String {
    db.log().into_iter().find(|s| s.starts_with("UPDATE Media SET VolJobs=")).unwrap()
}

#[test]
fn update_media_volfiles_backwards_resets_volblocks() {
    let db = update_media_db();
    let reply = run(&db, &update_media_msg(3, 900, 20_000, 5, 0));
    assert_eq!(reply.messages, vec![(MsgType::Info,
        "Ignoring Volume Files at 5 being set to 3 for Volume \"Full-0007\".\n".to_string())]);
    let sql = main_update(&db);
    assert!(sql.contains("VolFiles=5,VolBlocks=500,VolBytes=20000,"), "{sql}");
    assert!(reply.to_sd.starts_with("1000 OK VolName=Full-0007 VolJobs=2 VolFiles=5 VolBlocks=500 VolBytes=20000 VolMounts=3 "));
    // VolBlocks unchanged: LastWritten keeps the catalog value.
    assert!(db.log().iter().any(|s| s == "UPDATE Media Set LastWritten='2026-06-12 21:00:02' WHERE VolumeName='Full-0007'"));
}

#[test]
fn update_media_ignores_volblocks_only_when_volfiles_unchanged() {
    let db = update_media_db();
    let reply = run(&db, &update_media_msg(5, 400, 20_000, 5, 0));
    assert_eq!(reply.messages, vec![(MsgType::Info,
        "Ignoring Volume Blocks at 500 being set to 400 for Volume \"Full-0007\".\n".to_string())]);
    assert!(main_update(&db).contains("VolFiles=5,VolBlocks=500,"));

    // More files: fewer blocks are accepted (new file mark).
    let db = update_media_db();
    let reply = run(&db, &update_media_msg(6, 10, 20_000, 5, 0));
    assert!(reply.messages.is_empty());
    let sql = main_update(&db);
    assert!(sql.contains("VolFiles=6,VolBlocks=10,"), "{sql}");
    assert!(db.log().iter().any(|s| s == &format!("UPDATE Media Set LastWritten='{}' WHERE VolumeName='Full-0007'",
        bstrutime(1_781_000_100))));
}

#[test]
fn update_media_bytes_backwards_and_storage_id() {
    let db = update_media_db();
    let reply = run(&db, &update_media_msg(5, 500, 9_000, 4, 0));
    assert_eq!(reply.messages, vec![(MsgType::Info,
        "Ignoring Volume Bytes at 10000 being set to 9000 for Volume \"Full-0007\".\n".to_string())]);
    let sql = main_update(&db);
    assert!(sql.contains("VolBytes=10000,") && sql.contains("VolWrites=4,") && sql.contains(",StorageId=1,"), "{sql}");
    assert!(sql.ends_with("EncryptionKey='',MinBlocksize=0,MaxBlocksize=0 WHERE VolumeName='Full-0007'"), "{sql}");
}

#[test]
fn update_media_label_sets_label_date_and_skips_checks() {
    let db = update_media_db();
    let reply = run(&db, &update_media_msg(0, 0, 200, 1, 1));
    assert!(reply.messages.is_empty());
    assert!(db.log().iter().any(|s| s == &format!("UPDATE Media SET LabelDate='{}' WHERE VolumeName='Full-0007'",
        bstrutime(1_781_000_000))));
    assert!(main_update(&db).contains("VolFiles=0,VolBlocks=0,VolBytes=200,"));
}

#[test]
fn update_media_errors() {
    let db = MockDb::new(|_| None);
    let reply = run(&db, &update_media_msg(1, 1, 1, 1, 0));
    assert_eq!(reply.to_sd, "1991 Catalog Request for vol=Full-0007 failed: Media record for Volume \"Full-0007\" not found.\n");
    assert_eq!(reply.messages, vec![(MsgType::Error,
        "Unable to get Media record for Volume Full-0007: ERR=Media record for Volume \"Full-0007\" not found.\n\n".to_string())]);

    let db = update_media_db().with_exec(|sql| if sql.starts_with("UPDATE Media SET VolJobs=") { Ok(0) } else { Ok(1) });
    let reply = run(&db, &update_media_msg(5, 500, 20_000, 5, 0));
    assert_eq!(reply.to_sd, "1993 Update Media error\n");
    assert_eq!(reply.messages.last().unwrap().0, MsgType::Fatal);
}

#[test]
fn update_media_marks_full_volume() {
    let db = MockDb::new(|sql| sql.ends_with("WHERE VolumeName='Full-0007'").then(|| {
        let mut r = media(7, "Full-0007", "Append", 3, 5, 500, 10_000, 4);
        r[9] = Some("20000".into());
        Ok(vec![r])
    }));
    let reply = run(&db, &update_media_msg(5, 600, 20_000, 5, 0));
    assert_eq!(reply.messages, vec![(MsgType::Info,
        "Max Volume bytes=20,000 exceeded. Marking Volume \"Full-0007\" as Full.\n".to_string())]);
    assert!(reply.to_sd.contains(" VolStatus=Full "));
}

// --- JobMedia, job record, null JobMedia ----------------------------------------

#[test]
fn create_job_media() {
    let db = MockDb::new(|sql| sql.starts_with("SELECT count(*) from JobMedia WHERE JobId=42").then(|| Ok(vec![row(&["2"])])));
    let reply = run(&db, "CatReq Job=backup.2026 CreateJobMedia FirstIndex=1 LastIndex=12 StartFile=0 EndFile=0 \
StartBlock=226 EndBlock=7002 Copy=0 Strip=0 MediaId=7\n");
    assert_eq!(reply.to_sd, "1000 OK CreateJobMedia\n");
    assert_eq!(db.log(), vec![
        "SELECT count(*) from JobMedia WHERE JobId=42".to_string(),
        "INSERT INTO JobMedia (JobId,MediaId,FirstIndex,LastIndex,StartFile,EndFile,StartBlock,EndBlock,VolIndex,JobBytes) \
VALUES (42,7,1,12,0,0,226,7002,3,0)".to_string(),
        "UPDATE Media SET EndFile=0, EndBlock=7002 WHERE MediaId=7".to_string(),
    ]);

    let db = MockDb::new(|_| None).with_exec(|_| Err("ERROR:  boom\n".into()));
    let reply = run(&db, "CatReq Job=b CreateJobMedia FirstIndex=0 LastIndex=0 StartFile=0 EndFile=0 StartBlock=0 EndBlock=0 Copy=0 Strip=0 MediaId=7\n");
    assert_eq!(reply.to_sd, "1992 Create JobMedia error\n");
    assert!(reply.messages.last().unwrap().1.starts_with("Catalog error creating JobMedia record. Create JobMedia record INSERT INTO JobMedia"));
}

#[test]
fn update_job_record_answers_only_on_error() {
    let db = MockDb::new(|_| None);
    let reply = run(&db, "Catreq Job=backup.2026 UpdateJobRecord JobFiles=10 JobBytes=4294967306\n");
    assert_eq!((reply.to_sd.as_str(), reply.job_counts), ("", Some((10, 10))));
    assert_eq!(db.log(), vec!["UPDATE Job SET JobBytes=10,JobFiles=10 WHERE JobId=42".to_string()]);

    let db = MockDb::new(|_| None).with_exec(|_| Ok(0));
    let reply = run(&db, "Catreq Job=backup.2026 UpdateJobRecord JobFiles=10 JobBytes=1\n");
    assert_eq!(reply.to_sd, "1992 Update job record error\n");
    assert_eq!(reply.messages, vec![(MsgType::Fatal, "Catalog error updating Job record. \n".to_string())]);
}

#[test]
fn delete_null_jobmedia_and_update_file_list() {
    let db = MockDb::new(|_| None).with_exec(|_| Ok(0));
    let reply = run(&db, "CatReq Job=b DeleteNullJobmediaRecords jobid=17");
    assert_eq!(reply.to_sd, "1000 OK DeleteNullJobmediaRecords\n");
    assert_eq!(db.log(), vec!["DELETE FROM jobmedia WHERE jobid=17 AND firstindex=0 AND lastindex=0".to_string()]);
    let db = MockDb::new(|_| None).with_exec(|_| Err("x".into()));
    assert_eq!(run(&db, "CatReq Job=b DeleteNullJobmediaRecords jobid=17").to_sd, "1992 DeleteNullJobMediaRecords error\n");
    // No batch connection yet: nothing to do, no answer.
    let db = MockDb::new(|_| None);
    let reply = run(&db, "Catreq Job=b UpdateFileList\n");
    assert_eq!(reply, Reply::default());
}

// --- UpdCat attributes and the batch insert --------------------------------------

/// stored/askdir.cc FileAttributes + the serialized record header.
fn upd_cat(file_index: i32, stream: i32, record: &[u8]) -> Vec<u8> {
    let mut m = b"UpdCat Job=backup.2026 FileAttributes ".to_vec();
    for v in [1u32, 1_781_000_000, file_index as u32, stream as u32, record.len() as u32] {
        m.extend_from_slice(&v.to_be_bytes());
    }
    m.extend_from_slice(record);
    m
}

#[test]
fn attributes_go_through_the_batch_table() {
    let catalog = MockDb::new(|_| None);
    let batch = MockDb::new(|_| None);
    let opener_db = batch.clone();
    let opener = move || Some(Box::new(opener_db.clone()) as Box<dyn Db + Send>);
    let mut job = CatReqJob::new(context());
    let reg = b"1 3 /etc/hosts\0P0A CQOE IGk B A A A Ew BAA I BoX BoX BoX A A C\0\0\0" as &[u8];
    assert_eq!(job.dispatch(Some(&catalog), &opener, &upd_cat(1, STREAM_UNIX_ATTRIBUTES, reg)), Reply::default());
    // The digest completes the cached attribute.
    let md5 = [0xd4, 0x1d, 0x8c, 0xd9, 0x8f, 0x00, 0xb2, 0x04, 0xe9, 0x80, 0x09, 0x98, 0xec, 0xf8, 0x42, 0x7e];
    assert_eq!(job.dispatch(Some(&catalog), &opener, &upd_cat(1, STREAM_MD5_DIGEST, &md5)), Reply::default());
    let dir = b"2 5 /etc/\0P0A BAA IH4 C A A A BAA BAA BoX BoX BoX A A C\0\0" as &[u8];
    job.dispatch(Some(&catalog), &opener, &upd_cat(2, STREAM_UNIX_ATTRIBUTES, dir));
    // A digest for another file index.
    let reply = job.dispatch(Some(&catalog), &opener, &upd_cat(9, STREAM_SHA1_DIGEST, &[0; 20]));
    assert_eq!(reply.messages, vec![(MsgType::Warning, "SHA1 digest not same File=9 as attributes=2\n".to_string())]);
    let messages = job.end_of_job(Some(&catalog), &opener);
    assert_eq!(messages, vec![
        (MsgType::Info, "Insert of attributes batch table with 2 entries start\n".to_string()),
        (MsgType::Info, "Insert of attributes batch table done\n".to_string()),
    ]);
    let log = batch.log();
    assert!(log[0].starts_with("CREATE TEMPORARY TABLE batch (FileIndex int,JobId int,Path varchar,"));
    assert_eq!(log[1], "INSERT INTO batch VALUES (1,42,'/etc/','hosts','P0A CQOE IGk B A A A Ew BAA I BoX BoX BoX A A C',\
'1B2M2Y8AsgTpgAmY7PhCfg',0,0,0),(2,42,'/etc/','','P0A BAA IH4 C A A A BAA BAA BoX BoX BoX A A C','0',0,0,0)");
    assert_eq!(log[2], "BEGIN; LOCK TABLE Path IN SHARE ROW EXCLUSIVE MODE\n");
    assert!(log[3].starts_with("INSERT INTO Path (Path)\nSELECT a.Path"));
    assert_eq!(log[4], "COMMIT\n");
    assert!(log[5].starts_with("INSERT INTO File (FileIndex, JobId, PathId, Name, LStat, MD5, DeltaSeq, Fhinfo, Fhnode) SELECT"));
    assert_eq!(log[6], "DROP TABLE IF EXISTS batch");
    assert!(catalog.log().is_empty());
}

#[test]
fn update_file_list_writes_the_batch_and_restore_objects_are_stored() {
    let catalog = MockDb::new(|sql| sql.starts_with("INSERT INTO RestoreObject").then(|| Ok(vec![row(&["5"])])));
    let batch = MockDb::new(|_| None);
    let opener_db = batch.clone();
    let opener = move || Some(Box::new(opener_db.clone()) as Box<dyn Db + Send>);
    let mut job = CatReqJob::new(context());
    job.dispatch(Some(&catalog), &opener, &upd_cat(1, STREAM_UNIX_ATTRIBUTES, b"1 23 /gone\0P0A\0\0"));
    // The deleted file (FT_DELETED) has FileIndex 0; flushed by the next attribute.
    job.dispatch(Some(&catalog), &opener, &upd_cat(2, STREAM_UNIX_ATTRIBUTES, b"2 3 /a b\0P0B\0\0\0 7"));
    let reply = job.dispatch(Some(&catalog), &opener, b"Catreq Job=backup.2026 UpdateFileList\n");
    assert_eq!(reply.to_sd, "");
    assert_eq!(reply.messages[0].1, "Insert of attributes batch table with 1 entries start\n");
    assert!(batch.log().iter().any(|s| s == "INSERT INTO batch VALUES (0,42,'/','gone','P0A','0',0,0,0)"));
    // The cached /a b (DeltaSeq 7) goes into a new batch at the end of the job.
    let messages = job.end_of_job(Some(&catalog), &opener);
    assert_eq!(messages[0].1, "Insert of attributes batch table with 1 entries start\n");
    assert!(batch.log().iter().any(|s| s == "INSERT INTO batch VALUES (2,42,'/','a b','P0B','0',7,0,0)"));

    let robj = upd_cat(3, STREAM_RESTORE_OBJECT, b"3 27 0 3 3 0 plugin:x\0object.name\0ab'");
    assert_eq!(job.dispatch(Some(&catalog), &opener, &robj), Reply::default());
    assert_eq!(catalog.log().last().unwrap(), "INSERT INTO RestoreObject (ObjectName,PluginName,RestoreObject,\
ObjectLength,ObjectFullLength,ObjectIndex,ObjectType,ObjectCompression,FileIndex,JobId) \
VALUES ('object.name','plugin:x','\\x616227',3,3,0,27,0,3,42); SELECT currval('RestoreObject_RestoreObjectid_seq')");
}

#[test]
fn catalog_files_off_and_blast_attr() {
    let db = MockDb::new(|_| None);
    let mut ctx = context();
    ctx.catalog_files = false;
    let mut job = CatReqJob::new(ctx);
    assert_eq!(job.dispatch(Some(&db), &no_batch, &upd_cat(1, 1, b"1 3 /x\0P\0\0\0")), Reply::default());
    assert_eq!(job.dispatch(Some(&db), &no_batch, b"BlastAttr Job=b File=/nonexistent").to_sd, "1990 ERROR BlastAttr\n");
    let mut job = CatReqJob::new(context());
    assert_eq!(job.dispatch(Some(&db), &no_batch, b"BlastAttr Job=b").messages[0].1, "Malformed message: BlastAttr Job=b\n");
    // A spool file with one attribute record.
    let dir = std::env::temp_dir().join(format!("c055-blast-{}", std::process::id()));
    let record = upd_cat(1, 1, b"1 3 /x\0P\0\0\0");
    let mut file = (record.len() as i32).to_be_bytes().to_vec();
    file.extend_from_slice(&record);
    std::fs::write(&dir, file).unwrap();
    let batch = MockDb::new(|_| None);
    let opener_db = batch.clone();
    let opener = move || Some(Box::new(opener_db.clone()) as Box<dyn Db + Send>);
    let reply = job.dispatch(Some(&db), &opener, format!("BlastAttr Job=b File={}", dir.display()).as_bytes());
    std::fs::remove_file(&dir).ok();
    assert_eq!(reply.to_sd, "1000 OK BlastAttr\n");
    assert_eq!(job.end_of_job(Some(&db), &opener).len(), 2);
    assert!(batch.log().iter().any(|s| s == "INSERT INTO batch VALUES (1,42,'/','x','P','0',0,0,0)"));
}

// --- real catalog ------------------------------------------------------------------

/// With ONESIMUS_CATREQ_DB=<postgres url of a throw-away catalog copy>: run
/// a backup's catalog requests and check the catalog effects.
#[test]
fn real_catalog_round_trip() {
    let Ok(url) = std::env::var("ONESIMUS_CATREQ_DB") else { return };
    static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let (cat, _join) = crate::catalog_worker::spawn_catalog_worker(url, &SHUTDOWN);
    let job_id: u32 = cat.query_rows("INSERT INTO Job (Job,Name,Type,Level,JobStatus,PoolId,ClientId) \
VALUES ('c055.test','c055','B','F','R',3,1); SELECT currval('job_jobid_seq')").unwrap().rows[0][0].clone().unwrap().parse().unwrap();
    let storage_id: i64 = cat.query_rows("SELECT StorageId FROM Storage WHERE Name='File'").unwrap().rows[0][0].clone().unwrap().parse().unwrap();
    let ctx = JobContext { job_id, start_time: now(), pool_id: 3, pool_name: "Full".into(), catalog_files: true,
        write_storage: Some(WriteStorage { media_type: "File".into(), storage_id, autochanger: false }) };
    let mut job = CatReqJob::new(ctx);
    let opener = || cat.dedicated().ok().map(|c| Box::new(c) as Box<dyn Db + Send>);
    let db: &dyn Db = &cat;
    let reply = job.dispatch(Some(db), &opener, b"CatReq Job=c055.test FindMedia=1 pool_name=Full media_type=File unwanted_volumes=\n");
    assert!(reply.to_sd.starts_with("1000 OK VolName="), "{reply:?}");
    let name = reply.to_sd.split_whitespace().nth(2).unwrap().trim_start_matches("VolName=").to_string();
    let media_id = reply.to_sd.split_whitespace().find_map(|w| w.strip_prefix("MediaId=")).unwrap().to_string();
    let reply = job.dispatch(Some(db), &opener, format!("CatReq Job=c055.test GetVolInfo VolName={name} write=1\n").as_bytes());
    assert!(reply.to_sd.starts_with("1000 OK VolName="), "{reply:?}");
    let reply = job.dispatch(Some(db), &opener, format!("CatReq Job=c055.test CreateJobMedia FirstIndex=1 LastIndex=1 \
StartFile=0 EndFile=0 StartBlock=0 EndBlock=999 Copy=0 Strip=0 MediaId={media_id}\n").as_bytes());
    assert_eq!(reply.to_sd, "1000 OK CreateJobMedia\n");
    job.dispatch(Some(db), &opener, &upd_cat(1, 1, b"1 3 /c055/dir/file\0P0A\0\0\0"));
    let reply = job.dispatch(Some(db), &opener, b"Catreq Job=c055.test UpdateJobRecord JobFiles=1 JobBytes=99\n");
    assert_eq!(reply.to_sd, "");
    let messages = job.end_of_job(Some(db), &opener);
    assert_eq!(messages.len(), 2, "{messages:?}");
    let files = cat.query_rows(&format!("SELECT Path.Path, File.Name, Job.JobFiles, Job.JobBytes FROM File JOIN Path USING (PathId) \
JOIN Job USING (JobId) WHERE File.JobId={job_id}")).unwrap();
    assert_eq!(files.rows, vec![row(&["/c055/dir/", "file", "1", "99"])]);
    let jm = cat.query_rows(&format!("SELECT VolIndex, EndBlock FROM JobMedia WHERE JobId={job_id}")).unwrap();
    assert_eq!(jm.rows, vec![row(&["1", "999"])]);
}
