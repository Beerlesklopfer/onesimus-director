// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Stream-Kennungen: welche Art Daten ein Datensatz trägt.
//!
//! Zweck: Jeder Datensatz, den der FD an den SD schickt und der SD in ein
//! Volume schreibt, hat eine Stream-Kennung: Attribute, Dateidaten (roh,
//! komprimiert, sparse, verschlüsselt), Prüfsummen, ACLs, erweiterte
//! Attribute, Plugin-Daten. Die unteren elf Bit ([`STREAMMASK_TYPE`]) sind
//! die Art, die oberen Bits Zusatzflags.
//!
//! Datenfluss: FD (Sender, `backup`) → SD (Volume) → FD (Empfänger beim
//! Restore) und Director (Verify, Katalog-Einträge `catreq`).
//!
//! Bareos-Gegenstück: `core/src/include/streams.h`,
//! `core/src/findlib/bfile.cc` (`stream_to_ascii`, `is_win32_stream`).
//!
//! Zusammengeführt (19.11.2026) aus `ofd/src/streams.rs` (vollständig),
//! `odir/src/verify.rs` und `odir/src/catreq.rs` (je acht Kennungen).
//! Begriff: „Stream“ heißt hier immer Satzart, nie Netzverbindung
//! ([`crate::glossary`]).

/// Mask of the stream type bits (the rest are flags).
pub const STREAMMASK_TYPE: i32 = 0x7ff;

/// File attributes (LStat, name, link target).
pub const STREAM_UNIX_ATTRIBUTES: i32 = 1;
/// Plain file data.
pub const STREAM_FILE_DATA: i32 = 2;
/// MD5 digest of the file data.
pub const STREAM_MD5_DIGEST: i32 = 3;
/// GZIP compressed file data (old form).
pub const STREAM_GZIP_DATA: i32 = 4;
/// Extended attributes record (Win32 data in the attributes).
pub const STREAM_UNIX_ATTRIBUTES_EX: i32 = 5;
/// Sparse file data (each block with its offset).
pub const STREAM_SPARSE_DATA: i32 = 6;
/// GZIP compressed sparse file data.
pub const STREAM_SPARSE_GZIP_DATA: i32 = 7;
/// Program names (unused by Bareos).
pub const STREAM_PROGRAM_NAMES: i32 = 8;
/// Program data (unused by Bareos).
pub const STREAM_PROGRAM_DATA: i32 = 9;
/// SHA1 digest of the file data.
pub const STREAM_SHA1_DIGEST: i32 = 10;
/// Win32 BackupRead data.
pub const STREAM_WIN32_DATA: i32 = 11;
/// GZIP compressed Win32 data.
pub const STREAM_WIN32_GZIP_DATA: i32 = 12;
/// MacOS resource fork.
pub const STREAM_MACOS_FORK_DATA: i32 = 13;
/// HFS+ attributes.
pub const STREAM_HFSPLUS_ATTRIBUTES: i32 = 14;
/// Standard Unix access ACL.
pub const STREAM_UNIX_ACCESS_ACL: i32 = 15;
/// Standard Unix default ACL.
pub const STREAM_UNIX_DEFAULT_ACL: i32 = 16;
/// SHA256 digest of the file data.
pub const STREAM_SHA256_DIGEST: i32 = 17;
/// SHA512 digest of the file data.
pub const STREAM_SHA512_DIGEST: i32 = 18;
/// Signed digest (PKI).
pub const STREAM_SIGNED_DIGEST: i32 = 19;
/// Encrypted file data (PKI).
pub const STREAM_ENCRYPTED_FILE_DATA: i32 = 20;
/// Encrypted Win32 data.
pub const STREAM_ENCRYPTED_WIN32_DATA: i32 = 21;
/// Encrypted session key (PKI).
pub const STREAM_ENCRYPTED_SESSION_DATA: i32 = 22;
/// Encrypted GZIP file data.
pub const STREAM_ENCRYPTED_FILE_GZIP_DATA: i32 = 23;
/// Encrypted GZIP Win32 data.
pub const STREAM_ENCRYPTED_WIN32_GZIP_DATA: i32 = 24;
/// Encrypted MacOS resource fork.
pub const STREAM_ENCRYPTED_MACOS_FORK_DATA: i32 = 25;
/// Plugin name record.
pub const STREAM_PLUGIN_NAME: i32 = 26;
/// Plugin data record.
pub const STREAM_PLUGIN_DATA: i32 = 27;
/// Restore object (plugin configuration).
pub const STREAM_RESTORE_OBJECT: i32 = 28;
/// Compressed file data with compression header (LZO, LZ4, GZIP …).
pub const STREAM_COMPRESSED_DATA: i32 = 29;
/// Compressed sparse file data.
pub const STREAM_SPARSE_COMPRESSED_DATA: i32 = 30;
/// Compressed Win32 data.
pub const STREAM_WIN32_COMPRESSED_DATA: i32 = 31;
/// Encrypted compressed file data.
pub const STREAM_ENCRYPTED_FILE_COMPRESSED_DATA: i32 = 32;
/// Encrypted compressed Win32 data.
pub const STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA: i32 = 33;
/// XXH128 digest of the file data.
pub const STREAM_XXH128_DIGEST: i32 = 40;

