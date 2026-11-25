// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Rust types of the plugin broker protocols (`proto/postgresql_broker.proto`,
//! `openldap_broker.proto`, `podman_broker.proto`): the messages between a
//! WASM plugin's host and its native broker.
//!
//! A crate of its own so that the brokers link only these protos, which are
//! Onesimus's own (AGPL-3.0-or-later, dual-licensable), and not the
//! Bareos-near ones in `onesimus-proto-rs` (AGPL-3.0-only); docs/LIZENZEN.md.
//! The module paths are those of `onesimus-proto-rs`.

/// The `onesimus` protobuf package tree.
pub mod onesimus {
    /// `onesimus.filed`.
    pub mod filed {
        /// `onesimus.filed.broker`.
        pub mod broker {
            /// `onesimus.filed.broker.postgresql`.
            pub mod postgresql {
                /// `onesimus.filed.broker.postgresql.v1` (generated).
                #[allow(missing_docs, clippy::all)]
                pub mod v1 {
                    include!(concat!(env!("OUT_DIR"), "/onesimus.filed.broker.postgresql.v1.rs"));
                }
            }
            /// `onesimus.filed.broker.openldap`.
            pub mod openldap {
                /// `onesimus.filed.broker.openldap.v1` (generated).
                #[allow(missing_docs, clippy::all)]
                pub mod v1 {
                    include!(concat!(env!("OUT_DIR"), "/onesimus.filed.broker.openldap.v1.rs"));
                }
            }
            /// `onesimus.filed.broker.podman`.
            pub mod podman {
                /// `onesimus.filed.broker.podman.v1` (generated).
                #[allow(missing_docs, clippy::all)]
                pub mod v1 {
                    include!(concat!(env!("OUT_DIR"), "/onesimus.filed.broker.podman.v1.rs"));
                }
            }
        }
    }
}
