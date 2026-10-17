//! The remaining write commands as typed tools (S-36): add, create,
//! export/import/move, update slots/jobid/stats, sqlquery, query, .sql,
//! .bvfs_update/_restore/_cleanup/_clear_cache, .acl and .profile changes,
//! configure delete/update, use, setdevice and the session options
//! autodisplay/automount/trace. Same rules as the tools in mcp_write.rs:
//! one checked command line, dialogs answered only where the tool says so.

use super::*;

const SLOTS: &str = "slot selection, e.g. 1-3,7";
const ACL_ACTIONS: &[&str] = &["set", "delete", "clear", "copy"];
const PROFILE_ACTIONS: &[&str] = &["create", "set", "delete", "assign", "unassign"];
const SESSION_OPTIONS: &[&str] = &["autodisplay", "automount", "trace"];

pub(super) fn tools() -> Vec<Value> {
    let slots = |extra: Value| {
        let mut p = json!({
            "storage": s("Storage resource (autochanger)"),
            "srcslots": s(SLOTS),
            "dstslots": s(SLOTS),
        });
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    vec![
        tool("add_volumes", "Add volumes to the catalog",
            "Create volume records in a pool without labeling them (bconsole 'add'; the Director's \
             questions are answered from the arguments): count volumes named name0001… from start, \
             or one volume with count 0.",
            json!({
                "pool": s("Pool of the new volumes"),
                "storage": s("Storage (gives the media type)"),
                "count": i(0, "number of volumes; 0 = one volume with exactly this name"),
                "name": s("volume name (count 0) or base name"),
                "start": i(0, "first number (default 1)"),
                "slot": i(0, "autochanger slot (default 0)"),
                "inchanger": b("volume is in the autochanger"),
            }), &["pool", "storage", "count", "name"], annotations(false, false, false)),
        tool("create_pool", "Create a pool record",
            "Write the Pool resource into the catalog if missing (bconsole 'create pool=…').",
            json!({ "pool": s("Pool resource") }), &["pool"], annotations(false, false, true)),
        tool("export_volumes", "Move volumes to the import/export slots",
            "Autochanger: move volumes from srcslots (or the named volume) to the export slots \
             (bconsole 'export').",
            slots(json!({ "volume": s("volume name instead of srcslots"), "scan": b("scan the export slots") })),
            &["storage"], annotations(false, false, false)),
        tool("import_volumes", "Import volumes from the import/export slots",
            "Autochanger: move volumes from the import/export slots into free slots (bconsole 'import').",
            slots(json!({ "volume": s("only this volume"), "scan": b("scan the imported volumes") })),
            &["storage"], annotations(false, false, false)),
        tool("move_volumes", "Move volumes between slots",
            "Autochanger: move volumes from srcslots to dstslots (bconsole 'move').",
            slots(json!({})), &["storage", "srcslots", "dstslots"], annotations(false, false, false)),
        tool("update_slots", "Update slots from the autochanger",
            "Read the autochanger inventory and update Slot/InChanger in the catalog (bconsole 'update slots').",
            json!({ "storage": s("Storage (autochanger)"), "drive": i(0, "drive"), "scan": b("read every volume label") }),
            &["storage"], annotations(false, false, true)),
        tool("update_job", "Change a job record",
            "Change catalog fields of one job (bconsole 'update jobid=…').",
            json!({
                "jobid": i(1, "JobId"),
                "jobname": s("new job name"),
                "starttime": s("new start time 'YYYY-MM-DD HH:MM:SS'"),
                "client": s("new client"),
                "filesetid": i(1, "new FileSetId"),
                "jobtype": e(&["B", "R", "V", "D", "C", "c", "M", "g", "O", "A", "S", "U", "I"], "new job type letter"),
            }), &["jobid"], annotations(false, false, true)),
        tool("update_stats", "Update job statistics",
            "Copy job statistics into the JobHisto table (bconsole 'update stats').",
            json!({ "days": i(0, "only jobs of the last n days") }), &[], annotations(false, false, true)),
        tool("sqlquery", "Run SQL on the catalog",
            "Run one SQL statement on the catalog in the sqlquery dialog and return the result table. \
             Not checked: UPDATE/DELETE change the catalog permanently. Prefer the list_* tools.",
            json!({ "query": s("one SQL statement") }), &["query"], annotations(false, true, false)),
        tool("dot_sql", "Run SQL on the catalog (.sql)",
            "Run one SQL statement (bconsole '.sql query=…'). Not checked: writing statements change the catalog.",
            json!({ "query": s("one SQL statement"), "catalog": s("catalog (default: the session's)") }),
            &["query"], annotations(false, true, false)),
        tool("run_query", "Run a predefined catalog query",
            "Run one of the Director's predefined queries (bconsole 'query'; QueryFile) by number with its \
             parameters in order. The predefined queries only read.",
            json!({ "number": i(1, "query number"),
                "parameters": { "type": "array", "items": { "type": "string" }, "description": "answers to the query's questions, in order" } }),
            &["number"], annotations(false, false, true)),
        tool("bvfs_update", "Build the BVFS cache",
            "Fill the BVFS path cache for jobs (bconsole '.bvfs_update'); all jobs if jobids is omitted.",
            json!({ "jobids": { "type": "array", "items": { "type": "integer", "minimum": 1 }, "description": "JobIds" } }),
            &[], annotations(false, false, true)),
        tool("bvfs_restore", "Prepare a BVFS restore table",
            "Create the restore table <path> from files/directories selected with the bvfs_* tools \
             (bconsole '.bvfs_restore'); use it with restore/run and clean up with bvfs_cleanup.",
            json!({
                "path": s("table name, e.g. b2123"),
                "jobid": s("JobIds, comma-separated"),
                "fileid": s("FileIds, comma-separated"),
                "dirid": s("PathIds of whole directories, comma-separated"),
                "hardlink": s("JobId,FileIndex pairs, comma-separated"),
            }), &["path", "jobid"], annotations(false, false, false)),
        tool("bvfs_cleanup", "Drop a BVFS restore table", "Drop the table of bvfs_restore (bconsole '.bvfs_cleanup').",
            json!({ "path": s("table name") }), &["path"], annotations(false, false, true)),
        tool("bvfs_clear_cache", "Clear the BVFS cache", "Empty the BVFS path cache (bconsole '.bvfs_clear_cache yes').",
            json!({}), &[], annotations(false, false, true)),
        tool("acl_modify", "Change console ACLs",
            "Change the ACLs of a console in the running Director (Onesimus '.acl set/delete/clear/copy'). \
             type: e.g. job, client, command (…acl); value: entries, comma-separated.",
            json!({
                "action": e(ACL_ACTIONS, "set, delete, clear or copy"),
                "console": s("console (set, delete, clear)"),
                "type": s("ACL type, e.g. job, client, pool, command"),
                "value": s("entries, comma-separated (set; delete: optional)"),
                "from": s("copy: source console"),
                "to": s("copy: target console"),
            }), &["action"], annotations(false, true, false)),
        tool("profile_modify", "Change profiles",
            "Create or change profiles and their assignment (Onesimus '.profile create/set/delete/assign/unassign').",
            json!({
                "action": e(PROFILE_ACTIONS, "what to do"),
                "name": s("profile (create, set, delete)"),
                "type": s("ACL type (set, delete)"),
                "value": s("entries (set, delete)"),
                "console": s("console (assign, unassign)"),
                "profile": s("profile (assign, unassign)"),
            }), &["action"], annotations(false, true, false)),
        tool("configure_update", "Change a configuration resource",
            "Change directives of a resource in the running Director and its file (Onesimus 'configure update').",
            json!({
                "resource": e(CONFIGURE_TYPES, "resource type"),
                "name": s("resource name"),
                "directives": { "type": "object", "description": "directive -> new value",
                    "additionalProperties": { "type": ["string", "number", "boolean"] } },
            }), &["resource", "name", "directives"], annotations(false, false, true)),
        tool("configure_delete", "Delete a configuration resource",
            "Remove a resource from the running Director and delete its file (Onesimus 'configure delete'). \
             Requires confirm=true.",
            json!({ "resource": e(CONFIGURE_TYPES, "resource type"), "name": s("resource name"), "confirm": confirm() }),
            &["resource", "name", "confirm"], annotations(false, true, true)),
        tool("use_catalog", "Switch the catalog",
            "Use another catalog for this session (bconsole 'use catalog=…').",
            json!({ "catalog": s("Catalog resource") }), &["catalog"], annotations(false, false, true)),
        tool("setdevice", "Set device options",
            "Set Autoselect of a storage daemon device (bconsole 'setdevice').",
            json!({ "storage": s("Storage"), "device": s("device name"), "autoselect": b("autoselect") }),
            &["storage", "device"], annotations(false, false, true)),
        tool("session_option", "Console session options",
            "Switch autodisplay (show messages automatically), automount (mount after label) or trace \
             (Director trace file) on or off (bconsole 'autodisplay|automount|trace on|off').",
            json!({ "option": e(SESSION_OPTIONS, "which option"), "on": b("true = on, false = off") }),
            &["option", "on"], annotations(false, false, true)),
    ]
}

fn slot_selection(key: &str, value: &str) -> Result<String, String> {
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '-') {
        return Err(format!("{key} must be a slot selection like 1-3,7"));
    }
    Ok(format!("{key}={value}"))
}

