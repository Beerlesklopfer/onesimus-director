// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
use super::*;
use crate::testca::{self, Key, Kind, TestCa, grants, issue, rsa4096, sign};
use openssl::{pkey::PKey, x509::X509};

/// Ed25519 root → Ed25519 generation G1 → RSA 4096 leaf with grants for
/// `test.files` / `file.read`. Times: CA 1..1000, leaf 50..400, manifest
/// 100..300; checks run at 150.
struct Fixture {
    ca: TestCa,
    leaf: X509,
    key: Key,
    manifest: Manifest,
    component: Vec<u8>,
    policy: TrustPolicy,
    /// Bundle chain; `None`: leaf and generation.
    chain: Option<Vec<Vec<u8>>>,
}
impl Fixture {
    fn new() -> Self {
        let ca = TestCa::new("G1", 1, 1000);
        let key = rsa4096().clone();
        let leaf = ca.leaf(&key, &grants(&["test.files"], &["file.read"]), 50, 400);
        let component = COMPONENT_HEADER.to_vec();
        let manifest = testca::manifest(&component, &leaf, "test.files", &["file.read"], Default::default(), 100, 300);
        let policy = TrustPolicy { base: ca.trust_base(), local: LocalPolicy::default(), suite: testca::suite() };
        Self { ca, leaf, key, manifest, component, policy, chain: None }
    }
    fn with_leaf(&mut self, key: &Key, grants: &Grants, from: i64, until: i64) {
        self.key = key.clone();
        self.leaf = self.ca.leaf(key, grants, from, until);
        self.manifest.signing_key_id = signing_key_id(&self.leaf.to_der().unwrap()).unwrap();
    }
    fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.manifest).unwrap()
    }
    fn chain(&self) -> Vec<Vec<u8>> {
        self.chain.clone().unwrap_or_else(|| vec![self.leaf.to_der().unwrap(), self.ca.generation.to_der().unwrap()])
    }
    fn check_with(&self, signature: &[u8], time: u64) -> Result<VerifiedComponent, VerifyError> {
        verify(&self.bytes(), signature, self.component.clone(), &self.chain(), &self.policy, time)
    }
    fn check(&self, time: u64) -> Result<VerifiedComponent, VerifyError> {
        self.check_with(&sign(&self.key, &self.bytes()), time)
    }
}

macro_rules! refused {
    ($result:expr, $pattern:pat) => {
        match $result {
            Err($pattern) => {}
            other => panic!("expected {}, got {:?}", stringify!($pattern), other.map(|v| v.manifest().plugin_id.clone())),
        }
    };
}

#[test]
fn verifies_rsa_pss_and_ed25519_leaves_and_retains_exact_artifact() {
    let mut f = Fixture::new();
    let verified = f.check(150).unwrap();
    assert_eq!(verified.bytes(), f.component);
    assert_eq!(
        verified.signing_certificate_sha256(),
        hex::encode(Sha256::digest(f.leaf.to_der().unwrap()))
    );
    // The leaf alone suffices; the generation comes from the trust base.
    f.chain = Some(vec![f.leaf.to_der().unwrap()]);
    f.check(150).unwrap();
    // Ed25519 signing certificates are accepted as well.
    f.chain = None;
    f.with_leaf(&PKey::generate_ed25519().unwrap(), &grants(&["test.files"], &["file.read"]), 50, 400);
    f.check(150).unwrap();
}

#[test]
fn rejects_unsigned_tampered_and_changed_artifacts() {
    let f = Fixture::new();
    let bytes = f.bytes();
    let sig = sign(&f.key, &bytes);
    let chain = f.chain();
    refused!(verify(&bytes, &[], f.component.clone(), &chain, &f.policy, 150), VerifyError::Signature);
    let mut changed = bytes.clone();
    changed.push(b' ');
    refused!(verify(&changed, &sig, f.component.clone(), &chain, &f.policy, 150), VerifyError::Signature);
    let mut flipped = sig.clone();
    flipped[10] ^= 1;
    refused!(f.check_with(&flipped, 150), VerifyError::Signature);
    let mut component = f.component.clone();
    component.push(0);
    refused!(verify(&bytes, &sig, component, &chain, &f.policy, 150), VerifyError::Artifact);
}

