//! `odir -t` like `bareos-dir -t` (dird.cc main): ParseConfigOrExit,
//! CheckResources(), CheckCatalog(CHECK_CONNECTION), then exit.
//!
//! - parser errors (scan_err, M_CONFIG_ERROR): "bareos-dir: CONFIG ERROR at
//!   <src>" with the file position, exit 42;
//! - CheckResources()/CheckCatalog() failures: their messages, then
//!   M_ERROR_TERM "bareos-dir ERROR TERMINATION" / "Please correct the
//!   configuration in <base>", exit 1;
//! - success: no output, exit 0.
//!
//! Before InitDaemonMsg() every message goes to stdout; `my_name` is still
//! "bareos-dir". Compatibility log C-057, S-29.

use crate::config_check::{Anchor, ConfigError, Diag};
use crate::director_config::DirectorConfig;
use bareos_config::raw::{normalize_key, ConfResource};

/// my_name before the Director resource names the daemon.
const MY_NAME: &str = "bareos-dir";
/// BEXIT_CONFIG_ERROR
const EXIT_CONFIG_ERROR: i32 = 42;

/// Runs the configuration test: (text written to stdout, exit code).
pub fn run(path: &std::path::Path) -> (String, i32) {
    run_with(path, std::time::Duration::from_secs(5))
}

/// [`run`] with `pause` between the catalog connection attempts.
fn run_with(path: &std::path::Path, pause: std::time::Duration) -> (String, i32) {
    let base = crate::config_check::base_config_path(path);
    let config = match DirectorConfig::load(path) {
        Ok(config) => config,
        Err(error) => return (config_error("lib/parse_conf.cc", &error.to_string(), None), EXIT_CONFIG_ERROR),
    };
    if let Err(diags) = crate::config_check::check(&config.raw, &base) {
        return report(&config.raw, &diags, &base);
    }
    // CheckCatalog(CHECK_CONNECTION): Pmsg2(000, ...) of the first catalog
    // that does not open.
    if let Err(text) = crate::check_catalog::check_connections_with(&config, pause, |_| {}) {
        return (format!("{MY_NAME}: dird/check_catalog.cc:72-0 {text}{}", error_term(&base)), 1);
    }
    (String::new(), 0)
}

/// The messages of a rejected configuration and the exit code.
fn report(raw: &[ConfResource], diags: &[Diag], base: &str) -> (String, i32) {
    let mut out = String::new();
    for diag in diags {
        match diag {
            Diag::Job(msg_type, text) => out.push_str(&format!("{} {}", date(),
                crate::console_msg::job_message_text(MY_NAME, 0, *msg_type, text))),
            Diag::Daemon(text) => out.push_str(&format!("{} {MY_NAME}: {text}", date())),
            Diag::Parser(text) => {
                out.push_str(&config_error("lib/parse_conf.cc", text, None));
                return (out, EXIT_CONFIG_ERROR);
            }
            Diag::Config(error) => {
                out.push_str(&located_error(raw, error));
                return (out, EXIT_CONFIG_ERROR);
            }
        }
    }
    out.push_str(&error_term(base));
    (out, 1)
}

/// Jmsg(M_ERROR_TERM, "Please correct the configuration in %s\n").
fn error_term(base: &str) -> String {
    format!("{MY_NAME} ERROR TERMINATION\nPlease correct the configuration in {base}\n")
}

/// The message time of the temporary message handler ("%d-%b %H:%M").
fn date() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    crate::btime::strftime_tm("%d-%b %H:%M", &tm)
}

/// scan_err() output: "Config error: <msg>\n            : line L, col C of
/// file F\n<line>\n" after the M_CONFIG_ERROR header.
fn config_error(at: &str, message: &str, position: Option<&Position>) -> String {
    let mut out = format!("{MY_NAME}: CONFIG ERROR at {at}\nConfig error: {message}\n");
    if let Some(p) = position {
        out.push_str(&format!("            : line {}, col {} of file {}\n{}\n", p.line, p.col, p.file, p.text));
    }
    out.push('\n');
    out
}

