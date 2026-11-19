//! Public X.509 validation only. This module accepts no private key material.
//!
//! A chain is accepted only as leaf → generation → root, where generation and
//! root are the compiled-in certificates (byte for byte). Intermediates from
//! the bundle are never used to build the path: at most the bundle may repeat
//! the compiled-in generation. Validity of every member is checked at the
//! verification time (each load).
use crate::{
    VerifyError,
    trust::{GRANTS_OID_DER, Grants, Revocations, TrustBase},
};
use openssl::{
    hash::MessageDigest,
    pkey::{Id, PKey, Public},
    rsa::Padding,
    sign::{RsaPssSaltlen, Verifier},
    stack::Stack,
    x509::{
        X509, X509Ref, X509StoreContext,
        store::X509StoreBuilder,
        verify::{X509VerifyFlags, X509VerifyParam},
    },
};
use sha2::{Digest, Sha256};

/// Bundle chain: the leaf, optionally followed by the generation.
const MAX_BUNDLE_CHAIN: usize = 2;
const MAX_CERT_BYTES: usize = 64 * 1024;
/// The only RSA size accepted for manifest signatures.
pub const RSA_BITS: u32 = 4096;

pub fn signing_key_id(certificate_der: &[u8]) -> Result<String, VerifyError> {
    let certificate = X509::from_der(certificate_der).map_err(|_| VerifyError::Trust)?;
    key_id(&certificate.public_key().map_err(|_| VerifyError::Trust)?)
}
pub(crate) fn key_id(key: &PKey<Public>) -> Result<String, VerifyError> {
    let spki = key.public_key_to_der().map_err(|_| VerifyError::Trust)?;
    Ok(hex::encode(Sha256::digest(spki)))
}
fn fingerprint(certificate: &X509Ref) -> Result<String, VerifyError> {
    Ok(hex::encode(Sha256::digest(
        certificate.to_der().map_err(|_| VerifyError::Trust)?,
    )))
}

/// The grants extension of a signing certificate: exactly one, well formed.
/// Does not check the chain; use it only on a verified leaf or for tooling.
pub fn leaf_grants(der: &[u8]) -> Result<Grants, VerifyError> {
    let (_, cert) = x509_parser::parse_x509_certificate(der).map_err(|_| VerifyError::Trust)?;
    let mut found = cert
        .extensions()
        .iter()
        .filter(|e| e.oid.as_bytes() == GRANTS_OID_DER);
    let (Some(ext), None) = (found.next(), found.next()) else {
        return Err(VerifyError::Trust);
    };
    Grants::from_der(ext.value).map_err(|_| VerifyError::Trust)
}

/// Check explicit code-signing use. OpenSSL validates chain signatures, validity,
/// constraints and critical extensions; this parses KU/EKU without unsafe FFI
/// and also works on OpenSSL versions without a built-in code-signing purpose.
fn signing_use(der: &[u8], leaf: bool) -> Result<(), VerifyError> {
    let (remaining, cert) =
        x509_parser::parse_x509_certificate(der).map_err(|_| VerifyError::Trust)?;
    if !remaining.is_empty() {
        return Err(VerifyError::Trust);
    }
    cert.extensions_map().map_err(|_| VerifyError::Trust)?;
    let eku = cert.extended_key_usage().map_err(|_| VerifyError::Trust)?;
    if leaf {
        let usage = cert
            .key_usage()
            .map_err(|_| VerifyError::Trust)?
            .ok_or(VerifyError::Trust)?;
        let eku = eku.ok_or(VerifyError::Trust)?;
        if cert.is_ca() || !usage.value.digital_signature() || !eku.value.code_signing {
            return Err(VerifyError::Trust);
        }
    } else if eku.is_some_and(|usage| !usage.value.code_signing && !usage.value.any) {
        return Err(VerifyError::Trust);
    }
    Ok(())
}

/// A verified chain: the leaf's key, its certificate fingerprint and grants.
pub(crate) struct VerifiedChain {
    pub key: PKey<Public>,
    pub certificate_sha256: String,
    pub grants: Grants,
}

