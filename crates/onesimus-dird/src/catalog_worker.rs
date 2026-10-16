//! Catalog Worker — single DB connection pool shared via MPSC channel.
//!
//! All database access goes through `CatalogHandle::request()`. The worker
//! thread owns the only `PgPool` and processes requests sequentially via
//! `flume::recv_async().await`. This replaces the previous pattern of creating
//! a throwaway `PgPoolOptions::new().max_connections(1)` + `Runtime::new()`
//! for every single query.
//!
//! Architecture:
//! ```text
//! Session/Worker (sync)  ──flume::Sender<(Req, oneshot)>──→  dir:catalog (async tokio)
//!                        ←──────flume::bounded(1)────────────┘
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, debug, error, warn};

// ---------------------------------------------------------------------------
// Request / Response types
// ---------------------------------------------------------------------------

/// A catalog request sent through the MPSC channel.
pub enum CatalogRequest {
    /// Raw SQL query — returns formatted text output.
    RawQuery { sql: String },
    /// SQL query returning every value as libpq text (like the C++ catalog).
    TextRows { sql: String },
    /// Get full volume info in OK_media format (24-field protocol string).
    GetVolumeInfo { vol_name: String },
    /// Find an appendable volume in a named pool.
    FindAppendableVolume { pool_name: String },
    /// Update media record from SD UpdateMedia values.
    UpdateMedia {
        vol_name: String,
        mounts: i64,
        bytes: i64,
        jobs: i32,
        files: i64,
        errors: i64,
        writes: i64,
    },
    /// Generate bootstrap data for a restore.
    GenerateBootstrap { job_id: u32 },
    /// Find the most recent successful backup job for a client.
    FindLatestBackupJobId { client_name: String },
    /// Create a new job record in the catalog. Returns the DB JobId.
    CreateJob {
        job_name: String,      // e.g. "backup-bareos-fd.2026-03-28_20.14.22_01"
        name: String,          // job resource name, e.g. "backup-bareos-fd"
        job_type: String,      // "B" backup, "R" restore
        level: String,         // "F" full, "I" incremental, "D" differential
        client_name: String,   // resolved to client_id
        pool_name: String,     // resolved to pool_id
        fileset_name: String,  // resolved to fileset_id
        sched_time: i64,       // unix epoch
    },
    /// Update job end record with final status, files, bytes.
    UpdateJobEnd {
        job_id: i32,
        status: String,        // "T" success, "E" error, "f" fatal
        end_time: i64,
        job_files: i64,
        job_bytes: i64,
        job_errors: i32,
        vol_session_id: i32,
        vol_session_time: i32,
    },
    /// Create a JobMedia record (SD→Dir CatReq). Performance-critical.
    CreateJobMedia {
        job_id: i32,
        media_id: i32,
        first_index: i32,
        last_index: i32,
        start_file: i32,
        end_file: i32,
        start_block: i64,
        end_block: i64,
        copy: i32,
        strip: i32,
        job_bytes: i64,
    },
    /// Resolve a name to its ID (client, pool, fileset).
    ResolveId { table: String, name: String },
    /// Get since_utime for incremental/differential backup.
    SinceUtime { client_name: String, job_name: String, fileset_name: String, level: String },
    /// Ping / health check.
    Ping,
    /// Reserve one pooled connection for a sequence of statements (C++
    /// private catalog connection, e.g. BVFS with BEGIN/LOCK/COMMIT).
    Dedicated { ops: flume::Receiver<(ConnectionOp, flume::Sender<CatalogResponse>)> },
}

/// A statement on a dedicated connection.
pub enum ConnectionOp {
    /// Rows with text values (CatalogResponse::Rows).
    Rows(String),
    /// Statement(s) without result; CatalogResponse::Ok(affected rows).
    Execute(String),
}

/// Response from the catalog worker.
pub enum CatalogResponse {
    /// Text output (query results, formatted strings).
    Text(String),
    /// Query result with text values.
    Rows(SqlTable),
    /// Optional text (volume info, bootstrap, etc.).
    OptionalText(Option<String>),
    /// Optional job ID.
    OptionalJobId(Option<u32>),
    /// Job ID (from CreateJob).
    JobId(i32),
    /// Success with rows affected.
    Ok(u64),
    /// Error message.
    Error(String),
    /// Since-utime result.
    SinceUtime { since_utime: i64, prev_job: String },
    /// Pong.
    Pong,
}

/// A query result as the C++ catalog sees it: all values in PostgreSQL text
/// format (simple query protocol), NULL as `None`.
#[derive(Debug, Clone, Default)]
pub struct SqlTable {
    pub columns: Vec<SqlColumn>,
    pub rows: Vec<Vec<Option<String>>>,
}

#[derive(Debug, Clone)]
pub struct SqlColumn {
    /// Column name as returned by PostgreSQL (lowercase for unquoted names)
    pub name: String,
    /// Numeric SQL type (C++ SqlFieldIsNumeric: right-aligned, thousands separators)
    pub numeric: bool,
}

// ---------------------------------------------------------------------------
// CatalogHandle — cloneable sender side
// ---------------------------------------------------------------------------

/// Cloneable handle for sending DB requests. All methods are blocking.
#[derive(Clone)]
pub struct CatalogHandle {
    tx: flume::Sender<(CatalogRequest, flume::Sender<CatalogResponse>)>,
}

impl CatalogHandle {
    #[cfg(test)]
    pub(crate) fn mock(mut handle: impl FnMut(CatalogRequest) -> CatalogResponse + Send + 'static) -> Self {
        let (tx, rx) = flume::bounded::<(CatalogRequest, flume::Sender<CatalogResponse>)>(16);
        std::thread::spawn(move || while let Ok((request, response)) = rx.recv() {
            let _ = response.send(handle(request));
        });
        Self { tx }
    }
    /// Send a request and wait for the response (blocking).
    fn request(&self, req: CatalogRequest) -> Result<CatalogResponse, String> {
        let (resp_tx, resp_rx) = flume::bounded(1);
        self.tx.send((req, resp_tx))
            .map_err(|_| "catalog worker shut down".to_string())?;
        resp_rx.recv()
            .map_err(|_| "catalog worker dropped response".to_string())
    }

