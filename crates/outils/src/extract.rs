//! Dateien aus den Datensätzen eines Volumes zurückschreiben (`oextract`).
//!
//! Der [`Extractor`] bekommt die Datensätze in Volume-Reihenfolge. Je Job
//! (Session: `VolSessionId` und `VolSessionTime`) hält er die Datei, die
//! gerade geschrieben wird, und die Pfade der Dateien für Hartlinks.
//!
//! Unterstützt: gewöhnliche Dateien, Verzeichnisse, symbolische Links,
//! Hartlinks, spärliche Dateien, gzip- und `COMPRESS_*`-komprimierte Daten
//! (lz4, fastlz, lzo wenn liblzo2 da ist), Zeit und Rechte (Eigentümer mit
//! [`Options::owner`]). Nicht wiederhergestellt werden Windows-Datenströme,
//! ACLs und erweiterte Attribute, Geräte- und FIFO-Dateien und verschlüsselte
//! Daten; sie werden gezählt und gemeldet.

use std::collections::HashMap;
use std::fs::{File, FileTimes, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use bareos_format::compression::decompress_data;
use bareos_format::label::{SessionLabel, EOS_LABEL, SOS_LABEL};
use bareos_format::lstat::{decode_stat, Stat};
use bareos_format::stream::*;
use bareos_format::volume::VolRecord;

use crate::parse_attributes_raw;

/// Wie wiederhergestellt wird.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Zielverzeichnis; der Pfad der Datei hängt darunter.
    pub dest: PathBuf,
    /// Nur dieser Job (JobId aus dem Session-Label).
    pub job: Option<u32>,
    /// Nur Dateien, deren Pfad mit einem dieser Präfixe beginnt (leer: alle).
    pub prefixes: Vec<String>,
    /// Nichts schreiben, nur melden, was geschrieben würde.
    pub dry_run: bool,
    /// Eigentümer und Gruppe setzen (braucht Root).
    pub owner: bool,
}

/// Was wiederhergestellt wurde.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Counts {
    /// Gewöhnliche Dateien.
    pub files: u64,
    /// Verzeichnisse.
    pub directories: u64,
    /// Symbolische Links und Hartlinks.
    pub links: u64,
    /// Geschriebene Bytes.
    pub bytes: u64,
    /// Übersprungene Einträge (Gerät, FIFO, außerhalb der Auswahl nicht gezählt).
    pub skipped: u64,
    /// Fehler (die Meldung steht in [`Extractor::messages`]).
    pub errors: u64,
}

/// Die Datei, deren Daten gerade kommen.
struct Current {
    /// Zielpfad.
    path: PathBuf,
    /// Offene Datei (`None` im Trockenlauf oder bei Fehler).
    file: Option<File>,
    /// Attribute aus dem Katalogsatz.
    stat: Stat,
    /// Stream-Id des letzten Datenstroms (Fortsetzungen gehören zu ihm).
    last_stream: i32,
}

/// Zustand eines Jobs auf dem Volume.
#[derive(Default)]
struct Session {
    /// JobId aus dem SOS-Label; 0, wenn der Job auf dem Volume fortgesetzt wird.
    job_id: u32,
    /// Nicht wiederherstellen (anderer Job).
    skip: bool,
    /// Die Datei in Arbeit.
    current: Option<Current>,
    /// FileIndex → Zielpfad, für Hartlinks.
    paths: HashMap<i32, PathBuf>,
}

/// Schreibt Dateien aus Volume-Datensätzen.
pub struct Extractor {
    /// Die Optionen.
    options: Options,
    /// Je Session der Zustand.
    sessions: HashMap<(u32, u32), Session>,
    /// Verzeichnisse mit ihren Attributen, Zeit und Rechte am Ende.
    directories: Vec<(PathBuf, Stat)>,
    /// Die Zähler.
    pub counts: Counts,
    /// Meldungen (eine je Zeile) für den Aufrufer.
    pub messages: Vec<String>,
}

/// Der Pfad unter `dest` aus den Rohbytes des Namens, ohne Wurzel,
/// Laufwerksdoppelpunkt und `..`; `None` bei einem Pfad, der aus dem Ziel
/// hinausführt. Auf Unix bleiben die Bytes des Namens erhalten.
pub fn target_path_bytes(dest: &Path, name: &[u8]) -> Option<PathBuf> {
    let mut out = dest.to_path_buf();
    for part in name.split(|&b| b == b'/' || b == b'\\') {
        match part {
            b"" | b"." => {}
            b".." => return None,
            p => {
                let p = p.strip_suffix(b":").unwrap_or(p);
                #[cfg(unix)]
                let piece = <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(p.to_vec());
                #[cfg(not(unix))]
                let piece = std::ffi::OsString::from(String::from_utf8_lossy(p).into_owned());
                if Path::new(&piece).components().any(|c| !matches!(c, Component::Normal(_))) {
                    return None;
                }
                out.push(piece);
            }
        }
    }
    Some(out)
}

