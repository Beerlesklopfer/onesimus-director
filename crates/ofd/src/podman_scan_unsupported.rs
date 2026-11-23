// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! The host scan table of the Podman plugin on platforms without it
//! (Windows).
//!
//! The Podman plugin is Linux-only (`openat2`, user namespaces, `/proc`,
//! `podman_scan.rs`). The plugin session keeps a slot for its scan table;
//! here the table type has no values, so the slot is always `None` and a
//! `FileEntry` with `source_ref` is refused ("scan reference without a
//! host scan").

use std::sync::{Arc, Mutex};

/// The scan table of one plugin instance; there is none on this platform.
pub enum ScanTable {}

/// The shared scan table of a session.
pub type SharedScans = Arc<Mutex<ScanTable>>;

/// A taken scan entry; there is none on this platform.
pub enum Taken {}

impl ScanTable {
    /// Never called: no table exists.
    pub fn take(&mut self, _source_ref: &[u8], _path: &[u8], _kind: i32) -> Result<Taken, String> {
        match *self {}
    }

    /// Never called: no table exists.
    pub fn mark_saved(&mut self, _path: Vec<u8>, _file_index: i32) {
        match *self {}
    }

    /// Never called: no table exists.
    pub fn saved_index(&self, _path: &[u8]) -> Option<i32> {
        match *self {}
    }
}
