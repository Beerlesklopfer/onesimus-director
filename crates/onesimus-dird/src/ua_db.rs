//! Catalog selection of a console session.
//!
//! Port of C++ dird/ua_db.cc OpenClientDb()/OpenDb()/CloseDb() and
//! ua_select.cc get_catalog_resource(). The selected catalog and the opened
//! shared/private connections are per session state (SessionCatalog); the
//! first opening of a connection reports `Using Catalog "<name>"` and an
//! implicit selection `Automatically selected Catalog: <name>` — both only
//! for human (non-API) consoles.

use crate::acl::{self, Kind};
use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;
use crate::session_manager::SessionCatalog;

/// Messages and outcome of an open request (SendMsg text, ErrorMsg text).
#[derive(Debug, Default)]
struct Opening {
    out: String,
    err: String,
}

impl Opening {
    fn finish(self, ok: bool) -> Result<String, CommandResult> {
        if ok {
            Ok(self.out)
        } else {
            let mut r = CommandResult::error(self.err);
            r.text_output = self.out;
            Err(r)
        }
    }
}

/// Configuration data needed for the selection (copied out of the lock).
struct Resources {
    catalogs: Vec<(String, String)>, // (name, db_name) in resource order
    clients: Vec<(String, String)>,  // (name, catalog)
    jobs: Vec<(String, String)>,     // (name, client)
}

impl Resources {
    fn load(ctx: &ExecContext) -> Option<Self> {
        let config = ctx.config.as_ref()?.read().ok()?;
        Some(Self {
            catalogs: config.catalogs.iter().map(|c| (c.name.clone(), c.db_name.clone())).collect(),
            clients: config.clients.iter().map(|c| (c.name.clone(), c.catalog.clone())).collect(),
            jobs: config.jobs.iter().map(|j| (j.name.clone(), j.client.clone())).collect(),
        })
    }

    fn catalog(&self, name: &str) -> Option<&str> {
        self.catalogs.iter().find(|c| c.0 == name).map(|c| c.0.as_str())
    }

    /// Catalog of a client; a client without Catalog directive uses the
    /// first catalog (C++ dird.cc CheckResources).
    fn client_catalog(&self, client: &str) -> Option<String> {
        let (_, catalog) = self.clients.iter().find(|c| c.0 == client)?;
        if catalog.is_empty() {
            self.catalogs.first().map(|c| c.0.clone())
        } else {
            Some(catalog.clone())
        }
    }
}

/// C++ FindArgWithValue(): first argument with that keyword and a value.
fn find_arg_with_value<'a>(ast: &'a CommandAst, key: &str) -> Option<&'a str> {
    ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key) && a.value.is_some())
        .and_then(|a| a.value.as_deref())
}

/// C++ OpenClientDb(): choose the catalog from `catalog=`, `client=` or
/// `job=` (in that order), then open the connection. Ok carries the text to
/// put in front of the command output; Err the result to return instead.
pub fn open_client_db(ast: &CommandAst, ctx: &ExecContext, use_private: bool) -> Result<String, CommandResult> {
    let Some(res) = Resources::load(ctx) else { return Ok(String::new()) };
    let mut op = Opening::default();

    // Try for catalog keyword
    if let Some(name) = find_arg_with_value(ast, "catalog") {
        if let Some(catalog) = res.catalog(name).filter(|c| acl::access(ctx, Kind::Catalog, c)) {
            let catalog = catalog.to_string();
            select(ctx, &catalog);
            let ok = open(ast, ctx, &res, use_private, &mut op);
            return op.finish(ok);
        }
    }

    // Try for client keyword, then for Job keyword (catalog of the job's client)
    let client = find_arg_with_value(ast, "client")
        .filter(|c| acl::access(ctx, Kind::Client, c))
        .and_then(|c| res.client_catalog(c));
    let catalog = client.or_else(|| {
        find_arg_with_value(ast, "job")
            .filter(|j| acl::access(ctx, Kind::Job, j))
            .and_then(|j| res.jobs.iter().find(|job| job.0 == j))
            .filter(|job| !job.1.is_empty())
            .and_then(|job| res.client_catalog(&job.1))
    });
    if let Some(catalog) = catalog {
        if state(ctx, |s| s.name.as_ref().is_some_and(|n| *n != catalog)) == Some(true) {
            close_db(ctx);
        }
        if !acl::access(ctx, Kind::Catalog, &catalog) {
            op.err.push_str(&format!("No authorization for Catalog \"{catalog}\"\n"));
            return op.finish(false);
        }
        state(ctx, |s| s.name = Some(catalog.clone()));
        let ok = open(ast, ctx, &res, use_private, &mut op);
        return op.finish(ok);
    }

    let ok = open(ast, ctx, &res, use_private, &mut op);
    op.finish(ok)
}

