# llist -- Long List (Full Catalog Query)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L369) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Full or long list like list command

**Originalsyntax:**

```text
llist basefiles ( jobid=<jobid> | ujobid=<complete_name> ) | backups client=<client-name> [fileset=<fileset-name>] [jobstatus=<status>] [level=<level>] [order=<asc|desc>] [limit=<number>] [days=<number>] [hours=<number>] | client=<client-name> | clients | copies jobid=<jobid> | files jobid=<jobid> | files ujobid=<complete_name> | filesets | fileset [ filesetid=<filesetid> ] | fileset [ jobid=<jobid> ] | fileset [ ujobid=<complete_name> ] | jobs [job=<job-name>] [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [pool=<pool>] [days=<number>] [hours=<number>] [last] [count] | job=<job-name> [client=<client-name>] [jobstatus=<status>] [jobtype=<jobtype>] [joblevel=<joblevel>] [volume=<volumename>] [days=<number>] [hours=<number>] | jobid=<jobid> | ujobid=<complete_name> | joblog jobid=<jobid> [count] | joblog ujobid=<complete_name> [count] | jobmedia jobid=<jobid> | jobmedia ujobid=<complete_name> | jobtotals | jobstatistics jobid=<jobid> | log [client=<client-name>] [reverse] | media [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] | media=<media-name> | nextvolume job=<job-name> [days=<number>] | pools | pool=<pool-name> | poolid=<poolid> | storages | volumes [ jobid=<jobid> | ujobid=<complete_name> | pool=<pool-name> | all ] [count] | volume=<volume-name> | volumeid=<volumeid> | mediaid=<volumeid> | [current] | [enabled | disabled] | [limit=<number> [offset=<number>]]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Wesentliche Katalogabfragen sind portiert; die im Projekt-Backlog belegten zusätzlichen Pool-Prune-Summen (prunablevolumes/-jobs/-bytes) fehlen. Alle Originalfilter und Ausgabefelder sind noch nicht systematisch gegen Bareos getestet.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/list.rs:15](../../../crates/onesimus-dird/src/commands/list.rs).

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
