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
use std::collections::{BTreeMap, BTreeSet};
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
    pub(crate) fn key(self) -> &'static str {
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
    assistant_allowed(console, kind, item)
        && file_allowed(config, console, kind, item)
        && catalog_allowed(config, console, kind, item)
}

/// Name suffix of an assistant console: `oconsole --mcp` logs in only as
/// `<console>-mcp` (oconsole `MCP_CONSOLE_SUFFIX`).
pub const ASSISTANT_SUFFIX: &str = "-mcp";

/// Commands an assistant console never runs, whatever its ACLs say: ACL
/// changes (`.acl` and its alias `.profile`), with which it could lift its
/// own limits, and raw SQL (`sqlquery`, `.sql`, `query`), the only way to
/// stored contents (catalog table RestoreObject). Onesimus addition, Bareos
/// has no assistant consoles.
pub const ASSISTANT_REFUSED: [&str; 5] = [".acl", ".profile", "sqlquery", ".sql", "query"];

/// Whether `console` is an assistant console (name ends in
/// [`ASSISTANT_SUFFIX`], any case).
pub fn is_assistant(console: &str) -> bool {
    console.len() > ASSISTANT_SUFFIX.len()
        && console.to_ascii_lowercase().ends_with(ASSISTANT_SUFFIX)
}

/// The assistant tier: false only for a command of [`ASSISTANT_REFUSED`]
/// asked by an assistant console ([`is_assistant`]).
fn assistant_allowed(console: &str, kind: Kind, item: &str) -> bool {
    !(kind == Kind::Command
        && is_assistant(console)
        && ASSISTANT_REFUSED.iter().any(|c| c.eq_ignore_ascii_case(item)))
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

/// The ACL entries of type `kind` that decide for `console` in evaluation
/// order: its own list, then the lists of its existing profiles in
/// assignment order (the sequence [`file_allowed`] walks).
fn file_entries<'a>(config: &'a DirectorConfig, console: &'a ConfResource, kind: Kind) -> Vec<&'a str> {
    let own = console.directives.iter().filter(|d| d.norm_key() == kind.key()).flat_map(|d| d.values());
    let profiles = console.directives.iter()
        .filter(|d| d.norm_key() == "profile")
        .flat_map(|d| d.values())
        .filter_map(|name| config.raw.iter().find(|r| r.norm_type() == "profile" && r.name() == Some(name)))
        .flat_map(|profile| profile.directives.iter().filter(|d| d.norm_key() == kind.key()).flat_map(|d| d.values()));
    own.chain(profiles).collect()
}

/// The names an ACL list allows, for lists reduced to plain names and
/// `*all*`: a finite set, or every name except a finite set. Names are
/// ASCII-lowercased (list matching ignores ASCII case).
#[derive(Debug, Clone, PartialEq, Eq)]
enum NameSet {
    /// Exactly these names.
    Only(BTreeSet<String>),
    /// Every name except these.
    AllBut(BTreeSet<String>),
}

impl NameSet {
    /// Every name.
    fn all() -> Self {
        Self::AllBut(BTreeSet::new())
    }

    /// The names both sets contain.
    fn intersect(self, other: Self) -> Self {
        match (self, other) {
            (Self::Only(a), Self::Only(b)) => Self::Only(a.intersection(&b).cloned().collect()),
            (Self::Only(a), Self::AllBut(e)) | (Self::AllBut(e), Self::Only(a)) => Self::Only(a.difference(&e).cloned().collect()),
            (Self::AllBut(a), Self::AllBut(b)) => Self::AllBut(a.union(&b).cloned().collect()),
        }
    }

    /// A name of `self` that `other` lacks ("*all*" stands for names that
    /// are not listed anywhere), None when `self` ⊆ `other`.
    fn excess(&self, other: &Self) -> Option<String> {
        match (self, other) {
            (Self::Only(a), Self::Only(b)) => a.difference(b).next().cloned(),
            (Self::Only(a), Self::AllBut(e)) => a.intersection(e).next().cloned(),
            (Self::AllBut(_), Self::Only(_)) => Some("*all*".into()),
            (Self::AllBut(a), Self::AllBut(b)) => b.difference(a).next().cloned(),
        }
    }

    /// The names the Bareos list `entries` allows (first match decides, no
    /// match denies), approximated where regexes occur: with `over` a
    /// superset (a positive regex counts as `*all*`, a `!` regex is
    /// skipped), otherwise a subset (a positive regex is skipped, a `!`
    /// regex counts as `!*all*`). Each replacement only moves names in the
    /// stated direction: skipping a positive entry or ending the list early
    /// with a denial can only deny more, and vice versa.
    fn of_list<'a>(entries: impl IntoIterator<Item = &'a str>, over: bool) -> Self {
        let mut decided: BTreeMap<String, bool> = BTreeMap::new();
        let names = |decided: &BTreeMap<String, bool>, allow: bool| decided.iter()
            .filter(|(_, a)| **a == allow).map(|(n, _)| n.clone()).collect::<BTreeSet<_>>();
        for entry in entries {
            let (allow, pattern) = entry.strip_prefix('!').map_or((true, entry), |p| (false, p));
            let regex = is_regex(pattern);
            let all = pattern.eq_ignore_ascii_case("*all*") || (regex && allow == over);
            if all {
                return if allow { Self::AllBut(names(&decided, false)) } else { Self::Only(names(&decided, true)) };
            }
            if !regex {
                decided.entry(pattern.to_ascii_lowercase()).or_insert(allow);
            }
        }
        Self::Only(names(&decided, true))
    }
}

