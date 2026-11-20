//! Schema-Migration: vergleicht JSON-Schema mit Live-DB und migriert.

use std::path::{Path, PathBuf};

use tracing::{info, warn, debug};

use crate::schema::{split_qualified, CatalogSchema, ObjectKind};
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

    /// All versions folded into one schema: the Bareos base tables plus every
    /// later addition (tables, columns, indices, functions, views; of a
    /// function or view defined again later only the newest definition). Its
    /// `to_statements()` creates what a fresh `migrate()` creates, which a
    /// single extension file (no tables of its own) cannot.
    pub fn combined(&self) -> Result<Option<CatalogSchema>, OrmError> {
        let mut versions = self.load_versions()?.into_iter();
        let Some(mut combined) = versions.next() else { return Ok(None) };
        for next in versions {
            for table in next.tables {
                match combined.tables.iter_mut().find(|t| t.name == table.name) {
                    Some(existing) => {
                        for col in table.columns {
                            if !existing.columns.iter().any(|c| c.name == col.name) {
                                existing.columns.push(col);
                            }
                        }
                        for idx in table.indices {
                            if !existing.indices.iter().any(|i| i.name == idx.name) {
                                existing.indices.push(idx);
                            }
                        }
                    }
                    None => combined.tables.push(table),
                }
            }
            // A later version redefines a function or view of the same
            // name (v2254 the views of v2251/v2252): only the newest stays.
            for func in next.functions {
                combined.functions.retain(|f| f.name != func.name);
                combined.functions.push(func);
            }
            for view in next.views {
                combined.views.retain(|v| v.name != view.name);
                combined.views.push(view);
            }
            combined.version = next.version;
            combined.description = next.description;
        }
        Ok(Some(combined))
    }

    /// Generiere die Migration-SQL von Version `from` nach `to`.
    pub fn diff(from: &CatalogSchema, to: &CatalogSchema) -> Vec<String> {
        let mut statements = Vec::new();

        // Neue Tabellen
        for table in &to.tables {
            if from.find_table(&table.name).is_none() {
                statements.extend(table.to_statements());
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
    ///
    /// Per step: create the schemas its qualified objects need
    /// (`ensure_schemas`, error = stop), move the objects of
    /// [`CatalogSchema::relocate`] (`relocate_objects`), apply the step's
    /// statements (failures are logged and skipped), record the step. A
    /// schema or relocation error leaves the step unrecorded, so the next
    /// start tries again.
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

            // Own schemas first (hard error: the step's objects need them),
            // then objects that move there, then the step's own objects.
            ensure_schemas(pool, schema).await?;
            let problems = relocate_objects(pool, &versions, schema).await;

            let stmts = if db_version == 0 {
                // Fresh install: alle Tabellen anlegen (Statement fuer Statement)
                schema.to_statements()
            } else {
                // Inkrementelle Migration: nur Diff anwenden
                versions.iter()
                    .rfind(|s| s.version < schema.version)
                    .map(|prev_schema| Self::diff(prev_schema, schema))
                    .unwrap_or_default()
            };
            run_statements(pool, &stmts).await;

            if !problems.is_empty() {
                // Not recorded: the next start tries the step again.
                return Err(OrmError::Other(format!(
                    "schema v{}: {} (version not recorded, retried at the next start)",
                    schema.version, problems.join("; "))));
            }

            self.set_db_version(pool, schema).await?;
            applied += 1;
            info!("migration to v{} complete", schema.version);
        }

        repair_cpp_version(pool, &versions).await?;
        check_sequences(pool).await?;

        if applied == 0 {
            let latest = versions.last().map(|s| s.version).unwrap_or(0);
            info!("database already at v{db_version} (latest: v{latest})");
        }

        Ok(applied)
    }

    /// Applied schema version: the highest of the Bareos `version` table
    /// (base catalog) and our `bareos_schema_version` (incl. Onesimus extensions).
    pub(crate) async fn get_db_version(&self, pool: &sqlx::PgPool) -> Result<u32, OrmError> {
        let mut found: Option<u32> = None;

        // 1. C++ Version-Tabelle (VersionId) — Bareos-Basisschema
        let cpp_exists = table_exists(pool, "version").await?;
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
        let our_exists = table_exists(pool, "bareos_schema_version").await?;
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
    ///
    /// `bareos_schema_version` deliberately stays unqualified in the default
    /// schema (`public`), the only Onesimus table outside the schema
    /// `onesimus`: every odir release looks for it there, and an older odir
    /// rolled back onto the catalog that did not find it would take the
    /// catalog for 2250 and create the v2251/v2252 objects in `public` again.
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

        let cpp_exists = table_exists(pool, "version").await.unwrap_or(false);
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

/// Statement kinds a migration must never run against a catalog it shares
/// with the Bareos Director: each drops data or moves a sequence, after
/// which new rows reuse ids (Compatibility log C-039). Migrations are
/// strictly additive (CREATE ... IF NOT EXISTS, ADD COLUMN, CREATE OR
/// REPLACE FUNCTION/VIEW).
fn destructive_op(stmt: &str) -> Option<&'static str> {
    let upper = stmt.to_ascii_uppercase();
    let words: Vec<&str> = upper
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    let starts = |prefix: &[&str]| words.len() >= prefix.len() && words[..prefix.len()] == *prefix;
    if starts(&["DROP", "TABLE"]) || starts(&["DROP", "SEQUENCE"]) || starts(&["DROP", "SCHEMA"]) {
        Some("DROP")
    } else if starts(&["TRUNCATE"]) {
        Some("TRUNCATE")
    } else if starts(&["ALTER", "SEQUENCE"]) {
        Some("ALTER SEQUENCE")
    } else if words.contains(&"SETVAL") {
        Some("setval")
    } else if starts(&["ALTER", "TABLE"]) && (words.contains(&"IDENTITY") || words.contains(&"RESTART")) {
        Some("ALTER TABLE ... IDENTITY")
    } else if starts(&["ALTER", "TABLE"]) && words.contains(&"DROP") {
        Some("ALTER TABLE ... DROP")
    } else {
        None
    }
}

/// Execute migration statements one by one. A failing statement is logged
/// and skipped (IF NOT EXISTS / OR REPLACE make re-runs harmless); comment
/// lines, statements [`destructive_op`] rejects and `CREATE SCHEMA` (done
/// before by [`ensure_schemas`], which also accepts a schema an
/// administrator created) are not executed.
async fn run_statements(pool: &sqlx::PgPool, stmts: &[String]) {
    for stmt in stmts {
        let trimmed = stmt.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") || trimmed.starts_with("CREATE SCHEMA ") {
            continue;
        }
        if let Some(op) = destructive_op(trimmed) {
            warn!("migration stmt skipped ({op} could reset ids in the shared catalog)\n  SQL: {}", &trimmed[..trimmed.len().min(120)]);
            continue;
        }
        debug!("  {}", &trimmed[..trimmed.len().min(80)]);
        if let Err(e) = sqlx::query(trimmed).execute(pool).await {
            warn!("migration stmt failed: {e}\n  SQL: {}", &trimmed[..trimmed.len().min(120)]);
        }
    }
}

