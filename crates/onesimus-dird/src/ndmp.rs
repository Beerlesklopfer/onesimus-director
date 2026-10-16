//! NDMP branches of the console commands (compatibility log C-050).
//!
//! Storage and Client resources with `Protocol = NDMPv2|NDMPv3|NDMPv4` take
//! the NDMP branches of upstream (dird/ndmp_dma_storage.cc,
//! dird/ndmp_dma_generic.cc). Onesimus has no NDMP session layer: every
//! step before a session is opened behaves like upstream (silent skips,
//! paired storages, `fill_ndmp_agent_config()` checks, autochanger checks);
//! where upstream would open an NDMP session, Onesimus reports
//! [`NOT_SUPPORTED`] — the text of an upstream Director built without
//! NDMP — on the console instead.

use bareos_config::resources::{AuthProtocol, AuthType, ClientResource, StorageResource};

/// Upstream's message for NDMP entry points without NDMP support
/// (`#ifndef HAVE_NDMP`, ndmp_dma_storage.cc:1080 ff.).
pub const NOT_SUPPORTED: &str = "NDMP protocol not supported\n";

/// `fill_ndmp_agent_config()` (ndmp_dma_generic.cc:197): the checks before
/// any connection. `Err` carries the Jmsg(M_FATAL) text, which upstream
/// sends to the console JCR's message destinations, not to the console.
pub fn agent_config_check(protocol: AuthProtocol, _auth: AuthType, username: Option<&str>, port: u16)
    -> Result<(), String> {
    if !protocol.is_ndmp() {
        // Illegal protocol %d: APT_NATIVE is 0
        return Err("Illegal protocol 0 for NDMP Job\n".to_string());
    }
    // Address and Password are required directives, AuthType is a keyword.
    if username.is_none() {
        return Err("fill_ndmp_agent_config: username is missing\n".to_string());
    }
    if port == 0 {
        return Err("fill_ndmp_agent_config: port is missing\n".to_string());
    }
    Ok(())
}

pub fn storage_agent_check(store: &StorageResource) -> Result<(), String> {
    agent_config_check(store.protocol, store.auth_type, store.username.as_deref(), store.port)
}

pub fn client_agent_check(client: &ClientResource) -> Result<(), String> {
    agent_config_check(client.protocol, client.auth_type, client.username.as_deref(), client.port)
}

/// Jmsg() of the console JCR: upstream routes it to the Messages resource,
/// Onesimus logs it.
pub fn console_jmsg(text: &str) {
    tracing::warn!(target: "ndmp", "{}", text.trim_end());
}

/// `DoNdmpClientStatus()` (ndmp_dma_generic.cc:647): the DATA agent query.
/// Without a complete agent configuration upstream returns after the
/// Jmsg; otherwise it opens an NDMP session (not supported here).
pub fn client_status(client: &ClientResource) -> String {
    match client_agent_check(client) {
        Err(text) => {
            console_jmsg(&text);
            String::new()
        }
        Ok(()) => NOT_SUPPORTED.to_string(),
    }
}

/// `DoNdmpNativeStorageStatus()` (ndmp_dma_storage.cc:179) of a storage
/// without PairedStorage. With an incomplete agent configuration
/// NdmpBuildStorageJob() fails ("build_storage_job failed") and upstream
/// still runs NdmpDoQuery() with the half-filled job: fill_ndmp_agent_config()
/// stops before it copies the host, so the NDMP library's connect fails
/// at once and logs "err bad-host-name" to the console (observed with
/// bareos-dir 18be77e6, parity case c050-status-storage-native). A complete
/// configuration would query the NDMP server (not supported here).
pub fn native_storage_status(store: &StorageResource) -> String {
    match storage_agent_check(store) {
        Err(text) => {
            console_jmsg(&text);
            "build_storage_job failed\nerr bad-host-name\n".to_string()
        }
        Ok(()) => NOT_SUPPORTED.to_string(),
    }
}

/// `GetRobotElementStatus()` (ndmp_dma_storage.cc:328) as used by
/// NdmpGetNumSlots()/NdmpGetNumDrives()/ndmp_get_vol_list(): Some(message)
/// when upstream would open an NDMP session, None when it returns before
/// (no autochanger or no NdmpChangerDevice, or an incomplete agent
/// configuration).
pub fn robot_session(store: &StorageResource, autochanger: bool) -> Option<&'static str> {
    if !autochanger || store.ndmp_changer_device.is_none() {
        return None;
    }
    if let Err(text) = storage_agent_check(store) {
        console_jmsg(&text);
        return None;
    }
    Some(NOT_SUPPORTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(username: Option<&str>, changer: Option<&str>) -> StorageResource {
        StorageResource {
            name: "n".into(), protocol: AuthProtocol::NdmpV4,
            username: username.map(str::to_string), ndmp_changer_device: changer.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn agent_checks_stop_before_any_session() {
        assert_eq!(native_storage_status(&store(None, None)), "build_storage_job failed\nerr bad-host-name\n");
        assert_eq!(native_storage_status(&store(Some("u"), None)), NOT_SUPPORTED);
        let client = ClientResource { protocol: AuthProtocol::NdmpV2, ..Default::default() };
        assert_eq!(client_status(&client), "");
        let client = ClientResource { username: Some("u".into()), ..client };
        assert_eq!(client_status(&client), NOT_SUPPORTED);
    }

    #[test]
    fn robot_session_only_for_ndmp_changers() {
        assert_eq!(robot_session(&store(Some("u"), Some("robot")), false), None);
        assert_eq!(robot_session(&store(Some("u"), None), true), None);
        assert_eq!(robot_session(&store(None, Some("robot")), true), None);
        assert_eq!(robot_session(&store(Some("u"), Some("robot")), true), Some(NOT_SUPPORTED));
    }
}
