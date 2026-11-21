//! Persistent bearer credential for the HTTP MCP endpoint.
use std::fs::{DirBuilder, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rand::RngCore;

pub fn default_path() -> anyhow::Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(path) => PathBuf::from(path),
        // Windows without HOME: %APPDATA%\oconsole\mcp.token
        None if cfg!(windows) && std::env::var_os("HOME").is_none() => PathBuf::from(
            std::env::var_os("APPDATA").context("APPDATA is unset; supply --mcp-token-file")?,
        ),
        None => PathBuf::from(
            std::env::var_os("HOME").context("HOME is unset; supply --mcp-token-file")?,
        )
        .join(".config"),
    };
    Ok(base.join("oconsole/mcp.token"))
}

pub fn load_or_create(path: &Path) -> anyhow::Result<String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut builder = DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        builder.mode(0o700);
        builder.create(parent)?;
    }
    // Windows: no mode; the file inherits the ACL of the user's profile
    // directory (only the user, SYSTEM and Administrators).
    let mut create = OpenOptions::new();
    create.write(true).create_new(true);
    #[cfg(unix)]
    create.mode(0o600);
    match create.open(path) {
        Ok(mut file) => {
            let mut bytes = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut bytes);
            let token = hex::encode(bytes);
            writeln!(file, "{token}")?;
            file.sync_all()?;
            Ok(token)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            #[cfg(unix)]
            let mut file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)?;
            #[cfg(unix)]
            {
                let metadata = file.metadata()?;
                if metadata.len() > 128
                    || !metadata.is_file()
                    || metadata.mode() & 0o077 != 0
                    || metadata.uid() != unsafe { libc::geteuid() }
                {
                    bail!("MCP token file must be a regular file owned by this user with mode 0600");
                }
            }
            // Windows: no owner/mode check (open point: owner SID and ACL);
            // a link or anything but a small regular file is refused.
            #[cfg(windows)]
            let mut file = {
                if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
                    bail!("MCP token file must be a regular file, not a link");
                }
                let file = OpenOptions::new().read(true).open(path)?;
                let metadata = file.metadata()?;
                if metadata.len() > 128 || !metadata.is_file() {
                    bail!("MCP token file must be a regular file");
                }
                file
            };
            let mut token = String::new();
            Read::by_ref(&mut file)
                .take(128)
                .read_to_string(&mut token)?;
            let token = token.trim();
            if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
                bail!("MCP token file must contain exactly 64 hexadecimal characters");
            }
            Ok(token.to_string())
        }
        Err(e) => Err(anyhow::Error::from(e)),
    }
    .with_context(|| format!("MCP token file {}", path.display()))
}

pub fn authorized(header: Option<&str>, token: &str) -> bool {
    let Some(value) = header.and_then(|s| s.strip_prefix("Bearer ")) else {
        return false;
    };
    value.len() == token.len()
        && value
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn token_is_persistent_private_and_rejects_insecure_files() {
        let dir = std::env::temp_dir().join(format!(
            "oconsole-token-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let path = dir.join("mcp.token");
        let token = load_or_create(&path).unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(load_or_create(&path).unwrap(), token);
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert!(authorized(Some(&format!("Bearer {token}")), &token));
        assert!(!authorized(None, &token));
        assert!(!authorized(Some("Bearer wrong"), &token));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/dev/null", &path).unwrap();
        assert!(load_or_create(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
