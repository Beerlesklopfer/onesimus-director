# estimate -- Estimate Backup Size

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L334) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Performs FileSet estimate, listing gives full listing

**Originalsyntax:**

```text
estimate fileset=<fileset-name> client=<client-name> level=<level> accurate=<yes/no> job=<job-name> listing
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 38 von 38 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Fragt den echten FD ab (lesend).

**Paritätsfälle (38):** `c011-estimate-cancel`, `c017-full`, `c017-default-level`, `c017-incremental`, `c017-differential`, `c017-catalog-job`, `c017-listing`, `c017-listing-full`, `c017-accurate-yes`, `c017-accurate-incr`, `c017-accurate-no`, `c017-accurate-bad`, `c017-accurate-missing`, `c017-unknown-job`, `c017-job-missing`, `c017-restore-job`, `c017-client-override`, `c017-fd-override`, `c017-fileset-override`, `c017-client-fileset-nojob`, `c017-unknown-client`, `c017-client-missing`, `c017-unknown-fileset`, `c017-fileset-missing`, `c017-bad-level`, `c017-level-missing`, `c017-twice`, `c017-api2`, `c017-api2-unknown-job`, `c017-select-job`, `c041-estimate-backupcatalog`, `c041-estimate-backupfd`, `estimate`, `estimate-listing`, `w011-estimate-badnumber-cancel`, `w011-estimate-level-cancel`, `w011-estimate-api1-cancel`, `w011-estimate-api2`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/estimate.rs:139](../../../crates/onesimus-dird/src/commands/estimate.rs).

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
