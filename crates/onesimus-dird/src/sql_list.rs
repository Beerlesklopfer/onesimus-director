//! Rendering of catalog query results like C++ `BareosDb::ListResult`
//! (cats/sql.cc): bconsole tables (HORZ), `llist` blocks (VERT), raw rows,
//! and the JSON array of row objects for `.api 2`.
//!
//! SQL NULL: the C++ PostgreSQL backend fills rows with PQgetvalue(), which
//! returns "" for NULL (cats/postgresql.cc SqlFetchRow), so ListResult never
//! sees a NULL pointer and prints an empty value in every mode. Only the
//! column width counts NULL as 4 characters (PostgreSQL SqlFieldSeek uses
//! PQgetisnull for max_length).

use serde_json::{Map, Value};

use crate::catalog_worker::SqlTable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListType {
    /// `list`: table with +---+ borders
    Horz,
    /// `llist`: one "key: value" block per row
    Vert,
    /// unformatted " value" per column, newline per row (list files)
    Nf,
    /// like Nf without newline (log text already contains it)
    Raw,
}

thread_local! {
    /// `jcr->gui` of the console whose command is running on this thread
    /// (set by `gui on`, cats/sql.cc ListResult checks it per value).
    static GUI: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// `.api 1` of the running command (set by command_executor::dispatch).
    static API1: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Mark the OutputFormatter flushes (ObjectEnd after each row) in the
    /// output: a console command of a run script sends each flush as one
    /// job message (S-24).
    static FLUSH_MARKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marks an OutputFormatter::ProcessTextBuffer() point under [`with_flush_marks`].
pub const FLUSH_MARK: char = '\u{1d}';

/// Run `f` with the row flushes of ListResult marked by [`FLUSH_MARK`].
pub fn with_flush_marks<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let old = FLUSH_MARKS.with(|c| c.replace(on));
    let r = f();
    FLUSH_MARKS.with(|c| c.set(old));
    r
}

/// ListResult's send->ObjectEnd() after a row: the text so far is sent.
fn object_end(out: &mut String) {
    if FLUSH_MARKS.with(std::cell::Cell::get) {
        out.push(FLUSH_MARK);
    }
}

/// Run `f` with `jcr->gui` set to `gui` (restored afterwards, like the
/// gui_save pattern in ua_restore.cc).
pub fn with_gui<R>(gui: bool, f: impl FnOnce() -> R) -> R {
    let saved = GUI.with(|g| g.replace(gui));
    let result = f();
    GUI.with(|g| g.set(saved));
    result
}

fn gui() -> bool {
    GUI.with(std::cell::Cell::get)
}

/// Run `f` with the `.api 1` flag set: OutputFormatter::Decoration() prints
/// nothing in API_MODE_ON (lib/output_formatter.cc:90-104), so ListResult
/// drops dashes, header, the leading "|", the blank line after `llist`
/// records and "No results to list." (cats/sql.cc:661, 534-627).
pub fn with_api1<R>(on: bool, f: impl FnOnce() -> R) -> R {
    let old = API1.with(|c| c.replace(on));
    let r = f();
    API1.with(|c| c.set(old));
    r
}

/// True while OutputFormatter::Decoration() output is printed (not `.api 1`).
pub fn decorations() -> bool {
    !API1.with(|c| c.get())
}

/// Rows as JSON objects; every value is a string, NULL becomes "" (C++ semantics).
pub fn json_rows(t: &SqlTable) -> Vec<Value> {
    t.rows.iter().map(|row| {
        let mut obj = Map::new();
        for (col, v) in t.columns.iter().zip(row) {
            obj.insert(col.name.to_lowercase(), Value::String(v.clone().unwrap_or_default()));
        }
        Value::Object(obj)
    }).collect()
}

/// Text rendering for the human/API-1 console.
pub fn render(t: &SqlTable, ty: ListType) -> String {
    render_shown(t, ty, &vec![true; t.rows.len()])
}

