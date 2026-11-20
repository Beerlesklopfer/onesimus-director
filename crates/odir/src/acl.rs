//! Console ACL semantics from Bareos ua_acl.cc; policies are read after reload.
//!
//! Two tiers (docs/plans/ACL-ZWEISTUFIG.md): the ACL directives of the
//! Console and its Profiles in the configuration files are the outer frame
//! ([`file_allowed`]); the catalog ACL of the console
//! ([`crate::catalog_acl`]) can only restrict further. [`allowed`] and
//! [`any_allowed`] are the central decisions and combine both; every ACL
//! check of commands, typed RPCs, configuration CRUD, push events and
//! GetConfigCapabilities goes through them (or through [`access`] and its
//! variants, which call [`allowed`]). [`no_restrictions`] and
//! [`has_restrictions`] decide whether a listing may skip per-row checks and
//! count catalog entries as a restriction.
use crate::{command_executor::ExecContext, director_config::DirectorConfig};
use crate::catalog_acl::Restriction;
use bareos_config::raw::ConfResource;
use std::ffi::CString;

/// The ACL types of a Console or Profile resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// `CommandACL`
    Command,
    /// `JobACL`
    Job,
    /// `ClientACL`
    Client,
    /// `StorageACL`
    Storage,
    /// `ScheduleACL`
    Schedule,
    /// `PoolACL`
    Pool,
    /// `FileSetACL`
    FileSet,
    /// `CatalogACL`
    Catalog,
    /// `WhereACL`
    Where,
    /// `PluginOptionsACL`
    PluginOptions,
}
impl Kind {
    /// Every ACL type in the order of the Console directive table
    /// (dird_conf.cc ConsoleResource items), as `.acl list` shows them.
    pub const ALL: [Kind; 10] = [
        Self::Job, Self::Client, Self::Storage, Self::Schedule, Self::Pool, Self::Command,
        Self::FileSet, Self::Catalog, Self::Where, Self::PluginOptions,
    ];
    /// Normalized directive key (`bareos_config::raw::normalize_key`).
    fn key(self) -> &'static str {
        match self {
            Self::Command => "commandacl",
            Self::Job => "jobacl",
            Self::Client => "clientacl",
            Self::Storage => "storageacl",
            Self::Schedule => "scheduleacl",
            Self::Pool => "poolacl",
            Self::FileSet => "filesetacl",
            Self::Catalog => "catalogacl",
            Self::Where => "whereacl",
            Self::PluginOptions => "pluginoptionsacl",
        }
    }
    /// Directive name as written in the configuration (`JobAcl`, ...).
    pub fn directive(self) -> &'static str {
        match self {
            Self::Command => "CommandAcl",
            Self::Job => "JobAcl",
            Self::Client => "ClientAcl",
            Self::Storage => "StorageAcl",
            Self::Schedule => "ScheduleAcl",
            Self::Pool => "PoolAcl",
            Self::FileSet => "FileSetAcl",
            Self::Catalog => "CatalogAcl",
            Self::Where => "WhereAcl",
            Self::PluginOptions => "PluginOptionsAcl",
        }
    }
    /// Value of the column `acl_type` of `onesimus.console_acl`.
    pub fn catalog_name(self) -> &'static str {
        self.key().trim_end_matches("acl")
    }
    /// The type of a column value `acl_type` of `onesimus.console_acl`.
    pub fn from_catalog_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.catalog_name() == name)
    }
    /// The type of an `.acl` argument: `jobacl`, `JobAcl` or `job`.
    pub fn from_argument(key: &str) -> Option<Self> {
        let key = key.to_ascii_lowercase();
        Self::ALL.into_iter().find(|k| k.key() == key || k.catalog_name() == key)
    }
    /// The configuration resource type whose names this ACL type lists
    /// (None: Command, Where and PluginOptions name no resources).
    pub fn resource_type(self) -> Option<&'static str> {
        match self {
            Self::Job => Some("job"),
            Self::Client => Some("client"),
            Self::Storage => Some("storage"),
            Self::Schedule => Some("schedule"),
            Self::Pool => Some("pool"),
            Self::FileSet => Some("fileset"),
            Self::Catalog => Some("catalog"),
            Self::Command | Self::Where | Self::PluginOptions => None,
        }
    }
    /// `.authorized` selector (`cmd`, `job`, ...).
    pub fn selector(key: &str) -> Option<Self> {
        match key.to_ascii_lowercase().as_str() {
            "cmd" => Some(Self::Command),
            "job" => Some(Self::Job),
            "client" => Some(Self::Client),
            "storage" => Some(Self::Storage),
            "schedule" => Some(Self::Schedule),
            "pool" => Some(Self::Pool),
            "fileset" => Some(Self::FileSet),
            "catalog" => Some(Self::Catalog),
            _ => None,
        }
    }
}

