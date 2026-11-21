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
//! Windows: die Schlüsseldatei bekommt keinen Modus 0600, sondern erbt die
//! Zugriffsliste ihres Verzeichnisses; das Arbeitsverzeichnis legt der
//! Daemon dort nur für SYSTEM und Administratoren an (ofd `paths`). Der
//! Rechnername kommt aus `GetComputerNameExW` (DNS-Name).
//!
//! Bareos-Gegenstück: keins (Onesimus-Erweiterung).
//!
//! Herkunft: `ofd/src/grpc_tls.rs` (`load_or_generate`) und der Anfang von
//! `ofd/src/grpc.rs` `start` (Identity aus PEM-Dateien), am 20.11.2026
//! ohne Verhaltensänderung herausgelöst; am 21.11.2026 aus
//! `onesimus-daemon` (`grpc_tls`) hierher verschoben, damit die eigenen,
//! nicht von Bareos abgeleiteten Teile in einem Crate mit dualer Lizenz
//! liegen (docs/LIZENZEN.md). Die Fassung des Directors
//! (`odir/src/grpc/tls.rs`, mit mDNS-Namen und Fingerabdruck) bleibt in odir
//! und folgt in P1.

use std::io::Write;
#[cfg(unix)]
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
    if let Some(host) = host_name() {
        names.push(host);
    }
    names.sort();
    names.dedup();
    let certified = rcgen::generate_simple_self_signed(names).map_err(|e| e.to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut key_file = options
        .open(&key)
        .map_err(|e| format!("{}: {e}", key.display()))?;
    key_file.write_all(certified.key_pair.serialize_pem().as_bytes()).map_err(|e| e.to_string())?;
    std::fs::write(&cert, certified.cert.pem()).map_err(|e| format!("{}: {e}", cert.display()))?;
    tracing::info!("generated gRPC TLS certificate {}", cert.display());
    Ok((cert, key))
}

/// The host name (`gethostname`), `None` if it cannot be read.
#[cfg(unix)]
pub fn host_name() -> Option<String> {
    nix::unistd::gethostname().ok().map(|h| h.to_string_lossy().into_owned())
}

/// The DNS host name of the computer (`GetComputerNameExW` with
/// `ComputerNameDnsHostname`), `None` if it cannot be read.
#[cfg(windows)]
pub fn host_name() -> Option<String> {
    use windows_sys::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW};
    let mut buf = [0u16; 256];
    let mut len = buf.len() as u32;
    // SAFETY: `buf` holds `len` UTF-16 units; the call writes at most that many.
    if unsafe { GetComputerNameExW(ComputerNameDnsHostname, buf.as_mut_ptr(), &mut len) } == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..len as usize])).filter(|h| !h.is_empty())
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
        let dir = std::env::temp_dir().join(format!("onesimus-bnet-grpc-tls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_str().unwrap();
        let (cert, key) = load_or_generate(d, "onesimus-test-grpc", "test-fd").unwrap();
        assert_eq!(cert, dir.join("onesimus-test-grpc.crt"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let pem = std::fs::read(&cert).unwrap();
        assert_eq!(load_or_generate(d, "onesimus-test-grpc", "test-fd").unwrap(), (cert.clone(), key.clone()));
        assert_eq!(std::fs::read(&cert).unwrap(), pem);
        assert!(server_tls_config(&cert, &key).is_ok());
        assert!(server_tls_config(&dir.join("missing"), &key).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
