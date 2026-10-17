//! Startup part of C++ CheckCatalog() (dird/check_catalog.cc:118-162):
//! every Client of the catalog and every Storage (with its MediaType) gets
//! its basic catalog record, as upstream does at Director start. Pools,
//! counters and the job-record cleanup of CheckCatalog() are not part of
//! this (compatibility log C-050: the NDMP parity fixtures add clients and
//! storages that only exist in the catalog through this step).

use crate::catalog_worker::CatalogHandle;
use crate::director_config::DirectorConfig;

fn esc(text: &str) -> String {
    text.replace('\'', "''")
}

/// postgres::try_query() (cats/postgresql.cc:88) with Reconnect = yes (the
/// default): a failed statement is sent once more after PQreset(). For an
/// INSERT that collides with an existing key because the id sequence lags
/// behind (as in the parity catalog template), the second attempt takes
/// the next sequence value and succeeds.
fn execute(catalog: &CatalogHandle, sql: &str) -> Result<u64, String> {
    catalog.execute(sql).or_else(|_| catalog.execute(sql))
}

fn first_column(catalog: &CatalogHandle, sql: &str) -> Result<Vec<Option<String>>, String> {
    Ok(catalog.query_rows(sql)?.rows.into_iter().map(|row| row.into_iter().next().flatten()).collect())
}

/// BareosDb::CreateClientRecord() (sql_create.cc:484) with a fresh
/// ClientDbRecord: Uname '', AutoPrune 0, retentions 0.
fn create_client_record(catalog: &CatalogHandle, name: &str) -> Result<(), String> {
    if !first_column(catalog, &format!("SELECT ClientId,Uname FROM Client WHERE Name='{}'", esc(name)))?.is_empty() {
        return Ok(());
    }
    execute(catalog, &format!(
        "INSERT INTO Client (Name,Uname,AutoPrune,FileRetention,JobRetention) VALUES ('{}','',0,0,0)", esc(name)))?;
    Ok(())
}

/// BareosDb::CreateMediatypeRecord() (sql_create.cc:340), ReadOnly 0.
fn create_mediatype_record(catalog: &CatalogHandle, media_type: &str) -> Result<(), String> {
    let select = format!("SELECT MediaTypeId,MediaType FROM MediaType WHERE MediaType='{}'", esc(media_type));
    if !first_column(catalog, &select)?.is_empty() {
        return Ok(());
    }
    execute(catalog, &format!("INSERT INTO MediaType (MediaType,ReadOnly) VALUES ('{}',0)", esc(media_type)))?;
    Ok(())
}

/// BareosDb::CreateStorageRecord() (sql_create.cc:281), else
/// UpdateStorageRecord() with the configured AutoChanger.
fn create_storage_record(catalog: &CatalogHandle, name: &str, autochanger: bool) -> Result<(), String> {
    let ids = first_column(catalog, &format!("SELECT StorageId,AutoChanger FROM Storage WHERE Name='{}'", esc(name)))?;
    match ids.first() {
        None => {
            execute(catalog, &format!("INSERT INTO Storage (Name,AutoChanger) VALUES ('{}',{})",
                esc(name), autochanger as i32))?;
        }
        Some(id) => {
            let id = id.clone().unwrap_or_default();
            execute(catalog, &format!("UPDATE Storage SET AutoChanger={} WHERE StorageId={id}", autochanger as i32))?;
        }
    }
    Ok(())
}