fn located_error(raw: &[ConfResource], error: &ConfigError) -> String {
    let position = locate(raw, error);
    let message = match (&error.anchor, &position) {
        (Anchor::Value { .. }, Some(p)) => format!("{} on line {}: {}\n", error.message, p.line, p.text),
        _ => error.message.clone(),
    };
    config_error(error.at, &message, position.as_ref())
}

/// The lexer position of an error in its file.
#[derive(Debug, PartialEq)]
struct Position {
    file: String,
    line: usize,
    col: usize,
    text: String,
}

/// Text of `line` without a comment (`#` outside quotes).
fn code(line: &str) -> &str {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Byte offset just after `key` when `line` starts (after blanks) with the
/// directive keyword `key` (spaces and case ignored, followed by `=`/blank).
fn keyword_end(line: &str, key: &str) -> Option<usize> {
    let want = normalize_key(key);
    let start = line.len() - line.trim_start().len();
    let mut seen = String::new();
    for (i, c) in line[start..].char_indices() {
        if c == '=' || c == '{' {
            break;
        }
        if !c.is_whitespace() {
            seen.push(c.to_ascii_lowercase());
        }
        if seen == want {
            let end = start + i + c.len_utf8();
            let rest = line[end..].trim_start();
            return (rest.starts_with('=') || rest.starts_with('{')).then_some(end);
        }
        if !want.starts_with(&seen) {
            return None;
        }
    }
    None
}

/// Finds the resource of `error` in its file (the n-th resource of its type
/// there) and the line/column where the original's lexer stood.
fn locate(raw: &[ConfResource], error: &ConfigError) -> Option<Position> {
    let res = raw.get(error.resource)?;
    let source = res.source.as_ref()?;
    let content = std::fs::read_to_string(source).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let nth = raw[..error.resource].iter()
        .filter(|r| r.source == res.source && r.norm_type() == res.norm_type()).count();
    let rtype = res.norm_type();
    let starts = lines.iter().enumerate().filter(|(_, l)| keyword_end(code(l), &rtype).is_some()).map(|(i, _)| i);
    let start = starts.clone().nth(nth)?;
    // closing brace of the resource
    let mut depth = 0i32;
    let mut end = None;
    'scan: for (i, line) in lines.iter().enumerate().skip(start) {
        let mut quoted = false;
        for (j, c) in code(line).char_indices() {
            match c {
                '"' => quoted = !quoted,
                '{' if !quoted => depth += 1,
                '}' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some((i, j));
                        break 'scan;
                    }
                }
                _ => {}
            }
        }
    }
    let (end_line, end_col) = end?;
    let position = |line: usize, col: usize| Some(Position {
        file: source.display().to_string(), line: line + 1, col: col + 1, text: lines[line].to_string() });
    match &error.anchor {
        Anchor::End => position(end_line, end_col),
        Anchor::Keyword(key) => (start..=end_line)
            .find_map(|i| keyword_end(code(lines[i]), key).filter(|_| i != start).map(|col| (i, col)))
            .and_then(|(i, col)| position(i, col)),
        Anchor::Value { key, value } => (start + 1..=end_line).find_map(|i| {
            let line = code(lines[i]);
            let after = keyword_end(line, key)?;
            let at = after + line[after..].find(value.as_str())?;
            let mut col = at + value.len();
            if line[col..].starts_with('"') {
                col += 1;
            }
            Some((i, col))
        }).and_then(|(i, col)| position(i, col)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration directory with one file per resource group.
    fn config_dir(name: &str, job: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("onesimus-config-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["director", "messages", "catalog", "client", "storage", "pool", "fileset", "job"] {
            std::fs::create_dir_all(dir.join("bareos-dir.d").join(sub)).unwrap();
        }
        let write = |sub: &str, text: &str| std::fs::write(dir.join("bareos-dir.d").join(sub).join("x.conf"), text).unwrap();
        write("director", "Director {\n  Name = bareos-dir\n  Password = p\n  Messages = Daemon\n}\n");
        write("messages", "Messages {\n  Name = Daemon\n  Console = all\n}\nMessages {\n  Name = Standard\n  Console = all\n}\n");
        write("catalog", "Catalog {\n  Name = MyCatalog\n  DbName = onesimus_config_test_nosuchdb\n  DbUser = x\n  DbAddress = 127.0.0.2\n  DbPort = 1\n}\n");
        write("client", "Client {\n  Name = fd\n  Address = localhost\n  Password = p\n}\n");
        write("storage", "Storage {\n  Name = File\n  Address = localhost\n  Password = p\n  Device = FileStorage\n  Media Type = File\n}\n");
        write("pool", "Pool {\n  Name = Full\n  Pool Type = Backup\n}\n");
        write("fileset", "FileSet {\n  Name = fs\n  Include {\n    File = /etc\n  }\n}\n");
        write("job", job);
        dir
    }

    const JOB: &str = "# jobs\nJob {\n  Name = a\n  Type = Backup\n  Client = fd\n  FileSet = \"fs\"\n  Storage = File\n  Pool = Full\n  Messages = Standard\n}\n";

    fn file(dir: &std::path::Path) -> String {
        dir.join("bareos-dir.d/job/x.conf").display().to_string()
    }

    #[test]
    fn unknown_keyword_is_a_config_error_with_position() {
        let dir = config_dir("keyword", &JOB.replace("  Type = Backup\n", "  Base = a\n  Type = Backup\n"));
        assert_eq!(run_with(&dir, std::time::Duration::ZERO), (format!("bareos-dir: CONFIG ERROR at lib/parse_conf_state_machine.cc:162\n\
            Config error: Keyword \"Base\" not permitted in this resource.\n\
            Perhaps you left the trailing brace off of the previous resource.\n            : line 4, col 7 of file {}\n\
            \x20 Base = a\n\n", file(&dir)), 42));
    }

    #[test]
    fn unknown_reference_names_the_line() {
        let dir = config_dir("reference", &JOB.replace("Pool = Full", "Pool = NoSuchPool"));
        assert_eq!(run_with(&dir, std::time::Duration::ZERO), (format!("bareos-dir: CONFIG ERROR at lib/res.cc:626\n\
            Config error: Could not find config resource \"NoSuchPool\" referenced on line 8:   Pool = NoSuchPool\n\n\
            \x20           : line 8, col 20 of file {}\n  Pool = NoSuchPool\n\n", file(&dir)), 42));
    }

    #[test]
    fn second_resource_of_a_name_fails_at_its_closing_brace() {
        let dir = config_dir("duplicate", &format!("{JOB}{JOB}"));
        let (out, code) = run_with(&dir, std::time::Duration::ZERO);
        assert_eq!(code, 42);
        assert!(out.ends_with(&format!(" bareos-dir: ERROR in lib/parse_conf.cc:276 Attempt to define second Job resource \
            named \"a\" is not permitted.\nbareos-dir: CONFIG ERROR at lib/parse_conf_state_machine.cc:184\n\
            Config error: SaveResource failed\n            : line 20, col 1 of file {}\n}}\n\n", file(&dir))), "{out}");
    }

    #[test]
    fn check_resources_failure_terminates_with_exit_1() {
        let dir = config_dir("required", &JOB.replace("  FileSet = \"fs\"\n", ""));
        let (out, code) = run_with(&dir, std::time::Duration::ZERO);
        assert_eq!(code, 1);
        assert!(out.ends_with(&format!(" bareos-dir JobId 0: Error: \"fileset\" directive in Job \"a\" resource is required, \
            but not found.\nbareos-dir ERROR TERMINATION\nPlease correct the configuration in {}/bareos-dir.d/*/*.conf\n",
            dir.display())), "{out}");
    }

    #[test]
    fn unreachable_catalog_fails_the_test() {
        let dir = config_dir("catalog", JOB);
        let (out, code) = run_with(&dir, std::time::Duration::ZERO);
        assert_eq!(code, 1);
        assert!(out.starts_with("bareos-dir: dird/check_catalog.cc:72-0 Could not open Catalog \"MyCatalog\", \
            database \"onesimus_config_test_nosuchdb\": "), "{out}");
        assert!(out.ends_with("bareos-dir ERROR TERMINATION\nPlease correct the configuration in ")
            || out.contains(")\n\nbareos-dir ERROR TERMINATION\n"), "{out}");
    }
}