#[test]
fn only_rsa_pss_4096_with_digest_salt_is_accepted() {
    let mut f = Fixture::new();
    let bytes = f.bytes();
    refused!(f.check_with(&testca::sign_pkcs1(&f.key, &bytes), 150), VerifyError::Signature);
    refused!(f.check_with(&testca::sign_pss_max_salt(&f.key, &bytes), 150), VerifyError::Signature);
    // RSA 2048 passes the chain (auth level 2) but not the signature rule.
    f.with_leaf(&testca::rsa(2048), &grants(&["test.files"], &["file.read"]), 50, 400);
    refused!(f.check(150), VerifyError::Signature);
    // Neither does a signature of another key length on a 4096-bit leaf.
    let mut f = Fixture::new();
    f.key = testca::rsa(2048);
    refused!(f.check(150), VerifyError::Signature);
}

#[test]
fn rejects_foreign_ca_missing_chain_raw_keys_and_leaves_without_generation() {
    let mut f = Fixture::new();
    let bytes = f.bytes();
    let sig = sign(&f.key, &bytes);
    refused!(verify(&bytes, &sig, f.component.clone(), &[], &f.policy, 150), VerifyError::Trust);
    refused!(
        verify(&bytes, &sig, f.component.clone(), &[f.key.public_key_to_der().unwrap()], &f.policy, 150),
        VerifyError::Trust
    );
    // A foreign hierarchy, even with its own generation in the bundle.
    let foreign = TestCa::new("G1", 1, 1000);
    let leaf = foreign.leaf(&f.key, &grants(&["test.files"], &["file.read"]), 50, 400);
    f.chain = Some(vec![leaf.to_der().unwrap(), foreign.generation.to_der().unwrap()]);
    refused!(f.check(150), VerifyError::Trust);
    f.chain = Some(vec![leaf.to_der().unwrap()]);
    refused!(f.check(150), VerifyError::Trust);
    // A leaf issued by the root directly (no generation).
    let direct = issue("direct", &f.key, Some((&f.ca.root, &f.ca.root_key)), Kind::Leaf, 50, 400,
        Some(&grants(&["test.files"], &["file.read"])));
    f.chain = Some(vec![direct.to_der().unwrap()]);
    refused!(f.check(150), VerifyError::Trust);
    // The bundle may repeat the generation, nothing else.
    f.chain = Some(vec![f.leaf.to_der().unwrap(), f.ca.root.to_der().unwrap()]);
    refused!(f.check(150), VerifyError::Trust);
}

#[test]
fn only_the_generation_of_this_release_is_accepted() {
    let mut f = Fixture::new();
    let g2 = f.ca.next_generation("G2", 1, 1000);
    let leaf_g2 = g2.leaf(&f.key, &grants(&["test.files"], &["file.read"]), 50, 400);
    let g1_chain = f.chain();
    // ofd of G1 and a bundle of G2 (same root): refused, also with G2 in the bundle.
    f.chain = Some(vec![leaf_g2.to_der().unwrap(), g2.generation.to_der().unwrap()]);
    refused!(f.check(150), VerifyError::Trust);
    f.chain = Some(vec![leaf_g2.to_der().unwrap()]);
    refused!(f.check(150), VerifyError::Trust);
    // ofd of G2: the G2 bundle loads, the G1 bundle no longer (hard cut).
    f.policy.base = g2.trust_base();
    f.check(150).unwrap();
    f.chain = Some(g1_chain);
    refused!(f.check(150), VerifyError::Trust);
}