    /// Execute a raw SQL query, returning formatted text.
    pub fn raw_query(&self, sql: &str) -> Result<String, String> {
        match self.request(CatalogRequest::RawQuery { sql: sql.to_string() })? {
            CatalogResponse::Text(t) => Ok(t),
            CatalogResponse::Error(e) => Err(e),
            other => Err(format!("unexpected response: {:?}", resp_debug(&other))),
        }
    }

    /// Execute a query and return all values as text (C++ catalog semantics).
    pub fn query_rows(&self, sql: &str) -> Result<SqlTable, String> {
        match self.request(CatalogRequest::TextRows { sql: sql.to_string() })? {
            CatalogResponse::Rows(t) => Ok(t),
            CatalogResponse::Error(e) => Err(e),
            other => Err(format!("unexpected response: {:?}", resp_debug(&other))),
        }
    }

    /// Get full volume info in OK_media format.
    pub fn get_volume_info(&self, vol_name: &str) -> Option<String> {
        match self.request(CatalogRequest::GetVolumeInfo { vol_name: vol_name.to_string() }) {
            Ok(CatalogResponse::OptionalText(t)) => t,
            _ => None,
        }
    }

    /// Find an appendable volume in a pool.
    pub fn find_appendable_volume(&self, pool_name: &str) -> Option<String> {
        match self.request(CatalogRequest::FindAppendableVolume { pool_name: pool_name.to_string() }) {
            Ok(CatalogResponse::OptionalText(t)) => t,
            _ => None,
        }
    }

    /// Update media record in DB.
    pub fn update_media(
        &self, vol_name: &str, mounts: i64, bytes: i64, jobs: i32,
        files: i64, errors: i64, writes: i64,
    ) {
        let _ = self.request(CatalogRequest::UpdateMedia {
            vol_name: vol_name.to_string(), mounts, bytes, jobs, files, errors, writes,
        });
    }

    /// Generate bootstrap data for a restore.
    pub fn generate_bootstrap(&self, job_id: u32) -> Option<String> {
        match self.request(CatalogRequest::GenerateBootstrap { job_id }) {
            Ok(CatalogResponse::OptionalText(t)) => t,
            _ => None,
        }
    }

    /// Find the most recent successful backup job for a client.
    pub fn find_latest_backup_jobid(&self, client_name: &str) -> Option<u32> {
        match self.request(CatalogRequest::FindLatestBackupJobId { client_name: client_name.to_string() }) {
            Ok(CatalogResponse::OptionalJobId(id)) => id,
            _ => None,
        }
    }

    /// Create a job record in the DB. Returns the DB-assigned JobId.
    pub fn create_job(
        &self, job_name: &str, name: &str, job_type: &str, level: &str,
        client_name: &str, pool_name: &str, fileset_name: &str, sched_time: i64,
    ) -> Result<i32, String> {
        match self.request(CatalogRequest::CreateJob {
            job_name: job_name.to_string(),
            name: name.to_string(),
            job_type: job_type.to_string(),
            level: level.to_string(),
            client_name: client_name.to_string(),
            pool_name: pool_name.to_string(),
            fileset_name: fileset_name.to_string(),
            sched_time,
        })? {
            CatalogResponse::JobId(id) => Ok(id),
            CatalogResponse::Error(e) => Err(e),
            _ => Err("unexpected response".into()),
        }
    }

    /// Update job end record with final status.
    pub fn update_job_end(
        &self, job_id: i32, status: &str, end_time: i64,
        job_files: i64, job_bytes: i64, job_errors: i32,
        vol_session_id: i32, vol_session_time: i32,
    ) -> Result<(), String> {
        match self.request(CatalogRequest::UpdateJobEnd {
            job_id, status: status.to_string(), end_time,
            job_files, job_bytes, job_errors, vol_session_id, vol_session_time,
        })? {
            CatalogResponse::Ok(_) => Ok(()),
            CatalogResponse::Error(e) => Err(e),
            _ => Err("unexpected response".into()),
        }
    }

    /// Create a JobMedia record. Returns JobMediaId.
    pub fn create_jobmedia(
        &self, job_id: i32, media_id: i32, first_index: i32, last_index: i32,
        start_file: i32, end_file: i32, start_block: i64, end_block: i64,
        copy: i32, strip: i32, job_bytes: i64,
    ) -> Result<i32, String> {
        match self.request(CatalogRequest::CreateJobMedia {
            job_id, media_id, first_index, last_index,
            start_file, end_file, start_block, end_block,
            copy, strip, job_bytes,
        })? {
            CatalogResponse::JobId(id) => Ok(id),
            CatalogResponse::Error(e) => Err(e),
            _ => Err("unexpected response".into()),
        }
    }

    /// Resolve a name to its DB ID (client, pool, fileset).
    pub fn resolve_id(&self, table: &str, name: &str) -> Option<i32> {
        match self.request(CatalogRequest::ResolveId {
            table: table.to_string(), name: name.to_string(),
        }) {
            Ok(CatalogResponse::OptionalJobId(Some(id))) => Some(id as i32),
            _ => None,
        }
    }

    /// Get since_utime + prev_job for Incremental/Differential backup.
    /// level: "I" for incremental, "D" for differential.
    pub fn since_utime(&self, client_name: &str, job_name: &str, fileset_name: &str, level: &str) -> Result<Option<(i64, String)>, String> {
        match self.request(CatalogRequest::SinceUtime {
            client_name: client_name.to_string(), job_name: job_name.to_string(),
            fileset_name: fileset_name.to_string(), level: level.to_string(),
        })? {
            CatalogResponse::SinceUtime { since_utime, prev_job } => {
                Ok((since_utime > 0).then_some((since_utime, prev_job)))
            }
            CatalogResponse::Error(error) => Err(error),
            _ => Err("unexpected backup reference response".into()),
        }
    }

    /// Health check.
    pub fn ping(&self) -> bool {
        matches!(self.request(CatalogRequest::Ping), Ok(CatalogResponse::Pong))
    }

    /// Reserve one connection of the pool; it is returned when the
    /// CatalogConnection is dropped.
    pub fn dedicated(&self) -> Result<CatalogConnection, String> {
        let (tx, ops) = flume::unbounded();
        match self.request(CatalogRequest::Dedicated { ops })? {
            CatalogResponse::Ok(_) => Ok(CatalogConnection { tx }),
            CatalogResponse::Error(e) => Err(e),
            other => Err(format!("unexpected response: {:?}", resp_debug(&other))),
        }
    }
}

