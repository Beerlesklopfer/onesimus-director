# .volstatus — List Volume Status Values

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L280) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all volume status

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:335](../../../crates/onesimus-dird/src/commands/dot.rs).

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
