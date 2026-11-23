// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Build script: writes the module and type tree for rustdoc
//! (`scripts/doc/module_tree.rs`, docs/wiki/dokumentation.md).

include!("../../scripts/doc/module_tree.rs");

fn main() {
    write_module_tree();
}
