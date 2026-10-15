//! SQL commands: .sql, sqlquery
//!
//! Execute raw SQL against the PostgreSQL catalog via CatalogHandle. Results
//! are rendered like C++ cats BareosDb::ListSqlQuery (see sql_list.rs).

use serde_json::{Map, Value};

use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;
use crate::sql_list::{json_rows, render, ListType};

/// C++ cats BareosDb::ListSqlQuery(jcr, query, send, type, verbose):
/// Ok = ListResult text (table, or "No results to list.\n"), Err = errmsg
/// "Query failed: <PQerrorMessage>\n". With `verbose` the C++ code also
/// sends errmsg as decoration; callers emit the Err text accordingly.
pub fn list_sql_query(ctx: &ExecContext, sql: &str, ty: ListType, _verbose: bool) -> Result<String, String> {
    let Some(cat) = &ctx.catalog else {
        return Err("Query failed: no catalog connection\n".to_string());
    };
    match cat.query_rows(sql) {
        Ok(t) => Ok(render(&t, ty)),
        Err(e) => Err(format!("Query failed: {e}\n")),
    }
}

pub fn cmd_dot_sql(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // C++ ua_dotcmds.cc DotSqlCmd: OpenClientDb() before anything else.
    crate::ua_db::with_client_db(ast, ctx, true, || cmd_dot_sql_db(ast, ctx))
}

/// C++ ua_dotcmds.cc DotSqlCmd.
fn cmd_dot_sql_db(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    // FindArgWithValue(ua, "query")
    let query = ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case("query") && a.value.is_some())
        .and_then(|a| a.value.as_deref());
    let Some(query) = query else {
        return CommandResult::error("query keyword not found.\n".to_string());
    };
    let Some(cat) = &ctx.catalog else {
        return CommandResult::error("Could not open catalog database.\n".to_string());
    };
    match cat.query_rows(query) {
        Ok(t) => match ctx.api_mode {
            // API_MODE_ON: SqlQuery() with SqlHandler, "value\t" per field
            ApiMode::Api => CommandResult::ok(t.rows.iter()
                .filter(|row| !row.is_empty())
                .map(|row| row.iter().map(|v| format!("{}\t", v.as_deref().unwrap_or(""))).collect::<String>())
                .collect()).verbatim(),
            // ListSqlQuery(..., HORZ_LIST, false): ArrayStart("query") + ListResult
            _ => {
                let mut result = Map::new();
                result.insert("query".into(), Value::Array(json_rows(&t)));
                CommandResult::ok(render(&t, ListType::Horz)).with_json(Value::Object(result))
            }
        },
        Err(e) => {
            // SqlQuery() sets "Query failed: <query>: ERR=<msg>\n",
            // ListSqlQuery() "Query failed: <msg>\n"; DotSqlCmd wraps it.
            let errmsg = match ctx.api_mode {
                ApiMode::Api => format!("Query failed: {query}: ERR={e}\n"),
                _ => format!("Query failed: {e}\n"),
            };
            CommandResult::error(format!("Query failed: {}. ERR={errmsg}", ua_cmd(ast)))
                .with_error_parts()
        }
    }
}

/// ua->cmd, the command line as received (reconstructed from the AST;
/// values containing blanks are quoted as bconsole users type them).
fn ua_cmd(ast: &CommandAst) -> String {
    let mut cmd = ast.command_name.clone();
    for a in &ast.arguments {
        cmd.push(' ');
        cmd.push_str(&a.key);
        if let Some(v) = &a.value {
            if v.is_empty() || v.chars().any(char::is_whitespace) {
                cmd.push_str(&format!("=\"{v}\""));
            } else {
                cmd.push_str(&format!("={v}"));
            }
        }
    }
    cmd
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

/// `.sqltables` (onesimus extension, no Bareos counterpart)
pub fn cmd_sql_tables(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::ua_db::with_client_db(ast, ctx, true, || execute_sql(
        "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = 'public' ORDER BY tablename",
        ctx,
    ))
}

/// `.sqlschema` (onesimus extension, no Bareos counterpart)
pub fn cmd_sql_schema(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    crate::ua_db::with_client_db(ast, ctx, true, || execute_sql(
        "SELECT table_name, column_name, data_type FROM information_schema.columns \
         WHERE table_schema = 'public' ORDER BY table_name, ordinal_position",
        ctx,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_ast::Argument;

    #[test]
    fn ua_cmd_quotes_values_with_blanks() {
        let ast = CommandAst {
            command_name: ".sql".into(),
            arguments: vec![Argument { key: "query".into(), value: Some("selec nothing".into()) }],
            ..Default::default()
        };
        assert_eq!(ua_cmd(&ast), ".sql query=\"selec nothing\"");
    }
}
