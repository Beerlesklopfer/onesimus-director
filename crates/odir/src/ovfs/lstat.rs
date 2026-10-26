//! Port of `ovfs-core/src/lstat.rs` of the OVFS extension (commit `4041335`).
//!
//! Decoder for the Bareos `File.LStat` column.
//!
//! Bareos encodes `struct stat` as space-separated numbers in its own base64
//! digit format (`core/src/lib/attribs.cc` `EncodeStat`, `core/src/lib/base64.cc`
//! `FromBase64`). Field order:
//!
//! `st_dev st_ino st_mode st_nlink st_uid st_gid st_rdev st_size st_blksize
//!  st_blocks st_atime st_mtime st_ctime LinkFI st_flags data_stream`

const MODE: usize = 2;
const UID: usize = 4;
const GID: usize = 5;
const SIZE: usize = 7;
const MTIME: usize = 11;
const LINK_FI: usize = 13;

/// File type bits (`S_IFMT`) and values from POSIX.
const S_IFMT: i64 = 0o170000;
const S_IFDIR: i64 = 0o040000;
const S_IFREG: i64 = 0o100000;
const S_IFLNK: i64 = 0o120000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub mode: i64,
    pub uid: i64,
    pub gid: i64,
    pub size: i64,
    pub mtime: i64,
    /// FileIndex of the first copy of a hard link, 0 if none.
    pub link_fi: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Directory,
    File,
    Symlink,
    Other,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Directory => "directory",
            Kind::File => "file",
            Kind::Symlink => "symlink",
            Kind::Other => "other",
        }
    }
}

impl Stat {
    pub fn kind(&self) -> Kind {
        match self.mode & S_IFMT {
            S_IFDIR => Kind::Directory,
            S_IFREG => Kind::File,
            S_IFLNK => Kind::Symlink,
            _ => Kind::Other,
        }
    }
}

fn digit(byte: u8) -> u64 {
    match byte {
        b'A'..=b'Z' => (byte - b'A') as u64,
        b'a'..=b'z' => (byte - b'a') as u64 + 26,
        b'0'..=b'9' => (byte - b'0') as u64 + 52,
        b'+' => 62,
        b'/' => 63,
        // Bareos maps unknown characters to 0 as well
        _ => 0,
    }
}

/// One number in Bareos base64 notation, e.g. `"-B"` → -1.
pub fn from_base64(field: &str) -> i64 {
    let (negative, digits) = match field.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, field),
    };
    let value = digits
        .bytes()
        .fold(0u64, |acc, byte| acc.wrapping_shl(6).wrapping_add(digit(byte)));
    if negative {
        (value as i64).wrapping_neg()
    } else {
        value as i64
    }
}

/// Decode an LStat string. Returns `None` if it has too few fields.
pub fn decode(lstat: &str) -> Option<Stat> {
    let fields: Vec<i64> = lstat.split(' ').map(from_base64).collect();
    if fields.len() <= MTIME {
        return None;
    }
    Some(Stat {
        mode: fields[MODE],
        uid: fields[UID],
        gid: fields[GID],
        size: fields[SIZE],
        mtime: fields[MTIME],
        link_fi: fields.get(LINK_FI).copied().unwrap_or(0),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Inverse of `from_base64`, as in Bareos `ToBase64`.
    fn to_base64(value: i64) -> String {
        const DIGITS: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let negative = value < 0;
        let mut v = value.unsigned_abs();
        loop {
            out.push(DIGITS[(v & 0x3f) as usize]);
            v >>= 6;
            if v == 0 {
                break;
            }
        }
        if negative {
            out.push(b'-');
        }
        out.reverse();
        String::from_utf8(out).unwrap()
    }

    pub(crate) fn encode(stat: [i64; 16]) -> String {
        stat.iter().map(|v| to_base64(*v)).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn decodes_single_numbers() {
        assert_eq!(from_base64("A"), 0);
        assert_eq!(from_base64("B"), 1);
        assert_eq!(from_base64("/"), 63);
        assert_eq!(from_base64("BA"), 64);
        assert_eq!(from_base64("-B"), -1);
        for v in [0, 1, 63, 64, 4096, 0o100644, 1_792_116_000, i32::MAX as i64, -42] {
            assert_eq!(from_base64(&to_base64(v)), v);
        }
    }

    #[test]
    fn decodes_regular_file() {
        let lstat = encode([2049, 131, 0o100644, 1, 1000, 1000, 0, 4711, 4096, 16, 1, 1_700_000_000, 3, 0, 0, 1]);
        let stat = decode(&lstat).unwrap();
        assert_eq!(stat.mode, 0o100644);
        assert_eq!(stat.uid, 1000);
        assert_eq!(stat.size, 4711);
        assert_eq!(stat.mtime, 1_700_000_000);
        assert_eq!(stat.kind(), Kind::File);
    }

    #[test]
    fn decodes_kinds() {
        let with_mode = |mode| decode(&encode([0, 0, mode, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])).unwrap().kind();
        assert_eq!(with_mode(0o040755), Kind::Directory);
        assert_eq!(with_mode(0o120777), Kind::Symlink);
        assert_eq!(with_mode(0o020666), Kind::Other);
    }

    #[test]
    fn rejects_truncated_lstat() {
        assert_eq!(decode("A B C"), None);
        assert_eq!(decode(""), None);
    }
}
