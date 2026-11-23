// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Catalog schema location of the startup migration.
//!
//! The former `.catalog` command (local extension, compatibility log C-040)
//! was never reachable: Bareos resolves a command by the first prefix match
//! in its table, so `.catalog` is `.catalogs` (as in the original).

/// Where the odir package installs the schema JSON files
/// (crates/odir/Cargo.toml, package.metadata.deb assets).
pub const INSTALLED_SCHEMA_DIR: &str = "/usr/lib/bareos/scripts/ddl";

/// Schema directory, the same one the startup migration uses: next to the
/// build tree in development, [`INSTALLED_SCHEMA_DIR`] when installed.
/// Before, the installed path was /usr/lib/bareos/ddl/schema, where nothing
/// is installed, so the extension (views and functions that shorten the
/// catalog queries) never reached an installed catalog (T-11).
pub fn schema_dir() -> std::path::PathBuf {
    schema_dir_from(std::env::current_exe().ok().as_deref())
}

fn schema_dir_from(exe: Option<&std::path::Path>) -> std::path::PathBuf {
    let installed = std::path::PathBuf::from(INSTALLED_SCHEMA_DIR);
    [
        // Dev: binary in target/<profile>/, schema in crates/bareos-orm/schema/
        exe.and_then(|p| p.parent()?.parent()?.parent().map(|d| d.join("crates/bareos-orm/schema"))),
        Some(installed.clone()),
    ].into_iter().flatten().find(|p| p.exists()).unwrap_or(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed path is the one the package writes the JSON files to.
    #[test]
    fn installed_schema_dir_matches_the_package_assets() {
        let cargo = include_str!("../../Cargo.toml");
        let asset = cargo.lines().find(|l| l.contains("../bareos-orm/schema/*.json")).expect("schema asset line");
        assert!(asset.contains(&format!("\"{}/\"", INSTALLED_SCHEMA_DIR.trim_start_matches('/'))), "{asset}");
        // An installed binary (no build tree around it) uses that path.
        assert_eq!(schema_dir_from(Some(std::path::Path::new("/usr/sbin/odir"))), std::path::PathBuf::from(INSTALLED_SCHEMA_DIR));
    }

    /// In the build tree the schema next to the sources is used.
    #[test]
    fn development_binary_uses_the_source_schema() {
        let exe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/odir");
        assert!(schema_dir_from(Some(&exe)).ends_with("crates/bareos-orm/schema"));
    }
}
