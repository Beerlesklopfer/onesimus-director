// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Versionszeile eines Daemons, wie Bareos sie in Job-Meldungen und
//! `status` zeigt.
//!
//! Zweck: `Version: <Version> (<Datum>) <Betriebssystem>` steht in jedem
//! Job-Log eines Daemons (`Jmsg(M_INFO, "Version: %s (%s) %s")`). osd nutzt
//! es seit P1; odir und ofd haben noch eigene Fassungen
//! (`odir/src/version_strings.rs`, `ofd/src/version.rs`).
//!
//! Bareos-Gegenstück: `core/src/lib/version.cc` (`kBareosVersionStrings`),
//! `lib/bsys.cc` (`GetOsInfo`, liest `/etc/os-release`).

use std::sync::OnceLock;

/// The workspace version (`<Bareos version>+onesimus.<GUI version>`).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build date, `%d %B %Y` (e.g. `20 November 2026`).
pub const BUILD_DATE: &str = env!("ONESIMUS_DAEMON_BUILD_DATE");

/// `PRETTY_NAME` of `/etc/os-release` (or `/usr/lib/os-release`), else the
/// OS name of the build target.
pub fn os_info() -> &'static str {
    static INFO: OnceLock<String> = OnceLock::new();
    INFO.get_or_init(|| {
        ["/etc/os-release", "/usr/lib/os-release"]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| pretty_name(&text))
            .unwrap_or_else(|| std::env::consts::OS.to_string())
    })
}

/// `PRETTY_NAME="…"` of an os-release text.
fn pretty_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}

/// `Version: <VERSION> (<BUILD_DATE>) <os_info>`.
pub fn version_line() -> String {
    format!("Version: {VERSION} ({BUILD_DATE}) {}", os_info())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_name_is_read_from_os_release() {
        assert_eq!(pretty_name("ID=debian\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n").as_deref(), Some("Debian GNU/Linux 13 (trixie)"));
        assert_eq!(pretty_name("ID=x\n"), None);
        assert!(version_line().starts_with(&format!("Version: {VERSION} (")));
    }
}
