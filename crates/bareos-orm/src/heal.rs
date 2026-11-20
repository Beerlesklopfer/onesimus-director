//! Self-healing of the Onesimus catalog objects, run by odir at every start
//! after [`Migrator::migrate`].
//!
//! The migration records each step once in `bareos_schema_version`; an
//! object lost afterwards (a view dropped by hand before a Bareos upgrade, a
//! function replaced, the table of the catalog ACL removed) would never come
//! back, because the step counts as applied. [`Migrator::heal`] therefore
//! checks every shipped Onesimus object against the newest definition in
//! the schema files and re-applies that definition when the object is
//! missing or broken:
//!
//! - **schema** `onesimus` missing: created (as [`Migrator::migrate`] does);
//! - **table** missing: `CREATE TABLE IF NOT EXISTS` with its indices;
//! - **function** missing, its body (`prosrc`), result type or language
//!   differs from the shipped definition, or the language validator rejects
//!   it (`fmgr_sql_validator`/`plpgsql_validator`: parses and, for SQL,
//!   analyses the body against the current catalog without running it — no
//!   function is ever called, several of them write);
//! - **view** missing, reading a relation outside its own schema directly
//!   (the pre-v2254 form that blocks Bareos upgrades), or failing the probe
//!   `EXPLAIN SELECT * FROM <view>` (plans the query, which inlines the SQL
//!   function behind the view and so resolves every column, executes
//!   nothing).
//!
//! Re-applying is `CREATE OR REPLACE`; when PostgreSQL refuses that (changed
//! result type of a function, changed columns of a view) the object is
//! dropped **without CASCADE** and created again in one transaction, so a
//! failure leaves the old object in place. Shipped views depending on a
//! function that is dropped are dropped and re-created in the same
//! transaction; an operator's own object depending on it makes the repair
//! fail (reported) instead of vanishing. Every repair is checked again; what
//! stays broken is reported in [`HealReport::failed`].
//!
//! Healing runs only when the catalog is exactly at the newest shipped step:
//! below, the migration is incomplete (it retries at the next start); above,
//! a newer odir installed newer definitions that an older one must not
//! overwrite. Nothing here returns an error: odir logs the report and starts
//! in every case.

use std::collections::BTreeSet;

use tracing::{info, warn};

use crate::migration::{is_onesimus_extension, quote_ident, Migrator};
use crate::schema::{split_qualified, CatalogSchema, FunctionDef, TableDef, ViewDef};

/// Outcome of [`Migrator::heal`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HealReport {
    /// Why nothing was checked (catalog below or above the newest shipped
    /// step, schema files or catalog not readable); `None` when checked.
    pub skipped: Option<String>,
    /// Number of objects checked (schemas, tables, functions, views).
    pub checked: usize,
    /// Objects re-created, each as `<kind> <name>: <reason>`.
    pub repaired: Vec<String>,
    /// Objects missing or broken that could not be repaired, each as
    /// `<kind> <name>: <reason>: <error>`.
    pub failed: Vec<String>,
}

/// The newest definition of every Onesimus object of the schema files
/// (qualified names of Onesimus extension files only; the Bareos base is
/// never touched).
#[derive(Debug, Default)]
struct Shipped<'a> {
    /// Schemas the objects live in (`onesimus`).
    schemas: BTreeSet<String>,
    /// Tables, newest definition per name.
    tables: Vec<&'a TableDef>,
    /// Functions, newest definition per name.
    functions: Vec<&'a FunctionDef>,
    /// Views, newest definition per name.
    views: Vec<&'a ViewDef>,
}

impl<'a> Shipped<'a> {
    /// Collect the newest definitions from `versions` (sorted ascending).
    fn collect(versions: &'a [CatalogSchema]) -> Self {
        let mut shipped = Shipped::default();
        for version in versions.iter().rev().filter(|v| is_onesimus_extension(v)) {
            let qualified = |name: &str| split_qualified(name).0.map(str::to_string);
            for table in &version.tables {
                if let Some(schema) = qualified(&table.name)
                    && !shipped.tables.iter().any(|t| t.name == table.name)
                {
                    shipped.schemas.insert(schema);
                    shipped.tables.push(table);
                }
            }
            for func in &version.functions {
                if let Some(schema) = qualified(&func.name)
                    && !shipped.functions.iter().any(|f| f.name == func.name)
                {
                    shipped.schemas.insert(schema);
                    shipped.functions.push(func);
                }
            }
            for view in &version.views {
                if let Some(schema) = qualified(&view.name)
                    && !shipped.views.iter().any(|v| v.name == view.name)
                {
                    shipped.schemas.insert(schema);
                    shipped.views.push(view);
                }
            }
        }
        shipped
    }
}

