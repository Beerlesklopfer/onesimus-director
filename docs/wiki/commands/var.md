# var -- Display Variable Expansion

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L520) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Does variable expansion

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 16 von 16 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-limited, parity-operator.
- Nacktes `var` gibt im Original undefinierte Speicherreste aus; der Fall `var` prüft deshalb nur die Katalogzeilen davor.

**Paritätsfälle (16):** `c023-var-blank`, `c023-var-date`, `c023-var-job`, `c023-var-jobname`, `c023-var-undefined`, `c023-var-env`, `c023-var-ops`, `c023-var-errors`, `c023-var-escapes`, `c023-var-loops`, `c023-var-catalog-arg`, `c023-var-api2`, `c023-use-var`, `c023-limited`, `c023-operator`, `var`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:199](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Expand and display the value of a Bareos variable. Used for testing variable substitution in job definitions.

## Syntax

```
var <variable-name>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `<variable-name>` | Name of the variable to expand |

## Examples

### Expand a variable
```
* var Client
Variable Client: bareos-fd
```

### Undefined variable
```
* var FooBar
Variable FooBar: undefined
```

### Check job-related variables
```
* var Job
Variable Job: backup-bareos-fd
```

## Behavior

- Expands Bareos runtime variables as they would be substituted in RunScript or other contexts.
- Common variables include: `%c` (Client), `%n` (Job name), `%l` (Level), `%t` (Job type), `%d` (Director name).
- If the variable is not defined or not in a job context, it shows as `undefined`.
- Useful for verifying RunBefore/RunAfter scripts and path templates.
- This is not an audited command.

## Related Commands

- [show](show.md) -- Show resource configuration
- [run](run.md) -- Run a job (where variables are expanded)
