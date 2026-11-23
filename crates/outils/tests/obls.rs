//! `obls` gegen ein Volume, das `bareos-format` schreibt: Label, ein Job mit
//! zwei Dateien und EOS.

use std::process::Command;

use bareos_format::label::{SessionLabel, VolumeLabel, EOS_LABEL, SOS_LABEL, VOL_LABEL};
use bareos_format::lstat::{encode_stat, Stat};
use bareos_format::stream::{STREAM_FILE_DATA, STREAM_UNIX_ATTRIBUTES};
use bareos_format::volume::{BlockBuilder, RecordProgress, RecordRef};

/// Schreibt Datensätze in 4-KiB-Blöcke.
fn volume(records: &[(i32, i32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut builder = BlockBuilder::new(4096);
    let mut number = 0u32;
    for (file_index, stream, data) in records {
        let rec = RecordRef { vol_session_id: 3, vol_session_time: 99, file_index: *file_index, stream: *stream, data };
        let mut progress = RecordProgress::default();
        while !builder.write_record(&rec, &mut progress) {
            out.extend(builder.finish(number));
            number += 1;
        }
    }
    out.extend(builder.finish(number));
    out
}

/// Der Attributsatz einer Datei.
fn attributes(index: i32, name: &str, size: i64) -> Vec<u8> {
    let st = Stat { mode: 0o100_644, size, mtime: 1_700_000_000, ..Default::default() };
    let mut data = format!("{index} 3 {name}\0").into_bytes();
    data.extend(encode_stat(&st, 0, STREAM_FILE_DATA).bytes());
    data.extend(b"\0\0");
    data
}

#[test]
fn obls_lists_label_job_and_files() {
    let label = VolumeLabel::new("Vol-0001", "Full", "File", "host", "osd", "Ver. 1", "Build", 1_700_000_000_000_000);
    let start = SessionLabel {
        id: "Bareos 2.0 immortal\n".into(), ver_num: 20, job_id: 42, write_btime: 1_700_000_000_000_000,
        pool_name: "Full".into(), pool_type: "Backup".into(), job_name: "save".into(), client_name: "fd".into(),
        job: "save.2023-11-14_22.13.20_01".into(), fileset_name: "Set".into(), job_type: 66, job_level: 70, ..Default::default()
    };
    let end = SessionLabel { job_files: Some(2), job_bytes: Some(11), start_block: Some(0), end_block: Some(0), job_status: Some(84), ..start.clone() };
    let bytes = volume(&[
        (VOL_LABEL, 1, label.serialize()),
        (SOS_LABEL, 42, start.serialize()),
        (1, STREAM_UNIX_ATTRIBUTES, attributes(1, "/data/a.txt", 5)),
        (1, STREAM_FILE_DATA, b"hello".to_vec()),
        (2, STREAM_UNIX_ATTRIBUTES, attributes(2, "/data/b.txt", 6)),
        (2, STREAM_FILE_DATA, b"world!".to_vec()),
        (EOS_LABEL, 42, end.serialize()),
    ]);
    let dir = std::env::temp_dir().join(format!("obls-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Vol-0001");
    std::fs::write(&path, &bytes).unwrap();

    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_obls")).args(args).arg(&path).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    };
    let all = run(&[]);
    assert!(all.contains("VolName=Vol-0001") && all.contains("Pool=Full"), "{all}");
    assert!(all.contains("Job: JobId=42 Job=save.2023-11-14_22.13.20_01 Client=fd"), "{all}");
    assert!(all.contains("End: Files=2 Bytes=11") && all.contains("Status=T"), "{all}");
    assert!(all.contains("  /data/a.txt") && all.contains("  /data/b.txt"), "{all}");
    let jobs = run(&["-j"]);
    assert!(!jobs.contains("/data/a.txt") && jobs.contains("JobId=42"), "{jobs}");
    let long = run(&["-l"]);
    assert!(long.contains("-0644") && long.contains("2023-11-14 22:13:20") && long.contains("/data/b.txt"), "{long}");
    assert!(run(&["-i", "7"]).contains("0 files listed"));
    // a damaged block is named with its offset
    let mut broken = bytes.clone();
    broken[100] ^= 0xff;
    std::fs::write(&path, &broken).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_obls")).arg(&path).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("at byte 0"), "{}", String::from_utf8_lossy(&out.stderr));
    // -k does not look at the checksum: the damaged byte is in a record, the listing still works
    let skipped = Command::new(env!("CARGO_BIN_EXE_obls")).arg("-k").arg(&path).output().unwrap();
    assert!(!String::from_utf8_lossy(&skipped.stderr).contains("checksum"), "{}", String::from_utf8_lossy(&skipped.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}
