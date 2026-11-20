//! # osd-ring – der Blockring von osd im gemeinsamen Speicher
//!
//! Zweck: osd verteilt den Datenstrom eines Jobs über einen Ring im
//! gemeinsamen Speicher an die Geräteschreiber (Plan
//! `docs/plans/OSD-GERAETE-UND-VERTEILUNG.md`, „Datenpfad: Ringpuffer in
//! /dev/shm“, D-12, D-13). Ein **Erzeuger** (die Netzsitzung) legt Blöcke in
//! **Fächer**, N **Verbraucher** (Geräteschreiber, eigene Prozesse) lesen
//! sie. Synchronisiert wird nur über Wörter im Kopf des Rings:
//! Schreibzeiger, je Verbraucher Lesezeiger, Zustand und PID, Warten und
//! Wecken prozessübergreifend ([`sys`]). Dazu kommt je Verbraucher ein
//! **Postfach** für Anfragen an den Erzeuger (CatReq, Job-Meldungen) und
//! ein **Info-Bereich** für die Job-Angaben. Es gibt keinen weiteren Kanal.
//!
//! Regeln:
//!
//! - Ein Fach ist frei, wenn jeder Verbraucher, der noch zählt, es gelesen
//!   hat (Freigabe = kleinster Lesezeiger). Ein Verbraucher im Zustand
//!   [`State::Failed`] oder [`State::Done`] zählt nicht mehr; einer, dessen
//!   Prozess verschwunden ist, wird beim Warten als gescheitert markiert.
//! - Ein noch nicht angemeldeter Verbraucher ([`State::Free`]) hält die
//!   Fächer fest, damit nichts verloren geht, bevor er startet.
//! - Stufe 1 hat genau einen Verbraucher; der Code ist für N geschrieben.
//!
//! Schnittstelle (plattformneutral, der Ring heißt nur beim Namen):
//! [`Ring::create`]/[`Ring::open`]/[`Ring::remove`]/[`Ring::remove_stale`];
//! Erzeuger [`Ring::claim`] → [`SlotMut::publish`], [`Ring::finish`],
//! [`Ring::serve_mail`]; Verbraucher [`Ring::attach`], [`Ring::next`] →
//! [`Ring::release`], [`Ring::call`], [`Ring::set_consumer_state`].
//!
//! Lesen (Restore) nutzt denselben Ring in Gegenrichtung: Die Netzsitzung
//! legt ihn an und meldet sich als Verbraucher an, der Geräteleser
//! übernimmt mit [`Ring::take_producer`] die Rolle des Erzeugers und füllt
//! die Fächer mit gelesenen Blöcken. Seine Anfragen (CatReq, Job-Meldungen)
//! gehen mit [`Ring::ask_consumer`] an die Netzsitzung, die sie mit
//! [`Ring::serve_producer`] beantwortet; das Postfach eines Rings trägt
//! immer nur eine Richtung.
//!
//! Aufbau des Speichers (alle Zahlen im Prozess-eigenen Format, der Ring
//! verlässt den Rechner nie):
//!
//! | Bereich | Größe | Inhalt |
//! |---|---|---|
//! | Kopf | 4096 | Kennung, Fachzahl, Fachgröße, Verbraucherzahl, Erzeuger-PID/-Zustand/-Startzeit/-Herzschlag; Schreibzeiger und jedes Weckwort auf eigener Cache-Zeile (64 B); je Verbraucher Lesezeiger (eigene Zeile), Zustand, PID, Startzeit, Herzschlag, Postfach-Zustand |
//! | Postfächer | 8 × 64 KiB | je Verbraucher Anfrage bzw. Antwort |
//! | Info | 8 KiB | Länge + Bytes (Job-Angaben) |
//! | Fachköpfe | N × 64 | je Fach Länge und 7 Metawörter |
//! | Fächer | N × Fachgröße, auf 4096 gerundet | Nutzdaten, seitenweise ausgerichtet (später O_DIRECT, ungetestet) |
//!
//! Muster: Disruptor (ein Erzeuger-Cursor, je Verbraucher eine Folge, der
//! Erzeuger wartet, solange Cursor − Fachzahl ≥ kleinste Folge). Aus Aeron
//! übernommen: Zähler auf eigenen Cache-Zeilen und eine Lebendfrist – ein
//! Verbraucher, dessen Herzschlag bei vollem Ring [`LIVENESS`] lang steht,
//! wird als gescheitert ausgeschlossen, statt den Job aufzuhalten.
//! Striping (Verbraucher k liest Fach ≡ k mod N) ist vorbereitet, aber noch
//! nicht umgesetzt (P5).
//!
//! Bareos-Gegenstück: keins. bareos-sd schreibt aus dem Job-Thread direkt
//! auf das Device (`stored/append.cc` `DoAppendData`, `stored/block.cc`
//! `WriteBlockToDevice`).
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]

