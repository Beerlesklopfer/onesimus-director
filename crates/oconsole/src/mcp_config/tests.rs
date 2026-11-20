use super::*;
use sha2::{Digest, Sha256};

/// One value per directive.
fn ch(pairs: &[(&str, &str)]) -> Vec<(String, Vec<String>)> {
    pairs.iter().map(|(k, v)| (k.to_string(), vec![v.to_string()])).collect()
}

const CLIENT: &str = "Client {\n  # laptop\n  Name = \"laptop-fd\"   # name\n  Address = old.example   ; Port = 9102\n  \
    Password = \"[md5]0123\"\n  TLS Enable = yes\n  File Retention = 30 days  # keep\n  FileRetention = 40 days\n}\n";

#[test]
fn replaces_every_occurrence_at_the_first_and_keeps_everything_else() {
    let (out, report) = replace_directives(CLIENT, &ch(&[("file_retention", "\"60 days\""), ("ADDRESS", "new.example")])).unwrap();
    assert_eq!(out, CLIENT.replace("File Retention = 30 days  # keep\n  FileRetention = 40 days\n", "File Retention = \"60 days\"  # keep\n")
        .replace("Address = old.example   ;", "Address = new.example   ;"));
    assert_eq!(report[0], Change { directive: "file_retention".into(), old: vec!["30 days".into(), "40 days".into()],
        new: vec!["\"60 days\"".into()] });
    assert_eq!(report[1].old, ["old.example"]);
    // the comment after a replaced value stays
    let (out, _) = replace_directives(CLIENT, &ch(&[("Password", "x")])).unwrap();
    assert!(out.contains("  Password = x\n"));
    let text = "Client {\n\tName = a\n\tAddress = h  # the host\n}\n";
    let (out, _) = replace_directives(text, &ch(&[("address", "k")])).unwrap();
    assert_eq!(out, "Client {\n\tName = a\n\tAddress = k  # the host\n}\n");
    // second statement after `;` on the same line
    let (out, _) = replace_directives(CLIENT, &ch(&[("port", "9103")])).unwrap();
    assert!(out.contains("Address = old.example   ; Port = 9103\n"), "{out}");
}

#[test]
fn missing_directive_is_added_before_the_closing_brace_in_the_file_style() {
    let (out, report) = replace_directives(CLIENT, &ch(&[("Heartbeat Interval", "60")])).unwrap();
    assert_eq!(out, CLIENT.replace("}\n", "  Heartbeat Interval = 60\n}\n"));
    assert!(report[0].old.is_empty());
    // tab indentation and CRLF
    let crlf = "Client {\r\n\tName = a\r\n\tAddress = h\r\n}\r\n";
    let (out, _) = replace_directives(crlf, &ch(&[("Password", "p"), ("address", "k")])).unwrap();
    assert_eq!(out, "Client {\r\n\tName = a\r\n\tAddress = k\r\n\tPassword = p\r\n}\r\n");
    // one-line resource
    let (out, _) = replace_directives("Client { Name = a; Address = h }\n", &ch(&[("Password", "p")])).unwrap();
    assert_eq!(out, "Client { Name = a; Address = h; Password = p }\n");
    // the brace with a trailing comment
    let (out, _) = replace_directives("Client {\n  Name = a\n} # end\n", &ch(&[("Password", "p")])).unwrap();
    assert_eq!(out, "Client {\n  Name = a\n  Password = p\n} # end\n");
}

#[test]
fn crlf_values_keep_their_line_end() {
    let crlf = CLIENT.replace('\n', "\r\n");
    let (out, _) = replace_directives(&crlf, &ch(&[("tls enable", "no")])).unwrap();
    assert_eq!(out, crlf.replace("TLS Enable = yes\r\n", "TLS Enable = no\r\n"));
}

