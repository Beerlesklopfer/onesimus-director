//! Console ACL semantics from Bareos ua_acl.cc; policies are read after reload.
use crate::{command_executor::ExecContext, director_config::DirectorConfig};
use bareos_config::raw::ConfResource;
use std::ffi::CString;

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Command,
    Job,
    Client,
    Storage,
    Schedule,
    Pool,
    FileSet,
    Catalog,
    Where,
    PluginOptions,
}
impl Kind {
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
fn matches(pattern: &str, item: &str) -> bool {
    if pattern.eq_ignore_ascii_case("*all*") || pattern.eq_ignore_ascii_case(item) {
        return true;
    }
    if !pattern.bytes().any(|b| b".()[]|^$+?*".contains(&b)) {
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
fn list(resource: &ConfResource, kind: Kind, item: &str) -> Option<bool> {
    for rule in resource
        .directives
        .iter()
        .filter(|d| d.norm_key() == kind.key())
        .flat_map(|d| d.values())
    {
        let (allow, pattern) = rule.strip_prefix('!').map_or((true, rule), |p| (false, p));
        if matches(pattern, item) {
            return Some(allow);
        }
    }
    None
}

pub fn allowed(config: &DirectorConfig, console: &str, kind: Kind, item: &str) -> bool {
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

/// `UaContext::AclHasRestrictions` (ua_acl.cc:191): the first `!` entry or
/// `*all*` in the console list, then in its profiles, decides.
pub fn has_restrictions(ctx: &ExecContext, kind: Kind) -> bool {
    let Some(manager) = &ctx.session_mgr else { return false };
    let Some(console) = manager.authenticated_console(ctx.session_id) else { return true };
    if console == "*UserAgent*" {
        return false;
    }
    let Some(config) = ctx.config.as_ref().and_then(|c| c.read().ok()) else { return true };
    let Some(resource) = config.raw.iter()
        .find(|r| r.norm_type() == "console" && r.name() == Some(console.as_str())) else { return true };
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