pub mod glossary;
pub mod sys;

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Most consumers a ring can have.
pub const MAX_CONSUMERS: usize = 8;
/// Bytes of one mailbox (request or answer).
pub const MAILBOX_SIZE: usize = 64 * 1024;
/// Bytes of the info area.
pub const INFO_SIZE: usize = 8 * 1024;
/// Metadata words of a slot (free for the user).
pub const SLOT_META: usize = 7;

/// A consumer whose heartbeat stands still this long while the ring is
/// full is evicted (Aeron `client.liveness`).
pub const LIVENESS: Duration = Duration::from_secs(10);

/// Size of the header.
const HEADER_SIZE: usize = 4096;
/// Size of a slot header (length + meta words).
const SLOT_HEADER: usize = 64;
/// Page size the payloads are aligned to.
const PAGE: usize = 4096;
/// "OSDRING2".
const MAGIC: u64 = u64::from_le_bytes(*b"OSDRING2");
/// Header offset of the producer's pid.
const PRODUCER_PID: usize = 32;
/// Header offset of the producer's state.
const PRODUCER_STATE: usize = 40;
/// Header offset of the producer's start time (seconds since 1970).
const PRODUCER_START: usize = 48;
/// Header offset of the write index (own cache line).
const WRITE_INDEX: usize = 64;
/// Header offset of the publish wait word (own cache line).
const WRITE_SEQ: usize = 128;
/// Header offset of the release wait word (own cache line).
const READ_SEQ: usize = 192;
/// Header offset of the mailbox wait word (own cache line).
const MAIL_SEQ: usize = 256;
/// Offset of the first consumer record in the header.
const CONSUMER_BASE: usize = 512;
/// Size of a consumer record (read index on its own cache line).
const CONSUMER_SIZE: usize = 256;
/// Longest single wait before conditions and peers are rechecked.
const POLL: Duration = Duration::from_millis(100);

/// State of the producer or of a consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Not (yet) attached.
    Free = 0,
    /// Working.
    Running = 1,
    /// Finished normally.
    Done = 2,
    /// Failed; a failed consumer no longer holds slots.
    Failed = 3,
}

impl State {
    /// From a header word.
    fn from_word(v: u64) -> State {
        match v {
            1 => State::Running,
            2 => State::Done,
            3 => State::Failed,
            _ => State::Free,
        }
    }
}

/// Why a ring operation did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RingError {
    /// Nothing happened within the timeout.
    Timeout,
    /// No consumer holds slots any more (all failed or done).
    NoConsumer,
    /// The producer failed or its process is gone.
    ProducerGone,
    /// A mailbox message does not fit.
    TooLarge(usize),
    /// Creating, opening or mapping failed.
    Io(String),
}

impl std::fmt::Display for RingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RingError::Timeout => write!(f, "timeout"),
            RingError::NoConsumer => write!(f, "no device writer left"),
            RingError::ProducerGone => write!(f, "network session gone"),
            RingError::TooLarge(n) => write!(f, "message of {n} bytes does not fit the mailbox"),
            RingError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RingError {}

/// Sizes of a new ring.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingConfig {
    /// Number of slots.
    pub slots: u32,
    /// Payload bytes per slot (one Bareos block, D-12).
    pub slot_size: u32,
    /// Number of consumers (1 in stage 1, at most [`MAX_CONSUMERS`]).
    pub consumers: u32,
}

