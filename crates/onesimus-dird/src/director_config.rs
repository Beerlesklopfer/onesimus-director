//! Runtime configuration for the Director.
//!
//! Wraps the parsed config resources into an Arc<RwLock<>> for
//! thread-safe access and hot-reload support.

use std::path::Path;
use std::sync::{Arc, RwLock};

use bareos_config::resources::*;
use bareos_config::parser::{self, ConfigError};

/// All parsed configuration resources, shareable across threads.
/// Every vector keeps the C++ resource-chain order (config files in reverse
/// glob order, see `parser::parse_config_dir`), which listings show unchanged.
#[derive(Debug, Clone)]
pub struct DirectorConfig {
    pub director: DirectorResource,
    pub clients: Vec<ClientResource>,
    pub jobs: Vec<JobResource>,
    pub storages: Vec<StorageResource>,
    pub catalogs: Vec<CatalogResource>,
    pub schedules: Vec<ScheduleResource>,
    pub pools: Vec<PoolResource>,
    pub filesets: Vec<FileSetResource>,
    pub consoles: Vec<ConsoleResource>,
    pub jobdefs: Vec<JobDefsResource>,
    pub messages: Vec<MessagesResource>,
    pub profiles: Vec<ProfileResource>,
    /// Raw directives in C++ resource order (for `show`)
    pub raw: Vec<bareos_config::raw::ConfResource>,
}