/// One reserved catalog connection (all statements run on it in order).
pub struct CatalogConnection {
    tx: flume::Sender<(ConnectionOp, flume::Sender<CatalogResponse>)>,
}

impl CatalogConnection {
    #[cfg(test)]
    pub(crate) fn mock(mut handle: impl FnMut(ConnectionOp) -> CatalogResponse + Send + 'static) -> Self {
        let (tx, rx) = flume::unbounded::<(ConnectionOp, flume::Sender<CatalogResponse>)>();
        std::thread::spawn(move || while let Ok((op, response)) = rx.recv() {
            let _ = response.send(handle(op));
        });
        Self { tx }
    }

    fn request(&self, op: ConnectionOp) -> Result<CatalogResponse, String> {
        let (resp_tx, resp_rx) = flume::bounded(1);
        self.tx.send((op, resp_tx)).map_err(|_| "catalog connection closed".to_string())?;
        resp_rx.recv().map_err(|_| "catalog connection dropped response".to_string())
    }

    /// Query returning all values as text (C++ SqlQuery/QueryDb).
    pub fn query_rows(&self, sql: &str) -> Result<SqlTable, String> {
        match self.request(ConnectionOp::Rows(sql.to_string()))? {
            CatalogResponse::Rows(t) => Ok(t),
            CatalogResponse::Error(e) => Err(e),
            other => Err(format!("unexpected response: {:?}", resp_debug(&other))),
        }
    }

    /// Statement(s) returning the number of affected rows (C++ UpdateDb,
    /// SqlAffectedRows).
    pub fn execute(&self, sql: &str) -> Result<u64, String> {
        match self.request(ConnectionOp::Execute(sql.to_string()))? {
            CatalogResponse::Ok(n) => Ok(n),
            CatalogResponse::Error(e) => Err(e),
            other => Err(format!("unexpected response: {:?}", resp_debug(&other))),
        }
    }
}

fn resp_debug(r: &CatalogResponse) -> &'static str {
    match r {
        CatalogResponse::Text(_) => "Text",
        CatalogResponse::Rows(_) => "Rows",
        CatalogResponse::OptionalText(_) => "OptionalText",
        CatalogResponse::OptionalJobId(_) => "OptionalJobId",
        CatalogResponse::JobId(_) => "JobId",
        CatalogResponse::Ok(_) => "Ok",
        CatalogResponse::Error(_) => "Error",
        CatalogResponse::SinceUtime { .. } => "SinceUtime",
        CatalogResponse::Pong => "Pong",
    }
}

// ---------------------------------------------------------------------------
// Worker thread
// ---------------------------------------------------------------------------

/// Spawn the catalog worker. Returns the CatalogHandle for callers.
pub fn spawn_catalog_worker(
    catalog_url: String,
    shutdown: &'static AtomicBool,
) -> (CatalogHandle, std::thread::JoinHandle<()>) {
    let (tx, rx) = flume::bounded::<(CatalogRequest, flume::Sender<CatalogResponse>)>(256);
    let handle = CatalogHandle { tx };

    let join = std::thread::Builder::new()
        .name("dir:catalog".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("catalog worker: tokio runtime");

            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async {
                let pool = match sqlx::postgres::PgPoolOptions::new()
                    .max_connections(8)
                    .connect(&catalog_url)
                    .await
                {
                    Ok(p) => p,
                    Err(e) => {
                        error!("catalog worker: DB connect failed: {e}");
                        return;
                    }
                };

                info!("catalog worker connected (pool max=8)");

                while let Ok((req, resp_tx)) = rx.recv_async().await {
                    if shutdown.load(Ordering::Relaxed) { break; }

                    let response = match req {
                        CatalogRequest::RawQuery { sql } =>
                            handle_raw_query(&pool, &sql).await,
                        CatalogRequest::TextRows { sql } =>
                            handle_text_rows(&pool, &sql).await,
                        CatalogRequest::GetVolumeInfo { vol_name } =>
                            handle_get_volume_info(&pool, &vol_name).await,
                        CatalogRequest::FindAppendableVolume { pool_name } =>
                            handle_find_appendable_volume(&pool, &pool_name).await,
                        CatalogRequest::UpdateMedia { vol_name, mounts, bytes, jobs, files, errors, writes } =>
                            handle_update_media(&pool, &vol_name, mounts, bytes, jobs, files, errors, writes).await,
                        CatalogRequest::GenerateBootstrap { job_id } =>
                            handle_generate_bootstrap(&pool, job_id).await,
                        CatalogRequest::FindLatestBackupJobId { client_name } =>
                            handle_find_latest_backup_jobid(&pool, &client_name).await,
                        CatalogRequest::CreateJob { job_name, name, job_type, level, client_name, pool_name, fileset_name, sched_time } =>
                            handle_create_job(&pool, &job_name, &name, &job_type, &level, &client_name, &pool_name, &fileset_name, sched_time).await,
                        CatalogRequest::UpdateJobEnd { job_id, status, end_time, job_files, job_bytes, job_errors, vol_session_id, vol_session_time } =>
                            handle_update_job_end(&pool, job_id, &status, end_time, job_files, job_bytes, job_errors, vol_session_id, vol_session_time).await,
                        CatalogRequest::CreateJobMedia { job_id, media_id, first_index, last_index, start_file, end_file, start_block, end_block, copy, strip, job_bytes } =>
                            handle_create_jobmedia(&pool, job_id, media_id, first_index, last_index, start_file, end_file, start_block, end_block, copy, strip, job_bytes).await,
                        CatalogRequest::ResolveId { table, name } =>
                            handle_resolve_id(&pool, &table, &name).await,
                        CatalogRequest::SinceUtime { client_name, job_name, fileset_name, level } =>
                            handle_since_utime(&pool, &client_name, &job_name, &fileset_name, &level).await,
                        CatalogRequest::Ping => CatalogResponse::Pong,
                        CatalogRequest::Dedicated { ops } => {
                            // Serve the statements in a task so other requests
                            // keep running while the connection is reserved
                            // (local task: sqlx raw_sql futures are not Send).
                            let pool = pool.clone();
                            tokio::task::spawn_local(handle_dedicated(pool, ops));
                            CatalogResponse::Ok(0)
                        }
                    };

                    let _ = resp_tx.send(response);
                }

                pool.close().await;
                info!("catalog worker stopped");
            });
        })
        .expect("failed to spawn dir:catalog");

    (handle, join)
}

