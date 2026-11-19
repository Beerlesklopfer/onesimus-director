//! Schema step v2253: own PostgreSQL schema `onesimus` for every Onesimus
//! catalog object (views and functions of v2251/v2252, the console ACL table
//! `onesimus.console_acl` of the two-tier console ACLs,
//! docs/plans/ACL-ZWEISTUFIG.md); `public` stays pure Bareos.
//!
//! Needs a throw-away PostgreSQL cluster: set `BAREOS_ORM_TEST_DB` to a
//! connection URL whose role may create databases and roles (see
//! `common/mod.rs`). Each case works in a database of its own. Without the
//! variable the tests are skipped (ignored).

mod common;

use std::path::{Path, PathBuf};

use bareos_orm::migration::Migrator;
use common::{in_database, TestDb};

/// The schema directory shipped with the crate.
fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema")
}

/// The views and functions of v2251/v2252 (all shipped Onesimus views and
/// functions), unqualified.
const OBJECTS: &[&str] = &[
    "vw_job_summary", "vw_volume_status", "vw_last_backup_per_client", "vw_file_list",
    "sp_create_job_record", "sp_update_job_end", "fn_find_appendable_volume", "fn_find_latest_backup",
    "sp_create_jobmedia", "fn_since_utime",
    "vw_running_jobs", "vw_media_list",
    "sp_prune_expired_volumes", "sp_prune_expired_jobs", "sp_purge_volume", "fn_director_status",
];

/// A schema directory with what odir migrated before v2253: the Bareos base
/// (v2250) and the v2251/v2252 files as they were then, creating their
/// objects unqualified, i.e. in `public` (tests/data/pre-2253, copied from
/// the files of commit bbb470b).
fn pre_2253_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bareos-orm-pre2253-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(schema_dir().join("v2250_bareos_catalog.json"), dir.join("v2250_bareos_catalog.json")).unwrap();
    let old = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/pre-2253");
    for entry in std::fs::read_dir(old).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, dir.join(path.file_name().unwrap())).unwrap();
    }
    dir
}

/// Bring a catalog to v2252 the way an odir before v2253 did (objects in
/// `public`), as on the production catalog.
async fn migrate_pre_2253(pool: &sqlx::PgPool, tag: &str) -> Result<(), Box<dyn std::error::Error>> {
    let dir = pre_2253_dir(tag);
    let result = Migrator::new(&dir).migrate(pool).await;
    std::fs::remove_dir_all(&dir)?;
    result?;
    assert_eq!(onesimus_objects_in(pool, "public").await?.len(), OBJECTS.len(), "pre-2253 layout");
    Ok(())
}

/// Names of Onesimus views/functions/tables found in schema `schema`.
async fn onesimus_objects_in(pool: &sqlx::PgPool, schema: &str) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT c.relname::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = $1 AND (c.relname = ANY($2) OR c.relname IN ('console_acl', 'onesimus_console_acl')) \
         UNION ALL \
         SELECT p.proname::text FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = $1 AND p.proname = ANY($2) ORDER BY 1")
        .bind(schema).bind(OBJECTS).fetch_all(pool).await
}

/// Columns of `onesimus.console_acl` as `name type not-null`.
async fn acl_columns(pool: &sqlx::PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT column_name||' '||data_type||' '||is_nullable FROM information_schema.columns
         WHERE table_schema = 'onesimus' AND table_name = 'console_acl' ORDER BY ordinal_position")
        .fetch_all(pool).await
}

