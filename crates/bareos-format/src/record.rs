//! Datensätze (Records): Netz-Köpfe; die Volume-Köpfe stehen in [`crate::volume`].
//!
//! Zweck: Der SD zerlegt den Datenstrom des FD in Datensätze und schreibt sie
//! in Blöcke ([`crate::volume`]). Jeder Datensatz hat einen Kopf und Nutzdaten.
//! Den Volume-Kopf schreibt und liest seit 20.11.2026
//! [`crate::volume::BlockBuilder`] bzw. [`crate::volume::read_volume`], die
//! Label-Inhalte [`crate::label`].
//!
//! Struktur laut Bareos `core/src/stored/record.h` und `block.h`, Zweig
//! `bareos-25` (gelesen 19.11.2026):
//!
//! | Version | Länge | Felder (serialisiert, in dieser Reihenfolge) |
//! |---|---|---|
//! | 1 (`RECHDR1_LENGTH`) | 20 | `VolSessionId` u32, `VolSessionTime` u32, `FileIndex` i32, `Stream` i32, `data_len` u32 |
//! | 2 (`RECHDR2_LENGTH`, geschrieben: `WRITE_RECHDR_LENGTH`) | 12 | `FileIndex` i32, `Stream` i32, `data_len` u32 |
//!
//! In Version 2 stehen `VolSessionId` und `VolSessionTime` im Blockkopf
//! (`BB02`) statt im Datensatzkopf.
//!
//! Label-Datensätze haben einen negativen `FileIndex` (`enum : int32_t` in
//! `record.h`): `PRE_LABEL` −1, `VOL_LABEL` −2, `EOM_LABEL` −3, `SOS_LABEL` −4,
//! `EOS_LABEL` −5, `EOT_LABEL` −6, `SOB_LABEL` −7, `EOB_LABEL` −8. Die
//! Byte-Kodierung der Label-Inhalte steht in [`crate::label`].
//!
//! Auf dem Netz (FD ↔ SD) sieht ein Datensatz anders aus als im Volume:
//! Vor den Nutzdaten steht ein eigener Kopf als Textzeile. Diese Köpfe sind
//! hier als Code vorhanden, weil ofd sie heute schreibt und liest und osd
//! sie in P1 braucht:
//!
//! | Richtung | Kopf | Typ | Bareos |
//! |---|---|---|---|
//! | FD → SD (Backup) | `<FileIndex> <Stream> 0` | [`data_header`], [`parse_data_header`] | `filed/backup.cc` `fsend("%" PRIu32 " %" PRId32 " 0")`, `stored/append.cc` `sscanf("%ld %ld")` |
//! | SD → FD (Restore, Verify) | `rechdr <VolSessionId> <VolSessionTime> <FileIndex> <Stream> <Länge>` | [`RecHdr`] | `stored/read.cc` `fsend("rechdr …")`, `filed/restore.cc`, `filed/verify_vol.cc` `sscanf` |
//!
//! Gelesen wird mit der Semantik von `bsscanf` (`lib/scan.cc`): Ein
//! Leerzeichen im Format schluckt null oder mehr Leerzeichen, eine Zahl ohne
//! Ziffern gilt als 0 und zählt trotzdem, Text nach dem Format wird
//! ignoriert. Bis 20.11.2026 standen die Köpfe als Formatstrings in
//! `ofd/src/backup.rs` (`header`), `ofd/src/restore.rs` (`parse_rechdr`) und
//! `ofd/src/job/backup_cmds.rs` (`do_verify_volume`).
//!
//! Bareos-Gegenstück: `core/src/stored/record.cc` (`WriteRecordToBlock`,
//! `ReadRecordFromBlock`), `core/src/stored/record.h`.

/// `bsscanf()` whitespace (`isspace`): ASCII whitespace and the vertical tab.
fn is_space(c: u8) -> bool {
    c.is_ascii_whitespace() || c == 0x0b
}

