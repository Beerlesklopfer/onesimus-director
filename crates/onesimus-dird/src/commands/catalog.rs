//! Catalog management commands.
//!
//! AST-basierte Katalog-Verwaltung, erreichbar via:
//! - CLI: `.catalog schema`, `.catalog migrate`, `.catalog version`
//! - JSON-API: POST /api/catalog/schema
//! - gRPC: CatalogService.GetSchema, CatalogService.Migrate

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;

pub fn cmd_dot_catalog(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let action = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");

    match action {
        "schema" => catalog_schema(ast, ctx),
        "version" => catalog_version(ast, ctx),
        "migrate" => catalog_migrate(ast, ctx),
        "validate" => catalog_validate(ast, ctx),
        "ddl" => catalog_ddl(ast, ctx),
        "tables" => catalog_tables(ast, ctx),
        "describe" => catalog_describe(ast, ctx),
        "codegen" => catalog_codegen(ast, ctx),
        "diff" => catalog_diff(ast, ctx),
        "drop" | "create" | "reset" => catalog_admin(action, ast, ctx, latest_schema()),
        _ => CommandResult::ok(
            ".catalog: Katalog-Schema-Verwaltung\n\
             \n\
             Befehle:\n\
             .catalog schema              Schema als JSON anzeigen\n\
             .catalog version             Aktuelle Schema-Version\n\
             .catalog tables              Alle Tabellen auflisten\n\
             .catalog describe table=<n>  Tabellen-Spalten anzeigen\n\
             .catalog ddl                 CREATE TABLE SQL generieren\n\
             .catalog validate            Schema gegen DB validieren\n\
             .catalog migrate             Schema-Migration durchfuehren\n\
             .catalog diff                Aenderungen zur naechsten Version\n\
             .catalog codegen             Rust-Structs generieren\n\
             \n\
             Admin-Befehle (CommandACL muss \".catalog\" ausdruecklich nennen;\n\
             ohne \"yes confirm=<Datenbank>\" nur Vorschau):\n\
             .catalog create              Tabellen in DB erstellen\n\
             .catalog drop table=<name>   Tabelle loeschen (force bei Job-Records)\n\
             .catalog reset               Alle Tabellen loeschen + neu erstellen (force bei Job-Records)\n"
                .to_string()
        ),
    }
}

fn catalog_schema(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let schema_dir = &schema_dir();
    let migrator = bareos_orm::migration::Migrator::new(schema_dir);

    match migrator.latest() {
        Ok(Some(schema)) => {
            match serde_json::to_string_pretty(&schema) {
                Ok(json) => CommandResult::ok(format!("{json}\n")),
                Err(e) => CommandResult::error(format!("JSON error: {e}\n")),
            }
        }
        Ok(None) => CommandResult::ok(format!("Kein Schema gefunden in {}\n", schema_dir.display())),
        Err(e) => CommandResult::error(format!("Schema load error: {e}\n")),
    }
}

fn catalog_version(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let schema_dir = &schema_dir();
    let migrator = bareos_orm::migration::Migrator::new(schema_dir);

    match migrator.load_versions() {
        Ok(versions) => {
            if versions.is_empty() {
                return CommandResult::ok("Keine Schema-Versionen gefunden\n".to_string());
            }
            let mut output = String::from("Schema-Versionen:\n");
            for v in &versions {
                output.push_str(&format!(
                    "  v{:03}: {} ({})\n",
                    v.version, v.description, v.created
                ));
            }
            output.push_str(&format!("\nAktuelle Version: v{:03}\n",
                versions.last().map(|v| v.version).unwrap_or(0)));
            CommandResult::ok(output)
        }
        Err(e) => CommandResult::error(format!("Version error: {e}\n")),
    }
}

