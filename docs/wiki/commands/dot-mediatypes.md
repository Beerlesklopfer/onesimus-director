# .mediatypes — List Media Type Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L257) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all media types

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c002-private-then-shared`, `c025-mediatypes-limited`, `c025-mediatypes-operator`, `c025-mediatypes-bvfs`, `c025-mediatypes-acl`, `c029-mediatypes`, `c029-mediatypes-api1`, `c029-mediatypes-api2`, `dot_mediatypes`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:332](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one unique media type per line, derived from configured Storage resources.

## Syntax

```
.mediatypes
```

## Arguments

None.

## Examples

### List all media types
```
* .mediatypes
File
LTO-8
```

### Single storage type
```
* .mediatypes
File
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Values are extracted from the MediaType directive of all Storage resources, sorted and deduplicated.
- Used by console programs to populate the `mediatype=` argument completion list.

## Related Commands

- [.storage](dot-storage.md) -- List storage names
- [.media](dot-media.md) -- List volume names
- [.defaults](dot-defaults.md) -- Show storage defaults (includes mediatype)
