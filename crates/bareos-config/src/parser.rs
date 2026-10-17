//! Bareos configuration file parser.
//!
//! Parses resource blocks: `ResourceType { Key = Value ... }`

use std::collections::HashMap;
use std::time::Duration;

use crate::resources::*;
use crate::values::*;

/// A raw directive: key-value pair before type-specific parsing.
#[derive(Debug, Clone)]
pub struct RawDirective {
    pub key: String,
    pub value: String,
}

/// A raw resource block before type-specific parsing.
#[derive(Debug, Clone)]
pub struct RawResource {
    pub resource_type: String,
    pub directives: Vec<RawDirective>,
}

/// Parse a Bareos config directory tree (bareos-dir.d/**/*.conf).
pub fn parse_config_dir(dir: &std::path::Path) -> Result<Vec<Resource>, ConfigError> {
    let mut all = Vec::new();

    if dir.is_file() {
        // Single file mode
        return parse_config_file(dir);
    }

    if !dir.is_dir() {
        return Err(ConfigError::ParseError(format!("{} is not a file or directory", dir.display())));
    }

    // C++ Bareos convention: look for bareos-dir.d/ subdirectory first
    let dir_d = dir.join("bareos-dir.d");
    let config_dir = if dir_d.is_dir() { &dir_d } else { dir };

    // C++ globs <path>/*/*.conf — one level of subdirectories, not recursive —
    // and lex_add() pushes every matched file on a stack, so the last file in
    // glob order is read first. Resources are appended to their type's chain
    // in read order, which is the order every listing (foreach_res) shows.
    let entries = std::fs::read_dir(config_dir)
        .map_err(|e| ConfigError::ParseError(format!("readdir {}: {e}", config_dir.display())))?;
    let mut conf_files = Vec::new();
    for subdir in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
        if !subdir.is_dir() { continue; }
        let sub_entries = std::fs::read_dir(&subdir)
            .map_err(|e| ConfigError::ParseError(format!("readdir {}: {e}", subdir.display())))?;
        conf_files.extend(sub_entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "conf").unwrap_or(false) && p.is_file()));
    }
    crate::raw::sort_like_glob(&mut conf_files);
    // Each file on its own: resources never span files, and errors can
    // name the file (and line) like the C++ parser does.
    for conf in conf_files.iter().rev() {
        all.extend(parse_config_file(conf)?);
    }
    Ok(all)
}

/// Parse one config file; errors are prefixed with "<file>: ".
pub fn parse_config_file(path: &std::path::Path) -> Result<Vec<Resource>, ConfigError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::ParseError(format!("read {}: {e}", path.display())))?;
    parse_config(&content).map_err(|e| match e {
        ConfigError::ParseError(msg) => ConfigError::ParseError(format!("{}: {msg}", path.display())),
        ConfigError::MissingRequired { resource_type, directive } => ConfigError::ParseError(format!(
            "{}: missing required directive '{directive}' in {resource_type}", path.display())),
        ConfigError::InvalidValue { directive, message } => ConfigError::ParseError(format!(
            "{}: invalid value for '{directive}': {message}", path.display())),
        ConfigError::UnknownResourceType(t) => ConfigError::ParseError(format!(
            "{}: unknown resource type: {t}", path.display())),
    })
}

/// Parse a complete Bareos configuration into typed resources.
pub fn parse_config(input: &str) -> Result<Vec<Resource>, ConfigError> {
    let cleaned = strip_comments(input);
    let raw_resources = parse_raw_resources(&cleaned)
        .map_err(|(msg, offset)| {
            // strip_comments keeps line breaks, so offsets map to source lines
            let line = cleaned[..offset.min(cleaned.len())].matches('\n').count() + 1;
            ConfigError::ParseError(format!("line {line}: {msg}"))
        })?;

    let mut resources = Vec::new();
    for raw in raw_resources {
        let resource = convert_raw_resource(&raw)?;
        resources.push(resource);
    }
    Ok(resources)
}

/// Configuration parsing errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("unknown resource type: {0}")]
    UnknownResourceType(String),
    #[error("missing required directive '{directive}' in {resource_type}")]
    MissingRequired {
        resource_type: String,
        directive: String,
    },
    #[error("invalid value for '{directive}': {message}")]
    InvalidValue {
        directive: String,
        message: String,
    },
}

/// `StoreAuthprotocoltype` (dird_conf.cc:2613).
fn parse_auth_protocol(value: &str) -> Result<AuthProtocol, ConfigError> {
    AuthProtocol::from_keyword(value.trim()).ok_or_else(|| ConfigError::InvalidValue {
        directive: "Protocol".to_string(),
        message: format!("Expected a Auth Protocol Type keyword, got: {}", value.trim()),
    })
}

/// `StoreAuthtype` (dird_conf.cc:2640).
fn parse_auth_type(value: &str) -> Result<AuthType, ConfigError> {
    AuthType::from_keyword(value.trim()).ok_or_else(|| ConfigError::InvalidValue {
        directive: "AuthType".to_string(),
        message: format!("Expected a Authentication Type keyword, got: {}", value.trim()),
    })
}