/// Linux default ACL.
pub const STREAM_ACL_LINUX_DEFAULT_ACL: i32 = 1007;
/// Linux access ACL.
pub const STREAM_ACL_LINUX_ACCESS_ACL: i32 = 1008;
/// ACL of a plugin.
pub const STREAM_ACL_PLUGIN: i32 = 1020;
/// Extended attributes of a plugin.
pub const STREAM_XATTR_PLUGIN: i32 = 1988;
/// Solaris extended attributes.
pub const STREAM_XATTR_SOLARIS: i32 = 1995;
/// Darwin (macOS) extended attributes.
pub const STREAM_XATTR_DARWIN: i32 = 1996;
/// Linux extended attributes.
pub const STREAM_XATTR_LINUX: i32 = 1998;
/// NetBSD extended attributes.
pub const STREAM_XATTR_NETBSD: i32 = 1999;

/// The ACL streams of restore.cc (`STREAM_UNIX_ACCESS_ACL` .. `STREAM_ACL_PLUGIN`).
pub fn is_acl_stream(s: i32) -> bool {
    s == STREAM_UNIX_ACCESS_ACL || s == STREAM_UNIX_DEFAULT_ACL || (1000..=1020).contains(&s)
}

/// The xattr streams of restore.cc (`STREAM_XATTR_PLUGIN` .. `STREAM_XATTR_NETBSD`,
/// `STREAM_XATTR_SOLARIS` included).
pub fn is_xattr_stream(s: i32) -> bool {
    (1988..=1999).contains(&s)
}

/// `is_win32_stream()` (findlib/bfile.cc).
pub fn is_win32_stream(s: i32) -> bool {
    matches!(
        s,
        STREAM_WIN32_DATA
            | STREAM_WIN32_GZIP_DATA
            | STREAM_WIN32_COMPRESSED_DATA
            | STREAM_ENCRYPTED_WIN32_DATA
            | STREAM_ENCRYPTED_WIN32_GZIP_DATA
            | STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA
    )
}

/// `CryptoDigestStreamType() != CRYPTO_DIGEST_NONE`: a digest stream.
pub fn is_digest_stream(s: i32) -> bool {
    matches!(s, STREAM_MD5_DIGEST | STREAM_SHA1_DIGEST | STREAM_SHA256_DIGEST | STREAM_SHA512_DIGEST | STREAM_XXH128_DIGEST)
}

/// `crypto_digest_name()` of the digest a digest stream carries
/// (`CryptoDigestStreamType()`), `None` for any other stream.
pub fn digest_name(s: i32) -> Option<&'static str> {
    match s {
        STREAM_MD5_DIGEST => Some("MD5"),
        STREAM_SHA1_DIGEST => Some("SHA1"),
        STREAM_SHA256_DIGEST => Some("SHA256"),
        STREAM_SHA512_DIGEST => Some("SHA512"),
        STREAM_XXH128_DIGEST => Some("XXH128"),
        _ => None,
    }
}

