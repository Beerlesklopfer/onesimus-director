// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! First start on Windows: the configuration template with its
//! placeholders filled in.
//!
//! On Debian the package's `postinst` copies `config/bareos-fd.d` and
//! replaces `@CLIENT_NAME@` (`<hostname -s>-fd`), `@DIRECTOR_NAME@`
//! (`OFD_DIRECTOR_NAME`, default `bareos-dir`) and the password `CHANGEME`
//! (random). The Windows installer (MSI, GUI repository) only creates the
//! empty, protected directory `C:\ProgramData\Onesimus\bareos-fd.d`; it
//! has no step that could write files with a generated password without a
//! custom action. So ofd does it itself on Windows at every start, before
//! the configuration is read ([`prepare`]):
//!
//! - no `*.conf` below the directory: write the template (the same three
//!   files as the Debian package, embedded at build time);
//! - a file still holds a placeholder: replace it in place.
//!
//! An existing, filled configuration is never touched. The values are the
//! ones of `postinst`; the password is 24 random bytes in hex (no quoting
//! needed in the resource file). Linux is unchanged (`postinst` stays
//! responsible); the module is built everywhere so that its tests run.
//!
//! Bareos-Gegenstück: der NSIS-Installer des Windows-Clients
//! (`core/platforms/win32/winbareos.nsi`) schreibt Client-Name und
//! Passwort beim Installieren in die Vorlage.

#![cfg_attr(unix, allow(dead_code))]

use std::path::Path;

/// The template files (relative path, content) of `config/bareos-fd.d`.
const TEMPLATE: &[(&str, &str)] = &[
    ("client/myself.conf", include_str!("../../../config/bareos-fd.d/client/myself.conf")),
    ("director/bareos-dir.conf", include_str!("../../../config/bareos-fd.d/director/bareos-dir.conf")),
    ("messages/Standard.conf", include_str!("../../../config/bareos-fd.d/messages/Standard.conf")),
];

/// The values for the placeholders.
pub struct Values {
    /// `@CLIENT_NAME@`.
    pub client_name: String,
    /// `@DIRECTOR_NAME@`.
    pub director_name: String,
    /// Replaces the first `CHANGEME` of a file (the Director password).
    pub password: String,
}

/// `text` with the placeholders replaced; `None` when there was none.
fn fill(text: &str, values: &Values) -> Option<String> {
    if !text.contains("@CLIENT_NAME@") && !text.contains("@DIRECTOR_NAME@") && !text.contains("CHANGEME") {
        return None;
    }
    // sed -e s|@DIRECTOR_NAME@|…|g -e s|@CLIENT_NAME@|…|g -e s|CHANGEME|…| (first only)
    let text = text.replace("@DIRECTOR_NAME@", &values.director_name).replace("@CLIENT_NAME@", &values.client_name);
    Some(text.replacen("CHANGEME", &values.password, 1))
}

/// Every `*.conf` below `dir` (one level of subdirectories, as the
/// Bareos layout `<dir>/<resource type>/<name>.conf`).
fn conf_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Ok(sub) = std::fs::read_dir(&p) {
                out.extend(sub.flatten().map(|s| s.path()).filter(|s| s.extension().is_some_and(|x| x == "conf")));
            }
        } else if p.extension().is_some_and(|x| x == "conf") {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Writes the template into an empty `dir`, or fills placeholders left in
/// its files. Returns what was done, for the log.
pub fn apply(dir: &Path, values: &Values) -> std::io::Result<Vec<String>> {
    let mut done = Vec::new();
    let files = conf_files(dir);
    if files.is_empty() {
        for (rel, text) in TEMPLATE {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, fill(text, values).unwrap_or_else(|| text.to_string()))?;
            done.push(format!("created {}", path.display()));
        }
        return Ok(done);
    }
    for path in files {
        let text = std::fs::read_to_string(&path)?;
        if let Some(filled) = fill(&text, values) {
            std::fs::write(&path, filled)?;
            done.push(format!("filled placeholders in {}", path.display()));
        }
    }
    Ok(done)
}

/// Windows: [`apply`] to the configuration directory `config` (only when
/// it is a directory or missing) with the host name, `OFD_DIRECTOR_NAME`
/// and a random password; errors are logged, the start goes on (loading
/// the configuration then reports what is missing).
#[cfg(windows)]
pub fn prepare(config: &Path) {
    if config.is_file() {
        return;
    }
    let host = onesimus_bnet_grpc::tls::host_name().unwrap_or_else(|| "windows".into());
    let mut random = [0u8; 24];
    if let Err(e) = getrandom::getrandom(&mut random) {
        tracing::error!("first start: no random password: {e}");
        return;
    }
    let values = Values {
        client_name: format!("{host}-fd"),
        director_name: std::env::var("OFD_DIRECTOR_NAME").unwrap_or_else(|_| "bareos-dir".into()),
        password: random.iter().map(|b| format!("{b:02x}")).collect(),
    };
    if let Err(e) = crate::paths::ensure_private_dir(config) {
        tracing::error!("first start: cannot create {}: {e}", config.display());
        return;
    }
    match apply(config, &values) {
        Ok(done) => {
            for line in &done {
                tracing::info!("first start: {line}");
            }
            if !done.is_empty() {
                tracing::info!(
                    "first start: Director '{}', Client '{}'; the password is in {} and must be entered in the \
                     Client resource of the Director",
                    values.director_name,
                    values.client_name,
                    config.join("director").display()
                );
            }
        }
        Err(e) => tracing::error!("first start: {}: {e}", config.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Values {
        Values { client_name: "host-fd".into(), director_name: "pdc-dir".into(), password: "secret".into() }
    }

    #[test]
    fn empty_directory_gets_the_filled_template() {
        let dir = std::env::temp_dir().join(format!("ofd-first-start-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let done = apply(&dir, &values()).unwrap();
        assert_eq!(done.len(), 3);
        let client = std::fs::read_to_string(dir.join("client/myself.conf")).unwrap();
        assert!(client.contains("Name = host-fd"), "{client}");
        let director = std::fs::read_to_string(dir.join("director/bareos-dir.conf")).unwrap();
        assert!(director.contains("Name = pdc-dir") && director.contains("Password = \"secret\""), "{director}");
        let messages = std::fs::read_to_string(dir.join("messages/Standard.conf")).unwrap();
        assert!(messages.contains("Director = pdc-dir = all"), "{messages}");
        // The result loads as an FD configuration.
        let cfg = crate::config::FdConfig::load(&dir).unwrap();
        assert_eq!(cfg.name, "host-fd");
        // A second start changes nothing.
        assert!(apply(&dir, &values()).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn placeholders_left_in_files_are_filled_others_kept() {
        let dir = std::env::temp_dir().join(format!("ofd-first-start-fill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("client")).unwrap();
        std::fs::create_dir_all(dir.join("director")).unwrap();
        std::fs::write(dir.join("client/myself.conf"), "Client {\n  Name = @CLIENT_NAME@\n}\n").unwrap();
        let kept = "Director {\n  Name = x-dir\n  Password = \"given\"\n}\n";
        std::fs::write(dir.join("director/x-dir.conf"), kept).unwrap();
        let done = apply(&dir, &values()).unwrap();
        assert_eq!(done.len(), 1, "{done:?}");
        assert_eq!(std::fs::read_to_string(dir.join("client/myself.conf")).unwrap(), "Client {\n  Name = host-fd\n}\n");
        assert_eq!(std::fs::read_to_string(dir.join("director/x-dir.conf")).unwrap(), kept);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
