//! Convert BusMessage to gRPC PushEvent and decide which console may see it.
//!
//! Push events reach an interactive gRPC session only when the session's
//! console could see the same resource through the console commands
//! ([`visible_to`], finding K-19): job events by the Job ACL like
//! `status dir`, client events by the Client ACL like `status client`,
//! storage events by the Storage ACL like `status storage`. Profiles count
//! exactly as in [`crate::acl::allowed`].

use onesimus_proto_rs::onesimus::director::{
    PushEvent, push_event,
    ConsoleTextEvent, FdConnectedEvent, FdDisconnectedEvent, FdStatusEvent,
    SdConnectedEvent, SdDisconnectedEvent, SdStatusEvent, JobStatusEvent,
};

use crate::acl::{self, Kind};
use crate::director_config::DirectorConfig;
use crate::message_bus::BusMessage;

/// The resource types (configuration type name, ACL) of which a console
/// must see every configured resource to receive an event that names no
/// resource (see [`visible_to`]).
const ALL_RESOURCE_KINDS: [(&str, Kind); 7] = [
    ("job", Kind::Job), ("client", Kind::Client), ("storage", Kind::Storage), ("schedule", Kind::Schedule),
    ("pool", Kind::Pool), ("fileset", Kind::FileSet), ("catalog", Kind::Catalog),
];

/// Whether `console` may see every configured resource of every type in
/// [`ALL_RESOURCE_KINDS`]. Each decision goes through [`acl::allowed`], the
/// one central ACL function (a later inner ACL tier hooks in there), so no
/// Console or Profile list is read here. The default console passes.
fn sees_every_resource(config: &DirectorConfig, console: &str) -> bool {
    if console == "*UserAgent*" {
        return true;
    }
    // An unknown console must not pass because the configuration is empty.
    if !config.raw.iter().any(|r| r.norm_type() == "console" && r.name() == Some(console)) {
        return false;
    }
    ALL_RESOURCE_KINDS.iter().all(|(kind_name, kind)| config.raw.iter()
        .filter(|r| r.norm_type() == *kind_name)
        .filter_map(|r| r.name())
        .all(|name| acl::allowed(config, console, *kind, name)))
}

