//! Bareos configuration file parser.
//!
//! Parses the Bareos Director configuration syntax into typed resource structs.
//! Supports all major resource types and directive value formats.

pub mod resources;
pub mod jobdefs;
pub mod parser;
pub mod raw;
pub mod values;

pub use resources::*;
pub use parser::parse_config;
