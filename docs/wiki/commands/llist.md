# llist -- Long List (Full Catalog Query)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L369) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Full or long list like list command

**Originalsyntax:**

```text
llist basefiles ( jobid=<jobid> | ujobid=<complete_name> ) | backups client=<client-name> [fileset=<fileset-name>] [jobstatus=<status>] [level=<level>] [order=<asc|desc>] [limit=<number>] [days=<number>] [hours=<number>] | client=<client-name> | clients | copies jobid=<jobid> | files jobid=<jobid> | files ujobid=<complete_name> | filesets | fileset [ filesetid=<filesetid> ] | fileset [ jobid=<jobid> ] | fileset [ ujobid=<complete_name> ] | jobs [job=<job-name>] [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [pool=<pool>] [days=<number>] [hours=<number>] [last] [count] | job=<job-name> [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [days=<number>] [hours=<number>] | jobid=<jobid> | ujobid=<complete_name> | joblog jobid=<jobid> [count] | joblog ujobid=<complete_name> [count] | jobmedia jobid=<jobid> | jobmedia ujobid=<complete_name> | jobtotals | jobstatistics jobid=<jobid> | log [client=<client-name>] [reverse] | media [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] | media=<media-name> | nextvolume job=<job-name> [days=<number>] | pools | pool=<pool-name> | poolid=<poolid> | storages | volumes [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] [count] | volume=<volume-name> | volumeid=<volumeid> | mediaid=<volumeid> | [current] | [enabled | disabled] | [limit=<number> [offset=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 110 von 110 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-bvfs, parity-limited, parity-listacl, parity-operator.
- `llist nextvolume` teilt den Pfad von `list nextvolume` (C-046).

**Offene Logeinträge:** `C-046` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (110):** `c015-llist-no-keyword`, `c015-llist-jobs-jobstatus-f`, `c015-llist-jobs-last`, `c015-llist-jobs-count`, `c015-llist-jobs-limit-2-offset-1`, `c015-llist-jobs-reverse-limit-2`, `c015-llist-jobs-client-bareos-fd-limit-1`, `c015-llist-jobs-volume-Full-47321-limit-2`, `c015-llist-jobid-14`, `c015-llist-ujobid-nosuch`, `c015-llist-files-jobid-5`, `c015-llist-fileset-jobid-1`, `c015-llist-jobmedia-all`, `c015-llist-joblog-count`, `c015-llist-log`, `c015-llist-pool-nosuch`, `c015-llist-poolid-3`, `c015-llist-client`, `c015-llist-volumes-all`, `c015-llist-volumes-jobid-1`, `c015-llist-volumeid`, `c015-llist-backups`, `c015-llist-jobstats`, `c015-llist-pools-current`, `c015-llist-volumes-all-current`, `c015-llist-jobs-current-limit`, `c015-api2x-llist-jobs-disabled`, `c015-api2-llist-pool-Full`, `c015-api2-llist-jobs-jobid-14`, `c026-limited-llist-jobs`, `c026-limited-filesets`, `c026-limited-log`, `c026-limited-pools`, `c026-limited-clients`, `c026-limited-volumes`, `c026-limited-misc`, `c026-listacl-llist-jobs`, `c026-listacl-filesets`, `c026-listacl-log`, `c026-listacl-pools`, `c026-listacl-clients`, `c026-listacl-volumes`, `c026-listacl-misc`, `c026-operator-llist-jobs`, `c026-operator-filesets`, `c026-operator-log`, `c026-operator-pools`, `c026-operator-clients`, `c026-operator-volumes`, `c026-operator-misc`, `c026-listacl-api2-1`, `c026-listacl-api2-2`, `c026-listacl-api2-3`, `c026-bvfs-list-denied`, `c027-gui-llist-pools`, `c027-gui-llist-jobs`, `c027-gui-llist-media`, `c027-gui-list-clients`, `c027-gui-sqlquery-keyword`, `c027-gui-off-again`, `c028-llist-jobs`, `c028-llist-jobs-limit`, `c028-llist-jobid`, `c028-llist-jobtotals`, `c028-llist-files`, `c028-llist-fileset`, `c028-llist-filesets`, `c028-llist-jobmedia`, `c028-llist-joblog`, `c028-llist-log`, `c028-llist-pools`, `c028-llist-clients`, `c028-llist-storages`, `c028-llist-volumes`, `c028-llist-backups`, `c028-llist-jobs-last`, `c028-llist-pool-full`, `llist-jobs`, `llist-jobs-limit-5`, `llist-jobs-last`, `llist-jobs-jobstatus-T-limit-3`, `llist-jobs-client-bareos-fd-limit-3`, `llist-jobs-level-F-limit-3`, `llist-jobs-days-0`, `llist-jobid-1`, `llist-job-backup-bareos-fd-limit-2`, `llist-jobtotals`, `llist-clients`, `llist-pools`, `llist-pool-Full`, `llist-volumes`, `llist-volumes-pool-Full`, `llist-volume-Full-0001`, `llist-media`, `llist-filesets`, `llist-files-jobid-1-limit-5`, `llist-basefiles-jobid-1`, `llist-copies`, `llist-backups-client-bareos-fd-limit-3`, `llist-joblog-jobid-1`, `llist-jobmedia-jobid-1`, `llist-log-limit-5`, `llist-nextvolume-job-backup-bareos-fd`, `llist-storages`, `llist-mediatypes`, `llist-jobs-limit-3-api2`, `llist-pools-api2`, `llist-volumes-api2`, `llist-clients-api2`, `w011-llist-volumes-pool-cancel`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/list.rs:17](../../../crates/onesimus-dird/src/commands/list.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display catalog records in long (verbose) format. This command is an alias for `list` and produces the same output.

## Syntax

```
llist [jobs|clients|pools|volumes|media] [jobid=<id>] [limit=<N>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `jobs` | List job records (default) |
| `clients` | List client records |
| `pools` | List pool records |
| `volumes` / `media` | List volume records |
| `jobid=<id>` | Show a specific job by ID |
| `limit=<N>` | Limit number of results |

## Examples

### Long list of jobs
```
* llist jobs
jobid  jobname           type level jobstatus clientname  starttime            endtime              jobfiles jobbytes
10     backup-bareos-fd  B    F     T         bareos-fd   2026-03-28 21:20:40  2026-03-28 21:20:42  458      71683878
```

### Long list of volumes
```
* llist volumes
mediaid volumename volstatus volbytes   voljobs volfiles mediatype poolname pct_full
1       Full-0001  Append    839002751  4       0        File      Full     0.0
```

### Long list with limit
```
* llist jobs limit=5
```

## Behavior

- In standard Bareos, `llist` displays all columns from the catalog while `list` shows a condensed view. In this implementation, `llist` and `list` produce identical output.
- Both commands query the same catalog views and tables.
- Accepts the same arguments as `list`.
- This is not an audited command.

## Related Commands

- [list](list.md) -- List catalog contents (same output)
- [sqlquery](sqlquery.md) -- Raw SQL queries for custom reports