/// The names of `kind` the existing console `creator` may use, as a subset
/// ([`NameSet::of_list`] without `over`) of its effective ACL: file list
/// (own entries, then profiles) intersected with its catalog list.
fn creator_names(config: &DirectorConfig, creator: &str, kind: Kind) -> NameSet {
    if creator == "*UserAgent*" {
        return NameSet::all();
    }
    let Some(resource) = config.raw.iter().find(|r| r.norm_type() == "console" && r.name() == Some(creator)) else {
        return NameSet::Only(BTreeSet::new());
    };
    let file = NameSet::of_list(file_entries(config, resource, kind), false);
    let catalog = config.catalog_acl.with_restriction(creator, kind, |restriction| match restriction {
        Restriction::None => NameSet::all(),
        Restriction::List(list) => NameSet::of_list(list.iter().map(|e| e.entry.as_str()), false),
        Restriction::Locked => NameSet::Only(BTreeSet::new()),
    });
    file.intersect(catalog)
}

/// Every command name an ACL check of type Command can meet: the Bareos
/// command table, the registered commands and their aliases (all checks
/// of commands, typed RPCs and the literal checks such as `configure` or
/// `.sql` use these names).
fn command_names() -> BTreeSet<&'static str> {
    let registry = crate::command_executor::get_registry();
    crate::ua_commands_table::UA_COMMANDS.iter().map(|c| c.0)
        .chain(registry.iter().map(|c| c.name))
        .chain(registry.iter().flat_map(|c| c.aliases.iter().copied()))
        .collect()
}

/// Commands that change catalog ACLs (`.acl` and its alias `.profile`).
const ACL_COMMANDS: [&str; 2] = [".acl", ".profile"];

