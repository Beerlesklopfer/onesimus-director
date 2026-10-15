# estimate -- Estimate Backup Size

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L334) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Performs FileSet estimate, listing gives full listing

**Originalsyntax:**

```text
estimate fileset=<fileset-name> client=<client-name> level=<level> accurate=<yes/no> job=<job-name> listing
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `stub`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler liest Jobdaten, führt aber keine FD-Schätzung aus; die Ausgabe lautet ausdrücklich Estimate pending.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:368](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Contact the File Daemon to estimate the number of files and bytes that would be backed up, without actually running a backup job.

## Syntax

```
estimate [job=<name>] [client=<name>] [listing]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `job=<name>` | Job resource to estimate (required) |
| `client=<name>` | Override the client from the Job resource |
| `listing` | Show individual file listing with sizes |

## Examples

### Basic estimate
```
* estimate job=backup-bareos-fd
Connecting to Client "bareos-fd" for estimate...
Job: backup-bareos-fd
FileSet: SelfTest
2,458 files, 156,234,567 bytes (149.0 MB)
```

### Estimate with file listing
```
* estimate job=backup-bareos-fd listing
Connecting to Client "bareos-fd" for estimate...
Job: backup-bareos-fd
FileSet: SelfTest
Listing mode enabled
/usr/sbin/bareos-dird      1,234,567
/usr/sbin/bconsole           456,789
/usr/sbin/bareos-fd          987,654
...
2,458 files, 156,234,567 bytes (149.0 MB)
```

### Without arguments (shows available jobs)
```
* estimate
estimate: specify job=<name>
Backup jobs: backup-bareos-fd, BackupCatalog
```

## Behavior

- The Director connects to the File Daemon via TLS-PSK and requests a file listing based on the Job's FileSet.
- No data is transferred to the Storage Daemon -- this is a dry run.
- The `listing` flag causes individual files and their sizes to be printed.
- Useful for capacity planning and verifying FileSet Include/Exclude patterns.
- This is not an audited command.

## Related Commands

- [run](run.md) -- Actually run the backup job
- [show](show.md) -- Show FileSet configuration
- [status](status.md) -- Check client connectivity
