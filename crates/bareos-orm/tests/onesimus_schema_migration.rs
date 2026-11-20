//! Schema step v2253: own PostgreSQL schema `onesimus` for every Onesimus
//! catalog object (views and functions of v2251/v2252, the console ACL table
//! `onesimus.console_acl` of the two-tier console ACLs,
//! docs/plans/ACL-ZWEISTUFIG.md); `public` stays pure Bareos. Schema step
//! v2254: late-bound views (no Onesimus object depends on a Bareos table, so
//! Bareos upgrades are never blocked), and the self-healing of
//! [`Migrator::heal`] at every start.
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

/// The newest shipped schema step.
const LATEST: i32 = 2254;

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
    check_layout_at(pool, owner, 2250).await
}

/// [`check_layout`] for a catalog whose Bareos `version` is `bareos`.
async fn check_layout_at(pool: &sqlx::PgPool, owner: &str, bareos: i32) -> Result<(), Box<dyn std::error::Error>> {
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
    assert_eq!(version, bareos, "bareos-dir must still open the catalog");
    let ours: i32 = sqlx::query_scalar("SELECT MAX(version) FROM public.bareos_schema_version").fetch_one(pool).await?;
    assert_eq!(ours, LATEST);
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
    check_late_bound(pool).await?;
    let report = Migrator::new(&schema_dir()).heal(pool).await;
    assert_eq!(report.skipped, None);
    assert!(report.repaired.is_empty() && report.failed.is_empty(), "a healthy catalog needs no repair: {report:?}");
    assert_eq!(report.checked, 1 + 1 + 16 + 6, "schema, table, functions, views");
    check_table(pool).await
}

/// Every column of the Onesimus views as `view|position|name|type`, exactly
/// as v2251/v2252 created them (with typmod): v2254 must not change what
/// callers see.
const VIEW_COLUMNS: &[&str] = &[
    "vw_file_list|1|fileid|bigint", "vw_file_list|2|jobid|integer", "vw_file_list|3|filepath|text",
    "vw_file_list|4|fileindex|integer", "vw_file_list|5|markid|integer",
    "vw_job_summary|1|jobid|integer", "vw_job_summary|2|job|text", "vw_job_summary|3|jobname|text",
    "vw_job_summary|4|type|character(1)", "vw_job_summary|5|level|character(1)", "vw_job_summary|6|jobstatus|character(1)",
    "vw_job_summary|7|schedtime|timestamp without time zone", "vw_job_summary|8|starttime|timestamp without time zone",
    "vw_job_summary|9|endtime|timestamp without time zone", "vw_job_summary|10|jobfiles|integer",
    "vw_job_summary|11|jobbytes|bigint", "vw_job_summary|12|joberrors|integer", "vw_job_summary|13|volsessionid|integer",
    "vw_job_summary|14|volsessiontime|integer", "vw_job_summary|15|clientname|text", "vw_job_summary|16|poolname|text",
    "vw_last_backup_per_client|1|clientname|text", "vw_last_backup_per_client|2|jobid|integer",
    "vw_last_backup_per_client|3|jobname|text", "vw_last_backup_per_client|4|level|character(1)",
    "vw_last_backup_per_client|5|starttime|timestamp without time zone",
    "vw_last_backup_per_client|6|endtime|timestamp without time zone", "vw_last_backup_per_client|7|jobfiles|integer",
    "vw_last_backup_per_client|8|jobbytes|bigint",
    "vw_media_list|1|mediaid|integer", "vw_media_list|2|volumename|text", "vw_media_list|3|volstatus|text",
    "vw_media_list|4|mediatype|text", "vw_media_list|5|volbytes|bigint", "vw_media_list|6|voljobs|bigint",
    "vw_media_list|7|volfiles|bigint", "vw_media_list|8|maxvolbytes|bigint", "vw_media_list|9|poolname|text",
    "vw_running_jobs|1|jobid|integer", "vw_running_jobs|2|job|text", "vw_running_jobs|3|jobname|text",
    "vw_running_jobs|4|type|character(1)", "vw_running_jobs|5|level|character(1)", "vw_running_jobs|6|jobstatus|character(1)",
    "vw_running_jobs|7|starttime|timestamp without time zone", "vw_running_jobs|8|jobfiles|integer",
    "vw_running_jobs|9|jobbytes|bigint", "vw_running_jobs|10|joberrors|integer", "vw_running_jobs|11|clientname|text",
    "vw_running_jobs|12|volsessionid|integer", "vw_running_jobs|13|volsessiontime|integer",
    "vw_volume_status|1|mediaid|integer", "vw_volume_status|2|volumename|text", "vw_volume_status|3|volstatus|text",
    "vw_volume_status|4|volbytes|bigint", "vw_volume_status|5|voljobs|bigint", "vw_volume_status|6|volfiles|bigint",
    "vw_volume_status|7|volmounts|bigint", "vw_volume_status|8|volerrors|bigint", "vw_volume_status|9|maxvolbytes|bigint",
    "vw_volume_status|10|mediatype|text", "vw_volume_status|11|slot|bigint", "vw_volume_status|12|poolname|text",
    "vw_volume_status|13|pct_full|numeric",
];

