// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Bandbreitenbegrenzung einer Verbindung (`setbandwidth`, `Maximum Bandwidth`).
//!
//! Zweck: Ein Job darf auf seiner Datenverbindung höchstens eine eingestellte
//! Zahl Bytes je Sekunde bewegen. [`BwState`] hält Grenze und Zähler und
//! bremst den aufrufenden Thread, sobald zu viel durchging; [`Limited`]
//! hängt das vor `read` und `write` eines beliebigen Stroms. Die Grenze
//! lässt sich während des Jobs ändern (Director-Befehl `setbandwidth`), weil
//! [`BwState`] über `Arc` geteilt wird.
//!
//! Datenfluss: Director `setbandwidth`/Job-Ressource → `BwState::limit` →
//! jede Lese-/Schreiboperation auf [`Limited`] → [`BwState::control`]
//! schläft, bis der Durchsatz wieder unter der Grenze liegt.
//!
//! Bareos-Gegenstück: `core/src/lib/bsock.cc` (`SetBwlimit`,
//! `ControlBwlimit`).
//!
//! Herkunft: `ofd/src/sd.rs` (`BwState`, `Limited`), am 20.11.2026
//! unverändert hierher verschoben; `Limited` ist jetzt generisch über den
//! inneren Strom.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// BareosSocket bwlimit state (SetBwlimit, ControlBwlimit).
#[derive(Default)]
pub struct BwState {
    /// bytes per second, 0: no limit
    pub limit: AtomicU64,
    /// `Allow Bandwidth Bursting`: unused budget may be spent later.
    pub bursting: AtomicBool,
    /// (nb_bytes, last_tick in microseconds)
    ticks: Mutex<(i64, i64)>,
}

/// Wall clock in microseconds since the epoch.
fn now_us() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros() as i64).unwrap_or(0)
}

impl BwState {
    /// ControlBwlimit(): accounts `bytes` and sleeps while over the limit.
    pub fn control(&self, bytes: usize) {
        let limit = self.limit.load(Ordering::Relaxed);
        if limit == 0 || bytes == 0 {
            return;
        }
        let rate = limit as f64 / 1_000_000.0;
        let bursting = self.bursting.load(Ordering::Relaxed);
        let mut t = self.ticks.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_us();
        if t.1 == 0 {
            *t = (bytes as i64, now);
            return;
        }
        let temp = now - t.1;
        if temp < 100 {
            t.0 += bytes as i64;
            return;
        }
        t.0 += bytes as i64;
        t.1 = now;
        if temp > 10_000_000 {
            return;
        }
        t.0 -= (temp as f64 * rate) as i64;
        if t.0 < 0 {
            if !bursting {
                t.0 = 0;
            }
            return;
        }
        let mut usec_sleep = (t.0 as f64 / rate) as i64;
        if usec_sleep > 100 {
            loop {
                std::thread::sleep(Duration::from_micros(usec_sleep as u64));
                let now = now_us();
                if now - t.1 < usec_sleep {
                    usec_sleep -= now - t.1;
                    continue;
                }
                t.1 = now;
                break;
            }
            if bursting {
                t.0 -= (usec_sleep as f64 * rate) as i64;
            } else {
                t.0 = 0;
            }
        }
    }
}

/// A stream whose reads and writes are bandwidth limited.
pub struct Limited<S> {
    /// The stream data actually goes over.
    pub inner: S,
    /// Shared limit and counters.
    pub bw: Arc<BwState>,
}

impl<S: Read> Read for Limited<S> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(b)?;
        self.bw.control(n);
        Ok(n)
    }
}

impl<S: Write> Write for Limited<S> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(b)?;
        self.bw.control(n);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn no_limit_passes_everything_through() {
        let bw = Arc::new(BwState::default());
        let mut s = Limited { inner: std::io::Cursor::new(Vec::new()), bw };
        let start = Instant::now();
        for _ in 0..100 {
            s.write_all(&[0u8; 4096]).unwrap();
        }
        assert_eq!(s.inner.get_ref().len(), 409_600);
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn limit_slows_writes_down() {
        // 200 KiB/s, 100 KiB written after the first tick: about 0.5 s.
        // Bursts within 100 µs are only counted, so the writes are spaced.
        let bw = Arc::new(BwState::default());
        bw.limit.store(200 * 1024, Ordering::Relaxed);
        let mut s = Limited { inner: std::io::sink(), bw };
        s.write_all(&[0u8; 1024]).unwrap();
        let start = Instant::now();
        for _ in 0..100 {
            std::thread::sleep(Duration::from_micros(150));
            s.write_all(&[0u8; 1024]).unwrap();
        }
        let took = start.elapsed();
        assert!(took >= Duration::from_millis(300), "{took:?}");
        assert!(took < Duration::from_secs(3), "{took:?}");
    }
}
