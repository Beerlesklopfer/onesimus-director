# .messages — List Message Resource Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L254) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Display pending messages

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `stub`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der registrierte Handler gibt immer eine leere Ausgabe zurück; er ruft keine ausstehenden Nachrichten ab. Der Sessionloop hat zusätzlich automatische Nachrichtenverarbeitung, was diesen expliziten Befehl nicht ersetzt.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:368](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one Messages resource name per line from the Director configuration.

## Syntax

```
.messages
```

## Arguments

None.

## Examples

### List message resources
```
* .messages
Standard
Daemon
```

### Single messages resource
```
* .messages
Standard
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Messages resources define how and where the Director sends log messages (mail, file, console, etc.).
- Used by console programs to populate the `messages=` argument completion list.

## Related Commands

- [messages](messages.md) -- Display pending console messages
- [.msgs](dot-msgs.md) -- List pending message types
- [show](show.md) -- Show full resource configuration
