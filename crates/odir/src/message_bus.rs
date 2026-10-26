//! Typed message bus for inter-component communication.
//!
//! Uses crossbeam MPMC bounded channels. Each console session calls
//! subscribe() to get its own Receiver for events.

use crossbeam_channel::{Sender, Receiver, bounded};
use std::sync::Mutex;

/// All message types on the bus.
/// Maps 1:1 to the C++ BusMessage variant in message_bus.h.
#[derive(Debug, Clone)]
pub enum BusMessage {
    ConsoleText { text: String },
    FdConnected { client_name: String, protocol_version: i32, connect_time: i64 },
    FdDisconnected { client_name: String },
    FdStatus { client_name: String, alive: bool, check_time: i64 },
    SdConnected { job_id: u32, storage_name: String, connect_time: i64 },
    SdDisconnected { job_id: u32, storage_name: String },
    SdStatus { job_id: u32, storage_name: String, alive: bool, check_time: i64 },
    JobStatus { job_id: u32, job_name: String, status: String, timestamp: i64 },
    /// Job message — like C++ Jmsg(). Goes to all consoles that subscribed.
    /// msg_type maps to Messages resource types: info, warning, error, fatal, mount, ...
    JobMessage { job_id: u32, job_name: String, msg_type: MsgType, text: String, timestamp: i64 },
}

/// Message severity type (maps to C++ M_INFO, M_WARNING, M_ERROR, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    Info,
    Warning,
    Error,
    Fatal,
    Mount,
    Saved,
    Restored,
    Skipped,
    Audit,
}

/// Fan-out broadcast bus. Each subscriber gets its own channel.
pub struct MessageBus {
    subscribers: Mutex<Vec<Sender<BusMessage>>>,
    capacity: usize,
}

impl MessageBus {
    pub fn new(capacity: usize) -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            capacity,
        }
    }

    /// Post a message to all subscribers (non-blocking, drops on full).
    pub fn post(&self, msg: BusMessage) {
        let subs = self.subscribers.lock().unwrap();
        for tx in subs.iter() {
            let _ = tx.try_send(msg.clone());
        }
    }

    /// Convenience: post a console text message.
    pub fn post_text(&self, text: &str) {
        self.post(BusMessage::ConsoleText { text: text.to_string() });
    }

    /// Create a new subscriber. Returns a Receiver that gets all future messages.
    pub fn subscribe(&self) -> Receiver<BusMessage> {
        let (tx, rx) = bounded(self.capacity);
        let mut subs = self.subscribers.lock().unwrap();
        subs.push(tx);
        rx
    }

    /// Remove dead subscribers (whose Receivers have been dropped).
    pub fn cleanup(&self) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| !tx.is_full() || tx.len() < self.capacity);
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_and_receive() {
        let bus = MessageBus::new(16);
        let rx = bus.subscribe();

        bus.post(BusMessage::ConsoleText { text: "hello".into() });

        match rx.try_recv().unwrap() {
            BusMessage::ConsoleText { text } => assert_eq!(text, "hello"),
            _ => panic!("wrong message type"),
        }
    }

    #[test]
    fn multiple_subscribers() {
        let bus = MessageBus::new(16);
        let rx1 = bus.subscribe();
        let rx2 = bus.subscribe();

        bus.post_text("broadcast");

        match rx1.try_recv().unwrap() {
            BusMessage::ConsoleText { text } => assert_eq!(text, "broadcast"),
            _ => panic!("wrong type"),
        }
        match rx2.try_recv().unwrap() {
            BusMessage::ConsoleText { text } => assert_eq!(text, "broadcast"),
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn multiple_message_types() {
        let bus = MessageBus::new(16);
        let rx = bus.subscribe();

        bus.post(BusMessage::FdConnected {
            client_name: "client1-fd".into(),
            protocol_version: 54,
            connect_time: 1234567890,
        });
        bus.post(BusMessage::SdConnected {
            job_id: 42,
            storage_name: "File1".into(),
            connect_time: 1234567891,
        });
        bus.post_text("done");

        let mut count = 0;
        while rx.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
    }

    #[test]
    fn subscriber_count() {
        let bus = MessageBus::new(16);
        assert_eq!(bus.subscriber_count(), 0);
        let _rx1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);
        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
    }

    #[test]
    fn no_subscribers_no_panic() {
        let bus = MessageBus::new(16);
        // Should not panic even with no subscribers
        bus.post_text("nobody listening");
    }

    #[test]
    fn fd_connected_event() {
        let bus = MessageBus::new(16);
        let rx = bus.subscribe();

        bus.post(BusMessage::FdConnected {
            client_name: "test-fd".into(),
            protocol_version: 54,
            connect_time: 1000,
        });

        match rx.try_recv().unwrap() {
            BusMessage::FdConnected { client_name, protocol_version, connect_time } => {
                assert_eq!(client_name, "test-fd");
                assert_eq!(protocol_version, 54);
                assert_eq!(connect_time, 1000);
            }
            _ => panic!("wrong type"),
        }
    }

    #[test]
    fn sd_events() {
        let bus = MessageBus::new(16);
        let rx = bus.subscribe();

        bus.post(BusMessage::SdConnected {
            job_id: 1,
            storage_name: "Tape".into(),
            connect_time: 2000,
        });
        bus.post(BusMessage::SdDisconnected {
            job_id: 1,
            storage_name: "Tape".into(),
        });

        let msg1 = rx.try_recv().unwrap();
        let msg2 = rx.try_recv().unwrap();
        assert!(matches!(msg1, BusMessage::SdConnected { .. }));
        assert!(matches!(msg2, BusMessage::SdDisconnected { .. }));
    }

    #[test]
    fn job_status_event() {
        let bus = MessageBus::new(16);
        let rx = bus.subscribe();

        bus.post(BusMessage::JobStatus {
            job_id: 42,
            job_name: "BackupJob".into(),
            status: "Running".into(),
            timestamp: 3000,
        });

        match rx.try_recv().unwrap() {
            BusMessage::JobStatus { job_id, job_name, status, .. } => {
                assert_eq!(job_id, 42);
                assert_eq!(job_name, "BackupJob");
                assert_eq!(status, "Running");
            }
            _ => panic!("wrong type"),
        }
    }
}
