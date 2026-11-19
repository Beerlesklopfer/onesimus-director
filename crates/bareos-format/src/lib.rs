//! # bareos-format – das Bareos-Strom- und Volume-Format
//!
//! Zweck: die gemeinsame Grundlage für ofd, osd und odir. Was alle drei über
//! das Bareos-Format wissen müssen, steht hier genau einmal: die
//! LStat-Kodierung der Dateiattribute, das Bareos-eigene Base64, die
//! Stream-Kennungen (`STREAM_*`), die Dateitypen (`FT_*`), die
//! Meldungsarten (`M_*`), BashSpaces und das Job-Kommando an den Storage
//! Daemon. Das Crate hat keine Abhängigkeiten, macht kein IO und kennt weder
//! Netz noch Datenbank.
//!
//! Datenfluss: Ein Backup erzeugt im FD je Datei einen Attribut-Datensatz
//! (`STREAM_UNIX_ATTRIBUTES`, darin die LStat-Zeichenkette aus
//! [`lstat::encode_stat`]) und Datensätze mit Daten und Prüfsumme. Der SD
//! schreibt sie in Volumes ([`record`], [`volume`]), der Director legt die
//! Attribute im Katalog ab (`File.LStat`) und dekodiert sie für `bvfs`,
//! `ovfs` und Verify wieder ([`lstat::decode_stat`]).
//!
//! Bareos-Gegenstück: `core/src/include/streams.h`, `include/filetypes.h`,
//! `lib/message_severity.h`, `lib/attribs.cc` (`EncodeStat`, `DecodeStat`),
//! `lib/base64.cc`, `lib/util.cc` (`BashSpaces`), `dird/msgchan.cc`
//! (`StartStorageDaemonJob`) und für die Volume-Ebene `stored/block.cc`,
//! `stored/record.cc`, `stored/label.cc` (Zweig `bareos-25`).
//!
//! Begriffe: [`glossary`].
//!
//! Diagramme: Klassendiagramme am Modul [`sd_job`] und am Typ [`lstat::Stat`].
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod base64;
pub mod bash;
pub mod filetype;
pub mod glossary;
pub mod lstat;
pub mod msg;
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
