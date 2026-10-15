# .users — List User Resource Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L235) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all user resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- User-Ressourcen werden jetzt aus der rohen Konfiguration getrennt von Console-Ressourcen gelesen; Text/JSON und Reihenfolge sind durch Regressionstests geprüft. Der Live-SHA-256-Abgleich bleibt offen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:390](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one User resource name per line from the Director configuration.

## Syntax

```
.users
```

## Arguments

None.

## Examples

### List users
```
* .users
admin
operator
```

### No user resources configured (typical)
```
* .users
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- User resources are optional and rarely configured in most Bareos installations. Access control is typically handled via Console and Profile resources instead.
- Returns empty output when no User resources exist.

## Related Commands

- [.consoles](dot-consoles.md) -- List console names
- [.profiles](dot-profiles.md) -- List profile names
- [whoami](whoami.md) -- Show current console identity
