// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Tests of the ring: threads, a second process, failure and cleanup.

use super::*;
use std::process::Command;
use std::sync::Arc;

/// A ring name unique to this test process.
fn name(what: &str) -> String {
    format!("osd-ringtest-{}-{what}", std::process::id())
}

/// Removes the ring when the test ends.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = Ring::remove(&self.0);
    }
}

/// Payload of slot `seq`: its number, repeated.
fn pattern(seq: u64, len: usize) -> Vec<u8> {
    (0..len).map(|i| (seq as usize * 7 + i) as u8).collect()
}

/// Produces `count` slots of varying length; returns when all are published.
fn produce(ring: &Ring, count: u64) {
    for seq in 0..count {
        let mut slot = ring.claim(Duration::from_secs(20)).expect("claim");
        assert_eq!(slot.seq(), seq);
        let len = 1 + (seq as usize * 13) % ring.slot_size();
        slot.data[..len].copy_from_slice(&pattern(seq, len));
        slot.meta[0] = seq * 10;
        slot.publish(len);
    }
    ring.finish(State::Done);
}

/// Reads every slot as consumer `c` and checks it; returns the count.
fn consume(ring: &Ring, c: usize) -> u64 {
    ring.attach(c).unwrap();
    let mut n = 0;
    while let Some(slot) = ring.next(c, Duration::from_secs(20)).expect("next") {
        assert_eq!(slot.seq, n);
        assert_eq!(slot.meta[0], n * 10);
        assert_eq!(slot.data, &pattern(n, slot.data.len())[..]);
        ring.release(c);
        n += 1;
    }
    ring.set_consumer_state(c, State::Done);
    n
}

#[test]
fn blocks_pass_through_a_small_ring_in_order() {
    let n = name("order");
    let _c = Cleanup(n.clone());
    let ring = Arc::new(Ring::create(&n, RingConfig { slots: 3, slot_size: 100, consumers: 1 }).unwrap());
    let reader = {
        let ring = Arc::new(Ring::open(&n).unwrap());
        std::thread::spawn(move || consume(&ring, 0))
    };
    produce(&ring, 200);
    assert_eq!(reader.join().unwrap(), 200);
}

#[test]
fn a_full_ring_blocks_the_producer_until_release() {
    let n = name("full");
    let _c = Cleanup(n.clone());
    let ring = Ring::create(&n, RingConfig { slots: 2, slot_size: 8, consumers: 1 }).unwrap();
    for _ in 0..2 {
        ring.claim(Duration::from_millis(10)).unwrap().publish(1);
    }
    assert_eq!(ring.claim(Duration::from_millis(50)).unwrap_err(), RingError::Timeout);
    let reader = Ring::open(&n).unwrap();
    reader.attach(0).unwrap();
    assert_eq!(reader.next(0, Duration::from_millis(10)).unwrap().unwrap().seq, 0);
    reader.release(0);
    assert_eq!(ring.claim(Duration::from_millis(50)).unwrap().seq(), 2);
}

#[test]
fn a_failed_consumer_no_longer_holds_slots() {
    let n = name("failed");
    let _c = Cleanup(n.clone());
    let ring = Ring::create(&n, RingConfig { slots: 1, slot_size: 8, consumers: 2 }).unwrap();
    ring.attach(0).unwrap();
    ring.attach(1).unwrap();
    ring.claim(Duration::from_millis(10)).unwrap().publish(1);
    assert!(ring.next(0, Duration::from_millis(10)).unwrap().is_some());
    ring.release(0);
    // consumer 1 still holds slot 0
    assert_eq!(ring.claim(Duration::from_millis(30)).unwrap_err(), RingError::Timeout);
    ring.set_consumer_state(1, State::Failed);
    assert_eq!(ring.claim(Duration::from_millis(30)).unwrap().seq(), 1);
    ring.set_consumer_state(0, State::Failed);
    assert_eq!(ring.claim(Duration::from_millis(30)).unwrap_err(), RingError::NoConsumer);
}

