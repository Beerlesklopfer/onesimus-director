// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Changing directives in the raw text of one resource, byte for byte.
//!
//! Used by the Onesimus `configure update` command (odir) and the MCP tool
//! `configure_update` (oconsole) on top of the configuration CRUD
//! (ReadResource/UpdateResource): the resource text comes from the file,
//! only the named directives change, everything else (comments, order,
//! indentation, line ends, other directives, nested blocks) stays.
//!
//! Lexer rules as in `raw.rs` / C++ lib/lex.cc: `#` comments, `;` ends a
//! statement (several directives on one line), quoted strings with
//! backslash escapes. Directive keys compare like Bareos keywords: case,
//! blanks and underscores do not matter ([`norm_key`]). [`apply`] changes
//! the directives of the resource itself (brace depth 1) and never touches
//! nested blocks (Job `RunScript { }`, FileSet `Include { }` / `Options { }`);
//! [`apply_in`] changes the directives of one nested block named by a path
//! (`Include`, `Include[2]`, `Include/Options`) and leaves every other block
//! as it is.
//!
//! Onesimus extension without a bareos-dir counterpart.

/// Directive key as Bareos compares keywords: lowercase, without blanks and
/// underscores ("File Retention", "file_retention" and "FileRetention" are
/// the same directive).
pub fn norm_key(key: &str) -> String {
    key.chars().filter(|c| !c.is_whitespace() && *c != '_').flat_map(char::to_lowercase).collect()
}

/// One directive statement of the resource text (byte offsets into it).
#[derive(Debug, Clone)]
struct Stmt {
    /// Normalized key ([`norm_key`]).
    norm: String,
    /// Key as written (trimmed).
    key: String,
    /// Offset of the first byte of the key.
    key_start: usize,
    /// Offset of the first byte of the value (after `=` and blanks).
    value_start: usize,
    /// Offset after the last non-blank byte of the value.
    value_end: usize,
    /// `Key { … }` or `Key = { … }`: a nested block, never changed.
    block: bool,
}

/// One block of a resource text: the resource itself or a nested block.
#[derive(Debug, Default)]
struct Block {
    /// (normalized key, 1-based number among the blocks of that key in the
    /// parent) from the resource down to this block; empty for the resource.
    path: Vec<(String, usize)>,
    /// The path as written (`Include[2]/Options`), for messages.
    label: String,
    /// Offset of the block's key (0 for the resource): its line gives the
    /// indentation when the block has no directive of its own.
    key_start: usize,
    /// The block's own statements (nested blocks as statements with
    /// `block` set), in text order.
    stmts: Vec<Stmt>,
    /// Offset of the block's closing brace.
    close: usize,
}

/// The blocks of a resource text; `blocks[0]` is the resource itself.
#[derive(Debug, Default)]
struct Scan {
    /// Every block in the order of its opening brace.
    blocks: Vec<Block>,
}

/// Index of the line end (`\n`) at or after `i`, or the text length.
fn skip_line(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' {
        i += 1;
    }
    i
}

/// Parses the statements of block `at` of `scan` from `i` (after its `{`) to
/// its closing brace; sets and returns the offset of that brace.
fn parse_block(text: &str, mut i: usize, at: usize, scan: &mut Scan) -> Result<usize, String> {
    let b = text.as_bytes();
    loop {
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | b';') {
            i += 1;
        }
        if i >= b.len() {
            return Err("the resource text has unbalanced braces".into());
        }
        match b[i] {
            b'#' => {
                i = skip_line(b, i);
                continue;
            }
            b'}' => {
                scan.blocks[at].close = i;
                return Ok(i);
            }
            _ => {}
        }
        let key_start = i;
        while i < b.len() && !matches!(b[i], b'=' | b'{' | b'\n' | b'#' | b';' | b'}') {
            i += 1;
        }
        let key = text[key_start..i].trim().to_string();
        if i >= b.len() {
            return Err("the resource text has unbalanced braces".into());
        }
        let mut stmt = Stmt { norm: norm_key(&key), key: key.clone(), key_start, value_start: i, value_end: i, block: false };
        if b[i] == b'{' {
            stmt.block = true;
            let close = parse_block(text, i + 1, open_block(scan, at, &stmt), scan)?;
            scan.blocks[at].stmts.push(stmt);
            i = close + 1;
            continue;
        }
        if b[i] != b'=' {
            // a word without `=`: left to the Director's check
            continue;
        }
        i += 1;
        while i < b.len() && matches!(b[i], b' ' | b'\t') {
            i += 1;
        }
        stmt.value_start = i;
        let mut last = i;
        let mut quoted = false;
        let mut block_at = None;
        while i < b.len() {
            let c = b[i];
            if quoted {
                if c == b'\\' {
                    i += 2;
                    last = i.min(b.len());
                    continue;
                }
                if c == b'"' {
                    quoted = false;
                }
                i += 1;
                last = i;
                continue;
            }
            match c {
                b'"' => {
                    quoted = true;
                    i += 1;
                    last = i;
                }
                b'\n' | b'#' | b';' | b'}' => break,
                b'{' => {
                    block_at = Some(i);
                    break;
                }
                b' ' | b'\t' | b'\r' => i += 1,
                _ => {
                    i += 1;
                    last = i;
                }
            }
        }
        if quoted {
            return Err("the resource text has an unterminated quoted string".into());
        }
        stmt.value_end = last.min(b.len());
        if let Some(open) = block_at {
            stmt.block = true;
            let close = parse_block(text, open + 1, open_block(scan, at, &stmt), scan)?;
            scan.blocks[at].stmts.push(stmt);
            i = close + 1;
            continue;
        }
        scan.blocks[at].stmts.push(stmt);
    }
}