#[test]
fn certificate_validity_is_checked_at_load_time() {
    let mut f = Fixture::new();
    let g = grants(&["test.files"], &["file.read"]);
    let key = f.key.clone();
    // Expired and not yet valid signing certificate.
    for (from, until) in [(1, 120), (160, 400)] {
        f.with_leaf(&key, &g, from, until);
        refused!(f.check(150), VerifyError::Trust);
    }
    f.with_leaf(&key, &g, 50, 400);
    f.check(150).unwrap();
    // The same bundle stops loading once the certificate expires.
    f.manifest.not_after = 900;
    refused!(f.check(450), VerifyError::Trust);
    // Expired generation.
    let mut f = Fixture::new();
    let old = TestCa::new("G1", 1, 1000).next_generation("G1", 1, 120);
    f.ca = old;
    f.policy.base = f.ca.trust_base();
    f.with_leaf(&key, &g, 50, 400);
    refused!(f.check(150), VerifyError::Trust);
}

#[test]
fn rejects_wrong_purpose_certificates() {
    let g = grants(&["test.files"], &["file.read"]);
    // CA certificates as signing certificate.
    for kind in [Kind::Generation, Kind::Root] {
        let mut f = Fixture::new();
        f.leaf = issue("wrong", &f.key, Some((&f.ca.generation, &f.ca.generation_key)), kind, 50, 400, Some(&g));
        refused!(f.check(150), VerifyError::Trust);
    }
    // Without code signing or without digitalSignature.
    for (code_signing, digital_signature) in [(false, true), (true, false)] {
        let mut f = Fixture::new();
        f.leaf = testca::issue_leaf_with(&f.key, (&f.ca.generation, &f.ca.generation_key), 50, 400, &g, code_signing,
            digital_signature);
        refused!(f.check(150), VerifyError::Trust);
    }
}

#[test]
fn revocations_from_the_trust_base_and_the_local_policy_apply() {
    type Revoke = fn(&mut Revocations, &str, &str, &str);
    let revokes: [Revoke; 4] = [
        |r, cert, _, _| { r.revoked_certificates.insert(cert.into()); },
        |r, _, key, _| { r.revoked_keys.insert(key.into()); },
        |r, _, _, generation| { r.revoked_keys.insert(generation.into()); },
        |r, _, _, _| { r.revoked_plugins.insert("test.files".into()); },
    ];
    for revoke in revokes {
        let mut f = Fixture::new();
        let cert = hex::encode(Sha256::digest(f.leaf.to_der().unwrap()));
        let generation_key = signing_key_id(&f.ca.generation.to_der().unwrap()).unwrap();
        let mut built_in = Revocations::default();
        revoke(&mut built_in, &cert, &f.manifest.signing_key_id, &generation_key);
        f.check(150).unwrap();
        f.policy.base = f.ca.trust_base_with(built_in.clone());
        refused!(f.check(150), VerifyError::Trust);
        f.policy.base = f.ca.trust_base();
        f.policy.local.revoked = built_in;
        refused!(f.check(150), VerifyError::Trust);
    }
}

#[test]
fn grants_in_the_certificate_bind_plugins_and_capabilities() {
    let mut f = Fixture::new();
    let key = f.key.clone();
    f.with_leaf(&key, &grants(&["test.other"], &["file.read"]), 50, 400);
    refused!(f.check(150), VerifyError::Trust);
    f.with_leaf(&key, &grants(&["test.files"], &[]), 50, 400);
    refused!(f.check(150), VerifyError::Capability);
    f.with_leaf(&key, &grants(&["test.files", "test.other"], &["file.read", "file.write"]), 50, 400);
    f.check(150).unwrap();
    // No grants extension at all.
    f.leaf = issue("no grants", &key, Some((&f.ca.generation, &f.ca.generation_key)), Kind::Leaf, 50, 400, None);
    refused!(f.check(150), VerifyError::Trust);
    // The manifest names another key than the certificate.
    let mut f = Fixture::new();
    f.manifest.signing_key_id = "ab".repeat(32);
    refused!(f.check(150), VerifyError::Trust);
}