fn id_text(key: &str, value: &str) -> Result<String, String> {
    if value.is_empty() || !value.split(',').all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())) {
        return Err(format!("{key} must be comma-separated numbers"));
    }
    Ok(format!("{key}={value}"))
}

fn word(key: &str, value: &str) -> Result<String, String> {
    if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{key} must be one word of letters and digits"));
    }
    Ok(value.to_ascii_lowercase())
}

/// ACL entries: names, wildcards and regexes, comma-separated (no blanks, quotes or ';').
fn entries(key: &str, value: &str) -> Result<String, String> {
    if value.is_empty() || value.chars().any(|c| c.is_control() || c.is_whitespace() || "\"';@".contains(c)) {
        return Err(format!("{key} must be comma-separated entries without blanks or quotes"));
    }
    Ok(value.to_string())
}

pub(super) fn plan(tool: &str, arguments: &Value) -> Option<Result<Plan, String>> {
    let builder: fn(&Args) -> Result<Plan, String> = match tool {
        "add_volumes" => add_volumes,
        "create_pool" => |a| Ok(Plan::new(format!("create {}", name_arg("pool", &req(a, "pool")?)?))),
        "export_volumes" => |a| slots_cmd(a, "export"),
        "import_volumes" => |a| slots_cmd(a, "import"),
        "move_volumes" => |a| {
            if !a.has("srcslots") || !a.has("dstslots") {
                return Err("move needs srcslots and dstslots".into());
            }
            slots_cmd(a, "move")
        },
        "update_slots" => |a| {
            let mut p = vec!["update slots".to_string(), name_arg("storage", &req(a, "storage")?)?];
            if a.flag("scan")? {
                p.push("scan".into());
            }
            push_int(&mut p, a, "drive", 0)?;
            Ok(Plan::new(p.join(" ")))
        },
        "update_job" => update_job,
        "update_stats" => |a| {
            let mut p = vec!["update stats".to_string()];
            push_int(&mut p, a, "days", 0)?;
            Ok(Plan::new(p.join(" ")))
        },
        "sqlquery" => |a| {
            let mut query = req(a, "query")?;
            if !query.trim_end().ends_with(';') {
                query.push(';');
            }
            let mut plan = Plan::new("sqlquery".into());
            plan.answers = vec![("Enter SQL query", query), ("Enter SQL query", String::new())];
            Ok(plan)
        },
        "dot_sql" => |a| {
            let mut p = vec![".sql".to_string(), quoted("query", &req(a, "query")?)?];
            push_name(&mut p, a, "catalog")?;
            Ok(Plan::new(p.join(" ")))
        },
        "run_query" => |a| {
            let number = a.int("number", 1)?.ok_or("number is required")?;
            let mut plan = Plan::new("query".into());
            plan.answers.push(("Choose a query", number.to_string()));
            for v in a.strings("parameters")?.unwrap_or_default() {
                if v.chars().any(char::is_control) {
                    return Err("parameters must be single lines".into());
                }
                plan.answers.push(("", v));
            }
            Ok(plan)
        },
        "bvfs_update" => |a| {
            let mut p = vec![".bvfs_update".to_string()];
            if let Some(ids) = a.ids("jobids")? {
                p.push(format!("jobid={}", id_list(&ids)));
            }
            Ok(Plan::new(p.join(" ")))
        },
        "bvfs_restore" => |a| {
            let mut p = vec![".bvfs_restore".to_string(), format!("path={}", word("path", &req(a, "path")?)?)];
            p.push(id_text("jobid", &req(a, "jobid")?)?);
            for key in ["fileid", "dirid", "hardlink"] {
                if let Some(v) = a.text(key)? {
                    p.push(id_text(key, &v)?);
                }
            }
            Ok(Plan::new(p.join(" ")))
        },
        "bvfs_cleanup" => |a| Ok(Plan::new(format!(".bvfs_cleanup path={}", word("path", &req(a, "path")?)?))),
        "bvfs_clear_cache" => |_| Ok(Plan::new(".bvfs_clear_cache yes".into())),
        "acl_modify" => acl_modify,
        "profile_modify" => profile_modify,
        "configure_update" => |a| configure_change(a, "update"),
        "configure_delete" => |a| {
            a.confirmed()?;
            configure_change(a, "delete")
        },
        "use_catalog" => |a| Ok(Plan::new(format!("use {}", name_arg("catalog", &req(a, "catalog")?)?))),
        "setdevice" => |a| {
            let mut p = vec!["setdevice".to_string(), name_arg("storage", &req(a, "storage")?)?,
                name_arg("device", &req(a, "device")?)?];
            if let Some(v) = a.opt_bool("autoselect")? {
                p.push(format!("autoselect={}", yes_no(v)));
            }
            Ok(Plan::new(p.join(" ")))
        },
        "session_option" => |a| {
            let option = a.choice("option", SESSION_OPTIONS)?.ok_or("option is required")?;
            let on = a.opt_bool("on")?.ok_or("on is required")?;
            Ok(Plan::new(format!("{option} {}", if on { "on" } else { "off" })))
        },
        _ => return None,
    };
    Some(Args::new(tool, arguments).and_then(|args| {
        args.check_known()?;
        builder(&args)
    }))
}

