//! MCP tool `configure_update` over the Director's configuration CRUD
//! (docs/plans/CONFIG-CRUD-GRPC.md).
//!
//! The Onesimus console extension `configure update` of older odir versions
//! rewrote a Client file with four fields and lost every other directive
//! (K-10), so the tool never uses the console command; it talks to the CRUD
//! itself (current odir: `configure update` uses the same CRUD and the same
//! text editing, `bareos_config::edit`):
//!
//! 1. the capabilities of the connection decide (gRPC: `GetConfigCapabilities`;
//!    BNET: the dot command `.config capabilities`): the type must offer
//!    READ_RAW and UPDATE_RAW, otherwise the tool refuses (bareos-dir, an
//!    older odir);
//! 2. ReadResource gives the raw text and its sha256;
//! 3. only the given directives are replaced in the raw text
//!    ([`replace_directives`]; a list value replaces every line of a
//!    repeated directive such as Schedule `Run`); comments, indentation,
//!    line ends and every other byte stay;
//! 4. UpdateResource with `expected_sha256`; on CONFLICT the tool reads once
//!    more and tries again, then it returns the error. INVALID comes back
//!    with file, line and the text of the check.
//!
//! Over gRPC the typed RPCs are used, over BNET `.config read/update` (text
//! as base64 in `text64=`). The Director checks the changed configuration
//! like `odir -t` and reloads.

use base64::Engine;
use serde_json::{Map, Value, json};

use crate::mcp::{CommandRunner, TypedError};
use onesimus_proto_rs::typed;

// ---------------------------------------------------------------------------
// Replacing directives in the raw text
// ---------------------------------------------------------------------------

/// Directive names are compared like Bareos: case, blanks and underscores
/// do not matter.
pub use bareos_config::edit::norm_key;

/// One changed directive (old and new values as written; old empty: the
/// directive was added).
pub type Change = bareos_config::edit::Change;

/// Replaces the values of `changes` (directive, formatted values) in the raw
/// text of one resource ([`bareos_config::edit::apply`]):
/// - names compare like Bareos (case, blanks and underscores do not matter);
/// - every existing line of the directive is replaced by the given values,
///   at the place of the first one; its key, indentation, comment and line
///   end stay;
/// - a missing directive is added before the closing brace of the resource,
///   indented like the resource's directives and with the file's line end;
/// - directives of nested blocks (RunScript, Include …) are not touched:
///   a change of a block or of a directive that only occurs inside one is
///   refused.
pub fn replace_directives(text: &str, changes: &[(String, Vec<String>)]) -> Result<(String, Vec<Change>), String> {
    let edits: Vec<bareos_config::edit::Edit> = changes.iter()
        .map(|(key, values)| bareos_config::edit::Edit { key: key.clone(), values: values.clone() }).collect();
    bareos_config::edit::apply(text, &edits).map_err(|e| e.replace("(edit the raw text instead)",
        "(configure_update does not change nested blocks; edit the raw text instead)"))
}

/// The value as written in the configuration: booleans as yes/no, values
/// with blanks or special characters in quotes (backslash before `"` and
/// `\`).
pub fn format_value(key: &str, value: &Value) -> Result<String, String> {
    let text = match value {
        Value::Bool(b) => return Ok(if *b { "yes" } else { "no" }.into()),
        Value::Number(n) => return Ok(n.to_string()),
        Value::String(s) => s.clone(),
        _ => return Err(format!("directive '{key}': string, number or boolean")),
    };
    if text.chars().any(char::is_control) {
        return Err(format!("directive '{key}': control characters (line ends) are not allowed"));
    }
    match text.to_ascii_lowercase().as_str() {
        "yes" | "true" => return Ok("yes".into()),
        "no" | "false" => return Ok("no".into()),
        _ => {}
    }
    let plain = !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || "._:/-+".contains(c));
    if plain {
        Ok(text)
    } else {
        Ok(format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"")))
    }
}

/// The resource types the tool changes: those of the Director's
/// configuration CRUD.
pub const TYPES: &[&str] = &["client", "job", "jobdefs", "schedule", "fileset", "pool"];

