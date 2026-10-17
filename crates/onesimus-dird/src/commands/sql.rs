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
            ApiMode::Api => CommandResult::ok(api1_rows(&t)).verbatim(),
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

/// C++ ua_query.cc SqlqueryCmd(): arguments other than `sharedsession`
/// are ignored; the queries are read in SQL query mode until a blank line
/// (or a cancelled input) ends it.
pub fn cmd_sql_query(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    sql_query_mode(&mut ua, ast, |ctx, sql| list_sql_query(ctx, sql, crate::sql_list::ListType::Horz, true));
    ua.finish(true)
}

/// The SqlqueryCmd loop. `run` executes one complete query.
fn sql_query_mode(ua: &mut crate::ua_select::Ua, ast: &CommandAst,
    mut run: impl FnMut(&ExecContext, &str) -> Result<String, String>) {
    let shared = crate::ua_select::find_arg(ast, "sharedsession").is_some();
    if !ua.open_client_db(ast, !shared) {
        return;
    }
    ua.send_msg("Entering SQL query mode.\nTerminate each query with a semicolon.\nTerminate query mode with a blank line.\n");
    let mut query = String::new();
    let mut prompt = "Enter SQL query: ";
    while let Some(line) = ua.get_cmd(prompt) {
        if line.is_empty() {
            break;
        }
        if !query.is_empty() {
            query.push(' ');
        }
        query.push_str(&line);
        if line.ends_with(';') {
            // The query keeps its semicolon (only ua->cmd is zapped).
            match run(ua.ctx, &query) {
                Ok(text) | Err(text) => ua.send_msg(text),
            }
            query.clear();
            prompt = "Enter SQL query: ";
        } else {
            prompt = "Add to SQL query: ";
        }
    }
    ua.send_msg("End query mode.\n");
}

/// C++ ua_query.cc QueryCmd(): the queries of the Director's QueryFile
/// (":title", "*prompt" lines, SQL ending in ';', '!' for a vertical list).
pub fn cmd_query(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    let path = query_file(ctx);
    query_dialog(&mut ua, ast, &path, |ctx, sql, ty| list_sql_query(ctx, sql, ty, true));
    ua.finish(true)
}

/// Director `QueryFile` (default of the Debian packages).
fn query_file(ctx: &ExecContext) -> String {
    ctx.config.as_ref().and_then(|c| c.read().ok()).and_then(|config| {
        config.raw.iter().find(|r| r.norm_type() == "director")
            .and_then(|r| r.directives.iter().rev().find(|d| d.norm_key() == "queryfile"))
            .and_then(|d| d.first().map(str::to_string))
    }).unwrap_or_else(|| "/usr/lib/bareos/scripts/query.sql".to_string())
}

