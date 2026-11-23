// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Session state machine (enum + match, no external crate).

/// Console session states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Connected,
    Authenticating,
    Authenticated,
    GrpcMode,
    CommandLoop,
    Quitting,
}
