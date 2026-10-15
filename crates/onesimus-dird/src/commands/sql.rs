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

/// C++ ua_query.cc SqlqueryCmd(): arguments other than `sharedsession`
/// are ignored; the queries are read in SQL query mode until a blank line
/// (or a cancelled input) ends it.
pub fn cmd_sql_query(ast: &CommandAst, ctx: &ExecContext) -> CommandResult {
    let mut ua = crate::ua_select::Ua::new(ctx);
    sql_query_mode(&mut ua, ast, |ctx, sql| list_sql_query(ctx, sql, crate::sql_list::ListType::Horz));
    ua.finish(true)
}

/// Stand-in for cats ListSqlQuery(verbose): the rendered table, or the
/// "Query failed" text (sent as a normal message by verbose callers).
/// TODO(C-016): use commands::sql::list_sql_query once it is merged.
fn list_sql_query(ctx: &ExecContext, sql: &str, ty: crate::sql_list::ListType) -> Result<String, String> {
    let Some(cat) = &ctx.catalog else { return Err("Query failed: no catalog\n".to_string()) };
    match cat.query_rows(sql) {
        Ok(table) => Ok(crate::sql_list::render(&table, ty)),
        Err(error) => Err(format!("Query failed: {error}\n")),
    }
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
    query_dialog(&mut ua, ast, &path, list_sql_query);
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
    let mut substitutions: [Option<String>; 9] = Default::default();
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
            let statement = substitute_prompts(ua, &statement, &prompts, &mut substitutions);
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
        let statement = substitute_prompts(ua, &query, &prompts, &mut substitutions);
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

/// C++ substitute_prompts(): %1..%9 are asked once per query (GetCmd with
/// the "*" prompt) and inserted SQL-escaped; "%%" is a percent sign.
fn substitute_prompts(ua: &mut crate::ua_select::Ua, query: &str, prompts: &[String],
    substitutions: &mut [Option<String>; 9]) -> String {
    if prompts.is_empty() {
        return query.to_string();
    }
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
}
