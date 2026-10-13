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

    // C++ globs <path>/*/*.conf — one level of subdirectories, not recursive.
    // Collect all .conf files from immediate subdirectories.
    let entries = std::fs::read_dir(config_dir)
        .map_err(|e| ConfigError::ParseError(format!("readdir {}: {e}", config_dir.display())))?;
    let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for subdir in &paths {
        if !subdir.is_dir() { continue; }
        let sub_entries = std::fs::read_dir(subdir)
            .map_err(|e| ConfigError::ParseError(format!("readdir {}: {e}", subdir.display())))?;
        let mut conf_files: Vec<_> = sub_entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "conf").unwrap_or(false))
            .collect();
        conf_files.sort();
        // Each file on its own: resources never span files, and errors can
        // name the file (and line) like the C++ parser does.
        for conf in conf_files {
            all.extend(parse_config_file(&conf)?);
        }
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
    let get = |key: &str| directives.get(key).map(|s| s.as_str());
    let require = |key: &str| -> Result<&str, ConfigError> {
        get(key).ok_or_else(|| ConfigError::MissingRequired {
            resource_type: "Job".to_string(),
            directive: key.to_string(),
        })
    };

    let mut job = JobResource::default();
    job.name = require("name")?.to_string();
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
        .map(|d| (d.key.clone(), d.value.clone()))
        .collect();

    let get = |key: &str| directives.get(key).map(|s| s.as_str());
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
            if let Some(v) = get("port") { dir.port = v.parse().unwrap_or(9101); }
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
            if let Some(v) = get("heartbeat interval") {
                dir.heartbeat_interval = parse_time(v).map(|(_, d)| d).unwrap_or_default();
            }
            Ok(Resource::Director(dir))
        }

        "client" => {
            let mut client = ClientResource::default();
            client.name = require("name")?.to_string();
            if let Some(v) = get("address") { client.address = v.to_string(); }
            if let Some(v) = get("port") { client.port = v.parse().unwrap_or(9102); }
            if let Some(v) = get("password") { client.password = v.to_string(); }
            if let Some(v) = get("catalog") { client.catalog = v.to_string(); }
            if let Some(v) = get("maximum concurrent jobs") {
                client.max_concurrent_jobs = v.parse().unwrap_or(1);
            }
            if let Some(v) = get("enabled") {
                client.enabled = parse_bool(v).map(|(_, b)| b).unwrap_or(true);
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
            if let Some(v) = get("port") { stor.port = v.parse().unwrap_or(9103); }
            if let Some(v) = get("password") { stor.password = v.to_string(); }
            if let Some(v) = get("device") { stor.device = v.to_string(); }
            if let Some(v) = get("media type") { stor.media_type = v.to_string(); }
            if let Some(v) = get("maximum concurrent jobs") {
                stor.max_concurrent_jobs = v.parse().unwrap_or(1);
            }
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

/// Parse a Schedule Run directive value.
/// Examples: "Full 1st sun at 23:45", "Incremental mon-sat at 23:45"
fn parse_run_directive(input: &str) -> Option<RunDirective> {
    let input = input.trim();
    let mut run = RunDirective {
        level: None,
        pool: None,
        storage: None,
        priority: None,
        hour: 0,
        minute: 0,
    };

    let tokens: Vec<&str> = input.split_whitespace().collect();
    let mut i = 0;

    while i < tokens.len() {
        let token = tokens[i].to_lowercase();

        // Level keywords
        match token.as_str() {
            "full" => { run.level = Some(JobLevel::Full); i += 1; continue; }
            "incremental" => { run.level = Some(JobLevel::Incremental); i += 1; continue; }
            "differential" => { run.level = Some(JobLevel::Differential); i += 1; continue; }
            "virtualfull" => { run.level = Some(JobLevel::VirtualFull); i += 1; continue; }
            _ => {}
        }

        // Key=value overrides
        if let Some(eq_pos) = token.find('=') {
            let key = &token[..eq_pos];
            let val = &tokens[i][eq_pos + 1..];
            match key {
                "level" => {
                    run.level = match val.to_lowercase().as_str() {
                        "full" => Some(JobLevel::Full),
                        "incremental" => Some(JobLevel::Incremental),
                        "differential" => Some(JobLevel::Differential),
                        _ => None,
                    };
                }
                "pool" => { run.pool = Some(val.to_string()); }
                "storage" => { run.storage = Some(val.to_string()); }
                "priority" => { run.priority = val.parse().ok(); }
                _ => {}
            }
            i += 1;
            continue;
        }

        // "at HH:MM"
        if token == "at" && i + 1 < tokens.len() {
            let time_str = tokens[i + 1];
            if let Some(colon) = time_str.find(':') {
                run.hour = time_str[..colon].parse().unwrap_or(0);
                // Handle am/pm
                let min_part = &time_str[colon + 1..];
                let (min_str, pm) = if min_part.to_lowercase().ends_with("pm") {
                    (&min_part[..min_part.len() - 2], true)
                } else if min_part.to_lowercase().ends_with("am") {
                    (&min_part[..min_part.len() - 2], false)
                } else {
                    (min_part, false)
                };
                run.minute = min_str.parse().unwrap_or(0);
                if pm && run.hour < 12 { run.hour += 12; }
                if !pm && run.hour == 12 { run.hour = 0; } // 12am = 0
            }
            i += 2;
            continue;
        }

        // Skip day/month/ordinal keywords (not parsed into BitMasks yet)
        i += 1;
    }

    Some(run)
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
    fn parse_run_with_pool_override() {
        let run = parse_run_directive("level=Full pool=MyPool at 14:00").unwrap();
        assert_eq!(run.level, Some(JobLevel::Full));
        assert_eq!(run.pool, Some("MyPool".to_string()));
        assert_eq!(run.hour, 14);
        assert_eq!(run.minute, 0);
    }

    #[test]
    fn unknown_resource_type_skipped() {
        let input = "UnknownType { Name = foo }";
        let result = parse_config(input);
        assert!(result.is_ok()); // Unknown types are skipped, not errors
    }
}