/// Strip # comments from input.
fn strip_comments(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            // Don't strip # inside quoted strings
            let mut in_quotes = false;
            let mut comment_pos = None;
            for (i, c) in line.char_indices() {
                match c {
                    '"' => in_quotes = !in_quotes,
                    '#' if !in_quotes => {
                        comment_pos = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            match comment_pos {
                Some(pos) => &line[..pos],
                None => line,
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse all raw resource blocks from input.
/// Errors carry the byte offset into `input` for line reporting.
fn parse_raw_resources(input: &str) -> Result<Vec<RawResource>, (String, usize)> {
    parse_raw_resources_inner(input).map_err(|(msg, rest)| (msg, input.len() - rest))
}

fn parse_raw_resources_inner(input: &str) -> Result<Vec<RawResource>, (String, usize)> {
    let mut resources = Vec::new();
    let mut remaining = input;

    loop {
        // Skip whitespace
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            break;
        }

        // Parse resource type name
        let type_end = remaining
            .find(|c: char| c.is_whitespace() || c == '{')
            .ok_or_else(|| (format!("expected resource type, got: {}", &remaining[..20.min(remaining.len())]), remaining.len()))?;
        let resource_type = remaining[..type_end].to_string();
        // Like C++ "Expected a Resource name identifier": a block keyword or a
        // stray '}' at top level means a '}' too many above it. Other unknown
        // resource types are skipped later (lenient, as before).
        if resource_type.starts_with('}') || ["include", "exclude", "options", "runscript"]
            .contains(&resource_type.to_ascii_lowercase().as_str())
        {
            return Err((format!("expected a resource type, got: {resource_type} (is there one '}}' too many above?)"),
                        remaining.len()));
        }
        remaining = remaining[type_end..].trim_start();

        // Expect opening brace
        if !remaining.starts_with('{') {
            return Err((format!("expected '{{' after resource type '{resource_type}'"), remaining.len()));
        }
        remaining = &remaining[1..];

        // Parse directives until closing brace
        let mut directives = Vec::new();
        loop {
            remaining = remaining.trim_start();
            if remaining.starts_with('}') {
                remaining = &remaining[1..];
                break;
            }
            if remaining.is_empty() {
                return Err((format!("unexpected EOF in resource '{resource_type}'"), remaining.len()));
            }

            // Check for nested block (Include { ... }, Exclude { ... }, Options { ... }, etc.)
            let trimmed = remaining.trim_start();
            if let Some(brace_pos) = trimmed.find('{') {
                let before_brace = trimmed[..brace_pos].trim();
                if !before_brace.contains('=') && !before_brace.is_empty() {
                    let block_name = before_brace.split_whitespace().next().unwrap_or("").to_lowercase();
                    // Collect the block content (braces inside quotes, e.g. in a
                    // RunScript Command, do not count)
                    let block = &trimmed[brace_pos..];
                    let (block_content, rest) = parse_brace_value(block)
                        .map_err(|_| (format!("unbalanced '{{' in '{block_name}' block of resource '{resource_type}'"),
                                      block.len()))?;
                    remaining = rest;
                    // Store block content as a directive with "_block_" prefix
                    directives.push(RawDirective {
                        key: format!("_block_{block_name}"),
                        value: block_content,
                    });
                    continue;
                }
            }

            // Parse directive key (may contain spaces: "Maximum Concurrent Jobs")
            let (key, rest) = parse_directive_key(remaining).map_err(|e| (e, remaining.len()))?;
            remaining = rest;

            // Skip = sign
            remaining = remaining.trim_start();
            if remaining.starts_with('=') {
                remaining = remaining[1..].trim_start();
            }

            // Parse value (until newline, semicolon, or closing brace)
            let (val, rest) = parse_directive_value(remaining).map_err(|e| (e, remaining.len()))?;
            remaining = rest;

            if !key.is_empty() {
                directives.push(RawDirective {
                    key: key.to_lowercase(),
                    value: val.trim().to_string(),
                });
            }
        }

        resources.push(RawResource {
            resource_type,
            directives,
        });
    }

    Ok(resources)
}

/// Parse a directive key (everything before = sign).
/// Keys can contain spaces: "Maximum Concurrent Jobs"
fn parse_directive_key(input: &str) -> Result<(String, &str), String> {
    let eq_pos = input.find('=');
    let brace_pos = input.find('}');
    let nl_pos = input.find('\n');

    // If closing brace comes before = or newline, this isn't a directive
    if let Some(bp) = brace_pos {
        if eq_pos.is_none() || bp < eq_pos.unwrap() {
            return Ok((String::new(), input));
        }
    }

    let end = eq_pos
        .ok_or_else(|| format!("expected '=' in directive near: {}", &input[..40.min(input.len())]))?;

    let key = input[..end].trim().to_lowercase();
    Ok((key, &input[end..]))
}

/// Parse a directive value (until newline, semicolon, or end context).
fn parse_directive_value(input: &str) -> Result<(String, &str), String> {
    if input.starts_with('{') {
        return parse_brace_value(input);
    }
    let mut result = String::new();
    let mut remaining = input;
    let mut in_quotes = false;

    for (i, c) in input.char_indices() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                // Don't include outer quotes in value
            }
            '\n' | ';' if !in_quotes => {
                remaining = &input[i + 1..];
                return Ok((result.trim().to_string(), remaining));
            }
            '}' if !in_quotes => {
                remaining = &input[i..]; // Don't consume the brace
                return Ok((result.trim().to_string(), remaining));
            }
            '\\' if in_quotes => {
                // Skip next char
                if let Some(next) = input[i + 1..].chars().next() {
                    result.push(next);
                }
            }
            _ => {
                if !(c == '"') {
                    result.push(c);
                }
            }
        }
    }

    Ok((result.trim().to_string(), ""))
}

/// A value in braces (`Addresses = { ipv4 = { addr = 1.2.3.4; port = 9101 } }`):
/// returns the inner text (balanced, quotes respected) and the input after the
/// closing brace.
fn parse_brace_value(input: &str) -> Result<(String, &str), String> {
    let mut depth = 0usize;
    let mut in_quotes = false;
    let mut escaped = false;
    for (i, c) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            '{' if !in_quotes => depth += 1,
            '}' if !in_quotes => {
                depth -= 1;
                if depth == 0 {
                    return Ok((input[1..i].trim().to_string(), &input[i + 1..]));
                }
            }
            _ => {}
        }
    }
    Err("unbalanced '{' in directive value".to_string())
}

/// Extract File = <path> directives from a nested Include/Exclude block.
/// The block content may contain nested Options { } blocks which we skip.
fn extract_file_directives(block_content: &str) -> String {
    let mut files = Vec::new();
    for line in block_content.lines() {
        let trimmed = line.trim();
        // Skip comments, empty lines, and nested blocks
        if trimmed.is_empty() || trimmed.starts_with('#') { continue; }
        if trimmed.starts_with('{') || trimmed.starts_with('}') { continue; }
        // Look for "File = <path>" directives
        if let Some(pos) = trimmed.find('=') {
            let key = trimmed[..pos].trim().to_lowercase();
            let val = trimmed[pos+1..].trim().trim_matches('"');
            if key == "file" {
                files.push(val.to_string());
            }
        }
    }
    files.join("\n")
}

/// Parse Job/JobDefs directives into a JobResource.
fn parse_job_fields(directives: &HashMap<String, String>) -> Result<JobResource, ConfigError> {
    let get = |key: &str| directives.get(&crate::raw::normalize_key(key)).map(|s| s.as_str());
    let require = |key: &str| -> Result<&str, ConfigError> {
        get(key).ok_or_else(|| ConfigError::MissingRequired {
            resource_type: "Job".to_string(),
            directive: key.to_string(),
        })
    };

    let mut job = JobResource::default();
    job.name = require("name")?.to_string();
    job.present = directives.keys().map(|k| crate::jobdefs::item_key(k)).collect();
    if let Some(v) = get("type") {
        job.job_type = match v.to_lowercase().as_str() {
            "backup" => JobType::Backup,
            "restore" => JobType::Restore,
            "verify" => JobType::Verify,
            "admin" => JobType::Admin,
            "copy" => JobType::Copy,
            "migrate" => JobType::Migrate,
            "consolidate" => JobType::Consolidate,
            _ => JobType::Backup,
        };
    }
    if let Some(v) = get("level") {
        job.level = match v.to_lowercase().as_str() {
            "full" => JobLevel::Full,
            "incremental" => JobLevel::Incremental,
            "differential" => JobLevel::Differential,
            "virtualfull" => JobLevel::VirtualFull,
            _ => JobLevel::Full,
        };
    }
    if let Some(v) = get("client") { job.client = v.to_string(); }
    if let Some(v) = get("fileset") { job.fileset = v.to_string(); }
    if let Some(v) = get("file set") { job.fileset = v.to_string(); }
    if let Some(v) = get("schedule") { job.schedule = v.to_string(); }
    if let Some(v) = get("storage") { job.storage = v.to_string(); }
    if let Some(v) = get("pool") { job.pool = v.to_string(); }
    if let Some(v) = get("full backup pool") { job.full_backup_pool = v.to_string(); }
    if let Some(v) = get("differential backup pool") { job.differential_backup_pool = v.to_string(); }
    if let Some(v) = get("incremental backup pool") { job.incremental_backup_pool = v.to_string(); }
    if let Some(v) = get("messages") { job.messages = v.to_string(); }
    if let Some(v) = get("priority") { job.priority = v.parse().unwrap_or(10); }
    if let Some(v) = get("maximum concurrent jobs") {
        job.max_concurrent_jobs = v.parse().unwrap_or(1);
    }
    if let Some(v) = get("enabled") {
        job.enabled = parse_bool(v).map(|(_, b)| b).unwrap_or(true);
    }
    if let Some(v) = get("jobdefs") { job.jobdefs = v.to_string(); }
    if let Some(v) = get("write bootstrap") { job.write_bootstrap = v.to_string(); }
    if let Some(v) = get("run before job") { job.run_before_job = v.to_string(); }
    if let Some(v) = get("run after job") { job.run_after_job = v.to_string(); }
    if let Some(v) = get("runbeforejob") { job.run_before_job = v.to_string(); }
    if let Some(v) = get("runafterjob") { job.run_after_job = v.to_string(); }

    Ok(job)
}