/// C++ EstimateCmd(): `CloseDb(ua); ua->catalog = <client catalog>;
/// OpenDb(ua)` once job, client and fileset are known. (Pool resources
/// carry no Catalog directive in this configuration model yet.)
pub fn reopen_client_db(ast: &CommandAst, ctx: &ExecContext, client: &str) -> Result<String, CommandResult> {
    let Some(res) = Resources::load(ctx) else { return Ok(String::new()) };
    close_db(ctx);
    if let Some(catalog) = res.client_catalog(client) {
        state(ctx, |s| s.name = Some(catalog));
    }
    let mut op = Opening::default();
    let ok = open(ast, ctx, &res, false, &mut op);
    op.finish(ok)
}

/// C++ OpenDb(): open the session catalog (`catalog=` or the selection),
/// without looking at `client=`/`job=`.
pub fn open_db(ast: &CommandAst, ctx: &ExecContext) -> Result<String, CommandResult> {
    let Some(res) = Resources::load(ctx) else { return Ok(String::new()) };
    let mut op = Opening::default();
    let ok = open(ast, ctx, &res, false, &mut op);
    op.finish(ok)
}

/// C++ CloseDb(): drop the connections; the selected catalog stays.
pub fn close_db(ctx: &ExecContext) {
    state(ctx, |s| {
        s.shared_open = false;
        s.private_open = false;
    });
}

/// Run a handler whose C++ counterpart starts with
/// `if (!OpenClientDb(ua, use_private)) { return ...; }`.
pub fn with_client_db(ast: &CommandAst, ctx: &ExecContext, use_private: bool, handler: impl FnOnce() -> CommandResult) -> CommandResult {
    match open_client_db(ast, ctx, use_private) {
        Ok(prefix) => with_prefix(prefix, handler()),
        Err(result) => result,
    }
}

/// C++ ua_cmds.cc use_cmd(): `use [catalog=<name>]` switches the session
/// catalog and reopens the connection.
pub fn use_catalog(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let Some(res) = Resources::load(ctx) else { return CommandResult::ok(String::new()) };
    let mut op = Opening::default();
    close_db(ctx);
    let gui = state(ctx, |s| s.gui).unwrap_or(false);
    if let Some(catalog) = get_catalog_resource(ast, ctx, &res, gui, &mut op) {
        state(ctx, |s| s.name = Some(catalog));
    }
    if open(ast, ctx, &res, false, &mut op) {
        if let Some(name) = state(ctx, |s| s.name.clone()).flatten() {
            let db_name = res.catalogs.iter().find(|c| c.0 == name).map(|c| c.1.as_str()).unwrap_or("");
            op.out.push_str(&format!("Using Catalog name={name} DB={db_name}\n"));
        }
    }
    // use_cmd() always returns true; errors are only messages.
    let mut result = CommandResult::ok(op.out);
    result.error_message = op.err;
    result
}

/// Put the catalog messages in front of a command result.
pub fn with_prefix(prefix: String, mut result: CommandResult) -> CommandResult {
    if !prefix.is_empty() {
        result.text_output.insert_str(0, &prefix);
    }
    result
}

/// Run `f` with the session's catalog state (None for internal contexts).
fn state<R>(ctx: &ExecContext, f: impl FnOnce(&mut SessionCatalog) -> R) -> Option<R> {
    ctx.session_mgr.as_ref()?.with_session_catalog(ctx.session_id, f)
}

/// `ua->catalog = catalog`, closing the connections of another catalog first.
fn select(ctx: &ExecContext, catalog: &str) {
    state(ctx, |s| {
        if s.name.as_deref().is_some_and(|n| n != catalog) {
            s.shared_open = false;
            s.private_open = false;
        }
        s.name = Some(catalog.to_string());
    });
}

