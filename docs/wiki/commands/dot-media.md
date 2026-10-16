# .media — List Volume/Media Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L256) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all medias

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 5 von 5 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (5):** `c002-shared-then-private`, `c029-media`, `c029-media-api1`, `c029-media-api2`, `dot_media`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:322](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one volume name per line from the catalog database.

## Syntax

```
.media
```

## Arguments

None.

## Examples

### List all volumes
```
* .media
Full-0001
Full-0002
Incremental-0001
Incremental-0002
```

### No volumes labeled yet
```
* .media
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Requires a catalog database connection. Returns empty output if no catalog is configured.
- Lists volumes (media) registered in the catalog, not just currently mounted ones.
- Used by console programs to populate the `volume=` argument completion list.

## Related Commands

- [.mediatypes](dot-mediatypes.md) -- List media type names
- [.volstatus](dot-volstatus.md) -- List volume status values
- [.pools](dot-pools.md) -- List pool names
- [label](label.md) -- Label a new volume
- [list](list.md) -- List volumes from catalog