#[cfg_attr(doc, aquamarine::aquamarine)]
/// One mapped ring.
///
/// ```mermaid
/// classDiagram
///     class Ring {
///         +create(name, RingConfig) Ring
///         +open(name) Ring
///         +remove(name)
///         +remove_stale(prefix)
///         +claim(timeout) SlotMut
///         +finish(State)
///         +serve_mail(timeout, handler)
///         +attach(consumer)
///         +next(consumer, timeout) SlotRef
///         +release(consumer)
///         +call(consumer, request) answer
///         +take_producer()
///         +ask_consumer(consumer, request) answer
///         +serve_producer(consumer, timeout, handler)
///     }
///     class SlotMut { +data +meta +publish(len) }
///     class SlotRef { +seq +data +meta }
///     class Mapping { memmap2 }
///     Ring *-- Mapping : sys
///     Ring ..> SlotMut : Erzeuger
///     Ring ..> SlotRef : Verbraucher
/// ```
#[derive(Debug)]
pub struct Ring {
    /// The ring's name.
    name: String,
    /// The shared memory.
    map: sys::Mapping,
    /// Slot count.
    slots: u64,
    /// Payload bytes per slot.
    slot_size: usize,
    /// Distance between slots.
    slot_stride: usize,
    /// Consumer count.
    consumers: usize,
    /// Producer side: last seen heartbeat of each consumer and since when.
    seen: std::sync::Mutex<Vec<(u64, Instant)>>,
    /// Eviction timeout (default [`LIVENESS`]).
    liveness: Duration,
}

/// A claimed slot of the producer, filled and then published.
#[derive(Debug)]
pub struct SlotMut<'a> {
    /// The ring.
    ring: &'a Ring,
    /// Sequence number of the slot (write index).
    seq: u64,
    /// The payload area.
    pub data: &'a mut [u8],
    /// The metadata words.
    pub meta: [u64; SLOT_META],
}

/// A slot a consumer reads.
#[derive(Debug, Clone, Copy)]
pub struct SlotRef<'a> {
    /// Sequence number of the slot.
    pub seq: u64,
    /// The payload (exactly the published length).
    pub data: &'a [u8],
    /// The metadata words.
    pub meta: [u64; SLOT_META],
}

impl Ring {
    /// Offset of the slot header table.
    const fn table_offset() -> usize {
        HEADER_SIZE + MAX_CONSUMERS * MAILBOX_SIZE + INFO_SIZE
    }

    /// Offset of the first payload for `slots` slots (page aligned).
    fn payload_offset(slots: usize) -> usize {
        (Self::table_offset() + slots * SLOT_HEADER).div_ceil(PAGE) * PAGE
    }

    /// Total size of a ring with `cfg`.
    fn size_of(cfg: &RingConfig) -> usize {
        Self::payload_offset(cfg.slots as usize) + cfg.slots as usize * Self::stride(cfg.slot_size as usize)
    }

    /// Distance between payloads for `slot_size` (whole pages).
    fn stride(slot_size: usize) -> usize {
        slot_size.div_ceil(PAGE) * PAGE
    }

    /// A ring value around `map`.
    fn wrap(name: &str, map: sys::Mapping, cfg: &RingConfig) -> Ring {
        Ring {
            name: name.to_string(),
            map,
            slots: cfg.slots.into(),
            slot_size: cfg.slot_size as usize,
            slot_stride: Self::stride(cfg.slot_size as usize),
            consumers: cfg.consumers as usize,
            seen: std::sync::Mutex::new(vec![(0, Instant::now()); cfg.consumers as usize]),
            liveness: LIVENESS,
        }
    }

    /// Sets the eviction timeout of stalled consumers (default [`LIVENESS`]).
    pub fn set_liveness(&mut self, timeout: Duration) {
        self.liveness = timeout;
    }

    /// Creates ring `name` and makes the calling process its producer.
    pub fn create(name: &str, cfg: RingConfig) -> Result<Ring, RingError> {
        if cfg.slots == 0 || cfg.slot_size == 0 || cfg.consumers == 0 || cfg.consumers as usize > MAX_CONSUMERS {
            return Err(RingError::Io(format!("invalid ring configuration {cfg:?}")));
        }
        let map = sys::create(name, Self::size_of(&cfg)).map_err(|e| RingError::Io(format!("ring {name}: {e}")))?;
        let ring = Self::wrap(name, map, &cfg);
        ring.word(8).store(cfg.slots.into(), Ordering::Relaxed);
        ring.word(16).store(cfg.slot_size.into(), Ordering::Relaxed);
        ring.word(24).store(cfg.consumers.into(), Ordering::Relaxed);
        ring.word(PRODUCER_PID).store(sys::current_pid(), Ordering::Relaxed);
        ring.word(PRODUCER_START).store(unix_now(), Ordering::Relaxed);
        ring.word(PRODUCER_STATE).store(State::Running as u64, Ordering::Relaxed);
        ring.word(0).store(MAGIC, Ordering::Release);
        Ok(ring)
    }

