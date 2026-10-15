//! JSON API mode (`.api 2`) output — the json-rpc style envelope of the
//! C++ OutputFormatter (lib/output_formatter.cc, JsonFinalizeResult):
//!
//! success:  {"jsonrpc": "2.0", "id": null, "result": {...}}
//! failure:  {"jsonrpc": "2.0", "id": null,
//!            "error": {"code": 1, "message": "failed",
//!                      "data": {"result": {...}, "messages": {"error": [...]}}}}
//!
//! Like C++, plain text output is not part of the JSON result; only error
//! messages are carried along. Pretty printing uses 2-space indentation
//! (UA_JSON_FLAGS_NORMAL = JSON_INDENT(2)).

use serde_json::{json, Map, Value};

use crate::command_queue::CommandResult;

pub fn envelope(result: &CommandResult) -> String {
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
    serde_json::to_string_pretty(&msg).unwrap_or_else(|_| {
        "{ \"jsonrpc\": \"2.0\", \"id\": null, \"error\": { \"message\": \"failed to generate json\", \"code\": 1 } }\n".to_string()
    })
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
        let v: Value = serde_json::from_str(&envelope(&r)).unwrap();
        assert_eq!(v, json!({"jsonrpc": "2.0", "id": null, "result": {"api": 2}}));
    }

    #[test]
    fn text_only_command_yields_empty_result() {
        let r = CommandResult::ok("some text\n".into());
        let v: Value = serde_json::from_str(&envelope(&r)).unwrap();
        assert_eq!(v["result"], json!({}));
    }

    #[test]
    fn error_envelope_carries_messages() {
        let r = CommandResult::error("foo: is an invalid command.\n".into());
        let v: Value = serde_json::from_str(&envelope(&r)).unwrap();
        assert_eq!(v["error"]["code"], 1);
        assert_eq!(v["error"]["message"], "failed");
        assert_eq!(v["error"]["data"]["messages"]["error"][0], "foo: is an invalid command.\n");
        assert!(v.get("result").is_none());
    }

    #[test]
    fn errors_of_successful_command_fail_and_failure_without_errors_has_empty_messages() {
        let r = CommandResult::ok_with_errors("No Jobs running.\n".into(), "JobId 1 is not running.\n".into());
        let v: Value = serde_json::from_str(&envelope(&r)).unwrap();
        assert_eq!(v["error"]["data"]["messages"], json!({"error": ["JobId 1 is not running.\n"]}));
        let r = CommandResult { success: false, text_output: "usage\n".into(), ..Default::default() };
        let v: Value = serde_json::from_str(&envelope(&r)).unwrap();
        assert_eq!(v["error"]["data"]["messages"], json!({}));
    }

    #[test]
    fn key_order_matches_cpp() {
        let r = CommandResult::ok(String::new());
        let s = envelope(&r);
        let (a, b, c) = (s.find("jsonrpc").unwrap(), s.find("\"id\"").unwrap(), s.find("result").unwrap());
        assert!(a < b && b < c);
    }
}
