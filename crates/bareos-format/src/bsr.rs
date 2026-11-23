// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Bootstrap (BSR): welche Datensätze ein Restore von welchen Volumes liest.
//!
//! Zweck: Für Restore, Verify (VolumeToCatalog), Copy und Migration schickt
//! der Director dem SD einen Bootstrap. Er nennt je Abschnitt ein oder
//! mehrere Volumes, die Sitzung (`VolSessionId`/`VolSessionTime`) des
//! schreibenden Jobs, die Adressbereiche auf dem Volume (`VolAddr`) und die
//! gewünschten FileIndex-Bereiche samt Anzahl (`Count`). Der SD liest damit
//! nur die nötigen Volumes und Blöcke und reicht nur passende Datensätze an
//! den FD weiter. Dieses Modul zerlegt den Text ([`Bsr::parse`]), liefert
//! die Volume-Liste in Lesereihenfolge ([`Bsr::restore_volumes`]) und die
//! Lesebereiche je Volume ([`Bsr::volume_ranges`]) und entscheidet über
//! jeden Datensatz wie bareos-sd ([`BsrMatcher`]); dazu gehören die
//! „erledigt“-Merker, mit denen bareos-sd das Ende eines Restores erkennt.
//! Es macht kein IO.
//!
//! Beispiel, wie ihn der Director für einen Job schreibt:
//!
//! ```text
//! Storage="vsd"
//! Volume="VVol-0001"
//! MediaType="VFile"
//! VolSessionId=1
//! VolSessionTime=1795204762
//! VolAddr=226-12592458
//! FileIndex=1-13
//! Count=13
//! ```
//!
//! Ein neuer Abschnitt beginnt mit jedem weiteren `Volume=`. Schlüsselwörter
//! sind groß/klein-unabhängig; Listen stehen mit Komma, Bereiche als
//! `a-b`; `#` leitet einen Kommentar ein, `;` wirkt wie ein Zeilenende.
//!
//! | Schlüssel | Wert | Prüfung je Datensatz |
//! |---|---|---|
//! | `Volume` | Name, mehrere mit `\|` | Volume-Label muss passen |
//! | `MediaType`, `Device`, `Slot` | gilt für die Volumes des Abschnitts | – |
//! | `VolSessionId` | Bereiche | Sitzung des Datensatzes |
//! | `VolSessionTime` | Zahlen | Sitzung des Datensatzes |
//! | `VolAddr` | 64-Bit-Bereiche | Adresse des letzten Bytes des Blocks, aus dem der Datensatz gelesen wurde |
//! | `VolFile`, `VolBlock` | 32-Bit-Bereiche | obere bzw. untere 32 Bit dieser Adresse (alte Form) |
//! | `FileIndex` | Bereiche | FileIndex des Datensatzes |
//! | `Count` | Zahl | Dateien des Abschnitts; danach ist er erledigt |
//! | `JobId`, `Job`, `Client` | Bereiche bzw. Namen | aus dem SOS-Label, nur ohne `Count` |
//! | `Stream` | Zahlen | Stream des Datensatzes, nur ohne `Count` |
//! | `FileRegex` | POSIX-ERE | Dateiname im Attribut-Datensatz |
//! | `Storage`, `Include`, `Exclude`, `JobType`, `JobLevel` | – | werden gelesen und ignoriert |
//!
//! Bareos-Gegenstück: `core/src/lib/parse_bsr.cc` (`parse_bsr`,
//! `store_*`, `IsFastRejectionOk`, `IsPositioningOk`), `stored/bsr.h`,
//! `stored/bsr.cc` (`MatchBsr`, `MatchAll`, `MatchVolume`, `MatchVoladdr`,
//! `MatchFindex`, `MatchSesstime`, `MatchSessid`, `MatchBsrBlock`,
//! `IsThisBsrDone`, `find_next_bsr`, `GetBsrStartAddr`,
//! `CreateRestoreVolumeList`), `lib/lex.cc` (Zerlegung), Bareos `18be77e6`.
//!
//! Abweichungen: `JobType` und `JobLevel` überspringen ihren Wert (Bareos
//! liest ihn nicht und meldet den Wert dann als unbekanntes Schlüsselwort).

use crate::label::SessionLabel;

/// One volume of a bootstrap section (`BsrVolume`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BsrVolume {
    /// `Volume=`.
    pub name: String,
    /// `MediaType=` of the section.
    pub media_type: String,
    /// `Device=` of the section.
    pub device: String,
    /// `Slot=` (autochanger), 0 = none.
    pub slot: i32,
}

