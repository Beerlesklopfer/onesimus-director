//! Build-system tool only. Not part of the File Daemon runtime/key path.
//! Arguments: component.wasm manifest.json signer-private.pem chain.pem output.pb
use onesimus_plugins::{Manifest, bundle::read_bounded, signing_key_id};
use onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact;
use openssl::{
    pkey::{Id, PKey},
    sign::Signer,
    x509::X509,
};
use prost::Message;
use sha2::{Digest, Sha256};
use std::path::Path;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: sign-component component.wasm manifest.json signer-private.pem chain.pem output.pb".into());
    }
    let artifact = read_bounded(Path::new(&args[0]), 16 * 1024 * 1024)?;
    let manifest_bytes = read_bounded(Path::new(&args[1]), 64 * 1024)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    let key = PKey::private_key_from_pem(&read_bounded(Path::new(&args[2]), 64 * 1024)?)?;
    let certs = X509::stack_from_pem(&read_bounded(Path::new(&args[3]), 512 * 1024)?)?;
    if key.id() != Id::ED25519 || certs.is_empty() || certs.len() > 8 {
        return Err("requires Ed25519 signing key and leaf-first X.509 chain".into());
    }
    let chain: Vec<_> = certs.iter().map(|c| c.to_der()).collect::<Result<_, _>>()?;
    if manifest.schema_version != onesimus_plugins::MANIFEST_SCHEMA_VERSION
        || manifest.api_version != onesimus_plugins::API_VERSION
        || manifest.signing_key_id != signing_key_id(&chain[0])?
        || key.public_key_to_der()? != certs[0].public_key()?.public_key_to_der()?
        || manifest.artifact_sha256 != hex::encode(Sha256::digest(&artifact))
        || !artifact.starts_with(b"\0asm\x0d\0\x01\0")
    {
        return Err("manifest, component, certificate and private key do not match".into());
    }
    let signature = Signer::new_without_digest(&key)?.sign_oneshot_to_vec(&manifest_bytes)?;
    let bundle = SignedPluginArtifact {
        manifest_bytes,
        ed25519_signature: signature,
        artifact_bytes: artifact,
        x509_certificate_chain_der: chain,
    };
    std::fs::write(Path::new(&args[4]), bundle.encode_to_vec())?;
    Ok(())
}
