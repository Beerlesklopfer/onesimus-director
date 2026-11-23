// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! SQL query constants for catalog operations.
//!
//! These mirror the queries in the C++ cats/dml/ directory.

/// List all jobs with optional filters.
pub const LIST_JOBS: &str = "\
SELECT j.JobId, j.Job, j.Name, j.Type, j.Level, j.ClientId, c.Name as Client, \
       j.JobStatus, j.SchedTime, j.StartTime, j.EndTime, j.JobFiles, j.JobBytes, j.JobErrors \
FROM Job j LEFT JOIN Client c ON j.ClientId = c.ClientId \
ORDER BY j.JobId DESC LIMIT $1";

/// List jobs for a specific client.
pub const LIST_JOBS_BY_CLIENT: &str = "\
SELECT j.JobId, j.Job, j.Name, j.Type, j.Level, j.JobStatus, \
       j.SchedTime, j.StartTime, j.EndTime, j.JobFiles, j.JobBytes, j.JobErrors \
FROM Job j \
WHERE j.ClientId = $1 \
ORDER BY j.JobId DESC LIMIT $2";

/// List all clients.
pub const LIST_CLIENTS: &str = "\
SELECT ClientId, Name, Uname FROM Client ORDER BY Name";

/// List all pools.
pub const LIST_POOLS: &str = "\
SELECT PoolId, Name, PoolType, NumVols, MaxVols FROM Pool ORDER BY Name";

/// List volumes in a pool.
pub const LIST_VOLUMES_BY_POOL: &str = "\
SELECT MediaId, VolumeName, PoolId, MediaType, VolStatus, VolBytes, VolFiles, MaxVolBytes, Recycle \
FROM Media WHERE PoolId = $1 ORDER BY VolumeName";

/// List all volumes.
pub const LIST_VOLUMES: &str = "\
SELECT MediaId, VolumeName, PoolId, MediaType, VolStatus, VolBytes, VolFiles, MaxVolBytes, Recycle \
FROM Media ORDER BY VolumeName";

/// Get a single job by ID.
pub const GET_JOB: &str = "\
SELECT JobId, Job, Name, Type, Level, ClientId, JobStatus, \
       SchedTime, StartTime, EndTime, RealEndTime, JobTDate, \
       VolSessionId, VolSessionTime, JobFiles, JobBytes, JobErrors, \
       PoolId, FileSetId, PriorJobId, Comment \
FROM Job WHERE JobId = $1";

/// Get job status summary (like .jobstatus).
pub const JOB_STATUS_SUMMARY: &str = "\
SELECT JobStatus, COUNT(*) as Count FROM Job GROUP BY JobStatus ORDER BY JobStatus";

/// Get total bytes and files per pool.
pub const POOL_SUMMARY: &str = "\
SELECT p.Name, COUNT(m.MediaId) as NumVols, \
       COALESCE(SUM(m.VolBytes), 0) as TotalBytes, \
       COALESCE(SUM(m.VolFiles), 0) as TotalFiles \
FROM Pool p LEFT JOIN Media m ON p.PoolId = m.PoolId \
GROUP BY p.Name ORDER BY p.Name";
