// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Test CA hierarchy for tests only (this crate's tests and the `test-ca`
//! feature, which only dev-dependencies enable): Ed25519 root → Ed25519
//! generation → signing certificate (RSA 4096 by default) with the grants
//! extension. Private keys live in memory; tests that need a key file write
//! it to their own temporary directory. Nothing here bypasses verification:
//! a test trust base is passed explicitly, like the compiled-in one.
use crate::{
    Manifest, NativePart, TrustBase,
    trust::{GRANTS_OID, Grants, Revocations},
};
use onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact;
use openssl::{
    asn1::{Asn1Integer, Asn1Object, Asn1OctetString, Asn1Time},
    bn::BigNum,
    hash::MessageDigest,
    pkey::{Id, PKey, Private},
    rsa::{Padding, Rsa},
    sign::{RsaPssSaltlen, Signer},
    x509::{
        X509, X509Extension, X509NameBuilder,
        extension::{AuthorityKeyIdentifier, BasicConstraints, ExtendedKeyUsage, KeyUsage, SubjectKeyIdentifier},
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};

pub type Key = PKey<Private>;
/// Far future for test certificates (2100-01-01).
pub const FOREVER: i64 = 4_102_444_800;

/// What a test certificate is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Root,
    Generation,
    Leaf,
}

fn serial() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// One certificate. `issuer` `None` means self-signed. `grants` adds the
/// grants extension (leaf only in practice).
pub fn issue(cn: &str, key: &Key, issuer: Option<(&X509, &Key)>, kind: Kind, from: i64, until: i64,
    grants: Option<&Grants>) -> X509 {
    issue_inner(cn, key, issuer, kind, from, until, grants, true, true)
}

/// A signing certificate with code signing / digitalSignature switched off
/// as asked (for tests of the purpose rules).
pub fn issue_leaf_with(key: &Key, issuer: (&X509, &Key), from: i64, until: i64, grants: &Grants, code_signing: bool,
    digital_signature: bool) -> X509 {
    issue_inner("Onesimus Plugin Signer (test)", key, Some(issuer), Kind::Leaf, from, until, Some(grants), code_signing,
        digital_signature)
}

#[allow(clippy::too_many_arguments)]
fn issue_inner(cn: &str, key: &Key, issuer: Option<(&X509, &Key)>, kind: Kind, from: i64, until: i64,
    grants: Option<&Grants>, code_signing: bool, digital_signature: bool) -> X509 {
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", cn).unwrap();
    let name = name.build();
    let mut cert = X509::builder().unwrap();
    cert.set_version(2).unwrap();
    cert.set_serial_number(&Asn1Integer::from_bn(&BigNum::from_u32(serial()).unwrap()).unwrap()).unwrap();
    cert.set_subject_name(&name).unwrap();
    cert.set_issuer_name(issuer.map(|(c, _)| c.subject_name()).unwrap_or(&name)).unwrap();
    cert.set_pubkey(key).unwrap();
    cert.set_not_before(&Asn1Time::from_unix(from).unwrap()).unwrap();
    cert.set_not_after(&Asn1Time::from_unix(until).unwrap()).unwrap();
    let mut basic = BasicConstraints::new();
    basic.critical();
    let mut usage = KeyUsage::new();
    usage.critical();
    match kind {
        Kind::Root => {
            basic.ca();
            usage.key_cert_sign().crl_sign();
        }
        Kind::Generation => {
            basic.ca().pathlen(0);
            usage.key_cert_sign().crl_sign();
        }
        Kind::Leaf if digital_signature => {
            usage.digital_signature();
        }
        Kind::Leaf => {
            usage.key_encipherment();
        }
    }
    cert.append_extension(basic.build().unwrap()).unwrap();
    cert.append_extension(usage.build().unwrap()).unwrap();
    if kind == Kind::Leaf {
        let mut eku = ExtendedKeyUsage::new();
        if code_signing {
            eku.code_signing();
        } else {
            eku.server_auth();
        }
        cert.append_extension(eku.build().unwrap()).unwrap();
    }
    if let Some(grants) = grants {
        cert.append_extension(grants_extension(grants)).unwrap();
    }
    let ski = SubjectKeyIdentifier::new()
        .build(&cert.x509v3_context(issuer.map(|(c, _)| c.as_ref()), None))
        .unwrap();
    cert.append_extension(ski).unwrap();
    let aki = AuthorityKeyIdentifier::new()
        .keyid(true)
        .build(&cert.x509v3_context(issuer.map(|(c, _)| c.as_ref()), None))
        .unwrap();
    cert.append_extension(aki).unwrap();
    let signer = issuer.map(|(_, k)| k).unwrap_or(key);
    let digest = if signer.id() == Id::ED25519 { MessageDigest::null() } else { MessageDigest::sha256() };
    cert.sign(signer, digest).unwrap();
    cert.build()
}