fn query_dialog(ua: &mut crate::ua_select::Ua, ast: &CommandAst, path: &str,
    mut run: impl FnMut(&ExecContext, &str, crate::sql_list::ListType) -> Result<String, String>) {
    use crate::sql_list::ListType;
    if !ua.open_client_db(ast, true) {
        return;
    }
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) => {
            let message = error.raw_os_error()
                .map(|code| std::io::Error::from_raw_os_error(code).to_string())
                .unwrap_or_else(|| error.to_string());
            // strerror() text without Rust's " (os error N)" suffix.
            let message = message.split(" (os error").next().unwrap_or(&message).to_string();
            ua.error_msg(format!("Could not open {path}: ERR={message}\n"));
            return;
        }
    };
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let strip = |line: &str| line.trim_end_matches(|c: char| c.is_ascii_whitespace()).to_string();
    ua.start_prompt("Available queries:\n");
    for line in &lines {
        if let Some(title) = line.strip_prefix(':') {
            ua.add_prompt(&strip(title));
        }
    }
    let Some((item, _)) = ua.do_prompt("", "Choose a query") else { return };
    // Position behind the chosen ":" line.
    let mut index = -1i64;
    let mut position = None;
    for (n, line) in lines.iter().enumerate() {
        if line.starts_with(':') {
            index += 1;
        }
        if index == item as i64 {
            position = Some(n + 1);
            break;
        }
    }
    let Some(start) = position else {
        ua.error_msg("Could not find query.\n");
        return;
    };
    let mut prompts: Vec<String> = Vec::new();
    let mut query = String::new();
    for line in &lines[start..] {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with(':') {
            break;
        }
        let line = strip(line);
        if let Some(prompt) = line.strip_prefix('*') {
            if prompts.len() >= 9 {
                ua.error_msg("Too many prompts in query, max is 9.\n");
            } else {
                prompts.push(format!("{prompt} "));
                continue;
            }
        }
        if !query.is_empty() {
            query.push(' ');
        }
        query.push_str(&line);
        let Some(statement) = query.strip_suffix(';').map(str::to_string) else { continue };
        if !statement.is_empty() {
            let statement = substitute_prompts(ua, &statement, &prompts);
            match statement.strip_prefix('!') {
                Some(vertical) => {
                    if let Ok(text) = run(ua.ctx, vertical, ListType::Vert) {
                        ua.send_msg(text);
                    }
                }
                None => match run(ua.ctx, &statement, ListType::Horz) {
                    Ok(text) => ua.send_msg(text),
                    Err(text) => {
                        ua.send_msg(text);
                        ua.send_msg(format!("{statement}\n"));
                    }
                },
            }
        }
        query.clear();
    }
    if !query.is_empty() {
        let statement = substitute_prompts(ua, &query, &prompts);
        match statement.strip_prefix('!') {
            Some(vertical) => {
                if let Ok(text) = run(ua.ctx, vertical, ListType::Vert) {
                    ua.send_msg(text);
                }
            }
            None => match run(ua.ctx, &statement, ListType::Horz) {
                Ok(text) => ua.send_msg(text),
                Err(text) => {
                    ua.send_msg(text);
                    ua.error_msg(format!("{statement}\n"));
                }
            },
        }
    }
}

/// C++ substitute_prompts() (ua_query.cc:156, 18be77e6): %1..%9 are asked
/// once per statement (GetCmd with the "*" prompt; `subst[9]` is local to each
/// call, so every statement of a multi-statement query asks again) and
/// inserted SQL-escaped; a repeated %n in the same statement reuses the
/// answer (fc2c79b6 re-escaped the latest input instead); "%%" is a percent sign.
fn substitute_prompts(ua: &mut crate::ua_select::Ua, query: &str, prompts: &[String]) -> String {
    if prompts.is_empty() {
        return query.to_string();
    }
    let mut substitutions: [Option<String>; 9] = Default::default();
    let mut out = String::new();
    let mut chars = query.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some(digit @ '1'..='9') => {
                chars.next();
                let n = digit as usize - '1' as usize;
                match prompts.get(n) {
                    Some(prompt) => {
                        if substitutions[n].is_none() {
                            match ua.get_cmd(prompt) {
                                Some(answer) => substitutions[n] = Some(answer.replace('\'', "''")),
                                None => continue,
                            }
                        }
                        out.push_str(substitutions[n].as_deref().unwrap_or(""));
                    }
                    None => ua.error_msg(format!("Warning prompt {} missing.\n", n + 1)),
                }
            }
            Some('%') => {
                chars.next();
                out.push('%');
            }
            _ => out.push('%'),
        }
    }
    out
}

/// Onesimus extensions `.sqltables`, `.sqlschema` and `.sqlcount` (no Bareos
/// counterpart; the original answers "<cmd>: is an invalid command."). They
/// read raw catalog data, so besides their own CommandACL a console needs
/// what the original requires for raw catalog reads: the CommandACL `.sql`
/// or `sqlquery`. The catalog itself is chosen like `.sql` (OpenClientDb,
/// CatalogACL, `catalog=`).
pub(crate) fn raw_catalog_allowed(ctx: &ExecContext) -> bool {
    crate::acl::access(ctx, crate::acl::Kind::Command, ".sql")
        || crate::acl::access(ctx, crate::acl::Kind::Command, "sqlquery")
}

/// Table and column names accepted by `table=` (unquoted PostgreSQL identifiers).
fn identifier(ast: &CommandAst, key: &str) -> Result<Option<String>, String> {
    let Some(value) = ast.arguments.iter()
        .find(|a| a.key.eq_ignore_ascii_case(key))
        .map(|a| a.value.clone().unwrap_or_default()) else { return Ok(None) };
    let value = value.to_ascii_lowercase();
    let mut chars = value.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid {
        return Err(format!("{}: invalid table name \"{value}\".\n", ast.command_name));
    }
    Ok(Some(value))
}

