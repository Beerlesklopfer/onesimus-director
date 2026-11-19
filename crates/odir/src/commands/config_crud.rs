//! `.config` — configuration CRUD (raw text) over the classic console
//! (BNET). Onesimus extension without a bareos-dir counterpart; the same
//! operations as the DirectorApi RPCs (docs/plans/CONFIG-CRUD-GRPC.md,
//! section "BNET-Weg") with the same core logic (`crate::config_edit`) and
//! the same editor instance (one lock for gRPC and console).
//!
//! ```text
//! .config capabilities
//! .config list   type=<type>
//! .config read   type=<type> name=<name>
//! .config create type=<type> text64=<base64> [validate_only]
//! .config update type=<type> name=<name> sha256=<old> text64=<base64> [validate_only]
//! .config delete type=<type> name=<name> sha256=<old> [validate_only]
//! ```
//!
//! `<type>`: schedule, job, jobdefs, client (case-insensitive). The text is
//! passed base64 encoded (standard alphabet, padding optional) because a
//! console line cannot carry line ends; the answer carries it as UTF-8.
//!
//! Output: `.api 2` gives the JSON object (`result`) with the fields of
//! ResourceResponse, ListResourcesResponse or ConfigCapabilitiesResponse;
//! `status` and types are text (`OK`, `CONFLICT`, …; `SCHEDULE`, `CLIENT`,
//! …). The other modes print a readable text; the keyword `json` prints the
//! JSON object as one line in every mode (MCP).
//!
//! Rights: the Command ACL `.config` (as for every command) and in addition
//! `configure` (like the gRPC RPCs); without `configure` every operation
//! but `capabilities` fails, and `capabilities` lists no type. The resource
//! ACL of the type decides per name (status DENIED; `list` leaves denied
//! names out). Writes are logged like the gRPC path (journal and Director
//! message). The audit log of the command line shows `text64=` masked.

#![allow(clippy::result_large_err)] // CommandResult as the error of a closure, as in the other commands

use base64::Engine;
use serde_json::{json, Value};

use crate::acl::{self, Kind};
use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;
use crate::config_edit::{self, ConfigEditor, Outcome, RType, Summary};

/// interface_version, as GetConfigCapabilities.
pub const INTERFACE_VERSION: u32 = 1;

/// Operations implemented for every editable type.
pub const OPERATIONS: [&str; 4] = ["READ_RAW", "CREATE_RAW", "UPDATE_RAW", "DELETE"];

fn value<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter().rev().find(|a| a.key.eq_ignore_ascii_case(key)).and_then(|a| a.value.as_deref())
}

fn flag(ast: &CommandAst, key: &str) -> bool {
    ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_none())
}

/// Type word in the answers (proto names without the prefix).
pub fn type_text(rtype: RType) -> &'static str {
    match rtype {
        RType::Schedule => "SCHEDULE",
        RType::Job => "JOB",
        RType::JobDefs => "JOBDEFS",
        RType::Client => "CLIENT",
    }
}

fn parse_type(text: &str) -> Option<RType> {
    let lower = text.to_ascii_lowercase();
    RType::ALL.into_iter().find(|t| t.norm() == lower || type_text(*t).eq_ignore_ascii_case(&lower))
}

pub fn status_text(status: config_edit::Status) -> &'static str {
    use config_edit::Status::*;
    match status {
        Ok => "OK",
        Conflict => "CONFLICT",
        Invalid => "INVALID",
        Referenced => "REFERENCED",
        Denied => "DENIED",
        NotFound => "NOT_FOUND",
        Exists => "EXISTS",
        ReloadFailed => "RELOAD_FAILED",
    }
}

/// ResourceResponse as JSON.
pub fn outcome_json(rtype: RType, o: &Outcome) -> Value {
    json!({
        "status": status_text(o.status),
        "message": o.message,
        "resource": { "type": type_text(rtype), "name": o.name },
        "file": o.file,
        "text": o.text,
        "sha256": o.sha256,
        "shared_file": o.shared_file,
        "referenced_by": o.referenced_by.iter()
            .map(|(t, name)| json!({ "type": t.to_ascii_uppercase(), "name": name })).collect::<Vec<_>>(),
        "problem": o.problem.as_ref().map(|p| json!({ "file": p.file, "line": p.line, "column": p.column, "text": p.text })),
        "reloaded": o.reloaded,
        "validated_only": o.validated_only,
        "old_sha256": o.old_sha256,
        "followups": o.followups,
    })
}