    /// Opens the existing ring `name` (a consumer, or a check).
    pub fn open(name: &str) -> Result<Ring, RingError> {
        let map = sys::open(name).map_err(|e| RingError::Io(format!("ring {name}: {e}")))?;
        if map.len() < HEADER_SIZE {
            return Err(RingError::Io(format!("ring {name}: too short")));
        }
        let probe = Self::wrap(name, map, &RingConfig { slots: 0, slot_size: 0, consumers: 0 });
        if probe.word(0).load(Ordering::Acquire) != MAGIC {
            return Err(RingError::Io(format!("ring {name}: not an osd ring")));
        }
        let cfg = RingConfig {
            slots: probe.word(8).load(Ordering::Relaxed) as u32,
            slot_size: probe.word(16).load(Ordering::Relaxed) as u32,
            consumers: probe.word(24).load(Ordering::Relaxed) as u32,
        };
        if Self::size_of(&cfg) != probe.map.len() || cfg.consumers as usize > MAX_CONSUMERS {
            return Err(RingError::Io(format!("ring {name}: inconsistent header {cfg:?}")));
        }
        Ok(Self::wrap(name, probe.map, &cfg))
    }

    /// Removes ring `name`; existing mappings stay valid.
    pub fn remove(name: &str) -> Result<(), RingError> {
        sys::unlink(name).map_err(|e| RingError::Io(format!("ring {name}: {e}")))
    }

    /// Removes rings starting with `prefix` whose producer process is gone
    /// (or that are not readable rings); returns their names.
    pub fn remove_stale(prefix: &str) -> Vec<String> {
        let mut removed = Vec::new();
        for name in sys::list(prefix).unwrap_or_default() {
            let stale = match Ring::open(&name) {
                Ok(ring) => !sys::pid_alive(ring.word(PRODUCER_PID).load(Ordering::Relaxed)),
                Err(_) => true,
            };
            if stale && sys::unlink(&name).is_ok() {
                removed.push(name);
            }
        }
        removed
    }

    /// The ring's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Payload bytes per slot.
    pub fn slot_size(&self) -> usize {
        self.slot_size
    }

    /// Number of slots.
    pub fn slots(&self) -> u64 {
        self.slots
    }

    /// Number of consumers.
    pub fn consumers(&self) -> usize {
        self.consumers
    }

    /// The u64 header word at `offset`.
    fn word(&self, offset: usize) -> &AtomicU64 {
        debug_assert!(offset.is_multiple_of(8) && offset + 8 <= HEADER_SIZE);
        // SAFETY: inside the mapping, 8-byte aligned (mmap is page aligned).
        unsafe { &*(self.map.ptr().add(offset) as *const AtomicU64) }
    }

    /// The u32 wait word at `offset`.
    fn word32(&self, offset: usize) -> &AtomicU32 {
        debug_assert!(offset.is_multiple_of(4) && offset + 4 <= HEADER_SIZE);
        // SAFETY: inside the mapping, 4-byte aligned.
        unsafe { &*(self.map.ptr().add(offset) as *const AtomicU32) }
    }

    /// Write index: number of published slots.
    fn write_index(&self) -> &AtomicU64 {
        self.word(WRITE_INDEX)
    }

    /// Bumped on every publish and producer state change.
    fn write_seq(&self) -> &AtomicU32 {
        self.word32(WRITE_SEQ)
    }

    /// Bumped on every release and consumer state change.
    fn read_seq(&self) -> &AtomicU32 {
        self.word32(READ_SEQ)
    }

    /// Bumped when a consumer posts a request.
    fn mail_seq(&self) -> &AtomicU32 {
        self.word32(MAIL_SEQ)
    }

    /// Read index of consumer `c`.
    fn read_index(&self, c: usize) -> &AtomicU64 {
        self.word(CONSUMER_BASE + c * CONSUMER_SIZE)
    }

