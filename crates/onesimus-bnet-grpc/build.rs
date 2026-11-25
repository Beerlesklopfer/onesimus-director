// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Build script: writes the module and type tree for rustdoc
//! (`scripts/doc/module_tree.rs`, docs/wiki/dokumentation.md).

include!("../../scripts/doc/module_tree.rs");

fn main() {
    write_module_tree();
}
