# restore — Restore Files from Backup

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L402) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Restore files

**Originalsyntax:**

```text
restore where=</path> client=<client-name> storage=<storage-name> bootstrap=<file> restorejob=<job-name> comment=<text> jobid=<jobid> fileset=<fileset-name> replace=<always|never|ifolder|ifnewer> pluginoptions=<plugin-options-string> regexwhere=<regex> fileregex=<regex> restoreclient=<client-name> backupformat=<format> pool=<pool-name> file=<filename> directory=<directory> before=<date> strip_prefix=<prefix> add_prefix=<prefix> add_suffix=<suffix> select=<date> select before current copies done all
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler erzeugt Bootstrap für einen Job und startet direkt einen Restore-Worker. Interaktive Dateiauswahl fehlt; storage wird nicht als Override ausgewertet (erste konfigurierte Storage wird benutzt), viele Originaloptionen wie restoreclient/fileregex fehlen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:217](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Restore files from a previous backup to the original or alternate location.

## Syntax

```
restore [client=<name>] [jobid=<N>] [where=<path>] [replace=<mode>]
        [all] [done] [yes]
```

## Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `client` | from Job resource | Client that was backed up |
| `jobid` | latest successful | Backup JobId to restore from |
| `where` | `/tmp/bareos-restore` | Restore destination path |
| `replace` | `always` | File replace mode: `always`, `never`, `ifnewer`, `ifolder` |
| `all` | — | Select all files (skip interactive file selection) |
| `done` | — | Skip interactive mode, proceed immediately |
| `yes` | — | Skip confirmation prompt |

## Examples

### Restore latest backup to alternate location
```
* restore client=bareos-fd where=/tmp/restore-test all done yes
Restore job queued. JobId=10000
Restore from:  Backup JobId=10
Client:        bareos-fd
Storage:       File
Where:         /tmp/restore-test
Replace:       always
```

### Restore specific job
```
* restore client=bareos-fd jobid=7 where=/tmp/xxx all done yes
Restore job queued. JobId=10001
Restore from:  Backup JobId=7
Client:        bareos-fd
Storage:       File
Where:         /tmp/xxx
Replace:       always
```

### Restore to original location (careful!)
```
* restore client=bareos-fd jobid=10 where=/ replace=never all done yes
```
With `replace=never`, existing files are not overwritten.

## Behavior

1. Director generates bootstrap from JobMedia records in catalog
2. Bootstrap tells SD which volumes and byte ranges to read
3. Worker connects to SD (read mode) and FD
4. SD reads volume data, streams to FD
5. FD writes files to `where` path
6. Job status updated in catalog on completion

## Error: "No successful backup found"

```
No successful backup found for client 'bareos-fd'
Specify: restore client=bareos-fd jobid=<N> where=/path
```
This means no backup with `jobstatus='T'` exists for this client. Use `list jobs` to find available backups.

## Error: "Cannot generate bootstrap"

```
Cannot generate bootstrap for JobId=99 — no jobmedia records
```
The backup's JobMedia records are missing in the catalog. This can happen if the backup was interrupted before the SD sent CreateJobMedia.

## Related Commands

- [run](run.md) — Start a backup job
- [list](list.md) — List jobs to find available backups
- [.bvfs_lsfiles](dot-bvfs-lsfiles.md) — Browse backup contents interactively