#[test]
fn quoted_values_with_braces_and_comment_signs_are_one_value() {
    let text = "Job {\n  Name = j\n  Description = \"a } b # c \\\" d\"  # real comment\n  Client = fd\n}\n";
    let (out, report) = replace_directives(text, &ch(&[("description", "\"new\""), ("client", "fd2")])).unwrap();
    assert_eq!(out, "Job {\n  Name = j\n  Description = \"new\"  # real comment\n  Client = fd2\n}\n");
    assert_eq!(report[0].old, ["\"a } b # c \\\" d\""]);
}

#[test]
fn nested_blocks_are_not_touched() {
    let text = "Job {\n  Name = j\n  RunScript {\n    Command = \"/bin/true\"\n    RunsWhen = Before\n  }\n  Priority = 10\n}\n";
    let err = replace_directives(text, &ch(&[("Command", "x")])).unwrap_err();
    assert!(err.contains("nested block 'RunScript'"), "{err}");
    let err = replace_directives(text, &ch(&[("Run Script", "x")])).unwrap_err();
    assert!(err.contains("nested block"), "{err}");
    // a top-level directive next to the block changes, the block stays byte for byte
    let (out, _) = replace_directives(text, &ch(&[("priority", "11")])).unwrap();
    assert_eq!(out, text.replace("Priority = 10", "Priority = 11"));
    // a missing directive goes after the block, before the resource's brace
    let (out, _) = replace_directives(text, &ch(&[("Maximum Concurrent Jobs", "2")])).unwrap();
    assert!(out.ends_with("  Priority = 10\n  Maximum Concurrent Jobs = 2\n}\n"), "{out}");
    assert!(replace_directives("Client {\n  Name = a\n", &ch(&[("a", "b")])).is_err());
    assert!(replace_directives(CLIENT, &ch(&[("address", "a"), ("Address", "b")])).is_err());
}

#[test]
fn values_are_formatted_like_the_configuration() {
    let f = |v: Value| format_value("k", &v).unwrap();
    assert_eq!(f(json!(true)), "yes");
    assert_eq!(f(json!(false)), "no");
    assert_eq!(f(json!("True")), "yes");
    assert_eq!(f(json!("NO")), "no");
    assert_eq!(f(json!(9102)), "9102");
    assert_eq!(f(json!("host.example")), "host.example");
    assert_eq!(f(json!("30 days")), "\"30 days\"");
    assert_eq!(f(json!("[md5]x")), "\"[md5]x\"");
    assert_eq!(f(json!("a\"b\\c")), "\"a\\\"b\\\\c\"");
    assert_eq!(f(json!("x;@include y")), "\"x;@include y\"");
    assert_eq!(f(json!("")), "\"\"");
    assert!(format_value("k", &json!("a\nb")).is_err());
    assert!(format_value("k", &json!([1])).is_err());
    assert!(directives(&json!({"directives": {"name": "x"}}), "client").is_err());
    assert!(directives(&json!({"directives": {"N_ame": "x"}}), "client").is_err());
    assert!(directives(&json!({"directives": {"a=b": "x"}}), "client").is_err());
    assert!(directives(&json!({"directives": {}}), "client").is_err());
    assert!(directives(&json!({"directives": {"run": []}}), "schedule").is_err());
    assert_eq!(directives(&json!({"directives": {"File Retention": "60 days"}}), "client").unwrap(),
               ch(&[("File Retention", "\"60 days\"")]));
    assert_eq!(directives(&json!({"directives": {"Description": "Weekly"}}), "schedule").unwrap(), ch(&[("Description", "Weekly")]));
    // Schedule Run: unquoted, a list replaces every Run line
    assert_eq!(directives(&json!({"directives": {"run": ["Full sun at 23:30", "Incremental mon-sat at 23:30"]}}), "schedule").unwrap(),
        vec![("run".to_string(), vec!["Full sun at 23:30".to_string(), "Incremental mon-sat at 23:30".to_string()])]);
    assert!(directives(&json!({"directives": {"run": "Full; x"}}), "schedule").is_err());
}

// ---------------------------------------------------------------------------
// The tool against a fake Director (gRPC typed RPCs or BNET `.config`)

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

