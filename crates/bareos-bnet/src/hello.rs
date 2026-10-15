//! Bareos Hello protocol — connection type identification.
//!
//! The first message on a BNET connection identifies the peer:
//! - "Hello Client <name> FdProtocolVersion=<N> calling" — File Daemon
//! - "Hello Client <name> calling" — File Daemon (legacy)
//! - "Hello <name> calling version <ver>" — Named Console
//! - "Hello <name> calling version <ver> proto=grpc" — Named Console (gRPC)
//! - "Hello *UserAgent* calling" — Default Console
//! - "Hello Director <name> calling" — Director (outbound to FD/SD)

use crate::cram_md5::bash_spaces;

/// Parsed hello message from a connecting peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hello {
    /// File Daemon connecting to Director
    FileDaemon {
        client_name: String,
        fd_protocol_version: i32,
    },
    /// Default console (no name, uses Director password)
    DefaultConsole,
    /// Named console
    NamedConsole {
        name: String,
        version: String,
        wants_grpc: bool,
    },
}

/// Parse an incoming hello message.
pub fn parse_hello(msg: &str) -> Option<Hello> {
    let msg = msg.trim();

    // File Daemon with protocol version
    // "Hello Client <name> FdProtocolVersion=<N> calling"
    if msg.starts_with("Hello Client ") {
        return parse_fd_hello(msg);
    }

    // Console: "Hello <name> calling [version <ver>] [proto=grpc]"
    if msg.starts_with("Hello ") {
        return parse_console_hello(msg);
    }

    None
}

fn parse_fd_hello(msg: &str) -> Option<Hello> {
    // Strip "Hello Client "
    let rest = msg.strip_prefix("Hello Client ")?;

    // Try: "<name> FdProtocolVersion=<N> calling"
    if let Some(fd_pos) = rest.find(" FdProtocolVersion=") {
        let name = &rest[..fd_pos];
        let after_fd = &rest[fd_pos + " FdProtocolVersion=".len()..];
        if let Some(space) = after_fd.find(' ') {
            let version_str = &after_fd[..space];
            let fd_protocol_version = version_str.parse().ok()?;
            return Some(Hello::FileDaemon {
                client_name: unbash_name(name),
                fd_protocol_version,
            });
        }
    }

    // Fallback: "<name> calling"
    if let Some(pos) = rest.find(" calling") {
        let name = &rest[..pos];
        return Some(Hello::FileDaemon {
            client_name: unbash_name(name),
            fd_protocol_version: 0,
        });
    }

    None
}

fn parse_console_hello(msg: &str) -> Option<Hello> {
    let rest = msg.strip_prefix("Hello ")?;

    // Default console: "*UserAgent* calling"
    if rest.starts_with("*UserAgent*") {
        return Some(Hello::DefaultConsole);
    }

    // Named console: "<name> calling version <ver> [proto=grpc]"
    if let Some(calling_pos) = rest.find(" calling") {
        let name = &rest[..calling_pos];

        // Check for "version <ver>"
        let after_calling = &rest[calling_pos + " calling".len()..];
        let (version, wants_grpc) = if let Some(ver_rest) = after_calling.strip_prefix(" version ") {
            let wants_grpc = ver_rest.contains("proto=grpc");
            // Version string: everything up to "proto=" or end
            let version = if let Some(proto_pos) = ver_rest.find(" proto=") {
                ver_rest[..proto_pos].trim().to_string()
            } else {
                ver_rest.trim().to_string()
            };
            (version, wants_grpc)
        } else {
            (String::new(), false)
        };

        return Some(Hello::NamedConsole {
            name: unbash_name(name),
            version,
            wants_grpc,
        });
    }

    None
}

/// Build a Director hello message for outbound connections to FD/SD.
pub fn make_director_hello(director_name: &str) -> String {
    format!("Hello Director {} calling\n", bash_spaces(director_name))
}

/// Build a Console hello message.
pub fn make_console_hello(name: &str, version: &str, want_grpc: bool) -> String {
    let default_console = name == "*UserAgent*" || name.is_empty();
    if default_console && version.is_empty() {
        return "Hello *UserAgent* calling\n".to_string();
    }
    // C++ ConsoleAuthenticateWithDirector (lib/bsock.cc:348) sends the version
    // for the default console as well
    let bashed = if default_console { "*UserAgent*".to_string() } else { bash_spaces(name) };
    if want_grpc {
        format!("Hello {bashed} calling version {version} proto=grpc\n")
    } else {
        format!("Hello {bashed} calling version {version}\n")
    }
}

/// Unbash a name: replace \x01 with space (Bareos control char convention).
/// Note: underscores in names are NOT replaced — they may be literal.
/// Only \x01 (ASCII SOH) is the encoded space character.
fn unbash_name(name: &str) -> String {
    name.replace('\x01', " ").trim().to_string()
}

/// C++ BareosVersionNumber::kUndefined (include/version_numbers.h).
pub const BAREOS_VERSION_UNDEFINED: u32 = 1;
/// C++ BareosVersionNumber::kRelease_18_2: first console with the
/// response-message protocol (1000/1001/1002 with record separator).
pub const BAREOS_VERSION_RELEASE_18_2: u32 = 1802;