/// Columns of the views in schema `onesimus` (format of [`VIEW_COLUMNS`]).
async fn view_columns(pool: &sqlx::PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT c.relname || '|' || a.attnum || '|' || a.attname || '|' || format_type(a.atttypid, a.atttypmod) \
        FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
        WHERE n.nspname = 'onesimus' AND c.relkind = 'v' AND a.attnum > 0 ORDER BY c.relname, a.attnum")
        .fetch_all(pool).await
}

/// Every dependency recorded in `pg_depend` from an object of schema
/// `onesimus` (relation, view rule, function, type, constraint, default)
/// on a relation, type or function of `public`, described by PostgreSQL.
/// Empty: Bareos may alter or drop any of its tables and columns.
async fn public_dependencies(pool: &sqlx::PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "WITH dep AS (SELECT d.*, \
        CASE d.classid \
        WHEN 'pg_class'::regclass THEN (SELECT relnamespace FROM pg_class WHERE oid = d.objid) \
        WHEN 'pg_rewrite'::regclass THEN (SELECT c.relnamespace FROM pg_rewrite r JOIN pg_class c ON c.oid = r.ev_class WHERE r.oid = d.objid) \
        WHEN 'pg_proc'::regclass THEN (SELECT pronamespace FROM pg_proc WHERE oid = d.objid) \
        WHEN 'pg_type'::regclass THEN (SELECT typnamespace FROM pg_type WHERE oid = d.objid) \
        WHEN 'pg_constraint'::regclass THEN (SELECT connamespace FROM pg_constraint WHERE oid = d.objid) \
        WHEN 'pg_attrdef'::regclass THEN (SELECT c.relnamespace FROM pg_attrdef a JOIN pg_class c ON c.oid = a.adrelid WHERE a.oid = d.objid) \
        END AS dep_ns, \
        CASE d.refclassid \
        WHEN 'pg_class'::regclass THEN (SELECT relnamespace FROM pg_class WHERE oid = d.refobjid) \
        WHEN 'pg_proc'::regclass THEN (SELECT pronamespace FROM pg_proc WHERE oid = d.refobjid) \
        WHEN 'pg_type'::regclass THEN (SELECT typnamespace FROM pg_type WHERE oid = d.refobjid) \
        END AS ref_ns FROM pg_depend d) \
        SELECT pg_describe_object(classid, objid, objsubid) || ' -> ' || pg_describe_object(refclassid, refobjid, refobjsubid) \
        FROM dep WHERE dep_ns = 'onesimus'::regnamespace AND ref_ns = 'public'::regnamespace ORDER BY 1")
        .fetch_all(pool).await
}

/// v2254: no Onesimus object depends on a Bareos table, each view reads
/// only its function, and the view columns are unchanged.
async fn check_late_bound(pool: &sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(public_dependencies(pool).await?, Vec::<String>::new());
    assert_eq!(view_columns(pool).await?, VIEW_COLUMNS);
    let reads: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT c.relname || ' -> ' || CASE d.refclassid \
           WHEN 'pg_proc'::regclass THEN 'function ' || d.refobjid::regprocedure::text \
           WHEN 'pg_class'::regclass THEN 'relation ' || d.refobjid::regclass::text \
           ELSE d.refclassid::regclass::text END \
        FROM pg_rewrite r JOIN pg_class c ON c.oid = r.ev_class JOIN pg_namespace n ON n.oid = c.relnamespace \
        JOIN pg_depend d ON d.classid = 'pg_rewrite'::regclass AND d.objid = r.oid AND d.refobjid <> c.oid \
        WHERE n.nspname = 'onesimus' ORDER BY 1").fetch_all(pool).await?;
    let expected: Vec<String> = ["vw_file_list", "vw_job_summary", "vw_last_backup_per_client", "vw_media_list",
        "vw_running_jobs", "vw_volume_status"].iter()
        .map(|v| format!("{v} -> function onesimus.fn_{v}()")).collect();
    assert_eq!(reads, expected, "each view depends on its function only");
    Ok(())
}