impl Migrator {
    /// Check every shipped Onesimus object and re-apply the shipped
    /// definition of those missing or broken (module documentation). Never
    /// fails: problems are logged and returned in the report.
    pub async fn heal(&self, pool: &sqlx::PgPool) -> HealReport {
        let mut report = HealReport::default();
        let versions = match self.load_versions() {
            Ok(v) if !v.is_empty() => v,
            Ok(_) => {
                report.skipped = Some("no schema files".into());
                return report;
            }
            Err(e) => {
                report.skipped = Some(format!("schema files not readable: {e}"));
                return report;
            }
        };
        let state = match self.applied_steps(pool).await {
            Ok(state) => state,
            Err(e) => {
                report.skipped = Some(format!("catalog version not readable: {e}"));
                return report;
            }
        };
        // Per step, not by the highest number: a Bareos `version` above
        // the shipped steps (Bareos upgraded the catalog) is fine.
        let missing: Vec<u32> = state.plan(&versions).into_iter()
            .filter(|(_, record)| *record).map(|(i, _)| versions[i].version).collect();
        if !missing.is_empty() {
            report.skipped = Some(format!(
                "schema steps {missing:?} not applied: migration incomplete, retried at the next start"));
            return report;
        }
        let newer = state.newer_than(&versions);
        if !newer.is_empty() {
            report.skipped = Some(format!(
                "catalog has schema steps {newer:?} unknown to this odir: its objects belong to a newer odir"));
            return report;
        }
        let shipped = Shipped::collect(&versions);

        for schema in &shipped.schemas {
            report.checked += 1;
            match schema_exists(pool, schema).await {
                Ok(true) => {}
                Ok(false) => match sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {}", quote_ident(schema)))
                    .execute(pool).await
                {
                    Ok(_) => record_repair(&mut report, format!("schema {schema}: missing")),
                    Err(e) => {
                        record_failure(&mut report, format!("schema {schema}: missing: {e}"));
                        return report;
                    }
                },
                Err(e) => {
                    record_failure(&mut report, format!("schema {schema}: not checkable: {e}"));
                    return report;
                }
            }
        }

        for table in &shipped.tables {
            report.checked += 1;
            match relation_exists(pool, &table.name).await {
                Ok(true) => {}
                Ok(false) => match run_in_transaction(pool, &table.to_statements()).await {
                    Ok(()) => record_repair(&mut report, format!("table {}: missing", table.name)),
                    Err(e) => record_failure(&mut report, format!("table {}: missing: {e}", table.name)),
                },
                Err(e) => record_failure(&mut report, format!("table {}: not checkable: {e}", table.name)),
            }
        }

        for func in &shipped.functions {
            report.checked += 1;
            let Some(reason) = check_function(pool, func).await else { continue };
            match repair_function(pool, func, &shipped.views).await {
                Ok(()) => match check_function(pool, func).await {
                    None => record_repair(&mut report, format!("function {}: {reason}", func.name)),
                    Some(still) => record_failure(&mut report,
                        format!("function {}: {reason}: re-created, but still {still}", func.name)),
                },
                Err(e) => record_failure(&mut report, format!("function {}: {reason}: {e}", func.name)),
            }
        }

        for view in &shipped.views {
            report.checked += 1;
            let Some(reason) = check_view(pool, view).await else { continue };
            match repair_view(pool, view).await {
                Ok(()) => match check_view(pool, view).await {
                    None => record_repair(&mut report, format!("view {}: {reason}", view.name)),
                    Some(still) => record_failure(&mut report,
                        format!("view {}: {reason}: re-created, but still {still}", view.name)),
                },
                Err(e) => record_failure(&mut report, format!("view {}: {reason}: {e}", view.name)),
            }
        }
        report
    }
}