#[test]
fn mailbox_carries_requests_and_answers() {
    let n = name("mail");
    let _c = Cleanup(n.clone());
    let ring = Arc::new(Ring::create(&n, RingConfig { slots: 1, slot_size: 8, consumers: 1 }).unwrap());
    ring.set_info(b"JobId=7").unwrap();
    let client = {
        let ring = Arc::new(Ring::open(&n).unwrap());
        std::thread::spawn(move || {
            assert_eq!(ring.info(), b"JobId=7");
            ring.attach(0).unwrap();
            let a = ring.call(0, b"CatReq one", Duration::from_secs(10)).unwrap();
            let b = ring.call(0, b"CatReq two", Duration::from_secs(10)).unwrap();
            (a, b)
        })
    };
    let mut answered = 0;
    while answered < 2 {
        answered += ring.serve_mail(Duration::from_millis(100), |c, req| {
            assert_eq!(c, 0);
            [b"1000 OK ".as_slice(), req].concat()
        });
    }
    let (a, b) = client.join().unwrap();
    assert_eq!((a.as_slice(), b.as_slice()), (b"1000 OK CatReq one".as_slice(), b"1000 OK CatReq two".as_slice()));
    assert_eq!(ring.call(0, &vec![0; MAILBOX_SIZE + 1], Duration::ZERO).unwrap_err(), RingError::TooLarge(MAILBOX_SIZE + 1));
}

/// Environment variable that turns [`child_process`] into a consumer.
const CHILD_ENV: &str = "OSD_RING_TEST_CHILD";

/// Runs this test binary's [`child_process`] for ring `ring` with `mode`.
fn spawn_child(ring: &str, mode: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tests::child_process", "--ignored", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, format!("{ring}:{mode}"))
        .spawn()
        .unwrap()
}

/// The child side of the multi-process tests; does nothing without
/// `OSD_RING_TEST_CHILD`.
#[test]
#[ignore = "child process of the multi-process tests"]
fn child_process() {
    let Ok(spec) = std::env::var(CHILD_ENV) else { return };
    let (ring_name, mode) = spec.split_once(':').unwrap();
    let ring = Ring::open(ring_name).unwrap();
    match mode {
        "consume" => {
            let n = consume(&ring, 0);
            let answer = ring.call(0, format!("read {n}").as_bytes(), Duration::from_secs(20)).unwrap();
            assert_eq!(answer, b"ok");
        }
        "die" => {
            ring.attach(0).unwrap();
            // exits without reading: the producer must notice
        }
        other => panic!("unknown mode {other}"),
    }
}

#[test]
fn a_second_process_consumes_through_shared_memory() {
    let n = name("process");
    let _c = Cleanup(n.clone());
    let ring = Arc::new(Ring::create(&n, RingConfig { slots: 4, slot_size: 4096, consumers: 1 }).unwrap());
    let mut child = spawn_child(&n, "consume");
    let server = {
        let ring = ring.clone();
        std::thread::spawn(move || {
            let mut got = None;
            while got.is_none() {
                ring.serve_mail(Duration::from_millis(100), |_, req| {
                    got = Some(String::from_utf8_lossy(req).into_owned());
                    b"ok".to_vec()
                });
            }
            got.unwrap()
        })
    };
    produce(&ring, 500);
    assert_eq!(server.join().unwrap(), "read 500");
    assert!(child.wait().unwrap().success());
    assert_eq!(ring.consumer_state(0), State::Done);
}

#[test]
fn a_vanished_consumer_process_is_marked_failed() {
    let n = name("vanish");
    let _c = Cleanup(n.clone());
    let ring = Ring::create(&n, RingConfig { slots: 2, slot_size: 16, consumers: 1 }).unwrap();
    let mut child = spawn_child(&n, "die");
    assert!(child.wait().unwrap().success());
    assert_eq!(ring.consumer_state(0), State::Running);
    for _ in 0..2 {
        ring.claim(Duration::from_secs(1)).unwrap().publish(1);
    }
    assert_eq!(ring.claim(Duration::from_secs(5)).unwrap_err(), RingError::NoConsumer);
    assert_eq!(ring.consumer_state(0), State::Failed);
}

