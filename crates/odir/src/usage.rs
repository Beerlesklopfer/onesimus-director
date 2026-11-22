//! Usage of the month for the free tier display (docs/plans/FREE-TIER-LIZENZ.md).
//!
//! The source is the catalog function `onesimus.fn_usage_month()` (schema
//! step v2255): bytes of backup jobs without VirtualFull started in the
//! current month. A watcher thread reads it every [`POLL`], keeps the last
//! value ([`current`]) and posts a [`BusMessage::Usage`] whenever it changes;
//! a gRPC session gets the current value right after it starts. `status dir`
//! over the typed gRPC API adds the bytes of the running backup jobs.
//!
//! Display only: nothing is limited yet (limit 0, licence FREE) until the
//! maintainer decides L-1..L-13 of the plan.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::catalog_worker::CatalogHandle;
use crate::message_bus::{BusMessage, MessageBus};

/// How often the watcher reads the catalog.
pub const POLL: Duration = Duration::from_secs(30);

/// Share of the limit from which the state is NEAR.
const NEAR_PERCENT: u64 = 80;

/// The usage of one month.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    /// `YYYY-MM` in the Director's time zone.
    pub month: String,
    /// Bytes of the finished backup jobs of the month (catalog).
    pub bytes_written: u64,
    /// Number of counted jobs.
    pub jobs: u64,
    /// Scheduled jobs held by the limit (0 until the free tier holds jobs).
    pub held_jobs: u32,
    /// Limit per month, 0 = unlimited.
    pub limit_bytes: u64,
    /// Bytes the running counted jobs have written so far.
    pub bytes_running: u64,
    /// Unix time of the next month start (Director's time zone).
    pub resets_at: i64,
}

/// The state names of the proto enum `UsageState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Below 80 % or no limit.
    Ok,
    /// At least 80 %.
    Near,
    /// At least 100 %.
    Reached,
}

impl State {
    /// The value name in JSON: the proto enum name without its prefix
    /// (`USAGE_STATE_`), as the typed codec resolves it.
    pub fn json_name(self) -> &'static str {
        match self {
            State::Ok => "OK",
            State::Near => "NEAR",
            State::Reached => "REACHED",
        }
    }
}

impl Usage {
    /// OK/NEAR/REACHED from written plus running bytes.
    pub fn state(&self) -> State {
        if self.limit_bytes == 0 {
            return State::Ok;
        }
        let used = self.bytes_written.saturating_add(self.bytes_running);
        if used >= self.limit_bytes {
            State::Reached
        } else if used.saturating_mul(100) >= self.limit_bytes.saturating_mul(NEAR_PERCENT) {
            State::Near
        } else {
            State::Ok
        }
    }

    /// JSON with the field names of the proto `UsageInfo` (typed API codec).
    pub fn to_json(&self) -> Value {
        json!({
            "month": self.month,
            "bytes_written": self.bytes_written,
            "limit_bytes": self.limit_bytes,
            "state": self.state().json_name(),
            "resets_at": self.resets_at,
            "licence": { "kind": "FREE" },
            "bytes_running": self.bytes_running,
            "held_jobs_this_month": self.held_jobs,
        })
    }

    /// The proto message.
    pub fn to_proto(&self) -> onesimus_proto_rs::onesimus::director::usage::UsageInfo {
        use onesimus_proto_rs::onesimus::director::usage::{LicenceInfo, LicenceKind, UsageInfo, UsageState};
        let state = match self.state() {
            State::Ok => UsageState::Ok,
            State::Near => UsageState::Near,
            State::Reached => UsageState::Reached,
        };
        UsageInfo {
            month: self.month.clone(),
            bytes_written: self.bytes_written,
            limit_bytes: self.limit_bytes,
            state: state as i32,
            resets_at: self.resets_at,
            licence: Some(LicenceInfo { kind: LicenceKind::Free as i32, ..Default::default() }),
            bytes_running: self.bytes_running,
            held_jobs_this_month: self.held_jobs,
        }
    }
}

/// Unix time of the first day of the month after `year`/`month` at 00:00 in
/// the local time zone.
fn next_month_start(year: i32, month: u32) -> i64 {
    let (y, m) = if month >= 12 { (year + 1, 1) } else { (year, month + 1) };
    crate::localtime::local_time(y, m as i32, 1, 0)
}

/// The usage of the current month from `catalog` (None: no catalog, or the
/// function is missing, e.g. before schema step v2255).
pub fn query(catalog: &CatalogHandle) -> Option<Usage> {
    let table = catalog
        .query_rows("SELECT month::text, bytes_written::text, jobs::text, held_jobs::text FROM onesimus.fn_usage_month()")
        .ok()?;
    let row = table.rows.first()?;
    let cell = |i: usize| row.get(i).cloned().flatten().unwrap_or_default();
    let month_date = cell(0); // YYYY-MM-DD
    let year: i32 = month_date.get(0..4)?.parse().ok()?;
    let month: u32 = month_date.get(5..7)?.parse().ok()?;
    Some(Usage {
        month: format!("{year:04}-{month:02}"),
        bytes_written: cell(1).split('.').next().unwrap_or("0").parse().unwrap_or(0),
        jobs: cell(2).parse().unwrap_or(0),
        held_jobs: cell(3).parse().unwrap_or(0),
        limit_bytes: 0,
        bytes_running: 0,
        resets_at: next_month_start(year, month),
    })
}

