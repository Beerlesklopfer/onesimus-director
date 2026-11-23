// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Zweig-Nachrichten (Branch) zwischen odir und osd.
//!
//! Zweck: Zweige (3-2-1 im laufenden Betrieb, Plan
//! `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`) sind eine Onesimus-Erweiterung
//! und laufen nur über gRPC. Auf dem Draht sind sie typisierte Proto-Nachrichten
//! (`onesimus.stored.BranchFrame` im Feld `branch` des `StoredFrame`). Im
//! Prozess, wo alles Rahmen aus BNET-Nachrichten sind, erscheint eine solche
//! Nachricht als Nutzdaten mit einer Kennung vorn: [`BranchMessage::encode`]
//! liefert sie, [`BranchMessage::decode`] liest sie. Der Rahmenwandler
//! ([`crate::frame`]) macht daraus beim Senden die typisierte Nachricht und
//! beim Empfangen wieder die Kennung samt Nutzdaten; Befehlsschleifen und
//! `bareos_bnet::BnetStream` bleiben unverändert.
//!
//! Über BNET gibt es sie nicht: ein bareos-sd oder bareos-dir sieht die
//! Kennung nie, weil odir sie nur auf einer gRPC-Verbindung zu osd sendet.
//!
//! Bareos-Gegenstück: keins (Onesimus-Erweiterung).

use onesimus_proto_rs::onesimus::stored::{branch_frame, BranchAttach, BranchFrame, BranchPlan, BranchReply};
use prost::Message;

/// Prefix of the payload that stands for a [`BranchMessage`] inside the
/// process (never sent as such: the wire form is the typed `BranchFrame`).
pub const MARK: &[u8] = b"\0onesimus.stored.BranchFrame\0";

/// One message about branches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchMessage {
    /// Director to SD on a main job's connection: the unique names of the
    /// branch jobs that will attach to it.
    Plan(Vec<String>),
    /// Director to SD on a branch job's connection, in place of `run`: the
    /// unique name of the main job whose stream the branch takes.
    Attach(String),
    /// SD to Director: the answer to a plan or an attach; `Err` carries the
    /// reason of a refusal.
    Reply(Result<(), String>),
}

impl BranchMessage {
    /// The typed wire message.
    pub fn to_proto(&self) -> BranchFrame {
        let message = match self {
            BranchMessage::Plan(jobs) => branch_frame::Message::Plan(BranchPlan { branch_jobs: jobs.clone() }),
            BranchMessage::Attach(main) => branch_frame::Message::Attach(BranchAttach { main_job: main.clone() }),
            BranchMessage::Reply(result) => branch_frame::Message::Reply(BranchReply {
                ok: result.is_ok(),
                text: result.as_ref().err().cloned().unwrap_or_default(),
            }),
        };
        BranchFrame { message: Some(message) }
    }

    /// The message of a typed wire message; `None` when it is empty.
    pub fn from_proto(frame: BranchFrame) -> Option<BranchMessage> {
        Some(match frame.message? {
            branch_frame::Message::Plan(plan) => BranchMessage::Plan(plan.branch_jobs),
            branch_frame::Message::Attach(attach) => BranchMessage::Attach(attach.main_job),
            branch_frame::Message::Reply(reply) => BranchMessage::Reply(if reply.ok { Ok(()) } else { Err(reply.text) }),
        })
    }

    /// The in-process payload: [`MARK`] and the encoded `BranchFrame`.
    pub fn encode(&self) -> Vec<u8> {
        let mut payload = MARK.to_vec();
        payload.extend(self.to_proto().encode_to_vec());
        payload
    }

    /// The message of an in-process payload; `None` for any other payload.
    pub fn decode(payload: &[u8]) -> Option<BranchMessage> {
        BranchFrame::decode(payload.strip_prefix(MARK)?).ok().and_then(BranchMessage::from_proto)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_survives_the_payload_form() {
        for message in [
            BranchMessage::Plan(vec!["B.2026-11-23_10.00.00_01".into(), "C.2026-11-23_10.00.00_02".into()]),
            BranchMessage::Plan(Vec::new()),
            BranchMessage::Attach("Main.2026-11-23_10.00.00_03".into()),
            BranchMessage::Reply(Ok(())),
            BranchMessage::Reply(Err("no such main job".into())),
        ] {
            assert_eq!(BranchMessage::decode(&message.encode()), Some(message));
        }
    }

    #[test]
    fn other_payloads_are_not_branch_messages() {
        assert_eq!(BranchMessage::decode(b"run"), None);
        assert_eq!(BranchMessage::decode(b""), None);
        assert_eq!(BranchMessage::decode(MARK), None, "the mark alone is an empty frame");
    }
}
