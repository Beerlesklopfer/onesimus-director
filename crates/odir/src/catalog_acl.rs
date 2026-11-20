//! Inner tier of the two-tier console ACLs (docs/plans/ACL-ZWEISTUFIG.md).
//!
//! The ACL directives of the Console and Profile resources in the
//! configuration files are the outer frame (evaluated in [`crate::acl`] like
//! Bareos `ua_acl.cc`). The catalog table `onesimus.console_acl` (schema
//! extension v2253 of `bareos-orm`) holds a second list per console and ACL
//! type; a name is allowed only when the file ACL **and** the catalog list
//! allow it. A type without catalog entries does not restrict.
//!
//! [`crate::acl::allowed`] is synchronous and runs for every ACL check, so it
//! never touches the database. It reads the in-memory cache of this module,
//! [`CatalogAcl`], which is (re)loaded
//!
//! - at start, before the listeners accept connections ([`CatalogAcl::attach`]
//!   followed by [`CatalogAcl::refresh`] in `main`),
//! - after every successful configuration reload (`reload.rs`),
//! - after every change through `.acl set/delete/clear/copy`
//!   (`commands/acl.rs`),
//! - every [`RETRY_INTERVAL`] while the last load failed (thread
//!   `dir:catalog-acl`, [`spawn_refresh_thread`]), so the Director recovers
//!   by itself when the catalog is back, and
//! - every [`REFRESH_INTERVAL`] while the last load succeeded (same thread),
//!   so rows changed directly by SQL in `onesimus.console_acl` take effect
//!   without `reload` (logged when the content changed).
//!
//! Every database access of this module is bounded by [`CATALOG_TIMEOUT`]
//! ([`CatalogHandle::query_rows_within`]): during a catalog outage a load or
//! an `.acl` change fails after a few seconds with a clear message instead
//! of waiting for the pool's acquire timeout (30 s).
//!
//! # Unreadable catalog (decision F-4)
//!
//! When a load fails, the console is **locked** (only the commands in
//! [`LOCKED_COMMANDS`], nothing of any other ACL type) unless it is *known*
//! to have no catalog entries. Known means: the last successful load found
//! no entries for that console, and no `.acl` change of that console was
//! started since then. Before the first successful load nothing is known,
//! so every console is locked. The default console `*UserAgent*` is no
//! Console resource and can never have catalog entries; it is never locked.
//! A Director without a catalog ([`LoadState::Off`]) uses the file ACLs only.
//!
//! A missing table (catalog without the v2253 extension, e.g. the migration
//! failed) counts as a successful load with no entries, as long as no
//! earlier successful load found any entries; otherwise it is a failure.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use crate::acl::Kind;
use crate::catalog_worker::CatalogHandle;

/// Commands a locked console may still run (F-4): the Director's status and
/// version, nothing that reads or changes catalog data or resources, plus
/// `.api`, which only switches the output format of the session (API 0/1/2,
/// JSON) and reveals nothing: API clients (bconsole -api, WebUI, the
/// Onesimus GUI, MCP) send it first, and without it they could not even
/// parse the `status`/`version` answers a locked console still gets.
/// `.help` stays locked (it lists the commands, i.e. configuration), like
/// every other command. Every other ACL type allows nothing while locked.
/// `quit`, `exit` and `whoami` stay possible as for every console
/// (command_registry::resolve_permitted).
pub const LOCKED_COMMANDS: &[&str] = &["status", ".status", "version", ".api"];

/// Pause between two load attempts while the last load failed.
pub const RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// Pause between two loads while the last load succeeded: picks up rows
/// changed directly in the catalog (SQL) without `reload`.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Upper bound for every catalog access of the catalog ACL (load, `.acl`
/// write): a few seconds, so a console gets its refusal promptly during a
/// catalog outage (F-4) instead of after the pool's 30 s acquire timeout.
pub const CATALOG_TIMEOUT: Duration = Duration::from_secs(5);

/// Name of the catalog table of the inner tier, schema-qualified: Onesimus
/// objects live in the schema `onesimus` (v2253), `public` holds only the
/// Bareos tables. Qualified rather than via `search_path`, so the catalog
/// connections resolve the Bareos tables exactly as before. `to_regclass`
/// of this name is NULL while the schema or the table is missing.
pub const TABLE: &str = "onesimus.console_acl";