/// Clients of the (first) catalog, then storages, in resource order.
pub fn ensure_basic_records(config: &DirectorConfig, catalog: &CatalogHandle) {
    let Some(first) = config.catalogs.first() else { return };
    // Pools of this catalog: CreatePool(POOL_OP_UPDATE), then
    // UpdatePoolReferences (C-066: every pool record is rewritten at start).
    let pools: Vec<_> = config.pools.iter().filter(|pool| {
        crate::commands::defaults::raw(config, "pool", &pool.name)
            .and_then(|r| crate::commands::defaults::first(&r.directives, "catalog"))
            .is_none_or(|catalog| catalog == first.name)
    }).filter_map(|pool| crate::commands::pool_db::pool_settings(config, &pool.name)).collect();
    let (mut errmsg, mut msgs) = (String::new(), Vec::new());
    let mut cat = crate::catreq::Cat::new(catalog, &mut errmsg, &mut msgs);
    for pool in &pools {
        crate::commands::pool_db::create_pool(&mut cat, pool, false);
    }
    for pool in &pools {
        crate::commands::pool_db::update_pool_references(&mut cat, pool);
    }
    drop(cat);
    for client in &config.clients {
        // client->catalog defaults to the first Catalog resource.
        if !client.catalog.is_empty() && client.catalog != first.name {
            continue;
        }
        if let Err(error) = create_client_record(catalog, &client.name) {
            tracing::warn!("could not create client record for {}: {error}", client.name);
        }
    }
    for store in &config.storages {
        if !store.media_type.is_empty() {
            if let Err(error) = create_mediatype_record(catalog, &store.media_type) {
                tracing::warn!("could not create mediatype record {}: {error}", store.media_type);
            }
        }
        let autochanger = crate::ua_select::is_autochanger(config, &store.name);
        if let Err(error) = create_storage_record(catalog, &store.name, autochanger) {
            tracing::warn!("Could not create storage record for {}: {error}", store.name);
        }
    }
}

type CatalogResource = bareos_config::resources::CatalogResource;

/// Connection target of a Catalog resource as the Director connects to it
/// (main.rs): an empty/local address means the Unix socket directory.
fn uses_socket(cat: &CatalogResource) -> bool {
    cat.db_address.is_empty() || cat.db_address == "localhost" || cat.db_address == "127.0.0.1"
}

const SOCKET_DIR: &str = "/var/run/postgresql";

fn connect_options(cat: &CatalogResource) -> sqlx::postgres::PgConnectOptions {
    let mut options = sqlx::postgres::PgConnectOptions::new_without_pgpass()
        .port(cat.db_port)
        .database(&cat.db_name);
    options = if uses_socket(cat) {
        options.socket(SOCKET_DIR)
    } else {
        options.host(&cat.db_address)
    };
    if !cat.db_user.is_empty() {
        options = options.username(&cat.db_user);
    }
    if !cat.db_password.is_empty() {
        options = options.password(&cat.db_password);
    }
    options
}

/// PQerrorMessage() of a failed PQconnectdbParams(), including its
/// trailing newline (libpq 15+ wording).
fn libpq_error(cat: &CatalogResource, error: &sqlx::Error) -> String {
    let target = if uses_socket(cat) {
        format!("on socket \"{SOCKET_DIR}/.s.PGSQL.{}\"", cat.db_port)
    } else {
        use std::net::ToSocketAddrs;
        match (cat.db_address.as_str(), cat.db_port).to_socket_addrs().ok().and_then(|mut a| a.next()) {
            Some(addr) => format!("at \"{}\" ({}), port {}", cat.db_address, addr.ip(), cat.db_port),
            None => format!("at \"{}\", port {}", cat.db_address, cat.db_port),
        }
    };
    let reason = match error {
        sqlx::Error::Database(db) => {
            use sqlx::postgres::PgSeverity;
            let severity = db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                .map(|pg| match pg.severity() {
                    PgSeverity::Panic => "PANIC",
                    PgSeverity::Fatal => "FATAL",
                    PgSeverity::Error => "ERROR",
                    PgSeverity::Warning => "WARNING",
                    PgSeverity::Notice => "NOTICE",
                    PgSeverity::Debug => "DEBUG",
                    PgSeverity::Info => "INFO",
                    PgSeverity::Log => "LOG",
                })
                .unwrap_or("FATAL");
            format!("{severity}:  {}", db.message())
        }
        sqlx::Error::Io(io) => {
            let text = io.raw_os_error()
                .map(|code| nix::errno::Errno::from_raw(code).desc().to_string())
                .unwrap_or_else(|| io.to_string());
            if uses_socket(cat) {
                format!("{text}\n\tIs the server running locally and accepting connections on that socket?")
            } else {
                format!("{text}\n\tIs the server running on that host and accepting TCP/IP connections?")
            }
        }
        other => other.to_string(),
    };
    format!("connection to server {target} failed: {reason}\n")
}

