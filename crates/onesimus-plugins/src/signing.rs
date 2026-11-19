//! Build-system side of plugin signing, without any private key: `prepare`
//! writes the manifest bytes, an external signer (OpenSSL with the TPM
//! provider) signs them, `assemble` checks the signature and builds the
//! bundle. The example `sign-component` is the command-line front end.
//!
//! External signature (RSA 4096 in the TPM):
//! `openssl dgst -sha256 -sign handle:0x81000010 -provider tpm2 -provider default
//!  -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest -out manifest.sig manifest.json`
use crate::{
    API_VERSION, LocalPolicy, Manifest, NativePart, TARGET, TrustBase, TrustPolicy,
    certificate::{RSA_BITS, leaf_grants, signing_key_id, verify_manifest_signature},
    parse_manifest, trust::valid_suite,
};
use onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact;
use openssl::{pkey::Id, x509::X509};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const COMPONENT_HEADER: &[u8] = b"\0asm\x0d\0\x01\0";

/// What goes into a manifest besides what `prepare` derives itself.
#[derive(Debug, Clone)]
pub struct PrepareRequest {
    pub plugin_id: String,
    pub version: String,
    pub capabilities: Vec<String>,
    pub native: BTreeMap<String, Vec<NativePart>>,
    pub suite: String,
    /// Default: the signing certificate's notBefore / notAfter.
    pub not_before: Option<u64>,
    pub not_after: Option<u64>,
}

/// Grants of the leaf must cover the plugin ID and every capability; the key
/// must be RSA 4096 or Ed25519.
fn check_leaf(leaf_der: &[u8], plugin_id: &str, capabilities: &[String]) -> Result<(), String> {
    let leaf = X509::from_der(leaf_der).map_err(|_| "signing certificate is not DER")?;
    let key = leaf.public_key().map_err(|_| "signing certificate key unreadable")?;
    match key.id() {
        Id::RSA if key.bits() == RSA_BITS => {}
        Id::ED25519 => {}
        Id::RSA => return Err(format!("RSA key has {} bits; exactly {RSA_BITS} are required", key.bits())),
        _ => return Err("signing key must be RSA 4096 or Ed25519".into()),
    }
    let grants = leaf_grants(leaf_der).map_err(|_| {
        format!("signing certificate has no valid grants extension ({})", crate::GRANTS_OID)
    })?;
    if !grants.plugin_ids.contains(plugin_id) {
        return Err(format!("the signing certificate does not grant plugin {plugin_id}"));
    }
    if let Some(c) = capabilities.iter().find(|c| !grants.capabilities.contains(*c)) {
        return Err(format!("the signing certificate does not grant capability {c}"));
    }
    Ok(())
}

/// Manifest bytes (schema 3, pretty JSON) for `component`, signed later by
/// the key of `leaf_der`. These exact bytes must be signed.
pub fn prepare(component: &[u8], leaf_der: &[u8], request: &PrepareRequest) -> Result<Vec<u8>, String> {
    if !component.starts_with(COMPONENT_HEADER) {
        return Err("component is not a WebAssembly component".into());
    }
    if !valid_suite(&request.suite) {
        return Err(format!("invalid suite \"{}\"", request.suite.escape_default()));
    }
    check_leaf(leaf_der, &request.plugin_id, &request.capabilities)?;
    let (_, cert) = x509_parser::parse_x509_certificate(leaf_der).map_err(|_| "signing certificate unparsable")?;
    let seconds = |t: i64| u64::try_from(t).map_err(|_| "certificate time before 1970".to_string());
    let manifest = Manifest {
        schema_version: 3,
        plugin_id: request.plugin_id.clone(),
        version: request.version.clone(),
        api_version: API_VERSION,
        artifact_sha256: hex::encode(Sha256::digest(component)),
        signing_key_id: signing_key_id(leaf_der).map_err(|e| e.to_string())?,
        not_before: match request.not_before {
            Some(t) => t,
            None => seconds(cert.validity().not_before.timestamp())?,
        },
        not_after: match request.not_after {
            Some(t) => t,
            None => seconds(cert.validity().not_after.timestamp())?,
        },
        capabilities: request.capabilities.clone(),
        target: TARGET.into(),
        native: request.native.clone(),
        suite: Some(request.suite.clone()),
    };
    if manifest.not_before >= manifest.not_after {
        return Err("manifest validity is empty".into());
    }
    let mut bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    parse_manifest(&bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// The bundle from component, manifest, external signature and chain (leaf
/// first, optionally the generation). Refuses unless the signature verifies
/// with the leaf key and manifest, component and grants agree. With `trust`
/// it also runs the full check `ofd` does (chain to this trust base,
/// validity at `now`, suite of the manifest).
pub fn assemble(component: Vec<u8>, manifest_bytes: Vec<u8>, signature: Vec<u8>, chain_der: Vec<Vec<u8>>,
    trust: Option<&TrustBase>, now: u64) -> Result<SignedPluginArtifact, String> {
    let manifest = parse_manifest(&manifest_bytes).map_err(|e| e.to_string())?;
    if manifest.schema_version != 3 {
        return Err("only schema 3 manifests can be assembled".into());
    }
    if chain_der.is_empty() || chain_der.len() > 2 {
        return Err("chain: the signing certificate, optionally followed by the generation".into());
    }
    if !component.starts_with(COMPONENT_HEADER) || manifest.artifact_sha256 != hex::encode(Sha256::digest(&component)) {
        return Err("component does not match the manifest".into());
    }
    if manifest.signing_key_id != signing_key_id(&chain_der[0]).map_err(|e| e.to_string())? {
        return Err("manifest names another signing key than the certificate".into());
    }
    check_leaf(&chain_der[0], &manifest.plugin_id, &manifest.capabilities)?;
    verify_manifest_signature(&chain_der[0], &manifest_bytes, &signature)
        .map_err(|_| "signature does not verify (RSA-PSS/SHA-256, salt = digest length, 4096 bits; or Ed25519)")?;
    let bundle = SignedPluginArtifact {
        manifest_bytes,
        ed25519_signature: signature,
        artifact_bytes: component,
        x509_certificate_chain_der: chain_der,
    };
    if let Some(base) = trust {
        let policy = TrustPolicy {
            base: base.clone(),
            local: LocalPolicy::default(),
            suite: manifest.suite.clone().unwrap_or_default(),
        };
        crate::verify(&bundle.manifest_bytes, &bundle.ed25519_signature, bundle.artifact_bytes.clone(),
            &bundle.x509_certificate_chain_der, &policy, now)
            .map_err(|e| format!("trust check: {e}"))?;
    }
    Ok(bundle)
}