/// Make sure every schema the step's objects are qualified with exists and
/// the connected role may create objects in it. A missing schema is created
/// owned by the connected role (the catalog database user); that needs the
/// CREATE privilege on the database. An administrator may instead create it
/// beforehand (`CREATE SCHEMA onesimus AUTHORIZATION <catalog user>`): an
/// existing schema is never created again, so the database privilege is not
/// needed then. Errors end the migration before the step is recorded.
async fn ensure_schemas(pool: &sqlx::PgPool, schema: &CatalogSchema) -> Result<(), OrmError> {
    for name in schema.object_schemas() {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname = $1)")
            .bind(&name).fetch_one(pool).await?;
        if !exists {
            if let Err(e) = sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {}", quote_ident(&name)))
                .execute(pool).await
            {
                return Err(OrmError::Other(format!(
                    "schema v{}: cannot create schema {name}: {e} (the catalog user needs \
                     GRANT CREATE ON DATABASE <catalog> TO <user>, or a superuser creates it: \
                     CREATE SCHEMA {name} AUTHORIZATION <user>)", schema.version)));
            }
            info!("created schema {name}");
        }
        let may_create: bool = sqlx::query_scalar("SELECT has_schema_privilege($1, 'CREATE')")
            .bind(&name).fetch_one(pool).await?;
        if !may_create {
            return Err(OrmError::Other(format!(
                "schema v{}: the catalog user may not create objects in schema {name} \
                 (GRANT CREATE, USAGE ON SCHEMA {name} TO <user>)", schema.version)));
        }
    }
    Ok(())
}

