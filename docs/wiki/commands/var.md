# var -- Display Variable Expansion

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L520) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Does variable expansion

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `stub`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler gibt Variable <name>: undefined zurück; Bareos-Variablenexpansion fehlt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:156](../../../crates/onesimus-dird/src/commands/extra.rs).

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