    /// State word of consumer `c`.
    fn consumer_word(&self, c: usize) -> &AtomicU64 {
        self.word(CONSUMER_BASE + c * CONSUMER_SIZE + 64)
    }

    /// PID of consumer `c`.
    fn consumer_pid(&self, c: usize) -> &AtomicU64 {
        self.word(CONSUMER_BASE + c * CONSUMER_SIZE + 72)
    }

    /// Start time of consumer `c` (seconds since 1970).
    fn consumer_start(&self, c: usize) -> &AtomicU64 {
        self.word(CONSUMER_BASE + c * CONSUMER_SIZE + 80)
    }

    /// Heartbeat counter of consumer `c`.
    fn heartbeat(&self, c: usize) -> &AtomicU64 {
        self.word(CONSUMER_BASE + c * CONSUMER_SIZE + 88)
    }

    /// Mailbox state of consumer `c`: 0 empty, 1 request, 2 answer.
    fn mail_state(&self, c: usize) -> &AtomicU32 {
        self.word32(CONSUMER_BASE + c * CONSUMER_SIZE + 96)
    }

    /// Mailbox message length of consumer `c`.
    fn mail_len(&self, c: usize) -> &AtomicU64 {
        self.word(CONSUMER_BASE + c * CONSUMER_SIZE + 104)
    }

    /// Consumer `c` is alive: one heartbeat.
    pub fn beat(&self, c: usize) {
        self.heartbeat(c).fetch_add(1, Ordering::Relaxed);
    }

    /// The mailbox bytes of consumer `c`.
    #[allow(clippy::mut_from_ref)]
    fn mailbox(&self, c: usize) -> &mut [u8] {
        // SAFETY: inside the mapping; access is serialised by mail_state.
        unsafe { std::slice::from_raw_parts_mut(self.map.ptr().add(HEADER_SIZE + c * MAILBOX_SIZE), MAILBOX_SIZE) }
    }

    /// The info area (length word + bytes).
    #[allow(clippy::mut_from_ref)]
    fn info_area(&self) -> &mut [u8] {
        // SAFETY: inside the mapping; written once by the producer before
        // consumers start.
        unsafe { std::slice::from_raw_parts_mut(self.map.ptr().add(HEADER_SIZE + MAX_CONSUMERS * MAILBOX_SIZE), INFO_SIZE) }
    }

    /// Pointer to slot header `index`.
    fn slot_ptr(&self, index: u64) -> *mut u8 {
        // SAFETY: index < slots, inside the mapping.
        unsafe { self.map.ptr().add(Self::table_offset() + index as usize * SLOT_HEADER) }
    }

    /// Pointer to the payload of slot `index` (page aligned).
    fn payload_ptr(&self, index: u64) -> *mut u8 {
        let base = Self::payload_offset(self.slots as usize);
        // SAFETY: index < slots, inside the mapping.
        unsafe { self.map.ptr().add(base + index as usize * self.slot_stride) }
    }

    /// Writes the info area (producer, before consumers start).
    pub fn set_info(&self, bytes: &[u8]) -> Result<(), RingError> {
        if bytes.len() + 8 > INFO_SIZE {
            return Err(RingError::TooLarge(bytes.len()));
        }
        let area = self.info_area();
        area[..8].copy_from_slice(&(bytes.len() as u64).to_ne_bytes());
        area[8..8 + bytes.len()].copy_from_slice(bytes);
        std::sync::atomic::fence(Ordering::Release);
        Ok(())
    }

    /// The info area's bytes.
    pub fn info(&self) -> Vec<u8> {
        std::sync::atomic::fence(Ordering::Acquire);
        let area = self.info_area();
        let len = (u64::from_ne_bytes(area[..8].try_into().unwrap_or_default()) as usize).min(INFO_SIZE - 8);
        area[8..8 + len].to_vec()
    }

    /// State of consumer `c`.
    pub fn consumer_state(&self, c: usize) -> State {
        State::from_word(self.consumer_word(c).load(Ordering::Acquire))
    }

    /// State of the producer.
    pub fn producer_state(&self) -> State {
        State::from_word(self.word(PRODUCER_STATE).load(Ordering::Acquire))
    }

