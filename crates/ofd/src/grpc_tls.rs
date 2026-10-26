//! TLS identity of the FD's gRPC listener: a self-signed certificate generated
//! once in the working directory, so its fingerprint stays stable for pinning.
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

const CERT_FILE: &str = "onesimus-fd-grpc.crt";
const KEY_FILE: &str = "onesimus-fd-grpc.key";

pub fn load_or_generate(working_directory: &str, fd_name: &str) -> Result<(PathBuf, PathBuf), String> {
    let cert = PathBuf::from(working_directory).join(CERT_FILE);
    let key = PathBuf::from(working_directory).join(KEY_FILE);
    if cert.exists() && key.exists() {
        return Ok((cert, key));
    }
    let mut names = vec![fd_name.to_string(), "localhost".to_string()];
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
    tracing::info!("generated FD gRPC TLS certificate {}", cert.display());
    Ok((cert, key))
}