/// BareosDbPostgresql::OpenDatabase() (cats/postgresql.cc:288): six
/// connection attempts, each failure followed by a 5 second pause; the
/// error text of the last attempt.
fn open_database(cat: &CatalogResource, pause: std::time::Duration) -> Result<(), String> {
    let options = connect_options(cat);
    let attempt = || -> Result<(), sqlx::Error> {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
            .map_err(sqlx::Error::Io)?;
        runtime.block_on(async {
            use sqlx::Connection;
            let connection = sqlx::postgres::PgConnection::connect_with(&options).await?;
            connection.close().await
        })
    };
    let mut last = String::new();
    for _ in 0..6 {
        // own thread: the caller may run inside a tokio runtime
        let result = std::thread::scope(|scope| scope.spawn(attempt).join())
            .unwrap_or_else(|_| Err(sqlx::Error::Protocol("connect thread panicked".into())));
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                tracing::debug!("Could not connect to db: Err={error}");
                last = format!(
                    "cats/postgresql.cc:298 Unable to connect to PostgreSQL server. Database={} User={}\n\
                     Possible causes: SQL server not running; password incorrect; server requires ssl; \
                     max_connections exceeded.\n({})\n",
                    cat.db_name, cat.db_user, libpq_error(cat, &error));
            }
        }
        std::thread::sleep(pause);
    }
    Err(last)
}

/// CheckCatalog() connection part (dird/check_catalog.cc:42): every
/// Catalog resource must open, otherwise the Jmsg(M_FATAL) text of the
/// first one that fails. Catalogs before it already got their
/// UPDATE_CATALOG records in the original; `before_failure` runs then.
pub fn check_connections(config: &DirectorConfig, before_failure: impl FnOnce(&DirectorConfig)) -> Result<(), String> {
    check_connections_with(config, std::time::Duration::from_secs(5), before_failure)
}

fn check_connections_with(
    config: &DirectorConfig,
    pause: std::time::Duration,
    before_failure: impl FnOnce(&DirectorConfig),
) -> Result<(), String> {
    for (index, cat) in config.catalogs.iter().enumerate() {
        if let Err(error) = open_database(cat, pause) {
            tracing::error!("Could not open Catalog \"{}\", database \"{}\": {}", cat.name, cat.db_name, error.trim_end());
            if index > 0 {
                before_failure(config);
            }
            return Err(format!("Could not open Catalog \"{}\", database \"{}\": {error}\n", cat.name, cat.db_name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(name: &str, address: &str, port: u16) -> CatalogResource {
        CatalogResource {
            name: name.into(),
            db_name: "c075_nosuchdb".into(),
            db_user: "c075_user".into(),
            db_address: address.into(),
            db_port: port,
            ..Default::default()
        }
    }

    #[test]
    fn io_error_text_matches_libpq() {
        let cat = catalog("c", "", 5432);
        let error = sqlx::Error::Io(std::io::Error::from_raw_os_error(2));
        assert_eq!(libpq_error(&cat, &error),
            "connection to server on socket \"/var/run/postgresql/.s.PGSQL.5432\" failed: \
             No such file or directory\n\tIs the server running locally and accepting connections on that socket?\n");
    }

    #[test]
    fn tcp_error_names_host_address_and_port() {
        let cat = catalog("c", "127.0.0.2", 1);
        let error = sqlx::Error::Io(std::io::Error::from_raw_os_error(111));
        assert_eq!(libpq_error(&cat, &error),
            "connection to server at \"127.0.0.2\" (127.0.0.2), port 1 failed: Connection refused\n\
             \tIs the server running on that host and accepting TCP/IP connections?\n");
    }

    #[test]
    fn unreachable_catalog_fails_with_check_catalog_text() {
        // nothing listens on port 1: every attempt is refused at once
        let mut config = DirectorConfig::from_str("Director { Name = d\n Password = p\n }").unwrap();
        config.catalogs = vec![catalog("Bad", "127.0.0.2", 1)];
        let mut called = false;
        let error = check_connections_with(&config, std::time::Duration::ZERO, |_| called = true).unwrap_err();
        assert!(!called, "no catalog before the failing one");
        assert!(error.starts_with(
            "Could not open Catalog \"Bad\", database \"c075_nosuchdb\": cats/postgresql.cc:298 \
             Unable to connect to PostgreSQL server. Database=c075_nosuchdb User=c075_user\n\
             Possible causes: SQL server not running; password incorrect; server requires ssl; \
             max_connections exceeded.\n(connection to server at \"127.0.0.2\" (127.0.0.2), port 1 failed: "), "{error}");
        assert!(error.ends_with(")\n\n"), "{error:?}");
    }
}