/// Whether `item` is a valid resource name (IsNameValid).
pub(crate) fn valid_name(item: &str) -> bool {
    // include/baconfig.h MAX_NAME_LENGTH=128; lib/edit.cc IsNameValid.
    !item.is_empty()
        && item.len() < 128
        && !item.starts_with(' ')
        && !item.ends_with(' ')
        && item
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":.-_/ ".contains(&b))
}
/// Whether an ACL entry is evaluated as a regular expression (it contains
/// one of the characters FindInAcl() looks for); `*all*` is no regex.
pub(crate) fn is_regex(pattern: &str) -> bool {
    !pattern.eq_ignore_ascii_case("*all*") && pattern.bytes().any(|b| b".()[]|^$+?*".contains(&b))
}

/// Whether the ACL entry `pattern` (without `!`) matches `item`: `*all*`,
/// the name case-insensitively, or a POSIX ERE matching the whole item.
pub(crate) fn matches(pattern: &str, item: &str) -> bool {
    if pattern.eq_ignore_ascii_case("*all*") || pattern.eq_ignore_ascii_case(item) {
        return true;
    }
    if !is_regex(pattern) {
        return false;
    }
    let (Ok(pattern), Ok(item)) = (CString::new(pattern), CString::new(item)) else {
        return false;
    };
    // POSIX ERE, case-insensitive and a whole-item match, exactly as Bareos.
    unsafe {
        let mut regex = std::mem::MaybeUninit::<libc::regex_t>::uninit();
        if libc::regcomp(
            regex.as_mut_ptr(),
            pattern.as_ptr(),
            libc::REG_EXTENDED | libc::REG_ICASE,
        ) != 0
        {
            return false;
        }
        let mut regex = regex.assume_init();
        let mut found = std::mem::MaybeUninit::<libc::regmatch_t>::uninit();
        let result = libc::regexec(&regex, item.as_ptr(), 1, found.as_mut_ptr(), 0);
        libc::regfree(&mut regex);
        if result != 0 {
            return false;
        }
        let found = found.assume_init();
        found.rm_so == 0 && found.rm_eo as usize == item.as_bytes().len()
    }
}
/// Bareos list semantics: the first entry matching `item` decides (`!`
/// entries deny); None when no entry matches.
pub(crate) fn first_match<'a>(rules: impl IntoIterator<Item = &'a str>, item: &str) -> Option<bool> {
    for rule in rules {
        let (allow, pattern) = rule.strip_prefix('!').map_or((true, rule), |p| (false, p));
        if matches(pattern, item) {
            return Some(allow);
        }
    }
    None
}

/// [`first_match`] over the `kind` directives of one resource.
fn list(resource: &ConfResource, kind: Kind, item: &str) -> Option<bool> {
    first_match(resource.directives.iter().filter(|d| d.norm_key() == kind.key()).flat_map(|d| d.values()), item)
}