impl DirectorConfig {
    /// Load configuration from a file or directory tree.
    /// If path is a directory (e.g. /etc/bareos/bareos-dir.d/), reads all *.conf recursively.
    /// If path is a file, reads it as a single config file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let mut config = if path.is_dir() {
            let resources = parser::parse_config_dir(path)?;
            Self::from_resources(resources)?
        } else {
            let content = std::fs::read_to_string(path)
                .map_err(|e| ConfigError::ParseError(format!("cannot read {}: {e}", path.display())))?;
            Self::from_str(&content)?
        };
        match bareos_config::raw::load(path) {
            Ok(raw) => config.raw = raw,
            Err(e) => tracing::warn!("raw config view unavailable (show): {e}"),
        }
        Ok(config)
    }

    /// Parse configuration from a string.
    pub fn from_str(input: &str) -> Result<Self, ConfigError> {
        let resources = parser::parse_config(input)?;
        let mut config = Self::from_resources(resources)?;
        config.raw = bareos_config::raw::parse_str(input, None).unwrap_or_default();
        Ok(config)
    }

    /// Build config from parsed resources.
    fn from_resources(resources: Vec<bareos_config::Resource>) -> Result<Self, ConfigError> {
        let mut config = DirectorConfig {
            director: DirectorResource::default(),
            clients: Vec::new(),
            jobs: Vec::new(),
            storages: Vec::new(),
            catalogs: Vec::new(),
            schedules: Vec::new(),
            pools: Vec::new(),
            filesets: Vec::new(),
            consoles: Vec::new(),
            jobdefs: Vec::new(),
            messages: Vec::new(),
            profiles: Vec::new(),
            raw: Vec::new(),
        };

        for resource in resources {
            match resource {
                Resource::Director(d) => config.director = d,
                Resource::Client(c) => config.clients.push(c),
                Resource::Job(j) => config.jobs.push(j),
                Resource::Storage(s) => config.storages.push(s),
                Resource::Catalog(c) => config.catalogs.push(c),
                Resource::Schedule(s) => config.schedules.push(s),
                Resource::Pool(p) => config.pools.push(p),
                Resource::FileSet(f) => config.filesets.push(f),
                Resource::Console(c) => {
                    // Skip dummy entries from unknown resource types
                    if !c.name.starts_with("_skip_") {
                        config.consoles.push(c);
                    }
                }
                Resource::JobDefs(j) => config.jobdefs.push(j),
                Resource::Messages(m) => config.messages.push(m),
                Resource::Profile(p) => config.profiles.push(p),
            }
        }

        // Resolve JobDefs: merge defaults into Jobs that reference a JobDefs
        config.resolve_jobdefs();

        Ok(config)
    }

    /// Merge JobDefs defaults into Jobs.
    /// For each Job with a non-empty `jobdefs` field, copy missing values
    /// from the referenced JobDefs resource.
    fn resolve_jobdefs(&mut self) {
        // Clone jobdefs list to avoid borrow conflicts
        let defs: Vec<_> = self.jobdefs.clone();

        for job in &mut self.jobs {
            if job.jobdefs.is_empty() { continue; }

            let def = match defs.iter().find(|d| d.name == job.jobdefs) {
                Some(d) => &d.job,
                None => {
                    tracing::warn!("Job '{}': JobDefs '{}' not found", job.name, job.jobdefs);
                    continue;
                }
            };

            // Merge: Job-Feld leer → aus JobDefs uebernehmen
            if job.client.is_empty() { job.client = def.client.clone(); }
            if job.fileset.is_empty() { job.fileset = def.fileset.clone(); }
            if job.schedule.is_empty() { job.schedule = def.schedule.clone(); }
            if job.storage.is_empty() { job.storage = def.storage.clone(); }
            if job.pool.is_empty() { job.pool = def.pool.clone(); }
            if job.full_backup_pool.is_empty() { job.full_backup_pool = def.full_backup_pool.clone(); }
            if job.differential_backup_pool.is_empty() { job.differential_backup_pool = def.differential_backup_pool.clone(); }
            if job.incremental_backup_pool.is_empty() { job.incremental_backup_pool = def.incremental_backup_pool.clone(); }
            if job.messages.is_empty() { job.messages = def.messages.clone(); }
            if job.write_bootstrap.is_empty() { job.write_bootstrap = def.write_bootstrap.clone(); }
            // Type und Level nur uebernehmen wenn Job sie nicht explizit setzt
            // (Default = Backup/Full — wenn Job diese hat und JobDefs andere, behaelt Job)
            if job.job_type == JobType::Backup && def.job_type != JobType::Backup {
                job.job_type = def.job_type;
            }
            if job.level == JobLevel::Full && def.level != JobLevel::Full {
                job.level = def.level;
            }
            // Priority nur wenn Default (10)
            if job.priority == 10 && def.priority != 10 {
                job.priority = def.priority;
            }

            tracing::debug!("Job '{}': merged from JobDefs '{}' (client={}, fileset={}, storage={}, pool={})",
                job.name, job.jobdefs, job.client, job.fileset, job.storage, job.pool);
        }
    }

    /// Find a client by name.
    pub fn find_client(&self, name: &str) -> Option<&ClientResource> {
        self.clients.iter().find(|c| c.name == name)
    }

    /// Find a job by name.
    pub fn find_job(&self, name: &str) -> Option<&JobResource> {
        self.jobs.iter().find(|j| j.name == name)
    }

    /// Find a storage by name.
    pub fn find_storage(&self, name: &str) -> Option<&StorageResource> {
        self.storages.iter().find(|s| s.name == name)
    }

    /// Find a fileset by name.
    pub fn find_fileset(&self, name: &str) -> Option<&FileSetResource> {
        self.filesets.iter().find(|f| f.name == name)
    }

    /// Find a pool by name.
    pub fn find_pool(&self, name: &str) -> Option<&PoolResource> {
        self.pools.iter().find(|p| p.name == name)
    }

    /// Find a console by name.
    pub fn find_console(&self, name: &str) -> Option<&ConsoleResource> {
        self.consoles.iter().find(|c| c.name == name)
    }
}

/// Name of the Console resource the C++ Director adds after loading the
/// configuration (dird_conf.cc CreateAndAddUserAgentConsoleResource): it is
/// appended to the end of the Console chain whenever a Director resource
/// exists, so `foreach_res(R_CONSOLE)` listings end with it.
pub const USER_AGENT_CONSOLE: &str = "*UserAgent*";

/// Resource names for a selection prompt (ua_select.cc StartPrompt lists):
/// the C++ Director collects the names in resource-chain order and applies
/// SortCaseInsensitive (lib/util.cc) — ASCII lowercase, byte comparison.
/// Plain listings (dot commands, `show`) keep the chain order of the vectors
/// in [`DirectorConfig`] instead.
pub fn sort_case_insensitive<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let mut names: Vec<&str> = names.into_iter().collect();
    names.sort_by_cached_key(|name| name.to_ascii_lowercase());
    names
}

