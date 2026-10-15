# list — List Catalog Records

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L367) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List objects from catalog

**Originalsyntax:**

```text
list basefiles ( jobid=<jobid> | ujobid=<complete_name> ) | backups client=<client-name> [fileset=<fileset-name>] [jobstatus=<status>] [level=<level>] [order=<asc|desc>] [limit=<number>] [days=<number>] [hours=<number>] | client=<client-name> | clients | copies jobid=<jobid> | files jobid=<jobid> | files ujobid=<complete_name> | filesets | fileset [ filesetid=<filesetid> ] | fileset [ jobid=<jobid> ] | fileset [ ujobid=<complete_name> ] | jobs [job=<job-name>] [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [pool=<pool>] [days=<number>] [hours=<number>] [last] [count] | job=<job-name> [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [days=<number>] [hours=<number>] | jobid=<jobid> | ujobid=<complete_name> | joblog jobid=<jobid> [count] | joblog ujobid=<complete_name> [count] | jobmedia jobid=<jobid> | jobmedia ujobid=<complete_name> | jobtotals | jobstatistics jobid=<jobid> | log [client=<client-name>] [reverse] | media [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] | media=<media-name> | nextvolume job=<job-name> [days=<number>] | pools | pool=<pool-name> | poolid=<poolid> | storages | volumes [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] [count] | volume=<volume-name> | volumeid=<volumeid> | mediaid=<volumeid> | [current] | [enabled | disabled] | [limit=<number> [offset=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Wesentliche Katalogabfragen sind portiert; die im Projekt-Backlog belegten zusätzlichen Pool-Prune-Summen (prunablevolumes/-jobs/-bytes) fehlen. Alle Originalfilter und Ausgabefelder sind noch nicht systematisch gegen Bareos getestet.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/list.rs:15](../../../crates/onesimus-dird/src/commands/list.rs).

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