/// The ACL table: columns, primary key, type check, default of `changed_at`.
async fn check_table(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(acl_columns(pool).await?, [
        "console text NO", "acl_type text NO", "position integer NO", "entry text NO",
        "changed_by text NO", "changed_at timestamp with time zone NO",
    ]);
    sqlx::query("INSERT INTO onesimus.console_acl (console, acl_type, position, entry, changed_by) \
                 VALUES ('c', 'command', 0, '!delete', 'admin')").execute(pool).await?;
    let duplicate = sqlx::query("INSERT INTO onesimus.console_acl (console, acl_type, position, entry, changed_by) \
                 VALUES ('c', 'command', 0, 'status', 'admin')").execute(pool).await;
    assert!(duplicate.is_err(), "primary key (console, acl_type, position)");
    let bad_type = sqlx::query("INSERT INTO onesimus.console_acl (console, acl_type, position, entry, changed_by) \
                 VALUES ('c', 'commandacl', 0, 'status', 'admin')").execute(pool).await;
    assert!(bad_type.is_err(), "acl_type check");
    let recent: bool = sqlx::query_scalar("SELECT changed_at > now() - interval '1 hour' FROM onesimus.console_acl")
        .fetch_one(pool).await?;
    assert!(recent, "changed_at defaults to now()");
    Ok(())
}

/// The end state of every path: all Onesimus objects in `onesimus` (owned
/// by `owner`), none in `public`, Bareos `version` 2250, `bareos_schema_version`
/// in `public` at 2253, the objects usable qualified (and the Bareos tables
/// unqualified as before), the ACL table intact.
async fn check_layout(pool: &sqlx::PgPool, owner: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut expected: Vec<String> = OBJECTS.iter().map(|s| s.to_string()).chain(["console_acl".to_string()]).collect();
    expected.sort();
    assert_eq!(onesimus_objects_in(pool, "onesimus").await?, expected);
    assert!(onesimus_objects_in(pool, "public").await?.is_empty(), "public stays pure Bareos");
    // Views in public: only the operator's own (my_*), the Bareos DDL has none.
    let public_views: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_views WHERE schemaname = 'public' AND viewname NOT LIKE 'my\\_%'")
        .fetch_one(pool).await?;
    assert_eq!(public_views, 0);
    let schema_owner: String = sqlx::query_scalar("SELECT nspowner::regrole::text FROM pg_namespace WHERE nspname = 'onesimus'")
        .fetch_one(pool).await?;
    assert_eq!(schema_owner, owner);
    let version: i32 = sqlx::query_scalar("SELECT MAX(versionid) FROM public.version").fetch_one(pool).await?;
    assert_eq!(version, 2250, "bareos-dir must still open the catalog");
    let ours: i32 = sqlx::query_scalar("SELECT MAX(version) FROM public.bareos_schema_version").fetch_one(pool).await?;
    assert_eq!(ours, 2253);
    let in_onesimus: bool = sqlx::query_scalar("SELECT to_regclass('onesimus.bareos_schema_version') IS NOT NULL")
        .fetch_one(pool).await?;
    assert!(!in_onesimus, "version bookkeeping stays in public");
    // Qualified calls work with the default search_path ("$user", public).
    sqlx::query("INSERT INTO client (name, uname, autoprune, fileretention, jobretention) VALUES ('fd1', '', 1, 1, 1)")
        .execute(pool).await?;
    sqlx::query("INSERT INTO job (job, name, type, level, clientid, jobstatus, jobtdate) \
                 SELECT 'j.1', 'j', 'B', 'F', clientid, 'T', 0 FROM client WHERE name = 'fd1'")
        .execute(pool).await?;
    let latest: Option<i32> = sqlx::query_scalar("SELECT onesimus.fn_find_latest_backup('fd1')").fetch_one(pool).await?;
    assert!(latest.is_some());
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM onesimus.vw_job_summary WHERE clientname = 'fd1'")
        .fetch_one(pool).await?;
    assert_eq!(jobs, 1);
    let status: i64 = sqlx::query_scalar("SELECT total_jobs FROM onesimus.fn_director_status()").fetch_one(pool).await?;
    assert_eq!(status, 1);
    let unqualified = sqlx::query("SELECT * FROM vw_job_summary").execute(pool).await;
    assert!(unqualified.is_err(), "nothing of Onesimus resolves through the default search_path");
    check_table(pool).await
}

