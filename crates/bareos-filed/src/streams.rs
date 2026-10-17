//! Stream ids of include/streams.h.
#![allow(dead_code)]

pub const STREAMMASK_TYPE: i32 = 0x7ff;

pub const STREAM_UNIX_ATTRIBUTES: i32 = 1;
pub const STREAM_FILE_DATA: i32 = 2;
pub const STREAM_MD5_DIGEST: i32 = 3;
pub const STREAM_GZIP_DATA: i32 = 4;
pub const STREAM_UNIX_ATTRIBUTES_EX: i32 = 5;
pub const STREAM_SPARSE_DATA: i32 = 6;
pub const STREAM_SPARSE_GZIP_DATA: i32 = 7;
pub const STREAM_PROGRAM_NAMES: i32 = 8;
pub const STREAM_PROGRAM_DATA: i32 = 9;
pub const STREAM_SHA1_DIGEST: i32 = 10;
pub const STREAM_WIN32_DATA: i32 = 11;
pub const STREAM_WIN32_GZIP_DATA: i32 = 12;
pub const STREAM_MACOS_FORK_DATA: i32 = 13;
pub const STREAM_HFSPLUS_ATTRIBUTES: i32 = 14;
pub const STREAM_UNIX_ACCESS_ACL: i32 = 15;
pub const STREAM_UNIX_DEFAULT_ACL: i32 = 16;
pub const STREAM_SHA256_DIGEST: i32 = 17;
pub const STREAM_SHA512_DIGEST: i32 = 18;
pub const STREAM_SIGNED_DIGEST: i32 = 19;
pub const STREAM_ENCRYPTED_FILE_DATA: i32 = 20;
pub const STREAM_ENCRYPTED_WIN32_DATA: i32 = 21;
pub const STREAM_ENCRYPTED_SESSION_DATA: i32 = 22;
pub const STREAM_ENCRYPTED_FILE_GZIP_DATA: i32 = 23;
pub const STREAM_ENCRYPTED_WIN32_GZIP_DATA: i32 = 24;
pub const STREAM_ENCRYPTED_MACOS_FORK_DATA: i32 = 25;
pub const STREAM_PLUGIN_NAME: i32 = 26;
pub const STREAM_PLUGIN_DATA: i32 = 27;
pub const STREAM_RESTORE_OBJECT: i32 = 28;
pub const STREAM_COMPRESSED_DATA: i32 = 29;
pub const STREAM_SPARSE_COMPRESSED_DATA: i32 = 30;
pub const STREAM_WIN32_COMPRESSED_DATA: i32 = 31;
pub const STREAM_ENCRYPTED_FILE_COMPRESSED_DATA: i32 = 32;
pub const STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA: i32 = 33;
pub const STREAM_XXH128_DIGEST: i32 = 40;

pub const STREAM_ACL_LINUX_DEFAULT_ACL: i32 = 1007;
pub const STREAM_ACL_LINUX_ACCESS_ACL: i32 = 1008;
pub const STREAM_ACL_PLUGIN: i32 = 1020;
pub const STREAM_XATTR_PLUGIN: i32 = 1988;
pub const STREAM_XATTR_SOLARIS: i32 = 1995;
pub const STREAM_XATTR_LINUX: i32 = 1998;
pub const STREAM_XATTR_NETBSD: i32 = 1999;

/// The ACL streams of restore.cc (STREAM_UNIX_ACCESS_ACL .. STREAM_ACL_PLUGIN).
pub fn is_acl_stream(s: i32) -> bool {
    s == STREAM_UNIX_ACCESS_ACL || s == STREAM_UNIX_DEFAULT_ACL || (1000..=1020).contains(&s)
}

/// The xattr streams of restore.cc (STREAM_XATTR_PLUGIN .. STREAM_XATTR_NETBSD,
/// STREAM_XATTR_SOLARIS included).
pub fn is_xattr_stream(s: i32) -> bool {
    (1988..=1999).contains(&s)
}

/// is_win32_stream() (findlib/bfile.cc).
pub fn is_win32_stream(s: i32) -> bool {
    matches!(s, STREAM_WIN32_DATA | STREAM_WIN32_GZIP_DATA | STREAM_WIN32_COMPRESSED_DATA
        | STREAM_ENCRYPTED_WIN32_DATA | STREAM_ENCRYPTED_WIN32_GZIP_DATA | STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA)
}

/// stream_to_ascii() (findlib/bfile.cc) for the messages.
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
