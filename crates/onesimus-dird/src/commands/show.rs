//! `show` — print the Director configuration exactly like the C++ Director.
//!
//! Port of dird/ua_output.cc show_cmd(), lib/res.cc PrintResourceItem(),
//! dird/dird_conf.cc PrintConfigCb()/FilesetResource::PrintConfig(),
//! lib/res.cc MessagesResource::PrintConfig() and the OutputFormatterResource
//! text/JSON rendering (Bareos 25.1.2).
//!
//! The typed config in `DirectorConfig` does not keep enough information, so
//! this works on the raw directives (`bareos_config::raw`) together with the
//! generated C++ directive tables (`show_items.rs`): a directive is printed
//! when it is set in the resource itself and differs from its default
//! (`verbose` also prints inherited and default values, as comments).

use serde_json::{json, Map, Value};

use bareos_config::raw::{normalize_key, ConfResource, Directive, Tok};

use super::show_items::{self as tables, Item, Ty};
use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;

// ---------------------------------------------------------------------------
// Resource kinds
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Director, Client, JobDefs, Job, Storage, Catalog, Schedule, FileSet,
    Pool, Messages, Counter, Profile, Console, User,
}

/// C++ dird_resource_tables order (show all), R_DEVICE skipped.
const ALL_KINDS: &[Kind] = &[
    Kind::Director, Kind::Client, Kind::JobDefs, Kind::Job, Kind::Storage, Kind::Catalog,
    Kind::Schedule, Kind::FileSet, Kind::Pool, Kind::Messages, Kind::Counter, Kind::Profile,
    Kind::Console, Kind::User,
];

/// ua_output.h show_cmd_available_resources (prefix matched, in this order).
const SHOW_KEYWORDS: &[(&str, Kind)] = &[
    ("directors", Kind::Director), ("clients", Kind::Client), ("jobs", Kind::Job),
    ("jobdefs", Kind::JobDefs), ("storages", Kind::Storage), ("catalogs", Kind::Catalog),
    ("schedules", Kind::Schedule), ("filesets", Kind::FileSet), ("pools", Kind::Pool),
    ("messages", Kind::Messages), ("counters", Kind::Counter), ("profiles", Kind::Profile),
    ("consoles", Kind::Console), ("users", Kind::User),
];

impl Kind {
    fn from_type(t: &str) -> Option<Kind> {
        Some(match normalize_key(t).as_str() {
            "director" => Kind::Director,
            "client" => Kind::Client,
            "jobdefs" => Kind::JobDefs,
            "job" => Kind::Job,
            "storage" => Kind::Storage,
            "catalog" => Kind::Catalog,
            "schedule" => Kind::Schedule,
            "fileset" => Kind::FileSet,
            "pool" => Kind::Pool,
            "messages" => Kind::Messages,
            "counter" => Kind::Counter,
            "profile" => Kind::Profile,
            "console" => Kind::Console,
            "user" => Kind::User,
            _ => return None,
        })
    }

    /// ResToStr
    fn name(self) -> &'static str {
        match self {
            Kind::Director => "Director", Kind::Client => "Client", Kind::JobDefs => "JobDefs",
            Kind::Job => "Job", Kind::Storage => "Storage", Kind::Catalog => "Catalog",
            Kind::Schedule => "Schedule", Kind::FileSet => "FileSet", Kind::Pool => "Pool",
            Kind::Messages => "Messages", Kind::Counter => "Counter", Kind::Profile => "Profile",
            Kind::Console => "Console", Kind::User => "User",
        }
    }

    /// ResGroupToStr, lowercased as JSON key
    fn group(self) -> &'static str {
        match self {
            Kind::Director => "directors", Kind::Client => "clients", Kind::JobDefs => "jobdefs",
            Kind::Job => "jobs", Kind::Storage => "storages", Kind::Catalog => "catalogs",
            Kind::Schedule => "schedules", Kind::FileSet => "filesets", Kind::Pool => "pools",
            Kind::Messages => "messages", Kind::Counter => "counters", Kind::Profile => "profiles",
            Kind::Console => "consoles", Kind::User => "users",
        }
    }

    fn items(self) -> &'static [Item] {
        match self {
            Kind::Director => tables::DIR_ITEMS,
            Kind::Client => tables::CLIENT_ITEMS,
            Kind::JobDefs | Kind::Job => tables::JOB_ITEMS,
            Kind::Storage => tables::STORAGE_ITEMS,
            Kind::Catalog => tables::CATALOG_ITEMS,
            Kind::Schedule => tables::SCHEDULE_ITEMS,
            Kind::FileSet => tables::FILESET_ITEMS,
            Kind::Pool => tables::POOL_ITEMS,
            Kind::Messages => tables::MESSAGES_ITEMS,
            Kind::Counter => tables::COUNTER_ITEMS,
            Kind::Profile => tables::PROFILE_ITEMS,
            Kind::Console => tables::CONSOLE_ITEMS,
            Kind::User => tables::USER_ITEMS,
        }
    }
}

/// The configuration as `show` sees it.
struct Conf<'a> {
    res: Vec<(Kind, &'a ConfResource)>,
}

impl<'a> Conf<'a> {
    fn new(raw: &'a [ConfResource]) -> Self {
        Self { res: raw.iter().filter_map(|r| Kind::from_type(&r.rtype).map(|k| (k, r))).collect() }
    }

    fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &'a ConfResource> + '_ {
        self.res.iter().filter(move |(k, _)| *k == kind).map(|(_, r)| *r)
    }

    fn find(&self, kind: Kind, name: &str) -> Option<&'a ConfResource> {
        self.of_kind(kind).find(|r| r.name() == Some(name))
    }
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

pub fn cmd_show(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let raw = match ctx.config.as_ref().and_then(|c| c.read().ok()) {
        Some(cfg) => cfg.raw.clone(),
        None => Vec::new(),
    };
    show(ast, &raw)
}

fn show(ast: &CommandAst, raw: &[ConfResource]) -> CommandResult {
    let conf = Conf::new(raw);
    let args = &ast.arguments;
    let verbose = args.iter().any(|a| a.key.eq_ignore_ascii_case("verbose"));
    let mut out = Out::default();

    if args.iter().any(|a| a.key.eq_ignore_ascii_case("help")) {
        out.text.push_str("Keywords for the show command are:\n");
        for (kw, _) in SHOW_KEYWORDS {
            out.text.push_str(kw);
            out.text.push('\n');
        }
        return out.finish();
    }

    if args.is_empty() || args.iter().any(|a| a.key.eq_ignore_ascii_case("all")) {
        for kind in ALL_KINDS {
            dump_kind(&conf, *kind, None, verbose, &mut out, false);
        }
        return out.finish();
    }

    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a.key.eq_ignore_ascii_case("verbose") {
            continue;
        }
        if a.key.eq_ignore_ascii_case("disabled") {
            let next = args.get(i).map(|n| n.key.to_ascii_lowercase());
            show_disabled(&conf, next.as_deref(), &mut out);
            return out.finish();
        }
        let len = a.key.len();
        // bstrncasecmp(res_name, keyword, strlen(res_name)): keyword starts with the argument
        let kind = SHOW_KEYWORDS.iter()
            .find(|(kw, _)| kw.len() >= len && kw[..len].eq_ignore_ascii_case(&a.key));
        let Some((kw, kind)) = kind else {
            out.error(format!("Resource {} not found\n", a.key));
            continue;
        };
        match &a.value {
            Some(name) => {
                if conf.find(*kind, name).is_none() && !(*kind == Kind::Console && name == "*UserAgent*") {
                    out.error(format!("{kw} resource {name} not found.\n"));
                    return out.finish();
                }
                dump_kind(&conf, *kind, Some(name), verbose, &mut out, true);
            }
            None => dump_kind(&conf, *kind, None, verbose, &mut out, true),
        }
    }
    out.finish()
}

fn show_disabled(conf: &Conf, which: Option<&str>, out: &mut Out) {
    let mut obj = Map::new();
    let mut list = |kind: Kind, key: &str, title: &str, out: &mut Out| {
        let names: Vec<String> = conf.of_kind(kind)
            .filter(|r| r.directives.iter().any(|d| d.norm_key() == "enabled"
                && d.first().is_some_and(|v| parse_bool(v) == Some(false))))
            .filter_map(|r| r.name().map(str::to_string))
            .collect();
        // Decoration (ua_output.cc:334,340): not printed in `.api 1`
        let deco = crate::sql_list::decorations();
        if names.is_empty() {
            if deco {
                out.text.push_str(&format!("No disabled {title}.\n"));
            }
        } else {
            if deco {
                out.text.push_str(&format!("Disabled {title}:\n"));
            }
            for n in &names {
                out.text.push_str(&format!("   {n}\n"));
            }
        }
        obj.insert(key.into(), json!(names));
    };
    match which {
        Some("jobs") => list(Kind::Job, "jobs", "Jobs", out),
        Some("clients") => list(Kind::Client, "clients", "Clients", out),
        Some("schedules") => list(Kind::Schedule, "schedules", "Schedules", out),
        _ => {
            list(Kind::Job, "jobs", "Jobs", out);
            list(Kind::Client, "clients", "Clients", out);
            list(Kind::Schedule, "schedules", "Schedules", out);
        }
    }
    out.json.insert("disabled".into(), Value::Object(obj));
}

/// DumpResource for one resource type (all, or the one with `only` name).
fn dump_kind(conf: &Conf, kind: Kind, only: Option<&str>, verbose: bool, out: &mut Out, report_empty: bool) {
    let mut found = false;
    for res in conf.of_kind(kind) {
        if only.is_some_and(|n| res.name() != Some(n)) {
            continue;
        }
        found = true;
        print_resource(conf, kind, res, verbose, false, out);
    }
    // internal "*UserAgent*" console (root console), only shown in verbose mode
    if kind == Kind::Console && verbose && only.map_or(true, |n| n == "*UserAgent*") {
        if let Some(dir) = conf.of_kind(Kind::Director).next() {
            let pw = dir.directives.iter().find(|d| d.norm_key() == "password").and_then(|d| d.first()).unwrap_or("");
            let ua = ConfResource {
                rtype: "Console".into(),
                directives: vec![
                    plain("Name", "*UserAgent*"),
                    plain("Description", "root console definition"),
                    plain("Password", pw),
                    // CreateAndAddUserAgentConsoleResource: object defaults, not config defaults
                    Directive { key: "TlsRequire".into(), tokens: vec![Tok::Word("no".into())], block: None },
                ],
                source: None,
            };
            print_resource(conf, kind, &ua, verbose, true, out);
            found = true;
        }
    }
    if !found && report_empty && only.is_none() {
        // InfoMsg: text only, not part of the JSON result
        out.text.push_str(&format!("No {} resource defined\n", kind.name()));
    }
}

fn plain(key: &str, value: &str) -> Directive {
    Directive { key: key.into(), tokens: vec![Tok::Quoted(value.into())], block: None }
}

