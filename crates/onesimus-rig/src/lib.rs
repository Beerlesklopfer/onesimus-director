// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! # onesimus-rig – Testgerüst Director + FD + SD
//!
//! Zweck: ein Ende-zu-Ende-Netz für den Umbau von ofd und den Neubau von osd.
//! Das Gerüst startet `odir` und `ofd` aus dem Workspace und einen Storage
//! Daemon (heute `/usr/sbin/bareos-sd` 25, später `osd`, wählbar über
//! `ONESIMUS_RIG_SD`), dazu einen Wegwerf-Katalog in einem eigenen
//! PostgreSQL-Cluster (`initdb`, TCP auf `127.0.0.2`, hoher Port). Alles läuft
//! ohne root unter einem frischen Verzeichnis in `/tmp` und auf hohen Ports.
//!
//! Datenfluss eines Laufs:
//!
//! 1. [`prereq`] prüft die Programme und meldet fehlende laut (T-9: nie still
//!    bestehen).
//! 2. [`Rig::start`] legt das Verzeichnis an, schreibt die Konfigurationen
//!    ([`config`]), baut den Testbaum ([`tree`]), startet den Katalog
//!    ([`catalog`]) und die drei Daemons ([`process`]).
//! 3. Der Test spricht über die Konsole ([`console`]) mit dem Director, wie
//!    `bconsole` es täte, und liest das Ergebnis aus dem Katalog.
//! 4. Beim Verlassen räumt [`Rig`] in `Drop` alles ab, auch nach einem
//!    Fehlschlag: Prozesse, Cluster, Verzeichnis.
//!
//! Bareos-Gegenstück: die Regressionstests in `systemtests/` von Bareos, die
//! ebenfalls Director, FD und SD lokal mit eigener Konfiguration starten. Die
//! Vorlage hier ist das Wegwerf-Gerüst `ofdrig.py` aus dem Gutachten
//! `docs/audits/ofd-struktur-2026-11-19.md` §3.
//!
//! Anleitung: `docs/wiki/testgeruest.md`. Begriffe: [`glossary`].
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod catalog;
pub mod config;
pub mod console;
pub mod glossary;
pub mod prereq;
pub mod process;
mod rig;
pub mod tree;

pub use config::Grpc;
pub use rig::{FdEndJob, JobRow, Rig, RigOptions, build_workspace_binaries, build_workspace_packages, is_osd};
