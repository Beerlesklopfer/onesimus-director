//! MCP tool `configure_update` over the Director's configuration CRUD
//! (docs/plans/CONFIG-CRUD-GRPC.md).
//!
//! The Onesimus console extension `configure update` rewrites a Client file
//! with four fields and loses every other directive (K-10), so the tool no
//! longer uses it. Instead:
//!
//! 1. the capabilities of the connection decide (gRPC: `GetConfigCapabilities`;
//!    BNET: the dot command `.config capabilities`): the type must offer
//!    READ_RAW and UPDATE_RAW, otherwise the tool refuses (bareos-dir, an
//!    older odir);
//! 2. ReadResource gives the raw text and its sha256;
//! 3. only the given directives are replaced in the raw text
//!    ([`replace_directives`]); comments, indentation, line ends and every
//!    other byte stay;
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

/// One directive statement of the resource text.
#[derive(Debug, Clone)]
struct Stmt {
    norm: String,
    key_start: usize,
    value_start: usize,
    value_end: usize,
    block: bool,
}

/// The directives of a resource: those of the resource itself, those inside
/// nested blocks (normalized key, block name) and the closing brace.
#[derive(Debug, Default)]
struct Scan {
    top: Vec<Stmt>,
    nested: Vec<(String, String)>,
    close: usize,
}

/// Directive names are compared like Bareos: case and blanks do not matter.
pub fn norm_key(key: &str) -> String {
    key.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

fn skip_line(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// Parses the statements of a block from `i` (after its `{`) to its closing
/// brace; returns the index of that brace.
fn parse_block(text: &str, mut i: usize, top: bool, parent: &str, scan: &mut Scan) -> Result<usize, String> {
    let b = text.as_bytes();
    loop {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | b';') {
            i += 1;
        }
        if i >= b.len() {
            return Err("the resource text has unbalanced braces".into());
        }
        match b[i] {
            b'#' => {
                i = skip_line(b, i);
                continue;
            }
            b'}' => return Ok(i),
            _ => {}
        }
        let key_start = i;
        while i < b.len() && !matches!(b[i], b'=' | b'{' | b'\n' | b'#' | b';' | b'}') {
            i += 1;
        }
        let key = text[key_start..i].trim().to_string();
        if i >= b.len() {
            return Err("the resource text has unbalanced braces".into());
        }
        let mut stmt = Stmt { norm: norm_key(&key), key_start, value_start: i, value_end: i, block: false };
        if b[i] == b'{' {
            stmt.block = true;
            let close = parse_block(text, i + 1, false, &key, scan)?;
            record(scan, top, parent, stmt);
            i = close + 1;
            continue;
        }
        if b[i] != b'=' {
            // a word without `=`: leave it to the Director's check
            continue;
        }
        i += 1;
        while i < b.len() && matches!(b[i], b' ' | b'\t') {
            i += 1;
        }
        stmt.value_start = i;
        let mut last = i;
        let mut quoted = false;
        let mut block_at = None;
        while i < b.len() {
            let c = b[i];
            if quoted {
                if c == b'\\' {
                    i += 2;
                    last = i.min(b.len());
                    continue;
                }
                if c == b'"' {
                    quoted = false;
                }
                i += 1;
                last = i;
                continue;
            }
            match c {
                b'"' => {
                    quoted = true;
                    i += 1;
                    last = i;
                }
                b'\n' | b'#' | b';' | b'}' => break,
                b'{' => {
                    block_at = Some(i);
                    break;
                }
                b' ' | b'\t' | b'\r' => i += 1,
                _ => {
                    i += 1;
                    last = i;
                }
            }
        }
        if quoted {
            return Err("the resource text has an unterminated quoted string".into());
        }
        stmt.value_end = last.min(b.len());
        if let Some(open) = block_at {
            stmt.block = true;
            let close = parse_block(text, open + 1, false, &key, scan)?;
            record(scan, top, parent, stmt);
            i = close + 1;
            continue;
        }
        record(scan, top, parent, stmt);
    }
}

fn record(scan: &mut Scan, top: bool, parent: &str, stmt: Stmt) {
    if top {
        scan.top.push(stmt);
    } else {
        scan.nested.push((stmt.norm, parent.to_string()));
    }
}

fn scan(text: &str) -> Result<Scan, String> {
    let b = text.as_bytes();
    let mut i = 0;
    // comments and blanks before the type word
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'#' {
            i = skip_line(b, i);
        } else {
            break;
        }
    }
    while i < b.len() && b[i] != b'{' {
        if matches!(b[i], b'}' | b'=' | b'#' | b';') {
            return Err("the text is not a resource (type word and {)".into());
        }
        i += 1;
    }
    if i >= b.len() {
        return Err("the text is not a resource (type word and {)".into());
    }
    let mut scan = Scan::default();
    scan.close = parse_block(text, i + 1, true, "", &mut scan)?;
    Ok(scan)
}

/// One changed directive.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub directive: String,
    /// Old value as written (None: the directive was added).
    pub old: Option<String>,
    pub new: String,
}

/// Leading blanks of the line that holds `at`.
fn indentation(text: &str, at: usize) -> &str {
    let start = text[..at].rfind('\n').map_or(0, |p| p + 1);
    let line = &text[start..];
    let end = line.find(|c: char| c != ' ' && c != '\t').unwrap_or(line.len());
    &line[..end]
}