/// One catalog ACL entry as stored in `onesimus.console_acl`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    /// The entry as in a configuration file: name, regex, `!negation`, `*all*`.
    pub entry: String,
    /// Console that set the entry.
    pub changed_by: String,
    /// Time of the change as PostgreSQL prints it (text format).
    pub changed_at: String,
}

/// Catalog lists of one console by ACL type, each in evaluation order.
pub type ConsoleLists = BTreeMap<Kind, Vec<CatalogEntry>>;

/// State of the last load of the catalog ACL.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LoadState {
    /// No catalog attached: the inner tier is off, file ACLs apply alone.
    #[default]
    Off,
    /// The last load succeeded; the cache is the catalog's content.
    Loaded,
    /// The last load failed with this message (F-4 applies).
    Failed(String),
}

/// What the inner tier says about one console and ACL type.
#[derive(Debug)]
pub enum Restriction<'a> {
    /// No catalog entries (or inner tier off): the file ACL decides alone.
    None,
    /// The catalog list (non-empty, in evaluation order) must allow the name too.
    List(&'a [CatalogEntry]),
    /// The catalog is unreadable for this console (F-4): only
    /// [`LOCKED_COMMANDS`] are allowed.
    Locked,
}

/// Mutable part of [`CatalogAcl`], behind one lock.
#[derive(Debug, Default)]
struct State {
    /// Result of the last load.
    load: LoadState,
    /// Whether any load has succeeded since start.
    ever_loaded: bool,
    /// Content of the last successful load, by console.
    entries: BTreeMap<String, ConsoleLists>,
    /// Consoles whose catalog ACL a change started on since the last
    /// successful load (no longer "known" to have no entries, F-4).
    pending: BTreeSet<String>,
    /// The last successful load found no table (extension not installed).
    table_missing: bool,
}

/// The cached inner tier of the console ACLs, shared by every configuration
/// generation (a reload hands the same `Arc` on, `reload::install_config`).
#[derive(Default)]
pub struct CatalogAcl {
    /// Cache and load state.
    state: RwLock<State>,
    /// Catalog the cache is loaded from; None: inner tier off.
    catalog: Mutex<Option<CatalogHandle>>,
    /// Serialises loads, so an older snapshot never overwrites a newer one.
    refresh_lock: Mutex<()>,
}

impl std::fmt::Debug for CatalogAcl {
    /// The load state and cached consoles (the catalog handle has no Debug).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.read().map(|s| format!("{:?}", s.load)).unwrap_or_default();
        f.debug_struct("CatalogAcl").field("load", &state).finish_non_exhaustive()
    }
}

/// A snapshot of one console's catalog ACL for `.acl list`.
#[derive(Clone, Debug)]
pub struct ConsoleSnapshot {
    /// Load state at the time of the snapshot.
    pub load: LoadState,
    /// Whether the console is locked (F-4).
    pub locked: bool,
    /// The catalog lists of the last successful load.
    pub lists: ConsoleLists,
    /// The last successful load found no catalog table.
    pub table_missing: bool,
}

impl CatalogAcl {
    /// Attach the catalog the cache is loaded from. The state stays as it is
    /// until the next [`refresh`](Self::refresh); `main` refreshes right after.
    pub fn attach(&self, catalog: CatalogHandle) {
        if let Ok(mut slot) = self.catalog.lock() {
            *slot = Some(catalog);
        }
    }

    /// The attached catalog, if any.
    pub fn catalog(&self) -> Option<CatalogHandle> {
        self.catalog.lock().ok().and_then(|c| c.clone())
    }

    /// State of the last load.
    pub fn load_state(&self) -> LoadState {
        self.state.read().map(|s| s.load.clone()).unwrap_or_else(|_| LoadState::Failed("lock poisoned".into()))
    }