/// A fresh catalog: v2251/v2252 create directly in `onesimus` (v2253 moves
/// nothing), the Bareos `version` stays 2250, every step is recorded; a
/// second start is a no-op.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn fresh_catalog_gets_the_onesimus_schema() {
    in_database("ons_fresh", |pool| async move {
        let migrator = Migrator::new(&schema_dir());
        assert_eq!(migrator.migrate(&pool).await?, 4);
        let ours: Vec<i32> = sqlx::query_scalar("SELECT version FROM bareos_schema_version ORDER BY version")
            .fetch_all(&pool).await?;
        assert_eq!(ours, [2250, 2251, 2252, 2253]);
        let user: String = sqlx::query_scalar("SELECT current_user::text").fetch_one(&pool).await?;
        check_layout(&pool, &user).await?;
        assert_eq!(migrator.migrate(&pool).await?, 0);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM onesimus.console_acl").fetch_one(&pool).await?;
        assert_eq!(rows, 1);
        let steps: i64 = sqlx::query_scalar("SELECT count(*) FROM bareos_schema_version").fetch_one(&pool).await?;
        assert_eq!(steps, 4, "a second start records nothing");
        Ok(())
    }).await;
}

/// A catalog at 2252 with the objects in `public`, as an older odir left it
/// (production): the catalog user (no superuser, owner of the database as
/// in a Bareos installation where the catalog user owns its database) runs
/// the step. Afterwards everything is in `onesimus`, nothing of Onesimus in
/// `public`; an operator's own view on `vw_job_summary` keeps working
/// (ALTER ... SET SCHEMA keeps dependencies), grants stay. Second start:
/// no-op.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn catalog_at_2252_moves_its_objects_to_onesimus() {
    let admin = common::admin().await;
    let role = format!("ons_catalog_user_{}", std::process::id());
    let reader = format!("ons_reader_{}", std::process::id());
    for r in [&role, &reader] {
        sqlx::query(&format!("DROP ROLE IF EXISTS {r}")).execute(&admin).await.unwrap();
        sqlx::query(&format!("CREATE ROLE {r} LOGIN")).execute(&admin).await.unwrap();
    }
    let db = TestDb::create("ons_2252", Some(&role)).await;
    let pool = db.pool_as(&role).await;
    let result = async {
        // PostgreSQL 15+: public belongs to pg_database_owner, i.e. the role.
        migrate_pre_2253(&pool, "2252").await?;
        assert_eq!(sqlx::query_scalar::<_, i32>("SELECT MAX(version) FROM bareos_schema_version").fetch_one(&pool).await?, 2252);
        sqlx::query("CREATE VIEW public.my_report AS SELECT jobid, clientname FROM vw_job_summary")
            .execute(&pool).await?;
        sqlx::query(&format!("GRANT SELECT ON vw_job_summary TO {reader}")).execute(&pool).await?;

        let migrator = Migrator::new(&schema_dir());
        assert_eq!(migrator.migrate(&pool).await?, 1);
        check_layout(&pool, &role).await?;
        let report: i64 = sqlx::query_scalar("SELECT count(*) FROM public.my_report").fetch_one(&pool).await?;
        assert_eq!(report, 1, "the operator's view follows the moved view");
        let granted: bool = sqlx::query_scalar(&format!(
            "SELECT has_table_privilege('{reader}', 'onesimus.vw_job_summary', 'SELECT')"))
            .fetch_one(&pool).await?;
        assert!(granted, "grants move with the view");
        assert_eq!(migrator.migrate(&pool).await?, 0);
        assert!(onesimus_objects_in(&pool, "public").await?.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    pool.close().await;
    db.drop().await;
    for r in [&role, &reader] {
        sqlx::query(&format!("DROP ROLE IF EXISTS {r}")).execute(&admin).await.unwrap();
    }
    result.unwrap();
}

/// A catalog created by the Bareos DDL alone (`version` 2250, no
/// `bareos_schema_version`): v2251..v2253 create everything in `onesimus`.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn bareos_ddl_catalog_gets_the_onesimus_schema() {
    in_database("ons_cpp", |pool| async move {
        let ddl = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/bareos-18be77e6-creates-postgresql.sql"))?;
        sqlx::raw_sql(&ddl).execute(&pool).await?;
        let migrator = Migrator::new(&schema_dir());
        assert_eq!(migrator.migrate(&pool).await?, 3);
        let user: String = sqlx::query_scalar("SELECT current_user::text").fetch_one(&pool).await?;
        check_layout(&pool, &user).await?;
        assert_eq!(migrator.migrate(&pool).await?, 0);
        Ok(())
    }).await;
}

