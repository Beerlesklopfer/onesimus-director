//! Configuration checks the C++ Director makes while parsing and in
//! CheckResources() (compatibility log C-057).
//!
//! The typed parser accepts configurations the original refuses. A `reload`
//! with such a configuration must fail ("failed to reload") and keep the
//! previous configuration, like DoReloadConfig(). Checked here, in the
//! order the original meets them, the first error ends the check:
//!
//! - pass 1 (ParseConfig, SaveResource): unknown directives, missing
//!   required directives (ValidateResource, not for Job/JobDefs), a second
//!   resource of the same type and name (AppendToResourcesChain);
//! - pass 2: references to resources that do not exist (StoreRes,
//!   StoreAlistRes);
//! - CheckResources(): exactly one Director, a Messages resource, at least
//!   one Job, PopulateJobdefaults() + ValidateResource() per Job, the
//!   MaxFullConsolidations/AlwaysIncremental job type rules and PAM consoles
//!   without ACLs/profiles.
//!
//! The messages are the Jmsg()/Emsg() texts of the original. Lexer
//! diagnostics (scan_err: "Config error: ... : line L, col C of file F")
//! are not reproduced: the raw view keeps no token positions.

use bareos_config::jobdefs::{propagate_raw, Effective};
use bareos_config::raw::{normalize_key, ConfResource, Directive};

use crate::commands::show_items::{self as tables, Item, Ty};
use crate::message_bus::MsgType;

/// One message the original writes while rejecting a configuration.
#[derive(Debug, Clone, PartialEq)]
pub enum Diag {
    /// Jmsg(nullptr, type, 0, text): "<director> JobId 0: <Type>: text"
    Job(MsgType, String),
    /// Emsg(M_ERROR, ...): "<director>: ERROR in <file>:<line> text"
    Daemon(String),
    /// Lexer diagnostic (scan_err), not reproduced; text for the log only.
    Parser(String),
}

/// Resource types of the Director: (normalized type, C++ name, items, R_ code).
const KINDS: &[(&str, &str, &[Item], &str)] = &[
    ("director", "Director", tables::DIR_ITEMS, "R_DIRECTOR"),
    ("client", "Client", tables::CLIENT_ITEMS, "R_CLIENT"),
    ("jobdefs", "JobDefs", tables::JOB_ITEMS, "R_JOBDEFS"),
    ("job", "Job", tables::JOB_ITEMS, "R_JOB"),
    ("storage", "Storage", tables::STORAGE_ITEMS, "R_STORAGE"),
    ("catalog", "Catalog", tables::CATALOG_ITEMS, "R_CATALOG"),
    ("schedule", "Schedule", tables::SCHEDULE_ITEMS, "R_SCHEDULE"),
    ("fileset", "FileSet", tables::FILESET_ITEMS, "R_FILESET"),
    ("pool", "Pool", tables::POOL_ITEMS, "R_POOL"),
    ("messages", "Messages", tables::MESSAGES_ITEMS, "R_MSGS"),
    ("counter", "Counter", tables::COUNTER_ITEMS, "R_COUNTER"),
    ("profile", "Profile", tables::PROFILE_ITEMS, "R_PROFILE"),
    ("console", "Console", tables::CONSOLE_ITEMS, "R_CONSOLE"),
    ("user", "User", tables::USER_ITEMS, "R_USER"),
];

/// Onesimus directives the original does not know (bareos-config parser):
/// accepted, they are not a reason to refuse a configuration.
const EXTENSIONS: &[(&str, &str)] = &[
    ("director", "enablegrpc"), ("director", "grpcport"), ("director", "grpctlscertificate"),
    ("director", "grpctlskey"), ("director", "mdnsadvertise"), ("catalog", "dbdriver"),
];

fn kind(rtype: &str) -> Option<&'static (&'static str, &'static str, &'static [Item], &'static str)> {
    let t = normalize_key(rtype);
    KINDS.iter().find(|k| k.0 == t)
}

fn kind_of_code(code: &str) -> Option<&'static str> {
    KINDS.iter().find(|k| k.3 == code).map(|k| k.0)
}

