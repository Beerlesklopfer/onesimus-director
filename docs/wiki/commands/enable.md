# enable -- Enable a Job, Client, Schedule, or Storage

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L331) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Enable a job/client/schedule

**Originalsyntax:**

```text
enable job=<job-name> client=<client-name> schedule=<schedule-name>
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 29 von 29 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-device, parity-limited, parity-operator.

**Paritätsfälle (29):** `c006-enable-nojob-api2`, `c006-enable-noclient`, `c011-enable-cancel`, `c012-enable-storage`, `c025-enable-select-limited`, `c025-enable-select-limited-api2`, `c025-enable-client-select-limited`, `c025-enable-client-select-limited-api2`, `c025-enable-select-operator`, `c025-enable-select-operator-api2`, `c025-enable-client-select-operator`, `c025-enable-client-select-operator-api2`, `c025-enable-select-bvfs`, `c025-enable-select-bvfs-api2`, `c025-enable-client-select-bvfs`, `c025-enable-client-select-bvfs-api2`, `c025-enable-select-acl`, `c025-enable-select-acl-api2`, `c025-enable-client-select-acl`, `c025-enable-client-select-acl-api2`, `c032-job-disable-enable`, `c032-job-disable-enable-api1`, `c032-job-disable-enable-api2`, `c032-client-disable-enable`, `c032-client-disable-enable-api2`, `c032-job-select-disable-enable`, `c032-limited-job`, `c032-limited-job-api2`, `enable-nojob`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/job.rs:569](../../../crates/onesimus-dird/src/commands/job.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Re-enable a previously disabled resource so it can participate in scheduled and manual operations again.

## Syntax

```
enable [job=<name>|client=<name>|schedule=<name>|storage=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `job=<name>` | Enable a Job resource |
| `client=<name>` | Enable a Client resource |
| `schedule=<name>` | Enable a Schedule resource |
| `storage=<name>` | Enable a Storage resource |

## Examples

### Enable a job
```
* enable job=backup-bareos-fd
Job 'backup-bareos-fd' enabled
```

### Enable a client
```
* enable client=bareos-fd
Client 'bareos-fd' enabled
```

### Enable a schedule
```
* enable schedule=WeeklyCycle
Schedule 'WeeklyCycle' enabled
```

## Behavior

- Sets the `Enabled` flag to `yes` in the in-memory configuration for the named resource.
- The change is effective immediately but does not persist across Director restarts unless the configuration is also saved (via `configure` or manual edit).
- Enabling an already-enabled resource succeeds silently.
- If the resource name is not found, an error is returned.
- Without arguments, the command prints usage help.
- This is an audited command.

## Related Commands

- [disable](disable.md) -- Disable a job/client/schedule/storage
- [show](show.md) -- Show resource configuration (includes enabled state)
- [reload](reload.md) -- Reload configuration from disk
