//! Bareos Storage Daemon protocol library.
//!
//! Provides the Director→SD communication protocol:
//! - Connection (TLS-PSK + CRAM-MD5)
//! - Job setup (JobId, use device, bootstrap)
//! - CatReq relay loop (SD↔Director catalog requests)
//! - Status queries
//!
//! This is a protocol library, not a daemon binary.
//! The daemon will be `onesimus-stored` (gRPC-based).
//!
//! ## Forward: onesimus-stored
//!
//! `connect` und `catrec` werden aktuell nicht von odir genutzt
//! (der Director baut die SD-Verbindung im fork-Worker selbst auf und
//! hat einen eigenen CatReq-Loop mit PipeMessage-Format).
//!
//! Diese Module sind die Grundlage fuer `onesimus-stored` — den
//! Rust Storage Daemon mit gRPC-Interface. Dort werden `connect`
//! (fuer FD→SD accept) und `catrec` (fuer Director→SD CatReq)
//! als Server-seitige Implementierung genutzt.
//!
//! Der tote Code wird vom Linker (LTO) im Release-Build gestrippt.

pub mod connect;
pub mod command;
pub mod job;
pub mod catrec;
pub mod status;
