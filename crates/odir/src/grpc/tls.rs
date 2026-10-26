//! TLS identity for the gRPC endpoint.
//!
//! Uses `Grpc Tls Certificate`/`Grpc Tls Key` from the Director resource if
//! set. Otherwise a self-signed certificate is generated once and kept in the
//! working directory, so its fingerprint stays stable and clients can pin it
//! (trust on first use). The SHA-256 fingerprint is also announced via mDNS.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tracing::{info, warn};

pub struct GrpcIdentity {
    pub cert_pem: String,
    pub key_pem: String,
    /// Lowercase hex SHA-256 of the DER certificate
    pub fingerprint: String,
}

const CERT_FILE: &str = "onesimus-grpc.crt";
const KEY_FILE: &str = "onesimus-grpc.key";

pub fn load_or_generate(
    cert: Option<&str>,
    key: Option<&str>,
    working_dir: &str,
    director_name: &str,
) -> anyhow::Result<GrpcIdentity> {
    if let (Some(cert), Some(key)) = (cert, key) {
        return load(Path::new(cert), Path::new(key));
    }

    let cert_path = PathBuf::from(working_dir).join(CERT_FILE);
    let key_path = PathBuf::from(working_dir).join(KEY_FILE);
    if cert_path.exists() && key_path.exists() {
        return load(&cert_path, &key_path);
    }

    let identity = generate(director_name)?;
    match write_private(&key_path, &identity.key_pem)
        .and_then(|_| std::fs::write(&cert_path, &identity.cert_pem))
    {
        Ok(()) => info!("generated gRPC TLS certificate {}", cert_path.display()),
        Err(e) => warn!(
            "cannot store gRPC TLS certificate in {working_dir}: {e} — using a temporary one, \
             clients will see a new fingerprint after every restart"
        ),
    }
    Ok(identity)
}

fn load(cert: &Path, key: &Path) -> anyhow::Result<GrpcIdentity> {
    let cert_pem = std::fs::read_to_string(cert)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", cert.display()))?;
    let key_pem = std::fs::read_to_string(key)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", key.display()))?;
    let fingerprint = fingerprint_of_pem(&cert_pem)?;
    Ok(GrpcIdentity { cert_pem, key_pem, fingerprint })
}

fn generate(director_name: &str) -> anyhow::Result<GrpcIdentity> {
    let mut names = vec![director_name.to_string(), "localhost".to_string()];
    if let Ok(host) = nix::unistd::gethostname() {
        names.push(host.to_string_lossy().into_owned());
    }
    if let Some(short) = crate::mdns::short_hostname() {
        names.push(format!("{short}.local"));
        names.push(short);
    }
    names.sort();
    names.dedup();

    let certified = rcgen::generate_simple_self_signed(names)?;
    let fingerprint = hex::encode(Sha256::digest(certified.cert.der()));
    Ok(GrpcIdentity {
        cert_pem: certified.cert.pem(),
        key_pem: certified.key_pair.serialize_pem(),
        fingerprint,
    })
}

fn fingerprint_of_pem(pem: &str) -> anyhow::Result<String> {
    let der = rustls_pemfile::certs(&mut pem.as_bytes())
        .next()
        .ok_or_else(|| anyhow::anyhow!("no certificate in PEM"))??;
    Ok(hex::encode(Sha256::digest(der.as_ref())))
}

fn write_private(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(content.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_fingerprint_matches_pem() {
        let id = generate("test-dir").unwrap();
        assert_eq!(id.fingerprint.len(), 64);
        assert_eq!(fingerprint_of_pem(&id.cert_pem).unwrap(), id.fingerprint);
    }

    #[test]
    fn persisted_identity_is_reused() {
        let dir = std::env::temp_dir().join(format!("onesimus-tls-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wd = dir.to_str().unwrap();
        let a = load_or_generate(None, None, wd, "test-dir").unwrap();
        let b = load_or_generate(None, None, wd, "test-dir").unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
