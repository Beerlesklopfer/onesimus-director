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
    /// Consoles read from the configuration files; the C++ Director appends
    /// `*UserAgent*` after them, and `configure add console` after that.
    pub loaded_consoles: usize,
    /// Raw directives in C++ resource order (for `show`)
    pub raw: Vec<bareos_config::raw::ConfResource>,
    /// Parsed by `reload` rather than at startup. The C++ parser then runs
    /// with `omit_defaults_` (set by the first CheckResources()), which
    /// changes what `show` prints (compatibility log C-057).
    pub reloaded: bool,
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
            loaded_consoles: 0,
            raw: Vec::new(),
            reloaded: false,
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
        config.loaded_consoles = config.consoles.len();
        config.resolve_jobdefs();

        Ok(config)
    }

    /// PopulateJobdefaults() (dird_conf.cc): JobDefs take the items of their
    /// JobDefs in chain order, then Jobs take the items of their JobDefs; an
    /// item written in the resource always wins (see `bareos_config::jobdefs`).
    fn resolve_jobdefs(&mut self) {
        for job in &self.jobs {
            if !job.jobdefs.is_empty() && !self.jobdefs.iter().any(|d| d.name == job.jobdefs) {
                tracing::warn!("Job '{}': JobDefs '{}' not found", job.name, job.jobdefs);
            }
        }
        bareos_config::jobdefs::propagate_typed(&mut self.jobdefs, &mut self.jobs);
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
    fn explicit_job_items_win_over_jobdefs() {
        // C-041: BackupCatalog sets Level = Full and Priority explicitly
        let config = DirectorConfig::from_str(r#"
JobDefs { Name = "DefaultJob"; Type = Backup; Level = Incremental; Client = fd; FileSet = "SelfTest"
  Schedule = "WeeklyCycle"; Storage = File; Messages = Standard; Pool = Incremental; Priority = 10 }
Job { Name = "BackupCatalog"; JobDefs = "DefaultJob"; Level = Full; FileSet = "Catalog"; Priority = 11 }
Job { Name = "Other"; JobDefs = "Late"; Priority = 10; Type = Backup }
JobDefs { Name = "Late"; Type = Restore; Level = Differential; Priority = 3 }
"#).unwrap();
        let bc = config.find_job("BackupCatalog").unwrap();
        assert_eq!(bc.level, JobLevel::Full);
        assert_eq!(bc.fileset, "Catalog");
        assert_eq!(bc.priority, 11);
        assert_eq!(bc.pool, "Incremental");
        assert_eq!(bc.schedule, "WeeklyCycle");
        let other = config.find_job("Other").unwrap();
        assert_eq!(other.level, JobLevel::Differential);
        assert_eq!(other.priority, 10);
        assert_eq!(other.job_type, JobType::Backup);
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
