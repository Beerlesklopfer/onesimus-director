# trace -- Toggle Trace File Output

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L485) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Turn on/off trace to file

**Originalsyntax:**

```text
trace on | off
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, API 1.
- Verglichen sind Dialog und Abbruch; das Schreiben der Trace-Datei ist nur durch Einheitentests belegt.

**Paritätsfälle (3):** `c011-trace-cancel`, `c011-trace-api1-cancel`, `trace-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:177](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Enable or disable writing Director output to a trace file for debugging purposes.

## Syntax

```
trace [on|off]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `on` | Enable trace file output |
| `off` | Disable trace file output |

## Examples

### Enable trace
```
* trace on
trace on
```

### Disable trace
```
* trace off
trace off
```

### Typical debugging workflow
```
* trace on
trace on
* run job=backup-bareos-fd level=Full yes
Job queued. JobId=15
* wait jobid=15
* trace off
trace off
```

## Behavior

- When enabled, the Director writes a copy of all console output to a trace file in the working directory (typically `/var/lib/bareos/bareos-dir.<pid>.trace`).
- Useful for capturing command output and debugging information for later analysis.
- The trace file is appended to -- it is not truncated when tracing is re-enabled.
- This setting is per-session and does not affect other connected consoles.
- For daemon-level debug tracing, use `setdebug` with the `trace` option instead.
- This is not an audited command.

## Related Commands

- [setdebug](setdebug.md) -- Set debug level with trace option
- [messages](messages.md) -- Display pending messages
