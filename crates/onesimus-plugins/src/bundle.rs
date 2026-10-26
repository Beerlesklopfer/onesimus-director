//! Bounded file inputs for the administrator's plugin check/load path.
//! Only certificates and public trust policy are accepted by this module.
use crate::{SignerGrant, TrustPolicy};
use onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact;
use openssl::x509::X509;
use prost::Message;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read},
    path::{Path, PathBuf},
};

pub fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "plugin input exceeds size limit",
        ));
    }
    Ok(bytes)
}
pub fn read_bundle(path: &Path, max_component_bytes: usize) -> io::Result<SignedPluginArtifact> {
    let limit = max_component_bytes
        .checked_add(1024 * 1024)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "invalid component size limit")
        })?;
    SignedPluginArtifact::decode(read_bounded(path, limit)?.as_slice()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid signed plugin protobuf bundle",
        )
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    ca_certificates: Vec<PathBuf>,
    signers: BTreeMap<String, SignerGrant>,
    #[serde(default)]
    revoked_keys: BTreeSet<String>,
    #[serde(default)]
    revoked_certificates: BTreeSet<String>,
    #[serde(default)]
    revoked_plugins: BTreeSet<String>,
    allowed_capabilities: BTreeSet<String>,
    target: String,
    valid_from: u64,
    valid_until: u64,
}
/// Paths in the policy are relative to that administrative file, never to the
/// plugin bundle. No system TLS roots or bundle certificates become trusted.
pub fn read_policy(path: &Path) -> io::Result<TrustPolicy> {
    let invalid = |message| io::Error::new(io::ErrorKind::InvalidData, message);
    let policy: PolicyFile = serde_json::from_slice(&read_bounded(path, 1024 * 1024)?)
        .map_err(|_| invalid("invalid plugin trust policy JSON"))?;
    if policy.ca_certificates.is_empty() || policy.ca_certificates.len() > 32 {
        return Err(invalid(
            "policy requires between 1 and 32 CA certificate files",
        ));
    }
    let mut roots = Vec::new();
    for cert_path in policy.ca_certificates {
        let bytes = read_bounded(
            &path.parent().unwrap_or(Path::new(".")).join(cert_path),
            64 * 1024,
        )?;
        if bytes
            .windows(b"PRIVATE KEY".len())
            .any(|s| s == b"PRIVATE KEY")
        {
            return Err(invalid(
                "private keys are not accepted by the runtime trust policy",
            ));
        }
        let certs = if bytes.starts_with(b"-----BEGIN") {
            X509::stack_from_pem(&bytes).map_err(|_| invalid("invalid CA PEM certificates"))?
        } else {
            vec![X509::from_der(&bytes).map_err(|_| invalid("invalid CA DER certificate"))?]
        };
        if certs.is_empty() {
            return Err(invalid("CA certificate file is empty"));
        }
        for cert in certs {
            roots.push(
                cert.to_der()
                    .map_err(|_| invalid("invalid CA certificate"))?,
            );
            if roots.len() > 32 {
                return Err(invalid("too many CA certificates"));
            }
        }
    }
    Ok(TrustPolicy {
        trust_anchors_der: roots,
        signers: policy.signers,
        revoked_keys: policy.revoked_keys,
        revoked_certificates: policy.revoked_certificates,
        revoked_plugins: policy.revoked_plugins,
        allowed_capabilities: policy.allowed_capabilities,
        target: policy.target,
        valid_from: policy.valid_from,
        valid_until: policy.valid_until,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_bundle_and_relative_public_policy_roundtrip() {
        let root =
            std::env::temp_dir().join(format!("onesimus-plugin-bundle-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let (bundle, policy) = crate::tests::signed_component(b"\0asm\x0d\0\x01\0".to_vec());
        std::fs::write(root.join("ca.der"), &policy.trust_anchors_der[0]).unwrap();
        let policy_json = serde_json::json!({"ca_certificates":["ca.der"], "signers":policy.signers,
            "allowed_capabilities":policy.allowed_capabilities, "target":policy.target,
            "valid_from":policy.valid_from, "valid_until":policy.valid_until});
        std::fs::write(
            root.join("policy.json"),
            serde_json::to_vec(&policy_json).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("plugin.pb"), bundle.encode_to_vec()).unwrap();
        let loaded_policy = read_policy(&root.join("policy.json")).unwrap();
        let loaded = read_bundle(&root.join("plugin.pb"), 16 * 1024 * 1024).unwrap();
        crate::verify(
            &loaded.manifest_bytes,
            &loaded.ed25519_signature,
            loaded.artifact_bytes,
            &loaded.x509_certificate_chain_der,
            &loaded_policy,
            150,
        )
        .unwrap();
        assert!(read_bounded(&root.join("plugin.pb"), 1).is_err());
        std::fs::write(
            root.join("ca.der"),
            b"-----BEGIN PRIVATE KEY-----\nnot-public\n",
        )
        .unwrap();
        assert!(read_policy(&root.join("policy.json")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