/// Move the objects of [`CatalogSchema::relocate`] from the default schema
/// (`current_schema()`: the first existing schema of the `search_path`,
/// where earlier versions created them unqualified, normally `public`) to
/// their target schema, then apply their newest definition from `versions`
/// (up to `schema`) again. Idempotent: an object no longer in the default
/// schema is left alone; one present in both schemas (created again in the
/// default schema, e.g. by hand) loses its default-schema copy. `ALTER ...
/// SET SCHEMA` keeps the object's OID, owner, grants and dependent objects.
/// Returns the problems; the caller does not record the step then.
async fn relocate_objects(pool: &sqlx::PgPool, versions: &[CatalogSchema], schema: &CatalogSchema) -> Vec<String> {
    let mut problems = Vec::new();
    if schema.relocate.is_empty() {
        return problems;
    }
    let default: Option<String> = match sqlx::query_scalar("SELECT current_schema()::text").fetch_one(pool).await {
        Ok(d) => d,
        Err(e) => return vec![format!("current_schema(): {e}")],
    };
    let Some(default) = default else {
        return vec!["no schema of the search_path exists".to_string()];
    };
    for def in &schema.relocate {
        let (Some(target), object) = split_qualified(&def.name) else {
            problems.push(format!("relocate {}: name without target schema", def.name));
            continue;
        };
        if target != default
            && let Err(e) = move_object(pool, def.kind, &default, target, object).await
        {
            problems.push(format!("moving {object} from {default} to {target}: {e}"));
            continue;
        }
        let newest = versions.iter().rev().filter(|v| v.version <= schema.version);
        let sql = match def.kind {
            ObjectKind::View => newest.into_iter()
                .find_map(|v| v.views.iter().find(|x| x.name == def.name)).map(|x| x.to_sql()),
            ObjectKind::Function => newest.into_iter()
                .find_map(|v| v.functions.iter().find(|x| x.name == def.name)).map(|x| x.to_sql()),
        };
        match sql {
            Some(sql) => if let Err(e) = sqlx::query(&sql).execute(pool).await {
                problems.push(format!("re-creating {}: {e}", def.name));
            },
            None => problems.push(format!("relocate {}: no definition in the schema files", def.name)),
        }
    }
    problems
}