// ---------------------------------------------------------------------------
// Output (OutputFormatterResource): text and JSON in parallel
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Out {
    text: String,
    json: Map<String, Value>,
    errors: String,
}

impl Out {
    /// ErrorMsg: the session prints error_message after text_output
    fn error(&mut self, msg: String) {
        self.errors.push_str(&msg);
    }

    fn finish(self) -> CommandResult {
        let json = Value::Object(self.json);
        if self.errors.is_empty() {
            CommandResult::ok(self.text).with_json(json)
        } else {
            let mut r = CommandResult::error(self.errors);
            r.text_output = self.text.clone();
            r.with_json(json)
        }
    }
}

/// Body writer for one resource: indentation, comment prefix, JSON object stack.
struct W {
    text: String,
    indent: usize,
    stack: Vec<(String, Container)>,
}

enum Container {
    Obj(Map<String, Value>),
    Arr(Vec<Value>),
}

impl W {
    fn new() -> Self {
        Self { text: String::new(), indent: 1, stack: vec![(String::new(), Container::Obj(Map::new()))] }
    }

    fn prefix(&self, comment: bool) -> String {
        let mut s = "  ".repeat(self.indent);
        if comment {
            s.push_str("# ");
        }
        s
    }

    fn put(&mut self, key: &str, v: Value) {
        match &mut self.stack.last_mut().unwrap().1 {
            Container::Obj(m) => {
                m.insert(key.to_lowercase(), v);
            }
            Container::Arr(a) => a.push(v),
        }
    }

    fn push_obj(&mut self, key: &str) {
        self.stack.push((key.to_lowercase(), Container::Obj(Map::new())));
    }

    fn push_arr(&mut self, key: &str) {
        self.stack.push((key.to_lowercase(), Container::Arr(Vec::new())));
    }

    fn pop(&mut self) {
        let (key, c) = self.stack.pop().unwrap();
        let v = match c {
            Container::Obj(m) => Value::Object(m),
            Container::Arr(a) => Value::Array(a),
        };
        self.put(&key, v);
    }

    fn into_obj(mut self) -> (String, Map<String, Value>) {
        match self.stack.pop() {
            Some((_, Container::Obj(m))) => (self.text, m),
            _ => (self.text, Map::new()),
        }
    }

    // --- OutputFormatterResource primitives ---------------------------------

    /// KeyQuotedString (None → KeyUnquotedString(NULL): only as comment)
    fn quoted(&mut self, name: &str, value: Option<&str>, comment: bool) {
        match value {
            Some(v) => {
                let line = format!("{}{name} = \"{v}\"\n", self.prefix(comment));
                self.text.push_str(&line);
                self.put(name, json!(v));
            }
            None => self.unquoted(name, None, comment),
        }
    }

    /// KeyUnquotedString / KeyString
    fn unquoted(&mut self, name: &str, value: Option<&str>, comment: bool) {
        if value.is_none() && !comment {
            return;
        }
        let line = format!("{}{name} = {}\n", self.prefix(comment), value.unwrap_or(""));
        self.text.push_str(&line);
        self.put(name, json!(value.unwrap_or("")));
    }

    fn boolean(&mut self, name: &str, v: bool, comment: bool) {
        let line = format!("{}{name} = {}\n", self.prefix(comment), if v { "Yes" } else { "No" });
        self.text.push_str(&line);
        self.put(name, json!(v));
    }

    fn signed(&mut self, name: &str, v: i64, comment: bool) {
        let line = format!("{}{name} = {v}\n", self.prefix(comment));
        self.text.push_str(&line);
        self.put(name, json!(v));
    }

    fn unsigned(&mut self, name: &str, v: u64, comment: bool) {
        let line = format!("{}{name} = {v}\n", self.prefix(comment));
        self.text.push_str(&line);
        self.put(name, json!(v));
    }

    /// KeyMultipleStringsOnePerLine
    fn one_per_line(&mut self, name: &str, list: &[String], comment: bool, quoted: bool, escape: bool) {
        if list.is_empty() {
            if comment {
                let line = format!("{}{name} = \n", self.prefix(comment));
                self.text.push_str(&line);
                self.put(name, json!([]));
            }
            return;
        }
        for item in list {
            let shown = if escape || requires_escaping(item) { escape_string(item) } else { item.clone() };
            let line = if quoted {
                format!("{}{name} = \"{shown}\"\n", self.prefix(comment))
            } else {
                format!("{}{name} = {shown}\n", self.prefix(comment))
            };
            self.text.push_str(&line);
        }
        self.put(name, json!(list));
    }

    /// KeyMultipleStringsInOneLine
    fn one_line(&mut self, name: &str, list: &[String], comment: bool, quoted: bool) {
        let mut line = format!("{}{name} = ", self.prefix(comment));
        for (i, item) in list.iter().enumerate() {
            if i > 0 {
                line.push_str(", ");
            }
            if quoted {
                line.push_str(&format!("\"{item}\""));
            } else {
                line.push_str(item);
            }
        }
        line.push('\n');
        self.text.push_str(&line);
        self.put(name, json!(list));
    }

    /// SubResourceStart with a "Name {" header (text), JSON object in an array
    fn sub_start(&mut self, header: &str, comment: bool) {
        let line = format!("{}{header} {{\n", self.prefix(comment));
        self.text.push_str(&line);
        self.indent += 1;
        self.push_obj("");
    }

    fn sub_end(&mut self, comment: bool) {
        self.indent -= 1;
        let line = format!("{}}}\n", self.prefix(comment));
        self.text.push_str(&line);
        self.pop();
    }
}

/// lib/util.cc EscapeString (used when dumping config strings)
fn escape_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            '\'' => o.push_str("''"),
            '\\' => o.push_str("\\\\"),
            '(' | ')' | '<' | '>' | '"' => {
                o.push('\\');
                o.push(c);
            }
            c => o.push(c),
        }
    }
    o
}

/// OutputFormatterResource::requiresEscaping: an unescaped double quote
fn requires_escaping(s: &str) -> bool {
    let mut esc = false;
    for c in s.chars() {
        match c {
            '\\' => esc = !esc,
            '"' if !esc => return true,
            _ => esc = false,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Resource printing
// ---------------------------------------------------------------------------

fn print_resource(conf: &Conf, kind: Kind, res: &ConfResource, verbose: bool, internal: bool, out: &mut Out) {
    if internal && !verbose {
        return;
    }
    let Some(name) = res.name() else { return };

    let mut w = W::new();
    match kind {
        Kind::FileSet => print_fileset(res, &mut w),
        Kind::Messages => print_messages(res, verbose, &mut w),
        _ => print_items(conf, kind, res, verbose, internal, &mut w),
    }
    let (body, obj) = w.into_obj();

    let hdr = if internal { "# " } else { "" };
    out.text.push_str(&format!("{hdr}{} {{\n{body}{hdr}}}\n\n", kind.name()));
    let group = out.json.entry(kind.group()).or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(g) = group {
        g.insert(name.to_string(), Value::Object(obj));
    }
}

/// Explicit directives of `res` matching `item` (name or alias), in order.
fn matching<'a>(res: &'a ConfResource, item: &Item) -> Vec<&'a Directive> {
    let names: Vec<String> = std::iter::once(item.name).chain(item.aliases.iter().copied())
        .map(normalize_key).collect();
    res.directives.iter().filter(|d| names.contains(&d.norm_key())).collect()
}

fn print_items(conf: &Conf, kind: Kind, res: &ConfResource, verbose: bool, internal: bool, w: &mut W) {
    // JobDefs inheritance (PropagateResource): values set in the JobDefs
    let jobdefs = if kind == Kind::Job {
        res.directives.iter().find(|d| d.norm_key() == "jobdefs")
            .and_then(|d| d.first())
            .and_then(|n| conf.find(Kind::JobDefs, n))
    } else {
        None
    };

    let items = kind.items();
    for item in items {
        let own = matching(res, item);
        let mut explicit = !own.is_empty() && !internal;
        let mut dirs = own;

        match item.ty {
            // Director addresses: "Addresses" counts as set whenever there are addresses
            Ty::Addresses => explicit = !internal,
            Ty::Runscript => {
                let scripts = runscripts(res);
                explicit = !scripts.is_empty() && !internal;
                let mut all: Vec<(Script, bool)> = scripts.into_iter().map(|s| (s, false)).collect();
                if let Some(jd) = jobdefs {
                    all.extend(runscripts(jd).into_iter().map(|s| (s, true)));
                }
                print_runscripts(item, &all, !explicit, verbose, w);
                continue;
            }
            _ => {}
        }

        if !explicit {
            if let Some(jd) = jobdefs {
                if propagates(item.ty) {
                    let inh = matching(jd, item);
                    if !inh.is_empty() {
                        dirs = inh;
                    }
                }
            }
        }

        let inherited = !explicit;
        if inherited && !verbose {
            continue;
        }
        let val = value(conf, kind, res, item, &dirs, explicit || !dirs.is_empty());
        let mut comment = inherited;
        let mut print = item.required;
        if is_default(item, &val) {
            if verbose && !item.deprecated {
                print = true;
                comment = true;
            }
        } else {
            print = true;
        }
        if !print {
            continue;
        }
        render(item, &val, comment, w);
    }
}

/// Types copied from JobDefs by PropagateResource().
fn propagates(ty: Ty) -> bool {
    matches!(ty, Ty::Str | Ty::Dir | Ty::DirOrCmd | Ty::Res | Ty::AlistStr | Ty::AlistRes | Ty::Acl
        | Ty::Bit | Ty::Pint32 | Ty::Jobtype | Ty::Protocoltype | Ty::Level | Ty::Int32 | Ty::Size32
        | Ty::Migtype | Ty::Replace | Ty::Time | Ty::Size64 | Ty::Speed | Ty::Bool | Ty::Autopassword)
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Str(Option<String>),
    StdStr(String),
    Signed(i64),
    Unsigned(u64),
    Size(u64),
    Time(u64),
    Bool(bool),
    /// KeyMultipleStringsOnePerLine, quoted
    Lines(Vec<String>),
    /// KeyMultipleStringsInOneLine, quoted
    OneLine(Vec<String>),
    /// CFG_TYPE_RES: resource name or "" when unset
    Res(Option<String>),
    /// enum keyword printed unquoted (KeyString); None = unset
    Kw(Option<&'static str>),
    Level(Option<&'static str>),
    PoolType(String),
    Password(Option<String>),
    Runs(Vec<String>),
    Addresses(Vec<Addr>),
    /// printed elsewhere or never (ADDRESSES_PORT, MSGS, SHRTRUNSCRIPT, INCEXC)
    Nothing,
}

#[derive(Debug, Clone, PartialEq)]
struct Addr {
    family: &'static str,
    addr: String,
    port: u64,
}

const JOB_TYPES: &[(&str, char)] = &[
    ("Backup", 'B'), ("Admin", 'D'), ("Archive", 'A'), ("Verify", 'V'), ("Restore", 'R'),
    ("Migrate", 'g'), ("Copy", 'c'), ("Consolidate", 'O'),
];

/// dird_conf.cc joblevels[] (name, level code)
pub(crate) const JOB_LEVELS: &[(&str, char)] = &[
    ("Full", 'F'), ("Base", 'B'), ("Incremental", 'I'), ("Differential", 'D'), ("Since", 'S'),
    ("VirtualFull", 'f'), ("Catalog", 'C'), ("InitCatalog", 'V'), ("VolumeToCatalog", 'O'),
    ("DiskToCatalog", 'd'), ("Data", 'A'),
];

/// keyword tables: (name, code); GetName() returns the first name of a code
const PROTOCOLS: &[(&str, u8)] = &[("Native", 0), ("NDMP_BAREOS", 1), ("NDMP", 1), ("NDMP_NATIVE", 2)];
const AUTH_PROTOCOLS: &[(&str, u8)] = &[("Native", 0), ("NDMPV2", 1), ("NDMPV3", 2), ("NDMPV4", 3)];
const AUTH_TYPES: &[(&str, u8)] = &[("None", 0), ("Clear", 1), ("MD5", 2)];
const REPLACE: &[(&str, u8)] = &[("Always", 0), ("IfNewer", 1), ("IfOlder", 2), ("Never", 3)];
const ACTION_ON_PURGE: &[(&str, u8)] = &[("None", 0), ("Truncate", 1)];
const MIG_TYPES: &[(&str, u8)] = &[
    ("SmallestVolume", 1), ("OldestVolume", 2), ("PoolOccupancy", 3), ("PoolTime", 4),
    ("PoolUncopiedJobs", 5), ("Client", 6), ("Volume", 7), ("Job", 8), ("SqlQuery", 9),
];
const TAPE_LABELS: &[(&str, u8)] = &[("bareos", 0), ("ansi", 1), ("ibm", 2)];
const POOL_TYPES: &[&str] = &["Backup", "Copy", "Cloned", "Archive", "Migration", "Scratch"];

fn kw_lookup(table: &'static [(&'static str, u8)], v: &str) -> Option<&'static str> {
    let code = table.iter().find(|(n, _)| n.eq_ignore_ascii_case(v))?.1;
    table.iter().find(|(_, c)| *c == code).map(|(n, _)| *n)
}