fn add_volumes(a: &Args) -> Result<Plan, String> {
    let mut p = vec!["add".to_string(), name_arg("pool", &req(a, "pool")?)?, name_arg("storage", &req(a, "storage")?)?];
    p.retain(|w| !w.is_empty());
    let count = a.int("count", 0)?.ok_or("count is required")?;
    if count > 10000 {
        return Err("count must be at most 10000".into());
    }
    let name = req(a, "name")?;
    if !valid_name(&name) || name.contains(' ') {
        return Err("name must be a volume name without blanks".into());
    }
    let mut plan = Plan::new(p.join(" "));
    plan.answers.push(("number of Volumes", count.to_string()));
    plan.answers.push(("olume name", name));
    if count > 0 {
        plan.answers.push(("starting number", a.int("start", 0)?.unwrap_or(1).to_string()));
    } else if a.has("start") {
        return Err("start only with count > 0".into());
    }
    plan.answers.push(("Enter slot", a.int("slot", 0)?.unwrap_or(0).to_string()));
    plan.answers.push(("InChanger", yes_no(a.flag("inchanger")?).to_string()));
    Ok(plan)
}

fn slots_cmd(a: &Args, command: &str) -> Result<Plan, String> {
    let mut p = vec![command.to_string(), name_arg("storage", &req(a, "storage")?)?];
    for key in ["srcslots", "dstslots"] {
        if let Some(v) = a.text(key)? {
            p.push(slot_selection(key, &v)?);
        }
    }
    push_name(&mut p, a, "volume")?;
    if a.flag("scan")? {
        p.push("scan".into());
    }
    Ok(Plan::new(p.join(" ")))
}