/// [`target_path_bytes`] für einen Namen als Text.
pub fn target_path(dest: &Path, name: &str) -> Option<PathBuf> {
    target_path_bytes(dest, name.as_bytes())
}

impl Extractor {
    /// Ein Extraktor für `options`.
    pub fn new(options: Options) -> Self {
        Self { options, sessions: HashMap::new(), directories: Vec::new(), counts: Counts::default(), messages: Vec::new() }
    }

    /// Meldet einen Fehler.
    fn error(&mut self, text: String) {
        self.counts.errors += 1;
        self.messages.push(text);
    }

    /// Ob `name` zur Auswahl gehört.
    fn selected(&self, name: &str) -> bool {
        self.options.prefixes.is_empty() || self.options.prefixes.iter().any(|p| name.starts_with(p.as_str()))
    }

    /// Verarbeitet einen Datensatz.
    pub fn record(&mut self, rec: &VolRecord) {
        let key = (rec.vol_session_id, rec.vol_session_time);
        match rec.file_index {
            SOS_LABEL => {
                let job_id = SessionLabel::parse(&rec.data, false).map(|s| s.job_id).unwrap_or(0);
                let skip = self.options.job.is_some_and(|j| j != job_id);
                self.sessions.insert(key, Session { job_id, skip, ..Session::default() });
            }
            EOS_LABEL => {
                self.finish_current(key);
                self.sessions.remove(&key);
            }
            fi if fi > 0 => {
                let wanted = self.options.job;
                let session = self.sessions.entry(key).or_insert_with(|| Session { skip: wanted.is_some(), ..Session::default() });
                if session.skip {
                    return;
                }
                let stream = rec.stream & STREAMMASK_TYPE;
                match stream {
                    STREAM_UNIX_ATTRIBUTES | STREAM_UNIX_ATTRIBUTES_EX => self.attributes(key, rec),
                    STREAM_FILE_DATA | STREAM_SPARSE_DATA | STREAM_GZIP_DATA | STREAM_SPARSE_GZIP_DATA | STREAM_COMPRESSED_DATA
                    | STREAM_SPARSE_COMPRESSED_DATA => self.data(key, stream, &rec.data),
                    STREAM_WIN32_DATA | STREAM_WIN32_GZIP_DATA | STREAM_WIN32_COMPRESSED_DATA => {
                        self.messages.push("Windows data stream is not restored (oextract writes plain data only)".into());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Ein Attributsatz: die vorige Datei schließen, die neue anlegen.
    fn attributes(&mut self, key: (u32, u32), rec: &VolRecord) {
        self.finish_current(key);
        let Some((raw_name, lstat, raw_link)) = parse_attributes_raw(&rec.data) else {
            self.error(format!("FileIndex {}: attribute record not readable", rec.file_index));
            return;
        };
        let name = String::from_utf8_lossy(&raw_name).into_owned();
        let link = raw_link.clone();
        if !self.selected(&name) {
            return;
        }
        let Some(path) = target_path_bytes(&self.options.dest, &raw_name) else {
            self.error(format!("{name}: path leaves the destination, skipped"));
            return;
        };
        let (stat, link_fi, _) = decode_stat(&lstat);
        let dry = self.options.dry_run;
        if let Some(s) = self.sessions.get_mut(&key) {
            s.paths.insert(rec.file_index, path.clone());
        }
        let kind = stat.mode & 0o170_000;
        let result: std::io::Result<()> = (|| {
            if kind != 0o040_000 {
                if let Some(parent) = path.parent().filter(|_| !dry) {
                    std::fs::create_dir_all(parent)?;
                }
            }
            match kind {
                0o040_000 => {
                    if !dry {
                        std::fs::create_dir_all(&path)?;
                    }
                    self.counts.directories += 1;
                    self.directories.push((path.clone(), stat.clone()));
                }
                0o120_000 if !raw_link.is_empty() => {
                    if !dry {
                        let _ = std::fs::remove_file(&path);
                        make_symlink(&link, &path)?;
                    }
                    self.counts.links += 1;
                }
                0o100_000 | 0 if link_fi > 0 && link_fi != rec.file_index => {
                    let target = self.sessions.get(&key).and_then(|s| s.paths.get(&link_fi)).cloned();
                    match target {
                        Some(t) => {
                            if !dry {
                                let _ = std::fs::remove_file(&path);
                                std::fs::hard_link(&t, &path)?;
                            }
                            self.counts.links += 1;
                        }
                        None => self.messages.push(format!("{name}: hard link target (FileIndex {link_fi}) is not on this volume")),
                    }
                }
                0o100_000 | 0 => {
                    let file = if dry { None } else { Some(OpenOptions::new().write(true).create(true).truncate(true).open(&path)?) };
                    self.counts.files += 1;
                    if let Some(s) = self.sessions.get_mut(&key) {
                        s.current = Some(Current { path: path.clone(), file, stat: stat.clone(), last_stream: 0 });
                    }
                }
                _ => {
                    self.counts.skipped += 1;
                    self.messages.push(format!("{name}: device, FIFO or socket is not restored"));
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error(format!("{}: {e}", path.display()));
        }
    }

    /// Dateidaten (auch komprimiert oder spärlich) an die Datei in Arbeit.
    fn data(&mut self, key: (u32, u32), stream: i32, data: &[u8]) {
        let Some(cur) = self.sessions.get_mut(&key).and_then(|s| s.current.as_mut()) else { return };
        cur.last_stream = stream;
        let sparse = matches!(stream, STREAM_SPARSE_DATA | STREAM_SPARSE_GZIP_DATA | STREAM_SPARSE_COMPRESSED_DATA);
        let compressed = matches!(stream, STREAM_GZIP_DATA | STREAM_SPARSE_GZIP_DATA | STREAM_COMPRESSED_DATA | STREAM_SPARSE_COMPRESSED_DATA);
        let mut body = data;
        let mut offset = None;
        if sparse {
            if body.len() < 8 {
                let path = cur.path.display().to_string();
                self.error(format!("{path}: sparse record too short"));
                return;
            }
            offset = Some(u64::from_be_bytes(body[..8].try_into().unwrap()));
            body = &body[8..];
        }
        let decompressed;
        if compressed {
            match decompress_data(&cur.path.display().to_string(), stream, body) {
                Ok(d) => {
                    decompressed = d;
                    body = &decompressed;
                }
                Err(e) => {
                    let path = cur.path.display().to_string();
                    self.error(format!("{path}: {}", e.trim_end()));
                    return;
                }
            }
        }
        let len = body.len() as u64;
        let result = match cur.file.as_mut() {
            Some(f) => offset.map_or(Ok(()), |o| f.seek(SeekFrom::Start(o)).map(|_| ())).and_then(|()| f.write_all(body)),
            None => Ok(()),
        };
        match result {
            Ok(()) => self.counts.bytes += len,
            Err(e) => {
                let path = cur.path.display().to_string();
                self.error(format!("{path}: {e}"));
            }
        }
    }

    /// Schließt die Datei in Arbeit und setzt Zeit und Rechte.
    fn finish_current(&mut self, key: (u32, u32)) {
        let Some(cur) = self.sessions.get_mut(&key).and_then(|s| s.current.take()) else { return };
        if let Some(file) = cur.file {
            // a sparse file ends with a hole: the size comes from the attributes
            if cur.stat.size > 0 {
                let _ = file.set_len(cur.stat.size as u64);
            }
            drop(file);
            if let Err(e) = apply_attributes(&cur.path, &cur.stat, self.options.owner) {
                self.error(format!("{}: {e}", cur.path.display()));
            }
        }
    }

    /// Das Ende der Volumes: offene Dateien schließen, Verzeichnisse
    /// (zuletzt, tiefste zuerst) mit Zeit und Rechten versehen.
    pub fn finish(&mut self) {
        let keys: Vec<_> = self.sessions.keys().copied().collect();
        for key in keys {
            self.finish_current(key);
        }
        if self.options.dry_run {
            return;
        }
        let mut directories = std::mem::take(&mut self.directories);
        directories.sort_by_key(|(p, _)| std::cmp::Reverse(p.components().count()));
        for (path, stat) in directories {
            if let Err(e) = apply_attributes(&path, &stat, self.options.owner) {
                self.error(format!("{}: {e}", path.display()));
            }
        }
    }
}

/// Ein symbolischer Link (unter Windows nur als Datei-Link).
fn make_symlink(target: &[u8], path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(target), path)
    }
    #[cfg(not(unix))]
    {
        std::os::windows::fs::symlink_file(String::from_utf8_lossy(target).as_ref(), path)
    }
}

/// Rechte, Zeit und auf Wunsch Eigentümer.
fn apply_attributes(path: &Path, stat: &Stat, owner: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if owner {
            let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            if unsafe { libc::lchown(c.as_ptr(), stat.uid, stat.gid) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(stat.mode & 0o7777))?;
    }
    #[cfg(not(unix))]
    let _ = owner;
    let at = |s: i64| UNIX_EPOCH + Duration::from_secs(s.max(0) as u64);
    let times = FileTimes::new().set_accessed(at(stat.atime)).set_modified(at(stat.mtime));
    let file = OpenOptions::new().read(true).open(path)?;
    file.set_times(times)
}