fn kw_table(ty: Ty) -> Option<&'static [(&'static str, u8)]> {
    Some(match ty {
        Ty::Protocoltype => PROTOCOLS,
        Ty::Authprotocoltype => AUTH_PROTOCOLS,
        Ty::Authtype => AUTH_TYPES,
        Ty::Replace => REPLACE,
        Ty::Actiononpurge => ACTION_ON_PURGE,
        Ty::Migtype => MIG_TYPES,
        Ty::Label => TAPE_LABELS,
        _ => return None,
    })
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "yes" | "true" => Some(true),
        "no" | "false" => Some(false),
        _ => None,
    }
}

fn last<'a>(dirs: &[&'a Directive]) -> Option<&'a str> {
    dirs.last().and_then(|d| d.first())
}

fn joined(dirs: &[&Directive]) -> Option<String> {
    dirs.last().map(|d| d.values().join(" "))
}

fn all_values(dirs: &[&Directive]) -> Vec<String> {
    dirs.iter().flat_map(|d| d.values()).map(str::to_string).collect()
}

/// Value of an item: from the directives (`set`), else its default.
fn value(conf: &Conf, kind: Kind, res: &ConfResource, item: &Item, dirs: &[&Directive], set: bool) -> Val {
    let def = item.default;
    match item.ty {
        Ty::Name | Ty::Str | Ty::Dir | Ty::Strname | Ty::DirOrCmd => {
            Val::Str(if set { last(dirs).map(str::to_string) } else { def.map(str::to_string) })
        }
        Ty::Stdstr | Ty::Stdstrdir => {
            Val::StdStr(if set { last(dirs).unwrap_or("").to_string() } else { def.unwrap_or("").to_string() })
        }
        Ty::Int32 => {
            let v = if set { last(dirs).and_then(|v| v.parse().ok()) } else { None };
            Val::Signed(v.or_else(|| def.and_then(|d| d.parse().ok())).unwrap_or(0))
        }
        Ty::Pint32 => {
            let v = if set { last(dirs).and_then(|v| v.parse().ok()) } else { None };
            Val::Unsigned(v.or_else(|| def.and_then(|d| d.parse().ok())).unwrap_or(0))
        }
        Ty::Speed => {
            let v = if set { joined(dirs).and_then(|v| parse_units(&v, SPEED_MODS)) } else { None };
            Val::Unsigned(v.or_else(|| def.and_then(|d| parse_units(d, SPEED_MODS))).unwrap_or(0))
        }
        Ty::Size32 | Ty::Size64 => {
            let v = if set { joined(dirs).and_then(|v| parse_units(&v, SIZE_MODS)) } else { None };
            Val::Size(v.or_else(|| def.and_then(|d| parse_units(d, SIZE_MODS))).unwrap_or(0))
        }
        Ty::Time => {
            let v = if set { joined(dirs).and_then(|v| parse_duration(&v)) } else { None };
            Val::Time(v.or_else(|| def.and_then(parse_duration)).unwrap_or(0))
        }
        Ty::Bool | Ty::Bit => {
            let v = if set { last(dirs).and_then(parse_bool) } else { None };
            Val::Bool(v.or_else(|| def.and_then(parse_bool)).unwrap_or(false))
        }
        Ty::AlistStr | Ty::StrVector => Val::Lines(if set { all_values(dirs) } else { Vec::new() }),
        Ty::PluginNames => Val::Lines(if set {
            all_values(dirs).iter().flat_map(|v| v.split(':').map(str::to_string).collect::<Vec<_>>()).collect()
        } else { Vec::new() }),
        Ty::AlistRes => Val::Lines(if set { all_values(dirs) } else { Vec::new() }),
        Ty::Acl | Ty::Device | Ty::Audit => Val::OneLine(if set { all_values(dirs) } else { Vec::new() }),
        Ty::Res => {
            let mut v = if set { last(dirs).map(str::to_string) } else { None };
            // ValidateResource: clients/storages without catalog get the first one
            if v.is_none() && kind == Kind::Client && item.name == "Catalog" {
                v = conf.of_kind(Kind::Catalog).next().and_then(|c| c.name()).map(str::to_string);
            }
            Val::Res(v)
        }
        Ty::Jobtype => Val::Kw(if set {
            last(dirs).and_then(|v| JOB_TYPES.iter().find(|(n, _)| n.eq_ignore_ascii_case(v)).map(|(n, _)| *n))
        } else { None }),
        Ty::Level => Val::Level(if set {
            last(dirs).and_then(|v| JOB_LEVELS.iter().find(|(n, _)| n.eq_ignore_ascii_case(v)))
                .and_then(|(_, code)| JOB_LEVELS.iter().find(|(_, c)| c == code)).map(|(n, _)| *n)
        } else { None }),
        Ty::Protocoltype | Ty::Authprotocoltype | Ty::Authtype | Ty::Replace | Ty::Actiononpurge
        | Ty::Migtype | Ty::Label => {
            let table = kw_table(item.ty).unwrap();
            let v = if set { last(dirs).and_then(|v| kw_lookup(table, v)) } else { None };
            Val::Kw(v.or_else(|| def.and_then(|d| kw_lookup(table, d))).or_else(|| {
                // unset enum value 0 → first keyword with code 0 (GetName)
                if matches!(item.ty, Ty::Migtype) { None } else { table.iter().find(|(_, c)| *c == 0).map(|(n, _)| *n) }
            }))
        }
        Ty::Pooltype => {
            let v = if set {
                last(dirs).and_then(|v| POOL_TYPES.iter().find(|p| p.eq_ignore_ascii_case(v)).map(|p| p.to_string()))
            } else { None };
            Val::PoolType(v.unwrap_or_else(|| def.unwrap_or("").to_string()))
        }
        Ty::Autopassword => Val::Password(if set { last(dirs).map(|p| password_display(kind, item, p)) } else { None }),
        Ty::Run => Val::Runs(if set { dirs.iter().map(|d| run_string(&d.tokens)).collect() } else { Vec::new() }),
        Ty::Addresses => Val::Addresses(director_addresses(res)),
        _ => Val::Nothing,
    }
}

/// Password as stored by StoreAutopassword: MD5 for most resources, clear
/// text for the catalog and the director's KeyEncryptionKey.
fn password_display(kind: Kind, item: &Item, value: &str) -> String {
    let clear = kind == Kind::Catalog || (kind == Kind::Director && item.code == Some("1"));
    if clear {
        return value.to_string();
    }
    match value.strip_prefix("[md5]") {
        Some(hex) => format!("[md5]{hex}"),
        None => format!("[md5]{}", bareos_bnet::cram_md5::md5_hash_password(value)),
    }
}

/// HasDefaultValue() of lib/res.cc and dird_conf.cc
fn is_default(item: &Item, val: &Val) -> bool {
    let def = item.default;
    match (item.ty, val) {
        (Ty::Name | Ty::Str | Ty::Dir | Ty::DirOrCmd | Ty::Strname, Val::Str(v)) => v.as_deref() == def,
        (Ty::Stdstr | Ty::Stdstrdir, Val::StdStr(v)) => match def {
            Some(d) => v == d,
            None => v.is_empty(),
        },
        (_, Val::Signed(v)) => *v == def.and_then(|d| d.parse().ok()).unwrap_or(0),
        (Ty::Pint32, Val::Unsigned(v)) => *v == def.and_then(|d| d.parse().ok()).unwrap_or(0),
        (Ty::Speed, Val::Unsigned(v)) => *v == def.and_then(|d| parse_units(d, SPEED_MODS)).unwrap_or(0),
        (_, Val::Size(v)) => *v == def.and_then(|d| parse_units(d, SIZE_MODS)).unwrap_or(0),
        (_, Val::Time(v)) => *v == def.and_then(parse_duration).unwrap_or(0),
        (Ty::Bool, Val::Bool(v)) => *v == def.and_then(parse_bool).unwrap_or(false),
        (Ty::Acl | Ty::Device | Ty::Audit, Val::OneLine(l)) => match def {
            Some(d) => l.len() == 1 && l[0] == d,
            None => l.is_empty(),
        },
        (Ty::Run, Val::Runs(r)) => r.is_empty(),
        (Ty::Jobtype, Val::Kw(v)) => v.is_none(),
        (Ty::Level, Val::Level(v)) => v.is_none(),
        (_, Val::Kw(v)) => match def {
            Some(d) => v.is_some_and(|n| n.eq_ignore_ascii_case(d)),
            None => v.is_none(),
        },
        (Ty::Pooltype, Val::PoolType(v)) => Some(v.as_str()) == def,
        _ => false,
    }
}