/// Run one catalog query of a `.sql*` extension and answer like `.sql`:
/// API 0 a table (ListSqlQuery HORZ_LIST), API 1 "value\t" per field and a
/// newline-free row stream, API 2 the rows as objects under `key`.
fn catalog_rows(ast: &CommandAst, ctx: &ExecContext, key: &str, sql: Result<String, String>) -> CommandResult {
    if !raw_catalog_allowed(ctx) {
        return CommandResult::error(format!(
            "{}: permission denied, reading raw catalog data requires the CommandACL .sql or sqlquery.\n",
            ast.command_name));
    }
    let sql = match sql {
        Ok(sql) => sql,
        Err(e) => return CommandResult::error(e),
    };
    crate::ua_db::with_client_db(ast, ctx, true, || {
        let Some(cat) = &ctx.catalog else {
            return CommandResult::error("Could not open catalog database.\n".to_string());
        };
        match cat.query_rows(&sql) {
            Ok(t) => match ctx.api_mode {
                ApiMode::Api => CommandResult::ok(api1_rows(&t)).verbatim(),
                _ => {
                    let mut result = Map::new();
                    result.insert(key.into(), Value::Array(json_rows(&t)));
                    CommandResult::ok(render(&t, ListType::Horz)).with_json(Value::Object(result))
                }
            },
            Err(e) => CommandResult::error(format!("Query failed: {e}\n")),
        }
    })
}

/// API 1 rows of `.sql`: "value\t" per field, rows concatenated.
fn api1_rows(t: &crate::catalog_worker::SqlTable) -> String {
    t.rows.iter()
        .filter(|row| !row.is_empty())
        .map(|row| row.iter().map(|v| format!("{}\t", v.as_deref().unwrap_or(""))).collect::<String>())
        .collect()
}

const SQLTABLES: &str = "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = 'public' ORDER BY tablename";

fn sqlschema_query(table: Option<&str>) -> String {
    let filter = table.map(|t| format!(" AND table_name = '{t}'")).unwrap_or_default();
    format!("SELECT table_name, column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'public'{filter} ORDER BY table_name, ordinal_position")
}

/// Tables counted by `.sqlcount` without `table=` (File/Path are left out:
/// counting them takes long on large catalogs).
const SQLCOUNT_TABLES: &[&str] = &["client", "fileset", "job", "jobmedia", "media", "pool", "storage"];

fn sqlcount_query(table: Option<&str>) -> String {
    let tables: Vec<&str> = match table {
        Some(t) => vec![t],
        None => SQLCOUNT_TABLES.to_vec(),
    };
    tables.iter()
        .map(|t| format!("SELECT '{t}' AS tablename, COUNT(*) AS count FROM \"{t}\""))
        .collect::<Vec<_>>()
        .join(" UNION ALL ")
}

/// `.sqltables [catalog=<name>]` (Onesimus extension)
pub fn cmd_sql_tables(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    catalog_rows(ast, ctx, "sqltables", Ok(SQLTABLES.to_string()))
}

/// `.sqlschema [table=<name>] [catalog=<name>]` (Onesimus extension)
pub fn cmd_sql_schema(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let sql = identifier(ast, "table").map(|t| sqlschema_query(t.as_deref()));
    catalog_rows(ast, ctx, "sqlschema", sql)
}

