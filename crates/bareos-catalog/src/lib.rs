//! PostgreSQL catalog interface for the Bareos Director.
//!
//! Provides async database access for job records, media/volume management,
//! file catalog, and raw SQL queries.

pub mod records;
pub mod queries;

pub use records::*;

/// Unique job identifier.
pub type JobId = i32;

/// Unique volume/media identifier.
pub type MediaId = i32;

/// Database connection error.
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("configuration error: {0}")]
    Config(String),
}

/// Catalog database handle.
pub struct CatalogDb {
    pool: sqlx::PgPool,
}

impl CatalogDb {
    /// Connect to the catalog database.
    pub async fn connect(connection_string: &str) -> Result<Self, CatalogError> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(10)
            .connect(connection_string)
            .await?;
        Ok(Self { pool })
    }

    /// Connect with individual parameters.
    pub async fn connect_params(
        host: &str,
        port: u16,
        dbname: &str,
        user: &str,
        password: &str,
    ) -> Result<Self, CatalogError> {
        let conn_str = format!(
            "host={host} port={port} dbname={dbname} user={user} password={password}"
        );
        Self::connect(&conn_str).await
    }

    /// Access the connection pool.
    pub fn pool(&self) -> &sqlx::PgPool {
        &self.pool
    }

    // --- Job Records ---

    /// Create a new job record.
    pub async fn create_job(&self, job: &NewJobRecord) -> Result<JobId, CatalogError> {
        let rec = sqlx::query_scalar::<_, i32>(
            "INSERT INTO Job (Job, Name, Type, Level, ClientId, JobStatus, SchedTime, \
             StartTime, RealEndTime, JobTDate, VolSessionId, VolSessionTime, \
             JobFiles, JobBytes, JobErrors, PoolId, FileSetId, PriorJobId, \
             HasBase, HasCache, Reviewed, Comment) \
             VALUES ($1, $2, $3, $4, $5, $6, to_timestamp($7::float8), to_timestamp($8::float8), to_timestamp($9::float8), $10, 0, 0, 0, 0, 0, \
                     $11, $12, 0, 0, 0, 0, '') \
             RETURNING JobId"
        )
        .bind(&job.job)
        .bind(&job.name)
        .bind(&job.job_type)
        .bind(&job.level)
        .bind(job.client_id)
        .bind(&job.job_status)
        .bind(job.sched_time)
        .bind(job.start_time)
        .bind(job.end_time)
        .bind(job.job_tdate)
        .bind(job.pool_id)
        .bind(job.fileset_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(rec)
    }

    /// Get a job record by ID.
    pub async fn get_job(&self, job_id: JobId) -> Result<JobRecord, CatalogError> {
        sqlx::query_as::<_, JobRecord>(
            "SELECT JobId, Job, Name, Type, Level, ClientId, JobStatus, \
             SchedTime::text AS schedtime, StartTime::text AS starttime, \
             EndTime::text AS endtime, RealEndTime::text AS realendtime, JobTDate, \
             VolSessionId, VolSessionTime, JobFiles, JobBytes, JobErrors, \
             PoolId, FileSetId, PriorJobId, Comment \
             FROM Job WHERE JobId = $1"
        )
        .bind(job_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| CatalogError::NotFound(format!("job {job_id}")))
    }

    /// Update job end record.
    pub async fn update_job_end(
        &self,
        job_id: JobId,
        status: &str,
        end_time: i64,
        job_files: i64,
        job_bytes: i64,
        job_errors: i32,
    ) -> Result<(), CatalogError> {
        sqlx::query(
            "UPDATE Job SET JobStatus=$2, EndTime=to_timestamp($3::float8), \
             RealEndTime=to_timestamp($3::float8), JobFiles=$4, JobBytes=$5, \
             JobErrors=$6 WHERE JobId=$1"
        )
        .bind(job_id)
        .bind(status)
        .bind(end_time)
        .bind(job_files)
        .bind(job_bytes)
        .bind(job_errors)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // --- Raw SQL ---

    /// Execute a raw SQL query.
    pub async fn raw_query(&self, sql: &str) -> Result<Vec<Vec<(String, String)>>, CatalogError> {
        let rows = sqlx::query(sql)
            .fetch_all(&self.pool)
            .await?;

        let mut result = Vec::new();
        for row in &rows {
            use sqlx::{Row, Column};
            let columns = row.columns();
            let mut row_data = Vec::new();
            for col in columns {
                let name = col.name().to_string();
                let value: String = row.try_get::<String, _>(col.ordinal())
                    .unwrap_or_else(|_| {
                        row.try_get::<i64, _>(col.ordinal())
                            .map(|v| v.to_string())
                            .unwrap_or_else(|_| "NULL".to_string())
                    });
                row_data.push((name, value));
            }
            result.push(row_data);
        }
        Ok(result)
    }
}