#[test]
fn the_local_policy_can_only_restrict() {
    let mut f = Fixture::new();
    f.policy.local.plugin_ids = Some(testca::set(&["test.other"]));
    refused!(f.check(150), VerifyError::Trust);
    f.policy.local.plugin_ids = Some(testca::set(&["test.files", "test.more"]));
    f.check(150).unwrap();
    f.policy.local.allowed_capabilities = Some(testca::set(&[]));
    refused!(f.check(150), VerifyError::Capability);
    // Allowing more than the certificate grants does not widen anything.
    f.policy.local.allowed_capabilities = Some(testca::set(&["file.read", "file.write"]));
    f.manifest.capabilities = vec!["file.read".into(), "file.write".into()];
    refused!(f.check(150), VerifyError::Capability);
    f.manifest.capabilities = vec!["file.read".into()];
    f.policy.local.validity = Some((1, 150));
    refused!(f.check(150), VerifyError::Trust);
    f.policy.local.validity = Some((1, 151));
    f.check(150).unwrap();
}

#[test]
fn suite_must_match_exactly_and_old_schemas_no_longer_load() {
    let mut f = Fixture::new();
    f.policy.suite = "0.2.1".into();
    refused!(f.check(150), VerifyError::Suite);
    f.policy.suite = testca::suite();
    // Schema 1 and 2: still parsed and checked, but they carry no suite.
    f.manifest.schema_version = 1;
    f.manifest.suite = None;
    refused!(f.check(150), VerifyError::Suite);
    f.manifest.schema_version = 2;
    f.manifest.native = [("x86_64-linux-gnu".to_string(), vec![NativePart { path: "b".into(), sha256: "ab".repeat(32) }])].into();
    refused!(f.check(150), VerifyError::Suite);
    f.manifest.native.clear();
    refused!(f.check(150), VerifyError::Manifest(_));
    f.manifest.schema_version = 1;
    f.manifest.suite = Some(testca::suite());
    refused!(f.check(150), VerifyError::Manifest(_));
    f.manifest.schema_version = 3;
    f.manifest.suite = None;
    refused!(f.check(150), VerifyError::Manifest(_));
    f.manifest.suite = Some("0.2 0".into());
    refused!(f.check(150), VerifyError::Manifest(_));
    f.manifest.suite = Some(testca::suite());
    f.manifest.schema_version = 4;
    refused!(f.check(150), VerifyError::Compatibility);
}

#[test]
fn manifest_validity_api_target_and_component_format_are_checked() {
    let mut f = Fixture::new();
    for time in [99, 300] {
        refused!(f.check(time), VerifyError::Validity);
    }
    f.manifest.api_version = 99;
    refused!(f.check(150), VerifyError::Compatibility);
    f.manifest.api_version = 1;
    f.manifest.target = "native-linux".into();
    refused!(f.check(150), VerifyError::Compatibility);
    f.manifest.target = TARGET.into();
    f.component = b"executable".to_vec();
    f.manifest.artifact_sha256 = hex::encode(Sha256::digest(&f.component));
    refused!(f.check(150), VerifyError::Compatibility);
}

/// Contract versions {1, 2} load side by side (V-16, user decision
/// 19.11.2026); 0 and 3 do not.
#[test]
fn contract_versions_one_and_two_are_accepted() {
    let mut f = Fixture::new();
    for v in [1, 2] {
        f.manifest.api_version = v;
        assert_eq!(f.check(150).unwrap().manifest().api_version, v);
    }
    for v in [0, 3] {
        f.manifest.api_version = v;
        refused!(f.check(150), VerifyError::Compatibility);
    }
}

