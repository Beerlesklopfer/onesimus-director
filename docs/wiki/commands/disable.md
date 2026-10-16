# disable -- Disable a Job, Client, Schedule, or Storage

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L328) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Disable a job/client/schedule

**Originalsyntax:**

```text
disable job=<job-name> client=<client-name> schedule=<schedule-name>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 30 von 30 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-device, parity-limited, parity-operator.

**Paritätsfälle (30):** `c006-disable-nojob-api2`, `c006-disable-noschedule`, `c011-disable-cancel`, `c012-disable-storage`, `c025-disable-select-limited`, `c025-disable-select-limited-api2`, `c025-disable-schedule-select-limited`, `c025-disable-schedule-select-limited-api2`, `c025-disable-select-operator`, `c025-disable-select-operator-api2`, `c025-disable-select-bvfs`, `c025-disable-select-bvfs-api2`, `c025-disable-schedule-select-bvfs`, `c025-disable-schedule-select-bvfs-api2`, `c025-disable-select-acl`, `c025-disable-select-acl-api2`, `c025-disable-schedule-select-acl`, `c025-disable-schedule-select-acl-api2`, `c032-job-disable-enable`, `c032-job-disable-enable-api1`, `c032-job-disable-enable-api2`, `c032-client-disable-enable`, `c032-client-disable-enable-api2`, `c032-schedule-disable`, `c032-schedule-disable-api1`, `c032-schedule-disable-api2`, `c032-job-select-disable-enable`, `c032-limited-job`, `c032-limited-job-api2`, `disable-nojob`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/job.rs:573](../../../crates/onesimus-dird/src/commands/job.rs).

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
