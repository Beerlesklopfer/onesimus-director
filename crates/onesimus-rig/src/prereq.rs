// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Voraussetzungen des Gerüsts: welche Programme es braucht und wo sie liegen.
//!
//! Zweck: Fehlt eine Voraussetzung, soll der Test laut scheitern und sagen,
//! was fehlt und wie man es bekommt, statt still zu bestehen (T-9).
//!
//! Datenfluss: [`Programs::find`] liest die Umgebungsvariablen
//! (`ONESIMUS_RIG_SD`, `ONESIMUS_RIG_ODIR`, `ONESIMUS_RIG_OFD`,
//! `ONESIMUS_RIG_PGBIN`), sucht die übrigen Programme im `PATH` bzw. unter
//! `/usr/lib/postgresql/<Version>/bin` und sammelt alle fehlenden in einer
//! Meldung.
//!
//! Bareos-Gegenstück: die CMake-Prüfungen der Bareos-`systemtests`, die einen
//! Test abschalten, wenn ein Programm fehlt. Hier wird nicht abgeschaltet,
//! sondern gemeldet.

use std::path::{Path, PathBuf};

/// Default path of the Bareos storage daemon (package `bareos-storage`).
pub const DEFAULT_SD: &str = "/usr/sbin/bareos-sd";

/// Bareos catalog schema for PostgreSQL (package `bareos-database-postgresql`).
pub const DEFAULT_DDL: &str = "/usr/lib/bareos/scripts/ddl/creates/postgresql.sql";

/// Every program and file a rig run needs, resolved to absolute paths.
#[derive(Debug, Clone)]
pub struct Programs {
    /// The Director binary (`target/<profile>/odir` or `ONESIMUS_RIG_ODIR`).
    pub odir: PathBuf,
    /// The File Daemon binary (`target/<profile>/ofd` or `ONESIMUS_RIG_OFD`).
    pub ofd: PathBuf,
    /// The Storage Daemon binary (`/usr/sbin/bareos-sd` or `ONESIMUS_RIG_SD`).
    pub sd: PathBuf,
    /// Whether [`Programs::sd`] is a Bareos `bareos-sd` (true) or an `osd`.
    pub sd_is_bareos: bool,
    /// PostgreSQL `initdb`.
    pub initdb: PathBuf,
    /// PostgreSQL `pg_ctl`.
    pub pg_ctl: PathBuf,
    /// PostgreSQL `psql`.
    pub psql: PathBuf,
    /// The Bareos catalog schema (`ONESIMUS_RIG_DDL` or [`DEFAULT_DDL`]).
    pub ddl: PathBuf,
    /// `diff` for the bit-exact comparison of source and restore.
    pub diff: PathBuf,
}

impl Programs {
    /// Resolves all programs; `Err` lists every missing one with a hint.
    ///
    /// `target_dir` is the cargo target directory with the profile
    /// (`<workspace>/target/debug`), where `odir` and `ofd` are expected.
    pub fn find(target_dir: &Path) -> Result<Programs, String> {
        Programs::find_with_sd(target_dir, None)
    }

