// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! `-u <user>` / `-g <group>`: drop root privileges (like C++ lib/bsys drop()).
//!
//! The implementation moved unchanged to `daemon_crate::privileges`
//! (19.11.2026), shared with ofd and the coming osd. Must run while the
//! process is still single-threaded and before any database connection.

pub use daemon_crate::privileges::drop_privileges;
