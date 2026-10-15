//! JSON API mode (`.api 2`) output — the json-rpc style envelope of the
//! C++ OutputFormatter (lib/output_formatter.cc, JsonFinalizeResult):
//!
//! success:  {"jsonrpc": "2.0", "id": null, "result": {...}}
//! failure:  {"jsonrpc": "2.0", "id": null,
//!            "error": {"code": 1, "message": "failed",
//!                      "data": {"result": {...}, "messages": {"error": [...]}}}}
//!
//! Like C++, plain text output is not part of the JSON result; only error
//! messages are carried along. The text is produced like jansson's
//! json_dumps(): UA_JSON_FLAGS_NORMAL = JSON_INDENT(2), or JSON_COMPACT after
//! `.api json compact=yes` (OutputFormatter::SetCompact, kept for the whole
//! session); no trailing newline in either mode.

use serde_json::{json, Map, Value};

use crate::command_queue::CommandResult;

pub fn envelope(result: &CommandResult, compact: bool) -> String {
    let result_obj = result.json.clone().unwrap_or_else(|| Value::Object(Map::new()));

    // JsonFinalizeResult: failed when the command returned false or any
    // ErrorMsg was emitted. Only error messages are collected; SendMsg text
    // is dropped, so a failure without ErrorMsg carries "messages": {}.
    let errors = result.error_lines();
    let msg = if result.success && errors.is_empty() {
        json!({ "jsonrpc": "2.0", "id": null, "result": result_obj })
    } else {
        let mut messages = Map::new();
        if !errors.is_empty() {
            messages.insert("error".into(), Value::Array(errors.into_iter().map(Value::String).collect()));
        }
        json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": 1,
                "message": "failed",
                "data": {
                    "result": result_obj,
                    "messages": messages,
                },
            },
        })
    };
    dumps(&msg, compact)
}

/// jansson json_dumps() with JSON_INDENT(2) (`compact == false`) or
/// JSON_COMPACT: insertion key order, no JSON_ESCAPE_SLASH, no
/// JSON_ENSURE_ASCII, no trailing newline.
pub fn dumps(value: &Value, compact: bool) -> String {
    let mut out = String::new();
    dump_value(value, compact, 0, &mut out);
    out
}

/// jansson dump.c dump_indent(): newline plus indentation with JSON_INDENT;
/// JSON_COMPACT has neither indentation nor separating spaces.
fn dump_indent(compact: bool, depth: usize, out: &mut String) {
    if !compact {
        out.push('\n');
        out.extend(std::iter::repeat(' ').take(depth * 2));
    }
}

fn dump_value(value: &Value, compact: bool, depth: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(i), _, _) => out.push_str(&i.to_string()),
            (None, Some(u), _) => out.push_str(&u.to_string()),
            (_, _, Some(f)) => out.push_str(&dtostr(f)),
            _ => out.push_str(&n.to_string()),
        },
        Value::String(s) => dump_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                dump_indent(compact, depth + 1, out);
                dump_value(item, compact, depth + 1, out);
                if i + 1 < items.len() { out.push(','); } else { dump_indent(compact, depth, out); }
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                dump_indent(compact, depth + 1, out);
                dump_string(key, out);
                out.push_str(if compact { ":" } else { ": " });
                dump_value(item, compact, depth + 1, out);
                if i + 1 < map.len() { out.push(','); } else { dump_indent(compact, depth, out); }
            }
            out.push('}');
        }
    }
}

/// jansson dump.c dump_string(): only '"', '\\' and control characters are
/// escaped ('/' and non-ASCII stay literal); control characters without a
/// short form as `\u%04X` with upper-case hex digits.
fn dump_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// jansson strconv.c jsonp_dtostr(): printf "%.17g", ".0" appended to
/// integral values, '+' and leading zeros removed from the exponent.
fn dtostr(value: f64) -> String {
    const PRECISION: i32 = 17;
    let sci = format!("{:.*e}", (PRECISION - 1) as usize, value);
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    // %g drops trailing zeros of the fraction (no '#' flag).
    let trim = |s: &str| if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() };
    let mut buf = if exp < -4 || exp >= PRECISION {
        format!("{}e{}", trim(mantissa), exp)
    } else {
        trim(&format!("{:.*}", (PRECISION - 1 - exp) as usize, value))
    };
    if buf.chars().all(|c| c.is_ascii_digit() || c == '-') {
        buf.push_str(".0");
    }
    buf
}