/// One relocation of [`relocate_objects`]: `ALTER VIEW/FUNCTION ... SET
/// SCHEMA` for every match of `object` in `default`, or `DROP` of the
/// default-schema copy when `target` already has the same object (same view
/// name, same function signature). DROP is without CASCADE: an object of
/// the operator depending on the copy makes it fail instead of vanishing.
async fn move_object(pool: &sqlx::PgPool, kind: ObjectKind, default: &str, target: &str, object: &str)
    -> Result<(), sqlx::Error>
{
    let statements: Vec<String> = match kind {
        ObjectKind::View => {
            let rows: Vec<(String, bool)> = sqlx::query_as(
                "SELECT c.relkind::text, EXISTS(SELECT 1 FROM pg_class t JOIN pg_namespace tn ON tn.oid = t.relnamespace \
                                               WHERE tn.nspname = $3 AND t.relname = c.relname) \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('v', 'm')")
                .bind(default).bind(object).bind(target).fetch_all(pool).await?;
            rows.into_iter().map(|(relkind, in_target)| {
                let what = if relkind == "m" { "MATERIALIZED VIEW" } else { "VIEW" };
                let source = format!("{}.{}", quote_ident(default), quote_ident(object));
                if in_target {
                    format!("DROP {what} {source}")
                } else {
                    format!("ALTER {what} {source} SET SCHEMA {}", quote_ident(target))
                }
            }).collect()
        }
        ObjectKind::Function => {
            let rows: Vec<(String, bool)> = sqlx::query_as(
                "SELECT pg_get_function_identity_arguments(p.oid), \
                        EXISTS(SELECT 1 FROM pg_proc t JOIN pg_namespace tn ON tn.oid = t.pronamespace \
                               WHERE tn.nspname = $3 AND t.proname = p.proname AND t.proargtypes = p.proargtypes) \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname = $1 AND p.proname = $2")
                .bind(default).bind(object).bind(target).fetch_all(pool).await?;
            rows.into_iter().map(|(args, in_target)| {
                let source = format!("{}.{}({args})", quote_ident(default), quote_ident(object));
                if in_target {
                    format!("DROP FUNCTION {source}")
                } else {
                    format!("ALTER FUNCTION {source} SET SCHEMA {}", quote_ident(target))
                }
            }).collect()
        }
    };
    for sql in statements {
        info!("schema relocation: {sql}");
        sqlx::query(&sql).execute(pool).await?;
    }
    Ok(())
}

/// Whether `name` resolves to a table on the connection's `search_path` —
/// the same lookup the unqualified statements here use, so a catalog in
/// another schema of the same database is never mistaken for this one.
async fn table_exists(pool: &sqlx::PgPool, name: &str) -> Result<bool, OrmError> {
    Ok(sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(name)
        .fetch_one(pool)
        .await?)
}

/// A serial sequence whose next value is already taken in its table: the
/// next insert relying on the column default reuses an id or fails with a
/// duplicate key (Compatibility log C-039).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceLag {
    pub sequence: String,
    pub table: String,
    pub column: String,
    /// Value the next `nextval()` returns.
    pub next_value: i64,
    pub max_id: i64,
}

impl SequenceLag {
    /// SQL that moves the sequence to the highest id. It only ever advances
    /// the sequence; Onesimus never runs it itself — the catalog belongs to
    /// the Bareos Director, the operator decides.
    pub fn repair_sql(&self) -> String {
        format!(
            "SELECT setval('{}', (SELECT MAX({}) FROM {}));",
            quote_ident(&self.sequence).replace('\'', "''"),
            quote_ident(&self.column),
            quote_ident(&self.table)
        )
    }
}