/// The grants extension (non-critical; `ofd` requires it anyway).
pub fn grants_extension(grants: &Grants) -> X509Extension {
    let oid = Asn1Object::from_str(GRANTS_OID).unwrap();
    let value = Asn1OctetString::new_from_bytes(&grants.to_der()).unwrap();
    X509Extension::new_from_der(&oid, false, &value).unwrap()
}

pub fn grants(plugin_ids: &[&str], capabilities: &[&str]) -> Grants {
    Grants {
        plugin_ids: plugin_ids.iter().map(|s| s.to_string()).collect(),
        capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
    }
}

/// A root with one generation below it.
pub struct TestCa {
    pub root: X509,
    pub root_key: Key,
    pub generation: X509,
    pub generation_key: Key,
}

impl TestCa {
    pub fn new(label: &str, from: i64, until: i64) -> Self {
        let root_key = PKey::generate_ed25519().unwrap();
        let root = issue(&format!("Onesimus Plugin Test Root {label}"), &root_key, None, Kind::Root, from, until, None);
        let (generation, generation_key) = Self::generation_below(&root, &root_key, &format!("Onesimus Plugin Signing {label}"), from, until);
        Self { root, root_key, generation, generation_key }
    }

    /// One hierarchy per test process, valid until 2100.
    pub fn shared() -> &'static TestCa {
        static CA: OnceLock<TestCa> = OnceLock::new();
        CA.get_or_init(|| TestCa::new("G1", 1, FOREVER))
    }

    fn generation_below(root: &X509, root_key: &Key, cn: &str, from: i64, until: i64) -> (X509, Key) {
        let key = PKey::generate_ed25519().unwrap();
        (issue(cn, &key, Some((root, root_key)), Kind::Generation, from, until, None), key)
    }

    /// Another generation under the same root (e.g. "G2").
    pub fn next_generation(&self, label: &str, from: i64, until: i64) -> TestCa {
        let (generation, generation_key) =
            Self::generation_below(&self.root, &self.root_key, &format!("Onesimus Plugin Signing {label}"), from, until);
        TestCa {
            root: self.root.clone(),
            root_key: self.root_key.clone(),
            generation,
            generation_key,
        }
    }

    pub fn trust_base(&self) -> TrustBase {
        self.trust_base_with(Revocations::default())
    }

    pub fn trust_base_with(&self, revoked: Revocations) -> TrustBase {
        TrustBase::new(self.root.to_der().unwrap(), self.generation.to_der().unwrap(), revoked).unwrap()
    }

    /// A signing certificate for `key` issued by this generation.
    pub fn leaf(&self, key: &Key, grants: &Grants, from: i64, until: i64) -> X509 {
        issue("Onesimus Plugin Signer (test)", key, Some((&self.generation, &self.generation_key)), Kind::Leaf, from, until,
            Some(grants))
    }
}

/// One RSA 4096 key per test process (generating it takes a moment).
pub fn rsa4096() -> &'static Key {
    static KEY: OnceLock<Key> = OnceLock::new();
    KEY.get_or_init(|| PKey::from_rsa(Rsa::generate(4096).unwrap()).unwrap())
}

pub fn rsa(bits: u32) -> Key {
    PKey::from_rsa(Rsa::generate(bits).unwrap()).unwrap()
}