fn render(item: &Item, val: &Val, comment: bool, w: &mut W) {
    let name = item.name;
    match val {
        Val::Str(v) => {
            if item.ty == Ty::DirOrCmd {
                let e = v.as_deref().map(escape_string);
                w.quoted(name, e.as_deref(), comment);
            } else {
                w.quoted(name, v.as_deref(), comment);
            }
        }
        Val::StdStr(v) => w.quoted(name, Some(v), comment),
        Val::Signed(v) => w.signed(name, *v, comment),
        Val::Unsigned(v) => w.unsigned(name, *v, comment),
        Val::Size(v) => w.unquoted(name, Some(&size_si(*v)), comment),
        Val::Time(v) => w.unquoted(name, Some(&time_spec(*v)), comment),
        Val::Bool(v) => w.boolean(name, *v, comment),
        Val::Lines(l) => w.one_per_line(name, l, comment, true, false),
        Val::OneLine(l) => {
            // Device is always printed as a non-comment line
            let c = if item.ty == Ty::Device { false } else { comment };
            w.one_line(name, l, c, true);
        }
        Val::Res(v) => w.quoted(name, Some(v.as_deref().unwrap_or("")), comment),
        // LabelType is printed with KeyQuotedString, all other keywords with KeyString
        Val::Kw(Some(v)) if item.ty == Ty::Label => w.quoted(name, Some(v), comment),
        Val::Kw(Some(v)) => w.unquoted(name, Some(v), comment),
        Val::Kw(None) => {}
        Val::Level(Some(v)) => w.unquoted(name, Some(v), comment),
        Val::Level(None) => w.unquoted(name, Some(""), true),
        Val::PoolType(v) => w.unquoted(name, Some(v), comment),
        Val::Password(Some(p)) => w.quoted(name, Some(p), comment),
        Val::Password(None) => {}
        Val::Runs(r) => {
            if !r.is_empty() {
                w.one_per_line("Run", r, comment, false, false);
            }
        }
        Val::Addresses(addrs) => {
            let line = format!("{}{name} = {{\n", w.prefix(comment));
            w.text.push_str(&line);
            w.indent += 1;
            w.push_arr(name);
            for a in addrs {
                w.push_obj("");
                let line = format!("{}{} = {{\n", w.prefix(comment), a.family);
                w.text.push_str(&line);
                w.indent += 1;
                w.push_obj(a.family);
                w.unquoted("addr", Some(&a.addr), comment);
                w.unsigned("port", a.port, comment);
                w.pop();
                w.indent -= 1;
                let line = format!("{}}}\n", w.prefix(comment));
                w.text.push_str(&line);
                w.pop();
            }
            w.pop();
            w.indent -= 1;
            let line = format!("{}}}\n", w.prefix(comment));
            w.text.push_str(&line);
        }
        Val::Nothing => {}
    }
}

// --- sizes, durations -------------------------------------------------------

const SIZE_MODS: &[(&str, u64)] = &[
    ("*", 1), ("k", 1024), ("kb", 1000), ("m", 1_048_576), ("mb", 1_000_000),
    ("g", 1_073_741_824), ("gb", 1_000_000_000), ("t", 1_099_511_627_776), ("tb", 1_000_000_000_000),
    ("p", 1_125_899_906_842_624), ("pb", 1_000_000_000_000_000),
    ("e", 1_152_921_504_606_846_976), ("eb", 1_000_000_000_000_000_000),
];
const SPEED_MODS: &[(&str, u64)] = &[("*", 1), ("k/s", 1024), ("kb/s", 1000), ("m/s", 1_048_576), ("mb/s", 1_000_000)];
const TIME_MODS: &[(&str, u64)] = &[
    ("n", 60), ("seconds", 1), ("months", 2_592_000), ("minutes", 60), ("mins", 60), ("hours", 3600),
    ("days", 86_400), ("weeks", 604_800), ("quarters", 7_862_400), ("years", 31_536_000),
];

/// lib/edit.cc parse_number_with_mod: "1 g", "30 days 2 hours", "10mb/s"
fn parse_with_mods(s: &str, mods: &[(&str, u64)]) -> Option<u64> {
    let mut rest = s.trim_start();
    let mut total: u64 = 0;
    let mut any = false;
    while !rest.is_empty() {
        let num_len = rest.char_indices()
            .take_while(|(i, c)| c.is_ascii_digit() || *c == '.' || (*i == 0 && (*c == '+' || *c == '-')))
            .count();
        if num_len == 0 {
            break;
        }
        let number: f64 = rest[..num_len].parse().ok()?;
        rest = rest[num_len..].trim_start();
        let mod_len = rest.chars().take_while(|c| !c.is_ascii_digit() && !c.is_whitespace()).count();
        let modifier = &rest[..mod_len];
        rest = rest[mod_len..].trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        any = true;
        if modifier.is_empty() {
            total = total.saturating_add(number as u64);
        } else {
            let m = mods.iter().find(|(name, _)| {
                name.len() >= modifier.len() && name[..modifier.len()].eq_ignore_ascii_case(modifier)
            })?;
            total = total.saturating_add((number * m.1 as f64) as u64);
        }
    }
    if any && rest.is_empty() { Some(total) } else { None }
}

fn parse_units(s: &str, mods: &[(&str, u64)]) -> Option<u64> {
    parse_with_mods(s, mods)
}

/// lib/edit.cc size_to_uint64 (C-011 update dialogs).
pub(crate) fn parse_size(s: &str) -> Option<u64> {
    parse_with_mods(s, SIZE_MODS)
}

pub(crate) fn parse_duration(s: &str) -> Option<u64> {
    parse_with_mods(s, TIME_MODS)
}

/// lib/edit.cc SizeAsSiPrefixFormat: 10000000 -> "9 m 549 k 640"
fn size_si(v: u64) -> String {
    if v == 0 {
        return "0".into();
    }
    const M: &[(&str, u64)] = &[
        (" e", 1_152_921_504_606_846_976), (" p", 1_125_899_906_842_624), (" t", 1_099_511_627_776),
        (" g", 1_073_741_824), (" m", 1_048_576), (" k", 1024), ("", 1),
    ];
    let mut value = v;
    let mut out = String::new();
    for (suffix, mult) in M {
        if value == 0 {
            break;
        }
        let factor = value / mult;
        value %= mult;
        if factor > 0 {
            out.push_str(&factor.to_string());
            out.push_str(suffix);
            if value > 0 {
                out.push(' ');
            }
        }
    }
    out
}

/// lib/res.cc PrintConfigTime: 2592000 -> "1 months "
fn time_spec(secs: u64) -> String {
    if secs == 0 {
        return "0".into();
    }
    const M: &[(&str, u64)] = &[
        ("years", 31_536_000), ("months", 2_592_000), ("weeks", 604_800), ("days", 86_400),
        ("hours", 3600), ("minutes", 60), ("seconds", 1),
    ];
    let mut s = secs;
    let mut out = String::new();
    for (name, mult) in M {
        let factor = s / mult;
        s %= mult;
        if factor > 0 {
            out.push_str(&format!("{factor} {name} "));
        }
        if s == 0 {
            break;
        }
    }
    out
}

// --- director addresses -----------------------------------------------------

fn director_addresses(res: &ConfResource) -> Vec<Addr> {
    let get = |keys: &[&str]| res.directives.iter().rev()
        .find(|d| keys.contains(&d.norm_key().as_str()));
    let port_of = |d: Option<&Directive>| d.and_then(|d| d.first()).and_then(|p| p.parse().ok());
    let default_port: u64 = port_of(get(&["port", "dirport"])).unwrap_or(9101);

    if let Some(block) = get(&["addresses", "diraddresses"]) {
        let mut out = Vec::new();
        for t in &block.tokens {
            let Tok::Block(entries) = t else { continue };
            for e in entries {
                let Some(Tok::Block(inner)) = e.tokens.first() else { continue };
                let addr = inner.iter().find(|d| d.norm_key() == "addr").and_then(|d| d.first());
                let port = inner.iter().find(|d| d.norm_key() == "port").and_then(|d| d.first())
                    .and_then(|p| p.parse().ok()).unwrap_or(9101);
                let family = match e.norm_key().as_str() {
                    "ipv6" => "ipv6",
                    "ipv4" => "ipv4",
                    _ => if addr.is_some_and(|a| a.contains(':')) { "ipv6" } else { "ipv4" },
                };
                let addr = addr.map(str::to_string)
                    .unwrap_or_else(|| if family == "ipv6" { "::".into() } else { "0.0.0.0".into() });
                out.push(Addr { family, addr, port });
            }
        }
        return out;
    }
    if let Some(a) = get(&["address", "diraddress"]).and_then(|d| d.first()) {
        let family = if a.contains(':') { "ipv6" } else { "ipv4" };
        return vec![Addr { family, addr: a.to_string(), port: default_port }];
    }
    vec![
        Addr { family: "ipv4", addr: "0.0.0.0".into(), port: default_port },
        Addr { family: "ipv6", addr: "::".into(), port: default_port },
    ]
}

// --- runscripts ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Script {
    command: String,
    console: bool,
    short_form: bool,
    /// None = never
    when: Option<&'static str>,
    on_success: bool,
    on_failure: bool,
    fail_on_error: bool,
    target: String,
    short_name: Option<&'static str>,
}

fn runscripts(res: &ConfResource) -> Vec<Script> {
    let mut out = Vec::new();
    for d in &res.directives {
        let key = d.norm_key();
        let short = match key.as_str() {
            "runbeforejob" => Some("RunBeforeJob"),
            "runafterjob" => Some("RunAfterJob"),
            "runafterfailedjob" => Some("RunAfterFailedJob"),
            "clientrunbeforejob" => Some("ClientRunBeforeJob"),
            "clientrunafterjob" => Some("ClientRunAfterJob"),
            _ => None,
        };
        if let Some(name) = short {
            let mut s = Script {
                command: d.first().unwrap_or("").to_string(), console: false, short_form: true,
                when: None, on_success: true, on_failure: false, fail_on_error: true,
                target: String::new(), short_name: None,
            };
            match name {
                "RunBeforeJob" => s.when = Some("before"),
                "RunAfterJob" => { s.when = Some("after"); s.fail_on_error = false; }
                "ClientRunAfterJob" => { s.when = Some("after"); s.fail_on_error = false; s.target = "%c".into(); }
                "ClientRunBeforeJob" => { s.when = Some("before"); s.target = "%c".into(); }
                _ => { s.when = Some("after"); s.on_failure = true; s.on_success = false; s.fail_on_error = false; }
            }
            s.short_name = short_form_name(&s);
            out.push(s);
        } else if key == "runscript" {
            let Some(body) = &d.block else { continue };
            let mut base = Script {
                command: String::new(), console: false, short_form: false, when: None,
                on_success: true, on_failure: false, fail_on_error: true, target: "%c".into(), short_name: None,
            };
            let mut cmds = Vec::new();
            for b in body {
                let v = b.first().unwrap_or("");
                match b.norm_key().as_str() {
                    "command" => cmds.push((v.to_string(), false)),
                    "console" => cmds.push((v.to_string(), true)),
                    "target" | "runsonclient" => {
                        base.target = if v == "%c" { v.into() } else if v.eq_ignore_ascii_case("yes") {
                            "%c".into()
                        } else if v.eq_ignore_ascii_case("no") { String::new() } else { v.into() };
                    }
                    "runsonsuccess" => base.on_success = parse_bool(v).unwrap_or(base.on_success),
                    "runsonfailure" => base.on_failure = parse_bool(v).unwrap_or(base.on_failure),
                    "failjobonerror" | "abortjobonerror" => base.fail_on_error = parse_bool(v).unwrap_or(base.fail_on_error),
                    "runswhen" => base.when = match v.to_ascii_lowercase().as_str() {
                        "before" => Some("before"),
                        "after" => Some("after"),
                        "aftervss" => Some("aftervss"),
                        "always" => Some("always"),
                        _ => base.when,
                    },
                    _ => {}
                }
            }
            for (cmd, console) in cmds {
                let mut s = base.clone();
                s.command = cmd;
                s.console = console;
                if console {
                    s.target.clear();
                }
                out.push(s);
            }
        }
    }
    out
}

