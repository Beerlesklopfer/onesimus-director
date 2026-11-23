// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Trust base and grants (plan: docs/plans/SIGNIER-INFRASTRUKTUR.md).
//!
//! The trust base is compiled into `ofd`: the offline root (Ed25519), the
//! intermediate of this release's generation ("Onesimus Plugin Signing G<n>")
//! and a revocation list. A plugin loads only if its signing certificate was
//! issued by exactly this generation under exactly this root. What a signer
//! may sign (plugin IDs, capabilities) is stated by the CA in an X.509
//! extension of the signing certificate ([`GRANTS_OID`]), not in local files.
//! The local `Plugin Policy` ([`LocalPolicy`]) can only restrict.
use openssl::{
    pkey::Id,
    x509::{X509, X509VerifyResult},
};
use serde::Deserialize;
use std::collections::BTreeSet;

/// OID of the grants extension in a plugin signing certificate. Arc
/// `2.25.<UUID>` is the self-assigned UUID arc (ITU-T X.667, RFC 4122
/// section 4.4 / ISO/IEC 9834-8); `.1` below it is "plugin grants".
/// Changing it invalidates every signing certificate (= new generation).
pub const GRANTS_OID: &str = "2.25.166226948137511682553772901840964231856.1";
/// DER content octets of [`GRANTS_OID`] (checked against OpenSSL in tests).
pub(crate) const GRANTS_OID_DER: &[u8] = &[
    0x69, 0x81, 0xfa, 0x8e, 0x92, 0xd3, 0x8f, 0xa6, 0xba, 0x9a, 0xff, 0x9f, 0xc4, 0xee, 0xdd, 0xe5,
    0x89, 0xbc, 0xc5, 0x30, 0x01,
];
/// The only target a plugin may be built for.
pub const TARGET: &str = "wasm32-wasip2";
/// Version of the grants structure in the extension.
const GRANTS_VERSION: u8 = 1;
const MAX_GRANTS: usize = 64;
const MAX_NAME: usize = 128;

/// Suite version of a workspace version: the Onesimus part after
/// `+onesimus.`, e.g. `0.2.0` for `25.1.2+onesimus.0.2.0`. The Bareos
/// compatibility prefix does not count. `None` if there is no such part:
/// such a build loads no plugin.
pub fn suite_of(version: &str) -> Option<&str> {
    let (_, suite) = version.split_once("+onesimus.")?;
    valid_suite(suite).then_some(suite)
}

pub(crate) fn valid_suite(suite: &str) -> bool {
    !suite.is_empty()
        && suite.len() <= 64
        && suite.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
}

/// Plugin IDs and capabilities a signing certificate may sign for.
///
/// ```text
/// OnesimusPluginGrants ::= SEQUENCE {
///     version       INTEGER (1),
///     pluginIds     SEQUENCE SIZE (1..64) OF UTF8String,
///     capabilities  SEQUENCE SIZE (0..64) OF UTF8String }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    pub plugin_ids: BTreeSet<String>,
    pub capabilities: BTreeSet<String>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'))
}

impl Grants {
    /// DER of the extension value (for tests and tooling; the CA writes it
    /// with an OpenSSL config, see the plan).
    pub fn to_der(&self) -> Vec<u8> {
        let list = |items: &BTreeSet<String>| {
            let mut body = Vec::new();
            for item in items {
                body.extend(tlv(0x0c, item.as_bytes()));
            }
            tlv(0x30, &body)
        };
        let mut body = tlv(0x02, &[GRANTS_VERSION]);
        body.extend(list(&self.plugin_ids));
        body.extend(list(&self.capabilities));
        tlv(0x30, &body)
    }

    /// Strict DER: definite minimal lengths, no trailing bytes, version 1,
    /// at least one plugin ID, no duplicates, names `[a-z0-9._-]{1,128}`.
    pub fn from_der(der: &[u8]) -> Result<Self, String> {
        let (body, rest) = read_tlv(der, 0x30)?;
        if !rest.is_empty() {
            return Err("trailing bytes after grants".into());
        }
        let (version, body) = read_tlv(body, 0x02)?;
        if version != [GRANTS_VERSION] {
            return Err("unsupported grants version".into());
        }
        let (ids, body) = read_tlv(body, 0x30)?;
        let (caps, body) = read_tlv(body, 0x30)?;
        if !body.is_empty() {
            return Err("trailing fields in grants".into());
        }
        let names = |mut list: &[u8]| -> Result<BTreeSet<String>, String> {
            let mut out = BTreeSet::new();
            while !list.is_empty() {
                let (value, rest) = read_tlv(list, 0x0c)?;
                let name = std::str::from_utf8(value).map_err(|_| "grant is not UTF-8")?;
                if !valid_name(name) || !out.insert(name.to_string()) || out.len() > MAX_GRANTS {
                    return Err(format!("invalid or duplicate grant \"{}\"", name.escape_default()));
                }
                list = rest;
            }
            Ok(out)
        };
        let grants = Self { plugin_ids: names(ids)?, capabilities: names(caps)? };
        if grants.plugin_ids.is_empty() {
            return Err("grants name no plugin".into());
        }
        Ok(grants)
    }
}

fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = value.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes: Vec<u8> = len.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(value);
    out
}

/// One DER element with tag `tag`: (value, rest).
fn read_tlv(input: &[u8], tag: u8) -> Result<(&[u8], &[u8]), String> {
    let bad = || Err("malformed grants DER".to_string());
    let [t, first, rest @ ..] = input else { return bad() };
    if *t != tag {
        return bad();
    }
    let (len, rest) = if *first < 0x80 {
        (*first as usize, rest)
    } else {
        let n = (*first & 0x7f) as usize;
        if n == 0 || n > 2 || rest.len() < n || rest[0] == 0 {
            return bad();
        }
        let len = rest[..n].iter().fold(0usize, |acc, b| acc << 8 | *b as usize);
        if len < 0x80 {
            return bad();
        }
        (len, &rest[n..])
    };
    if rest.len() < len {
        return bad();
    }
    Ok(rest.split_at(len))
}

/// Revoked SPKI fingerprints (any chain member), certificate fingerprints
/// and plugin IDs. Fingerprints are SHA-256, 64 lowercase hex digits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revocations {
    #[serde(default)]
    pub revoked_keys: BTreeSet<String>,
    #[serde(default)]
    pub revoked_certificates: BTreeSet<String>,
    #[serde(default)]
    pub revoked_plugins: BTreeSet<String>,
}

impl Revocations {
    /// A mistyped fingerprint would silently revoke nothing: refuse it.
    pub fn check(&self) -> Result<(), String> {
        let hex = |s: &String| s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if let Some(bad) = self.revoked_keys.iter().chain(&self.revoked_certificates).find(|s| !hex(s)) {
            return Err(format!("revocation \"{}\" is not 64 lowercase hex digits", bad.escape_default()));
        }
        if let Some(bad) = self.revoked_plugins.iter().find(|s| !valid_name(s)) {
            return Err(format!("revoked plugin \"{}\" is not a plugin ID", bad.escape_default()));
        }
        Ok(())
    }
    pub(crate) fn key(&self, id: &str) -> bool {
        self.revoked_keys.contains(id)
    }
    pub(crate) fn certificate(&self, id: &str) -> bool {
        self.revoked_certificates.contains(id)
    }
    pub(crate) fn plugin(&self, id: &str) -> bool {
        self.revoked_plugins.contains(id)
    }
}

/// The compiled-in trust base: root, the generation of this release and the
/// built-in revocation list. Bundles cannot add to it.
#[derive(Debug, Clone)]
pub struct TrustBase {
    root_der: Vec<u8>,
    generation_der: Vec<u8>,
    generation_name: String,
    revoked: Revocations,
}

impl TrustBase {
    /// Checks the shape: the root is a self-signed Ed25519 CA, the
    /// generation a CA certificate issued and signed by that root.
    pub fn new(root_der: Vec<u8>, generation_der: Vec<u8>, revoked: Revocations) -> Result<Self, String> {
        revoked.check()?;
        let root = X509::from_der(&root_der).map_err(|_| "root is not a DER certificate")?;
        let generation = X509::from_der(&generation_der).map_err(|_| "generation is not a DER certificate")?;
        let root_key = root.public_key().map_err(|_| "root key unreadable")?;
        if root_key.id() != Id::ED25519 {
            return Err("the plugin root CA must be an Ed25519 key".into());
        }
        let ca = |der: &[u8]| -> Result<bool, String> {
            let (_, cert) = x509_parser::parse_x509_certificate(der).map_err(|_| "certificate unparsable")?;
            Ok(cert.is_ca())
        };
        if !ca(&root_der)? || !ca(&generation_der)? {
            return Err("root and generation must be CA certificates".into());
        }
        if root.issued(&root) != X509VerifyResult::OK || !root.verify(&root_key).unwrap_or(false) {
            return Err("root is not self-signed".into());
        }
        if root.issued(&generation) != X509VerifyResult::OK || !generation.verify(&root_key).unwrap_or(false) {
            return Err("generation was not issued by the root".into());
        }
        let generation_name = generation
            .subject_name()
            .entries_by_nid(openssl::nid::Nid::COMMONNAME)
            .next()
            .and_then(|e| e.data().to_string().ok())
            .unwrap_or_default();
        Ok(Self { root_der, generation_der, generation_name, revoked })
    }