/// Item of `items` a directive key names (name or alias).
fn item_of<'a>(items: &'a [Item], key: &str) -> Option<&'a Item> {
    let key = normalize_key(key);
    items.iter().find(|i| normalize_key(i.name) == key
        || i.aliases.iter().any(|a| normalize_key(a) == key))
}

fn has_item(dirs: &[&Directive], items: &[Item], name: &str) -> bool {
    dirs.iter().any(|d| item_of(items, &d.key).is_some_and(|i| i.name == name))
}

fn value<'a>(dirs: &[&'a Directive], items: &[Item], name: &str) -> Option<&'a str> {
    dirs.iter().rev().find(|d| item_of(items, &d.key).is_some_and(|i| i.name == name))
        .and_then(|d| d.first())
}

fn of<'a>(raw: &'a [ConfResource], k: &'static str) -> impl Iterator<Item = &'a ConfResource> + 'a {
    raw.iter().filter(move |r| normalize_key(&r.rtype) == k)
}

/// Check `raw` like the original's ParseConfig() + CheckResources().
/// `base` is the configuration path as the original prints it
/// (`get_base_config_path()`).
pub fn check(raw: &[ConfResource], base: &str) -> Result<(), Vec<Diag>> {
    // ---- pass 1 -------------------------------------------------------
    let mut seen: Vec<(&str, &str)> = Vec::new();
    for res in raw {
        let Some(&(norm, cname, items, _)) = kind(&res.rtype) else {
            return Err(vec![Diag::Parser(format!("unknown resource type {}", res.rtype))]);
        };
        if let Some(d) = res.directives.iter().find(|d| {
            item_of(items, &d.key).is_none() && !EXTENSIONS.contains(&(norm, d.norm_key().as_str()))
        }) {
            return Err(vec![Diag::Parser(format!(
                "Keyword \"{}\" not permitted in this resource.", d.key))]);
        }
        let Some(name) = res.name() else {
            return Err(vec![Diag::Daemon(format!(
                "ERROR in lib/parse_conf.cc:260 Name item is required in {cname} resource, but not found.\n"))]);
        };
        // Job resources are validated after applying JobDefs
        if norm != "job" && norm != "jobdefs" {
            let dirs: Vec<&Directive> = res.directives.iter().collect();
            if let Some(item) = items.iter().find(|i| i.required && !has_item(&dirs, items, i.name)) {
                return Err(vec![
                    Diag::Job(MsgType::Error, format!(
                        "\"{}\" directive in {cname} \"{name}\" resource is required, but not found.\n", item.name)),
                    Diag::Parser("SaveResource failed".into()),
                ]);
            }
        }
        if seen.iter().any(|(k, n)| *k == norm && *n == name) {
            return Err(vec![
                Diag::Daemon(format!(
                    "ERROR in lib/parse_conf.cc:276 Attempt to define second {cname} resource named \"{name}\" is not permitted.\n")),
                Diag::Parser("SaveResource failed".into()),
            ]);
        }
        seen.push((norm, name));
    }

    // ---- pass 2: resource references --------------------------------
    let exists = |kind: &str, name: &str| raw.iter().any(|r| {
        normalize_key(&r.rtype) == kind && r.name() == Some(name)
    });
    for res in raw {
        let Some(&(_, _, items, _)) = kind(&res.rtype) else { continue };
        for d in &res.directives {
            let Some(item) = item_of(items, &d.key) else { continue };
            if !matches!(item.ty, Ty::Res | Ty::AlistRes) {
                continue;
            }
            let Some(target) = item.code.and_then(kind_of_code) else { continue };
            for v in d.values() {
                if !exists(target, v) {
                    return Err(vec![Diag::Parser(format!(
                        "Could not find config resource \"{v}\" referenced by {}", d.key))]);
                }
            }
        }
    }

    // ---- CheckResources() -------------------------------------------
    let fatal = |text: String| Err(vec![Diag::Job(MsgType::Fatal, text)]);
    let directors: Vec<&ConfResource> = of(raw, "director").collect();
    let Some(me) = directors.first() else {
        return fatal(format!("No Director resource defined in {base}\nWithout that I don't know who I am :-(\n"));
    };
    let me_dirs: Vec<&Directive> = me.directives.iter().collect();
    if value(&me_dirs, tables::DIR_ITEMS, "Messages").is_none() && of(raw, "messages").next().is_none() {
        return fatal(format!("No Messages resource defined in {base}\n"));
    }
    if directors.len() > 1 {
        return fatal(format!("Only one Director resource permitted in {base}\n"));
    }
    if of(raw, "job").next().is_none() {
        return fatal(format!("No Job records defined in {base}\n"));
    }

    // PopulateJobdefaults(): ValidateResource(R_JOB) per job
    let propagated = propagate_raw(raw);
    let job_dirs = |e: &[Effective]| -> Vec<Directive> { e.iter().map(|e| e.directive.clone()).collect() };
    let mut jobs: Vec<(String, Vec<Directive>)> = Vec::new();
    for res in of(raw, "job") {
        let name = res.name().unwrap_or_default().to_string();
        let dirs = propagated.job(&name).map(job_dirs).unwrap_or_else(|| res.directives.clone());
        jobs.push((name, dirs));
    }
    let items = tables::JOB_ITEMS;
    for (name, dirs) in &jobs {
        let dirs: Vec<&Directive> = dirs.iter().collect();
        let jobtype = value(&dirs, items, "Type").unwrap_or("").to_ascii_lowercase();
        if jobtype != "copy" && jobtype != "migrate" {
            let error = |text: String| Err(vec![Diag::Job(MsgType::Error, text)]);
            if !has_item(&dirs, items, "Client") {
                return error(format!("\"client\" directive in Job \"{name}\" resource is required, but not found.\n"));
            }
            if !has_item(&dirs, items, "FileSet") {
                return error(format!("\"fileset\" directive in Job \"{name}\" resource is required, but not found.\n"));
            }
            let pool_storage = value(&dirs, items, "Pool")
                .and_then(|p| of(raw, "pool").find(|r| r.name() == Some(p)))
                .is_some_and(|p| p.directives.iter().any(|d| d.norm_key() == "storage"));
            if !has_item(&dirs, items, "Storage") && !pool_storage {
                return error(format!("No storage specified in Job \"{name}\" nor in Pool.\n"));
            }
        }
        if let Some(item) = items.iter().find(|i| i.required && !has_item(&dirs, items, i.name)) {
            return Err(vec![Diag::Job(MsgType::Error, format!(
                "\"{}\" directive in Job \"{name}\" resource is required, but not found.\n", item.name))]);
        }
    }
    for (name, dirs) in &jobs {
        let dirs: Vec<&Directive> = dirs.iter().collect();
        let jobtype = value(&dirs, items, "Type").unwrap_or("").to_ascii_lowercase();
        let nonzero = |item: &str| value(&dirs, items, item).is_some_and(|v| {
            !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "no" | "false")
        });
        if nonzero("MaxFullConsolidations") && jobtype != "consolidate" {
            return fatal(format!("MaxFullConsolidations configured in job {name} which is not of job type \"consolidate\" in file {base}\n"));
        }
        if jobtype != "backup"
            && ["AlwaysIncremental", "AlwaysIncrementalJobRetention", "AlwaysIncrementalKeepNumber",
                "AlwaysIncrementalMaxFullAge"].iter().any(|i| nonzero(i)) {
            return fatal(format!("AlwaysIncremental configured in job {name} which is not of job type \"backup\" in file {base}\n"));
        }
    }

    let pam: Vec<&str> = of(raw, "console").filter(|c| {
        let dirs: Vec<&Directive> = c.directives.iter().collect();
        let items = tables::CONSOLE_ITEMS;
        let uses_pam = value(&dirs, items, "UsePamAuthentication")
            .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "yes" | "true"));
        let acl = dirs.iter().any(|d| item_of(items, &d.key)
            .is_some_and(|i| i.ty == Ty::Acl || i.name == "Profile"));
        uses_pam && acl
    }).filter_map(|c| c.name()).collect();
    if !pam.is_empty() {
        return fatal(format!("Combining `Use Pam Authentication` with ACL commands or `Profile` in Console(s) `{}` is not allowed\n",
            pam.join(",")));
    }
    Ok(())
}