/// Log and record one repaired object.
fn record_repair(report: &mut HealReport, what: String) {
    info!("catalog self-heal: re-created {what}");
    report.repaired.push(what);
}

/// Log and record one object that stays missing or broken.
fn record_failure(report: &mut HealReport, what: String) {
    warn!("catalog self-heal: cannot repair {what}");
    report.failed.push(what);
}

/// Whether the schema `name` exists.
async fn schema_exists(pool: &sqlx::PgPool, name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname = $1)")
        .bind(name).fetch_one(pool).await
}

/// Whether the (qualified) relation `name` exists.
async fn relation_exists(pool: &sqlx::PgPool, name: &str) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL").bind(name).fetch_one(pool).await
}

/// Run `statements` in one transaction (all or nothing).
async fn run_in_transaction(pool: &sqlx::PgPool, statements: &[String]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for sql in statements {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    tx.commit().await
}

/// Lower case with single spaces, for comparing type texts
/// (`TABLE(a  bigint)` = `table(a bigint)`).
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase()
}

/// The installed function as [`check_function`] reads it: oid, body
/// (`prosrc`), result type, language name, language validator (`-`: none).
type InstalledFunction = (i64, String, String, String, String);

/// What is wrong with the installed function of `func`, `None` when it
/// matches the shipped definition and its language validator accepts it.
async fn check_function(pool: &sqlx::PgPool, func: &FunctionDef) -> Option<String> {
    let row: Result<Option<InstalledFunction>, sqlx::Error> = sqlx::query_as(
        "SELECT p.oid::bigint, p.prosrc, pg_get_function_result(p.oid), l.lanname::text, \
                l.lanvalidator::regproc::text \
         FROM pg_proc p JOIN pg_language l ON l.oid = p.prolang WHERE p.oid = to_regprocedure($1)")
        .bind(func.identity()).fetch_optional(pool).await;
    let (oid, source, result, language, validator) = match row {
        Ok(Some(row)) => row,
        Ok(None) => return Some("missing".into()),
        Err(e) => return Some(format!("not checkable ({e})")),
    };
    if normalize(&result) != normalize(&func.returns) {
        return Some(format!("returns {result}, shipped {}", func.returns));
    }
    if source.trim() != func.body.trim() {
        return Some("body differs from the shipped definition".into());
    }
    if !language.eq_ignore_ascii_case(&func.language) {
        return Some(format!("language {language}, shipped {}", func.language));
    }
    // "-" = no validator. The validator name comes from pg_language, the oid
    // is a number: nothing of the text is user input.
    if validator != "-"
        && let Err(e) = sqlx::query(&format!("SELECT {validator}({oid}::oid)")).execute(pool).await
    {
        return Some(format!("rejected by {validator} ({e})"));
    }
    None
}

/// Re-apply the shipped definition of `func`: `CREATE OR REPLACE`, and when
/// PostgreSQL refuses that (e.g. the result type changed), DROP (no CASCADE)
/// and CREATE in one transaction, together with the shipped `views` that
/// depend on the function (dropped first, re-created after).
async fn repair_function(pool: &sqlx::PgPool, func: &FunctionDef, views: &[&ViewDef]) -> Result<(), String> {
    let replace = match sqlx::query(&func.to_sql()).execute(pool).await {
        Ok(_) => return Ok(()),
        Err(e) => e.to_string(),
    };
    let dependent: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT n.nspname || '.' || c.relname FROM pg_depend d \
         JOIN pg_rewrite r ON d.classid = 'pg_rewrite'::regclass AND r.oid = d.objid \
         JOIN pg_class c ON c.oid = r.ev_class JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE d.refclassid = 'pg_proc'::regclass AND d.refobjid = to_regprocedure($1)")
        .bind(func.identity()).fetch_all(pool).await
        .map_err(|e| format!("CREATE OR REPLACE: {replace}; dependent views: {e}"))?;
    let mut drop_views = Vec::new();
    let mut create_views = Vec::new();
    for name in &dependent {
        match views.iter().find(|v| &v.name == name) {
            Some(view) => {
                drop_views.push(format!("DROP VIEW {name}"));
                create_views.push(view.to_sql());
            }
            None => return Err(format!("CREATE OR REPLACE: {replace}; not dropped, {name} (not shipped) depends on it")),
        }
    }
    let mut statements = drop_views;
    statements.push(format!("DROP FUNCTION {}", func.identity()));
    statements.push(func.to_sql());
    statements.extend(create_views);
    run_in_transaction(pool, &statements).await
        .map_err(|e| format!("CREATE OR REPLACE: {replace}; DROP and CREATE: {e}"))
}

