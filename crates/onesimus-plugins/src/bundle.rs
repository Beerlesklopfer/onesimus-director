// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Bounded file inputs for the administrator's plugin check/load path.
//! Only certificates and public policy are accepted by this module.
use crate::{LocalPolicy, Revocations, TrustBase};
use onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact;
use openssl::x509::X509;
use prost::Message;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
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

/// A trust base directory as `ONESIMUS_PLUGIN_CA` names it: `root.pem`,
/// `generation.pem` and `revoked.json`. Used by tooling (`sign-component
/// assemble --trust-dir`); `ofd` gets the same files compiled in.
pub fn read_trust_dir(dir: &Path) -> io::Result<TrustBase> {
    let read = |name: &str| read_bounded(&dir.join(name), 64 * 1024);
    TrustBase::from_pem(&read("root.pem")?, &read("generation.pem")?, &read("revoked.json")?)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The local `Plugin Policy`. Every field is optional and can only restrict.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFile {
    /// Allowed for pinning only: each file must hold compiled-in certificates.
    #[serde(default)]
    ca_certificates: Vec<PathBuf>,
    /// Grants of former versions; refused (grants come from the certificate).
    #[serde(default)]
    signers: Option<serde_json::Value>,
    #[serde(default)]
    plugin_ids: Option<BTreeSet<String>>,
    #[serde(default)]
    allowed_capabilities: Option<BTreeSet<String>>,
    #[serde(default)]
    revoked_keys: BTreeSet<String>,
    #[serde(default)]
    revoked_certificates: BTreeSet<String>,
    #[serde(default)]
    revoked_plugins: BTreeSet<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    valid_from: Option<u64>,
    #[serde(default)]
    valid_until: Option<u64>,
}

/// Reads the local policy. Anything that would widen the compiled-in trust
/// is a load error: a CA that is not compiled in, signer grants, another
/// target. Paths are relative to the policy file, never to a bundle.
pub fn read_local_policy(path: &Path, base: &TrustBase) -> io::Result<LocalPolicy> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    let policy: PolicyFile = serde_json::from_slice(&read_bounded(path, 1024 * 1024)?)
        .map_err(|e| invalid(format!("invalid plugin policy JSON: {e}")))?;
    if policy.signers.is_some() {
        return Err(invalid(
            "the local plugin policy cannot grant (\"signers\"): grants come from the signing certificate; \
             the policy may only restrict"
                .into(),
        ));
    }
    if policy.target.as_deref().is_some_and(|t| t != crate::TARGET) {
        return Err(invalid(format!("plugin policy target must be {}", crate::TARGET)));
    }
    if policy.ca_certificates.len() > 32 {
        return Err(invalid("too many CA certificate files".into()));
    }
    for cert_path in &policy.ca_certificates {
        let bytes = read_bounded(
            &path.parent().unwrap_or(Path::new(".")).join(cert_path),
            64 * 1024,
        )?;
        if bytes
            .windows(b"PRIVATE KEY".len())
            .any(|s| s == b"PRIVATE KEY")
        {
            return Err(invalid(
                "private keys are not accepted by the runtime trust policy".into(),
            ));
        }
        let certs = if bytes.starts_with(b"-----BEGIN") {
            X509::stack_from_pem(&bytes).map_err(|_| invalid("invalid CA PEM certificates".into()))?
        } else {
            vec![X509::from_der(&bytes).map_err(|_| invalid("invalid CA DER certificate".into()))?]
        };
        if certs.is_empty() {
            return Err(invalid("CA certificate file is empty".into()));
        }
        for cert in certs {
            let der = cert.to_der().map_err(|_| invalid("invalid CA certificate".into()))?;
            if !base.contains(&der) {
                return Err(invalid(format!(
                    "{}: CA certificate is not compiled into this ofd; the local policy cannot add trust",
                    cert_path.display()
                )));
            }
        }
    }
    let validity = match (policy.valid_from, policy.valid_until) {
        (None, None) => None,
        (Some(from), Some(until)) if from < until => Some((from, until)),
        _ => return Err(invalid("valid_from and valid_until go together, from before until".into())),
    };
    let revoked = Revocations {
        revoked_keys: policy.revoked_keys,
        revoked_certificates: policy.revoked_certificates,
        revoked_plugins: policy.revoked_plugins,
    };
    revoked.check().map_err(invalid)?;
    Ok(LocalPolicy {
        plugin_ids: policy.plugin_ids,
        allowed_capabilities: policy.allowed_capabilities,
        revoked,
        validity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TrustPolicy, testca};

    fn dir(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("onesimus-plugin-bundle-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn bounded_bundle_and_restricting_local_policy_roundtrip() {
        let root = dir("roundtrip");
        let ca = testca::TestCa::shared();
        let base = ca.trust_base();
        let (bundle, _) = crate::tests::signed_component(b"\0asm\x0d\0\x01\0".to_vec());
        std::fs::write(root.join("ca.pem"), ca.root.to_pem().unwrap()).unwrap();
        let policy_json = serde_json::json!({"ca_certificates":["ca.pem"], "plugin_ids":["test.files"],
            "allowed_capabilities":["file.read"], "target":"wasm32-wasip2", "valid_from":1, "valid_until":500});
        std::fs::write(root.join("policy.json"), serde_json::to_vec(&policy_json).unwrap()).unwrap();
        std::fs::write(root.join("plugin.pb"), bundle.encode_to_vec()).unwrap();
        let local = read_local_policy(&root.join("policy.json"), &base).unwrap();
        let loaded = read_bundle(&root.join("plugin.pb"), 16 * 1024 * 1024).unwrap();
        let policy = TrustPolicy { base: base.clone(), local, suite: testca::suite() };
        crate::verify(
            &loaded.manifest_bytes,
            &loaded.ed25519_signature,
            loaded.artifact_bytes,
            &loaded.x509_certificate_chain_der,
            &policy,
            150,
        )
        .unwrap();
        assert!(read_bounded(&root.join("plugin.pb"), 1).is_err());
        // An empty policy restricts nothing.
        std::fs::write(root.join("empty.json"), b"{}").unwrap();
        let empty = read_local_policy(&root.join("empty.json"), &base).unwrap();
        assert!(empty.plugin_ids.is_none() && empty.allowed_capabilities.is_none() && empty.validity.is_none());
        std::fs::write(root.join("ca.pem"), b"-----BEGIN PRIVATE KEY-----\nnot-public\n").unwrap();
        assert!(read_local_policy(&root.join("policy.json"), &base).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_local_policy_that_widens_trust_is_refused() {
        let root = dir("widen");
        let base = testca::TestCa::shared().trust_base();
        let foreign = testca::TestCa::new("foreign", 1, testca::FOREVER);
        std::fs::write(root.join("foreign.pem"), foreign.root.to_pem().unwrap()).unwrap();
        let write = |json: serde_json::Value| {
            std::fs::write(root.join("p.json"), serde_json::to_vec(&json).unwrap()).unwrap();
            read_local_policy(&root.join("p.json"), &base)
        };
        let error = write(serde_json::json!({"ca_certificates":["foreign.pem"]})).unwrap_err();
        assert!(error.to_string().contains("not compiled into this ofd"), "{error}");
        let error = write(serde_json::json!({"signers":{"ab":{"plugin_ids":["x"],"capabilities":[]}}})).unwrap_err();
        assert!(error.to_string().contains("cannot grant"), "{error}");
        assert!(write(serde_json::json!({"target":"native-linux"})).is_err());
        assert!(write(serde_json::json!({"valid_from":5})).is_err());
        assert!(write(serde_json::json!({"revoked_keys":["not-hex"]})).is_err());
        assert!(write(serde_json::json!({"unknown":1})).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trust_dir_reads_the_build_inputs() {
        let root = dir("trustdir");
        let ca = testca::TestCa::shared();
        std::fs::write(root.join("root.pem"), ca.root.to_pem().unwrap()).unwrap();
        std::fs::write(root.join("generation.pem"), ca.generation.to_pem().unwrap()).unwrap();
        std::fs::write(root.join("revoked.json"), b"{}").unwrap();
        let base = read_trust_dir(&root).unwrap();
        assert_eq!(base.root_der(), ca.root.to_der().unwrap());
        assert_eq!(base.generation_name(), "Onesimus Plugin Signing G1");
        // Swapped files: the generation is not self-signed and not Ed25519-rooted.
        std::fs::write(root.join("root.pem"), ca.generation.to_pem().unwrap()).unwrap();
        assert!(read_trust_dir(&root).is_err());
        std::fs::write(root.join("root.pem"), ca.root.to_pem().unwrap()).unwrap();
        std::fs::write(root.join("revoked.json"), br#"{"revoked_keys":["xyz"]}"#).unwrap();
        assert!(read_trust_dir(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
