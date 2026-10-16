//! Bareos configuration resource types.

use std::time::Duration;

/// All resource types in a Bareos Director configuration.
#[derive(Debug, Clone)]
pub enum Resource {
    Director(DirectorResource),
    Client(ClientResource),
    Job(JobResource),
    Storage(StorageResource),
    Catalog(CatalogResource),
    Schedule(ScheduleResource),
    FileSet(FileSetResource),
    Pool(PoolResource),
    Console(ConsoleResource),
    JobDefs(JobDefsResource),
    Messages(MessagesResource),
    Profile(ProfileResource),
}

#[derive(Debug, Clone)]
pub struct DirectorResource {
    pub name: String,
    pub password: String,
    pub port: u16,
    pub max_concurrent_jobs: u32,
    pub fd_connect_timeout: Duration,
    pub sd_connect_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub enable_grpc: bool,
    /// gRPC port; None = BNET port + 1000
    pub grpc_port: Option<u16>,
    /// PEM files for the gRPC TLS endpoint; None = self-signed, generated on first start
    pub grpc_tls_certificate: Option<String>,
    pub grpc_tls_key: Option<String>,
    /// Announce the Director via mDNS (_onesimus-dir._tcp)
    pub mdns_advertise: bool,
    pub working_directory: String,
}

impl Default for DirectorResource {
    fn default() -> Self {
        Self {
            name: String::new(),
            password: String::new(),
            port: 9101,
            max_concurrent_jobs: 1,
            fd_connect_timeout: Duration::from_secs(180),
            sd_connect_timeout: Duration::from_secs(1800),
            heartbeat_interval: Duration::ZERO,
            enable_grpc: false,
            grpc_port: None,
            grpc_tls_certificate: None,
            grpc_tls_key: None,
            mdns_advertise: true,
            working_directory: "/var/lib/bareos".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientResource {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub password: String,
    pub catalog: String,
    pub max_concurrent_jobs: u32,
    pub enabled: bool,
    /// `Protocol` (dird_conf.cc:202, default Native).
    pub protocol: AuthProtocol,
    /// `AuthType` (NDMP authentication, default None).
    pub auth_type: AuthType,
    /// `Username` (NDMP account).
    pub username: Option<String>,
    /// `NdmpLogLevel` (default 4).
    pub ndmp_log_level: u32,
    /// `NdmpBlockSize` (default 64512).
    pub ndmp_block_size: u32,
    /// `NdmpUseLmdb` (default true).
    pub ndmp_use_lmdb: bool,
}

impl Default for ClientResource {
    fn default() -> Self {
        Self {
            name: String::new(),
            address: String::new(),
            port: 9102,
            password: String::new(),
            catalog: String::new(),
            max_concurrent_jobs: 1,
            enabled: true,
            protocol: AuthProtocol::Native,
            auth_type: AuthType::None,
            username: None,
            ndmp_log_level: 4,
            ndmp_block_size: 64512,
            ndmp_use_lmdb: true,
        }
    }
}

/// Storage/Client `Protocol` (C++ `AuthenticationProtocolType`,
/// include/auth_protocol_types.h; keywords dird_conf.cc:571 `authprotocols`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthProtocol {
    #[default]
    Native,
    NdmpV2,
    NdmpV3,
    NdmpV4,
}

impl AuthProtocol {
    /// Case-insensitive keyword lookup (`StoreAuthprotocoltype`).
    pub fn from_keyword(value: &str) -> Option<Self> {
        [("Native", Self::Native), ("NDMPV2", Self::NdmpV2), ("NDMPV3", Self::NdmpV3), ("NDMPV4", Self::NdmpV4)]
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(value))
            .map(|(_, p)| p)
    }

    /// `AuthenticationProtocolTypeToString()` (dird_conf.cc:2079).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "Native",
            Self::NdmpV2 => "NDMPV2",
            Self::NdmpV3 => "NDMPV3",
            Self::NdmpV4 => "NDMPV4",
        }
    }

    /// `case APT_NDMPV2: case APT_NDMPV3: case APT_NDMPV4:`
    pub fn is_ndmp(self) -> bool {
        self != Self::Native
    }
}

/// Storage/Client `AuthType` (keywords dird_conf.cc `authmethods`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthType {
    #[default]
    None,
    Clear,
    Md5,
}

impl AuthType {
    /// Case-insensitive keyword lookup (`StoreAuthtype`).
    pub fn from_keyword(value: &str) -> Option<Self> {
        [("None", Self::None), ("Clear", Self::Clear), ("MD5", Self::Md5)]
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(value))
            .map(|(_, t)| t)
    }
}

#[derive(Debug, Clone)]
pub struct JobResource {
    pub name: String,
    pub job_type: JobType,
    pub level: JobLevel,
    pub client: String,
    pub fileset: String,
    pub schedule: String,
    pub storage: String,
    pub pool: String,
    pub full_backup_pool: String,
    pub differential_backup_pool: String,
    pub incremental_backup_pool: String,
    pub messages: String,
    pub priority: i32,
    pub max_concurrent_jobs: u32,
    pub enabled: bool,
    pub write_bootstrap: String,
    /// Name of the JobDefs resource to inherit from
    pub jobdefs: String,
    pub run_before_job: String,
    pub run_after_job: String,
    /// Items written in the resource or inherited from a JobDefs
    /// (`IsMemberPresent`, normalized item names, see `jobdefs::item_key`).
    pub present: std::collections::BTreeSet<String>,
}