/// Adds the nested block that `stmt` opens inside block `parent` to `scan`
/// and returns its index.
fn open_block(scan: &mut Scan, parent: usize, stmt: &Stmt) -> usize {
    let number = 1 + scan.blocks[parent].stmts.iter().filter(|s| s.block && s.norm == stmt.norm).count();
    let mut path = scan.blocks[parent].path.clone();
    path.push((stmt.norm.clone(), number));
    let mut label = scan.blocks[parent].label.clone();
    if !label.is_empty() {
        label.push('/');
    }
    label.push_str(&stmt.key);
    if number > 1 {
        label.push_str(&format!("[{number}]"));
    }
    scan.blocks.push(Block { path, label, key_start: stmt.key_start, ..Block::default() });
    scan.blocks.len() - 1
}

/// The block named by `spec` (see [`apply_in`]): segments separated by `/`,
/// each a block key with an optional 1-based number (`Include[2]`); without
/// a number the key must name exactly one block.
fn find_block(scan: &Scan, spec: &str) -> Result<usize, String> {
    let mut at = 0;
    for segment in spec.split('/').map(str::trim).filter(|s| !s.is_empty()) {
        let (key, number) = match segment.strip_suffix(']').and_then(|s| s.split_once('[')) {
            Some((key, n)) => (key, Some(n.trim().parse::<usize>().ok().filter(|n| *n > 0)
                .ok_or_else(|| format!("block '{segment}': the number in [] must be 1 or more"))?)),
            None => (segment, None),
        };
        let norm = norm_key(key);
        let depth = scan.blocks[at].path.len();
        let children: Vec<usize> = (0..scan.blocks.len()).filter(|&b| {
            let p = &scan.blocks[b].path;
            p.len() == depth + 1 && p[..depth] == scan.blocks[at].path[..] && p[depth].0 == norm
        }).collect();
        let within = if at == 0 { "the resource".to_string() } else { format!("block '{}'", scan.blocks[at].label) };
        at = match (number, children.len()) {
            (_, 0) => return Err(format!("{within} has no block '{key}'")),
            (None, 1) => children[0],
            (None, n) => return Err(format!("{within} has {n} blocks '{key}'; name one as {key}[1] … {key}[{n}]")),
            (Some(k), n) if k <= n => children[k - 1],
            (Some(k), n) => return Err(format!("{within} has {n} block(s) '{key}', not {k}")),
        };
    }
    Ok(at)
}

/// Scans one resource text: comments before the type word, the type word,
/// `{`, the statements, the closing brace.
fn scan(text: &str) -> Result<Scan, String> {
    let b = text.as_bytes();
    let mut i = 0;
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'#' {
            i = skip_line(b, i);
        } else {
            break;
        }
    }
    while i < b.len() && b[i] != b'{' {
        if matches!(b[i], b'}' | b'=' | b'#' | b';') {
            return Err("the text is not a resource (type word and {)".into());
        }
        i += 1;
    }
    if i >= b.len() {
        return Err("the text is not a resource (type word and {)".into());
    }
    let mut scan = Scan { blocks: vec![Block::default()] };
    parse_block(text, i + 1, 0, &mut scan)?;
    Ok(scan)
}

/// One requested change: a directive and the values it gets.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    /// Key as given; written as is when the directive is added.
    pub key: String,
    /// Values as they are written into the file (already quoted where
    /// needed, see [`format_value`]). One line per value; empty: every line
    /// of the directive is removed.
    pub values: Vec<String>,
}

/// What [`apply`] changed for one directive.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    /// Key as given in the [`Edit`].
    pub directive: String,
    /// The values as written before (empty: the directive was missing).
    pub old: Vec<String>,
    /// The values as written now (empty: the directive was removed).
    pub new: Vec<String>,
}

/// Start of the line that holds `at`.
fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |p| p + 1)
}

/// Leading blanks of the line that holds `at`.
fn indentation(text: &str, at: usize) -> &str {
    let line = &text[line_start(text, at)..];
    let end = line.find(|c: char| c != ' ' && c != '\t').unwrap_or(line.len());
    &line[..end]
}