/// SendMsg/DoPrompt messages are suppressed for API consoles (`ua->api`).
fn quiet(ctx: &ExecContext) -> bool {
    ctx.api_mode != ApiMode::Human
}

fn open(ast: &CommandAst, ctx: &ExecContext, res: &Resources, use_private: bool, op: &mut Opening) -> bool {
    // Internal contexts (runscripts, no console session) have no state and
    // print nothing (C++ `ua->runscript`).
    let Some(current) = state(ctx, |s| s.clone()) else { return true };

    // See if we need to do any work at all.
    if (use_private && current.private_open) || (!use_private && current.shared_open) {
        return true;
    }

    // Select the right catalog to use.
    let catalog = match current.name {
        Some(name) => name,
        None => match get_catalog_resource(ast, ctx, res, current.gui, op) {
            Some(name) => {
                state(ctx, |s| s.name = Some(name.clone()));
                name
            }
            None => {
                op.err.push_str("Could not find a Catalog resource\n");
                return false;
            }
        },
    };

    if ctx.catalog.is_none() {
        let db_name = res.catalogs.iter().find(|c| c.0 == catalog).map(|c| c.1.as_str()).unwrap_or("");
        op.err.push_str(&format!("Could not open catalog database \"{db_name}\".\n"));
        return false;
    }
    state(ctx, |s| if use_private { s.private_open = true } else { s.shared_open = true });

    if !quiet(ctx) {
        op.out.push_str(&format!("Using Catalog \"{catalog}\"\n"));
    }
    true
}

