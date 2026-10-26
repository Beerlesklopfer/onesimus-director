//! Real File Daemon CLI, real X.509 bundle and a real Component Model fixture.
//! Private test keys are generated in memory and never written to the bundle.
use onesimus_plugins::Manifest;
use onesimus_proto_rs::onesimus::{
    ccc::{PluginMetadata, ReturnCode},
    filed::plugin::v1::{InitializeResponse, OperationResult, SignedPluginArtifact},
};
use openssl::{pkey::PKey, sign::Signer};
use prost::Message;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    KeyUsagePurpose, PKCS_ED25519,
};
use sha2::{Digest, Sha256};
use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn fd_checks_a_signed_component_and_rejects_a_tampered_signature() {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let root = std::env::temp_dir().join(format!(
        "onesimus-plugin-cli-{}-{}",
        std::process::id(),
        now.as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let ca_key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut ca = CertificateParams::default();
    ca.distinguished_name
        .push(DnType::CommonName, "plugin-test-ca");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = ca.self_signed(&ca_key).unwrap();
    let signing_key = KeyPair::generate_for(&PKCS_ED25519).unwrap();
    let mut leaf = CertificateParams::default();
    leaf.distinguished_name
        .push(DnType::CommonName, "plugin-test-signer");
    leaf.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::CodeSigning];
    leaf.use_authority_key_identifier_extension = true;
    let leaf = leaf.signed_by(&signing_key, &ca, &ca_key).unwrap();
    let key = PKey::private_key_from_der(&signing_key.serialize_der()).unwrap();
    let response = InitializeResponse {
        result: Some(OperationResult {
            code: ReturnCode::RcOk as i32,
            diagnostic: String::new(),
        }),
        metadata: Some(PluginMetadata {
            version: "1.0.0".into(),
            interface_version: 1,
            ..Default::default()
        }),
    }
    .encode_to_vec();
    let escaped: String = response.iter().map(|b| format!("\\{b:02x}")).collect();
    let mut exports = String::new();
    for name in [
        "initialize",
        "handle-event",
        "next-file",
        "read-file",
        "write-file",
        "close-file",
        "shutdown",
    ] {
        exports.push_str(&format!("(export \"{name}\" (func $call))"));
    }
    let artifact = wat::parse_str(format!(
        r#"(component
        (core module $m
            (memory (export "memory") 1)
            (data (i32.const 64) "{escaped}")
            (func (export "realloc") (param i32 i32 i32 i32) (result i32) (i32.const 4096))
            (func (export "call") (param i32 i32) (result i32)
                (i32.store (i32.const 0) (i32.const 64))
                (i32.store (i32.const 4) (i32.const {length})) (i32.const 0)))
        (core instance $i (instantiate $m))
        (alias core export $i "memory" (core memory $mem))
        (alias core export $i "realloc" (core func $realloc))
        (alias core export $i "call" (core func $core-call))
        (func $call (param "request" (list u8)) (result (list u8))
            (canon lift (core func $core-call) (memory $mem) (realloc $realloc)))
        {exports})"#,
        length = response.len()
    ))
    .unwrap();
    let key_id = onesimus_plugins::signing_key_id(leaf.der()).unwrap();
    let hash: String = Sha256::digest(&artifact)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let manifest = Manifest {
        schema_version: onesimus_plugins::MANIFEST_SCHEMA_VERSION,
        plugin_id: "test.cli".into(),
        version: "1.0.0".into(),
        api_version: 1,
        artifact_sha256: hash,
        signing_key_id: key_id.clone(),
        not_before: now.as_secs() - 1,
        not_after: now.as_secs() + 300,
        capabilities: vec![],
        target: "wasm32-wasip2".into(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    let signature = Signer::new_without_digest(&key)
        .unwrap()
        .sign_oneshot_to_vec(&manifest_bytes)
        .unwrap();
    let mut bundle = SignedPluginArtifact {
        manifest_bytes,
        ed25519_signature: signature,
        artifact_bytes: artifact,
        x509_certificate_chain_der: vec![leaf.der().to_vec()],
    };
    std::fs::write(root.join("bundle.pb"), bundle.encode_to_vec()).unwrap();
    std::fs::write(root.join("ca.pem"), ca.pem()).unwrap();
    let policy = serde_json::json!({ "ca_certificates": ["ca.pem"],
        "signers": {key_id: {"plugin_ids":["test.cli"], "capabilities":[]}},
        "allowed_capabilities":[], "target":"wasm32-wasip2", "valid_from":now.as_secs()-1, "valid_until":now.as_secs()+300 });
    std::fs::write(
        root.join("policy.json"),
        serde_json::to_vec(&policy).unwrap(),
    )
    .unwrap();
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_ofd"))
            .arg("--check-plugin")
            .arg(root.join("bundle.pb"))
            .arg("--plugin-policy")
            .arg(root.join("policy.json"))
            .output()
            .unwrap()
    };
    let output = check();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("test.cli 1.0.0"));
    bundle.ed25519_signature[0] ^= 1;
    std::fs::write(root.join("bundle.pb"), bundle.encode_to_vec()).unwrap();
    let output = check();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid plugin signature"));
    std::fs::remove_dir_all(root).unwrap();
}