// ---------------------------------------------------------------------------
// Async handlers — one per CatalogRequest variant
// ---------------------------------------------------------------------------

async fn handle_raw_query(pool: &sqlx::PgPool, sql: &str) -> CatalogResponse {
    let rows = match sqlx::query(sql).fetch_all(pool).await {
        Ok(r) => r,
        Err(e) => return CatalogResponse::Error(format!("Query: {e}")),
    };

    use sqlx::{Row, Column};
    let mut output = String::new();

    if rows.is_empty() {
        output.push_str("No results.\n");
        return CatalogResponse::Text(output);
    }

    let columns = rows[0].columns();
    for col in columns {
        output.push_str(&format!("{:<20}", col.name()));
    }
    output.push('\n');
    output.push_str(&"-".repeat(columns.len() * 20));
    output.push('\n');

    for row in &rows {
        for col in columns {
            let val: String = row.try_get::<String, _>(col.ordinal())
                .or_else(|_| row.try_get::<i64, _>(col.ordinal()).map(|v| v.to_string()))
                .or_else(|_| row.try_get::<i32, _>(col.ordinal()).map(|v| v.to_string()))
                .or_else(|_| row.try_get::<bool, _>(col.ordinal()).map(|v| v.to_string()))
                .or_else(|_| row.try_get::<chrono::NaiveDateTime, _>(col.ordinal()).map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string()))
                .or_else(|_| row.try_get::<chrono::DateTime<chrono::Utc>, _>(col.ordinal()).map(|v| v.format("%Y-%m-%d %H:%M:%S").to_string()))
                .unwrap_or_else(|_| "NULL".to_string());
            output.push_str(&format!("{:<20}", val));
        }
        output.push('\n');
    }

    CatalogResponse::Text(output)
}

/// Simple query protocol: PostgreSQL sends every value in text format, exactly
/// what libpq hands to the C++ Director (timestamps, intervals, numerics).
/// Like PQexec, a string with several statements yields the last result set;
/// errors carry the libpq PQerrorMessage text (see `pq_error_message`).
async fn handle_text_rows(pool: &sqlx::PgPool, sql: &str) -> CatalogResponse {
    use sqlx::Either;
    use tokio_stream::StreamExt;
    let mut last: Vec<sqlx::postgres::PgRow> = Vec::new();
    let mut current = Vec::new();
    {
        let mut stream = sqlx::raw_sql(sql).fetch_many(pool);
        while let Some(item) = stream.next().await {
            match item {
                Ok(Either::Left(_done)) => last = std::mem::take(&mut current),
                Ok(Either::Right(row)) => current.push(row),
                Err(e) => {
                    drop(stream);
                    return CatalogResponse::Error(pq_error_message(pool, sql, &e).await);
                }
            }
        }
    }
    let rows = if current.is_empty() { last } else { current };
    CatalogResponse::Rows(text_table(&rows))
}

/// Run the statements of one CatalogConnection on a single pooled connection.
async fn handle_dedicated(pool: sqlx::PgPool, ops: flume::Receiver<(ConnectionOp, flume::Sender<CatalogResponse>)>) {
    let mut conn = match pool.acquire().await {
        Ok(c) => Some(c),
        Err(e) => {
            error!("catalog worker: no connection for dedicated use: {e}");
            None
        }
    };
    while let Ok((op, resp_tx)) = ops.recv_async().await {
        let response = match (&mut conn, op) {
            (None, _) => CatalogResponse::Error("no catalog connection".into()),
            (Some(c), op) => connection_op(&mut **c, op).await,
        };
        let _ = resp_tx.send(response);
    }
}

async fn connection_op(conn: &mut sqlx::PgConnection, op: ConnectionOp) -> CatalogResponse {
    match op {
        ConnectionOp::Rows(sql) => match sqlx::raw_sql(&sql).fetch_all(conn).await {
            Ok(rows) => CatalogResponse::Rows(text_table(&rows)),
            Err(e) => CatalogResponse::Error(format!("Query: {e}")),
        },
        ConnectionOp::Execute(sql) => match sqlx::raw_sql(&sql).execute(conn).await {
            Ok(r) => CatalogResponse::Ok(r.rows_affected()),
            Err(e) => CatalogResponse::Error(format!("Query: {e}")),
        },
    }
}

fn text_table(rows: &[sqlx::postgres::PgRow]) -> SqlTable {
    use sqlx::{Column, Row, TypeInfo};
    let mut table = SqlTable::default();
    if let Some(first) = rows.first() {
        table.columns = first.columns().iter().map(|c| SqlColumn {
            name: c.name().to_string(),
            numeric: matches!(c.type_info().name(), "INT2" | "INT4" | "INT8" | "NUMERIC" | "FLOAT4" | "FLOAT8" | "OID"),
        }).collect();
    }
    for row in rows {
        let values = (0..table.columns.len())
            .map(|i| row.try_get_unchecked::<Option<String>, _>(i).ok().flatten())
            .collect();
        table.rows.push(values);
    }
    table
}

/// libpq PQerrorMessage (fe-protocol3.c pqBuildErrorMessage3, default
/// verbosity): "<SEVERITY>:  <message>\n", the "LINE n: …" excerpt with the
/// cursor line, then DETAIL/HINT/QUERY lines. The severity word is the
/// server-localized one (field S), which sqlx does not expose; it is derived
/// from the server's lc_messages instead.
async fn pq_error_message(pool: &sqlx::PgPool, sql: &str, e: &sqlx::Error) -> String {
    use sqlx::postgres::{PgDatabaseError, PgErrorPosition, PgSeverity};
    let Some(db) = e.as_database_error().and_then(|d| d.try_downcast_ref::<PgDatabaseError>()) else {
        return format!("{e}\n");
    };
    let lc: Option<String> = sqlx::query_scalar("SHOW lc_messages").fetch_one(pool).await.ok();
    let severity = match db.severity() {
        PgSeverity::Fatal => "FATAL",
        PgSeverity::Panic => "PANIC",
        _ => localized_error_word(lc.as_deref().unwrap_or("")),
    };
    let mut msg = format!("{severity}:  {}\n", db.message());
    match db.position() {
        Some(PgErrorPosition::Original(loc)) => msg.push_str(&report_error_position(sql, loc)),
        _ => {}
    }
    if let Some(d) = db.detail() { msg.push_str(&format!("DETAIL:  {d}\n")); }
    if let Some(h) = db.hint() { msg.push_str(&format!("HINT:  {h}\n")); }
    if let Some(PgErrorPosition::Internal { query, .. }) = db.position() {
        msg.push_str(&format!("QUERY:  {query}\n"));
    }
    msg
}