    /// From PEM files as `ONESIMUS_PLUGIN_CA` holds them: `root.pem`,
    /// `generation.pem` (one certificate each) and `revoked.json`.
    pub fn from_pem(root_pem: &[u8], generation_pem: &[u8], revoked_json: &[u8]) -> Result<Self, String> {
        let one = |pem: &[u8], what: &str| -> Result<Vec<u8>, String> {
            if pem.windows(b"PRIVATE KEY".len()).any(|w| w == b"PRIVATE KEY") {
                return Err(format!("{what}: private keys do not belong into the trust base"));
            }
            let certs = X509::stack_from_pem(pem).map_err(|_| format!("{what}: invalid PEM"))?;
            match certs.as_slice() {
                [cert] => cert.to_der().map_err(|_| format!("{what}: invalid certificate")),
                _ => Err(format!("{what}: exactly one certificate expected")),
            }
        };
        let revoked: Revocations =
            serde_json::from_slice(revoked_json).map_err(|e| format!("revoked.json: {e}"))?;
        Self::new(one(root_pem, "root.pem")?, one(generation_pem, "generation.pem")?, revoked)
    }

    pub fn root_der(&self) -> &[u8] {
        &self.root_der
    }
    pub fn generation_der(&self) -> &[u8] {
        &self.generation_der
    }
    /// Common name of the generation, e.g. "Onesimus Plugin Signing G1".
    pub fn generation_name(&self) -> &str {
        &self.generation_name
    }
    pub fn revoked(&self) -> &Revocations {
        &self.revoked
    }
    /// Whether `der` is one of the two compiled-in certificates.
    pub fn contains(&self, der: &[u8]) -> bool {
        der == self.root_der || der == self.generation_der
    }
}

/// The local `Plugin Policy`: restrictions only. `None` means "no further
/// restriction"; it never widens what the certificate grants.
#[derive(Debug, Clone, Default)]
pub struct LocalPolicy {
    /// Only these plugin IDs may load.
    pub plugin_ids: Option<BTreeSet<String>>,
    /// Upper bound for capabilities, in addition to the certificate's grants.
    pub allowed_capabilities: Option<BTreeSet<String>>,
    /// Revocations in addition to the compiled-in list.
    pub revoked: Revocations,
    /// Validity of this local snapshot (from inclusive, until exclusive).
    pub validity: Option<(u64, u64)>,
}

/// Everything one verification needs: the compiled-in base, the local
/// restrictions and the suite version of the running `ofd`.
#[derive(Debug, Clone)]
pub struct TrustPolicy {
    pub base: TrustBase,
    pub local: LocalPolicy,
    /// Must equal the manifest's `suite` exactly.
    pub suite: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_is_the_onesimus_part_only() {
        assert_eq!(suite_of("25.1.2+onesimus.0.2.0"), Some("0.2.0"));
        assert_eq!(suite_of("26.0.0+onesimus.0.2.0"), Some("0.2.0"));
        assert_eq!(suite_of("0.2.0"), None);
        assert_eq!(suite_of("25.1.2+onesimus."), None);
        assert_eq!(suite_of("25.1.2+onesimus.0.2 0"), None);
    }

    #[test]
    fn oid_constant_matches_openssl() {
        let oid = openssl::asn1::Asn1Object::from_str(GRANTS_OID).unwrap();
        assert_eq!(oid.as_slice(), GRANTS_OID_DER);
    }

    #[test]
    fn grants_roundtrip_and_strict_parsing() {
        let grants = Grants {
            plugin_ids: ["io.onesimus.postgresql".into()].into(),
            capabilities: ["file.read".into(), "postgresql.read".into()].into(),
        };
        let der = grants.to_der();
        assert_eq!(Grants::from_der(&der).unwrap(), grants);
        let mut trailing = der.clone();
        trailing.push(0);
        assert!(Grants::from_der(&trailing).is_err());
        assert!(Grants::from_der(&der[..der.len() - 1]).is_err());
        assert!(Grants::from_der(&Grants { plugin_ids: Default::default(), ..grants.clone() }.to_der()).is_err());
        let upper = Grants { plugin_ids: ["Org.X".into()].into(), ..Default::default() };
        assert!(Grants::from_der(&upper.to_der()).is_err());
        // Long lists use the long length form.
        let many = Grants {
            plugin_ids: (0..40).map(|i| format!("org.example.plugin{i}")).collect(),
            capabilities: Default::default(),
        };
        assert_eq!(Grants::from_der(&many.to_der()).unwrap(), many);
    }

    #[test]
    fn revocations_must_be_fingerprints() {
        let mut r = Revocations::default();
        r.revoked_keys.insert("ab".repeat(32));
        assert!(r.check().is_ok());
        r.revoked_keys.insert("AB".repeat(32));
        assert!(r.check().is_err());
    }
}
