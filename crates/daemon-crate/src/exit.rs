// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Exit-Codes wie Bareos.
//!
//! Zweck: systemd soll einen Daemon nach einem Laufzeitfehler neu starten,
//! nach falschen Argumenten oder einer kaputten Konfiguration aber nicht –
//! ein Neustart würde nur denselben Fehler wiederholen. Die Bareos-Units und
//! die odir-Unit setzen dafür `RestartPreventExitStatus=41 42`.
//!
//! Datenfluss: `main` → `std::process::exit(<Code>)` → systemd.
//!
//! Bareos-Gegenstück: `core/src/include/exit_codes.h` (`BEXIT_SUCCESS`,
//! `BEXIT_FAILURE`, `BEXIT_CONFIG_ERROR`); die 41 für falsche Argumente steht
//! in `docs/wiki/betrieb-odir.md` und den systemd-Units (im C++-Quelltext
//! nicht nachgeprüft).

/// Regular end (`BEXIT_SUCCESS`).
pub const EXIT_SUCCESS: i32 = 0;
/// Runtime error, systemd may restart (`BEXIT_FAILURE`).
pub const EXIT_FAILURE: i32 = 1;
/// Wrong command line arguments; no restart.
pub const EXIT_ARGUMENTS: i32 = 41;
/// Configuration error, also a missing or unreadable configuration
/// (`BEXIT_CONFIG_ERROR`); no restart.
pub const EXIT_CONFIG_ERROR: i32 = 42;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_bareos_and_odir() {
        assert_eq!((EXIT_SUCCESS, EXIT_FAILURE, EXIT_ARGUMENTS, EXIT_CONFIG_ERROR), (0, 1, 41, 42));
    }
}