fn catalog_tables(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let schema_dir = &schema_dir();
    let migrator = bareos_orm::migration::Migrator::new(schema_dir);

    match migrator.latest() {
        Ok(Some(schema)) => {
            let mut output = format!("Tabellen in Schema v{:03}:\n\n", schema.version);
            output.push_str(&format!("{:<20} {:<10} {:<10} {}\n",
                "Tabelle", "Spalten", "Indizes", "Beschreibung"));
            output.push_str(&"-".repeat(70));
            output.push('\n');

            for table in &schema.tables {
                output.push_str(&format!("{:<20} {:<10} {:<10} {}\n",
                    table.name,
                    table.columns.len(),
                    table.indices.len(),
                    table.description,
                ));
            }
            CommandResult::ok(output)
        }
        Ok(None) => CommandResult::ok("Kein Schema\n".to_string()),
        Err(e) => CommandResult::error(format!("{e}\n")),
    }
}

fn catalog_describe(ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let table_name = ast.arguments.iter()
        .find(|a| a.key == "table")
        .and_then(|a| a.value.as_deref())
        .unwrap_or("");

    if table_name.is_empty() {
        return CommandResult::ok(".catalog describe table=<name>\n".to_string());
    }

    let schema_dir = &schema_dir();
    let migrator = bareos_orm::migration::Migrator::new(schema_dir);

    match migrator.latest() {
        Ok(Some(schema)) => {
            match schema.find_table(table_name) {
                Some(table) => {
                    let mut output = format!("Tabelle: {} (PK: {})\n{}\n\n",
                        table.name, table.primary_key, table.description);

                    output.push_str(&format!("{:<20} {:<15} {:<10} {:<15} {}\n",
                        "Spalte", "Typ", "NOT NULL", "Default", "Beschreibung"));
                    output.push_str(&"-".repeat(80));
                    output.push('\n');

                    for col in &table.columns {
                        output.push_str(&format!("{:<20} {:<15} {:<10} {:<15} {}\n",
                            col.name,
                            col.col_type,
                            if col.not_null { "YES" } else { "no" },
                            col.default.as_deref().unwrap_or("-"),
                            col.description,
                        ));
                    }

                    if !table.indices.is_empty() {
                        output.push_str("\nIndizes:\n");
                        for idx in &table.indices {
                            output.push_str(&format!("  {} ({}) {}{}\n",
                                idx.name,
                                idx.columns.join(", "),
                                if idx.unique { "UNIQUE " } else { "" },
                                if idx.index_type != "btree" { &idx.index_type } else { "" },
                            ));
                        }
                    }

                    if !table.foreign_keys.is_empty() {
                        output.push_str("\nForeign Keys:\n");
                        for fk in &table.foreign_keys {
                            output.push_str(&format!("  {} → {}.{}\n",
                                fk.column, fk.references_table, fk.references_column));
                        }
                    }

                    CommandResult::ok(output)
                }
                None => CommandResult::error(format!("Tabelle '{table_name}' nicht gefunden\n")),
            }
        }
        Ok(None) => CommandResult::ok("Kein Schema\n".to_string()),
        Err(e) => CommandResult::error(format!("{e}\n")),
    }
}

fn catalog_ddl(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let schema_dir = &schema_dir();
    let migrator = bareos_orm::migration::Migrator::new(schema_dir);

    match migrator.full_ddl() {
        Ok(ddl) => CommandResult::ok(ddl),
        Err(e) => CommandResult::error(format!("{e}\n")),
    }
}

fn catalog_validate(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    match &ctx.catalog {
        Some(_cat) => {
            // TODO: Connect to DB, compare schema
            CommandResult::ok("Schema-Validierung: DB-Verbindung erforderlich\n".to_string())
        }
        None => CommandResult::ok("Keine Catalog-DB konfiguriert\n".to_string()),
    }
}