pub(crate) fn verify_chain(
    chain_der: &[Vec<u8>],
    base: &TrustBase,
    local_revoked: &Revocations,
    now: u64,
) -> Result<VerifiedChain, VerifyError> {
    if chain_der.is_empty()
        || chain_der.len() > MAX_BUNDLE_CHAIN
        || chain_der.iter().any(|d| d.len() > MAX_CERT_BYTES)
        // A bundle may only repeat the compiled-in generation, nothing else.
        || chain_der.iter().skip(1).any(|d| d.as_slice() != base.generation_der())
    {
        return Err(VerifyError::Trust);
    }
    let leaf = X509::from_der(&chain_der[0]).map_err(|_| VerifyError::Trust)?;
    // Reject malformed/trailing DER before passing data to the chain builder.
    signing_use(&chain_der[0], true)?;
    let grants = leaf_grants(&chain_der[0])?;
    let root = X509::from_der(base.root_der()).map_err(|_| VerifyError::Trust)?;
    let generation = X509::from_der(base.generation_der()).map_err(|_| VerifyError::Trust)?;

    let mut store = X509StoreBuilder::new().map_err(|_| VerifyError::Trust)?;
    store.add_cert(root).map_err(|_| VerifyError::Trust)?;
    let mut param = X509VerifyParam::new().map_err(|_| VerifyError::Trust)?;
    param.set_time(now.try_into().map_err(|_| VerifyError::Validity)?);
    // leaf, generation, root
    param.set_depth(2);
    param.set_auth_level(2);
    param
        .set_flags(X509VerifyFlags::X509_STRICT | X509VerifyFlags::CHECK_SS_SIGNATURE)
        .map_err(|_| VerifyError::Trust)?;
    store.set_param(&param).map_err(|_| VerifyError::Trust)?;
    let store = store.build();
    let mut untrusted = Stack::new().map_err(|_| VerifyError::Trust)?;
    untrusted.push(generation).map_err(|_| VerifyError::Trust)?;

    let revoked = |id: &str, cert_id: &str| {
        base.revoked().key(id) || local_revoked.key(id) || base.revoked().certificate(cert_id) || local_revoked.certificate(cert_id)
    };
    let mut context = X509StoreContext::new().map_err(|_| VerifyError::Trust)?;
    let valid = context
        .init(&store, &leaf, &untrusted, |ctx| {
            if !ctx.verify_cert()? {
                return Ok(false);
            }
            let Some(chain) = ctx.chain() else {
                return Ok(false);
            };
            if chain.len() != 3 {
                return Ok(false);
            }
            for (i, cert) in chain.iter().enumerate() {
                let Ok(der) = cert.to_der() else {
                    return Ok(false);
                };
                // Exactly the compiled-in generation and root, nothing else.
                if (i == 1 && der != base.generation_der()) || (i == 2 && der != base.root_der()) {
                    return Ok(false);
                }
                if signing_use(&der, i == 0).is_err() {
                    return Ok(false);
                }
                let (Ok(key), Ok(cert_id)) = (cert.public_key(), fingerprint(cert)) else {
                    return Ok(false);
                };
                let Ok(id) = key_id(&key) else {
                    return Ok(false);
                };
                if revoked(&id, &cert_id) {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .map_err(|_| VerifyError::Trust)?;
    if !valid {
        return Err(VerifyError::Trust);
    }
    Ok(VerifiedChain {
        key: leaf.public_key().map_err(|_| VerifyError::Trust)?,
        certificate_sha256: fingerprint(&leaf)?,
        grants,
    })
}

/// Manifest signature by the key type of the (verified) leaf: Ed25519 (64
/// bytes) or RSA with exactly 4096 bits, RSA-PSS, SHA-256, MGF1-SHA-256,
/// salt length = digest length. PKCS#1 v1.5, other salt lengths and other
/// key sizes or types are refused.
pub(crate) fn verify_signature(key: &PKey<Public>, message: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
    let valid = match key.id() {
        Id::ED25519 if signature.len() == 64 => Verifier::new_without_digest(key)
            .and_then(|mut v| v.verify_oneshot(signature, message)),
        Id::RSA if key.bits() == RSA_BITS && signature.len() == (RSA_BITS / 8) as usize => {
            Verifier::new(MessageDigest::sha256(), key).and_then(|mut v| {
                v.set_rsa_padding(Padding::PKCS1_PSS)?;
                v.set_rsa_pss_saltlen(RsaPssSaltlen::DIGEST_LENGTH)?;
                v.set_rsa_mgf1_md(MessageDigest::sha256())?;
                v.verify_oneshot(signature, message)
            })
        }
        _ => return Err(VerifyError::Signature),
    };
    match valid {
        Ok(true) => Ok(()),
        _ => Err(VerifyError::Signature),
    }
}

/// Signature check for tooling (`sign-component assemble`): the manifest
/// signature against the leaf certificate's key, without chain validation.
pub fn verify_manifest_signature(leaf_der: &[u8], manifest: &[u8], signature: &[u8]) -> Result<(), VerifyError> {
    let leaf = X509::from_der(leaf_der).map_err(|_| VerifyError::Trust)?;
    verify_signature(&leaf.public_key().map_err(|_| VerifyError::Trust)?, manifest, signature)
}
