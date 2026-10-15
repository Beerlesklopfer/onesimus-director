//! SQL commands: .sql, sqlquery
//!
//! Execute raw SQL against the PostgreSQL catalog via CatalogHandle.

use crate::command_ast::CommandAst;
use crate::command_executor::ExecContext;
use crate::command_queue::CommandResult;

pub fn cmd_dot_sql(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotSqlCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_dot_sql_db(ast, ctx))
}

fn cmd_dot_sql_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let query = ast.arguments.iter()
        .find(|a| a.key == "query")
        .and_then(|a| a.value.as_deref());

    let query = match query {
        Some(q) => q,
        None => {
            let sub = ast.arguments.first().map(|a| a.key.as_str()).unwrap_or("");
            match sub {
                "tables" => return sql_tables(ctx),
                "schema" => return sql_schema(ctx),
                _ => return CommandResult::ok(
                    ".sql: specify one of:\n  .sql query=\"SELECT ...\"\n  .sql tables\n  .sql schema\n"
                        .to_string()
                ),
            }
        }
    };

    execute_sql(query, ctx)
}

pub fn cmd_sql_query(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_query.cc SqlqueryCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, !ast.arguments.iter().any(|a| a.key.eq_ignore_ascii_case("sharedsession")), || cmd_sql_query_db(ast, ctx))
}

fn cmd_sql_query_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let query: String = ast.arguments.iter()
        .map(|a| {
            if let Some(v) = &a.value {
                format!("{}={}", a.key, v)
            } else {
                a.key.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    if query.is_empty() {
        return CommandResult::ok("sqlquery: enter SQL query\n".to_string());
    }

    execute_sql(&query, ctx)
}

fn execute_sql(query: &str, ctx: &ExecContext) -> CommandResult {
    match &ctx.catalog {
        Some(cat) => match cat.raw_query(query) {
            Ok(output) => CommandResult::ok(output),
            Err(e) => CommandResult::error(format!("SQL error: {e}\n")),
        },
        None => CommandResult::ok(format!("No catalog configured. Query: {query}\n")),
    }
}

fn sql_tables(ctx: &ExecContext) -> CommandResult {
    execute_sql(
        "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = 'public' ORDER BY tablename",
        ctx,
    )
}

fn sql_schema(ctx: &ExecContext) -> CommandResult {
    execute_sql(
        "SELECT table_name, column_name, data_type FROM information_schema.columns \
         WHERE table_schema = 'public' ORDER BY table_name, ordinal_position",
        ctx,
    )
}
