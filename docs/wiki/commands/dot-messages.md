# .messages — List Message Resource Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L254) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Display pending messages

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 8 von 8 Fällen byte-identisch; Modi: Text, API 1, API 2, GUI, Konsolen parity-limited, parity-operator.
- Jobmeldungen selbst (Texte aus Jobs) sind Gegenstand von C-051, nicht dieses Befehls.

**Paritätsfälle (8):** `c036-dot-messages-twice`, `c036-autodisplay-on-messages`, `c036-gui-on-messages`, `c036-messages-api1`, `c036-messages-api2`, `c036-messages-limited`, `c036-messages-operator`, `dot-messages`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:357](../../../crates/onesimus-dird/src/commands/dot.rs).

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
