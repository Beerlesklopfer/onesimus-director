// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Glossar von onesimus-bnet-grpc: ein Begriff je Bedeutung.
//!
//! Zweck: die Begriffe des gRPC-Transports ohne Bareos-Vorwissen. Kein Code.
//!
//! | Begriff | Bedeutung |
//! |---|---|
//! | **BNET** | Das Netzprotokoll von Bareos: Nachrichten mit 4-Byte-Längenkopf; eine negative Länge ist ein Signal. |
//! | **Rahmen** (Frame) | Eine BNET-Nachricht ohne Längenkopf, als gRPC-Nachricht ([`crate::Frame`]). |
//! | **Signal** | Steuernachricht ohne Nutzdaten, z. B. `EOD` (Ende der Daten) oder `TERMINATE`. |
//! | **Strom** (Stream) | Eine bidirektionale gRPC-RPC; sie lebt so lange wie die Verbindung, die sie ersetzt. |
//! | **Steuerstrom** (Control) | Director ↔ FD bzw. Director ↔ SD: Anmeldung, Befehle, Antworten. |
//! | **Datenstrom** (Data) | FD ↔ SD eines Jobs: Datensätze beim Backup und beim Restore. |
//! | **aktiver Client** | Der FD ruft den SD an (`StorageDaemon/Data`). |
//! | **passiver Client** | Der SD ruft den FD an (`FileDaemon/Data`, Client `Passive = yes`). |
//! | **Pumpe** | Die tokio-Aufgabe, die Rahmen zwischen gRPC und den Warteschlangen des [`crate::ControlIo`] bewegt. |
//! | **Rückfall** (Fallback) | Scheitert gRPC (kein Dienst, TLS, Anmeldung), nimmt der Aufrufer den BNET-Weg. |
//! | **festhalten** (Pinning) | Der Client vertraut genau dem Zertifikat, das in seiner Konfiguration steht (`Grpc CA Certificate File`). |
