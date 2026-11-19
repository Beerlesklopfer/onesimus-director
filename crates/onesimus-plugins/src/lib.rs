//! Verification boundary for signed Rust/WebAssembly plugins. No Python runtime
//! or native dynamic libraries are loaded. Component execution is a separate
//! host implementation and must only accept a VerifiedComponent.
//!
//! Trust model (plan: docs/plans/SIGNIER-INFRASTRUKTUR.md): an offline
//! Ed25519 root, one intermediate per generation, signing certificates with
//! the plugin grants in an X.509 extension. Root, the generation of this
//! release and a revocation list are compiled into `ofd` ([`TrustBase`]);
//! the local policy ([`LocalPolicy`]) can only restrict.
pub mod bundle;
mod certificate;
pub mod discovery;
pub mod executor;
pub mod signing;
#[cfg(any(test, feature = "test-ca"))]
pub mod testca;
pub mod trust;
pub mod wasm;
pub use certificate::{RSA_BITS, leaf_grants, signing_key_id, verify_manifest_signature};
pub use trust::{GRANTS_OID, Grants, LocalPolicy, Revocations, TARGET, TrustBase, TrustPolicy, suite_of};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const API_VERSION: u32 = 1;
/// Contract versions `ofd` accepts (docs/plans/PODMAN-PLUGIN.md,
/// "Vertragsversion", user decision 19.11.2026): 1 for the plugins of
/// phase 1 and 2 (PostgreSQL, OpenLDAP), 2 adds `FileEntry.source_ref` and
/// the FIFO/HARDLINK kinds (Podman). No hard cut: both load side by side.
pub const SUPPORTED_API_VERSIONS: &[u32] = &[1, 2];
/// The contract version that introduced host scan references.
pub const API_VERSION_SOURCE_REF: u32 = 2;
/// The only plugin id the Podman grants and imports are bound to
/// (V-11, user decision 19.11.2026): a suite certificate grants its
/// capabilities to each of its plugin ids, so without this fixed binding any
/// other signed plugin could request `podman.rootful`.
pub const PODMAN_PLUGIN_ID: &str = "org.onesimus.podman";
/// Prefix of every Podman grant (`podman.inventory`, `podman.rootful`, ...).
pub const PODMAN_GRANT_PREFIX: &str = "podman.";

/// Whether `capability` is a Podman grant.
pub fn is_podman_grant(capability: &str) -> bool {
    capability.starts_with(PODMAN_GRANT_PREFIX)
}
/// Manifest schema: 3 adds `suite` (binding to the suite version of `ofd`).
/// Schemas 1 and 2 are still parsed and checked, but no longer load: they
/// carry no suite, and the plan's hard cut lets only bundles of the running
/// suite load.
pub const MANIFEST_SCHEMA_VERSION: u32 = 3;
const MAX_MANIFEST: usize = 64 * 1024;
const COMPONENT_HEADER: &[u8] = b"\0asm\x0d\0\x01\0";

/// The detached signature (Ed25519 or RSA-PSS, by the leaf's key type) covers
/// the exact UTF-8 JSON bytes, including all fields and whitespace. Do not
/// deserialize and reserialize before verifying.
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
    /// Host target, always [`TARGET`].
    pub target: String,
    /// Schema 2 and 3: native parts per Debian multiarch triplet (e.g. a
    /// broker), each bound to its SHA-256. Absent in schema 1.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub native: BTreeMap<String, Vec<NativePart>>,
    /// Schema 3: suite version this bundle was built for (Onesimus part of
    /// the workspace version, e.g. `0.2.0`). Must equal the host's exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
}

/// One native file of a plugin package: a path below the plugin directory
/// and the SHA-256 the host checks on the open file before running it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePart {
    pub path: String,
    pub sha256: String,
}

impl Manifest {
    /// The native part `path` for `triplet`, if the signed manifest lists it.
    pub fn native_part(&self, triplet: &str, path: &str) -> Option<&NativePart> {
        self.native.get(triplet)?.iter().find(|p| p.path == path)
    }
}

const MAX_TRIPLETS: usize = 16;
const MAX_PARTS: usize = 8;

/// Native parts (schema 2 and 3): lowercase triplets, safe relative paths,
/// hex hashes.
fn check_native(native: &BTreeMap<String, Vec<NativePart>>) -> Result<(), VerifyError> {
    let bad = |why: &str| Err(VerifyError::Manifest(format!("native: {why}")));
    if native.len() > MAX_TRIPLETS {
        return bad("too many triplets");
    }
    for (triplet, parts) in native {
        let triplet_ok = !triplet.is_empty()
            && triplet.len() <= 64
            && triplet.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'));
        if !triplet_ok {
            return bad("invalid triplet");
        }
        if parts.is_empty() || parts.len() > MAX_PARTS {
            return bad("each triplet needs 1 to 8 parts");
        }
        for part in parts {
            let path_ok = !part.path.is_empty()
                && part.path.len() <= 255
                && !part.path.starts_with('/')
                && part.path.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
                && part.path.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'));
            if !path_ok {
                return bad("invalid path");
            }
            let hash_ok = part.sha256.len() == 64 && part.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !hash_ok {
                return bad("sha256 must be 64 lowercase hex digits");
            }
        }
    }
    Ok(())
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
    #[error("plugin was built for another Onesimus suite version (or has no suite); install the plugin of this release")]
    Suite,
    #[error("plugin artifact hash does not match its signed manifest")]
    Artifact,
}

