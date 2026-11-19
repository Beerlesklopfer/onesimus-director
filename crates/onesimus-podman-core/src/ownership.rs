//! `ownership.tsv` of a volume (docs/plans/PODMAN-PLUGIN.md, "ownership.tsv").
//!
//! # Purpose
//!
//! The namespace view of everything the stream leaves out: per path the
//! ns-UID, ns-GID, the full mode, capabilities, ACL in namespace IDs and
//! the SELinux label. The **host** generates it (never the guest) and it
//! follows `data/` in the stream. It is applied only by an online restore
//! under host control; a restore to files leaves it as an ordinary file
//! (0600, euid of the FD).
//!
//! # Data flow
//!
//! `sanitize::classify` → [`OwnershipRow`] → [`Table::push`] (bounded) →
//! [`Table::render`] → bytes behind a scan reference → SD stream.
//!
//! # Counterpart
//!
//! [`parse`] reads it back for the online restore and for tests.

use crate::idmap::IdMap;
use crate::sanitize::OwnershipRow;

/// Default limit of rows per volume.
pub const DEFAULT_MAX_ROWS: usize = 100_000;

/// The format line, first line of every file.
pub const FORMAT: &str = "# onesimus-podman-ownership/1";

/// The rows of one volume plus its header data.
#[derive(Debug, Clone)]
pub struct Table {
    /// Domain id (`root`, `uid-<n>`).
    domain: String,
    /// Volume name.
    volume: String,
    /// Source uid map.
    uid_map: IdMap,
    /// Source gid map.
    gid_map: IdMap,
    /// Rows in scan order.
    rows: Vec<OwnershipRow>,
    /// Limit; more rows are an error.
    max_rows: usize,
}

impl Table {
    /// An empty table with the header data.
    pub fn new(domain: &str, volume: &str, uid_map: IdMap, gid_map: IdMap, max_rows: usize) -> Self {
        Self { domain: domain.into(), volume: volume.into(), uid_map, gid_map, rows: Vec::new(), max_rows }
    }

    /// Adds a row; an error past the limit (the plan: "darüber Fehler").
    pub fn push(&mut self, row: OwnershipRow) -> Result<(), String> {
        if self.rows.len() >= self.max_rows {
            return Err(format!("ownership.tsv of volume {} exceeds {} rows", self.volume, self.max_rows));
        }
        self.rows.push(row);
        Ok(())
    }

    /// Number of rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Rows with special bits (setuid, setgid, sticky).
    pub fn special_count(&self) -> usize {
        self.rows.iter().filter(|r| r.mode & 0o7000 != 0).count()
    }

    /// Rows with capabilities.
    pub fn caps_count(&self) -> usize {
        self.rows.iter().filter(|r| r.caps.is_some()).count()
    }

    /// The file: header lines (`#`), then one tab-separated line per row:
    /// path, ns_uid, ns_gid, mode (octal), caps, acl, selinux (`-` empty).
    pub fn render(&self) -> Vec<u8> {
        let mut out = format!(
            "{FORMAT}\n# domain\t{}\n# volume\t{}\n# uid_map\t{}\n# gid_map\t{}\n# path\tns_uid\tns_gid\tmode\tcaps\tacl\tselinux\n",
            escape(self.domain.as_bytes()),
            escape(self.volume.as_bytes()),
            self.uid_map.compact(),
            self.gid_map.compact()
        )
        .into_bytes();
        for r in &self.rows {
            let field = |v: &Option<String>| v.as_deref().map(|s| escape(s.as_bytes())).unwrap_or_else(|| "-".into());
            out.extend_from_slice(
                format!(
                    "{}\t{}\t{}\t{:04o}\t{}\t{}\t{}\n",
                    escape(&r.relpath),
                    r.ns_uid,
                    r.ns_gid,
                    r.mode,
                    field(&r.caps),
                    field(&r.acl),
                    field(&r.selinux)
                )
                .as_bytes(),
            );
        }
        out
    }
}

/// `%XX` for bytes outside printable ASCII, `%`, tab, newline and `\`, so
/// every path (also non-UTF-8) is one field.
pub fn escape(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        if (0x21..0x7f).contains(&b) && b != b'%' && b != b'\\' || b == b' ' {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    if s.is_empty() {
        s.push('.');
    }
    s
}

/// Inverse of [`escape`].
pub fn unescape(text: &str) -> Option<Vec<u8>> {
    if text == "." {
        return Some(Vec::new());
    }
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    Some(out)
}

/// The rows of a rendered table (header lines skipped).
pub fn parse(bytes: &[u8]) -> Result<Vec<OwnershipRow>, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "ownership.tsv is not ASCII")?;
    let mut lines = text.lines();
    if lines.next() != Some(FORMAT) {
        return Err("not an ownership.tsv".into());
    }
    let opt = |f: &str| -> Option<String> { (f != "-").then(|| unescape(f).map(|b| String::from_utf8_lossy(&b).into_owned())).flatten() };
    lines
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let bad = || format!("bad line: {l}");
            if f.len() != 7 {
                return Err(bad());
            }
            Ok(OwnershipRow {
                relpath: unescape(f[0]).ok_or_else(bad)?,
                ns_uid: f[1].parse().map_err(|_| bad())?,
                ns_gid: f[2].parse().map_err(|_| bad())?,
                mode: u32::from_str_radix(f[3], 8).map_err(|_| bad())?,
                caps: opt(f[4]),
                acl: opt(f[5]),
                selinux: opt(f[6]),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(path: &[u8], mode: u32, caps: Option<&str>) -> OwnershipRow {
        OwnershipRow { relpath: path.to_vec(), ns_uid: 0, ns_gid: 0, mode, caps: caps.map(String::from), acl: None, selinux: None }
    }

    #[test]
    fn renders_header_and_rows_and_parses_back() {
        let m = IdMap::rootless(1000, &[(100000, 65536)]).unwrap();
        let mut t = Table::new("uid-1000", "data", m.clone(), m, 10);
        t.push(row(b"bin/su", 0o4755, None)).unwrap();
        t.push(row(b"ping\tx\xff", 0o755, Some("permitted=0x2000 raw=ab"))).unwrap();
        let bytes = t.render();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with(FORMAT));
        assert!(text.contains("# uid_map\t0:1000:1,1:100000:65536\n"));
        assert!(text.contains("bin/su\t0\t0\t4755\t-\t-\t-\n"));
        assert!(text.contains("ping%09x%FF\t0\t0\t0755\tpermitted=0x2000 raw=ab"));
        assert_eq!(parse(&bytes).unwrap(), vec![row(b"bin/su", 0o4755, None), row(b"ping\tx\xff", 0o755, Some("permitted=0x2000 raw=ab"))]);
        assert_eq!((t.special_count(), t.caps_count(), t.len()), (1, 1, 2));
    }

    #[test]
    fn the_row_limit_is_an_error() {
        let mut t = Table::new("root", "v", IdMap::identity(), IdMap::identity(), 1);
        t.push(row(b"a", 0o4755, None)).unwrap();
        assert!(t.push(row(b"b", 0o4755, None)).is_err());
    }

    #[test]
    fn escaping_round_trips() {
        for p in [&b""[..], b"a b", b"%\\\n", b"\x00\x7f\x80"] {
            assert_eq!(unescape(&escape(p)).unwrap(), p);
        }
    }
}
