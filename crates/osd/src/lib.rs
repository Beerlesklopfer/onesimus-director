//! # osd – Schnittstelle zum Storage Daemon
//!
//! Zweck: Alles, was eine Gegenstelle des Storage Daemons (heute der
//! Director odir, ab P1 auch der File Daemon) über das SD-Protokoll wissen
//! muss, steht hier: der Verbindungsaufbau ([`connect`], ein Handshake für
//! alle Wege), das Job-Kommando, `use storage`/`use device` und der
//! Bootstrap ([`job`]), einzelne Befehle wie `status`, `setdebug`,
//! `.status` ([`command`], [`status`]) und die Weiterleitung der
//! Katalog-Anfragen ([`catrec`]) sowie die Nachrichten, die der SD selbst
//! im Job an den Director schickt ([`askdir`]: CatReq, UpdCat, Jmsg,
//! Status; seit P1 genutzt vom Daemon `osd`). Das Crate ist eine Bibliothek, kein
//! Daemon; der Daemon selbst ist das Paket `osd-daemon`
//! (`crates/osd/daemon`, Programm `osd`).
//!
//! Abnehmer (gemessen 20.11.2026): odir nutzt [`connect::connect_sd`]
//! (`status storage`, `label`, `purge`, `setbandwidth`, Storage-Auswahl,
//! `.status`), [`connect::connect_sd_on`] (Job-Pfad in `job_worker`,
//! `backup_worker`), [`command`] (`setdebug`, `resolve`) und [`job`]
//! (Restore). [`catrec`] und [`status::query_sd_status`] haben noch keinen
//! Aufrufer; sie sind für den Client-Teil des osd-Daemons in P1 gedacht.
//! (Bis 20.11.2026 behauptete dieser Kommentar, `connect` werde von odir
//! nicht genutzt; das war falsch, ofd-Gutachten §4.)
//!
//! Datenfluss: odir → [`connect`] (TLS-PSK, Hello, CRAM-MD5, OK-Hello) →
//! [`job`] (Job-Kommando aus `bareos_format::sd_job`, `use storage`,
//! Bootstrap, `run`) → SD; Katalog-Anfragen des SD → [`catrec`] → odir.
//!
//! Die Datensatz-Ebene zwischen FD und SD (Köpfe, Streams) liegt neutral in
//! `bareos-format`; der Verbindungsaufbau „Client zum SD“ (`R_JOB`) kommt
//! in P1 hierher.
//!
//! Bareos-Gegenstück: `core/src/dird/authenticate.cc`
//! (`AuthenticateWithStorageDaemon`), `dird/msgchan.cc`
//! (`StartStorageDaemonJob`, `ReserveReadDevice`, `ReserveWriteDevice`),
//! `dird/catreq.cc`, `dird/storage.cc`.

#![deny(missing_docs)]

pub mod askdir;
pub mod connect;
pub mod command;
pub mod job;
pub mod catrec;
pub mod status;
