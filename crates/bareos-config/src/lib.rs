// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Bareos configuration file parser.
//!
//! Parses the Bareos Director configuration syntax into typed resource structs,
//! and the Storage Daemon configuration (`bareos-sd.d`, [`sd`]).
//! Supports all major resource types and directive value formats.

pub mod resources;
pub mod jobdefs;
pub mod parser;
pub mod raw;
pub mod edit;
pub mod values;
pub mod secrets;
pub mod sd;
pub mod onesimus;

pub use resources::*;
pub use parser::parse_config;
