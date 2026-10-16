# sqlquery — Execute Raw SQL

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L482) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Use SQL to query catalog

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/sql.rs:92](../../../crates/onesimus-dird/src/commands/sql.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Execute a raw SQL query against the PostgreSQL catalog database.

## Syntax

```
sqlquery <SQL statement>
```

## Examples

### Count jobs
```
* sqlquery SELECT COUNT(*) AS total FROM job
total
--------------------
10
```

### Find large backups
```
* sqlquery SELECT jobid, name, jobbytes FROM job WHERE jobbytes > 50000000 ORDER BY jobbytes DESC
jobid               name                jobbytes
------------------------------------------------------------
10                  backup-bareos-fd    71683878
9                   backup-bareos-fd    71683878
```

### Check volume usage
```
* sqlquery SELECT volumename, volstatus, volbytes, voljobs FROM media
volumename          volstatus           volbytes            voljobs
--------------------------------------------------------------------------------
Full-0001           Append              839002751           4
```

### Use views
```
* sqlquery SELECT * FROM vw_last_backup_per_client
clientname          jobid  jobname           level starttime            endtime              jobfiles jobbytes
------------------------------------------------------------------------------------------------------------------
bareos-fd           10     backup-bareos-fd  F     2026-03-28 21:20:40  2026-03-28 21:20:42  458      71683878
```

### Call stored procedures
```
* sqlquery SELECT fn_find_appendable_volume('Full')
fn_find_appendable_volume
--------------------
Full-0001

* sqlquery SELECT fn_find_latest_backup('bareos-fd')
fn_find_latest_backup
--------------------
10
```

## Available Views

| View | Description |
|------|-------------|
| `vw_job_summary` | Jobs with client + pool names |
| `vw_volume_status` | Volumes with pool name + fill % |
| `vw_file_list` | Files with full path |
| `vw_last_backup_per_client` | Latest successful backup per client |

## Available Functions

| Function | Returns | Description |
|----------|---------|-------------|
| `fn_find_appendable_volume(pool)` | `text` | Find writable volume |
| `fn_find_latest_backup(client)` | `integer` | Latest successful backup JobId |
| `fn_since_utime(client, level)` | `(bigint, text)` | Reference time for incr/diff |

## Security

SQL queries run with the Director's database user privileges. Only SELECT is typically safe. INSERT/UPDATE/DELETE can corrupt the catalog.

## Related Commands

- [.sql](dot-sql.md) — Dot-command SQL (with sub-commands: tables, schema)
- [list](list.md) — Structured catalog queries
- [.sqlcount](dot-sqlcount.md) — Quick record count