/// Anti-escalation check of `configure add console` (F-1 amendment of
/// 20.11.2026, docs/plans/ACL-ZWEISTUFIG.md): may `creator` create a
/// console whose only ACLs are those of `profiles` (no own ACL lines, no
/// catalog entries)? Returns the reason for a refusal, None when allowed.
///
/// The new console's file ACL is the first match over the profiles' lists
/// in the given order ([`file_allowed`] with an empty own list). For every
/// ACL type ([`Kind::ALL`]) its allowed names must be a subset of the
/// creator's effective names (file ∩ catalog tier, as [`allowed`] decides):
/// - Command: checked name by name over every command name a check can
///   meet ([`command_names`]), with [`first_match`] for the new console and
///   [`allowed`] for the creator, so dot commands (regexes in Bareos terms,
///   `.api`) are decided exactly;
/// - all other types (resource names, Where, PluginOptions: open-ended):
///   the new console's names as a superset and the creator's as a subset
///   ([`NameSet::of_list`]), and the superset must lie in the subset. Lists
///   of plain names, `!` entries and `*all*` are compared exactly; regexes
///   make the comparison conservative (a positive regex in a profile counts
///   as `*all*`, a `!` regex of the creator as `!*all*`).
///
/// Further refusals: no authenticated creator; a locked creator (F-4); and
/// profiles that grant `.acl`/`.profile` while the creator has catalog
/// entries of any type, because the new console could then remove them
/// (F-2 restricts changes of the *own* console only) and lift the creator
/// above the rights it had when it created the console.
pub(crate) fn grant_error(config: &DirectorConfig, creator: &str, profiles: &[&str]) -> Option<String> {
    if creator.is_empty() {
        return Some("no authenticated console".into());
    }
    if is_assistant(creator) {
        return Some(format!("Console \"{creator}\" is an assistant console (*{ASSISTANT_SUFFIX}): it creates no consoles"));
    }
    if config.catalog_acl.with_restriction(creator, Kind::Command, |r| matches!(r, Restriction::Locked)) {
        return Some(format!("Console \"{creator}\" is locked (catalog ACL not readable)"));
    }
    let mut resources = Vec::new();
    for name in profiles {
        match config.raw.iter().find(|r| r.norm_type() == "profile" && r.name() == Some(*name)) {
            Some(profile) => resources.push(profile),
            None => return Some(format!("unknown Profile \"{name}\"")),
        }
    }
    let entries = |kind: Kind| -> Vec<&str> {
        resources.iter()
            .flat_map(|p| p.directives.iter().filter(|d| d.norm_key() == kind.key()).flat_map(|d| d.values()))
            .collect()
    };
    let commands = entries(Kind::Command);
    for name in command_names() {
        if first_match(commands.iter().copied(), name) == Some(true) && !allowed(config, creator, Kind::Command, name) {
            return Some(format!("the profiles grant CommandAcl \"{name}\", which Console \"{creator}\" may not use"));
        }
    }
    for kind in Kind::ALL.into_iter().filter(|k| *k != Kind::Command) {
        let granted = NameSet::of_list(entries(kind).iter().copied(), true);
        if let Some(name) = granted.excess(&creator_names(config, creator, kind)) {
            let what = if name == "*all*" { "names beyond a fixed list (*all* or a pattern)".to_string() } else { format!("\"{name}\"") };
            return Some(format!("the profiles grant {} {what}, which Console \"{creator}\" may not use", kind.directive()));
        }
    }
    let grants_acl = ACL_COMMANDS.iter().any(|c| first_match(commands.iter().copied(), c) == Some(true));
    if grants_acl && Kind::ALL.into_iter().any(|kind| config.catalog_acl.restricts(creator, kind)) {
        return Some(format!("the profiles grant .acl, and Console \"{creator}\" has catalog ACL entries the new \
            console could remove"));
    }
    None
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
    /// An assistant console (`*-mcp`) never runs ACL changes or raw SQL,
    /// not even with `CommandACL = *all*`; the shipped profile `assistant`
    /// reads and configures but runs, restores and deletes nothing.
    #[test]
    fn assistant_consoles_never_change_acls_or_read_sql() {
        let profile = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/bareos-dir.d/profile/assistant.conf")).unwrap();
        let cfg = DirectorConfig::from_str(&format!(r#"
Director {{ Name=dir; Password=secret }}
{profile}
Console {{ Name=admin-mcp; Password=pw; CommandACL=*all*; JobACL=*all* }}
Console {{ Name=Helper-MCP; Password=pw; Profile=assistant }}
Console {{ Name=admin; Password=pw; CommandACL=*all* }}
"#)).unwrap();
        for c in ASSISTANT_REFUSED.iter().chain(&["SQLQUERY", ".Acl"]) {
            assert!(!allowed(&cfg, "admin-mcp", Kind::Command, c), "{c}");
            assert!(!allowed(&cfg, "Helper-MCP", Kind::Command, c), "{c}");
            assert!(allowed(&cfg, "admin", Kind::Command, c), "{c}: other consoles keep their ACLs");
        }
        assert!(allowed(&cfg, "admin-mcp", Kind::Command, "purge"), "everything else follows the ACLs");
        for c in ["status", "list", "show", "configure", ".ovfs_ls", ".bvfs_lsfiles", ".jobs", ".api"] {
            assert!(allowed(&cfg, "Helper-MCP", Kind::Command, c), "{c}");
        }
        for c in ["run", "restore", "purge", "delete", "cancel", "label", "reload", ".bvfs_restore"] {
            assert!(!allowed(&cfg, "Helper-MCP", Kind::Command, c), "{c}");
        }
        assert!(!is_assistant("-mcp") && is_assistant("x-mcp") && !is_assistant("mcp"));
        let refused = grant_error(&cfg, "admin-mcp", &["assistant"]).unwrap();
        assert!(refused.contains("creates no consoles"), "{refused}");
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

    /// `configure add console`: the name sets are exact for plain names,
    /// `!` and `*all*`, and approximate regex lists in the stated direction
    /// (over: superset, under: subset of what [`first_match`] allows).
    #[test]
    fn name_sets_approximate_in_the_safe_direction() {
        let set = |e: &[&str], over| NameSet::of_list(e.iter().copied(), over);
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(set(&["A", "!b", "*all*", "!c"], false), NameSet::AllBut(names(&["b"])));
        assert_eq!(set(&["a", "!A"], true), NameSet::Only(names(&["a"])));
        assert_eq!(set(&["a", "!*all*", "b"], true), NameSet::Only(names(&["a"])));
        assert_eq!(set(&["x.*", "a"], true), NameSet::all());
        assert_eq!(set(&["x.*", "a"], false), NameSet::Only(names(&["a"])));
        assert_eq!(set(&["!x.*", "a"], false), NameSet::Only(names(&[])));
        assert_eq!(set(&["!x.*", "a"], true), NameSet::Only(names(&["a"])));
        let contains = |set: &NameSet, name: &str| match set {
            NameSet::Only(n) => n.contains(&name.to_ascii_lowercase()),
            NameSet::AllBut(n) => !n.contains(&name.to_ascii_lowercase()),
        };
        let lists: [&[&str]; 6] = [&["a", "!b", "*all*"], &["!job-.*", "job-a", "*all*"], &["job-.*", "!job-a"],
            &["!b", "a.c", "!*all*", "b"], &["B", "!b.*", "*ALL*"], &[]];
        for list in lists {
            for name in ["a", "A", "b", "abc", "job-a", "job-b", "other", "b1"] {
                let real = first_match(list.iter().copied(), name) == Some(true);
                assert!(!real || contains(&set(list, true), name), "over {list:?} {name}");
                assert!(real || !contains(&set(list, false), name), "under {list:?} {name}");
            }
        }
        assert_eq!(NameSet::Only(names(&["a"])).excess(&NameSet::AllBut(names(&["b"]))), None);
        assert_eq!(NameSet::Only(names(&["b"])).excess(&NameSet::AllBut(names(&["b"]))), Some("b".into()));
        assert_eq!(NameSet::all().excess(&NameSet::Only(names(&["a"]))), Some("*all*".into()));
        assert_eq!(NameSet::AllBut(names(&["b"])).excess(&NameSet::AllBut(names(&["b", "c"]))), Some("c".into()));
        assert_eq!(NameSet::AllBut(names(&["b", "c"])).excess(&NameSet::AllBut(names(&["b"]))), None);
        // a profile regex after the creator's own negations stays within "!geheim, *all*"
        assert_eq!(set(&["!geheim", "backup-.*"], true).excess(&set(&["!geheim", "*all*"], false)), None);
        assert_eq!(set(&["backup-.*"], true).excess(&set(&["!geheim", "*all*"], false)), Some("geheim".into()));
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