/// A fresh catalog: v2251/v2252 create directly in `onesimus` (v2253 moves
/// nothing, v2254 makes the views late-bound), the Bareos `version` stays
/// 2250, every step is recorded; a second start is a no-op.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn fresh_catalog_gets_the_onesimus_schema() {
    in_database("ons_fresh", |pool| async move {
        let migrator = Migrator::new(&schema_dir());
        assert_eq!(migrator.migrate(&pool).await?, 5);
        let ours: Vec<i32> = sqlx::query_scalar("SELECT version FROM bareos_schema_version ORDER BY version")
            .fetch_all(&pool).await?;
        assert_eq!(ours, [2250, 2251, 2252, 2253, 2254]);
        let user: String = sqlx::query_scalar("SELECT current_user::text").fetch_one(&pool).await?;
        check_layout(&pool, &user).await?;
        assert_eq!(migrator.migrate(&pool).await?, 0);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM onesimus.console_acl").fetch_one(&pool).await?;
        assert_eq!(rows, 1);
        let steps: i64 = sqlx::query_scalar("SELECT count(*) FROM bareos_schema_version").fetch_one(&pool).await?;
        assert_eq!(steps, 5, "a second start records nothing");
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
        assert_eq!(migrator.migrate(&pool).await?, 2, "v2253 and v2254");
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
/// `bareos_schema_version`): v2251..v2254 create everything in `onesimus`.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn bareos_ddl_catalog_gets_the_onesimus_schema() {
    in_database("ons_cpp", |pool| async move {
        let ddl = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/bareos-18be77e6-creates-postgresql.sql"))?;
        sqlx::raw_sql(&ddl).execute(&pool).await?;
        let migrator = Migrator::new(&schema_dir());
        assert_eq!(migrator.migrate(&pool).await?, 4);
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
        assert_eq!(Migrator::new(&schema_dir()).migrate(&pool).await?, 2);
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
        assert_eq!(migrator.migrate(&pool).await?, 2);
        check_layout(&pool, &role).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    pool.close().await;
    owner_pool.close().await;
    db.drop().await;
    sqlx::query(&format!("DROP ROLE IF EXISTS {role}")).execute(&admin).await.unwrap();
    result.unwrap();
}

/// One client and one successful backup job of it, for the view probes.
async fn insert_job(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO client (name, uname, autoprune, fileretention, jobretention) VALUES ('fd9', '', 1, 1, 1)")
        .execute(pool).await?;
    sqlx::query("INSERT INTO job (job, name, type, level, clientid, jobstatus, jobtdate, jobfiles) \
                 SELECT 'j.9', 'j', 'B', 'F', clientid, 'T', 0, 7 FROM client WHERE name = 'fd9'")
        .execute(pool).await?;
    Ok(())
}

/// The DDL of a Bareos upgrade (`update_bareos_tables`) on a production-like
/// catalog (v2252 with the objects in `public`, migrated through v2253 and
/// v2254): ALTER COLUMN TYPE and DROP COLUMN on columns the views read,
/// DROP TABLE of tables they read — all succeed with the extension
/// installed. A changed type is absorbed by the casts (view columns stay);
/// a dropped column only breaks the functions reading it (self-heal reports
/// them, cannot repair, never fails), and they work again once the column
/// is back.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn bareos_upgrade_ddl_is_not_blocked_by_the_extension() {
    in_database("ons_upgrade", |pool| async move {
        migrate_pre_2253(&pool, "upgrade").await?;
        let migrator = Migrator::new(&schema_dir());
        assert_eq!(migrator.migrate(&pool).await?, 2);
        check_late_bound(&pool).await?;
        insert_job(&pool).await?;

        // Type change of a column every job view reads.
        sqlx::query("ALTER TABLE public.job ALTER COLUMN jobfiles TYPE bigint").execute(&pool).await?;
        sqlx::query("ALTER TABLE public.media ALTER COLUMN volbytes TYPE numeric(20)").execute(&pool).await?;
        let files: i32 = sqlx::query_scalar("SELECT jobfiles FROM onesimus.vw_job_summary WHERE clientname = 'fd9'")
            .fetch_one(&pool).await?;
        assert_eq!(files, 7);
        sqlx::query("SELECT * FROM onesimus.vw_volume_status").execute(&pool).await?;
        assert_eq!(view_columns(&pool).await?, VIEW_COLUMNS, "the casts keep the view columns");
        let report = migrator.heal(&pool).await;
        assert!(report.repaired.is_empty() && report.failed.is_empty(), "{report:?}");

        // DROP COLUMN of a column no view reads, and DROP TABLE of tables
        // views read (in a transaction rolled back: the test needs them).
        sqlx::query("ALTER TABLE public.job DROP COLUMN comment").execute(&pool).await?;
        let mut tx = pool.begin().await?;
        sqlx::query("DROP TABLE public.file").execute(&mut *tx).await?;
        sqlx::query("DROP TABLE public.path").execute(&mut *tx).await?;
        sqlx::query("ALTER TABLE public.job DROP COLUMN jobstatus").execute(&mut *tx).await?;
        tx.rollback().await?;

        // DROP COLUMN of a column two views read: not blocked; the functions
        // reading it fail until the column is back.
        sqlx::query("ALTER TABLE public.job DROP COLUMN joberrors").execute(&pool).await?;
        assert!(sqlx::query("SELECT * FROM onesimus.vw_job_summary").execute(&pool).await.is_err());
        let report = migrator.heal(&pool).await;
        assert_eq!(report.skipped, None);
        assert!(report.repaired.is_empty(), "{report:?}");
        let failed = report.failed.join("\n");
        for object in ["function onesimus.fn_vw_job_summary", "function onesimus.fn_vw_running_jobs",
                       "view onesimus.vw_job_summary", "view onesimus.vw_running_jobs"] {
            assert!(failed.contains(object), "{object} in {failed}");
        }
        assert_eq!(report.failed.len(), 4, "{failed}");
        let summary: Option<i32> = sqlx::query_scalar("SELECT to_regprocedure('onesimus.fn_vw_job_summary()')::oid::int")
            .fetch_one(&pool).await?;
        assert!(summary.is_some(), "a failed repair leaves the function in place");
        sqlx::query("ALTER TABLE public.job ADD COLUMN joberrors integer NOT NULL DEFAULT 0").execute(&pool).await?;
        let report = migrator.heal(&pool).await;
        assert!(report.repaired.is_empty() && report.failed.is_empty(), "{report:?}");
        let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM onesimus.vw_job_summary").fetch_one(&pool).await?;
        assert_eq!(jobs, 1);
        check_late_bound(&pool).await
    }).await;
}

/// Self-heal at start: dropped views, functions and the ACL table come
/// back, an edited function body and a changed result type are replaced
/// (DROP + CREATE together with the shipped view depending on it), a view
/// put back in its old direct form is made late-bound again. An operator's
/// own view on a function that needs DROP blocks only that repair. A second
/// run changes nothing.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn self_heal_restores_missing_and_broken_objects() {
    in_database("ons_heal", |pool| async move {
        let migrator = Migrator::new(&schema_dir());
        migrator.migrate(&pool).await?;
        let user: String = sqlx::query_scalar("SELECT current_user::text").fetch_one(&pool).await?;
        for sql in [
            "DROP VIEW onesimus.vw_file_list",
            "DROP FUNCTION onesimus.fn_vw_volume_status() CASCADE",
            "DROP FUNCTION onesimus.sp_purge_volume(text)",
            "DROP TABLE onesimus.console_acl",
            "CREATE OR REPLACE FUNCTION onesimus.fn_director_status() RETURNS TABLE(total_jobs bigint, running bigint, \
             successful bigint, errors bigint, total_bytes bigint, total_files bigint) AS $$ SELECT 0::bigint, 0::bigint, \
             0::bigint, 0::bigint, 0::bigint, 0::bigint $$ LANGUAGE sql",
            // Result type changed, with the shipped view on it.
            "DROP VIEW onesimus.vw_media_list",
            "DROP FUNCTION onesimus.fn_vw_media_list()",
            "CREATE FUNCTION onesimus.fn_vw_media_list() RETURNS TABLE(mediaid integer) AS $$ SELECT 1 $$ LANGUAGE sql",
            "CREATE VIEW onesimus.vw_media_list AS SELECT mediaid FROM onesimus.fn_vw_media_list()",
            // The pre-v2254 direct form (same columns).
            "CREATE OR REPLACE VIEW onesimus.vw_last_backup_per_client AS SELECT DISTINCT ON (c.name) c.name AS clientname, \
             j.jobid, j.name AS jobname, j.level, j.starttime, j.endtime, j.jobfiles, j.jobbytes FROM job j \
             JOIN client c ON j.clientid = c.clientid WHERE j.type = 'B' AND j.jobstatus = 'T' ORDER BY c.name, j.starttime DESC",
            // Result type changed, an operator's view on it.
            "DROP FUNCTION onesimus.fn_find_latest_backup(text)",
            "CREATE FUNCTION onesimus.fn_find_latest_backup(p_client_name text) RETURNS bigint AS $$ SELECT 1::bigint $$ LANGUAGE sql",
            "CREATE VIEW public.my_latest AS SELECT onesimus.fn_find_latest_backup('fd9') AS id",
        ] {
            sqlx::query(sql).execute(&pool).await?;
        }
        assert!(!public_dependencies(&pool).await?.is_empty(), "the direct view form depends on public tables");

        let report = migrator.heal(&pool).await;
        let repaired = report.repaired.join("\n");
        for line in [
            "table onesimus.console_acl: missing",
            "function onesimus.fn_vw_volume_status: missing",
            "function onesimus.sp_purge_volume: missing",
            "function onesimus.fn_director_status: body differs from the shipped definition",
            "function onesimus.fn_vw_media_list: returns TABLE(mediaid integer), shipped TABLE(mediaid integer, volumename text",
            "view onesimus.vw_file_list: missing",
            "view onesimus.vw_volume_status: missing",
            "view onesimus.vw_last_backup_per_client: reads public.client, public.job directly (blocks Bareos upgrades)",
        ] {
            assert!(repaired.contains(line), "{line} in\n{repaired}");
        }
        assert_eq!(report.repaired.len(), 8, "{repaired}");
        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert!(report.failed[0].starts_with("function onesimus.fn_find_latest_backup: returns bigint, shipped integer:")
            && report.failed[0].contains("public.my_latest (not shipped) depends on it"), "{}", report.failed[0]);

        sqlx::query("DROP VIEW public.my_latest").execute(&pool).await?;
        let report = migrator.heal(&pool).await;
        assert_eq!(report.repaired, ["function onesimus.fn_find_latest_backup: returns bigint, shipped integer"]);
        assert!(report.failed.is_empty());
        let report = migrator.heal(&pool).await;
        assert!(report.repaired.is_empty() && report.failed.is_empty(), "second run: {report:?}");
        check_layout(&pool, &user).await
    }).await;
}

/// Self-heal only touches a catalog at exactly the shipped step: below, the
/// migration is incomplete; above, a newer odir owns the definitions.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn self_heal_skips_catalogs_at_another_step() {
    in_database("ons_heal_skip", |pool| async move {
        migrate_pre_2253(&pool, "heal-skip").await?;
        let migrator = Migrator::new(&schema_dir());
        let report = migrator.heal(&pool).await;
        assert!(report.skipped.as_deref().is_some_and(|s| s.contains("migration incomplete")), "{report:?}");
        assert_eq!(report.checked, 0);
        migrator.migrate(&pool).await?;
        sqlx::query("DROP VIEW onesimus.vw_file_list").execute(&pool).await?;
        sqlx::query("INSERT INTO bareos_schema_version (version) VALUES ($1)").bind(LATEST + 1).execute(&pool).await?;
        let report = migrator.heal(&pool).await;
        assert!(report.skipped.as_deref().is_some_and(|s| s.contains("newer odir")), "{report:?}");
        let gone: bool = sqlx::query_scalar("SELECT to_regclass('onesimus.vw_file_list') IS NULL").fetch_one(&pool).await?;
        assert!(gone, "not touched");
        Ok(())
    }).await;
}

