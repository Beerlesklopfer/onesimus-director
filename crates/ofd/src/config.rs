//! FD configuration — parsed from bareos-fd.d/ or bareos-fd.conf
//! (filed/filed_conf.cc: Client/FileDaemon, Director, Messages).

use std::path::Path;
use tracing::debug;

/// A Director allowed to connect to this FD.
#[derive(Debug, Clone, Default)]
pub struct DirectorAuth {
    pub name: String,
    pub password: String,
    pub monitor: bool,
    /// MaximumBandwidthPerJob (bytes/s, 0 = none)
    pub max_bandwidth_per_job: u64,
    /// AllowedScriptDir (empty: any)
    pub allowed_script_dirs: Vec<String>,
    /// AllowedJobCommand (empty: all)
    pub allowed_job_cmds: Vec<String>,
}

/// A Messages resource: the types sent to the Director and append files.
#[derive(Debug, Clone, Default)]
pub struct MessagesRes {
    pub name: String,
    /// Bit i set: type i goes to the Director.
    pub director_types: u64,
    /// (file, types)
    pub append: Vec<(String, u64)>,
}

/// File Daemon configuration.
#[derive(Debug, Clone)]
pub struct FdConfig {
    pub name: String,
    pub port: u16,
    pub max_concurrent_jobs: u32,
    pub directors: Vec<DirectorAuth>,
    pub heartbeat_interval_secs: u64,
    /// "SD Connect Timeout" (default 30 min)
    pub sd_connect_timeout_secs: u64,
    pub working_directory: String,
    /// MaximumNetworkBufferSize (0: default 256 KiB)
    pub max_network_buffer_size: u32,
    /// MaximumWorkersPerJob (default 2)
    pub max_workers_per_job: u32,
    pub max_bandwidth_per_job: u64,
    pub allow_bw_bursting: bool,
    pub allowed_script_dirs: Vec<String>,
    pub allowed_job_cmds: Vec<String>,
    /// Client `Messages =`, else the first Messages resource.
    pub messages: Option<MessagesRes>,
    messages_name: Option<String>,
    all_messages: Vec<MessagesRes>,
}

impl FdConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let content = if path.is_dir() {
            // Read all .conf files from directory tree
            let mut all = String::new();
            walk_conf_dir(path, &mut all)?;
            all
        } else {
            std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?
        };

        Self::parse(&content)
    }

    pub(crate) fn parse(input: &str) -> Result<Self, String> {
        let mut config = FdConfig {
            name: "bareos-fd".to_string(),
            port: 9102,
            max_concurrent_jobs: 20,
            directors: Vec::new(),
            heartbeat_interval_secs: 0,
            sd_connect_timeout_secs: 1800,
            working_directory: "/var/lib/bareos".to_string(),
            max_network_buffer_size: 0,
            max_workers_per_job: 2,
            max_bandwidth_per_job: 0,
            allow_bw_bursting: false,
            allowed_script_dirs: Vec::new(),
            allowed_job_cmds: Vec::new(),
            messages: None,
            messages_name: None,
            all_messages: Vec::new(),
        };

        let cleaned = strip_comments(input);
        let bytes = cleaned.as_bytes();
        let mut current_block = Vec::new();
        let mut in_block = false;
        let mut brace_depth = 0;
        let mut block_type = String::new();

        let mut pos = 0;
        while pos < bytes.len() {
            if !in_block {
                let rest = &cleaned[pos..];
                if let Some(brace_pos) = rest.find('{') {
                    block_type = rest[..brace_pos].trim().to_string();
                    in_block = true;
                    brace_depth = 1;
                    current_block.clear();
                    pos += brace_pos + 1;
                    continue;
                } else {
                    break;
                }
            }
            let ch = bytes[pos];
            if ch == b'{' {
                brace_depth += 1;
                current_block.push(ch);
            } else if ch == b'}' {
                brace_depth -= 1;
                if brace_depth == 0 {
                    parse_block(&block_type, &String::from_utf8_lossy(&current_block), &mut config);
                    in_block = false;
                } else {
                    current_block.push(ch);
                }
            } else {
                current_block.push(ch);
            }
            pos += 1;
        }

        if config.directors.is_empty() {
            return Err("No Director resources found in configuration".to_string());
        }
        config.messages = match &config.messages_name {
            Some(n) => config.all_messages.iter().find(|m| m.name == *n).cloned(),
            None => config.all_messages.first().cloned(),
        };

        Ok(config)
    }

    /// Find director auth by name.
    pub fn find_director(&self, name: &str) -> Option<&DirectorAuth> {
        self.directors.iter().find(|d| d.name == name)
    }

    /// The network buffer size (DEFAULT_NETWORK_BUFFER_SIZE 256 KiB).
    pub fn buf_size(&self) -> usize {
        if self.max_network_buffer_size > 0 { self.max_network_buffer_size as usize } else { 256 * 1024 }
    }
}