/// A step interrupted after some objects moved, or an object created in
/// `public` again by hand: the step completes, the copy in `public` goes,
/// the one in `onesimus` stays.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn partly_moved_catalog_is_completed() {
    in_database("ons_partly", |pool| async move {
        migrate_pre_2253(&pool, "partly").await?;
        sqlx::query("CREATE SCHEMA onesimus").execute(&pool).await?;
        sqlx::query("ALTER VIEW public.vw_media_list SET SCHEMA onesimus").execute(&pool).await?;
        sqlx::query("ALTER FUNCTION public.sp_purge_volume(text) SET SCHEMA onesimus").execute(&pool).await?;
        // Both schemas have these two:
        let definition: String = sqlx::query_scalar("SELECT pg_get_viewdef('public.vw_job_summary'::regclass)")
            .fetch_one(&pool).await?;
        sqlx::query(&format!("CREATE VIEW onesimus.vw_job_summary AS {definition}")).execute(&pool).await?;
        sqlx::query("CREATE FUNCTION onesimus.fn_director_status() RETURNS TABLE(total_jobs bigint, running bigint, \
                     successful bigint, errors bigint, total_bytes bigint, total_files bigint) AS $$ SELECT 0::bigint, \
                     0::bigint, 0::bigint, 0::bigint, 0::bigint, 0::bigint $$ LANGUAGE sql").execute(&pool).await?;
        assert_eq!(Migrator::new(&schema_dir()).migrate(&pool).await?, 1);
        let user: String = sqlx::query_scalar("SELECT current_user::text").fetch_one(&pool).await?;
        // check_layout also proves the stub fn_director_status was replaced
        // by the shipped definition (total_jobs counts the job).
        check_layout(&pool, &user).await
    }).await;
}

/// Privileges: without CREATE on the database the schema cannot be
/// created; the migration fails, records nothing and leaves the objects in
/// `public` (odir runs on, without the extension, and retries at the next
/// start). Once an administrator creates the schema for the catalog user
/// (`CREATE SCHEMA onesimus AUTHORIZATION <user>`), the step succeeds.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn missing_create_privilege_fails_without_recording() {
    let admin = common::admin().await;
    let role = format!("ons_noncreate_{}", std::process::id());
    sqlx::query(&format!("DROP ROLE IF EXISTS {role}")).execute(&admin).await.unwrap();
    sqlx::query(&format!("CREATE ROLE {role} LOGIN")).execute(&admin).await.unwrap();
    // Database owned by the cluster role (as `create_bareos_database` run as
    // postgres does); the catalog user only gets rights in public.
    let db = TestDb::create("ons_priv", None).await;
    let owner_pool = db.pool().await;
    sqlx::query(&format!("GRANT CREATE, USAGE ON SCHEMA public TO {role}")).execute(&owner_pool).await.unwrap();
    let pool = db.pool_as(&role).await;
    let result = async {
        migrate_pre_2253(&pool, "priv").await?;
        let migrator = Migrator::new(&schema_dir());
        let error = migrator.migrate(&pool).await.expect_err("no CREATE on the database");
        assert!(error.to_string().contains("cannot create schema onesimus"), "{error}");
        assert_eq!(sqlx::query_scalar::<_, i32>("SELECT MAX(version) FROM bareos_schema_version").fetch_one(&pool).await?, 2252);
        assert_eq!(onesimus_objects_in(&pool, "public").await?.len(), OBJECTS.len());

        sqlx::query(&format!("CREATE SCHEMA onesimus AUTHORIZATION {role}")).execute(&owner_pool).await?;
        assert_eq!(migrator.migrate(&pool).await?, 1);
        check_layout(&pool, &role).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    pool.close().await;
    owner_pool.close().await;
    db.drop().await;
    sqlx::query(&format!("DROP ROLE IF EXISTS {role}")).execute(&admin).await.unwrap();
    result.unwrap();
}