struct FakeDirector {
    grpc: bool,
    /// None: no configuration CRUD (bareos-dir, older odir)
    crud: bool,
    text: String,
    /// changed behind the tool's back before the next n updates
    interfere: usize,
    /// INVALID for every update
    invalid: bool,
    updates: Vec<String>,
    lines: Vec<String>,
}

impl FakeDirector {
    fn new(grpc: bool) -> Self {
        Self { grpc, crud: true, text: CLIENT.into(), interfere: 0, invalid: false, updates: Vec::new(), lines: Vec::new() }
    }

    fn caps(&self) -> Value {
        json!({ "interface_version": 1, "types": [
            { "type": "SCHEDULE", "operations": ["READ_RAW", "CREATE_RAW", "UPDATE_RAW", "DELETE"] },
            { "type": "CLIENT", "operations": ["READ_RAW", "CREATE_RAW", "UPDATE_RAW", "DELETE"] } ] })
    }

    fn read(&self, name: &str) -> Value {
        if name != "laptop-fd" {
            return json!({ "status": "NOT_FOUND", "message": format!("Client \"{name}\" not found") });
        }
        json!({ "status": "OK", "resource": { "type": "CLIENT", "name": name }, "file": "/etc/bareos/bareos-dir.d/client/laptop-fd.conf",
                "text": self.text, "sha256": sha(&self.text) })
    }

    fn update(&mut self, sha256: &str, text: &str) -> Value {
        if self.interfere > 0 {
            self.interfere -= 1;
            self.text = self.text.replace("# laptop", "# laptop (edited)");
        }
        if sha256 != sha(&self.text) {
            return json!({ "status": "CONFLICT", "message": "Client \"laptop-fd\" changed since it was read",
                           "text": self.text, "sha256": sha(&self.text) });
        }
        if self.invalid {
            return json!({ "status": "INVALID", "message": "configuration invalid: Keyword \"Bogus\" not permitted",
                "problem": { "file": "/etc/bareos/bareos-dir.d/client/laptop-fd.conf", "line": 9, "column": 3,
                             "text": "Config error: Keyword \"Bogus\" not permitted in this resource." } });
        }
        let old = sha(&self.text);
        self.text = text.to_string();
        self.updates.push(text.to_string());
        json!({ "status": "OK", "message": "Client \"laptop-fd\" changed", "file": "/etc/bareos/bareos-dir.d/client/laptop-fd.conf",
                "text": text, "sha256": sha(text), "old_sha256": old, "reloaded": true })
    }

    fn arg<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        line.split(' ').find_map(|w| w.strip_prefix(key))
    }
}

/// Encode a JSON answer as the output message of `rpc` (enums by name).
fn encode_reply(rpc: &str, value: &Value) -> Vec<u8> {
    let (_, output) = crate::mcp_typed::rpc_types(rpc).unwrap();
    let encoded = typed::json_to_message(&output, value);
    assert!(encoded.unmapped.is_empty(), "{rpc}: {:?}", encoded.unmapped);
    encoded.bytes
}

impl CommandRunner for FakeDirector {
    fn run(&mut self, command: &str) -> anyhow::Result<String> {
        self.lines.push(command.to_string());
        if command.starts_with(".api") {
            return Ok(String::new());
        }
        anyhow::bail!("unexpected console line {command}")
    }

