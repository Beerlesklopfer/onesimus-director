//! Catalog schema location of the startup migration.
//!
//! The former `.catalog` command (local extension, compatibility log C-040)
//! was never reachable: Bareos resolves a command by the first prefix match
//! in its table, so `.catalog` is `.catalogs` (as in the original).

/// Schema directory, the same one the startup migration uses: next to the
/// build tree in development, `/usr/lib/bareos/ddl/schema` when installed.
pub fn schema_dir() -> std::path::PathBuf {
    let installed = std::path::PathBuf::from("/usr/lib/bareos/ddl/schema");
    [
        // Dev: binary in target/<profile>/, schema in crates/bareos-orm/schema/
        std::env::current_exe().ok()
            .and_then(|p| p.parent()?.parent()?.parent().map(|d| d.join("crates/bareos-orm/schema"))),
        Some(installed.clone()),
    ].into_iter().flatten().find(|p| p.exists()).unwrap_or(installed)
}
