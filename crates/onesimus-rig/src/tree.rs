//! Testbaum mit den Sonderfällen eines Datei-Backups.
//!
//! Zweck: Ein Backup soll nicht nur gewöhnliche Dateien sichern. Der Baum
//! enthält deshalb eine leere Datei, eine große Datei, einen Symlink, zwei
//! Hardlinks auf dieselbe Inode, einen Namen, der kein UTF-8 ist, und eine
//! Sparse-Datei. Nach dem Restore vergleicht `diff -r` Quelle und Ziel
//! bitgenau; [`hardlinks_preserved`] prüft zusätzlich die Inode-Gleichheit.
//!
//! Datenfluss: [`build`] schreibt den Baum, [`build_big`] die große Datei des
//! Abbruch-Falls, [`compare_trees`] vergleicht Quelle und Restore samt
//! Attributen (Rechte, Eigentümer, Zeiten, Symlink-Ziele). Inhalte sind deterministisch (xorshift), damit ein
//! Fehlschlag reproduzierbar ist.
//!
//! Bareos-Gegenstück: die Testdaten der `systemtests` (`tmp/data`), dort meist
//! ein Abzug des Bareos-Quellbaums.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Size of the large file of the main tree (several SD blocks, GZIP).
pub const LARGE_FILE_BYTES: usize = 12 * 1024 * 1024;

/// Size of the file of the cancel case; at the 4 MB/s of the FD it needs
/// about 12 s, enough to cancel it while it runs.
pub const BIG_FILE_BYTES: usize = 48 * 1024 * 1024;

/// Apparent size of the sparse file (most of it is a hole).
pub const SPARSE_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// The file name that is not UTF-8 (Latin-1 `café.txt`).
pub const NON_UTF8_NAME: &[u8] = b"caf\xe9.txt";

/// Deterministic pseudo-random bytes (xorshift64), `seed` != 0.
pub fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.max(1);
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// Writes the source tree of the main backup into `data`.
pub fn build(data: &Path) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    std::fs::create_dir_all(data.join("sub"))?;
    for i in 0..3 {
        std::fs::write(data.join(format!("file{i}.txt")), format!("content {i}\n").repeat(i + 1))?;
    }
    std::fs::write(data.join("sub").join("inner.txt"), "inner\n")?;
    std::fs::write(data.join("empty"), b"")?;
    std::fs::write(data.join("large.bin"), pseudo_random(LARGE_FILE_BYTES, 0x5eed))?;
    std::os::unix::fs::symlink("file0.txt", data.join("link0"))?;
    std::fs::write(data.join("hard1"), "same inode\n")?;
    std::fs::hard_link(data.join("hard1"), data.join("sub").join("hard2"))?;
    std::fs::write(data.join(OsStr::from_bytes(NON_UTF8_NAME)), "latin-1 name\n")?;
    let mut sparse = std::fs::File::create(data.join("sparse.bin"))?;
    sparse.write_all(b"head")?;
    sparse.seek(SeekFrom::Start(SPARSE_FILE_BYTES / 2))?;
    sparse.write_all(b"middle")?;
    sparse.set_len(SPARSE_FILE_BYTES)?;
    sparse.seek(SeekFrom::End(-4))?;
    sparse.write_all(b"tail")?;
    Ok(())
}

/// Writes the single large file of the cancel case into `big`.
pub fn build_big(big: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(big)?;
    std::fs::write(big.join("big.bin"), pseudo_random(BIG_FILE_BYTES, 0xb16))
}

/// Whether `hard1` and `sub/hard2` below `root` share one inode.
pub fn hardlinks_preserved(root: &Path) -> std::io::Result<bool> {
    let a = std::fs::metadata(root.join("hard1"))?;
    let b = std::fs::metadata(root.join("sub").join("hard2"))?;
    Ok(a.dev() == b.dev() && a.ino() == b.ino())
}

/// Every difference between the tree `source` and its restored copy
/// `restored` in what a restore must keep: the set of names, the file
/// type, content, size, permission bits, owner, group, modification time
/// (seconds; for directories too) and symlink targets. Empty when both
/// agree. Hardlinks are checked by [`hardlinks_preserved`].
pub fn compare_trees(source: &Path, restored: &Path) -> Vec<String> {
    let mut diffs = Vec::new();
    compare_entry(source, restored, Path::new(""), &mut diffs);
    diffs
}