/// Checked directive arguments of a resource of type `resource`: (name as
/// given, formatted values). A list value is a repeated directive (Schedule
/// `Run`): every existing line is replaced by one line per element. The
/// Schedule `Run` directive is written unquoted
/// ([`bareos_config::edit::format_value`]).
pub fn directives(arguments: &Value, resource: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let Some(Value::Object(map)) = arguments.get("directives") else {
        return Err("directives must be a non-empty object".into());
    };
    if map.is_empty() {
        return Err("directives must be a non-empty object".into());
    }
    let mut out = Vec::new();
    for (key, value) in map {
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_') {
            return Err(format!("directive '{key}': letters, digits, blanks and underscores only"));
        }
        if norm_key(key) == "name" {
            return Err("the Name cannot be changed (renaming is not supported)".into());
        }
        let items = match value {
            // null removes every line of the directive
            Value::Null => {
                out.push((key.to_string(), Vec::new()));
                continue;
            }
            Value::Array(items) if items.is_empty() => return Err(format!("directive '{key}': empty list")),
            Value::Array(items) => items.clone(),
            v => vec![v.clone()],
        };
        let mut values = Vec::new();
        for v in &items {
            if bareos_config::edit::raw_directive(resource, key) {
                let text = v.as_str().ok_or_else(|| format!("directive '{key}': strings only"))?;
                values.push(bareos_config::edit::format_value(resource, key, text)?);
            } else {
                values.push(format_value(key, v)?);
            }
        }
        out.push((key.to_string(), values));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// The Director's CRUD over gRPC (typed RPCs) or BNET (`.config`)
// ---------------------------------------------------------------------------

/// The answer of ReadResource / UpdateResource.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resp {
    pub status: String,
    pub message: String,
    pub file: String,
    pub text: String,
    pub sha256: String,
    pub old_sha256: String,
    pub reloaded: bool,
    /// (file, line, column, text)
    pub problem: Option<(String, u64, u64, String)>,
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Enum value name without its prefix (gRPC responses carry numbers).
fn enum_name(enum_type: &str, prefix: &str, value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.trim_start_matches(prefix).to_string();
    }
    let number = value.as_i64().unwrap_or(0) as i32;
    typed::registry().enumeration(&format!("onesimus.director.api.{enum_type}"))
        .and_then(|e| e.value.iter().find(|v| v.number() == number))
        .map(|v| v.name().trim_start_matches(prefix).to_string())
        .unwrap_or_else(|| "UNSPECIFIED".into())
}

impl Resp {
    fn from_json(v: &Value) -> Resp {
        let problem = v.get("problem").filter(|p| p.is_object()).map(|p| (s(p, "file"),
            p.get("line").and_then(Value::as_u64).unwrap_or(0), p.get("column").and_then(Value::as_u64).unwrap_or(0),
            s(p, "text")));
        Resp {
            status: enum_name("ConfigStatus", "CONFIG_STATUS_", v.get("status").unwrap_or(&Value::Null)),
            message: s(v, "message"),
            file: s(v, "file"),
            text: s(v, "text"),
            sha256: s(v, "sha256"),
            old_sha256: s(v, "old_sha256"),
            reloaded: v.get("reloaded").and_then(Value::as_bool).unwrap_or(false),
            problem,
        }
    }
}

/// Which way the connection talks to the Director.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Route {
    Grpc,
    Bnet,
}

struct Api<'a, R: CommandRunner + ?Sized> {
    runner: &'a mut R,
    route: Route,
}

impl<'a, R: CommandRunner + ?Sized> Api<'a, R> {
    fn new(runner: &'a mut R) -> Self {
        let route = if runner.is_grpc() { Route::Grpc } else { Route::Bnet };
        Self { runner, route }
    }

    fn typed(&mut self, rpc: &str, request: Value) -> Result<Value, String> {
        let bytes = crate::mcp_typed::encode(rpc, &request)?;
        match self.runner.typed_call(rpc, bytes) {
            None | Some(Err(TypedError::Unimplemented)) => Err(format!("the Director does not implement {rpc}")),
            Some(Err(TypedError::Status(code, message))) => Err(format!("{rpc}: gRPC {code}: {}", message.trim_end())),
            Some(Ok(reply)) => Ok(crate::mcp_typed::decode(rpc, &reply)),
        }
    }

