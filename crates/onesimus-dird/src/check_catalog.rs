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