    /// Like [`Programs::find`], with the Storage Daemon `sd` taking
    /// precedence over `ONESIMUS_RIG_SD` and [`DEFAULT_SD`]
    /// ([`crate::RigOptions::sd`]).
    pub fn find_with_sd(target_dir: &Path, sd: Option<PathBuf>) -> Result<Programs, String> {
        let mut missing = Vec::new();
        let mut need = |what: &str, path: Option<PathBuf>, hint: &str| -> PathBuf {
            match path {
                Some(p) if p.is_file() => p,
                other => {
                    let shown = other.map(|p| p.display().to_string()).unwrap_or_else(|| "nicht gefunden".into());
                    missing.push(format!("  - {what} ({shown}): {hint}"));
                    PathBuf::new()
                }
            }
        };
        let env_path = |name: &str| std::env::var_os(name).map(PathBuf::from);
        let odir = need(
            "odir",
            env_path("ONESIMUS_RIG_ODIR").or(Some(target_dir.join("odir"))),
            "cargo build -p odir (oder ONESIMUS_RIG_ODIR setzen)",
        );
        let ofd = need(
            "ofd",
            env_path("ONESIMUS_RIG_OFD").or(Some(target_dir.join("ofd"))),
            "cargo build -p ofd (oder ONESIMUS_RIG_OFD setzen)",
        );
        let sd_path = sd.or_else(|| env_path("ONESIMUS_RIG_SD")).unwrap_or_else(|| PathBuf::from(DEFAULT_SD));
        let sd_is_bareos = sd_path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("bareos-sd"));
        let sd = need(
            "Storage Daemon",
            Some(sd_path),
            "apt install bareos-storage (bareos-sd 25) oder ONESIMUS_RIG_SD=<Pfad zu osd>",
        );
        let pg_bin = env_path("ONESIMUS_RIG_PGBIN").or_else(postgres_bin_dir);
        let in_pg = |name: &str| pg_bin.as_ref().map(|d| d.join(name)).filter(|p| p.is_file()).or_else(|| which(name));
        let initdb = need("initdb", in_pg("initdb"), "apt install postgresql (oder ONESIMUS_RIG_PGBIN=<bin-Verzeichnis>)");
        let pg_ctl = need("pg_ctl", in_pg("pg_ctl"), "apt install postgresql (oder ONESIMUS_RIG_PGBIN=<bin-Verzeichnis>)");
        let psql = need("psql", in_pg("psql").or_else(|| which("psql")), "apt install postgresql-client");
        let ddl = need(
            "Katalogschema",
            Some(env_path("ONESIMUS_RIG_DDL").unwrap_or_else(|| PathBuf::from(DEFAULT_DDL))),
            "apt install bareos-database-postgresql (oder ONESIMUS_RIG_DDL setzen)",
        );
        let diff = need("diff", which("diff"), "apt install diffutils");
        if missing.is_empty() {
            Ok(Programs { odir, ofd, sd, sd_is_bareos, initdb, pg_ctl, psql, ddl, diff })
        } else {
            Err(format!(
                "Testgerüst: Voraussetzungen fehlen (docs/wiki/testgeruest.md):\n{}",
                missing.join("\n")
            ))
        }
    }
}

/// The newest `/usr/lib/postgresql/<version>/bin` (Debian layout), if any.
fn postgres_bin_dir() -> Option<PathBuf> {
    let mut versions: Vec<(u32, PathBuf)> = std::fs::read_dir("/usr/lib/postgresql")
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let version = e.file_name().to_string_lossy().parse::<u32>().ok()?;
            Some((version, e.path().join("bin")))
        })
        .filter(|(_, bin)| bin.join("initdb").is_file())
        .collect();
    versions.sort();
    versions.pop().map(|(_, bin)| bin)
}

/// The first executable `name` in `PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// The cargo target directory with the profile of the running test binary:
/// `CARGO_TARGET_DIR` or `<workspace>/target`, plus `debug`/`release`.
pub fn target_profile_dir() -> PathBuf {
    // The test binary lives in <target>/<profile>/deps/<name>-<hash>.
    if let Ok(exe) = std::env::current_exe()
        && let Some(profile) = exe.parent().and_then(Path::parent)
        && profile.join("deps").is_dir()
    {
        return profile.to_path_buf();
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| workspace.join("target")).join("debug")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_programs_are_listed_with_a_hint() {
        let error = Programs::find(Path::new("/nonexistent-target")).unwrap_err();
        assert!(error.contains("odir (/nonexistent-target/odir): cargo build -p odir"), "{error}");
        assert!(error.contains("docs/wiki/testgeruest.md"), "{error}");
    }

    #[test]
    fn which_finds_sh() {
        assert!(which("sh").is_some());
        assert!(which("no-such-program-onesimus").is_none());
    }
}