/// Translation of "ERROR" in the PostgreSQL message catalogs (backend po/).
fn localized_error_word(lc_messages: &str) -> &'static str {
    let lang = lc_messages.split(['_', '.', '@']).next().unwrap_or("");
    match lang {
        "de" => "FEHLER",
        "fr" => "ERREUR",
        "it" => "ERRORE",
        "pt" => "ERRO",
        "ru" => "ОШИБКА",
        "sv" => "FEL",
        "pl" => "BŁĄD",
        "tr" => "HATA",
        _ => "ERROR",
    }
}

/// libpq reportErrorPosition(): "LINE n: <line>\n" plus the cursor line,
/// truncated to DISPLAY_SIZE columns around the 1-based character `loc`.
/// Every character counts as one screen column.
fn report_error_position(query: &str, loc: usize) -> String {
    const DISPLAY_SIZE: usize = 60;
    const MIN_RIGHT_CUT: usize = 10;
    if loc == 0 {
        return String::new();
    }
    let loc = loc - 1;
    let chars: Vec<char> = query.chars().map(|c| if c == '\t' { ' ' } else { c }).collect();
    let (mut loc_line, mut ibeg, mut iend) = (1, 0, None);
    for (cno, &ch) in chars.iter().enumerate() {
        if ch == '\r' || ch == '\n' {
            if cno < loc {
                if ch == '\r' || cno == 0 || chars[cno - 1] != '\r' {
                    loc_line += 1;
                }
                ibeg = cno + 1;
            } else {
                iend = Some(cno);
                break;
            }
        }
    }
    let mut iend = iend.unwrap_or(chars.len());
    if loc > chars.len() {
        return String::new();
    }
    let (mut beg_trunc, mut end_trunc) = (false, false);
    if iend - ibeg > DISPLAY_SIZE {
        if ibeg + DISPLAY_SIZE >= loc + MIN_RIGHT_CUT {
            while iend - ibeg > DISPLAY_SIZE {
                iend -= 1;
            }
            end_trunc = true;
        } else {
            while loc + MIN_RIGHT_CUT < iend {
                iend -= 1;
                end_trunc = true;
            }
            while iend - ibeg > DISPLAY_SIZE {
                ibeg += 1;
                beg_trunc = true;
            }
        }
    }
    let mut prefix = format!("LINE {loc_line}: ");
    if beg_trunc {
        prefix.push_str("...");
    }
    let line: String = chars[ibeg..iend].iter().collect();
    let cursor = prefix.chars().count() + loc - ibeg;
    format!("{prefix}{line}{}\n{}^\n", if end_trunc { "..." } else { "" }, " ".repeat(cursor))
}

async fn handle_get_volume_info(pool: &sqlx::PgPool, vol_name: &str) -> CatalogResponse {
    let row: Option<(
        String, i32, i64, i64, i64, i64, i64, i64, i64,
        String, i64, i32, i64, i64, i64, i64,
    )> = match sqlx::query_as(
        "SELECT volumename, COALESCE(voljobs,0)::int, COALESCE(volfiles,0)::bigint, \
         COALESCE(volblocks,0)::bigint, COALESCE(volbytes,0)::bigint, COALESCE(volmounts,0)::bigint, \
         COALESCE(volerrors,0)::bigint, COALESCE(volwrites,0)::bigint, COALESCE(maxvolbytes,0)::bigint, \
         COALESCE(volstatus,'Append'), COALESCE(slot,0)::bigint, mediaid, \
         COALESCE(endfile,0)::bigint, COALESCE(endblock,0)::bigint, \
         COALESCE(minblocksize,0)::bigint, COALESCE(maxblocksize,0)::bigint \
         FROM media WHERE volumename = $1"
    ).bind(vol_name).fetch_optional(pool).await {
        Ok(r) => r,
        Err(e) => return CatalogResponse::Error(format!("GetVolumeInfo: {e}")),
    };

    CatalogResponse::OptionalText(row.map(|(vn, vj, vf, vbl, vby, vm, ve, vw, mvb, vs, sl, mid, ef, eb, minbs, maxbs)| {
        format!(
            "1000 OK VolName={vn} VolJobs={vj} VolFiles={vf} \
             VolBlocks={vbl} VolBytes={vby} VolMounts={vm} VolErrors={ve} VolWrites={vw} \
             MaxVolBytes={mvb} VolCapacityBytes=0 VolStatus={vs} Slot={sl} \
             MaxVolJobs=0 MaxVolFiles=0 InChanger=0 VolReadTime=0 \
             VolWriteTime=0 EndFile={ef} EndBlock={eb} LabelType=0 \
             MediaId={mid} EncryptionKey=*None* MinBlocksize={minbs} MaxBlocksize={maxbs}\n"
        )
    }))
}

async fn handle_find_appendable_volume(pool: &sqlx::PgPool, pool_name: &str) -> CatalogResponse {
    // fn_find_appendable_volume(p_pool_name text) RETURNS text
    let row: Option<(Option<String>,)> = match sqlx::query_as(
        "SELECT fn_find_appendable_volume($1)"
    ).bind(pool_name).fetch_optional(pool).await {
        Ok(r) => r,
        Err(_) => {
            // Fallback: SP not installed
            let r: Option<(String,)> = sqlx::query_as(
                "SELECT m.volumename FROM media m JOIN pool p ON m.poolid = p.poolid \
                 WHERE p.name = $1 AND m.volstatus = 'Append' ORDER BY m.mediaid LIMIT 1"
            ).bind(pool_name).fetch_optional(pool).await.ok().flatten();
            return CatalogResponse::OptionalText(r.map(|(n,)| n));
        }
    };

    CatalogResponse::OptionalText(row.and_then(|(v,)| v))
}