    fn run_json(&mut self, command: &str) -> anyhow::Result<String> {
        assert!(!self.grpc, "the gRPC route must not use console lines");
        self.lines.push(command.to_string());
        if !self.crud || !command.starts_with(".config ") {
            return Ok(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": 1, "message": "failed",
                "data": { "result": {}, "messages": { "error": [format!("{}: is an invalid command.\n", command.split(' ').next().unwrap())] } } } }).to_string());
        }
        let result = if command == ".config capabilities" {
            self.caps()
        } else if command.starts_with(".config read ") {
            assert_eq!(Self::arg(command, "type="), Some("client"));
            self.read(Self::arg(command, "name=").unwrap())
        } else if command.starts_with(".config update ") {
            let text = base64::engine::general_purpose::STANDARD.decode(Self::arg(command, "text64=").unwrap()).unwrap();
            let sha256 = Self::arg(command, "sha256=").unwrap().to_string();
            self.update(&sha256, &String::from_utf8(text).unwrap())
        } else {
            panic!("unexpected {command}")
        };
        Ok(json!({ "jsonrpc": "2.0", "id": null, "result": result }).to_string())
    }

    fn is_grpc(&self) -> bool {
        self.grpc
    }

    fn typed_call(&mut self, rpc: &str, request: Vec<u8>) -> Option<Result<Vec<u8>, TypedError>> {
        if !self.grpc {
            return None;
        }
        self.lines.push(rpc.to_string());
        if !self.crud {
            return Some(Err(TypedError::Unimplemented));
        }
        let (input, _) = crate::mcp_typed::rpc_types(rpc).unwrap();
        let req = typed::message_to_json(&input, &request);
        let reply = match rpc {
            "GetConfigCapabilities" => self.caps(),
            "ReadResource" => {
                assert_eq!(enum_name("ResourceType", "RESOURCE_TYPE_", &req["resource"]["type"]), "CLIENT");
                self.read(req["resource"]["name"].as_str().unwrap())
            }
            "UpdateResource" => self.update(req["expected_sha256"].as_str().unwrap_or(""), req["text"].as_str().unwrap()),
            other => panic!("unexpected RPC {other}"),
        };
        Some(Ok(encode_reply(rpc, &reply)))
    }
}

fn args(directives: Value) -> Value {
    json!({ "resource": "client", "name": "laptop-fd", "directives": directives })
}

#[test]
fn updates_over_grpc_and_bnet_with_the_same_result() {
    for grpc in [true, false] {
        let mut d = FakeDirector::new(grpc);
        let old = sha(CLIENT);
        let r = configure_update(&mut d, &args(json!({ "File Retention": "60 days", "Heartbeat Interval": 30 })));
        assert_eq!(r["isError"], false, "{r}");
        let expected = CLIENT.replace("File Retention = 30 days  # keep\n  FileRetention = 40 days\n", "File Retention = \"60 days\"  # keep\n")
            .replace("}\n", "  Heartbeat Interval = 30\n}\n");
        assert_eq!(d.updates, vec![expected.clone()], "grpc={grpc}");
        let s = &r["structuredContent"];
        assert_eq!((s["old_sha256"].as_str().unwrap(), s["new_sha256"].as_str().unwrap()), (old.as_str(), sha(&expected).as_str()));
        assert_eq!((s["reloaded"].as_bool(), s["route"].as_str()), (Some(true), Some(if grpc { "grpc" } else { "bnet" })));
        assert_eq!(s["changed"][0]["old"], json!(["30 days", "40 days"]));
        assert_eq!(s["changed"][1]["added"], true);
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("File Retention: 30 days | 40 days -> \"60 days\"") && text.contains("reloaded: yes"), "{text}");
        if grpc {
            assert_eq!(d.lines, ["GetConfigCapabilities", "ReadResource", "UpdateResource"]);
        } else {
            assert_eq!(d.lines[0], ".config capabilities");
            assert_eq!(d.lines[1], ".config read type=client name=laptop-fd");
            assert!(d.lines[2].starts_with(&format!(".config update type=client name=laptop-fd sha256={old} text64=")));
            assert!(!d.lines.iter().any(|l| l.starts_with("configure")));
        }
    }
}

#[test]
fn conflict_reads_once_more_then_gives_up() {
    for grpc in [true, false] {
        let mut d = FakeDirector::new(grpc);
        d.interfere = 1;
        let r = configure_update(&mut d, &args(json!({ "address": "new.example" })));
        assert_eq!(r["isError"], false, "{r}");
        // the retry kept the other change
        assert!(d.text.contains("# laptop (edited)") && d.text.contains("Address = new.example   ;"), "{}", d.text);

        let mut d = FakeDirector::new(grpc);
        d.interfere = 2;
        let r = configure_update(&mut d, &args(json!({ "address": "new.example" })));
        assert_eq!(r["isError"], true);
        assert!(r["content"][0]["text"].as_str().unwrap().contains("CONFLICT again"), "{r}");
        assert!(d.updates.is_empty());
    }
}

