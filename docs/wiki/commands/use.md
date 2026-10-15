# use -- Switch to a Different Catalog

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre, `fc2c79b6c43e`](https://github.com/bareos/bareos/blob/fc2c79b6c43eb5752e98ebd86116593dbbbe0c68/core/src/dird/ua_cmds.cc#L518) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Use specific catalog

**Originalsyntax:**

```text
use catalog=<catalog>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `stub`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.

- Der Handler gibt immer Using default catalog zurück; er wechselt keinen Sitzungskatalog.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:188](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Select a different catalog database for the current console session.

## Syntax

```
use [catalog=<name>]
```

## Arguments

| Argument | Description |
|----------|-------------|
| `catalog=<name>` | Name of the Catalog resource to switch to |

## Examples

### Switch to a named catalog
```
* use catalog=MyCatalog
Using catalog MyCatalog
```

### Use default catalog
```
* use
Using default catalog
```

### Switch and verify
```
* use catalog=ArchiveCatalog
Using catalog ArchiveCatalog
* list jobs
(results from ArchiveCatalog database)
```

## Behavior

- Switches the current session's catalog connection to the specified Catalog resource.
- Subsequent `list`, `llist`, `sqlquery`, and other catalog-querying commands will use the new catalog.
- The Catalog resource must be defined in the Director configuration.
- Without arguments, resets to the default catalog.
- Most installations use a single catalog, making this command rarely needed.
- This is not an audited command.

## Related Commands

- [.catalogs](dot-catalogs.md) -- List all configured catalog resources
- [list](list.md) -- List records from the active catalog
- [sqlquery](sqlquery.md) -- Run SQL queries against the active catalog
