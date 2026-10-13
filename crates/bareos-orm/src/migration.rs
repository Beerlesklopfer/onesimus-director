//! Schema-Migration: vergleicht JSON-Schema mit Live-DB und migriert.

use std::path::{Path, PathBuf};

use tracing::{info, warn, debug};

use crate::schema::CatalogSchema;
use crate::error::OrmError;

/// Schema-Migrator: liest JSON-Versionen und migriert die DB.
pub struct Migrator {
    schema_dir: PathBuf,
}

impl Migrator {
    pub fn new(schema_dir: &Path) -> Self {
        Self {
            schema_dir: schema_dir.to_path_buf(),
        }
    }

    /// Lade alle Schema-Versionen aus dem Verzeichnis, sortiert nach Version.
    pub fn load_versions(&self) -> Result<Vec<CatalogSchema>, OrmError> {
        let mut schemas = Vec::new();

        if !self.schema_dir.exists() {
            return Ok(schemas);
        }

        for entry in std::fs::read_dir(&self.schema_dir)
            .map_err(|e| OrmError::Other(format!("read dir: {e}")))?
        {
            let entry = entry.map_err(|e| OrmError::Other(format!("dir entry: {e}")))?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                match CatalogSchema::load(&path) {
                    Ok(schema) => schemas.push(schema),
                    Err(e) => warn!("skip {}: {e}", path.display()),
                }
            }
        }