/// [`compare_trees`] for the entry `rel` below both roots, recursively.
fn compare_entry(source: &Path, restored: &Path, rel: &Path, diffs: &mut Vec<String>) {
    let (a_path, b_path) = (source.join(rel), restored.join(rel));
    let shown = if rel.as_os_str().is_empty() { ".".to_string() } else { rel.display().to_string() };
    let (a, b) = match (std::fs::symlink_metadata(&a_path), std::fs::symlink_metadata(&b_path)) {
        (Ok(a), Ok(b)) => (a, b),
        (Ok(_), Err(e)) => return diffs.push(format!("{shown}: missing in the restore ({e})")),
        (Err(e), _) => return diffs.push(format!("{shown}: unreadable in the source ({e})")),
    };
    let kind = |m: &std::fs::Metadata| m.mode() & libc::S_IFMT;
    if kind(&a) != kind(&b) {
        return diffs.push(format!("{shown}: file type {:o} != {:o}", kind(&a), kind(&b)));
    }
    if a.mode() & 0o7777 != b.mode() & 0o7777 {
        diffs.push(format!("{shown}: mode {:o} != {:o}", a.mode() & 0o7777, b.mode() & 0o7777));
    }
    let mut check = |what: &str, x: u64, y: u64| {
        if x != y {
            diffs.push(format!("{shown}: {what} {x} != {y}"));
        }
    };
    check("uid", u64::from(a.uid()), u64::from(b.uid()));
    check("gid", u64::from(a.gid()), u64::from(b.gid()));
    if !a.file_type().is_symlink() {
        check("mtime", a.mtime() as u64, b.mtime() as u64);
    }
    if a.file_type().is_file() {
        check("size", a.len(), b.len());
        if a.len() == b.len() && std::fs::read(&a_path).ok() != std::fs::read(&b_path).ok() {
            diffs.push(format!("{shown}: content differs"));
        }
    } else if a.file_type().is_symlink() {
        let (x, y) = (std::fs::read_link(&a_path).ok(), std::fs::read_link(&b_path).ok());
        if x != y {
            diffs.push(format!("{shown}: symlink target {x:?} != {y:?}"));
        }
    } else if a.file_type().is_dir() {
        let names = |p: &Path| -> std::collections::BTreeSet<std::ffi::OsString> {
            std::fs::read_dir(p).map(|d| d.filter_map(|e| e.ok().map(|e| e.file_name())).collect()).unwrap_or_default()
        };
        let (na, nb) = (names(&a_path), names(&b_path));
        for extra in nb.difference(&na) {
            diffs.push(format!("{}: only in the restore", rel.join(extra).display()));
        }
        for name in &na {
            compare_entry(source, restored, &rel.join(name), diffs);
        }
    }
}

/// Allocated bytes of `path` (`st_blocks` × 512), to see whether holes stay.
pub fn allocated_bytes(path: &Path) -> std::io::Result<u64> {
    Ok(std::fs::metadata(path)?.blocks() * 512)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_has_every_special_case() {
        let dir = std::env::temp_dir().join(format!("onesimus-rig-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        build(&dir).unwrap();
        assert_eq!(std::fs::metadata(dir.join("empty")).unwrap().len(), 0);
        assert_eq!(std::fs::metadata(dir.join("large.bin")).unwrap().len(), LARGE_FILE_BYTES as u64);
        assert!(std::fs::symlink_metadata(dir.join("link0")).unwrap().file_type().is_symlink());
        assert!(hardlinks_preserved(&dir).unwrap());
        assert!(dir.join(OsStr::from_bytes(NON_UTF8_NAME)).exists());
        assert_eq!(std::fs::metadata(dir.join("sparse.bin")).unwrap().len(), SPARSE_FILE_BYTES);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compare_trees_sees_content_mode_and_missing_entries() {
        let base = std::env::temp_dir().join(format!("onesimus-rig-cmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (a, b) = (base.join("a"), base.join("b"));
        build(&a).unwrap();
        let copy = std::process::Command::new("cp").arg("-a").arg(&a).arg(&b).status().unwrap();
        assert!(copy.success());
        assert_eq!(compare_trees(&a, &b), Vec::<String>::new());
        std::fs::write(b.join("file0.txt"), "Content 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(b.join("file1.txt"), std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::remove_file(b.join("sub").join("inner.txt")).unwrap();
        let diffs = compare_trees(&a, &b).join("\n");
        assert!(diffs.contains("file0.txt: content differs"), "{diffs}");
        assert!(diffs.contains("file1.txt: mode"), "{diffs}");
        assert!(diffs.contains("sub/inner.txt: missing in the restore"), "{diffs}");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn pseudo_random_is_deterministic() {
        assert_eq!(pseudo_random(100, 7), pseudo_random(100, 7));
        assert_ne!(pseudo_random(100, 7), pseudo_random(100, 8));
        assert_eq!(pseudo_random(13, 1).len(), 13);
    }
}