    /// Run `decide` with what the inner tier says about `console` and `kind`
    /// (no copy of the entries: this is on the path of every ACL check).
    pub fn with_restriction<R>(&self, console: &str, kind: Kind, decide: impl FnOnce(Restriction<'_>) -> R) -> R {
        if console == "*UserAgent*" {
            return decide(Restriction::None);
        }
        let Ok(state) = self.state.read() else { return decide(Restriction::Locked) };
        match &state.load {
            LoadState::Off => decide(Restriction::None),
            LoadState::Loaded => match state.entries.get(console).and_then(|lists| lists.get(&kind)) {
                Some(list) if !list.is_empty() => decide(Restriction::List(list)),
                _ => decide(Restriction::None),
            },
            LoadState::Failed(_) => {
                if Self::locked_in(&state, console) {
                    decide(Restriction::Locked)
                } else {
                    decide(Restriction::None)
                }
            }
        }
    }

    /// F-4 while a load failed: locked unless known to have no entries.
    fn locked_in(state: &State, console: &str) -> bool {
        matches!(state.load, LoadState::Failed(_))
            && console != "*UserAgent*"
            && (!state.ever_loaded
                || state.pending.contains(console)
                || state.entries.get(console).is_some_and(|lists| lists.values().any(|l| !l.is_empty())))
    }

    /// Whether the inner tier restricts `console` for `kind` at all
    /// (entries of that type, or locked). Used by `acl::has_restrictions`
    /// and `acl::no_restrictions`, which pick unfiltered SQL otherwise.
    pub fn restricts(&self, console: &str, kind: Kind) -> bool {
        self.with_restriction(console, kind, |r| !matches!(r, Restriction::None))
    }

    /// Snapshot of `console` for `.acl list`.
    pub fn snapshot(&self, console: &str) -> ConsoleSnapshot {
        match self.state.read() {
            Ok(state) => ConsoleSnapshot {
                load: state.load.clone(),
                locked: Self::locked_in(&state, console),
                lists: state.entries.get(console).cloned().unwrap_or_default(),
                table_missing: state.table_missing,
            },
            Err(_) => ConsoleSnapshot {
                load: LoadState::Failed("lock poisoned".into()),
                locked: true,
                lists: ConsoleLists::new(),
                table_missing: false,
            },
        }
    }

    /// Consoles with catalog entries in the last successful load.
    pub fn consoles(&self) -> Vec<String> {
        self.state.read().map(|s| s.entries.keys().cloned().collect()).unwrap_or_default()
    }

    /// Mark `console` as changed (no longer known to have no entries) before
    /// a write; the next successful load clears the mark.
    pub fn mark_pending(&self, console: &str) {
        if let Ok(mut state) = self.state.write() {
            state.pending.insert(console.to_string());
        }
    }

    /// Load the cache from the catalog. Without an attached catalog the
    /// inner tier is off. A failure keeps the entries of the last
    /// successful load (they tell which consoles are not known to be free
    /// of entries) and sets [`LoadState::Failed`].
    pub fn refresh(&self) -> LoadState {
        let _serial = self.refresh_lock.lock();
        let Some(catalog) = self.catalog() else {
            if let Ok(mut state) = self.state.write() {
                state.load = LoadState::Off;
            }
            return LoadState::Off;
        };
        let loaded = load(&catalog);
        let Ok(mut state) = self.state.write() else { return LoadState::Failed("lock poisoned".into()) };
        match loaded {
            Ok(Some(entries)) => {
                state.entries = entries;
                state.table_missing = false;
                state.ever_loaded = true;
                state.pending.clear();
                state.load = LoadState::Loaded;
            }
            Ok(None) if state.entries.is_empty() => {
                if !state.table_missing {
                    tracing::warn!("catalog ACL: table {TABLE} not found (schema extension v2253 missing), \
                        console ACLs come from the configuration files only");
                }
                state.table_missing = true;
                state.ever_loaded = true;
                state.pending.clear();
                state.load = LoadState::Loaded;
            }
            Ok(None) => {
                let error = format!("table {TABLE} disappeared, it held entries at the last load");
                tracing::error!("catalog ACL: {error}; consoles with catalog entries are limited to {}",
                    LOCKED_COMMANDS.join(", "));
                state.load = LoadState::Failed(error);
            }
            Err(error) => {
                if state.load != LoadState::Failed(error.clone()) {
                    tracing::error!("catalog ACL not readable: {error}; consoles with (possible) catalog \
                        entries are limited to {}", LOCKED_COMMANDS.join(", "));
                }
                state.load = LoadState::Failed(error);
            }
        }
        state.load.clone()
    }

    /// Replace the catalog lists of `console` for the given types in one
    /// transaction (an empty list removes the type) and reload the cache.
    /// The console is marked pending first, so a write whose reload fails
    /// leaves it locked rather than on stale entries (F-4).
    pub fn write(&self, console: &str, changes: &[(Kind, Vec<String>)], changed_by: &str) -> Result<(), String> {
        let catalog = self.catalog().ok_or("no catalog")?;
        self.mark_pending(console);
        let result = catalog.execute_within(&write_sql(console, changes, changed_by), CATALOG_TIMEOUT).map(|_| ());
        self.refresh();
        result
    }

    /// Copy of the entries of the last successful load (change detection
    /// of the refresh thread).
    fn entries_snapshot(&self) -> BTreeMap<String, ConsoleLists> {
        self.state.read().map(|s| s.entries.clone()).unwrap_or_default()
    }

    /// Replace the cache directly (tests).
    #[cfg(test)]
    pub(crate) fn set_for_test(&self, load: LoadState, entries: &[(&str, Kind, &[&str])]) {
        let mut state = self.state.write().unwrap();
        state.entries.clear();
        for (console, kind, list) in entries {
            state.entries.entry(console.to_string()).or_default().insert(*kind, list.iter()
                .map(|e| CatalogEntry { entry: e.to_string(), changed_by: "test".into(), changed_at: String::new() })
                .collect());
        }
        // As after an earlier successful load.
        state.ever_loaded = true;
        state.load = load;
    }
}

/// SQL string literal content (standard_conforming_strings: only `'` doubles).
fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The statements of [`CatalogAcl::write`]: one simple-protocol query string,
/// which PostgreSQL runs as one implicit transaction (no explicit BEGIN, so
/// a failure never leaves an open transaction on the pooled connection).
fn write_sql(console: &str, changes: &[(Kind, Vec<String>)], changed_by: &str) -> String {
    let mut sql = String::new();
    for (kind, list) in changes {
        sql.push_str(&format!("DELETE FROM {TABLE} WHERE console = {} AND acl_type = {};\n",
            quote(console), quote(kind.catalog_name())));
        for (position, entry) in list.iter().enumerate() {
            sql.push_str(&format!(
                "INSERT INTO {TABLE} (console, acl_type, position, entry, changed_by) VALUES ({}, {}, {position}, {}, {});\n",
                quote(console), quote(kind.catalog_name()), quote(entry), quote(changed_by)));
        }
    }
    sql
}

/// Read the whole table. `Ok(None)`: the table does not exist.
fn load(catalog: &CatalogHandle) -> Result<Option<BTreeMap<String, ConsoleLists>>, String> {
    let exists = catalog.query_rows_within(&format!("SELECT to_regclass('{TABLE}')::text"), CATALOG_TIMEOUT)?;
    if exists.rows.first().and_then(|r| r.first()).is_none_or(|v| v.is_none()) {
        return Ok(None);
    }
    let table = catalog.query_rows_within(&format!(
        "SELECT console, acl_type, position, entry, changed_by, changed_at FROM {TABLE} \
         ORDER BY console, acl_type, position"), CATALOG_TIMEOUT)?;
    let mut entries: BTreeMap<String, ConsoleLists> = BTreeMap::new();
    for row in &table.rows {
        let text = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
        let Some(kind) = Kind::from_catalog_name(&text(1)) else {
            // The table's CHECK constraint makes this impossible; a row of an
            // unknown type could not restrict a known type anyway.
            tracing::warn!("catalog ACL: ignoring row of console \"{}\" with unknown acl_type \"{}\"", text(0), text(1));
            continue;
        };
        entries.entry(text(0)).or_default().entry(kind).or_default()
            .push(CatalogEntry { entry: text(3), changed_by: text(4), changed_at: text(5) });
    }
    Ok(Some(entries))
}

/// Whether the refresh thread loads again after `waited` since its last
/// attempt: every [`RETRY_INTERVAL`] while the last load failed, every
/// [`REFRESH_INTERVAL`] while it succeeded, never without a catalog.
fn refresh_due(load: &LoadState, waited: Duration) -> bool {
    match load {
        LoadState::Off => false,
        LoadState::Failed(_) => waited >= RETRY_INTERVAL,
        LoadState::Loaded => waited >= REFRESH_INTERVAL,
    }
}

/// One attempt of the refresh thread: reload when [`refresh_due`]; logs a
/// recovery and a content change (rows edited by SQL; changes through
/// `.acl` reload at once and show no difference here). Returns whether it
/// loaded.
fn refresh_tick(store: &CatalogAcl, waited: Duration) -> bool {
    let before_state = store.load_state();
    if !refresh_due(&before_state, waited) {
        return false;
    }
    let before = store.entries_snapshot();
    if store.refresh() == LoadState::Loaded {
        if matches!(before_state, LoadState::Failed(_)) {
            tracing::info!("catalog ACL readable again, consoles use their catalog ACL");
        } else if store.entries_snapshot() != before {
            tracing::info!("catalog ACL changed in the catalog, reloaded ({} console(s) with entries)",
                store.consoles().len());
        }
    }
    true
}

/// Thread `dir:catalog-acl`: reloads the cache every [`RETRY_INTERVAL`] while
/// the last load failed and every [`REFRESH_INTERVAL`] while it succeeded
/// ([`refresh_tick`]), until `shutdown` is set.
pub fn spawn_refresh_thread(store: Arc<CatalogAcl>, shutdown: &'static std::sync::atomic::AtomicBool) {
    let spawned = std::thread::Builder::new().name("dir:catalog-acl".into()).spawn(move || {
        let step = Duration::from_millis(500);
        let mut waited = Duration::ZERO;
        while !shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(step);
            waited += step;
            if refresh_tick(&store, waited) {
                waited = Duration::ZERO;
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!("cannot start dir:catalog-acl: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogRequest, CatalogResponse, SqlTable};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A mock catalog with the given rows (console, type, position, entry);
    /// `fail` makes every query fail, `missing` reports no table.
    fn mock(rows: Vec<[&'static str; 4]>, fail: Arc<AtomicBool>, missing: bool) -> CatalogHandle {
        CatalogHandle::mock(move |request| {
            if fail.load(Ordering::SeqCst) {
                return CatalogResponse::Error("connection refused".into());
            }
            match request {
                CatalogRequest::TextRows { sql } if sql.contains("to_regclass") => CatalogResponse::Rows(SqlTable {
                    columns: vec![],
                    rows: vec![vec![if missing { None } else { Some(TABLE.into()) }]],
                }),
                CatalogRequest::TextRows { .. } => CatalogResponse::Rows(SqlTable {
                    columns: vec![],
                    rows: rows.iter().map(|[c, t, p, e]| vec![Some(c.to_string()), Some(t.to_string()),
                        Some(p.to_string()), Some(e.to_string()), Some("admin".into()), Some("now".into())]).collect(),
                }),
                _ => CatalogResponse::Ok(1),
            }
        })
    }

    /// F-4 through the real load path: never loaded locks every console,
    /// a later failure locks consoles with entries or a pending change only.
    #[test]
    fn load_failure_locks_consoles_with_or_without_known_state() {
        let fail = Arc::new(AtomicBool::new(true));
        let store = CatalogAcl::default();
        // Off without a catalog.
        assert_eq!(store.refresh(), LoadState::Off);
        assert!(!store.restricts("op", Kind::Command));
        store.attach(mock(vec![["op", "command", "0", "!delete"], ["op", "command", "1", "*all*"]], fail.clone(), false));
        // Never loaded: every console is locked, the default console is not.
        assert!(matches!(store.refresh(), LoadState::Failed(_)));
        assert!(store.restricts("op", Kind::Job) && store.restricts("viewer", Kind::Job));
        assert!(!store.restricts("*UserAgent*", Kind::Job));
        // Loaded: op restricted for commands only, viewer not at all.
        fail.store(false, Ordering::SeqCst);
        assert_eq!(store.refresh(), LoadState::Loaded);
        assert!(store.restricts("op", Kind::Command) && !store.restricts("op", Kind::Job));
        assert!(!store.restricts("viewer", Kind::Command));
        // Failing again: op (has entries) locked for every type, viewer known free.
        fail.store(true, Ordering::SeqCst);
        assert!(matches!(store.refresh(), LoadState::Failed(_)));
        assert!(store.with_restriction("op", Kind::Job, |r| matches!(r, Restriction::Locked)));
        assert!(!store.restricts("viewer", Kind::Command));
        // A change started on viewer makes it unknown.
        store.mark_pending("viewer");
        assert!(store.restricts("viewer", Kind::Command));
        fail.store(false, Ordering::SeqCst);
        assert_eq!(store.refresh(), LoadState::Loaded);
        assert!(!store.restricts("viewer", Kind::Command));
    }

    /// A missing table is "no entries" on a fresh catalog, but a failure
    /// once entries were seen.
    #[test]
    fn missing_table_is_empty_only_while_no_entries_were_seen() {
        let store = CatalogAcl::default();
        store.attach(mock(vec![], Arc::default(), true));
        assert_eq!(store.refresh(), LoadState::Loaded);
        assert!(store.snapshot("op").table_missing);
        assert!(!store.restricts("op", Kind::Command));
        let store = CatalogAcl::default();
        store.set_for_test(LoadState::Loaded, &[("op", Kind::Command, &["!delete"])]);
        store.attach(mock(vec![], Arc::default(), true));
        assert!(matches!(store.refresh(), LoadState::Failed(_)));
        assert!(store.restricts("op", Kind::Job));
    }

    /// The refresh thread's schedule: failed loads retry every 30 s, a
    /// healthy cache reloads every 60 s, no catalog never.
    #[test]
    fn refresh_schedule() {
        assert!(!refresh_due(&LoadState::Off, Duration::from_secs(3600)));
        assert!(!refresh_due(&LoadState::Failed("x".into()), Duration::from_secs(29)));
        assert!(refresh_due(&LoadState::Failed("x".into()), RETRY_INTERVAL));
        assert!(!refresh_due(&LoadState::Loaded, Duration::from_secs(59)));
        assert!(refresh_due(&LoadState::Loaded, REFRESH_INTERVAL));
    }

    /// Rows changed by SQL (behind the Director's back) take effect at the
    /// next periodic tick, without reload.
    #[test]
    fn periodic_refresh_picks_up_rows_changed_by_sql() {
        let rows: Arc<Mutex<Vec<[&'static str; 4]>>> = Arc::default();
        let source = rows.clone();
        let store = CatalogAcl::default();
        store.attach(CatalogHandle::mock(move |request| match request {
            CatalogRequest::TextRows { sql } if sql.contains("to_regclass") => CatalogResponse::Rows(SqlTable {
                columns: vec![], rows: vec![vec![Some(TABLE.into())]] }),
            CatalogRequest::TextRows { .. } => CatalogResponse::Rows(SqlTable {
                columns: vec![],
                rows: source.lock().unwrap().iter().map(|[c, t, p, e]| vec![Some(c.to_string()), Some(t.to_string()),
                    Some(p.to_string()), Some(e.to_string()), Some("sql".into()), Some("now".into())]).collect(),
            }),
            _ => CatalogResponse::Ok(0),
        }));
        assert_eq!(store.refresh(), LoadState::Loaded);
        assert!(!store.restricts("op", Kind::Command));
        rows.lock().unwrap().push(["op", "command", "0", "status"]);
        assert!(!refresh_tick(&store, Duration::from_secs(30)), "not yet due");
        assert!(!store.restricts("op", Kind::Command));
        assert!(refresh_tick(&store, REFRESH_INTERVAL));
        assert!(store.restricts("op", Kind::Command));
    }

    /// During a catalog outage a load and an `.acl` write fail within
    /// [`CATALOG_TIMEOUT`] (plus grace), not after the pool's 30 s.
    #[test]
    fn outage_answers_within_the_catalog_timeout() {
        let store = CatalogAcl::default();
        store.attach(CatalogHandle::mock(|_| {
            std::thread::sleep(Duration::from_secs(40));
            CatalogResponse::Error("pool timed out while waiting for an open connection".into())
        }));
        let start = std::time::Instant::now();
        assert_eq!(store.refresh(), LoadState::Failed("catalog did not answer within 5 s".into()));
        let error = store.write("op", &[(Kind::Job, vec!["a".into()])], "admin").unwrap_err();
        assert_eq!(error, "catalog did not answer within 5 s");
        let limit = 3 * (CATALOG_TIMEOUT + crate::catalog_worker::WITHIN_GRACE) + Duration::from_secs(3);
        assert!(start.elapsed() < limit, "{:?}", start.elapsed());
        assert!(store.restricts("op", Kind::Command), "the console of the failed write is locked");
    }

    /// The write replaces whole types and quotes every literal.
    #[test]
    fn write_sql_replaces_types_and_quotes() {
        let sql = write_sql("o'p", &[(Kind::Job, vec!["a".into(), "!b'c".into()]), (Kind::Pool, vec![])], "adm");
        assert_eq!(sql, "DELETE FROM onesimus.console_acl WHERE console = 'o''p' AND acl_type = 'job';\n\
            INSERT INTO onesimus.console_acl (console, acl_type, position, entry, changed_by) VALUES ('o''p', 'job', 0, 'a', 'adm');\n\
            INSERT INTO onesimus.console_acl (console, acl_type, position, entry, changed_by) VALUES ('o''p', 'job', 1, '!b''c', 'adm');\n\
            DELETE FROM onesimus.console_acl WHERE console = 'o''p' AND acl_type = 'pool';\n");
    }
}
