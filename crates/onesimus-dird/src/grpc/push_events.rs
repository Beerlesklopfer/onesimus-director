//! Convert BusMessage to gRPC PushEvent.

use onesimus_proto_rs::onesimus::director::{
    PushEvent, push_event,
    ConsoleTextEvent, FdConnectedEvent, FdDisconnectedEvent, FdStatusEvent,
    SdConnectedEvent, SdDisconnectedEvent, SdStatusEvent, JobStatusEvent,
};

use crate::message_bus::BusMessage;

/// Convert a BusMessage to a gRPC PushEvent.
pub fn to_push_event(msg: &BusMessage) -> PushEvent {
    let event = match msg {
        BusMessage::ConsoleText { text } => {
            push_event::Event::ConsoleText(ConsoleTextEvent {
                text: text.clone(),
            })
        }
        BusMessage::FdConnected { client_name, protocol_version, connect_time } => {
            push_event::Event::FdConnected(FdConnectedEvent {
                client_name: client_name.clone(),
                protocol_version: *protocol_version,
                connect_time: *connect_time,
            })
        }
        BusMessage::FdDisconnected { client_name } => {
            push_event::Event::FdDisconnected(FdDisconnectedEvent {
                client_name: client_name.clone(),
            })
        }
        BusMessage::FdStatus { client_name, alive, check_time } => {
            push_event::Event::FdStatus(FdStatusEvent {
                client_name: client_name.clone(),
                alive: *alive,
                check_time: *check_time,
            })
        }
        BusMessage::SdConnected { job_id, storage_name, connect_time } => {
            push_event::Event::SdConnected(SdConnectedEvent {
                job_id: *job_id,
                storage_name: storage_name.clone(),
                connect_time: *connect_time,
            })
        }
        BusMessage::SdDisconnected { job_id, storage_name } => {
            push_event::Event::SdDisconnected(SdDisconnectedEvent {
                job_id: *job_id,
                storage_name: storage_name.clone(),
            })
        }
        BusMessage::SdStatus { job_id, storage_name, alive, check_time } => {
            push_event::Event::SdStatus(SdStatusEvent {
                job_id: *job_id,
                storage_name: storage_name.clone(),
                alive: *alive,
                check_time: *check_time,
            })
        }
        BusMessage::JobStatus { job_id, job_name, status, timestamp } => {
            push_event::Event::JobStatus(JobStatusEvent {
                job_id: *job_id,
                job_name: job_name.clone(),
                status: status.clone(),
                timestamp: *timestamp,
            })
        }
        BusMessage::JobMessage { job_id, job_name, text, timestamp, .. } => {
            // JobMessage als ConsoleText ueber gRPC senden
            push_event::Event::ConsoleText(ConsoleTextEvent {
                text: format!("JobId={job_id} {job_name}: {text}"),
            })
        }
    };

    PushEvent { event: Some(event) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_text_conversion() {
        let msg = BusMessage::ConsoleText { text: "hello".into() };
        let event = to_push_event(&msg);
        match event.event.unwrap() {
            push_event::Event::ConsoleText(e) => assert_eq!(e.text, "hello"),
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn fd_connected_conversion() {
        let msg = BusMessage::FdConnected {
            client_name: "fd1".into(),
            protocol_version: 54,
            connect_time: 1000,
        };
        let event = to_push_event(&msg);
        match event.event.unwrap() {
            push_event::Event::FdConnected(e) => {
                assert_eq!(e.client_name, "fd1");
                assert_eq!(e.protocol_version, 54);
                assert_eq!(e.connect_time, 1000);
            }
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn job_status_conversion() {
        let msg = BusMessage::JobStatus {
            job_id: 42,
            job_name: "Backup".into(),
            status: "Running".into(),
            timestamp: 2000,
        };
        let event = to_push_event(&msg);
        match event.event.unwrap() {
            push_event::Event::JobStatus(e) => {
                assert_eq!(e.job_id, 42);
                assert_eq!(e.status, "Running");
            }
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn all_message_types_convert() {
        let messages = vec![
            BusMessage::ConsoleText { text: "t".into() },
            BusMessage::FdConnected { client_name: "f".into(), protocol_version: 1, connect_time: 0 },
            BusMessage::FdDisconnected { client_name: "f".into() },
            BusMessage::FdStatus { client_name: "f".into(), alive: true, check_time: 0 },
            BusMessage::SdConnected { job_id: 1, storage_name: "s".into(), connect_time: 0 },
            BusMessage::SdDisconnected { job_id: 1, storage_name: "s".into() },
            BusMessage::SdStatus { job_id: 1, storage_name: "s".into(), alive: true, check_time: 0 },
            BusMessage::JobStatus { job_id: 1, job_name: "j".into(), status: "ok".into(), timestamp: 0 },
        ];

        for msg in &messages {
            let event = to_push_event(msg);
            assert!(event.event.is_some(), "event should not be None for {msg:?}");
        }
    }
}
