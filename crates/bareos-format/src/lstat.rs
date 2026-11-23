// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! LStat: die Attribut-Zeichenkette einer Datei (`File.LStat`).
//!
//! Zweck: Bareos speichert die Felder von `struct stat` einer gesicherten
//! Datei als Zeichenkette aus Base64-Zahlen ([`crate::base64`]), durch je ein
//! Leerzeichen getrennt. Der FD erzeugt sie beim Backup
//! ([`encode_stat`]), der Director legt sie im Katalog ab, und FD (Restore),
//! Director (`bvfs`, `ovfs`, Verify) lesen sie wieder ([`decode_stat`]).
//!
//! Feldreihenfolge:
//!
//! | # | Feld | # | Feld |
//! |---|---|---|---|
//! | 0 | `st_dev` | 8 | `st_blksize` |
//! | 1 | `st_ino` | 9 | `st_blocks` |
//! | 2 | `st_mode` | 10 | `st_atime` |
//! | 3 | `st_nlink` | 11 | `st_mtime` |
//! | 4 | `st_uid` | 12 | `st_ctime` |
//! | 5 | `st_gid` | 13 | `LinkFI` (FileIndex des ersten Hardlinks, sonst 0) |
//! | 6 | `st_rdev` | 14 | `st_flags` |
//! | 7 | `st_size` | 15 | Daten-Stream-Kennung |
//!
//! Ältere Bareos-Versionen schrieben nur die Felder 0–12; [`decode_stat`]
//! liest beide Formen.
//!
//! Datenfluss: FD `encode_stat` → Attribut-Datensatz → SD → Director →
//! `File.LStat` → `decode_stat` in Director oder FD.
//!
//! Bareos-Gegenstück: `core/src/lib/attribs.cc` (`EncodeStat`, `DecodeStat`).
//!
//! Zusammengeführt (19.11.2026) aus `ofd/src/attribs.rs` (`Stat`,
//! `encode_stat`, `decode_stat`) und `odir/src/commands/bvfs.rs`
//! (`decode_stat`). `odir/src/ovfs/lstat.rs` behält seine schmale Sicht
//! (sechs Felder, Feldzerlegung an Leerzeichen) und nutzt nur noch
//! [`crate::base64::from_base64`].

use crate::base64::{from_base64, to_base64};

#[cfg_attr(doc, aquamarine::aquamarine)]
/// The fields of `struct stat` a Bareos attribute record carries.
///
/// ```mermaid
/// classDiagram
///     class Stat {
///         +u64 dev
///         +u64 ino
///         +u32 mode
///         +u64 nlink
///         +u32 uid
///         +u32 gid
///         +u64 rdev
///         +i64 size
///         +i64 blksize
///         +i64 blocks
///         +i64 atime
///         +i64 mtime
///         +i64 ctime
///         +u64 flags
///     }
///     class Decoded {
///         +Stat stat
///         +i32 link_fi
///         +i32 data_stream
///     }
///     Decoded *-- Stat
///     encode_stat ..> Stat : liest
///     decode_stat ..> Decoded : liefert
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stat {
    /// `st_dev`: device of the file system.
    pub dev: u64,
    /// `st_ino`: inode number.
    pub ino: u64,
    /// `st_mode`: file type and permission bits.
    pub mode: u32,
    /// `st_nlink`: number of hard links.
    pub nlink: u64,
    /// `st_uid`: owner.
    pub uid: u32,
    /// `st_gid`: group.
    pub gid: u32,
    /// `st_rdev`: device number of a device file.
    pub rdev: u64,
    /// `st_size`: size in bytes.
    pub size: i64,
    /// `st_blksize`: preferred I/O block size.
    pub blksize: i64,
    /// `st_blocks`: allocated 512-byte blocks.
    pub blocks: i64,
    /// `st_atime`: last access (seconds since the epoch).
    pub atime: i64,
    /// `st_mtime`: last modification.
    pub mtime: i64,
    /// `st_ctime`: last status change.
    pub ctime: i64,
    /// `st_flags`: BSD file flags (0 on Linux).
    pub flags: u64,
}

