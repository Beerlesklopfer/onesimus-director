// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! # onesimus-bnet-grpc – BNET-Nachrichten über gRPC
//!
//! Zweck: Zwischen den Onesimus-Daemons (odir, ofd, osd) ist gRPC mit TLS
//! der Hauptweg (Plan `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`,
//! „Grundsatz“). Über gRPC laufen dieselben BNET-Nachrichten wie über den
//! Bareos-Port: Hello, CRAM-MD5, Befehle, Datensätze. Dieses Crate ist der
//! eine Adapter dafür: Ein bidirektionaler gRPC-Strom erscheint als
//! [`std::io::Read`]/[`std::io::Write`] von BNET-Rahmen ([`ControlIo`]),
//! sodass `bareos_bnet::BnetStream` und alle Befehlsschleifen unverändert
//! darüber laufen.
//!
//! Vier Ströme nutzen ihn ([`Rpc`]):
//!
//! | Strom | Proto | Wer ruft | Inhalt |
//! |---|---|---|---|
//! | [`Rpc::FileDaemonControl`] | `onesimus.filed.FileDaemon/Control` | odir → ofd | Director-Sitzung |
//! | [`Rpc::FileDaemonData`] | `onesimus.filed.FileDaemon/Data` | osd → ofd | Datenverbindung bei passivem Client |
//! | [`Rpc::StorageDaemonControl`] | `onesimus.stored.StorageDaemon/Control` | odir → osd | Director-Sitzung |
//! | [`Rpc::StorageDaemonData`] | `onesimus.stored.StorageDaemon/Data` | ofd → osd | Datenverbindung bei aktivem Client |
//!
//! Client: [`client::connect`] baut die TLS-Verbindung (CA und Servername
//! aus der Konfiguration der Gegenstelle, [`client::Target`]) und gibt den
//! Strom als [`ControlIo`] zurück. Server: [`server::accept`] macht aus dem
//! eingehenden Strom einer RPC einen [`ControlIo`] und den Antwortstrom
//! für `tonic`. Die Anmeldung (Hello, CRAM-MD5) läuft danach im Strom wie
//! über BNET.
//!
//! Datenfluss: `BnetStream::send` → [`ControlIo`] (`Write`, ein Rahmen je
//! BNET-Nachricht) → Warteschlange → Pumpe (tokio) → gRPC → Gegenstelle →
//! Pumpe → Warteschlange → [`ControlIo`] (`Read`, Längenkopf wird
//! nachgebildet) → `BnetStream::recv`.
//!
//! Herkunft: `bareos-bnet/src/fd_grpc.rs` (`pair`, `ControlIo`, `connect`)
//! und die Strompumpe aus `ofd/src/grpc.rs`, am 20.11.2026 hierher
//! verschoben und für FileDaemon und StorageDaemon verallgemeinert. Seitdem
//! ist `bareos-bnet` frei von tonic und Protos (ofd-Gutachten §5 Punkt 1,
//! §6 Schritt 3).
//!
//! Bareos-Gegenstück: keins (Onesimus-Erweiterung); die Rahmen entsprechen
//! `core/src/lib/bsock_tcp.cc` (`send`, `recv`, `signal`).
//!
//! Dazu die TLS-Identität des gRPC-Servers eines Daemons ([`tls`]:
//! selbstsigniertes Zertifikat beim ersten Start, `ServerTlsConfig`).
//!
//! Begriffe: [`glossary`]. Klassendiagramm: an [`ControlIo`].
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
#![deny(missing_docs)]

pub mod client;
pub mod frame;
pub mod glossary;
pub mod io;
pub mod server;
pub mod tls;

pub use client::{Rpc, Target};
pub use frame::{Frame, WireFrame};
pub use io::{ControlIo, ControlPeer, pair};

/// What [`bareos_bnet::stream::ExternalStream::transport`] reports for a
/// gRPC stream; the text the job messages show as "encryption".
pub const TRANSPORT: &str = "gRPC TLS";

/// Installs rustls' `ring` crypto provider as the process default unless
/// one is installed already; [`client::connect`] and [`server::spawn`]
/// call it, so a daemon needs no setup of its own.
pub fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
