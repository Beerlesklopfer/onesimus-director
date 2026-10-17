//! Storage daemon read+write session of the jobs that copy data from
//! volume to volume without a File daemon (Compatibility log C-058, S-05):
//! Copy/Migrate worker jobs (migrate.cc DoActualMigration, local replicate)
//! and VirtualFull backups (vbackup.cc). Runs in the forked job worker:
//! msgchan.cc StartStorageDaemonJob (Job command, bootstrap, secure erase),
//! ReserveReadAndWriteDevices, "run", then the Storage daemon message
//! thread (catalog requests over the worker pipe) until "3099 Job end".

/// One Storage resource as ReserveReadDevice/ReserveWriteDevice send it.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StorageUse {
    /// Storage resource name (`use storage=`).
    pub name: String,
    pub address: String,
    pub port: u16,
    pub password: String,
    pub media_type: String,
    /// Device names of the Storage resource (`use device=` each).
    pub devices: Vec<String>,
    /// Pool name and Pool Type (read: rpool, write: pool).
    pub pool: String,
    pub pool_type: String,
}

/// What the worker needs for the SD session (JobParams.copy).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CopySession {
    pub read: StorageUse,
    pub write: StorageUse,
    /// Bootstrap file content (SendBootstrapFileToSd).
    pub bootstrap: String,
    /// Job command `type=` (jcr->getJobType(): 'c', 'g', or 'B').
    pub job_type: char,
    /// Job command `level=` (jcr->getJobLevel()).
    pub level: char,
    /// Job command job_name= (Job resource), client_name=, FileSet=.
    pub job_resource: String,
    pub client: String,
    pub fileset: String,
    pub fileset_md5: String,
    /// !pool->catalog_files
    pub no_attributes: bool,
    pub spool_attributes: bool,
    pub spool_data: bool,
    pub prefer_mounted_volumes: bool,
    pub spool_size: i64,
    /// Job messages the worker sends first (M_* code, text).
    #[serde(default)]
    pub start_messages: Vec<(u32, String)>,
    /// End the job JS_FatalError after the start messages, without the SD.
    #[serde(default)]
    pub fail_before_sd: bool,
}