/// A Bareos time value: "N", "N sec", "N min", "N hours" ... in seconds.
fn parse_duration_secs(v: &str) -> Option<u64> {
    let v = v.trim().to_lowercase();
    let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: u64 = digits.parse().ok()?;
    let unit = v[digits.len()..].trim();
    let mult = match unit.chars().next() {
        None | Some('s') => 1,
        Some('m') if unit.starts_with("mo") => 30 * 86400,
        Some('m') => 60,
        Some('h') => 3600,
        Some('d') => 86400,
        Some('w') => 7 * 86400,
        Some('y') => 365 * 86400,
        _ => return None,
    };
    Some(n * mult)
}

/// CFG_TYPE_SPEED: "N", "N k/s", "N kb/s", "N m/s", "N mb/s" in bytes/s.
pub fn parse_speed(v: &str) -> Option<u64> {
    let v = v.trim().to_lowercase();
    let digits: String = v.chars().take_while(|c| c.is_ascii_digit()).collect();
    let n: u64 = digits.parse().ok()?;
    let mult = match v[digits.len()..].trim() {
        "" => 1,
        "k/s" => 1024,
        "kb/s" => 1000,
        "m/s" => 1024 * 1024,
        "mb/s" => 1_000_000,
        _ => return None,
    };
    Some(n * mult)
}

/// lib/generic_res.h msg_types (M_MAX = M_AUDIT = 17).
fn msg_type_code(name: &str) -> Option<u32> {
    Some(match name.to_ascii_lowercase().as_str() {
        "debug" => 2,
        "abort" => 1,
        "fatal" => 3,
        "error" => 4,
        "warning" => 5,
        "info" => 6,
        "saved" => 7,
        "notsaved" => 8,
        "skipped" => 9,
        "mount" => 10,
        "terminate" => 12,
        "restored" => 13,
        "security" => 14,
        "alert" => 15,
        "volmgmt" => 16,
        "audit" => 17,
        "all" => 18,
        _ => return None,
    })
}

/// ScanTypes(): "all, !skipped, !restored"
pub fn scan_types(list: &str) -> u64 {
    let mut bits = 0u64;
    for t in list.split(',') {
        let t = t.trim();
        let (not, name) = match t.strip_prefix('!') {
            Some(n) => (true, n),
            None => (false, t),
        };
        match msg_type_code(name) {
            Some(18) => {
                for i in 1..=17 {
                    bits |= 1 << i;
                }
            }
            Some(c) if not => bits &= !(1u64 << c),
            Some(c) => bits |= 1u64 << c,
            None => {}
        }
    }
    bits
}

fn values(v: &str) -> Vec<String> {
    v.split(',').map(|s| s.trim().trim_matches('"').to_string()).filter(|s| !s.is_empty()).collect()
}

fn parse_block(block_type: &str, content: &str, config: &mut FdConfig) {
    let list = parse_directive_list(content);
    let get = |k: &str| list.iter().rev().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let all = |k: &str| list.iter().filter(|(key, _)| key == k).flat_map(|(_, v)| values(v)).collect::<Vec<_>>();

    match block_type.to_lowercase().as_str() {
        "client" | "filedaemon" => {
            if let Some(v) = get("name") { config.name = v; }
            if let Some(v) = get("port").or_else(|| get("fdport")) {
                config.port = v.parse().unwrap_or(9102);
            }
            if let Some(v) = get("maximumconcurrentjobs") {
                config.max_concurrent_jobs = v.parse().unwrap_or(20);
            }
            if let Some(v) = get("heartbeatinterval") {
                config.heartbeat_interval_secs = parse_duration_secs(&v).unwrap_or(0);
            }
            if let Some(v) = get("sdconnecttimeout") {
                config.sd_connect_timeout_secs = parse_duration_secs(&v).unwrap_or(1800);
            }
            if let Some(v) = get("workingdirectory") { config.working_directory = v; }
            if let Some(v) = get("maximumnetworkbuffersize") {
                config.max_network_buffer_size = v.parse().unwrap_or(0);
            }
            if let Some(v) = get("maximumworkersperjob") {
                config.max_workers_per_job = v.parse().unwrap_or(2);
            }
            if let Some(v) = get("maximumbandwidthperjob") {
                config.max_bandwidth_per_job = parse_speed(&v).unwrap_or(0);
            }
            if let Some(v) = get("allowbandwidthbursting") {
                config.allow_bw_bursting = is_yes(&v);
            }
            config.allowed_script_dirs = all("allowedscriptdir");
            config.allowed_job_cmds = all("allowedjobcommand");
            if let Some(v) = get("messages") { config.messages_name = Some(v); }
        }
        "director" => {
            let name = get("name").unwrap_or_default();
            let password = get("password").unwrap_or_default();
            let monitor = get("monitor").map(|v| is_yes(&v)).unwrap_or(false);
            if !name.is_empty() {
                debug!("FD config: allowing director '{name}' (monitor={monitor})");
                config.directors.push(DirectorAuth {
                    name,
                    password,
                    monitor,
                    max_bandwidth_per_job: get("maximumbandwidthperjob").and_then(|v| parse_speed(&v)).unwrap_or(0),
                    allowed_script_dirs: all("allowedscriptdir"),
                    allowed_job_cmds: all("allowedjobcommand"),
                });
            }
        }
        "messages" => {
            let mut m = MessagesRes { name: get("name").unwrap_or_default(), ..Default::default() };
            for (k, v) in &list {
                match k.as_str() {
                    "director" => {
                        if let Some((_, types)) = v.split_once('=') {
                            m.director_types |= scan_types(types);
                        }
                    }
                    "append" => {
                        if let Some((file, types)) = v.split_once('=') {
                            m.append.push((file.trim().trim_matches('"').to_string(), scan_types(types)));
                        }
                    }
                    _ => {}
                }
            }
            config.all_messages.push(m);
        }
        other => {
            debug!("FD config: skipping unknown block type '{other}'");
        }
    }
}