/// One section of a bootstrap (`BootStrapRecord`): everything from one
/// `Volume=` line to the next.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BsrEntry {
    /// The volumes (`Volume=a|b`).
    pub volumes: Vec<BsrVolume>,
    /// `Count=`: files of this section, 0 = not given.
    pub count: u32,
    /// `VolSessionId=` ranges.
    pub sessid: Vec<(u32, u32)>,
    /// `VolSessionTime=` values.
    pub sesstime: Vec<u32>,
    /// `VolFile=` ranges.
    pub volfile: Vec<(u32, u32)>,
    /// `VolBlock=` ranges.
    pub volblock: Vec<(u32, u32)>,
    /// `VolAddr=` ranges.
    pub voladdr: Vec<(u64, u64)>,
    /// `FileIndex=` ranges.
    pub file_index: Vec<(i32, i32)>,
    /// `JobId=` ranges.
    pub job_id: Vec<(u32, u32)>,
    /// `Job=` names.
    pub job: Vec<String>,
    /// `Client=` names.
    pub client: Vec<String>,
    /// `Stream=` ids.
    pub stream: Vec<i32>,
    /// `FileRegex=`.
    pub fileregex: Option<String>,
}

/// A parsed bootstrap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bsr {
    /// The sections in file order.
    pub entries: Vec<BsrEntry>,
    /// `use_fast_rejection`: every section has a session id and time.
    pub use_fast_rejection: bool,
    /// `use_positioning`: every section has `VolAddr` or `VolFile`+`VolBlock`.
    pub use_positioning: bool,
}

/// An error of [`Bsr::parse`] with its place (`scan_err`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BsrError {
    /// Line number, from 1.
    pub line: usize,
    /// What went wrong.
    pub message: String,
}

impl std::fmt::Display for BsrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bootstrap file error: {}\n            : Line {}", self.message, self.line)
    }
}

impl std::error::Error for BsrError {}

/// A volume of the restore volume list (`VolumeList` of
/// `CreateRestoreVolumeList`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreVolume {
    /// Volume name.
    pub name: String,
    /// Media type from the bootstrap.
    pub media_type: String,
    /// Device from the bootstrap.
    pub device: String,
    /// Slot.
    pub slot: i32,
    /// Smallest `VolFile` of the first section naming it (0 for later volumes).
    pub start_file: u32,
}

/// One token of the bootstrap lexer (`lib/lex.cc`, reduced to what
/// `parse_bsr` sees).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// End of line (`\n` or `;`).
    Eol,
    /// `=`.
    Equals,
    /// `,`.
    Comma,
    /// A word or quoted string.
    Word(String),
    /// End of text.
    Eof,
}

/// The lexer over the bootstrap text.
struct Lexer<'a> {
    /// Text bytes.
    text: &'a [u8],
    /// Position.
    at: usize,
    /// Current line, from 1.
    line: usize,
}

impl Lexer<'_> {
    /// An error at the current line.
    fn error(&self, message: String) -> BsrError {
        BsrError { line: self.line, message }
    }

    /// `LexGetToken(BCT_ALL)`.
    fn next(&mut self) -> Result<Token, BsrError> {
        loop {
            let Some(&c) = self.text.get(self.at) else { return Ok(Token::Eof) };
            match c {
                b'\n' => {
                    self.at += 1;
                    self.line += 1;
                    return Ok(Token::Eol);
                }
                b';' => {
                    self.at += 1;
                    return Ok(Token::Eol);
                }
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'#' => {
                    while self.text.get(self.at).is_some_and(|&c| c != b'\n') {
                        self.at += 1;
                    }
                }
                b'=' => {
                    self.at += 1;
                    return Ok(Token::Equals);
                }
                b',' => {
                    self.at += 1;
                    return Ok(Token::Comma);
                }
                b'"' => {
                    self.at += 1;
                    let mut out = Vec::new();
                    loop {
                        let Some(&c) = self.text.get(self.at) else {
                            return Err(self.error("unterminated quoted string".into()));
                        };
                        self.at += 1;
                        match c {
                            b'"' => break,
                            b'\\' => {
                                if let Some(&n) = self.text.get(self.at) {
                                    out.push(n);
                                    self.at += 1;
                                }
                            }
                            b'\n' => {
                                self.line += 1;
                                out.push(b'\n');
                            }
                            _ => out.push(c),
                        }
                    }
                    return Ok(Token::Word(String::from_utf8_lossy(&out).into_owned()));
                }
                _ => {
                    let start = self.at;
                    while let Some(&c) = self.text.get(self.at) {
                        if matches!(c, b'\n' | b'\r' | b' ' | b'\t' | b'=' | b',' | b';' | b'#' | b'"' | b'{' | b'}') {
                            break;
                        }
                        self.at += 1;
                    }
                    return Ok(Token::Word(String::from_utf8_lossy(&self.text[start..self.at]).into_owned()));
                }
            }
        }
    }

    /// The value token after `=`: a word (`BCT_STRING`/`BCT_NAME`).
    fn word(&mut self) -> Result<String, BsrError> {
        match self.next()? {
            Token::Word(w) => Ok(w),
            other => Err(self.error(format!("expected a string, got {other:?}"))),
        }
    }

    /// `ScanToEol`.
    fn scan_to_eol(&mut self) -> Result<(), BsrError> {
        loop {
            match self.next()? {
                Token::Eol | Token::Eof => return Ok(()),
                _ => {}
            }
        }
    }

    /// A list of values separated by commas (the `for (;;)` loops of the
    /// `store_*` handlers); the token after the list is consumed.
    fn list<T>(&mut self, mut item: impl FnMut(&Self, &str) -> Result<T, BsrError>) -> Result<Vec<T>, BsrError> {
        let mut out = Vec::new();
        loop {
            let w = self.word()?;
            out.push(item(self, &w)?);
            if self.next()? != Token::Comma {
                return Ok(out);
            }
        }
    }
}

