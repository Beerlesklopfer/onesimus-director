//! Positive lists for Podman answers (docs/plans/PODMAN-PLUGIN.md,
//! "Umgebungsvariablen", V-6).
//!
//! # Purpose
//!
//! The host takes from the answers of the broker or the rootful backend
//! **only the fields of a positive list**, instead of filtering values out
//! of the full answer. Unknown fields fall away; a new Podman version
//! cannot leak a field nobody reviewed. The lists belong to the certified
//! version profile (Podman 5.4.2).
//!
//! # Data flow
//!
//! raw JSON (broker) → [`filter`] with a list ([`ENGINE_INFO`],
//! [`ENGINE_VERSION`], [`INVENTORY_CONTAINERS`], …) → JSON the guest sees.
//!
//! Paths are dot-separated object keys; arrays are transparent (the rest of
//! the path applies to every element). A path ending at an object keeps the
//! whole subtree, so lists name leaves wherever values could be secret.
//!
//! # Counterpart
//!
//! The P1 inspect and spec lists (with env handling) build on [`filter`].

use serde_json::{Map, Value};

/// `podman info`: identity of the engine and the storage, no host load,
/// no paths of helper programs.
pub const ENGINE_INFO: &[&str] = &[
    "host.arch",
    "host.cgroupManager",
    "host.cgroupVersion",
    "host.databaseBackend",
    "host.networkBackend",
    "host.os",
    "host.security.rootless",
    "host.security.selinuxEnabled",
    "host.idMappings.uidmap.container_id",
    "host.idMappings.uidmap.host_id",
    "host.idMappings.uidmap.size",
    "host.idMappings.gidmap.container_id",
    "host.idMappings.gidmap.host_id",
    "host.idMappings.gidmap.size",
    "store.graphDriverName",
    "store.graphRoot",
    "store.runRoot",
    "store.volumePath",
    "store.transientStore",
    "version.Version",
    "version.APIVersion",
    "version.OsArch",
];

/// `podman version`.
pub const ENGINE_VERSION: &[&str] = &["Client.Version", "Client.APIVersion", "Client.OsArch"];

/// `podman ps -a`: identity and state, no command lines, no environment.
pub const INVENTORY_CONTAINERS: &[&str] =
    &["Id", "Names", "Image", "ImageID", "State", "Pod", "PodName", "IsInfra", "Created", "Networks"];

/// `podman pod ps`.
pub const INVENTORY_PODS: &[&str] = &["Id", "Name", "Status", "InfraId", "Containers.Id", "Containers.Names"];

/// `podman network ls`.
pub const INVENTORY_NETWORKS: &[&str] = &["name", "id", "driver", "internal"];

/// `podman volume ls`.
pub const INVENTORY_VOLUMES: &[&str] = &["Name", "Driver", "Scope"];

/// `podman images`.
pub const INVENTORY_IMAGES: &[&str] = &["Id", "Names", "Digest", "Size", "Created"];

/// `podman secret ls`: metadata only, never the secret.
pub const INVENTORY_SECRETS: &[&str] = &["ID", "Spec.Name", "Spec.Driver.Name"];

/// A tree of allowed keys.
#[derive(Debug, Default)]
struct Node {
    /// Keep everything below (the path ended here).
    all: bool,
    /// Allowed child keys.
    children: std::collections::BTreeMap<String, Node>,
}

/// Builds the key tree of a list.
fn tree(allowed: &[&str]) -> Node {
    let mut root = Node::default();
    for path in allowed {
        let mut n = &mut root;
        for key in path.split('.') {
            n = n.children.entry(key.to_string()).or_default();
        }
        n.all = true;
    }
    root
}

/// Applies a key tree; arrays are transparent.
fn apply(value: &Value, node: &Node) -> Option<Value> {
    if node.all {
        return Some(value.clone());
    }
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, child) in &node.children {
                if let Some(v) = map.get(k).and_then(|v| apply(v, child)) {
                    out.insert(k.clone(), v);
                }
            }
            Some(Value::Object(out))
        }
        Value::Array(items) => Some(Value::Array(items.iter().filter_map(|v| apply(v, node)).collect())),
        // A scalar where the list expects an object: not allowed.
        _ => None,
    }
}

/// The parts of `value` the list allows.
pub fn filter(value: &Value, allowed: &[&str]) -> Value {
    apply(value, &tree(allowed)).unwrap_or(Value::Null)
}

/// Parses `bytes` (at most `limit`) and filters it.
pub fn filter_bytes(bytes: &[u8], allowed: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    if bytes.len() > limit {
        return Err("answer exceeds its size limit".into());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "answer is not JSON".to_string())?;
    serde_json::to_vec(&filter(&value, allowed)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_listed_fields_survive() {
        let info = json!({
            "host": {"arch": "amd64", "hostname": "secret-host", "security": {"rootless": true, "capabilities": "CAP_X"},
                     "idMappings": {"uidmap": [{"container_id": 0, "host_id": 1000, "size": 1, "extra": 1}]}},
            "store": {"graphRoot": "/g", "configFile": "/home/x/.config/containers/storage.conf"},
            "registries": {"search": ["docker.io"]},
            "version": {"Version": "5.4.2", "GitCommit": ""}
        });
        let out = filter(&info, ENGINE_INFO);
        assert_eq!(out, json!({
            "host": {"arch": "amd64", "security": {"rootless": true},
                     "idMappings": {"uidmap": [{"container_id": 0, "host_id": 1000, "size": 1}]}},
            "store": {"graphRoot": "/g"},
            "version": {"Version": "5.4.2"}
        }));
    }

    #[test]
    fn unknown_fields_of_list_elements_fall_away() {
        let ps = json!([{"Id": "a", "Names": ["web"], "Command": ["sh", "-c", "PASSWORD=x"], "Labels": {"k": "v"}}]);
        assert_eq!(filter(&ps, INVENTORY_CONTAINERS), json!([{"Id": "a", "Names": ["web"]}]));
        let secrets = json!([{"ID": "s", "Spec": {"Name": "db", "Driver": {"Name": "file", "Options": {"path": "/x"}}}, "SecretData": "x"}]);
        assert_eq!(filter(&secrets, INVENTORY_SECRETS), json!([{"ID": "s", "Spec": {"Name": "db", "Driver": {"Name": "file"}}}]));
    }

    #[test]
    fn size_and_syntax_are_checked() {
        assert!(filter_bytes(b"{}", ENGINE_INFO, 1).is_err());
        assert!(filter_bytes(b"nope", ENGINE_INFO, 100).is_err());
        assert_eq!(filter_bytes(br#"{"host":{"arch":"x","cpus":3}}"#, ENGINE_INFO, 100).unwrap(), br#"{"host":{"arch":"x"}}"#);
    }
}
