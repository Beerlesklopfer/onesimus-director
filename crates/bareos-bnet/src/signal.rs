// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! BNET signal constants.
//!
//! Negative message lengths in the BNET wire protocol encode out-of-band
//! signals.  These map 1:1 to the C++ constants in `lib/bsock.h`.

/// Out-of-band signals transmitted as negative length values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BnetSignal {
    /// End of data stream, new data may follow
    Eod = -1,
    /// End of data and poll all in one
    EodPoll = -2,
    /// Send full status
    Status = -3,
    /// Conversation terminated, doing close()
    Terminate = -4,
    /// Poll request, I'm hanging on a read
    Poll = -5,
    /// Heartbeat Response requested
    Heartbeat = -6,
    /// Only response permitted to HB
    HeartbeatResponse = -7,
    /// No longer used — was Prompt for subcommand
    Prompt = -8,
    /// Send UTC btime
    Btime = -9,
    /// Stop current command (ctl-c)
    Break = -10,
    /// Start of a selection list
    StartSelect = -11,
    /// End of a select list
    EndSelect = -12,
    /// Invalid command sent
    InvalidCmd = -13,
    /// Command failed
    CmdFailed = -14,
    /// Command succeeded
    CmdOk = -15,
    /// Start command execution
    CmdBegin = -16,
    /// Messages pending
    MsgsPending = -17,
    /// Server ready and waiting
    MainPrompt = -18,
    /// Return selection input
    SelectInput = -19,
    /// Warning message
    WarningMsg = -20,
    /// Error message — command failed
    ErrorMsg = -21,
    /// Info message — status line
    InfoMsg = -22,
    /// Run command follows
    RunCmd = -23,
    /// Request yes/no response
    YesNo = -24,
    /// Start restore tree mode
    StartRtree = -25,
    /// End restore tree mode
    EndRtree = -26,
    /// Indicate we are at a subprompt
    SubPrompt = -27,
    /// Get text input from user
    TextInput = -28,
}

impl BnetSignal {
    /// Try to convert an i32 to a BnetSignal.
    pub fn from_i32(val: i32) -> Option<Self> {
        match val {
            -1 => Some(Self::Eod),
            -2 => Some(Self::EodPoll),
            -3 => Some(Self::Status),
            -4 => Some(Self::Terminate),
            -5 => Some(Self::Poll),
            -6 => Some(Self::Heartbeat),
            -7 => Some(Self::HeartbeatResponse),
            -8 => Some(Self::Prompt),
            -9 => Some(Self::Btime),
            -10 => Some(Self::Break),
            -11 => Some(Self::StartSelect),
            -12 => Some(Self::EndSelect),
            -13 => Some(Self::InvalidCmd),
            -14 => Some(Self::CmdFailed),
            -15 => Some(Self::CmdOk),
            -16 => Some(Self::CmdBegin),
            -17 => Some(Self::MsgsPending),
            -18 => Some(Self::MainPrompt),
            -19 => Some(Self::SelectInput),
            -20 => Some(Self::WarningMsg),
            -21 => Some(Self::ErrorMsg),
            -22 => Some(Self::InfoMsg),
            -23 => Some(Self::RunCmd),
            -24 => Some(Self::YesNo),
            -25 => Some(Self::StartRtree),
            -26 => Some(Self::EndRtree),
            -27 => Some(Self::SubPrompt),
            -28 => Some(Self::TextInput),
            _ => None,
        }
    }
}

/// ASCII control characters used in BNET response formatting.
pub const RECORD_SEPARATOR: u8 = 0x1e;
pub const UNIT_SEPARATOR: u8 = 0x1f;
pub const GROUP_SEPARATOR: u8 = 0x1d;

/// BNET protocol constants.
pub const MAX_PACKET_SIZE: usize = 1_000_000;
pub const HEADER_LENGTH: usize = 4;
pub const MAX_MESSAGE_LEN: usize = MAX_PACKET_SIZE - HEADER_LENGTH;

/// Well-known BNET message IDs.
pub mod message_id {
    pub const OK: u32 = 1000;
    pub const PAM_REQUIRED: u32 = 1001;
    pub const INFO_MESSAGE: u32 = 1002;
    pub const PAM_INTERACTIVE: u32 = 4001;
    pub const PAM_USER_CREDENTIALS: u32 = 4002;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_signals_roundtrip() {
        for val in -28..=-1 {
            let sig = BnetSignal::from_i32(val);
            assert!(sig.is_some(), "signal {val} should be valid");
            assert_eq!(sig.unwrap() as i32, val);
        }
    }

    #[test]
    fn invalid_signals() {
        assert_eq!(BnetSignal::from_i32(0), None);
        assert_eq!(BnetSignal::from_i32(1), None);
        assert_eq!(BnetSignal::from_i32(-29), None);
        assert_eq!(BnetSignal::from_i32(-99), None);
    }

    #[test]
    fn signal_values_match_c() {
        assert_eq!(BnetSignal::Eod as i32, -1);
        assert_eq!(BnetSignal::Terminate as i32, -4);
        assert_eq!(BnetSignal::Heartbeat as i32, -6);
        assert_eq!(BnetSignal::MainPrompt as i32, -18);
        assert_eq!(BnetSignal::TextInput as i32, -28);
    }

    #[test]
    fn protocol_constants() {
        assert_eq!(MAX_PACKET_SIZE, 1_000_000);
        assert_eq!(HEADER_LENGTH, 4);
        assert_eq!(MAX_MESSAGE_LEN, 999_996);
    }
}