/// `scan_pint64`: a positive integer.
fn pint(lexer: &Lexer<'_>, text: &str) -> Result<u64, BsrError> {
    text.trim().parse::<u64>().map_err(|_| lexer.error(format!("expected a positive integer number, got: {text}")))
}

/// `BCT_PINT32_RANGE`/`BCT_PINT64_RANGE`: `n` or `a-b`.
fn range(lexer: &Lexer<'_>, text: &str) -> Result<(u64, u64), BsrError> {
    if text.bytes().all(|b| b.is_ascii_digit()) {
        let v = pint(lexer, text)?;
        return Ok((v, v));
    }
    let Some((a, b)) = text.split_once('-') else {
        return Err(lexer.error(format!("expected an integer or a range, got: {text}")));
    };
    Ok((pint(lexer, a)?, pint(lexer, b)?))
}

/// A 32-bit range.
fn range32(lexer: &Lexer<'_>, text: &str) -> Result<(u32, u32), BsrError> {
    let (a, b) = range(lexer, text)?;
    Ok((a as u32, b as u32))
}

impl Bsr {
    /// `parse_bsr`: the bootstrap `text`.
    pub fn parse(text: &str) -> Result<Bsr, BsrError> {
        let mut lexer = Lexer { text: text.as_bytes(), at: 0, line: 1 };
        let mut entries = vec![BsrEntry::default()];
        loop {
            let keyword = match lexer.next()? {
                Token::Eof => break,
                Token::Eol => continue,
                Token::Word(w) => w,
                other => return Err(lexer.error(format!("Keyword {other:?} not found"))),
            };
            if lexer.next()? != Token::Equals {
                return Err(lexer.error(format!("expected an equals, got: {keyword}")));
            }
            let Some(bsr) = entries.last_mut() else { unreachable!("entries is never empty") };
            match keyword.to_ascii_lowercase().as_str() {
                "volume" => {
                    let value = lexer.word()?;
                    let bsr = if bsr.volumes.is_empty() {
                        bsr
                    } else {
                        entries.push(BsrEntry::default());
                        entries.last_mut().unwrap_or_else(|| unreachable!())
                    };
                    for name in value.split('|').filter(|n| !n.is_empty()) {
                        bsr.volumes.push(BsrVolume { name: name.to_string(), ..Default::default() });
                    }
                }
                "mediatype" => {
                    let value = lexer.word()?;
                    bsr.volumes.iter_mut().for_each(|v| v.media_type = value.clone());
                }
                "device" => {
                    let value = lexer.word()?;
                    bsr.volumes.iter_mut().for_each(|v| v.device = value.clone());
                }
                "storage" => {
                    lexer.word()?;
                }
                "slot" => {
                    let w = lexer.word()?;
                    let slot = pint(&lexer, &w)? as i32;
                    if let Some(v) = bsr.volumes.first_mut() {
                        v.slot = slot;
                    }
                    lexer.scan_to_eol()?;
                }
                "count" => {
                    let w = lexer.word()?;
                    bsr.count = pint(&lexer, &w)? as u32;
                    lexer.scan_to_eol()?;
                }
                "client" => bsr.client.extend(lexer.list(|_, w| Ok(w.to_string()))?),
                "job" => bsr.job.extend(lexer.list(|_, w| Ok(w.to_string()))?),
                "jobid" => bsr.job_id.extend(lexer.list(range32)?),
                "fileindex" => bsr.file_index.extend(lexer.list(range32)?.into_iter().map(|(a, b)| (a as i32, b as i32))),
                "volsessionid" => bsr.sessid.extend(lexer.list(range32)?),
                "volsessiontime" => bsr.sesstime.extend(lexer.list(|l, w| Ok(pint(l, w)? as u32))?),
                "volfile" => bsr.volfile.extend(lexer.list(range32)?),
                "volblock" => bsr.volblock.extend(lexer.list(range32)?),
                "voladdr" => bsr.voladdr.extend(lexer.list(range)?),
                "stream" => bsr.stream.extend(lexer.list(|l, w| w.trim().parse::<i32>().map_err(|_| l.error(format!("expected an integer number, got: {w}"))))?),
                "fileregex" => {
                    let value = lexer.word()?;
                    PosixRegex::new(&value).map_err(|e| lexer.error(format!("REGEX '{value}' compile error. ERR={e}")))?;
                    bsr.fileregex = Some(value);
                }
                "include" | "exclude" => lexer.scan_to_eol()?,
                "jobtype" | "joblevel" => lexer.scan_to_eol()?,
                _ => return Err(lexer.error(format!("Keyword {keyword} not found"))),
            }
        }
        let use_fast_rejection = entries.iter().all(|e| !e.sesstime.is_empty() && !e.sessid.is_empty());
        let use_positioning = entries.iter().all(|e| !e.voladdr.is_empty() || (!e.volfile.is_empty() && !e.volblock.is_empty()));
        Ok(Bsr { entries, use_fast_rejection, use_positioning })
    }

