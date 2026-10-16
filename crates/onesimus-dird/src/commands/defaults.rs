//! `.defaults` field order and types from Bareos ua_dotcmds.cc::DotDefaultsCmd.
use crate::{
    command_ast::CommandAst, command_executor::ExecContext, command_queue::CommandResult,
    director_config::DirectorConfig,
};
use bareos_config::raw::{ConfResource, Directive};
use serde_json::{Value, json};

fn raw<'a>(cfg: &'a DirectorConfig, kind: &str, name: &str) -> Option<&'a ConfResource> {
    cfg.raw
        .iter()
        .find(|r| r.norm_type() == kind && r.name() == Some(name))
}
fn directive<'a>(dirs: &'a [Directive], key: &str) -> Option<&'a Directive> {
    dirs.iter().rev().find(|d| d.norm_key() == key)
}
fn first<'a>(dirs: &'a [Directive], key: &str) -> Option<&'a str> {
    directive(dirs, key).and_then(Directive::first)
}
/// Directives of a Job resource after PopulateJobdefaults() (raw config):
/// own directives first, then the items and run scripts of its JobDefs
/// chain (`bareos_config::jobdefs`).
pub(crate) fn job_directives(cfg: &DirectorConfig, name: &str) -> Vec<Directive> {
    bareos_config::jobdefs::effective(&cfg.raw, "job", name)
        .map(|e| bareos_config::jobdefs::directives(&e))
        .unwrap_or_default()
}
/// A time directive in seconds (0 when absent or invalid).
pub(crate) fn time_directive(dirs: &[Directive], key: &str) -> u64 {
    number(dirs, key, 0, TIMES).unwrap_or(0)
}
/// A yes/no directive.
pub(crate) fn bool_directive(dirs: &[Directive], key: &str, default: bool) -> bool {
    boolean(dirs, key, default).map(|v| v != 0).unwrap_or(default)
}
const TIMES: &[(&str, u64)] = &[
    ("n", 60),
    ("seconds", 1),
    ("months", 2592000),
    ("minutes", 60),
    ("mins", 60),
    ("hours", 3600),
    ("days", 86400),
    ("weeks", 604800),
    ("quarters", 7862400),
    ("years", 31536000),
];
const SIZES: &[(&str, u64)] = &[
    ("k", 1024),
    ("kb", 1000),
    ("m", 1048576),
    ("mb", 1000000),
    ("g", 1073741824),
    ("gb", 1000000000),
    ("t", 1099511627776),
    ("tb", 1000000000000),
    ("p", 1125899906842624),
    ("pb", 1000000000000000),
    ("e", 1152921504606846976),
    ("eb", 1000000000000000000),
];
// Bareos parse_number_with_mod accepts compound values and fractional units.
fn quantity(input: &str, mods: &[(&str, u64)]) -> Result<u64, String> {
    let mut rest = input.trim();
    let mut total = 0u64;
    let mut any = false;
    while !rest.is_empty() {
        // Use the same strtod grammar as GetModifier: scientific and hex
        // notation must not be mistaken for the exabyte unit `e`.
        let c_input =
            std::ffi::CString::new(rest).map_err(|_| format!("Invalid quantity: {input}"))?;
        let mut end = std::ptr::null_mut();
        nix::errno::Errno::clear();
        let num = unsafe { libc::strtod(c_input.as_ptr(), &mut end) };
        if num == 0.0 && nix::errno::Errno::last_raw() != 0 {
            return Err(format!("Invalid quantity: {input}"));
        }
        // strtod returns a pointer into the supplied, still live C string.
        let consumed = unsafe { end.offset_from(c_input.as_ptr()) } as usize;
        if consumed == 0 {
            return Err(format!("Invalid quantity: {input}"));
        }
        rest = rest[consumed..].trim_start();
        let u = rest
            .bytes()
            .take_while(|c| !c.is_ascii_digit() && !c.is_ascii_whitespace())
            .count();
        let unit = &rest[..u];
        rest = rest[u..].trim_start();
        let multiplier = if unit.is_empty() {
            1
        } else {
            mods.iter()
                .find(|(key, _)| key.starts_with(&unit.to_ascii_lowercase()))
                .map(|(_, v)| *v)
                .ok_or_else(|| format!("Invalid unit: {unit}"))?
        };
        let value = num * multiplier as f64;
        if !value.is_finite() || value < 0.0 || value >= 18446744073709551616.0 {
            return Err(format!("Quantity overflow: {input}"));
        }
        total = total
            .checked_add(value as u64)
            .ok_or_else(|| format!("Quantity overflow: {input}"))?;
        any = true;
    }
    if any {
        Ok(total)
    } else {
        Err(format!("Invalid quantity: {input}"))
    }
}
fn number(
    dirs: &[Directive],
    key: &str,
    default: u64,
    mods: &[(&str, u64)],
) -> Result<u64, String> {
    match directive(dirs, key) {
        Some(d) => quantity(&d.values().join(" "), mods),
        None => Ok(default),
    }
}
// CFG_TYPE_PINT32 is an unsigned integer directive, not a time/size.
fn pint32(dirs: &[Directive], key: &str, default: u32) -> Result<u32, String> {
    match directive(dirs, key) {
        None => Ok(default),
        Some(d) => d
            .values()
            .join(" ")
            .parse::<u32>()
            .map_err(|_| format!("Invalid unsigned 32-bit directive {key}")),
    }
}
fn boolean(dirs: &[Directive], key: &str, default: bool) -> Result<i32, String> {
    match first(dirs, key) {
        Some(v) => bareos_config::values::parse_bool(v).map(|(_, v)| v as i32),
        None => Ok(default as i32),
    }
}
fn render(fields: Vec<(&str, Value)>) -> CommandResult {
    let text = fields
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}={}\n",
                v.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| v.to_string())
            )
        })
        .collect();
    let values: serde_json::Map<String, Value> = fields
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    CommandResult::ok(text).with_json(json!({"defaults":values}))
}
fn selector(ast: &CommandAst) -> Option<(&str, &str)> {
    ["job", "client", "storage", "pool"]
        .into_iter()
        .find_map(|key| {
            ast.arguments
                .iter()
                .find(|a| a.key.eq_ignore_ascii_case(key))
                .and_then(|a| a.value.as_deref().map(|value| (key, value)))
        })
}
fn defaults(ast: &CommandAst, cfg: &DirectorConfig) -> Result<CommandResult, String> {
    let (kind, name) = selector(ast).ok_or_else(|| ".defaults command requires a parameter.\n".to_string())?;
    let empty = || render(Vec::new());
    let mut f = Vec::new();
    let catalog = |client: &bareos_config::resources::ClientResource| {
        if client.catalog.is_empty() {
            cfg.catalogs.first().map(|c| c.name.as_str()).unwrap_or("")
        } else {
            client.catalog.as_str()
        }
        .to_string()
    };
    let none = |s: &str| {
        if s.is_empty() {
            "*None*".to_string()
        } else {
            s.to_string()
        }
    };
    match kind {
        "job" => {
            let Some(job) = cfg.find_job(name) else {
                return Ok(empty());
            };
            let dirs = job_directives(cfg, name);
            let value =
                |key: &str, fallback: &str| first(&dirs, key).unwrap_or(fallback).to_string();
            let pool = value("pool", &job.pool);
            let client = value("client", &job.client);
            let pool_storage = raw(cfg, "pool", &pool).and_then(|r| {
                r.directives
                    .iter()
                    .find(|d| d.norm_key() == "storage")
                    .and_then(Directive::first)
            });
            let storage = pool_storage.map(str::to_string).unwrap_or_else(|| {
                dirs.iter()
                    .find(|d| d.norm_key() == "storage")
                    .and_then(Directive::first)
                    .unwrap_or(&job.storage)
                    .to_string()
            });
            let kind_value = value("type", &format!("{:?}", job.job_type));
            let kind = [
                "Backup",
                "Restore",
                "Verify",
                "Admin",
                "Archive",
                "Copy",
                "Migrate",
                "Consolidate",
            ]
            .into_iter()
            .find(|k| k.eq_ignore_ascii_case(&kind_value))
            .unwrap_or("Unknown Type");
            f.push(("job", json!(job.name)));
            f.push(("pool", json!(pool)));
            f.push(("messages", json!(value("messages", &job.messages))));
            f.push(("client", json!(none(&client))));
            f.push(("storage", json!(none(&storage))));
            // RegexWhere causes ValidateResource to clear Where.
            f.push((
                "where",
                json!(if ["regexwhere", "stripprefix", "addprefix", "addsuffix"]
                    .iter()
                    .any(|key| first(&dirs, key).is_some())
                {
                    "".to_string()
                } else {
                    value("where", "")
                }),
            ));
            if kind.eq_ignore_ascii_case("restore") {
                f.push((
                    "replace",
                    json!(value("replace", "always").to_ascii_lowercase()),
                ));
            }
            // C++ JobLevel starts at 0; JobLevelToString emits an empty
            // C string for that code, rather than the runtime Full fallback.
            let level_fallback = if raw(cfg, "job", name).is_some() {
                String::new()
            } else {
                format!("{:?}", job.level)
            };
            let level_value = value("level", &level_fallback);
            let level = [
                "Full",
                "Base",
                "Incremental",
                "Differential",
                "Since",
                "VirtualFull",
                "Catalog",
                "InitCatalog",
                "VolumeToCatalog",
                "DiskToCatalog",
                "Data",
                " ",
            ]
            .into_iter()
            .find(|level| level.eq_ignore_ascii_case(&level_value))
            .unwrap_or(&level_value);
            f.push(("level", json!(level)));
            f.push(("type", json!(kind)));
            f.push(("fileset", json!(none(&value("fileset", &job.fileset)))));
            f.push(("enabled", json!(boolean(&dirs, "enabled", job.enabled)?)));
            f.push((
                "catalog",
                json!(
                    cfg.find_client(&client)
                        .map(catalog)
                        .unwrap_or_else(|| "*None*".to_string())
                ),
            ));
        }
        "client" => {
            let Some(c) = cfg.find_client(name) else {
                return Ok(empty());
            };
            let dirs = raw(cfg, "client", name)
                .map(|r| r.directives.as_slice())
                .unwrap_or(&[]);
            f.extend([
                ("client", json!(c.name)),
                ("address", json!(c.address)),
                ("port", json!(c.port)),
                (
                    "file_retention",
                    json!(number(dirs, "fileretention", 5184000, TIMES)?.to_string()),
                ),
                (
                    "job_retention",
                    json!(number(dirs, "jobretention", 15552000, TIMES)?.to_string()),
                ),
                ("autoprune", json!(boolean(dirs, "autoprune", false)?)),
                ("enabled", json!(c.enabled as i32)),
                ("catalog", json!(catalog(c))),
            ]);
        }
        "storage" => {
            let Some(s) = cfg.find_storage(name) else {
                return Ok(empty());
            };
            let dirs = raw(cfg, "storage", name)
                .map(|r| r.directives.as_slice())
                .unwrap_or(&[]);
            let devices: Vec<&str> = dirs
                .iter()
                .filter(|d| d.norm_key() == "device")
                .flat_map(Directive::values)
                .collect();
            f.extend([
                ("storage", json!(s.name)),
                ("address", json!(s.address)),
                ("port", json!(s.port)),
                ("enabled", json!(s.enabled as i32)),
                ("media_type", json!(s.media_type)),
                (
                    "device",
                    json!(if devices.is_empty() {
                        s.device.clone()
                    } else {
                        devices.join(",")
                    }),
                ),
            ]);
        }
        "pool" => {
            let Some(p) = cfg.find_pool(name) else {
                return Ok(empty());
            };
            let dirs = raw(cfg, "pool", name)
                .map(|r| r.directives.as_slice())
                .unwrap_or(&[]);
            f.extend([
                ("pool", json!(p.name)),
                ("pool_type", json!(p.pool_type)),
                ("label_format", json!(p.label_format)),
                (
                    "purge_oldest_volume=",
                    json!(boolean(dirs, "purgeoldestvolume", false)?),
                ),
                (
                    "recycle_oldest_volume",
                    json!(boolean(dirs, "recycleoldestvolume", false)?),
                ),
                (
                    "max_volumes",
                    json!(pint32(dirs, "maximumvolumes", p.maximum_volumes)?),
                ),
                (
                    "vol_retention",
                    json!(
                        number(dirs, "volumeretention", p.volume_retention.as_secs(), TIMES)?
                            .to_string()
                    ),
                ),
                (
                    "vol_use_duration",
                    json!(number(dirs, "volumeuseduration", 0, TIMES)?.to_string()),
                ),
                ("max_vol_jobs", json!(pint32(dirs, "maximumvolumejobs", 0)?)),
                (
                    "max_vol_files",
                    json!(pint32(dirs, "maximumvolumefiles", 0)?),
                ),
                (
                    "max_vol_bytes",
                    json!(
                        number(dirs, "maximumvolumebytes", p.maximum_volume_bytes, SIZES)?
                            .to_string()
                    ),
                ),
                ("auto_prune", json!(p.auto_prune as i32)),
                ("recycle", json!(p.recycle as i32)),
                (
                    "file_retention",
                    json!(number(dirs, "fileretention", 0, TIMES)?.to_string()),
                ),
                (
                    "job_retention",
                    json!(number(dirs, "jobretention", 0, TIMES)?.to_string()),
                ),
            ]);
        }
        _ => unreachable!(),
    }
    Ok(render(f))
}
pub fn cmd_defaults(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some(config) = ctx.config.as_ref() else {
        return CommandResult::error("Director configuration unavailable.\n".to_string());
    };
    let Ok(cfg) = config.read() else {
        return CommandResult::error("Director configuration unavailable.\n".to_string());
    };
    // Snapshot before access() acquires the configuration lock again. Nested
    // RwLock reads can deadlock when a configuration reload is waiting.
    let snapshot = cfg.clone();
    drop(cfg);
    let cfg = snapshot;
    if let Some((kind, name)) = selector(ast) {
        let target = match kind {
            "job" => cfg.find_job(name).map(|r| (crate::acl::Kind::Job, r.name.as_str())),
            "client" => cfg.find_client(name).map(|r| (crate::acl::Kind::Client, r.name.as_str())),
            "storage" => cfg.find_storage(name).map(|r| (crate::acl::Kind::Storage, r.name.as_str())),
            "pool" => cfg.find_pool(name).map(|r| (crate::acl::Kind::Pool, r.name.as_str())),
            _ => unreachable!(),
        };
        // DotDefaultsCmd checks only the selected resource. Job-linked
        // resources are read directly, without additional ACL lookups.
        // Get*ResWithName(argv) audits a denial (ua_dotcmds.cc DotDefaultsCmd).
        if target.is_some_and(|(kind, name)| !crate::acl::access_audited(ctx, kind, name)) {
            return render(Vec::new());
        }
    }
    defaults(ast, &cfg).unwrap_or_else(|message| {
        if message == ".defaults command requires a parameter.\n" {
            CommandResult {
                success: false,
                text_output: message,
                ..Default::default()
            }
        } else {
            CommandResult::error(message)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_ast::Argument;
    fn config(extra: &str) -> DirectorConfig {
        DirectorConfig::from_str(&format!(
            r#"
Director {{ Name = dir; Password = secret }}
Catalog {{ Name = Cat }}
Client {{ Name = fd; Address = localhost; Password = secret }}
Storage {{ Name = sd; Address = localhost; Password = secret; Device = tape; MediaType = File }}
Pool {{ Name = Backup; PoolType = Backup; Storage = sd }}
Messages {{ Name = Standard }}
{extra}
"#
        ))
        .unwrap()
    }
    fn run(cfg: &DirectorConfig, kind: &str, name: &str) -> CommandResult {
        defaults(
            &CommandAst {
                arguments: vec![Argument {
                    key: kind.into(),
                    value: Some(name.into()),
                }],
                ..Default::default()
            },
            cfg,
        )
        .unwrap()
    }
    fn restricted_context(policy: &str) -> ExecContext {
        use std::sync::{Arc, RwLock};
        let cfg = config(&format!("Job {{Name=J; Type=Backup; Pool=Backup; Client=fd; Storage=sd}}\nConsole {{Name=limited; Password=x; {policy}}}"));
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session("limited");
        manager.authenticate_session(id);
        ExecContext {session_id:id, config:Some(Arc::new(RwLock::new(cfg))), session_mgr:Some(manager),
            catalog:None, api_mode:crate::command_executor::ApiMode::Human, bus_rx:None, job_manager:None}
    }
    #[test]
    fn defaults_denied_primary_resources_are_indistinguishable_from_missing() {
        let ctx = restricted_context("");
        for (kind, name) in [("job", "J"), ("client", "fd"), ("storage", "sd"), ("pool", "Backup")] {
            let result = cmd_defaults(&crate::session::parse_cli_command(&format!(".defaults {kind}={name}")), &ctx);
            assert!(result.success);
            assert_eq!(result.text_output, "");
            assert_eq!(result.json, Some(json!({"defaults":{}})));
        }
    }
    #[test]
    fn defaults_allowed_job_keeps_linked_metadata_as_in_original() {
        // Original checks JobACL once, then directly reads linked pointers.
        let ctx = restricted_context("JobACL=J; ClientACL=!fd; StorageACL=!sd; PoolACL=!Backup; FileSetACL=!*all*");
        let result = cmd_defaults(&crate::session::parse_cli_command(".defaults job=J"), &ctx);
        assert!(result.success);
        let fields = &result.json.unwrap()["defaults"];
        assert_eq!(fields["job"], "J");
        assert_eq!(fields["client"], "fd");
        assert_eq!(fields["pool"], "Backup");
        assert_eq!(fields["storage"], "sd");
    }
    #[test]
    fn defaults_resource_acl_is_read_again_after_reload() {
        let ctx = restricted_context("ClientACL=fd");
        let command = crate::session::parse_cli_command(".defaults client=fd");
        assert!(cmd_defaults(&command, &ctx).text_output.contains("client=fd"));
        ctx.config.as_ref().unwrap().write().unwrap().raw.retain(|r| r.norm_type() != "console");
        assert_eq!(cmd_defaults(&command, &ctx).json, Some(json!({"defaults":{}})));
    }
    #[test]
    fn client_defaults_include_retention_and_boolean_types() {
        let cfg = config("");
        let r = run(&cfg, "client", "fd");
        assert_eq!(
            r.text_output,
            "client=fd\naddress=localhost\nport=9102\nfile_retention=5184000\njob_retention=15552000\nautoprune=0\nenabled=1\ncatalog=Cat\n"
        );
        assert_eq!(
            r.json.unwrap()["defaults"]["file_retention"],
            json!("5184000")
        );
        let cfg = config(
            "Client {Name=custom; Address=localhost; Password=x; FileRetention=2 days 3 hours; JobRetention=4 weeks; AutoPrune=yes}",
        );
        let r = run(&cfg, "client", "custom");
        assert!(
            r.text_output
                .contains("file_retention=183600\njob_retention=2419200\nautoprune=1\n")
        );
    }
    #[test]
    fn pool_all_fields_and_upstream_double_equals() {
        let cfg = config(
            "Pool {Name=Custom; PurgeOldestVolume=yes; RecycleOldestVolume=yes; VolumeUseDuration=2 hours; MaximumVolumeJobs=5; MaximumVolumeFiles=6; MaximumVolumeBytes=2G; FileRetention=3 days; JobRetention=4 days}",
        );
        let r = run(&cfg, "pool", "Custom");
        let j = r.json.unwrap();
        assert_eq!(j["defaults"].as_object().unwrap().len(), 15);
        assert!(r.text_output.contains("purge_oldest_volume==1\n"));
        assert_eq!(j["defaults"]["purge_oldest_volume="], json!(1));
        assert_eq!(j["defaults"]["vol_use_duration"], json!("7200"));
        assert_eq!(j["defaults"]["max_vol_bytes"], json!("2147483648"));
        assert_eq!(j["defaults"]["max_vol_jobs"], json!(5));
    }
    #[test]
    fn device_lists_include_every_directive() {
        let cfg = config(
            "Storage {Name=Multi; Address=localhost; Password=x; Device=one,two; Device=three; MediaType=File}",
        );
        assert_eq!(
            run(&cfg, "storage", "Multi").json.unwrap()["defaults"]["device"],
            json!("one,two,three")
        );
    }
    #[test]
    fn restore_job_inherits_where_replace_and_pool_storage() {
        let cfg = config(
            r#"
JobDefs {Name=RestoreDefaults; Type=Restore; Pool=Backup; Messages=Standard; Client=fd; Storage=ignored; Where=/restore; Replace=IfOlder; Level=Full}
Job {Name=Restore; JobDefs=RestoreDefaults}
"#,
        );
        let r = run(&cfg, "job", "Restore");
        let j = r.json.unwrap();
        assert_eq!(j["defaults"]["storage"], json!("sd"));
        assert_eq!(j["defaults"]["where"], json!("/restore"));
        assert_eq!(j["defaults"]["replace"], json!("ifolder"));
        assert_eq!(j["defaults"]["type"], json!("Restore"));
    }
    #[test]
    fn explicit_job_values_override_jobdefs_and_regex_where() {
        let cfg = config(
            r#"
JobDefs {Name=D; Type=Restore; Level=Incremental; Where=/restore; Replace=Never; Pool=Backup; Client=fd}
Job {Name=J; JobDefs=D; Level=Full; RegexWhere="!a!b!"; Replace=Always}
"#,
        );
        let j = run(&cfg, "job", "J").json.unwrap();
        assert_eq!(j["defaults"]["level"], json!("Full"));
        assert_eq!(j["defaults"]["where"], json!(""));
        assert_eq!(j["defaults"]["replace"], json!("always"));
    }
    #[test]
    fn jobdefs_chain_and_explicit_full() {
        let cfg = config(
            r#"
JobDefs {Name=C; Type=Backup; Level=Incremental; Pool=Backup; Client=fd; Messages=Standard; Where=/c}
JobDefs {Name=B; JobDefs=C; Level=Differential}
Job {Name=J; JobDefs=B; Level=Full}
Job {Name=K; JobDefs=B}
"#,
        );
        let j = run(&cfg, "job", "J").json.unwrap();
        assert_eq!(j["defaults"]["level"], json!("Full"));
        assert_eq!(j["defaults"]["client"], json!("fd"));
        assert_eq!(j["defaults"]["where"], json!("/c"));
        let k = run(&cfg, "job", "K").json.unwrap();
        assert_eq!(k["defaults"]["level"], json!("Differential"));
        assert_eq!(cfg.find_job("J").unwrap().level, bareos_config::JobLevel::Full);
        assert_eq!(cfg.find_job("K").unwrap().level, bareos_config::JobLevel::Differential);
        assert_eq!(cfg.find_job("K").unwrap().client, "fd");
    }
    #[test]
    fn strtod_exponents_and_modifiers_match_original_grammar() {
        assert_eq!(quantity("1e3", TIMES).unwrap(), 1000);
        assert!(quantity("1e-9999", TIMES).is_err());
        assert_eq!(quantity("1e3", SIZES).unwrap(), 1000);
        assert_eq!(quantity("1e-3 hours", TIMES).unwrap(), 3);
        assert_eq!(quantity("0x1p10", SIZES).unwrap(), 1024);
        assert_eq!(quantity("1e 1", SIZES).unwrap(), 1152921504606846977);
        assert!(quantity("1 day, 2 hours", TIMES).is_err());
        assert!(quantity("1 invalid", TIMES).is_err());
    }
    #[test]
    fn pool_integer_limits_reject_fraction_and_overflow() {
        for value in [".5", "4294967296", "-1", "1e3"] {
            let cfg = config(&format!("Pool {{Name=P; MaximumVolumeJobs={value}}}"));
            let ast = crate::session::parse_cli_command(".defaults pool=P");
            assert!(defaults(&ast, &cfg).is_err(), "{value}");
        }
    }
    #[test]
    fn first_bare_selector_masks_later_value_as_in_find_arg_with_value() {
        let cfg = config("Job {Name=J; Type=Backup; Pool=Backup}");
        let ast = crate::session::parse_cli_command(".defaults job job=J client=fd");
        let j = defaults(&ast, &cfg).unwrap().json.unwrap();
        assert_eq!(j["defaults"]["client"], json!("fd"));
        assert!(j["defaults"].get("job").is_none());
    }
    #[test]
    fn absent_level_remains_empty_and_prefix_clears_where() {
        let cfg = config("Job {Name=R; Type=Restore; Pool=Backup; Where=/restore; AddPrefix=/new}");
        let j = run(&cfg, "job", "R").json.unwrap();
        assert_eq!(j["defaults"]["level"], json!(""));
        assert_eq!(j["defaults"]["where"], json!(""));
        assert_eq!(j["defaults"]["replace"], json!("always"));
    }
    #[test]
    fn selectors_fail_missing_parameter_and_missing_resource_is_empty() {
        let cfg = config("");
        assert!(
            defaults(&CommandAst::default(), &cfg)
                .unwrap_err()
                .contains("requires a parameter")
        );
        assert_eq!(
            run(&cfg, "client", "missing").json,
            Some(json!({"defaults":{}}))
        );
        for bad in ["18446744073709551616", "-1", "1 invalid", "1 hour garbage"] {
            assert!(quantity(bad, TIMES).is_err(), "{bad}");
        }
    }
}