/// Podman grants are bound to io.onesimus.podman (V-11, user decision
/// 19.11.2026), even when the certificate grants them to another plugin
/// id, and need contract 2.
#[test]
fn podman_grants_are_bound_to_the_podman_plugin_id() {
    let mut f = Fixture::new();
    let key = f.key.clone();
    let caps = ["podman.inventory", "podman.rootful"];
    // A suite certificate granting podman.* to another plugin id too.
    f.with_leaf(&key, &grants(&["test.files", PODMAN_PLUGIN_ID], &caps), 50, 400);
    f.manifest.capabilities = caps.iter().map(|c| c.to_string()).collect();
    f.manifest.api_version = 2;
    refused!(f.check(150), VerifyError::Capability);
    f.manifest.plugin_id = PODMAN_PLUGIN_ID.into();
    assert_eq!(f.check(150).unwrap().manifest().plugin_id, PODMAN_PLUGIN_ID);
    f.manifest.api_version = 1;
    refused!(f.check(150), VerifyError::Compatibility);
    assert!(is_podman_grant("podman.lifecycle") && !is_podman_grant("postgresql.read"));
}

#[test]
fn schema_3_binds_native_parts() {
    let hash = "ab".repeat(32);
    let part = |path: &str, sha: &str| NativePart { path: path.into(), sha256: sha.into() };
    let mut f = Fixture::new();
    f.manifest.native = [("x86_64-linux-gnu".to_string(), vec![part("postgresql/onesimus-pg-broker", &hash)])].into();
    let verified = f.check(200).unwrap();
    assert_eq!(verified.manifest().native_part("x86_64-linux-gnu", "postgresql/onesimus-pg-broker").unwrap().sha256, hash);
    assert!(verified.manifest().native_part("aarch64-linux-gnu", "postgresql/onesimus-pg-broker").is_none());
    for (triplet, p) in [
        ("x86_64-linux-gnu", part("../evil", &hash)),
        ("x86_64-linux-gnu", part("/usr/bin/evil", &hash)),
        ("x86_64-linux-gnu", part("a//b", &hash)),
        ("x86_64-linux-gnu", part("broker", "AB")),
        ("x86_64-linux-gnu", part("broker", &"AB".repeat(32))),
        ("X86_64", part("broker", &hash)),
    ] {
        f.manifest.native = [(triplet.to_string(), vec![p.clone()])].into();
        refused!(f.check(200), VerifyError::Manifest(_));
    }
    f.manifest.native = [("x86_64-linux-gnu".to_string(), vec![])].into();
    refused!(f.check(200), VerifyError::Manifest(_));
}

/// A tiny real component for the signing round trip.
fn component() -> Vec<u8> {
    wat::parse_str("(component)").unwrap()
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("osig-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn openssl_sign(dir: &std::path::Path, key: &Key, pss: bool) -> Vec<u8> {
    use std::os::unix::fs::OpenOptionsExt;
    let key_path = dir.join("test-signing-key.pem");
    std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&key_path).unwrap();
    std::fs::write(&key_path, key.private_key_to_pem_pkcs8().unwrap()).unwrap();
    let mut command = std::process::Command::new("openssl");
    command.args(["dgst", "-sha256", "-sign"]).arg(&key_path);
    if pss {
        command.args(["-sigopt", "rsa_padding_mode:pss", "-sigopt", "rsa_pss_saltlen:digest"]);
    }
    let status = command.arg("-out").arg(dir.join("manifest.sig")).arg(dir.join("manifest.json")).status()
        .expect("openssl CLI");
    assert!(status.success());
    std::fs::remove_file(&key_path).unwrap();
    std::fs::read(dir.join("manifest.sig")).unwrap()
}