/// Convert a raw resource to a typed resource.
fn convert_raw_resource(raw: &RawResource) -> Result<Resource, ConfigError> {
    let directives: HashMap<String, String> = raw
        .directives
        .iter()
        .map(|d| (crate::raw::normalize_key(&d.key), d.value.clone()))
        .collect();

    // C++ ignores case and spaces in directive names.
    let get = |key: &str| directives.get(&crate::raw::normalize_key(key)).map(|s| s.as_str());
    let require = |key: &str| -> Result<&str, ConfigError> {
        get(key).ok_or_else(|| ConfigError::MissingRequired {
            resource_type: raw.resource_type.clone(),
            directive: key.to_string(),
        })
    };

    match raw.resource_type.to_lowercase().as_str() {
        "director" => {
            let mut dir = DirectorResource::default();
            dir.name = require("name")?.to_string();
            if let Some(v) = get("password") { dir.password = v.to_string(); }
            if let Some(v) = get("port").or(get("dirport")) { dir.port = v.parse().unwrap_or(9101); }
            if let Some(v) = get("maximum concurrent jobs") {
                dir.max_concurrent_jobs = v.parse().unwrap_or(1);
            }
            if let Some(v) = get("enablegrpc") {
                dir.enable_grpc = parse_bool(v).map(|(_, b)| b).unwrap_or(false);
            }
            if let Some(v) = get("enable grpc") {
                dir.enable_grpc = parse_bool(v).map(|(_, b)| b).unwrap_or(false);
            }
            let get_any = |keys: &[&str]| keys.iter().find_map(|k| get(k));
            if let Some(v) = get_any(&["grpc port", "grpcport"]) {
                dir.grpc_port = v.parse().ok();
            }
            if let Some(v) = get_any(&["grpc tls certificate", "grpctlscertificate"]) {
                dir.grpc_tls_certificate = Some(v.to_string());
            }
            if let Some(v) = get_any(&["grpc tls key", "grpctlskey"]) {
                dir.grpc_tls_key = Some(v.to_string());
            }
            if let Some(v) = get_any(&["mdns advertise", "mdnsadvertise"]) {
                dir.mdns_advertise = parse_bool(v).map(|(_, b)| b).unwrap_or(true);
            }
            if let Some(v) = get_any(&["working directory", "workingdirectory"]) {
                dir.working_directory = v.to_string();
            }
            if let Some(v) = get("key encryption key") {
                dir.key_encryption_key = Some(v.to_string());
            }
            if let Some(v) = get("heartbeat interval") {
                dir.heartbeat_interval = parse_time(v).map(|(_, d)| d).unwrap_or_default();
            }
            Ok(Resource::Director(dir))
        }

        "client" => {
            let mut client = ClientResource::default();
            client.name = require("name")?.to_string();
            if let Some(v) = get("address") { client.address = v.to_string(); }
            if let Some(v) = get("port").or(get("fdport")) { client.port = v.parse().unwrap_or(9102); }
            if let Some(v) = get("password") { client.password = v.to_string(); }
            if let Some(v) = get("catalog") { client.catalog = v.to_string(); }
            if let Some(v) = get("maximum concurrent jobs") {
                client.max_concurrent_jobs = v.parse().unwrap_or(1);
            }
            if let Some(v) = get("enabled") {
                client.enabled = parse_bool(v).map(|(_, b)| b).unwrap_or(true);
            }
            if let Some(v) = get("protocol") { client.protocol = parse_auth_protocol(v)?; }
            if let Some(v) = get("authtype") { client.auth_type = parse_auth_type(v)?; }
            if let Some(v) = get("username") { client.username = Some(v.to_string()); }
            if let Some(v) = get("ndmploglevel") { client.ndmp_log_level = v.parse().unwrap_or(4); }
            if let Some(v) = get("ndmpblocksize") {
                client.ndmp_block_size = parse_size(v).map(|(_, n)| n as u32).unwrap_or(64512);
            }
            if let Some(v) = get("ndmpuselmdb") {
                client.ndmp_use_lmdb = parse_bool(v).map(|(_, b)| b).unwrap_or(true);
            }
            Ok(Resource::Client(client))
        }

        "job" => {
            let job = parse_job_fields(&directives)?;
            Ok(Resource::Job(job))
        }

        "jobdefs" => {
            let job = parse_job_fields(&directives)?;
            Ok(Resource::JobDefs(JobDefsResource { name: job.name.clone(), job }))
        }

        "storage" => {
            let mut stor = StorageResource::default();
            stor.name = require("name")?.to_string();
            if let Some(v) = get("address") { stor.address = v.to_string(); }
            if let Some(v) = get("port").or(get("sdport")) { stor.port = v.parse().unwrap_or(9103); }
            if let Some(v) = get("password") { stor.password = v.to_string(); }
            if let Some(v) = get("device") { stor.device = v.to_string(); }
            if let Some(v) = get("media type") { stor.media_type = v.to_string(); }
            if let Some(v) = get("maximum concurrent jobs") {
                stor.max_concurrent_jobs = v.parse().unwrap_or(1);
            }
            if let Some(v) = get("protocol") { stor.protocol = parse_auth_protocol(v)?; }
            if let Some(v) = get("authtype") { stor.auth_type = parse_auth_type(v)?; }
            if let Some(v) = get("username") { stor.username = Some(v.to_string()); }
            if let Some(v) = get("pairedstorage") { stor.paired_storage = Some(v.to_string()); }
            if let Some(v) = get("ndmpchangerdevice") { stor.ndmp_changer_device = Some(v.to_string()); }
            Ok(Resource::Storage(stor))
        }

        "catalog" => {
            let mut cat = CatalogResource::default();
            cat.name = require("name")?.to_string();
            if let Some(v) = get("dbdriver") { cat.db_driver = v.to_string(); }
            if let Some(v) = get("db driver") { cat.db_driver = v.to_string(); }
            if let Some(v) = get("dbname") { cat.db_name = v.to_string(); }
            if let Some(v) = get("db name") { cat.db_name = v.to_string(); }
            if let Some(v) = get("dbaddress") { cat.db_address = v.to_string(); }
            if let Some(v) = get("db address") { cat.db_address = v.to_string(); }
            if let Some(v) = get("dbport") { cat.db_port = v.parse().unwrap_or(5432); }
            if let Some(v) = get("db port") { cat.db_port = v.parse().unwrap_or(5432); }
            if let Some(v) = get("dbuser") { cat.db_user = v.to_string(); }
            if let Some(v) = get("db user") { cat.db_user = v.to_string(); }
            if let Some(v) = get("dbpassword") { cat.db_password = v.to_string(); }
            if let Some(v) = get("db password") { cat.db_password = v.to_string(); }
            Ok(Resource::Catalog(cat))
        }

        "schedule" => {
            let mut sched = ScheduleResource {
                name: require("name")?.to_string(),
                enabled: true,
                runs: Vec::new(),
            };
            if let Some(v) = get("enabled") {
                sched.enabled = parse_bool(v).map(|(_, b)| b).unwrap_or(true);
            }
            // Collect all "run" directives
            for d in &raw.directives {
                if d.key == "run" {
                    if let Some(run) = parse_run_directive(&d.value) {
                        sched.runs.push(run);
                    }
                }
            }
            Ok(Resource::Schedule(sched))
        }

        "pool" => {
            let mut pool = PoolResource {
                name: require("name")?.to_string(),
                pool_type: get("pool type").unwrap_or("Backup").to_string(),
                recycle: get("recycle")
                    .and_then(|v| parse_bool(v).ok())
                    .map(|(_, b)| b)
                    .unwrap_or(true),
                auto_prune: get("autoprune")
                    .or(get("auto prune"))
                    .and_then(|v| parse_bool(v).ok())
                    .map(|(_, b)| b)
                    .unwrap_or(true),
                volume_retention: get("volume retention")
                    .and_then(|v| parse_time(v).ok())
                    .map(|(_, d)| d)
                    .unwrap_or(Duration::from_secs(365 * 86400)),
                maximum_volume_bytes: get("maximum volume bytes")
                    .and_then(|v| parse_size(v).ok())
                    .map(|(_, s)| s)
                    .unwrap_or(0),
                maximum_volumes: get("maximum volumes")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0),
                label_format: get("label format").unwrap_or("").to_string(),
            };
            Ok(Resource::Pool(pool))
        }

        "fileset" => {
            let name = require("name")?.to_string();
            // Collect File= directives from Include/Exclude blocks
            let mut include_blocks = Vec::new();
            let mut exclude_blocks = Vec::new();
            for d in raw.directives.iter() {
                if d.key.starts_with("_block_include") {
                    let files = extract_file_directives(&d.value);
                    if !files.is_empty() {
                        include_blocks.push(files);
                    }
                } else if d.key.starts_with("_block_exclude") {
                    let files = extract_file_directives(&d.value);
                    if !files.is_empty() {
                        exclude_blocks.push(files);
                    }
                }
            }
            Ok(Resource::FileSet(FileSetResource { name, include_blocks, exclude_blocks }))
        }

        "console" => {
            let mut con = ConsoleResource {
                name: require("name")?.to_string(),
                password: String::new(),
            };
            if let Some(v) = get("password") { con.password = v.to_string(); }
            Ok(Resource::Console(con))
        }

        // "jobdefs" is handled above together with "job"

        "messages" => {
            let name = require("name")?.to_string();
            Ok(Resource::Messages(MessagesResource { name }))
        }

        "profile" => {
            let name = require("name")?.to_string();
            Ok(Resource::Profile(ProfileResource { name }))
        }

        // Unbekannte Resource-Typen tolerieren
        _other => {
            tracing::debug!("skipping unknown resource type: {}", raw.resource_type);
            Ok(Resource::Console(ConsoleResource {
                name: format!("_skip_{}", raw.resource_type),
                password: String::new(),
            }))
        }
    }
}