/// Render a value the way Bareos prints catalog rows: every column as string,
/// NULL as empty string (C++ list commands use ObjectKeyValue with "%s").
pub fn db_str(v: Option<impl ToString>) -> Value {
    Value::String(v.map(|x| x.to_string()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_envelope() {
        let r = CommandResult::ok("ignored text\n".into()).with_json(json!({"api": 2}));
        let v: Value = serde_json::from_str(&envelope(&r, false)).unwrap();
        assert_eq!(v, json!({"jsonrpc": "2.0", "id": null, "result": {"api": 2}}));
    }

    #[test]
    fn text_only_command_yields_empty_result() {
        let r = CommandResult::ok("some text\n".into());
        let v: Value = serde_json::from_str(&envelope(&r, false)).unwrap();
        assert_eq!(v["result"], json!({}));
    }

    #[test]
    fn error_envelope_carries_messages() {
        let r = CommandResult::error("foo: is an invalid command.\n".into());
        let v: Value = serde_json::from_str(&envelope(&r, false)).unwrap();
        assert_eq!(v["error"]["code"], 1);
        assert_eq!(v["error"]["message"], "failed");
        assert_eq!(v["error"]["data"]["messages"]["error"][0], "foo: is an invalid command.\n");
        assert!(v.get("result").is_none());
    }

    #[test]
    fn errors_of_successful_command_fail_and_failure_without_errors_has_empty_messages() {
        let r = CommandResult::ok_with_errors("No Jobs running.\n".into(), "JobId 1 is not running.\n".into());
        let v: Value = serde_json::from_str(&envelope(&r, false)).unwrap();
        assert_eq!(v["error"]["data"]["messages"], json!({"error": ["JobId 1 is not running.\n"]}));
        let r = CommandResult { success: false, text_output: "usage\n".into(), ..Default::default() };
        let v: Value = serde_json::from_str(&envelope(&r, false)).unwrap();
        assert_eq!(v["error"]["data"]["messages"], json!({}));
    }

    #[test]
    fn compact_and_indented_layout_match_jansson() {
        let r = CommandResult::ok(String::new()).with_json(json!({"api": 2}));
        assert_eq!(envelope(&r, true), r#"{"jsonrpc":"2.0","id":null,"result":{"api":2}}"#);
        assert_eq!(envelope(&r, false),
            "{\n  \"jsonrpc\": \"2.0\",\n  \"id\": null,\n  \"result\": {\n    \"api\": 2\n  }\n}");
        let v = json!({"a": [], "b": {}, "c": [1, true, null], "d": -7});
        assert_eq!(dumps(&v, true), r#"{"a":[],"b":{},"c":[1,true,null],"d":-7}"#);
        assert_eq!(dumps(&v, false),
            "{\n  \"a\": [],\n  \"b\": {},\n  \"c\": [\n    1,\n    true,\n    null\n  ],\n  \"d\": -7\n}");
        assert_eq!(dumps(&json!([]), false), "[]");
    }

    #[test]
    fn string_escapes_match_jansson() {
        let v = json!("q\"b\\s/\u{1}\u{1f}\u{7f}\u{8}\u{c}\n\r\täöü€😀");
        assert_eq!(dumps(&v, true), "\"q\\\"b\\\\s/\\u0001\\u001F\u{7f}\\b\\f\\n\\r\\täöü€😀\"");
    }

    #[test]
    fn reals_match_jansson_dtostr() {
        for (f, s) in [(1.0, "1.0"), (0.1, "0.10000000000000001"), (-2.5, "-2.5"), (1e20, "1e20"),
            (1e-5, "1.0000000000000001e-5"), (123456789012345680.0, "1.2345678901234568e17"),
            (0.0001, "0.0001"), (-0.0, "-0.0")] {
            assert_eq!(dumps(&json!(f), true), s, "{f}");
        }
    }

    #[test]
    fn key_order_matches_cpp() {
        let r = CommandResult::ok(String::new());
        let s = envelope(&r, false);
        let (a, b, c) = (s.find("jsonrpc").unwrap(), s.find("\"id\"").unwrap(), s.find("result").unwrap());
        assert!(a < b && b < c);
    }
}
