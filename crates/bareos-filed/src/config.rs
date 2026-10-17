//! FD configuration — parsed from bareos-fd.d/ or bareos-fd.conf.

use std::path::Path;
use tracing::debug;

/// A Director allowed to connect to this FD.
#[derive(Debug, Clone)]
pub struct DirectorAuth {
    pub name: String,
    pub password: String,
    pub monitor: bool,
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

    fn parse(input: &str) -> Result<Self, String> {
        let mut config = FdConfig {
            name: "bareos-fd".to_string(),
            port: 9102,
            max_concurrent_jobs: 20,
            directors: Vec::new(),
            heartbeat_interval_secs: 0,
            sd_connect_timeout_secs: 1800,
        };

        // Simple block parser (reuse logic from bareos-config)
        let cleaned = strip_comments(input);
        let mut chars = cleaned.chars().peekable();
        let mut current_block = String::new();
        let mut in_block = false;
        let mut brace_depth = 0;
        let mut block_type = String::new();

        let mut pos = 0;
        while pos < cleaned.len() {
            let rest = &cleaned[pos..];

            if !in_block {
                // Find next block: "Type {" or "Type\n{"
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

            let ch = cleaned.as_bytes()[pos] as char;
            if ch == '{' {
                brace_depth += 1;
                current_block.push(ch);
            } else if ch == '}' {
                brace_depth -= 1;
                if brace_depth == 0 {
                    // Block complete
                    parse_block(&block_type, &current_block, &mut config);
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

        Ok(config)
    }

    /// Find director auth by name.
    pub fn find_director(&self, name: &str) -> Option<&DirectorAuth> {
        self.directors.iter().find(|d| d.name == name)
    }
}

fn parse_block(block_type: &str, content: &str, config: &mut FdConfig) {
    let directives = parse_directives(content);

    match block_type.to_lowercase().as_str() {
        "client" | "filedaemon" => {
            if let Some(v) = directives.get("name") { config.name = v.clone(); }
            if let Some(v) = directives.get("port") {
                if let Some(v) = directives.get("fdport") {
                    config.port = v.parse().unwrap_or(9102);
                }
                config.port = v.parse().unwrap_or(9102);
            }
            if let Some(v) = directives.get("fdport") {
                config.port = v.parse().unwrap_or(9102);
            }
            if let Some(v) = directives.get("maximum concurrent jobs") {
                config.max_concurrent_jobs = v.parse().unwrap_or(20);
            }
            if let Some(v) = directives.get("heartbeat interval") {
                // Simple: parse "N min" or "N" (seconds)
                let secs = if v.contains("min") {
                    v.split_whitespace().next()
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap_or(0) * 60
                } else {
                    v.parse().unwrap_or(0)
                };
                config.heartbeat_interval_secs = secs;
            }
            if let Some(v) = directives.get("sd connect timeout") {
                config.sd_connect_timeout_secs = parse_duration_secs(v).unwrap_or(1800);
            }
        }
        "director" => {
            let name = directives.get("name").cloned().unwrap_or_default();
            let password = directives.get("password").cloned().unwrap_or_default();
            let monitor = directives.get("monitor")
                .map(|v| v.eq_ignore_ascii_case("yes") || v == "true")
                .unwrap_or(false);

            if !name.is_empty() {
                debug!("FD config: allowing director '{name}' (monitor={monitor})");
                config.directors.push(DirectorAuth { name, password, monitor });
            }
        }
        other => {
            debug!("FD config: skipping unknown block type '{other}'");
        }
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

fn parse_directives(content: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some(pos) = line.find('=') {
            let key = line[..pos].trim().to_lowercase();
            let val = line[pos+1..].trim().trim_matches('"').to_string();
            map.insert(key, val);
        }
    }
    map
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
}