/// The statement is alone on its line: only blanks before it, and after it
/// at most blanks, one `;` and a comment.
fn alone_on_line(text: &str, s: &Stmt) -> bool {
    let start = line_start(text, s.key_start);
    if !text[start..s.key_start].chars().all(|c| c == ' ' || c == '\t') {
        return false;
    }
    let end = skip_line(text.as_bytes(), s.value_end);
    let rest = text[s.value_end..end].trim_matches(|c| c == ' ' || c == '\t' || c == '\r');
    let rest = rest.strip_prefix(';').unwrap_or(rest).trim_matches(|c| c == ' ' || c == '\t' || c == '\r');
    rest.is_empty() || rest.starts_with('#')
}

/// Offset of the line after the one holding `at` (the text length on the
/// last line).
fn next_line(text: &str, at: usize) -> usize {
    let end = skip_line(text.as_bytes(), at);
    (end + 1).min(text.len())
}

/// The byte range that removes statement `s`: its whole line when it is
/// alone on it (comment and line end included), otherwise the statement and
/// one adjacent `;` separator.
fn removal(text: &str, s: &Stmt) -> (usize, usize) {
    if alone_on_line(text, s) {
        return (line_start(text, s.key_start), next_line(text, s.value_end));
    }
    let b = text.as_bytes();
    let blank = |c: u8| c == b' ' || c == b'\t';
    // `Key = v ; next` -> remove through the `;` and the blanks after it
    let mut j = s.value_end;
    while j < b.len() && blank(b[j]) {
        j += 1;
    }
    if j < b.len() && b[j] == b';' {
        j += 1;
        while j < b.len() && blank(b[j]) {
            j += 1;
        }
        return (s.key_start, j);
    }
    // `prev ; Key = v` -> remove from the `;`
    let before = text[..s.key_start].trim_end_matches([' ', '\t']);
    if let Some(prev) = before.strip_suffix(';') {
        return (prev.trim_end_matches([' ', '\t']).len(), s.value_end);
    }
    // `Type { Key = v }`: the statement and the blanks after it
    (s.key_start, j)
}