/// The central ACL decision: whether `console` may access `item` of ACL
/// `kind`. Allowed only when the file ACL ([`file_allowed`]) **and** the
/// catalog ACL ([`catalog_allowed`]) allow it (two-tier ACLs).
pub fn allowed(config: &DirectorConfig, console: &str, kind: Kind, item: &str) -> bool {
    file_allowed(config, console, kind, item) && catalog_allowed(config, console, kind, item)
}

/// The inner tier alone: a type without catalog entries allows everything;
/// a catalog list is evaluated like a Bareos list (first match decides, no
/// match denies); a locked console (catalog unreadable, F-4) may run only
/// [`crate::catalog_acl::LOCKED_COMMANDS`] and nothing of any other type.
pub(crate) fn catalog_allowed(config: &DirectorConfig, console: &str, kind: Kind, item: &str) -> bool {
    config.catalog_acl.with_restriction(console, kind, |restriction| match restriction {
        Restriction::None => true,
        Restriction::List(list) => first_match(list.iter().map(|e| e.entry.as_str()), item) == Some(true),
        Restriction::Locked => kind == Kind::Command
            && crate::catalog_acl::LOCKED_COMMANDS.iter().any(|c| c.eq_ignore_ascii_case(item)),
    })
}

/// The outer tier alone, ua_acl.cc AclAccessOk(): the console's own list,
/// then its profiles in assignment order; the first matching entry decides,
/// no match denies. The default console allows every valid name.
pub(crate) fn file_allowed(config: &DirectorConfig, console: &str, kind: Kind, item: &str) -> bool {
    if !matches!(kind, Kind::Where | Kind::PluginOptions) && !valid_name(item) {
        return false;
    }
    if console == "*UserAgent*" {
        return true;
    }
    // Console credentials never silently acquire a same-named PAM User ACL.
    let Some(resource) = config
        .raw
        .iter()
        .find(|r| r.norm_type() == "console" && r.name() == Some(console))
    else {
        return false;
    };
    if let Some(allow) = list(resource, kind, item) {
        return allow;
    }
    for name in resource
        .directives
        .iter()
        .filter(|d| d.norm_key() == "profile")
        .flat_map(|d| d.values())
    {
        let Some(profile) = config
            .raw
            .iter()
            .find(|r| r.norm_type() == "profile" && r.name() == Some(name))
        else {
            continue;
        };
        if let Some(allow) = list(profile, kind, item) {
            return allow;
        }
    }
    false
}

/// Whether the ACL `kind` of `console` allows any resource at all: the file
/// ACL ([`file_any_allowed`]) and the catalog ACL (no entries, or one entry
/// that is not a `!` negation; a locked console only commands). Used by
/// GetConfigCapabilities (a type without any allowed resource is not offered).
pub fn any_allowed(config: &DirectorConfig, console: &str, kind: Kind) -> bool {
    file_any_allowed(config, console, kind)
        && config.catalog_acl.with_restriction(console, kind, |restriction| match restriction {
            Restriction::None => true,
            Restriction::List(list) => list.iter().any(|e| !e.entry.starts_with('!')),
            Restriction::Locked => kind == Kind::Command,
        })
}

/// The file ACL alone: whether the `kind` list of `console` (its own list,
/// then its profiles) has one entry that is not a `!` negation. The default
/// console allows everything.
pub(crate) fn file_any_allowed(config: &DirectorConfig, console: &str, kind: Kind) -> bool {
    if console == "*UserAgent*" {
        return true;
    }
    let Some(resource) = config.raw.iter()
        .find(|r| r.norm_type() == "console" && r.name() == Some(console))
    else {
        return false;
    };
    let positive = |res: &ConfResource| res.directives.iter()
        .filter(|d| d.norm_key() == kind.key())
        .flat_map(|d| d.values())
        .any(|v| !v.starts_with('!'));
    if positive(resource) {
        return true;
    }
    resource.directives.iter()
        .filter(|d| d.norm_key() == "profile")
        .flat_map(|d| d.values())
        .filter_map(|name| config.raw.iter().find(|r| r.norm_type() == "profile" && r.name() == Some(name)))
        .any(positive)
}

