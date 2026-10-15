# disable -- Disable a Job, Client, Schedule, or Storage

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L328) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Disable a job/client/schedule

**Originalsyntax:**

```text
disable job=<job-name> client=<client-name> schedule=<schedule-name>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/job.rs:290](../../../crates/onesimus-dird/src/commands/job.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Disable a resource so it will not be used for scheduled operations or manual runs.

## Syntax

```
disable [job=<name>|client=<name>|schedule=<name>|storage=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `job=<name>` | Disable a Job resource |
| `client=<name>` | Disable a Client resource |
| `schedule=<name>` | Disable a Schedule resource |
| `storage=<name>` | Disable a Storage resource |

## Examples

### Disable a job during maintenance
```
* disable job=backup-bareos-fd
Job 'backup-bareos-fd' disabled
```

### Disable a client
```
* disable client=bareos-fd
Client 'bareos-fd' disabled
```

### Verify with show
```
* disable job=backup-bareos-fd
Job 'backup-bareos-fd' disabled
* show jobs
Job {
  Name = "backup-bareos-fd"
  ...
  Enabled = no
}
```

## Behavior

- Sets the `Enabled` flag to `no` in the in-memory configuration for the named resource.
- Disabled jobs will not run via the scheduler and `run job=<name>` will be rejected with "Job is disabled".
- Disabled clients are excluded from automatic job selection.
- Disabled schedules stop triggering their associated jobs.
- The change is effective immediately but does not persist across Director restarts unless saved.
- Without arguments, the command prints usage help.
- This is an audited command.

## Related Commands

- [enable](enable.md) -- Enable a job/client/schedule/storage
- [show](show.md) -- Show resource configuration
- [reload](reload.md) -- Reload configuration from disk
