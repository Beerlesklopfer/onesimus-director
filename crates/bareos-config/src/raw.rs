//! Raw (untyped) view of a Bareos configuration, as the C++ lexer sees it.
//!
//! The typed parser in `parser.rs` keeps only what the Director needs at
//! runtime. Commands that have to reproduce the configuration exactly
//! (`show`) need every directive with its tokens, nested blocks
//! (Include/Options/RunScript/Addresses) and the resource order of the C++
//! Director. This module provides that:
//!
//! - lexer semantics of lib/lex.cc: `#` comments, `;` as line end, quoted
//!   strings with backslash escapes (`\x` -> `x`), adjacent quoted strings
//!   concatenated, `=`, `,`, `{`, `}` as separate tokens;
//! - resource order of the C++ Director: the config files matched by the glob
//!   `bareos-dir.d/*/*.conf` are sorted with the process collation (strcoll)
//!   and pushed onto the lexer stack, so they are read in *reverse* order;
//!   resources inside one file keep their order.

use std::path::{Path, PathBuf};

/// One token of a directive value.
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// Unquoted word
    Word(String),
    /// Quoted string (escapes already resolved)
    Quoted(String),
    Eq,
    Comma,
    /// Nested `{ ... }` block inside a value (e.g. `Addresses = { ipv4 = {...} }`)
    Block(Vec<Directive>),
}

impl Tok {
    /// Text of a word or quoted string.
    pub fn text(&self) -> Option<&str> {
        match self {
            Tok::Word(s) | Tok::Quoted(s) => Some(s),
            _ => None,
        }
    }
}

/// A directive line `Key = tokens...` or a block `Key { ... }`.
#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    /// Key as written ("Maximum Concurrent Jobs")
    pub key: String,
    /// Tokens after `=` (empty for blocks without `=`)
    pub tokens: Vec<Tok>,
    /// Body of `Key { ... }` (Include, Exclude, Options, RunScript)
    pub block: Option<Vec<Directive>>,
}

impl Directive {
    /// Normalized key: lowercase, without spaces (C++ ignores both).
    pub fn norm_key(&self) -> String {
        normalize_key(&self.key)
    }

    /// Value tokens that are words or strings (commas/equals skipped).
    pub fn values(&self) -> Vec<&str> {
        self.tokens.iter().filter_map(Tok::text).collect()
    }

    /// First value token.
    pub fn first(&self) -> Option<&str> {
        self.tokens.iter().find_map(Tok::text)
    }
}

pub fn normalize_key(key: &str) -> String {
    key.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect()
}

/// A resource block `Type { ... }` with its source file.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfResource {
    /// Resource type as written ("Job", "jobdefs", "FileSet", ...)
    pub rtype: String,
    pub directives: Vec<Directive>,
    pub source: Option<PathBuf>,
}

impl ConfResource {
    /// Normalized resource type (lowercase, no spaces).
    pub fn norm_type(&self) -> String {
        normalize_key(&self.rtype)
    }