/// C++ ua_acl.cc UaContext::AclNoRestrictions(): the first `!` entry means
/// restricted, the first `*all*` unrestricted (console list, then profiles);
/// the default console is unrestricted. Catalog entries of the type (or a
/// locked console, F-4) always mean restricted.
pub fn no_restrictions(ctx: &ExecContext, kind: Kind) -> bool {
    let console = match &ctx.session_mgr {
        Some(manager) => match manager.authenticated_console(ctx.session_id) {
            Some(name) => name,
            None => return false,
        },
        None => "*UserAgent*".into(),
    };
    if console == "*UserAgent*" {
        return true;
    }
    let Some(config) = &ctx.config else { return false };
    let Ok(config) = config.read() else { return false };
    let Some(resource) = config.raw.iter()
        .find(|r| r.norm_type() == "console" && r.name() == Some(console.as_str()))
    else {
        return false;
    };
    if config.catalog_acl.restricts(&console, kind) {
        return false;
    }
    let decide = |resource: &ConfResource| {
        resource.directives.iter()
            .filter(|d| d.norm_key() == kind.key())
            .flat_map(|d| d.values())
            .find_map(|v| if v.starts_with('!') { Some(false) } else if v.eq_ignore_ascii_case("*all*") { Some(true) } else { None })
    };
    if let Some(result) = decide(resource) {
        return result;
    }
    for name in resource.directives.iter().filter(|d| d.norm_key() == "profile").flat_map(|d| d.values()) {
        if let Some(profile) = config.raw.iter().find(|r| r.norm_type() == "profile" && r.name() == Some(name)) {
            if let Some(result) = decide(profile) {
                return result;
            }
        }
    }
    false
}

/// AclAccessOk(acl, item) for the console of the session in `ctx` (via
/// [`allowed`]); contexts without a session manager act as the default console.
pub fn access(ctx: &ExecContext, kind: Kind, item: &str) -> bool {
    let console = match &ctx.session_mgr {
        Some(manager) => match manager.authenticated_console(ctx.session_id) {
            Some(name) => name,
            None => return false,
        },
        // Internal contexts have no remote session. Production transports always
        // supply a manager; absence of a session within one is never unrestricted.
        None => "*UserAgent*".into(),
    };
    let Some(config) = &ctx.config else {
        return ctx.session_mgr.is_none()
            && (matches!(kind, Kind::Where | Kind::PluginOptions) || valid_name(item));
    };
    config
        .read()
        .map(|config| allowed(&config, &console, kind, item))
        .unwrap_or(false)
}

/// `access` for a caller that already holds the configuration lock.
pub fn access_locked(ctx: &ExecContext, config: &DirectorConfig, kind: Kind, item: &str) -> bool {
    let console = match &ctx.session_mgr {
        Some(manager) => match manager.authenticated_console(ctx.session_id) {
            Some(name) => name,
            None => return false,
        },
        None => "*UserAgent*".into(),
    };
    allowed(config, &console, kind, item)
}

