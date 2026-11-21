//! The Win32 attributes of a file as they travel in the attribute record
//! (`attribsEx`, the fifth field of `STREAM_UNIX_ATTRIBUTES`).
//!
//! Zweck: Auf Unix bleibt `attribsEx` leer. Ein Windows-FD schreibt dort die
//! `WIN32_FILE_ATTRIBUTE_DATA` der Datei, damit eine Wiederherstellung
//! Attribute (schreibgeschützt, versteckt, System, Archiv) und die genauen
//! Zeiten (100 ns, mit Erstellungszeit) setzen kann; der Katalog speichert
//! nur Sekunden.
//!
//! Format: zehn durch Leerzeichen getrennte Zahlen in Bareos-Base64
//! (`to_base64`): `dwFileAttributes`, `ftCreationTime` (niedrig, hoch),
//! `ftLastAccessTime` (niedrig, hoch), `ftLastWriteTime` (niedrig, hoch),
//! `nFileSizeHigh`, `nFileSizeLow`, `dwReserved0` (Tag des Reparse-Punkts).
//! Das Format ist aus dem Gedächtnis des Bareos-Quelltextes
//! (`findlib/attribs.cc`, `encode_attribsEx`/`set_win32_attributes`)
//! nachgebaut und **nicht gegen einen Bareos-Windows-FD geprüft**; die
//! Neun-Zahlen-Form ohne `dwReserved0` wird beim Lesen ebenfalls angenommen.
//! Das Modul ist auf jeder Plattform übersetzt, damit das Format auf Unix
//! getestet werden kann.

use bareos_format::base64::{from_base64, to_base64};

/// The Win32 data of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WinMeta {
    /// `dwFileAttributes`
    pub attributes: u32,
    /// `ftCreationTime` as 100-ns ticks since 1601
    pub creation: i64,
    /// `ftLastAccessTime`
    pub access: i64,
    /// `ftLastWriteTime`
    pub write: i64,
    /// File size
    pub size: u64,
    /// `dwReserved0`: the reparse tag of a reparse point
    pub reparse_tag: u32,
}

/// The `attribsEx` field of `meta`.
pub fn encode(meta: &WinMeta) -> Vec<u8> {
    let low = |t: i64| i64::from(t as u64 as u32);
    let high = |t: i64| i64::from((t as u64 >> 32) as u32);
    [
        i64::from(meta.attributes),
        low(meta.creation),
        high(meta.creation),
        low(meta.access),
        high(meta.access),
        low(meta.write),
        high(meta.write),
        (meta.size >> 32) as i64,
        i64::from(meta.size as u32),
        i64::from(meta.reparse_tag),
    ]
    .iter()
    .map(|v| to_base64(*v))
    .collect::<Vec<_>>()
    .join(" ")
    .into_bytes()
}

/// The data of an `attribsEx` field; `None` when it is empty (a Unix FD)
/// or not in this format.
pub fn decode(field: &[u8]) -> Option<WinMeta> {
    let text = std::str::from_utf8(field).ok()?.trim_matches('\0').trim();
    if text.is_empty() {
        return None;
    }
    let numbers: Vec<i64> = text.split(' ').map(|t| from_base64(t.as_bytes()).0).collect();
    if numbers.len() < 9 {
        return None;
    }
    let ticks = |low: i64, high: i64| (((high as u64 & 0xffff_ffff) << 32) | (low as u64 & 0xffff_ffff)) as i64;
    Some(WinMeta {
        attributes: numbers[0] as u32,
        creation: ticks(numbers[1], numbers[2]),
        access: ticks(numbers[3], numbers[4]),
        write: ticks(numbers[5], numbers[6]),
        size: ((numbers[7] as u64 & 0xffff_ffff) << 32) | (numbers[8] as u64 & 0xffff_ffff),
        reparse_tag: numbers.get(9).map_or(0, |v| *v as u32),
    })
}

/// The attribute bits a restore may set: read-only, hidden, system,
/// archive, temporary, not content indexed. Directory, reparse point,
/// compression, encryption, sparse and offline are properties of the
/// object, not of a setting.
pub const SETTABLE: u32 = 0x1 | 0x2 | 0x4 | 0x20 | 0x100 | 0x2000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_field_reads_back() {
        let meta = WinMeta {
            attributes: 0x23,
            creation: 133_800_000_123_456_789,
            access: 133_800_000_987_654_321,
            write: 133_799_999_000_000_001,
            size: (5u64 << 32) | 77,
            reparse_tag: 0xA000_000C,
        };
        let field = encode(&meta);
        assert!(field.iter().all(|b| b.is_ascii_graphic() || *b == b' '));
        assert_eq!(decode(&field), Some(meta));
    }

    #[test]
    fn a_unix_field_and_a_bad_one_are_not_win32_data() {
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"\0"), None);
        assert_eq!(decode(b"A B C"), None);
    }

    #[test]
    fn the_nine_number_form_without_reparse_tag_is_read() {
        let meta = WinMeta { attributes: 1, write: 42, ..Default::default() };
        let text = String::from_utf8(encode(&meta)).unwrap();
        let nine: Vec<&str> = text.split(' ').take(9).collect();
        assert_eq!(decode(nine.join(" ").as_bytes()), Some(meta));
    }
}