    /// Sets the state of consumer `c` and wakes everyone.
    pub fn set_consumer_state(&self, c: usize, state: State) {
        self.consumer_word(c).store(state as u64, Ordering::Release);
        self.bump_all();
    }

    /// Wakes every waiter on every word.
    fn bump_all(&self) {
        for w in [self.read_seq(), self.write_seq(), self.mail_seq()] {
            w.fetch_add(1, Ordering::AcqRel);
            sys::wake(w);
        }
        for c in 0..self.consumers {
            sys::wake(self.mail_state(c));
        }
    }

    /// Marks consumers as failed whose process is gone or whose heartbeat
    /// stood still for the liveness timeout (D-11); true if any.
    fn reap_consumers(&self) -> bool {
        let mut any = false;
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        for c in 0..self.consumers {
            if self.consumer_state(c) != State::Running {
                continue;
            }
            let pid = self.consumer_pid(c).load(Ordering::Acquire);
            let beat = self.heartbeat(c).load(Ordering::Relaxed);
            if seen[c].0 != beat {
                seen[c] = (beat, Instant::now());
            }
            let dead = pid != 0 && !sys::pid_alive(pid);
            if dead || seen[c].1.elapsed() >= self.liveness {
                self.consumer_word(c).store(State::Failed as u64, Ordering::Release);
                any = true;
            }
        }
        drop(seen);
        if any {
            self.bump_all();
        }
        any
    }

    /// Smallest read index of the consumers that still hold slots; `None`
    /// when none does.
    fn min_read_index(&self) -> Option<u64> {
        (0..self.consumers)
            .filter(|&c| matches!(self.consumer_state(c), State::Free | State::Running))
            .map(|c| self.read_index(c).load(Ordering::Acquire))
            .min()
    }