/// Version scheme: Bareos upgrades the catalog (`version` 2260, its own
/// update script) before odir ever ran v2253/v2254. The old rule (highest
/// of Bareos `version` and `bareos_schema_version`) took the catalog for
/// complete and skipped both steps silently; every Onesimus step is now
/// checked on its own. The Bareos `version` stays 2260, self-heal checks
/// the catalog (a Bareos version above the shipped steps is no "newer
/// odir").
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn bareos_upgrade_before_odir_never_skips_onesimus_steps() {
    in_database("ons_bareos2260", |pool| async move {
        migrate_pre_2253(&pool, "bareos2260").await?;
        sqlx::query("UPDATE version SET versionid = 2260").execute(&pool).await?;
        let migrator = Migrator::new(&schema_dir());
        assert_eq!(migrator.migrate(&pool).await?, 2, "v2253 and v2254 despite Bareos 2260");
        let user: String = sqlx::query_scalar("SELECT current_user::text").fetch_one(&pool).await?;
        check_layout_at(&pool, &user, 2260).await?;
        assert_eq!(migrator.migrate(&pool).await?, 0);
        let report = migrator.heal(&pool).await;
        assert_eq!(report.skipped, None, "{report:?}");
        assert!(report.failed.is_empty() && report.repaired.is_empty(), "{report:?}");
        Ok(())
    }).await;
}

