// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Ein BNET-Rahmen und seine Proto-Formen.
//!
//! Zweck: [`Frame`] ist eine BNET-Nachricht ohne Längenkopf: Nutzdaten oder
//! ein Signal. FileDaemon (`onesimus.filed.ControlFrame`) und StorageDaemon
//! (`onesimus.stored.StoredFrame`) haben je eine eigene Proto-Nachricht mit
//! denselben Feldern; [`WireFrame`] wandelt beide in [`Frame`] und zurück,
//! damit der Adapter ([`crate::io`]) nur einen Typ kennt.
//!
//! Bareos-Gegenstück: der Längenkopf von `core/src/lib/bsock_tcp.cc`
//! (positive Länge = Nutzdaten, negative = Signal).

use onesimus_proto_rs::onesimus::filed::{ControlFrame, control_frame};
use onesimus_proto_rs::onesimus::stored::{StoredFrame, stored_frame};

/// One BNET message without its length header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// Data: a command line, a reply or a binary record.
    Payload(Vec<u8>),
    /// A BNET signal (always negative, e.g. `EOD` = -1).
    Signal(i32),
}

/// A protobuf message that carries one [`Frame`].
pub trait WireFrame: prost::Message + Default + Send + Sync + 'static {
    /// The message for `frame`.
    fn from_frame(frame: Frame) -> Self;
    /// The frame of this message; `None` when the `oneof` is empty.
    fn into_frame(self) -> Option<Frame>;
}

impl WireFrame for ControlFrame {
    fn from_frame(frame: Frame) -> Self {
        ControlFrame {
            frame: Some(match frame {
                Frame::Payload(p) => control_frame::Frame::Payload(p),
                Frame::Signal(s) => control_frame::Frame::Signal(s),
            }),
        }
    }

    fn into_frame(self) -> Option<Frame> {
        match self.frame? {
            control_frame::Frame::Payload(p) => Some(Frame::Payload(p)),
            control_frame::Frame::Signal(s) => Some(Frame::Signal(s)),
        }
    }
}

impl WireFrame for StoredFrame {
    fn from_frame(frame: Frame) -> Self {
        StoredFrame {
            frame: Some(match frame {
                Frame::Payload(p) => stored_frame::Frame::Payload(p),
                Frame::Signal(s) => stored_frame::Frame::Signal(s),
            }),
        }
    }

    fn into_frame(self) -> Option<Frame> {
        match self.frame? {
            stored_frame::Frame::Payload(p) => Some(Frame::Payload(p)),
            stored_frame::Frame::Signal(s) => Some(Frame::Signal(s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn both_protos_round_trip_and_share_the_wire_format() {
        for frame in [Frame::Payload(vec![0, 255, 7]), Frame::Signal(-1)] {
            let filed = ControlFrame::from_frame(frame.clone());
            let stored = StoredFrame::from_frame(frame.clone());
            assert_eq!(filed.encode_to_vec(), stored.encode_to_vec());
            assert_eq!(filed.into_frame(), Some(frame.clone()));
            assert_eq!(stored.into_frame(), Some(frame));
        }
        assert_eq!(StoredFrame::default().into_frame(), None);
    }
}
