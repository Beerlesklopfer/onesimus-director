//! Throw-away databases for the live PostgreSQL tests of this crate.
//!
//! `BAREOS_ORM_TEST_DB` is the connection URL of a throw-away cluster
//! (e.g. `postgres://user@127.0.0.2:55433/postgres`) whose role may create
//! databases and roles. Every case gets a database of its own,
//! `<tag>_<pid>`, created from `template0` and dropped afterwards: the
//! Onesimus objects live in the fixed schema `onesimus` of the catalog
//! database, so cases sharing one database would collide. Inside its
//! database a case sees the catalog as a real installation does (Bareos
//! tables in `public`, default `search_path`).

#![allow(dead_code)]

use std::str::FromStr;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

/// Connection URL of the throw-away cluster (`BAREOS_ORM_TEST_DB`).
pub fn url() -> String {
    std::env::var("BAREOS_ORM_TEST_DB").expect("BAREOS_ORM_TEST_DB not set")
}

/// Connection to the database named in [`url`] (administration: create and
/// drop databases and roles).
pub async fn admin() -> PgPool {
    PgPoolOptions::new().max_connections(1).connect(&url()).await.unwrap()
}

/// A database created for one test case; [`TestDb::drop`] removes it.
pub struct TestDb {
    /// Database name, `<tag>_<pid>`.
    pub name: String,
    /// Administrative connection (database of [`url`]).
    admin: PgPool,
}

impl TestDb {
    /// Create the database `<tag>_<pid>` (an old one of the same name, left
    /// by an aborted run, is dropped first), owned by `owner` if given.
    pub async fn create(tag: &str, owner: Option<&str>) -> TestDb {
        let name = format!("{tag}_{}", std::process::id());
        let admin = admin().await;
        sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).execute(&admin).await.unwrap();
        let owner = owner.map(|o| format!(" OWNER {o}")).unwrap_or_default();
        sqlx::query(&format!("CREATE DATABASE {name} TEMPLATE template0{owner}")).execute(&admin).await.unwrap();
        TestDb { name, admin }
    }

    /// Connection parameters of this database, as the cluster role.
    pub fn options(&self) -> PgConnectOptions {
        PgConnectOptions::from_str(&url()).unwrap().database(&self.name)
    }

    /// A single-connection pool on this database as the cluster role.
    pub async fn pool(&self) -> PgPool {
        PgPoolOptions::new().max_connections(1).connect_with(self.options()).await.unwrap()
    }

    /// A single-connection pool on this database as `role` (the throw-away
    /// cluster trusts every local role).
    pub async fn pool_as(&self, role: &str) -> PgPool {
        PgPoolOptions::new().max_connections(1).connect_with(self.options().username(role)).await.unwrap()
    }

    /// Drop the database (open connections are terminated).
    pub async fn drop(self) {
        sqlx::query(&format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name))
            .execute(&self.admin).await.unwrap();
    }
}

/// Run `case` against a fresh database `<tag>_<pid>` (connected as the
/// cluster role) and drop the database afterwards, also when the case fails.
pub async fn in_database<F, Fut>(tag: &str, case: F)
where
    F: FnOnce(PgPool) -> Fut,
    Fut: std::future::Future<Output = Result<(), Box<dyn std::error::Error>>>,
{
    let db = TestDb::create(tag, None).await;
    let pool = db.pool().await;
    let result = case(pool.clone()).await;
    pool.close().await;
    db.drop().await;
    result.unwrap();
}