/// The last value the watcher read.
static CURRENT: Mutex<Option<Usage>> = Mutex::new(None);

/// The last value the watcher read (None before the first read).
pub fn current() -> Option<Usage> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Stores `usage`; true when it differs from the previous value.
fn store(usage: Usage) -> bool {
    let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if current.as_ref() == Some(&usage) {
        return false;
    }
    *current = Some(usage);
    true
}

/// Starts the watcher: reads the catalog now and every [`POLL`], posts a
/// [`BusMessage::Usage`] on every change (month, bytes, held jobs).
pub fn spawn_watcher(catalog: CatalogHandle, bus: Arc<MessageBus>) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new().name("usage".into()).spawn(move || loop {
        if let Some(usage) = query(&catalog)
            && store(usage.clone())
        {
            bus.post(BusMessage::Usage(usage));
        }
        std::thread::sleep(POLL);
    })
}

/// Bytes of the running backup jobs without VirtualFull, from the `running`
/// rows of `status dir` in JSON mode (`type`, `level`, `bytes`).
pub fn running_bytes(running: Option<&Value>) -> u64 {
    running.and_then(Value::as_array).map(|rows| {
        rows.iter()
            .filter(|r| r["type"] == "Backup" && !r["level"].as_str().unwrap_or("").starts_with("Virtual"))
            .filter_map(|r| r["bytes"].as_u64())
            .sum()
    }).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_follows_written_plus_running() {
        let mut u = Usage { limit_bytes: 1_000, ..Default::default() };
        assert_eq!(u.state(), State::Ok);
        u.bytes_written = 700;
        u.bytes_running = 100;
        assert_eq!(u.state(), State::Near);
        u.bytes_running = 300;
        assert_eq!(u.state(), State::Reached);
        u.limit_bytes = 0;
        assert_eq!(u.state(), State::Ok, "no limit");
    }

    #[test]
    fn json_and_proto_carry_the_agreed_fields() {
        let u = Usage { month: "2026-11".into(), bytes_written: 5, jobs: 2, held_jobs: 0, limit_bytes: 0, bytes_running: 3, resets_at: 7 };
        let j = u.to_json();
        assert_eq!(j["month"], "2026-11");
        assert_eq!(j["state"], "OK");
        assert_eq!(j["licence"]["kind"], "FREE");
        let p = u.to_proto();
        assert_eq!((p.bytes_written, p.bytes_running, p.resets_at), (5, 3, 7));
        assert_eq!(p.licence.unwrap().kind, 1);
    }

    /// The JSON of `status dir` (typed path) becomes GetDirectorStatus.usage.
    #[test]
    fn status_json_maps_to_the_typed_usage_field() {
        use onesimus_proto_rs::typed;
        use prost::Message;
        let u = Usage { month: "2026-11".into(), bytes_written: 412, jobs: 1, held_jobs: 2, limit_bytes: 1_000, bytes_running: 400, resets_at: 99 };
        let (_, output) = typed::registry().rpc_types(typed::DIRECTOR_API_SERVICE, "GetDirectorStatus").unwrap();
        let encoded = typed::json_to_message(&output, &json!({ "usage": u.to_json() }));
        assert!(encoded.unmapped.is_empty(), "{:?}", encoded.unmapped);
        let response = onesimus_proto_rs::onesimus::director::api::DirectorStatusResponse::decode(encoded.bytes.as_slice()).unwrap();
        assert_eq!(response.usage, Some(u.to_proto()));
        assert_eq!(response.usage.unwrap().state, 2, "412 + 400 of 1000 = 81 %: NEAR");
    }

    #[test]
    fn running_bytes_count_backups_without_virtual_full() {
        let rows = json!([
            {"type": "Backup", "level": "Full", "bytes": 10},
            {"type": "Backup", "level": "VirtualFull", "bytes": 100},
            {"type": "Restore", "level": "", "bytes": 1000},
            {"type": "Backup", "level": "Incremental", "bytes": 5},
        ]);
        assert_eq!(running_bytes(Some(&rows)), 15);
        assert_eq!(running_bytes(None), 0);
    }

    #[test]
    fn next_month_wraps_the_year() {
        let dec = next_month_start(2026, 12);
        let jan = crate::localtime::local_tm(dec);
        assert_eq!((jan.tm_year + 1900, jan.tm_mon + 1, jan.tm_mday), (2027, 1, 1));
    }
}
