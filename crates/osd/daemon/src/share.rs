// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Ein Device, auf das mehrere Jobs zugleich schreiben (`Maximum Concurrent Jobs` > 1, OP-2).
//!
//! Zweck: bareos-sd schreibt die Blöcke mehrerer Jobs verschachtelt in das eingelegte Volume; das
//! Volume und seine Katalogzähler (`dev->VolCatInfo`) gehören dem Device, nicht dem Job. In osd
//! schreibt jeder Job in einem eigenen Prozess (D-13). Die Schreiber eines Devices teilen sich
//! deshalb eine kleine Gerätedatei im Arbeitsverzeichnis: das eingelegte Volume, seine MediaId,
//! die Zähler und die PIDs der Schreiber, die es gerade benutzen. Wer etwas am Volume tut (Block
//! anhängen, Volume wechseln, Katalog melden), hält die Sperre der Datei (`flock`). Ein
//! abgestürzter Schreiber hält keine Sperre; seine PID fällt beim nächsten Lesen heraus.
//!
//! Bareos-Gegenstück: `core/src/stored/dev.h` (`VolCatInfo`, `num_writers`),
//! `stored/acquire.cc` (`AcquireDeviceForAppend` auf einem schon beschriebenen Device).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::Path;

use osd::askdir::UpdateMedia;

/// What the writers of one device share.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceState {
    /// The mounted volume (empty: none).
    pub volume: String,
    /// Its `MediaId`.
    pub media_id: u64,
    /// The writers using it.
    pub pids: Vec<u32>,
    /// Its catalog counters.
    pub cat: UpdateMedia,
}

impl DeviceState {
    /// `key=value` lines.
    fn encode(&self) -> String {
        let c = &self.cat;
        let pids: Vec<String> = self.pids.iter().map(u32::to_string).collect();
        format!(
            "volume={}\nmedia_id={}\npids={}\njobs={}\nfiles={}\nblocks={}\nbytes={}\nmounts={}\nerrors={}\nwrites={}\n\
             max_bytes={}\nend_time={}\nstatus={}\nslot={}\nin_changer={}\nread_time={}\nwrite_time={}\nfirst_written={}\n",
            self.volume, self.media_id, pids.join(","), c.jobs, c.files, c.blocks, c.bytes, c.mounts, c.errors, c.writes,
            c.max_bytes, c.end_time, c.status, c.slot, c.in_changer as u8, c.read_time, c.write_time, c.first_written
        )
    }

    /// From the file; an empty or unreadable file is no state.
    fn decode(text: &str) -> DeviceState {
        let get = |k: &str| text.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix('='))).unwrap_or("");
        let n = |k: &str| get(k).parse::<u64>().unwrap_or(0);
        let i = |k: &str| get(k).parse::<i64>().unwrap_or(0);
        let volume = get("volume").to_string();
        DeviceState {
            media_id: n("media_id"),
            pids: get("pids").split(',').filter_map(|p| p.parse().ok()).collect(),
            cat: UpdateMedia {
                volume: volume.clone(),
                jobs: n("jobs") as u32,
                files: n("files") as u32,
                blocks: n("blocks") as u32,
                bytes: n("bytes"),
                mounts: n("mounts") as u32,
                errors: n("errors") as u32,
                writes: n("writes") as u32,
                max_bytes: n("max_bytes"),
                end_time: n("end_time"),
                status: get("status").to_string(),
                slot: i("slot") as i32,
                relabel: false,
                in_changer: n("in_changer") != 0,
                read_time: i("read_time"),
                write_time: i("write_time"),
                first_written: n("first_written"),
            },
            volume,
        }
    }

    /// Whether a writer other than `me` uses the volume.
    pub fn others(&self, me: u32) -> bool {
        self.pids.iter().any(|p| *p != me)
    }
}

/// Whether process `pid` lives.
fn alive(pid: u32) -> bool {
    // SAFETY: kill with signal 0 only checks the process.
    pid > 0 && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
}

/// The open device file of one writer.
pub struct DeviceShare {
    /// The file (`flock` on it).
    file: File,
    /// Whether this writer holds the lock now.
    locked: bool,
}

impl DeviceShare {
    /// Opens (creates) the device file `path`.
    pub fn open(path: &Path) -> Result<DeviceShare, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| format!("device state {}: {e}", path.display()))?;
        Ok(DeviceShare { file, locked: false })
    }

    /// Whether this writer holds the lock.
    pub fn locked(&self) -> bool {
        self.locked
    }

    /// Takes the lock (waits for the other writers); a no-op when held.
    pub fn lock(&mut self) -> Result<(), String> {
        if self.locked {
            return Ok(());
        }
        // SAFETY: flock on an open file descriptor.
        if unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(format!("device state lock: {}", std::io::Error::last_os_error()));
        }
        self.locked = true;
        Ok(())
    }

    /// Gives the lock up.
    pub fn unlock(&mut self) {
        if self.locked {
            // SAFETY: flock on an open file descriptor.
            unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
            self.locked = false;
        }
    }

    /// The state, without writers that died (lock held).
    pub fn read(&mut self) -> DeviceState {
        let mut text = String::new();
        let _ = self.file.seek(SeekFrom::Start(0)).and_then(|_| self.file.read_to_string(&mut text));
        let mut state = DeviceState::decode(&text);
        state.pids.retain(|p| alive(*p));
        state
    }

    /// Writes the state (lock held).
    pub fn write(&mut self, state: &DeviceState) -> Result<(), String> {
        let text = state.encode();
        self.file
            .set_len(0)
            .and_then(|_| self.file.seek(SeekFrom::Start(0)))
            .and_then(|_| self.file.write_all(text.as_bytes()))
            .map_err(|e| format!("device state write: {e}"))
    }
}

impl Drop for DeviceShare {
    fn drop(&mut self) {
        self.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip_and_dead_writers_drop_out() {
        let dir = std::env::temp_dir().join(format!("osd-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Dev.device");
        let mut share = DeviceShare::open(&path).unwrap();
        share.lock().unwrap();
        assert_eq!(share.read(), DeviceState::default(), "a new file is no state");
        let me = std::process::id();
        let state = DeviceState {
            volume: "Vol 1".into(),
            media_id: 7,
            pids: vec![me, u32::MAX - 1],
            cat: UpdateMedia { volume: "Vol 1".into(), jobs: 2, bytes: 1 << 33, status: "Append".into(), write_time: 12, ..Default::default() },
        };
        share.write(&state).unwrap();
        let back = share.read();
        assert_eq!(back.pids, vec![me], "the pid of no process is dropped");
        assert_eq!(DeviceState { pids: vec![me], ..state.clone() }, back);
        assert!(!back.others(me));
        share.unlock();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