#[test]
fn stale_rings_of_dead_producers_are_removed() {
    let prefix = name("stale");
    let live = format!("{prefix}-live");
    let dead = format!("{prefix}-dead");
    let _c = (Cleanup(live.clone()), Cleanup(dead.clone()));
    let _live = Ring::create(&live, RingConfig { slots: 1, slot_size: 8, consumers: 1 }).unwrap();
    let dead_ring = Ring::create(&dead, RingConfig { slots: 1, slot_size: 8, consumers: 1 }).unwrap();
    // A pid above pid_max never exists.
    dead_ring.word(PRODUCER_PID).store(i32::MAX as u64, Ordering::Relaxed);
    assert_eq!(Ring::remove_stale(&prefix), vec![dead.clone()]);
    assert!(Ring::open(&dead).is_err());
    assert!(Ring::open(&live).is_ok());
    assert!(Ring::create(&live, RingConfig { slots: 1, slot_size: 8, consumers: 1 }).is_err());
}

#[test]
fn a_stalled_consumer_is_evicted_after_the_liveness_timeout() {
    let n = name("stall");
    let _c = Cleanup(n.clone());
    let mut ring = Ring::create(&n, RingConfig { slots: 1, slot_size: 8, consumers: 2 }).unwrap();
    ring.set_liveness(Duration::from_millis(300));
    ring.attach(0).unwrap();
    ring.attach(1).unwrap();
    ring.claim(Duration::ZERO).unwrap().publish(1);
    // consumer 0 keeps reading and beating, consumer 1 stalls
    let reader = Ring::open(&n).unwrap();
    assert!(reader.next(0, Duration::ZERO).unwrap().is_some());
    reader.release(0);
    let keep_beating = std::thread::spawn(move || {
        for _ in 0..20 {
            reader.beat(0);
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    assert_eq!(ring.claim(Duration::from_secs(5)).unwrap().seq(), 1);
    assert_eq!(ring.consumer_state(1), State::Failed);
    assert_eq!(ring.consumer_state(0), State::Running);
    keep_beating.join().unwrap();
}

#[test]
fn payloads_are_page_aligned() {
    let n = name("align");
    let _c = Cleanup(n.clone());
    let ring = Ring::create(&n, RingConfig { slots: 3, slot_size: 65536, consumers: 1 }).unwrap();
    for _ in 0..3 {
        let slot = ring.claim(Duration::ZERO).unwrap();
        assert_eq!(slot.data.as_ptr() as usize % 4096, 0);
        slot.publish(0);
    }
}

#[test]
fn bad_names_and_configurations_are_refused() {
    assert!(Ring::create("../x", RingConfig { slots: 1, slot_size: 8, consumers: 1 }).is_err());
    assert!(Ring::create(&name("cfg"), RingConfig { slots: 0, slot_size: 8, consumers: 1 }).is_err());
    assert!(Ring::create(&name("cfg"), RingConfig { slots: 1, slot_size: 8, consumers: 9 }).is_err());
    assert!(Ring::open(&name("missing")).is_err());
}

#[test]
fn the_read_direction_reader_produces_and_asks_the_session() {
    let n = name("reverse");
    let _c = Cleanup(n.clone());
    // the session creates the ring and attaches as consumer 0
    let session = Arc::new(Ring::create(&n, RingConfig { slots: 2, slot_size: 64, consumers: 1 }).unwrap());
    session.attach(0).unwrap();
    let reader = std::thread::spawn({
        let n = n.clone();
        move || {
            let ring = Ring::open(&n).unwrap();
            ring.take_producer();
            let answer = ring.ask_consumer(0, b"CatReq GetVolInfo", Duration::from_secs(10)).unwrap();
            produce(&ring, 20);
            answer
        }
    });
    let mut served = false;
    let mut read = 0u64;
    loop {
        if !served {
            served = session.serve_producer(0, Duration::from_millis(50), |req| [b"1000 OK ".as_slice(), req].concat());
            continue;
        }
        match session.next(0, Duration::from_secs(10)).unwrap() {
            Some(slot) => {
                assert_eq!(slot.data, &pattern(read, slot.data.len())[..]);
                session.release(0);
                read += 1;
            }
            None => break,
        }
    }
    assert_eq!(read, 20);
    assert_eq!(reader.join().unwrap(), b"1000 OK CatReq GetVolInfo");
    // a consumer that is done does not hold slots, a failed one answers no more
    session.set_consumer_state(0, State::Failed);
    assert_eq!(session.ask_consumer(0, b"x", Duration::from_secs(1)).unwrap_err(), RingError::NoConsumer);
}

/// Throughput: `total` bytes in slots of `slot_size` to `consumers` readers,
/// each reader reads every slot (broadcast); prints MiB/s of the producer.
fn throughput(slot_size: usize, slots: u32, consumers: u32, total: usize) -> f64 {
    let n = name(&format!("bench{slot_size}"));
    let _c = Cleanup(n.clone());
    let ring = Arc::new(Ring::create(&n, RingConfig { slots, slot_size: slot_size as u32, consumers }).unwrap());
    let count = (total / slot_size) as u64;
    let readers: Vec<_> = (0..consumers as usize)
        .map(|c| {
            let ring = Arc::new(Ring::open(&n).unwrap());
            std::thread::spawn(move || {
                ring.attach(c).unwrap();
                let mut bytes = 0usize;
                while let Some(slot) = ring.next(c, Duration::from_secs(30)).expect("next") {
                    bytes += slot.data.len();
                    ring.release(c);
                }
                ring.set_consumer_state(c, State::Done);
                bytes
            })
        })
        .collect();
    let start = Instant::now();
    for _ in 0..count {
        let mut slot = ring.claim(Duration::from_secs(30)).expect("claim");
        slot.data[0] = 1;
        slot.publish(slot_size);
    }
    ring.finish(State::Done);
    for r in readers {
        assert_eq!(r.join().unwrap(), count as usize * slot_size);
    }
    total as f64 / (1 << 20) as f64 / start.elapsed().as_secs_f64()
}

/// Round trip with a single slot: the producer can only claim again after the
/// reader woke up, read and released, so one step is wake + wake.
fn round_trip_micros(rounds: u32) -> f64 {
    let n = name("latency");
    let _c = Cleanup(n.clone());
    let ring = Arc::new(Ring::create(&n, RingConfig { slots: 1, slot_size: 4096, consumers: 1 }).unwrap());
    let reader = {
        let ring = Arc::new(Ring::open(&n).unwrap());
        std::thread::spawn(move || {
            ring.attach(0).unwrap();
            while let Some(_slot) = ring.next(0, Duration::from_secs(30)).expect("next") {
                ring.release(0);
            }
        })
    };
    let start = Instant::now();
    for _ in 0..rounds {
        ring.claim(Duration::from_secs(30)).expect("claim").publish(1);
    }
    ring.finish(State::Done);
    reader.join().unwrap();
    start.elapsed().as_secs_f64() * 1e6 / rounds as f64
}

/// Prints the numbers of the ring (`cargo test -p osd-ring --release -- --ignored --nocapture bench`).
#[test]
#[ignore = "measurement, not a check"]
fn bench() {
    for (slot, slots, consumers, total) in [
        (64 << 10, 8, 1, 1 << 30),
        (64 << 10, 8, 3, 1 << 30),
        (64 << 10, 64, 3, 1 << 30),
        (64 << 10, 512, 3, 1 << 30),
        (1 << 20, 8, 3, 1 << 30),
    ] {
        let mibs = throughput(slot, slots, consumers, total);
        println!("BENCH slot {:>4} KiB, {slots} slots, {consumers} reader(s): {mibs:8.0} MiB/s", slot >> 10);
    }
    println!("BENCH one slot per step, producer to reader and back: {:.1} us", round_trip_micros(20_000));
}