/// Replaces the values of `changes` (directive, formatted value) in the raw
/// text of one resource:
/// - names compare like Bareos (case and blanks do not matter);
/// - the last occurrence among the resource's own directives gets the new
///   value; its key, indentation, comment and line end stay;
/// - a missing directive is added as a line before the closing brace of the
///   resource, indented like the resource's last directive and with the
///   file's line end;
/// - directives of nested blocks (RunScript, Include …) are not touched:
///   a change of a block or of a directive that only occurs inside one is
///   refused.
pub fn replace_directives(text: &str, changes: &[(String, String)]) -> Result<(String, Vec<Change>), String> {
    let scan = scan(text)?;
    let mut seen = Vec::new();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut added: Vec<(String, String)> = Vec::new();
    let mut report = Vec::new();
    for (key, value) in changes {
        let norm = norm_key(key);
        if seen.contains(&norm) {
            return Err(format!("directive '{key}' is given twice"));
        }
        seen.push(norm.clone());
        match scan.top.iter().rev().find(|s| s.norm == norm) {
            Some(s) if s.block => {
                return Err(format!("'{key}' is a nested block; configure_update does not change nested blocks \
                    (edit the raw text instead)"));
            }
            Some(s) => {
                report.push(Change { directive: key.clone(), old: Some(text[s.value_start..s.value_end].to_string()),
                    new: value.clone() });
                edits.push((s.value_start, s.value_end, value.clone()));
            }
            None => {
                if let Some((_, parent)) = scan.nested.iter().find(|(n, _)| *n == norm) {
                    return Err(format!("directive '{key}' only occurs inside the nested block '{parent}'; \
                        configure_update does not change nested blocks (edit the raw text instead)"));
                }
                report.push(Change { directive: key.clone(), old: None, new: value.clone() });
                added.push((key.clone(), value.clone()));
            }
        }
    }
    if !added.is_empty() {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let indent = scan.top.last().map(|s| indentation(text, s.key_start).to_string())
            .filter(|i| !i.is_empty()).unwrap_or_else(|| "  ".into());
        let line_start = text[..scan.close].rfind('\n').map_or(0, |p| p + 1);
        if text[line_start..scan.close].trim().is_empty() {
            let lines: String = added.iter().map(|(k, v)| format!("{indent}{k} = {v}{eol}")).collect();
            edits.push((line_start, line_start, lines));
        } else {
            // closing brace on a line with content (one-line resource)
            let at = text[..scan.close].trim_end().len();
            let sep = if text[..at].ends_with('{') { " " } else { "; " };
            let inline: Vec<String> = added.iter().map(|(k, v)| format!("{k} = {v}")).collect();
            edits.push((at, at, format!("{sep}{}", inline.join("; "))));
        }
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = text.to_string();
    for (start, end, new) in edits {
        out.replace_range(start..end, &new);
    }
    Ok((out, report))
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

/// Checked directive arguments: (name as given, formatted value).
pub fn directives(arguments: &Value) -> Result<Vec<(String, String)>, String> {
    let Some(Value::Object(map)) = arguments.get("directives") else {
        return Err("directives must be a non-empty object".into());
    };
    if map.is_empty() {
        return Err("directives must be a non-empty object".into());
    }
    let mut out = Vec::new();
    for (key, value) in map {
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') {
            return Err(format!("directive '{key}': letters, digits and blanks only"));
        }
        if norm_key(key) == "name" {
            return Err("the Name cannot be changed (renaming is not supported)".into());
        }
        out.push((key.to_string(), format_value(key, value)?));
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
pub type Checked = (String, String, Vec<(String, String)>);

pub fn check_arguments(arguments: &Value) -> Result<Checked, String> {
    let Some(map) = arguments.as_object() else { return Err("arguments must be an object".into()) };
    if let Some(key) = map.keys().find(|k| !matches!(k.as_str(), "resource" | "name" | "directives")) {
        return Err(format!("unknown argument '{key}'"));
    }
    let resource = map.get("resource").and_then(Value::as_str).ok_or("resource is required")?.to_ascii_lowercase();
    let name = map.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or("name is required")?;
    if name.chars().any(|c| c.is_control() || c == '"') {
        return Err("name: control characters and quotes are not allowed".into());
    }
    Ok((resource, name.to_string(), directives(arguments)?))
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
        which keeps every other directive and comment; it no longer uses 'configure update', which rewrites the \
        resource file and loses directives (K-10). Change the file by hand or use an odir that offers {resource} \
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
        match &c.old {
            Some(old) => text.push_str(&format!("  {}: {old} -> {}\n", c.directive, c.new)),
            None => text.push_str(&format!("  {}: (added) {}\n", c.directive, c.new)),
        }
    }
    text.push_str(&format!("old sha256 {old}\nnew sha256 {new}\nreloaded: {}\n", if reloaded { "yes" } else { "no" }));
    let changed: Vec<Value> = report.iter().map(|c| {
        let mut m = Map::new();
        m.insert("directive".into(), json!(c.directive));
        m.insert("old".into(), json!(c.old));
        m.insert("new".into(), json!(c.new));
        m.insert("added".into(), json!(c.old.is_none()));
        Value::Object(m)
    }).collect();
    json!({ "content": [{ "type": "text", "text": text }], "isError": false,
        "structuredContent": { "resource": resource, "name": name, "route": match route { Route::Grpc => "grpc", Route::Bnet => "bnet" },
            "changed": changed, "old_sha256": old, "new_sha256": new, "reloaded": reloaded, "file": file } })
}

#[cfg(test)]
mod tests;