    /// `CreateRestoreVolumeList`: every volume once, in bootstrap order;
    /// empty when the first section names no volume.
    pub fn restore_volumes(&self) -> Vec<RestoreVolume> {
        let mut out: Vec<RestoreVolume> = Vec::new();
        if self.entries.first().is_none_or(|e| e.volumes.first().is_none_or(|v| v.name.is_empty())) {
            return out;
        }
        for entry in &self.entries {
            let mut sfile = entry.volfile.iter().map(|&(s, _)| s).min().unwrap_or(u32::MAX);
            for v in &entry.volumes {
                match out.iter_mut().find(|o| o.name == v.name) {
                    Some(o) => o.start_file = o.start_file.min(sfile),
                    None => out.push(RestoreVolume {
                        name: v.name.clone(),
                        media_type: v.media_type.clone(),
                        device: v.device.clone(),
                        slot: v.slot,
                        start_file: sfile,
                    }),
                }
                sfile = 0;
            }
        }
        out
    }

    /// The address ranges to read on `volume`, sorted and merged; `None`
    /// when some section naming it has no position (read it all). A
    /// section with `VolFile`/`VolBlock` gives `(sfile<<32|sblock,
    /// efile<<32|eblock)` per pair (`GetBsrStartAddr`).
    pub fn volume_ranges(&self, volume: &str) -> Option<Vec<(u64, u64)>> {
        let mut ranges = Vec::new();
        for e in self.entries.iter().filter(|e| e.volumes.iter().any(|v| v.name == volume)) {
            if !e.voladdr.is_empty() {
                ranges.extend_from_slice(&e.voladdr);
            } else if !e.volfile.is_empty() && !e.volblock.is_empty() {
                for &(sf, ef) in &e.volfile {
                    let sb = e.volblock.iter().map(|&(s, _)| s).min().unwrap_or(0);
                    let eb = e.volblock.iter().map(|&(_, e)| e).max().unwrap_or(u32::MAX);
                    ranges.push(((u64::from(sf) << 32) | u64::from(sb), (u64::from(ef) << 32) | u64::from(eb)));
                }
            } else {
                return None;
            }
        }
        ranges.sort_unstable();
        let mut merged: Vec<(u64, u64)> = Vec::new();
        for (s, e) in ranges {
            match merged.last_mut() {
                Some(last) if s <= last.1.saturating_add(1) => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        Some(merged)
    }
}

/// A compiled POSIX extended regular expression (`regcomp` with
/// `REG_EXTENDED | REG_NOSUB`, as `store_fileregex`).
struct PosixRegex(crate::posix_regex::Regex);

impl PosixRegex {
    /// Compiles `pattern`; the error text of `regerror` on failure.
    fn new(pattern: &str) -> Result<PosixRegex, String> {
        let opts = crate::posix_regex::Options { nosub: true, ..Default::default() };
        crate::posix_regex::Regex::new(pattern.as_bytes(), opts).map(PosixRegex)
    }

    /// `regexec(...) == 0` for `name`.
    fn is_match(&self, name: &[u8]) -> bool {
        self.0.is_match(name)
    }
}

/// What [`BsrMatcher::match_record`] decided (`MatchBsr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Match {
    /// The record is wanted; the index of the section that matched
    /// (`rec->bsr` when that section has `Count` and `FileIndex`).
    Yes(Option<usize>),
    /// Not wanted.
    No,
    /// No further record can match: the restore is complete (`-1`).
    Done,
}

/// The record fields the matcher looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordKey<'a> {
    /// Name of the volume the record is read from (`VolHdr.VolumeName`).
    pub volume: &'a str,
    /// Address of the last byte of the block the record was read from
    /// (`GetRecordAddress`: on a file device `rec->File`/`rec->Block` are
    /// `dcr->EndFile`/`dcr->EndBlock` of that block, `stored/block.cc`).
    pub address: u64,
    /// `VolSessionId`.
    pub vol_session_id: u32,
    /// `VolSessionTime`.
    pub vol_session_time: u32,
    /// `FileIndex`.
    pub file_index: i32,
    /// Stream (positive).
    pub stream: i32,
    /// The record data (file name of an attributes record for `FileRegex`).
    pub data: &'a [u8],
}

