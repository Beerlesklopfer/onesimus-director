//! The `ofd.d/` overlay of the Onesimus configuration directory (`--overlay`,
//! default `/etc/onesimus`): Onesimus directives of the Client resource that
//! Bareos does not know. `bareos-fd.d` stays what `bareos-fd` reads.
//!
//! `ofd.d/client/<Name>.conf`:
//!
//! ```text
//! Client {
//!   Name = bareos-fd
//!   Grpc Enable = yes          # default yes; no: BNET only (like --disable-grpc)
//!   Grpc Port = 19102          # default: FD Port + 1000
//!   Grpc Address = 127.0.0.1   # default: the FD Address
//! }
//! ```
//!
//! The command line (`--grpc-listen`, `--disable-grpc`) wins over the overlay.

use std::net::IpAddr;
use std::path::Path;

/// The gRPC settings of the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrpcSettings {
    /// `Grpc Enable` (default yes).
    pub enable: bool,
    /// `Grpc Port`.
    pub port: Option<u16>,
    /// `Grpc Address`.
    pub address: Option<IpAddr>,
}

impl Default for GrpcSettings {
    fn default() -> Self {
        GrpcSettings { enable: true, port: None, address: None }
    }
}

/// Reads the overlay tree `dir` for the Client `client_name`. A missing tree is the defaults;
/// another resource type, another client name or another directive is an error.
pub fn load(dir: &Path, client_name: &str) -> Result<GrpcSettings, String> {
    let mut settings = GrpcSettings::default();
    if !dir.is_dir() {
        return Ok(settings);
    }
    for res in bareos_config::raw::load_tree(dir, "ofd.d").map_err(|e| format!("ofd.d: {e}"))? {
        let name = res.name().unwrap_or_default();
        if res.norm_type() != "client" {
            return Err(format!("ofd.d: resource type \"{}\" is not supported (only Client)", res.rtype));
        }
        if !name.eq_ignore_ascii_case(client_name) {
            return Err(format!("ofd.d: Client \"{name}\" extends no Client of that name (this File Daemon is \"{client_name}\")"));
        }
        for d in &res.directives {
            let value = d.values().join(" ");
            match d.norm_key().as_str() {
                "name" => {}
                "grpcenable" => {
                    settings.enable = bareos_config::values::parse_bool(&value).map(|(_, b)| b).map_err(|_| format!("ofd.d: Grpc Enable: \"{value}\" is not yes or no"))?;
                }
                "grpcport" => settings.port = Some(value.parse().map_err(|_| format!("ofd.d: Grpc Port: \"{value}\" is not a port"))?),
                "grpcaddress" => settings.address = Some(value.parse().map_err(|_| format!("ofd.d: Grpc Address: \"{value}\" is not an IP address"))?),
                _ => return Err(format!("ofd.d: Client \"{name}\": \"{}\" is not an Onesimus directive of the File Daemon", d.key)),
            }
        }
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A overlay tree with one client file.
    fn tree(tag: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ofd-overlay-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("client")).unwrap();
        std::fs::write(dir.join("client/bareos-fd.conf"), text).unwrap();
        dir
    }

    #[test]
    fn no_tree_is_the_defaults() {
        assert_eq!(load(Path::new("/nonexistent/ofd.d"), "bareos-fd").unwrap(), GrpcSettings::default());
    }

    #[test]
    fn the_client_file_sets_the_grpc_directives() {
        let dir = tree("ok", "Client {\n Name = bareos-fd\n Grpc Enable = no\n Grpc Port = 19102\n Grpc Address = 127.0.0.1\n}\n");
        let s = load(&dir, "bareos-fd").unwrap();
        assert_eq!((s.enable, s.port, s.address), (false, Some(19102), Some("127.0.0.1".parse().unwrap())));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn other_names_types_and_directives_are_refused() {
        let dir = tree("name", "Client {\n Name = other-fd\n Grpc Port = 1\n}\n");
        assert!(load(&dir, "bareos-fd").unwrap_err().contains("extends no Client of that name"));
        let dir = tree("dir", "Client {\n Name = bareos-fd\n Password = x\n}\n");
        assert!(load(&dir, "bareos-fd").unwrap_err().contains("not an Onesimus directive"));
        let dir = tree("type", "Director {\n Name = bareos-fd\n}\n");
        assert!(load(&dir, "bareos-fd").unwrap_err().contains("only Client"));
        let dir = tree("port", "Client {\n Name = bareos-fd\n Grpc Port = many\n}\n");
        assert!(load(&dir, "bareos-fd").unwrap_err().contains("not a port"));
    }
}