/// Catalog objects of schema `onesimus` with their definitions: columns
/// of tables and views, function source and result, view definitions,
/// constraints and indexes.
async fn onesimus_catalog(pool: &sqlx::PgPool) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT 'column ' || table_name || '.' || column_name || ' ' || data_type || ' ' || is_nullable \
                || ' ' || coalesce(column_default, '') \
           FROM information_schema.columns WHERE table_schema = 'onesimus' \
         UNION ALL SELECT 'function ' || p.oid::regprocedure::text || ' ' || pg_get_function_result(p.oid) \
                || ' ' || p.prosrc || ' ' || p.provolatile::text || p.prosecdef::text \
           FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'onesimus' \
         UNION ALL SELECT 'view ' || c.relname || ' ' || pg_get_viewdef(c.oid) \
           FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
          WHERE n.nspname = 'onesimus' AND c.relkind IN ('v', 'm') \
         UNION ALL SELECT 'constraint ' || conname || ' ' || pg_get_constraintdef(oid) \
           FROM pg_constraint WHERE connamespace = 'onesimus'::regnamespace \
         UNION ALL SELECT 'index ' || indexdef FROM pg_indexes WHERE schemaname = 'onesimus' \
         ORDER BY 1").fetch_all(pool).await
}

/// The whole DDL of every schema file is applied, whichever way a catalog
/// gets there: a fresh install (every step's full statements), a catalog
/// the Bareos DDL created (incremental steps v2251..v2254) and the
/// production path (pre-2253 odir, then v2253/v2254) end with the same
/// objects in `onesimus`, column for column and definition for definition.
#[tokio::test]
#[ignore = "needs BAREOS_ORM_TEST_DB=<postgres url of a throw-away cluster>"]
async fn every_migration_path_ends_with_the_same_catalog() {
    let mut results = Vec::new();
    for path in ["fresh", "bareos_ddl", "pre2253"] {
        let db = TestDb::create(&format!("ons_path_{path}"), None).await;
        let pool = db.pool().await;
        let result = async {
            match path {
                "bareos_ddl" => {
                    let ddl = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/data/bareos-18be77e6-creates-postgresql.sql"))?;
                    sqlx::raw_sql(&ddl).execute(&pool).await?;
                }
                "pre2253" => migrate_pre_2253(&pool, "paths").await?,
                _ => {}
            }
            Migrator::new(&schema_dir()).migrate(&pool).await?;
            Ok::<_, Box<dyn std::error::Error>>(onesimus_catalog(&pool).await?)
        }.await;
        pool.close().await;
        db.drop().await;
        results.push((path, result.unwrap()));
    }
    let (_, fresh) = &results[0];
    assert!(fresh.iter().filter(|l| l.starts_with("view ")).count() == 6, "{fresh:#?}");
    for (path, catalog) in &results[1..] {
        let only_fresh: Vec<_> = fresh.iter().filter(|l| !catalog.contains(l)).collect();
        let only_here: Vec<_> = catalog.iter().filter(|l| !fresh.contains(l)).collect();
        assert!(only_fresh.is_empty() && only_here.is_empty(),
            "{path} differs from fresh:\n only fresh: {only_fresh:#?}\n only {path}: {only_here:#?}");
    }
}