async fn handle_update_media(
    pool: &sqlx::PgPool, vol_name: &str,
    mounts: i64, bytes: i64, jobs: i32, files: i64, errors: i64, writes: i64,
) -> CatalogResponse {
    match sqlx::query(
        "UPDATE media SET volmounts=$1, volbytes=$2, voljobs=$3, volfiles=$4, \
         volerrors=$5, volwrites=$6 WHERE volumename=$7"
    )
    .bind(mounts).bind(bytes).bind(jobs).bind(files).bind(errors).bind(writes).bind(vol_name)
    .execute(pool).await {
        Ok(r) => {
            debug!("UpdateMedia: vol={vol_name} bytes={bytes} mounts={mounts}");
            CatalogResponse::Ok(r.rows_affected())
        }
        Err(e) => CatalogResponse::Error(format!("UpdateMedia: {e}")),
    }
}

async fn handle_generate_bootstrap(pool: &sqlx::PgPool, job_id: u32) -> CatalogResponse {
    // Get job info
    let job_row: Option<(i32, i32, String)> = match sqlx::query_as(
        "SELECT COALESCE(volsessionid,0)::int, COALESCE(volsessiontime,0)::int, \
         COALESCE(name,'') FROM job WHERE jobid = $1"
    ).bind(job_id as i32).fetch_optional(pool).await {
        Ok(r) => r,
        Err(e) => return CatalogResponse::Error(format!("GenerateBootstrap: {e}")),
    };

    let Some((vol_session_id, vol_session_time, _job_name)) = job_row else {
        return CatalogResponse::OptionalText(None);
    };

    // Get jobmedia records
    let jm_rows: Vec<(i32, i32, i32, i64, i64)> = match sqlx::query_as(
        "SELECT jm.mediaid, COALESCE(jm.firstindex,0)::int, COALESCE(jm.lastindex,0)::int, \
         COALESCE(jm.startblock,0)::bigint, COALESCE(jm.endblock,0)::bigint \
         FROM jobmedia jm WHERE jm.jobid = $1 ORDER BY jm.jobmediaid"
    ).bind(job_id as i32).fetch_all(pool).await {
        Ok(r) => r,
        Err(e) => return CatalogResponse::Error(format!("GenerateBootstrap jobmedia: {e}")),
    };

    if jm_rows.is_empty() {
        info!("No jobmedia records for JobId={job_id}");
        return CatalogResponse::OptionalText(None);
    }

    let mut bsr = String::new();

    for (media_id, first_idx, last_idx, start_block, end_block) in &jm_rows {
        let vol_row: Option<(String, String)> = match sqlx::query_as(
            "SELECT volumename, COALESCE(mediatype,'File') FROM media WHERE mediaid = $1"
        ).bind(media_id).fetch_optional(pool).await {
            Ok(r) => r,
            Err(e) => return CatalogResponse::Error(format!("GenerateBootstrap media: {e}")),
        };

        let Some((vol_name, media_type)) = vol_row else {
            continue;
        };

        let storage_name = "File"; // TODO: resolve from config
        let device_name = "FileStorage"; // TODO: resolve from config

        bsr.push_str(&format!("Storage=\"{storage_name}\"\n"));
        bsr.push_str(&format!("Volume=\"{vol_name}\"\n"));
        bsr.push_str(&format!("MediaType=\"{media_type}\"\n"));
        bsr.push_str(&format!("Device=\"{device_name}\"\n"));
        bsr.push_str(&format!("VolSessionId={vol_session_id}\n"));
        bsr.push_str(&format!("VolSessionTime={vol_session_time}\n"));
        if *start_block > 0 || *end_block > 0 {
            bsr.push_str(&format!("VolAddr={start_block}-{end_block}\n"));
        }
        if *first_idx > 0 && *last_idx > 0 {
            bsr.push_str(&format!("FileIndex={first_idx}-{last_idx}\n"));
        }
        bsr.push_str(&format!("Count={}\n", last_idx - first_idx + 1));
    }

    debug!("Bootstrap for JobId={job_id}: {} lines", bsr.lines().count());
    CatalogResponse::OptionalText(Some(bsr))
}

/// Get since_utime for incremental/differential backup.
async fn handle_since_utime(pool: &sqlx::PgPool, client_name: &str, job_name: &str, fileset_name: &str, level: &str) -> CatalogResponse {
    // A different Job/FileSet on the same client must never become our base.
    // Require a successful Full in the chain, even when selecting a later I/D.
    let level_filter = if level == "D" { "j.level = 'F'" } else { "j.level IN ('F','I','D')" };
    let sql = format!(
        "SELECT COALESCE(EXTRACT(EPOCH FROM j.starttime)::bigint, 0), j.job \
         FROM job j JOIN client c ON j.clientid = c.clientid \
         JOIN fileset f ON j.filesetid = f.filesetid \
         WHERE c.name = $1 AND j.name = $2 AND f.fileset = $3 \
         AND j.type = 'B' AND j.jobstatus IN ('T','W') AND {level_filter} \
         AND EXISTS (SELECT 1 FROM job fulljob JOIN fileset fullset ON fulljob.filesetid = fullset.filesetid \
             WHERE fulljob.clientid = j.clientid AND fulljob.name = j.name \
             AND fullset.fileset = f.fileset AND fulljob.type = 'B' \
             AND fulljob.level = 'F' AND fulljob.jobstatus IN ('T','W') \
             AND fulljob.starttime <= j.starttime) \
         ORDER BY j.starttime DESC, j.jobid DESC LIMIT 1"
    );
    match sqlx::query_as::<_, (i64, String)>(&sql)
        .bind(client_name).bind(job_name).bind(fileset_name).fetch_optional(pool).await {
        Ok(Some((ts, prev))) => CatalogResponse::SinceUtime { since_utime: ts, prev_job: prev },
        Ok(None) => CatalogResponse::SinceUtime { since_utime: 0, prev_job: String::new() },
        Err(error) => CatalogResponse::Error(format!("Backup reference lookup failed: {error}")),
    }
}

