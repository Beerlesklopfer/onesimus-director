// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Konfigurationsprüfung mit `-t`.
//!
//! Zweck: `<daemon> -t -c <Pfad>` liest die Konfiguration, meldet das
//! Ergebnis und endet, ohne etwas zu starten. systemd (`ExecReload`) und
//! Paket-Skripte nutzen das, bevor sie einen Daemon neu laden.
//!
//! Datenfluss: `main` lädt die Konfiguration → [`report`] schreibt die
//! Erfolgszeile nach stdout bzw. den Fehler nach stderr → Exit-Code.
//!
//! Bareos-Gegenstück: `filed/filed.cc` und `stored/stored.cc` `main`
//! (`test_config` → `TerminateFiled(BEXIT_SUCCESS)`, Fehler über
//! `ParseConfigOrExit` mit `BEXIT_CONFIG_ERROR`).

/// Prints the outcome of a configuration test and returns the exit code:
/// `Ok(line)` prints `line` to stdout and gives 0, `Err(text)` prints
/// `text` to stderr and gives `failure_code`.
///
/// Bareos ends a configuration error with 42 ([`crate::exit::EXIT_CONFIG_ERROR`]);
/// ofd passes 1, its exit code before 19.11.2026 (open point).
pub fn report(outcome: Result<String, String>, failure_code: i32) -> i32 {
    match outcome {
        Ok(line) => {
            println!("{line}");
            crate::exit::EXIT_SUCCESS
        }
        Err(text) => {
            eprintln!("{text}");
            failure_code
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(report(Ok("configuration OK".into()), 42), 0);
        assert_eq!(report(Err("bad".into()), crate::exit::EXIT_CONFIG_ERROR), 42);
        assert_eq!(report(Err("bad".into()), 1), 1);
    }
}