/// `UaContext::AclHasRestrictions` (ua_acl.cc:191): the first `!` entry or
/// `*all*` in the console list, then in its profiles, decides. Catalog
/// entries of the type (or a locked console, F-4) always mean restricted.
pub fn has_restrictions(ctx: &ExecContext, kind: Kind) -> bool {
    let Some(manager) = &ctx.session_mgr else { return false };
    let Some(console) = manager.authenticated_console(ctx.session_id) else { return true };
    if console == "*UserAgent*" {
        return false;
    }
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else { return true };
    let Some(resource) = config.raw.iter()
        .find(|r| r.norm_type() == "console" && r.name() == Some(console.as_str())) else { return true };
    if config.catalog_acl.restricts(&console, kind) {
        return true;
    }
    let decide = |resource: &ConfResource| resource.directives.iter()
        .filter(|d| d.norm_key() == kind.key()).flat_map(|d| d.values())
        .find_map(|v| if v.starts_with('!') { Some(true) }
            else if v.eq_ignore_ascii_case("*all*") { Some(false) } else { None });
    if let Some(restricted) = decide(resource) {
        return restricted;
    }
    for name in resource.directives.iter().filter(|d| d.norm_key() == "profile").flat_map(|d| d.values()) {
        if let Some(profile) = config.raw.iter().find(|r| r.norm_type() == "profile" && r.name() == Some(name)) {
            if let Some(restricted) = decide(profile) {
                return restricted;
            }
        }
    }
    true
}

