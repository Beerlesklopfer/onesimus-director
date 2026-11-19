//! Testbaum mit den Sonderfällen eines Datei-Backups.
//!
//! Zweck: Ein Backup soll nicht nur gewöhnliche Dateien sichern. Der Baum
//! enthält deshalb eine leere Datei, eine große Datei, einen Symlink, zwei
//! Hardlinks auf dieselbe Inode, einen Namen, der kein UTF-8 ist, und eine
//! Sparse-Datei. Nach dem Restore vergleicht `diff -r` Quelle und Ziel
//! bitgenau; [`hardlinks_preserved`] prüft zusätzlich die Inode-Gleichheit.
//!
//! Datenfluss: [`build`] schreibt den Baum, [`build_big`] die große Datei des
//! Abbruch-Falls. Inhalte sind deterministisch (xorshift), damit ein
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
    fn pseudo_random_is_deterministic() {
        assert_eq!(pseudo_random(100, 7), pseudo_random(100, 7));
        assert_ne!(pseudo_random(100, 7), pseudo_random(100, 8));
        assert_eq!(pseudo_random(13, 1).len(), 13);
    }
}
