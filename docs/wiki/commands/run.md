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

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Incremental/Differential bleiben bei passender erfolgreicher Backup-Kette erhalten. Referenzen sind nach Client, Job und FileSet eingegrenzt; ohne Full-Basis wird vor Poolwahl und Jobanlage auf Full hochgestuft, DB-Fehler brechen ab. FileSet-/Pool-Overrides werden ausgewertet. Accurate/VirtualFull, FileSet-Inhaltsänderungsprüfung, weitere Originaloptionen und Live-SHA-256-/Backup-Abgleich bleiben offen.

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