/// Double-quoted SQL identifier (`"` doubled).
pub(crate) fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Read-only check of every column-owned sequence in the `search_path`
/// schema against the highest id of its column. Lagging sequences are
/// logged with their repair SQL but never changed: the catalog is shared
/// with the Bareos Director, and resetting or lowering a sequence would let
/// both Directors reuse ids. Cause seen in C-039: rows copied with explicit
/// ids into a freshly created catalog (`\copy` without `setval`).
pub async fn check_sequences(pool: &sqlx::PgPool) -> Result<Vec<SequenceLag>, OrmError> {
    let owned: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT s.relname::text, t.relname::text, a.attname::text \
         FROM pg_class s \
         JOIN pg_depend d ON d.classid = 'pg_class'::regclass AND d.objid = s.oid \
              AND d.refclassid = 'pg_class'::regclass AND d.deptype IN ('a', 'i') \
         JOIN pg_class t ON t.oid = d.refobjid \
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = d.refobjsubid \
         WHERE s.relkind = 'S' AND s.relnamespace = to_regnamespace(current_schema())::oid \
         ORDER BY s.relname"
    ).fetch_all(pool).await?;

    let mut lags = Vec::new();
    for (sequence, table, column) in owned {
        let (last_value, is_called): (i64, bool) = sqlx::query_as(&format!(
            "SELECT last_value, is_called FROM {}", quote_ident(&sequence)
        )).fetch_one(pool).await?;
        let max_id: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT MAX({})::bigint FROM {}", quote_ident(&column), quote_ident(&table)
        )).fetch_one(pool).await?;
        let next_value = if is_called { last_value + 1 } else { last_value };
        if let Some(max_id) = max_id.filter(|&m| next_value <= m) {
            let lag = SequenceLag { sequence, table, column, next_value, max_id };
            warn!("catalog sequence {} hands out {} next, but {}.{} already reaches {}: ids would be reused \
                   (not repaired automatically): {}",
                  lag.sequence, lag.next_value, lag.table, lag.column, lag.max_id, lag.repair_sql());
            lags.push(lag);
        }
    }
    Ok(lags)
}

/// Schema files authored by Onesimus extend the catalog (views, functions)
/// but are no Bareos schema versions.
pub(crate) fn is_onesimus_extension(schema: &CatalogSchema) -> bool {
    schema.author.eq_ignore_ascii_case("onesimus")
}

