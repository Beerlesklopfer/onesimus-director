// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Database record types for the Bareos catalog.

use sqlx::FromRow;

/// New job record (for INSERT).
#[derive(Debug, Clone)]
pub struct NewJobRecord {
    pub job: String,
    pub name: String,
    pub job_type: String,
    pub level: String,
    pub client_id: i32,
    pub job_status: String,
    pub sched_time: i64,
    pub start_time: i64,
    pub end_time: i64,
    pub job_tdate: i64,
    pub pool_id: i32,
    pub fileset_id: i32,
}

/// Job record (from SELECT).
#[derive(Debug, Clone, FromRow)]
pub struct JobRecord {
    pub jobid: i32,
    pub job: String,
    pub name: String,
    pub r#type: String,
    pub level: String,
    pub clientid: i32,
    pub jobstatus: String,
    pub schedtime: Option<String>,
    pub starttime: Option<String>,
    pub endtime: Option<String>,
    pub realendtime: Option<String>,
    pub jobtdate: i64,
    pub volsessionid: i32,
    pub volsessiontime: i32,
    pub jobfiles: i64,
    pub jobbytes: i64,
    pub joberrors: i32,
    pub poolid: i32,
    pub filesetid: i32,
    pub priorjobid: i32,
    pub comment: Option<String>,
}

/// Client record.
#[derive(Debug, Clone, FromRow)]
pub struct ClientRecord {
    pub clientid: i32,
    pub name: String,
    pub uname: Option<String>,
}

/// Pool record.
#[derive(Debug, Clone, FromRow)]
pub struct PoolRecord {
    pub poolid: i32,
    pub name: String,
    pub pooltype: String,
    pub numvols: i32,
    pub maxvols: i32,
}

/// Media/Volume record.
#[derive(Debug, Clone, FromRow)]
pub struct MediaRecord {
    pub mediaid: i32,
    pub volumename: String,
    pub poolid: i32,
    pub mediatype: String,
    pub volstatus: String,
    pub volbytes: i64,
    pub volfiles: i32,
    pub maxvolbytes: i64,
    pub recycle: bool,
}
