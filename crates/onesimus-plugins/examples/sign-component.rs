// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Build-system tool for plugin bundles. It never sees a private key: the
//! manifest is signed in between by OpenSSL (TPM provider in production).
//!
//! ```text
//! sign-component prepare  --component plugin.wasm --cert signer.pem --plugin-id ID --version V
//!                         [--capability CAP]... [--native TRIPLET:RELPATH=FILE]... [--suite S]
//!                         [--not-before UNIX] [--not-after UNIX] --out manifest.json
//! openssl dgst -sha256 -sign handle:0x81000010 -provider tpm2 -provider default \
//!         -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest -out manifest.sig manifest.json
//! sign-component assemble --component plugin.wasm --manifest manifest.json --signature manifest.sig
//!                         --chain chain.pem [--trust-dir DIR] --out plugin.pb
//! ```
//! `--suite` defaults to the suite of this workspace (Onesimus part of its
//! version). `chain.pem`: the signing certificate, optionally followed by the
//! generation certificate. `--trust-dir`: the directory given to the `ofd`
//! build as ONESIMUS_PLUGIN_CA; then `assemble` also runs the full check.
use onesimus_plugins::{
    NativePart,
    bundle::{read_bounded, read_trust_dir},
    signing::{self, PrepareRequest},
};
use openssl::x509::X509;
use prost::Message;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

type Error = Box<dyn std::error::Error>;
const MAX_COMPONENT: usize = 16 * 1024 * 1024;

const USAGE: &str = "usage:
  sign-component prepare --component plugin.wasm --cert signer.pem --plugin-id ID --version V
                         [--capability CAP]... [--native TRIPLET:RELPATH=FILE]... [--suite S]
                         [--not-before UNIX] [--not-after UNIX] --out manifest.json
  sign-component assemble --component plugin.wasm --manifest manifest.json --signature manifest.sig
                          --chain chain.pem [--trust-dir DIR] --out plugin.pb";

/// `--name value` pairs; repeatable names collect several values.
fn options(args: &[String], allowed: &[&str]) -> Result<HashMap<String, Vec<String>>, Error> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    let mut it = args.iter();
    while let Some(name) = it.next() {
        let key = name.strip_prefix("--").filter(|k| allowed.contains(k)).ok_or_else(|| format!("unknown option {name}\n{USAGE}"))?;
        let value = it.next().ok_or_else(|| format!("{name} needs a value"))?;
        out.entry(key.to_string()).or_default().push(value.clone());
    }
    Ok(out)
}

fn one<'a>(opts: &'a HashMap<String, Vec<String>>, name: &str) -> Result<Option<&'a str>, Error> {
    match opts.get(name).map(Vec::as_slice) {
        None => Ok(None),
        Some([v]) => Ok(Some(v.as_str())),
        Some(_) => Err(format!("--{name} given more than once").into()),
    }
}
fn required<'a>(opts: &'a HashMap<String, Vec<String>>, name: &str) -> Result<&'a str, Error> {
    one(opts, name)?.ok_or_else(|| format!("--{name} is required\n{USAGE}").into())
}

fn chain_der(path: &Path) -> Result<Vec<Vec<u8>>, Error> {
    let pem = read_bounded(path, 512 * 1024)?;
    if pem.windows(b"PRIVATE KEY".len()).any(|w| w == b"PRIVATE KEY") {
        return Err("certificate files must not contain private keys".into());
    }
    let certs = X509::stack_from_pem(&pem)?;
    Ok(certs.iter().map(|c| c.to_der()).collect::<Result<_, _>>()?)
}

/// Writes next to the target and renames, so no half bundle is left.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn prepare(args: &[String]) -> Result<(), Error> {
    let o = options(args, &["component", "cert", "plugin-id", "version", "capability", "native", "suite", "not-before",
        "not-after", "out"])?;
    let component = read_bounded(Path::new(required(&o, "component")?), MAX_COMPONENT)?;
    let chain = chain_der(Path::new(required(&o, "cert")?))?;
    let leaf = chain.first().ok_or("--cert holds no certificate")?;
    let mut native: BTreeMap<String, Vec<NativePart>> = BTreeMap::new();
    for spec in o.get("native").into_iter().flatten() {
        let (triplet, rest) = spec.split_once(':').ok_or("--native TRIPLET:RELPATH=FILE")?;
        let (path, file) = rest.split_once('=').ok_or("--native TRIPLET:RELPATH=FILE")?;
        let bytes = read_bounded(Path::new(file), 256 * 1024 * 1024)?;
        native.entry(triplet.into()).or_default().push(NativePart { path: path.into(), sha256: hex::encode(Sha256::digest(&bytes)) });
    }
    let default_suite = onesimus_plugins::suite_of(env!("CARGO_PKG_VERSION")).ok_or("workspace version has no suite")?;
    let number = |name: &str| -> Result<Option<u64>, Error> { Ok(one(&o, name)?.map(str::parse).transpose()?) };
    let request = PrepareRequest {
        plugin_id: required(&o, "plugin-id")?.into(),
        version: required(&o, "version")?.into(),
        capabilities: o.get("capability").cloned().unwrap_or_default(),
        native,
        suite: one(&o, "suite")?.unwrap_or(default_suite).into(),
        not_before: number("not-before")?,
        not_after: number("not-after")?,
    };
    let manifest = signing::prepare(&component, leaf, &request)?;
    write_atomic(Path::new(required(&o, "out")?), &manifest)?;
    eprintln!("manifest for {} {} (suite {}) written; sign these exact bytes", request.plugin_id, request.version, request.suite);
    Ok(())
}

fn assemble(args: &[String]) -> Result<(), Error> {
    let o = options(args, &["component", "manifest", "signature", "chain", "trust-dir", "out"])?;
    let component = read_bounded(Path::new(required(&o, "component")?), MAX_COMPONENT)?;
    let manifest = read_bounded(Path::new(required(&o, "manifest")?), 64 * 1024)?;
    let signature = read_bounded(Path::new(required(&o, "signature")?), 4096)?;
    let chain = chain_der(Path::new(required(&o, "chain")?))?;
    let trust = one(&o, "trust-dir")?.map(|d| read_trust_dir(Path::new(d))).transpose()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let bundle = signing::assemble(component, manifest, signature, chain, trust.as_ref(), now)?;
    write_atomic(Path::new(required(&o, "out")?), &bundle.encode_to_vec())?;
    eprintln!("bundle written; signature verified{}", if trust.is_some() { ", trust check passed" } else { "" });
    Ok(())
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.split_first() {
        Some((command, rest)) if command == "prepare" => prepare(rest),
        Some((command, rest)) if command == "assemble" => assemble(rest),
        _ => Err(USAGE.into()),
    }
}
