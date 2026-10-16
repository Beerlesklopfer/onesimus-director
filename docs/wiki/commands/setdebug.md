# setdebug -- Set Debug Level

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L459) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets debug level

**Originalsyntax:**

```text
setdebug level=<nn> trace=0/1 timestamp=0/1 client=<client-name> | dir | storage=<storage-name> | all
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 7 von 7 Fällen byte-identisch; Modi: Text, API 1.
- Verglichen sind nur Dialog/Abbruch und `setdebug level=0 dir`; Debug-Pegel an echte FD/SD sind laut Testregeln nicht live erlaubt und nur durch Einheitentests belegt (NDMP: C-050).

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (7):** `c011-setdebug-cancel`, `c011-setdebug-badlevel-cancel`, `c011-setdebug-daemon-cancel`, `c011-setdebug-daemon-api1-cancel`, `c011-setdebug-daemon-badnumber`, `setdebug-dir`, `setdebug-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/debug.rs:33](../../../crates/onesimus-dird/src/commands/debug.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Set the debug verbosity level on the Director, a File Daemon, or a Storage Daemon.

## Syntax

```
setdebug level=<nn> [trace=0|1] [timestamp=0|1] [client=<name>] [storage=<name>] [dir]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `level=<nn>` | Debug level (0=off, higher=more verbose, up to 999) |
| `trace=0\|1` | Enable/disable trace file output |
| `timestamp=0\|1` | Enable/disable timestamps in debug output |
| `client=<name>` | Set debug on this File Daemon |
| `storage=<name>` | Set debug on this Storage Daemon |
| `dir` | Set debug on the Director itself |

## Examples

### Set Director debug level
```
* setdebug level=100 dir
Debug level set to 100
```

### Set debug on a File Daemon
```
* setdebug level=200 client=bareos-fd
Debug level set to 200
```

### Enable trace file on Storage Daemon
```
* setdebug level=150 trace=1 storage=File
Debug level set to 150
```

## Behavior

- Without a target (`client`, `storage`, `dir`), sets the debug level on the Director.
- When targeting a client or storage, the Director connects to the daemon via TLS-PSK and sets the debug level remotely.
- Debug level 0 disables debug output.
- Common useful levels: 50 (basic tracing), 100 (protocol), 200 (detailed), 500+ (very verbose).
- `trace=1` writes debug output to a trace file in `/var/lib/bareos/` (or the working directory).
- `timestamp=1` prefixes each debug line with a timestamp.
- This is an audited command.

## Related Commands

- [trace](trace.md) -- Toggle trace file output for the console session
- [status](status.md) -- Check daemon status
- [setdevice](setdevice.md) -- Set device parameters
