// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Shared logic of the Podman plugin (docs/plans/PODMAN-PLUGIN.md).
//!
//! # Purpose
//!
//! Everything the host (`ofd`) and the rootless broker
//! (`onesimus-podman-broker`) must decide the same way, without system
//! calls that need privileges: the plugin line ([`line`](mod@line)), the root-owned
//! account policy ([`policy`]), ID maps from `/etc/subuid` ([`idmap`]),
//! the rules that make a volume file fit for the Bareos stream
//! ([`sanitize`], V-1, S-13), the `ownership.tsv` the host writes for every
//! volume ([`ownership`]), the lifecycle journal with its recovery
//! decisions ([`journal`]), the fixed argument lists of every program the
//! broker or the rootful backend may run ([`exec`]) and the positive lists
//! that filter every answer of Podman ([`positive`]).
//!
//! # Data flow
//!
//! The guest decides tree, manifest and order; the host decides everything
//! that concerns security; the broker only reports metadata as the
//! account. This crate holds the host's rules; the scan itself (openat2 as
//! root) lives in `ofd` (`podman_scan.rs`).
//!
//! ```mermaid
//! flowchart LR
//!     G[WASM guest podman] -- WIT podman / podman-volume --> H[ofd: PodmanPort]
//!     H -- line::PodmanLine, policy::AccountPolicy --> H
//!     H -- exec::Operation argv --> B[onesimus-podman-broker<br/>account uid, root-owned userns]
//!     B -- podman --format json --> P[(podman)]
//!     B -- raw JSON --> H
//!     H -- positive::filter --> G
//!     H -- openat2 scan --> S[(volume _data)]
//!     S -- sanitize::classify --> H
//!     H -- stream attributes 0o777, host IDs --> SD[(SD stream)]
//!     H -- ownership::Table --> SD
//!     H -- journal::Journal --> J[(podman-journal/&lt;domain&gt;/&lt;job&gt;.json)]
//! ```
//!
//! ```mermaid
//! classDiagram
//!     class PodmanLine { user; selection; env; consistency; max_downtime }
//!     class AccountPolicy { accounts }
//!     class IdMap { ranges; to_ns(); contains() }
//!     class RawEntry { relpath; kind; mode; uid; gid; xattrs }
//!     class Outcome { Stream | Rejected }
//!     class Table { rows; render() }
//!     class Journal { state; members; recovery() }
//!     RawEntry --> Outcome : sanitize::classify
//!     IdMap --> Outcome
//!     Outcome --> Table : ownership row
//!     AccountPolicy --> PodmanLine : user must be confirmed
//! ```
//!
//! # Counterparts
//!
//! `ofd` (`podman_port.rs`, `podman_scan.rs`, `podman_ns.rs`,
//! `podman_rootful.rs`) uses every module; `onesimus-podman-broker` uses
//! [`exec`] and the frame limits. The guest never links this crate.

#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod exec;
pub mod idmap;
pub mod journal;
pub mod line;
pub mod ownership;
pub mod policy;
pub mod positive;
pub mod sanitize;

/// The plugin id every Podman grant and import is bound to (V-11).
pub const PLUGIN_ID: &str = "io.onesimus.podman";

/// Largest frame between host and broker (4-byte length + message).
pub const MAX_FRAME: usize = 2 * 1024 * 1024;

/// Largest single inspect or spec answer the host accepts (1 MiB).
pub const MAX_INSPECT: usize = 1024 * 1024;

/// Writes one frame: 4-byte big-endian length, then `message`.
pub fn write_frame(out: &mut impl std::io::Write, message: &[u8]) -> std::io::Result<()> {
    if message.len() > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    out.write_all(&(message.len() as u32).to_be_bytes())?;
    out.write_all(message)?;
    out.flush()
}

/// Reads one frame; `Ok(None)` at a clean end of the stream.
pub fn read_frame(input: &mut impl std::io::Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut buf = vec![0u8; len];
    input.read_exact(&mut buf)?;
    Ok(Some(buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_are_bounded() {
        let mut buf = Vec::new();
        write_frame(&mut buf, b"hello").unwrap();
        write_frame(&mut buf, b"").unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"hello");
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"");
        assert!(read_frame(&mut r).unwrap().is_none());
        let big = (MAX_FRAME as u32 + 1).to_be_bytes();
        assert!(read_frame(&mut big.as_slice()).is_err());
        assert!(write_frame(&mut Vec::new(), &vec![0; MAX_FRAME + 1]).is_err());
    }
}