fn short_form_name(s: &Script) -> Option<&'static str> {
    let before = s.when == Some("before");
    let after = s.when == Some("after");
    if before && s.target.is_empty() {
        Some("RunBeforeJob")
    } else if after && s.on_success && !s.on_failure && !s.fail_on_error && s.target.is_empty() {
        Some("RunAfterJob")
    } else if after && s.on_success && !s.on_failure && !s.fail_on_error && !s.target.is_empty() {
        Some("ClientRunAfterJob")
    } else if before && !s.target.is_empty() {
        Some("ClientRunBeforeJob")
    } else if after && s.on_failure && !s.on_success && !s.fail_on_error && s.target.is_empty() {
        Some("RunAfterFailedJob")
    } else {
        None
    }
}

/// dird_conf.cc PrintConfigRunscript (only for the "RunScript" item)
fn print_runscripts(item: &Item, scripts: &[(Script, bool)], inherited: bool, verbose: bool, w: &mut W) {
    // RUNSCRIPT HasDefaultValue: no scripts at all
    let print = !scripts.is_empty();
    if inherited && !verbose {
        return;
    }
    if !print {
        return;
    }
    w.push_arr(item.name);
    for (s, from_jobdef) in scripts {
        let mut comment = inherited;
        if *from_jobdef {
            if verbose {
                comment = true;
            } else {
                continue;
            }
        }
        let esc = escape_string(&s.command);
        if s.short_form {
            w.push_obj("");
            if let Some(name) = s.short_name {
                w.quoted(name, Some(&esc), comment);
            }
            w.pop();
        } else {
            w.sub_start("RunScript", comment);
            w.quoted(if s.console { "Console" } else { "Command" }, Some(&esc), comment);
            match s.when {
                Some(when) => w.quoted("RunsWhen", Some(when), comment),
                None if verbose => w.quoted("RunsWhen", Some("never"), true),
                None => {}
            }
            if !s.fail_on_error {
                w.boolean("FailJobOnError", false, comment);
            } else if verbose {
                w.boolean("FailJobOnError", true, true);
            }
            if !s.on_success {
                w.boolean("RunsOnSuccess", false, comment);
            } else if verbose {
                w.boolean("RunsOnSuccess", true, true);
            }
            if s.on_failure {
                w.boolean("RunsOnFailure", true, comment);
            } else if verbose {
                w.boolean("RunsOnFailure", false, true);
            }
            if !s.console {
                if s.target.is_empty() {
                    w.boolean("RunsOnClient", false, comment);
                } else if verbose {
                    w.boolean("RunsOnClient", true, true);
                }
            }
            w.sub_end(comment);
        }
    }
    w.pop();
}

// --- schedule run -------------------------------------------------------------

pub(crate) struct RunSpec {
    pub(crate) pool: Option<String>,
    full_pool: Option<String>,
    inc_pool: Option<String>,
    diff_pool: Option<String>,
    next_pool: Option<String>,
    pub(crate) level: Option<char>,
    pub(crate) storage: Option<String>,
    msgs: Option<String>,
    priority: u32,
    maxrunschedtime: u64,
    accurate: bool,
    pub(crate) hour: [bool; 24],
    pub(crate) mday: [bool; 31],
    pub(crate) wday: [bool; 7],
    pub(crate) month: [bool; 12],
    pub(crate) wom: [bool; 5],
    pub(crate) woy: [bool; 54],
    pub(crate) last_7days: bool,
    pub(crate) minute: u32,
}

const RUN_KEYWORDS: &[(&str, char, usize)] = &[
    ("on", 'n', 0), ("at", 'a', 0), ("last", 'l', 0),
    ("sun", 'w', 0), ("mon", 'w', 1), ("tue", 'w', 2), ("wed", 'w', 3), ("thu", 'w', 4), ("fri", 'w', 5), ("sat", 'w', 6),
    ("jan", 'm', 0), ("feb", 'm', 1), ("mar", 'm', 2), ("apr", 'm', 3), ("may", 'm', 4), ("jun", 'm', 5),
    ("jul", 'm', 6), ("aug", 'm', 7), ("sep", 'm', 8), ("oct", 'm', 9), ("nov", 'm', 10), ("dec", 'm', 11),
    ("sunday", 'w', 0), ("monday", 'w', 1), ("tuesday", 'w', 2), ("wednesday", 'w', 3), ("thursday", 'w', 4),
    ("friday", 'w', 5), ("saturday", 'w', 6),
    ("january", 'm', 0), ("february", 'm', 1), ("march", 'm', 2), ("april", 'm', 3), ("june", 'm', 5),
    ("july", 'm', 6), ("august", 'm', 7), ("september", 'm', 8), ("october", 'm', 9), ("november", 'm', 10),
    ("december", 'm', 11),
    ("daily", 'D', 0), ("weekly", 'W', 0), ("monthly", 'M', 0), ("hourly", 'H', 0),
    ("1st", 'o', 0), ("2nd", 'o', 1), ("3rd", 'o', 2), ("4th", 'o', 3), ("5th", 'o', 4),
    ("first", 'o', 0), ("second", 'o', 1), ("third", 'o', 2), ("fourth", 'o', 3), ("fifth", 'o', 4),
];

fn run_kw(s: &str) -> Option<(char, usize)> {
    RUN_KEYWORDS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(s)).map(|(_, st, c)| (*st, *c))
}

fn set_range(bits: &mut [bool], from: usize, to: usize) {
    for b in bits.iter_mut().take(to + 1).skip(from) {
        *b = true;
    }
}

/// run_conf.cc StoreRun (schedule part) + dird_conf.cc PrintConfigRun
fn run_string(tokens: &[Tok]) -> String {
    print_run(&parse_run(tokens))
}

/// run_conf.cc StoreRun: modifiers and the date/time mask of one `Run =`.
pub(crate) fn parse_run(tokens: &[Tok]) -> RunSpec {
    let words: Vec<(Option<&str>, bool)> = tokens.iter().map(|t| match t {
        Tok::Word(s) | Tok::Quoted(s) => (Some(s.as_str()), false),
        Tok::Eq => (None, true),
        _ => (None, false),
    }).collect();
    let mut r = RunSpec {
        pool: None, full_pool: None, inc_pool: None, diff_pool: None, next_pool: None, level: None,
        storage: None, msgs: None, priority: 0, maxrunschedtime: 0, accurate: false,
        hour: [true; 24], mday: [true; 31], wday: [true; 7], month: [true; 12],
        wom: [true; 5], woy: [true; 54], last_7days: false, minute: 0,
    };

    // modifiers: Level=Full Pool=... or a bare level name
    let mut i = 0;
    while i < words.len() {
        let Some(w) = words[i].0 else { i += 1; continue };
        let has_eq = words.get(i + 1).is_some_and(|x| x.1);
        let field = w.to_ascii_lowercase();
        if has_eq && ["pool", "fullpool", "incrementalpool", "differentialpool", "nextpool", "level", "storage",
            "messages", "priority", "spooldata", "maxrunschedtime", "accurate"].contains(&field.as_str()) {
            let v = words.get(i + 2).and_then(|x| x.0).unwrap_or("");
            match field.as_str() {
                "pool" => r.pool = Some(v.into()),
                "fullpool" => r.full_pool = Some(v.into()),
                "incrementalpool" => r.inc_pool = Some(v.into()),
                "differentialpool" => r.diff_pool = Some(v.into()),
                "nextpool" => r.next_pool = Some(v.into()),
                "level" => r.level = JOB_LEVELS.iter().find(|(n, _)| n.eq_ignore_ascii_case(v)).map(|(_, c)| *c),
                "storage" => r.storage = Some(v.into()),
                "messages" => r.msgs = Some(v.into()),
                "priority" => r.priority = v.parse().unwrap_or(0),
                "maxrunschedtime" => r.maxrunschedtime = parse_duration(v).unwrap_or(0),
                "accurate" => r.accurate = parse_bool(v).unwrap_or(false),
                _ => {}
            }
            i += 3;
            continue;
        }
        if let Some((_, c)) = JOB_LEVELS.iter().find(|(n, _)| n.eq_ignore_ascii_case(w)) {
            r.level = Some(*c);
            i += 1;
            continue;
        }
        break;
    }

    // schedule time spec
    let (mut have_hour, mut have_mday, mut have_wday, mut have_month, mut have_wom, mut have_woy, mut have_at) =
        (false, false, false, false, false, false, false);
    for (w, _) in &words[i..] {
        let Some(w) = w else { continue };
        let lw = w.to_ascii_lowercase();
        if !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()) {
            let code: usize = w.parse::<usize>().unwrap_or(0).saturating_sub(1);
            if code > 30 { continue; }
            if !have_mday { r.mday = [false; 31]; have_mday = true; }
            r.mday[code] = true;
        } else if let Some((a, b)) = lw.split_once('-') {
            // ranges: days, weeks of year, weekdays/months/positions
            if let (Ok(x), Ok(y)) = (a.parse::<usize>(), b.parse::<usize>()) {
                let (x, y) = (x.saturating_sub(1), y.saturating_sub(1));
                if x > 30 || y > 30 { continue; }
                if !have_mday { r.mday = [false; 31]; have_mday = true; }
                if x < y { set_range(&mut r.mday, x, y) } else { set_range(&mut r.mday, x, 30); set_range(&mut r.mday, 0, y) }
            } else if a.len() == 3 && b.len() == 3 && a.starts_with('w') && b.starts_with('w') {
                let (Ok(x), Ok(y)) = (a[1..].parse::<usize>(), b[1..].parse::<usize>()) else { continue };
                if x > 53 || y > 53 { continue; }
                if !have_woy { r.woy = [false; 54]; have_woy = true; }
                if x < y { set_range(&mut r.woy, x, y) } else { set_range(&mut r.woy, x, 53); set_range(&mut r.woy, 0, y) }
            } else if let (Some((s1, c1)), Some((s2, c2))) = (run_kw(a), run_kw(b)) {
                if s1 != s2 || c1 == c2 { continue; }
                let (bits, have, max): (&mut [bool], &mut bool, usize) = match s1 {
                    'w' => (&mut r.wday, &mut have_wday, 6),
                    'm' => (&mut r.month, &mut have_month, 11),
                    'o' => (&mut r.wom, &mut have_wom, 4),
                    _ => continue,
                };
                if !*have { bits.iter_mut().for_each(|b| *b = false); *have = true; }
                if c1 < c2 { set_range(bits, c1, c2) } else { set_range(bits, c1, max); set_range(bits, 0, c2) }
            }
        } else if let Some((h, m)) = lw.split_once(':') {
            if !have_at { continue; }
            if !have_hour { r.hour = [false; 24]; }
            let mut hour: u32 = h.parse().unwrap_or(0);
            let digits: String = m.chars().take_while(char::is_ascii_digit).collect();
            let minute: u32 = digits.parse().unwrap_or(0);
            let suffix = if m.len() >= 2 { &m[2..] } else { "" };
            if suffix.eq_ignore_ascii_case("pm") {
                if hour != 12 { hour += 12; }
            } else if suffix.eq_ignore_ascii_case("am") {
                if hour == 12 { hour = 0; }
            } else if m.len() != 2 {
                continue;
            }
            if hour > 23 || minute > 59 { continue; }
            r.hour[hour as usize] = true;
            r.minute = minute;
            have_hour = true;
        } else if let Some((a, b)) = lw.split_once('/') {
            if let (Ok(x), Ok(y)) = (a.parse::<usize>(), b.parse::<usize>()) {
                let x = x.saturating_sub(1);
                if x > 30 || y > 30 || x > y || y == 0 { continue; }
                if !have_mday { r.mday = [false; 31]; have_mday = true; }
                for d in 0..31 {
                    if d % y == 0 && d + x < 31 { r.mday[d + x] = true; }
                }
            } else if a.len() == 3 && b.len() == 3 && a.starts_with('w') && b.starts_with('w') {
                let (Ok(x), Ok(y)) = (a[1..].parse::<usize>(), b[1..].parse::<usize>()) else { continue };
                if x > 53 || y == 0 || y > 53 || x > y { continue; }
                if !have_woy { r.woy = [false; 54]; have_woy = true; }
                let mut wk = x;
                while wk <= 53 { r.woy[wk] = true; wk += y; }
            }
        } else if lw.len() == 3 && lw.starts_with('w') && lw[1..].bytes().all(|b| b.is_ascii_digit()) {
            let code: usize = lw[1..].parse().unwrap_or(0);
            if code > 53 { continue; }
            if !have_woy { r.woy = [false; 54]; have_woy = true; }
            r.woy[code] = true;
        } else if let Some((state, code)) = run_kw(&lw) {
            match state {
                'w' => { if !have_wday { r.wday = [false; 7]; have_wday = true; } r.wday[code] = true; }
                'm' => { if !have_month { r.month = [false; 12]; have_month = true; } r.month[code] = true; }
                'o' => { if !have_wom { r.wom = [false; 5]; have_wom = true; } r.wom[code] = true; }
                'a' => have_at = true,
                'l' => { r.last_7days = true; if !have_wom { r.wom = [false; 5]; have_wom = true; } }
                'H' => { have_hour = true; r.hour = [true; 24]; }
                'W' => { have_mday = true; have_wom = true; have_woy = true; r.mday = [true; 31]; r.wom = [true; 5]; r.woy = [true; 54]; }
                'D' => { have_mday = true; r.wday = [true; 7]; }
                'M' => { have_month = true; r.month = [true; 12]; }
                _ => {}
            }
        }
    }
    let _ = (have_mday, have_month);
    r
}

