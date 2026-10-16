# .catalogs — List Catalog Names

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L229) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** List all catalog resources

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-acl, parity-bvfs, parity-limited, parity-operator.

**Paritätsfälle (9):** `c025-catalogs-limited`, `c025-catalogs-limited-api2`, `c025-catalogs-operator`, `c025-catalogs-operator-api2`, `c025-catalogs-bvfs`, `c025-catalogs-bvfs-api2`, `c025-catalogs-acl`, `c025-catalogs-acl-api2`, `dot_catalogs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:231](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Machine-interface command used by bconsole/oconsole for tab-completion. Returns one catalog resource name per line from the Director configuration.

## Syntax

```
.catalogs
```

## Arguments

None.

## Examples

### List all catalogs
```
* .catalogs
MyCatalog
```

### Typical single-catalog setup
```
* .catalogs
bareos
```

## Notes

- Output is one name per line, no headers, no formatting.
- Output is terminated by the EOD signal.
- Most installations have exactly one catalog.
- Used by console programs to populate the `catalog=` argument completion list.

## Related Commands

- [.catalog](dot-catalog.md) -- Catalog schema management
- [.sql](dot-sql.md) -- Execute SQL against the catalog
- [.sqltables](dot-sqltables.md) -- List catalog table names
- [use](use.md) -- Switch active catalog