/// The "done" flags of one section (`done`, `found` and the local `done`
/// of its items in `stored/bsr.h`).
#[derive(Debug, Clone, Default)]
struct EntryState {
    /// `bsr->done`.
    done: bool,
    /// `bsr->found`.
    found: u32,
    /// `bsr->skip_file` (`FileRegex`).
    skip_file: bool,
    /// `volfile->done` per item.
    volfile: Vec<bool>,
    /// `voladdr->done` per item.
    voladdr: Vec<bool>,
    /// `findex->done` per item.
    findex: Vec<bool>,
    /// `sesstime->done` per item.
    sesstime: Vec<bool>,
}

/// A bootstrap with the state bareos-sd keeps while reading
/// (`jcr->sd_impl->read_session.bsr`).
pub struct BsrMatcher {
    /// The bootstrap.
    bsr: Bsr,
    /// Per section.
    state: Vec<EntryState>,
    /// Compiled `FileRegex` per section.
    regex: Vec<Option<PosixRegex>>,
    /// `root->Reposition`: some section became done.
    pub reposition: bool,
}

impl BsrMatcher {
    /// A matcher for `bsr`, nothing done yet.
    pub fn new(bsr: Bsr) -> BsrMatcher {
        let state = bsr
            .entries
            .iter()
            .map(|e| EntryState {
                volfile: vec![false; e.volfile.len()],
                voladdr: vec![false; e.voladdr.len()],
                findex: vec![false; e.file_index.len()],
                sesstime: vec![false; e.sesstime.len()],
                ..Default::default()
            })
            .collect();
        let regex = bsr.entries.iter().map(|e| e.fileregex.as_deref().and_then(|r| PosixRegex::new(r).ok())).collect();
        BsrMatcher { bsr, state, regex, reposition: false }
    }

    /// The bootstrap.
    pub fn bsr(&self) -> &Bsr {
        &self.bsr
    }

    /// Whether section `i` is done.
    pub fn is_done(&self, i: usize) -> bool {
        self.state.get(i).is_some_and(|s| s.done)
    }

    /// `MatchBsrBlock`: false when a block with this session (block
    /// version 2) cannot hold a wanted record.
    pub fn match_block(&self, vol_session_id: u32, vol_session_time: u32, block_version: u8) -> bool {
        if !self.bsr.use_fast_rejection || block_version < 2 {
            return true;
        }
        self.bsr.entries.iter().any(|e| {
            (e.sesstime.is_empty() || e.sesstime.contains(&vol_session_time))
                && (e.sessid.is_empty() || e.sessid.iter().any(|&(a, b)| a <= vol_session_id && vol_session_id <= b))
        })
    }

    /// `MatchBsr`/`MatchAll` for one record; `session` is the SOS label of
    /// its session, if seen (for `JobId`, `Job`, `Client`).
    pub fn match_record(&mut self, rec: &RecordKey<'_>, session: Option<&SessionLabel>) -> Match {
        self.reposition = false;
        let mut all_done = true;
        for i in 0..self.bsr.entries.len() {
            if self.match_entry(i, rec, session) {
                self.reposition = false;
                let e = &self.bsr.entries[i];
                return Match::Yes((e.count > 0 && !e.file_index.is_empty()).then_some(i));
            }
            all_done = all_done && self.state[i].done;
        }
        if !self.bsr.use_positioning {
            self.reposition = false;
        }
        if all_done && !self.bsr.entries.is_empty() { Match::Done } else { Match::No }
    }

    /// Marks section `i` done (`bsr->done = true; root->Reposition = true`).
    fn set_done(&mut self, i: usize) {
        self.state[i].done = true;
        self.reposition = true;
    }