/// `EncodeStat()`: the LStat string with `LinkFI` and the data stream id.
pub fn encode_stat(st: &Stat, link_fi: i32, data_stream: i32) -> String {
    [
        st.dev as i64,
        st.ino as i64,
        i64::from(st.mode),
        st.nlink as i64,
        i64::from(st.uid),
        i64::from(st.gid),
        st.rdev as i64,
        st.size,
        st.blksize,
        st.blocks,
        st.atime,
        st.mtime,
        st.ctime,
        i64::from(link_fi),
        0,
        i64::from(data_stream),
    ]
    .iter()
    .map(|v| to_base64(*v))
    .collect::<Vec<_>>()
    .join(" ")
}

/// `DecodeStat()`: the stat fields, `LinkFI` and the data stream id.
///
/// Reads like the C++ code: thirteen numbers, each followed by one skipped
/// byte, then `LinkFI`, `st_flags` and the data stream only if the next byte
/// is a blank (or the one after it). Missing fields are 0.
pub fn decode_stat(buf: &[u8]) -> (Stat, i32, i32) {
    let mut st = Stat::default();
    let mut p = 0usize;
    let next = |p: &mut usize| -> i64 {
        let (v, n) = from_base64(&buf[(*p).min(buf.len())..]);
        *p += n;
        v
    };
    st.dev = next(&mut p) as u64;
    p += 1;
    st.ino = next(&mut p) as u64;
    p += 1;
    st.mode = next(&mut p) as u32;
    p += 1;
    st.nlink = next(&mut p) as u64;
    p += 1;
    st.uid = next(&mut p) as u32;
    p += 1;
    st.gid = next(&mut p) as u32;
    p += 1;
    st.rdev = next(&mut p) as u64;
    p += 1;
    st.size = next(&mut p);
    p += 1;
    st.blksize = next(&mut p);
    p += 1;
    st.blocks = next(&mut p);
    p += 1;
    st.atime = next(&mut p);
    p += 1;
    st.mtime = next(&mut p);
    p += 1;
    st.ctime = next(&mut p);
    let at = |i: usize| buf.get(i).copied().unwrap_or(0);
    // "*p == ' ' || (*p != 0 && *(p + 1) == ' ')"
    let more = |p: usize| at(p) == b' ' || (at(p) != 0 && at(p + 1) == b' ');
    if !more(p) {
        return (st, 0, 0);
    }
    p += 1;
    let link_fi = next(&mut p) as u32 as i32;
    if more(p) {
        p += 1;
        st.flags = next(&mut p) as u64;
    }
    let data_stream = if more(p) {
        p += 1;
        next(&mut p)
    } else {
        0
    };
    (st, link_fi, data_stream as i32)
}

#[cfg(test)]
mod tests {
    //! Golden values: the replaced decoders, kept verbatim in [`old`].

    use super::*;

    /// The replaced implementations, copied unchanged (19.11.2026).
    mod old {
        /// odir/src/commands/bvfs.rs `Stat`.
        #[derive(Debug, Default, PartialEq)]
        pub struct BvfsStat {
            /// dev
            pub dev: i64,
            /// ino
            pub ino: i64,
            /// mode
            pub mode: i64,
            /// nlink
            pub nlink: i64,
            /// uid
            pub uid: u32,
            /// gid
            pub gid: u32,
            /// rdev
            pub rdev: i64,
            /// size
            pub size: i64,
            /// atime
            pub atime: i64,
            /// mtime
            pub mtime: i64,
            /// ctime
            pub ctime: i64,
            /// link_fi
            pub link_fi: i64,
        }

        /// odir/src/commands/bvfs.rs `from_base64`.
        fn from_base64(b: &[u8], i: &mut usize) -> i64 {
            let at = |i: usize| b.get(i).copied().unwrap_or(0);
            let mut neg = false;
            if at(*i) == b'-' {
                *i += 1;
                neg = true;
            }
            let mut val: u64 = 0;
            while at(*i) != 0 && at(*i) != b' ' {
                val = (val << 6) + crate::base64::digit_value(at(*i));
                *i += 1;
            }
            if neg { (val as i64).wrapping_neg() } else { val as i64 }
        }