/// Like [`render`], but only rows with `shown[i]` are printed: the
/// OutputFormatter filters (FilterData) drop rows while ListResult runs, so
/// "No results to list." and the column widths still count every row.
pub fn render_shown(t: &SqlTable, ty: ListType, shown: &[bool]) -> String {
    if t.rows.is_empty() {
        return if decorations() { "No results to list.\n".to_string() } else { String::new() };
    }
    let rows = || t.rows.iter().zip(shown).filter(|(_, s)| **s).map(|(r, _)| r);
    match ty {
        ListType::Horz => horz(t, rows()),
        ListType::Vert => vert(t, rows()),
        ListType::Nf | ListType::Raw => {
            let mut out = String::new();
            for row in rows() {
                for v in row {
                    out.push(' ');
                    out.push_str(v.as_deref().unwrap_or(""));
                }
                if ty == ListType::Nf && decorations() {
                    out.push('\n');
                }
                object_end(&mut out);
            }
            out
        }
    }
}

/// lib/edit.cc IsAnInteger: digits only (no sign).
fn is_integer(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// lib/edit.cc add_commas: 1234567 -> 1,234,567
pub fn add_commas(s: &str) -> String {
    let (sign, digits) = match s.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", s),
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

fn column_widths(t: &SqlTable) -> Vec<usize> {
    t.columns.iter().enumerate().map(|(i, col)| {
        let mut max_len = t.rows.iter()
            .map(|r| r[i].as_deref().map_or(4, str::len)) // NULL counts as "NULL"
            .max()
            .unwrap_or(0);
        if col.numeric && max_len > 0 && col.name != "jobid" {
            max_len += (max_len - 1) / 3; // room for commas
        }
        // minimum 4 (PostgreSQL fields are never flagged NOT NULL)
        col.name.len().max(max_len).max(4).min(100)
    }).collect()
}

fn dashes(widths: &[usize]) -> String {
    let mut s = String::from("+");
    for w in widths {
        s.push_str(&"-".repeat((w + 2).min(100)));
        s.push('+');
    }
    s.push('\n');
    s
}

fn horz<'a>(t: &SqlTable, rows: impl Iterator<Item = &'a Vec<Option<String>>>) -> String {
    let widths = column_widths(t);
    let deco = decorations();
    let mut out = String::new();
    if deco {
        out.push_str(&dashes(&widths));
        out.push('|');
        for (col, w) in t.columns.iter().zip(&widths) {
            out.push_str(&format!(" {:<w$} |", col.name, w = w));
        }
        out.push('\n');
        out.push_str(&dashes(&widths));
    }
    for row in rows {
        if deco {
            out.push('|');
        }
        for ((col, v), w) in t.columns.iter().zip(row).zip(&widths) {
            let cell = match v {
                None => format!(" {:<w$} |", "", w = w),
                // cats/sql.cc:796: in gui mode numbers are left-aligned
                // without commas.
                Some(v) if col.numeric && !gui() => {
                    let v = if is_integer(v) && col.name != "jobid" { add_commas(v) } else { v.clone() };
                    format!(" {:>w$} |", v, w = w)
                }
                Some(v) => format!(" {:<w$} |", v, w = w),
            };
            out.push_str(&cell);
        }
        out.push('\n');
        object_end(&mut out);
    }
    if deco {
        out.push_str(&dashes(&widths));
    }
    out
}

fn vert<'a>(t: &SqlTable, rows: impl Iterator<Item = &'a Vec<Option<String>>>) -> String {
    let key_len = t.columns.iter().map(|c| c.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for row in rows {
        for (col, v) in t.columns.iter().zip(row) {
            let value = match v {
                None => String::new(),
                Some(v) if col.numeric && !gui() && is_integer(v) && col.name != "jobid" => add_commas(v),
                Some(v) => v.clone(),
            };
            out.push_str(&format!(" {:>w$}: {value}\n", col.name, w = key_len));
        }
        if decorations() {
            out.push('\n');
        }
        object_end(&mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_worker::SqlColumn;

    fn table() -> SqlTable {
        SqlTable {
            columns: vec![
                SqlColumn { name: "jobid".into(), numeric: true },
                SqlColumn { name: "name".into(), numeric: false },
                SqlColumn { name: "jobbytes".into(), numeric: true },
            ],
            rows: vec![
                vec![Some("479".into()), Some("benchmark-small".into()), Some("824021543".into())],
                vec![Some("12".into()), None, Some("0".into())],
            ],
        }
    }

    #[test]
    fn commas() {
        assert_eq!(add_commas("824021543"), "824,021,543");
        assert_eq!(add_commas("1000"), "1,000");
        assert_eq!(add_commas("999"), "999");
        assert_eq!(add_commas("-1234"), "-1,234");
    }

    #[test]
    fn horizontal_table() {
        let out = render(&table(), ListType::Horz);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "+-------+-----------------+-------------+");
        assert_eq!(lines[1], "| jobid | name            | jobbytes    |");
        assert_eq!(lines[3], "|   479 | benchmark-small | 824,021,543 |");
        assert_eq!(lines[4], "|    12 |                 |           0 |");
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn vertical_list() {
        let out = render(&table(), ListType::Vert);
        assert!(out.starts_with("    jobid: 479\n     name: benchmark-small\n jobbytes: 824,021,543\n\n"));
    }

    #[test]
    fn null_is_empty_but_four_wide() {
        let t = SqlTable {
            columns: vec![
                SqlColumn { name: "id".into(), numeric: true },
                SqlColumn { name: "endtime".into(), numeric: false },
            ],
            rows: vec![vec![None, None]],
        };
        assert_eq!(render(&t, ListType::Horz).lines().nth(3), Some("|       |         |"));
        assert_eq!(render(&t, ListType::Vert), "      id: \n endtime: \n\n");
        assert_eq!(render(&t, ListType::Nf), "  \n");
    }

    #[test]
    fn json_strings_and_empty_null() {
        let rows = json_rows(&table());
        assert_eq!(rows[0]["jobid"], "479");
        assert_eq!(rows[1]["name"], "");
    }

    #[test]
    fn filtered_rows_keep_widths() {
        let out = render_shown(&table(), ListType::Horz, &[false, true]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[1], "| jobid | name            | jobbytes    |");
        assert_eq!(lines[3], "|    12 |                 |           0 |");
        assert_eq!(lines.len(), 5);
        let none = render_shown(&table(), ListType::Horz, &[false, false]);
        assert_eq!(none.lines().count(), 4);
    }

    #[test]
    fn negative_numbers_get_no_commas() {
        let t = SqlTable {
            columns: vec![SqlColumn { name: "n".into(), numeric: true }],
            rows: vec![vec![Some("-1234".into())]],
        };
        assert_eq!(render(&t, ListType::Vert), " n: -1234\n\n");
    }

    #[test]
    fn gui_mode_left_aligns_numbers_without_commas() {
        let out = with_gui(true, || render(&table(), ListType::Horz));
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[1], "| jobid | name            | jobbytes    |");
        assert_eq!(lines[3], "| 479   | benchmark-small | 824021543   |");
        assert_eq!(lines[4], "| 12    |                 | 0           |");
        let vert = with_gui(true, || render(&table(), ListType::Vert));
        assert!(vert.starts_with("    jobid: 479\n     name: benchmark-small\n jobbytes: 824021543\n\n"));
        assert!(!gui());
        assert!(render(&table(), ListType::Horz).contains("824,021,543"));
    }

    #[test]
    fn empty_result() {
        assert_eq!(render(&SqlTable::default(), ListType::Horz), "No results to list.\n");
    }

    /// `.api 1`: Decoration() is silent (lib/output_formatter.cc:90-104).
    #[test]
    fn api1_drops_decorations() {
        with_api1(true, || {
            assert_eq!(render(&table(), ListType::Horz),
                "   479 | benchmark-small | 824,021,543 |\n    12 |                 |           0 |\n");
            assert!(render(&table(), ListType::Vert)
                .starts_with("    jobid: 479\n     name: benchmark-small\n jobbytes: 824,021,543\n    jobid: 12\n"));
            assert_eq!(render(&table(), ListType::Nf), " 479 benchmark-small 824021543 12  0");
            assert_eq!(render(&SqlTable::default(), ListType::Horz), "");
        });
        assert!(decorations());
    }
}