    /// `MatchAll` for section `i` without the recursion.
    fn match_entry(&mut self, i: usize, rec: &RecordKey<'_>, session: Option<&SessionLabel>) -> bool {
        if self.state[i].done {
            return false;
        }
        let e = &self.bsr.entries[i];
        if !e.volumes.iter().any(|v| v.name == rec.volume) {
            return false;
        }
        // MatchVolfile
        if !e.volfile.is_empty() {
            let file = (rec.address >> 32) as u32;
            let mut hit = false;
            let mut done = true;
            for (k, &(s, f)) in e.volfile.iter().enumerate() {
                if s <= file && file <= f {
                    hit = true;
                    break;
                }
                if file > f {
                    self.state[i].volfile[k] = true;
                }
                done = done && self.state[i].volfile[k];
            }
            if !hit {
                if done {
                    self.set_done(i);
                }
                return false;
            }
        }
        // MatchVoladdr
        let e = &self.bsr.entries[i];
        if !e.voladdr.is_empty() {
            let mut hit = false;
            let mut done = true;
            for (k, &(s, f)) in e.voladdr.iter().enumerate() {
                if s <= rec.address && rec.address <= f {
                    hit = true;
                    break;
                }
                if rec.address > f {
                    self.state[i].voladdr[k] = true;
                }
                done = done && self.state[i].voladdr[k];
            }
            if !hit {
                if done {
                    self.set_done(i);
                }
                return false;
            }
        }
        // MatchSesstime
        let e = &self.bsr.entries[i];
        if !e.sesstime.is_empty() {
            let mut hit = false;
            let mut done = true;
            for (k, &t) in e.sesstime.iter().enumerate() {
                if t == rec.vol_session_time {
                    hit = true;
                    break;
                }
                if rec.vol_session_time > t {
                    self.state[i].sesstime[k] = true;
                }
                done = done && self.state[i].sesstime[k];
            }
            if !hit {
                if done {
                    self.set_done(i);
                }
                return false;
            }
        }
        // MatchSessid (never sets done: records may be interleaved)
        let e = &self.bsr.entries[i];
        if !e.sessid.is_empty() && !e.sessid.iter().any(|&(a, b)| a <= rec.vol_session_id && rec.vol_session_id <= b) {
            return false;
        }
        // MatchFindex
        if !e.file_index.is_empty() {
            let mut hit = false;
            let mut done = true;
            for (k, &(a, b)) in e.file_index.iter().enumerate() {
                if !self.state[i].findex[k] {
                    if a <= rec.file_index && rec.file_index <= b {
                        hit = true;
                        break;
                    }
                    if rec.file_index > b {
                        self.state[i].findex[k] = true;
                    }
                }
                done = done && self.state[i].findex[k];
            }
            if !hit {
                if done {
                    self.set_done(i);
                }
                return false;
            }
        }
        // MatchFileregex
        if let Some(re) = &self.regex[i] {
            let masked = rec.stream & crate::stream::STREAMMASK_TYPE;
            if masked == crate::stream::STREAM_UNIX_ATTRIBUTES || masked == crate::stream::STREAM_UNIX_ATTRIBUTES_EX {
                // "<FileIndex> <Type> <Filename>\0<Attributes>\0…"
                let mut parts = rec.data.splitn(3, |&b| b == b' ');
                let _ = (parts.next(), parts.next());
                let name = parts.next().and_then(|r| r.split(|&b| b == 0).next()).unwrap_or_default();
                self.state[i].skip_file = !re.is_match(name);
            }
        }
        if self.state[i].skip_file {
            return false;
        }
        let e = &self.bsr.entries[i];
        if e.count > 0 && !e.file_index.is_empty() {
            return true;
        }
        let jobid = session.map(|s| s.job_id).unwrap_or(0);
        if !e.job_id.is_empty() && !e.job_id.iter().any(|&(a, b)| a <= jobid && jobid <= b) {
            return false;
        }
        let job = session.map(|s| s.job.as_str()).unwrap_or("");
        if !e.job.is_empty() && !e.job.iter().any(|j| j == job) {
            return false;
        }
        let client = session.map(|s| s.client_name.as_str()).unwrap_or("");
        if !e.client.is_empty() && !e.client.iter().any(|c| c == client) {
            return false;
        }
        e.stream.is_empty() || e.stream.contains(&rec.stream)
    }

    /// `IsThisBsrDone`: after the records of one file of section `entry`
    /// (the caller saw the FileIndex change): `found + 1`; true when the
    /// section has reached its `Count` and is done now.
    pub fn file_done(&mut self, entry: usize) -> bool {
        let count = self.bsr.entries[entry].count;
        let s = &mut self.state[entry];
        s.found += 1;
        if count > 0 && s.found >= count {
            self.set_done(entry);
            return true;
        }
        false
    }

    /// `find_next_bsr` + `GetBsrStartAddr`: the smallest start address of
    /// a section on `volume` that is not done; `None` when every section of
    /// that volume is done (mount the next volume).
    pub fn next_address(&self, volume: &str) -> Option<u64> {
        self.bsr
            .entries
            .iter()
            .enumerate()
            .filter(|(i, e)| !self.state[*i].done && e.volumes.iter().any(|v| v.name == volume))
            .map(|(i, e)| {
                let addr = e
                    .voladdr
                    .iter()
                    .zip(&self.state[i].voladdr)
                    .filter(|(_, d)| !**d)
                    .map(|(&(s, _), _)| s)
                    .min();
                addr.unwrap_or_else(|| {
                    let f = e.volfile.iter().map(|&(s, _)| s).min().unwrap_or(0);
                    let b = e.volblock.iter().map(|&(s, _)| s).min().unwrap_or(0);
                    (u64::from(f) << 32) | u64::from(b)
                })
            })
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bootstrap odir wrote for the rig backup (one volume, one job).
    const RIG: &str = "Storage=\"vsd\"\nVolume=\"VVol-0001\"\nMediaType=\"VFile\"\nVolSessionId=1\n\
        VolSessionTime=1795204762\nVolAddr=226-12592458\nFileIndex=1-13\nCount=13\n";

    /// A job over two volumes, as odir writes it for a multi-volume backup.
    const TWO: &str = "Storage=\"vsd\"\nVolume=\"VVol-0001\"\nMediaType=\"VFile\"\nVolSessionId=3\nVolSessionTime=100\n\
        VolAddr=226-4999999\nFileIndex=1-7\nCount=7\nStorage=\"vsd\"\nVolume=\"VVol-0002\"\nMediaType=\"VFile\"\n\
        VolSessionId=3\nVolSessionTime=100\nVolAddr=230-3000000\nFileIndex=7-13\nCount=7\n";

    /// A record key on `volume` at `address` of session 1/1795204762.
    fn key(volume: &str, address: u64, file_index: i32) -> RecordKey<'_> {
        RecordKey { volume, address, vol_session_id: 1, vol_session_time: 1_795_204_762, file_index, stream: 1, data: b"" }
    }

