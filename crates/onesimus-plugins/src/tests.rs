use super::*;
use openssl::{
    asn1::{Asn1Integer, Asn1Time},
    bn::BigNum,
    hash::MessageDigest,
    pkey::{PKey, Private},
    sign::Signer,
    x509::{
        X509, X509NameBuilder,
        extension::{
            AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage,
            SubjectKeyIdentifier,
        },
    },
};

type Key = PKey<Private>;
fn certificate(
    key: &Key,
    issuer: Option<(&X509, &Key)>,
    serial: u32,
    ca: bool,
    code_signing: Option<bool>,
    digital_signature: bool,
    from: i64,
    until: i64,
) -> X509 {
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", &format!("onesimus-test-{serial}"))
        .unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    let serial = Asn1Integer::from_bn(&BigNum::from_u32(serial).unwrap()).unwrap();
    cert.set_serial_number(&serial).unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(issuer.map(|(cert, _)| cert.subject_name()).unwrap_or(&name))
        .unwrap();
    cert.set_pubkey(key).unwrap();
    cert.set_not_before(&Asn1Time::from_unix(from).unwrap())
        .unwrap();
    cert.set_not_after(&Asn1Time::from_unix(until).unwrap())
        .unwrap();
    let mut basic = BasicConstraints::new();
    basic.critical();
    if ca {
        basic.ca();
    }
    cert.append_extension(basic.build().unwrap()).unwrap();
    let mut usage = KeyUsage::new();
    usage.critical();
    if ca {
        usage.key_cert_sign().crl_sign();
    }
    if digital_signature {
        usage.digital_signature();
    } else if !ca {
        usage.key_encipherment();
    }
    cert.append_extension(usage.build().unwrap()).unwrap();
    if let Some(code) = code_signing {
        let mut eku = ExtendedKeyUsage::new();
        if code {
            eku.code_signing();
        } else {
            eku.server_auth();
        }
        cert.append_extension(eku.build().unwrap()).unwrap();
    }
    let ski = SubjectKeyIdentifier::new()
        .build(&cert.x509v3_context(issuer.map(|(cert, _)| cert.as_ref()), None))
        .unwrap();
    cert.append_extension(ski).unwrap();
    let aki = AuthorityKeyIdentifier::new()
        .keyid(true)
        .build(&cert.x509v3_context(issuer.map(|(cert, _)| cert.as_ref()), None))
        .unwrap();
    cert.append_extension(aki).unwrap();
    cert.sign(
        issuer.map(|(_, key)| key).unwrap_or(key),
        MessageDigest::null(),
    )
    .unwrap();
    cert.build()
}
struct Fixture {
    root: X509,
    root_key: Key,
    leaf: X509,
    key: Key,
    manifest: Manifest,
    component: Vec<u8>,
    policy: TrustPolicy,
}
impl Fixture {
    fn new() -> Self {
        let root_key = PKey::generate_ed25519().unwrap();
        let root = certificate(&root_key, None, 1, true, None, false, 1, 1000);
        let key = PKey::generate_ed25519().unwrap();
        let leaf = certificate(
            &key,
            Some((&root, &root_key)),
            2,
            false,
            Some(true),
            true,
            50,
            400,
        );
        let id = signing_key_id(&leaf.to_der().unwrap()).unwrap();
        let component = COMPONENT_HEADER.to_vec();
        let manifest = Manifest {
            schema_version: 1,
            plugin_id: "test.files".into(),
            version: "1.0.0".into(),
            api_version: 1,
            artifact_sha256: hex::encode(Sha256::digest(&component)),
            signing_key_id: id.clone(),
            not_before: 100,
            not_after: 300,
            capabilities: vec!["file.read".into()],
            target: "wasm32-wasip2".into(),
        };
        let policy = TrustPolicy {
            trust_anchors_der: vec![root.to_der().unwrap()],
            signers: [(
                id,
                SignerGrant {
                    plugin_ids: ["test.files".into()].into(),
                    capabilities: ["file.read".into()].into(),
                },
            )]
            .into(),
            allowed_capabilities: ["file.read".into()].into(),
            target: "wasm32-wasip2".into(),
            valid_from: 1,
            valid_until: 500,
            ..Default::default()
        };
        Self {
            root,
            root_key,
            leaf,
            key,
            manifest,
            component,
            policy,
        }
    }
    fn signed(&self) -> (Vec<u8>, Vec<u8>) {
        let bytes = serde_json::to_vec(&self.manifest).unwrap();
        let sig = Signer::new_without_digest(&self.key)
            .unwrap()
            .sign_oneshot_to_vec(&bytes)
            .unwrap();
        (bytes, sig)
    }
    fn check(&self, time: u64) -> Result<VerifiedComponent, VerifyError> {
        let (bytes, sig) = self.signed();
        verify(
            &bytes,
            &sig,
            self.component.clone(),
            &[self.leaf.to_der().unwrap()],
            &self.policy,
            time,
        )
    }
}
#[test]
fn verifies_public_chain_signature_and_retains_exact_artifact() {
    let f = Fixture::new();
    let verified = f.check(150).unwrap();
    assert_eq!(verified.bytes(), f.component);
    assert_eq!(
        verified.manifest().signing_key_id,
        signing_key_id(&f.leaf.to_der().unwrap()).unwrap()
    );
    assert_eq!(
        verified.signing_certificate_sha256(),
        hex::encode(Sha256::digest(f.leaf.to_der().unwrap()))
    );
}
#[test]
fn rejects_unsigned_tampered_and_changed_artifacts() {
    let f = Fixture::new();
    let (bytes, sig) = f.signed();
    let chain = [f.leaf.to_der().unwrap()];
    assert!(matches!(
        verify(&bytes, &[], f.component.clone(), &chain, &f.policy, 150),
        Err(VerifyError::Signature)
    ));
    let mut changed = bytes.clone();
    changed.push(b' ');
    assert!(matches!(
        verify(&changed, &sig, f.component.clone(), &chain, &f.policy, 150),
        Err(VerifyError::Signature)
    ));
    let mut component = f.component.clone();
    component.push(0);
    assert!(matches!(
        verify(&bytes, &sig, component, &chain, &f.policy, 150),
        Err(VerifyError::Artifact)
    ));
}
#[test]
fn rejects_untrusted_chain_missing_chain_and_legacy_raw_keys() {
    let mut f = Fixture::new();
    let (bytes, sig) = f.signed();
    assert!(matches!(
        verify(&bytes, &sig, f.component.clone(), &[], &f.policy, 150),
        Err(VerifyError::Trust)
    ));
    assert!(matches!(
        verify(
            &bytes,
            &sig,
            f.component.clone(),
            &[f.key.public_key_to_der().unwrap()],
            &f.policy,
            150
        ),
        Err(VerifyError::Trust)
    ));
    f.policy.trust_anchors_der = Fixture::new().policy.trust_anchors_der;
    assert!(matches!(
        verify(
            &bytes,
            &sig,
            f.component.clone(),
            &[f.leaf.to_der().unwrap(), f.root.to_der().unwrap()],
            &f.policy,
            150
        ),
        Err(VerifyError::Trust)
    ));

    assert!(matches!(f.check(150), Err(VerifyError::Trust)));
    f.manifest.schema_version = 0;
    assert!(matches!(f.check(150), Err(VerifyError::Compatibility)));
}
#[test]
fn rejects_expired_future_and_wrong_purpose_certificates() {
    let mut f = Fixture::new();
    for (from, until, eku, digital, ca) in [
        (1, 120, Some(true), true, false),
        (160, 400, Some(true), true, false),
        (1, 400, Some(false), true, false),
        (1, 400, None, true, false),
        (1, 400, Some(true), false, false),
        (1, 400, Some(true), true, true),
    ] {
        f.leaf = certificate(
            &f.key,
            Some((&f.root, &f.root_key)),
            3,
            ca,
            eku,
            digital,
            from,
            until,
        );
        assert!(matches!(f.check(150), Err(VerifyError::Trust)));
    }
}
#[test]
fn certificate_renewal_keeps_key_identity_but_key_revocation_blocks_all_renewals() {
    let mut f = Fixture::new();
    let original_id = f.manifest.signing_key_id.clone();
    let old_cert = hex::encode(Sha256::digest(f.leaf.to_der().unwrap()));
    f.policy.revoked_certificates.insert(old_cert);
    assert!(matches!(f.check(150), Err(VerifyError::Trust)));
    f.leaf = certificate(
        &f.key,
        Some((&f.root, &f.root_key)),
        4,
        false,
        Some(true),
        true,
        80,
        450,
    );
    assert_eq!(
        signing_key_id(&f.leaf.to_der().unwrap()).unwrap(),
        original_id
    );
    f.check(150).unwrap();
    f.policy.revoked_keys.insert(original_id);
    assert!(matches!(f.check(150), Err(VerifyError::Trust)));
}
#[test]
fn key_rotation_accepts_new_identity_and_rejects_compromised_old_identity() {
    let mut old = Fixture::new();
    let key = PKey::generate_ed25519().unwrap();
    let leaf = certificate(
        &key,
        Some((&old.root, &old.root_key)),
        5,
        false,
        Some(true),
        true,
        50,
        400,
    );
    let id = signing_key_id(&leaf.to_der().unwrap()).unwrap();
    assert_ne!(id, old.manifest.signing_key_id);
    old.policy
        .revoked_keys
        .insert(old.manifest.signing_key_id.clone());
    old.policy.signers.insert(
        id.clone(),
        SignerGrant {
            plugin_ids: ["test.files".into()].into(),
            capabilities: ["file.read".into()].into(),
        },
    );
    assert!(matches!(old.check(150), Err(VerifyError::Trust)));
    old.key = key;
    old.leaf = leaf;
    old.manifest.signing_key_id = id;
    old.check(150).unwrap();
    old.manifest.signing_key_id = "arbitrary-alias".into();
    assert!(matches!(old.check(150), Err(VerifyError::Trust)));
}
#[test]
fn intermediate_chain_and_ca_revocation_are_enforced() {
    let mut f = Fixture::new();
    let intermediate_key = PKey::generate_ed25519().unwrap();
    let intermediate = certificate(
        &intermediate_key,
        Some((&f.root, &f.root_key)),
        6,
        true,
        Some(true),
        false,
        1,
        700,
    );
    f.leaf = certificate(
        &f.key,
        Some((&intermediate, &intermediate_key)),
        7,
        false,
        Some(true),
        true,
        50,
        400,
    );
    assert!(matches!(f.check(150), Err(VerifyError::Trust)));
    let (bytes, sig) = f.signed();
    let chain = [f.leaf.to_der().unwrap(), intermediate.to_der().unwrap()];
    verify(&bytes, &sig, f.component.clone(), &chain, &f.policy, 150).unwrap();
    f.policy
        .revoked_keys
        .insert(signing_key_id(&intermediate.to_der().unwrap()).unwrap());
    assert!(matches!(
        verify(&bytes, &sig, f.component.clone(), &chain, &f.policy, 150),
        Err(VerifyError::Trust)
    ));
}
#[test]
fn signer_plugin_capabilities_manifest_and_offline_policy_validity_are_enforced() {
    let mut f = Fixture::new();
    for time in [99, 300] {
        assert!(matches!(f.check(time), Err(VerifyError::Validity)));
    }
    f.policy.valid_until = 150;
    assert!(matches!(f.check(150), Err(VerifyError::Trust)));
    f.policy.valid_until = 500;
    f.policy
        .signers
        .get_mut(&f.manifest.signing_key_id)
        .unwrap()
        .capabilities
        .clear();
    assert!(matches!(f.check(150), Err(VerifyError::Capability)));
    f.policy
        .signers
        .get_mut(&f.manifest.signing_key_id)
        .unwrap()
        .plugin_ids
        .clear();
    assert!(matches!(f.check(150), Err(VerifyError::Trust)));
}
#[test]
fn rejects_wrong_api_target_and_native_binary() {
    let mut f = Fixture::new();
    f.manifest.api_version = 99;
    assert!(matches!(f.check(150), Err(VerifyError::Compatibility)));
    f.manifest.api_version = 1;
    f.manifest.target = "native-linux".into();
    assert!(matches!(f.check(150), Err(VerifyError::Compatibility)));
    f.manifest.target = "wasm32-wasip2".into();
    f.component = b"executable".to_vec();
    assert!(matches!(f.check(150), Err(VerifyError::Compatibility)));
}

pub(crate) fn signed_component(
    component: Vec<u8>,
) -> (
    onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact,
    TrustPolicy,
) {
    let mut f = Fixture::new();
    f.manifest.artifact_sha256 = hex::encode(Sha256::digest(&component));
    let (manifest_bytes, ed25519_signature) = f.signed();
    (
        onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact {
            manifest_bytes,
            ed25519_signature,
            artifact_bytes: component,
            x509_certificate_chain_der: vec![f.leaf.to_der().unwrap()],
        },
        f.policy,
    )
}
