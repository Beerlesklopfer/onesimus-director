# .volstatus — List Volume Status Values

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L280) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all volume status

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 2 von 2 Fällen byte-identisch; Modi: Text, API 2.

**Paritätsfälle (2):** `c029-volstatus-api2`, `dot_volstatus`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:390](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one possible volume status value per line.

## Syntax

```
.volstatus
```

## Arguments

None.

## Examples

### List all volume statuses
```
* .volstatus
Append
Archive
Disabled
Error
Full
Purged
Recycle
Read-Only
Used
Cleaning
```

## Notes

- Output is one status name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Returns a static list of all possible VolStatus values in the Media table.
- Used by console programs to populate the `volstatus=` argument completion list (e.g., for `update volume=... volstatus=`).

## Related Commands

- [.media](dot-media.md) -- List volume names
- [.pools](dot-pools.md) -- List pool names
- [update](update.md) -- Update volume attributes
- [list](list.md) -- List volumes from catalog