    /// Value of the `Name` directive.
    pub fn name(&self) -> Option<&str> {
        self.directives.iter().find(|d| d.norm_key() == "name").and_then(|d| d.first())
    }
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Lex {
    Word(String),
    Quoted(String),
    Eq,
    Comma,
    Bob,
    Eob,
    Eol,
}

fn lex(input: &str) -> Vec<Lex> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let n = chars.len();
    while i < n {
        let c = chars[i];
        match c {
            '#' => {
                while i < n && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\n' | ';' => {
                out.push(Lex::Eol);
                i += 1;
            }
            c if c.is_whitespace() => i += 1,
            '{' => {
                out.push(Lex::Bob);
                i += 1;
            }
            '}' => {
                out.push(Lex::Eob);
                i += 1;
            }
            '=' => {
                out.push(Lex::Eq);
                i += 1;
            }
            ',' => {
                out.push(Lex::Comma);
                i += 1;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                loop {
                    // read one quoted segment
                    let mut esc = false;
                    while i < n {
                        let ch = chars[i];
                        i += 1;
                        if esc {
                            s.push(ch);
                            esc = false;
                        } else if ch == '\\' {
                            esc = true;
                        } else if ch == '"' {
                            break;
                        } else {
                            s.push(ch);
                        }
                    }
                    // adjacent quoted string on the same or the next line continues it
                    let mut j = i;
                    while j < n && (chars[j] == ' ' || chars[j] == '\t') {
                        j += 1;
                    }
                    if j < n && chars[j] == '"' {
                        i = j + 1;
                        continue;
                    }
                    if j < n && chars[j] == '\n' {
                        let mut k = j + 1;
                        while k < n && (chars[k] == ' ' || chars[k] == '\t') {
                            k += 1;
                        }
                        if k < n && chars[k] == '"' {
                            i = k + 1;
                            continue;
                        }
                    }
                    break;
                }
                out.push(Lex::Quoted(s));
            }
            _ => {
                let mut s = String::new();
                while i < n {
                    let ch = chars[i];
                    if ch.is_whitespace() || "{}=,;#\"".contains(ch) {
                        break;
                    }
                    s.push(ch);
                    i += 1;
                }
                out.push(Lex::Word(s));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct P {
    toks: Vec<Lex>,
    pos: usize,
}

impl P {
    fn peek(&self) -> Option<&Lex> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Lex> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn skip_eol(&mut self) {
        while self.peek() == Some(&Lex::Eol) {
            self.pos += 1;
        }
    }

    /// Directives until the closing `}` (consumed).
    fn block(&mut self) -> Result<Vec<Directive>, String> {
        let mut dirs = Vec::new();
        loop {
            self.skip_eol();
            match self.peek() {
                None => return Err("unexpected end of file inside a block".into()),
                Some(Lex::Eob) => {
                    self.pos += 1;
                    return Ok(dirs);
                }
                _ => {}
            }
            // key: words up to '=' / '{' / end of line
            let mut key = Vec::new();
            while let Some(Lex::Word(w)) | Some(Lex::Quoted(w)) = self.peek().cloned().as_ref() {
                key.push(w.clone());
                self.pos += 1;
            }
            match self.peek() {
                Some(Lex::Eq) => {
                    self.pos += 1;
                    let tokens = self.value_tokens()?;
                    dirs.push(Directive { key: key.join(" "), tokens, block: None });
                }
                Some(Lex::Bob) => {
                    self.pos += 1;
                    let body = self.block()?;
                    dirs.push(Directive { key: key.join(" "), tokens: Vec::new(), block: Some(body) });
                }
                Some(Lex::Eol) | Some(Lex::Eob) | None => {
                    if !key.is_empty() {
                        dirs.push(Directive { key: key.join(" "), tokens: Vec::new(), block: None });
                    }
                }
                Some(other) => return Err(format!("unexpected token {other:?}")),
            }
        }
    }

    /// Value tokens up to the end of the line (or a closing brace, not consumed).
    fn value_tokens(&mut self) -> Result<Vec<Tok>, String> {
        let mut tokens = Vec::new();
        // a value may start on the next line after '=' only for blocks (Addresses = \n {)
        loop {
            match self.peek().cloned() {
                None | Some(Lex::Eob) => return Ok(tokens),
                Some(Lex::Eol) => {
                    if tokens.is_empty() {
                        // `Key =` followed by a block on the next line
                        let save = self.pos;
                        self.skip_eol();
                        if self.peek() == Some(&Lex::Bob) {
                            continue;
                        }
                        self.pos = save;
                    }
                    self.pos += 1;
                    return Ok(tokens);
                }
                Some(Lex::Word(w)) => {
                    tokens.push(Tok::Word(w));
                    self.pos += 1;
                }
                Some(Lex::Quoted(s)) => {
                    tokens.push(Tok::Quoted(s));
                    self.pos += 1;
                }
                Some(Lex::Eq) => {
                    tokens.push(Tok::Eq);
                    self.pos += 1;
                }
                Some(Lex::Comma) => {
                    tokens.push(Tok::Comma);
                    self.pos += 1;
                    // a list may continue on the next line after a comma
                    self.skip_eol();
                }
                Some(Lex::Bob) => {
                    self.pos += 1;
                    let body = self.block()?;
                    tokens.push(Tok::Block(body));
                }
            }
        }
    }
}

/// Parse configuration text into resources (file order).
pub fn parse_str(input: &str, source: Option<&Path>) -> Result<Vec<ConfResource>, String> {
    let mut p = P { toks: lex(input), pos: 0 };
    let mut res = Vec::new();
    loop {
        p.skip_eol();
        let Some(tok) = p.next() else { break };
        let rtype = match tok {
            Lex::Word(w) => w,
            other => return Err(format!("expected resource type, got {other:?}")),
        };
        if rtype.starts_with('@') {
            // @include directives are not supported here; skip the line
            while !matches!(p.peek(), None | Some(Lex::Eol)) {
                p.pos += 1;
            }
            continue;
        }
        p.skip_eol();
        match p.next() {
            Some(Lex::Bob) => {}
            other => return Err(format!("expected '{{' after resource type '{rtype}', got {other:?}")),
        }
        let directives = p.block()?;
        res.push(ConfResource { rtype, directives, source: source.map(Path::to_path_buf) });
    }
    Ok(res)
}

/// Load a configuration file or `bareos-dir.d` tree in C++ resource order.
pub fn load(path: &Path) -> Result<Vec<ConfResource>, String> {
    if path.is_file() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        return parse_str(&text, Some(path));
    }
    let dir_d = path.join("bareos-dir.d");
    let base = if dir_d.is_dir() { dir_d } else { path.to_path_buf() };

    let mut files = Vec::new();
    for sub in std::fs::read_dir(&base).map_err(|e| format!("readdir {}: {e}", base.display()))? {
        let sub = sub.map_err(|e| e.to_string())?.path();
        if !sub.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&sub).map_err(|e| format!("readdir {}: {e}", sub.display()))? {
            let f = f.map_err(|e| e.to_string())?.path();
            if f.extension().is_some_and(|e| e == "conf") && f.is_file() {
                files.push(f);
            }
        }
    }
    sort_like_glob(&mut files);

    let mut res = Vec::new();
    // lex_add() pushes every globbed file on a stack: the last one is read first
    for f in files.iter().rev() {
        let text = std::fs::read_to_string(f).map_err(|e| format!("read {}: {e}", f.display()))?;
        res.extend(parse_str(&text, Some(f)).map_err(|e| format!("{}: {e}", f.display()))?);
    }
    Ok(res)
}

/// Sort paths like glob(3): strcoll() with the process locale.
/// "C"/"POSIX" (or no locale) sorts bytewise; other locales are approximated
/// like glibc's ISO 14651 tables: alphanumerics case-insensitively first,
/// punctuation and case only as tie breakers.
pub fn sort_like_glob(paths: &mut [PathBuf]) {
    let bytewise = collation_is_bytewise();
    paths.sort_by(|a, b| {
        let (a, b) = (a.to_string_lossy(), b.to_string_lossy());
        if bytewise {
            a.as_bytes().cmp(b.as_bytes())
        } else {
            collate_key(&a).cmp(&collate_key(&b)).then_with(|| a.cmp(&b))
        }
    });
}

fn collation_is_bytewise() -> bool {
    let locale = ["LC_ALL", "LC_COLLATE", "LANG"]
        .iter()
        .find_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty()));
    match locale {
        None => true,
        Some(l) => l == "C" || l == "POSIX" || l.starts_with("C.") || l.starts_with("POSIX."),
    }
}