#[test]
fn prepare_external_openssl_signature_assemble_round_trip() {
    let dir = tmp("roundtrip");
    let ca = TestCa::shared();
    let key = rsa4096();
    let leaf = ca.leaf(key, &grants(&["io.onesimus.example"], &["file.read"]), 100, testca::FOREVER);
    let request = signing::PrepareRequest {
        plugin_id: "io.onesimus.example".into(),
        version: "1.2.3".into(),
        capabilities: vec!["file.read".into()],
        native: Default::default(),
        suite: testca::suite(),
        not_before: None,
        not_after: None,
    };
    let manifest = signing::prepare(&component(), &leaf.to_der().unwrap(), &request).unwrap();
    std::fs::write(dir.join("manifest.json"), &manifest).unwrap();
    let parsed = parse_manifest(&manifest).unwrap();
    assert_eq!((parsed.schema_version, parsed.not_before, parsed.suite.as_deref()), (3, 100, Some(testca::suite().as_str())));

    let chain = vec![leaf.to_der().unwrap(), ca.generation.to_der().unwrap()];
    let signature = openssl_sign(&dir, key, true);
    assert_eq!(signature.len(), 512);
    let base = ca.trust_base();
    let bundle = signing::assemble(component(), manifest.clone(), signature.clone(), chain.clone(), Some(&base), 1000).unwrap();
    let policy = TrustPolicy { base: base.clone(), local: LocalPolicy::default(), suite: testca::suite() };
    verify(&bundle.manifest_bytes, &bundle.ed25519_signature, bundle.artifact_bytes, &bundle.x509_certificate_chain_der,
        &policy, 1000).unwrap();

    // PKCS#1 v1.5 from the same CLI: assemble refuses to write a bundle.
    let pkcs1 = openssl_sign(&dir, key, false);
    assert!(signing::assemble(component(), manifest.clone(), pkcs1, chain.clone(), None, 1000).is_err());
    // A changed component or a foreign trust base.
    let mut other = component();
    other.push(0);
    assert!(signing::assemble(other, manifest.clone(), signature.clone(), chain.clone(), None, 1000).is_err());
    let foreign = TestCa::new("G1", 1, testca::FOREVER).trust_base();
    assert!(signing::assemble(component(), manifest, signature, chain, Some(&foreign), 1000).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn prepare_refuses_what_the_certificate_does_not_grant() {
    let ca = TestCa::shared();
    let leaf = ca.leaf(rsa4096(), &grants(&["io.onesimus.example"], &["file.read"]), 100, testca::FOREVER);
    let mut request = signing::PrepareRequest {
        plugin_id: "io.onesimus.example".into(),
        version: "1".into(),
        capabilities: vec!["postgresql.read".into()],
        native: Default::default(),
        suite: testca::suite(),
        not_before: None,
        not_after: None,
    };
    let der = leaf.to_der().unwrap();
    assert!(signing::prepare(&component(), &der, &request).unwrap_err().contains("postgresql.read"));
    request.capabilities.clear();
    request.plugin_id = "io.onesimus.other".into();
    assert!(signing::prepare(&component(), &der, &request).is_err());
    request.plugin_id = "io.onesimus.example".into();
    assert!(signing::prepare(b"not wasm", &der, &request).is_err());
    let small = ca.leaf(&testca::rsa(2048), &grants(&["io.onesimus.example"], &[]), 100, testca::FOREVER);
    assert!(signing::prepare(&component(), &small.to_der().unwrap(), &request).unwrap_err().contains("4096"));
    signing::prepare(&component(), &der, &request).unwrap();
}

pub(crate) fn signed_component(
    component: Vec<u8>,
) -> (
    onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact,
    TrustPolicy,
) {
    signed_component_with(component, &["file.read"])
}

/// A bundle under the shared test hierarchy with these capabilities granted
/// and requested; valid at time 150 (leaf 50..400, manifest 100..300).
pub(crate) fn signed_component_with(
    component: Vec<u8>,
    capabilities: &[&str],
) -> (
    onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact,
    TrustPolicy,
) {
    let ca = TestCa::shared();
    let key = rsa4096();
    let leaf = ca.leaf(key, &grants(&["test.files"], capabilities), 50, 400);
    let manifest = testca::manifest(&component, &leaf, "test.files", capabilities, Default::default(), 100, 300);
    let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
    (
        onesimus_proto_rs::onesimus::filed::plugin::v1::SignedPluginArtifact {
            ed25519_signature: sign(key, &manifest_bytes),
            manifest_bytes,
            artifact_bytes: component,
            x509_certificate_chain_der: vec![leaf.to_der().unwrap()],
        },
        TrustPolicy { base: ca.trust_base(), local: LocalPolicy::default(), suite: testca::suite() },
    )
}