    #[test]
    fn rig_bootstrap_is_parsed_like_parse_bsr() {
        let bsr = Bsr::parse(RIG).unwrap();
        assert_eq!(bsr.entries.len(), 1);
        let e = &bsr.entries[0];
        assert_eq!(e.volumes, vec![BsrVolume { name: "VVol-0001".into(), media_type: "VFile".into(), device: String::new(), slot: 0 }]);
        assert_eq!((e.sessid.as_slice(), e.sesstime.as_slice()), (&[(1, 1)][..], &[1_795_204_762][..]));
        assert_eq!((e.voladdr.as_slice(), e.file_index.as_slice(), e.count), (&[(226, 12_592_458)][..], &[(1, 13)][..], 13));
        assert!(bsr.use_fast_rejection && bsr.use_positioning);
    }

    #[test]
    fn lists_ranges_comments_and_case() {
        let text = "# comment\nvolume=A|B ; MEDIATYPE = \"File X\"\nDevice=Dev\nSlot=3 rest ignored\n\
            FileIndex=1-3,5,7-9\nJobId=4\nJob=J.1,J.2\nClient=c1\nStream=1,-2\nVolFile=0-0\nVolBlock=10-20\nInclude = anything\n";
        let bsr = Bsr::parse(text).unwrap();
        let e = &bsr.entries[0];
        assert_eq!(e.volumes.iter().map(|v| (v.name.as_str(), v.media_type.as_str(), v.device.as_str())).collect::<Vec<_>>(),
            vec![("A", "File X", "Dev"), ("B", "File X", "Dev")]);
        assert_eq!(e.volumes[0].slot, 3);
        assert_eq!(e.file_index, vec![(1, 3), (5, 5), (7, 9)]);
        assert_eq!((e.job_id.clone(), e.job.clone(), e.client.clone(), e.stream.clone()), (vec![(4, 4)], vec!["J.1".into(), "J.2".into()], vec!["c1".into()], vec![1, -2]));
        assert!(!bsr.use_fast_rejection && bsr.use_positioning);
        assert_eq!(bsr.volume_ranges("A"), Some(vec![(10, 20)]));
    }

    #[test]
    fn errors_name_the_line() {
        let e = Bsr::parse("Volume=V\nFrobnicate=1\n").unwrap_err();
        assert_eq!(e.line, 2);
        assert!(e.message.contains("Keyword Frobnicate not found"), "{e}");
        assert!(Bsr::parse("Volume V\n").unwrap_err().message.contains("expected an equals"));
        assert!(Bsr::parse("Volume=V\nFileIndex=x-3\n").unwrap_err().message.contains("positive integer"));
        assert!(Bsr::parse("Volume=V\nFileIndex=3;\n").is_ok());
        assert!(Bsr::parse("Volume=V\nFileRegex=(\n").unwrap_err().message.contains("compile error"));
    }

    #[test]
    fn each_volume_starts_a_section_and_the_volume_list_is_deduplicated() {
        let bsr = Bsr::parse(&format!("{TWO}Volume=\"VVol-0001\"\nVolAddr=5000000-6000000\nFileIndex=20\n")).unwrap();
        assert_eq!(bsr.entries.len(), 3);
        let vols: Vec<String> = bsr.restore_volumes().into_iter().map(|v| v.name).collect();
        assert_eq!(vols, ["VVol-0001", "VVol-0002"]);
        assert_eq!(bsr.volume_ranges("VVol-0001"), Some(vec![(226, 6_000_000)]));
        assert_eq!(bsr.volume_ranges("VVol-0002"), Some(vec![(230, 3_000_000)]));
        assert!(Bsr::parse("FileIndex=1\n").unwrap().restore_volumes().is_empty());
        assert_eq!(Bsr::parse("Volume=X\nFileIndex=1\n").unwrap().volume_ranges("X"), None);
    }

