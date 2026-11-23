//! Gemeinsamer Teil der Onesimus-Volume-Werkzeuge (`ols`, später `oscan`,
//! `oextract`): ein Volume Block für Block aus einer Datei lesen.
//!
//! Die Werkzeuge ersetzen `bls`, `bscan` und `bextract` von Bareos. Sie
//! lesen Datei-Volumes (auch die von bareos-sd geschriebenen, Blockformat
//! BB01/BB02) und brauchen weder Director noch Storage Daemon noch Katalog.

#![deny(missing_docs)]

pub mod extract;
pub mod scan;

use std::fs::File;
use std::io::Read;
use std::path::Path;

use bareos_format::volume::{BlockHeader, RecordAssembler, VolRecord, BLKHDR1_LENGTH, BLKHDR2_LENGTH};

/// Ein Volume, das Block für Block gelesen wird. Der Speicherbedarf ist ein
/// Block, nicht das Volume.
pub struct VolumeReader {
    /// Die geöffnete Volume-Datei.
    file: File,
    /// Byte-Offset des nächsten Blocks.
    offset: u64,
    /// Setzt Datensätze über Blockgrenzen zusammen.
    assembler: RecordAssembler,
    /// Prüfsummen der Blöcke kontrollieren.
    verify: bool,
}

impl VolumeReader {
    /// Öffnet `path`; mit `verify` wird jede Blockprüfsumme kontrolliert.
    pub fn open(path: &Path, verify: bool) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self { file, offset: 0, assembler: RecordAssembler::new(), verify })
    }

    /// Der nächste Block: Kopf und die in ihm abgeschlossenen Datensätze;
    /// `None` am Ende der Datei. Ein Fehler nennt den Byte-Offset.
    pub fn next_block(&mut self) -> Result<Option<(u64, BlockHeader, Vec<VolRecord>)>, String> {
        let mut head = [0u8; BLKHDR2_LENGTH];
        let got = read_up_to(&mut self.file, &mut head).map_err(|e| format!("at byte {}: {e}", self.offset))?;
        if got == 0 {
            return Ok(None);
        }
        if got < BLKHDR1_LENGTH {
            return Err(format!("at byte {}: short block header ({got} bytes)", self.offset));
        }
        let header = BlockHeader::parse(&head[..got]).map_err(|e| format!("at byte {}: {e}", self.offset))?;
        let mut block = vec![0u8; header.block_len as usize];
        let have = got.min(block.len());
        block[..have].copy_from_slice(&head[..have]);
        let rest = read_up_to(&mut self.file, &mut block[have..]).map_err(|e| format!("at byte {}: {e}", self.offset))?;
        block.truncate(have + rest);
        let start = self.offset;
        self.offset += block.len() as u64;
        let (header, records) = self.assembler.push_block(&block, start, self.verify).map_err(|e| format!("at byte {start}: {e}"))?;
        Ok(Some((start, header, records)))
    }

    /// Datensätze, die am Ende des Volumes unvollständig sind (der Job läuft
    /// auf dem nächsten Volume weiter), mit der Zahl fehlender Bytes.
    pub fn take_pending(&mut self) -> Vec<(VolRecord, usize)> {
        self.assembler.take_pending()
    }
}

/// Liest bis `buf` voll ist oder die Datei endet; die Zahl gelesener Bytes.
fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match file.read(&mut buf[done..])? {
            0 => break,
            n => done += n,
        }
    }
    Ok(done)
}

/// Name (Rohbytes, auf Unix ohne Umwandlung gültig), LStat und Linkziel
/// aus einem Attributsatz (`STREAM_UNIX_ATTRIBUTES[_EX]`):
/// `<FileIndex> <Typ> <Name>\0<LStat>\0<Linkziel>\0…`.
pub fn parse_attributes_raw(data: &[u8]) -> Option<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let mut parts = data.splitn(3, |&b| b == b' ');
    let (_index, _kind, rest) = (parts.next()?, parts.next()?, parts.next()?);
    let mut fields = rest.split(|&b| b == 0);
    let name = fields.next()?.to_vec();
    let lstat = fields.next().unwrap_or_default().to_vec();
    let link = fields.next().unwrap_or_default().to_vec();
    Some((name, lstat, link))
}

/// Wie [`parse_attributes_raw`], Name und Linkziel als Text (ungültiges UTF-8
/// wird ersetzt: zur Anzeige und für SQL, nicht zum Schreiben von Dateien).
pub fn parse_attributes(data: &[u8]) -> Option<(String, Vec<u8>, String)> {
    let (name, lstat, link) = parse_attributes_raw(data)?;
    Some((String::from_utf8_lossy(&name).into_owned(), lstat, String::from_utf8_lossy(&link).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_attribute_record_gives_name_lstat_and_link() {
        let data = b"7 3 /etc/hosts\0abc def\0\0";
        let (name, lstat, link) = parse_attributes(data).unwrap();
        assert_eq!((name.as_str(), lstat.as_slice(), link.as_str()), ("/etc/hosts", &b"abc def"[..], ""));
        assert!(parse_attributes(b"garbage").is_none());
    }
}