    fn dot(&mut self, line: &str) -> Result<Value, String> {
        let out = self.runner.run_json(line).map_err(|e| format!("{}: {e}", line.split(" text64=").next().unwrap_or(line)))?;
        let v: Value = serde_json::from_str(out.trim()).map_err(|_| format!("the Director does not answer \
            '.config' (no Onesimus configuration CRUD): {}", out.trim()))?;
        if let Some(error) = v.get("error") {
            let message = error.get("data").and_then(|d| d.get("messages")).map(|m| m.to_string())
                .or_else(|| error.get("message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| error.to_string());
            return Err(format!("the Director refused '.config': {message}"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }

    /// (type, operations) the connection offers, names without prefix.
    fn capabilities(&mut self) -> Result<Vec<(String, Vec<String>)>, String> {
        let v = match self.route {
            Route::Grpc => self.typed("GetConfigCapabilities", json!({}))?,
            Route::Bnet => self.dot(".config capabilities")?,
        };
        Ok(v.get("types").and_then(Value::as_array).into_iter().flatten().map(|t| (
            enum_name("ResourceType", "RESOURCE_TYPE_", t.get("type").unwrap_or(&Value::Null)),
            t.get("operations").and_then(Value::as_array).into_iter().flatten()
                .map(|o| enum_name("ConfigOperation", "CONFIG_OPERATION_", o)).collect(),
        )).collect())
    }

    fn read(&mut self, rtype: &str, name: &str) -> Result<Resp, String> {
        let v = match self.route {
            Route::Grpc => self.typed("ReadResource", json!({ "resource": { "type": rtype, "name": name } }))?,
            Route::Bnet => self.dot(&format!(".config read type={} name={}", rtype.to_ascii_lowercase(), quote_arg(name)))?,
        };
        Ok(Resp::from_json(&v))
    }

    fn update(&mut self, rtype: &str, name: &str, sha: &str, text: &str) -> Result<Resp, String> {
        let v = match self.route {
            Route::Grpc => self.typed("UpdateResource", json!({ "resource": { "type": rtype, "name": name },
                "expected_sha256": sha, "text": text }))?,
            Route::Bnet => self.dot(&format!(".config update type={} name={} sha256={sha} text64={}",
                rtype.to_ascii_lowercase(), quote_arg(name), base64::engine::general_purpose::STANDARD.encode(text)))?,
        };
        Ok(Resp::from_json(&v))
    }
}

/// A resource name as console argument value.
fn quote_arg(name: &str) -> String {
    if name.contains(' ') { format!("\"{name}\"") } else { name.to_string() }
}

fn error(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// Arguments: resource, name, directives (checked like the tool schema).
/// (resource type, name, checked directives)
pub type Checked = (String, String, Vec<(String, Vec<String>)>);

pub fn check_arguments(arguments: &Value) -> Result<Checked, String> {
    let Some(map) = arguments.as_object() else { return Err("arguments must be an object".into()) };
    if let Some(key) = map.keys().find(|k| !matches!(k.as_str(), "resource" | "name" | "directives")) {
        return Err(format!("unknown argument '{key}'"));
    }
    let resource = map.get("resource").and_then(Value::as_str).ok_or("resource is required")?.to_ascii_lowercase();
    if !TYPES.contains(&resource.as_str()) {
        return Err(format!("resource: one of {} (the types of the configuration CRUD)", TYPES.join(", ")));
    }
    let name = map.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or("name is required")?;
    if name.chars().any(|c| c.is_control() || c == '"') {
        return Err("name: control characters and quotes are not allowed".into());
    }
    let directives = directives(arguments, &resource)?;
    Ok((resource, name.to_string(), directives))
}

/// The tool `configure_update`.
pub fn configure_update<R: CommandRunner + ?Sized>(runner: &mut R, arguments: &Value) -> Value {
    let (resource, name, changes) = match check_arguments(arguments) {
        Ok(v) => v,
        Err(e) => return error(e),
    };
    let rtype = resource.to_ascii_uppercase();
    let mut api = Api::new(runner);
    let route = api.route;
    let refuse = |why: String| error(format!("configure_update refused: {why}. The tool only changes resources \
        through the Director's configuration CRUD (ReadResource/UpdateResource over gRPC, '.config' over BNET), \
        which keeps every other directive and comment; it never uses the console command 'configure update', \
        which rewrote the resource file and lost directives in older odir versions (K-10). Change the file by hand or use an odir that offers {resource} \
        in GetConfigCapabilities."));
    let caps = match api.capabilities() {
        Ok(c) => c,
        Err(e) => return refuse(e),
    };
    let offered = caps.iter().find(|(t, _)| *t == rtype)
        .is_some_and(|(_, ops)| ops.iter().any(|o| o == "READ_RAW") && ops.iter().any(|o| o == "UPDATE_RAW"));
    if !offered {
        return refuse(format!("the Director does not offer READ_RAW and UPDATE_RAW for {resource} on this connection \
            (offered: {})", if caps.is_empty() { "nothing".to_string() } else {
                caps.iter().map(|(t, _)| t.to_ascii_lowercase()).collect::<Vec<_>>().join(", ") }));
    }
    let mut conflicts = 0;
    loop {
        let read = match api.read(&rtype, &name) {
            Ok(r) => r,
            Err(e) => return error(format!("ReadResource {resource} \"{name}\": {e}")),
        };
        if read.status != "OK" {
            return error(format!("ReadResource {resource} \"{name}\": {} {}", read.status, read.message));
        }
        let (text, report) = match replace_directives(&read.text, &changes) {
            Ok(v) => v,
            Err(e) => return error(format!("{resource} \"{name}\": {e}")),
        };
        if text == read.text {
            return summary(&resource, &name, route, &report, &read.sha256, &read.sha256, false, &read.file,
                "no change: the directives already have these values");
        }
        let updated = match api.update(&rtype, &name, &read.sha256, &text) {
            Ok(r) => r,
            Err(e) => return error(format!("UpdateResource {resource} \"{name}\": {e}")),
        };
        match updated.status.as_str() {
            "OK" => {
                let old = if updated.old_sha256.is_empty() { read.sha256.clone() } else { updated.old_sha256.clone() };
                return summary(&resource, &name, route, &report, &old, &updated.sha256, updated.reloaded,
                    &updated.file, &updated.message);
            }
            "CONFLICT" if conflicts == 0 => conflicts += 1,
            "CONFLICT" => return error(format!("UpdateResource {resource} \"{name}\": CONFLICT again after reading \
                once more (the resource keeps changing): {}", updated.message)),
            "INVALID" => {
                let mut text = format!("UpdateResource {resource} \"{name}\": INVALID {}", updated.message);
                if let Some((file, line, column, problem)) = &updated.problem {
                    text.push_str(&format!("\n{file}:{line}:{column}\n{problem}"));
                }
                return json!({ "content": [{ "type": "text", "text": text }], "isError": true,
                    "structuredContent": { "status": "INVALID", "message": updated.message,
                        "problem": updated.problem.as_ref().map(|(f, l, c, t)| json!({ "file": f, "line": l, "column": c, "text": t })) } });
            }
            other => return error(format!("UpdateResource {resource} \"{name}\": {other} {}", updated.message)),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn summary(resource: &str, name: &str, route: Route, report: &[Change], old: &str, new: &str, reloaded: bool,
           file: &str, message: &str) -> Value {
    let mut text = format!("{resource} \"{name}\"{}: {message}\n", if file.is_empty() { String::new() } else { format!(" ({file})") });
    for c in report {
        let old = c.old.join(" | ");
        let new = if c.new.is_empty() { "(removed)".to_string() } else { c.new.join(" | ") };
        if c.old.is_empty() {
            text.push_str(&format!("  {}: (added) {new}\n", c.directive));
        } else {
            text.push_str(&format!("  {}: {old} -> {new}\n", c.directive));
        }
    }
    text.push_str(&format!("old sha256 {old}\nnew sha256 {new}\nreloaded: {}\n", if reloaded { "yes" } else { "no" }));
    let changed: Vec<Value> = report.iter().map(|c| {
        let mut m = Map::new();
        m.insert("directive".into(), json!(c.directive));
        m.insert("old".into(), values_json(&c.old));
        m.insert("new".into(), values_json(&c.new));
        m.insert("added".into(), json!(c.old.is_empty()));
        Value::Object(m)
    }).collect();
    json!({ "content": [{ "type": "text", "text": text }], "isError": false,
        "structuredContent": { "resource": resource, "name": name, "route": match route { Route::Grpc => "grpc", Route::Bnet => "bnet" },
            "changed": changed, "old_sha256": old, "new_sha256": new, "reloaded": reloaded, "file": file } })
}

/// Values of a [`Change`] as JSON: null when there is none, a string for
/// one, a list for a repeated directive.
fn values_json(values: &[String]) -> Value {
    match values {
        [] => Value::Null,
        [one] => json!(one),
        many => json!(many),
    }
}

#[cfg(test)]
mod tests;
