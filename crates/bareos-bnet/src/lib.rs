// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! BNET wire protocol, TLS, and CRAM-MD5 authentication for Bareos.
//!
//! This crate implements the custom Bareos network protocol used for
//! communication between Director, File Daemon, and Storage Daemon.
//!
//! # Protocol layers
//!
//! 1. **TCP** — raw socket connection
//! 2. **TLS** — optional encryption (wraps entire socket)
//! 3. **BNET** — length-prefixed framing: `[i32 BE length][payload]`
//! 4. **Hello** — first message identifies connection type
//! 5. **CRAM-MD5** — two-way authentication
//!
//! The same BNET messages also travel over gRPC between the Onesimus
//! daemons; that transport lives in the crate `onesimus-bnet-grpc` and
//! plugs in as [`stream::ExternalStream`], so this crate needs neither
//! tonic nor tokio nor the protos.

pub mod debug;
pub mod listen;
pub mod wire;
pub mod signal;
pub mod tls;
pub mod tls_psk;
pub mod stream;
pub mod cram_md5;
pub mod hello;

// Re-exports
pub use signal::BnetSignal;
pub use wire::{BnetMessage, BnetStream, BnetError};
pub use hello::Hello;
pub use crate::stream::AnyStream;
pub use cram_md5::{CramMd5Auth, AuthResult};
