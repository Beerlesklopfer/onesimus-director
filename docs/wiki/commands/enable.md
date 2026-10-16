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

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

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
