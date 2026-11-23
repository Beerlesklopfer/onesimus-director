// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Plugin option masking, shared with odir: see bareos_config::secrets.

pub use bareos_config::secrets::{is_secret, mask_plugin_secrets, MASK};