/// Repair catalogs where an older odir wrote an extension version
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
    fn destructive_statements_are_recognised() {
        for stmt in [
            "DROP TABLE IF EXISTS job CASCADE",
            "drop sequence job_jobid_seq",
            "TRUNCATE job RESTART IDENTITY",
            "ALTER SEQUENCE job_jobid_seq RESTART WITH 1",
            "SELECT setval('job_jobid_seq', 1)",
            "ALTER TABLE job ALTER COLUMN jobid ADD GENERATED ALWAYS AS IDENTITY",
            "ALTER TABLE job DROP COLUMN comment",
        ] {
            assert!(destructive_op(stmt).is_some(), "{stmt}");
        }
        for stmt in [
            "CREATE TABLE IF NOT EXISTS job (jobid serial NOT NULL)",
            "ALTER TABLE job ADD COLUMN metadata jsonb",
            "CREATE INDEX IF NOT EXISTS job_name_idx ON job (name)",
            "CREATE OR REPLACE VIEW vw_job_summary AS SELECT jobid FROM job",
        ] {
            assert_eq!(destructive_op(stmt), None, "{stmt}");
        }
    }

    /// The shipped migrations are additive: no statement may drop data or
    /// touch a sequence, and Onesimus versions add no Bareos tables/columns
    /// (C-039).
    #[test]
    fn shipped_migrations_never_touch_sequences() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let versions = Migrator::new(&dir).load_versions().unwrap();
        assert!(versions.len() >= 3);
        let base = &versions[0];
        assert_eq!(base.version, 2250, "Bareos 25.1.2 base schema (BDB_VERSION)");
        assert!(!is_onesimus_extension(base));
        let mut stmts = base.to_statements();
        for pair in versions.windows(2) {
            let diff = Migrator::diff(&pair[0], &pair[1]);
            assert!(is_onesimus_extension(&pair[1]),
                "v{} would be written to the Bareos version table", pair[1].version);
            for table in &base.tables {
                let alter = format!("ALTER TABLE {} ", table.name);
                assert!(!diff.iter().any(|s| s.starts_with(&alter)),
                    "v{} changes Bareos table {}", pair[1].version, table.name);
            }
            stmts.extend(diff);
        }
        // Comment lines are not executed by migrate() (diff marks tables
        // missing from an extension file as "-- DROP TABLE ...").
        for stmt in stmts.iter().filter(|s| !s.trim().starts_with("--")) {
            assert_eq!(destructive_op(stmt.trim()), None, "{stmt}");
            assert!(!stmt.to_ascii_uppercase().contains("SETVAL"), "{stmt}");
        }
    }

    /// `.catalog create` builds from all versions folded together: the Bareos
    /// base tables with their original keys plus the Onesimus extensions
    /// (Compatibility log C-052).
    #[test]
    fn combined_schema_creates_base_tables_and_extensions() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let migrator = Migrator::new(&dir);
        let combined = migrator.combined().unwrap().unwrap();
        let versions = migrator.load_versions().unwrap();
        assert_eq!(combined.version, versions.last().unwrap().version);
        let stmts = combined.to_statements();
        let pv = stmts.iter().find(|s| s.starts_with("CREATE TABLE IF NOT EXISTS pathvisibility")).unwrap();
        assert!(pv.contains("PRIMARY KEY (jobid, pathid)"), "{pv}");
        let version = stmts.iter().find(|s| s.starts_with("CREATE TABLE IF NOT EXISTS version")).unwrap();
        assert!(!version.contains("PRIMARY KEY"), "{version}");
        assert!(stmts.iter().any(|s| s.starts_with("INSERT INTO version (versionid) SELECT 2250 WHERE NOT EXISTS")));
        assert_eq!(stmts.iter().filter(|s| s.starts_with("INSERT INTO status ")).count(), 26);
        for f in versions.iter().flat_map(|v| &v.functions) {
            assert!(stmts.iter().any(|s| s.starts_with(&format!("CREATE OR REPLACE FUNCTION {}(", f.name))), "{}", f.name);
        }
    }

    /// Every Onesimus object of the shipped files is qualified with the
    /// schema `onesimus` (public stays pure Bareos), the Bareos base has no
    /// qualified names, and v2253 moves exactly the views and functions
    /// v2251/v2252 created unqualified before, each with a definition.
    #[test]
    fn onesimus_objects_live_in_their_own_schema() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let versions = Migrator::new(&dir).load_versions().unwrap();
        assert!(versions[0].object_schemas().is_empty());
        let mut defined: Vec<(ObjectKind, String)> = Vec::new();
        for v in versions.iter().filter(|v| is_onesimus_extension(v)) {
            assert_eq!(v.object_schemas(), ["onesimus"], "v{}", v.version);
            let names = v.tables.iter().map(|t| &t.name)
                .chain(v.views.iter().map(|x| &x.name)).chain(v.functions.iter().map(|x| &x.name));
            for name in names {
                assert!(name.starts_with("onesimus."), "v{}: {name}", v.version);
            }
            if v.version < 2253 {
                defined.extend(v.views.iter().map(|x| (ObjectKind::View, x.name.clone())));
                defined.extend(v.functions.iter().map(|x| (ObjectKind::Function, x.name.clone())));
            }
        }
        let step = versions.iter().find(|v| v.version == 2253).unwrap();
        let relocated: Vec<(ObjectKind, String)> = step.relocate.iter().map(|r| (r.kind, r.name.clone())).collect();
        assert_eq!(relocated, defined);
        assert_eq!(defined.len(), 16);
        let stmts = step.to_statements();
        assert_eq!(stmts[0], "CREATE SCHEMA IF NOT EXISTS onesimus;");
        assert!(stmts[1].starts_with("CREATE TABLE IF NOT EXISTS onesimus.console_acl ("), "{}", stmts[1]);
    }

    #[test]
    fn repair_sql_only_advances_to_max() {
        let lag = SequenceLag {
            sequence: "job_jobid_seq".into(), table: "job".into(), column: "jobid".into(),
            next_value: 9, max_id: 629,
        };
        assert_eq!(lag.repair_sql(),
            "SELECT setval('\"job_jobid_seq\"', (SELECT MAX(\"jobid\") FROM \"job\"));");
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
