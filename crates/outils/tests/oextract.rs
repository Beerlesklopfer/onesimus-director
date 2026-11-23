// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `oextract` gegen ein von `bareos-format` geschriebenes Volume: Verzeichnis,
//! einfache, gzip-komprimierte und spärliche Datei, Symlink, Hartlink.

use std::process::Command;

use bareos_format::compression::{compress_data, with_header, COMPRESS_FZ4L, COMPRESS_GZIP};
use bareos_format::label::{SessionLabel, VolumeLabel, EOS_LABEL, SOS_LABEL, VOL_LABEL};
use bareos_format::lstat::{encode_stat, Stat};
use bareos_format::stream::*;
use bareos_format::volume::{BlockBuilder, RecordProgress, RecordRef};

/// Schreibt Datensätze in kleine Blöcke (Datensätze über Blockgrenzen).
fn volume(records: &[(i32, i32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut builder = BlockBuilder::new(1024);
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

/// Attributsatz: `<FileIndex> <Typ> <Name>\0<LStat>\0<Link>\0`.
fn attributes(index: i32, name: &str, mode: u32, size: i64, link: &str, link_fi: i32) -> Vec<u8> {
    let st = Stat { mode, size, mtime: 1_700_000_000, atime: 1_700_000_100, ..Default::default() };
    let mut data = format!("{index} 1 {name}\0").into_bytes();
    data.extend(encode_stat(&st, link_fi, STREAM_FILE_DATA).bytes());
    data.extend(format!("\0{link}\0").bytes());
    data
}

#[test]
fn oextract_restores_files_links_sparse_and_compressed_data() {
    let label = VolumeLabel::new("V1", "Full", "File", "h", "osd", "v", "b", 1_700_000_000_000_000);
    let start = SessionLabel { id: "Bareos 2.0 immortal\n".into(), ver_num: 20, job_id: 42, pool_name: "Full".into(), pool_type: "Backup".into(),
        job_name: "j".into(), client_name: "c".into(), job: "j.1".into(), fileset_name: "s".into(), job_type: 66, job_level: 70, ..Default::default() };
    let end = SessionLabel { job_files: Some(5), job_bytes: Some(0), job_status: Some(84), ..start.clone() };
    let big: Vec<u8> = (0..5000u32).flat_map(|i| i.to_be_bytes()).collect();
    let gz = with_header(COMPRESS_GZIP, 6, &compress_data(COMPRESS_GZIP, 6, &big).unwrap());
    let lz = with_header(COMPRESS_FZ4L, 1, &compress_data(COMPRESS_FZ4L, 1, b"lz4 text lz4 text lz4 text").unwrap());
    let mut sparse_a = 0u64.to_be_bytes().to_vec();
    sparse_a.extend(b"head");
    let mut sparse_b = 100_000u64.to_be_bytes().to_vec();
    sparse_b.extend(b"tail");
    let bytes = volume(&[
        (VOL_LABEL, 1, label.serialize()),
        (SOS_LABEL, 42, start.serialize()),
        (1, STREAM_UNIX_ATTRIBUTES, attributes(1, "/data/", 0o040_755, 0, "", 0)),
        (2, STREAM_UNIX_ATTRIBUTES, attributes(2, "/data/plain.txt", 0o100_640, 5, "", 0)),
        (2, STREAM_FILE_DATA, b"hello".to_vec()),
        (3, STREAM_UNIX_ATTRIBUTES, attributes(3, "/data/big.bin", 0o100_644, big.len() as i64, "", 0)),
        (3, STREAM_COMPRESSED_DATA, gz),
        (4, STREAM_UNIX_ATTRIBUTES, attributes(4, "/data/lz4.txt", 0o100_644, 26, "", 0)),
        (4, STREAM_COMPRESSED_DATA, lz),
        (5, STREAM_UNIX_ATTRIBUTES, attributes(5, "/data/sparse", 0o100_600, 100_004, "", 0)),
        (5, STREAM_SPARSE_DATA, sparse_a),
        (5, STREAM_SPARSE_DATA, sparse_b),
        (6, STREAM_UNIX_ATTRIBUTES, attributes(6, "/data/link", 0o120_777, 0, "plain.txt", 0)),
        (7, STREAM_UNIX_ATTRIBUTES, attributes(7, "/data/hard.txt", 0o100_640, 5, "", 2)),
        (8, STREAM_UNIX_ATTRIBUTES, attributes(8, "/etc/other", 0o100_644, 1, "", 0)),
        (8, STREAM_FILE_DATA, b"x".to_vec()),
        (EOS_LABEL, 42, end.serialize()),
    ]);
    let dir = std::env::temp_dir().join(format!("oextract-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vol = dir.join("V1");
    std::fs::write(&vol, &bytes).unwrap();
    let dest = dir.join("out");

    let run = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_oextract")).args(args).output().unwrap();
    // a dry run writes nothing
    let dry = run(&["-n", dest.to_str().unwrap(), vol.to_str().unwrap()]);
    assert!(dry.status.success(), "{}", String::from_utf8_lossy(&dry.stderr));
    assert!(!dest.exists());
    // restore of the /data tree
    let out = run(&["-p", "/data", dest.to_str().unwrap(), vol.to_str().unwrap()]);
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let summary = String::from_utf8_lossy(&out.stdout);
    assert!(summary.contains("4 files, 1 directories, 2 links"), "{summary}");
    assert_eq!(std::fs::read(dest.join("data/plain.txt")).unwrap(), b"hello");
    assert_eq!(std::fs::read(dest.join("data/big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(dest.join("data/lz4.txt")).unwrap(), b"lz4 text lz4 text lz4 text");
    let sparse = std::fs::read(dest.join("data/sparse")).unwrap();
    assert_eq!((sparse.len(), &sparse[..4], &sparse[100_000..], sparse[50_000]), (100_004, &b"head"[..], &b"tail"[..], 0));
    assert_eq!(std::fs::read_link(dest.join("data/link")).unwrap().to_str(), Some("plain.txt"));
    assert_eq!(std::fs::read(dest.join("data/hard.txt")).unwrap(), b"hello");
    assert!(!dest.join("etc").exists(), "outside the selection");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let m = std::fs::metadata(dest.join("data/plain.txt")).unwrap();
        assert_eq!((m.permissions().mode() & 0o7777, m.mtime()), (0o640, 1_700_000_000));
        assert_eq!(std::fs::metadata(dest.join("data")).unwrap().permissions().mode() & 0o7777, 0o755);
        assert_eq!(std::fs::metadata(dest.join("data/plain.txt")).unwrap().nlink(), 2);
    }
    // a job that is not on the volume restores nothing
    let none = run(&["-i", "7", dir.join("none").to_str().unwrap(), vol.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&none.stdout).contains("nothing found"));
    // a path that leaves the destination is refused
    assert!(outils::extract::target_path(&dest, "/a/../../etc/passwd").is_none());
    assert_eq!(outils::extract::target_path(&dest, "C:/Users/x").unwrap(), dest.join("C/Users/x"));
    let _ = std::fs::remove_dir_all(&dir);
}
