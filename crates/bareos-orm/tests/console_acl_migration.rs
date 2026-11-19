//! Schema extension v2253 (`onesimus_console_acl`, inner tier of the
//! two-tier console ACLs, docs/plans/ACL-ZWEISTUFIG.md).
//!
//! Needs a PostgreSQL database like `catalog_sequences.rs`: set
//! `BAREOS_ORM_TEST_DB` to a connection URL of a throw-away database. Each
//! case works inside a schema it creates and drops itself (`search_path` is
//! pinned to it). Without the variable the tests are skipped (ignored).

use std::path::{Path, PathBuf};
use std::str::FromStr;

use bareos_orm::migration::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// The schema directory shipped with the crate.
fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema")
}

/// Run `case` in a fresh schema `<tag>_<pid>` and drop the schema afterwards.
async fn in_schema<F, Fut>(tag: &str, case: F)
where
    F: FnOnce(sqlx::PgPool) -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    let url = std::env::var("BAREOS_ORM_TEST_DB").expect("BAREOS_ORM_TEST_DB not set");
    let schema = format!("{tag}_{}", std::process::id());
    let admin = PgPoolOptions::new().max_connections(1).connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}")).execute(&admin).await.unwrap();
    let options = PgConnectOptions::from_str(&url).unwrap().options([("search_path", schema.as_str())]);
    let pool = PgPoolOptions::new().max_connections(1).connect_with(options).await.unwrap();
    let result = case(pool).await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE")).execute(&admin).await.unwrap();
    result.unwrap();
}

/// Columns of `onesimus_console_acl` in the current schema as `name type not-null`.
async fn acl_columns(pool: &sqlx::PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT column_name||' '||data_type||' '||is_nullable FROM information_schema.columns
         WHERE table_schema = current_schema() AND table_name = 'onesimus_console_acl' ORDER BY ordinal_position")
        .fetch_all(pool).await
}

/// What the table must look like after the migration, and that the primary
/// key and the type check hold.
async fn check_table(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(acl_columns(pool).await?, [
        "console text NO", "acl_type text NO", "position integer NO", "entry text NO",
        "changed_by text NO", "changed_at timestamp with time zone NO",
    ]);
    sqlx::query("INSERT INTO onesimus_console_acl (console, acl_type, position, entry, changed_by) \
                 VALUES ('c', 'command', 0, '!delete', 'admin')").execute(pool).await?;
    let duplicate = sqlx::query("INSERT INTO onesimus_console_acl (console, acl_type, position, entry, changed_by) \
                 VALUES ('c', 'command', 0, 'status', 'admin')").execute(pool).await;
    assert!(duplicate.is_err(), "primary key (console, acl_type, position)");
    let bad_type = sqlx::query("INSERT INTO onesimus_console_acl (console, acl_type, position, entry, changed_by) \
                 VALUES ('c', 'commandacl', 0, 'status', 'admin')").execute(pool).await;
    assert!(bad_type.is_err(), "acl_type check");
    let recent: bool = sqlx::query_scalar("SELECT changed_at > now() - interval '1 hour' FROM onesimus_console_acl")
        .fetch_one(pool).await?;
    assert!(recent, "changed_at defaults to now()");
    Ok(())
}

/// A fresh catalog gets the table with the other extensions; the Bareos
/// `version` table stays at 2250, the extension step is recorded in
/// `bareos_schema_version`.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away database>"]
async fn fresh_catalog_gets_the_console_acl_table() {
    in_schema("acl_fresh", |pool| async move {
        let migrator = Migrator::new(&schema_dir());
        migrator.migrate(&pool).await?;
        let version: i32 = sqlx::query_scalar("SELECT MAX(versionid) FROM version").fetch_one(&pool).await?;
        assert_eq!(version, 2250);
        let ours: Vec<i32> = sqlx::query_scalar("SELECT version FROM bareos_schema_version ORDER BY version")
            .fetch_all(&pool).await?;
        assert_eq!(ours, [2250, 2251, 2252, 2253]);
        check_table(&pool).await?;
        // A second start changes nothing and keeps the rows.
        assert_eq!(migrator.migrate(&pool).await?, 0);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM onesimus_console_acl").fetch_one(&pool).await?;
        assert_eq!(rows, 1);
        Ok(())
    }).await;
}

/// A catalog at the previous extension step (2252, as every odir before the
/// two-tier ACLs left it) gets the table at the next start, and so does a
/// catalog created by the Bareos DDL alone (`version` 2250, no
/// `bareos_schema_version`).
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away database>"]
async fn existing_catalog_gets_the_console_acl_table() {
    in_schema("acl_existing", |pool| async move {
        // The schema files up to 2252 only: what an older odir migrated.
        let old = std::env::temp_dir().join(format!("bareos-orm-acl-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&old);
        std::fs::create_dir_all(&old)?;
        for entry in std::fs::read_dir(schema_dir())? {
            let path = entry?.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if name.starts_with("v2250") || name.starts_with("v2251") || name.starts_with("v2252") {
                std::fs::copy(&path, old.join(&name))?;
            }
        }
        Migrator::new(&old).migrate(&pool).await?;
        std::fs::remove_dir_all(&old)?;
        assert!(acl_columns(&pool).await?.is_empty());
        assert_eq!(Migrator::new(&schema_dir()).migrate(&pool).await?, 1);
        let ours: i32 = sqlx::query_scalar("SELECT MAX(version) FROM bareos_schema_version").fetch_one(&pool).await?;
        assert_eq!(ours, 2253);
        check_table(&pool).await?;
        Ok(())
    }).await;
    in_schema("acl_cpp", |pool| async move {
        let ddl = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/bareos-18be77e6-creates-postgresql.sql"))?;
        sqlx::raw_sql(&ddl).execute(&pool).await?;
        assert_eq!(Migrator::new(&schema_dir()).migrate(&pool).await?, 3);
        let version: i32 = sqlx::query_scalar("SELECT MAX(versionid) FROM version").fetch_one(&pool).await?;
        assert_eq!(version, 2250, "bareos-dir must still open the catalog");
        check_table(&pool).await?;
        Ok(())
    }).await;
}