/// The signature `ofd` expects for this key type: Ed25519, or RSA-PSS with
/// SHA-256 and salt length = digest length (as `openssl dgst -sigopt
/// rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest` writes it).
pub fn sign(key: &Key, message: &[u8]) -> Vec<u8> {
    if key.id() == Id::ED25519 {
        return Signer::new_without_digest(key).unwrap().sign_oneshot_to_vec(message).unwrap();
    }
    let mut signer = Signer::new(MessageDigest::sha256(), key).unwrap();
    signer.set_rsa_padding(Padding::PKCS1_PSS).unwrap();
    signer.set_rsa_pss_saltlen(RsaPssSaltlen::DIGEST_LENGTH).unwrap();
    signer.set_rsa_mgf1_md(MessageDigest::sha256()).unwrap();
    signer.sign_oneshot_to_vec(message).unwrap()
}

/// RSA PKCS#1 v1.5 with SHA-256: must be refused.
pub fn sign_pkcs1(key: &Key, message: &[u8]) -> Vec<u8> {
    let mut signer = Signer::new(MessageDigest::sha256(), key).unwrap();
    signer.set_rsa_padding(Padding::PKCS1).unwrap();
    signer.sign_oneshot_to_vec(message).unwrap()
}

/// RSA-PSS with the maximum salt length: must be refused too.
pub fn sign_pss_max_salt(key: &Key, message: &[u8]) -> Vec<u8> {
    let mut signer = Signer::new(MessageDigest::sha256(), key).unwrap();
    signer.set_rsa_padding(Padding::PKCS1_PSS).unwrap();
    signer.set_rsa_pss_saltlen(RsaPssSaltlen::MAXIMUM_LENGTH).unwrap();
    signer.set_rsa_mgf1_md(MessageDigest::sha256()).unwrap();
    signer.sign_oneshot_to_vec(message).unwrap()
}

/// Suite version of this workspace (as `ofd` of the same build sees it).
pub fn suite() -> String {
    crate::suite_of(env!("CARGO_PKG_VERSION")).expect("workspace version has a +onesimus. part").to_string()
}

/// A schema-3 manifest for `artifact` signed by the leaf `leaf`.
pub fn manifest(artifact: &[u8], leaf: &X509, plugin_id: &str, capabilities: &[&str], native: BTreeMap<String, Vec<NativePart>>,
    not_before: u64, not_after: u64) -> Manifest {
    Manifest {
        schema_version: 3,
        plugin_id: plugin_id.into(),
        version: "1.0.0".into(),
        // Podman grants need contract 2, like signing::prepare decides.
        api_version: if capabilities.iter().any(|c| crate::is_podman_grant(c)) {
            crate::API_VERSION_SOURCE_REF
        } else {
            crate::API_VERSION
        },
        artifact_sha256: hex::encode(Sha256::digest(artifact)),
        signing_key_id: crate::signing_key_id(&leaf.to_der().unwrap()).unwrap(),
        not_before,
        not_after,
        capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
        target: crate::TARGET.into(),
        native,
        suite: Some(suite()),
    }
}

/// A bundle signed under the shared hierarchy with the shared RSA 4096 key,
/// valid from `now - 60` for an hour, with grants for exactly this plugin
/// and these capabilities.
pub fn signed_bundle(artifact: Vec<u8>, plugin_id: &str, capabilities: &[&str], native: BTreeMap<String, Vec<NativePart>>,
    now: u64) -> SignedPluginArtifact {
    let ca = TestCa::shared();
    let key = rsa4096();
    let leaf = ca.leaf(key, &grants(&[plugin_id], capabilities), now as i64 - 60, now as i64 + 3600);
    let manifest = manifest(&artifact, &leaf, plugin_id, capabilities, native, now - 60, now + 3600);
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    SignedPluginArtifact {
        ed25519_signature: sign(key, &manifest_bytes),
        manifest_bytes,
        artifact_bytes: artifact,
        x509_certificate_chain_der: vec![leaf.to_der().unwrap(), ca.generation.to_der().unwrap()],
    }
}

/// Capabilities as a set (helper for policies in tests).
pub fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}