fn catalog_migrate(_ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    match &ctx.catalog {
        Some(cat) => {
            // Check if version table exists
            match cat.raw_query("SELECT MAX(versionid) FROM version") {
                Ok(output) => CommandResult::ok(format!("Schema aktuell: {}\n", output.trim())),
                Err(e) => CommandResult::error(format!("Migration error: {e}\n")),
            }
        }
        None => CommandResult::ok("Keine Catalog-DB konfiguriert\n".to_string()),
    }
}

fn catalog_codegen(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let schema_dir = &schema_dir();
    let migrator = bareos_orm::migration::Migrator::new(schema_dir);

    match migrator.latest() {
        Ok(Some(schema)) => {
            let code = bareos_orm::codegen::generate_rust_models(&schema);
            CommandResult::ok(code)
        }
        Ok(None) => CommandResult::ok("Kein Schema\n".to_string()),
        Err(e) => CommandResult::error(format!("{e}\n")),
    }
}

fn catalog_diff(_ast: &CommandAst, _ctx: &ExecContext) -> CommandResult {
    let schema_dir = &schema_dir();
    let migrator = bareos_orm::migration::Migrator::new(schema_dir);

    match migrator.load_versions() {
        Ok(versions) if versions.len() >= 2 => {
            let prev = &versions[versions.len() - 2];
            let curr = &versions[versions.len() - 1];

            let stmts = bareos_orm::migration::Migrator::diff(prev, curr);
            if stmts.is_empty() {
                CommandResult::ok(format!(
                    "Keine Aenderungen zwischen v{:03} und v{:03}\n",
                    prev.version, curr.version
                ))
            } else {
                let mut output = format!(
                    "Aenderungen v{:03} → v{:03}:\n\n",
                    prev.version, curr.version
                );
                for stmt in &stmts {
                    output.push_str(stmt);
                    output.push('\n');
                }
                CommandResult::ok(output)
            }
        }
        Ok(_) => CommandResult::ok("Mindestens 2 Schema-Versionen noetig fuer diff\n".to_string()),
        Err(e) => CommandResult::error(format!("{e}\n")),
    }
}

/// Schema directory, the same one the startup migration uses: next to the
/// build tree in development, `/usr/lib/bareos/ddl/schema` when installed.
pub fn schema_dir() -> std::path::PathBuf {
    let installed = std::path::PathBuf::from("/usr/lib/bareos/ddl/schema");
    [
        // Dev: binary in target/<profile>/, schema in crates/bareos-orm/schema/
        std::env::current_exe().ok()
            .and_then(|p| p.parent()?.parent()?.parent().map(|d| d.join("crates/bareos-orm/schema"))),
        Some(installed.clone()),
    ].into_iter().flatten().find(|p| p.exists()).unwrap_or(installed)
}

/// Schema for `.catalog create|drop|reset`: every version folded together,
/// so `create` builds the Bareos base tables (Compatibility log C-052) and
/// not only the objects of the newest extension file.
fn latest_schema() -> Result<bareos_orm::schema::CatalogSchema, String> {
    let migrator = bareos_orm::migration::Migrator::new(&schema_dir());
    match migrator.combined() {
        Ok(Some(schema)) => Ok(schema),
        Ok(None) => Err("Kein Schema\n".to_string()),
        Err(e) => Err(format!("Schema error: {e}\n")),
    }
}

fn first_cell(table: crate::catalog_worker::SqlTable) -> Option<String> {
    table.rows.into_iter().next()?.into_iter().next()?
}