/// The configuration path as the original prints it: the file given with
/// -c, or `<dir>/bareos-dir.d/*/*.conf` (`<dir>/*/*.conf` when -c names
/// the bareos-dir.d directory itself).
pub fn base_config_path(path: &std::path::Path) -> String {
    if path.is_dir() {
        let dir_d = path.join("bareos-dir.d");
        let base = if dir_d.is_dir() { dir_d } else { path.to_path_buf() };
        format!("{}/*/*.conf", base.display())
    } else {
        path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bareos_config::raw::parse_str;

    const BASE: &str = r#"
Director { Name = d; Password = p; Messages = Daemon }
Messages { Name = Daemon; append = "/tmp/x" = all }
Catalog { Name = MyCatalog; dbname = bareos; dbuser = bareos }
Client { Name = fd; Address = localhost; Password = p }
Storage { Name = File; Address = localhost; Password = p; Device = FileStorage; MediaType = File }
Pool { Name = Full; Pool Type = Backup }
FileSet { Name = fs; Include { File = /etc } }
JobDefs { Name = DefaultJob; Type = Backup; Client = fd; FileSet = fs; Storage = File; Pool = Full; Messages = Daemon }
Job { Name = j; JobDefs = DefaultJob }
"#;

    fn run(extra: &str) -> Result<(), Vec<Diag>> {
        check(&parse_str(&format!("{BASE}{extra}"), None).unwrap(), "/etc/bareos/bareos-dir.d/*/*.conf")
    }

    #[test]
    fn valid_configuration_passes() {
        assert_eq!(run(""), Ok(()));
    }

    #[test]
    fn installed_configurations_pass() {
        // config/bareos-dir.d of this repository is a sketch the original
        // refuses too (FileSet "SelfTest" undefined), not checked here.
        for dir in ["/etc/bareos/bareos-dir.d"] {
            let path = std::path::Path::new(dir);
            if !path.is_dir() {
                continue;
            }
            let raw = bareos_config::raw::load(path).unwrap();
            if raw.iter().any(|r| normalize_key(&r.rtype) == "job") {
                assert_eq!(check(&raw, "x"), Ok(()), "{dir}");
            }
        }
    }

    #[test]
    fn parse_errors_are_reported() {
        assert!(matches!(run("Client { Name = b; Address = a; Password = p; Bogus = 1 }").unwrap_err()[0], Diag::Parser(_)));
        assert!(matches!(run("Job { Name = r; JobDefs = DefaultJob; Client = nosuch }").unwrap_err()[0], Diag::Parser(_)));
        assert_eq!(run("Client { Name = fd; Address = b; Password = p }").unwrap_err()[0],
            Diag::Daemon("ERROR in lib/parse_conf.cc:276 Attempt to define second Client resource named \"fd\" is not permitted.\n".into()));
        assert_eq!(run("Client { Name = n; Password = p }").unwrap_err()[0],
            Diag::Job(MsgType::Error, "\"Address\" directive in Client \"n\" resource is required, but not found.\n".into()));
    }

    #[test]
    fn check_resources_rules() {
        assert_eq!(run("Job { Name = c; JobDefs = DefaultJob; Max Full Consolidations = 1 }").unwrap_err(),
            vec![Diag::Job(MsgType::Fatal, "MaxFullConsolidations configured in job c which is not of job type \"consolidate\" in file /etc/bareos/bareos-dir.d/*/*.conf\n".into())]);
        assert_eq!(run("Job { Name = n; Type = Backup; Client = fd; Storage = File; Pool = Full; Messages = Daemon }").unwrap_err(),
            vec![Diag::Job(MsgType::Error, "\"fileset\" directive in Job \"n\" resource is required, but not found.\n".into())]);
        assert_eq!(run("Job { Name = m; Type = Backup; Client = fd; FileSet = fs; Storage = File; Pool = Full }").unwrap_err(),
            vec![Diag::Job(MsgType::Error, "\"Messages\" directive in Job \"m\" resource is required, but not found.\n".into())]);
        assert!(run("Job { Name = cp; Type = Copy; Pool = Full; Messages = Daemon }").is_ok());
    }
}
