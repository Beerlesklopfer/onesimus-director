# run — Start a Job

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L431) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Run a job

**Originalsyntax:**

```text
run job=<job-name> client=<client-name> fileset=<fileset-name> level=<level> storage=<storage-name> where=<directory-prefix> when=<universal-time-specification> pool=<pool-name> pluginoptions=<plugin-options-string> accurate=<yes|no> comment=<text> spooldata=<yes|no> priority=<number> jobid=<jobid> catalog=<catalog> migrationjob=<job-name> backupclient=<client-name> backupformat=<format> nextpool=<pool-name> since=<universal-time-specification> verifyjob=<job-name> verifylist=<verify-list> migrationjob=<complete_name> [ yes ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.
- `run job=RestoreFiles` fragt im Original nach der JobId (C-043); Onesimus geht direkt zu „OK to run?“.
- Der Paritätslauf startet keine echten Jobs (nur Zusammenfassung, `mod`/`no`/Abbruch). Jobablauf, Jobmeldungen (C-051) und CatReq-Updates (C-055) sind nur durch Einheitentests belegt.

**Offene Logeinträge:** `C-043`, `C-051`, `C-055` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (30):** `c011-run-cancel`, `c011-run-badnumber`, `c011-run-api1-cancel`, `c011-run-api2`, `c011-run-job-no`, `c011-run-job-cancel`, `c011-run-job-illegal-no`, `c011-run-job-mod-cancel`, `c011-run-job-mod-level-no`, `c011-run-job-mod-priority-no`, `c011-run-job-mod-pool-no`, `c011-run-job-dotmod-no`, `c011-run-select-no`, `c011-run-job-api1-no`, `c012-run-unknown-noyes`, `c012-run-confirm`, `c025-run-select-limited`, `c025-run-select-limited-api2`, `c025-run-select-operator`, `c025-run-select-operator-api2`, `c025-run-select-bvfs`, `c025-run-select-bvfs-api2`, `c025-run-select-acl`, `c025-run-select-acl-api2`, `c041-run-backupcatalog-no`, `c041-run-backupcatalog-api1-no`, `c041-run-backupcatalog-mod-cancel`, `c041-run-backupfd-no`, `run-noargs`, `run-unknownjob`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/job.rs:15](../../../crates/onesimus-dird/src/commands/job.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Start a backup, restore, verify, or admin job.

## Syntax

```
run [job=<name>] [level=<Full|Incremental|Differential>] [client=<name>]
    [storage=<name>] [pool=<name>] [fileset=<name>] [when=<datetime>]
    [priority=<n>] [yes]
```

## Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `job` | — | Job resource name (required) |
| `level` | from Job resource | Backup level: `Full`, `Incremental`, `Differential` |
| `client` | from Job resource | Client to back up |
| `storage` | from Job resource | Storage daemon to use |
| `pool` | from Job resource | Volume pool |
| `fileset` | from Job resource | FileSet resource |
| `when` | `now` | Schedule time (ISO 8601 or `now`) |
| `priority` | from Job resource | Job priority (1-100, lower = higher) |
| `yes` | — | Skip confirmation prompt |

## Examples

### Full backup with confirmation
```
* run job=backup-bareos-fd level=Full
Run Backup job
Job:        backup-bareos-fd
Level:      Full
Client:     bareos-fd
Storage:    File
Pool:       Full
FileSet:    SelfTest
Priority:   10
When:       now
OK to run? (yes/mod/no): yes
Job queued. JobId=10
```

### Full backup without confirmation
```
* run job=backup-bareos-fd level=Full yes
Job queued. JobId=11
```

### Incremental backup
```
* run job=backup-bareos-fd level=Incremental yes
Job queued. JobId=12
```
If no previous Full backup exists, the level is automatically upgraded to Full.

### Differential backup
```
* run job=backup-bareos-fd level=Differential yes
Job queued. JobId=13
```
References the last successful Full backup for the same client.

### List available jobs
```
* run
The defined Job resources are:
     1. backup-bareos-fd
     2. BackupCatalog
     3. RestoreFiles
Select Job resource (1):
```

## Behavior

1. The Director creates a Job record in the catalog (DB-generated JobId)
2. A worker process is forked
3. Worker connects to FD and SD via TLS-PSK
4. FD scans files, sends data to SD
5. SD writes to volume, sends CatReq to Director
6. On completion: job status, files, bytes updated in catalog

## Related Commands

- [cancel](cancel.md) — Cancel a running job
- [restore](restore.md) — Restore files from a backup
- [list](list.md) — List jobs in catalog
- [status](status.md) — Show running jobs