fn print_run(r: &RunSpec) -> String {
    const WEEKDAYS: &[&str] = &["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: &[&str] = &["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    const ORDINALS: &[&str] = &["1st", "2nd", "3rd", "4th", "5th"];
    let mut s = String::new();
    for (key, v) in [("pool", &r.pool), ("fullpool", &r.full_pool), ("incrementalpool", &r.inc_pool),
                     ("differentialpool", &r.diff_pool), ("nextpool", &r.next_pool)] {
        if let Some(p) = v {
            s.push_str(&format!("{key}=\"{p}\" "));
        }
    }
    if let Some(l) = r.level {
        if let Some((n, _)) = JOB_LEVELS.iter().find(|(_, c)| *c == l) {
            s.push_str(n);
            s.push(' ');
        }
    }
    if let Some(st) = &r.storage { s.push_str(&format!("storage=\"{st}\" ")); }
    if let Some(m) = &r.msgs { s.push_str(&format!("messages=\"{m}\" ")); }
    if r.priority != 0 && r.priority != 10 { s.push_str(&format!("priority={} ", r.priority)); }
    if r.maxrunschedtime != 0 { s.push_str(&format!("maxrunschedtime={} ", r.maxrunschedtime)); }
    if r.accurate { s.push_str("accurate=\"yes\" "); }

    let ranges = |bits: &[bool], label: &dyn Fn(usize) -> String| -> Option<String> {
        if bits.iter().all(|b| *b) {
            return None;
        }
        let n = bits.len();
        let mut temp = String::new();
        let mut start: Option<usize> = None;
        for i in 0..n {
            if bits[i] && start.is_none() {
                start = Some(i);
                temp.push(',');
                temp.push_str(&label(i));
            }
            if !bits[i] {
                if let Some(st) = start {
                    if i - st > 1 {
                        temp.push('-');
                        temp.push_str(&label(i - 1));
                    }
                    start = None;
                }
            }
        }
        if let Some(st) = start {
            if bits[n - 1] && n - 1 - st > 1 {
                temp.push('-');
                temp.push_str(&label(n - 1));
            }
        }
        temp.push(' ');
        Some(temp[1..].to_string())
    };

    // mday: the C++ closes an interval with the *unset* position (i), i.e. day i
    if let Some(t) = ranges_mday(&r.mday) { s.push_str(&t); }
    if let Some(t) = ranges(&r.wom, &|i| ORDINALS[i].to_string()) { s.push_str(&t); }
    if r.last_7days { s.push_str("last "); }
    if let Some(t) = ranges(&r.wday, &|i| WEEKDAYS[i].to_string()) { s.push_str(&t); }
    if let Some(t) = ranges(&r.month, &|i| MONTHS[i].to_string()) { s.push_str(&t); }
    if let Some(t) = ranges(&r.woy, &|i| format!("w{i:02}")) { s.push_str(&t); }
    for (h, set) in r.hour.iter().enumerate() {
        if *set {
            s.push_str(&format!("at {h:02}:{:02}", r.minute));
        }
    }
    s
}

/// mday printing: "-%d" with i (the first unset index) == last set day number
fn ranges_mday(bits: &[bool; 31]) -> Option<String> {
    if bits.iter().all(|b| *b) {
        return None;
    }
    let mut temp = String::new();
    let mut start: Option<usize> = None;
    for i in 0..31 {
        if bits[i] && start.is_none() {
            start = Some(i);
            temp.push_str(&format!(",{}", i + 1));
        }
        if !bits[i] {
            if let Some(st) = start {
                if i - st > 1 {
                    temp.push_str(&format!("-{i}"));
                }
                start = None;
            }
        }
    }
    if let Some(st) = start {
        if bits[30] && 30 - st > 1 {
            temp.push_str("-31");
        }
    }
    temp.push(' ');
    Some(temp[1..].to_string())
}

// --- messages -------------------------------------------------------------------

/// lib/generic_res.h msg_types (name, M_ code), printing order
const MSG_TYPES: &[(&str, u32)] = &[
    ("debug", 2), ("abort", 1), ("fatal", 3), ("error", 4), ("warning", 5), ("info", 6),
    ("saved", 7), ("notsaved", 8), ("skipped", 9), ("mount", 10), ("Terminate", 12),
    ("restored", 13), ("security", 14), ("alert", 15), ("volmgmt", 16), ("audit", 17),
];
const M_MAX: u32 = 17;

pub(crate) struct Dest {
    pub(crate) code: &'static str,
    pub(crate) where_: String,
    pub(crate) types: u32, // bit mask over M_ codes (bit n = code n)
}

fn print_messages(res: &ConfResource, verbose: bool, w: &mut W) {
    w.quoted("Name", res.name(), false);
    for (key, label) in [("mailcommand", "MailCommand"), ("operatorcommand", "OperatorCommand"),
                         ("timestampformat", "TimestampFormat")] {
        if let Some(v) = res.directives.iter().rev().find(|d| d.norm_key() == key).and_then(|d| d.first()) {
            if !v.is_empty() {
                w.quoted(label, Some(&escape_string(v)), false);
            }
        }
    }

    let chain = destinations(res);

    for e in &chain {
        let what = message_types(e.types, verbose);
        let has_where = matches!(e.code, "mail" | "file" | "append" | "director" | "operator" | "mailonerror" | "mailonsuccess");
        let mut line = format!("{}{}", w.prefix(false), e.code);
        let mut obj = Map::new();
        if has_where {
            line.push_str(&format!(" = {}", e.where_));
            obj.insert("where".into(), json!(e.where_));
        }
        line.push_str(&format!(" = {what}\n"));
        obj.insert("what".into(), json!(what));
        w.text.push_str(&line);
        w.put(e.code, Value::Object(obj));
    }
}

/// Destination chain of a Messages resource (lib/msg_res.cc StoreMsgs).
pub(crate) fn destinations(res: &ConfResource) -> Vec<Dest> {
    let mut chain: Vec<Dest> = Vec::new();
    for d in &res.directives {
        let key = d.norm_key();
        let code: &'static str = match key.as_str() {
            "syslog" => "syslog", "mail" => "mail", "mailonerror" => "mailonerror",
            "mailonsuccess" => "mailonsuccess", "file" => "file", "append" => "append",
            "stdout" => "stdout", "stderr" => "stderr", "director" => "director",
            "console" => "console", "operator" => "operator", "catalog" => "catalog",
            _ => continue,
        };
        // split tokens at the first '=' (destination = types)
        let eq = d.tokens.iter().position(|t| *t == Tok::Eq);
        let (dest_toks, type_toks) = match (code, eq) {
            ("stdout" | "stderr" | "console" | "catalog", _) | (_, None) => (&d.tokens[..0], &d.tokens[..]),
            (_, Some(p)) => (&d.tokens[..p], &d.tokens[p + 1..]),
        };
        let where_ = match code {
            "syslog" => String::new(),
            "file" | "append" => dest_toks.iter().find_map(Tok::text).unwrap_or("").to_string(),
            _ => dest_toks.iter().filter_map(Tok::text).collect::<Vec<_>>().join(" "),
        };
        for t in type_toks.iter().filter_map(Tok::text) {
            let (not, name) = match t.strip_prefix('!') {
                Some(n) => (true, n),
                None => (false, t),
            };
            if name.eq_ignore_ascii_case("all") {
                for m in 1..=M_MAX {
                    add_dest(&mut chain, code, &where_, m);
                }
                continue;
            }
            let Some((_, m)) = MSG_TYPES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else { continue };
            if not {
                if let Some(e) = chain.iter_mut()
                    .find(|e| e.code == code && e.where_ == where_ && e.types & (1 << m) != 0) {
                    e.types &= !(1 << m);
                }
            } else {
                add_dest(&mut chain, code, &where_, *m);
            }
        }
    }
    chain
}

