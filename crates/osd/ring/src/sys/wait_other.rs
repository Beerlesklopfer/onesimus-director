// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! macOS und Windows: Warten durch Abfragen mit Zeitgrenze (siehe [`super`]).
//!
//! Zweck: Der Ring wartet immer mit Zeitgrenze und prüft nach jedem Warten
//! seine Bedingung neu. Ein Wecken ist deshalb nur eine Abkürzung: ohne
//! prozessübergreifendes Wecken sieht der Wartende die Änderung beim
//! nächsten Abfragen (höchstens eine Millisekunde später).
//!
//! Vorgesehen als spätere Optimierung (Recherche Nachtrag C): macOS
//! `os_sync_wait_on_address`/`os_sync_wake_by_address_all` mit
//! `OS_SYNC_WAIT_ON_ADDRESS_SHARED` (ab 14.4); Windows je Wartendem ein
//! benanntes Auto-Reset-Event, dessen Name im Ringkopf steht
//! (`WaitOnAddress` wirkt nicht prozessübergreifend).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Spins with `yield_now` before the first sleep.
const YIELDS: u32 = 64;
/// Longest single sleep between two looks at the word.
const MAX_SLEEP: Duration = Duration::from_millis(1);

/// Cross-process wait: returns when `word` differs from `expected` or after
/// `timeout`, whichever is first (polls; the caller rechecks its condition).
pub fn wait(word: &AtomicU32, expected: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    let mut round = 0u32;
    while word.load(Ordering::Acquire) == expected {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        if round < YIELDS {
            std::thread::yield_now();
            round += 1;
        } else {
            std::thread::sleep(MAX_SLEEP.min(deadline - now));
        }
    }
}

/// Cross-process wake: nothing to do, waiters poll.
pub fn wake(_word: &AtomicU32) {}