    /// Producer: waits until a slot is free and returns it.
    pub fn claim(&self, timeout: Duration) -> Result<SlotMut<'_>, RingError> {
        let deadline = Instant::now() + timeout;
        loop {
            let seen = self.read_seq().load(Ordering::Acquire);
            let w = self.write_index().load(Ordering::Relaxed);
            let Some(min) = self.min_read_index() else { return Err(RingError::NoConsumer) };
            if w - min < self.slots {
                let p = self.payload_ptr(w % self.slots);
                // SAFETY: the slot is free (all holding consumers passed it)
                // and only the single producer writes slots.
                let data = unsafe { std::slice::from_raw_parts_mut(p, self.slot_size) };
                return Ok(SlotMut { ring: self, seq: w, data, meta: [0; SLOT_META] });
            }
            if self.reap_consumers() {
                continue;
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RingError::Timeout);
            }
            sys::wait(self.read_seq(), seen, POLL.min(deadline - now));
        }
    }

    /// Takes over the producer role in the calling process (the read
    /// direction: the creator, the network session, attaches as consumer
    /// and starts a device reader that fills the slots). Producer liveness
    /// checks then watch this process.
    pub fn take_producer(&self) {
        self.word(PRODUCER_PID).store(sys::current_pid(), Ordering::Release);
        self.word(PRODUCER_START).store(unix_now(), Ordering::Release);
        self.word(PRODUCER_STATE).store(State::Running as u64, Ordering::Release);
        self.bump_all();
    }

    /// Whether consumer `c` failed or its process is gone.
    fn consumer_gone(&self, c: usize) -> bool {
        self.consumer_state(c) == State::Failed || {
            let pid = self.consumer_pid(c).load(Ordering::Relaxed);
            pid != 0 && !sys::pid_alive(pid)
        }
    }

    /// Producer: sends `request` to consumer `c` and waits for its answer
    /// (the read direction: the device reader asks the network session for
    /// catalog requests and job messages). Uses the consumer's mailbox with
    /// the states 3 (request) and 4 (answer); a mailbox carries one
    /// direction per ring. `NoConsumer` when the consumer failed or its
    /// process is gone; a consumer that is `Done` still answers.
    pub fn ask_consumer(&self, c: usize, request: &[u8], timeout: Duration) -> Result<Vec<u8>, RingError> {
        if request.len() > MAILBOX_SIZE {
            return Err(RingError::TooLarge(request.len()));
        }
        if c >= self.consumers {
            return Err(RingError::Io(format!("ring {}: no consumer {c}", self.name)));
        }
        self.mailbox(c)[..request.len()].copy_from_slice(request);
        self.mail_len(c).store(request.len() as u64, Ordering::Relaxed);
        self.mail_state(c).store(3, Ordering::Release);
        self.mail_seq().fetch_add(1, Ordering::AcqRel);
        sys::wake(self.mail_seq());
        let deadline = Instant::now() + timeout;
        loop {
            if self.mail_state(c).load(Ordering::Acquire) == 4 {
                let len = (self.mail_len(c).load(Ordering::Relaxed) as usize).min(MAILBOX_SIZE);
                let answer = self.mailbox(c)[..len].to_vec();
                self.mail_state(c).store(0, Ordering::Release);
                return Ok(answer);
            }
            if self.consumer_gone(c) {
                self.mail_state(c).store(0, Ordering::Release);
                return Err(RingError::NoConsumer);
            }
            let now = Instant::now();
            if now >= deadline {
                self.mail_state(c).store(0, Ordering::Release);
                return Err(RingError::Timeout);
            }
            sys::wait(self.mail_state(c), 3, POLL.min(deadline - now));
        }
    }

    /// Consumer `c`: answers a pending request of the producer with
    /// `handler(request)`; waits at most `timeout` for one. True when one
    /// was answered.
    pub fn serve_producer(&self, c: usize, timeout: Duration, handler: impl FnOnce(&[u8]) -> Vec<u8>) -> bool {
        let seen = self.mail_seq().load(Ordering::Acquire);
        if self.mail_state(c).load(Ordering::Acquire) == 3 {
            let len = (self.mail_len(c).load(Ordering::Relaxed) as usize).min(MAILBOX_SIZE);
            let request = self.mailbox(c)[..len].to_vec();
            let mut answer = handler(&request);
            answer.truncate(MAILBOX_SIZE);
            self.mailbox(c)[..answer.len()].copy_from_slice(&answer);
            self.mail_len(c).store(answer.len() as u64, Ordering::Relaxed);
            self.mail_state(c).store(4, Ordering::Release);
            sys::wake(self.mail_state(c));
            return true;
        }
        if !timeout.is_zero() {
            sys::wait(self.mail_seq(), seen, timeout);
        }
        false
    }

    /// Producer: all slots are published (`Done`) or the job failed.
    pub fn finish(&self, state: State) {
        self.word(PRODUCER_STATE).store(state as u64, Ordering::Release);
        self.bump_all();
    }

    /// Smallest read index of the consumers still counting (for "has every
    /// consumer passed slot n?"); the write index when none counts.
    pub fn released_up_to(&self) -> u64 {
        self.min_read_index().unwrap_or_else(|| self.write_index().load(Ordering::Acquire))
    }

    /// Consumer: attaches as consumer `c` with the calling process id.
    pub fn attach(&self, c: usize) -> Result<(), RingError> {
        if c >= self.consumers {
            return Err(RingError::Io(format!("ring {}: no consumer {c}", self.name)));
        }
        self.consumer_pid(c).store(sys::current_pid(), Ordering::Release);
        self.consumer_start(c).store(unix_now(), Ordering::Release);
        self.set_consumer_state(c, State::Running);
        Ok(())
    }

    /// Whether the producer failed or its process is gone.
    fn producer_gone(&self) -> bool {
        self.producer_state() == State::Failed || !sys::pid_alive(self.word(PRODUCER_PID).load(Ordering::Relaxed))
    }

    /// Consumer `c`: the next slot, `None` after the producer finished and
    /// every slot was read.
    pub fn next(&self, c: usize, timeout: Duration) -> Result<Option<SlotRef<'_>>, RingError> {
        let deadline = Instant::now() + timeout;
        loop {
            self.beat(c);
            let seen = self.write_seq().load(Ordering::Acquire);
            let r = self.read_index(c).load(Ordering::Relaxed);
            let w = self.write_index().load(Ordering::Acquire);
            if r < w {
                let p = self.slot_ptr(r % self.slots);
                // SAFETY: published slot, not freed before this consumer
                // releases it.
                let (len, meta) = unsafe {
                    let words = p as *const u64;
                    let len = (*words) as usize;
                    let mut meta = [0u64; SLOT_META];
                    for (i, m) in meta.iter_mut().enumerate() {
                        *m = *words.add(1 + i);
                    }
                    (len.min(self.slot_size), meta)
                };
                // SAFETY: as above.
                let data = unsafe { std::slice::from_raw_parts(self.payload_ptr(r % self.slots), len) };
                return Ok(Some(SlotRef { seq: r, data, meta }));
            }
            match self.producer_state() {
                State::Done => return Ok(None),
                _ if self.producer_gone() => return Err(RingError::ProducerGone),
                _ => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(RingError::Timeout);
            }
            sys::wait(self.write_seq(), seen, POLL.min(deadline - now));
        }
    }

    /// Consumer `c`: the slot from [`Ring::next`] is done.
    pub fn release(&self, c: usize) {
        self.beat(c);
        self.read_index(c).fetch_add(1, Ordering::AcqRel);
        self.read_seq().fetch_add(1, Ordering::AcqRel);
        sys::wake(self.read_seq());
    }

    /// Consumer `c`: sends `request` to the producer and waits for the
    /// answer.
    pub fn call(&self, c: usize, request: &[u8], timeout: Duration) -> Result<Vec<u8>, RingError> {
        if request.len() > MAILBOX_SIZE {
            return Err(RingError::TooLarge(request.len()));
        }
        self.mailbox(c)[..request.len()].copy_from_slice(request);
        self.mail_len(c).store(request.len() as u64, Ordering::Relaxed);
        self.mail_state(c).store(1, Ordering::Release);
        self.mail_seq().fetch_add(1, Ordering::AcqRel);
        sys::wake(self.mail_seq());
        let deadline = Instant::now() + timeout;
        loop {
            self.beat(c);
            if self.mail_state(c).load(Ordering::Acquire) == 2 {
                let len = (self.mail_len(c).load(Ordering::Relaxed) as usize).min(MAILBOX_SIZE);
                let answer = self.mailbox(c)[..len].to_vec();
                self.mail_state(c).store(0, Ordering::Release);
                return Ok(answer);
            }
            if self.producer_gone() {
                return Err(RingError::ProducerGone);
            }
            let now = Instant::now();
            if now >= deadline {
                self.mail_state(c).store(0, Ordering::Release);
                return Err(RingError::Timeout);
            }
            sys::wait(self.mail_state(c), 1, POLL.min(deadline - now));
        }
    }

    /// Producer: answers every pending request with `handler(consumer,
    /// request)`; waits at most `timeout` for one. Returns how many were
    /// answered.
    pub fn serve_mail(&self, timeout: Duration, mut handler: impl FnMut(usize, &[u8]) -> Vec<u8>) -> usize {
        let seen = self.mail_seq().load(Ordering::Acquire);
        let mut served = 0;
        for c in 0..self.consumers {
            if self.mail_state(c).load(Ordering::Acquire) == 1 {
                let len = (self.mail_len(c).load(Ordering::Relaxed) as usize).min(MAILBOX_SIZE);
                let request = self.mailbox(c)[..len].to_vec();
                let mut answer = handler(c, &request);
                answer.truncate(MAILBOX_SIZE);
                self.mailbox(c)[..answer.len()].copy_from_slice(&answer);
                self.mail_len(c).store(answer.len() as u64, Ordering::Relaxed);
                self.mail_state(c).store(2, Ordering::Release);
                sys::wake(self.mail_state(c));
                served += 1;
            }
        }
        if served == 0 && !timeout.is_zero() {
            sys::wait(self.mail_seq(), seen, timeout);
        }
        served
    }
}

/// Seconds since 1970.
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl SlotMut<'_> {
    /// Sequence number this slot gets.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Publishes the first `len` payload bytes with the meta words.
    pub fn publish(self, len: usize) {
        let p = self.ring.slot_ptr(self.seq % self.ring.slots);
        // SAFETY: the claimed slot's header, written before the index moves.
        unsafe {
            let words = p as *mut u64;
            *words = len.min(self.ring.slot_size) as u64;
            for (i, m) in self.meta.iter().enumerate() {
                *words.add(1 + i) = *m;
            }
        }
        self.ring.write_index().store(self.seq + 1, Ordering::Release);
        self.ring.write_seq().fetch_add(1, Ordering::AcqRel);
        sys::wake(self.ring.write_seq());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
