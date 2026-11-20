//! TLS des gRPC-Servers eines Daemons.
//!
//! Zweck: Der gRPC-Zugang (ofd heute, osd ab P1) läuft immer über TLS.
//! Ohne eigenes Zertifikat erzeugt der Daemon beim ersten Start ein
//! selbstsigniertes ([`load_or_generate`]) und legt es im Arbeitsverzeichnis
//! ab; der Fingerabdruck bleibt so über Neustarts gleich, und Clients können
//! ihn festhalten (trust on first use). [`server_tls_config`] macht daraus
//! die TLS-Einstellung für `tonic`.
//!
//! Datenfluss: `main` → [`load_or_generate`] (oder `--grpc-cert/--grpc-key`)
//! → [`server_tls_config`] → `tonic::transport::Server::tls_config`.
//!
//! Bareos-Gegenstück: keins (Onesimus-Erweiterung).
//!
//! Herkunft: `ofd/src/grpc_tls.rs` (`load_or_generate`) und der Anfang von
//! `ofd/src/grpc.rs` `start` (Identity aus PEM-Dateien), am 20.11.2026
//! ohne Verhaltensänderung herausgelöst. Die Fassung des Directors
//! (`odir/src/grpc/tls.rs`, mit mDNS-Namen und Fingerabdruck) bleibt in odir
//! und folgt in P1.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use tonic::transport::{Identity, ServerTlsConfig};

/// The certificate and key files `<dir>/<base>.crt` and `<dir>/<base>.key`;
/// generated (self-signed for `name`, `localhost` and the host name, key
/// mode 0600) unless both exist.
pub fn load_or_generate(working_directory: &str, base: &str, name: &str) -> Result<(PathBuf, PathBuf), String> {
    let cert = PathBuf::from(working_directory).join(format!("{base}.crt"));
    let key = PathBuf::from(working_directory).join(format!("{base}.key"));
    if cert.exists() && key.exists() {
        return Ok((cert, key));
    }
    let mut names = vec![name.to_string(), "localhost".to_string()];
    if let Ok(host) = nix::unistd::gethostname() {
        names.push(host.to_string_lossy().into_owned());
    }
    names.sort();
    names.dedup();
    let certified = rcgen::generate_simple_self_signed(names).map_err(|e| e.to_string())?;
    let mut key_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&key)
        .map_err(|e| format!("{}: {e}", key.display()))?;
    key_file.write_all(certified.key_pair.serialize_pem().as_bytes()).map_err(|e| e.to_string())?;
    std::fs::write(&cert, certified.cert.pem()).map_err(|e| format!("{}: {e}", cert.display()))?;
    tracing::info!("generated gRPC TLS certificate {}", cert.display());
    Ok((cert, key))
}

/// The server TLS configuration of a PEM certificate and key file.
pub fn server_tls_config(cert: &Path, key: &Path) -> Result<ServerTlsConfig, String> {
    let identity = Identity::from_pem(
        std::fs::read(cert).map_err(|e| e.to_string())?,
        std::fs::read(key).map_err(|e| e.to_string())?,
    );
    Ok(ServerTlsConfig::new().identity(identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_once_then_reuses() {
        let dir = std::env::temp_dir().join(format!("onesimus-daemon-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let (cert, key) = load_or_generate(d, "onesimus-test-grpc", "test-fd").unwrap();
        assert_eq!(cert, dir.join("onesimus-test-grpc.crt"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
        let pem = std::fs::read(&cert).unwrap();
        assert_eq!(load_or_generate(d, "onesimus-test-grpc", "test-fd").unwrap(), (cert.clone(), key.clone()));
        assert_eq!(std::fs::read(&cert).unwrap(), pem);
        assert!(server_tls_config(&cert, &key).is_ok());
        assert!(server_tls_config(&dir.join("missing"), &key).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
