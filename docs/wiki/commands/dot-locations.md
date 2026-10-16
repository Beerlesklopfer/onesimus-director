# .locations — List Location Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L253) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Keine Beschreibung in der Originaltabelle (NULL)

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 4 von 4 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (4):** `c002-shared-then-private`, `c029-locations`, `c029-locations-api2`, `dot_locations`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:312](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one location resource name per line from the Director configuration.

## Syntax

```
.locations
```

## Arguments

None.

## Examples

### List locations
```
* .locations
Vault-A
Offsite-B
```

### No locations configured (typical)
```
* .locations
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Location resources are rarely used in modern Bareos installations. They map to physical volume storage locations.
- Returns empty output when no Location resources are configured.

## Related Commands

- [.media](dot-media.md) -- List volume names
- [.storage](dot-storage.md) -- List storage names