fn collate_key(s: &str) -> (String, String, Vec<u8>) {
    let alnum: String = s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect();
    let lower: String = s.chars().flat_map(char::to_lowercase).collect();
    // lowercase before uppercase at the third level
    let case: Vec<u8> = s.chars().filter(|c| c.is_alphabetic()).map(|c| u8::from(c.is_uppercase())).collect();
    (alnum, lower, case)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directives_values_and_blocks() {
        let r = parse_str(r#"
Job {
  Name = "BackupCatalog"   # comment
  Maximum Concurrent Jobs = 10; Priority = 11
  Write Bootstrap = "|/usr/bin/bsmtp -f \"\(Bareos\) \<%r\>\" root"
  JobAcl = a, "b c",
     d
  RunScript {
    Command = "echo hi"
    RunsWhen = After
  }
}
"#, None).unwrap();
        assert_eq!(r.len(), 1);
        let d = &r[0].directives;
        assert_eq!(r[0].name(), Some("BackupCatalog"));
        assert_eq!(d[1].norm_key(), "maximumconcurrentjobs");
        assert_eq!(d[2].first(), Some("11"));
        assert_eq!(d[3].first(), Some("|/usr/bin/bsmtp -f \"(Bareos) <%r>\" root"));
        assert_eq!(d[4].values(), vec!["a", "b c", "d"]);
        let rs = d[5].block.as_ref().unwrap();
        assert_eq!(rs[1].first(), Some("After"));
    }

    #[test]
    fn messages_and_addresses() {
        let r = parse_str("Messages {\n Name = Std\n mail = a@b, c@d = all, !skipped\n}\n\
            Director {\n Name = d\n Addresses = {\n  ipv4 = { addr = 1.2.3.4; port = 9101 }\n }\n}\n", None).unwrap();
        let mail = &r[0].directives[1];
        assert_eq!(mail.tokens, vec![Tok::Word("a@b".into()), Tok::Comma, Tok::Word("c@d".into()), Tok::Eq,
            Tok::Word("all".into()), Tok::Comma, Tok::Word("!skipped".into())]);
        let Tok::Block(addrs) = &r[1].directives[1].tokens[0] else { panic!() };
        assert_eq!(addrs[0].key, "ipv4");
        let Tok::Block(v4) = &addrs[0].tokens[0] else { panic!() };
        assert_eq!(v4[0].first(), Some("1.2.3.4"));
        assert_eq!(v4[1].first(), Some("9101"));
    }

    #[test]
    fn adjacent_quoted_strings_concatenate() {
        let r = parse_str("Job {\n Name = \"a\" \"b\"\n Description = \"x\"\n   \"y\"\n}\n", None).unwrap();
        assert_eq!(r[0].name(), Some("ab"));
        assert_eq!(r[0].directives[1].first(), Some("xy"));
    }

    #[test]
    fn locale_collation_ignores_case_and_punctuation() {
        let mut v: Vec<PathBuf> = ["job/RestoreFiles.conf", "job/backup-bareos-fd.conf", "job/BackupCatalog.conf",
            "job/benchmark.conf", "job/backup-bareos-repo.conf"].iter().map(PathBuf::from).collect();
        let keys: Vec<_> = v.iter().map(|p| collate_key(&p.to_string_lossy())).collect();
        let mut sorted = v.clone();
        sorted.sort_by(|a, b| collate_key(&a.to_string_lossy()).cmp(&collate_key(&b.to_string_lossy())));
        assert_eq!(sorted.iter().map(|p| p.file_stem().unwrap().to_str().unwrap()).collect::<Vec<_>>(),
            vec!["backup-bareos-fd", "backup-bareos-repo", "BackupCatalog", "benchmark", "RestoreFiles"]);
        assert_eq!(keys.len(), 5);
        v.sort();
        assert_eq!(v[0], PathBuf::from("job/BackupCatalog.conf"));
    }
}
