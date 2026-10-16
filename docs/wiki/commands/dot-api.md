# .api — Set API Output Mode

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L222) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Switch between different api modes

**Originalsyntax:**

```text
.api [ 0 | 1 | 2 | off | on | json ] [compact=<yes|no>]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 28 von 28 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (28):** `c018-api2`, `c018-api2-client`, `c020-api-noarg`, `c020-api-off`, `c020-api-on`, `c020-api-json`, `c020-api-upper`, `c020-api-invalid`, `c020-api-invalid-api2`, `c020-api-toomany`, `c020-api-compact-yes`, `c020-api-compact-no`, `c020-api-compact-upper`, `c020-api-compact-novalue`, `c020-api-1-compact`, `c020-compact-persists`, `c020-compact-reset`, `c028-api-toggle`, `c042-status-storage-api1`, `c042-status-storage-api2`, `c042-dot-status-storage-api2`, `c042-status-storage-slots-api1`, `c042-status-storage-slots-api2`, `c042-status-storage-api1-twice`, `dot-api-0`, `dot-api-1`, `dot-api-2`, `dot-api-json`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:411](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Switches the output format for the current console session. Used by bconsole, oconsole, and the WebUI to select between human-readable, machine-readable, and JSON output.

## Syntax

```
.api [0|1|2|off|on|json] [compact=<yes|no>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `0` / `off` | Human-readable output (colored, formatted) |
| `1` / `on` | Machine-readable output (tab-separated, for automation) |
| `2` / `json` | JSON output |
| `compact=yes` | Use compact JSON (no extra whitespace) |
| `compact=no` | Use pretty-printed JSON |

## Examples

### Switch to machine-readable mode
```
* .api 1
api: mode=1
```

### Switch to JSON mode
```
* .api json
api: mode=2
```

### Switch back to human-readable
```
* .api 0
api: mode=0
```

## Notes

- Returns `api: mode=<N>` confirming the new mode.
- Output is terminated by the EOD signal.
- The mode persists for the entire session (until changed or disconnected).
- Mode 0 is the default for interactive console use.
- Mode 1 is used by bconsole internally for tab-completion (via `.help` output parsing).
- Mode 2 is used by the Bareos WebUI for all API calls.

## Related Commands

- [.help](dot-help.md) -- List commands (output format depends on API mode)
- [.grpc](dot-grpc.md) -- Toggle gRPC streaming mode