/// Parse a Schedule Run directive value — port of C++ dird/run_conf.cc
/// StoreRun(). Examples: "Full 1st sun at 23:45", "Incremental mon-sat at
/// 23:45", "Level=Full Pool=Monthly 1 jan at 0:05". C++ scan errors are
/// logged and the Run directive is skipped.
fn parse_run_directive(input: &str) -> Option<RunDirective> {
    match store_run(input) {
        Ok(run) => Some(run),
        Err(e) => {
            tracing::warn!("Run = {input}: {e}");
            None
        }
    }
}

/// Lexer tokens of a Run value (C++ lex.cc with NoIdent).
#[derive(Debug, Clone, PartialEq)]
enum RunToken {
    Number(String),
    Str(String),
    Equals,
    Comma,
}

fn run_tokens(input: &str) -> Vec<RunToken> {
    fn flush(cur: &mut String, out: &mut Vec<RunToken>) {
        if !cur.is_empty() {
            let word = std::mem::take(cur);
            if word.bytes().all(|b| b.is_ascii_digit()) {
                out.push(RunToken::Number(word));
            } else {
                out.push(RunToken::Str(word));
            }
        }
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in input.chars() {
        match c {
            c if c.is_whitespace() || c == '"' => flush(&mut cur, &mut out),
            '=' => { flush(&mut cur, &mut out); out.push(RunToken::Equals); }
            ',' => { flush(&mut cur, &mut out); out.push(RunToken::Comma); }
            '#' | ';' => break,
            c => cur.push(c),
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum RunState {
    None, Range, Mday, Month, Time, At, Wday, Daily, Weekly, Monthly, Hourly, Wom, Woy, Last, Modulo,
}

/// run_conf.cc keyw[]
const RUN_KEYWORDS: &[(&str, RunState, i32)] = &[
    ("on", RunState::None, 0), ("at", RunState::At, 0), ("last", RunState::Last, 0),
    ("sun", RunState::Wday, 0), ("mon", RunState::Wday, 1), ("tue", RunState::Wday, 2),
    ("wed", RunState::Wday, 3), ("thu", RunState::Wday, 4), ("fri", RunState::Wday, 5),
    ("sat", RunState::Wday, 6),
    ("jan", RunState::Month, 0), ("feb", RunState::Month, 1), ("mar", RunState::Month, 2),
    ("apr", RunState::Month, 3), ("may", RunState::Month, 4), ("jun", RunState::Month, 5),
    ("jul", RunState::Month, 6), ("aug", RunState::Month, 7), ("sep", RunState::Month, 8),
    ("oct", RunState::Month, 9), ("nov", RunState::Month, 10), ("dec", RunState::Month, 11),
    ("sunday", RunState::Wday, 0), ("monday", RunState::Wday, 1), ("tuesday", RunState::Wday, 2),
    ("wednesday", RunState::Wday, 3), ("thursday", RunState::Wday, 4), ("friday", RunState::Wday, 5),
    ("saturday", RunState::Wday, 6),
    ("january", RunState::Month, 0), ("february", RunState::Month, 1), ("march", RunState::Month, 2),
    ("april", RunState::Month, 3), ("june", RunState::Month, 5), ("july", RunState::Month, 6),
    ("august", RunState::Month, 7), ("september", RunState::Month, 8), ("october", RunState::Month, 9),
    ("november", RunState::Month, 10), ("december", RunState::Month, 11),
    ("daily", RunState::Daily, 0), ("weekly", RunState::Weekly, 0), ("monthly", RunState::Monthly, 0),
    ("hourly", RunState::Hourly, 0),
    ("1st", RunState::Wom, 0), ("2nd", RunState::Wom, 1), ("3rd", RunState::Wom, 2),
    ("4th", RunState::Wom, 3), ("5th", RunState::Wom, 4),
    ("first", RunState::Wom, 0), ("second", RunState::Wom, 1), ("third", RunState::Wom, 2),
    ("fourth", RunState::Wom, 3), ("fifth", RunState::Wom, 4),
];

fn run_keyword(word: &str) -> Option<(RunState, i32)> {
    RUN_KEYWORDS.iter().find(|k| k.0.eq_ignore_ascii_case(word)).map(|k| (k.1, k.2))
}

/// dird_conf.cc joblevels[]; Base and Since have no Rust variant and keep
/// the job's level.
fn run_level(word: &str) -> Option<Option<JobLevel>> {
    let level = match word.to_ascii_lowercase().as_str() {
        "full" => Some(JobLevel::Full),
        "incremental" => Some(JobLevel::Incremental),
        "differential" => Some(JobLevel::Differential),
        "virtualfull" => Some(JobLevel::VirtualFull),
        "catalog" => Some(JobLevel::Catalog),
        "initcatalog" => Some(JobLevel::InitCatalog),
        "volumetocatalog" => Some(JobLevel::VolumeToCatalog),
        "disktocatalog" => Some(JobLevel::DiskToCatalog),
        "data" => Some(JobLevel::Data),
        "base" | "since" => None,
        _ => return None,
    };
    Some(level)
}

/// C atoi(): leading digits, 0 if none.
fn atoi(s: &str) -> i32 {
    s.bytes().take_while(u8::is_ascii_digit)
        .fold(0i64, |acc, b| (acc * 10 + (b - b'0') as i64).min(i32::MAX as i64)) as i32
}

fn is_integer(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn is_week(s: &str) -> bool {
    s.len() == 3 && s.starts_with(['w', 'W']) && is_integer(&s[1..])
}

fn set_range(mask: &mut u64, from: i32, to: i32) {
    for i in from..=to {
        *mask |= 1 << i;
    }
}

fn run_yes_no(word: &str) -> Result<bool, String> {
    match word.to_ascii_lowercase().as_str() {
        "yes" | "true" => Ok(true),
        "no" | "false" => Ok(false),
        _ => Err(format!("Expect a YES or NO, got: {word}")),
    }
}

/// Bit sets of a Run directive while parsing (run_conf.cc have_* flags).
#[derive(Default)]
struct RunMasks {
    hour: u64, mday: u64, month: u64, wday: u64, wom: u64, woy: u64,
    have_hour: bool, have_mday: bool, have_wday: bool, have_month: bool,
    have_wom: bool, have_at: bool, have_woy: bool,
}

fn store_run(input: &str) -> Result<RunDirective, String> {
    let tokens = run_tokens(input);
    let mut run = RunDirective {
        level: None,
        pool: None,
        full_pool: None,
        incremental_pool: None,
        differential_pool: None,
        next_pool: None,
        storage: None,
        messages: None,
        priority: None,
        spool_data: None,
        max_run_sched_time: None,
        accurate: None,
        hour: 0,
        minute: 0,
        mask: DateTimeMask::all(),
    };
    let word = |t: Option<&RunToken>| match t {
        Some(RunToken::Str(s)) | Some(RunToken::Number(s)) => s.clone(),
        _ => String::new(),
    };

    // Scan for Job level "full", "incremental", ... and keyword=value pairs
    const FIELDS: &[&str] = &["pool", "fullpool", "incrementalpool", "differentialpool", "nextpool",
        "level", "storage", "messages", "priority", "spooldata", "maxrunschedtime", "accurate"];
    let mut pos = 0;
    while pos < tokens.len() {
        let key = word(tokens.get(pos));
        if let Some(field) = FIELDS.iter().find(|f| f.eq_ignore_ascii_case(&key)) {
            if tokens.get(pos + 1) != Some(&RunToken::Equals) {
                return Err(format!("Expected an equals, got: {}", word(tokens.get(pos + 1))));
            }
            let value = word(tokens.get(pos + 2));
            pos += 3;
            match *field {
                "spooldata" => run.spool_data = Some(run_yes_no(&value)?),
                "accurate" => run.accurate = Some(run_yes_no(&value)?),
                "level" => match run_level(&value) {
                    Some(level) => run.level = level,
                    None => return Err(format!("Job level field: {value} not found in run record")),
                },
                "priority" => run.priority = Some(value.parse()
                    .map_err(|_| format!("expected a positive integer, got: {value}"))?),
                "pool" => run.pool = Some(value),
                "storage" => run.storage = Some(value),
                "messages" => run.messages = Some(value),
                "fullpool" => run.full_pool = Some(value),
                "incrementalpool" => run.incremental_pool = Some(value),
                "differentialpool" => run.differential_pool = Some(value),
                "nextpool" => run.next_pool = Some(value),
                "maxrunschedtime" => run.max_run_sched_time = Some(crate::values::parse_time(&value)
                    .map_err(|_| format!("expected a time period, got: {value}"))?.1.as_secs()),
                _ => {}
            }
            continue;
        }
        // Old style Job levels without keyword
        if let Some(level) = run_level(&key) {
            run.level = level;
            pos += 1;
            continue;
        }
        break;
    }

    // Scan schedule times; defaults: every hour of every day, minute 0
    let all = DateTimeMask::all();
    let mut m = RunMasks {
        hour: all.hour as u64, mday: all.mday as u64, month: all.month as u64,
        wday: all.wday as u64, wom: all.wom as u64, woy: all.woy,
        ..Default::default()
    };
    let mut first_hour = None;
    for token in &tokens[pos..] {
        let (state, code, text) = match token {
            RunToken::Comma => continue,
            RunToken::Equals => return Err("Unexpected token: =".into()),
            RunToken::Number(n) => {
                let code = atoi(n) - 1;
                if !(0..=30).contains(&code) {
                    return Err("Day number out of range (1-31)".into());
                }
                (RunState::Mday, code, n.as_str())
            }
            RunToken::Str(s) if s.contains('-') => (RunState::Range, 0, s.as_str()),
            RunToken::Str(s) if s.contains(':') => (RunState::Time, 0, s.as_str()),
            RunToken::Str(s) if s.contains('/') => (RunState::Modulo, 0, s.as_str()),
            RunToken::Str(s) if is_week(s) => {
                let code = atoi(&s[1..]);
                if !(0..=53).contains(&code) {
                    return Err("Week number out of range (0-53)".into());
                }
                (RunState::Woy, code, s.as_str())
            }
            RunToken::Str(s) => match run_keyword(s) {
                Some((state, code)) => (state, code, s.as_str()),
                None => return Err(format!("Job type field: {s} in run record not found")),
            },
        };
        match state {
            RunState::None => continue,
            RunState::Mday => {
                if !m.have_mday { m.mday = 0; m.have_mday = true; }
                m.mday |= 1 << code;
            }
            RunState::Month => {
                if !m.have_month { m.month = 0; m.have_month = true; }
                m.month |= 1 << code;
            }
            RunState::Wday => {
                if !m.have_wday { m.wday = 0; m.have_wday = true; }
                m.wday |= 1 << code;
            }
            RunState::Wom => {
                if !m.have_wom { m.wom = 0; m.have_wom = true; }
                m.wom |= 1 << code;
            }
            RunState::Woy => {
                if !m.have_woy { m.woy = 0; m.have_woy = true; }
                m.woy |= 1 << code;
            }
            RunState::Time => {
                if !m.have_at {
                    return Err("Time must be preceded by keyword AT.".into());
                }
                if !m.have_hour { m.hour = 0; }
                let (h, rest) = text.split_once(':').unwrap_or((text, ""));
                let mut h = atoi(h);
                let minute = atoi(rest);
                let suffix = rest.get(2..).unwrap_or(rest);
                let pm = suffix.eq_ignore_ascii_case("pm");
                let am = suffix.eq_ignore_ascii_case("am");
                if !pm && !am && rest.len() != 2 {
                    return Err("Bad time specification.".into());
                }
                // 12am is 00:00 and 12pm is 12:00
                if pm {
                    if h != 12 { h += 12; }
                } else if am && h == 12 {
                    h -= 12;
                }
                if !(0..=23).contains(&h) || !(0..=59).contains(&minute) {
                    return Err("Bad time specification.".into());
                }
                m.hour |= 1 << h;
                first_hour.get_or_insert(h as u32);
                run.minute = minute as u32;
                m.have_hour = true;
            }
            RunState::At => m.have_at = true,
            RunState::Last => {
                run.mask.last_7days_of_month = true;
                if !m.have_wom { m.wom = 0; m.have_wom = true; }
            }
            RunState::Modulo => {
                let (l, r) = text.split_once('/').unwrap_or((text, ""));
                if is_integer(l) && is_integer(r) {
                    let code = atoi(l) - 1;
                    let code2 = atoi(r);
                    if !(0..=30).contains(&code) || !(0..=30).contains(&code2) {
                        return Err("Bad day specification in modulo.".into());
                    }
                    if code > code2 {
                        return Err("Bad day specification, offset must always be <= than modulo.".into());
                    }
                    if !m.have_mday { m.mday = 0; m.have_mday = true; }
                    // C++ divides by zero for "1/0"; no bit is set here
                    for i in 0..31 {
                        if code2 != 0 && i % code2 == 0 && i + code < 64 {
                            m.mday |= 1 << (i + code);
                        }
                    }
                } else if is_week(l) && is_week(r) {
                    let code = atoi(&l[1..]);
                    let code2 = atoi(&r[1..]);
                    if !(0..=53).contains(&code) {
                        return Err("Week number out of range (0-53) in modulo".into());
                    }
                    if code2 <= 0 || code2 > 53 {
                        return Err("Week interval out of range (1-53) in modulo".into());
                    }
                    if code > code2 {
                        return Err("Bad week number specification in modulo, offset must always be <= than modulo.".into());
                    }
                    if !m.have_woy { m.woy = 0; m.have_woy = true; }
                    let mut week = code;
                    while week <= 53 {
                        m.woy |= 1 << week;
                        week += code2;
                    }
                } else {
                    return Err("Bad modulo time specification. Format for weekdays is '01/02', for yearweeks is 'w01/w02'.".into());
                }
            }
            RunState::Range => {
                let (l, r) = text.split_once('-').unwrap_or((text, ""));
                if is_integer(l) && is_integer(r) {
                    let code = atoi(l) - 1;
                    let code2 = atoi(r) - 1;
                    if !(0..=30).contains(&code) || !(0..=30).contains(&code2) {
                        return Err("Bad day range specification.".into());
                    }
                    if !m.have_mday { m.mday = 0; m.have_mday = true; }
                    if code < code2 {
                        set_range(&mut m.mday, code, code2);
                    } else {
                        set_range(&mut m.mday, code, 30);
                        set_range(&mut m.mday, 0, code2);
                    }
                } else if is_week(l) && is_week(r) {
                    let code = atoi(&l[1..]);
                    let code2 = atoi(&r[1..]);
                    if !(0..=53).contains(&code) || !(0..=53).contains(&code2) {
                        return Err("Week number out of range (0-53)".into());
                    }
                    if !m.have_woy { m.woy = 0; m.have_woy = true; }
                    if code < code2 {
                        set_range(&mut m.woy, code, code2);
                    } else {
                        set_range(&mut m.woy, code, 53);
                        set_range(&mut m.woy, 0, code2);
                    }
                } else {
                    // Week days, months or positions
                    let Some((state, code)) = run_keyword(l)
                        .filter(|k| matches!(k.0, RunState::Month | RunState::Wday | RunState::Wom))
                    else {
                        return Err("Invalid month, week or position day range".into());
                    };
                    let Some((_, code2)) = run_keyword(r).filter(|k| k.0 == state && k.1 != code) else {
                        return Err("Invalid month, weekday or position range".into());
                    };
                    let (mask, have, max) = match state {
                        RunState::Wday => (&mut m.wday, &mut m.have_wday, 6),
                        // This is a bit odd for months, but C++ accepts it
                        RunState::Month => (&mut m.month, &mut m.have_month, 11),
                        _ => (&mut m.wom, &mut m.have_wom, 4),
                    };
                    if !*have { *mask = 0; *have = true; }
                    if code < code2 {
                        set_range(mask, code, code2);
                    } else {
                        set_range(mask, code, max);
                        set_range(mask, 0, code2);
                    }
                }
            }
            RunState::Hourly => {
                m.have_hour = true;
                set_range(&mut m.hour, 0, 23);
            }
            RunState::Weekly => {
                m.have_mday = true;
                m.have_wom = true;
                m.have_woy = true;
                set_range(&mut m.mday, 0, 30);
                set_range(&mut m.wom, 0, 4);
                set_range(&mut m.woy, 0, 53);
            }
            RunState::Daily => {
                m.have_mday = true;
                set_range(&mut m.wday, 0, 6);
            }
            RunState::Monthly => {
                m.have_month = true;
                set_range(&mut m.month, 0, 11);
            }
        }
    }
    run.mask.hour = m.hour as u32;
    run.mask.mday = m.mday as u32;
    run.mask.month = m.month as u16;
    run.mask.wday = m.wday as u8;
    run.mask.wom = m.wom as u8;
    run.mask.woy = m.woy;
    run.hour = first_hour.unwrap_or(0);
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_comments_basic() {
        assert_eq!(strip_comments("key = value # comment"), "key = value ");
        assert_eq!(strip_comments("# full line comment"), "");
        assert_eq!(strip_comments(r#"key = "value # not comment""#), r#"key = "value # not comment""#);
    }

    #[test]
    fn parse_director() {
        let input = r#"
Director {
  Name = bareos-dir
  Port = 9101
  Password = "secret"
  Maximum Concurrent Jobs = 10
  Enable Grpc = yes
}
"#;
        let resources = parse_config(input).unwrap();
        assert_eq!(resources.len(), 1);
        match &resources[0] {
            Resource::Director(d) => {
                assert_eq!(d.name, "bareos-dir");
                assert_eq!(d.port, 9101);
                assert_eq!(d.password, "secret");
                assert_eq!(d.max_concurrent_jobs, 10);
                assert!(d.enable_grpc);
                assert_eq!(d.grpc_port, None);
                assert!(d.mdns_advertise);
                assert_eq!(d.working_directory, "/var/lib/bareos");
            }
            _ => panic!("expected Director"),
        }
    }

    /// C-001: DirPort/FdPort/SdPort are aliases of Port; case and spaces in
    /// directive names are ignored (dird_conf.cc Alias{"DirPort"}).
    #[test]
    fn port_aliases_and_spaced_keywords() {
        let input = "Director {\n Name = d\n Dir Port = 39101\n MaximumConcurrentJobs = 7\n}\n\
            Client {\n Name = c\n Address = h\n Password = p\n FDPort = 19102\n}\n\
            Storage {\n Name = s\n Address = h\n Password = p\n SdPort = 19103\n Device = x\n Media Type = File\n}\n";
        let mut seen = 0;
        for resource in &parse_config(input).unwrap() {
            match resource {
                Resource::Director(d) => { assert_eq!((d.port, d.max_concurrent_jobs), (39101, 7)); seen += 1; }
                Resource::Client(c) => { assert_eq!(c.port, 19102); seen += 1; }
                Resource::Storage(s) => { assert_eq!(s.port, 19103); seen += 1; }
                _ => {}
            }
        }
        assert_eq!(seen, 3);
    }

    /// C-050: Protocol/AuthType keywords (case-insensitive), NDMP items.
    #[test]
    fn storage_and_client_protocol() {
        let input = "Client {\n Name = c\n Protocol = ndmpv3\n AuthType = md5\n Username = u\n Address = h\n \
            Password = p\n Ndmp Block Size = 1024\n NdmpUseLmdb = no\n}\n\
            Storage {\n Name = s\n Protocol = NDMPv4\n Address = h\n Password = p\n Device = x\n \
            Media Type = File\n PairedStorage = File\n NdmpChangerDevice = robot\n}\n\
            Storage {\n Name = n\n Address = h\n Password = p\n Device = x\n Media Type = File\n}\n";
        let resources = parse_config(input).unwrap();
        let Resource::Client(c) = &resources[0] else { panic!() };
        assert_eq!((c.protocol, c.auth_type, c.username.as_deref()), (AuthProtocol::NdmpV3, AuthType::Md5, Some("u")));
        assert_eq!((c.ndmp_log_level, c.ndmp_block_size, c.ndmp_use_lmdb), (4, 1024, false));
        let Resource::Storage(s) = &resources[1] else { panic!() };
        assert!(s.protocol.is_ndmp());
        assert_eq!((s.paired_storage.as_deref(), s.ndmp_changer_device.as_deref()), (Some("File"), Some("robot")));
        let Resource::Storage(n) = &resources[2] else { panic!() };
        assert_eq!((n.protocol, n.auth_type), (AuthProtocol::Native, AuthType::None));
        assert!(parse_config("Storage {\n Name = s\n Protocol = NDMP\n Address = h\n Password = p\n}\n").is_err());
    }

    #[test]
    fn brace_values_and_line_numbers() {
        let input = r#"
Director {
  Name = bareos-dir
  Password = "secret"
  DirAddresses = {
    ipv4 = { addr = 0.0.0.0; port = 9101 }
    ipv6 = {
      addr = ::
      port = 9101
    }
  }
  Maximum Concurrent Jobs = 7
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Director(d) => {
                assert_eq!(d.name, "bareos-dir");
                assert_eq!(d.max_concurrent_jobs, 7);
            }
            _ => panic!("expected Director"),
        }

        let err = parse_config("Director {\n  Name = x\n}\nBroken\n").unwrap_err().to_string();
        assert!(err.contains("line 4"), "{err}");
    }

    #[test]
    fn braces_in_quoted_block_values() {
        let input = "Job {\n  Name = a\n  Type = Backup\n  RunScript {\n    RunsWhen = Before\n    Command = \"sh -c 'test -f /x && { echo ok; }' # not a comment\"\n  }\n  Priority = 5\n}\n";
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Job(j) => assert_eq!(j.name, "a"),
            _ => panic!("expected Job"),
        }
    }

    #[test]
    fn extra_closing_brace_is_reported_at_the_block() {
        let input = "FileSet {\n  Name = fs\n  Include {\n    File = /a\n  }\n  }\n\n  Exclude {\n    File = /tmp\n  }\n}\n";
        let err = parse_config(input).unwrap_err().to_string();
        assert!(err.contains("line 8"), "{err}");
        assert!(err.contains("Exclude"), "{err}");
        assert!(err.contains("too many"), "{err}");
    }

    #[test]
    fn errors_name_the_file() {
        let dir = std::env::temp_dir().join(format!("bareos-config-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("director")).unwrap();
        std::fs::write(dir.join("director/bad.conf"), "Director {\n  Name = x\n").unwrap();
        let err = parse_config_dir(&dir).unwrap_err().to_string();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(err.contains("director/bad.conf"), "{err}");
    }

    #[test]
    fn config_dir_is_read_in_reverse_glob_order() {
        // lex_open_file(): glob() sorts, lex_add() stacks — last file first,
        // resources inside one file keep their order.
        let dir = std::env::temp_dir().join(format!("bareos-config-order-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("pool")).unwrap();
        std::fs::create_dir_all(dir.join("profile")).unwrap();
        for name in ["Differential", "Full", "Incremental", "Scratch"] {
            std::fs::write(dir.join(format!("pool/{name}.conf")), format!("Pool {{\n  Name = {name}\n}}\n")).unwrap();
        }
        std::fs::write(dir.join("profile/Both.conf"), "Profile {\n  Name = first\n}\nProfile {\n  Name = second\n}\n").unwrap();
        std::fs::write(dir.join("pool/notes.txt"), "not a config file").unwrap();
        let resources = parse_config_dir(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let names: Vec<String> = resources.iter().map(|r| match r {
            Resource::Pool(p) => p.name.clone(),
            Resource::Profile(p) => p.name.clone(),
            other => panic!("unexpected {other:?}"),
        }).collect();
        assert_eq!(names, ["first", "second", "Scratch", "Incremental", "Full", "Differential"]);
    }

    #[test]
    fn parse_director_grpc_and_mdns() {
        let input = r#"
Director {
  Name = bareos-dir
  Password = "secret"
  Grpc Port = 9111
  Grpc Tls Certificate = /etc/bareos/tls/dir.crt
  GrpcTlsKey = /etc/bareos/tls/dir.key
  Mdns Advertise = no
  Working Directory = /tmp/dir-work
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Director(d) => {
                assert_eq!(d.grpc_port, Some(9111));
                assert_eq!(d.grpc_tls_certificate.as_deref(), Some("/etc/bareos/tls/dir.crt"));
                assert_eq!(d.grpc_tls_key.as_deref(), Some("/etc/bareos/tls/dir.key"));
                assert!(!d.mdns_advertise);
                assert_eq!(d.working_directory, "/tmp/dir-work");
            }
            _ => panic!("expected Director"),
        }
    }

    #[test]
    fn parse_client() {
        let input = r#"
Client {
  Name = bareos-fd
  Address = localhost
  Port = 9102
  Password = "client-pass"
  Catalog = MyCatalog
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Client(c) => {
                assert_eq!(c.name, "bareos-fd");
                assert_eq!(c.address, "localhost");
                assert_eq!(c.port, 9102);
                assert_eq!(c.password, "client-pass");
            }
            _ => panic!("expected Client"),
        }
    }

    #[test]
    fn parse_job() {
        let input = r#"
Job {
  Name = "BackupClient1"
  Type = Backup
  Level = Full
  Client = bareos-fd
  FileSet = "Full Set"
  Storage = File
  Pool = Full
  Priority = 5
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Job(j) => {
                assert_eq!(j.name, "BackupClient1");
                assert_eq!(j.job_type, JobType::Backup);
                assert_eq!(j.level, JobLevel::Full);
                assert_eq!(j.client, "bareos-fd");
                assert_eq!(j.fileset, "Full Set");
                assert_eq!(j.priority, 5);
            }
            _ => panic!("expected Job"),
        }
    }

    #[test]
    fn parse_catalog() {
        let input = r#"
Catalog {
  Name = MyCatalog
  dbname = bareos
  dbuser = bareos
  dbpassword = "dbpass"
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Catalog(c) => {
                assert_eq!(c.name, "MyCatalog");
                assert_eq!(c.db_name, "bareos");
                assert_eq!(c.db_user, "bareos");
                assert_eq!(c.db_password, "dbpass");
            }
            _ => panic!("expected Catalog"),
        }
    }

    #[test]
    fn parse_pool_with_sizes_and_times() {
        let input = r#"
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
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Pool(p) => {
                assert_eq!(p.name, "Full");
                assert!(p.recycle);
                assert!(p.auto_prune);
                assert_eq!(p.volume_retention, Duration::from_secs(365 * 86400));
                assert_eq!(p.maximum_volume_bytes, 50_000_000_000);
                assert_eq!(p.maximum_volumes, 100);
                assert_eq!(p.label_format, "Full-");
            }
            _ => panic!("expected Pool"),
        }
    }

    #[test]
    fn parse_schedule_with_runs() {
        let input = r#"
Schedule {
  Name = "WeeklyCycle"
  Run = Full 1st sun at 23:05
  Run = Differential 2nd-5th sun at 23:05
  Run = Incremental mon-sat at 23:05
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Schedule(s) => {
                assert_eq!(s.name, "WeeklyCycle");
                assert_eq!(s.runs.len(), 3);
                assert_eq!(s.runs[0].level, Some(JobLevel::Full));
                assert_eq!(s.runs[0].hour, 23);
                assert_eq!(s.runs[0].minute, 5);
                assert_eq!(s.runs[1].level, Some(JobLevel::Differential));
                assert_eq!(s.runs[2].level, Some(JobLevel::Incremental));
            }
            _ => panic!("expected Schedule"),
        }
    }

    #[test]
    fn parse_multiple_resources() {
        let input = r#"
Director {
  Name = bareos-dir
  Port = 9101
}

Client {
  Name = bareos-fd
  Address = localhost
}

Storage {
  Name = File
  Address = localhost
  Device = FileStorage
  Media Type = File
}
"#;
        let resources = parse_config(input).unwrap();
        assert_eq!(resources.len(), 3);
        assert!(matches!(&resources[0], Resource::Director(_)));
        assert!(matches!(&resources[1], Resource::Client(_)));
        assert!(matches!(&resources[2], Resource::Storage(_)));
    }

    #[test]
    fn parse_with_comments() {
        let input = r#"
# This is a comment
Director {
  Name = bareos-dir  # inline comment
  # Port = 9102
  Port = 9101
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Director(d) => {
                assert_eq!(d.name, "bareos-dir");
                assert_eq!(d.port, 9101);
            }
            _ => panic!("expected Director"),
        }
    }

    #[test]
    fn parse_console() {
        let input = r#"
Console {
  Name = admin
  Password = "admin-pass"
}
"#;
        let resources = parse_config(input).unwrap();
        match &resources[0] {
            Resource::Console(c) => {
                assert_eq!(c.name, "admin");
                assert_eq!(c.password, "admin-pass");
            }
            _ => panic!("expected Console"),
        }
    }

    #[test]
    fn parse_run_at_time() {
        let run = parse_run_directive("Full at 23:45").unwrap();
        assert_eq!(run.level, Some(JobLevel::Full));
        assert_eq!(run.hour, 23);
        assert_eq!(run.minute, 45);
    }

    #[test]
    fn parse_run_with_level_pools_and_max_run_sched_time() {
        let run = parse_run_directive("Incremental FullPool=F IncrementalPool=I DifferentialPool=D NextPool=N \
            MaxRunSchedTime=7200 daily at 1:00").unwrap();
        assert_eq!(run.full_pool.as_deref(), Some("F"));
        assert_eq!(run.incremental_pool.as_deref(), Some("I"));
        assert_eq!(run.differential_pool.as_deref(), Some("D"));
        assert_eq!(run.next_pool.as_deref(), Some("N"));
        assert_eq!(run.max_run_sched_time, Some(7200));
    }

    #[test]
    fn parse_run_with_pool_override() {
        let run = parse_run_directive("level=Full pool=MyPool at 14:00").unwrap();
        assert_eq!(run.level, Some(JobLevel::Full));
        assert_eq!(run.pool, Some("MyPool".to_string()));
        assert_eq!(run.hour, 14);
        assert_eq!(run.minute, 0);
    }

    #[test]
    fn run_masks_like_store_run() {
        let run = parse_run_directive("Full 1st sat at 21:00").unwrap();
        assert_eq!(run.mask.wday, 1 << 6);
        assert_eq!(run.mask.wom, 1);
        assert_eq!(run.mask.hour, 1 << 21);
        assert_eq!(run.mask.mday, (1 << 31) - 1);
        assert_eq!((run.hour, run.minute), (21, 0));

        let run = parse_run_directive("Differential 2nd-5th sat at 21:00").unwrap();
        assert_eq!(run.mask.wom, 0b11110);
        let run = parse_run_directive("Incremental mon-fri at 21:00").unwrap();
        assert_eq!(run.mask.wday, 0b0111110);
        let run = parse_run_directive("Level=Full fri-mon at 1:05pm").unwrap();
        assert_eq!(run.mask.wday, 0b1100011);
        assert_eq!((run.hour, run.minute), (13, 5));
        let run = parse_run_directive("Full 12:30am").map(|r| r.mask);
        assert!(run.is_none(), "time without at is rejected");
        let run = parse_run_directive("Full at 12:30am").unwrap();
        assert_eq!(run.mask.hour, 1);
        let run = parse_run_directive("Full on 1, 15 jan,jul at 0:05").unwrap();
        assert_eq!(run.mask.mday, 1 | 1 << 14);
        assert_eq!(run.mask.month, 1 | 1 << 6);
        let run = parse_run_directive("Full last sun at 2:00").unwrap();
        assert!(run.mask.last_7days_of_month);
        assert_eq!(run.mask.wom, 0);
        let run = parse_run_directive("Full w02-w04 at 2:00").unwrap();
        assert_eq!(run.mask.woy, 0b11100);
        let run = parse_run_directive("Full w00/w02 at 2:00").unwrap();
        assert_eq!(run.mask.woy & 0b1111, 0b0101);
        let run = parse_run_directive("Full 2/3 at 2:00").unwrap();
        assert_eq!(run.mask.mday & 0b1111111, 0b0010010);
        let run = parse_run_directive("Incremental hourly at 0:15").unwrap();
        assert_eq!(run.mask.hour, (1 << 24) - 1);
        assert_eq!(run.minute, 15);
        // Without "at" every hour triggers (run_conf.cc set_defaults)
        let run = parse_run_directive("Full sun").unwrap();
        assert_eq!(run.mask.hour, (1 << 24) - 1);
        let run = parse_run_directive("Pool=Monthly Storage=File Messages=Standard SpoolData=yes Accurate=no Priority=5 Full at 1:00").unwrap();
        assert_eq!(run.pool.as_deref(), Some("Monthly"));
        assert_eq!(run.storage.as_deref(), Some("File"));
        assert_eq!(run.messages.as_deref(), Some("Standard"));
        assert_eq!((run.spool_data, run.accurate, run.priority), (Some(true), Some(false), Some(5)));
        assert!(parse_run_directive("Full nosuchday at 1:00").is_none());
        assert!(parse_run_directive("Full mon-jan at 1:00").is_none());
    }

    #[test]
    fn unknown_resource_type_skipped() {
        let input = "UnknownType { Name = foo }";
        let result = parse_config(input);
        assert!(result.is_ok()); // Unknown types are skipped, not errors
    }
}
