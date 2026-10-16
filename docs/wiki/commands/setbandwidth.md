# setbandwidth -- Set Bandwidth Limit

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L454) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets bandwidth

**Originalsyntax:**

```text
setbandwidth [ client=<client-name> | storage=<storage-name> | jobid=<jobid> | job=<job-name> | ujobid=<unique-jobid> state=<job_state> | all ] limit=<nn-kbs> [ yes ]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 6 von 6 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Verglichen sind Dialog/Abbruch und `setbandwidth limit=100 client=bareos-fd` in API 1/2 (setzt das Limit am gemeinsamen echten FD; die tatsächliche Drosselung wird nicht geprüft). SD-Limits und Jobselektoren sind nur durch Einheitentests belegt (NDMP: C-050).

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (6):** `c011-setbandwidth-cancel`, `c011-setbandwidth-badlimit-cancel`, `c011-setbandwidth-api2`, `c035-setbandwidth-api1`, `c035-setbandwidth-api2`, `setbandwidth-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/bandwidth.rs:10](../../../crates/onesimus-dird/src/commands/bandwidth.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Set a bandwidth limit for data transfer to/from a specific client or running job.

## Syntax

```
setbandwidth limit=<speed> [client=<name>] [jobid=<id>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `limit=<speed>` | Bandwidth limit in bytes/second (required). Use suffixes: `k` (KB/s), `m` (MB/s) |
| `client=<name>` | Apply limit to all jobs for this client |
| `jobid=<id>` | Apply limit to this specific running job |

## Examples

### Limit a client to 10 MB/s
```
* setbandwidth limit=10m client=bareos-fd
Bandwidth limit set to 10m
```

### Limit a specific running job
```
* setbandwidth limit=5m jobid=15
Bandwidth limit set to 5m
```

### Remove bandwidth limit
```
* setbandwidth limit=0 client=bareos-fd
Bandwidth limit set to 0
```

## Behavior

- The Director sends the bandwidth limit to the File Daemon via TLS-PSK.
- When `client` is specified, the limit applies to all current and future jobs for that client.
- When `jobid` is specified, the limit applies only to that specific running job.
- Setting `limit=0` removes the bandwidth restriction.
- The limit is not persistent -- it resets when the FD restarts or the job completes.
- Useful for throttling backups during business hours to reduce network impact.
- This is an audited command.

## Related Commands

- [status](status.md) -- Check running jobs and their throughput
- [setdebug](setdebug.md) -- Set debug level on daemons
