//! File digests (lib/crypto.cc crypto_digest_new/Update/Finalize,
//! lib/xxhash.cc): MD5, SHA1, SHA256, SHA512, XXH128 (canonical form).

use md5::Digest as _;

use crate::fileset::{FO_MD5, FO_SHA1, FO_SHA256, FO_SHA512, FO_XXH128, Flags};
use crate::streams::*;

pub enum Digest {
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
    Xxh128(Box<twox_hash::XxHash3_128>),
}

impl Digest {
    /// The digest of the first FO_* digest flag (SetupEncryptionDigests
    /// order) and its stream.
    pub fn for_flags(flags: Flags) -> Option<(Digest, i32)> {
        if flags.has(FO_MD5) {
            Some((Digest::Md5(md5::Md5::new()), STREAM_MD5_DIGEST))
        } else if flags.has(FO_SHA1) {
            Some((Digest::Sha1(sha1::Sha1::new()), STREAM_SHA1_DIGEST))
        } else if flags.has(FO_SHA256) {
            Some((Digest::Sha256(sha2::Sha256::new()), STREAM_SHA256_DIGEST))
        } else if flags.has(FO_SHA512) {
            Some((Digest::Sha512(sha2::Sha512::new()), STREAM_SHA512_DIGEST))
        } else if flags.has(FO_XXH128) {
            Some((Digest::Xxh128(Box::new(twox_hash::XxHash3_128::new())), STREAM_XXH128_DIGEST))
        } else {
            None
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        match self {
            Digest::Md5(d) => d.update(data),
            Digest::Sha1(d) => d.update(data),
            Digest::Sha256(d) => d.update(data),
            Digest::Sha512(d) => d.update(data),
            Digest::Xxh128(d) => d.write(data),
        }
    }

    pub fn finalize(self) -> Vec<u8> {
        match self {
            Digest::Md5(d) => d.finalize().to_vec(),
            Digest::Sha1(d) => d.finalize().to_vec(),
            Digest::Sha256(d) => d.finalize().to_vec(),
            Digest::Sha512(d) => d.finalize().to_vec(),
            // XXH128_canonicalFromHash: big endian high64, low64
            Digest::Xxh128(d) => d.finish_128().to_be_bytes().to_vec(),
        }
    }

    /// crypto_digest_name()
    pub fn name(&self) -> &'static str {
        match self {
            Digest::Md5(_) => "MD5",
            Digest::Sha1(_) => "SHA1",
            Digest::Sha256(_) => "SHA256",
            Digest::Sha512(_) => "SHA512",
            Digest::Xxh128(_) => "XXH128",
        }
    }
}

/// BinToBase64(..., compatible=true)
pub fn to_base64(bin: &[u8]) -> String {
    bareos_bnet::cram_md5::bareos_bin_to_base64(bin, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        let mut f = Flags::default();
        f.set(FO_MD5);
        let (mut d, s) = Digest::for_flags(f).unwrap();
        d.update(b"abc");
        assert_eq!(s, STREAM_MD5_DIGEST);
        assert_eq!(to_base64(&d.finalize()), "kAFQmDzST7DWlj99KOF/cg");
        let mut f = Flags::default();
        f.set(FO_XXH128);
        let (mut d, _) = Digest::for_flags(f).unwrap();
        d.update(b"");
        // XXH3_128("") = 99aa06d3014798d86001c324468d497f
        assert_eq!(d.finalize(), vec![0x99, 0xaa, 0x06, 0xd3, 0x01, 0x47, 0x98, 0xd8, 0x60, 0x01, 0xc3, 0x24, 0x46, 0x8d, 0x49, 0x7f]);
    }
}
