// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! # bareos-format – das Bareos-Strom- und Volume-Format
//!
//! Zweck: die gemeinsame Grundlage für ofd, osd und odir. Was alle drei über
//! das Bareos-Format wissen müssen, steht hier genau einmal: die
//! LStat-Kodierung der Dateiattribute, das Bareos-eigene Base64, die
//! Stream-Kennungen (`STREAM_*`), die Dateitypen (`FT_*`), die
//! Meldungsarten (`M_*`), die Prüfsummenarten, BashSpaces, die Köpfe der
//! Datensätze zwischen FD und SD, das Umschreiben von Dateinamen beim
//! Restore (`regexwhere`) und das Job-Kommando an den Storage Daemon. Das
//! Crate macht kein IO und kennt weder Netz noch Datenbank; einzige
//! Abhängigkeit ist die libc für POSIX-Regex ([`breg`], [`posix_regex`]),
//! auf Windows statt ihrer die Crate `regex`.
//!
//! Datenfluss: Ein Backup erzeugt im FD je Datei einen Attribut-Datensatz
//! (`STREAM_UNIX_ATTRIBUTES`, darin die LStat-Zeichenkette aus
//! [`lstat::encode_stat`]) und Datensätze mit Daten und Prüfsumme. Der SD
//! schreibt sie in Volumes ([`volume`] mit Blöcken und Datensätzen, [`label`]), der Director legt die
//! Attribute im Katalog ab (`File.LStat`) und dekodiert sie für `bvfs`,
//! `ovfs` und Verify wieder ([`lstat::decode_stat`]).
//!
//! Bareos-Gegenstück: `core/src/include/streams.h`, `include/filetypes.h`,
//! `lib/message_severity.h`, `lib/attribs.cc` (`EncodeStat`, `DecodeStat`),
//! `lib/base64.cc`, `lib/util.cc` (`BashSpaces`), `lib/breg.cc`,
//! `stored/read.cc` / `filed/backup.cc` (Netz-Köpfe), `dird/msgchan.cc`
//! (`StartStorageDaemonJob`) und für die Volume-Ebene `stored/block.cc`,
//! `stored/record.cc`, `stored/label.cc`, für den Bootstrap ([`bsr`])
//! `lib/parse_bsr.cc` und `stored/bsr.cc` (Zweig `bareos-25`).
//!
//! Begriffe: [`glossary`].
//!
//! Diagramme: Klassendiagramme am Modul [`sd_job`] und am Typ [`lstat::Stat`].
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod base64;
pub mod bash;
pub mod breg;
pub mod compression;
pub mod bsr;
pub mod filetype;
pub mod glossary;
pub mod label;
pub mod lstat;
pub mod msg;
pub mod posix_regex;
pub mod record;
#[cfg_attr(doc, aquamarine::aquamarine)]
/// Job-Kommando des Directors an den Storage Daemon.
///
/// ```mermaid
/// classDiagram
///     class JobCommand {
///         +u32 job_id
///         +String unique_job
///         +String job_name
///         +String client_name
///         +char job_type
///         +char level
///         +String fileset
///         +String fileset_md5
///         +format() String
///     }
///     class bash { bash_spaces() }
///     JobCommand ..> bash : Namen mit Leerzeichen
///     odir_sd_copy ..> JobCommand
///     odir_verify ..> JobCommand
///     osd_job ..> JobCommand
/// ```
pub mod sd_job;
pub mod stream;
pub mod volume;