/// Whether the push event made from `msg` may go to a session of `console`.
///
/// - `JobStatus` and `JobMessage`: the Job ACL must allow the job name, as
///   `status dir` lists only running jobs the console may see
///   (commands/status.rs). A job message is delivered only for such jobs.
/// - `FdConnected`, `FdDisconnected`, `FdStatus`: the Client ACL must
///   allow the client.
/// - `SdConnected`, `SdDisconnected`, `SdStatus`: the Storage ACL must
///   allow the storage.
/// - `ConsoleText` and a `JobMessage` of JobId 0 (the console JCR: a
///   Director message without a job) name no resource and may mention any.
///   Bareos hands such messages to every console that reads the message
///   queue; here they go only to a console that may see every configured
///   job, client, storage, schedule, pool, fileset and catalog
///   ([`sees_every_resource`]), which includes the default console. A
///   restricted console gets them through the `messages` command as in
///   Bareos.
///
/// Every ACL decision is made by [`acl::allowed`] (Console and its
/// profiles today, plus any inner tier added there later). Names that are
/// no valid resource name (e.g. empty) are never allowed. An unknown
/// console sees nothing.
pub fn visible_to(config: &DirectorConfig, console: &str, msg: &BusMessage) -> bool {
    let sees_everything = || sees_every_resource(config, console);
    match msg {
        BusMessage::ConsoleText { .. } => sees_everything(),
        BusMessage::JobMessage { job_id: 0, .. } => sees_everything(),
        BusMessage::JobStatus { job_name, .. } | BusMessage::JobMessage { job_name, .. } => {
            acl::allowed(config, console, Kind::Job, job_name)
        }
        BusMessage::FdConnected { client_name, .. }
        | BusMessage::FdDisconnected { client_name }
        | BusMessage::FdStatus { client_name, .. } => acl::allowed(config, console, Kind::Client, client_name),
        BusMessage::SdConnected { storage_name, .. }
        | BusMessage::SdDisconnected { storage_name, .. }
        | BusMessage::SdStatus { storage_name, .. } => acl::allowed(config, console, Kind::Storage, storage_name),
    }
}

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
        BusMessage::JobMessage { job_id, job_name, text, .. } => {
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
    use crate::message_bus::MsgType;

    /// Consoles covering each resource kind, a profile, negation and the
    /// "sees everything" case.
    fn acl_config() -> DirectorConfig {
        DirectorConfig::from_str(r#"
Director { Name=dir; Password=secret }
Pool { Name=Secret; Pool Type=Backup }
Pool { Name=Full; Pool Type=Backup }
Profile { Name=webui-limited; JobACL=ProfJob; ClientACL=prof-fd; StorageACL=ProfSD }
Profile { Name=everything; JobACL=*all*; ClientACL=*all*; StorageACL=*all*; ScheduleACL=*all*; PoolACL=*all*; FileSetACL=*all*; CatalogACL=*all* }
Console { Name=jobs; Password=pw; JobACL=!BackupSecret,Backup.*; ClientACL=fd1; StorageACL=File }
Console { Name=viaprofile; Password=pw; Profile=webui-limited }
Console { Name=admin; Password=pw; Profile=everything }
Console { Name=almost; Password=pw; JobACL=*all*; ClientACL=*all*; StorageACL=*all*; ScheduleACL=*all*; PoolACL=!Secret,*all*; FileSetACL=*all*; CatalogACL=*all* }
Console { Name=nothing; Password=pw }
"#).unwrap()
    }

    /// A job status event of job `name`.
    fn job(name: &str) -> BusMessage {
        BusMessage::JobStatus { job_id: 7, job_name: name.into(), status: "R".into(), timestamp: 0 }
    }

    /// A job message of job `name` with JobId `id`.
    fn job_msg(id: u32, name: &str) -> BusMessage {
        BusMessage::JobMessage { job_id: id, job_name: name.into(), msg_type: MsgType::Info, text: "t".into(), timestamp: 0 }
    }

    /// The three client events of `name`.
    fn fd(name: &str) -> [BusMessage; 3] {
        [
            BusMessage::FdConnected { client_name: name.into(), protocol_version: 1, connect_time: 0 },
            BusMessage::FdDisconnected { client_name: name.into() },
            BusMessage::FdStatus { client_name: name.into(), alive: true, check_time: 0 },
        ]
    }

    /// The three storage events of `name`.
    fn sd(name: &str) -> [BusMessage; 3] {
        [
            BusMessage::SdConnected { job_id: 1, storage_name: name.into(), connect_time: 0 },
            BusMessage::SdDisconnected { job_id: 1, storage_name: name.into() },
            BusMessage::SdStatus { job_id: 1, storage_name: name.into(), alive: true, check_time: 0 },
        ]
    }

    /// Job status and job messages follow the Job ACL (first match, regex, negation).
    #[test]
    fn job_events_follow_job_acl() {
        let cfg = acl_config();
        assert!(visible_to(&cfg, "jobs", &job("BackupHome")));
        assert!(visible_to(&cfg, "jobs", &job_msg(7, "BackupHome")));
        assert!(!visible_to(&cfg, "jobs", &job("BackupSecret")));
        assert!(!visible_to(&cfg, "jobs", &job_msg(7, "BackupSecret")));
        assert!(!visible_to(&cfg, "jobs", &job("RestoreFiles")));
        assert!(!visible_to(&cfg, "jobs", &job("")));
        assert!(!visible_to(&cfg, "nothing", &job("BackupHome")));
    }

    /// All three client events follow the Client ACL.
    #[test]
    fn client_events_follow_client_acl() {
        let cfg = acl_config();
        for event in fd("fd1") {
            assert!(visible_to(&cfg, "jobs", &event), "{event:?}");
        }
        for event in fd("fd2") {
            assert!(!visible_to(&cfg, "jobs", &event), "{event:?}");
            assert!(!visible_to(&cfg, "nothing", &event), "{event:?}");
        }
    }

    /// All three storage events follow the Storage ACL.
    #[test]
    fn storage_events_follow_storage_acl() {
        let cfg = acl_config();
        for event in sd("File") {
            assert!(visible_to(&cfg, "jobs", &event), "{event:?}");
        }
        for event in sd("Tape") {
            assert!(!visible_to(&cfg, "jobs", &event), "{event:?}");
        }
    }

    /// The inner ACL tier (catalog ACL) restricts push events like every
    /// other ACL check: a catalog negation hides job, client and storage
    /// events and the events naming no resource; an unreadable catalog
    /// (F-4) hides everything from a console with catalog entries.
    #[test]
    fn catalog_acl_restricts_push_events() {
        use crate::catalog_acl::LoadState;
        let cfg = acl_config();
        assert!(visible_to(&cfg, "admin", &job("BackupHome")));
        assert!(visible_to(&cfg, "admin", &BusMessage::ConsoleText { text: "x".into() }));
        cfg.catalog_acl.set_for_test(LoadState::Loaded, &[
            ("admin", Kind::Job, &["!BackupHome", "*all*"]),
            ("admin", Kind::Client, &["fd1"]),
            ("jobs", Kind::Storage, &["!File", "*all*"]),
        ]);
        assert!(!visible_to(&cfg, "admin", &job("BackupHome")));
        assert!(!visible_to(&cfg, "admin", &job_msg(7, "BackupHome")));
        assert!(visible_to(&cfg, "admin", &job("BackupOther")));
        for event in fd("fd1") {
            assert!(visible_to(&cfg, "admin", &event), "{event:?}");
        }
        for event in fd("fd2") {
            assert!(!visible_to(&cfg, "admin", &event), "{event:?}");
        }
        for event in sd("File") {
            assert!(!visible_to(&cfg, "jobs", &event), "{event:?}");
        }
        // Inner tier cannot widen: the file allows no Tape for jobs.
        for event in sd("Tape") {
            assert!(!visible_to(&cfg, "jobs", &event), "{event:?}");
        }
        // Events naming no resource need every configured resource: a catalog
        // list that denies the configured pool Secret hides them.
        cfg.catalog_acl.set_for_test(LoadState::Loaded, &[("admin", Kind::Pool, &["Full"])]);
        assert!(!visible_to(&cfg, "admin", &BusMessage::ConsoleText { text: "x".into() }));
        assert!(!visible_to(&cfg, "admin", &job_msg(0, "")));
        // Catalog unreadable: admin (has entries) sees nothing, almost (none) keeps the file ACL.
        cfg.catalog_acl.set_for_test(LoadState::Failed("down".into()), &[("admin", Kind::Pool, &["Full"])]);
        assert!(!visible_to(&cfg, "admin", &job("BackupOther")));
        for event in fd("fd1") {
            assert!(!visible_to(&cfg, "admin", &event), "{event:?}");
        }
        assert!(visible_to(&cfg, "almost", &job("BackupOther")));
    }

    /// A Console Profile grants and limits events like acl::allowed does.
    #[test]
    fn profiles_grant_like_console_commands() {
        let cfg = acl_config();
        assert!(visible_to(&cfg, "viaprofile", &job("ProfJob")));
        assert!(!visible_to(&cfg, "viaprofile", &job("BackupHome")));
        assert!(fd("prof-fd").iter().all(|e| visible_to(&cfg, "viaprofile", e)));
        assert!(!fd("fd1").iter().any(|e| visible_to(&cfg, "viaprofile", e)));
        assert!(sd("ProfSD").iter().all(|e| visible_to(&cfg, "viaprofile", e)));
        assert!(!sd("File").iter().any(|e| visible_to(&cfg, "viaprofile", e)));
        assert!(visible_to(&cfg, "admin", &job("Anything")));
    }

    /// Daemon text and JobId 0 messages reach only consoles without any resource restriction.
    #[test]
    fn messages_without_resource_only_for_unrestricted_consoles() {
        let cfg = acl_config();
        let text = BusMessage::ConsoleText { text: "daemon".into() };
        let console_jcr = job_msg(0, "*Console*");
        for msg in [&text, &console_jcr] {
            assert!(visible_to(&cfg, "*UserAgent*", msg));
            assert!(visible_to(&cfg, "admin", msg), "profile with *all* everywhere");
            assert!(!visible_to(&cfg, "almost", msg), "one hidden configured pool is a restriction");
            assert!(!visible_to(&cfg, "jobs", msg));
            assert!(!visible_to(&cfg, "viaprofile", msg));
            assert!(!visible_to(&cfg, "nothing", msg));
            assert!(!visible_to(&cfg, "unknown", msg));
        }
    }

    /// The default console sees every event that names a resource.
    #[test]
    fn default_console_sees_every_named_event() {
        let cfg = acl_config();
        assert!(visible_to(&cfg, "*UserAgent*", &job("X")));
        assert!(fd("x").iter().chain(sd("y").iter()).all(|e| visible_to(&cfg, "*UserAgent*", e)));
    }

    /// ConsoleText converts to a ConsoleText push event.
    #[test]
    fn console_text_conversion() {
        let msg = BusMessage::ConsoleText { text: "hello".into() };
        let event = to_push_event(&msg);
        match event.event.unwrap() {
            push_event::Event::ConsoleText(e) => assert_eq!(e.text, "hello"),
            _ => panic!("wrong event type"),
        }
    }

    /// FdConnected keeps all its fields.
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

    /// JobStatus keeps job id and status.
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

    /// Every bus message converts to some push event.
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