fn update_job(a: &Args) -> Result<Plan, String> {
    let jobid = a.int("jobid", 1)?.ok_or("jobid is required")?;
    let mut p = vec!["update".to_string(), format!("jobid={jobid}")];
    push_name(&mut p, a, "jobname")?;
    if let Some(v) = a.text("starttime")? {
        p.push(datetime("starttime", &v)?);
    }
    push_name(&mut p, a, "client")?;
    push_int(&mut p, a, "filesetid", 1)?;
    if let Some(t) = a.text("jobtype")? {
        if t.len() != 1 || !t.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err("jobtype must be one job type letter".into());
        }
        p.push(format!("jobtype={t}"));
    }
    if p.len() == 2 {
        return Err("give at least one field to change".into());
    }
    Ok(Plan::new(p.join(" ")))
}

fn acl_modify(a: &Args) -> Result<Plan, String> {
    let action = a.choice("action", ACL_ACTIONS)?.ok_or("action is required")?;
    let allowed: &[&str] = match action.as_str() {
        "set" => &["console", "type", "value"],
        "delete" => &["console", "type", "value"],
        "clear" => &["console"],
        _ => &["from", "to"],
    };
    for key in ["console", "type", "value", "from", "to"] {
        if a.has(key) && !allowed.contains(&key) {
            return Err(format!("{key} does not apply to .acl {action}"));
        }
    }
    let mut p = vec![format!(".acl {action}")];
    if action == "copy" {
        p.push(name_arg("from", &req(a, "from")?)?);
        p.push(name_arg("to", &req(a, "to")?)?);
        return Ok(Plan::new(p.join(" ")));
    }
    p.push(name_arg("console", &req(a, "console")?)?);
    if action == "clear" {
        return Ok(Plan::new(p.join(" ")));
    }
    let kind = word("type", &req(a, "type")?)?;
    let kind = if kind.ends_with("acl") { kind } else { format!("{kind}acl") };
    match (action.as_str(), a.text("value")?) {
        (_, Some(v)) => p.push(format!("{kind}={}", entries("value", &v)?)),
        ("delete", None) => p.push(kind),
        _ => return Err("value is required".into()),
    }
    Ok(Plan::new(p.join(" ")))
}