impl Default for JobResource {
    fn default() -> Self {
        Self {
            name: String::new(),
            job_type: JobType::Backup,
            level: JobLevel::Full,
            client: String::new(),
            fileset: String::new(),
            schedule: String::new(),
            storage: String::new(),
            pool: String::new(),
            full_backup_pool: String::new(),
            differential_backup_pool: String::new(),
            incremental_backup_pool: String::new(),
            messages: String::new(),
            priority: 10,
            max_concurrent_jobs: 1,
            enabled: true,
            write_bootstrap: String::new(),
            jobdefs: String::new(),
            run_before_job: String::new(),
            run_after_job: String::new(),
            present: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobType {
    Backup,
    Restore,
    Verify,
    Admin,
    Archive,
    Copy,
    Migrate,
    Consolidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobLevel {
    Full,
    Incremental,
    Differential,
    VirtualFull,
    InitCatalog,
    Catalog,
    VolumeToCatalog,
    DiskToCatalog,
    Data,
}

#[derive(Debug, Clone)]
pub struct StorageResource {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub password: String,
    pub device: String,
    pub media_type: String,
    pub max_concurrent_jobs: u32,
    pub enabled: bool,
    /// `Protocol` (dird_conf.cc:237, default Native).
    pub protocol: AuthProtocol,
    /// `AuthType` (NDMP authentication, default None).
    pub auth_type: AuthType,
    /// `Username` (NDMP account).
    pub username: Option<String>,
    /// `PairedStorage`: native storage daemon serving this NDMP storage.
    pub paired_storage: Option<String>,
    /// `NdmpChangerDevice` (NDMP_NATIVE robot).
    pub ndmp_changer_device: Option<String>,
}

impl Default for StorageResource {
    fn default() -> Self {
        Self {
            name: String::new(),
            address: String::new(),
            port: 9103,
            password: String::new(),
            device: String::new(),
            media_type: String::new(),
            max_concurrent_jobs: 1,
            enabled: true,
            protocol: AuthProtocol::Native,
            auth_type: AuthType::None,
            username: None,
            paired_storage: None,
            ndmp_changer_device: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CatalogResource {
    pub name: String,
    pub db_driver: String,
    pub db_name: String,
    pub db_address: String,
    pub db_port: u16,
    pub db_user: String,
    pub db_password: String,
}

impl Default for CatalogResource {
    fn default() -> Self {
        Self {
            name: String::new(),
            db_driver: "postgresql".into(),
            db_name: "bareos".into(),
            db_address: "localhost".into(),
            db_port: 5432,
            db_user: String::new(),
            db_password: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScheduleResource {
    pub name: String,
    pub enabled: bool,
    pub runs: Vec<RunDirective>,
}

#[derive(Debug, Clone)]
pub struct RunDirective {
    pub level: Option<JobLevel>,
    pub pool: Option<String>,
    pub storage: Option<String>,
    pub messages: Option<String>,
    pub priority: Option<i32>,
    pub spool_data: Option<bool>,
    pub accurate: Option<bool>,
    /// First hour of `at HH:MM` (0 without `at`); the full set is `mask.hour`.
    pub hour: u32,
    pub minute: u32,
    /// When the run triggers (C++ RunResource::date_time_mask).
    pub mask: DateTimeMask,
}

/// C++ dird/date_time_mask.h: one bit per hour, day of month (0-based),
/// month (0-based), weekday (0 = Sunday), week of month (0-4) and ISO-like
/// week of year (0-53).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DateTimeMask {
    pub hour: u32,
    pub mday: u32,
    pub month: u16,
    pub wday: u8,
    pub wom: u8,
    pub woy: u64,
    pub last_7days_of_month: bool,
}

impl DateTimeMask {
    /// run_conf.cc set_defaults(): every hour, day, month and week.
    pub fn all() -> Self {
        Self {
            hour: (1 << 24) - 1,
            mday: (1 << 31) - 1,
            month: (1 << 12) - 1,
            wday: (1 << 7) - 1,
            wom: (1 << 5) - 1,
            woy: (1 << 54) - 1,
            last_7days_of_month: false,
        }
    }

    pub fn has_hour(&self, h: i32) -> bool { bit(self.hour as u64, h) }
    pub fn has_mday(&self, d: i32) -> bool { bit(self.mday as u64, d) }
    pub fn has_month(&self, m: i32) -> bool { bit(self.month as u64, m) }
    pub fn has_wday(&self, d: i32) -> bool { bit(self.wday as u64, d) }
    pub fn has_wom(&self, w: i32) -> bool { bit(self.wom as u64, w) }
    pub fn has_woy(&self, w: i32) -> bool { bit(self.woy, w) }
}

fn bit(mask: u64, n: i32) -> bool {
    (0..64).contains(&n) && mask & (1 << n) != 0
}

#[derive(Debug, Clone)]
pub struct FileSetResource {
    pub name: String,
    pub include_blocks: Vec<String>,
    pub exclude_blocks: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PoolResource {
    pub name: String,
    pub pool_type: String,
    pub recycle: bool,
    pub auto_prune: bool,
    pub volume_retention: Duration,
    pub maximum_volume_bytes: u64,
    pub maximum_volumes: u32,
    pub label_format: String,
}

#[derive(Debug, Clone)]
pub struct ConsoleResource {
    pub name: String,
    pub password: String,
}

/// JobDefs resource — template for Job resources.
/// Contains a full JobResource that serves as default values.
#[derive(Debug, Clone)]
pub struct JobDefsResource {
    pub name: String,
    pub job: JobResource,
}

/// Messages resource — message routing configuration.
#[derive(Debug, Clone)]
pub struct MessagesResource {
    pub name: String,
}

/// Profile resource — ACL profile for Console access control.
#[derive(Debug, Clone)]
pub struct ProfileResource {
    pub name: String,
}