        schemas.sort_by_key(|s| s.version);
        Ok(schemas)
    }

    /// Lade die neueste Schema-Version.
    pub fn latest(&self) -> Result<Option<CatalogSchema>, OrmError> {
        let versions = self.load_versions()?;
        Ok(versions.into_iter().last())
    }

    /// Generiere die Migration-SQL von Version `from` nach `to`.
    pub fn diff(from: &CatalogSchema, to: &CatalogSchema) -> Vec<String> {
        let mut statements = Vec::new();

        // Neue Tabellen
        for table in &to.tables {
            if from.find_table(&table.name).is_none() {
                statements.push(table.to_create_sql());
                for idx in &table.indices {
                    statements.push(idx.to_sql(&table.name));
                }
            }
        }

        // Neue Spalten in bestehenden Tabellen
        for to_table in &to.tables {
            if let Some(from_table) = from.find_table(&to_table.name) {
                let from_cols: Vec<&str> = from_table.columns.iter()
                    .map(|c| c.name.as_str()).collect();

                for col in &to_table.columns {
                    if !from_cols.contains(&col.name.as_str()) {
                        statements.push(
                            crate::schema::TableDef::alter_add_column(&to_table.name, col)
                        );
                    }
                }

                // Neue Indizes
                let from_idx_names: Vec<&str> = from_table.indices.iter()
                    .map(|i| i.name.as_str()).collect();
                for idx in &to_table.indices {
                    if !from_idx_names.contains(&idx.name.as_str()) {
                        statements.push(idx.to_sql(&to_table.name));
                    }
                }
            }
        }

        // Gelöschte Tabellen (optional, vorsichtig)
        for from_table in &from.tables {
            if to.find_table(&from_table.name).is_none() {
                statements.push(format!("-- DROP TABLE {} (removed in v{});\n",
                    from_table.name, to.version));
            }
        }

        // Functions: CREATE OR REPLACE ist idempotent — immer anwenden
        for func in &to.functions {
            statements.push(func.to_sql());
        }

        // Views: CREATE OR REPLACE ist idempotent — immer anwenden
        for view in &to.views {
            statements.push(view.to_sql());
        }

        statements
    }

    /// Generiere vollständiges DDL für die neueste Version.
    pub fn full_ddl(&self) -> Result<String, OrmError> {
        match self.latest()? {
            Some(schema) => Ok(schema.to_ddl()),
            None => Err(OrmError::Other("no schema versions found".to_string())),
        }
    }

    /// Führe Migration auf einer PostgreSQL-Datenbank durch.
    /// Prüft die aktuelle Version (C++ `version` Tabelle oder unsere `bareos_schema_version`),
    /// wendet fehlende Schema-Versionen an (Statement für Statement).
    pub async fn migrate(&self, pool: &sqlx::PgPool) -> Result<u32, OrmError> {
        let db_version = self.get_db_version(pool).await?;

        let versions = self.load_versions()?;
        if versions.is_empty() {
            return Err(OrmError::Other("no schema versions found".to_string()));
        }

        let mut applied = 0;

        for schema in &versions {
            if schema.version <= db_version {
                debug!("schema v{} already applied (db=v{}), skipping", schema.version, db_version);
                continue;
            }

            info!("migrating to schema v{}: {}", schema.version, schema.description);

            if db_version == 0 {
                // Fresh install: alle Tabellen anlegen (Statement fuer Statement)
                let stmts = schema.to_statements();
                for stmt in &stmts {
                    let trimmed = stmt.trim();
                    if trimmed.is_empty() || trimmed.starts_with("--") { continue; }
                    debug!("  {}", &trimmed[..trimmed.len().min(80)]);
                    if let Err(e) = sqlx::query(trimmed).execute(pool).await {
                        warn!("migration stmt failed: {e}\n  SQL: {}", &trimmed[..trimmed.len().min(120)]);
                        // Continue — IF NOT EXISTS handles duplicates
                    }
                }
            } else {
                // Inkrementelle Migration: nur Diff anwenden
                let prev = versions.iter()
                    .filter(|s| s.version < schema.version)
                    .last();

                if let Some(prev_schema) = prev {
                    let stmts = Self::diff(prev_schema, schema);
                    for stmt in &stmts {
                        let trimmed = stmt.trim();
                        if trimmed.is_empty() || trimmed.starts_with("--") { continue; }
                        debug!("  {}", &trimmed[..trimmed.len().min(80)]);
                        if let Err(e) = sqlx::query(trimmed).execute(pool).await {
                            warn!("migration stmt failed: {e}\n  SQL: {}", &trimmed[..trimmed.len().min(120)]);
                        }
                    }
                }
            }

            self.set_db_version(pool, schema).await?;
            applied += 1;
            info!("migration to v{} complete", schema.version);
        }

        repair_cpp_version(pool, &versions).await?;

        if applied == 0 {
            let latest = versions.last().map(|s| s.version).unwrap_or(0);
            info!("database already at v{db_version} (latest: v{latest})");
        }

        Ok(applied)
    }

    /// Applied schema version: the highest of the Bareos `version` table
    /// (base catalog) and our `bareos_schema_version` (incl. Onesimus extensions).
    async fn get_db_version(&self, pool: &sqlx::PgPool) -> Result<u32, OrmError> {
        let mut found: Option<u32> = None;

        // 1. C++ Version-Tabelle (VersionId) — Bareos-Basisschema
        let cpp_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name = 'version')"
        ).fetch_one(pool).await?;
        if cpp_exists {
            let version: Option<i32> = sqlx::query_scalar(
                "SELECT MAX(versionid) FROM version"
            ).fetch_one(pool).await.unwrap_or(None);
            if let Some(v) = version {
                debug!("found C++ version table: VersionId={v}");
                found = Some(v as u32);
            }
        }

        // 2. Unsere Version-Tabelle — enthält auch die Onesimus-Erweiterungen
        let our_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name = 'bareos_schema_version')"
        ).fetch_one(pool).await?;
        if our_exists {
            let version: Option<i32> = sqlx::query_scalar(
                "SELECT MAX(version) FROM bareos_schema_version"
            ).fetch_one(pool).await.unwrap_or(None);
            if let Some(v) = version {
                debug!("found bareos_schema_version: v={v}");
                found = Some(found.map_or(v as u32, |f| f.max(v as u32)));
            }
        }

        if found.is_none() {
            info!("no version table found — fresh install");
        }
        Ok(found.unwrap_or(0))
    }

    /// Record an applied schema version.
    ///
    /// The Bareos `version` table only ever gets Bareos base schema versions:
    /// the C++ Director refuses to open a catalog whose VersionId it does not
    /// know, so Onesimus extensions (views, functions) must not bump it —
    /// otherwise a C++ Director on the same catalog stops working.
    async fn set_db_version(&self, pool: &sqlx::PgPool, schema: &CatalogSchema) -> Result<(), OrmError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS bareos_schema_version (version integer NOT NULL, applied_at timestamp DEFAULT now())"
        ).execute(pool).await?;
        sqlx::query("INSERT INTO bareos_schema_version (version) VALUES ($1)")
            .bind(schema.version as i32)
            .execute(pool)
            .await?;

        if is_onesimus_extension(schema) {
            return Ok(());
        }

        let cpp_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name = 'version')"
        ).fetch_one(pool).await.unwrap_or(false);
        if cpp_exists {
            sqlx::query("DELETE FROM version").execute(pool).await?;
            sqlx::query("INSERT INTO version (versionid) VALUES ($1)")
                .bind(schema.version as i32)
                .execute(pool)
                .await?;
        }

        Ok(())
    }
}