/// `stream_to_ascii()` (findlib/bfile.cc) for the messages; unknown ids as number.
pub fn stream_to_ascii(s: i32) -> String {
    match s {
        STREAM_UNIX_ATTRIBUTES => "Unix attributes".into(),
        STREAM_FILE_DATA => "File data".into(),
        STREAM_MD5_DIGEST => "MD5 digest".into(),
        STREAM_GZIP_DATA => "GZIP data".into(),
        STREAM_COMPRESSED_DATA => "Compressed data".into(),
        STREAM_UNIX_ATTRIBUTES_EX => "Extended attributes".into(),
        STREAM_SPARSE_DATA => "Sparse data".into(),
        STREAM_SPARSE_GZIP_DATA => "GZIP sparse data".into(),
        STREAM_SPARSE_COMPRESSED_DATA => "Compressed sparse data".into(),
        STREAM_PROGRAM_NAMES => "Program names".into(),
        STREAM_PROGRAM_DATA => "Program data".into(),
        STREAM_SHA1_DIGEST => "SHA1 digest".into(),
        STREAM_WIN32_DATA => "Win32 data".into(),
        STREAM_WIN32_GZIP_DATA => "Win32 GZIP data".into(),
        STREAM_WIN32_COMPRESSED_DATA => "Win32 compressed data".into(),
        STREAM_MACOS_FORK_DATA => "MacOS Fork data".into(),
        STREAM_HFSPLUS_ATTRIBUTES => "HFS+ attribs".into(),
        STREAM_UNIX_ACCESS_ACL => "Standard Unix ACL attribs".into(),
        STREAM_UNIX_DEFAULT_ACL => "Default Unix ACL attribs".into(),
        STREAM_SHA256_DIGEST => "SHA256 digest".into(),
        STREAM_SHA512_DIGEST => "SHA512 digest".into(),
        STREAM_SIGNED_DIGEST => "Signed digest".into(),
        STREAM_ENCRYPTED_FILE_DATA => "Encrypted File data".into(),
        STREAM_ENCRYPTED_WIN32_DATA => "Encrypted Win32 data".into(),
        STREAM_ENCRYPTED_SESSION_DATA => "Encrypted session data".into(),
        STREAM_ENCRYPTED_FILE_GZIP_DATA => "Encrypted GZIP data".into(),
        STREAM_ENCRYPTED_FILE_COMPRESSED_DATA => "Encrypted compressed data".into(),
        STREAM_ENCRYPTED_WIN32_GZIP_DATA => "Encrypted Win32 GZIP data".into(),
        STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA => "Encrypted Win32 Compressed data".into(),
        STREAM_ENCRYPTED_MACOS_FORK_DATA => "Encrypted MacOS fork data".into(),
        STREAM_ACL_LINUX_DEFAULT_ACL => "Linux Default ACL attribs".into(),
        STREAM_ACL_LINUX_ACCESS_ACL => "Linux Access ACL attribs".into(),
        STREAM_XATTR_LINUX => "Linux Extended attributes".into(),
        STREAM_PLUGIN_NAME => "Plugin Name".into(),
        STREAM_PLUGIN_DATA => "Plugin Data".into(),
        STREAM_RESTORE_OBJECT => "Restore Object".into(),
        STREAM_XXH128_DIGEST => "XXH128 digest".into(),
        _ => format!("{s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_match_the_replaced_constants() {
        // odir/src/verify.rs and odir/src/catreq.rs (19.11.2026)
        assert_eq!(
            [STREAM_UNIX_ATTRIBUTES, STREAM_MD5_DIGEST, STREAM_UNIX_ATTRIBUTES_EX, STREAM_SHA1_DIGEST,
             STREAM_SHA256_DIGEST, STREAM_SHA512_DIGEST, STREAM_RESTORE_OBJECT, STREAM_XXH128_DIGEST],
            [1, 3, 5, 10, 17, 18, 28, 40]
        );
    }

    #[test]
    fn digest_streams_and_names() {
        let digests: Vec<i32> = (0..2000).filter(|s| is_digest_stream(*s)).collect();
        assert_eq!(digests, [3, 10, 17, 18, 40]);
        // odir verify.rs/catreq.rs stream_to_ascii() for digest streams
        assert_eq!(stream_to_ascii(STREAM_MD5_DIGEST), "MD5 digest");
        assert_eq!(stream_to_ascii(STREAM_XXH128_DIGEST), "XXH128 digest");
        assert_eq!(stream_to_ascii(4711), "4711");
        // ofd backup_cmds.rs do_verify_volume() and digest.rs Digest::name()
        let names: Vec<_> = digests.iter().map(|s| digest_name(*s).unwrap()).collect();
        assert_eq!(names, ["MD5", "SHA1", "SHA256", "SHA512", "XXH128"]);
        assert_eq!(digest_name(STREAM_FILE_DATA), None);
    }

    #[test]
    fn stream_classes() {
        assert!(is_acl_stream(STREAM_ACL_LINUX_ACCESS_ACL) && !is_acl_stream(STREAM_XATTR_LINUX));
        assert!(is_xattr_stream(STREAM_XATTR_SOLARIS) && !is_xattr_stream(STREAM_ACL_PLUGIN));
        assert!(is_win32_stream(STREAM_WIN32_DATA) && !is_win32_stream(STREAM_FILE_DATA));
    }
}