/// AclAccessOk(acl, item, audit_event=true): a denial is written to the
/// audit log (UaContext::LogAuditEventAclFailure).
pub fn access_audited(ctx: &ExecContext, kind: Kind, item: &str) -> bool {
    let allowed = access(ctx, kind, item);
    if !allowed {
        crate::audit::acl_failure(ctx, kind, item);
    }
    allowed
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn cfg() -> DirectorConfig {
        DirectorConfig::from_str(r#"
Director { Name=dir; Password=secret }
Profile { Name=p1; CommandACL=status; JobACL=!Forbidden }
Profile { Name=p2; CommandACL=*all*; JobACL=*all* }
Console { Name=restricted; Password=test; CommandACL=!delete; CommandACL=help,.authorized; Profile=p1,p2 }
Console { Name=ordered; Password=test; CommandACL=*all*,!delete }
User { Name=restricted; CommandACL=*all* }
"#).unwrap()
    }
    pub(crate) fn context() -> ExecContext {
        use std::sync::{Arc, RwLock};
        let config=DirectorConfig::from_str(r#"Director {Name=dir; Password=secret}
Console {Name=limited; Password=pw; CommandACL=help,.help,.authorized,version,remove; JobACL=!blocked,*all*; ClientACL=*all*}
User {Name=limited; CommandACL=*all*}"#).unwrap();
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16),
        )));
        let id = manager.create_session("limited");
        manager.authenticate_session(id);
        ExecContext {
            session_id: id,
            config: Some(Arc::new(RwLock::new(config))),
            session_mgr: Some(manager),
            catalog: None,
            api_mode: crate::command_executor::ApiMode::Json,
            bus_rx: None,
            job_manager: None,
        }
    }
    #[test]
    fn forbidden_dispatch_never_invokes_handler_through_prefix_or_alias() {
        fn never(
            _: &crate::command_ast::CommandAst,
            _: &ExecContext,
        ) -> crate::command_queue::CommandResult {
            panic!("forbidden side effect");
        }
        let registry = vec![crate::command_registry::CommandDef {
            name: "delete",
            aliases: &["remove"],
            description: "",
            args: vec![],
            handler: never,
            dot_command: false,
        }];
        let ctx = context();
        for input in ["delete", "del", "remove"] {
            let result = crate::command_registry::dispatch_registry(
                &registry,
                &crate::session::parse_cli_command(input),
                &ctx,
            );
            assert!(!result.success);
        }
    }
    #[test]
    fn authorized_selector_order_and_existence_independence_match_source() {
        let ctx = context();
        for (line, allowed) in [
            (".authorized", false),
            (".authorized unknown=x", false),
            (".authorized JOB=nonexistent", true),
            (".authorized job=blocked client=notconfigured", true),
            (".authorized client=notconfigured job=blocked", false),
            (".authorized job=nonexistent job", false),
            (".authorized cmd=delete", false),
        ] {
            let result = crate::commands::dot::cmd_dot_authorized(
                &crate::session::parse_cli_command(line),
                &ctx,
            );
            assert_eq!(result.success, allowed, "{line}");
            assert_eq!(result.json.unwrap()[".authorized"]["authorized"], allowed);
            assert_eq!(
                result.text_output,
                if allowed {
                    "authorized\n"
                } else {
                    "not authorized\n"
                }
            );
        }
    }
    #[test]
    fn help_details_expose_permission_false_while_overview_filters() {
        let ctx = context();
        let overview =
            crate::commands::basic::cmd_help(&crate::session::parse_cli_command("help"), &ctx);
        assert!(overview.json.unwrap().get("delete").is_none());
        for line in ["help delete", ".help item=delete", ".help full"] {
            let ast = crate::session::parse_cli_command(line);
            let result = if ast.command_name == "help" {
                crate::commands::basic::cmd_help(&ast, &ctx)
            } else {
                crate::commands::basic::cmd_dot_help(&ast, &ctx)
            };
            assert_eq!(
                result.json.unwrap()["delete"]["permission"],
                false,
                "{line}"
            );
        }
    }
    #[test]
    fn reload_removal_and_same_named_user_do_not_grant_access() {
        let ctx = context();
        assert!(!access(&ctx, Kind::Command, "delete"));
        ctx.config
            .as_ref()
            .unwrap()
            .write()
            .unwrap()
            .raw
            .retain(|r| r.norm_type() != "console");
        assert!(!access(&ctx, Kind::Command, "version"));
    }
    #[test]
    fn direct_first_match_then_profiles_in_assignment_order() {
        let cfg = cfg();
        assert!(!allowed(&cfg, "restricted", Kind::Command, "delete"));
        assert!(allowed(&cfg, "restricted", Kind::Command, "STATUS"));
        assert!(allowed(&cfg, "restricted", Kind::Command, "version"));
        assert!(!allowed(&cfg, "restricted", Kind::Job, "Forbidden"));
        assert!(allowed(&cfg, "restricted", Kind::Job, "Nonexistent"));
        assert!(allowed(&cfg, "ordered", Kind::Command, "delete"));
        assert!(!allowed(&cfg, "missing", Kind::Command, "version"));
    }
    /// Two tiers: the catalog list is intersected with the file list per
    /// type; a type without catalog entries does not restrict; any_allowed,
    /// has_restrictions and no_restrictions see catalog entries too, so no
    /// listing skips its per-row check for a catalog-restricted type.
    #[test]
    fn catalog_tier_intersects_and_counts_as_restriction() {
        use crate::catalog_acl::LoadState;
        let ctx = context(); // limited: JobACL=!blocked,*all*; ClientACL=*all*
        let config = ctx.config.as_ref().unwrap().clone();
        assert!(!has_restrictions(&ctx, Kind::Client) && no_restrictions(&ctx, Kind::Client));
        config.read().unwrap().catalog_acl.set_for_test(LoadState::Loaded, &[
            ("limited", Kind::Client, &["!fd-secret", "fd-.*"]),
            ("limited", Kind::Job, &["*all*"]),
            ("limited", Kind::Pool, &["!x"]),
        ]);
        let cfg = config.read().unwrap();
        assert!(allowed(&cfg, "limited", Kind::Client, "fd-one"));
        assert!(!allowed(&cfg, "limited", Kind::Client, "fd-secret"));
        assert!(!allowed(&cfg, "limited", Kind::Client, "other"), "no catalog match denies");
        assert!(!allowed(&cfg, "limited", Kind::Job, "blocked"), "catalog *all* cannot widen");
        assert!(allowed(&cfg, "limited", Kind::Job, "nightly"));
        assert!(allowed(&cfg, "limited", Kind::Command, "version"), "no catalog entries of the type");
        assert!(!any_allowed(&cfg, "limited", Kind::Pool), "only negations in the catalog");
        assert!(any_allowed(&cfg, "limited", Kind::Client));
        drop(cfg);
        assert!(has_restrictions(&ctx, Kind::Client) && !no_restrictions(&ctx, Kind::Client));
        config.read().unwrap().catalog_acl.set_for_test(LoadState::Loaded, &[("limited", Kind::Pool, &["!x"])]);
        assert!(!has_restrictions(&ctx, Kind::Client) && no_restrictions(&ctx, Kind::Client), "no client entries");
        // Locked (F-4): only status, .status, version and .api (output
        // format only), nothing else; .help stays locked.
        config.read().unwrap().catalog_acl.set_for_test(LoadState::Failed("down".into()), &[
            ("limited", Kind::Pool, &["!x"]),
        ]);
        let cfg = config.read().unwrap();
        assert!(allowed(&cfg, "limited", Kind::Command, "version"));
        assert!(!allowed(&cfg, "limited", Kind::Command, "help"));
        assert!(catalog_allowed(&cfg, "limited", Kind::Command, ".api"), "API clients switch the format first");
        assert!(!allowed(&cfg, "limited", Kind::Command, ".help"), "in the file ACL, but locked");
        assert!(!allowed(&cfg, "limited", Kind::Client, "fd-one"));
        assert!(!any_allowed(&cfg, "limited", Kind::Job) && any_allowed(&cfg, "limited", Kind::Command));
        assert!(allowed(&cfg, "*UserAgent*", Kind::Job, "any"), "default console never locked");
        drop(cfg);
        // Locked although the console has no client entries.
        assert!(has_restrictions(&ctx, Kind::Client) && !no_restrictions(&ctx, Kind::Client));
    }

    #[test]
    fn posix_regex_requires_full_match_and_skips_invalid_patterns() {
        assert!(matches("[[:alpha:]]+", "Backup"));
        assert!(!matches("Backup.*", "OtherBackup"));
        assert!(!matches("Back", "Backup"));
        assert!(!matches("[", "Backup"));
        assert!(matches("a.b", "A.B"));
        assert!(matches("*ALL*", "Anything"));
    }
    #[test]
    fn default_console_still_validates_names_except_where_and_plugin_options() {
        let cfg = cfg();
        for name in ["", " bad", "bad ", "bad*", "bad\n", &"a".repeat(128)] {
            assert!(!allowed(&cfg, "*UserAgent*", Kind::Job, name));
        }
        for name in ["a", "a:b.c-d_e/f g", &"a".repeat(127)] {
            assert!(allowed(&cfg, "*UserAgent*", Kind::Job, name));
        }
        assert!(allowed(
            &cfg,
            "*UserAgent*",
            Kind::Where,
            "/path with*wildcard"
        ));
        assert!(allowed(
            &cfg,
            "*UserAgent*",
            Kind::PluginOptions,
            "key=value"
        ));
    }
    #[test]
    fn queued_command_is_denied_after_grpc_manager_expiry_without_other_requests() {
        let ctx = context();
        let manager = ctx.session_mgr.as_ref().unwrap();
        manager.authenticate_session_with_ttl(ctx.session_id, std::time::Duration::ZERO);
        assert!(!access(&ctx, Kind::Command, "version"));
        assert!(!manager.refresh_session_ttl(ctx.session_id, std::time::Duration::from_secs(1)));
    }
    #[test]
    fn unknown_or_unauthenticated_session_is_not_default_console() {
        use std::sync::{Arc, RwLock};
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16),
        )));
        let id = manager.create_session("*UserAgent*");
        let mut ctx = ExecContext {
            session_id: id,
            config: Some(Arc::new(RwLock::new(cfg()))),
            session_mgr: Some(manager.clone()),
            catalog: None,
            api_mode: crate::command_executor::ApiMode::Human,
            bus_rx: None,
            job_manager: None,
        };
        assert!(!access(&ctx, Kind::Command, "version"));
        manager.authenticate_session(id);
        assert!(access(&ctx, Kind::Command, "version"));
        ctx.session_id = u64::MAX;
        assert!(!access(&ctx, Kind::Command, "version"));
    }
}
