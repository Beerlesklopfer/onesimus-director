//! Public X.509 validation only. This module accepts no private key material.
use crate::{TrustPolicy, VerifyError};
use openssl::{
    pkey::{PKey, Public},
    stack::Stack,
    x509::{
        X509, X509Ref, X509StoreContext,
        store::X509StoreBuilder,
        verify::{X509VerifyFlags, X509VerifyParam},
    },
};
use sha2::{Digest, Sha256};

const MAX_CHAIN: usize = 8;
const MAX_CERT_BYTES: usize = 64 * 1024;

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

pub(crate) fn verify_chain(
    chain_der: &[Vec<u8>],
    policy: &TrustPolicy,
    now: u64,
) -> Result<(PKey<Public>, String), VerifyError> {
    if chain_der.is_empty()
        || chain_der.len() > MAX_CHAIN
        || chain_der.iter().any(|d| d.len() > MAX_CERT_BYTES)
        || policy.trust_anchors_der.is_empty()
        || policy.valid_from >= policy.valid_until
        || now < policy.valid_from
        || now >= policy.valid_until
    {
        return Err(VerifyError::Trust);
    }
    let certs: Vec<X509> = chain_der
        .iter()
        .map(|der| X509::from_der(der).map_err(|_| VerifyError::Trust))
        .collect::<Result<_, _>>()?;
    // Reject malformed/trailing DER before passing data to the chain builder.
    for (i, der) in chain_der.iter().enumerate() {
        signing_use(der, i == 0)?;
    }
    let mut store = X509StoreBuilder::new().map_err(|_| VerifyError::Trust)?;
    for root in &policy.trust_anchors_der {
        store
            .add_cert(X509::from_der(root).map_err(|_| VerifyError::Trust)?)
            .map_err(|_| VerifyError::Trust)?;
    }
    let mut param = X509VerifyParam::new().map_err(|_| VerifyError::Trust)?;
    param.set_time(now.try_into().map_err(|_| VerifyError::Validity)?);
    param.set_depth(MAX_CHAIN as i32);
    param.set_auth_level(2);
    param
        .set_flags(X509VerifyFlags::X509_STRICT | X509VerifyFlags::CHECK_SS_SIGNATURE)
        .map_err(|_| VerifyError::Trust)?;
    store.set_param(&param).map_err(|_| VerifyError::Trust)?;
    let store = store.build();
    let mut untrusted = Stack::new().map_err(|_| VerifyError::Trust)?;
    for cert in certs.iter().skip(1) {
        untrusted
            .push(cert.clone())
            .map_err(|_| VerifyError::Trust)?;
    }
    let mut context = X509StoreContext::new().map_err(|_| VerifyError::Trust)?;
    let valid = context
        .init(&store, &certs[0], &untrusted, |ctx| {
            if !ctx.verify_cert()? {
                return Ok(false);
            }
            let Some(chain) = ctx.chain() else {
                return Ok(false);
            };
            for (i, cert) in chain.iter().enumerate() {
                let Ok(der) = cert.to_der() else {
                    return Ok(false);
                };
                if signing_use(&der, i == 0).is_err() {
                    return Ok(false);
                }
                let Ok(key) = cert.public_key() else {
                    return Ok(false);
                };
                let Ok(id) = key_id(&key) else {
                    return Ok(false);
                };
                let Ok(cert_id) = fingerprint(cert) else {
                    return Ok(false);
                };
                if policy.revoked_keys.contains(&id)
                    || policy.revoked_certificates.contains(&cert_id)
                {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .map_err(|_| VerifyError::Trust)?;
    if !valid {
        return Err(VerifyError::Trust);
    }
    Ok((
        certs[0].public_key().map_err(|_| VerifyError::Trust)?,
        fingerprint(&certs[0])?,
    ))
}
