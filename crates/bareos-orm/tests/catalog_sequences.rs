//! Catalog sequences across migrations (Compatibility log C-039).
//!
//! Needs a PostgreSQL database: set `BAREOS_ORM_TEST_DB` to a connection URL
//! (e.g. `postgres:///scratch?host=/tmp/pg`). The test works only inside a
//! schema it creates and drops itself (`search_path` is pinned to it), so
//! nothing outside that schema is touched. Without the variable it is skipped.

use std::str::FromStr;

use bareos_orm::migration::{check_sequences, Migrator};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

#[tokio::test]
async fn migrations_never_move_sequences_and_lags_are_reported() {
    let Ok(url) = std::env::var("BAREOS_ORM_TEST_DB") else {
        eprintln!("BAREOS_ORM_TEST_DB not set, skipping");
        return;
    };
    let schema = format!("onesimus_c039_{}", std::process::id());
    let admin = PgPoolOptions::new().max_connections(1).connect(&url).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}")).execute(&admin).await.unwrap();

    let result = run(&url, &schema).await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE")).execute(&admin).await.unwrap();
    result.unwrap();
}

async fn run(url: &str, schema: &str) -> Result<(), Box<dyn std::error::Error>> {
    let options = PgConnectOptions::from_str(url)?.options([("search_path", schema)]);
    let pool = PgPoolOptions::new().max_connections(1).connect_with(options).await?;
    let migrator = Migrator::new(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema"));

    // Fresh install: the Bareos version table stays at the Bareos version.
    migrator.migrate(&pool).await?;
    let version: i32 = sqlx::query_scalar("SELECT MAX(versionid) FROM version").fetch_one(&pool).await?;
    assert_eq!(version, 2250, "Onesimus extensions must not bump the Bareos VersionId");
    let ours: i32 = sqlx::query_scalar("SELECT MAX(version) FROM bareos_schema_version").fetch_one(&pool).await?;
    assert_eq!(ours, 2252);
    assert!(check_sequences(&pool).await?.is_empty());

    // Normal insert, then rows with explicit ids as a `\copy` would load them.
    let insert = "INSERT INTO job (job, name, type, level, jobstatus) VALUES ('j', 'n', 'B', 'F', 'T')";
    sqlx::query(insert).execute(&pool).await?;
    sqlx::query("INSERT INTO job (jobid, job, name, type, level, jobstatus) VALUES (629, 'old', 'n', 'B', 'F', 'T')")
        .execute(&pool).await?;
    let before: (i64, bool) = sqlx::query_as("SELECT last_value, is_called FROM job_jobid_seq").fetch_one(&pool).await?;

    // Running the migrations again leaves every sequence alone ...
    assert_eq!(migrator.migrate(&pool).await?, 0);
    let after: (i64, bool) = sqlx::query_as("SELECT last_value, is_called FROM job_jobid_seq").fetch_one(&pool).await?;
    assert_eq!(before, after);

    // ... and the lag is reported, not repaired.
    let lags = check_sequences(&pool).await?;
    assert_eq!(lags.len(), 1, "{lags:?}");
    assert_eq!((lags[0].sequence.as_str(), lags[0].next_value, lags[0].max_id), ("job_jobid_seq", 2, 629));

    // The proposed repair SQL makes the next id follow the highest one.
    sqlx::query(&lags[0].repair_sql()).execute(&pool).await?;
    assert!(check_sequences(&pool).await?.is_empty());
    let next: i32 = sqlx::query_scalar(&format!("{insert} RETURNING jobid")).fetch_one(&pool).await?;
    assert_eq!(next, 630);
    Ok(())
}