#[test]
fn invalid_comes_back_with_file_line_and_text() {
    for grpc in [true, false] {
        let mut d = FakeDirector::new(grpc);
        d.invalid = true;
        let r = configure_update(&mut d, &args(json!({ "Bogus": "1" })));
        assert_eq!(r["isError"], true);
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("INVALID") && text.contains("laptop-fd.conf:9:3") && text.contains("Keyword \"Bogus\""), "{text}");
        assert_eq!(r["structuredContent"]["problem"]["line"], 9);
    }
}

#[test]
fn refuses_without_crud_and_never_uses_configure_update() {
    for grpc in [true, false] {
        let mut d = FakeDirector::new(grpc);
        d.crud = false;
        let r = configure_update(&mut d, &args(json!({ "address": "h" })));
        assert_eq!(r["isError"], true);
        let text = r["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("configure_update refused") && text.contains("K-10"), "{text}");
        assert!(!d.lines.iter().any(|l| l.starts_with("configure")), "{:?}", d.lines);
        // a type the Director does not offer
        let mut d = FakeDirector::new(grpc);
        let r = configure_update(&mut d, &json!({ "resource": "fileset", "name": "fs", "directives": { "a": "b" } }));
        assert!(r["content"][0]["text"].as_str().unwrap().contains("does not offer READ_RAW and UPDATE_RAW for fileset"), "{r}");
        // a type without CRUD
        let r = configure_update(&mut d, &json!({ "resource": "pool", "name": "Full", "directives": { "a": "b" } }));
        assert!(r["content"][0]["text"].as_str().unwrap().contains("resource: one of client, job"), "{r}");
        // nested block, unknown resource
        let mut d = FakeDirector::new(grpc);
        d.text = "Client {\n  Name = laptop-fd\n  X {\n    Y = 1\n  }\n}\n".into();
        let r = configure_update(&mut d, &args(json!({ "y": "2" })));
        assert!(r["content"][0]["text"].as_str().unwrap().contains("nested block 'X'"), "{r}");
        let r = configure_update(&mut d, &json!({ "resource": "client", "name": "other", "directives": { "a": "b" } }));
        assert!(r["content"][0]["text"].as_str().unwrap().contains("NOT_FOUND"), "{r}");
        assert!(d.updates.is_empty());
    }
}

#[test]
fn no_change_writes_nothing() {
    let mut d = FakeDirector::new(true);
    let r = configure_update(&mut d, &args(json!({ "tls enable": true })));
    assert_eq!(r["isError"], false);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("no change"));
    assert!(d.updates.is_empty());
}

#[test]
fn the_write_tool_routes_configure_update_here() {
    let d = std::sync::Mutex::new(FakeDirector::new(true));
    let r = crate::mcp_write::call("configure_update", &args(json!({ "address": "x.example" })), &d).unwrap().unwrap();
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(d.lock().unwrap().lines, ["GetConfigCapabilities", "ReadResource", "UpdateResource"]);
}

/// `null` in `directives` removes every line of the directive, the rest stays.
#[test]
fn null_removes_the_directive() {
    let changes = directives(&serde_json::json!({"directives": {"File Retention": null, "address": "h2"}}), "client").unwrap();
    assert_eq!(changes[0], ("File Retention".to_string(), Vec::<String>::new()));
    let (out, report) = replace_directives(CLIENT, &changes).unwrap();
    assert!(!out.contains("Retention"), "{out}");
    assert!(out.contains("Address = h2   ; Port = 9102\n") && out.contains("TLS Enable = yes"), "{out}");
    assert_eq!(report[0].old, ["30 days", "40 days"]);
    assert!(report[0].new.is_empty());
}