fn add_dest(chain: &mut Vec<Dest>, code: &'static str, where_: &str, m: u32) {
    if let Some(e) = chain.iter_mut().rev().find(|e| e.code == code && e.where_ == where_) {
        e.types |= 1 << m;
    } else {
        chain.push(Dest { code, where_: where_.to_string(), types: 1 << m });
    }
}

/// MessagesResource::GetMessageTypesAsSring
fn message_types(mask: u32, verbose: bool) -> String {
    let mut set = Vec::new();
    let mut unset = Vec::new();
    for (name, m) in MSG_TYPES {
        if mask & (1 << m) != 0 {
            set.push(name.to_string());
        } else {
            unset.push(format!("!{name}"));
        }
    }
    if verbose {
        let mut v = set;
        v.extend(unset);
        v.join(",")
    } else if set.len() > unset.len() {
        let mut v = vec!["all".to_string()];
        v.extend(unset);
        v.join(",")
    } else {
        set.join(",")
    }
}

// --- filesets ---------------------------------------------------------------------

/// inc_conf.cc FS_option_kw + FS_options: keyword -> value -> option letters
fn fs_option(keyword: &str, value: &str) -> Option<String> {
    let v = value.to_ascii_lowercase();
    let pick = |pairs: &[(&str, &str)]| pairs.iter().find(|(n, _)| *n == v).map(|(_, o)| o.to_string());
    match keyword {
        "signature" => pick(&[("md5", "M"), ("sha1", "S"), ("sha256", "S2"), ("sha512", "S3"), ("xxh128", "S4")]),
        "compression" => pick(&[("gzip", "Z6"), ("gzip1", "Z1"), ("gzip2", "Z2"), ("gzip3", "Z3"), ("gzip4", "Z4"),
            ("gzip5", "Z5"), ("gzip6", "Z6"), ("gzip7", "Z7"), ("gzip8", "Z8"), ("gzip9", "Z9"), ("lzo", "Zo"),
            ("lzfast", "Zff"), ("lz4", "Zf4"), ("lz4hc", "Zfh")]),
        "encryption" => pick(&[("blowfish", "Eb"), ("3des", "E3"), ("aes128", "Ea1"), ("aes192", "Ea2"),
            ("aes256", "Ea3"), ("camellia128", "Ec1"), ("camellia192", "Ec2"), ("camellia256", "Ec3"),
            ("aes128hmacsha1", "Eh1"), ("aes256hmacsha1", "Eh2")]),
        "onefs" => pick(&[("yes", "0"), ("no", "f")]),
        "recurse" => pick(&[("yes", "0"), ("no", "h")]),
        "sparse" => pick(&[("yes", "s"), ("no", "0")]),
        "hardlinks" => pick(&[("yes", "0"), ("no", "H")]),
        "replace" => pick(&[("always", "a"), ("ifnewer", "w"), ("never", "n")]),
        "readfifo" => pick(&[("yes", "r"), ("no", "0")]),
        "portable" => pick(&[("yes", "p"), ("no", "0")]),
        "mtimeonly" => pick(&[("yes", "m"), ("no", "0")]),
        "keepatime" => pick(&[("yes", "k"), ("no", "0")]),
        "exclude" => pick(&[("yes", "e"), ("no", "0")]),
        "aclsupport" => pick(&[("yes", "A"), ("no", "0")]),
        "ignorecase" => pick(&[("yes", "i"), ("no", "0")]),
        "hfsplussupport" => pick(&[("yes", "R"), ("no", "0")]),
        "noatime" => pick(&[("yes", "K"), ("no", "0")]),
        // C++ maps EnhancedWild=yes to "K" as well
        "enhancedwild" => pick(&[("yes", "K"), ("no", "0")]),
        "checkfilechanges" => pick(&[("yes", "c"), ("no", "0")]),
        "honornodumpflag" => pick(&[("yes", "N"), ("no", "0")]),
        "xattrsupport" => pick(&[("yes", "X"), ("no", "0")]),
        "shadowing" => pick(&[("localwarn", "d1"), ("localremove", "d2"), ("globalwarn", "d3"),
            ("globalremove", "d4"), ("none", "0")]),
        "autoexclude" => pick(&[("yes", "0"), ("no", "x")]),
        "forceencryption" => pick(&[("yes", "Ef"), ("no", "0")]),
        "verify" => Some(format!("V{value}:")),
        "accurate" => Some(format!("C{value}:")),
        "strippath" => Some(format!("P{value}:")),
        "size" => Some(format!("z{value}:")),
        _ => None,
    }
}

#[derive(Default)]
pub(crate) struct FileOptions {
    pub(crate) opts: String,
    pub(crate) regex: Vec<String>,
    pub(crate) regexdir: Vec<String>,
    pub(crate) regexfile: Vec<String>,
    pub(crate) wild: Vec<String>,
    pub(crate) wilddir: Vec<String>,
    pub(crate) wildfile: Vec<String>,
    pub(crate) wildbase: Vec<String>,
    pub(crate) fstype: Vec<String>,
    pub(crate) drivetype: Vec<String>,
    pub(crate) meta: Vec<String>,
    pub(crate) plugin: Option<String>,
}

pub(crate) fn default_options(configured: &[&str]) -> String {
    // OptionsDefaultValues (std::map ordered by keyword id): HardLinks, AclSupport, XattrSupport
    let mut s = String::new();
    for (kw, letter) in [("hardlinks", "H"), ("aclsupport", "A"), ("xattrsupport", "X")] {
        if !configured.contains(&kw) {
            s.push_str(letter);
        }
    }
    s
}

pub(crate) fn parse_options(body: &[Directive]) -> FileOptions {
    let mut fo = FileOptions::default();
    let mut configured = Vec::new();
    for d in body {
        let key = d.norm_key();
        let v = d.first().unwrap_or("").to_string();
        match key.as_str() {
            "regex" => fo.regex.push(v),
            "regexdir" => fo.regexdir.push(v),
            "regexfile" => fo.regexfile.push(v),
            "wild" => fo.wild.push(v),
            "wilddir" => fo.wilddir.push(v),
            "wildfile" => if v.contains('/') || v.contains('\\') { fo.wildfile.push(v) } else { fo.wildbase.push(v) },
            "fstype" => fo.fstype.extend(d.values().into_iter().map(str::to_string)),
            "drivetype" => fo.drivetype.extend(d.values().into_iter().map(str::to_string)),
            "meta" => fo.meta.extend(d.values().into_iter().map(str::to_string)),
            "plugin" => fo.plugin = Some(v),
            _ => {
                if let Some(letters) = fs_option(&key, &v) {
                    if ["hardlinks", "aclsupport", "xattrsupport"].contains(&key.as_str()) {
                        configured.push(match key.as_str() { "hardlinks" => "hardlinks", "aclsupport" => "aclsupport", _ => "xattrsupport" });
                    }
                    fo.opts.push_str(&letters);
                }
            }
        }
    }
    fo.opts.push_str(&default_options(&configured));
    fo
}

fn option_value(chars: &[char], i: &mut usize) -> String {
    let mut v = String::new();
    *i += 1;
    while *i < chars.len() && chars[*i] != ':' {
        v.push(chars[*i]);
        *i += 1;
    }
    v
}