/// Create a JobMedia record — performance-critical (many per backup).
/// Uses sp_create_jobmedia SP with inline SQL fallback.
async fn handle_create_jobmedia(
    pool: &sqlx::PgPool, job_id: i32, media_id: i32,
    first_index: i32, last_index: i32, start_file: i32, end_file: i32,
    start_block: i64, end_block: i64, _copy: i32, _strip: i32, job_bytes: i64,
) -> CatalogResponse {
    // Try SP first
    let result: Result<i32, _> = sqlx::query_scalar(
        "SELECT sp_create_jobmedia($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"
    )
    .bind(job_id).bind(media_id).bind(first_index).bind(last_index)
    .bind(start_file).bind(end_file).bind(start_block).bind(end_block)
    .bind(_copy).bind(_strip).bind(job_bytes)
    .fetch_one(pool).await;

    match result {
        Ok(jm_id) => CatalogResponse::JobId(jm_id),
        Err(_) => {
            // Fallback: inline SQL
            let vol_index: i32 = sqlx::query_scalar(
                "SELECT COALESCE(COUNT(*), 0)::int + 1 FROM jobmedia WHERE jobid = $1"
            ).bind(job_id).fetch_one(pool).await.unwrap_or(1);

            match sqlx::query_scalar(
                "INSERT INTO jobmedia (jobid, mediaid, firstindex, lastindex, \
                 startfile, endfile, startblock, endblock, volindex, jobbytes) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING jobmediaid"
            )
            .bind(job_id).bind(media_id).bind(first_index).bind(last_index)
            .bind(start_file).bind(end_file).bind(start_block).bind(end_block)
            .bind(vol_index).bind(job_bytes)
            .fetch_one(pool).await {
                Ok(jm_id) => {
                    // Update media EndFile/EndBlock
                    let _ = sqlx::query(
                        "UPDATE media SET endfile = GREATEST(endfile, $1), \
                         endblock = GREATEST(endblock, $2) WHERE mediaid = $3"
                    ).bind(end_file).bind(end_block).bind(media_id)
                    .execute(pool).await;

                    CatalogResponse::JobId(jm_id)
                }
                Err(e) => CatalogResponse::Error(format!("CreateJobMedia: {e}")),
            }
        }
    }
}

async fn handle_find_latest_backup_jobid(pool: &sqlx::PgPool, client_name: &str) -> CatalogResponse {
    // fn_find_latest_backup(p_client_name text) RETURNS integer
    let row: Option<(Option<i32>,)> = match sqlx::query_as(
        "SELECT fn_find_latest_backup($1)"
    ).bind(client_name).fetch_optional(pool).await {
        Ok(r) => r,
        Err(_) => {
            // Fallback: SP not installed
            let r: Option<(i32,)> = sqlx::query_as(
                "SELECT j.jobid FROM job j JOIN client c ON j.clientid = c.clientid \
                 WHERE c.name = $1 AND j.type = 'B' AND j.jobstatus = 'T' \
                 ORDER BY j.jobid DESC LIMIT 1"
            ).bind(client_name).fetch_optional(pool).await.ok().flatten();
            return CatalogResponse::OptionalJobId(r.map(|(id,)| id as u32));
        }
    };

    CatalogResponse::OptionalJobId(row.and_then(|(v,)| v).map(|id| id as u32))
}

/// Resolve a name to its DB ID (client, pool, fileset, storage).
async fn handle_resolve_id(pool: &sqlx::PgPool, table: &str, name: &str) -> CatalogResponse {
    let (id_col, tbl) = match table {
        "client" => ("clientid", "client"),
        "pool" => ("poolid", "pool"),
        "fileset" => ("filesetid", "fileset"),
        "storage" => ("storageid", "storage"),
        _ => return CatalogResponse::Error(format!("unknown table: {table}")),
    };

    let sql = format!("SELECT {id_col} FROM {tbl} WHERE name = $1");
    let row: Option<(i32,)> = match sqlx::query_as(&sql).bind(name).fetch_optional(pool).await {
        Ok(r) => r,
        Err(e) => return CatalogResponse::Error(format!("ResolveId {table}/{name}: {e}")),
    };

    CatalogResponse::OptionalJobId(row.map(|(id,)| id as u32))
}