    #[test]
    fn records_are_filtered_by_volume_session_address_and_file_index() {
        let mut m = BsrMatcher::new(Bsr::parse("Volume=V\nVolSessionId=1\nVolSessionTime=1795204762\nVolAddr=100-200\nFileIndex=2-3,5\nCount=3\n").unwrap());
        assert_eq!(m.match_record(&key("W", 150, 2), None), Match::No);
        assert_eq!(m.match_record(&key("V", 50, 2), None), Match::No);
        assert_eq!(m.match_record(&key("V", 150, 1), None), Match::No);
        assert_eq!(m.match_record(&key("V", 150, 2), None), Match::Yes(Some(0)));
        assert_eq!(m.match_record(&key("V", 150, 4), None), Match::No);
        assert_eq!(m.match_record(&key("V", 150, 5), None), Match::Yes(Some(0)));
        let other = RecordKey { vol_session_id: 2, ..key("V", 150, 5) };
        assert_eq!(m.match_record(&other, None), Match::No);
        // past the last FileIndex: the section is done, no match is possible
        assert_eq!(m.match_record(&key("V", 150, 6), None), Match::Done);
        assert!(m.is_done(0) && m.reposition);
        assert_eq!(m.match_record(&key("V", 150, 5), None), Match::Done);
    }

    #[test]
    fn an_address_past_every_range_ends_the_section() {
        let mut m = BsrMatcher::new(Bsr::parse("Volume=V\nVolAddr=100-200,300-400\n").unwrap());
        assert_eq!(m.match_record(&key("V", 250, 1), None), Match::No);
        assert_eq!(m.match_record(&key("V", 350, 1), None), Match::Yes(None));
        assert_eq!(m.match_record(&key("V", 401, 1), None), Match::Done);
    }

    #[test]
    fn count_marks_a_section_done_and_next_address_moves_on() {
        let mut m = BsrMatcher::new(Bsr::parse(TWO).unwrap());
        assert_eq!(m.next_address("VVol-0001"), Some(226));
        assert!(!m.file_done(0));
        for _ in 0..5 {
            m.file_done(0);
        }
        assert!(m.file_done(0), "7 files found");
        assert!(m.is_done(0));
        assert_eq!(m.next_address("VVol-0001"), None, "mount the next volume");
        assert_eq!(m.next_address("VVol-0002"), Some(230));
        // a record of the continuation on the second volume still matches
        let k = RecordKey { volume: "VVol-0002", address: 230, vol_session_id: 3, vol_session_time: 100, file_index: 7, stream: 2, data: b"" };
        assert_eq!(m.match_record(&k, None), Match::Yes(Some(1)));
    }

    #[test]
    fn job_client_and_stream_need_the_session_label_when_there_is_no_count() {
        let mut m = BsrMatcher::new(Bsr::parse("Volume=V\nJobId=5-6\nClient=c\nStream=2\n").unwrap());
        let sos = SessionLabel { job_id: 5, client_name: "c".into(), ..Default::default() };
        let k = RecordKey { stream: 2, ..key("V", 1, 1) };
        assert_eq!(m.match_record(&k, Some(&sos)), Match::Yes(None));
        assert_eq!(m.match_record(&RecordKey { stream: 1, ..k }, Some(&sos)), Match::No);
        assert_eq!(m.match_record(&k, Some(&SessionLabel { job_id: 7, ..sos.clone() })), Match::No);
        assert_eq!(m.match_record(&k, None), Match::No);
    }

    #[test]
    fn file_regex_skips_every_record_of_an_unwanted_file() {
        let mut m = BsrMatcher::new(Bsr::parse("Volume=V\nFileRegex=\\.txt$\n").unwrap());
        let attrs = |name: &str| format!("1 3 {name}\0P0A\0\0").into_bytes();
        let a = attrs("/d/a.bin");
        let k = RecordKey { stream: crate::stream::STREAM_UNIX_ATTRIBUTES, data: &a, ..key("V", 1, 1) };
        assert_eq!(m.match_record(&k, None), Match::No);
        assert_eq!(m.match_record(&RecordKey { stream: 2, data: b"x", ..k }, None), Match::No, "data of a.bin");
        let b = attrs("/d/b.txt");
        assert_eq!(m.match_record(&RecordKey { data: &b, file_index: 2, ..k }, None), Match::Yes(None));
        assert_eq!(m.match_record(&RecordKey { stream: 2, data: b"x", file_index: 2, ..k }, None), Match::Yes(None));
    }

    #[test]
    fn fast_rejection_uses_the_block_session() {
        let m = BsrMatcher::new(Bsr::parse(RIG).unwrap());
        assert!(m.match_block(1, 1_795_204_762, 2));
        assert!(!m.match_block(2, 1_795_204_762, 2));
        assert!(m.match_block(2, 1_795_204_762, 1), "BB01 blocks carry no session");
    }
}