/// Port of C++ GetNameAndResourceTypeAndVersionFromHello (lib/util.cc:207).
/// Returns name, resource type and version number (major * 100 + minor,
/// `BAREOS_VERSION_UNDEFINED` without version); None for an invalid Hello.
pub fn name_type_and_version_from_hello(input: &str) -> Option<(String, &'static str, u32)> {
    // (prefix, resource type, position of name, position of version); order matters
    const HELLO_LIST: [(&str, &str, usize, Option<usize>); 7] = [
        ("Hello Storage calling Start Job", "R_JOB", 5, None),
        ("Hello Start Storage Job", "R_JOB", 4, None),
        ("Hello Start Job", "R_JOB", 3, None),
        ("Hello Director", "R_DIRECTOR", 2, None),
        ("Hello Storage", "R_STORAGE", 2, None),
        ("Hello Client", "R_CLIENT", 2, None),
        ("Hello", "R_CONSOLE", 1, Some(4)), // "Hello %s calling version %s"
    ];
    let &(_, r_type, name_pos, version_pos) =
        HELLO_LIST.iter().find(|(prefix, ..)| input.starts_with(prefix))?;
    // BStringList(input, ' ') splits like std::getline: no trailing empty token
    let mut args: Vec<&str> = if input.is_empty() { Vec::new() } else { input.split(' ').collect() };
    if args.last() == Some(&"") { args.pop(); }
    let name = args.get(name_pos)?.replace('\x01', " ");
    let mut version = BAREOS_VERSION_UNDEFINED;
    if let Some(version_str) = version_pos.and_then(|pos| args.get(pos)).filter(|v| !v.is_empty()) {
        let parts: Vec<&str> = version_str.split('.').collect();
        if parts.len() < 2 { return None; }
        let major = stoul(parts[0])?;
        let minor = stoul(parts[1])?;
        version = major.wrapping_mul(100).wrapping_add(minor) as u32;
    }
    Some((name, r_type, version))
}

/// std::stoul: leading blanks, optional '+', at least one digit, rest ignored.
fn stoul(text: &str) -> Option<u64> {
    let text = text.trim_start();
    let text = text.strip_prefix('+').unwrap_or(text);
    let digits = text.find(|c: char| !c.is_ascii_digit()).unwrap_or(text.len());
    text[..digits].parse().ok()
}

/// Well-known response messages.
pub const FD_OK_HELLO: &str = "2000 OK Hello";
pub const SD_OK_HELLO: &str = "3000 OK Hello";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fd_hello_with_version() {
        let msg = "Hello Client bareos-fd FdProtocolVersion=54 calling";
        assert_eq!(
            parse_hello(msg),
            Some(Hello::FileDaemon {
                client_name: "bareos-fd".into(),
                fd_protocol_version: 54,
            })
        );
    }

    #[test]
    fn parse_fd_hello_without_version() {
        let msg = "Hello Client old-client-fd calling";
        assert_eq!(
            parse_hello(msg),
            Some(Hello::FileDaemon {
                client_name: "old-client-fd".into(),
                fd_protocol_version: 0,
            })
        );
    }

    #[test]
    fn parse_default_console() {
        let msg = "Hello *UserAgent* calling";
        assert_eq!(parse_hello(msg), Some(Hello::DefaultConsole));
    }

    #[test]
    fn parse_named_console() {
        let msg = "Hello admin-console calling version 25.0.0";
        assert_eq!(
            parse_hello(msg),
            Some(Hello::NamedConsole {
                name: "admin-console".into(),
                version: "25.0.0".into(),
                wants_grpc: false,
            })
        );
    }

    #[test]
    fn parse_console_with_grpc() {
        let msg = "Hello admin calling version 25.0.0 proto=grpc";
        assert_eq!(
            parse_hello(msg),
            Some(Hello::NamedConsole {
                name: "admin".into(),
                version: "25.0.0".into(),
                wants_grpc: true,
            })
        );
    }

    #[test]
    fn make_director_hello_bashes_spaces() {
        assert_eq!(
            make_director_hello("my director"),
            "Hello Director my_director calling\n"
        );
    }

    #[test]
    fn make_console_hello_default() {
        assert_eq!(
            make_console_hello("*UserAgent*", "", false),
            "Hello *UserAgent* calling\n"
        );
    }

    #[test]
    fn make_console_hello_grpc() {
        assert_eq!(
            make_console_hello("admin", "25.0.0", true),
            "Hello admin calling version 25.0.0 proto=grpc\n"
        );
    }

    #[test]
    fn make_console_hello_default_with_version() {
        assert_eq!(make_console_hello("*UserAgent*", "25.1.2", false),
                   "Hello *UserAgent* calling version 25.1.2\n");
    }

    #[test]
    fn hello_version_numbers() {
        let v = name_type_and_version_from_hello;
        assert_eq!(v("Hello *UserAgent* calling\n"), Some(("*UserAgent*".into(), "R_CONSOLE", BAREOS_VERSION_UNDEFINED)));
        assert_eq!(v("Hello admin calling version 25.1.2+onesimus.0.2.0\n"), Some(("admin".into(), "R_CONSOLE", 2501)));
        assert_eq!(v("Hello a\x01b calling version 18.2.0"), Some(("a b".into(), "R_CONSOLE", 1802)));
        assert_eq!(v("Hello admin calling version 17.2.7\n").unwrap().2, 1702);
        assert_eq!(v("Hello admin calling version 25\n"), None);
        assert_eq!(v("Hello admin calling version x.1\n"), None);
        assert_eq!(v("Hello admin calling version "), Some(("admin".into(), "R_CONSOLE", BAREOS_VERSION_UNDEFINED)));
        assert_eq!(v("Hello Director bareos-dir calling version 25.1"),
                   Some(("bareos-dir".into(), "R_DIRECTOR", BAREOS_VERSION_UNDEFINED)));
        assert_eq!(v("Hello"), None);
        assert_eq!(v("Bye"), None);
    }

    #[test]
    fn invalid_hello() {
        assert_eq!(parse_hello("not a hello"), None);
        assert_eq!(parse_hello(""), None);
    }
}
