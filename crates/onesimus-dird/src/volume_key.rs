//! Volume encryption keys (dird/volume_key.cc, lib/passphrase.cc,
//! lib/crypto_wrap.cc): a semi random ASCII passphrase, optionally wrapped
//! with the Director's `Key Encryption Key` (RFC 3394 AES-128 key wrap),
//! always stored base64 encoded in `Media.EncryptionKey`.

use base64::Engine as _;
use rand::RngCore as _;

/// DEFAULT_PASSPHRASE_LENGTH (include/baconfig.h).
pub(crate) const DEFAULT_PASSPHRASE_LENGTH: usize = 32;

/// generate_crypto_passphrase(): RAND_bytes mapped onto the valid chars.
pub(crate) fn generate_crypto_passphrase(length: usize) -> Vec<u8> {
    const VALID: &[u8] = b"abcdefghijklmnopqrstuvwxyz\
ABCDEFGHIJKLMNOPQRSTUVWXYZ\
0123456789\
!@#$%^&*()-_=+|[]{};:,.<>?/~";
    let mut bytes = vec![0u8; length];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| VALID[usize::from(*b) % VALID.len()]).collect()
}

/// AesWrap() with the RFC 3394 default IV: `kek` is used as a 16 byte
/// AES-128 key (C++ reads 16 bytes from the configured string; a shorter
/// key is padded with NUL bytes here).
pub(crate) fn aes_wrap(kek: &[u8], plain: &[u8]) -> Result<Vec<u8>, String> {
    let mut key = [0u8; 16];
    let n = kek.len().min(16);
    key[..n].copy_from_slice(&kek[..n]);
    let aes = openssl::aes::AesKey::new_encrypt(&key).map_err(|_| "AES_set_encrypt_key() failed".to_string())?;
    let mut out = vec![0u8; plain.len() + 8];
    openssl::aes::wrap_key(&aes, None, &mut out, plain)
        .map_err(|_| "EVP_EncryptUpdate() failed".to_string())?;
    Ok(out)
}

/// BinToBase64(compatible=true): standard alphabet, no padding.
pub(crate) fn bin_to_base64(bin: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(bin)
}

/// GenerateNewEncryptionKey(): the catalog value of a new volume key, or
/// the error message of the console.
pub(crate) fn generate_new_encryption_key(volume: &str, key_encryption_key: Option<&str>) -> Result<String, String> {
    let passphrase = generate_crypto_passphrase(DEFAULT_PASSPHRASE_LENGTH);
    if passphrase.len() != DEFAULT_PASSPHRASE_LENGTH {
        return Err(format!("Failed to generate new encryption passphrase for Volume {volume}.\n"));
    }
    let stored = match key_encryption_key {
        Some(kek) => aes_wrap(kek.as_bytes(), &passphrase)
            .map_err(|error| format!("Failed to wrap passphrase ERR={error}.\n"))?,
        None => passphrase,
    };
    Ok(bin_to_base64(&stored))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passphrase_uses_valid_chars() {
        let p = generate_crypto_passphrase(32);
        assert_eq!(p.len(), 32);
        assert!(p.iter().all(|c| c.is_ascii_graphic()));
        assert_ne!(p, generate_crypto_passphrase(32));
    }

    #[test]
    fn rfc3394_test_vector() {
        // RFC 3394 4.1: 128 bit key data with a 128 bit KEK.
        let kek = hex::decode("000102030405060708090A0B0C0D0E0F").unwrap();
        let data = hex::decode("00112233445566778899AABBCCDDEEFF").unwrap();
        let wrapped = aes_wrap(&kek, &data).unwrap();
        assert_eq!(hex::encode_upper(wrapped), "1FA68B0A8112B447AEF34BD8FB5A7B829D3E862371D2CFE5");
    }

    #[test]
    fn base64_is_compatible_without_padding() {
        // BinToBase64(compatible=true) emits no '=' padding.
        assert_eq!(bin_to_base64(b"ab"), "YWI");
        assert_eq!(bin_to_base64(b"abc"), "YWJj");
    }

    #[test]
    fn key_lengths() {
        // 32 bytes -> 43 chars, wrapped 40 bytes -> 54 chars.
        assert_eq!(generate_new_encryption_key("v", None).unwrap().len(), 43);
        let wrapped = generate_new_encryption_key("v", Some("0123456789abcdef")).unwrap();
        assert_eq!(wrapped.len(), 54);
        // Unwrapping with the same KEK yields printable passphrase bytes.
        let bin = base64::engine::general_purpose::STANDARD_NO_PAD.decode(wrapped).unwrap();
        let aes = openssl::aes::AesKey::new_decrypt(b"0123456789abcdef").unwrap();
        let mut plain = [0u8; 32];
        openssl::aes::unwrap_key(&aes, None, &mut plain, &bin).unwrap();
        assert!(plain.iter().all(|c| c.is_ascii_graphic()));
    }
}