/// Create a job record in the DB, resolving client/pool/fileset names to IDs.
async fn handle_create_job(
    pool: &sqlx::PgPool,
    job_name: &str, name: &str, job_type: &str, level: &str,
    client_name: &str, pool_name: &str, fileset_name: &str, sched_time: i64,
) -> CatalogResponse {
    // Use stored procedure: create_job_record(job, name, type, level, client, pool, fileset, sched_time)
    let job_id: i32 = match sqlx::query_scalar(
        "SELECT sp_create_job_record($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(job_name).bind(name).bind(job_type).bind(level)
    .bind(client_name).bind(pool_name).bind(fileset_name).bind(sched_time)
    .fetch_one(pool).await {
        Ok(id) => id,
        Err(e) => {
            // Fallback: SP not yet installed, use inline SQL
            warn!("SP sp_create_job_record not available ({e}), using inline SQL");
            return handle_create_job_inline(
                pool, job_name, name, job_type, level,
                client_name, pool_name, fileset_name, sched_time,
            ).await;
        }
    };

    info!("Created job record: JobId={job_id} name={name} type={job_type} level={level}");
    CatalogResponse::JobId(job_id)
}

/// Update job end record.
async fn handle_update_job_end(
    pool: &sqlx::PgPool, job_id: i32, status: &str, end_time: i64,
    job_files: i64, job_bytes: i64, job_errors: i32,
    vol_session_id: i32, vol_session_time: i32,
) -> CatalogResponse {
    // Try SP first, fallback to inline SQL
    let result = sqlx::query(
        "SELECT sp_update_job_end($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(job_id).bind(status).bind(end_time)
    .bind(job_files).bind(job_bytes).bind(job_errors)
    .bind(vol_session_id).bind(vol_session_time)
    .execute(pool).await;

    match result {
        Ok(_) => {
            info!("Updated job {job_id}: status={status} files={job_files} bytes={job_bytes}");
            CatalogResponse::Ok(1)
        }
        Err(_) => {
            // Fallback: SP not installed
            match sqlx::query(
                "UPDATE Job SET JobStatus=$2, EndTime=to_timestamp($3::float8), \
                 RealEndTime=to_timestamp($3::float8), JobFiles=$4, JobBytes=$5, \
                 JobErrors=$6, VolSessionId=$7, VolSessionTime=$8 WHERE JobId=$1"
            )
            .bind(job_id).bind(status).bind(end_time)
            .bind(job_files).bind(job_bytes).bind(job_errors)
            .bind(vol_session_id).bind(vol_session_time)
            .execute(pool).await {
                Ok(_) => {
                    info!("Updated job {job_id}: status={status} files={job_files} bytes={job_bytes} (inline)");
                    CatalogResponse::Ok(1)
                }
                Err(e) => CatalogResponse::Error(format!("UpdateJobEnd: {e}")),
            }
        }
    }
}

/// Inline fallback when SP create_job_record is not yet installed.
async fn handle_create_job_inline(
    pool: &sqlx::PgPool,
    job_name: &str, name: &str, job_type: &str, level: &str,
    client_name: &str, pool_name: &str, fileset_name: &str, sched_time: i64,
) -> CatalogResponse {
    let client_id = resolve_or_create_client(pool, client_name).await;
    let pool_id = resolve_name_id(pool, "poolid", "pool", pool_name).await.unwrap_or(0);
    let fileset_id = resolve_name_id(pool, "filesetid", "fileset", fileset_name).await.unwrap_or(0);

    let job_id: i32 = match sqlx::query_scalar(
        "INSERT INTO Job (Job, Name, Type, Level, ClientId, JobStatus, SchedTime, \
         StartTime, RealEndTime, JobTDate, VolSessionId, VolSessionTime, \
         JobFiles, JobBytes, JobErrors, PoolId, FileSetId, PriorJobId, \
         HasBase, HasCache, Reviewed, Comment) \
         VALUES ($1, $2, $3, $4, $5, 'C', to_timestamp($6::float8), to_timestamp($6::float8), \
                 to_timestamp($6::float8), $6, 0, 0, 0, 0, 0, $7, $8, 0, 0, 0, 0, '') \
         RETURNING JobId"
    )
    .bind(job_name).bind(name).bind(job_type).bind(level)
    .bind(client_id).bind(sched_time).bind(pool_id).bind(fileset_id)
    .fetch_one(pool).await {
        Ok(id) => id,
        Err(e) => return CatalogResponse::Error(format!("CreateJob inline: {e}")),
    };

    info!("Created job record (inline): JobId={job_id} name={name}");
    CatalogResponse::JobId(job_id)
}

// Helper: resolve name → id
async fn resolve_name_id(pool: &sqlx::PgPool, id_col: &str, table: &str, name: &str) -> Option<i32> {
    let sql = format!("SELECT {id_col} FROM {table} WHERE name = $1");
    let row: Option<(i32,)> = sqlx::query_as(&sql).bind(name).fetch_optional(pool).await.ok()?;
    row.map(|(id,)| id)
}

// Helper: resolve or auto-create client
async fn resolve_or_create_client(pool: &sqlx::PgPool, client_name: &str) -> i32 {
    // Try to find existing
    if let Some(id) = resolve_name_id(pool, "clientid", "client", client_name).await {
        return id;
    }

    // Auto-create client record
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO Client (Name, Uname, AutoPrune, FileRetention, JobRetention) \
         VALUES ($1, '', 1, 2592000, 15552000) RETURNING ClientId"
    )
    .bind(client_name)
    .fetch_one(pool).await.unwrap_or(0);

    if id > 0 {
        info!("Auto-created client record: {client_name} → ClientId={id}");
    }
    id
}

#[cfg(test)]
mod backup_reference_tests {
    use super::*;

    #[test]
    fn reference_requests_are_scoped_and_errors_propagate() {
        for response in [CatalogResponse::SinceUtime { since_utime: 123, prev_job: "base".into() },
            CatalogResponse::Error("query failed".into())] {
            let (tx, rx) = flume::bounded(1);
            let handle = CatalogHandle { tx };
            let worker = std::thread::spawn(move || {
                let (request, reply) = rx.recv().unwrap();
                match request {
                    CatalogRequest::SinceUtime { client_name, job_name, fileset_name, level } => {
                        assert_eq!((client_name.as_str(), job_name.as_str(), fileset_name.as_str(), level.as_str()),
                            ("client", "backup", "set", "D"));
                    },
                    _ => panic!("wrong request"),
                }
                reply.send(response).unwrap();
            });
            let result = handle.since_utime("client", "backup", "set", "D");
            match result {
                Ok(Some((timestamp, name))) => assert_eq!((timestamp, name.as_str()), (123, "base")),
                Err(error) => assert_eq!(error, "query failed"),
                _ => panic!("unexpected result"),
            }
            worker.join().unwrap();
        }
    }
}

#[cfg(test)]
mod pq_error_tests {
    use super::*;

    #[test]
    fn error_position_like_libpq() {
        // captured from bareos-dir 25.1.2 with PostgreSQL (c016 parity cases)
        assert_eq!(report_error_position("selec nothing", 1), "LINE 1: selec nothing\n        ^\n");
        assert_eq!(report_error_position("select * from nosuchtable", 15),
            "LINE 1: select * from nosuchtable\n                      ^\n");
        assert_eq!(report_error_position("select 1\nfrom\tx y", 15), "LINE 2: from x y\n             ^\n");
        assert_eq!(report_error_position("x", 5), "");
    }

    #[test]
    fn long_lines_are_truncated() {
        let q = format!("select {} frm", "a, ".repeat(40));
        let out = report_error_position(&q, q.len() - 2);
        let first = out.lines().next().unwrap();
        assert!(first.starts_with("LINE 1: ...") && first.ends_with("frm"), "{out}");
        assert_eq!(first.chars().count(), "LINE 1: ...".len() + 60);
        let caret = out.lines().nth(1).unwrap();
        assert_eq!(caret.len(), first.len() - 2); // cursor under the "f" of "frm"
    }

    #[test]
    fn severity_word() {
        assert_eq!(localized_error_word("de_DE.UTF-8"), "FEHLER");
        assert_eq!(localized_error_word("C"), "ERROR");
        assert_eq!(localized_error_word("en_US.UTF-8"), "ERROR");
    }
}
