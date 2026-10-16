# list — List Catalog Records

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L367) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List objects from catalog

**Originalsyntax:**

```text
list basefiles ( jobid=<jobid> | ujobid=<complete_name> ) | backups client=<client-name> [fileset=<fileset-name>] [jobstatus=<status>] [level=<level>] [order=<asc|desc>] [limit=<number>] [days=<number>] [hours=<number>] | client=<client-name> | clients | copies jobid=<jobid> | files jobid=<jobid> | files ujobid=<complete_name> | filesets | fileset [ filesetid=<filesetid> ] | fileset [ jobid=<jobid> ] | fileset [ ujobid=<complete_name> ] | jobs [job=<job-name>] [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [pool=<pool>] [days=<number>] [hours=<number>] [last] [count] | job=<job-name> [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [days=<number>] [hours=<number>] | jobid=<jobid> | ujobid=<complete_name> | joblog jobid=<jobid> [count] | joblog ujobid=<complete_name> [count] | jobmedia jobid=<jobid> | jobmedia ujobid=<complete_name> | jobtotals | jobstatistics jobid=<jobid> | log [client=<client-name>] [reverse] | media [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] | media=<media-name> | nextvolume job=<job-name> [days=<number>] | pools | pool=<pool-name> | poolid=<poolid> | storages | volumes [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] [count] | volume=<volume-name> | volumeid=<volumeid> | mediaid=<volumeid> | [current] | [enabled | disabled] | [limit=<number> [offset=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 299 von 299 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-bvfs, parity-limited, parity-listacl, parity-operator.
- `list nextvolume` mit unbekanntem Job scheitert im Original im Batchmodus (CMD_FAILED), Onesimus meldet Erfolg (C-046).

**Offene Logeinträge:** `C-046` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (299):** `c002-list-twice`, `c002-private-then-shared`, `c002-shared-then-private`, `c002-client-keyword`, `c002-job-keyword`, `c002-catalog-keyword`, `c002-use-then-list`, `c002-api1`, `c002-api2`, `c002-gui`, `c002-limited-twice`, `c002-bvfs-noargs`, `c002-sql-then-list`, `c015-list-no-keyword`, `c015-list-mediatypes-unknown`, `c015-list-jobs-jobstatus-T_E`, `c015-list-jobs-jobstatus-terminated_fatal`, `c015-list-jobs-jobstatus-bogus`, `c015-list-jobs-jobstatus-f`, `c015-list-jobs-level-Full`, `c015-list-jobs-level-I`, `c015-list-jobs-level-Incremental_Differential`, `c015-list-jobs-level-bogus`, `c015-list-jobs-jobtype-B`, `c015-list-jobs-jobtype-Backup`, `c015-list-jobs-jobtype-Copy`, `c015-list-jobs-jobtype-bogus`, `c015-list-jobs-client-bareos-fd-limit-2`, `c015-list-jobs-client-nosuch`, `c015-list-jobs-job-BackupCatalog`, `c015-list-jobs-jobname-BackupCatalog-limit-2`, `c015-list-jobs-job-nosuch`, `c015-list-jobs-days-1`, `c015-list-jobs-hours-1`, `c015-list-jobs-hours-1000000`, `c015-list-jobs-last`, `c015-list-jobs-last-jobstatus-T`, `c015-list-jobs-limit-2-offset-1`, `c015-list-jobs-offset-3`, `c015-list-jobs-reverse-limit-3`, `c015-list-jobs-count`, `c015-list-jobs-count-jobstatus-T`, `c015-list-jobs-count-last`, `c015-list-jobs-volume-Full-47321`, `c015-list-jobs-pool-Full-limit-2`, `c015-list-jobs-pool-nosuch`, `c015-list-jobs-current`, `c015-list-jobs-enabled`, `c015-list-jobs-disabled`, `c015-list-jobs-limit-0`, `c015-list-jobs-limit-abc`, `c015-list-jobs-value`, `c015-list-job-missing`, `c015-list-jobid-missing`, `c015-list-jobid-14`, `c015-list-jobid-99999`, `c015-list-ujobid`, `c015-list-jobname`, `c015-list-jobtotals-limit`, `c015-list-files-jobid-5`, `c015-list-files-limit-offset`, `c015-list-files-nojobid`, `c015-list-files-jobid-99999`, `c015-list-files-jobid-14`, `c015-list-basefiles-nojobid`, `c015-list-files-ujobid`, `c015-list-fileset-jobid-1`, `c015-list-fileset-filesetid-3`, `c015-list-fileset-none`, `c015-list-filesets-limit`, `c015-list-fileset-jobid-14`, `c015-list-jobmedia-all`, `c015-list-jobmedia-99999`, `c015-list-joblog-none`, `c015-list-joblog-99999`, `c015-list-joblog-count`, `c015-list-joblog-limit`, `c015-list-log`, `c015-list-log-reverse`, `c015-list-log-client`, `c015-list-log-client-nosuch`, `c015-list-pools`, `c015-list-pool-nosuch`, `c015-list-poolid-3`, `c015-list-poolid-none`, `c015-list-pool-Incremental`, `c015-list-pool-Scratch`, `c015-list-pools-current`, `c015-list-client`, `c015-list-client-none`, `c015-list-client-nosuch`, `c015-list-clients-enabled`, `c015-list-storages-current`, `c015-list-volumes-all`, `c015-list-media-all`, `c015-list-volumes-jobid-1`, `c015-list-volumes-jobid-99999`, `c015-list-volumes-pool-nosuch`, `c015-list-volumes-limit`, `c015-list-volumes-count`, `c015-list-volumes-pool-count`, `c015-list-volume-nosuch`, `c015-list-mediaid`, `c015-list-mediaid-none`, `c015-list-volumes-all-limit`, `c015-list-copies-jobid`, `c015-list-copies-jobid-list`, `c015-list-copies-jobid-bad`, `c015-list-backups-none`, `c015-list-backups-fileset`, `c015-list-backups-fileset-any`, `c015-list-backups-fileset-nosuch`, `c015-list-backups-order-desc`, `c015-list-backups-order-asc`, `c015-list-backups-order-bad`, `c015-list-backups-jobstatus`, `c015-list-backups-jobstatus-bad`, `c015-list-backups-level`, `c015-list-backups-level-diff`, `c015-list-backups-client-nosuch`, `c015-list-jobstats`, `c015-list-jobstatistics-none`, `c015-list-nextvol`, `c015-list-nextvolume-days-3`, `c015-list-nextvolume-days-99`, `c015-list-nextvolume-catalog`, `c015-list-nextvolume-restore`, `c015-list-nextvolume-days-0`, `c015-list-jobs-x-jobtype-backup`, `c015-list-jobs-x-level-VirtualFull`, `c015-list-jobs-x-level-f`, `c015-list-jobs-x-jobstatus-[`, `c015-list-jobs-x-jobstatus-`, `c015-list-volumes-all-current`, `c015-list-backups-level-bad`, `c015-list-jobs-enabled-count`, `c015-api2x-list-jobs-current-limit-3`, `c015-api2x-list-clients-disabled`, `c015-api2x-list-files-jobid-1-limit-2-current`, `c015-api2-list-jobs-jobid-99999`, `c015-api2-list-files-jobid-5-limit-2-offset-1`, `c015-api2-list-log-limit-2`, `c015-api2-list-volumes-pool-Full`, `c015-api2-list-volumes-all`, `c015-api2-list-copies`, `c015-api2-list-backups-client-bareos-fd-limit-2`, `c015-api2-list-jobtotals`, `c015-api2-list-pool-Full`, `c015-api2-list-joblog-jobid-1-limit-2`, `c015-api2-list-nextvolume-job-backup-bareos-fd`, `c015-api2-list-jobs-count`, `c015-api2-list-volume-Full-47321`, `c015-api2-list-jobmedia-jobid-1`, `c015-api2-list-fileset-jobid-1`, `c015-api2-list-nosuch`, `c015-api2-list`, `c015-api2-list-jobid-14`, `c015-api2-list-basefiles-jobid-1`, `c015-api2-list-clients`, `c015-api2-list-storages`, `c015-api2-list-jobs-limit-2-offset-1`, `c015-api2-list-jobs-job-nosuch`, `c015-api2-list-joblog-jobid-1-count`, `c015-api2-list-mediaid-47321`, `c015-api2-list-volumes-jobid-1`, `c015-api2-list-jobstats-jobid-1`, `c015-api2-list-backups`, `c020-compact-list-pools`, `c026-limited-jobs`, `c026-limited-jobtotals`, `c026-limited-files`, `c026-limited-filesets`, `c026-limited-jobmedia`, `c026-limited-log`, `c026-limited-pools`, `c026-limited-clients`, `c026-limited-volumes`, `c026-limited-misc`, `c026-listacl-jobs`, `c026-listacl-jobtotals`, `c026-listacl-files`, `c026-listacl-filesets`, `c026-listacl-jobmedia`, `c026-listacl-log`, `c026-listacl-pools`, `c026-listacl-clients`, `c026-listacl-volumes`, `c026-listacl-misc`, `c026-operator-jobs`, `c026-operator-jobtotals`, `c026-operator-files`, `c026-operator-filesets`, `c026-operator-jobmedia`, `c026-operator-log`, `c026-operator-pools`, `c026-operator-clients`, `c026-operator-volumes`, `c026-operator-misc`, `c026-listacl-api2-1`, `c026-listacl-api2-2`, `c026-listacl-api2-3`, `c026-bvfs-list-denied`, `c026-bvfs-api2-list-denied`, `c027-gui-list-pools`, `c027-gui-list-jobs`, `c027-gui-list-media`, `c027-gui-list-clients`, `c027-gui-list-jobtotals`, `c027-gui-list-jobmedia`, `c027-gui-list-filesets`, `c027-gui-list-log`, `c027-gui-sqlquery-keyword`, `c027-gui-off-again`, `c027-gui-noarg`, `c027-gui-api1`, `c027-gui-api2`, `c028-list-jobs`, `c028-list-jobs-limit`, `c028-list-jobs-count`, `c028-list-jobs-last`, `c028-list-jobid`, `c028-list-job`, `c028-list-jobtotals`, `c028-list-basefiles`, `c028-list-files`, `c028-list-fileset`, `c028-list-filesets`, `c028-list-jobmedia`, `c028-list-joblog`, `c028-list-log`, `c028-list-pools`, `c028-list-pool`, `c028-list-clients`, `c028-list-client`, `c028-list-storages`, `c028-list-volumes`, `c028-list-volumes-pool`, `c028-list-media-all`, `c028-list-volume`, `c028-list-mediaid`, `c028-list-nextvol`, `c028-list-copies`, `c028-list-copies-jobid`, `c028-list-backups`, `c028-list-jobstats`, `c028-list-jobs-disabled`, `c028-list-clients-disabled`, `c028-list-schedules-disabled`, `c028-list-jobid-none`, `c028-list-files-none`, `c028-list-noarg`, `c028-list-bogus`, `c028-list-files-all`, `c028-list-volumes-count`, `c028-api-toggle`, `c035-list-jobtype-bogus-api1`, `c035-list-jobtype-bogus-api2`, `c035-list-jobtype-bogus-api2-compact`, `c035-list-nextvol-days-api1`, `c035-list-nextvol-days-api2`, `c035-list-nextvol-days-nojob-api2`, `c041-list-nextvol-backupcatalog`, `list-jobs`, `list-jobs-limit-5`, `list-jobs-last`, `list-jobs-jobstatus-T-limit-3`, `list-jobs-client-bareos-fd-limit-3`, `list-jobs-level-F-limit-3`, `list-jobs-days-0`, `list-jobid-1`, `list-job-backup-bareos-fd-limit-2`, `list-jobtotals`, `list-clients`, `list-pools`, `list-pool-Full`, `list-volumes`, `list-volumes-pool-Full`, `list-volume-Full-0001`, `list-media`, `list-filesets`, `list-files-jobid-1-limit-5`, `list-basefiles-jobid-1`, `list-copies`, `list-backups-client-bareos-fd-limit-3`, `list-joblog-jobid-1`, `list-jobmedia-jobid-1`, `list-log-limit-5`, `list-nextvolume-job-backup-bareos-fd`, `list-storages`, `list-mediatypes`, `list-jobs-limit-3-api2`, `list-pools-api2`, `list-volumes-api2`, `list-clients-api2`, `list-jobs-limited`, `list-bogus`, `w011-list-volumes-pool-cancel`, `w011-list-volumes-pool-api2`, `w011-list-nextvol-cancel`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/list.rs:17](../../../crates/onesimus-dird/src/commands/list.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Query the PostgreSQL catalog for job, volume, client, pool, and file records.

## Syntax

```
list <what> [limit=<N>] [jobid=<N>] [job=<name>]
```

## Sub-Commands

| What | Description | Source |
|------|-------------|--------|
| `jobs` | Job records (default) | `vw_job_summary` view |
| `jobid=<N>` | Single job by ID | `job` table |
| `job=<name>` | Jobs by name | `job` table |
| `volumes` / `media` | Volume records | `vw_volume_status` view |
| `clients` | Client records | `client` table |
| `pools` | Pool records | `pool` table |
| `files` | File records | `vw_file_list` view |
| `storages` | Storage records | `storage` table |
| `schedules` | Schedule records | `schedule` table |
| `filesets` | FileSet resources | config (not DB) |
| `jobtotals` | Summary statistics | `job` table (aggregate) |

## Examples

### List recent jobs
```
* list jobs
jobid  jobname           type level jobstatus clientname  starttime            endtime              jobfiles jobbytes
10     backup-bareos-fd  B    F     T         bareos-fd   2026-03-28 21:20:40  2026-03-28 21:20:42  458      71683878
9      backup-bareos-fd  B    F     T         bareos-fd   2026-03-28 21:18:16  2026-03-28 21:18:18  458      71683878
```

### List specific job
```
* list jobid=10
jobid  name              type level jobstatus starttime            endtime              jobfiles jobbytes  joberrors
10     backup-bareos-fd  B    F     T         2026-03-28 21:20:40  2026-03-28 21:20:42  458      71683878  0
```

### List with limit
```
* list jobs limit=3
```

### List volumes with fill percentage
```
* list volumes
mediaid volumename volstatus volbytes   voljobs volfiles mediatype poolname pct_full
1       Full-0001  Append    839002751  4       0        File      Full     0.0
```

### List files from a backup
```
* list files
fileid jobid filepath
902    2     /usr/sbin/
901    2     /usr/sbin/bareos-dird
900    2     /usr/sbin/bconsole
```

### List filesets (from config, not DB)
```
* list filesets
Name                      Include                                  Exclude
SelfTest                  /usr/sbin                                -
LinuxAll                  /                                        /proc, /tmp, /sys
```

### Job totals
```
* list jobtotals
jobs  files  bytes
10    4580   716838780
```

## Job Status Codes

| Code | Meaning |
|------|---------|
| `C` | Created (not yet running) |
| `R` | Running |
| `T` | Terminated successfully |
| `E` | Error |
| `f` | Fatal error |
| `W` | Warnings |
| `A` | Canceled by user |

## Related Commands

- [llist](llist.md) — Same as list (alias)
- [sqlquery](sqlquery.md) — Raw SQL queries
- [.sql](dot-sql.md) — Dot-command SQL interface