/// Thread-safe shared config handle.
pub type SharedConfig = Arc<RwLock<DirectorConfig>>;

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_CONFIG: &str = r#"
Director {
  Name = bareos-dir
  Port = 9101
  Password = "dirpass"
  Maximum Concurrent Jobs = 10
  Enable Grpc = yes
}

Client {
  Name = bareos-fd
  Address = localhost
  Port = 9102
  Password = "clientpass"
  Catalog = MyCatalog
}

Client {
  Name = remote-fd
  Address = 10.0.0.2
  Port = 9102
  Password = "remotepass"
  Catalog = MyCatalog
}

Storage {
  Name = File
  Address = localhost
  Port = 9103
  Password = "storpass"
  Device = FileStorage
  Media Type = File
}

Catalog {
  Name = MyCatalog
  dbname = bareos
  dbuser = bareos
  dbpassword = "dbpass"
}

Job {
  Name = "BackupClient1"
  Type = Backup
  Level = Full
  Client = bareos-fd
  FileSet = "Full Set"
  Storage = File
  Pool = Full
  Priority = 10
}

Pool {
  Name = Full
  Pool Type = Backup
  Recycle = yes
  AutoPrune = yes
  Volume Retention = 365 days
  Maximum Volume Bytes = 50G
  Maximum Volumes = 100
  Label Format = "Full-"
}

Schedule {
  Name = "WeeklyCycle"
  Run = Full 1st sun at 23:05
  Run = Differential 2nd-5th sun at 23:05
  Run = Incremental mon-sat at 23:05
}

Console {
  Name = admin
  Password = "adminpass"
}
"#;

    #[test]
    fn load_full_config() {
        let config = DirectorConfig::from_str(TEST_CONFIG).unwrap();

        assert_eq!(config.director.name, "bareos-dir");
        assert_eq!(config.director.port, 9101);
        assert_eq!(config.director.password, "dirpass");
        assert_eq!(config.director.max_concurrent_jobs, 10);
        assert!(config.director.enable_grpc);

        assert_eq!(config.clients.len(), 2);
        assert_eq!(config.clients[0].name, "bareos-fd");
        assert_eq!(config.clients[1].name, "remote-fd");

        assert_eq!(config.storages.len(), 1);
        assert_eq!(config.storages[0].name, "File");

        assert_eq!(config.catalogs.len(), 1);
        assert_eq!(config.catalogs[0].db_name, "bareos");

        assert_eq!(config.jobs.len(), 1);
        assert_eq!(config.jobs[0].name, "BackupClient1");

        assert_eq!(config.pools.len(), 1);
        assert_eq!(config.pools[0].name, "Full");
        assert_eq!(config.pools[0].maximum_volumes, 100);

        assert_eq!(config.schedules.len(), 1);
        assert_eq!(config.schedules[0].runs.len(), 3);

        assert_eq!(config.consoles.len(), 1);
        assert_eq!(config.consoles[0].name, "admin");
    }

    #[test]
    fn selection_names_sort_case_insensitively() {
        // ua_select.cc select_job_resource: chain order in, SortCaseInsensitive out
        let chain = ["backup-bareos-fd", "RestoreFiles", "BackupCatalog"];
        assert_eq!(sort_case_insensitive(chain), ["backup-bareos-fd", "BackupCatalog", "RestoreFiles"]);
    }

    #[test]
    fn find_resources() {
        let config = DirectorConfig::from_str(TEST_CONFIG).unwrap();

        assert!(config.find_client("bareos-fd").is_some());
        assert!(config.find_client("nonexistent").is_none());

        assert!(config.find_job("BackupClient1").is_some());
        assert!(config.find_storage("File").is_some());
        assert!(config.find_pool("Full").is_some());
        assert!(config.find_console("admin").is_some());
    }

    #[test]
    fn shared_config_thread_safe() {
        let config = DirectorConfig::from_str(TEST_CONFIG).unwrap();
        let shared: SharedConfig = Arc::new(RwLock::new(config));

        let handles: Vec<_> = (0..10).map(|_| {
            let c = shared.clone();
            std::thread::spawn(move || {
                let config = c.read().unwrap();
                assert_eq!(config.director.name, "bareos-dir");
            })
        }).collect();

        for h in handles {
            h.join().unwrap();
        }
    }
}