/// Schema files authored by Onesimus extend the catalog (views, functions)
/// but are no Bareos schema versions.
fn is_onesimus_extension(schema: &CatalogSchema) -> bool {
    schema.author.eq_ignore_ascii_case("onesimus")
}

/// Repair catalogs where an older onesimus-dird wrote an extension version
/// into the Bareos `version` table: reset it to the newest Bareos base
/// version so the C++ Director can open the catalog again.
pub async fn repair_cpp_version(pool: &sqlx::PgPool, versions: &[CatalogSchema]) -> Result<(), OrmError> {
    let base = versions.iter().filter(|s| !is_onesimus_extension(s)).map(|s| s.version).max();
    let Some(base) = base else { return Ok(()) };
    let cpp_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_name = 'version')"
    ).fetch_one(pool).await.unwrap_or(false);
    if !cpp_exists {
        return Ok(());
    }
    let current: Option<i32> = sqlx::query_scalar("SELECT MAX(versionid) FROM version")
        .fetch_one(pool).await.unwrap_or(None);
    let is_extension = current.is_some_and(|v| versions.iter()
        .any(|s| s.version == v as u32 && is_onesimus_extension(s)));
    if is_extension {
        info!("resetting Bareos catalog VersionId {} -> {base} (Onesimus extension versions are tracked in bareos_schema_version)", current.unwrap_or(0));
        sqlx::query("UPDATE version SET versionid = $1").bind(base as i32).execute(pool).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_detects_new_table() {
        let v1: CatalogSchema = serde_json::from_str(r#"{
            "version": 1, "description": "v1", "created": "2026", "author": "test",
            "tables": [
                {"name": "client", "primary_key": "id", "columns": [
                    {"name": "id", "type": "serial", "not_null": true}
                ]}
            ]
        }"#).unwrap();

        let v2: CatalogSchema = serde_json::from_str(r#"{
            "version": 2, "description": "v2", "created": "2026", "author": "test",
            "tables": [
                {"name": "client", "primary_key": "id", "columns": [
                    {"name": "id", "type": "serial", "not_null": true}
                ]},
                {"name": "job", "primary_key": "id", "columns": [
                    {"name": "id", "type": "serial", "not_null": true},
                    {"name": "name", "type": "text", "not_null": true}
                ]}
            ]
        }"#).unwrap();

        let stmts = Migrator::diff(&v1, &v2);
        assert!(!stmts.is_empty());
        let sql = stmts.join("\n");
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS job"));
    }

    #[test]
    fn diff_detects_new_column() {
        let v1: CatalogSchema = serde_json::from_str(r#"{
            "version": 1, "description": "v1", "created": "2026", "author": "test",
            "tables": [
                {"name": "job", "primary_key": "id", "columns": [
                    {"name": "id", "type": "serial", "not_null": true}
                ]}
            ]
        }"#).unwrap();

        let v2: CatalogSchema = serde_json::from_str(r#"{
            "version": 2, "description": "v2", "created": "2026", "author": "test",
            "tables": [
                {"name": "job", "primary_key": "id", "columns": [
                    {"name": "id", "type": "serial", "not_null": true},
                    {"name": "metadata", "type": "jsonb"}
                ]}
            ]
        }"#).unwrap();

        let stmts = Migrator::diff(&v1, &v2);
        let sql = stmts.join("\n");
        assert!(sql.contains("ALTER TABLE job ADD COLUMN metadata jsonb"));
    }
}
