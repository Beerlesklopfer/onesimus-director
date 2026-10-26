//! Verification boundary for signed Rust/WebAssembly plugins. No Python runtime
//! or native dynamic libraries are loaded. Component execution is a separate
//! host implementation and must only accept a VerifiedComponent.
pub mod bundle;
mod certificate;
pub mod discovery;
pub mod executor;
pub mod wasm;
pub use certificate::signing_key_id;

use openssl::{pkey::Id, sign::Verifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const API_VERSION: u32 = 1;
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST: usize = 64 * 1024;
const COMPONENT_HEADER: &[u8] = b"\0asm\x0d\0\x01\0";

/// Detached Ed25519 signature covers the exact UTF-8 JSON bytes, including all
/// fields and whitespace. Do not deserialize and reserialize before verifying.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub plugin_id: String,
    pub version: String,
    pub api_version: u32,
    pub artifact_sha256: String,
    pub signing_key_id: String,
    pub not_before: u64,
    pub not_after: u64,
    pub capabilities: Vec<String>,
    /// Host target, e.g. wasm32-wasip2. The verifier compares it to host policy.
    pub target: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignerGrant {
    pub plugin_ids: BTreeSet<String>,
    pub capabilities: BTreeSet<String>,
}

#[derive(Default)]
pub struct TrustPolicy {
    /// Administratively trusted CA certificates (DER). Bundle roots are never
    /// imported here and the system's TLS/web root store is not consulted.
    pub trust_anchors_der: Vec<Vec<u8>>,
    /// SHA-256(SPKI DER) -> explicitly authorized plugin identities/capabilities.
    pub signers: BTreeMap<String, SignerGrant>,
    /// Applies to every validated chain member, including intermediate/CA keys.
    pub revoked_keys: BTreeSet<String>,
    pub revoked_certificates: BTreeSet<String>,
    pub revoked_plugins: BTreeSet<String>,
    pub allowed_capabilities: BTreeSet<String>,
    pub target: String,
    /// Freshness of the administratively supplied offline revocation/policy
    /// snapshot. No network CRL/OCSP fetch is performed by this verifier.
    pub valid_from: u64,
    pub valid_until: u64,
}

/// Owns the exact verified bytes to avoid a path swap between checking and
/// loading. A runtime must consume these bytes, not reopen the artifact path.
#[derive(Debug)]
pub struct VerifiedComponent {
    manifest: Manifest,
    component: Vec<u8>,
    certificate_sha256: String,
}
impl VerifiedComponent {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn signing_certificate_sha256(&self) -> &str {
        &self.certificate_sha256
    }
    pub fn bytes(&self) -> &[u8] {
        &self.component
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("plugin manifest is oversized or invalid: {0}")]
    Manifest(String),
    #[error("plugin key or identity is untrusted or revoked")]
    Trust,
    #[error("invalid plugin signature")]
    Signature,
    #[error("plugin certification is outside its validity period")]
    Validity,
    #[error("unsupported plugin API, target or component format")]
    Compatibility,
    #[error("plugin requests a capability not granted by the host")]
    Capability,
    #[error("plugin artifact hash does not match its signed manifest")]
    Artifact,
}

pub fn verify(
    manifest_bytes: &[u8],
    signature: &[u8],
    component: Vec<u8>,
    certificate_chain_der: &[Vec<u8>],
    policy: &TrustPolicy,
    now_unix: u64,
) -> Result<VerifiedComponent, VerifyError> {
    if manifest_bytes.len() > MAX_MANIFEST {
        return Err(VerifyError::Manifest("maximum size is 64 KiB".into()));
    }
    let manifest: Manifest =
        serde_json::from_slice(manifest_bytes).map_err(|e| VerifyError::Manifest(e.to_string()))?;
    if manifest.plugin_id.is_empty()
        || manifest.version.is_empty()
        || manifest.signing_key_id.is_empty()
    {
        return Err(VerifyError::Manifest(
            "identity, version and signing key are required".into(),
        ));
    }
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(VerifyError::Compatibility);
    }
    let (key, certificate_sha256) =
        certificate::verify_chain(certificate_chain_der, policy, now_unix)?;
    if manifest.signing_key_id != certificate::key_id(&key)?
        || policy.revoked_plugins.contains(&manifest.plugin_id)
    {
        return Err(VerifyError::Trust);
    }
    let grant = policy
        .signers
        .get(&manifest.signing_key_id)
        .ok_or(VerifyError::Trust)?;
    if !grant.plugin_ids.contains(&manifest.plugin_id) {
        return Err(VerifyError::Trust);
    }
    if key.id() != Id::ED25519 || signature.len() != 64 {
        return Err(VerifyError::Signature);
    }
    let valid = Verifier::new_without_digest(&key)
        .and_then(|mut verifier| verifier.verify_oneshot(signature, manifest_bytes))
        .map_err(|_| VerifyError::Signature)?;
    if !valid {
        return Err(VerifyError::Signature);
    }
    if manifest.not_before >= manifest.not_after
        || now_unix < manifest.not_before
        || now_unix >= manifest.not_after
    {
        return Err(VerifyError::Validity);
    }
    if manifest.api_version != API_VERSION
        || manifest.target != policy.target
        || !component.starts_with(COMPONENT_HEADER)
    {
        return Err(VerifyError::Compatibility);
    }
    if manifest
        .capabilities
        .iter()
        .any(|c| !policy.allowed_capabilities.contains(c) || !grant.capabilities.contains(c))
    {
        return Err(VerifyError::Capability);
    }
    let expected = hex::decode(&manifest.artifact_sha256).map_err(|_| VerifyError::Artifact)?;
    if expected.as_slice() != Sha256::digest(&component).as_slice() {
        return Err(VerifyError::Artifact);
    }
    Ok(VerifiedComponent {
        manifest,
        component,
        certificate_sha256,
    })
}

#[cfg(test)]
mod tests;