fn outcome_text(rtype: RType, o: &Outcome) -> String {
    let mut out = format!("status: {}\n", status_text(o.status));
    if !o.message.is_empty() {
        out.push_str(&format!("message: {}\n", o.message));
    }
    out.push_str(&format!("resource: {} \"{}\"\n", rtype.word(), o.name));
    for (key, value) in [("file", &o.file), ("sha256", &o.sha256), ("old_sha256", &o.old_sha256)] {
        if !value.is_empty() {
            out.push_str(&format!("{key}: {value}\n"));
        }
    }
    if o.shared_file {
        out.push_str("shared_file: yes\n");
    }
    for (t, name) in &o.referenced_by {
        out.push_str(&format!("referenced_by: {t} \"{name}\"\n"));
    }
    if let Some(p) = &o.problem {
        out.push_str(&format!("problem: {}:{}:{}\n{}", p.file, p.line, p.column, p.text));
        if !p.text.ends_with('\n') {
            out.push('\n');
        }
    }
    if o.reloaded {
        out.push_str("reloaded: yes\n");
    }
    if o.validated_only {
        out.push_str("validated_only: yes\n");
    }
    for f in &o.followups {
        out.push_str(&format!("followup: {f}\n"));
    }
    if !o.text.is_empty() {
        out.push_str("text:\n");
        out.push_str(&o.text);
        if !o.text.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn output(ast: &CommandAst, ctx: &ExecContext, object: Value, text: String) -> CommandResult {
    let body = if ctx.api_mode != ApiMode::Json && flag(ast, "json") {
        format!("{}\n", serde_json::to_string(&object).unwrap_or_default())
    } else {
        text
    };
    CommandResult::ok(body).with_json(object)
}

fn failure(message: impl Into<String>) -> CommandResult {
    let mut message = message.into();
    if !message.ends_with('\n') {
        message.push('\n');
    }
    CommandResult::error(message)
}

fn console_of(ctx: &ExecContext) -> String {
    match &ctx.session_mgr {
        Some(manager) => manager.authenticated_console(ctx.session_id).unwrap_or_default(),
        None => "*UserAgent*".into(),
    }
}

/// The base64 text of `text64=`.
fn text64(ast: &CommandAst) -> Result<String, String> {
    let encoded = value(ast, "text64").ok_or("text64=<base64> is required")?;
    let engine = base64::engine::general_purpose::GeneralPurposeConfig::new()
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent);
    let engine = base64::engine::GeneralPurpose::new(&base64::alphabet::STANDARD, engine);
    let bytes = engine.decode(encoded.trim()).map_err(|e| format!("text64: not base64 ({e})"))?;
    String::from_utf8(bytes).map_err(|_| "text64: the text is not UTF-8".to_string())
}

/// The audit line of a `.config` command: the value of `text64=` replaced
/// by its length (the text may carry passwords).
pub fn mask_cmdline(cmdline: &str) -> String {
    cmdline.split(' ').map(|word| {
        if word.len() > 7 && word[..7].eq_ignore_ascii_case("text64=") {
            format!("text64=<{} bytes>", word.len() - 7)
        } else {
            word.to_string()
        }
    }).collect::<Vec<_>>().join(" ")
}

const USAGE: &str = "Usage: .config capabilities | list type=<type> | read type=<type> name=<name> | \
    create type=<type> text64=<base64> [validate_only] | update type=<type> name=<name> sha256=<sha> text64=<base64> \
    [validate_only] | delete type=<type> name=<name> sha256=<sha> [validate_only]; type: schedule, job, jobdefs, client";

/// `.config …` (production editor: the configuration path of the Director).
pub fn cmd_dot_config(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let editor = || -> Result<std::sync::Arc<ConfigEditor>, CommandResult> {
        ctx.config.as_ref().and_then(config_edit::shared_editor)
            .ok_or_else(|| failure("configuration editing is not available (no configuration path)"))
    };
    dot_config(ast, ctx, &editor)
}

/// `.config …` with the editor from `editor` (tests: a throw-away configuration).
pub fn dot_config(ast: &CommandAst, ctx: &ExecContext,
                  editor: &dyn Fn() -> Result<std::sync::Arc<ConfigEditor>, CommandResult>) -> CommandResult {
    let Some(op) = ast.arguments.first().filter(|a| a.value.is_none()).map(|a| a.key.to_ascii_lowercase()) else {
        return failure(USAGE);
    };
    let configure = acl::access(ctx, Kind::Command, "configure");
    if op == "capabilities" {
        let mut types = Vec::new();
        if configure {
            let console = console_of(ctx);
            let config = ctx.config.as_ref().and_then(|c| c.read().ok());
            for rtype in RType::ALL {
                let any = match (&ctx.session_mgr, &config) {
                    (None, _) => true,
                    (Some(_), Some(cfg)) => acl::any_allowed(cfg, &console, rtype.acl()),
                    _ => false,
                };
                if any {
                    types.push(json!({ "type": type_text(rtype), "operations": OPERATIONS }));
                }
            }
        }
        let text = format!("interface_version: {INTERFACE_VERSION}\n{}",
            types.iter().map(|t| format!("{}: {}\n", t["type"].as_str().unwrap_or(""),
                OPERATIONS.join(", "))).collect::<String>());
        return output(ast, ctx, json!({ "interface_version": INTERFACE_VERSION, "types": types }), text);
    }
    if !matches!(op.as_str(), "list" | "read" | "create" | "update" | "delete") {
        return failure(format!("unknown operation \"{op}\". {USAGE}"));
    }
    if !configure {
        return failure("command \"configure\" not allowed");
    }
    let Some(type_word) = value(ast, "type") else { return failure(format!("type=<type> is required. {USAGE}")) };
    let Some(rtype) = parse_type(type_word) else {
        return failure(format!("type \"{type_word}\" cannot be edited (schedule, job, jobdefs, client)"));
    };
    let editor = match editor() {
        Ok(e) => e,
        Err(r) => return r,
    };
    let kind = rtype.acl();
    let allow = |name: &str| acl::access(ctx, kind, name);
    let name = || value(ast, "name").filter(|n| !n.is_empty()).ok_or_else(|| failure("name=<name> is required"));
    let sha = || value(ast, "sha256").unwrap_or("").to_string();
    let validate_only = flag(ast, "validate_only");
    let console = console_of(ctx);
    let result = match op.as_str() {
        "list" => {
            return match editor.list(rtype, &allow) {
                Ok(list) => list_output(ast, ctx, rtype, &list),
                Err(e) => failure(e),
            };
        }
        "read" => match name() {
            Ok(n) => editor.read(rtype, n, &allow),
            Err(r) => return r,
        },
        "create" => match text64(ast) {
            Ok(text) => editor.create(rtype, &text, validate_only, &console, &allow),
            Err(e) => return failure(e),
        },
        "update" => {
            let n = match name() {
                Ok(n) => n,
                Err(r) => return r,
            };
            match text64(ast) {
                Ok(text) => editor.update(rtype, n, &sha(), &text, validate_only, &console, &allow),
                Err(e) => return failure(e),
            }
        }
        _ => match name() {
            Ok(n) => editor.delete(rtype, n, &sha(), validate_only, &console, &allow),
            Err(r) => return r,
        },
    };
    match result {
        Ok(o) => output(ast, ctx, outcome_json(rtype, &o), outcome_text(rtype, &o)),
        Err(e) => failure(format!("configuration edit: {e}")),
    }
}

fn list_output(ast: &CommandAst, ctx: &ExecContext, rtype: RType, list: &[Summary]) -> CommandResult {
    let resources: Vec<Value> = list.iter().map(|s| json!({
        "resource": { "type": type_text(rtype), "name": s.name },
        "file": s.file,
        "sha256": s.sha256,
        "shared_file": s.shared_file,
    })).collect();
    let text: String = list.iter().map(|s| format!("{} {} {}{}\n", s.name, s.sha256, s.file,
        if s.shared_file { " (shared)" } else { "" })).collect();
    output(ast, ctx, json!({ "resources": resources }), text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_edit::tests::{harness, Harness, CLIENT_TLS, WEEKLY};
    use crate::director_config::DirectorConfig;
    use std::sync::{Arc, RwLock};

    fn b64(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    /// Context of an authenticated console of the harness configuration.
    fn context(h: &Harness, console: &str) -> ExecContext {
        std::fs::write(h.dir.join("bareos-dir.d/console/consoles.conf"), "\
Console {\n  Name = admin\n  Password = pw\n  CommandACL = .config, configure\n  ClientACL = fd, laptop-fd\n  ScheduleACL = *all*\n}\n\
Console {\n  Name = viewer\n  Password = pw\n  CommandACL = .config\n  ClientACL = *all*\n}\n").unwrap();
        let config = Arc::new(RwLock::new(DirectorConfig::load(&h.dir).unwrap()));
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session(console);
        manager.authenticate_session(id);
        ExecContext { session_id: id, config: Some(config), session_mgr: Some(manager), catalog: None,
            api_mode: ApiMode::Json, bus_rx: None, job_manager: None }
    }

    fn run(h: &Arc<Harness>, ctx: &ExecContext, line: &str) -> CommandResult {
        let ast = crate::session::parse_cli_command(line);
        let h = h.clone();
        dot_config(&ast, ctx, &move || Ok(Arc::new(h_editor(&h))))
    }

    // The harness owns its editor; the console layer needs an Arc. A second
    // editor on the same directory shares the reload/notify closures' effects
    // through the files, which is all these tests look at.
    fn h_editor(h: &Harness) -> ConfigEditor {
        let d = h.dir.clone();
        ConfigEditor::new(h.dir.clone(), Box::new(move || {
            let config = DirectorConfig::load(&d).map_err(|e| e.to_string())?;
            crate::config_check::check(&config.raw, &crate::config_check::base_config_path(&d))
                .map_err(|diags| diags[0].text().to_string())
        }), Box::new(|_| {}))
    }

    #[test]
    fn crud_over_the_console() {
        let h = Arc::new(harness("dot-config"));
        let ctx = context(&h, "admin");
        let caps = run(&h, &ctx, ".config capabilities").json.unwrap();
        assert_eq!(caps["interface_version"], 1);
        let types: Vec<&str> = caps["types"].as_array().unwrap().iter().map(|t| t["type"].as_str().unwrap()).collect();
        assert_eq!(types, ["SCHEDULE", "CLIENT"]);

        let created = run(&h, &ctx, &format!(".config create type=client text64={}", b64(CLIENT_TLS))).json.unwrap();
        assert_eq!(created["status"], "OK", "{created}");
        assert_eq!(created["reloaded"], true);
        let path = h.dir.join("bareos-dir.d/client/laptop-fd.conf");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), CLIENT_TLS);

        let read = run(&h, &ctx, ".config read type=Client name=laptop-fd").json.unwrap();
        assert_eq!((read["text"].as_str().unwrap(), read["resource"]["type"].as_str().unwrap()), (CLIENT_TLS, "CLIENT"));
        let sha = read["sha256"].as_str().unwrap().to_string();
        assert_eq!(sha, config_edit::sha256_hex(CLIENT_TLS));

        let changed = CLIENT_TLS.replace("Heartbeat Interval = 60", "Heartbeat Interval = 90");
        // base64 without padding is accepted
        let conflict = run(&h, &ctx, &format!(".config update type=client name=laptop-fd sha256=00 text64={}", b64(&changed).trim_end_matches('='))).json.unwrap();
        assert_eq!((conflict["status"].as_str(), conflict["sha256"].as_str()), (Some("CONFLICT"), Some(sha.as_str())));
        let validated = run(&h, &ctx, &format!(".config update type=client name=laptop-fd sha256={sha} text64={} validate_only", b64(&changed))).json.unwrap();
        assert_eq!((validated["status"].as_str(), validated["validated_only"].as_bool()), (Some("OK"), Some(true)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), CLIENT_TLS);
        let updated = run(&h, &ctx, &format!(".config update type=client name=laptop-fd sha256={sha} text64={}", b64(&changed))).json.unwrap();
        assert_eq!((updated["status"].as_str(), updated["old_sha256"].as_str()), (Some("OK"), Some(sha.as_str())));
        assert_eq!(std::fs::read(&path).unwrap(), changed.as_bytes());

        let invalid = run(&h, &ctx, &format!(".config update type=client name=laptop-fd sha256={} text64={}",
            config_edit::sha256_hex(&changed), b64(&changed.replace("Heartbeat Interval", "Heartbeat Nonsense")))).json.unwrap();
        assert_eq!(invalid["status"], "INVALID");
        assert!(invalid["problem"]["line"].as_u64().unwrap() > 0, "{invalid}");

        let list = run(&h, &ctx, ".config list type=client").json.unwrap();
        let names: Vec<&str> = list["resources"].as_array().unwrap().iter().map(|r| r["resource"]["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["laptop-fd", "fd"]);

        let fd = run(&h, &ctx, ".config read type=client name=fd").json.unwrap();
        let referenced = run(&h, &ctx, &format!(".config delete type=client name=fd sha256={}", fd["sha256"].as_str().unwrap())).json.unwrap();
        assert_eq!(referenced["status"], "REFERENCED");
        assert_eq!(referenced["referenced_by"], json!([{ "type": "JOB", "name": "a" }]));

        let deleted = run(&h, &ctx, &format!(".config delete type=client name=laptop-fd sha256={}", config_edit::sha256_hex(&changed))).json.unwrap();
        assert_eq!(deleted["status"], "OK", "{deleted}");
        assert!(!path.exists());
    }

    #[test]
    fn acl_and_arguments() {
        let h = Arc::new(harness("dot-config-acl"));
        let admin = context(&h, "admin");
        // Client ACL per name
        let denied = run(&h, &admin, &format!(".config create type=client text64={}", b64(&CLIENT_TLS.replace("laptop-fd", "other-fd")))).json.unwrap();
        assert_eq!(denied["status"], "DENIED");
        // Job ACL: none allowed
        let job = run(&h, &admin, ".config read type=job name=a").json.unwrap();
        assert_eq!(job["status"], "DENIED");
        assert!(run(&h, &admin, ".config list type=job").json.unwrap()["resources"].as_array().unwrap().is_empty());
        // without configure: no types, every other operation fails
        let viewer = context(&h, "viewer");
        let caps = run(&h, &viewer, ".config capabilities").json.unwrap();
        assert!(caps["types"].as_array().unwrap().is_empty());
        let r = run(&h, &viewer, ".config read type=client name=fd");
        assert!(!r.success && r.error_message.contains("configure"), "{r:?}");
        // arguments
        for line in [".config", ".config frobnicate", ".config read name=fd", ".config read type=pool name=x",
                     ".config read type=client", ".config create type=schedule", ".config create type=schedule text64=!!"] {
            assert!(!run(&h, &admin, line).success, "{line}");
        }
        let ok = run(&h, &admin, &format!(".config create type=schedule text64={} validate_only", b64(WEEKLY)));
        assert_eq!(ok.json.unwrap()["validated_only"], true);
    }

    #[test]
    fn text_mode_and_masked_audit_line() {
        let h = Arc::new(harness("dot-config-text"));
        let mut ctx = context(&h, "admin");
        ctx.api_mode = ApiMode::Human;
        let r = run(&h, &ctx, ".config read type=client name=fd");
        assert!(r.text_output.starts_with("status: OK\n") && r.text_output.contains("text:\nClient {"), "{}", r.text_output);
        let r = run(&h, &ctx, ".config read type=client name=fd json");
        assert!(r.text_output.starts_with('{') && r.text_output.contains("\"status\":\"OK\""), "{}", r.text_output);
        assert_eq!(mask_cmdline(".config create type=client text64=QUJD"), ".config create type=client text64=<4 bytes>");
    }

    #[test]
    fn the_command_is_registered_without_a_bareos_name_clash() {
        let registry = crate::command_executor::get_registry();
        assert_eq!(crate::command_registry::resolve_command_name(registry, ".config"), Some(".config"));
        assert!(!crate::ua_commands_table::UA_COMMANDS.iter().any(|e| e.0.starts_with(".config")));
        assert_eq!(crate::command_registry::resolve_command_name(registry, "configure"), Some("configure"));
    }
}