/// Checks one bundle against the compiled-in trust base, the local
/// restrictions and the host suite. Call it for every load: certificate
/// validity, revocations and the local policy apply at `now_unix`.
pub fn verify(
    manifest_bytes: &[u8],
    signature: &[u8],
    component: Vec<u8>,
    certificate_chain_der: &[Vec<u8>],
    policy: &TrustPolicy,
    now_unix: u64,
) -> Result<VerifiedComponent, VerifyError> {
    let manifest = parse_manifest(manifest_bytes)?;
    if let Some((from, until)) = policy.local.validity
        && (from >= until || now_unix < from || now_unix >= until)
    {
        return Err(VerifyError::Trust);
    }
    let chain = certificate::verify_chain(
        certificate_chain_der,
        &policy.base,
        &policy.local.revoked,
        now_unix,
    )?;
    let id = &manifest.plugin_id;
    if manifest.signing_key_id != certificate::key_id(&chain.key)?
        || policy.base.revoked().plugin(id)
        || policy.local.revoked.plugin(id)
        || !chain.grants.plugin_ids.contains(id)
        || policy.local.plugin_ids.as_ref().is_some_and(|ids| !ids.contains(id))
    {
        return Err(VerifyError::Trust);
    }
    certificate::verify_signature(&chain.key, manifest_bytes, signature)?;
    if manifest.not_before >= manifest.not_after
        || now_unix < manifest.not_before
        || now_unix >= manifest.not_after
    {
        return Err(VerifyError::Validity);
    }
    if manifest.suite.as_deref() != Some(policy.suite.as_str()) {
        return Err(VerifyError::Suite);
    }
    if !SUPPORTED_API_VERSIONS.contains(&manifest.api_version)
        || manifest.target != trust::TARGET
        || !component.starts_with(COMPONENT_HEADER)
    {
        return Err(VerifyError::Compatibility);
    }
    // Podman grants: only org.onesimus.podman, only with contract 2 (its
    // file entries carry host scan references).
    if manifest.capabilities.iter().any(|c| is_podman_grant(c)) {
        if manifest.plugin_id != PODMAN_PLUGIN_ID {
            return Err(VerifyError::Capability);
        }
        if manifest.api_version < API_VERSION_SOURCE_REF {
            return Err(VerifyError::Compatibility);
        }
    }
    if manifest.capabilities.iter().any(|c| {
        !chain.grants.capabilities.contains(c)
            || policy.local.allowed_capabilities.as_ref().is_some_and(|allowed| !allowed.contains(c))
    }) {
        return Err(VerifyError::Capability);
    }
    let expected = hex::decode(&manifest.artifact_sha256).map_err(|_| VerifyError::Artifact)?;
    if expected.as_slice() != Sha256::digest(&component).as_slice() {
        return Err(VerifyError::Artifact);
    }
    Ok(VerifiedComponent {
        manifest,
        component,
        certificate_sha256: chain.certificate_sha256,
    })
}

/// Size, JSON, required fields and the rules of each schema. Schemas 1 and
/// 2 pass this check (their structure is still validated); `verify` refuses
/// them at the suite comparison because they carry no suite.
pub fn parse_manifest(manifest_bytes: &[u8]) -> Result<Manifest, VerifyError> {
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
    let suite = |needed: bool| match (&manifest.suite, needed) {
        (None, false) => Ok(()),
        (Some(_), false) => Err(VerifyError::Manifest("suite needs schema 3".into())),
        (Some(s), true) if trust::valid_suite(s) => Ok(()),
        (_, true) => Err(VerifyError::Manifest("schema 3 needs a valid suite".into())),
    };
    match manifest.schema_version {
        1 if manifest.native.is_empty() => suite(false)?,
        1 => return Err(VerifyError::Manifest("native parts need schema 2".into())),
        2 if manifest.native.is_empty() => {
            return Err(VerifyError::Manifest("native: schema 2 needs at least one triplet".into()));
        }
        2 => {
            suite(false)?;
            check_native(&manifest.native)?
        }
        3 => {
            suite(true)?;
            check_native(&manifest.native)?
        }
        _ => return Err(VerifyError::Compatibility),
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests;