fn profile_modify(a: &Args) -> Result<Plan, String> {
    let action = a.choice("action", PROFILE_ACTIONS)?.ok_or("action is required")?;
    let keys: &[&str] = match action.as_str() {
        "create" => &["name"],
        "set" | "delete" => &["name", "type", "value"],
        _ => &["console", "profile"],
    };
    for key in ["name", "type", "value", "console", "profile"] {
        if a.has(key) && !keys.contains(&key) {
            return Err(format!("{key} does not apply to .profile {action}"));
        }
    }
    let mut p = vec![format!(".profile {action}")];
    for key in keys {
        let v = req(a, key)?;
        p.push(match *key {
            "type" => format!("type={}", word("type", &v)?),
            "value" => format!("value={}", entries("value", &v)?),
            _ => name_arg(key, &v)?,
        });
    }
    Ok(Plan::new(p.join(" ")))
}

/// `configure update|delete <type>=<name> [directive=value …]` (Onesimus).
fn configure_change(a: &Args, action: &str) -> Result<Plan, String> {
    let kind = a.choice("resource", CONFIGURE_TYPES)?.ok_or("resource is required")?;
    let mut p = vec![format!("configure {action}"), name_arg(&kind, &req(a, "name")?)?];
    match a.map.get("directives") {
        None | Some(Value::Null) if action == "delete" => {}
        Some(Value::Object(directives)) if action == "update" && !directives.is_empty() => {
            for (key, value) in directives {
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) || key == "name" {
                    return Err(format!("directive '{key}': lowercase letters and digits only, without spaces (not name)"));
                }
                let text = match value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => yes_no(*b).to_string(),
                    _ => return Err(format!("directive '{key}': string, number or boolean")),
                };
                if text.contains('@') || text.contains(';') {
                    return Err(format!("directive '{key}': '@' and ';' are not allowed"));
                }
                p.push(quoted(key, &text)?);
            }
        }
        _ if action == "update" => return Err("directives must be a non-empty object".into()),
        _ => return Err("directives does not apply to configure delete".into()),
    }
    if action == "delete" {
        p.push("yes".into());
    }
    Ok(Plan::new(p.join(" ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(tool: &str, args: Value) -> String {
        plan(tool, &args).expect("admin tool").unwrap_or_else(|e| panic!("{tool} {args}: {e}")).command
    }

    #[test]
    fn admin_lines() {
        assert_eq!(line("create_pool", json!({"pool":"Full"})), "create pool=Full");
        assert_eq!(line("export_volumes", json!({"storage":"Tape","srcslots":"1-3"})), "export storage=Tape srcslots=1-3");
        assert_eq!(line("move_volumes", json!({"storage":"Tape","srcslots":"1","dstslots":"5"})), "move storage=Tape srcslots=1 dstslots=5");
        assert_eq!(line("update_slots", json!({"storage":"Tape","scan":true,"drive":0})), "update slots storage=Tape scan drive=0");
        assert_eq!(line("update_job", json!({"jobid":3,"jobname":"x","jobtype":"B"})), "update jobid=3 jobname=x jobtype=B");
        assert_eq!(line("update_stats", json!({"days":7})), "update stats days=7");
        assert_eq!(line("dot_sql", json!({"query":"select 1"})), ".sql query=\"select 1\"");
        assert_eq!(line("bvfs_update", json!({"jobids":[1,2]})), ".bvfs_update jobid=1,2");
        assert_eq!(line("bvfs_restore", json!({"path":"b21","jobid":"5","fileid":"1,2"})), ".bvfs_restore path=b21 jobid=5 fileid=1,2");
        assert_eq!(line("bvfs_cleanup", json!({"path":"b21"})), ".bvfs_cleanup path=b21");
        assert_eq!(line("acl_modify", json!({"action":"set","console":"c","type":"job","value":"a,b"})), ".acl set console=c jobacl=a,b");
        assert_eq!(line("acl_modify", json!({"action":"delete","console":"c","type":"JobAcl"})), ".acl delete console=c jobacl");
        assert_eq!(line("acl_modify", json!({"action":"copy","from":"a","to":"b"})), ".acl copy from=a to=b");
        assert_eq!(line("profile_modify", json!({"action":"assign","console":"c","profile":"p"})), ".profile assign console=c profile=p");
        assert_eq!(line("configure_update", json!({"resource":"client","name":"fd","directives":{"address":"h"}})), "configure update client=fd address=\"h\"");
        assert_eq!(line("configure_delete", json!({"resource":"client","name":"fd","confirm":true})), "configure delete client=fd yes");
        assert_eq!(line("use_catalog", json!({"catalog":"MyCatalog"})), "use catalog=MyCatalog");
        assert_eq!(line("setdevice", json!({"storage":"S","device":"D","autoselect":false})), "setdevice storage=S device=D autoselect=no");
        assert_eq!(line("session_option", json!({"option":"automount","on":false})), "automount off");
        let add = plan("add_volumes", &json!({"pool":"P","storage":"S","count":2,"name":"V"})).unwrap().unwrap();
        assert_eq!(add.command, "add pool=P storage=S");
        assert_eq!(add.answers[0], ("number of Volumes", "2".to_string()));
        let q = plan("run_query", &json!({"number":3,"parameters":["fd"]})).unwrap().unwrap();
        assert_eq!(q.answers, vec![("Choose a query", "3".to_string()), ("", "fd".to_string())]);
    }

    #[test]
    fn admin_rejects() {
        for (tool, args) in [
            ("export_volumes", json!({"storage":"T","srcslots":"1;x"})),
            ("move_volumes", json!({"storage":"T","srcslots":"1"})),
            ("update_job", json!({"jobid":3})),
            ("update_job", json!({"jobid":3,"starttime":"now"})),
            ("bvfs_restore", json!({"path":"b 1","jobid":"5"})),
            ("acl_modify", json!({"action":"set","console":"c","type":"job","value":"a b"})),
            ("acl_modify", json!({"action":"clear","console":"c","type":"job"})),
            ("configure_delete", json!({"resource":"client","name":"fd"})),
            ("configure_update", json!({"resource":"client","name":"fd"})),
            ("session_option", json!({"option":"gui","on":true})),
            ("add_volumes", json!({"pool":"P","storage":"S","count":0,"name":"a b"})),
        ] {
            assert!(plan(tool, &args).unwrap().is_err(), "{tool} {args}");
        }
    }
}