/// Longest common subsequence of the old values and the new ones as
/// (old index, new index) pairs in order.
fn common_subsequence(old: &[&str], new: &[String]) -> Vec<(usize, usize)> {
    let (n, m) = (old.len(), new.len());
    // len[i][j]: length for old[i..] and new[j..]
    let mut len = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            len[i][j] = if old[i] == new[j] { len[i + 1][j + 1] + 1 } else { len[i + 1][j].max(len[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut pairs) = (0, 0, Vec::new());
    while i < n && j < m {
        if old[i] == new[j] {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if len[i + 1][j] >= len[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// The insertion of `values` as statements of `s`'s key right after `s`:
/// lines of their own when `s` is alone on its line, else inline with `;`.
fn after(text: &str, s: &Stmt, values: &[String], eol: &str) -> (usize, usize, String) {
    if alone_on_line(text, s) {
        let lead = indentation(text, s.key_start);
        let at = next_line(text, s.value_end);
        (at, at, values.iter().map(|v| format!("{lead}{} = {v}{eol}", s.key)).collect())
    } else {
        (s.value_end, s.value_end, values.iter().map(|v| format!("; {} = {v}", s.key)).collect())
    }
}

/// The insertion of `values` as statements of `s`'s key right before `s`.
fn before(text: &str, s: &Stmt, values: &[String], eol: &str) -> (usize, usize, String) {
    if alone_on_line(text, s) {
        let lead = indentation(text, s.key_start);
        let at = line_start(text, s.key_start);
        (at, at, values.iter().map(|v| format!("{lead}{} = {v}{eol}", s.key)).collect())
    } else {
        (s.key_start, s.key_start, values.iter().map(|v| format!("{} = {v}; ", s.key)).collect())
    }
}

/// Applies `edits` to the raw text of one resource:
/// - keys compare like Bareos ([`norm_key`]); a key given twice is refused;
/// - a directive that exists gets the values of its edit: lines whose value
///   stays (the longest common subsequence of old and new values) are not
///   touched, comments included; the other lines go, and a new value goes
///   in right after the kept line before it (before the first kept line
///   when there is none), as a line of its own or inline with `;` like that
///   line. When no value stays, the first line keeps its key, indentation,
///   comment and line end and gets the first value, the further values
///   follow it;
/// - a missing directive is added before the closing brace of the resource,
///   one line per value, indented like the other directives (or two
///   blanks) and with the file's line end; on a one-line resource inline
///   with `;`;
/// - an edit without values removes every line of the directive (a line
///   holding only that directive as a whole, otherwise only the statement);
/// - a nested block, or a directive that only occurs inside one, is
///   refused: nested blocks are never touched (see [`apply_in`]).
///
/// Values containing a line end are refused; quoting is the caller's job
/// ([`format_value`]).
pub fn apply(text: &str, edits: &[Edit]) -> Result<(String, Vec<Change>), String> {
    apply_in(text, "", edits)
}

/// [`apply`] on the directives of one nested block instead of the
/// resource's: `block` is a path of block keys separated by `/`, each with
/// an optional 1-based number when the parent has several blocks of that
/// key (`Include`, `Include[2]`, `Exclude`, `Include/Options[2]`, `RunScript`;
/// keys compare like directives). Empty: the resource itself. The rules are
/// those of [`apply`] within the block: repeated directives (`File`) are
/// replaced as a whole, a missing one is added before the block's closing
/// brace, blocks inside it stay untouched.
pub fn apply_in(text: &str, block: &str, edits: &[Edit]) -> Result<(String, Vec<Change>), String> {
    let scan = scan(text)?;
    let target = find_block(&scan, block)?;
    let this = &scan.blocks[target];
    let close = this.close;
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let indent = this.stmts.iter().rev().filter(|s| alone_on_line(text, s))
        .map(|s| indentation(text, s.key_start).to_string()).find(|i| !i.is_empty())
        .unwrap_or_else(|| format!("{}  ", if target == 0 { "" } else { indentation(text, this.key_start) }));
    // (normalized key, block label) of the statements of the blocks below the target
    let depth = this.path.len();
    let nested: Vec<(&str, &str)> = scan.blocks.iter()
        .filter(|b| b.path.len() > depth && b.path[..depth] == this.path[..])
        .flat_map(|b| b.stmts.iter().map(move |s| (s.norm.as_str(), b.label.as_str()))).collect();
    let mut seen: Vec<String> = Vec::new();
    // (start, end, replacement)
    let mut changes: Vec<(usize, usize, String)> = Vec::new();
    let mut added: Vec<(String, String)> = Vec::new();
    let mut report = Vec::new();
    for edit in edits {
        let norm = norm_key(&edit.key);
        if norm.is_empty() {
            return Err("empty directive name".into());
        }
        if seen.contains(&norm) {
            return Err(format!("directive '{}' is given twice", edit.key));
        }
        seen.push(norm.clone());
        if let Some(v) = edit.values.iter().find(|v| v.contains(['\n', '\r'])) {
            return Err(format!("directive '{}': the value {v:?} contains a line end", edit.key));
        }
        let found: Vec<&Stmt> = this.stmts.iter().filter(|s| s.norm == norm).collect();
        if found.iter().any(|s| s.block) {
            return Err(format!("'{}' is a nested block; a block is not replaced as a whole, its directives \
                are changed with the block path (block \"{}\")", edit.key, join_label(&this.label, &edit.key)));
        }
        if found.is_empty() {
            if let Some((_, label)) = nested.iter().find(|(n, _)| *n == norm) {
                return Err(format!("directive '{}' only occurs inside the nested block '{label}'; change it with \
                    the block path (block \"{label}\")", edit.key));
            }
            for v in &edit.values {
                added.push((edit.key.clone(), v.clone()));
            }
            report.push(Change { directive: edit.key.clone(), old: Vec::new(), new: edit.values.clone() });
            continue;
        }
        report.push(Change {
            directive: edit.key.clone(),
            old: found.iter().map(|s| text[s.value_start..s.value_end].to_string()).collect(),
            new: edit.values.clone(),
        });
        let (first, rest) = match edit.values.split_first() {
            Some((first, rest)) => (first, rest),
            None => {
                for s in &found {
                    let (a, z) = removal(text, s);
                    changes.push((a, z, String::new()));
                }
                continue;
            }
        };
        let olds: Vec<&str> = found.iter().map(|s| &text[s.value_start..s.value_end]).collect();
        let kept = common_subsequence(&olds, &edit.values);
        if kept.is_empty() {
            // nothing stays: the first line takes the first value (its key,
            // indentation and comment stay), the others follow it
            let head = found[0];
            changes.push((head.value_start, head.value_end, first.clone()));
            if !rest.is_empty() {
                changes.push(after(text, head, rest, eol));
            }
            for s in &found[1..] {
                let (a, z) = removal(text, s);
                changes.push((a, z, String::new()));
            }
            continue;
        }
        // lines whose value stays are not touched (comments included); the
        // others go, new values go in after the kept line before them
        for (i, s) in found.iter().enumerate() {
            if !kept.iter().any(|&(k, _)| k == i) {
                let (a, z) = removal(text, s);
                changes.push((a, z, String::new()));
            }
        }
        let (first_old, first_new) = kept[0];
        if first_new > 0 {
            changes.push(before(text, found[first_old], &edit.values[..first_new], eol));
        }
        for (n, &(i, j)) in kept.iter().enumerate() {
            let next = kept.get(n + 1).map_or(edit.values.len(), |&(_, j)| j);
            if next > j + 1 {
                changes.push(after(text, found[i], &edit.values[j + 1..next], eol));
            }
        }
    }
    if !added.is_empty() {
        let start = line_start(text, close);
        if text[start..close].trim().is_empty() {
            let lines: String = added.iter().map(|(k, v)| format!("{indent}{k} = {v}{eol}")).collect();
            changes.push((start, start, lines));
        } else {
            // closing brace on a line with content (one-line resource)
            let at = text[..close].trim_end().len();
            let sep = if text[..at].ends_with('{') || text[..at].ends_with(';') { " " } else { "; " };
            let inline: Vec<String> = added.iter().map(|(k, v)| format!("{k} = {v}")).collect();
            changes.push((at, at, format!("{sep}{}", inline.join("; "))));
        }
    }
    // from the end: a removal starting where an insertion goes comes first;
    // of two insertions at the same place the later one goes in first, so
    // the text keeps the order of the edits
    let mut changes: Vec<(usize, (usize, usize, String))> = changes.into_iter().enumerate().collect();
    changes.sort_by(|(i, a), (j, b)| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(j.cmp(i)));
    let mut out = text.to_string();
    for (_, (start, end, new)) in changes {
        out.replace_range(start..end, &new);
    }
    Ok((out, report))
}

/// `parent/key` (or `key` below the resource), for messages.
fn join_label(parent: &str, key: &str) -> String {
    if parent.is_empty() { key.to_string() } else { format!("{parent}/{key}") }
}

/// Directives of type time, size or speed (dird_conf.cc: CFG_TYPE_TIME,
/// SIZE32, SIZE64, SPEED; names and aliases normalized with [`norm_key`]).
/// bareos-dir refuses them in quotes whatever the value ("expected a time
/// period", lib/res.cc:1095; "expected a size"/"a speed", lib/res.cc:1024),
/// so they are written unquoted. A JobDefs uses the Job table. Kept equal to
/// odir's item tables by a test there (`show_items`).
pub const UNQUOTED_DIRECTIVES: &[(&str, &str)] = &[
    ("director", "fdconnecttimeout"),
    ("director", "sdconnecttimeout"),
    ("director", "heartbeatinterval"),
    ("director", "statisticsretention"),
    ("client", "hardquota"),
    ("client", "softquota"),
    ("client", "softquotagraceperiod"),
    ("client", "fileretention"),
    ("client", "jobretention"),
    ("client", "heartbeatinterval"),
    ("client", "maximumbandwidthperjob"),
    ("client", "ndmpblocksize"),
    ("storage", "heartbeatinterval"),
    ("storage", "cachestatusinterval"),
    ("storage", "maximumbandwidthperjob"),
    ("job", "maximumbandwidth"),
    ("job", "maxrunschedtime"),
    ("job", "maxruntime"),
    ("job", "fullmaxruntime"),
    ("job", "incrementalmaxruntime"),
    ("job", "differentialmaxruntime"),
    ("job", "maxwaittime"),
    ("job", "maxstartdelay"),
    ("job", "maxfullinterval"),
    ("job", "maxvirtualfullinterval"),
    ("job", "maxdiffinterval"),
    ("job", "spoolsize"),
    ("job", "rescheduleinterval"),
    ("job", "filehistorysize"),
    ("job", "alwaysincrementaljobretention"),
    ("job", "alwaysincrementalmaxfullage"),
    ("job", "runonincomingconnectinterval"),
    ("pool", "maximumvolumebytes"),
    ("pool", "volumeretention"),
    ("pool", "volumeuseduration"),
    ("pool", "migrationtime"),
    ("pool", "migrationhighbytes"),
    ("pool", "migrationlowbytes"),
    ("pool", "fileretention"),
    ("pool", "jobretention"),
    ("pool", "minimumblocksize"),
    ("pool", "maximumblocksize"),
];

/// Whether `key` of a resource of type `rtype` takes an unquoted time, size
/// or speed ([`UNQUOTED_DIRECTIVES`]).
pub fn unquoted_directive(rtype: &str, key: &str) -> bool {
    let rtype = match norm_key(rtype).as_str() {
        "jobdefs" => "job".to_string(),
        t => t.to_string(),
    };
    let key = norm_key(key);
    UNQUOTED_DIRECTIVES.iter().any(|(t, k)| *t == rtype && *k == key)
}

/// Directives written without quotes whatever they contain: the Schedule
/// `Run` directive (C++ StoreRun() scans its own tokens; a quoted value is
/// one token and would not be understood).
pub fn raw_directive(rtype: &str, key: &str) -> bool {
    norm_key(rtype) == "schedule" && norm_key(key) == "run"
}

/// A value Bareos reads as one unquoted token: not empty, no blanks, none
/// of `{ } = ; # " , \ @`.
pub fn plain_token(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control() || "{}=;#\",\\@".contains(c))
}

/// The value of `key` in a resource of type `rtype` as written into the
/// file: a plain token ([`plain_token`]) as is, anything else in quotes
/// (backslash before `"` and `\`); a [`raw_directive`] or an
/// [`unquoted_directive`] (time, size, speed) as is, trimmed, and
/// refused when it holds `"`, `;`, `#`, `{` or `}`. Control characters
/// (line ends) are refused.
pub fn format_value(rtype: &str, key: &str, value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err(format!("directive '{key}': control characters (line ends) are not allowed"));
    }
    if raw_directive(rtype, key) || unquoted_directive(rtype, key) {
        let value = value.trim();
        if value.is_empty() || value.contains(['"', ';', '#', '{', '}']) {
            return Err(format!("directive '{key}': empty value or one of \" ; # {{ }} (written unquoted)"));
        }
        return Ok(value.to_string());
    }
    if plain_token(value) {
        return Ok(value.to_string());
    }
    Ok(format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Edits from (key, values) pairs.
    fn ed(pairs: &[(&str, &[&str])]) -> Vec<Edit> {
        pairs.iter().map(|(k, v)| Edit { key: k.to_string(), values: v.iter().map(|s| s.to_string()).collect() }).collect()
    }

    /// The changed text.
    fn run(text: &str, pairs: &[(&str, &[&str])]) -> String {
        apply(text, &ed(pairs)).unwrap_or_else(|e| panic!("{e}")).0
    }

    const SCHED: &str = "# weekly\nSchedule {\n  Name = \"WeeklyCycle\"\n  Description = \"old text\"  # why\n  \
        Run = Full 1st sat at 21:00\n  Run = Differential 2nd-5th sat at 21:00\n  Enabled = yes\n}\n";

    #[test]
    fn keys_compare_like_bareos() {
        assert_eq!(norm_key("File Retention"), "fileretention");
        assert_eq!(norm_key("file_retention"), "fileretention");
        assert_eq!(norm_key("FileRetention"), "fileretention");
        assert_eq!(run(SCHED, &[("DESCRIPTION", &["new"])]), SCHED.replace("\"old text\"", "new"));
        assert_eq!(run(SCHED, &[("en_abled", &["no"])]), SCHED.replace("Enabled = yes", "Enabled = no"));
    }

    #[test]
    fn replace_keeps_comments_order_and_line_ends() {
        let crlf = SCHED.replace('\n', "\r\n");
        assert_eq!(run(&crlf, &[("description", &["\"a b\""])]), crlf.replace("\"old text\"", "\"a b\""));
        let tabs = "Client {\n\tName = a\n\tAddress = h  # the host\n}\n";
        assert_eq!(run(tabs, &[("address", &["k"])]), "Client {\n\tName = a\n\tAddress = k  # the host\n}\n");
    }

    #[test]
    fn insert_before_the_closing_brace() {
        assert_eq!(run(SCHED, &[("Maximum Run Sched Time", &["1h"])]),
            SCHED.replace("}\n", "  Maximum Run Sched Time = 1h\n}\n"));
        let tabs = "Client {\r\n\tName = a\r\n}\r\n";
        assert_eq!(run(tabs, &[("Port", &["9102"])]), "Client {\r\n\tName = a\r\n\tPort = 9102\r\n}\r\n");
        assert_eq!(run("Client { Name = a; Address = h }\n", &[("Password", &["p"])]),
            "Client { Name = a; Address = h; Password = p }\n");
        assert_eq!(run("Client {\n  Name = a\n} # end\n", &[("Password", &["p"])]), "Client {\n  Name = a\n  Password = p\n} # end\n");
    }

    #[test]
    fn remove_whole_lines_and_statements() {
        assert_eq!(run(SCHED, &[("description", &[])]), SCHED.replace("  Description = \"old text\"  # why\n", ""));
        assert_eq!(run(SCHED, &[("run", &[])]),
            SCHED.replace("  Run = Full 1st sat at 21:00\n  Run = Differential 2nd-5th sat at 21:00\n", ""));
        let joined = "Client { Name = c1; Address = old; Password = pw }\n";
        assert_eq!(run(joined, &[("address", &[])]), "Client { Name = c1; Password = pw }\n");
        assert_eq!(run(joined, &[("password", &[])]), "Client { Name = c1; Address = old }\n");
        let lines = "Client {\n  Name = c1\n  Address = old ; Port = 9102\n}\n";
        assert_eq!(run(lines, &[("port", &[])]), "Client {\n  Name = c1\n  Address = old\n}\n");
        assert_eq!(run(lines, &[("address", &[])]), "Client {\n  Name = c1\n  Port = 9102\n}\n");
        // removing a directive that is not there changes nothing
        assert_eq!(run(lines, &[("description", &[])]), lines);
    }

    #[test]
    fn semicolon_joined_statements() {
        let joined = "Client { Name = c1; Address = old; Password = pw }\n";
        assert_eq!(run(joined, &[("address", &["new"]), ("port", &["9112"])]),
            "Client { Name = c1; Address = new; Password = pw; port = 9112 }\n");
        let lines = "Client {\n  Name = c1\n  Address = old ; Port = 9102  # c\n}\n";
        assert_eq!(run(lines, &[("port", &["9103"])]), lines.replace("9102", "9103"));
    }

    #[test]
    fn repeated_directives_are_replaced_as_a_whole() {
        let out = run(SCHED, &[("run", &["Full sun at 23:30", "Incremental mon-sat at 23:30"])]);
        assert_eq!(out, SCHED.replace("Run = Full 1st sat at 21:00\n  Run = Differential 2nd-5th sat at 21:00",
            "Run = Full sun at 23:30\n  Run = Incremental mon-sat at 23:30"));
        let out = run(SCHED, &[("Run", &["Full sun at 1:00"])]);
        assert_eq!(out, SCHED.replace("Run = Full 1st sat at 21:00\n  Run = Differential 2nd-5th sat at 21:00",
            "Run = Full sun at 1:00"));
        let out = run(SCHED, &[("run", &["A", "B", "C"])]);
        assert!(out.contains("  Run = A\n  Run = B\n  Run = C\n  Enabled = yes\n"), "{out}");
        // inline: the extra values stay on the line
        let out = run("Schedule { Name = s; Run = Full sun at 1:00; Run = Full mon at 1:00 }\n", &[("run", &["x", "y"])]);
        assert_eq!(out, "Schedule { Name = s; Run = x; Run = y }\n");
        // missing: one line per value
        let out = run("Schedule {\n  Name = s\n}\n", &[("run", &["a", "b"])]);
        assert_eq!(out, "Schedule {\n  Name = s\n  run = a\n  run = b\n}\n");
        let (_, report) = apply(SCHED, &ed(&[("run", &["x"])])).unwrap();
        assert_eq!(report[0].old, ["Full 1st sat at 21:00", "Differential 2nd-5th sat at 21:00"]);
    }

    #[test]
    fn nested_blocks_are_untouched() {
        let job = "Job {\n  Name = j\n  RunScript {\n    Command = \"/bin/true\"\n    RunsWhen = Before\n  }\n  Priority = 10\n}\n";
        assert!(apply(job, &ed(&[("Command", &["x"])])).unwrap_err().contains("nested block 'RunScript'"));
        assert!(apply(job, &ed(&[("run script", &["x"])])).unwrap_err().contains("nested block"));
        assert_eq!(run(job, &[("priority", &["11"])]), job.replace("Priority = 10", "Priority = 11"));
        assert!(run(job, &[("Maximum Concurrent Jobs", &["2"])]).ends_with("  Priority = 10\n  Maximum Concurrent Jobs = 2\n}\n"));
        let fs = "FileSet {\n  Name = fs\n  Include {\n    Options {\n      Signature = MD5\n    }\n    File = /etc\n  }\n}\n";
        assert!(apply(fs, &ed(&[("file", &["/srv"])])).unwrap_err().contains("nested block 'Include'"));
        assert!(apply(fs, &ed(&[("signature", &["SHA1"])])).unwrap_err().contains("nested block 'Include/Options'; change it with the block path (block \"Include/Options\")"));
        assert_eq!(run(fs, &[("description", &["x"])]), fs.replace("  }\n}\n", "  }\n  description = x\n}\n"));
    }

    /// The changed text of block `block`.
    fn run_in(text: &str, block: &str, pairs: &[(&str, &[&str])]) -> String {
        apply_in(text, block, &ed(pairs)).unwrap_or_else(|e| panic!("{e}")).0
    }

    const FS: &str = "FileSet {\n  Name = fs\n  Include {\n    Options {\n      Signature = MD5\n    }\n\
        \x20   # config\n    File = /etc\n\n    # data\n    File = /var/lib/samba\n    File = /var/lib/samba/private  # twice\n  }\n\
        \x20 Include {\n    File = /srv\n  }\n  Exclude {\n    File = /tmp\n  }\n}\n";

    #[test]
    fn directives_of_a_named_block() {
        // the File lines of the first Include as a whole, comments and the other blocks stay
        let out = run_in(FS, "Include[1]", &[("file", &["/etc", "/var/lib/samba"])]);
        assert_eq!(out, FS.replace("    File = /var/lib/samba/private  # twice\n", ""));
        assert_eq!(run_in(FS, "include[2]", &[("File", &["/home"])]), FS.replace("/srv", "/home"));
        assert_eq!(run_in(FS, " Exclude ", &[("file", &[])]), FS.replace("    File = /tmp\n", ""));
        assert_eq!(run_in(FS, "Exclude", &[("File", &["/tmp", "/proc"])]), FS.replace("/tmp\n", "/tmp\n    File = /proc\n"));
        // a missing directive goes before the closing brace of the block, indented like its directives
        assert_eq!(run_in(FS, "Include[1]/Options", &[("signature", &["SHA1"]), ("Compression", &["GZIP"])]),
            FS.replace("Signature = MD5\n", "Signature = SHA1\n      Compression = GZIP\n"));
        // an empty block: one level deeper than its key
        let empty = "FileSet {\n  Name = fs\n  Exclude {\n  }\n}\n";
        assert_eq!(run_in(empty, "Exclude", &[("File", &["/tmp"])]), "FileSet {\n  Name = fs\n  Exclude {\n    File = /tmp\n  }\n}\n");
        assert_eq!(run_in(empty, "", &[("Description", &["x"])]), run(empty, &[("Description", &["x"])]));
        // the block's own blocks stay untouched
        let err = apply_in(FS, "Include[1]", &ed(&[("signature", &["SHA1"])])).unwrap_err();
        assert!(err.contains("(block \"Include/Options\")"), "{err}");
        let err = apply_in(FS, "Include[1]", &ed(&[("options", &["x"])])).unwrap_err();
        assert!(err.contains("(block \"Include/options\")"), "{err}");
        let job = "Job {\n  Name = j\n  RunScript {\n    Command = \"/bin/true\"\n  }\n}\n";
        assert_eq!(run_in(job, "RunScript", &[("command", &["\"/bin/false\""])]), job.replace("true", "false"));
    }

    #[test]
    fn kept_values_keep_their_lines() {
        let text = "Schedule {\n  Name = s\n  Run = b  # bee\n  Run = d  # dee\n}\n";
        // new values before, between and after the kept ones
        assert_eq!(run(text, &[("run", &["a", "b", "c", "d", "e"])]),
            "Schedule {\n  Name = s\n  Run = a\n  Run = b  # bee\n  Run = c\n  Run = d  # dee\n  Run = e\n}\n");
        assert_eq!(run(text, &[("run", &["d"])]), "Schedule {\n  Name = s\n  Run = d  # dee\n}\n");
        assert_eq!(run(text, &[("run", &["b", "d"])]), text);
        // the order changes: one stays, the other goes in after it
        assert_eq!(run(text, &[("run", &["d", "b"])]), "Schedule {\n  Name = s\n  Run = d  # dee\n  Run = b\n}\n");
        let inline = "Schedule { Name = s; Run = b; Run = d }\n";
        assert_eq!(run(inline, &[("run", &["a", "b", "c", "d"])]), "Schedule { Name = s; Run = a; Run = b; Run = c; Run = d }\n");
    }

    #[test]
    fn block_paths_must_name_one_block() {
        let err = |block: &str| apply_in(FS, block, &ed(&[("file", &["/x"])])).unwrap_err();
        assert!(err("Include").contains("the resource has 2 blocks 'Include'; name one as Include[1] … Include[2]"), "{}", err("Include"));
        assert!(err("Include[3]").contains("has 2 block(s) 'Include', not 3"));
        assert!(err("Include[0]").contains("1 or more"));
        assert!(err("Include[x]").contains("1 or more"));
        assert!(err("RunScript").contains("the resource has no block 'RunScript'"));
        assert!(err("Include[2]/Options").contains("block 'Include[2]' has no block 'Options'"), "{}", err("Include[2]/Options"));
    }

    #[test]
    fn quotes_and_comments_are_respected() {
        let text = "Job {\n  Name = j\n  Description = \"a } b # c \\\" d ; e\"  # real ; comment }\n  Client = fd\n}\n";
        let (out, report) = apply(text, &ed(&[("description", &["\"new\""]), ("client", &["fd2"])])).unwrap();
        assert_eq!(out, "Job {\n  Name = j\n  Description = \"new\"  # real ; comment }\n  Client = fd2\n}\n");
        assert_eq!(report[0].old, ["\"a } b # c \\\" d ; e\""]);
        // a commented-out directive is not a directive
        let text = "Client {\n  Name = a\n  # Address = commented\n}\n";
        assert_eq!(run(text, &[("address", &["h"])]), "Client {\n  Name = a\n  # Address = commented\n  address = h\n}\n");
    }

    #[test]
    fn refusals() {
        assert!(apply("Client {\n  Name = a\n", &ed(&[("a", &["b"])])).is_err());
        assert!(apply(SCHED, &ed(&[("description", &["a"]), ("Description", &["b"])])).is_err());
        assert!(apply(SCHED, &ed(&[("description", &["a\nb"])])).is_err());
        assert!(apply(SCHED, &ed(&[(" _ ", &["a"])])).is_err());
    }

    #[test]
    fn values_are_quoted_where_needed() {
        let f = |k: &str, v: &str| format_value("Client", k, v).unwrap();
        assert_eq!(f("address", "host.example"), "host.example");
        assert_eq!(f("port", "9102"), "9102");
        assert_eq!(f("description", "Weekly full"), "\"Weekly full\"");
        assert_eq!(f("description", "a=b"), "\"a=b\"");
        assert_eq!(f("description", "C:\\x"), "\"C:\\\\x\"");
        assert_eq!(f("description", ""), "\"\"");
        assert!(format_value("Client", "description", "a\nb").is_err());
        // Schedule Run unquoted
        assert_eq!(format_value("schedule", "Run", " Level=Full sun at 23:30 ").unwrap(), "Level=Full sun at 23:30");
        assert!(format_value("schedule", "run", "Full; x").is_err());
        assert_eq!(format_value("Job", "run", "job=x level=Full").unwrap(), "\"job=x level=Full\"");
        // time, size and speed unquoted (bareos-dir refuses them quoted, C-102)
        assert_eq!(format_value("Pool", "Volume Retention", " 3 months ").unwrap(), "3 months");
        assert_eq!(format_value("pool", "maximumvolumebytes", "50 g").unwrap(), "50 g");
        assert_eq!(format_value("JobDefs", "Max Run Time", "2 hours").unwrap(), "2 hours");
        assert_eq!(format_value("Pool", "Description", "3 months").unwrap(), "\"3 months\"");
        assert!(format_value("pool", "volumeretention", "3 months\"; x").is_err());
    }
}