/// C++ ua_select.cc get_catalog_resource().
fn get_catalog_resource(ast: &CommandAst, ctx: &ExecContext, res: &Resources, gui: bool, op: &mut Opening) -> Option<String> {
    for arg in &ast.arguments {
        if arg.key.eq_ignore_ascii_case("catalog") {
            if let Some(name) = arg.value.as_deref() {
                if let Some(catalog) = res.catalog(name).filter(|c| acl::access(ctx, Kind::Catalog, c)) {
                    return Some(catalog.to_string());
                }
            }
        }
    }

    if gui {
        let Some((first, _)) = res.catalogs.first() else {
            op.err.push_str("Could not find a Catalog resource\n");
            return None;
        };
        if !acl::access(ctx, Kind::Catalog, first) {
            op.err.push_str("You must specify a \"use <catalog-name>\" command before continuing.\n");
            return None;
        }
        return Some(first.clone());
    }

    // StartPrompt/AddPrompt over the permitted catalogs, then DoPrompt().
    let allowed: Vec<&String> = res.catalogs.iter()
        .map(|c| &c.0)
        .filter(|c| acl::access(ctx, Kind::Catalog, c))
        .collect();
    match allowed.as_slice() {
        [] => {
            op.out.push_str("The defined Catalog resources are:\n");
            op.err.push_str("Selection list for \"Catalog\" is empty!\n");
            None
        }
        [only] => {
            if !quiet(ctx) {
                op.out.push_str(&format!("Automatically selected Catalog: {only}\n"));
            }
            Some((*only).clone())
        }
        // Interactive selection (BNET_SUB_PROMPT) is not available yet
        // (compatibility log C-011); use the first permitted catalog.
        [first, ..] => Some((*first).clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    use crate::catalog_worker::{CatalogHandle, CatalogResponse};
    use crate::command_ast::Argument;
    use crate::director_config::DirectorConfig;
    use crate::session_manager::SessionManager;

    const CONFIG: &str = r#"
Director { Name=dir; Password=secret }
Catalog { Name=MyCatalog; DbName=bareos }
Catalog { Name=Other; DbName=other }
Client { Name=fd1; Password=x; Catalog=Other }
Client { Name=fd2; Password=x }
Job { Name=job1; Client=fd1 }
Console { Name=limited; Password=pw; CommandACL=*all*; CatalogACL=Other; ClientACL=*all*; JobACL=*all* }
Console { Name=nocat; Password=pw; CommandACL=*all*; ClientACL=*all*; JobACL=*all* }
Console { Name=mycat; Password=pw; CommandACL=*all*; CatalogACL=MyCatalog; ClientACL=*all*; JobACL=*all* }
"#;

    fn context(console: &str) -> ExecContext {
        let manager = Arc::new(SessionManager::new(Arc::new(crate::message_bus::MessageBus::new(16))));
        let id = manager.create_session(console);
        manager.authenticate_session(id);
        ExecContext {
            session_id: id,
            config: Some(Arc::new(RwLock::new(DirectorConfig::from_str(CONFIG).unwrap()))),
            session_mgr: Some(manager),
            catalog: Some(CatalogHandle::mock(|_| CatalogResponse::Ok(0))),
            api_mode: ApiMode::Human,
            bus_rx: None,
            job_manager: None,
        }
    }

    fn ast(args: &[(&str, &str)]) -> CommandAst {
        CommandAst {
            command_name: "list".into(),
            arguments: args.iter().map(|(k, v)| Argument { key: k.to_string(), value: Some(v.to_string()) }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn first_access_selects_automatically_and_reports_once_per_connection() {
        let ctx = context("mycat");
        assert_eq!(open_client_db(&ast(&[]), &ctx, true).unwrap(),
            "Automatically selected Catalog: MyCatalog\nUsing Catalog \"MyCatalog\"\n");
        // Same (private) connection: silent.
        assert_eq!(open_client_db(&ast(&[]), &ctx, true).unwrap(), "");
        // Shared connection is opened separately; catalog already selected.
        assert_eq!(open_client_db(&ast(&[]), &ctx, false).unwrap(), "Using Catalog \"MyCatalog\"\n");
        assert_eq!(open_client_db(&ast(&[]), &ctx, false).unwrap(), "");
        // CloseDb keeps the catalog but reopens with a message.
        close_db(&ctx);
        assert_eq!(open_client_db(&ast(&[]), &ctx, false).unwrap(), "Using Catalog \"MyCatalog\"\n");
    }

    #[test]
    fn client_and_job_select_catalog_without_automatic_message() {
        let ctx = context("*UserAgent*");
        assert_eq!(open_client_db(&ast(&[("client", "fd2")]), &ctx, true).unwrap(), "Using Catalog \"MyCatalog\"\n");
        // Another catalog closes the open connection first.
        assert_eq!(open_client_db(&ast(&[("job", "job1")]), &ctx, true).unwrap(), "Using Catalog \"Other\"\n");
        assert_eq!(open_client_db(&ast(&[("catalog", "Other")]), &ctx, true).unwrap(), "");
        assert_eq!(open_client_db(&ast(&[("catalog", "MyCatalog")]), &ctx, true).unwrap(), "Using Catalog \"MyCatalog\"\n");
    }

    #[test]
    fn several_catalogs_are_filtered_by_acl() {
        let ctx = context("limited");
        assert_eq!(open_client_db(&ast(&[]), &ctx, false).unwrap(),
            "Automatically selected Catalog: Other\nUsing Catalog \"Other\"\n");
        // client=fd2 uses MyCatalog, which this console may not access.
        let ctx = context("limited");
        let err = open_client_db(&ast(&[("client", "fd2")]), &ctx, false).unwrap_err();
        assert_eq!(err.error_message, "No authorization for Catalog \"MyCatalog\"\n");
    }

    #[test]
    fn no_permitted_catalog() {
        let ctx = context("nocat");
        let err = open_client_db(&ast(&[]), &ctx, false).unwrap_err();
        assert_eq!(err.text_output, "The defined Catalog resources are:\n");
        assert_eq!(err.error_message, "Selection list for \"Catalog\" is empty!\nCould not find a Catalog resource\n");
    }

    #[test]
    fn api_mode_and_gui_mode_are_silent() {
        let mut ctx = context("mycat");
        ctx.api_mode = ApiMode::Json;
        assert_eq!(open_client_db(&ast(&[]), &ctx, true).unwrap(), "");
        ctx.api_mode = ApiMode::Human;
        // Already open in this session: nothing more.
        assert_eq!(open_client_db(&ast(&[]), &ctx, true).unwrap(), "");

        let ctx = context("*UserAgent*");
        ctx.session_mgr.as_ref().unwrap().set_session_gui(ctx.session_id, true);
        assert_eq!(open_client_db(&ast(&[]), &ctx, false).unwrap(), "Using Catalog \"MyCatalog\"\n");
    }

    #[test]
    fn internal_context_has_no_messages() {
        let mut ctx = context("mycat");
        ctx.session_mgr = None;
        assert_eq!(open_client_db(&ast(&[]), &ctx, true).unwrap(), "");
    }
}
