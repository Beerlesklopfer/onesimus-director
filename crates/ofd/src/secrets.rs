//! Plugin option masking, shared with odir: see bareos_config::secrets.

pub use bareos_config::secrets::{is_secret, mask_plugin_secrets, MASK};