/// What is wrong with the installed view of `view`, `None` when it exists,
/// reads nothing outside its own schema directly and plans without error.
async fn check_view(pool: &sqlx::PgPool, view: &ViewDef) -> Option<String> {
    let (Some(schema), name) = split_qualified(&view.name) else { return Some("name without schema".into()) };
    let oid: Result<Option<i64>, sqlx::Error> = sqlx::query_scalar(
        "SELECT c.oid::bigint FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('v', 'm')")
        .bind(schema).bind(name).fetch_optional(pool).await;
    let oid = match oid {
        Ok(Some(oid)) => oid,
        Ok(None) => return Some("missing".into()),
        Err(e) => return Some(format!("not checkable ({e})")),
    };
    let foreign: Result<Option<String>, sqlx::Error> = sqlx::query_scalar(
        "SELECT string_agg(DISTINCT rn.nspname || '.' || t.relname, ', ') FROM pg_rewrite r \
         JOIN pg_depend d ON d.classid = 'pg_rewrite'::regclass AND d.objid = r.oid \
              AND d.refclassid = 'pg_class'::regclass \
         JOIN pg_class t ON t.oid = d.refobjid JOIN pg_namespace rn ON rn.oid = t.relnamespace \
         WHERE r.ev_class = $1::bigint::oid AND t.oid <> r.ev_class AND rn.nspname <> $2")
        .bind(oid).bind(schema).fetch_one(pool).await;
    match foreign {
        Ok(Some(tables)) => return Some(format!("reads {tables} directly (blocks Bareos upgrades)")),
        Ok(None) => {}
        Err(e) => return Some(format!("not checkable ({e})")),
    }
    if let Err(e) = sqlx::query(&format!("EXPLAIN SELECT * FROM {}", view.name)).execute(pool).await {
        return Some(format!("broken ({e})"));
    }
    None
}

/// Re-apply the shipped definition of `view`: `CREATE OR REPLACE VIEW`, and
/// when PostgreSQL refuses that (changed columns), DROP VIEW (no CASCADE)
/// and CREATE in one transaction.
async fn repair_view(pool: &sqlx::PgPool, view: &ViewDef) -> Result<(), String> {
    let replace = match sqlx::query(&view.to_sql()).execute(pool).await {
        Ok(_) => return Ok(()),
        Err(e) => e.to_string(),
    };
    let what = if view.materialized { "MATERIALIZED VIEW" } else { "VIEW" };
    run_in_transaction(pool, &[format!("DROP {what} {}", view.name), view.to_sql()]).await
        .map_err(|e| format!("CREATE OR REPLACE: {replace}; DROP and CREATE (own views depending on it?): {e}"))
}

/// Unit tests without a database (the live cases are in
/// `tests/onesimus_schema_migration.rs`).
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The shipped files give one newest definition per object, all in the
    /// schema `onesimus`, the views from v2254 (late-bound).
    #[test]
    fn shipped_objects_are_the_newest_definitions() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("schema");
        let versions = Migrator::new(&dir).load_versions().unwrap();
        let shipped = Shipped::collect(&versions);
        assert_eq!(shipped.schemas.iter().collect::<Vec<_>>(), ["onesimus"]);
        assert_eq!(shipped.tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["onesimus.console_acl"]);
        assert_eq!(shipped.views.len(), 6);
        assert_eq!(shipped.functions.len(), 16);
        for view in &shipped.views {
            let function = format!("onesimus.fn_{}()", split_qualified(&view.name).1);
            assert!(view.query.contains(&format!("FROM {function}")), "{}: {}", view.name, view.query);
        }
    }

    /// Result types compare without case and whitespace differences.
    #[test]
    fn type_texts_compare_normalized() {
        assert_eq!(normalize("TABLE(since_utime  bigint,\n prev_job text)"), "table(since_utime bigint, prev_job text)");
    }
}