/// `.catalog create|drop|reset` — local extension without Bareos equivalent
/// (compatibility log C-040). Guard, in this order:
/// 1. the CommandACL must name `.catalog` literally (`*all*` is not enough),
/// 2. without `yes confirm=<database>` (the connected database's name) the
///    command only reports what it would do,
/// 3. dropping tables of a catalog with job records (or whose job table
///    cannot be read) additionally needs `force`.
/// Every attempt and every executed statement is logged with an `AUDIT` tag.
fn catalog_admin(
    action: &str,
    ast: &CommandAst,
    ctx: &ExecContext,
    schema: Result<bareos_orm::schema::CatalogSchema, String>,
) -> CommandResult {
    let session = ctx.session_id;
    let console = ctx.session_mgr.as_ref()
        .and_then(|m| m.authenticated_console(session))
        .unwrap_or_else(|| "*UserAgent*".to_string());
    if !crate::acl::lists_command(ctx, ".catalog") {
        tracing::warn!("AUDIT .catalog {action}: denied for console {console} (session {session}), CommandACL does not name .catalog");
        return CommandResult::error(format!(
            ".catalog {action}: denied, the console's CommandACL must name \".catalog\" explicitly (*all* is not sufficient)\n"
        ));
    }
    let schema = match schema {
        Ok(schema) => schema,
        Err(e) => return CommandResult::error(e),
    };
    let table = ast.arguments.iter().find(|a| a.key == "table").and_then(|a| a.value.as_deref()).unwrap_or("");
    let drops: Vec<String> = match action {
        "drop" if table.is_empty() => return CommandResult::ok(".catalog drop table=<name>\n".to_string()),
        // Only schema tables: the name goes verbatim into the SQL.
        "drop" if schema.find_table(table).is_none() => {
            return CommandResult::error(format!("Tabelle '{table}' nicht im Schema\n"))
        }
        "drop" => vec![table.to_string()],
        "reset" => schema.tables.iter().rev().map(|t| t.name.clone())
            .chain(std::iter::once("bareos_schema_version".to_string())).collect(),
        _ => Vec::new(),
    };
    let mut statements: Vec<String> = drops.iter().map(|t| format!("DROP TABLE IF EXISTS {t} CASCADE")).collect();
    if action != "drop" {
        statements.extend(schema.to_statements());
    }

    let Some(cat) = &ctx.catalog else {
        return CommandResult::ok("Keine Catalog-DB konfiguriert\n".to_string());
    };
    let database = match cat.query_rows("SELECT current_database()").map(first_cell) {
        Ok(Some(name)) => name,
        Ok(None) => return CommandResult::error(".catalog: Datenbankname nicht ermittelbar\n".to_string()),
        Err(e) => return CommandResult::error(format!(".catalog: {e}\n")),
    };
    let jobs = if drops.is_empty() {
        Some(0)
    } else {
        cat.query_rows("SELECT COUNT(*) FROM job").ok().and_then(first_cell).and_then(|n| n.parse::<u64>().ok())
    };

    let mut summary = format!("Katalog-Datenbank: {database}\n");
    if !drops.is_empty() {
        match jobs {
            Some(n) => summary.push_str(&format!("Job-Records: {n}\n")),
            None => summary.push_str("Job-Records: unbekannt (Tabelle job nicht lesbar)\n"),
        }
        summary.push_str(&format!("Zu loeschende Tabellen (CASCADE): {}\n", drops.join(", ")));
    }
    if statements.len() > drops.len() {
        summary.push_str(&format!("CREATE-Anweisungen: {}\n", statements.len() - drops.len()));
    }

    let confirmed = ast.arguments.iter().any(|a| a.key == "yes" && a.value.is_none())
        && ast.arguments.iter().any(|a| a.key == "confirm" && a.value.as_deref() == Some(database.as_str()));
    let forced = ast.arguments.iter().any(|a| a.key == "force" && a.value.is_none());
    if !confirmed {
        tracing::warn!("AUDIT .catalog {action}: preview for console {console} (session {session}) on database {database}, nothing changed");
        let table_arg = if action == "drop" { format!(" table={table}") } else { String::new() };
        let force_arg = if jobs != Some(0) { " force" } else { "" };
        return CommandResult::ok(format!(
            "{summary}Nichts geaendert. Ausfuehren mit: .catalog {action}{table_arg} yes confirm={database}{force_arg}\n"
        ));
    }
    if jobs != Some(0) && !forced {
        tracing::warn!("AUDIT .catalog {action}: refused for console {console} (session {session}) on database {database}, job records present and no force");
        return CommandResult::error(format!(
            "{summary}.catalog {action}: der Katalog enthaelt Job-Records (oder sie sind nicht lesbar); zusaetzlich \"force\" angeben\n"
        ));
    }

    tracing::warn!("AUDIT .catalog {action}: executing for console {console} (session {session}) on database {database}, {} statements, force={forced}", statements.len());
    for stmt in &statements {
        tracing::warn!("AUDIT .catalog {action}: {}", stmt.lines().next().unwrap_or(""));
        if let Err(e) = cat.raw_query(stmt) {
            tracing::warn!("AUDIT .catalog {action}: failed: {e}");
            return CommandResult::error(format!("{summary}.catalog {action} error: {e}\n"));
        }
    }
    CommandResult::ok(format!("{summary}.catalog {action}: {} Anweisungen ausgefuehrt\n", statements.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::{CatalogHandle, CatalogRequest, CatalogResponse, SqlColumn, SqlTable};
    use crate::command_ast::Argument;
    use std::sync::{Arc, Mutex, RwLock};

    fn schema() -> Result<bareos_orm::schema::CatalogSchema, String> {
        Ok(serde_json::from_value(serde_json::json!({
            "version": 1, "description": "test", "created": "2026-10-15", "author": "test",
            "min_compatible_version": null,
            "tables": [
                {"name": "client", "columns": [{"name": "clientid", "type": "serial"}], "primary_key": "clientid"},
                {"name": "job", "columns": [{"name": "jobid", "type": "serial"}], "primary_key": "jobid"}
            ]
        })).unwrap())
    }

    fn cell(value: &str) -> CatalogResponse {
        CatalogResponse::Rows(SqlTable {
            columns: vec![SqlColumn { name: "x".into(), numeric: false }],
            rows: vec![vec![Some(value.to_string())]],
        })
    }

    /// Catalog double: answers the two read queries, records every write and
    /// panics on it unless `writable`.
    fn catalog(jobs: &'static str, writable: bool) -> (CatalogHandle, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let handle = CatalogHandle::mock(move |request| match request {
            CatalogRequest::TextRows { sql } if sql == "SELECT current_database()" => cell("scratch_db"),
            CatalogRequest::TextRows { sql } if sql == "SELECT COUNT(*) FROM job" => cell(jobs),
            CatalogRequest::RawQuery { sql } => {
                seen.lock().unwrap().push(sql.clone());
                assert!(writable, "unexpected catalog write: {sql}");
                CatalogResponse::Text(String::new())
            }
            _ => CatalogResponse::Error("unexpected request".into()),
        });
        (handle, log)
    }

    fn context(command_acl: &str, catalog: CatalogHandle) -> ExecContext {
        let config = crate::director_config::DirectorConfig::from_str(&format!(
            "Director {{Name=dir; Password=secret}}\n\
             Console {{Name=admin; Password=pw; CommandACL={command_acl}}}\n"
        )).unwrap();
        let manager = Arc::new(crate::session_manager::SessionManager::new(Arc::new(
            crate::message_bus::MessageBus::new(16),
        )));
        let id = manager.create_session("admin");
        manager.authenticate_session(id);
        ExecContext {
            session_id: id,
            config: Some(Arc::new(RwLock::new(config))),
            session_mgr: Some(manager),
            catalog: Some(catalog),
            // JSON/API mode never implies confirmation.
            api_mode: crate::command_executor::ApiMode::Json,
            bus_rx: None,
            job_manager: None,
        }
    }

    fn ast(line: &str) -> CommandAst {
        let arguments = line.split_whitespace().map(|token| match token.split_once('=') {
            Some((k, v)) => Argument { key: k.into(), value: Some(v.into()) },
            None => Argument { key: token.into(), value: None },
        }).collect();
        CommandAst { command_name: ".catalog".into(), arguments, ..Default::default() }
    }

    fn run(line: &str, ctx: &ExecContext) -> CommandResult {
        let ast = ast(line);
        catalog_admin(&ast.arguments[0].key, &ast, ctx, schema())
    }

    #[test]
    fn denied_unless_command_acl_names_dot_catalog() {
        for acl in ["*all*", ".*", "status,.catalogs", "!.catalog,*all*"] {
            let (cat, log) = catalog("0", false);
            let ctx = context(acl, cat);
            for line in ["reset yes confirm=scratch_db force", "drop table=job yes confirm=scratch_db force", "create yes confirm=scratch_db"] {
                let result = run(line, &ctx);
                assert!(!result.success, "{acl}: {line}");
                assert!(result.error_message.contains("denied"), "{}", result.error_message);
            }
            assert!(log.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn without_confirmation_reports_and_changes_nothing() {
        let (cat, log) = catalog("3", false);
        let ctx = context("status,.catalog", cat);
        for line in [
            "reset",
            "reset yes",
            "reset confirm=scratch_db",
            "reset yes=no confirm=scratch_db force",
            "reset yes confirm=bareos force",
            "drop table=job force",
            "create yes confirm=other",
        ] {
            let result = run(line, &ctx);
            assert!(result.success, "{line}: {}", result.error_message);
            assert!(result.text_output.contains("Nichts geaendert"), "{line}: {}", result.text_output);
        }
        let preview = run("reset", &ctx).text_output;
        assert!(preview.contains("Katalog-Datenbank: scratch_db"));
        assert!(preview.contains("Job-Records: 3"));
        assert!(preview.contains("Zu loeschende Tabellen (CASCADE): job, client, bareos_schema_version"));
        assert!(preview.contains(".catalog reset yes confirm=scratch_db force"));
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn job_records_need_force_and_unknown_tables_are_refused() {
        let (cat, log) = catalog("3", false);
        let ctx = context(".catalog", cat);
        let result = run("reset yes confirm=scratch_db", &ctx);
        assert!(!result.success);
        assert!(result.error_message.contains("force"));
        let result = run("drop table=job;DROP yes confirm=scratch_db force", &ctx);
        assert!(!result.success);
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn executes_with_full_confirmation() {
        let (cat, log) = catalog("0", true);
        let ctx = context(".catalog", cat);
        let result = run("drop table=job yes confirm=scratch_db", &ctx);
        assert!(result.success, "{}", result.error_message);
        assert_eq!(*log.lock().unwrap(), vec!["DROP TABLE IF EXISTS job CASCADE".to_string()]);

        let (cat, log) = catalog("7", true);
        let ctx = context(".catalog", cat);
        let result = run("reset yes confirm=scratch_db force", &ctx);
        assert!(result.success, "{}", result.error_message);
        let log = log.lock().unwrap();
        assert_eq!(log[..3], [
            "DROP TABLE IF EXISTS job CASCADE".to_string(),
            "DROP TABLE IF EXISTS client CASCADE".to_string(),
            "DROP TABLE IF EXISTS bareos_schema_version CASCADE".to_string(),
        ]);
        assert!(log.len() > 3 && log[3..].iter().all(|s| s.starts_with("CREATE")));
    }

    #[test]
    fn registry_dispatch_with_all_acl_never_reaches_the_catalog() {
        // Do_a_command prefix matching resolves `.catalog` to the Bareos
        // command `.catalogs` first; the handler guard above is defence in
        // depth for every other caller.
        let (cat, log) = catalog("0", false);
        let ctx = context("*all*", cat);
        let result = crate::command_executor::dispatch(&ast("reset yes confirm=scratch_db force"), &ctx);
        assert!(!result.text_output.contains("Katalog-Datenbank"), "{}", result.text_output);
        assert!(log.lock().unwrap().is_empty());
    }
}