        /// odir/src/commands/bvfs.rs `decode_stat`.
        pub fn bvfs_decode_stat(lstat: &str) -> BvfsStat {
            let b = lstat.as_bytes();
            let at = |i: usize| b.get(i).copied().unwrap_or(0);
            let mut i = 0;
            let mut v = [0i64; 13];
            for (n, value) in v.iter_mut().enumerate() {
                *value = from_base64(b, &mut i);
                if n < 12 {
                    i += 1;
                }
            }
            let mut link_fi = 0;
            if at(i) == b' ' || (at(i) != 0 && at(i + 1) == b' ') {
                i += 1;
                link_fi = i64::from(from_base64(b, &mut i) as u32 as i32);
            }
            BvfsStat {
                dev: v[0],
                ino: v[1],
                mode: i64::from(v[2] as u32),
                nlink: v[3],
                uid: v[4] as u32,
                gid: v[5] as u32,
                rdev: v[6],
                size: v[7],
                atime: v[10],
                mtime: v[11],
                ctime: v[12],
                link_fi,
            }
        }
    }

    /// LStat strings of every shape: long, short, odd separators, garbage.
    fn samples() -> Vec<String> {
        let enc = |vals: &[i64]| vals.iter().map(|v| to_base64(*v)).collect::<Vec<_>>().join(" ");
        let full = [2049i64, 1234, 0o100644, 1, 1000, 100, 0, 5, 4096, 8, 1_700_000_001, 1_700_000_002, 1_700_000_003, 7, 0, 2];
        let mut v = vec![
            enc(&full),
            enc(&full[..13]),
            enc(&full[..14]),
            enc(&full[..15]),
            enc(&[-1, -2, 0o40755, 3, 0, 0, 0, -9, 0, 0, 0, 0, 0, -1, 0, 0]),
            String::new(),
            "A".into(),
            "A B C".into(),
            "gB GwBB IHt B Po Po A A BAA B BgQ0Ur BgQ0Ur BgQ0Ur A A C".into(),
            "x y z * ? ! a b c d e f g h i j".into(),
        ];
        let mut x: u64 = 7;
        for _ in 0..200 {
            let vals: Vec<i64> = (0..16).map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> (x % 40)) as i64
            }).collect();
            v.push(enc(&vals[..13 + (x % 4) as usize]));
        }
        v
    }

    #[test]
    fn decode_stat_matches_bvfs() {
        for s in samples() {
            let (st, link_fi, _) = decode_stat(s.as_bytes());
            let b = old::bvfs_decode_stat(&s);
            assert_eq!(
                (st.dev as i64, st.ino as i64, i64::from(st.mode), st.nlink as i64, st.uid, st.gid, st.rdev as i64),
                (b.dev, b.ino, b.mode, b.nlink, b.uid, b.gid, b.rdev),
                "{s}"
            );
            assert_eq!((st.size, st.atime, st.mtime, st.ctime, i64::from(link_fi)), (b.size, b.atime, b.mtime, b.ctime, b.link_fi), "{s}");
        }
    }

    #[test]
    fn decode_stat_like_original() {
        // from ofd/src/attribs.rs tests
        let vals = [2049i64, 1234, 0o100644, 1, 1000, 100, 0, 5, 4096, 8, 1_700_000_001, 1_700_000_002, 1_700_000_003, 0, 0, 2];
        let s = vals.iter().map(|v| to_base64(*v)).collect::<Vec<_>>().join(" ");
        let (st, lfi, ds) = decode_stat(s.as_bytes());
        assert_eq!((st.mode, st.uid, st.size, st.mtime, lfi, ds), (0o100644, 1000, 5, 1_700_000_002, 0, 2));
        let s = vals[..13].iter().map(|v| to_base64(*v)).collect::<Vec<_>>().join(" ");
        let (st, lfi, ds) = decode_stat(s.as_bytes());
        assert_eq!((st.ctime, lfi, ds), (1_700_000_003, 0, 0));
    }

    #[test]
    fn encode_decode_roundtrip_and_golden() {
        let st = Stat { dev: 2049, ino: 1234, mode: 0o100644, nlink: 1, uid: 1000, gid: 100, size: 5, blksize: 4096,
            blocks: 8, atime: 1, mtime: 2, ctime: 3, ..Default::default() };
        let s = encode_stat(&st, 7, 2);
        // golden: ofd/src/attribs.rs encode_stat output for these values
        assert_eq!(s, "gB TS IGk B Po Bk A F BAA I B C D H A C");
        assert_eq!(decode_stat(s.as_bytes()), (st, 7, 2));
    }
}