fn is_yes(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "yes" | "true" | "1" | "on")
}

/// Directives in order: (key without blanks, lowercase; value).
fn parse_directive_list(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        for part in line.split(';') {
            let part = part.trim();
            if part.is_empty() || part.starts_with('#') {
                continue;
            }
            if let Some(pos) = part.find('=') {
                let key: String = part[..pos].chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
                let val = part[pos + 1..].trim();
                let val = if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 && !val[1..val.len() - 1].contains('"') {
                    val[1..val.len() - 1].to_string()
                } else {
                    val.to_string()
                };
                out.push((key, val));
            }
        }
    }
    out
}

fn strip_comments(input: &str) -> String {
    input.lines()
        .map(|line| {
            if let Some(pos) = line.find('#') {
                // Don't strip # inside quotes
                let before = &line[..pos];
                if before.matches('"').count() % 2 == 0 {
                    before
                } else {
                    line
                }
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn walk_conf_dir(dir: &Path, output: &mut String) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("readdir {}: {e}", dir.display()))?;
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            walk_conf_dir(&path, output)?;
        } else if path.extension().map(|e| e == "conf").unwrap_or(false) {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            output.push_str(&content);
            output.push('\n');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_config() {
        let input = r#"
FileDaemon {
  Name = test-fd
  Port = 9102
  Maximum Concurrent Jobs = 20
}
Director {
  Name = bareos-dir
  Password = "secret123"
}
"#;
        let config = FdConfig::parse(input).unwrap();
        assert_eq!(config.name, "test-fd");
        assert_eq!(config.port, 9102);
        assert_eq!(config.directors.len(), 1);
        assert_eq!(config.directors[0].name, "bareos-dir");
        assert_eq!(config.directors[0].password, "secret123");
    }

    #[test]
    fn parse_multiple_directors() {
        let input = r#"
FileDaemon { Name = fd1 }
Director { Name = dir1
  Password = "p1" }
Director { Name = bareos-mon
  Password = "p2"
  Monitor = yes }
"#;
        let config = FdConfig::parse(input).unwrap();
        assert_eq!(config.directors.len(), 2);
        assert!(!config.directors[0].monitor);
        assert!(config.directors[1].monitor);
    }

    #[test]
    fn parse_messages_and_limits() {
        let input = r#"
Client {
  Name = c-fd
  FDPort = 9
  Maximum Bandwidth Per Job = 100 kb/s
  Allowed Script Dir = /etc/bareos/scripts
  Allowed Script Dir = /usr/lib/bareos/scripts
  Messages = Standard
}
Director { Name = d; Password = "p"; Allowed Job Command = backup, restore }
Messages {
  Name = Standard
  director = d = all, !skipped, !restored
  append = "/tmp/x.log" = all
}
"#;
        let c = FdConfig::parse(input).unwrap();
        assert_eq!(c.max_bandwidth_per_job, 100_000);
        assert_eq!(c.allowed_script_dirs.len(), 2);
        assert_eq!(c.directors[0].allowed_job_cmds, vec!["backup", "restore"]);
        let m = c.messages.unwrap();
        assert!(m.director_types & (1 << 4) != 0);
        assert!(m.director_types & (1 << 9) == 0);
        assert!(m.director_types & (1 << 13) == 0);
        assert_eq!(m.append[0].0, "/tmp/x.log");
    }
}