/// FilesetResource::PrintConfigIncludeExcludeOptions
fn print_options(fo: &FileOptions, w: &mut W) {
    w.sub_start("Options", false);
    let c: Vec<char> = fo.opts.chars().collect();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '0' => {}
            'a' => w.quoted("Replace", Some("Always"), false),
            'C' => { let v = option_value(&c, &mut i); w.quoted("Accurate", Some(&v), false); }
            'c' => w.boolean("CheckFileChanges", true, false),
            'd' => {
                let s = match c.get(i + 1) {
                    Some('1') => Some("LocalWarn"), Some('2') => Some("LocalRemove"),
                    Some('3') => Some("GlobalWarn"), Some('4') => Some("GlobalRemove"), _ => None,
                };
                if let Some(s) = s { w.quoted("Shadowing", Some(s), false); }
                i += 1;
            }
            'e' => w.boolean("Exclude", true, false),
            'f' => w.boolean("OneFS", false, false),
            'h' => w.boolean("Recurse", false, false),
            'H' => w.boolean("HardLinks", false, false),
            'i' => w.boolean("IgnoreCase", true, false),
            'M' => w.quoted("Signature", Some("MD5"), false),
            'n' => w.quoted("Replace", Some("Never"), false),
            'p' => w.boolean("Portable", true, false),
            'P' => { let v = option_value(&c, &mut i); w.quoted("Strip", Some(&v), false); }
            'R' => w.boolean("HfsPlusSupport", true, false),
            'r' => w.boolean("ReadFifo", true, false),
            'S' => match c.get(i + 1) {
                Some('2') => { w.quoted("Signature", Some("SHA256"), false); i += 1; }
                Some('3') => { w.quoted("Signature", Some("SHA512"), false); i += 1; }
                Some('4') => { w.quoted("Signature", Some("XXH128"), false); i += 1; }
                _ => w.quoted("Signature", Some("SHA1"), false),
            },
            's' => w.boolean("Sparse", true, false),
            'm' => w.boolean("MtimeOnly", true, false),
            'k' => w.boolean("KeepAtime", true, false),
            'K' => w.boolean("NoAtime", true, false),
            'A' => w.boolean("AclSupport", true, false),
            'N' => w.boolean("HonorNoDumpFlag", true, false),
            'V' => { let v = option_value(&c, &mut i); w.quoted("Verify", Some(&v), false); }
            'w' => w.quoted("Replace", Some("IfNewer"), false),
            'W' => w.boolean("EnhancedWild", true, false),
            'z' => { let v = option_value(&c, &mut i); w.unquoted("Size", Some(&v), false); }
            'Z' => {
                i += 1;
                match c.get(i) {
                    Some(d) if d.is_ascii_digit() => w.quoted("Compression", Some(&format!("GZIP{d}")), false),
                    Some('o') => w.quoted("Compression", Some("LZO"), false),
                    Some('f') => {
                        i += 1;
                        match c.get(i) {
                            Some('f') => w.quoted("Compression", Some("LZFAST"), false),
                            Some('4') => w.quoted("Compression", Some("LZ4"), false),
                            Some('h') => w.quoted("Compression", Some("LZ4HC"), false),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            'X' => w.boolean("XattrSupport", true, false),
            'x' => w.boolean("AutoExclude", false, false),
            _ => {} // unknown option letter (C++ logs an error and continues)
        }
        i += 1;
    }
    w.one_per_line("Regex", &fo.regex, false, true, false);
    w.one_per_line("RegexDir", &fo.regexdir, false, true, false);
    w.one_per_line("RegexFile", &fo.regexfile, false, true, false);
    w.one_per_line("Wild", &fo.wild, false, true, false);
    w.one_per_line("WildDir", &fo.wilddir, false, true, false);
    w.one_per_line("WildFile", &fo.wildfile, false, true, false);
    w.one_per_line("WildBase", &fo.wildbase, false, true, false);
    w.one_per_line("FsType", &fo.fstype, false, true, false);
    w.one_per_line("DriveType", &fo.drivetype, false, true, false);
    w.one_per_line("Meta", &fo.meta, false, true, false);
    if let Some(p) = &fo.plugin {
        w.unquoted("Plugin", Some(p), false);
    }
    w.sub_end(false);
}

fn print_fileset(res: &ConfResource, w: &mut W) {
    w.quoted("Name", res.name(), false);
    if let Some(d) = res.directives.iter().rev().find(|d| d.norm_key() == "description").and_then(|d| d.first()) {
        w.quoted("Description", Some(d), false);
    }
    let blocks = |key: &str| -> Vec<&Vec<Directive>> {
        res.directives.iter().filter(|d| d.norm_key() == key).filter_map(|d| d.block.as_ref()).collect()
    };
    let includes = blocks("include");
    if !includes.is_empty() {
        w.push_arr("Include");
        for body in includes {
            w.sub_start("Include", false);
            let mut opts: Vec<FileOptions> = body.iter().filter(|d| d.norm_key() == "options")
                .filter_map(|d| d.block.as_ref()).map(|b| parse_options(b)).collect();
            if opts.is_empty() {
                opts.push(FileOptions { opts: default_options(&[]), ..Default::default() });
            }
            w.push_arr("Options");
            for fo in &opts {
                print_options(fo, w);
            }
            w.pop();
            let files: Vec<String> = body.iter().filter(|d| d.norm_key() == "file")
                .flat_map(|d| d.values()).map(str::to_string).collect();
            w.one_per_line("File", &files, false, true, true);
            let plugins: Vec<String> = body.iter().filter(|d| d.norm_key() == "plugin")
                .filter_map(|d| d.first()).map(str::to_string).collect();
            w.one_per_line("Plugin", &plugins, false, true, false);
            let ignore: Vec<String> = body.iter().filter(|d| d.norm_key() == "excludedircontaining")
                .filter_map(|d| d.first()).map(str::to_string).collect();
            w.one_per_line("ExcludeDirContaining", &ignore, false, true, false);
            w.sub_end(false);
        }
        w.pop();
    }
    let excludes = blocks("exclude");
    if !excludes.is_empty() {
        w.push_arr("Exclude");
        for body in excludes {
            w.sub_start("Exclude", false);
            let files: Vec<String> = body.iter().filter(|d| d.norm_key() == "file")
                .flat_map(|d| d.values()).map(str::to_string).collect();
            w.one_per_line("File", &files, false, true, false);
            w.sub_end(false);
        }
        w.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_ast::Argument;
    use bareos_config::raw::parse_str;

    fn ast(args: &[(&str, Option<&str>)]) -> CommandAst {
        CommandAst {
            command_name: "show".into(),
            arguments: args.iter().map(|(k, v)| Argument { key: k.to_string(), value: v.map(str::to_string) }).collect(),
            ..Default::default()
        }
    }

    const CONF: &str = r#"
Director {
  Name = bareos-dir
  QueryFile = "/usr/lib/bareos/scripts/query.sql"
  Maximum Concurrent Jobs = 10
  Password = "secret"
  Messages = Daemon
  DirPort = 29201
}
JobDefs {
  Name = "DefaultJob"
  Type = Backup
  Level = Incremental
  Storage = File
  Priority = 10
}
Job {
  Name = "BackupCatalog"
  JobDefs = "DefaultJob"
  Level = full
  Write Bootstrap = "|/usr/bin/bsmtp -f \"\(Bareos\) \" root"
  RunBeforeJob = "/usr/lib/bareos/scripts/make_catalog_backup MyCatalog"
  Priority = 11
}
Pool {
  Name = Incremental
  Pool Type = Backup
  Recycle = yes
  Volume Retention = 30 days
  Maximum Volume Bytes = 1G
  Maximum Volumes = 100
}
Schedule {
  Name = "WeeklyCycle"
  Run = Full 1st sat at 21:00
  Run = Differential 2nd-5th sat at 21:00
  Run = Incremental mon-fri at 21:00
}
Console {
  Name = mon
  Password = "x"
  CommandACL = status, .status
  JobACL = *all*
}
Messages {
  Name = Standard
  mailcommand = "bsmtp -f \"\(Bareos\) \<%r\>\""
  operator = root = mount
  mail = root = all, !skipped, !saved, !audit
  append = "/var/log/bareos/bareos.log" = all, !skipped
  append = "/var/log/bareos/audit.log" = audit
}
FileSet {
  Name = "Win"
  Enable VSS = yes
  Include {
    Options {
      Signature = XXH128
      IgnoreCase = yes
      WildFile = "[A-Z]:/pagefile.sys"
      WildDir = "[A-Z]:/RECYCLER"
      Exclude = yes
      Compression = LZ4
    }
    File = /
  }
  Exclude {
    File = /proc
  }
}
"#;

    fn run(args: &[(&str, Option<&str>)]) -> CommandResult {
        show(&ast(args), &parse_str(CONF, None).unwrap())
    }

    #[test]
    fn jobs_only_explicit_non_default() {
        let r = run(&[("jobs", None)]);
        let j = &r.json.unwrap()["jobs"]["BackupCatalog"];
        assert_eq!(j, &json!({
            "name": "BackupCatalog", "level": "Full", "jobdefs": "DefaultJob",
            "writebootstrap": "|/usr/bin/bsmtp -f \\\"\\(Bareos\\) \\\" root", "priority": 11,
            "runscript": [{"runbeforejob": "/usr/lib/bareos/scripts/make_catalog_backup MyCatalog"}]
        }));
        assert!(r.text_output.starts_with("Job {\n  Name = \"BackupCatalog\"\n  Level = Full\n"));
        assert!(r.text_output.contains("  RunBeforeJob = \"/usr/lib/bareos/scripts/make_catalog_backup MyCatalog\"\n"));
    }

    #[test]
    fn director_addresses_and_md5_password() {
        let r = run(&[("director", None)]);
        let d = &r.json.unwrap()["directors"]["bareos-dir"];
        assert_eq!(d["maximumconcurrentjobs"], 10);
        assert!(d.get("queryfile").is_none(), "default value is not shown");
        assert_eq!(d["password"], format!("[md5]{}", bareos_bnet::cram_md5::md5_hash_password("secret")));
        assert_eq!(d["addresses"], json!([{"ipv4": {"addr": "0.0.0.0", "port": 29201}}, {"ipv6": {"addr": "::", "port": 29201}}]));
        assert!(r.text_output.contains("  Addresses = {\n    ipv4 = {\n      addr = 0.0.0.0\n      port = 29201\n    }\n"));
    }

    #[test]
    fn pools_sizes_and_times() {
        let r = run(&[("pools", None)]);
        let p = &r.json.unwrap()["pools"]["Incremental"];
        assert_eq!(p, &json!({"name": "Incremental", "maximumvolumes": 100, "maximumvolumebytes": "1 g", "volumeretention": "1 months "}));
    }

    #[test]
    fn schedule_runs_normalized() {
        let r = run(&[("schedules", None)]);
        assert_eq!(r.json.unwrap()["schedules"]["WeeklyCycle"]["run"],
            json!(["Full 1st Sat at 21:00", "Differential 2nd-5th Sat at 21:00", "Incremental Mon-Fri at 21:00"]));
        assert!(r.text_output.contains("  Run = Full 1st Sat at 21:00\n"));
    }

    #[test]
    fn console_acls_one_line() {
        let r = run(&[("console", Some("mon"))]);
        let c = &r.json.unwrap()["consoles"]["mon"];
        assert_eq!(c["jobacl"], json!(["*all*"]));
        assert_eq!(c["commandacl"], json!(["status", ".status"]));
        assert!(r.text_output.contains("  CommandAcl = \"status\", \".status\"\n"));
    }

    #[test]
    fn messages_destinations() {
        let r = run(&[("messages", None)]);
        let m = &r.json.unwrap()["messages"]["Standard"];
        assert_eq!(m["mailcommand"], "bsmtp -f \\\"\\(Bareos\\) \\<%r\\>\\\"");
        assert_eq!(m["operator"], json!({"where": "root", "what": "mount"}));
        assert_eq!(m["mail"], json!({"where": "root", "what": "all,!saved,!skipped,!audit"}));
        // second append overwrites the JSON key, both are printed as text
        assert_eq!(m["append"], json!({"where": "/var/log/bareos/audit.log", "what": "audit"}));
        assert!(r.text_output.contains("  append = /var/log/bareos/bareos.log = all,!skipped\n"));
    }

    #[test]
    fn fileset_options_order_and_defaults() {
        let r = run(&[("fileset", Some("Win"))]);
        let f = &r.json.unwrap()["filesets"]["Win"];
        assert_eq!(f["include"][0]["options"][0], json!({
            "signature": "XXH128", "ignorecase": true, "exclude": true, "compression": "LZ4",
            "hardlinks": false, "aclsupport": true, "xattrsupport": true,
            "wilddir": ["[A-Z]:/RECYCLER"], "wildfile": ["[A-Z]:/pagefile.sys"]
        }));
        assert_eq!(f["include"][0]["file"], json!(["/"]));
        assert_eq!(f["exclude"], json!([{"file": ["/proc"]}]));
        assert!(f.get("enablevss").is_none());
    }

    #[test]
    fn unknown_resource_is_an_error() {
        let r = run(&[("job", Some("nope"))]);
        assert!(!r.success);
        assert_eq!(r.error_message, "jobs resource nope not found.\n");
        let r = run(&[("bogus", None)]);
        assert_eq!(r.error_message, "Resource bogus not found\n");
    }

    #[test]
    fn verbose_shows_defaults_as_comments() {
        let r = run(&[("pools", None), ("verbose", None)]);
        assert!(r.text_output.contains("  # PoolType = Backup\n"));
        assert_eq!(r.json.unwrap()["pools"]["Incremental"]["recycle"], true);
    }

    #[test]
    fn number_formats() {
        assert_eq!(size_si(10_000_000), "9 m 549 k 640");
        assert_eq!(size_si(64512), "63 k");
        assert_eq!(time_spec(1800), "30 minutes ");
        assert_eq!(time_spec(2_592_000 * 3), "3 months ");
        assert_eq!(parse_duration("30 days"), Some(2_592_000));
        assert_eq!(parse_units("1G", SIZE_MODS), Some(1_073_741_824));
        assert_eq!(parse_units("10 mb/s", SPEED_MODS), Some(10_000_000));
    }
}