/// `bsscanf(buf, "<prefix> %n %n …")` for a literal prefix followed by
/// integer conversions (`signed[k]`: `%d`/`%ld`, else `%u`), each preceded by
/// a blank in the format unless it is the very first character. `None` on a
/// literal mismatch, as `bsscanf` returns -1 there; numbers without digits
/// are 0, values wrap like the C original.
fn scan_ints(buf: &[u8], prefix: &[u8], signed: &[bool]) -> Option<Vec<i64>> {
    if !buf.starts_with(prefix) {
        return None;
    }
    let at = |b: usize| buf.get(b).copied().unwrap_or(0);
    let mut b = prefix.len();
    let mut out = Vec::with_capacity(signed.len());
    for (k, &is_signed) in signed.iter().enumerate() {
        if k > 0 || !prefix.is_empty() {
            while b < buf.len() && is_space(buf[b]) {
                b += 1;
            }
        }
        let neg = is_signed && at(b) == b'-';
        if neg {
            b += 1;
        }
        let mut v: u64 = 0;
        while at(b).is_ascii_digit() {
            v = v.wrapping_mul(10).wrapping_add(u64::from(at(b) - b'0'));
            b += 1;
        }
        let v = v as i64;
        out.push(if neg { v.wrapping_neg() } else { v });
    }
    Some(out)
}

/// The header line the FD sends to the SD before each record of a backup:
/// `<FileIndex> <Stream> 0` (`filed/backup.cc`).
pub fn data_header(file_index: u32, stream: i32) -> String {
    format!("{file_index} {stream} 0")
}

/// The SD side of [`data_header`]: `sscanf("%ld %ld")` in
/// `stored/append.cc`, giving (FileIndex, Stream).
pub fn parse_data_header(line: &[u8]) -> Option<(i64, i64)> {
    let v = scan_ints(line, b"", &[true, true])?;
    Some((v[0], v[1]))
}

/// The record header the SD sends to the FD before each record of a
/// restore or a volume verify (`stored/read.cc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecHdr {
    /// `VolSessionId` of the job that wrote the record.
    pub vol_session_id: u32,
    /// `VolSessionTime` of the job that wrote the record.
    pub vol_session_time: u32,
    /// `FileIndex` of the file the record belongs to.
    pub file_index: i32,
    /// Stream id (`STREAM_*`, [`crate::stream`]).
    pub stream: i32,
    /// Length of the data record that follows.
    pub data_len: u32,
}

impl RecHdr {
    /// `rechdr %u %u %d %d %u` as `stored/read.cc` writes it.
    pub fn format(&self) -> String {
        format!(
            "rechdr {} {} {} {} {}",
            self.vol_session_id, self.vol_session_time, self.file_index, self.stream, self.data_len
        )
    }

    /// `sscanf(msg, "rechdr %u %u %d %d %u")` as `filed/restore.cc` reads
    /// it; `None` when the line does not start with `rechdr`.
    pub fn parse(msg: &[u8]) -> Option<Self> {
        let v = scan_ints(msg, b"rechdr", &[false, false, true, true, false])?;
        Some(Self {
            vol_session_id: v[0] as u32,
            vol_session_time: v[1] as u32,
            file_index: v[2] as i32,
            stream: v[3] as i32,
            data_len: v[4] as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rechdr_roundtrip_and_bsscanf_semantics() {
        let h = RecHdr { vol_session_id: 1, vol_session_time: 2, file_index: -3, stream: 4, data_len: 5 };
        assert_eq!(h.format(), "rechdr 1 2 -3 4 5");
        assert_eq!(RecHdr::parse(h.format().as_bytes()), Some(h));
        // ofd/src/scan.rs test: bsscanf(b"rechdr 1 2 -3 4 5", ...)[2] == -3
        assert_eq!(RecHdr::parse(b"rechdr 1 2 -3 4 5").unwrap().file_index, -3);
        // several blanks, trailing text ignored, missing numbers are 0
        assert_eq!(
            RecHdr::parse(b"rechdr  7\t8 9 10 11 trailing"),
            Some(RecHdr { vol_session_id: 7, vol_session_time: 8, file_index: 9, stream: 10, data_len: 11 })
        );
        assert_eq!(RecHdr::parse(b"rechdr 1"), Some(RecHdr { vol_session_id: 1, vol_session_time: 0, file_index: 0, stream: 0, data_len: 0 }));
        assert_eq!(RecHdr::parse(b"1 2 3"), None);
        assert_eq!(RecHdr::parse(b"Rechdr 1 2 3 4 5"), None);
    }

    #[test]
    fn data_header_like_filed_backup_cc() {
        assert_eq!(data_header(12, 2), "12 2 0");
        assert_eq!(parse_data_header(b"12 2 0"), Some((12, 2)));
        assert_eq!(parse_data_header(b"-3 26 0"), Some((-3, 26)));
    }
}