/// `.sqlcount [table=<name>] [catalog=<name>]` (Onesimus extension)
pub fn cmd_sql_count(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let sql = identifier(ast, "table").map(|t| sqlcount_query(t.as_deref()));
    catalog_rows(ast, ctx, "sqlcount", sql)
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

#[cfg(test)]
mod dialog_tests {
    use super::*;
    use crate::command_executor::ApiMode;
    use crate::ua_select::tests::{context, scripted};

    const CONFIG: &str = "Director { Name = dir }";

    #[test]
    fn sqlquery_collects_lines_until_semicolon_and_blank_line() {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut seen = Vec::new();
        let (_, console) = scripted(&["select 1", "from pool;", "bad;", ""], || {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            sql_query_mode(&mut ua, &crate::session::parse_cli_command("sqlquery ignored args"), |_, sql| {
                seen.push(sql.to_string());
                if sql == "bad;" { Err("Query failed: boom\n".into()) } else { Ok("table\n".into()) }
            });
            ua.finish(true);
        });
        assert_eq!(seen, vec!["select 1 from pool;", "bad;"]);
        assert_eq!(console.transcript(), "Entering SQL query mode.\nTerminate each query with a semicolon.\n\
            Terminate query mode with a blank line.\nEnter SQL query: select 1\nAdd to SQL query: from pool;\n\
            table\nEnter SQL query: bad;\nQuery failed: boom\nEnter SQL query: \nEnd query mode.\n");
        // Batch mode: no question, only the frame.
        let mut ua = crate::ua_select::Ua::new(&ctx);
        sql_query_mode(&mut ua, &crate::session::parse_cli_command("sqlquery"), |_, _| panic!("no query"));
        assert!(ua.finish(true).text_output.ends_with("End query mode.\n"));
    }

    #[test]
    fn query_lists_titles_and_substitutes_prompts() {
        let ctx = context(CONFIG, ApiMode::Human);
        let dir = std::env::temp_dir().join(format!("c011-query-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("query.sql");
        std::fs::write(&path, "# comment\n:First query\n*Enter Client name:\nSELECT '%1' AS a,\n '%1%%' AS b;\n\
            :Second\n!SELECT 2;\n").unwrap();
        let mut seen = Vec::new();
        let (_, console) = scripted(&["5", "1", "it's"], || {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            query_dialog(&mut ua, &crate::session::parse_cli_command("query"), path.to_str().unwrap(), |_, sql, ty| {
                seen.push((sql.to_string(), ty));
                Ok("rows\n".into())
            });
            ua.finish(true);
        });
        assert_eq!(seen, vec![("SELECT 'it''s' AS a,  'it''s%' AS b".to_string(), crate::sql_list::ListType::Horz)]);
        assert_eq!(console.transcript(), "Available queries:\n1: First query\n2: Second\nChoose a query (1-2): 5\n\
            Please enter a number between 1 and 2\nChoose a query (1-2): 1\nEnter Client name: it's\nrows\n");
        let mut seen = Vec::new();
        scripted(&["2"], || {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            query_dialog(&mut ua, &crate::session::parse_cli_command("query"), path.to_str().unwrap(), |_, sql, ty| {
                seen.push((sql.to_string(), ty));
                Ok(String::new())
            });
        });
        assert_eq!(seen, vec![("SELECT 2".to_string(), crate::sql_list::ListType::Vert)]);
        // Missing file: ErrorMsg with strerror text.
        let mut ua = crate::ua_select::Ua::new(&ctx);
        query_dialog(&mut ua, &crate::session::parse_cli_command("query"), "/nonexistent/q.sql", |_, _, _| panic!());
        assert_eq!(ua.finish(true).error_message, "Could not open /nonexistent/q.sql: ERR=No such file or directory\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ua_query.cc:164 (18be77e6): `subst[9]` is local to substitute_prompts(),
    /// so each statement asks its prompts again; within one statement a
    /// repeated %n reuses the answer of that statement.
    #[test]
    fn query_prompts_again_for_every_statement() {
        let ctx = context(CONFIG, ApiMode::Human);
        let dir = std::env::temp_dir().join(format!("query-per-statement-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("query.sql");
        std::fs::write(&path, ":Two statements\n*Name:\n*Other:\nSELECT '%1', '%2', '%1';\nSELECT '%1';\n").unwrap();
        let mut seen = Vec::new();
        let (_, console) = scripted(&["a", "b", "c"], || {
            let mut ua = crate::ua_select::Ua::new(&ctx);
            query_dialog(&mut ua, &crate::session::parse_cli_command("query"), path.to_str().unwrap(), |_, sql, _| {
                seen.push(sql.to_string());
                Ok(String::new())
            });
            ua.finish(true);
        });
        assert_eq!(seen, vec!["SELECT 'a', 'b', 'a'".to_string(), "SELECT 'c'".to_string()]);
        assert_eq!(console.transcript(), "Automatically selected : Two statements\nName: a\nOther: b\nName: c\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
