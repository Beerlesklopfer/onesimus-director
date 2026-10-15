# .locations — List Location Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L253) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Keine Beschreibung in der Originaltabelle (NULL)

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:330](../../../crates/onesimus-dird/src/commands/dot.rs).

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
